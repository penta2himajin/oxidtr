use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::lean;
use oxidtr::backend::GeneratedFile;

fn generate_lean(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    lean::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── Types.lean ──────────────────────────────────────────────────────────────

#[test]
fn lean_structure_for_sig() {
    let files = generate_lean("sig User { name: one Role }\nsig Role {}");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("structure User where"));
    assert!(t.contains("name : Role"));
}

#[test]
fn lean_structure_empty_sig() {
    let files = generate_lean("sig Token {}");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("structure Token where"));
}

#[test]
fn lean_option_for_lone() {
    let files = generate_lean("sig Node { parent: lone Node }");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("parent : Option Node"));
}

#[test]
fn lean_list_for_set() {
    let files = generate_lean("sig Group { members: set User }\nsig User {}");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("members : List User"));
}

#[test]
fn lean_list_for_seq() {
    let files = generate_lean("sig Order { items: seq Item }\nsig Item {}");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("items : List Item"));
}

#[test]
fn lean_inductive_for_abstract_sig() {
    let files = generate_lean(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("inductive Color where"));
    assert!(t.contains("| red : Color"));
    assert!(t.contains("| blue : Color"));
}

#[test]
fn lean_abstract_with_fields_uses_structure_and_inductive() {
    let files = generate_lean(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let t = find_file(&files, "Types.lean");
    // Abstract with variant fields → inductive
    assert!(t.contains("inductive Expr where"));
    assert!(t.contains("| literal : Expr"));
    assert!(t.contains("| binOp (left : Expr) (right : Expr) : Expr"),
        "should use named constructor params:\n{t}");
}

#[test]
fn lean_singleton_def() {
    let files = generate_lean("one sig Admin {}");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("structure Admin where"));
    assert!(t.contains("def adminInstance : Admin"));
}

#[test]
fn lean_var_field_comment() {
    let files = generate_lean("sig Counter { var count: one Int }");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("-- Alloy var field: mutable across state transitions"));
}

// ── Constraints.lean ────────────────────────────────────────────────────────

#[test]
fn lean_no_self_ref_theorem() {
    let files = generate_lean(
        "sig Node { parent: lone Node }\nfact NoSelfRef { no n: Node | n in n.parent }",
    );
    let c = find_file(&files, "Constraints.lean");
    assert!(c.contains("theorem"));
    assert!(c.contains(":= by"), "should use tactic block:\n{c}");
    assert!(c.contains("intro x"), "should intro the variable:\n{c}");
    assert!(c.contains("sorry"), "should still have sorry for unfinished proof:\n{c}");
}

#[test]
fn lean_acyclic_theorem() {
    let files = generate_lean(
        "sig Node { next: lone Node }\nfact Acyclic { no n: Node | n in n.^next }",
    );
    let c = find_file(&files, "Constraints.lean");
    assert!(c.contains("theorem"));
    assert!(c.contains(":= by"), "should use tactic block:\n{c}");
    assert!(c.contains("intro x h"), "should intro both x and hypothesis:\n{c}");
    assert!(c.contains("sorry"));
}

#[test]
fn lean_field_ordering_theorem() {
    let files = generate_lean(
        "sig Range { lo: one Int, hi: one Int }\nfact { all r: Range | r.lo < r.hi }",
    );
    let c = find_file(&files, "Constraints.lean");
    assert!(c.contains("theorem"));
    assert!(c.contains(":= by"), "should use tactic block:\n{c}");
    assert!(c.contains("intro x"), "should intro the variable:\n{c}");
}

#[test]
fn lean_no_constraints_no_file() {
    let files = generate_lean("sig Foo {}");
    assert!(files.iter().all(|f| f.path != "Constraints.lean"));
}

#[test]
fn lean_iff_theorem_uses_constructor_tactic() {
    let files = generate_lean(
        "sig Item { active: one Bool, visible: one Bool }\nfact { all i: Item | i.active iff i.visible }",
    );
    let c = find_file(&files, "Constraints.lean");
    assert!(c.contains("constructor"), "iff should use constructor tactic:\n{c}");
    assert!(c.contains("forward direction"), "should label forward direction:\n{c}");
    assert!(c.contains("backward direction"), "should label backward direction:\n{c}");
}

