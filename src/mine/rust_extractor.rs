/// Extracts Alloy model candidates from Rust source code.
/// Uses lightweight line-based parsing (same approach as check/impl_parser).

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        // struct → sig
        if let Some(name) = parse_type_decl(trimmed, "pub struct ") {
            // Unit struct: "pub struct Foo;" or "pub struct Foo {}" (self-closing)
            let is_unit = trimmed.ends_with(';')
                || !trimmed.contains('{')
                || (trimmed.contains('{') && trimmed.contains('}'));
            if is_unit {
                sigs.push(MinedSig {
                    name,
                    fields: vec![],
                    is_abstract: false,
                    parent: None,
                    source_location: format!("line {}", line_num + 1),
                });
            } else {
                let (fields, body_lines) = collect_struct_fields(&mut lines);
                sigs.push(MinedSig {
                    name,
                    fields,
                    is_abstract: false,
                    parent: None,
                    source_location: format!("line {}", line_num + 1),
                });
                extract_facts_from_lines(&body_lines, line_num, &mut fact_candidates);
            }
        }

        // enum → abstract sig + child sigs
        if let Some(name) = parse_type_decl(trimmed, "pub enum ") {
            let variants = collect_enum_variants(&mut lines);
            sigs.push(MinedSig {
                name: name.clone(),
                fields: vec![],
                is_abstract: true,
                parent: None,
                source_location: format!("line {}", line_num + 1),
            });
            for (vname, vfields) in variants {
                sigs.push(MinedSig {
                    name: vname,
                    fields: vfields,
                    is_abstract: false,
                    parent: Some(name.clone()),
                    source_location: format!("line {}", line_num + 1),
                });
            }
        }

        // Fact candidates from function bodies
        if trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") {
            let body = collect_fn_body(&mut lines);
            extract_facts_from_lines(&body, line_num, &mut fact_candidates);
        }
    }

    MinedModel { sigs, fact_candidates }
}

