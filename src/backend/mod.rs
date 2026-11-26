pub mod type_env;
pub mod rust;
pub mod typescript;
pub mod jvm;
pub mod swift;
pub mod go;
pub mod csharp;
pub mod lean;
pub mod schema;
pub mod coverage;

use crate::parser::ast::{Expr, CompareOp, QuantKind, Multiplicity, SigMultiplicity};
use crate::ir::nodes::{IRField, OxidtrIR, StructureNode};
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

// ── Keyword identifiers ─────────────────────────────────────────────────────
//
// A field may be named after a keyword of the target (`use`, `where`, `long`).
// Rust, Kotlin and Java derive further identifiers from field names
// (`validate_kw_use`, test names, factory parameters), so neither `r#use` nor
// a backtick survives everywhere a name travels. Instead a keyword gets a
// trailing underscore. A name that already looks like an escaped keyword
// (`long_`) gets one more, so the mapping is injective and `extract` undoes it
// by removing exactly one (#152).

/// Rust strict and reserved keywords (2024 edition).
pub fn is_rust_keyword(name: &str) -> bool {
    matches!(name,
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
        | "enum" | "extern" | "false" | "fn" | "for" | "gen" | "if" | "impl" | "in" | "let"
        | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self"
        | "Self" | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe"
        | "use" | "where" | "while" | "abstract" | "become" | "box" | "do" | "final"
        | "macro" | "override" | "priv" | "typeof" | "unsized" | "virtual" | "yield" | "try")
}

/// Kotlin hard keywords — the ones that cannot name a property.
pub fn is_kotlin_keyword(name: &str) -> bool {
    matches!(name,
        "as" | "break" | "class" | "continue" | "do" | "else" | "false" | "for" | "fun" | "if"
        | "in" | "interface" | "is" | "null" | "object" | "package" | "return" | "super"
        | "this" | "throw" | "true" | "try" | "typealias" | "typeof" | "val" | "var"
        | "when" | "while")
}

/// Java reserved keywords and literals.
pub fn is_java_keyword(name: &str) -> bool {
    matches!(name,
        "abstract" | "assert" | "boolean" | "break" | "byte" | "case" | "catch" | "char"
        | "class" | "const" | "continue" | "default" | "do" | "double" | "else" | "enum"
        | "extends" | "final" | "finally" | "float" | "for" | "goto" | "if" | "implements"
        | "import" | "instanceof" | "int" | "interface" | "long" | "native" | "new"
        | "package" | "private" | "protected" | "public" | "return" | "short" | "static"
        | "strictfp" | "super" | "switch" | "synchronized" | "this" | "throw" | "throws"
        | "transient" | "try" | "void" | "volatile" | "while" | "true" | "false" | "null"
        | "_")
}

