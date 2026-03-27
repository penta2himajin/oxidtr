pub mod expr_translator;

use crate::backend::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity};
use crate::analyze;
use crate::backend;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = CsContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "Models.cs".to_string(),
        content: generate_models(ir, &ctx),
    });

    if !ir.operations.is_empty() {
        files.push(GeneratedFile {
            path: "Operations.cs".to_string(),
            content: generate_operations(ir),
        });
    }

    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        files.push(GeneratedFile {
            path: "Tests.cs".to_string(),
            content: generate_tests(ir),
        });
    }

    files.push(GeneratedFile {
        path: "Fixtures.cs".to_string(),
        content: generate_fixtures(ir, &ctx),
    });

    files
}

// ── Context ──────────────────────────────────────────────────────────────────

struct CsContext {
    children: HashMap<String, Vec<String>>,
    variant_names: HashSet<String>,
    struct_map: HashMap<String, StructureNode>,
}

impl CsContext {
    fn from_ir(ir: &OxidtrIR) -> Self {
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        for s in &ir.structures {
            if let Some(parent) = &s.parent {
                children.entry(parent.clone()).or_default().push(s.name.clone());
            }
        }
        let enum_parents: HashSet<String> = ir.structures.iter()
            .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
        let variant_names: HashSet<String> = ir.structures.iter()
            .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
            .map(|s| s.name.clone()).collect();
        let struct_map: HashMap<String, StructureNode> = ir.structures.iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect();
        CsContext { children, variant_names, struct_map }
    }

    fn is_variant(&self, name: &str) -> bool {
        self.variant_names.contains(name)
    }
}

// ── Models.cs ────────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR, ctx: &CsContext) -> String {
    let mut out = String::new();
    writeln!(out, "using System.Collections.Generic;").unwrap();
    writeln!(out).unwrap();

    for s in &ir.structures {
        if ctx.is_variant(&s.name) { continue; }

        if s.is_enum {
            generate_enum(&mut out, s, ctx);
        } else {
            generate_class(&mut out, s, ir, ctx);
        }
        writeln!(out).unwrap();
    }

    // Derived fields: receiver functions → extension methods / partial classes
    generate_derived_fields(&mut out, ir);

    out
}

