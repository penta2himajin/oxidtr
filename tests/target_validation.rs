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
        ("ambiguous_field_multiplicity",
         "sig Item {}\nsig Many { rel: set Item }\nsig Maybe { rel: lone Item }\n\
          fact NoMember { all m: Maybe | all i: Item | not (i in m.rel) }",
         "different multiplicities"),
        ("reserved_argument_label",
         "sig Val {}\nsig Node { inout: one Val, values: set Val, next: lone Node }\n\
          assert Reflexive { all n: Node | n = n }",
         "func anomalyEmptyNode() -> Node {\n    Node(\n        `inout`: defaultVal(),"),
        ("boundary_set_of_natives",
         "sig Box { marks: set Int }\nfact ExactlyTwo { all b: Box | #b.marks = 2 }",
         "marks: Set([0, 1])"),
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
