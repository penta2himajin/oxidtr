use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use crate::backend::{is_native_type_alias, resolve_type, TargetLang};
use std::collections::{BTreeSet, HashSet};

use super::{cs_ident, cs_zero_value, compose_ident};

pub fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

/// A sig field that participates in a `^field` transitive-closure expression
/// somewhere in the model, keyed by owning sig × field name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TCField {
    pub field_name: String,
    /// The sig that *declares* the field — the `start` parameter's type.
    pub sig_name: String,
    /// The field's own target type — what the closure collects. Equal to
    /// `sig_name` for a self-referential field; the declaring sig's
    /// *ancestor* for a subtype-to-supertype closure (`Branch.parent: lone
    /// Node` where `Branch extends Node`). See #102 round 3 defect 3.
    pub target_type: String,
    pub mult: Multiplicity,
}

/// Whether `target` is `sig_name` itself or one of its Alloy `extends`
/// ancestors — the exact condition under which a `^field` closure over a
/// field declared on `sig_name` and typed `target` is well-defined.
fn is_self_or_ancestor(target: &str, sig_name: &str, ir: &OxidtrIR) -> bool {
    let mut cur = sig_name;
    loop {
        if cur == target {
            return true;
        }
        match ir.structures.iter().find(|s| s.name == cur).and_then(|s| s.parent.as_deref()) {
            Some(parent) => cur = parent,
            None => return false,
        }
    }
}

pub fn extract_tc_fields(expr: &Expr, ir: &OxidtrIR) -> Vec<TCField> {
    let mut fields = Vec::new();
    collect_tc_fields(expr, ir, &mut fields);
    fields.sort_by(|a, b| (&a.sig_name, &a.field_name).cmp(&(&b.sig_name, &b.field_name)));
    fields.dedup();
    fields
}

/// Extract all RTC (`*field`) field usages from an expression.
pub fn extract_rtc_fields(expr: &Expr, ir: &OxidtrIR) -> Vec<TCField> {
    let mut fields = Vec::new();
    collect_rtc_fields(expr, ir, &mut fields);
    fields.sort_by(|a, b| (&a.sig_name, &a.field_name).cmp(&(&b.sig_name, &b.field_name)));
    fields.dedup();
    fields
}

