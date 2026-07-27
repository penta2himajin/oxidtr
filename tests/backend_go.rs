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

// ── Alloy 6: var field ──────────────────────────────────────────────────────

#[test]
fn go_var_field_annotated() {
    // Go fields are mutable by default, so no @alloy: var annotation is needed.
    // The field should still be present without any var-specific comment.
    let files = generate_go(r#"
        sig Account { var balance: one Int }
    "#);
    let m = find_file(&files, "models.go");
    assert!(!m.contains("@alloy: var"),
        "Go var field should not have @alloy: var annotation (fields are mutable by default):\n{m}");
    assert!(m.contains("Balance"),
        "var field should still be present in struct:\n{m}");
}

// ── Binary temporal static test ──────────────────────────────────────────────

#[test]
fn go_binary_temporal_static_test_is_comment_only() {
    let files = generate_go(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let t = find_file(&files, "models_test.go");
    assert!(t.contains("Test_temporal_WaitUntilDone"),
        "should generate temporal test:\n{t}");
    assert!(t.contains("binary temporal: requires trace-based verification"),
        "should document trace-based verification:\n{t}");
}

// ── Disjoint constraint validation ──────────────────────────────────────────

#[test]
fn go_test_generates_disjoint_check() {
    let files = generate_go(r#"
        sig Schedule { morning: set Task, evening: set Task }
        sig Task {}
        fact NoOverlap { no (Schedule.morning & Schedule.evening) }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(tests.contains("Morning"), "test should reference Morning field (Go PascalCase):\n{tests}");
    assert!(tests.contains("Evening"), "test should reference Evening field (Go PascalCase):\n{tests}");
    // The disjoint fact translates through expr_translator using set intersection
    assert!(tests.contains("NoOverlap"),
        "test should generate a test for the disjoint fact:\n{tests}");
}

// ── Derived fields (fun Sig.name → method) ──────────────────────────────────

#[test]
fn go_derived_field_generates_method() {
    let files = generate_go(r#"
        sig Account { deposits: set Int }
        fun Account.balance: one Int { #this.deposits }
    "#);
    let models = find_file(&files, "models.go");
    assert!(models.contains("func (s *Account) Balance()"), "should generate method:\n{models}");
}

// ── Compilable quantifier output (#80) ──────────────────────────────────────

/// Go func literals require an explicit parameter type — `func(p) bool` is a
/// syntax error. The quantifier translation must emit the element type of the
/// domain it iterates.
#[test]
fn go_quantifier_closure_declares_parameter_type() {
    let files = generate_go(r#"
        sig Person { age: one Int }
        assert AllAdults { all p: Person | p.age >= 0 }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        !tests.contains("func(p) bool"),
        "func literal must not omit its parameter type:\n{tests}"
    );
    assert!(
        tests.contains("func(p Person) bool"),
        "expected the domain's element type on the closure parameter:\n{tests}"
    );
}

/// A nested quantifier over a field domain (`all f: s.fields | ...`) must
/// resolve the element type through the field's target, not just bare sigs.
#[test]
fn go_nested_quantifier_over_field_domain_declares_parameter_type() {
    let files = generate_go(r#"
        sig Container { items: set Item }
        sig Item { size: one Int }
        assert AllSized { all c: Container | all i: c.items | i.size >= 0 }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        !tests.contains("func(i) bool") && !tests.contains("func(c) bool"),
        "no func literal may omit its parameter type:\n{tests}"
    );
    assert!(
        tests.contains("func(c Container) bool") && tests.contains("func(i Item) bool"),
        "expected both closure parameters typed:\n{tests}"
    );
}

/// The quantifier translation calls `all(...)` / `any(...)`, so those generic
/// helpers must actually be defined, or nothing compiles.
#[test]
fn go_helpers_define_all_and_any_when_quantifiers_used() {
    let files = generate_go(r#"
        sig Person { age: one Int }
        assert AllAdults { all p: Person | p.age >= 0 }
    "#);
    let helpers = find_file(&files, "helpers.go");
    assert!(
        helpers.contains("func forAll["),
        "generated code calls forAll() — it must be defined:\n{helpers}"
    );
    assert!(
        helpers.contains("func exists["),
        "generated code calls exists() — it must be defined:\n{helpers}"
    );
}

/// An abstract sig with at least one data-carrying variant becomes a Go
/// interface. Fields targeting it still call `DefaultX()`, so that factory
/// must exist — previously it was only emitted when *every* variant was a
/// unit variant, leaving `undefined: DefaultExpr` for mixed hierarchies.
#[test]
fn go_fixture_factory_exists_for_interface_style_abstract_sig() {
    let files = generate_go(r#"
        abstract sig Expr {}
        sig VarRef extends Expr {}
        sig Comparison extends Expr { left: one Expr }
        sig Binding { domain: one Expr }
    "#);
    let fixtures = find_file(&files, "fixtures.go");
    assert!(
        fixtures.contains("func DefaultExpr() Expr"),
        "a field targeting the interface calls DefaultExpr() — it must be defined:\n{fixtures}"
    );
    assert!(
        fixtures.contains("return VarRef{}"),
        "expected the fieldless variant as the default:\n{fixtures}"
    );
}
