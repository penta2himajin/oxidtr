pub mod expr_translator;
mod import_rewrite;

use self::import_rewrite::rewrite_models_import;

use super::{GeneratedFile, TargetLang, is_native_type_alias, resolve_type};
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Expr, Multiplicity, SigMultiplicity, TemporalBinaryOp};
use crate::analyze;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    generate_with_config(ir, &RustBackendConfig::default())
}

/// Configuration for the Rust backend.
#[derive(Debug, Clone, Default)]
pub struct RustBackendConfig {
    pub features: Vec<String>,
}

pub fn generate_with_config(ir: &OxidtrIR, config: &RustBackendConfig) -> Vec<GeneratedFile> {
    // If any structure has a module, use module-based layout
    let has_modules = ir.structures.iter().any(|s| s.module.is_some());
    if has_modules {
        return generate_modular(ir, config);
    }

    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "models.rs".to_string(),
        content: generate_models_with_config(ir, config),
    });

    // Check if TC functions are needed by any expression
    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    // Generate helpers.rs for TC functions (replaces invariants.rs)
    if has_tc {
        files.push(GeneratedFile {
            path: "helpers.rs".to_string(),
            content: generate_helpers(ir),
        });
    }

    if !ir.operations.is_empty() {
        files.push(GeneratedFile {
            path: "operations.rs".to_string(),
            content: generate_operations(ir),
        });
    }

    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        files.push(GeneratedFile {
            path: "tests.rs".to_string(),
            content: generate_tests(ir),
        });
    }

    files.push(GeneratedFile {
        path: "fixtures.rs".to_string(),
        content: generate_fixtures(ir),
    });

    // Generate newtypes for named constraints
    let newtype_content = generate_newtypes(ir);
    if !newtype_content.is_empty() {
        files.push(GeneratedFile {
            path: "newtypes.rs".to_string(),
            content: newtype_content,
        });
    }

    files
}

/// Group structures into "concepts": abstract sig + all sub-sigs form one concept,
/// standalone sigs form individual concepts.
fn group_into_concepts<'a>(structures: &[&'a StructureNode]) -> Vec<(String, Vec<&'a StructureNode>)> {
    let enum_parents: HashSet<String> = structures.iter()
        .filter(|s| s.is_enum)
        .map(|s| s.name.clone())
        .collect();

    // Build parent→children map
    let mut children_of: HashMap<String, Vec<&'a StructureNode>> = HashMap::new();

    for s in structures {
        if super::is_native_type_alias(&s.name) {
            continue;
        }
        if let Some(parent) = &s.parent {
            if enum_parents.contains(parent) {
                // This is an enum variant — grouped with parent
                children_of.entry(parent.clone()).or_default().push(s);
            }
        }
    }

    let mut concepts: Vec<(String, Vec<&'a StructureNode>)> = Vec::new();
    let mut emitted: HashSet<String> = HashSet::new();

    for s in structures {
        if emitted.contains(&s.name) || super::is_native_type_alias(&s.name) {
            continue;
        }
        // Skip enum variants — they'll be grouped with their parent
        if s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)) {
            continue;
        }
        let mut members = vec![*s];
        if let Some(kids) = children_of.get(&s.name) {
            for k in kids {
                members.push(k);
                emitted.insert(k.name.clone());
            }
        }
        emitted.insert(s.name.clone());
        concepts.push((s.name.clone(), members));
    }
    concepts
}

/// Generate module-based layout: each module gets a subdirectory,
/// each domain concept gets its own file within that module.
fn generate_modular(ir: &OxidtrIR, config: &RustBackendConfig) -> Vec<GeneratedFile> {
    let use_serde = config.features.contains(&"serde".to_string());
    let mut files = Vec::new();

    // Group structures by module
    let mut module_order: Vec<String> = Vec::new();
    let mut by_module: HashMap<String, Vec<&StructureNode>> = HashMap::new();
    let mut ungrouped: Vec<&StructureNode> = Vec::new();

    for s in &ir.structures {
        if let Some(m) = &s.module {
            by_module.entry(m.clone()).or_default().push(s);
            if !module_order.contains(m) {
                module_order.push(m.clone());
            }
        } else {
            ungrouped.push(s);
        }
    }

    // Collect all type names across the entire IR (for cross-module imports)
    let all_type_names: HashSet<String> = ir.structures.iter()
        .map(|s| s.name.clone())
        .collect();

    // Track which types are defined in which module (for import resolution)
    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for s in &ir.structures {
        if let Some(m) = &s.module {
            type_to_module.insert(s.name.clone(), m.clone());
        }
    }

    // Build self-ref fields for the entire IR
    let self_ref_fields = find_cyclic_fields(ir);
    let disj_fields = analyze::disj_fields(ir);

    // Generate each module directory
    for module_name in &module_order {
        let module_structures = &by_module[module_name];
        let concepts = group_into_concepts(module_structures);

        let mut mod_rs_items: Vec<String> = Vec::new();

        for (concept_name, members) in &concepts {
            let file_name = to_snake_case(concept_name);
            let file_path = format!("{module_name}/{file_name}.rs");

            let content = generate_concept_file(
                members, ir, &self_ref_fields, use_serde, &disj_fields,
                module_name, &type_to_module, &all_type_names,
            );

            files.push(GeneratedFile { path: file_path, content });
            mod_rs_items.push(file_name);
        }

        // Generate mod.rs for this module
        let mut mod_rs = String::new();
        for item in &mod_rs_items {
            writeln!(mod_rs, "pub mod {item};").unwrap();
        }
        writeln!(mod_rs).unwrap();
        // Re-export all public types for convenience
        for item in &mod_rs_items {
            writeln!(mod_rs, "pub use {item}::*;").unwrap();
        }
        files.push(GeneratedFile {
            path: format!("{module_name}/mod.rs"),
            content: mod_rs,
        });
    }

    // Ungrouped structures → top-level models.rs (if any)
    if !ungrouped.is_empty() {
        let mut sub_ir = ir.clone();
        sub_ir.structures = ungrouped.iter().map(|s| (*s).clone()).collect();
        files.push(GeneratedFile {
            path: "models.rs".to_string(),
            content: generate_models_with_config(&sub_ir, config),
        });
    }

    // Generate lib.rs (top-level module declarations)
    let mut lib_rs = String::new();
    for module_name in &module_order {
        writeln!(lib_rs, "pub mod {module_name};").unwrap();
    }
    if !ungrouped.is_empty() {
        writeln!(lib_rs, "pub mod models;").unwrap();
    }

    // Check if TC, operations, tests, fixtures, newtypes are needed
    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));
    if has_tc {
        writeln!(lib_rs, "pub mod helpers;").unwrap();
        let helpers_content = rewrite_models_import(
            generate_helpers(ir),
            &module_order,
            !ungrouped.is_empty(),
        );
        files.push(GeneratedFile {
            path: "helpers.rs".to_string(),
            content: helpers_content,
        });
    }
    if !ir.operations.is_empty() {
        writeln!(lib_rs, "pub mod operations;").unwrap();
        files.push(GeneratedFile {
            path: "operations.rs".to_string(),
            content: generate_operations_modular(ir, &module_order),
        });
    }
    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        writeln!(lib_rs, "#[cfg(test)]").unwrap();
        writeln!(lib_rs, "mod tests;").unwrap();
        files.push(GeneratedFile {
            path: "tests.rs".to_string(),
            content: generate_tests_modular(ir, &module_order),
        });
    }
    files.push(GeneratedFile {
        path: "fixtures.rs".to_string(),
        content: generate_fixtures_modular(ir, &module_order),
    });
    writeln!(lib_rs, "pub mod fixtures;").unwrap();

    let newtype_content = generate_newtypes_modular(ir, &module_order);
    if !newtype_content.is_empty() {
        writeln!(lib_rs, "pub mod newtypes;").unwrap();
        files.push(GeneratedFile {
            path: "newtypes.rs".to_string(),
            content: newtype_content,
        });
    }

    files.push(GeneratedFile {
        path: "mod.rs".to_string(),
        content: lib_rs,
    });

    files
}

