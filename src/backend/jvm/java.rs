use super::{JvmContext, expr_translator};
use super::expr_translator::JvmLang;
use crate::backend::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity};
use crate::analyze;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

struct JavaLang;

impl JvmLang for JavaLang {
    fn all_quantifier(&self, collection: &str, var: &str, body: &str) -> String {
        format!("{collection}.stream().allMatch({var} -> {body})")
    }
    fn some_quantifier(&self, collection: &str, var: &str, body: &str) -> String {
        format!("{collection}.stream().anyMatch({var} -> {body})")
    }
    fn no_quantifier(&self, collection: &str, var: &str, body: &str) -> String {
        format!("{collection}.stream().noneMatch({var} -> {body})")
    }
    fn contains(&self, collection: &str, element: &str) -> String {
        format!("{collection}.contains({element})")
    }
    fn cardinality(&self, expr: &str) -> String {
        format!("{expr}.size()")
    }
    fn lone_eq(&self, base: &str, field: &str, value: &str) -> String {
        format!("java.util.Objects.equals({base}.{field}(), {value})")
    }
    fn tc_call(&self, field: &str, base: &str) -> String {
        format!("Helpers.tc{}({base})", capitalize(field))
    }
    fn eq_op(&self) -> &str { "==" }
    fn neq_op(&self) -> &str { "!=" }
    fn field_access(&self, base: &str, field: &str) -> String {
        format!("{base}.{field}()")
    }
}

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = JvmContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "Models.java".to_string(),
        content: generate_models(ir, &ctx),
    });

    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    // Generate Helpers.java for TC functions (replaces Invariants.java)
    if has_tc {
        files.push(GeneratedFile {
            path: "Helpers.java".to_string(),
            content: generate_helpers(ir),
        });
    }

    if !ir.operations.is_empty() {
        files.push(GeneratedFile {
            path: "Operations.java".to_string(),
            content: generate_operations(ir),
        });
    }

    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        files.push(GeneratedFile {
            path: "Tests.java".to_string(),
            content: generate_tests(ir),
        });
    }

    files.push(GeneratedFile {
        path: "Fixtures.java".to_string(),
        content: generate_fixtures(ir, &ctx),
    });

    files
}

// ── Models.java ────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR, ctx: &JvmContext) -> String {
    let mut out = String::new();
    let disj_fields = analyze::disj_fields(ir);

    writeln!(out, "import java.util.List;").unwrap();
    let has_map = ir.structures.iter().any(|s| s.fields.iter().any(|f| f.value_type.is_some()));
    if has_map {
        writeln!(out, "import java.util.Map;").unwrap();
    }
    writeln!(out, "import java.util.Optional;").unwrap();
    writeln!(out, "import java.util.Set;").unwrap();
    writeln!(out).unwrap();

    for s in &ir.structures {
        if ctx.is_variant(&s.name) { continue; }

        let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
        if !constraint_names.is_empty() {
            writeln!(out, "/**").unwrap();
            for cn in &constraint_names {
                writeln!(out, " * @invariant {cn}").unwrap();
            }
            writeln!(out, " */").unwrap();
        }

        if s.is_enum {
            generate_sealed_interface(&mut out, s, ctx);
        } else {
            generate_record(&mut out, s, ir, &disj_fields);
        }
        writeln!(out).unwrap();
    }

    out
}

