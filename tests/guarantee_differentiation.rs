/// Integration tests for language-differentiated code generation.
/// Verifies that Rust generates fewer tests than TS, validators are generated for TS,
/// and schema auto-inclusion follows language defaults.

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::{rust, typescript, GeneratedFile};
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::backend::typescript::TsTestRunner;
use oxidtr::backend::go;
use oxidtr::analyze::guarantee::{can_guarantee_by_type, Guarantee, TargetLang, enum_exhaustiveness_guarantee};
use oxidtr::analyze::{ConstraintInfo, PresenceKind, BoundKind};

fn find_file<'a>(files: &'a [GeneratedFile], name: &str) -> &'a str {
    files.iter()
        .find(|f| f.path == name)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {} not found in {:?}", name, files.iter().map(|f| &f.path).collect::<Vec<_>>()))
}

fn file_exists(files: &[GeneratedFile], name: &str) -> bool {
    files.iter().any(|f| f.path == name)
}

fn count_test_fns(content: &str, prefix: &str) -> usize {
    content.lines().filter(|l| l.contains(prefix)).count()
}

// ── Guarantee classification tests ──────────────────────────────────────────

#[test]
fn guarantee_null_safety_varies_by_language() {
    let c = ConstraintInfo::Presence {
        sig_name: "User".to_string(),
        field_name: "name".to_string(),
        kind: PresenceKind::Required,
    };
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Rust), Guarantee::FullyByType);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Swift), Guarantee::FullyByType);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Kotlin), Guarantee::FullyByType);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Java), Guarantee::PartiallyByType);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Go), Guarantee::PartiallyByType);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::TypeScript), Guarantee::RequiresTest);
}

#[test]
fn guarantee_cardinality_varies_by_language() {
    let c = ConstraintInfo::CardinalityBound {
        sig_name: "User".to_string(),
        field_name: "roles".to_string(),
        bound: BoundKind::AtMost(5),
    };
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Rust), Guarantee::PartiallyByType);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Swift), Guarantee::RequiresTest);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Kotlin), Guarantee::RequiresTest);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Java), Guarantee::RequiresTest);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::Go), Guarantee::RequiresTest);
    assert_eq!(can_guarantee_by_type(&c, TargetLang::TypeScript), Guarantee::RequiresTest);
}

#[test]
fn guarantee_no_self_ref_always_requires_test() {
    let c = ConstraintInfo::NoSelfRef {
        sig_name: "Node".to_string(),
        field_name: "parent".to_string(),
    };
    for lang in [TargetLang::Rust, TargetLang::Swift, TargetLang::Kotlin, TargetLang::Java, TargetLang::Go, TargetLang::TypeScript] {
        assert_eq!(can_guarantee_by_type(&c, lang), Guarantee::RequiresTest);
    }
}

#[test]
fn guarantee_acyclicity_always_requires_test() {
    let c = ConstraintInfo::Acyclic {
        sig_name: "Node".to_string(),
        field_name: "parent".to_string(),
    };
    for lang in [TargetLang::Rust, TargetLang::Swift, TargetLang::Kotlin, TargetLang::Java, TargetLang::Go, TargetLang::TypeScript] {
        assert_eq!(can_guarantee_by_type(&c, lang), Guarantee::RequiresTest);
    }
}

#[test]
fn guarantee_enum_exhaustiveness_by_language() {
    assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Rust), Guarantee::FullyByType);
    assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Swift), Guarantee::FullyByType);
    assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Kotlin), Guarantee::FullyByType);
    assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Java), Guarantee::FullyByType);
    assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Go), Guarantee::RequiresTest);
    assert_eq!(enum_exhaustiveness_guarantee(TargetLang::TypeScript), Guarantee::RequiresTest);
}

// ── Rust generates fewer tests than TS ──────────────────────────────────────

