use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::csharp;
use oxidtr::backend::csharp::expr_translator;
use oxidtr::backend::GeneratedFile;

fn generate_cs(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    csharp::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── Models.cs ────────────────────────────────────────────────────────────────

#[test]
fn cs_class_for_sig() {
    let files = generate_cs("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public class User"));
    assert!(m.contains("public Role Name { get; set; }"));
}

#[test]
fn cs_nullable_for_lone() {
    let files = generate_cs("sig Node { parent: lone Node }");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public Node? Parent { get; set; }"));
}

#[test]
fn cs_list_for_set() {
    let files = generate_cs("sig Group { members: set User }\nsig User {}");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public List<User> Members { get; set; }"));
}

#[test]
fn cs_list_for_seq() {
    let files = generate_cs("sig Order { items: seq Item }\nsig Item {}");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public List<Item> Items { get; set; }"));
}

#[test]
fn cs_enum_for_all_singleton() {
    let files = generate_cs(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public enum Color"));
    assert!(m.contains("Red"));
    assert!(m.contains("Blue"));
}

#[test]
fn cs_abstract_class_with_fields() {
    let files = generate_cs(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public abstract class Expr"));
    assert!(m.contains("public class BinOp : Expr"));
    assert!(m.contains("public Expr Left { get; set; }"));
    assert!(m.contains("public Expr Right { get; set; }"));
}

// ── Operations.cs ────────────────────────────────────────────────────────────

#[test]
fn cs_operations_use_throw() {
    let files = generate_cs("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "Operations.cs");
    assert!(ops.contains("public static void ChangeRole("));
    assert!(ops.contains("throw new NotImplementedException("));
}

#[test]
fn cs_operations_with_return() {
    let files = generate_cs("sig User {}\nfun getUser[]: one User { User }");
    let ops = find_file(&files, "Operations.cs");
    assert!(ops.contains("public static User GetUser("));
}

// ── Fixtures.cs ──────────────────────────────────────────────────────────────

#[test]
fn cs_fixtures_default_factory() {
    let files = generate_cs("sig User { name: one Role }\nsig Role {}");
    let fix = find_file(&files, "Fixtures.cs");
    assert!(fix.contains("public static User DefaultUser()"));
    assert!(fix.contains("new User"));
}

#[test]
fn cs_fixtures_boundary() {
    let files = generate_cs("sig Group { members: set User }\nsig User {}");
    let fix = find_file(&files, "Fixtures.cs");
    assert!(fix.contains("public static Group BoundaryGroup()"));
}

// ── Tests.cs ─────────────────────────────────────────────────────────────────

#[test]
fn cs_tests_generated_for_constraints() {
    let files = generate_cs("sig Node { parent: lone Node }\nfact NoSelfRef { all n: Node | n.parent != n }");
    let t = find_file(&files, "Tests.cs");
    assert!(t.contains("[Fact]") || t.contains("[Test]"));
    assert!(t.contains("NoSelfRef"));
}

// ── expr_translator ──────────────────────────────────────────────────────────

fn translate_cs(alloy: &str, constraint_name: &str) -> String {
    let model = parser::parse(alloy).expect("parse");
    let ir_result = ir::lower(&model).expect("lower");
    let constraint = ir_result.constraints.iter()
        .find(|c| c.name.as_deref() == Some(constraint_name))
        .expect("constraint not found");
    expr_translator::translate_with_ir(&constraint.expr, &ir_result)
}

#[test]
fn cs_expr_comparison_eq() {
    let result = translate_cs("sig User { name: one Role }\nsig Role {}\nfact Eq { all u: User | u.name = u.name }", "Eq");
    assert!(result.contains("=="), "expected == in: {result}");
}

#[test]
fn cs_expr_comparison_neq() {
    let result = translate_cs("sig Node { parent: lone Node }\nfact NoSelf { all n: Node | n.parent != n }", "NoSelf");
    assert!(result.contains("!="), "expected != in: {result}");
}

#[test]
fn cs_expr_field_access_pascal_case() {
    let result = translate_cs("sig User { name: one Role }\nsig Role {}\nfact F { all u: User | u.name = u.name }", "F");
    assert!(result.contains(".Name"), "expected .Name in: {result}");
}

#[test]
fn cs_expr_quantifier_all() {
    let result = translate_cs("sig User {}\nfact F { all u: User | u = u }", "F");
    assert!(result.contains(".All(") || result.contains("All(") || result.contains("TrueForAll("),
        "expected LINQ All in: {result}");
}

#[test]
fn cs_expr_not() {
    let result = translate_cs("sig Node { parent: lone Node }\nfact F { all n: Node | not (n.parent = n) }", "F");
    assert!(result.contains("!"), "expected ! in: {result}");
}

#[test]
fn cs_expr_implies() {
    let result = translate_cs("sig A { x: lone A }\nfact F { all a: A | a.x = a implies a = a }", "F");
    // C# implies: !(cond) || (consequent)
    assert!(result.contains("||"), "expected || for implies in: {result}");
}

#[test]
fn cs_expr_prime() {
    let result = translate_cs("sig S { var x: one S }\nfact F { all s: S | s.x' = s }", "F");
    assert!(result.contains("NextX") || result.contains("nextX"),
        "expected prime translation in: {result}");
}

#[test]
fn cs_expr_cardinality() {
    let result = translate_cs("sig G { members: set G }\nfact F { all g: G | #g.members = #g.members }", "F");
    assert!(result.contains(".Count") || result.contains("Count("),
        "expected Count in: {result}");
}
