pub mod expr_translator;

use crate::backend::{self, GeneratedFile, TargetLang, is_native_type_alias, resolve_type};
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity};
use crate::analyze;
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

/// The suffix a fixture factory's name carries: the sig's name, title-cased.
///
/// The type half of the signature goes through `cs_ident`, which wraps a
/// keyword in `@`. The method half must not — `@` is only valid where the
/// identifier *is* the keyword — but it was taking the raw name, so `sig lock`
/// produced `Defaultlock` beside every other sig's `DefaultFoo` (#107).
fn cs_factory_suffix(name: &str) -> String {
    expr_translator::capitalize(name)
}

// ── Models.cs ────────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR, ctx: &CsContext) -> String {
    let mut out = String::new();
    writeln!(out, "using System;").unwrap();
    writeln!(out, "using System.Collections.Generic;").unwrap();
    // The validators emitted below write `.Any(` and `.Distinct()`, which are
    // `System.Linq` extension methods — CS1061 without this (#111).
    writeln!(out, "using System.Linq;").unwrap();
    writeln!(out).unwrap();

    // Alloy quantifies over relations of any multiplicity, but `one`/`lone`
    // fields lower to a bare `T` / `T?`, which carry no `TrueForAll`. Lifting
    // them to a one- or zero-element list keeps one quantifier rendering for
    // every multiplicity — the same trick Go uses with `oneOf`/`loneOf`.
    writeln!(out, "public static class Rel").unwrap();
    writeln!(out, "{{").unwrap();
    writeln!(out, "    public static List<T> OneOf<T>(T v) => new List<T> {{ v }};").unwrap();
    writeln!(out, "    public static List<T> LoneOf<T>(T v) where T : class =>").unwrap();
    writeln!(out, "        v == null ? new List<T>() : new List<T> {{ v }};").unwrap();
    writeln!(out, "    public static List<T> LoneOf<T>(T? v) where T : struct =>").unwrap();
    writeln!(out, "        v.HasValue ? new List<T> {{ v.Value }} : new List<T>();").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    for s in &ir.structures {
        if ctx.is_variant(&s.name) { continue; }
        if is_native_type_alias(&s.name) { continue; }

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
    let mut by_sig: std::collections::BTreeMap<String, Vec<&OperationNode>> =
        std::collections::BTreeMap::new();
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
                None => "bool".to_string(),
            };
            let env = crate::backend::type_env::operation_env(op);
            let body_str = if op.body.is_empty() {
                "true".to_string()
            } else if op.return_type.is_some() {
                expr_translator::translate_with_env(&op.body[0], ir, &env)
            } else {
                op.body.iter()
                    .map(|e| expr_translator::translate_with_env(e, ir, &env))
                    .collect::<Vec<_>>()
                    .join(" && ")
            };

            if op.params.is_empty() {
                // C# has no extension properties, so a no-parameter derived
                // field is an extension *method*. It used to be emitted as
                // `public static T Name => ..`, a static property with no
                // receiver at all — nothing could call it on an instance.
                writeln!(out, "    public static {return_type} {}(this {} self) => {body_str};",
                    capitalize(&op.name), cs_ident(sig_name)).unwrap();
            } else {
                let params = op.params.iter().map(|p| {
                    let type_str = mult_to_cs_type(&p.type_name, &p.mult);
                    format!("{type_str} {}", to_camel_case(&p.name))
                }).collect::<Vec<_>>().join(", ");
                writeln!(out, "    public static {return_type} {}(this {} self, {params})", capitalize(&op.name), cs_ident(sig_name)).unwrap();
                writeln!(out, "    {{").unwrap();
                writeln!(out, "        return {body_str};").unwrap();
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
        writeln!(out, "public class {}", cs_ident(&s.name)).unwrap();
        writeln!(out, "{{").unwrap();
        writeln!(out, "    public static readonly {} Instance = new {}();", cs_ident(&s.name), cs_ident(&s.name)).unwrap();
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

    writeln!(out, "public class {}", cs_ident(&s.name)).unwrap();
    writeln!(out, "{{").unwrap();
    for f in &s.fields {
        if f.mult == Multiplicity::Seq {
            writeln!(out, "    // @alloy: seq").unwrap();
        }
        let type_str = mult_to_cs_type(&f.target, &f.mult);
        writeln!(out, "    public {} {} {{ get; set; }}", type_str, expr_translator::cs_property_name(&s.name, &f.name)).unwrap();
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
        writeln!(out, "public enum {}", cs_ident(&s.name)).unwrap();
        writeln!(out, "{{").unwrap();
        if let Some(variants) = variants {
            for v in variants {
                writeln!(out, "    {},", cs_ident(v)).unwrap();
            }
        }
        writeln!(out, "}}").unwrap();
    } else {
        writeln!(out, "public abstract class {}", cs_ident(&s.name)).unwrap();
        writeln!(out, "{{").unwrap();
        for f in parent_fields {
            let type_str = mult_to_cs_type(&f.target, &f.mult);
            writeln!(out, "    public {} {} {{ get; set; }}", type_str, expr_translator::cs_property_name(&s.name, &f.name)).unwrap();
        }
        writeln!(out, "}}").unwrap();
        if let Some(variants) = variants {
            for v in variants {
                let child = ctx.struct_map.get(v.as_str());
                let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
                writeln!(out).unwrap();
                writeln!(out, "public class {} : {}", cs_ident(v), cs_ident(&s.name)).unwrap();
                writeln!(out, "{{").unwrap();
                for f in &child_fields {
                    if f.mult == Multiplicity::Seq {
                        writeln!(out, "    // @alloy: seq").unwrap();
                    }
                    let type_str = mult_to_cs_type(&f.target, &f.mult);
                    writeln!(out, "    public {} {} {{ get; set; }}", type_str, expr_translator::cs_property_name(&s.name, &f.name)).unwrap();
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

        // An Alloy `pred` is a formula, not a procedure (#82).
        let return_type = match &op.return_type {
            Some(rt) => mult_to_cs_type(&rt.type_name, &rt.mult),
            None => "bool".to_string(),
        };

        writeln!(out, "    public static {} {}({params})", return_type, capitalize(&op.name)).unwrap();
        writeln!(out, "    {{").unwrap();
        {
            let env = crate::backend::type_env::operation_env(op);
            if op.body.is_empty() {
                writeln!(out, "        return true;").unwrap();
            } else if op.return_type.is_some() {
                let body = expr_translator::translate_with_env(&op.body[0], ir, &env);
                writeln!(out, "        return {body};").unwrap();
            } else {
                let conjuncts: Vec<String> = op.body.iter()
                    .map(|e| expr_translator::translate_with_env(e, ir, &env))
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

// ── Fixtures.cs ──────────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &CsContext) -> String {
    let mut out = String::new();
    // `NotSupportedException` lives in `System`, which Fixtures.cs did not
    // import — it had no reason to before a factory could refuse to exist.
    writeln!(out, "using System;").unwrap();
    // Which types have a finite value at all — a least fixed point, so a cycle
    // that closes through a second type is caught (#109).
    let (terminating, _witness) = backend::terminating_types(ir);
    writeln!(out, "using System.Collections.Generic;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "public static class Fixtures").unwrap();
    writeln!(out, "{{").unwrap();

    let fixture_types = backend::collect_fixture_types(ir);

    // Default/Boundary factories. Variants of an enum hierarchy (e.g. `Quantifier
    // extends Expr`) get one too whenever they carry fields — a fact can quantify
    // over the variant directly (`all q: Quantifier | ...`), and the abstract
    // parent's own default (below) needs *some* concrete factory to pick from.
    // A variant is normally not a fixture type — it is a case of its parent, not
    // a value on its own. It becomes one the moment a field is declared to hold
    // it, and a variant that inherits every field has an empty *own* field list,
    // so the `fields.is_empty()` guard skipped it and left `DefaultChild`
    // referenced but never declared (CS0103) (#93).
    let needed_variants = backend::variants_used_as_field_targets(ir);
    for s in &ir.structures {
        if s.is_enum { continue; }
        if s.fields.is_empty() && !needed_variants.contains(&s.name) { continue; }

        // Inherited fields are set through the object initialiser too — the
        // emitted class declares them on the abstract parent.
        let init_fields: Vec<(&IRField, String)> = inherited_and_own_fields(ir, s)
            .into_iter()
            .map(|(owner, f)| {
                let val = default_value_for(&f.target, &f.mult, &owner, ir, &fixture_types, ctx);
                (f, val)
            })
            .collect();

        // Every value of this type contains another: a single-step check
        // cannot see a cycle that closes through a second type (#109).
        if !terminating.contains(&s.name) {
            writeln!(out, "    /// <summary>{} has no finite default: every value of it contains another.</summary>", s.name).unwrap();
            writeln!(out, "    public static {} Default{}() =>", cs_ident(&s.name), cs_factory_suffix(&s.name)).unwrap();
            writeln!(out, "        throw new NotSupportedException(\"oxidtr: {} has no finite default: every value of it contains another\");", s.name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        // Default factory
        writeln!(out, "    public static {} Default{}()", cs_ident(&s.name), cs_factory_suffix(&s.name)).unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        return new {}", cs_ident(&s.name)).unwrap();
        writeln!(out, "        {{").unwrap();
        for (f, val) in &init_fields {
            writeln!(out, "            {} = {},", expr_translator::cs_property_name(&s.name, &f.name), val).unwrap();
        }
        writeln!(out, "        }};").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();

        // Boundary factory
        writeln!(out, "    public static {} Boundary{}()", cs_ident(&s.name), cs_factory_suffix(&s.name)).unwrap();
        writeln!(out, "    {{").unwrap();
        writeln!(out, "        return new {}", cs_ident(&s.name)).unwrap();
        writeln!(out, "        {{").unwrap();
        for f in &s.fields {
            let val = boundary_value_for(ir, &s.name, f, &fixture_types, ctx);
            writeln!(out, "            {} = {},", expr_translator::cs_property_name(&s.name, &f.name), val).unwrap();
        }
        writeln!(out, "        }};").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Enum defaults: an abstract sig lowers to `abstract class`, so its default
    // must construct a concrete variant instead. Prefer a fieldless variant
    // (`new Unit()`); failing that, prefer one that does not recurse into the
    // enum type itself (a self-referential pick, e.g. `Neg { inner: Expr }`,
    // would build forever); failing that, fall back to the first declared.
    for s in &ir.structures {
        if !s.is_enum { continue; }
        let variants = match ctx.children.get(&s.name) {
            Some(vs) if !vs.is_empty() => vs,
            // No children: the enum lowers to a variantless `public enum X {}`
            // (see `generate_enum`'s `all_unit` branch). `one_value_for` does
            // not know that — it treats any enum sig as needing a `Default*`
            // factory — so one must exist even though there is nothing to
            // pick. A variantless C# enum still has an implicit zero value,
            // so `default` is a real answer. See #102 round 3 defect 4.
            _ => {
                writeln!(out, "    public static {} Default{}() => default;", cs_ident(&s.name), cs_factory_suffix(&s.name)).unwrap();
                writeln!(out).unwrap();
                continue;
            }
        };
        // `terminates` below only rejected a *direct* self-reference, so a
        // cycle closing through a second type (`A1 { b: B }` / `B1 { a: A }`)
        // looked terminating and produced a mutually recursive factory pair
        // (#109). The shared fixed point sees the whole cycle.
        //
        // Checked here rather than before the match: a variantless enum has no
        // case to be constructible *through*, but C# gives it an implicit zero
        // value, so `default` remains a real answer for it.
        if !terminating.contains(&s.name) {
            writeln!(out, "    /// <summary>{} has no finite default: every value of it contains another.</summary>", s.name).unwrap();
            writeln!(out, "    public static {} Default{}() =>", cs_ident(&s.name), cs_factory_suffix(&s.name)).unwrap();
            writeln!(out, "        throw new NotSupportedException(\"oxidtr: {} has no finite default: every value of it contains another\");", s.name).unwrap();
            writeln!(out).unwrap();
            continue;
        }
        let effective_fields = |v: &str| -> usize {
            ctx.struct_map.get(v).map_or(0, |st| st.fields.len()) + s.fields.len()
        };
        let all_unit = variants.iter().all(|v| effective_fields(v) == 0);
        let default_expr = if all_unit {
            format!("{}.{}", cs_ident(&s.name), cs_ident(&variants[0]))
        } else if let Some(unit) = variants.iter().find(|v| effective_fields(v) == 0) {
            format!("new {}()", cs_ident(unit))
        } else {
            let terminates = |v: &str| -> bool {
                let own = ctx.struct_map.get(v).map(|st| st.fields.as_slice()).unwrap_or(&[]);
                own.iter().chain(s.fields.iter()).all(|f| f.target != s.name)
            };
            let chosen = variants.iter().find(|v| terminates(v)).unwrap_or(&variants[0]);
            let own_fields: Vec<IRField> = ctx.struct_map.get(chosen.as_str())
                .map(|st| st.fields.clone()).unwrap_or_default();
            let fields_str = own_fields.iter().chain(s.fields.iter())
                .map(|f| format!("{} = {}", capitalize(&f.name),
                    default_value_for(&f.target, &f.mult, chosen, ir, &fixture_types, ctx)))
                .collect::<Vec<_>>().join(", ");
            format!("new {} {{ {fields_str} }}", cs_ident(chosen))
        };
        writeln!(out, "    public static {} Default{}() => {};", cs_ident(&s.name), cs_factory_suffix(&s.name), default_expr).unwrap();
        writeln!(out).unwrap();
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

            // Object-initializer syntax, matching the settable-property shape
            // every generated type actually has (see Models.cs) — not the
            // positional-constructor shape these types don't have.
            writeln!(out, "    /// <summary>Anomaly fixture: all collections empty</summary>").unwrap();
            writeln!(out, "    public static {} AnomalyEmpty{}() => new {}",
                cs_ident(sig_name), cs_factory_suffix(sig_name), cs_ident(sig_name)).unwrap();
            writeln!(out, "    {{").unwrap();
            for f in &s.fields {
                let upper = capitalize(&f.name);
                let val = match &f.mult {
                    // Properties are always `List<T>` (see `mult_to_cs_type`), even
                    // for a `set` field — `HashSet<T>` here would be a type mismatch.
                    Multiplicity::Set | Multiplicity::Seq => format!("new List<{}>()", cs_type_name(&f.target)),
                    _ => cs_default_value(&f.target, &f.mult, &fixture_types, ctx),
                };
                writeln!(out, "        {upper} = {val},").unwrap();
            }
            writeln!(out, "    }};").unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

/// A `One`-multiplicity value for `target`: a native zero value for a
/// resolved Alloy alias, a `Default*()` factory call for anything that needs
/// one (abstract sigs, enum variants, other fixture-bearing sigs — none of
/// which have a usable parameterless `new` since they're either `abstract` or
/// need their own fields populated), else a bare constructor call.
fn one_value_for(target: &str, fixture_types: &HashSet<String>, ctx: &CsContext) -> String {
    if is_native_type_alias(target) {
        return cs_zero_value(&resolve_type(TargetLang::CSharp, target)).to_string();
    }
    if ctx.variant_names.contains(target)
        || ctx.struct_map.get(target).map_or(false, |s| s.is_enum)
        || fixture_types.contains(target)
    {
        format!("Default{}()", cs_factory_suffix(target))
    } else {
        format!("new {}()", cs_type_name(target))
    }
}

/// C#'s zero value for a resolved native type.
fn cs_zero_value(cs_ty: &str) -> &'static str {
    match cs_ty {
        "string" => "\"\"",
        "bool" => "false",
        "double" => "0.0",
        _ => "0",
    }
}

fn default_value_for(target: &str, mult: &Multiplicity, owner: &str, ir: &OxidtrIR, fixture_types: &HashSet<String>, ctx: &CsContext) -> String {
    match mult {
        Multiplicity::One => one_value_for(target, fixture_types, ctx),
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set | Multiplicity::Seq => {
            if backend::is_safe_set_population(owner, target, ir, fixture_types) {
                format!("new List<{}>() {{ Default{}() }}", cs_type_name(target), cs_factory_suffix(target))
            } else {
                format!("new List<{}>()", cs_type_name(target))
            }
        }
    }
}

/// A native-alias literal that differs per index. `Bool` is absent: two values
/// cannot carry a cardinality of three.
fn cs_native_element(target: &str, i: usize) -> Option<String> {
    match target {
        "Int" => Some(format!("{i}L")),
        "Str" => Some(format!("\"item{i}\"")),
        "Float" => Some(format!("{i}.0")),
        _ => None,
    }
}

/// `count` elements of `target`, each distinct from the others.
///
/// `List<T>` does not deduplicate, so C# never had #96's collapse — but a
/// boundary fixture of indistinguishable elements is what every other backend
/// now avoids, and a later switch to a real set type would regress silently.
fn cs_distinct_elements(ir: &OxidtrIR, target: &str, count: usize) -> Vec<String> {
    let fallback = || vec![format!("Default{}()", cs_factory_suffix(target)); count];
    if cs_native_element(target, 0).is_some() {
        return (0..count).filter_map(|i| cs_native_element(target, i)).collect();
    }
    if crate::backend::is_native_type_alias(target) {
        return fallback();
    }
    let Some(s) = ir.structures.iter().find(|st| st.name == target) else { return fallback() };
    let scalar = s.fields.iter().find(|f| {
        f.value_type.is_none()
            && f.mult == Multiplicity::One
            && cs_native_element(&f.target, 0).is_some()
    });
    let Some(f) = scalar else { return fallback() };
    (0..count)
        .map(|i| format!("new {} {{ {} = {} }}",
            cs_ident(target), capitalize(&f.name), cs_native_element(&f.target, i).unwrap()))
        .collect()
}

/// The value a boundary fixture gives a field.
///
/// A `set`/`seq` under a cardinality bound is `count` elements; it used to be
/// an empty list whatever the bound said, so the fixture never reached the
/// boundary it was named for (#140).
fn boundary_value_for(
    ir: &OxidtrIR, sig_name: &str, f: &IRField,
    fixture_types: &HashSet<String>, ctx: &CsContext,
) -> String {
    match f.mult {
        Multiplicity::One => one_value_for(&f.target, fixture_types, ctx),
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set | Multiplicity::Seq => {
            let elem_ty = cs_type_name(&f.target);
            let Some(bound) = analyze::bounds_for_field(ir, sig_name, &f.name) else {
                return format!("new List<{elem_ty}>()");
            };
            let count = match bound {
                analyze::BoundKind::Exact(n) | analyze::BoundKind::AtMost(n)
                | analyze::BoundKind::AtLeast(n) => n,
            };
            let items = cs_distinct_elements(ir, &f.target, count);
            if items.is_empty() {
                format!("new List<{elem_ty}>()")
            } else {
                format!("new List<{elem_ty}> {{ {} }}", items.join(", "))
            }
        }
    }
}

// ── Tests.cs ─────────────────────────────────────────────────────────────────

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

/// A `^field` closure over a self-referential field, chasing `Lone`/`One` as a
/// single pointer and `Set`/`Seq` as a worklist — every variant must terminate
/// even on a cyclic graph.
fn generate_tc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("Tc{}", capitalize(&tc.field_name));
    // `sig` is the type that *declares* the field (the `start` parameter);
    // `target` is the field's own target type (what gets collected). These
    // coincide for a self-referential field (`Node.parent: lone Node`) but
    // differ for a subtype-to-supertype closure (`Branch.parent: lone Node`,
    // where `Branch extends Node`) — chasing a `Node` past the first hop
    // needs a downcast back to `Branch` since `Node` itself carries no
    // `Parent` property. See #102 round 3 defect 3.
    let sig = cs_ident(&tc.sig_name);
    let target = cs_ident(&tc.target_type);
    let field = capitalize(&tc.field_name);
    writeln!(out, "    private static List<{target}> {fn_name}({sig} start)").unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        var result = new List<{target}>();").unwrap();
    match &tc.mult {
        Multiplicity::Lone | Multiplicity::One => {
            writeln!(out, "        var seen = new HashSet<{target}>();").unwrap();
            writeln!(out, "        var current = start.{field};").unwrap();
            writeln!(out, "        while (current != null && seen.Add(current))").unwrap();
            writeln!(out, "        {{").unwrap();
            writeln!(out, "            result.Add(current);").unwrap();
            if sig == target {
                writeln!(out, "            current = current.{field};").unwrap();
            } else {
                writeln!(out, "            current = (current as {sig})?.{field};").unwrap();
            }
            writeln!(out, "        }}").unwrap();
        }
        Multiplicity::Set | Multiplicity::Seq => {
            writeln!(out, "        var queue = new List<{target}>(start.{field});").unwrap();
            writeln!(out, "        while (queue.Count > 0)").unwrap();
            writeln!(out, "        {{").unwrap();
            writeln!(out, "            var next = queue[0];").unwrap();
            writeln!(out, "            queue.RemoveAt(0);").unwrap();
            writeln!(out, "            if (result.Contains(next)) continue;").unwrap();
            writeln!(out, "            result.Add(next);").unwrap();
            if sig == target {
                writeln!(out, "            queue.AddRange(next.{field});").unwrap();
            } else {
                writeln!(out, "            if (next is {sig} typed) queue.AddRange(typed.{field});").unwrap();
            }
            writeln!(out, "        }}").unwrap();
        }
    }
    writeln!(out, "        return result;").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

fn generate_rtc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("Rtc{}", capitalize(&tc.field_name));
    let tc_name = format!("Tc{}", capitalize(&tc.field_name));
    let sig = cs_ident(&tc.sig_name);
    let target = cs_ident(&tc.target_type);

    writeln!(out, "    private static List<{target}> {fn_name}({sig} start)").unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        var result = new List<{target}> {{ start }};").unwrap();
    writeln!(out, "        result.AddRange({tc_name}(start));").unwrap();
    writeln!(out, "        return result;").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "using Xunit;").unwrap();
    writeln!(out, "using System.Collections.Generic;").unwrap();
    // `Zip`/`Union`/`Intersect`/`Except` below are `System.Linq` extension
    // methods, not `List<T>` members — without this using they are CS1061.
    writeln!(out, "using System.Linq;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "public class ModelsTest").unwrap();
    writeln!(out, "{{").unwrap();

    let sig_names: HashSet<String> = ir.structures.iter().map(|s| s.name.clone()).collect();
    let has_fixture: HashSet<String> = ir.structures.iter()
        .filter(|s| !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone())
        .collect();
    let all_constraints = analyze::analyze(ir);

    // Transitive-closure helpers (`^field` / `*field`): C# has no `check_*` fact-path
    // infrastructure to hang these off of (see #78), so emit only the ones
    // the constraint/property/operation bodies below actually call.
    let (tc_fields, rtc_fields) = collect_closure_fields(ir);
    for tc in &tc_fields {
        generate_tc_function(&mut out, tc);
    }
    for rtc in &rtc_fields {
        generate_rtc_function(&mut out, rtc);
    }

    // --- Constraint tests (facts) ---
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };

        // Temporal facts with prime → transition test
        if analyze::expr_contains_prime(&constraint.expr) {
            let test_name = format!("Transition_{}", capitalize(&fact_name));
            let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "    /// @temporal Transition constraint: {fact_name}").unwrap();
            writeln!(out, "    /// Verifies: pre→post state relationship ({desc})").unwrap();
            writeln!(out, "    [Fact]").unwrap();
            writeln!(out, "    public void {test_name}()").unwrap();
            writeln!(out, "    {{").unwrap();
            for (pname, tname) in &params {
                let tcs = cs_ident(tname);
                if has_fixture.contains(tname) {
                    writeln!(out, "        var {pname} = new List<{tcs}>{{ Fixtures.Default{}() }};", cs_factory_suffix(tname)).unwrap();
                } else {
                    writeln!(out, "        var {pname} = new List<{tcs}>();").unwrap();
                }
                let next_pname = compose_ident("next", pname);
                writeln!(out, "        var {next_pname} = new List<{tcs}>({pname});").unwrap();
            }
            if let Some((_kind, bindings, inner_body)) = analyze::strip_outer_quantifier(&constraint.expr) {
                let bind_vars: Vec<String> = bindings.iter()
                    .flat_map(|b| b.vars.clone())
                    .collect();
                let bound: HashSet<String> = bind_vars.iter().cloned().collect();
                let rewritten_body = expr_translator::finalize_post_state_idents(
                    &analyze::rewrite_prime_as_post_state(inner_body), &bound,
                );
                let body_str = expr_translator::translate_with_ir(&rewritten_body, ir);
                // The pre/post pair is walked over the *binding's own* domain.
                // Taking `params[0]` paired the binder with whichever sig
                // sorted first, so `all f: Foo` iterated `auxs` (#110).
                let domain = match &bindings[0].domain {
                    crate::parser::ast::Expr::VarRef(sig) => Some(sig.clone()),
                    _ => None,
                };
                let pname = domain.as_ref()
                    .and_then(|sig| params.iter().find(|(_, t)| t == sig).map(|(p, _)| p.clone()));
                match (bind_vars.as_slice(), pname) {
                    ([v], Some(pname)) => {
                        let next_pname = compose_ident("next", &pname);
                        let v_id = cs_ident(v);
                        let next_v = cs_ident(&compose_ident("next", v));
                        writeln!(out, "        foreach (var ({v_id}, {next_v}) in {pname}.Zip({next_pname}))").unwrap();
                        writeln!(out, "        {{").unwrap();
                        writeln!(out, "            Assert.True({body_str});").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    // Anything else has no pre/post pairing to walk: two
                    // binders over one domain leave it ambiguous which side of
                    // the transition each names, and a domain that is not a
                    // bare sig has no materialised list. Emitting the body
                    // unbound is worse than emitting nothing — Rust already
                    // declines this shape and says so (#110, #104).
                    _ => {
                        writeln!(out, "        // oxidtr: skipped — a transition over {} binding(s) \
                            has no pre/post pairing to walk. See #104.", bind_vars.len()).unwrap();
                    }
                }
            } else {
                let rewritten = expr_translator::finalize_post_state_idents(
                    &analyze::rewrite_prime_as_post_state(&constraint.expr), &HashSet::new(),
                );
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
        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
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

        // Binary temporal / liveness: a single snapshot cannot decide these, so
        // they get a trace checker — the machinery every other backend has and
        // C# did not, which left the operator erased entirely (#78).
        if temporal_kind == Some(analyze::TemporalKind::Binary) || matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            emit_temporal_test_and_checker(&mut out, &test_name, &fact_name, &constraint.expr, &params, ir, temporal_kind);
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
            let tcs = cs_ident(tname);
            if has_fixture.contains(tname) {
                writeln!(out, "        var {pname} = new List<{tcs}>{{ Fixtures.Default{}() }};", cs_factory_suffix(tname)).unwrap();
            } else {
                writeln!(out, "        var {pname} = new List<{tcs}>();").unwrap();
            }
        }
        writeln!(out, "        Assert.True({body});").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // --- Property tests ---
    for prop in &ir.properties {
        let test_name = capitalize(&prop.name);
        let params = expr_translator::extract_params(&prop.expr, &sig_names, ir);
        let body = expr_translator::translate_with_ir(&prop.expr, ir);

        // An `assert` carries temporal operators just as a `fact` does, and
        // translating its operand alone silently drops them (#78).
        let temporal_kind = analyze::expr_temporal_kind(&prop.expr);
        if matches!(
            temporal_kind,
            Some(analyze::TemporalKind::Liveness)
                | Some(analyze::TemporalKind::PastLiveness)
                | Some(analyze::TemporalKind::Binary)
        ) {
            emit_temporal_test_and_checker(&mut out, &test_name, &prop.name, &prop.expr, &params, ir, temporal_kind);
            continue;
        }

        writeln!(out, "    [Fact]").unwrap();
        writeln!(out, "    public void {test_name}()").unwrap();
        writeln!(out, "    {{").unwrap();
        for (pname, tname) in &params {
            let tcs = cs_ident(tname);
            if has_fixture.contains(tname) {
                writeln!(out, "        var {pname} = new List<{tcs}>{{ Fixtures.Default{}() }};", cs_factory_suffix(tname)).unwrap();
            } else {
                writeln!(out, "        var {pname} = new List<{tcs}>();").unwrap();
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
                        let upper = capitalize(field_name);
                        writeln!(out, "    [Fact]").unwrap();
                        writeln!(out, "    public void Anomaly_{sig_name}_{upper}_Unconstrained()").unwrap();
                        writeln!(out, "    {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.Default{}();", cs_factory_suffix(sig_name)).unwrap();
                        writeln!(out, "        Assert.NotNull(instance.{upper} as object);").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        let upper = capitalize(field_name);
                        writeln!(out, "    [Fact]").unwrap();
                        writeln!(out, "    public void Anomaly_{sig_name}_{upper}_Empty()").unwrap();
                        writeln!(out, "    {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.AnomalyEmpty{}();", cs_factory_suffix(sig_name)).unwrap();
                        writeln!(out, "        Assert.NotNull(instance.{upper});").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        let upper = capitalize(field_name);
                        writeln!(out, "    [Fact]").unwrap();
                        writeln!(out, "    public void Anomaly_{sig_name}_{upper}_SelfRef()").unwrap();
                        writeln!(out, "    {{").unwrap();
                        writeln!(out, "        var instance = Fixtures.Default{}();", cs_factory_suffix(sig_name)).unwrap();
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
            let params_a = expr_translator::extract_params(&ca.expr, &sig_names, ir);
            let params_b = expr_translator::extract_params(&cb.expr, &sig_names, ir);
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
                let tcs = cs_ident(tname);
                if has_fixture.contains(tname) {
                    writeln!(out, "        var {pname} = new List<{tcs}>{{ Fixtures.Default{}() }};", cs_factory_suffix(tname)).unwrap();
                } else {
                    writeln!(out, "        var {pname} = new List<{tcs}>();").unwrap();
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

/// Build a field/param/return type string, resolving Alloy native aliases
/// (Int/Str/Bool/Float) to their C# primitives and escaping any remaining
/// identifier that collides with a C# reserved word.
fn mult_to_cs_type(target: &str, mult: &Multiplicity) -> String {
    let ty = cs_type_name(target);
    match mult {
        Multiplicity::One => ty,
        Multiplicity::Lone => format!("{ty}?"),
        Multiplicity::Set | Multiplicity::Seq => format!("List<{ty}>"),
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
    let camel = match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    };
    cs_ident(&camel)
}

/// All 77 C# reserved keywords, including the primitive-type ones (bool, int,
/// long, string, double, …). The `@` verbatim-identifier escape makes any of
/// them usable as a declaration or reference. Shared by `expr_translator`
/// (`use super::{CS_KEYWORDS, cs_ident};`) rather than duplicated.
///
/// The primitive-type keywords are the tricky half: they are ALSO the correct
/// bare token in a *resolved-type* position (an Alloy `Int` field resolves to
/// `long`, which must stay bare, not become `@long`). `cs_ident` alone cannot
/// tell those two positions apart — see `cs_type_name`, which discriminates
/// on the original Alloy target before resolution and is the one that should
/// wrap `resolve_type`'s output. `cs_ident` itself is only for pure
/// user-defined identifier positions (sig/field/variant names, quantifier
/// vars, generated locals) where escaping all 77 is always correct.
pub(crate) const CS_KEYWORDS: &[&str] = &[
    "abstract", "as", "base", "bool", "break", "byte", "case", "catch", "char",
    "checked", "class", "const", "continue", "decimal", "default", "delegate", "do",
    "double", "else", "enum", "event", "explicit", "extern", "false", "finally",
    "fixed", "float", "for", "foreach", "goto", "if", "implicit", "in", "int",
    "interface", "internal", "is", "lock", "long", "namespace", "new", "null",
    "object", "operator", "out", "override", "params", "private", "protected",
    "public", "readonly", "ref", "return", "sbyte", "sealed", "short", "sizeof",
    "stackalloc", "static", "string", "struct", "switch", "this", "throw", "true",
    "try", "typeof", "uint", "ulong", "unchecked", "unsafe", "ushort", "using",
    "virtual", "void", "volatile", "while",
];

pub(crate) fn cs_ident(name: &str) -> String {
    if CS_KEYWORDS.contains(&name) {
        format!("@{name}")
    } else {
        name.to_string()
    }
}

/// Compose `prefix` + a PascalCased `name` into a single identifier,
/// escaping only the *finished* result. `name` may already be an escaped
/// identifier (a leading `@`, e.g. the local generated for a keyword-named
/// sig's plural, `@params`) — capitalizing and concatenating that as-is
/// strands the `@` mid-token (`next@params`, CS1002/CS1003), so the escape is
/// stripped before composing and re-applied, if needed, to the composed
/// whole instead. See #102 round 3 defect 2.
pub(crate) fn compose_ident(prefix: &str, name: &str) -> String {
    let bare = name.strip_prefix('@').unwrap_or(name);
    cs_ident(&format!("{prefix}{}", capitalize(bare)))
}

/// Resolve `target` to its C# type name for a *resolved-type* position (a
/// field's declared type, a `List<T>` element, a fixture's constructed
/// type). Discriminates on the **original** Alloy `target`, not on the
/// resolved string: a native alias (`Int`/`Str`/`Bool`/`Float`) resolves to
/// the genuine bare C# keyword (`long`, `string`, …); anything else is a
/// user sig name and gets escaped if it collides with a C# keyword (e.g. a
/// sig named `object` → `@object`). Escaping the resolved string instead
/// would turn a correct bare `long` into an unresolvable `@long` (CS0246).
fn cs_type_name(target: &str) -> String {
    let resolved = resolve_type(TargetLang::CSharp, target);
    if is_native_type_alias(target) {
        resolved
    } else {
        cs_ident(&resolved)
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

fn cs_default_value(target: &str, mult: &Multiplicity, fixture_types: &HashSet<String>, ctx: &CsContext) -> String {
    match mult {
        Multiplicity::One => one_value_for(target, fixture_types, ctx),
        Multiplicity::Lone => "null".to_string(),
        Multiplicity::Set | Multiplicity::Seq => format!("new List<{}>()", cs_type_name(target)),
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

/// The name the trace checker for this constraint is emitted under.
fn temporal_checker_name(
    name: &str,
    expr: &crate::parser::ast::Expr,
    kind: Option<analyze::TemporalKind>,
) -> Option<String> {
    let cap = capitalize(name);
    match kind {
        Some(analyze::TemporalKind::Binary) => {
            let (op, _, _, _) = analyze::find_temporal_binary_with_bindings(expr)?;
            let op_label = match op {
                crate::parser::ast::TemporalBinaryOp::Until => "Until",
                crate::parser::ast::TemporalBinaryOp::Since => "Since",
                crate::parser::ast::TemporalBinaryOp::Release => "Release",
                crate::parser::ast::TemporalBinaryOp::Triggered => "Triggered",
            };
            Some(format!("Check{op_label}{cap}"))
        }
        Some(analyze::TemporalKind::PastLiveness) => Some(format!("CheckPastLiveness{cap}")),
        Some(analyze::TemporalKind::Liveness) => Some(format!("CheckLiveness{cap}")),
        _ => None,
    }
}

/// Emit a temporal constraint's test together with the trace checker it calls.
///
/// Shared by the `fact` and `assert` paths. A single snapshot cannot decide a
/// liveness or binary temporal property, so the test pins the one thing a
/// snapshot *can* decide — the empty trace satisfies neither — and that is also
/// what keeps the checker referenced rather than dead.
fn emit_temporal_test_and_checker(
    out: &mut String,
    test_name: &str,
    name: &str,
    expr: &crate::parser::ast::Expr,
    params: &[(String, String)],
    ir: &OxidtrIR,
    temporal_kind: Option<analyze::TemporalKind>,
) {
    let checker = temporal_checker_name(name, expr, temporal_kind);
    let elem = params.first().map(|(_, t)| cs_type_name(t)).unwrap_or_else(|| "object".to_string());

    writeln!(out, "    [Fact]").unwrap();
    writeln!(out, "    public void {test_name}()").unwrap();
    writeln!(out, "    {{").unwrap();
    writeln!(out, "        // full verification needs a trace; an empty trace can never").unwrap();
    writeln!(out, "        // satisfy it, which at least exercises the checker.").unwrap();
    match &checker {
        Some(c) => {
            writeln!(out, "        var trace = new List<List<{elem}>>();").unwrap();
            writeln!(out, "        Assert.False({c}(trace));").unwrap();
        }
        None => writeln!(out, "        // oxidtr: no checker emitted for this shape").unwrap(),
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    let Some(checker) = checker else { return };
    let pname = params.first().map(|(p, _)| p.clone()).unwrap_or_else(|| "state".to_string());

    match temporal_kind {
        Some(analyze::TemporalKind::Binary) => {
            let Some((op, left, right, _)) = analyze::find_temporal_binary_with_bindings(expr) else { return };
            let left_pred = expr_translator::translate_trace_body(left, ir);
            let right_pred = expr_translator::translate_trace_body(right, ir);
            let semantics = match op {
                crate::parser::ast::TemporalBinaryOp::Until => "left holds until right becomes true",
                crate::parser::ast::TemporalBinaryOp::Since => "left has held since right was true",
                crate::parser::ast::TemporalBinaryOp::Release => "right holds until left releases it",
                crate::parser::ast::TemporalBinaryOp::Triggered => "left triggers right",
            };
            writeln!(out, "    /// <summary>Trace checker: {semantics}.</summary>").unwrap();
            writeln!(out, "    private static bool {checker}(List<List<{elem}>> trace)").unwrap();
            writeln!(out, "    {{").unwrap();
            match op {
                crate::parser::ast::TemporalBinaryOp::Until => {
                    writeln!(out, "        var pos = trace.FindIndex({pname} => {right_pred});").unwrap();
                    writeln!(out, "        return pos >= 0 && trace.Take(pos).All({pname} => {left_pred});").unwrap();
                }
                crate::parser::ast::TemporalBinaryOp::Since => {
                    writeln!(out, "        var pos = trace.FindLastIndex({pname} => {right_pred});").unwrap();
                    writeln!(out, "        return pos >= 0 && trace.Skip(pos).All({pname} => {left_pred});").unwrap();
                }
                crate::parser::ast::TemporalBinaryOp::Release => {
                    writeln!(out, "        var pos = trace.FindIndex({pname} => {left_pred});").unwrap();
                    writeln!(out, "        return pos >= 0").unwrap();
                    writeln!(out, "            ? trace.Take(pos + 1).All({pname} => {right_pred})").unwrap();
                    writeln!(out, "            : trace.All({pname} => {right_pred});").unwrap();
                }
                crate::parser::ast::TemporalBinaryOp::Triggered => {
                    writeln!(out, "        for (var i = 0; i < trace.Count; i++)").unwrap();
                    writeln!(out, "        {{").unwrap();
                    writeln!(out, "            var {pname} = trace[i];").unwrap();
                    writeln!(out, "            if ({right_pred} && !trace.Take(i + 1).Any({pname} => {left_pred})) return false;").unwrap();
                    writeln!(out, "        }}").unwrap();
                    writeln!(out, "        return true;").unwrap();
                }
            }
            writeln!(out, "    }}").unwrap();
        }
        _ => {
            let past = temporal_kind == Some(analyze::TemporalKind::PastLiveness);
            let semantics = if past {
                "property held in at least one past state"
            } else {
                "property holds in at least one future state"
            };
            let pred = expr_translator::translate_trace_body(expr, ir);
            writeln!(out, "    /// <summary>Trace checker: {semantics}.</summary>").unwrap();
            writeln!(out, "    private static bool {checker}(List<List<{elem}>> trace) =>").unwrap();
            writeln!(out, "        trace.Any({pname} => {pred});").unwrap();
        }
    }
    writeln!(out).unwrap();
}

/// A sig's own fields plus those it inherits from an abstract ancestor, paired
/// with the sig that declares each. A variant that adds nothing of its own
/// still has to initialise what its parent declares.
fn inherited_and_own_fields<'a>(ir: &'a OxidtrIR, s: &'a StructureNode) -> Vec<(String, &'a IRField)> {
    let mut out: Vec<(String, &IRField)> = Vec::new();
    let mut cur = s.parent.clone();
    let mut chain: Vec<&StructureNode> = Vec::new();
    while let Some(name) = cur {
        match ir.structures.iter().find(|p| p.name == name) {
            Some(p) => {
                chain.push(p);
                cur = p.parent.clone();
            }
            None => break,
        }
    }
    // Parent-most first, so the initialiser reads top-down.
    for p in chain.into_iter().rev() {
        out.extend(p.fields.iter().map(|f| (p.name.clone(), f)));
    }
    out.extend(s.fields.iter().map(|f| (s.name.clone(), f)));
    out
}
