/// Target validation tests — verify generated code compiles and tests pass
/// in each target language's own toolchain.
///
/// These tests require external tools (cargo, bun, gradle, go) and are separated
/// from unit/self-hosting tests so they can be run independently.
///
/// All tests are `#[ignore]` by default — run with:
///   cargo test --test target_validation -- --include-ignored

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;
use oxidtr::backend::typescript;
use oxidtr::backend::jvm::{kotlin, java};
use oxidtr::backend::go;
use oxidtr::backend::swift;
use oxidtr::backend::csharp;
use oxidtr::backend::lean;

const SELF_MODEL: &str = include_str!("../models/oxidtr.als");

fn parse_and_lower() -> ir::nodes::OxidtrIR {
    let model = parser::parse(SELF_MODEL).expect("parse oxidtr.als");
    ir::lower(&model).expect("lower oxidtr.als")
}

// ═══════════════════════════════════════════════════════════════════════════════
// Rust — cargo check / cargo test
// ═══════════════════════════════════════════════════════════════════════════════

fn write_rust_crate(ir: &ir::nodes::OxidtrIR, crate_dir: &str) {
    std::fs::create_dir_all(format!("{crate_dir}/src")).unwrap();

    let files = rust::generate(ir);

    // Write Cargo.toml
    std::fs::write(
        format!("{crate_dir}/Cargo.toml"),
        r#"[package]
name = "oxidtr_generated"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    // Detect modular layout (has lib.rs) vs flat layout (has models.rs)
    let has_lib_rs = files.iter().any(|f| f.path == "mod.rs");

    if has_lib_rs {
        // Modular layout: the generator produces its own lib.rs
        // Write all generated files (creating subdirectories as needed)
        for file in &files {
            // Rename top-level mod.rs → lib.rs for crate root
            let dest = if file.path == "mod.rs" { "lib.rs".to_string() } else { file.path.clone() };
            let file_path = format!("{crate_dir}/src/{dest}");
            if let Some(parent) = std::path::Path::new(&file_path).parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut content = String::new();
            // lib.rs and mod.rs need crate-level allow, others get file-level
            if dest == "lib.rs" {
                content.push_str("#![allow(dead_code, unused_variables, unused_imports, non_snake_case)]\n");
            } else if !file.path.ends_with("mod.rs") {
                content.push_str("#![allow(dead_code, unused_variables, unused_imports, non_snake_case)]\n");
            }
            content.push_str(&file.content);
            std::fs::write(&file_path, content).unwrap();
        }
    } else {
        // Flat layout: construct lib.rs manually
        let mut lib_rs = String::new();
        lib_rs.push_str("#[allow(dead_code, unused_variables, unused_imports)]\n");
        lib_rs.push_str("pub mod models;\n");

        let has_helpers = files.iter().any(|f| f.path == "helpers.rs");
        let has_operations = files.iter().any(|f| f.path == "operations.rs");
        let has_tests = files.iter().any(|f| f.path == "tests.rs");
        let has_fixtures = files.iter().any(|f| f.path == "fixtures.rs");
        let has_newtypes = files.iter().any(|f| f.path == "newtypes.rs");

        if has_helpers {
            lib_rs.push_str("pub mod helpers;\n");
        }
        if has_operations {
            lib_rs.push_str("pub mod operations;\n");
        }
        if has_fixtures {
            lib_rs.push_str("#[allow(dead_code, unused_variables, unused_imports)]\n");
            lib_rs.push_str("pub mod fixtures;\n");
        }
        if has_newtypes {
            lib_rs.push_str("#[allow(dead_code, unused_variables, unused_imports)]\n");
            lib_rs.push_str("pub mod newtypes;\n");
        }
        if has_tests {
            lib_rs.push_str("#[allow(dead_code, unused_variables, unused_imports)]\n");
            lib_rs.push_str("mod tests;\n");
        }

        std::fs::write(format!("{crate_dir}/src/lib.rs"), lib_rs).unwrap();

        // Write generated files
        for file in &files {
            let mut content = String::new();
            content.push_str("#![allow(dead_code, unused_variables, unused_imports, non_snake_case)]\n");
            content.push_str(&file.content);
            std::fs::write(format!("{crate_dir}/src/{}", file.path), content).unwrap();
        }
    }
}

/// Same as `rust_self_hosted_crate_compiles` but uses the Alloy-6
/// spec-compliant multi-file variant (`models/oxidtr-split.als`). Exercises
/// the modular codegen path — nested `module oxidtr/ast` etc. — and guards
/// against regressions in intermediate `mod.rs` generation and `::`-path
/// cross-module imports.
#[test]
#[ignore]
fn rust_self_hosted_split_crate_compiles() {
    let model = parser::parse_from_path(std::path::Path::new("models/oxidtr-split.als"))
        .expect("parse oxidtr-split.als");
    let ir = ir::lower(&model).expect("lower oxidtr-split.als");

    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("selfhost_split_crate");
    let crate_dir = crate_dir.to_str().unwrap();

    write_rust_crate(&ir, crate_dir);

    let output = std::process::Command::new("cargo")
        .arg("check")
        .current_dir(crate_dir)
        .output()
        .expect("failed to run cargo check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "cargo check on split model crate failed!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Generate a complete crate from oxidtr.als and verify it type-checks.
#[test]
#[ignore]
fn rust_self_hosted_crate_compiles() {
    let ir = parse_and_lower();
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("selfhost_crate");
    let crate_dir = crate_dir.to_str().unwrap();

    write_rust_crate(&ir, crate_dir);

    // Run cargo check
    let output = std::process::Command::new("cargo")
        .arg("check")
        .current_dir(crate_dir)
        .output()
        .expect("failed to run cargo check");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "cargo check failed!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // Cross-tests contain todo!() stubs by design — humans/AI fill them in.
    // We only verify compilation here; stub tests are not expected to pass.
}

/// Generate a crate from oxidtr.als and run cargo test (non-cross, non-invalid).
#[test]
#[ignore]
fn rust_self_hosted_tests_pass() {
    let ir = parse_and_lower();
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("selfhost_test_crate");
    let crate_dir_str = crate_dir.to_str().unwrap();

    write_rust_crate(&ir, crate_dir_str);

    // Run cargo test on generated code.
    // Skip cross-tests (require human implementation, marked #[ignore])
    // and invalid_ tests (tautological identity constraints).
    let output = std::process::Command::new("cargo")
        .args([
            "test", "--",
            "--skip", "preserved_after",
            "--skip", "invalid_",
        ])
        .current_dir(crate_dir_str)
        .output()
        .expect("failed to run cargo test");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "cargo test on generated crate failed!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TypeScript — bun test
// ═══════════════════════════════════════════════════════════════════════════════

/// Generate TS code from oxidtr.als and run bun test.
#[test]
#[ignore]
fn ts_self_hosted_tests_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let ir = parse_and_lower();
    let ts_config = typescript::TsBackendConfig {
        test_runner: typescript::TsTestRunner::Bun,
    };
    let files = typescript::generate_with_config(&ir, &ts_config);

    // Write generated files
    for file in &files {
        std::fs::write(dir.join(&file.path), &file.content).unwrap();
    }

    // Also write validators.ts
    let validators = typescript::generate_validators(&ir);
    if !validators.is_empty() {
        std::fs::write(dir.join("validators.ts"), &validators).unwrap();
    }

    // Run bun test on generated code.
    // Skip cross-tests (it.skip) and invalid_ tests.
    let output = std::process::Command::new("bun")
        .args(["test", "./tests.ts"])
        .current_dir(dir)
        .output()
        .expect("failed to run bun test (is bun installed?)");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // bun test exits 0 only if no failures (skips are OK)
    assert!(
        output.status.success(),
        "bun test on generated TS code failed!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Kotlin — gradle test
// ═══════════════════════════════════════════════════════════════════════════════

/// Generate Kotlin code from oxidtr.als and run gradle test.
#[test]
#[ignore]
fn kotlin_self_hosted_tests_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let src_main = dir.join("src/main/kotlin");
    let src_test = dir.join("src/test/kotlin");
    std::fs::create_dir_all(&src_main).unwrap();
    std::fs::create_dir_all(&src_test).unwrap();

    let ir = parse_and_lower();
    let files = kotlin::generate(&ir);

    // Write build.gradle.kts
    std::fs::write(dir.join("build.gradle.kts"), format!(r#"
plugins {{
    kotlin("jvm") version "2.1.20"
}}
repositories {{ mavenCentral() }}
dependencies {{
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.2")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}}
tasks.test {{ useJUnitPlatform() }}
kotlin {{
    jvmToolchain(21)
    compilerOptions {{ freeCompilerArgs.add("-Xjdk-release=21") }}
}}
// Use local kotlinc
tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {{
    kotlinOptions.freeCompilerArgs += listOf("-Xjdk-release=21")
}}
"#)).unwrap();

    // Write settings.gradle.kts (needed for Kotlin plugin)
    std::fs::write(dir.join("settings.gradle.kts"), r#"
pluginManagement {
    repositories {
        mavenCentral()
        gradlePluginPortal()
    }
}
rootProject.name = "oxidtr-kt-test"
"#).unwrap();

    // Write generated files
    for file in &files {
        let dest = if file.path == "Tests.kt" { &src_test } else { &src_main };
        std::fs::write(dest.join(&file.path), &file.content).unwrap();
    }

    // Run gradle test
    let output = std::process::Command::new("gradle")
        .args(["test", "--no-daemon", "-q"])
        .current_dir(dir)
        .output()
        .expect("failed to run gradle test (is gradle installed?)");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "gradle test (Kotlin) failed!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Java — gradle test
// ═══════════════════════════════════════════════════════════════════════════════

/// Generate Java code from oxidtr.als and run gradle test.
#[test]
#[ignore]
fn java_self_hosted_tests_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let src_main = dir.join("src/main/java");
    let src_test = dir.join("src/test/java");
    std::fs::create_dir_all(&src_main).unwrap();
    std::fs::create_dir_all(&src_test).unwrap();

    let ir = parse_and_lower();
    let files = java::generate(&ir);

    // Write build.gradle + settings.gradle
    std::fs::write(dir.join("settings.gradle"), "rootProject.name = 'oxidtr-java-test'\n").unwrap();
    std::fs::write(dir.join("build.gradle"), r#"
plugins { id 'java' }
java { sourceCompatibility = JavaVersion.VERSION_21; targetCompatibility = JavaVersion.VERSION_21 }
repositories { mavenCentral() }
dependencies {
    testImplementation 'org.junit.jupiter:junit-jupiter:5.10.2'
    testRuntimeOnly 'org.junit.platform:junit-platform-launcher'
}
test { useJUnitPlatform() }
"#).unwrap();

    // Write generated files
    for file in &files {
        let dest = if file.path == "Tests.java" { &src_test } else { &src_main };
        std::fs::write(dest.join(&file.path), &file.content).unwrap();
    }

    // Run gradle test
    let output = std::process::Command::new("gradle")
        .args(["test", "--no-daemon", "-q"])
        .current_dir(dir)
        .output()
        .expect("failed to run gradle test (is gradle installed?)");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "gradle test (Java) failed!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Go — go vet + go test
// ═══════════════════════════════════════════════════════════════════════════════

/// Generate Go from oxidtr.als and type-check + run it with the Go toolchain.
///
/// Go was previously absent from this file, and every Go defect below shipped
/// undetected as a result — `go vet` catches all of them in one pass, while the
/// string-level assertions in `tests/backend_go.rs` caught none:
///   * quantifier closures emitted as `func(p) bool` (no parameter type)
///   * `forAll`/`exists`/`contains` called but never defined
///   * `DefaultX()` missing for an interface-style abstract sig
///   * `==` applied to structs containing slices, which Go rejects
/// See #80 / #84.
#[test]
#[ignore]
fn go_self_hosted_compiles_and_tests_pass() {
    let ir = parse_and_lower();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    for file in &go::generate(&ir) {
        std::fs::write(dir.join(&file.path), &file.content).unwrap();
    }
    std::fs::write(dir.join("go.mod"), "module models\n\ngo 1.24\n").unwrap();

    let vet = std::process::Command::new("go")
        .args(["vet", "./..."])
        .current_dir(dir)
        .output()
        .expect("failed to run go vet (is go installed?)");
    assert!(
        vet.status.success(),
        "go vet on generated code failed!\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&vet.stdout),
        String::from_utf8_lossy(&vet.stderr)
    );

    let test = std::process::Command::new("go")
        .args(["test", "./..."])
        .current_dir(dir)
        .output()
        .expect("failed to run go test");
    assert!(
        test.status.success(),
        "go test on generated code failed!\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
}

/// Table-driven compile check over the shapes that broke the Go backend in
/// peer review of #89. Each previously produced either non-compiling Go or —
/// worse — vet-clean code with wrong semantics. Compiling only
/// `models/oxidtr.als` did not exercise any of them.
#[test]
#[ignore]
fn go_adversarial_models_compile() {
    // (name, model, expected substring proving the semantics, not just the syntax)
    let cases: &[(&str, &str, &str)] = &[
        ("same_named_fields",
         "sig Item {}\nsig Other {}\nsig A { items: set Item }\nsig B { items: set Other }\n\
          assert R { all a: A | all i: a.items | i = i }",
         "func(i Item) bool"),
        ("map_field",
         "sig Value {}\nsig Config { settings: one Int -> Value }\n\
          assert R { all c: Config | c.settings = c.settings }",
         "equal(c.Settings, c.Settings)"),
        ("set_membership_shadowed_by_lone",
         "sig Item {}\nsig OptionalBox { item: lone Item }\nsig SetBox { item: set Item }\n\
          assert R { all b: SetBox | all i: Item | i in b.item }",
         "contains(b.Item, i)"),
        ("variant_in_interface_slice",
         "sig Marker {}\nabstract sig Shape {}\nsig Circle extends Shape { marker: lone Marker }\n\
          sig Square extends Shape {}\nsig Drawing { shapes: set Shape }\n\
          assert R { all d: Drawing | all c: Circle | c in d.shapes }",
         "contains(d.Shapes, c)"),
        ("singleton_domain",
         "sig Item { marker: lone Int }\nsig A { item: one Item }\n\
          assert R { all a: A | all i: a.item | i = i }",
         "oneOf(a.Item)"),
        ("all_variants_carry_data",
         "sig Leaf {}\nabstract sig Shape {}\nsig Circle extends Shape { leaf: one Leaf }\n\
          sig Square extends Shape { leaf: one Leaf }\nsig Drawing { shape: one Shape }",
         "func DefaultLeaf() Leaf"),
        ("native_scalar_fields",
         "sig Node { tag: one Int, name: one Str, ok: one Bool, marks: set Int }",
         "Tag: 0,"),
        ("transitive_closure_domain",
         "sig Node { parent: lone Node, tag: one Int }\n\
          assert R { all n: Node | all p: n.^parent | p.tag = p.tag }",
         "func(p Node) bool"),
        ("variant_with_inherited_fields",
         "sig Leaf {}\nabstract sig Shape { leaf: one Leaf }\nsig Circle extends Shape {}\n\
          sig Square extends Shape {}\nsig Drawing { shape: one Shape }",
         "Circle{"),
        ("dependent_bindings",
         "sig Item {}\nsig Box { items: set Item }\n\
          assert R { all b: Box, x: b.items | x = x }",
         "func(x Item) bool"),
        ("inherited_field_through_child",
         "sig Item {}\nabstract sig Parent { items: set Item }\n\
          sig Child extends Parent { marker: lone Item }\n\
          assert R { all c: Child | all i: c.items | i = i }",
         "func(i Item) bool"),
        ("membership_in_one_relation",
         "sig Item {}\nsig Box { item: one Item }\n\
          assert R { all b: Box | b.item in b.item }",
         "equal(b.Item, b.Item)"),
        ("one_multiplicity_closure",
         "sig Node { next: one Node }\n\
          assert R { all n: Node | all x: n.^next | x = x }",
         "result = append(result, *current)"),
        ("set_in_set_is_subset",
         "sig Item {}\nsig Box { xs: set Item, ys: set Item }\n\
          assert R { all b: Box | b.xs in b.ys }",
         "isSubset(b.Xs, b.Ys)"),
        ("two_disjoint_groups",
         "sig S { id: one Int }\n\
          assert R { all disj a, b: S, disj c, d: S | a = c }",
         "!equal(c, d)"),
        ("disjoint_quantifier",
         "sig Tag {}\nsig Person { tags: set Tag }\n\
          assert R { all disj a, b: Person | a != b }",
         "!equal(a, b)"),
    ];

    for (name, model, expected) in cases {
        let parsed = parser::parse(model).unwrap_or_else(|e| panic!("{name}: parse failed: {e:?}"));
        let ir = ir::lower(&parsed).unwrap_or_else(|e| panic!("{name}: lower failed: {e:?}"));
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        let mut all = String::new();
        for file in &go::generate(&ir) {
            std::fs::write(dir.join(&file.path), &file.content).unwrap();
            all.push_str(&file.content);
        }
        std::fs::write(dir.join("go.mod"), "module models\n\ngo 1.24\n").unwrap();

        let vet = std::process::Command::new("go")
            .args(["vet", "./..."])
            .current_dir(dir)
            .output()
            .expect("failed to run go vet (is go installed?)");
        assert!(
            vet.status.success(),
            "{name}: go vet failed!\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&vet.stdout),
            String::from_utf8_lossy(&vet.stderr)
        );
        assert!(
            all.contains(expected),
            "{name}: compiled, but expected {expected:?} in the output — \
             a vet-clean but semantically wrong translation:\n{all}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Swift — swiftc -typecheck
// ═══════════════════════════════════════════════════════════════════════════════

/// Type-check a generated Swift package. On macOS the XCTest framework lives in
/// the SDK platform dir and needs an explicit `-F`; on Linux it is on the
/// default search path.
fn swiftc_typecheck(dir: &std::path::Path, files: &[String]) -> std::process::Output {
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = std::process::Command::new("xcrun");
        cmd.args(["swiftc", "-typecheck"]);
        let out = std::process::Command::new("xcrun")
            .arg("--show-sdk-platform-path").output()
            .expect("xcrun --show-sdk-platform-path");
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // -F finds XCTest.framework; -I finds its Swift overlay, without which
        // XCTAssert* resolve to the (unusable) C macros.
        cmd.arg("-F").arg(format!("{path}/Developer/Library/Frameworks"));
        cmd.arg("-I").arg(format!("{path}/Developer/usr/lib"));
    } else {
        cmd = std::process::Command::new("swiftc");
        cmd.arg("-typecheck");
    }
    cmd.args(files)
        .current_dir(dir)
        .output()
        .expect("failed to run swiftc (is swift installed?)")
}

fn assert_swift_typechecks(ir: &ir::nodes::OxidtrIR, label: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut names = Vec::new();
    for file in &swift::generate(ir) {
        std::fs::write(dir.join(&file.path), &file.content).unwrap();
        names.push(file.path.clone());
    }
    let out = swiftc_typecheck(dir, &names);
    assert!(
        out.status.success(),
        "{label}: swiftc -typecheck on generated Swift failed!\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore]
fn swift_self_hosted_compiles() {
    assert_swift_typechecks(&parse_and_lower(), "models/oxidtr.als");

    let split = parser::parse_from_path(std::path::Path::new("models/oxidtr-split.als"))
        .expect("parse oxidtr-split.als");
    let split_ir = ir::lower(&split).expect("lower oxidtr-split.als");
    assert_swift_typechecks(&split_ir, "models/oxidtr-split.als");
}

/// Shapes that the self-hosting model does not exercise. Mirrors
/// `go_adversarial_models_compile`: the third element pins the expected
/// translation, so a regression to wrong-but-compiling code still fails.
#[test]
#[ignore]
fn swift_adversarial_models_compile() {
    let cases: &[(&str, &str, &str)] = &[
        ("self_recursive_lone",
         "sig Node { parent: lone Node }",
         "final class Node: Equatable, Hashable {"),
        ("self_recursive_one",
         "sig Node { next: one Node }",
         "final class Node: Equatable, Hashable {"),
        ("mutual_recursion",
         "sig A { b: one B }\nsig B { a: lone A }",
         "final class B: Equatable, Hashable {"),
        ("recursion_through_collection_stays_value",
         "sig Tree { children: set Tree }",
         "struct Tree: Equatable, Hashable {"),
        ("recursive_enum",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          sig Neg extends Expr { inner: one Expr }",
         "indirect enum Expr:"),
        ("swift_keyword_cases",
         "abstract sig Op {}\none sig In extends Op {}\none sig Default extends Op {}\n\
          sig Node { op: one Op }",
         "case `in`"),
        ("swift_keyword_field",
         "sig Val {}\nsig Cfg { default: one Val, repeat: lone Val }",
         "let `default`: Val"),
        ("singleton_sig_in_a_set",
         "one sig Marker {}\nsig Box { ms: set Marker }",
         "struct Marker: Equatable, Hashable {"),
        ("set_of_structs_needs_hashable",
         "sig Item { tag: one Int }\nsig Box { items: set Item }",
         "struct Item: Equatable, Hashable {"),
        ("map_field",
         "sig Value {}\nsig Config { settings: one Int -> Value }",
         "let settings: [Int: Value]"),
        ("native_scalar_fields",
         "sig Node { tag: one Int, name: one Str, ok: one Bool, marks: set Int }",
         "func defaultNode() -> Node {\n    Node(\n        tag: 0,\n        name: \"\","),
        ("variant_with_inherited_fields",
         "sig Leaf {}\nabstract sig Shape { leaf: one Leaf }\nsig Circle extends Shape {}\n\
          sig Square extends Shape {}\nsig Drawing { shape: one Shape }",
         "case circle(leaf: Leaf)"),
        ("variant_with_inherited_fields_fixture",
         "sig Leaf {}\nabstract sig Shape { leaf: one Leaf }\nsig Circle extends Shape {}\n\
          sig Square extends Shape {}\nsig Drawing { shape: one Shape }",
         "func defaultShape() -> Shape { .circle(leaf: defaultLeaf()) }"),
        ("enum_map_payload_native_alias",
         "abstract sig Choice {}\nsig Entry extends Choice { values: one Int -> Str }",
         "case entry(values: [Int: String])"),
        ("membership_in_payload_variant",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          sig Other extends Expr {}\nassert IsLiteral { all e: Expr | e in Lit }",
         "e.isLit"),
        ("negated_lone_membership",
         "sig Node { parent: lone Node }\nfact NoSelf { all n: Node | not (n in n.parent) }",
         "!(n.parent == n)"),
        ("reserved_member_names",
         "sig Val {}\nsig Cfg { Type: one Val, Protocol: lone Val }",
         "let `Type`: Val"),
        ("transitively_recursive_enum_fixture",
         "abstract sig Expr {}\nsig Wrap extends Expr { node: one Node }\nsig Leaf extends Expr {}\n\
          sig Node { expr: one Expr }",
         "func defaultExpr() -> Expr { .leaf }"),
        ("enum_fixture_picks_constructible_case",
         "abstract sig Expr {}\nsig Loop extends Expr { expr: one Expr }\n\
          sig ViaInner extends Expr { inner: one Inner }\n\
          abstract sig Inner {}\nsig Safe extends Inner {}\nsig Back extends Inner { expr: one Expr }",
         "func defaultExpr() -> Expr { .viaInner(inner: defaultInner()) }"),
        ("unconstructible_sig_traps",
         "sig Node { next: one Node }",
         "func defaultNode() -> Node { fatalError("),
        ("mutually_recursive_sets_still_build",
         "sig A { bs: set B }\nsig B { as: set A }",
         "func defaultA() -> A {"),
        ("set_equality_against_payload_variant_is_skipped",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          one sig Other extends Expr {}\nsig Box { exprs: set Expr }\n\
          assert A { all b: Box | Lit = b.exprs }",
         "is a case constructor"),
        ("case_prefix_is_not_a_case_reference",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          one sig Literal extends Expr {}\nassert A { all e: Expr | e = Literal }",
         "e == Expr.literal"),
        ("equality_against_payload_variant",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          sig Other extends Expr {}\nassert A { all e: Expr | e = Lit }",
         "e.isLit"),
        // `rel` is `set` on one sig and `lone` on the other. This used to be
        // dropped with a "different multiplicities across sigs" note, because a
        // name-keyed lookup could not tell which one `m.rel` meant. The shared
        // TypeEnv types `m` as `Maybe`, so the `lone` branch is reached and the
        // fact is translated instead of skipped.
        ("ambiguous_field_multiplicity",
         "sig Item {}\nsig Many { rel: set Item }\nsig Maybe { rel: lone Item }\n\
          fact NoMember { all m: Maybe | all i: Item | not (i in m.rel) }",
         "!(m.rel == i)"),
        ("reserved_argument_label",
         "sig Val {}\nsig Node { inout: one Val, values: set Val, next: lone Node }\n\
          assert Reflexive { all n: Node | n = n }",
         "func anomalyEmptyNode() -> Node {\n    Node(\n        `inout`: defaultVal(),"),
        ("boundary_set_of_natives",
         "sig Box { marks: set Int }\nfact ExactlyTwo { all b: Box | #b.marks = 2 }",
         "func boundaryBox() -> Box {\n    Box(\n        marks: Set([0, 1])"),
        ("nested_enum_is_declared",
         "abstract sig Outer {}\nabstract sig Inner extends Outer {}\nsig Leaf extends Inner {}",
         "enum Inner:"),
        ("set_not_seeded_with_trapping_factory",
         "abstract sig Expr {}\nsig ViaBox extends Expr { box: one Box }\n\
          sig Box { nodes: set Node }\nsig Node { next: one Node }",
         "func defaultBox() -> Box {\n    Box(\n        nodes: Set()"),
        ("case_ref_guard_catches_member_access_on_constructor",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          sig Other extends Expr {}\nassert A { Lit.name = Lit.name }",
         "is a case constructor"),
        ("case_ref_guard_respects_left_boundary",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          sig Value { lit: one Int }\nsig Holder { myExpr: one Value }\n\
          assert A { all h: Holder | h.myExpr.lit = h.myExpr.lit }",
         "h.myExpr.lit == h.myExpr.lit"),
        ("transition_fact_with_case_ref",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          sig Other extends Expr {}\nsig Holder { var expr: one Expr }\n\
          fact T { all h: Holder | h.expr' = h.expr and Lit.name = Lit.name }",
         "is a case constructor"),
        ("recursive_class_identity_equality",
         "sig Node { var parent: lone Node }",
         "        lhs === rhs"),
        ("assert_over_quantified_domain",
         "sig Item {}\nsig Box { items: set Item }\n\
          assert R { all b: Box | all i: b.items | i = i }",
         "allSatisfy"),
        ("transitive_closure",
         "sig Node { parent: lone Node, tag: one Int }\n\
          assert R { all n: Node | all p: n.^parent | p.tag = p.tag }",
         "func tcParent"),
    ];

    for (name, model, expected) in cases {
        let parsed = parser::parse(model).unwrap_or_else(|e| panic!("{name}: parse failed: {e:?}"));
        let lowered = ir::lower(&parsed).unwrap_or_else(|e| panic!("{name}: lower failed: {e:?}"));

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut names = Vec::new();
        let mut all = String::new();
        for file in &swift::generate(&lowered) {
            std::fs::write(dir.join(&file.path), &file.content).unwrap();
            names.push(file.path.clone());
            all.push_str(&file.content);
        }

        let out = swiftc_typecheck(dir, &names);
        assert!(
            out.status.success(),
            "{name}: swiftc -typecheck failed!\nstderr:\n{}\n--- generated ---\n{all}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            all.contains(expected),
            "{name}: expected {expected:?} in generated Swift, got:\n{all}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// C# — dotnet build
// ═══════════════════════════════════════════════════════════════════════════════

/// The project file the C# checks below compile against.
///
/// `net10.0` must stay in step with `dotnet-version` in the `target-validation`
/// job of `.github/workflows/ci.yml`; a mismatch means CI has no targeting pack
/// for the framework and the check fails before it compiles a single line.
///
/// The xunit references are not optional: generated `Tests.cs` opens with
/// `using Xunit;`, so without them every `[Fact]` is a CS0246 and the build
/// error count says nothing about the backend.
const CS_PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <Nullable>enable</Nullable>
    <AssemblyName>OxidtrGenerated</AssemblyName>
    <RootNamespace>OxidtrGenerated</RootNamespace>
    <NoWarn>CS8618;CS8625;CS8600;CS8602;CS8603</NoWarn>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="xunit" Version="2.9.2" />
    <PackageReference Include="xunit.runner.visualstudio" Version="2.8.2" />
    <PackageReference Include="Microsoft.NET.Test.Sdk" Version="17.11.1" />
  </ItemGroup>
</Project>
"#;

/// Emit the generated C# into a scratch project and run `dotnet build` on it.
/// Returns the build result together with the concatenated generated sources,
/// so a caller can both require a clean compile and pin what was compiled.
fn dotnet_build(ir: &ir::nodes::OxidtrIR) -> (std::process::Output, String) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let mut all = String::new();
    for file in &csharp::generate(ir) {
        std::fs::write(dir.join(&file.path), &file.content).unwrap();
        all.push_str(&file.content);
    }
    std::fs::write(dir.join("generated.csproj"), CS_PROJECT).unwrap();

    let out = std::process::Command::new("dotnet")
        .args(["build", "--nologo", "-v", "q"])
        .current_dir(dir)
        .output()
        .expect("failed to run dotnet build (is the .NET SDK installed?)");
    (out, all)
}

fn assert_cs_builds(ir: &ir::nodes::OxidtrIR, label: &str) {
    let (out, _) = dotnet_build(ir);
    assert!(
        out.status.success(),
        "{label}: dotnet build on generated C# failed!\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// C# has never been compile-verified; it was *assumed* to work because it
/// PascalCases enum members (dodging reserved words) and its classes are
/// reference types (dodging recursive value types). Go and Swift were assumed
/// to work too, and both shipped non-compiling output. See #102 / #84.
#[test]
#[ignore]
fn cs_self_hosted_compiles() {
    assert_cs_builds(&parse_and_lower(), "models/oxidtr.als");

    let split = parser::parse_from_path(std::path::Path::new("models/oxidtr-split.als"))
        .expect("parse oxidtr-split.als");
    let split_ir = ir::lower(&split).expect("lower oxidtr-split.als");
    assert_cs_builds(&split_ir, "models/oxidtr-split.als");
}

/// Shapes the self-hosting model never exercises — it has no `one Int` field,
/// no transitive-closure domain, no two sigs sharing a field name, and no
/// identifier that collides with a C# keyword. Mirrors
/// `go_adversarial_models_compile` / `swift_adversarial_models_compile`: the
/// third element pins the expected *translation*, so a regression to
/// wrong-but-compiling C# still fails.
#[test]
#[ignore]
fn cs_adversarial_models_compile() {
    // (name, model, expected substring proving the semantics, not just the syntax)
    let cases: &[(&str, &str, &str)] = &[
        // An abstract sig lowers to `abstract class`, so fixtures may not say
        // `new Shape()` — they need a factory returning a concrete child.
        ("abstract_sig_with_concrete_children",
         "sig Radius {}\nabstract sig Shape {}\nsig Circle extends Shape { radius: one Radius }\n\
          sig Square extends Shape {}\nsig Drawing { shape: one Shape }",
         "public static Shape DefaultShape()"),
        // `Neg` is self-recursive; seeding the default from it never terminates,
        // so the factory has to pick `Lit`.
        //
        // The pin is the *whole* factory head. The bare substring `new Lit` also
        // occurs in `DefaultLit()` and `BoundaryLit()`, which every model with a
        // `Lit` sig emits regardless of what `DefaultExpr` picks — so a
        // regression to `DefaultExpr() => new Neg { Inner = new Lit { .. } }`
        // left three `new Lit` occurrences standing and kept this green. See
        // defect 7 of the #102 round-3 contract.
        ("abstract_default_picks_terminating_variant",
         "sig Name {}\nabstract sig Expr {}\nsig Lit extends Expr { name: one Name }\n\
          sig Neg extends Expr { inner: one Expr }\nsig Holder { expr: one Expr }",
         "public static Expr DefaultExpr() => new Lit"),
        // Two sigs with a field of the same name must each resolve to their own.
        ("shared_field_name_resolves_per_sig",
         "sig Item {}\nsig Marker {}\nsig Holder { items: set Item }\nsig Other { items: set Marker }\n\
          assert R { all h: Holder | all i: h.items | i = i }",
         "h.Items.TrueForAll(i => i == i)"),
        // `^parent` is called but the closure helper was never emitted.
        ("recursive_sig_transitive_closure_helper",
         "sig Node { parent: lone Node, tag: one Int }\n\
          assert R { all n: Node | all p: n.^parent | p.tag = p.tag }",
         "List<Node> TcParent("),
        // `Int` resolves to `long`, so the fixture value is a literal, not `new Int()`.
        ("native_int_field_default",
         "sig Node { parent: lone Node, tag: one Int }\n\
          assert R { all n: Node | all p: n.^parent | p.tag = p.tag }",
         "Tag = 0,"),
        ("unit_enum_stays_an_enum",
         "enum Suit { Hearts, Spades }\nsig Card { suit: one Suit }",
         "public enum Suit"),
        ("set_multiplicity_is_a_list",
         "sig Item {}\nsig Box { items: set Item, opt: lone Item, ordered: seq Item }",
         "public List<Item> Items { get; set; }"),
        ("lone_multiplicity_is_nullable",
         "sig Item {}\nsig Box { items: set Item, opt: lone Item, ordered: seq Item }",
         "public Item? Opt { get; set; }"),
        ("seq_multiplicity_is_an_annotated_list",
         "sig Item {}\nsig Box { items: set Item, opt: lone Item, ordered: seq Item }",
         "// @alloy: seq\n    public List<Item> Ordered { get; set; }"),
        // A sig named after a C# keyword needs the `@` verbatim escape, the same
        // way the Swift backend backticks its reserved words.
        ("csharp_keyword_sig_name",
         "sig Val {}\nsig lock { v: one Val }",
         "public class @lock"),
        // `extract_params` pluralises the sig name into a local; `Param` yields
        // `params`, which is a keyword.
        ("csharp_keyword_generated_local",
         "sig Param { tag: one Int }\nassert R { all p: Param | p.tag = p.tag }",
         "var @params ="),
        // Quantifier variables become lambda parameters.
        ("csharp_keyword_quantifier_var",
         "sig Item {}\nsig Box { items: set Item }\n\
          assert R { all params: Box | all event: params.items | event = event }",
         "@event => @event == @event"),
        // Predicate parameters become method parameters.
        ("csharp_keyword_pred_param",
         "sig Val {}\npred touch[event: Val, lock: Val] { event = lock }",
         "Val @event"),
        // A derived field's return type must go through the same native-type
        // resolution as a stored field, or it emits the Alloy name `Int`.
        ("derived_field_native_return_type",
         "sig Item {}\nsig Box { items: set Item }\nfun Box.size: one Int { #items }",
         "public static long Size"),
        // The three cases above pick keywords from the 61 the escape list
        // already covered, so they pass by construction. C# has 77 reserved
        // words; the 16 primitive-type ones (bool, int, object, string, void,
        // …) were deliberately excluded, so `sig object` emitted
        // `public class object` — 16 compile errors. See #102.
        ("csharp_primitive_keyword_sig_name",
         "sig Val {}\nsig object { v: one Val }\nsig Holder { string: set Val }",
         "public class @object"),
        // Escaping only the declaration is a half-fix: every reference site
        // would still name System.Object instead of the generated class.
        ("csharp_primitive_keyword_sig_name_in_reference_position",
         "sig Val {}\nsig object { v: one Val }\nsig Holder { string: set Val }",
         "return new @object"),
        // A field named after a primitive keyword. `capitalize` already lifts
        // it clear of keyword space, so the pin is that it stays *bare* —
        // escaping the property to `@String` would be just as wrong.
        ("csharp_primitive_keyword_field_name",
         "sig Val {}\nsig object { v: one Val }\nsig Holder { string: set Val }",
         "public List<Val> String { get; set; }"),
        // A user sig named after a primitive keyword, used as a field *type*.
        // Escaping here must key off whether the *original* Alloy target is a
        // native alias, not off the string `resolve_type` handed back.
        ("csharp_primitive_keyword_sig_as_field_type",
         "sig bool {}\nsig double {}\nsig void {}\n\
          sig Holder { a: one bool, b: one double, c: one void }",
         "public @double B { get; set; }"),
        // The other half of that discrimination: an Alloy `Int` field resolves
        // to the genuine C# keyword `long`, which must stay bare. Appending the
        // 16 primitives to one shared escape list makes this `@long` — a
        // verbatim identifier naming a type that does not exist (CS0246).
        // `native_int_field_default` pins only the fixture *value* (`Tag = 0`);
        // this pins the declared type.
        ("native_int_field_type_stays_bare",
         "sig Node { parent: lone Node, tag: one Int }\n\
          assert R { all n: Node | all p: n.^parent | p.tag = p.tag }",
         "public long Tag { get; set; }"),

        // --- #102 round 3: six shapes an external adversarial review
        // reproduced as non-compiling. Error counts measured at 469f9cb are
        // quoted per case; they are what these assertions were watched to fail
        // with before any src/ change.

        // A transition test declares the post-state binding `nextC` and then
        // references `next_c` (8 errors: CS0103 on `next_c`, CS1061 on `Zip`
        // for want of `using System.Linq;`, and two CS8130s because an
        // unresolved `Zip` leaves the deconstruction untypeable).
        //
        // This is a `fact`, not an `assert`, so #78 (the assert path) and #104
        // (compositional temporal translation) do not cover it.
        //
        // The pin is the assertion body: it survives whether the post-state is
        // walked with `Zip` or an index loop, and it fails if the emitter goes
        // back to naming the binding one way and reading it another.
        ("temporal_transition_fact_compiles",
         "sig Counter { var tag: one Int }\nfact R { always all c: Counter | c.tag' = c.tag }",
         "Assert.True(nextC.Tag == c.Tag);"),
        // The same transition test over a sig whose plural local is a C#
        // keyword. `capitalize("@params")` kept the leading `@`, so composing
        // the post-state name yielded the un-lexable `next@params` (4 errors:
        // CS1002/CS1003 twice over). Escaping belongs at the point an
        // identifier is *finalised*, not carried through string composition:
        // `next` + `Params` is not a keyword, so the composed local must come
        // out bare even though the local it copies from stays `@params`.
        ("temporal_transition_keyword_sig_local",
         "sig Param { var tag: one Int }\nfact R { always all p: Param | p.tag' = p.tag }",
         "var nextParams = new List<Param>(@params);"),
        // `Branch.parent : lone Node` is a perfectly good closure field — the
        // target is the sig's *parent*, not the sig itself — but
        // `extract_tc_fields` required `f.target == s.name` exactly, so no
        // helper was emitted and `TcParent(b)` was a CS0103 (2 errors).
        //
        // The pin fixes both halves of the helper's typing: it starts from the
        // sig that *declares* the field (`Branch`) and collects the field's
        // *target* (`List<Node>`). Getting either wrong does not compile —
        // `List<Branch>` cannot hold a `Node?`, and a `Node` parameter has no
        // `.Parent` to chase.
        ("subtype_field_transitive_closure_helper",
         "abstract sig Node {}\nsig Branch extends Node { parent: lone Node }\n\
          assert Reach { all b: Branch | no b.^parent }",
         "private static List<Node> TcParent(Branch start)"),
        // An abstract sig with no children lowers to a variantless `enum`. The
        // enum-default loop skipped it while `one_value_for` still emitted
        // `DefaultEmpty()` at both use sites (4 errors: CS0103 ×2, reported
        // twice). Of the contract's two permitted repairs — emit the factory,
        // or stop referencing it — this pins the factory: it is the smaller
        // change (the loop already writes exactly this shape for every other
        // enum) and it keeps `one_value_for` uniform across enum targets.
        // A variantless C# enum still has a zero value, so `default` is a real
        // answer rather than a stub.
        ("empty_abstract_sig_has_a_default_factory",
         "abstract sig Empty {}\nsig Holder { e: one Empty }",
         "public static Empty DefaultEmpty() => default;"),
        // A set union as a quantifier domain (2 errors). `Union` needs
        // `using System.Linq;`, and adding it alone is not enough: `Union`
        // returns `IEnumerable<T>`, which has no `TrueForAll` — that is a
        // `List<T>` method.
        //
        // Of the contract's two options, materialising is the one the existing
        // table already forces: `shared_field_name_resolves_per_sig` and
        // `csharp_keyword_quantifier_var` pin `TrueForAll`, so switching the
        // universal quantifier wholesale to LINQ's `All` would break passing
        // tests. Hence `.ToList()` on the set-op result, applied consistently
        // to union/intersection/difference.
        ("set_union_in_quantifier_domain",
         "sig Item {}\nsig Box { a: set Item, b: set Item }\n\
          assert R { all x: Box | all i: (x.a + x.b) | i = i }",
         "x.A.Union(x.B).ToList().TrueForAll(i => i == i)"),
        // A quantifier over a native domain kept the Alloy name and emitted
        // `Int.TrueForAll(...)` (2 errors: CS0103 on `Int`, reported twice).
        //
        // The contract asks what such a quantifier should *mean*. It means the
        // same thing every other quantifier in this backend already means: the
        // generator never enumerates a sig's true extent either — `all c:
        // Counter` becomes `TrueForAll` over a one-element sample list built
        // from a fixture. So a native domain gets the same treatment, a sample
        // list seeded with that type's zero value, and no new concept is
        // introduced. Emitting nothing was the alternative; this keeps the
        // assertion exercising the body instead of vacuously passing.
        ("quantifier_over_native_domain",
         "assert R { all i: Int | i = i }",
         "new List<long>{ 0 }.TrueForAll(i => i == i)"),
    ];

    for (name, model, expected) in cases {
        let parsed = parser::parse(model).unwrap_or_else(|e| panic!("{name}: parse failed: {e:?}"));
        let lowered = ir::lower(&parsed).unwrap_or_else(|e| panic!("{name}: lower failed: {e:?}"));

        let (out, all) = dotnet_build(&lowered);
        assert!(
            out.status.success(),
            "{name}: dotnet build failed!\nstdout:\n{}\n--- generated ---\n{all}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            all.contains(expected),
            "{name}: compiled, but expected {expected:?} in the generated C# — \
             a clean build with a wrong translation:\n{all}"
        );
    }
}

/// Two sigs sharing a field name with *differing* multiplicities. `field_mult`
/// used to resolve a field by name alone and return the first sig that declared
/// it, so `o.items` — a `lone Marker` — was translated as if it were `Holder`'s
/// `set Item` and emitted `.TrueForAll` on a bare `Marker`:
/// `CS1061: 'Marker' does not contain a definition for 'TrueForAll'`.
///
/// This was the tripwire for #95 and asserted the *known-broken* output. The
/// shared `TypeEnv` now resolves `o.items` through the binding of `o`, and a
/// `lone` domain is lifted with `Rel.LoneOf` before it is quantified over, so
/// the assertion is the right way round: the build must succeed.
#[test]
#[ignore]
fn cs_shared_field_name_differing_multiplicity_compiles() {
    const MODEL: &str = "sig Item {}\nsig Marker {}\nsig Holder { items: set Item }\n\
        sig Other { items: lone Marker }\n\
        assert R { all o: Other | all i: o.items | i = i }";
    let parsed = parser::parse(MODEL).expect("parse");
    let lowered = ir::lower(&parsed).expect("lower");

    let (out, all) = dotnet_build(&lowered);
    let diagnostics = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "generated C# must compile once the field resolves through its binding\n\
         stdout:\n{diagnostics}\n--- generated ---\n{all}"
    );
    // A clean build is not enough on its own: pin the translation that makes it
    // clean, so a future regression to name-keyed lookup cannot pass silently.
    assert!(
        all.contains("Rel.LoneOf(o.Items).TrueForAll("),
        "`o.items` is a `lone Marker` and must be lifted, not treated as a list:\n{all}"
    );
    assert!(
        !all.contains("o.Items.TrueForAll("),
        "the old name-keyed mis-resolution is back:\n{all}"
    );
}

/// A model mixing a temporal fact with an ordinary one must still produce
/// generated tests that *pass*. `eventually some p | p.x = 1` used to seed a
/// current-state fixture that made the unrelated `NowZero` fact fail.
#[test]
#[ignore]
fn rust_mixed_temporal_model_tests_pass() {
    const MODEL: &str = "some sig P { var x: one Int }\nsome sig Q { y: one Int }\n\
        fact NowZero { all q: Q | all p: P | p.x = 0 }\n\
        fact LaterOne { eventually some p: P | p.x = 1 }";
    let model = parser::parse(MODEL).expect("parse");
    let lowered = ir::lower(&model).expect("lower");

    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("mixed_crate");
    write_rust_crate(&lowered, crate_dir.to_str().unwrap());

    let out = std::process::Command::new("cargo")
        .arg("test")
        .current_dir(&crate_dir)
        .output()
        .expect("failed to run cargo test");
    assert!(
        out.status.success(),
        "generated tests for a mixed temporal model failed!\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Compile and *run* the generated temporal trace checkers against every trace
/// of length 1–3, comparing them to the operator definitions computed
/// independently. String assertions cannot catch a checker that is semantically
/// right but does not compile, nor one that compiles and is subtly wrong —
/// both happened during review of #78.
#[test]
#[ignore]
fn rust_temporal_checkers_match_their_definitions() {
    // Covers the one-param checker shape *and* the zero-param `()` shape, and
    // the traces below use empty and multi-atom states so a quantifier
    // translation error cannot hide behind exactly-one-atom states.
    const MODEL: &str = "sig P { f: one Int, g: one Int }\n\
        assert UntilOk { (all p: P | p.f = 1) until (all p: P | p.g = 1) }\n\
        assert SinceOk { (all p: P | p.f = 1) since (all p: P | p.g = 1) }\n\
        assert ReleaseOk { (all p: P | p.f = 1) release (all p: P | p.g = 1) }\n\
        assert TriggeredOk { (all p: P | p.f = 1) triggered (all p: P | p.g = 1) }\n\
        assert NoParamUntil { 1 = 1 until 1 = 2 }\n\
        assert NoParamEventually { eventually 1 = 2 }\n\
        assert EventuallyOk { eventually all p: P | p.g = 1 }\n\
        assert OnceOk { once all p: P | p.g = 1 }\n\
        sig Q { h: one Int }\n\
        assert TupleUntil { (all p: P | p.f = 1) until (all q: Q | q.h = 1) }";

    let model = parser::parse(MODEL).expect("parse");
    let ir = ir::lower(&model).expect("lower");

    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("temporal_crate");
    write_rust_crate(&ir, crate_dir.to_str().unwrap());

    // Appended to tests.rs so the private `check_*` fns are in scope.
    let harness = r#"

#[cfg(test)]
mod semantics {
    use super::*;

    /// One state per (F, G) truth combination. `shape` varies how the state
    /// realises that combination: 0 = one atom, 1 = two atoms, 2 = an empty
    /// collection (where both `all` quantifiers are vacuously true).
    fn state(f: bool, g: bool, shape: usize) -> Vec<P> {
        let atom = |f: bool, g: bool| P { f: if f { 1 } else { 0 }, g: if g { 1 } else { 0 } };
        match shape {
            1 => vec![atom(true, true), atom(f, g)],
            2 if f && g => Vec::new(),
            _ => vec![atom(f, g)],
        }
    }

    fn f_of(s: &Vec<P>) -> bool { s.iter().all(|p| p.f == 1) }
    fn g_of(s: &Vec<P>) -> bool { s.iter().all(|p| p.g == 1) }

    // `F until G`     : ∃j. G(j) ∧ ∀k<j. F(k)
    fn until_def(t: &[Vec<P>]) -> bool {
        (0..t.len()).any(|j| g_of(&t[j]) && t[..j].iter().all(f_of))
    }
    // `F since G`     : ∃j. G(j) ∧ ∀k>j. F(k)
    fn since_def(t: &[Vec<P>]) -> bool {
        (0..t.len()).any(|j| g_of(&t[j]) && t[j + 1..].iter().all(f_of))
    }
    // `F release G`   : ¬(¬F until ¬G) = ∀j. G(j) ∨ ∃k<j. F(k)
    fn release_def(t: &[Vec<P>]) -> bool {
        (0..t.len()).all(|j| g_of(&t[j]) || t[..j].iter().any(f_of))
    }
    // `F triggered G` : ¬(¬F since ¬G) = ∀j. G(j) ∨ ∃k>j. F(k)
    fn triggered_def(t: &[Vec<P>]) -> bool {
        (0..t.len()).all(|j| g_of(&t[j]) || t[j + 1..].iter().any(f_of))
    }

    fn traces() -> Vec<Vec<Vec<P>>> {
        let combos = [(false, false), (false, true), (true, false), (true, true)];
        let mut out = Vec::new();
        for shape in 0..3usize {
            for len in 1..=3usize {
                for mut n in 0..4usize.pow(len as u32) {
                    let mut trace = Vec::new();
                    for _ in 0..len {
                        let (f, g) = combos[n % 4];
                        trace.push(state(f, g, shape));
                        n /= 4;
                    }
                    out.push(trace);
                }
            }
        }
        out
    }

    /// The zero-parameter shape: a trace of unit states. `1 = 1 until 1 = 2`
    /// can never be satisfied, and `eventually 1 = 2` likewise.
    fn unit_traces() -> Vec<Vec<()>> {
        (0..=3usize).map(|n| vec![(); n]).collect()
    }

    /// The heterogeneous tuple branch: two quantified params of different
    /// sigs, so the checker takes `&[(Vec<P>, Vec<Q>)]` rather than `&[Vec<P>]`.
    #[test]
    fn tuple_checker_matches_its_definition() {
        let p = |f: bool| vec![P { f: if f { 1 } else { 0 }, g: 0 }];
        let q = |h: bool| vec![Q { h: if h { 1 } else { 0 } }];
        let combos = [(false, false), (false, true), (true, false), (true, true)];
        for len in 1..=3usize {
            for mut n in 0..4usize.pow(len as u32) {
                let mut t: Vec<(Vec<P>, Vec<Q>)> = Vec::new();
                for _ in 0..len {
                    let (f, h) = combos[n % 4];
                    t.push((p(f), q(h)));
                    n /= 4;
                }
                let f_of = |s: &(Vec<P>, Vec<Q>)| s.0.iter().all(|x| x.f == 1);
                let g_of = |s: &(Vec<P>, Vec<Q>)| s.1.iter().all(|x| x.h == 1);
                let expected = (0..t.len()).any(|j| g_of(&t[j]) && t[..j].iter().all(&f_of));
                assert_eq!(check_until_tuple_until(&t), expected, "tuple until mismatch on {t:?}");
            }
        }
    }

    /// Liveness on non-empty traces, satisfying and not — the empty-trace test
    /// the generator emits only pins the false case.
    #[test]
    fn unary_checkers_on_non_empty_traces() {
        let g = |ok: bool| vec![P { f: 1, g: if ok { 1 } else { 0 } }];
        assert!(check_liveness_eventually_ok(&[g(false), g(true)]), "eventually should find the later state");
        assert!(!check_liveness_eventually_ok(&[g(false), g(false)]), "no satisfying state");
        assert!(check_past_liveness_once_ok(&[g(true), g(false)]), "once should find the earlier state");
        assert!(!check_past_liveness_once_ok(&[g(false), g(false)]), "no satisfying state");
    }

    #[test]
    fn parameterless_checkers_are_callable_and_correct() {
        for t in unit_traces() {
            assert!(!check_until_no_param_until(&t), "1=1 until 1=2 can never hold: {t:?}");
            assert!(!check_liveness_no_param_eventually(&t), "eventually 1=2 can never hold: {t:?}");
        }
    }

    #[test]
    fn checkers_match_definitions() {
        for t in traces() {
            assert_eq!(check_until_until_ok(&t), until_def(&t), "until mismatch on {t:?}");
            assert_eq!(check_since_since_ok(&t), since_def(&t), "since mismatch on {t:?}");
            assert_eq!(check_release_release_ok(&t), release_def(&t), "release mismatch on {t:?}");
            assert_eq!(check_triggered_triggered_ok(&t), triggered_def(&t), "triggered mismatch on {t:?}");
        }
    }
}
"#;
    let tests_path = crate_dir.join("src/tests.rs");
    let mut content = std::fs::read_to_string(&tests_path).unwrap();
    content.push_str(harness);
    std::fs::write(&tests_path, content).unwrap();

    let out = std::process::Command::new("cargo")
        .args(["test", "semantics"])
        .current_dir(&crate_dir)
        .output()
        .expect("failed to run cargo test");
    assert!(
        out.status.success(),
        "generated temporal checkers disagree with their definitions!\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── Lean 4 ───────────────────────────────────────────────────────────────────

/// Typecheck the generated Lean in a scratch directory. `lean` resolves
/// `import Types` from `LEAN_PATH`, so compiling the three files in dependency
/// order with the scratch dir on that path needs no `lake` project scaffolding
/// — which keeps a full run at roughly two seconds.
///
/// Requires `lean` on PATH; `elan` puts it there. Note that PATH does not
/// propagate through `mise exec rust -- cargo …`, so invoke cargo directly.
///
/// Returns (clean, diagnostics, concatenated sources). `sorry` is deliberate in
/// generated theorems and is only a warning, so the gate is errors alone.
fn lean_typecheck(ir: &ir::nodes::OxidtrIR) -> (bool, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    let files = lean::generate(ir);
    let mut all = String::new();
    for file in &files {
        std::fs::write(dir.join(&file.path), &file.content).unwrap();
        // Separator, so a pinned substring cannot match across a file boundary.
        all.push_str("\n-- <oxidtr file boundary> --\n");
        all.push_str(&file.content);
    }
    assert!(!files.is_empty(), "lean::generate produced no files — nothing was typechecked");

    let mut diagnostics = String::new();
    let mut clean = true;
    // Dependency order first, then anything else `generate` emitted — a
    // hardcoded list would let a future file into `all` (and so into a pin)
    // while never being typechecked.
    let mut stems: Vec<String> = ["Types", "Constraints", "Operations"].iter()
        .map(|s| s.to_string())
        .filter(|s| files.iter().any(|f| f.path == format!("{s}.lean")))
        .collect();
    for f in &files {
        let stem = f.path.trim_end_matches(".lean").to_string();
        if !stems.contains(&stem) { stems.push(stem); }
    }
    for stem in &stems {
        // Run *inside* the scratch dir with relative names. `lean` takes the
        // working directory as its root and rejects a source outside it with
        // "must be contained in root directory" — a message with no `: error`
        // in it, which is how an earlier version of this harness passed while
        // never typechecking anything.
        let out = std::process::Command::new("lean")
            .arg("-o").arg(format!("{stem}.olean"))
            .arg(format!("{stem}.lean"))
            .current_dir(dir)
            .env("LEAN_PATH", dir)
            .output()
            .expect("failed to run lean (is the Lean 4 toolchain on PATH?)");
        let text = format!("{}{}",
            String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        // Gate on the exit status, not only on the text. `sorry` is a warning and
        // leaves lean at 0, so the status alone already admits it — whereas the
        // substring alone would read a toolchain failure ("error: no default
        // toolchain") or a signal death as a clean run and pass vacuously.
        if !out.status.success() || text.contains(": error") { clean = false; }
        diagnostics.push_str(&text);
    }
    (clean, diagnostics, all)
}

fn assert_lean_typechecks(ir: &ir::nodes::OxidtrIR, label: &str) {
    let (clean, diagnostics, all) = lean_typecheck(ir);
    assert!(clean,
        "{label}: lean reported errors on generated Lean!\n{diagnostics}\n--- generated ---\n{all}");
}

/// Lean had never been compile-verified — CI's `self-host` job runs only
/// `oxidtr check`, a structural diff that never invokes a target compiler, and
/// that is how the backend shipped 91 errors on its own model. See #79 / #84.
#[test]
#[ignore]
fn lean_self_hosted_compiles() {
    assert_lean_typechecks(&parse_and_lower(), "models/oxidtr.als");

    let split = parser::parse_from_path(std::path::Path::new("models/oxidtr-split.als"))
        .expect("parse oxidtr-split.als");
    let split_ir = ir::lower(&split).expect("lower oxidtr-split.als");
    assert_lean_typechecks(&split_ir, "models/oxidtr-split.als");
}

/// Shapes the self-hosting model does not exercise, frozen before the fix (see
/// the recon in #79). Mirrors `go_`/`swift_`/`cs_adversarial_models_compile`:
/// the third element pins the expected *translation*, so output that is
/// wrong-but-compiling still fails.
#[test]
#[ignore]
fn lean_adversarial_models_compile() {
    // (name, model, expected substring proving the semantics, not just the syntax)
    let cases: &[(&str, &str, &str)] = &[
        // Lean has no forward declaration: a sig used before it is declared gets
        // auto-bound as an implicit universe variable, which poisons every
        // `deriving` line after it. `B` must be emitted first. The pin spans
        // both declarations, so reordering alone regressing still fails.
        ("forward_reference_is_reordered",
         "sig A { b: one B }\nsig B { n: one Int }",
         "structure B where\n  n : Int\n  deriving Repr, BEq, DecidableEq\n\nstructure A where"),
        // A genuine cycle cannot be reordered away — it needs one `mutual` block.
        ("mutual_recursion_shares_one_block",
         "sig A { bs: set B }\nsig B { a: lone A }",
         "mutual\nstructure A where\n  bs : List B\n  deriving Repr, BEq\n\nstructure B where\n  a : Option A\n  deriving Repr, BEq\n\nend"),
        // Lean's `DecidableEq` handler has no case for a recursive type, and
        // `Option T` counts as nested. Deriving it anyway is a hard error.
        ("self_reference_through_option_drops_decidable_eq",
         "sig Node { next: lone Node }",
         "structure Node where\n  next : Option Node\n  deriving Repr, BEq\n"),
        ("self_reference_through_list_drops_decidable_eq",
         "sig Node { kids: set Node }",
         "structure Node where\n  kids : List Node\n  deriving Repr, BEq\n"),
        // `Type` is a Lean token: `structure Type where` does not even parse.
        ("keyword_sig_name_is_escaped_at_declaration",
         "sig Type { n: one Int }\nsig Holder { t: one Type }",
         "structure «Type» where"),
        // Escaping the declaration alone leaves every reference dangling, so the
        // use site is pinned separately.
        ("keyword_sig_name_is_escaped_at_use_site",
         "sig Type { n: one Int }\nsig Holder { t: one Type }",
         "  t : «Type»"),
        // Lower-camelling *manufactures* the keyword: `End` is a fine Alloy
        // name, `end` closes a Lean scope. Escaping has to happen after the case
        // change, not before.
        ("field_name_lowercamels_into_a_keyword",
         "sig Span { End: one Int, Where: one Int }",
         "  «end» : Int\n  «where» : Int"),
        ("keyword_pred_name_is_escaped",
         "sig Leaf { n: one Int }\npred Match[x: Leaf] { x.n > 0 }",
         "def «match» (x : Leaf) : Prop :="),
        // The parameter binder was emitted verbatim while the body went through
        // lower-camelling, so a capitalised parameter bound one name and the
        // body referenced another.
        ("capitalised_param_binder_matches_its_body",
         "sig Leaf { n: one Int }\npred atLeast[Limit: Leaf] { Limit.n > 0 }",
         "def atLeast (limit : Leaf) : Prop :=\n  limit.n > 0"),
        // An assert name reaches `theorem` verbatim. It has to be the lowercase
        // `def` to bite: `Def` is a perfectly good Lean identifier, and pinning
        // that would have passed without the escaping ever running.
        ("keyword_assert_name_is_escaped",
         "sig Leaf { n: one Int }\nassert def { all x: Leaf | x.n = x.n }\ncheck def for 3",
         "theorem «def» :"),
        // `∀ x : T, y : T,` is not Lean syntax, and `∀ x y ∈ e,` is rejected
        // too — one quantifier per bound variable is the only form that works
        // for both a type domain and a set domain.
        ("multi_binding_quantifier_repeats_the_quantifier",
         "sig Leaf { n: one Int }\npred bothPos[a: Leaf] { all x, y: Leaf | x.n = y.n implies x = y }",
         "∀ x : Leaf, ∀ y : Leaf, (x.n = y.n) → x = y"),
        // Lean has no forward declaration for `def` either — a pred that calls a
        // pred declared later in the model must be emitted after its callee.
        ("pred_is_emitted_after_its_callee",
         "sig Leaf { n: one Int }\npred outer[x: Leaf] { inner[x] }\npred inner[x: Leaf] { x.n > 0 }",
         "def inner (x : Leaf) : Prop :=\n  x.n > 0\n\ndef outer (x : Leaf) : Prop :=\n  inner x"),
        // An Alloy pred is a formula. Declared `: Bool`, a quantified body is a
        // `Prop` and does not coerce.
        ("quantified_pred_body_is_a_prop",
         "sig Leaf { n: one Int }\npred allPositive[a: Leaf] { all x: Leaf | x.n > 0 }",
         "def allPositive (a : Leaf) : Prop :=\n  ∀ x : Leaf, x.n > 0"),
        // Constraints.lean was gated on facts alone, so a model carrying only
        // asserts silently produced no theorems at all.
        ("assert_without_any_fact_still_emits_a_theorem",
         "sig Leaf { n: one Int }\nassert Trivial { all x: Leaf | x.n = x.n }\ncheck Trivial for 3",
         "theorem Trivial :"),
        // Alloy's `this` is the receiver; the emitted binder is `self`.
        ("derived_field_receiver_is_named_self",
         "sig Item {}\nsig Bag { items: set Item }\nfun Bag.size: one Int { #this.items }",
         "def Bag.size (self : Bag) : Int :=\n  self.items.length"),
        // A fact restricts which instances exist. As `∀ x : Sig, …` it is a
        // claim about every inhabitant of the type and is generally false, so
        // `omega` / `simp [List.length]` / `cases x <;> simp` cannot close it —
        // and a failing tactic is a hard error, not a warning.
        ("field_ordering_fact_defers_instead_of_omega",
         "sig Span { lo: one Int, hi: one Int }\nfact Ordered { all s: Span | s.lo < s.hi }",
         "∀ (x : Span), x.lo < x.hi := by\n  intro x\n  sorry"),
        ("cardinality_fact_defers_instead_of_simp",
         "sig Item {}\nsig Bag { items: set Item }\nfact Capped { all b: Bag | #b.items <= 3 }",
         "∀ (x : Bag), x.items.length ≤ 3 := by\n  intro x\n  sorry"),
        // Ordering keys on the *shape* of the call, not the bare name. A free
        // pred calling `x.g[…]` depends on `S.g` in Types.lean, not on the free
        // pred that happens to also be called `g` — reading it as a dependency
        // made the solvable order (`f`, `g`) unsolvable and fell back to model
        // order, emitting `g` first and dangling the reference to `f`.
        ("receiver_call_is_not_a_free_op_dependency",
         "sig S { n: one Int }\nfun S.g[k: Int]: one Int { this.n }\n\
          pred g[y: S] { f[y] }\npred f[x: S] { x.g[1] > 0 }",
         "def f (x : S) : Prop :=\n  x.g 1 > 0\n\ndef g (y : S) : Prop :=\n  f y"),
        // Two sigs may each declare a derived field of the same name. Exempting
        // a callee by name equality read `A.size`'s call to `B.size` as
        // self-recursion and emitted them backwards; the exemption is by op
        // identity.
        ("same_named_derived_fields_order_by_identity",
         "sig B { m: one Int }\nsig A { b: one B }\n\
          fun A.size[u: Int]: one Int { this.b.size[u] }\nfun B.size[u: Int]: one Int { this.m }",
         "def B.size (self : B) (u : Int) : Int :=\n  self.m\n\ndef A.size (self : A) (u : Int) : Int :="),
        // `x in y.^f` is reachability, not membership: `TransGen` wants the
        // relation and *both* endpoints, so the closure cannot be translated on
        // its own and then tested with `∈`. `b ∈ a.f` is well-typed for `Option`
        // and `List` alike, which is why this needs no multiplicity lookup.
        ("transitive_closure_membership_is_reachability",
         "sig Node { parent: lone Node }\nassert NoCycle { no n: Node | n in n.^parent }\n\
          check NoCycle for 4",
         "¬ ∃ n : Node, Relation.TransGen (fun a b => b ∈ a.parent) n n"),
        // `x in Circle` asks which variant an atom is. `Circle` lowers to a Lean
        // type, so `x ∈ Circle` is a membership test against a `Type`; the
        // variant test is a pattern match.
        ("exhaustive_categories_become_pattern_matches",
         "abstract sig Shape {}\nsig Circle extends Shape {}\nsig Square extends Shape {}\n\
          fact Covers { all x: Shape | x in Circle or x in Square }",
         "∀ (x : Shape), x matches .circle .. ∨ x matches .square .."),
    ];

    for (name, model, expected) in cases {
        let parsed = parser::parse(model).unwrap_or_else(|e| panic!("{name}: parse failed: {e:?}"));
        let lowered = ir::lower(&parsed).unwrap_or_else(|e| panic!("{name}: lower failed: {e:?}"));

        let (clean, diagnostics, all) = lean_typecheck(&lowered);
        assert!(clean, "{name}: lean reported errors!\n{diagnostics}\n--- generated ---\n{all}");
        assert!(
            all.contains(expected),
            "{name}: typechecked, but expected {expected:?} in the generated Lean — \
             a clean build with a wrong translation:\n{all}"
        );
    }
}
