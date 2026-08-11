pub mod expr_translator;

use crate::backend::{GeneratedFile, TargetLang, is_native_type_alias, resolve_type, variant_parent};
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity, TemporalBinaryOp};
use crate::analyze;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = SwiftContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "Models.swift".to_string(),
        content: generate_models(ir, &ctx),
    });

    let has_tc = ir_uses_tc(ir);

    if has_tc {
        files.push(GeneratedFile {
            path: "Helpers.swift".to_string(),
            content: generate_helpers(ir),
        });
    }

    if !ir.operations.is_empty() {
        files.push(GeneratedFile {
            path: "Operations.swift".to_string(),
            content: generate_operations(ir),
        });
    }

    if !ir.properties.is_empty() || !ir.constraints.is_empty() {
        files.push(GeneratedFile {
            path: "Tests.swift".to_string(),
            content: generate_tests(ir, &ctx),
        });
    }

    files.push(GeneratedFile {
        path: "Fixtures.swift".to_string(),
        content: generate_fixtures(ir, &ctx),
    });

    files
}

// ── Context ──────────────────────────────────────────────────────────────────

struct SwiftContext {
    children: HashMap<String, Vec<String>>,
    variant_names: HashSet<String>,
    struct_map: HashMap<String, StructureNode>,
    /// Types whose generated fixture terminates; see `find_terminating_types`.
    terminating: HashSet<String>,
    /// Per enum, the case whose payload was constructible first.
    enum_witness: HashMap<String, String>,
    /// Types that store themselves inline (directly or transitively). Swift
    /// rejects those as value types: structs become `final class`, enums
    /// become `indirect enum`.
    recursive_types: HashSet<String>,
}

impl SwiftContext {
    fn from_ir(ir: &OxidtrIR) -> Self {
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        for s in &ir.structures {
            if let Some(parent) = &s.parent {
                children.entry(parent.clone()).or_default().push(s.name.clone());
            }
        }
        let enum_parents: HashSet<String> = ir.structures.iter()
            .filter(|s| s.is_enum).map(|s| s.name.clone()).collect();
        // A child that is itself an enum still needs its own declaration — it is
        // a nested type, not a case whose payload got inlined into the parent.
        let variant_names: HashSet<String> = ir.structures.iter()
            .filter(|s| !s.is_enum
                && s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
            .map(|s| s.name.clone()).collect();
        let struct_map: HashMap<String, StructureNode> = ir.structures.iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect();
        let recursive_types = find_recursive_types(ir, &children, &variant_names);
        let (terminating, enum_witness) = find_terminating_types(ir, &children, &variant_names);
        SwiftContext { children, variant_names, struct_map, recursive_types, terminating, enum_witness }
    }

    fn is_variant(&self, name: &str) -> bool {
        self.variant_names.contains(name)
    }

    fn is_recursive(&self, name: &str) -> bool {
        self.recursive_types.contains(name)
    }

}

/// Fields stored *inline* in their owner: `one T` is a `T`, `lone T` is a `T?`,
/// and both are laid out in place. `set`/`seq`/map fields become
/// Set/Array/Dictionary, which are heap-backed and therefore break a cycle.
fn is_inline_field(f: &IRField) -> bool {
    f.value_type.is_none() && matches!(f.mult, Multiplicity::One | Multiplicity::Lone)
}

/// Names of every type that transitively contains itself by value. Enum types
/// are edges from the union of the abstract parent's fields and each variant's
/// fields, since that is exactly what the payload carries.
fn find_recursive_types(
    ir: &OxidtrIR,
    children: &HashMap<String, Vec<String>>,
    variant_names: &HashSet<String>,
) -> HashSet<String> {
    let struct_map: HashMap<&str, &StructureNode> = ir.structures.iter()
        .map(|s| (s.name.as_str(), s)).collect();

    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for s in &ir.structures {
        if variant_names.contains(&s.name) { continue; }
        let mut targets: Vec<&str> = s.fields.iter()
            .filter(|f| is_inline_field(f)).map(|f| f.target.as_str()).collect();
        if s.is_enum {
            for v in children.get(&s.name).map(|v| v.as_slice()).unwrap_or(&[]) {
                if let Some(child) = struct_map.get(v.as_str()) {
                    targets.extend(child.fields.iter()
                        .filter(|f| is_inline_field(f)).map(|f| f.target.as_str()));
                }
            }
        }
        edges.insert(s.name.as_str(), targets);
    }

    // ponytail: break every type on a cycle, not a minimum feedback vertex set.
    // Over-boxing costs an allocation; under-boxing does not compile.
    let mut result = HashSet::new();
    for start in edges.keys() {
        let mut visited: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = edges[start].clone();
        while let Some(cur) = stack.pop() {
            if cur == *start {
                result.insert(start.to_string());
                break;
            }
            if !visited.insert(cur) { continue; }
            if let Some(next) = edges.get(cur) {
                stack.extend(next.iter().copied());
            }
        }
    }
    result
}

// ── Models.swift ─────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR, ctx: &SwiftContext) -> String {
    let mut out = String::new();
    writeln!(out, "import Foundation").unwrap();
    writeln!(out).unwrap();

    let disj_fields = analyze::disj_fields(ir);

    for s in &ir.structures {
        if ctx.is_variant(&s.name) { continue; }
        if is_native_type_alias(&s.name) { continue; }

        let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
        if !constraint_names.is_empty() {
            writeln!(out, "/// Invariants:").unwrap();
            for cn in &constraint_names {
                writeln!(out, "/// - {cn}").unwrap();
            }
        }

        // Exhaustive constraint doc comments
        let sig_constraints = analyze::constraints_for_sig(ir, &s.name);
        for c in &sig_constraints {
            if let analyze::ConstraintInfo::Exhaustive { categories, .. } = c {
                let cats = categories.join(", ");
                writeln!(out, "/// - exhaustive: must belong to one of [{cats}]").unwrap();
            }
        }

        if s.is_enum {
            generate_enum(&mut out, s, ctx);
        } else {
            generate_struct(&mut out, s, ir, ctx, &disj_fields);
        }
        writeln!(out).unwrap();
    }

    // Payload cases are not values, so membership in one is a case test.
    generate_case_tests(&mut out, ir, ctx);

    // Derived fields: receiver functions → extensions
    generate_derived_fields(&mut out, ir);

    // A field declared to hold a variant takes the parent's type, so the
    // variant itself would otherwise be lost — keep it as a check (#93).
    generate_variant_field_validators(&mut out, ir);

    out
}

