pub mod expr_translator;

use super::GeneratedFile;
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

    out
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

    if use_serde {
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]").unwrap();
        // Check if any variant has fields — if so, use tagged representation
        let has_data_variants = children.map_or(false, |vs| {
            vs.iter().any(|v| struct_map.get(v.as_str()).map_or(false, |st| !st.fields.is_empty()))
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
            let fields = child.map(|c| &c.fields).filter(|f| !f.is_empty());
            if let Some(fields) = fields {
                writeln!(out, "    {v} {{").unwrap();
                for f in fields {
                    // Fields referencing the parent enum type need Box to break recursion
                    let needs_box = f.target == s.name;
                    let is_self_ref = needs_box
                        || self_ref_fields.contains(&(v.clone(), f.name.clone()));
                    let type_str = if let Some(vt) = &f.value_type {
                        format!("BTreeMap<{}, {}>", f.target, vt)
                    } else if let Some(raw) = &f.raw_union_type {
                        rust_union_type(raw, &f.mult)
                    } else {
                        multiplicity_to_type(&f.target, &f.mult, is_self_ref)
                    };
                    writeln!(out, "        {}: {type_str},", f.name).unwrap();
                }
                writeln!(out, "    }},").unwrap();
            } else {
                writeln!(out, "    {v},").unwrap();
            }
        }
    }
    writeln!(out, "}}").unwrap();
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
            let type_str = if let Some(vt) = &f.value_type {
                format!("BTreeMap<{}, {}>", f.target, vt)
            } else if let Some(raw) = &f.raw_union_type {
                rust_union_type(raw, &f.mult)
            } else {
                multiplicity_to_type(&f.target, &f.mult, is_self_ref)
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

    writeln!(out, "use crate::models::*;").unwrap();

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
            None => String::new(),
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
    writeln!(out, "use crate::models::*;").unwrap();
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
        .filter(|s| !variant_names_set.contains(&s.name) && !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone()).collect();

    // Check if any expression uses TC functions → need helpers import
    let needs_helpers = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    writeln!(out, "#[cfg(test)]").unwrap();
    writeln!(out, "mod property_tests {{").unwrap();
    writeln!(out, "    #[allow(unused_imports)]").unwrap();
    writeln!(out, "    use crate::models::*;").unwrap();
    if needs_helpers {
        writeln!(out, "    #[allow(unused_imports)]").unwrap();
        writeln!(out, "    use crate::helpers::*;").unwrap();
    }
    writeln!(out, "    #[allow(unused_imports)]").unwrap();
    writeln!(out, "    use crate::fixtures::*;").unwrap();
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

        writeln!(out, "    #[test]").unwrap();
        writeln!(out, "    fn {test_name}() {{").unwrap();

        for (pname, tname) in &params {
            if has_fixture.contains(tname) {
                let snake = to_snake_case(tname);
                writeln!(out, "        let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
            } else {
                writeln!(out, "        let {pname}: Vec<{tname}> = Vec::new();").unwrap();
            }
        }

        writeln!(out, "        assert!({body});").unwrap();
        writeln!(out, "    }}").unwrap();
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

        // Alloy 6: temporal facts with prime → generate scaffold test
        // Prime references (x') require before/after state capture which cannot be
        // expressed as a simple field access; emit a scaffold with TODO comments.
        if analyze::expr_contains_prime(&constraint.expr) {
            let test_name = format!("transition_{}", to_snake_case(&fact_name));
            let params = expr_translator::extract_params(&constraint.expr, &sig_names);
            if params.iter().any(|(_, tname)| variant_names_set.contains(tname) || enum_parents.contains(tname)) {
                continue;
            }
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "    /// @temporal Transition constraint: {fact_name}").unwrap();
            writeln!(out, "    /// Scaffold: prime (next-state) references require a before/after transition mechanism.").unwrap();
            writeln!(out, "    #[test]").unwrap();
            writeln!(out, "    fn {test_name}() {{").unwrap();
            writeln!(out, "        // TODO: apply transition, then assert post-condition").unwrap();
            writeln!(out, "        // Alloy constraint: {desc}").unwrap();
            for (pname, tname) in &params {
                writeln!(out, "        // pre: capture {pname}: Vec<{tname}> before transition").unwrap();
                writeln!(out, "        // post: assert condition on {pname} after transition").unwrap();
            }
            writeln!(out, "    }}").unwrap();
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
            writeln!(out, "    // Type-guaranteed: {} — no test needed (Rust type system encodes this)", fact_name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        // Check if any constraint is PartiallyByType → generate regression test
        let any_partial = sig_constraints.iter().any(|c| {
            can_guarantee_by_type(c, TargetLang::Rust) == Guarantee::PartiallyByType
        });

        if any_partial {
            writeln!(out, "    /// @regression Partially type-guaranteed — regression test only.").unwrap();
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
            writeln!(out, "    /// {annotation}").unwrap();
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
            writeln!(out, "    #[test]").unwrap();
            writeln!(out, "    fn {test_name}() {{").unwrap();
            writeln!(out, "        // binary temporal: requires trace-based verification; see check_{op_label}_{snake_name}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else if matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            // Liveness/past_liveness: cannot be verified with single snapshot
            let kind_label = if temporal_kind == Some(analyze::TemporalKind::Liveness) {
                "liveness" } else { "past_liveness" };
            let snake_name = to_snake_case(&fact_name);
            writeln!(out, "    #[test]").unwrap();
            writeln!(out, "    fn {test_name}() {{").unwrap();
            writeln!(out, "        // {kind_label}: requires trace-based verification; see check_{kind_label}_{snake_name}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else {
        // Detect ownership facts: `all x: A | some y: B | x in y.field`
        // These need linked fixture setup where B.field contains x.
        let ownership = detect_ownership_pattern(&constraint.expr, ir);

        writeln!(out, "    #[test]").unwrap();
        writeln!(out, "    fn {test_name}() {{").unwrap();
        if let Some((owned_var, owner_var, _owner_type, field_name)) = &ownership {
            // Generate linked setup: create owned item, insert into owner's field
            let owned_param = params.iter().find(|(p, _)| p == owned_var);
            let owner_param = params.iter().find(|(p, _)| p == owner_var);
            if let (Some((opname, otname)), Some((cpname, ctname))) = (owned_param, owner_param) {
                let owned_snake = to_snake_case(otname);
                let owner_snake = to_snake_case(ctname);
                writeln!(out, "        let item = default_{owned_snake}();").unwrap();
                writeln!(out, "        let mut owner = default_{owner_snake}();").unwrap();
                writeln!(out, "        owner.{field_name}.insert(item.clone());").unwrap();
                writeln!(out, "        let {opname}: Vec<{otname}> = vec![item];").unwrap();
                writeln!(out, "        let {cpname}: Vec<{ctname}> = vec![owner];").unwrap();
                // Emit remaining params normally
                for (pname, tname) in &params {
                    if pname == opname || pname == cpname { continue; }
                    if has_fixture.contains(tname) {
                        let snake = to_snake_case(tname);
                        writeln!(out, "        let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                    } else {
                        writeln!(out, "        let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                    }
                }
            }
        } else {
            for (pname, tname) in &params {
                if has_fixture.contains(tname) {
                    let snake = to_snake_case(tname);
                    writeln!(out, "        let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                } else {
                    writeln!(out, "        let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                }
            }
        }
        writeln!(out, "        assert!({body});").unwrap();
        writeln!(out, "    }}").unwrap();
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
                    writeln!(out, "    /// Trace checker for {kind_label}: {semantics}.").unwrap();
                    writeln!(out, "    #[allow(dead_code)]").unwrap();
                    // Generate trace fn signature with tuple params
                    if params.len() == 1 {
                        let (pname, tname) = &params[0];
                        writeln!(out, "    fn check_{kind_label}_{snake_name}(trace: &[Vec<{tname}>]) -> bool {{").unwrap();
                        writeln!(out, "        trace.iter().any(|{pname}| {{").unwrap();
                    } else {
                        let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("Vec<{t}>")).collect();
                        let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                        writeln!(out, "    fn check_{kind_label}_{snake_name}(trace: &[({})]) -> bool {{", tuple_types.join(", ")).unwrap();
                        writeln!(out, "        trace.iter().any(|({})| {{", tuple_names.join(", ")).unwrap();
                    }
                    writeln!(out, "            {body}").unwrap();
                    writeln!(out, "        }})").unwrap();
                    writeln!(out, "    }}").unwrap();
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
                        writeln!(out, "    /// Trace checker for {op_name}: {semantics}.").unwrap();
                        writeln!(out, "    #[allow(dead_code)]").unwrap();
                        if params.len() == 1 {
                            let (pname, tname) = &params[0];
                            writeln!(out, "    fn check_{op_name}_{snake_name}(trace: &[Vec<{tname}>]) -> bool {{").unwrap();
                            match op {
                                TemporalBinaryOp::Until => {
                                    writeln!(out, "        match trace.iter().position(|{pname}| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "            Some(pos) => trace[..pos].iter().all(|{pname}| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "            None => false,").unwrap();
                                    writeln!(out, "        }}").unwrap();
                                }
                                TemporalBinaryOp::Since => {
                                    writeln!(out, "        match trace.iter().rposition(|{pname}| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "            Some(pos) => trace[pos..].iter().all(|{pname}| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "            None => false,").unwrap();
                                    writeln!(out, "        }}").unwrap();
                                }
                                TemporalBinaryOp::Release => {
                                    // Release: right holds until (and including when) left becomes true
                                    writeln!(out, "        match trace.iter().position(|{pname}| {{ {left_body} }}) {{").unwrap();
                                    writeln!(out, "            Some(pos) => trace[..=pos].iter().all(|{pname}| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "            None => trace.iter().all(|{pname}| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "        }}").unwrap();
                                }
                                TemporalBinaryOp::Triggered => {
                                    // Triggered: if right ever holds, left must hold at or before that point
                                    writeln!(out, "        trace.iter().enumerate().all(|(i, {pname})| {{").unwrap();
                                    writeln!(out, "            if {right_body} {{ trace[..=i].iter().any(|{pname}| {{ {left_body} }}) }} else {{ true }}").unwrap();
                                    writeln!(out, "        }})").unwrap();
                                }
                            }
                        } else {
                            let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("Vec<{t}>")).collect();
                            let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                            let pnames = tuple_names.join(", ");
                            writeln!(out, "    fn check_{op_name}_{snake_name}(trace: &[({})]) -> bool {{", tuple_types.join(", ")).unwrap();
                            match op {
                                TemporalBinaryOp::Until => {
                                    writeln!(out, "        match trace.iter().position(|({pnames})| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "            Some(pos) => trace[..pos].iter().all(|({pnames})| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "            None => false,").unwrap();
                                    writeln!(out, "        }}").unwrap();
                                }
                                TemporalBinaryOp::Since => {
                                    writeln!(out, "        match trace.iter().rposition(|({pnames})| {{ {right_body} }}) {{").unwrap();
                                    writeln!(out, "            Some(pos) => trace[pos..].iter().all(|({pnames})| {{ {left_body} }}),").unwrap();
                                    writeln!(out, "            None => false,").unwrap();
                                    writeln!(out, "        }}").unwrap();
                                }
                                TemporalBinaryOp::Release => {
                                    writeln!(out, "        match trace.iter().position(|({pnames})| {{ {left_body} }}) {{").unwrap();
                                    writeln!(out, "            Some(pos) => trace[..=pos].iter().all(|({pnames})| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "            None => trace.iter().all(|({pnames})| {{ {right_body} }}),").unwrap();
                                    writeln!(out, "        }}").unwrap();
                                }
                                TemporalBinaryOp::Triggered => {
                                    writeln!(out, "        trace.iter().enumerate().all(|(i, ({pnames}))| {{").unwrap();
                                    writeln!(out, "            if {right_body} {{ trace[..=i].iter().any(|({pnames})| {{ {left_body} }}) }} else {{ true }}").unwrap();
                                    writeln!(out, "        }})").unwrap();
                                }
                            }
                        }
                        writeln!(out, "    }}").unwrap();
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
            writeln!(out, "    #[test]").unwrap();
            writeln!(out, "    fn {test_name}() {{").unwrap();
            for (pname, tname) in &params {
                    let snake = to_snake_case(tname);
                    let has_b = ir.structures.iter().any(|s| {
                        s.name == *tname && s.fields.iter().any(|f| {
                            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                        })
                    });
                    if has_b {
                        writeln!(out, "        let {pname}: Vec<{tname}> = vec![boundary_{snake}()];").unwrap();
                    } else if has_fixture.contains(tname) {
                        writeln!(out, "        let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                    } else {
                        writeln!(out, "        let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                    }
                }
            writeln!(out, "        assert!({body}, \"boundary values should satisfy invariant\");").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();

            // Negative test
            let test_name = format!("invalid_{}", to_snake_case(&fact_name));
            writeln!(out, "    #[test]").unwrap();
            writeln!(out, "    fn {test_name}() {{").unwrap();
            for (pname, tname) in &params {
                let snake = to_snake_case(tname);
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "        let {pname}: Vec<{tname}> = vec![invalid_{snake}()];").unwrap();
                } else if has_fixture.contains(tname) {
                    writeln!(out, "        let {pname}: Vec<{tname}> = vec![default_{snake}()];").unwrap();
                } else {
                    writeln!(out, "        let {pname}: Vec<{tname}> = Vec::new();").unwrap();
                }
            }
            writeln!(out, "        assert!(!({body}), \"invalid values should violate invariant\");").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Cross-tests: for each (fact, operation) pair, verify fact is preserved
    if !ir.constraints.is_empty() && !ir.operations.is_empty() {
        writeln!(out, "    // --- Cross-tests: fact × operation ---").unwrap();
        writeln!(out).unwrap();

        for constraint in &ir.constraints {
            let fact_name = match &constraint.name {
                Some(name) => name.clone(),
                None => continue,
            };

            for op in &ir.operations {
                let op_name = to_snake_case(&op.name);
                let test_name = format!("{}_preserved_after_{}", to_snake_case(&fact_name), op_name);

                writeln!(out, "    #[test]").unwrap();
                writeln!(out, "    #[ignore]").unwrap();
                writeln!(out, "    fn {test_name}() {{").unwrap();
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
                writeln!(out, "    }}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    writeln!(out, "}}").unwrap();

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
    }

    if newtype_pairs.is_empty() {
        return String::new();
    }

    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use crate::models::*;").unwrap();
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use crate::fixtures::*;").unwrap();

    // Check if TC functions are needed → import helpers
    let needs_helpers = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr));
    if needs_helpers {
        writeln!(out, "#[allow(unused_imports)]").unwrap();
        writeln!(out, "use crate::helpers::*;").unwrap();
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

        // Exhaustive checks
        for c in &all_constraints {
            if let analyze::ConstraintInfo::Exhaustive { sig_name: s, categories } = c {
                if s == sig_name {
                    let cats = categories.join(", ");
                    writeln!(out, "        // Exhaustive: must belong to one of [{cats}]").unwrap();
                    writeln!(out, "        // (cross-sig membership — checked at integration level)").unwrap();
                    writeln!(out, "        // To enable: pass category collections and verify membership").unwrap();
                    writeln!(out, "        // must belong to one of [{cats}]").unwrap();
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

fn generate_fixtures(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();
    let cyclic = find_cyclic_fields(ir);

    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use crate::models::*;").unwrap();

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

    // Generate enum default fixtures (first variant)
    for s in &ir.structures {
        if !s.is_enum { continue; }
        let variants = match children.get(&s.name) {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        let enum_snake = to_snake_case(&s.name);
        // Find first unit variant (no fields)
        let first_unit = variants.iter().find(|v| {
            struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
        });
        if let Some(variant) = first_unit {
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
        if s.fields.is_empty() { continue; }

        let struct_snake = to_snake_case(&s.name);

        writeln!(out, "/// Factory: create a default valid {}", s.name).unwrap();
        writeln!(out, "#[allow(dead_code)]").unwrap();
        writeln!(out, "pub fn default_{}() -> {} {{", struct_snake, s.name).unwrap();
        writeln!(out, "    {} {{", s.name).unwrap();
        for f in &s.fields {
            let val = if f.value_type.is_some() {
                "BTreeMap::new()".to_string()
            } else {
                let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                {
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

    out
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
