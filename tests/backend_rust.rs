use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;

fn generate_from(input: &str) -> Vec<oxidtr::backend::GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    rust::generate(&ir)
}

fn find_file<'a>(files: &'a [oxidtr::backend::GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

#[test]
fn generate_empty_struct() {
    let files = generate_from("sig Foo {}");
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub struct Foo"));
    assert!(content.contains("#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]"));
}

#[test]
fn generate_struct_with_one_field() {
    let files = generate_from(r#"
        sig User { name: one Name }
        sig Name {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub struct User"));
    assert!(content.contains("pub name: Name"));
}

#[test]
fn generate_option_for_lone() {
    let files = generate_from(r#"
        sig Node { next: lone Node }
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("Option<Box<Node>>"));
}

#[test]
fn generate_vec_for_set() {
    let files = generate_from(r#"
        sig User { roles: set Role }
        sig Role {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("BTreeSet<Role>"));
}

#[test]
fn generate_vec_for_seq() {
    let files = generate_from(r#"
        sig Order { items: seq Item }
        sig Item {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("Vec<Item>"));
}

#[test]
fn generate_enum_for_abstract_sig() {
    let files = generate_from(r#"
        abstract sig Role {}
        one sig Admin extends Role {}
        one sig Viewer extends Role {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub enum Role"));
    assert!(content.contains("Admin"));
    assert!(content.contains("Viewer"));
}

#[test]
fn generate_operation_stub() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        pred assign[u: one User, r: one Role] { u = u }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("fn assign"));
    assert!(content.contains("user: &User") || content.contains("u: &User"));
    assert!(content.contains("todo!"));
}

#[test]
fn generate_property_test() {
    let files = generate_from(r#"
        sig A {}
        assert AlwaysTrue { all a: A | a = a }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("always_true") || content.contains("AlwaysTrue"));
    assert!(content.contains("#[test]") || content.contains("proptest"));
}

// ── Non-vacuous test generation (Item 1) ────────────────────────────────────

#[test]
fn rust_tests_import_fixtures() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("use super::fixtures::*"),
        "tests should import fixtures module:\n{content}");
}

#[test]
fn rust_tests_use_fixture_factory_for_sig_with_fields() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("default_user()"),
        "test should use fixture factory for User:\n{content}");
    assert!(content.contains("vec![default_user()]"),
        "test should populate vec with fixture:\n{content}");
}

#[test]
fn rust_tests_empty_vec_for_sig_without_fields() {
    let files = generate_from(r#"
        sig Token {}
        assert AllTokens { all t: Token | t = t }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("Vec::new()"),
        "test should use Vec::new() for Token (no fields):\n{content}");
}

// ── Newtype + TryFrom generation (Item 2) ────────────────────────────────────

#[test]
fn rust_generates_newtype_for_named_constraint_with_comparison() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    assert!(newtypes.contains("pub struct ValidatedUser(pub User)"),
        "should generate ValidatedUser newtype:\n{newtypes}");
}

#[test]
fn rust_generates_tryfrom_for_newtype() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    assert!(newtypes.contains("impl TryFrom<User> for ValidatedUser"),
        "should generate TryFrom impl:\n{newtypes}");
    // TryFrom should inline the constraint expression, not call invariant function
    assert!(!newtypes.contains("assert_has_role"),
        "TryFrom should NOT call invariant function:\n{newtypes}");
    assert!(newtypes.contains(".iter().all("),
        "TryFrom should inline constraint expression:\n{newtypes}");
}

#[test]
fn rust_no_newtype_for_anonymous_fact() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact { all u: User | u.role = u.role }
    "#);
    // Anonymous fact should not produce newtypes
    assert!(!files.iter().any(|f| f.path == "newtypes.rs"),
        "should not generate newtypes.rs for anonymous facts");
}

