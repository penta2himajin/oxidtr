//! oxidtr `--konpu`: detect algebraic structures proven by Alloy facts and emit
//! konpu annotations on the generated Rust structs.

use oxidtr::generate::{self, GenerateConfig};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn gen_rust(src: &str, konpu: bool) -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("oxidtr_konpu_{n}"));
    let _ = fs::remove_dir_all(&dir);
    let model = dir.join("model.als");
    fs::create_dir_all(&dir).unwrap();
    fs::write(&model, src).unwrap();

    let mut config = GenerateConfig::new("rust", dir.to_str().unwrap());
    config.warnings = generate::WarningLevel::Off;
    config.konpu = konpu;
    generate::run(model.to_str().unwrap(), &config).unwrap();
    fs::read_to_string(dir.join("models.rs")).unwrap()
}

fn gen_rust_tests(src: &str, konpu: bool) -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("oxidtr_konpu_{n}"));
    let _ = fs::remove_dir_all(&dir);
    let model = dir.join("model.als");
    fs::create_dir_all(&dir).unwrap();
    fs::write(&model, src).unwrap();

    let mut config = GenerateConfig::new("rust", dir.to_str().unwrap());
    config.warnings = generate::WarningLevel::Off;
    config.konpu = konpu;
    generate::run(model.to_str().unwrap(), &config).unwrap();
    fs::read_to_string(dir.join("tests.rs")).unwrap()
}

// Fun-form identity (not a `one sig`), so the identity law tests are actually
// generated — this is the shape `--konpu` recommends.
const MONOID_FUN: &str = r#"
sig Money { amount: one Int }
fun zero: Money { Money }
fun add[a, b: Money]: Money { a }
fact Assoc { all a, b, c: Money | add[add[a, b], c] = add[a, add[b, c]] }
fact Ident { all a: Money | add[a, zero] = a and add[zero, a] = a }
"#;

#[test]
fn konpu_law_annotations_on_invariant_tests() {
    // Regression #65: --konpu must tag law tests so konpu doesn't warn
    // MissingLawTest even though the laws are tested.
    let out = gen_rust_tests(MONOID_FUN, true);
    assert!(out.contains("#[konpu::law(associativity)]"),
        "assoc test should carry the associativity law annotation:\n{out}");
    assert!(out.contains("#[konpu::law(left_identity, right_identity)]"),
        "ident test should carry both identity-side annotations:\n{out}");
    // The annotation must sit on the matching test.
    let idx = out.find("#[konpu::law(associativity)]").unwrap();
    assert!(out[idx..].contains("fn invariant_assoc"),
        "associativity annotation must precede invariant_assoc:\n{out}");
}

#[test]
fn no_law_annotations_without_konpu_flag() {
    let out = gen_rust_tests(MONOID_FUN, false);
    assert!(!out.contains("konpu::law"),
        "flag off must not emit law annotations:\n{out}");
}

const GROUP_FUN: &str = r#"
sig Elem {}
fun e: Elem { Elem }
fun op[a, b: Elem]: Elem { a }
fun inv[a: Elem]: Elem { a }
fact Assoc { all a, b, c: Elem | op[op[a, b], c] = op[a, op[b, c]] }
fact Ident { all a: Elem | op[a, e] = a and op[e, a] = a }
fact Inverse { all a: Elem | op[a, inv[a]] = e and op[inv[a], a] = e }
"#;

#[test]
fn konpu_group_inverse_law_annotation() {
    // konpu's law tokens for a group's inverse are inverse_left / inverse_right
    // (verified against konpu law_from_name / all_law_requirements) — not
    // "invertibility". The inverse-law test must carry both.
    let out = gen_rust_tests(GROUP_FUN, true);
    assert!(out.contains("#[konpu::law(inverse_left, inverse_right)]"),
        "inverse-law test should carry both inverse-side annotations:\n{out}");
    let idx = out.find("#[konpu::law(inverse_left, inverse_right)]").unwrap();
    assert!(out[idx..].contains("fn invariant_inverse"),
        "inverse annotation must precede invariant_inverse:\n{out}");
}

const MONOID: &str = r#"
sig Money { amount: one Int }
one sig Zero extends Money {}
fun add[a, b: Money]: Money { a }
fact Assoc { all a, b, c: Money | add[add[a, b], c] = add[a, add[b, c]] }
fact Ident { all a: Money | add[a, Zero] = a and add[Zero, a] = a }
"#;

