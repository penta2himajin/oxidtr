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
    assert!(m.contains("val members: List<User>"));
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
fn kt_invariants_use_all_any() {
    let files = generate_kt(
        "sig Item { tags: set Tag }\nsig Tag {}\nfact Tagged { all i: Item | some t: Tag | t in i.tags }",
    );
    let inv = find_file(&files, "Invariants.kt");
    assert!(inv.contains(".all {"), "expected .all {{}}");
    assert!(inv.contains(".any {"), "expected .any {{}}");
    assert!(inv.contains(".contains("), "expected .contains()");
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

    let inv = find_file(&files, "Invariants.kt");
    assert!(inv.contains("assertSigToStructureBijection"));
}

// ── Java ───────────────────────────────────────────────────────────────────

#[test]
fn java_record_for_sig() {
    let files = generate_java("sig User { name: one Role }\nsig Role {}");
    let m = find_file(&files, "Models.java");
    assert!(m.contains("public record User("));
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
    assert!(m.contains("List<User> members"));
}

#[test]
fn java_enum_for_all_singleton() {
    let files = generate_java(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let m = find_file(&files, "Models.java");
    assert!(m.contains("public enum Color {"));
}

#[test]
fn java_sealed_interface_for_enum_with_fields() {
    let files = generate_java(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let m = find_file(&files, "Models.java");
    assert!(m.contains("sealed interface Expr"));
    assert!(m.contains("public record BinOp("));
    assert!(m.contains("implements Expr"));
}

#[test]
fn java_operations_throw() {
    let files = generate_java("sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }");
    let ops = find_file(&files, "Operations.java");
    assert!(ops.contains("public static void changeRole("));
    assert!(ops.contains("UnsupportedOperationException"));
}

#[test]
fn java_invariants_use_stream() {
    let files = generate_java(
        "sig Item { tags: set Tag }\nsig Tag {}\nfact Tagged { all i: Item | some t: Tag | t in i.tags }",
    );
    let inv = find_file(&files, "Invariants.java");
    assert!(inv.contains(".stream().allMatch("), "expected .stream().allMatch()");
    assert!(inv.contains(".stream().anyMatch("), "expected .stream().anyMatch()");
}

#[test]
fn java_self_hosting() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).unwrap();
    let ir = ir::lower(&model).unwrap();
    let files = java::generate(&ir);

    let m = find_file(&files, "Models.java");
    assert!(m.contains("public record SigDecl("));
    assert!(m.contains("public record OxidtrIR("));
    assert!(m.contains("public enum Multiplicity"));

    let inv = find_file(&files, "Invariants.java");
    assert!(inv.contains("assertSigToStructureBijection"));
}
