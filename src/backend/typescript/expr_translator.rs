use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use std::collections::{HashSet, BTreeSet};

/// Translate an Alloy expression to a TypeScript expression string with IR context.
pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir)
}

/// Extract the collection parameters needed by an expression.
pub fn extract_params(expr: &Expr, sig_names: &HashSet<String>) -> Vec<(String, String)> {
    let mut params = BTreeSet::new();
    collect_params(expr, sig_names, &mut params);
    params.into_iter().collect()
}

/// Info about a transitive closure field needed for function generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TCField {
    pub field_name: String,
    pub sig_name: String,
    pub mult: Multiplicity,
}

/// Extract all TC field usages from an expression, resolved against IR structures.
pub fn extract_tc_fields(expr: &Expr, ir: &OxidtrIR) -> Vec<TCField> {
    let mut fields = Vec::new();
    collect_tc_fields(expr, ir, &mut fields);
    fields.sort_by(|a, b| a.field_name.cmp(&b.field_name));
    fields.dedup();
    fields
}

fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

fn collect_params(expr: &Expr, sig_names: &HashSet<String>, params: &mut BTreeSet<(String, String)>) {
    match expr {
        Expr::Quantifier { domain, body, .. } => {
            if let Expr::VarRef(name) = domain.as_ref() {
                if sig_names.contains(name) {
                    params.insert((to_camel_plural(name), name.clone()));
                }
            }
            collect_params(domain, sig_names, params);
            collect_params(body, sig_names, params);
        }
        Expr::BinaryLogic { left, right, .. } | Expr::Comparison { left, right, .. } => {
            collect_params(left, sig_names, params);
            collect_params(right, sig_names, params);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            collect_params(inner, sig_names, params);
        }
        Expr::FieldAccess { base, .. } => collect_params(base, sig_names, params),
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
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
        Expr::Quantifier { domain, body, .. } => {
            collect_tc_fields(domain, ir, out);
            collect_tc_fields(body, ir, out);
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
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

fn translate_inner(
    expr: &Expr,
    parens_if_complex: bool,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> String {
    let ti = |e: &Expr, p: bool| translate_inner(e, p, sig_names, ir);

    let result = match expr {
        Expr::IntLiteral(n) => n.to_string(),

        Expr::VarRef(name) => name.clone(),

        Expr::FieldAccess { base, field } => {
            format!("{}.{}", ti(base, false), to_camel_case(field))
        }

        Expr::Cardinality(inner) => {
            // Set<T> uses .size, T[] uses .length
            let is_set = if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                matches!(field_mult(field, ir), Some((Multiplicity::Set, _)))
            } else {
                false
            };
            if is_set {
                format!("{}.size", ti(inner, false))
            } else {
                format!("{}.length", ti(inner, false))
            }
        }

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("tc{}({})", capitalize(field), ti(base, false))
            } else {
                format!("transitiveClosure({})", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                CompareOp::Eq => {
                    lone_comparison(left, right, "===", ir, &|e, p| ti(e, p))
                }
                CompareOp::NotEq => {
                    lone_comparison(left, right, "!==", ir, &|e, p| ti(e, p))
                }
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} <= {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} >= {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    let l = ti(left, false);
                    if let Expr::FieldAccess { base, field } = right.as_ref() {
                        let r_base = ti(base, false);
                        if let Some((Multiplicity::Lone, _)) = field_mult(field, ir) {
                            return format!("{r_base}.{} === {l}", to_camel_case(field));
                        }
                        // Set<T> uses .has(), T[] uses .includes()
                        if let Some((Multiplicity::Set, _)) = field_mult(field, ir) {
                            return format!("{r_base}.{}.has({l})", to_camel_case(field));
                        }
                    }
                    let r = ti(right, false);
                    format!("{r}.includes({l})")
                }
            }
        }

        Expr::BinaryLogic { op, left, right } => match op {
            LogicOp::And     => format!("{} && {}", ti(left, false), ti(right, false)),
            LogicOp::Or      => format!("{} || {}", ti(left, false), ti(right, false)),
            LogicOp::Implies => format!("!{} || {}", ti(left, true), ti(right, false)),
            LogicOp::Iff     => format!("{} === {}", ti(left, true), ti(right, true)),
        },

        Expr::Not(inner) => {
            format!("!{}", ti(inner, true))
        }

        Expr::Quantifier { kind, var, domain, body } => {
            let b = ti(body, false);
            let d = if let Expr::VarRef(name) = domain.as_ref() {
                if sig_names.contains(name) {
                    to_camel_plural(name)
                } else { name.clone() }
            } else { ti(domain, false) };
            match kind {
                QuantKind::All  => format!("{d}.every(({var}) => {b})"),
                QuantKind::Some => format!("{d}.some(({var}) => {b})"),
                QuantKind::No   => format!("!{d}.some(({var}) => {b})"),
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
        Expr::Comparison { .. } | Expr::BinaryLogic { .. } | Expr::Quantifier { .. }
    )
}

/// Generate Eq/NotEq comparison handling lone (T | null) fields.
fn lone_comparison<F>(
    left: &Expr,
    right: &Expr,
    op: &str,
    _ir: &OxidtrIR,
    ti: &F,
) -> String
where
    F: Fn(&Expr, bool) -> String,
{
    // In TypeScript, lone fields are `T | null`, so direct === works
    format!("{} {op} {}", ti(left, false), ti(right, false))
}

fn to_camel_case(s: &str) -> String {
    // Alloy field names are already camelCase typically; pass through
    s.to_string()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

fn to_camel_plural(name: &str) -> String {
    // "SigDecl" -> "sigDecls"
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
