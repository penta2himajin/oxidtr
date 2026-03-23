use oxidtr::parser;
use oxidtr::ir;
use oxidtr::parser::ast::*;

fn parse_and_lower(input: &str) -> ir::nodes::OxidtrIR {
    let model = parser::parse(input).expect("should parse");
    ir::lower(&model).expect("should lower")
}

#[test]
fn lower_empty_sig_to_structure() {
    let ir = parse_and_lower("sig Foo {}");
    assert_eq!(ir.structures.len(), 1);
    assert_eq!(ir.structures[0].name, "Foo");
    assert!(!ir.structures[0].is_enum);
    assert!(ir.structures[0].parent.is_none());
    assert!(ir.structures[0].fields.is_empty());
}

#[test]
fn lower_abstract_sig_to_enum() {
    let ir = parse_and_lower("abstract sig Base {}");
    assert_eq!(ir.structures[0].name, "Base");
    assert!(ir.structures[0].is_enum);
}

#[test]
fn lower_extends_preserves_parent() {
    let ir = parse_and_lower(r#"
        abstract sig Role {}
        sig Admin extends Role {}
        sig Viewer extends Role {}
    "#);
    assert_eq!(ir.structures.len(), 3);

    let admin = ir.structures.iter().find(|s| s.name == "Admin").unwrap();
    assert_eq!(admin.parent.as_deref(), Some("Role"));

    let viewer = ir.structures.iter().find(|s| s.name == "Viewer").unwrap();
    assert_eq!(viewer.parent.as_deref(), Some("Role"));
}

#[test]
fn lower_fields_preserve_multiplicity() {
    let ir = parse_and_lower(r#"
        sig User { name: one Name, roles: set Role, backup: lone User }
        sig Name {}
        sig Role {}
    "#);
    let user = ir.structures.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.fields.len(), 3);
    assert_eq!(user.fields[0].name, "name");
    assert_eq!(user.fields[0].mult, Multiplicity::One);
    assert_eq!(user.fields[0].target, "Name");
    assert_eq!(user.fields[1].mult, Multiplicity::Set);
    assert_eq!(user.fields[2].mult, Multiplicity::Lone);
}

#[test]
fn lower_fact_to_constraint() {
    let ir = parse_and_lower(r#"
        sig A {}
        fact SomeFact { all a: A | a = a }
    "#);
    assert_eq!(ir.constraints.len(), 1);
    assert_eq!(ir.constraints[0].name.as_deref(), Some("SomeFact"));
}

#[test]
fn lower_anonymous_fact() {
    let ir = parse_and_lower(r#"
        sig A {}
        fact { all a: A | a = a }
    "#);
    assert_eq!(ir.constraints.len(), 1);
    assert!(ir.constraints[0].name.is_none());
}

#[test]
fn lower_pred_to_operation() {
    let ir = parse_and_lower(r#"
        sig User {}
        sig Role {}
        pred assign[u: one User, r: one Role] { u = u }
    "#);
    assert_eq!(ir.operations.len(), 1);
    assert_eq!(ir.operations[0].name, "assign");
    assert_eq!(ir.operations[0].params.len(), 2);
    assert_eq!(ir.operations[0].params[0].name, "u");
    assert_eq!(ir.operations[0].params[0].type_name, "User");
}

#[test]
fn lower_assert_to_property() {
    let ir = parse_and_lower(r#"
        sig A {}
        assert NoSelfRef { all a: A | a = a }
    "#);
    assert_eq!(ir.properties.len(), 1);
    assert_eq!(ir.properties[0].name, "NoSelfRef");
}

#[test]
fn lower_full_model() {
    let ir = parse_and_lower(r#"
        abstract sig Role {}
        one sig Admin extends Role {}
        one sig Viewer extends Role {}

        sig User {
            role: one Role,
            owns: set Resource
        }

        sig Resource {}

        fact AdminOwnsNothing {
            all u: User | u.role in Admin implies #u.owns = #u.owns
        }

        pred changeRole[u: one User, r: one Role] {
            u.role = r
        }

        assert UniqueOwnership {
            all r: Resource | all u1: User | all u2: User |
                r in u1.owns and r in u2.owns implies u1 = u2
        }

        check UniqueOwnership for 5
    "#);

    assert_eq!(ir.structures.len(), 5); // Role, Admin, Viewer, User, Resource
    assert_eq!(ir.constraints.len(), 1); // AdminOwnsNothing
    assert_eq!(ir.operations.len(), 1);  // changeRole
    assert_eq!(ir.properties.len(), 1);  // UniqueOwnership
}

#[test]
fn lower_one_sig_preserves_singleton_flag() {
    use oxidtr::parser;
    use oxidtr::ir;
    let src = "abstract sig Mult {} one sig MultOne extends Mult {}";
    let ast = parser::parse(src).unwrap();
    let ir = ir::lower(&ast).unwrap();
    let mult_one = ir.structures.iter().find(|s| s.name == "MultOne").unwrap();
    assert!(mult_one.is_singleton, "one sig should propagate is_singleton to IR");
}

#[test]
fn unhandled_response_not_fired_for_singleton_children() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    // All children are one sig → value enum → no UNHANDLED_RESPONSE_PATTERN
    fs::write(&model_path, r#"
        abstract sig Multiplicity {}
        one sig MultOne  extends Multiplicity {}
        one sig MultLone extends Multiplicity {}
        one sig MultSet  extends Multiplicity {}
    "#).unwrap();
    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: dir.path().join("out").to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();
    assert!(
        !result.warnings.iter().any(|w| matches!(w.kind, WarningKind::UnhandledResponsePattern)),
        "singleton-only abstract sig should not trigger UNHANDLED_RESPONSE_PATTERN"
    );
}

#[test]
fn unhandled_response_still_fires_for_non_singleton_children() {
    use oxidtr::generate::{self, GenerateConfig, WarningLevel, WarningKind};
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    // Regular sigs → response pattern → UNHANDLED_RESPONSE_PATTERN expected
    fs::write(&model_path, r#"
        abstract sig Response {}
        sig OkResponse extends Response {}
        sig ErrResponse extends Response {}
        pred handleOk[r: one OkResponse] { r = r }
    "#).unwrap();
    let config = GenerateConfig {
        target: "rust".to_string(),
        output_dir: dir.path().join("out").to_str().unwrap().to_string(),
        warnings: WarningLevel::Off,
        features: vec![],
    };
    let result = generate::run(model_path.to_str().unwrap(), &config).unwrap();
    // ErrResponse は is_error_name() でエラー扱い → MissingErrorPropagation
    // OkResponse に pred があるが ErrResponse にはない → どちらかの警告が出ることを確認
    assert!(
        result.warnings.iter().any(|w| matches!(
            w.kind,
            WarningKind::UnhandledResponsePattern | WarningKind::MissingErrorPropagation
        )),
        "non-singleton response sig should still trigger pattern or error warning"
    );
}
