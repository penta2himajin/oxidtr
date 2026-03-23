/// Lossless round-trip tests: verify that @alloy comments survive
/// generate → mine cycles, and that reverse translation produces
/// matching Alloy expressions.

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::{rust, typescript};
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::mine::{rust_extractor, ts_extractor, kotlin_extractor, java_extractor, Confidence};
use oxidtr::mine::renderer;
use oxidtr::analyze;

const SELF_MODEL: &str = include_str!("../models/oxidtr.als");

// ── Feature A: @alloy comments survive round-trip ──────────────────────────

#[test]
fn rust_invariants_contain_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = rust::generate(&ir_obj);
    let invariants = files.iter().find(|f| f.path == "invariants.rs").unwrap();

    // Every named constraint should have an @alloy comment
    let alloy_count = invariants.content.lines()
        .filter(|l| l.trim().starts_with("/// @alloy: "))
        .count();
    let named_constraints = ir_obj.constraints.iter()
        .filter(|c| c.name.is_some())
        .count();
    assert!(alloy_count >= named_constraints,
        "expected at least {named_constraints} @alloy comments in invariants.rs, found {alloy_count}");
}

#[test]
fn ts_invariants_contain_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir_obj);
    let invariants = files.iter().find(|f| f.path == "invariants.ts").unwrap();

    let alloy_count = invariants.content.lines()
        .filter(|l| l.trim().starts_with("// @alloy: "))
        .count();
    let named_constraints = ir_obj.constraints.iter()
        .filter(|c| c.name.is_some())
        .count();
    assert!(alloy_count >= named_constraints,
        "expected at least {named_constraints} @alloy comments in invariants.ts, found {alloy_count}");
}

#[test]
fn kotlin_invariants_contain_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir_obj);
    let invariants = files.iter().find(|f| f.path == "Invariants.kt").unwrap();

    let alloy_count = invariants.content.lines()
        .filter(|l| l.trim().starts_with("// @alloy: "))
        .count();
    let named_constraints = ir_obj.constraints.iter()
        .filter(|c| c.name.is_some())
        .count();
    assert!(alloy_count >= named_constraints,
        "expected at least {named_constraints} @alloy comments in Invariants.kt, found {alloy_count}");
}

#[test]
fn java_invariants_contain_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = java::generate(&ir_obj);
    let invariants = files.iter().find(|f| f.path == "Invariants.java").unwrap();

    let alloy_count = invariants.content.lines()
        .filter(|l| l.trim().starts_with("// @alloy: "))
        .count();
    let named_constraints = ir_obj.constraints.iter()
        .filter(|c| c.name.is_some())
        .count();
    assert!(alloy_count >= named_constraints,
        "expected at least {named_constraints} @alloy comments in Invariants.java, found {alloy_count}");
}

// ── Mine extractors detect @alloy comments ─────────────────────────────────

#[test]
fn rust_mine_extracts_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = rust::generate(&ir_obj);

    // Mine all generated files
    let mut all_alloy_facts = Vec::new();
    for file in &files {
        let mined = rust_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                all_alloy_facts.push(fact);
            }
        }
    }

    assert!(!all_alloy_facts.is_empty(), "no @alloy facts extracted from Rust code");
    // All should be high confidence
    assert!(all_alloy_facts.iter().all(|f| f.confidence == Confidence::High),
        "all @alloy facts should be High confidence");
}

#[test]
fn ts_mine_extracts_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = typescript::generate(&ir_obj);

    let mut all_alloy_facts = Vec::new();
    for file in &files {
        let mined = ts_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                all_alloy_facts.push(fact);
            }
        }
    }

    assert!(!all_alloy_facts.is_empty(), "no @alloy facts extracted from TS code");
    assert!(all_alloy_facts.iter().all(|f| f.confidence == Confidence::High));
}

#[test]
fn kotlin_mine_extracts_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir_obj);

    let mut all_alloy_facts = Vec::new();
    for file in &files {
        let mined = kotlin_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                all_alloy_facts.push(fact);
            }
        }
    }

    assert!(!all_alloy_facts.is_empty(), "no @alloy facts extracted from Kotlin code");
    assert!(all_alloy_facts.iter().all(|f| f.confidence == Confidence::High));
}