#[test]
fn lean_implication_theorem_intros_hypothesis() {
    let files = generate_lean(
        "sig User { age: one Int, canDrive: one Bool }\nfact { all u: User | u.age > 16 implies u.canDrive = true }",
    );
    let c = find_file(&files, "Constraints.lean");
    assert!(c.contains(":= by"), "should use tactic block:\n{c}");
    assert!(c.contains("intro x h"), "should intro variable and hypothesis:\n{c}");
}

/// An Alloy fact restricts which instances exist. Restated as a claim about
/// every inhabitant of the Lean type it is generally false, so the canned
/// `simp [List.length]; omega` script could never close it — and a failing
/// tactic is a hard error, which is how the backend shipped output that has
/// never typechecked (#79). `sorry` is a warning, so the file still builds.
#[test]
fn lean_cardinality_theorem_defers_instead_of_running_a_doomed_tactic() {
    let files = generate_lean(
        "sig Team { members: set User }\nsig User {}\nfact { all t: Team | #t.members <= 10 }",
    );
    let c = find_file(&files, "Constraints.lean");
    assert!(c.contains(":= by"), "should use tactic block:\n{c}");
    assert!(c.contains("x.members.length ≤ 10"), "should state the bound:\n{c}");
    assert!(c.contains("sorry"), "cardinality goal is not provable, must defer:\n{c}");
    assert!(!c.contains("omega"), "omega cannot close an axiom restated as a goal:\n{c}");
}

// ── Operations.lean ─────────────────────────────────────────────────────────

#[test]
fn lean_pred_as_def() {
    let files = generate_lean(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "Operations.lean");
    assert!(ops.contains("def changeRole"));
    // Body is translated: u = u
    assert!(ops.contains("u = u"));
}

#[test]
fn lean_fun_as_def_with_return() {
    let files = generate_lean(
        "sig User { age: one Int }\nfun User.displayAge[]: one Int { this.age }",
    );
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("def User.displayAge"));
}

#[test]
fn lean_no_operations_no_file() {
    let files = generate_lean("sig Foo {}");
    assert!(files.iter().all(|f| f.path != "Operations.lean"));
}

// ── Map fields ──────────────────────────────────────────────────────────────

#[test]
fn lean_map_field() {
    let files = generate_lean("sig Config { settings: one Key -> Value }\nsig Key {}\nsig Value {}");
    let t = find_file(&files, "Types.lean");
    // Map type should produce something reasonable
    assert!(t.contains("Key") && t.contains("Value"));
}

// ── Deriving ────────────────────────────────────────────────────────────────

#[test]
fn lean_structure_has_deriving() {
    let files = generate_lean("sig User { name: one Role }\nsig Role {}");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("deriving Repr, BEq, DecidableEq"));
}

#[test]
fn lean_inductive_has_deriving() {
    let files = generate_lean(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("deriving Repr, BEq, DecidableEq"));
}

// ── Body translation ────────────────────────────────────────────────────────

#[test]
fn lean_derived_field_body_translated() {
    let files = generate_lean(
        "sig User { age: one Int }\nfun User.displayAge[]: one Int { this.age }",
    );
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("def User.displayAge"));
    // Body should be translated, not sorry
    assert!(t.contains(".age"));
    assert!(!t.contains("sorry"));
}

#[test]
fn lean_pred_body_translated() {
    let files = generate_lean(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "Operations.lean");
    assert!(ops.contains("u = u"));
    assert!(!ops.contains("sorry"));
}

#[test]
fn lean_empty_body_uses_sorry() {
    // pred with no body should still have sorry
    let files = generate_lean(
        "sig User {}\npred doSomething[u: one User] {}",
    );
    let ops = find_file(&files, "Operations.lean");
    assert!(ops.contains("def doSomething"));
    assert!(ops.contains("sorry"));
}

// ── Singleton defaults ──────────────────────────────────────────────────────

