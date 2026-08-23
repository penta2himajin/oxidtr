/// Tests for the check command.
/// TDD: these tests define expected behavior before full implementation.

use oxidtr::check::{self, CheckConfig, ExtractedImpl, ExtractedStruct, ExtractedField};
use oxidtr::check::differ::{self, DiffItem};
use oxidtr::ir::nodes::{OxidtrIR, StructureNode, ConstraintNode, IRField, OperationNode};
use oxidtr::parser::ast::{self, Multiplicity, SigMultiplicity, Expr};

// ── differ ────────────────────────────────────────────────────────────────────

fn make_ir(structs: Vec<StructureNode>, ops: Vec<OperationNode>) -> OxidtrIR {
    OxidtrIR {
        structures: structs,
        constraints: vec![],
        operations: ops,
        properties: vec![],
    }
}

#[test]
fn differ_no_diff_when_in_sync() {
    use oxidtr::check::{ExtractedImpl, ExtractedStruct};
    let ir = make_ir(
        vec![StructureNode {
            name: "User".into(),
            is_enum: false,
            is_var: false,
            sig_multiplicity: SigMultiplicity::Default,
            parent: None,
            fields: vec![IRField {
                name: "name".into(),
                is_var: false,
                mult: Multiplicity::One,
                target: "String".into(),
                value_type: None, raw_union_type: None }],
            intersection_of: vec![], module: None,
        }],
        vec![],
    );
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct {
            name: "User".into(),
            is_enum: false,
            is_var: false,
            fields: vec![ExtractedField {
                name: "name".into(),
                mult: Multiplicity::One,
                target: "String".into(),
                is_var: false,
            }],
        }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.is_empty(), "expected no diffs, got: {diffs:?}");
}

#[test]
fn differ_missing_struct() {
    use oxidtr::check::ExtractedImpl;
    let ir = make_ir(
        vec![StructureNode { name: "User".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![], module: None }],
        vec![],
    );
    let extracted = ExtractedImpl { structs: vec![], fns: vec![] };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MissingStruct { name: "User".into() }));
}

#[test]
fn differ_extra_struct() {
    use oxidtr::check::{ExtractedImpl, ExtractedStruct};
    let ir = make_ir(vec![], vec![]);
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct { name: "Ghost".into(), is_enum: false, is_var: false, fields: vec![] }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::ExtraStruct { name: "Ghost".into() }));
}

#[test]
fn differ_missing_field() {
    use oxidtr::check::{ExtractedImpl, ExtractedStruct};
    let ir = make_ir(
        vec![StructureNode {
            name: "User".into(),
            is_enum: false,
            is_var: false,
            sig_multiplicity: SigMultiplicity::Default,
            parent: None,
            fields: vec![IRField { name: "email".into(), is_var: false, mult: Multiplicity::One, target: "String".into(), value_type: None, raw_union_type: None }],
            intersection_of: vec![], module: None,
        }],
        vec![],
    );
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct { name: "User".into(), is_enum: false, is_var: false, fields: vec![] }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MissingField {
        struct_name: "User".into(),
        field_name: "email".into(),
    }));
}

#[test]
fn differ_extra_field() {
    use oxidtr::check::{ExtractedImpl, ExtractedStruct, ExtractedField};
    let ir = make_ir(
        vec![StructureNode { name: "User".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![], module: None }],
        vec![],
    );
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct {
            name: "User".into(),
            is_enum: false,
            is_var: false,
            fields: vec![ExtractedField { name: "phantom".into(), mult: Multiplicity::One, target: "String".into(), is_var: false }],
        }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::ExtraField {
        struct_name: "User".into(),
        field_name: "phantom".into(),
    }));
}

#[test]
fn differ_multiplicity_mismatch() {
    use oxidtr::check::{ExtractedImpl, ExtractedStruct, ExtractedField};
    let ir = make_ir(
        vec![StructureNode {
            name: "User".into(),
            is_enum: false,
            is_var: false,
            sig_multiplicity: SigMultiplicity::Default,
            parent: None,
            fields: vec![IRField { name: "manager".into(), is_var: false, mult: Multiplicity::Lone, target: "User".into(), value_type: None, raw_union_type: None }],
            intersection_of: vec![], module: None,
        }],
        vec![],
    );
    // impl has One instead of Lone
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct {
            name: "User".into(),
            is_enum: false,
            is_var: false,
            fields: vec![ExtractedField { name: "manager".into(), mult: Multiplicity::One, target: "User".into(), is_var: false }],
        }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MultiplicityMismatch {
        struct_name: "User".into(),
        field_name: "manager".into(),
        expected: Multiplicity::Lone,
        actual: Multiplicity::One,
    }));
}

