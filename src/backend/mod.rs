pub mod rust;
pub mod typescript;
pub mod jvm;
pub mod schema;

use crate::parser::ast::{Expr, CompareOp, QuantKind};
use crate::ir::nodes::OxidtrIR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

/// Detect direct ownership pattern: `all x: A | some y: B | x in y.field`
/// Returns (owned_param_name, owner_param_name, field_name) using the
/// provided name-transform function to build param names from type names.
pub fn detect_ownership_pattern<F>(expr: &Expr, _ir: &OxidtrIR, name_fn: F) -> Option<(String, String, String, String)>
where F: Fn(&str) -> String {
    if let Expr::Quantifier { kind: QuantKind::All, bindings, body } = expr {
        if bindings.len() != 1 || bindings[0].vars.len() != 1 { return None; }
        let owned_var = &bindings[0].vars[0];
        let owned_type = if let Expr::VarRef(name) = &bindings[0].domain { name.clone() } else { return None; };

        if let Expr::Quantifier { kind: QuantKind::Some, bindings: inner_bindings, body: inner_body } = body.as_ref() {
            if inner_bindings.len() == 1 && inner_bindings[0].vars.len() == 1 {
                let owner_var = &inner_bindings[0].vars[0];
                let owner_type = if let Expr::VarRef(name) = &inner_bindings[0].domain { name.clone() } else { return None; };

                if let Expr::Comparison { op: CompareOp::In, left, right } = inner_body.as_ref() {
                    if let (Expr::VarRef(lvar), Expr::FieldAccess { base, field }) = (left.as_ref(), right.as_ref()) {
                        if let Expr::VarRef(rvar) = base.as_ref() {
                            if lvar == owned_var && rvar == owner_var {
                                return Some((name_fn(&owned_type), name_fn(&owner_type), owner_type, field.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}
