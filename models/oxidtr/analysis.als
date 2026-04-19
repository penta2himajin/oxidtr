module oxidtr/analysis

-- Constraint / anomaly / coverage analysis over the lowered IR.
-- Depends on: oxidtr/ast, oxidtr/ir.

open oxidtr/ast
open oxidtr/ir

-------------------------------------------------------------------------------
-- Constraint analysis (ConstraintInfo variants)
-------------------------------------------------------------------------------

sig FieldOrdering {
  foSig:        one StructureNode,
  foLeftField:  one FieldDecl,
  foOp:         one CompareOp,
  foRightField: one FieldDecl
}

sig Implication {
  implSig:        one StructureNode,
  implCondition:  one Expr,
  implConsequent: one Expr
}

sig Prohibition {
  prohSig:       one StructureNode,
  prohCondition: one Expr
}

sig Disjoint {
  disjSig:   one StructureNode,
  disjLeft:  one Expr,
  disjRight: one Expr
}

sig Exhaustive {
  exhSig:        one StructureNode,
  exhCategories: set Expr
}

sig ValueBound {
  vbSig:   one StructureNode,
  vbField: one FieldDecl
}

-------------------------------------------------------------------------------
-- Anomaly detection (AnomalyPattern variants)
-------------------------------------------------------------------------------

abstract sig AnomalyPattern {}

sig UnconstrainedField extends AnomalyPattern {
  ucfSig:   one StructureNode,
  ucfField: one IRField
}

sig UnboundedCollection extends AnomalyPattern {
  ubcSig:   one StructureNode,
  ubcField: one IRField
}

sig UnguardedSelfRef extends AnomalyPattern {
  usrSig:   one StructureNode,
  usrField: one IRField
}

-------------------------------------------------------------------------------
-- Fact coverage analysis
-------------------------------------------------------------------------------

sig PairwiseCoverage {
  pcSig:   one StructureNode,
  pcFactA: one ConstraintNode,
  pcFactB: one ConstraintNode
}

sig FactCoverage {
  fcPairwise:        set PairwiseCoverage,
  fcUncoveredFields: set IRField
}
