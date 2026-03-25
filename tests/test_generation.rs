use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;
use oxidtr::backend::typescript;

fn generate_from(input: &str) -> Vec<oxidtr::backend::GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    rust::generate(&ir)
}

fn generate_ts_from(input: &str) -> Vec<oxidtr::backend::GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    typescript::generate(&ir)
}

fn find_file<'a>(files: &'a [oxidtr::backend::GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found in {:?}", files.iter().map(|f| &f.path).collect::<Vec<_>>()))
}

#[test]
fn generate_inlined_constraint_in_tests() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact UserHasRole { all u: User | u.role = u.role }
    "#);
    // No invariants.rs should exist
    assert!(!files.iter().any(|f| f.path == "invariants.rs"),
        "should NOT generate invariants.rs");
    let content = find_file(&files, "tests.rs");
    // Tests should contain inlined translated expression
    assert!(content.contains(".iter().all("), "missing translated quantifier in tests");
}

#[test]
fn generate_inlined_constraint_with_implies() {
    let files = generate_from(r#"
        sig User { role: one Role, owns: set Resource }
        sig Role {}
        sig Resource {}
        fact AdminOwnsNothing {
            all u: User | u.role = u.role implies #u.owns = #u.owns
        }
    "#);
    // No invariants.rs
    assert!(!files.iter().any(|f| f.path == "invariants.rs"),
        "should NOT generate invariants.rs");
    let content = find_file(&files, "tests.rs");
    assert!(content.contains(".len()"), "missing cardinality translation in tests");
}

#[test]
fn generate_property_test_from_assert() {
    let files = generate_from(r#"
        sig A {}
        assert NoSelfRef { all a: A | a = a }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("fn no_self_ref") || content.contains("fn prop_no_self_ref"));
    assert!(content.contains(".iter().all("), "missing translated expression in test");
}

#[test]
fn generate_operation_pre_post_conditions() {
    let files = generate_from(r#"
        sig Account { balance: one Account }
        pred withdraw[a: one Account, amount: one Account] {
            a.balance = a.balance
        }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("fn withdraw"));
    // Operations still have todo!() bodies — humans/AI fill these
    assert!(content.contains("todo!"));
}

#[test]
fn generate_cross_test_fact_times_operation() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact UserHasRole { all u: User | u.role = u.role }
        pred changeRole[u: one User, r: one Role] { u.role = r }
    "#);
    let content = find_file(&files, "tests.rs");
    // Should have a cross-test that verifies fact preservation after operation
    assert!(
        content.contains("user_has_role") && content.contains("change_role"),
        "missing cross-test for fact×operation"
    );
}

// ── ⑤ Liveness trace checker generation ─────────────────────────────────────

#[test]
fn generate_liveness_trace_checker() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WillConverge { eventually all s: S | s.x = s.x }
    "#);
    let content = find_file(&files, "tests.rs");
    // Should have liveness test
    assert!(content.contains("fn liveness_"), "missing liveness test:\n{content}");
    // Should have trace checker function
    assert!(content.contains("fn check_liveness_"), "missing liveness trace checker:\n{content}");
    assert!(content.contains(".any("), "trace checker should use .any() for liveness:\n{content}");
}

#[test]
fn generate_past_liveness_trace_checker() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WasReached { once all s: S | s.x = s.x }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("fn check_past_liveness_"), "missing past_liveness trace checker:\n{content}");
}

// ── ④ Binary temporal trace checker generation ──────────────────────────────

#[test]
fn generate_until_trace_checker() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let content = find_file(&files, "tests.rs");
    // Should have temporal test
    assert!(content.contains("fn temporal_"), "missing temporal binary test:\n{content}");
    // Should have trace checker with until semantics
    assert!(content.contains("fn check_until_"), "missing until trace checker:\n{content}");
    assert!(content.contains(".position("), "until checker should use .position():\n{content}");
}

#[test]
fn generate_since_trace_checker() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact HeldSince { (all s: S | s.x = s.x) since (all s: S | s.x = s.x) }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("fn check_since_"), "missing since trace checker:\n{content}");
    assert!(content.contains(".rposition("), "since checker should use .rposition():\n{content}");
}

