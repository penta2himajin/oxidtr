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
        Expr::MultFormula { expr: inner, .. } => {
            collect_params(inner, sig_names, params);
        }
        Expr::FieldAccess { base, .. } => collect_params(base, sig_names, params),
        Expr::Prime(inner) => collect_params(inner, sig_names, params),
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
                format!("helpers.tc{}({})", capitalize(field), ti(base, false))
            } else {
                format!("helpers.transitiveClosure({})", ti(inner, false))
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

        Expr::Quantifier { kind, bindings, body } => {
            let b = ti(body, false);
            build_nested_quantifier_ts(kind, bindings, &b, sig_names, ir)
        }

        Expr::SetOp { op, left, right } => {
            let l = ti(left, false);
            let r = ti(right, false);
            match op {
                SetOpKind::Union => format!("new Set([...{l}, ...{r}])"),
                SetOpKind::Intersection => format!("new Set([...{l}].filter(x => {r}.has(x)))"),
                SetOpKind::Difference => format!("new Set([...{l}].filter(x => !{r}.has(x)))"),
            }
        }

        Expr::Product { left, right } => {
            let l = ti(left, false);
            let r = ti(right, false);
            format!("[{l}, {r}]")
        }

        Expr::MultFormula { kind, expr } => {
            let inner = ti(expr, false);
            match kind {
                QuantKind::Some => format!("{inner} != null"),
                QuantKind::No => format!("{inner} == null"),
                _ => inner,
            }
        }

        // TODO: Alloy 6 temporal — translate inner as-is for now
        Expr::Prime(inner) => {
            format!("/* next-state */ {}", ti(inner, false))
        }
    };

    if parens_if_complex && needs_parens(expr) {
        format!("({result})")
    } else {
        result
    }
}

fn build_nested_quantifier_ts(
    kind: &QuantKind,
    bindings: &[QuantBinding],
    body_str: &str,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> String {
    // Collect all (var, domain_str, is_disj) expanding multi-var bindings
    let mut vars: Vec<(String, String, bool)> = Vec::new();
    for b in bindings {
        let d = if let Expr::VarRef(name) = &b.domain {
            if sig_names.contains(name) { to_camel_plural(name) }
            else { name.clone() }
        } else {
            translate_inner(&b.domain, false, sig_names, ir)
        };
        for v in &b.vars {
            vars.push((v.clone(), d.clone(), b.disj));
        }
    }

    // Build disj checks
    let mut disj_checks = Vec::new();
    let mut i = 0;
    while i < vars.len() {
        if vars[i].2 {
            let domain = &vars[i].1;
            let start = i;
            while i < vars.len() && vars[i].2 && vars[i].1 == *domain { i += 1; }
            for a in start..i {
                for b_idx in (a+1)..i {
                    disj_checks.push(format!("{} !== {}", vars[a].0, vars[b_idx].0));
                }
            }
        } else { i += 1; }
    }

    let guarded_body = if disj_checks.is_empty() {
        body_str.to_string()
    } else {
        let guard = disj_checks.join(" && ");
        match kind {
            QuantKind::All | QuantKind::No => format!("{guard} ? {body_str} : true"),
            QuantKind::Some => format!("{guard} && {body_str}"),
        }
    };

    // Build from inside-out.
    // Wrap domain in [...domain] to handle both Set and Array uniformly.
    let mut result = guarded_body;
    for idx in (0..vars.len()).rev() {
        let (ref var, ref domain, _) = vars[idx];
        let arr = format!("[...{domain}]");
        result = match kind {
            QuantKind::All => format!("{arr}.every(({var}) => {result})"),
            QuantKind::Some => format!("{arr}.some(({var}) => {result})"),
            QuantKind::No => {
                if idx == 0 {
                    format!("!{arr}.some(({var}) => {result})")
                } else {
                    format!("{arr}.some(({var}) => {result})")
                }
            }
        };
    }
    result
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
    ir: &OxidtrIR,
    ti: &F,
) -> String
where
    F: Fn(&Expr, bool) -> String,
{
    let l = ti(left, false);
    let r = ti(right, false);
    // For struct types (non-primitive), use JSON.stringify for deep comparison.
    // Detect: if either side is a VarRef bound to a sig, or a FieldAccess to a
    // struct field (One mult), we need deep comparison.
    let needs_deep = is_struct_expr(left, ir) || is_struct_expr(right, ir);
    if needs_deep {
        if op == "===" {
            format!("JSON.stringify({l}) === JSON.stringify({r})")
        } else {
            format!("JSON.stringify({l}) !== JSON.stringify({r})")
        }
    } else {
        format!("{l} {op} {r}")
    }
}

/// Check if an expression likely evaluates to a struct (non-primitive) value.
fn is_struct_expr(expr: &Expr, ir: &OxidtrIR) -> bool {
    match expr {
        Expr::FieldAccess { field, .. } => {
            // Look up field type in IR. If target is a sig with fields, it's a struct.
            for s in &ir.structures {
                for f in &s.fields {
                    if f.name == *field && f.mult == Multiplicity::One {
                        return ir.structures.iter()
                            .any(|target_sig| target_sig.name == f.target && !target_sig.fields.is_empty());
                    }
                }
            }
            false
        }
        // Deref: *expr in Rust, just expr in TS — check inner
        _ => false,
    }
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
