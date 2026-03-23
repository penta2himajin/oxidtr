use oxidtr::parser;
use oxidtr::ir;
use oxidtr::analyze::{self, ConstraintInfo, BeanValidation};

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

// ── Bean Validation ─────────────────────────────────────────────────────────

#[test]
fn bean_validation_size_from_cardinality() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members = #t.members }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let validations = analyze::bean_validations_for_field(&ir, "Team", "members");
    assert!(validations.iter().any(|v| matches!(v, BeanValidation::Size { .. })),
        "expected @Size validation for members field");
}

#[test]
fn bean_validation_min_max_from_comparison() {
    // u.role != u (field compared to different expr) should trigger @Min/@Max
    let model = parser::parse(
        "sig User { role: one User }\nfact ValidRole { all u: User | u.role != u }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let validations = analyze::bean_validations_for_field(&ir, "User", "role");
    assert!(validations.iter().any(|v| matches!(v, BeanValidation::MinMax { .. })),
        "expected @Min/@Max validation for role field");
}

#[test]
fn bean_validation_empty_for_unrelated_field() {
    let model = parser::parse(
        "sig User { name: one Role, age: one Role }\nsig Role {}\nfact ValidRole { all u: User | u.name = u.name }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let validations = analyze::bean_validations_for_field(&ir, "User", "age");
    // age is not referenced in the constraint, so no @Min/@Max
    assert!(!validations.iter().any(|v| matches!(v, BeanValidation::MinMax { .. })),
        "expected no @Min/@Max validation for age field");
}

// ── Feature 5: Boundary value analysis ──────────────────────────────────────

#[test]
fn bounds_for_field_exact() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members = 3 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let bound = analyze::bounds_for_field(&ir, "Team", "members");
    assert_eq!(bound, Some(analyze::BoundKind::Exact(3)));
}

#[test]
fn bounds_for_field_at_most() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact MaxSize { all t: Team | #t.members <= 5 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let bound = analyze::bounds_for_field(&ir, "Team", "members");
    assert_eq!(bound, Some(analyze::BoundKind::AtMost(5)));
}

#[test]
fn bounds_for_field_at_least() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact MinSize { all t: Team | #t.members >= 2 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let bound = analyze::bounds_for_field(&ir, "Team", "members");
    assert_eq!(bound, Some(analyze::BoundKind::AtLeast(2)));
}

#[test]
fn bounds_for_field_none_when_no_constraint() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let bound = analyze::bounds_for_field(&ir, "Team", "members");
    assert_eq!(bound, None);
}

// ── Feature 5: Boundary fixtures in Rust backend ────────────────────────────

#[test]
fn rust_boundary_fixtures_generated() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact MaxSize { all t: Team | #t.members <= 5 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = oxidtr::backend::rust::generate(&ir);
    let fixtures = files.iter().find(|f| f.path == "fixtures.rs").unwrap();
    assert!(fixtures.content.contains("pub fn boundary_team()"), "missing boundary fixture");
    assert!(fixtures.content.contains("pub fn invalid_team()"), "missing invalid fixture");
}

#[test]
fn rust_boundary_tests_generated() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact MaxSize { all t: Team | #t.members <= 5 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = oxidtr::backend::rust::generate(&ir);
    let tests = files.iter().find(|f| f.path == "tests.rs").unwrap();
    assert!(tests.content.contains("fn boundary_max_size"), "missing boundary test");
    assert!(tests.content.contains("fn invalid_max_size"), "missing invalid test");
}

// ── Feature 6: disj → @unique annotation ────────────────────────────────────

#[test]
fn rust_unique_annotation_on_disj_constraint() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact DistinctMembers { all disj m1, m2: User | m1 != m2 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = oxidtr::backend::rust::generate(&ir);
    let invariants = files.iter().find(|f| f.path == "invariants.rs").unwrap();
    assert!(invariants.content.contains("@unique"), "missing @unique annotation for disj constraint");
}

#[test]
fn ts_unique_annotation_on_disj_constraint() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact DistinctMembers { all disj m1, m2: User | m1 != m2 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = oxidtr::backend::typescript::generate(&ir);
    let invariants = files.iter().find(|f| f.path == "invariants.ts").unwrap();
    assert!(invariants.content.contains("@unique"), "missing @unique annotation for disj constraint in TS");
}

// ── Feature 7: pre/post condition separation ────────────────────────────────

#[test]
fn rust_pre_post_separation_in_operations() {
    let model = parser::parse(
        "sig Account { balance: one Account }\npred withdraw[a: one Account, amount: one Account] { a.balance = a.balance }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = oxidtr::backend::rust::generate(&ir);
    let ops = files.iter().find(|f| f.path == "operations.rs").unwrap();
    // Body expression references only param names, should be @pre
    assert!(ops.content.contains("@pre"), "missing @pre tag in operations");
}

#[test]
fn ts_pre_post_separation_in_operations() {
    let model = parser::parse(
        "sig Account { balance: one Account }\npred withdraw[a: one Account, amount: one Account] { a.balance = a.balance }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = oxidtr::backend::typescript::generate(&ir);
    let ops = files.iter().find(|f| f.path == "operations.ts").unwrap();
    assert!(ops.content.contains("@pre"), "missing @pre tag in TS operations");
}

#[test]
fn is_pre_condition_for_param_comparison() {
    use oxidtr::parser::ast::*;
    let expr = Expr::Comparison {
        op: CompareOp::Eq,
        left: Box::new(Expr::FieldAccess {
            base: Box::new(Expr::VarRef("a".to_string())),
            field: "balance".to_string(),
        }),
        right: Box::new(Expr::FieldAccess {
            base: Box::new(Expr::VarRef("a".to_string())),
            field: "balance".to_string(),
        }),
    };
    let params = vec!["a".to_string(), "amount".to_string()];
    assert!(analyze::is_pre_condition(&expr, &params));
}

// ── Mine tests: verify features don't confuse mine ──────────────────────────

#[test]
fn mine_handles_boundary_fixture_functions() {
    // Verify that mine doesn't create extra sigs from boundary/invalid fixture functions
    let src = r#"
pub struct Team {
    pub members: Vec<User>,
}
pub struct User {
    pub name: String,
}
"#;
    let mined = oxidtr::mine::rust_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 2, "should only have Team and User sigs, not boundary/invalid");
}

#[test]
fn schema_concrete_min_max_items() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact TeamLimit { all t: Team | #t.members <= 10 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = oxidtr::backend::schema::generate(&ir);
    assert!(file.content.contains("\"maxItems\": 10"),
        "schema should have maxItems: 10:\n{}", file.content);
}

#[test]
fn schema_concrete_min_items() {
    let model = parser::parse(
        "sig Team { members: set User }\nsig User {}\nfact TeamMin { all t: Team | #t.members >= 3 }"
    ).unwrap();
    let ir = ir::lower(&model).unwrap();
    let file = oxidtr::backend::schema::generate(&ir);
    assert!(file.content.contains("\"minItems\": 3"),
        "schema should have minItems: 3:\n{}", file.content);
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