/// `name` with a trailing underscore if it is, or looks like an escaped form
/// of, a keyword.
pub fn escape_keyword(name: &str, is_keyword: fn(&str) -> bool) -> String {
    let base = name.trim_end_matches('_');
    if !base.is_empty() && is_keyword(base) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

/// The inverse of `escape_keyword`.
pub fn unescape_keyword(name: &str, is_keyword: fn(&str) -> bool) -> String {
    let base = name.trim_end_matches('_');
    if name.ends_with('_') && !base.is_empty() && is_keyword(base) {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

/// `ir` with every field, variable and parameter named after a keyword
/// escaped, in declarations and in every expression that refers to them.
/// Sig names are left alone: they are capitalised, and a `VarRef` that names
/// a sig must keep matching the type. So is Alloy's `this`, the receiver of a
/// derived field, which each backend translates itself.
pub fn escape_keyword_idents(ir: &OxidtrIR, is_keyword: fn(&str) -> bool) -> OxidtrIR {
    let sigs: HashSet<&str> = ir.structures.iter().map(|s| s.name.as_str()).collect();
    let esc = |n: &str| {
        if n == "this" || sigs.contains(n) { n.to_string() } else { escape_keyword(n, is_keyword) }
    };
    let mut out = ir.clone();
    for s in &mut out.structures {
        for f in &mut s.fields {
            f.name = esc(&f.name);
        }
    }
    for c in &mut out.constraints {
        rename_in_expr(&mut c.expr, &esc);
    }
    for p in &mut out.properties {
        rename_in_expr(&mut p.expr, &esc);
    }
    for op in &mut out.operations {
        for p in &mut op.params {
            p.name = esc(&p.name);
        }
        for e in &mut op.body {
            rename_in_expr(e, &esc);
        }
    }
    out
}

fn rename_in_expr(e: &mut Expr, f: &dyn Fn(&str) -> String) {
    match e {
        Expr::IntLiteral(_) => {}
        Expr::VarRef(name) => *name = f(name),
        Expr::FieldAccess { base, field } => {
            rename_in_expr(base, f);
            *field = f(field);
        }
        Expr::Cardinality(x) | Expr::TransitiveClosure(x) | Expr::ReflexiveClosure(x)
        | Expr::Not(x) | Expr::Prime(x) => rename_in_expr(x, f),
        Expr::MultFormula { expr, .. } | Expr::TemporalUnary { expr, .. } => rename_in_expr(expr, f),
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right }
        | Expr::TemporalBinary { left, right, .. } => {
            rename_in_expr(left, f);
            rename_in_expr(right, f);
        }
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings {
                for v in &mut b.vars {
                    *v = f(v);
                }
                rename_in_expr(&mut b.domain, f);
            }
            rename_in_expr(body, f);
        }
        Expr::FunApp { receiver, args, .. } => {
            if let Some(r) = receiver {
                rename_in_expr(r, f);
            }
            for a in args {
                rename_in_expr(a, f);
            }
        }
    }
}

// ── Cases of an enum of constants ───────────────────────────────────────────
//
// An abstract sig whose cases carry no data — none of their own fields, none
// on the parent — becomes an enum of constants (`enum class Color { Red }`,
// a union of string literals, Go iota constants). A case is then a value, not
// a type, so a field declared `red: one Red` cannot be typed `Red` (#154).
// Each backend types it as the parent enum (TypeScript as the literal) and
// gives it the constant as its default.
//
// The type and default-value helpers that need this are called from dozens
// of sites that do not carry the IR, so the mapping is installed for the
// duration of one `generate` call by `UnitVariantScope`.

thread_local! {
    static UNIT_VARIANTS: std::cell::RefCell<HashMap<String, String>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Every case of an enum of constants, mapped to its enum.
pub fn constant_enum_cases(ir: &OxidtrIR) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for parent in ir.structures.iter().filter(|s| s.is_enum && s.fields.is_empty()) {
        let cases: Vec<&StructureNode> = ir.structures.iter()
            .filter(|c| c.parent.as_deref() == Some(parent.name.as_str()))
            .collect();
        if !cases.is_empty() && cases.iter().all(|c| c.fields.is_empty()) {
            for c in cases {
                out.insert(c.name.clone(), parent.name.clone());
            }
        }
    }
    out
}

/// Installs `constant_enum_cases(ir)` until dropped, restoring what was there.
pub(crate) struct UnitVariantScope(HashMap<String, String>);

impl UnitVariantScope {
    pub fn new(ir: &OxidtrIR) -> Self {
        let cases = constant_enum_cases(ir);
        UnitVariantScope(UNIT_VARIANTS.with(|m| std::mem::replace(&mut *m.borrow_mut(), cases)))
    }
}

impl Drop for UnitVariantScope {
    fn drop(&mut self) {
        let previous = std::mem::take(&mut self.0);
        UNIT_VARIANTS.with(|m| *m.borrow_mut() = previous);
    }
}

/// The enum `name` is a constant of, within the current `UnitVariantScope`.
pub fn constant_enum_of(name: &str) -> Option<String> {
    UNIT_VARIANTS.with(|m| m.borrow().get(name).cloned())
}

/// The multiplicity a reader of the generated code assumes for `s` when
/// nothing says otherwise.
///
/// A case of an enum that carries no data — neither its own fields nor any
/// inherited from an ancestor — generates a unique value (`Color::Red`,
/// `case red`, an iota constant), which means `one`. Every other sig
/// generates a type with many possible values, which means a plain `sig`.
/// `extract` applies the same rule, so the two agree without a marker.
pub fn natural_sig_multiplicity(ir: &OxidtrIR, s: &StructureNode) -> SigMultiplicity {
    let find = |name: &str| ir.structures.iter().find(|x| x.name == name);
    let Some(parent) = s.parent.as_deref().and_then(find) else {
        return SigMultiplicity::Default;
    };
    if !parent.is_enum || !s.fields.is_empty() {
        return SigMultiplicity::Default;
    }
    let mut ancestor = Some(parent);
    let mut seen = 0;
    while let Some(a) = ancestor {
        if !a.fields.is_empty() || seen > ir.structures.len() {
            return SigMultiplicity::Default;
        }
        seen += 1;
        ancestor = a.parent.as_deref().and_then(find);
    }
    SigMultiplicity::One
}

/// Marker lines stating what the target's types cannot, for every sig in
/// `ir` that `include` accepts:
///
/// - the multiplicity of a sig whose multiplicity differs from
///   `natural_sig_multiplicity` (`oxidtr: lone sig Cache`) — no target's type
///   system says how many atoms a sig has (#156);
/// - the multiplicity of a map field other than `set`
///   (`oxidtr: field Holder.table: one Str -> Int`) — every target writes
///   `one K -> V` and `set K -> V` as the same map type (#153).
///
/// Each line names what it is about, so `extract` applies it wherever in the
/// output it stands. `comment` is the target's line-comment token.
pub fn model_markers(
    ir: &OxidtrIR,
    include: &dyn Fn(&StructureNode) -> bool,
    comment: &str,
) -> String {
    let mut out = String::new();
    for s in ir.structures.iter().filter(|s| include(s)) {
        if s.sig_multiplicity == natural_sig_multiplicity(ir, s) {
            continue;
        }
        let (decl, meaning) = match s.sig_multiplicity {
            SigMultiplicity::One => ("one sig", "exactly one atom"),
            SigMultiplicity::Lone => ("lone sig", "at most one atom"),
            SigMultiplicity::Some => ("some sig", "at least one atom"),
            SigMultiplicity::Default => ("sig", "any number of atoms, although the case carries no data"),
        };
        out.push_str(&format!("{comment} oxidtr: {decl} {} — {meaning}\n", s.name));
    }
    for s in ir.structures.iter().filter(|s| include(s)) {
        let Some(parent) = s.parent.as_deref() else { continue };
        if ir.structures.iter().any(|p| p.name == parent && !p.is_enum) {
            // `flatten_concrete_inheritance` repeated the parent's fields
            // here; this line is what says they belong to the parent.
            out.push_str(&format!("{comment} oxidtr: {} extends {parent} — the fields of {parent} are repeated here\n", s.name));
        }
    }
    for s in ir.structures.iter().filter(|s| include(s) && s.is_enum && !s.fields.is_empty()) {
        let names: Vec<&str> = s.fields.iter().map(|f| f.name.as_str()).collect();
        out.push_str(&format!("{comment} oxidtr: {} declares {} — each case repeats them\n", s.name, names.join(", ")));
    }
    for s in ir.structures.iter().filter(|s| include(s)) {
        for f in &s.fields {
            let case_of = ir.structures.iter().find(|t| t.name == f.target)
                .and_then(|t| t.parent.as_deref())
                .is_some_and(|p| ir.structures.iter().any(|q| q.name == p && q.is_enum));
            let unstated_map = f.value_type.is_some() && f.mult != Multiplicity::Set;
            if !case_of && !unstated_map {
                continue;
            }
            let mult = match f.mult {
                Multiplicity::One => "one",
                Multiplicity::Lone => "lone",
                Multiplicity::Set => "set",
                Multiplicity::Seq => "seq",
            };
            let ty = match &f.value_type {
                Some(value) => format!("{} -> {value}", f.target),
                None => f.target.clone(),
            };
            let why = if case_of {
                "a case of an enum, which the field's type cannot name"
            } else {
                "a map type cannot state this multiplicity"
            };
            out.push_str(&format!("{comment} oxidtr: field {}.{}: {mult} {ty} — {why}\n", s.name, f.name));
        }
    }
    out
}

/// `ir` with every sig that extends a non-abstract sig also carrying the
/// fields it inherits, ancestors' first.
///
/// In Alloy an atom of `Derived extends Base` is an atom of `Base` and has
/// its fields. No backend emits a subtype relation for a non-abstract parent,
/// so the generated `Derived` lacked `Base`'s fields altogether, and a fact
/// reading `d.id` did not compile. The relation itself is kept in an
/// `oxidtr: Derived extends Base` marker (#157).
pub fn flatten_concrete_inheritance(ir: &OxidtrIR) -> OxidtrIR {
    let find = |name: &str| ir.structures.iter().find(|s| s.name == name);
    let mut out = ir.clone();
    for s in &mut out.structures {
        let mut inherited: Vec<IRField> = Vec::new();
        let mut parent = s.parent.as_deref().and_then(find);
        let mut seen = 0;
        while let Some(p) = parent.filter(|p| !p.is_enum) {
            if seen > ir.structures.len() { break; }
            seen += 1;
            let mut fields = p.fields.clone();
            fields.extend(inherited);
            inherited = fields;
            parent = p.parent.as_deref().and_then(find);
        }
        if inherited.is_empty() { continue; }
        inherited.retain(|f| !s.fields.iter().any(|own| own.name == f.name));
        inherited.extend(std::mem::take(&mut s.fields));
        s.fields = inherited;
    }
    out
}

/// Marker lines for every `seq` field in `ir`, for a target that writes `set`
/// and `seq` as the same type (Lean's `List`, #158).
pub fn seq_field_markers(ir: &OxidtrIR, comment: &str) -> String {
    let mut out = String::new();
    for s in &ir.structures {
        for f in s.fields.iter().filter(|f| f.mult == Multiplicity::Seq && f.value_type.is_none()) {
            out.push_str(&format!(
                "{comment} oxidtr: field {}.{}: seq {} — `List` is also how a set is written\n",
                s.name, f.name, f.target,
            ));
        }
    }
    out
}

/// `content` with the multiplicity markers for `ir` appended, if any.
pub fn with_model_markers(content: String, ir: &OxidtrIR, comment: &str) -> String {
    append_markers(content, &model_markers(ir, &|_| true, comment))
}

/// `content` followed by `markers`, separated by a blank line.
pub fn append_markers(mut content: String, markers: &str) -> String {
    if markers.is_empty() {
        return content;
    }
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push('\n');
    content.push_str(markers);
    content
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

/// Make a name transform injective over the whole model.
///
/// Every backend builds a domain local or a factory name by transforming a
/// sig's name — lowercasing a leading character, appending `s`, prefixing
/// `default`. Alloy is case-sensitive and those transforms are not injective,
/// so `sig Foo` and `sig foo` both asked for `foos` and for `default_foo`. Six
/// backends failed to compile; Rust shadowed the local instead, and the
/// quantifier ranged over `foo` for both binders — `all a: Foo` never touched
/// `Foo`, and the test passed while proving nothing (#112).
///
/// Names are claimed in declaration order, which is stable, and a name already
/// spoken for takes the lowest free numeric suffix — repeating until the
/// candidate is free, so a suffix cannot land on a name that is itself taken.
/// A model whose names do not collide is left exactly as it was.
///
/// These names are internal to the generated code: neither `extract` nor
/// `check` reads a local or a factory, so renaming one costs nothing. A *field*
/// name is compared against the model by both, and a numeric suffix is not
/// reversible there — `Item2` is itself a legal Alloy name — which is why
/// `field_name_collisions` reports those rather than renaming them.
pub fn disambiguate<F>(name: &str, ir: &OxidtrIR, render: F) -> String
where
    F: Fn(&str) -> String,
{
    disambiguate_in(name, ir.structures.iter().map(|s| s.name.as_str()), render)
}

/// `disambiguate` over an explicit scope.
///
/// The competing names are not always the model's sigs. A Rust test is named
/// after a sig *and* a field — `anomaly_unconstrained_box_item` — so `item` and
/// `Item` collide there even though Rust emits both field names verbatim and
/// the struct itself is fine. The scope for that name is one sig's field list,
/// not the model's structures.
///
/// `universe` must be in declaration order; that is what makes the allocation
/// deterministic.
pub fn disambiguate_in<'a, I, F>(name: &str, universe: I, render: F) -> String
where
    I: IntoIterator<Item = &'a str>,
    F: Fn(&str) -> String,
{
    let mut taken: HashSet<String> = HashSet::new();
    for member in universe {
        let base = render(member);
        let mut candidate = base.clone();
        let mut n = 1usize;
        while taken.contains(&candidate) {
            n += 1;
            candidate = format!("{base}{n}");
        }
        if member == name {
            return candidate;
        }
        taken.insert(candidate);
    }
    // Not in the scope — a binder or a synthetic name the model does not own.
    render(name)
}

/// What kind of declaration two model names collapsed onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionKind {
    /// Two fields of one sig.
    Field,
    /// Two children of one abstract sig, emitted as cases of one enum.
    Variant,
}

impl std::fmt::Display for CollisionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollisionKind::Field => write!(f, "field"),
            CollisionKind::Variant => write!(f, "variant"),
        }
    }
}

