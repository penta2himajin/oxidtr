/// JSON Schema generation from oxidtr IR.
/// Produces a schemas.json conforming to JSON Schema draft-07.

use crate::ir::nodes::*;
use crate::parser::ast::{Multiplicity, SigMultiplicity, SetOpKind};
use crate::analyze;
use crate::backend::GeneratedFile;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// Collect set operation descriptions for fields across all constraints.
/// Returns a map of (sig_name, field_name) → human-readable description.
fn collect_set_op_descriptions(ir: &OxidtrIR) -> HashMap<(String, String), String> {
    let mut result = HashMap::new();
    for s in &ir.structures {
        for f in &s.fields {
            let ops = analyze::set_ops_for_field(ir, &s.name, &f.name);
            if !ops.is_empty() {
                let descs: Vec<String> = ops.iter().map(|(op, left, right)| {
                    match op {
                        SetOpKind::Union => format!("Union of {left} and {right}"),
                        SetOpKind::Intersection => format!("Intersection of {left} and {right}"),
                        SetOpKind::Difference => format!("Difference of {left} and {right}"),
                    }
                }).collect();
                result.insert((s.name.clone(), f.name.clone()), descs.join("; "));
            }
        }
    }
    result
}

pub fn generate(ir: &OxidtrIR) -> GeneratedFile {
    let constraints = analyze::analyze(ir);
    let disj_fields = analyze::disj_fields(ir);
    let set_op_descs = collect_set_op_descriptions(ir);
    let mut out = String::new();

    // Collect enum parents and variants
    let enum_parents: HashSet<&str> = ir.structures.iter()
        .filter(|s| s.is_enum)
        .map(|s| s.name.as_str())
        .collect();
    let children: HashMap<&str, Vec<&str>> = {
        let mut m: HashMap<&str, Vec<&str>> = HashMap::new();
        for s in &ir.structures {
            if let Some(p) = &s.parent {
                m.entry(p.as_str()).or_default().push(s.name.as_str());
            }
        }
        m
    };
    let variant_names: HashSet<&str> = ir.structures.iter()
        .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p.as_str())))
        .map(|s| s.name.as_str())
        .collect();

    writeln!(out, "{{").unwrap();
    writeln!(out, "  \"$schema\": \"http://json-schema.org/draft-07/schema#\",").unwrap();
    writeln!(out, "  \"definitions\": {{").unwrap();

    let top_sigs: Vec<&StructureNode> = ir.structures.iter()
        .filter(|s| !variant_names.contains(s.name.as_str()))
        .collect();

    for (i, s) in top_sigs.iter().enumerate() {
        let comma = if i < top_sigs.len() - 1 { "," } else { "" };

        if s.is_enum {
            generate_enum_schema(&mut out, s, children.get(s.name.as_str()), &ir.structures, &constraints);
        } else {
            generate_struct_schema(&mut out, s, &constraints, &disj_fields, ir, &set_op_descs);
        }
        writeln!(out, "    }}{comma}").unwrap();
    }

    writeln!(out, "  }}").unwrap();
    writeln!(out, "}}").unwrap();

    GeneratedFile {
        path: "schemas.json".to_string(),
        content: out,
    }
}

fn generate_struct_schema(
    out: &mut String,
    s: &StructureNode,
    constraints: &[analyze::ConstraintInfo],
    disj_fields: &[(String, String)],
    ir: &OxidtrIR,
    set_op_descs: &HashMap<(String, String), String>,
) {
    writeln!(out, "    \"{}\": {{", s.name).unwrap();
    writeln!(out, "      \"type\": \"object\",").unwrap();

    if s.fields.is_empty() {
        writeln!(out, "      \"properties\": {{}},").unwrap();
        writeln!(out, "      \"required\": []").unwrap();
        return;
    }

    // Properties
    writeln!(out, "      \"properties\": {{").unwrap();
    for (i, f) in s.fields.iter().enumerate() {
        let comma = if i < s.fields.len() - 1 { "," } else { "" };
        let bounds = field_bounds(constraints, &s.name, &f.name);
        let is_disj = disj_fields.iter().any(|(sig, field)| sig == &s.name && field == &f.name);
        let target_sig_mult = analyze::sig_multiplicity_for(ir, &f.target);
        let set_op_desc = set_op_descs.get(&(s.name.clone(), f.name.clone()));
        write_field_schema_with_bounds(out, f, &bounds, is_disj, target_sig_mult, set_op_desc);
        writeln!(out, "        }}{comma}").unwrap();
    }
    writeln!(out, "      }},").unwrap();

    // Required: all One fields, but lone sig targets are nullable even if field mult is One
    let required: Vec<&str> = s.fields.iter()
        .filter(|f| f.mult == Multiplicity::One && analyze::sig_multiplicity_for(ir, &f.target) != SigMultiplicity::Lone)
        .map(|f| f.name.as_str())
        .collect();
    let req_json: Vec<String> = required.iter().map(|r| format!("\"{r}\"")).collect();
    writeln!(out, "      \"required\": [{}]", req_json.join(", ")).unwrap();
}

