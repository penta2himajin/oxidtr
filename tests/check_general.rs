/// Tests for check command with general (non-oxidtr-generated) code.

use oxidtr::check::{self, CheckConfig};

#[test]
fn check_general_rust_code_against_model() {
    let tmp = tempfile::tempdir().unwrap();

    // Write a simple Alloy model
    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, r#"
sig User { group: lone Group, roles: set Role }
sig Group {}
sig Role {}
"#).unwrap();

    // Write general Rust code (NOT oxidtr-generated, different file structure)
    let impl_dir = tmp.path().join("impl");
    std::fs::create_dir_all(impl_dir.join("domain")).unwrap();

    std::fs::write(impl_dir.join("domain/user.rs"), r#"
pub struct User {
    pub group: Option<Group>,
    pub roles: Vec<Role>,
}
"#).unwrap();

    std::fs::write(impl_dir.join("domain/group.rs"), r#"
pub struct Group {}
"#).unwrap();

    std::fs::write(impl_dir.join("domain/role.rs"), r#"
pub struct Role {}
"#).unwrap();

    let config = CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() };
    let result = check::run(model_path.to_str().unwrap(), &config).unwrap();

    // Should find all sigs via mine fallback (no models.rs needed)
    let missing_structs: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d, oxidtr::check::differ::DiffItem::MissingStruct { .. }))
        .collect();
    assert!(missing_structs.is_empty(),
        "should find all sigs via mine fallback: {:?}", missing_structs);
}

#[test]
fn check_general_ts_code_against_model() {
    let tmp = tempfile::tempdir().unwrap();

    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, r#"
sig User { name: one Name }
sig Name {}
"#).unwrap();

    let impl_dir = tmp.path().join("impl");
    std::fs::create_dir_all(&impl_dir).unwrap();

    // General TS code, not oxidtr-generated (no models.ts)
    std::fs::write(impl_dir.join("types.ts"), r#"
export interface User {
  name: Name;
}

export interface Name {}
"#).unwrap();

    let config = CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() };
    let result = check::run(model_path.to_str().unwrap(), &config).unwrap();

    let missing: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d, oxidtr::check::differ::DiffItem::MissingStruct { .. }))
        .collect();
    assert!(missing.is_empty(), "should find sigs from general TS: {:?}", missing);
}

#[test]
fn check_general_mixed_code_against_model() {
    let tmp = tempfile::tempdir().unwrap();

    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, r#"
sig Config { items: set Item }
sig Item {}
"#).unwrap();

    let impl_dir = tmp.path().join("impl");
    std::fs::create_dir_all(impl_dir.join("src")).unwrap();

    // Mixed: Rust struct in src/, TS interface in a different file
    std::fs::write(impl_dir.join("src/config.rs"), r#"
pub struct Config {
    pub items: Vec<Item>,
}
pub struct Item {}
"#).unwrap();

    let config = CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() };
    let result = check::run(model_path.to_str().unwrap(), &config).unwrap();

    let missing: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d, oxidtr::check::differ::DiffItem::MissingStruct { .. }))
        .collect();
    assert!(missing.is_empty(), "should find sigs from nested src/: {:?}", missing);
}

#[test]
fn check_detects_missing_sig_in_general_code() {
    let tmp = tempfile::tempdir().unwrap();

    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, r#"
sig User { role: one Role }
sig Role {}
sig Permission {}
"#).unwrap();

    let impl_dir = tmp.path().join("impl");
    std::fs::create_dir_all(&impl_dir).unwrap();

    // Only User and Role exist, Permission is missing
    std::fs::write(impl_dir.join("types.rs"), r#"
pub struct User {
    pub role: Role,
}
pub struct Role {}
"#).unwrap();

    let config = CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() };
    let result = check::run(model_path.to_str().unwrap(), &config).unwrap();

    assert!(result.diffs.iter().any(|d| matches!(d,
        oxidtr::check::differ::DiffItem::MissingStruct { name } if name == "Permission"
    )), "should detect Permission is missing: {:?}", result.diffs);
}

#[test]
fn check_general_kotlin_code_against_model() {
    let tmp = tempfile::tempdir().unwrap();

    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, r#"
sig User { group: lone Group, roles: set Role }
sig Group {}
sig Role {}
"#).unwrap();

    let impl_dir = tmp.path().join("impl");
    std::fs::create_dir_all(impl_dir.join("domain")).unwrap();

    std::fs::write(impl_dir.join("domain/User.kt"), r#"
data class User(
    val group: Group?,
    val roles: Set<Role>
)
"#).unwrap();

    std::fs::write(impl_dir.join("domain/Group.kt"), "data class Group(val placeholder: Unit = Unit)\n").unwrap();
    std::fs::write(impl_dir.join("domain/Role.kt"), "data class Role(val placeholder: Unit = Unit)\n").unwrap();

    let config = CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() };
    let result = check::run(model_path.to_str().unwrap(), &config).unwrap();

    let missing: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d, oxidtr::check::differ::DiffItem::MissingStruct { .. }))
        .collect();
    assert!(missing.is_empty(), "should find sigs from general Kotlin: {:?}", missing);
}

#[test]
fn check_general_java_code_against_model() {
    let tmp = tempfile::tempdir().unwrap();

    let model_path = tmp.path().join("model.als");
    std::fs::write(&model_path, r#"
sig User { name: one Name }
sig Name {}
"#).unwrap();

    let impl_dir = tmp.path().join("impl");
    std::fs::create_dir_all(impl_dir.join("model")).unwrap();

    std::fs::write(impl_dir.join("model/User.java"), r#"
public record User(Name name) {}
"#).unwrap();

    std::fs::write(impl_dir.join("model/Name.java"), "public record Name() {}\n").unwrap();

    let config = CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() };
    let result = check::run(model_path.to_str().unwrap(), &config).unwrap();

    let missing: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d, oxidtr::check::differ::DiffItem::MissingStruct { .. }))
        .collect();
    assert!(missing.is_empty(), "should find sigs from general Java: {:?}", missing);
}

#[test]
fn check_self_hosting_general_mode() {
    // Check oxidtr's own src/ against oxidtr-internal.als
    // This uses the mine fallback since src/ has no models.rs at root
    let config = CheckConfig { impl_dir: "src/".to_string() };

    // Using domain model — this should find most sigs via mine
    let result = check::run("models/oxidtr-domain.als", &config);

    // Should not error (mine fallback should work)
    assert!(result.is_ok(), "check should work on oxidtr src/ via mine fallback: {:?}", result.err());
}
