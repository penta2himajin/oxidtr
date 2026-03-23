/// Extracts Alloy model candidates from TypeScript source code.
/// Handles: interface → sig, discriminated union → abstract sig + sub sigs,
/// T | null → lone, T[] → set, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    // Extract top-level @alloy comments
    extract_alloy_comments(
        source.lines().enumerate().map(|(ln, line)| (ln, line.to_string())),
        &mut fact_candidates,
    );

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        // export interface Foo { ... } → sig
        if let Some(name) = parse_interface_decl(trimmed) {
            // Self-closing interface on one line: "export interface Foo {}"
            let fields = if trimmed.contains('{') && trimmed.contains('}') {
                vec![]
            } else {
                collect_interface_fields(&mut lines)
            };
            // Skip discriminant-only interfaces (will be handled as union variants)
            let real_fields: Vec<MinedField> = fields
                .into_iter()
                .filter(|f| f.name != "kind")
                .collect();
            sigs.push(MinedSig {
                name,
                fields: real_fields,
                is_abstract: false,
                parent: None,
                source_location: format!("line {}", line_num + 1),
            });
        }

        // export type Foo = "A" | "B" → abstract sig + one sig children (string literal union)
        // export type Foo = A | B     → abstract sig + sub sig children (discriminated union)
        if let Some((name, variants)) = parse_type_union(trimmed) {
            let is_string_literal = variants.iter().all(|v| v.starts_with('"'));
            if is_string_literal {
                // String literal union → abstract sig + one sig per literal
                sigs.push(MinedSig {
                    name: name.clone(),
                    fields: vec![],
                    is_abstract: true,
                    parent: None,
                    source_location: format!("line {}", line_num + 1),
                });
                for v in &variants {
                    let vname = v.trim_matches('"').to_string();
                    sigs.push(MinedSig {
                        name: vname,
                        fields: vec![],
                        is_abstract: false,
                        parent: Some(name.clone()),
                        source_location: format!("line {}", line_num + 1),
                    });
                }
            } else {
                // Discriminated union → mark parent as abstract, set parent on existing sigs
                sigs.push(MinedSig {
                    name: name.clone(),
                    fields: vec![],
                    is_abstract: true,
                    parent: None,
                    source_location: format!("line {}", line_num + 1),
                });
                // Set parent on existing sigs that match variant names
                for v in &variants {
                    if let Some(existing) = sigs.iter_mut().find(|s| s.name == *v) {
                        existing.parent = Some(name.clone());
                    }
                }
            }
        }

        // Fact candidates from function bodies
        if trimmed.starts_with("export function ") || trimmed.starts_with("function ") {
            let body = collect_block(&mut lines);
            extract_ts_facts(&body, line_num, &mut fact_candidates);
        }
    }

    MinedModel { sigs, fact_candidates }
}