fn generate_record(out: &mut String, s: &StructureNode, ir: &OxidtrIR, disj_fields: &[(String, String)]) {
    // Singleton: one sig → Java enum with INSTANCE
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        writeln!(out, "enum {} {{", s.name).unwrap();
        writeln!(out, "    INSTANCE").unwrap();
        writeln!(out, "}}").unwrap();
        return;
    }

    if s.fields.is_empty() {
        writeln!(out, "record {}() {{}}", s.name).unwrap();
    } else {
        let params: Vec<String> = s.fields.iter()
            .map(|f| {
                let mut annotations = Vec::new();
                let target_mult = analyze::sig_multiplicity_for(ir, &f.target);
                match f.mult {
                    Multiplicity::One => {
                        // Gap 1: lone sig target → nullable even if field mult is One
                        if target_mult == SigMultiplicity::Lone {
                            annotations.push("/* @Nullable — lone sig may not exist */".to_string());
                        } else {
                            annotations.push("/* @NotNull */".to_string());
                        }
                    }
                    _ => {}
                };
                // Gap 1: some sig → @NotEmpty on collection fields
                if target_mult == SigMultiplicity::Some && matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    annotations.push("/* @NotEmpty */".to_string());
                }
                // Bean Validation: @Size for set/seq fields with cardinality constraints
                let validations = analyze::bean_validations_for_field(ir, &s.name, &f.name);
                for v in &validations {
                    match v {
                        analyze::BeanValidation::Size { min, max, fact_name } => {
                            if min.is_some() || max.is_some() {
                                let mut parts = Vec::new();
                                if let Some(n) = min { parts.push(format!("min = {n}")); }
                                if let Some(n) = max { parts.push(format!("max = {n}")); }
                                annotations.push(format!("/* @Size({}) */", parts.join(", ")));
                            } else {
                                annotations.push(format!("/* @Size see fact: {fact_name} */"));
                            }
                        }
                        analyze::BeanValidation::MinMax { fact_name } => {
                            annotations.push(format!("/* @Min/@Max see fact: {fact_name} */"));
                        }
                    }
                }
                // NoSelfRef: field must not reference self
                let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
                let no_self_ref = sig_constraints.iter().any(|c| {
                    matches!(c, analyze::ConstraintInfo::NoSelfRef { field_name: fname, .. } if fname == &f.name)
                });
                if no_self_ref {
                    annotations.push(format!("/* requires {} != this — no self-reference */", f.name));
                }
                // Acyclic: field chain must not form a cycle
                let acyclic = sig_constraints.iter().any(|c| {
                    matches!(c, analyze::ConstraintInfo::Acyclic { field_name: fname, .. } if fname == &f.name)
                });
                if acyclic {
                    annotations.push(format!("/* acyclic: {}.^{} must not contain this */", f.name, f.name));
                }
                // Gap 3: disj → suggest Set
                if disj_fields.iter().any(|(sig, field)| sig == &s.name && field == &f.name) {
                    if f.mult == Multiplicity::Seq {
                        annotations.push("/* Consider using Set<T> for uniqueness (disj constraint) */".to_string());
                    }
                }
                let annotation_str = if annotations.is_empty() {
                    String::new()
                } else {
                    format!("{} ", annotations.join(" "))
                };
                let java_type = if let Some(vt) = &f.value_type {
                    format!("Map<{}, {}>", f.target, vt)
                } else {
                    mult_to_java_type(&f.target, &f.mult)
                };
                format!("{annotation_str}{} {}", java_type, f.name)
            })
            .collect();

        // FieldOrdering → compact constructor with validation
        let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
        let field_orderings: Vec<_> = sig_constraints.iter().filter_map(|c| {
            if let analyze::ConstraintInfo::FieldOrdering { left_field, op, right_field, .. } = c {
                let op_str = match op {
                    CompareOp::Lt => "<",
                    CompareOp::Gt => ">",
                    CompareOp::Lte => "<=",
                    CompareOp::Gte => ">=",
                    _ => return None,
                };
                Some((left_field.clone(), op_str, right_field.clone()))
            } else {
                None
            }
        }).collect();
        if field_orderings.is_empty() {
            writeln!(out, "record {}({}) {{}}", s.name, params.join(", ")).unwrap();
        } else {
            writeln!(out, "record {}({}) {{", s.name, params.join(", ")).unwrap();
            writeln!(out, "    {} {{", s.name).unwrap();
            for (lf, op, rf) in &field_orderings {
                let negated = match *op {
                    "<" => ">=",
                    ">" => "<=",
                    "<=" => ">",
                    ">=" => "<",
                    _ => continue,
                };
                writeln!(out, "        if ({lf} {negated} {rf}) throw new IllegalArgumentException(\"{lf} must be {op} {rf}\");").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
}

fn generate_sealed_interface(out: &mut String, s: &StructureNode, ctx: &JvmContext) {
    let variants = ctx.children.get(&s.name);

    let all_unit = variants.map_or(true, |vs| {
        vs.iter().all(|v| ctx.struct_map.get(v).map_or(true, |st| st.fields.is_empty()))
    });

    if all_unit {
        // Java enum
        writeln!(out, "enum {} {{", s.name).unwrap();
        if let Some(variants) = variants {
            let entries: Vec<&str> = variants.iter().map(|v| v.as_str()).collect();
            writeln!(out, "    {}", entries.join(", ")).unwrap();
        }
        writeln!(out, "}}").unwrap();
    } else {
        // Sealed interface with record variants
        write!(out, "sealed interface {}",  s.name).unwrap();
        if let Some(variants) = variants {
            let permits: Vec<&str> = variants.iter().map(|v| v.as_str()).collect();
            write!(out, " permits {}", permits.join(", ")).unwrap();
        }
        writeln!(out, " {{}}").unwrap();
        writeln!(out).unwrap();

        if let Some(variants) = variants {
            for v in variants {
                let child = ctx.struct_map.get(v.as_str());
                let fields = child.map(|c| &c.fields).filter(|f| !f.is_empty());
                if let Some(fields) = fields {
                    let params: Vec<String> = fields.iter()
                        .map(|f| {
                            let t = if let Some(vt) = &f.value_type {
                                format!("Map<{}, {}>", f.target, vt)
                            } else {
                                mult_to_java_type(&f.target, &f.mult)
                            };
                            format!("{} {}", t, f.name)
                        })
                        .collect();
                    writeln!(out, "record {}({}) implements {} {{}}", v, params.join(", "), s.name).unwrap();
                } else {
                    writeln!(out, "record {}() implements {} {{}}", v, s.name).unwrap();
                }
                writeln!(out).unwrap();
            }
        }
    }
}

fn mult_to_java_type(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => target.to_string(),
        Multiplicity::Lone => format!("{target} /* @Nullable */"),
        Multiplicity::Set => format!("Set<{target}>"),
        Multiplicity::Seq => format!("List<{target}>"),
    }
}

