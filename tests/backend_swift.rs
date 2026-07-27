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

/// Slice one generated function out of a file. Asserting against a whole file
/// lets a claim about `defaultX` be satisfied by `anomalyEmptyX` instead.
fn func_body<'a>(src: &'a str, header: &str) -> &'a str {
    let start = src.find(header)
        .unwrap_or_else(|| panic!("no `{header}` in:\n{src}"));
    let rest = &src[start + header.len()..];
    let end = rest.find("\nfunc ").unwrap_or(rest.len());
    &rest[..end]
}

// ── Models.swift ─────────────────────────────────────────────────────────────

#[test]
fn swift_struct_for_sig() {
    let files = generate_swift("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("struct User: Equatable, Hashable {"));
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
    assert!(m.contains("enum Expr: Equatable, Hashable {"));
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

// ── Regression: enum-member references in constraint expressions ──────────────

#[test]
fn swift_enum_member_ref_in_assertion_is_qualified() {
    // A `one sig` member lowered into an enum case must be referenced as
    // `Enum.case` in generated expressions, not as the bare Alloy sig name
    // (which is undefined in Swift and fails to compile).
    let files = generate_swift(
        "abstract sig Level {}\n\
         one sig Low extends Level {}\n\
         one sig High extends Level {}\n\
         sig Node { level: one Level }\n\
         assert HighNotLow { all n: Node | n.level = High implies n.level != Low }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("Level.high"), "expected qualified enum case, got:\n{t}");
    assert!(t.contains("Level.low"), "expected qualified enum case, got:\n{t}");
    assert!(!t.contains("== High"), "bare Alloy sig name leaked into Swift:\n{t}");
    assert!(!t.contains("!= Low"), "bare Alloy sig name leaked into Swift:\n{t}");
}

#[test]
fn swift_tests_call_bare_fixtures_and_fixtures_are_free_functions() {
    // Generated Swift must be self-consistent: fixtures are free functions in the
    // same module, so tests must call them bare (no `Fixtures.` namespace that is
    // never defined), and no fixture may be emitted as a top-level `static func`
    // (invalid Swift outside a type).
    let files = generate_swift(
        "abstract sig Level {}\n\
         one sig Low extends Level {}\n\
         one sig High extends Level {}\n\
         sig Bag { items: set Item, level: one Level }\n\
         sig Item {}\n\
         assert Ok { all b: Bag | b.level = High implies b.level != Low }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(!t.contains("Fixtures."), "tests must call fixtures bare, got:\n{t}");
    let f = find_file(&files, "Fixtures.swift");
    assert!(!f.contains("static func"), "fixtures must be free functions, got:\n{f}");
    assert!(f.contains("func anomalyEmptyBag()"), "expected free anomaly fixture, got:\n{f}");
}

// ── #88: generated Swift must actually compile ───────────────────────────────

#[test]
fn swift_recursive_struct_becomes_class() {
    // A struct that transitively stores itself by value has infinite size in
    // Swift. Break the cycle by emitting a reference type.
    let files = generate_swift("sig Node { parent: lone Node }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("final class Node: Equatable, Hashable {"), "got:\n{m}");
    assert!(m.contains("let parent: Node?"), "field type must be unchanged:\n{m}");
    assert!(m.contains("init(parent: Node?) {"), "class needs an explicit memberwise init:\n{m}");
    assert!(m.contains("static func == (lhs: Node, rhs: Node) -> Bool"), "class needs ==:\n{m}");
    assert!(m.contains("func hash(into hasher: inout Hasher)"), "class needs hash(into:):\n{m}");
}

#[test]
fn swift_mutually_recursive_structs_become_classes() {
    let files = generate_swift("sig A { b: one B }\nsig B { a: lone A }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("final class A:"), "got:\n{m}");
    assert!(m.contains("final class B:"), "got:\n{m}");
}

#[test]
fn swift_self_reference_through_collection_stays_struct() {
    // Set/Array are heap-allocated, so `Set<Tree>` does not make Tree infinite.
    let files = generate_swift("sig Tree { children: set Tree }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("struct Tree: Equatable, Hashable {"), "got:\n{m}");
    assert!(!m.contains("final class Tree"), "must not over-convert:\n{m}");
}

#[test]
fn swift_recursive_enum_is_indirect() {
    let files = generate_swift(
        "abstract sig Expr {}\nsig Lit extends Expr {}\nsig Neg extends Expr { inner: one Expr }",
    );
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("indirect enum Expr:"), "got:\n{m}");
}

#[test]
fn swift_non_recursive_enum_is_not_indirect() {
    let files = generate_swift("abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}");
    let m = find_file(&files, "Models.swift");
    assert!(!m.contains("indirect"), "must not over-convert:\n{m}");
}

#[test]
fn swift_keyword_enum_cases_are_escaped() {
    let files = generate_swift(
        "abstract sig Op {}\none sig In extends Op {}\none sig Default extends Op {}",
    );
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("case `in`"), "Swift keyword case must be escaped:\n{m}");
    assert!(m.contains("case `default`"), "Swift keyword case must be escaped:\n{m}");
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains(".`in`"), "escaped case must be used at reference sites too:\n{f}");
}

#[test]
fn swift_keyword_field_names_are_escaped() {
    let files = generate_swift("sig Val {}\nsig Cfg { default: one Val }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("let `default`: Val"), "Swift keyword field must be escaped:\n{m}");
}

#[test]
fn swift_structs_are_hashable_so_set_fields_typecheck() {
    // `Set<T>` requires `T: Hashable`; declaring only Equatable makes every
    // set-valued field a compile error.
    let files = generate_swift("sig Item { tag: one Int }\nsig Box { items: set Item }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("struct Item: Equatable, Hashable {"), "got:\n{m}");
    assert!(m.contains("struct Box: Equatable, Hashable {"), "got:\n{m}");
}

#[test]
fn swift_payload_enum_is_hashable() {
    let files = generate_swift(
        "sig Leaf {}\nabstract sig Shape {}\nsig Circle extends Shape { leaf: one Leaf }\nsig Square extends Shape {}",
    );
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("enum Shape: Equatable, Hashable {"), "got:\n{m}");
}

#[test]
fn swift_fixture_for_field_less_sig() {
    // A `one Val` field's fixture calls defaultVal(), so a sig with no fields
    // still needs a factory (Rust already emits one).
    let files = generate_swift("sig Val {}\nsig Cfg { x: one Val }");
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultVal() -> Val { Val() }"), "got:\n{f}");
}

#[test]
fn swift_fixture_uses_literals_for_native_types() {
    // Int/Str/Bool have no generated factory, so `one Int` must be a literal.
    let files = generate_swift("sig Node { tag: one Int, name: one Str, ok: one Bool }");
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("tag: 0"), "got:\n{f}");
    assert!(f.contains("name: \"\""), "got:\n{f}");
    assert!(f.contains("ok: false"), "got:\n{f}");
    assert!(!f.contains("defaultInt()"), "no factory exists for native types:\n{f}");
}

#[test]
fn swift_enum_payload_resolves_native_types() {
    let files = generate_swift(
        "abstract sig Tok {}\nsig Word extends Tok { text: one Str }\nsig Num extends Tok { n: one Int }",
    );
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("case word(text: String)"), "Str must resolve to String:\n{m}");
    assert!(m.contains("case num(n: Int)"), "got:\n{m}");
}

#[test]
fn swift_singleton_sig_is_hashable() {
    // `one sig` lowers to a struct with a `shared` instance; a `set Marker`
    // field elsewhere makes it a Set element, which requires Hashable.
    let files = generate_swift("one sig Marker {}\nsig Box { ms: set Marker }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("struct Marker: Equatable, Hashable {"), "got:\n{m}");
    assert!(m.contains("static let shared = Marker()"), "got:\n{m}");
}

// ── #92 peer review (Codex) ──────────────────────────────────────────────────

#[test]
fn swift_recursive_class_uses_identity_equality() {
    // A field-by-field `==` recurses forever on a cyclic instance, and hashing
    // a `var` field breaks Set invariants once it is mutated. An Alloy atom is
    // its identity, so compare identities.
    let files = generate_swift("sig Node { var parent: lone Node }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("        lhs === rhs"), "got:\n{m}");
    assert!(m.contains("hasher.combine(ObjectIdentifier(self))"), "got:\n{m}");
    assert!(!m.contains("lhs.parent == rhs.parent"), "structural == is the bug:\n{m}");
}

#[test]
fn swift_enum_map_payload_resolves_native_types() {
    let files = generate_swift("abstract sig Choice {}\nsig Entry extends Choice { values: one Int -> Str }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("case entry(values: [Int: String])"), "Str must resolve to String:\n{m}");
}

#[test]
fn swift_type_and_protocol_are_escaped_as_members() {
    // Swift rejects a type member literally named `Type` or `Protocol`.
    let files = generate_swift("sig Val {}\nsig Cfg { Type: one Val, Protocol: lone Val }");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("let `Type`: Val"), "got:\n{m}");
    assert!(m.contains("let `Protocol`: Val?"), "got:\n{m}");
}

#[test]
fn swift_negated_lone_membership_is_parenthesized() {
    // `not (n in n.parent)` must be `!(n.parent == n)`, not `!n.parent == n`.
    let files = generate_swift(
        "sig Node { parent: lone Node }\nfact NoSelf { all n: Node | not (n in n.parent) }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("!(n.parent == n)"), "got:\n{t}");
}

#[test]
fn swift_enum_fixture_avoids_transitively_recursive_case() {
    // `.wrap` reaches Expr again through Node, so defaultExpr() would not
    // terminate; the unit case must win.
    let files = generate_swift(
        "abstract sig Expr {}\nsig Wrap extends Expr { node: one Node }\nsig Leaf extends Expr {}\nsig Node { expr: one Expr }",
    );
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultExpr() -> Expr { .leaf }"), "got:\n{f}");
}

#[test]
fn swift_membership_in_payload_variant_uses_case_test() {
    // `Expr.lit` is a constructor, not a value — `Expr.lit.contains(e)` does
    // not compile. Membership in a payload-bearing subsig is a case test.
    let files = generate_swift(
        "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
         sig Other extends Expr {}\nassert IsLiteral { all e: Expr | e in Lit }",
    );
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("var isLit: Bool {"), "got:\n{m}");
    assert!(m.contains("if case .lit = self { return true }"), "got:\n{m}");
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("e.isLit"), "got:\n{t}");
    assert!(!t.contains("Expr.lit.contains"), "constructor cannot be searched:\n{t}");
}

#[test]
fn swift_unit_variant_membership_stays_a_value_comparison() {
    // A case with no payload *is* a value, so it needs no case test.
    let files = generate_swift(
        "abstract sig Level {}\none sig Low extends Level {}\none sig High extends Level {}\n\
         sig Node { level: one Level }\nassert HighOnly { all n: Node | n.level = High }",
    );
    let m = find_file(&files, "Models.swift");
    assert!(!m.contains("var isHigh"), "unit cases need no case test:\n{m}");
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("Level.high"), "got:\n{t}");
}

// ── #92 peer review round 2 (Codex) ─────────────────────────────────────────

#[test]
fn swift_enum_fixture_picks_a_constructible_case() {
    // ViaInner *is* constructible because defaultInner() picks `.safe`;
    // a reachability check that ignores that rejects both cases and falls
    // back to the recursive one.
    let files = generate_swift(
        "abstract sig Expr {}\nsig Loop extends Expr { expr: one Expr }\n\
         sig ViaInner extends Expr { inner: one Inner }\n\
         abstract sig Inner {}\nsig Safe extends Inner {}\nsig Back extends Inner { expr: one Expr }",
    );
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultExpr() -> Expr { .viaInner(inner: defaultInner()) }"), "got:\n{f}");
    assert!(!f.contains(".loop(expr: defaultExpr())"), "picked the diverging case:\n{f}");
}

#[test]
fn swift_unconstructible_sig_traps_instead_of_recursing() {
    let files = generate_swift("sig Node { next: one Node }");
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultNode() -> Node { fatalError("), "got:\n{f}");
    assert!(!f.contains("next: defaultNode()"), "non-terminating fixture:\n{f}");
}

#[test]
fn swift_equality_against_payload_variant_uses_case_test() {
    let files = generate_swift(
        "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
         sig Other extends Expr {}\nassert A { all e: Expr | e = Lit }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("e.isLit"), "got:\n{t}");
    assert!(!t.contains("== Expr.lit"), "a case constructor is not a value:\n{t}");
}

#[test]
fn swift_skips_tests_referencing_a_case_constructor_as_a_value() {
    // `e in Lit + Ref` has no rendering without payload destructuring; emitting
    // `Expr.lit.union(...)` would not compile.
    let files = generate_swift(
        "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
         sig Ref extends Expr { rname: one Name }\nassert A { all e: Expr | e in Lit + Ref }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("skipped test_A"), "got:\n{t}");
    assert!(!t.contains("Expr.lit.union"), "got:\n{t}");
}

#[test]
fn swift_skips_tests_with_ambiguously_typed_field() {
    // `rel` is `set` in one sig and `lone` in another; resolving by name picks
    // one and emits `Optional.contains`.
    let files = generate_swift(
        "sig Item {}\nsig Many { rel: set Item }\nsig Maybe { rel: lone Item }\n\
         fact NoMember { all m: Maybe | all i: Item | not (i in m.rel) }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("different multiplicities"), "got:\n{t}");
    assert!(!t.contains("m.rel.contains(i)"), "wrong branch for a lone field:\n{t}");
}

#[test]
fn swift_anomaly_fixture_escapes_reserved_argument_labels() {
    let files = generate_swift(
        "sig Val {}\nsig Node { inout: one Val, values: set Val, next: lone Node }\n\
         assert Reflexive { all n: Node | n = n }",
    );
    let f = find_file(&files, "Fixtures.swift");
    let anomaly = func_body(f, "func anomalyEmptyNode() -> Node {");
    assert!(anomaly.contains("`inout`: defaultVal()"), "got:\n{anomaly}");
}

#[test]
fn swift_boundary_set_of_natives_has_distinct_elements() {
    // Repeating one literal collapses the Set, so #marks = 2 never holds.
    let files = generate_swift("sig Box { marks: set Int }\nfact ExactlyTwo { all b: Box | #b.marks = 2 }");
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("marks: Set([0, 1])"), "got:\n{f}");
    assert!(!f.contains("defaultInt()"), "no factory exists for Int:\n{f}");
}

// ── #92 peer review round 3 (Codex) ─────────────────────────────────────────

#[test]
fn swift_enum_witness_case_is_not_the_recursive_one() {
    // Once Expr is known constructible (via ViaInner), a naive check finds
    // `.loop(expr: defaultExpr())` satisfiable too. Selection must use the case
    // that made the enum constructible in the first place.
    let files = generate_swift(
        "abstract sig Expr {}\nsig Loop extends Expr { expr: one Expr }\n\
         sig ViaInner extends Expr { inner: one Inner }\n\
         abstract sig Inner {}\nsig Safe extends Inner {}\nsig Back extends Inner { expr: one Expr }",
    );
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultExpr() -> Expr { .viaInner(inner: defaultInner()) }"), "got:\n{f}");
}

#[test]
fn swift_set_valued_equality_against_variant_is_not_a_case_test() {
    // `Lit = b.exprs` compares against a Set, which has no `isLit`.
    let files = generate_swift(
        "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
         one sig Other extends Expr {}\nsig Box { exprs: set Expr }\n\
         assert A { all b: Box | Lit = b.exprs }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(!t.contains("exprs.isLit"), "Set has no case test:\n{t}");
    assert!(t.contains("is a case constructor"), "expected a skip:\n{t}");
}

#[test]
fn swift_case_ref_guard_matches_whole_identifiers() {
    // `Expr.lit` is a prefix of the perfectly valid `Expr.literal`.
    let files = generate_swift(
        "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
         one sig Literal extends Expr {}\nassert A { all e: Expr | e = Literal }",
    );
    let t = find_file(&files, "Tests.swift");
    assert!(t.contains("e == Expr.literal"), "valid test was skipped:\n{t}");
}

#[test]
fn swift_mutually_recursive_sets_are_still_constructible() {
    // `A(bs: [])` is finite — neither sig may be written off as unbuildable.
    let files = generate_swift("sig A { bs: set B }\nsig B { as: set A }");
    let f = find_file(&files, "Fixtures.swift");
    assert!(f.contains("func defaultA() -> A {"), "got:\n{f}");
    assert!(!f.contains("func defaultA() -> A { fatalError"), "A is constructible:\n{f}");
}

#[test]
fn swift_nested_enum_gets_its_own_declaration() {
    // An abstract child of an abstract sig is itself an enum, so suppressing it
    // as a "variant" leaves `defaultInner() -> Inner` with no `Inner` type.
    let files = generate_swift("abstract sig Outer {}\nabstract sig Inner extends Outer {}\nsig Leaf extends Inner {}");
    let m = find_file(&files, "Models.swift");
    assert!(m.contains("enum Inner:"), "nested enum must be declared:\n{m}");
    assert!(m.contains("case leaf"), "got:\n{m}");
}

#[test]
fn swift_set_is_not_seeded_with_a_trapping_factory() {
    // defaultNode() is a fatalError, so seeding Box.nodes with it would trap.
    let files = generate_swift(
        "abstract sig Expr {}\nsig ViaBox extends Expr { box: one Box }\n\
         sig Box { nodes: set Node }\nsig Node { next: one Node }",
    );
    let f = find_file(&files, "Fixtures.swift");
    let boxed = func_body(f, "func defaultBox() -> Box {");
    assert!(boxed.contains("nodes: Set()"), "got:\n{boxed}");
}

#[test]
fn swift_native_defaults_are_in_the_default_fixture_itself() {
    // Not merely somewhere in the file — `anomalyEmptyNode` has the same shape.
    let files = generate_swift("sig Node { tag: one Int, name: one Str, ok: one Bool, marks: set Int }");
    let f = find_file(&files, "Fixtures.swift");
    let default_node = func_body(f, "func defaultNode() -> Node {");
    assert!(default_node.contains("tag: 0"), "got:\n{default_node}");
    assert!(default_node.contains("name: \"\""), "got:\n{default_node}");
    assert!(default_node.contains("ok: false"), "got:\n{default_node}");
}

#[test]
fn swift_case_ref_guard_is_asymmetric_about_dots() {
    // A dot *before* the match means a longer path (`h.myExpr.lit` is fine);
    // a dot *after* it is member access on the constructor (`Expr.lit.name`).
    let path = generate_swift(
        "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
         sig Value { lit: one Int }\nsig Holder { myExpr: one Value }\n\
         assert A { all h: Holder | h.myExpr.lit = h.myExpr.lit }",
    );
    let t = find_file(&path, "Tests.swift");
    assert!(t.contains("h.myExpr.lit == h.myExpr.lit"), "valid test was skipped:\n{t}");

    let ctor = generate_swift(
        "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
         sig Other extends Expr {}\nassert A { Lit.name = Lit.name }",
    );
    let t = find_file(&ctor, "Tests.swift");
    assert!(t.contains("skipped test_A"), "member access on a constructor:\n{t}");
    assert!(!t.contains("Expr.lit.name"), "got:\n{t}");
}