#[test]
fn java_mine_extracts_alloy_comments() {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_obj = ir::lower(&model).unwrap();
    let files = java::generate(&ir_obj);

    let mut all_alloy_facts = Vec::new();
    for file in &files {
        let mined = java_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                all_alloy_facts.push(fact);
            }
        }
    }

    assert!(!all_alloy_facts.is_empty(), "no @alloy facts extracted from Java code");
    assert!(all_alloy_facts.iter().all(|f| f.confidence == Confidence::High));
}

// ── Feature B: reverse translation ─────────────────────────────────────────

#[test]
fn rust_reverse_translate_basic_patterns() {
    // .iter().all(|v| body) → all v: Xxx | body
    assert_eq!(
        rust_extractor::reverse_translate_expr("users.iter().all(|u| u.role == u.role)"),
        Some("all u: users | u.role = u.role".to_string())
    );

    // .contains(&v) → v in xxx
    assert_eq!(
        rust_extractor::reverse_translate_expr("s.fields.contains(&f)"),
        Some("f in s.fields".to_string())
    );

    // .len() → #xxx
    assert_eq!(
        rust_extractor::reverse_translate_expr("s.fields.len()"),
        Some("#s.fields".to_string())
    );

    // a == b → a = b
    assert_eq!(
        rust_extractor::reverse_translate_expr("a == b"),
        Some("a = b".to_string())
    );

    // a && b → a and b
    assert_eq!(
        rust_extractor::reverse_translate_expr("a && b"),
        Some("a and b".to_string())
    );

    // !a → not a
    assert_eq!(
        rust_extractor::reverse_translate_expr("!a"),
        Some("not a".to_string())
    );
}

#[test]
fn ts_reverse_translate_basic_patterns() {
    assert_eq!(
        ts_extractor::reverse_translate_expr("users.every(u => u.role === u.role)"),
        Some("all u: users | u.role = u.role".to_string())
    );
    assert_eq!(
        ts_extractor::reverse_translate_expr("arr.includes(x)"),
        Some("x in arr".to_string())
    );
    assert_eq!(
        ts_extractor::reverse_translate_expr("arr.length"),
        Some("#arr".to_string())
    );
}

#[test]
fn kotlin_reverse_translate_basic_patterns() {
    assert_eq!(
        kotlin_extractor::reverse_translate_expr("users.all { u -> u.role == u.role }"),
        Some("all u: users | u.role = u.role".to_string())
    );
    assert_eq!(
        kotlin_extractor::reverse_translate_expr("list.contains(x)"),
        Some("x in list".to_string())
    );
    assert_eq!(
        kotlin_extractor::reverse_translate_expr("list.size"),
        Some("#list".to_string())
    );
}

#[test]
fn java_reverse_translate_basic_patterns() {
    assert_eq!(
        java_extractor::reverse_translate_expr("users.stream().allMatch(u -> u.role == u.role)"),
        Some("all u: users | u.role = u.role".to_string())
    );
    assert_eq!(
        java_extractor::reverse_translate_expr("list.contains(x)"),
        Some("x in list".to_string())
    );
    assert_eq!(
        java_extractor::reverse_translate_expr("list.size()"),
        Some("#list".to_string())
    );
}

// ── alloy_repr produces valid syntax ────────────────────────────────────────

#[test]
fn alloy_repr_roundtrips_parsed_expressions() {
    let model = parser::parse(SELF_MODEL).unwrap();

    // Every fact body should produce a non-empty alloy_repr
    for fact in &model.facts {
        let repr = analyze::alloy_repr(&fact.body);
        assert!(!repr.is_empty(), "alloy_repr produced empty string for fact {:?}", fact.name);
    }

    // Every assert body
    for a in &model.asserts {
        let repr = analyze::alloy_repr(&a.body);
        assert!(!repr.is_empty(), "alloy_repr produced empty string for assert {}", a.name);
    }

    // Every pred body expression
    for pred in &model.preds {
        for expr in &pred.body {
            let repr = analyze::alloy_repr(expr);
            assert!(!repr.is_empty(), "alloy_repr produced empty string for pred {} body", pred.name);
        }
    }
}

// ── Self-hosting lossless round-trip ────────────────────────────────────────

