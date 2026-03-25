/// Extracts Alloy model candidates from Kotlin source code.
/// Handles: data class → sig, sealed class/enum class → abstract sig,
/// T? → lone, List<T> → set, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();
    let mut prev_line_has_var_sig = false;

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        // Detect @alloy: var sig annotation for the next declaration
        if trimmed.contains("@alloy: var sig") {
            prev_line_has_var_sig = true;
            continue;
        }

        let sig_is_var = prev_line_has_var_sig;
        prev_line_has_var_sig = false;

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
                is_var: sig_is_var,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
            // Check for extends (": Parent()")
            if let Some((_child, parent)) = parse_extends(&full_text) {
                sigs.last_mut().unwrap().parent = Some(parent);
            }
        }

        // object Foo → singleton sig (one sig)
        if let Some(name) = parse_object_decl(trimmed) {
            sigs.push(MinedSig {
                name,
                fields: vec![],
                is_abstract: false,
                is_var: sig_is_var,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
        }

        // data object Foo : Parent() → child sig (unit)
        if let Some((name, parent)) = parse_data_object(trimmed) {
            sigs.push(MinedSig {
                name,
                fields: vec![],
                is_abstract: false,
                is_var: sig_is_var,
                parent: Some(parent),
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
        }

        // sealed class Foo → abstract sig
        if let Some(name) = parse_sealed_class(trimmed) {
            sigs.push(MinedSig {
                name,
                fields: vec![],
                is_abstract: true,
                is_var: sig_is_var,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
        }

        // enum class Foo { A, B } → abstract sig + children
        if let Some(name) = parse_enum_class(trimmed) {
            let variants = collect_enum_entries(&mut lines);
            sigs.push(MinedSig {
                name: name.clone(),
                fields: vec![],
                is_abstract: true,
                is_var: sig_is_var,
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
        }

        // data class Foo(...) : Parent() → set parent on existing sig
        if let Some((child, parent)) = parse_extends(trimmed) {
            if let Some(s) = sigs.iter_mut().find(|s| s.name == child) {
                s.parent = Some(parent);
            }
        }

        // Function bodies: extract general patterns + require() constraints
        if trimmed.starts_with("fun ") {
            let body = collect_block(&mut lines);
            extract_kt_facts(&body, line_num, &mut fact_candidates);
        }
    }

    // Extract @temporal annotations from generated tests
    super::extract_temporal_annotations(source, &mut fact_candidates);

    MinedModel { sigs, fact_candidates }
}

fn collect_until_close_paren(
    first_line: &str,
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> String {
    let mut full = first_line.to_string();
    // Track paren depth to handle nested parens (e.g., @Size(min = 0, max = 0))
    let mut depth: i32 = full.chars().filter(|&c| c == '(').count() as i32
        - full.chars().filter(|&c| c == ')').count() as i32;
    if depth <= 0 { return full; }
    for (_ln, line) in lines.by_ref() {
        full.push_str(" ");
        full.push_str(line.trim());
        depth += line.chars().filter(|&c| c == '(').count() as i32;
        depth -= line.chars().filter(|&c| c == ')').count() as i32;
        if depth <= 0 { break; }
    }
    full
}

fn parse_object_decl(line: &str) -> Option<String> {
    // Match "object Foo" but not "data object Foo" (handled separately)
    if line.starts_with("data object ") { return None; }
    let rest = line.strip_prefix("object ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_data_class(line: &str) -> Option<String> {
    let rest = line.strip_prefix("data class ")
        .or_else(|| line.strip_prefix("value class "))?;
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
    // Find the matching close paren (not rfind which could overshoot into `: Parent()`)
    let close = {
        let mut depth = 1;
        let mut pos = None;
        for (i, ch) in line[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 { pos = Some(open + i); break; }
                }
                _ => {}
            }
        }
        match pos { Some(p) => p, None => return vec![] }
    };
    if open >= close { return vec![]; }

    let params = &line[open..close];
    // Split on commas, but not commas inside parentheses (e.g., @Size(min = 0, max = 0))
    split_top_level_commas(params)
        .iter()
        .filter_map(|p| parse_kt_param(p.trim()))
        .collect()
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

fn parse_kt_param(param: &str) -> Option<MinedField> {
    // Strip block comments (e.g., /* @Size see fact: ... */) before parsing
    let cleaned = strip_block_comments(param);
    // Strip @Annotation(...) patterns (e.g., @Size(min = 0, max = 0))
    let cleaned = strip_annotations(&cleaned);
    let param = cleaned.trim();
    // "val name: Type" or "var name: Type?"
    let is_var = param.starts_with("var ");
    let rest = param.strip_prefix("val ").or_else(|| param.strip_prefix("var "))?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim().to_string();
    if name.is_empty() { return None; }
    let type_str = rest[colon + 1..].trim();
    let (mult, target) = kt_type_to_mult(type_str);
    Some(MinedField { name, is_var, mult, target, raw_union_type: None })
}

fn strip_annotations(s: &str) -> String {
    let mut result = s.to_string();
    // Remove @Annotation(...) patterns — handles nested parens
    while let Some(at_pos) = result.find('@') {
        // Find the annotation name
        let rest = &result[at_pos + 1..];
        let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
        if name_end == 0 { break; }
        let after_name = &rest[name_end..];
        if after_name.starts_with('(') {
            // Find matching close paren
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
            } else {
                break;
            }
        } else {
            // Annotation without parens: just @Name
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

fn kt_type_to_mult(kt_type: &str) -> (MinedMultiplicity, String) {
    let t = kt_type.trim();
    // T? → lone
    if let Some(inner) = t.strip_suffix('?') {
        return (MinedMultiplicity::Lone, inner.to_string());
    }
    // Map<K, V> → set of K (V info lost)
    if let Some(inner) = strip_wrapper(t, "Map<", ">") {
        let key = inner.split(',').next().unwrap_or(inner).trim();
        return (MinedMultiplicity::Set, key.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "MutableMap<", ">") {
        let key = inner.split(',').next().unwrap_or(inner).trim();
        return (MinedMultiplicity::Set, key.to_string());
    }
    // Set<T> → set
    if let Some(inner) = strip_wrapper(t, "Set<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "MutableSet<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }
    // List<T> → seq
    if let Some(inner) = strip_wrapper(t, "List<", ">") {
        return (MinedMultiplicity::Seq, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "MutableList<", ">") {
        return (MinedMultiplicity::Seq, inner.to_string());
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
    // The opening '{' is on the trigger line (already consumed by the caller's loop),
    // so we start at depth=1.
    let mut depth = 1usize;
    for (ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => { if depth > 0 { depth -= 1; } } _ => {} }
        }
        if depth == 0 { break; }
        body.push((ln, trimmed.to_string()));
    }
    body
}

/// Reverse-translate a Kotlin expression back to Alloy syntax.
/// Robust: handles balanced parens/braces, TC calls, lambda syntax.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    if s.is_empty() { return None; }

    let s = strip_balanced_parens(s);

    // tcField(base) → base.^field
    if let Some(result) = try_reverse_tc_call(s) {
        return Some(result);
    }

    // !xxx.any { v -> body } → no v: Xxx | body
    if s.starts_with('!') {
        let inner = &s[1..];
        if let Some(result) = try_reverse_kt_any(inner, "no") {
            return Some(result);
        }
    }

    // .all { v -> body } → all v: Xxx | body
    if let Some(result) = try_reverse_kt_all(s) {
        return Some(result);
    }

    // .any { v -> body } → some v: Xxx | body
    if let Some(result) = try_reverse_kt_any(s, "some") {
        return Some(result);
    }

    // .contains(v) → v in xxx
    if let Some(result) = try_reverse_kt_contains(s) {
        return Some(result);
    }

    // .size → #xxx
    if let Some(result) = try_reverse_kt_size(s) {
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

    // Variable references and field access chains
    if s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !s.is_empty() {
        return Some(s.to_string());
    }

    None
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
    let close = find_matching_close_kt(&rest[paren + 1..])?;
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

fn try_reverse_kt_all(s: &str) -> Option<String> {
    let pos = s.find(".all { ")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".all { ".len()..];
    let (var, body) = extract_kt_lambda(rest)?;
    let body_alloy = reverse_translate_expr(body).unwrap_or_else(|| body.to_string());
    Some(format!("all {var}: {collection} | {body_alloy}"))
}

fn try_reverse_kt_any(s: &str, quant: &str) -> Option<String> {
    let pos = s.find(".any { ")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".any { ".len()..];
    let (var, body) = extract_kt_lambda(rest)?;
    let body_alloy = reverse_translate_expr(body).unwrap_or_else(|| body.to_string());
    Some(format!("{quant} {var}: {collection} | {body_alloy}"))
}

/// Extract `v -> body }` from Kotlin lambda.
fn extract_kt_lambda(s: &str) -> Option<(&str, &str)> {
    let arrow = s.find(" -> ")?;
    let var = s[..arrow].trim();
    let body_start = arrow + 4;
    let body_rest = &s[body_start..];
    // Find the matching close brace
    let mut depth = 0i32;
    let mut end = body_rest.len();
    for (i, ch) in body_rest.chars().enumerate() {
        match ch {
            '{' | '(' => depth += 1,
            '}' | ')' => {
                if ch == '}' && depth == 0 { end = i; break; }
                depth -= 1;
            }
            _ => {}
        }
    }
    let body = body_rest[..end].trim();
    if var.is_empty() || body.is_empty() { return None; }
    Some((var, body))
}

fn try_reverse_kt_contains(s: &str) -> Option<String> {
    let pos = s.find(".contains(")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".contains(".len()..];
    let end = find_matching_close_kt(rest)?;
    let element = rest[..end].trim();
    let element_alloy = reverse_translate_expr(element).unwrap_or_else(|| element.to_string());
    let collection_alloy = reverse_translate_expr(collection).unwrap_or_else(|| collection.to_string());
    Some(format!("{element_alloy} in {collection_alloy}"))
}

fn try_reverse_kt_size(s: &str) -> Option<String> {
    let pos = s.rfind(".size")?;
    let after = &s[pos + 5..];
    if !after.is_empty() && !after.starts_with(' ') { return None; }
    let inner = s[..pos].trim();
    let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
    Some(format!("#{inner_alloy}"))
}

fn try_reverse_comparison(s: &str) -> Option<String> {
    for (kt_op, alloy_op) in &[(" == ", " = "), (" != ", " != "), (" <= ", " <= "),
                                 (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = find_top_level(s, kt_op) {
            let left = s[..pos].trim();
            let right = s[pos + kt_op.len()..].trim();
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

fn find_matching_close_kt(s: &str) -> Option<usize> {
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

        // Category 1: require(condition) / check(condition) → extract condition
        if line.contains("require(") || line.contains("check(") {
            let cond = extract_kt_validation_condition(line);
            facts.push(MinedFactCandidate {
                alloy_text: cond,
                confidence: Confidence::High,
                source_location: loc.clone(),
                source_pattern: "require/check".to_string(),
            });
        }

        // Category 2: elvis throw — ?: throw
        if line.contains("?: throw") {
            if let Some(field) = extract_elvis_throw_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("some {field} -- presence constraint"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "elvis throw".to_string(),
                });
            }
        }

        // Category 2: elvis return — ?: return
        if line.contains("?: return") {
            if let Some(field) = extract_elvis_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is lone (optional)"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "elvis return".to_string(),
                });
            }
        }

        // Category 3: when exhaustiveness
        if line.contains("when (") || line.contains("when(") {
            let variants = collect_when_variants(body, *ln);
            if !variants.is_empty() {
                let variants_str = variants.join(", ");
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- enum variants: {variants_str}"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "when exhaustiveness".to_string(),
                });
            }
        }

        // Category 4: safe call ?.
        if line.contains("?.") && !line.contains("?:") {
            if let Some(field) = extract_kt_safe_call_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is lone (nullable)"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "safe call".to_string(),
                });
            }
        }

        // Category 4: non-null assertion !!
        if line.contains("!!") {
            if let Some(field) = extract_kt_double_bang_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is one (non-null assertion)"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "non-null assertion".to_string(),
                });
            }
        }

        // Category 5: .filter { } — subset
        if line.contains(".filter {") || line.contains(".filter(") {
            if let Some(collection) = extract_kt_collection_before(line, ".filter") {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- subset of {collection}"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "filter subset".to_string(),
                });
            }
        }

        // Category 5: .map { } — field projection
        if (line.contains(".map {") || line.contains(".map(")) && !line.contains(".filter") {
            if let Some(collection) = extract_kt_collection_before(line, ".map") {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- field projection from {collection}"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "map projection".to_string(),
                });
            }
        }

        if (line.contains("throw ") || line.contains("TODO("))
            && !line.contains("?: throw") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "throw/TODO pattern".to_string(),
            });
        }
    }
}

