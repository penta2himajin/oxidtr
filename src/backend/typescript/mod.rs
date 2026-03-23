pub mod expr_translator;

use super::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity};
use crate::analyze;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// TypeScript test runner selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsTestRunner {
    Bun,
    Vitest,
}

/// Config for TypeScript backend.
#[derive(Debug, Clone)]
pub struct TsBackendConfig {
    pub test_runner: TsTestRunner,
}

impl Default for TsBackendConfig {
    fn default() -> Self {
        Self { test_runner: TsTestRunner::Bun }
    }
}

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    generate_with_config(ir, &TsBackendConfig::default())
}

pub fn generate_with_config(ir: &OxidtrIR, config: &TsBackendConfig) -> Vec<GeneratedFile> {
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "models.ts".to_string(),
        content: generate_models(ir),
    });

    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    // Generate helpers.ts for TC functions (replaces invariants.ts)
    if has_tc {
        files.push(GeneratedFile {
            path: "helpers.ts".to_string(),
            content: generate_helpers(ir),
        });
    }

    if !ir.operations.is_empty() {
        files.push(GeneratedFile {
            path: "operations.ts".to_string(),
            content: generate_operations(ir),
        });
    }

    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        files.push(GeneratedFile {
            path: "tests.ts".to_string(),
            content: generate_tests(ir, config.test_runner),
        });
    }

    files.push(GeneratedFile {
        path: "fixtures.ts".to_string(),
        content: generate_fixtures(ir),
    });

    files
}

// ── models.ts ──────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    let children: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for s in &ir.structures {
            if let Some(parent) = &s.parent {
                map.entry(parent.clone()).or_default().push(s.name.clone());
            }
        }
        map
    };

    let enum_parents: HashSet<String> = ir
        .structures
        .iter()
        .filter(|s| s.is_enum)
        .map(|s| s.name.clone())
        .collect();

    let variant_names: HashSet<String> = ir
        .structures
        .iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone())
        .collect();

    // Build struct map for looking up child sig fields
    let struct_map: HashMap<&str, &StructureNode> = ir
        .structures
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();

    let disj_fields = analyze::disj_fields(ir);

    for s in &ir.structures {
        if variant_names.contains(&s.name) {
            continue;
        }

        // JSDoc from constraints
        let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
        if !constraint_names.is_empty() {
            writeln!(out, "/**").unwrap();
            for cn in &constraint_names {
                writeln!(out, " * @invariant {cn}").unwrap();
            }
            writeln!(out, " */").unwrap();
        }

        if s.is_enum {
            generate_union_type(&mut out, s, children.get(&s.name), &struct_map);
        } else {
            generate_interface(&mut out, s, ir, &disj_fields);
        }
        writeln!(out).unwrap();
    }

    out
}

fn generate_interface(out: &mut String, s: &StructureNode, ir: &OxidtrIR, disj_fields: &[(String, String)]) {
    // Singleton: one sig → interface + exported const
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        writeln!(out, "export interface {} {{}}", s.name).unwrap();
        writeln!(out, "export const {}: {} = {{}};", s.name, s.name).unwrap();
        return;
    }

    if s.fields.is_empty() {
        writeln!(out, "export interface {} {{}}", s.name).unwrap();
    } else {
        writeln!(out, "export interface {} {{", s.name).unwrap();
        for f in &s.fields {
            // Gap 1 & 3: annotations for sig multiplicity and disj constraints
            write_field_annotations_ts(out, ir, &s.name, f, disj_fields);
            let type_str = if let Some(vt) = &f.value_type {
                format!("Map<{}, {}>", f.target, vt)
            } else {
                mult_to_ts_type(&f.target, &f.mult)
            };
            writeln!(out, "  {}: {};", f.name, type_str).unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
}

fn write_field_annotations_ts(
    out: &mut String,
    ir: &OxidtrIR,
    sig_name: &str,
    f: &IRField,
    disj_fields: &[(String, String)],
) {
    let target_mult = analyze::sig_multiplicity_for(ir, &f.target);
    match target_mult {
        SigMultiplicity::Some => {
            if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                writeln!(out, "  /** @NotEmpty Target is `some sig` — collection must not be empty. */").unwrap();
            }
        }
        SigMultiplicity::Lone => {
            if f.mult == Multiplicity::One {
                writeln!(out, "  /** @constraint Target is `lone sig` — reference may not exist. */").unwrap();
            }
        }
        _ => {}
    }
    // Gap 3: disj → suggest Set
    if disj_fields.iter().any(|(sig, field)| sig == sig_name && field == &f.name) {
        if f.mult == Multiplicity::Seq {
            writeln!(out, "  /** Consider using Set<T> for uniqueness (disj constraint). */").unwrap();
        }
    }
}

