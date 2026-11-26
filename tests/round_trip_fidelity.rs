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

// ── #152: fields named after keywords ────────────────────────────────────────

/// Keywords of at least one target, plus `long_`, a legal Alloy name that
/// looks like an escaped `long` and must stay distinct from it.
const KEYWORDS: &str = "\
sig Kw {
  use: one Int, where: one Int, long: one Int, long_: one Int, class: one Int,
  type: one Int, match: one Int, fn: one Int, val: one Int, when: one Int,
  default: one Int, self: one Int, short: one Int, object: one Int, mod: one Int
}
";

#[test]
fn keyword_field_names_round_trip() {
    let names = ["use", "where", "long", "long_", "class", "type", "match", "fn", "val",
                 "when", "default", "self", "short", "object", "mod"];
    for target in TARGETS {
        let m = round_trip(KEYWORDS, target);
        let got: Vec<&str> = sig(&m, target, "Kw").fields.iter().map(|f| f.name.as_str()).collect();
        for name in names {
            assert!(got.contains(&name), "[{target}] field `{name}` not recovered; got {got:?}");
        }
        assert_eq!(got.len(), names.len(), "[{target}] fields: {got:?}");
        let diffs = check_diffs(KEYWORDS, target);
        assert!(diffs.is_empty(), "[{target}] check reported:\n{}", diffs.join("\n"));
    }
}

#[test]
fn keyword_escape_is_injective_and_reversible() {
    use oxidtr::backend::{escape_keyword, unescape_keyword, is_java_keyword};
    for name in ["long", "long_", "long__", "length", "x_", "_"] {
        let escaped = escape_keyword(name, is_java_keyword);
        assert_eq!(unescape_keyword(&escaped, is_java_keyword), name, "round trip of `{name}`");
    }
    assert_eq!(escape_keyword("long", is_java_keyword), "long_");
    assert_eq!(escape_keyword("long_", is_java_keyword), "long__");
    assert_eq!(escape_keyword("length", is_java_keyword), "length");
}

// ── #153: map fields ─────────────────────────────────────────────────────────

/// Map fields of every multiplicity, keyed by a primitive and by a struct sig
/// (which Go must key by pointer), with a sig and a primitive as the value.
const MAPS: &str = "\
abstract sig Color {}
one sig Red extends Color {}
one sig Green extends Color {}
sig Holder {
  table:  one Str -> Int,
  lookup: set Holder -> Color,
  byName: lone Str -> Holder,
  name:   one Str
}
fact LookupStable { all h: Holder | h.lookup = h.lookup }
";

#[test]
fn map_fields_keep_their_value_type_and_multiplicity() {
    use oxidtr::extract::MinedMultiplicity::*;
    for target in TARGETS {
        let m = round_trip(MAPS, target);
        for (name, mult, key, value) in [
            ("table", One, "Str", "Int"),
            ("lookup", Set, "Holder", "Color"),
            ("byName", Lone, "Str", "Holder"),
        ] {
            let f = field(&m, target, "Holder", name);
            assert_eq!((&f.mult, f.target.as_str(), f.value_type.as_deref()), (&mult, key, Some(value)),
                "[{target}] Holder.{name}");
        }
        assert_eq!(field(&m, target, "Holder", "name").value_type, None, "[{target}] Holder.name");
        let diffs: Vec<String> = check_diffs(MAPS, target).into_iter()
            .filter(|d| !d.starts_with("[DECLINED]")).collect();
        assert!(diffs.is_empty(), "[{target}] check reported:\n{}", diffs.join("\n"));
    }
}

/// The mined model renders a map field as Alloy that parses back to the same
/// field — Kotlin and Lean used to write `Map<String` and `String × Int`.
#[test]
fn rendered_map_fields_parse_back() {
    for target in TARGETS {
        let als = extract::renderer::render(&round_trip(MAPS, target));
        let model = parser::parse(&als).unwrap_or_else(|e| panic!("[{target}] {e:?}:\n{als}"));
        let ir = ir::lower(&model).unwrap();
        let holder = ir.structures.iter().find(|s| s.name == "Holder").unwrap();
        let table = holder.fields.iter().find(|f| f.name == "table").unwrap();
        assert_eq!(table.value_type.as_deref(), Some("Int"), "[{target}]:\n{als}");
    }
}

// ── #157: inheritance ────────────────────────────────────────────────────────

/// Both kinds of parent: an abstract one whose fields every case repeats, and
/// a concrete one, two levels deep, whose fields the generated child used to
/// lack. Plus fields declared with a case (with and without data), and an
/// abstract sig with no case at all.
const INHERITANCE: &str = "\
abstract sig Shape { label: one Str }
sig Circle extends Shape { radius: one Int }
sig Square extends Shape { side: one Int }

sig Base { id: one Int }
sig Derived extends Base { extra: lone Str }
sig Leaf extends Derived { depth: one Int }

abstract sig Color {}
one sig Red extends Color {}
one sig Green extends Color {}

abstract sig Orphan {}

sig Holder { circle: one Circle, red: one Red, squares: set Square, maybe: lone Circle }
fact LeafIds { all l: Leaf | l.id = l.id and l.depth = l.depth }
";

