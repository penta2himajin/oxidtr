/// Mine tests for new patterns: Map types, singletons, concrete annotations,
/// boundary fixtures, and enriched output round-trips.

use oxidtr::extract::{self, rust_extractor, ts_extractor, kotlin_extractor, java_extractor, MinedMultiplicity};
use oxidtr::backend::{rust, typescript, GeneratedFile};
use oxidtr::backend::typescript::TsTestRunner;
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::parser;
use oxidtr::ir;

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── Map type round-trip ────────────────────────────────────────────────────

#[test]
fn mine_rust_btreemap_field() {
    let src = r#"
use std::collections::BTreeMap;
pub struct Registry {
    pub entries: BTreeMap<String, User>,
}
pub struct User {}
"#;
    let mined = rust_extractor::extract(src);
    let reg = mined.sigs.iter().find(|s| s.name == "Registry").unwrap();
    assert_eq!(reg.fields.len(), 1);
    // BTreeMap should be detected as a map/set-like field
    assert_eq!(reg.fields[0].name, "entries");
}

#[test]
fn mine_ts_map_field() {
    let src = r#"
export interface Registry {
  entries: Map<string, User>;
}
export interface User {}
"#;
    let mined = ts_extractor::extract(src);
    let reg = mined.sigs.iter().find(|s| s.name == "Registry").unwrap();
    assert_eq!(reg.fields.len(), 1);
    assert_eq!(reg.fields[0].name, "entries");
}

#[test]
fn mine_kotlin_map_field() {
    let src = r#"
data class Registry(
    val entries: Map<String, User>
)
object User
"#;
    let mined = kotlin_extractor::extract(src);
    let reg = mined.sigs.iter().find(|s| s.name == "Registry").unwrap();
    assert_eq!(reg.fields.len(), 1);
    assert_eq!(reg.fields[0].name, "entries");
}

#[test]
fn mine_java_map_field() {
    let src = r#"
import java.util.Map;
public record Registry(Map<String, User> entries) {}
public record User() {}
"#;
    let mined = java_extractor::extract(src);
    let reg = mined.sigs.iter().find(|s| s.name == "Registry").unwrap();
    assert_eq!(reg.fields.len(), 1);
    assert_eq!(reg.fields[0].name, "entries");
}

// ── Singleton round-trip ───────────────────────────────────────────────────

#[test]
fn mine_kotlin_object_singleton() {
    let src = "object Red\n";
    let mined = kotlin_extractor::extract(src);
    assert!(mined.sigs.iter().any(|s| s.name == "Red"), "should extract object as sig");
}

#[test]
fn mine_java_enum_singleton() {
    let src = "public enum Red { INSTANCE }\n";
    let mined = java_extractor::extract(src);
    assert!(mined.sigs.iter().any(|s| s.name == "Red"), "should extract single-value enum");
}

// ── Concrete @Size annotations don't break mine ────────────────────────────

#[test]
fn mine_kotlin_with_concrete_size_annotation() {
    let alloy_src = "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 5 }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let models = find_file(&files, "Models.kt");

    let mined = kotlin_extractor::extract(models);
    let team = mined.sigs.iter().find(|s| s.name == "Team").unwrap();
    assert_eq!(team.fields.len(), 1, "Team should have 1 field: {:?}", team.fields);
    assert_eq!(team.fields[0].name, "members");
    assert_eq!(team.fields[0].mult, MinedMultiplicity::Set);
}

#[test]
fn mine_java_with_concrete_size_and_compact_constructor() {
    let alloy_src = "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 5 }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    let mined = java_extractor::extract(models);
    let team = mined.sigs.iter().find(|s| s.name == "Team").unwrap();
    assert_eq!(team.fields.len(), 1, "Team should have 1 field: {:?}", team.fields);
    assert_eq!(team.fields[0].name, "members");
    assert_eq!(team.fields[0].mult, MinedMultiplicity::Set);
}

// ── Boundary fixtures don't pollute mine ───────────────────────────────────

#[test]
fn mine_rust_ignores_boundary_fixtures() {
    let alloy_src = "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 5 }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = rust::generate(&ir);
    let fixtures = find_file(&files, "fixtures.rs");

    // Fixtures file may have boundary/invalid functions
    let mined = rust_extractor::extract(fixtures);
    // Should NOT create sigs from fixture functions
    assert!(mined.sigs.is_empty() || mined.sigs.iter().all(|s| s.name != "boundary_team"),
        "fixture functions should not become sigs");
}

#[test]
fn mine_ts_ignores_boundary_fixtures() {
    let alloy_src = "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 5 }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir);
    let fixtures = find_file(&files, "fixtures.ts");

    let mined = ts_extractor::extract(fixtures);
    assert!(mined.sigs.is_empty(), "TS fixture functions should not become sigs");
}

// ── Full generate → mine directory round-trip ──────────────────────────────

#[test]
fn mine_directory_round_trip_rust_with_all_enrichments() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("gen");

    let config = oxidtr::generate::GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: oxidtr::generate::WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    oxidtr::generate::run("models/oxidtr.als", &config).unwrap();

    // Mine the whole directory
    let mined = extract::run(out_dir.to_str().unwrap(), None).unwrap();
    assert!(mined.sigs.iter().any(|s| s.name == "SigDecl"), "should find SigDecl");
    assert!(mined.sigs.iter().any(|s| s.name == "OxidtrIR"), "should find OxidtrIR");
    assert!(mined.sigs.iter().any(|s| s.name == "Multiplicity"), "should find Multiplicity");
}

#[test]
fn mine_directory_round_trip_ts_with_all_enrichments() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("gen");

    let config = oxidtr::generate::GenerateConfig {
        target: "ts".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: oxidtr::generate::WarningLevel::Off,
        features: vec![],
        schema: None,
        ts_test_runner: TsTestRunner::Bun,
    };
    oxidtr::generate::run("models/oxidtr.als", &config).unwrap();

    let mined = extract::run(out_dir.to_str().unwrap(), None).unwrap();
    assert!(mined.sigs.iter().any(|s| s.name == "SigDecl"));
    assert!(mined.sigs.iter().any(|s| s.name == "OxidtrIR"));
}
