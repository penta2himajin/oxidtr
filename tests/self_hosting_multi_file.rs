//! End-to-end self-hosting verification using the spec-compliant multi-file
//! variant (`models/oxidtr-split.als` + `models/oxidtr/*.als`).

use oxidtr::{check, generate, ir, parser};
use std::path::Path;

#[test]
fn split_model_parses_via_parse_from_path() {
    let model = parser::parse_from_path(Path::new("models/oxidtr-split.als"))
        .expect("parse_from_path resolves all opens");
    assert!(
        model.sigs.len() > 50,
        "expected 50+ sigs after merging all sub-modules, got {}",
        model.sigs.len()
    );
    // Imports should be recorded on the top-level model.
    let import_paths: Vec<&str> = model.imports.iter().map(|i| i.path.as_str()).collect();
    assert!(import_paths.contains(&"oxidtr/ast"));
    assert!(import_paths.contains(&"oxidtr/ir"));
    assert!(import_paths.contains(&"oxidtr/analysis"));
    assert!(import_paths.contains(&"oxidtr/validated"));
}

#[test]
fn split_model_preserves_qualified_module_names() {
    let model = parser::parse_from_path(Path::new("models/oxidtr-split.als"))
        .expect("parse_from_path");
    // SigDecl from ast should carry module "oxidtr/ast"
    let sig_decl = model
        .sigs
        .iter()
        .find(|s| s.name == "SigDecl")
        .expect("SigDecl should be present");
    assert_eq!(sig_decl.module.as_deref(), Some("oxidtr/ast"));

    // StructureNode from ir should carry module "oxidtr/ir"
    let structure_node = model
        .sigs
        .iter()
        .find(|s| s.name == "StructureNode")
        .expect("StructureNode should be present");
    assert_eq!(structure_node.module.as_deref(), Some("oxidtr/ir"));
}

#[test]
fn split_model_lowers_to_ir() {
    let model = parser::parse_from_path(Path::new("models/oxidtr-split.als"))
        .expect("parse_from_path");
    let lowered = ir::lower(&model).expect("lower");
    assert!(lowered.structures.len() > 50);
}

#[test]
fn split_model_generate_round_trip_rust() {
    let out = std::env::temp_dir().join(format!("oxidtr-split-gen-{}", std::process::id()));
    if out.exists() { let _ = std::fs::remove_dir_all(&out); }

    let config = generate::GenerateConfig::new("rust", out.to_str().unwrap());
    generate::run("models/oxidtr-split.als", &config)
        .expect("generate rust from split model");

    let check_config = check::CheckConfig { impl_dir: out.to_str().unwrap().to_string() };
    let result = check::run("models/oxidtr-split.als", &check_config)
        .expect("check round-trip");
    assert!(
        result.is_ok(),
        "split model round-trip should be clean; diffs: {:?}",
        result.diffs
    );
}

#[test]
fn legacy_and_split_models_agree_on_sig_set() {
    // Both the legacy flat `oxidtr.als` and the new split variant should
    // describe the same structural model.
    let legacy = parser::parse_from_path(Path::new("models/oxidtr.als"))
        .expect("parse legacy oxidtr.als");
    let split = parser::parse_from_path(Path::new("models/oxidtr-split.als"))
        .expect("parse split variant");

    let legacy_names: std::collections::BTreeSet<&str> =
        legacy.sigs.iter().map(|s| s.name.as_str()).collect();
    let split_names: std::collections::BTreeSet<&str> =
        split.sigs.iter().map(|s| s.name.as_str()).collect();

    let only_in_legacy: Vec<&&str> = legacy_names.difference(&split_names).collect();
    let only_in_split: Vec<&&str> = split_names.difference(&legacy_names).collect();

    assert!(
        only_in_legacy.is_empty() && only_in_split.is_empty(),
        "sig sets diverge — only_in_legacy={only_in_legacy:?}, only_in_split={only_in_split:?}"
    );
}
