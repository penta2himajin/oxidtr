use oxidtr::extract::renderer::{render_files, RenderedFile};
use oxidtr::extract::{MinedModel, MinedSig, MinedField, MinedMultiplicity};

fn sig_with_module(name: &str, module: Option<&str>, parent: Option<&str>, fields: Vec<MinedField>) -> MinedSig {
    MinedSig {
        name: name.to_string(),
        fields,
        is_abstract: false,
        is_var: false,
        parent: parent.map(|s| s.to_string()),
        source_location: "test".to_string(),
        intersection_of: vec![],
        module: module.map(|s| s.to_string()),
    }
}

fn field(name: &str, target: &str) -> MinedField {
    MinedField {
        name: name.to_string(),
        is_var: false,
        mult: MinedMultiplicity::One,
        target: target.to_string(),
        raw_union_type: None,
    }
}

fn find_file<'a>(files: &'a [RenderedFile], path: &str) -> &'a RenderedFile {
    files
        .iter()
        .find(|f| f.path.to_string_lossy() == path)
        .unwrap_or_else(|| panic!("expected file {path} in {:?}",
            files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()))
}

#[test]
fn no_modules_returns_single_main_als() {
    let model = MinedModel {
        sigs: vec![sig_with_module("A", None, None, vec![])],
        fact_candidates: vec![],
    };
    let files = render_files(&model);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path.to_string_lossy(), "main.als");
    assert!(files[0].content.contains("sig A"));
}

#[test]
fn single_module_produces_module_file_and_main() {
    let model = MinedModel {
        sigs: vec![sig_with_module("Leaf", Some("foo/bar"), None, vec![])],
        fact_candidates: vec![],
    };
    let files = render_files(&model);
    let main = find_file(&files, "oxidtr.als");
    assert!(main.content.contains("open foo/bar"));
    let sub = find_file(&files, "foo/bar.als");
    assert!(sub.content.starts_with("module foo/bar"));
    assert!(sub.content.contains("sig Leaf"));
}

#[test]
fn cross_module_field_reference_emits_open_in_dependent_file() {
    let model = MinedModel {
        sigs: vec![
            sig_with_module("Base", Some("oxidtr/ast"), None, vec![]),
            sig_with_module(
                "IR",
                Some("oxidtr/ir"),
                None,
                vec![field("origin", "Base")],
            ),
        ],
        fact_candidates: vec![],
    };
    let files = render_files(&model);
    let ir_file = find_file(&files, "oxidtr/ir.als");
    assert!(
        ir_file.content.contains("open oxidtr/ast"),
        "ir.als should import ast:\n{}",
        ir_file.content
    );
    let ast_file = find_file(&files, "oxidtr/ast.als");
    // ast.als should not depend on ir (unidirectional ref)
    assert!(
        !ast_file.content.contains("open oxidtr/ir"),
        "ast.als must not import ir"
    );
}

#[test]
fn parent_sig_in_other_module_emits_open() {
    let model = MinedModel {
        sigs: vec![
            sig_with_module("Token", Some("lex"), None, vec![]),
            sig_with_module("Ident", Some("parse"), Some("Token"), vec![]),
        ],
        fact_candidates: vec![],
    };
    let files = render_files(&model);
    let parse = find_file(&files, "parse.als");
    assert!(parse.content.contains("open lex"));
}

#[test]
fn main_file_contains_ungrouped_sigs_and_opens() {
    let model = MinedModel {
        sigs: vec![
            sig_with_module("Leaf", Some("sub"), None, vec![]),
            sig_with_module("Top", None, None, vec![]),
        ],
        fact_candidates: vec![],
    };
    let files = render_files(&model);
    let main = find_file(&files, "oxidtr.als");
    assert!(main.content.contains("open sub"));
    assert!(main.content.contains("sig Top"));
}

#[test]
fn rendered_multi_file_output_parses_via_parse_from_path() {
    // Round-trip: render multi-file, write to temp dir, parse_from_path, confirm sigs recovered.
    use std::fs;
    let model = MinedModel {
        sigs: vec![
            sig_with_module("A", Some("pkg/one"), None, vec![field("x", "B")]),
            sig_with_module("B", Some("pkg/two"), None, vec![]),
        ],
        fact_candidates: vec![],
    };
    let files = render_files(&model);

    let dir = std::env::temp_dir().join(format!("oxidtr-render-rt-{}", std::process::id()));
    if dir.exists() { let _ = fs::remove_dir_all(&dir); }
    fs::create_dir_all(&dir).unwrap();
    for f in &files {
        let full = dir.join(&f.path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, &f.content).unwrap();
    }

    let parsed = oxidtr::parser::parse_from_path(&dir.join("oxidtr.als"))
        .expect("parse_from_path succeeds");
    let names: std::collections::HashSet<&str> =
        parsed.sigs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains("A"));
    assert!(names.contains("B"));
}
