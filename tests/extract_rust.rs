use oxidtr::extract::rust_extractor;
use oxidtr::extract::renderer;
use oxidtr::extract::{MinedMultiplicity, Confidence, resolve_external_types};

#[test]
fn mine_rust_struct_to_sig() {
    let src = r#"
pub struct User {
    pub name: String,
    pub age: i32,
}
"#;
    let mined = rust_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 1);
    assert_eq!(mined.sigs[0].name, "User");
    assert_eq!(mined.sigs[0].fields.len(), 2);
    assert!(!mined.sigs[0].is_abstract);
}

#[test]
fn mine_rust_option_to_lone() {
    let src = r#"
pub struct Node {
    pub parent: Option<Node>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.name, "parent");
    assert_eq!(f.mult, MinedMultiplicity::Lone);
    assert_eq!(f.target, "Node");
}

#[test]
fn mine_rust_option_box_to_lone() {
    let src = r#"
pub struct Node {
    pub parent: Option<Box<Node>>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Lone);
    assert_eq!(f.target, "Node");
}

#[test]
fn mine_rust_vec_to_seq() {
    let src = r#"
pub struct Group {
    pub members: Vec<User>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Seq);
    assert_eq!(f.target, "User");
}

#[test]
fn mine_rust_btreeset_to_set() {
    let src = r#"
pub struct Group {
    pub members: BTreeSet<User>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Set);
    assert_eq!(f.target, "User");
}

#[test]
fn mine_rust_enum_to_abstract_sig() {
    let src = r#"
pub enum Status {
    Active,
    Inactive,
}
"#;
    let mined = rust_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 3);
    assert!(mined.sigs[0].is_abstract);
    assert_eq!(mined.sigs[0].name, "Status");
    assert_eq!(mined.sigs[1].name, "Active");
    assert_eq!(mined.sigs[1].parent.as_deref(), Some("Status"));
    assert_eq!(mined.sigs[2].name, "Inactive");
    assert_eq!(mined.sigs[2].parent.as_deref(), Some("Status"));
}

#[test]
fn mine_rust_enum_with_fields() {
    let src = r#"
pub enum Expr {
    Literal,
    BinOp {
        left: Box<Expr>,
        right: Box<Expr>,
    },
}
"#;
    let mined = rust_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 3); // Expr, Literal, BinOp
    let binop = &mined.sigs[2];
    assert_eq!(binop.name, "BinOp");
    assert_eq!(binop.fields.len(), 2);
    assert_eq!(binop.fields[0].name, "left");
    assert_eq!(binop.fields[0].target, "Expr");
}

#[test]
fn mine_rust_assert_fact_candidate() {
    let src = r#"
pub fn validate(items: &[Item]) {
    assert!(!items.is_empty());
}
"#;
    let mined = rust_extractor::extract(src);
    assert!(!mined.fact_candidates.is_empty());
    assert!(mined.fact_candidates.iter().any(|f| f.confidence == Confidence::High));
}

#[test]
fn mine_rust_contains_fact_candidate() {
    let src = r#"
pub fn check(group: &Group, user: &User) {
    if group.members.contains(&user) {
        return;
    }
}
"#;
    let mined = rust_extractor::extract(src);
    assert!(mined.fact_candidates.iter().any(|f|
        f.confidence == Confidence::Medium && f.source_pattern.contains("contains")
    ));
}

#[test]
fn mine_rust_renderer_produces_valid_alloy() {
    let src = r#"
pub struct User {
    pub name: String,
    pub group: Option<Group>,
    pub roles: Vec<Role>,
}
pub struct Group {}
pub struct Role {}
"#;
    let mined = rust_extractor::extract(src);
    let rendered = renderer::render(&mined);

    // Should be parseable by our parser
    let result = oxidtr::parser::parse(&rendered);
    assert!(result.is_ok(), "rendered Alloy should parse: {:?}\n---\n{rendered}", result.err());
    let model = result.unwrap();
    assert_eq!(model.sigs.len(), 3);
}

#[test]
fn mine_rust_round_trip_struct_preserves_structure() {
    // Generate Rust from a simple model, then mine it back
    let alloy_src = r#"
sig User { group: lone Group, roles: set Role }
sig Group {}
sig Role {}
"#;
    let model = oxidtr::parser::parse(alloy_src).unwrap();
    let ir = oxidtr::ir::lower(&model).unwrap();
    let files = oxidtr::backend::rust::generate(&ir);
    let models_rs = files.iter().find(|f| f.path == "models.rs").unwrap();

    // Mine from generated Rust
    let mined = rust_extractor::extract(&models_rs.content);

    // Should recover the same sigs
    assert_eq!(mined.sigs.len(), 3);
    let user = mined.sigs.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.fields.len(), 2);
    let group_field = user.fields.iter().find(|f| f.name == "group").unwrap();
    assert_eq!(group_field.mult, MinedMultiplicity::Lone);
    assert_eq!(group_field.target, "Group");
    let roles_field = user.fields.iter().find(|f| f.name == "roles").unwrap();
    assert_eq!(roles_field.mult, MinedMultiplicity::Set);
    assert_eq!(roles_field.target, "Role");
}

#[test]
fn resolve_external_types_adds_placeholder_sigs() {
    let src = r#"
pub struct User {
    pub name: String,
    pub age: usize,
    pub group: Group,
}

pub struct Group {
    pub title: String,
}
"#;
    let mut mined = rust_extractor::extract(src);
    resolve_external_types(&mut mined);

    let sig_names: Vec<&str> = mined.sigs.iter().map(|s| s.name.as_str()).collect();

    // String and usize are referenced but not defined — should be added as placeholders
    assert!(sig_names.contains(&"String"), "String should be added as placeholder sig");
    assert!(sig_names.contains(&"usize"), "usize should be added as placeholder sig");

    // Group IS defined — should NOT be duplicated
    assert_eq!(sig_names.iter().filter(|&&n| n == "Group").count(), 1);

    // Placeholder sigs should have no fields and not be abstract
    let string_sig = mined.sigs.iter().find(|s| s.name == "String").unwrap();
    assert!(string_sig.fields.is_empty());
    assert!(!string_sig.is_abstract);
}

#[test]
fn resolve_external_types_skips_self_referencing() {
    let src = r#"
pub struct Node {
    pub parent: Option<Box<Node>>,
}
"#;
    let mut mined = rust_extractor::extract(src);
    resolve_external_types(&mut mined);

    // Node references itself — should NOT create a duplicate
    assert_eq!(mined.sigs.len(), 1);
    assert_eq!(mined.sigs[0].name, "Node");
}

#[test]
fn rendered_als_is_self_contained_after_resolve() {
    let src = r#"
pub struct Config {
    pub name: String,
    pub count: usize,
}
"#;
    let mut mined = rust_extractor::extract(src);
    resolve_external_types(&mut mined);
    let rendered = renderer::render(&mined);

    // The rendered .als should contain sig definitions for String and usize
    assert!(rendered.contains("sig String {"), "rendered should define sig String");
    assert!(rendered.contains("sig usize {"), "rendered should define sig usize");
}
