use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use crate::backend::{TargetLang, resolve_type, is_native_type_alias};
use crate::backend::type_env::{TypeEnv, expr_sig, resolve_field};
use std::collections::{HashSet, BTreeSet};

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

/// Extract all RTC (`*field`) field usages from an expression.
pub fn extract_rtc_fields(expr: &Expr, ir: &OxidtrIR) -> Vec<TCField> {
    let mut fields = Vec::new();
    collect_rtc_fields(expr, ir, &mut fields);
    fields.sort_by(|a, b| a.field_name.cmp(&b.field_name));
    fields.dedup();
    fields
}

fn collect_rtc_fields(expr: &Expr, ir: &OxidtrIR, out: &mut Vec<TCField>) {
    match expr {
        Expr::ReflexiveClosure(inner) => {
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
            collect_rtc_fields(inner, ir, out);
        }
        Expr::TransitiveClosure(inner) => collect_rtc_fields(inner, ir, out),
        Expr::FieldAccess { base, .. } => collect_rtc_fields(base, ir, out),
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. } => {
            collect_rtc_fields(left, ir, out);
            collect_rtc_fields(right, ir, out);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) => collect_rtc_fields(inner, ir, out),
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings { collect_rtc_fields(&b.domain, ir, out); }
            collect_rtc_fields(body, ir, out);
        }
        Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            collect_rtc_fields(left, ir, out);
            collect_rtc_fields(right, ir, out);
        }
        Expr::MultFormula { expr: inner, .. } => collect_rtc_fields(inner, ir, out),
        Expr::Prime(inner) => collect_rtc_fields(inner, ir, out),
        Expr::TemporalUnary { expr: inner, .. } => collect_rtc_fields(inner, ir, out),
        Expr::TemporalBinary { left, right, .. } => {
            collect_rtc_fields(left, ir, out);
            collect_rtc_fields(right, ir, out);
        }
        Expr::FunApp { receiver, args, .. } => {
            if let Some(r) = receiver { collect_rtc_fields(r, ir, out); }
            for arg in args { collect_rtc_fields(arg, ir, out); }
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
}

