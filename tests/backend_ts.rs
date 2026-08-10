use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::typescript;
use oxidtr::backend::GeneratedFile;

fn generate_from(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    typescript::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

#[test]
fn ts_generates_interface_for_sig() {
    let files = generate_from("sig User { name: one Role }\nsig Role {}");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("export interface User {"));
    assert!(models.contains("name: Role;"));
    assert!(models.contains("export interface Role {}"));
}

#[test]
fn ts_generates_lone_as_nullable() {
    let files = generate_from("sig Node { parent: lone Node }");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("parent: Node | null;"));
}

#[test]
fn ts_generates_set_as_array() {
    let files = generate_from("sig Group { members: set User }\nsig User {}");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("members: Set<User>;"));
}

#[test]
fn ts_generates_string_literal_union_for_all_singleton_enum() {
    let files = generate_from(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let models = find_file(&files, "models.ts");
    assert!(models.contains("export type Color = \"Red\" | \"Blue\";"));
}

#[test]
fn ts_generates_discriminated_union_for_enum_with_fields() {
    let files = generate_from(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let models = find_file(&files, "models.ts");
    assert!(models.contains("kind: \"BinOp\";"));
    assert!(models.contains("left: Expr;"));
    assert!(models.contains("export type Expr = Literal | BinOp;"));
}

/// A pred used to be emitted as a `void` function that threw. It is a formula,
/// so it returns `boolean` and its clauses are translated (#82).
#[test]
fn ts_generates_operations_as_boolean_relations() {
    let files = generate_from(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("export function changeRole("));
    assert!(ops.contains("): boolean {"), "a pred denotes true or false:\n{ops}");
    assert!(!ops.contains("throw new Error"), "the stub must be gone:\n{ops}");
}

#[test]
fn ts_no_invariants_file() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    assert!(!files.iter().any(|f| f.path == "invariants.ts"),
        "should NOT generate invariants.ts");
}

#[test]
fn ts_generates_vitest_tests() {
    let files = generate_from(
        "sig User {}\nassert AllUsersExist { all u: User | u = u }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("import { describe, it, expect }"));
    assert!(tests.contains("describe('property tests'"));
    assert!(tests.contains("expect("));
}

#[test]
fn ts_tests_inline_every_some_includes() {
    let files = generate_from(
        "sig Item { tags: set Tag }\nsig Tag {}\nfact Tagged { all i: Item | some t: Tag | t in i.tags }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains(".every("), "expected .every() in tests");
    assert!(tests.contains(".some("), "expected .some() in tests");
}

#[test]
fn ts_tests_inline_size_for_cardinality() {
    let files = generate_from(
        "sig Box { items: set Item }\nsig Item {}\nfact Bounded { all b: Box | #b.items = #b.items }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains(".size"), "expected .size for Set cardinality in tests");
}

#[test]
fn ts_self_hosting_model_generates() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    let files = typescript::generate(&ir);

    let models = find_file(&files, "models.ts");
    assert!(models.contains("export interface SigDecl"));
    assert!(models.contains("export interface OxidtrIR"));
    assert!(models.contains("export type Multiplicity"));
    assert!(models.contains("export type Expr ="));

    // No invariants.ts file should be generated
    assert!(!files.iter().any(|f| f.path == "invariants.ts"),
        "should NOT generate invariants.ts for self-hosting model");

    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("lowerOneSig"));
}

// ── Operations JSDoc from body ──────────────────────────────────────────────

#[test]
fn ts_operations_jsdoc_from_body() {
    let files = generate_from(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u.r = r }",
    );
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("/**"), "expected JSDoc comment:\n{ops}");
    assert!(ops.contains("@pre"), "expected @pre tag:\n{ops}");
}

#[test]
fn ts_operations_jsdoc_body_content() {
    let files = generate_from(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "operations.ts");
    // The body u = u should be described
    assert!(ops.contains("@pre u = u"), "expected body description:\n{ops}");
}

// ── Non-vacuous test generation ─────────────────────────────────────────────

#[test]
fn ts_tests_import_fixtures() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("import * as fix from './fixtures'"),
        "tests should import fixtures:\n{tests}");
}

#[test]
fn ts_tests_use_fixture_factory_for_sig_with_fields() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("fix.defaultUser()"),
        "test should use fixture factory for User:\n{tests}");
}

