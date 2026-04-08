use oxidtr::extract::rust_extractor;
use oxidtr::extract::renderer;
use oxidtr::extract::{MinedMultiplicity, Confidence, resolve_external_types, is_language_primitive, is_type_parameter};

#[test]
fn mine_rust_struct_to_sig() {
    let src = r#"
pub struct User {
    pub name: String,
    pub age: i32,
}
"#;
    let mined = rust_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 1);
    assert_eq!(mined.sigs[0].name, "User");
    assert_eq!(mined.sigs[0].fields.len(), 2);
    assert!(!mined.sigs[0].is_abstract);
}

#[test]
fn mine_rust_option_to_lone() {
    let src = r#"
pub struct Node {
    pub parent: Option<Node>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.name, "parent");
    assert_eq!(f.mult, MinedMultiplicity::Lone);
    assert_eq!(f.target, "Node");
}

#[test]
fn mine_rust_option_box_to_lone() {
    let src = r#"
pub struct Node {
    pub parent: Option<Box<Node>>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Lone);
    assert_eq!(f.target, "Node");
}

#[test]
fn mine_rust_vec_to_seq() {
    let src = r#"
pub struct Group {
    pub members: Vec<User>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Seq);
    assert_eq!(f.target, "User");
}

#[test]
fn mine_rust_btreeset_to_set() {
    let src = r#"
pub struct Group {
    pub members: BTreeSet<User>,
}
"#;
    let mined = rust_extractor::extract(src);
    let f = &mined.sigs[0].fields[0];
    assert_eq!(f.mult, MinedMultiplicity::Set);
    assert_eq!(f.target, "User");
}

#[test]
fn mine_rust_enum_to_abstract_sig() {
    let src = r#"
pub enum Status {
    Active,
    Inactive,
}
"#;
    let mined = rust_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 3);
    assert!(mined.sigs[0].is_abstract);
    assert_eq!(mined.sigs[0].name, "Status");
    assert_eq!(mined.sigs[1].name, "Active");
    assert_eq!(mined.sigs[1].parent.as_deref(), Some("Status"));
    assert_eq!(mined.sigs[2].name, "Inactive");
    assert_eq!(mined.sigs[2].parent.as_deref(), Some("Status"));
}

#[test]
fn mine_rust_enum_with_fields() {
    let src = r#"
pub enum Expr {
    Literal,
    BinOp {
        left: Box<Expr>,
        right: Box<Expr>,
    },
}
"#;
    let mined = rust_extractor::extract(src);
    assert_eq!(mined.sigs.len(), 3); // Expr, Literal, BinOp
    let binop = &mined.sigs[2];
    assert_eq!(binop.name, "BinOp");
    assert_eq!(binop.fields.len(), 2);
    assert_eq!(binop.fields[0].name, "left");
    assert_eq!(binop.fields[0].target, "Expr");
}

#[test]
fn mine_rust_assert_fact_candidate() {
    let src = r#"
pub fn validate(items: &[Item]) {
    assert!(!items.is_empty());
}
"#;
    let mined = rust_extractor::extract(src);
    assert!(!mined.fact_candidates.is_empty());
    assert!(mined.fact_candidates.iter().any(|f| f.confidence == Confidence::High));
}

#[test]
fn mine_rust_contains_fact_candidate() {
    let src = r#"
pub fn check(group: &Group, user: &User) {
    if group.members.contains(&user) {
        return;
    }
}
"#;
    let mined = rust_extractor::extract(src);
    assert!(mined.fact_candidates.iter().any(|f|
        f.confidence == Confidence::Medium && f.source_pattern.contains("contains")
    ));
}

#[test]
fn mine_rust_renderer_produces_valid_alloy() {
    let src = r#"
pub struct User {
    pub name: String,
    pub group: Option<Group>,
    pub roles: Vec<Role>,
}
pub struct Group {}
pub struct Role {}
"#;
    let mined = rust_extractor::extract(src);
    let rendered = renderer::render(&mined);

    // Should be parseable by our parser
    let result = oxidtr::parser::parse(&rendered);
    assert!(result.is_ok(), "rendered Alloy should parse: {:?}\n---\n{rendered}", result.err());
    let model = result.unwrap();
    assert_eq!(model.sigs.len(), 3);
}

#[test]
fn mine_rust_round_trip_struct_preserves_structure() {
    // Generate Rust from a simple model, then mine it back
    let alloy_src = r#"
sig User { group: lone Group, roles: set Role }
sig Group {}
sig Role {}
"#;
    let model = oxidtr::parser::parse(alloy_src).unwrap();
    let ir = oxidtr::ir::lower(&model).unwrap();
    let files = oxidtr::backend::rust::generate(&ir);
    let models_rs = files.iter().find(|f| f.path == "models.rs").unwrap();

    // Mine from generated Rust
    let mined = rust_extractor::extract(&models_rs.content);

    // Should recover the same sigs
    assert_eq!(mined.sigs.len(), 3);
    let user = mined.sigs.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.fields.len(), 2);
    let group_field = user.fields.iter().find(|f| f.name == "group").unwrap();
    assert_eq!(group_field.mult, MinedMultiplicity::Lone);
    assert_eq!(group_field.target, "Group");
    let roles_field = user.fields.iter().find(|f| f.name == "roles").unwrap();
    assert_eq!(roles_field.mult, MinedMultiplicity::Set);
    assert_eq!(roles_field.target, "Role");
}

