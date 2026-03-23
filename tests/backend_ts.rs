use oxidtr::parser;
use oxidtr::ir;
use oxidtr::backend::typescript;
use oxidtr::backend::GeneratedFile;

fn generate_from(input: &str) -> Vec<GeneratedFile> {
    let model = parser::parse(input).expect("should parse");
    let ir = ir::lower(&model).expect("should lower");
    typescript::generate(&ir)
}

fn find_file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("file {path} not found"))
}

#[test]
fn ts_generates_interface_for_sig() {
    let files = generate_from("sig User { name: one Role }\nsig Role {}");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("export interface User {"));
    assert!(models.contains("name: Role;"));
    assert!(models.contains("export interface Role {}"));
}

#[test]
fn ts_generates_lone_as_nullable() {
    let files = generate_from("sig Node { parent: lone Node }");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("parent: Node | null;"));
}

#[test]
fn ts_generates_set_as_array() {
    let files = generate_from("sig Group { members: set User }\nsig User {}");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("members: Set<User>;"));
}

#[test]
fn ts_generates_string_literal_union_for_all_singleton_enum() {
    let files = generate_from(
        "abstract sig Color {}\none sig Red extends Color {}\none sig Blue extends Color {}",
    );
    let models = find_file(&files, "models.ts");
    assert!(models.contains("export type Color = \"Red\" | \"Blue\";"));
}

#[test]
fn ts_generates_discriminated_union_for_enum_with_fields() {
    let files = generate_from(
        "abstract sig Expr {}\nsig Literal extends Expr {}\nsig BinOp extends Expr { left: one Expr, right: one Expr }",
    );
    let models = find_file(&files, "models.ts");
    assert!(models.contains("kind: \"BinOp\";"));
    assert!(models.contains("left: Expr;"));
    assert!(models.contains("export type Expr = Literal | BinOp;"));
}

#[test]
fn ts_generates_operation_stubs() {
    let files = generate_from(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("export function changeRole("));
    assert!(ops.contains("throw new Error"));
}

#[test]
fn ts_generates_invariant_functions() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let inv = find_file(&files, "invariants.ts");
    assert!(inv.contains("export function assertHasRole("));
    assert!(inv.contains("return "));
    assert!(inv.contains(".every("));
}

#[test]
fn ts_generates_vitest_tests() {
    let files = generate_from(
        "sig User {}\nassert AllUsersExist { all u: User | u = u }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("import { describe, it, expect }"));
    assert!(tests.contains("describe('property tests'"));
    assert!(tests.contains("expect("));
}

#[test]
fn ts_expr_uses_every_some_includes() {
    let files = generate_from(
        "sig Item { tags: set Tag }\nsig Tag {}\nfact Tagged { all i: Item | some t: Tag | t in i.tags }",
    );
    let inv = find_file(&files, "invariants.ts");
    assert!(inv.contains(".every("), "expected .every()");
    assert!(inv.contains(".some("), "expected .some()");
    assert!(inv.contains(".has("), "expected .has() for Set membership");
}

#[test]
fn ts_expr_uses_length_for_cardinality() {
    let files = generate_from(
        "sig Box { items: set Item }\nsig Item {}\nfact Bounded { all b: Box | #b.items = #b.items }",
    );
    let inv = find_file(&files, "invariants.ts");
    assert!(inv.contains(".size"), "expected .size for Set cardinality");
}

#[test]
fn ts_self_hosting_model_generates() {
    let source = std::fs::read_to_string("models/oxidtr.als").expect("read model");
    let model = parser::parse(&source).expect("parse");
    let ir = ir::lower(&model).expect("lower");
    let files = typescript::generate(&ir);

    let models = find_file(&files, "models.ts");
    assert!(models.contains("export interface SigDecl"));
    assert!(models.contains("export interface OxidtrIR"));
    assert!(models.contains("export type Multiplicity"));
    assert!(models.contains("export type Expr ="));

    let inv = find_file(&files, "invariants.ts");
    assert!(inv.contains("assertSigToStructureBijection"));

    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("lowerOneSig"));
}

// ── Operations JSDoc from body ──────────────────────────────────────────────

#[test]
fn ts_operations_jsdoc_from_body() {
    let files = generate_from(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u.r = r }",
    );
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("/**"), "expected JSDoc comment:\n{ops}");
    assert!(ops.contains("@pre"), "expected @pre tag:\n{ops}");
}

#[test]
fn ts_operations_jsdoc_body_content() {
    let files = generate_from(
        "sig User {}\nsig Role {}\npred changeRole[u: one User, r: one Role] { u = u }",
    );
    let ops = find_file(&files, "operations.ts");
    // The body u = u should be described
    assert!(ops.contains("@pre u = u"), "expected body description:\n{ops}");
}

// ── Non-vacuous test generation ─────────────────────────────────────────────

#[test]
fn ts_tests_import_fixtures() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("import * as fix from './fixtures'"),
        "tests should import fixtures:\n{tests}");
}

#[test]
fn ts_tests_use_fixture_factory_for_sig_with_fields() {
    let files = generate_from(
        "sig User { role: one Role }\nsig Role {}\nfact HasRole { all u: User | u.role = u.role }",
    );
    let tests = find_file(&files, "tests.ts");
    assert!(tests.contains("fix.defaultUser()"),
        "test should use fixture factory for User:\n{tests}");
}

#[test]
fn ts_tests_empty_array_for_sig_without_fields() {
    let files = generate_from(
        "sig Token {}\nassert AllTokens { all t: Token | t = t }",
    );
    let tests = find_file(&files, "tests.ts");
    // Token has no fields → no fixture factory → stays as empty array
    assert!(tests.contains("Token[] = []"),
        "test should use empty array for Token (no fields):\n{tests}");
}

// ── Feature 1: Fun return type in TS ────────────────────────────────────────

#[test]
fn ts_fun_return_type_one() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun getRole[u: one User]: one Role { u }
    "#);
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("): M.Role {"), "should have return type M.Role:\n{ops}");
}

#[test]
fn ts_fun_return_type_lone() {
    let files = generate_from(r#"
        sig User {}
        sig Role {}
        fun findRole[u: one User]: lone Role { u }
    "#);
    let ops = find_file(&files, "operations.ts");
    assert!(ops.contains("): M.Role | null {"), "should have return type M.Role | null:\n{ops}");
}

// ── Feature 2: Singleton support in TS ──────────────────────────────────────

#[test]
fn ts_singleton_const_object() {
    let files = generate_from("one sig Config {}");
    let models = find_file(&files, "models.ts");
    assert!(models.contains("export const Config: Config = {};"),
        "should generate const object for singleton:\n{models}");
}

// ── Feature 4: Product → Map type in TS ─────────────────────────────────────

#[test]
fn ts_product_field_to_map() {
    let files = generate_from(r#"
        sig Config { settings: one Key -> Value }
        sig Key {}
        sig Value {}
    "#);
    let models = find_file(&files, "models.ts");
    assert!(models.contains("Map<Key, Value>"),
        "product field should map to Map:\n{models}");
}