fn generate_union_type(
    out: &mut String,
    s: &StructureNode,
    children: Option<&Vec<String>>,
    struct_map: &HashMap<&str, &StructureNode>,
) {
    let Some(variants) = children else {
        writeln!(out, "export type {} = never;", s.name).unwrap();
        return;
    };

    // Check if all variants are unit (no fields) — use string literal union
    let all_unit = variants.iter().all(|v| {
        struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
    });

    if all_unit {
        // Simple string literal union: type Multiplicity = "MultOne" | "MultLone" | ...
        let parts: Vec<String> = variants.iter()
            .map(|v| format!("\"{}\"", v))
            .collect();
        writeln!(out, "export type {} = {};", s.name, parts.join(" | ")).unwrap();
    } else {
        // Discriminated union with kind field
        for v in variants {
            let child = struct_map.get(v.as_str());
            let fields = child.map(|c| &c.fields).filter(|f| !f.is_empty());
            writeln!(out, "export interface {} {{", v).unwrap();
            writeln!(out, "  kind: \"{}\";", v).unwrap();
            if let Some(fields) = fields {
                for f in fields {
                    let type_str = if let Some(vt) = &f.value_type {
                        format!("Map<{}, {}>", f.target, vt)
                    } else {
                        mult_to_ts_type(&f.target, &f.mult)
                    };
                    writeln!(out, "  {}: {};", f.name, type_str).unwrap();
                }
            }
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
        let parts: Vec<String> = variants.clone();
        writeln!(out, "export type {} = {};", s.name, parts.join(" | ")).unwrap();
    }
}

fn mult_to_ts_type(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => target.to_string(),
        Multiplicity::Lone => format!("{target} | null"),
        Multiplicity::Set => format!("Set<{target}>"),
        Multiplicity::Seq => format!("{target}[]"),
    }
}

// ── helpers.ts ─────────────────────────────────────────────────────────────

/// Generate helpers.ts containing TC (transitive closure) functions.
fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "import type * as M from './models';").unwrap();
    writeln!(out).unwrap();

    // TC functions
    let mut tc_fields = Vec::new();
    for c in &ir.constraints {
        tc_fields.extend(expr_translator::extract_tc_fields(&c.expr, ir));
    }
    for p in &ir.properties {
        tc_fields.extend(expr_translator::extract_tc_fields(&p.expr, ir));
    }
    tc_fields.sort_by(|a, b| (&a.sig_name, &a.field_name).cmp(&(&b.sig_name, &b.field_name)));
    tc_fields.dedup();

    for tc in &tc_fields {
        generate_tc_function(&mut out, tc);
    }

    out
}