#[test]
fn resolve_external_types_adds_placeholder_sigs() {
    let src = r#"
pub struct User {
    pub name: String,
    pub age: usize,
    pub group: Group,
    pub config: AppConfig,
}

pub struct Group {
    pub title: String,
}
"#;
    let mut mined = rust_extractor::extract(src);
    resolve_external_types(&mut mined);

    let sig_names: Vec<&str> = mined.sigs.iter().map(|s| s.name.as_str()).collect();

    // String and usize are language primitives — should NOT be added
    assert!(!sig_names.contains(&"String"), "String is a primitive, should not be added");
    assert!(!sig_names.contains(&"usize"), "usize is a primitive, should not be added");

    // AppConfig is a user-defined type not in this file — should be added as placeholder
    assert!(sig_names.contains(&"AppConfig"), "AppConfig should be added as placeholder sig");

    // Group IS defined — should NOT be duplicated
    assert_eq!(sig_names.iter().filter(|&&n| n == "Group").count(), 1);

    // Placeholder sigs should have no fields and not be abstract
    let config_sig = mined.sigs.iter().find(|s| s.name == "AppConfig").unwrap();
    assert!(config_sig.fields.is_empty());
    assert!(!config_sig.is_abstract);
}

#[test]
fn resolve_external_types_skips_self_referencing() {
    let src = r#"
pub struct Node {
    pub parent: Option<Box<Node>>,
}
"#;
    let mut mined = rust_extractor::extract(src);
    resolve_external_types(&mut mined);

    // Node references itself — should NOT create a duplicate
    assert_eq!(mined.sigs.len(), 1);
    assert_eq!(mined.sigs[0].name, "Node");
}

#[test]
fn rendered_als_excludes_primitives_after_resolve() {
    let src = r#"
pub struct Config {
    pub name: String,
    pub count: usize,
    pub owner: Owner,
}
"#;
    let mut mined = rust_extractor::extract(src);
    resolve_external_types(&mut mined);
    let rendered = renderer::render(&mined);

    // Primitives should NOT appear as sig definitions
    assert!(!rendered.contains("sig String {"), "rendered should NOT define sig String");
    assert!(!rendered.contains("sig usize {"), "rendered should NOT define sig usize");

    // User-defined external types should still be added
    assert!(rendered.contains("sig Owner {"), "rendered should define sig Owner");
}

#[test]
fn is_language_primitive_covers_all_languages() {
    // Rust primitives
    assert!(is_language_primitive("String"));
    assert!(is_language_primitive("bool"));
    assert!(is_language_primitive("i32"));
    assert!(is_language_primitive("u64"));
    assert!(is_language_primitive("usize"));
    assert!(is_language_primitive("isize"));
    assert!(is_language_primitive("f64"));
    assert!(is_language_primitive("char"));
    assert!(is_language_primitive("str"));

    // TypeScript primitives
    assert!(is_language_primitive("string"));
    assert!(is_language_primitive("number"));
    assert!(is_language_primitive("boolean"));
    assert!(is_language_primitive("any"));
    assert!(is_language_primitive("unknown"));
    assert!(is_language_primitive("never"));
    assert!(is_language_primitive("void"));
    assert!(is_language_primitive("null"));
    assert!(is_language_primitive("undefined"));
    assert!(is_language_primitive("object"));
    assert!(is_language_primitive("bigint"));

    // Kotlin/Java primitives
    assert!(is_language_primitive("Int"));
    assert!(is_language_primitive("Long"));
    assert!(is_language_primitive("Short"));
    assert!(is_language_primitive("Byte"));
    assert!(is_language_primitive("Float"));
    assert!(is_language_primitive("Double"));
    assert!(is_language_primitive("Boolean"));
    assert!(is_language_primitive("Unit"));
    assert!(is_language_primitive("Nothing"));
    assert!(is_language_primitive("Any"));
    assert!(is_language_primitive("Integer"));
    assert!(is_language_primitive("Character"));

    // Swift primitives
    assert!(is_language_primitive("Int8"));
    assert!(is_language_primitive("UInt"));
    assert!(is_language_primitive("Bool"));

    // Go primitives
    assert!(is_language_primitive("int"));
    assert!(is_language_primitive("int64"));
    assert!(is_language_primitive("uint"));
    assert!(is_language_primitive("float32"));
    assert!(is_language_primitive("float64"));
    assert!(is_language_primitive("byte"));
    assert!(is_language_primitive("rune"));
    assert!(is_language_primitive("error"));
    assert!(is_language_primitive("complex64"));
    assert!(is_language_primitive("complex128"));

    // Boolean literal types (subtypes of Boolean)
    assert!(is_language_primitive("true"));
    assert!(is_language_primitive("false"));

    // User-defined types should NOT be primitives
    assert!(!is_language_primitive("User"));
    assert!(!is_language_primitive("AppConfig"));
    assert!(!is_language_primitive("MyService"));
}

