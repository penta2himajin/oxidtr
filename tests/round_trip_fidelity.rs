/// Round-trip fidelity: Alloy → generate → write → extract must give back the
/// model the generator was handed. Each test drives all eight targets through
/// the same path the CLI takes (`extract::run` on the output directory), so a
/// target that loses information shows up as a named failure here instead of
/// a silent difference in the mined `.als`.

use oxidtr::backend::{self, csharp, go, jvm, lean, rust, swift, typescript, GeneratedFile};
use oxidtr::extract::{self, MinedField, MinedModel, MinedSig};
use oxidtr::{ir, parser};

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

/// Generate `src` for `target`, write it out, and extract it back the way
/// `oxidtr extract <dir>` does.
fn round_trip(src: &str, target: &str) -> MinedModel {
    let dir = tempfile::tempdir().unwrap();
    for f in generate(src, target) {
        let path = dir.path().join(&f.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &f.content).unwrap();
    }
    extract::run(dir.path().to_str().unwrap(), None)
        .unwrap_or_else(|e| panic!("[{target}] extract failed: {e}"))
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