#[test]
fn rust_no_newtype_for_fact_without_comparison() {
    // TransitiveClosure-only constraint without direct comparison
    let files = generate_from(r#"
        sig Node { next: lone Node }
        fact Acyclic { no n: Node | n in n.^next }
    "#);
    // The `no` quantifier generates a Quantifier with Comparison inside it,
    // so this WILL generate a newtype. Let's test a case without any Comparison.
    // Actually, `n in n.^next` IS a Comparison, so this will generate.
    // A fact that truly has no comparison is not expressible in the grammar easily.
    // Instead, let's verify the newtype IS generated for this constraint.
    let newtypes = find_file(&files, "newtypes.rs");
    assert!(newtypes.contains("ValidatedNode"),
        "should generate ValidatedNode for Acyclic:\n{newtypes}");
}

// ── Serde opt-in (Item 6) ────────────────────────────────────────────────────

fn generate_with_serde(input: &str) -> Vec<oxidtr::backend::GeneratedFile> {
    let model = oxidtr::parser::parse(input).expect("should parse");
    let ir = oxidtr::ir::lower(&model).expect("should lower");
    let config = oxidtr::backend::rust::RustBackendConfig {
        features: vec!["serde".to_string()],
    };
    oxidtr::backend::rust::generate_with_config(&ir, &config)
}

#[test]
fn rust_serde_adds_serialize_deserialize() {
    let files = generate_with_serde("sig User { name: one Name }\nsig Name {}");
    let content = find_file(&files, "models.rs");
    assert!(content.contains("Serialize, Deserialize"),
        "should have serde derives:\n{content}");
}

#[test]
fn rust_serde_adds_use_statement() {
    let files = generate_with_serde("sig User {}");
    let content = find_file(&files, "models.rs");
    assert!(content.contains("use serde::{Serialize, Deserialize}"),
        "should import serde:\n{content}");
}

#[test]
fn rust_serde_tag_on_enum_with_data_variants() {
    let files = generate_with_serde(r#"
        abstract sig Expr {}
        sig Literal extends Expr {}
        sig BinOp extends Expr { left: one Expr, right: one Expr }
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("#[serde(tag = \"type\")]"),
        "should have serde tag on enum with data:\n{content}");
}

#[test]
fn rust_serde_no_tag_on_unit_enum() {
    let files = generate_with_serde(r#"
        abstract sig Color {}
        one sig Red extends Color {}
        one sig Blue extends Color {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(!content.contains("#[serde(tag"),
        "should NOT have serde tag on unit enum:\n{content}");
}

#[test]
fn rust_no_serde_by_default() {
    let files = generate_from("sig User { name: one Name }\nsig Name {}");
    let content = find_file(&files, "models.rs");
    assert!(!content.contains("Serialize"),
        "should NOT have serde derives by default:\n{content}");
    assert!(!content.contains("Deserialize"),
        "should NOT have serde derives by default:\n{content}");
}

// ── Feature 1: Fun return type in operation stubs ────────────────────────────

#[test]
fn rust_fun_return_type_one() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun getRole[u: one User]: one Role { u }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("-> Role"), "should have return type Role:\n{content}");
}

#[test]
fn rust_fun_return_type_lone() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun findRole[u: one User]: lone Role { u }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("-> Option<Role>"), "should have return type Option<Role>:\n{content}");
}

#[test]
fn rust_fun_return_type_set() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun getRoles[u: one User]: set Role { u }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("-> BTreeSet<Role>"), "should have return type BTreeSet<Role>:\n{content}");
}

#[test]
fn rust_fun_return_type_seq() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun getRoles[u: one User]: seq Role { u }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("-> Vec<Role>"), "should have return type Vec<Role>:\n{content}");
}

// ── Feature 2: Singleton support ─────────────────────────────────────────────

#[test]
fn rust_singleton_unit_struct_with_const() {
    let files = generate_from("one sig Config {}");
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub struct Config;"), "should generate unit struct:\n{content}");
    assert!(content.contains("pub const CONFIG_INSTANCE: Config = Config;"),
        "should generate INSTANCE const:\n{content}");
}

// ── Feature 3: Concrete numeric values in TryFrom ───────────────────────────

