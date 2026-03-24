use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::backend::GeneratedFile;
use oxidtr::extract::{kotlin_extractor, java_extractor, MinedMultiplicity};
use oxidtr::check::{self, CheckConfig};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::backend::typescript::TsTestRunner;

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

fn assert_sig_exists<'a>(sigs: &'a [oxidtr::extract::MinedSig], name: &str) -> &'a oxidtr::extract::MinedSig {
    sigs.iter().find(|s| s.name == name)
        .unwrap_or_else(|| panic!("expected sig '{name}' not found"))
}

fn assert_field(sig: &oxidtr::extract::MinedSig, name: &str, mult: MinedMultiplicity, target: &str) {
    let f = sig.fields.iter().find(|f| f.name == name)
        .unwrap_or_else(|| panic!("field '{name}' not found in sig '{}'", sig.name));
    assert_eq!(f.mult, mult, "field {}.{name} multiplicity", sig.name);
    assert_eq!(f.target, target, "field {}.{name} target", sig.name);
}

// ── Kotlin round-trip ──────────────────────────────────────────────────────

#[test]
fn round_trip_kotlin_simple() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let models = find_file(&files, "Models.kt");

    let mined = kotlin_extractor::extract(models);

    let user = assert_sig_exists(&mined.sigs, "User");
    assert_field(user, "group", MinedMultiplicity::Lone, "Group");
    assert_field(user, "roles", MinedMultiplicity::Set, "Role");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

#[test]
fn round_trip_kotlin_enum() {
    let alloy_src = "abstract sig Status {}\none sig Active extends Status {}\none sig Inactive extends Status {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let models = find_file(&files, "Models.kt");

    let mined = kotlin_extractor::extract(models);

    let status = assert_sig_exists(&mined.sigs, "Status");
    assert!(status.is_abstract);
    let active = assert_sig_exists(&mined.sigs, "Active");
    assert_eq!(active.parent.as_deref(), Some("Status"));
}

#[test]
fn round_trip_kotlin_sealed_with_fields() {
    let alloy_src = "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let models = find_file(&files, "Models.kt");

    let mined = kotlin_extractor::extract(models);

    let expr = assert_sig_exists(&mined.sigs, "Expr");
    assert!(expr.is_abstract);
    let binop = assert_sig_exists(&mined.sigs, "BinOp");
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 2);
}

#[test]
fn self_hosting_round_trip_kotlin() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let models = find_file(&files, "Models.kt");

    let mined = kotlin_extractor::extract(models);

    assert_sig_exists(&mined.sigs, "SigDecl");
    assert_sig_exists(&mined.sigs, "OxidtrIR");
    let mult = assert_sig_exists(&mined.sigs, "Multiplicity");
    assert!(mult.is_abstract);
}

#[test]
fn check_kotlin_self_hosting() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "kt".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    let check_config = CheckConfig {
        impl_dir: out_dir.to_str().unwrap().to_string(),
    };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();

    if !result.is_ok() {
        for d in &result.diffs { eprintln!("  {d}"); }
    }
    assert!(result.is_ok(), "check Kotlin self-hosting: {} diffs", result.diffs.len());
}

// ── Java round-trip ────────────────────────────────────────────────────────

#[test]
fn round_trip_java_simple() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    let mined = java_extractor::extract(models);

    let user = assert_sig_exists(&mined.sigs, "User");
    assert_field(user, "group", MinedMultiplicity::Lone, "Group");
    assert_field(user, "roles", MinedMultiplicity::Set, "Role");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

#[test]
fn round_trip_java_enum() {
    let alloy_src = "abstract sig Status {}\none sig Active extends Status {}\none sig Inactive extends Status {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    let mined = java_extractor::extract(models);

    let status = assert_sig_exists(&mined.sigs, "Status");
    assert!(status.is_abstract);
    let active = assert_sig_exists(&mined.sigs, "Active");
    assert_eq!(active.parent.as_deref(), Some("Status"));
}

#[test]
fn round_trip_java_sealed_with_fields() {
    let alloy_src = "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    let mined = java_extractor::extract(models);

    let expr = assert_sig_exists(&mined.sigs, "Expr");
    assert!(expr.is_abstract);
    let binop = assert_sig_exists(&mined.sigs, "BinOp");
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 2);
}

#[test]
fn self_hosting_round_trip_java() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    let mined = java_extractor::extract(models);

    assert_sig_exists(&mined.sigs, "SigDecl");
    assert_sig_exists(&mined.sigs, "OxidtrIR");
    let mult = assert_sig_exists(&mined.sigs, "Multiplicity");
    assert!(mult.is_abstract);
}

#[test]
fn check_java_self_hosting() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "java".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    let check_config = CheckConfig {
        impl_dir: out_dir.to_str().unwrap().to_string(),
    };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();

    if !result.is_ok() {
        for d in &result.diffs { eprintln!("  {d}"); }
    }
    assert!(result.is_ok(), "check Java self-hosting: {} diffs", result.diffs.len());
}
