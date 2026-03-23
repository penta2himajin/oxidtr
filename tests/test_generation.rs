use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::rust;

fn generate_from(input: &str) -> Vec<rust::GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    rust::generate(&ir)
}

fn find_file<'a>(files: &'a [rust::GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found in {:?}", files.iter().map(|f| &f.path).collect::<Vec<_>>()))
}

#[test]
fn generate_invariant_function_from_fact() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact UserHasRole { all u: User | u.role = u.role }
    "#);
    let content = find_file(&files, "invariants.rs");
    // Should contain an assertion function
    assert!(content.contains("fn assert_user_has_role"), "missing invariant function");
    // Should contain translated expression, not todo
    assert!(content.contains(".iter().all("), "missing translated quantifier");
    assert!(!content.contains("todo!"), "should not contain todo for simple facts");
}

#[test]
fn generate_invariant_with_implies() {
    let files = generate_from(r#"
        sig User { role: one Role, owns: set Resource }
        sig Role {}
        sig Resource {}
        fact AdminOwnsNothing {
            all u: User | u.role = u.role implies #u.owns = #u.owns
        }
    "#);
    let content = find_file(&files, "invariants.rs");
    assert!(content.contains("fn assert_admin_owns_nothing"));
    assert!(content.contains(".len()"), "missing cardinality translation");
}

#[test]
fn generate_property_test_from_assert() {
    let files = generate_from(r#"
        sig A {}
        assert NoSelfRef { all a: A | a = a }
    "#);
    let content = find_file(&files, "tests.rs");
    assert!(content.contains("fn no_self_ref") || content.contains("fn prop_no_self_ref"));
    assert!(content.contains(".iter().all("), "missing translated expression in test");
}

#[test]
fn generate_operation_pre_post_conditions() {
    let files = generate_from(r#"
        sig Account { balance: one Account }
        pred withdraw[a: one Account, amount: one Account] {
            a.balance = a.balance
        }
    "#);
    let content = find_file(&files, "operations.rs");
    assert!(content.contains("fn withdraw"));
    // Operations still have todo!() bodies — humans/AI fill these
    assert!(content.contains("todo!"));
}

#[test]
fn generate_cross_test_fact_times_operation() {
    let files = generate_from(r#"
        sig User { role: one Role }
        sig Role {}
        fact UserHasRole { all u: User | u.role = u.role }
        pred changeRole[u: one User, r: one Role] { u.role = r }
    "#);
    let content = find_file(&files, "tests.rs");
    // Should have a cross-test that verifies fact preservation after operation
    assert!(
        content.contains("user_has_role") && content.contains("change_role"),
        "missing cross-test for fact×operation"
    );
}
