use super::{JvmContext, expr_translator};
use super::expr_translator::JvmLang;
use crate::backend::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::{Multiplicity, SigMultiplicity};
use crate::analyze;
use std::fmt::Write;

struct KotlinLang;

impl JvmLang for KotlinLang {
    fn all_quantifier(&self, collection: &str, var: &str, body: &str) -> String {
        format!("{collection}.all {{ {var} -> {body} }}")
    }
    fn some_quantifier(&self, collection: &str, var: &str, body: &str) -> String {
        format!("{collection}.any {{ {var} -> {body} }}")
    }
    fn no_quantifier(&self, collection: &str, var: &str, body: &str) -> String {
        format!("!{collection}.any {{ {var} -> {body} }}")
    }
    fn contains(&self, collection: &str, element: &str) -> String {
        format!("{collection}.contains({element})")
    }
    fn cardinality(&self, expr: &str) -> String {
        format!("{expr}.size")
    }
    fn lone_eq(&self, base: &str, field: &str, value: &str) -> String {
        format!("{base}.{field} == {value}")
    }
    fn tc_call(&self, field: &str, base: &str) -> String {
        format!("tc{}({base})", capitalize(field))
    }
    fn eq_op(&self) -> &str { "==" }
    fn neq_op(&self) -> &str { "!=" }
}

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = JvmContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "Models.kt".to_string(),
        content: generate_models(ir, &ctx),
    });

    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

    if !ir.constraints.is_empty() || has_tc {
        files.push(GeneratedFile {
            path: "Invariants.kt".to_string(),
            content: generate_invariants(ir),
        });
    }

    if !ir.operations.is_empty() {
        files.push(GeneratedFile {
            path: "Operations.kt".to_string(),
            content: generate_operations(ir),
        });
    }

    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        files.push(GeneratedFile {
            path: "Tests.kt".to_string(),
            content: generate_tests(ir),
        });
    }

    files.push(GeneratedFile {
        path: "Fixtures.kt".to_string(),
        content: generate_fixtures(ir, &ctx),
    });

    files
}

// ── Models.kt ──────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR, ctx: &JvmContext) -> String {
    let mut out = String::new();
    let disj_fields = analyze::disj_fields(ir);

    for s in &ir.structures {
        if ctx.is_variant(&s.name) { continue; }

        let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
        if !constraint_names.is_empty() {
            writeln!(out, "/**").unwrap();
            for cn in &constraint_names {
                writeln!(out, " * @property Invariant: {cn}").unwrap();
            }
            writeln!(out, " */").unwrap();
        }

        if s.is_enum {
            generate_sealed_class(&mut out, s, ctx);
        } else {
            generate_data_class(&mut out, s, ir, &disj_fields);
        }
        writeln!(out).unwrap();
    }

    out
}

fn generate_data_class(out: &mut String, s: &StructureNode, ir: &OxidtrIR, disj_fields: &[(String, String)]) {
    // Singleton: one sig → Kotlin object
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        writeln!(out, "object {}", s.name).unwrap();
        return;
    }

    if s.fields.is_empty() {
        writeln!(out, "data class {}(val placeholder: Unit = Unit)", s.name).unwrap();
    } else {
        writeln!(out, "data class {}(", s.name).unwrap();
        for (i, f) in s.fields.iter().enumerate() {
            let type_str = if let Some(vt) = &f.value_type {
                format!("Map<{}, {}>", f.target, vt)
            } else {
                mult_to_kt_type(&f.target, &f.mult)
            };
            let comma = if i < s.fields.len() - 1 { "," } else { "" };
            // Bean Validation annotations
            let validations = analyze::bean_validations_for_field(ir, &s.name, &f.name);
            let mut annotations = Vec::new();
            // Gap 1: some sig → @NotEmpty on collection fields
            let target_mult = analyze::sig_multiplicity_for(ir, &f.target);
            if target_mult == SigMultiplicity::Some && matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                annotations.push("/* @NotEmpty */".to_string());
            }
            for v in &validations {
                match v {
                    analyze::BeanValidation::Size { min, max, fact_name } => {
                        if min.is_some() || max.is_some() {
                            let mut parts = Vec::new();
                            if let Some(n) = min { parts.push(format!("min = {n}")); }
                            if let Some(n) = max { parts.push(format!("max = {n}")); }
                            annotations.push(format!("@Size({})", parts.join(", ")));
                        } else {
                            annotations.push(format!("/* @Size see fact: {fact_name} */"));
                        }
                    }
                    analyze::BeanValidation::MinMax { fact_name } => {
                        annotations.push(format!("/* @Min/@Max see fact: {fact_name} */"));
                    }
                }
            }
            // Gap 1: lone sig target → make nullable even if field mult is One
            if target_mult == SigMultiplicity::Lone && f.mult == Multiplicity::One {
                annotations.push("/* @Nullable — lone sig may not exist */".to_string());
            }
            // Gap 3: disj → suggest Set
            if disj_fields.iter().any(|(sig, field)| sig == &s.name && field == &f.name) {
                if f.mult == Multiplicity::Seq {
                    annotations.push("/* Consider using Set<T> for uniqueness (disj constraint) */".to_string());
                }
            }
            for ann in &annotations {
                writeln!(out, "    {ann}").unwrap();
            }
            writeln!(out, "    val {}: {type_str}{comma}", f.name).unwrap();
        }
        writeln!(out, ")").unwrap();
    }
}

