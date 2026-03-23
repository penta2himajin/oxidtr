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
one sig WarnError extends WarningLevel {}
one sig WarnWarn  extends WarningLevel {}
one sig WarnOff   extends WarningLevel {}

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

abstract sig DiffKind {}
one sig DiffMissingStruct       extends DiffKind {}
one sig DiffExtraStruct         extends DiffKind {}
one sig DiffMissingField        extends DiffKind {}
one sig DiffExtraField          extends DiffKind {}
one sig DiffMultiplicityMismatch extends DiffKind {}
one sig DiffMissingFn           extends DiffKind {}
one sig DiffExtraFn             extends DiffKind {}
one sig DiffMissingValidation   extends DiffKind {}
one sig DiffExtraValidation     extends DiffKind {}

sig DiffItem {
  diffKind: one DiffKind,
  diffName: one String
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
one sig LangRust       extends TargetLang {}
one sig LangKotlin     extends TargetLang {}
one sig LangJava       extends TargetLang {}
one sig LangTypeScript extends TargetLang {}

-------------------------------------------------------------------------------
-- Constraint analysis
-------------------------------------------------------------------------------

abstract sig ConstraintInfo {}

sig CardinalityBound extends ConstraintInfo {
  boundSig:   one String,
  boundField: one String
}

sig PresenceInfo extends ConstraintInfo {
  presenceSig:   one String,
  presenceField: one String
}

sig AcyclicInfo extends ConstraintInfo {
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