fn collect_rtc_fields(expr: &Expr, ir: &OxidtrIR, out: &mut Vec<TCField>) {
    match expr {
        Expr::ReflexiveClosure(inner) => {
            if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                for s in &ir.structures {
                    for f in &s.fields {
                        if f.name == *field && is_self_or_ancestor(&f.target, &s.name, ir) {
                            out.push(TCField {
                                field_name: field.clone(),
                                sig_name: s.name.clone(),
                                target_type: f.target.clone(),
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
                        if f.name == *field && is_self_or_ancestor(&f.target, &s.name, ir) {
                            out.push(TCField {
                                field_name: field.clone(),
                                sig_name: s.name.clone(),
                                target_type: f.target.clone(),
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

pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir)
}

/// Finalise the synthesized `next_x` post-state names that
/// `analyze::rewrite_prime_as_post_state` bakes into an AST into this
/// backend's own composed identifier (`nextX`).
///
/// This has to happen as a *targeted* pre-pass over exactly the rewritten
/// AST, not as a blanket rule inside the general `VarRef` translation arm:
/// the general arm runs on every expression this backend ever translates,
/// including ordinary asserts and invariants that never went through the
/// rewrite and may legitimately declare a variable whose Alloy name simply
/// starts with `next_` (`all next_c: Foo | next_c.tag = next_c.tag` has no
/// prime in it at all). A blanket "strip next_ and recompose" rule there
/// mistranslates that variable's every reference to a name nothing declares.
/// See #102 round 3 defect 1. Call this only on the output of
/// `rewrite_prime_as_post_state`, where every `next_`-prefixed `VarRef` is
/// known to be this rewrite's own synthesis.
pub fn finalize_post_state_idents(expr: &Expr) -> Expr {
    let r = finalize_post_state_idents;
    match expr {
        Expr::VarRef(name) => match name.strip_prefix("next_") {
            Some(rest) => Expr::VarRef(compose_ident("next", rest)),
            None => expr.clone(),
        },
        Expr::IntLiteral(_) => expr.clone(),
        Expr::FieldAccess { base, field } => Expr::FieldAccess {
            base: Box::new(r(base)),
            field: field.clone(),
        },
        Expr::Cardinality(inner) => Expr::Cardinality(Box::new(r(inner))),
        Expr::TransitiveClosure(inner) => Expr::TransitiveClosure(Box::new(r(inner))),
        Expr::ReflexiveClosure(inner) => Expr::ReflexiveClosure(Box::new(r(inner))),
        Expr::Comparison { op, left, right } => Expr::Comparison {
            op: op.clone(),
            left: Box::new(r(left)),
            right: Box::new(r(right)),
        },
        Expr::BinaryLogic { op, left, right } => Expr::BinaryLogic {
            op: op.clone(),
            left: Box::new(r(left)),
            right: Box::new(r(right)),
        },
        Expr::Not(inner) => Expr::Not(Box::new(r(inner))),
        Expr::MultFormula { kind, expr: inner } => Expr::MultFormula {
            kind: kind.clone(),
            expr: Box::new(r(inner)),
        },
        Expr::Quantifier { kind, bindings, body } => Expr::Quantifier {
            kind: kind.clone(),
            bindings: bindings.clone(),
            body: Box::new(r(body)),
        },
        Expr::SetOp { op, left, right } => Expr::SetOp {
            op: *op,
            left: Box::new(r(left)),
            right: Box::new(r(right)),
        },
        Expr::Product { left, right } => Expr::Product {
            left: Box::new(r(left)),
            right: Box::new(r(right)),
        },
        Expr::Prime(inner) => Expr::Prime(Box::new(r(inner))),
        Expr::TemporalUnary { op, expr: inner } => Expr::TemporalUnary {
            op: op.clone(),
            expr: Box::new(r(inner)),
        },
        Expr::TemporalBinary { op, left, right } => Expr::TemporalBinary {
            op: *op,
            left: Box::new(r(left)),
            right: Box::new(r(right)),
        },
        Expr::FunApp { name, receiver, args } => Expr::FunApp {
            name: name.clone(),
            receiver: receiver.as_ref().map(|x| Box::new(r(x))),
            args: args.iter().map(r).collect(),
        },
    }
}

fn field_mult(field_name: &str, ir: &OxidtrIR) -> Option<(Multiplicity, bool)> {
    for s in &ir.structures {
        for f in &s.fields {
            if f.name == field_name {
                let is_self_ref = f.target == s.name;
                return Some((f.mult.clone(), is_self_ref));
            }
        }
    }
    None
}

fn translate_inner(
    expr: &Expr,
    parens_if_complex: bool,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> String {
    let ti = |e: &Expr, p: bool| translate_inner(e, p, sig_names, ir);

    let result = match expr {
        Expr::IntLiteral(n) => n.to_string(),

        Expr::VarRef(name) => cs_ident(name),

        Expr::FieldAccess { base, field } => {
            format!("{}.{}", ti(base, false), capitalize(field))
        }

        Expr::Cardinality(inner) => format!("{}.Count", ti(inner, false)),

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Tc{}({})", capitalize(field), ti(base, false))
            } else {
                format!("TransitiveClosure({})", ti(inner, false))
            }
        }

        Expr::ReflexiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Rtc{}({})", capitalize(field), ti(base, false))
            } else {
                format!("ReflexiveTransitiveClosure({})", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                CompareOp::Eq => format!("{} == {}", ti(left, false), ti(right, false)),
                CompareOp::NotEq => format!("{} != {}", ti(left, false), ti(right, false)),
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} <= {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} >= {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    let l = ti(left, false);
                    if let Expr::FieldAccess { base, field } = right.as_ref() {
                        let r_base = ti(base, false);
                        if let Some((Multiplicity::Lone, _)) = field_mult(field, ir) {
                            return format!("{r_base}.{} == {l}", capitalize(field));
                        }
                    }
                    let r = ti(right, false);
                    format!("{r}.Contains({l})")
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
            let b = ti(body, false);
            build_nested_quantifier(kind, bindings, &b, sig_names, ir)
        }

        Expr::SetOp { op, left, right } => {
            // `Union`/`Intersect`/`Except` are `System.Linq` and return
            // `IEnumerable<T>`, which has no `TrueForAll`/`Exists` — those
            // are `List<T>` members. Materialise so the result is usable
            // anywhere a set-multiplicity value is (a quantifier domain, a
            // field assignment, …). See #102 round 3 defect 5.
            let l = ti(left, false);
            let r = ti(right, false);
            match op {
                SetOpKind::Union => format!("{l}.Union({r}).ToList()"),
                SetOpKind::Intersection => format!("{l}.Intersect({r}).ToList()"),
                SetOpKind::Difference => format!("{l}.Except({r}).ToList()"),
            }
        }

        Expr::Product { left, right } => {
            let l = ti(left, false);
            let r = ti(right, false);
            format!("({l}, {r})")
        }

        Expr::MultFormula { kind, expr: inner } => {
            let translated = ti(inner, false);
            match kind {
                QuantKind::Some => format!("{translated} != null"),
                QuantKind::No => format!("{translated} == null"),
                _ => format!("{kind:?}({translated})"),
            }
        }

        Expr::Prime(inner) => {
            match inner.as_ref() {
                Expr::FieldAccess { base, field } => {
                    let base_str = ti(base, false);
                    format!("{base_str}.Next{}", capitalize(field))
                }
                Expr::VarRef(name) => compose_ident("next", name),
                _ => format!("{}.Next()", ti(inner, false)),
            }
        }

        Expr::TemporalUnary { expr: inner, .. } => ti(inner, false),
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

fn build_nested_quantifier(
    kind: &QuantKind,
    bindings: &[QuantBinding],
    body_str: &str,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> String {
    let mut vars: Vec<(String, String, bool)> = Vec::new();
    for b in bindings {
        let d = if let Expr::VarRef(name) = &b.domain {
            if sig_names.contains(name) {
                to_camel_plural(name)
            } else if is_native_type_alias(name) {
                // A quantifier can't enumerate a native domain's true extent
                // any more than it enumerates a sig's — every other domain
                // here is already a one-element sample list built from a
                // fixture, so a native domain gets the same treatment: a
                // sample list seeded with that type's zero value, rather
                // than keeping the bare Alloy name (`Int.TrueForAll(...)`,
                // CS0103 — `Int` names no C# type). See #102 round 3 defect 6.
                let cs_ty = resolve_type(TargetLang::CSharp, name);
                let zero = cs_zero_value(&cs_ty);
                format!("new List<{cs_ty}>{{ {zero} }}")
            } else {
                cs_ident(name)
            }
        } else {
            translate_inner(&b.domain, false, sig_names, ir)
        };
        for v in &b.vars {
            vars.push((cs_ident(v), d.clone(), b.disj));
        }
    }

    let mut disj_checks = Vec::new();
    let mut i = 0;
    while i < vars.len() {
        if vars[i].2 {
            let domain = &vars[i].1;
            let start = i;
            while i < vars.len() && vars[i].2 && vars[i].1 == *domain { i += 1; }
            for a in start..i {
                for b_idx in (a+1)..i {
                    disj_checks.push(format!("{} != {}", vars[a].0, vars[b_idx].0));
                }
            }
        } else { i += 1; }
    }

    let guarded_body = if disj_checks.is_empty() {
        body_str.to_string()
    } else {
        let guard = disj_checks.join(" && ");
        match kind {
            QuantKind::All | QuantKind::No => format!("(!({guard}) || ({body_str}))"),
            QuantKind::Some => format!("{guard} && {body_str}"),
        }
    };

    let mut result = guarded_body;
    for idx in (0..vars.len()).rev() {
        let (ref var, ref domain, _) = vars[idx];
        result = match kind {
            QuantKind::All => format!("{domain}.TrueForAll({var} => {body})", body = result),
            QuantKind::Some => format!("{domain}.Exists({var} => {body})", body = result),
            QuantKind::No => {
                if idx == 0 {
                    format!("!{domain}.Exists({var} => {body})", body = result)
                } else {
                    format!("{domain}.Exists({var} => {body})", body = result)
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
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) | Expr::ReflexiveClosure(inner) => {
            collect_params(inner, sig_names, params);
        }
        Expr::MultFormula { expr: inner, .. } => {
            collect_params(inner, sig_names, params);
        }
        Expr::FieldAccess { base, .. } => collect_params(base, sig_names, params),
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
    cs_ident(&out)
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

// `cs_ident` (and the `CS_KEYWORDS` it escapes against) live in `super` —
// see the doc comment there for the identifier-vs-resolved-type split.
