-- oxidtr-internal.als
-- Internal implementation model: Generate, Check, Mine, Guarantee pipelines.
-- This layer describes oxidtr's own implementation structure.
-- It is the target for `oxidtr mine src/` (self-mine).

-------------------------------------------------------------------------------
-- Lexer / Token
-------------------------------------------------------------------------------

abstract sig Token {}
one sig Sig      extends Token {}
one sig Abstract extends Token {}
one sig Extends  extends Token {}
one sig One      extends Token {}
one sig Lone     extends Token {}
one sig Set      extends Token {}
one sig Seq      extends Token {}
one sig Fact     extends Token {}
one sig Pred     extends Token {}
one sig Fun      extends Token {}
one sig Assert   extends Token {}
one sig All      extends Token {}
one sig Some_    extends Token {}
one sig No       extends Token {}
one sig Not      extends Token {}
one sig And      extends Token {}
one sig Or       extends Token {}
one sig Implies  extends Token {}
one sig Iff      extends Token {}
one sig In       extends Token {}
one sig Check    extends Token {}
one sig Run      extends Token {}
one sig Disj     extends Token {}
one sig LBrace   extends Token {}
one sig RBrace   extends Token {}
one sig LBracket extends Token {}
one sig RBracket extends Token {}
one sig LParen   extends Token {}
one sig RParen   extends Token {}
one sig Colon    extends Token {}
one sig Comma    extends Token {}
one sig Dot      extends Token {}
one sig Hash     extends Token {}
one sig Caret    extends Token {}
one sig Eq       extends Token {}
one sig NotEq    extends Token {}
one sig Lt       extends Token {}
one sig Gt       extends Token {}
one sig Lte      extends Token {}
one sig Gte      extends Token {}
one sig Arrow    extends Token {}
one sig Pipe     extends Token {}
one sig Plus     extends Token {}
one sig Ampersand extends Token {}
one sig Minus    extends Token {}
one sig Ident    extends Token {}
one sig Int      extends Token {}
one sig Eof      extends Token {}

sig Lexer {
  lexerPos: one String
}

-------------------------------------------------------------------------------
-- Parser errors
-------------------------------------------------------------------------------

abstract sig ParseError {}
sig UnexpectedToken extends ParseError {}
sig UnexpectedEof   extends ParseError {}
sig InvalidSyntax   extends ParseError {}

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
  message:    one String,
  location:   one String,
  suggestion: lone String
}

sig GenerateConfig {
  target:    one String,
  outputDir: one String,
  warnings:  one WarningLevel,
  features:  set String,
  schema:    lone String
}

sig GenerateResult {
  filesWritten: set String,
  genWarnings:  set Warning
}

abstract sig GenerateError {}

sig GeneratedFile {
  filePath:    one String,
  fileContent: one String
}

-- Placeholder sigs for host-language primitive types.
-- These have no Alloy-level structure but are referenced in field declarations.
sig String {}
sig bool {}
sig i64 {}
sig usize {}

-------------------------------------------------------------------------------
-- Backend
-------------------------------------------------------------------------------

sig RustBackendConfig {
  rustFeatures: set String
}

abstract sig TsTestRunner {}
one sig Bun    extends TsTestRunner {}
one sig Vitest extends TsTestRunner {}

sig TsBackendConfig {
  testRunner: one TsTestRunner
}

sig TCField {
  tcFieldName: one String,
  tcSigName:   one String
}

sig JvmContext {
  jvmChildren:     set String,
  jvmEnumParents:  set String,
  jvmVariantNames: set String,
  jvmStructMap:    set String
}

-------------------------------------------------------------------------------
-- Check pipeline
-------------------------------------------------------------------------------

abstract sig CheckError {}
sig IoError         extends CheckError {}
sig ParseError extends CheckError {}
sig LoweringError extends CheckError {}
sig ImplNotFound    extends CheckError {}

sig CheckConfig {
  implDir: one String
}

abstract sig DiffItem {}
sig MissingStruct          extends DiffItem {}
sig ExtraStruct            extends DiffItem {}
sig MissingField           extends DiffItem {}
sig ExtraField             extends DiffItem {}
sig MultiplicityMismatch   extends DiffItem {}
sig MissingFn              extends DiffItem {}
sig ExtraFn                extends DiffItem {}
sig MissingValidation      extends DiffItem {}
sig ExtraValidation        extends DiffItem {}

sig CheckResult {
  diffs: set DiffItem
}

sig ExtractedField {
  exFieldName: one String,
  exFieldMult: one String,
  exFieldTarget: one String
}

sig ExtractedStruct {
  exStructName:   one String,
  exStructFields: set ExtractedField
}

sig ExtractedFn {
  exFnName: one String
}

sig ExtractedImpl {
  exStructs: set ExtractedStruct,
  exFns:     set ExtractedFn
}

