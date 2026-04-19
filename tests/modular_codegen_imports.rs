//! Regression tests for modular Rust codegen imports when ungrouped sigs
//! coexist with module-tagged sigs.
//!
//! When an Alloy model mixes ungrouped sigs (which land in `models.rs`)
//! with sigs tagged by inline `module X` directives (which land in
//! `X/*.rs`), the generated `fixtures.rs` / `tests.rs` / `newtypes.rs`
//! / `helpers.rs` must import **both** the root `models` module *and*
//! each submodule — otherwise factory/test functions referencing
//! ungrouped sigs fail to compile.
//!
//! A prior implementation regressed this contract by only routing
//! `helpers.rs` through the `rewrite_models_import` helper while leaving
//! the three other `_modular` generators on a raw `.replace()` that
//! dropped the models import. These tests pin the contract down.

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

/// Core regression: Asterinas-shaped model (mixed ungrouped + module-tagged)
/// must produce `fixtures.rs` that imports **both** the models module
/// and the submodule.
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
        "fixtures.rs must import the models module when ungrouped sigs \
         exist — otherwise factory fns referencing those sigs fail to \
         compile. Got:\n{fixtures}"
    );
    assert!(
        fixtures.contains("use super::sub::*;"),
        "fixtures.rs must also import the submodule. Got:\n{fixtures}"
    );
}

/// tests.rs had the same bug historically — it dropped the models import
/// under the same conditions.
#[test]
fn tests_imports_both_models_and_submodule_when_mixed() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = write_model(
        tmp.path(),
        "model.als",
        r#"
        sig TopLevel {}

        module sub
        sig InSub {}

        fact TopLevelFact { all t: TopLevel | t = t }
        fact SubFact { all s: InSub | s = s }
    "#,
    );

    generate::run(&model_path, &test_config(out_dir.to_str().unwrap())).unwrap();

    // tests.rs is produced only when constraints exist; both asserts are
    // meaningful here.
    let tests_path = out_dir.join("tests.rs");
    if !tests_path.exists() {
        return;
    }
    let tests = std::fs::read_to_string(&tests_path).unwrap();
    assert!(
        tests.contains("use super::models::*;"),
        "tests.rs must keep the models import. Got:\n{tests}"
    );
    assert!(
        tests.contains("use super::sub::*;"),
        "tests.rs must also import the submodule. Got:\n{tests}"
    );
}

/// Sanity: when every sig is module-tagged, `models.rs` is not produced,
/// and fixtures must NOT import a nonexistent models module.
#[test]
fn fixtures_uses_module_only_import_when_no_ungrouped() {
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
        "fixtures.rs must not import a nonexistent models module. \
         Got:\n{fixtures}"
    );
    assert!(
        fixtures.contains("use super::sub::*;"),
        "fixtures.rs must import the submodule. Got:\n{fixtures}"
    );
}

/// End-to-end: the generated crate must actually compile. This is the
/// strongest regression guard because any future drift in import
/// emission will show up as a compilation failure, not just a string
/// mismatch.
#[test]
fn generated_crate_compiles_for_mixed_modular_model() {
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("crate");
    let src_dir = crate_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let model_path = write_model(
        tmp.path(),
        "model.als",
        r#"
        sig TopLevel {}
        sig AnotherTop {}

        module sub
        sig InSub {}

        module other
        sig InOther {}
    "#,
    );

    generate::run(&model_path, &test_config(src_dir.to_str().unwrap())).unwrap();

    // mod.rs → lib.rs to make it a library crate.
    std::fs::rename(src_dir.join("mod.rs"), src_dir.join("lib.rs")).unwrap();

    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"rt_mixed\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
         [dependencies]\n",
    )
    .unwrap();

    let status = std::process::Command::new("cargo")
        .args(["check", "--quiet", "--offline"])
        .current_dir(&crate_dir)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!(
            "generated crate failed to compile (exit {s}). \
             This almost certainly means a modular codegen file \
             dropped the `use super::models::*;` import."
        ),
        Err(e) => {
            // cargo unavailable in this environment — skip rather than
            // mark as failure.
            eprintln!("skipping compile check: {e}");
        }
    }
}