#[test]
fn lean_singleton_primitive_fields_have_defaults() {
    let files = generate_lean("one sig Config { maxRetries: one Int, debug: one Bool }");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("maxRetries := 0"));
    assert!(t.contains("debug := false"));
    assert!(!t.contains("sorry"));
}

#[test]
fn lean_singleton_complex_fields_use_sorry() {
    let files = generate_lean("sig Role {}\none sig Admin { role: one Role }");
    let t = find_file(&files, "Types.lean");
    assert!(t.contains("sorry"));
}

// ── Extract ─────────────────────────────────────────────────────────────────

#[test]
fn lean_extract_structure() {
    let source = r#"
structure User where
  name : String
  age : Int
"#;
    let model = oxidtr::extract::lean_extractor::extract(source);
    assert_eq!(model.sigs.len(), 1);
    assert_eq!(model.sigs[0].name, "User");
    assert_eq!(model.sigs[0].fields.len(), 2);
    assert_eq!(model.sigs[0].fields[0].name, "name");
}

#[test]
fn lean_extract_inductive() {
    let source = r#"
inductive Color where
  | red : Color
  | blue : Color
"#;
    let model = oxidtr::extract::lean_extractor::extract(source);
    assert_eq!(model.sigs.len(), 3); // Color + Red + Blue
    assert!(model.sigs[0].is_abstract);
    assert_eq!(model.sigs[1].parent, Some("Color".to_string()));
}

#[test]
fn lean_extract_inductive_named_params() {
    let source = r#"
inductive Expr where
  | literal : Expr
  | binOp (left : Expr) (right : Expr) : Expr
  deriving Repr, BEq, DecidableEq
"#;
    let model = oxidtr::extract::lean_extractor::extract(source);
    assert_eq!(model.sigs.len(), 3); // Expr + Literal + BinOp
    let bin_op = &model.sigs[2];
    assert_eq!(bin_op.name, "BinOp");
    assert_eq!(bin_op.fields.len(), 2, "should extract named constructor params:\n{:?}", bin_op.fields);
    assert_eq!(bin_op.fields[0].name, "left");
    assert_eq!(bin_op.fields[1].name, "right");
}

#[test]
fn lean_extract_option_and_list() {
    let source = r#"
structure Node where
  parent : Option Node
  children : List Node
"#;
    let model = oxidtr::extract::lean_extractor::extract(source);
    let fields = &model.sigs[0].fields;
    assert_eq!(fields[0].mult, oxidtr::extract::MinedMultiplicity::Lone);
    assert_eq!(fields[1].mult, oxidtr::extract::MinedMultiplicity::Set);
}

#[test]
fn lean_extract_theorem() {
    let source = "theorem no_self_ref : ∀ (x : Node), x.parent ≠ some x := sorry\n";
    let model = oxidtr::extract::lean_extractor::extract(source);
    assert_eq!(model.fact_candidates.len(), 1);
    assert!(model.fact_candidates[0].source_pattern.contains("lean-theorem"));
}

// ── A field targeting a variant of an abstract sig (#93) ───────────────────

const LEAN_VARIANT_FIELD_MODEL: &str = "\
sig Item {}
abstract sig Parent { items: set Item }
sig Child extends Parent {}
sig Holder { child: one Child }
";

/// Lean folds an abstract sig's children into one `inductive`, so `Child` is a
/// *constructor* of `Parent`, not a type. Naming it in a field made Lean bind
/// it as an implicit universe variable, and the structure failed to elaborate:
/// "Constructor field `child` of `Holder.mk` contains universe level
/// metavariables".
#[test]
fn lean_variant_field_uses_the_parent_type() {
    let files = generate_lean(LEAN_VARIANT_FIELD_MODEL);
    let types = find_file(&files, "Types.lean");

    assert!(types.contains("child : Parent"), "got:\n{types}");
    assert!(!types.contains("child : Child"), "`Child` names no Lean type:\n{types}");
}

// ── Whole-sig expressions (#105) ──────────────────────────────────────────

