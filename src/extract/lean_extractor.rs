/// Extracts Alloy model candidates from Lean 4 source code.
/// Handles: structure → sig, inductive → abstract sig,
/// Option T → lone, List T → set, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with("--") { continue; }

        // structure Foo where
        if let Some(name) = parse_lean_structure(trimmed) {
            let fields = collect_lean_structure_fields(&mut lines);
            sigs.push(MinedSig {
                name,
                fields,
                is_abstract: false,
                is_var: false,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
            continue;
        }

        // inductive Foo where
        if let Some(name) = parse_lean_inductive(trimmed) {
            let variants = collect_lean_inductive_variants(&mut lines, &name);
            sigs.push(MinedSig {
                name: name.clone(),
                fields: vec![],
                is_abstract: true,
                is_var: false,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
            for v in variants {
                sigs.push(MinedSig {
                    name: v,
                    fields: vec![],
                    is_abstract: false,
                    is_var: false,
                    parent: Some(name.clone()),
                    source_location: format!("line {}", line_num + 1),
                    intersection_of: vec![],
                });
            }
            continue;
        }

        // theorem foo : ... := sorry  →  fact candidate
        if let Some(theorem) = parse_lean_theorem(trimmed, line_num) {
            fact_candidates.push(theorem);
        }
    }

    MinedModel { sigs, fact_candidates }
}

fn parse_lean_structure(line: &str) -> Option<String> {
    let rest = line.strip_prefix("structure ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() || !name.chars().next()?.is_ascii_uppercase() { return None; }
    // Must end with "where" (possibly after spaces)
    let after_name = rest[name.len()..].trim();
    if after_name != "where" && !after_name.starts_with("where") { return None; }
    Some(name)
}

fn collect_lean_structure_fields(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<MinedField> {
    let mut fields = Vec::new();

    while let Some(&(_, next_line)) = lines.peek() {
        let trimmed = next_line.trim();
        // Empty line or non-indented line → end of structure
        if trimmed.is_empty() || (!next_line.starts_with(' ') && !next_line.starts_with('\t')) {
            break;
        }
        lines.next();

        // Skip "mk ::" and comments
        if trimmed.starts_with("mk") || trimmed.starts_with("--") { continue; }

        // Parse "fieldName : Type"
        if let Some(field) = parse_lean_field(trimmed) {
            fields.push(field);
        }
    }

    fields
}

fn parse_lean_field(line: &str) -> Option<MinedField> {
    // "fieldName : Type"
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() != 2 { return None; }
    let name = parts[0].trim().to_string();
    if name.is_empty() || name.starts_with('|') || name.starts_with("--") { return None; }
    let type_str = parts[1].trim();
    let (mult, target) = lean_type_to_mult(type_str);
    Some(MinedField {
        name,
        is_var: false,
        mult,
        target,
        raw_union_type: None,
    })
}

fn lean_type_to_mult(lean_type: &str) -> (MinedMultiplicity, String) {
    let t = lean_type.trim();

    // Option T → lone
    if let Some(rest) = t.strip_prefix("Option ") {
        return (MinedMultiplicity::Lone, rest.trim().to_string());
    }

    // List T → set
    if let Some(rest) = t.strip_prefix("List ") {
        let inner = rest.trim();
        // Handle parenthesized types: List (Foo × Bar)
        let inner = if inner.starts_with('(') && inner.ends_with(')') {
            &inner[1..inner.len()-1]
        } else {
            inner
        };
        return (MinedMultiplicity::Set, inner.to_string());
    }

    // Array T → set
    if let Some(rest) = t.strip_prefix("Array ") {
        return (MinedMultiplicity::Set, rest.trim().to_string());
    }

    // Finset T → set
    if let Some(rest) = t.strip_prefix("Finset ") {
        return (MinedMultiplicity::Set, rest.trim().to_string());
    }

    (MinedMultiplicity::One, t.to_string())
}

fn parse_lean_inductive(line: &str) -> Option<String> {
    let rest = line.strip_prefix("inductive ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() || !name.chars().next()?.is_ascii_uppercase() { return None; }
    let after_name = rest[name.len()..].trim();
    if after_name != "where" && !after_name.starts_with("where") { return None; }
    Some(name)
}

fn collect_lean_inductive_variants(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
    _parent_name: &str,
) -> Vec<String> {
    let mut variants = Vec::new();

    while let Some(&(_, next_line)) = lines.peek() {
        let trimmed = next_line.trim();
        if trimmed.is_empty() || (!next_line.starts_with(' ') && !next_line.starts_with('\t')) {
            break;
        }
        lines.next();

        if trimmed.starts_with("--") { continue; }

        // "| variantName : ParentType" or "| variantName : Arg → ParentType"
        if let Some(rest) = trimmed.strip_prefix("| ") {
            let vname: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !vname.is_empty() {
                // Capitalize first letter for sig name
                let sig_name = capitalize_first(&vname);
                variants.push(sig_name);
            }
        }
    }

    variants
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

fn parse_lean_theorem(line: &str, line_num: usize) -> Option<MinedFactCandidate> {
    let rest = line.strip_prefix("theorem ")?;
    // Extract name
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { return None; }

    // Try to extract the body between : and :=
    let after_name = &rest[name.len()..].trim();
    if let Some(colon_pos) = after_name.find(':') {
        let body_start = colon_pos + 1;
        let body = if let Some(assign_pos) = after_name.find(":=") {
            after_name[body_start..assign_pos].trim()
        } else {
            after_name[body_start..].trim()
        };

        if !body.is_empty() {
            return Some(MinedFactCandidate {
                alloy_text: format!("-- theorem {name}: {body}"),
                confidence: Confidence::Low,
                source_location: format!("line {}", line_num + 1),
                source_pattern: "lean-theorem".to_string(),
            });
        }
    }

    None
}
