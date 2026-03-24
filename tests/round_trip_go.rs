use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::go;
use oxidtr::backend::GeneratedFile;
use oxidtr::extract::{go_extractor, MinedMultiplicity};
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

// ── Go round-trip ────────────────────────────────────────────────────────────

#[test]
fn round_trip_go_simple() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = go::generate(&ir);
    let models = find_file(&files, "models.go");

    let mined = go_extractor::extract(models);

    let user = assert_sig_exists(&mined.sigs, "User");
    assert_field(user, "group", MinedMultiplicity::Lone, "Group");
    assert_field(user, "roles", MinedMultiplicity::Set, "Role");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

#[test]
fn round_trip_go_enum() {
    let alloy_src = "abstract sig Status {}\none sig Active extends Status {}\none sig Inactive extends Status {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = go::generate(&ir);
    let models = find_file(&files, "models.go");

    let mined = go_extractor::extract(models);

    let status = assert_sig_exists(&mined.sigs, "Status");
    assert!(status.is_abstract);
    let active = assert_sig_exists(&mined.sigs, "Active");
    assert_eq!(active.parent.as_deref(), Some("Status"));
}

#[test]
fn round_trip_go_seq_field() {
    let alloy_src = "sig Order { items: seq Item }\nsig Item {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = go::generate(&ir);
    let models = find_file(&files, "models.go");

    let mined = go_extractor::extract(models);

    let order = assert_sig_exists(&mined.sigs, "Order");
    // Go slices map to Set in mine ([]T → set)
    assert_field(order, "items", MinedMultiplicity::Set, "Item");
}

#[test]
fn round_trip_go_check_consistency() {
    let dir = tempfile::TempDir::new().unwrap();
    let model_path = dir.path().join("test.als");
    std::fs::write(&model_path, "sig User { roles: set Role }\nsig Role {}").unwrap();

    let config = GenerateConfig {
        target: "go".to_string(),
        output_dir: dir.path().join("out").display().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    generate::run(model_path.to_str().unwrap(), &config).unwrap();

    let check_config = CheckConfig {
        impl_dir: dir.path().join("out").display().to_string(),
    };
    let result = check::run(model_path.to_str().unwrap(), &check_config).unwrap();
    assert!(result.is_ok(), "check should pass: {:?}", result.diffs);
}

#[test]
fn round_trip_go_self_model() {
    let self_model = include_str!("../models/oxidtr.als");
    let model = parser::parse(self_model).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = go::generate(&ir);

    // Verify key files are generated
    assert!(files.iter().any(|f| f.path == "models.go"), "no models.go");
    assert!(files.iter().any(|f| f.path == "models_test.go"), "no models_test.go");
    assert!(files.iter().any(|f| f.path == "fixtures.go"), "no fixtures.go");

    let models = find_file(&files, "models.go");

    // Verify key types from the self-hosting model exist
    assert!(models.contains("SigDecl"), "should contain SigDecl");
    assert!(models.contains("AlloyModel"), "should contain AlloyModel");
    assert!(models.contains("OxidtrIR"), "should contain OxidtrIR");
    assert!(models.contains("StructureNode"), "should contain StructureNode");
}
