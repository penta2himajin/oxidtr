/// Benchmark tests for hand-written code extraction accuracy.
/// Uses CommonMark AST model with fixtures from real OSS patterns
/// (pulldown-cmark, mdast, goldmark, commonmark-java).

use oxidtr::check::{self, CheckConfig, differ::DiffItem};

/// Expected sigs in commonmark.als model (20 types total, including abstract).
const EXPECTED_SIGS: &[&str] = &[
    "Document",
    // abstract + concrete block sigs
    "Block", "Heading", "Paragraph", "BlockQuote", "CodeBlock",
    "HtmlBlock", "ThematicBreak", "ListBlock", "ListItem",
    // abstract + concrete inline sigs
    "Inline", "Text", "CodeSpan", "Emphasis", "Strong", "Link", "Image",
    "HtmlInline", "SoftBreak", "LineBreak",
];

/// Count diffs by category and print a summary report.
fn report_diffs(lang: &str, diffs: &[DiffItem]) -> (usize, usize, usize, usize) {
    let missing_structs: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::MissingStruct { name } = d { Some(name.as_str()) } else { None }
    }).collect();
    let extra_structs: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::ExtraStruct { name } = d { Some(name.as_str()) } else { None }
    }).collect();
    let missing_fields: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::MissingField { struct_name, field_name } = d {
            Some(format!("{struct_name}.{field_name}"))
        } else { None }
    }).collect();
    let extra_fields: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::ExtraField { struct_name, field_name } = d {
            Some(format!("{struct_name}.{field_name}"))
        } else { None }
    }).collect();
    let mult_mismatches: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::MultiplicityMismatch { struct_name, field_name, expected, actual } = d {
            Some(format!("{struct_name}.{field_name}: expected {expected:?}, got {actual:?}"))
        } else { None }
    }).collect();
    let other_count = diffs.len()
        - missing_structs.len() - extra_structs.len()
        - missing_fields.len() - extra_fields.len()
        - mult_mismatches.len();

    let found = EXPECTED_SIGS.len().saturating_sub(missing_structs.len());

    eprintln!("=== {lang} benchmark ===");
    eprintln!("  Sigs found:    {found}/{} ({:.0}%)",
        EXPECTED_SIGS.len(),
        found as f64 / EXPECTED_SIGS.len() as f64 * 100.0);
    if !missing_structs.is_empty() {
        eprintln!("  Missing sigs:  {:?}", missing_structs);
    }
    if !extra_structs.is_empty() {
        eprintln!("  Extra sigs:    {:?}", extra_structs);
    }
    if !missing_fields.is_empty() {
        eprintln!("  Missing fields: {:?}", missing_fields);
    }
    if !extra_fields.is_empty() {
        eprintln!("  Extra fields:  {:?}", extra_fields);
    }
    if !mult_mismatches.is_empty() {
        eprintln!("  Mult mismatch: {:?}", mult_mismatches);
    }
    if other_count > 0 {
        eprintln!("  Other diffs:   {other_count}");
    }
    eprintln!("  Total diffs:   {}", diffs.len());
    eprintln!();

    (found, missing_fields.len(), extra_fields.len(), diffs.len())
}

// ---------------------------------------------------------------------------
// Per-language benchmarks: run check and report precision/recall
// ---------------------------------------------------------------------------

// Baseline values as of latest measurement.
// Improve extractors → update these thresholds upward.
//
// Current baselines:
//   Rust:       15/20 sigs (75%), 20 diffs — tuple variants unextracted
//   TypeScript: 20/20 sigs (100%), 29 diffs — sig detection perfect, remaining: ops/validation
//   Go:         20/20 sigs (100%),  7 diffs — sig detection perfect, remaining: ops/validation
//   Java:        0/20 sigs  (0%), 27 diffs — Java class hierarchy completely unextracted

#[test]
fn benchmark_rust_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/rust".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("Rust", &result.diffs);

    // Baseline: 15/20. Tuple variants (Text, CodeSpan, Emphasis, Strong, HtmlInline) are lost.
    assert!(found >= 15, "Rust regression: expected >=15 sigs, got {found}");
}

#[test]
fn benchmark_ts_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/ts".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("TypeScript", &result.diffs);

    // Baseline: 20/20. Sig detection is perfect after multi-line union + discriminant fixes.
    assert!(found >= 20, "TypeScript regression: expected >=20 sigs, got {found}");
}

#[test]
fn benchmark_go_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/go".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("Go", &result.diffs);

    // Baseline: 20/20. Sig detection is perfect after empty interface + field parsing fixes.
    assert!(found >= 20, "Go regression: expected >=20 sigs, got {found}");
}

#[test]
fn benchmark_java_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/java".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("Java", &result.diffs);

    // Baseline: 0/20. Java class hierarchy completely unextracted via mine fallback.
    // This is the primary target for improvement.
    eprintln!("Java baseline: {found}/20 sigs found");
}

// ---------------------------------------------------------------------------
// Summary: run all 4 and compare
// ---------------------------------------------------------------------------

#[test]
fn benchmark_summary() {
    let model = "models/commonmark.als";

    let langs = [
        ("Rust", "tests/fixtures/commonmark/rust"),
        ("TypeScript", "tests/fixtures/commonmark/ts"),
        ("Go", "tests/fixtures/commonmark/go"),
        ("Java", "tests/fixtures/commonmark/java"),
    ];

    eprintln!("\n╔══════════════════════════════════════════════════╗");
    eprintln!("║   CommonMark AST Extraction Benchmark            ║");
    eprintln!("╠══════════════════════════════════════════════════╣");

    let mut all_results = Vec::new();
    for (lang, dir) in &langs {
        let config = CheckConfig { impl_dir: dir.to_string() };
        let result = check::run(model, &config).unwrap();
        let metrics = report_diffs(lang, &result.diffs);
        all_results.push((lang, metrics));
    }

    eprintln!("╠══════════════════════════════════════════════════╣");
    eprintln!("║  Lang        Sigs   MissFld ExtraFld  TotalDiff ║");
    eprintln!("╠══════════════════════════════════════════════════╣");
    for (lang, (found, miss_f, extra_f, total)) in &all_results {
        eprintln!("║  {lang:<12} {found:>2}/{:<2}   {miss_f:>3}      {extra_f:>3}      {total:>3}     ║",
            EXPECTED_SIGS.len());
    }
    eprintln!("╚══════════════════════════════════════════════════╝\n");
}