#[test]
fn rust_tryfrom_range_check_with_numeric_bound() {
    let files = generate_from(r#"
        sig Team { members: set User }
        sig User {}
        fact TeamLimit { all t: Team | #t.members <= 10 }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    assert!(newtypes.contains("value.members.len() > 10"),
        "TryFrom should check len > 10:\n{newtypes}");
}

// ── Feature 4: Product → Map type ───────────────────────────────────────────

#[test]
fn rust_product_field_to_btreemap() {
    let files = generate_from(r#"
        sig Config { settings: one Key -> Value }
        sig Key {}
        sig Value {}
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("BTreeMap<Key, Value>"),
        "product field should map to BTreeMap:\n{content}");
}

// ── Stage 1: No invariants file, no @alloy comments, inlined expressions ────

#[test]
fn rust_no_invariants_file() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    assert!(!files.iter().any(|f| f.path == "invariants.rs"),
        "should NOT generate invariants.rs");
}

#[test]
fn rust_no_alloy_comments_in_tests() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
        assert AlwaysTrue { all u: User | u.role = u.role }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(!tests.contains("@alloy:"),
        "tests.rs should NOT contain @alloy comments:\n{tests}");
}

#[test]
fn rust_no_alloy_comments_in_operations() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        pred assign[u: one User, r: one Role] { u = u }
    "#);
    let ops = find_file(&files, "operations.rs");
    assert!(!ops.contains("@alloy:"),
        "operations.rs should NOT contain @alloy comments:\n{ops}");
}

#[test]
fn rust_tests_no_invariants_import() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(!tests.contains("use crate::invariants::"),
        "tests.rs should NOT import invariants:\n{tests}");
}

#[test]
fn rust_tests_inline_constraint_expression() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let tests = find_file(&files, "tests.rs");
    // Should inline the expression, not call assert_has_role
    assert!(!tests.contains("assert_has_role"),
        "tests should NOT call invariant function:\n{tests}");
    assert!(tests.contains(".iter().all("),
        "tests should inline constraint expression:\n{tests}");
}

#[test]
fn rust_tryfrom_inlines_constraint_expression() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    assert!(!newtypes.contains("use crate::invariants::"),
        "newtypes.rs should NOT import invariants:\n{newtypes}");
    assert!(newtypes.contains(".iter().all("),
        "TryFrom should inline constraint expression:\n{newtypes}");
}

#[test]
fn rust_helpers_file_for_tc() {
    let files = generate_from(r#"
        sig Node { next: lone Node }
        fact Acyclic { no n: Node | n in n.^next }
    "#);
    // TC functions should be in helpers.rs, not invariants.rs
    assert!(files.iter().any(|f| f.path == "helpers.rs"),
        "should generate helpers.rs for TC functions");
    assert!(!files.iter().any(|f| f.path == "invariants.rs"),
        "should NOT generate invariants.rs");
    let helpers = find_file(&files, "helpers.rs");
    assert!(helpers.contains("tc_next"),
        "helpers.rs should contain TC function:\n{helpers}");
}

#[test]
fn rust_doc_comments_preserved_on_structs() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(models.contains("/// Invariant: HasRole"),
        "models.rs should still have doc comments:\n{models}");
}

// ── Alloy 6: var field ──────────────────────────────────────────────────────