#[test]
fn ts_tests_empty_array_for_sig_without_fields() {
    let files = generate_from(
        "sig Token {}\nassert AllTokens { all t: Token | t = t }",
    );
    let tests = find_file(&files, "tests.ts");
    // Token has no fields → no fixture factory → stays as empty array
    assert!(tests.contains("Token[] = []"),
        "test should use empty array for Token (no fields):\n{tests}");
}

// ── Feature 1: Fun return type in TS ────────────────────────────────────────

#[test]
fn ts_fun_return_type_one() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun getRole[u: one User]: one Role { u }
    "#);
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("): M.Role {"), "should have return type M.Role:\n{ops}");
}

#[test]
fn ts_fun_return_type_lone() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun findRole[u: one User]: lone Role { u }
    "#);
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("): M.Role | null {"), "should have return type M.Role | null:\n{ops}");
}

// ── Feature 2: Singleton support in TS ──────────────────────────────────────

#[test]
fn ts_singleton_const_object() {
    let files = generate_from("one sig Config {}");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("export const Config: Config = {};"),
        "should generate const object for singleton:\n{models}");
}

// ── Feature 4: Product → Map type in TS ─────────────────────────────────────

#[test]
fn ts_product_field_to_map() {
    let files = generate_from(r#"
        sig Config { settings: one Key -> Value }
        sig Key {}
        sig Value {}
    "#);
    let models = find_file(&files, "models.ts");
    assert!(models.contains("Map<Key, Value>"),
        "product field should map to Map:\n{models}");
}

// ── Stage 2: No invariants file, no @alloy comments, inlined expressions ────

#[test]
fn ts_no_alloy_comments_in_tests() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }\nassert AlwaysTrue { all u: User | u.role = u.role }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(!tests.contains("@alloy:"),
        "tests.ts should NOT contain @alloy comments:\n{tests}");
}

#[test]
fn ts_no_alloy_comments_in_operations() {
    let files = generate_from(
        "sig User {}\nsig Role {}\npred assign[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "operations.ts");
    assert!(!ops.contains("@alloy:"),
        "operations.ts should NOT contain @alloy comments:\n{ops}");
}

#[test]
fn ts_tests_no_invariants_import() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(!tests.contains("import * as inv from './invariants'"),
        "tests.ts should NOT import invariants:\n{tests}");
}

#[test]
fn ts_tests_inline_constraint_expression() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let tests = find_file(&files, "tests.ts");
    // Should inline the expression, not call inv.assertHasRole
    assert!(!tests.contains("inv.assertHasRole"),
        "tests should NOT call invariant function:\n{tests}");
    assert!(tests.contains(".every("),
        "tests should inline constraint expression:\n{tests}");
}