fn generate_sealed_class(out: &mut String, s: &StructureNode, ctx: &JvmContext) {
    let variants = ctx.children.get(&s.name);

    // Check if all variants are unit (no fields, singleton)
    let all_unit = variants.map_or(true, |vs| {
        vs.iter().all(|v| ctx.struct_map.get(v).map_or(true, |st| st.fields.is_empty()))
    });

    if all_unit {
        // Kotlin enum class
        writeln!(out, "enum class {} {{", s.name).unwrap();
        if let Some(variants) = variants {
            let entries: Vec<&str> = variants.iter().map(|v| v.as_str()).collect();
            writeln!(out, "    {}", entries.join(", ")).unwrap();
        }
        writeln!(out, "}}").unwrap();
    } else {
        // Sealed class with data class variants
        writeln!(out, "sealed class {}", s.name).unwrap();
        writeln!(out).unwrap();
        if let Some(variants) = variants {
            for v in variants {
                let child = ctx.struct_map.get(v.as_str());
                let fields = child.map(|c| &c.fields).filter(|f| !f.is_empty());
                if let Some(fields) = fields {
                    writeln!(out, "data class {}(", v).unwrap();
                    for (i, f) in fields.iter().enumerate() {
                        let type_str = if let Some(vt) = &f.value_type {
                            format!("Map<{}, {}>", f.target, vt)
                        } else {
                            mult_to_kt_type(&f.target, &f.mult)
                        };
                        let comma = if i < fields.len() - 1 { "," } else { "" };
                        writeln!(out, "    val {}: {type_str}{comma}", f.name).unwrap();
                    }
                    writeln!(out, ") : {}()", s.name).unwrap();
                } else {
                    writeln!(out, "data object {} : {}()", v, s.name).unwrap();
                }
                writeln!(out).unwrap();
            }
        }
    }
}

fn mult_to_kt_type(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => target.to_string(),
        Multiplicity::Lone => format!("{target}?"),
        Multiplicity::Set => format!("Set<{target}>"),
        Multiplicity::Seq => format!("List<{target}>"),
    }
}

// ── Invariants.kt ──────────────────────────────────────────────────────────

