pub mod expr_translator;

use super::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::Multiplicity;
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

    if !ir.constraints.is_empty() || has_tc {
        files.push(GeneratedFile {
            path: "invariants.rs".to_string(),
            content: generate_invariants(ir),
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
    if needs_btreeset {
        writeln!(out, "use std::collections::BTreeSet;").unwrap();
        writeln!(out).unwrap();
    }

    // Collect parent→children mapping for enum generation
    let children: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for s in &ir.structures {
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

        if s.is_enum {
            generate_enum(&mut out, s, children.get(&s.name), ir, &self_ref_fields, use_serde);
        } else {
            generate_struct(&mut out, s, &self_ref_fields, use_serde);
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
                    let type_str = multiplicity_to_type(&f.target, &f.mult, is_self_ref);
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
) {
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
            let is_self_ref = self_ref_fields.contains(&(s.name.clone(), f.name.clone()));
            let type_str = multiplicity_to_type(&f.target, &f.mult, is_self_ref);
            writeln!(out, "    pub {}: {type_str},", f.name).unwrap();
        }
        writeln!(out, "}}").unwrap();
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

    // Check if any operation parameter uses Set multiplicity
    let needs_btreeset = ir.operations.iter().any(|op| {
        op.params.iter().any(|p| p.mult == Multiplicity::Set)
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

        writeln!(out, "pub fn {fn_name}({params}) {{").unwrap();
        writeln!(out, "    todo!(\"oxidtr: implement {}\");", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

fn generate_invariants(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = collect_sig_names(ir);

    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use crate::models::*;").unwrap();
    writeln!(out).unwrap();

    // Check if any constraint uses transitive closure
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

    for constraint in &ir.constraints {
        let fn_name = match &constraint.name {
            Some(name) => format!("assert_{}", to_snake_case(name)),
            None => continue, // skip anonymous facts for now
        };

        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        let param_str = params
            .iter()
            .map(|(pname, tname)| format!("{pname}: &[{tname}]"))
            .collect::<Vec<_>>()
            .join(", ");

        writeln!(out, "/// Invariant derived from Alloy fact.").unwrap();
        writeln!(out, "#[allow(dead_code)]").unwrap();
        writeln!(out, "pub fn {fn_name}({param_str}) -> bool {{").unwrap();
        writeln!(out, "    {body}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
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
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. } => {
            expr_uses_tc(left) || expr_uses_tc(right)
        }
        Expr::Quantifier { domain, body, .. } => expr_uses_tc(domain) || expr_uses_tc(body),
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

    writeln!(out, "#[cfg(test)]").unwrap();
    writeln!(out, "mod property_tests {{").unwrap();
    writeln!(out, "    #[allow(unused_imports)]").unwrap();
    writeln!(out, "    use crate::models::*;").unwrap();
    writeln!(out, "    #[allow(unused_imports)]").unwrap();
    writeln!(out, "    use crate::invariants::*;").unwrap();
    writeln!(out, "    #[allow(unused_imports)]").unwrap();
    writeln!(out, "    use crate::fixtures::*;").unwrap();
    writeln!(out).unwrap();

    // Property tests from asserts — translated expressions
    for prop in &ir.properties {
        let test_name = to_snake_case(&prop.name);
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
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

    // Invariant tests — call each invariant function
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let fn_name = format!("assert_{}", to_snake_case(&fact_name));
        let test_name = format!("invariant_{}", to_snake_case(&fact_name));
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);

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
        let args = params
            .iter()
            .map(|(pname, _)| format!("&{pname}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "        assert!({fn_name}({args}));").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
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
            let fact_fn = format!("assert_{}", to_snake_case(&fact_name));

            for op in &ir.operations {
                let op_name = to_snake_case(&op.name);
                let test_name = format!("{}_preserved_after_{}", to_snake_case(&fact_name), op_name);

                writeln!(out, "    #[test]").unwrap();
                writeln!(out, "    fn {test_name}() {{").unwrap();
                writeln!(
                    out,
                    "        // Verify that {} holds after {}",
                    fact_name, op.name
                )
                .unwrap();
                writeln!(
                    out,
                    "        // pre: assert!({fact_fn}());"
                )
                .unwrap();
                writeln!(
                    out,
                    "        // {op_name}(...);"
                )
                .unwrap();
                writeln!(
                    out,
                    "        // post: assert!({fact_fn}());"
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

    // Collect (fact_name, sig_name) pairs where the fact has a Comparison in its expression
    let mut newtype_pairs: Vec<(String, String)> = Vec::new();
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        // Check if this constraint contains a Comparison
        if !expr_has_comparison(&constraint.expr) {
            continue;
        }
        // Find which sigs this constraint references
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        for (_pname, tname) in &params {
            newtype_pairs.push((fact_name.clone(), tname.clone()));
        }
    }

    if newtype_pairs.is_empty() {
        return String::new();
    }

    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use crate::models::*;").unwrap();
    writeln!(out, "#[allow(unused_imports)]").unwrap();
    writeln!(out, "use crate::invariants::*;").unwrap();
    writeln!(out).unwrap();

    // Deduplicate by (fact, sig)
    newtype_pairs.sort();
    newtype_pairs.dedup();

    for (fact_name, sig_name) in &newtype_pairs {
        let newtype_name = format!("Validated{sig_name}");
        let assert_fn = format!("assert_{}", to_snake_case(fact_name));

        writeln!(out, "/// Newtype wrapper: {sig_name} validated by {fact_name}.").unwrap();
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, Hash)]").unwrap();
        writeln!(out, "pub struct {newtype_name}(pub {sig_name});").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "impl TryFrom<{sig_name}> for {newtype_name} {{").unwrap();
        writeln!(out, "    type Error = &'static str;").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "    fn try_from(value: {sig_name}) -> Result<Self, Self::Error> {{").unwrap();
        writeln!(out, "        if {assert_fn}(&[value.clone()]) {{").unwrap();
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
        Expr::BinaryLogic { left, right, .. } => {
            expr_has_comparison(left) || expr_has_comparison(right)
        }
        Expr::Quantifier { body, domain, .. } => {
            expr_has_comparison(body) || expr_has_comparison(domain)
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            expr_has_comparison(inner)
        }
        Expr::FieldAccess { base, .. } => expr_has_comparison(base),
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
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

    for s in &ir.structures {
        if variant_names.contains(&s.name) || s.is_enum { continue; }
        if s.fields.is_empty() { continue; }

        let struct_snake = to_snake_case(&s.name);

        writeln!(out, "/// Factory: create a default valid {}", s.name).unwrap();
        writeln!(out, "#[allow(dead_code)]").unwrap();
        writeln!(out, "pub fn default_{}() -> {} {{", struct_snake, s.name).unwrap();
        writeln!(out, "    {} {{", s.name).unwrap();
        for f in &s.fields {
            let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
            let val = default_value_for_field(&f.target, &f.mult, is_boxed);
            writeln!(out, "        {}: {},", f.name, val).unwrap();
        }
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

fn default_value_for_field(target: &str, mult: &Multiplicity, is_boxed: bool) -> String {
    match mult {
        Multiplicity::Lone => "None".to_string(),
        Multiplicity::Set => "BTreeSet::new()".to_string(),
        Multiplicity::Seq => "Vec::new()".to_string(),
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