/// Generate a single concept file containing one or more related structures.
fn generate_concept_file(
    members: &[&StructureNode],
    ir: &OxidtrIR,
    self_ref_fields: &HashSet<(String, String)>,
    use_serde: bool,
    disj_fields: &[(String, String)],
    current_module: &str,
    type_to_module: &HashMap<String, String>,
    _all_type_names: &HashSet<String>,
) -> String {
    let mut out = String::new();

    // Collect types referenced by members that are in other modules
    let member_names: HashSet<String> = members.iter().map(|m| m.name.clone()).collect();
    let mut needed_imports: HashMap<String, HashSet<String>> = HashMap::new(); // module → types

    for s in members {
        for f in &s.fields {
            if let Some(module) = type_to_module.get(&f.target) {
                if module != current_module && !super::is_native_type_alias(&f.target) {
                    needed_imports.entry(module.clone()).or_default().insert(f.target.clone());
                }
            }
        }
        // Parent in another module
        if let Some(parent) = &s.parent {
            if !member_names.contains(parent) {
                if let Some(module) = type_to_module.get(parent) {
                    if module != current_module {
                        needed_imports.entry(module.clone()).or_default().insert(parent.clone());
                    }
                }
            }
        }
    }

    // Check if any field uses Set/Map
    let needs_btreeset = members.iter().any(|s| s.fields.iter().any(|f| f.mult == Multiplicity::Set));
    let needs_btreemap = members.iter().any(|s| s.fields.iter().any(|f| f.value_type.is_some()));

    if needs_btreeset || needs_btreemap {
        let mut imports = Vec::new();
        if needs_btreemap { imports.push("BTreeMap"); }
        if needs_btreeset { imports.push("BTreeSet"); }
        writeln!(out, "use std::collections::{{{}}};", imports.join(", ")).unwrap();
    }

    if use_serde {
        writeln!(out, "use serde::{{Serialize, Deserialize}};").unwrap();
    }

    // Cross-module imports
    let mut sorted_modules: Vec<&String> = needed_imports.keys().collect();
    sorted_modules.sort();
    for module in sorted_modules {
        let types = needed_imports.get(module).unwrap();
        let mut sorted_types: Vec<&String> = types.iter().collect();
        sorted_types.sort();
        writeln!(out, "use crate::{}::{{{}}};", module, sorted_types.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")).unwrap();
    }

    // Intra-module imports: types from sibling files within the same module
    let mut intra_module_types: HashSet<String> = HashSet::new();
    for s in members {
        for f in &s.fields {
            if !member_names.contains(&f.target) && !super::is_native_type_alias(&f.target) {
                if let Some(module) = type_to_module.get(&f.target) {
                    if module == current_module {
                        intra_module_types.insert(f.target.clone());
                    }
                }
            }
        }
        if let Some(parent) = &s.parent {
            if !member_names.contains(parent) {
                if let Some(module) = type_to_module.get(parent) {
                    if module == current_module {
                        intra_module_types.insert(parent.clone());
                    }
                }
            }
        }
    }
    if !intra_module_types.is_empty() {
        let mut sorted: Vec<&String> = intra_module_types.iter().collect();
        sorted.sort();
        // Use super:: to access sibling types within the same module
        for t in &sorted {
            writeln!(out, "use super::{t};").unwrap();
        }
    }

    if needs_btreeset || needs_btreemap || use_serde || !needed_imports.is_empty() || !intra_module_types.is_empty() {
        writeln!(out).unwrap();
    }

    // Build sub-IR scoped to these members for constraint analysis
    let enum_parents: HashSet<String> = members.iter()
        .filter(|s| s.is_enum)
        .map(|s| s.name.clone())
        .collect();
    let variant_names: HashSet<String> = members.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone())
        .collect();

    // Collect parent→children mapping for enum generation
    let children: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for s in members {
            if let Some(parent) = &s.parent {
                map.entry(parent.clone()).or_default().push(s.name.clone());
            }
        }
        map
    };

    for s in members {
        // Skip variant sigs — they become enum variants
        if variant_names.contains(&s.name) {
            continue;
        }
        if super::is_native_type_alias(&s.name) {
            continue;
        }

        // Intersection type alias
        if !s.intersection_of.is_empty() {
            let first = &s.intersection_of[0];
            let rest: Vec<&str> = s.intersection_of[1..].iter().map(|x| x.as_str()).collect();
            if rest.is_empty() {
                writeln!(out, "pub type {} = {};", s.name, first).unwrap();
            } else {
                writeln!(out, "// Intersection: {} = {}", s.name, s.intersection_of.join(" & ")).unwrap();
                writeln!(out, "pub type {} = {}; // also includes: {}", s.name, first, rest.join(", ")).unwrap();
            }
            writeln!(out).unwrap();
            continue;
        }

        // Doc comments from constraints
        let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
        for cn in &constraint_names {
            writeln!(out, "/// Invariant: {cn}").unwrap();
        }

        if s.is_enum {
            generate_enum(&mut out, s, children.get(&s.name), ir, self_ref_fields, use_serde);
        } else {
            generate_struct(&mut out, s, self_ref_fields, use_serde, ir, disj_fields);
        }
        writeln!(out).unwrap();
    }

    // Derived fields for types in this concept
    let member_set: HashSet<&str> = members.iter().map(|s| s.name.as_str()).collect();
    for op in &ir.operations {
        if let Some(ref sig) = op.receiver_sig {
            if member_set.contains(sig.as_str()) {
                // Generate impl block for this receiver
                let fn_name = to_snake_case(&op.name);
                let params = op.params.iter().map(|p| {
                    let type_str = match p.mult {
                        Multiplicity::One => format!("&{}", p.type_name),
                        Multiplicity::Lone => format!("Option<&{}>", p.type_name),
                        Multiplicity::Set => format!("&std::collections::BTreeSet<{}>", p.type_name),
                        Multiplicity::Seq => format!("&[{}]", p.type_name),
                    };
                    format!("{}: {type_str}", to_snake_case(&p.name))
                }).collect::<Vec<_>>().join(", ");

                let return_str = match &op.return_type {
                    Some(rt) => {
                        let t = rust_return_type(&rt.type_name, &rt.mult);
                        format!(" -> {t}")
                    }
                    None => " -> bool".to_string(),
                };

                let param_str = if params.is_empty() {
                    "&self".to_string()
                } else {
                    format!("&self, {params}")
                };

                writeln!(out, "impl {sig} {{").unwrap();
                writeln!(out, "    pub fn {fn_name}({param_str}){return_str} {{").unwrap();
                writeln!(out, "        todo!(\"oxidtr: implement {}\");", op.name).unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out, "}}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    out
}

/// Generate operations.rs with modular imports
fn generate_operations_modular(ir: &OxidtrIR, modules: &[String]) -> String {
    let mut out = String::new();

    for module in modules {
        writeln!(out, "use super::{module}::*;").unwrap();
    }
    writeln!(out).unwrap();

    // Check if any operation parameter or return type uses Set multiplicity
    let needs_btreeset = ir.operations.iter().any(|op| {
        op.params.iter().any(|p| p.mult == Multiplicity::Set)
            || op.return_type.as_ref().map_or(false, |r| r.mult == Multiplicity::Set)
    });
    if needs_btreeset {
        writeln!(out, "use std::collections::BTreeSet;").unwrap();
        writeln!(out).unwrap();
    }

    for op in &ir.operations {
        if op.receiver_sig.is_some() {
            continue;
        }
        let fn_name = to_snake_case(&op.name);
        let params = op.params.iter().map(|p| {
            let type_str = match p.mult {
                Multiplicity::One => format!("&{}", p.type_name),
                Multiplicity::Lone => format!("Option<&{}>", p.type_name),
                Multiplicity::Set => format!("&BTreeSet<{}>", p.type_name),
                Multiplicity::Seq => format!("&[{}]", p.type_name),
            };
            format!("{}: {type_str}", to_snake_case(&p.name))
        }).collect::<Vec<_>>().join(", ");

        let return_str = match &op.return_type {
            Some(rt) => {
                let t = rust_return_type(&rt.type_name, &rt.mult);
                format!(" -> {t}")
            }
            None => " -> bool".to_string(),
        };

        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, "/// @{tag}: {desc}").unwrap();
            }
        }

        writeln!(out, "pub fn {fn_name}({params}){return_str} {{").unwrap();
        writeln!(out, "    todo!(\"oxidtr: implement {}\");", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

/// Generate tests.rs with modular imports
fn generate_tests_modular(ir: &OxidtrIR, modules: &[String]) -> String {
    // For now, reuse the existing test generation but fix imports
    let original = generate_tests(ir);
    // Replace `use crate::models::*` with module imports
    let mut result = original.replace("use super::models::*;",
        &modules.iter().map(|m| format!("use super::{m}::*;")).collect::<Vec<_>>().join("\n    "));
    // Also update fixtures import
    result = result.replace("use super::fixtures::*;", "use super::fixtures::*;");
    result
}

/// Generate fixtures.rs with modular imports
fn generate_fixtures_modular(ir: &OxidtrIR, modules: &[String]) -> String {
    let original = generate_fixtures(ir);
    original.replace("use super::models::*;",
        &modules.iter().map(|m| format!("use super::{m}::*;")).collect::<Vec<_>>().join("\n"))
}

/// Generate newtypes.rs with modular imports
fn generate_newtypes_modular(ir: &OxidtrIR, modules: &[String]) -> String {
    let original = generate_newtypes(ir);
    if original.is_empty() { return original; }
    original.replace("use super::models::*;",
        &modules.iter().map(|m| format!("use super::{m}::*;")).collect::<Vec<_>>().join("\n"))
}


/// Convert a raw union type string from TypeScript/other languages to Rust.
/// e.g. "number | string" → "serde_json::Value" (opaque union)
/// For simple numeric unions: use an enum if recognizable, otherwise serde_json::Value.
fn rust_union_type(raw: &str, mult: &Multiplicity) -> String {
    // Simple known mappings
    let base = if raw == "number | string" || raw == "string | number" {
        "serde_json::Value".to_string()
    } else if raw.split(" | ").all(|s| s.trim() == "number" || s.trim() == "string" || s.trim() == "boolean") {
        "serde_json::Value".to_string()
    } else {
        // Unknown union: use String as safe fallback
        "String".to_string()
    };
    match mult {
        Multiplicity::Lone => format!("Option<{base}>"),
        Multiplicity::Set => format!("BTreeSet<{base}>"),
        Multiplicity::Seq => format!("Vec<{base}>"),
        Multiplicity::One => base,
    }
}

fn generate_models_with_config(ir: &OxidtrIR, config: &RustBackendConfig) -> String {
    let use_serde = config.features.contains(&"serde".to_string());
    let mut out = generate_models_inner(ir, use_serde);
    if use_serde {
        out.insert_str(0, "use serde::{Serialize, Deserialize};\n\n");
    }
    out
}

fn generate_models_inner(ir: &OxidtrIR, use_serde: bool) -> String {
    let mut out = String::new();

    // Check if any field uses Set multiplicity → need BTreeSet import
    let needs_btreeset = ir.structures.iter().any(|s| {
        s.fields.iter().any(|f| f.mult == Multiplicity::Set)
    });
    // Check if any field uses map type → need BTreeMap import
    let needs_btreemap = ir.structures.iter().any(|s| {
        s.fields.iter().any(|f| f.value_type.is_some())
    });
    if needs_btreeset || needs_btreemap {
        let mut imports = Vec::new();
        if needs_btreemap { imports.push("BTreeMap"); }
        if needs_btreeset { imports.push("BTreeSet"); }
        writeln!(out, "use std::collections::{{{}}};", imports.join(", ")).unwrap();
        writeln!(out).unwrap();
    }

    // Collect parent→children mapping for enum generation
    let children: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for s in &ir.structures {
        // Intersection type alias: type Foo = A & B & C → pub type Foo = A; // & B & C
        if !s.intersection_of.is_empty() {
            let first = &s.intersection_of[0];
            let rest: Vec<&str> = s.intersection_of[1..].iter().map(|x| x.as_str()).collect();
            if rest.is_empty() {
                writeln!(out, "pub type {} = {};", s.name, first).unwrap();
            } else {
                writeln!(out, "// Intersection: {} = {}", s.name, s.intersection_of.join(" & ")).unwrap();
                writeln!(out, "pub type {} = {}; // also includes: {}", s.name, first, rest.join(", ")).unwrap();
            }
            writeln!(out).unwrap();
            continue;
        }
            if let Some(parent) = &s.parent {
                map.entry(parent.clone()).or_default().push(s.name.clone());
            }
        }
        map
    };

    // Collect which sigs are enum variants (have a parent that is_enum)
    let enum_parents: HashSet<String> = ir
        .structures
        .iter()
        .filter(|s| s.is_enum)
        .map(|s| s.name.clone())
        .collect();

    let variant_names: HashSet<String> = ir
        .structures
        .iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone())
        .collect();

    // Collect fields that need Box<> wrapping: self-referential or part of a reference cycle.
    let self_ref_fields = find_cyclic_fields(ir);

    for s in &ir.structures {
        // Skip variant sigs — they become enum variants
        if variant_names.contains(&s.name) {
            continue;
        }
        // Skip native type aliases (Str, Int, Float, Bool)
        if is_native_type_alias(&s.name) {
            continue;
        }

        // Doc comments from constraints
        let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
        for cn in &constraint_names {
            writeln!(out, "/// Invariant: {cn}").unwrap();
        }

        let disj_fields = analyze::disj_fields(ir);
        if s.is_enum {
            generate_enum(&mut out, s, children.get(&s.name), ir, &self_ref_fields, use_serde);
        } else {
            generate_struct(&mut out, s, &self_ref_fields, use_serde, ir, &disj_fields);
        }
        writeln!(out).unwrap();
    }

    // Derived fields: receiver functions → impl blocks
    generate_derived_fields(&mut out, ir);

    out
}