fn parse_interface_decl(line: &str) -> Option<String> {
    let rest = line.strip_prefix("export interface ")
        .or_else(|| line.strip_prefix("interface "))?;
    let name: String = rest.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_type_union(line: &str) -> Option<(String, Vec<String>)> {
    let rest = line.strip_prefix("export type ")
        .or_else(|| line.strip_prefix("type "))?;
    let eq_pos = rest.find('=')?;
    let name: String = rest[..eq_pos].trim().to_string();
    if name.is_empty() { return None; }

    let rhs = rest[eq_pos + 1..].trim().trim_end_matches(';').trim();
    let variants: Vec<String> = rhs.split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if variants.len() < 2 { return None; }

    Some((name, variants))
}

fn collect_interface_fields(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<MinedField> {
    let mut fields = Vec::new();
    let mut depth = 1usize;

    for (_ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }

        if let Some(field) = parse_ts_field(trimmed) {
            fields.push(field);
        }
    }
    fields
}

fn parse_ts_field(line: &str) -> Option<MinedField> {
    let trimmed = line.trim().trim_end_matches(';').trim_end_matches(',').trim();
    if trimmed.is_empty() || trimmed.starts_with("//") { return None; }

    // "readonly kind: "Foo"" → skip (discriminant)
    let rest = trimmed.strip_prefix("readonly ").unwrap_or(trimmed);

    let colon = rest.find(':')?;
    let mut name = rest[..colon].trim().to_string();
    let optional = name.ends_with('?');
    if optional {
        name = name.trim_end_matches('?').to_string();
    }
    if name.is_empty() { return None; }

    let type_str = rest[colon + 1..].trim();
    if type_str.is_empty() { return None; }

    // Check for string literal type (discriminant field)
    if type_str.starts_with('"') && type_str.ends_with('"') {
        // This is a discriminant like kind: "Foo" — include as-is to let caller filter
        return Some(MinedField {
            name,
            mult: MinedMultiplicity::One,
            target: type_str.trim_matches('"').to_string(),
        });
    }

    let (mult, target) = ts_type_to_mult(type_str, optional);
    Some(MinedField { name, mult, target })
}

fn ts_type_to_mult(ts_type: &str, optional: bool) -> (MinedMultiplicity, String) {
    let t = ts_type.trim();

    // Map<K, V> → set of K (V info lost)
    if let Some(inner) = strip_wrapper_ts(t, "Map<", ">") {
        let key = inner.split(',').next().unwrap_or(inner).trim();
        return (MinedMultiplicity::Set, key.to_string());
    }
    // Set<T> → set
    if let Some(inner) = strip_wrapper_ts(t, "Set<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    // ReadonlySet<T> → set
    if let Some(inner) = strip_wrapper_ts(t, "ReadonlySet<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    // T[] → seq
    if let Some(inner) = t.strip_suffix("[]") {
        return (MinedMultiplicity::Seq, inner.to_string());
    }
    // Array<T> → seq
    if let Some(inner) = strip_wrapper_ts(t, "Array<", ">") {
        return (MinedMultiplicity::Seq, inner.to_string());
    }
    // ReadonlyArray<T> → seq
    if let Some(inner) = strip_wrapper_ts(t, "ReadonlyArray<", ">") {
        return (MinedMultiplicity::Seq, inner.to_string());
    }
    // T | null or null | T → lone
    if t.contains(" | null") || t.contains("null | ") {
        let target = t.replace(" | null", "").replace("null | ", "").trim().to_string();
        return (MinedMultiplicity::Lone, target);
    }
    // Optional field (field?: T) → lone
    if optional {
        return (MinedMultiplicity::Lone, t.to_string());
    }
    (MinedMultiplicity::One, t.to_string())
}

fn strip_wrapper_ts<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)?.strip_suffix(suffix)
}

fn collect_block(
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

/// Extract @alloy: comments from lines.
fn extract_alloy_comments(
    lines: impl Iterator<Item = (usize, String)>,
    facts: &mut Vec<MinedFactCandidate>,
) {
    for (ln, line) in lines {
        let trimmed = line.trim();
        let alloy_text = trimmed.strip_prefix("// @alloy: ")
            .or_else(|| trimmed.strip_prefix("/// @alloy: "))
            .or_else(|| {
                // Also check inside JSDoc: " * @alloy: ..."
                trimmed.strip_prefix("* @alloy: ")
            });
        if let Some(text) = alloy_text {
            facts.push(MinedFactCandidate {
                alloy_text: text.trim().to_string(),
                confidence: Confidence::High,
                source_location: format!("line {}", ln + 1),
                source_pattern: "@alloy comment".to_string(),
            });
        }
    }
}

/// Reverse-translate a TypeScript expression back to Alloy syntax.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    // .every(v => body) → all v: Xxx | body
    if let Some(pos) = s.find(".every(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".every(".len()..];
        let arrow = rest.find("=>")?;
        let var = rest[..arrow].trim();
        let body = rest[arrow + 2..].trim().trim_end_matches(')');
        let body_alloy = reverse_translate_expr(body).unwrap_or_else(|| body.to_string());
        return Some(format!("all {var}: {collection} | {body_alloy}"));
    }
    // .some(v => body) → some v: Xxx | body
    if let Some(pos) = s.find(".some(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".some(".len()..];
        let arrow = rest.find("=>")?;
        let var = rest[..arrow].trim();
        let body = rest[arrow + 2..].trim().trim_end_matches(')');
        let body_alloy = reverse_translate_expr(body).unwrap_or_else(|| body.to_string());
        return Some(format!("some {var}: {collection} | {body_alloy}"));
    }
    // .includes(v) → v in xxx
    if let Some(pos) = s.find(".includes(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".includes(".len()..];
        let end = rest.find(')')?;
        let element = rest[..end].trim();
        return Some(format!("{element} in {collection}"));
    }
    // .has(v) → v in xxx
    if let Some(pos) = s.find(".has(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".has(".len()..];
        let end = rest.find(')')?;
        let element = rest[..end].trim();
        return Some(format!("{element} in {collection}"));
    }
    // .length → #xxx or .size → #xxx
    if let Some(pos) = s.find(".length") {
        let inner = s[..pos].trim();
        return Some(format!("#{inner}"));
    }
    if let Some(pos) = s.find(".size") {
        let inner = s[..pos].trim();
        return Some(format!("#{inner}"));
    }
    // === / !== → = / !=
    for (ts_op, alloy_op) in &[(" === ", " = "), (" !== ", " != "), (" <= ", " <= "),
                                 (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = s.find(ts_op) {
            let left = s[..pos].trim();
            let right = s[pos + ts_op.len()..].trim();
            let left_alloy = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
            let right_alloy = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
            return Some(format!("{left_alloy}{alloy_op}{right_alloy}"));
        }
    }
    // && → and, || → or
    if let Some(pos) = s.find(" && ") {
        let left = s[..pos].trim();
        let right = s[pos + 4..].trim();
        let l = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
        let r = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
        return Some(format!("{l} and {r}"));
    }
    if let Some(pos) = s.find(" || ") {
        let left = s[..pos].trim();
        let right = s[pos + 4..].trim();
        let l = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
        let r = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
        return Some(format!("{l} or {r}"));
    }
    if s.starts_with('!') {
        let inner = s[1..].trim();
        let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
        return Some(format!("not {inner_alloy}"));
    }
    None
}

fn extract_ts_facts(
    body: &[(usize, String)],
    _context_line: usize,
    facts: &mut Vec<MinedFactCandidate>,
) {
    // Extract @alloy comments from function body
    extract_alloy_comments(
        body.iter().map(|(ln, line)| (*ln, line.clone())),
        facts,
    );

    for (ln, line) in body {
        let loc = format!("line {}", ln + 1);

        // .includes() → fact candidate: in (Medium)
        if line.contains(".includes(") {
            facts.push(MinedFactCandidate {
                alloy_text: extract_includes_fact(line),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".includes() check".to_string(),
            });
        }

        // .length === 0 or .length > 0 → fact candidate (Medium)
        if line.contains(".length") {
            facts.push(MinedFactCandidate {
                alloy_text: extract_length_fact(line),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".length check".to_string(),
            });
        }

        // === null or !== null → lone field fact (Medium)
        if line.contains("=== null") || line.contains("!== null") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- null check (lone field constraint)".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "null check".to_string(),
            });
        }

        // throw new Error → precondition guard (Low)
        if line.contains("throw new Error") || line.contains("throw ") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "throw pattern".to_string(),
            });
        }
    }
}

fn extract_includes_fact(line: &str) -> String {
    if let Some(pos) = line.find(".includes(") {
        let before = line[..pos].trim();
        let after_paren = pos + ".includes(".len();
        let rest = &line[after_paren..];
        let end = rest.find(')').unwrap_or(rest.len());
        let arg = rest[..end].trim();
        format!("{arg} in {before}")
    } else {
        "-- includes check (review)".to_string()
    }
}

fn extract_length_fact(line: &str) -> String {
    if let Some(pos) = line.find(".length") {
        let before = line[..pos].trim();
        format!("#{before} constraint")
    } else {
        "-- length check (review)".to_string()
    }
}
