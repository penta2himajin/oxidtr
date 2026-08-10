pub mod nodes;
pub mod algebra;

use crate::parser::ast::{AlloyModel, SigDecl, FactDecl, PredDecl, FunDecl, AssertDecl, Expr, QuantBinding};
use nodes::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweringError {
    InvalidReference { name: String, context: String },
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoweringError::InvalidReference { name, context } => {
                write!(f, "invalid reference to '{name}' in {context}")
            }
        }
    }
}

impl std::error::Error for LoweringError {}

pub fn lower(model: &AlloyModel) -> Result<OxidtrIR, LoweringError> {
    let structures = model.sigs.iter().map(lower_sig).collect();
    let constraints = model.facts.iter().map(lower_fact).collect();
    let mut operations: Vec<OperationNode> = model.preds.iter().map(lower_pred).collect();
    operations.extend(model.funs.iter().map(|f| lower_fun(f, model)));
    let properties = model.asserts.iter().map(lower_assert).collect();

    Ok(OxidtrIR {
        structures,
        constraints,
        operations,
        properties,
    })
}

fn lower_sig(sig: &SigDecl) -> StructureNode {
    let fields = sig
        .fields
        .iter()
        .map(|f| IRField {
            name: f.name.clone(),
            is_var: f.is_var,
            mult: f.mult.clone(),
            target: f.target.clone(),
            value_type: f.value_type.clone(),
            raw_union_type: f.raw_union_type.clone(), // propagate from AST annotation
        })
        .collect();

    StructureNode {
        name: sig.name.clone(),
        is_enum: sig.is_abstract,
        is_var: sig.is_var,
        sig_multiplicity: sig.multiplicity,
        parent: sig.parent.clone(),
        fields,
        intersection_of: sig.intersection_of.clone(),
        module: sig.module.clone(),
    }
}

fn lower_fact(fact: &FactDecl) -> ConstraintNode {
    ConstraintNode {
        name: fact.name.clone(),
        expr: fact.body.clone(),
        module: fact.module.clone(),
    }
}

fn lower_pred(pred: &PredDecl) -> OperationNode {
    let params = pred
        .params
        .iter()
        .map(|p| IRParam {
            name: p.name.clone(),
            mult: p.mult.clone(),
            type_name: p.type_name.clone(),
        })
        .collect();

    OperationNode {
        name: pred.name.clone(),
        receiver_sig: None,
        params,
        return_type: None,
        body: pred.body.clone(),
        module: pred.module.clone(),
    }
}

/// Field names reachable on `sig`, walking the inheritance chain: a field
/// declared on an abstract parent is a field of every child.
fn fields_of(sig: &str, model: &AlloyModel) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    let mut cur = Some(sig.to_string());
    while let Some(name) = cur {
        match model.sigs.iter().find(|s| s.name == name) {
            Some(s) => {
                names.extend(s.fields.iter().map(|f| f.name.clone()));
                cur = s.parent.clone();
            }
            None => break,
        }
    }
    names
}

/// Alloy's implicit receiver: inside `fun Sig.x { .. }` a bare field name means
/// `this.field`. Backends translate a `VarRef` as a plain identifier, so the
/// bare form referenced a name none of them had declared — invisible only
/// because the self-hosting model always writes `this.` explicitly.
///
/// A name bound closer wins: a parameter or a quantifier binder of the same
/// name shadows the field, exactly as it does in Alloy.
fn desugar_implicit_receiver(
    expr: &Expr,
    fields: &std::collections::HashSet<String>,
    bound: &std::collections::HashSet<String>,
) -> Expr {
    use Expr as E;
    let go = |e: &Expr| desugar_implicit_receiver(e, fields, bound);
    match expr {
        E::VarRef(name) if fields.contains(name) && !bound.contains(name) => E::FieldAccess {
            base: Box::new(E::VarRef("this".to_string())),
            field: name.clone(),
        },
        E::VarRef(_) | E::IntLiteral(_) => expr.clone(),
        // The base of a field access is rewritten, but the field name itself is
        // already a selector — not a reference that could mean `this.field`.
        E::FieldAccess { base, field } => E::FieldAccess {
            base: Box::new(go(base)),
            field: field.clone(),
        },
        E::Cardinality(i) => E::Cardinality(Box::new(go(i))),
        E::TransitiveClosure(i) => E::TransitiveClosure(Box::new(go(i))),
        E::ReflexiveClosure(i) => E::ReflexiveClosure(Box::new(go(i))),
        E::Not(i) => E::Not(Box::new(go(i))),
        E::Prime(i) => E::Prime(Box::new(go(i))),
        E::TemporalUnary { op, expr: i } => E::TemporalUnary { op: op.clone(), expr: Box::new(go(i)) },
        E::MultFormula { kind, expr: i } => E::MultFormula { kind: kind.clone(), expr: Box::new(go(i)) },
        E::Comparison { op, left, right } => E::Comparison {
            op: op.clone(), left: Box::new(go(left)), right: Box::new(go(right)),
        },
        E::BinaryLogic { op, left, right } => E::BinaryLogic {
            op: op.clone(), left: Box::new(go(left)), right: Box::new(go(right)),
        },
        E::SetOp { op, left, right } => E::SetOp {
            op: op.clone(), left: Box::new(go(left)), right: Box::new(go(right)),
        },
        E::Product { left, right } => E::Product {
            left: Box::new(go(left)), right: Box::new(go(right)),
        },
        E::TemporalBinary { op, left, right } => E::TemporalBinary {
            op: op.clone(), left: Box::new(go(left)), right: Box::new(go(right)),
        },
        E::FunApp { name, receiver, args } => E::FunApp {
            name: name.clone(),
            receiver: receiver.as_ref().map(|r| Box::new(go(r))),
            args: args.iter().map(&go).collect(),
        },
        E::Quantifier { kind, bindings, body } => {
            // Bindings are sequential, and each one shadows from its own domain
            // onward.
            let mut inner = bound.clone();
            let mut new_bindings = Vec::with_capacity(bindings.len());
            for b in bindings {
                let domain = desugar_implicit_receiver(&b.domain, fields, &inner);
                inner.extend(b.vars.iter().cloned());
                new_bindings.push(QuantBinding {
                    vars: b.vars.clone(),
                    domain,
                    disj: b.disj,
                });
            }
            E::Quantifier {
                kind: kind.clone(),
                bindings: new_bindings,
                body: Box::new(desugar_implicit_receiver(body, fields, &inner)),
            }
        }
    }
}

fn lower_fun(fun: &FunDecl, model: &AlloyModel) -> OperationNode {
    let params = fun
        .params
        .iter()
        .map(|p| IRParam {
            name: p.name.clone(),
            mult: p.mult.clone(),
            type_name: p.type_name.clone(),
        })
        .collect();

    OperationNode {
        name: fun.name.clone(),
        receiver_sig: fun.receiver_sig.clone(),
        params,
        return_type: Some(IRReturnType {
            mult: fun.return_mult.clone(),
            type_name: fun.return_type.clone(),
        }),
        body: vec![match &fun.receiver_sig {
            Some(sig) => {
                let fields = fields_of(sig, model);
                let bound: std::collections::HashSet<String> =
                    fun.params.iter().map(|p| p.name.clone()).collect();
                desugar_implicit_receiver(&fun.body, &fields, &bound)
            }
            None => fun.body.clone(),
        }],
        module: fun.module.clone(),
    }
}

fn lower_assert(assert_decl: &AssertDecl) -> PropertyNode {
    PropertyNode {
        name: assert_decl.name.clone(),
        expr: assert_decl.body.clone(),
        module: assert_decl.module.clone(),
    }
}