fn generate_derived_fields(out: &mut String, ir: &OxidtrIR) {
    // Group receiver operations by sig name
    let mut by_sig: HashMap<String, Vec<&OperationNode>> = HashMap::new();
    for op in &ir.operations {
        if let Some(ref sig) = op.receiver_sig {
            by_sig.entry(sig.clone()).or_default().push(op);
        }
    }

    for (sig_name, ops) in &by_sig {
        writeln!(out, "impl {sig_name} {{").unwrap();
        for op in ops {
            let fn_name = to_snake_case(&op.name);
            let params = op.params.iter().map(|p| {
                let type_str = match p.mult {
                    Multiplicity::One => format!("&{}", p.type_name),
                    Multiplicity::Lone => format!("Option<&{}>", p.type_name),
                    Multiplicity::Set => format!("&std::collections::BTreeSet<{}>", p.type_name),
                    Multiplicity::Seq => format!("&[{}]", p.type_name),
                };
                format!("{}: {type_str}", to_snake_case(&p.name))
            }).collect::<Vec<_>>().join(", ");

            let return_str = match &op.return_type {
                Some(rt) => {
                    let t = rust_return_type(&rt.type_name, &rt.mult);
                    format!(" -> {t}")
                }
                None => " -> bool".to_string(),
            };

            let param_str = if params.is_empty() {
                "&self".to_string()
            } else {
                format!("&self, {params}")
            };

            writeln!(out, "    pub fn {fn_name}({param_str}){return_str} {{").unwrap();
            writeln!(out, "        todo!(\"oxidtr: implement {}\");", op.name).unwrap();
            writeln!(out, "    }}").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

fn generate_enum(
    out: &mut String,
    s: &StructureNode,
    children: Option<&Vec<String>>,
    ir: &OxidtrIR,
    self_ref_fields: &HashSet<(String, String)>,
    use_serde: bool,
) {
    // Build name→StructureNode lookup for child sigs
    let struct_map: HashMap<&str, &StructureNode> = ir
        .structures
        .iter()
        .map(|st| (st.name.as_str(), st))
        .collect();

    // Parent abstract sig may have fields that should be inherited by all variants
    let parent_fields = &s.fields;

    // Detect fixed-value fields: one sig + fact { Sig.field = constant }
    let fixed_values = analyze::fixed_value_fields(ir);

    // Check if a variant has all its fields fixed by facts (→ unit variant + const method)
    let variant_is_unit = |variant_name: &str, fields: &[&IRField]| -> bool {
        if fields.is_empty() { return true; }
        let child_node = struct_map.get(variant_name);
        let is_singleton = child_node.map_or(false, |c| c.sig_multiplicity == SigMultiplicity::One);
        if !is_singleton { return false; }
        fields.iter().all(|f| fixed_values.contains_key(&(variant_name.to_string(), f.name.clone())))
    };

    // Collect (field_name, type, values per variant) for const methods
    let mut const_methods: Vec<(String, String, Vec<(String, i64)>)> = Vec::new();

    if use_serde {
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]").unwrap();
        // Check if any variant has non-fixed fields (including inherited parent fields)
        let has_data_variants = children.as_ref().map_or(false, |vs| {
            vs.iter().any(|v| {
                let child = struct_map.get(v.as_str());
                let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
                let all_fields: Vec<&IRField> = parent_fields.iter().chain(child_fields.iter().copied()).collect();
                !variant_is_unit(v, &all_fields)
            })
        });
        if has_data_variants {
            writeln!(out, "#[serde(tag = \"type\")]").unwrap();
        }
    } else {
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]").unwrap();
    }
    writeln!(out, "pub enum {} {{", s.name).unwrap();
    if let Some(variants) = children {
        for v in variants {
            let child = struct_map.get(v.as_str());
            let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
            // Combine parent fields + child fields
            let all_fields: Vec<&IRField> = parent_fields.iter().chain(child_fields.iter().copied()).collect();
            if !all_fields.is_empty() && !variant_is_unit(v, &all_fields) {
                writeln!(out, "    {v} {{").unwrap();
                for f in &all_fields {
                    // Fields referencing the parent enum type need Box to break recursion
                    let needs_box = f.target == s.name;
                    let is_self_ref = needs_box
                        || self_ref_fields.contains(&(v.clone(), f.name.clone()));
                    let resolved_target = resolve_type(TargetLang::Rust, &f.target);
                    let type_str = if let Some(vt) = &f.value_type {
                        let resolved_vt = resolve_type(TargetLang::Rust, vt);
                        format!("BTreeMap<{}, {}>", resolved_target, resolved_vt)
                    } else if let Some(raw) = &f.raw_union_type {
                        rust_union_type(raw, &f.mult)
                    } else {
                        multiplicity_to_type(&resolved_target, &f.mult, is_self_ref)
                    };
                    writeln!(out, "        {}: {type_str},", f.name).unwrap();
                }
                writeln!(out, "    }},").unwrap();
            } else {
                // Unit variant: collect fixed values for const methods
                for f in &all_fields {
                    if let Some(val) = fixed_values.get(&(v.to_string(), f.name.clone())) {
                        let resolved_target = resolve_type(TargetLang::Rust, &f.target);
                        if let Some(entry) = const_methods.iter_mut().find(|(name, _, _)| name == &f.name) {
                            entry.2.push((v.clone(), *val));
                        } else {
                            const_methods.push((f.name.clone(), resolved_target, vec![(v.clone(), *val)]));
                        }
                    }
                }
                writeln!(out, "    {v},").unwrap();
            }
        }
    }
    writeln!(out, "}}").unwrap();

    // Generate const methods for fixed-value fields
    if !const_methods.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "impl {} {{", s.name).unwrap();
        for (field_name, field_type, variant_values) in &const_methods {
            writeln!(out, "    pub const fn {field_name}(&self) -> {field_type} {{").unwrap();
            writeln!(out, "        match self {{").unwrap();
            for (variant, val) in variant_values {
                writeln!(out, "            Self::{variant} => {val},").unwrap();
            }
            writeln!(out, "        }}").unwrap();
            writeln!(out, "    }}").unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
}

fn generate_struct(
    out: &mut String,
    s: &StructureNode,
    self_ref_fields: &HashSet<(String, String)>,
    use_serde: bool,
    ir: &OxidtrIR,
    disj_fields: &[(String, String)],
) {
    // Singleton: one sig → unit struct + INSTANCE constant
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        if s.is_var {
            writeln!(out, "/// MUTABLE SIG: instances of this sig change across state transitions").unwrap();
        }
        if use_serde {
            writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]").unwrap();
        } else {
            writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]").unwrap();
        }
        writeln!(out, "pub struct {};", s.name).unwrap();
        writeln!(out, "pub const {}_INSTANCE: {} = {};", to_snake_case(&s.name).to_uppercase(), s.name, s.name).unwrap();
        return;
    }

    if s.is_var {
        writeln!(out, "/// MUTABLE SIG: instances of this sig change across state transitions").unwrap();
    }
    if use_serde {
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]").unwrap();
    } else {
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]").unwrap();
    }
    if s.fields.is_empty() {
        writeln!(out, "pub struct {};", s.name).unwrap();
    } else {
        writeln!(out, "pub struct {} {{", s.name).unwrap();
        for f in &s.fields {
            // Gap 1 & 3: annotations for sig multiplicity and disj constraints
            write_field_annotations_rust(out, ir, &s.name, f, "    ", disj_fields);
            let is_self_ref = self_ref_fields.contains(&(s.name.clone(), f.name.clone()));
            let resolved_target = resolve_type(TargetLang::Rust, &f.target);
            let type_str = if let Some(vt) = &f.value_type {
                let resolved_vt = resolve_type(TargetLang::Rust, vt);
                format!("BTreeMap<{}, {}>", resolved_target, resolved_vt)
            } else if let Some(raw) = &f.raw_union_type {
                rust_union_type(raw, &f.mult)
            } else {
                multiplicity_to_type(&resolved_target, &f.mult, is_self_ref)
            };
            if f.is_var {
                writeln!(out, "    /// MUTABLE: this field changes across state transitions").unwrap();
            }
            writeln!(out, "    pub {}: {type_str},", f.name).unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
}

/// Generate doc comments for fields based on target sig multiplicity and disj constraints.
fn write_field_annotations_rust(
    out: &mut String,
    ir: &OxidtrIR,
    sig_name: &str,
    f: &IRField,
    indent: &str,
    disj_fields: &[(String, String)],
) {
    use crate::parser::ast::SigMultiplicity;

    let target_mult = analyze::sig_multiplicity_for(ir, &f.target);
    match target_mult {
        SigMultiplicity::Some => {
            if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                writeln!(out, "{indent}/// @constraint Target is `some sig` — collection must not be empty.").unwrap();
            }
        }
        SigMultiplicity::Lone => {
            if f.mult == Multiplicity::One {
                writeln!(out, "{indent}/// @constraint Target is `lone sig` — reference may not exist.").unwrap();
            }
        }
        _ => {}
    }

    // Gap 3: disj → suggest Set
    if disj_fields.iter().any(|(sig, field)| sig == sig_name && field == &f.name) {
        if f.mult == Multiplicity::Seq {
            writeln!(out, "{indent}/// Consider using BTreeSet<T> for uniqueness (disj constraint).").unwrap();
        }
    }
}

fn multiplicity_to_type(target: &str, mult: &Multiplicity, is_self_ref: bool) -> String {
    match mult {
        Multiplicity::One => {
            if is_self_ref {
                format!("Box<{target}>")
            } else {
                target.to_string()
            }
        }
        Multiplicity::Lone => {
            if is_self_ref {
                format!("Option<Box<{target}>>")
            } else {
                format!("Option<{target}>")
            }
        }
        Multiplicity::Set => format!("BTreeSet<{target}>"),
        Multiplicity::Seq => format!("Vec<{target}>"),
    }
}

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "use super::models::*;").unwrap();

    // Check if any operation parameter or return type uses Set multiplicity
    let needs_btreeset = ir.operations.iter().any(|op| {
        op.params.iter().any(|p| p.mult == Multiplicity::Set)
            || op.return_type.as_ref().map_or(false, |r| r.mult == Multiplicity::Set)
    });
    if needs_btreeset {
        writeln!(out, "use std::collections::BTreeSet;").unwrap();
    }
    writeln!(out).unwrap();

    for op in &ir.operations {
        // Skip receiver functions — they are rendered as impl methods in models.rs
        if op.receiver_sig.is_some() {
            continue;
        }
        let fn_name = to_snake_case(&op.name);
        let params = op
            .params
            .iter()
            .map(|p| {
                let type_str = match p.mult {
                    Multiplicity::One => format!("&{}", p.type_name),
                    Multiplicity::Lone => format!("Option<&{}>", p.type_name),
                    Multiplicity::Set => format!("&BTreeSet<{}>", p.type_name),
                    Multiplicity::Seq => format!("&[{}]", p.type_name),
                };
                format!("{}: {type_str}", to_snake_case(&p.name))
            })
            .collect::<Vec<_>>()
            .join(", ");

        let return_str = match &op.return_type {
            Some(rt) => {
                let t = rust_return_type(&rt.type_name, &rt.mult);
                format!(" -> {t}")
            }
            None => " -> bool".to_string(),
        };

        // Doc comments with pre/post separation (Feature 7)
        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, "/// @{tag}: {desc}").unwrap();
            }
        }

        writeln!(out, "pub fn {fn_name}({params}){return_str} {{").unwrap();
        writeln!(out, "    todo!(\"oxidtr: implement {}\");", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

fn rust_return_type(type_name: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => type_name.to_string(),
        Multiplicity::Lone => format!("Option<{type_name}>"),
        Multiplicity::Set => format!("BTreeSet<{type_name}>"),
        Multiplicity::Seq => format!("Vec<{type_name}>"),
    }
}

/// Generate helpers.rs containing TC (transitive closure) functions.
/// These were previously in invariants.rs.
fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use super::models::*;").unwrap();
    writeln!(out).unwrap();

    // Collect all TC fields and generate specific traversal functions
    let mut tc_fields = Vec::new();
    for c in &ir.constraints {
        tc_fields.extend(expr_translator::extract_tc_fields(&c.expr, ir));
    }
    for p in &ir.properties {
        tc_fields.extend(expr_translator::extract_tc_fields(&p.expr, ir));
    }
    tc_fields.sort_by(|a, b| (&a.sig_name, &a.field_name).cmp(&(&b.sig_name, &b.field_name)));
    tc_fields.dedup();

    for tc in &tc_fields {
        generate_tc_function(&mut out, tc);
    }

    out
}

fn collect_sig_names(ir: &OxidtrIR) -> std::collections::HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

