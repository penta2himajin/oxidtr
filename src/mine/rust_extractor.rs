/// Extracts Alloy model candidates from Rust source code.
/// Uses lightweight line-based parsing (same approach as check/impl_parser).

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    // First pass: extract top-level @alloy comments
    extract_alloy_comments(
        source.lines().enumerate().map(|(ln, line)| (ln, line.to_string())),
        &mut fact_candidates,
    );

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

        // Invariant functions: pub fn assert_xxx(...) -> bool { BODY }
        if (trimmed.starts_with("pub fn ") || trimmed.starts_with("fn "))
            && trimmed.contains("-> bool")
        {
            let fn_name = extract_rust_fn_name(trimmed);
            let body = collect_fn_body(&mut lines);
            // Try commentless reverse translation first
            if let Some(ref name) = fn_name {
                if name.starts_with("assert_") {
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
            extract_facts_from_lines(&body, line_num, &mut fact_candidates);
        } else if trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") {
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
    // BTreeMap<K, V> or HashMap<K, V> → treat as Set of K (with V info lost for now)
    if let Some(inner) = strip_wrapper(t, "BTreeMap<", ">") {
        // Extract key type (first element before comma)
        let key = inner.split(',').next().unwrap_or(inner).trim();
        return (MinedMultiplicity::Set, key.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "HashMap<", ">") {
        let key = inner.split(',').next().unwrap_or(inner).trim();
        return (MinedMultiplicity::Set, key.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "Vec<", ">") {
        return (MinedMultiplicity::Seq, inner.to_string());
    }
    if let Some(inner) = strip_wrapper(t, "BTreeSet<", ">") {
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

/// Extract @alloy: comments from lines, returning high-confidence fact candidates.
fn extract_alloy_comments(
    lines: impl Iterator<Item = (usize, String)>,
    facts: &mut Vec<MinedFactCandidate>,
) {
    for (ln, line) in lines {
        let trimmed = line.trim();
        // Match "/// @alloy: ..." or "// @alloy: ..."
        let alloy_text = trimmed.strip_prefix("/// @alloy: ")
            .or_else(|| trimmed.strip_prefix("// @alloy: "));
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

/// Extract function name from a Rust fn declaration line.
fn extract_rust_fn_name(line: &str) -> Option<String> {
    let rest = line.trim()
        .strip_prefix("pub fn ")
        .or_else(|| line.trim().strip_prefix("fn "))?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Reverse-translate the body of a Rust invariant function.
/// Handles `{ let v = v.clone(); BODY }` wrapping and `return EXPR` patterns.
/// Also converts snake_case plural param names back to PascalCase sig names.
fn reverse_translate_invariant_body(body_text: &str) -> Option<String> {
    let s = body_text.trim();
    // Extract the return expression: may be just the expression or `return EXPR`
    let expr = if let Some(pos) = s.find("return ") {
        s[pos + "return ".len()..].trim().trim_end_matches(';').trim()
    } else {
        s.trim_end_matches(';').trim()
    };
    if expr.is_empty() {
        return None;
    }
    let alloy = reverse_translate_expr(expr)?;
    // Post-process: convert quantifier domain names from snake_plural to PascalCase
    Some(fix_quantifier_domains_rust(&alloy))
}

/// Fix quantifier domains: "all s: sig_decls | ..." → "all s: SigDecl | ..."
fn fix_quantifier_domains_rust(alloy: &str) -> String {
    // Pattern: "all/some/no VAR: DOMAIN | BODY"
    let mut result = alloy.to_string();
    for quant in &["all ", "some ", "no "] {
        while let Some(qpos) = result.find(quant) {
            let after_quant = &result[qpos + quant.len()..];
            if let Some(colon) = after_quant.find(": ") {
                let domain_start = qpos + quant.len() + colon + 2;
                let after_domain = &result[domain_start..];
                if let Some(pipe) = after_domain.find(" | ") {
                    let domain = &result[domain_start..domain_start + pipe];
                    let converted = snake_to_pascal(domain);
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
            // Prevent infinite loop: move past this occurrence
            break;
        }
    }
    result
}

/// Reverse-translate a Rust expression back to Alloy syntax.
/// Only handles patterns we know our own generator produces.
/// Robust: handles balanced parens, clone blocks, TC calls.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    if s.is_empty() {
        return None;
    }

    // Strip outer parens if balanced: "(expr)" → "expr"
    let s = strip_balanced_parens(s);

    // Strip Rust clone blocks: { let v = v.clone(); body } → body
    let s = strip_clone_block(s);

    // tc_field(&v) → v.^field
    if let Some(result) = try_reverse_tc_call(s) {
        return Some(result);
    }

    // Quantifiers (must come before contains/logic to handle nested cases)
    if let Some(result) = try_reverse_quantifier(s) {
        return Some(result);
    }

    // (*base.field) → strip deref for Box<T> fields
    if let Some(inner) = s.strip_prefix("(*").and_then(|s| s.strip_suffix(')')) {
        return reverse_translate_expr(inner);
    }

    // .contains(&v) → v in xxx (must come before .len check)
    if let Some(result) = try_reverse_contains(s) {
        return Some(result);
    }

    // .len() → #xxx
    if let Some(result) = try_reverse_cardinality(s) {
        return Some(result);
    }

    // Comparison operators (balanced — find at top level)
    if let Some(result) = try_reverse_comparison(s) {
        return Some(result);
    }

    // Boolean logic (balanced — find at top level)
    if let Some(result) = try_reverse_logic(s) {
        return Some(result);
    }

    // Integer literals
    if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        return Some(s.to_string());
    }

    // Variable references and field access chains: v.field → v.field
    if s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !s.is_empty() {
        return Some(s.to_string());
    }

    None
}

/// Strip balanced outer parentheses.
fn strip_balanced_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    // Check that the parens are actually balanced at the outermost level
    let inner = &s[1..s.len() - 1];
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 { return s; } // unbalanced
            }
            _ => {}
        }
    }
    if depth == 0 { inner.trim() } else { s }
}

/// Strip Rust clone block: `{ let v = v.clone(); BODY }` → `BODY`
fn strip_clone_block(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('{') || !s.ends_with('}') {
        return s;
    }
    let inner = s[1..s.len() - 1].trim();
    // Pattern: "let v = v.clone(); BODY"
    if let Some(semi) = inner.find(';') {
        let prefix = inner[..semi].trim();
        if prefix.starts_with("let ") && prefix.contains(".clone()") {
            return inner[semi + 1..].trim();
        }
    }
    s
}

/// Reverse tc_field(&v) → v.^field
fn try_reverse_tc_call(s: &str) -> Option<String> {
    // Pattern: tc_fieldname(&base) — only match if the tc call is the WHOLE expression
    // or directly followed by a method call like .contains(...)
    let rest = s.strip_prefix("tc_")?;
    let paren = rest.find('(')?;
    let field = &rest[..paren];
    if field.is_empty() { return None; }
    // Find the matching close paren
    let close = find_matching_close(&rest[paren + 1..], '(', ')')?;
    let args = &rest[paren + 1..paren + 1 + close];
    let base = args.trim().strip_prefix('&').unwrap_or(args.trim());
    let base_alloy = reverse_translate_expr(base).unwrap_or_else(|| base.to_string());
    let tc_expr = format!("{base_alloy}.^{field}");

    // Check if there's a method call after the tc call
    let after = &rest[paren + 1 + close + 1..];
    if after.is_empty() {
        return Some(tc_expr);
    }
    // If there's .contains(&x) or similar after, recursively translate the whole thing
    // by treating tc_result as a normal expression
    let full_after = format!("{tc_expr}{after}");
    reverse_translate_expr(&full_after).or(Some(tc_expr))
}

fn try_reverse_quantifier(s: &str) -> Option<String> {
    // !xxx.iter().any(|v| body) → no v: Xxx | body
    // Must check before bare `.iter().any` to catch the negation
    if s.starts_with('!') {
        let inner = &s[1..];
        if let Some(result) = try_reverse_iter_any(inner, "no") {
            return Some(result);
        }
    }
    // xxx.iter().all(|v| body) → all v: Xxx | body
    if let Some(result) = try_reverse_iter_method(s, ".iter().all(|", "all") {
        return Some(result);
    }
    // xxx.iter().any(|v| body) → some v: Xxx | body
    if let Some(result) = try_reverse_iter_any(s, "some") {
        return Some(result);
    }
    None
}

fn try_reverse_iter_method(s: &str, pattern: &str, quant: &str) -> Option<String> {
    let pos = s.find(pattern)?;
    let collection = s[..pos].trim();
    let rest = &s[pos + pattern.len()..];
    let pipe_pos = rest.find('|')?;
    let var = rest[..pipe_pos].trim();
    let body_start = pipe_pos + 1;
    let body = extract_balanced_body(&rest[body_start..]);
    let body_alloy = reverse_translate_expr(body.trim()).unwrap_or_else(|| body.trim().to_string());
    Some(format!("{quant} {var}: {collection} | {body_alloy}"))
}

fn try_reverse_iter_any(s: &str, quant: &str) -> Option<String> {
    let pattern = ".iter().any(|";
    let pos = s.find(pattern)?;
    let collection = s[..pos].trim();
    let rest = &s[pos + pattern.len()..];
    let pipe_pos = rest.find('|')?;
    let var = rest[..pipe_pos].trim();
    let body_start = pipe_pos + 1;
    let body = extract_balanced_body(&rest[body_start..]);
    let body_alloy = reverse_translate_expr(body.trim()).unwrap_or_else(|| body.trim().to_string());
    Some(format!("{quant} {var}: {collection} | {body_alloy}"))
}

/// Extract body from inside a closure, respecting balanced parens/braces.
/// Finds the point where depth goes negative (the closing paren of the enclosing call).
fn extract_balanced_body(s: &str) -> &str {
    let s = s.trim();
    let mut depth = 0i32;
    let mut end = s.len();
    for (i, ch) in s.chars().enumerate() {
        match ch {
            '(' | '{' => depth += 1,
            ')' | '}' => {
                depth -= 1;
                if depth < 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    s[..end].trim()
}

fn try_reverse_contains(s: &str) -> Option<String> {
    // xxx.contains(&v) → v in xxx
    let pos = s.find(".contains(&")?;
    let collection = s[..pos].trim();
    let rest = &s[pos + ".contains(&".len()..];
    // Find matching close paren
    let end = find_matching_close(rest, '(', ')')?;
    let element = rest[..end].trim();
    let element_alloy = reverse_translate_expr(element).unwrap_or_else(|| element.to_string());
    let collection_alloy = reverse_translate_expr(collection).unwrap_or_else(|| collection.to_string());
    Some(format!("{element_alloy} in {collection_alloy}"))
}

fn try_reverse_cardinality(s: &str) -> Option<String> {
    // xxx.len() → #xxx
    if !s.ends_with(".len()") && !s.contains(".len() ") {
        // Only match .len() at end or before an operator
        if let Some(pos) = s.rfind(".len()") {
            let after = &s[pos + 6..];
            if !after.is_empty() && !after.starts_with(' ') {
                return None;
            }
            let inner = s[..pos].trim();
            let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
            return Some(format!("#{inner_alloy}"));
        }
        return None;
    }
    let pos = s.rfind(".len()")?;
    let inner = s[..pos].trim();
    let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
    Some(format!("#{inner_alloy}"))
}

fn try_reverse_comparison(s: &str) -> Option<String> {
    // Find comparison operators at the top level (not inside parens/braces/closures)
    for (rust_op, alloy_op) in &[(" == ", " = "), (" != ", " != "), (" <= ", " <= "),
                                   (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = find_top_level(s, rust_op) {
            let left = s[..pos].trim();
            let right = s[pos + rust_op.len()..].trim();
            let left_alloy = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
            let right_alloy = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
            return Some(format!("{left_alloy}{alloy_op}{right_alloy}"));
        }
    }
    None
}

fn try_reverse_logic(s: &str) -> Option<String> {
    // !a || b → a implies b (only when ! applies to the left side of ||)
    if s.starts_with('!') {
        let inner = &s[1..];
        if let Some(pos) = find_top_level(inner, " || ") {
            let a = inner[..pos].trim();
            let a = strip_balanced_parens(a);
            let b = inner[pos + 4..].trim();
            let a_alloy = reverse_translate_expr(a).unwrap_or_else(|| a.to_string());
            let b_alloy = reverse_translate_expr(b).unwrap_or_else(|| b.to_string());
            return Some(format!("{a_alloy} implies {b_alloy}"));
        }
    }
    // a && b → a and b
    if let Some(pos) = find_top_level(s, " && ") {
        let left = s[..pos].trim();
        let right = s[pos + 4..].trim();
        let left_alloy = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
        let right_alloy = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
        return Some(format!("{left_alloy} and {right_alloy}"));
    }
    // a || b → a or b
    if let Some(pos) = find_top_level(s, " || ") {
        let left = s[..pos].trim();
        let right = s[pos + 4..].trim();
        let left_alloy = reverse_translate_expr(left).unwrap_or_else(|| left.to_string());
        let right_alloy = reverse_translate_expr(right).unwrap_or_else(|| right.to_string());
        return Some(format!("{left_alloy} or {right_alloy}"));
    }
    // !a → not a
    if s.starts_with('!') {
        let inner = s[1..].trim();
        let inner_alloy = reverse_translate_expr(inner).unwrap_or_else(|| inner.to_string());
        return Some(format!("not {inner_alloy}"));
    }
    None
}

/// Find a pattern at the top level (not inside parens, braces, or closures).
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

/// Find matching close delimiter, starting after the opening one.
fn find_matching_close(s: &str, _open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in s.chars().enumerate() {
        if ch == '(' || ch == '{' { depth += 1; }
        if ch == ')' || ch == '}' {
            if depth == 0 && ch == close { return Some(i); }
            depth -= 1;
        }
    }
    None
}

/// Convert snake_case_plural to PascalCase (e.g., "sig_decls" → "SigDecl").
fn snake_to_pascal(s: &str) -> String {
    // Strip trailing 's' for plural
    let s = s.strip_suffix('s').unwrap_or(s);
    let mut result = String::new();
    let mut capitalize_next = true;
    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Extract fact candidates from code patterns.
fn extract_facts_from_lines(
    body: &[(usize, String)],
    _context_line: usize,
    facts: &mut Vec<MinedFactCandidate>,
) {
    // First extract @alloy comments
    extract_alloy_comments(
        body.iter().map(|(ln, line)| (*ln, line.clone())),
        facts,
    );

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
