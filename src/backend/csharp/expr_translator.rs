use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use crate::backend::{is_native_type_alias, resolve_type, TargetLang};
use crate::backend::type_env::{TypeEnv, resolve_field, resolve_field_owner};
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
    translate_with_env(expr, ir, &TypeEnv::new())
}

/// Translate in an explicit scope — an operation's parameters, for instance.
pub fn translate_with_env(expr: &Expr, ir: &OxidtrIR, env: &TypeEnv) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir, env)
}

/// Translate a temporal constraint for a trace-checker body: strip the temporal
/// wrapper and translate what it quantifies over, since each trace element is
/// already the collection the quantifier ranges across.
pub fn translate_trace_body(expr: &Expr, ir: &OxidtrIR) -> String {
    let inner = match expr {
        Expr::TemporalUnary { expr, .. } => expr.as_ref(),
        _ => expr,
    };
    translate_with_ir(inner, ir)
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
pub fn finalize_post_state_idents(expr: &Expr, bound: &HashSet<String>) -> Expr {
    let r = |e: &Expr| finalize_post_state_idents(e, bound);
    match expr {
        // `rewrite_prime_as_post_state` names a post-state `next_x`, and this
        // pass gives it C# casing. A *user* binder called `next_c` reaches the
        // same tree and is indistinguishable afterwards, so it was renamed to
        // `nextC` — an identifier nothing declares (#110). The binders in
        // scope are the one thing that tells the two apart.
        Expr::VarRef(name) if bound.contains(name) => expr.clone(),
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

        // Alloy's implicit receiver: a derived field is an extension method,
        // whose first parameter is `self`.
        Expr::VarRef(name) if name == "this" => "self".to_string(),

        Expr::VarRef(name) => cs_ident(name),

        // `Schedule.Morning` reads a field across every atom of the sig (#142).
        Expr::FieldAccess { .. } if relational_image(expr, sig_names, ir, env).is_some() => {
            relational_image(expr, sig_names, ir, env).unwrap()
        }

        Expr::FieldAccess { base, field } => {
            // The property name depends on the declaring sig: a field whose
            // capitalised form is that sig's own name keeps Alloy's spelling
            // (#137).
            let owner = resolve_field_owner(base, field, sig_names, ir, env)
                .map(|(o, _)| o)
                .unwrap_or_default();
            format!("{}.{}", ti(base, false), cs_property_name(&owner, field))
        }

        // A sig's extent is the list the caller materialised for it (#105); so
        // is a relational image, which `ti` already renders as one (#142).
        Expr::Cardinality(inner) => format!("{}.Count",
            whole_sig_extent_in(inner, sig_names, ir, env).unwrap_or_else(|| ti(inner, false))),

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
                CompareOp::Eq | CompareOp::NotEq => {
                    let negated = matches!(op, CompareOp::NotEq);
                    if let Some(s) = variant_case_test(left, right, ir, negated, &|e| ti(e, false)) {
                        return s;
                    }
                    // `n.C = Config` compares an atom against the sig's whole
                    // extent; for a `one sig` that is membership in it.
                    if let Some(s) = whole_sig_membership(left, right, sig_names, ir, env, negated, &|e| ti(e, false)) {
                        return s;
                    }
                    let op_str = if negated { "!=" } else { "==" };
                    format!("{} {op_str} {}", ti(left, false), ti(right, false))
                }
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} <= {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} >= {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    let l = ti(left, false);
                    if let Expr::FieldAccess { base, field } = right.as_ref() {
                        let r_base = ti(base, false);
                        if let Some(Multiplicity::Lone) =
                            resolve_field(base, field, sig_names, ir, env).map(|f| f.mult.clone())
                        {
                            return format!("{r_base}.{} == {l}", capitalize(field));
                        }
                    }
                    let r = whole_sig_extent_in(right, sig_names, ir, env)
                        .unwrap_or_else(|| ti(right, false));
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
            let inner = env.extended(bindings, sig_names, ir);
            let b = translate_inner(body, false, sig_names, ir, &inner);
            build_nested_quantifier(kind, bindings, &b, sig_names, ir, env)
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
            // `some Person` is about the sig's extent, which is a list — not a
            // nullable reference (#105).
            if let Some(e) = whole_sig_extent_in(inner, sig_names, ir, env) {
                return match kind {
                    QuantKind::Some => format!("{e}.Count > 0"),
                    QuantKind::No => format!("{e}.Count == 0"),
                    _ => e,
                };
            }
            let translated = ti(inner, false);
            // A `set`/`seq` field lowers to a `List<T>` that fixtures always
            // initialise, so it is never null: emptiness is `.Count`. Only a
            // `lone` field is nullable. Which one this is can only be answered
            // through the binding — the field name alone is ambiguous (#108).
            let is_collection = is_collection_expr(inner, sig_names, ir, env);
            match (kind, is_collection) {
                (QuantKind::Some, true) => format!("{translated}.Count > 0"),
                (QuantKind::No, true) => format!("{translated}.Count == 0"),
                (QuantKind::Some, false) => format!("{translated} != null"),
                (QuantKind::No, false) => format!("{translated} == null"),
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

/// The multiplicity of the relation a quantifier ranges over, when the domain
/// is a field access. Any other domain is already a collection or a sig.
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
    let mut scope = env.clone();
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
            let raw = translate_inner(&b.domain, false, sig_names, ir, &scope);
            // A `one`/`lone` domain is not a list; lift it so the quantifier
            // renders the same way regardless of the relation's multiplicity.
            match domain_multiplicity(&b.domain, sig_names, ir, &scope) {
                Some(Multiplicity::One) => format!("Rel.OneOf({raw})"),
                Some(Multiplicity::Lone) => format!("Rel.LoneOf({raw})"),
                _ => raw,
            }
        };
        // Sequential bindings: an earlier binder must be in scope before a
        // later domain that refers to it is rendered.
        scope = scope.extended(std::slice::from_ref(b), sig_names, ir);
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
    // A `MultFormula` renders as a comparison too — `x != null`, `x.Count > 0`
    // — so the `!` an implication puts in front of its antecedent bound to the
    // first token only: `!x.A.Union(x.B).ToList() != null` is CS0023 (#111).
    matches!(expr, Expr::Comparison { .. } | Expr::BinaryLogic { .. }
        | Expr::Quantifier { .. } | Expr::MultFormula { .. })
}

pub fn extract_params(expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR) -> Vec<(String, String)> {
    let mut params = BTreeSet::new();
    collect_params(expr, sig_names, ir, &mut params);
    params.into_iter().collect()
}

/// The name of the sample domain a bare sig reference stands for.
///
/// In Alloy a sig name in an expression is the set of its atoms — `#P`,
/// `n.c = Config`, `x in Person`. C# has no such value: the name is a type, so
/// `P.Count` and `Person.Contains(x)` do not compile. What the reference
/// denotes is the list the caller materialised for it (#105).
///
/// A variant of an abstract parent is excluded: it is a *case*, and a
/// comparison against it is a type test.
fn whole_sig_extent(expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR) -> Option<String> {
    let Expr::VarRef(name) = expr else { return None };
    if !sig_names.contains(name) { return None; }
    if crate::backend::is_native_type_alias(name) { return None; }
    if crate::backend::variant_parent(ir, name).is_some() { return None; }
    Some(to_camel_plural(name))
}

/// Whether an expression is a collection rather than a nullable reference.
///
/// A `set`/`seq` field lowers to a `List<T>` that fixtures always initialise,
/// so it is never null and emptiness is `.Count`. So is a sig's extent (#105)
/// and a relational image (#142) — and so is a set operation, whose
/// `Union`/`Intersect`/`Except` all end in `.ToList()`. That last case was
/// missing, so `some (x.a + x.b)` compared a `List` against null (#111).
fn is_collection_expr(
    expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv,
) -> bool {
    if relational_image(expr, sig_names, ir, env).is_some()
        || whole_sig_extent_in(expr, sig_names, ir, env).is_some()
    {
        return true;
    }
    match expr {
        Expr::FieldAccess { base, field } => matches!(
            resolve_field(base, field, sig_names, ir, env).map(|f| f.mult.clone()),
            Some(Multiplicity::Set) | Some(Multiplicity::Seq)
        ),
        // `Union`/`Intersect`/`Except` are materialised with `.ToList()`.
        Expr::SetOp { left, right, .. } => {
            is_collection_expr(left, sig_names, ir, env)
                || is_collection_expr(right, sig_names, ir, env)
        }
        _ => false,
    }
}

/// `Sig.field`, as the union of `field` over every atom of `Sig`.
///
/// Alloy's `Schedule.Morning` is the *relational image*, not member access on
/// a type — `Morning` is an instance property, so the receiver form is CS0120.
/// #105 rendered a sig name as its materialised extent wherever the name
/// stands alone; this is the position it left out (#142).
fn relational_image(
    expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv,
) -> Option<String> {
    let Expr::FieldAccess { base, field } = expr else { return None };
    let extent = whole_sig_extent_in(base, sig_names, ir, env)?;
    let f = resolve_field(base, field, sig_names, ir, env)?;
    let name = capitalize(field);
    Some(match f.mult {
        Multiplicity::Set | Multiplicity::Seq =>
            format!("{extent}.SelectMany(s => s.{name}).ToList()"),
        Multiplicity::Lone =>
            format!("{extent}.Select(s => s.{name}).Where(v => v != null).ToList()"),
        Multiplicity::One => format!("{extent}.Select(s => s.{name}).ToList()"),
    })
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

/// `x = Variant` / `x != Variant`, as a pattern match on the case.
///
/// A `one sig` extending an abstract sig is one atom, so equality with it asks
/// which case the other side is. C# emits the parent as an abstract class and
/// each case as a subclass, so the bare name `Low` is a type (#105).
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
    let test = format!("{} is {}", ti(subject), cs_ident(&variant));
    Some(if negated { format!("!({test})") } else { test })
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
    let test = format!("{extent}.Contains({})", ti(subject));
    Some(if negated { format!("!{test}") } else { test })
}

fn collect_params(
    expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR,
    params: &mut BTreeSet<(String, String)>,
) {
    // Positions where `translate_inner` renders a bare sig name as its extent.
    // The two must agree: a domain rendered but never materialised does not
    // compile, and one materialised but never rendered is unused.
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
    cs_ident(&out)
}

/// The C# property name a field gets on a given sig.
///
/// Normally the field name title-cased. But a member may not share its
/// enclosing type's name — CS0542, which is how a constructor is declared — so
/// `sig Level { level: … }` produced `public L Level` inside `class Level` and
/// did not compile. The collision is one *we* create by capitalising, so the
/// answer is to not: C# is case-sensitive, `level` and `Level` are distinct,
/// and the extractor lowercases the leading character either way, so the Alloy
/// name round-trips unchanged (#137).
pub fn cs_property_name(owner: &str, field: &str) -> String {
    let upper = capitalize(field);
    if upper == owner { field.to_string() } else { upper }
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
