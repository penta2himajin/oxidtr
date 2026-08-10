//! `generate` must be a pure function of the model.
//!
//! "全コマンドが決定的 — AI非依存、確率的要素なし" is the project's first design
//! principle, and nothing enforced it: CI generates each target exactly once, so
//! run-to-run instability was invisible. Seven of the eight backends grouped
//! anomaly patterns in a `HashMap` and wrote the groups straight out, and Rust
//! seeds every `HashMap` differently, so the emitted test order changed on every
//! run while the *set* of tests stayed the same.
//!
//! Byte-identical output is also what makes "this refactor changed no output" a
//! checkable claim rather than an assertion.

use oxidtr::backend::{self, GeneratedFile};
use oxidtr::ir;
use oxidtr::parser;

const SELF_HOSTING_MODEL: &str = include_str!("../models/oxidtr.als");

fn lower_self_hosting_model() -> ir::nodes::OxidtrIR {
    let model = parser::parse(SELF_HOSTING_MODEL).expect("self-hosting model should parse");
    ir::lower(&model).expect("self-hosting model should lower")
}

fn render(files: &[GeneratedFile]) -> Vec<(String, String)> {
    files.iter().map(|f| (f.path.clone(), f.content.clone())).collect()
}

/// Two generations from one IR must agree byte for byte, path order included.
///
/// Both runs happen in the same process on purpose: Rust gives each `HashMap`
/// its own seed, so a single process is enough to expose the instability — no
/// re-exec, no sleeping, no flake.
fn assert_generates_identically(label: &str, gen: fn(&ir::nodes::OxidtrIR) -> Vec<GeneratedFile>) {
    let ir = lower_self_hosting_model();
    let first = render(&gen(&ir));
    let second = render(&gen(&ir));

    assert_eq!(
        first.len(),
        second.len(),
        "{label}: run 1 emitted {} file(s), run 2 emitted {}",
        first.len(),
        second.len()
    );

    for ((p1, c1), (p2, c2)) in first.iter().zip(second.iter()) {
        assert_eq!(p1, p2, "{label}: file order differs between runs");
        if c1 != c2 {
            let line = c1
                .lines()
                .zip(c2.lines())
                .position(|(a, b)| a != b)
                .map(|i| i + 1);
            panic!(
                "{label}: {p1} differs between two runs of the same generator \
                 (first differing line: {line:?}). generate must be deterministic."
            );
        }
    }
}

#[test]
fn rust_generates_identically_twice() {
    assert_generates_identically("rust", backend::rust::generate);
}

#[test]
fn typescript_generates_identically_twice() {
    assert_generates_identically("ts", backend::typescript::generate);
}

#[test]
fn kotlin_generates_identically_twice() {
    assert_generates_identically("kt", backend::jvm::kotlin::generate);
}

#[test]
fn java_generates_identically_twice() {
    assert_generates_identically("java", backend::jvm::java::generate);
}

#[test]
fn swift_generates_identically_twice() {
    assert_generates_identically("swift", backend::swift::generate);
}

#[test]
fn go_generates_identically_twice() {
    assert_generates_identically("go", backend::go::generate);
}

#[test]
fn csharp_generates_identically_twice() {
    assert_generates_identically("cs", backend::csharp::generate);
}

#[test]
fn lean_generates_identically_twice() {
    assert_generates_identically("lean", backend::lean::generate);
}
