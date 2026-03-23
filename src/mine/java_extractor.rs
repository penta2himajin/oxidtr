/// Extracts Alloy model candidates from Java source code.
/// Handles: record → sig, sealed interface → abstract sig, enum → abstract sig,
/// @Nullable/null comment → lone, List<T> → set, T → one.

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

    // Find matching close paren (skipping nested parens from annotations)
    let close = find_matching_paren(rest, paren)?;
    let params_str = &rest[paren + 1..close];
    if params_str.trim().is_empty() {
        return Some((name, vec![]));
    }

    let fields: Vec<MinedField> = split_top_level_commas(params_str)
        .iter()
        .filter_map(|p| parse_java_param(p.trim()))
        .collect();
    Some((name, fields))
}

fn find_matching_paren(s: &str, open_pos: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s[open_pos..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 { return Some(open_pos + i); }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '(' => { depth += 1; current.push(ch); }
            ')' => { depth -= 1; current.push(ch); }
            ',' if depth == 0 => {
                parts.push(current.clone());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
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
    // Strip @Annotation(...) patterns but preserve block comments (e.g., /* @Nullable */)
    let cleaned = strip_java_annotations(param);
    let param = cleaned.trim();
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

    // Map<K, V> → set of K (V info lost)
    if let Some(inner) = strip_wrapper(t, "Map<", ">") {
        let key = inner.split(',').next().unwrap_or(inner).trim();
        return (MinedMultiplicity::Set, key.to_string());
    }
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

fn strip_java_annotations(s: &str) -> String {
    let mut result = s.to_string();
    let mut search_from = 0;
    loop {
        let at_pos = match result[search_from..].find('@') {
            Some(p) => search_from + p,
            None => break,
        };
        // Skip @ inside block comments (e.g., /* @Nullable */)
        let before = &result[..at_pos];
        let open_comment = before.rfind("/*");
        let close_comment = before.rfind("*/");
        let in_comment = match (open_comment, close_comment) {
            (Some(o), Some(c)) => o > c,
            (Some(_), None) => true,
            _ => false,
        };
        if in_comment {
            search_from = at_pos + 1;
            continue;
        }
        let rest = &result[at_pos + 1..];
        let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
        if name_end == 0 { search_from = at_pos + 1; continue; }
        let after_name = &rest[name_end..];
        if after_name.starts_with('(') {
            let mut depth = 0;
            let mut end = 0;
            for (i, ch) in after_name.chars().enumerate() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 { end = i + 1; break; }
                    }
                    _ => {}
                }
            }
            if end > 0 {
                let remove_end = at_pos + 1 + name_end + end;
                result = format!("{}{}", &result[..at_pos], &result[remove_end..]);
                // Don't advance search_from since we removed content
            } else {
                search_from = at_pos + 1;
            }
        } else {
            let remove_end = at_pos + 1 + name_end;
            result = format!("{}{}", &result[..at_pos], &result[remove_end..]);
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// Extract @alloy: comments from lines.
fn extract_alloy_comments(
    lines: impl Iterator<Item = (usize, String)>,
    facts: &mut Vec<MinedFactCandidate>,
) {
    for (ln, line) in lines {
        let trimmed = line.trim();
        let alloy_text = trimmed.strip_prefix("// @alloy: ")
            .or_else(|| trimmed.strip_prefix("/// @alloy: "));
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

/// Reverse-translate a Java expression back to Alloy syntax.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    // .stream().allMatch(v -> body) → all v: Xxx | body
    if let Some(pos) = s.find(".stream().allMatch(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".stream().allMatch(".len()..];
        let arrow = rest.find(" -> ")?;
        let var = rest[..arrow].trim();
        let body = rest[arrow + 4..].trim().trim_end_matches(')');
        let body_alloy = reverse_translate_expr(body.trim()).unwrap_or_else(|| body.trim().to_string());
        return Some(format!("all {var}: {collection} | {body_alloy}"));
    }
    // .stream().noneMatch(v -> body) → no v: Xxx | body
    if let Some(pos) = s.find(".stream().noneMatch(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".stream().noneMatch(".len()..];
        let arrow = rest.find(" -> ")?;
        let var = rest[..arrow].trim();
        let body = rest[arrow + 4..].trim().trim_end_matches(')');
        let body_alloy = reverse_translate_expr(body.trim()).unwrap_or_else(|| body.trim().to_string());
        return Some(format!("no {var}: {collection} | {body_alloy}"));
    }
    // .stream().anyMatch(v -> body) → some v: Xxx | body
    if let Some(pos) = s.find(".stream().anyMatch(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".stream().anyMatch(".len()..];
        let arrow = rest.find(" -> ")?;
        let var = rest[..arrow].trim();
        let body = rest[arrow + 4..].trim().trim_end_matches(')');
        let body_alloy = reverse_translate_expr(body.trim()).unwrap_or_else(|| body.trim().to_string());
        return Some(format!("some {var}: {collection} | {body_alloy}"));
    }
    // .contains(v) → v in xxx
    if let Some(pos) = s.find(".contains(") {
        let collection = s[..pos].trim();
        let rest = &s[pos + ".contains(".len()..];
        let end = rest.find(')')?;
        let element = rest[..end].trim();
        return Some(format!("{element} in {collection}"));
    }
    // .size() → #xxx
    if let Some(pos) = s.find(".size()") {
        let inner = s[..pos].trim();
        return Some(format!("#{inner}"));
    }
    // == / != comparisons
    for (java_op, alloy_op) in &[(" == ", " = "), (" != ", " != "), (" <= ", " <= "),
                                   (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = s.find(java_op) {
            let left = s[..pos].trim();
            let right = s[pos + java_op.len()..].trim();
            let l = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
            let r = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
            return Some(format!("{l}{alloy_op}{r}"));
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

fn extract_java_facts(
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
