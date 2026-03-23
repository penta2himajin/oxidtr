use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;

fn generate_from(input: &str) -> Vec<rust::GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    rust::generate(&ir)
}

fn find_file<'a>(files: &'a [rust::GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

#[test]
fn generate_empty_struct() {
    let files = generate_from("sig Foo {}");
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub struct Foo"));
    assert!(content.contains("#[derive(Debug, Clone, PartialEq, Eq, Hash)]"));
}

#[test]
fn generate_struct_with_one_field() {
    let files = generate_from(r#"
        sig User { name: one Name }
        sig Name {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub struct User"));
    assert!(content.contains("pub name: Name"));
}

#[test]
fn generate_option_for_lone() {
    let files = generate_from(r#"
        sig Node { next: lone Node }
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("Option<Box<Node>>"));
}

#[test]
fn generate_vec_for_set() {
    let files = generate_from(r#"
        sig User { roles: set Role }
        sig Role {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("Vec<Role>"));
}

#[test]
fn generate_vec_for_seq() {
    let files = generate_from(r#"
        sig Order { items: seq Item }
        sig Item {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("Vec<Item>"));
}

#[test]
fn generate_enum_for_abstract_sig() {
    let files = generate_from(r#"
        abstract sig Role {}
        one sig Admin extends Role {}
        one sig Viewer extends Role {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub enum Role"));
    assert!(content.contains("Admin"));
    assert!(content.contains("Viewer"));
}

#[test]
fn generate_operation_stub() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        pred assign[u: one User, r: one Role] { u = u }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("fn assign"));
    assert!(content.contains("user: &User") || content.contains("u: &User"));
    assert!(content.contains("todo!"));
}

#[test]
fn generate_property_test() {
    let files = generate_from(r#"
        sig A {}
        assert AlwaysTrue { all a: A | a = a }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("always_true") || content.contains("AlwaysTrue"));
    assert!(content.contains("#[test]") || content.contains("proptest"));
}
