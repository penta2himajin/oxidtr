use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;
use oxidtr::extract;

const SELF_MODEL: &str = include_str!("../models/oxidtr.als");

#[test]
fn self_model_parses() {
    let model = parser::parse(SELF_MODEL);
    assert!(model.is_ok(), "self-model parse failed: {:?}", model.err());
    let model = model.unwrap();
    assert!(!model.sigs.is_empty(), "no sigs parsed");
    assert!(!model.facts.is_empty(), "no facts parsed");
    assert!(!model.asserts.is_empty(), "no asserts parsed");
}

#[test]
fn self_model_lowers() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir = ir::lower(&model);
    assert!(ir.is_ok(), "self-model lowering failed: {:?}", ir.err());
    let ir = ir.unwrap();
    assert!(!ir.structures.is_empty());
    assert!(!ir.constraints.is_empty());
    assert!(!ir.properties.is_empty());
}

#[test]
fn self_model_generates_rust() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = rust::generate(&ir);

    // Modular layout: lib.rs + module dirs; Flat layout: models.rs
    let has_modular = files.iter().any(|f| f.path == "lib.rs");
    let has_flat = files.iter().any(|f| f.path == "models.rs");
    assert!(has_modular || has_flat, "should generate lib.rs (modular) or models.rs (flat)");
    assert!(files.iter().any(|f| f.path == "tests.rs"), "no tests.rs");

    // Concatenate all model content for assertion checks
    let all_models: String = if has_flat {
        files.iter().find(|f| f.path == "models.rs").unwrap().content.clone()
    } else {
        files.iter()
            .filter(|f| f.path.ends_with(".rs") && !f.path.ends_with("mod.rs")
                && !f.path.ends_with("lib.rs") && f.path != "tests.rs"
                && f.path != "fixtures.rs" && f.path != "helpers.rs"
                && f.path != "operations.rs" && f.path != "newtypes.rs")
            .map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n")
    };

    // Verify key types from the self-hosting model exist
    assert!(all_models.contains("pub enum Multiplicity"));
    assert!(all_models.contains("pub struct SigDecl"));
    assert!(all_models.contains("pub struct FieldDecl"));
    assert!(all_models.contains("pub struct AlloyModel"));
    assert!(all_models.contains("pub struct OxidtrIR"));
    assert!(all_models.contains("pub struct StructureNode"));
    assert!(all_models.contains("pub struct ConstraintNode"));
    assert!(all_models.contains("pub struct OperationNode"));
    assert!(all_models.contains("pub struct PropertyNode"));
}

#[test]
fn self_model_generated_rust_compiles_check() {
    // Verify the generated code is syntactically valid Rust
    // by checking for balanced braces and basic structure
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = rust::generate(&ir);

    for file in &files {
        let open = file.content.matches('{').count();
        let close = file.content.matches('}').count();
        assert_eq!(open, close, "unbalanced braces in {}", file.path);
    }
}

const DOMAIN_MODEL: &str = include_str!("../models/oxidtr-domain.als");
const INTERNAL_MODEL: &str = include_str!("../models/oxidtr-internal.als");

/// Every sig that `oxidtr mine src/` extracts from oxidtr's own source
/// must have a corresponding sig in the domain or internal Alloy model.
/// This is the self-hosting proof: oxidtr can fully model itself.
#[test]
fn self_hosting_mine_sig_coverage_100_percent() {
    // Collect sig names from both models
    let model_sigs: std::collections::HashSet<String> = DOMAIN_MODEL.lines()
        .chain(INTERNAL_MODEL.lines())
        .filter_map(|line| extract_sig_name(line.trim()))
        .collect();

    // Mine oxidtr's own source
    let mined = extract::run("src/", Some("rust")).expect("mine src/ failed");

    let mut missing: Vec<&str> = Vec::new();
    for sig in &mined.sigs {
        if !model_sigs.contains(&sig.name) {
            missing.push(&sig.name);
        }
    }

    assert!(missing.is_empty(),
        "mined sigs not covered by model ({} missing): {:?}",
        missing.len(), missing);

    // Sanity: we should have a substantial number of sigs
    assert!(mined.sigs.len() > 100,
        "expected 100+ mined sigs, got {}", mined.sigs.len());
}

/// Verify the generated crate contains expected structures and tests.
/// (Moved from self_hosting_compile.rs — this is a content check, not compilation.)
#[test]
fn self_hosted_crate_content_check() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = rust::generate(&ir);

    // Concatenate all model content for modular or flat layout
    let all_models: String = if let Some(m) = files.iter().find(|f| f.path == "models.rs") {
        m.content.clone()
    } else {
        files.iter()
            .filter(|f| f.path.ends_with(".rs") && !f.path.ends_with("mod.rs")
                && !f.path.ends_with("lib.rs") && f.path != "tests.rs"
                && f.path != "fixtures.rs" && f.path != "helpers.rs"
                && f.path != "operations.rs" && f.path != "newtypes.rs")
            .map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n")
    };
    let tests = files.iter().find(|f| f.path == "tests.rs").unwrap();

    // No invariants.rs should be generated
    assert!(!files.iter().any(|f| f.path == "invariants.rs"),
        "should NOT generate invariants.rs");

    // Key types from oxidtr domain
    assert!(all_models.contains("pub enum Multiplicity"));
    assert!(all_models.contains("pub struct SigDecl"));
    assert!(all_models.contains("pub struct OxidtrIR"));
    assert!(all_models.contains("pub struct StructureNode"));

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

fn extract_sig_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("sig ")
        .or_else(|| line.strip_prefix("one sig "))
        .or_else(|| line.strip_prefix("abstract sig "))?;
    let name: String = rest.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}
