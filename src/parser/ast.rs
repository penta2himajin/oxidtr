/// Alloy AST — direct representation of parsed Alloy source.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Multiplicity {
    One,
    Lone,
    Set,
    Seq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecl {
    pub name: String,
    pub mult: Multiplicity,
    pub target: String, // refers to sig name (key type for maps)
    pub value_type: Option<String>, // Some(B) when declared as `A -> B` (map type)
    pub raw_union_type: Option<String>, // From `-- union: A | B` comment annotation
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SigMultiplicity {
    Default,  // plain `sig`
    One,      // `one sig` — exactly one instance (singleton)
    Some,     // `some sig` — one or more instances
    Lone,     // `lone sig` — zero or one instance
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigDecl {
    pub name: String,
    pub is_abstract: bool,
    pub multiplicity: SigMultiplicity,
    pub parent: Option<String>,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareOp {
    In,
    Eq,
    NotEq,
    Lt,
    Gt,
    Lte,
    Gte,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    Implies,
    Iff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantKind {
    All,
    Some,
    No,
}

/// A single quantifier binding: `[disj] x, y, ...: Domain`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantBinding {
    pub vars: Vec<String>,
    pub domain: Expr,
    pub disj: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOpKind {
    Union,
    Intersection,
    Difference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    IntLiteral(i64),
    VarRef(String),
    FieldAccess {
        base: Box<Expr>,
        field: String,
    },
    Cardinality(Box<Expr>),
    TransitiveClosure(Box<Expr>),
    Comparison {
        op: CompareOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    BinaryLogic {
        op: LogicOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Not(Box<Expr>),
    /// Multiplicity formula: `some expr` (non-empty) or `no expr` (empty).
    /// Distinct from Quantifier which has bindings and a body.
    MultFormula {
        kind: QuantKind, // Some or No
        expr: Box<Expr>,
    },
    Quantifier {
        kind: QuantKind,
        bindings: Vec<QuantBinding>,
        body: Box<Expr>,
    },
    SetOp {
        op: SetOpKind,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Product {
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDecl {
    pub name: String,
    pub mult: Multiplicity,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactDecl {
    pub name: Option<String>,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredDecl {
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub body: Vec<Expr>, // pre/post conditions combined for now
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertDecl {
    pub name: String,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunDecl {
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub return_mult: Multiplicity,
    pub return_type: String,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlloyModel {
    pub sigs: Vec<SigDecl>,
    pub facts: Vec<FactDecl>,
    pub preds: Vec<PredDecl>,
    pub funs: Vec<FunDecl>,
    pub asserts: Vec<AssertDecl>,
}
