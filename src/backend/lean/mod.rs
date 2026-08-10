pub mod expr_translator;

use crate::backend::{GeneratedFile, TargetLang, is_native_type_alias, resolve_type};
use crate::ir::nodes::*;
use crate::parser::ast::{CompareOp, Multiplicity, SigMultiplicity};
use crate::analyze;
use expr_translator::{lean_field, lean_ident};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn generate(ir: &OxidtrIR) -> Vec<GeneratedFile> {
    let ctx = LeanContext::from_ir(ir);
    let mut files = Vec::new();

    files.push(GeneratedFile {
        path: "Types.lean".to_string(),
        content: generate_types(ir, &ctx),
    });

    // `assert`s become theorems in the same file, so a model with properties but
    // no facts still needs it — gating on constraints alone dropped them silently.
    if !ir.constraints.is_empty() || !ir.properties.is_empty() {
        let constraints_content = generate_constraints(ir, &ctx);
        if !constraints_content.is_empty() {
            files.push(GeneratedFile {
                path: "Constraints.lean".to_string(),
                content: constraints_content,
            });
        }
    }

    if ir.operations.iter().any(|op| op.receiver_sig.is_none()) {
        files.push(GeneratedFile {
            path: "Operations.lean".to_string(),
            content: generate_operations(ir),
        });
    }

    files
}

// ── Context ──────────────────────────────────────────────────────────────────

struct LeanContext {
    children: HashMap<String, Vec<String>>,
    variant_names: HashSet<String>,
    struct_map: HashMap<String, StructureNode>,
    /// Parent fields: fields defined on the abstract sig that children inherit
    parent_fields: HashMap<String, Vec<IRField>>,
}

impl LeanContext {
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

        // Collect parent fields for abstract sigs
        let mut parent_fields: HashMap<String, Vec<IRField>> = HashMap::new();
        for s in &ir.structures {
            if s.is_enum && !s.fields.is_empty() {
                parent_fields.insert(s.name.clone(), s.fields.clone());
            }
        }

        LeanContext { children, variant_names, struct_map, parent_fields }
    }

    fn is_variant(&self, name: &str) -> bool {
        self.variant_names.contains(name)
    }

    /// Every emitted declaration this one names in a field type. Unlike Swift's
    /// `find_recursive_types`, containers do *not* break the edge: Lean needs
    /// `List B` declared before `A` just as much as a bare `B`, and its
    /// `DecidableEq` handler bails on recursion through `List`/`Option` too.
    fn type_deps(&self, s: &StructureNode) -> Vec<String> {
        let mut fields: Vec<&IRField> = s.fields.iter().collect();
        if s.is_enum {
            for kid in self.children.get(&s.name).map(|v| v.as_slice()).unwrap_or(&[]) {
                if let Some(k) = self.struct_map.get(kid) {
                    fields.extend(k.fields.iter());
                }
            }
        }
        let mut out = Vec::new();
        for f in fields {
            for t in [Some(&f.target), f.value_type.as_ref()].into_iter().flatten() {
                // A variant has no declaration of its own — it is a constructor
                // of its parent inductive, so the edge points at the parent.
                let owner = if self.is_variant(t) {
                    self.struct_map.get(t).and_then(|v| v.parent.clone()).unwrap_or_else(|| t.clone())
                } else {
                    t.clone()
                };
                if !out.contains(&owner) {
                    out.push(owner);
                }
            }
        }
        out
    }

    fn is_all_singleton_enum(&self, s: &StructureNode) -> bool {
        if !s.is_enum { return false; }
        let kids = match self.children.get(&s.name) {
            Some(v) => v,
            None => return false,
        };
        if kids.is_empty() { return false; }
        // All children have no fields AND parent has no fields
        if !s.fields.is_empty() { return false; }
        kids.iter().all(|name| {
            self.struct_map.get(name)
                .map_or(false, |cs| cs.fields.is_empty() && cs.sig_multiplicity == SigMultiplicity::One)
        })
    }
}

// ── Types.lean ──────────────────────────────────────────────────────────────

