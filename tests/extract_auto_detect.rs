/// Tests for mine auto-detection and directory support.

use oxidtr::extract;
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::backend::typescript::TsTestRunner;

#[test]
fn detect_lang_from_rs_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("models.rs");
    std::fs::write(&file, "pub struct Foo {}").unwrap();
    assert_eq!(extract::detect_lang(&file).as_deref(), Some("rust"));
}

#[test]
fn detect_lang_from_ts_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("models.ts");
    std::fs::write(&file, "export interface Foo {}").unwrap();
    assert_eq!(extract::detect_lang(&file).as_deref(), Some("ts"));
}

#[test]
fn detect_lang_from_kt_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("Models.kt");
    std::fs::write(&file, "data class Foo(val x: Int)").unwrap();
    assert_eq!(extract::detect_lang(&file).as_deref(), Some("kotlin"));
}

#[test]
fn detect_lang_from_java_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("Models.java");
    std::fs::write(&file, "public record Foo() {}").unwrap();
    assert_eq!(extract::detect_lang(&file).as_deref(), Some("java"));
}

#[test]
fn detect_lang_from_json_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("schemas.json");
    std::fs::write(&file, "{}").unwrap();
    assert_eq!(extract::detect_lang(&file).as_deref(), Some("schema"));
}

#[test]
fn detect_lang_from_directory_with_rs() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.rs"), "pub struct Foo {}").unwrap();
    assert_eq!(extract::detect_lang(tmp.path()).as_deref(), Some("rust"));
}

#[test]
fn detect_lang_from_directory_with_ts() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.ts"), "export interface Foo {}").unwrap();
    assert_eq!(extract::detect_lang(tmp.path()).as_deref(), Some("ts"));
}

#[test]
fn detect_lang_from_directory_with_schema() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("schemas.json"), "{}").unwrap();
    assert_eq!(extract::detect_lang(tmp.path()).as_deref(), Some("schema"));
}

#[test]
fn mine_run_single_file_auto_detect() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("models.rs");
    std::fs::write(&file, "pub struct User { pub name: String }").unwrap();

    let mined = extract::run(file.to_str().unwrap(), None).unwrap();
    assert_eq!(mined.sigs.len(), 1);
    assert_eq!(mined.sigs[0].name, "User");
}

#[test]
fn mine_run_directory_auto_detect_rust() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("generated");

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    // Mine the whole generated directory — should auto-detect Rust
    let mined = extract::run(out_dir.to_str().unwrap(), None).unwrap();
    assert!(mined.sigs.iter().any(|s| s.name == "SigDecl"), "should find SigDecl");
    assert!(mined.sigs.iter().any(|s| s.name == "OxidtrIR"), "should find OxidtrIR");
}

#[test]
fn mine_run_directory_auto_detect_ts() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("generated");

    let config = GenerateConfig {
        target: "ts".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    let mined = extract::run(out_dir.to_str().unwrap(), None).unwrap();
    assert!(mined.sigs.iter().any(|s| s.name == "SigDecl"));
}

#[test]
fn mine_run_directory_auto_detect_kotlin() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("generated");

    let config = GenerateConfig {
        target: "kt".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    let mined = extract::run(out_dir.to_str().unwrap(), None).unwrap();
    assert!(mined.sigs.iter().any(|s| s.name == "SigDecl"));
}

#[test]
fn mine_run_directory_auto_detect_java() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("generated");

    let config = GenerateConfig {
        target: "java".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    let mined = extract::run(out_dir.to_str().unwrap(), None).unwrap();
    assert!(mined.sigs.iter().any(|s| s.name == "SigDecl"));
}

#[test]
fn mine_run_with_lang_override() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("data.txt"); // no recognized extension
    std::fs::write(&file, "pub struct Foo { pub x: Bar }\npub struct Bar {}").unwrap();

    // Auto-detect fails
    assert!(extract::run(file.to_str().unwrap(), None).is_err());

    // Override works
    let mined = extract::run(file.to_str().unwrap(), Some("rust")).unwrap();
    assert_eq!(mined.sigs.len(), 2);
}

#[test]
fn mine_run_merges_multiple_files_in_directory() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "pub struct Alpha {}").unwrap();
    std::fs::write(tmp.path().join("b.rs"), "pub struct Beta {}").unwrap();

    let mined = extract::run(tmp.path().to_str().unwrap(), None).unwrap();
    assert!(mined.sigs.iter().any(|s| s.name == "Alpha"));
    assert!(mined.sigs.iter().any(|s| s.name == "Beta"));
}
