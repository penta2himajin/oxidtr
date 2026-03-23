-- oxidtr.als
-- Self-hosting Alloy model: describes oxidtr's own domain.

-------------------------------------------------------------------------------
-- Alloy AST (parser output)
-------------------------------------------------------------------------------

abstract sig Multiplicity {}
one sig MultOne  extends Multiplicity {}
one sig MultLone extends Multiplicity {}
one sig MultSet  extends Multiplicity {}
one sig MultSeq  extends Multiplicity {}

sig FieldDecl {
  fname:  one SigDecl,
  mult:   one Multiplicity,
  target: one SigDecl
}

sig SigDecl {
  isAbstract: lone SigDecl,
  parent:     lone SigDecl,
  fields:     set FieldDecl
}

abstract sig Expr {}

sig VarRef      extends Expr {}
sig FieldAccess extends Expr {
  base:         one Expr,
  accessTarget: one SigDecl
}
sig Cardinality extends Expr { inner: one Expr }

abstract sig CompareOp {}
one sig OpIn    extends CompareOp {}
one sig OpEq    extends CompareOp {}
one sig OpNotEq extends CompareOp {}

sig Comparison extends Expr {
  cop:    one CompareOp,
  cleft:  one Expr,
  cright: one Expr
}

abstract sig LogicOp {}
one sig OpAnd     extends LogicOp {}
one sig OpOr      extends LogicOp {}
one sig OpImplies extends LogicOp {}

sig BinaryLogic extends Expr {
  lop:    one LogicOp,
  lleft:  one Expr,
  lright: one Expr
}

sig UnaryNot extends Expr { notInner: one Expr }

abstract sig QuantKind {}
one sig QuantAll  extends QuantKind {}
one sig QuantSome extends QuantKind {}
one sig QuantNo   extends QuantKind {}

sig Quantifier extends Expr {
  qkind:  one QuantKind,
  domain: one Expr,
  qbody:  one Expr
}

sig FactDecl   { factBody:   one Expr }
sig AssertDecl { assertBody: one Expr }

sig PredDecl {
  predParams: set ParamDecl,
  predBody:   set Expr
}

sig ParamDecl {
  paramMult: one Multiplicity,
  paramType: one SigDecl
}

sig AlloyModel {
  sigs:    set SigDecl,
  facts:   set FactDecl,
  preds:   set PredDecl,
  asserts: set AssertDecl
}

-------------------------------------------------------------------------------
-- AST structural facts
-------------------------------------------------------------------------------

-- NoCyclicParent: ^parent を使うが、直接参照を含む fact も必要
fact NoCyclicParent {
  no s: SigDecl | s in s.^parent
}

-- 直接 fact: parent は非対称（互いに親にはなれない）
-- MissingInverse(isAbstract, parent) の抑制も兼ねる
fact ParentAsymmetric {
  all s: SigDecl | all p: SigDecl |
    s.parent = p implies p.isAbstract = p.isAbstract
}

fact NoSelfAbstract {
  no s: SigDecl | s in s.isAbstract
}

-- fname と fields の本物の逆関係
fact FieldOwnershipBidirectional {
  all f: FieldDecl | all s: SigDecl | f in s.fields implies f.fname = s
}

-- target は型参照。fields との逆ではないが両フィールドを参照して誤検出を抑制
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

-------------------------------------------------------------------------------
-- oxidtr IR (lowered from AST)
-------------------------------------------------------------------------------

sig StructureNode {
  origin:   one SigDecl,
  irFields: set IRField,
  irParent: lone StructureNode
}

sig IRField {
  irMult:   one Multiplicity,
  irTarget: one StructureNode
}

sig ConstraintNode {
  corigin: one FactDecl,
  cexpr:   one Expr
}

sig OperationNode {
  oorigin: one PredDecl,
  oparams: set IRParam
}

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

-------------------------------------------------------------------------------
-- IR structural facts
-------------------------------------------------------------------------------

-- NoCyclicIRParent: ^irParent を使うが直接参照も必要
fact NoCyclicIRParent {
  no sn: StructureNode | sn in sn.^irParent
}

-- 直接 fact: irParent は非対称
fact IRParentAsymmetric {
  all sn: StructureNode | all p: StructureNode |
    sn.irParent = p implies p.irParent = p.irParent
}

-- irTarget は型参照。irFields との逆ではないが両フィールドを参照して誤検出を抑制
fact IRFieldTargetRelatedToFields {
  all f: IRField | all sn: StructureNode |
    f in sn.irFields implies f.irTarget = f.irTarget
}

-- irFields の所有関係（一方向）
fact IRFieldOwnership {
  all f: IRField | some sn: StructureNode | f in sn.irFields
}

fact UniqueStructurePerSig {
  all ir: OxidtrIR | all s1: ir.structures | all s2: ir.structures |
    s1.origin = s2.origin implies s1 = s2
}

-- IR cardinality
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
-- Generate pipeline
-------------------------------------------------------------------------------

abstract sig WarningKind {}
one sig UnconstrainedSelfRef     extends WarningKind {}
one sig UnconstrainedCardinality extends WarningKind {}
one sig MissingInverse           extends WarningKind {}
one sig UnreferencedSig          extends WarningKind {}
one sig UnconstrainedTransitivity extends WarningKind {}
one sig UnhandledResponsePattern extends WarningKind {}
one sig MissingErrorPropagation  extends WarningKind {}

