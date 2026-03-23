use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::swift;
use oxidtr::backend::GeneratedFile;
use oxidtr::mine::{swift_extractor, MinedMultiplicity};
use oxidtr::check::{self, CheckConfig};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::backend::typescript::TsTestRunner;

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

fn assert_sig_exists<'a>(sigs: &'a [oxidtr::mine::MinedSig], name: &str) -> &'a oxidtr::mine::MinedSig {
    sigs.iter().find(|s| s.name == name)
        .unwrap_or_else(|| panic!("expected sig '{name}' not found"))
}

fn assert_field(sig: &oxidtr::mine::MinedSig, name: &str, mult: MinedMultiplicity, target: &str) {
    let f = sig.fields.iter().find(|f| f.name == name)
        .unwrap_or_else(|| panic!("field '{name}' not found in sig '{}'", sig.name));
    assert_eq!(f.mult, mult, "field {}.{name} multiplicity", sig.name);
    assert_eq!(f.target, target, "field {}.{name} target", sig.name);
}

// ── Swift round-trip ─────────────────────────────────────────────────────────

#[test]
fn round_trip_swift_simple() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = swift::generate(&ir);
    let models = find_file(&files, "Models.swift");

    let mined = swift_extractor::extract(models);

    let user = assert_sig_exists(&mined.sigs, "User");
    assert_field(user, "group", MinedMultiplicity::Lone, "Group");
    assert_field(user, "roles", MinedMultiplicity::Set, "Role");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

#[test]
fn round_trip_swift_enum() {
    let alloy_src = "abstract sig Status {}\none sig Active extends Status {}\none sig Inactive extends Status {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = swift::generate(&ir);
    let models = find_file(&files, "Models.swift");

    let mined = swift_extractor::extract(models);

    let status = assert_sig_exists(&mined.sigs, "Status");
    assert!(status.is_abstract);
    let active = assert_sig_exists(&mined.sigs, "Active");
    assert_eq!(active.parent.as_deref(), Some("Status"));
}

#[test]
fn round_trip_swift_seq_field() {
    let alloy_src = "sig Order { items: seq Item }\nsig Item {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = swift::generate(&ir);
    let models = find_file(&files, "Models.swift");

    let mined = swift_extractor::extract(models);

    let order = assert_sig_exists(&mined.sigs, "Order");
    assert_field(order, "items", MinedMultiplicity::Seq, "Item");
}

#[test]
fn round_trip_swift_check_consistency() {
    let dir = tempfile::TempDir::new().unwrap();
    let model_path = dir.path().join("test.als");
    std::fs::write(&model_path, "sig User { roles: set Role }\nsig Role {}").unwrap();

    let config = GenerateConfig {
        target: "swift".to_string(),
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
fn round_trip_swift_self_model() {
    let self_model = include_str!("../models/oxidtr.als");
    let model = parser::parse(self_model).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = swift::generate(&ir);

    // Verify key files are generated
    assert!(files.iter().any(|f| f.path == "Models.swift"), "no Models.swift");
    assert!(files.iter().any(|f| f.path == "Tests.swift"), "no Tests.swift");
    assert!(files.iter().any(|f| f.path == "Fixtures.swift"), "no Fixtures.swift");

    let models = find_file(&files, "Models.swift");

    // Verify key types from the self-hosting model exist
    assert!(models.contains("struct SigDecl"), "should contain SigDecl");
    assert!(models.contains("struct AlloyModel"), "should contain AlloyModel");
    assert!(models.contains("struct OxidtrIR"), "should contain OxidtrIR");
    assert!(models.contains("struct StructureNode"), "should contain StructureNode");
}
