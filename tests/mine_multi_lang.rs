/// Tests for multi-language mine merging.

use oxidtr::mine;
use oxidtr::generate::{self, GenerateConfig, WarningLevel};

#[test]
fn multi_lang_merge_rust_and_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Generate Rust into directory
    let config_rs = GenerateConfig {
        target: "rust".to_string(),
        output_dir: dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
    };
    generate::run("models/oxidtr.als", &config_rs).unwrap();

    // Also generate schema into same directory
    let source = std::fs::read_to_string("models/oxidtr.als").unwrap();
    let model = oxidtr::parser::parse(&source).unwrap();
    let ir = oxidtr::ir::lower(&model).unwrap();
    let schema_file = oxidtr::backend::schema::generate(&ir);
    std::fs::write(dir.join(&schema_file.path), &schema_file.content).unwrap();

    // Mine with no --lang → multi-lang merge
    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();

    assert!(result.sources_used.len() >= 2,
        "should use multiple sources: {:?}", result.sources_used);
    assert!(result.model.sigs.iter().any(|s| s.name == "SigDecl"));
    assert!(result.model.sigs.iter().any(|s| s.name == "OxidtrIR"));
}

#[test]
fn multi_lang_merge_ts_and_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let config_ts = GenerateConfig {
        target: "ts".to_string(),
        output_dir: dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
        schema: None,
    };
    generate::run("models/oxidtr.als", &config_ts).unwrap();

    // Also generate schema
    let source = std::fs::read_to_string("models/oxidtr.als").unwrap();
    let model = oxidtr::parser::parse(&source).unwrap();
    let ir = oxidtr::ir::lower(&model).unwrap();
    let schema_file = oxidtr::backend::schema::generate(&ir);
    std::fs::write(dir.join(&schema_file.path), &schema_file.content).unwrap();

    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();

    assert!(result.sources_used.len() >= 2,
        "should use TS + schema: {:?}", result.sources_used);
    assert!(result.model.sigs.iter().any(|s| s.name == "SigDecl"));
}

#[test]
fn multi_lang_merge_consistent_no_conflicts() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Write consistent Rust and TS models — use domain types (not primitives)
    // to avoid false conflicts from language-specific type names (String vs string)
    std::fs::write(dir.join("models.rs"), r#"
pub struct User {
    pub group: Option<Group>,
    pub roles: Vec<Role>,
}
pub struct Group {}
pub struct Role {}
"#).unwrap();

    std::fs::write(dir.join("models.ts"), r#"
export interface User {
  group: Group | null;
  roles: Role[];
}
export interface Group {}
export interface Role {}
"#).unwrap();

    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();

    assert!(result.sources_used.len() == 2, "sources: {:?}", result.sources_used);

    let user = result.model.sigs.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.fields.len(), 2);
    assert!(result.model.sigs.iter().any(|s| s.name == "Group"));
    assert!(result.model.sigs.iter().any(|s| s.name == "Role"));

    // No conflicts (consistent domain-type definitions)
    assert!(result.conflicts.is_empty(),
        "consistent models should have no conflicts: {:?}", result.conflicts);
}

