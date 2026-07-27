use crate::parser::ast::*;
use crate::ir::nodes::{OxidtrIR, IRField};
use crate::backend::{TargetLang, resolve_type, is_native_type_alias};
use std::collections::{HashSet, BTreeSet, HashMap};

/// TC field info.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TCField {
    pub field_name: String,
    pub sig_name: String,
    pub mult: Multiplicity,
}

pub fn extract_tc_fields(expr: &Expr, ir: &OxidtrIR) -> Vec<TCField> {
    let mut fields = Vec::new();
    collect_tc_fields(expr, ir, &mut fields);
    fields.sort_by(|a, b| a.field_name.cmp(&b.field_name));
    fields.dedup();
    fields
}

fn collect_tc_fields(expr: &Expr, ir: &OxidtrIR, out: &mut Vec<TCField>) {
    match expr {
        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                for s in &ir.structures {
                    for f in &s.fields {
                        if f.name == *field && f.target == s.name {
                            out.push(TCField {
                                field_name: field.clone(),
                                sig_name: s.name.clone(),
                                mult: f.mult.clone(),
                            });
                        }
                    }
                }
            }
            collect_tc_fields(inner, ir, out);
        }
        Expr::FieldAccess { base, .. } => collect_tc_fields(base, ir, out),
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. } => {
            collect_tc_fields(left, ir, out);
            collect_tc_fields(right, ir, out);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) => collect_tc_fields(inner, ir, out),
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings { collect_tc_fields(&b.domain, ir, out); }
            collect_tc_fields(body, ir, out);
        }
        Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            collect_tc_fields(left, ir, out);
            collect_tc_fields(right, ir, out);
        }
        Expr::MultFormula { expr: inner, .. } => collect_tc_fields(inner, ir, out),
        Expr::Prime(inner) => collect_tc_fields(inner, ir, out),
        Expr::TemporalUnary { expr: inner, .. } => collect_tc_fields(inner, ir, out),
        Expr::TemporalBinary { left, right, .. } => {
            collect_tc_fields(left, ir, out);
            collect_tc_fields(right, ir, out);
        }
        Expr::FunApp { receiver, args, .. } => {
            if let Some(r) = receiver { collect_tc_fields(r, ir, out); }
            for arg in args { collect_tc_fields(arg, ir, out); }
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
}

pub fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

pub fn extract_params(expr: &Expr, sig_names: &HashSet<String>) -> Vec<(String, String)> {
    let mut params = BTreeSet::new();
    collect_params(expr, sig_names, &mut params);
    params.into_iter().collect()
}

fn collect_params(expr: &Expr, sig_names: &HashSet<String>, params: &mut BTreeSet<(String, String)>) {
    match expr {
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings {
                if let Expr::VarRef(name) = &b.domain {
                    if sig_names.contains(name) {
                        params.insert((to_camel_plural(name), name.clone()));
                    }
                }
                collect_params(&b.domain, sig_names, params);
            }
            collect_params(body, sig_names, params);
        }
        Expr::BinaryLogic { left, right, .. } | Expr::Comparison { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            collect_params(left, sig_names, params);
            collect_params(right, sig_names, params);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            collect_params(inner, sig_names, params);
        }
        Expr::FieldAccess { base, .. } => collect_params(base, sig_names, params),
        Expr::MultFormula { expr: inner, .. } => collect_params(inner, sig_names, params),
        Expr::Prime(inner) => collect_params(inner, sig_names, params),
        Expr::TemporalUnary { expr: inner, .. } => collect_params(inner, sig_names, params),
        Expr::TemporalBinary { left, right, .. } => {
            collect_params(left, sig_names, params);
            collect_params(right, sig_names, params);
        }
        Expr::FunApp { receiver, args, .. } => {
            if let Some(r) = receiver { collect_params(r, sig_names, params); }
            for arg in args { collect_params(arg, sig_names, params); }
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
}

pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir, &TypeEnv::new())
}

/// Maps a quantifier-bound variable to the sig it ranges over.
///
/// Resolving a field through the *binding* is the only sound way to type an
/// expression: looking a field up by name across every sig silently picks the
/// wrong one whenever two sigs share a field name, which produced both
/// non-compiling closures and vet-clean-but-always-false membership tests.
type TypeEnv = HashMap<String, String>;