#[test]
fn differ_missing_fn() {
    use oxidtr::check::ExtractedImpl;
    let ir = make_ir(
        vec![],
        vec![OperationNode { name: "add_user".into(), receiver_sig: None, params: vec![], return_type: None, body: vec![], module: None }],
    );
    let extracted = ExtractedImpl { structs: vec![], fns: vec![] };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MissingFn { name: "add_user".into() }));
}

#[test]
fn differ_extra_fn() {
    use oxidtr::check::{ExtractedImpl, ExtractedFn};
    let ir = make_ir(vec![], vec![]);
    let extracted = ExtractedImpl {
        structs: vec![],
        fns: vec![ExtractedFn { name: "orphan_fn".into() }],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::ExtraFn { name: "orphan_fn".into() }));
}

// ── integration: check::run ───────────────────────────────────────────────────

#[test]
fn check_run_in_sync() {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("src");
    fs::create_dir_all(&impl_dir).unwrap();

    fs::write(&model_path, r#"
sig User {
    manager: lone User
}
pred add_user[u: User] {}
"#).unwrap();

    fs::write(impl_dir.join("models.rs"), r#"
pub struct User {
    pub manager: Option<User>,
}
"#).unwrap();

    fs::write(impl_dir.join("operations.rs"), r#"
pub fn add_user(u: &User) -> Result<(), String> { todo!() }
"#).unwrap();

    let result = check::run(
        model_path.to_str().unwrap(),
        &CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() },
    ).unwrap();

    assert!(result.is_ok(), "expected no diffs, got: {:?}", result.diffs);
}

#[test]
fn check_no_drift_on_own_enum_variant_output() {
    // Regression #64: an abstract sig's field is folded into every enum
    // variant during generation. check must fold those variant fields back
    // to the parent, so its own unmodified output reports 0 diffs.
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("gen");

    fs::write(&model_path, r#"
sig Money { amount: one Int }
abstract sig Transaction { value: one Money }
sig Deposit extends Transaction {}
sig Withdrawal extends Transaction {}
"#).unwrap();

    oxidtr::generate::run(
        model_path.to_str().unwrap(),
        &oxidtr::generate::GenerateConfig::new("rust", impl_dir.to_str().unwrap()),
    ).expect("generate should succeed");

    let result = check::run(
        model_path.to_str().unwrap(),
        &CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() },
    ).unwrap();

    assert!(result.is_ok(),
        "check on own generate output should report 0 diffs, got: {:?}", result.diffs);
}

#[test]
fn check_run_detects_missing_struct() {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("src");
    fs::create_dir_all(&impl_dir).unwrap();

    fs::write(&model_path, r#"sig User {} sig Group {}"#).unwrap();
    // Group is missing from impl
    fs::write(impl_dir.join("models.rs"), r#"pub struct User {}"#).unwrap();

    let result = check::run(
        model_path.to_str().unwrap(),
        &CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() },
    ).unwrap();

    assert!(!result.is_ok());
    assert!(result.diffs.iter().any(|d| matches!(
        d, DiffItem::MissingStruct { name } if name == "Group"
    )));
}

#[test]
fn check_run_missing_models_rs_is_error() {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("src");
    fs::create_dir_all(&impl_dir).unwrap();
    fs::write(&model_path, "sig User {}").unwrap();
    // models.rs not created

    let err = check::run(
        model_path.to_str().unwrap(),
        &CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() },
    );
    assert!(matches!(err, Err(check::CheckError::ImplNotFound(_))));
}

// ── Native type aliases should not appear as MISSING_STRUCT ──────────────────

#[test]
fn differ_skips_native_type_aliases() {
    // If the model contains sigs Str, Int, Float, Bool (native type aliases),
    // the differ should NOT report them as MISSING_STRUCT because backends
    // map them to language primitives instead of emitting struct definitions.
    let ir = make_ir(
        vec![
            StructureNode { name: "User".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![
                IRField { name: "name".into(), is_var: false, mult: Multiplicity::One, target: "Str".into(), value_type: None, raw_union_type: None },
            ], intersection_of: vec![], module: None },
            StructureNode { name: "Str".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![], module: None },
            StructureNode { name: "Int".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![], module: None },
            StructureNode { name: "Float".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![], module: None },
            StructureNode { name: "Bool".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![], module: None },
        ],
        vec![],
    );
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct {
            name: "User".into(),
            is_enum: false,
            is_var: false,
            fields: vec![ExtractedField { name: "name".into(), mult: Multiplicity::One, target: "String".into(), is_var: false }],
        }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    // Should not contain MissingStruct for any native type alias
    let native_missing: Vec<_> = diffs.iter().filter(|d| matches!(d,
        DiffItem::MissingStruct { name } if matches!(name.as_str(), "Str" | "Int" | "Float" | "Bool")
    )).collect();
    assert!(native_missing.is_empty(),
        "native type aliases should not be reported as MISSING_STRUCT: {native_missing:?}");
    // Should not contain ExtraStruct for User either
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingStruct { name } if name == "User")),
        "User should not be missing: {diffs:?}");
}