/// `sig Holder { child: one Child }` where `Child` is a case of the abstract
/// `Parent` lowers `child` to `Parent`, because Swift has no type for a single
/// enum case. That the value must be *that* case is still part of the model, so
/// it becomes a check.
fn generate_variant_field_validators(out: &mut String, ir: &OxidtrIR) {
    if crate::backend::variants_used_as_field_targets(ir).is_empty() {
        return;
    }
    for s in &ir.structures {
        if s.is_enum || variant_parent(ir, &s.name).is_some() {
            continue;
        }
        for f in &s.fields {
            let Some(parent) = variant_parent(ir, &f.target) else { continue };
            // A set/lone of a variant needs a per-element check; see #93 follow-up.
            if f.mult != Multiplicity::One {
                continue;
            }
            let sig = &s.name;
            let variant = &f.target;
            let field = to_swift_field_name(&f.name);
            let case = to_swift_case_name(variant);
            writeln!(out, "/// `{sig}.{}` is declared as `{variant}`, a case of `{parent}`.", f.name).unwrap();
            writeln!(out, "func validate{sig}{variant}(_ value: {sig}) -> Bool {{").unwrap();
            writeln!(out, "    if case .{case} = value.{field} {{ return true }}").unwrap();
            writeln!(out, "    return false").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }
}

/// `is<Case>` for every payload-bearing enum case, so `x in Lit` has something
/// to translate to — `Expr.lit` is a constructor, not a value.
fn generate_case_tests(out: &mut String, ir: &OxidtrIR, ctx: &SwiftContext) {
    for s in &ir.structures {
        if !s.is_enum { continue; }
        let variants = match ctx.children.get(&s.name) {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };
        let tests: Vec<(String, String)> = variants.iter()
            .filter_map(|v| expr_translator::variant_case_test(v, ir)
                .map(|name| (name, to_swift_case_name(v))))
            .collect();
        if tests.is_empty() { continue; }

        writeln!(out, "extension {} {{", s.name).unwrap();
        for (name, case) in tests {
            writeln!(out, "    var {name}: Bool {{").unwrap();
            writeln!(out, "        if case .{case} = self {{ return true }}").unwrap();
            writeln!(out, "        return false").unwrap();
            writeln!(out, "    }}").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
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
        writeln!(out, "extension {sig_name} {{").unwrap();
        for op in ops {
            let return_str = match &op.return_type {
                Some(rt) => swift_return_type(&rt.type_name, &rt.mult),
                None => "Bool".to_string(),
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
                // No params → computed property
                writeln!(out, "    var {}: {return_str} {{", op.name).unwrap();
                writeln!(out, "        {body_str}").unwrap();
                writeln!(out, "    }}").unwrap();
            } else {
                let params = op.params.iter().map(|p| {
                    let type_str = swift_return_type(&p.type_name, &p.mult);
                    format!("{}: {type_str}", p.name)
                }).collect::<Vec<_>>().join(", ");
                writeln!(out, "    func {}({params}) -> {return_str} {{", op.name).unwrap();
                writeln!(out, "        return {body_str}").unwrap();
                writeln!(out, "    }}").unwrap();
            }
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

fn generate_struct(out: &mut String, s: &StructureNode, ir: &OxidtrIR, ctx: &SwiftContext, disj_fields: &[(String, String)]) {
    // Singleton: one sig → static let
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        if s.is_var {
            writeln!(out, "/// @alloy: var sig").unwrap();
        }
        writeln!(out, "struct {}: Equatable, Hashable {{", s.name).unwrap();
        writeln!(out, "    static let shared = {}()", s.name).unwrap();
        writeln!(out, "}}").unwrap();
        return;
    }

    if s.is_var {
        writeln!(out, "/// @alloy: var sig").unwrap();
    }
    if s.fields.is_empty() {
        writeln!(out, "struct {}: Equatable, Hashable {{", s.name).unwrap();
        writeln!(out, "}}").unwrap();
    } else {
        // A value type that transitively stores itself has infinite size in
        // Swift; emitting it as a reference type is what breaks the cycle.
        let is_class = ctx.is_recursive(&s.name);
        let keyword = if is_class { "final class" } else { "struct" };
        writeln!(out, "{keyword} {}: Equatable, Hashable {{", s.name).unwrap();
        let mut props: Vec<(String, String)> = Vec::new();
        for f in &s.fields {
            // A variant is a case of the parent enum, not a type (#93).
            let target = variant_parent(ir, &f.target).unwrap_or_else(|| f.target.clone());
            let resolved_target = resolve_type(TargetLang::Swift, &target);
            let type_str = if let Some(vt) = &f.value_type {
                let resolved_vt = resolve_type(TargetLang::Swift, vt);
                format!("[{}: {}]", resolved_target, resolved_vt)
            } else {
                mult_to_swift_type(&resolved_target, &f.mult)
            };
            props.push((to_swift_field_name(&f.name), type_str.clone()));

            // Comments for special patterns
            let target_mult = analyze::sig_multiplicity_for(ir, &f.target);
            if target_mult == SigMultiplicity::Lone && f.mult == Multiplicity::One {
                writeln!(out, "    // Note: lone sig target — may not exist").unwrap();
            }
            if disj_fields.iter().any(|(sig, field)| sig == &s.name && field == &f.name) {
                if f.mult == Multiplicity::Seq {
                    writeln!(out, "    // Consider using Set for uniqueness (disj constraint)").unwrap();
                }
            }

            let let_or_var = if f.is_var { "var" } else { "let" };
            writeln!(out, "    {let_or_var} {}: {type_str}", to_swift_field_name(&f.name)).unwrap();
        }

        // Generate validate() method for constraint validation
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
            writeln!(out, "    func validate() -> [String] {{").unwrap();
            writeln!(out, "        var errors: [String] = []").unwrap();
            for c in &sig_constraints {
                match c {
                    analyze::ConstraintInfo::NoSelfRef { field_name, .. } => {
                        let fname = to_swift_field_name(field_name);
                        writeln!(out, "        if {fname} as AnyObject === self as AnyObject {{").unwrap();
                        writeln!(out, "            errors.append(\"{fname} must not reference self\")").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    analyze::ConstraintInfo::Acyclic { field_name, .. } => {
                        let fname = to_swift_field_name(field_name);
                        writeln!(out, "        do {{").unwrap();
                        writeln!(out, "            var seen = Set<ObjectIdentifier>()").unwrap();
                        writeln!(out, "            var cur: {type_name}? = self", type_name = s.name).unwrap();
                        writeln!(out, "            while let node = cur {{").unwrap();
                        writeln!(out, "                let id = ObjectIdentifier(node as AnyObject)").unwrap();
                        writeln!(out, "                if seen.contains(id) {{").unwrap();
                        writeln!(out, "                    errors.append(\"{fname} must not form a cycle\")").unwrap();
                        writeln!(out, "                    break").unwrap();
                        writeln!(out, "                }}").unwrap();
                        writeln!(out, "                seen.insert(id)").unwrap();
                        writeln!(out, "                cur = node.{fname}").unwrap();
                        writeln!(out, "            }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    analyze::ConstraintInfo::FieldOrdering { left_field, op, right_field, .. } => {
                        let lf = to_swift_field_name(left_field);
                        let rf = to_swift_field_name(right_field);
                        let (swift_op, negated_op) = match op {
                            CompareOp::Lt => ("<", ">="),
                            CompareOp::Gt => (">", "<="),
                            CompareOp::Lte => ("<=", ">"),
                            CompareOp::Gte => (">=", "<"),
                            _ => continue,
                        };
                        writeln!(out, "        if {lf} {negated_op} {rf} {{").unwrap();
                        writeln!(out, "            errors.append(\"{lf} must be {swift_op} {rf}\")").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    analyze::ConstraintInfo::Implication { condition, consequent, .. } => {
                        let cond = translate_validator_expr_swift(condition, &s.name);
                        let cons = translate_validator_expr_swift(consequent, &s.name);
                        let desc = format!("{} implies {}", analyze::describe_expr(condition), analyze::describe_expr(consequent));
                        writeln!(out, "        if {cond} && !({cons}) {{").unwrap();
                        writeln!(out, "            errors.append(\"{}\"))", desc.replace('"', "\\\"")).unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    analyze::ConstraintInfo::Iff { left, right, .. } => {
                        let l = translate_validator_expr_swift(left, &s.name);
                        let r = translate_validator_expr_swift(right, &s.name);
                        let desc = format!("{} iff {}", analyze::describe_expr(left), analyze::describe_expr(right));
                        writeln!(out, "        if ({l}) != ({r}) {{").unwrap();
                        writeln!(out, "            errors.append(\"{}\"))", desc.replace('"', "\\\"")).unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    analyze::ConstraintInfo::Prohibition { condition, .. } => {
                        let cond = translate_validator_expr_swift(condition, &s.name);
                        let desc = analyze::describe_expr(condition);
                        writeln!(out, "        if {cond} {{").unwrap();
                        writeln!(out, "            errors.append(\"prohibited: {}\"))", desc.replace('"', "\\\"")).unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    analyze::ConstraintInfo::Disjoint { left, right, .. } => {
                        let left_field = to_swift_field_name(left.rsplit('.').next().unwrap_or(left));
                        let right_field = to_swift_field_name(right.rsplit('.').next().unwrap_or(right));
                        writeln!(out, "        if !{left_field}.isDisjoint(with: {right_field}) {{").unwrap();
                        writeln!(out, "            errors.append(\"{left_field} and {right_field} must not overlap (disjoint constraint)\")").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    analyze::ConstraintInfo::Exhaustive { categories, .. } => {
                        let cats = categories.join(", ");
                        let checks: Vec<String> = categories.iter().map(|cat| {
                            let parts: Vec<&str> = cat.split('.').collect();
                            if parts.len() == 2 {
                                format!("{}.{}.contains(self)", parts[0], to_swift_field_name(parts[1]))
                            } else {
                                format!("{cat}.contains(self)")
                            }
                        }).collect();
                        let condition = checks.join(" || ");
                        writeln!(out, "        if !({condition}) {{").unwrap();
                        writeln!(out, "            errors.append(\"must belong to one of [{cats}] (exhaustive constraint)\")").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    _ => {}
                }
            }
            // Disj uniqueness checks for seq fields
            for (dsig, dfield) in &disj {
                if dsig == &s.name {
                    if let Some(f) = s.fields.iter().find(|f| f.name == *dfield) {
                        if f.mult == Multiplicity::Seq {
                            let fname = to_swift_field_name(dfield);
                            writeln!(out, "        if Set({fname}).count != {fname}.count {{").unwrap();
                            writeln!(out, "            errors.append(\"{fname} must not contain duplicates (disj constraint)\")").unwrap();
                            writeln!(out, "        }}").unwrap();
                        }
                    }
                }
            }
            writeln!(out, "        return errors").unwrap();
            writeln!(out, "    }}").unwrap();
        }

        // Classes get no memberwise init and no synthesized conformances.
        if is_class {
            let params = props.iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect::<Vec<_>>().join(", ");
            writeln!(out).unwrap();
            writeln!(out, "    init({params}) {{").unwrap();
            for (n, _) in &props {
                writeln!(out, "        self.{n} = {n}").unwrap();
            }
            writeln!(out, "    }}").unwrap();

            // Identity, not structure: an Alloy atom *is* its identity, a
            // field-by-field walk recurses forever on a cyclic instance, and
            // hashing mutable (`var`) fields breaks Set/Dictionary invariants
            // the moment one is mutated.
            writeln!(out).unwrap();
            writeln!(out, "    static func == (lhs: {0}, rhs: {0}) -> Bool {{", s.name).unwrap();
            writeln!(out, "        lhs === rhs").unwrap();
            writeln!(out, "    }}").unwrap();

            writeln!(out).unwrap();
            writeln!(out, "    func hash(into hasher: inout Hasher) {{").unwrap();
            writeln!(out, "        hasher.combine(ObjectIdentifier(self))").unwrap();
            writeln!(out, "    }}").unwrap();
        }

        writeln!(out, "}}").unwrap();
    }
}


fn generate_enum(out: &mut String, s: &StructureNode, ctx: &SwiftContext) {
    let variants = ctx.children.get(&s.name);

    // Parent abstract sig may have fields that should be inherited by all variants
    let parent_fields = &s.fields;

    // Check if all variants are unit (no fields, including inherited)
    let all_unit = parent_fields.is_empty() && variants.map_or(true, |vs| {
        vs.iter().all(|v| ctx.struct_map.get(v).map_or(true, |st| st.fields.is_empty()))
    });

    if all_unit {
        // Simple enum
        writeln!(out, "enum {}: Equatable, Hashable, CaseIterable {{", s.name).unwrap();
        if let Some(variants) = variants {
            for v in variants {
                writeln!(out, "    case {}", to_swift_case_name(v)).unwrap();
            }
        }
        writeln!(out, "}}").unwrap();
    } else {
        // Enum with associated values. A payload that reaches the enum itself
        // needs `indirect` or the case has infinite size.
        let prefix = if ctx.is_recursive(&s.name) { "indirect " } else { "" };
        writeln!(out, "{prefix}enum {}: Equatable, Hashable {{", s.name).unwrap();
        if let Some(variants) = variants {
            for v in variants {
                let child = ctx.struct_map.get(v.as_str());
                let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
                // Combine parent fields + child fields
                let all_fields: Vec<&IRField> = parent_fields.iter().chain(child_fields.iter().copied()).collect();
                if !all_fields.is_empty() {
                    let params: Vec<String> = all_fields.iter().map(|f| {
                        let type_str = if let Some(vt) = &f.value_type {
                            format!("[{}: {}]",
                                resolve_type(TargetLang::Swift, &f.target),
                                resolve_type(TargetLang::Swift, vt))
                        } else {
                            mult_to_swift_type(&resolve_type(TargetLang::Swift, &f.target), &f.mult)
                        };
                        format!("{}: {type_str}", to_swift_field_name(&f.name))
                    }).collect();
                    writeln!(out, "    case {}({})", to_swift_case_name(v), params.join(", ")).unwrap();
                } else {
                    writeln!(out, "    case {}", to_swift_case_name(v)).unwrap();
                }
            }
        }
        writeln!(out, "}}").unwrap();
    }
}

fn mult_to_swift_type(target: &str, mult: &Multiplicity) -> String {
    match mult {
        Multiplicity::One => target.to_string(),
        Multiplicity::Lone => format!("{target}?"),
        Multiplicity::Set => format!("Set<{target}>"),
        Multiplicity::Seq => format!("[{target}]"),
    }
}

// ── Helpers.swift ────────────────────────────────────────────────────────────

fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "import Foundation").unwrap();
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
    let fn_name = format!("tc{}", expr_translator::capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "/// Transitive closure traversal for {sig}.{field}.").unwrap();
    match tc.mult {
        Multiplicity::Lone => {
            writeln!(out, "func {fn_name}(_ start: {sig}) -> [{sig}] {{").unwrap();
            writeln!(out, "    var result: [{sig}] = []").unwrap();
            writeln!(out, "    var current: {sig}? = start.{field}").unwrap();
            writeln!(out, "    while let node = current {{").unwrap();
            writeln!(out, "        result.append(node)").unwrap();
            writeln!(out, "        current = node.{field}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    return result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::Set | Multiplicity::Seq => {
            writeln!(out, "func {fn_name}(_ start: {sig}) -> [{sig}] {{").unwrap();
            writeln!(out, "    var result: [{sig}] = []").unwrap();
            writeln!(out, "    var queue = Array(start.{field})").unwrap();
            writeln!(out, "    while !queue.isEmpty {{").unwrap();
            writeln!(out, "        let next = queue.removeFirst()").unwrap();
            writeln!(out, "        if !result.contains(next) {{").unwrap();
            writeln!(out, "            result.append(next)").unwrap();
            writeln!(out, "            queue.append(contentsOf: next.{field})").unwrap();
            writeln!(out, "        }}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    return result").unwrap();
            writeln!(out, "}}").unwrap();
        }
        Multiplicity::One => {
            writeln!(out, "func {fn_name}(_ start: {sig}) -> [{sig}] {{").unwrap();
            writeln!(out, "    var result: [{sig}] = []").unwrap();
            writeln!(out, "    var current: {sig} = start.{field}").unwrap();
            writeln!(out, "    for _ in 0..<1000 {{").unwrap();
            writeln!(out, "        if result.contains(current) {{ return result }}").unwrap();
            writeln!(out, "        result.append(current)").unwrap();
            writeln!(out, "        current = current.{field}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "    return result").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
    writeln!(out).unwrap();
}

fn generate_rtc_function(out: &mut String, tc: &expr_translator::TCField) {
    let fn_name = format!("rtc{}", expr_translator::capitalize(&tc.field_name));
    let tc_name = format!("tc{}", expr_translator::capitalize(&tc.field_name));
    let sig = &tc.sig_name;
    let field = &tc.field_name;

    writeln!(out, "/// Reflexive-transitive closure for {sig}.{field} (id ∪ ^{field}).").unwrap();
    writeln!(out, "func {fn_name}(_ start: {sig}) -> [{sig}] {{").unwrap();
    writeln!(out, "    var result: [{sig}] = [start]").unwrap();
    writeln!(out, "    result.append(contentsOf: {tc_name}(start))").unwrap();
    writeln!(out, "    return result").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ── Operations.swift ─────────────────────────────────────────────────────────

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "import Foundation").unwrap();
    writeln!(out).unwrap();

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
                    Multiplicity::Seq => format!("[{}]", p.type_name),
                };
                format!("_ {}: {type_str}", p.name)
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Doc comments from body expressions
        if !op.body.is_empty() {
            let param_names: Vec<String> = op.params.iter().map(|p| p.name.clone()).collect();
            writeln!(out, "/// Operation: {}", op.name).unwrap();
            for expr in &op.body {
                let desc = analyze::describe_expr(expr);
                let tag = if analyze::is_pre_condition(expr, &param_names) { "pre" } else { "post" };
                writeln!(out, "/// - {tag}: {desc}").unwrap();
            }
        }

        // An Alloy `pred` is a formula, not a procedure (#82).
        let return_str = match &op.return_type {
            Some(rt) => format!(" -> {}", swift_return_type(&rt.type_name, &rt.mult)),
            None => " -> Bool".to_string(),
        };

        writeln!(out, "func {}({params}){return_str} {{", op.name).unwrap();
        {
            let env = crate::backend::type_env::operation_env(op);
            if op.body.is_empty() {
                writeln!(out, "    return true").unwrap();
            } else if op.return_type.is_some() {
                let body = expr_translator::translate_with_env(&op.body[0], ir, &env);
                writeln!(out, "    return {body}").unwrap();
            } else {
                let conjuncts: Vec<String> = op.body.iter()
                    .map(|e| expr_translator::translate_with_env(e, ir, &env))
                    .collect();
                writeln!(out, "    return {}", conjuncts.join(" && ")).unwrap();
            }
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── Tests.swift ──────────────────────────────────────────────────────────────

/// A sig lowered into an enum case is not a Swift type, so a test cannot
/// declare `[Case]` as its domain. ponytail: skip the test rather than
/// destructure every payload binding — tracked separately.
fn variant_domain<'a>(params: &'a [(String, String)], ctx: &SwiftContext) -> Option<&'a str> {
    params.iter().map(|(_, t)| t.as_str()).find(|t| ctx.is_variant(t))
}

/// A payload case that survived translation as a bare `Enum.case` is a
/// constructor in value position — invalid Swift. Skip the test rather than
/// emit it; rendering it properly needs payload destructuring.
fn unrenderable_case_ref(body: &str, refs: &[String]) -> Option<String> {
    // Match on an identifier boundary: `Expr.lit` is a prefix of the perfectly
    // valid `Expr.literal`.
    // Asymmetric on `.`: a dot *before* the match means it is the tail of a
    // longer path (`h.myExpr.lit` is not `Expr.lit`), but a dot *after* it just
    // starts a member access on the constructor (`Expr.lit.name` is).
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let left_ok = |c: Option<char>| c.is_none_or(|c| !ident(c) && c != '.');
    let right_ok = |c: Option<char>| c.is_none_or(|c| !ident(c));
    refs.iter().find(|r| {
        body.match_indices(r.as_str()).any(|(i, m)| {
            left_ok(body[..i].chars().next_back()) && right_ok(body[i + m.len()..].chars().next())
        })
    }).cloned()
}

fn generate_tests(ir: &OxidtrIR, ctx: &SwiftContext) -> String {
    let mut out = String::new();
    let fixture_types = crate::backend::collect_fixture_types(ir);
    let sig_names = expr_translator::collect_sig_names(ir);
    let case_refs = expr_translator::payload_case_refs(ir);

    writeln!(out, "import XCTest").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "final class PropertyTests: XCTestCase {{").unwrap();

    for prop in &ir.properties {
        let params = expr_translator::extract_params(&prop.expr, &sig_names, ir);
        if let Some(t) = variant_domain(&params, ctx) {
            writeln!(out, "    // oxidtr: skipped test_{} — `{t}` is an enum case, not a Swift type", prop.name).unwrap();
            continue;
        }
        let body = expr_translator::translate_with_ir(&prop.expr, ir);
        if let Some(r) = unrenderable_case_ref(&body, &case_refs) {
            writeln!(out, "    // oxidtr: skipped test_{} — `{r}` is a case constructor, not a value", prop.name).unwrap();
            continue;
        }

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
            writeln!(out, "    func test_{}() {{", prop.name).unwrap();
            writeln!(out, "        // {label}: full verification needs a trace; an empty trace").unwrap();
            writeln!(out, "        // can never satisfy it, which at least exercises the checker.").unwrap();
            match temporal_checker_name(&prop.name, &prop.expr, temporal_kind) {
                Some(checker) => {
                    let tname = params.first().map(|(_, t)| t.as_str()).unwrap_or("Never");
                    writeln!(out, "        let trace: [[{tname}]] = []").unwrap();
                    writeln!(out, "        XCTAssertFalse({checker}(trace: trace))").unwrap();
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

        writeln!(out, "    func test_{}() {{", prop.name).unwrap();
        for (pname, tname) in &params {
            // An empty domain makes `allSatisfy` vacuously true, so the test
            // passes whatever the implementation does (#81). Seed it from the
            // fixture wherever one exists, and disclose it where one does not.
            if fixture_types.contains(tname) {
                writeln!(out, "        let {pname}: [{tname}] = [default{tname}()]").unwrap();
            } else {
                writeln!(out, "        // @coverage empty domain: no fixture for `{tname}`;").unwrap();
                writeln!(out, "        // this quantifier is vacuously satisfied.").unwrap();
                writeln!(out, "        let {pname}: [{tname}] = []").unwrap();
            }
        }
        writeln!(out, "        XCTAssertTrue({body})").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
    }

    // Swift has strong null safety (T?) — skip tests for null-safety constraints
    let all_constraints = analyze::analyze(ir);
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };

        // Preflight before *any* branch below: the transition path returns
        // early, so a guard placed further down never sees it.
        if let Some(r) = unrenderable_case_ref(
            &expr_translator::translate_with_ir(&constraint.expr, ir), &case_refs)
        {
            writeln!(out, "    // oxidtr: skipped tests for {fact_name} — `{r}` is a case constructor, not a value").unwrap();
            continue;
        }

        // Alloy 6: temporal facts with prime → generate transition test
        if analyze::expr_contains_prime(&constraint.expr) {
            let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
            if let Some(t) = variant_domain(&params, ctx) {
                writeln!(out, "    // oxidtr: skipped test_transition_{fact_name} — `{t}` is an enum case, not a Swift type").unwrap();
                continue;
            }
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "    /// @temporal Transition constraint: {fact_name}").unwrap();
            writeln!(out, "    /// Verifies: pre→post state relationship ({desc})").unwrap();
            writeln!(out, "    func test_transition_{}() {{", fact_name).unwrap();
            for (pname, tname) in &params {
                writeln!(out, "        let {pname}: [{tname}] = []").unwrap();
                writeln!(out, "        let next_{pname}: [{tname}] = {pname}").unwrap();
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
                        writeln!(out, "        for (i, {v}) in {pname}.enumerated() {{").unwrap();
                        writeln!(out, "            let next_{v} = next_{pname}[i]").unwrap();
                        writeln!(out, "            XCTAssertTrue({body_str})").unwrap();
                        writeln!(out, "        }}").unwrap();
                    }
                    _ => {
                        writeln!(out, "        // oxidtr: skipped — a transition over {} binding(s) has no \
                            pre/post pairing to walk. See #104.", bind_vars.len()).unwrap();
                    }
                }
            } else {
                let rewritten = analyze::rewrite_prime_as_post_state(&constraint.expr);
                let body = expr_translator::translate_with_ir(&rewritten, ir);
                writeln!(out, "        XCTAssertTrue({body})").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
        if let Some(t) = variant_domain(&params, ctx) {
            writeln!(out, "    // oxidtr: skipped tests for {fact_name} — `{t}` is an enum case, not a Swift type").unwrap();
            continue;
        }
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);

        // Check if all related constraints are type-guaranteed in Swift
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
            can_guarantee_by_type(c, TargetLang::Swift) == Guarantee::FullyByType
        });

        if all_fully {
            writeln!(out, "    // Type-guaranteed: {} — Swift type system handles this", fact_name).unwrap();
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
            writeln!(out, "    /// @temporal {:?} constraint: {fact_name}{note}", kind).unwrap();
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
            let snake_name = to_snake_case(&fact_name);
            writeln!(out, "    func test_{}_{}() {{", test_prefix, fact_name).unwrap();
            writeln!(out, "        // binary temporal: requires trace-based verification; see check_{op_label}_{snake_name}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else if matches!(temporal_kind, Some(analyze::TemporalKind::Liveness) | Some(analyze::TemporalKind::PastLiveness)) {
            let kind_label = if temporal_kind == Some(analyze::TemporalKind::Liveness) {
                "liveness" } else { "past_liveness" };
            let snake_name = to_snake_case(&fact_name);
            writeln!(out, "    func test_{}_{}() {{", test_prefix, fact_name).unwrap();
            writeln!(out, "        // {kind_label}: requires trace-based verification; see check_{kind_label}_{snake_name}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        } else {
        writeln!(out, "    func test_{}_{}() {{", test_prefix, fact_name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        let {pname}: [{tname}] = []").unwrap();
        }
        writeln!(out, "        XCTAssertTrue({body})").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();
        } // end non-binary temporal

        emit_temporal_trace_checkers(&mut out, &fact_name, &constraint.expr, &params, &body, ir, temporal_kind);
    }

    // Boundary value tests
    for constraint in &ir.constraints {
        let fact_name = match &constraint.name {
            Some(name) => name.clone(),
            None => continue,
        };
        let params = expr_translator::extract_params(&constraint.expr, &sig_names, ir);
        if variant_domain(&params, ctx).is_some() { continue; }
        let body = expr_translator::translate_with_ir(&constraint.expr, ir);
        if unrenderable_case_ref(&body, &case_refs).is_some() { continue; }

        let has_boundary = params.iter().any(|(_, tname)| {
            ir.structures.iter().any(|s| {
                s.name == *tname && !s.is_enum && s.fields.iter().any(|f| {
                    matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                        && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                })
            })
        });

        if has_boundary {
            writeln!(out, "    func test_boundary_{}() {{", fact_name).unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "        let {pname}: [{tname}] = [boundary{tname}()]").unwrap();
                } else {
                    writeln!(out, "        let {pname}: [{tname}] = []").unwrap();
                }
            }
            writeln!(out, "        XCTAssertTrue({body})").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();

            writeln!(out, "    func test_invalid_{}() {{", fact_name).unwrap();
            for (pname, tname) in &params {
                let has_b = ir.structures.iter().any(|s| {
                    s.name == *tname && s.fields.iter().any(|f| {
                        matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                            && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
                    })
                });
                if has_b {
                    writeln!(out, "        let {pname}: [{tname}] = [invalid{tname}()]").unwrap();
                } else {
                    writeln!(out, "        let {pname}: [{tname}] = []").unwrap();
                }
            }
            writeln!(out, "        XCTAssertFalse(!({body}))").unwrap();
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
            let body = expr_translator::translate_with_ir(&constraint.expr, ir);
            for op in &ir.operations {
                writeln!(out, "    /// oxidtr: implement cross-test").unwrap();
                writeln!(out, "    func disabled_test_{fact_name}_preserved_after_{}() {{", op.name).unwrap();
                writeln!(out, "        // pre: XCTAssertTrue({body})").unwrap();
                writeln!(out, "        // {}(...)", op.name).unwrap();
                writeln!(out, "        // post: XCTAssertTrue({body})").unwrap();
                writeln!(out, "        XCTFail(\"oxidtr: implement cross-test\")").unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out).unwrap();
            }
        }
    }

    // --- Anomaly tests ---
    let anomalies = analyze::detect_anomalies(ir);
    let variant_names: HashSet<String> = ir.structures.iter()
        .filter(|s| s.parent.is_some())
        .map(|s| s.name.clone()).collect();
    let has_fixture: HashSet<String> = ir.structures.iter()
        .filter(|s| !variant_names.contains(&s.name) && !s.is_enum && !s.fields.is_empty())
        .map(|s| s.name.clone()).collect();
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
            let snake = to_snake_case(sig_name);
            for pattern in patterns {
                match pattern {
                    analyze::AnomalyPattern::UnconstrainedField { field_name, .. } => {
                        writeln!(out, "    func testAnomaly_{snake}_{field_name}_unconstrained() {{").unwrap();
                        writeln!(out, "        let instance = default{sig_name}()").unwrap();
                        writeln!(out, "        _ = instance.{field_name}").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        writeln!(out, "    func testAnomaly_{snake}_{field_name}_empty() {{").unwrap();
                        writeln!(out, "        let instance = anomalyEmpty{sig_name}()").unwrap();
                        writeln!(out, "        _ = instance.{field_name}").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        writeln!(out, "    func testAnomaly_{snake}_{field_name}_selfRef() {{").unwrap();
                        writeln!(out, "        let instance = default{sig_name}()").unwrap();
                        writeln!(out, "        _ = instance.{field_name}").unwrap();
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

        let mut cover_names_seen: HashSet<String> = HashSet::new();
        for pair in &coverage.pairwise {
            if !has_fixture.contains(&pair.sig_name) { continue; }

            let fact_a_snake = to_snake_case(&pair.fact_a);
            let fact_b_snake = to_snake_case(&pair.fact_b);
            let test_name = format!("testCover_{fact_a_snake}_x_{fact_b_snake}");

            // Skip duplicate test names (same fact pair from different sig perspectives)
            if !cover_names_seen.insert(test_name.clone()) { continue; }

            // Find the constraint nodes for both facts
            let constraint_a = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_a));
            let constraint_b = ir.constraints.iter()
                .find(|c| c.name.as_deref() == Some(&pair.fact_b));

            let (Some(ca), Some(cb)) = (constraint_a, constraint_b) else { continue; };

            // Extract all params from both facts to declare all needed variables
            let params_a = expr_translator::extract_params(&ca.expr, &sig_names, ir);
            let params_b = expr_translator::extract_params(&cb.expr, &sig_names, ir);
            if variant_domain(&params_a, ctx).is_some() || variant_domain(&params_b, ctx).is_some() {
                continue;
            }
            let mut all_params: Vec<(String, String)> = Vec::new();
            let mut param_names_seen: HashSet<String> = HashSet::new();
            for (pname, tname) in params_a.iter().chain(params_b.iter()) {
                if param_names_seen.insert(pname.clone()) {
                    all_params.push((pname.clone(), tname.clone()));
                }
            }

            writeln!(out, "    /// Coverage: {} × {}", pair.fact_a, pair.fact_b).unwrap();
            writeln!(out, "    func {test_name}() {{").unwrap();
            for (pname, tname) in &all_params {
                if has_fixture.contains(tname) {
                    writeln!(out, "        let {pname}: [{}] = [default{tname}()]", tname).unwrap();
                } else {
                    writeln!(out, "        let {pname}: [{}] = []", tname).unwrap();
                }
            }
            writeln!(out, "        // TODO: pairwise coverage – add assertions when coverage strategy is finalized").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

// ── Fixtures.swift ───────────────────────────────────────────────────────────

fn generate_fixtures(ir: &OxidtrIR, ctx: &SwiftContext) -> String {
    let mut out = String::new();
    writeln!(out, "import Foundation").unwrap();
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
            // A case carries the abstract parent's fields *plus* its own, so a
            // variant with no fields of its own is still not a unit case.
            let payload_of = |v: &str| -> Vec<IRField> {
                let own = ctx.struct_map.get(v).map(|st| st.fields.clone()).unwrap_or_default();
                s.fields.iter().cloned().chain(own).collect()
            };
            // Prefer a unit case, else the first whose payload can actually be
            // built. Picking a case whose factory re-enters this enum produces
            // code that compiles and then overflows the stack.
            let variant = variants.iter().find(|v| payload_of(v).is_empty())
                .or_else(|| ctx.enum_witness.get(&s.name).and_then(|w| variants.iter().find(|v| *v == w)));

            writeln!(out, "/// Factory: default value for {}", s.name).unwrap();
            match variant {
                Some(variant) => {
                    let payload = payload_of(variant);
                    let args = if payload.is_empty() {
                        String::new()
                    } else {
                        let list = payload.iter().map(|f| {
                            let val = if f.value_type.is_some() {
                                "[:]".to_string()
                            } else {
                                swift_default_value(&f.target, &f.mult)
                            };
                            format!("{}: {val}", to_swift_arg_label(&f.name))
                        }).collect::<Vec<_>>().join(", ");
                        format!("({list})")
                    };
                    writeln!(out, "func default{0}() -> {0} {{ .{1}{2} }}", s.name, to_swift_case_name(variant), args).unwrap();
                }
                // Every case re-enters this enum: no finite value exists. Trap
                // instead of emitting a factory that recurses forever.
                None => writeln!(out, "func default{0}() -> {0} {{ {1} }}", s.name, no_finite_default(&s.name)).unwrap(),
            }
            writeln!(out).unwrap();
        }
    }

    // A field declared to hold a variant takes the parent's type (#93), but the
    // variant it was declared as is still what the fixture should build — and
    // `default{Variant}()` has to exist for the call site to resolve. Emit one
    // that returns the parent, constructing exactly that case.
    {
        let mut needed: Vec<String> = crate::backend::variants_used_as_field_targets(ir).into_iter().collect();
        needed.sort();
        for variant in needed {
            let Some(vs) = ir.structures.iter().find(|s| s.name == variant) else { continue };
            let Some(parent) = variant_parent(ir, &variant) else { continue };
            let parent_fields: Vec<&IRField> = ir.structures.iter()
                .find(|p| p.name == parent)
                .map(|p| p.fields.iter().collect())
                .unwrap_or_default();
            let fields: Vec<&IRField> = parent_fields.into_iter().chain(vs.fields.iter()).collect();
            let args = if fields.is_empty() {
                String::new()
            } else {
                let list = fields.iter().map(|f| {
                    let val = if f.value_type.is_some() {
                        "[:]".to_string()
                    } else {
                        swift_default_value(&f.target, &f.mult)
                    };
                    format!("{}: {val}", to_swift_arg_label(&f.name))
                }).collect::<Vec<_>>().join(", ");
                format!("({list})")
            };
            writeln!(out, "/// Factory: `{parent}` as the `{variant}` case it was declared as.").unwrap();
            writeln!(out, "func default{variant}() -> {parent} {{ .{}{args} }}", to_swift_case_name(&variant)).unwrap();
            writeln!(out).unwrap();
        }
    }

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum { continue; }
        if is_native_type_alias(&s.name) { continue; }
        if s.fields.is_empty() {
            // A `one Val` field still calls defaultVal(), so unit sigs need one.
            writeln!(out, "/// Factory: default value for unit sig {}", s.name).unwrap();
            writeln!(out, "func default{0}() -> {0} {{ {0}() }}", s.name).unwrap();
            writeln!(out).unwrap();
            continue;
        }

        writeln!(out, "/// Factory: create a default valid {}", s.name).unwrap();
        if !ctx.terminating.contains(&s.name) {
            writeln!(out, "func default{0}() -> {0} {{ {1} }}", s.name, no_finite_default(&s.name)).unwrap();
            writeln!(out).unwrap();
            continue;
        }
        writeln!(out, "func default{}() -> {} {{", s.name, s.name).unwrap();
        writeln!(out, "    {}(", s.name).unwrap();
        for (i, f) in s.fields.iter().enumerate() {
            let val = if f.value_type.is_some() {
                "[:]".to_string()
            } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && super::is_safe_set_population(&s.name, &f.target, ir, &fixture_types)
                && ctx.terminating.contains(&f.target) {
                let safe = HashSet::from([f.target.clone()]);
                swift_default_value_inner(&f.target, &f.mult, &safe)
            } else {
                swift_default_value(&f.target, &f.mult)
            };
            let comma = if i < s.fields.len() - 1 { "," } else { "" };
            writeln!(out, "        {}: {val}{comma}", to_swift_arg_label(&f.name)).unwrap();
        }
        writeln!(out, "    )").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Boundary value fixtures
        let has_bounds = s.fields.iter().any(|f| {
            matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && analyze::bounds_for_field(ir, &s.name, &f.name).is_some()
        });
        if has_bounds {
            writeln!(out, "/// Factory: create {} at cardinality boundary", s.name).unwrap();
            writeln!(out, "func boundary{}() -> {} {{", s.name, s.name).unwrap();
            writeln!(out, "    {}(", s.name).unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = if f.value_type.is_some() {
                    "[:]".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let count = match &bound {
                            analyze::BoundKind::Exact(n) => *n,
                            analyze::BoundKind::AtMost(n) => *n,
                            analyze::BoundKind::AtLeast(n) => *n,
                        };
                        swift_boundary_value(ir, &f.target, &f.mult, count)
                    } else {
                        swift_default_value(&f.target, &f.mult)
                    }
                } else {
                    swift_default_value(&f.target, &f.mult)
                };
                writeln!(out, "        {}: {val}{comma}", to_swift_arg_label(&f.name)).unwrap();
            }
            writeln!(out, "    )").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();

            writeln!(out, "/// Factory: create {} that violates cardinality constraint", s.name).unwrap();
            writeln!(out, "func invalid{}() -> {} {{", s.name, s.name).unwrap();
            writeln!(out, "    {}(", s.name).unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = if f.value_type.is_some() {
                    "[:]".to_string()
                } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq) {
                    if let Some(bound) = analyze::bounds_for_field(ir, &s.name, &f.name) {
                        let violation = match &bound {
                            analyze::BoundKind::Exact(n) => n + 1,
                            analyze::BoundKind::AtMost(n) => n + 1,
                            analyze::BoundKind::AtLeast(n) => if *n > 0 { n - 1 } else { 0 },
                        };
                        swift_boundary_value(ir, &f.target, &f.mult, violation)
                    } else {
                        swift_default_value(&f.target, &f.mult)
                    }
                } else {
                    swift_default_value(&f.target, &f.mult)
                };
                writeln!(out, "        {}: {val}{comma}", to_swift_arg_label(&f.name)).unwrap();
            }
            writeln!(out, "    )").unwrap();
            writeln!(out, "}}").unwrap();
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

            let _snake = to_snake_case(sig_name);
            writeln!(out, "/// Anomaly fixture: all collections empty").unwrap();
            writeln!(out, "func anomalyEmpty{sig_name}() -> {sig_name} {{").unwrap();
            writeln!(out, "    {sig_name}(").unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = match &f.mult {
                    Multiplicity::Set => "Set()".to_string(),
                    Multiplicity::Seq => "[]".to_string(),
                    _ => swift_default_value(&f.target, &f.mult),
                };
                writeln!(out, "        {}: {}{}", to_swift_arg_label(&f.name), val, comma).unwrap();
            }
            writeln!(out, "    )").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

/// A native-alias literal that differs per index. `Bool` is not among them:
/// two values cannot carry a cardinality of three.
fn swift_native_element(target: &str, i: usize) -> Option<String> {
    match target {
        "Int" => Some(i.to_string()),
        "Float" => Some(format!("{i}.0")),
        "Str" => Some(format!("\"item{i}\"")),
        _ => None,
    }
}

/// `count` elements of `target`, each distinct from the others.
///
/// #92 varied native elements, but a struct element still fell back to
/// `default{target}()` repeated — and `Set` deduplicates structurally
/// identical values, so the fixture never reached the cardinality it was
/// named for (#96). A struct varies its first native-scalar field instead;
/// Swift has no `..default` update syntax, so the whole memberwise
/// initializer is written out with the other fields at their defaults.
///
/// A target offering nothing to vary still repeats: there is no second value
/// to give it without inventing a field the sig does not have.
fn swift_distinct_elements(ir: &OxidtrIR, target: &str, count: usize) -> Vec<String> {
    let fallback = || vec![format!("default{target}()"); count];
    if swift_native_element(target, 0).is_some() {
        return (0..count).filter_map(|i| swift_native_element(target, i)).collect();
    }
    if is_native_type_alias(target) {
        // `Bool`, which has no per-index literal.
        return fallback();
    }
    let Some(s) = ir.structures.iter().find(|st| st.name == target) else {
        return fallback();
    };
    let scalar = s.fields.iter().position(|f| {
        f.value_type.is_none()
            && f.mult == Multiplicity::One
            && swift_native_element(&f.target, 0).is_some()
    });
    let Some(idx) = scalar else { return fallback() };

    (0..count)
        .map(|i| {
            let args: Vec<String> = s.fields.iter().enumerate()
                .map(|(j, f)| {
                    let val = if j == idx {
                        swift_native_element(&f.target, i).unwrap()
                    } else if f.value_type.is_some() {
                        "[:]".to_string()
                    } else {
                        swift_default_value(&f.target, &f.mult)
                    };
                    format!("{}: {val}", to_swift_arg_label(&f.name))
                })
                .collect();
            format!("{target}({})", args.join(", "))
        })
        .collect()
}

fn swift_boundary_value(ir: &OxidtrIR, target: &str, mult: &Multiplicity, count: usize) -> String {
    let items = swift_distinct_elements(ir, target, count);
    match mult {
        Multiplicity::Set if items.is_empty() => "Set()".to_string(),
        Multiplicity::Set => format!("Set([{}])", items.join(", ")),
        Multiplicity::Seq if items.is_empty() => "[]".to_string(),
        Multiplicity::Seq => format!("[{}]", items.join(", ")),
        _ => swift_default_value(target, mult),
    }
}

fn swift_return_type(type_name: &str, mult: &Multiplicity) -> String {
    // `Int`/`Str`/`Bool` are Alloy marker sigs, not emitted types.
    let type_name = &if is_native_type_alias(type_name) {
        resolve_type(TargetLang::Swift, type_name)
    } else {
        type_name.to_string()
    };
    match mult {
        Multiplicity::One => type_name.to_string(),
        Multiplicity::Lone => format!("{type_name}?"),
        Multiplicity::Set => format!("Set<{type_name}>"),
        Multiplicity::Seq => format!("[{type_name}]"),
    }
}

fn swift_default_value(target: &str, mult: &Multiplicity) -> String {
    swift_default_value_inner(target, mult, &HashSet::new())
}

fn no_finite_default(name: &str) -> String {
    format!("fatalError(\"oxidtr: {name} has no finite default \\u{{2014}} every value of it contains another\")")
}

/// Types whose `default{T}()` provably terminates, as a least fixed point:
/// start with nothing constructible and keep adding types all of whose `one`
/// fields are already constructible. A `lone` field bottoms out at `nil` and a
/// set/seq at whatever `is_safe_set_population` decides, so neither is an edge
/// here. An enum is constructible as soon as one of its cases is.
///
/// Computed once rather than walked per query: a recursive walk with a
/// visiting stack is both exponential on a wide DAG and inconsistent, since
/// the answer would depend on where the walk started.
/// Also records, per enum, the case that *made* it constructible — the one
/// whose payload was already satisfiable when the enum was admitted. Selection
/// cannot re-derive this from the finished set: once `Expr` is known
/// constructible, a self-recursive case like `.loop(expr: defaultExpr())`
/// looks satisfiable too.
fn find_terminating_types(
    ir: &OxidtrIR,
    children: &HashMap<String, Vec<String>>,
    variant_names: &HashSet<String>,
) -> (HashSet<String>, HashMap<String, String>) {
    let mut done: HashSet<String> = HashSet::new();
    let mut witness: HashMap<String, String> = HashMap::new();
    let edge_ok = |f: &IRField, done: &HashSet<String>| {
        // A field declared to hold a variant takes the parent's type (#93), so
        // constructibility is the parent's — a variant is never in `done`, and
        // treating it as its own type made every holder look non-constructible.
        let target = variant_parent(ir, &f.target).unwrap_or_else(|| f.target.clone());
        f.value_type.is_some()
            || f.mult != Multiplicity::One
            || swift_native_default(&target).is_some()
            || done.contains(&target)
            // A target with no structure of its own has nothing to recurse into.
            || !ir.structures.iter().any(|s| s.name == target)
    };

    loop {
        let mut changed = false;
        for s in &ir.structures {
            if done.contains(&s.name) || variant_names.contains(&s.name) { continue; }
            let ok = if s.is_enum {
                let found = children.get(&s.name).and_then(|vs| vs.iter().find(|v| {
                    let own = ir.structures.iter().find(|c| &c.name == *v);
                    s.fields.iter()
                        .chain(own.into_iter().flat_map(|c| c.fields.iter()))
                        .all(|f| edge_ok(f, &done))
                }));
                if let Some(v) = found { witness.insert(s.name.clone(), v.clone()); }
                found.is_some()
            } else {
                s.fields.iter().all(|f| edge_ok(f, &done))
            };
            if ok {
                done.insert(s.name.clone());
                changed = true;
            }
        }
        if !changed { return (done, witness); }
    }
}

/// Native aliases have no generated factory — they need a literal.
fn swift_native_default(alloy_name: &str) -> Option<&'static str> {
    match alloy_name {
        "Str" => Some("\"\""),
        "Int" => Some("0"),
        "Float" => Some("0.0"),
        "Bool" => Some("false"),
        _ => None,
    }
}

fn swift_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    let element = || match swift_native_default(target) {
        Some(lit) => lit.to_string(),
        None => format!("default{target}()"),
    };
    match mult {
        Multiplicity::Lone => "nil".to_string(),
        Multiplicity::Set => {
            if safe_targets.contains(target) {
                format!("Set([{}])", element())
            } else {
                "Set()".to_string()
            }
        }
        Multiplicity::Seq => {
            if safe_targets.contains(target) {
                format!("[{}]", element())
            } else {
                "[]".to_string()
            }
        }
        Multiplicity::One => element(),
    }
}