fn generate_tc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("tc{}", capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "/** Transitive closure traversal for {sig}.{field}. */").unwrap();

    match tc.mult {
        Multiplicity::Lone => {
            writeln!(out, "export function {fn_name}(start: M.{sig}): M.{sig}[] {{").unwrap();
            writeln!(out, "  const result: M.{sig}[] = [];").unwrap();
            writeln!(out, "  let current: M.{sig} | null = start.{field};").unwrap();
            writeln!(out, "  while (current !== null) {{").unwrap();
            writeln!(out, "    result.push(current);").unwrap();
            writeln!(out, "    current = current.{field};").unwrap();
            writeln!(out, "  }}").unwrap();
            writeln!(out, "  return result;").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::Set | Multiplicity::Seq => {
            writeln!(out, "export function {fn_name}(start: M.{sig}): M.{sig}[] {{").unwrap();
            writeln!(out, "  const result: M.{sig}[] = [];").unwrap();
            writeln!(out, "  const queue: M.{sig}[] = [...start.{field}];").unwrap();
            writeln!(out, "  while (queue.length > 0) {{").unwrap();
            writeln!(out, "    const next = queue.pop()!;").unwrap();
            writeln!(out, "    if (!result.includes(next)) {{").unwrap();
            writeln!(out, "      result.push(next);").unwrap();
            writeln!(out, "      queue.push(...next.{field});").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "  }}").unwrap();
            writeln!(out, "  return result;").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::One => {
            writeln!(out, "export function {fn_name}(start: M.{sig}): M.{sig}[] {{").unwrap();
            writeln!(out, "  const result: M.{sig}[] = [];").unwrap();
            writeln!(out, "  let current: M.{sig} = start.{field};").unwrap();
            writeln!(out, "  for (let i = 0; i < 1000; i++) {{").unwrap();
            writeln!(out, "    if (result.includes(current)) break;").unwrap();
            writeln!(out, "    result.push(current);").unwrap();
            writeln!(out, "    current = current.{field};").unwrap();
            writeln!(out, "  }}").unwrap();
            writeln!(out, "  return result;").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
    writeln!(out).unwrap();
}

// ── operations.ts ──────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "import type * as M from './models';").unwrap();
    writeln!(out).unwrap();

    for op in &ir.operations {
        let fn_name = to_camel_case(&op.name);
        let params = op
            .params
            .iter()
            .map(|p| {
                let type_str = match p.mult {
                    Multiplicity::One => format!("M.{}", p.type_name),
                    Multiplicity::Lone => format!("M.{} | null", p.type_name),
                    Multiplicity::Set => format!("Set<M.{}>", p.type_name),
                    Multiplicity::Seq => format!("M.{}[]", p.type_name),
                };
                format!("{}: {type_str}", to_camel_case(&p.name))
            })
            .collect::<Vec<_>>()
            .join(", ");

        // JSDoc from body expressions with pre/post separation (Feature 7)
        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            writeln!(out, "/**").unwrap();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, " * @{tag} {desc}").unwrap();
            }
            writeln!(out, " */").unwrap();
        }

        let return_str = match &op.return_type {
            Some(rt) => ts_return_type(&rt.type_name, &rt.mult),
            None => "void".to_string(),
        };

        writeln!(out, "export function {fn_name}({params}): {return_str} {{").unwrap();
        writeln!(out, "  throw new Error(\"oxidtr: implement {}\");", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── tests.ts ───────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR, test_runner: TsTestRunner) -> String {
    let mut out = String::new();
    let sig_names = collect_sig_names(ir);

    // Collect which sigs have fixture factories (non-enum, non-variant, with fields)
    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();
    let has_fixture: HashSet<String> = ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone()).collect();

    // Check if any expression uses TC functions → need helpers import
    let needs_helpers = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    let test_import = match test_runner {
        TsTestRunner::Bun => "bun:test",
        TsTestRunner::Vitest => "vitest",
    };
    writeln!(out, "import {{ describe, it, expect }} from '{}';", test_import).unwrap();
    writeln!(out, "import type * as M from './models';").unwrap();
    if needs_helpers {
        writeln!(out, "import * as helpers from './helpers';").unwrap();
    }
    writeln!(out, "import * as fix from './fixtures';").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "describe('property tests', () => {{").unwrap();
    // Property tests from asserts — inline expressions
    for prop in &ir.properties {
        let test_name = to_camel_case(&prop.name);
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&prop.expr, ir);

        writeln!(out, "  it('{}', () => {{", test_name).unwrap();
        for (pname, tname) in &params {
            if has_fixture.contains(tname) {
                writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
            } else {
                writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
            }
        }
        writeln!(out, "    expect({body}).toBe(true);").unwrap();
        writeln!(out, "  }});").unwrap();
        writeln!(out).unwrap();
    }

    // Invariant tests — inline constraint expressions directly
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let test_name = format!("invariant {fact_name}");
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        let ownership = super::detect_ownership_pattern(&constraint.expr, ir, ts_param_name);

        writeln!(out, "  it('{}', () => {{", test_name).unwrap();
        if let Some((owned_var, owner_var, _owner_type, field_name)) = &ownership {
            let owned_param = params.iter().find(|(p, _)| p == owned_var);
            let owner_param = params.iter().find(|(p, _)| p == owner_var);
            if let (Some((opname, otname)), Some((cpname, ctname))) = (owned_param, owner_param) {
                writeln!(out, "    const item = fix.default{otname}();").unwrap();
                writeln!(out, "    const owner = fix.default{ctname}();").unwrap();
                writeln!(out, "    owner.{field_name}.add(item);").unwrap();
                writeln!(out, "    const {opname}: M.{otname}[] = [item];").unwrap();
                writeln!(out, "    const {cpname}: M.{ctname}[] = [owner];").unwrap();
                for (pname, tname) in &params {
                    if pname == opname || pname == cpname { continue; }
                    if has_fixture.contains(tname) {
                        writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
                    } else {
                        writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
                    }
                }
            }
        } else {
            for (pname, tname) in &params {
                if has_fixture.contains(tname) {
                    writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
                } else {
                    writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
                }
            }
        }
        writeln!(out, "    expect({body}).toBe(true);").unwrap();
        writeln!(out, "  }});").unwrap();
        writeln!(out).unwrap();
    }

    // Boundary value tests — inline expressions (Feature 5)
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        let has_boundary = params.iter().any(|(_, tname)| {
            ir.structures.iter().any(|s| {
                s.name == *tname && !s.is_enum && s.fields.iter().any(|f| {
                    matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                        && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                })
            })
        });

        if has_boundary {
            let test_name = format!("boundary {fact_name}");
            writeln!(out, "  it('{test_name}', () => {{").unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "    const {pname}: M.{tname}[] = [fix.boundary{tname}()];").unwrap();
                } else if has_fixture.contains(tname) {
                    writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
                } else {
                    writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
                }
            }
            writeln!(out, "    expect({body}).toBe(true);").unwrap();
            writeln!(out, "  }});").unwrap();
            writeln!(out).unwrap();

            let test_name = format!("invalid {fact_name}");
            writeln!(out, "  it('{test_name}', () => {{").unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "    const {pname}: M.{tname}[] = [fix.invalid{tname}()];").unwrap();
                } else if has_fixture.contains(tname) {
                    writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
                } else {
                    writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
                }
            }
            writeln!(out, "    expect(!({body})).toBe(true);").unwrap();
            writeln!(out, "  }});").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Cross-tests
    if !ir.constraints.is_empty() && !ir.operations.is_empty() {
        writeln!(out, "  // --- Cross-tests: fact × operation ---").unwrap();
        writeln!(out).unwrap();
        for constraint in &ir.constraints {
            let fact_name = match &constraint.name {
                Some(name) => name.clone(),
                None => continue,
            };
            for op in &ir.operations {
                let op_name = to_camel_case(&op.name);
                let test_name = format!("{fact_name} preserved after {op_name}");
                writeln!(out, "  it.skip('{test_name}', () => {{").unwrap();
                writeln!(out, "    // pre: expect(/* {fact_name} constraint */).toBe(true);").unwrap();
                writeln!(out, "    // {op_name}(...);").unwrap();
                writeln!(out, "    // post: expect(/* {fact_name} constraint */).toBe(true);").unwrap();
                writeln!(out, "    throw new Error('oxidtr: implement cross-test');").unwrap();
                writeln!(out, "  }});").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    writeln!(out, "}});").unwrap();

    out
}

// ── helpers ────────────────────────────────────────────────────────────────

fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

fn expr_uses_tc(expr: &crate::parser::ast::Expr) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::TransitiveClosure(_) => true,
        Expr::FieldAccess { base, .. } => expr_uses_tc(base),
        Expr::Cardinality(inner) | Expr::Not(inner) => expr_uses_tc(inner),
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            expr_uses_tc(left) || expr_uses_tc(right)
        }
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| expr_uses_tc(&b.domain)) || expr_uses_tc(body)
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
}

