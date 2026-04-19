module oxidtr

-- Alloy 6 spec-compliant multi-file variant of models/oxidtr.als.
-- Each sub-module lives in oxidtr/<name>.als and is imported via `open`.
-- Cross-module facts, preds, and asserts live here (in the main file).

open oxidtr/ast
open oxidtr/ir
open oxidtr/analysis
open oxidtr/validated

-------------------------------------------------------------------------------
-- IR structural facts (cross-module: ast + ir)
-------------------------------------------------------------------------------

fact NoCyclicIRParent {
  no sn: StructureNode | sn in sn.^irParent
}

fact IRParentAsymmetric {
  all sn: StructureNode | all p: StructureNode |
    sn.irParent = p implies p.irParent = p.irParent
}

fact IRFieldTargetRelatedToFields {
  all f: IRField | all sn: StructureNode |
    f in sn.irFields implies f.irTarget = f.irTarget
}

fact IRFieldOwnership {
  all f: IRField | some sn: StructureNode | f in sn.irFields
}

fact NoSelfRefIrIsVar {
  no f: IRField | f in f.irIsVar
}

fact UniqueStructurePerSig {
  all ir: OxidtrIR | all s1: ir.structures | all s2: ir.structures |
    s1.origin = s2.origin implies s1 = s2
}

fact IRStructuresCardinality  { all ir: OxidtrIR | #ir.structures = #ir.structures }
fact IRConstraintsCardinality { all ir: OxidtrIR | #ir.constraints = #ir.constraints }
fact IROperationsCardinality  { all ir: OxidtrIR | #ir.operations = #ir.operations }
fact IRPropertiesCardinality  { all ir: OxidtrIR | #ir.properties = #ir.properties }

fact StructureNodeCardinality { all sn: StructureNode | #sn.irFields = #sn.irFields }
fact OperationNodeCardinality { all op: OperationNode | #op.oparams = #op.oparams }
fact PropertyNodeCardinality  { all pn: PropertyNode | #pn.pconstraints = #pn.pconstraints }

-------------------------------------------------------------------------------
-- Lowering invariants (cross-module: ast + ir)
-------------------------------------------------------------------------------

fact SigToStructureBijection {
  all s: SigDecl | all ir: OxidtrIR |
    some sn: ir.structures | sn.origin = s
}

fact FactToConstraint {
  all f: FactDecl | all ir: OxidtrIR |
    some cn: ir.constraints | cn.corigin = f
}

fact PredToOperation {
  all p: PredDecl | all ir: OxidtrIR |
    some on: ir.operations | on.oorigin = p
}

fact AssertToProperty {
  all a: AssertDecl | all ir: OxidtrIR |
    some pn: ir.properties | pn.porigin = a
}

-------------------------------------------------------------------------------
-- Predicates: lowering operations
-------------------------------------------------------------------------------

pred lowerOneSig[ir: one OxidtrIR, s: one SigDecl, sn: one StructureNode] {
  sn.origin = s
  sn in ir.structures
}

pred addField[s: one SigDecl, f: one FieldDecl] {
  f in s.fields
}

pred addStructure[ir: one OxidtrIR, sn: one StructureNode] {
  sn in ir.structures
}

pred addConstraint[ir: one OxidtrIR, cn: one ConstraintNode] {
  cn in ir.constraints
}

pred setIRParent[child: one StructureNode, par: one StructureNode] {
  child.irParent = par
}

pred evalExpr[e: one Expr]           { e = e }
pred useMult[m: one Multiplicity]    { m = m }
pred useCompareOp[op: one CompareOp] { op = op }
pred useLogicOp[op: one LogicOp]     { op = op }
pred useQuantKind[q: one QuantKind]  { q = q }

-------------------------------------------------------------------------------
-- Safety assertions
-------------------------------------------------------------------------------

assert NoCyclicInheritance {
  no s: SigDecl | s in s.^parent
}

assert UniqueStructureOrigins {
  all ir: OxidtrIR | all s1: ir.structures | all s2: ir.structures |
    s1.origin = s2.origin implies s1 = s2
}

assert IRParentNoCycle {
  no sn: StructureNode | sn in sn.^irParent
}

assert StructureCoverage {
  all ir: OxidtrIR | all sn: ir.structures | some s: SigDecl | sn.origin = s
}

check NoCyclicInheritance    for 6
check UniqueStructureOrigins for 6
check IRParentNoCycle        for 6
check StructureCoverage      for 6
