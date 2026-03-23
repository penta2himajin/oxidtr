/// JSON Schema generation from oxidtr IR.
/// Produces a schemas.json conforming to JSON Schema draft-07.

use crate::ir::nodes::*;
use crate::parser::ast::Multiplicity;
use crate::analyze;
use crate::backend::GeneratedFile;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn generate(ir: &OxidtrIR) -> GeneratedFile {
    let constraints = analyze::analyze(ir);
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
            generate_struct_schema(&mut out, s, &constraints);
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
    _constraints: &[analyze::ConstraintInfo],
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
        write_field_schema(out, f);
        writeln!(out, "        }}{comma}").unwrap();
    }
    writeln!(out, "      }},").unwrap();

    // Required: all One fields
    let required: Vec<&str> = s.fields.iter()
        .filter(|f| f.mult == Multiplicity::One)
        .map(|f| f.name.as_str())
        .collect();
    let req_json: Vec<String> = required.iter().map(|r| format!("\"{r}\"")).collect();
    writeln!(out, "      \"required\": [{}]", req_json.join(", ")).unwrap();
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