// ── Alloy 6: temporal constraint checking ──────────────────────────────────

#[test]
fn check_detects_missing_transition_test() {
    // A fact with prime operator should require a transition_ test
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode { module: None,
            name: Some("StateUpdate".to_string()),
            expr: Expr::TemporalUnary {
                op: ast::TemporalUnaryOp::Always,
                expr: Box::new(Expr::Quantifier {
                    kind: ast::QuantKind::All,
                    bindings: vec![ast::QuantBinding {
                        vars: vec!["s".to_string()],
                        domain: Expr::VarRef("S".to_string()),
                        disj: false,
                    }],
                    body: Box::new(Expr::Comparison {
                        op: ast::CompareOp::Eq,
                        left: Box::new(Expr::Prime(Box::new(Expr::FieldAccess {
                            base: Box::new(Expr::VarRef("s".to_string())),
                            field: "x".to_string(),
                        }))),
                        right: Box::new(Expr::FieldAccess {
                            base: Box::new(Expr::VarRef("s".to_string())),
                            field: "x".to_string(),
                        }),
                    }),
                }),
            },
        }],
        operations: vec![],
        properties: vec![],
    };
    // Source has the fact name but NOT the transition_ prefixed test
    let sources = vec!["fn test_state_update() { /* StateUpdate */ }".to_string()];
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(diffs.iter().any(|d| matches!(d,
        DiffItem::MissingTemporalTest { fact_name, expected_kind }
        if fact_name == "StateUpdate" && expected_kind == "transition"
    )), "should detect missing transition test: {diffs:?}");
}

#[test]
fn check_passes_when_transition_test_present() {
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode { module: None,
            name: Some("StateUpdate".to_string()),
            expr: Expr::TemporalUnary {
                op: ast::TemporalUnaryOp::Always,
                expr: Box::new(Expr::Prime(Box::new(Expr::VarRef("x".to_string())))),
            },
        }],
        operations: vec![],
        properties: vec![],
    };
    let sources = vec!["fn transition_state_update() { /* StateUpdate */ }".to_string()];
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should not report missing temporal test: {diffs:?}");
}

#[test]
fn check_detects_missing_invariant_test_for_temporal_without_prime() {
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode { module: None,
            name: Some("AlwaysPositive".to_string()),
            expr: Expr::TemporalUnary {
                op: ast::TemporalUnaryOp::Always,
                expr: Box::new(Expr::Comparison {
                    op: ast::CompareOp::Gte,
                    left: Box::new(Expr::VarRef("x".to_string())),
                    right: Box::new(Expr::IntLiteral(0)),
                }),
            },
        }],
        operations: vec![],
        properties: vec![],
    };
    let sources = vec!["fn test_always_positive() { /* AlwaysPositive */ }".to_string()];
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(diffs.iter().any(|d| matches!(d,
        DiffItem::MissingTemporalTest { fact_name, expected_kind }
        if fact_name == "AlwaysPositive" && expected_kind == "invariant"
    )), "should detect missing invariant test: {diffs:?}");
}

// ── Temporal test name: space-separated form (TS/Kotlin) ────────────────────────

#[test]
fn check_accepts_space_separated_invariant_test_name() {
    // TS/Kotlin generate `it('invariant FlagImpliesPositive', ...)` (space separator)
    // check should recognize this as matching the temporal test requirement
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode { module: None,
            name: Some("FlagImpliesPositive".to_string()),
            expr: Expr::TemporalUnary {
                op: ast::TemporalUnaryOp::Always,
                expr: Box::new(Expr::Comparison {
                    op: ast::CompareOp::Gte,
                    left: Box::new(Expr::VarRef("x".to_string())),
                    right: Box::new(Expr::IntLiteral(0)),
                }),
            },
        }],
        operations: vec![],
        properties: vec![],
    };
    // Source uses space-separated form (as TS/Kotlin backends emit)
    let sources = vec!["it('invariant FlagImpliesPositive', () => {".to_string()];
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should accept space-separated invariant test name: {diffs:?}");
}

