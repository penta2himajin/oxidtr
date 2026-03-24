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
    assert_eq!(sig.multiplicity, SigMultiplicity::One, "one sig should set multiplicity=One");
    assert!(!sig.is_abstract);
}

#[test]
fn parse_regular_sig_not_singleton() {
    use oxidtr::parser::parse;
    let model = parse("sig Foo {}").unwrap();
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Default);
}

#[test]
fn parse_some_sig() {
    let model = parser::parse("some sig Foo {}").unwrap();
    assert_eq!(model.sigs.len(), 1);
    assert_eq!(model.sigs[0].name, "Foo");
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Some);
    assert!(!model.sigs[0].is_abstract);
}

#[test]
fn parse_lone_sig() {
    let model = parser::parse("lone sig Foo {}").unwrap();
    assert_eq!(model.sigs.len(), 1);
    assert_eq!(model.sigs[0].name, "Foo");
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Lone);
    assert!(!model.sigs[0].is_abstract);
}

#[test]
fn parse_some_sig_extends() {
    let model = parser::parse("some sig Foo extends Bar {}").unwrap();
    assert_eq!(model.sigs[0].name, "Foo");
    assert_eq!(model.sigs[0].parent.as_deref(), Some("Bar"));
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Some);
}

#[test]
fn parse_lone_sig_extends() {
    let model = parser::parse("lone sig Foo extends Bar {}").unwrap();
    assert_eq!(model.sigs[0].name, "Foo");
    assert_eq!(model.sigs[0].parent.as_deref(), Some("Bar"));
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Lone);
}

#[test]
fn parse_some_sig_with_fields() {
    let model = parser::parse("some sig Foo { bar: one Baz }").unwrap();
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Some);
    assert_eq!(model.sigs[0].fields.len(), 1);
    assert_eq!(model.sigs[0].fields[0].name, "bar");
}

#[test]
fn parse_lone_sig_with_fields() {
    let model = parser::parse("lone sig Foo { bar: lone Baz }").unwrap();
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Lone);
    assert_eq!(model.sigs[0].fields.len(), 1);
    assert_eq!(model.sigs[0].fields[0].mult, Multiplicity::Lone);
}

#[test]
fn parse_mixed_sig_multiplicities() {
    let input = r#"
        abstract sig Base {}
        one sig A extends Base {}
        some sig B extends Base {}
        lone sig C extends Base {}
        sig D extends Base {}
    "#;
    let model = parser::parse(input).unwrap();
    assert_eq!(model.sigs.len(), 5);
    assert_eq!(model.sigs[0].multiplicity, SigMultiplicity::Default); // abstract sig
    assert_eq!(model.sigs[1].multiplicity, SigMultiplicity::One);
    assert_eq!(model.sigs[2].multiplicity, SigMultiplicity::Some);
    assert_eq!(model.sigs[3].multiplicity, SigMultiplicity::Lone);
    assert_eq!(model.sigs[4].multiplicity, SigMultiplicity::Default);
}

// ── Alloy 6: var field tests ───────────────────────────────────────────────────

#[test]
fn parse_var_field() {
    let input = "sig Server { var load: one Int, name: one String }";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs[0].fields.len(), 2);
    assert!(model.sigs[0].fields[0].is_var, "var field should have is_var=true");
    assert_eq!(model.sigs[0].fields[0].name, "load");
    assert!(!model.sigs[0].fields[1].is_var, "non-var field should have is_var=false");
}

#[test]
fn parse_var_field_set_mult() {
    let input = "sig Server { var connections: set Client }";
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs[0].fields[0].name, "connections");
    assert!(model.sigs[0].fields[0].is_var);
    assert_eq!(model.sigs[0].fields[0].mult, Multiplicity::Set);
    assert_eq!(model.sigs[0].fields[0].target, "Client");
}

#[test]
fn parse_var_field_lone() {
    let input = "sig Node { var next: lone Node }";
    let model = parser::parse(input).expect("should parse");
    assert!(model.sigs[0].fields[0].is_var);
    assert_eq!(model.sigs[0].fields[0].mult, Multiplicity::Lone);
}

#[test]
fn parse_non_var_field_is_var_false() {
    let input = "sig Foo { bar: one Baz }";
    let model = parser::parse(input).expect("should parse");
    assert!(!model.sigs[0].fields[0].is_var);
}
