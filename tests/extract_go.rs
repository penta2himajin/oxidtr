use oxidtr::extract::go_extractor;
use oxidtr::extract::MinedMultiplicity;

#[test]
fn go_extract_struct() {
    let model = go_extractor::extract(r#"
package models

type User struct {
    Name string
    Age  int
}
"#);
    assert_eq!(model.sigs.len(), 1);
    let sig = &model.sigs[0];
    assert_eq!(sig.name, "User");
    assert_eq!(sig.fields.len(), 2);
    assert_eq!(sig.fields[0].name, "name");
    assert_eq!(sig.fields[0].target, "string");
    assert_eq!(sig.fields[0].mult, MinedMultiplicity::One);
}

#[test]
fn go_extract_pointer() {
    let model = go_extractor::extract(r#"
package models

type Node struct {
    Parent *Node
}
"#);
    let sig = &model.sigs[0];
    let f = &sig.fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Lone);
    assert_eq!(f.target, "Node");
}

#[test]
fn go_extract_slice() {
    let model = go_extractor::extract(r#"
package models

type Group struct {
    Members []User
}
"#);
    let f = &model.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Set);
    assert_eq!(f.target, "User");
}

#[test]
fn go_extract_iota_enum() {
    let model = go_extractor::extract(r#"
package models

type Status int

const (
    Active Status = iota
    Inactive
)
"#);
    let status = model.sigs.iter().find(|s| s.name == "Status").expect("Status sig");
    assert!(status.is_abstract);
    let active = model.sigs.iter().find(|s| s.name == "Active").expect("Active sig");
    assert_eq!(active.parent.as_deref(), Some("Status"));
    let inactive = model.sigs.iter().find(|s| s.name == "Inactive").expect("Inactive sig");
    assert_eq!(inactive.parent.as_deref(), Some("Status"));
}

#[test]
fn go_extract_interface_enum() {
    let model = go_extractor::extract(r#"
package models

type Expr interface {
    isExpr()
}

type Literal struct{}

func (Literal) isExpr() {}

type BinOp struct {
    Left  Expr
    Right Expr
}

func (BinOp) isExpr() {}
"#);
    let expr = model.sigs.iter().find(|s| s.name == "Expr").expect("Expr sig");
    assert!(expr.is_abstract);
    let binop = model.sigs.iter().find(|s| s.name == "BinOp").expect("BinOp sig");
    assert_eq!(binop.parent.as_deref(), Some("Expr"));
    assert_eq!(binop.fields.len(), 2);
}

#[test]
fn go_extract_nil_check_fact() {
    let model = go_extractor::extract(r#"
package models

func validate(x *int) {
    if x == nil {
        return
    }
}
"#);
    assert!(!model.fact_candidates.is_empty());
    assert!(model.fact_candidates.iter().any(|f| f.source_pattern == "nil check"));
}

#[test]
fn go_extract_panic_fact() {
    let model = go_extractor::extract(r#"
package models

func mustPositive(x int) {
    if x <= 0 {
        panic("must be positive")
    }
}
"#);
    assert!(model.fact_candidates.iter().any(|f| f.source_pattern == "panic"));
}

#[test]
fn go_reverse_translate_basic() {
    assert_eq!(
        go_extractor::reverse_translate_expr("x == y"),
        Some("x = y".to_string()),
    );
    assert_eq!(
        go_extractor::reverse_translate_expr("len(items)"),
        Some("#items".to_string()),
    );
    assert_eq!(
        go_extractor::reverse_translate_expr("contains(items, x)"),
        Some("x in items".to_string()),
    );
}
