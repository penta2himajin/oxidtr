use oxidtr::parser::{self, ast::*};

#[test]
fn parse_simple_fact() {
    let input = r#"
        sig A {}
        fact SomeFact { all a: A | a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.facts.len(), 1);
    assert_eq!(model.facts[0].name.as_deref(), Some("SomeFact"));
    match &model.facts[0].body {
        Expr::Quantifier { kind, bindings, .. } => {
            assert_eq!(*kind, QuantKind::All);
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].vars, vec!["a".to_string()]);
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_anonymous_fact() {
    let input = r#"
        sig A {}
        fact { all a: A | a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert!(model.facts[0].name.is_none());
}

#[test]
fn parse_assert_decl() {
    let input = r#"
        sig Node { next: lone Node }
        assert NoSelfLoop { all n: Node | not n.next = n }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.asserts.len(), 1);
    assert_eq!(model.asserts[0].name, "NoSelfLoop");
}

#[test]
fn parse_pred_no_params() {
    let input = r#"
        sig A {}
        pred myPred { all a: A | a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.preds.len(), 1);
    assert_eq!(model.preds[0].name, "myPred");
    assert!(model.preds[0].params.is_empty());
}

#[test]
fn parse_pred_with_params() {
    let input = r#"
        sig User {}
        sig Role {}
        pred assign[u: one User, r: one Role] { u = u }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.preds[0].params.len(), 2);
    assert_eq!(model.preds[0].params[0].name, "u");
    assert_eq!(model.preds[0].params[0].mult, Multiplicity::One);
    assert_eq!(model.preds[0].params[0].type_name, "User");
}

#[test]
fn parse_field_access_expr() {
    let input = r#"
        sig User { role: one Role }
        sig Role { perms: set Perm }
        sig Perm {}
        fact { all u: User | u.role.perms = u.role.perms }
    "#;
    let model = parser::parse(input).expect("should parse");
    // verify it parsed without error — deep structure check would be verbose
    assert_eq!(model.facts.len(), 1);
}

