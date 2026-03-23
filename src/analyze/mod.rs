/// Constraint analysis: extract structured information from ConstraintNode expressions.
/// Used by fixtures, schemas, doc comments, Bean Validation, and TryFrom generation.

use crate::parser::ast::*;
use crate::ir::nodes::*;

/// A structured constraint extracted from an Alloy fact expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintInfo {
    /// Cardinality bound: #sig.field <= N or #sig.field >= N
    CardinalityBound {
        sig_name: String,
        field_name: String,
        bound: BoundKind,
    },
    /// Non-null / presence: some sig.field or no sig.field
    Presence {
        sig_name: String,
        field_name: String,
        kind: PresenceKind,
    },
    /// Membership: x in sig.field or x not in sig.field
    Membership {
        sig_name: String,
        field_name: String,
    },
    /// Self-reference exclusion: all s: S | s not in s.field
    NoSelfRef {
        sig_name: String,
        field_name: String,
    },
    /// Acyclicity: no s: S | s in s.^field
    Acyclic {
        sig_name: String,
        field_name: String,
    },
    /// Generic named constraint (for doc comments)
    Named {
        name: String,
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundKind {
    Exact(usize),
    AtMost(usize),
    AtLeast(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceKind {
    Required, // some
    Absent,   // no
}

/// Analyze all constraints in the IR and return structured info.
pub fn analyze(ir: &OxidtrIR) -> Vec<ConstraintInfo> {
    let mut results = Vec::new();
    for c in &ir.constraints {
        let name = c.name.clone().unwrap_or_default();
        results.extend(analyze_expr(&c.expr, &name));
    }
    results
}

/// Get constraints relevant to a specific sig.
pub fn constraints_for_sig(ir: &OxidtrIR, sig_name: &str) -> Vec<ConstraintInfo> {
    analyze(ir).into_iter().filter(|c| match c {
        ConstraintInfo::CardinalityBound { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::Presence { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::Membership { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::NoSelfRef { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::Acyclic { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::Named { .. } => false,
    }).collect()
}

/// Get constraint names relevant to a specific sig (for doc comments).
pub fn constraint_names_for_sig(ir: &OxidtrIR, sig_name: &str) -> Vec<String> {
    ir.constraints.iter()
        .filter(|c| c.name.is_some() && expr_references_sig(&c.expr, sig_name))
        .filter_map(|c| c.name.clone())
        .collect()
}

/// Render a constraint expression to a human-readable description.
pub fn describe_expr(expr: &Expr) -> String {
    match expr {
        Expr::Quantifier { kind, var, domain, body } => {
            let q = match kind {
                QuantKind::All => "for all",
                QuantKind::Some => "there exists",
                QuantKind::No => "no",
            };
            let d = describe_expr(domain);
            let b = describe_expr(body);
            format!("{q} {var}: {d} | {b}")
        }
        Expr::Comparison { op, left, right } => {
            let o = match op {
                CompareOp::Eq => "=",
                CompareOp::NotEq => "!=",
                CompareOp::In => "in",
            };
            format!("{} {o} {}", describe_expr(left), describe_expr(right))
        }
        Expr::BinaryLogic { op, left, right } => {
            let o = match op {
                LogicOp::And => "and",
                LogicOp::Or => "or",
                LogicOp::Implies => "implies",
                LogicOp::Iff => "iff",
            };
            format!("{} {o} {}", describe_expr(left), describe_expr(right))
        }
        Expr::Not(inner) => format!("not {}", describe_expr(inner)),
        Expr::Cardinality(inner) => format!("#{}", describe_expr(inner)),
        Expr::TransitiveClosure(inner) => format!("^{}", describe_expr(inner)),
        Expr::FieldAccess { base, field } => format!("{}.{field}", describe_expr(base)),
        Expr::VarRef(name) => name.clone(),
    }
}

fn analyze_expr(expr: &Expr, fact_name: &str) -> Vec<ConstraintInfo> {
    let mut results = Vec::new();

    match expr {
        // all s: Sig | ... → recurse into body with sig context
        Expr::Quantifier { kind: QuantKind::All, var, domain, body } => {
            if let Expr::VarRef(sig_name) = domain.as_ref() {
                analyze_body_for_sig(body, sig_name, var, &mut results);
            }
        }
        // no s: Sig | s in s.^field → Acyclic
        Expr::Quantifier { kind: QuantKind::No, var, domain, body } => {
            if let Expr::VarRef(sig_name) = domain.as_ref() {
                if let Expr::Comparison { op: CompareOp::In, left, right } = body.as_ref() {
                    if let Expr::VarRef(v) = left.as_ref() {
                        if v == var {
                            if let Expr::TransitiveClosure(inner) = right.as_ref() {
                                if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                                    results.push(ConstraintInfo::Acyclic {
                                        sig_name: sig_name.clone(),
                                        field_name: field.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    // Always add a named constraint if the fact has a name
    if !fact_name.is_empty() {
        results.push(ConstraintInfo::Named {
            name: fact_name.to_string(),
            description: describe_expr(expr),
        });
    }

    results
}

fn analyze_body_for_sig(
    body: &Expr,
    sig_name: &str,
    var: &str,
    results: &mut Vec<ConstraintInfo>,
) {
    match body {
        // s not in s.field → NoSelfRef
        Expr::Not(inner) => {
            if let Expr::Comparison { op: CompareOp::In, left, right } = inner.as_ref() {
                if let (Expr::VarRef(v), Expr::FieldAccess { base, field }) = (left.as_ref(), right.as_ref()) {
                    if v == var {
                        if let Expr::VarRef(b) = base.as_ref() {
                            if b == var {
                                results.push(ConstraintInfo::NoSelfRef {
                                    sig_name: sig_name.to_string(),
                                    field_name: field.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
        // #s.field = N or #s.field <= N etc. (via Comparison on Cardinality)
        Expr::Comparison { op, left, .. } => {
            // Look for cardinality on left
            if let Expr::Cardinality(inner) = left.as_ref() {
                if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                    // Try to detect self-referencing #x.field pattern
                    // For now, mark as cardinality constraint
                    if let CompareOp::Eq = op {
                        results.push(ConstraintInfo::CardinalityBound {
                            sig_name: sig_name.to_string(),
                            field_name: field.clone(),
                            bound: BoundKind::Exact(0), // placeholder
                        });
                    }
                }
            }
        }
        // Conjunction: analyze both sides
        Expr::BinaryLogic { op: LogicOp::And, left, right } => {
            analyze_body_for_sig(left, sig_name, var, results);
            analyze_body_for_sig(right, sig_name, var, results);
        }
        // Implication body
        Expr::BinaryLogic { op: LogicOp::Implies, right, .. } => {
            analyze_body_for_sig(right, sig_name, var, results);
        }
        _ => {}
    }
}

fn expr_references_sig(expr: &Expr, sig_name: &str) -> bool {
    match expr {
        Expr::VarRef(name) => name == sig_name,
        Expr::Quantifier { domain, body, .. } => {
            expr_references_sig(domain, sig_name) || expr_references_sig(body, sig_name)
        }
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. } => {
            expr_references_sig(left, sig_name) || expr_references_sig(right, sig_name)
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            expr_references_sig(inner, sig_name)
        }
        Expr::FieldAccess { base, .. } => expr_references_sig(base, sig_name),
    }
}
