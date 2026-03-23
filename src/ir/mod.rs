pub mod nodes;

use crate::parser::ast::{AlloyModel, SigDecl, FactDecl, PredDecl, FunDecl, AssertDecl};
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
    operations.extend(model.funs.iter().map(lower_fun));
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
            mult: f.mult.clone(),
            target: f.target.clone(),
        })
        .collect();

    StructureNode {
        name: sig.name.clone(),
        is_enum: sig.is_abstract,
        sig_multiplicity: sig.multiplicity,
        parent: sig.parent.clone(),
        fields,
    }
}

fn lower_fact(fact: &FactDecl) -> ConstraintNode {
    ConstraintNode {
        name: fact.name.clone(),
        expr: fact.body.clone(),
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
        params,
        return_type: None,
        body: pred.body.clone(),
    }
}

fn lower_fun(fun: &FunDecl) -> OperationNode {
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
        params,
        return_type: Some(IRReturnType {
            mult: fun.return_mult.clone(),
            type_name: fun.return_type.clone(),
        }),
        body: vec![fun.body.clone()],
    }
}

fn lower_assert(assert_decl: &AssertDecl) -> PropertyNode {
    PropertyNode {
        name: assert_decl.name.clone(),
        expr: assert_decl.body.clone(),
    }
}
