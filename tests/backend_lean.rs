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

#[test]
fn lean_cardinality_theorem_uses_simp() {
    let files = generate_lean(
        "sig Team { members: set User }\nsig User {}\nfact { all t: Team | #t.members <= 10 }",
    );
    let c = find_file(&files, "Constraints.lean");
    assert!(c.contains(":= by"), "should use tactic block:\n{c}");
    assert!(c.contains("simp"), "cardinality should use simp tactic:\n{c}");
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
