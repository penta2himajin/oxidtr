/// Round-trip tests for enriched output: verify mine/check still work
/// when generated code includes Bean Validation, compact constructors,
/// operations doc, newtypes, serde derives, and non-vacuous tests.

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::{rust, typescript, GeneratedFile};
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::mine::{rust_extractor, ts_extractor, kotlin_extractor, java_extractor, MinedMultiplicity};
use oxidtr::check::{self, CheckConfig};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── Bean Validation annotations don't break mine ───────────────────────────

#[test]
fn mine_kotlin_with_bean_validation_annotations() {
    let alloy_src = "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members = #t.members }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let models = find_file(&files, "Models.kt");

    // Models.kt now has @Size annotations as comments
    let mined = kotlin_extractor::extract(models);

    let team = mined.sigs.iter().find(|s| s.name == "Team").unwrap();
    assert_eq!(team.fields.len(), 1, "Team should have 1 field after mine: {:?}", team.fields);
    assert_eq!(team.fields[0].name, "members");
    assert_eq!(team.fields[0].mult, MinedMultiplicity::Set);
    assert_eq!(team.fields[0].target, "User");
}

#[test]
fn mine_java_with_bean_validation_and_compact_constructor() {
    let alloy_src = "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    // Models.java now has @NotNull, compact constructor
    let mined = java_extractor::extract(models);

    let user = mined.sigs.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.fields.len(), 1, "User should have 1 field after mine: {:?}", user.fields);
    assert_eq!(user.fields[0].name, "role");
    assert_eq!(user.fields[0].mult, MinedMultiplicity::One);
}

// ── Serde derives don't break mine ─────────────────────────────────────────

#[test]
fn mine_rust_with_serde_derives() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let config = rust::RustBackendConfig {
        features: vec!["serde".to_string()],
    };
    let files = rust::generate_with_config(&ir, &config);
    let models = find_file(&files, "models.rs");

    // models.rs now has Serialize, Deserialize derives
    assert!(models.contains("Serialize"), "serde should be present");

    let mined = rust_extractor::extract(models);
    let user = mined.sigs.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.fields.len(), 2);
    assert_eq!(user.fields.iter().find(|f| f.name == "group").unwrap().mult, MinedMultiplicity::Lone);
    assert_eq!(user.fields.iter().find(|f| f.name == "roles").unwrap().mult, MinedMultiplicity::Set);
}

#[test]
fn mine_rust_serde_enum_with_tag() {
    let alloy_src = "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let config = rust::RustBackendConfig {
        features: vec!["serde".to_string()],
    };
    let files = rust::generate_with_config(&ir, &config);
    let models = find_file(&files, "models.rs");

    assert!(models.contains("#[serde(tag"), "serde tag should be present");

    let mined = rust_extractor::extract(models);
    let expr = mined.sigs.iter().find(|s| s.name == "Expr").unwrap();
    assert!(expr.is_abstract);
    let binop = mined.sigs.iter().find(|s| s.name == "BinOp").unwrap();
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 1);
}

// ── Operations doc don't break mine ────────────────────────────────────────

#[test]
fn mine_ts_with_operations_doc() {
    let alloy_src = "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir);
    let ops = find_file(&files, "operations.ts");

    // operations.ts now has JSDoc @pre comments
    assert!(ops.contains("@pre"), "JSDoc should be present");

    // Mine should not be confused by doc comments
    let mined = ts_extractor::extract(ops);
    // Operations file has no interfaces, so no sigs expected
    assert_eq!(mined.sigs.len(), 0);
}

// ── Self-hosting check with enriched output ────────────────────────────────

#[test]
fn check_rust_self_hosting_with_enriched_output() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    // Verify enriched files exist
    assert!(out_dir.join("fixtures.rs").exists(), "fixtures.rs should be generated");
    assert!(out_dir.join("newtypes.rs").exists(), "newtypes.rs should be generated");

    let check_config = CheckConfig { impl_dir: out_dir.to_str().unwrap().to_string() };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();
    assert!(result.is_ok(), "check should pass with enriched Rust output, got {} diffs", result.diffs.len());
}

#[test]
fn check_rust_serde_self_hosting() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec!["serde".to_string()],
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    let check_config = CheckConfig { impl_dir: out_dir.to_str().unwrap().to_string() };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();
    assert!(result.is_ok(), "check should pass with serde Rust output, got {} diffs", result.diffs.len());
}

#[test]
fn check_ts_self_hosting_with_enriched_output() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "ts".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    assert!(out_dir.join("fixtures.ts").exists(), "fixtures.ts should be generated");

    let check_config = CheckConfig { impl_dir: out_dir.to_str().unwrap().to_string() };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();
    assert!(result.is_ok(), "check should pass with enriched TS output, got {} diffs", result.diffs.len());
}

#[test]
fn check_kotlin_self_hosting_with_enriched_output() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "kt".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    assert!(out_dir.join("Fixtures.kt").exists(), "Fixtures.kt should be generated");

    let check_config = CheckConfig { impl_dir: out_dir.to_str().unwrap().to_string() };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();
    if !result.is_ok() { for d in &result.diffs { eprintln!("  {d}"); } }
    assert!(result.is_ok(), "check should pass with enriched Kotlin output, got {} diffs", result.diffs.len());
}

#[test]
fn check_java_self_hosting_with_enriched_output() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "java".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    assert!(out_dir.join("Fixtures.java").exists(), "Fixtures.java should be generated");

    let check_config = CheckConfig { impl_dir: out_dir.to_str().unwrap().to_string() };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();
    if !result.is_ok() { for d in &result.diffs { eprintln!("  {d}"); } }
    assert!(result.is_ok(), "check should pass with enriched Java output, got {} diffs", result.diffs.len());
}
