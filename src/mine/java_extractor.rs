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

        // Invariant functions: public static boolean assertXxx(...) { return BODY; }
        if trimmed.contains("static boolean ") && trimmed.contains('(') && trimmed.contains('{') {
            let fn_name = extract_java_fn_name(trimmed);
            let body = collect_block(&mut lines);
            if let Some(ref name) = fn_name {
                if name.starts_with("assert") && name.len() > 6 && name.as_bytes()[6].is_ascii_uppercase() {
                    let body_text = body.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join(" ");
                    if let Some(alloy_text) = reverse_translate_invariant_body(&body_text) {
                        fact_candidates.push(MinedFactCandidate {
                            alloy_text,
                            confidence: Confidence::Medium,
                            source_location: format!("line {}", line_num + 1),
                            source_pattern: format!("reverse-translated fn {name}"),
                        });
                    }
                }
            }
            extract_java_facts(&body, line_num, &mut fact_candidates);
        } else if trimmed.contains("static ") && trimmed.contains('(') && trimmed.contains('{') {
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

/// Extract function name from a Java method declaration line.
fn extract_java_fn_name(line: &str) -> Option<String> {
    // "public static boolean assertXxx(..." → "assertXxx"
    let s = line.trim();
    let bool_pos = s.find("boolean ")?;
    let rest = &s[bool_pos + "boolean ".len()..];
    let paren = rest.find('(')?;
    let name = rest[..paren].trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Reverse-translate the body of a Java invariant function.
/// Converts camelCase plural param names back to PascalCase sig names.
fn reverse_translate_invariant_body(body_text: &str) -> Option<String> {
    let s = body_text.trim();
    let expr = if let Some(pos) = s.find("return ") {
        s[pos + "return ".len()..].trim().trim_end_matches(';').trim()
    } else {
        s.trim_end_matches(';').trim()
    };
    if expr.is_empty() { return None; }
    let alloy = reverse_translate_expr(expr)?;
    Some(fix_quantifier_domains(&alloy))
}

/// Fix quantifier domains: "all s: sigDecls | ..." → "all s: SigDecl | ..."
fn fix_quantifier_domains(alloy: &str) -> String {
    let mut result = alloy.to_string();
    for quant in &["all ", "some ", "no "] {
        if let Some(qpos) = result.find(quant) {
            let after_quant = &result[qpos + quant.len()..];
            if let Some(colon) = after_quant.find(": ") {
                let domain_start = qpos + quant.len() + colon + 2;
                let after_domain = &result[domain_start..];
                if let Some(pipe) = after_domain.find(" | ") {
                    let domain = &result[domain_start..domain_start + pipe];
                    let converted = camel_to_pascal(domain);
                    if converted != domain {
                        result = format!(
                            "{}{}{}",
                            &result[..domain_start],
                            converted,
                            &result[domain_start + pipe..]
                        );
                    }
                }
            }
        }
    }
    result
}

/// Reverse-translate a Java expression back to Alloy syntax.
/// Robust: handles balanced parens, TC calls, stream API patterns.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    if s.is_empty() { return None; }

    let s = strip_balanced_parens(s);

    // tcField(base) → base.^field
    if let Some(result) = try_reverse_tc_call(s) {
        return Some(result);
    }

    // .stream().noneMatch(v -> body) → no v: Xxx | body (must come before anyMatch)
    if let Some(result) = try_reverse_stream_method(s, ".stream().noneMatch(", "no") {
        return Some(result);
    }

    // .stream().allMatch(v -> body) → all v: Xxx | body
    if let Some(result) = try_reverse_stream_method(s, ".stream().allMatch(", "all") {
        return Some(result);
    }

    // !xxx.stream().anyMatch(v -> body) → no v: Xxx | body
    if s.starts_with('!') {
        let inner = &s[1..];
        if let Some(result) = try_reverse_stream_method(inner, ".stream().anyMatch(", "no") {
            return Some(result);
        }
    }

    // .stream().anyMatch(v -> body) → some v: Xxx | body
    if let Some(result) = try_reverse_stream_method(s, ".stream().anyMatch(", "some") {
        return Some(result);
    }

    // .contains(v) → v in xxx
    if let Some(result) = try_reverse_java_contains(s) {
        return Some(result);
    }

    // .size() → #xxx
    if let Some(result) = try_reverse_java_size(s) {
        return Some(result);
    }

    // Comparison operators
    if let Some(result) = try_reverse_comparison(s) {
        return Some(result);
    }

    // Boolean logic
    if let Some(result) = try_reverse_logic(s) {
        return Some(result);
    }

    // Integer literals
    if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        return Some(s.to_string());
    }

    // Variable references and field access chains (including Java record accessor calls like s.field())
    // Strip trailing () from Java record accessor: s.parent() → s.parent
    let stripped = strip_java_accessors(s);
    if stripped.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !stripped.is_empty() {
        return Some(stripped);
    }

    None
}

/// Strip Java record accessor `()` calls: `s.parent()` → `s.parent`
fn strip_java_accessors(s: &str) -> String {
    // Handle chains like s.parent().field() → s.parent.field
    s.replace("()", "")
}

