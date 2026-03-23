/// Target validation tests — verify generated code compiles and tests pass
/// in each target language's own toolchain.
///
/// These tests require external tools (cargo, bun, gradle) and are separated
/// from unit/self-hosting tests so they can be run independently.
///
/// All tests are `#[ignore]` by default — run with:
///   cargo test --test target_validation -- --include-ignored

use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;
use oxidtr::backend::typescript;
use oxidtr::backend::jvm::{kotlin, java};

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

    // Write lib.rs that includes generated modules
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
#[ignore] // TODO: Java backend has pre-existing issues (unqualified enum access, missing sealed interface fixtures)
fn java_self_hosted_tests_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let src_main = dir.join("src/main/java/oxidtr");
    let src_test = dir.join("src/test/java/oxidtr");
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
dependencies { testImplementation 'org.junit.jupiter:junit-jupiter:5.10.2' }
test { useJUnitPlatform() }
"#).unwrap();

    // Write generated files.
    // Make all types package-private (remove `public`) so they can stay in one file.
    // Strip @NotNull annotations (require external dependency).
    for file in &files {
        let dest = if file.path == "Tests.java" { &src_test } else { &src_main };
        let mut content = String::from("package oxidtr;\n");
        if file.path == "Tests.java" {
            content.push_str("import static oxidtr.Fixtures.*;\nimport static oxidtr.Helpers.*;\n");
        }
        let body = file.content
            .replace("@NotNull ", "")
            .replace("public record ", "record ")
            .replace("public enum ", "enum ")
            .replace("public sealed interface ", "sealed interface ")
            .replace("public class Fixtures", "class Fixtures")
            .replace("public class Helpers", "class Helpers")
            .replace("public class Operations", "class Operations");
        content.push_str(&body);
        std::fs::write(dest.join(&file.path), content).unwrap();
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
