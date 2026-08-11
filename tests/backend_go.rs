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

/// A pred used to be emitted as a procedure that panicked. It is a formula, so
/// it returns `bool` and its clauses are translated (#82).
#[test]
fn go_operations_are_boolean_relations() {
    let files = generate_go("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "operations.go");
    assert!(ops.contains("func ChangeRole("));
    assert!(ops.contains(") bool {"), "a pred denotes true or false:\n{ops}");
    assert!(!ops.contains("panic("), "the stub must be gone:\n{ops}");
}

/// `#e` is an Alloy Int, which Go models as int64, but `len` yields `int` —
/// comparing or returning it needs the conversion or it does not compile.
#[test]
fn go_cardinality_converts_to_int64() {
    let files = generate_go(
        "sig Item {}\nsig Box { items: set Item, cap: one Int }\n\
         pred hasRoom[b: one Box] { #b.items < b.cap }",
    );
    let ops = find_file(&files, "operations.go");
    assert!(
        ops.contains("int64(len(b.Items)) < b.Cap"),
        "got:\n{ops}"
    );
}

/// Alloy's implicit receiver is the method's receiver, not a variable named
/// `this` that Go never declared.
#[test]
fn go_derived_field_uses_the_receiver() {
    let files = generate_go(
        "sig Item {}\nsig Box { items: set Item }\nfun Box.count: one Int { #this.items }",
    );
    let m = find_file(&files, "models.go");
    assert!(m.contains("func (s *Box) Count() int64 {"), "got:\n{m}");
    assert!(m.contains("return int64(len(s.Items))"), "got:\n{m}");
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

// ── Adversarial compile correctness (#89 peer review) ───────────────────────
// Each of these produced non-compiling or silently-wrong Go. Root cause for
// most: the translator resolved field types by NAME across all sigs instead of
// through the bound variable's actual type.

/// Two sigs may share a field name with different targets. Resolving `a.items`
/// by name alone is ambiguous, and falling back to `any` makes the closure
/// incompatible with `forAll`'s `func(T) bool`.
#[test]
fn go_same_named_fields_on_different_sigs_resolve_per_binding() {
    let files = generate_go(r#"
        sig Item {}
        sig Other {}
        sig A { items: set Item }
        sig B { items: set Other }
        assert ReflexiveItems { all a: A | all i: a.items | i = i }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("func(i Item) bool"),
        "the domain must resolve through the binding `a: A`, not by field name:\n{tests}"
    );
    assert!(
        !tests.contains("func(i any) bool"),
        "must not degrade to `any` — forAll requires an exact func(T) bool:\n{tests}"
    );
}

/// A map field's `target` is only its KEY type; the Go type is a map, and Go
/// rejects `==` on maps.
#[test]
fn go_map_field_comparison_does_not_use_equality_operator() {
    let files = generate_go(r#"
        sig Value {}
        sig Config { settings: one Int -> Value }
        assert ReflexiveSettings { all c: Config | c.settings = c.settings }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        !tests.contains("c.Settings == c.Settings"),
        "maps can only be compared to nil in Go:\n{tests}"
    );
    assert!(
        tests.contains("equal(c.Settings, c.Settings)"),
        "map comparison must route through the DeepEqual helper:\n{tests}"
    );
}

/// `x in setField` must become a membership test. Picking the first same-named
/// field's multiplicity turned this into an equality check that is always
/// false — vet-clean but silently wrong, the worst failure mode.
#[test]
fn go_membership_uses_binding_type_not_first_same_named_field() {
    let files = generate_go(r#"
        sig Item {}
        sig OptionalBox { item: lone Item }
        sig SetBox { item: set Item }
        assert SetMembership { all b: SetBox | all i: Item | i in b.item }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("contains(b.Item, i)"),
        "`in` over a set field must be a membership test:\n{tests}"
    );
    assert!(
        !tests.contains("equal(b.Item, i)"),
        "must not treat SetBox.item as lone just because OptionalBox.item is:\n{tests}"
    );
}

/// A concrete variant is assignable to its abstract interface, but a generic
/// `contains[T]` forces both arguments to the same static type.
#[test]
fn go_contains_helper_accepts_variant_against_interface_slice() {
    let files = generate_go(r#"
        sig Marker {}
        abstract sig Shape {}
        sig Circle extends Shape { marker: lone Marker }
        sig Square extends Shape {}
        sig Drawing { shapes: set Shape }
        assert CircleMembership { all d: Drawing | all c: Circle | c in d.shapes }
    "#);
    let helpers = find_file(&files, "helpers.go");
    assert!(
        !helpers.contains("func contains[T"),
        "a generic contains rejects Circle against []Shape:\n{helpers}"
    );
    assert!(
        helpers.contains("func contains(xs any, v any) bool"),
        "contains must accept a variant against its interface slice:\n{helpers}"
    );
}

/// Alloy permits quantifying over a singleton relation. `forAll` takes `[]T`,
/// so a `one`/`lone` domain must be lifted into a slice first.
#[test]
fn go_quantification_over_singleton_field_is_lifted_to_slice() {
    let files = generate_go(r#"
        sig Item { marker: lone Int }
        sig A { item: one Item }
        assert ReflexiveItem { all a: A | all i: a.item | i = i }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("oneOf(a.Item)"),
        "a `one` domain must be lifted into a slice for forAll:\n{tests}"
    );
}

/// A field targeting a fieldless sig emits `DefaultX()`, so that factory must
/// exist — previously fieldless sigs were skipped, leaving it undefined.
#[test]
fn go_default_value_for_fieldless_target_constructs_directly() {
    let files = generate_go(r#"
        sig Leaf {}
        abstract sig Shape {}
        sig Circle extends Shape { leaf: one Leaf }
        sig Square extends Shape { leaf: one Leaf }
        sig Drawing { shape: one Shape }
    "#);
    let fixtures = find_file(&files, "fixtures.go");
    assert!(
        fixtures.contains("func DefaultLeaf() Leaf { return Leaf{} }"),
        "a fieldless sig still needs a factory — fields targeting it emit DefaultLeaf():\n{fixtures}"
    );
}

/// `disj` guards were emitted as an `if` statement in expression position,
/// which does not parse as Go.
#[test]
fn go_disjoint_quantifier_emits_boolean_expression_not_if_statement() {
    let files = generate_go(r#"
        sig Tag {}
        sig Person { tags: set Tag }
        assert DistinctPeople { all disj a, b: Person | a != b }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        !tests.contains("if ("),
        "Go's `if` is a statement and cannot be a return expression:\n{tests}"
    );
    assert!(
        !tests.contains("a != b"),
        "Person contains a slice, so `!=` is illegal:\n{tests}"
    );
}

/// Native scalar targets (Int/Str/Bool/Float) are not sigs, so they have no
/// `DefaultX()` factory and no Go type named `Int`. Fields of those types must
/// use Go zero values. Never surfaced before because oxidtr's own model has no
/// `one Int` field.
#[test]
fn go_native_scalar_fields_use_zero_values_not_factories() {
    let files = generate_go(r#"
        sig Node { tag: one Int, name: one Str, ok: one Bool, ratio: one Float, marks: set Int }
    "#);
    let fixtures = find_file(&files, "fixtures.go");
    for bad in ["DefaultInt()", "DefaultStr()", "DefaultBool()", "DefaultFloat()", "[]Int{"] {
        assert!(
            !fixtures.contains(bad),
            "native scalars have no factory and no Go type by that name, found {bad}:\n{fixtures}"
        );
    }
    assert!(fixtures.contains("Tag: 0,"), "expected int zero value:\n{fixtures}");
    assert!(fixtures.contains("Name: \"\","), "expected string zero value:\n{fixtures}");
    assert!(fixtures.contains("Ok: false,"), "expected bool zero value:\n{fixtures}");
    assert!(fixtures.contains("[]int64{}"), "expected a slice of the resolved Go type:\n{fixtures}");
}

/// A transitive-closure domain (`all p: n.^parent | ...`) yields the same sig
/// as the underlying field, and lowers to a slice-returning Tc* helper — so it
/// must type the closure parameter, not degrade to `any`.
#[test]
fn go_transitive_closure_domain_resolves_element_type() {
    let files = generate_go(r#"
        sig Node { parent: lone Node, tag: one Int }
        assert TC { all n: Node | all p: n.^parent | p.tag = p.tag }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("func(p Node) bool"),
        "TC domain must resolve to the field's target sig:\n{tests}"
    );
    assert!(!tests.contains("func(p any) bool"), "must not degrade to any:\n{tests}");
}

// ── Second review round (#89) ───────────────────────────────────────────────

/// A variant with no fields of its own still inherits its abstract parent's,
/// so it is a struct — `return Circle` (a bare type name) is not an expression.
#[test]
fn go_variant_default_accounts_for_inherited_parent_fields() {
    let files = generate_go(r#"
        sig Leaf {}
        abstract sig Shape { leaf: one Leaf }
        sig Circle extends Shape {}
        sig Square extends Shape {}
        sig Drawing { shape: one Shape }
    "#);
    let fixtures = find_file(&files, "fixtures.go");
    assert!(
        !fixtures.contains("return Circle }") && !fixtures.contains("return Circle\n"),
        "Circle inherits `leaf`, so it must be constructed, not named:\n{fixtures}"
    );
    assert!(
        fixtures.contains("Circle{"),
        "expected a constructed variant:\n{fixtures}"
    );
}

/// Bindings in one quantifier are sequential: `all b: Box, x: b.items | ...`
/// binds `b` before `x`'s domain is resolved.
#[test]
fn go_dependent_bindings_in_one_quantifier_resolve_sequentially() {
    let files = generate_go(r#"
        sig Item {}
        sig Box { items: set Item }
        assert R { all b: Box, x: b.items | x = x }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("func(x Item) bool"),
        "`x: b.items` must resolve through the earlier binding `b: Box`:\n{tests}"
    );
}

/// A field declared on an abstract parent is reachable through the child.
#[test]
fn go_inherited_field_resolves_through_child_sig() {
    let files = generate_go(r#"
        sig Item {}
        abstract sig Parent { items: set Item }
        sig Child extends Parent { marker: lone Item }
        assert R { all c: Child | all i: c.items | i = i }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("func(i Item) bool"),
        "`items` is inherited from Parent and must still resolve:\n{tests}"
    );
}

/// `disj` applies within each declaration group, not across groups: in
/// `all disj a,b: S, disj c,d: S | ...`, `a` and `c` may be equal.
#[test]
fn go_disjoint_groups_do_not_impose_cross_group_distinctness() {
    let files = generate_go(r#"
        sig S { id: one Int }
        assert R { all disj a, b: S, disj c, d: S | a = c }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("!equal(a, b)") && tests.contains("!equal(c, d)"),
        "each group must be pairwise distinct:\n{tests}"
    );
    assert!(
        !tests.contains("!equal(a, c)") && !tests.contains("!equal(b, d)"),
        "distinctness must not be imposed across separate disj groups:\n{tests}"
    );
}

/// `x in oneField` is membership in a singleton relation — true iff equal.
/// Routing it through the slice-only `contains` made it always false.
#[test]
fn go_membership_in_one_relation_is_equality_not_slice_contains() {
    let files = generate_go(r#"
        sig Item {}
        sig Box { item: one Item }
        assert R { all b: Box | b.item in b.item }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("equal(b.Item, b.Item)"),
        "a `one` relation's membership is equality:\n{tests}"
    );
    assert!(
        !tests.contains("contains(b.Item"),
        "contains() takes a slice; b.Item is a single value:\n{tests}"
    );
}

/// Transitive closure must terminate on a cyclic graph. All three
/// multiplicities were unsafe: `lone` looped forever on a self-reference,
/// `set` keyed its visited map on `len(result)` (always new), and `one`
/// simply emitted a hardcoded 1000 iterations.
#[test]
fn go_transitive_closure_is_cycle_safe() {
    let files = generate_go(r#"
        sig Node { parent: lone Node }
        assert R { all n: Node | all x: n.^parent | x = x }
    "#);
    let helpers = find_file(&files, "helpers.go");
    assert!(
        !helpers.contains("i < 1000"),
        "a hardcoded iteration cap is not a closure:\n{helpers}"
    );
    assert!(
        !helpers.contains("idx := len(result)"),
        "keying `seen` on len(result) never repeats and never terminates a cycle:\n{helpers}"
    );
    assert!(
        helpers.contains("seen[") || helpers.contains("contains(result"),
        "closure must track visited nodes to terminate on a cycle:\n{helpers}"
    );
}

/// A transitive-closure field is self-referential by definition, so a `one`
/// one is boxed to `*T` — the closure must deref it, and the default fixture
/// must be nil rather than an infinitely recursive construction. Previously
/// only the `lone` shape was covered, so this never compiled.
#[test]
fn go_one_multiplicity_transitive_closure_compiles() {
    let files = generate_go(r#"
        sig Node { next: one Node }
        assert R { all n: Node | all x: n.^next | x = x }
    "#);
    let helpers = find_file(&files, "helpers.go");
    assert!(
        helpers.contains("result = append(result, *current)"),
        "a boxed `one` self-reference must be dereferenced before append:\n{helpers}"
    );
    let fixtures = find_file(&files, "fixtures.go");
    assert!(
        fixtures.contains("Next: nil"),
        "a self-referential `one` field cannot be built eagerly:\n{fixtures}"
    );
}

/// `setA in setB` is subset containment in Alloy, not element membership.
#[test]
fn go_set_in_set_is_subset_not_element_membership() {
    let files = generate_go(r#"
        sig Item {}
        sig Box { xs: set Item, ys: set Item }
        assert R { all b: Box | b.xs in b.ys }
    "#);
    let tests = find_file(&files, "models_test.go");
    assert!(
        tests.contains("isSubset(b.Xs, b.Ys)"),
        "set-to-set `in` is subset containment:\n{tests}"
    );
    assert!(
        !tests.contains("contains(b.Ys, b.Xs)"),
        "must not treat a whole set as a single element:\n{tests}"
    );
}

// ── Assert domains must not be empty (#81) ─────────────────────────────────

/// An `assert` test whose quantified domain is initialised empty passes
/// regardless of the model or the implementation — the failure #74/#75 fixed on
/// the Rust `fact` path, still live on the `assert` path here.
#[test]
fn go_assert_domain_is_seeded_from_the_fixture() {
    let files = generate_go(
        "sig Person { age: one Int }\nassert AllAdults { all p: Person | p.age >= 0 }",
    );
    let src = find_file(&files, "models_test.go");

    assert!(
        src.contains("[]Person{DefaultPerson()}"),
        "the assert domain must contain an instance. got:\n{src}"
    );
    assert!(
        !src.contains("persons := []Person{}"),
        "an empty domain makes the quantifier vacuously true. got:\n{src}"
    );
}

/// A domain with no fixture stays empty but says so — the "single-point check"
/// disclosure precedent from #74.
#[test]
fn go_discloses_an_empty_assert_domain() {
    let files = generate_go(
        "abstract sig Shape {}\nsig Circle extends Shape {}\nsig Person { age: one Int }\n\
         assert Mixed { all s: Shape | all p: Person | p.age >= 0 }",
    );
    let src = find_file(&files, "models_test.go");

    assert!(src.contains("[]Person{DefaultPerson()}"), "got:\n{src}");
    assert!(src.contains("@coverage"), "an empty domain must be disclosed. got:\n{src}");
}

// ── A field targeting a variant of an abstract sig (#93) ───────────────────

const VARIANT_FIELD_MODEL: &str = "\
sig Item {}
abstract sig Parent { items: set Item }
sig Child extends Parent {}
sig Holder { child: one Child }
";

/// Go emits a variant as a real struct, so the field type is fine — but the
/// fixture generator excludes variants, so `DefaultChild()` was referenced and
/// never defined (`undefined: DefaultChild`).
#[test]
fn go_emits_a_fixture_for_a_variant_used_as_a_field_type() {
    let files = generate_go(VARIANT_FIELD_MODEL);
    let fixtures = find_file(&files, "fixtures.go");

    assert!(
        fixtures.contains("func DefaultChild() Child"),
        "a variant used as a field type needs a factory:\n{fixtures}"
    );
}