/// Extract condition from require(cond) or check(cond), translating to Alloy-like syntax.
fn extract_kt_validation_condition(line: &str) -> String {
    let keyword = if line.contains("require(") { "require(" } else { "check(" };
    if let Some(start) = line.find(keyword) {
        let rest = &line[start + keyword.len()..];
        // Find matching close paren
        let mut depth = 0;
        let mut end = rest.len();
        for (i, ch) in rest.chars().enumerate() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 { end = i; break; }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let cond = rest[..end].trim();
        if !cond.is_empty() {
            return translate_kt_condition(cond);
        }
    }
    "-- precondition (require/check)".to_string()
}

/// Translate Kotlin condition to Alloy-like syntax.
fn translate_kt_condition(cond: &str) -> String {
    let mut result = cond.to_string();
    // .size → #
    if result.contains(".size ") {
        if let Some(pos) = result.find(".size ") {
            let before = &result[..pos];
            let after = &result[pos + 5..];
            result = format!("#{before}{after}");
        }
    }
    // .isNotEmpty() → some XXX
    if result.contains(".isNotEmpty()") {
        if let Some(pos) = result.find(".isNotEmpty()") {
            let field = &result[..pos];
            return format!("some {field}");
        }
    }
    // .isEmpty() → no XXX
    if result.contains(".isEmpty()") {
        if let Some(pos) = result.find(".isEmpty()") {
            let field = &result[..pos];
            return format!("no {field}");
        }
    }
    result
}

