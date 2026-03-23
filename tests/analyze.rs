use oxidtr::parser;
use oxidtr::ir;
use oxidtr::analyze::{self, ConstraintInfo};

fn analyze_from(input: &str) -> Vec<ConstraintInfo> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    analyze::analyze(&ir)
}

#[test]
fn analyze_acyclic_constraint() {
    let infos = analyze_from("sig Node { parent: lone Node }\nfact NoCycle { no n: Node | n in n.^parent }");
    assert!(infos.iter().any(|c| matches!(c,
        ConstraintInfo::Acyclic { sig_name, field_name }
        if sig_name == "Node" && field_name == "parent"
    )));
}

#[test]
fn analyze_no_self_ref_constraint() {
    let infos = analyze_from(
        "sig User { manages: set User }\nfact NoSelfManage { all u: User | not u in u.manages }"
    );
    assert!(infos.iter().any(|c| matches!(c,
        ConstraintInfo::NoSelfRef { sig_name, field_name }
        if sig_name == "User" && field_name == "manages"
    )));
}

#[test]
fn analyze_named_constraint() {
    let infos = analyze_from("sig User {}\nfact AllValid { all u: User | u = u }");
    assert!(infos.iter().any(|c| matches!(c,
        ConstraintInfo::Named { name, .. } if name == "AllValid"
    )));
}

#[test]
fn analyze_constraint_names_for_sig() {
    let model = parser::parse(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let names = analyze::constraint_names_for_sig(&ir, "User");
    assert!(names.contains(&"HasRole".to_string()));
}

#[test]
fn analyze_describe_expr() {
    let model = parser::parse("sig Node {}\nfact F { all n: Node | n = n }").unwrap();
    let desc = analyze::describe_expr(&model.facts[0].body);
    assert!(desc.contains("for all"));
    assert!(desc.contains("Node"));
}

#[test]
fn schema_generates_valid_json() {
    let model = parser::parse(
        "sig User { group: lone Group, roles: set Role }\nsig Group {}\nsig Role {}"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = oxidtr::backend::schema::generate(&ir);
    assert_eq!(file.path, "schemas.json");
    assert!(file.content.contains("\"$schema\""));
    assert!(file.content.contains("\"User\""));
    assert!(file.content.contains("\"Group\""));
    assert!(file.content.contains("\"array\""));   // set → array
    assert!(file.content.contains("\"null\""));     // lone → nullable
}

#[test]
fn schema_enum_as_string_enum() {
    let model = parser::parse(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = oxidtr::backend::schema::generate(&ir);
    assert!(file.content.contains("\"enum\""));
    assert!(file.content.contains("\"Red\""));
    assert!(file.content.contains("\"Blue\""));
}

#[test]
fn schema_discriminated_union() {
    let model = parser::parse(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = oxidtr::backend::schema::generate(&ir);
    assert!(file.content.contains("\"oneOf\""));
    assert!(file.content.contains("\"discriminator\""));
    assert!(file.content.contains("\"kind\""));
}

#[test]
fn schema_self_hosting() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = oxidtr::backend::schema::generate(&ir);
    assert!(file.content.contains("\"SigDecl\""));
    assert!(file.content.contains("\"OxidtrIR\""));
    assert!(file.content.contains("\"Multiplicity\""));
}