#[test]
fn inheritance_round_trips() {
    for target in TARGETS {
        let m = round_trip(INHERITANCE, target);
        let names = |s: &str| -> Vec<String> {
            sig(&m, target, s).fields.iter().map(|f| f.name.clone()).collect()
        };
        assert_eq!(names("Shape"), ["label"], "[{target}] Shape declares label");
        assert_eq!(names("Circle"), ["radius"], "[{target}] Circle");
        assert_eq!(names("Derived"), ["extra"], "[{target}] Derived");
        assert_eq!(names("Leaf"), ["depth"], "[{target}] Leaf");
        assert_eq!(sig(&m, target, "Derived").parent.as_deref(), Some("Base"), "[{target}] Derived extends Base");
        assert_eq!(sig(&m, target, "Leaf").parent.as_deref(), Some("Derived"), "[{target}] Leaf extends Derived");
        assert!(sig(&m, target, "Orphan").is_abstract, "[{target}] Orphan is abstract");
        for (f, t) in [("circle", "Circle"), ("red", "Red"), ("squares", "Square"), ("maybe", "Circle")] {
            assert_eq!(field(&m, target, "Holder", f).target, t, "[{target}] Holder.{f}");
        }
        let diffs: Vec<String> = check_diffs(INHERITANCE, target).into_iter()
            .filter(|d| !d.starts_with("[DECLINED]")).collect();
        assert!(diffs.is_empty(), "[{target}] check reported:\n{}", diffs.join("\n"));
    }
}

/// A concrete child carries the fields it inherits, so a fact reading them
/// through the child has something to read.
#[test]
fn a_concrete_child_carries_inherited_fields() {
    let model = parser::parse(INHERITANCE).unwrap();
    let ir = ir::lower(&model).unwrap();
    let flat = oxidtr::backend::flatten_concrete_inheritance(&ir);
    let leaf = flat.structures.iter().find(|s| s.name == "Leaf").unwrap();
    let names: Vec<&str> = leaf.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["id", "extra", "depth"]);
    let circle = flat.structures.iter().find(|s| s.name == "Circle").unwrap();
    assert_eq!(circle.fields.len(), 1, "a case of an abstract sig is left to each backend");
}

// ── #158: seq in Lean, helper classes in Java and C# ─────────────────────────

const SEQUENCES: &str = "\
sig Item {}
sig Bag { ordered: seq Item, unordered: set Item, numbers: seq Int }
fact OrderedKept { all b: Bag | #b.ordered = #b.ordered }
";

/// Lean writes `set` and `seq` as the same `List`; every other target has a
/// distinct type for each.
#[test]
fn seq_and_set_stay_distinct() {
    use oxidtr::extract::MinedMultiplicity::*;
    for target in TARGETS {
        let m = round_trip(SEQUENCES, target);
        for (name, mult) in [("ordered", Seq), ("unordered", Set), ("numbers", Seq)] {
            assert_eq!(field(&m, target, "Bag", name).mult, mult, "[{target}] Bag.{name}");
        }
    }
}

/// The classes a backend writes beside the model — fixtures, helpers,
/// operations, tests — are not sigs.
///
/// Rust's `ValidatedBag(pub Bag)` newtype is not one of them: the self-hosted
/// model declares those wrappers as sigs (`models/oxidtr/validated.als`), and
/// CI checks that extracting `src/` with the generated code finds them.
#[test]
fn helper_classes_are_not_sigs() {
    let src = format!("{SEQUENCES}pred addItem[b: one Bag, i: one Item] {{ i in b.unordered }}\n");
    let model = parser::parse(&src).unwrap();
    let declared: Vec<String> = ir::lower(&model).unwrap().structures.iter().map(|s| s.name.clone()).collect();
    for target in TARGETS {
        let m = round_trip(&src, target);
        let extra: Vec<&str> = m.sigs.iter().map(|s| s.name.as_str())
            .filter(|n| !declared.iter().any(|d| d == n))
            .filter(|n| !(target == "rust" && n.strip_prefix("Validated").is_some_and(|d| declared.iter().any(|x| x == d))))
            .collect();
        assert!(extra.is_empty(), "[{target}] sigs the model does not declare: {extra:?}");
    }
}

/// A hand-written class with no fields is still a sig when it has a parent or
/// an instance method (a node class with `accept`); one with only static
/// methods is not.
#[test]
fn a_fieldless_data_class_is_still_a_sig() {
    let java = extract::java_extractor::extract(
        "public class Marker {\n}\n\npublic class Leaf {\n    public void accept(Visitor v) { v.visit(this); }\n}\n\n\
         public class Util {\n    static int twice(int x) { return 2 * x; }\n}\n");
    let names: Vec<&str> = java.sigs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Marker", "Leaf"], "java");
    let cs = extract::csharp_extractor::extract(
        "public class Marker\n{\n}\n\npublic class Leaf\n{\n    public void Accept(Visitor v) => v.Visit(this);\n}\n\n\
         public class Util\n{\n    public static int Twice(int x) => 2 * x;\n}\n");
    let names: Vec<&str> = cs.sigs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Marker", "Leaf"], "cs");
}