/// Extract field from `val x = FIELD ?: throw ...`
fn extract_elvis_throw_field(line: &str) -> Option<String> {
    let pos = line.find("?: throw")?;
    let before = line[..pos].trim();
    // Look for the value after = sign
    if let Some(eq) = before.rfind('=') {
        let field = before[eq + 1..].trim();
        if !field.is_empty() { return Some(field.to_string()); }
    }
    let field = before.rsplit(|c: char| c == ' ' || c == '(').next().unwrap_or(before).trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Extract field from elvis expression like `field?.xxx ?: return`
fn extract_elvis_field(line: &str) -> Option<String> {
    let pos = line.find("?: return")?;
    let before = line[..pos].trim();
    // Look for ?.
    if let Some(safe_pos) = before.find("?.") {
        // Get the field before ?.
        let field_part = &before[..safe_pos];
        if let Some(eq) = field_part.rfind('=') {
            let field = field_part[eq + 1..].trim();
            if !field.is_empty() { return Some(field.to_string()); }
        }
        let field = field_part.rsplit(|c: char| c == ' ' || c == '(').next().unwrap_or(field_part).trim();
        if !field.is_empty() { return Some(field.to_string()); }
    }
    None
}

/// Collect variant names from when arms.
fn collect_when_variants(body: &[(usize, String)], when_ln: usize) -> Vec<String> {
    let mut variants = Vec::new();
    let mut in_when = false;
    for (ln, line) in body {
        if *ln == when_ln { in_when = true; continue; }
        if !in_when { continue; }
        let trimmed = line.trim();
        if trimmed == "}" { break; }
        // "is VariantName ->" or "VariantName ->"
        if trimmed.contains("->") {
            let arm = trimmed.split("->").next().unwrap_or("").trim();
            let arm = arm.strip_prefix("is ").unwrap_or(arm).trim();
            if !arm.is_empty() && arm.chars().next().map_or(false, |c| c.is_ascii_uppercase())
                && arm.chars().all(|c| c.is_alphanumeric() || c == '_') {
                variants.push(arm.to_string());
            }
        }
    }
    variants
}

/// Extract field from safe call like `field?.method`
fn extract_kt_safe_call_field(line: &str) -> Option<String> {
    let pos = line.find("?.")?;
    let before = line[..pos].trim();
    let field = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{' || c == ',').next().unwrap_or(before).trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Extract field from `field!!.method`
fn extract_kt_double_bang_field(line: &str) -> Option<String> {
    let pos = line.find("!!")?;
    let before = line[..pos].trim();
    let field = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{' || c == ',').next().unwrap_or(before).trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Extract collection before a method call
fn extract_kt_collection_before(line: &str, method: &str) -> Option<String> {
    let pos = line.find(method)?;
    let before = line[..pos].trim();
    let collection = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{').next().unwrap_or(before).trim();
    if collection.is_empty() { None } else { Some(collection.to_string()) }
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
