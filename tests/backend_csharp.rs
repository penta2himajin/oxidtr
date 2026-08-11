use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::csharp;
use oxidtr::backend::csharp::expr_translator;
use oxidtr::backend::GeneratedFile;

fn generate_cs(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    csharp::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── Models.cs ────────────────────────────────────────────────────────────────

#[test]
fn cs_class_for_sig() {
    let files = generate_cs("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public class User"));
    assert!(m.contains("public Role Name { get; set; }"));
}

#[test]
fn cs_nullable_for_lone() {
    let files = generate_cs("sig Node { parent: lone Node }");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public Node? Parent { get; set; }"));
}

#[test]
fn cs_list_for_set() {
    let files = generate_cs("sig Group { members: set User }\nsig User {}");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public List<User> Members { get; set; }"));
}

#[test]
fn cs_list_for_seq() {
    let files = generate_cs("sig Order { items: seq Item }\nsig Item {}");
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public List<Item> Items { get; set; }"));
}

#[test]
fn cs_enum_for_all_singleton() {
    let files = generate_cs(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public enum Color"));
    assert!(m.contains("Red"));
    assert!(m.contains("Blue"));
}

#[test]
fn cs_abstract_class_with_fields() {
    let files = generate_cs(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let m = find_file(&files, "Models.cs");
    assert!(m.contains("public abstract class Expr"));
    assert!(m.contains("public class BinOp : Expr"));
    assert!(m.contains("public Expr Left { get; set; }"));
    assert!(m.contains("public Expr Right { get; set; }"));
}

// ── Operations.cs ────────────────────────────────────────────────────────────

/// A pred used to be emitted as `static void` that threw. It is a formula, so
/// it returns `bool` and its clauses are translated (#82).
#[test]
fn cs_operations_are_boolean_relations() {
    let files = generate_cs("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "Operations.cs");
    assert!(ops.contains("public static bool ChangeRole("), "a pred denotes true or false:\n{ops}");
    assert!(!ops.contains("throw new NotImplementedException("), "the stub must be gone:\n{ops}");
}

#[test]
fn cs_operations_with_return() {
    let files = generate_cs("sig User {}\nfun getUser[]: one User { User }");
    let ops = find_file(&files, "Operations.cs");
    assert!(ops.contains("public static User GetUser("));
}

// ── Fixtures.cs ──────────────────────────────────────────────────────────────

#[test]
fn cs_fixtures_default_factory() {
    let files = generate_cs("sig User { name: one Role }\nsig Role {}");
    let fix = find_file(&files, "Fixtures.cs");
    assert!(fix.contains("public static User DefaultUser()"));
    assert!(fix.contains("new User"));
}

#[test]
fn cs_fixtures_boundary() {
    let files = generate_cs("sig Group { members: set User }\nsig User {}");
    let fix = find_file(&files, "Fixtures.cs");
    assert!(fix.contains("public static Group BoundaryGroup()"));
}

// ── Tests.cs ─────────────────────────────────────────────────────────────────

#[test]
fn cs_tests_generated_for_constraints() {
    let files = generate_cs("sig Node { parent: lone Node }\nfact NoSelfRef { all n: Node | n.parent != n }");
    let t = find_file(&files, "Tests.cs");
    assert!(t.contains("[Fact]") || t.contains("[Test]"));
    assert!(t.contains("NoSelfRef"));
}

// ── expr_translator ──────────────────────────────────────────────────────────

fn translate_cs(alloy: &str, constraint_name: &str) -> String {
    let model = parser::parse(alloy).expect("parse");
    let ir_result = ir::lower(&model).expect("lower");
    let constraint = ir_result.constraints.iter()
        .find(|c| c.name.as_deref() == Some(constraint_name))
        .expect("constraint not found");
    expr_translator::translate_with_ir(&constraint.expr, &ir_result)
}

#[test]
fn cs_expr_comparison_eq() {
    let result = translate_cs("sig User { name: one Role }\nsig Role {}\nfact Eq { all u: User | u.name = u.name }", "Eq");
    assert!(result.contains("=="), "expected == in: {result}");
}

#[test]
fn cs_expr_comparison_neq() {
    let result = translate_cs("sig Node { parent: lone Node }\nfact NoSelf { all n: Node | n.parent != n }", "NoSelf");
    assert!(result.contains("!="), "expected != in: {result}");
}

#[test]
fn cs_expr_field_access_pascal_case() {
    let result = translate_cs("sig User { name: one Role }\nsig Role {}\nfact F { all u: User | u.name = u.name }", "F");
    assert!(result.contains(".Name"), "expected .Name in: {result}");
}

#[test]
fn cs_expr_quantifier_all() {
    let result = translate_cs("sig User {}\nfact F { all u: User | u = u }", "F");
    assert!(result.contains(".All(") || result.contains("All(") || result.contains("TrueForAll("),
        "expected LINQ All in: {result}");
}

#[test]
fn cs_expr_not() {
    let result = translate_cs("sig Node { parent: lone Node }\nfact F { all n: Node | not (n.parent = n) }", "F");
    assert!(result.contains("!"), "expected ! in: {result}");
}

#[test]
fn cs_expr_implies() {
    let result = translate_cs("sig A { x: lone A }\nfact F { all a: A | a.x = a implies a = a }", "F");
    // C# implies: !(cond) || (consequent)
    assert!(result.contains("||"), "expected || for implies in: {result}");
}

#[test]
fn cs_expr_prime() {
    let result = translate_cs("sig S { var x: one S }\nfact F { all s: S | s.x' = s }", "F");
    assert!(result.contains("NextX") || result.contains("nextX"),
        "expected prime translation in: {result}");
}

#[test]
fn cs_expr_cardinality() {
    let result = translate_cs("sig G { members: set G }\nfact F { all g: G | #g.members = #g.members }", "F");
    assert!(result.contains(".Count") || result.contains("Count("),
        "expected Count in: {result}");
}

// ── Derived fields (fun Sig.name → property) ────────────────────────────────

#[test]
fn cs_derived_field_generates_extension_method() {
    let files = generate_cs(r#"
        sig Account { deposits: set Int }
        fun Account.balance: one Int { #this.deposits }
    "#);
    let models = find_file(&files, "Models.cs");
    // `Int` is an Alloy native alias, not a real C# type — it must resolve to
    // `long` the same way a stored field does, or this doesn't compile.
    //
    // C# has no extension properties, so a no-parameter derived field is an
    // extension *method*. `public static long Balance => ..` was a static
    // property with no receiver: nothing could call it on an instance.
    assert!(
        models.contains("public static long Balance(this Account self) =>"),
        "should generate an extension method:\n{models}"
    );
    assert!(
        models.contains("=> self.Deposits.Count;"),
        "the body must be translated against `self`:\n{models}"
    );
}

// ── Field resolution through the binding (#95, #108, #111) ─────────────────

/// Both sigs name the field `rel`, with different multiplicities, and one fact
/// applies `no` to each — so a single translated expression has to get both
/// right, and only the binding can tell them apart.
const CS_SHARED_FIELD_MODEL: &str = "\
sig Item {}
sig Many { rel: set Item }
sig Maybe { rel: lone Item }
fact BothEmpty { all m: Many | all q: Maybe | no m.rel and no q.rel }
";

/// #108: `no e` became `e == null` unconditionally. A `set` field lowers to a
/// `List<T>` that fixtures initialise with `new List<Item>()`, so the check was
/// never null and the assertion was always false — while compiling clean.
#[test]
fn cs_no_formula_uses_count_for_a_set_field() {
    let files = generate_cs(CS_SHARED_FIELD_MODEL);
    let src = find_file(&files, "Tests.cs");

    assert!(
        src.contains("m.Rel.Count == 0"),
        "`no` over a set field is an emptiness test, not a null test. got:\n{src}"
    );
    assert!(
        !src.contains("m.Rel == null"),
        "a List is never null, so this assertion could never hold. got:\n{src}"
    );
}

/// The same `no`, over the `lone` field of the other sig, stays a null test.
#[test]
fn cs_no_formula_stays_a_null_check_for_a_lone_field() {
    let files = generate_cs(CS_SHARED_FIELD_MODEL);
    let src = find_file(&files, "Tests.cs");

    assert!(
        src.contains("q.Rel == null"),
        "`no` over a lone field is a null test. got:\n{src}"
    );
}

// ── Temporal operators must not be erased (#78) ────────────────────────────

const CS_TEMPORAL_MODEL: &str = "\
sig Counter { var n: one Int }
fact Live { eventually (all c: Counter | c.n > 0) }
assert Ordered { (all c: Counter | c.n > 0) until (all c: Counter | c.n > 5) }
";

/// C# used the temporal classification for the test *name* only and translated
/// the erased operand, on both the fact and assert paths — it is the one
/// backend with no trace-checker machinery at all.
#[test]
fn cs_temporal_fact_gets_a_trace_checker() {
    let files = generate_cs(CS_TEMPORAL_MODEL);
    let src = find_file(&files, "Tests.cs");

    assert!(src.contains("CheckLivenessLive"), "eventually needs a trace checker:\n{src}");
    assert!(src.contains(".Any("), "liveness holds in at least one state:\n{src}");
}

#[test]
fn cs_temporal_assert_gets_a_trace_checker() {
    let files = generate_cs(CS_TEMPORAL_MODEL);
    let src = find_file(&files, "Tests.cs");

    assert!(src.contains("CheckUntilOrdered"), "until needs a trace checker:\n{src}");
    assert!(
        src.contains("FindIndex("),
        "until is a position search, not a conjunction:\n{src}"
    );
}

// ── A field targeting a variant of an abstract sig (#93) ───────────────────

/// C# emits a variant as a real class, so the field type is fine — but the
/// fixture generator excludes variants, so `DefaultChild` was referenced and
/// never defined (CS0103).
#[test]
fn cs_emits_a_fixture_for_a_variant_used_as_a_field_type() {
    let files = generate_cs(
        "sig Item {}\nabstract sig Parent { items: set Item }\nsig Child extends Parent {}\n\
         sig Holder { child: one Child }",
    );
    let fixtures = find_file(&files, "Fixtures.cs");

    assert!(
        fixtures.contains("DefaultChild()"),
        "a variant used as a field type needs a factory:\n{fixtures}"
    );
    assert!(
        fixtures.contains("static Child DefaultChild"),
        "and the factory must be declared, not just called:\n{fixtures}"
    );
}

// ── Mutually recursive defaults must not recurse forever (#109) ────────────

/// `DefaultA()` built an `A1` holding `DefaultB()`, and `DefaultB()` called
/// straight back — code that builds and blows the stack when run.
#[test]
fn cs_mutually_recursive_default_does_not_recurse() {
    let files = generate_cs(
        "abstract sig A {}\nsig A1 extends A { b: one B }\n\
         abstract sig B {}\nsig B1 extends B { a: one A }",
    );
    let fixtures = find_file(&files, "Fixtures.cs");

    assert!(
        fixtures.contains("no finite default"),
        "the impossibility must be stated, not silently skipped:\n{fixtures}"
    );
    assert!(
        !fixtures.contains("B = DefaultB()"),
        "no finite value of A exists, so its factory must not build one:\n{fixtures}"
    );
}

// ── Whole-sig expressions (#105) ──────────────────────────────────────────

/// In Alloy a sig name in an expression is the set of its atoms. C# has no
/// such value — the name is a type — so `#P` became `P.Count`.
#[test]
fn cs_whole_sig_expressions_use_the_materialised_domain() {
    let card = find_file(
        &generate_cs("one sig P { x: one Int }\nfact CardOne { all p: P | p.x = #P }"),
        "Tests.cs",
    ).to_string();
    assert!(!card.contains("P.Count"), "`P` is a type, not a value:\n{card}");
    assert!(card.contains("p.X == ps.Count"),
        "the sig's cardinality is that of the sample the test builds:\n{card}");

    let eq = find_file(
        &generate_cs(
            "one sig Config { limit: one Int }\nsig N { c: one Config }\n\
             fact UsesConfig { all n: N | n.c = Config }",
        ),
        "Tests.cs",
    ).to_string();
    assert!(eq.contains("configs.Contains(n.C)"),
        "equality with a sig is membership in its extent:\n{eq}");
}

/// A variant is a subclass, so `v.Level != Low` compared a value against a
/// type. Which case an atom is, is a pattern match.
#[test]
fn cs_comparison_with_a_variant_tests_the_case() {
    let files = generate_cs(
        "abstract sig L { tag: one Int }\none sig High extends L {}\none sig Low extends L {}\n\
         sig Holder { level: one L }\nfact NotLow { all v: Holder | v.level != Low }",
    );
    let tests = find_file(&files, "Tests.cs");

    assert!(tests.contains("!(v.Level is Low)"),
        "being the Low atom is being the Low case:\n{tests}");
}

/// Alloy's `Schedule.Morning` is the union of `morning` over every `Schedule`
/// atom, not member access on a type — `Morning` is an instance property, so
/// the receiver form was CS0120 (#142).
#[test]
fn cs_relational_image_flat_maps_the_extent() {
    let files = generate_cs(
        "sig Task {}\nsig Schedule { morning: set Task }\n\
         assert R { no Schedule.morning }\ncheck R for 3",
    );
    let tests = find_file(&files, "Tests.cs");

    assert!(
        tests.contains("schedules.SelectMany(s => s.Morning).ToList().Count == 0"),
        "the image is a list built over the extent:\n{tests}"
    );
}