#[test]
fn check_accepts_space_separated_temporal_binary_test_name() {
    // TS/Kotlin generate `it('temporal FlagUntilLarge', ...)` for binary temporal
    use oxidtr::parser::ast::TemporalBinaryOp;
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode { module: None,
            name: Some("FlagUntilLarge".to_string()),
            expr: Expr::TemporalBinary {
                op: TemporalBinaryOp::Until,
                left: Box::new(Expr::VarRef("x".to_string())),
                right: Box::new(Expr::VarRef("y".to_string())),
            },
        }],
        operations: vec![],
        properties: vec![],
    };
    let sources = vec!["it('temporal FlagUntilLarge', () => {".to_string()];
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should accept space-separated temporal binary test name: {diffs:?}");
}

#[test]
fn check_accepts_space_separated_liveness_test_name() {
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode { module: None,
            name: Some("WillConverge".to_string()),
            expr: Expr::TemporalUnary {
                op: ast::TemporalUnaryOp::Eventually,
                expr: Box::new(Expr::VarRef("x".to_string())),
            },
        }],
        operations: vec![],
        properties: vec![],
    };
    let sources = vec!["it('liveness WillConverge', () => {".to_string()];
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should accept space-separated liveness test name: {diffs:?}");
}

#[test]
fn check_accepts_space_separated_transition_test_name() {
    // TS generates `it('transition StateUpdate', ...)` for prime constraints
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode { module: None,
            name: Some("StateUpdate".to_string()),
            expr: Expr::TemporalUnary {
                op: ast::TemporalUnaryOp::Always,
                expr: Box::new(Expr::Prime(Box::new(Expr::VarRef("x".to_string())))),
            },
        }],
        operations: vec![],
        properties: vec![],
    };
    let sources = vec!["it('transition StateUpdate', () => {".to_string()];
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should accept space-separated transition test name: {diffs:?}");
}

// ── Assert check ────────────────────────────────────────────────────────────────

#[test]
fn missing_assert_detected() {
    use oxidtr::ir::nodes::PropertyNode;
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![],
        operations: vec![],
        properties: vec![PropertyNode {
            name: "NoSelfLoop".to_string(),
            expr: Expr::VarRef("placeholder".to_string()),
            module: None,
        }],
    };
    let sources = vec!["fn some_other_test() {}".to_string()];
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(diffs.iter().any(|d| matches!(d,
        DiffItem::MissingAssert { name } if name == "NoSelfLoop"
    )), "should detect missing assert test: {diffs:?}");
}

#[test]
fn present_assert_not_flagged() {
    use oxidtr::ir::nodes::PropertyNode;
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![],
        operations: vec![],
        properties: vec![PropertyNode {
            name: "NoSelfLoop".to_string(),
            expr: Expr::VarRef("placeholder".to_string()),
            module: None,
        }],
    };
    let sources = vec!["fn no_self_loop() { assert!(true); }".to_string()];
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources, None);
    assert!(!diffs.iter().any(|d| matches!(d,
        DiffItem::MissingAssert { .. }
    )), "should not flag present assert test: {diffs:?}");
}

// ── the manifest as the answer, not the source text (#97) ───────────────────
//
// `check` used to ask "was this fact verified?" by searching the generated
// source for the fact's name. A fact whose only trace was the comment saying
// it had been skipped satisfied that search, so the diagnostic announcing the
// dropped guarantee was itself the evidence the guarantee was kept. These
// tests pin the replacement: the backend's manifest answers, and the source
// text is consulted only for implementations that never had a manifest.