/// The sig name an expression denotes, or `None` if it is not a sig-valued
/// expression (a literal, a native scalar, an unresolvable domain).
fn expr_sig(expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv) -> Option<String> {
    match expr {
        Expr::VarRef(name) => env.get(name).cloned()
            .or_else(|| sig_names.contains(name).then(|| name.clone())),
        Expr::FieldAccess { base, field } => {
            resolve_field(base, field, sig_names, ir, env).map(|f| f.target.clone())
        }
        // `^f` ranges over the same sig as `f` itself, and lowers to a
        // slice-returning Tc* helper, so it needs no singleton lifting.
        Expr::TransitiveClosure(inner) => expr_sig(inner, sig_names, ir, env),
        _ => None,
    }
}

/// Resolve `base.field` to the actual field declaration, via `base`'s sig.
fn resolve_field<'a>(
    base: &Expr,
    field: &str,
    sig_names: &HashSet<String>,
    ir: &'a OxidtrIR,
    env: &TypeEnv,
) -> Option<&'a IRField> {
    let mut sig = expr_sig(base, sig_names, ir, env)?;
    // Walk the inheritance chain: a field declared on an abstract parent is
    // reachable through the child.
    loop {
        let st = ir.structures.iter().find(|s| s.name == sig)?;
        if let Some(f) = st.fields.iter().find(|f| f.name == field) {
            return Some(f);
        }
        sig = st.parent.clone()?;
    }
}

fn translate_inner(
    expr: &Expr,
    parens_if_complex: bool,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
    env: &TypeEnv,
) -> String {
    let ti = |e: &Expr, p: bool| translate_inner(e, p, sig_names, ir, env);

    let result = match expr {
        Expr::IntLiteral(n) => n.to_string(),

        Expr::VarRef(name) => name.clone(),

        Expr::FieldAccess { base, field } => {
            format!("{}.{}", ti(base, false), capitalize(field))
        }

        Expr::Cardinality(inner) => format!("len({})", ti(inner, false)),

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Tc{}({})", capitalize(field), ti(base, false))
            } else {
                format!("transitiveClosure({})", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                // Go refuses `==` on a struct containing a slice, and most sigs
                // lower to exactly that. Keep `==` where both operands are
                // statically primitive; otherwise defer to the DeepEqual-based
                // `equal` helper, which is correct for every type.
                CompareOp::Eq => {
                    let (l, r) = (ti(left, false), ti(right, false));
                    if is_primitive_operand(left, sig_names, ir, env) && is_primitive_operand(right, sig_names, ir, env) {
                        format!("{l} == {r}")
                    } else {
                        format!("equal({l}, {r})")
                    }
                }
                CompareOp::NotEq => {
                    let (l, r) = (ti(left, false), ti(right, false));
                    if is_primitive_operand(left, sig_names, ir, env) && is_primitive_operand(right, sig_names, ir, env) {
                        format!("{l} != {r}")
                    } else {
                        format!("!equal({l}, {r})")
                    }
                }
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} <= {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} >= {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    let l = ti(left, false);
                    if let Expr::FieldAccess { base, field } = right.as_ref() {
                        // Resolve through the base's own sig: picking the first
                        // same-named field would treat a `set` as `lone` and
                        // silently emit an always-false equality check.
                        if let Some(f) = resolve_field(base, field, sig_names, ir, env) {
                            // A singleton relation contains exactly its own
                            // value; contains() takes a slice and would be
                            // vet-clean but always false here.
                            if matches!(f.mult, Multiplicity::Lone | Multiplicity::One) {
                                // `lone` lowers to *T, so `==` against the bare
                                // value is a type error; equal() derefs first.
                                let r_base = ti(base, false);
                                return format!("equal({r_base}.{}, {l})", capitalize(field));
                            }
                        }
                    }
                    let r = ti(right, false);
                    format!("contains({r}, {l})")
                }
            }
        }

        Expr::BinaryLogic { op, left, right } => match op {
            LogicOp::And     => format!("{} && {}", ti(left, false), ti(right, false)),
            LogicOp::Or      => format!("{} || {}", ti(left, false), ti(right, false)),
            LogicOp::Implies => format!("!{} || {}", ti(left, true), ti(right, false)),
            LogicOp::Iff     => format!("{} == {}", ti(left, true), ti(right, true)),
        },

        Expr::Not(inner) => format!("!{}", ti(inner, true)),

        Expr::Quantifier { kind, bindings, body } => {
            // Bindings are sequential: `all b: Box, x: b.items | ..` binds `b`
            // before `x`'s domain is resolved, so extend as we go.
            let mut inner = env.clone();
            for bd in bindings {
                if let Some(sig) = expr_sig(&bd.domain, sig_names, ir, &inner) {
                    for v in &bd.vars { inner.insert(v.clone(), sig.clone()); }
                }
            }
            let b = translate_inner(body, false, sig_names, ir, &inner);
            build_nested_quantifier(kind, bindings, &b, sig_names, ir, env)
        }

        Expr::SetOp { op, left, right } => {
            let l = ti(left, false);
            let r = ti(right, false);
            match op {
                SetOpKind::Union => format!("union({l}, {r})"),
                SetOpKind::Intersection => format!("intersection({l}, {r})"),
                SetOpKind::Difference => format!("difference({l}, {r})"),
            }
        }

        Expr::Product { left, right } => {
            let l = ti(left, false);
            let r = ti(right, false);
            format!("Pair{{{l}, {r}}}")
        }

        Expr::MultFormula { kind, expr: inner } => {
            let translated = ti(inner, false);
            match kind {
                crate::parser::ast::QuantKind::Some => format!("{translated} != nil"),
                crate::parser::ast::QuantKind::No => format!("{translated} == nil"),
                _ => format!("{kind:?}({translated})"),
            }
        }

        // Alloy 6: prime operator — next-state reference
        Expr::Prime(inner) => {
            match inner.as_ref() {
                Expr::FieldAccess { base, field } => {
                    let base_str = ti(base, false);
                    format!("{base_str}.Next{}", capitalize(field))
                }
                Expr::VarRef(name) => format!("next{}", capitalize(name)),
                _ => format!("{}.Next()", ti(inner, false)),
            }
        }
        // Alloy 6: temporal unary operators — translate inner expression
        Expr::TemporalUnary { expr: inner, .. } => ti(inner, false),
        // Alloy 6: temporal binary operators — translate both sides
        Expr::TemporalBinary { left, right, .. } => {
            let l = ti(left, false);
            let r = ti(right, false);
            format!("{l} && {r}")
        }
        Expr::FunApp { name, receiver, args } => {
            translate_fun_app(name, receiver.as_deref(), args, |e| ti(e, false))
        }
    };

    if parens_if_complex && needs_parens(expr) {
        format!("({result})")
    } else {
        result
    }
}

