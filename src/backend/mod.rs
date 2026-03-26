pub mod rust;
pub mod typescript;
pub mod jvm;
pub mod swift;
pub mod go;
pub mod csharp;
pub mod schema;

use crate::parser::ast::{Expr, CompareOp, QuantKind, Multiplicity};
use crate::ir::nodes::{OxidtrIR, StructureNode};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

/// Detect direct ownership pattern: `all x: A | some y: B | x in y.field`
/// Returns (owned_param_name, owner_param_name, field_name) using the
/// provided name-transform function to build param names from type names.
pub fn detect_ownership_pattern<F>(expr: &Expr, _ir: &OxidtrIR, name_fn: F) -> Option<(String, String, String, String)>
where F: Fn(&str) -> String {
    if let Expr::Quantifier { kind: QuantKind::All, bindings, body } = expr {
        if bindings.len() != 1 || bindings[0].vars.len() != 1 { return None; }
        let owned_var = &bindings[0].vars[0];
        let owned_type = if let Expr::VarRef(name) = &bindings[0].domain { name.clone() } else { return None; };

        if let Expr::Quantifier { kind: QuantKind::Some, bindings: inner_bindings, body: inner_body } = body.as_ref() {
            if inner_bindings.len() == 1 && inner_bindings[0].vars.len() == 1 {
                let owner_var = &inner_bindings[0].vars[0];
                let owner_type = if let Expr::VarRef(name) = &inner_bindings[0].domain { name.clone() } else { return None; };

                if let Expr::Comparison { op: CompareOp::In, left, right } = inner_body.as_ref() {
                    if let (Expr::VarRef(lvar), Expr::FieldAccess { base, field }) = (left.as_ref(), right.as_ref()) {
                        if let Expr::VarRef(rvar) = base.as_ref() {
                            if lvar == owned_var && rvar == owner_var {
                                return Some((name_fn(&owned_type), name_fn(&owner_type), owner_type, field.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Check if populating a set/seq field of `owner` with `default_{target}()`
/// would cause infinite recursion. Returns true if safe (no cycle).
pub fn is_safe_set_population(
    owner: &str, target: &str,
    ir: &OxidtrIR, fixture_types: &HashSet<String>,
) -> bool {
    if !fixture_types.contains(target) { return false; }
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s))
        .collect();
    let mut visited = HashSet::new();
    let mut stack = vec![target.to_string()];
    while let Some(cur) = stack.pop() {
        if cur == owner { return false; }
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

/// Collect fixture-eligible types: non-enum, non-variant sigs with fields.
pub fn collect_fixture_types(ir: &OxidtrIR) -> HashSet<String> {
    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();
    ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone())
        .collect()
}
