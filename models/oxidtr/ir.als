module oxidtr/ir

-- oxidtr IR (lowered from AST).
-- Depends on: oxidtr/ast.

open oxidtr/ast

sig StructureNode {
  origin:     one SigDecl,
  irIsVarSig: lone StructureNode,
  irFields:   set IRField,
  irParent:   lone StructureNode
}

sig IRField {
  irIsVar:  lone IRField,
  irMult:   one Multiplicity,
  irTarget: one StructureNode
}

sig ConstraintNode {
  corigin: one FactDecl,
  cexpr:   one Expr
}

sig IRReturnType {
  retMult: one Multiplicity
}

sig OperationNode {
  oorigin:      one PredDecl,
  oreceiverSig: lone StructureNode,
  oparams:      set IRParam,
  oreturnType:  lone IRReturnType
}

abstract sig LoweringError {}
sig InvalidReference extends LoweringError {}

sig IRParam {
  ipMult: one Multiplicity,
  ipType: one StructureNode
}

sig PropertyNode {
  porigin:      one AssertDecl,
  pconstraints: set ConstraintNode
}

sig OxidtrIR {
  source:      one AlloyModel,
  structures:  set StructureNode,
  constraints: set ConstraintNode,
  operations:  set OperationNode,
  properties:  set PropertyNode
}

fun OxidtrIR.origin: one AlloyModel {
  this.source
}
