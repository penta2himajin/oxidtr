/// Round-trip tests: Alloy → generate → mine → compare to original IR.
/// Verifies structural preservation through the full pipeline.

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::{rust, typescript};
use oxidtr::mine::{rust_extractor, ts_extractor, MinedMultiplicity};
use oxidtr::check::{self, CheckConfig};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};

// ── Helpers ────────────────────────────────────────────────────────────────

fn assert_sig_exists<'a>(sigs: &'a [oxidtr::mine::MinedSig], name: &str) -> &'a oxidtr::mine::MinedSig {
    sigs.iter().find(|s| s.name == name)
        .unwrap_or_else(|| panic!("expected sig '{name}' not found in mined model"))
}

fn assert_field(sig: &oxidtr::mine::MinedSig, name: &str, mult: MinedMultiplicity, target: &str) {
    let f = sig.fields.iter().find(|f| f.name == name)
        .unwrap_or_else(|| panic!("field '{name}' not found in sig '{}'", sig.name));
    assert_eq!(f.mult, mult, "field {}.{name} multiplicity", sig.name);
    assert_eq!(f.target, target, "field {}.{name} target", sig.name);
}

// ── Simple model round-trips ───────────────────────────────────────────────

#[test]
fn round_trip_rust_simple_model() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = rust::generate(&ir_obj);
    let models_rs = files.iter().find(|f| f.path == "models.rs").unwrap();

    let mined = rust_extractor::extract(&models_rs.content);

    assert_eq!(mined.sigs.len(), 3);
    let user = assert_sig_exists(&mined.sigs, "User");
    assert_field(user, "group", MinedMultiplicity::Lone, "Group");
    assert_field(user, "roles", MinedMultiplicity::Set, "Role");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

#[test]
fn round_trip_ts_simple_model() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir_obj);
    let models_ts = files.iter().find(|f| f.path == "models.ts").unwrap();

    let mined = ts_extractor::extract(&models_ts.content);

    let user = assert_sig_exists(&mined.sigs, "User");
    assert_field(user, "group", MinedMultiplicity::Lone, "Group");
    assert_field(user, "roles", MinedMultiplicity::Set, "Role");
    assert_sig_exists(&mined.sigs, "Group");
    assert_sig_exists(&mined.sigs, "Role");
}

#[test]
fn round_trip_rust_enum_model() {
    let alloy_src = "abstract sig Status {}\nsig Active extends Status {}\nsig Inactive extends Status {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = rust::generate(&ir_obj);
    let models_rs = files.iter().find(|f| f.path == "models.rs").unwrap();

    let mined = rust_extractor::extract(&models_rs.content);

    let status = assert_sig_exists(&mined.sigs, "Status");
    assert!(status.is_abstract);
    let active = assert_sig_exists(&mined.sigs, "Active");
    assert_eq!(active.parent.as_deref(), Some("Status"));
    let inactive = assert_sig_exists(&mined.sigs, "Inactive");
    assert_eq!(inactive.parent.as_deref(), Some("Status"));
}

#[test]
fn round_trip_ts_enum_model() {
    let alloy_src = "abstract sig Status {}\none sig Active extends Status {}\none sig Inactive extends Status {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir_obj);
    let models_ts = files.iter().find(|f| f.path == "models.ts").unwrap();

    let mined = ts_extractor::extract(&models_ts.content);

    let status = assert_sig_exists(&mined.sigs, "Status");
    assert!(status.is_abstract);
    let active = assert_sig_exists(&mined.sigs, "Active");
    assert_eq!(active.parent.as_deref(), Some("Status"));
}

#[test]
fn round_trip_rust_enum_with_fields() {
    let alloy_src = r#"
abstract sig Expr {}
sig Literal extends Expr {}
sig BinOp extends Expr { left: one Expr, right: one Expr }
"#;
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = rust::generate(&ir_obj);
    let models_rs = files.iter().find(|f| f.path == "models.rs").unwrap();

    let mined = rust_extractor::extract(&models_rs.content);

    let expr = assert_sig_exists(&mined.sigs, "Expr");
    assert!(expr.is_abstract);
    let binop = assert_sig_exists(&mined.sigs, "BinOp");
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 2);
    assert_field(binop, "left", MinedMultiplicity::One, "Expr");
}

#[test]
fn round_trip_ts_enum_with_fields() {
    let alloy_src = r#"
abstract sig Expr {}
sig Literal extends Expr {}
sig BinOp extends Expr { left: one Expr, right: one Expr }
"#;
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir_obj);
    let models_ts = files.iter().find(|f| f.path == "models.ts").unwrap();

    let mined = ts_extractor::extract(&models_ts.content);

    let expr = assert_sig_exists(&mined.sigs, "Expr");
    assert!(expr.is_abstract);
    let binop = assert_sig_exists(&mined.sigs, "BinOp");
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 2);
    assert_field(binop, "left", MinedMultiplicity::One, "Expr");
}

