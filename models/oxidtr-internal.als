-- oxidtr-internal.als
-- Internal implementation model: Generate, Check, Mine, Guarantee pipelines.
-- This layer describes oxidtr's own implementation structure.
-- It is the target for `oxidtr mine src/` (self-mine).

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

-- String is a placeholder sig for primitive string values
sig String {}

-------------------------------------------------------------------------------
-- Check pipeline
-------------------------------------------------------------------------------

sig CheckConfig {
  implDir: one String
}

abstract sig DiffItem {}
one sig MissingStruct          extends DiffItem {}
one sig ExtraStruct            extends DiffItem {}
one sig MissingField           extends DiffItem {}
one sig ExtraField             extends DiffItem {}
one sig MultiplicityMismatch   extends DiffItem {}
one sig MissingFn              extends DiffItem {}
one sig ExtraFn                extends DiffItem {}
one sig MissingValidation      extends DiffItem {}
one sig ExtraValidation        extends DiffItem {}

sig DiffItem {
  diffKind: one DiffItem,
  diffName: one String
}

sig CheckResult {
  diffs: set DiffItem
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

sig Presence extends ConstraintInfo {
  presenceSig:   one String,
  presenceField: one String
}

sig Acyclic extends ConstraintInfo {
  acyclicSig:   one String,
  acyclicField: one String
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

-------------------------------------------------------------------------------
-- Safety assertions
-------------------------------------------------------------------------------

assert MineProducesModel {
  all mr: MergeResult | mr.mergedModel = mr.mergedModel
}

check MineProducesModel for 4