#[test]
fn ts_helpers_file_for_tc() {
    let files = generate_from(r#"
        sig Node { next: lone Node }
        fact Acyclic { no n: Node | n in n.^next }
    "#);
    assert!(files.iter().any(|f| f.path == "helpers.ts"),
        "should generate helpers.ts for TC functions");
    assert!(!files.iter().any(|f| f.path == "invariants.ts"),
        "should NOT generate invariants.ts");
    let helpers = find_file(&files, "helpers.ts");
    assert!(helpers.contains("tcNext"),
        "helpers.ts should contain TC function:\n{helpers}");
}

// ── Alloy 6: var field ──────────────────────────────────────────────────────

#[test]
fn ts_var_field_not_readonly() {
    let files = generate_from(r#"
        sig Account { var balance: one Int, name: one String }
    "#);
    let models = find_file(&files, "models.ts");
    assert!(!models.contains("readonly balance"),
        "var field should NOT have readonly:\n{models}");
    assert!(models.contains("readonly name"),
        "non-var field should have readonly:\n{models}");
}

#[test]
fn ts_temporal_prime_fact_generates_transition_test() {
    let files = generate_from(r#"
        sig Counter { var value: one Int }
        fact MonotonicallyIncreasing { always all c: Counter | c.value' = c.value }
    "#);
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("transition"),
        "should generate transition test for prime-containing fact:\n{tests}");
    assert!(tests.contains("next_counters"),
        "transition test should define post-state collection:\n{tests}");
    assert!(tests.contains("next_c"),
        "transition test should reference post-state element:\n{tests}");
    assert!(!tests.contains("TODO: apply transition"),
        "transition test should be materialized, not a TODO scaffold:\n{tests}");
}

// ── Binary temporal static test ──────────────────────────────────────────────

#[test]
fn ts_binary_temporal_static_test_is_comment_only() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("temporal WaitUntilDone"),
        "should generate temporal test:\n{tests}");
    assert!(tests.contains("binary temporal: requires trace-based verification"),
        "should document trace-based verification:\n{tests}");
}

// ── Disjoint constraint validation ──────────────────────────────────────────

#[test]
fn ts_validator_generates_disjoint_check() {
    let model = parser::parse(r#"
        sig Schedule { morning: set Task, evening: set Task }
        sig Task {}
        fact NoOverlap { no (Schedule.morning & Schedule.evening) }
    "#).unwrap();
    let ir_val = ir::lower(&model).unwrap();
    let validators = typescript::generate_validators(&ir_val);
    assert!(validators.contains("morning"), "validator should reference morning field:\n{validators}");
    assert!(validators.contains("evening"), "validator should reference evening field:\n{validators}");
    assert!(!validators.contains("// Disjoint"), "should generate actual code, not just a comment:\n{validators}");
    // Should check for overlap/intersection
    assert!(validators.contains("must not overlap"),
        "validator should check disjoint constraint:\n{validators}");
}

#[test]
fn ts_abstract_sig_fields_propagated_to_union_variants() {
    let files = generate_from(r#"
        sig Tick {}
        abstract sig Event { tick: one Tick }
        sig Started extends Event { source: one Tick }
        sig Stopped extends Event {}
    "#);
    let models = find_file(&files, "models.ts");
    // Parent field `tick` must appear in each variant interface
    assert!(models.contains("export interface Started {"),
        "Started should be a discriminated union variant:\n{models}");
    assert!(models.contains("tick: Tick"),
        "parent field `tick` should appear in variant:\n{models}");
    // Stopped has no own fields, but inherits `tick` — must be interface, not string literal
    assert!(models.contains("export interface Stopped {"),
        "Stopped should be interface (inherited field), not string literal:\n{models}");
}

// ── Derived fields (fun Sig.name → class method) ───────────────────────────

#[test]
fn ts_derived_field_generates_method() {
    let files = generate_from(r#"
        sig Account { deposits: set Int }
        fun Account.balance: one Int { #this.deposits }
    "#);
    let models = find_file(&files, "models.ts");
    // Inside models.ts the types are local, and `Int` is a marker sig that
    // lowers to `number` — `M.Int` named nothing (TS2503).
    assert!(models.contains("balance(): number"), "should generate method on class:\n{models}");
    assert!(
        models.contains("return this.self.deposits.size"),
        "Alloy `this` is the wrapper's `self` field, not the wrapper:\n{models}"
    );
}

// ── Native type alias mapping ───────────────────────────────────────────────

#[test]
fn ts_native_str_maps_to_string() {
    let files = generate_from(r#"
        sig Str {}
        sig User { name: one Str }
    "#);
    let models = find_file(&files, "models.ts");
    assert!(!models.contains("export interface Str"), "Str sig should not be emitted:\n{models}");
    assert!(models.contains("name: string;"), "Str field should map to string:\n{models}");
}

#[test]
fn ts_native_int_maps_to_number() {
    let files = generate_from(r#"
        sig Int {}
        sig Counter { value: one Int, items: set Int }
    "#);
    let models = find_file(&files, "models.ts");
    assert!(!models.contains("export interface Int"), "Int sig should not be emitted:\n{models}");
    assert!(models.contains("value: number;"), "Int field should map to number:\n{models}");
    assert!(models.contains("items: Set<number>;"), "set Int → Set<number>:\n{models}");
}

#[test]
fn ts_native_bool_maps_to_boolean() {
    let files = generate_from(r#"
        sig Bool {}
        sig Flag { active: one Bool, flags: seq Bool }
    "#);
    let models = find_file(&files, "models.ts");
    assert!(!models.contains("export interface Bool"), "Bool sig should not be emitted:\n{models}");
    assert!(models.contains("active: boolean;"), "Bool field should map to boolean:\n{models}");
    assert!(models.contains("flags: boolean[];"), "seq Bool → boolean[]:\n{models}");
}

// ── Field resolution through the binding (#95) ─────────────────────────────

/// Both sigs name the field `next`, with different multiplicities, and one fact
/// tests membership in each — so a single translated expression has to get both
/// right, and only the binding can tell them apart.
const TS_SHARED_FIELD_MODEL: &str = "\
sig Page {}
sig Node { next: lone Page }
sig Cursor { next: set Page }
fact BothHold { all n: Node | all c: Cursor | all p: Page | p in n.next and p in c.next }
";

/// A `set` field lowers to `Set<T>`, so membership is `.has`. Resolving `next`
/// by name found `Node`'s `lone` first and emitted `c.next === p`, which
/// compares a `Set` to an element and is therefore always false.
#[test]
fn ts_shared_field_name_uses_has_for_a_set_field() {
    let files = generate_from(TS_SHARED_FIELD_MODEL);
    let src = find_file(&files, "tests.ts");

    assert!(
        src.contains("c.next.has(p)"),
        "a `set` field lowers to `Set<T>`, so membership is `.has`. got:\n{src}"
    );
    assert!(
        !src.contains("c.next === p"),
        "a Set is never identical to one of its elements. got:\n{src}"
    );
}

/// The same membership over the `lone` field of the other sig stays an identity
/// comparison.
#[test]
fn ts_shared_field_name_keeps_identity_for_a_lone_field() {
    let files = generate_from(TS_SHARED_FIELD_MODEL);
    let src = find_file(&files, "tests.ts");

    assert!(
        src.contains("n.next === p"),
        "membership in a lone field is an identity test. got:\n{src}"
    );
}

// ── pred bodies are formulas, not stubs (#82) ──────────────────────────────

const PRED_MODEL: &str = "\
sig Item {}
sig Box { items: set Item, cap: one Int }
pred hasRoom[b: one Box] { #b.items < b.cap }
pred isEmpty[b: one Box] { no b.items }
";

/// An Alloy `pred` is a formula: it denotes true or false. Generating it as
/// `void` with a `throw` makes it unusable as a relation — the cross-tests that
/// are supposed to check `Op(..) implies Fact(..)` have nothing to call.
#[test]
fn ts_pred_returns_boolean_and_translates_its_body() {
    let files = generate_from(PRED_MODEL);
    let src = find_file(&files, "operations.ts");

    assert!(
        src.contains("export function hasRoom(b: M.Box): boolean {"),
        "a pred is a formula, so it returns boolean. got:\n{src}"
    );
    assert!(
        src.contains("b.items.size < b.cap"),
        "the body must be translated, not stubbed. got:\n{src}"
    );
    assert!(
        !src.contains("oxidtr: implement hasRoom"),
        "the stub must be gone. got:\n{src}"
    );
}

#[test]
fn ts_pred_with_a_no_formula_translates_to_emptiness() {
    let files = generate_from(PRED_MODEL);
    let src = find_file(&files, "operations.ts");

    assert!(
        src.contains("export function isEmpty(b: M.Box): boolean {"),
        "got:\n{src}"
    );
    assert!(
        src.contains("b.items.size === 0"),
        "`no b.items` over a Set is an emptiness test. got:\n{src}"
    );
}

// ── Temporal operators must not be erased on the assert path (#78) ─────────

const TEMPORAL_ASSERT_MODEL: &str = "\
sig Counter { var n: one Int }
assert Live { eventually (all c: Counter | c.n > 0) }
assert Ordered { (all c: Counter | c.n > 0) until (all c: Counter | c.n > 5) }
";

/// `eventually P` is not `P`. The assert path never consulted the temporal
/// classification, so it translated the operand and dropped the operator —
/// producing a test that asserts something the model never claimed.
#[test]
fn ts_assert_liveness_gets_a_trace_checker() {
    let files = generate_from(TEMPORAL_ASSERT_MODEL);
    let src = find_file(&files, "tests.ts");

    assert!(
        src.contains("check_liveness_live") || src.contains("check_liveness_Live"),
        "an `eventually` assert needs a trace checker, as the fact path emits. got:\n{src}"
    );
    assert!(
        src.contains("trace.some("),
        "liveness is satisfied in at least one state of the trace. got:\n{src}"
    );
}

/// `P until Q` is not `P && Q`.
#[test]
fn ts_assert_until_gets_a_trace_checker() {
    let files = generate_from(TEMPORAL_ASSERT_MODEL);
    let src = find_file(&files, "tests.ts");

    assert!(
        src.contains("check_until_ordered") || src.contains("check_until_Ordered"),
        "an `until` assert needs a trace checker. got:\n{src}"
    );
    assert!(
        src.contains("findIndex("),
        "until is a position search, not a conjunction. got:\n{src}"
    );
}
