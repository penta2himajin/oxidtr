/// Round-trip fidelity: Alloy → generate → write → extract must give back the
/// model the generator was handed. Each test drives all eight targets through
/// the same path the CLI takes (`extract::run` on the output directory), so a
/// target that loses information shows up as a named failure here instead of
/// a silent difference in the mined `.als`.

use oxidtr::backend::{self, csharp, go, jvm, lean, rust, swift, typescript, GeneratedFile};
use oxidtr::extract::{self, MinedField, MinedModel, MinedSig};
use oxidtr::parser::ast::SigMultiplicity;
use oxidtr::{check, ir, parser};

const TARGETS: [&str; 8] = ["rust", "ts", "kt", "java", "swift", "go", "cs", "lean"];

fn generate(src: &str, target: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(src).expect("model parses");
    let ir = ir::lower(&model).expect("model lowers");
    match target {
        "rust" => rust::generate(&ir),
        "ts" => typescript::generate(&ir),
        "kt" => jvm::kotlin::generate(&ir),
        "java" => jvm::java::generate(&ir),
        "swift" => swift::generate(&ir),
        "go" => go::generate(&ir),
        "cs" => csharp::generate(&ir),
        "lean" => lean::generate(&ir),
        other => panic!("unknown target {other}"),
    }
}

fn write_out(src: &str, target: &str, dir: &std::path::Path) {
    for f in generate(src, target) {
        let path = dir.join(&f.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &f.content).unwrap();
    }
}

/// Generate `src` for `target`, write it out, and extract it back the way
/// `oxidtr extract <dir>` does.
fn round_trip(src: &str, target: &str) -> MinedModel {
    let dir = tempfile::tempdir().unwrap();
    write_out(src, target, dir.path());
    extract::run(dir.path().to_str().unwrap(), None)
        .unwrap_or_else(|e| panic!("[{target}] extract failed: {e}"))
}

/// The diffs `oxidtr check` reports for `src` against its own generated code.
fn check_diffs(src: &str, target: &str) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    write_out(src, target, &out);
    let model = dir.path().join("model.als");
    std::fs::write(&model, src).unwrap();
    let result = check::run(
        model.to_str().unwrap(),
        &check::CheckConfig { impl_dir: out.to_str().unwrap().to_string() },
    )
    .unwrap_or_else(|e| panic!("[{target}] check failed to run: {e:?}"));
    result.diffs.iter().map(|d| d.to_string()).collect()
}

fn sig<'a>(m: &'a MinedModel, target: &str, name: &str) -> &'a MinedSig {
    m.sigs.iter().find(|s| s.name == name).unwrap_or_else(|| {
        let names: Vec<_> = m.sigs.iter().map(|s| s.name.as_str()).collect();
        panic!("[{target}] sig `{name}` not extracted; got {names:?}")
    })
}

fn field<'a>(m: &'a MinedModel, target: &str, sig_name: &str, name: &str) -> &'a MinedField {
    let s = sig(m, target, sig_name);
    s.fields.iter().find(|f| f.name == name).unwrap_or_else(|| {
        let names: Vec<_> = s.fields.iter().map(|f| f.name.as_str()).collect();
        panic!("[{target}] field `{sig_name}.{name}` not extracted; got {names:?}")
    })
}

// ── #155: primitive types come back as the Alloy names ───────────────────────

const PRIMITIVES: &str = "\
sig P {
  s:  one Str,
  i:  one Int,
  b:  one Bool,
  ms: lone Str,
  si: seq Int,
  sb: set Bool
}
";

