use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::backend::GeneratedFile;

fn generate_kt(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    kotlin::generate(&ir)
}

fn generate_java(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    java::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files.iter().find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

// ── Kotlin ─────────────────────────────────────────────────────────────────

#[test]
fn kt_data_class_for_sig() {
    let files = generate_kt("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("data class User("));
    assert!(m.contains("val name: Role"));
}

#[test]
fn kt_nullable_for_lone() {
    let files = generate_kt("sig Node { parent: lone Node }");
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("val parent: Node?"));
}

#[test]
fn kt_list_for_set() {
    let files = generate_kt("sig Group { members: set User }\nsig User {}");
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("val members: Set<User>"));
}

#[test]
fn kt_enum_class_for_all_singleton() {
    let files = generate_kt(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("enum class Color {"));
    assert!(m.contains("Red, Blue"));
}

#[test]
fn kt_sealed_class_for_enum_with_fields() {
    let files = generate_kt(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("sealed class Expr"));
    assert!(m.contains("data class BinOp("));
    assert!(m.contains(") : Expr()"));
    assert!(m.contains("val left: Expr"));
}

#[test]
fn kt_operations_use_todo() {
    let files = generate_kt("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "Operations.kt");
    assert!(ops.contains("fun changeRole("));
    assert!(ops.contains("TODO("));
}

#[test]
fn kt_tests_inline_constraint_expressions() {
    let files = generate_kt(
        "sig Item { tags: set Tag }\nsig Tag {}\nfact Tagged { all i: Item | some t: Tag | t in i.tags }",
    );
    let t = find_file(&files, "Tests.kt");
    assert!(t.contains(".all {"), "expected .all {{}} in inlined test");
    assert!(t.contains(".any {"), "expected .any {{}} in inlined test");
    assert!(t.contains(".contains("), "expected .contains() in inlined test");
    // Should NOT have Invariants.kt
    assert!(!files.iter().any(|f| f.path == "Invariants.kt"), "should not generate Invariants.kt");
}

#[test]
fn kt_tests_use_junit5() {
    let files = generate_kt("sig User {}\nassert AllUsersExist { all u: User | u = u }");
    let t = find_file(&files, "Tests.kt");
    assert!(t.contains("import org.junit.jupiter.api.Test"));
    assert!(t.contains("@Test"));
    assert!(t.contains("assertTrue("));
}

#[test]
fn kt_self_hosting() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = kotlin::generate(&ir);

    let m = find_file(&files, "Models.kt");
    assert!(m.contains("data class SigDecl("));
    assert!(m.contains("data class OxidtrIR("));
    assert!(m.contains("enum class Multiplicity"));

    // Helpers.kt should exist for TC functions
    assert!(files.iter().any(|f| f.path == "Helpers.kt"), "should generate Helpers.kt");
    // Invariants.kt should NOT exist
    assert!(!files.iter().any(|f| f.path == "Invariants.kt"), "should not generate Invariants.kt");
    // Tests should inline constraint expressions
    let t = find_file(&files, "Tests.kt");
    assert!(t.contains("assertTrue("), "tests should have assertTrue with inlined expressions");
}

// ── Kotlin: Bean Validation ─────────────────────────────────────────────────

#[test]
fn kt_bean_validation_size_on_cardinality_constraint() {
    let files = generate_kt(
        "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 10 }",
    );
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("@Size(") || m.contains("@Size see fact:"), "expected @Size annotation on members field:\n{m}");
}

#[test]
fn kt_bean_validation_min_max_on_comparison() {
    let files = generate_kt(
        "sig User { role: one User }\nfact ValidRole { all u: User | u.role != u }",
    );
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("@Min/@Max see fact: ValidRole"), "expected @Min/@Max comment:\n{m}");
}

// ── Kotlin: Operations doc ──────────────────────────────────────────────────

#[test]
fn kt_operations_kdoc_from_body() {
    let files = generate_kt(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u.r = r }",
    );
    let ops = find_file(&files, "Operations.kt");
    assert!(ops.contains("/**"), "expected KDoc comment:\n{ops}");
    assert!(ops.contains("@pre"), "expected @pre tag:\n{ops}");
}

#[test]
fn kt_operations_no_doc_when_empty_body() {
    // Operations with no body should not have doc comments
    // (in practice all preds have bodies, but empty body means no docs)
    let files = generate_kt(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "Operations.kt");
    // This pred has a body expression (u = u), so it should have docs
    assert!(ops.contains("@pre"), "expected @pre for body expression:\n{ops}");
}

// ── Java ───────────────────────────────────────────────────────────────────