/// One `mutual … end` block's worth of declarations, or a single standalone
/// type when `names.len() == 1 && !recursive`.
struct DeclGroup {
    names: Vec<String>,
    /// Whether `deriving DecidableEq` is viable. Lean's handler has no case for
    /// a (nested-)recursive type — `List T` and `Option T` count as nested — and
    /// the property is *transitive*: a plain `structure` holding a recursive
    /// type cannot derive it either, because the field's own instance is missing.
    decidable_eq: bool,
}

/// Emitted declarations grouped into strongly-connected components and ordered
/// so every type is declared before it is referenced. Lean rejects a forward
/// reference outright — it auto-binds the unknown name as an implicit universe
/// variable, which then poisons every `deriving` line downstream.
///
/// ponytail: O(n²)-ish reachability rather than Tarjan. `n` is the number of
/// sigs in a model (tens); swap in Tarjan if a model ever makes this show up in
/// a profile.
fn declaration_groups(ir: &OxidtrIR, ctx: &LeanContext) -> Vec<DeclGroup> {
    let emitted: Vec<String> = ir.structures.iter()
        .filter(|s| !ctx.is_variant(&s.name) && !is_native_type_alias(&s.name))
        .map(|s| s.name.clone())
        .collect();
    let in_scope: HashSet<&str> = emitted.iter().map(|s| s.as_str()).collect();

    let deps: HashMap<&str, Vec<String>> = ir.structures.iter()
        .filter(|s| in_scope.contains(s.name.as_str()))
        .map(|s| {
            let d = ctx.type_deps(s).into_iter()
                .filter(|t| in_scope.contains(t.as_str()))
                .collect();
            (s.name.as_str(), d)
        })
        .collect();

    let reaches = |from: &str, to: &str| -> bool {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = deps.get(from).map(|v| v.iter().map(|s| s.as_str()).collect()).unwrap_or_default();
        while let Some(cur) = stack.pop() {
            if cur == to { return true; }
            if !seen.insert(cur) { continue; }
            if let Some(next) = deps.get(cur) {
                stack.extend(next.iter().map(|s| s.as_str()));
            }
        }
        false
    };

    // Types that sit on a cycle: those are exactly the ones Lean's DecidableEq
    // handler bails on, and anything that reaches one inherits the gap.
    let on_cycle: HashSet<&str> = emitted.iter()
        .map(|n| n.as_str()).filter(|n| reaches(n, n)).collect();

    // Group by mutual reachability, keeping model order so output is deterministic.
    let mut groups: Vec<DeclGroup> = Vec::new();
    let mut placed: HashSet<&str> = HashSet::new();
    for name in &emitted {
        if placed.contains(name.as_str()) { continue; }
        let mut names = vec![name.clone()];
        placed.insert(name.as_str());
        for other in &emitted {
            if placed.contains(other.as_str()) { continue; }
            if reaches(name, other) && reaches(other, name) {
                names.push(other.clone());
                placed.insert(other.as_str());
            }
        }
        let decidable_eq = !on_cycle.contains(name.as_str())
            && !emitted.iter().any(|t| on_cycle.contains(t.as_str()) && reaches(name, t));
        groups.push(DeclGroup { names, decidable_eq });
    }

    // Topological order over the groups: emit one whose remaining dependencies
    // are all satisfied. A group always satisfies its own members.
    let mut ordered: Vec<DeclGroup> = Vec::with_capacity(groups.len());
    let mut done: HashSet<String> = HashSet::new();
    while !groups.is_empty() {
        let pick = groups.iter().position(|g| {
            g.names.iter().all(|n| {
                deps.get(n.as_str()).map(|d| d.iter().all(|t| done.contains(t) || g.names.contains(t))).unwrap_or(true)
            })
        });
        // A cycle across two groups is impossible by construction; if one ever
        // appears, emit in model order rather than looping forever.
        let idx = pick.unwrap_or(0);
        let g = groups.remove(idx);
        done.extend(g.names.iter().cloned());
        ordered.push(g);
    }
    ordered
}

