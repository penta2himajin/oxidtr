/// Extracts Alloy model candidates from JSON Schema (oxidtr's generated schemas.json).
/// Lightweight hand-written line-based parser -- no serde_json dependency.
/// Handles: "type":"object" + properties -> sig, "$ref" -> one, oneOf with null -> lone,
/// "type":"array" + items -> set, "enum":[...] -> abstract sig + children,
/// discriminated union (oneOf + kind) -> abstract sig + sub sigs.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let len = lines.len();
    let mut i = 0;

    // Find "definitions" block
    while i < len {
        if lines[i].trim().starts_with("\"definitions\"") {
            i += 1;
            break;
        }
        i += 1;
    }

    // Parse each top-level definition
    while i < len {
        let trimmed = lines[i].trim();

        if let Some(name) = parse_definition_key(trimmed) {
            // Collect the body of this definition (find matching })
            let body_start = i;
            let mut depth = 0i32;
            let mut body_end = i;
            for j in i..len {
                for ch in lines[j].chars() {
                    match ch {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if depth == 0 {
                    body_end = j;
                    break;
                }
            }

            let body: String = lines[body_start..=body_end]
                .iter()
                .map(|l| *l)
                .collect::<Vec<_>>()
                .join("\n");

            let parsed = parse_definition(&name, &body);
            sigs.extend(parsed);

            i = body_end + 1;
        } else {
            i += 1;
        }
    }

    MinedModel {
        sigs,
        fact_candidates: Vec::new(),
    }
}

/// Parse `"SomeName": {` and return the name.
fn parse_definition_key(line: &str) -> Option<String> {
    let rest = line.strip_prefix('"')?;
    let end = rest.find('"')?;
    let name = &rest[..end];
    let after = rest[end + 1..].trim();
    if after.starts_with(':') {
        let after_colon = after[1..].trim();
        if after_colon.starts_with('{') {
            return Some(name.to_string());
        }
    }
    None
}

/// Parse a single definition body and return extracted sigs.
fn parse_definition(name: &str, body: &str) -> Vec<MinedSig> {
    let mut sigs = Vec::new();
    let loc = format!("schema definition {name}");

    if has_json_key(body, "enum") {
        // Simple enum: "enum": ["A", "B", ...]
        let variants = extract_enum_values(body);
        sigs.push(MinedSig {
            name: name.to_string(),
            fields: vec![],
            is_abstract: true,
                is_var: false,
            parent: None,
            source_location: loc.clone(),
            intersection_of: vec![], module: None,
        });
        for v in variants {
            sigs.push(MinedSig {
                name: v,
                fields: vec![],
                is_abstract: false,
                is_var: false,
                parent: Some(name.to_string()),
                source_location: loc.clone(),
                intersection_of: vec![], module: None,
            });
        }
    } else if body.contains("\"oneOf\"") && body.contains("\"discriminator\"") {
        // Discriminated union with kind
        sigs.push(MinedSig {
            name: name.to_string(),
            fields: vec![],
            is_abstract: true,
                is_var: false,
            parent: None,
            source_location: loc.clone(),
            intersection_of: vec![], module: None,
        });
        let variant_sigs = extract_discriminated_variants(name, body, &loc);
        sigs.extend(variant_sigs);
    } else if body.contains("\"type\": \"object\"") {
        // Regular struct with properties
        let fields = extract_object_properties(body);
        sigs.push(MinedSig {
            name: name.to_string(),
            fields,
            is_abstract: false,
                is_var: false,
            parent: None,
            source_location: loc,
            intersection_of: vec![], module: None,
        });
    } else if body.contains("\"type\": \"string\"") {
        // Bare abstract sig (enum with no known variants)
        sigs.push(MinedSig {
            name: name.to_string(),
            fields: vec![],
            is_abstract: true,
                is_var: false,
            parent: None,
            source_location: loc,
            intersection_of: vec![], module: None,
        });
    }

    sigs
}

/// Check if the body has a top-level JSON key (not nested too deep).
fn has_json_key(body: &str, key: &str) -> bool {
    let pattern = format!("\"{}\"", key);
    body.contains(&pattern)
}

/// Extract enum values from `"enum": ["A", "B", ...]`.
fn extract_enum_values(body: &str) -> Vec<String> {
    let mut values = Vec::new();
    let pos = match body.find("\"enum\"") {
        Some(p) => p,
        None => return values,
    };
    let rest = &body[pos..];
    let bracket_start = match rest.find('[') {
        Some(p) => p,
        None => return values,
    };
    let after = &rest[bracket_start + 1..];
    let bracket_end = match after.find(']') {
        Some(p) => p,
        None => return values,
    };
    let content = &after[..bracket_end];
    for part in content.split(',') {
        let trimmed = part.trim();
        if let Some(val) = strip_quotes(trimmed) {
            values.push(val.to_string());
        }
    }
    values
}

/// Extract fields from a "properties" block of an object definition.
fn extract_object_properties(body: &str) -> Vec<MinedField> {
    let mut fields = Vec::new();

    // Find "properties": { block
    let props_pos = match body.find("\"properties\"") {
        Some(p) => p,
        None => return fields,
    };
    let after_props = &body[props_pos..];
    let brace_start = match after_props.find('{') {
        Some(p) => p,
        None => return fields,
    };
    let props_body_start = props_pos + brace_start;

    // Find matching } for properties block
    let props_block = match extract_brace_block(body, props_body_start) {
        Some(b) => b,
        None => return fields,
    };

    // Parse each field in the properties block
    parse_field_entries(&props_block, &mut fields);

    fields
}

/// Extract discriminated union variants from oneOf blocks.
fn extract_discriminated_variants(parent_name: &str, body: &str, loc: &str) -> Vec<MinedSig> {
    let mut sigs = Vec::new();

    // Find "oneOf": [ block
    let oneof_pos = match body.find("\"oneOf\"") {
        Some(p) => p,
        None => return sigs,
    };
    let rest = &body[oneof_pos..];
    let bracket_pos = match rest.find('[') {
        Some(p) => p,
        None => return sigs,
    };

    // Split the oneOf array into individual variant objects
    let arr_start = oneof_pos + bracket_pos + 1;
    let variants = split_array_objects(body, arr_start);

    for variant_body in &variants {
        // Extract the variant name from "const": "Name"
        let variant_name = match extract_const_value(variant_body) {
            Some(n) => n,
            None => continue,
        };

        // Extract fields from the variant's properties (excluding "kind")
        let mut fields = Vec::new();
        if let Some(props_pos) = variant_body.find("\"properties\"") {
            let after = &variant_body[props_pos..];
            if let Some(brace) = after.find('{') {
                let block_start = props_pos + brace;
                if let Some(props_block) = extract_brace_block(variant_body, block_start) {
                    parse_field_entries(&props_block, &mut fields);
                }
            }
        }

        sigs.push(MinedSig {
            name: variant_name,
            fields,
            is_abstract: false,
                is_var: false,
            parent: Some(parent_name.to_string()),
            source_location: loc.to_string(),
            intersection_of: vec![], module: None,
        });
    }

    sigs
}

/// Extract the text inside a matched brace pair starting at `start`.
fn extract_brace_block(source: &str, start: usize) -> Option<String> {
    let bytes = source.as_bytes();
    if start >= bytes.len() || bytes[start] != b'{' {
        return None;
    }
    let mut depth = 0i32;
    let mut end = start;
    for (j, &b) in bytes[start..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + j;
                    break;
                }
            }
            _ => {}
        }
    }
    Some(source[start..=end].to_string())
}