/// Bounds for a field extracted from constraints.
struct FieldBounds {
    min_items: Option<usize>,
    max_items: Option<usize>,
}

fn field_bounds(constraints: &[analyze::ConstraintInfo], sig_name: &str, field_name: &str) -> FieldBounds {
    let mut min_items = None;
    let mut max_items = None;
    for c in constraints {
        if let analyze::ConstraintInfo::CardinalityBound { sig_name: s, field_name: f, bound } = c {
            if s == sig_name && f == field_name {
                match bound {
                    analyze::BoundKind::Exact(n) => { min_items = Some(*n); max_items = Some(*n); }
                    analyze::BoundKind::AtMost(n) => { max_items = Some(*n); }
                    analyze::BoundKind::AtLeast(n) => { min_items = Some(*n); }
                }
            }
        }
    }
    FieldBounds { min_items, max_items }
}

fn write_field_schema_with_bounds(
    out: &mut String,
    f: &IRField,
    bounds: &FieldBounds,
    is_disj: bool,
    target_sig_mult: SigMultiplicity,
    set_op_desc: Option<&String>,
) {
    writeln!(out, "        \"{}\": {{", f.name).unwrap();

    // Map field (A -> B): use object with additionalProperties
    if let Some(vt) = &f.value_type {
        writeln!(out, "          \"type\": \"object\",").unwrap();
        writeln!(out, "          \"additionalProperties\": {{ \"$ref\": \"#/definitions/{vt}\" }}").unwrap();
        return;
    }

    // Gap 2: add set operation description if present
    if let Some(desc) = set_op_desc {
        writeln!(out, "          \"description\": \"{}\",", desc).unwrap();
    }

    // Gap 1: lone sig target makes reference nullable even for One fields
    if target_sig_mult == SigMultiplicity::Lone && f.mult == Multiplicity::One {
        writeln!(out, "          \"oneOf\": [").unwrap();
        writeln!(out, "            {{ \"$ref\": \"#/definitions/{}\" }},", f.target).unwrap();
        writeln!(out, "            {{ \"type\": \"null\" }}").unwrap();
        writeln!(out, "          ]").unwrap();
        return;
    }

    match f.mult {
        Multiplicity::One => {
            writeln!(out, "          \"$ref\": \"#/definitions/{}\"", f.target).unwrap();
        }
        Multiplicity::Lone => {
            writeln!(out, "          \"oneOf\": [").unwrap();
            writeln!(out, "            {{ \"$ref\": \"#/definitions/{}\" }},", f.target).unwrap();
            writeln!(out, "            {{ \"type\": \"null\" }}").unwrap();
            writeln!(out, "          ]").unwrap();
        }
        Multiplicity::Set => {
            writeln!(out, "          \"type\": \"array\",").unwrap();
            writeln!(out, "          \"uniqueItems\": true,").unwrap();
            // Gap 1: some sig target → at least 1 item
            let effective_min = match target_sig_mult {
                SigMultiplicity::Some => Some(bounds.min_items.map_or(1, |n| n.max(1))),
                _ => bounds.min_items,
            };
            if let Some(n) = effective_min {
                writeln!(out, "          \"minItems\": {n},").unwrap();
            }
            if let Some(n) = bounds.max_items {
                writeln!(out, "          \"maxItems\": {n},").unwrap();
            }
            writeln!(out, "          \"items\": {{ \"$ref\": \"#/definitions/{}\" }}", f.target).unwrap();
        }
        Multiplicity::Seq => {
            writeln!(out, "          \"type\": \"array\",").unwrap();
            // Feature 6: disj implies uniqueItems for Seq fields
            if is_disj {
                writeln!(out, "          \"uniqueItems\": true,").unwrap();
            }
            // Gap 1: some sig target → at least 1 item
            let effective_min = match target_sig_mult {
                SigMultiplicity::Some => Some(bounds.min_items.map_or(1, |n| n.max(1))),
                _ => bounds.min_items,
            };
            if let Some(n) = effective_min {
                writeln!(out, "          \"minItems\": {n},").unwrap();
            }
            if let Some(n) = bounds.max_items {
                writeln!(out, "          \"maxItems\": {n},").unwrap();
            }
            writeln!(out, "          \"items\": {{ \"$ref\": \"#/definitions/{}\" }}", f.target).unwrap();
        }
    }
}