// ── ④a Binary temporal static test should be comment-only ────────────────────

#[test]
fn binary_temporal_static_test_does_not_assert_body() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let content = find_file(&files, "tests.rs");
    // The static test should exist (for check diff purposes)
    assert!(content.contains("fn temporal_"), "missing temporal test:\n{content}");
    // But it should NOT assert the body — binary temporal requires trace-based verification
    // The test body should trivially pass, not contain a meaningless snapshot assertion
    assert!(
        !content.contains("assert!(s.iter()"),
        "binary temporal static test should NOT assert body inline:\n{content}"
    );
    // Should contain a comment about trace-based verification
    assert!(
        content.contains("binary temporal: requires trace-based verification"),
        "binary temporal static test should document the limitation:\n{content}"
    );
}

#[test]
fn binary_temporal_since_static_test_does_not_assert_body() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact HeldSince { (all s: S | s.x = s.x) since (all s: S | s.x = s.x) }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("fn temporal_"), "missing temporal test:\n{content}");
    assert!(
        content.contains("binary temporal: requires trace-based verification"),
        "since static test should document the limitation:\n{content}"
    );
}

// ── ④b Liveness static test should reference trace checker, not assert body ──

#[test]
fn liveness_static_test_references_trace_checker_rust() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WillConverge { eventually all s: S | s.x = s.x }
    "#);
    let content = find_file(&files, "tests.rs");
    // Liveness test should exist
    assert!(content.contains("fn liveness_"), "missing liveness test:\n{content}");
    // But should NOT assert body — liveness cannot be verified with single snapshot
    assert!(
        !content.contains("assert!(s.iter()"),
        "liveness static test should NOT assert body inline:\n{content}"
    );
    // Should reference trace checker
    assert!(
        content.contains("liveness: requires trace-based verification; see check_liveness_"),
        "liveness static test should reference trace checker:\n{content}"
    );
}

#[test]
fn past_liveness_static_test_references_trace_checker_rust() {
    let files = generate_from(r#"
        sig S { x: one S }
        fact WasReached { once all s: S | s.x = s.x }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("fn past_liveness_"), "missing past_liveness test:\n{content}");
    assert!(
        content.contains("past_liveness: requires trace-based verification; see check_past_liveness_"),
        "past_liveness static test should reference trace checker:\n{content}"
    );
}

// ── ④c TS: liveness/binary temporal test names and trace checker references ──

#[test]
fn ts_liveness_static_test_references_trace_checker() {
    let files = generate_ts_from(r#"
        sig S { x: one S }
        fact WillConverge { eventually all s: S | s.x = s.x }
    "#);
    let content = find_file(&files, "tests.ts");
    // TS should generate `it('liveness WillConverge', ...)`
    assert!(content.contains("it('liveness WillConverge'"), "missing liveness test with correct name:\n{content}");
    // Should reference trace checker, not assert body
    assert!(
        content.contains("liveness: requires trace-based verification; see check_liveness_"),
        "liveness static test should reference trace checker:\n{content}"
    );
}

#[test]
fn ts_binary_temporal_test_uses_temporal_prefix() {
    let files = generate_ts_from(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let content = find_file(&files, "tests.ts");
    // TS should generate `it('temporal WaitUntilDone', ...)` not `it('invariant WaitUntilDone', ...)`
    assert!(content.contains("it('temporal WaitUntilDone'"), "missing temporal binary test with correct name:\n{content}");
    assert!(
        content.contains("binary temporal: requires trace-based verification; see check_until_"),
        "binary temporal should reference trace checker:\n{content}"
    );
}

// ── ③ Integer arithmetic translation ────────────────────────────────────────

#[test]
fn generate_arithmetic_plus_with_receiver() {
    let files = generate_from(r#"
        sig Counter { count: one Int }
        fact Increment { all c: Counter | c.count.plus[1] = c.count }
    "#);
    let content = find_file(&files, "tests.rs");
    // Should translate plus[1] with receiver to arithmetic: count + 1
    assert!(content.contains("+ 1"), "plus[1] should translate to arithmetic + 1:\n{content}");
}
