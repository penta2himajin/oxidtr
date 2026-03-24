pub mod expr_translator;

use crate::backend::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::{Multiplicity, SigMultiplicity};
use crate::analyze;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = GoContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "models.go".to_string(),
        content: generate_models(ir, &ctx),
    });

    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    if has_tc {
        files.push(GeneratedFile {
            path: "helpers.go".to_string(),
            content: generate_helpers(ir),
        });
    }

    if !ir.operations.is_empty() {
        files.push(GeneratedFile {
            path: "operations.go".to_string(),
            content: generate_operations(ir),
        });
    }

    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        files.push(GeneratedFile {
            path: "models_test.go".to_string(),
            content: generate_tests(ir),
        });
    }

    files.push(GeneratedFile {
        path: "fixtures.go".to_string(),
        content: generate_fixtures(ir, &ctx),
    });

    files
}

// ── Context ──────────────────────────────────────────────────────────────────

struct GoContext {
    children: HashMap<String, Vec<String>>,
    variant_names: HashSet<String>,
    struct_map: HashMap<String, StructureNode>,
    cyclic_fields: HashSet<(String, String)>,
}

impl GoContext {
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
        let cyclic_fields = find_cyclic_fields(ir);
        GoContext { children, variant_names, struct_map, cyclic_fields }
    }

    fn is_variant(&self, name: &str) -> bool {
        self.variant_names.contains(name)
    }
}