/// Split an array of JSON objects into separate strings for each object.
fn split_array_objects(source: &str, start: usize) -> Vec<String> {
    let mut objects = Vec::new();
    let mut i = start;
    let bytes = source.as_bytes();
    let len = bytes.len();

    while i < len {
        match bytes[i] {
            b'{' => {
                if let Some(block) = extract_brace_block(source, i) {
                    let block_len = block.len();
                    objects.push(block);
                    i += block_len;
                } else {
                    i += 1;
                }
            }
            b']' => break,
            _ => i += 1,
        }
    }

    objects
}

/// Parse field entries from a properties block content.
/// Each entry looks like: "fieldName": { ... }
fn parse_field_entries(block: &str, fields: &mut Vec<MinedField>) {
    let bytes = block.as_bytes();
    let len = bytes.len();
    let mut i = 1; // skip opening {

    while i < len {
        // Find next quoted string
        let q_start = match find_char(bytes, b'"', i) {
            Some(p) => p,
            None => break,
        };
        let q_end = match find_char(bytes, b'"', q_start + 1) {
            Some(p) => p,
            None => break,
        };
        let field_name = &block[q_start + 1..q_end];

        // After the name, look for : {
        let after = &block[q_end + 1..];
        let trimmed = after.trim_start();
        if !trimmed.starts_with(':') {
            i = q_end + 1;
            continue;
        }
        let after_colon = trimmed[1..].trim_start();
        if !after_colon.starts_with('{') {
            i = q_end + 1;
            continue;
        }

        // Skip "kind" field
        if field_name == "kind" {
            // Skip past this field's block
            let brace_pos = block.len() - after_colon.len();
            if let Some(fb) = extract_brace_block(block, brace_pos) {
                i = brace_pos + fb.len();
            } else {
                i = q_end + 1;
            }
            continue;
        }

        // Extract the field's value block
        let brace_pos = block.len() - after_colon.len();
        if let Some(field_block) = extract_brace_block(block, brace_pos) {
            if let Some(field) = classify_field(field_name, &field_block) {
                fields.push(field);
            }
            i = brace_pos + field_block.len();
        } else {
            i = q_end + 1;
        }
    }
}

