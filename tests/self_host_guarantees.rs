/// Self-hosting guarantee tests for oxidtr.
///
/// These tests verify that oxidtr's own Alloy model (oxidtr.als) is correctly
/// round-tripped through the generate pipeline, and that the generated code
/// satisfies key properties.

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;
use oxidtr::backend::typescript;
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::backend::swift;
use oxidtr::backend::go;
use oxidtr::check;
// NOTE: Tests that require external toolchains (cargo test, bun test, gradle test)
// have been moved to target_validation.rs.

const SELF_MODEL: &str = include_str!("../models/oxidtr.als");

fn parse_and_lower() -> ir::nodes::OxidtrIR {
    let model = parser::parse(SELF_MODEL).expect("parse oxidtr.als");
    ir::lower(&model).expect("lower oxidtr.als")
}

/// Collect named facts from the IR.
fn named_facts(ir: &ir::nodes::OxidtrIR) -> Vec<String> {
    ir.constraints.iter()
        .filter_map(|c| c.name.clone())
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Guarantee 1: Facts correctly converted to validation functions
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn guarantee_1_rust_tests_cover_named_facts() {
    let ir = parse_and_lower();
    let files = rust::generate(&ir);
    let tests = files.iter().find(|f| f.path == "tests.rs")
        .expect("tests.rs should be generated");

    let facts = named_facts(&ir);
    assert!(!facts.is_empty(), "model should have named facts");

    // Each named fact should appear in the test file (as invariant_, boundary_, or invalid_)
    for fact in &facts {
        let snake = to_snake_case(fact);
        let has_invariant = tests.content.contains(&format!("fn invariant_{snake}"))
            || tests.content.contains(&format!("Type-guaranteed: {fact}"));
        let has_boundary = tests.content.contains(&format!("fn boundary_{snake}"));
        let has_cross = tests.content.contains(&format!("{snake}_preserved_after_"));
        assert!(
            has_invariant || has_boundary || has_cross,
            "fact {fact} should appear in tests.rs (looked for invariant_{snake}, boundary_{snake}, or {snake}_preserved_after_)"
        );
    }
}

#[test]
fn guarantee_1_rust_fixtures_pass_invariants() {
    let ir = parse_and_lower();
    let files = rust::generate(&ir);
    let tests = files.iter().find(|f| f.path == "tests.rs")
        .expect("tests.rs should be generated");
    let fixtures = files.iter().find(|f| f.path == "fixtures.rs")
        .expect("fixtures.rs should be generated");

    // Verify fixtures exist for concrete sigs with fields
    assert!(fixtures.content.contains("fn default_"), "fixtures should have default factories");

    // Tests should reference fixtures (non-empty collections)
    assert!(
        tests.content.contains("vec![default_"),
        "tests should use fixture-backed collections, not empty vecs"
    );
}

#[test]
fn guarantee_1_ts_validators_check_constraints() {
    let ir = parse_and_lower();
    let validators = typescript::generate_validators(&ir);

    // TS validators should exist for constrained sigs
    if !validators.is_empty() {
        assert!(validators.contains("export function validate"), "validators should have validate functions");
        assert!(validators.contains("errors"), "validators should collect errors");
    }

    // Tests should also cover named facts
    let files = typescript::generate(&ir);
    let tests = files.iter().find(|f| f.path == "tests.ts")
        .expect("tests.ts should be generated");

    let facts = named_facts(&ir);
    for fact in &facts {
        let has_invariant = tests.content.contains(&format!("invariant {fact}"));
        let has_boundary = tests.content.contains(&format!("boundary {fact}"));
        let has_cross = tests.content.contains(&format!("{fact} preserved after"));
        assert!(
            has_invariant || has_boundary || has_cross,
            "fact {fact} should appear in tests.ts"
        );
    }
}

#[test]
fn guarantee_1_kotlin_tests_cover_named_facts() {
    let ir = parse_and_lower();
    let files = kotlin::generate(&ir);
    let tests = files.iter().find(|f| f.path == "Tests.kt")
        .expect("Tests.kt should be generated");

    let facts = named_facts(&ir);
    for fact in &facts {
        let has_invariant = tests.content.contains(&format!("invariant {fact}"))
            || tests.content.contains(&format!("Type-guaranteed: {fact}"));
        let has_boundary = tests.content.contains(&format!("boundary {fact}"));
        let has_cross = tests.content.contains(&format!("{fact} preserved after"));
        assert!(
            has_invariant || has_boundary || has_cross,
            "fact {fact} should appear in Tests.kt"
        );
    }
}

#[test]
fn guarantee_1_java_tests_cover_named_facts() {
    let ir = parse_and_lower();
    let files = java::generate(&ir);
    let tests = files.iter().find(|f| f.path == "Tests.java")
        .expect("Tests.java should be generated");

    let facts = named_facts(&ir);
    for fact in &facts {
        let has_invariant = tests.content.contains(&format!("invariant_{fact}"));
        let has_boundary = tests.content.contains(&format!("boundary_{fact}"));
        assert!(
            has_invariant || has_boundary,
            "fact {fact} should appear in Tests.java"
        );
    }
}

#[test]
fn guarantee_1_swift_tests_cover_named_facts() {
    let ir = parse_and_lower();
    let files = swift::generate(&ir);
    let tests = files.iter().find(|f| f.path == "Tests.swift")
        .expect("Tests.swift should be generated");

    let facts = named_facts(&ir);
    for fact in &facts {
        let has_invariant = tests.content.contains(&format!("test_invariant_{fact}"))
            || tests.content.contains(&format!("Type-guaranteed: {fact}"));
        let has_boundary = tests.content.contains(&format!("test_boundary_{fact}"));
        let has_cross = tests.content.contains(&format!("{fact}_preserved_after_"));
        assert!(
            has_invariant || has_boundary || has_cross,
            "fact {fact} should appear in Tests.swift"
        );
    }
}

#[test]
fn guarantee_1_go_tests_cover_named_facts() {
    let ir = parse_and_lower();
    let files = go::generate(&ir);
    let tests = files.iter().find(|f| f.path == "models_test.go")
        .expect("models_test.go should be generated");

    let facts = named_facts(&ir);
    for fact in &facts {
        let has_invariant = tests.content.contains(&format!("Test_invariant_{fact}"))
            || tests.content.contains(&format!("Type-guaranteed: {fact}"));
        let has_boundary = tests.content.contains(&format!("Test_boundary_{fact}"));
        let has_cross = tests.content.contains(&format!("{fact}_preserved_after_"));
        assert!(
            has_invariant || has_boundary || has_cross,
            "fact {fact} should appear in models_test.go"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Guarantee 2: Facts correctly converted to TryFrom (Rust newtypes)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn guarantee_2_tryfrom_contains_inlined_constraint() {
    let ir = parse_and_lower();
    let files = rust::generate(&ir);
    let newtypes = files.iter().find(|f| f.path == "newtypes.rs");

    // If newtypes exist, each should have TryFrom with error messages referencing fact names
    if let Some(nt) = newtypes {
        assert!(nt.content.contains("impl TryFrom<"), "newtypes should have TryFrom impls");
        assert!(nt.content.contains("type Error"), "TryFrom should declare Error type");

        // Each TryFrom should reference a fact name in its error message
        let facts = named_facts(&ir);
        let has_any_fact_reference = facts.iter().any(|f| nt.content.contains(f));
        assert!(
            has_any_fact_reference,
            "TryFrom error messages should reference fact names"
        );
    }
    // If no newtypes generated, that's OK — it means no comparisons in facts
}

#[test]
fn guarantee_2_newtypes_use_inlined_expressions() {
    let ir = parse_and_lower();
    let files = rust::generate(&ir);
    let newtypes = files.iter().find(|f| f.path == "newtypes.rs");

    if let Some(nt) = newtypes {
        // Newtypes should NOT reference invariants module
        assert!(
            !nt.content.contains("use crate::invariants::"),
            "newtypes should NOT import invariants module"
        );
        // Should contain inlined expressions (iter/all/any patterns)
        assert!(
            nt.content.contains(".iter()") || nt.content.contains("if true"),
            "newtypes should contain inlined expressions or simple passthrough"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Guarantee 3: Cross-tests are properly marked as ignored/skipped
// (Actual test execution moved to target_validation.rs)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn guarantee_3_rust_cross_tests_are_ignored() {
    let ir = parse_and_lower();
    let files = rust::generate(&ir);
    let tests = files.iter().find(|f| f.path == "tests.rs")
        .expect("tests.rs should be generated");

    // Cross-tests should have #[ignore] attribute
    if tests.content.contains("Cross-tests") {
        // Find all cross-test functions
        let lines: Vec<&str> = tests.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("_preserved_after_") && line.contains("fn ") {
                // The line before should be #[test] and before that #[ignore]
                let has_ignore = (i >= 2 && lines[i - 2].trim().contains("#[ignore]"))
                    || (i >= 1 && lines[i - 1].trim().contains("#[ignore]"));
                assert!(
                    has_ignore,
                    "cross-test at line {} should be marked #[ignore]: {}",
                    i + 1, line.trim()
                );
            }
        }
    }
}

#[test]
fn guarantee_3_ts_cross_tests_are_skipped() {
    let ir = parse_and_lower();
    let files = typescript::generate(&ir);
    let tests = files.iter().find(|f| f.path == "tests.ts")
        .expect("tests.ts should be generated");

    // Cross-tests should use it.skip()
    if tests.content.contains("Cross-tests") {
        assert!(
            tests.content.contains("it.skip("),
            "TS cross-tests should use it.skip()"
        );
        // Should NOT have non-skipped cross-tests that throw
        // (all cross-tests with "preserved after" should be in it.skip)
        let lines: Vec<&str> = tests.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("preserved after") && line.contains("it(") {
                // This is a non-skipped cross-test — should not exist
                panic!(
                    "cross-test at line {} should use it.skip(), not it(): {}",
                    i + 1, line.trim()
                );
            }
        }
    }
}

#[test]
fn guarantee_3_kotlin_cross_tests_are_disabled() {
    let ir = parse_and_lower();
    let files = kotlin::generate(&ir);
    let tests = files.iter().find(|f| f.path == "Tests.kt")
        .expect("Tests.kt should be generated");

    if tests.content.contains("Cross-tests") {
        let lines: Vec<&str> = tests.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("preserved after") && line.contains("fun ") {
                // Should have @Disabled before the @Test
                let has_disabled = (0..i).rev().take(4).any(|j| lines[j].trim().contains("@Disabled"));
                assert!(
                    has_disabled,
                    "Kotlin cross-test at line {} should be marked @Disabled: {}",
                    i + 1, line.trim()
                );
            }
        }
    }
}

#[test]
fn guarantee_3_java_cross_tests_are_disabled() {
    let ir = parse_and_lower();
    let files = java::generate(&ir);
    let tests = files.iter().find(|f| f.path == "Tests.java")
        .expect("Tests.java should be generated");

    if tests.content.contains("Cross-tests") {
        let lines: Vec<&str> = tests.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("preserved_after_") && line.contains("void ") {
                let has_disabled = (0..i).rev().take(4).any(|j| lines[j].trim().contains("@Disabled"));
                assert!(
                    has_disabled,
                    "Java cross-test at line {} should be marked @Disabled: {}",
                    i + 1, line.trim()
                );
            }
        }
    }
}

#[test]
fn guarantee_3_swift_cross_tests_are_disabled() {
    let ir = parse_and_lower();
    let files = swift::generate(&ir);
    let tests = files.iter().find(|f| f.path == "Tests.swift")
        .expect("Tests.swift should be generated");

    if tests.content.contains("Cross-tests") {
        let lines: Vec<&str> = tests.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("preserved_after_") && line.contains("func ") {
                assert!(
                    line.contains("disabled_test_"),
                    "Swift cross-test at line {} should use disabled_test_ prefix: {}",
                    i + 1, line.trim()
                );
            }
        }
    }
}

#[test]
fn guarantee_3_go_cross_tests_are_disabled() {
    let ir = parse_and_lower();
    let files = go::generate(&ir);
    let tests = files.iter().find(|f| f.path == "models_test.go")
        .expect("models_test.go should be generated");

    if tests.content.contains("Cross-tests") {
        let lines: Vec<&str> = tests.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("preserved_after_") && line.contains("func ") {
                assert!(
                    line.contains("disabled_Test_"),
                    "Go cross-test at line {} should use disabled_Test_ prefix: {}",
                    i + 1, line.trim()
                );
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Guarantee 4: Mine results match original model semantically
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn guarantee_4_mine_covers_all_sigs_and_fields() {
    let ir = parse_and_lower();

    // For each language, generate code, mine it back, and compare
    let languages: Vec<(&str, Box<dyn Fn(&ir::nodes::OxidtrIR) -> Vec<oxidtr::backend::GeneratedFile>>)> = vec![
        ("rust", Box::new(|ir| rust::generate(ir))),
        ("ts", Box::new(|ir| typescript::generate(ir))),
        ("kt", Box::new(|ir| kotlin::generate(ir))),
        ("java", Box::new(|ir| java::generate(ir))),
        ("swift", Box::new(|ir| swift::generate(ir))),
        ("go", Box::new(|ir| go::generate(ir))),
    ];

    for (lang, generate_fn) in &languages {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let files = generate_fn(&ir);
        for file in &files {
            std::fs::write(dir.join(&file.path), &file.content).unwrap();
        }

        // Also write validators.ts for TS
        if *lang == "ts" {
            let validators = typescript::generate_validators(&ir);
            if !validators.is_empty() {
                std::fs::write(dir.join("validators.ts"), &validators).unwrap();
            }
        }

        // Run check
        let config = check::CheckConfig {
            impl_dir: dir.to_str().unwrap().to_string(),
        };
        let result = check::run("models/oxidtr.als", &config)
            .unwrap_or_else(|e| panic!("check failed for {lang}: {e}"));

        // Filter out expected diffs (ExtraStruct from fixture artifacts etc.)
        let real_issues: Vec<_> = result.diffs.iter()
            .filter(|d| !matches!(d, check::differ::DiffItem::ExtraStruct { .. }))
            .filter(|d| !matches!(d, check::differ::DiffItem::ExtraFn { .. }))
            .collect();

        // No missing structs or fields
        let missing_structs: Vec<_> = real_issues.iter()
            .filter(|d| matches!(d, check::differ::DiffItem::MissingStruct { .. }))
            .collect();
        let missing_fields: Vec<_> = real_issues.iter()
            .filter(|d| matches!(d, check::differ::DiffItem::MissingField { .. }))
            .collect();
        let mult_mismatches: Vec<_> = real_issues.iter()
            .filter(|d| matches!(d, check::differ::DiffItem::MultiplicityMismatch { .. }))
            .collect();

        assert!(
            missing_structs.is_empty(),
            "[{lang}] missing structs: {:?}", missing_structs
        );
        assert!(
            missing_fields.is_empty(),
            "[{lang}] missing fields: {:?}", missing_fields
        );
        assert!(
            mult_mismatches.is_empty(),
            "[{lang}] multiplicity mismatches: {:?}", mult_mismatches
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Guarantee 5: Check detects constraint divergence
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn guarantee_5_check_detects_missing_validation() {
    let ir = parse_and_lower();

    // Generate Rust code
    let files = rust::generate(&ir);

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    for file in &files {
        std::fs::write(dir.join(&file.path), &file.content).unwrap();
    }

    // Verify the model has named facts
    let facts = named_facts(&ir);
    assert!(!facts.is_empty(), "model should have named facts");

    // The check command currently does structural diff (sigs, fields, fns).
    // Guarantee 5 extends it to also detect missing validations.
    // For now, verify the structural check passes on unmodified code.
    let config = check::CheckConfig {
        impl_dir: dir.to_str().unwrap().to_string(),
    };
    let result = check::run("models/oxidtr.als", &config)
        .expect("check should succeed");

    // Structural check should pass (no missing structs/fields on unmodified code)
    let structural_issues: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d,
            check::differ::DiffItem::MissingStruct { .. }
            | check::differ::DiffItem::MissingField { .. }
            | check::differ::DiffItem::MissingFn { .. }
        ))
        .collect();

    assert!(
        structural_issues.is_empty(),
        "unmodified generated code should have no structural issues: {:?}", structural_issues
    );

    // Verify MissingValidation detection works via the new diff items
    let validation_issues: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d,
            check::differ::DiffItem::MissingValidation { .. }
            | check::differ::DiffItem::ExtraValidation { .. }
        ))
        .collect();

    // On unmodified code, there should be no missing validations
    assert!(
        validation_issues.is_empty(),
        "unmodified generated code should have no missing validations: {:?}", validation_issues
    );
}

#[test]
fn guarantee_5_check_detects_removed_validation() {
    let ir = parse_and_lower();
    let files = rust::generate(&ir);

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    for file in &files {
        std::fs::write(dir.join(&file.path), &file.content).unwrap();
    }

    // Remove the tests.rs file entirely — this simulates losing all validations
    let tests_path = dir.join("tests.rs");
    if tests_path.exists() {
        std::fs::remove_file(&tests_path).unwrap();
    }

    // Also remove newtypes.rs if present
    let newtypes_path = dir.join("newtypes.rs");
    if newtypes_path.exists() {
        std::fs::remove_file(&newtypes_path).unwrap();
    }

    // Check should detect missing validations for each named fact
    let config = check::CheckConfig {
        impl_dir: dir.to_str().unwrap().to_string(),
    };
    let result = check::run("models/oxidtr.als", &config)
        .expect("check should succeed");

    let _facts = named_facts(&ir);
    let missing_validations: Vec<_> = result.diffs.iter()
        .filter(|d| matches!(d, check::differ::DiffItem::MissingValidation { .. }))
        .collect();

    // Should detect at least some missing validations
    assert!(
        !missing_validations.is_empty(),
        "removing test files should trigger MissingValidation diffs; got diffs: {:?}", result.diffs
    );
}

// ── helpers ────────────────────────────────────────────────────────────────────

fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}
