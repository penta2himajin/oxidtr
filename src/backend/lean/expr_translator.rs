use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use std::collections::HashSet;

pub fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir)
}

pub fn to_lower_camel(name: &str) -> String {
    if name.is_empty() { return name.to_string(); }
    let mut chars = name.chars();
    let first = chars.next().unwrap().to_lowercase().to_string();
    format!("{first}{}", chars.collect::<String>())
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

        Expr::VarRef(name) => {
            if sig_names.contains(name) {
                name.clone()
            } else {
                to_lower_camel(name)
            }
        }

        Expr::FieldAccess { base, field } => {
            format!("{}.{}", ti(base, false), to_lower_camel(field))
        }

        Expr::Cardinality(inner) => format!("{}.length", ti(inner, false)),

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Relation.TransGen (· {} ·) {}", to_lower_camel(field), ti(base, false))
            } else {
                format!("Relation.TransGen {}", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                CompareOp::Eq => format!("{} = {}", ti(left, false), ti(right, false)),
                CompareOp::NotEq => format!("{} ≠ {}", ti(left, false), ti(right, false)),
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} ≤ {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} ≥ {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    let l = ti(left, false);
                    let r = ti(right, false);
                    format!("{l} ∈ {r}")
                }
            }
        }

        Expr::BinaryLogic { op, left, right } => match op {
            LogicOp::And     => format!("{} ∧ {}", ti(left, true), ti(right, true)),
            LogicOp::Or      => format!("{} ∨ {}", ti(left, true), ti(right, true)),
            LogicOp::Implies => format!("{} → {}", ti(left, true), ti(right, false)),
            LogicOp::Iff     => format!("{} ↔ {}", ti(left, true), ti(right, true)),
        },

        Expr::Not(inner) => format!("¬{}", ti(inner, true)),

        Expr::Quantifier { kind, bindings, body } => {
            let quantifier = match kind {
                QuantKind::All => "∀",
                QuantKind::Some => "∃",
                QuantKind::No => "¬ ∃",
            };
            let binding_strs: Vec<String> = bindings.iter().map(|b| {
                let vars = b.vars.join(" ");
                let domain = ti(&b.domain, false);
                format!("{vars} : {domain}")
            }).collect();
            format!("{quantifier} {}, {}", binding_strs.join(", "), ti(body, false))
        }

        Expr::MultFormula { kind, expr: inner } => {
            match kind {
                QuantKind::Some => format!("{} ≠ none", ti(inner, false)),
                QuantKind::No => format!("{} = none", ti(inner, false)),
                _ => ti(inner, false),
            }
        }

        Expr::SetOp { op, left, right } => {
            let op_str = match op {
                SetOpKind::Union => "∪",
                SetOpKind::Intersection => "∩",
                SetOpKind::Difference => "\\",
            };
            format!("{} {} {}", ti(left, true), op_str, ti(right, true))
        }

        Expr::Product { left, right } => {
            format!("{} × {}", ti(left, true), ti(right, true))
        }

        Expr::Prime(inner) => {
            format!("{}' ", ti(inner, false)).trim().to_string()
        }

        Expr::TemporalUnary { op, expr: inner } => {
            let op_str = match op {
                TemporalUnaryOp::Always => "□",
                TemporalUnaryOp::Eventually => "◇",
                TemporalUnaryOp::After => "◯",
                TemporalUnaryOp::Historically => "■",
                TemporalUnaryOp::Once => "◆",
                TemporalUnaryOp::Before => "◯⁻¹",
            };
            format!("{op_str} {}", ti(inner, false))
        }

        Expr::TemporalBinary { op, left, right } => {
            let op_str = match op {
                TemporalBinaryOp::Until => "𝒰",
                TemporalBinaryOp::Since => "𝒮",
                TemporalBinaryOp::Release => "ℛ",
                TemporalBinaryOp::Triggered => "𝒯",
            };
            format!("{} {op_str} {}", ti(left, true), ti(right, true))
        }

        Expr::FunApp { name, receiver, args } => {
            let args_str: Vec<String> = args.iter().map(|a| ti(a, false)).collect();
            if let Some(recv) = receiver {
                if args_str.is_empty() {
                    format!("{}.{name}", ti(recv, false))
                } else {
                    format!("{}.{name} {}", ti(recv, false), args_str.join(" "))
                }
            } else if args_str.is_empty() {
                name.clone()
            } else {
                format!("{name} {}", args_str.join(" "))
            }
        }
    };

    if parens_if_complex && result.contains(' ') {
        format!("({result})")
    } else {
        result
    }
}
