/// Enrichment tests: verify that all backend targets produce enriched output
/// (fixtures, doc comments, Bean Validation, JSON Schema generation + mine round-trip).

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::{rust, typescript, schema};
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::backend::GeneratedFile;
use oxidtr::extract::{schema_extractor, MinedMultiplicity};

// ── Helpers ────────────────────────────────────────────────────────────────

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

fn assert_sig_exists<'a>(
    sigs: &'a [oxidtr::extract::MinedSig],
    name: &str,
) -> &'a oxidtr::extract::MinedSig {
    sigs.iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("expected sig '{name}' not found in mined model"))
}

fn assert_field(sig: &oxidtr::extract::MinedSig, name: &str, mult: MinedMultiplicity, target: &str) {
    let f = sig
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("field '{name}' not found in sig '{}'", sig.name));
    assert_eq!(f.mult, mult, "field {}.{name} multiplicity", sig.name);
    assert_eq!(f.target, target, "field {}.{name} target", sig.name);
}

/// Model with a named fact for doc-comment testing.
const MODEL_WITH_FACT: &str = r#"
sig User { role: one Role, groups: set Group }
sig Role {}
sig Group {}
fact HasRole { all u: User | u.role = u.role }
"#;

// ── Rust enrichment ────────────────────────────────────────────────────────

#[test]
fn rust_generates_fixtures() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = rust::generate(&ir);
    let fixtures = find_file(&files, "fixtures.rs");

    assert!(
        fixtures.contains("pub fn default_user()"),
        "Rust fixtures should contain default_user()"
    );
    assert!(
        fixtures.contains("use crate::models::*"),
        "Rust fixtures should import models"
    );
}

#[test]
fn rust_generates_doc_comments() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = rust::generate(&ir);
    let models = find_file(&files, "models.rs");

    assert!(
        models.contains("/// Invariant: HasRole"),
        "Rust models should contain doc comment for HasRole constraint"
    );
}

// ── TypeScript enrichment ──────────────────────────────────────────────────

#[test]
fn ts_generates_fixtures() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir);
    let fixtures = find_file(&files, "fixtures.ts");

    assert!(
        fixtures.contains("export function defaultUser()"),
        "TS fixtures should contain defaultUser()"
    );
    assert!(
        fixtures.contains("import type * as M from './models'"),
        "TS fixtures should import models"
    );
}

#[test]
fn ts_generates_jsdoc() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir);
    let models = find_file(&files, "models.ts");

    assert!(
        models.contains("@invariant HasRole"),
        "TS models should contain JSDoc @invariant for HasRole"
    );
}

// ── Kotlin enrichment ──────────────────────────────────────────────────────

#[test]
fn kotlin_generates_fixtures() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let fixtures = find_file(&files, "Fixtures.kt");

    assert!(
        fixtures.contains("fun defaultUser()"),
        "Kotlin fixtures should contain defaultUser()"
    );
}

#[test]
fn kotlin_generates_kdoc() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);
    let models = find_file(&files, "Models.kt");

    assert!(
        models.contains("@property Invariant: HasRole"),
        "Kotlin models should contain KDoc @property Invariant for HasRole"
    );
}

// ── Java enrichment ────────────────────────────────────────────────────────

#[test]
fn java_generates_fixtures() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let fixtures = find_file(&files, "Fixtures.java");

    assert!(
        fixtures.contains("static User defaultUser()"),
        "Java fixtures should contain defaultUser()"
    );
}

#[test]
fn java_generates_javadoc() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    assert!(
        models.contains("@invariant HasRole"),
        "Java models should contain Javadoc @invariant for HasRole"
    );
}

#[test]
fn java_generates_notnull() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);
    let models = find_file(&files, "Models.java");

    assert!(
        models.contains("/* @NotNull */"),
        "Java models should contain /* @NotNull */ comment on `one` fields"
    );
    // Specifically: `role` is `one Role` so it should be /* @NotNull */
    assert!(
        models.contains("/* @NotNull */ Role role"),
        "Java record for User should have /* @NotNull */ Role role"
    );
}