/// A model with one fact and one assert, and a directory holding an
/// implementation of it — plus whatever manifest the caller wants beside it.
fn impl_with_manifest(manifest: Option<&str>) -> (tempfile::TempDir, String, String) {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("gen");
    fs::create_dir_all(&impl_dir).unwrap();

    fs::write(&model_path, r#"
sig Node {
    rank: one Int
}
fact NoSelfLoop {
    all n: Node | n.rank = 0
}
"#).unwrap();

    fs::write(impl_dir.join("models.rs"), r#"
pub struct Node {
    pub rank: i64,
}
"#).unwrap();

    // The mention that used to pass for a proof: the fact's name appears, in a
    // comment saying nothing was generated for it.
    fs::write(impl_dir.join("tests.rs"),
        "// oxidtr: no_self_loop was skipped\n").unwrap();

    if let Some(text) = manifest {
        fs::write(impl_dir.join("coverage.txt"), text).unwrap();
    }
    (dir, model_path.to_string_lossy().into_owned(),
     impl_dir.to_string_lossy().into_owned())
}

fn run_check(model: &str, impl_dir: &str) -> Result<check::CheckResult, check::CheckError> {
    check::run(model, &CheckConfig { impl_dir: impl_dir.to_string() })
}

#[test]
fn a_declined_fact_fails_the_check_even_though_its_name_appears() {
    let (_d, model, impl_dir) = impl_with_manifest(Some(
        "declined fact NoSelfLoop -- nothing was generated for this shape\n"));
    let result = run_check(&model, &impl_dir).unwrap();
    assert!(result.diffs.iter().any(|d| matches!(d,
        DiffItem::DeclinedCoverage { name, .. } if name == "NoSelfLoop")),
        "a declined fact must be reported, got: {:?}", result.diffs);
}

#[test]
fn a_verified_fact_passes() {
    let (_d, model, impl_dir) = impl_with_manifest(Some(
        "verified fact NoSelfLoop\n"));
    let result = run_check(&model, &impl_dir).unwrap();
    assert!(!result.diffs.iter().any(|d| matches!(d,
        DiffItem::DeclinedCoverage { .. } | DiffItem::MissingValidation { .. })),
        "a verified fact must pass: {:?}", result.diffs);
}

#[test]
fn a_verified_fact_whose_code_is_gone_is_still_reported() {
    // The manifest and the source answer different questions. The manifest
    // records what the backend did, once, at generation time; it cannot know
    // the tree drifted afterwards. Deleting the tests must still be caught.
    use std::fs;
    let (_d, model, impl_dir) = impl_with_manifest(Some(
        "verified fact NoSelfLoop\n"));
    fs::remove_file(std::path::Path::new(&impl_dir).join("tests.rs")).unwrap();

    let result = run_check(&model, &impl_dir).unwrap();
    assert!(result.diffs.iter().any(|d| matches!(d,
        DiffItem::MissingValidation { fact_name } if fact_name == "NoSelfLoop")),
        "a verified fact with no code left must be reported: {:?}", result.diffs);
}

#[test]
fn a_by_type_fact_passes() {
    // `Guarantee::FullyByType` is not a gap: the type system encodes the fact,
    // which is why the guarantee budget balances across languages (#87).
    let (_d, model, impl_dir) = impl_with_manifest(Some(
        "by-type  fact NoSelfLoop\n"));
    let result = run_check(&model, &impl_dir).unwrap();
    assert!(!result.diffs.iter().any(|d| matches!(d,
        DiffItem::DeclinedCoverage { .. } | DiffItem::MissingValidation { .. })),
        "a by-type fact must pass: {:?}", result.diffs);
}

#[test]
fn a_fact_absent_from_the_manifest_is_missing_not_passing() {
    // The backend never recorded an outcome for it. That is a gap in the
    // backend, and silence must not read as success.
    let (_d, model, impl_dir) = impl_with_manifest(Some(""));
    let result = run_check(&model, &impl_dir).unwrap();
    assert!(result.diffs.iter().any(|d| matches!(d,
        DiffItem::MissingValidation { fact_name } if fact_name == "NoSelfLoop")),
        "an unrecorded fact must be reported: {:?}", result.diffs);
}

#[test]
fn an_unreadable_manifest_is_an_error_not_an_empty_one() {
    // Treating a corrupt manifest as "nothing was declined" would restore the
    // failure this whole mechanism exists to remove.
    let (_d, model, impl_dir) = impl_with_manifest(Some("this is not a manifest\n"));
    match run_check(&model, &impl_dir) {
        Err(check::CheckError::CoverageUnreadable(_)) => {}
        other => panic!("expected a coverage parse error, got: {other:?}"),
    }
}

#[test]
fn without_a_manifest_the_source_text_is_still_consulted() {
    // A hand-written implementation has no manifest and never will. For those
    // the old substring search remains the only available evidence.
    let (_d, model, impl_dir) = impl_with_manifest(None);
    let result = run_check(&model, &impl_dir).unwrap();
    assert!(!result.diffs.iter().any(|d| matches!(d,
        DiffItem::MissingValidation { .. })),
        "the fallback must still find the name in the source: {:?}", result.diffs);
}