fn generate_invariants(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = expr_translator::collect_sig_names(ir);
    let lang = KotlinLang;

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
            Some(name) => format!("assert{name}"),
            None => continue,
        };

        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir, &lang);

        let param_str = params
            .iter()
            .map(|(pname, tname)| format!("{pname}: List<{tname}>"))
            .collect::<Vec<_>>()
            .join(", ");

        // Feature 6: add @unique annotation when disj is used
        if expr_uses_disj(&constraint.expr) {
            writeln!(out, "/**").unwrap();
            writeln!(out, " * Invariant derived from Alloy fact.").unwrap();
            writeln!(out, " * @unique Elements must be distinct (disj).").unwrap();
            writeln!(out, " */").unwrap();
        } else {
            writeln!(out, "/** Invariant derived from Alloy fact. */").unwrap();
        }
        writeln!(out, "// @alloy: {}", crate::analyze::alloy_repr(&constraint.expr)).unwrap();
        writeln!(out, "fun {fn_name}({param_str}): Boolean {{").unwrap();
        writeln!(out, "    return {body}").unwrap();
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
            writeln!(out, "fun {fn_name}(start: {sig}): List<{sig}> {{").unwrap();
            writeln!(out, "    val result = mutableListOf<{sig}>()").unwrap();
            writeln!(out, "    var current: {sig}? = start.{field}").unwrap();
            writeln!(out, "    while (current != null) {{").unwrap();
            writeln!(out, "        result.add(current)").unwrap();
            writeln!(out, "        current = current.{field}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    return result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::Set | Multiplicity::Seq => {
            writeln!(out, "fun {fn_name}(start: {sig}): List<{sig}> {{").unwrap();
            writeln!(out, "    val result = mutableListOf<{sig}>()").unwrap();
            writeln!(out, "    val queue = ArrayDeque(start.{field})").unwrap();
            writeln!(out, "    while (queue.isNotEmpty()) {{").unwrap();
            writeln!(out, "        val next = queue.removeFirst()").unwrap();
            writeln!(out, "        if (next !in result) {{").unwrap();
            writeln!(out, "            result.add(next)").unwrap();
            writeln!(out, "            queue.addAll(next.{field})").unwrap();
            writeln!(out, "        }}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    return result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::One => {
            writeln!(out, "fun {fn_name}(start: {sig}): List<{sig}> {{").unwrap();
            writeln!(out, "    val result = mutableListOf<{sig}>()").unwrap();
            writeln!(out, "    var current: {sig} = start.{field}").unwrap();
            writeln!(out, "    repeat(1000) {{").unwrap();
            writeln!(out, "        if (current in result) return result").unwrap();
            writeln!(out, "        result.add(current)").unwrap();
            writeln!(out, "        current = current.{field}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    return result").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
    writeln!(out).unwrap();
}

// ── Operations.kt ──────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    for op in &ir.operations {
        let params = op.params.iter()
            .map(|p| {
                let type_str = match p.mult {
                    Multiplicity::One => p.type_name.clone(),
                    Multiplicity::Lone => format!("{}?", p.type_name),
                    Multiplicity::Set => format!("Set<{}>", p.type_name),
                    Multiplicity::Seq => format!("List<{}>", p.type_name),
                };
                format!("{}: {type_str}", p.name)
            })
            .collect::<Vec<_>>()
            .join(", ");

        // KDoc from body expressions with pre/post separation (Feature 7)
        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            writeln!(out, "/**").unwrap();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, " * @{tag} {desc}").unwrap();
            }
            writeln!(out, " */").unwrap();
            for expr in &op.body {
                writeln!(out, "// @alloy: {}", crate::analyze::alloy_repr(expr)).unwrap();
            }
        }

        let return_str = match &op.return_type {
            Some(rt) => format!(": {}", kt_return_type(&rt.type_name, &rt.mult)),
            None => String::new(),
        };

        writeln!(out, "fun {}({params}){return_str} {{", op.name).unwrap();
        writeln!(out, "    TODO(\"oxidtr: implement {}\")", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── Tests.kt ───────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = expr_translator::collect_sig_names(ir);
    let lang = KotlinLang;

    writeln!(out, "import org.junit.jupiter.api.Test").unwrap();
    writeln!(out, "import org.junit.jupiter.api.Assertions.*").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "class PropertyTests {{").unwrap();

    for prop in &ir.properties {
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&prop.expr, ir, &lang);

        writeln!(out, "    // @alloy: {}", crate::analyze::alloy_repr(&prop.expr)).unwrap();
        writeln!(out, "    @Test").unwrap();
        writeln!(out, "    fun `{}`() {{", prop.name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
        }
        writeln!(out, "        assertTrue({body})").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let fn_name = format!("assert{fact_name}");
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);

        writeln!(out, "    // @alloy: {}", crate::analyze::alloy_repr(&constraint.expr)).unwrap();
        writeln!(out, "    @Test").unwrap();
        writeln!(out, "    fun `invariant {fact_name}`() {{").unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
        }
        let args = params.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", ");
        writeln!(out, "        assertTrue({fn_name}({args}))").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Boundary value tests (Feature 5)
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let fn_name = format!("assert{fact_name}");
        let params = expr_translator::extract_params(&constraint.expr, &sig_names);

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
            writeln!(out, "    fun `boundary {fact_name}`() {{").unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "        val {pname}: List<{tname}> = listOf(boundary{tname}())").unwrap();
                } else {
                    writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
                }
            }
            let args = params.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", ");
            writeln!(out, "        assertTrue({fn_name}({args}))").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();

            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    fun `invalid {fact_name}`() {{").unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "        val {pname}: List<{tname}> = listOf(invalid{tname}())").unwrap();
                } else {
                    writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
                }
            }
            let args = params.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", ");
            writeln!(out, "        assertFalse({fn_name}({args}))").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Cross-tests
    if !ir.constraints.is_empty() && !ir.operations.is_empty() {
        writeln!(out, "    // --- Cross-tests: fact x operation ---").unwrap();
        writeln!(out).unwrap();
        for constraint in &ir.constraints {
            let fact_name = match &constraint.name { Some(n) => n.clone(), None => continue };
            let fact_fn = format!("assert{fact_name}");
            for op in &ir.operations {
                writeln!(out, "    // @alloy: {}", crate::analyze::alloy_repr(&constraint.expr)).unwrap();
                writeln!(out, "    @Test").unwrap();
                writeln!(out, "    fun `{fact_name} preserved after {}`() {{", op.name).unwrap();
                writeln!(out, "        // pre: assertTrue({fact_fn}())").unwrap();
                writeln!(out, "        // {}(...)", op.name).unwrap();
                writeln!(out, "        // post: assertTrue({fact_fn}())").unwrap();
                writeln!(out, "        TODO(\"oxidtr: implement cross-test\")").unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── helpers ────────────────────────────────────────────────────────────────

fn expr_uses_disj(expr: &crate::parser::ast::Expr) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| b.disj) || expr_uses_disj(body)
        }
        Expr::BinaryLogic { left, right, .. } | Expr::Comparison { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            expr_uses_disj(left) || expr_uses_disj(right)
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            expr_uses_disj(inner)
        }
        Expr::FieldAccess { base, .. } => expr_uses_disj(base),
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
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

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

// ── Fixtures.kt ────────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &JvmContext) -> String {
    let mut out = String::new();

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum { continue; }
        if s.fields.is_empty() { continue; }

        writeln!(out, "/** Factory: create a default valid {} */", s.name).unwrap();
        writeln!(out, "fun default{}(): {} = {}(", s.name, s.name, s.name).unwrap();
        for (i, f) in s.fields.iter().enumerate() {
            let val = if f.value_type.is_some() {
                "emptyMap()".to_string()
            } else {
                kt_default_value(&f.target, &f.mult)
            };
            let comma = if i < s.fields.len() - 1 { "," } else { "" };
            writeln!(out, "    {} = {val}{comma}", f.name).unwrap();
        }
        writeln!(out, ")").unwrap();
        writeln!(out).unwrap();

        // Boundary value fixtures (Feature 5)
        let has_bounds = s.fields.iter().any(|f| {
            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
        });
        if has_bounds {
            writeln!(out, "/** Factory: create {} at cardinality boundary */", s.name).unwrap();
            writeln!(out, "fun boundary{}(): {} = {}(", s.name, s.name, s.name).unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = if f.value_type.is_some() {
                    "emptyMap()".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let count = match &bound {
                            analyze::BoundKind::Exact(n) => *n,
                            analyze::BoundKind::AtMost(n) => *n,
                            analyze::BoundKind::AtLeast(n) => *n,
                        };
                        kt_boundary_value(&f.target, &f.mult, count)
                    } else {
                        kt_default_value(&f.target, &f.mult)
                    }
                } else {
                    kt_default_value(&f.target, &f.mult)
                };
                writeln!(out, "    {} = {val}{comma}", f.name).unwrap();
            }
            writeln!(out, ")").unwrap();
            writeln!(out).unwrap();

            writeln!(out, "/** Factory: create {} that violates cardinality constraint */", s.name).unwrap();
            writeln!(out, "fun invalid{}(): {} = {}(", s.name, s.name, s.name).unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = if f.value_type.is_some() {
                    "emptyMap()".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let violation = match &bound {
                            analyze::BoundKind::Exact(n) => n + 1,
                            analyze::BoundKind::AtMost(n) => n + 1,
                            analyze::BoundKind::AtLeast(n) => if *n > 0 { n - 1 } else { 0 },
                        };
                        kt_boundary_value(&f.target, &f.mult, violation)
                    } else {
                        kt_default_value(&f.target, &f.mult)
                    }
                } else {
                    kt_default_value(&f.target, &f.mult)
                };
                writeln!(out, "    {} = {val}{comma}", f.name).unwrap();
            }
            writeln!(out, ")").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

fn kt_boundary_value(target: &str, mult: &Multiplicity, count: usize) -> String {
    match mult {
        Multiplicity::Set => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            if items.is_empty() {
                "emptySet()".to_string()
            } else {
                format!("setOf({})", items.join(", "))
            }
        }
        Multiplicity::Seq => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            if items.is_empty() {
                "emptyList()".to_string()
            } else {
                format!("listOf({})", items.join(", "))
            }
        }
        _ => kt_default_value(target, mult),
    }
}

fn kt_return_type(type_name: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => type_name.to_string(),
        Multiplicity::Lone => format!("{type_name}?"),
        Multiplicity::Set => format!("Set<{type_name}>"),
        Multiplicity::Seq => format!("List<{type_name}>"),
    }
}

fn kt_default_value(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set => "emptySet()".to_string(),
        Multiplicity::Seq => "emptyList()".to_string(),
        Multiplicity::One => format!("default{target}()"),
    }
}