#[test]
fn monoid_annotation_emitted_with_konpu() {
    let out = gen_rust(MONOID, true);
    assert!(
        out.contains(r#"#[konpu::monoid(op = "add", identity = "Zero")]"#),
        "expected monoid annotation, got:\n{out}"
    );
    // annotation sits on the Money struct
    let idx = out.find("#[konpu::monoid").unwrap();
    assert!(out[idx..].contains("pub struct Money"));
}

#[test]
fn no_annotation_without_konpu_flag() {
    let out = gen_rust(MONOID, false);
    assert!(!out.contains("konpu::"), "flag off must not emit, got:\n{out}");
}

#[test]
fn semigroup_when_no_identity() {
    let src = r#"
sig S { x: one Int }
fun combine[a, b: S]: S { a }
fact Assoc { all a, b, c: S | combine[combine[a, b], c] = combine[a, combine[b, c]] }
"#;
    let out = gen_rust(src, true);
    assert!(
        out.contains(r#"#[konpu::semigroup(op = "combine")]"#),
        "expected semigroup, got:\n{out}"
    );
}

#[test]
fn one_sig_identity_warns_suggesting_fun_form() {
    // one-sig identity: annotation still emitted, but warn that identity-law
    // tests need a carrier-valued `fun` identity.
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("oxidtr_konpu_{n}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let model = dir.join("model.als");
    fs::write(&model, MONOID).unwrap();

    let mut config = GenerateConfig::new("rust", dir.to_str().unwrap());
    config.warnings = generate::WarningLevel::Off;
    config.konpu = true;
    let result = generate::run(model.to_str().unwrap(), &config).unwrap();
    assert!(
        result.warnings.iter().any(|w| matches!(w.kind, generate::WarningKind::KonpuSingletonIdentity)),
        "expected KonpuSingletonIdentity warning, got: {:?}",
        result.warnings.iter().map(|w| &w.message).collect::<Vec<_>>()
    );

    // fun-form identity → no such warning
    let n2 = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir2 = std::env::temp_dir().join(format!("oxidtr_konpu_{n2}"));
    let _ = fs::remove_dir_all(&dir2);
    fs::create_dir_all(&dir2).unwrap();
    let model2 = dir2.join("model.als");
    fs::write(&model2, r#"
sig Money { amount: one Int }
fun zero: Money { Money }
fun add[a, b: Money]: Money { a }
fact Assoc { all a, b, c: Money | add[add[a, b], c] = add[a, add[b, c]] }
fact Ident { all a: Money | add[a, zero] = a and add[zero, a] = a }
"#).unwrap();
    let mut config2 = GenerateConfig::new("rust", dir2.to_str().unwrap());
    config2.warnings = generate::WarningLevel::Off;
    config2.konpu = true;
    let result2 = generate::run(model2.to_str().unwrap(), &config2).unwrap();
    assert!(
        !result2.warnings.iter().any(|w| matches!(w.kind, generate::WarningKind::KonpuSingletonIdentity)),
        "fun-form identity must not warn"
    );
}

const FUN_MONOID: &str = r#"
sig Money { amount: one Int }
fun zero: Money { Money }
fun add[a, b: Money]: Money { a }
fact Assoc { all a, b, c: Money | add[add[a, b], c] = add[a, add[b, c]] }
fact Ident { all a: Money | add[a, zero] = a and add[zero, a] = a }
"#;

fn gen_target(src: &str, target: &str, models_file: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("oxidtr_konpu_{n}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let model = dir.join("model.als");
    fs::write(&model, src).unwrap();
    let mut config = GenerateConfig::new(target, dir.to_str().unwrap());
    config.warnings = generate::WarningLevel::Off;
    config.konpu = true;
    generate::run(model.to_str().unwrap(), &config).unwrap();
    fs::read_to_string(dir.join(models_file)).unwrap()
}

#[test]
fn comment_directive_emitted_for_ts_swift_kotlin() {
    // Comment form: `// konpu: monoid(op: add, identity: zero)` directly above
    // the type declaration (konpu attaches it to the next named declaration).
    for (target, models_file, decl) in [
        ("typescript", "models.ts", "export interface Money"),
        ("swift", "Models.swift", "struct Money"),
        ("kotlin", "Models.kt", "data class Money"),
    ] {
        let out = gen_target(FUN_MONOID, target, models_file);
        let directive = "// konpu: monoid(op: add, identity: zero)";
        let d = out.find(directive).unwrap_or_else(|| {
            panic!("{target}: expected directive, got:\n{out}")
        });
        // The declaration must be the next non-empty line after the directive.
        let after = &out[d + directive.len()..];
        let next_line = after.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        assert!(
            next_line.trim_start().starts_with(decl),
            "{target}: directive must sit directly above `{decl}`, next line was: {next_line}"
        );
    }
}

#[test]
fn no_annotation_without_associativity() {
    // op + identity but no associativity fact → not even a semigroup
    let src = r#"
sig S { x: one Int }
one sig E extends S {}
fun combine[a, b: S]: S { a }
fact Ident { all a: S | combine[a, E] = a }
"#;
    let out = gen_rust(src, true);
    assert!(!out.contains("konpu::"), "no assoc → no emit, got:\n{out}");
}