-------------------------------------------------------------------------------
-- Mine pipeline
-------------------------------------------------------------------------------

abstract sig Confidence {}
one sig High   extends Confidence {}
one sig Medium extends Confidence {}
one sig Low    extends Confidence {}

abstract sig MinedMultiplicity {}
one sig One  extends MinedMultiplicity {}
one sig Lone extends MinedMultiplicity {}
one sig Set  extends MinedMultiplicity {}
one sig Seq  extends MinedMultiplicity {}

sig MinedField {
  minedName:   one String,
  minedMult:   one MinedMultiplicity,
  minedTarget: one String
}

sig MinedSig {
  minedSigName:    one String,
  minedFields:     set MinedField,
  minedIsAbstract: lone String,
  minedParent:     lone MinedSig
}

sig MinedFactCandidate {
  alloyText:     one String,
  confidence:    one Confidence,
  sourcePattern: one String
}

sig MinedModel {
  minedSigs:      set MinedSig,
  factCandidates: set MinedFactCandidate
}

sig MergeConflict {
  conflictSig:   one String,
  conflictField: one String,
  description:   one String
}

sig MergeResult {
  mergedModel: one MinedModel,
  conflicts:   set MergeConflict
}

-------------------------------------------------------------------------------
-- Guarantee analysis
-------------------------------------------------------------------------------

abstract sig Guarantee {}
one sig FullyByType     extends Guarantee {}
one sig PartiallyByType extends Guarantee {}
one sig RequiresTest    extends Guarantee {}

abstract sig TargetLang {}
one sig Rust       extends TargetLang {}
one sig Kotlin     extends TargetLang {}
one sig Java       extends TargetLang {}
one sig TypeScript extends TargetLang {}

-------------------------------------------------------------------------------
-- Constraint analysis
-------------------------------------------------------------------------------

abstract sig ConstraintInfo {}

sig CardinalityBound extends ConstraintInfo {
  boundSig:   one String,
  boundField: one String
}

abstract sig BoundKind {}
sig Exact   extends BoundKind {}
sig AtMost  extends BoundKind {}
sig AtLeast extends BoundKind {}

sig Presence extends ConstraintInfo {
  presenceSig:   one String,
  presenceField: one String
}

abstract sig PresenceKind {}
one sig Required extends PresenceKind {}
one sig Absent   extends PresenceKind {}

sig Membership extends ConstraintInfo {
  memberSig:   one String,
  memberField: one String
}

sig NoSelfRef extends ConstraintInfo {
  nsrSig:   one String,
  nsrField: one String
}

sig Acyclic extends ConstraintInfo {
  acyclicSig:   one String,
  acyclicField: one String
}

sig Named extends ConstraintInfo {
  namedName:        one String,
  namedDescription: one String
}

abstract sig BeanValidation {}
sig Size extends BeanValidation {
  sizeMin:  lone String,
  sizeMax:  lone String,
  sizeFact: one String
}
sig MinMax extends BeanValidation {
  mmFact: one String
}

-------------------------------------------------------------------------------
-- Internal structural facts
-------------------------------------------------------------------------------

fact NoCyclicMinedParent {
  no ms: MinedSig | ms in ms.^minedParent
}

fact MinedParentAsymmetric {
  all ms: MinedSig | all p: MinedSig |
    ms.minedParent = p implies p.minedParent = p.minedParent
}

fact MinedSigFieldsCardinality { all ms: MinedSig | #ms.minedFields = #ms.minedFields }
fact MinedModelSigsCardinality { all mm: MinedModel | #mm.minedSigs = #mm.minedSigs }
fact MinedModelFactsCardinality { all mm: MinedModel | #mm.factCandidates = #mm.factCandidates }
fact GenerateConfigFeaturesCardinality { all gc: GenerateConfig | #gc.features = #gc.features }
fact GenerateResultFilesCardinality { all gr: GenerateResult | #gr.filesWritten = #gr.filesWritten }
fact GenerateResultWarningsCardinality { all gr: GenerateResult | #gr.genWarnings = #gr.genWarnings }
fact CheckResultDiffsCardinality { all cr: CheckResult | #cr.diffs = #cr.diffs }
fact MergeResultConflictsCardinality { all mr: MergeResult | #mr.conflicts = #mr.conflicts }
fact ExtractedImplCardinality { all ei: ExtractedImpl | #ei.exStructs = #ei.exStructs }
fact ExtractedImplFnsCardinality { all ei: ExtractedImpl | #ei.exFns = #ei.exFns }
fact ExtractedStructFieldsCardinality { all es: ExtractedStruct | #es.exStructFields = #es.exStructFields }

-------------------------------------------------------------------------------
-- Safety assertions
-------------------------------------------------------------------------------

assert MineProducesModel {
  all mr: MergeResult | mr.mergedModel = mr.mergedModel
}

check MineProducesModel for 4