fn find_cyclic_fields(ir: &OxidtrIR) -> HashSet<(String, String)> {
    let mut result = HashSet::new();
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s)).collect();
    for s in &ir.structures {
        for f in &s.fields {
            if f.mult == Multiplicity::One && f.target == s.name {
                result.insert((s.name.clone(), f.name.clone()));
            }
        }
    }
    for s in &ir.structures {
        for f in &s.fields {
            if f.mult == Multiplicity::One && f.target != s.name {
                let mut visited = HashSet::new();
                let mut stack = vec![f.target.as_str()];
                while let Some(cur) = stack.pop() {
                    if cur == s.name {
                        result.insert((s.name.clone(), f.name.clone()));
                        break;
                    }
                    if !visited.insert(cur) { continue; }
                    if let Some(target_s) = struct_map.get(cur) {
                        for tf in &target_s.fields {
                            if tf.mult == Multiplicity::One {
                                stack.push(&tf.target);
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

// ── models.go ────────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR, ctx: &GoContext) -> String {
    let mut out = String::new();
    writeln!(out, "package models").unwrap();
    writeln!(out).unwrap();

    let disj_fields = analyze::disj_fields(ir);

    for s in &ir.structures {
        if ctx.is_variant(&s.name) { continue; }

        let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
        if !constraint_names.is_empty() {
            writeln!(out, "// Invariants:").unwrap();
            for cn in &constraint_names {
                writeln!(out, "// - {cn}").unwrap();
            }
        }

        if s.is_enum {
            generate_enum(&mut out, s, ctx);
        } else {
            generate_struct(&mut out, s, ir, ctx, &disj_fields);
        }
        writeln!(out).unwrap();
    }

    out
}

fn generate_struct(out: &mut String, s: &StructureNode, ir: &OxidtrIR, ctx: &GoContext, disj_fields: &[(String, String)]) {
    // Singleton: one sig → package-level var
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        writeln!(out, "type {} struct{{}}", s.name).unwrap();
        writeln!(out).unwrap();
        writeln!(out, "var {}Instance = {}{{}}",
            s.name, s.name).unwrap();
        return;
    }

    if s.fields.is_empty() {
        writeln!(out, "type {} struct{{}}", s.name).unwrap();
    } else {
        writeln!(out, "type {} struct {{", s.name).unwrap();
        for f in &s.fields {
            let type_str = if let Some(vt) = &f.value_type {
                format!("map[{}]{}", f.target, vt)
            } else {
                mult_to_go_type(&f.target, &f.mult, ctx.cyclic_fields.contains(&(s.name.clone(), f.name.clone())))
            };

            // Comments for special patterns
            let target_mult = analyze::sig_multiplicity_for(ir, &f.target);
            if target_mult == SigMultiplicity::Lone && f.mult == Multiplicity::One {
                writeln!(out, "\t// Note: lone sig target — may not exist").unwrap();
            }
            if disj_fields.iter().any(|(sig, field)| sig == &s.name && field == &f.name) {
                if f.mult == Multiplicity::Seq {
                    writeln!(out, "\t// Consider using a set for uniqueness (disj constraint)").unwrap();
                }
            }

            writeln!(out, "\t{} {type_str}", expr_translator::capitalize(&f.name)).unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
}

fn generate_enum(out: &mut String, s: &StructureNode, ctx: &GoContext) {
    let variants = ctx.children.get(&s.name);

    // Check if all variants are unit (no fields)
    let all_unit = variants.map_or(true, |vs| {
        vs.iter().all(|v| ctx.struct_map.get(v).map_or(true, |st| st.fields.is_empty()))
    });

    if all_unit {
        // Go: interface + iota constants
        writeln!(out, "type {} int", s.name).unwrap();
        writeln!(out).unwrap();
        writeln!(out, "const (").unwrap();
        if let Some(variants) = variants {
            for (i, v) in variants.iter().enumerate() {
                if i == 0 {
                    writeln!(out, "\t{} {} = iota", v, s.name).unwrap();
                } else {
                    writeln!(out, "\t{}", v).unwrap();
                }
            }
        }
        writeln!(out, ")").unwrap();
    } else {
        // Interface-based sum type
        writeln!(out, "type {} interface {{", s.name).unwrap();
        writeln!(out, "\tis{}()", s.name).unwrap();
        writeln!(out, "}}").unwrap();
        if let Some(variants) = variants {
            for v in variants {
                let child = ctx.struct_map.get(v.as_str());
                let fields = child.map(|c| &c.fields).filter(|f| !f.is_empty());
                writeln!(out).unwrap();
                if let Some(fields) = fields {
                    writeln!(out, "type {} struct {{", v).unwrap();
                    for f in fields {
                        let type_str = if let Some(vt) = &f.value_type {
                            format!("map[{}]{}", f.target, vt)
                        } else {
                            mult_to_go_type(&f.target, &f.mult, false)
                        };
                        writeln!(out, "\t{} {type_str}", expr_translator::capitalize(&f.name)).unwrap();
                    }
                    writeln!(out, "}}").unwrap();
                } else {
                    writeln!(out, "type {} struct{{}}", v).unwrap();
                }
                writeln!(out).unwrap();
                writeln!(out, "func ({}) is{}() {{}}", v, s.name).unwrap();
            }
        }
    }
}

fn mult_to_go_type(target: &str, mult: &Multiplicity, is_indirect: bool) -> String {
    match mult {
        Multiplicity::One => {
            if is_indirect {
                format!("*{target}")
            } else {
                target.to_string()
            }
        }
        Multiplicity::Lone => format!("*{target}"),
        Multiplicity::Set => format!("[]{target}"),
        Multiplicity::Seq => format!("[]{target}"),
    }
}

// ── helpers.go ───────────────────────────────────────────────────────────────

fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "package models").unwrap();
    writeln!(out).unwrap();

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
    let fn_name = format!("Tc{}", expr_translator::capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = expr_translator::capitalize(&tc.field_name);

    writeln!(out, "// {fn_name} computes the transitive closure for {sig}.{field}.").unwrap();
    match tc.mult {
        Multiplicity::Lone => {
            writeln!(out, "func {fn_name}(start {sig}) []{sig} {{").unwrap();
            writeln!(out, "\tvar result []{sig}").unwrap();
            writeln!(out, "\tcurrent := start.{field}").unwrap();
            writeln!(out, "\tfor current != nil {{").unwrap();
            writeln!(out, "\t\tresult = append(result, *current)").unwrap();
            writeln!(out, "\t\tcurrent = current.{field}").unwrap();
            writeln!(out, "\t}}").unwrap();
            writeln!(out, "\treturn result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::Set | Multiplicity::Seq => {
            writeln!(out, "func {fn_name}(start {sig}) []{sig} {{").unwrap();
            writeln!(out, "\tvar result []{sig}").unwrap();
            writeln!(out, "\tqueue := make([]{sig}, len(start.{field}))").unwrap();
            writeln!(out, "\tcopy(queue, start.{field})").unwrap();
            writeln!(out, "\tseen := make(map[int]bool)").unwrap();
            writeln!(out, "\tfor len(queue) > 0 {{").unwrap();
            writeln!(out, "\t\tnext := queue[0]").unwrap();
            writeln!(out, "\t\tqueue = queue[1:]").unwrap();
            writeln!(out, "\t\tidx := len(result)").unwrap();
            writeln!(out, "\t\tif !seen[idx] {{").unwrap();
            writeln!(out, "\t\t\tseen[idx] = true").unwrap();
            writeln!(out, "\t\t\tresult = append(result, next)").unwrap();
            writeln!(out, "\t\t\tqueue = append(queue, next.{field}...)").unwrap();
            writeln!(out, "\t\t}}").unwrap();
            writeln!(out, "\t}}").unwrap();
            writeln!(out, "\treturn result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::One => {
            writeln!(out, "func {fn_name}(start {sig}) []{sig} {{").unwrap();
            writeln!(out, "\tvar result []{sig}").unwrap();
            writeln!(out, "\tcurrent := start.{field}").unwrap();
            writeln!(out, "\tfor i := 0; i < 1000; i++ {{").unwrap();
            writeln!(out, "\t\tresult = append(result, current)").unwrap();
            writeln!(out, "\t\tcurrent = current.{field}").unwrap();
            writeln!(out, "\t}}").unwrap();
            writeln!(out, "\treturn result").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
    writeln!(out).unwrap();
}

// ── operations.go ────────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "package models").unwrap();
    writeln!(out).unwrap();

    for op in &ir.operations {
        let params = op.params.iter()
            .map(|p| {
                let type_str = match p.mult {
                    Multiplicity::One => p.type_name.clone(),
                    Multiplicity::Lone => format!("*{}", p.type_name),
                    Multiplicity::Set | Multiplicity::Seq => format!("[]{}", p.type_name),
                };
                format!("{} {type_str}", to_go_param_name(&p.name))
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Doc comments from body expressions
        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            writeln!(out, "// {} performs the operation.", expr_translator::capitalize(&op.name)).unwrap();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, "// - {tag}: {desc}").unwrap();
            }
        }

        let return_str = match &op.return_type {
            Some(rt) => {
                let rt_str = go_return_type(&rt.type_name, &rt.mult);
                format!(" {rt_str}")
            }
            None => String::new(),
        };

        writeln!(out, "func {}({params}){return_str} {{", expr_translator::capitalize(&op.name)).unwrap();
        writeln!(out, "\tpanic(\"oxidtr: implement {}\")", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── models_test.go ───────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = expr_translator::collect_sig_names(ir);

    writeln!(out, "package models").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "import \"testing\"").unwrap();
    writeln!(out).unwrap();

    for prop in &ir.properties {
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&prop.expr, ir);

        writeln!(out, "func Test_{}(t *testing.T) {{", to_snake_case(&prop.name)).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "\t{pname} := []{tname}{{}}").unwrap();
        }
        writeln!(out, "\tif !({body}) {{").unwrap();
        writeln!(out, "\t\tt.Error(\"property {} violated\")", prop.name).unwrap();
        writeln!(out, "\t}}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    // Go has partial null safety (*T) — skip tests for null-safety constraints
    // that are partially guaranteed by pointer types
    let all_constraints = analyze::analyze(ir);
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        // Check if all related constraints are type-guaranteed in Go
        use crate::analyze::guarantee::{can_guarantee_by_type, Guarantee, TargetLang};
        let sig_constraints: Vec<_> = params.iter()
            .flat_map(|(_, tname)| {
                all_constraints.iter().filter(move |c| match c {
                    analyze::ConstraintInfo::Presence { sig_name, .. } => sig_name == tname,
                    analyze::ConstraintInfo::CardinalityBound { sig_name, .. } => sig_name == tname,
                    _ => false,
                })
            })
            .collect();

        let all_fully = !sig_constraints.is_empty() && sig_constraints.iter().all(|c| {
            can_guarantee_by_type(c, TargetLang::Go) == Guarantee::FullyByType
        });

        if all_fully {
            writeln!(out, "// Type-guaranteed: {} — Go type system handles this", fact_name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        writeln!(out, "func Test_invariant_{}(t *testing.T) {{", fact_name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "\t{pname} := []{tname}{{}}").unwrap();
        }
        writeln!(out, "\tif !({body}) {{").unwrap();
        writeln!(out, "\t\tt.Error(\"invariant {} violated\")", fact_name).unwrap();
        writeln!(out, "\t}}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    // Boundary value tests
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
            writeln!(out, "func Test_boundary_{}(t *testing.T) {{", fact_name).unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "\t{pname} := []{tname}{{Boundary{tname}()}}").unwrap();
                } else {
                    writeln!(out, "\t{pname} := []{tname}{{}}").unwrap();
                }
            }
            writeln!(out, "\tif !({body}) {{").unwrap();
            writeln!(out, "\t\tt.Error(\"boundary {} violated\")", fact_name).unwrap();
            writeln!(out, "\t}}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();

            writeln!(out, "func Test_invalid_{}(t *testing.T) {{", fact_name).unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "\t{pname} := []{tname}{{Invalid{tname}()}}").unwrap();
                } else {
                    writeln!(out, "\t{pname} := []{tname}{{}}").unwrap();
                }
            }
            writeln!(out, "\tif ({body}) {{").unwrap();
            writeln!(out, "\t\tt.Error(\"invalid {} should fail\")", fact_name).unwrap();
            writeln!(out, "\t}}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Cross-tests
    if !ir.constraints.is_empty() && !ir.operations.is_empty() {
        writeln!(out, "// --- Cross-tests: fact x operation ---").unwrap();
        writeln!(out).unwrap();
        for constraint in &ir.constraints {
            let fact_name = match &constraint.name { Some(n) => n.clone(), None => continue };
            let body = expr_translator::translate_with_ir(&constraint.expr, ir);
            for op in &ir.operations {
                writeln!(out, "// oxidtr: implement cross-test").unwrap();
                writeln!(out, "func disabled_Test_{fact_name}_preserved_after_{}(t *testing.T) {{", op.name).unwrap();
                writeln!(out, "\t// pre: assert({body})").unwrap();
                writeln!(out, "\t// {}(...)", op.name).unwrap();
                writeln!(out, "\t// post: assert({body})").unwrap();
                writeln!(out, "\tt.Fatal(\"oxidtr: implement cross-test\")").unwrap();
                writeln!(out, "}}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    out
}

// ── fixtures.go ──────────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &GoContext) -> String {
    let mut out = String::new();
    writeln!(out, "package models").unwrap();
    writeln!(out).unwrap();

    let fixture_types = super::collect_fixture_types(ir);

    // Generate enum default fixtures
    {
        let children: HashMap<String, Vec<String>> = {
            let mut map: HashMap<String, Vec<String>> = HashMap::new();
            for s in &ir.structures {
                if let Some(parent) = &s.parent {
                    map.entry(parent.clone()).or_default().push(s.name.clone());
                }
            }
            map
        };
        for s in &ir.structures {
            if !s.is_enum { continue; }
            let variants = match children.get(&s.name) {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };
            let all_unit = variants.iter().all(|v| {
                ctx.struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
            });
            if all_unit {
                let first = &variants[0];
                writeln!(out, "// Default{} returns a default value for {}.", s.name, s.name).unwrap();
                writeln!(out, "func Default{}() {} {{ return {} }}", s.name, s.name, first).unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum { continue; }
        if s.fields.is_empty() { continue; }

        writeln!(out, "// Default{} creates a default valid {}.", s.name, s.name).unwrap();
        writeln!(out, "func Default{}() {} {{", s.name, s.name).unwrap();
        writeln!(out, "\treturn {} {{", s.name).unwrap();
        for f in &s.fields {
            let val = if f.value_type.is_some() {
                "nil".to_string()
            } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && super::is_safe_set_population(&s.name, &f.target, ir, &fixture_types) {
                let safe = HashSet::from([f.target.clone()]);
                go_default_value_inner(&f.target, &f.mult, &safe)
            } else {
                go_default_value(&f.target, &f.mult)
            };
            writeln!(out, "\t\t{}: {val},", expr_translator::capitalize(&f.name)).unwrap();
        }
        writeln!(out, "\t}}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Boundary value fixtures
        let has_bounds = s.fields.iter().any(|f| {
            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
        });
        if has_bounds {
            writeln!(out, "// Boundary{} creates {} at cardinality boundary.", s.name, s.name).unwrap();
            writeln!(out, "func Boundary{}() {} {{", s.name, s.name).unwrap();
            writeln!(out, "\treturn {} {{", s.name).unwrap();
            for f in &s.fields {
                let val = if f.value_type.is_some() {
                    "nil".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let count = match &bound {
                            analyze::BoundKind::Exact(n) => *n,
                            analyze::BoundKind::AtMost(n) => *n,
                            analyze::BoundKind::AtLeast(n) => *n,
                        };
                        go_boundary_value(&f.target, &f.mult, count)
                    } else {
                        go_default_value(&f.target, &f.mult)
                    }
                } else {
                    go_default_value(&f.target, &f.mult)
                };
                writeln!(out, "\t\t{}: {val},", expr_translator::capitalize(&f.name)).unwrap();
            }
            writeln!(out, "\t}}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();

            writeln!(out, "// Invalid{} creates {} that violates cardinality constraint.", s.name, s.name).unwrap();
            writeln!(out, "func Invalid{}() {} {{", s.name, s.name).unwrap();
            writeln!(out, "\treturn {} {{", s.name).unwrap();
            for f in &s.fields {
                let val = if f.value_type.is_some() {
                    "nil".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let violation = match &bound {
                            analyze::BoundKind::Exact(n) => n + 1,
                            analyze::BoundKind::AtMost(n) => n + 1,
                            analyze::BoundKind::AtLeast(n) => if *n > 0 { n - 1 } else { 0 },
                        };
                        go_boundary_value(&f.target, &f.mult, violation)
                    } else {
                        go_default_value(&f.target, &f.mult)
                    }
                } else {
                    go_default_value(&f.target, &f.mult)
                };
                writeln!(out, "\t\t{}: {val},", expr_translator::capitalize(&f.name)).unwrap();
            }
            writeln!(out, "\t}}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

fn go_boundary_value(target: &str, mult: &Multiplicity, count: usize) -> String {
    match mult {
        Multiplicity::Set | Multiplicity::Seq => {
            let items: Vec<String> = (0..count).map(|_| format!("Default{target}()")).collect();
            if items.is_empty() {
                format!("[]{target}{{}}")
            } else {
                format!("[]{target}{{{}}}", items.join(", "))
            }
        }
        _ => go_default_value(target, mult),
    }
}

fn go_return_type(type_name: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => type_name.to_string(),
        Multiplicity::Lone => format!("*{type_name}"),
        Multiplicity::Set | Multiplicity::Seq => format!("[]{type_name}"),
    }
}

fn go_default_value(target: &str, mult: &Multiplicity) -> String {
    go_default_value_inner(target, mult, &HashSet::new())
}

fn go_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    match mult {
        Multiplicity::Lone => "nil".to_string(),
        Multiplicity::Set | Multiplicity::Seq => {
            if safe_targets.contains(target) {
                format!("[]{target}{{Default{target}()}}")
            } else {
                format!("[]{target}{{}}")
            }
        }
        Multiplicity::One => format!("Default{target}()"),
    }
}

// ── Naming helpers ───────────────────────────────────────────────────────────

fn to_go_param_name(name: &str) -> &str {
    // Go uses camelCase for local variables — Alloy param names are already camelCase
    name
}

fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_lowercase().next().unwrap());
    }
    result
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
        Expr::MultFormula { expr: inner, .. } => expr_uses_tc(inner),
        Expr::Prime(inner) => expr_uses_tc(inner),
        Expr::TemporalUnary { expr: inner, .. } => expr_uses_tc(inner),
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
}
