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
        for d in diffs {
            let already_counted = matches!(d,
                DiffItem::MissingStruct { .. } | DiffItem::ExtraStruct { .. } |
                DiffItem::MissingField { .. } | DiffItem::ExtraField { .. } |
                DiffItem::MultiplicityMismatch { .. }
            );
            if !already_counted {
                eprintln!("    {d}");
            }
        }
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
//   Rust:       20/20 sigs (100%), 0 diffs — PERFECT
//   TypeScript: 20/20 sigs (100%), 0 diffs — PERFECT
//   Go:         20/20 sigs (100%), 0 diffs — PERFECT
//   Java:       20/20 sigs (100%), 0 diffs — PERFECT

#[test]
fn benchmark_rust_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/rust".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("Rust", &result.diffs);

    // Baseline: 20/20. Tuple variants now extracted (fields positional: _0, _1, ...).
    assert!(found >= 20, "Rust regression: expected >=20 sigs, got {found}");
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

    // Baseline: 20/20. Class hierarchy extraction with private fields + boxed type mapping.
    assert!(found >= 20, "Java regression: expected >=20 sigs, got {found}");
}

#[test]
fn benchmark_kotlin_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/kotlin".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("Kotlin", &result.diffs);

    // Baseline: 20/20 (0 diffs). Sealed class + data class + data object pattern works perfectly.
    assert!(found >= 20, "Kotlin regression: expected >=20 sigs, got {found}");
}

#[test]
fn benchmark_swift_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/swift".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("Swift", &result.diffs);

    // Baseline: 20/20 (0 diffs). Enum + struct + case params pattern works perfectly.
    assert!(found >= 20, "Swift regression: expected >=20 sigs, got {found}");
}

#[test]
fn benchmark_csharp_handwritten() {
    let config = CheckConfig {
        impl_dir: "tests/fixtures/commonmark/csharp".to_string(),
    };
    let result = check::run("models/commonmark.als", &config).unwrap();
    let (found, _, _, _) = report_diffs("C#", &result.diffs);

    // Baseline: 20/20 (0 diffs). Class hierarchy + C# properties + nullable pattern.
    assert!(found >= 20, "C# regression: expected >=20 sigs, got {found}");
}

// ---------------------------------------------------------------------------
// Summary: run all 7 and compare
// ---------------------------------------------------------------------------

#[test]
fn benchmark_summary() {
    let model = "models/commonmark.als";

    let langs = [
        ("Rust", "tests/fixtures/commonmark/rust"),
        ("TypeScript", "tests/fixtures/commonmark/ts"),
        ("Go", "tests/fixtures/commonmark/go"),
        ("Java", "tests/fixtures/commonmark/java"),
        ("Kotlin", "tests/fixtures/commonmark/kotlin"),
        ("Swift", "tests/fixtures/commonmark/swift"),
        ("C#", "tests/fixtures/commonmark/csharp"),
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