fn expr_uses_tc(expr: &crate::parser::ast::Expr) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::TransitiveClosure(_) => true,
        Expr::FieldAccess { base, .. } => expr_uses_tc(base),
        Expr::Cardinality(inner) | Expr::Not(inner) => expr_uses_tc(inner),
        Expr::MultFormula { expr: inner, .. } => expr_uses_tc(inner),
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            expr_uses_tc(left) || expr_uses_tc(right)
        }
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| expr_uses_tc(&b.domain)) || expr_uses_tc(body)
        }
        Expr::Prime(inner) => expr_uses_tc(inner),
        Expr::TemporalUnary { expr: inner, .. } => expr_uses_tc(inner),
        Expr::TemporalBinary { left, right, .. } => {
            expr_uses_tc(left) || expr_uses_tc(right)
        }
        Expr::FunApp { receiver, args, .. } => receiver.as_ref().map_or(false, |r| expr_uses_tc(r)) || args.iter().any(|a| expr_uses_tc(a)),
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
}

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = collect_sig_names(ir);

    // Collect which sigs have fixture factories (non-enum, non-variant, with fields)
    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names_set: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();
    let has_fixture: HashSet<String> = ir.structures.iter()
        .filter(|s| !variant_names_set.contains(&s.name) && !s.is_enum && !s.fields.is_empty()
            && !is_native_type_alias(&s.name))
        .map(|s| s.name.clone()).collect();

    // Check if any expression uses TC functions → need helpers import
    let needs_helpers = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    // tests.rs is already #[cfg(test)] gated via mod.rs, no wrapper needed
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use super::models::*;").unwrap();
    if needs_helpers {
        writeln!(out, "#[allow(unused_imports)]").unwrap();
        writeln!(out, "use super::helpers::*;").unwrap();
    }
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use super::fixtures::*;").unwrap();
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use super::operations::*;").unwrap();
    writeln!(out).unwrap();

    // Property tests from asserts — translated expressions
    for prop in &ir.properties {
        let test_name = to_snake_case(&prop.name);
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
        // Skip tests that reference enum variants (not standalone types in Rust)
        if params.iter().any(|(_, tname)| variant_names_set.contains(tname) || enum_parents.contains(tname)) {
            continue;
        }
        let body = expr_translator::translate_with_ir(&prop.expr, ir);

        writeln!(out, "#[test]").unwrap();
        writeln!(out, "fn {test_name}() {{").unwrap();

        // Only populate primary sig (outermost quantifier domain) with fixtures.
        // Secondary sigs get Vec::new() to avoid vacuously-true assertions.
        let primary_sigs: HashSet<String> = if let Some((_kind, bindings, _body)) = analyze::strip_outer_quantifier(&prop.expr) {
            bindings.iter().flat_map(|b| {
                if let Expr::VarRef(name) = &b.domain { Some(name.clone()) } else { None }
            }).collect()
        } else {
            params.iter().map(|(_, t)| t.clone()).collect()
        };
        for (pname, tname) in &params {
            if primary_sigs.contains(tname) && has_fixture.contains(tname) {
                let snake = to_snake_case(tname);
                writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
            } else if has_fixture.contains(tname) {
                let snake = to_snake_case(tname);
                writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
            } else {
                writeln!(out, "    let {pname}: Vec<{tname}> = Vec::new();").unwrap();
            }
        }

        writeln!(out, "    assert!({body});").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    // Invariant tests — inline constraint expressions directly
    // Rust has a strong type system: skip tests for constraints guaranteed by types.
    let all_constraints = analyze::analyze(ir);
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };

        // Alloy 6: temporal facts with prime → generate transition test
        // Strips quantifier, rewrites prime refs (x') to post-state variables (next_x),
        // and generates zip-based pre/post assertion.
        if analyze::expr_contains_prime(&constraint.expr) {
            let test_name = format!("transition_{}", to_snake_case(&fact_name));
            let params = expr_translator::extract_params(&constraint.expr, &sig_names);
            if params.iter().any(|(_, tname)| variant_names_set.contains(tname) || enum_parents.contains(tname)) {
                continue;
            }
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "/// @temporal Transition constraint: {fact_name}").unwrap();
            writeln!(out, "/// Verifies: pre→post state relationship ({desc})").unwrap();
            writeln!(out, "#[test]").unwrap();
            writeln!(out, "fn {test_name}() {{").unwrap();
            for (pname, tname) in &params {
                if has_fixture.contains(tname) {
                    let snake = to_snake_case(tname);
                    writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                } else {
                    writeln!(out, "    let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                }
                writeln!(out, "    let next_{pname}: Vec<{tname}> = {pname}.clone();").unwrap();
            }
            // Strip quantifier, rewrite body, generate zip assertion
            if let Some((_kind, bindings, inner_body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                let rewritten_body = analyze::rewrite_prime_as_post_state(inner_body);
                let body_str = expr_translator::translate_with_ir(&rewritten_body, ir);
                // Generate zip-based iteration over pre/post pairs
                let bind_vars: Vec<String> = bindings.iter()
                    .flat_map(|b| b.vars.clone())
                    .collect();
                if bind_vars.len() == 1 {
                    let v = &bind_vars[0];
                    let pname = &params[0].0;
                    writeln!(out, "    for ({v}, next_{v}) in {pname}.iter().zip(next_{pname}.iter()) {{").unwrap();
                    writeln!(out, "        assert!({body_str});").unwrap();
                    writeln!(out, "    }}").unwrap();
                } else {
                    writeln!(out, "    assert!({body_str});").unwrap();
                }
            } else {
                let rewritten = analyze::rewrite_prime_as_post_state(&constraint.expr);
                let body = expr_translator::translate_with_ir(&rewritten, ir);
                writeln!(out, "    assert!({body});").unwrap();
            }
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        // Use temporal classification for test name prefix
        let temporal_kind = analyze::expr_temporal_kind(&constraint.expr);
        let test_prefix = match temporal_kind {
            Some(analyze::TemporalKind::Liveness) => "liveness",
            Some(analyze::TemporalKind::PastInvariant) => "past_invariant",
            Some(analyze::TemporalKind::PastLiveness) => "past_liveness",
            Some(analyze::TemporalKind::Step) => "step",
            Some(analyze::TemporalKind::Binary) => "temporal",
            _ => "invariant",
        };
        let test_name = format!("{}_{}", test_prefix, to_snake_case(&fact_name));
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        // Skip tests that reference enum variants (not standalone types in Rust)
        if params.iter().any(|(_, tname)| variant_names_set.contains(tname) || enum_parents.contains(tname)) {
            continue;
        }
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        // Check the analyzed constraints for the sigs referenced by this fact
        let sig_constraints: Vec<&analyze::ConstraintInfo> = params.iter()
            .flat_map(|(_, tname)| {
                all_constraints.iter().filter(move |c| match c {
                    analyze::ConstraintInfo::Presence { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::CardinalityBound { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::NoSelfRef { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::Acyclic { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::Membership { sig_name, .. } => sig_name == tname,
                    _ => false,
                })
            })
            .collect();

        use crate::analyze::guarantee::{can_guarantee_by_type, Guarantee, TargetLang};

        // If ALL related constraints are FullyByType, skip this test
        let all_fully = !sig_constraints.is_empty() && sig_constraints.iter().all(|c| {
            can_guarantee_by_type(c, TargetLang::Rust) == Guarantee::FullyByType
        });

        if all_fully {
            writeln!(out, "// Type-guaranteed: {} — no test needed (Rust type system encodes this)", fact_name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        // Check if any constraint is PartiallyByType → generate regression test
        let any_partial = sig_constraints.iter().any(|c| {
            can_guarantee_by_type(c, TargetLang::Rust) == Guarantee::PartiallyByType
        });

        if any_partial {
            writeln!(out, "/// @regression Partially type-guaranteed — regression test only.").unwrap();
        }
        // Add temporal kind annotation for temporal tests
        if let Some(kind) = temporal_kind {
            let annotation = match kind {
                analyze::TemporalKind::Invariant => "@temporal Invariant: property must hold in all states",
                analyze::TemporalKind::Liveness => "@temporal Liveness property — cannot be fully verified at runtime; static test approximates via implies",
                analyze::TemporalKind::PastInvariant => "@temporal PastInvariant: property must have held in all past states",
                analyze::TemporalKind::PastLiveness => "@temporal PastLiveness property — cannot be fully verified at runtime; static test approximates via implies",
                analyze::TemporalKind::Step => "@temporal Step: relates adjacent states",
                analyze::TemporalKind::Binary => "@temporal Binary: temporal binary constraint",
            };
            writeln!(out, "/// {annotation}").unwrap();
        }

        // Binary temporal: static test cannot meaningfully assert the body
        // (e.g. `p until q` requires a trace, not a snapshot)
        if temporal_kind == Some(analyze::TemporalKind::Binary) {
            let op_label = if let Some((op, _, _)) = analyze::find_temporal_binary(&constraint.expr) {
                match op {
                    TemporalBinaryOp::Until => "until",
                    TemporalBinaryOp::Since => "since",
                    TemporalBinaryOp::Release => "release",
                    TemporalBinaryOp::Triggered => "triggered",
                }
            } else { "binary" };
            let snake_name = to_snake_case(&fact_name);
            writeln!(out, "#[test]").unwrap();
            writeln!(out, "fn {test_name}() {{").unwrap();
            writeln!(out, "    // binary temporal: requires trace-based verification; see check_{op_label}_{snake_name}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        } else if matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            // Liveness/past_liveness: cannot be verified with single snapshot
            let kind_label = if temporal_kind == Some(analyze::TemporalKind::Liveness) {
                "liveness" } else { "past_liveness" };
            let snake_name = to_snake_case(&fact_name);
            writeln!(out, "#[test]").unwrap();
            writeln!(out, "fn {test_name}() {{").unwrap();
            writeln!(out, "    // {kind_label}: requires trace-based verification; see check_{kind_label}_{snake_name}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        } else {
        // Detect ownership facts: `all x: A | some y: B | x in y.field`
        // These need linked fixture setup where B.field contains x.
        let ownership = detect_ownership_pattern(&constraint.expr, ir);

        writeln!(out, "#[test]").unwrap();
        writeln!(out, "fn {test_name}() {{").unwrap();
        if let Some((owned_var, owner_var, _owner_type, field_name)) = &ownership {
            // Generate linked setup: create owned item, insert into owner's field
            let owned_param = params.iter().find(|(p, _)| p == owned_var);
            let owner_param = params.iter().find(|(p, _)| p == owner_var);
            if let (Some((opname, otname)), Some((cpname, ctname))) = (owned_param, owner_param) {
                let owned_snake = to_snake_case(otname);
                let owner_snake = to_snake_case(ctname);
                writeln!(out, "    let item = default_{owned_snake}();").unwrap();
                writeln!(out, "    let mut owner = default_{owner_snake}();").unwrap();
                writeln!(out, "    owner.{field_name}.insert(item.clone());").unwrap();
                writeln!(out, "    let {opname}: Vec<{otname}> = vec![item];").unwrap();
                writeln!(out, "    let {cpname}: Vec<{ctname}> = vec![owner];").unwrap();
                // Emit remaining params normally
                for (pname, tname) in &params {
                    if pname == opname || pname == cpname { continue; }
                    if has_fixture.contains(tname) {
                        let snake = to_snake_case(tname);
                        writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                    } else {
                        writeln!(out, "    let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                    }
                }
            }
        } else {
            // Extract primary sig and quantifier kind for fixture selection.
            let (quant_kind, primary_sigs): (Option<&crate::parser::ast::QuantKind>, HashSet<String>) =
                if let Some((kind, bindings, _body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                    (Some(kind), bindings.iter().flat_map(|b| {
                        if let Expr::VarRef(name) = &b.domain { Some(name.clone()) } else { None }
                    }).collect())
                } else {
                    (None, params.iter().map(|(_, t)| t.clone()).collect())
                };
            let is_existential = matches!(quant_kind, Some(crate::parser::ast::QuantKind::Some));
            for (pname, tname) in &params {
                if primary_sigs.contains(tname) && has_fixture.contains(tname) {
                    if is_existential {
                        let snake = to_snake_case(tname);
                        writeln!(out, "    let {pname} = all_{snake}s();").unwrap();
                    } else {
                        let snake = to_snake_case(tname);
                        writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                    }
                } else if has_existential_fixture(tname, ir) {
                    // Secondary sig with existential facts: use all_{plural}s()
                    let snake = to_snake_case(tname);
                    writeln!(out, "    let {pname} = all_{snake}s();").unwrap();
                } else if has_fixture.contains(tname) {
                    // Secondary sig with fixture: use default to avoid empty collection
                    let snake = to_snake_case(tname);
                    writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                } else {
                    writeln!(out, "    let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                }
            }
        }
        writeln!(out, "    assert!({body});").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
        } // end non-binary temporal

        // Generate trace checker functions for temporal constraints (⑤ liveness, ④ binary temporal)
        if let Some(kind) = temporal_kind {
            let snake_name = to_snake_case(&fact_name);
            match kind {
                analyze::TemporalKind::Liveness | analyze::TemporalKind::PastLiveness => {
                    let kind_label = if kind == analyze::TemporalKind::Liveness {
                        "liveness" } else { "past_liveness" };
                    let semantics = if kind == analyze::TemporalKind::Liveness {
                        "property holds in at least one future state"
                    } else {
                        "property held in at least one past state"
                    };
                    writeln!(out, "/// Trace checker for {kind_label}: {semantics}.").unwrap();
                    writeln!(out, "#[allow(dead_code)]").unwrap();
                    // Generate trace fn signature with tuple params
                    if params.len() == 1 {
                        let (pname, tname) = &params[0];
                        writeln!(out, "fn check_{kind_label}_{snake_name}(trace: &[Vec<{tname}>]) -> bool {{").unwrap();
                        writeln!(out, "    trace.iter().any(|{pname}| {{").unwrap();
                    } else {
                        let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("Vec<{t}>")).collect();
                        let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                        writeln!(out, "fn check_{kind_label}_{snake_name}(trace: &[({})]) -> bool {{", tuple_types.join(", ")).unwrap();
                        writeln!(out, "    trace.iter().any(|({})| {{", tuple_names.join(", ")).unwrap();
                    }
                    writeln!(out, "        {body}").unwrap();
                    writeln!(out, "    }})").unwrap();
                    writeln!(out, "}}").unwrap();
                    writeln!(out).unwrap();
                }
                analyze::TemporalKind::Binary => {
                    // Extract left/right sub-expressions for until/since
                    if let Some((op, left, right)) = analyze::find_temporal_binary(&constraint.expr) {
                        let left_body = expr_translator::translate_with_ir(left, ir);
                        let right_body = expr_translator::translate_with_ir(right, ir);
                        let op_name = match op {
                            TemporalBinaryOp::Until => "until",
                            TemporalBinaryOp::Since => "since",
                            TemporalBinaryOp::Release => "release",
                            TemporalBinaryOp::Triggered => "triggered",
                        };
                        let semantics = match op {
                            TemporalBinaryOp::Until => "left holds until right becomes true",
                            TemporalBinaryOp::Since => "left has held since right was true",
                            TemporalBinaryOp::Release => "right holds until left releases it",
                            TemporalBinaryOp::Triggered => "left triggers right",
                        };
                        writeln!(out, "/// Trace checker for {op_name}: {semantics}.").unwrap();
                        writeln!(out, "#[allow(dead_code)]").unwrap();
                        if params.len() == 1 {
                            let (pname, tname) = &params[0];
                            writeln!(out, "fn check_{op_name}_{snake_name}(trace: &[Vec<{tname}>]) -> bool {{").unwrap();
                            match op {
                                TemporalBinaryOp::Until => {
                                    writeln!(out, "    match trace.iter().position(|{pname}| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "        Some(pos) => trace[..pos].iter().all(|{pname}| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "        None => false,").unwrap();
                                    writeln!(out, "    }}").unwrap();
                                }
                                TemporalBinaryOp::Since => {
                                    writeln!(out, "    match trace.iter().rposition(|{pname}| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "        Some(pos) => trace[pos..].iter().all(|{pname}| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "        None => false,").unwrap();
                                    writeln!(out, "    }}").unwrap();
                                }
                                TemporalBinaryOp::Release => {
                                    // Release: right holds until (and including when) left becomes true
                                    writeln!(out, "    match trace.iter().position(|{pname}| {{ {left_body} }}) {{").unwrap();
                                    writeln!(out, "        Some(pos) => trace[..=pos].iter().all(|{pname}| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "        None => trace.iter().all(|{pname}| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "    }}").unwrap();
                                }
                                TemporalBinaryOp::Triggered => {
                                    // Triggered: if right ever holds, left must hold at or before that point
                                    writeln!(out, "    trace.iter().enumerate().all(|(i, {pname})| {{").unwrap();
                                    writeln!(out, "        if {right_body} {{ trace[..=i].iter().any(|{pname}| {{ {left_body} }}) }} else {{ true }}").unwrap();
                                    writeln!(out, "    }})").unwrap();
                                }
                            }
                        } else {
                            let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("Vec<{t}>")).collect();
                            let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                            let pnames = tuple_names.join(", ");
                            writeln!(out, "fn check_{op_name}_{snake_name}(trace: &[({})]) -> bool {{", tuple_types.join(", ")).unwrap();
                            match op {
                                TemporalBinaryOp::Until => {
                                    writeln!(out, "    match trace.iter().position(|({pnames})| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "        Some(pos) => trace[..pos].iter().all(|({pnames})| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "        None => false,").unwrap();
                                    writeln!(out, "    }}").unwrap();
                                }
                                TemporalBinaryOp::Since => {
                                    writeln!(out, "    match trace.iter().rposition(|({pnames})| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "        Some(pos) => trace[pos..].iter().all(|({pnames})| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "        None => false,").unwrap();
                                    writeln!(out, "    }}").unwrap();
                                }
                                TemporalBinaryOp::Release => {
                                    writeln!(out, "    match trace.iter().position(|({pnames})| {{ {left_body} }}) {{").unwrap();
                                    writeln!(out, "        Some(pos) => trace[..=pos].iter().all(|({pnames})| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "        None => trace.iter().all(|({pnames})| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "    }}").unwrap();
                                }
                                TemporalBinaryOp::Triggered => {
                                    writeln!(out, "    trace.iter().enumerate().all(|(i, ({pnames}))| {{").unwrap();
                                    writeln!(out, "        if {right_body} {{ trace[..=i].iter().any(|({pnames})| {{ {left_body} }}) }} else {{ true }}").unwrap();
                                    writeln!(out, "    }})").unwrap();
                                }
                            }
                        }
                        writeln!(out, "}}").unwrap();
                        writeln!(out).unwrap();
                    }
                }
                _ => {} // Invariant, PastInvariant, Step — static tests are sufficient
            }
        }
    }

    // Boundary value tests — use boundary fixtures with inlined constraints (Feature 5)
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        // Check if any param type has boundary fixtures
        let has_boundary = params.iter().any(|(_, tname)| {
            ir.structures.iter().any(|s| {
                s.name == *tname && !s.is_enum && s.fields.iter().any(|f| {
                    matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                        && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                })
            })
        });

        if has_boundary {
            let test_name = format!("boundary_{}", to_snake_case(&fact_name));
            // For boundary tests, use default fixtures for container types
            // (ownership pattern linking is handled by populated set fields in fixtures)
            writeln!(out, "#[test]").unwrap();
            writeln!(out, "fn {test_name}() {{").unwrap();
            for (pname, tname) in &params {
                    let snake = to_snake_case(tname);
                    let has_b = ir.structures.iter().any(|s| {
                        s.name == *tname && s.fields.iter().any(|f| {
                            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                        })
                    });
                    if has_b {
                        writeln!(out, "    let {pname}: Vec<{tname}> = vec![boundary_{snake}()];").unwrap();
                    } else if has_fixture.contains(tname) {
                        writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                    } else {
                        writeln!(out, "    let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                    }
                }
            writeln!(out, "    assert!({body}, \"boundary values should satisfy invariant\");").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();

            // Negative test
            let test_name = format!("invalid_{}", to_snake_case(&fact_name));
            writeln!(out, "#[test]").unwrap();
            writeln!(out, "fn {test_name}() {{").unwrap();
            for (pname, tname) in &params {
                let snake = to_snake_case(tname);
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "    let {pname}: Vec<{tname}> = vec![invalid_{snake}()];").unwrap();
                } else if has_fixture.contains(tname) {
                    writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                } else {
                    writeln!(out, "    let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                }
            }
            writeln!(out, "    assert!(!({body}), \"invalid values should violate invariant\");").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Cross-tests: for each (fact, operation) pair, verify fact is preserved
    if !ir.constraints.is_empty() && !ir.operations.is_empty() {
        writeln!(out, "// --- Cross-tests: fact × operation ---").unwrap();
        writeln!(out).unwrap();

        for constraint in &ir.constraints {
            let fact_name = match &constraint.name {
                Some(name) => name.clone(),
                None => continue,
            };

            for op in &ir.operations {
                let op_name = to_snake_case(&op.name);
                let test_name = format!("{}_preserved_after_{}", to_snake_case(&fact_name), op_name);

                writeln!(out, "#[test]").unwrap();
                writeln!(out, "#[ignore]").unwrap();
                writeln!(out, "fn {test_name}() {{").unwrap();
                writeln!(
                    out,
                    "        // Verify that {} holds after {}",
                    fact_name, op.name
                )
                .unwrap();
                writeln!(
                    out,
                    "        // pre: assert!(/* {fact_name} constraint */);",
                )
                .unwrap();
                writeln!(
                    out,
                    "        // {op_name}(...);"
                )
                .unwrap();
                writeln!(
                    out,
                    "        // post: assert!(/* {fact_name} constraint */);",
                )
                .unwrap();
                writeln!(
                    out,
                    "        todo!(\"oxidtr: implement cross-test {test_name}\");"
                )
                .unwrap();
                writeln!(out, "}}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    // --- Anomaly tests: edge-case tests for anomaly patterns ---
    let anomalies = analyze::detect_anomalies(ir);
    let has_any_anomaly = !anomalies.is_empty();
    if has_any_anomaly {
        writeln!(out, "// --- Anomaly tests: edge-case coverage ---").unwrap();
        writeln!(out).unwrap();

        // Group anomalies by sig
        let mut anomaly_sigs: HashMap<String, Vec<&analyze::AnomalyPattern>> = HashMap::new();
        for a in &anomalies {
            let sig = match a {
                analyze::AnomalyPattern::UnconstrainedField { sig_name, .. } => sig_name,
                analyze::AnomalyPattern::UnboundedCollection { sig_name, .. } => sig_name,
                analyze::AnomalyPattern::UnguardedSelfRef { sig_name, .. } => sig_name,
            };
            anomaly_sigs.entry(sig.clone()).or_default().push(a);
        }

        for (sig_name, patterns) in &anomaly_sigs {
            if !has_fixture.contains(sig_name) { continue; }
            let snake = to_snake_case(sig_name);

            for pattern in patterns {
                match pattern {
                    analyze::AnomalyPattern::UnconstrainedField { field_name, .. } => {
                        let field_snake = to_snake_case(field_name);
                        writeln!(out, "/// Anomaly: field `{field_name}` is not constrained by any fact.").unwrap();
                        writeln!(out, "#[test]").unwrap();
                        writeln!(out, "fn anomaly_unconstrained_{snake}_{field_snake}() {{").unwrap();
                        writeln!(out, "    let instance = default_{snake}();").unwrap();
                        writeln!(out, "    // {sig_name}.{field_name} is not constrained — verify it is handled").unwrap();
                        writeln!(out, "    let _ = &instance.{field_name};").unwrap();
                        writeln!(out, "}}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        let field_snake = to_snake_case(field_name);
                        writeln!(out, "/// Anomaly: `{field_name}` has no cardinality upper bound.").unwrap();
                        writeln!(out, "#[test]").unwrap();
                        writeln!(out, "fn anomaly_empty_{snake}_{field_snake}() {{").unwrap();
                        writeln!(out, "    let instance = anomaly_empty_{snake}();").unwrap();
                        writeln!(out, "    // Verify invariants hold even with empty collection").unwrap();
                        writeln!(out, "    let _ = &instance.{field_name};").unwrap();
                        writeln!(out, "}}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        let field_snake = to_snake_case(field_name);
                        writeln!(out, "/// Anomaly: self-referential field `{field_name}` without NoSelfRef/Acyclic guard.").unwrap();
                        writeln!(out, "#[test]").unwrap();
                        writeln!(out, "fn anomaly_self_ref_{snake}_{field_snake}() {{").unwrap();
                        writeln!(out, "    let instance = default_{snake}();").unwrap();
                        writeln!(out, "    // Self-referential field without guard — check for safety").unwrap();
                        writeln!(out, "    let _ = &instance.{field_name};").unwrap();
                        writeln!(out, "}}").unwrap();
                        writeln!(out).unwrap();
                    }
                }
            }
        }
    }

    // --- Coverage tests: pairwise fact combinations ---
    let coverage = analyze::fact_coverage(ir);
    if !coverage.pairwise.is_empty() {
        writeln!(out, "// --- Coverage tests: fact × fact pairwise ---").unwrap();
        writeln!(out).unwrap();

        let mut cover_names_seen: HashSet<String> = HashSet::new();
        for pair in &coverage.pairwise {
            if !has_fixture.contains(&pair.sig_name) { continue; }

            let fact_a_snake = to_snake_case(&pair.fact_a);
            let fact_b_snake = to_snake_case(&pair.fact_b);
            let test_name = format!("cover_{fact_a_snake}_x_{fact_b_snake}");

            // Skip duplicate test names (same fact pair from different sig perspectives)
            if !cover_names_seen.insert(test_name.clone()) { continue; }

            // Find the constraint nodes for both facts
            let constraint_a = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_a));
            let constraint_b = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_b));

            let (Some(ca), Some(cb)) = (constraint_a, constraint_b) else { continue; };

            let body_a = expr_translator::translate_with_ir(&ca.expr, ir);
            let body_b = expr_translator::translate_with_ir(&cb.expr, ir);

            // Extract all params from both facts to declare all needed variables
            let params_a = expr_translator::extract_params(&ca.expr, &sig_names);
            let params_b = expr_translator::extract_params(&cb.expr, &sig_names);
            let mut all_params: Vec<(String, String)> = Vec::new();
            let mut param_names_seen: HashSet<String> = HashSet::new();
            for (pname, tname) in params_a.iter().chain(params_b.iter()) {
                if param_names_seen.insert(pname.clone()) {
                    all_params.push((pname.clone(), tname.clone()));
                }
            }

            // Skip if any param is an enum variant
            if all_params.iter().any(|(_, tname)| variant_names_set.contains(tname) || enum_parents.contains(tname)) {
                continue;
            }

            // Detect vacuously-true tests: domains without fixtures are empty
            let empty_domains: HashSet<String> = all_params.iter()
                .filter(|(_, tname)| !has_fixture.contains(tname))
                .flat_map(|(_, tname)| {
                    // The domain is the sig name (tname), check if any quantifier uses it
                    std::iter::once(tname.clone())
                })
                .collect();
            let vacuous_a = analyze::is_vacuously_true(&ca.expr, &empty_domains);
            let vacuous_b = analyze::is_vacuously_true(&cb.expr, &empty_domains);
            let is_vacuous = vacuous_a || vacuous_b;

            writeln!(out, "/// Coverage: {} × {}", pair.fact_a, pair.fact_b).unwrap();
            if is_vacuous {
                writeln!(out, "/// WARNING: vacuously true — fixture makes quantifier domain empty").unwrap();
            }
            writeln!(out, "#[test]").unwrap();
            writeln!(out, "#[ignore]").unwrap();
            writeln!(out, "fn {test_name}() {{").unwrap();
            for (pname, tname) in &all_params {
                let snake = to_snake_case(tname);
                if has_fixture.contains(tname) {
                    writeln!(out, "    let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                } else {
                    writeln!(out, "    let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                }
            }
            writeln!(out, "    assert!({body_a}, \"fact {} should hold\");", pair.fact_a).unwrap();
            writeln!(out, "    assert!({body_b}, \"fact {} should hold\");", pair.fact_b).unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

/// Generate newtype wrappers for sigs that have named constraints.
/// For each (constraint_name, sig_name) pair where the named constraint references the sig,
/// generates `ValidatedSig(Sig)` + `TryFrom<Sig> for ValidatedSig`.
fn generate_newtypes(ir: &OxidtrIR) -> String {
    let sig_names = collect_sig_names(ir);
    let mut out = String::new();

    // Collect (fact_name, sig_name) pairs where the fact has a Comparison or Disjoint pattern
    let mut newtype_pairs: Vec<(String, String)> = Vec::new();
    let all_constraints = analyze::analyze(ir);
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        // Check if this constraint contains a Comparison
        if expr_has_comparison(&constraint.expr) {
            let params = expr_translator::extract_params(&constraint.expr, &sig_names);
            for (_pname, tname) in &params {
                newtype_pairs.push((fact_name.clone(), tname.clone()));
            }
            continue;
        }
        // Check if this constraint contains a Disjoint pattern (no (A & B))
        if expr_has_disjoint_pattern(&constraint.expr) {
            // For Disjoint, extract sig name from the analyzed constraints
            for c in &all_constraints {
                if let analyze::ConstraintInfo::Disjoint { sig_name, .. } = c {
                    if !sig_name.is_empty() {
                        newtype_pairs.push((fact_name.clone(), sig_name.clone()));
                    }
                }
            }
        }
        // Check if this constraint contains an Exhaustive pattern
        for c in &all_constraints {
            if let analyze::ConstraintInfo::Exhaustive { sig_name, .. } = c {
                if !sig_name.is_empty() {
                    newtype_pairs.push((fact_name.clone(), sig_name.clone()));
                }
            }
        }
    }

    if newtype_pairs.is_empty() {
        return String::new();
    }

    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use super::models::*;").unwrap();
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use super::fixtures::*;").unwrap();

    // Check if TC functions are needed → import helpers
    let needs_helpers = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr));
    if needs_helpers {
        writeln!(out, "#[allow(unused_imports)]").unwrap();
        writeln!(out, "use super::helpers::*;").unwrap();
    }
    writeln!(out).unwrap();

    // Deduplicate: only one newtype per sig (first fact wins)
    newtype_pairs.sort();
    newtype_pairs.dedup();
    // Remove duplicate sig_names, keeping only the first (fact, sig) for each sig
    {
        let mut seen_sigs = HashSet::new();
        newtype_pairs.retain(|(_, sig)| seen_sigs.insert(sig.clone()));
    }
    // Filter out enum variants — they are not standalone types, so newtypes make no sense
    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();
    newtype_pairs.retain(|(_, sig)| !variant_names.contains(sig) && !enum_parents.contains(sig));

    // Build constraint info for field-level range checks
    let all_constraints = analyze::analyze(ir);

    for (fact_name, sig_name) in &newtype_pairs {
        let newtype_name = format!("Validated{sig_name}");

        // Find the constraint to get the inlined expression and params
        let constraint = ir.constraints.iter()
            .find(|c| c.name.as_deref() == Some(fact_name.as_str()));
        let inlined_info = constraint.map(|c| {
            let body = expr_translator::translate_with_ir(&c.expr, ir);
            let params = expr_translator::extract_params(&c.expr, &sig_names);
            (body, params)
        });

        // Collect field-level cardinality bounds for this sig
        let field_checks: Vec<(String, Option<usize>, Option<usize>)> = all_constraints.iter()
            .filter_map(|c| {
                if let analyze::ConstraintInfo::CardinalityBound { sig_name: s, field_name, bound } = c {
                    if s == sig_name {
                        let (min, max) = match bound {
                            analyze::BoundKind::Exact(n) => (Some(*n), Some(*n)),
                            analyze::BoundKind::AtMost(n) => (None, Some(*n)),
                            analyze::BoundKind::AtLeast(n) => (Some(*n), None),
                        };
                        return Some((field_name.clone(), min, max));
                    }
                }
                None
            })
            .collect();

        // Collect NoSelfRef fields for this sig
        let no_self_ref_fields: Vec<String> = all_constraints.iter()
            .filter_map(|c| {
                if let analyze::ConstraintInfo::NoSelfRef { sig_name: s, field_name } = c {
                    if s == sig_name {
                        return Some(field_name.clone());
                    }
                }
                None
            })
            .collect();

        writeln!(out, "/// Newtype wrapper: {sig_name} validated by {fact_name}.").unwrap();
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, Hash)]").unwrap();
        writeln!(out, "pub struct {newtype_name}(pub {sig_name});").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "impl TryFrom<{sig_name}> for {newtype_name} {{").unwrap();
        writeln!(out, "    type Error = &'static str;").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "    fn try_from(value: {sig_name}) -> Result<Self, Self::Error> {{").unwrap();

        // Generate concrete field-level range checks
        for (field_name, min, max) in &field_checks {
            if let Some(n) = min {
                writeln!(out, "        if value.{field_name}.len() < {n} {{").unwrap();
                writeln!(out, "            return Err(\"{fact_name}: {field_name} has fewer than {n} elements\");").unwrap();
                writeln!(out, "        }}").unwrap();
            }
            if let Some(n) = max {
                writeln!(out, "        if value.{field_name}.len() > {n} {{").unwrap();
                writeln!(out, "            return Err(\"{fact_name}: {field_name} has more than {n} elements\");").unwrap();
                writeln!(out, "        }}").unwrap();
            }
        }

        // NoSelfRef field checks
        if let Some(structure) = ir.structures.iter().find(|s| s.name == *sig_name) {
            for field_name in &no_self_ref_fields {
                if let Some(f) = structure.fields.iter().find(|f| f.name == *field_name) {
                    match f.mult {
                        Multiplicity::Lone => {
                            writeln!(out, "        if value.{field_name}.as_ref().map_or(false, |f| f.as_ref() == &value) {{").unwrap();
                            writeln!(out, "            return Err(\"{field_name} must not reference self\");").unwrap();
                            writeln!(out, "        }}").unwrap();
                        }
                        Multiplicity::One => {
                            writeln!(out, "        if *value.{field_name} == value {{").unwrap();
                            writeln!(out, "            return Err(\"{field_name} must not reference self\");").unwrap();
                            writeln!(out, "        }}").unwrap();
                        }
                        _ => {}
                    }
                }
            }
        }

        // Disj uniqueness checks for seq fields
        {
            let disj = analyze::disj_fields(ir);
            if let Some(structure) = ir.structures.iter().find(|s| s.name == *sig_name) {
                for (dsig, dfield) in &disj {
                    if dsig == sig_name {
                        if let Some(f) = structure.fields.iter().find(|f| f.name == *dfield) {
                            if f.mult == Multiplicity::Seq {
                                writeln!(out, "        {{").unwrap();
                                writeln!(out, "            let mut seen = std::collections::HashSet::new();").unwrap();
                                writeln!(out, "            if !value.{dfield}.iter().all(|e| seen.insert(e)) {{").unwrap();
                                writeln!(out, "                return Err(\"{dfield} must not contain duplicates (disj constraint)\");").unwrap();
                                writeln!(out, "            }}").unwrap();
                                writeln!(out, "        }}").unwrap();
                            }
                        }
                    }
                }
            }
        }

        // Acyclic field checks (walk the chain, detect if value is reachable from itself)
        {
            let acyclic_fields: Vec<String> = all_constraints.iter()
                .filter_map(|c| {
                    if let analyze::ConstraintInfo::Acyclic { sig_name: s, field_name } = c {
                        if s == sig_name {
                            return Some(field_name.clone());
                        }
                    }
                    None
                })
                .collect();
            if let Some(structure) = ir.structures.iter().find(|s| s.name == *sig_name) {
                for field_name in &acyclic_fields {
                    if let Some(f) = structure.fields.iter().find(|f| f.name == *field_name) {
                        if f.mult == Multiplicity::Lone && f.target == *sig_name {
                            writeln!(out, "        {{").unwrap();
                            writeln!(out, "            let mut cur = value.{field_name}.as_deref();").unwrap();
                            writeln!(out, "            while let Some(node) = cur {{").unwrap();
                            writeln!(out, "                if node == &value {{").unwrap();
                            writeln!(out, "                    return Err(\"{field_name} must not form a cycle\");").unwrap();
                            writeln!(out, "                }}").unwrap();
                            writeln!(out, "                cur = node.{field_name}.as_deref();").unwrap();
                            writeln!(out, "            }}").unwrap();
                            writeln!(out, "        }}").unwrap();
                        }
                    }
                }
            }
        }

        // FieldOrdering checks
        for c in &all_constraints {
            if let analyze::ConstraintInfo::FieldOrdering { sig_name: s, left_field, op, right_field } = c {
                if s == sig_name {
                    let rust_op = match op {
                        CompareOp::Lt => "<",
                        CompareOp::Gt => ">",
                        CompareOp::Lte => "<=",
                        CompareOp::Gte => ">=",
                        _ => continue,
                    };
                    let negated = match op {
                        CompareOp::Lt => ">=",
                        CompareOp::Gt => "<=",
                        CompareOp::Lte => ">",
                        CompareOp::Gte => "<",
                        _ => continue,
                    };
                    writeln!(out, "        if value.{left_field} {negated} value.{right_field} {{").unwrap();
                    writeln!(out, "            return Err(\"{left_field} must be {rust_op} {right_field}\");").unwrap();
                    writeln!(out, "        }}").unwrap();
                }
            }
        }

        // Disjoint checks
        for c in &all_constraints {
            if let analyze::ConstraintInfo::Disjoint { sig_name: s, left, right } = c {
                if s == sig_name {
                    let left_field = left.rsplit('.').next().unwrap_or(left);
                    let right_field = right.rsplit('.').next().unwrap_or(right);
                    writeln!(out, "        {{").unwrap();
                    writeln!(out, "            let left_set: std::collections::HashSet<_> = value.{left_field}.iter().collect();").unwrap();
                    writeln!(out, "            if value.{right_field}.iter().any(|e| left_set.contains(e)) {{").unwrap();
                    writeln!(out, "                return Err(\"{left_field} and {right_field} must not overlap (disjoint constraint)\");").unwrap();
                    writeln!(out, "            }}").unwrap();
                    writeln!(out, "        }}").unwrap();
                }
            }
        }

        // Exhaustive checks — validate_exhaustive helper generated below
        for c in &all_constraints {
            if let analyze::ConstraintInfo::Exhaustive { sig_name: s, categories } = c {
                if s == sig_name {
                    let cats = categories.join(", ");
                    let fn_name = format!("validate_exhaustive_{}", to_snake_case(sig_name));
                    writeln!(out, "        // Exhaustive: must belong to one of [{cats}]").unwrap();
                    writeln!(out, "        // Call {fn_name}(&value, &[...]) at integration level").unwrap();
                }
            }
        }

        // Implication checks
        for c in &all_constraints {
            if let analyze::ConstraintInfo::Implication { sig_name: s, condition, consequent } = c {
                if s == sig_name {
                    let cond = translate_validator_expr_rust(condition, sig_name, ir);
                    let cons = translate_validator_expr_rust(consequent, sig_name, ir);
                    let desc = format!("{} implies {}", analyze::describe_expr(condition), analyze::describe_expr(consequent));
                    // Parenthesize the condition so disjunctive antecedents
                    // (`(A or B) implies C`) keep their grouping when combined
                    // with the `&& !(cons)` check — Rust's `&&` binds tighter
                    // than `||` so bare `A || B && !(C)` misparses.
                    writeln!(out, "        if ({cond}) && !({cons}) {{").unwrap();
                    writeln!(out, "            return Err(\"{}\");", desc.replace('"', "\\\"")).unwrap();
                    writeln!(out, "        }}").unwrap();
                }
            }
        }

        // Iff checks
        for c in &all_constraints {
            if let analyze::ConstraintInfo::Iff { sig_name: s, left, right } = c {
                if s == sig_name {
                    let l = translate_validator_expr_rust(left, sig_name, ir);
                    let r = translate_validator_expr_rust(right, sig_name, ir);
                    let desc = format!("{} iff {}", analyze::describe_expr(left), analyze::describe_expr(right));
                    writeln!(out, "        if ({l}) != ({r}) {{").unwrap();
                    writeln!(out, "            return Err(\"{}\");", desc.replace('"', "\\\"")).unwrap();
                    writeln!(out, "        }}").unwrap();
                }
            }
        }

        // Prohibition checks
        for c in &all_constraints {
            if let analyze::ConstraintInfo::Prohibition { sig_name: s, condition } = c {
                if s == sig_name {
                    let cond = translate_validator_expr_rust(condition, sig_name, ir);
                    let desc = analyze::describe_expr(condition);
                    writeln!(out, "        if {cond} {{").unwrap();
                    writeln!(out, "            return Err(\"prohibited: {}\");", desc.replace('"', "\\\"")).unwrap();
                    writeln!(out, "        }}").unwrap();
                }
            }
        }

        // Inline the constraint expression directly instead of calling invariant function
        // Bind param names with type annotations for closure inference
        // Only the param matching sig_name gets value.clone(); others get empty Vec
        if let Some((body, params)) = &inlined_info {
            for (pname, tname) in params {
                if tname == sig_name {
                    writeln!(out, "        let {pname}: Vec<{tname}> = vec![value.clone()];").unwrap();
                } else {
                    writeln!(out, "        let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                }
            }
            writeln!(out, "        if {body} {{").unwrap();
        } else {
            writeln!(out, "        if true {{").unwrap();
        }
        writeln!(out, "            Ok({newtype_name}(value))").unwrap();
        writeln!(out, "        }} else {{").unwrap();
        writeln!(out, "            Err(\"{fact_name} invariant violated\")").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    // Generate standalone validate_exhaustive functions for cross-sig membership
    let all_constraints_final = analyze::analyze(ir);
    let mut seen_exhaustive = HashSet::new();
    for c in &all_constraints_final {
        if let analyze::ConstraintInfo::Exhaustive { sig_name, categories } = c {
            if seen_exhaustive.insert(sig_name.clone()) {
                let fn_name = format!("validate_exhaustive_{}", to_snake_case(sig_name));
                let cats = categories.join(", ");
                writeln!(out, "/// Validates exhaustive constraint: must belong to one of [{cats}]").unwrap();
                writeln!(out, "pub fn {fn_name}(item: &{sig_name}, categories: &[&std::collections::BTreeSet<{sig_name}>]) -> Result<(), &'static str> {{").unwrap();
                writeln!(out, "    if categories.iter().any(|cat| cat.contains(item)) {{").unwrap();
                writeln!(out, "        Ok(())").unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(out, "        Err(\"must belong to one of [{cats}]\")").unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out, "}}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    out
}

/// Check if an expression contains a Comparison node.
fn expr_has_comparison(expr: &crate::parser::ast::Expr) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::Comparison { .. } => true,
        Expr::BinaryLogic { left, right, .. } | Expr::SetOp { left, right, .. }
        | Expr::Product { left, right } => {
            expr_has_comparison(left) || expr_has_comparison(right)
        }
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| expr_has_comparison(&b.domain))
                || expr_has_comparison(body)
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            expr_has_comparison(inner)
        }
        Expr::MultFormula { expr: inner, .. } => expr_has_comparison(inner),
        Expr::FieldAccess { base, .. } => expr_has_comparison(base),
        Expr::Prime(inner) => expr_has_comparison(inner),
        Expr::TemporalUnary { expr: inner, .. } => expr_has_comparison(inner),
        Expr::TemporalBinary { left, right, .. } => {
            expr_has_comparison(left) || expr_has_comparison(right)
        }
        Expr::FunApp { receiver, args, .. } => receiver.as_ref().map_or(false, |r| expr_has_comparison(r)) || args.iter().any(|a| expr_has_comparison(a)),
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
}

/// Check if expression matches the Disjoint pattern: `no (A & B)`
fn expr_has_disjoint_pattern(expr: &crate::parser::ast::Expr) -> bool {
    use crate::parser::ast::{Expr, QuantKind, SetOpKind};
    match expr {
        Expr::MultFormula { kind: QuantKind::No, expr } => {
            matches!(expr.as_ref(), Expr::SetOp { op: SetOpKind::Intersection, .. })
        }
        Expr::TemporalUnary { expr: inner, .. } => expr_has_disjoint_pattern(inner),
        _ => false,
    }
}

fn generate_tc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("tc_{}", tc.field_name);
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "/// Transitive closure traversal for {sig}.{field}.").unwrap();
    writeln!(out, "#[allow(dead_code)]").unwrap();

    match tc.mult {
        Multiplicity::Lone => {
            // lone self-ref: Option<Box<T>> chain traversal
            writeln!(out, "pub fn {fn_name}(start: &{sig}) -> Vec<{sig}> {{").unwrap();
            writeln!(out, "    let mut result = Vec::new();").unwrap();
            writeln!(out, "    let mut current = start.{field}.as_deref();").unwrap();
            writeln!(out, "    while let Some(next) = current {{").unwrap();
            writeln!(out, "        result.push(next.clone());").unwrap();
            writeln!(out, "        current = next.{field}.as_deref();").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::Set | Multiplicity::Seq => {
            // set/seq self-ref: BFS traversal
            writeln!(out, "pub fn {fn_name}(start: &{sig}) -> Vec<{sig}> {{").unwrap();
            writeln!(out, "    let mut result = Vec::new();").unwrap();
            writeln!(out, "    let mut queue: Vec<&{sig}> = start.{field}.iter().collect();").unwrap();
            writeln!(out, "    while let Some(next) = queue.pop() {{").unwrap();
            writeln!(out, "        if !result.contains(next) {{").unwrap();
            writeln!(out, "            result.push(next.clone());").unwrap();
            writeln!(out, "            queue.extend(next.{field}.iter());").unwrap();
            writeln!(out, "        }}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::One => {
            // one self-ref: infinite chain (unusual, generate with depth limit)
            writeln!(out, "pub fn {fn_name}(start: &{sig}) -> Vec<{sig}> {{").unwrap();
            writeln!(out, "    let mut result = Vec::new();").unwrap();
            writeln!(out, "    let mut current = &start.{field};").unwrap();
            writeln!(out, "    for _ in 0..1000 {{").unwrap();
            writeln!(out, "        if result.contains(current) {{ break; }}").unwrap();
            writeln!(out, "        result.push(current.clone());").unwrap();
            writeln!(out, "        current = &current.{field};").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    result").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
    writeln!(out).unwrap();
}

/// Find all (sig, field) pairs that participate in reference cycles and need Box<>.
/// This includes direct self-references (A.field → A) and mutual cycles (A → B → A).
pub fn find_cyclic_fields(ir: &OxidtrIR) -> HashSet<(String, String)> {
    // Build adjacency: sig name → set of sig names it references via fields
    let mut adj: HashMap<&str, Vec<(&str, &str)>> = HashMap::new(); // sig → [(target, field_name)]
    for s in &ir.structures {
        for f in &s.fields {
            adj.entry(s.name.as_str())
                .or_default()
                .push((f.target.as_str(), f.name.as_str()));
        }
    }

    // Find all sigs that participate in any cycle using DFS
    let sig_names: Vec<&str> = ir.structures.iter().map(|s| s.name.as_str()).collect();
    let mut in_cycle: HashSet<&str> = HashSet::new();

    for &start in &sig_names {
        // DFS from start, looking for paths back to start
        let mut visited = HashSet::new();
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) { continue; }
            if let Some(edges) = adj.get(current) {
                for &(target, _) in edges {
                    if target == start && visited.contains(start) {
                        in_cycle.insert(start);
                    }
                    stack.push(target);
                }
            }
        }
    }

    // Collect all fields on cyclic sigs that point to another sig in the cycle
    let mut result = HashSet::new();
    for s in &ir.structures {
        for f in &s.fields {
            if f.target == s.name {
                // Direct self-reference — always needs Box
                result.insert((s.name.clone(), f.name.clone()));
            } else if in_cycle.contains(s.name.as_str()) && in_cycle.contains(f.target.as_str()) {
                // Mutual cycle — needs Box to break the recursion
                result.insert((s.name.clone(), f.name.clone()));
            }
        }
    }
    result
}

/// Translate an Alloy expression to Rust for single-instance validator context.
fn translate_validator_expr_rust(expr: &Expr, sig_name: &str, ir: &OxidtrIR) -> String {
    use crate::parser::ast::{LogicOp, QuantKind};
    match expr {
        Expr::VarRef(name) => {
            if name == sig_name { return "value".to_string(); }
            // Qualify enum variants: MissingIdentity → DiagnosticRule::MissingIdentity
            if let Some(s) = ir.structures.iter().find(|s| s.name == *name) {
                if let Some(parent) = &s.parent {
                    if ir.structures.iter().any(|p| p.name == *parent && p.is_enum) {
                        return format!("{parent}::{name}");
                    }
                }
            }
            name.clone()
        }
        Expr::IntLiteral(n) => n.to_string(),
        Expr::FieldAccess { base, field } => {
            let base_str = translate_validator_expr_rust(base, sig_name, ir);
            if expr_translator::is_const_method_field(field, ir) {
                format!("{base_str}.{field}()")
            } else {
                format!("{base_str}.{field}")
            }
        }
        Expr::Comparison { op, left, right } => {
            let l = translate_validator_expr_rust(left, sig_name, ir);
            let r = translate_validator_expr_rust(right, sig_name, ir);
            let o = match op {
                CompareOp::Eq | CompareOp::NotEq => {
                    let op_str = if matches!(op, CompareOp::Eq) { "==" } else { "!=" };
                    if let Expr::FieldAccess { field: l_field, .. } = left.as_ref() {
                        if expr_translator::is_lone_field(l_field, ir) {
                            if let Expr::IntLiteral(n) = right.as_ref() {
                                return format!("{l} {op_str} Some({n}i64)");
                            }
                            return format!("{l}.as_ref() {op_str} Some(&{r})");
                        }
                    }
                    if let Expr::FieldAccess { field: r_field, .. } = right.as_ref() {
                        if expr_translator::is_lone_field(r_field, ir) {
                            if let Expr::IntLiteral(n) = left.as_ref() {
                                return format!("Some({n}i64) {op_str} {r}");
                            }
                            return format!("Some(&{l}) {op_str} {r}.as_ref()");
                        }
                    }
                    op_str
                }
                CompareOp::In => return format!("{r}.contains(&{l})"),
                CompareOp::Lt | CompareOp::Gt | CompareOp::Lte | CompareOp::Gte => {
                    let op_str = match op {
                        CompareOp::Lt => "<",
                        CompareOp::Gt => ">",
                        CompareOp::Lte => "<=",
                        CompareOp::Gte => ">=",
                        _ => unreachable!(),
                    };
                    if let Expr::FieldAccess { field: l_field, .. } = left.as_ref() {
                        if expr_translator::is_lone_field(l_field, ir) {
                            return format!("{l}.is_none_or(|v| v {op_str} {r})");
                        }
                    }
                    if let Expr::FieldAccess { field: r_field, .. } = right.as_ref() {
                        if expr_translator::is_lone_field(r_field, ir) {
                            return format!("{r}.is_none_or(|v| {l} {op_str} v)");
                        }
                    }
                    op_str
                }
            };
            format!("{l} {o} {r}")
        }
        Expr::BinaryLogic { op, left, right } => {
            let l = translate_validator_expr_rust(left, sig_name, ir);
            let r = translate_validator_expr_rust(right, sig_name, ir);
            match op {
                LogicOp::And => format!("{l} && {r}"),
                LogicOp::Or => format!("{l} || {r}"),
                LogicOp::Implies => format!("!({l}) || {r}"),
                LogicOp::Iff => format!("({l}) == ({r})"),
            }
        }
        Expr::Not(inner) => format!("!({})", translate_validator_expr_rust(inner, sig_name, ir)),
        Expr::MultFormula { kind, expr: inner } => {
            let e = translate_validator_expr_rust(inner, sig_name, ir);
            match kind {
                QuantKind::Some => format!("{e}.is_some()"),
                QuantKind::No => format!("{e}.is_none()"),
                _ => e,
            }
        }
        Expr::Cardinality(inner) => {
            format!("{}.len()", translate_validator_expr_rust(inner, sig_name, ir))
        }
        _ => format!("/* {} */true", analyze::describe_expr(expr)), // fallback
    }
}

fn generate_fixtures(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();
    let cyclic = find_cyclic_fields(ir);

    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use super::models::*;").unwrap();

    // Check if any fixture needs BTreeSet
    let needs_btreeset = ir.structures.iter().any(|s| {
        !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty()
            && s.fields.iter().any(|f| f.mult == Multiplicity::Set)
    });
    if needs_btreeset {
        writeln!(out, "#[allow(unused_imports)]").unwrap();
        writeln!(out, "use std::collections::BTreeSet;").unwrap();
    }
    writeln!(out).unwrap();

    // Build children map for enum default fixtures
    let children: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for s in &ir.structures {
            if let Some(parent) = &s.parent {
                map.entry(parent.clone()).or_default().push(s.name.clone());
            }
        }
        map
    };
    // Build struct map for looking up child sig fields
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s))
        .collect();

    // Collect variant names referenced in existential facts
    let existential_variants: HashSet<String> = ir.constraints.iter().flat_map(|c| {
        if let Some((kind, bindings, body)) = analyze::strip_outer_quantifier(&c.expr) {
            if matches!(kind, crate::parser::ast::QuantKind::Some) && bindings.len() == 1 {
                let var_name = &bindings[0].vars[0];
                return extract_equality_fields(body, var_name, ir)
                    .into_iter().map(|(_, v)| v).collect::<Vec<_>>();
            }
        }
        Vec::new()
    }).collect();

    // Generate enum default fixtures (prefer variant referenced in existential facts)
    for s in &ir.structures {
        if !s.is_enum { continue; }
        let variants = match children.get(&s.name) {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        let enum_snake = to_snake_case(&s.name);
        let is_unit = |v: &&String| struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty());
        // Prefer a unit variant that's referenced in existential facts
        let chosen = variants.iter()
            .find(|v| is_unit(v) && existential_variants.contains(&format!("{}::{}", s.name, v)))
            .or_else(|| variants.iter().find(|v| is_unit(v)));
        if let Some(variant) = chosen {
            writeln!(out, "/// Factory: default value for enum {}", s.name).unwrap();
            writeln!(out, "#[allow(dead_code)]").unwrap();
            writeln!(out, "pub fn default_{}() -> {} {{", enum_snake, s.name).unwrap();
            writeln!(out, "    {}::{}", s.name, variant).unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Collect which types have fixture factories (for populating set/seq fields).
    // Only populate set/seq fields when the target fixture doesn't create a cycle.
    let fixture_types: HashSet<String> = ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone())
        .collect();

    // Note: safe_set_targets is computed per-field below using is_safe_set_population()

    for s in &ir.structures {
        if variant_names.contains(&s.name) || s.is_enum { continue; }
        if is_native_type_alias(&s.name) { continue; }

        let struct_snake = to_snake_case(&s.name);

        // Unit struct (no fields): generate a trivial factory.
        if s.fields.is_empty() {
            writeln!(out, "/// Factory: default value for unit struct {}", s.name).unwrap();
            writeln!(out, "#[allow(dead_code)]").unwrap();
            writeln!(out, "pub fn default_{}() -> {} {{ {} }}", struct_snake, s.name, s.name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        writeln!(out, "/// Factory: create a default valid {}", s.name).unwrap();
        writeln!(out, "#[allow(dead_code)]").unwrap();
        writeln!(out, "pub fn default_{}() -> {} {{", struct_snake, s.name).unwrap();
        writeln!(out, "    {} {{", s.name).unwrap();
        for f in &s.fields {
            let val = if f.value_type.is_some() {
                "BTreeMap::new()".to_string()
            } else {
                let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                // Check for value bounds (e.g., s.field > 0 → use minimum valid value)
                let value_override = if is_native_type_alias(&f.target) {
                    if let Some(analyze::BoundKind::AtLeast(n)) = analyze::value_bounds_for_field(ir, &s.name, &f.name) {
                        match f.target.as_str() {
                            "Int" => Some(format!("{}i64", n)),
                            "Float" => Some(format!("{}.0f64", n)),
                            _ => None,
                        }
                    } else if let Some(analyze::BoundKind::Exact(n)) = analyze::value_bounds_for_field(ir, &s.name, &f.name) {
                        match f.target.as_str() {
                            "Int" => Some(format!("{}i64", n)),
                            "Float" => Some(format!("{}.0f64", n)),
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(v) = value_override {
                    if f.mult == Multiplicity::Lone {
                        format!("Some({})", v)
                    } else {
                        v
                    }
                } else {
                    let safe_targets: HashSet<String> = if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                        && is_safe_set_population(&s.name, &f.target, ir, &fixture_types)
                    {
                        HashSet::from([f.target.clone()])
                    } else {
                        HashSet::new()
                    };
                    default_value_for_field_inner(&f.target, &f.mult, is_boxed, &safe_targets)
                }
            };
            writeln!(out, "        {}: {},", f.name, val).unwrap();
        }
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Boundary value fixtures: generate if any field has a cardinality bound
        let has_bounds = s.fields.iter().any(|f| {
            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
        });
        if has_bounds {
            // Boundary fixture
            writeln!(out, "/// Factory: create {} at cardinality boundary", s.name).unwrap();
            writeln!(out, "#[allow(dead_code)]").unwrap();
            writeln!(out, "pub fn boundary_{}() -> {} {{", struct_snake, s.name).unwrap();
            writeln!(out, "    {} {{", s.name).unwrap();
            for f in &s.fields {
                let val = if f.value_type.is_some() {
                    "BTreeMap::new()".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let count = match &bound {
                            analyze::BoundKind::Exact(n) => *n,
                            analyze::BoundKind::AtMost(n) => *n,
                            analyze::BoundKind::AtLeast(n) => *n,
                        };
                        boundary_value_for_field(&f.target, &f.mult, count)
                    } else {
                        let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                        let safe_targets: HashSet<String> = if is_safe_set_population(&s.name, &f.target, ir, &fixture_types) {
                            HashSet::from([f.target.clone()])
                        } else { HashSet::new() };
                        default_value_for_field_inner(&f.target, &f.mult, is_boxed, &safe_targets)
                    }
                } else {
                    let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                    default_value_for_field(&f.target, &f.mult, is_boxed)
                };
                writeln!(out, "        {}: {},", f.name, val).unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();

            // Invalid fixture
            writeln!(out, "/// Factory: create {} that violates cardinality constraint", s.name).unwrap();
            writeln!(out, "#[allow(dead_code)]").unwrap();
            writeln!(out, "pub fn invalid_{}() -> {} {{", struct_snake, s.name).unwrap();
            writeln!(out, "    {} {{", s.name).unwrap();
            for f in &s.fields {
                let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                if f.value_type.is_some() {
                    writeln!(out, "        {}: BTreeMap::new(),", f.name).unwrap();
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let violation_count = match &bound {
                            analyze::BoundKind::Exact(n) => n + 1,
                            analyze::BoundKind::AtMost(n) => n + 1,
                            analyze::BoundKind::AtLeast(n) => if *n > 0 { n - 1 } else { 0 },
                        };
                        let val = boundary_value_for_field(&f.target, &f.mult, violation_count);
                        writeln!(out, "        {}: {},", f.name, val).unwrap();
                    } else {
                        let val = default_value_for_field(&f.target, &f.mult, is_boxed);
                        writeln!(out, "        {}: {},", f.name, val).unwrap();
                    }
                } else {
                    let val = default_value_for_field(&f.target, &f.mult, is_boxed);
                    writeln!(out, "        {}: {},", f.name, val).unwrap();
                }
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Anomaly fixtures: empty set/seq for unbounded collections
    let anomalies = analyze::detect_anomalies(ir);
    let mut anomaly_sigs_done: HashSet<String> = HashSet::new();
    for anomaly in &anomalies {
        if let analyze::AnomalyPattern::UnboundedCollection { sig_name, .. } = anomaly {
            if anomaly_sigs_done.contains(sig_name) { continue; }
            let s = match ir.structures.iter().find(|s| s.name == *sig_name) {
                Some(s) => s,
                None => continue,
            };
            if variant_names.contains(&s.name) || s.is_enum || s.fields.is_empty() { continue; }
            anomaly_sigs_done.insert(sig_name.clone());

            let struct_snake = to_snake_case(sig_name);
            writeln!(out, "/// Anomaly fixture: all set/seq fields empty (edge case for unbounded collections)").unwrap();
            writeln!(out, "#[allow(dead_code)]").unwrap();
            writeln!(out, "pub fn anomaly_empty_{}() -> {} {{", struct_snake, sig_name).unwrap();
            writeln!(out, "    {} {{", sig_name).unwrap();
            for f in &s.fields {
                let val = if f.value_type.is_some() {
                    "BTreeMap::new()".to_string()
                } else {
                    match &f.mult {
                        Multiplicity::Set => "BTreeSet::new()".to_string(),
                        Multiplicity::Seq => "Vec::new()".to_string(),
                        _ => {
                            let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                            default_value_for_field(&f.target, &f.mult, is_boxed)
                        }
                    }
                };
                writeln!(out, "        {}: {},", f.name, val).unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Generate all_{plural}() helpers for sig types with existential (some) constraints.
    {
        let mut existential_by_sig: HashMap<String, Vec<Vec<(String, String)>>> = HashMap::new();
        for constraint in &ir.constraints {
            if let Some((_kind, bindings, body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                if matches!(_kind, crate::parser::ast::QuantKind::Some) && bindings.len() == 1 {
                    if let Expr::VarRef(sig_name) = &bindings[0].domain {
                        let var_name = &bindings[0].vars[0];
                        let fields = extract_equality_fields(body, var_name, ir);
                        if !fields.is_empty() {
                            existential_by_sig.entry(sig_name.clone()).or_default().push(fields);
                        }
                    }
                }
            }
        }
        for (sig_name, field_sets) in &existential_by_sig {
            let s = match ir.structures.iter().find(|s| s.name == *sig_name) {
                Some(s) => s,
                None => continue,
            };
            if s.fields.is_empty() { continue; }
            let snake_plural = to_snake_case(sig_name);
            writeln!(out, "/// Factory: all instances needed by existential facts for {sig_name}").unwrap();
            writeln!(out, "#[allow(dead_code)]").unwrap();
            writeln!(out, "pub fn all_{snake_plural}s() -> Vec<{sig_name}> {{").unwrap();
            writeln!(out, "    vec![").unwrap();
            for fields in field_sets {
                writeln!(out, "        {sig_name} {{").unwrap();
                for f in &s.fields {
                    let val = if let Some((_, v)) = fields.iter().find(|(fname, _)| fname == &f.name) {
                        v.clone()
                    } else {
                        let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                        default_value_for_field(&f.target, &f.mult, is_boxed)
                    };
                    writeln!(out, "            {}: {},", f.name, val).unwrap();
                }
                writeln!(out, "        }},").unwrap();
            }
            writeln!(out, "    ]").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

/// Extract field=value equalities from an existential fact body.
/// For `r.structure = Semigroup and r.requiredLaw = Associativity`,
/// returns `[("structure", "AlgebraicStructure::Semigroup"), ("requiredLaw", "Law::Associativity")]`.
fn extract_equality_fields(expr: &Expr, var_name: &str, ir: &OxidtrIR) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    match expr {
        Expr::BinaryLogic { op: crate::parser::ast::LogicOp::And, left, right } => {
            fields.extend(extract_equality_fields(left, var_name, ir));
            fields.extend(extract_equality_fields(right, var_name, ir));
        }
        Expr::Comparison { op: CompareOp::Eq, left, right } => {
            // Pattern: var.field = Value
            if let Expr::FieldAccess { base, field } = left.as_ref() {
                if let Expr::VarRef(name) = base.as_ref() {
                    if name == var_name {
                        let val = expr_translator::translate_with_ir(right, ir);
                        fields.push((field.clone(), val));
                    }
                }
            }
        }
        _ => {}
    }
    fields
}

fn boundary_value_for_field(target: &str, mult: &Multiplicity, count: usize) -> String {
    match mult {
        Multiplicity::Set => {
            let items: Vec<String> = (0..count)
                .map(|_| format!("default_{}()", to_snake_case(target)))
                .collect();
            if items.is_empty() {
                "BTreeSet::new()".to_string()
            } else {
                format!("BTreeSet::from([{}])", items.join(", "))
            }
        }
        Multiplicity::Seq => {
            let items: Vec<String> = (0..count)
                .map(|_| format!("default_{}()", to_snake_case(target)))
                .collect();
            if items.is_empty() {
                "Vec::new()".to_string()
            } else {
                format!("vec![{}]", items.join(", "))
            }
        }
        _ => default_value_for_field(target, mult, false),
    }
}

fn detect_ownership_pattern(expr: &Expr, ir: &OxidtrIR) -> Option<(String, String, String, String)> {
    super::detect_ownership_pattern(expr, ir, to_snake_plural)
}


fn to_snake_plural(name: &str) -> String {
    let snake = to_snake_case(name);
    // Simple pluralization: add 's'
    format!("{snake}s")
}

/// Check if populating a set/seq field of `owner` with `default_target()`
/// would cause infinite recursion. Returns true if safe.
/// Unsafe when: default_target() transitively depends on owner through One fields.
fn is_safe_set_population(
    owner: &str, target: &str,
    ir: &OxidtrIR,
    fixture_types: &HashSet<String>,
) -> bool {
    if !fixture_types.contains(target) { return false; }
    // BFS: does default_target() transitively reach owner through One-mult fields?
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s))
        .collect();
    let mut visited = HashSet::new();
    let mut stack = vec![target.to_string()];
    while let Some(cur) = stack.pop() {
        if cur == owner { return false; } // cycle detected
        if !visited.insert(cur.clone()) { continue; }
        if let Some(s) = struct_map.get(cur.as_str()) {
            for f in &s.fields {
                if f.mult == Multiplicity::One && fixture_types.contains(&f.target) {
                    stack.push(f.target.clone());
                }
            }
        }
    }
    true
}

fn default_value_for_field(target: &str, mult: &Multiplicity, is_boxed: bool) -> String {
    default_value_for_field_inner(target, mult, is_boxed, &HashSet::new())
}

fn default_value_for_field_inner(
    target: &str, mult: &Multiplicity, is_boxed: bool,
    has_fixture: &HashSet<String>,
) -> String {
    // Native type aliases get language-native default values
    if let Some(native_default) = rust_native_default(target) {
        return match mult {
            Multiplicity::Lone => "None".to_string(),
            Multiplicity::Set => format!("BTreeSet::from([{native_default}])"),
            Multiplicity::Seq => format!("vec![{native_default}]"),
            Multiplicity::One => {
                if is_boxed {
                    format!("Box::new({native_default})")
                } else {
                    native_default.to_string()
                }
            }
        };
    }
    match mult {
        Multiplicity::Lone => "None".to_string(),
        Multiplicity::Set => {
            if has_fixture.contains(target) {
                format!("BTreeSet::from([default_{}()])", to_snake_case(target))
            } else {
                "BTreeSet::new()".to_string()
            }
        }
        Multiplicity::Seq => {
            if has_fixture.contains(target) {
                format!("vec![default_{}()]", to_snake_case(target))
            } else {
                "Vec::new()".to_string()
            }
        }
        Multiplicity::One => {
            if is_boxed {
                format!("Box::new(default_{}())", to_snake_case(target))
            } else {
                format!("default_{}()", to_snake_case(target))
            }
        }
    }
}

/// Returns a Rust default value literal for a native type alias.
fn rust_native_default(alloy_name: &str) -> Option<&'static str> {
    match alloy_name {
        "Str" => Some("String::new()"),
        "Int" => Some("0i64"),
        "Float" => Some("0.0f64"),
        "Bool" => Some("false"),
        _ => None,
    }
}

fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

/// Check if a sig type has existential (some) constraints, meaning all_{plural}s() was generated.
fn has_existential_fixture(sig_name: &str, ir: &OxidtrIR) -> bool {
    for constraint in &ir.constraints {
        if let Some((_kind, bindings, body)) = analyze::strip_outer_quantifier(&constraint.expr) {
            if matches!(_kind, crate::parser::ast::QuantKind::Some) && bindings.len() == 1 {
                if let Expr::VarRef(name) = &bindings[0].domain {
                    if name == sig_name {
                        let var_name = &bindings[0].vars[0];
                        let fields = extract_equality_fields(body, var_name, ir);
                        if !fields.is_empty() {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}
