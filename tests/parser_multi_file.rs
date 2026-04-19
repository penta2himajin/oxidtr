use oxidtr::parser;
use std::fs;

/// Create a temp dir inside cargo's target dir (self-contained, no external deps).
fn fresh_tmp_dir(name: &str) -> std::path::PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("oxidtr-multifile-{}-{}", name, std::process::id()));
    if d.exists() {
        let _ = fs::remove_dir_all(&d);
    }
    fs::create_dir_all(&d).expect("create tmp dir");
    d
}

#[test]
fn parse_records_open_directives_as_imports() {
    let src = r#"
module foo

open bar
open baz/qux as Q

sig A {}
"#;
    let model = parser::parse(src).expect("parse");
    assert_eq!(model.module_decl.as_deref(), Some("foo"));
    assert_eq!(model.imports.len(), 2);
    assert_eq!(model.imports[0].path, "bar");
    assert_eq!(model.imports[0].alias, None);
    assert_eq!(model.imports[1].path, "baz/qux");
    assert_eq!(model.imports[1].alias.as_deref(), Some("Q"));
}

#[test]
fn parse_slash_separated_module_path() {
    let src = "module oxidtr/ast\n\nsig A {}\n";
    let model = parser::parse(src).expect("parse");
    assert_eq!(model.module_decl.as_deref(), Some("oxidtr/ast"));
    assert_eq!(model.sigs.len(), 1);
    assert_eq!(model.sigs[0].module.as_deref(), Some("oxidtr/ast"));
}

#[test]
fn parse_open_with_parameter_brackets_is_skipped() {
    // parameterized modules like `open util/ordering[Time]` must parse
    // (we record them but cannot resolve the file)
    let src = "module m\n\nopen util/ordering[Time]\n\nsig S {}\n";
    let model = parser::parse(src).expect("parse");
    assert_eq!(model.imports.len(), 1);
    assert_eq!(model.imports[0].path, "util/ordering");
    assert_eq!(model.sigs.len(), 1);
}

#[test]
fn parse_from_path_merges_opened_files() {
    let dir = fresh_tmp_dir("merge");
    let sub = dir.join("sub");
    fs::create_dir_all(&sub).unwrap();

    fs::write(
        dir.join("main.als"),
        "module main\nopen sub/child\n\nsig Root { child: one Leaf }\n",
    )
    .unwrap();
    fs::write(
        sub.join("child.als"),
        "module sub/child\n\nsig Leaf {}\n",
    )
    .unwrap();

    let model = parser::parse_from_path(&dir.join("main.als")).expect("parse_from_path");
    let names: Vec<&str> = model.sigs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Root"), "sigs: {names:?}");
    assert!(names.contains(&"Leaf"), "sigs: {names:?}");

    // qualified module names preserved per file
    let leaf = model.sigs.iter().find(|s| s.name == "Leaf").unwrap();
    assert_eq!(leaf.module.as_deref(), Some("sub/child"));
    let root = model.sigs.iter().find(|s| s.name == "Root").unwrap();
    assert_eq!(root.module.as_deref(), Some("main"));
}

#[test]
fn parse_from_path_cycle_does_not_infinite_loop() {
    let dir = fresh_tmp_dir("cycle");
    fs::write(
        dir.join("a.als"),
        "module a\nopen b\n\nsig A {}\n",
    )
    .unwrap();
    fs::write(
        dir.join("b.als"),
        "module b\nopen a\n\nsig B {}\n",
    )
    .unwrap();

    let model = parser::parse_from_path(&dir.join("a.als")).expect("parse_from_path");
    let names: Vec<&str> = model.sigs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"A"));
    assert!(names.contains(&"B"));
}

#[test]
fn parse_from_path_missing_open_is_non_fatal() {
    let dir = fresh_tmp_dir("missing");
    fs::write(
        dir.join("main.als"),
        "module main\nopen util/ordering[Time]\n\nsig S {}\n",
    )
    .unwrap();
    // Should not error out — library/parameterized opens without a local file
    // are skipped with a stderr notice.
    let model = parser::parse_from_path(&dir.join("main.als")).expect("parse_from_path");
    assert_eq!(model.sigs.len(), 1);
}

#[test]
fn legacy_midfile_module_backward_compat() {
    // Existing oxidtr.als uses mid-file `module X` to group sigs.
    // The first declaration remains the file-level header; subsequent
    // ones simply switch the grouping context for backward compat.
    let src = "module ast\n\nsig A {}\n\nmodule ir\n\nsig B {}\n";
    let model = parser::parse(src).expect("parse");
    assert_eq!(model.module_decl.as_deref(), Some("ast"));
    let a = model.sigs.iter().find(|s| s.name == "A").unwrap();
    let b = model.sigs.iter().find(|s| s.name == "B").unwrap();
    assert_eq!(a.module.as_deref(), Some("ast"));
    assert_eq!(b.module.as_deref(), Some("ir"));
}
