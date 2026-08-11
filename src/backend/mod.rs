pub mod type_env;
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

/// Variants that some field names as its target.
///
/// A variant is normally not a fixture type — it is a case of its parent, not a
/// value on its own. But a backend that emits variants as real types (Go, C#)
/// still needs a factory for one the moment a field is declared to hold it,
/// or the fixture references a function that was never generated (#93).
pub fn variants_used_as_field_targets(ir: &OxidtrIR) -> HashSet<String> {
    let enum_parents: HashSet<&str> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.as_str()).collect();
    let variants: HashSet<&str> = ir.structures.iter()
        .filter(|s| s.parent.as_deref().is_some_and(|p| enum_parents.contains(p)))
        .map(|s| s.name.as_str())
        .collect();
    ir.structures.iter()
        .flat_map(|s| s.fields.iter())
        .filter(|f| variants.contains(f.target.as_str()))
        .map(|f| f.target.clone())
        .collect()
}

/// Types whose `default{T}()` provably terminates, as a least fixed point:
/// start with nothing constructible and keep adding types all of whose `one`
/// fields are already constructible. A `lone` field bottoms out at nil/None and
/// a set/seq at the empty collection, so neither is an edge here. An enum is
/// constructible as soon as one of its cases is.
///
/// Anything left out has no finite value: every one of its `one` fields leads
/// back to it. Emitting a factory for such a type produces code that compiles
/// and then blows the stack — a single-step "does this variant look
/// terminating" check cannot see a cycle that closes through another type
/// (#109), which is why `A1 { b: B }` / `B1 { a: A }` slipped through.
///
/// Also records, per enum, the case that *made* it constructible — the one
/// whose payload was already satisfiable when the enum was admitted. Selection
/// cannot re-derive this from the finished set: once `Expr` is known
/// constructible, a self-recursive case like `.loop(expr: defaultExpr())` looks
/// satisfiable too.
pub fn terminating_types(ir: &OxidtrIR) -> (HashSet<String>, HashMap<String, String>) {
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for s in &ir.structures {
        if let Some(parent) = &s.parent {
            children.entry(parent.clone()).or_default().push(s.name.clone());
        }
    }
    let enum_parents: HashSet<&str> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.as_str()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_deref().is_some_and(|p| enum_parents.contains(p)))
        .map(|s| s.name.clone())
        .collect();

    let mut done: HashSet<String> = HashSet::new();
    let mut witness: HashMap<String, String> = HashMap::new();
    let edge_ok = |f: &crate::ir::nodes::IRField, done: &HashSet<String>| {
        // A variant is a case of its parent, not a type — constructibility is
        // the parent's (#93).
        let target = variant_parent(ir, &f.target).unwrap_or_else(|| f.target.clone());
        f.value_type.is_some()
            || f.mult != Multiplicity::One
            || is_native_type_alias(&target)
            || done.contains(&target)
            // A target with no structure of its own has nothing to recurse into.
            || !ir.structures.iter().any(|s| s.name == target)
    };

    loop {
        let mut changed = false;
        for s in &ir.structures {
            if done.contains(&s.name) || variant_names.contains(&s.name) { continue; }
            let ok = if s.is_enum {
                let found = children.get(&s.name).and_then(|vs| vs.iter().find(|v| {
                    let own = ir.structures.iter().find(|c| &c.name == *v);
                    s.fields.iter()
                        .chain(own.into_iter().flat_map(|c| c.fields.iter()))
                        .all(|f| edge_ok(f, &done))
                }));
                if let Some(v) = found { witness.insert(s.name.clone(), v.clone()); }
                found.is_some()
            } else {
                s.fields.iter().all(|f| edge_ok(f, &done))
            };
            if ok {
                done.insert(s.name.clone());
                changed = true;
            }
        }
        if !changed { return (done, witness); }
    }
}

/// The parent an emitted variant belongs to, if `target` is one.
///
/// Backends that fold an abstract sig's children into a single type (Rust's
/// enum, Swift's enum, Lean's inductive) have no declaration for the variant
/// itself, so a field declared to hold one must take the parent's type; that it
/// is *that* variant becomes a constraint (#93).
pub fn variant_parent(ir: &OxidtrIR, target: &str) -> Option<String> {
    let s = ir.structures.iter().find(|s| s.name == target)?;
    let parent = s.parent.as_ref()?;
    ir.structures
        .iter()
        .any(|p| p.name == *parent && p.is_enum)
        .then(|| parent.clone())
}

/// Collect fixture-eligible types: non-enum, non-variant sigs that have a value.
///
/// Having no field is not having no value: `sig Person {}` is `{}`, and every
/// backend already emits a factory for it. Keying on `!fields.is_empty()` left
/// those sigs out, so their domains were materialised empty and the quantifier
/// over them went vacuous (#136, #105). What has no factory is what has no
/// finite value at all — a cycle of `one` fields — which is what
/// `terminating_types` decides (#109).
pub fn collect_fixture_types(ir: &OxidtrIR) -> HashSet<String> {
    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();
    let (terminating, _) = terminating_types(ir);
    ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum
            && terminating.contains(&s.name)
            && !is_native_type_alias(&s.name))
        .map(|s| s.name.clone())
        .collect()
}