/// Resolve the Go element type of a quantifier domain — the type that must be
/// written on the generated closure parameter (`func(p Person) bool`). Go, unlike
/// Rust, never infers a func literal's parameter type from context, so omitting
/// it is a syntax error rather than a style choice.
///
/// A bare sig domain (`p: Person`) resolves directly. A field domain
/// (`i: c.items`) is resolved by field name across the IR; when several sigs
/// share that field name they must agree on the target type, otherwise the
/// domain is ambiguous and we report `None` rather than guess.
/// True when the operand's static Go type is a primitive, so `==` is legal.
/// A sig lowers to a struct that may contain slice fields, and Go rejects `==`
/// on those outright ("struct containing []T cannot be compared").
fn is_primitive_operand(expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv) -> bool {
    match expr {
        Expr::IntLiteral(_) | Expr::Cardinality(_) => true,
        Expr::FieldAccess { base, field } => {
            match resolve_field(base, field, sig_names, ir, env) {
                // A map field's `target` names only its KEY type — the Go type
                // is a map, which Go can compare to nothing but nil.
                Some(f) => f.value_type.is_none()
                    && f.mult == Multiplicity::One
                    && is_native_type_alias(&f.target),
                None => false,
            }
        }
        _ => false,
    }
}

fn domain_element_type(
    domain: &Expr,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
    env: &TypeEnv,
) -> Option<String> {
    expr_sig(domain, sig_names, ir, env).map(|s| resolve_type(TargetLang::Go, &s))
}

/// The multiplicity of a field-access domain, so a singleton relation can be
/// lifted into the slice `forAll`/`exists` require. A bare sig domain is the
/// generated collection parameter and is already a slice.
fn domain_multiplicity(
    domain: &Expr,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
    env: &TypeEnv,
) -> Option<Multiplicity> {
    match domain {
        Expr::FieldAccess { base, field } => {
            resolve_field(base, field, sig_names, ir, env).map(|f| f.mult.clone())
        }
        _ => None,
    }
}