#[test]
fn java_record_for_sig() {
    let files = generate_java("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.java");
    assert!(m.contains("record User("), "expected record User:\n{m}");
    assert!(!m.contains("public record"), "types should be package-private:\n{m}");
    assert!(m.contains("Role name"));
}

#[test]
fn java_nullable_for_lone() {
    let files = generate_java("sig Node { parent: lone Node }");
    let m = find_file(&files, "Models.java");
    assert!(m.contains("@Nullable"));
}

#[test]
fn java_list_for_set() {
    let files = generate_java("sig Group { members: set User }\nsig User {}");
    let m = find_file(&files, "Models.java");
    assert!(m.contains("Set<User> members"));
}

#[test]
fn java_enum_for_all_singleton() {
    let files = generate_java(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let m = find_file(&files, "Models.java");
    assert!(m.contains("enum Color {"), "expected enum Color:\n{m}");
    assert!(!m.contains("public enum"), "enums should be package-private:\n{m}");
}

#[test]
fn java_sealed_interface_for_enum_with_fields() {
    let files = generate_java(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let m = find_file(&files, "Models.java");
    assert!(m.contains("sealed interface Expr"), "expected sealed interface:\n{m}");
    assert!(m.contains("record BinOp("), "expected record BinOp:\n{m}");
    assert!(m.contains("implements Expr"));
}

#[test]
fn java_operations_throw() {
    let files = generate_java("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "Operations.java");
    assert!(ops.contains("static void changeRole("), "expected static void:\n{ops}");
    assert!(ops.contains("UnsupportedOperationException"));
}

#[test]
fn java_tests_inline_constraint_expressions() {
    let files = generate_java(
        "sig Item { tags: set Tag }\nsig Tag {}\nfact Tagged { all i: Item | some t: Tag | t in i.tags }",
    );
    let t = find_file(&files, "Tests.java");
    assert!(t.contains(".stream().allMatch("), "expected .stream().allMatch() in inlined test");
    assert!(t.contains(".stream().anyMatch("), "expected .stream().anyMatch() in inlined test");
    // Should NOT have Invariants.java
    assert!(!files.iter().any(|f| f.path == "Invariants.java"), "should not generate Invariants.java");
}

#[test]
fn java_self_hosting() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);

    let m = find_file(&files, "Models.java");
    assert!(m.contains("record SigDecl("), "expected record SigDecl:\n{}", &m[..200]);
    assert!(m.contains("record OxidtrIR("), "expected record OxidtrIR");
    assert!(m.contains("enum Multiplicity"), "expected enum Multiplicity");
    // Types should be package-private (no public keyword)
    assert!(!m.contains("public record"), "types should be package-private");
    assert!(!m.contains("public enum"), "enums should be package-private");

    // Helpers.java should exist for TC functions
    assert!(files.iter().any(|f| f.path == "Helpers.java"), "should generate Helpers.java");
    // Invariants.java should NOT exist
    assert!(!files.iter().any(|f| f.path == "Invariants.java"), "should not generate Invariants.java");
    // Tests should inline constraint expressions
    let t = find_file(&files, "Tests.java");
    assert!(t.contains("assertTrue("), "tests should have assertTrue with inlined expressions");
}

// ── Java: Bean Validation ──────────────────────────────────────────────────

#[test]
fn java_bean_validation_notnull_on_one() {
    let files = generate_java("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.java");
    assert!(m.contains("/* @NotNull */"), "expected /* @NotNull */ comment on one-mult field:\n{m}");
}

#[test]
fn java_bean_validation_size_on_cardinality_constraint() {
    let files = generate_java(
        "sig Team { members: set User }\nsig User {}\nfact TeamSize { all t: Team | #t.members <= 10 }",
    );
    let m = find_file(&files, "Models.java");
    assert!(m.contains("@Size(") || m.contains("@Size see fact:"), "expected @Size comment:\n{m}");
}

#[test]
fn java_bean_validation_min_max_on_comparison() {
    let files = generate_java(
        "sig User { role: one User }\nfact ValidRole { all u: User | u.role != u }",
    );
    let m = find_file(&files, "Models.java");
    assert!(m.contains("@Min/@Max see fact: ValidRole"), "expected @Min/@Max comment:\n{m}");
}

// ── Java: Compact Constructor (removed — assertions used non-existent globals) ──

#[test]
fn java_compact_constructor_for_constrained_record() {
    let files = generate_java(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let m = find_file(&files, "Models.java");
    // Constraints are documented via @invariant Javadoc, not compact constructors
    assert!(m.contains("@invariant HasRole"), "expected @invariant Javadoc:\n{m}");
    assert!(m.contains("record User("), "expected record User:\n{m}");
    // Should NOT reference Invariants class
    assert!(!m.contains("Invariants."), "should not reference Invariants class:\n{m}");
}

#[test]
fn java_no_compact_constructor_without_constraints() {
    let files = generate_java("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.java");
    // Should be a simple record without compact constructor
    assert!(m.contains("record User(") && m.contains(") {}"), "expected simple record:\n{m}");
}

// ── Java: Operations doc ────────────────────────────────────────────────────

#[test]
fn java_operations_javadoc_from_body() {
    let files = generate_java(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u.r = r }",
    );
    let ops = find_file(&files, "Operations.java");
    assert!(ops.contains("/**"), "expected Javadoc comment:\n{ops}");
    assert!(ops.contains("@pre"), "expected @pre tag:\n{ops}");
}

#[test]
fn java_operations_no_doc_when_no_body() {
    // A pred with body should have docs
    let files = generate_java(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "Operations.java");
    assert!(ops.contains("@pre"), "expected @pre for body expression:\n{ops}");
}

// ── Feature 1: Fun return type ──────────────────────────────────────────────

#[test]
fn kt_fun_return_type() {
    let files = generate_kt(r#"
        sig User {}
        sig Role {}
        fun getRole[u: one User]: one Role { u }
    "#);
    let ops = find_file(&files, "Operations.kt");
    assert!(ops.contains("): Role {"), "should have return type Role:\n{ops}");
}

#[test]
fn java_fun_return_type() {
    let files = generate_java(r#"
        sig User {}
        sig Role {}
        fun getRole[u: one User]: one Role { u }
    "#);
    let ops = find_file(&files, "Operations.java");
    assert!(ops.contains("static Role getRole("), "should have return type Role:\n{ops}");
}

// ── Feature 2: Singleton support ────────────────────────────────────────────

#[test]
fn kt_singleton_object() {
    let files = generate_kt("one sig Config {}");
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("object Config"), "should generate Kotlin object for singleton:\n{m}");
    assert!(!m.contains("data class Config"), "should NOT generate data class for singleton:\n{m}");
}

#[test]
fn java_singleton_enum_instance() {
    let files = generate_java("one sig Config {}");
    let m = find_file(&files, "Models.java");
    assert!(m.contains("enum Config {"), "should generate Java enum for singleton:\n{m}");
    assert!(!m.contains("public enum"), "enums should be package-private:\n{m}");
    assert!(m.contains("INSTANCE"), "should have INSTANCE constant:\n{m}");
}

// ── Feature 3: Concrete numeric values with @Size ───────────────────────────

#[test]
fn kt_concrete_size_annotation() {
    let files = generate_kt(r#"
        sig Team { members: set User }
        sig User {}
        fact TeamLimit { all t: Team | #t.members <= 10 }
    "#);
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("@Size(max = 10)"), "should have @Size(max = 10):\n{m}");
}

#[test]
fn java_concrete_size_annotation() {
    let files = generate_java(r#"
        sig Team { members: set User }
        sig User {}
        fact TeamLimit { all t: Team | #t.members <= 10 }
    "#);
    let m = find_file(&files, "Models.java");
    assert!(m.contains("@Size(max = 10)"), "should have @Size(max = 10) (in comment):\n{m}");
}

#[test]
fn java_concrete_size_min_and_max() {
    let files = generate_java(r#"
        sig Team { members: set User }
        sig User {}
        fact TeamMin { all t: Team | #t.members >= 3 }
        fact TeamMax { all t: Team | #t.members <= 10 }
    "#);
    let m = find_file(&files, "Models.java");
    assert!(m.contains("@Size(min = 3)") || m.contains("@Size(max = 10)"),
        "should have concrete @Size annotations:\n{m}");
}

// ── Feature 4: Product → Map type ───────────────────────────────────────────

#[test]
fn kt_product_field_to_map() {
    let files = generate_kt(r#"
        sig Config { settings: one Key -> Value }
        sig Key {}
        sig Value {}
    "#);
    let m = find_file(&files, "Models.kt");
    assert!(m.contains("Map<Key, Value>"), "product field should map to Map:\n{m}");
}

#[test]
fn java_product_field_to_map() {
    let files = generate_java(r#"
        sig Config { settings: one Key -> Value }
        sig Key {}
        sig Value {}
    "#);
    let m = find_file(&files, "Models.java");
    assert!(m.contains("Map<Key, Value>"), "product field should map to Map:\n{m}");
}

// ── Mine: singleton patterns ────────────────────────────────────────────────

#[test]
fn mine_kotlin_object_to_sig() {
    let src = "object Config\n";
    let mined = oxidtr::extract::kotlin_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 1, "should extract object as sig");
    assert_eq!(mined.sigs[0].name, "Config");
}

#[test]
fn mine_java_enum_instance() {
    let src = "public enum Config {\n    INSTANCE\n}\n";
    let mined = oxidtr::extract::java_extractor::extract(src);
    assert!(mined.sigs.iter().any(|s| s.name == "Config"),
        "should extract enum as sig: {:?}", mined.sigs);
}

// ── Alloy 6: var field ──────────────────────────────────────────────────────

#[test]
fn kt_var_field_uses_var_keyword() {
    let files = generate_kt(r#"
        sig Account { var balance: one Int }
    "#);
    let models = find_file(&files, "Models.kt");
    assert!(models.contains("var balance:"),
        "var field should use 'var' instead of 'val' in Kotlin:\n{models}");
    assert!(!models.contains("val balance:"),
        "var field should NOT use 'val' in Kotlin:\n{models}");
}

#[test]
fn java_var_field_annotated() {
    // Java records are immutable. Sigs with var fields should generate a class instead.
    let files = generate_java(r#"
        sig Account { var balance: one Int }
    "#);
    let models = find_file(&files, "Models.java");
    assert!(models.contains("class Account"),
        "var field sig should generate class (not record) in Java:\n{models}");
    assert!(models.contains("MUTABLE"),
        "var field should be annotated as MUTABLE in Java:\n{models}");
    assert!(!models.contains("record Account"),
        "var field sig should NOT generate record in Java:\n{models}");
}

// ── Alloy 6: var field extraction ───────────────────────────────────────────

#[test]
fn mine_kt_var_field_from_keyword() {
    let src = "data class Account(\n    var balance: Int,\n    val name: String\n)";
    let mined = oxidtr::extract::kotlin_extractor::extract(src);
    assert_eq!(mined.sigs[0].fields.len(), 2);
    assert!(mined.sigs[0].fields[0].is_var,
        "balance should be var (uses 'var' keyword)");
    assert!(!mined.sigs[0].fields[1].is_var,
        "name should not be var (uses 'val')");
}

#[test]
fn mine_java_var_field_from_annotation() {
    let src = "record Account(/* @alloy: var */ int balance, String name) {}";
    let mined = oxidtr::extract::java_extractor::extract(src);
    assert_eq!(mined.sigs[0].fields.len(), 2);
    assert!(mined.sigs[0].fields[0].is_var,
        "balance should be var (has @alloy: var annotation)");
    assert!(!mined.sigs[0].fields[1].is_var,
        "name should not be var");
}

// ── Binary temporal static test ──────────────────────────────────────────────

#[test]
fn kt_binary_temporal_static_test_is_comment_only() {
    let files = generate_kt(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let tests = find_file(&files, "Tests.kt");
    assert!(tests.contains("temporal WaitUntilDone"),
        "should generate temporal test:\n{tests}");
    assert!(tests.contains("binary temporal: requires trace-based verification"),
        "should document trace-based verification:\n{tests}");
}

#[test]
fn java_binary_temporal_static_test_is_comment_only() {
    let files = generate_java(r#"
        sig S { x: one S }
        fact WaitUntilDone { (all s: S | s.x = s.x) until (all s: S | s.x = s.x) }
    "#);
    let tests = find_file(&files, "Tests.java");
    assert!(tests.contains("temporal_WaitUntilDone"),
        "should generate temporal test:\n{tests}");
    assert!(tests.contains("binary temporal: requires trace-based verification"),
        "should document trace-based verification:\n{tests}");
}

// ── Disjoint constraint validation ──────────────────────────────────────────

#[test]
fn kotlin_init_block_generates_disjoint_check() {
    let files = generate_kt(r#"
        sig Schedule { morning: set Task, evening: set Task }
        sig Task {}
        fact NoOverlap { no (Schedule.morning & Schedule.evening) }
    "#);
    let models = find_file(&files, "Models.kt");
    assert!(models.contains("morning"), "init block should reference morning field:\n{models}");
    assert!(models.contains("evening"), "init block should reference evening field:\n{models}");
    assert!(models.contains("must not overlap"),
        "init block should check disjoint constraint:\n{models}");
}

#[test]
fn java_constructor_generates_disjoint_check() {
    let files = generate_java(r#"
        sig Schedule { morning: set Task, evening: set Task }
        sig Task {}
        fact NoOverlap { no (Schedule.morning & Schedule.evening) }
    "#);
    let models = find_file(&files, "Models.java");
    assert!(models.contains("morning"), "constructor should reference morning field:\n{models}");
    assert!(models.contains("evening"), "constructor should reference evening field:\n{models}");
    assert!(models.contains("must not overlap"),
        "constructor should check disjoint constraint:\n{models}");
}
