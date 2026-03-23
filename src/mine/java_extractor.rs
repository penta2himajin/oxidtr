/// Extracts Alloy model candidates from Java source code.
/// Handles: record → sig, sealed interface → abstract sig, enum → abstract sig,
/// @Nullable/null comment → lone, List<T> → set, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        // public record Foo(Type field, ...) {} → sig
        if let Some((name, fields)) = parse_record(trimmed) {
            sigs.push(MinedSig {
                name,
                fields,
                is_abstract: false,
                parent: None,
                source_location: format!("line {}", line_num + 1),
            });
        }

        // public record Foo(...) implements Parent {} → child sig
        if let Some((child, parent)) = parse_record_implements(trimmed) {
            if let Some(s) = sigs.iter_mut().find(|s| s.name == child) {
                s.parent = Some(parent);
            }
        }

        // public sealed interface Foo permits A, B {} → abstract sig
        if let Some(name) = parse_sealed_interface(trimmed) {
            sigs.push(MinedSig {
                name,
                fields: vec![],
                is_abstract: true,
                parent: None,
                source_location: format!("line {}", line_num + 1),
            });
        }

        // public enum Foo { A, B } → abstract sig + children
        if let Some(name) = parse_enum(trimmed) {
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

        // public static void/boolean ... { → fact candidates
        if trimmed.contains("static ") && trimmed.contains('(') && trimmed.contains('{') {
            let body = collect_block(&mut lines);
            extract_java_facts(&body, line_num, &mut fact_candidates);
        }
    }

    MinedModel { sigs, fact_candidates }
}

fn parse_record(line: &str) -> Option<(String, Vec<MinedField>)> {
    let rest = line.strip_prefix("public record ")?;
    let paren = rest.find('(')?;
    let name: String = rest[..paren].trim().to_string();
    if name.is_empty() { return None; }

    let close = rest.find(')')?;
    let params_str = &rest[paren + 1..close];
    if params_str.trim().is_empty() {
        return Some((name, vec![]));
    }

    let fields: Vec<MinedField> = params_str.split(',')
        .filter_map(|p| parse_java_param(p.trim()))
        .collect();
    Some((name, fields))
}

fn parse_record_implements(line: &str) -> Option<(String, String)> {
    if !line.contains("implements") { return None; }
    let rest = line.strip_prefix("public record ")?;
    let paren = rest.find('(')?;
    let name: String = rest[..paren].trim().to_string();

    let impl_pos = rest.find("implements")?;
    let after = rest[impl_pos + "implements".len()..].trim();
    let parent: String = after.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if parent.is_empty() { None } else { Some((name, parent)) }
}

fn parse_sealed_interface(line: &str) -> Option<String> {
    let rest = line.strip_prefix("public sealed interface ")?;
    let name: String = rest.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_enum(line: &str) -> Option<String> {
    let rest = line.strip_prefix("public enum ")?;
    let name: String = rest.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_java_param(param: &str) -> Option<MinedField> {
    // "Type name" or "List<Type> name" or "Type /* @Nullable */ name"
    let parts: Vec<&str> = param.split_whitespace().collect();
    if parts.len() < 2 { return None; }

    let name = parts.last()?.to_string();
    if name.is_empty() || !name.chars().next()?.is_ascii_lowercase() { return None; }

    // Reconstruct type (everything before the last word)
    let type_str = parts[..parts.len() - 1].join(" ");
    let (mult, target) = java_type_to_mult(&type_str);
    Some(MinedField { name, mult, target })
}

fn java_type_to_mult(java_type: &str) -> (MinedMultiplicity, String) {
    let t = java_type.trim();

    // @Nullable annotation or comment — check before stripping other comments
    if t.contains("/* @Nullable */") {
        let clean = t.replace("/* @Nullable */", "").trim().to_string();
        return (MinedMultiplicity::Lone, strip_block_comments(&clean));
    }
    if t.contains("@Nullable") {
        let clean = t.replace("@Nullable", "").trim().to_string();
        return (MinedMultiplicity::Lone, strip_block_comments(&clean));
    }

    // Strip remaining block comments (e.g., /* @Size see fact: ... */)
    let t = strip_block_comments(t);
    let t = t.trim();

    // Set<T> → set
    if let Some(inner) = strip_wrapper(t, "Set<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    // List<T> → seq
    if let Some(inner) = strip_wrapper(t, "List<", ">") {
        return (MinedMultiplicity::Seq, inner.to_string());
    }
    // Optional<T> → lone
    if let Some(inner) = strip_wrapper(t, "Optional<", ">") {
        return (MinedMultiplicity::Lone, inner.to_string());
    }
    (MinedMultiplicity::One, t.to_string())
}

fn strip_block_comments(s: &str) -> String {
    let mut result = s.to_string();
    while let (Some(start), Some(end)) = (result.find("/*"), result.find("*/")) {
        if end > start {
            result = format!("{}{}", &result[..start], &result[end + 2..]);
        } else {
            break;
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
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

fn extract_java_facts(
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

        if line.contains(".isEmpty()") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- emptiness check".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".isEmpty() check".to_string(),
            });
        }

        if line.contains("assert ") || line.contains("assertEquals") || line.contains("assertTrue") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- assertion".to_string(),
                confidence: Confidence::High,
                source_location: loc.clone(),
                source_pattern: "assert pattern".to_string(),
            });
        }

        if line.contains("throw ") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "throw pattern".to_string(),
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
