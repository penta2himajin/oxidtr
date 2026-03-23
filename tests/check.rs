/// Tests for the check command.
/// TDD: these tests define expected behavior before full implementation.

use oxidtr::check::{self, CheckConfig};
use oxidtr::check::impl_parser::{self, ExtractedField};
use oxidtr::check::differ::{self, DiffItem};
use oxidtr::ir::nodes::{OxidtrIR, StructureNode, IRField, OperationNode};
use oxidtr::parser::ast::{Multiplicity, SigMultiplicity};

// ── impl_parser: type_to_mult ─────────────────────────────────────────────────

#[test]
fn type_to_mult_one() {
    let (mult, target) = impl_parser::type_to_mult("User");
    assert_eq!(mult, Multiplicity::One);
    assert_eq!(target, "User");
}

#[test]
fn type_to_mult_lone() {
    let (mult, target) = impl_parser::type_to_mult("Option<User>");
    assert_eq!(mult, Multiplicity::Lone);
    assert_eq!(target, "User");
}

#[test]
fn type_to_mult_set() {
    let (mult, target) = impl_parser::type_to_mult("BTreeSet<User>");
    assert_eq!(mult, Multiplicity::Set);
    assert_eq!(target, "User");
}

#[test]
fn type_to_mult_seq() {
    let (mult, target) = impl_parser::type_to_mult("Vec<User>");
    assert_eq!(mult, Multiplicity::Seq);
    assert_eq!(target, "User");
}

#[test]
fn type_to_mult_lone_box() {
    // self-referential lone field
    let (mult, target) = impl_parser::type_to_mult("Option<Box<Node>>");
    assert_eq!(mult, Multiplicity::Lone);
    assert_eq!(target, "Node");
}

// ── impl_parser: parse_impl structs ──────────────────────────────────────────

#[test]
fn parse_impl_simple_struct() {
    let models_src = r#"
#[derive(Debug, Clone)]
pub struct User {
    pub name: String,
    pub manager: Option<User>,
    pub reports: Vec<User>,
}
"#;
    let result = impl_parser::parse_impl(models_src, "");
    assert_eq!(result.structs.len(), 1);
    let s = &result.structs[0];
    assert_eq!(s.name, "User");
    assert!(!s.is_enum);

    let manager = s.fields.iter().find(|f| f.name == "manager").expect("manager field");
    assert_eq!(manager.mult, Multiplicity::Lone);
    assert_eq!(manager.target, "User");

    let reports = s.fields.iter().find(|f| f.name == "reports").expect("reports field");
    assert_eq!(reports.mult, Multiplicity::Seq);
    assert_eq!(reports.target, "User");
}

#[test]
fn parse_impl_enum() {
    let models_src = r#"
#[derive(Debug, Clone)]
pub enum Status {
    Active,
    Inactive,
}
"#;
    let result = impl_parser::parse_impl(models_src, "");
    // enum itself + 2 variants extracted as structs
    assert_eq!(result.structs.len(), 3);
    assert_eq!(result.structs[0].name, "Status");
    assert!(result.structs[0].is_enum);
    assert_eq!(result.structs[1].name, "Active");
    assert!(!result.structs[1].is_enum);
    assert_eq!(result.structs[2].name, "Inactive");
    assert!(!result.structs[2].is_enum);
}

#[test]
fn parse_impl_multiple_structs() {
    let models_src = r#"
pub struct User {
    pub group: Group,
}

pub struct Group {
    pub members: Vec<User>,
}
"#;
    let result = impl_parser::parse_impl(models_src, "");
    assert_eq!(result.structs.len(), 2);
    let names: Vec<&str> = result.structs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"User"));
    assert!(names.contains(&"Group"));
}

// ── impl_parser: parse_impl fns ──────────────────────────────────────────────

#[test]
fn parse_impl_fns() {
    let ops_src = r#"
pub fn add_user(user: User) -> Result<(), String> {
    todo!()
}

pub fn remove_user(user: &User) -> Result<(), String> {
    todo!()
}
"#;
    let result = impl_parser::parse_impl("", ops_src);
    assert_eq!(result.fns.len(), 2);
    let names: Vec<&str> = result.fns.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"add_user"));
    assert!(names.contains(&"remove_user"));
}

// ── differ ────────────────────────────────────────────────────────────────────

fn make_ir(structs: Vec<StructureNode>, ops: Vec<OperationNode>) -> OxidtrIR {
    OxidtrIR {
        structures: structs,
        constraints: vec![],
        operations: ops,
        properties: vec![],
    }
}

#[test]
fn differ_no_diff_when_in_sync() {
    use oxidtr::check::impl_parser::{ExtractedImpl, ExtractedStruct};
    let ir = make_ir(
        vec![StructureNode {
            name: "User".into(),
            is_enum: false,
            sig_multiplicity: SigMultiplicity::Default,
            parent: None,
            fields: vec![IRField {
                name: "name".into(),
                mult: Multiplicity::One,
                target: "String".into(),
            }],
        }],
        vec![],
    );
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct {
            name: "User".into(),
            is_enum: false,
            fields: vec![ExtractedField {
                name: "name".into(),
                mult: Multiplicity::One,
                target: "String".into(),
            }],
        }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.is_empty(), "expected no diffs, got: {diffs:?}");
}

#[test]
fn differ_missing_struct() {
    use oxidtr::check::impl_parser::ExtractedImpl;
    let ir = make_ir(
        vec![StructureNode { name: "User".into(), is_enum: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![] }],
        vec![],
    );
    let extracted = ExtractedImpl { structs: vec![], fns: vec![] };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MissingStruct { name: "User".into() }));
}