fn to_camel_case(s: &str) -> String {
    // Already camelCase in Alloy; pass through
    s.to_string()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

// ── fixtures.ts ────────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();

    writeln!(out, "import type * as M from './models';").unwrap();
    writeln!(out).unwrap();

    // Build children map for enum default fixtures
    let children: HashMap<String, Vec<String>> = {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for s in &ir.structures {
            if let Some(parent) = &s.parent {
                map.entry(parent.clone()).or_default().push(s.name.clone());
            }
        }
        map
    };
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s))
        .collect();

    // Generate enum default fixtures (first variant as string literal)
    for s in &ir.structures {
        if !s.is_enum { continue; }
        let variants = match children.get(&s.name) {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        // Find first unit variant (no fields)
        let first_unit = variants.iter().find(|v| {
            struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
        });
        if let Some(variant) = first_unit {
            writeln!(out, "/** Factory: default value for enum {} */", s.name).unwrap();
            writeln!(out, "export function default{}(): M.{} {{", s.name, s.name).unwrap();
            writeln!(out, "  return \"{}\";", variant).unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Collect which types have fixture factories (for populating set/seq fields)
    let fixture_types: HashSet<String> = ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone())
        .collect();

    for s in &ir.structures {
        if variant_names.contains(&s.name) || s.is_enum { continue; }
        if s.fields.is_empty() { continue; }

        let fn_name = format!("default{}", s.name);
        writeln!(out, "/** Factory: create a default valid {} */", s.name).unwrap();
        writeln!(out, "export function {fn_name}(): M.{} {{", s.name).unwrap();
        writeln!(out, "  return {{").unwrap();
        for f in &s.fields {
            let val = if f.value_type.is_some() {
                "new Map()".to_string()
            } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && ts_is_safe_set_population(&s.name, &f.target, ir, &fixture_types) {
                let safe = HashSet::from([f.target.clone()]);
                ts_default_value_inner(&f.target, &f.mult, &safe)
            } else {
                ts_default_value(&f.target, &f.mult)
            };
            writeln!(out, "    {}: {},", f.name, val).unwrap();
        }
        writeln!(out, "  }};").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Boundary value fixtures (Feature 5)
        let has_bounds = s.fields.iter().any(|f| {
            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
        });
        if has_bounds {
            let boundary_fn = format!("boundary{}", s.name);
            writeln!(out, "/** Factory: create {} at cardinality boundary */", s.name).unwrap();
            writeln!(out, "export function {boundary_fn}(): M.{} {{", s.name).unwrap();
            writeln!(out, "  return {{").unwrap();
            for f in &s.fields {
                let val = if f.value_type.is_some() {
                    "new Map()".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let count = match &bound {
                            analyze::BoundKind::Exact(n) => *n,
                            analyze::BoundKind::AtMost(n) => *n,
                            analyze::BoundKind::AtLeast(n) => *n,
                        };
                        ts_boundary_value(&f.target, &f.mult, count)
                    } else {
                        ts_default_value(&f.target, &f.mult)
                    }
                } else {
                    ts_default_value(&f.target, &f.mult)
                };
                writeln!(out, "    {}: {},", f.name, val).unwrap();
            }
            writeln!(out, "  }};").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();

            let invalid_fn = format!("invalid{}", s.name);
            writeln!(out, "/** Factory: create {} that violates cardinality constraint */", s.name).unwrap();
            writeln!(out, "export function {invalid_fn}(): M.{} {{", s.name).unwrap();
            writeln!(out, "  return {{").unwrap();
            for f in &s.fields {
                let val = if f.value_type.is_some() {
                    "new Map()".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let violation = match &bound {
                            analyze::BoundKind::Exact(n) => n + 1,
                            analyze::BoundKind::AtMost(n) => n + 1,
                            analyze::BoundKind::AtLeast(n) => if *n > 0 { n - 1 } else { 0 },
                        };
                        ts_boundary_value(&f.target, &f.mult, violation)
                    } else {
                        ts_default_value(&f.target, &f.mult)
                    }
                } else {
                    ts_default_value(&f.target, &f.mult)
                };
                writeln!(out, "    {}: {},", f.name, val).unwrap();
            }
            writeln!(out, "  }};").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

fn ts_boundary_value(target: &str, mult: &Multiplicity, count: usize) -> String {
    match mult {
        Multiplicity::Set => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            if items.is_empty() {
                "new Set()".to_string()
            } else {
                format!("new Set([{}])", items.join(", "))
            }
        }
        Multiplicity::Seq => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            format!("[{}]", items.join(", "))
        }
        _ => ts_default_value(target, mult),
    }
}