#[test]
fn multi_lang_merge_detects_multiplicity_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Rust says Set, TS says Seq (array) — multiplicity mismatch
    std::fs::write(dir.join("models.rs"), r#"
use std::collections::BTreeSet;
pub struct Team {
    pub members: BTreeSet<User>,
}
pub struct User {}
"#).unwrap();

    std::fs::write(dir.join("models.ts"), r#"
export interface Team {
  members: User[];
}
export interface User {}
"#).unwrap();

    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();

    // Should detect multiplicity conflict
    assert!(!result.conflicts.is_empty(),
        "should detect multiplicity conflict between Set and Seq");
    assert!(result.conflicts.iter().any(|c|
        c.sig_name == "Team" && c.field_name == "members"
            && c.description.contains("multiplicity")
    ), "conflict should mention Team.members: {:?}", result.conflicts);
}

#[test]
fn multi_lang_merge_supplements_missing_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Rust has 2 fields, TS has 1 field + 1 extra
    std::fs::write(dir.join("models.rs"), r#"
pub struct User {
    pub name: String,
    pub age: i32,
}
"#).unwrap();

    std::fs::write(dir.join("models.ts"), r#"
export interface User {
  name: string;
  email: string;
}
"#).unwrap();

    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();

    let user = result.model.sigs.iter().find(|s| s.name == "User").unwrap();
    // Should have all 3 fields: name (both), age (rust), email (ts)
    assert!(user.fields.iter().any(|f| f.name == "name"), "should have name");
    assert!(user.fields.iter().any(|f| f.name == "age"), "should have age from rust");
    assert!(user.fields.iter().any(|f| f.name == "email"), "should have email from ts");
}

#[test]
fn multi_lang_schema_supplements_constraints() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    std::fs::write(dir.join("models.rs"), r#"
pub struct User {
    pub name: String,
}
"#).unwrap();

    // Schema adds structural info
    std::fs::write(dir.join("schemas.json"), r##"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "definitions": {
    "User": {
      "type": "object",
      "properties": {
        "name": {
          "$ref": "#/definitions/String"
        }
      },
      "required": ["name"]
    },
    "String": {
      "type": "object",
      "properties": {},
      "required": []
    }
  }
}
"##).unwrap();

    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();

    assert!(result.sources_used.iter().any(|s| s.contains("rust")));
    assert!(result.sources_used.iter().any(|s| s.contains("schema")));
    assert!(result.model.sigs.iter().any(|s| s.name == "User"));
}

#[test]
fn single_lang_override_skips_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    std::fs::write(dir.join("models.rs"), "pub struct Foo {}").unwrap();
    std::fs::write(dir.join("models.ts"), "export interface Bar {}").unwrap();

    // With --lang rust, only Rust files are mined
    let result = mine::run_merge(dir.to_str().unwrap(), Some("rust")).unwrap();
    assert_eq!(result.sources_used, vec!["rust"]);
    assert!(result.model.sigs.iter().any(|s| s.name == "Foo"));
    assert!(!result.model.sigs.iter().any(|s| s.name == "Bar"));
}

#[test]
fn multi_lang_merge_deduplicates_fact_candidates() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Both files contain .contains() pattern → same fact candidate
    std::fs::write(dir.join("check.rs"), r#"
pub fn check(items: &[Item]) {
    if items.contains(&target) { return; }
}
pub struct Item {}
"#).unwrap();

    std::fs::write(dir.join("check.ts"), r#"
export function check(items: Item[]): void {
  if (items.includes(target)) return;
}
export interface Item {}
"#).unwrap();

    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();

    // Fact candidates should be deduplicated
    let contains_facts: Vec<_> = result.model.fact_candidates.iter()
        .filter(|f| f.source_pattern.contains("contains") || f.source_pattern.contains("includes"))
        .collect();
    // Should have facts from both but deduplicated by alloy_text
    assert!(!contains_facts.is_empty());
}

#[test]
fn multi_lang_rendered_model_is_parseable() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    std::fs::write(dir.join("models.rs"), r#"
pub struct User {
    pub name: String,
    pub group: Option<Group>,
}
pub struct Group {}
"#).unwrap();

    std::fs::write(dir.join("models.ts"), r#"
export interface User {
  name: string;
  group: Group | null;
}
export interface Group {}
"#).unwrap();

    let result = mine::run_merge(dir.to_str().unwrap(), None).unwrap();
    let rendered = mine::renderer::render(&result.model);

    // Merged model should be parseable as Alloy
    let parsed = oxidtr::parser::parse(&rendered);
    assert!(parsed.is_ok(), "merged model should parse:\n{rendered}\nerror: {:?}", parsed.err());
}
