pub mod expr_translator;

use super::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::Multiplicity;
use crate::analyze;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "models.ts".to_string(),
        content: generate_models(ir),
    });

    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    if !ir.constraints.is_empty() || has_tc {
        files.push(GeneratedFile {
            path: "invariants.ts".to_string(),
            content: generate_invariants(ir),
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
            content: generate_tests(ir),
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
            generate_interface(&mut out, s);
        }
        writeln!(out).unwrap();
    }

    out
}

fn generate_interface(out: &mut String, s: &StructureNode) {
    if s.fields.is_empty() {
        writeln!(out, "export interface {} {{}}", s.name).unwrap();
    } else {
        writeln!(out, "export interface {} {{", s.name).unwrap();
        for f in &s.fields {
            let type_str = mult_to_ts_type(&f.target, &f.mult);
            writeln!(out, "  {}: {};", f.name, type_str).unwrap();
        }
        writeln!(out, "}}").unwrap();
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
                    let type_str = mult_to_ts_type(&f.target, &f.mult);
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

// ── invariants.ts ──────────────────────────────────────────────────────────

fn generate_invariants(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = collect_sig_names(ir);

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

    for constraint in &ir.constraints {
        let fn_name = match &constraint.name {
            Some(name) => format!("assert{}", name),
            None => continue,
        };

        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        let param_str = params
            .iter()
            .map(|(pname, tname)| format!("{pname}: M.{tname}[]"))
            .collect::<Vec<_>>()
            .join(", ");

        writeln!(out, "/** Invariant derived from Alloy fact. */").unwrap();
        writeln!(out, "export function {fn_name}({param_str}): boolean {{").unwrap();
        writeln!(out, "  return {body};").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
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

        // JSDoc from body expressions
        if !op.body.is_empty() {
            writeln!(out, "/**").unwrap();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                writeln!(out, " * @pre {desc}").unwrap();
            }
            writeln!(out, " */").unwrap();
        }

        writeln!(out, "export function {fn_name}({params}): void {{").unwrap();
        writeln!(out, "  throw new Error(\"oxidtr: implement {}\");", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── tests.ts ───────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
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

    writeln!(out, "import {{ describe, it, expect }} from 'vitest';").unwrap();
    writeln!(out, "import type * as M from './models';").unwrap();
    writeln!(out, "import * as inv from './invariants';").unwrap();
    writeln!(out, "import * as fix from './fixtures';").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "describe('property tests', () => {{").unwrap();
    // Property tests from asserts
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

    // Invariant tests
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let fn_name = format!("assert{fact_name}");
        let test_name = format!("invariant {fact_name}");
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);

        writeln!(out, "  it('{}', () => {{", test_name).unwrap();
        for (pname, tname) in &params {
            if has_fixture.contains(tname) {
                writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
            } else {
                writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
            }
        }
        let args = params
            .iter()
            .map(|(pname, _)| pname.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "    expect(inv.{fn_name}({args})).toBe(true);").unwrap();
        writeln!(out, "  }});").unwrap();
        writeln!(out).unwrap();
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
            let fact_fn = format!("assert{fact_name}");
            for op in &ir.operations {
                let op_name = to_camel_case(&op.name);
                let test_name = format!("{fact_name} preserved after {op_name}");
                writeln!(out, "  it('{test_name}', () => {{").unwrap();
                writeln!(out, "    // pre: expect(inv.{fact_fn}()).toBe(true);").unwrap();
                writeln!(out, "    // {op_name}(...);").unwrap();
                writeln!(out, "    // post: expect(inv.{fact_fn}()).toBe(true);").unwrap();
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
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. } => {
            expr_uses_tc(left) || expr_uses_tc(right)
        }
        Expr::Quantifier { domain, body, .. } => expr_uses_tc(domain) || expr_uses_tc(body),
        Expr::VarRef(_) => false,
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

    for s in &ir.structures {
        if variant_names.contains(&s.name) || s.is_enum { continue; }
        if s.fields.is_empty() { continue; }

        let fn_name = format!("default{}", s.name);
        writeln!(out, "/** Factory: create a default valid {} */", s.name).unwrap();
        writeln!(out, "export function {fn_name}(): M.{} {{", s.name).unwrap();
        writeln!(out, "  return {{").unwrap();
        for f in &s.fields {
            let val = ts_default_value(&f.target, &f.mult);
            writeln!(out, "    {}: {},", f.name, val).unwrap();
        }
        writeln!(out, "  }};").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

fn ts_default_value(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set => "new Set()".to_string(),
        Multiplicity::Seq => "[]".to_string(),
        Multiplicity::One => format!("default{}()", target),
    }
}
