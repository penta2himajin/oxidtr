module oxidtr/ast

-- Alloy AST (parser output).
-- Depends on: nothing (leaf module).

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
  isAbstract:      lone SigDecl,
  isVarSig:        lone SigDecl,
  sigMultiplicity: one SigMultiplicity,
  parent:          lone SigDecl,
  fields:          set FieldDecl
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
  tuOp:   one TemporalUnaryOp,
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
  tbOp:    one TemporalBinaryOp,
  tbLeft:  one Expr,
  tbRight: one Expr
}

abstract sig TemporalBinaryOp {}
one sig Until     extends TemporalBinaryOp {}
one sig Since     extends TemporalBinaryOp {}
one sig Release   extends TemporalBinaryOp {}
one sig Triggered extends TemporalBinaryOp {}

sig FunApp extends Expr {
  receiver: lone Expr,
  funArgs:  seq Expr
}

abstract sig TemporalKind {}
one sig Invariant     extends TemporalKind {}
one sig Liveness      extends TemporalKind {}
one sig PastInvariant extends TemporalKind {}
one sig PastLiveness  extends TemporalKind {}
one sig Step          extends TemporalKind {}
one sig Binary        extends TemporalKind {}

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
  receiverSig:   lone SigDecl,
  funParams:     set ParamDecl,
  funReturnMult: one Multiplicity,
  funBody:       one Expr
}

sig ParamDecl {
  paramMult: one Multiplicity,
  paramType: one SigDecl
}

sig ImportDecl {
  importPath:  one SigDecl,
  importAlias: lone SigDecl
}

sig AlloyModel {
  moduleDecl: lone SigDecl,
  imports:    set ImportDecl,
  sigs:       set SigDecl,
  facts:      set FactDecl,
  preds:      set PredDecl,
  funs:       set FunDecl,
  asserts:    set AssertDecl
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

fact AlloyModelSigsCardinality    { all m: AlloyModel | #m.sigs = #m.sigs }
fact AlloyModelFactsCardinality   { all m: AlloyModel | #m.facts = #m.facts }
fact AlloyModelPredsCardinality   { all m: AlloyModel | #m.preds = #m.preds }
fact AlloyModelAssertsCardinality { all m: AlloyModel | #m.asserts = #m.asserts }
fact AlloyModelFunsCardinality    { all m: AlloyModel | #m.funs = #m.funs }
fact PredParamsCardinality { all p: PredDecl | #p.predParams = #p.predParams }
fact PredBodyCardinality   { all p: PredDecl | #p.predBody = #p.predBody }
fact FunParamsCardinality  { all f: FunDecl | #f.funParams = #f.funParams }
fact QuantifierBindingsCardinality { all q: Quantifier | #q.bindings = #q.bindings }