#[test]
fn is_type_parameter_detects_generic_params() {
    // Single uppercase letter — typical type parameters
    assert!(is_type_parameter("T"));
    assert!(is_type_parameter("S"));
    assert!(is_type_parameter("U"));
    assert!(is_type_parameter("K"));
    assert!(is_type_parameter("V"));
    assert!(is_type_parameter("E"));
    assert!(is_type_parameter("R"));

    // All-uppercase 2 chars — likely type parameters (IO, ID, etc.)
    assert!(is_type_parameter("IO"));
    assert!(is_type_parameter("ID"));

    // Mixed case 2 chars — likely user-defined types, NOT type params
    assert!(!is_type_parameter("Go"));
    assert!(!is_type_parameter("Of"));
    assert!(!is_type_parameter("Io"));

    // Longer names — NOT type parameters
    assert!(!is_type_parameter("User"));
    assert!(!is_type_parameter("ABC"));
    assert!(!is_type_parameter("Config"));

    // Lowercase single letter — not a conventional type param
    assert!(!is_type_parameter("t"));
    assert!(!is_type_parameter("a"));
}

#[test]
fn resolve_external_types_filters_type_parameters() {
    // Simulate what happens when mine extracts from generic code:
    // fields with target "T" or "S" should not become placeholder sigs
    let mut model = oxidtr::extract::MinedModel {
        sigs: vec![
            oxidtr::extract::MinedSig {
                name: "Container".to_string(),
                is_var: false,
                fields: vec![
                    oxidtr::extract::MinedField {
                        name: "value".to_string(),
                        is_var: false,
                        target: "T".to_string(),
                        mult: MinedMultiplicity::One,
                        raw_union_type: None,
                    },
                    oxidtr::extract::MinedField {
                        name: "handler".to_string(),
                        is_var: false,
                        target: "Handler".to_string(),
                        mult: MinedMultiplicity::One,
                        raw_union_type: None,
                    },
                ],
                is_abstract: false,
                parent: None,
                source_location: "test".to_string(),
                intersection_of: vec![], module: None,
            },
        ],
        fact_candidates: vec![],
    };
    resolve_external_types(&mut model);

    let sig_names: Vec<&str> = model.sigs.iter().map(|s| s.name.as_str()).collect();
    assert!(!sig_names.contains(&"T"), "T is a type parameter, should not be added");
    assert!(sig_names.contains(&"Handler"), "Handler should be added as placeholder");
}

// ── Alloy 6: var field extraction ───────────────────────────────────────────

#[test]
fn mine_rust_var_field_from_annotation() {
    let src = r#"
pub struct Account {
    /// @alloy: var
    pub balance: i32,
    pub name: String,
}
"#;
    let mined = rust_extractor::extract(src);
    assert_eq!(mined.sigs[0].fields.len(), 2);
    assert!(mined.sigs[0].fields[0].is_var,
        "balance should be var (has @alloy: var annotation)");
    assert!(!mined.sigs[0].fields[1].is_var,
        "name should not be var");
}

#[test]
fn mine_rust_var_field_renders_with_var() {
    let src = r#"
pub struct Account {
    /// @alloy: var
    pub balance: i32,
    pub name: String,
}
"#;
    let mined = rust_extractor::extract(src);
    let rendered = renderer::render(&mined);
    assert!(rendered.contains("var balance:"),
        "rendered should contain 'var balance:':\n{rendered}");
    assert!(!rendered.contains("var name:"),
        "name should not have var prefix:\n{rendered}");
}

// ── Alloy 6: temporal annotation extraction ─────────────────────────────────

#[test]
fn mine_rust_temporal_transition_annotation() {
    let src = r#"
pub struct S {
    pub x: i32,
}

#[cfg(test)]
mod tests {
    /// @temporal Transition constraint: StateUpdate
    /// Verifies state-transition invariant (prime = next-state).
    #[test]
    fn transition_state_update() {
        // test body
    }
}
"#;
    let mined = rust_extractor::extract(src);
    assert!(mined.fact_candidates.iter().any(|f|
        f.source_pattern.contains("@temporal Transition constraint: StateUpdate")),
        "should extract @temporal Transition annotation: {:?}",
        mined.fact_candidates.iter().map(|f| &f.source_pattern).collect::<Vec<_>>());
}

#[test]
fn mine_rust_temporal_invariant_annotation() {
    let src = r#"
pub struct S {
    pub x: i32,
}

#[cfg(test)]
mod tests {
    /// @temporal Invariant constraint: AlwaysPositive
    #[test]
    fn invariant_always_positive() {
        // test body
    }
}
"#;
    let mined = rust_extractor::extract(src);
    assert!(mined.fact_candidates.iter().any(|f|
        f.source_pattern.contains("@temporal Invariant constraint: AlwaysPositive")),
        "should extract @temporal Invariant annotation: {:?}",
        mined.fact_candidates.iter().map(|f| &f.source_pattern).collect::<Vec<_>>());
}
