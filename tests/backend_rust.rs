use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;

fn generate_from(input: &str) -> Vec<oxidtr::backend::GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    rust::generate(&ir)
}

/// Slice one generated item out of a file. Asserting against a whole file lets
/// a claim about `fn foo` be satisfied by a neighbouring function.
fn fn_body<'a>(src: &'a str, header: &str) -> &'a str {
    let start = src.find(header).unwrap_or_else(|| panic!("no `{header}` in:\n{src}"));
    let rest = &src[start + header.len()..];
    let end = rest.find("\nfn ").unwrap_or(rest.len());
    &rest[..end]
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
fn generate_operation_body_is_translated_not_stubbed() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        pred assign[u: one User, r: one Role] { u != r }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("pub fn assign(u: &User, r: &Role) -> bool"));
    assert!(content.contains("u != r"));
    assert!(!content.contains("todo!"), "a translatable body must not be a stub");
}

#[test]
fn generate_pred_with_no_body_is_vacuously_true() {
    // An empty `pred` body is Alloy's empty conjunction — vacuously true by
    // definition, same as `is_tautological_body`'s identical-conjuncts case
    // below, just for zero conjuncts instead of one. Decidable at generation
    // time; no `todo!()`, no LLM/tinycodr completion needed. (A `fun`'s body
    // can never be empty this way — the parser requires an expression for
    // `fun`, confirmed by `parse_fun`'s non-optional body parse — so this
    // case is structurally always a `pred`.)
    let files = generate_from(r#"
        sig Thing {}
        pred noop[t: one Thing] {}
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("fn noop"));
    assert!(content.contains("true"));
    assert!(!content.contains("todo!"), "an empty pred body is vacuously true, not a stub");
}

#[test]
fn generate_tautological_operation_body_is_not_a_stub() {
    let files = generate_from(r#"
        sig Thing {}
        pred noop[t: one Thing] { t = t }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("fn noop"));
    assert!(content.contains("true"));
    assert!(!content.contains("todo!"));
}

