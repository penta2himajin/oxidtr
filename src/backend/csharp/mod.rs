pub mod expr_translator;

use crate::backend::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::{Multiplicity, SigMultiplicity};
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

    out
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

    // Generate Validate() method for Disjoint and Exhaustive constraints
    let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
    let has_validation = sig_constraints.iter().any(|c| matches!(c,
        analyze::ConstraintInfo::Disjoint { .. } | analyze::ConstraintInfo::Exhaustive { .. }
    ));
    if has_validation {
        writeln!(out).unwrap();
        writeln!(out, "    public List<string> Validate()").unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        var errors = new List<string>();").unwrap();
        for c in &sig_constraints {
            match c {
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

    for c in &ir.constraints {
        let name = c.name.as_deref().unwrap_or("Unnamed");
        writeln!(out, "    [Fact]").unwrap();
        writeln!(out, "    public void {}()", name).unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        // constraint: {}", analyze::describe_expr(&c.expr)).unwrap();
        writeln!(out, "        Assert.True(true); // TODO: implement").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    for p in &ir.properties {
        writeln!(out, "    [Fact]").unwrap();
        writeln!(out, "    public void {}()", p.name).unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        // property: {}", analyze::describe_expr(&p.expr)).unwrap();
        writeln!(out, "        Assert.True(true); // TODO: implement").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    let has_fixture: HashSet<String> = ir.structures.iter()
        .filter(|s| !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone())
        .collect();

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

        for pair in &coverage.pairwise {
            if !has_fixture.contains(&pair.sig_name) { continue; }
            let snake_a = to_snake_case(&pair.fact_a);
            let snake_b = to_snake_case(&pair.fact_b);

            let body_a = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_a))
                .map(|c| expr_translator::translate_with_ir(&c.expr, ir));
            let body_b = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_b))
                .map(|c| expr_translator::translate_with_ir(&c.expr, ir));

            writeln!(out, "    [Fact]").unwrap();
            writeln!(out, "    public void Cover_{snake_a}_x_{snake_b}()").unwrap();
            writeln!(out, "    {{").unwrap();
            writeln!(out, "        var {}s = new List<{}>{{ Fixtures.Default{}() }};", to_camel_case(&pair.sig_name), pair.sig_name, pair.sig_name).unwrap();
            if let (Some(a), Some(b)) = (&body_a, &body_b) {
                writeln!(out, "        Assert.True({a});").unwrap();
                writeln!(out, "        Assert.True({b});").unwrap();
            }
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
