use oxidtr::extract::ts_extractor;
use oxidtr::extract::renderer;
use oxidtr::extract::{MinedMultiplicity, Confidence};

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
fn mine_ts_array_to_seq() {
    let src = r#"
export interface Group {
  members: User[];
}
"#;
    let mined = ts_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Seq);
    assert_eq!(f.target, "User");
}

#[test]
fn mine_ts_set_to_set() {
    let src = r#"
export interface Group {
  members: Set<User>;
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

// --- Tests for parse_interface_decl: extends parent extraction ---

#[test]
fn mine_ts_interface_extends_parent() {
    let src = r#"
export interface Child extends Parent {
  name: string;
}
export interface Parent {}
"#;
    let mined = ts_extractor::extract(src);
    let child = mined.sigs.iter().find(|s| s.name == "Child").unwrap();
    assert_eq!(child.parent.as_deref(), Some("Parent"));
    assert_eq!(child.fields.len(), 1);
}

#[test]
fn mine_ts_interface_extends_with_generics() {
    let src = r#"
export interface Container<T> extends Base {
  items: T[];
}
"#;
    let mined = ts_extractor::extract(src);
    let container = mined.sigs.iter().find(|s| s.name == "Container").unwrap();
    assert_eq!(container.parent.as_deref(), Some("Base"));
}

#[test]
fn mine_ts_interface_extends_multiple_takes_first() {
    let src = r#"
export interface Combo extends Alpha, Beta {
  value: number;
}
"#;
    let mined = ts_extractor::extract(src);
    let combo = mined.sigs.iter().find(|s| s.name == "Combo").unwrap();
    assert_eq!(combo.parent.as_deref(), Some("Alpha"));
}

#[test]
fn mine_ts_interface_no_extends_has_no_parent() {
    let src = r#"
export interface Plain {
  x: number;
}
"#;
    let mined = ts_extractor::extract(src);
    let plain = mined.sigs.iter().find(|s| s.name == "Plain").unwrap();
    assert!(plain.parent.is_none());
}

// --- Tests for collect_interface_fields: inline object types ---

#[test]
fn mine_ts_interface_with_inline_object_field() {
    // Inline object type on one line: braces should not confuse depth tracking
    let src = r#"
export interface Widget {
  name: string;
  bounds: { x: number; y: number; };
  color: string;
}
"#;
    let mined = ts_extractor::extract(src);
    let widget = mined.sigs.iter().find(|s| s.name == "Widget").unwrap();
    // Should have 3 fields: name, bounds, color
    assert_eq!(widget.fields.len(), 3, "fields: {:?}", widget.fields.iter().map(|f| &f.name).collect::<Vec<_>>());
}

#[test]
fn mine_ts_interface_with_multiline_inline_object() {
    // Multi-line inline object: the '{' on the field line is not closed on the same line
    let src = r#"
export interface Config {
  name: string;
  options: {
    debug: boolean;
    verbose: boolean;
  };
  version: number;
}
"#;
    let mined = ts_extractor::extract(src);
    let config = mined.sigs.iter().find(|s| s.name == "Config").unwrap();
    // Should have 3 fields: name, options, version (inline object lines skipped)
    let field_names: Vec<&str> = config.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"name"), "missing 'name' in {field_names:?}");
    assert!(field_names.contains(&"version"), "missing 'version' in {field_names:?}");
}

#[test]
fn mine_ts_interface_with_jsdoc_braces() {
    // JSDoc comments with braces should not affect depth
    let src = r#"
export interface Widget {
  /** default value is { x: 0, y: 0 } */
  position: string;
  name: string;
}
"#;
    let mined = ts_extractor::extract(src);
    let widget = mined.sigs.iter().find(|s| s.name == "Widget").unwrap();
    assert_eq!(widget.fields.len(), 2, "fields: {:?}", widget.fields.iter().map(|f| &f.name).collect::<Vec<_>>());
}

// --- Tests for TS collect_block: depth=0 start with started flag ---

