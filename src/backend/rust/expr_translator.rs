use crate::parser::ast::*;
use crate::ir::nodes::{StructureNode, OxidtrIR};
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
        Expr::Quantifier { domain, body, .. } => {
            collect_tc_fields(domain, ir, out);
            collect_tc_fields(body, ir, out);
        }
        Expr::VarRef(_) => {}
    }
}

fn collect_params(expr: &Expr, sig_names: &HashSet<String>, params: &mut BTreeSet<(String, String)>) {
    match expr {
        Expr::Quantifier { domain, body, .. } => {
            if let Expr::VarRef(name) = domain.as_ref() {
                if sig_names.contains(name) {
                    params.insert((to_snake_plural(name), name.clone()));
                }
            }
            collect_params(domain, sig_names, params);
            collect_params(body, sig_names, params);
        }
        Expr::BinaryLogic { left, right, .. } => {
            collect_params(left, sig_names, params);
            collect_params(right, sig_names, params);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            collect_params(inner, sig_names, params);
        }
        Expr::Comparison { left, right, .. } => {
            collect_params(left, sig_names, params);
            collect_params(right, sig_names, params);
        }
        Expr::FieldAccess { base, .. } => {
            collect_params(base, sig_names, params);
        }
        Expr::VarRef(_) => {}
    }
}

fn translate_inner(expr: &Expr, parens_if_complex: bool, sig_names: &HashSet<String>) -> String {
    let result = match expr {
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

        Expr::Quantifier { kind, var, domain, body } => {
            let d = match domain.as_ref() {
                Expr::VarRef(name) if sig_names.contains(name) => to_snake_plural(name),
                _ => translate_inner(domain, false, sig_names),
            };
            let b = translate_inner(body, false, sig_names);
            // When translating in a typed context (sig_names non-empty),
            // clone the iterator variable to convert &T → T, avoiding
            // type mismatches in comparisons (field: T vs iter var: &T).
            let body_expr = if !sig_names.is_empty() {
                format!("{{ let {var} = {var}.clone(); {b} }}")
            } else {
                b
            };
            match kind {
                QuantKind::All => format!("{d}.iter().all(|{var}| {body_expr})"),
                QuantKind::Some => format!("{d}.iter().any(|{var}| {body_expr})"),
                QuantKind::No => format!("!{d}.iter().any(|{var}| {body_expr})"),
            }
        }
    };

    if parens_if_complex && needs_parens(expr) {
        format!("({result})")
    } else {
        result
    }
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

/// Look up the multiplicity of a named field across all IR structures.
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

fn translate_inner_ir(
    expr: &Expr,
    parens_if_complex: bool,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> String {
    let ti = |e: &Expr, p: bool| translate_inner_ir(e, p, sig_names, ir);

    let result = match expr {
        Expr::VarRef(name) => name.clone(),

        Expr::FieldAccess { base, field } => {
            format!("{}.{field}", ti(base, false))
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

        Expr::Quantifier { kind, var, domain, body } => {
            let b = ti(body, false);
            let wrapped = format!("{{ let {var} = {var}.clone(); {b} }}");
            let d = if let Expr::VarRef(name) = domain.as_ref() {
                if sig_names.contains(name) {
                    to_snake_plural(name)
                } else { name.clone() }
            } else { ti(domain, false) };
            match kind {
                QuantKind::All  => format!("{d}.iter().all(|{var}| {wrapped})"),
                QuantKind::Some => format!("{d}.iter().any(|{var}| {wrapped})"),
                QuantKind::No   => format!("!{d}.iter().any(|{var}| {wrapped})"),
            }
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