/// Two declarations a target's naming convention cannot tell apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCollision {
    pub kind: CollisionKind,
    /// The sig declaring the fields, or the abstract sig owning the variants.
    pub sig: String,
    /// The single name the target would emit for all of them.
    pub emitted: String,
    /// The model's own names, in declaration order.
    pub sources: Vec<String>,
}

impl std::fmt::Display for NameCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} of `{}`: {} both become `{}`",
            self.kind, self.sig,
            self.sources.iter().map(|s| format!("`{s}`"))
                .collect::<Vec<_>>().join(" and "),
            self.emitted)
    }
}

/// Declarations whose emitted names collapse under `target`'s conventions.
///
/// Unlike a local or a factory, these names round-trip: `extract` reads them
/// back out of the generated source and `check` compares them against the
/// model. Disambiguating with a suffix would break both, because the suffixed
/// name is itself a legal Alloy name and nothing can tell the two apart
/// afterwards. So the collision is reported and `generate` refuses, naming the
/// model's own names so the author can act on them.
///
/// This is per target, not global:
///
/// - **fields** — Go and C# capitalize a field's leading character and Lean
///   lower-camels it, so each collapses one case variant onto the other. Rust,
///   TypeScript, Kotlin, Java and Swift emit Alloy's own casing into
///   case-sensitive languages and have nothing to collapse.
/// - **variants** — Swift folds an abstract sig's children into `case` names
///   and Lean into inductive constructors, both lower-camelled. The other six
///   keep the child's name verbatim.
///
/// Rejecting a model everywhere would fail one that most targets handle
/// correctly, so `generate --target ts` still succeeds where `--target cs`
/// refuses.
pub fn round_trip_collisions(ir: &OxidtrIR, target: &str) -> Vec<NameCollision> {
    let field_render: Option<fn(&str) -> String> = match target {
        "go" => Some(go::expr_translator::capitalize),
        "csharp" | "cs" => Some(csharp::expr_translator::capitalize),
        "lean" => Some(lean::expr_translator::lean_field),
        _ => None,
    };
    let variant_render: Option<fn(&str) -> String> = match target {
        "swift" => Some(swift::to_swift_case_name),
        "lean" => Some(lean::expr_translator::lean_field),
        _ => None,
    };

    let mut out = Vec::new();
    if let Some(render) = field_render {
        for s in &ir.structures {
            let names: Vec<&str> = s.fields.iter().map(|f| f.name.as_str()).collect();
            out.extend(collapsed(CollisionKind::Field, &s.name, &names, render));
        }
    }
    if let Some(render) = variant_render {
        for parent in ir.structures.iter().filter(|s| s.is_enum) {
            let names: Vec<&str> = ir.structures.iter()
                .filter(|c| c.parent.as_deref() == Some(parent.name.as_str()))
                .map(|c| c.name.as_str())
                .collect();
            out.extend(collapsed(CollisionKind::Variant, &parent.name, &names, render));
        }
    }
    out
}

/// Group `names` by what `render` makes of them, keeping only the groups with
/// more than one member.
///
/// A `Vec` rather than a map: the result reaches the author as an error
/// message, so its order has to come from the model rather than from hashing.
fn collapsed<F>(kind: CollisionKind, sig: &str, names: &[&str], render: F) -> Vec<NameCollision>
where
    F: Fn(&str) -> String,
{
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for name in names {
        let emitted = render(name);
        match groups.iter_mut().find(|(e, _)| *e == emitted) {
            Some((_, taken)) => taken.push((*name).to_string()),
            None => groups.push((emitted, vec![(*name).to_string()])),
        }
    }
    groups.into_iter()
        .filter(|(_, taken)| taken.len() > 1)
        .map(|(emitted, sources)| NameCollision {
            kind, sig: sig.to_string(), emitted, sources,
        })
        .collect()
}