// ── JSON Schema generation ─────────────────────────────────────────────────

#[test]
fn schema_generates_valid_structure() {
    let model = parser::parse(MODEL_WITH_FACT).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = schema::generate(&ir);

    assert_eq!(file.path, "schemas.json");
    assert!(file.content.contains("\"$schema\""));
    assert!(file.content.contains("\"User\""));
    assert!(file.content.contains("\"Role\""));
    assert!(file.content.contains("\"Group\""));
}

// ── JSON Schema mine round-trip ────────────────────────────────────────────

#[test]
fn schema_mine_round_trip_simple() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = schema::generate(&ir);

    let mined = schema_extractor::extract(&file.content);

    let user = assert_sig_exists(&mined.sigs, "User");
    assert_field(user, "group", MinedMultiplicity::Lone, "Group");
    assert_field(user, "roles", MinedMultiplicity::Set, "Role");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

#[test]
fn schema_mine_round_trip_enum() {
    let alloy_src =
        "abstract sig Status {}\none sig Active extends Status {}\none sig Inactive extends Status {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = schema::generate(&ir);

    let mined = schema_extractor::extract(&file.content);

    let status = assert_sig_exists(&mined.sigs, "Status");
    assert!(status.is_abstract, "Status should be abstract");
    let active = assert_sig_exists(&mined.sigs, "Active");
    assert_eq!(active.parent.as_deref(), Some("Status"));
    let inactive = assert_sig_exists(&mined.sigs, "Inactive");
    assert_eq!(inactive.parent.as_deref(), Some("Status"));
}

#[test]
fn schema_mine_round_trip_discriminated_union() {
    let alloy_src = r#"
abstract sig Expr {}
sig Literal extends Expr {}
sig BinOp extends Expr { left: one Expr, right: one Expr }
"#;
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = schema::generate(&ir);

    let mined = schema_extractor::extract(&file.content);

    let expr = assert_sig_exists(&mined.sigs, "Expr");
    assert!(expr.is_abstract, "Expr should be abstract");
    let binop = assert_sig_exists(&mined.sigs, "BinOp");
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 2);
    assert_field(binop, "left", MinedMultiplicity::One, "Expr");
    assert_field(binop, "right", MinedMultiplicity::One, "Expr");
}

#[test]
fn schema_mine_round_trip_one_field() {
    let alloy_src = "sig A { b: one B }\nsig B {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = schema::generate(&ir);

    let mined = schema_extractor::extract(&file.content);

    let a = assert_sig_exists(&mined.sigs, "A");
    assert_field(a, "b", MinedMultiplicity::One, "B");
}

// ── Self-hosting schema round-trip ─────────────────────────────────────────

#[test]
fn schema_self_hosting_mine_round_trip() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = schema::generate(&ir);

    let mined = schema_extractor::extract(&file.content);

    // Verify key structures survive the round-trip
    assert_sig_exists(&mined.sigs, "SigDecl");
    assert_sig_exists(&mined.sigs, "OxidtrIR");
    assert_sig_exists(&mined.sigs, "StructureNode");

    // Verify Multiplicity enum
    let mult = assert_sig_exists(&mined.sigs, "Multiplicity");
    assert!(mult.is_abstract, "Multiplicity should be abstract");

    // Verify field multiplicities on SigDecl
    let sig_decl = assert_sig_exists(&mined.sigs, "SigDecl");
    assert_field(sig_decl, "fields", MinedMultiplicity::Set, "FieldDecl");
    assert_field(sig_decl, "parent", MinedMultiplicity::Lone, "SigDecl");

    // Verify OxidtrIR fields
    let ir_node = assert_sig_exists(&mined.sigs, "OxidtrIR");
    assert_field(
        ir_node,
        "structures",
        MinedMultiplicity::Set,
        "StructureNode",
    );
    assert_field(
        ir_node,
        "constraints",
        MinedMultiplicity::Set,
        "ConstraintNode",
    );
}

