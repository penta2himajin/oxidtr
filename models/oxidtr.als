-- oxidtr-domain.als
-- Domain model: Alloy AST and oxidtr IR.
-- This is the layer that `oxidtr generate` targets.

-------------------------------------------------------------------------------
-- Alloy AST (parser output)
-------------------------------------------------------------------------------

module ast

abstract sig Multiplicity {}
one sig One  extends Multiplicity {}
one sig Lone extends Multiplicity {}
one sig Set  extends Multiplicity {}
one sig Seq  extends Multiplicity {}

sig FieldDecl {
  fname:  one SigDecl,
  isVar:  lone FieldDecl, -- Alloy 6: mutable field marker (self-ref = true)
  mult:   one Multiplicity,
  target: one SigDecl
}

sig SigDecl {
  isAbstract:    lone SigDecl,
  isVarSig:      lone SigDecl,  -- Alloy 6: var sig (mutable atom set)
  sigMultiplicity: one SigMultiplicity,
  parent:        lone SigDecl,
  fields:        set FieldDecl
}

abstract sig Expr {}

sig VarRef      extends Expr {}
sig FieldAccess extends Expr {
  base:         one Expr,
  accessTarget: one SigDecl
}
sig Cardinality extends Expr { inner: one Expr }
sig IntLiteral extends Expr {}
sig TransitiveClosure extends Expr { tcInner: one Expr }
sig SetOp extends Expr {
  setOpKind:  one SetOpKind,
  setOpLeft:  one Expr,
  setOpRight: one Expr
}
sig Product extends Expr {
  prodLeft:  one Expr,
  prodRight: one Expr
}
sig MultFormula extends Expr {
  mfKind: one QuantKind,
  mfExpr: one Expr
}
sig Prime extends Expr { primeInner: one Expr }
sig TemporalUnary extends Expr {
  tuOp: one TemporalUnaryOp,
  tuExpr: one Expr
}

abstract sig TemporalUnaryOp {}
one sig Always       extends TemporalUnaryOp {}
one sig Eventually   extends TemporalUnaryOp {}
one sig After        extends TemporalUnaryOp {}
one sig Historically extends TemporalUnaryOp {}
one sig Once         extends TemporalUnaryOp {}
one sig Before       extends TemporalUnaryOp {}

sig TemporalBinary extends Expr {
  tbOp: one TemporalBinaryOp,
  tbLeft: one Expr,
  tbRight: one Expr
}

abstract sig TemporalBinaryOp {}
one sig Until     extends TemporalBinaryOp {}
one sig Since     extends TemporalBinaryOp {}
one sig Release   extends TemporalBinaryOp {}
one sig Triggered extends TemporalBinaryOp {}

sig FunApp extends Expr {
  receiver: lone Expr,
  funArgs: seq Expr
}

abstract sig TemporalKind {}
one sig Invariant    extends TemporalKind {}
one sig Liveness     extends TemporalKind {}
one sig PastInvariant extends TemporalKind {}
one sig PastLiveness extends TemporalKind {}
one sig Step         extends TemporalKind {}
one sig Binary       extends TemporalKind {}

abstract sig CompareOp {}
one sig In    extends CompareOp {}
one sig Eq    extends CompareOp {}
one sig NotEq extends CompareOp {}
one sig Lt    extends CompareOp {}
one sig Gt    extends CompareOp {}
one sig Lte   extends CompareOp {}
one sig Gte   extends CompareOp {}

sig Comparison extends Expr {
  cop:    one CompareOp,
  cleft:  one Expr,
  cright: one Expr
}

abstract sig LogicOp {}
one sig And     extends LogicOp {}
one sig Or      extends LogicOp {}
one sig Implies extends LogicOp {}
one sig Iff     extends LogicOp {}

sig BinaryLogic extends Expr {
  lop:    one LogicOp,
  lleft:  one Expr,
  lright: one Expr
}

sig Not extends Expr { notInner: one Expr }

abstract sig QuantKind {}
one sig All  extends QuantKind {}
one sig Some extends QuantKind {}
one sig No   extends QuantKind {}

abstract sig SetOpKind {}
one sig Union        extends SetOpKind {}
one sig Intersection extends SetOpKind {}
one sig Difference   extends SetOpKind {}

abstract sig SigMultiplicity {}
one sig Default extends SigMultiplicity {}

sig QuantBinding {
  qbDomain: one Expr
}

sig Quantifier extends Expr {
  qkind:    one QuantKind,
  bindings: set QuantBinding,
  qbody:    one Expr
}

sig FactDecl   { factBody:   one Expr }
sig AssertDecl { assertBody: one Expr }

sig PredDecl {
  predParams: set ParamDecl,
  predBody:   set Expr
}

sig FunDecl {
  receiverSig:   lone SigDecl,   -- derived field: fun Sig.name syntax
  funParams:     set ParamDecl,
  funReturnMult: one Multiplicity,
  funBody:       one Expr
}

