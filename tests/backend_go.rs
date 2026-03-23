use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::go;
use oxidtr::backend::GeneratedFile;

fn generate_go(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    go::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── models.go ────────────────────────────────────────────────────────────────

#[test]
fn go_struct_for_sig() {
    let files = generate_go("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "models.go");
    assert!(m.contains("type User struct {"));
    assert!(m.contains("Name Role"));
}

#[test]
fn go_pointer_for_lone() {
    let files = generate_go("sig Node { parent: lone Node }");
    let m = find_file(&files, "models.go");
    assert!(m.contains("Parent *Node"));
}

#[test]
fn go_slice_for_set() {
    let files = generate_go("sig Group { members: set User }\nsig User {}");
    let m = find_file(&files, "models.go");
    assert!(m.contains("Members []User"));
}

#[test]
fn go_slice_for_seq() {
    let files = generate_go("sig Order { items: seq Item }\nsig Item {}");
    let m = find_file(&files, "models.go");
    assert!(m.contains("Items []Item"));
}

#[test]
fn go_enum_iota_for_all_singleton() {
    let files = generate_go(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let m = find_file(&files, "models.go");
    assert!(m.contains("type Color int"));
    assert!(m.contains("iota"));
    assert!(m.contains("Red"));
    assert!(m.contains("Blue"));
}

#[test]
fn go_enum_interface_with_fields() {
    let files = generate_go(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let m = find_file(&files, "models.go");
    assert!(m.contains("type Expr interface {"));
    assert!(m.contains("isExpr()"));
    assert!(m.contains("type BinOp struct {"));
    assert!(m.contains("func (BinOp) isExpr()"));
}

// ── operations.go ────────────────────────────────────────────────────────────

#[test]
fn go_operations_use_panic() {
    let files = generate_go("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "operations.go");
    assert!(ops.contains("func ChangeRole("));
    assert!(ops.contains("panic("));
}

#[test]
fn go_operations_return_type() {
    let files = generate_go("sig User {}\nfun findUser[name: one User]: one User { name = name }");
    let ops = find_file(&files, "operations.go");
    assert!(ops.contains("User"));
}

// ── models_test.go ───────────────────────────────────────────────────────────

#[test]
fn go_tests_inline_constraint_expressions() {
    let files = generate_go(
        "sig User { roles: set Role }\nsig Role {}\nfact AllUsersHaveRoles { all u: User | #u.roles > 0 }",
    );
    let t = find_file(&files, "models_test.go");
    assert!(t.contains("testing"));
    assert!(t.contains("func Test_invariant_"));
}

#[test]
fn go_tests_generated_properly() {
    let files = generate_go(
        "sig User { roles: set Role }\nsig Role {}\nfact UserHasRoles { all u: User | #u.roles > 0 }",
    );
    let t = find_file(&files, "models_test.go");
    assert!(t.contains("func Test_invariant_"));
    assert!(t.contains("t.Error("));
}

// ── fixtures.go ──────────────────────────────────────────────────────────────

#[test]
fn go_fixtures_generated() {
    let files = generate_go("sig User { name: one Role, group: lone Group }\nsig Role {}\nsig Group {}");
    let f = find_file(&files, "fixtures.go");
    assert!(f.contains("func DefaultUser()"));
    assert!(f.contains("nil"));
}

#[test]
fn go_fixtures_enum_default() {
    let files = generate_go(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let f = find_file(&files, "fixtures.go");
    assert!(f.contains("func DefaultColor()"));
    assert!(f.contains("Red"));
}

#[test]
fn go_fixtures_boundary() {
    let files = generate_go(
        "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 5 }",
    );
    let f = find_file(&files, "fixtures.go");
    assert!(f.contains("func BoundaryTeam()"));
    assert!(f.contains("func InvalidTeam()"));
}

// ── helpers.go ───────────────────────────────────────────────────────────────

#[test]
fn go_helpers_for_tc() {
    let files = generate_go(
        "sig Node { parent: lone Node }\nassert Acyclic { all n: Node | not (n in n.^parent) }",
    );
    let h = files.iter().find(|f| f.path == "helpers.go");
    assert!(h.is_some(), "helpers.go should be generated for TC");
    let h = h.unwrap();
    assert!(h.content.contains("func TcParent("));
    assert!(h.content.contains("current != nil"));
}

// ── Cross-tests ──────────────────────────────────────────────────────────────

#[test]
fn go_cross_tests_are_disabled() {
    let files = generate_go(
        "sig User { name: one Role }\nsig Role {}\nfact F { all u: User | u = u }\npred doSomething[u: one User] { u = u }",
    );
    let t = find_file(&files, "models_test.go");
    if t.contains("Cross-tests") {
        assert!(t.contains("disabled_Test_"), "Go cross-tests should be disabled via naming convention");
    }
}

// ── Package declaration ──────────────────────────────────────────────────────

#[test]
fn go_models_package_declaration() {
    let files = generate_go("sig User {}");
    let m = find_file(&files, "models.go");
    assert!(m.contains("package models"));
}

#[test]
fn go_tests_import_testing() {
    let files = generate_go("sig User {}\nassert P { all u: User | u = u }");
    let t = find_file(&files, "models_test.go");
    assert!(t.contains("import \"testing\""));
}