fn generate_types(ir: &OxidtrIR, ctx: &LeanContext) -> String {
    let mut out = String::new();
    writeln!(out, "-- Generated by oxidtr (Lean 4 backend)").unwrap();
    writeln!(out, "-- Mathlib imports (uncomment when proving theorems):").unwrap();
    writeln!(out, "-- import Mathlib.Data.List.Basic").unwrap();
    writeln!(out, "-- import Mathlib.Order.RelClasses").unwrap();
    writeln!(out).unwrap();

    let groups = declaration_groups(ir, ctx);
    let mut singletons: Vec<&StructureNode> = Vec::new();

    for group in &groups {
        let mutual = group.names.len() > 1;
        if mutual {
            writeln!(out, "mutual").unwrap();
        }
        for name in &group.names {
            let s = match ctx.struct_map.get(name) { Some(s) => s, None => continue };

            let constraint_names = analyze::constraint_names_for_sig(ir, &s.name);
            if !constraint_names.is_empty() {
                writeln!(out, "-- Invariants:").unwrap();
                for cn in &constraint_names {
                    writeln!(out, "-- - {cn}").unwrap();
                }
            }

            if s.is_enum {
                generate_inductive(&mut out, s, ir, ctx, group.decidable_eq);
            } else {
                generate_structure(&mut out, s, ir, ctx, group.decidable_eq);
                if s.sig_multiplicity == SigMultiplicity::One {
                    singletons.push(s);
                }
            }
            writeln!(out).unwrap();
        }
        if mutual {
            writeln!(out, "end").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Singleton `def`s trail the types: a `def` cannot live inside `mutual`.
    for s in singletons {
        generate_singleton_instance(&mut out, s);
    }

    // Derived fields: receiver functions → Lean defs on the type
    generate_derived_fields(&mut out, ir);

    out
}

/// `DecidableEq` derives only for a non-recursive type: Lean's handler has no
/// case for a (nested-)recursive one, and `List T`/`Option T` count as nested.
fn deriving_clause(decidable_eq: bool) -> &'static str {
    if decidable_eq { "  deriving Repr, BEq, DecidableEq" } else { "  deriving Repr, BEq" }
}

fn generate_structure(out: &mut String, s: &StructureNode, _ir: &OxidtrIR, ctx: &LeanContext, decidable_eq: bool) {
    if s.is_var {
        writeln!(out, "-- Alloy var sig: instances change across state transitions").unwrap();
    }

    writeln!(out, "structure {} where", lean_ident(&s.name)).unwrap();
    if s.fields.is_empty() {
        writeln!(out, "  mk ::").unwrap();
    } else {
        for f in &s.fields {
            if f.is_var {
                writeln!(out, "  -- Alloy var field: mutable across state transitions").unwrap();
            }
            let type_str = field_type_str(f, ctx);
            writeln!(out, "  {} : {}", lean_field(&f.name), type_str).unwrap();
        }
    }
    writeln!(out, "{}", deriving_clause(decidable_eq)).unwrap();
}

fn generate_singleton_instance(out: &mut String, s: &StructureNode) {
    writeln!(out, "def {}Instance : {} :=", expr_translator::to_lower_camel(&s.name), lean_ident(&s.name)).unwrap();
    {
        if s.fields.is_empty() {
            writeln!(out, "  {}.mk", lean_ident(&s.name)).unwrap();
        } else {
            // Generate concrete values: primitives get defaults, collections get empty
            let can_default = s.fields.iter().all(|f| {
                match f.mult {
                    Multiplicity::One => is_lean_defaultable(&f.target),
                    Multiplicity::Lone | Multiplicity::Set | Multiplicity::Seq => true,
                }
            });
            if can_default {
                write!(out, "  {{ ").unwrap();
                let field_inits: Vec<String> = s.fields.iter().map(|f| {
                    let val = match f.mult {
                        Multiplicity::One => lean_default_value(&f.target).to_string(),
                        Multiplicity::Lone => "none".to_string(),
                        Multiplicity::Set | Multiplicity::Seq => "[]".to_string(),
                    };
                    format!("{} := {}", lean_field(&f.name), val)
                }).collect();
                write!(out, "{}", field_inits.join(", ")).unwrap();
                writeln!(out, " }}").unwrap();
            } else {
                writeln!(out, "  sorry -- provide concrete values").unwrap();
            }
        }
    }
    writeln!(out).unwrap();
}

fn generate_inductive(out: &mut String, s: &StructureNode, _ir: &OxidtrIR, ctx: &LeanContext, decidable_eq: bool) {
    let kids = ctx.children.get(&s.name);
    let kid_list = match kids {
        Some(v) => v.clone(),
        None => Vec::new(),
    };

    // Check if it's a simple enum (all singletons, no fields)
    let is_simple_enum = ctx.is_all_singleton_enum(s);

    // Get parent fields (fields on the abstract sig)
    let pfields = ctx.parent_fields.get(&s.name).cloned().unwrap_or_default();

    writeln!(out, "inductive {} where", lean_ident(&s.name)).unwrap();

    for kid_name in &kid_list {
        let variant_name = lean_field(kid_name);
        if is_simple_enum {
            writeln!(out, "  | {} : {}", variant_name, lean_ident(&s.name)).unwrap();
        } else {
            // Collect fields: parent fields + variant-specific fields
            let kid_fields: Vec<&IRField> = if let Some(kid) = ctx.struct_map.get(kid_name) {
                pfields.iter().chain(kid.fields.iter()).collect()
            } else {
                pfields.iter().collect()
            };

            if kid_fields.is_empty() {
                writeln!(out, "  | {} : {}", variant_name, lean_ident(&s.name)).unwrap();
            } else {
                let named_params: Vec<String> = kid_fields.iter()
                    .map(|f| {
                        let fname = lean_field(&f.name);
                        let ftype = field_type_str(f, ctx);
                        format!("({fname} : {ftype})")
                    })
                    .collect();
                writeln!(out, "  | {} {} : {}", variant_name, named_params.join(" "), lean_ident(&s.name)).unwrap();
            }
        }
    }
    writeln!(out, "{}", deriving_clause(decidable_eq)).unwrap();
}

fn generate_derived_fields(out: &mut String, ir: &OxidtrIR) {
    for op in order_callee_first(ir.operations.iter().filter(|op| op.receiver_sig.is_some()), true) {
        if let Some(ref sig) = op.receiver_sig {
            let fn_name = lean_field(&op.name);
            let params: Vec<String> = op.params.iter().map(|p| {
                let type_str = lean_type(&p.type_name, &p.mult);
                format!("({} : {type_str})", lean_field(&p.name))
            }).collect();
            let params_str = if params.is_empty() {
                String::new()
            } else {
                format!(" {}", params.join(" "))
            };

            let return_str = match &op.return_type {
                Some(rt) => lean_type(&rt.type_name, &rt.mult),
                None => "Prop".to_string(),  // an Alloy pred is a formula, not a Bool-valued function
            };

            let sig = lean_ident(sig);
            writeln!(out, "def {sig}.{fn_name} (self : {sig}){params_str} : {return_str} :=").unwrap();
            if !op.body.is_empty() {
                let body_expr = &op.body[op.body.len() - 1];
                let body_str = expr_translator::translate_with_ir(body_expr, ir);
                writeln!(out, "  {body_str}").unwrap();
            } else {
                writeln!(out, "  sorry -- oxidtr: implement {}", op.name).unwrap();
            }
            writeln!(out).unwrap();
        }
    }
}

fn field_type_str(f: &IRField, _ctx: &LeanContext) -> String {
    if let Some(vt) = &f.value_type {
        let resolved_key = resolve_type(TargetLang::Lean, &f.target);
        let resolved_val = resolve_type(TargetLang::Lean, vt);
        // Map type: Key → Value as List (Key × Value)
        return format!("List ({} × {})", lean_ident(&resolved_key), lean_ident(&resolved_val));
    }
    let resolved = resolve_type(TargetLang::Lean, &f.target);
    lean_mult_type(&resolved, &f.mult)
}

fn lean_mult_type(target: &str, mult: &Multiplicity) -> String {
    let lean_target = lean_ident(&lean_primitive_type(target));
    match mult {
        Multiplicity::One => lean_target,
        Multiplicity::Lone => format!("Option {lean_target}"),
        Multiplicity::Set => format!("List {lean_target}"),
        Multiplicity::Seq => format!("List {lean_target}"),
    }
}

fn lean_type(type_name: &str, mult: &Multiplicity) -> String {
    lean_mult_type(type_name, mult)
}

fn is_lean_defaultable(name: &str) -> bool {
    matches!(name, "Int" | "String" | "Bool" | "Nat")
}

fn lean_default_value(name: &str) -> &str {
    match name {
        "Int" => "0",
        "Nat" => "0",
        "String" => "\"\"",
        "Bool" => "false",
        _ => "sorry",
    }
}

fn lean_primitive_type(name: &str) -> String {
    match name {
        "Int" => "Int".to_string(),
        "String" => "String".to_string(),
        "Bool" => "Bool".to_string(),
        other => other.to_string(),
    }
}

/// An Alloy `fact` restricts which instances exist. Restated as `∀ x : Sig, …`
/// it is a claim about *every* inhabitant of the Lean type, which is generally
/// false — `omega`/`simp` cannot close it, and a failing tactic is a hard error,
/// not a warning. Emit `sorry` (a warning) until facts are encoded as
/// hypotheses rather than goals. See issue #79.
fn write_fact_sorry(out: &mut String) {
    writeln!(out, "  sorry -- an Alloy fact is an axiom over instances, not a theorem about the type").unwrap();
}

// ── Constraints.lean ────────────────────────────────────────────────────────

/// Alloy's `x in Category` asks which *variant* an atom is. A sig name lowers
/// to a Lean type, so `x ∈ Circle` is a membership test against a `Type` and
/// never elaborates; the variant test is a pattern match. Falls back to `∈` for
/// a category that is not a variant (a genuine set-valued expression).
fn category_test(ctx: &LeanContext, category: &str) -> String {
    if ctx.is_variant(category) {
        format!("x matches .{} ..", lean_field(category))
    } else {
        format!("x ∈ {}", lean_ident(category))
    }
}

fn generate_constraints(ir: &OxidtrIR, ctx: &LeanContext) -> String {
    let sig_constraints = analyze::analyze(ir);
    if sig_constraints.is_empty() && ir.properties.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    writeln!(out, "-- Generated by oxidtr (Lean 4 backend)").unwrap();
    writeln!(out, "-- Constraint theorems with proof strategies").unwrap();
    writeln!(out, "-- Mathlib imports (uncomment when proving):").unwrap();
    writeln!(out, "-- import Mathlib.Data.List.Basic").unwrap();
    writeln!(out, "-- import Mathlib.Order.RelClasses").unwrap();
    writeln!(out, "import Types").unwrap();
    writeln!(out).unwrap();

    let mut theorem_idx = 0;

    for c in &sig_constraints {
        match c {
            analyze::ConstraintInfo::NoSelfRef { sig_name, field_name } => {
                let fname = lean_field(field_name);
                writeln!(out, "theorem no_self_ref_{sig_name}_{field_name} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), x.{fname} ≠ some x := by").unwrap();
                writeln!(out, "  intro x").unwrap();
                writeln!(out, "  sorry -- Needs: subtype encoding or refinement type").unwrap();
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::Acyclic { sig_name, field_name } => {
                let fname = lean_field(field_name);
                writeln!(out, "theorem acyclic_{sig_name}_{field_name} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), ¬ Relation.TransGen (fun a b => a.{fname} = some b) x x := by").unwrap();
                writeln!(out, "  intro x h").unwrap();
                writeln!(out, "  sorry -- Needs: well-founded recursion on Relation.TransGen").unwrap();
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::FieldOrdering { sig_name, left_field, op, right_field } => {
                let lf = lean_field(left_field);
                let rf = lean_field(right_field);
                let lean_op = match op {
                    CompareOp::Lt => "<",
                    CompareOp::Gt => ">",
                    CompareOp::Lte => "≤",
                    CompareOp::Gte => "≥",
                    CompareOp::Eq => "=",
                    CompareOp::NotEq => "≠",
                    _ => "?",
                };
                writeln!(out, "theorem field_ordering_{sig_name}_{left_field}_{right_field} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), x.{lf} {lean_op} x.{rf} := by").unwrap();
                writeln!(out, "  intro x").unwrap();
                write_fact_sorry(&mut out);
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::Implication { sig_name, condition, consequent } => {
                let cond_str = expr_translator::translate_with_ir(condition, ir);
                let cons_str = expr_translator::translate_with_ir(consequent, ir);
                writeln!(out, "theorem implication_{sig_name}_{theorem_idx} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), {cond_str} → {cons_str} := by").unwrap();
                writeln!(out, "  intro x h").unwrap();
                writeln!(out, "  sorry -- Hint: derive consequent from hypothesis h").unwrap();
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::Iff { sig_name, left, right } => {
                let left_str = expr_translator::translate_with_ir(left, ir);
                let right_str = expr_translator::translate_with_ir(right, ir);
                writeln!(out, "theorem iff_{sig_name}_{theorem_idx} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), {left_str} ↔ {right_str} := by").unwrap();
                writeln!(out, "  intro x").unwrap();
                writeln!(out, "  constructor").unwrap();
                writeln!(out, "  · intro h; sorry -- forward direction").unwrap();
                writeln!(out, "  · intro h; sorry -- backward direction").unwrap();
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::Prohibition { sig_name, condition } => {
                let cond_str = expr_translator::translate_with_ir(condition, ir);
                writeln!(out, "theorem prohibition_{sig_name}_{theorem_idx} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), ¬({cond_str}) := by").unwrap();
                writeln!(out, "  intro x h").unwrap();
                writeln!(out, "  sorry -- Hint: derive contradiction from h").unwrap();
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::Disjoint { sig_name, left, right } => {
                writeln!(out, "theorem disjoint_{sig_name}_{theorem_idx} :").unwrap();
                let (l, r) = (category_test(ctx, left), category_test(ctx, right));
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), ¬({l} ∧ {r}) := by").unwrap();
                writeln!(out, "  intro x ⟨h₁, h₂⟩").unwrap();
                writeln!(out, "  sorry -- Hint: derive contradiction from h₁ and h₂").unwrap();
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::Exhaustive { sig_name, categories } => {
                let cats = categories.iter()
                    .map(|c| category_test(ctx, c))
                    .collect::<Vec<_>>()
                    .join(" ∨ ");
                writeln!(out, "theorem exhaustive_{sig_name}_{theorem_idx} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), {cats} := by").unwrap();
                writeln!(out, "  intro x").unwrap();
                write_fact_sorry(&mut out);
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            analyze::ConstraintInfo::CardinalityBound { sig_name, field_name, bound } => {
                let fname = lean_field(field_name);
                let bound_str = match bound {
                    analyze::BoundKind::Exact(n) => format!("x.{fname}.length = {n}"),
                    analyze::BoundKind::AtMost(n) => format!("x.{fname}.length ≤ {n}"),
                    analyze::BoundKind::AtLeast(n) => format!("x.{fname}.length ≥ {n}"),
                };
                writeln!(out, "theorem cardinality_{sig_name}_{field_name} :").unwrap();
                let sig = lean_ident(sig_name);
                writeln!(out, "    ∀ (x : {sig}), {bound_str} := by").unwrap();
                writeln!(out, "  intro x").unwrap();
                write_fact_sorry(&mut out);
                writeln!(out).unwrap();
                theorem_idx += 1;
            }
            // Presence: type-guaranteed in Lean (non-Option = required)
            analyze::ConstraintInfo::Presence { sig_name, field_name, kind } => {
                let fname = lean_field(field_name);
                match kind {
                    analyze::PresenceKind::Required => {
                        writeln!(out, "-- {sig_name}.{fname}: required (guaranteed by type — field is non-Option)").unwrap();
                    }
                    analyze::PresenceKind::Absent => {
                        writeln!(out, "-- {sig_name}.{fname}: absent (guaranteed by type — field not present)").unwrap();
                    }
                }
                writeln!(out).unwrap();
            }
            // Membership, Named — skip
            _ => {}
        }
    }

    // Emit fact name anchors so `check` validation can find them
    writeln!(out, "-- Validated facts:").unwrap();
    for c in &ir.constraints {
        if let Some(ref name) = c.name {
            writeln!(out, "-- {name}").unwrap();
        }
    }
    writeln!(out).unwrap();

    // Properties (asserts) as theorems
    for p in &ir.properties {
        let body_str = expr_translator::translate_with_ir(&p.expr, ir);
        writeln!(out, "theorem {} :", lean_ident(&p.name)).unwrap();
        writeln!(out, "    {body_str} := by").unwrap();
        writeln!(out, "  sorry").unwrap();
        writeln!(out).unwrap();
    }

    out
}

// ── Operations.lean ─────────────────────────────────────────────────────────

/// Names of the preds an expression calls, so `generate_operations` can emit a
/// callee before its caller — Lean has no forward declaration for `def`.
fn called_ops(expr: &crate::parser::ast::Expr, want_receiver: bool, out: &mut Vec<String>) {
    use crate::parser::ast::Expr as E;
    match expr {
        E::FunApp { name, receiver, args } => {
            // Only calls of the same shape as the ops being ordered create an
            // in-file dependency: a receiver call lands in Types.lean, a free
            // call in Operations.lean. Recording both under a bare name made a
            // receiver call `x.g[..]` look like a dependency on an unrelated
            // free pred that merely shares the name `g`.
            if receiver.is_some() == want_receiver && !out.contains(name) {
                out.push(name.clone());
            }
            if let Some(r) = receiver { called_ops(r, want_receiver, out); }
            for a in args { called_ops(a, want_receiver, out); }
        }
        E::FieldAccess { base, .. } => called_ops(base, want_receiver, out),
        E::Cardinality(i) | E::TransitiveClosure(i) | E::ReflexiveClosure(i) | E::Not(i)
            | E::Prime(i) | E::TemporalUnary { expr: i, .. }
            | E::MultFormula { expr: i, .. } => called_ops(i, want_receiver, out),
        E::Comparison { left, right, .. } | E::BinaryLogic { left, right, .. }
            | E::SetOp { left, right, .. } | E::Product { left, right }
            | E::TemporalBinary { left, right, .. } => { called_ops(left, want_receiver, out); called_ops(right, want_receiver, out); }
        E::Quantifier { bindings, body, .. } => {
            for b in bindings { called_ops(&b.domain, want_receiver, out); }
            called_ops(body, want_receiver, out);
        }
        E::VarRef(_) | E::IntLiteral(_) => {}
    }
}

/// Callee before caller — Lean has no forward declaration for `def` either.
///
/// ponytail: a call *cycle* would need `mutual` plus a termination proof; those
/// fall back to model order and Lean reports the forward reference. Note the
/// ordering is per-file: a derived field in Types.lean still cannot call a free
/// pred, because Operations.lean imports Types.lean and not the other way round.
fn order_callee_first<'a>(ops: impl Iterator<Item = &'a OperationNode>, want_receiver: bool)
    -> Vec<&'a OperationNode>
{
    let mut pending: Vec<&OperationNode> = ops.collect();
    let mut ordered: Vec<&OperationNode> = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let pick = pending.iter().position(|op| {
            let mut calls = Vec::new();
            for b in &op.body { called_ops(b, want_receiver, &mut calls); }
            // A callee is satisfied when every op still pending under that name
            // *is* this op — genuine self-recursion, or nothing left to wait
            // for. Identity, not name equality: two sigs may each declare a
            // derived field called `size`, and comparing names would read the
            // call to the other one as self-recursion and emit them backwards.
            calls.iter().all(|c| {
                pending.iter().filter(|p| p.name == *c).all(|p| std::ptr::eq(*p, *op))
            })
        }).unwrap_or(0);
        ordered.push(pending.remove(pick));
    }
    ordered
}

fn generate_operations(ir: &OxidtrIR) -> String {
    let mut out = String::new();
    writeln!(out, "-- Generated by oxidtr (Lean 4 backend)").unwrap();
    writeln!(out, "import Types").unwrap();
    writeln!(out).unwrap();

    for op in order_callee_first(ir.operations.iter().filter(|op| op.receiver_sig.is_none()), false) {

        let fn_name = lean_field(&op.name);
        let params: Vec<String> = op.params.iter().map(|p| {
            let type_str = lean_type(&p.type_name, &p.mult);
            format!("({} : {type_str})", lean_field(&p.name))
        }).collect();
        let params_str = params.join(" ");

        let return_str = match &op.return_type {
            Some(rt) => lean_type(&rt.type_name, &rt.mult),
            None => "Prop".to_string(),  // an Alloy pred is a formula, not a Bool-valued function
        };

        writeln!(out, "def {fn_name} {params_str} : {return_str} :=").unwrap();
        if !op.body.is_empty() {
            let body_expr = &op.body[op.body.len() - 1];
            let body_str = expr_translator::translate_with_ir(body_expr, ir);
            writeln!(out, "  {body_str}").unwrap();
        } else {
            writeln!(out, "  sorry -- oxidtr: implement {}", op.name).unwrap();
        }
        writeln!(out).unwrap();
    }

    out
}
