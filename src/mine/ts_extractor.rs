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

        // Invariant functions: export function assertXxx(...): boolean { return BODY; }
        if (trimmed.starts_with("export function ") || trimmed.starts_with("function "))
            && trimmed.contains(": boolean")
        {
            let fn_name = extract_ts_fn_name(trimmed);
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
            extract_ts_facts(&body, line_num, &mut fact_candidates);
        } else if trimmed.starts_with("export function ") || trimmed.starts_with("function ") {
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

/// Extract function name from a TS function declaration line.
fn extract_ts_fn_name(line: &str) -> Option<String> {
    let rest = line.trim()
        .strip_prefix("export function ")
        .or_else(|| line.trim().strip_prefix("function "))?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Reverse-translate the body of a TS invariant function.
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

/// Reverse-translate a TypeScript expression back to Alloy syntax.
/// Robust: handles balanced parens, TC calls, camelCase → snake_case field conversion.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    if s.is_empty() { return None; }

    // Strip outer parens
    let s = strip_balanced_parens(s);

    // tcField(base) → base.^field
    if let Some(result) = try_reverse_tc_call(s) {
        return Some(result);
    }

    // !xxx.some(...) → no quantifier (must come before bare .some)
    if s.starts_with('!') {
        let inner = &s[1..];
        if let Some(result) = try_reverse_ts_some(inner, "no") {
            return Some(result);
        }
    }

    // .every((v) => body) → all v: Xxx | body
    if let Some(result) = try_reverse_ts_every(s) {
        return Some(result);
    }

    // .some((v) => body) → some v: Xxx | body
    if let Some(result) = try_reverse_ts_some(s, "some") {
        return Some(result);
    }

    // .includes(v) → v in xxx
    if let Some(result) = try_reverse_includes(s) {
        return Some(result);
    }

    // .has(v) → v in xxx
    if let Some(result) = try_reverse_has(s) {
        return Some(result);
    }

    // .length → #xxx
    if let Some(result) = try_reverse_length(s) {
        return Some(result);
    }

    // .size → #xxx
    if let Some(result) = try_reverse_size(s) {
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
    // Pattern: tcFieldName(base) → base.^fieldName
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
    // Find matching close paren
    let close = find_matching_close_ts(&rest[paren + 1..])?;
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

fn try_reverse_ts_every(s: &str) -> Option<String> {
    let pos = s.find(".every(")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".every(".len()..];
    let (var, body) = extract_arrow_fn(rest)?;
    let body_alloy = reverse_translate_expr(body.trim()).unwrap_or_else(|| body.trim().to_string());
    Some(format!("all {var}: {collection} | {body_alloy}"))
}

fn try_reverse_ts_some(s: &str, quant: &str) -> Option<String> {
    let pos = s.find(".some(")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".some(".len()..];
    let (var, body) = extract_arrow_fn(rest)?;
    let body_alloy = reverse_translate_expr(body.trim()).unwrap_or_else(|| body.trim().to_string());
    Some(format!("{quant} {var}: {collection} | {body_alloy}"))
}

/// Extract `(var) => body)` or `var => body)` from arrow function in a method call.
/// Returns (var_name, body_str).
fn extract_arrow_fn(s: &str) -> Option<(&str, &str)> {
    let arrow = s.find("=>")?;
    let var_part = s[..arrow].trim();
    // Strip parens from (v)
    let var = var_part.trim_start_matches('(').trim_end_matches(')').trim();
    let body_start = arrow + 2;
    // The body is everything after => up to the matching close paren of the method call
    let body_rest = &s[body_start..];
    // Find the point where parens become unbalanced (the closing paren of .every/.some)
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

fn try_reverse_includes(s: &str) -> Option<String> {
    let pos = s.find(".includes(")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".includes(".len()..];
    let end = find_matching_close_ts(rest)?;
    let element = rest[..end].trim();
    let element_alloy = reverse_translate_expr(element).unwrap_or_else(|| element.to_string());
    let collection_alloy = reverse_translate_expr(collection).unwrap_or_else(|| collection.to_string());
    Some(format!("{element_alloy} in {collection_alloy}"))
}

fn try_reverse_has(s: &str) -> Option<String> {
    let pos = s.find(".has(")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".has(".len()..];
    let end = find_matching_close_ts(rest)?;
    let element = rest[..end].trim();
    let element_alloy = reverse_translate_expr(element).unwrap_or_else(|| element.to_string());
    let collection_alloy = reverse_translate_expr(collection).unwrap_or_else(|| collection.to_string());
    Some(format!("{element_alloy} in {collection_alloy}"))
}

fn try_reverse_length(s: &str) -> Option<String> {
    let pos = s.rfind(".length")?;
    // Make sure .length is at end or followed by space/operator
    let after = &s[pos + 7..];
    if !after.is_empty() && !after.starts_with(' ') { return None; }
    let inner = s[..pos].trim();
    let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
    Some(format!("#{inner_alloy}"))
}

fn try_reverse_size(s: &str) -> Option<String> {
    let pos = s.rfind(".size")?;
    let after = &s[pos + 5..];
    if !after.is_empty() && !after.starts_with(' ') { return None; }
    let inner = s[..pos].trim();
    let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
    Some(format!("#{inner_alloy}"))
}

fn try_reverse_comparison(s: &str) -> Option<String> {
    for (ts_op, alloy_op) in &[(" === ", " = "), (" !== ", " != "), (" <= ", " <= "),
                                 (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = find_top_level(s, ts_op) {
            let left = s[..pos].trim();
            let right = s[pos + ts_op.len()..].trim();
            let left_alloy = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
            let right_alloy = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
            return Some(format!("{left_alloy}{alloy_op}{right_alloy}"));
        }
    }
    None
}

fn try_reverse_logic(s: &str) -> Option<String> {
    // !a || b → a implies b
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

fn find_matching_close_ts(s: &str) -> Option<usize> {
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

/// Convert camelCase plural to PascalCase singular (e.g., "sigDecls" → "SigDecl").
fn camel_to_pascal(s: &str) -> String {
    // Strip trailing 's' for plural
    let s = s.strip_suffix('s').unwrap_or(s);
    let mut result = String::new();
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        result.push(first.to_ascii_uppercase());
    }
    result.extend(chars);
    result
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
