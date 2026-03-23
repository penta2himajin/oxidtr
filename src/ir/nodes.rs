/// oxidtr IR — language-independent intermediate representation.
/// Lowered from Alloy AST; consumed by target backends.

use crate::parser::ast::{Expr, Multiplicity, SigMultiplicity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IRField {
    pub name: String,
    pub mult: Multiplicity,
    pub target: String, // refers to StructureNode name
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureNode {
    pub name: String,
    pub is_enum: bool,
    pub sig_multiplicity: SigMultiplicity,
    pub parent: Option<String>,
    pub fields: Vec<IRField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintNode {
    pub name: Option<String>,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IRParam {
    pub name: String,
    pub mult: Multiplicity,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationNode {
    pub name: String,
    pub params: Vec<IRParam>,
    pub body: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyNode {
    pub name: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OxidtrIR {
    pub structures: Vec<StructureNode>,
    pub constraints: Vec<ConstraintNode>,
    pub operations: Vec<OperationNode>,
    pub properties: Vec<PropertyNode>,
}
