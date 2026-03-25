/// oxidtr IR — language-independent intermediate representation.
/// Lowered from Alloy AST; consumed by target backends.

use crate::parser::ast::{Expr, Multiplicity, SigMultiplicity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IRField {
    pub name: String,
    pub is_var: bool, // Alloy 6: mutable field
    pub mult: Multiplicity,
    pub target: String, // refers to StructureNode name (key type for maps)
    pub value_type: Option<String>, // Some(B) for map fields (A -> B)
    /// Raw union type string from source language (e.g. "number | string").
    /// When present, backends use this for precise output instead of `target`.
    /// Alloy cannot express field-level union types; `target` holds the first variant.
    pub raw_union_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureNode {
    pub name: String,
    pub is_enum: bool,
    pub is_var: bool, // Alloy 6: `var sig` (mutable atom set across states)
    pub sig_multiplicity: SigMultiplicity,
    pub parent: Option<String>,
    pub fields: Vec<IRField>,
    /// For type aliases that are intersections (e.g. `type Base = A & B & C`).
    /// Backends render these as intersection/composition types rather than plain structs.
    pub intersection_of: Vec<String>,
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
pub struct IRReturnType {
    pub mult: Multiplicity,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationNode {
    pub name: String,
    pub params: Vec<IRParam>,
    pub return_type: Option<IRReturnType>,
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
