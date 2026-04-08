/// Tests for the check command.
/// TDD: these tests define expected behavior before full implementation.

use oxidtr::check::{self, CheckConfig, ExtractedImpl, ExtractedStruct, ExtractedField, ExtractedFn};
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
            intersection_of: vec![],
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
        vec![StructureNode { name: "User".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![] }],
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
            intersection_of: vec![],
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
        vec![StructureNode { name: "User".into(), is_enum: false, is_var: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![], intersection_of: vec![] }],
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
            intersection_of: vec![],
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
        vec![OperationNode { name: "add_user".into(), receiver_sig: None, params: vec![], return_type: None, body: vec![] }],
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

// ── Alloy 6: temporal constraint checking ──────────────────────────────────

#[test]
fn check_detects_missing_transition_test() {
    // A fact with prime operator should require a transition_ test
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode {
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
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
    assert!(diffs.iter().any(|d| matches!(d,
        DiffItem::MissingTemporalTest { fact_name, expected_kind }
        if fact_name == "StateUpdate" && expected_kind == "transition"
    )), "should detect missing transition test: {diffs:?}");
}

#[test]
fn check_passes_when_transition_test_present() {
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode {
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
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should not report missing temporal test: {diffs:?}");
}

#[test]
fn check_detects_missing_invariant_test_for_temporal_without_prime() {
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode {
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
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
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
        constraints: vec![ConstraintNode {
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
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should accept space-separated invariant test name: {diffs:?}");
}

#[test]
fn check_accepts_space_separated_temporal_binary_test_name() {
    // TS/Kotlin generate `it('temporal FlagUntilLarge', ...)` for binary temporal
    use oxidtr::parser::ast::TemporalBinaryOp;
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode {
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
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should accept space-separated temporal binary test name: {diffs:?}");
}

#[test]
fn check_accepts_space_separated_liveness_test_name() {
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode {
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
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
    assert!(!diffs.iter().any(|d| matches!(d, DiffItem::MissingTemporalTest { .. })),
        "should accept space-separated liveness test name: {diffs:?}");
}

#[test]
fn check_accepts_space_separated_transition_test_name() {
    // TS generates `it('transition StateUpdate', ...)` for prime constraints
    let ir = OxidtrIR {
        structures: vec![],
        constraints: vec![ConstraintNode {
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
    let diffs = differ::diff_identity_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
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
        }],
    };
    let sources = vec!["fn some_other_test() {}".to_string()];
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
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
        }],
    };
    let sources = vec!["fn no_self_loop() { assert!(true); }".to_string()];
    let diffs = differ::diff_with_validation(&ir, &ExtractedImpl { structs: vec![], fns: vec![] }, &sources);
    assert!(!diffs.iter().any(|d| matches!(d,
        DiffItem::MissingAssert { .. }
    )), "should not flag present assert test: {diffs:?}");
}