fn strip_balanced_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') { return s; }
    let inner = &s[1..s.len() - 1];
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch { '(' => depth += 1, ')' => { depth -= 1; if depth < 0 { return s; } } _ => {} }
    }
    if depth == 0 { inner.trim() } else { s }
}

fn try_reverse_tc_call(s: &str) -> Option<String> {
    let rest = s.strip_prefix("tc")?;
    if rest.is_empty() || !rest.chars().next()?.is_ascii_uppercase() { return None; }
    let paren = rest.find('(')?;
    let field_pascal = &rest[..paren];
    let field = {
        let mut chars = field_pascal.chars();
        match chars.next() {
            None => return None,
            Some(c) => format!("{}{}", c.to_lowercase(), chars.as_str()),
        }
    };
    let close = find_matching_close_java(&rest[paren + 1..])?;
    let args = &rest[paren + 1..paren + 1 + close];
    let base_alloy = reverse_translate_expr(args.trim()).unwrap_or_else(|| args.trim().to_string());
    let tc_expr = format!("{base_alloy}.^{field}");

    let after = &rest[paren + 1 + close + 1..];
    if after.is_empty() {
        return Some(tc_expr);
    }
    let full_after = format!("{tc_expr}{after}");
    reverse_translate_expr(&full_after).or(Some(tc_expr))
}

fn try_reverse_stream_method(s: &str, pattern: &str, quant: &str) -> Option<String> {
    let pos = s.find(pattern)?;
    let collection = s[..pos].trim();
    let rest = &s[pos + pattern.len()..];
    let (var, body) = extract_java_lambda(rest)?;
    let body_alloy = reverse_translate_expr(body).unwrap_or_else(|| body.to_string());
    Some(format!("{quant} {var}: {collection} | {body_alloy}"))
}

/// Extract `v -> body)` from Java lambda in a stream call.
fn extract_java_lambda(s: &str) -> Option<(&str, &str)> {
    let arrow = s.find(" -> ")?;
    let var = s[..arrow].trim();
    let body_start = arrow + 4;
    let body_rest = &s[body_start..];
    // Find the matching close paren of the stream method call
    let mut depth = 0i32;
    let mut end = body_rest.len();
    for (i, ch) in body_rest.chars().enumerate() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                depth -= 1;
                if depth < 0 { end = i; break; }
            }
            _ => {}
        }
    }
    let body = body_rest[..end].trim();
    if var.is_empty() || body.is_empty() { return None; }
    Some((var, body))
}

fn try_reverse_java_contains(s: &str) -> Option<String> {
    let pos = s.find(".contains(")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".contains(".len()..];
    let end = find_matching_close_java(rest)?;
    let element = rest[..end].trim();
    let element_alloy = reverse_translate_expr(element).unwrap_or_else(|| element.to_string());
    let collection_alloy = reverse_translate_expr(collection).unwrap_or_else(|| collection.to_string());
    Some(format!("{element_alloy} in {collection_alloy}"))
}

fn try_reverse_java_size(s: &str) -> Option<String> {
    let pos = s.rfind(".size()")?;
    let inner = s[..pos].trim();
    let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
    Some(format!("#{inner_alloy}"))
}

fn try_reverse_comparison(s: &str) -> Option<String> {
    for (java_op, alloy_op) in &[(" == ", " = "), (" != ", " != "), (" <= ", " <= "),
                                   (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = find_top_level(s, java_op) {
            let left = s[..pos].trim();
            let right = s[pos + java_op.len()..].trim();
            let l = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
            let r = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
            return Some(format!("{l}{alloy_op}{r}"));
        }
    }
    None
}

fn try_reverse_logic(s: &str) -> Option<String> {
    if s.starts_with('!') {
        let inner = &s[1..];
        if let Some(pos) = find_top_level(inner, " || ") {
            let a = strip_balanced_parens(inner[..pos].trim());
            let b = inner[pos + 4..].trim();
            let a_alloy = reverse_translate_expr(a).unwrap_or_else(|| a.to_string());
            let b_alloy = reverse_translate_expr(b).unwrap_or_else(|| b.to_string());
            return Some(format!("{a_alloy} implies {b_alloy}"));
        }
    }
    if let Some(pos) = find_top_level(s, " && ") {
        let left = s[..pos].trim();
        let right = s[pos + 4..].trim();
        let l = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
        let r = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
        return Some(format!("{l} and {r}"));
    }
    if let Some(pos) = find_top_level(s, " || ") {
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

fn find_top_level(s: &str, pattern: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let pat_bytes = pattern.as_bytes();
    if pat_bytes.len() > bytes.len() { return None; }
    for i in 0..=bytes.len() - pat_bytes.len() {
        match bytes[i] {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            _ => {}
        }
        if depth == 0 && &bytes[i..i + pat_bytes.len()] == pat_bytes {
            return Some(i);
        }
    }
    None
}

fn find_matching_close_java(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in s.chars().enumerate() {
        match ch {
            '(' | '{' => depth += 1,
            ')' | '}' => {
                if depth == 0 { return Some(i); }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Convert camelCase plural to PascalCase singular.
fn camel_to_pascal(s: &str) -> String {
    let s = s.strip_suffix('s').unwrap_or(s);
    let mut result = String::new();
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        result.push(first.to_ascii_uppercase());
    }
    result.extend(chars);
    result
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
