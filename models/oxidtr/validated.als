module oxidtr/validated

-- Validated newtype wrappers for constrained sigs (Rust TryFrom targets).
-- Depends on: oxidtr/ast, oxidtr/ir.

open oxidtr/ast
open oxidtr/ir

sig ValidatedSigDecl       { vSigDecl:       one SigDecl }
-- ValidatedFactDecl / ValidatedAssertDecl removed: their only facts quantify
-- over another whole sig (`all f: FactDecl | all ir: OxidtrIR | ...`), so a
-- wrapper around one value would check them against an empty universe and
-- certify nothing. See #78.
sig ValidatedFieldDecl     { vFieldDecl:     one FieldDecl }
sig ValidatedAlloyModel    { vAlloyModel:    one AlloyModel }
sig ValidatedPredDecl      { vPredDecl:      one PredDecl }
sig ValidatedFunDecl       { vFunDecl:       one FunDecl }
sig ValidatedIRField       { vIRField:       one IRField }
sig ValidatedStructureNode { vStructureNode: one StructureNode }
sig ValidatedOperationNode { vOperationNode: one OperationNode }
sig ValidatedPropertyNode  { vPropertyNode:  one PropertyNode }
sig ValidatedOxidtrIR      { vOxidtrIR:      one OxidtrIR }
