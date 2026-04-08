/// Language-specific benchmark tests for hand-written code extraction.
/// Each test uses a real-world project's domain model as the Alloy spec
/// and a hand-written fixture modeled on that project's actual code patterns.
///
/// Projects:
///   Rust:       mdBook (documentation tool)
///   TypeScript: Hono (edge web framework)
///   Go:         cobra (CLI framework)
///   Java:       Bukkit (Minecraft server API)
///   Kotlin:     Exposed (SQL DSL)
///   Swift:      Vapor (web framework)
///   C#:         Spectre.Console (terminal UI)

use oxidtr::check::{self, CheckConfig, differ::DiffItem};

fn report(name: &str, diffs: &[DiffItem]) -> (usize, usize) {
    let missing: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::MissingStruct { name } = d { Some(name.as_str()) } else { None }
    }).collect();
    let extra: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::ExtraStruct { name } = d { Some(name.as_str()) } else { None }
    }).collect();
    let miss_f: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::MissingField { struct_name, field_name } = d {
            Some(format!("{struct_name}.{field_name}"))
        } else { None }
    }).collect();
    let extra_f: Vec<_> = diffs.iter().filter_map(|d| {
        if let DiffItem::ExtraField { struct_name, field_name } = d {
            Some(format!("{struct_name}.{field_name}"))
        } else { None }
    }).collect();

    let total_sigs = diffs.iter().filter(|d| matches!(d, DiffItem::MissingStruct { .. })).count();

    eprintln!("=== {name} ===");
    if !missing.is_empty() { eprintln!("  Missing sigs:   {:?}", missing); }
    if !extra.is_empty()   { eprintln!("  Extra sigs:     {:?}", extra); }
    if !miss_f.is_empty()  { eprintln!("  Missing fields: {:?}", miss_f); }
    if !extra_f.is_empty() { eprintln!("  Extra fields:   {:?}", extra_f); }
    // Print other diffs
    for d in diffs {
        let counted = matches!(d,
            DiffItem::MissingStruct { .. } | DiffItem::ExtraStruct { .. } |
            DiffItem::MissingField { .. } | DiffItem::ExtraField { .. }
        );
        if !counted { eprintln!("  {d}"); }
    }
    eprintln!("  Total diffs: {}", diffs.len());
    eprintln!();

    (total_sigs, diffs.len())
}

#[test]
fn lang_rust_mdbook() {
    let config = CheckConfig { impl_dir: "tests/fixtures/mdbook/rust".to_string() };
    let result = check::run("models/mdbook.als", &config).unwrap();
    let (_, total) = report("Rust / mdBook", &result.diffs);
    eprintln!("Rust/mdBook: {total} diffs");
}

#[test]
fn lang_ts_hono() {
    let config = CheckConfig { impl_dir: "tests/fixtures/hono/ts".to_string() };
    let result = check::run("models/hono.als", &config).unwrap();
    let (_, total) = report("TypeScript / Hono", &result.diffs);
    eprintln!("TS/Hono: {total} diffs");
}

#[test]
fn lang_go_cobra() {
    let config = CheckConfig { impl_dir: "tests/fixtures/cobra/go".to_string() };
    let result = check::run("models/cobra.als", &config).unwrap();
    let (_, total) = report("Go / cobra", &result.diffs);
    eprintln!("Go/cobra: {total} diffs");
}

#[test]
fn lang_java_bukkit() {
    let config = CheckConfig { impl_dir: "tests/fixtures/bukkit/java".to_string() };
    let result = check::run("models/bukkit.als", &config).unwrap();
    let (_, total) = report("Java / Bukkit", &result.diffs);
    eprintln!("Java/Bukkit: {total} diffs");
}

#[test]
fn lang_kotlin_exposed() {
    let config = CheckConfig { impl_dir: "tests/fixtures/exposed/kotlin".to_string() };
    let result = check::run("models/exposed.als", &config).unwrap();
    let (_, total) = report("Kotlin / Exposed", &result.diffs);
    eprintln!("Kotlin/Exposed: {total} diffs");
}

#[test]
fn lang_swift_vapor() {
    let config = CheckConfig { impl_dir: "tests/fixtures/vapor/swift".to_string() };
    let result = check::run("models/vapor.als", &config).unwrap();
    let (_, total) = report("Swift / Vapor", &result.diffs);
    eprintln!("Swift/Vapor: {total} diffs");
}

#[test]
fn lang_csharp_spectre() {
    let config = CheckConfig { impl_dir: "tests/fixtures/spectre/csharp".to_string() };
    let result = check::run("models/spectre.als", &config).unwrap();
    let (_, total) = report("C# / Spectre.Console", &result.diffs);
    eprintln!("C#/Spectre: {total} diffs");
}

#[test]
fn lang_summary() {
    let benchmarks = [
        ("Rust / mdBook",          "models/mdbook.als",   "tests/fixtures/mdbook/rust"),
        ("TypeScript / Hono",      "models/hono.als",     "tests/fixtures/hono/ts"),
        ("Go / cobra",             "models/cobra.als",    "tests/fixtures/cobra/go"),
        ("Java / Bukkit",          "models/bukkit.als",   "tests/fixtures/bukkit/java"),
        ("Kotlin / Exposed",       "models/exposed.als",  "tests/fixtures/exposed/kotlin"),
        ("Swift / Vapor",          "models/vapor.als",    "tests/fixtures/vapor/swift"),
        ("C# / Spectre.Console",   "models/spectre.als",  "tests/fixtures/spectre/csharp"),
    ];

    eprintln!("\n╔═══════════════════════════════════════════════════════╗");
    eprintln!("║   Language-Specific Extraction Benchmark              ║");
    eprintln!("╠═══════════════════════════════════════════════════════╣");

    let mut results = Vec::new();
    for (name, model, dir) in &benchmarks {
        let config = CheckConfig { impl_dir: dir.to_string() };
        let result = check::run(model, &config).unwrap();
        let metrics = report(name, &result.diffs);
        results.push((name, metrics));
    }

    eprintln!("╠═══════════════════════════════════════════════════════╣");
    eprintln!("║  Project                MissSig  TotalDiff           ║");
    eprintln!("╠═══════════════════════════════════════════════════════╣");
    for (name, (miss, total)) in &results {
        eprintln!("║  {name:<24} {miss:>3}      {total:>3}               ║");
    }
    eprintln!("╚═══════════════════════════════════════════════════════╝\n");
}