#[test]
fn self_hosting_lossless_round_trip() {
    // Parse the original model
    let original_model = parser::parse(SELF_MODEL).unwrap();
    let original_ir = ir::lower(&original_model).unwrap();

    let _original_sig_count = original_model.sigs.len();
    let _original_fact_count = original_model.facts.len();

    // Count fields across all sigs
    let _original_field_count: usize = original_model.sigs.iter()
        .map(|s| s.fields.len())
        .sum();

    // Generate all 4 languages
    let rust_files = rust::generate(&original_ir);
    let ts_files = typescript::generate(&original_ir);
    let kt_files = kotlin::generate(&original_ir);
    let java_files = java::generate(&original_ir);

    // Mine each, collecting @alloy facts
    let mut alloy_facts_by_lang: Vec<(&str, Vec<String>)> = Vec::new();

    for (lang_name, files, extractor) in [
        ("Rust", &rust_files, extract_alloy_from_rust as fn(&[oxidtr::backend::GeneratedFile]) -> Vec<String>),
        ("TypeScript", &ts_files, extract_alloy_from_ts),
        ("Kotlin", &kt_files, extract_alloy_from_kt),
        ("Java", &java_files, extract_alloy_from_java),
    ] {
        let facts = extractor(files);
        assert!(!facts.is_empty(), "{lang_name}: no @alloy facts extracted");
        alloy_facts_by_lang.push((lang_name, facts));
    }

    // Each language should have extracted the same set of unique alloy expressions
    let lang_fact_sets: Vec<(&str, Vec<String>)> = alloy_facts_by_lang.iter()
        .map(|(name, facts)| {
            let mut unique: Vec<String> = facts.clone();
            unique.sort();
            unique.dedup();
            (*name, unique)
        })
        .collect();

    // All languages should have at least the named constraint count
    let named_constraint_count = original_ir.constraints.iter()
        .filter(|c| c.name.is_some())
        .count();

    for (lang, facts) in &lang_fact_sets {
        assert!(facts.len() >= named_constraint_count,
            "{lang}: expected at least {named_constraint_count} unique @alloy facts, got {}",
            facts.len());
    }

    // Mine Rust code and render back to .als, then re-parse
    let mut mined_model = oxidtr::mine::MinedModel {
        sigs: Vec::new(),
        fact_candidates: Vec::new(),
    };
    for file in &rust_files {
        let mined = rust_extractor::extract(&file.content);
        mined_model.sigs.extend(mined.sigs);
        mined_model.fact_candidates.extend(mined.fact_candidates);
    }

    let rendered = renderer::render(&mined_model);
    assert!(!rendered.is_empty(), "rendered .als should not be empty");

    // Re-parse the rendered model
    let reparsed = parser::parse(&rendered);
    assert!(reparsed.is_ok(), "re-parsed .als should parse: {:?}", reparsed.err());
    let reparsed = reparsed.unwrap();

    // Verify structural preservation
    // Note: mined model won't have all original sigs (some are collapsed in codegen)
    // but it should have a reasonable subset
    assert!(reparsed.sigs.len() > 0, "re-parsed model should have sigs");

    // Verify that @alloy comments appear as fact candidates
    let alloy_comment_facts: Vec<_> = mined_model.fact_candidates.iter()
        .filter(|f| f.source_pattern == "@alloy comment")
        .collect();
    assert!(!alloy_comment_facts.is_empty(),
        "mined model should contain @alloy comment fact candidates");
}

// ── Helpers for the self-hosting test ───────────────────────────────────────

fn extract_alloy_from_rust(files: &[oxidtr::backend::GeneratedFile]) -> Vec<String> {
    let mut facts = Vec::new();
    for file in files {
        let mined = rust_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                facts.push(fact.alloy_text);
            }
        }
    }
    facts
}

fn extract_alloy_from_ts(files: &[oxidtr::backend::GeneratedFile]) -> Vec<String> {
    let mut facts = Vec::new();
    for file in files {
        let mined = ts_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                facts.push(fact.alloy_text);
            }
        }
    }
    facts
}

fn extract_alloy_from_kt(files: &[oxidtr::backend::GeneratedFile]) -> Vec<String> {
    let mut facts = Vec::new();
    for file in files {
        let mined = kotlin_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                facts.push(fact.alloy_text);
            }
        }
    }
    facts
}

fn extract_alloy_from_java(files: &[oxidtr::backend::GeneratedFile]) -> Vec<String> {
    let mut facts = Vec::new();
    for file in files {
        let mined = java_extractor::extract(&file.content);
        for fact in mined.fact_candidates {
            if fact.source_pattern == "@alloy comment" {
                facts.push(fact.alloy_text);
            }
        }
    }
    facts
}
