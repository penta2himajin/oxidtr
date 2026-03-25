use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use std::collections::{HashSet, BTreeSet};

/// Info about a transitive closure field needed for function generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TCField {
    pub field_name: String,
    pub sig_name: String,
    pub mult: Multiplicity,
}

/// Translate an Alloy expression to a Rust expression string (no context).
pub fn translate(expr: &Expr) -> String {
    translate_ctx(expr, &HashSet::new())
}

/// Translate with knowledge of sig names. Bare sig names in quantifier domains
/// are replaced with snake_cased plural parameter names.
/// TC expressions generate `tc_{field}(&base)` calls.
pub fn translate_ctx(expr: &Expr, sig_names: &HashSet<String>) -> String {
    translate_inner(expr, false, sig_names)
}

/// Extract the collection parameters needed by an expression.
pub fn extract_params(expr: &Expr, sig_names: &HashSet<String>) -> Vec<(String, String)> {
    let mut params = BTreeSet::new();
    collect_params(expr, sig_names, &mut params);
    params.into_iter().collect()
}

/// Extract all TC field usages from an expression, resolved against IR structures.
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
            // Pattern: TransitiveClosure(FieldAccess { base, field })
            if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                // Find which structure contains this field
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
        Expr::MultFormula { expr: inner, .. } => collect_tc_fields(inner, ir, out),
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings { collect_tc_fields(&b.domain, ir, out); }
            collect_tc_fields(body, ir, out);
        }
        Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            collect_tc_fields(left, ir, out);
            collect_tc_fields(right, ir, out);
        }
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

fn collect_params(expr: &Expr, sig_names: &HashSet<String>, params: &mut BTreeSet<(String, String)>) {
    match expr {
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings {
                if let Expr::VarRef(name) = &b.domain {
                    if sig_names.contains(name) {
                        params.insert((to_snake_plural(name), name.clone()));
                    }
                }
                collect_params(&b.domain, sig_names, params);
            }
            collect_params(body, sig_names, params);
        }
        Expr::BinaryLogic { left, right, .. } => {
            collect_params(left, sig_names, params);
            collect_params(right, sig_names, params);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            collect_params(inner, sig_names, params);
        }
        Expr::MultFormula { expr: inner, .. } => {
            collect_params(inner, sig_names, params);
        }
        Expr::Comparison { left, right, .. } | Expr::SetOp { left, right, .. }
        | Expr::Product { left, right } => {
            collect_params(left, sig_names, params);
            collect_params(right, sig_names, params);
        }
        Expr::FieldAccess { base, .. } => {
            collect_params(base, sig_names, params);
        }
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

fn translate_inner(expr: &Expr, parens_if_complex: bool, sig_names: &HashSet<String>) -> String {
    let result = match expr {
        Expr::IntLiteral(n) => n.to_string(),

        Expr::VarRef(name) => name.clone(),

        Expr::FieldAccess { base, field } => {
            format!("{}.{field}", translate_inner(base, false, sig_names))
        }

        Expr::Cardinality(inner) => {
            format!("{}.len()", translate_inner(inner, false, sig_names))
        }

        Expr::TransitiveClosure(inner) => {
            // Pattern: TC(FieldAccess { base, field }) → tc_{field}(&base)
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                let b = translate_inner(base, false, sig_names);
                format!("tc_{field}(&{b})")
            } else {
                // Fallback for non-field-access TC (shouldn't happen in practice)
                format!("transitive_closure({})", translate_inner(inner, false, sig_names))
            }
        }

        Expr::Comparison { op, left, right } => {
            let l = translate_inner(left, false, sig_names);
            let r = translate_inner(right, false, sig_names);
            match op {
                CompareOp::Eq => format!("{l} == {r}"),
                CompareOp::NotEq => format!("{l} != {r}"),
                CompareOp::In => format!("{r}.contains(&{l})"),
                CompareOp::Lt => format!("{l} < {r}"),
                CompareOp::Gt => format!("{l} > {r}"),
                CompareOp::Lte => format!("{l} <= {r}"),
                CompareOp::Gte => format!("{l} >= {r}"),
            }
        }

        Expr::BinaryLogic { op, left, right } => {
            match op {
                LogicOp::And => {
                    let l = translate_inner(left, false, sig_names);
                    let r = translate_inner(right, false, sig_names);
                    format!("{l} && {r}")
                }
                LogicOp::Or => {
                    let l = translate_inner(left, false, sig_names);
                    let r = translate_inner(right, false, sig_names);
                    format!("{l} || {r}")
                }
                LogicOp::Implies => {
                    let l = translate_inner(left, true, sig_names);
                    let r = translate_inner(right, false, sig_names);
                    format!("!{l} || {r}")
                }
                LogicOp::Iff => {
                    let l = translate_inner(left, true, sig_names);
                    let r = translate_inner(right, true, sig_names);
                    format!("{l} == {r}")
                }
            }
        }

        Expr::Not(inner) => {
            let s = translate_inner(inner, true, sig_names);
            format!("!{s}")
        }

        Expr::Quantifier { kind, bindings, body } => {
            let vars = collect_quant_vars(bindings, sig_names);
            let b = translate_inner(body, false, sig_names);
            build_nested_quantifier(kind, &vars, &b, !sig_names.is_empty())
        }

        Expr::SetOp { op, left, right } => {
            let l = translate_inner(left, false, sig_names);
            let r = translate_inner(right, false, sig_names);
            match op {
                SetOpKind::Union => format!("{l}.union(&{r}).cloned().collect()"),
                SetOpKind::Intersection => format!("{l}.intersection(&{r}).cloned().collect()"),
                SetOpKind::Difference => format!("{l}.difference(&{r}).cloned().collect()"),
            }
        }

        Expr::Product { left, right } => {
            let l = translate_inner(left, false, sig_names);
            let r = translate_inner(right, false, sig_names);
            format!("({l}, {r})")
        }

        Expr::MultFormula { kind, expr } => {
            let inner = translate_inner(expr, false, sig_names);
            match kind {
                QuantKind::Some => format!("{inner}.is_some()"),
                QuantKind::No => format!("{inner}.is_none()"),
                _ => inner,
            }
        }

        // Alloy 6: prime operator — next-state reference
        Expr::Prime(inner) => {
            match inner.as_ref() {
                Expr::FieldAccess { base, field } => {
                    let base_str = translate_inner(base, false, sig_names);
                    format!("{base_str}.next_{field}")
                }
                Expr::VarRef(name) => format!("next_{name}"),
                _ => format!("{}.next()", translate_inner(inner, false, sig_names)),
            }
        }
        // Alloy 6: temporal unary operators — translate inner expression
        Expr::TemporalUnary { expr: inner, .. } => {
            translate_inner(inner, false, sig_names)
        }
        // Alloy 6: temporal binary operators — translate both sides
        Expr::TemporalBinary { left, right, .. } => {
            let l = translate_inner(left, false, sig_names);
            let r = translate_inner(right, false, sig_names);
            format!("{l} && {r}")
        }
        Expr::FunApp { name, receiver, args } => {
            translate_fun_app(name, receiver.as_deref(), args, |e| translate_inner(e, false, sig_names))
        }
    };

    if parens_if_complex && needs_parens(expr) {
        format!("({result})")
    } else {
        result
    }
}

