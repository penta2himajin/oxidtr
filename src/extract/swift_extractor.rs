/// Extracts Alloy model candidates from Swift source code.
/// Handles: struct → sig, enum → abstract sig,
/// T? → lone, Set<T> → set, [T] → seq, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        // struct Foo: ... { → sig
        if let Some(name) = parse_struct(trimmed) {
            let fields = collect_struct_fields(&mut lines);
            sigs.push(MinedSig {
                name,
                fields,
                is_abstract: false,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
        }

        // enum Foo: ... { → abstract sig
        if let Some(name) = parse_enum(trimmed) {
            let (variants, variant_sigs) = collect_enum_cases(&name, &mut lines);
            sigs.push(MinedSig {
                name: name.clone(),
                fields: vec![],
                is_abstract: true,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
            for vs in variant_sigs {
                sigs.push(vs);
            }
            let _ = variants;
        }

        // Function bodies: extract general patterns
        if trimmed.starts_with("func ") {
            let body = collect_block(&mut lines);
            extract_swift_facts(&body, line_num, &mut fact_candidates);
        }
    }

    // Extract @temporal annotations from generated tests
    super::extract_temporal_annotations(source, &mut fact_candidates);

    MinedModel { sigs, fact_candidates }
}

fn parse_struct(line: &str) -> Option<String> {
    let rest = line.strip_prefix("struct ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { return None; }
    // Must have { somewhere (not a forward declaration)
    if !line.contains('{') { return None; }
    Some(name)
}

fn parse_enum(line: &str) -> Option<String> {
    // "enum Foo" or "enum Foo: ..." or "enum Foo {"
    let rest = line.strip_prefix("enum ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { return None; }
    Some(name)
}

fn collect_struct_fields(
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

        // "let name: Type" or "var name: Type"
        if let Some(field) = parse_swift_property(trimmed) {
            fields.push(field);
        }
    }

    fields
}

fn parse_swift_property(line: &str) -> Option<MinedField> {
    let is_var = line.starts_with("var ");
    let rest = line.strip_prefix("let ")
        .or_else(|| line.strip_prefix("var "))?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim().to_string();
    if name.is_empty() || name.contains(' ') { return None; }

    let type_part = rest[colon + 1..].trim();
    // Remove default value assignment
    let type_str = if let Some(eq_pos) = type_part.find('=') {
        type_part[..eq_pos].trim()
    } else {
        type_part
    };
    // Remove trailing comments
    let type_str = if let Some(comment_pos) = type_str.find("//") {
        type_str[..comment_pos].trim()
    } else {
        type_str
    };

    let (mult, target) = swift_type_to_mult(type_str);
    Some(MinedField { name, is_var, mult, target, raw_union_type: None })
}

fn swift_type_to_mult(swift_type: &str) -> (MinedMultiplicity, String) {
    let t = swift_type.trim();

    // T? → lone
    if let Some(inner) = t.strip_suffix('?') {
        return (MinedMultiplicity::Lone, inner.to_string());
    }

    // Set<T> → set
    if let Some(inner) = strip_wrapper(t, "Set<", ">") {
        return (MinedMultiplicity::Set, inner.to_string());
    }

    // [K: V] → set (dictionary → map, key as target)
    if t.starts_with('[') && t.ends_with(']') && t.contains(':') {
        let inner = &t[1..t.len()-1];
        let key = inner.split(':').next().unwrap_or(inner).trim();
        return (MinedMultiplicity::Set, key.to_string());
    }

    // [T] → seq
    if t.starts_with('[') && t.ends_with(']') {
        let inner = &t[1..t.len()-1];
        return (MinedMultiplicity::Seq, inner.trim().to_string());
    }

    // Array<T> → seq
    if let Some(inner) = strip_wrapper(t, "Array<", ">") {
        return (MinedMultiplicity::Seq, inner.to_string());
    }

    (MinedMultiplicity::One, t.to_string())
}

fn strip_wrapper<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)?.strip_suffix(suffix)
}

fn collect_enum_cases(
    parent_name: &str,
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> (Vec<String>, Vec<MinedSig>) {
    let mut variant_names = Vec::new();
    let mut variant_sigs = Vec::new();
    let mut depth = 1usize;

    for (ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }

        // "case variantName" or "case variantName(params)"
        if let Some(rest) = trimmed.strip_prefix("case ") {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if name.is_empty() { continue; }
            // PascalCase the variant name for Alloy sig
            let pascal_name = capitalize(&name);

            let fields = if rest.contains('(') {
                extract_case_params(rest)
            } else {
                vec![]
            };

            variant_names.push(pascal_name.clone());
            variant_sigs.push(MinedSig {
                name: pascal_name,
                fields,
                is_abstract: false,
                parent: Some(parent_name.to_string()),
                source_location: format!("line {}", ln + 1),
                intersection_of: vec![],
            });
        }
    }

    (variant_names, variant_sigs)
}

fn extract_case_params(line: &str) -> Vec<MinedField> {
    let open = match line.find('(') { Some(p) => p + 1, None => return vec![] };
    let close = match line.rfind(')') { Some(p) => p, None => return vec![] };
    if open >= close { return vec![]; }

    let params = &line[open..close];
    params.split(',')
        .filter_map(|p| {
            let p = p.trim();
            // "name: Type" — may have label
            let colon = p.find(':')?;
            let name = p[..colon].trim().to_string();
            // Remove external label if present (e.g., "_ name: Type")
            let name = name.rsplit_once(' ').map_or(name.as_str(), |(_, n)| n).to_string();
            if name.is_empty() { return None; }
            let type_str = p[colon + 1..].trim();
            let (mult, target) = swift_type_to_mult(type_str);
            Some(MinedField { name, is_var: false, mult, target, raw_union_type: None })
        })
        .collect()
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

fn extract_swift_facts(
    body: &[(usize, String)],
    _context_line: usize,
    facts: &mut Vec<MinedFactCandidate>,
) {
    for (ln, line) in body {
        let loc = format!("line {}", ln + 1);

        // .contains() checks
        if line.contains(".contains(") {
            facts.push(MinedFactCandidate {
                alloy_text: extract_contains_fact(line),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".contains() check".to_string(),
            });
        }

        // .isEmpty / !xxx.isEmpty
        if line.contains(".isEmpty") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- emptiness check".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".isEmpty check".to_string(),
            });
        }

        // guard let / guard ... else — presence constraint
        if line.starts_with("guard let ") || line.starts_with("guard ") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- guard constraint".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "guard".to_string(),
            });
        }

        // precondition() / assert()
        if line.contains("precondition(") || line.contains("assert(") {
            let cond = extract_swift_validation_condition(line);
            facts.push(MinedFactCandidate {
                alloy_text: cond,
                confidence: Confidence::High,
                source_location: loc.clone(),
                source_pattern: "precondition/assert".to_string(),
            });
        }

        // if let — optional binding → lone
        if line.contains("if let ") {
            if let Some(field) = extract_if_let_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is lone (optional)"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "if let binding".to_string(),
                });
            }
        }

        // Force unwrap ! → one (non-null assertion)
        if line.contains("!.") || (line.contains('!') && !line.contains("!=")) {
            facts.push(MinedFactCandidate {
                alloy_text: "-- force unwrap (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "force unwrap".to_string(),
            });
        }

        // switch — exhaustiveness
        if line.contains("switch ") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- enum exhaustiveness (switch)".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "switch exhaustiveness".to_string(),
            });
        }

        // .filter / .map — subset/projection
        if line.contains(".filter(") || line.contains(".filter {") {
            if let Some(collection) = extract_collection_before(line, ".filter") {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- subset of {collection}"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "filter subset".to_string(),
                });
            }
        }

        // fatalError / preconditionFailure
        if (line.contains("fatalError(") || line.contains("preconditionFailure("))
            && !line.contains("guard") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "fatalError/preconditionFailure".to_string(),
            });
        }
    }
}

