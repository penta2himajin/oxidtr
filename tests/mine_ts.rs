use oxidtr::mine::ts_extractor;
use oxidtr::mine::renderer;
use oxidtr::mine::{MinedMultiplicity, Confidence};

#[test]
fn mine_ts_interface_to_sig() {
    let src = r#"
export interface User {
  name: string;
  age: number;
}
"#;
    let mined = ts_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 1);
    assert_eq!(mined.sigs[0].name, "User");
    assert_eq!(mined.sigs[0].fields.len(), 2);
    assert!(!mined.sigs[0].is_abstract);
}

#[test]
fn mine_ts_nullable_to_lone() {
    let src = r#"
export interface Node {
  parent: Node | null;
}
"#;
    let mined = ts_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.name, "parent");
    assert_eq!(f.mult, MinedMultiplicity::Lone);
    assert_eq!(f.target, "Node");
}

#[test]
fn mine_ts_optional_field_to_lone() {
    let src = r#"
export interface Node {
  parent?: Node;
}
"#;
    let mined = ts_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Lone);
}

#[test]
fn mine_ts_array_to_set() {
    let src = r#"
export interface Group {
  members: User[];
}
"#;
    let mined = ts_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Set);
    assert_eq!(f.target, "User");
}

#[test]
fn mine_ts_string_literal_union_to_abstract_sig() {
    let src = r#"
export type Status = "Active" | "Inactive";
"#;
    let mined = ts_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 3);
    assert!(mined.sigs[0].is_abstract);
    assert_eq!(mined.sigs[0].name, "Status");
    assert_eq!(mined.sigs[1].name, "Active");
    assert_eq!(mined.sigs[1].parent.as_deref(), Some("Status"));
}

#[test]
fn mine_ts_discriminated_union_to_abstract_sig() {
    let src = r#"
export interface Literal {
  kind: "Literal";
}

export interface BinOp {
  kind: "BinOp";
  left: Expr;
  right: Expr;
}

export type Expr = Literal | BinOp;
"#;
    let mined = ts_extractor::extract(src);
    // Literal, BinOp (from interfaces), Expr (abstract from union)
    let expr = mined.sigs.iter().find(|s| s.name == "Expr").unwrap();
    assert!(expr.is_abstract);

    let literal = mined.sigs.iter().find(|s| s.name == "Literal").unwrap();
    assert_eq!(literal.parent.as_deref(), Some("Expr"));

    let binop = mined.sigs.iter().find(|s| s.name == "BinOp").unwrap();
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    // kind field should be filtered out
    assert!(binop.fields.iter().all(|f| f.name != "kind"));
    assert_eq!(binop.fields.len(), 2);
}

#[test]
fn mine_ts_includes_fact_candidate() {
    let src = r#"
export function check(group: Group, user: User): boolean {
  return group.members.includes(user);
}
"#;
    let mined = ts_extractor::extract(src);
    assert!(mined.fact_candidates.iter().any(|f|
        f.confidence == Confidence::Medium && f.source_pattern.contains("includes")
    ));
}

#[test]
fn mine_ts_renderer_produces_valid_alloy() {
    let src = r#"
export interface User {
  name: string;
  group: Group | null;
  roles: Role[];
}

export interface Group {}

export interface Role {}
"#;
    let mined = ts_extractor::extract(src);
    let rendered = renderer::render(&mined);

    let result = oxidtr::parser::parse(&rendered);
    assert!(result.is_ok(), "rendered Alloy should parse: {:?}\n---\n{rendered}", result.err());
    let model = result.unwrap();
    assert_eq!(model.sigs.len(), 3);
}

#[test]
fn mine_ts_round_trip_from_generated() {
    // Alloy → generate TS → mine from TS → compare structures
    let alloy_src = r#"
sig User { group: lone Group, roles: set Role }
sig Group {}
sig Role {}
"#;
    let model = oxidtr::parser::parse(alloy_src).unwrap();
    let ir = oxidtr::ir::lower(&model).unwrap();
    let files = oxidtr::backend::typescript::generate(&ir);
    let models_ts = files.iter().find(|f| f.path == "models.ts").unwrap();

    let mined = ts_extractor::extract(&models_ts.content);

    // Should recover the same sigs
    let user = mined.sigs.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.fields.len(), 2);
    let group_field = user.fields.iter().find(|f| f.name == "group").unwrap();
    assert_eq!(group_field.mult, MinedMultiplicity::Lone);
    assert_eq!(group_field.target, "Group");
    let roles_field = user.fields.iter().find(|f| f.name == "roles").unwrap();
    assert_eq!(roles_field.mult, MinedMultiplicity::Set);
    assert_eq!(roles_field.target, "Role");
}