fn collect_tc_fields(expr: &Expr, ir: &OxidtrIR, out: &mut Vec<TCField>) {
    match expr {
        Expr::TransitiveClosure(inner) | Expr::ReflexiveClosure(inner) => {
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

pub use crate::backend::type_env::collect_sig_names;

pub fn extract_params(expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR) -> Vec<(String, String)> {
    let mut params = BTreeSet::new();
    collect_params(expr, sig_names, ir, &mut params);
    params.into_iter().collect()
}

/// The name of the sample domain a bare sig reference stands for.
///
/// In Alloy a sig name in an expression is the set of its atoms — `#P`,
/// `n.c = Config`, `x in Person`. Go has no such value: the name is a type, so
/// `len(P)` and `contains(Person, x)` do not compile. What the reference
/// denotes is the slice the caller materialised for it (#105).
///
/// A variant of a sum interface is excluded: it is a *case*, and a comparison
/// against it is a type assertion.
fn whole_sig_extent(expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR) -> Option<String> {
    let Expr::VarRef(name) = expr else { return None };
    if !sig_names.contains(name) { return None; }
    if crate::backend::is_native_type_alias(name) { return None; }
    if crate::backend::variant_parent(ir, name).is_some() { return None; }
    Some(to_camel_plural(name))
}

/// `Sig.field`, as the union of `field` over every atom of `Sig`.
///
/// Alloy's `Schedule.Morning` is the *relational image*, not member access on
/// a type — `Schedule` is a Go type, so the receiver form does not compile.
/// #105 rendered a sig name as its materialised extent wherever the name
/// stands alone; this is the position it left out (#142).
///
/// Go infers nothing about a func literal, so the closure's receiver and
/// element types are both written out.
fn relational_image(
    expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv,
) -> Option<String> {
    let Expr::FieldAccess { base, field } = expr else { return None };
    let Expr::VarRef(sig) = base.as_ref() else { return None };
    let extent = whole_sig_extent_in(base, sig_names, ir, env)?;
    let f = resolve_field(base, field, sig_names, ir, env)?;
    let name = capitalize(field);
    // `oneOf`/`loneOf` lift a singleton relation into the slice the quantifier
    // helpers already take.
    let elem = match f.mult {
        Multiplicity::Set | Multiplicity::Seq => format!("s.{name}"),
        Multiplicity::One => format!("oneOf(s.{name})"),
        Multiplicity::Lone => format!("loneOf(s.{name})"),
    };
    let elem_ty = crate::backend::resolve_type(crate::backend::TargetLang::Go, &f.target);
    Some(format!("flatMap({extent}, func(s {sig}) []{elem_ty} {{ return {elem} }})"))
}

/// `whole_sig_extent`, but also refusing a name a binder has shadowed.
fn whole_sig_extent_in(
    expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv,
) -> Option<String> {
    match expr {
        Expr::VarRef(name) if env.sig_of(name).is_some() => None,
        _ => whole_sig_extent(expr, sig_names, ir),
    }
}

/// `x = Variant` / `x != Variant`, as a type assertion on the sum interface.
///
/// A `one sig` extending an abstract sig is one atom, so equality with it asks
/// which case the other side is. Go models the sum as an interface and each
/// case as a struct type, so the bare name `Low` is a type (#105).
fn variant_case_test<F>(
    left: &Expr, right: &Expr, ir: &OxidtrIR, negated: bool, ti: &F,
) -> Option<String>
where F: Fn(&Expr) -> String {
    let variant_of = |e: &Expr| match e {
        Expr::VarRef(name) => crate::backend::variant_parent(ir, name).map(|_| name.clone()),
        _ => None,
    };
    let (variant, subject) = match (variant_of(left), variant_of(right)) {
        (Some(_), Some(_)) | (None, None) => return None,
        (Some(v), None) => (v, right),
        (None, Some(v)) => (v, left),
    };
    let test = format!("isVariant[{variant}]({})", ti(subject));
    Some(if negated { format!("!{test}") } else { test })
}

/// `x = Sig` / `x != Sig`, as membership in the sig's materialised extent.
fn whole_sig_membership<F>(
    left: &Expr, right: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv,
    negated: bool, ti: &F,
) -> Option<String>
where F: Fn(&Expr) -> String {
    let l = whole_sig_extent_in(left, sig_names, ir, env);
    let r = whole_sig_extent_in(right, sig_names, ir, env);
    let (extent, subject) = match (l, r) {
        (Some(_), Some(_)) | (None, None) => return None,
        (Some(e), None) => (e, right),
        (None, Some(e)) => (e, left),
    };
    let test = format!("contains({extent}, {})", ti(subject));
    Some(if negated { format!("!{test}") } else { test })
}

fn collect_params(
    expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR,
    params: &mut BTreeSet<(String, String)>,
) {
    // Positions where `translate_inner` renders a bare sig name as its extent.
    // The two must agree: a domain rendered but never materialised does not
    // compile, and one materialised but never rendered is an unused variable —
    // which in Go does not compile either.
    let extent = |e: &Expr, params: &mut BTreeSet<(String, String)>| {
        if let (Expr::VarRef(name), Some(plural)) = (e, whole_sig_extent(e, sig_names, ir)) {
            params.insert((plural, name.clone()));
        }
    };
    match expr {
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings {
                if let Expr::VarRef(name) = &b.domain {
                    if sig_names.contains(name) {
                        params.insert((to_camel_plural(name), name.clone()));
                    }
                }
                collect_params(&b.domain, sig_names, ir, params);
            }
            collect_params(body, sig_names, ir, params);
        }
        Expr::Comparison { op, left, right } => {
            match op {
                CompareOp::Eq | CompareOp::NotEq => {
                    extent(left, params);
                    extent(right, params);
                }
                CompareOp::In => extent(right, params),
                _ => {}
            }
            collect_params(left, sig_names, ir, params);
            collect_params(right, sig_names, ir, params);
        }
        Expr::BinaryLogic { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            collect_params(left, sig_names, ir, params);
            collect_params(right, sig_names, ir, params);
        }
        Expr::Cardinality(inner) => {
            extent(inner, params);
            collect_params(inner, sig_names, ir, params);
        }
        Expr::MultFormula { expr: inner, .. } => {
            extent(inner, params);
            collect_params(inner, sig_names, ir, params);
        }
        Expr::Not(inner) | Expr::TransitiveClosure(inner) | Expr::ReflexiveClosure(inner) => {
            collect_params(inner, sig_names, ir, params);
        }
        Expr::FieldAccess { base, .. } => {
            // The image reads the field across the sig's extent, so the domain
            // has to be declared here as well (#142).
            extent(base, params);
            collect_params(base, sig_names, ir, params);
        }
        Expr::Prime(inner) => collect_params(inner, sig_names, ir, params),
        Expr::TemporalUnary { expr: inner, .. } => collect_params(inner, sig_names, ir, params),
        Expr::TemporalBinary { left, right, .. } => {
            collect_params(left, sig_names, ir, params);
            collect_params(right, sig_names, ir, params);
        }
        Expr::FunApp { receiver, args, .. } => {
            if let Some(r) = receiver { collect_params(r, sig_names, ir, params); }
            for arg in args { collect_params(arg, sig_names, ir, params); }
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
}

pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR) -> String {
    translate_with_env(expr, ir, &TypeEnv::new())
}

/// Translate in an explicit scope — an operation's parameters, for instance.
/// A field access is typed through the binding of its base, so a caller holding
/// free variables has to say what they range over.
pub fn translate_with_env(expr: &Expr, ir: &OxidtrIR, env: &TypeEnv) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir, env)
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

        // Alloy's implicit receiver in `fun Sig.op { this... }`. A derived
        // field is emitted as a method with receiver `s`.
        Expr::VarRef(name) if name == "this" => "s".to_string(),

        Expr::VarRef(name) => name.clone(),

        // `Schedule.Morning` reads a field across every atom of the sig (#142).
        Expr::FieldAccess { .. } if relational_image(expr, sig_names, ir, env).is_some() => {
            relational_image(expr, sig_names, ir, env).unwrap()
        }

        Expr::FieldAccess { base, field } => {
            format!("{}.{}", ti(base, false), capitalize(field))
        }

        // Alloy `#e` is an Int, which Go models as int64; `len` yields `int`,
        // so comparing it against an `int64` field — or returning it from a
        // `fun ...: one Int` — does not type-check without the conversion.
        Expr::Cardinality(inner) => format!("int64(len({}))",
            whole_sig_extent_in(inner, sig_names, ir, env).unwrap_or_else(|| ti(inner, false))),

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Tc{}({})", capitalize(field), ti(base, false))
            } else {
                format!("transitiveClosure({})", ti(inner, false))
            }
        }

        Expr::ReflexiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Rtc{}({})", capitalize(field), ti(base, false))
            } else {
                format!("reflexiveTransitiveClosure({})", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                // Go refuses `==` on a struct containing a slice, and most sigs
                // lower to exactly that. Keep `==` where both operands are
                // statically primitive; otherwise defer to the DeepEqual-based
                // `equal` helper, which is correct for every type.
                CompareOp::Eq | CompareOp::NotEq => {
                    let negated = matches!(op, CompareOp::NotEq);
                    // `v.Level = Low` asks which case an atom is; `Low` is a
                    // struct type, so the question is a type assertion (#105).
                    if let Some(s) = variant_case_test(left, right, ir, negated, &|e| ti(e, false)) {
                        return s;
                    }
                    // `n.C = Config` compares an atom against the sig's whole
                    // extent; for a `one sig` that is membership in it.
                    if let Some(s) = whole_sig_membership(left, right, sig_names, ir, env, negated, &|e| ti(e, false)) {
                        return s;
                    }
                    let (l, r) = (ti(left, false), ti(right, false));
                    let primitive = is_primitive_operand(left, sig_names, ir, env)
                        && is_primitive_operand(right, sig_names, ir, env);
                    match (primitive, negated) {
                        (true, false) => format!("{l} == {r}"),
                        (true, true) => format!("{l} != {r}"),
                        (false, false) => format!("equal({l}, {r})"),
                        (false, true) => format!("!equal({l}, {r})"),
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
                    let r = whole_sig_extent_in(right, sig_names, ir, env)
                        .unwrap_or_else(|| ti(right, false));
                    // A set-valued left operand makes `in` subset containment,
                    // not element membership.
                    let left_is_set = matches!(left.as_ref(), Expr::FieldAccess { base, field }
                        if resolve_field(base, field, sig_names, ir, env)
                            .is_some_and(|f| matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)));
                    if left_is_set {
                        format!("isSubset({l}, {r})")
                    } else {
                        format!("contains({r}, {l})")
                    }
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
            let inner = env.extended(bindings, sig_names, ir);
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
            // `some Person` is about the sig's extent, which is a slice — not a
            // nilable pointer (#105); so is a relational image (#142).
            let collection = whole_sig_extent_in(inner, sig_names, ir, env)
                .or_else(|| relational_image(inner, sig_names, ir, env));
            if let Some(e) = collection {
                return match kind {
                    crate::parser::ast::QuantKind::Some => format!("len({e}) > 0"),
                    crate::parser::ast::QuantKind::No => format!("len({e}) == 0"),
                    _ => e,
                };
            }
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
        // Enter this binding before the next domain is typed — the bindings are
        // sequential, and this loop interleaves scoping with rendering.
        scope = scope.extended(std::slice::from_ref(b), sig_names, ir);
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

pub fn to_camel_plural(name: &str) -> String {
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