fn write_field_schema(out: &mut String, f: &IRField) {
    writeln!(out, "        \"{}\": {{", f.name).unwrap();
    match f.mult {
        Multiplicity::One => {
            writeln!(out, "          \"$ref\": \"#/definitions/{}\"", f.target).unwrap();
        }
        Multiplicity::Lone => {
            writeln!(out, "          \"oneOf\": [").unwrap();
            writeln!(out, "            {{ \"$ref\": \"#/definitions/{}\" }},", f.target).unwrap();
            writeln!(out, "            {{ \"type\": \"null\" }}").unwrap();
            writeln!(out, "          ]").unwrap();
        }
        Multiplicity::Set => {
            writeln!(out, "          \"type\": \"array\",").unwrap();
            writeln!(out, "          \"uniqueItems\": true,").unwrap();
            writeln!(out, "          \"items\": {{ \"$ref\": \"#/definitions/{}\" }}", f.target).unwrap();
        }
        Multiplicity::Seq => {
            writeln!(out, "          \"type\": \"array\",").unwrap();
            writeln!(out, "          \"items\": {{ \"$ref\": \"#/definitions/{}\" }}", f.target).unwrap();
        }
    }
}

fn generate_enum_schema(
    out: &mut String,
    s: &StructureNode,
    children: Option<&Vec<&str>>,
    all_structures: &[StructureNode],
    _constraints: &[analyze::ConstraintInfo],
) {
    writeln!(out, "    \"{}\": {{", s.name).unwrap();

    let Some(variants) = children else {
        writeln!(out, "      \"type\": \"string\"").unwrap();
        return;
    };

    // Check if all variants are unit (singletons or fieldless)
    let all_unit = variants.iter().all(|v| {
        all_structures.iter()
            .find(|st| st.name == *v)
            .map_or(true, |st| st.fields.is_empty())
    });

    if all_unit {
        let vals: Vec<String> = variants.iter().map(|v| format!("\"{v}\"")).collect();
        writeln!(out, "      \"enum\": [{}]", vals.join(", ")).unwrap();
    } else {
        // Discriminated union: oneOf with kind discriminator
        writeln!(out, "      \"oneOf\": [").unwrap();
        for (i, v) in variants.iter().enumerate() {
            let comma = if i < variants.len() - 1 { "," } else { "" };
            let child = all_structures.iter().find(|st| st.name == *v);
            writeln!(out, "        {{").unwrap();
            writeln!(out, "          \"type\": \"object\",").unwrap();
            writeln!(out, "          \"properties\": {{").unwrap();
            writeln!(out, "            \"kind\": {{ \"const\": \"{v}\" }}").unwrap();
            if let Some(child) = child {
                for f in &child.fields {
                    writeln!(out, "            ,").unwrap();
                    write_field_schema(out, f);
                    writeln!(out, "            }}").unwrap();
                }
            }
            writeln!(out, "          }},").unwrap();
            writeln!(out, "          \"required\": [\"kind\"]").unwrap();
            writeln!(out, "        }}{comma}").unwrap();
        }
        writeln!(out, "      ],").unwrap();
        writeln!(out, "      \"discriminator\": {{ \"propertyName\": \"kind\" }}").unwrap();
    }
}
