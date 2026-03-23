pub mod expr_translator;

use crate::backend::GeneratedFile;
use crate::ir::nodes::*;
use crate::parser::ast::{Multiplicity, SigMultiplicity};
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

        if s.is_enum {
            generate_enum(&mut out, s, ctx);
        } else {
            generate_struct(&mut out, s, ir, ctx, &disj_fields);
        }
        writeln!(out).unwrap();
    }

    out
}

fn generate_struct(out: &mut String, s: &StructureNode, ir: &OxidtrIR, ctx: &SwiftContext, disj_fields: &[(String, String)]) {
    // Singleton: one sig → static let
    if s.sig_multiplicity == SigMultiplicity::One && s.fields.is_empty() {
        writeln!(out, "struct {} {{", s.name).unwrap();
        writeln!(out, "    static let shared = {}()", s.name).unwrap();
        writeln!(out, "}}").unwrap();
        return;
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

            writeln!(out, "    let {}: {type_str}", to_swift_field_name(&f.name)).unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
}

fn generate_enum(out: &mut String, s: &StructureNode, ctx: &SwiftContext) {
    let variants = ctx.children.get(&s.name);

    // Check if all variants are unit (no fields)
    let all_unit = variants.map_or(true, |vs| {
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
                let fields = child.map(|c| &c.fields).filter(|f| !f.is_empty());
                if let Some(fields) = fields {
                    let params: Vec<String> = fields.iter().map(|f| {
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

        writeln!(out, "    func test_{}() {{", to_snake_case(&prop.name)).unwrap();
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

        writeln!(out, "    func test_invariant_{}() {{", fact_name).unwrap();
        for (pname, tname) in &params {
            writeln!(out, "        let {pname}: [{tname}] = []").unwrap();
        }
        writeln!(out, "        XCTAssertTrue({body})").unwrap();
        writeln!(out, "    }}").unwrap();
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
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
}
