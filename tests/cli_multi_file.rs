//! End-to-end tests for multi-file `.als` input/output across generate/check/extract.

use oxidtr::generate::{load_model, run as generate_run, GenerateConfig};
use oxidtr::check::{run as check_run, CheckConfig};
use std::fs;
use std::path::PathBuf;

fn fresh_dir(name: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("oxidtr-cli-mf-{}-{}", name, std::process::id()));
    if d.exists() { let _ = fs::remove_dir_all(&d); }
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn load_model_resolves_open_from_file_input() {
    let root = fresh_dir("load-file");
    let sub = root.join("oxidtr");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        root.join("oxidtr.als"),
        "module oxidtr\nopen oxidtr/ast\n\nsig Top {}\n",
    ).unwrap();
    fs::write(
        sub.join("ast.als"),
        "module oxidtr/ast\n\nsig Node {}\n",
    ).unwrap();

    let m = load_model(root.join("oxidtr.als").to_str().unwrap()).expect("load");
    let names: std::collections::HashSet<&str> =
        m.sigs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains("Top"));
    assert!(names.contains("Node"));
}

#[test]
fn load_model_resolves_from_directory_convention() {
    let root = fresh_dir("load-dir");
    let sub = root.join("oxidtr");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        root.join("oxidtr.als"),
        "module oxidtr\nopen oxidtr/ast\n\nsig Top {}\n",
    ).unwrap();
    fs::write(
        sub.join("ast.als"),
        "module oxidtr/ast\n\nsig Node {}\n",
    ).unwrap();

    // Passing the directory should discover `oxidtr.als` as the main file.
    let m = load_model(root.to_str().unwrap()).expect("load");
    let names: std::collections::HashSet<&str> =
        m.sigs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains("Top"));
    assert!(names.contains("Node"));
}

#[test]
fn generate_uses_load_model_for_multi_file_input() {
    let root = fresh_dir("gen-mf");
    let sub = root.join("oxidtr");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        root.join("oxidtr.als"),
        "module oxidtr\nopen oxidtr/ast\n\nsig Top { node: one Node }\n",
    ).unwrap();
    fs::write(
        sub.join("ast.als"),
        "module oxidtr/ast\n\nsig Node {}\n",
    ).unwrap();

    let out = root.join("generated");
    let config = GenerateConfig::new("rust", out.to_str().unwrap());
    let result = generate_run(root.join("oxidtr.als").to_str().unwrap(), &config)
        .expect("generate");
    assert!(!result.files_written.is_empty(),
        "generate should produce files for multi-file input");
    // Ensure both sigs appear somewhere in generated output
    let all: String = result.files_written.iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect();
    assert!(all.contains("Top"), "Top sig missing in output");
    assert!(all.contains("Node"), "Node sig missing in output");
}

#[test]
fn check_accepts_multi_file_model() {
    let root = fresh_dir("check-mf");
    let sub = root.join("oxidtr");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        root.join("oxidtr.als"),
        "module oxidtr\nopen oxidtr/ast\n\nsig Top { node: one Node }\n",
    ).unwrap();
    fs::write(
        sub.join("ast.als"),
        "module oxidtr/ast\n\nsig Node {}\n",
    ).unwrap();

    // Generate impl first
    let out = root.join("generated");
    let config = GenerateConfig::new("rust", out.to_str().unwrap());
    generate_run(root.join("oxidtr.als").to_str().unwrap(), &config)
        .expect("generate before check");

    let cfg = CheckConfig { impl_dir: out.to_str().unwrap().to_string() };
    let result = check_run(root.join("oxidtr.als").to_str().unwrap(), &cfg)
        .expect("check");
    assert!(result.is_ok(),
        "check should be clean after round-trip; diffs: {:?}", result.diffs);
}

#[test]
fn extract_directory_output_writes_multiple_files() {
    // Build a small Rust crate with module subdirs, extract, then ensure
    // render_files produces >=2 files under `-o <dir>`.
    let root = fresh_dir("extract-dir");
    let src = root.join("src");
    let ast = src.join("ast");
    let ir = src.join("ir");
    fs::create_dir_all(&ast).unwrap();
    fs::create_dir_all(&ir).unwrap();
    fs::write(ast.join("types.rs"), "pub struct Node { pub name: String }\n").unwrap();
    fs::write(ir.join("types.rs"), "pub struct IRNode { pub origin: Node }\n").unwrap();

    let merge = oxidtr::extract::run_merge(src.to_str().unwrap(), Some("rust"))
        .expect("run_merge");
    let files = oxidtr::extract::renderer::render_files(&merge.model);
    let has_ast = files.iter().any(|f| f.path.to_string_lossy().contains("ast"));
    let has_ir = files.iter().any(|f| f.path.to_string_lossy().contains("ir"));
    assert!(has_ast, "expected ast file in {:?}",
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>());
    assert!(has_ir, "expected ir file in {:?}",
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>());
}
