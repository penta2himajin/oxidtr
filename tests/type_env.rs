//! The shared, language-agnostic typing layer every backend's expr_translator
//! resolves fields through.
//!
//! Resolving `base.field` by scanning every sig for a field of that name is the
//! root cause behind #90 / #93 / #95 / #108 / #111 / #115: whenever two sigs
//! share a field name the scan silently picks whichever comes first, which is
//! how non-compiling closures and vet-clean-but-always-false membership tests
//! reached `main`. These tests pin the binding-directed behaviour instead.

use oxidtr::backend::type_env::{TypeEnv, collect_sig_names, expr_sig, resolve_field};
use oxidtr::ir;
use oxidtr::parser;
use oxidtr::parser::ast::*;

fn parse_and_lower(input: &str) -> ir::nodes::OxidtrIR {
    let model = parser::parse(input).expect("should parse");
    ir::lower(&model).expect("should lower")
}

fn var(name: &str) -> Expr {
    Expr::VarRef(name.to_string())
}

fn field(base: Expr, f: &str) -> Expr {
    Expr::FieldAccess {
        base: Box::new(base),
        field: f.to_string(),
    }
}

#[test]
fn sig_name_denotes_itself() {
    let ir = parse_and_lower("sig Box {}");
    let sigs = collect_sig_names(&ir);
    let env = TypeEnv::new();

    assert_eq!(expr_sig(&var("Box"), &sigs, &ir, &env), Some("Box".to_string()));
}

#[test]
fn unbound_variable_has_no_sig() {
    let ir = parse_and_lower("sig Box {}");
    let sigs = collect_sig_names(&ir);
    let env = TypeEnv::new();

    assert_eq!(expr_sig(&var("x"), &sigs, &ir, &env), None);
}

#[test]
fn bound_variable_resolves_through_the_binder() {
    let ir = parse_and_lower("sig Item {}\nsig Box { items: set Item }");
    let sigs = collect_sig_names(&ir);
    let mut env = TypeEnv::new();
    env.bind("b", "Box");

    assert_eq!(expr_sig(&var("b"), &sigs, &ir, &env), Some("Box".to_string()));
    assert_eq!(
        expr_sig(&field(var("b"), "items"), &sigs, &ir, &env),
        Some("Item".to_string())
    );
}

/// The defect the whole layer exists to prevent (#95). Both sigs declare
/// `next`, pointing at different targets; a name-keyed lookup returns whichever
/// sig the IR happens to list first regardless of what `n` is bound to.
#[test]
fn shared_field_name_resolves_by_binding_not_by_name() {
    let ir = parse_and_lower(
        "sig Page {}\nsig Slot {}\nsig Node { next: one Page }\nsig Cursor { next: one Slot }",
    );
    let sigs = collect_sig_names(&ir);

    let mut env = TypeEnv::new();
    env.bind("n", "Node");
    assert_eq!(
        expr_sig(&field(var("n"), "next"), &sigs, &ir, &env),
        Some("Page".to_string())
    );

    let mut env = TypeEnv::new();
    env.bind("c", "Cursor");
    assert_eq!(
        expr_sig(&field(var("c"), "next"), &sigs, &ir, &env),
        Some("Slot".to_string())
    );
}

/// Multiplicity has to come from the *resolved* declaration too — #108 turns
/// `no x.f` into `== null`, which is always false when `f` is a `set`.
#[test]
fn resolve_field_returns_the_declaration_of_the_bound_sig() {
    let ir = parse_and_lower(
        "sig Page {}\nsig Slot {}\nsig Node { next: one Page }\nsig Cursor { next: set Slot }",
    );
    let sigs = collect_sig_names(&ir);

    let mut env = TypeEnv::new();
    env.bind("c", "Cursor");
    let f = resolve_field(&var("c"), "next", &sigs, &ir, &env).expect("Cursor.next resolves");
    assert_eq!(f.mult, Multiplicity::Set);
    assert_eq!(f.target, "Slot");

    let mut env = TypeEnv::new();
    env.bind("n", "Node");
    let f = resolve_field(&var("n"), "next", &sigs, &ir, &env).expect("Node.next resolves");
    assert_eq!(f.mult, Multiplicity::One);
    assert_eq!(f.target, "Page");
}