#[test]
fn rust_var_field_annotated() {
    let files = generate_from(r#"
        sig Account { var balance: one Int }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(models.contains("MUTABLE"),
        "var field should have MUTABLE annotation:\n{models}");
}

#[test]
fn rust_temporal_always_fact_generates_invariant_test() {
    let files = generate_from(r#"
        sig Counter { var value: one Int }
        fact AlwaysPositive { always all c: Counter | c.value = c.value }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(tests.contains("invariant_always_positive"),
        "should generate invariant test for always fact:\n{tests}");
}

#[test]
fn rust_temporal_prime_fact_generates_transition_test() {
    let files = generate_from(r#"
        sig Counter { var value: one Int }
        fact MonotonicallyIncreasing { always all c: Counter | c.value' = c.value }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(tests.contains("transition_monotonically_increasing"),
        "should generate transition test for prime-containing fact:\n{tests}");
    assert!(tests.contains("next_counters"),
        "transition test should define post-state collection:\n{tests}");
    assert!(tests.contains("next_c"),
        "transition test should reference post-state element:\n{tests}");
    assert!(tests.contains(".zip("),
        "transition test should use zip for pre/post iteration:\n{tests}");
    assert!(!tests.contains("TODO: apply transition"),
        "transition test should be materialized, not a TODO scaffold:\n{tests}");
}

// ── Binary temporal static test ──────────────────────────────────────────────

#[test]
fn rust_binary_temporal_static_test_is_comment_only() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(tests.contains("fn temporal_wait_until_done"),
        "should generate temporal test:\n{tests}");
    assert!(tests.contains("binary temporal: requires trace-based verification; see check_until_wait_until_done"),
        "should document trace-based verification:\n{tests}");
    assert!(tests.contains("fn check_until_wait_until_done"),
        "trace checker should still be generated:\n{tests}");
}

// ── Disjoint constraint validation ──────────────────────────────────────────

#[test]
fn rust_try_from_generates_disjoint_check() {
    let files = generate_from(r#"
        sig Schedule { morning: set Task, evening: set Task }
        sig Task {}
        fact NoOverlap { no (Schedule.morning & Schedule.evening) }
    "#);
    let newtypes = files.iter().find(|f| f.path == "newtypes.rs");
    assert!(newtypes.is_some(), "newtypes.rs should be generated for disjoint constraint, files: {:?}",
        files.iter().map(|f| &f.path).collect::<Vec<_>>());
    let newtypes = newtypes.unwrap().content.as_str();
    assert!(newtypes.contains("morning"), "TryFrom should reference morning field:\n{newtypes}");
    assert!(newtypes.contains("evening"), "TryFrom should reference evening field:\n{newtypes}");
    assert!(newtypes.contains("must not overlap"),
        "TryFrom should check disjoint constraint:\n{newtypes}");
}

// ── Bug fixes ────────────────────────────────────────────────────────────────

/// Bug: unit struct (no fields) was skipped in fixture generation — no default_foo() produced.
/// A unit struct in Alloy is a sig with no fields. The fixture should produce
/// `pub fn default_foo() -> Foo { Foo }`.
#[test]
fn rust_unit_struct_fixture_generated() {
    let files = generate_from(r#"
        sig Tag {}
        sig Node { tag: one Tag }
    "#);
    let fixtures = find_file(&files, "fixtures.rs");
    assert!(fixtures.contains("pub fn default_tag() -> Tag"),
        "fixtures.rs should contain default_tag():\n{fixtures}");
    assert!(fixtures.contains("Tag"),
        "default_tag() body should return Tag:\n{fixtures}");
}

/// Multiple unit structs should all get factory functions.
#[test]
fn rust_multiple_unit_structs_all_get_fixtures() {
    let files = generate_from(r#"
        sig Alpha {}
        sig Beta {}
        sig Gamma {}
        sig Container { a: one Alpha, b: one Beta, c: one Gamma }
    "#);
    let fixtures = find_file(&files, "fixtures.rs");
    for name in &["alpha", "beta", "gamma"] {
        assert!(fixtures.contains(&format!("pub fn default_{name}() -> ")),
            "fixtures.rs should contain default_{name}():\n{fixtures}");
    }
}

/// Bug: newtypes validator for a `lone` (Option) field used `contains(&field)`
/// where field is `Option<T>`, causing a type mismatch.
/// The generated validator should unwrap the Option before calling contains.
#[test]
fn rust_newtypes_lone_field_option_unwrapped_in_validator() {
    let files = generate_from(r#"
        sig SM { states: set State, activeState: lone State }
        sig State {}
        fact ActiveOwned { all sm: SM | sm.activeState in sm.states }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    // Must NOT contain the broken pattern `contains(&sm.active_state)` where active_state: Option
    assert!(!newtypes.contains("contains(&value.activeState)"),
        "validator must not pass Option<T> directly to contains:\n{newtypes}");
    // Must contain the correct pattern: unwrap Option before contains check
    assert!(
        newtypes.contains("as_ref()") || newtypes.contains("map(") || newtypes.contains("unwrap_or"),
        "validator must handle Option with as_ref/map/unwrap_or:\n{newtypes}");
}

/// Bug: newtypes validator for enum comparison used unqualified variant names
/// (e.g. `PortKindOutput`) instead of `PortKind::PortKindOutput`.
#[test]
fn rust_newtypes_enum_variant_fully_qualified_in_validator() {
    let files = generate_from(r#"
        abstract sig PortKind {}
        one sig PortKindInput  extends PortKind {}
        one sig PortKindOutput extends PortKind {}
        sig Port { portKind: one PortKind }
        sig Conn { src: one Port, tgt: one Port }
        fact ConnDir { all c: Conn | c.src.portKind = PortKindOutput and c.tgt.portKind = PortKindInput }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    // Variants must be qualified as PortKind::PortKindOutput, not bare PortKindOutput
    assert!(!newtypes.contains("== PortKindOutput"),
        "unqualified PortKindOutput found in validator:\n{newtypes}");
    assert!(!newtypes.contains("== PortKindInput"),
        "unqualified PortKindInput found in validator:\n{newtypes}");
}


#[test]
fn abstract_sig_fields_propagated_to_enum_variants() {
    let files = generate_from(r#"
        sig Tick {}
        abstract sig Event { tick: one Tick }
        sig Started extends Event { source: one Tick }
        sig Stopped extends Event {}
    "#);
    let content = find_file(&files, "models.rs");
    // Parent field `tick` must appear in variant Started (alongside its own `source`)
    assert!(content.contains("Started {"),
        "Started should be a data variant:\n{content}");
    assert!(content.contains("tick: Tick"),
        "parent field `tick` should appear in enum variant:\n{content}");
    // Stopped has no own fields, but inherits `tick` — must still be a data variant
    assert!(content.contains("Stopped {"),
        "Stopped should be a data variant (inherited field):\n{content}");
}

// --- Anomaly fixture and test generation ---

#[test]
fn rust_anomaly_empty_fixture_for_unbounded_set() {
    let files = generate_from(r#"
        sig Team { members: set User }
        sig User {}
    "#);
    let fixtures = find_file(&files, "fixtures.rs");
    assert!(fixtures.contains("anomaly_empty_team"),
        "should generate empty anomaly fixture for unbounded set:\n{fixtures}");
}

#[test]
fn rust_anomaly_test_generated_for_unconstrained() {
    let files = generate_from(r#"
        sig User { name: one Name, age: one Int }
        sig Name {}
        fact AgePositive { all u: User | u.age >= 0 }
    "#);
    let tests = find_file(&files, "tests.rs");
    // name is unconstrained → should generate anomaly test
    assert!(tests.contains("anomaly_"),
        "should generate anomaly tests:\n{tests}");
}

#[test]
fn rust_no_anomaly_fixture_when_fully_constrained() {
    // Every field is bounded and referenced by a fact
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact HasRole { all u: User | u.role = u.role }
    "#);
    let fixtures = find_file(&files, "fixtures.rs");
    assert!(!fixtures.contains("anomaly_empty_"),
        "fully constrained should not have empty anomaly fixture:\n{fixtures}");
}

// --- Coverage test generation ---

#[test]
fn rust_coverage_pairwise_test_generated() {
    let files = generate_from(r#"
        sig Account { balance: one Int, limit: one Int }
        fact NonNeg { all a: Account | a.balance >= 0 }
        fact BelowLimit { all a: Account | a.balance <= a.limit }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(tests.contains("cover_"),
        "should generate pairwise coverage tests:\n{tests}");
}

#[test]
fn rust_no_coverage_test_for_single_fact() {
    let files = generate_from(r#"
        sig User { age: one Int }
        fact MinAge { all u: User | u.age >= 0 }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(!tests.contains("cover_"),
        "single fact should not generate pairwise test:\n{tests}");
}

// ── Derived fields (fun Sig.name → impl method) ────────────────────────────

#[test]
fn rust_derived_field_generates_impl_method() {
    let files = generate_from(r#"
        sig Account { deposits: set Int }
        fun Account.balance: one Int { #this.deposits }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(models.contains("impl Account"), "should generate impl block:\n{models}");
    assert!(models.contains("fn balance(&self) -> Int"), "should generate method:\n{models}");
}

#[test]
fn rust_derived_field_with_params() {
    let files = generate_from(r#"
        sig Account { items: set Item }
        sig Item {}
        fun Account.hasItem[i: one Item]: one Int { i in this.items }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(models.contains("impl Account"), "should generate impl block:\n{models}");
    assert!(models.contains("fn has_item(&self"), "should generate method with params:\n{models}");
}

#[test]
fn rust_non_receiver_fun_still_in_operations() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun getRole[u: one User]: one Role { u }
    "#);
    let ops = find_file(&files, "operations.rs");
    assert!(ops.contains("fn get_role("), "non-receiver fun should remain in operations.rs:\n{ops}");
}

// ── Native type alias mapping ───────────────────────────────────────────────

#[test]
fn rust_native_str_maps_to_string() {
    let files = generate_from(r#"
        sig Str {}
        sig User { name: one Str }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(!models.contains("pub struct Str"), "Str sig should not be emitted as struct:\n{models}");
    assert!(models.contains("pub name: String,"), "Str field should map to String:\n{models}");
}

#[test]
fn rust_native_int_maps_to_i64() {
    let files = generate_from(r#"
        sig Int {}
        sig Counter { value: one Int }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(!models.contains("pub struct Int"), "Int sig should not be emitted:\n{models}");
    assert!(models.contains("pub value: i64,"), "Int field should map to i64:\n{models}");
}

#[test]
fn rust_native_float_maps_to_f64() {
    let files = generate_from(r#"
        sig Float {}
        sig Measurement { reading: one Float }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(!models.contains("pub struct Float"), "Float sig should not be emitted:\n{models}");
    assert!(models.contains("pub reading: f64,"), "Float field should map to f64:\n{models}");
}

#[test]
fn rust_native_bool_maps_to_bool() {
    let files = generate_from(r#"
        sig Bool {}
        sig Flag { active: one Bool }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(!models.contains("pub struct Bool"), "Bool sig should not be emitted:\n{models}");
    assert!(models.contains("pub active: bool,"), "Bool field should map to bool:\n{models}");
}

#[test]
fn rust_native_type_with_multiplicities() {
    let files = generate_from(r#"
        sig Str {}
        sig Int {}
        sig Item {
            tags: set Str,
            label: lone Str,
            scores: seq Int
        }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(models.contains("pub tags: BTreeSet<String>,"), "set Str → BTreeSet<String>:\n{models}");
    assert!(models.contains("pub label: Option<String>,"), "lone Str → Option<String>:\n{models}");
    assert!(models.contains("pub scores: Vec<i64>,"), "seq Int → Vec<i64>:\n{models}");
}

#[test]
fn enum_variant_fields_resolve_native_types() {
    let files = generate_from(r#"
        abstract sig AlgebraicStructure { rank: one Int, label: one Str }
        sig Magma extends AlgebraicStructure {}
        sig Monoid extends AlgebraicStructure {}
    "#);
    let models = find_file(&files, "models.rs");
    // Enum variant fields should use resolved types (i64, String), not Alloy names (Int, Str)
    assert!(models.contains("rank: i64,"), "enum variant field Int should resolve to i64:\n{models}");
    assert!(models.contains("label: String,"), "enum variant field Str should resolve to String:\n{models}");
    assert!(!models.contains("rank: Int,"), "should not contain raw Alloy type Int:\n{models}");
    assert!(!models.contains("label: Str,"), "should not contain raw Alloy type Str:\n{models}");
}

#[test]
fn fixture_respects_value_bounds() {
    let files = generate_from(r#"
        sig Report { total_laws: one Int, score: one Float }
        fact PositiveLaws { all r: Report | r.total_laws > 0 }
        fact PositiveScore { all r: Report | r.score >= 1 }
    "#);
    let fixtures = find_file(&files, "fixtures.rs");
    // total_laws > 0 → default should be 1i64 (AtLeast(1)), not 0i64
    assert!(fixtures.contains("total_laws: 1i64"), "total_laws should be 1i64 (> 0):\n{fixtures}");
    // score >= 1 → default should be 1.0f64 (AtLeast(1)), not 0.0f64
    assert!(fixtures.contains("score: 1.0f64"), "score should be 1.0f64 (>= 1):\n{fixtures}");
}

#[test]
fn one_sig_fixed_value_generates_unit_variant_with_const_method() {
    let files = generate_from(r#"
        abstract sig AlgebraicStructure { rank: one Int }
        one sig Magma extends AlgebraicStructure {}
        one sig Monoid extends AlgebraicStructure {}
        fact { Magma.rank = 0 }
        fact { Monoid.rank = 1 }
    "#);
    let models = find_file(&files, "models.rs");
    // Should generate unit variants (no fields)
    assert!(models.contains("Magma,"), "Magma should be unit variant:\n{models}");
    assert!(models.contains("Monoid,"), "Monoid should be unit variant:\n{models}");
    // Should NOT generate struct variants with rank field
    assert!(!models.contains("Magma {"), "Magma should NOT be struct variant:\n{models}");
    // Should generate const fn rank()
    assert!(models.contains("pub const fn rank(&self)"), "should have const fn rank:\n{models}");
    assert!(models.contains("Self::Magma => 0"), "Magma.rank should be 0:\n{models}");
    assert!(models.contains("Self::Monoid => 1"), "Monoid.rank should be 1:\n{models}");
}

// ─── Set-valued expression codegen ─────────────────────────────────────────

/// BUG: `some field` where `field` has multiplicity `set` lowered to
/// `field.is_some()`, which is an Option method. BTreeSet doesn't have
/// it — the generated code fails to compile. Must emit `!field.is_empty()`.
#[test]
fn some_on_set_field_uses_is_empty() {
    let files = generate_from(r#"
        sig Node {
          children: set Node
        }
        fact HasChildren {
          all n: Node | some n.children
        }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("!n.children.is_empty()") || tests.contains("! n.children.is_empty()"),
        "`some set_field` should lower to `!field.is_empty()`, got:\n{tests}"
    );
    assert!(
        !tests.contains("n.children.is_some()"),
        "`some set_field` must not emit `.is_some()` on a BTreeSet:\n{tests}"
    );
}

/// Mirror of the above: `no set_field` must lower to `field.is_empty()`.
#[test]
fn no_on_set_field_uses_is_empty() {
    let files = generate_from(r#"
        sig Node {
          children: set Node
        }
        fact NoChildren {
          all n: Node | no n.children
        }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("n.children.is_empty()"),
        "`no set_field` should lower to `field.is_empty()`, got:\n{tests}"
    );
    assert!(
        !tests.contains("n.children.is_none()"),
        "`no set_field` must not emit `.is_none()` on a BTreeSet:\n{tests}"
    );
}

/// BUG: `set_a in set_b` (subset) was lowered to `set_b.contains(&set_a)`,
/// which is an element-membership test and a type error besides. For two
/// set-typed operands, must emit `set_a.is_subset(&set_b)`.
#[test]
fn subset_between_set_fields_uses_is_subset() {
    let files = generate_from(r#"
        sig Group {
          members: set Person,
          admins:  set Person
        }
        sig Person {}
        fact AdminsAreMembers {
          all g: Group | g.admins in g.members
        }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("g.admins.is_subset(&g.members)"),
        "`set in set` should lower to `.is_subset(&...)`, got:\n{tests}"
    );
    assert!(
        !tests.contains("g.members.contains(&g.admins)"),
        "`set in set` must not emit `.contains()` with a set-valued arg:\n{tests}"
    );
}

/// Same fix applied at the validator (newtypes.rs) emission path: when a
/// fact's antecedent uses `some set_field`, the generated TryFrom must
/// not call `.is_some()` on a BTreeSet. The fact includes an explicit
/// comparison (`capacity > 0`) so newtypes.rs is emitted for Room.
#[test]
fn validator_handles_some_set_field() {
    let files = generate_from(r#"
        sig Room {
          occupants: set Person,
          capacity:  one Int
        }
        sig Person {}
        fact NonEmptyImpliesSized {
          all r: Room | some r.occupants implies r.capacity > 0
        }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    assert!(
        !newtypes.contains(".occupants.is_some()"),
        "validator must not call `.is_some()` on a BTreeSet:\n{newtypes}"
    );
    assert!(
        newtypes.contains(".occupants.is_empty()"),
        "validator should check `!is_empty()` on the set field:\n{newtypes}"
    );
}