/// `v = Low` asks which case an atom is. `Low` is a constructor of the parent
/// `inductive`, not a type, so the bare name does not elaborate as a value.
#[test]
fn lean_equality_with_a_variant_is_a_pattern_match() {
    let files = generate_lean(
        "abstract sig L { tag: one Int }\none sig High extends L {}\none sig Low extends L {}\n\
         pred notLow[v: L] { v != Low }",
    );
    let ops = find_file(&files, "Operations.lean");

    assert!(!ops.contains("≠ Low"), "`Low` is a constructor, not a value:\n{ops}");
    assert!(ops.contains("¬(v matches .low ..)"),
        "being the Low atom is being the Low case:\n{ops}");
}

/// A sig name outside a quantifier domain is the set of its atoms, and this
/// encoding has no term for that. Deferring is honest; `P.length` is not.
#[test]
fn lean_whole_sig_extent_defers() {
    let files = generate_lean("one sig P { x: one Int }\npred cardOk[p: P] { p.x = #P }");
    let ops = find_file(&files, "Operations.lean");

    assert!(!ops.contains("P.length"), "`P` is a type, not a value:\n{ops}");
    assert!(ops.contains("sorry -- oxidtr: cardOk reads a sig's extent"),
        "the gap must be stated, not silently mistranslated:\n{ops}");
}

/// The quantifier domain is the one position where the type *is* what is
/// meant, so the rule above must not sweep it up.
#[test]
fn lean_quantifier_domain_is_still_the_type() {
    let files = generate_lean("sig Leaf { n: one Int }\npred allPos[a: Leaf] { all x: Leaf | x.n > 0 }");
    let ops = find_file(&files, "Operations.lean");

    assert!(ops.contains("∀ x : Leaf, x.n > 0"), "the domain is the type:\n{ops}");
}

// ── Constraint theorems bind their own variable (#117) ────────────────────

/// `analyze` strips the `all a: Sig |` prefix and rewrites the bound variable
/// to the *sig name*, which every other backend maps back to its own receiver.
/// Lean had no such step, so the theorem bound `∀ (x : Account)` and then read
/// `Account.active` — the projection function, compared against a value.
#[test]
fn lean_constraint_theorems_bind_their_own_variable() {
    let implication = find_file(
        &generate_lean(
            "sig Account { active: one Int, balance: one Int }\n\
             fact Rule { all a: Account | a.active > 0 implies a.balance > 0 }",
        ),
        "Constraints.lean",
    ).to_string();
    assert!(!implication.contains("Account.active"),
        "`Account.active` is the projection function:\n{implication}");
    assert!(implication.contains("∀ (x : Account), x.active > 0 → x.balance > 0"),
        "the theorem's own binder is what the body reads:\n{implication}");

    let iff = find_file(
        &generate_lean(
            "sig Account { active: one Int, balance: one Int }\n\
             fact Iffy { all a: Account | a.active > 0 iff a.balance > 0 }",
        ),
        "Constraints.lean",
    ).to_string();
    assert!(iff.contains("∀ (x : Account), x.active > 0 ↔ x.balance > 0"), "iff too:\n{iff}");

    let prohibition = find_file(
        &generate_lean("sig Account { balance: one Int }\nfact Never { no a: Account | a.balance < 0 }"),
        "Constraints.lean",
    ).to_string();
    assert!(prohibition.contains("∀ (x : Account), ¬(x.balance < 0)"),
        "and prohibition:\n{prohibition}");
}

/// A binder of the theorem's own name shadows the receiver, so the body under
/// it must be left alone.
#[test]
fn lean_rebinding_stops_at_a_shadowing_binder() {
    let files = generate_lean(
        "sig Item {}\nsig Box { items: set Item, ok: one Int }\n\
         fact R { all b: Box | b.ok > 0 implies (all x: Item | x = x) }",
    );
    let src = find_file(&files, "Constraints.lean");

    assert!(src.contains("∀ x : Item, x = x"),
        "the inner binder is its own:\n{src}");
}

