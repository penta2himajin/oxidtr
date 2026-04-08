use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::backend::typescript::TsTestRunner;
use std::path::Path;

fn test_config(dir: &str) -> GenerateConfig {
    GenerateConfig {
        target: "rust".to_string(),
        output_dir: dir.to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    }
}

fn write_model(dir: &Path, name: &str, content: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn generate_pipeline_writes_files() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        sig User { role: one Role }
        sig Role {}
        fact UserHasRole { all u: User | u.role = u.role }
        pred changeRole[u: one User, r: one Role] { u.role = r }
        assert RoleExists { all u: User | u.role = u.role }
    "#);

    let result = generate::run(&model_path, &test_config(out_dir.to_str().unwrap())).unwrap();

    assert!(result.files_written.iter().any(|f| f.contains("models.rs")));
    assert!(result.files_written.iter().any(|f| f.contains("operations.rs")));
    assert!(result.files_written.iter().any(|f| f.contains("tests.rs")));

    assert!(out_dir.join("models.rs").exists());
    assert!(out_dir.join("operations.rs").exists());
    assert!(out_dir.join("tests.rs").exists());

    let models = std::fs::read_to_string(out_dir.join("models.rs")).unwrap();
    assert!(models.contains("pub struct User"));
    assert!(models.contains("pub struct Role") || models.contains("pub enum Role"));

    let ops = std::fs::read_to_string(out_dir.join("operations.rs")).unwrap();
    assert!(ops.contains("fn change_role"));
}

#[test]
fn generate_self_model_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let result = generate::run("models/oxidtr.als", &test_config(out_dir.to_str().unwrap())).unwrap();
    assert!(!result.files_written.is_empty());

    // oxidtr.als uses module declarations → modular layout
    let sig_decl = std::fs::read_to_string(out_dir.join("ast/sig_decl.rs")).unwrap();
    assert!(sig_decl.contains("pub struct SigDecl"));

    let oxidtr_ir = std::fs::read_to_string(out_dir.join("ir/oxidtr_i_r.rs")).unwrap();
    assert!(oxidtr_ir.contains("pub struct OxidtrIR"));

    let mult = std::fs::read_to_string(out_dir.join("ast/multiplicity.rs")).unwrap();
    assert!(mult.contains("pub enum Multiplicity"));

    // Verify lib.rs with module declarations
    let lib_rs = std::fs::read_to_string(out_dir.join("lib.rs")).unwrap();
    assert!(lib_rs.contains("pub mod ast;"));
    assert!(lib_rs.contains("pub mod ir;"));
}

#[test]
fn generate_detects_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        sig Node { next: lone Node }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };

    let result = generate::run(&model_path, &config).unwrap();

    assert!(
        result.warnings.iter().any(|w| matches!(w.kind, generate::WarningKind::UnconstrainedSelfRef)),
        "expected UNCONSTRAINED_SELF_REF warning"
    );
}

#[test]
fn generate_warnings_error_level_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        sig Node { next: lone Node }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Error,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };

    let result = generate::run(&model_path, &config);
    assert!(result.is_err(), "expected error with --warnings=error and warnings present");
}

#[test]
fn generate_detects_missing_inverse() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        sig User { group: lone Group }
        sig Group { members: set User }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };

    let result = generate::run(&model_path, &config).unwrap();
    assert!(
        result.warnings.iter().any(|w| matches!(w.kind, WarningKind::MissingInverse)),
        "expected MISSING_INVERSE warning for User.group <-> Group.members, got: {:?}",
        result.warnings.iter().map(|w| format!("{:?}", w.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn generate_no_missing_inverse_when_constrained() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        sig User { group: lone Group }
        sig Group { members: set User }
        fact InverseHolds { all u: User | u.group = u.group implies u.group = u.group }
        fact Members { all g: Group | g.members = g.members implies g.members = g.members }
        fact Link { all u: User | all g: Group | u.group = u.group implies g.members = g.members }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };

    let result = generate::run(&model_path, &config).unwrap();
    assert!(
        !result.warnings.iter().any(|w| matches!(w.kind, WarningKind::MissingInverse)),
        "should not warn when inverse is constrained"
    );
}

#[test]
fn generate_detects_unconstrained_transitivity() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        sig Node { parent: lone Node }
        assert Acyclic { all n: Node | n.^parent = n.^parent }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };

    let result = generate::run(&model_path, &config).unwrap();
    assert!(
        result.warnings.iter().any(|w| matches!(w.kind, WarningKind::UnconstrainedTransitivity)),
        "expected UNCONSTRAINED_TRANSITIVITY warning, got: {:?}",
        result.warnings.iter().map(|w| format!("{:?}", w.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn generate_no_unconstrained_transitivity_when_fact_exists() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        sig Node { parent: lone Node }
        fact NoSelfParent { all n: Node | n.parent = n.parent }
        assert Acyclic { all n: Node | n.^parent = n.^parent }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };

    let result = generate::run(&model_path, &config).unwrap();
    assert!(
        !result.warnings.iter().any(|w| matches!(w.kind, WarningKind::UnconstrainedTransitivity)),
        "should not warn when transitivity is constrained by a fact"
    );
}

// ── UNHANDLED_RESPONSE_PATTERN / MISSING_ERROR_PROPAGATION ───────────────────

#[test]
fn generate_detects_unhandled_response_pattern() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        abstract sig Response {}
        sig OkResponse extends Response {}
        sig TimeoutResponse extends Response {}
        pred handleOk[r: one OkResponse] { r = r }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(&model_path, &config).unwrap();

    assert!(
        result.warnings.iter().any(|w| matches!(w.kind, WarningKind::UnhandledResponsePattern)),
        "expected UNHANDLED_RESPONSE_PATTERN for TimeoutResponse, got: {:?}",
        result.warnings.iter().map(|w| format!("{:?}", w.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn generate_detects_missing_error_propagation() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        abstract sig Response {}
        sig OkResponse extends Response {}
        sig ErrorResponse extends Response {}
        pred handleOk[r: one OkResponse] { r = r }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(&model_path, &config).unwrap();

    assert!(
        result.warnings.iter().any(|w| matches!(w.kind, WarningKind::MissingErrorPropagation)),
        "expected MISSING_ERROR_PROPAGATION for ErrorResponse, got: {:?}",
        result.warnings.iter().map(|w| format!("{:?}", w.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn generate_no_unhandled_when_all_variants_have_preds() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        abstract sig Response {}
        sig OkResponse extends Response {}
        sig ErrorResponse extends Response {}
        pred handleOk[r: one OkResponse] { r = r }
        pred handleError[r: one ErrorResponse] { r = r }
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(&model_path, &config).unwrap();

    assert!(
        !result.warnings.iter().any(|w| matches!(
            w.kind,
            WarningKind::UnhandledResponsePattern | WarningKind::MissingErrorPropagation
        )),
        "should not warn when all variants handled, got: {:?}",
        result.warnings.iter().map(|w| format!("{:?}", w.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn generate_no_unhandled_for_single_child_abstract() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
use oxidtr::backend::typescript::TsTestRunner;

    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(tmp.path(), "model.als", r#"
        abstract sig Base {}
        sig Concrete extends Base {}
    "#);

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(&model_path, &config).unwrap();

    assert!(
        !result.warnings.iter().any(|w| matches!(
            w.kind,
            WarningKind::UnhandledResponsePattern | WarningKind::MissingErrorPropagation
        )),
        "single-child abstract should not trigger response pattern warnings"
    );
}