// ── Helpers.java ───────────────────────────────────────────────────────────

/// Generate Helpers.java containing TC (transitive closure) functions.
fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "import java.util.List;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "class Helpers {{").unwrap();

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

    writeln!(out, "}}").unwrap();
    out
}

fn generate_tc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("tc{}", capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "    /** Transitive closure traversal for {sig}.{field}. */").unwrap();
    match tc.mult {
        Multiplicity::Lone => {
            writeln!(out, "    static List<{sig}> {fn_name}({sig} start) {{").unwrap();
            writeln!(out, "        var result = new java.util.ArrayList<{sig}>();").unwrap();
            writeln!(out, "        var current = start.{field}();").unwrap();
            writeln!(out, "        while (current != null) {{").unwrap();
            writeln!(out, "            result.add(current);").unwrap();
            writeln!(out, "            current = current.{field}();").unwrap();
            writeln!(out, "        }}").unwrap();
            writeln!(out, "        return result;").unwrap();
            writeln!(out, "    }}").unwrap();
        }
        Multiplicity::Set | Multiplicity::Seq => {
            writeln!(out, "    static List<{sig}> {fn_name}({sig} start) {{").unwrap();
            writeln!(out, "        var result = new java.util.ArrayList<{sig}>();").unwrap();
            writeln!(out, "        var queue = new java.util.ArrayDeque<>(start.{field}());").unwrap();
            writeln!(out, "        while (!queue.isEmpty()) {{").unwrap();
            writeln!(out, "            var next = queue.poll();").unwrap();
            writeln!(out, "            if (!result.contains(next)) {{").unwrap();
            writeln!(out, "                result.add(next);").unwrap();
            writeln!(out, "                queue.addAll(next.{field}());").unwrap();
            writeln!(out, "            }}").unwrap();
            writeln!(out, "        }}").unwrap();
            writeln!(out, "        return result;").unwrap();
            writeln!(out, "    }}").unwrap();
        }
        Multiplicity::One => {
            writeln!(out, "    static List<{sig}> {fn_name}({sig} start) {{").unwrap();
            writeln!(out, "        var result = new java.util.ArrayList<{sig}>();").unwrap();
            writeln!(out, "        var current = start.{field}();").unwrap();
            writeln!(out, "        for (int i = 0; i < 1000; i++) {{").unwrap();
            writeln!(out, "            if (result.contains(current)) break;").unwrap();
            writeln!(out, "            result.add(current);").unwrap();
            writeln!(out, "            current = current.{field}();").unwrap();
            writeln!(out, "        }}").unwrap();
            writeln!(out, "        return result;").unwrap();
            writeln!(out, "    }}").unwrap();
        }
    }
    writeln!(out).unwrap();
}

