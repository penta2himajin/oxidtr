use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;

fn generate_from(input: &str) -> Vec<oxidtr::backend::GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    rust::generate(&ir)
}

fn find_file<'a>(files: &'a [oxidtr::backend::GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

#[test]
fn tc_lone_self_ref_generates_traversal_function() {
    // lone self-ref: parent: lone Node → Option<Box<Node>>
    // TC traversal follows the Option chain
    let files = generate_from(r#"
        sig Node { parent: lone Node }
        assert NoCycle { no n: Node | n in n.^parent }
    "#);
    let helpers = find_file(&files, "helpers.rs");

    // Should generate a specific traversal function, not generic
    assert!(
        helpers.contains("fn tc_parent("),
        "missing tc_parent function in:\n{helpers}"
    );
    // Should take &Node and return Vec<Node>
    assert!(
        helpers.contains("start: &Node") && helpers.contains("-> Vec<Node>"),
        "tc_parent should take &Node and return Vec<Node> in:\n{helpers}"
    );
    // Should use as_deref() for Option<Box<T>> traversal
    assert!(
        helpers.contains("as_deref()"),
        "tc_parent should use as_deref() for Option<Box<T>> in:\n{helpers}"
    );
}

#[test]
fn tc_lone_self_ref_test_compiles_with_correct_types() {
    let files = generate_from(r#"
        sig Node { parent: lone Node }
        assert NoCycle { no n: Node | n in n.^parent }
    "#);
    let tests = find_file(&files, "tests.rs");

    // Should call tc_parent, not generic transitive_closure
    assert!(
        tests.contains("tc_parent("),
        "test should call tc_parent in:\n{tests}"
    );
    // Should NOT contain generic transitive_closure
    assert!(
        !tests.contains("transitive_closure("),
        "should not use generic transitive_closure in:\n{tests}"
    );
}

#[test]
fn tc_set_self_ref_generates_bfs_traversal() {
    // set self-ref: children: set Node → Vec<Node>
    // TC traversal does BFS/DFS over the collection
    let files = generate_from(r#"
        sig Node { children: set Node }
        assert NoCycle { no n: Node | n in n.^children }
    "#);
    let helpers = find_file(&files, "helpers.rs");

    assert!(
        helpers.contains("fn tc_children("),
        "missing tc_children function in:\n{helpers}"
    );
    assert!(
        helpers.contains("start: &Node") && helpers.contains("-> Vec<Node>"),
        "tc_children should take &Node and return Vec<Node> in:\n{helpers}"
    );
}

#[test]
fn tc_in_assert_translates_to_contains() {
    // `n in n.^parent` should become `tc_parent(&n).contains(&n)`
    let files = generate_from(r#"
        sig Node { parent: lone Node }
        assert NoCycle { no n: Node | n in n.^parent }
    "#);
    let tests = find_file(&files, "tests.rs");

    assert!(
        tests.contains(".contains("),
        "TC in-check should use .contains() in:\n{tests}"
    );
}

#[test]
fn tc_self_hosting_model_no_generic_transitive_closure() {
    // The self-hosting model uses ^parent on SigDecl
    // It should generate tc_parent for SigDecl, not generic transitive_closure
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).expect("parse");
    let ir_result = ir::lower(&model).expect("lower");
    let files = rust::generate(&ir_result);

    let helpers = find_file(&files, "helpers.rs");
    assert!(
        !helpers.contains("fn transitive_closure<T>"),
        "should not generate generic transitive_closure in:\n{helpers}"
    );
    assert!(
        helpers.contains("fn tc_parent("),
        "should generate tc_parent for SigDecl.parent in:\n{helpers}"
    );
}