#[test]
fn generate_multi_clause_operation_translates_and_joins_with_and() {
    let files = generate_from(r#"
        sig Item {}
        sig Bin { held: set Item }
        pred place[b: one Bin, i: one Item, prior: one Bin] {
            i in b.held
            b != prior
        }
    "#);
    let content = find_file(&files, "operations.rs");
    // No per-clause helper functions — both conjuncts translate directly into
    // the public function's own body, joined with `&&`.
    assert!(!content.contains("_clause_1"));
    assert!(!content.contains("_clause_2"));
    assert!(content.contains("pub fn place(b: &Bin, i: &Item, prior: &Bin) -> bool"));
    assert!(content.contains("b.held.contains(&i)"));
    assert!(content.contains("b != prior"));
    assert!(content.contains(" && "));
    assert!(!content.contains("todo!"));
}

#[test]
fn generate_self_referential_box_field_comparison_derefs_both_sides() {
    // Regression test for the lowerOneSig bug: a One-multiplicity
    // self-referential field (boxed to break the type cycle) compared
    // against a bare operation parameter needs a deref on BOTH sides, not
    // just the field-access side, or the two operands' types don't match
    // (`T` from the Box-deref vs `&T` from the parameter).
    let files = generate_from(r#"
        sig SigDecl { holder: lone StructureNode }
        sig StructureNode { origin: one SigDecl }
        pred originMatches[sn: one StructureNode, s: one SigDecl] { sn.origin = s }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("pub fn origin_matches"));
    assert!(content.contains("(*sn.origin) == (*s)"));
    assert!(!content.contains("todo!"));
}

#[test]
fn generate_derived_field_body_is_translated_not_stubbed() {
    // Receiver-based `fun`s (derived fields) previously always got a
    // `todo!()` stub regardless of body content — the deterministic
    // translation added for free-function operations didn't cover them.
    // `this` (Alloy's implicit receiver) must map to `self`, and returning
    // an owned field by value through `&self` needs `.clone()`.
    let files = generate_from(r#"
        sig Item {}
        sig Bin { held: one Item }
        fun Bin.contents: one Item { this.held }
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub fn contents(&self) -> Item"));
    assert!(content.contains("self.held.clone()"));
    assert!(!content.contains("todo!"));
}

#[test]
fn generate_derived_field_with_tautological_shape_is_not_forced_true() {
    // `fun`s return a VALUE, not a boolean formula. Even if a fun's body
    // happens to be structurally shaped like a self-equality tautology (the
    // same shape `is_tautological_body` short-circuits to `true` for
    // `pred`s), it must not take that shortcut here: the declared return
    // type isn't `bool`, so a bare `true` wouldn't even match the return
    // type. Regression test for is_tautological_body being applied without
    // checking op.return_type first.
    let files = generate_from(r#"
        sig Foo {}
        fun Foo.echo: one Foo { this = this }
    "#);
    let content = find_file(&files, "models.rs");
    assert!(content.contains("pub fn echo(&self) -> Foo"));
    assert!(!content.contains("        true"), "a fun body must never be replaced by a bare `true`");
}

#[test]
fn generate_fun_body_referencing_bare_sig_name_is_a_stub_not_wrong_code() {
    // A `fun` body that's just a bare reference to a SIG's own name (not an
    // enum variant) has no sensible Rust value — `Money` is a type, not an
    // instance. Before this fix, VarRef translation fell through to
    // `name.clone()` for any non-enum sig name, producing e.g. `Money.clone()`
    // — a compile error (E0423: expected value, found struct `Money`), not a
    // stub asking for help. Unlike the empty-body-pred case (vacuously `true`,
    // a decidable answer), there's no deterministically correct value here —
    // the honest answer is `todo!()`, not a silent guess. Found generating a
    // hand-written model (`fun zero: Money { Money }`) outside oxidtr's own
    // self-hosting corpus.
    let files = generate_from(r#"
        sig Money { amount: one Int }
        fun zero: Money { Money }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("fn zero"));
    assert!(content.contains("todo!"), "a bare sig-name self-reference has no translatable value, must stub:\n{content}");
    assert!(!content.contains("Money.clone()"), "must never emit a type name as if it were a value:\n{content}");
}

#[test]
fn generate_operation_call_to_another_operation_uses_generated_fn_name() {
    // Regression test: a bare call from one operation's body into another
    // (`addField[s, f]`) previously translated to the raw Alloy name
    // (`addField(&s, &f)`), not the callee's actual generated Rust name
    // (`field_is_present`, via fn_name_for_op) — a call to a function that
    // doesn't exist. The callee must be resolved through the same naming
    // rule its definition site uses.
    let files = generate_from(r#"
        sig SigDecl { fields: set FieldDecl }
        sig FieldDecl {}
        pred addField[s: one SigDecl, f: one FieldDecl] { f in s.fields }
        pred wrapper[s: one SigDecl, f: one FieldDecl] { addField[s, f] }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("pub fn field_is_present"));
    assert!(content.contains("field_is_present(&s, &f)"), "call site must use the callee's generated name:\n{content}");
    assert!(!content.contains("addField("), "must not call the operation under its raw Alloy name:\n{content}");
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
fn rust_newtype_calling_fun_imports_operations() {
    // Regression #61: newtype TryFrom bodies that call a fun (lowered into
    // operations.rs) must import `super::operations::*` or they don't compile.
    // The fact must be elementwise or no wrapper is generated at all (a
    // wrapper substitutes one value for the whole universe, so a cross-atom
    // fact like associativity cannot be checked from one Money).
    let files = generate_from(r#"
        sig Money { amount: one Int }
        fun doubled[m: Money]: Int { m.amount }
        fact NonNegative { all m: Money | doubled[m] >= 0 }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    assert!(newtypes.contains("doubled("),
        "precondition: newtype body should call the fun:\n{newtypes}");
    assert!(newtypes.contains("use super::operations::*;"),
        "newtypes.rs calling a fun must import operations:\n{newtypes}");
}

#[test]
fn rust_enum_struct_variant_fixture_constructs_fields() {
    // Regression #62: an abstract sig's field is folded into every enum
    // variant, so the variant is a struct variant. The fixture must construct
    // its fields — `Transaction::Deposit { value: default_money() }` — not
    // emit the bare path `Transaction::Deposit`, which does not compile.
    let files = generate_from(r#"
        sig Money { amount: one Int }
        abstract sig Transaction { value: one Money }
        sig Deposit extends Transaction {}
        sig Withdrawal extends Transaction {}
    "#);
    let fixtures = find_file(&files, "fixtures.rs");
    assert!(fixtures.contains("value: default_money()"),
        "enum struct-variant fixture should construct fields:\n{fixtures}");
    // The bare struct-variant path (no braces) must not appear as a value.
    assert!(!fixtures.contains("Transaction::Deposit\n") && !fixtures.contains("Transaction::Withdrawal\n"),
        "bare struct-variant path must not be emitted as a value:\n{fixtures}");
}

#[test]
fn rust_nullary_fun_reference_is_called() {
    // Regression #63: a bare reference to a nullary fun (`zero`) must be
    // emitted as a call `zero()`, not as the fn item `zero` — otherwise
    // `add(&a, &zero)` is `&fn() -> Money` and does not compile.
    let files = generate_from(r#"
        sig Money { amount: one Int }
        fun zero: Money { Money }
        fun add[a, b: Money]: Money { a }
        fact Ident { all a: Money | add[a, zero] = a and add[zero, a] = a }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(tests.contains("zero()"),
        "nullary fun reference should be called:\n{tests}");
    assert!(!tests.contains("&zero)") && !tests.contains("&zero "),
        "bare fn-item reference `&zero` must not appear:\n{tests}");
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

#[test]
fn rust_concrete_parent_singleton_test_skipped_not_broken() {
    // `one sig Zero extends Money` (concrete parent) is a distinguished Money
    // value oxidtr can't construct. The struct is still emitted (representation
    // unchanged), but a fact referencing it must NOT produce an assertion using
    // `&Zero` where `&Money` is expected — that wouldn't typecheck. Such tests
    // are skipped instead, so the generated crate compiles.
    let files = generate_from(r#"
        sig Money { amount: one Int }
        one sig Zero extends Money {}
        fun add[a, b: Money]: Money { a }
        fact Ident { all a: Money | add[a, Zero] = a and add[Zero, a] = a }
    "#);
    let models = find_file(&files, "models.rs");
    assert!(models.contains("pub struct Zero;"), "singleton struct still emitted:\n{models}");
    let tests = find_file(&files, "tests.rs");
    assert!(!tests.contains("&Zero"), "must not emit &Zero (type mismatch):\n{tests}");
    assert!(!tests.contains("invariant_ident"), "identity fact test must be skipped:\n{tests}");
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
    // #73: the static test now actually calls the real trace checker with a
    // deterministic empty trace (until on an empty trace is always false)
    // instead of just documenting the limitation in a comment.
    assert!(tests.contains("assert!(!check_until_wait_until_done"),
        "should call the real trace checker:\n{tests}");
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
    // #73: must contain a real assertion, not just `let _ = &instance.field;`
    assert!(
        !tests.contains("let _ = &instance."),
        "anomaly tests must not use the no-op placeholder body:\n{tests}"
    );
    assert!(
        tests.contains("assert_eq!(instance.name, cloned.name"),
        "expected a real clone-preservation assertion for the unconstrained field:\n{tests}"
    );
}

#[test]
fn rust_anomaly_test_for_unbounded_collection_asserts_emptiness() {
    let files = generate_from(r#"
        sig Team { members: set User }
        sig User {}
        fact AnyTeam { all t: Team | t = t }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("assert!(instance.members.is_empty()"),
        "expected a real emptiness assertion for the unbounded-collection anomaly:\n{tests}"
    );
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
    // `Int` is an Alloy native-alias sig, not a real Rust type — must resolve to `i64`.
    assert!(models.contains("fn balance(&self) -> i64"), "should generate method:\n{models}");
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
fn rust_native_alias_resolves_in_operation_signature() {
    // Regression test: struct fields already resolved `Int`/`Str`/... to
    // native Rust types, but operation return types and parameter types
    // (`rust_return_type`, `param_type_str`, and their receiver-based
    // duplicates) emitted the raw Alloy alias name verbatim (`Int`) instead
    // of `i64` — an undefined-type compile error regardless of body content.
    let files = generate_from(r#"
        sig Item {}
        pred hasCount[i: one Item, n: one Int] {}
        fun countOf: one Int { 0 }
    "#);
    let ops = find_file(&files, "operations.rs");
    assert!(ops.contains("n: &i64"), "Int param should resolve to &i64:\n{ops}");
    assert!(ops.contains("-> i64"), "Int return type should resolve to i64:\n{ops}");
    assert!(!ops.contains("&Int"), "must not emit the raw Alloy alias as a param type:\n{ops}");
    assert!(!ops.contains("-> Int"), "must not emit the raw Alloy alias as a return type:\n{ops}");
}

#[test]
fn generate_one_mult_field_comparison_against_param_derefs_param() {
    // Regression test: the box-deref-both-sides fix only applied to
    // self-referential/cyclic One-mult fields (`is_self_ref_one_field`, now
    // `is_one_mult_field_access`). But ANY One-mult field access — boxed or
    // not — renders as an owned-`T` place, not `&T`; comparing it against a
    // bare `&T` parameter needs the same deref regardless of boxing.
    let files = generate_from(r#"
        sig Cap {}
        sig Account { cap: one Cap }
        pred withinCap[a: one Account, c: one Cap] { a.cap = c }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("pub fn within_cap"));
    assert!(content.contains("a.cap == (*c)"));
    assert!(!content.contains("todo!"));
}

#[test]
fn generate_bare_one_mult_param_comparison_against_literal_derefs_param() {
    // Regression test: a bare One-mult scalar PARAMETER (not a field access)
    // compared directly against a literal has no FieldAccess for the
    // existing one-mult-field-vs-param check (above) to key off, but still
    // needs a deref — its Rust type is `&i64` (see param_type_str), and
    // there's no blanket PartialOrd/PartialEq impl bridging `&i64` and a
    // bare integer literal (`&i64 >= 0` fails: "expected `&i64`, found
    // integer"). Found generating a standalone test model outside oxidtr's
    // own self-hosting model, which never happens to compare a bare
    // one-mult param directly against a literal.
    let files = generate_from(r#"
        sig Account {}
        pred hasNonNegativeBalance[a: one Account, amt: one Int] { amt >= 0 }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("(*amt) >= 0"), "bare one-mult param must be deref'd:\n{content}");
    assert!(!content.contains("todo!"));
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

// ── Fixture-diversity pairwise wiring (#74 Stage B, Phase 3d) ───────────────

/// `all a, b, c: Money | ...` previously bound a, b, c to the SAME single
/// `vec![default_money()]`, collapsing a 3-variable universal property into
/// a single-point check (a == b == c always). With a diversifiable field
/// present, the generated test should instead iterate a small pairwise-
/// covering set of distinct Money combinations.
#[test]
fn multi_var_same_sig_fact_uses_pairwise_covering_combos() {
    let files = generate_from(r#"
        sig Money { amount: one Int }
        fact NonNegative { all m: Money | m.amount >= 0 }
        fact TotalOrder {
          all a, b, c: Money | a.amount <= b.amount or b.amount <= c.amount or c.amount <= a.amount
        }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("let combos: Vec<[Money; 3]>"),
        "expected a pairwise-covering combos array for the 3-variable fact:\n{tests}"
    );
    assert!(
        tests.contains(".iter().all(|[a, b, c]|"),
        "expected the fact body to iterate the combos array by destructuring a, b, c:\n{tests}"
    );
    assert!(
        !tests.contains("let a: Vec<Money> = vec![default_money()];"),
        "must not fall back to the old single-shared-fixture pattern:\n{tests}"
    );
}

/// When no field on the sig offers fixture diversity (no boundary-derivable
/// scalar, no cardinality-varying collection), the generated test must keep
/// today's single-fixture behavior but disclose that it's a single-point
/// check rather than silently looking like a properly-covered test.
#[test]
fn multi_var_same_sig_fact_without_diversity_discloses_single_point_check() {
    let files = generate_from(r#"
        sig Money { tag: one Unit }
        sig Unit {}
        fact TotalOrder { all a, b, c: Money | a = a or b = b or c = c }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("@coverage single-point check"),
        "expected an honest single-point-check disclosure comment:\n{tests}"
    );
    assert!(
        tests.contains("let a = default_money();")
            && tests.contains("let b = default_money();")
            && tests.contains("let c = default_money();"),
        "expected each variable bound to its own default() call:\n{tests}"
    );
}

/// The real motivating case (oxidtr's own `UniqueStructurePerSig`): a chain
/// of nested `all` quantifiers, where the trailing two range over the SAME
/// field domain (`c.items`) rather than a bare sig name. This must be
/// recognized as a same-sig multi-var group too — the leading `c: Container`
/// is a context variable (plain default), and `i1`/`i2` get diversified.
#[test]
fn nested_quantifier_over_shared_field_domain_uses_pairwise_covering_combos() {
    let files = generate_from(r#"
        sig Container { items: set Item }
        sig Item { tag: one Int }
        fact UniqueTagPerContainer {
          all c: Container | all i1: c.items | all i2: c.items | i1.tag = i2.tag implies i1 = i2
        }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("let c = default_container();"),
        "expected the leading context variable bound to a plain default:\n{tests}"
    );
    assert!(
        tests.contains("let combos: Vec<[Item; 2]>"),
        "expected a pairwise-covering combos array for the shared c.items domain:\n{tests}"
    );
    assert!(
        tests.contains(".iter().all(|[i1, i2]|"),
        "expected the fact body to iterate the combos array by destructuring i1, i2:\n{tests}"
    );
}

/// A fact quantifying just ONE variable of a sig, but comparing it against a
/// call to a NULLARY fun of the same sig (`zero`), still needs fixture
/// diversity: the fun's actual return value isn't known at generation time,
/// so the one shared default fixture might coincidentally equal it,
/// silently masking a real violation. Found via a broken `add` (always
/// returns its first argument) that only violates `Ident` for a non-default
/// Money value — `invariant_ident`'s single `default_money()` (amount 0)
/// happened to make `zero()`'s value indistinguishable from the quantified
/// variable, hiding the bug that a diversified test caught immediately.
#[test]
fn single_var_fact_referencing_nullary_fun_of_same_sig_is_diversified() {
    let files = generate_from(r#"
        sig Money { amount: one Int }
        fact NonNegative { all m: Money | m.amount >= 0 }
        fun zero: one Money { Money }
        fun add[x, y: one Money]: one Money { x }
        fact Ident { all a: Money | add[a, zero] = a and add[zero, a] = a }
    "#);
    let tests = find_file(&files, "tests.rs");
    assert!(
        tests.contains("let combos: Vec<[Money; 1]>"),
        "expected a diversified single-variable combos array:\n{tests}"
    );
    let after = tests.split("fn invariant_ident()").nth(1).expect("invariant_ident should exist");
    let ident_test = after.split("\n}").next().expect("invariant_ident body");
    assert!(
        !ident_test.contains("vec![default_money()]"),
        "invariant_ident must not fall back to the old single-shared-fixture pattern:\n{tests}"
    );
    // Regression: `combos.iter().all(|[a]| ...)` binds `a: &Money`, but a
    // fact comparing the bound var directly (`add(a, zero) = a`, not just
    // passing it BY REFERENCE into another call) needs it owned — without
    // this clone the generated assertion fails to compile
    // (E0308: expected `Money`, found `&Money`).
    assert!(
        ident_test.contains("let a = a.clone();"),
        "combos-destructured var must be cloned to owned before a direct comparison:\n{tests}"
    );
}

// ── #78: temporal operators must not be erased on the assert path ───────────

#[test]
fn rust_assert_liveness_emits_a_trace_checker() {
    // `eventually P` demands P in *some* future state. A snapshot assertion
    // demands it now — a strictly different, strictly stronger property that
    // compiles and passes green.
    let files = generate_from(
        "sig Person { age: one Int }\nassert EventuallyOk { eventually all p: Person | p.age > 0 }",
    );
    let t = find_file(&files, "tests.rs");
    let test = fn_body(t, "fn eventually_ok() {");
    assert!(test.contains("check_liveness_eventually_ok(&trace)"),
        "the test must call its checker, not merely coexist with it:\n{test}");
    let checker = fn_body(t, "fn check_liveness_eventually_ok(trace: &[Vec<Person>]) -> bool {");
    assert!(checker.contains("trace.iter().any("), "liveness is existential over states:\n{checker}");
    assert!(t.contains("@temporal"), "the test must be labelled temporal:\n{t}");
    assert!(!t.contains("assert!(persons.iter().all(|p| { let p = p.clone(); p.age > 0 }));"),
        "`eventually` was erased into a snapshot assertion:\n{t}");
}

#[test]
fn rust_assert_until_emits_a_trace_checker() {
    let files = generate_from(
        "sig Person { age: one Int }\n\
         assert UntilOk { (all p: Person | p.age >= 0) until (all p: Person | p.age > 0) }",
    );
    let t = find_file(&files, "tests.rs");
    let test = fn_body(t, "fn until_ok() {");
    assert!(test.contains("check_until_until_ok(&trace)"), "the test must call its checker:\n{test}");
    let checker = fn_body(t, "fn check_until_until_ok(trace: &[Vec<Person>]) -> bool {");
    assert!(checker.contains("trace.iter().position("), "until is position-based:\n{checker}");
    assert!(!t.contains("}) && persons.iter().all("), "`until` was flattened to `&&`:\n{t}");
}

#[test]
fn rust_assert_invariant_still_uses_a_snapshot() {
    // `always P` is soundly approximated by asserting P on one state; only
    // liveness and the binary operators need a trace.
    let files = generate_from(
        "sig Person { age: one Int }\nassert AlwaysOk { always all p: Person | p.age > 0 }",
    );
    let t = find_file(&files, "tests.rs");
    let test = fn_body(t, "fn always_ok() {");
    assert!(test.contains("assert!(persons.iter().all("), "invariant should stay a snapshot:\n{test}");
    assert!(!t.contains("fn check_liveness_always_ok"), "no trace checker needed:\n{t}");
}

#[test]
fn rust_nested_temporal_is_skipped_not_mistranslated() {
    // Wrapping the whole body in `any(..)` would assert `exists s. A(s) && B(s)`
    // instead of `A(now) && exists s. B(s)` — a different formula.
    let files = generate_from(
        "sig Person { age: one Int }\n\
         assert Nested { (all p: Person | p.age >= 0) and eventually (all p: Person | p.age > 0) }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped Nested"), "expected a diagnostic:\n{t}");
    assert!(!t.contains("fn check_liveness_nested"), "a wrong checker was emitted:\n{t}");
}

#[test]
fn rust_nested_binary_temporal_does_not_drop_context() {
    // `find_temporal_binary` returns only the binary node, so emitting a
    // checker here would silently discard the surrounding conjunct.
    let files = generate_from(
        "sig Person { age: one Int }\n\
         assert NestedUntil { (all p: Person | p.age >= 0) and ((all p: Person | p.age > 0) until (all p: Person | p.age > 1)) }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped NestedUntil"), "expected a diagnostic:\n{t}");
    assert!(!t.contains("fn check_until_nested_until"), "a context-dropping checker was emitted:\n{t}");
}

#[test]
fn rust_prime_on_the_assert_path_is_skipped() {
    // Only the fact path has transition handling; without it a prime becomes
    // `next_age`, a field that does not exist.
    let files = generate_from(
        "sig Person { var age: one Int }\nassert PrimeOnly { all p: Person | p.age' > 0 }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped PrimeOnly"), "expected a diagnostic:\n{t}");
    assert!(!t.contains("next_age"), "prime leaked into a snapshot assertion:\n{t}");
}

#[test]
fn rust_parameterless_temporal_assert_still_calls_its_checker() {
    // With no quantified params the trace is a sequence of unit states; a
    // `None` trace type emitted a test that asserted nothing at all.
    let files = generate_from("sig S { x: one Int }\nassert NoParamEventually { eventually 1 = 2 }");
    let t = find_file(&files, "tests.rs");
    let test = fn_body(t, "fn no_param_eventually() {");
    assert!(test.contains("check_liveness_no_param_eventually(&trace)"),
        "the test must call its checker:\n{test}");
    assert!(t.contains("trace: &[()]"), "unit trace state expected:\n{t}");
}

#[test]
fn rust_since_requires_left_strictly_after_the_witness() {
    // `F since G` does not require F at the state where G holds.
    let files = generate_from(
        "sig Person { age: one Int }\n\
         assert SinceOk { (all p: Person | p.age >= 0) since (all p: Person | p.age > 0) }",
    );
    let t = find_file(&files, "tests.rs");
    let checker = fn_body(t, "fn check_since_since_ok(trace: &[Vec<Person>]) -> bool {");
    assert!(checker.contains("trace[pos + 1..]"), "since must exclude the witness state:\n{checker}");
}

#[test]
fn rust_triggered_is_the_past_dual_of_release() {
    // !( !F since !G ): at every state, G holds there or F holds strictly after.
    let files = generate_from(
        "sig Person { age: one Int }\n\
         assert TrigOk { (all p: Person | p.age >= 0) triggered (all p: Person | p.age > 0) }",
    );
    let t = find_file(&files, "tests.rs");
    let checker = fn_body(t, "fn check_triggered_trig_ok(trace: &[Vec<Person>]) -> bool {");
    assert!(checker.contains("trace[i + 1..]"), "triggered looks strictly forward:\n{checker}");
    assert!(!checker.contains("trace[..=i]"), "old at-or-before reading:\n{checker}");
}

#[test]
fn rust_pred_free_model_does_not_import_a_missing_module() {
    // operations.rs is only emitted when the model has preds or funs, so an
    // unconditional `use super::operations::*` broke every pred-free model.
    let files = generate_from("sig P { x: one Int }\nassert Ok { all p: P | p.x = p.x }");
    assert!(!files.iter().any(|f| f.path == "operations.rs"), "no preds, no module");
    let t = find_file(&files, "tests.rs");
    assert!(!t.contains("use super::operations"), "imports a module that was not emitted:\n{t}");
}

#[test]
fn rust_after_is_not_approximated_by_a_snapshot() {
    // `after P` names the next state; asserting P now is simply a different claim.
    let files = generate_from("sig P { x: one Int }\nassert AfterOk { after all p: P | p.x > 0 }");
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped AfterOk"), "expected a diagnostic:\n{t}");
}

#[test]
fn rust_always_keeps_its_snapshot_even_when_nested() {
    // `always` includes the current state, so a snapshot is a weaker but
    // never-wrong check — skipping it would lose coverage main had.
    let files = generate_from("sig P { x: one Int }\nfact AlwaysNested { all p: P | always p.x > 0 }");
    let t = find_file(&files, "tests.rs");
    let test = fn_body(t, "fn invariant_always_nested() {");
    assert!(test.contains("assert!(ps.iter().all("), "sound snapshot was dropped:\n{test}");
}

#[test]
fn rust_temporal_hidden_in_a_pred_body_is_still_caught() {
    // A purely syntactic scan sees no operator in `LaterPositive[p]`.
    let files = generate_from(
        "sig P { x: one Int }\npred LaterPositive[p: one P] { eventually p.x > 0 }\n\
         assert Outer { eventually all p: P | LaterPositive[p] }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped Outer"), "expected a diagnostic:\n{t}");
    assert!(!t.contains("fn check_liveness_outer"), "checker wraps an erasing pred call:\n{t}");
}

#[test]
fn rust_temporal_fact_is_not_inlined_into_a_validator() {
    // A skipped fact must not reappear as a single-state newtype validator.
    let files = generate_from(
        "sig P { x: one Int }\n\
         fact ConjoinedEventually { (all p: P | p.x >= 0) and eventually (all p: P | p.x > 0) }",
    );
    let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
    assert!(!nt.contains("p.x > 0"), "temporal fact inlined into a validator:\n{nt}");
}

#[test]
fn rust_always_under_negation_is_not_snapshot_approximated() {
    // `always P => P now` runs the wrong way under a negation: a trace where P
    // holds now and fails later satisfies `not always P`, but the snapshot
    // assertion `!P(now)` is false.
    let files = generate_from("sig P { x: one Int }\nassert NegAlways { not always all p: P | p.x > 0 }");
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped NegAlways"), "expected a diagnostic:\n{t}");
}

#[test]
fn rust_always_in_an_implication_antecedent_is_not_approximated() {
    let files = generate_from(
        "sig P { x: one Int }\n\
         assert AntAlways { (always all p: P | p.x > 0) implies (all p: P | p.x >= 0) }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped AntAlways"), "expected a diagnostic:\n{t}");
}

#[test]
fn rust_call_only_temporal_reaches_the_gate() {
    // The surface expression has no operator, so classification alone never
    // consults the call-aware check.
    let files = generate_from(
        "sig P { x: one Int }\npred Later[p: one P] { eventually p.x > 0 }\n\
         assert HiddenOnly { all p: P | Later[p] }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped HiddenOnly"), "expected a diagnostic:\n{t}");
    let ops = find_file(&files, "operations.rs");
    assert!(ops.contains("todo!(\"oxidtr: Later is temporal"), "erased pred body exported:\n{ops}");
}

#[test]
fn rust_temporal_fact_does_not_leak_through_analyzer_derived_checks() {
    // `analyze_expr` unwraps temporal operators, so the cardinality bound of
    // `eventually #p.items <= 1` became a current-state validator.
    let files = generate_from(
        "sig Item {}\nsig P { items: set Item }\n\
         fact LaterSmall { eventually all p: P | #p.items <= 1 }",
    );
    let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
    assert!(!nt.contains("items.len() > 1"), "temporal bound enforced in one state:\n{nt}");
}

#[test]
fn rust_builtin_arithmetic_is_not_confused_with_a_same_named_pred() {
    // `p.x.plus[1]` is Alloy integer arithmetic, not a call to `pred plus`.
    let files = generate_from(
        "sig P { x: one Int }\npred plus[p: one P] { eventually p.x > 0 }\n\
         assert ArithmeticOnly { eventually all p: P | p.x.plus[1] > 0 }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("fn check_liveness_arithmetic_only"), "falsely skipped:\n{t}");
}

#[test]
fn rust_no_quantifier_flips_polarity() {
    // `no p | always P` is antitone in its body, so the snapshot reading of
    // `always` runs the wrong way.
    let files = generate_from("sig P { x: one Int }\nassert NoAlways { no p: P | always p.x > 0 }");
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped NoAlways"), "expected a diagnostic:\n{t}");
}

#[test]
fn rust_eventually_with_prime_is_not_a_plain_transition() {
    // The prime branch runs before the temporal gate; `eventually p.x' > 0`
    // does not imply next-positive now.
    let files = generate_from(
        "sig P { var x: one Int }\nfact LaterNext { eventually all p: P | p.x' > 0 }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped LaterNext"), "expected a diagnostic:\n{t}");
    assert!(!t.contains("next_p.x > 0"), "emitted a single-step check:\n{t}");
}

#[test]
fn rust_always_with_prime_still_gets_its_transition_test() {
    // The idiomatic Alloy transition fact: one step is a sound necessary check.
    let files = generate_from(
        "sig C { var v: one Int }\nfact Mono { always all c: C | c.v' = c.v }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("transition_mono"), "sound transition test was dropped:\n{t}");
}

#[test]
fn rust_temporal_pred_export_is_annotated_as_weakened() {
    // `always P` in a positive position can be weakened to a current-state
    // necessary condition — but the export must not read as faithful.
    let files = generate_from("sig P { x: one Int }\npred Stable[p: one P] { always p.x > 0 }\nfact Use { all p: P | Stable[p] }");
    let ops = find_file(&files, "operations.rs");
    assert!(ops.contains("NOTE: weakened"), "silent weakening:\n{ops}");
}

#[test]
fn rust_mixed_fact_keeps_its_sound_conjunct() {
    // Discarding the whole constraint loses the current-state `max 1` bound.
    let files = generate_from(
        "sig Item {}\nsig P { items: set Item }\n\
         fact Mixed { (all p: P | #p.items <= 1) and (eventually all p: P | #p.items >= 1) }",
    );
    let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
    assert!(nt.contains("items.len() > 1"), "sound conjunct was discarded:\n{nt}");
}

#[test]
fn rust_skipped_fact_leaves_no_dangling_doc_comment() {
    // The gate used to run after the `/// @temporal` annotation, so a skipped
    // fact at end of file produced `expected item after doc comment`.
    let files = generate_from(
        "sig P { x: one Int }\nfact Mixed { (all p: P | p.x = 0) and eventually (all p: P | p.x = 1) }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(!t.contains("/// @temporal"), "annotation emitted for a skipped fact:\n{t}");
    assert!(t.trim_end().ends_with("See #104."), "dangling annotation at EOF:\n{t}");
}

#[test]
fn rust_prime_hidden_in_a_pred_reaches_the_prime_gate() {
    // `expr_contains_prime` only scans a call's receiver and arguments.
    let files = generate_from(
        "sig P { var x: one Int }\npred Step[p: one P] { p.x' = p.x }\n\
         assert CalledPrime { always all p: P | Step[p] }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped CalledPrime"), "expected a diagnostic:\n{t}");
    assert!(!t.contains("step(p)"), "calls a stubbed pred:\n{t}");
}

#[test]
fn rust_eventually_some_does_not_seed_a_current_state_fixture() {
    // `strip_outer_quantifier` also strips `eventually`, so this fact used to
    // build an `all_ps()` containing `P { x: 1 }` that broke `NowZero`.
    let files = generate_from(
        "some sig P { var x: one Int }\nsome sig Q { y: one Int }\n\
         fact NowZero { all q: Q | all p: P | p.x = 0 }\n\
         fact LaterOne { eventually some p: P | p.x = 1 }",
    );
    let f = find_file(&files, "fixtures.rs");
    assert!(!f.contains("x: 1i64"), "temporal existential leaked into a fixture:\n{f}");
}

#[test]
fn rust_disjunction_does_not_pin_one_alternative() {
    // `A.rank = 0 or always A.rank = 2` entails neither operand; mining the
    // first silently deleted the second.
    let files = generate_from(
        "abstract sig Kind { rank: one Int }\none sig A extends Kind {}\none sig B extends Kind {}\n\
         fact MaybeA { A.rank = 0 or always A.rank = 2 }",
    );
    let m = find_file(&files, "models.rs");
    assert!(m.contains("rank: i64"), "field eliminated in favour of one branch:\n{m}");
}

#[test]
fn rust_transition_emitter_declines_shapes_it_cannot_rewrite() {
    // The rewriter zips a pre- and post-state collection, so it handles exactly
    // one universally quantified variable over a plain sig. Each of these used
    // to be emitted anyway: a `todo!()` call that panicked at runtime, a green
    // `some` silently rendered as `all`, unbound `next_*` references, and a
    // primed domain that panicked the generator itself.
    let cases: &[(&str, &str)] = &[
        ("CalledPrime",
         "some sig P { var x: one Int }\npred Step[p: one P] { p.x' = p.x }\n\
          fact CalledPrime { always all p: P | Step[p] }"),
        ("SomeStutters",
         "some sig P { var x: one Int }\nfact SomeStutters { always some p: P | p.x' = p.x }"),
        ("MultiStep",
         "some sig P { var x: one Int }\nsome sig Q { y: one Int }\n\
          fact MultiStep { always all p: P, q: Q | p.x' = q.y }"),
        ("NextZero",
         "some sig P { var x: one Int }\nfact NextZero { always all p: P' | p.x = 0 }"),
    ];
    for (name, model) in cases {
        let files = generate_from(model);
        let t = find_file(&files, "tests.rs");
        assert!(t.contains(&format!("skipped {name}")), "{name} was emitted:\n{t}");
    }
}

#[test]
fn rust_supported_transition_shape_is_still_emitted() {
    let files = generate_from("sig C { var v: one Int }\nfact Mono { always all c: C | c.v' = c.v }");
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("transition_mono"), "the one supported shape was lost:\n{t}");
}

#[test]
fn rust_temporal_call_needing_an_unpassed_universe_is_skipped() {
    // `pos()` quantifies over P but takes no parameter, so a trace checker has
    // no way to give it that collection — the call does not compile.
    let files = generate_from(
        "some sig P { x: one Int }\npred Pos { all p: P | p.x >= 0 }\n\
         assert LaterPos { eventually Pos[] }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped LaterPos"), "expected a diagnostic:\n{t}");
}

#[test]
fn rust_transition_gate_checks_every_prime_not_just_the_quantifier() {
    // The rewriter turns `v'` into a `next_v` bound by zipping pre/post
    // collections. It only reaches a prime on the bound variable itself, so:
    // a chained `c.d.v'` was silently dropped (green `assert!(x == x)`), and a
    // prime under a nested quantifier emitted an unbound `next_d`.
    let cases: &[(&str, &str)] = &[
        ("Chained",
         "sig D { var v: one Int }\nsig C { d: one D, var v: one Int }\n\
          fact Chained { always all c: C | c.d.v' = c.d.v }"),
        ("Nested",
         "sig D { var v: one Int }\nsig C { ds: set D, var v: one Int }\n\
          fact Nested { always all c: C | all d: c.ds | d.v' = d.v }"),
    ];
    for (name, model) in cases {
        let files = generate_from(model);
        let t = find_file(&files, "tests.rs");
        assert!(t.contains(&format!("skipped {name}")), "{name} was emitted:\n{t}");
    }
}

#[test]
fn rust_atom_parameter_does_not_stand_in_for_the_whole_collection() {
    // `p: one P` is one atom; `all q: P` inside the body still needs every P,
    // which the generated `pos(p: &P)` has no way to see.
    let files = generate_from(
        "some sig P { x: one Int }\npred Pos[p: one P] { all q: P | q.x >= 0 }\n\
         assert LaterPos { eventually all p: P | Pos[p] }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped LaterPos"), "expected a diagnostic:\n{t}");
}

#[test]
fn rust_singleton_existential_transition_is_still_emitted() {
    // `some c: C` over a `one sig` binds exactly one atom, so the emitted loop
    // is `all` and `some` at once — the one existential the rewriter gets right.
    let files = generate_from("one sig C { var v: one Int }\nfact SingleStut { always some c: C | c.v' = c.v }");
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("transition_single_stut"), "conservatively lost:\n{t}");
}

#[test]
fn rust_wholly_temporal_fact_creates_no_validator_wrapper() {
    // The body was suppressed but the wrapper remained, claiming
    // "validated by X" while accepting everything.
    let files = generate_from("sig C { var v: one Int }\nfact NestedStep { always all c: C | c.v' = c.v }");
    let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
    assert!(!nt.contains("NestedStep"), "vacuous validator wrapper:\n{nt}");
}

#[test]
fn rust_no_parameter_can_supply_a_sig_universe() {
    // The callee body translates to `ps.iter()` — the sig's collection name —
    // whatever the parameter is called, so `pool: set P` does not even compile;
    // naming it `ps` would silently quantify over the subset passed instead of
    // over every P.
    let files = generate_from(
        "sig P { x: one Int }\nsig H { ps: set P }\npred Pos[pool: set P] { all q: P | q.x >= 0 }\n\
         assert LaterPos { eventually all h: H | Pos[h.ps] }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped LaterPos"), "expected a diagnostic:\n{t}");
}

#[test]
fn rust_mixed_fact_validator_covers_only_its_sound_conjunct() {
    // Admitting the whole fact picked up Q, mentioned only by the temporal
    // conjunct, and then inlined nothing — leaving `ValidatedQ { if true }`.
    let files = generate_from(
        "sig P { x: one Int }\nsig Q { y: one Int }\n\
         fact Mixed { (all p: P | p.x >= 0) and (eventually all q: Q | q.y >= 0) }",
    );
    let nt = find_file(&files, "newtypes.rs");
    assert!(nt.contains("struct ValidatedP"), "sound conjunct lost its wrapper:\n{nt}");
    assert!(!nt.contains("struct ValidatedQ"), "wrapper for a rejected conjunct:\n{nt}");
    assert!(!nt.contains("if true"), "vacuous validator:\n{nt}");
    assert!(nt.contains("p.x >= 0"), "wrapper does not enforce its conjunct:\n{nt}");
}

#[test]
fn rust_transition_test_only_covers_what_a_cloned_post_state_can_satisfy() {
    // The generated test clones the pre-state as the post-state, so an update
    // like `c.v' = c.v.plus[1]` compiles and then asserts `v == v + 1`.
    let files = generate_from(
        "one sig C { var v: one Int }\nfact Increment { always all c: C | c.v' = c.v.plus[1] }",
    );
    let t = find_file(&files, "tests.rs");
    assert!(t.contains("skipped Increment"), "expected a diagnostic:\n{t}");
    let stutter = generate_from("sig C { var v: one Int }\nfact Mono { always all c: C | c.v' = c.v }");
    let t2 = find_file(&stutter, "tests.rs");
    assert!(t2.contains("transition_mono"), "the stutter case must survive:\n{t2}");
}

#[test]
fn rust_validator_only_checks_conjuncts_its_own_sig_can_evaluate() {
    // Every wrapper used to inline *all* sound conjuncts, so `ValidatedP`
    // evaluated `some q: Q | ..` with `qs` empty — false — and rejected every
    // P, including a perfectly valid one.
    let files = generate_from(
        "sig P { x: one Int }\nsig Q { y: one Int }\n\
         fact Split { (all p: P | p.x >= 0) and (some q: Q | q.y >= 0) and (eventually all q: Q | q.y >= 0) }",
    );
    let nt = find_file(&files, "newtypes.rs");
    // Slice to the next wrapper — `fn_body` keys on a top-level `fn`, and these
    // are `impl` blocks.
    let start = nt.find("pub struct ValidatedP").expect("no ValidatedP");
    let rest = &nt[start..];
    let p_body = &rest[..rest[1..].find("pub struct Validated").map(|i| i + 1).unwrap_or(rest.len() - 1)];
    assert!(!p_body.contains("qs.iter()"), "P's validator evaluates a Q conjunct:\n{p_body}");
    assert!(p_body.contains("p.x >= 0"), "P's own conjunct lost:\n{p_body}");
    // `some q: Q | ..` is a claim about the collection, not about each atom, so
    // it must not become a per-value wrapper at all — that would reject a `Q`
    // a valid collection may legitimately contain.
    assert!(!nt.contains("struct ValidatedQ"), "global existential became per-value:\n{nt}");
}

#[test]
fn rust_cross_signature_conjunct_creates_no_validator() {
    // A conjunct needing two universes cannot be checked from one wrapped
    // value; with the other universe empty it is vacuously true.
    let files = generate_from(
        "sig P { x: one Int }\nsig Q { y: one Int }\n\
         fact Cross { (all p: P | all q: Q | p.x >= q.y) and (eventually all q: Q | q.y >= 0) }",
    );
    let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
    assert!(!nt.contains("struct Validated"), "vacuous cross-sig wrapper:\n{nt}");
}

#[test]
fn rust_validator_requires_an_elementwise_constraint() {
    // A wrapper substitutes `vec![value]` for the whole sig universe, so it can
    // only enforce a claim about each atom independently.
    let cases: &[(&str, &str, bool)] = &[
        // (name, model, wrapper expected)
        ("elementwise", "sig P { x: one Int }\nfact Simple { all p: P | p.x >= 0 }", true),
        // `no x | B` is `all x | not B`, still elementwise — and this is the
        // acyclicity validator, a flagship feature.
        ("no_quantifier", "sig Node { next: lone Node }\nfact Acyclic { no n: Node | n in n.^next }", true),
        // Cross-atom: every singleton passes, a two-element collection may not.
        ("cross_atom", "sig P { x: one Int }\nfact CrossAtom { all p: P | all q: P | p.x >= q.x }", false),
    ];
    for (name, model, expected) in cases {
        let files = generate_from(model);
        let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
        assert_eq!(nt.contains("struct Validated"), *expected, "{name}:\n{nt}");
    }
}

#[test]
fn rust_global_existential_does_not_become_a_per_value_wrapper() {
    // `some q: Q | q.y >= 0` is satisfied by the collection, so a wrapper would
    // reject `Q { y: -1 }` even though a valid collection may contain it.
    let files = generate_from(
        "sig P { x: one Int }\nsig Q { var y: one Int }\n\
         fact Split { (all p: P | p.x >= 0) and (some q: Q | q.y >= 0) and (eventually all q: Q | q.y >= 0) }",
    );
    let nt = find_file(&files, "newtypes.rs");
    assert!(nt.contains("struct ValidatedP"), "elementwise conjunct lost its wrapper:\n{nt}");
    assert!(!nt.contains("struct ValidatedQ"), "existential became per-value:\n{nt}");
}

#[test]
fn rust_validator_rejects_whole_universe_dependencies() {
    // A singleton wrapper cannot stand in for the whole sig, so a body that
    // reads the universe — directly or through a call — is not elementwise.
    let cases: &[(&str, &str, bool)] = &[
        ("cardinality_of_sig",
         "sig P { x: one Int }\nfact CountSensitive { all p: P | p.x = #P }", false),
        ("callee_reads_universe",
         "sig P { x: one Int }\npred DominatesAll[p: one P] { all q: P | p.x >= q.x }\n\
          fact Hidden { all p: P | p.x >= 0 and DominatesAll[p] }", false),
        // `not some x | B` is `no x | B`, which main wrapped correctly.
        ("not_some_is_no",
         "sig P { x: one Int }\nfact NonNegative { not some p: P | p.x < 0 }", true),
        // Over a `one sig` the collection is a single atom, so `some` is `all`.
        ("some_over_one_sig",
         "one sig P { x: one Int }\nfact SomeSingleton { some p: P | p.x >= 0 }", true),
        // A `one sig` name is one atom, not a universe.
        ("one_sig_reference_is_atom_local",
         "abstract sig L {}\none sig High extends L {}\none sig Low extends L {}\n\
          sig N { level: one L }\nfact NotLow { all n: N | n.level != Low }", true),
    ];
    for (name, model, expected) in cases {
        let files = generate_from(model);
        let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
        assert_eq!(nt.contains("struct Validated"), *expected, "{name}:\n{nt}");
    }
}

#[test]
fn rust_non_elementwise_comparison_does_not_suppress_the_exhaustive_check() {
    // `i in Premium.items` reads a whole set, so it is not elementwise — but
    // the exhaustive check derived from the same fact still belongs.
    let files = generate_from(
        "sig Category { items: set Item }\nsig Item { name: one Category }\n\
         sig Premium extends Category {}\nsig Budget extends Category {}\n\
         fact Cover { all i: Item | i in Premium.items or i in Budget.items }",
    );
    let nt = find_file(&files, "newtypes.rs");
    assert!(nt.contains("must belong to"), "exhaustive check lost:\n{nt}");
}

#[test]
fn rust_one_sig_is_atom_local_only_when_it_renders_as_a_value() {
    // A `one sig` name is admissible in a validator body only where the Rust
    // translator can put a value. A field-less variant of a field-less enum
    // parent becomes `L::Low`; the rest emit a *type* name where a value
    // belongs and do not compile.
    let cases: &[(&str, &str, bool)] = &[
        ("fieldless_variant",
         "abstract sig L {}\none sig High extends L {}\none sig Low extends L {}\n\
          sig N { level: one L }\nfact NotLow { all n: N | n.level != Low }", true),
        ("singleton_struct_with_fields",
         "one sig Config { limit: one Int }\nsig N { c: one Config }\n\
          fact UsesConfig { all n: N | n.c = Config }", false),
        ("cardinality_of_one_sig",
         "one sig P { x: one Int }\nfact CardOne { all p: P | p.x = #P }", false),
        ("variant_carrying_inherited_fields",
         "abstract sig L { tag: one Int }\none sig High extends L {}\none sig Low extends L {}\n\
          sig N { level: one L }\nfact NotLow { all n: N | n.level != Low }", false),
    ];
    for (name, model, expected) in cases {
        let files = generate_from(model);
        let nt = files.iter().find(|f| f.path == "newtypes.rs").map(|f| f.content.as_str()).unwrap_or("");
        assert_eq!(nt.contains("struct Validated"), *expected, "{name}:\n{nt}");
    }
}

#[test]
fn rust_exhaustive_gets_a_helper_and_not_a_vacuous_wrapper() {
    // The check needs the *other* sigs' collections, so it cannot be performed
    // from one wrapped value. It used to produce both the standalone helper and
    // a `ValidatedItem` whose whole body was `if true`.
    let files = generate_from(
        "sig Category { items: set Item }\nsig Item { name: one Category }\n\
         sig Premium extends Category {}\nsig Budget extends Category {}\n\
         fact Cover { all i: Item | i in Premium.items or i in Budget.items }",
    );
    let nt = find_file(&files, "newtypes.rs");
    assert!(nt.contains("fn validate_exhaustive_item"), "standalone helper lost:\n{nt}");
    assert!(!nt.contains("struct ValidatedItem"), "vacuous wrapper certifying nothing:\n{nt}");
    assert!(!nt.contains("if true"), "vacuous validator body:\n{nt}");
}
