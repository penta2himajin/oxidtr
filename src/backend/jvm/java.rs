use super::{JvmContext, expr_translator};
use super::expr_translator::JvmLang;
use crate::backend::{GeneratedFile, TargetLang, is_native_type_alias, resolve_type};
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity, TemporalBinaryOp};
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
    fn rtc_call(&self, field: &str, base: &str) -> String {
        format!("Helpers.rtc{}({base})", capitalize(field))
    }
    fn eq_op(&self) -> &str { "==" }
    // Alloy `=` is atom equality, and a fixture builds a fresh instance every
    // call — `==` compares references and would be false for two structurally
    // identical atoms. Boxing an `int` through `Objects.equals` still compares
    // by value, so one form covers both.
    fn value_eq(&self, l: &str, r: &str) -> String {
        format!("java.util.Objects.equals({l}, {r})")
    }
    fn value_neq(&self, l: &str, r: &str) -> String {
        format!("!java.util.Objects.equals({l}, {r})")
    }
    // A sealed interface permitting records: the case is an `instanceof`.
    fn is_variant(&self, subject: &str, variant: &str) -> String {
        format!("{subject} instanceof {variant}")
    }
    // Java has no extension methods: the value arrives as a leading parameter.
    fn receiver_expr(&self) -> &str { "self" }
    fn neq_op(&self) -> &str { "!=" }
    fn field_access(&self, base: &str, field: &str, mutable_owner: bool) -> String {
        if mutable_owner {
            // A sig with a `var` field is emitted as a mutable class, whose
            // fields are plain — `.field()` names no method there.
            format!("{base}.{field}")
        } else {
            format!("{base}.{field}()")
        }
    }
}

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = JvmContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "Models.java".to_string(),
        content: generate_models(ir, &ctx),
    });

    let has_tc = ir_uses_tc(ir);

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
        // Intersection type → interface Foo extends A, B, C
        if !s.intersection_of.is_empty() {
            let parents = s.intersection_of.join(", ");
            writeln!(out, "public interface {} extends {} {{}}", s.name, parents).unwrap();
            writeln!(out).unwrap();
            continue;
        }
        if ctx.is_variant(&s.name) { continue; }
        if is_native_type_alias(&s.name) { continue; }

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

    // Derived fields: receiver functions → static methods taking self
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
        writeln!(out, "class {sig_name}Derived {{").unwrap();
        for op in ops {
            let params_with_self = {
                let mut ps = vec![format!("{sig_name} self")];
                for p in &op.params {
                    let type_str = java_return_type(&p.type_name, &p.mult);
                    ps.push(format!("{type_str} {}", p.name));
                }
                ps.join(", ")
            };

            let return_str = match &op.return_type {
                Some(rt) => java_return_type(&rt.type_name, &rt.mult),
                None => "boolean".to_string(),
            };

            writeln!(out, "    static {return_str} {}({params_with_self}) {{", op.name).unwrap();
            {
                let lang = JavaLang;
                let env = crate::backend::type_env::operation_env(op);
                if op.body.is_empty() {
                    writeln!(out, "        return true;").unwrap();
                } else {
                    let body_expr = &op.body[op.body.len() - 1];
                    let body = expr_translator::translate_with_env(body_expr, ir, &lang, &env);
                    writeln!(out, "        return {body};").unwrap();
                }
            }
            writeln!(out, "    }}").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

fn generate_record(out: &mut String, s: &StructureNode, ir: &OxidtrIR, disj_fields: &[(String, String)]) {
    // Singleton: one sig → Java enum with INSTANCE
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        if s.is_var {
            writeln!(out, "/* @alloy: var sig */").unwrap();
        }
        writeln!(out, "enum {} {{", s.name).unwrap();
        writeln!(out, "    INSTANCE").unwrap();
        writeln!(out, "}}").unwrap();
        return;
    }

    if s.is_var {
        writeln!(out, "/* @alloy: var sig */").unwrap();
    }
    if s.fields.is_empty() {
        writeln!(out, "record {}() {{}}", s.name).unwrap();
    } else {
        // Java records are immutable (fields are final). If any field is var
        // (mutable across state transitions), generate a class instead.
        let has_var_field = s.fields.iter().any(|f| f.is_var);

        // Collect field annotations (shared between record and class generation)
        let field_infos: Vec<(String, String, bool)> = s.fields.iter()
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
                let resolved_target = resolve_type(TargetLang::Java, &f.target);
                let java_type = if let Some(vt) = &f.value_type {
                    let resolved_vt = resolve_type(TargetLang::Java, vt);
                    format!("Map<{}, {}>", resolved_target, resolved_vt)
                } else if let Some(_raw) = &f.raw_union_type {
                    // Union types → Object
                    match f.mult {
                        Multiplicity::Lone => "@Nullable Object".to_string(),
                        Multiplicity::Set  => "List<Object>".to_string(),
                        Multiplicity::Seq  => "List<Object>".to_string(),
                        Multiplicity::One  => "Object".to_string(),
                    }
                } else {
                    mult_to_java_type(&resolved_target, &f.mult)
                };
                (annotation_str, java_type, f.is_var)
            })
            .collect();

        // FieldOrdering → constructor/compact-constructor validation
        let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
        let mut constructor_checks: Vec<String> = Vec::new();
        for c in &sig_constraints {
            match c {
                analyze::ConstraintInfo::FieldOrdering { left_field, op, right_field, .. } => {
                    let (op_str, negated) = match op {
                        CompareOp::Lt => ("<", ">="),
                        CompareOp::Gt => (">", "<="),
                        CompareOp::Lte => ("<=", ">"),
                        CompareOp::Gte => (">=", "<"),
                        _ => continue,
                    };
                    constructor_checks.push(format!("if ({left_field} {negated} {right_field}) throw new IllegalArgumentException(\"{left_field} must be {op_str} {right_field}\");"));
                }
                analyze::ConstraintInfo::NoSelfRef { field_name, .. } => {
                    constructor_checks.push(format!("if ({field_name} == this) throw new IllegalArgumentException(\"{field_name} must not reference self\");"));
                }
                analyze::ConstraintInfo::Acyclic { field_name, .. } => {
                    constructor_checks.push(format!("{{ var seen = new java.util.HashSet<>(); var cur = ({sig_name}) this; while (cur != null) {{ if (!seen.add(cur)) throw new IllegalArgumentException(\"{field_name} must not form a cycle\"); cur = cur.{field_name}(); }} }}", sig_name = s.name));
                }
                analyze::ConstraintInfo::Implication { condition, consequent, .. } => {
                    let cond = translate_validator_expr_java(condition, &s.name);
                    let cons = translate_validator_expr_java(consequent, &s.name);
                    let desc = format!("{} implies {}", analyze::describe_expr(condition), analyze::describe_expr(consequent));
                    constructor_checks.push(format!("if ({cond} && !({cons})) throw new IllegalArgumentException(\"{}\");", desc.replace('"', "\\\"")));
                }
                analyze::ConstraintInfo::Iff { left, right, .. } => {
                    let l = translate_validator_expr_java(left, &s.name);
                    let r = translate_validator_expr_java(right, &s.name);
                    let desc = format!("{} iff {}", analyze::describe_expr(left), analyze::describe_expr(right));
                    constructor_checks.push(format!("if (({l}) != ({r})) throw new IllegalArgumentException(\"{}\");", desc.replace('"', "\\\"")));
                }
                analyze::ConstraintInfo::Prohibition { condition, .. } => {
                    let cond = translate_validator_expr_java(condition, &s.name);
                    let desc = analyze::describe_expr(condition);
                    constructor_checks.push(format!("if ({cond}) throw new IllegalArgumentException(\"prohibited: {}\");", desc.replace('"', "\\\"")));
                }
                analyze::ConstraintInfo::Disjoint { left, right, .. } => {
                    let left_field = left.rsplit('.').next().unwrap_or(left);
                    let right_field = right.rsplit('.').next().unwrap_or(right);
                    constructor_checks.push(format!("if ({left_field}.stream().anyMatch({right_field}::contains)) throw new IllegalArgumentException(\"{left_field} and {right_field} must not overlap (disjoint constraint)\");"));
                }
                analyze::ConstraintInfo::Exhaustive { categories, .. } => {
                    let cats = categories.join(", ");
                    let checks: Vec<String> = categories.iter().map(|cat| {
                        let parts: Vec<&str> = cat.split('.').collect();
                        if parts.len() == 2 {
                            format!("!{}.{}.contains(this)", parts[0], parts[1])
                        } else {
                            format!("!{cat}.contains(this)")
                        }
                    }).collect();
                    let condition = checks.join(" && ");
                    constructor_checks.push(format!("if ({condition}) throw new IllegalArgumentException(\"must belong to one of [{cats}] (exhaustive constraint)\");"));
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
                        constructor_checks.push(format!("if (new java.util.HashSet<>({dfield}).size() != {dfield}.size()) throw new IllegalArgumentException(\"{dfield} must not contain duplicates (disj constraint)\");"));
                    }
                }
            }
        }

        if has_var_field {
            // Generate a mutable class instead of record for var fields
            writeln!(out, "/* var fields require mutable state — using class instead of record */").unwrap();
            writeln!(out, "class {} {{", s.name).unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let (ref ann, ref jtype, is_var) = field_infos[i];
                let final_kw = if is_var { "/* MUTABLE: changes across state transitions */ " } else { "final " };
                writeln!(out, "    {ann}{final_kw}{jtype} {};", f.name).unwrap();
            }
            // Generate constructor
            let ctor_params: Vec<String> = s.fields.iter().enumerate()
                .map(|(i, f)| {
                    let (_, ref jtype, _) = field_infos[i];
                    format!("{jtype} {}", f.name)
                })
                .collect();
            writeln!(out, "    {}({}) {{", s.name, ctor_params.join(", ")).unwrap();
            for check in &constructor_checks {
                writeln!(out, "        {check}").unwrap();
            }
            for f in &s.fields {
                writeln!(out, "        this.{} = {};", f.name, f.name).unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
        } else {
            // Standard record generation (all fields immutable)
            let params: Vec<String> = s.fields.iter().enumerate()
                .map(|(i, f)| {
                    let (ref ann, ref jtype, _) = field_infos[i];
                    format!("{ann}{jtype} {}", f.name)
                })
                .collect();
            if constructor_checks.is_empty() {
                writeln!(out, "record {}({}) {{}}", s.name, params.join(", ")).unwrap();
            } else {
                writeln!(out, "record {}({}) {{", s.name, params.join(", ")).unwrap();
                writeln!(out, "    {} {{", s.name).unwrap();
                for check in &constructor_checks {
                    writeln!(out, "        {check}").unwrap();
                }
                writeln!(out, "    }}").unwrap();
                writeln!(out, "}}").unwrap();
            }
        }
    }
}