// ── Full round-trip: all targets ───────────────────────────────────────────

#[test]
fn full_round_trip_all_targets() {
    let alloy_src = r#"
sig User { group: lone Group, roles: set Role }
sig Group {}
sig Role {}
fact HasRole { all u: User | u.role = u.role }
"#;
    let model = parser::parse(alloy_src).unwrap();
    let ir = ir::lower(&model).unwrap();

    // Generate all targets
    let rust_files = rust::generate(&ir);
    let ts_files = typescript::generate(&ir);
    let kt_files = kotlin::generate(&ir);
    let java_files = java::generate(&ir);
    let schema_file = schema::generate(&ir);

    // Verify fixtures exist for all targets
    assert!(
        find_file(&rust_files, "fixtures.rs").contains("default_user"),
        "Rust fixtures"
    );
    assert!(
        find_file(&ts_files, "fixtures.ts").contains("defaultUser"),
        "TS fixtures"
    );
    assert!(
        find_file(&kt_files, "Fixtures.kt").contains("defaultUser"),
        "Kotlin fixtures"
    );
    assert!(
        find_file(&java_files, "Fixtures.java").contains("defaultUser"),
        "Java fixtures"
    );

    // Verify doc comments for all targets
    assert!(
        find_file(&rust_files, "models.rs").contains("Invariant: HasRole"),
        "Rust doc comments"
    );
    assert!(
        find_file(&ts_files, "models.ts").contains("@invariant HasRole"),
        "TS JSDoc"
    );
    assert!(
        find_file(&kt_files, "Models.kt").contains("@property Invariant: HasRole"),
        "Kotlin KDoc"
    );
    assert!(
        find_file(&java_files, "Models.java").contains("@invariant HasRole"),
        "Java Javadoc"
    );

    // Verify schema mine round-trip recovers sigs
    let mined = schema_extractor::extract(&schema_file.content);
    assert_sig_exists(&mined.sigs, "User");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

// ── Self-hosting enrichment verification ───────────────────────────────────

#[test]
fn self_hosting_enriched_output() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();

    // Rust: fixtures generated
    let rust_files = rust::generate(&ir);
    let rust_fixtures = find_file(&rust_files, "fixtures.rs");
    assert!(
        rust_fixtures.contains("pub fn default_"),
        "Rust self-hosting should generate fixture functions"
    );

    // TS: fixtures generated
    let ts_files = typescript::generate(&ir);
    let ts_fixtures = find_file(&ts_files, "fixtures.ts");
    assert!(
        ts_fixtures.contains("export function default"),
        "TS self-hosting should generate fixture functions"
    );

    // Kotlin: fixtures generated
    let kt_files = kotlin::generate(&ir);
    let kt_fixtures = find_file(&kt_files, "Fixtures.kt");
    assert!(
        kt_fixtures.contains("fun default"),
        "Kotlin self-hosting should generate fixture functions"
    );

    // Java: fixtures generated with /* @NotNull */
    let java_files = java::generate(&ir);
    let java_fixtures = find_file(&java_files, "Fixtures.java");
    assert!(
        java_fixtures.contains("static ") && java_fixtures.contains("default"),
        "Java self-hosting should generate fixture functions"
    );
    let java_models = find_file(&java_files, "Models.java");
    assert!(
        java_models.contains("/* @NotNull */"),
        "Java self-hosting should have /* @NotNull */ comments"
    );

    // Schema: generated and mineable
    let schema_file = schema::generate(&ir);
    assert!(
        schema_file.content.contains("\"definitions\""),
        "Schema should have definitions"
    );
    let mined = schema_extractor::extract(&schema_file.content);
    assert!(
        mined.sigs.len() >= 5,
        "Schema mine should recover at least 5 sigs, got {}",
        mined.sigs.len()
    );
}
