/// Tests for commentless reverse translation: mining generated code WITHOUT @alloy comments
/// and verifying that the reverse translator can reconstruct the original Alloy expressions.

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::{rust, typescript, jvm};
use oxidtr::mine::{rust_extractor, ts_extractor, kotlin_extractor, java_extractor};
use oxidtr::mine::Confidence;

const SELF_MODEL: &str = include_str!("../models/oxidtr.als");

/// Strip all @alloy comment lines from generated code.
fn strip_alloy_comments(code: &str) -> String {
    code.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("// @alloy:")
                && !trimmed.starts_with("/// @alloy:")
                && !trimmed.starts_with("// @alloy: ")
                && !trimmed.starts_with("/// @alloy: ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Helper: generate code for a language and return the invariants file content.
fn generate_invariants_for_lang(lang: &str) -> String {
    let model = parser::parse(SELF_MODEL).unwrap();
    let ir_result = ir::lower(&model).unwrap();

    match lang {
        "rust" => {
            let files = rust::generate(&ir_result);
            files.iter().find(|f| f.path == "invariants.rs")
                .map(|f| f.content.clone())
                .unwrap_or_default()
        }
        "ts" => {
            let files = typescript::generate(&ir_result);
            files.iter().find(|f| f.path == "invariants.ts")
                .map(|f| f.content.clone())
                .unwrap_or_default()
        }
        "kotlin" => {
            let files = jvm::kotlin::generate(&ir_result);
            files.iter().find(|f| f.path == "Invariants.kt")
                .map(|f| f.content.clone())
                .unwrap_or_default()
        }
        "java" => {
            let files = jvm::java::generate(&ir_result);
            files.iter().find(|f| f.path == "Invariants.java")
                .map(|f| f.content.clone())
                .unwrap_or_default()
        }
        _ => panic!("unknown language: {lang}"),
    }
}

// ── Rust ────────────────────────────────────────────────────────────────────

#[test]
fn self_hosting_commentless_round_trip_rust() {
    let invariants_code = generate_invariants_for_lang("rust");
    assert!(!invariants_code.is_empty(), "no invariants.rs generated");

    let stripped = strip_alloy_comments(&invariants_code);

    // Mine the stripped code
    let mined = rust_extractor::extract(&stripped);

    // Collect reverse-translated facts (exclude @alloy comment sourced ones)
    let reverse_facts: Vec<_> = mined.fact_candidates.iter()
        .filter(|f| f.source_pattern.starts_with("reverse-translated fn"))
        .collect();

    assert!(!reverse_facts.is_empty(),
        "no reverse-translated facts from stripped Rust code");

    for fact in &reverse_facts {
        assert_eq!(fact.confidence, Confidence::Medium,
            "expected Medium confidence for reverse-translated fact: {}", fact.alloy_text);
    }

    // Verify that a significant portion of reverse-translated facts are parseable
    let mut parseable_facts = Vec::new();
    for fact in &reverse_facts {
        let als = format!("sig Dummy {{}}\nfact {{ {} }}\n", fact.alloy_text);
        if parser::parse(&als).is_ok() {
            parseable_facts.push(fact.alloy_text.clone());
        }
    }

    assert!(!parseable_facts.is_empty(),
        "no reverse-translated Rust facts were parseable");

    // The canonical NoCyclicParent constraint should be recovered
    assert!(parseable_facts.iter().any(|f| f.contains("s in s.^parent")),
        "NoCyclicParent constraint not recovered. Parseable facts: {:?}", parseable_facts);
}

// ── TypeScript ──────────────────────────────────────────────────────────────

#[test]
fn self_hosting_commentless_round_trip_ts() {
    let invariants_code = generate_invariants_for_lang("ts");
    assert!(!invariants_code.is_empty(), "no invariants.ts generated");

    let stripped = strip_alloy_comments(&invariants_code);
    let mined = ts_extractor::extract(&stripped);

    let reverse_facts: Vec<_> = mined.fact_candidates.iter()
        .filter(|f| f.source_pattern.starts_with("reverse-translated fn"))
        .collect();

    assert!(!reverse_facts.is_empty(),
        "no reverse-translated facts from stripped TS code. \
         All facts: {:?}", mined.fact_candidates.iter()
            .map(|f| format!("[{}] {}", f.source_pattern, f.alloy_text))
            .collect::<Vec<_>>());

    for fact in &reverse_facts {
        assert_eq!(fact.confidence, Confidence::Medium);
    }

    let mut parseable_count = 0;
    let mut parseable_facts = Vec::new();
    for fact in &reverse_facts {
        let als = format!("sig Dummy {{}}\nfact {{ {} }}\n", fact.alloy_text);
        if parser::parse(&als).is_ok() {
            parseable_count += 1;
            parseable_facts.push(fact.alloy_text.clone());
        }
    }

    assert!(parseable_count > 0,
        "no reverse-translated TS facts were parseable. Facts: {:?}",
        reverse_facts.iter().map(|f| &f.alloy_text).collect::<Vec<_>>());

    assert!(parseable_facts.iter().any(|f| f.contains("s in s.^parent")),
        "NoCyclicParent not recovered. Parseable: {:?}", parseable_facts);
}

// ── Kotlin ──────────────────────────────────────────────────────────────────

#[test]
fn self_hosting_commentless_round_trip_kotlin() {
    let invariants_code = generate_invariants_for_lang("kotlin");
    assert!(!invariants_code.is_empty(), "no Invariants.kt generated");

    let stripped = strip_alloy_comments(&invariants_code);
    let mined = kotlin_extractor::extract(&stripped);

    let reverse_facts: Vec<_> = mined.fact_candidates.iter()
        .filter(|f| f.source_pattern.starts_with("reverse-translated fn"))
        .collect();

    assert!(!reverse_facts.is_empty(),
        "no reverse-translated facts from stripped Kotlin code. \
         All facts: {:?}", mined.fact_candidates.iter()
            .map(|f| format!("[{}] {}", f.source_pattern, f.alloy_text))
            .collect::<Vec<_>>());

    for fact in &reverse_facts {
        assert_eq!(fact.confidence, Confidence::Medium);
    }

    let mut parseable_count = 0;
    let mut parseable_facts = Vec::new();
    for fact in &reverse_facts {
        let als = format!("sig Dummy {{}}\nfact {{ {} }}\n", fact.alloy_text);
        if parser::parse(&als).is_ok() {
            parseable_count += 1;
            parseable_facts.push(fact.alloy_text.clone());
        }
    }

    assert!(parseable_count > 0,
        "no reverse-translated Kotlin facts were parseable. Facts: {:?}",
        reverse_facts.iter().map(|f| &f.alloy_text).collect::<Vec<_>>());

    assert!(parseable_facts.iter().any(|f| f.contains("s in s.^parent") || f.contains("sn in sn.^irParent")),
        "Acyclicity constraint not recovered. Parseable: {:?}", parseable_facts);
}

// ── Java ────────────────────────────────────────────────────────────────────

#[test]
fn self_hosting_commentless_round_trip_java() {
    let invariants_code = generate_invariants_for_lang("java");
    assert!(!invariants_code.is_empty(), "no Invariants.java generated");

    let stripped = strip_alloy_comments(&invariants_code);
    let mined = java_extractor::extract(&stripped);

    let reverse_facts: Vec<_> = mined.fact_candidates.iter()
        .filter(|f| f.source_pattern.starts_with("reverse-translated fn"))
        .collect();

    assert!(!reverse_facts.is_empty(),
        "no reverse-translated facts from stripped Java code. \
         All facts: {:?}", mined.fact_candidates.iter()
            .map(|f| format!("[{}] {}", f.source_pattern, f.alloy_text))
            .collect::<Vec<_>>());

    for fact in &reverse_facts {
        assert_eq!(fact.confidence, Confidence::Medium);
    }

    let mut parseable_count = 0;
    let mut parseable_facts = Vec::new();
    for fact in &reverse_facts {
        let als = format!("sig Dummy {{}}\nfact {{ {} }}\n", fact.alloy_text);
        if parser::parse(&als).is_ok() {
            parseable_count += 1;
            parseable_facts.push(fact.alloy_text.clone());
        }
    }

    assert!(parseable_count > 0,
        "no reverse-translated Java facts were parseable. Facts: {:?}",
        reverse_facts.iter().map(|f| &f.alloy_text).collect::<Vec<_>>());

    assert!(parseable_facts.iter().any(|f| f.contains("s in s.^parent") || f.contains("sn in sn.^irParent")),
        "Acyclicity constraint not recovered. Parseable: {:?}", parseable_facts);
}

// ── Unit tests for robust patterns ──────────────────────────────────────────

#[test]
fn rust_reverse_translate_tc_call() {
    assert_eq!(
        rust_extractor::reverse_translate_expr("tc_parent(&s)"),
        Some("s.^parent".to_string())
    );
}

#[test]
fn rust_reverse_translate_clone_block() {
    assert_eq!(
        rust_extractor::reverse_translate_expr("{ let s = s.clone(); tc_parent(&s).contains(&s) }"),
        Some("s in s.^parent".to_string())
    );
}

#[test]
fn rust_reverse_translate_nested_negated_any() {
    let input = "!sig_decls.iter().any(|s| { let s = s.clone(); tc_parent(&s).contains(&s) })";
    let result = rust_extractor::reverse_translate_expr(input);
    assert!(result.is_some(), "failed to reverse-translate: {input}");
    let alloy = result.unwrap();
    assert!(alloy.starts_with("no s: sig_decls |"), "unexpected result: {alloy}");
}

#[test]
fn ts_reverse_translate_tc_call() {
    assert_eq!(
        ts_extractor::reverse_translate_expr("tcParent(s)"),
        Some("s.^parent".to_string())
    );
}

#[test]
fn ts_reverse_translate_includes_with_tc() {
    assert_eq!(
        ts_extractor::reverse_translate_expr("tcParent(s).includes(s)"),
        Some("s in s.^parent".to_string())
    );
}

#[test]
fn kotlin_reverse_translate_tc_call() {
    assert_eq!(
        kotlin_extractor::reverse_translate_expr("tcParent(s)"),
        Some("s.^parent".to_string())
    );
}

#[test]
fn kotlin_reverse_translate_contains_with_tc() {
    assert_eq!(
        kotlin_extractor::reverse_translate_expr("tcParent(s).contains(s)"),
        Some("s in s.^parent".to_string())
    );
}

#[test]
fn java_reverse_translate_tc_call() {
    assert_eq!(
        java_extractor::reverse_translate_expr("tcParent(s)"),
        Some("s.^parent".to_string())
    );
}

#[test]
fn java_reverse_translate_stream_nonematch() {
    let input = "sigDecls.stream().noneMatch(s -> tcParent(s).contains(s))";
    let result = java_extractor::reverse_translate_expr(input);
    assert!(result.is_some(), "failed to reverse-translate: {input}");
    let alloy = result.unwrap();
    assert!(alloy.starts_with("no s: sigDecls |"), "unexpected result: {alloy}");
    assert!(alloy.contains("s in s.^parent"), "unexpected body in: {alloy}");
}

#[test]
fn rust_reverse_translate_invariant_body_with_domain_conversion() {
    // Simulate an invariant function body (what reverse_translate_invariant_body processes)
    let body = "!sig_decls.iter().any(|s| { let s = s.clone(); tc_parent(&s).contains(&s) })";
    let result = rust_extractor::reverse_translate_expr(body);
    assert!(result.is_some(), "failed: {body}");
    // The raw reverse should keep sig_decls as-is
    let alloy = result.unwrap();
    assert!(alloy.contains("sig_decls"), "expected raw domain name: {alloy}");
}

#[test]
fn ts_reverse_translate_balanced_parens() {
    // Nested parens in .some/.every calls
    let input = "sigDecls.every((s) => tcParent(s).includes(s))";
    let result = ts_extractor::reverse_translate_expr(input);
    assert!(result.is_some(), "failed: {input}");
}

#[test]
fn java_reverse_translate_accessor_stripping() {
    // Java record accessors: s.parent() → s.parent
    assert_eq!(
        java_extractor::reverse_translate_expr("s.parent()"),
        Some("s.parent".to_string())
    );
}
