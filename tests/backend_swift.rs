use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::swift;
use oxidtr::backend::GeneratedFile;

fn generate_swift(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    swift::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── Models.swift ─────────────────────────────────────────────────────────────

#[test]
fn swift_struct_for_sig() {
    let files = generate_swift("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("struct User: Equatable {"));
    assert!(m.contains("let name: Role"));
}

#[test]
fn swift_optional_for_lone() {
    let files = generate_swift("sig Node { parent: lone Node }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("let parent: Node?"));
}

#[test]
fn swift_set_for_set() {
    let files = generate_swift("sig Group { members: set User }\nsig User {}");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("let members: Set<User>"));
}

#[test]
fn swift_array_for_seq() {
    let files = generate_swift("sig Order { items: seq Item }\nsig Item {}");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("let items: [Item]"));
}

#[test]
fn swift_enum_for_all_singleton() {
    let files = generate_swift(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("enum Color: Equatable, Hashable, CaseIterable {"));
    assert!(m.contains("case red"));
    assert!(m.contains("case blue"));
}

#[test]
fn swift_enum_with_associated_values() {
    let files = generate_swift(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("enum Expr: Equatable {"));
    assert!(m.contains("case binOp(left: Expr, right: Expr)"));
    assert!(m.contains("case literal"));
}

// ── Operations.swift ─────────────────────────────────────────────────────────

#[test]
fn swift_operations_use_fatalerror() {
    let files = generate_swift("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "Operations.swift");
    assert!(ops.contains("func changeRole("));
    assert!(ops.contains("fatalError("));
}

#[test]
fn swift_operations_return_type() {
    let files = generate_swift("sig User {}\nfun findUser[name: one User]: one User { name = name }");
    let ops = find_file(&files, "Operations.swift");
    assert!(ops.contains("-> User"));
}

// ── Tests.swift ──────────────────────────────────────────────────────────────

#[test]
fn swift_tests_inline_constraint_expressions() {
    let files = generate_swift(
        "sig User { roles: set Role }\nsig Role {}\nfact AllUsersHaveRoles { all u: User | #u.roles > 0 }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("XCTAssertTrue("));
    assert!(t.contains(".allSatisfy"));
}

#[test]
fn swift_tests_generated_properly() {
    // Constraint with cardinality check — Swift should generate test
    let files = generate_swift(
        "sig User { roles: set Role }\nsig Role {}\nfact UserHasRoles { all u: User | #u.roles > 0 }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("func test_invariant_"));
    assert!(t.contains("XCTAssertTrue("));
}

// ── Fixtures.swift ───────────────────────────────────────────────────────────

#[test]
fn swift_fixtures_generated() {
    let files = generate_swift("sig User { name: one Role, group: lone Group }\nsig Role {}\nsig Group {}");
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultUser()"));
    assert!(f.contains("-> User"));
    assert!(f.contains("nil"));
}

#[test]
fn swift_fixtures_enum_default() {
    let files = generate_swift(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultColor()"));
    assert!(f.contains(".red"));
}

#[test]
fn swift_fixtures_boundary() {
    let files = generate_swift(
        "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 5 }",
    );
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func boundaryTeam()"));
    assert!(f.contains("func invalidTeam()"));
}

// ── Helpers.swift ────────────────────────────────────────────────────────────

#[test]
fn swift_helpers_for_tc() {
    let files = generate_swift(
        "sig Node { parent: lone Node }\nassert Acyclic { all n: Node | not (n in n.^parent) }",
    );
    let h = files.iter().find(|f| f.path == "Helpers.swift");
    assert!(h.is_some(), "Helpers.swift should be generated for TC");
    let h = h.unwrap();
    assert!(h.content.contains("func tcParent("));
    assert!(h.content.contains("while let node = current"));
}

// ── Cross-tests ──────────────────────────────────────────────────────────────

#[test]
fn swift_cross_tests_are_disabled() {
    let files = generate_swift(
        "sig User { name: one Role }\nsig Role {}\nfact F { all u: User | u = u }\npred doSomething[u: one User] { u = u }",
    );
    let t = find_file(&files, "Tests.swift");
    if t.contains("Cross-tests") {
        assert!(t.contains("disabled_test_"), "Swift cross-tests should be disabled via naming convention");
    }
}

// ── Import statements ────────────────────────────────────────────────────────

#[test]
fn swift_models_import_foundation() {
    let files = generate_swift("sig User {}");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("import Foundation"));
}

#[test]
fn swift_tests_import_xctest() {
    let files = generate_swift("sig User {}\nassert P { all u: User | u = u }");
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("import XCTest"));
    assert!(t.contains("XCTestCase"));
}

// ── Alloy 6: var field ──────────────────────────────────────────────────────

#[test]
fn swift_var_field_uses_var_keyword() {
    let files = generate_swift(r#"
        sig Account { var balance: one Int }
    "#);
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("var balance:"),
        "var field should use 'var' instead of 'let' in Swift:\n{m}");
    assert!(!m.contains("let balance:"),
        "var field should NOT use 'let' in Swift:\n{m}");
}

// ── Binary temporal static test ──────────────────────────────────────────────

#[test]
fn swift_binary_temporal_static_test_is_comment_only() {
    let files = generate_swift(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let tests = find_file(&files, "Tests.swift");
    assert!(tests.contains("test_temporal_WaitUntilDone"),
        "should generate temporal test:\n{tests}");
    assert!(tests.contains("binary temporal: requires trace-based verification"),
        "should document trace-based verification:\n{tests}");
}

// ── Disjoint constraint validation ──────────────────────────────────────────

#[test]
fn swift_test_generates_disjoint_check() {
    let files = generate_swift(r#"
        sig Schedule { morning: set Task, evening: set Task }
        sig Task {}
        fact NoOverlap { no (Schedule.morning & Schedule.evening) }
    "#);
    let tests = find_file(&files, "Tests.swift");
    assert!(tests.contains("morning"), "test should reference morning field:\n{tests}");
    assert!(tests.contains("evening"), "test should reference evening field:\n{tests}");
    // The disjoint fact translates through expr_translator using set intersection
    assert!(tests.contains("isDisjoint") || tests.contains("intersection"),
        "test should check disjoint using set operations:\n{tests}");
}

// ── Derived fields (fun Sig.name → computed property) ───────────────────────

#[test]
fn swift_derived_field_generates_computed_property() {
    let files = generate_swift(r#"
        sig Account { deposits: set Int }
        fun Account.balance: one Int { #this.deposits }
    "#);
    let models = find_file(&files, "Models.swift");
    assert!(models.contains("var balance: Int"), "should generate computed property:\n{models}");
}