// ── Mine output re-parseable as Alloy ──────────────────────────────────────

#[test]
fn round_trip_rust_mine_renders_parseable_alloy() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = rust::generate(&ir_obj);
    let models_rs = files.iter().find(|f| f.path == "models.rs").unwrap();

    let mined = rust_extractor::extract(&models_rs.content);
    let rendered = oxidtr::mine::renderer::render(&mined);

    let reparsed = parser::parse(&rendered);
    assert!(reparsed.is_ok(), "mine → render → parse should succeed:\n{rendered}");
    assert_eq!(reparsed.unwrap().sigs.len(), 3);
}

#[test]
fn round_trip_ts_mine_renders_parseable_alloy() {
    let alloy_src = "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}";
    let model = parser::parse(alloy_src).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir_obj);
    let models_ts = files.iter().find(|f| f.path == "models.ts").unwrap();

    let mined = ts_extractor::extract(&models_ts.content);
    let rendered = oxidtr::mine::renderer::render(&mined);

    let reparsed = parser::parse(&rendered);
    assert!(reparsed.is_ok(), "mine → render → parse should succeed:\n{rendered}");
    assert_eq!(reparsed.unwrap().sigs.len(), 3);
}

// ── Self-hosting round-trip ────────────────────────────────────────────────

#[test]
fn self_hosting_round_trip_rust() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = rust::generate(&ir_obj);
    let models_rs = files.iter().find(|f| f.path == "models.rs").unwrap();

    let mined = rust_extractor::extract(&models_rs.content);

    // Verify key structures survive the round-trip
    assert_sig_exists(&mined.sigs, "SigDecl");
    assert_sig_exists(&mined.sigs, "FieldDecl");
    assert_sig_exists(&mined.sigs, "OxidtrIR");
    assert_sig_exists(&mined.sigs, "StructureNode");
    assert_sig_exists(&mined.sigs, "ConstraintNode");

    // Verify Multiplicity enum and its variants
    let mult = assert_sig_exists(&mined.sigs, "Multiplicity");
    assert!(mult.is_abstract);

    // Verify Expr discriminated union
    let expr = assert_sig_exists(&mined.sigs, "Expr");
    assert!(expr.is_abstract);

    // Verify field multiplicities
    let sig_decl = assert_sig_exists(&mined.sigs, "SigDecl");
    assert_field(sig_decl, "fields", MinedMultiplicity::Set, "FieldDecl");
    assert_field(sig_decl, "parent", MinedMultiplicity::Lone, "SigDecl");

    let ir_node = assert_sig_exists(&mined.sigs, "OxidtrIR");
    assert_field(ir_node, "structures", MinedMultiplicity::Set, "StructureNode");
    assert_field(ir_node, "constraints", MinedMultiplicity::Set, "ConstraintNode");
}

#[test]
fn self_hosting_round_trip_ts() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir_obj);
    let models_ts = files.iter().find(|f| f.path == "models.ts").unwrap();

    let mined = ts_extractor::extract(&models_ts.content);

    // Same structural checks as Rust
    assert_sig_exists(&mined.sigs, "SigDecl");
    assert_sig_exists(&mined.sigs, "FieldDecl");
    assert_sig_exists(&mined.sigs, "OxidtrIR");
    assert_sig_exists(&mined.sigs, "StructureNode");

    let mult = assert_sig_exists(&mined.sigs, "Multiplicity");
    assert!(mult.is_abstract);

    let expr = assert_sig_exists(&mined.sigs, "Expr");
    assert!(expr.is_abstract);

    let sig_decl = assert_sig_exists(&mined.sigs, "SigDecl");
    assert_field(sig_decl, "fields", MinedMultiplicity::Set, "FieldDecl");
    assert_field(sig_decl, "parent", MinedMultiplicity::Lone, "SigDecl");

    let ir_node = assert_sig_exists(&mined.sigs, "OxidtrIR");
    assert_field(ir_node, "structures", MinedMultiplicity::Set, "StructureNode");
    assert_field(ir_node, "constraints", MinedMultiplicity::Set, "ConstraintNode");
}

// ── Check command TS support ───────────────────────────────────────────────

#[test]
fn check_ts_self_hosting() {
    let tmp = tempfile::tempdir().unwrap();
    let out_dir = tmp.path().join("output");

    let config = GenerateConfig {
        target: "ts".to_string(),
        output_dir: out_dir.to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
    };
    generate::run("models/oxidtr.als", &config).unwrap();

    let check_config = CheckConfig {
        impl_dir: out_dir.to_str().unwrap().to_string(),
    };
    let result = check::run("models/oxidtr.als", &check_config).unwrap();

    if !result.is_ok() {
        for d in &result.diffs {
            eprintln!("  {d}");
        }
    }
    assert!(result.is_ok(), "check TS self-hosting should have 0 diffs, got {}", result.diffs.len());
}
