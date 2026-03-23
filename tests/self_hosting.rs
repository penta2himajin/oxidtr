use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;
use oxidtr::mine;

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

    assert!(files.iter().any(|f| f.path == "models.rs"), "no models.rs");
    assert!(files.iter().any(|f| f.path == "tests.rs"), "no tests.rs");

    let models = files.iter().find(|f| f.path == "models.rs").unwrap();

    // Verify key types from the self-hosting model exist
    assert!(models.content.contains("pub enum Multiplicity"));
    assert!(models.content.contains("pub struct SigDecl"));
    assert!(models.content.contains("pub struct FieldDecl"));
    assert!(models.content.contains("pub struct AlloyModel"));
    assert!(models.content.contains("pub struct OxidtrIR"));
    assert!(models.content.contains("pub struct StructureNode"));
    assert!(models.content.contains("pub struct ConstraintNode"));
    assert!(models.content.contains("pub struct OperationNode"));
    assert!(models.content.contains("pub struct PropertyNode"));
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
    let mined = mine::run("src/", Some("rust")).expect("mine src/ failed");

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

fn extract_sig_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("sig ")
        .or_else(|| line.strip_prefix("one sig "))
        .or_else(|| line.strip_prefix("abstract sig "))?;
    let name: String = rest.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}
