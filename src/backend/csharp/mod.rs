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