#[test]
fn differ_extra_struct() {
    use oxidtr::check::impl_parser::{ExtractedImpl, ExtractedStruct};
    let ir = make_ir(vec![], vec![]);
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct { name: "Ghost".into(), is_enum: false, fields: vec![] }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::ExtraStruct { name: "Ghost".into() }));
}

#[test]
fn differ_missing_field() {
    use oxidtr::check::impl_parser::{ExtractedImpl, ExtractedStruct};
    let ir = make_ir(
        vec![StructureNode {
            name: "User".into(),
            is_enum: false,
            sig_multiplicity: SigMultiplicity::Default,
            parent: None,
            fields: vec![IRField { name: "email".into(), mult: Multiplicity::One, target: "String".into() }],
        }],
        vec![],
    );
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct { name: "User".into(), is_enum: false, fields: vec![] }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MissingField {
        struct_name: "User".into(),
        field_name: "email".into(),
    }));
}

#[test]
fn differ_extra_field() {
    use oxidtr::check::impl_parser::{ExtractedImpl, ExtractedStruct, ExtractedField};
    let ir = make_ir(
        vec![StructureNode { name: "User".into(), is_enum: false, sig_multiplicity: SigMultiplicity::Default, parent: None, fields: vec![] }],
        vec![],
    );
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct {
            name: "User".into(),
            is_enum: false,
            fields: vec![ExtractedField { name: "phantom".into(), mult: Multiplicity::One, target: "String".into() }],
        }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::ExtraField {
        struct_name: "User".into(),
        field_name: "phantom".into(),
    }));
}

#[test]
fn differ_multiplicity_mismatch() {
    use oxidtr::check::impl_parser::{ExtractedImpl, ExtractedStruct, ExtractedField};
    let ir = make_ir(
        vec![StructureNode {
            name: "User".into(),
            is_enum: false,
            sig_multiplicity: SigMultiplicity::Default,
            parent: None,
            fields: vec![IRField { name: "manager".into(), mult: Multiplicity::Lone, target: "User".into() }],
        }],
        vec![],
    );
    // impl has One instead of Lone
    let extracted = ExtractedImpl {
        structs: vec![ExtractedStruct {
            name: "User".into(),
            is_enum: false,
            fields: vec![ExtractedField { name: "manager".into(), mult: Multiplicity::One, target: "User".into() }],
        }],
        fns: vec![],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MultiplicityMismatch {
        struct_name: "User".into(),
        field_name: "manager".into(),
        expected: Multiplicity::Lone,
        actual: Multiplicity::One,
    }));
}

#[test]
fn differ_missing_fn() {
    use oxidtr::check::impl_parser::ExtractedImpl;
    let ir = make_ir(
        vec![],
        vec![OperationNode { name: "add_user".into(), params: vec![], return_type: None, body: vec![] }],
    );
    let extracted = ExtractedImpl { structs: vec![], fns: vec![] };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::MissingFn { name: "add_user".into() }));
}

#[test]
fn differ_extra_fn() {
    use oxidtr::check::impl_parser::{ExtractedImpl, ExtractedFn};
    let ir = make_ir(vec![], vec![]);
    let extracted = ExtractedImpl {
        structs: vec![],
        fns: vec![ExtractedFn { name: "orphan_fn".into() }],
    };
    let diffs = differ::diff(&ir, &extracted);
    assert!(diffs.contains(&DiffItem::ExtraFn { name: "orphan_fn".into() }));
}

// ── integration: check::run ───────────────────────────────────────────────────

#[test]
fn check_run_in_sync() {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("src");
    fs::create_dir_all(&impl_dir).unwrap();

    fs::write(&model_path, r#"
sig User {
    manager: lone User
}
pred add_user[u: User] {}
"#).unwrap();

    fs::write(impl_dir.join("models.rs"), r#"
pub struct User {
    pub manager: Option<User>,
}
"#).unwrap();

    fs::write(impl_dir.join("operations.rs"), r#"
pub fn add_user(u: &User) -> Result<(), String> { todo!() }
"#).unwrap();

    let result = check::run(
        model_path.to_str().unwrap(),
        &CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() },
    ).unwrap();

    assert!(result.is_ok(), "expected no diffs, got: {:?}", result.diffs);
}

#[test]
fn check_run_detects_missing_struct() {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("src");
    fs::create_dir_all(&impl_dir).unwrap();

    fs::write(&model_path, r#"sig User {} sig Group {}"#).unwrap();
    // Group is missing from impl
    fs::write(impl_dir.join("models.rs"), r#"pub struct User {}"#).unwrap();

    let result = check::run(
        model_path.to_str().unwrap(),
        &CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() },
    ).unwrap();

    assert!(!result.is_ok());
    assert!(result.diffs.iter().any(|d| matches!(
        d, DiffItem::MissingStruct { name } if name == "Group"
    )));
}

#[test]
fn check_run_missing_models_rs_is_error() {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("model.als");
    let impl_dir = dir.path().join("src");
    fs::create_dir_all(&impl_dir).unwrap();
    fs::write(&model_path, "sig User {}").unwrap();
    // models.rs not created

    let err = check::run(
        model_path.to_str().unwrap(),
        &CheckConfig { impl_dir: impl_dir.to_str().unwrap().to_string() },
    );
    assert!(matches!(err, Err(check::CheckError::ImplNotFound(_))));
}
