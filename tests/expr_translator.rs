use oxidtr::backend::rust::expr_translator::{translate, translate_with_ir};
use oxidtr::parser::ast::*;
use oxidtr::ir::nodes::*;

// Helper constructors
fn var(name: &str) -> Expr {
    Expr::VarRef(name.to_string())
}

fn field(base: Expr, f: &str) -> Expr {
    Expr::FieldAccess {
        base: Box::new(base),
        field: f.to_string(),
    }
}

fn eq(left: Expr, right: Expr) -> Expr {
    Expr::Comparison {
        op: CompareOp::Eq,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn in_op(left: Expr, right: Expr) -> Expr {
    Expr::Comparison {
        op: CompareOp::In,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn and(left: Expr, right: Expr) -> Expr {
    Expr::BinaryLogic {
        op: LogicOp::And,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn implies(left: Expr, right: Expr) -> Expr {
    Expr::BinaryLogic {
        op: LogicOp::Implies,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn not(inner: Expr) -> Expr {
    Expr::Not(Box::new(inner))
}

fn card(inner: Expr) -> Expr {
    Expr::Cardinality(Box::new(inner))
}

fn all(v: &str, domain: Expr, body: Expr) -> Expr {
    Expr::Quantifier {
        kind: QuantKind::All,
        bindings: vec![QuantBinding { vars: vec![v.to_string()], domain, disj: false }],
        body: Box::new(body),
    }
}

fn some(v: &str, domain: Expr, body: Expr) -> Expr {
    Expr::Quantifier {
        kind: QuantKind::Some,
        bindings: vec![QuantBinding { vars: vec![v.to_string()], domain, disj: false }],
        body: Box::new(body),
    }
}

fn no(v: &str, domain: Expr, body: Expr) -> Expr {
    Expr::Quantifier {
        kind: QuantKind::No,
        bindings: vec![QuantBinding { vars: vec![v.to_string()], domain, disj: false }],
        body: Box::new(body),
    }
}


// ── IR builder helpers for lone/set field tests ──────────────────────────────

fn make_ir_two_sigs(
    sig_a: &str,
    field_name: &str,
    mult: Multiplicity,
    sig_b: &str,
) -> OxidtrIR {
    let node_b = StructureNode {
        name: sig_b.to_string(),
        is_enum: false,
        is_var: false,
        sig_multiplicity: SigMultiplicity::Default,
        parent: None,
        fields: vec![],
        intersection_of: vec![],
    };
    let node_a = StructureNode {
        name: sig_a.to_string(),
        is_enum: false,
        is_var: false,
        sig_multiplicity: SigMultiplicity::Default,
        parent: None,
        fields: vec![IRField {
            name: field_name.to_string(),
            mult,
            target: sig_b.to_string(),
            is_var: false, value_type: None, raw_union_type: None }],
        intersection_of: vec![],
    };
    OxidtrIR {
        structures: vec![node_a, node_b],
        constraints: vec![],
        operations: vec![],
        properties: vec![],
    }
}

fn make_ir_self_ref(sig_name: &str, field_name: &str, mult: Multiplicity) -> OxidtrIR {
    let node = StructureNode {
        name: sig_name.to_string(),
        is_enum: false,
        is_var: false,
        sig_multiplicity: SigMultiplicity::Default,
        parent: None,
        fields: vec![IRField {
            name: field_name.to_string(),
            mult,
            target: sig_name.to_string(),
            is_var: false, value_type: None, raw_union_type: None }],
        intersection_of: vec![],
    };
    OxidtrIR {
        structures: vec![node],
        constraints: vec![],
        operations: vec![],
        properties: vec![],
    }
}

// --- Tests ---

#[test]
fn translate_var_ref() {
    assert_eq!(translate(&var("x")), "x");
}

#[test]
fn translate_field_access() {
    assert_eq!(translate(&field(var("u"), "role")), "u.role");
}

#[test]
fn translate_chained_field_access() {
    assert_eq!(
        translate(&field(field(var("u"), "role"), "perms")),
        "u.role.perms"
    );
}

#[test]
fn translate_eq() {
    assert_eq!(translate(&eq(var("a"), var("b"))), "a == b");
}

#[test]
fn translate_not_eq() {
    let expr = Expr::Comparison {
        op: CompareOp::NotEq,
        left: Box::new(var("a")),
        right: Box::new(var("b")),
    };
    assert_eq!(translate(&expr), "a != b");
}

#[test]
fn translate_in_as_contains() {
    // `a in b` → `b.contains(&a)` for set membership
    assert_eq!(translate(&in_op(var("a"), var("b"))), "b.contains(&a)");
}

#[test]
fn translate_and() {
    assert_eq!(translate(&and(var("a"), var("b"))), "a && b");
}

#[test]
fn translate_or() {
    let expr = Expr::BinaryLogic {
        op: LogicOp::Or,
        left: Box::new(var("a")),
        right: Box::new(var("b")),
    };
    assert_eq!(translate(&expr), "a || b");
}

#[test]
fn translate_implies() {
    assert_eq!(translate(&implies(var("a"), var("b"))), "!a || b");
}

#[test]
fn translate_not() {
    assert_eq!(translate(&not(var("a"))), "!a");
}

#[test]
fn translate_cardinality() {
    assert_eq!(translate(&card(field(var("a"), "items"))), "a.items.len()");
}

#[test]
fn translate_all_quantifier() {
    let expr = all("x", var("xs"), eq(var("x"), var("x")));
    assert_eq!(translate(&expr), "xs.iter().all(|x| x == x)");
}

#[test]
fn translate_some_quantifier() {
    let expr = some("x", var("xs"), eq(var("x"), var("x")));
    assert_eq!(translate(&expr), "xs.iter().any(|x| x == x)");
}

#[test]
fn translate_no_quantifier() {
    let expr = no("x", var("xs"), eq(var("x"), var("x")));
    assert_eq!(translate(&expr), "!xs.iter().any(|x| x == x)");
}

#[test]
fn translate_nested_quantifiers() {
    // all u: users | all r: u.roles | r == r
    let inner = all("r", field(var("u"), "roles"), eq(var("r"), var("r")));
    let expr = all("u", var("users"), inner);
    assert_eq!(
        translate(&expr),
        "users.iter().all(|u| u.roles.iter().all(|r| r == r))"
    );
}

#[test]
fn translate_implies_with_comparison() {
    // a == b implies c == d  →  !(a == b) || c == d
    let expr = implies(eq(var("a"), var("b")), eq(var("c"), var("d")));
    assert_eq!(translate(&expr), "!(a == b) || c == d");
}

#[test]
fn translate_complex_fact() {
    // all u: users | u.role == admin implies #u.owns == 0
    let body = implies(
        eq(field(var("u"), "role"), var("admin")),
        eq(card(field(var("u"), "owns")), var("0")),
    );
    let expr = all("u", var("users"), body);
    assert_eq!(
        translate(&expr),
        "users.iter().all(|u| !(u.role == admin) || u.owns.len() == 0)"
    );
}

// ── lone field membership (In operator) ──────────────────────────────────────

#[test]
fn translate_in_with_lone_field_non_selfref() {
    // u.group is lone Group (Option<Group>) → u.group.as_ref() == Some(&u)
    let ir = make_ir_two_sigs("User", "group", Multiplicity::Lone, "Group");
    let expr = Expr::Comparison {
        op: CompareOp::In,
        left:  Box::new(Expr::VarRef("u".into())),
        right: Box::new(Expr::FieldAccess {
            base:  Box::new(Expr::VarRef("u".into())),
            field: "group".into(),
        }),
    };
    let result = translate_with_ir(&expr, &ir);
    assert!(
        result.contains("as_ref()") && result.contains("Some"),
        "lone non-selfref should use as_ref() == Some, got: {result}"
    );
}

#[test]
fn translate_in_with_lone_selfref_field() {
    // s.parent is lone SigDecl (Option<Box<SigDecl>>) → s.parent.as_deref() == Some(&s)
    let ir = make_ir_self_ref("SigDecl", "parent", Multiplicity::Lone);
    let expr = Expr::Comparison {
        op: CompareOp::In,
        left:  Box::new(Expr::VarRef("s".into())),
        right: Box::new(Expr::FieldAccess {
            base:  Box::new(Expr::VarRef("s".into())),
            field: "parent".into(),
        }),
    };
    let result = translate_with_ir(&expr, &ir);
    assert!(
        result.contains("as_deref()") && result.contains("Some"),
        "lone self-ref should use as_deref() == Some, got: {result}"
    );
}

#[test]
fn translate_in_with_set_field_unchanged() {
    // s.fields is set FieldDecl (Vec<FieldDecl>) → s.fields.contains(&f)
    let ir = make_ir_two_sigs("SigDecl", "fields", Multiplicity::Set, "FieldDecl");
    let expr = Expr::Comparison {
        op: CompareOp::In,
        left:  Box::new(Expr::VarRef("f".into())),
        right: Box::new(Expr::FieldAccess {
            base:  Box::new(Expr::VarRef("s".into())),
            field: "fields".into(),
        }),
    };
    let result = translate_with_ir(&expr, &ir);
    assert!(
        result.contains("contains"),
        "set field should still use contains, got: {result}"
    );
}

// ── lone field Eq operator ────────────────────────────────────────────────────

#[test]
fn translate_eq_with_lone_selfref_field() {
    // s.parent = p where parent is lone SigDecl (self-ref) → as_deref() == Some
    let ir = make_ir_self_ref("SigDecl", "parent", Multiplicity::Lone);
    let expr = Expr::Comparison {
        op: CompareOp::Eq,
        left:  Box::new(Expr::FieldAccess {
            base:  Box::new(Expr::VarRef("s".into())),
            field: "parent".into(),
        }),
        right: Box::new(Expr::VarRef("p".into())),
    };
    let result = translate_with_ir(&expr, &ir);
    assert!(
        result.contains("as_deref()") && result.contains("Some"),
        "lone self-ref Eq should use as_deref() == Some, got: {result}"
    );
}

#[test]
fn translate_eq_with_lone_non_selfref_field() {
    // u.group = g where group is lone Group (non self-ref) → as_ref() == Some
    let ir = make_ir_two_sigs("User", "group", Multiplicity::Lone, "Group");
    let expr = Expr::Comparison {
        op: CompareOp::Eq,
        left:  Box::new(Expr::FieldAccess {
            base:  Box::new(Expr::VarRef("u".into())),
            field: "group".into(),
        }),
        right: Box::new(Expr::VarRef("g".into())),
    };
    let result = translate_with_ir(&expr, &ir);
    assert!(
        result.contains("as_ref()") && result.contains("Some"),
        "lone non-selfref Eq should use as_ref() == Some, got: {result}"
    );
}

#[test]
fn translate_eq_one_field_unchanged() {
    // s.role = r where role is one Role → normal ==
    let ir = make_ir_two_sigs("User", "role", Multiplicity::One, "Role");
    let expr = Expr::Comparison {
        op: CompareOp::Eq,
        left:  Box::new(Expr::FieldAccess {
            base:  Box::new(Expr::VarRef("u".into())),
            field: "role".into(),
        }),
        right: Box::new(Expr::VarRef("r".into())),
    };
    let result = translate_with_ir(&expr, &ir);
    assert!(
        result.contains("==") && !result.contains("Some"),
        "one field Eq should use plain ==, got: {result}"
    );
}

// ── Integer literal and comparison operator tests ─────────────────────────────

fn int_lit(n: i64) -> Expr {
    Expr::IntLiteral(n)
}

fn cmp(op: CompareOp, left: Expr, right: Expr) -> Expr {
    Expr::Comparison {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

#[test]
fn translate_int_literal() {
    assert_eq!(translate(&int_lit(42)), "42");
    assert_eq!(translate(&int_lit(-1)), "-1");
    assert_eq!(translate(&int_lit(0)), "0");
}

#[test]
fn translate_lte_comparison() {
    let expr = cmp(CompareOp::Lte, card(field(var("a"), "items")), int_lit(5));
    assert_eq!(translate(&expr), "a.items.len() <= 5");
}

#[test]
fn translate_gte_comparison() {
    let expr = cmp(CompareOp::Gte, field(var("a"), "count"), int_lit(0));
    assert_eq!(translate(&expr), "a.count >= 0");
}

#[test]
fn translate_lt_comparison() {
    let expr = cmp(CompareOp::Lt, field(var("a"), "x"), int_lit(10));
    assert_eq!(translate(&expr), "a.x < 10");
}

#[test]
fn translate_gt_comparison() {
    let expr = cmp(CompareOp::Gt, field(var("a"), "x"), int_lit(1));
    assert_eq!(translate(&expr), "a.x > 1");
}

#[test]
fn translate_cardinality_eq_int() {
    let expr = cmp(CompareOp::Eq, card(field(var("t"), "members")), int_lit(3));
    assert_eq!(translate(&expr), "t.members.len() == 3");
}

#[test]
fn translate_int_comparison_with_ir() {
    let ir = make_ir_two_sigs("Team", "members", Multiplicity::Set, "User");
    let expr = cmp(CompareOp::Lte, card(field(var("t"), "members")), int_lit(5));
    let result = translate_with_ir(&expr, &ir);
    assert_eq!(result, "t.members.len() <= 5");
}

// ── Multi-variable quantifier translation tests ───────────────────────────────

fn all_multi(vars: &[&str], domain: Expr, body: Expr) -> Expr {
    Expr::Quantifier {
        kind: QuantKind::All,
        bindings: vec![QuantBinding {
            vars: vars.iter().map(|v| v.to_string()).collect(),
            domain,
            disj: false,
        }],
        body: Box::new(body),
    }
}

fn all_disj(vars: &[&str], domain: Expr, body: Expr) -> Expr {
    Expr::Quantifier {
        kind: QuantKind::All,
        bindings: vec![QuantBinding {
            vars: vars.iter().map(|v| v.to_string()).collect(),
            domain,
            disj: true,
        }],
        body: Box::new(body),
    }
}

fn all_multi_binding(bindings: Vec<QuantBinding>, body: Expr) -> Expr {
    Expr::Quantifier {
        kind: QuantKind::All,
        bindings,
        body: Box::new(body),
    }
}

#[test]
fn translate_multi_var_same_domain() {
    // all x, y: S | x = y → nested iteration
    let expr = all_multi(&["x", "y"], var("S"), eq(var("x"), var("y")));
    let result = translate(&expr);
    assert!(result.contains("S.iter().all(|x|"));
    assert!(result.contains("S.iter().all(|y|"));
    assert!(result.contains("x == y"));
}

#[test]
fn translate_multi_binding_different_domains() {
    // all x: S, y: T | x = y → nested iteration over S then T
    let expr = all_multi_binding(
        vec![
            QuantBinding { vars: vec!["x".to_string()], domain: var("S"), disj: false },
            QuantBinding { vars: vec!["y".to_string()], domain: var("T"), disj: false },
        ],
        eq(var("x"), var("y")),
    );
    let result = translate(&expr);
    assert!(result.contains("S.iter().all(|x|"));
    assert!(result.contains("T.iter().all(|y|"));
}

#[test]
fn translate_disj_adds_inequality_guard() {
    // all disj x, y: S | body → nested with x != y guard
    let expr = all_disj(&["x", "y"], var("S"), eq(var("x"), var("y")));
    let result = translate(&expr);
    assert!(result.contains("x != y"));
    assert!(result.contains("if x != y"));
    assert!(result.contains("else { true }"));
}

// ── Alloy 6 temporal expression translation ──────────────────────────────────

#[test]
fn translate_prime_on_field_access() {
    // s.x' → next_x (reference to next-state value)
    use oxidtr::parser::ast::Expr;
    let expr = Expr::Prime(Box::new(field(var("s"), "x")));
    let result = translate(&expr);
    assert!(!result.contains("/* next-state */"), "should not use comment placeholder, got: {result}");
    assert!(result.contains("next_"), "should reference next-state field, got: {result}");
}

#[test]
fn translate_always_unwraps_inner() {
    // always (s.x == s.y) → just translate inner expression
    use oxidtr::parser::ast::{Expr, TemporalUnaryOp};
    let inner = eq(field(var("s"), "x"), field(var("s"), "y"));
    let expr = Expr::TemporalUnary {
        op: TemporalUnaryOp::Always,
        expr: Box::new(inner),
    };
    let result = translate(&expr);
    assert!(!result.contains("/* temporal */"), "should not use comment placeholder, got: {result}");
    assert!(result.contains("s.x"), "should contain inner expression, got: {result}");
}

#[test]
fn translate_eventually_unwraps_inner() {
    use oxidtr::parser::ast::{Expr, TemporalUnaryOp};
    let inner = eq(field(var("s"), "x"), field(var("s"), "y"));
    let expr = Expr::TemporalUnary {
        op: TemporalUnaryOp::Eventually,
        expr: Box::new(inner),
    };
    let result = translate(&expr);
    assert!(!result.contains("/* temporal */"), "should not use comment placeholder, got: {result}");
}

#[test]
fn translate_until_translates_both_sides() {
    use oxidtr::parser::ast::{Expr, TemporalBinaryOp};
    let left = eq(field(var("s"), "x"), field(var("s"), "y"));
    let right = eq(field(var("s"), "a"), field(var("s"), "b"));
    let expr = Expr::TemporalBinary {
        op: TemporalBinaryOp::Until,
        left: Box::new(left),
        right: Box::new(right),
    };
    let result = translate(&expr);
    assert!(result.contains("s.x == s.y"), "should contain left side, got: {result}");
    assert!(result.contains("s.a == s.b"), "should contain right side, got: {result}");
    assert!(result.contains("&&"), "should combine with &&, got: {result}");
}

#[test]
fn translate_since_translates_both_sides() {
    use oxidtr::parser::ast::{Expr, TemporalBinaryOp};
    let left = eq(field(var("s"), "x"), field(var("s"), "y"));
    let right = eq(field(var("s"), "a"), field(var("s"), "b"));
    let expr = Expr::TemporalBinary {
        op: TemporalBinaryOp::Since,
        left: Box::new(left),
        right: Box::new(right),
    };
    let result = translate(&expr);
    assert!(result.contains("&&"), "should combine with &&, got: {result}");
}

// ── Alloy 6: function application translation ──────────────────────────────────

#[test]
fn translate_fun_app_bare_call() {
    let expr = Expr::FunApp {
        name: "myFun".to_string(),
        receiver: None,
        args: vec![var("x")],
    };
    let result = translate(&expr);
    assert_eq!(result, "myFun(x)");
}

#[test]
fn translate_fun_app_no_args() {
    let expr = Expr::FunApp {
        name: "noop".to_string(),
        receiver: None,
        args: vec![],
    };
    let result = translate(&expr);
    assert_eq!(result, "noop()");
}

// ── Integer arithmetic: receiver.plus[n] → receiver + n ─────────────────────

#[test]
fn translate_fun_app_plus_with_receiver() {
    let expr = Expr::FunApp {
        name: "plus".to_string(),
        receiver: Some(Box::new(Expr::FieldAccess {
            base: Box::new(var("c")),
            field: "count".to_string(),
        })),
        args: vec![Expr::IntLiteral(1)],
    };
    let result = translate(&expr);
    assert_eq!(result, "c.count + 1");
}

#[test]
fn translate_fun_app_minus_with_receiver() {
    let expr = Expr::FunApp {
        name: "minus".to_string(),
        receiver: Some(Box::new(var("x"))),
        args: vec![Expr::IntLiteral(3)],
    };
    let result = translate(&expr);
    assert_eq!(result, "x - 3");
}

#[test]
fn translate_fun_app_mul_with_receiver() {
    let expr = Expr::FunApp {
        name: "mul".to_string(),
        receiver: Some(Box::new(var("x"))),
        args: vec![var("y")],
    };
    let result = translate(&expr);
    assert_eq!(result, "x * y");
}

#[test]
fn translate_fun_app_non_arithmetic_with_receiver() {
    let expr = Expr::FunApp {
        name: "custom".to_string(),
        receiver: Some(Box::new(var("obj"))),
        args: vec![var("a"), var("b")],
    };
    let result = translate(&expr);
    assert_eq!(result, "obj.custom(a, b)");
}
