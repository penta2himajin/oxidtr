//! Distinct Alloy names must stay distinct after a backend renames them (#112).
//!
//! Alloy is case-sensitive; several target conventions are not. Lowercasing a
//! leading character to build a domain local, or capitalising one to build a
//! field, maps two model names onto one emitted name — and the generated code
//! either fails to compile or, in Rust's case, silently ranges a quantifier
//! over the wrong domain.

use oxidtr::backend;
use oxidtr::ir::{self, nodes::OxidtrIR};

/// `Foo` and `foo` differ only by case, so every leading-lowercase transform
/// collapses them.
const CASE_PAIR: &str = "\
sig Foo { tag: one Int }
sig foo { tag: one Int }
assert R { all a: Foo | all b: foo | a.tag = a.tag and b.tag = b.tag }
";

/// `item` and `Item` differ only by case, so every leading-uppercase transform
/// collapses them.
const FIELD_PAIR: &str = "\
sig Item {}
sig Box { item: one Item, Item: one Item }
";

fn lower(model: &str) -> OxidtrIR {
    ir::lower(&oxidtr::parser::parse(model).expect("parse")).expect("lower")
}

// ── the allocator ──────────────────────────────────────────────────────────

/// The point of the helper: two names that render the same must not stay the
/// same, and the suffix must come from a stable ordering rather than from
/// however a hash map happened to iterate.
#[test]
fn colliding_names_are_separated_in_declaration_order() {
    let ir = lower(CASE_PAIR);
    let lower_first = |n: &str| n.to_lowercase();
    assert_eq!(backend::disambiguate("Foo", &ir, lower_first), "foo");
    assert_eq!(backend::disambiguate("foo", &ir, lower_first), "foo2");
}

/// A model with no case-colliding names must render exactly as it did before,
/// or this change rewrites every generated file in the repository.
#[test]
fn a_model_without_collisions_is_untouched() {
    let ir = lower("sig Alpha {}\nsig Beta {}");
    let plural = |n: &str| format!("{}s", n.to_lowercase());
    assert_eq!(backend::disambiguate("Alpha", &ir, plural), "alphas");
    assert_eq!(backend::disambiguate("Beta", &ir, plural), "betas");
}

/// The suffix must not itself collide with a name already spoken for.
#[test]
fn a_suffix_that_is_already_taken_moves_on() {
    let ir = lower("sig A {}\nsig a {}\nsig a2 {}");
    let low = |n: &str| n.to_lowercase();
    let names: Vec<String> = ["A", "a", "a2"].iter()
        .map(|n| backend::disambiguate(n, &ir, low)).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 3, "names must stay distinct: {names:?}");
}

/// Determinism is a first principle: the same model must allocate the same
/// names on every run.
#[test]
fn allocation_is_deterministic() {
    let ir = lower(CASE_PAIR);
    let low = |n: &str| n.to_lowercase();
    for _ in 0..20 {
        assert_eq!(backend::disambiguate("foo", &ir, low), "foo2");
    }
}

// ── field names: rejected, not renamed ─────────────────────────────────────

/// A field name is compared against the model by `extract` and `check`, and a
/// numeric suffix is not reversible — `Item2` is itself a legal Alloy name. So
/// a target that cannot tell two fields apart says so instead of guessing.
#[test]
fn a_target_that_collapses_two_field_names_refuses_the_model() {
    for target in ["go", "cs", "lean"] {
        let collisions = backend::field_name_collisions(&lower(FIELD_PAIR), target);
        assert_eq!(collisions.len(), 1, "{target}: {collisions:?}");
        assert_eq!(collisions[0].sig, "Box");
        assert_eq!(collisions[0].sources, vec!["item".to_string(), "Item".to_string()],
            "the model's own names, in declaration order, are what the author has to act on");
    }
}

/// The other five keep Alloy's casing, and their target languages are
/// case-sensitive, so the same model is fine for them. Rejecting it everywhere
/// would fail a model that most targets handle correctly.
#[test]
fn a_target_that_keeps_alloys_casing_accepts_the_model() {
    for target in ["rust", "ts", "kt", "java", "swift"] {
        assert!(backend::field_name_collisions(&lower(FIELD_PAIR), target).is_empty(),
            "{target} keeps Alloy's field casing and has nothing to reject");
    }
}

/// oxidtr's own model must not be rejected by any target.
#[test]
fn no_target_rejects_oxidtrs_own_model() {
    let model = oxidtr::generate::load_model("models/oxidtr.als").expect("load");
    let ir = ir::lower(&model).expect("lower");
    for target in ["rust", "ts", "kt", "java", "swift", "go", "cs", "lean"] {
        assert!(backend::field_name_collisions(&ir, target).is_empty(),
            "{target} rejects models/oxidtr.als");
    }
}
