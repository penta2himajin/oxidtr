/// Extracts Alloy model candidates from Kotlin source code.
/// Handles: data class → sig, sealed class/enum class → abstract sig,
/// T? → lone, List<T> → set, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        // data class Foo(...) → sig
        if let Some(name) = parse_data_class(trimmed) {
            // Collect full constructor text (may span multiple lines)
            let full_text = if trimmed.contains(')') {
                trimmed.to_string()
            } else {
                collect_until_close_paren(trimmed, &mut lines)
            };
            let fields = extract_constructor_params(&full_text);
            sigs.push(MinedSig {
                name,
                fields,
                is_abstract: false,
                parent: None,
                source_location: format!("line {}", line_num + 1),
            });
            // Check for extends (": Parent()")
            if let Some((_child, parent)) = parse_extends(&full_text) {
                sigs.last_mut().unwrap().parent = Some(parent);
            }
        }

        // data object Foo : Parent() → child sig (unit)
        if let Some((name, parent)) = parse_data_object(trimmed) {
            sigs.push(MinedSig {
                name,
                fields: vec![],
                is_abstract: false,
                parent: Some(parent),
                source_location: format!("line {}", line_num + 1),
            });
        }

        // sealed class Foo → abstract sig
        if let Some(name) = parse_sealed_class(trimmed) {
            sigs.push(MinedSig {
                name,
                fields: vec![],
                is_abstract: true,
                parent: None,
                source_location: format!("line {}", line_num + 1),
            });
        }

        // enum class Foo { A, B } → abstract sig + children
        if let Some(name) = parse_enum_class(trimmed) {
            let variants = collect_enum_entries(&mut lines);
            sigs.push(MinedSig {
                name: name.clone(),
                fields: vec![],
                is_abstract: true,
                parent: None,
                source_location: format!("line {}", line_num + 1),
            });
            for v in variants {
                sigs.push(MinedSig {
                    name: v,
                    fields: vec![],
                    is_abstract: false,
                    parent: Some(name.clone()),
                    source_location: format!("line {}", line_num + 1),
                });
            }
        }

        // data class Foo(...) : Parent() → set parent on existing sig
        if let Some((child, parent)) = parse_extends(trimmed) {
            if let Some(s) = sigs.iter_mut().find(|s| s.name == child) {
                s.parent = Some(parent);
            }
        }

        // fun ... { → fact candidates from body
        if trimmed.starts_with("fun ") {
            let body = collect_block(&mut lines);
            extract_kt_facts(&body, line_num, &mut fact_candidates);
        }
    }

    MinedModel { sigs, fact_candidates }
}

fn collect_until_close_paren(
    first_line: &str,
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> String {
    let mut full = first_line.to_string();
    for (_ln, line) in lines.by_ref() {
        full.push_str(" ");
        full.push_str(line.trim());
        if line.contains(')') { break; }
    }
    full
}

fn parse_data_class(line: &str) -> Option<String> {
    let rest = line.strip_prefix("data class ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_data_object(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("data object ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { return None; }
    let after_name = rest[name.len()..].trim();
    let after_colon = after_name.strip_prefix(':')?;
    let parent: String = after_colon.trim().chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if parent.is_empty() { None } else { Some((name, parent)) }
}

fn parse_sealed_class(line: &str) -> Option<String> {
    let rest = line.strip_prefix("sealed class ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_enum_class(line: &str) -> Option<String> {
    let rest = line.strip_prefix("enum class ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_extends(line: &str) -> Option<(String, String)> {
    // "data class Foo(...) : Parent()" or ") : Parent()"
    if !line.contains(") :") { return None; }
    let name = if line.starts_with("data class ") {
        let rest = &line["data class ".len()..];
        rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>()
    } else {
        return None;
    };
    if name.is_empty() { return None; }

    let colon_pos = line.find(") :")?;
    let after = line[colon_pos + 3..].trim();
    let parent: String = after.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if parent.is_empty() { None } else { Some((name, parent)) }
}

fn extract_constructor_params(line: &str) -> Vec<MinedField> {
    let open = match line.find('(') { Some(p) => p + 1, None => return vec![] };
    let close = match line.rfind(')') { Some(p) => p, None => return vec![] };
    if open >= close { return vec![]; }

    let params = &line[open..close];
    params.split(',')
        .filter_map(|p| parse_kt_param(p.trim()))
        .collect()
}

fn parse_kt_param(param: &str) -> Option<MinedField> {
    // "val name: Type" or "val name: Type?"
    let rest = param.strip_prefix("val ").or_else(|| param.strip_prefix("var "))?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim().to_string();
    if name.is_empty() { return None; }
    let type_str = rest[colon + 1..].trim();
    let (mult, target) = kt_type_to_mult(type_str);
    Some(MinedField { name, mult, target })
}

fn kt_type_to_mult(kt_type: &str) -> (MinedMultiplicity, String) {
    let t = kt_type.trim();
    // T? → lone
    if let Some(inner) = t.strip_suffix('?') {
        return (MinedMultiplicity::Lone, inner.to_string());
    }
    // List<T> → set
    if let Some(inner) = strip_wrapper(t, "List<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "MutableList<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "Set<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    (MinedMultiplicity::One, t.to_string())
}

fn strip_wrapper<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)?.strip_suffix(suffix)
}

fn collect_enum_entries(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<String> {
    let mut entries = Vec::new();
    let mut depth = 1usize;
    for (_ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }
        // Entries: "A, B, C" or "A," per line
        for part in trimmed.split(',') {
            let entry = part.trim().trim_end_matches(';');
            if !entry.is_empty()
                && entry.chars().next().map_or(false, |c| c.is_ascii_uppercase())
                && entry.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                entries.push(entry.to_string());
            }
        }
    }
    entries
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

fn extract_kt_facts(
    body: &[(usize, String)],
    _context_line: usize,
    facts: &mut Vec<MinedFactCandidate>,
) {
    for (ln, line) in body {
        let loc = format!("line {}", ln + 1);

        if line.contains(".contains(") {
            facts.push(MinedFactCandidate {
                alloy_text: extract_contains_fact(line),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".contains() check".to_string(),
            });
        }

        if line.contains(".isEmpty()") || line.contains(".isNotEmpty()") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- emptiness check".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".isEmpty()/.isNotEmpty() check".to_string(),
            });
        }

        if line.contains("require(") || line.contains("check(") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition (require/check)".to_string(),
                confidence: Confidence::High,
                source_location: loc.clone(),
                source_pattern: "require/check".to_string(),
            });
        }

        if line.contains("throw ") || line.contains("TODO(") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "throw/TODO pattern".to_string(),
            });
        }
    }
}

fn extract_contains_fact(line: &str) -> String {
    if let Some(pos) = line.find(".contains(") {
        let before = line[..pos].trim();
        let after_paren = pos + ".contains(".len();
        let rest = &line[after_paren..];
        let end = rest.find(')').unwrap_or(rest.len());
        let arg = rest[..end].trim();
        format!("{arg} in {before}")
    } else {
        "-- contains check (review)".to_string()
    }
}