/// Collect all (var, domain_str, is_disj) tuples from bindings, expanding multi-var bindings.
fn collect_quant_vars(bindings: &[QuantBinding], sig_names: &HashSet<String>) -> Vec<(String, String, bool)> {
    let mut vars = Vec::new();
    for b in bindings {
        let domain_str = match &b.domain {
            Expr::VarRef(name) if sig_names.contains(name) => to_snake_plural(name),
            _ => translate_inner(&b.domain, false, sig_names),
        };
        for v in &b.vars {
            vars.push((v.clone(), domain_str.clone(), b.disj));
        }
    }
    vars
}

/// Like collect_quant_vars but uses IR-aware translation for domain expressions.
fn collect_quant_vars_ir(
    bindings: &[QuantBinding],
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> Vec<(String, String, bool)> {
    let mut vars = Vec::new();
    for b in bindings {
        let domain_str = match &b.domain {
            Expr::VarRef(name) if sig_names.contains(name) => to_snake_plural(name),
            Expr::VarRef(name) => name.clone(),
            _ => translate_inner_ir(&b.domain, false, sig_names, ir),
        };
        for v in &b.vars {
            vars.push((v.clone(), domain_str.clone(), b.disj));
        }
    }
    vars
}

/// Build nested quantifier code from inside-out.
fn build_nested_quantifier(
    kind: &QuantKind,
    vars: &[(String, String, bool)],
    body_str: &str,
    use_clone: bool,
) -> String {
    if vars.is_empty() {
        return body_str.to_string();
    }

    // Build disj checks: for each disj binding group (same domain, disj=true),
    // all pairs of vars must be !=.
    // We collect groups of consecutive disj vars with the same domain.
    let mut disj_checks = Vec::new();
    let mut i = 0;
    while i < vars.len() {
        if vars[i].2 {
            let domain = &vars[i].1;
            let start = i;
            while i < vars.len() && vars[i].2 && vars[i].1 == *domain {
                i += 1;
            }
            for a in start..i {
                for b in (a+1)..i {
                    disj_checks.push(format!("{} != {}", vars[a].0, vars[b].0));
                }
            }
        } else {
            i += 1;
        }
    }

    // Wrap body with disj guard if needed
    let guarded_body = if disj_checks.is_empty() {
        body_str.to_string()
    } else {
        let guard = disj_checks.join(" && ");
        match kind {
            QuantKind::All | QuantKind::No => {
                format!("if {guard} {{ {body_str} }} else {{ true }}")
            }
            QuantKind::Some => {
                format!("{guard} && {body_str}")
            }
        }
    };

    // Build from inside-out: last var is innermost
    let mut result = guarded_body;
    for idx in (0..vars.len()).rev() {
        let (ref var, ref domain, _) = vars[idx];
        let inner = if use_clone {
            format!("{{ let {var} = {var}.clone(); {result} }}")
        } else {
            result
        };
        result = match kind {
            QuantKind::All => format!("{domain}.iter().all(|{var}| {inner})"),
            QuantKind::Some => {
                if idx == 0 {
                    format!("{domain}.iter().any(|{var}| {inner})")
                } else {
                    // Inner iterations for "some" are also "any"
                    format!("{domain}.iter().any(|{var}| {inner})")
                }
            }
            QuantKind::No => {
                if idx == 0 {
                    format!("!{domain}.iter().any(|{var}| {inner})")
                } else {
                    // Inner iterations for "no" use "any" — the negation is only on the outermost
                    format!("{domain}.iter().any(|{var}| {inner})")
                }
            }
        };
    }
    result
}

/// Translate Alloy function application. Known integer functions (plus, minus, mul, div, rem)
/// with a receiver are translated to arithmetic operators.
fn translate_fun_app(name: &str, receiver: Option<&Expr>, args: &[Expr], translate: impl Fn(&Expr) -> String) -> String {
    // Alloy integer arithmetic: receiver.plus[n] → receiver + n
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
            let r = translate(recv);
            let a = translate(arg);
            return format!("{r} {op} {a}");
        }
        // Non-arithmetic method call with receiver
        let a: Vec<_> = args.iter().map(&translate).collect();
        let r = translate(recv);
        return format!("{r}.{name}({})", a.join(", "));
    }
    // Bare function call: f[x, y] → f(x, y)
    let a: Vec<_> = args.iter().map(translate).collect();
    format!("{name}({})", a.join(", "))
}