fn generate_derived_fields(out: &mut String, ir: &OxidtrIR) {
    use std::collections::HashMap;
    let mut by_sig: HashMap<String, Vec<&OperationNode>> = HashMap::new();
    for op in &ir.operations {
        if let Some(ref sig) = op.receiver_sig {
            by_sig.entry(sig.clone()).or_default().push(op);
        }
    }

    for (sig_name, ops) in &by_sig {
        writeln!(out, "public static class {sig_name}Extensions").unwrap();
        writeln!(out, "{{").unwrap();
        for op in ops {
            let return_type = match &op.return_type {
                Some(rt) => mult_to_cs_type(&rt.type_name, &rt.mult),
                None => "void".to_string(),
            };

            if op.params.is_empty() {
                // No params → extension property (C# uses method for this)
                writeln!(out, "    public static {return_type} {} => throw new NotImplementedException(\"oxidtr: implement {}\");", capitalize(&op.name), op.name).unwrap();
            } else {
                let params = op.params.iter().map(|p| {
                    let type_str = mult_to_cs_type(&p.type_name, &p.mult);
                    format!("{type_str} {}", to_camel_case(&p.name))
                }).collect::<Vec<_>>().join(", ");
                writeln!(out, "    public static {return_type} {}(this {sig_name} self, {params})", capitalize(&op.name)).unwrap();
                writeln!(out, "    {{").unwrap();
                writeln!(out, "        throw new NotImplementedException(\"oxidtr: implement {}\");", op.name).unwrap();
                writeln!(out, "    }}").unwrap();
            }
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

fn generate_class(out: &mut String, s: &StructureNode, ir: &OxidtrIR, _ctx: &CsContext) {
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        if s.is_var {
            writeln!(out, "// Alloy var sig: instances change across state transitions").unwrap();
        }
        writeln!(out, "public class {}", s.name).unwrap();
        writeln!(out, "{{").unwrap();
        writeln!(out, "    public static readonly {} Instance = new {}();", s.name, s.name).unwrap();
        writeln!(out, "}}").unwrap();
        return;
    }

    if s.is_var {
        writeln!(out, "// Alloy var sig: instances change across state transitions").unwrap();
    }

    let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
    if !constraint_names.is_empty() {
        writeln!(out, "// Invariants:").unwrap();
        for cn in &constraint_names {
            writeln!(out, "// - {cn}").unwrap();
        }
    }

    writeln!(out, "public class {}", s.name).unwrap();
    writeln!(out, "{{").unwrap();
    for f in &s.fields {
        if f.mult == Multiplicity::Seq {
            writeln!(out, "    // @alloy: seq").unwrap();
        }
        let type_str = mult_to_cs_type(&f.target, &f.mult);
        writeln!(out, "    public {} {} {{ get; set; }}", type_str, capitalize(&f.name)).unwrap();
    }

    // Generate Validate() method for constraint validation
    let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
    let disj = analyze::disj_fields(ir);
    let has_validation = sig_constraints.iter().any(|c| matches!(c,
        analyze::ConstraintInfo::Disjoint { .. } | analyze::ConstraintInfo::Exhaustive { .. }
        | analyze::ConstraintInfo::NoSelfRef { .. } | analyze::ConstraintInfo::Acyclic { .. }
        | analyze::ConstraintInfo::FieldOrdering { .. }
        | analyze::ConstraintInfo::Implication { .. } | analyze::ConstraintInfo::Iff { .. }
        | analyze::ConstraintInfo::Prohibition { .. }
    )) || disj.iter().any(|(dsig, _)| dsig == &s.name);
    if has_validation {
        writeln!(out).unwrap();
        writeln!(out, "    public List<string> Validate()").unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        var errors = new List<string>();").unwrap();
        for c in &sig_constraints {
            match c {
                analyze::ConstraintInfo::NoSelfRef { field_name, .. } => {
                    let fname = capitalize(field_name);
                    writeln!(out, "        if (ReferenceEquals({fname}, this))").unwrap();
                    writeln!(out, "            errors.Add(\"{fname} must not reference self\");").unwrap();
                }
                analyze::ConstraintInfo::Acyclic { field_name, .. } => {
                    let fname = capitalize(field_name);
                    writeln!(out, "        {{").unwrap();
                    writeln!(out, "            var seen = new HashSet<object>(ReferenceEqualityComparer.Instance);").unwrap();
                    writeln!(out, "            var cur = this;").unwrap();
                    writeln!(out, "            while (cur != null)").unwrap();
                    writeln!(out, "            {{").unwrap();
                    writeln!(out, "                if (!seen.Add(cur))").unwrap();
                    writeln!(out, "                {{").unwrap();
                    writeln!(out, "                    errors.Add(\"{fname} must not form a cycle\");").unwrap();
                    writeln!(out, "                    break;").unwrap();
                    writeln!(out, "                }}").unwrap();
                    writeln!(out, "                cur = cur.{fname};").unwrap();
                    writeln!(out, "            }}").unwrap();
                    writeln!(out, "        }}").unwrap();
                }
                analyze::ConstraintInfo::FieldOrdering { left_field, op, right_field, .. } => {
                    let lf = capitalize(left_field);
                    let rf = capitalize(right_field);
                    let (cs_op, negated_op) = match op {
                        CompareOp::Lt => ("<", ">="),
                        CompareOp::Gt => (">", "<="),
                        CompareOp::Lte => ("<=", ">"),
                        CompareOp::Gte => (">=", "<"),
                        _ => continue,
                    };
                    writeln!(out, "        if ({lf} {negated_op} {rf})").unwrap();
                    writeln!(out, "            errors.Add(\"{lf} must be {cs_op} {rf}\");").unwrap();
                }
                analyze::ConstraintInfo::Implication { condition, consequent, .. } => {
                    let cond = translate_validator_expr_cs(condition, &s.name);
                    let cons = translate_validator_expr_cs(consequent, &s.name);
                    let desc = format!("{} implies {}", analyze::describe_expr(condition), analyze::describe_expr(consequent));
                    writeln!(out, "        if ({cond} && !({cons}))").unwrap();
                    writeln!(out, "            errors.Add(\"{}\");", desc.replace('"', "\\\"")).unwrap();
                }
                analyze::ConstraintInfo::Iff { left, right, .. } => {
                    let l = translate_validator_expr_cs(left, &s.name);
                    let r = translate_validator_expr_cs(right, &s.name);
                    let desc = format!("{} iff {}", analyze::describe_expr(left), analyze::describe_expr(right));
                    writeln!(out, "        if (({l}) != ({r}))").unwrap();
                    writeln!(out, "            errors.Add(\"{}\");", desc.replace('"', "\\\"")).unwrap();
                }
                analyze::ConstraintInfo::Prohibition { condition, .. } => {
                    let cond = translate_validator_expr_cs(condition, &s.name);
                    let desc = analyze::describe_expr(condition);
                    writeln!(out, "        if ({cond})").unwrap();
                    writeln!(out, "            errors.Add(\"prohibited: {}\");", desc.replace('"', "\\\"")).unwrap();
                }
                analyze::ConstraintInfo::Disjoint { left, right, .. } => {
                    let left_field = capitalize(left.rsplit('.').next().unwrap_or(left));
                    let right_field = capitalize(right.rsplit('.').next().unwrap_or(right));
                    writeln!(out, "        if ({left_field}.Any(e => {right_field}.Contains(e)))").unwrap();
                    writeln!(out, "            errors.Add(\"{left_field} and {right_field} must not overlap (disjoint constraint)\");").unwrap();
                }
                analyze::ConstraintInfo::Exhaustive { categories, .. } => {
                    let cats = categories.join(", ");
                    let checks: Vec<String> = categories.iter().map(|cat| {
                        let parts: Vec<&str> = cat.split('.').collect();
                        if parts.len() == 2 {
                            format!("{}.{}.Contains(this)", parts[0], capitalize(parts[1]))
                        } else {
                            format!("{cat}.Contains(this)")
                        }
                    }).collect();
                    let condition = checks.join(" || ");
                    writeln!(out, "        if (!({condition}))").unwrap();
                    writeln!(out, "            errors.Add(\"must belong to one of [{cats}] (exhaustive constraint)\");").unwrap();
                }
                _ => {}
            }
        }
        // Disj uniqueness checks for seq fields
        for (dsig, dfield) in &disj {
            if dsig == &s.name {
                if let Some(f) = s.fields.iter().find(|f| f.name == *dfield) {
                    if f.mult == Multiplicity::Seq {
                        let fname = capitalize(dfield);
                        writeln!(out, "        if ({fname}.Distinct().Count() != {fname}.Count)").unwrap();
                        writeln!(out, "            errors.Add(\"{fname} must not contain duplicates (disj constraint)\");").unwrap();
                    }
                }
            }
        }
        writeln!(out, "        return errors;").unwrap();
        writeln!(out, "    }}").unwrap();
    }

    writeln!(out, "}}").unwrap();
}

fn generate_enum(out: &mut String, s: &StructureNode, ctx: &CsContext) {
    let variants = ctx.children.get(&s.name);
    let parent_fields = &s.fields;

    let all_unit = parent_fields.is_empty() && variants.map_or(true, |vs| {
        vs.iter().all(|v| ctx.struct_map.get(v).map_or(true, |st| st.fields.is_empty()))
    });

    if all_unit {
        writeln!(out, "public enum {}", s.name).unwrap();
        writeln!(out, "{{").unwrap();
        if let Some(variants) = variants {
            for v in variants {
                writeln!(out, "    {},", v).unwrap();
            }
        }
        writeln!(out, "}}").unwrap();
    } else {
        writeln!(out, "public abstract class {}", s.name).unwrap();
        writeln!(out, "{{").unwrap();
        for f in parent_fields {
            let type_str = mult_to_cs_type(&f.target, &f.mult);
            writeln!(out, "    public {} {} {{ get; set; }}", type_str, capitalize(&f.name)).unwrap();
        }
        writeln!(out, "}}").unwrap();
        if let Some(variants) = variants {
            for v in variants {
                let child = ctx.struct_map.get(v.as_str());
                let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
                writeln!(out).unwrap();
                writeln!(out, "public class {} : {}", v, s.name).unwrap();
                writeln!(out, "{{").unwrap();
                for f in &child_fields {
                    if f.mult == Multiplicity::Seq {
                        writeln!(out, "    // @alloy: seq").unwrap();
                    }
                    let type_str = mult_to_cs_type(&f.target, &f.mult);
                    writeln!(out, "    public {} {} {{ get; set; }}", type_str, capitalize(&f.name)).unwrap();
                }
                writeln!(out, "}}").unwrap();
            }
        }
    }
}

// ── Operations.cs ────────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "using System;").unwrap();
    writeln!(out, "using System.Collections.Generic;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "public static class Operations").unwrap();
    writeln!(out, "{{").unwrap();

    for op in &ir.operations {
        if op.receiver_sig.is_some() {
            continue;
        }
        let params = op.params.iter()
            .map(|p| {
                let type_str = mult_to_cs_type(&p.type_name, &p.mult);
                format!("{} {}", type_str, to_camel_case(&p.name))
            })
            .collect::<Vec<_>>()
            .join(", ");

        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            writeln!(out, "    /// <summary>{} performs the operation.</summary>", capitalize(&op.name)).unwrap();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, "    /// <param>{tag}: {desc}</param>").unwrap();
            }
        }

        let return_type = match &op.return_type {
            Some(rt) => mult_to_cs_type(&rt.type_name, &rt.mult),
            None => "void".to_string(),
        };

        writeln!(out, "    public static {} {}({params})", return_type, capitalize(&op.name)).unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        throw new NotImplementedException(\"oxidtr: implement {}\");", op.name).unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── Fixtures.cs ──────────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &CsContext) -> String {
    let mut out = String::new();
    writeln!(out, "using System.Collections.Generic;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "public static class Fixtures").unwrap();
    writeln!(out, "{{").unwrap();

    let fixture_types = backend::collect_fixture_types(ir);

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum || s.fields.is_empty() { continue; }

        // Default factory
        writeln!(out, "    public static {} Default{}()", s.name, s.name).unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        return new {}", s.name).unwrap();
        writeln!(out, "        {{").unwrap();
        for f in &s.fields {
            let val = default_value_for(&f.target, &f.mult, &s.name, ir, &fixture_types, ctx);
            writeln!(out, "            {} = {},", capitalize(&f.name), val).unwrap();
        }
        writeln!(out, "        }};").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();

        // Boundary factory
        writeln!(out, "    public static {} Boundary{}()", s.name, s.name).unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        return new {}", s.name).unwrap();
        writeln!(out, "        {{").unwrap();
        for f in &s.fields {
            let val = boundary_value_for(&f.target, &f.mult);
            writeln!(out, "            {} = {},", capitalize(&f.name), val).unwrap();
        }
        writeln!(out, "        }};").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Enum defaults
    for s in &ir.structures {
        if !s.is_enum { continue; }
        let variants = ctx.children.get(&s.name);
        let all_unit = s.fields.is_empty() && variants.map_or(true, |vs| {
            vs.iter().all(|v| ctx.struct_map.get(v).map_or(true, |st| st.fields.is_empty()))
        });
        if all_unit {
            if let Some(vs) = variants {
                if let Some(first) = vs.first() {
                    writeln!(out, "    public static {} Default{}() => {}.{};",
                        s.name, s.name, s.name, first).unwrap();
                    writeln!(out).unwrap();
                }
            }
        }
    }

    // Anomaly fixtures
    let anomalies = analyze::detect_anomalies(ir);
    let mut anomaly_sigs_done: std::collections::HashSet<String> = std::collections::HashSet::new();
    for anomaly in &anomalies {
        if let analyze::AnomalyPattern::UnboundedCollection { sig_name, .. } = anomaly {
            if anomaly_sigs_done.contains(sig_name) { continue; }
            let s = match ir.structures.iter().find(|s| s.name == *sig_name) {
                Some(s) => s,
                None => continue,
            };
            if ctx.variant_names.contains(&s.name) || s.is_enum || s.fields.is_empty() { continue; }
            anomaly_sigs_done.insert(sig_name.clone());

            writeln!(out, "    /// <summary>Anomaly fixture: all collections empty</summary>").unwrap();
            writeln!(out, "    public static {sig_name} AnomalyEmpty{sig_name}() => new(").unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let upper = capitalize(&f.name);
                let val = match &f.mult {
                    Multiplicity::Set => format!("new HashSet<{}>(){}", f.target, comma),
                    Multiplicity::Seq => format!("new List<{}>(){}", f.target, comma),
                    _ => format!("{}{}", cs_default_value(&f.target, &f.mult), comma),
                };
                writeln!(out, "        {upper}: {val}").unwrap();
            }
            writeln!(out, "    );").unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

fn default_value_for(target: &str, mult: &Multiplicity, owner: &str, ir: &OxidtrIR, fixture_types: &HashSet<String>, ctx: &CsContext) -> String {
    match mult {
        Multiplicity::One => {
            if ctx.variant_names.contains(target) || ctx.struct_map.get(target).map_or(false, |s| s.is_enum) {
                format!("Default{}()", target)
            } else if fixture_types.contains(target) {
                format!("Default{}()", target)
            } else {
                format!("new {}()", target)
            }
        }
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set | Multiplicity::Seq => {
            if backend::is_safe_set_population(owner, target, ir, fixture_types) {
                format!("new List<{}>() {{ Default{}() }}", target, target)
            } else {
                format!("new List<{}>()", target)
            }
        }
    }
}

fn boundary_value_for(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => format!("new {}()", target),
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set | Multiplicity::Seq => format!("new List<{}>()", target),
    }
}

// ── Tests.cs ─────────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "using Xunit;").unwrap();
    writeln!(out, "using System.Collections.Generic;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "public class ModelsTest").unwrap();
    writeln!(out, "{{").unwrap();

    let sig_names: HashSet<String> = ir.structures.iter().map(|s| s.name.clone()).collect();
    let has_fixture: HashSet<String> = ir.structures.iter()
        .filter(|s| !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone())
        .collect();
    let all_constraints = analyze::analyze(ir);

    // --- Constraint tests (facts) ---
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };

        // Temporal facts with prime → transition test
        if analyze::expr_contains_prime(&constraint.expr) {
            let test_name = format!("Transition_{}", capitalize(&fact_name));
            let params = expr_translator::extract_params(&constraint.expr, &sig_names);
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "    /// @temporal Transition constraint: {fact_name}").unwrap();
            writeln!(out, "    /// Verifies: pre→post state relationship ({desc})").unwrap();
            writeln!(out, "    [Fact]").unwrap();
            writeln!(out, "    public void {test_name}()").unwrap();
            writeln!(out, "    {{").unwrap();
            for (pname, tname) in &params {
                if has_fixture.contains(tname) {
                    writeln!(out, "        var {pname} = new List<{tname}>{{ Fixtures.Default{tname}() }};").unwrap();
                } else {
                    writeln!(out, "        var {pname} = new List<{tname}>();").unwrap();
                }
                writeln!(out, "        var next{cap} = new List<{tname}>({pname});", cap = capitalize(pname)).unwrap();
            }
            if let Some((_kind, bindings, inner_body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                let rewritten_body = analyze::rewrite_prime_as_post_state(inner_body);
                let body_str = expr_translator::translate_with_ir(&rewritten_body, ir);
                let bind_vars: Vec<String> = bindings.iter()
                    .flat_map(|b| b.vars.clone())
                    .collect();
                if bind_vars.len() == 1 {
                    let v = &bind_vars[0];
                    let pname = &params[0].0;
                    let cap = capitalize(pname);
                    writeln!(out, "        foreach (var ({v}, next{ucv}) in {pname}.Zip(next{cap}))", ucv = capitalize(v)).unwrap();
                    writeln!(out, "        {{").unwrap();
                    writeln!(out, "            Assert.True({body_str});").unwrap();
                    writeln!(out, "        }}").unwrap();
                } else {
                    writeln!(out, "        Assert.True({body_str});").unwrap();
                }
            } else {
                let rewritten = analyze::rewrite_prime_as_post_state(&constraint.expr);
                let body = expr_translator::translate_with_ir(&rewritten, ir);
                writeln!(out, "        Assert.True({body});").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        let temporal_kind = analyze::expr_temporal_kind(&constraint.expr);
        let test_name = match temporal_kind {
            Some(analyze::TemporalKind::Liveness) => format!("Liveness_{}", capitalize(&fact_name)),
            Some(analyze::TemporalKind::PastInvariant) => format!("PastInvariant_{}", capitalize(&fact_name)),
            Some(analyze::TemporalKind::PastLiveness) => format!("PastLiveness_{}", capitalize(&fact_name)),
            Some(analyze::TemporalKind::Step) => format!("Step_{}", capitalize(&fact_name)),
            Some(analyze::TemporalKind::Binary) => format!("Temporal_{}", capitalize(&fact_name)),
            _ => format!("Invariant_{}", capitalize(&fact_name)),
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        // Check guarantee level — skip type-guaranteed constraints
        let sig_constraints: Vec<&analyze::ConstraintInfo> = params.iter()
            .flat_map(|(_, tname)| {
                all_constraints.iter().filter(move |c| match c {
                    analyze::ConstraintInfo::Presence { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::CardinalityBound { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::NoSelfRef { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::Acyclic { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::Membership { sig_name, .. } => sig_name == tname,
                    _ => false,
                })
            })
            .collect();

        use crate::analyze::guarantee::{can_guarantee_by_type, Guarantee, TargetLang};

        let all_fully = !sig_constraints.is_empty() && sig_constraints.iter().all(|c| {
            can_guarantee_by_type(c, TargetLang::CSharp) == Guarantee::FullyByType
        });

        if all_fully {
            writeln!(out, "    // Type-guaranteed: {} — no test needed", fact_name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        // Binary temporal / liveness: cannot assert with single snapshot
        if temporal_kind == Some(analyze::TemporalKind::Binary) || matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            writeln!(out, "    [Fact]").unwrap();
            writeln!(out, "    public void {test_name}()").unwrap();
            writeln!(out, "    {{").unwrap();
            writeln!(out, "        // temporal: requires trace-based verification").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        let any_partial = sig_constraints.iter().any(|c| {
            can_guarantee_by_type(c, TargetLang::CSharp) == Guarantee::PartiallyByType
        });

        writeln!(out, "    [Fact]").unwrap();
        if any_partial {
            writeln!(out, "    /// @regression Partially type-guaranteed — regression test only.").unwrap();
        }
        writeln!(out, "    public void {test_name}()").unwrap();
        writeln!(out, "    {{").unwrap();
        for (pname, tname) in &params {
            if has_fixture.contains(tname) {
                writeln!(out, "        var {pname} = new List<{tname}>{{ Fixtures.Default{tname}() }};").unwrap();
            } else {
                writeln!(out, "        var {pname} = new List<{tname}>();").unwrap();
            }
        }
        writeln!(out, "        Assert.True({body});").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // --- Property tests ---
    for prop in &ir.properties {
        let test_name = capitalize(&prop.name);
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&prop.expr, ir);

        writeln!(out, "    [Fact]").unwrap();
        writeln!(out, "    public void {test_name}()").unwrap();
        writeln!(out, "    {{").unwrap();
        for (pname, tname) in &params {
            if has_fixture.contains(tname) {
                writeln!(out, "        var {pname} = new List<{tname}>{{ Fixtures.Default{tname}() }};").unwrap();
            } else {
                writeln!(out, "        var {pname} = new List<{tname}>();").unwrap();
            }
        }
        writeln!(out, "        Assert.True({body});").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // --- Anomaly tests ---
    let anomalies = analyze::detect_anomalies(ir);
    if !anomalies.is_empty() {
        writeln!(out, "    // --- Anomaly tests: edge-case coverage ---").unwrap();
        writeln!(out).unwrap();

        let mut anomaly_sigs: std::collections::HashMap<String, Vec<&analyze::AnomalyPattern>> = std::collections::HashMap::new();
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
                        let upper = capitalize(field_name);
                        writeln!(out, "    [Fact]").unwrap();
                        writeln!(out, "    public void Anomaly_{sig_name}_{upper}_Unconstrained()").unwrap();
                        writeln!(out, "    {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.Default{sig_name}();").unwrap();
                        writeln!(out, "        Assert.NotNull(instance.{upper} as object);").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        let upper = capitalize(field_name);
                        writeln!(out, "    [Fact]").unwrap();
                        writeln!(out, "    public void Anomaly_{sig_name}_{upper}_Empty()").unwrap();
                        writeln!(out, "    {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.AnomalyEmpty{sig_name}();").unwrap();
                        writeln!(out, "        Assert.NotNull(instance.{upper});").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        let upper = capitalize(field_name);
                        writeln!(out, "    [Fact]").unwrap();
                        writeln!(out, "    public void Anomaly_{sig_name}_{upper}_SelfRef()").unwrap();
                        writeln!(out, "    {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.Default{sig_name}();").unwrap();
                        writeln!(out, "        // Self-referential without guard").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                }
            }
        }
    }

    // --- Coverage tests ---
    let coverage = analyze::fact_coverage(ir);
    if !coverage.pairwise.is_empty() {
        writeln!(out, "    // --- Coverage tests: fact × fact pairwise ---").unwrap();
        writeln!(out).unwrap();

        let sig_names: HashSet<String> = ir.structures.iter().map(|s| s.name.clone()).collect();
        let mut cover_names_seen: HashSet<String> = HashSet::new();
        for pair in &coverage.pairwise {
            if !has_fixture.contains(&pair.sig_name) { continue; }
            let snake_a = to_snake_case(&pair.fact_a);
            let snake_b = to_snake_case(&pair.fact_b);
            let test_name = format!("Cover_{snake_a}_x_{snake_b}");

            // Skip duplicate test names (same fact pair from different sig perspectives)
            if !cover_names_seen.insert(test_name.clone()) { continue; }

            // Find the constraint nodes for both facts
            let constraint_a = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_a));
            let constraint_b = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_b));

            let (Some(ca), Some(cb)) = (constraint_a, constraint_b) else { continue; };

            let body_a = expr_translator::translate_with_ir(&ca.expr, ir);
            let body_b = expr_translator::translate_with_ir(&cb.expr, ir);

            // Extract all params from both facts to declare all needed variables
            let params_a = expr_translator::extract_params(&ca.expr, &sig_names);
            let params_b = expr_translator::extract_params(&cb.expr, &sig_names);
            let mut all_params: Vec<(String, String)> = Vec::new();
            let mut param_names_seen: HashSet<String> = HashSet::new();
            for (pname, tname) in params_a.iter().chain(params_b.iter()) {
                if param_names_seen.insert(pname.clone()) {
                    all_params.push((pname.clone(), tname.clone()));
                }
            }

            writeln!(out, "    [Fact(Skip = \"pairwise coverage scaffold\")]").unwrap();
            writeln!(out, "    public void {test_name}()").unwrap();
            writeln!(out, "    {{").unwrap();
            for (pname, tname) in &all_params {
                if has_fixture.contains(tname) {
                    writeln!(out, "        var {pname} = new List<{tname}>{{ Fixtures.Default{tname}() }};").unwrap();
                } else {
                    writeln!(out, "        var {pname} = new List<{tname}>();").unwrap();
                }
            }
            writeln!(out, "        Assert.True({body_a});").unwrap();
            writeln!(out, "        Assert.True({body_b});").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn mult_to_cs_type(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => target.to_string(),
        Multiplicity::Lone => format!("{target}?"),
        Multiplicity::Set | Multiplicity::Seq => format!("List<{target}>"),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

fn to_camel_case(s: &str) -> String {
    let cap = capitalize(s);
    let mut chars = cap.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    }
}

fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

fn cs_default_value(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => format!("new {}()", target),
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set => format!("new HashSet<{}>()", target),
        Multiplicity::Seq => format!("new List<{}>()", target),
    }
}

/// Translate an Alloy expression to C# for single-instance validator context.
fn translate_validator_expr_cs(expr: &crate::parser::ast::Expr, sig_name: &str) -> String {
    use crate::parser::ast::{Expr, LogicOp, QuantKind};
    match expr {
        Expr::VarRef(name) => {
            if name == sig_name { "this".to_string() } else { name.clone() }
        }
        Expr::IntLiteral(n) => n.to_string(),
        Expr::FieldAccess { base, field } => {
            format!("{}.{}", translate_validator_expr_cs(base, sig_name), capitalize(field))
        }
        Expr::Comparison { op, left, right } => {
            let l = translate_validator_expr_cs(left, sig_name);
            let r = translate_validator_expr_cs(right, sig_name);
            let o = match op {
                CompareOp::Eq => "==",
                CompareOp::NotEq => "!=",
                CompareOp::In => return format!("{r}.Contains({l})"),
                CompareOp::Lt => "<",
                CompareOp::Gt => ">",
                CompareOp::Lte => "<=",
                CompareOp::Gte => ">=",
            };
            format!("{l} {o} {r}")
        }
        Expr::BinaryLogic { op, left, right } => {
            let l = translate_validator_expr_cs(left, sig_name);
            let r = translate_validator_expr_cs(right, sig_name);
            match op {
                LogicOp::And => format!("{l} && {r}"),
                LogicOp::Or => format!("{l} || {r}"),
                LogicOp::Implies => format!("!({l}) || {r}"),
                LogicOp::Iff => format!("({l}) == ({r})"),
            }
        }
        Expr::Not(inner) => format!("!({})", translate_validator_expr_cs(inner, sig_name)),
        Expr::MultFormula { kind, expr: inner } => {
            let e = translate_validator_expr_cs(inner, sig_name);
            match kind {
                QuantKind::Some => format!("{e} != null"),
                QuantKind::No => format!("{e} == null"),
                _ => e,
            }
        }
        Expr::Cardinality(inner) => {
            format!("{}.Count", translate_validator_expr_cs(inner, sig_name))
        }
        _ => analyze::describe_expr(expr), // fallback: human-readable
    }
}