#[test]
fn parse_not_expr() {
    let input = r#"
        sig A {}
        fact { all a: A | not a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            assert!(matches!(body.as_ref(), Expr::Not(_)));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_cardinality_expr() {
    let input = r#"
        sig A { items: set B }
        sig B {}
        fact { all a: A | #a.items = #a.items }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.facts.len(), 1);
}

#[test]
fn parse_implies_expr() {
    let input = r#"
        sig A { flag: one B }
        sig B {}
        fact { all a: A | a.flag = a.flag implies a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            assert!(matches!(body.as_ref(), Expr::BinaryLogic { op: LogicOp::Implies, .. }));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_and_expr() {
    let input = r#"
        sig A {}
        fact { all a: A | a = a and a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            assert!(matches!(body.as_ref(), Expr::BinaryLogic { op: LogicOp::And, .. }));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_in_expr() {
    let input = r#"
        sig A { items: set B }
        sig B {}
        fact { all a: A | a in a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            assert!(matches!(body.as_ref(), Expr::Comparison { op: CompareOp::In, .. }));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_check_and_run_skipped() {
    let input = r#"
        sig A {}
        assert Foo { all a: A | a = a }
        check Foo for 5
        run {} for 3
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs.len(), 1);
    assert_eq!(model.asserts.len(), 1);
    // check and run are silently skipped
}

#[test]
fn parse_comments_ignored() {
    let input = r#"
        -- this is a comment
        sig A {} // another comment
        /* block comment */
        sig B {}
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.sigs.len(), 2);
}

// ── Integer literal and comparison operator tests ─────────────────────────────

#[test]
fn parse_int_literal_in_cardinality_bound() {
    let input = r#"
        sig Team { members: set User }
        sig User {}
        fact { all t: Team | #t.members <= 5 }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { op: CompareOp::Lte, left, right } => {
                    assert!(matches!(left.as_ref(), Expr::Cardinality(_)));
                    assert_eq!(*right.as_ref(), Expr::IntLiteral(5));
                }
                other => panic!("expected Comparison(Lte), got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_gte_comparison() {
    let input = r#"
        sig Item { count: one Item }
        fact { all i: Item | i.count >= 0 }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { op: CompareOp::Gte, right, .. } => {
                    assert_eq!(*right.as_ref(), Expr::IntLiteral(0));
                }
                other => panic!("expected Comparison(Gte), got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_eq_with_int_literal() {
    let input = r#"
        sig A { items: set B }
        sig B {}
        fact { all a: A | #a.items = 3 }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { op: CompareOp::Eq, right, .. } => {
                    assert_eq!(*right.as_ref(), Expr::IntLiteral(3));
                }
                other => panic!("expected Comparison(Eq), got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_lt_comparison() {
    let input = r#"
        sig A { x: one A }
        fact { all a: A | a.x < 10 }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            assert!(matches!(body.as_ref(), Expr::Comparison { op: CompareOp::Lt, .. }));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_gt_comparison() {
    let input = r#"
        sig A { x: one A }
        fact { all a: A | a.x > 1 }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            assert!(matches!(body.as_ref(), Expr::Comparison { op: CompareOp::Gt, .. }));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_negative_int_literal() {
    let input = r#"
        sig A { x: one A }
        fact { all a: A | a.x >= -1 }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { op: CompareOp::Gte, right, .. } => {
                    assert_eq!(*right.as_ref(), Expr::IntLiteral(-1));
                }
                other => panic!("expected Comparison(Gte), got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

// ── Set operation tests ───────────────────────────────────────────────────────

#[test]
fn parse_set_union() {
    let input = r#"
        sig A {}
        sig B {}
        fact { all a: A | a + a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    assert!(matches!(left.as_ref(), Expr::SetOp { op: SetOpKind::Union, .. }));
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_set_intersection() {
    let input = r#"
        sig A { f: set B }
        sig B { g: set A }
        fact { all a: A | a.f & a.f = a.f }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    assert!(matches!(left.as_ref(), Expr::SetOp { op: SetOpKind::Intersection, .. }));
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_set_difference() {
    let input = r#"
        sig A {}
        sig B {}
        fact { all a: A | a - a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    assert!(matches!(left.as_ref(), Expr::SetOp { op: SetOpKind::Difference, .. }));
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

// ── Product (arrow) tests ─────────────────────────────────────────────────────

#[test]
fn parse_product() {
    let input = r#"
        sig A {}
        sig B {}
        sig R { rel: set A }
        fact { all a: A | a -> a in a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { op: CompareOp::In, left, .. } => {
                    assert!(matches!(left.as_ref(), Expr::Product { .. }));
                }
                other => panic!("expected Comparison(In), got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

// ── Fun declaration tests ─────────────────────────────────────────────────────

#[test]
fn parse_fun_decl() {
    let input = r#"
        sig User { role: one Role }
        sig Role {}
        fun getRole[u: one User]: one Role { u.role }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.funs.len(), 1);
    assert_eq!(model.funs[0].name, "getRole");
    assert_eq!(model.funs[0].params.len(), 1);
    assert_eq!(model.funs[0].params[0].name, "u");
    assert_eq!(model.funs[0].params[0].mult, Multiplicity::One);
    assert_eq!(model.funs[0].params[0].type_name, "User");
    assert_eq!(model.funs[0].return_mult, Multiplicity::One);
    assert_eq!(model.funs[0].return_type, "Role");
    assert!(matches!(&model.funs[0].body, Expr::FieldAccess { .. }));
}

#[test]
fn parse_fun_no_params() {
    let input = r#"
        sig A {}
        fun allAs: set A { A }
    "#;
    let model = parser::parse(input).expect("should parse");
    assert_eq!(model.funs.len(), 1);
    assert_eq!(model.funs[0].name, "allAs");
    assert!(model.funs[0].params.is_empty());
    assert_eq!(model.funs[0].return_mult, Multiplicity::Set);
    assert_eq!(model.funs[0].return_type, "A");
}

#[test]
fn parse_fun_lowers_to_operation() {
    let input = r#"
        sig User { role: one Role }
        sig Role {}
        fun getRole[u: one User]: one Role { u.role }
    "#;
    let model = parser::parse(input).expect("should parse");
    let ir = oxidtr::ir::lower(&model).expect("should lower");
    // fun should be lowered as an operation with return type
    assert!(ir.operations.iter().any(|op| op.name == "getRole" && op.return_type.is_some()));
}

// ── Multi-variable quantifier and disj tests ──────────────────────────────────

#[test]
fn parse_multi_var_same_domain() {
    let input = r#"
        sig S {}
        fact { all x, y: S | x = y }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { kind, bindings, .. } => {
            assert_eq!(*kind, QuantKind::All);
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].vars, vec!["x".to_string(), "y".to_string()]);
            assert!(!bindings[0].disj);
            assert!(matches!(&bindings[0].domain, Expr::VarRef(name) if name == "S"));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_multi_binding_different_domains() {
    let input = r#"
        sig S {}
        sig T {}
        fact { all x: S, y: T | x = y }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { kind, bindings, .. } => {
            assert_eq!(*kind, QuantKind::All);
            assert_eq!(bindings.len(), 2);
            assert_eq!(bindings[0].vars, vec!["x".to_string()]);
            assert!(matches!(&bindings[0].domain, Expr::VarRef(name) if name == "S"));
            assert_eq!(bindings[1].vars, vec!["y".to_string()]);
            assert!(matches!(&bindings[1].domain, Expr::VarRef(name) if name == "T"));
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_disj_quantifier() {
    let input = r#"
        sig S {}
        fact { all disj x, y: S | x != y }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { kind, bindings, .. } => {
            assert_eq!(*kind, QuantKind::All);
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].vars, vec!["x".to_string(), "y".to_string()]);
            assert!(bindings[0].disj);
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_some_disj_quantifier() {
    let input = r#"
        sig S {}
        fact { some disj x, y: S | x != y }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { kind, bindings, .. } => {
            assert_eq!(*kind, QuantKind::Some);
            assert_eq!(bindings.len(), 1);
            assert!(bindings[0].disj);
            assert_eq!(bindings[0].vars.len(), 2);
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_single_var_backwards_compatible() {
    let input = r#"
        sig A {}
        fact { all a: A | a = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { bindings, .. } => {
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].vars.len(), 1);
            assert_eq!(bindings[0].vars[0], "a");
            assert!(!bindings[0].disj);
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_multi_binding_with_disj() {
    let input = r#"
        sig S {}
        sig T {}
        fact { all disj x, y: S, z: T | x = z }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { bindings, .. } => {
            assert_eq!(bindings.len(), 2);
            assert!(bindings[0].disj);
            assert_eq!(bindings[0].vars, vec!["x".to_string(), "y".to_string()]);
            assert!(!bindings[1].disj);
            assert_eq!(bindings[1].vars, vec!["z".to_string()]);
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

// ── Alloy 6: prime operator tests ──────────────────────────────────────────────

#[test]
fn parse_prime_expr() {
    let input = r#"
        sig S { var x: set S }
        fact { all s: S | s.x' = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { op: CompareOp::Eq, left, .. } => {
                    assert!(matches!(left.as_ref(), Expr::Prime(_)),
                        "expected Prime, got {:?}", left);
                }
                other => panic!("expected Comparison(Eq), got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_prime_on_var_ref() {
    let input = r#"
        sig S { var x: one S }
        fact { all s: S | x' = x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    assert!(matches!(left.as_ref(), Expr::Prime(_)),
                        "expected Prime, got {:?}", left);
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_prime_in_field_access_chain() {
    // s.connections' — prime on the final field access
    let input = r#"
        sig S { var connections: set S }
        fact { all s: S | s.connections' = s.connections }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    assert!(matches!(left.as_ref(), Expr::Prime(_)),
                        "expected Prime, got {:?}", left);
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

// ── Alloy 6: temporal unary operator tests ─────────────────────────────────────

#[test]
fn parse_always_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { always all s: S | s.x = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op, .. } => {
            assert_eq!(*op, TemporalUnaryOp::Always);
        }
        other => panic!("expected TemporalUnary(Always), got {other:?}"),
    }
}

#[test]
fn parse_eventually_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { eventually all s: S | s.x = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op, .. } => {
            assert_eq!(*op, TemporalUnaryOp::Eventually);
        }
        other => panic!("expected TemporalUnary(Eventually), got {other:?}"),
    }
}

#[test]
fn parse_after_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { after all s: S | s.x = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op, .. } => {
            assert_eq!(*op, TemporalUnaryOp::After);
        }
        other => panic!("expected TemporalUnary(After), got {other:?}"),
    }
}

#[test]
fn parse_historically_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { historically all s: S | s.x = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op, .. } => {
            assert_eq!(*op, TemporalUnaryOp::Historically);
        }
        other => panic!("expected TemporalUnary(Historically), got {other:?}"),
    }
}

#[test]
fn parse_once_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { once all s: S | s.x = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op, .. } => {
            assert_eq!(*op, TemporalUnaryOp::Once);
        }
        other => panic!("expected TemporalUnary(Once), got {other:?}"),
    }
}

#[test]
fn parse_before_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { before all s: S | s.x = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op, .. } => {
            assert_eq!(*op, TemporalUnaryOp::Before);
        }
        other => panic!("expected TemporalUnary(Before), got {other:?}"),
    }
}

#[test]
fn parse_nested_temporal() {
    let input = r#"
        sig S { var x: one S }
        fact { always eventually all s: S | s.x = s.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op: TemporalUnaryOp::Always, expr } => {
            assert!(matches!(expr.as_ref(), Expr::TemporalUnary { op: TemporalUnaryOp::Eventually, .. }));
        }
        other => panic!("expected Always(Eventually(...)), got {other:?}"),
    }
}

// ── Alloy 6: temporal binary operator tests ─────────────────────────────────────

#[test]
fn parse_until_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalBinary { op, .. } => {
            assert_eq!(*op, TemporalBinaryOp::Until);
        }
        other => panic!("expected TemporalBinary(Until), got {other:?}"),
    }
}

#[test]
fn parse_since_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { (all s: S | s.x = s.x) since (all s: S | s.x = s.x) }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalBinary { op, .. } => {
            assert_eq!(*op, TemporalBinaryOp::Since);
        }
        other => panic!("expected TemporalBinary(Since), got {other:?}"),
    }
}

#[test]
fn parse_release_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { (all s: S | s.x = s.x) release (all s: S | s.x = s.x) }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalBinary { op, .. } => {
            assert_eq!(*op, TemporalBinaryOp::Release);
        }
        other => panic!("expected TemporalBinary(Release), got {other:?}"),
    }
}

#[test]
fn parse_triggered_formula() {
    let input = r#"
        sig S { var x: one S }
        fact { (all s: S | s.x = s.x) triggered (all s: S | s.x = s.x) }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalBinary { op, .. } => {
            assert_eq!(*op, TemporalBinaryOp::Triggered);
        }
        other => panic!("expected TemporalBinary(Triggered), got {other:?}"),
    }
}

#[test]
fn parse_nested_always_until() {
    let input = r#"
        sig S { var x: one S }
        fact { always ((all s: S | s.x = s.x) until (all s: S | s.x = s.x)) }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::TemporalUnary { op: TemporalUnaryOp::Always, expr } => {
            assert!(matches!(expr.as_ref(), Expr::TemporalBinary { op: TemporalBinaryOp::Until, .. }));
        }
        other => panic!("expected Always(Until(...)), got {other:?}"),
    }
}

// ── Alloy 6: function application tests ─────────────────────────────────────

#[test]
fn parse_fun_app_simple() {
    // c.count.plus[1] — function application on field access chain
    let input = r#"
        sig Counter { count: one Int }
        fact Increment { all c: Counter | c.count.plus[1] = c.count }
    "#;
    let model = parser::parse(input).expect("should parse");
    // The fact body should contain a FunApp
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    match left.as_ref() {
                        Expr::FunApp { name, receiver, args } => {
                            assert_eq!(name, "plus");
                            assert!(receiver.is_some(), "method-style FunApp should have receiver");
                            assert_eq!(args.len(), 1);
                            assert!(matches!(&args[0], Expr::IntLiteral(1)));
                        }
                        other => panic!("expected FunApp, got {other:?}"),
                    }
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_fun_app_multiple_args() {
    let input = r#"
        sig A { x: one Int, y: one Int }
        fact Multi { all a: A | a.x.add[a.y, 1] = a.x }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    match left.as_ref() {
                        Expr::FunApp { name, receiver, args } => {
                            assert_eq!(name, "add");
                            assert!(receiver.is_some(), "method-style FunApp should have receiver");
                            assert_eq!(args.len(), 2);
                        }
                        other => panic!("expected FunApp, got {other:?}"),
                    }
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}

#[test]
fn parse_bare_fun_app() {
    // f[x] — bare identifier function application
    let input = r#"
        sig A {}
        sig B {}
        fun myFun[a: one A] : one B { a }
        fact UseFun { all a: A | myFun[a] = a }
    "#;
    let model = parser::parse(input).expect("should parse");
    match &model.facts[0].body {
        Expr::Quantifier { body, .. } => {
            match body.as_ref() {
                Expr::Comparison { left, .. } => {
                    match left.as_ref() {
                        Expr::FunApp { name, receiver, args } => {
                            assert_eq!(name, "myFun");
                            assert!(receiver.is_none(), "bare FunApp should have no receiver");
                            assert_eq!(args.len(), 1);
                        }
                        other => panic!("expected FunApp, got {other:?}"),
                    }
                }
                other => panic!("expected Comparison, got {other:?}"),
            }
        }
        other => panic!("expected Quantifier, got {other:?}"),
    }
}