// ── Naming helpers ───────────────────────────────────────────────────────────

/// Argument labels may be keywords bare — escaping one is a warning, not an
/// error. Only `inout`/`var`/`let` are actually rejected in label position.
fn to_swift_arg_label(name: &str) -> String {
    if matches!(name, "inout" | "var" | "let") {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

pub(crate) fn to_swift_field_name(name: &str) -> String {
    // Swift uses camelCase for properties — Alloy field names are already camelCase
    escape_swift_keyword(name)
}

pub(crate) fn to_swift_case_name(name: &str) -> String {
    // Enum case names in Swift are lowerCamelCase
    let mut chars = name.chars();
    let lowered = match chars.next() {
        None => String::new(),
        Some(c) => format!("{}{}", c.to_lowercase(), chars.as_str()),
    };
    escape_swift_keyword(&lowered)
}

/// Swift reserved words that are legal Alloy identifiers. Backticks make any
/// of them usable as a declaration or reference.
/// Strictly the *reserved* words — contextual keywords (`get`, `left`, `final`,
/// `any`, …) are legal identifiers and must not be escaped, or the generated
/// names stop round-tripping through `extract`.
const SWIFT_KEYWORDS: &[&str] = &[
    "Any", "Protocol", "Self", "Type", "as", "associatedtype", "borrowing", "break", "case", "catch", "class",
    "consuming", "continue", "default", "defer", "deinit", "do", "else", "enum", "extension",
    "fallthrough", "false", "fileprivate", "for", "func", "guard", "if", "import", "in", "init",
    "inout", "internal", "is", "let", "macro", "nil", "operator", "package", "precedencegroup",
    "private", "protocol", "public", "repeat", "rethrows", "return", "self", "static", "struct",
    "subscript", "super", "switch", "throw", "throws", "true", "try", "typealias", "var", "where",
    "while",
];

fn escape_swift_keyword(name: &str) -> String {
    if SWIFT_KEYWORDS.contains(&name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
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

/// Translate an Alloy expression to Swift for single-instance validator context.
fn translate_validator_expr_swift(expr: &crate::parser::ast::Expr, sig_name: &str) -> String {
    use crate::parser::ast::{Expr, LogicOp, QuantKind};
    match expr {
        Expr::VarRef(name) => {
            if name == sig_name { "self".to_string() } else { name.clone() }
        }
        Expr::IntLiteral(n) => n.to_string(),
        Expr::FieldAccess { base, field } => {
            format!("{}.{}", translate_validator_expr_swift(base, sig_name), to_swift_field_name(field))
        }
        Expr::Comparison { op, left, right } => {
            let l = translate_validator_expr_swift(left, sig_name);
            let r = translate_validator_expr_swift(right, sig_name);
            let o = match op {
                CompareOp::Eq => "==",
                CompareOp::NotEq => "!=",
                CompareOp::In => return format!("{r}.contains({l})"),
                CompareOp::Lt => "<",
                CompareOp::Gt => ">",
                CompareOp::Lte => "<=",
                CompareOp::Gte => ">=",
            };
            format!("{l} {o} {r}")
        }
        Expr::BinaryLogic { op, left, right } => {
            let l = translate_validator_expr_swift(left, sig_name);
            let r = translate_validator_expr_swift(right, sig_name);
            match op {
                LogicOp::And => format!("{l} && {r}"),
                LogicOp::Or => format!("{l} || {r}"),
                LogicOp::Implies => format!("!({l}) || {r}"),
                LogicOp::Iff => format!("({l}) == ({r})"),
            }
        }
        Expr::Not(inner) => format!("!({})", translate_validator_expr_swift(inner, sig_name)),
        Expr::MultFormula { kind, expr: inner } => {
            let e = translate_validator_expr_swift(inner, sig_name);
            match kind {
                QuantKind::Some => format!("{e} != nil"),
                QuantKind::No => format!("{e} == nil"),
                _ => e,
            }
        }
        Expr::Cardinality(inner) => {
            format!("{}.count", translate_validator_expr_swift(inner, sig_name))
        }
        _ => analyze::describe_expr(expr), // fallback: human-readable
    }
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
    let _ = (&constraint, body);
    // Generate trace checker functions for temporal constraints
    if let Some(kind) = temporal_kind {
        let snake_name = to_snake_case(name);
        match kind {
            analyze::TemporalKind::Liveness | analyze::TemporalKind::PastLiveness => {
                let kind_label = if kind == analyze::TemporalKind::Liveness {
                    "liveness" } else { "past_liveness" };
                let semantics = if kind == analyze::TemporalKind::Liveness {
                    "property holds in at least one future state"
                } else {
                    "property held in at least one past state"
                };
                writeln!(out, "    /// Trace checker for {kind_label}: {semantics}.").unwrap();
                if params.len() == 1 {
                    let (pname, tname) = &params[0];
                    writeln!(out, "    func check_{kind_label}_{snake_name}(trace: [[{tname}]]) -> Bool {{").unwrap();
                    writeln!(out, "        trace.contains {{ {pname} in").unwrap();
                } else {
                    let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("[{t}]")).collect();
                    let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                    writeln!(out, "    func check_{kind_label}_{snake_name}(trace: [({})]) -> Bool {{", tuple_types.join(", ")).unwrap();
                    writeln!(out, "        trace.contains {{ ({}) in", tuple_names.join(", ")).unwrap();
                }
                writeln!(out, "            {body}").unwrap();
                writeln!(out, "        }}").unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out).unwrap();
            }
            analyze::TemporalKind::Binary => {
                if let Some((op, left, right)) = analyze::find_temporal_binary(&constraint.expr) {
                    let left_body = expr_translator::translate_with_ir(left, ir);
                    let right_body = expr_translator::translate_with_ir(right, ir);
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
                    writeln!(out, "    /// Trace checker for {op_name}: {semantics}.").unwrap();
                    if params.len() == 1 {
                        let (pname, tname) = &params[0];
                        writeln!(out, "    func check_{op_name}_{snake_name}(trace: [[{tname}]]) -> Bool {{").unwrap();
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "        guard let pos = trace.firstIndex(where: {{ {pname} in {right_body} }}) else {{ return false }}").unwrap();
                                writeln!(out, "        return trace.prefix(pos).allSatisfy {{ {pname} in {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "        guard let pos = trace.lastIndex(where: {{ {pname} in {right_body} }}) else {{ return false }}").unwrap();
                                writeln!(out, "        return trace.suffix(from: pos).allSatisfy {{ {pname} in {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "        if let pos = trace.firstIndex(where: {{ {pname} in {left_body} }}) {{").unwrap();
                                writeln!(out, "            return trace.prefix(through: pos).allSatisfy {{ {pname} in {right_body} }}").unwrap();
                                writeln!(out, "        }} else {{").unwrap();
                                writeln!(out, "            return trace.allSatisfy {{ {pname} in {right_body} }}").unwrap();
                                writeln!(out, "        }}").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "        return trace.enumerated().allSatisfy {{ (i, {pname}) in").unwrap();
                                writeln!(out, "            if {right_body} {{ return trace.prefix(through: i).contains {{ {pname} in {left_body} }} }} else {{ return true }}").unwrap();
                                writeln!(out, "        }}").unwrap();
                            }
                        }
                    } else {
                        let tuple_types: Vec<_> = params.iter().map(|(_, t)| format!("[{t}]")).collect();
                        let tuple_names: Vec<_> = params.iter().map(|(p, _)| p.as_str()).collect();
                        let pnames = tuple_names.join(", ");
                        writeln!(out, "    func check_{op_name}_{snake_name}(trace: [({})]) -> Bool {{", tuple_types.join(", ")).unwrap();
                        match op {
                            TemporalBinaryOp::Until => {
                                writeln!(out, "        guard let pos = trace.firstIndex(where: {{ ({pnames}) in {right_body} }}) else {{ return false }}").unwrap();
                                writeln!(out, "        return trace.prefix(pos).allSatisfy {{ ({pnames}) in {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Since => {
                                writeln!(out, "        guard let pos = trace.lastIndex(where: {{ ({pnames}) in {right_body} }}) else {{ return false }}").unwrap();
                                writeln!(out, "        return trace.suffix(from: pos).allSatisfy {{ ({pnames}) in {left_body} }}").unwrap();
                            }
                            TemporalBinaryOp::Release => {
                                writeln!(out, "        if let pos = trace.firstIndex(where: {{ ({pnames}) in {left_body} }}) {{").unwrap();
                                writeln!(out, "            return trace.prefix(through: pos).allSatisfy {{ ({pnames}) in {right_body} }}").unwrap();
                                writeln!(out, "        }} else {{").unwrap();
                                writeln!(out, "            return trace.allSatisfy {{ ({pnames}) in {right_body} }}").unwrap();
                                writeln!(out, "        }}").unwrap();
                            }
                            TemporalBinaryOp::Triggered => {
                                writeln!(out, "        return trace.enumerated().allSatisfy {{ (i, ({pnames})) in").unwrap();
                                writeln!(out, "            if {right_body} {{ return trace.prefix(through: i).contains {{ ({pnames}) in {left_body} }} }} else {{ return true }}").unwrap();
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

/// The name `emit_temporal_trace_checkers` will give this constraint's checker,
/// so the generated test can call it rather than leaving it unreferenced.
fn temporal_checker_name(
    name: &str,
    expr: &crate::parser::ast::Expr,
    kind: Option<analyze::TemporalKind>,
) -> Option<String> {
    let snake = to_snake_case(name);
    match kind {
        Some(analyze::TemporalKind::Binary) => {
            let (op, _, _, _) = analyze::find_temporal_binary_with_bindings(expr)?;
            let op_label = match op {
                TemporalBinaryOp::Until => "until",
                TemporalBinaryOp::Since => "since",
                TemporalBinaryOp::Release => "release",
                TemporalBinaryOp::Triggered => "triggered",
            };
            Some(format!("check_{op_label}_{snake}"))
        }
        Some(analyze::TemporalKind::PastLiveness) => Some(format!("check_past_liveness_{snake}")),
        Some(analyze::TemporalKind::Liveness) => Some(format!("check_liveness_{snake}")),
        _ => None,
    }
}