/// A field declared on an abstract parent is reachable through a child (#93).
#[test]
fn inherited_field_resolves_through_the_child() {
    let ir = parse_and_lower(
        "sig Owner {}\nabstract sig Shape { owner: one Owner }\nsig Circle extends Shape {}",
    );
    let sigs = collect_sig_names(&ir);
    let mut env = TypeEnv::new();
    env.bind("c", "Circle");

    let f = resolve_field(&var("c"), "owner", &sigs, &ir, &env).expect("inherited field resolves");
    assert_eq!(f.target, "Owner");
    assert_eq!(
        expr_sig(&field(var("c"), "owner"), &sigs, &ir, &env),
        Some("Owner".to_string())
    );
}

#[test]
fn unknown_field_does_not_resolve() {
    let ir = parse_and_lower("sig Box { items: set Box }");
    let sigs = collect_sig_names(&ir);
    let mut env = TypeEnv::new();
    env.bind("b", "Box");

    assert!(resolve_field(&var("b"), "missing", &sigs, &ir, &env).is_none());
}

#[test]
fn chained_access_resolves_step_by_step() {
    let ir = parse_and_lower(
        "sig Leaf {}\nsig Mid { leaf: one Leaf }\nsig Top { mid: one Mid }",
    );
    let sigs = collect_sig_names(&ir);
    let mut env = TypeEnv::new();
    env.bind("t", "Top");

    let chained = field(field(var("t"), "mid"), "leaf");
    assert_eq!(expr_sig(&chained, &sigs, &ir, &env), Some("Leaf".to_string()));
}

/// `^f` and `*f` range over the same sig as `f`, so closure must be transparent
/// to typing — otherwise `x in y.^f` loses its element type.
#[test]
fn closures_preserve_the_underlying_sig() {
    let ir = parse_and_lower("sig Node { next: set Node }");
    let sigs = collect_sig_names(&ir);
    let mut env = TypeEnv::new();
    env.bind("n", "Node");

    let tc = Expr::TransitiveClosure(Box::new(field(var("n"), "next")));
    let rtc = Expr::ReflexiveClosure(Box::new(field(var("n"), "next")));
    assert_eq!(expr_sig(&tc, &sigs, &ir, &env), Some("Node".to_string()));
    assert_eq!(expr_sig(&rtc, &sigs, &ir, &env), Some("Node".to_string()));
}

/// Bindings are sequential: `all b: Box, x: b.items | ..` types `x` only if `b`
/// is already in scope when `x`'s domain is resolved.
#[test]
fn sequential_bindings_extend_scope_left_to_right() {
    let ir = parse_and_lower("sig Item {}\nsig Box { items: set Item }");
    let sigs = collect_sig_names(&ir);

    let bindings = vec![
        QuantBinding { vars: vec!["b".to_string()], domain: var("Box"), disj: false },
        QuantBinding { vars: vec!["x".to_string()], domain: field(var("b"), "items"), disj: false },
    ];
    let env = TypeEnv::new().extended(&bindings, &sigs, &ir);

    assert_eq!(env.sig_of("b"), Some("Box"));
    assert_eq!(env.sig_of("x"), Some("Item"));
}

/// An inner binder of the same name wins, and the outer scope is untouched.
#[test]
fn inner_binding_shadows_outer_without_mutating_it() {
    let ir = parse_and_lower("sig Item {}\nsig Box {}");
    let sigs = collect_sig_names(&ir);

    let mut outer = TypeEnv::new();
    outer.bind("x", "Box");
    let bindings = vec![QuantBinding { vars: vec!["x".to_string()], domain: var("Item"), disj: false }];
    let inner = outer.extended(&bindings, &sigs, &ir);

    assert_eq!(inner.sig_of("x"), Some("Item"));
    assert_eq!(outer.sig_of("x"), Some("Box"));
}

/// A domain that types to nothing must leave the variable untyped rather than
/// inherit a stale binding from an enclosing scope.
#[test]
fn unresolvable_domain_leaves_the_variable_untyped() {
    let ir = parse_and_lower("sig Box {}");
    let sigs = collect_sig_names(&ir);

    let bindings = vec![QuantBinding { vars: vec!["q".to_string()], domain: var("Nonexistent"), disj: false }];
    let env = TypeEnv::new().extended(&bindings, &sigs, &ir);

    assert_eq!(env.sig_of("q"), None);
}

#[test]
fn collect_sig_names_covers_every_structure() {
    let ir = parse_and_lower("sig Item {}\nabstract sig Shape {}\nsig Circle extends Shape {}");
    let sigs = collect_sig_names(&ir);

    assert!(sigs.contains("Item"));
    assert!(sigs.contains("Shape"));
    assert!(sigs.contains("Circle"));
    assert_eq!(sigs.len(), 3);
}