fn ts_return_type(type_name: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => format!("M.{type_name}"),
        Multiplicity::Lone => format!("M.{type_name} | null"),
        Multiplicity::Set => format!("Set<M.{type_name}>"),
        Multiplicity::Seq => format!("M.{type_name}[]"),
    }
}

/// Convert type name to TS param name (camelCase + plural 's').
fn ts_param_name(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out.push('s');
    out
}

fn ts_default_value(target: &str, mult: &Multiplicity) -> String {
    ts_default_value_inner(target, mult, &HashSet::new())
}

fn ts_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    match mult {
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set => {
            if safe_targets.contains(target) {
                format!("new Set([default{}()])", target)
            } else {
                "new Set()".to_string()
            }
        }
        Multiplicity::Seq => {
            if safe_targets.contains(target) {
                format!("[default{}()]", target)
            } else {
                "[]".to_string()
            }
        }
        Multiplicity::One => format!("default{}()", target),
    }
}

/// Check if populating a set/seq field of `owner` with `default{target}()`
/// would cause infinite recursion in TS fixtures.
fn ts_is_safe_set_population(
    owner: &str, target: &str,
    ir: &OxidtrIR, fixture_types: &HashSet<String>,
) -> bool {
    if !fixture_types.contains(target) { return false; }
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s))
        .collect();
    let mut visited = HashSet::new();
    let mut stack = vec![target.to_string()];
    while let Some(cur) = stack.pop() {
        if cur == owner { return false; }
        if !visited.insert(cur.clone()) { continue; }
        if let Some(s) = struct_map.get(cur.as_str()) {
            for f in &s.fields {
                if f.mult == Multiplicity::One && fixture_types.contains(&f.target) {
                    stack.push(f.target.clone());
                }
            }
        }
    }
    true
}

