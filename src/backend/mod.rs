pub mod rust;
pub mod typescript;
pub mod jvm;
pub mod swift;
pub mod go;
pub mod csharp;
pub mod lean;
pub mod schema;

use crate::parser::ast::{Expr, CompareOp, QuantKind, Multiplicity};
use crate::ir::nodes::{OxidtrIR, StructureNode};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

// ── Alloy native type aliases ───────────────────────────────────────────────
//
// Alloy has no primitive types — everything is a sig instance.  These marker
// sig names are mapped to language-native primitives by each backend so that
// `sig Str {}` is never emitted as `pub struct Str;` but instead the field
// type becomes `String` (Rust), `string` (TS/Go), etc.

/// Returns `true` if `name` is a well-known Alloy marker sig that should be
/// mapped to a native primitive type rather than emitted as a struct/class.
pub fn is_native_type_alias(name: &str) -> bool {
    matches!(name, "Str" | "Int" | "Float" | "Bool")
}

/// Target-language enum for native type resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetLang {
    Rust,
    TypeScript,
    Kotlin,
    Java,
    Swift,
    Go,
    CSharp,
    Lean,
}

/// Map an Alloy native-alias sig name to the corresponding language primitive.
/// Returns `None` if `name` is not a native alias.
pub fn native_type_for(lang: TargetLang, name: &str) -> Option<&'static str> {
    match name {
        "Str" => Some(match lang {
            TargetLang::Rust => "String",
            TargetLang::TypeScript => "string",
            TargetLang::Kotlin => "String",
            TargetLang::Java => "String",
            TargetLang::Swift => "String",
            TargetLang::Go => "string",
            TargetLang::CSharp => "string",
            TargetLang::Lean => "String",
        }),
        "Int" => Some(match lang {
            TargetLang::Rust => "i64",
            TargetLang::TypeScript => "number",
            TargetLang::Kotlin => "Long",
            TargetLang::Java => "long",
            TargetLang::Swift => "Int",
            TargetLang::Go => "int64",
            TargetLang::CSharp => "long",
            TargetLang::Lean => "Int",
        }),
        "Float" => Some(match lang {
            TargetLang::Rust => "f64",
            TargetLang::TypeScript => "number",
            TargetLang::Kotlin => "Double",
            TargetLang::Java => "double",
            TargetLang::Swift => "Double",
            TargetLang::Go => "float64",
            TargetLang::CSharp => "double",
            TargetLang::Lean => "Float",
        }),
        "Bool" => Some(match lang {
            TargetLang::Rust => "bool",
            TargetLang::TypeScript => "boolean",
            TargetLang::Kotlin => "Boolean",
            TargetLang::Java => "boolean",
            TargetLang::Swift => "Bool",
            TargetLang::Go => "bool",
            TargetLang::CSharp => "bool",
            TargetLang::Lean => "Bool",
        }),
        _ => None,
    }
}

/// Resolve a type name: if it's a native alias, return the mapped name;
/// otherwise return the original name unchanged.
pub fn resolve_type(lang: TargetLang, name: &str) -> String {
    native_type_for(lang, name)
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Reverse-map a language primitive back to the Alloy alias name.
/// Used by `check` and `extract` to compare impl types against the model.
pub fn reverse_native_type(lang: TargetLang, native: &str) -> Option<&'static str> {
    // Check each alias
    for alias in &["Str", "Int", "Float", "Bool"] {
        if native_type_for(lang, alias) == Some(native) {
            return Some(alias);
        }
    }
    None
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
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty()
            && !is_native_type_alias(&s.name))
        .map(|s| s.name.clone())
        .collect()
}