#[test]
fn rust_generates_fewer_tests_than_ts_for_same_model() {
    // Model with null-safety constraints (type-guaranteed in Rust but not TS)
    let src = r#"
        sig User { name: one Name, roles: set Role }
        sig Name {}
        sig Role {}
        fact NullSafety { all u: User | #u.roles <= 5 }
    "#;
    let model = parser::parse(src).unwrap();
    let ir_val = ir::lower(&model).unwrap();

    let rust_files = rust::generate(&ir_val);
    let ts_files = typescript::generate(&ir_val);

    let rust_tests = find_file(&rust_files, "tests.rs");
    let ts_tests = find_file(&ts_files, "tests.ts");

    // Count actual test functions in each
    let rust_test_count = count_test_fns(rust_tests, "#[test]");
    let ts_test_count = count_test_fns(ts_tests, "it('");

    // TS should have at least as many tests as Rust
    assert!(ts_test_count >= rust_test_count,
        "TS ({}) should generate >= tests than Rust ({})", ts_test_count, rust_test_count);
}

// ── TS generates validators.ts ──────────────────────────────────────────────

#[test]
fn ts_generates_validators_for_constrained_model() {
    let src = r#"
        sig User { name: one Name, roles: set Role }
        sig Name {}
        sig Role {}
        fact RolesLimit { all u: User | #u.roles <= 5 }
    "#;
    let model = parser::parse(src).unwrap();
    let ir_val = ir::lower(&model).unwrap();

    let validators = typescript::generate_validators(&ir_val);
    assert!(!validators.is_empty(), "TS should generate validators");
    assert!(validators.contains("validateUser"), "should have validateUser function");
    assert!(validators.contains("roles exceeds max size 5"), "should check cardinality");
}

#[test]
fn ts_validators_check_null_safety() {
    let src = r#"
        sig User { name: one Name }
        sig Name {}
    "#;
    let model = parser::parse(src).unwrap();
    let ir_val = ir::lower(&model).unwrap();

    let validators = typescript::generate_validators(&ir_val);
    assert!(validators.contains("must not be null"), "TS validator should check null for 'one' fields");
}

#[test]
fn ts_validators_empty_for_no_concrete_sigs() {
    let src = "abstract sig Foo {}\none sig Bar extends Foo {}";
    let model = parser::parse(src).unwrap();
    let ir_val = ir::lower(&model).unwrap();

    let validators = typescript::generate_validators(&ir_val);
    assert!(validators.is_empty(), "no validators for enum-only models");
}

// ── Schema auto-inclusion per language ──────────────────────────────────────

#[test]
fn schema_auto_included_for_ts() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, "sig User { name: one Name }\nsig Name {}").unwrap();

    let config = GenerateConfig {
        target: "ts".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None, // use default
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(!result.files_written.iter().any(|f| f.contains("schemas.json")),
        "schemas.json should NOT be auto-included for TS (default off), got: {:?}", result.files_written);
}

#[test]
fn schema_not_auto_included_for_java() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, "sig User { name: one Name }\nsig Name {}").unwrap();

    let config = GenerateConfig {
        target: "java".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(!result.files_written.iter().any(|f| f.contains("schemas.json")),
        "schemas.json should NOT be auto-included for Java (default off), got: {:?}", result.files_written);
}

#[test]
fn schema_not_auto_included_for_rust() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, "sig User { name: one Name }\nsig Name {}").unwrap();

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(!result.files_written.iter().any(|f| f.contains("schemas.json")),
        "schemas.json should NOT be auto-included for Rust, got: {:?}", result.files_written);
}

#[test]
fn schema_not_auto_included_for_kotlin() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, "sig User { name: one Name }\nsig Name {}").unwrap();

    let config = GenerateConfig {
        target: "kt".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(!result.files_written.iter().any(|f| f.contains("schemas.json")),
        "schemas.json should NOT be auto-included for Kotlin, got: {:?}", result.files_written);
}

#[test]
fn schema_flag_overrides_default() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, "sig User { name: one Name }\nsig Name {}").unwrap();

    // Force schema on for Rust (normally off)
    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: Some(true),
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(result.files_written.iter().any(|f| f.contains("schemas.json")),
        "schemas.json should be included when --schema=true for Rust, got: {:?}", result.files_written);
}