// ── validators.ts ──────────────────────────────────────────────────────────
// TS has the weakest type system → generate runtime validation functions.

/// Generate runtime validation functions for TypeScript.
/// Returns empty string if there are no constraints and no collection fields to validate.
pub fn generate_validators(ir: &OxidtrIR) -> String {
    let enum_parents: HashSet<String> = ir.structures.iter()
        .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
        .map(|s| s.name.clone()).collect();

    // Only generate validators for concrete sigs with fields
    let sigs_to_validate: Vec<&StructureNode> = ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty())
        .collect();

    if sigs_to_validate.is_empty() {
        return String::new();
    }

    let mut out = String::new();

    writeln!(out, "import type * as M from './models';").unwrap();
    writeln!(out).unwrap();

    for s in &sigs_to_validate {
        let fn_name = format!("validate{}", s.name);
        let param_name = s.name[..1].to_lowercase();
        let constraints = analyze::constraints_for_sig(ir, &s.name);

        // Named facts that reference this sig — may not be fully translatable,
        // but their names must appear here so `oxidtr check` can verify coverage.
        let named_facts = analyze::constraint_names_for_sig(ir, &s.name);

        writeln!(out, "/** Runtime validator for {} — checks all known constraints. */", s.name).unwrap();
        for fact in &named_facts {
            writeln!(out, "// @covers: {fact}").unwrap();
        }
        writeln!(out, "export function {fn_name}({param_name}: M.{}): string[] {{", s.name).unwrap();
        writeln!(out, "  const errors: string[] = [];").unwrap();

        // Null/presence checks for lone fields
        for f in &s.fields {
            match f.mult {
                Multiplicity::One => {
                    // In TS, "one" fields could still be null at runtime
                    writeln!(out, "  if ({param_name}.{} == null) errors.push(\"{} must not be null\");",
                        f.name, f.name).unwrap();
                }
                _ => {}
            }
        }

        // Constraint-derived checks
        for c in &constraints {
            match c {
                analyze::ConstraintInfo::CardinalityBound { field_name, bound, .. } => {
                    // Find the field to determine if it's Set or Seq
                    let field_opt = s.fields.iter().find(|f| f.name == *field_name);
                    let size_expr = match field_opt {
                        Some(f) if f.mult == Multiplicity::Set => format!("{param_name}.{field_name}.size"),
                        _ => format!("{param_name}.{field_name}.length"),
                    };
                    match bound {
                        analyze::BoundKind::Exact(n) => {
                            writeln!(out, "  if ({size_expr} !== {n}) errors.push(\"{field_name} must have exactly {n} element(s)\");").unwrap();
                        }
                        analyze::BoundKind::AtMost(n) => {
                            writeln!(out, "  if ({size_expr} > {n}) errors.push(\"{field_name} exceeds max size {n}\");").unwrap();
                        }
                        analyze::BoundKind::AtLeast(n) => {
                            writeln!(out, "  if ({size_expr} < {n}) errors.push(\"{field_name} must have at least {n} element(s)\");").unwrap();
                        }
                    }
                }
                analyze::ConstraintInfo::Presence { field_name, kind: analyze::PresenceKind::Required, .. } => {
                    writeln!(out, "  if ({param_name}.{field_name} == null) errors.push(\"{field_name} must not be null\");").unwrap();
                }
                analyze::ConstraintInfo::Presence { field_name, kind: analyze::PresenceKind::Absent, .. } => {
                    writeln!(out, "  if ({param_name}.{field_name} != null) errors.push(\"{field_name} must be null\");").unwrap();
                }
                analyze::ConstraintInfo::NoSelfRef { field_name, .. } => {
                    writeln!(out, "  if ({param_name}.{field_name} === {param_name}) errors.push(\"{field_name} must not reference self\");").unwrap();
                }
                analyze::ConstraintInfo::Acyclic { field_name, .. } => {
                    writeln!(out, "  {{ const seen = new Set<unknown>(); let cur: unknown = {param_name}; while (cur != null) {{ if (seen.has(cur)) {{ errors.push(\"{field_name} must not form a cycle\"); break; }} seen.add(cur); cur = (cur as Record<string, unknown>).{field_name}; }} }}").unwrap();
                }
                analyze::ConstraintInfo::FieldOrdering { left_field, op, right_field, .. } => {
                    let ts_op = match op {
                        CompareOp::Lt => "<",
                        CompareOp::Gt => ">",
                        CompareOp::Lte => "<=",
                        CompareOp::Gte => ">=",
                        _ => continue,
                    };
                    let negated_op = match op {
                        CompareOp::Lt => ">=",
                        CompareOp::Gt => "<=",
                        CompareOp::Lte => ">",
                        CompareOp::Gte => "<",
                        _ => continue,
                    };
                    writeln!(out, "  if ({param_name}.{left_field} {negated_op} {param_name}.{right_field}) errors.push(\"{left_field} must be {ts_op} {right_field}\");").unwrap();
                }
                analyze::ConstraintInfo::Iff { left, right, .. } => {
                    let desc_l = analyze::describe_expr(left);
                    let desc_r = analyze::describe_expr(right);
                    writeln!(out, "  // TODO: iff constraint — {desc_l} iff {desc_r}").unwrap();
                }
                analyze::ConstraintInfo::Prohibition { condition, .. } => {
                    let desc = analyze::describe_expr(condition);
                    writeln!(out, "  // TODO: prohibition — no instance where {desc}").unwrap();
                }
                _ => {} // Named, Membership — not directly translatable to simple validators
            }
        }

        // Disj uniqueness checks for seq fields
        let disj = analyze::disj_fields(ir);
        for (dsig, dfield) in &disj {
            if dsig == &s.name {
                if let Some(f) = s.fields.iter().find(|f| f.name == *dfield) {
                    if f.mult == Multiplicity::Seq {
                        writeln!(out, "  if (new Set({param_name}.{dfield}).size !== {param_name}.{dfield}.length) errors.push(\"{dfield} must not contain duplicates (disj constraint)\");").unwrap();
                    }
                }
            }
        }

        writeln!(out, "  return errors;").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}