fn generate_sealed_interface(out: &mut String, s: &StructureNode, ctx: &JvmContext) {
    let variants = ctx.children.get(&s.name);

    // Parent abstract sig may have fields that should be inherited by all variants
    let parent_fields = &s.fields;

    let all_unit = parent_fields.is_empty() && variants.map_or(true, |vs| {
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
                let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
                // Combine parent fields + child fields
                let all_fields: Vec<&IRField> = parent_fields.iter().chain(child_fields.iter().copied()).collect();
                if !all_fields.is_empty() {
                    let params: Vec<String> = all_fields.iter()
                        .map(|f| {
                            let resolved = resolve_type(TargetLang::Java, &f.target);
                            let t = if let Some(vt) = &f.value_type {
                                let rv = resolve_type(TargetLang::Java, vt);
                                format!("Map<{}, {}>", resolved, rv)
                            } else if let Some(_raw) = &f.raw_union_type {
                                "Object".to_string()
                            } else {
                                mult_to_java_type(&resolved, &f.mult)
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

/// Generate Helpers.java containing TC / RTC functions.
fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "import java.util.List;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "class Helpers {{").unwrap();

    let (tc_fields, rtc_fields) = collect_closure_fields(ir);

    for tc in &tc_fields {
        generate_tc_function(&mut out, tc);
    }
    for rtc in &rtc_fields {
        generate_rtc_function(&mut out, rtc);
    }

    writeln!(out, "}}").unwrap();
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

fn generate_rtc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("rtc{}", capitalize(&tc.field_name));
    let tc_name = format!("tc{}", capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "    /** Reflexive-transitive closure for {sig}.{field} (id ∪ ^{field}). */").unwrap();
    writeln!(out, "    static List<{sig}> {fn_name}({sig} start) {{").unwrap();
    writeln!(out, "        var result = new java.util.ArrayList<{sig}>();").unwrap();
    writeln!(out, "        result.add(start);").unwrap();
    writeln!(out, "        result.addAll({tc_name}(start));").unwrap();
    writeln!(out, "        return result;").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

// ── Operations.java ────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();

    writeln!(out, "class Operations {{").unwrap();

    for op in &ir.operations {
        if op.receiver_sig.is_some() {
            continue;
        }
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
            // An Alloy `pred` is a formula, not a procedure (#82).
            None => "boolean".to_string(),
        };

        writeln!(out, "    static {} {}({params}) {{", return_type, op.name).unwrap();
        {
            let lang = JavaLang;
            let env = crate::backend::type_env::operation_env(op);
            if op.body.is_empty() {
                writeln!(out, "        return true;").unwrap();
            } else if op.return_type.is_some() {
                let body = expr_translator::translate_with_env(&op.body[0], ir, &lang, &env);
                writeln!(out, "        return {body};").unwrap();
            } else {
                let conjuncts: Vec<String> = op.body.iter()
                    .map(|e| expr_translator::translate_with_env(e, ir, &lang, &env))
                    .collect();
                writeln!(out, "        return {};", conjuncts.join(" && ")).unwrap();
            }
        }
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── Tests.java ─────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let fixture_types = crate::backend::collect_fixture_types(ir);
    let sig_names = expr_translator::collect_sig_names(ir);
    let lang = JavaLang;

    writeln!(out, "import org.junit.jupiter.api.Test;").unwrap();
    writeln!(out, "import org.junit.jupiter.api.Disabled;").unwrap();
    writeln!(out, "import static org.junit.jupiter.api.Assertions.*;").unwrap();
    writeln!(out, "import java.util.List;").unwrap();

    writeln!(out).unwrap();
    writeln!(out, "class PropertyTests {{").unwrap();

    for prop in &ir.properties {
        let params = expr_translator::extract_params(&prop.expr, &sig_names, ir);
        let body = expr_translator::translate_with_ir(&prop.expr, ir, &lang);

        // An `assert` carries temporal operators just as a `fact` does, and
        // translating its operand alone silently drops them (#78).
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
            writeln!(out, "    void {}() {{", prop.name).unwrap();
            writeln!(out, "        // {label}: full verification needs a trace; an empty trace").unwrap();
            writeln!(out, "        // can never satisfy it, which at least exercises the checker.").unwrap();
            match temporal_checker_name(&prop.name, &prop.expr, temporal_kind) {
                Some(checker) => {
                    let tname = params.first().map(|(_, t)| t.as_str()).unwrap_or("Object");
                    writeln!(out, "        List<List<{tname}>> trace = List.of();").unwrap();
                    writeln!(out, "        assertFalse({checker}(trace));").unwrap();
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
        writeln!(out, "    void {}() {{", prop.name).unwrap();
        for (pname, tname) in &params {
            // An empty domain makes `allMatch` vacuously true, so the test
            // passes whatever the implementation does (#81). Seed it from the
            // fixture wherever one exists, and disclose it where one does not.
            if fixture_types.contains(tname) {
                // Java fixtures are static methods on `Fixtures`, unlike
                // Kotlin's and Swift's top-level functions — an unqualified
                // call does not resolve from inside `PropertyTests`.
                writeln!(out, "        List<{tname}> {pname} = List.of(Fixtures.default{tname}());").unwrap();
            } else {
                writeln!(out, "        // @coverage empty domain: no fixture for `{tname}`;").unwrap();
                writeln!(out, "        // this quantifier is vacuously satisfied.").unwrap();
                writeln!(out, "        List<{tname}> {pname} = List.of();").unwrap();
            }
        }
        writeln!(out, "        assertTrue({body});").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Invariant tests — inline constraint expressions
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name { Some(n) => n.clone(), None => continue };

        // Alloy 6: temporal facts with prime → generate transition test
        if analyze::expr_contains_prime(&constraint.expr) {
            let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "    /** @temporal Transition constraint: {fact_name} */").unwrap();
            writeln!(out, "    /** Verifies: pre→post state relationship ({desc}) */").unwrap();
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    void transition_{}() {{", fact_name).unwrap();
            for (pname, tname) in &params {
                writeln!(out, "        var {pname} = List.<{tname}>of();").unwrap();
                writeln!(out, "        var next_{pname} = new java.util.ArrayList<>({pname});").unwrap();
            }
            if let Some((_kind, bindings, inner_body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                let rewritten_body = analyze::rewrite_prime_as_post_state(inner_body);
                let body_str = expr_translator::translate_with_ir(&rewritten_body, ir, &lang);
                let bind_vars: Vec<String> = bindings.iter()
                    .flat_map(|b| b.vars.clone()).collect();
                if bind_vars.len() == 1 {
                    let v = &bind_vars[0];
                    let pname = &params[0].0;
                    writeln!(out, "        for (int i = 0; i < {pname}.size(); i++) {{").unwrap();
                    writeln!(out, "            var {v} = {pname}.get(i);").unwrap();
                    writeln!(out, "            var next_{v} = next_{pname}.get(i);").unwrap();
                    writeln!(out, "            assertTrue({body_str});").unwrap();
                    writeln!(out, "        }}").unwrap();
                } else {
                    writeln!(out, "        assertTrue({body_str});").unwrap();
                }
            } else {
                let rewritten = analyze::rewrite_prime_as_post_state(&constraint.expr);
                let body = expr_translator::translate_with_ir(&rewritten, ir, &lang);
                writeln!(out, "        assertTrue({body});").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
        let body = expr_translator::translate_with_ir(&constraint.expr, ir, &lang);

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
            writeln!(out, "    void {}_{}() {{", test_prefix, fact_name).unwrap();
            writeln!(out, "        // binary temporal: requires trace-based verification; see check{op_label}{fact_name}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else if matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            let kind_label = if temporal_kind == Some(analyze::TemporalKind::Liveness) {
                "Liveness" } else { "PastLiveness" };
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    void {}_{}() {{", test_prefix, fact_name).unwrap();
            writeln!(out, "        // {}: requires trace-based verification; see check{kind_label}{fact_name}", test_prefix).unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else {
        writeln!(out, "    @Test").unwrap();
        writeln!(out, "    void {}_{}() {{", test_prefix, fact_name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        List<{tname}> {pname} = List.of();").unwrap();
        }
        writeln!(out, "        assertTrue({body});").unwrap();
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

        let has_fixture: HashSet<String> = ir.structures.iter()
            .filter(|s| !s.is_enum && !s.fields.is_empty())
            .map(|s| s.name.clone())
            .collect();

        for (sig_name, patterns) in &anomaly_sigs {
            if !has_fixture.contains(sig_name) { continue; }
            for pattern in patterns {
                match pattern {
                    analyze::AnomalyPattern::UnconstrainedField { field_name, .. } => {
                        let camel = to_camel_case(field_name);
                        writeln!(out, "    @Test").unwrap();
                        writeln!(out, "    void anomaly_{sig_name}_{field_name}_unconstrained() {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.default{sig_name}();").unwrap();
                        writeln!(out, "        {} // unconstrained field access",
                            java_field_touch(ir, sig_name, &camel)).unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        let camel = to_camel_case(field_name);
                        writeln!(out, "    @Test").unwrap();
                        writeln!(out, "    void anomaly_{sig_name}_{field_name}_empty() {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.anomalyEmpty{sig_name}();").unwrap();
                        writeln!(out, "        {} // empty edge case",
                            java_field_touch(ir, sig_name, &camel)).unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        let _camel = to_camel_case(field_name);
                        writeln!(out, "    @Test").unwrap();
                        writeln!(out, "    void anomaly_{sig_name}_{field_name}_selfRef() {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.default{sig_name}();").unwrap();
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

        let has_fixture: HashSet<String> = ir.structures.iter()
            .filter(|s| !s.is_enum && !s.fields.is_empty())
            .map(|s| s.name.clone())
            .collect();

        let mut seen_cover: HashSet<String> = HashSet::new();
        for pair in &coverage.pairwise {
            if !has_fixture.contains(&pair.sig_name) { continue; }
            let snake_a = to_snake_case(&pair.fact_a);
            let snake_b = to_snake_case(&pair.fact_b);
            let test_name = format!("cover_{snake_a}_x_{snake_b}");
            if !seen_cover.insert(test_name.clone()) { continue; }
            let camel = to_camel_case(&pair.sig_name);

            let constraint_a = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_a));
            let constraint_b = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_b));

            let body_a = constraint_a
                .map(|c| expr_translator::translate_with_ir(&c.expr, ir, &lang));
            let body_b = constraint_b
                .map(|c| expr_translator::translate_with_ir(&c.expr, ir, &lang));

            // Collect params from both constraint expressions
            let mut params: Vec<(String, String)> = Vec::new();
            let mut param_names: HashSet<String> = HashSet::new();
            for constraint in [constraint_a, constraint_b].into_iter().flatten() {
                for (pname, tname) in expr_translator::extract_params(&constraint.expr, &sig_names, ir) {
                    if param_names.insert(pname.clone()) {
                        params.push((pname, tname));
                    }
                }
            }

            writeln!(out, "    @Disabled").unwrap();
            writeln!(out, "    @Test").unwrap();
            writeln!(out, "    void {test_name}() {{").unwrap();
            for (pname, tname) in &params {
                writeln!(out, "        List<{tname}> {pname} = List.of(Fixtures.default{tname}());").unwrap();
            }
            if params.is_empty() {
                writeln!(out, "        var {camel}s = List.of(Fixtures.default{}());", pair.sig_name).unwrap();
            }
            if let (Some(a), Some(b)) = (&body_a, &body_b) {
                writeln!(out, "        assertTrue({a});").unwrap();
                writeln!(out, "        assertTrue({b});").unwrap();
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

/// Translate an Alloy expression to Java for single-instance validator context.
/// Java records use `.field()` accessor methods.
fn translate_validator_expr_java(expr: &crate::parser::ast::Expr, sig_name: &str) -> String {
    use crate::parser::ast::{Expr, LogicOp, QuantKind};
    match expr {
        Expr::VarRef(name) => {
            if name == sig_name { "this".to_string() } else { name.clone() }
        }
        Expr::IntLiteral(n) => n.to_string(),
        Expr::FieldAccess { base, field } => {
            format!("{}.{}()", translate_validator_expr_java(base, sig_name), field)
        }
        Expr::Comparison { op, left, right } => {
            let l = translate_validator_expr_java(left, sig_name);
            let r = translate_validator_expr_java(right, sig_name);
            let o = match op {
                CompareOp::Eq => return format!("{l}.equals({r})"),
                CompareOp::NotEq => return format!("!{l}.equals({r})"),
                CompareOp::In => return format!("{r}.contains({l})"),
                CompareOp::Lt => "<",
                CompareOp::Gt => ">",
                CompareOp::Lte => "<=",
                CompareOp::Gte => ">=",
            };
            format!("{l} {o} {r}")
        }
        Expr::BinaryLogic { op, left, right } => {
            let l = translate_validator_expr_java(left, sig_name);
            let r = translate_validator_expr_java(right, sig_name);
            match op {
                LogicOp::And => format!("{l} && {r}"),
                LogicOp::Or => format!("{l} || {r}"),
                LogicOp::Implies => format!("!({l}) || {r}"),
                LogicOp::Iff => format!("({l}) == ({r})"),
            }
        }
        Expr::Not(inner) => format!("!({})", translate_validator_expr_java(inner, sig_name)),
        Expr::MultFormula { kind, expr: inner } => {
            let e = translate_validator_expr_java(inner, sig_name);
            match kind {
                QuantKind::Some => format!("{e} != null"),
                QuantKind::No => format!("{e} == null"),
                _ => e,
            }
        }
        Expr::Cardinality(inner) => {
            format!("{}.size()", translate_validator_expr_java(inner, sig_name))
        }
        _ => analyze::describe_expr(expr), // fallback: human-readable
    }
}

// ── Fixtures.java ──────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &JvmContext) -> String {
    let mut out = String::new();

    writeln!(out, "import java.util.List;").unwrap();
    writeln!(out, "import java.util.Set;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "class Fixtures {{").unwrap();

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
            writeln!(out, "    /** Factory: default value for {} */", s.name).unwrap();
            writeln!(out, "    static {} default{}() {{", s.name, s.name).unwrap();
            if ctx.enum_is_flat(s) {
                // Java enum → qualified access. Any variant will do; none of
                // them carries a field.
                let Some(variant) = variants.first() else { continue };
                writeln!(out, "        return {}.{};", s.name, variant).unwrap();
            } else {
                // Sealed interface → the case whose payload was satisfiable
                // when the enum was admitted; any other may lead straight back
                // into it.
                let Some(variant) = enum_witness.get(&s.name) else { continue };
                let args: Vec<String> = ctx.variant_fields(s, variant).iter()
                    .map(|f| if f.value_type.is_some() {
                        "Map.of()".to_string()
                    } else {
                        java_default_value(&f.target, &f.mult)
                    })
                    .collect();
                writeln!(out, "        return new {}({});", variant, args.join(", ")).unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum { continue; }
        // A field-less sig is still a value, and a fixture with a field of that
        // type calls the factory whether or not one was emitted (#105).
        if !terminating.contains(&s.name) { continue; }
        if s.fields.is_empty() {
            // `one sig` is an enum holding INSTANCE; anything else a record.
            let value = if s.sig_multiplicity == SigMultiplicity::One {
                format!("{}.INSTANCE", s.name)
            } else {
                format!("new {}()", s.name)
            };
            writeln!(out, "    /** Factory: create a default valid {} */", s.name).unwrap();
            writeln!(out, "    static {} default{}() {{", s.name, s.name).unwrap();
            writeln!(out, "        return {value};").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

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

            writeln!(out, "    /** Anomaly fixture: all collections empty */").unwrap();
            writeln!(out, "    public static {} anomalyEmpty{}() {{", sig_name, sig_name).unwrap();
            writeln!(out, "        return new {}(", sig_name).unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = match &f.mult {
                    Multiplicity::Set => "Set.of()".to_string(),
                    Multiplicity::Seq => "List.of()".to_string(),
                    _ => java_default_value(&f.target, &f.mult),
                };
                writeln!(out, "            {}{}", val, comma).unwrap();
            }
            writeln!(out, "        );").unwrap();
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
    // `Int`/`Str`/`Bool` are Alloy marker sigs, not emitted types.
    let type_name = &if crate::backend::is_native_type_alias(type_name) {
        crate::backend::resolve_type(crate::backend::TargetLang::Java, type_name)
    } else {
        type_name.to_string()
    };
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

/// The zero value for an Alloy marker sig, which has no generated factory:
/// `defaultInt()` names no method, so every fixture holding such a field failed
/// to compile.
/// A statement that reads a field of `sig`, for an anomaly test whose point is
/// simply to touch it.
///
/// A sig with a `var` field is emitted as a mutable class with plain fields, so
/// the record accessor does not exist — and a bare field read is not a
/// statement in Java the way a method call is, so it has to be assigned.
fn java_field_touch(ir: &OxidtrIR, sig: &str, field: &str) -> String {
    let mutable = ir.structures.iter()
        .any(|s| s.name == sig && s.fields.iter().any(|f| f.is_var));
    if mutable {
        format!("var touched_{field} = instance.{field};")
    } else {
        format!("instance.{field}();")
    }
}

fn java_native_zero(target: &str) -> Option<&'static str> {
    match target {
        "Int" => Some("0L"),
        "Str" => Some("\"\""),
        "Bool" => Some("false"),
        "Float" => Some("0.0"),
        _ => None,
    }
}

fn java_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    if let (Multiplicity::One, Some(zero)) = (mult, java_native_zero(target)) {
        return zero.to_string();
    }
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

fn to_camel_case(s: &str) -> String {
    let mut out = String::new();
    let mut cap_next = false;
    for (i, c) in s.chars().enumerate() {
        if c == '_' {
            cap_next = true;
        } else if cap_next {
            out.push(c.to_uppercase().next().unwrap());
            cap_next = false;
        } else if i == 0 {
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
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
    let lang = JavaLang;
    let _ = (&constraint, &lang, body);
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
                    writeln!(out, "    boolean check{kind_label}{name}(List<List<{tname}>> trace) {{").unwrap();
                    writeln!(out, "        return trace.stream().anyMatch({pname} -> {body});").unwrap();
                } else {
                    writeln!(out, "    boolean check{kind_label}{name}(List<List<Object>> trace) {{").unwrap();
                    writeln!(out, "        return trace.stream().anyMatch(entry -> {{").unwrap();
                    for (i, (pname, tname)) in params.iter().enumerate() {
                        writeln!(out, "            @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) entry.get({i});").unwrap();
                    }
                    writeln!(out, "            return {body};").unwrap();
                    writeln!(out, "        }});").unwrap();
                }
                writeln!(out, "    }}").unwrap();
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
                        writeln!(out, "    boolean check{op_name}{name}(List<List<{tname}>> trace) {{").unwrap();
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "        int pos = java.util.stream.IntStream.range(0, trace.size())").unwrap();
                                writeln!(out, "            .filter(i -> {{ List<{tname}> {pname} = trace.get(i); return {right_body}; }})").unwrap();
                                writeln!(out, "            .findFirst().orElse(-1);").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(0, pos).stream().allMatch({pname} -> {left_body});").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "        int pos = -1;").unwrap();
                                writeln!(out, "        for (int i = trace.size() - 1; i >= 0; i--) {{ List<{tname}> {pname} = trace.get(i); if ({right_body}) {{ pos = i; break; }} }}").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(pos, trace.size()).stream().allMatch({pname} -> {left_body});").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "        int pos = java.util.stream.IntStream.range(0, trace.size())").unwrap();
                                writeln!(out, "            .filter(i -> {{ List<{tname}> {pname} = trace.get(i); return {left_body}; }})").unwrap();
                                writeln!(out, "            .findFirst().orElse(-1);").unwrap();
                                writeln!(out, "        return pos >= 0 ? trace.subList(0, pos + 1).stream().allMatch({pname} -> {right_body}) : trace.stream().allMatch({pname} -> {right_body});").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "        return java.util.stream.IntStream.range(0, trace.size()).allMatch(i -> {{").unwrap();
                                writeln!(out, "            List<{tname}> {pname} = trace.get(i);").unwrap();
                                writeln!(out, "            if ({right_body}) {{ return trace.subList(0, i + 1).stream().anyMatch({pname}2 -> {{ List<{tname}> {pname} = {pname}2; return {left_body}; }}); }} else {{ return true; }}").unwrap();
                                writeln!(out, "        }});").unwrap();
                            }
                        }
                    } else {
                        writeln!(out, "    boolean check{op_name}{name}(List<List<Object>> trace) {{").unwrap();
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "        int pos = java.util.stream.IntStream.range(0, trace.size())").unwrap();
                                writeln!(out, "            .filter(i -> {{").unwrap();
                                for (i, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "                @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) trace.get(i).get({i});").unwrap();
                                }
                                writeln!(out, "                return {right_body};").unwrap();
                                writeln!(out, "            }}).findFirst().orElse(-1);").unwrap();
                                writeln!(out, "        final int p = pos;").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(0, p).stream().allMatch(entry -> {{").unwrap();
                                for (i, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "            @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) entry.get({i});").unwrap();
                                }
                                writeln!(out, "            return {left_body};").unwrap();
                                writeln!(out, "        }});").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "        int pos = -1;").unwrap();
                                writeln!(out, "        for (int i = trace.size() - 1; i >= 0; i--) {{").unwrap();
                                for (i, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "            @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) trace.get(i).get({i});").unwrap();
                                }
                                writeln!(out, "            if ({right_body}) {{ pos = i; break; }}").unwrap();
                                writeln!(out, "        }}").unwrap();
                                writeln!(out, "        final int p = pos;").unwrap();
                                writeln!(out, "        return pos >= 0 && trace.subList(p, trace.size()).stream().allMatch(entry -> {{").unwrap();
                                for (i, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "            @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) entry.get({i});").unwrap();
                                }
                                writeln!(out, "            return {left_body};").unwrap();
                                writeln!(out, "        }});").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "        int pos = java.util.stream.IntStream.range(0, trace.size())").unwrap();
                                writeln!(out, "            .filter(i -> {{").unwrap();
                                for (i, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "                @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) trace.get(i).get({i});").unwrap();
                                }
                                writeln!(out, "                return {left_body};").unwrap();
                                writeln!(out, "            }}).findFirst().orElse(-1);").unwrap();
                                writeln!(out, "        final int p = pos;").unwrap();
                                writeln!(out, "        if (pos >= 0) {{").unwrap();
                                writeln!(out, "            return trace.subList(0, p + 1).stream().allMatch(entry -> {{").unwrap();
                                for (i, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "                @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) entry.get({i});").unwrap();
                                }
                                writeln!(out, "                return {right_body};").unwrap();
                                writeln!(out, "            }});").unwrap();
                                writeln!(out, "        }} else {{").unwrap();
                                writeln!(out, "            return trace.stream().allMatch(entry -> {{").unwrap();
                                for (i, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "                @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) entry.get({i});").unwrap();
                                }
                                writeln!(out, "                return {right_body};").unwrap();
                                writeln!(out, "            }});").unwrap();
                                writeln!(out, "        }}").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "        return java.util.stream.IntStream.range(0, trace.size()).allMatch(i -> {{").unwrap();
                                for (idx, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "            @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) trace.get(i).get({idx});").unwrap();
                                }
                                writeln!(out, "            if ({right_body}) {{").unwrap();
                                writeln!(out, "                return trace.subList(0, i + 1).stream().anyMatch(entry -> {{").unwrap();
                                for (idx, (pname, tname)) in params.iter().enumerate() {
                                    writeln!(out, "                    @SuppressWarnings(\"unchecked\") List<{tname}> {pname} = (List<{tname}>) entry.get({idx});").unwrap();
                                }
                                writeln!(out, "                    return {left_body};").unwrap();
                                writeln!(out, "                }});").unwrap();
                                writeln!(out, "            }} else {{ return true; }}").unwrap();
                                writeln!(out, "        }});").unwrap();
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

/// The name `emit_temporal_trace_checkers` will give this constraint's checker,
/// so the generated test can call it rather than leaving it unreferenced.
fn temporal_checker_name(
    name: &str,
    expr: &crate::parser::ast::Expr,
    kind: Option<analyze::TemporalKind>,
) -> Option<String> {
    match kind {
        Some(analyze::TemporalKind::Binary) => {
            let (op, _, _, _) = analyze::find_temporal_binary_with_bindings(expr)?;
            let op_name = match op {
                TemporalBinaryOp::Until => "Until",
                TemporalBinaryOp::Since => "Since",
                TemporalBinaryOp::Release => "Release",
                TemporalBinaryOp::Triggered => "Triggered",
            };
            Some(format!("check{op_name}{name}"))
        }
        Some(analyze::TemporalKind::PastLiveness) => Some(format!("checkPastLiveness{name}")),
        Some(analyze::TemporalKind::Liveness) => Some(format!("checkLiveness{name}")),
        _ => None,
    }
}
