use oxidtr::parser::{self, ast::*};

#[test]
fn parse_empty_sig() {
    let input = "sig Foo {}";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs.len(), 1);
    assert_eq!(model.sigs[0].name, "Foo");
    assert!(!model.sigs[0].is_abstract);
    assert!(model.sigs[0].parent.is_none());
    assert!(model.sigs[0].fields.is_empty());
}

#[test]
fn parse_abstract_sig() {
    let input = "abstract sig Base {}";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs.len(), 1);
    assert_eq!(model.sigs[0].name, "Base");
    assert!(model.sigs[0].is_abstract);
}

#[test]
fn parse_sig_extends() {
    let input = "sig Child extends Base {}";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs[0].name, "Child");
    assert_eq!(model.sigs[0].parent.as_deref(), Some("Base"));
}

#[test]
fn parse_sig_with_fields() {
    let input = "sig User { name: one Name, roles: set Role }";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs[0].fields.len(), 2);

    let f0 = &model.sigs[0].fields[0];
    assert_eq!(f0.name, "name");
    assert_eq!(f0.mult, Multiplicity::One);
    assert_eq!(f0.target, "Name");

    let f1 = &model.sigs[0].fields[1];
    assert_eq!(f1.name, "roles");
    assert_eq!(f1.mult, Multiplicity::Set);
    assert_eq!(f1.target, "Role");
}

#[test]
fn parse_multiple_sigs() {
    let input = "sig A {}\nsig B {}";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs.len(), 2);
    assert_eq!(model.sigs[0].name, "A");
    assert_eq!(model.sigs[1].name, "B");
}

#[test]
fn parse_lone_field() {
    let input = "sig Foo { bar: lone Baz }";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs[0].fields[0].mult, Multiplicity::Lone);
}

#[test]
fn parse_one_sig_keyword() {
    // "one sig" is Alloy's singleton sig syntax
    let input = "one sig Admin extends Role {}";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs[0].name, "Admin");
    assert_eq!(model.sigs[0].parent.as_deref(), Some("Role"));
}

#[test]
fn parse_one_sig_sets_singleton_flag() {
    use oxidtr::parser::parse;
    let model = parse("one sig Foo {}").unwrap();
    let sig = &model.sigs[0];
    assert!(sig.is_singleton, "one sig should set is_singleton=true");
    assert!(!sig.is_abstract);
}

#[test]
fn parse_regular_sig_not_singleton() {
    use oxidtr::parser::parse;
    let model = parse("sig Foo {}").unwrap();
    assert!(!model.sigs[0].is_singleton);
}