fn extract_swift_validation_condition(line: &str) -> String {
    let keyword = if line.contains("precondition(") { "precondition(" } else { "assert(" };
    if let Some(start) = line.find(keyword) {
        let rest = &line[start + keyword.len()..];
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
        // Remove message part after comma (e.g., precondition(cond, "msg"))
        let cond = if let Some(comma) = find_top_level_comma(cond) {
            cond[..comma].trim()
        } else {
            cond
        };
        if !cond.is_empty() {
            return translate_swift_condition(cond);
        }
    }
    "-- precondition".to_string()
}

fn translate_swift_condition(cond: &str) -> String {
    let mut result = cond.to_string();
    if result.contains(".count ") {
        if let Some(pos) = result.find(".count ") {
            let before = &result[..pos];
            let after = &result[pos + 6..];
            result = format!("#{before}{after}");
        }
    }
    if result.contains("!.isEmpty") {
        if let Some(pos) = result.find("!.isEmpty") {
            let before = &result[1..pos]; // skip leading !
            return format!("some {before}");
        }
    }
    if result.contains(".isEmpty") {
        if let Some(pos) = result.find(".isEmpty") {
            let field = &result[..pos];
            return format!("no {field}");
        }
    }
    result
}

fn extract_if_let_field(line: &str) -> Option<String> {
    let rest = line.strip_prefix("if let ")?;
    let eq = rest.find('=')?;
    let _binding = rest[..eq].trim();
    let rhs = rest[eq + 1..].trim();
    // Get the first identifier chain
    let field: String = rhs.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect();
    if field.is_empty() { None } else { Some(field) }
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

fn extract_collection_before(line: &str, method: &str) -> Option<String> {
    let pos = line.find(method)?;
    let before = line[..pos].trim();
    let collection = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{').next().unwrap_or(before).trim();
    if collection.is_empty() { None } else { Some(collection.to_string()) }
}

fn find_top_level_comma(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in s.chars().enumerate() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

/// Reverse-translate a Swift expression back to Alloy syntax.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    if s.is_empty() { return None; }

    let s = strip_balanced_parens(s);

    // tcField(base) → base.^field
    if let Some(result) = try_reverse_tc_call(s) {
        return Some(result);
    }

    // !xxx.contains { v in body } → no v: Xxx | body
    if s.starts_with('!') {
        let inner = &s[1..];
        if let Some(result) = try_reverse_contains_closure(inner, "no") {
            return Some(result);
        }
    }

    // .allSatisfy { v in body } → all v: Xxx | body
    if let Some(result) = try_reverse_all_satisfy(s) {
        return Some(result);
    }

    // .contains { v in body } → some v: Xxx | body
    if let Some(result) = try_reverse_contains_closure(s, "some") {
        return Some(result);
    }

    // .contains(v) → v in xxx
    if let Some(result) = try_reverse_contains(s) {
        return Some(result);
    }

    // .count → #xxx
    if let Some(result) = try_reverse_count(s) {
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
    let close = find_matching_close(&rest[paren + 1..])?;
    let args = &rest[paren + 1..paren + 1 + close];
    let base_alloy = reverse_translate_expr(args.trim()).unwrap_or_else(|| args.trim().to_string());
    Some(format!("{base_alloy}.^{field}"))
}

fn try_reverse_all_satisfy(s: &str) -> Option<String> {
    let pos = s.find(".allSatisfy { ")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".allSatisfy { ".len()..];
    let (var, body) = extract_swift_closure(rest)?;
    let body_alloy = reverse_translate_expr(body).unwrap_or_else(|| body.to_string());
    Some(format!("all {var}: {collection} | {body_alloy}"))
}

fn try_reverse_contains_closure(s: &str, quant: &str) -> Option<String> {
    let pos = s.find(".contains { ")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".contains { ".len()..];
    let (var, body) = extract_swift_closure(rest)?;
    let body_alloy = reverse_translate_expr(body).unwrap_or_else(|| body.to_string());
    Some(format!("{quant} {var}: {collection} | {body_alloy}"))
}

fn extract_swift_closure(s: &str) -> Option<(&str, &str)> {
    // "v in body }"
    let in_pos = s.find(" in ")?;
    let var = s[..in_pos].trim();
    let body_start = in_pos + 4;
    let body_rest = &s[body_start..];
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

fn try_reverse_contains(s: &str) -> Option<String> {
    let pos = s.find(".contains(")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".contains(".len()..];
    let end = find_matching_close(rest)?;
    let element = rest[..end].trim();
    let element_alloy = reverse_translate_expr(element).unwrap_or_else(|| element.to_string());
    let collection_alloy = reverse_translate_expr(collection).unwrap_or_else(|| collection.to_string());
    Some(format!("{element_alloy} in {collection_alloy}"))
}

fn try_reverse_count(s: &str) -> Option<String> {
    let pos = s.rfind(".count")?;
    let after = &s[pos + 6..];
    if !after.is_empty() && !after.starts_with(' ') { return None; }
    let inner = s[..pos].trim();
    let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
    Some(format!("#{inner_alloy}"))
}

fn try_reverse_comparison(s: &str) -> Option<String> {
    for (swift_op, alloy_op) in &[(" == ", " = "), (" != ", " != "), (" <= ", " <= "),
                                    (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = find_top_level_op(s, swift_op) {
            let left = s[..pos].trim();
            let right = s[pos + swift_op.len()..].trim();
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
        if let Some(pos) = find_top_level_op(inner, " || ") {
            let a = strip_balanced_parens(inner[..pos].trim());
            let b = inner[pos + 4..].trim();
            let a_alloy = reverse_translate_expr(a).unwrap_or_else(|| a.to_string());
            let b_alloy = reverse_translate_expr(b).unwrap_or_else(|| b.to_string());
            return Some(format!("{a_alloy} implies {b_alloy}"));
        }
    }
    if let Some(pos) = find_top_level_op(s, " && ") {
        let left = s[..pos].trim();
        let right = s[pos + 4..].trim();
        let l = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
        let r = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
        return Some(format!("{l} and {r}"));
    }
    if let Some(pos) = find_top_level_op(s, " || ") {
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

fn find_top_level_op(s: &str, pattern: &str) -> Option<usize> {
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

fn find_matching_close(s: &str) -> Option<usize> {
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
