use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;
use std::process::Command;

/// Generate a complete crate from oxidtr.als and verify it compiles.
#[test]
fn self_hosted_crate_compiles() {
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("selfhost_crate");
    let crate_dir = crate_dir.to_str().unwrap();
    std::fs::create_dir_all(format!("{crate_dir}/src")).unwrap();

    // Parse and generate
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    let files = rust::generate(&ir);

    // Write Cargo.toml
    std::fs::write(
        format!("{crate_dir}/Cargo.toml"),
        r#"[package]
name = "oxidtr_generated"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    // Write lib.rs that includes generated modules
    let mut lib_rs = String::new();
    lib_rs.push_str("#[allow(dead_code, unused_variables, unused_imports)]\n");
    lib_rs.push_str("pub mod models;\n");

    let has_helpers = files.iter().any(|f| f.path == "helpers.rs");
    let has_operations = files.iter().any(|f| f.path == "operations.rs");
    let has_tests = files.iter().any(|f| f.path == "tests.rs");

    if has_helpers {
        lib_rs.push_str("pub mod helpers;\n");
    }
    if has_operations {
        lib_rs.push_str("pub mod operations;\n");
    }
    if has_tests {
        lib_rs.push_str("#[allow(dead_code, unused_variables, unused_imports)]\n");
        lib_rs.push_str("mod tests;\n");
    }

    std::fs::write(format!("{crate_dir}/src/lib.rs"), lib_rs).unwrap();

    // Write generated files
    for file in &files {
        let mut content = String::new();
        content.push_str("#![allow(dead_code, unused_variables, unused_imports, non_snake_case)]\n");
        content.push_str(&file.content);
        std::fs::write(format!("{crate_dir}/src/{}", file.path), content).unwrap();
    }

    // Run cargo check
    let output = Command::new("cargo")
        .arg("check")
        .current_dir(crate_dir)
        .output()
        .expect("failed to run cargo check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "cargo check failed!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // Cross-tests contain todo!() stubs by design — humans/AI fill them in.
    // We only verify compilation here; stub tests are not expected to pass.

}

/// Verify the generated crate contains expected structures and tests.
#[test]
fn self_hosted_crate_content_check() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    let files = rust::generate(&ir);

    let models = files.iter().find(|f| f.path == "models.rs").unwrap();
    let tests = files.iter().find(|f| f.path == "tests.rs").unwrap();

    // No invariants.rs should be generated
    assert!(!files.iter().any(|f| f.path == "invariants.rs"),
        "should NOT generate invariants.rs");

    // Key types from oxidtr domain
    assert!(models.content.contains("pub enum Multiplicity"));
    assert!(models.content.contains("pub struct SigDecl"));
    assert!(models.content.contains("pub struct OxidtrIR"));
    assert!(models.content.contains("pub struct StructureNode"));

    // Tests use inlined translated expressions, not invariant function calls
    assert!(tests.content.contains(".iter().all("),
        "tests should contain inlined expressions");

    // Tests should NOT import invariants
    assert!(!tests.content.contains("use crate::invariants::"),
        "tests should NOT import invariants");

    // Tests should NOT contain @alloy comments
    assert!(!tests.content.contains("@alloy:"),
        "tests should NOT contain @alloy comments");

    // Assert property tests exist
    assert!(tests.content.contains("fn no_cyclic_inheritance"));
    assert!(tests.content.contains("fn unique_structure_origins"));
}
