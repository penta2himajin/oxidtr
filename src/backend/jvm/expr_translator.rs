use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use std::collections::{HashSet, BTreeSet};

/// Language-specific expression syntax.
pub trait JvmLang {
    fn all_quantifier(&self, collection: &str, var: &str, body: &str) -> String;
    fn some_quantifier(&self, collection: &str, var: &str, body: &str) -> String;
    fn no_quantifier(&self, collection: &str, var: &str, body: &str) -> String;
    fn contains(&self, collection: &str, element: &str) -> String;
    fn cardinality(&self, expr: &str) -> String;
    fn lone_eq(&self, base: &str, field: &str, value: &str) -> String;
    fn tc_call(&self, field: &str, base: &str) -> String;
    fn eq_op(&self) -> &str;
    fn neq_op(&self) -> &str;
}

pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR, lang: &dyn JvmLang) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir, lang)
}

pub fn extract_params(expr: &Expr, sig_names: &HashSet<String>) -> Vec<(String, String)> {
    let mut params = BTreeSet::new();
    collect_params(expr, sig_names, &mut params);
    params.into_iter().collect()
}

pub fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

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
        Expr::Quantifier { domain, body, .. } => {
            collect_tc_fields(domain, ir, out);
            collect_tc_fields(body, ir, out);
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
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
    lang: &dyn JvmLang,
) -> String {
    let ti = |e: &Expr, p: bool| translate_inner(e, p, sig_names, ir, lang);

    let result = match expr {
        Expr::IntLiteral(n) => n.to_string(),

        Expr::VarRef(name) => name.clone(),

        Expr::FieldAccess { base, field } => {
            format!("{}.{field}", ti(base, false))
        }

        Expr::Cardinality(inner) => lang.cardinality(&ti(inner, false)),

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                lang.tc_call(field, &ti(base, false))
            } else {
                format!("transitiveClosure({})", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                CompareOp::Eq => format!("{} {} {}", ti(left, false), lang.eq_op(), ti(right, false)),
                CompareOp::NotEq => format!("{} {} {}", ti(left, false), lang.neq_op(), ti(right, false)),
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} <= {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} >= {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    let l = ti(left, false);
                    if let Expr::FieldAccess { base, field } = right.as_ref() {
                        let r_base = ti(base, false);
                        if let Some((Multiplicity::Lone, _)) = field_mult(field, ir) {
                            return lang.lone_eq(&r_base, field, &l);
                        }
                    }
                    let r = ti(right, false);
                    lang.contains(&r, &l)
                }
            }
        }

        Expr::BinaryLogic { op, left, right } => match op {
            LogicOp::And     => format!("{} && {}", ti(left, false), ti(right, false)),
            LogicOp::Or      => format!("{} || {}", ti(left, false), ti(right, false)),
            LogicOp::Implies => format!("!{} || {}", ti(left, true), ti(right, false)),
            LogicOp::Iff     => format!("{} {} {}", ti(left, true), lang.eq_op(), ti(right, true)),
        },

        Expr::Not(inner) => format!("!{}", ti(inner, true)),

        Expr::Quantifier { kind, var, domain, body } => {
            let b = ti(body, false);
            let d = if let Expr::VarRef(name) = domain.as_ref() {
                if sig_names.contains(name) { to_camel_plural(name) }
                else { name.clone() }
            } else { ti(domain, false) };
            match kind {
                QuantKind::All  => lang.all_quantifier(&d, var, &b),
                QuantKind::Some => lang.some_quantifier(&d, var, &b),
                QuantKind::No   => lang.no_quantifier(&d, var, &b),
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
