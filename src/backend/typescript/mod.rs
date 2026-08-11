pub mod expr_translator;

use super::{GeneratedFile, TargetLang, is_native_type_alias, resolve_type};
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity, TemporalBinaryOp};
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

    let has_tc = ir_uses_tc(ir);

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
        // Skip native type aliases (Str, Int, Float, Bool)
        if is_native_type_alias(&s.name) {
            continue;
        }

        // Intersection type alias: type Foo = A & B & C
        if !s.intersection_of.is_empty() {
            let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
            if !constraint_names.is_empty() {
                writeln!(out, "/**").unwrap();
                for cn in &constraint_names {
                    writeln!(out, " * @invariant {cn}").unwrap();
                }
                writeln!(out, " */").unwrap();
            }
            let components = s.intersection_of.join(" & ");
            writeln!(out, "export type {} = {};", s.name, components).unwrap();
            writeln!(out).unwrap();
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

    // Derived fields: receiver functions → class methods
    generate_derived_fields(&mut out, ir);

    out
}

fn generate_derived_fields(out: &mut String, ir: &OxidtrIR) {
    let mut by_sig: std::collections::BTreeMap<String, Vec<&OperationNode>> =
        std::collections::BTreeMap::new();
    for op in &ir.operations {
        if let Some(ref sig) = op.receiver_sig {
            by_sig.entry(sig.clone()).or_default().push(op);
        }
    }

    for (sig_name, ops) in &by_sig {
        writeln!(out, "export class {sig_name}Methods {{").unwrap();
        writeln!(out, "  constructor(private readonly self: {sig_name}) {{}}").unwrap();
        for op in ops {
            let fn_name = to_camel_case(&op.name);
            let params = op.params.iter().map(|p| {
                let type_str = match p.mult {
                    Multiplicity::One => p.type_name.clone(),
                    Multiplicity::Lone => format!("{} | null", p.type_name),
                    Multiplicity::Set => format!("Set<{}>", p.type_name),
                    Multiplicity::Seq => format!("{}[]", p.type_name),
                };
                format!("{}: {type_str}", to_camel_case(&p.name))
            }).collect::<Vec<_>>().join(", ");

            // A derived field (`fun Sig.name: T { body }`) is an expression,
            // and a receiver `pred` is a formula — neither is a procedure to
            // be filled in by hand (#82/#83).
            // These methods live *in* models.ts, so the types are local: an
            // `M.` qualifier names nothing (TS2503, present since the signature
            // was first emitted).
            let return_str = match &op.return_type {
                Some(rt) => ts_local_return_type(&rt.type_name, &rt.mult),
                None => "boolean".to_string(),
            };

            writeln!(out, "  {fn_name}({params}): {return_str} {{").unwrap();
            let env = crate::backend::type_env::operation_env(op);
            if op.body.is_empty() {
                writeln!(out, "    return true;").unwrap();
            } else if op.return_type.is_some() {
                let body = expr_translator::translate_with_env(&op.body[0], ir, &env);
                writeln!(out, "    return {body};").unwrap();
            } else {
                let conjuncts: Vec<String> = op.body.iter()
                    .map(|e| expr_translator::translate_with_env(e, ir, &env))
                    .collect();
                writeln!(out, "    return {};", conjuncts.join(" && ")).unwrap();
            }
            writeln!(out, "  }}").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

fn generate_interface(out: &mut String, s: &StructureNode, ir: &OxidtrIR, disj_fields: &[(String, String)]) {
    // Singleton: one sig → interface + exported const
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        if s.is_var {
            writeln!(out, "// @alloy: var sig").unwrap();
        }
        writeln!(out, "export interface {} {{}}", s.name).unwrap();
        writeln!(out, "export const {}: {} = {{}};", s.name, s.name).unwrap();
        return;
    }

    if s.is_var {
        writeln!(out, "// @alloy: var sig").unwrap();
    }
    if s.fields.is_empty() {
        writeln!(out, "export interface {} {{}}", s.name).unwrap();
    } else {
        writeln!(out, "export interface {} {{", s.name).unwrap();
        for f in &s.fields {
            // Gap 1 & 3: annotations for sig multiplicity and disj constraints
            write_field_annotations_ts(out, ir, &s.name, f, disj_fields);
            let resolved_target = resolve_type(TargetLang::TypeScript, &f.target);
            let type_str = if let Some(vt) = &f.value_type {
                let resolved_vt = resolve_type(TargetLang::TypeScript, vt);
                format!("Map<{}, {}>", resolved_target, resolved_vt)
            } else if let Some(raw) = &f.raw_union_type {
                // Preserve raw union type (e.g. "number | string") from source language
                if f.mult == Multiplicity::Lone {
                    format!("{} | null", raw)
                } else {
                    raw.clone()
                }
            } else {
                mult_to_ts_type(&resolved_target, &f.mult)
            };
            let readonly = if f.is_var { "" } else { "readonly " };
            if f.is_var {
                writeln!(out, "  /** @mutable changes across state transitions */").unwrap();
            }
            writeln!(out, "  {readonly}{}: {};", f.name, type_str).unwrap();
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

/// Whether an enum parent's union is a union of string literals rather than of
/// tagged objects. Nothing carries a field — not the parent, not any variant —
/// so there is no `kind` to discriminate on and the atom *is* its own name.
///
/// The fixture factory and the expression translator both have to agree with
/// `generate_union_type` on this, or the fixture's value has the wrong shape
/// for the type and `x.kind` reads a property of a string.
pub(crate) fn is_string_literal_union(
    s: &StructureNode,
    variants: &[String],
    struct_map: &HashMap<&str, &StructureNode>,
) -> bool {
    s.fields.is_empty() && variants.iter().all(|v| {
        struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
    })
}

/// `is_string_literal_union` resolved straight from the IR.
pub(crate) fn enum_is_string_literal(ir: &OxidtrIR, enum_name: &str) -> bool {
    let Some(s) = ir.structures.iter().find(|s| s.name == enum_name) else { return false };
    if !s.is_enum { return false; }
    let variants: Vec<String> = ir.structures.iter()
        .filter(|c| c.parent.as_deref() == Some(enum_name))
        .map(|c| c.name.clone())
        .collect();
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s)).collect();
    is_string_literal_union(s, &variants, &struct_map)
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

    // Parent abstract sig may have fields that should be inherited by all variants
    let parent_fields = &s.fields;

    if is_string_literal_union(s, variants, struct_map) {
        // Simple string literal union: type Multiplicity = "MultOne" | "MultLone" | ...
        let parts: Vec<String> = variants.iter()
            .map(|v| format!("\"{}\"", v))
            .collect();
        writeln!(out, "export type {} = {};", s.name, parts.join(" | ")).unwrap();
    } else {
        // Discriminated union with kind field
        for v in variants {
            let child = struct_map.get(v.as_str());
            let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
            // Combine parent fields + child fields
            let all_fields: Vec<&IRField> = parent_fields.iter().chain(child_fields.iter().copied()).collect();
            writeln!(out, "export interface {} {{", v).unwrap();
            writeln!(out, "  readonly kind: \"{}\";", v).unwrap();
            for f in &all_fields {
                let resolved_target = resolve_type(TargetLang::TypeScript, &f.target);
                let type_str = if let Some(vt) = &f.value_type {
                    let resolved_vt = resolve_type(TargetLang::TypeScript, vt);
                    format!("Map<{}, {}>", resolved_target, resolved_vt)
                } else if let Some(raw) = &f.raw_union_type {
                    if f.mult == Multiplicity::Lone {
                        format!("{} | null", raw)
                    } else {
                        raw.clone()
                    }
                } else {
                    mult_to_ts_type(&resolved_target, &f.mult)
                };
                let readonly = if f.is_var { "" } else { "readonly " };
                writeln!(out, "  {readonly}{}: {};", f.name, type_str).unwrap();
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

/// Generate helpers.ts containing TC / RTC functions.
fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "import type * as M from './models';").unwrap();
    writeln!(out).unwrap();

    let (tc_fields, rtc_fields) = collect_closure_fields(ir);

    for tc in &tc_fields {
        generate_tc_function(&mut out, tc);
    }
    for rtc in &rtc_fields {
        generate_rtc_function(&mut out, rtc);
    }

    out
}

fn collect_closure_fields(ir: &OxidtrIR) -> (Vec<expr_translator::TCField>, Vec<expr_translator::TCField>) {
    let mut tc_fields = Vec::new();
    let mut rtc_fields = Vec::new();
    let mut push_expr = |expr: &crate::parser::ast::Expr| {
        tc_fields.extend(expr_translator::extract_tc_fields(expr, ir));
        rtc_fields.extend(expr_translator::extract_rtc_fields(expr, ir));
    };
    for c in &ir.constraints {
        push_expr(&c.expr);
    }
    for p in &ir.properties {
        push_expr(&p.expr);
    }
    for op in &ir.operations {
        for e in &op.body {
            push_expr(e);
        }
    }
    tc_fields.sort_by(|a, b| (&a.sig_name, &a.field_name).cmp(&(&b.sig_name, &b.field_name)));
    tc_fields.dedup();
    rtc_fields.sort_by(|a, b| (&a.sig_name, &a.field_name).cmp(&(&b.sig_name, &b.field_name)));
    rtc_fields.dedup();
    (tc_fields, rtc_fields)
}

fn ir_uses_tc(ir: &OxidtrIR) -> bool {
    ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr))
        || ir.operations.iter().any(|op| op.body.iter().any(expr_uses_tc))
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

fn generate_rtc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("rtc{}", capitalize(&tc.field_name));
    let tc_name = format!("tc{}", capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "/** Reflexive-transitive closure for {sig}.{field} (id ∪ ^{field}). */").unwrap();
    writeln!(out, "export function {fn_name}(start: M.{sig}): M.{sig}[] {{").unwrap();
    writeln!(out, "  const result: M.{sig}[] = [start];").unwrap();
    writeln!(out, "  result.push(...{tc_name}(start));").unwrap();
    writeln!(out, "  return result;").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ── operations.ts ──────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "import type * as M from './models';").unwrap();
    writeln!(out).unwrap();

    for op in &ir.operations {
        if op.receiver_sig.is_some() {
            continue;
        }
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

        // An Alloy `pred` is a formula, not a procedure: it denotes true or
        // false, and its body is the conjunction of its clauses. Generating it
        // as `void` with a throw left it uncallable, which is why the
        // `Op(..) implies Fact(..)` cross-tests had nothing to call (#82).
        let return_str = match &op.return_type {
            Some(rt) => ts_return_type(&rt.type_name, &rt.mult),
            None => "boolean".to_string(),
        };

        writeln!(out, "export function {fn_name}({params}): {return_str} {{").unwrap();
        let env = crate::backend::type_env::operation_env(op);
        if op.body.is_empty() {
            // A pred with no clauses constrains nothing.
            writeln!(out, "  return true;").unwrap();
        } else if op.return_type.is_some() {
            let body = expr_translator::translate_with_env(&op.body[0], ir, &env);
            writeln!(out, "  return {body};").unwrap();
        } else {
            let conjuncts: Vec<String> = op.body.iter()
                .map(|e| expr_translator::translate_with_env(e, ir, &env))
                .collect();
            writeln!(out, "  return {};", conjuncts.join(" && ")).unwrap();
        }
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
    // Having no field is not having no value: `sig Person {}` is `{}`, and
    // materialising its domain empty is a sample nothing can be true of.
    // What has no factory is what has no finite value at all (#109, #105).
    let (terminating, _) = crate::backend::terminating_types(ir);
    let has_fixture: HashSet<String> = ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum
            && terminating.contains(&s.name))
        .map(|s| s.name.clone()).collect();

    // Check if any expression uses TC functions → need helpers import
    let needs_helpers = ir_uses_tc(ir);

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
        let params = expr_translator::extract_params(&prop.expr, &sig_names, ir);
        let body = expr_translator::translate_with_ir(&prop.expr, ir);

        // An `assert` carries temporal operators just as a `fact` does, and
        // translating its operand alone silently drops them: `eventually P`
        // became `P`, `P until Q` became `P && Q` (#78). Route it through the
        // same trace checkers the fact path already emits.
        let temporal_kind = analyze::expr_temporal_kind(&prop.expr);
        if matches!(
            temporal_kind,
            Some(analyze::TemporalKind::Liveness)
                | Some(analyze::TemporalKind::PastLiveness)
                | Some(analyze::TemporalKind::Binary)
        ) {
            let label = match temporal_kind {
                Some(analyze::TemporalKind::Binary) => "binary temporal",
                _ => "liveness",
            };
            // Name the checker exactly as `emit_temporal_trace_checkers` will,
            // so the test actually calls it. An empty body would leave the
            // checker unreferenced — a test that asserts nothing, which is the
            // failure #74/#81 were about.
            let camel = to_camel_case(&prop.name);
            let checker = match temporal_kind {
                Some(analyze::TemporalKind::Binary) => analyze::find_temporal_binary_with_bindings(&prop.expr)
                    .map(|(op, _, _, _)| {
                        let op_name = match op {
                            TemporalBinaryOp::Until => "until",
                            TemporalBinaryOp::Since => "since",
                            TemporalBinaryOp::Release => "release",
                            TemporalBinaryOp::Triggered => "triggered",
                        };
                        format!("check_{op_name}_{camel}")
                    }),
                Some(analyze::TemporalKind::PastLiveness) => Some(format!("check_pastLiveness_{camel}")),
                _ => Some(format!("check_liveness_{camel}")),
            };
            writeln!(out, "  it('{test_name}', () => {{").unwrap();
            writeln!(out, "    // {label}: full verification needs a trace; an empty trace").unwrap();
            writeln!(out, "    // can never satisfy it, which at least exercises the checker.").unwrap();
            match &checker {
                Some(c) => {
                    writeln!(out, "    const trace: M.{}[][] = [];",
                        params.first().map(|(_, t)| t.as_str()).unwrap_or("never")).unwrap();
                    writeln!(out, "    expect({c}(trace)).toBe(false);").unwrap();
                }
                None => {
                    writeln!(out, "    // oxidtr: no checker emitted for this shape").unwrap();
                }
            }
            writeln!(out, "  }});").unwrap();
            writeln!(out).unwrap();
            emit_temporal_trace_checkers(&mut out, &prop.name, &prop.expr, &params, ir, temporal_kind);
            continue;
        }

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
        // Alloy 6: temporal facts with prime → generate transition test
        if analyze::expr_contains_prime(&constraint.expr) {
            let test_name = format!("transition {fact_name}");
            let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "  /** @temporal Transition constraint: {fact_name} */").unwrap();
            writeln!(out, "  /** Verifies: pre→post state relationship ({desc}) */").unwrap();
            writeln!(out, "  it('{}', () => {{", test_name).unwrap();
            for (pname, tname) in &params {
                if has_fixture.contains(tname) {
                    writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
                } else {
                    writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
                }
                writeln!(out, "    const next_{pname}: M.{tname}[] = structuredClone({pname});").unwrap();
            }
            if let Some((_kind, bindings, inner_body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                let rewritten_body = analyze::rewrite_prime_as_post_state(inner_body);
                let body_str = expr_translator::translate_with_ir(&rewritten_body, ir);
                let bind_vars: Vec<String> = bindings.iter()
                    .flat_map(|b| b.vars.clone()).collect();
                // The pre/post pair is walked over the *binding's own* domain.
                // `params[0]` is whichever sig sorted first, so `all f: Foo`
                // iterated some other sig's list entirely (#110 item 4).
                let bound_pname = match &bindings[0].domain {
                    crate::parser::ast::Expr::VarRef(sig) => {
                        params.iter().find(|(_, t)| t == sig).map(|(p, _)| p.clone())
                    }
                    _ => None,
                };
                match (bind_vars.as_slice(), bound_pname) {
                    ([v], Some(pname)) => {
                        writeln!(out, "    {pname}.forEach(({v}, i) => {{").unwrap();
                        writeln!(out, "      const next_{v} = next_{pname}[i];").unwrap();
                        writeln!(out, "      expect({body_str}).toBe(true);").unwrap();
                        writeln!(out, "    }});").unwrap();
                    }
                    _ => {
                        writeln!(out, "    // oxidtr: skipped — a transition over {} binding(s) has no \
                            pre/post pairing to walk. See #104.", bind_vars.len()).unwrap();
                    }
                }
            } else {
                let rewritten = analyze::rewrite_prime_as_post_state(&constraint.expr);
                let body = expr_translator::translate_with_ir(&rewritten, ir);
                writeln!(out, "    expect({body}).toBe(true);").unwrap();
            }
            writeln!(out, "  }});").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        // Use temporal classification for test name prefix
        let temporal_kind = analyze::expr_temporal_kind(&constraint.expr);
        let test_prefix = match temporal_kind {
            Some(analyze::TemporalKind::Liveness) => "liveness",
            Some(analyze::TemporalKind::PastInvariant) => "past_invariant",
            Some(analyze::TemporalKind::PastLiveness) => "past_liveness",
            Some(analyze::TemporalKind::Step) => "step",
            Some(analyze::TemporalKind::Binary) => "temporal",
            _ => "invariant",
        };
        let test_name = format!("{test_prefix} {fact_name}");
        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        let ownership = super::detect_ownership_pattern(&constraint.expr, ir, ts_param_name);

        if let Some(ref kind) = temporal_kind {
            let note = match kind {
                analyze::TemporalKind::Liveness | analyze::TemporalKind::PastLiveness =>
                    " — liveness property: cannot be fully verified at runtime; static test approximates via implies",
                analyze::TemporalKind::Binary =>
                    " — binary temporal: requires trace-based verification",
                _ => "",
            };
            writeln!(out, "  /** @temporal {:?} constraint: {fact_name}{note} */", kind).unwrap();
        }

        // Binary temporal: static test cannot meaningfully assert the body
        if temporal_kind == Some(analyze::TemporalKind::Binary) {
            let op_label = if let Some((op, _, _)) = analyze::find_temporal_binary(&constraint.expr) {
                match op {
                    TemporalBinaryOp::Until => "until",
                    TemporalBinaryOp::Since => "since",
                    TemporalBinaryOp::Release => "release",
                    TemporalBinaryOp::Triggered => "triggered",
                }
            } else { "binary" };
            let camel_name = to_camel_case(&fact_name);
            writeln!(out, "  it('{}', () => {{", test_name).unwrap();
            writeln!(out, "    // binary temporal: requires trace-based verification; see check_{op_label}_{camel_name}").unwrap();
            writeln!(out, "  }});").unwrap();
            writeln!(out).unwrap();
        } else if matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            // Liveness/past_liveness: cannot be verified with single snapshot
            let kind_label = if temporal_kind == Some(analyze::TemporalKind::Liveness) {
                "liveness" } else { "past_liveness" };
            let camel_name = to_camel_case(&fact_name);
            writeln!(out, "  it('{}', () => {{", test_name).unwrap();
            writeln!(out, "    // {kind_label}: requires trace-based verification; see check_{kind_label}_{camel_name}").unwrap();
            writeln!(out, "  }});").unwrap();
            writeln!(out).unwrap();
        } else {
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
        } // end non-binary temporal

        emit_temporal_trace_checkers(&mut out, &fact_name, &constraint.expr, &params, ir, temporal_kind);
    }

    // Boundary value tests — inline expressions (Feature 5)
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
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

    // --- Anomaly tests ---
    let anomalies = analyze::detect_anomalies(ir);
    if !anomalies.is_empty() {
        writeln!(out, "  // --- Anomaly tests: edge-case coverage ---").unwrap();
        writeln!(out).unwrap();

        let mut anomaly_sigs: std::collections::BTreeMap<String, Vec<&analyze::AnomalyPattern>> =
            std::collections::BTreeMap::new();
        for a in &anomalies {
            let sig = match a {
                analyze::AnomalyPattern::UnconstrainedField { sig_name, .. } => sig_name,
                analyze::AnomalyPattern::UnboundedCollection { sig_name, .. } => sig_name,
                analyze::AnomalyPattern::UnguardedSelfRef { sig_name, .. } => sig_name,
            };
            anomaly_sigs.entry(sig.clone()).or_default().push(a);
        }

        for (sig_name, patterns) in &anomaly_sigs {
            if !has_fixture.contains(sig_name) { continue; }
            for pattern in patterns {
                match pattern {
                    analyze::AnomalyPattern::UnconstrainedField { field_name, .. } => {
                        writeln!(out, "  it('anomaly: {sig_name}.{field_name} is unconstrained', () => {{").unwrap();
                        writeln!(out, "    const instance = fix.default{sig_name}();").unwrap();
                        writeln!(out, "    expect(instance.{field_name}).toBeDefined();").unwrap();
                        writeln!(out, "  }});").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        writeln!(out, "  it('anomaly: {sig_name}.{field_name} empty edge case', () => {{").unwrap();
                        writeln!(out, "    const instance = fix.anomalyEmpty{sig_name}();").unwrap();
                        writeln!(out, "    expect(instance.{field_name}).toBeDefined();").unwrap();
                        writeln!(out, "  }});").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        writeln!(out, "  it('anomaly: {sig_name}.{field_name} self-referential without guard', () => {{").unwrap();
                        writeln!(out, "    const instance = fix.default{sig_name}();").unwrap();
                        writeln!(out, "    expect(instance.{field_name}).toBeDefined();").unwrap();
                        writeln!(out, "  }});").unwrap();
                        writeln!(out).unwrap();
                    }
                }
            }
        }
    }

    // --- Coverage tests: pairwise fact combinations ---
    let coverage = analyze::fact_coverage(ir);
    if !coverage.pairwise.is_empty() {
        writeln!(out, "  // --- Coverage tests: fact × fact pairwise ---").unwrap();
        writeln!(out).unwrap();

        let mut seen_cover_tests: HashSet<String> = HashSet::new();
        for pair in &coverage.pairwise {
            if !has_fixture.contains(&pair.sig_name) { continue; }

            let test_name = format!("cover: {} × {}", pair.fact_a, pair.fact_b);
            if !seen_cover_tests.insert(test_name.clone()) { continue; }

            let constraint_a = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_a));
            let constraint_b = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_b));

            let body_a = constraint_a.map(|c| expr_translator::translate_with_ir(&c.expr, ir));
            let body_b = constraint_b.map(|c| expr_translator::translate_with_ir(&c.expr, ir));

            // Collect params from both constraints to avoid undefined variables
            let mut all_params: Vec<(String, String)> = Vec::new();
            let mut param_names_seen: HashSet<String> = HashSet::new();
            if let Some(c) = constraint_a {
                for p in expr_translator::extract_params(&c.expr, &sig_names, ir) {
                    if param_names_seen.insert(p.0.clone()) {
                        all_params.push(p);
                    }
                }
            }
            if let Some(c) = constraint_b {
                for p in expr_translator::extract_params(&c.expr, &sig_names, ir) {
                    if param_names_seen.insert(p.0.clone()) {
                        all_params.push(p);
                    }
                }
            }

            writeln!(out, "  it.skip('{}', () => {{", test_name).unwrap();
            for (pname, tname) in &all_params {
                if has_fixture.contains(tname) {
                    writeln!(out, "    const {pname}: M.{tname}[] = [fix.default{tname}()];").unwrap();
                } else {
                    writeln!(out, "    const {pname}: M.{tname}[] = [];").unwrap();
                }
            }
            if let (Some(a), Some(b)) = (&body_a, &body_b) {
                writeln!(out, "    expect({a}).toBe(true);").unwrap();
                writeln!(out, "    expect({b}).toBe(true);").unwrap();
            }
            writeln!(out, "  }});").unwrap();
            writeln!(out).unwrap();
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
        Expr::TransitiveClosure(_) | Expr::ReflexiveClosure(_) => true,
        Expr::FieldAccess { base, .. } => expr_uses_tc(base),
        Expr::Cardinality(inner) | Expr::Not(inner) => expr_uses_tc(inner),
        Expr::MultFormula { expr: inner, .. } => expr_uses_tc(inner),
        Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            expr_uses_tc(left) || expr_uses_tc(right)
        }
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| expr_uses_tc(&b.domain)) || expr_uses_tc(body)
        }
        Expr::Prime(inner) => expr_uses_tc(inner),
        Expr::TemporalUnary { expr: inner, .. } => expr_uses_tc(inner),
        Expr::TemporalBinary { left, right, .. } => {
            expr_uses_tc(left) || expr_uses_tc(right)
        }
        Expr::FunApp { receiver, args, .. } => receiver.as_ref().map_or(false, |r| expr_uses_tc(r)) || args.iter().any(|a| expr_uses_tc(a)),
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

    // Generate enum default fixtures. A union of string literals takes the
    // variant's name; a discriminated union takes an object tagged with `kind`
    // and carrying every field the interface declares — returning the bare name
    // there was `Type 'string' is not assignable to type 'L'`.
    let (_, enum_witness) = crate::backend::terminating_types(ir);
    for s in &ir.structures {
        if !s.is_enum { continue; }
        let variants = match children.get(&s.name) {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        if is_string_literal_union(s, variants, &struct_map) {
            // Any variant will do — none of them carries a field.
            let Some(variant) = variants.first() else { continue };
            writeln!(out, "/** Factory: default value for enum {} */", s.name).unwrap();
            writeln!(out, "export function default{}(): M.{} {{", s.name, s.name).unwrap();
            writeln!(out, "  return \"{}\";", variant).unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
            continue;
        }
        // The case whose payload was satisfiable when the enum was admitted;
        // any other may lead straight back into the enum.
        let Some(variant) = enum_witness.get(&s.name) else { continue };
        let own: Vec<&IRField> = struct_map.get(variant.as_str())
            .map(|c| c.fields.iter().collect()).unwrap_or_default();
        writeln!(out, "/** Factory: default value for enum {} */", s.name).unwrap();
        writeln!(out, "export function default{}(): M.{} {{", s.name, s.name).unwrap();
        writeln!(out, "  return {{").unwrap();
        writeln!(out, "    kind: \"{variant}\",").unwrap();
        for f in s.fields.iter().chain(own.into_iter()) {
            let val = if f.value_type.is_some() {
                "new Map()".to_string()
            } else {
                ts_default_value(&f.target, &f.mult)
            };
            writeln!(out, "    {}: {val},", f.name).unwrap();
        }
        writeln!(out, "  }};").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    // Collect which types have fixture factories (for populating set/seq fields)
    let (terminating, _) = crate::backend::terminating_types(ir);
    let fixture_types: HashSet<String> = ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum
            && terminating.contains(&s.name))
        .map(|s| s.name.clone())
        .collect();

    for s in &ir.structures {
        if variant_names.contains(&s.name) || s.is_enum { continue; }
        // A field-less sig is `{}` — a value, and one other fixtures reference.
        if !terminating.contains(&s.name) { continue; }

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

    // Anomaly fixtures: empty collections for unbounded set/seq fields
    let anomalies = analyze::detect_anomalies(ir);
    let mut anomaly_sigs_done: std::collections::HashSet<String> = std::collections::HashSet::new();
    for anomaly in &anomalies {
        if let analyze::AnomalyPattern::UnboundedCollection { sig_name, .. } = anomaly {
            if anomaly_sigs_done.contains(sig_name) { continue; }
            let s = match ir.structures.iter().find(|s| s.name == *sig_name) {
                Some(s) => s,
                None => continue,
            };
            if variant_names.contains(&s.name) || s.is_enum || s.fields.is_empty() { continue; }
            anomaly_sigs_done.insert(sig_name.clone());

            writeln!(out, "/** Anomaly fixture: all collections empty (edge case) */").unwrap();
            writeln!(out, "export function anomalyEmpty{}(): M.{} {{", sig_name, sig_name).unwrap();
            writeln!(out, "  return {{").unwrap();
            for f in &s.fields {
                let val = if f.value_type.is_some() {
                    "new Map()".to_string()
                } else {
                    match &f.mult {
                        Multiplicity::Set => "new Set()".to_string(),
                        Multiplicity::Seq => "[]".to_string(),
                        _ => ts_default_value(&f.target, &f.mult),
                    }
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

/// As `ts_return_type`, but for a declaration inside models.ts, where the
/// generated types are in scope unqualified.
fn ts_local_return_type(type_name: &str, mult: &Multiplicity) -> String {
    let base = if is_native_type_alias(type_name) {
        resolve_type(TargetLang::TypeScript, type_name)
    } else {
        type_name.to_string()
    };
    match mult {
        Multiplicity::One => base,
        Multiplicity::Lone => format!("{base} | null"),
        Multiplicity::Set => format!("Set<{base}>"),
        Multiplicity::Seq => format!("{base}[]"),
    }
}

fn ts_return_type(type_name: &str, mult: &Multiplicity) -> String {
    // `Int`/`Str`/`Bool` are Alloy marker sigs, not emitted types — `M.Int`
    // names nothing. Harmless while the body was a `throw`; now that the body
    // returns a value, the signature has to be a real type.
    let base = if is_native_type_alias(type_name) {
        resolve_type(TargetLang::TypeScript, type_name)
    } else {
        format!("M.{type_name}")
    };
    match mult {
        Multiplicity::One => base,
        Multiplicity::Lone => format!("{base} | null"),
        Multiplicity::Set => format!("Set<{base}>"),
        Multiplicity::Seq => format!("{base}[]"),
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

/// The zero value for an Alloy marker sig, which has no generated factory:
/// `default Int()` names nothing (a ReferenceError at fixture-construction
/// time, so every test touching such a fixture failed).
fn ts_native_zero(target: &str) -> Option<&'static str> {
    match target {
        "Int" => Some("0"),
        "Str" => Some("''"),
        "Bool" => Some("false"),
        "Float" => Some("0"),
        _ => None,
    }
}

fn ts_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    if let (Multiplicity::One, Some(zero)) = (mult, ts_native_zero(target)) {
        return zero.to_string();
    }
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

    // Generate validators for concrete sigs with fields or constraints
    let sigs_to_validate: Vec<&StructureNode> = ir.structures.iter()
        .filter(|s| {
            if variant_names.contains(&s.name) || s.is_enum { return false; }
            if !s.fields.is_empty() { return true; }
            // Also include sigs that have constraints (e.g., Exhaustive) even without fields
            let constraints = analyze::constraints_for_sig(ir, &s.name);
            !constraints.is_empty()
        })
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
                analyze::ConstraintInfo::Implication { condition, consequent, .. } => {
                    let cond = translate_validator_expr(condition, &s.name, &param_name);
                    let cons = translate_validator_expr(consequent, &s.name, &param_name);
                    let desc = format!("{} implies {}", analyze::describe_expr(condition), analyze::describe_expr(consequent));
                    writeln!(out, "  if ({cond} && !({cons})) errors.push(\"{}\");", desc.replace('"', "\\\"")).unwrap();
                }
                analyze::ConstraintInfo::Iff { left, right, .. } => {
                    let l = translate_validator_expr(left, &s.name, &param_name);
                    let r = translate_validator_expr(right, &s.name, &param_name);
                    let desc = format!("{} iff {}", analyze::describe_expr(left), analyze::describe_expr(right));
                    writeln!(out, "  if (({l}) !== ({r})) errors.push(\"{}\");", desc.replace('"', "\\\"")).unwrap();
                }
                analyze::ConstraintInfo::Prohibition { condition, .. } => {
                    let cond = translate_validator_expr(condition, &s.name, &param_name);
                    let desc = analyze::describe_expr(condition);
                    writeln!(out, "  if ({cond}) errors.push(\"prohibited: {}\");", desc.replace('"', "\\\"")).unwrap();
                }
                analyze::ConstraintInfo::Disjoint { left, right, .. } => {
                    let left_field = left.rsplit('.').next().unwrap_or(left);
                    let right_field = right.rsplit('.').next().unwrap_or(right);
                    writeln!(out, "  {{ const leftSet = new Set({param_name}.{left_field}); if ({param_name}.{right_field}.some((e: unknown) => leftSet.has(e))) errors.push(\"{left_field} and {right_field} must not overlap (disjoint constraint)\"); }}").unwrap();
                }
                analyze::ConstraintInfo::Exhaustive { categories, .. } => {
                    let cats = categories.join(", ");
                    let checks: Vec<String> = categories.iter().map(|cat| {
                        let parts: Vec<&str> = cat.split('.').collect();
                        if parts.len() == 2 {
                            format!("{}.{}.has({param_name})", parts[0], parts[1])
                        } else {
                            format!("{cat}.has({param_name})")
                        }
                    }).collect();
                    let condition = checks.join(" || ");
                    writeln!(out, "  if (!({condition})) errors.push(\"must belong to one of [{cats}]\");").unwrap();
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

/// Translate an Alloy expression to TypeScript for single-instance validator context.
/// `sig_name` is the sig name used by substitute_var, `param` is the TS parameter name.
fn translate_validator_expr(expr: &crate::parser::ast::Expr, sig_name: &str, param: &str) -> String {
    use crate::parser::ast::{Expr, LogicOp, QuantKind};
    match expr {
        Expr::VarRef(name) => {
            if name == sig_name { param.to_string() } else { name.clone() }
        }
        Expr::IntLiteral(n) => n.to_string(),
        Expr::FieldAccess { base, field } => {
            format!("{}.{}", translate_validator_expr(base, sig_name, param), field)
        }
        Expr::Comparison { op, left, right } => {
            let l = translate_validator_expr(left, sig_name, param);
            let r = translate_validator_expr(right, sig_name, param);
            let o = match op {
                CompareOp::Eq => "===",
                CompareOp::NotEq => "!==",
                CompareOp::In => return format!("{r}.includes({l})"),
                CompareOp::Lt => "<",
                CompareOp::Gt => ">",
                CompareOp::Lte => "<=",
                CompareOp::Gte => ">=",
            };
            format!("{l} {o} {r}")
        }
        Expr::BinaryLogic { op, left, right } => {
            let l = translate_validator_expr(left, sig_name, param);
            let r = translate_validator_expr(right, sig_name, param);
            match op {
                LogicOp::And => format!("{l} && {r}"),
                LogicOp::Or => format!("{l} || {r}"),
                LogicOp::Implies => format!("!({l}) || {r}"),
                LogicOp::Iff => format!("({l}) === ({r})"),
            }
        }
        Expr::Not(inner) => format!("!({})", translate_validator_expr(inner, sig_name, param)),
        Expr::MultFormula { kind, expr: inner } => {
            let e = translate_validator_expr(inner, sig_name, param);
            match kind {
                QuantKind::Some => format!("{e} != null"),
                QuantKind::No => format!("{e} == null"),
                _ => e,
            }
        }
        Expr::Cardinality(inner) => {
            format!("{}.length", translate_validator_expr(inner, sig_name, param))
        }
        _ => analyze::describe_expr(expr), // fallback: human-readable
    }
}

/// Emit the trace-checker functions a temporal constraint needs.
///
/// Shared by the `fact` and `assert` paths. Only the fact path used to call it,
/// so an `assert` erased its temporal operators entirely — `eventually P`
/// became `P` and `P until Q` became `P && Q` (#78).
fn emit_temporal_trace_checkers(
    out: &mut String,
    name: &str,
    expr: &crate::parser::ast::Expr,
    params: &[(String, String)],
    ir: &OxidtrIR,
    temporal_kind: Option<analyze::TemporalKind>,
) {
    let constraint = TemporalSource { expr };
    // Generate trace checker functions for temporal constraints
    if let Some(kind) = temporal_kind {
        let camel_name = to_camel_case(name);
        match kind {
            analyze::TemporalKind::Liveness | analyze::TemporalKind::PastLiveness => {
                let kind_label = if kind == analyze::TemporalKind::Liveness {
                    "liveness" } else { "pastLiveness" };
                let semantics = if kind == analyze::TemporalKind::Liveness {
                    "property holds in at least one future state"
                } else {
                    "property held in at least one past state"
                };
                let trace_body = expr_translator::translate_trace_body(&constraint.expr, ir);
                writeln!(out, "  /** Trace checker for {kind_label}: {semantics}. */").unwrap();
                if params.len() == 1 {
                    let (pname, tname) = &params[0];
                    writeln!(out, "  function check_{kind_label}_{camel_name}(trace: M.{tname}[][]): boolean {{").unwrap();
                    writeln!(out, "    return trace.some({pname} => {trace_body});").unwrap();
                } else {
                    let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("M.{t}[]")).collect();
                    let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                    writeln!(out, "  function check_{kind_label}_{camel_name}(trace: [{}][]): boolean {{", tuple_types.join(", ")).unwrap();
                    writeln!(out, "    return trace.some(([{}]) => {trace_body});", tuple_names.join(", ")).unwrap();
                }
                writeln!(out, "  }}").unwrap();
                writeln!(out).unwrap();
            }
            analyze::TemporalKind::Binary => {
                if let Some((op, left, right, bound_vars)) =
                    analyze::find_temporal_binary_with_bindings(&constraint.expr)
                {
                    let op_name = match op {
                        TemporalBinaryOp::Until => "until",
                        TemporalBinaryOp::Since => "since",
                        TemporalBinaryOp::Release => "release",
                        TemporalBinaryOp::Triggered => "triggered",
                    };
                    let semantics = match op {
                        TemporalBinaryOp::Until => "left holds until right becomes true",
                        TemporalBinaryOp::Since => "left has held since right was true",
                        TemporalBinaryOp::Release => "right holds until left releases it",
                        TemporalBinaryOp::Triggered => "left triggers right",
                    };
                    writeln!(out, "  /** Trace checker for {op_name}: {semantics}. */").unwrap();
                    if params.len() == 1 {
                        let (pname, tname) = &params[0];
                        writeln!(out, "  function check_{op_name}_{camel_name}(trace: M.{tname}[][]): boolean {{").unwrap();
                        // Use snapshot-aware translation: each trace element is M.T[]
                        let snap = pname.as_str();
                        let left_pred = expr_translator::translate_trace_binary_snapshot(left, snap, &bound_vars, ir);
                        let right_pred = expr_translator::translate_trace_binary_snapshot(right, snap, &bound_vars, ir);
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "    const pos = trace.findIndex({snap} => {right_pred});").unwrap();
                                writeln!(out, "    return pos >= 0 && trace.slice(0, pos).every({snap} => {left_pred});").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "    let pos = -1;").unwrap();
                                writeln!(out, "    for (let i = trace.length - 1; i >= 0; i--) {{ if ({}) {{ pos = i; break; }} }}", right_pred.replace(snap, &format!("trace[i]"))).unwrap();
                                writeln!(out, "    return pos >= 0 && trace.slice(pos).every({snap} => {left_pred});").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "    const pos = trace.findIndex({snap} => {left_pred});").unwrap();
                                writeln!(out, "    return pos >= 0 ? trace.slice(0, pos + 1).every({snap} => {right_pred}) : trace.every({snap} => {right_pred});").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "    return trace.every(({snap}, i) => {{").unwrap();
                                writeln!(out, "      if ({right_pred}) {{ return trace.slice(0, i + 1).some({snap} => {left_pred}); }} else {{ return true; }}").unwrap();
                                writeln!(out, "    }});").unwrap();
                            }
                        }
                    } else {
                        let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("M.{t}[]")).collect();
                        let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                        let pnames = tuple_names.join(", ");
                        writeln!(out, "  function check_{op_name}_{camel_name}(trace: [{}][]): boolean {{", tuple_types.join(", ")).unwrap();
                        let snap = format!("[{pnames}]");
                        let left_pred = expr_translator::translate_trace_binary_snapshot(left, &snap, &bound_vars, ir);
                        let right_pred = expr_translator::translate_trace_binary_snapshot(right, &snap, &bound_vars, ir);
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "    const pos = trace.findIndex({snap} => {right_pred});").unwrap();
                                writeln!(out, "    return pos >= 0 && trace.slice(0, pos).every({snap} => {left_pred});").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "    let pos = -1;").unwrap();
                                writeln!(out, "    for (let i = trace.length - 1; i >= 0; i--) {{ if ({}) {{ pos = i; break; }} }}", right_pred.replace(snap.as_str(), "trace[i]")).unwrap();
                                writeln!(out, "    return pos >= 0 && trace.slice(pos).every({snap} => {left_pred});").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "    const pos = trace.findIndex({snap} => {left_pred});").unwrap();
                                writeln!(out, "    return pos >= 0 ? trace.slice(0, pos + 1).every({snap} => {right_pred}) : trace.every({snap} => {right_pred});").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "    return trace.every(({snap}, i) => {{").unwrap();
                                writeln!(out, "      if ({right_pred}) {{ return trace.slice(0, i + 1).some({snap} => {left_pred}); }} else {{ return true; }}").unwrap();
                                writeln!(out, "    }});").unwrap();
                            }
                        }
                    }
                    writeln!(out, "  }}").unwrap();
                    writeln!(out).unwrap();
                }
            }
            _ => {} // Invariant, PastInvariant, Step — static tests are sufficient
        }
    }
}

/// Adapter so the extracted block keeps reading `constraint.expr`.
struct TemporalSource<'a> {
    expr: &'a crate::parser::ast::Expr,
}