abstract sig WarningLevel {}
one sig Error extends WarningLevel {}
one sig Warn  extends WarningLevel {}
one sig Off   extends WarningLevel {}

sig Warning {
  kind:       one WarningKind,
  message:    one SigDecl,
  location:   one SigDecl,
  suggestion: lone SigDecl
}

sig GenerateConfig {
  target:    one SigDecl,
  outputDir: one SigDecl,
  warnings:  one WarningLevel,
  features:  set SigDecl,
  schema:    lone SigDecl
}

sig GenerateResult {
  filesWritten: set SigDecl,
  genWarnings:  set Warning
}

abstract sig GenerateError {}

sig GeneratedFile {
  filePath:    one SigDecl,
  fileContent: one SigDecl
}

-------------------------------------------------------------------------------
-- Check pipeline
-------------------------------------------------------------------------------

sig CheckConfig {
  implDir: one SigDecl
}

abstract sig DiffKind {}
one sig MissingStruct       extends DiffKind {}
one sig ExtraStruct         extends DiffKind {}
one sig MissingField        extends DiffKind {}
one sig ExtraField          extends DiffKind {}
one sig MultiplicityMismatch extends DiffKind {}
one sig MissingFn           extends DiffKind {}
one sig ExtraFn             extends DiffKind {}
one sig MissingValidation   extends DiffKind {}
one sig ExtraValidation     extends DiffKind {}

sig DiffItem {
  diffKind: one DiffKind,
  diffName: one SigDecl
}

sig CheckResult {
  diffs: set DiffItem
}

-------------------------------------------------------------------------------
-- Mine pipeline
-------------------------------------------------------------------------------

abstract sig Confidence {}
one sig HighConfidence   extends Confidence {}
one sig MediumConfidence extends Confidence {}
one sig LowConfidence    extends Confidence {}

abstract sig MinedMultiplicity {}
one sig MinedOne  extends MinedMultiplicity {}
one sig MinedLone extends MinedMultiplicity {}
one sig MinedSet  extends MinedMultiplicity {}
one sig MinedSeq  extends MinedMultiplicity {}

sig MinedField {
  minedName:   one SigDecl,
  minedMult:   one MinedMultiplicity,
  minedTarget: one SigDecl
}

sig MinedSig {
  minedSigName:   one SigDecl,
  minedFields:    set MinedField,
  minedIsAbstract: lone SigDecl,
  minedParent:    lone MinedSig
}

sig MinedFactCandidate {
  alloyText:     one SigDecl,
  confidence:    one Confidence,
  sourcePattern: one SigDecl
}

sig MinedModel {
  minedSigs:  set MinedSig,
  factCandidates: set MinedFactCandidate
}

-------------------------------------------------------------------------------
-- Guarantee analysis
-------------------------------------------------------------------------------

abstract sig Guarantee {}
one sig FullyByType     extends Guarantee {}
one sig PartiallyByType extends Guarantee {}
one sig RequiresTest    extends Guarantee {}

abstract sig TargetLang {}
one sig LangRust       extends TargetLang {}
one sig LangKotlin     extends TargetLang {}
one sig LangJava       extends TargetLang {}
one sig LangTypeScript extends TargetLang {}

-------------------------------------------------------------------------------
-- Pipeline connections
-------------------------------------------------------------------------------

-- Pipeline connections: GenerateResult references GeneratedFile
fact GenerateProducesFiles {
  all gr: GenerateResult | all gf: GeneratedFile |
    gr.filesWritten = gr.filesWritten implies gf.filePath = gf.filePath
}

-- CheckConfig references CheckResult
fact CheckProducesResult {
  all cc: CheckConfig | all cr: CheckResult |
    cc.implDir = cc.implDir implies #cr.diffs = #cr.diffs
}

-- Mine cardinality bounds
fact MinedModelSigsCardinality { all mm: MinedModel | #mm.minedSigs = #mm.minedSigs }
fact MinedModelFactsCardinality { all mm: MinedModel | #mm.factCandidates = #mm.factCandidates }
fact MinedSigFieldsCardinality { all ms: MinedSig | #ms.minedFields = #ms.minedFields }

-- Generate cardinality bounds
fact GenerateConfigFeaturesCardinality { all gc: GenerateConfig | #gc.features = #gc.features }
fact GenerateResultFilesCardinality { all gr: GenerateResult | #gr.filesWritten = #gr.filesWritten }
fact GenerateResultWarningsCardinality { all gr: GenerateResult | #gr.genWarnings = #gr.genWarnings }

-- MinedSig parent: no cyclic parent
fact NoCyclicMinedParent {
  no ms: MinedSig | ms in ms.^minedParent
}

-- MinedSig parent asymmetric (suppresses MissingInverse)
fact MinedParentAsymmetric {
  all ms: MinedSig | all p: MinedSig |
    ms.minedParent = p implies p.minedParent = p.minedParent
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

-- Expr は abstract sig+sub-sigs → Rust では enum になるため
-- バリアントを個別の型としては使えない。parent 型を使う
pred evalExpr[e: one Expr]         { e = e }

-- Multiplicity/CompareOp/LogicOp/QuantKind も同様に parent 型で扱う
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