// ── Operations.java ────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "class Operations {{").unwrap();

    for op in &ir.operations {
        let params = op.params.iter()
            .map(|p| {
                let type_str = match p.mult {
                    Multiplicity::One => p.type_name.clone(),
                    Multiplicity::Lone => format!("{} /* @Nullable */", p.type_name),
                    Multiplicity::Set => format!("Set<{}>", p.type_name),
                    Multiplicity::Seq => format!("List<{}>", p.type_name),
                };
                format!("{type_str} {}", p.name)
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Javadoc from body expressions with pre/post separation (Feature 7)
        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            writeln!(out, "    /**").unwrap();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, "     * @{tag} {desc}").unwrap();
            }
            writeln!(out, "     */").unwrap();
        }

        let return_type = match &op.return_type {
            Some(rt) => java_return_type(&rt.type_name, &rt.mult),
            None => "void".to_string(),
        };

        writeln!(out, "    static {} {}({params}) {{", return_type, op.name).unwrap();
        writeln!(out, "        throw new UnsupportedOperationException(\"oxidtr: implement {}\");", op.name).unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── Tests.java ─────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = expr_translator::collect_sig_names(ir);
    let lang = JavaLang;

    writeln!(out, "import org.junit.jupiter.api.Test;").unwrap();
    writeln!(out, "import org.junit.jupiter.api.Disabled;").unwrap();
    writeln!(out, "import static org.junit.jupiter.api.Assertions.*;").unwrap();
    writeln!(out, "import java.util.List;").unwrap();

    writeln!(out).unwrap();
    writeln!(out, "class PropertyTests {{").unwrap();

    for prop in &ir.properties {
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&prop.expr, ir, &lang);

        writeln!(out, "    @Test").unwrap();
        writeln!(out, "    void {}() {{", prop.name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        List<{tname}> {pname} = List.of();").unwrap();
        }
        writeln!(out, "        assertTrue({body});").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Invariant tests — inline constraint expressions
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name { Some(n) => n.clone(), None => continue };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir, &lang);

        writeln!(out, "    @Test").unwrap();
        writeln!(out, "    void invariant_{}() {{", fact_name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        List<{tname}> {pname} = List.of();").unwrap();
        }
        writeln!(out, "        assertTrue({body});").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Boundary value tests (Feature 5) — inline expressions
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir, &lang);

        let has_boundary = params.iter().any(|(_, tname)| {
            ir.structures.iter().any(|s| {
                s.name == *tname && !s.is_enum && s.fields.iter().any(|f| {
                    matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                        && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                })
            })
        });

        if has_boundary {
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    void boundary_{}() {{", fact_name).unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "        List<{tname}> {pname} = List.of(Fixtures.boundary{tname}());").unwrap();
                } else {
                    writeln!(out, "        List<{tname}> {pname} = List.of();").unwrap();
                }
            }
            writeln!(out, "        assertTrue({body});").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();

            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    void invalid_{}() {{", fact_name).unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "        List<{tname}> {pname} = List.of(Fixtures.invalid{tname}());").unwrap();
                } else {
                    writeln!(out, "        List<{tname}> {pname} = List.of();").unwrap();
                }
            }
            writeln!(out, "        assertFalse(!({body}));").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Cross-tests — inline expressions
    if !ir.constraints.is_empty() && !ir.operations.is_empty() {
        writeln!(out, "    // --- Cross-tests: fact x operation ---").unwrap();
        writeln!(out).unwrap();
        for constraint in &ir.constraints {
            let fact_name = match &constraint.name { Some(n) => n.clone(), None => continue };
            let body = expr_translator::translate_with_ir(&constraint.expr, ir, &lang);
            for op in &ir.operations {
                writeln!(out, "    @Disabled(\"oxidtr: implement cross-test\")").unwrap();
                writeln!(out, "    @Test").unwrap();
                writeln!(out, "    void {fact_name}_preserved_after_{}() {{", op.name).unwrap();
                writeln!(out, "        // pre: assertTrue({body});").unwrap();
                writeln!(out, "        // {}(...);", op.name).unwrap();
                writeln!(out, "        // post: assertTrue({body});").unwrap();
                writeln!(out, "        throw new UnsupportedOperationException(\"oxidtr: implement cross-test\");").unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── helpers ────────────────────────────────────────────────────────────────

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

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

// ── Fixtures.java ──────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &JvmContext) -> String {
    let mut out = String::new();

    writeln!(out, "import java.util.List;").unwrap();
    writeln!(out, "import java.util.Set;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "class Fixtures {{").unwrap();

    let fixture_types = super::super::collect_fixture_types(ir);

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
        let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
            .map(|s| (s.name.as_str(), s))
            .collect();
        for s in &ir.structures {
            if !s.is_enum { continue; }
            let variants = match children.get(&s.name) {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };
            let all_unit = variants.iter().all(|v| {
                struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
            });
            let first_unit = variants.iter().find(|v| {
                struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
            });
            if let Some(variant) = first_unit {
                if all_unit {
                    // Java enum → qualified access: EnumName.Variant
                    writeln!(out, "    /** Factory: default value for {} */", s.name).unwrap();
                    writeln!(out, "    static {} default{}() {{", s.name, s.name).unwrap();
                    writeln!(out, "        return {}.{};", s.name, variant).unwrap();
                    writeln!(out, "    }}").unwrap();
                    writeln!(out).unwrap();
                } else {
                    // Sealed interface → first unit variant as record instance
                    let has_fields = struct_map.get(variant.as_str())
                        .map_or(false, |st| !st.fields.is_empty());
                    if !has_fields {
                        writeln!(out, "    /** Factory: default value for {} */", s.name).unwrap();
                        writeln!(out, "    static {} default{}() {{", s.name, s.name).unwrap();
                        writeln!(out, "        return new {}();", variant).unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                }
            }
        }
    }

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum { continue; }
        if s.fields.is_empty() { continue; }

        let params: Vec<String> = s.fields.iter()
            .map(|f| {
                if f.value_type.is_some() {
                    "Map.of()".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                    && super::super::is_safe_set_population(&s.name, &f.target, ir, &fixture_types) {
                    let safe = HashSet::from([f.target.clone()]);
                    java_default_value_inner(&f.target, &f.mult, &safe)
                } else {
                    java_default_value(&f.target, &f.mult)
                }
            })
            .collect();

        writeln!(out, "    /** Factory: create a default valid {} */", s.name).unwrap();
        writeln!(out, "    static {} default{}() {{", s.name, s.name).unwrap();
        writeln!(out, "        return new {}({});", s.name, params.join(", ")).unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();

        // Boundary value fixtures (Feature 5)
        let has_bounds = s.fields.iter().any(|f| {
            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
        });
        if has_bounds {
            let boundary_params: Vec<String> = s.fields.iter()
                .map(|f| {
                    if f.value_type.is_some() {
                        return "Map.of()".to_string();
                    }
                    if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                        if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                            let count = match &bound {
                                analyze::BoundKind::Exact(n) => *n,
                                analyze::BoundKind::AtMost(n) => *n,
                                analyze::BoundKind::AtLeast(n) => *n,
                            };
                            return java_boundary_value(&f.target, &f.mult, count);
                        }
                    }
                    java_default_value(&f.target, &f.mult)
                })
                .collect();
            writeln!(out, "    /** Factory: create {} at cardinality boundary */", s.name).unwrap();
            writeln!(out, "    static {} boundary{}() {{", s.name, s.name).unwrap();
            writeln!(out, "        return new {}({});", s.name, boundary_params.join(", ")).unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();

            let invalid_params: Vec<String> = s.fields.iter()
                .map(|f| {
                    if f.value_type.is_some() {
                        return "Map.of()".to_string();
                    }
                    if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                        if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                            let violation = match &bound {
                                analyze::BoundKind::Exact(n) => n + 1,
                                analyze::BoundKind::AtMost(n) => n + 1,
                                analyze::BoundKind::AtLeast(n) => if *n > 0 { n - 1 } else { 0 },
                            };
                            return java_boundary_value(&f.target, &f.mult, violation);
                        }
                    }
                    java_default_value(&f.target, &f.mult)
                })
                .collect();
            writeln!(out, "    /** Factory: create {} that violates cardinality constraint */", s.name).unwrap();
            writeln!(out, "    static {} invalid{}() {{", s.name, s.name).unwrap();
            writeln!(out, "        return new {}({});", s.name, invalid_params.join(", ")).unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

fn java_boundary_value(target: &str, mult: &Multiplicity, count: usize) -> String {
    match mult {
        Multiplicity::Set => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            if items.is_empty() {
                "Set.of()".to_string()
            } else {
                format!("Set.of({})", items.join(", "))
            }
        }
        Multiplicity::Seq => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            if items.is_empty() {
                "List.of()".to_string()
            } else {
                format!("List.of({})", items.join(", "))
            }
        }
        _ => java_default_value(target, mult),
    }
}

fn java_return_type(type_name: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => type_name.to_string(),
        Multiplicity::Lone => format!("{type_name} /* @Nullable */"),
        Multiplicity::Set => format!("Set<{type_name}>"),
        Multiplicity::Seq => format!("List<{type_name}>"),
    }
}

fn java_default_value(target: &str, mult: &Multiplicity) -> String {
    java_default_value_inner(target, mult, &HashSet::new())
}

fn java_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    match mult {
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set => {
            if safe_targets.contains(target) {
                format!("new java.util.HashSet<>(Set.of(default{target}()))")
            } else {
                "Set.of()".to_string()
            }
        }
        Multiplicity::Seq => {
            if safe_targets.contains(target) {
                format!("new java.util.ArrayList<>(List.of(default{target}()))")
            } else {
                "List.of()".to_string()
            }
        }
        Multiplicity::One => format!("default{target}()"),
    }
}
