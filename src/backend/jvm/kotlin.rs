use super::{JvmContext, expr_translator};
use super::expr_translator::JvmLang;
use crate::backend::{GeneratedFile, TargetLang, is_native_type_alias, resolve_type};
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity, TemporalBinaryOp};
use crate::analyze;
use std::collections::{HashMap, HashSet};
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
        // Alloy `#e` is an Int, which Kotlin models as Long; `size` is an Int,
        // so returning it from a `fun ...: one Int` does not type-check.
        format!("{expr}.size.toLong()")
    }
    fn lone_eq(&self, base: &str, field: &str, value: &str) -> String {
        format!("{base}.{field} == {value}")
    }
    fn tc_call(&self, field: &str, base: &str) -> String {
        format!("tc{}({base})", capitalize(field))
    }
    fn rtc_call(&self, field: &str, base: &str) -> String {
        format!("rtc{}({base})", capitalize(field))
    }
    fn eq_op(&self) -> &str { "==" }
    fn neq_op(&self) -> &str { "!=" }
    // Kotlin's `==` is `equals()`, so value equality needs nothing special.
    fn value_eq(&self, l: &str, r: &str) -> String { format!("{l} == {r}") }
    fn value_neq(&self, l: &str, r: &str) -> String { format!("{l} != {r}") }
    // Alloy's `Int` is `Long` here, and Kotlin will not compare the two.
    fn int_literal(&self, n: i64) -> String { format!("{n}L") }
    // `List` has `flatMap`/`map` directly.
    fn relational_image(&self, extent: &str, read: &str, mult: &Multiplicity) -> String {
        match mult {
            Multiplicity::Set | Multiplicity::Seq => format!("{extent}.flatMap {{ s -> {read} }}"),
            Multiplicity::Lone => format!("{extent}.mapNotNull {{ s -> {read} }}"),
            Multiplicity::One => format!("{extent}.map {{ s -> {read} }}"),
        }
    }
    // A sealed class hierarchy: the case is a smart-castable type test.
    fn is_variant(&self, subject: &str, variant: &str) -> String {
        format!("{subject} is {variant}")
    }
    // A derived field is an extension function, so the receiver is `this`.
    fn receiver_expr(&self) -> &str { "this" }
}

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = JvmContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "Models.kt".to_string(),
        content: generate_models(ir, &ctx),
    });

    let has_tc = ir_uses_tc(ir);

    // Generate Helpers.kt for TC functions (replaces Invariants.kt)
    if has_tc {
        files.push(GeneratedFile {
            path: "Helpers.kt".to_string(),
            content: generate_helpers(ir),
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
        // Intersection type → interface Foo : A, B, C
        if !s.intersection_of.is_empty() {
            let parents = s.intersection_of.join(", ");
            writeln!(out, "interface {} : {}", s.name, parents).unwrap();
            writeln!(out).unwrap();
            continue;
        }
        if ctx.is_variant(&s.name) { continue; }
        if is_native_type_alias(&s.name) { continue; }

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

    // Derived fields: receiver functions → extension functions
    generate_derived_fields(&mut out, ir);

    out
}

fn generate_derived_fields(out: &mut String, ir: &OxidtrIR) {
    for op in &ir.operations {
        if let Some(ref sig) = op.receiver_sig {
            let params = op.params.iter().map(|p| {
                let type_str = kt_return_type(&p.type_name, &p.mult);
                format!("{}: {type_str}", p.name)
            }).collect::<Vec<_>>().join(", ");

            let return_str = match &op.return_type {
                Some(rt) => format!(": {}", kt_return_type(&rt.type_name, &rt.mult)),
                None => String::new(),
            };

            writeln!(out, "fun {sig}.{}({params}){return_str} {{", op.name).unwrap();
            {
                let lang = KotlinLang;
                let env = crate::backend::type_env::operation_env(op);
                if op.body.is_empty() {
                    writeln!(out, "    return true").unwrap();
                } else {
                    let body_expr = &op.body[op.body.len() - 1];
                    let body_str = expr_translator::translate_with_env(body_expr, ir, &lang, &env);
                    writeln!(out, "    return {body_str}").unwrap();
                }
            }
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }
}

fn generate_data_class(out: &mut String, s: &StructureNode, ir: &OxidtrIR, disj_fields: &[(String, String)]) {
    // Empty sig → Kotlin object (no need for placeholder fields)
    if s.fields.is_empty() {
        if s.is_var {
            writeln!(out, "// @alloy: var sig").unwrap();
        }
        writeln!(out, "object {}", s.name).unwrap();
        return;
    }

    // Kotlin-specific: single-field wrappers with constraints → @JvmInline value class
    // Only when: single field, has cardinality constraint (suggests validated wrapper),
    // not self-referential, not targeting a lone/some sig
    if s.fields.len() == 1 && s.parent.is_none() && !s.is_enum {
        let f = &s.fields[0];
        let target_mult = analyze::sig_multiplicity_for(ir, &f.target);
        let has_constraint = analyze::constraints_for_sig(ir, &s.name).iter().any(|c| {
            matches!(c, analyze::ConstraintInfo::CardinalityBound { .. })
        });
        if has_constraint
            && f.mult == Multiplicity::One && f.target != s.name && f.value_type.is_none()
            && target_mult == SigMultiplicity::Default
        {
            let type_str = mult_to_kt_type(&resolve_type(TargetLang::Kotlin, &f.target), &f.mult);
            if s.is_var {
                writeln!(out, "// @alloy: var sig").unwrap();
            }
            writeln!(out, "@JvmInline").unwrap();
            writeln!(out, "value class {}(val {}: {type_str})", s.name, f.name).unwrap();
            return;
        }
    }

    if s.is_var {
        writeln!(out, "// @alloy: var sig").unwrap();
    }
    // s.fields is non-empty here (empty sigs return early as `object`)
    {
        writeln!(out, "data class {}(", s.name).unwrap();
        for (i, f) in s.fields.iter().enumerate() {
            let resolved_target = resolve_type(TargetLang::Kotlin, &f.target);
            let type_str = if let Some(vt) = &f.value_type {
                let resolved_vt = resolve_type(TargetLang::Kotlin, vt);
                format!("Map<{}, {}>", resolved_target, resolved_vt)
            } else if let Some(_raw) = &f.raw_union_type {
                // Union types → Any (Kotlin lacks field-level union types without sealed classes)
                match f.mult {
                    Multiplicity::Lone => "Any?".to_string(),
                    Multiplicity::Set  => "Set<Any>".to_string(),
                    Multiplicity::Seq  => "List<Any>".to_string(),
                    Multiplicity::One  => "Any".to_string(),
                }
            } else {
                mult_to_kt_type(&resolved_target, &f.mult)
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
                            // A comment, as Java's emitter and every other
                            // annotation here already are: a live `@Size` is
                            // `jakarta.validation.constraints.Size`, which is
                            // an external dependency the generated project
                            // does not carry — `Unresolved reference 'Size'`.
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
            // Gap 1: lone sig target → make nullable even if field mult is One
            if target_mult == SigMultiplicity::Lone && f.mult == Multiplicity::One {
                annotations.push("/* @Nullable — lone sig may not exist */".to_string());
            }
            // NoSelfRef: field must not reference self
            let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
            let no_self_ref = sig_constraints.iter().any(|c| {
                matches!(c, analyze::ConstraintInfo::NoSelfRef { field_name: fname, .. } if fname == &f.name)
            });
            if no_self_ref {
                annotations.push(format!("/* require({} != this) — no self-reference */", f.name));
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
            for ann in &annotations {
                writeln!(out, "    {ann}").unwrap();
            }
            let val_or_var = if f.is_var { "var" } else { "val" };
            writeln!(out, "    {val_or_var} {}: {type_str}{comma}", f.name).unwrap();
        }
        // Sig-level constraint annotations (FieldOrdering → init block)
        let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
        let mut init_checks: Vec<String> = Vec::new();
        for c in &sig_constraints {
            match c {
                analyze::ConstraintInfo::FieldOrdering { left_field, op, right_field, .. } => {
                    let op_str = match op {
                        CompareOp::Lt => "<",
                        CompareOp::Gt => ">",
                        CompareOp::Lte => "<=",
                        CompareOp::Gte => ">=",
                        _ => continue,
                    };
                    init_checks.push(format!("require({left_field} {op_str} {right_field}) {{ \"{left_field} must be {op_str} {right_field}\" }}"));
                }
                analyze::ConstraintInfo::NoSelfRef { field_name, .. } => {
                    init_checks.push(format!("require({field_name} !== this) {{ \"{field_name} must not reference self\" }}"));
                }
                analyze::ConstraintInfo::Acyclic { field_name, .. } => {
                    init_checks.push(format!("run {{ val seen = mutableSetOf<Any>(); var cur: Any? = this; while (cur != null) {{ require(seen.add(cur)) {{ \"{field_name} must not form a cycle\" }}; cur = (cur as? {sig_name})?.{field_name} }} }}", sig_name = s.name));
                }
                analyze::ConstraintInfo::Implication { condition, consequent, .. } => {
                    let cond = translate_validator_expr_kt(condition, &s.name);
                    let cons = translate_validator_expr_kt(consequent, &s.name);
                    let desc = format!("{} implies {}", analyze::describe_expr(condition), analyze::describe_expr(consequent));
                    init_checks.push(format!("require(!({cond}) || ({cons})) {{ \"{}\" }}", desc.replace('"', "\\\"")));
                }
                analyze::ConstraintInfo::Iff { left, right, .. } => {
                    let l = translate_validator_expr_kt(left, &s.name);
                    let r = translate_validator_expr_kt(right, &s.name);
                    let desc = format!("{} iff {}", analyze::describe_expr(left), analyze::describe_expr(right));
                    init_checks.push(format!("require(({l}) == ({r})) {{ \"{}\" }}", desc.replace('"', "\\\"")));
                }
                analyze::ConstraintInfo::Prohibition { condition, .. } => {
                    let cond = translate_validator_expr_kt(condition, &s.name);
                    let desc = analyze::describe_expr(condition);
                    init_checks.push(format!("require(!({cond})) {{ \"prohibited: {}\" }}", desc.replace('"', "\\\"")));
                }
                analyze::ConstraintInfo::Disjoint { left, right, .. } => {
                    let left_field = left.rsplit('.').next().unwrap_or(left);
                    let right_field = right.rsplit('.').next().unwrap_or(right);
                    init_checks.push(format!("require({left_field}.none {{ it in {right_field} }}) {{ \"{left_field} and {right_field} must not overlap (disjoint constraint)\" }}"));
                }
                analyze::ConstraintInfo::Exhaustive { categories, .. } => {
                    let cats = categories.join(", ");
                    let checks: Vec<String> = categories.iter().map(|cat| {
                        let parts: Vec<&str> = cat.split('.').collect();
                        if parts.len() == 2 {
                            format!("this in {}.{}", parts[0], parts[1])
                        } else {
                            format!("this in {cat}")
                        }
                    }).collect();
                    let condition = checks.join(" || ");
                    init_checks.push(format!("require({condition}) {{ \"must belong to one of [{cats}] (exhaustive constraint)\" }}"));
                }
                _ => {}
            }
        }
        // Disj uniqueness checks for seq fields
        let disj = analyze::disj_fields(ir);
        for (dsig, dfield) in &disj {
            if dsig == &s.name {
                if let Some(f) = s.fields.iter().find(|f| f.name == *dfield) {
                    if f.mult == Multiplicity::Seq {
                        init_checks.push(format!("require({dfield}.toSet().size == {dfield}.size) {{ \"{dfield} must not contain duplicates (disj constraint)\" }}"));
                    }
                }
            }
        }
        if init_checks.is_empty() {
            writeln!(out, ")").unwrap();
        } else {
            writeln!(out, ") {{").unwrap();
            writeln!(out, "    init {{").unwrap();
            for check in &init_checks {
                writeln!(out, "        {check}").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
}


fn generate_sealed_class(out: &mut String, s: &StructureNode, ctx: &JvmContext) {
    let variants = ctx.children.get(&s.name);

    // Parent abstract sig may have fields that should be inherited by all variants
    let parent_fields = &s.fields;

    // Check if all variants are unit (no fields, including inherited, singleton)
    let all_unit = parent_fields.is_empty() && variants.map_or(true, |vs| {
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
                let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
                // Combine parent fields + child fields
                let all_fields: Vec<&IRField> = parent_fields.iter().chain(child_fields.iter().copied()).collect();
                if !all_fields.is_empty() {
                    writeln!(out, "data class {}(", v).unwrap();
                    for (i, f) in all_fields.iter().enumerate() {
                        // A variant's field needs its native alias resolved
                        // just as a data class's does — `Int` is a *32-bit*
                        // type in Kotlin, so the unresolved name compiled and
                        // then rejected the `Long` the fixture passes.
                        let resolved = resolve_type(TargetLang::Kotlin, &f.target);
                        let type_str = if let Some(vt) = &f.value_type {
                            format!("Map<{}, {}>", resolved,
                                resolve_type(TargetLang::Kotlin, vt))
                        } else {
                            mult_to_kt_type(&resolved, &f.mult)
                        };
                        let comma = if i < all_fields.len() - 1 { "," } else { "" };
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

// ── Helpers.kt ─────────────────────────────────────────────────────────────

/// Generate Helpers.kt containing TC / RTC functions.
fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();

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

fn generate_rtc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("rtc{}", capitalize(&tc.field_name));
    let tc_name = format!("tc{}", capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "/** Reflexive-transitive closure for {sig}.{field} (id ∪ ^{field}). */").unwrap();
    writeln!(out, "fun {fn_name}(start: {sig}): List<{sig}> {{").unwrap();
    writeln!(out, "    val result = mutableListOf(start)").unwrap();
    writeln!(out, "    result.addAll({tc_name}(start))").unwrap();
    writeln!(out, "    return result").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ── Operations.kt ──────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    for op in &ir.operations {
        if op.receiver_sig.is_some() {
            continue;
        }
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
        }

        // An Alloy `pred` is a formula, not a procedure: it denotes true or
        // false, and its body is the conjunction of its clauses (#82).
        let return_str = match &op.return_type {
            Some(rt) => format!(": {}", kt_return_type(&rt.type_name, &rt.mult)),
            None => ": Boolean".to_string(),
        };

        writeln!(out, "fun {}({params}){return_str} {{", op.name).unwrap();
        {
            let lang = KotlinLang;
            let env = crate::backend::type_env::operation_env(op);
            if op.body.is_empty() {
                writeln!(out, "    return true").unwrap();
            } else if op.return_type.is_some() {
                let body = expr_translator::translate_with_env(&op.body[0], ir, &lang, &env);
                writeln!(out, "    return {body}").unwrap();
            } else {
                let conjuncts: Vec<String> = op.body.iter()
                    .map(|e| expr_translator::translate_with_env(e, ir, &lang, &env))
                    .collect();
                writeln!(out, "    return {}", conjuncts.join(" && ")).unwrap();
            }
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── Tests.kt ───────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let fixture_types = crate::backend::collect_fixture_types(ir);
    let sig_names = expr_translator::collect_sig_names(ir);
    let lang = KotlinLang;

    let has_fixture = crate::backend::collect_fixture_types(ir);

    writeln!(out, "import org.junit.jupiter.api.Test").unwrap();
    writeln!(out, "import org.junit.jupiter.api.Disabled").unwrap();
    writeln!(out, "import org.junit.jupiter.api.Assertions.*").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "class PropertyTests {{").unwrap();

    for prop in &ir.properties {
        let params = expr_translator::extract_params(&prop.expr, &sig_names, ir);
        let body = expr_translator::translate_with_ir(&prop.expr, ir, &lang);

        // An `assert` carries temporal operators just as a `fact` does, and
        // translating its operand alone silently drops them: `eventually P`
        // became `P`, `P until Q` became `P && Q` (#78). Route it through the
        // trace checkers the fact path already emits.
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
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    fun `{}`() {{", prop.name).unwrap();
            writeln!(out, "        // {label}: full verification needs a trace; an empty trace").unwrap();
            writeln!(out, "        // can never satisfy it, which at least exercises the checker.").unwrap();
            match temporal_checker_name(&prop.name, &prop.expr, temporal_kind) {
                Some(checker) => {
                    let tname = params.first().map(|(_, t)| t.as_str()).unwrap_or("Nothing");
                    writeln!(out, "        val trace: List<List<{tname}>> = emptyList()").unwrap();
                    writeln!(out, "        assertFalse({checker}(trace))").unwrap();
                }
                None => {
                    writeln!(out, "        // oxidtr: no checker emitted for this shape").unwrap();
                }
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            emit_temporal_trace_checkers(&mut out, &prop.name, &prop.expr, &params, &body, ir, temporal_kind);
            continue;
        }

        writeln!(out, "    @Test").unwrap();
        writeln!(out, "    fun `{}`() {{", prop.name).unwrap();
        for (pname, tname) in &params {
            // An empty domain makes `all` vacuously true, so the test passes
            // whatever the implementation does (#81). Seed it from the fixture
            // wherever one exists, and disclose it where one does not.
            if fixture_types.contains(tname) {
                writeln!(out, "        val {pname}: List<{tname}> = listOf(default{tname}())").unwrap();
            } else {
                writeln!(out, "        // @coverage empty domain: no fixture for `{tname}`;").unwrap();
                writeln!(out, "        // this quantifier is vacuously satisfied.").unwrap();
                writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
            }
        }
        writeln!(out, "        assertTrue({body})").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Kotlin has strong null safety (T?) — skip tests for null-safety constraints
    let all_constraints = analyze::analyze(ir);
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };

        // Alloy 6: temporal facts with prime → generate transition test
        if analyze::expr_contains_prime(&constraint.expr) {
            let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "    /** @temporal Transition constraint: {fact_name} */").unwrap();
            writeln!(out, "    /** Verifies: pre→post state relationship ({desc}) */").unwrap();
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    fun `transition {fact_name}`() {{").unwrap();
            for (pname, tname) in &params {
                writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
                writeln!(out, "        val next_{pname}: List<{tname}> = {pname}.toList()").unwrap();
            }
            if let Some((_kind, bindings, inner_body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                let rewritten_body = analyze::rewrite_prime_as_post_state(inner_body);
                let body_str = expr_translator::translate_with_ir(&rewritten_body, ir, &lang);
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
                        writeln!(out, "        {pname}.zip(next_{pname}).forEach {{ ({v}, next_{v}) ->").unwrap();
                        writeln!(out, "            assertTrue({body_str})").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    _ => {
                        writeln!(out, "        // oxidtr: skipped — a transition over {} binding(s) has no \
                            pre/post pairing to walk. See #104.", bind_vars.len()).unwrap();
                    }
                }
            } else {
                let rewritten = analyze::rewrite_prime_as_post_state(&constraint.expr);
                let body = expr_translator::translate_with_ir(&rewritten, ir, &lang);
                writeln!(out, "        assertTrue({body})").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir, &lang);

        // Check if all related constraints are type-guaranteed in Kotlin
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
            can_guarantee_by_type(c, TargetLang::Kotlin) == Guarantee::FullyByType
        });

        if all_fully {
            writeln!(out, "    // Type-guaranteed: {} — Kotlin type system handles this", fact_name).unwrap();
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
        if let Some(ref kind) = temporal_kind {
            let note = match kind {
                analyze::TemporalKind::Liveness | analyze::TemporalKind::PastLiveness =>
                    " — liveness property: cannot be fully verified at runtime; static test approximates via implies",
                analyze::TemporalKind::Binary =>
                    " — binary temporal: requires trace-based verification",
                _ => "",
            };
            writeln!(out, "    /** @temporal {:?} constraint: {fact_name}{note} */", kind).unwrap();
        }

        // Binary temporal: static test cannot meaningfully assert the body
        if temporal_kind == Some(analyze::TemporalKind::Binary) {
            let op_label = if let Some((op, _, _)) = analyze::find_temporal_binary(&constraint.expr) {
                match op {
                    TemporalBinaryOp::Until => "Until",
                    TemporalBinaryOp::Since => "Since",
                    TemporalBinaryOp::Release => "Release",
                    TemporalBinaryOp::Triggered => "Triggered",
                }
            } else { "Binary" };
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    fun `{test_prefix} {fact_name}`() {{").unwrap();
            writeln!(out, "        // binary temporal: requires trace-based verification; see check{op_label}{fact_name}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else if matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            let kind_label = if temporal_kind == Some(analyze::TemporalKind::Liveness) {
                "Liveness" } else { "PastLiveness" };
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    fun `{test_prefix} {fact_name}`() {{").unwrap();
            writeln!(out, "        // {}: requires trace-based verification; see check{kind_label}{fact_name}", test_prefix).unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else {
        writeln!(out, "    @Test").unwrap();
        writeln!(out, "    fun `{test_prefix} {fact_name}`() {{").unwrap();
        // `all f: IRField | some sn: StructureNode | f in sn.irFields` is not
        // true of a sample of two unrelated defaults. Build the link first, as
        // the TypeScript backend already does — otherwise seeding the domains
        // turns a vacuous pass into a false failure rather than a real check.
        let ownership = crate::backend::detect_ownership_pattern(
            &constraint.expr, ir, expr_translator::to_camel_plural);
        let mut linked: HashSet<String> = HashSet::new();
        if let Some((owned_var, owner_var, owner_type, field_name)) = &ownership {
            let owned = params.iter().find(|(p, _)| p == owned_var);
            let owner = params.iter().find(|(p, _)| p == owner_var);
            if let (Some((opname, otname)), Some((cpname, ctname))) = (owned, owner) {
                let of = collection_of(ir, owner_type, field_name);
                writeln!(out, "        val item = default{otname}()").unwrap();
                writeln!(out, "        val owner = default{ctname}().copy({field_name} = {of}(item))").unwrap();
                writeln!(out, "        val {opname}: List<{otname}> = listOf(item)").unwrap();
                writeln!(out, "        val {cpname}: List<{ctname}> = listOf(owner)").unwrap();
                linked.insert(opname.clone());
                linked.insert(cpname.clone());
            }
        }
        for (pname, tname) in &params {
            // The `assert` path above already seeds from the factory; this one
            // wrote `emptyList()` unconditionally, so `all` was vacuously true
            // and the test proved nothing about the fact (#136, #81).
            if linked.contains(pname) {
                // already materialised as the owner/owned pair
            } else if fixture_types.contains(tname) {
                writeln!(out, "        val {pname}: List<{tname}> = listOf(default{tname}())").unwrap();
            } else {
                writeln!(out, "        // @coverage empty domain: no fixture for `{tname}`;").unwrap();
                writeln!(out, "        // this quantifier is vacuously satisfied.").unwrap();
                writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
            }
        }
        writeln!(out, "        assertTrue({body})").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
        } // end non-binary temporal

        emit_temporal_trace_checkers(&mut out, &fact_name, &constraint.expr, &params, &body, ir, temporal_kind);
    }

    // Boundary value tests (Feature 5) — inline expressions
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
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
            writeln!(out, "        assertTrue({body})").unwrap();
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
            writeln!(out, "        assertFalse(!({body}))").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Cross-tests — verify constraints are preserved after operations
    if !ir.constraints.is_empty() && !ir.operations.is_empty() {
        writeln!(out, "    // --- Cross-tests: fact x operation ---").unwrap();
        writeln!(out).unwrap();
        for constraint in &ir.constraints {
            let fact_name = match &constraint.name { Some(n) => n.clone(), None => continue };
            let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
            let body = expr_translator::translate_with_ir(&constraint.expr, ir, &lang);
            for op in &ir.operations {
                writeln!(out, "    @Disabled(\"cross-test: operation stub not yet implemented\")").unwrap();
                writeln!(out, "    @Test").unwrap();
                writeln!(out, "    fun `{fact_name} preserved after {}`() {{", op.name).unwrap();
                for (pname, tname) in &params {
                    if has_fixture.contains(tname) {
                        writeln!(out, "        val {pname}: List<{tname}> = listOf(default{tname}())").unwrap();
                    } else {
                        writeln!(out, "        val {pname}: List<{tname}> = emptyList()").unwrap();
                    }
                }
                writeln!(out, "        // pre: constraint holds before operation").unwrap();
                writeln!(out, "        assertTrue({body})").unwrap();
                writeln!(out, "        // operation: {}(...)", op.name).unwrap();
                writeln!(out, "        // post: constraint still holds").unwrap();
                writeln!(out, "        assertTrue({body})").unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    // --- Anomaly tests ---
    let anomalies = analyze::detect_anomalies(ir);
    if !anomalies.is_empty() {
        writeln!(out, "    // --- Anomaly tests: edge-case coverage ---").unwrap();
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
                        writeln!(out, "    @Test").unwrap();
                        writeln!(out, "    fun `anomaly - {sig_name} {field_name} unconstrained`() {{").unwrap();
                        writeln!(out, "        val instance = default{sig_name}()").unwrap();
                        writeln!(out, "        instance.{field_name} // unconstrained field access").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        writeln!(out, "    @Test").unwrap();
                        writeln!(out, "    fun `anomaly - {sig_name} {field_name} empty edge case`() {{").unwrap();
                        writeln!(out, "        val instance = anomalyEmpty{sig_name}()").unwrap();
                        writeln!(out, "        instance.{field_name} // empty edge case").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        writeln!(out, "    @Test").unwrap();
                        writeln!(out, "    fun `anomaly - {sig_name} {field_name} self-ref unguarded`() {{").unwrap();
                        writeln!(out, "        val instance = default{sig_name}()").unwrap();
                        writeln!(out, "        instance.{field_name} // self-ref without guard").unwrap();
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

        let mut seen_cover_names: HashSet<String> = HashSet::new();
        for pair in &coverage.pairwise {
            if !has_fixture.contains(&pair.sig_name) { continue; }
            let fact_a_snake = to_snake_case(&pair.fact_a);
            let fact_b_snake = to_snake_case(&pair.fact_b);

            let test_name = format!("cover {fact_a_snake} x {fact_b_snake}");
            if !seen_cover_names.insert(test_name.clone()) { continue; }

            let constraint_a = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_a));
            let constraint_b = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_b));

            let body_a = constraint_a
                .map(|c| expr_translator::translate_with_ir(&c.expr, ir, &lang));
            let body_b = constraint_b
                .map(|c| expr_translator::translate_with_ir(&c.expr, ir, &lang));

            // Collect params from both constraint expressions
            let mut all_params: Vec<(String, String)> = Vec::new();
            let mut param_names: HashSet<String> = HashSet::new();
            if let Some(c) = constraint_a {
                for p in expr_translator::extract_params(&c.expr, &sig_names, ir) {
                    if param_names.insert(p.0.clone()) {
                        all_params.push(p);
                    }
                }
            }
            if let Some(c) = constraint_b {
                for p in expr_translator::extract_params(&c.expr, &sig_names, ir) {
                    if param_names.insert(p.0.clone()) {
                        all_params.push(p);
                    }
                }
            }

            writeln!(out, "    @Disabled").unwrap();
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    fun `{test_name}`() {{").unwrap();
            for (pname, tname) in &all_params {
                writeln!(out, "        val {pname}: List<{tname}> = listOf(default{tname}())").unwrap();
            }
            if let (Some(a), Some(b)) = (&body_a, &body_b) {
                writeln!(out, "        assertTrue({a})").unwrap();
                writeln!(out, "        assertTrue({b})").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── helpers ────────────────────────────────────────────────────────────────

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

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

/// Translate an Alloy expression to Kotlin for single-instance validator context.
fn translate_validator_expr_kt(expr: &crate::parser::ast::Expr, sig_name: &str) -> String {
    use crate::parser::ast::{Expr, LogicOp, QuantKind};
    match expr {
        Expr::VarRef(name) => {
            if name == sig_name { "this".to_string() } else { name.clone() }
        }
        Expr::IntLiteral(n) => n.to_string(),
        Expr::FieldAccess { base, field } => {
            format!("{}.{}", translate_validator_expr_kt(base, sig_name), field)
        }
        Expr::Comparison { op, left, right } => {
            let l = translate_validator_expr_kt(left, sig_name);
            let r = translate_validator_expr_kt(right, sig_name);
            let o = match op {
                CompareOp::Eq => "==",
                CompareOp::NotEq => "!=",
                CompareOp::In => return format!("{l} in {r}"),
                CompareOp::Lt => "<",
                CompareOp::Gt => ">",
                CompareOp::Lte => "<=",
                CompareOp::Gte => ">=",
            };
            format!("{l} {o} {r}")
        }
        Expr::BinaryLogic { op, left, right } => {
            let l = translate_validator_expr_kt(left, sig_name);
            let r = translate_validator_expr_kt(right, sig_name);
            match op {
                LogicOp::And => format!("{l} && {r}"),
                LogicOp::Or => format!("{l} || {r}"),
                LogicOp::Implies => format!("!({l}) || {r}"),
                LogicOp::Iff => format!("({l}) == ({r})"),
            }
        }
        Expr::Not(inner) => format!("!({})", translate_validator_expr_kt(inner, sig_name)),
        Expr::MultFormula { kind, expr: inner } => {
            let e = translate_validator_expr_kt(inner, sig_name);
            match kind {
                QuantKind::Some => format!("{e} != null"),
                QuantKind::No => format!("{e} == null"),
                _ => e,
            }
        }
        Expr::Cardinality(inner) => {
            format!("{}.size", translate_validator_expr_kt(inner, sig_name))
        }
        _ => analyze::describe_expr(expr), // fallback: human-readable
    }
}

// ── Fixtures.kt ────────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &JvmContext) -> String {
    let mut out = String::new();

    // Having no field is not having no value, and what has no factory is what
    // has no finite value at all (#109, #105).
    let (terminating, enum_witness) = crate::backend::terminating_types(ir);
    let fixture_types: std::collections::HashSet<String> = ir.structures.iter()
        .filter(|s| !ctx.is_variant(&s.name) && !s.is_enum
            && !crate::backend::is_native_type_alias(&s.name)
            && terminating.contains(&s.name))
        .map(|s| s.name.clone())
        .collect();

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
            if ctx.enum_is_flat(s) {
                // Enum class → qualified access: EnumName.Variant. Any variant
                // will do; none of them carries a field.
                let Some(variant) = variants.first() else { continue };
                writeln!(out, "/** Factory: default value for {} */", s.name).unwrap();
                writeln!(out, "fun default{}(): {} = {}.{}", s.name, s.name, s.name, variant).unwrap();
                writeln!(out).unwrap();
                continue;
            }
            // Sealed class → the case whose payload was satisfiable when the
            // enum was admitted; any other may lead straight back into it.
            let Some(variant) = enum_witness.get(&s.name) else { continue };
            let fields = ctx.variant_fields(s, variant);
            writeln!(out, "/** Factory: default value for {} */", s.name).unwrap();
            if fields.is_empty() {
                writeln!(out, "fun default{}(): {} = {}", s.name, s.name, variant).unwrap();
            } else {
                let args: Vec<String> = fields.iter()
                    .map(|f| {
                        let val = if f.value_type.is_some() {
                            "emptyMap()".to_string()
                        } else {
                            kt_default_value(&f.target, &f.mult)
                        };
                        format!("{} = {val}", f.name)
                    })
                    .collect();
                writeln!(out, "fun default{}(): {} = {}({})",
                    s.name, s.name, variant, args.join(", ")).unwrap();
            }
            writeln!(out).unwrap();
        }
    }

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum { continue; }
        // A field-less sig is still a value, and a fixture with a field of that
        // type calls the factory whether or not one was emitted (#105).
        if !terminating.contains(&s.name) { continue; }
        if s.fields.is_empty() {
            // A field-less sig is emitted as an `object`: the singleton is the
            // value, and `Person()` would be a call to a constructor it has not.
            writeln!(out, "/** Factory: create a default valid {} */", s.name).unwrap();
            writeln!(out, "fun default{}(): {} = {}", s.name, s.name, s.name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        writeln!(out, "/** Factory: create a default valid {} */", s.name).unwrap();
        writeln!(out, "fun default{}(): {} = {}(", s.name, s.name, s.name).unwrap();
        for (i, f) in s.fields.iter().enumerate() {
            let val = if f.value_type.is_some() {
                "emptyMap()".to_string()
            } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && super::super::is_safe_set_population(&s.name, &f.target, ir, &fixture_types) {
                let safe = HashSet::from([f.target.clone()]);
                kt_default_value_inner(&f.target, &f.mult, &safe)
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
                        kt_boundary_value(ir, &f.target, &f.mult, count)
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
                        kt_boundary_value(ir, &f.target, &f.mult, violation)
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
            if ctx.is_variant(&s.name) || s.is_enum || s.fields.is_empty() { continue; }
            anomaly_sigs_done.insert(sig_name.clone());

            writeln!(out, "/** Anomaly fixture: all collections empty */").unwrap();
            writeln!(out, "fun anomalyEmpty{sig_name}(): {sig_name} = {sig_name}(").unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = match &f.mult {
                    Multiplicity::Set => "emptySet()".to_string(),
                    Multiplicity::Seq => "emptyList()".to_string(),
                    _ => kt_default_value(&f.target, &f.mult),
                };
                writeln!(out, "    {} = {}{}", f.name, val, comma).unwrap();
            }
            writeln!(out, ")").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

/// `count` elements of `target`, each distinct from the others.
///
/// `setOf(defaultItem(), defaultItem())` collapses to one element — a data
/// class has structural `equals` — so the fixture never reached the
/// cardinality it was named for. A native element had it worse still: it
/// emitted `defaultInt()`, a call to a factory that does not exist (#96).
fn kt_distinct_elements(ir: &OxidtrIR, target: &str, count: usize) -> Vec<String> {
    let fallback = || vec![format!("default{target}()"); count];
    if super::jvm_native_element(target, 0).is_some() {
        return (0..count).filter_map(|i| super::jvm_native_element(target, i)).collect();
    }
    if crate::backend::is_native_type_alias(target) {
        // `Bool`, which has no per-index literal; `kt_default_value` knows it.
        return vec![kt_default_value(target, &Multiplicity::One); count];
    }
    let Some(s) = ir.structures.iter().find(|st| st.name == target) else {
        return fallback();
    };
    let Some(idx) = super::jvm_diversity_field(s) else { return fallback() };

    (0..count)
        .map(|i| {
            let args: Vec<String> = s.fields.iter().enumerate()
                .map(|(j, f)| {
                    let val = if j == idx {
                        super::jvm_native_element(&f.target, i).unwrap()
                    } else if f.value_type.is_some() {
                        "emptyMap()".to_string()
                    } else {
                        kt_default_value(&f.target, &f.mult)
                    };
                    format!("{} = {val}", f.name)
                })
                .collect();
            format!("{target}({})", args.join(", "))
        })
        .collect()
}

fn kt_boundary_value(ir: &OxidtrIR, target: &str, mult: &Multiplicity, count: usize) -> String {
    let items = kt_distinct_elements(ir, target, count);
    match mult {
        Multiplicity::Set if items.is_empty() => "emptySet()".to_string(),
        Multiplicity::Set => format!("setOf({})", items.join(", ")),
        Multiplicity::Seq if items.is_empty() => "emptyList()".to_string(),
        Multiplicity::Seq => format!("listOf({})", items.join(", ")),
        _ => kt_default_value(target, mult),
    }
}

fn kt_return_type(type_name: &str, mult: &Multiplicity) -> String {
    // `Int`/`Str`/`Bool` are Alloy marker sigs, not emitted types.
    let type_name = &if crate::backend::is_native_type_alias(type_name) {
        crate::backend::resolve_type(crate::backend::TargetLang::Kotlin, type_name)
    } else {
        type_name.to_string()
    };
    match mult {
        Multiplicity::One => type_name.to_string(),
        Multiplicity::Lone => format!("{type_name}?"),
        Multiplicity::Set => format!("Set<{type_name}>"),
        Multiplicity::Seq => format!("List<{type_name}>"),
    }
}

fn kt_default_value(target: &str, mult: &Multiplicity) -> String {
    kt_default_value_inner(target, mult, &HashSet::new())
}

/// The zero value for an Alloy marker sig, which has no generated factory:
/// `defaultInt()` names nothing, so `Fixtures.kt` did not compile the moment a
/// sig declared a primitive-typed field (#105).
fn kt_native_zero(target: &str) -> Option<&'static str> {
    match target {
        "Int" => Some("0L"),
        "Str" => Some("\"\""),
        "Bool" => Some("false"),
        "Float" => Some("0.0"),
        _ => None,
    }
}

fn kt_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    if let Some(zero) = kt_native_zero(target) {
        return match mult {
            Multiplicity::Lone => "null".to_string(),
            Multiplicity::Set => format!("setOf({zero})"),
            Multiplicity::Seq => format!("listOf({zero})"),
            Multiplicity::One => zero.to_string(),
        };
    }
    match mult {
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set => {
            if safe_targets.contains(target) {
                format!("setOf(default{target}())")
            } else {
                "emptySet()".to_string()
            }
        }
        Multiplicity::Seq => {
            if safe_targets.contains(target) {
                format!("listOf(default{target}())")
            } else {
                "emptyList()".to_string()
            }
        }
        Multiplicity::One => format!("default{target}()"),
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

/// Emit the trace-checker functions a temporal constraint needs.
///
/// Shared by the `fact` and `assert` paths. Only the fact path used to call it,
/// so an `assert` erased its temporal operators entirely (#78).
fn emit_temporal_trace_checkers(
    out: &mut String,
    name: &str,
    expr: &crate::parser::ast::Expr,
    params: &[(String, String)],
    body: &str,
    ir: &OxidtrIR,
    temporal_kind: Option<analyze::TemporalKind>,
) {
    let constraint = TemporalSource { expr };
    let lang = KotlinLang;
    // Generate trace checker functions for temporal constraints
    if let Some(kind) = temporal_kind {
        match kind {
            analyze::TemporalKind::Liveness | analyze::TemporalKind::PastLiveness => {
                let kind_label = if kind == analyze::TemporalKind::Liveness {
                    "Liveness" } else { "PastLiveness" };
                let semantics = if kind == analyze::TemporalKind::Liveness {
                    "property holds in at least one future state"
                } else {
                    "property held in at least one past state"
                };
                writeln!(out, "    /** Trace checker for {kind_label}: {semantics}. */").unwrap();
                if params.len() == 1 {
                    let (pname, tname) = &params[0];
                    writeln!(out, "    fun check{kind_label}{name}(trace: List<List<{tname}>>): Boolean =").unwrap();
                    writeln!(out, "        trace.any {{ {pname} -> {body} }}").unwrap();
                } else {
                    let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("List<{t}>")).collect();
                    writeln!(out, "    fun check{kind_label}{name}(trace: List<Pair<{}>>): Boolean =", tuple_types.join(", ")).unwrap();
                    let destructure: Vec<_> = params.iter().map(|(p, _)| format!("{p}")).collect();
                    writeln!(out, "        trace.any {{ ({}) -> {body} }}", destructure.join(", ")).unwrap();
                }
                writeln!(out).unwrap();
            }
            analyze::TemporalKind::Binary => {
                if let Some((op, left, right)) = analyze::find_temporal_binary(&constraint.expr) {
                    let left_body = expr_translator::translate_with_ir(left, ir, &lang);
                    let right_body = expr_translator::translate_with_ir(right, ir, &lang);
                    let op_name = match op {
                        TemporalBinaryOp::Until => "Until",
                        TemporalBinaryOp::Since => "Since",
                        TemporalBinaryOp::Release => "Release",
                        TemporalBinaryOp::Triggered => "Triggered",
                    };
                    let semantics = match op {
                        TemporalBinaryOp::Until => "left holds until right becomes true",
                        TemporalBinaryOp::Since => "left has held since right was true",
                        TemporalBinaryOp::Release => "right holds until left releases it",
                        TemporalBinaryOp::Triggered => "left triggers right",
                    };
                    writeln!(out, "    /** Trace checker for {op_name}: {semantics}. */").unwrap();
                    if params.len() == 1 {
                        let (pname, tname) = &params[0];
                        writeln!(out, "    fun check{op_name}{name}(trace: List<List<{tname}>>): Boolean {{").unwrap();
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "        val pos = trace.indexOfFirst {{ {pname} -> {right_body} }}").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(0, pos).all {{ {pname} -> {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "        val pos = trace.indexOfLast {{ {pname} -> {right_body} }}").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(pos, trace.size).all {{ {pname} -> {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "        val pos = trace.indexOfFirst {{ {pname} -> {left_body} }}").unwrap();
                                writeln!(out, "        return if (pos >= 0) trace.subList(0, pos + 1).all {{ {pname} -> {right_body} }} else trace.all {{ {pname} -> {right_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "        return trace.indices.all {{ i ->").unwrap();
                                writeln!(out, "            val {pname} = trace[i]").unwrap();
                                writeln!(out, "            if ({right_body}) trace.subList(0, i + 1).any {{ {pname} -> {left_body} }} else true").unwrap();
                                writeln!(out, "        }}").unwrap();
                            }
                        }
                    } else {
                        let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("List<{t}>")).collect();
                        let destructure: Vec<_> = params.iter().map(|(p, _)| format!("{p}")).collect();
                        let pnames = destructure.join(", ");
                        writeln!(out, "    fun check{op_name}{name}(trace: List<Pair<{}>>): Boolean {{", tuple_types.join(", ")).unwrap();
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "        val pos = trace.indexOfFirst {{ ({pnames}) -> {right_body} }}").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(0, pos).all {{ ({pnames}) -> {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "        val pos = trace.indexOfLast {{ ({pnames}) -> {right_body} }}").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(pos, trace.size).all {{ ({pnames}) -> {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "        val pos = trace.indexOfFirst {{ ({pnames}) -> {left_body} }}").unwrap();
                                writeln!(out, "        return if (pos >= 0) trace.subList(0, pos + 1).all {{ ({pnames}) -> {right_body} }} else trace.all {{ ({pnames}) -> {right_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "        return trace.indices.all {{ i ->").unwrap();
                                writeln!(out, "            val ({pnames}) = trace[i]").unwrap();
                                writeln!(out, "            if ({right_body}) trace.subList(0, i + 1).any {{ ({pnames}) -> {left_body} }} else true").unwrap();
                                writeln!(out, "        }}").unwrap();
                            }
                        }
                    }
                    writeln!(out, "    }}").unwrap();
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

/// The Kotlin builder for a one-element collection of `sig.field`, matching the
/// container `resolve_type` gave the field — `setOf` for `set`, `listOf` for
/// `seq`. Passing the wrong one is a type error, not a wrong value.
fn collection_of(ir: &OxidtrIR, sig: &str, field: &str) -> &'static str {
    let mult = ir.structures.iter()
        .find(|s| s.name == sig)
        .and_then(|s| s.fields.iter().find(|f| f.name == field))
        .map(|f| f.mult.clone());
    match mult {
        Some(Multiplicity::Seq) => "listOf",
        _ => "setOf",
    }
}

/// The name `emit_temporal_trace_checkers` will give this constraint's checker,
/// so the generated test can call it. A checker nothing references is a test
/// that asserts nothing — the failure #74/#81 were about.
fn temporal_checker_name(
    name: &str,
    expr: &crate::parser::ast::Expr,
    kind: Option<analyze::TemporalKind>,
) -> Option<String> {
    // The emitter interpolates the constraint name verbatim.
    let camel = name.to_string();
    match kind {
        Some(analyze::TemporalKind::Binary) => {
            let (op, _, _, _) = analyze::find_temporal_binary_with_bindings(expr)?;
            let op_name = match op {
                TemporalBinaryOp::Until => "Until",
                TemporalBinaryOp::Since => "Since",
                TemporalBinaryOp::Release => "Release",
                TemporalBinaryOp::Triggered => "Triggered",
            };
            Some(format!("check{op_name}{camel}"))
        }
        Some(analyze::TemporalKind::PastLiveness) => Some(format!("checkPastLiveness{camel}")),
        Some(analyze::TemporalKind::Liveness) => Some(format!("checkLiveness{camel}")),
        _ => None,
    }
}