fn find_char(bytes: &[u8], ch: u8, from: usize) -> Option<usize> {
    for i in from..bytes.len() {
        if bytes[i] == ch {
            return Some(i);
        }
    }
    None
}

/// Classify a field from its JSON Schema body into a MinedField.
fn classify_field(name: &str, body: &str) -> Option<MinedField> {
    // Case 1: "oneOf": [...{"$ref":...}, {"type":"null"}] -> Lone
    if body.contains("\"oneOf\"") && body.contains("\"null\"") {
        if let Some(target) = extract_ref_from(body) {
            return Some(MinedField {
                name: name.to_string(),
                is_var: false,
                mult: MinedMultiplicity::Lone,
                target,
                raw_union_type: None,
            });
        }
    }

    // Case 2: "type": "array", "items": {"$ref": ...} -> Set (if uniqueItems) or Seq
    if body.contains("\"array\"") {
        if let Some(items_pos) = body.find("\"items\"") {
            let rest = &body[items_pos..];
            if let Some(target) = extract_ref_from(rest) {
                let mult = if body.contains("\"uniqueItems\"") && body.contains("true") {
                    MinedMultiplicity::Set
                } else {
                    MinedMultiplicity::Seq
                };
                return Some(MinedField {
                    name: name.to_string(),
                    is_var: false,
                    mult,
                    target,
                    raw_union_type: None,
                });
            }
        }
    }

    // Case 3: "$ref": "#/definitions/X" -> One
    if let Some(target) = extract_ref_from(body) {
        return Some(MinedField {
            name: name.to_string(),
            is_var: false,
            mult: MinedMultiplicity::One,
            target,
            raw_union_type: None,
        });
    }

    None
}

/// Extract target from `"$ref": "#/definitions/X"`.
fn extract_ref_from(body: &str) -> Option<String> {
    let pos = body.find("\"$ref\"")?;
    let rest = &body[pos + 6..]; // skip "$ref"
    let colon_pos = rest.find(':')?;
    let after_colon = rest[colon_pos + 1..].trim_start();
    let val = strip_quotes(after_colon)?;
    val.strip_prefix("#/definitions/").map(|s| s.to_string())
}

/// Extract value from `"const": "Name"`.
fn extract_const_value(body: &str) -> Option<String> {
    let pos = body.find("\"const\"")?;
    let rest = &body[pos + 7..]; // skip "const"
    let colon_pos = rest.find(':')?;
    let after_colon = rest[colon_pos + 1..].trim_start();
    strip_quotes(after_colon).map(|s| s.to_string())
}

/// Strip surrounding quotes from a string.
fn strip_quotes(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.starts_with('"') {
        let inner = &s[1..];
        let end = inner.find('"')?;
        Some(&inner[..end])
    } else {
        None
    }
}