fn parse_type_decl(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

fn collect_struct_fields(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> (Vec<MinedField>, Vec<(usize, String)>) {
    let mut fields = Vec::new();
    let mut body_lines = Vec::new();
    let mut depth = 1usize;

    for (ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }
        body_lines.push((ln, trimmed.to_string()));

        if let Some(field) = parse_rust_field(trimmed) {
            fields.push(field);
        }
    }
    (fields, body_lines)
}

fn parse_rust_field(line: &str) -> Option<MinedField> {
    let rest = line.strip_prefix("pub ")?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim().to_string();
    if name.is_empty() { return None; }
    let type_str = rest[colon + 1..].trim().trim_end_matches(',').trim();
    if type_str.is_empty() { return None; }
    let (mult, target) = rust_type_to_mult(type_str);
    Some(MinedField { name, mult, target })
}

fn rust_type_to_mult(rust_type: &str) -> (MinedMultiplicity, String) {
    let t = rust_type.trim();
    if let Some(inner) = strip_wrapper(t, "Vec<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "HashSet<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "Option<Box<", ">>") {
        return (MinedMultiplicity::Lone, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "Option<", ">") {
        return (MinedMultiplicity::Lone, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "Box<", ">") {
        return (MinedMultiplicity::One, inner.to_string());
    }
    (MinedMultiplicity::One, t.to_string())
}

fn strip_wrapper<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)?.strip_suffix(suffix)
}

fn collect_enum_variants(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<(String, Vec<MinedField>)> {
    let mut variants = Vec::new();
    let mut depth = 1usize;

    while let Some((_ln, line)) = lines.next() {
        let trimmed = line.trim();
        let open = trimmed.chars().filter(|&c| c == '{').count();
        let close = trimmed.chars().filter(|&c| c == '}').count();
        depth = depth + open - close;
        if depth == 0 { break; }

        let cleaned = trimmed.trim_end_matches(',');
        if cleaned.is_empty() { continue; }
        let first = cleaned.chars().next().unwrap_or(' ');
        if !first.is_ascii_uppercase() { continue; }

        // Struct variant: "VariantName {"
        if let Some(brace_pos) = cleaned.find('{') {
            let name: String = cleaned[..brace_pos].trim().to_string();
            if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.is_empty() {
                let fields = collect_variant_fields(lines, &mut depth);
                variants.push((name, fields));
            }
        } else if cleaned.chars().all(|c| c.is_alphanumeric() || c == '_') {
            // Unit variant
            variants.push((cleaned.to_string(), vec![]));
        }
    }
    variants
}

fn collect_variant_fields(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
    depth: &mut usize,
) -> Vec<MinedField> {
    let target_depth = *depth - 1;
    let mut fields = Vec::new();
    while let Some((_ln, line)) = lines.next() {
        let trimmed = line.trim();
        let open = trimmed.chars().filter(|&c| c == '{').count();
        let close = trimmed.chars().filter(|&c| c == '}').count();
        *depth = *depth + open - close;

        // Variant fields have no `pub` prefix
        let cleaned = trimmed.trim_end_matches(',').trim_end_matches('}').trim();
        if let Some(colon) = cleaned.find(':') {
            let name = cleaned[..colon].trim().to_string();
            if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_ascii_lowercase()) {
                let type_str = cleaned[colon + 1..].trim();
                if !type_str.is_empty() {
                    let (mult, target) = rust_type_to_mult(type_str);
                    fields.push(MinedField { name, mult, target });
                }
            }
        }

        if *depth <= target_depth { break; }
    }
    fields
}

fn collect_fn_body(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<(usize, String)> {
    let mut body = Vec::new();
    let mut depth = 1usize;

    for (ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }
        body.push((ln, trimmed.to_string()));
    }
    body
}

/// Extract fact candidates from code patterns.
fn extract_facts_from_lines(
    body: &[(usize, String)],
    _context_line: usize,
    facts: &mut Vec<MinedFactCandidate>,
) {
    for (ln, line) in body {
        let loc = format!("line {}", ln + 1);

        // assert! / debug_assert! → assert candidate (High)
        if line.contains("assert!(") || line.contains("debug_assert!(") {
            if let Some(cond) = extract_assert_condition(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: cond.clone(),
                    confidence: Confidence::High,
                    source_location: loc.clone(),
                    source_pattern: format!("assert!({cond})"),
                });
            }
        }

        // .is_empty() → fact candidate: no/some (Medium)
        if line.contains(".is_empty()") {
            facts.push(MinedFactCandidate {
                alloy_text: extract_emptiness_fact(line),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".is_empty() check".to_string(),
            });
        }

        // .contains() → fact candidate: in (Medium)
        if line.contains(".contains(") {
            facts.push(MinedFactCandidate {
                alloy_text: extract_contains_fact(line),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".contains() check".to_string(),
            });
        }

        // if ... { return Err } → pred precondition candidate (Low)
        if line.contains("return Err(") || line.contains("return Err ") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "return Err pattern".to_string(),
            });
        }
    }
}

fn extract_assert_condition(line: &str) -> Option<String> {
    let start = line.find("assert!(").or_else(|| line.find("debug_assert!("))?;
    let after_paren = line[start..].find('(')? + start + 1;
    // Simple extraction: content up to last )
    let rest = &line[after_paren..];
    let end = rest.rfind(')')?;
    let cond = rest[..end].trim().to_string();
    if cond.is_empty() { None } else { Some(cond) }
}

fn extract_emptiness_fact(line: &str) -> String {
    // Best effort: extract field chain before .is_empty()
    if let Some(pos) = line.find(".is_empty()") {
        let before = line[..pos].trim().trim_start_matches("if ").trim_start_matches('!');
        format!("no/some {before}")
    } else {
        "-- emptiness check (review)".to_string()
    }
}

fn extract_contains_fact(line: &str) -> String {
    if let Some(pos) = line.find(".contains(") {
        let before = line[..pos].trim().trim_start_matches("if ").trim_start_matches('!');
        let after_paren = pos + ".contains(".len();
        let rest = &line[after_paren..];
        let end = rest.find(')').unwrap_or(rest.len());
        let arg = rest[..end].trim().trim_start_matches('&');
        format!("{arg} in {before}")
    } else {
        "-- contains check (review)".to_string()
    }
}