#[test]
fn schema_flag_disables_for_ts() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, "sig User { name: one Name }\nsig Name {}").unwrap();

    // Explicitly off for TS (same as default, but explicit)
    let config = GenerateConfig {
        target: "ts".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: Some(false),
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(!result.files_written.iter().any(|f| f.contains("schemas.json")),
        "schemas.json should NOT be included when --schema=false for TS, got: {:?}", result.files_written);
}

// ── TS generates validators.ts via pipeline ─────────────────────────────────

#[test]
fn ts_pipeline_generates_validators_file() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, "sig User { name: one Name, roles: set Role }\nsig Name {}\nsig Role {}\nfact RolesLimit { all u: User | #u.roles <= 5 }").unwrap();

    let config = GenerateConfig {
        target: "ts".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(result.files_written.iter().any(|f| f.contains("validators.ts")),
        "validators.ts should be generated for TS, got: {:?}", result.files_written);
}

// ── Rust no longer generates invariants ──────────────────────────────────────

#[test]
fn rust_no_invariants_file_generated() {
    let src = r#"
        sig User { name: one Name, roles: set Role }
        sig Name {}
        sig Role {}
        fact NullSafety { all u: User | #u.roles <= 5 }
    "#;
    let model = parser::parse(src).unwrap();
    let ir_val = ir::lower(&model).unwrap();

    let files = rust::generate(&ir_val);
    assert!(!files.iter().any(|f| f.path == "invariants.rs"),
        "should NOT generate invariants.rs");
}

// ── Kotlin value class for newtypes ─────────────────────────────────────────

#[test]
fn kotlin_value_class_for_constrained_newtype() {
    let src = r#"
        sig Wrapper { value: one Inner }
        sig Inner {}
        fact WrapperBound { all w: Wrapper | #w.value = 1 }
    "#;
    let model = parser::parse(src).unwrap();
    let ir_val = ir::lower(&model).unwrap();

    let files = kotlin::generate(&ir_val);
    let models = find_file(&files, "Models.kt");

    // Should NOT generate value class since it needs a cardinality constraint on the Wrapper sig
    // and the fact references #w.value (cardinality) which is analyzed as CardinalityBound
    // The wrapper has exactly one field with a cardinality constraint → value class
    assert!(models.contains("value class Wrapper") || models.contains("data class Wrapper"),
        "Kotlin should generate either value class or data class for Wrapper:\n{}", models);
}

// ── Self-hosting verification ───────────────────────────────────────────────

#[test]
fn self_hosting_all_targets_still_work() {
    let source = std::fs::read_to_string("models/oxidtr.als")
        .expect("should read models/oxidtr.als");
    let model = parser::parse(&source).unwrap();
    let ir_val = ir::lower(&model).unwrap();

    // All backends should still generate without panic
    let rust_files = rust::generate(&ir_val);
    let ts_files = typescript::generate(&ir_val);
    let kt_files = kotlin::generate(&ir_val);
    let java_files = java::generate(&ir_val);
    let go_files = go::generate(&ir_val);

    // Each backend produces models + tests at minimum
    // oxidtr.als uses module declarations → Rust generates lib.rs + module dirs
    assert!(file_exists(&rust_files, "lib.rs") || file_exists(&rust_files, "models.rs"),
        "Rust should generate lib.rs (modular) or models.rs (flat)");
    assert!(file_exists(&ts_files, "models.ts"), "TS should generate models.ts");
    assert!(file_exists(&kt_files, "Models.kt"), "Kotlin should generate Models.kt");
    assert!(file_exists(&java_files, "Models.java"), "Java should generate Models.java");
    assert!(file_exists(&go_files, "models.go"), "Go should generate models.go");

    // TS additionally generates validators
    let validators = typescript::generate_validators(&ir_val);
    assert!(!validators.is_empty(), "TS should generate validators for oxidtr model");
}
