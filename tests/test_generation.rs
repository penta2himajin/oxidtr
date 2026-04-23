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
fn one_sig_quantifier_simplified_to_direct_binding() {
    let files = generate_from(r#"
        one sig Config { max_retries: one Int }
        fact ConfigValid { all c: Config | c.max_retries = c.max_retries }
    "#);
    let content = find_file(&files, "tests.rs");
    // For `one sig`, should use direct binding instead of .iter().all()
    assert!(
        !content.contains(".iter().all("),
        "one sig quantifier should NOT use .iter().all():\n{content}"
    );
    assert!(
        content.contains("[0].clone()"),
        "one sig quantifier should use direct [0].clone() binding:\n{content}"
    );
}

#[test]
fn coverage_test_warns_vacuously_true() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        sig Order { owner: one User }
        fact UserHasRole { all u: User | u.role = u.role }
        fact OrderOwnership { all o: Order | o.owner = o.owner }
    "#);
    let content = find_file(&files, "tests.rs");
    // Order has no fixture → coverage test involving OrderOwnership should warn vacuously true
    if content.contains("cover_") {
        // If there is a coverage test that involves Order (which has no fixture),
        // it should have a vacuous truth warning
        let has_order_coverage = content.contains("order_ownership");
        if has_order_coverage {
            assert!(
                content.contains("vacuously true"),
                "coverage test with empty domain should warn about vacuous truth:\n{content}"
            );
        }
    }
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

// ── ④d Trace checker variable scope: quantifier vars properly bound ─────────

#[test]
fn ts_trace_checker_binds_quantifier_variable_explicitly() {
    let files = generate_ts_from(r#"
        sig State { x: one Int }
        fact WillConverge { eventually all s: State | s.x > 10 }
    "#);
    let content = find_file(&files, "tests.ts");
    // Trace checker should use direct iteration without spread
    assert!(
        content.contains("states.every(s =>") || content.contains("states.every((s) =>"),
        "trace checker should iterate with quantifier variable bound to trace element:\n{content}"
    );
    // Should NOT have [...states] spread — trace elements are already arrays
    assert!(
        !content.contains("[...states]"),
        "trace checker should not spread trace elements (already arrays):\n{content}"
    );
}

#[test]
fn rust_trace_checker_binds_quantifier_variable_explicitly() {
    let files = generate_from(r#"
        sig Counter { value: one Int }
        fact WillConverge { eventually all c: Counter | c.value > 0 }
    "#);
    let content = find_file(&files, "tests.rs");
    // Should use counters.iter().all(|c| ...) directly, not nested quantifier expansion
    assert!(
        content.contains("counters.iter().all(|c|"),
        "trace checker should iterate trace element with quantifier variable:\n{content}"
    );
}

#[test]
fn ts_binary_trace_checker_binds_quantifier_variable() {
    let files = generate_ts_from(r#"
        sig S { x: one Int }
        fact WaitDone { (all s: S | s.x > 0) until (all s: S | s.x > 10) }
    "#);
    let content = find_file(&files, "tests.ts");
    // Binary temporal trace checker should iterate with quantifier variable
    assert!(
        content.contains("ss.every(s =>") || content.contains("ss.every((s) =>"),
        "binary trace checker should bind quantifier variable via iteration:\n{content}"
    );
    assert!(
        !content.contains("[...ss]"),
        "binary trace checker should not spread trace elements:\n{content}"
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

// ── ④ Predicate call + tests import ────────────────────────────────────────

/// BUG: tests.rs was missing `use super::operations::*;` so predicate
/// calls inside translated assert bodies failed to resolve (E0425).
#[test]
fn tests_rs_imports_operations() {
    let files = generate_from(r#"
        sig Instr { op: one OpCode }
        sig OpCode {}
        pred is_memory[i: one Instr] { i.op = i.op }
        assert Dummy { all i: Instr | is_memory[i] }
    "#);
    let tests_rs = find_file(&files, "tests.rs");
    assert!(
        tests_rs.contains("use super::operations::*;"),
        "tests.rs must import operations::*; to resolve predicate calls:\n{tests_rs}"
    );
}

/// BUG: predicates were emitted as `fn f(x: &T)` with no return type, so
/// tests calling them in boolean contexts (`!(is_memory(i) && ...)`)
/// failed with E0308 "expected bool, found ()".
#[test]
fn predicate_returns_bool() {
    let files = generate_from(r#"
        sig Instr { op: one OpCode }
        sig OpCode {}
        pred is_memory[i: one Instr] { i.op = i.op }
    "#);
    let ops = find_file(&files, "operations.rs");
    // Predicate (no explicit return type) must lower to `-> bool`.
    assert!(
        ops.contains("fn is_memory(") && ops.contains(") -> bool"),
        "pred should lower to `fn is_memory(...) -> bool`:\n{ops}"
    );
}

/// BUG: `is_X(i)` was emitted with owned `i` while the signature took
/// `&T`, producing E0308 type mismatches. Call sites targeting a known
/// predicate/function must pass args by reference.
#[test]
fn predicate_call_passes_args_by_reference() {
    let files = generate_from(r#"
        sig Instr { op: one OpCode }
        sig OpCode {}
        pred is_memory[i: one Instr] { i.op = i.op }
        assert Dummy { all i: Instr | is_memory[i] }
    "#);
    let tests_rs = find_file(&files, "tests.rs");
    assert!(
        tests_rs.contains("is_memory(&i)"),
        "pred call in tests.rs should pass `&i`, not owned `i`:\n{tests_rs}"
    );
}

// ── ⑤ Validator implication precedence ──────────────────────────────────────

/// BUG: `(A or B) implies C` in a fact lowered to `A || B && !C` in the
/// newtype validator, which Rust parses as `A || (B && !C)` — the wrong
/// semantics. The condition must be parenthesized.
#[test]
fn validator_implication_disjunctive_antecedent_is_parenthesized() {
    let files = generate_from(r#"
        sig S { a: one Color, b: one Color }
        sig Color {}
        fact F {
            all s: S |
                (s.a = s.a or s.a = s.b) implies s.b = s.b
        }
    "#);
    let newtypes = find_file(&files, "newtypes.rs");
    // The implication must render as `if (A || B) && !(C)` not `if A || B && !(C)`.
    // We conservatively require that somewhere in newtypes.rs the sequence
    // `if (` appears followed by `||` on the same conditional line.
    let has_parenthesized_or = newtypes
        .lines()
        .any(|l| l.trim_start().starts_with("if (") && l.contains("||") && l.contains(") && !("));
    assert!(
        has_parenthesized_or,
        "validator must parenthesize disjunctive antecedent of implies:\n{newtypes}"
    );
}