/// `no (A.xs & B.ys)` is about the *elements*, read across every atom of A and
/// of B. This encoding has no term for a sig's extent, and a variant has no
/// type to bind — the theorem named `Additive`, a constructor of `Cat`.
#[test]
fn lean_disjointness_over_an_extent_defers() {
    let files = generate_lean(
        "sig Item {}\nabstract sig Cat { covers: set Item }\n\
         sig Additive extends Cat {}\nsig Multiplicative extends Cat {}\n\
         fact NoOverlap { no (Additive.covers & Multiplicative.covers) }",
    );
    let src = find_file(&files, "Constraints.lean");

    assert!(!src.contains("theorem disjoint_"),
        "the theorem cannot be stated here:\n{src}");
    assert!(src.contains("-- oxidtr: `no (Additive.covers & Multiplicative.covers)` reads a sig's extent"),
        "the gap must be stated, not silently mistranslated:\n{src}");
}

// ── Facts and preds are no longer dropped silently (#118) ─────────────────

/// A pred's clauses are conjoined in Alloy. Lean took `body.last()` and
/// dropped the rest — silently, because what remained still elaborated.
#[test]
fn lean_multi_clause_pred_keeps_every_clause() {
    let files = generate_lean(
        "sig Acct { bal: one Int, cap: one Int }\n\
         pred solvent[a: Acct] { a.bal > 0\n  a.bal < a.cap }",
    );
    let ops = find_file(&files, "Operations.lean");

    assert!(ops.contains("a.bal > 0 ∧ a.bal < a.cap"),
        "every clause of a pred is part of it:\n{ops}");
}

/// `Presence` was copied from Rust, where `lone` is not an `Option`. Here it
/// is, so "guaranteed by type" was a false claim and the fact was lost.
#[test]
fn lean_presence_of_a_lone_field_is_a_theorem() {
    let lone = find_file(
        &generate_lean("sig Cfg { name: lone Str }\nfact HasName { all c: Cfg | some c.name }"),
        "Constraints.lean",
    ).to_string();
    assert!(!lone.contains("field is non-Option"),
        "a `lone` field *is* an Option here:\n{lone}");
    assert!(lone.contains("∀ (x : Cfg), x.name ≠ none"),
        "so presence is a claim to prove, not a comment:\n{lone}");
}

/// `ValueBound` had no arm at all and fell into the catch-all.
#[test]
fn lean_value_bound_gets_a_theorem() {
    let files = generate_lean("sig Cfg { size: one Int }\nfact Big { all c: Cfg | c.size > 3 }");
    let src = find_file(&files, "Constraints.lean");

    // `analyze` normalises `> 3` to `AtLeast(4)`, the same claim over an Int.
    assert!(src.contains("∀ (x : Cfg), x.size ≥ 4"), "the bound is a theorem:\n{src}");
}

// ── Uninhabited types derive nothing (#122) ───────────────────────────────

/// A `one` field is stored by value, so `structure Node where next : Node` is
/// infinitely sized: nothing constructs it and `Repr`/`BEq` cannot be derived.
#[test]
fn lean_self_referential_one_field_derives_nothing() {
    let files = generate_lean("sig Node { next: one Node }");
    let types = find_file(&files, "Types.lean");

    assert!(!types.contains("deriving"), "no instance can be synthesised:\n{types}");
    assert!(types.contains("-- oxidtr: no finite value of Node exists"),
        "and the reason must be stated:\n{types}");
}

/// The gap is transitive exactly as `DecidableEq`'s is: a holder inherits it
/// through a container as readily as through a bare field.
#[test]
fn lean_holder_of_an_uninhabited_type_derives_nothing() {
    let files = generate_lean("sig Node { next: one Node }\nsig Box { maybe: lone Node }");
    let types = find_file(&files, "Types.lean");

    assert!(!types.contains("deriving"), "`Repr (Option Node)` needs `Repr Node`:\n{types}");
    assert!(types.contains("-- oxidtr: nothing derives for Box — it holds a type with no finite value"),
        "Box itself is finite; what it holds is not:\n{types}");
}

/// A `lone`/`set` self-reference still breaks the cycle, so it keeps deriving
/// what it always did.
#[test]
fn lean_lone_self_reference_still_derives() {
    let files = generate_lean("sig Node { next: lone Node }");
    let types = find_file(&files, "Types.lean");

    assert!(types.contains("  next : Option Node\n  deriving Repr, BEq\n"),
        "`Option Node` is finite:\n{types}");
}