fn needs_parens(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Comparison { .. }
            | Expr::BinaryLogic { .. }
            | Expr::Quantifier { .. }
    )
}

fn to_snake_plural(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out.push('s');
    out
}

/// Translate with IR context to handle lone-field membership correctly.
/// Unlike `translate_ctx`, this can distinguish `Option<T>` vs `Vec<T>` fields
/// in `In` comparisons, generating the correct Rust accessor.
pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR) -> String {
    translate_inner_ir(expr, false, &collect_sig_names_set(ir), ir)
}

fn collect_sig_names_set(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

/// Look up the multiplicity and boxing info of a named field.
/// Returns (multiplicity, needs_box) where needs_box is true for self-ref or cyclic-ref fields.
fn field_mult(field_name: &str, ir: &OxidtrIR) -> Option<(Multiplicity, bool)> {
    let cyclic = super::find_cyclic_fields(ir);
    for s in &ir.structures {
        for f in &s.fields {
            if f.name == field_name {
                let is_boxed = cyclic.contains(&(s.name.clone(), f.name.clone()));
                return Some((f.mult.clone(), is_boxed));
            }
        }
    }
    None
}

fn translate_inner_ir(
    expr: &Expr,
    parens_if_complex: bool,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> String {
    let ti = |e: &Expr, p: bool| translate_inner_ir(e, p, sig_names, ir);

    let result = match expr {
        Expr::IntLiteral(n) => n.to_string(),

        Expr::VarRef(name) => name.clone(),

        Expr::FieldAccess { base, field } => {
            let base_str = ti(base, false);
            // Box<T> one-fields need deref for comparisons to work
            if let Some((Multiplicity::One, true)) = field_mult(field, ir) {
                format!("(*{base_str}.{field})")
            } else {
                format!("{base_str}.{field}")
            }
        }

        Expr::Cardinality(inner) => format!("{}.len()", ti(inner, false)),

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("tc_{field}(&{})", ti(base, false))
            } else {
                format!("transitive_closure({})", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                CompareOp::Eq => {
                    lone_comparison(left, right, "==", ir, &|e, p| ti(e, p))
                }
                CompareOp::NotEq => {
                    lone_comparison(left, right, "!=", ir, &|e, p| ti(e, p))
                }
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} <= {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} >= {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    let l = ti(left, false);
                    // Check if right side is a field access to a lone field
                    if let Expr::FieldAccess { base, field } = right.as_ref() {
                        let r_base = ti(base, false);
                        if let Some((Multiplicity::Lone, is_self_ref)) = field_mult(field, ir) {
                            return if is_self_ref {
                                // Option<Box<T>> — use as_deref()
                                format!("{r_base}.{field}.as_deref() == Some(&{l})")
                            } else {
                                // Option<T> — use as_ref()
                                format!("{r_base}.{field}.as_ref() == Some(&{l})")
                            };
                        }
                    }
                    // Default: Set / One field → .contains()
                    let r = ti(right, false);
                    format!("{r}.contains(&{l})")
                }
            }
        }

        Expr::BinaryLogic { op, left, right } => match op {
            LogicOp::And     => format!("{} && {}", ti(left, false), ti(right, false)),
            LogicOp::Or      => format!("{} || {}", ti(left, false), ti(right, false)),
            LogicOp::Implies => format!("!{} || {}", ti(left, true), ti(right, false)),
            LogicOp::Iff     => format!("{} == {}", ti(left, true), ti(right, true)),
        },

        Expr::Not(inner) => {
            let i = ti(inner, true);
            format!("!{i}")
        }

        Expr::Quantifier { kind, bindings, body } => {
            let vars = collect_quant_vars_ir(bindings, sig_names, ir);
            let b = ti(body, false);
            build_nested_quantifier(kind, &vars, &b, true)
        }

        Expr::SetOp { op, left, right } => {
            let l = ti(left, false);
            let r = ti(right, false);
            match op {
                SetOpKind::Union => format!("{l}.union(&{r}).cloned().collect()"),
                SetOpKind::Intersection => format!("{l}.intersection(&{r}).cloned().collect()"),
                SetOpKind::Difference => format!("{l}.difference(&{r}).cloned().collect()"),
            }
        }

        Expr::Product { left, right } => {
            let l = ti(left, false);
            let r = ti(right, false);
            format!("({l}, {r})")
        }

        Expr::MultFormula { kind, expr } => {
            let inner = ti(expr, false);
            match kind {
                QuantKind::Some => format!("{inner}.is_some()"),
                QuantKind::No => format!("{inner}.is_none()"),
                _ => inner,
            }
        }

        // Alloy 6: prime operator — next-state reference
        Expr::Prime(inner) => {
            match inner.as_ref() {
                Expr::FieldAccess { base, field } => {
                    let base_str = ti(base, false);
                    format!("{base_str}.next_{field}")
                }
                Expr::VarRef(name) => format!("next_{name}"),
                _ => format!("{}.next()", ti(inner, false)),
            }
        }
        // Alloy 6: temporal unary operators — translate inner expression
        Expr::TemporalUnary { expr: inner, .. } => {
            ti(inner, false)
        }
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

/// Generate Eq/NotEq comparison handling lone (Option<T>) fields correctly.
///
/// Cases:
///   lone == one   → field.as_(de)ref() == Some(&other)
///   one  == lone  → other.as_(de)ref() == Some(&field)  [symmetric]
///   lone == lone  → direct == (both are Option<T>, compare directly)
///   one  == one   → direct ==
fn lone_comparison<F>(
    left: &Expr,
    right: &Expr,
    op: &str,
    ir: &OxidtrIR,
    ti: &F,
) -> String
where
    F: Fn(&Expr, bool) -> String,
{
    let left_lone  = lone_field_info(left, ir);
    let right_lone = lone_field_info(right, ir);

    match (left_lone, right_lone) {
        // Both sides are lone → compare Option<T> directly
        (Some(_), Some(_)) => {
            format!("{} {op} {}", ti(left, false), ti(right, false))
        }
        // Left is lone, right is not → left.as_(de)ref() == Some(&right)
        (Some((field, base, is_self_ref)), None) => {
            let b = ti(base, false);
            let r = ti(right, false);
            if is_self_ref {
                format!("{b}.{field}.as_deref() {op} Some(&{r})")
            } else {
                format!("{b}.{field}.as_ref() {op} Some(&{r})")
            }
        }
        // Right is lone, left is not → right.as_(de)ref() == Some(&left)
        (None, Some((field, base, is_self_ref))) => {
            let b = ti(base, false);
            let l = ti(left, false);
            if is_self_ref {
                format!("{b}.{field}.as_deref() {op} Some(&{l})")
            } else {
                format!("{b}.{field}.as_ref() {op} Some(&{l})")
            }
        }
        // Neither side is lone → plain comparison
        (None, None) => {
            format!("{} {op} {}", ti(left, false), ti(right, false))
        }
    }
}

/// If expr is a FieldAccess to a lone field, return (field_name, base_expr, is_self_ref).
fn lone_field_info<'a>(
    expr: &'a Expr,
    ir: &OxidtrIR,
) -> Option<(&'a str, &'a Expr, bool)> {
    if let Expr::FieldAccess { base, field } = expr {
        if let Some((Multiplicity::Lone, is_self_ref)) = field_mult(field, ir) {
            return Some((field.as_str(), base.as_ref(), is_self_ref));
        }
    }
    None
}
