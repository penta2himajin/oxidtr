/// Constraint analysis: extract structured information from ConstraintNode expressions.
/// Used by fixtures, schemas, doc comments, Bean Validation, and TryFrom generation.

pub mod guarantee;

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
    /// Biconditional: all s: S | A iff B
    Iff {
        sig_name: String,
        left: Expr,
        right: Expr,
    },
    /// Field ordering: all s: S | s.x <= s.y (direct field-to-field comparison)
    FieldOrdering {
        sig_name: String,
        left_field: String,
        op: CompareOp,
        right_field: String,
    },
    /// Implication: all s: S | condition implies consequent
    Implication {
        sig_name: String,
        condition: Expr,
        consequent: Expr,
    },
    /// Prohibition: no s: S | condition (negated existential)
    Prohibition {
        sig_name: String,
        condition: Expr,
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
        ConstraintInfo::Iff { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::FieldOrdering { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::Implication { sig_name: s, .. } => s == sig_name,
        ConstraintInfo::Prohibition { sig_name: s, .. } => s == sig_name,
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
        Expr::Quantifier { kind, bindings, body } => {
            let q = match kind {
                QuantKind::All => "for all",
                QuantKind::Some => "there exists",
                QuantKind::No => "no",
            };
            let bindings_str: Vec<String> = bindings.iter().map(|b| {
                let disj_prefix = if b.disj { "disj " } else { "" };
                let vars = b.vars.join(", ");
                let d = describe_expr(&b.domain);
                format!("{disj_prefix}{vars}: {d}")
            }).collect();
            let b = describe_expr(body);
            format!("{q} {} | {b}", bindings_str.join(", "))
        }
        Expr::Comparison { op, left, right } => {
            let o = match op {
                CompareOp::Eq => "=",
                CompareOp::NotEq => "!=",
                CompareOp::In => "in",
                CompareOp::Lt => "<",
                CompareOp::Gt => ">",
                CompareOp::Lte => "<=",
                CompareOp::Gte => ">=",
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
        Expr::IntLiteral(n) => n.to_string(),
        Expr::SetOp { op, left, right } => {
            let o = match op {
                SetOpKind::Union => "+",
                SetOpKind::Intersection => "&",
                SetOpKind::Difference => "-",
            };
            format!("{} {o} {}", describe_expr(left), describe_expr(right))
        }
        Expr::Product { left, right } => {
            format!("{} -> {}", describe_expr(left), describe_expr(right))
        }
        Expr::MultFormula { kind, expr } => {
            let q = match kind {
                QuantKind::Some => "some",
                QuantKind::No => "no",
                _ => "all",
            };
            format!("{q} {}", describe_expr(expr))
        }
    }
}

fn analyze_expr(expr: &Expr, fact_name: &str) -> Vec<ConstraintInfo> {
    let mut results = Vec::new();

    match expr {
        // all s: Sig | ... → recurse into body with sig context
        Expr::Quantifier { kind: QuantKind::All, bindings, body } => {
            // For backwards compatibility, handle single binding with single var
            if bindings.len() == 1 && bindings[0].vars.len() == 1 {
                let var = &bindings[0].vars[0];
                let domain = &bindings[0].domain;
                if let Expr::VarRef(sig_name) = domain {
                    analyze_body_for_sig(body, sig_name, var, &mut results);
                }
            }
        }
        // no s: Sig | ... → Acyclic or Prohibition
        Expr::Quantifier { kind: QuantKind::No, bindings, body } => {
            if bindings.len() == 1 && bindings[0].vars.len() == 1 {
                let var = &bindings[0].vars[0];
                let domain = &bindings[0].domain;
                if let Expr::VarRef(sig_name) = domain {
                    let mut is_acyclic = false;
                    // s in s.^field → Acyclic
                    if let Expr::Comparison { op: CompareOp::In, left, right } = body.as_ref() {
                        if let Expr::VarRef(v) = left.as_ref() {
                            if v == var {
                                if let Expr::TransitiveClosure(inner) = right.as_ref() {
                                    if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                                        results.push(ConstraintInfo::Acyclic {
                                            sig_name: sig_name.clone(),
                                            field_name: field.clone(),
                                        });
                                        is_acyclic = true;
                                    }
                                }
                            }
                        }
                    }
                    // Other no-quantifier patterns → Prohibition
                    if !is_acyclic {
                        results.push(ConstraintInfo::Prohibition {
                            sig_name: sig_name.clone(),
                            condition: substitute_var(body, var, sig_name),
                        });
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
        // OR s.fieldA op s.fieldB → FieldOrdering
        Expr::Comparison { op, left, right } => {
            // Look for cardinality on left
            if let Expr::Cardinality(inner) = left.as_ref() {
                if let Expr::FieldAccess { field, .. } = inner.as_ref() {
                    let n = extract_int(right);
                    let bound = match op {
                        CompareOp::Eq => n.map(|v| BoundKind::Exact(v as usize)),
                        CompareOp::Lte => n.map(|v| BoundKind::AtMost(v as usize)),
                        CompareOp::Lt => n.map(|v| BoundKind::AtMost((v - 1) as usize)),
                        CompareOp::Gte => n.map(|v| BoundKind::AtLeast(v as usize)),
                        CompareOp::Gt => n.map(|v| BoundKind::AtLeast((v + 1) as usize)),
                        _ => None,
                    };
                    if let Some(bound) = bound {
                        results.push(ConstraintInfo::CardinalityBound {
                            sig_name: sig_name.to_string(),
                            field_name: field.clone(),
                            bound,
                        });
                    }
                }
            }
            // FieldOrdering: s.fieldA op s.fieldB (non-cardinality, non-tautological)
            if !matches!(left.as_ref(), Expr::Cardinality(_))
                && matches!(op, CompareOp::Lt | CompareOp::Gt | CompareOp::Lte | CompareOp::Gte)
            {
                if let (
                    Expr::FieldAccess { base: bl, field: fl },
                    Expr::FieldAccess { base: br, field: fr },
                ) = (left.as_ref(), right.as_ref()) {
                    if let (Expr::VarRef(vl), Expr::VarRef(vr)) = (bl.as_ref(), br.as_ref()) {
                        if vl == var && vr == var && fl != fr {
                            results.push(ConstraintInfo::FieldOrdering {
                                sig_name: sig_name.to_string(),
                                left_field: fl.clone(),
                                op: op.clone(),
                                right_field: fr.clone(),
                            });
                        }
                    }
                }
            }
        }
        // Conjunction: analyze both sides
        Expr::BinaryLogic { op: LogicOp::And, left, right } => {
            analyze_body_for_sig(left, sig_name, var, results);
            analyze_body_for_sig(right, sig_name, var, results);
        }
        // Iff: A iff B → biconditional constraint
        Expr::BinaryLogic { op: LogicOp::Iff, left, right } => {
            results.push(ConstraintInfo::Iff {
                sig_name: sig_name.to_string(),
                left: substitute_var(left, var, sig_name),
                right: substitute_var(right, var, sig_name),
            });
        }
        // Implication: A implies B → emit Implication + recurse into consequent
        Expr::BinaryLogic { op: LogicOp::Implies, left, right } => {
            results.push(ConstraintInfo::Implication {
                sig_name: sig_name.to_string(),
                condition: substitute_var(left, var, sig_name),
                consequent: substitute_var(right, var, sig_name),
            });
            analyze_body_for_sig(right, sig_name, var, results);
        }
        _ => {}
    }
}

/// Bean Validation annotation for a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeanValidation {
    /// @Size(min=N, max=M) for collection fields
    Size { min: Option<usize>, max: Option<usize>, fact_name: String },
    /// @Min/@Max for comparison constraints (no integer literal in AST)
    MinMax { fact_name: String },
}

/// Get Bean Validation annotations for a specific field on a sig.
pub fn bean_validations_for_field(ir: &OxidtrIR, sig_name: &str, field_name: &str) -> Vec<BeanValidation> {
    let mut results = Vec::new();
    let constraints = constraints_for_sig(ir, sig_name);
    for c in &constraints {
        match c {
            ConstraintInfo::CardinalityBound { field_name: fname, bound, .. } if fname == field_name => {
                let (min, max) = match bound {
                    BoundKind::Exact(n) => (Some(*n), Some(*n)),
                    BoundKind::AtMost(n) => (None, Some(*n)),
                    BoundKind::AtLeast(n) => (Some(*n), None),
                };
                // Find the fact name for this constraint
                let fact_name = ir.constraints.iter()
                    .filter_map(|cn| cn.name.clone())
                    .find(|name| {
                        ir.constraints.iter().any(|cn| cn.name.as_deref() == Some(name)
                            && expr_references_sig(&cn.expr, sig_name))
                    })
                    .unwrap_or_default();
                results.push(BeanValidation::Size { min, max, fact_name });
            }
            _ => {}
        }
    }
    // Check for Comparison constraints referencing this field (for @Min/@Max)
    for c in &ir.constraints {
        let fact_name = match &c.name {
            Some(n) => n.clone(),
            None => continue,
        };
        if expr_has_comparison_on_field(&c.expr, sig_name, field_name) {
            results.push(BeanValidation::MinMax { fact_name });
        }
    }
    results
}

/// Check if an expression contains a direct comparison (not cardinality) on a field.
fn expr_has_comparison_on_field(expr: &Expr, sig_name: &str, field_name: &str) -> bool {
    match expr {
        Expr::Quantifier { kind: QuantKind::All, bindings, body } => {
            if bindings.len() == 1 && bindings[0].vars.len() == 1 {
                if let Expr::VarRef(name) = &bindings[0].domain {
                    if name == sig_name {
                        return body_has_comparison_on_field(body, field_name);
                    }
                }
            }
            false
        }
        _ => false,
    }
}

fn body_has_comparison_on_field(body: &Expr, field_name: &str) -> bool {
    match body {
        Expr::Comparison { op, left, right, .. } => {
            // Skip cardinality comparisons (handled by @Size) and In comparisons
            if matches!(op, CompareOp::In) { return false; }
            if matches!(left.as_ref(), Expr::Cardinality(_)) { return false; }
            let left_matches = field_access_matches(left, field_name);
            let right_matches = field_access_matches(right, field_name);
            // Only flag if one side is the field and the other is different
            // (e.g., u.role = someValue, not u.role = u.role which is tautological)
            if left_matches && right_matches { return false; }
            left_matches || right_matches
        }
        Expr::BinaryLogic { op: LogicOp::And, left, right } => {
            body_has_comparison_on_field(left, field_name)
                || body_has_comparison_on_field(right, field_name)
        }
        Expr::BinaryLogic { op: LogicOp::Implies, right, .. } => {
            body_has_comparison_on_field(right, field_name)
        }
        _ => false,
    }
}

fn field_access_matches(expr: &Expr, field_name: &str) -> bool {
    matches!(expr, Expr::FieldAccess { field, .. } if field == field_name)
}

/// Extract an integer value from an expression, if it is an IntLiteral.
fn extract_int(expr: &Expr) -> Option<i64> {
    if let Expr::IntLiteral(n) = expr {
        Some(*n)
    } else {
        None
    }
}

/// Substitute a quantifier variable in an expression with the sig name.
/// This normalizes expressions so they reference the sig name directly.
fn substitute_var(expr: &Expr, var: &str, sig_name: &str) -> Expr {
    match expr {
        Expr::VarRef(name) => {
            if name == var {
                Expr::VarRef(sig_name.to_string())
            } else {
                expr.clone()
            }
        }
        Expr::FieldAccess { base, field } => Expr::FieldAccess {
            base: Box::new(substitute_var(base, var, sig_name)),
            field: field.clone(),
        },
        Expr::Comparison { op, left, right } => Expr::Comparison {
            op: op.clone(),
            left: Box::new(substitute_var(left, var, sig_name)),
            right: Box::new(substitute_var(right, var, sig_name)),
        },
        Expr::BinaryLogic { op, left, right } => Expr::BinaryLogic {
            op: op.clone(),
            left: Box::new(substitute_var(left, var, sig_name)),
            right: Box::new(substitute_var(right, var, sig_name)),
        },
        Expr::Not(inner) => Expr::Not(Box::new(substitute_var(inner, var, sig_name))),
        Expr::Cardinality(inner) => Expr::Cardinality(Box::new(substitute_var(inner, var, sig_name))),
        Expr::TransitiveClosure(inner) => Expr::TransitiveClosure(Box::new(substitute_var(inner, var, sig_name))),
        Expr::SetOp { op, left, right } => Expr::SetOp {
            op: *op,
            left: Box::new(substitute_var(left, var, sig_name)),
            right: Box::new(substitute_var(right, var, sig_name)),
        },
        Expr::Product { left, right } => Expr::Product {
            left: Box::new(substitute_var(left, var, sig_name)),
            right: Box::new(substitute_var(right, var, sig_name)),
        },
        Expr::Quantifier { kind, bindings, body } => Expr::Quantifier {
            kind: kind.clone(),
            bindings: bindings.clone(),
            body: Box::new(substitute_var(body, var, sig_name)),
        },
        Expr::MultFormula { kind, expr } => Expr::MultFormula {
            kind: kind.clone(),
            expr: Box::new(substitute_var(expr, var, sig_name)),
        },
        Expr::IntLiteral(_) => expr.clone(),
    }
}

/// Get the bound for a specific field on a sig, if one exists.
pub fn bounds_for_field(ir: &OxidtrIR, sig_name: &str, field_name: &str) -> Option<BoundKind> {
    let constraints = constraints_for_sig(ir, sig_name);
    for c in &constraints {
        if let ConstraintInfo::CardinalityBound { field_name: fname, bound, .. } = c {
            if fname == field_name {
                return Some(bound.clone());
            }
        }
    }
    None
}

/// Check if a quantifier expression uses `disj` on a binding that iterates a specific sig's field.
/// Returns a list of (sig_name, field_name) pairs where `disj` implies uniqueness.
pub fn disj_fields(ir: &OxidtrIR) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for c in &ir.constraints {
        collect_disj_fields(&c.expr, &mut results);
    }
    results.sort();
    results.dedup();
    results
}

fn collect_disj_fields(expr: &Expr, results: &mut Vec<(String, String)>) {
    match expr {
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings {
                if b.disj {
                    // If domain is sig.field, extract (sig, field)
                    if let Expr::FieldAccess { base, field } = &b.domain {
                        if let Expr::VarRef(sig) = base.as_ref() {
                            results.push((sig.clone(), field.clone()));
                        }
                    }
                    // If domain is just a VarRef (sig name), the disj applies to the sig iteration
                    // This doesn't directly map to a field, but we note it for doc comments
                }
            }
            collect_disj_fields(body, results);
        }
        Expr::BinaryLogic { left, right, .. } | Expr::Comparison { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            collect_disj_fields(left, results);
            collect_disj_fields(right, results);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            collect_disj_fields(inner, results);
        }
        Expr::MultFormula { expr: inner, .. } => collect_disj_fields(inner, results),
        Expr::FieldAccess { base, .. } => collect_disj_fields(base, results),
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
}

/// Classify a body expression as pre-condition or post-condition.
/// Returns true if the expression is a pre-condition (references only input params).
pub fn is_pre_condition(expr: &Expr, param_names: &[String]) -> bool {
    match expr {
        Expr::Comparison { op: CompareOp::In, .. } => {
            // Membership checks are typically pre-conditions
            true
        }
        Expr::Comparison { left, right, .. } => {
            // If both sides only reference params, it's a pre-condition
            expr_only_refs_params(left, param_names) && expr_only_refs_params(right, param_names)
        }
        Expr::BinaryLogic { left, right, .. } => {
            is_pre_condition(left, param_names) && is_pre_condition(right, param_names)
        }
        Expr::Quantifier { body, .. } => {
            is_pre_condition(body, param_names)
        }
        Expr::Not(inner) => is_pre_condition(inner, param_names),
        _ => {
            // Field accesses on params are pre-conditions (param guards)
            expr_only_refs_params(expr, param_names)
        }
    }
}

/// Check if an expression only references parameter names (no state fields).
fn expr_only_refs_params(expr: &Expr, param_names: &[String]) -> bool {
    match expr {
        Expr::VarRef(name) => param_names.contains(name),
        Expr::IntLiteral(_) => true,
        Expr::FieldAccess { base, .. } => {
            // Field access on a param is still a param reference (e.g., a.balance)
            expr_only_refs_params(base, param_names)
        }
        Expr::Cardinality(inner) | Expr::Not(inner) | Expr::TransitiveClosure(inner)
        | Expr::MultFormula { expr: inner, .. } => {
            expr_only_refs_params(inner, param_names)
        }
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            expr_only_refs_params(left, param_names) && expr_only_refs_params(right, param_names)
        }
        Expr::Quantifier { bindings, body, .. } => {
            // Quantifier vars are local, check domain and body
            let mut extended_params: Vec<String> = param_names.to_vec();
            for b in bindings {
                extended_params.extend(b.vars.clone());
            }
            bindings.iter().all(|b| expr_only_refs_params(&b.domain, param_names))
                && expr_only_refs_params(body, &extended_params)
        }
    }
}

/// Render an expression back to valid Alloy syntax.
/// Unlike `describe_expr` which is human-readable, this produces parseable Alloy.
pub fn alloy_repr(expr: &Expr) -> String {
    match expr {
        Expr::Quantifier { kind, bindings, body } => {
            let q = match kind {
                QuantKind::All => "all",
                QuantKind::Some => "some",
                QuantKind::No => "no",
            };
            let bindings_str: Vec<String> = bindings.iter().map(|b| {
                let disj_prefix = if b.disj { "disj " } else { "" };
                let vars = b.vars.join(", ");
                let d = alloy_repr(&b.domain);
                format!("{disj_prefix}{vars}: {d}")
            }).collect();
            let b = alloy_repr(body);
            format!("{q} {} | {b}", bindings_str.join(", "))
        }
        Expr::Comparison { op, left, right } => {
            let o = match op {
                CompareOp::Eq => "=",
                CompareOp::NotEq => "!=",
                CompareOp::In => "in",
                CompareOp::Lt => "<",
                CompareOp::Gt => ">",
                CompareOp::Lte => "<=",
                CompareOp::Gte => ">=",
            };
            format!("{} {} {}", alloy_repr(left), o, alloy_repr(right))
        }
        Expr::BinaryLogic { op, left, right } => {
            let o = match op {
                LogicOp::And => "and",
                LogicOp::Or => "or",
                LogicOp::Implies => "implies",
                LogicOp::Iff => "iff",
            };
            let left_str = alloy_repr_maybe_paren(left, op);
            let right_str = alloy_repr_maybe_paren(right, op);
            format!("{left_str} {o} {right_str}")
        }
        Expr::Not(inner) => format!("not {}", alloy_repr_atom(inner)),
        Expr::Cardinality(inner) => format!("#{}", alloy_repr_atom(inner)),
        Expr::TransitiveClosure(inner) => format!("^{}", alloy_repr_atom(inner)),
        Expr::FieldAccess { base, field } => format!("{}.{field}", alloy_repr_atom(base)),
        Expr::VarRef(name) => name.clone(),
        Expr::IntLiteral(n) => n.to_string(),
        Expr::SetOp { op, left, right } => {
            let o = match op {
                SetOpKind::Union => "+",
                SetOpKind::Intersection => "&",
                SetOpKind::Difference => "-",
            };
            format!("{} {o} {}", alloy_repr(left), alloy_repr(right))
        }
        Expr::Product { left, right } => {
            format!("{} -> {}", alloy_repr(left), alloy_repr(right))
        }
        Expr::MultFormula { kind, expr } => {
            let q = match kind {
                QuantKind::Some => "some",
                QuantKind::No => "no",
                _ => "all",
            };
            format!("{q} {}", alloy_repr(expr))
        }
    }
}

/// Wrap in parens if the sub-expression is a lower-precedence binary logic.
fn alloy_repr_maybe_paren(expr: &Expr, parent_op: &LogicOp) -> String {
    match expr {
        Expr::BinaryLogic { op, .. } if needs_paren(op, parent_op) => {
            format!("({})", alloy_repr(expr))
        }
        _ => alloy_repr(expr),
    }
}

/// Whether a child op needs parens inside a parent op.
fn needs_paren(child: &LogicOp, parent: &LogicOp) -> bool {
    let precedence = |op: &LogicOp| -> u8 {
        match op {
            LogicOp::Iff => 0,
            LogicOp::Implies => 1,
            LogicOp::Or => 2,
            LogicOp::And => 3,
        }
    };
    precedence(child) < precedence(parent)
}

/// Render an expression as an atomic unit (add parens if it's complex).
fn alloy_repr_atom(expr: &Expr) -> String {
    match expr {
        Expr::VarRef(_) | Expr::IntLiteral(_) => alloy_repr(expr),
        Expr::FieldAccess { .. } => alloy_repr(expr),
        _ => format!("({})", alloy_repr(expr)),
    }
}

/// Look up the sig multiplicity for a given sig name in the IR.
/// Returns `SigMultiplicity::Default` if the sig is not found.
pub fn sig_multiplicity_for(ir: &OxidtrIR, sig_name: &str) -> SigMultiplicity {
    ir.structures.iter()
        .find(|s| s.name == sig_name)
        .map(|s| s.sig_multiplicity)
        .unwrap_or(SigMultiplicity::Default)
}

/// Collect set operations found in constraint expressions that reference a given field.
/// Returns a list of (SetOpKind, left_operand, right_operand) descriptions for the field.
pub fn set_ops_for_field(ir: &OxidtrIR, sig_name: &str, field_name: &str) -> Vec<(SetOpKind, String, String)> {
    let mut results = Vec::new();
    for c in &ir.constraints {
        collect_set_ops_for_field(&c.expr, sig_name, field_name, &mut results);
    }
    results
}

fn collect_set_ops_for_field(
    expr: &Expr,
    sig_name: &str,
    field_name: &str,
    results: &mut Vec<(SetOpKind, String, String)>,
) {
    match expr {
        Expr::Quantifier { kind: QuantKind::All, bindings, body } => {
            if bindings.len() == 1 && bindings[0].vars.len() == 1 {
                if let Expr::VarRef(name) = &bindings[0].domain {
                    if name == sig_name {
                        collect_set_ops_in_body(body, field_name, results);
                    }
                }
            }
        }
        Expr::BinaryLogic { left, right, .. } => {
            collect_set_ops_for_field(left, sig_name, field_name, results);
            collect_set_ops_for_field(right, sig_name, field_name, results);
        }
        _ => {}
    }
}

fn collect_set_ops_in_body(
    expr: &Expr,
    field_name: &str,
    results: &mut Vec<(SetOpKind, String, String)>,
) {
    match expr {
        Expr::Comparison { left, right, .. } => {
            // Check if left references the field and right is a set op (or vice versa)
            if field_access_matches(left, field_name) {
                if let Expr::SetOp { op, left: sl, right: sr } = right.as_ref() {
                    results.push((*op, describe_expr(sl), describe_expr(sr)));
                }
            }
            if field_access_matches(right, field_name) {
                if let Expr::SetOp { op, left: sl, right: sr } = left.as_ref() {
                    results.push((*op, describe_expr(sl), describe_expr(sr)));
                }
            }
            // Also check if the set op contains a field reference
            collect_set_ops_in_body(left, field_name, results);
            collect_set_ops_in_body(right, field_name, results);
        }
        Expr::SetOp { op, left, right } => {
            // If either side is a field access to our field
            if field_access_matches(left, field_name) || field_access_matches(right, field_name) {
                results.push((*op, describe_expr(left), describe_expr(right)));
            }
            collect_set_ops_in_body(left, field_name, results);
            collect_set_ops_in_body(right, field_name, results);
        }
        Expr::BinaryLogic { left, right, .. } => {
            collect_set_ops_in_body(left, field_name, results);
            collect_set_ops_in_body(right, field_name, results);
        }
        _ => {}
    }
}

fn expr_references_sig(expr: &Expr, sig_name: &str) -> bool {
    match expr {
        Expr::VarRef(name) => name == sig_name,
        Expr::IntLiteral(_) => false,
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| expr_references_sig(&b.domain, sig_name))
                || expr_references_sig(body, sig_name)
        }
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            expr_references_sig(left, sig_name) || expr_references_sig(right, sig_name)
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner)
        | Expr::MultFormula { expr: inner, .. } => {
            expr_references_sig(inner, sig_name)
        }
        Expr::FieldAccess { base, .. } => expr_references_sig(base, sig_name),
    }
}
