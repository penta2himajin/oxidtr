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
        Expr::Quantifier { kind, var, .. } => {
            assert_eq!(*kind, QuantKind::All);
            assert_eq!(var, "a");
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
