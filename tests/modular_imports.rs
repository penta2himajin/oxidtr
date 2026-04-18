//! Regression tests for modular code generation imports.
//!
//! When an Alloy model mixes ungrouped sigs (which land in `models.rs`) with
//! sigs tagged by inline `module X` directives (which land in `X/*.rs`),
//! the generated `fixtures.rs` / `helpers.rs` / `tests.rs` / `newtypes.rs`
//! must import **both** the root `models` module *and* each submodule.
//!
//! Previously, the `_modular` helpers replaced `use super::models::*;`
//! wholesale with module imports, dropping the `models` import and
//! producing code that fails to compile when any ungrouped sig exists.

use oxidtr::backend::typescript::TsTestRunner;
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
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
fn fixtures_imports_both_models_and_submodule_when_mixed() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(
        tmp.path(),
        "model.als",
        r#"
        sig TopLevel {}

        module sub
        sig InSub {}
    "#,
    );

    generate::run(&model_path, &test_config(out_dir.to_str().unwrap())).unwrap();

    // Both models.rs (ungrouped) and sub/ dir should exist.
    assert!(
        out_dir.join("models.rs").exists(),
        "models.rs should exist for ungrouped sigs"
    );
    assert!(
        out_dir.join("sub").exists(),
        "sub/ directory should exist for module-tagged sigs"
    );

    let fixtures = std::fs::read_to_string(out_dir.join("fixtures.rs")).unwrap();
    assert!(
        fixtures.contains("use super::models::*;"),
        "fixtures.rs must still import the models module when ungrouped \
         sigs exist — otherwise factory fns referencing those sigs fail to \
         compile. Got:\n{fixtures}"
    );
    assert!(
        fixtures.contains("use super::sub::*;"),
        "fixtures.rs must import the submodule too. Got:\n{fixtures}"
    );
}

#[test]
fn newtypes_imports_both_models_and_submodule_when_mixed() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    // Use newtype-producing sigs (sigs with a single primitive-looking field).
    // Even without newtypes, the _modular helper should preserve the models
    // import when both ungrouped and module-tagged sigs are present.
    let model_path = write_model(
        tmp.path(),
        "model.als",
        r#"
        sig Name {}
        sig User { name: one Name }

        module sub
        sig Tag {}
    "#,
    );

    generate::run(&model_path, &test_config(out_dir.to_str().unwrap())).unwrap();

    // newtypes.rs is optional — skip if not generated.
    let nt = out_dir.join("newtypes.rs");
    if !nt.exists() {
        return;
    }
    let newtypes = std::fs::read_to_string(&nt).unwrap();
    // If the file imports the models module at all, it must not have dropped it.
    if newtypes.contains("use super::") {
        assert!(
            newtypes.contains("use super::models::*;") || !newtypes.contains("use super::sub::*;"),
            "newtypes.rs must not drop the models import when submodules \
             exist. Got:\n{newtypes}"
        );
    }
}

#[test]
fn fixtures_uses_module_only_import_when_no_ungrouped() {
    // Sanity: when *every* sig is in a module, models.rs is not produced,
    // and fixtures.rs should import only the submodule (not a dangling
    // `use super::models::*;`).
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(
        tmp.path(),
        "model.als",
        r#"
        module sub
        sig Only {}
    "#,
    );

    generate::run(&model_path, &test_config(out_dir.to_str().unwrap())).unwrap();

    assert!(
        !out_dir.join("models.rs").exists(),
        "models.rs should not exist when every sig is module-tagged"
    );

    let fixtures = std::fs::read_to_string(out_dir.join("fixtures.rs")).unwrap();
    assert!(
        !fixtures.contains("use super::models::*;"),
        "fixtures.rs should not import a nonexistent models module. Got:\n{fixtures}"
    );
    assert!(
        fixtures.contains("use super::sub::*;"),
        "fixtures.rs must import the submodule. Got:\n{fixtures}"
    );
}
