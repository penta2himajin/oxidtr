pub mod expr_translator;

use crate::backend::GeneratedFile;
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

    let has_tc = ir.constraints.iter().any(|c| expr_uses_tc(&c.expr))
        || ir.properties.iter().any(|p| expr_uses_tc(&p.expr));

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
            content: generate_tests(ir),
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
    cyclic_fields: HashSet<(String, String)>,
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
        let variant_names: HashSet<String> = ir.structures.iter()
            .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
            .map(|s| s.name.clone()).collect();
        let struct_map: HashMap<String, StructureNode> = ir.structures.iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect();
        let cyclic_fields = find_cyclic_fields(ir);
        SwiftContext { children, variant_names, struct_map, cyclic_fields }
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
    // Also check indirect cycles via BFS
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

// ── Models.swift ─────────────────────────────────────────────────────────────

fn generate_models(ir: &OxidtrIR, ctx: &SwiftContext) -> String {
    let mut out = String::new();
    writeln!(out, "import Foundation").unwrap();
    writeln!(out).unwrap();

    let disj_fields = analyze::disj_fields(ir);

    for s in &ir.structures {
        if ctx.is_variant(&s.name) { continue; }

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

    // Derived fields: receiver functions → extensions
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
        writeln!(out, "extension {sig_name} {{").unwrap();
        for op in ops {
            let return_str = match &op.return_type {
                Some(rt) => swift_return_type(&rt.type_name, &rt.mult),
                None => "Void".to_string(),
            };

            if op.params.is_empty() {
                // No params → computed property
                writeln!(out, "    var {}: {return_str} {{", op.name).unwrap();
                writeln!(out, "        fatalError(\"oxidtr: implement {}\")", op.name).unwrap();
                writeln!(out, "    }}").unwrap();
            } else {
                let params = op.params.iter().map(|p| {
                    let type_str = swift_return_type(&p.type_name, &p.mult);
                    format!("{}: {type_str}", p.name)
                }).collect::<Vec<_>>().join(", ");
                writeln!(out, "    func {}({params}) -> {return_str} {{", op.name).unwrap();
                writeln!(out, "        fatalError(\"oxidtr: implement {}\")", op.name).unwrap();
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
        writeln!(out, "struct {} {{", s.name).unwrap();
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
        writeln!(out, "struct {}: Equatable {{", s.name).unwrap();
        for f in &s.fields {
            let type_str = if let Some(vt) = &f.value_type {
                format!("[{}: {}]", f.target, vt)
            } else {
                mult_to_swift_type(&f.target, &f.mult, ctx.cyclic_fields.contains(&(s.name.clone(), f.name.clone())))
            };

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
        // Enum with associated values
        writeln!(out, "enum {}: Equatable {{", s.name).unwrap();
        if let Some(variants) = variants {
            for v in variants {
                let child = ctx.struct_map.get(v.as_str());
                let child_fields: Vec<&IRField> = child.map(|c| c.fields.iter().collect()).unwrap_or_default();
                // Combine parent fields + child fields
                let all_fields: Vec<&IRField> = parent_fields.iter().chain(child_fields.iter().copied()).collect();
                if !all_fields.is_empty() {
                    let params: Vec<String> = all_fields.iter().map(|f| {
                        let type_str = if let Some(vt) = &f.value_type {
                            format!("[{}: {}]", f.target, vt)
                        } else {
                            mult_to_swift_type(&f.target, &f.mult, false)
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

fn mult_to_swift_type(target: &str, mult: &Multiplicity, is_indirect: bool) -> String {
    let base = match mult {
        Multiplicity::One => {
            if is_indirect {
                // Would need indirect/Box equivalent — use class or indirect enum
                target.to_string()
            } else {
                target.to_string()
            }
        }
        Multiplicity::Lone => format!("{target}?"),
        Multiplicity::Set => format!("Set<{target}>"),
        Multiplicity::Seq => format!("[{target}]"),
    };
    base
}

// ── Helpers.swift ────────────────────────────────────────────────────────────

fn generate_helpers(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "import Foundation").unwrap();
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

        let return_str = match &op.return_type {
            Some(rt) => format!(" -> {}", swift_return_type(&rt.type_name, &rt.mult)),
            None => String::new(),
        };

        writeln!(out, "func {}({params}){return_str} {{", op.name).unwrap();
        writeln!(out, "    fatalError(\"oxidtr: implement {}\")", op.name).unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── Tests.swift ──────────────────────────────────────────────────────────────

fn generate_tests(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    let sig_names = expr_translator::collect_sig_names(ir);

    writeln!(out, "import XCTest").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "final class PropertyTests: XCTestCase {{").unwrap();

    for prop in &ir.properties {
        let params = expr_translator::extract_params(&prop.expr, &sig_names);
        let body = expr_translator::translate_with_ir(&prop.expr, ir);

        writeln!(out, "    func test_{}() {{", prop.name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        let {pname}: [{tname}] = []").unwrap();
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

        // Alloy 6: temporal facts with prime → generate scaffold test
        // Prime references (x') require before/after state capture; emit scaffold.
        if analyze::expr_contains_prime(&constraint.expr) {
            let params = expr_translator::extract_params(&constraint.expr, &sig_names);
            let desc = analyze::describe_expr(&constraint.expr);

            writeln!(out, "    /// @temporal Transition constraint: {fact_name}").unwrap();
            writeln!(out, "    /// Scaffold: prime (next-state) references require a before/after transition mechanism.").unwrap();
            writeln!(out, "    func test_transition_{}() {{", fact_name).unwrap();
            writeln!(out, "        // TODO: apply transition, then assert post-condition").unwrap();
            writeln!(out, "        // Alloy constraint: {desc}").unwrap();
            for (pname, tname) in &params {
                writeln!(out, "        // pre: capture {pname}: [{tname}] before transition").unwrap();
                writeln!(out, "        // post: assert condition on {pname} after transition").unwrap();
            }
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
            continue;
        }

        let params = expr_translator::extract_params(&constraint.expr, &sig_names);
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

        // Generate trace checker functions for temporal constraints
        if let Some(kind) = temporal_kind {
            let snake_name = to_snake_case(&fact_name);
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
            let snake = to_snake_case(sig_name);
            for pattern in patterns {
                match pattern {
                    analyze::AnomalyPattern::UnconstrainedField { field_name, .. } => {
                        writeln!(out, "    func testAnomaly_{snake}_{field_name}_unconstrained() {{").unwrap();
                        writeln!(out, "        let instance = Fixtures.default{sig_name}()").unwrap();
                        writeln!(out, "        _ = instance.{field_name}").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnboundedCollection { field_name, .. } => {
                        writeln!(out, "    func testAnomaly_{snake}_{field_name}_empty() {{").unwrap();
                        writeln!(out, "        let instance = Fixtures.anomalyEmpty{sig_name}()").unwrap();
                        writeln!(out, "        _ = instance.{field_name}").unwrap();
                        writeln!(out, "    }}").unwrap();
                        writeln!(out).unwrap();
                    }
                    analyze::AnomalyPattern::UnguardedSelfRef { field_name, .. } => {
                        writeln!(out, "    func testAnomaly_{snake}_{field_name}_selfRef() {{").unwrap();
                        writeln!(out, "        let instance = Fixtures.default{sig_name}()").unwrap();
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
            let params_a = expr_translator::extract_params(&ca.expr, &sig_names);
            let params_b = expr_translator::extract_params(&cb.expr, &sig_names);
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
                    writeln!(out, "        let {pname}: [{}] = [Fixtures.default{tname}()]", tname).unwrap();
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
            let first_unit = variants.iter().find(|v| {
                ctx.struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
            });
            if let Some(variant) = first_unit {
                let all_unit = variants.iter().all(|v| {
                    ctx.struct_map.get(v.as_str()).map_or(true, |st| st.fields.is_empty())
                });
                if all_unit {
                    writeln!(out, "/// Factory: default value for {}", s.name).unwrap();
                    writeln!(out, "func default{}() -> {} {{ .{} }}", s.name, s.name, to_swift_case_name(variant)).unwrap();
                    writeln!(out).unwrap();
                } else {
                    let has_fields = ctx.struct_map.get(variant.as_str())
                        .map_or(false, |st| !st.fields.is_empty());
                    if !has_fields {
                        writeln!(out, "/// Factory: default value for {}", s.name).unwrap();
                        writeln!(out, "func default{}() -> {} {{ .{} }}", s.name, s.name, to_swift_case_name(variant)).unwrap();
                        writeln!(out).unwrap();
                    }
                }
            }
        }
    }

    for s in &ir.structures {
        if ctx.is_variant(&s.name) || s.is_enum { continue; }
        if s.fields.is_empty() { continue; }

        writeln!(out, "/// Factory: create a default valid {}", s.name).unwrap();
        writeln!(out, "func default{}() -> {} {{", s.name, s.name).unwrap();
        writeln!(out, "    {}(", s.name).unwrap();
        for (i, f) in s.fields.iter().enumerate() {
            let val = if f.value_type.is_some() {
                "[:]".to_string()
            } else if matches!(f.mult, Multiplicity::Set | Multiplicity::Seq)
                && super::is_safe_set_population(&s.name, &f.target, ir, &fixture_types) {
                let safe = HashSet::from([f.target.clone()]);
                swift_default_value_inner(&f.target, &f.mult, &safe)
            } else {
                swift_default_value(&f.target, &f.mult)
            };
            let comma = if i < s.fields.len() - 1 { "," } else { "" };
            writeln!(out, "        {}: {val}{comma}", to_swift_field_name(&f.name)).unwrap();
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
                        swift_boundary_value(&f.target, &f.mult, count)
                    } else {
                        swift_default_value(&f.target, &f.mult)
                    }
                } else {
                    swift_default_value(&f.target, &f.mult)
                };
                writeln!(out, "        {}: {val}{comma}", to_swift_field_name(&f.name)).unwrap();
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
                        swift_boundary_value(&f.target, &f.mult, violation)
                    } else {
                        swift_default_value(&f.target, &f.mult)
                    }
                } else {
                    swift_default_value(&f.target, &f.mult)
                };
                writeln!(out, "        {}: {val}{comma}", to_swift_field_name(&f.name)).unwrap();
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
            writeln!(out, "static func anomalyEmpty{sig_name}() -> {sig_name} {{").unwrap();
            writeln!(out, "    {sig_name}(").unwrap();
            for (i, f) in s.fields.iter().enumerate() {
                let comma = if i < s.fields.len() - 1 { "," } else { "" };
                let val = match &f.mult {
                    Multiplicity::Set => "Set()".to_string(),
                    Multiplicity::Seq => "[]".to_string(),
                    _ => swift_default_value(&f.target, &f.mult),
                };
                writeln!(out, "        {}: {}{}", f.name, val, comma).unwrap();
            }
            writeln!(out, "    )").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

fn swift_boundary_value(target: &str, mult: &Multiplicity, count: usize) -> String {
    match mult {
        Multiplicity::Set => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            if items.is_empty() {
                "Set()".to_string()
            } else {
                format!("Set([{}])", items.join(", "))
            }
        }
        Multiplicity::Seq => {
            let items: Vec<String> = (0..count).map(|_| format!("default{target}()")).collect();
            if items.is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", items.join(", "))
            }
        }
        _ => swift_default_value(target, mult),
    }
}

fn swift_return_type(type_name: &str, mult: &Multiplicity) -> String {
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

fn swift_default_value_inner(target: &str, mult: &Multiplicity, safe_targets: &HashSet<String>) -> String {
    match mult {
        Multiplicity::Lone => "nil".to_string(),
        Multiplicity::Set => {
            if safe_targets.contains(target) {
                format!("Set([default{target}()])")
            } else {
                "Set()".to_string()
            }
        }
        Multiplicity::Seq => {
            if safe_targets.contains(target) {
                format!("[default{target}()]")
            } else {
                "[]".to_string()
            }
        }
        Multiplicity::One => format!("default{target}()"),
    }
}

// ── Naming helpers ───────────────────────────────────────────────────────────

fn to_swift_field_name(name: &str) -> &str {
    // Swift uses camelCase for properties — Alloy field names are already camelCase
    name
}

fn to_swift_case_name(name: &str) -> String {
    // Enum case names in Swift are lowerCamelCase
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => format!("{}{}", c.to_lowercase(), chars.as_str()),
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
        Expr::TransitiveClosure(_) => true,
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