fn build_nested_quantifier(
    kind: &QuantKind,
    bindings: &[QuantBinding],
    body_str: &str,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
    env: &TypeEnv,
) -> String {
    let mut vars: Vec<(String, String, bool, String)> = Vec::new();
    let mut scope = env.clone();
    for b in bindings {
        let raw = if let Expr::VarRef(name) = &b.domain {
            if sig_names.contains(name) { to_camel_plural(name) }
            else { name.clone() }
        } else {
            translate_inner(&b.domain, false, sig_names, ir, &scope)
        };
        // forAll/exists take a slice, but Alloy also quantifies over singleton
        // relations — lift `one`/`lone` domains rather than emit a type error.
        let d = match domain_multiplicity(&b.domain, sig_names, ir, &scope) {
            Some(Multiplicity::One) => format!("oneOf({raw})"),
            Some(Multiplicity::Lone) => format!("loneOf({raw})"),
            _ => raw,
        };
        // `any` is Go's empty interface: a domain we cannot resolve still
        // parses, instead of emitting an outright syntax error.
        let elem_ty = domain_element_type(&b.domain, sig_names, ir, &scope)
            .unwrap_or_else(|| "any".to_string());
        if let Some(sig) = expr_sig(&b.domain, sig_names, ir, &scope) {
            for v in &b.vars { scope.insert(v.clone(), sig.clone()); }
        }
        for v in &b.vars {
            vars.push((v.clone(), d.clone(), b.disj, elem_ty.clone()));
        }
    }

    // `disj` scopes to its own declaration: in `all disj a,b: S, disj c,d: S`
    // only (a,b) and (c,d) must differ — a and c may be equal. Grouping by
    // rendered domain instead merged adjacent declarations and over-constrained.
    let mut disj_checks = Vec::new();
    for b in bindings {
        if !b.disj { continue; }
        for a in 0..b.vars.len() {
            for c in (a + 1)..b.vars.len() {
                disj_checks.push(format!("!equal({}, {})", b.vars[a], b.vars[c]));
            }
        }
    }

    let guarded_body = if disj_checks.is_empty() {
        body_str.to_string()
    } else {
        let guard = disj_checks.join(" && ");
        match kind {
            // A non-distinct tuple must not falsify a universal, so it yields
            // true. For `no`/`some` the tuple must instead simply not count,
            // and those are wrapped in exists(..) — so it yields false.
            QuantKind::All => format!("!({guard}) || ({body_str})"),
            QuantKind::Some | QuantKind::No => format!("({guard}) && ({body_str})"),
        }
    };

    let mut result = guarded_body;
    for idx in (0..vars.len()).rev() {
        let (ref var, ref domain, _, ref ty) = vars[idx];
        result = match kind {
            QuantKind::All => format!("forAll({domain}, func({var} {ty}) bool {{ return {body} }})", body = result),
            QuantKind::Some => format!("exists({domain}, func({var} {ty}) bool {{ return {body} }})", body = result),
            QuantKind::No => {
                if idx == 0 {
                    format!("!exists({domain}, func({var} {ty}) bool {{ return {body} }})", body = result)
                } else {
                    format!("exists({domain}, func({var} {ty}) bool {{ return {body} }})", body = result)
                }
            }
        };
    }
    result
}

fn translate_fun_app(name: &str, receiver: Option<&Expr>, args: &[Expr], translate: impl Fn(&Expr) -> String) -> String {
    if let Some(recv) = receiver {
        let op = match name {
            "plus" | "add" => Some("+"),
            "minus" | "sub" => Some("-"),
            "mul" => Some("*"),
            "div" => Some("/"),
            "rem" => Some("%"),
            _ => None,
        };
        if let (Some(op), Some(arg)) = (op, args.first()) {
            return format!("{} {} {}", translate(recv), op, translate(arg));
        }
        let a: Vec<_> = args.iter().map(&translate).collect();
        return format!("{}.{name}({})", translate(recv), a.join(", "));
    }
    let a: Vec<_> = args.iter().map(translate).collect();
    format!("{name}({})", a.join(", "))
}

fn needs_parens(expr: &Expr) -> bool {
    matches!(expr, Expr::Comparison { .. } | Expr::BinaryLogic { .. } | Expr::Quantifier { .. })
}

fn to_camel_plural(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out.push('s');
    out
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}