#[test]
fn primitives_round_trip_as_alloy_names() {
    for target in TARGETS {
        let m = round_trip(PRIMITIVES, target);
        for (name, want) in [("s", "Str"), ("i", "Int"), ("b", "Bool"), ("ms", "Str"), ("si", "Int"), ("sb", "Bool")] {
            let f = field(&m, target, "P", name);
            assert_eq!(f.target, want, "[{target}] P.{name} should extract as `{want}`");
        }
        assert!(
            !m.sigs.iter().any(|s| backend::is_native_type_alias(&s.name)),
            "[{target}] a native alias must not be declared as a placeholder sig: {:?}",
            m.sigs.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }
}

/// `Float` maps to `number` in TypeScript, the same type as `Int`, so the two
/// cannot be told apart there. Every other target keeps them distinct.
#[test]
fn float_round_trips_where_the_target_distinguishes_it() {
    let src = "sig P { f: one Float }";
    for target in TARGETS.iter().filter(|t| **t != "ts") {
        let m = round_trip(src, target);
        assert_eq!(field(&m, target, "P", "f").target, "Float", "[{target}] P.f");
    }
}

/// Hand-written code uses other widths of the same primitive. They mean the
/// same Alloy type, so they read back the same way.
#[test]
fn hand_written_primitive_synonyms_read_as_alloy_names() {
    let rust = extract::rust_extractor::extract(
        "pub struct P {\n    pub a: i32,\n    pub b: u8,\n    pub c: String,\n    pub d: f32,\n}\n",
    );
    let java = extract::java_extractor::extract("public record P(int a, Integer b, String c, Boolean d) {}\n");
    let go = extract::go_extractor::extract("type P struct {\n\tA int\n\tB uint32\n\tC string\n\tD bool\n}\n");
    for (lang, m, want) in [
        ("rust", &rust, [("a", "Int"), ("b", "Int"), ("c", "Str"), ("d", "Float")]),
        ("java", &java, [("a", "Int"), ("b", "Int"), ("c", "Str"), ("d", "Bool")]),
    ] {
        for (name, t) in want {
            assert_eq!(field(m, lang, "P", name).target, t, "[{lang}] P.{name}");
        }
    }
    for (name, t) in [("a", "Int"), ("b", "Int"), ("c", "Str"), ("d", "Bool")] {
        assert_eq!(field(&go, "go", "P", name).target, t, "[go] P.{name}");
    }
}

// ── #156: sig multiplicity and `var` ─────────────────────────────────────────

/// Every sig multiplicity in both of the shapes a backend gives a sig: a case
/// of an enum (with and without data) and a standalone type. A fieldless case
/// reads as `one` by default, since the value it generates is unique.
const MULTIPLICITIES: &str = "\
abstract sig Color {}
one sig Red extends Color {}
one sig Green extends Color {}

abstract sig Token {}
sig Word extends Token {}
lone sig Blank extends Token {}
sig Number extends Token { value: one Int }
one sig Eof extends Token { pos: one Int }

one sig Config { verbose: one Bool }
lone sig Cache { size: one Int }
some sig Worker { busy: one Bool }
sig Plain { n: one Int }
one sig Marker {}

var sig Session { owner: one Plain }
sig Holder {
  var counter: one Int,
  var state:   lone Color,
  fixed:       one Int
}
";

#[test]
fn sig_multiplicity_round_trips() {
    use SigMultiplicity::*;
    let want = [
        ("Red", One), ("Green", One),
        ("Word", Default), ("Blank", Lone), ("Number", Default), ("Eof", One),
        ("Config", One), ("Cache", Lone), ("Worker", Some), ("Plain", Default), ("Marker", One),
        ("Session", Default), ("Holder", Default),
    ];
    for target in TARGETS {
        let m = round_trip(MULTIPLICITIES, target);
        for (name, mult) in &want {
            assert_eq!(&sig(&m, target, name).multiplicity, mult, "[{target}] multiplicity of {name}");
        }
    }
}

#[test]
fn var_sigs_and_fields_round_trip() {
    for target in TARGETS {
        let m = round_trip(MULTIPLICITIES, target);
        assert!(sig(&m, target, "Session").is_var, "[{target}] Session is a var sig");
        assert!(!sig(&m, target, "Holder").is_var, "[{target}] Holder is not a var sig");
        assert!(field(&m, target, "Holder", "counter").is_var, "[{target}] Holder.counter is var");
        assert!(field(&m, target, "Holder", "state").is_var, "[{target}] Holder.state is var");
        assert!(!field(&m, target, "Holder", "fixed").is_var, "[{target}] Holder.fixed is not var");
    }
}

#[test]
fn check_accepts_its_own_output_for_every_multiplicity_and_var() {
    for target in TARGETS {
        let diffs = check_diffs(MULTIPLICITIES, target);
        assert!(diffs.is_empty(), "[{target}] check reported:\n{}", diffs.join("\n"));
    }
}

/// The mined model renders back to Alloy that keeps the multiplicities.
#[test]
fn rendered_alloy_keeps_sig_multiplicity() {
    for target in TARGETS {
        let als = extract::renderer::render(&round_trip(MULTIPLICITIES, target));
        for decl in ["one sig Red extends Color", "lone sig Blank extends Token", "sig Word extends Token",
                     "one sig Config", "lone sig Cache", "some sig Worker", "var sig Session"] {
            assert!(als.lines().any(|l| l.starts_with(decl) && l[decl.len()..].starts_with(' ')),
                "[{target}] rendered Alloy lacks `{decl}`:\n{als}");
        }
    }
}