#[test]
fn mine_ts_function_body_with_nested_braces() {
    // Function with nested braces should still be collected correctly
    let src = r#"
export function validate(x: User): boolean {
  if (x.name.length === 0) {
    throw new Error("empty");
  }
  return true;
}
"#;
    let mined = ts_extractor::extract(src);
    // Should extract a fact candidate from the throw
    assert!(!mined.fact_candidates.is_empty(), "should extract facts from function body");
}

#[test]
fn mine_ts_function_with_brace_on_next_line() {
    // Function where '{' is on the next line (collect_block starts at depth=0)
    let src = r#"
export function check(x: User): boolean
{
  if (x.name.includes("admin")) {
    return true;
  }
  return false;
}
"#;
    let mined = ts_extractor::extract(src);
    // Should extract includes fact candidate
    assert!(mined.fact_candidates.iter().any(|f|
        f.source_pattern.contains("includes")
    ), "should extract .includes() fact from function body");
}

#[test]
fn ts_extract_union_field_type() {
    use oxidtr::extract::ts_extractor;
    let src = r#"
export interface DisplayProps {
  blendMode?: number | string;
  tint?: number;
}
"#;
    let model = ts_extractor::extract(src);
    let sig = model.sigs.iter().find(|s| s.name == "DisplayProps").expect("DisplayProps not found");
    let blend = sig.fields.iter().find(|f| f.name == "blendMode").expect("blendMode not found");
    assert_eq!(blend.raw_union_type, Some("number | string".to_string()),
        "blendMode should have raw_union_type preserved");
    // tint has no union type
    let tint = sig.fields.iter().find(|f| f.name == "tint").expect("tint not found");
    assert_eq!(tint.raw_union_type, None, "tint should have no raw_union_type");
}

#[test]
fn ts_extract_intersection_type_alias() {
    use oxidtr::extract::ts_extractor;
    let src = r#"
export type BaseProps = TransformProps & DisplayProps & OriginProps;
"#;
    let model = ts_extractor::extract(src);
    let sig = model.sigs.iter().find(|s| s.name == "BaseProps").expect("BaseProps not found");
    assert_eq!(
        sig.intersection_of,
        vec!["TransformProps".to_string(), "DisplayProps".to_string(), "OriginProps".to_string()],
        "BaseProps should have intersection_of set"
    );
}

#[test]
fn ts_generate_union_type_passthrough() {
    let als = r#"
sig DisplayProps {
  blendMode: lone Num, -- union: number | string
  tint: lone Num
}
sig Num {}
"#;
    let model = oxidtr::parser::parse(als).expect("parse");
    let ir = oxidtr::ir::lower(&model).expect("lower");
    let files = oxidtr::backend::typescript::generate(&ir);
    let models = files.iter().find(|f| f.path == "models.ts").expect("models.ts");
    assert!(models.content.contains("blendMode: number | string | null"),
        "blendMode should use raw union type, got:\n{}", models.content);
    assert!(models.content.contains("tint: Num | null"),
        "tint without annotation should use sig type");
}

#[test]
fn ts_generate_intersection_type_alias() {
    let als = r#"
abstract sig BaseProps {}
sig TransformProps extends BaseProps {}
sig DisplayProps extends BaseProps {}
"#;
    // Manually construct IR with intersection_of
    use oxidtr::ir::nodes::StructureNode;
    use oxidtr::parser::ast::SigMultiplicity;
    let model4 = oxidtr::parser::parse(als).expect("parse");
    let mut ir = oxidtr::ir::lower(&model4).expect("lower");
    // Add a fake intersection node
    ir.structures.push(oxidtr::ir::nodes::StructureNode {
        name: "AllProps".to_string(),
        is_enum: false,
        sig_multiplicity: SigMultiplicity::Default,
        parent: None,
        fields: vec![],
        intersection_of: vec!["TransformProps".to_string(), "DisplayProps".to_string()],
    });
    let files = oxidtr::backend::typescript::generate(&ir);
    let models = files.iter().find(|f| f.path == "models.ts").expect("models.ts");
    assert!(models.content.contains("export type AllProps = TransformProps & DisplayProps"),
        "AllProps should be generated as intersection type alias, got:\n{}", models.content);
}