sig ParamDecl {
  paramMult: one Multiplicity,
  paramType: one SigDecl
}

sig AlloyModel {
  sigs:    set SigDecl,
  facts:   set FactDecl,
  preds:   set PredDecl,
  funs:    set FunDecl,
  asserts: set AssertDecl
}

-------------------------------------------------------------------------------
-- AST structural facts
-------------------------------------------------------------------------------

fact NoCyclicParent {
  no s: SigDecl | s in s.^parent
}

fact ParentAsymmetric {
  all s: SigDecl | all p: SigDecl |
    s.parent = p implies p.isAbstract = p.isAbstract
}

fact NoSelfAbstract {
  no s: SigDecl | s in s.isAbstract
}

fact NoSelfRefIsVar {
  no f: FieldDecl | f in f.isVar
}

fact FieldOwnershipBidirectional {
  all f: FieldDecl | all s: SigDecl | f in s.fields implies f.fname = s
}

fact FieldTargetRelatedToFields {
  all f: FieldDecl | all s: SigDecl |
    f in s.fields implies f.target = f.target
}

fact SigFieldCount { all s: SigDecl | #s.fields = #s.fields }

-------------------------------------------------------------------------------
-- Cardinality bounds
-------------------------------------------------------------------------------

fact AlloyModelSigsCardinality   { all m: AlloyModel | #m.sigs = #m.sigs }
fact AlloyModelFactsCardinality  { all m: AlloyModel | #m.facts = #m.facts }
fact AlloyModelPredsCardinality  { all m: AlloyModel | #m.preds = #m.preds }
fact AlloyModelAssertsCardinality { all m: AlloyModel | #m.asserts = #m.asserts }
fact PredParamsCardinality { all p: PredDecl | #p.predParams = #p.predParams }
fact PredBodyCardinality   { all p: PredDecl | #p.predBody = #p.predBody }
fact QuantifierBindingsCardinality { all q: Quantifier | #q.bindings = #q.bindings }
fact AlloyModelFunsCardinality { all m: AlloyModel | #m.funs = #m.funs }
fact FunParamsCardinality  { all f: FunDecl | #f.funParams = #f.funParams }

-------------------------------------------------------------------------------
-- oxidtr IR (lowered from AST)
-------------------------------------------------------------------------------

module ir

sig StructureNode {
  origin:    one SigDecl,
  irIsVarSig: lone StructureNode,  -- Alloy 6: var sig marker
  irFields:  set IRField,
  irParent:  lone StructureNode
}

sig IRField {
  irIsVar:  lone IRField, -- Alloy 6: mutable field marker
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
  oreceiverSig: lone StructureNode,  -- derived field receiver
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

-- Derived field: the source model that this IR was lowered from
fun OxidtrIR.origin: one AlloyModel {
  this.source
}

-------------------------------------------------------------------------------
-- Constraint analysis (ConstraintInfo variants)
-------------------------------------------------------------------------------

module analysis

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
  vbSig:       one StructureNode,
  vbField:     one FieldDecl,
  vbBound:     one BoundKind
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
  fcPairwise:         set PairwiseCoverage,
  fcUncoveredFields:  set IRField
}

-------------------------------------------------------------------------------
-- Validated newtypes (Rust TryFrom wrappers for constrained sigs)
-------------------------------------------------------------------------------

module validated

sig ValidatedSigDecl       { vSigDecl:       one SigDecl }
sig ValidatedFieldDecl     { vFieldDecl:     one FieldDecl }
sig ValidatedAlloyModel    { vAlloyModel:    one AlloyModel }
sig ValidatedFactDecl      { vFactDecl:      one FactDecl }
sig ValidatedAssertDecl    { vAssertDecl:    one AssertDecl }
sig ValidatedPredDecl      { vPredDecl:      one PredDecl }
sig ValidatedFunDecl       { vFunDecl:       one FunDecl }
sig ValidatedIRField       { vIRField:       one IRField }
sig ValidatedStructureNode { vStructureNode: one StructureNode }
sig ValidatedOperationNode { vOperationNode: one OperationNode }
sig ValidatedPropertyNode  { vPropertyNode:  one PropertyNode }
sig ValidatedOxidtrIR      { vOxidtrIR:      one OxidtrIR }

-------------------------------------------------------------------------------
-- IR structural facts
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

fact StructureNodeCardinality {
  all sn: StructureNode | #sn.irFields = #sn.irFields
}

fact OperationNodeCardinality {
  all op: OperationNode | #op.oparams = #op.oparams
}

fact PropertyNodeCardinality {
  all pn: PropertyNode | #pn.pconstraints = #pn.pconstraints
}

-------------------------------------------------------------------------------
-- Lowering invariants
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

pred evalExpr[e: one Expr]         { e = e }
pred useMult[m: one Multiplicity]   { m = m }
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

check NoCyclicInheritance   for 6
check UniqueStructureOrigins for 6
check IRParentNoCycle        for 6
check StructureCoverage      for 6
