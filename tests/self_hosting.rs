use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;

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
