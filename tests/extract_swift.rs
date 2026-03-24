use oxidtr::extract::swift_extractor;
use oxidtr::extract::{MinedMultiplicity, Confidence};

#[test]
fn swift_extract_struct() {
    let model = swift_extractor::extract(r#"
struct User: Equatable {
    let name: String
    let age: Int
}
"#);
    assert_eq!(model.sigs.len(), 1);
    let sig = &model.sigs[0];
    assert_eq!(sig.name, "User");
    assert_eq!(sig.fields.len(), 2);
    assert_eq!(sig.fields[0].name, "name");
    assert_eq!(sig.fields[0].target, "String");
    assert_eq!(sig.fields[0].mult, MinedMultiplicity::One);
}

#[test]
fn swift_extract_optional() {
    let model = swift_extractor::extract(r#"
struct Node: Equatable {
    let parent: Node?
}
"#);
    let sig = &model.sigs[0];
    let f = &sig.fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Lone);
    assert_eq!(f.target, "Node");
}

#[test]
fn swift_extract_set() {
    let model = swift_extractor::extract(r#"
struct Group: Equatable {
    let members: Set<User>
}
"#);
    let f = &model.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Set);
    assert_eq!(f.target, "User");
}

#[test]
fn swift_extract_array() {
    let model = swift_extractor::extract(r#"
struct Order: Equatable {
    let items: [Item]
}
"#);
    let f = &model.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Seq);
    assert_eq!(f.target, "Item");
}

#[test]
fn swift_extract_enum() {
    let model = swift_extractor::extract(r#"
enum Status: Equatable, Hashable, CaseIterable {
    case active
    case inactive
}
"#);
    // Should produce abstract sig + 2 children
    assert!(model.sigs.len() >= 3, "expected 3 sigs, got {}", model.sigs.len());
    let status = model.sigs.iter().find(|s| s.name == "Status").expect("Status sig");
    assert!(status.is_abstract);
    let active = model.sigs.iter().find(|s| s.name == "Active").expect("Active sig");
    assert_eq!(active.parent.as_deref(), Some("Status"));
}

#[test]
fn swift_extract_enum_with_associated_values() {
    let model = swift_extractor::extract(r#"
enum Expr: Equatable {
    case literal
    case binOp(left: Expr, right: Expr)
}
"#);
    let expr = model.sigs.iter().find(|s| s.name == "Expr").expect("Expr sig");
    assert!(expr.is_abstract);
    let binop = model.sigs.iter().find(|s| s.name == "BinOp").expect("BinOp sig");
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 2);
}

#[test]
fn swift_extract_precondition_fact() {
    let model = swift_extractor::extract(r#"
func validate(x: Int) {
    precondition(x > 0)
}
"#);
    assert!(!model.fact_candidates.is_empty());
    assert!(model.fact_candidates.iter().any(|f| f.confidence == Confidence::High));
}

#[test]
fn swift_extract_guard_fact() {
    let model = swift_extractor::extract(r#"
func process() {
    guard let value = optional else { return }
}
"#);
    assert!(model.fact_candidates.iter().any(|f| f.source_pattern == "guard"));
}

#[test]
fn swift_extract_contains_fact() {
    let model = swift_extractor::extract(r#"
func check(items: Set<String>, item: String) {
    if items.contains(item) { }
}
"#);
    assert!(model.fact_candidates.iter().any(|f| f.source_pattern == ".contains() check"));
}

#[test]
fn swift_reverse_translate_basic() {
    assert_eq!(
        swift_extractor::reverse_translate_expr("x == y"),
        Some("x = y".to_string()),
    );
    assert_eq!(
        swift_extractor::reverse_translate_expr("items.count"),
        Some("#items".to_string()),
    );
    assert_eq!(
        swift_extractor::reverse_translate_expr("items.contains(x)"),
        Some("x in items".to_string()),
    );
}
