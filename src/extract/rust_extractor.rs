/// Extracts Alloy model candidates from Rust source code.
/// Uses lightweight line-based parsing (same approach as check/impl_parser).

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();

    // Pass 1: Extract struct/enum declarations using depth tracking.
    // Only recognize struct/enum at depth 0 (top-level).
    {
        let mut depth: usize = 0;
        let all_lines: Vec<(usize, &str)> = source.lines().enumerate().collect();
        let mut i = 0;
        let mut prev_line_has_var_sig = false;
        while i < all_lines.len() {
            let (line_num, line) = all_lines[i];
            let trimmed = line.trim();

            // At top level (depth 0), check for struct/enum
            if depth == 0 {
                if let Some(name) = parse_type_decl(trimmed, "pub struct ") {
                    let sig_is_var = prev_line_has_var_sig;
                    prev_line_has_var_sig = false;
                    let is_unit = trimmed.ends_with(';')
                        || !trimmed.contains('{')
                        || (trimmed.contains('{') && trimmed.contains('}'));
                    if is_unit {
                        sigs.push(MinedSig {
                            name,
                            fields: vec![],
                            is_abstract: false,
                            is_var: sig_is_var,
                            parent: None,
                            source_location: format!("line {}", line_num + 1),
                            intersection_of: vec![],
                        });
                        i += 1; // advance past this unit struct line
                    } else {
                        // Collect fields from subsequent lines
                        let mut fields = Vec::new();
                        let mut block_depth = 1usize;
                        let mut prev_line_has_var = false;
                        i += 1;
                        while i < all_lines.len() && block_depth > 0 {
                            let inner_trimmed = all_lines[i].1.trim();
                            for ch in inner_trimmed.chars() {
                                match ch {
                                    '{' => block_depth += 1,
                                    '}' => block_depth = block_depth.saturating_sub(1),
                                    _ => {}
                                }
                            }
                            if block_depth > 0 {
                                if let Some(mut field) = parse_rust_field(inner_trimmed) {
                                    if prev_line_has_var {
                                        field.is_var = true;
                                    }
                                    fields.push(field);
                                    prev_line_has_var = false;
                                } else if inner_trimmed.contains("@alloy: var") {
                                    prev_line_has_var = true;
                                } else {
                                    prev_line_has_var = false;
                                }
                            }
                            i += 1;
                        }
                        sigs.push(MinedSig {
                            name,
                            fields,
                            is_abstract: false,
                            is_var: sig_is_var,
                            parent: None,
                            source_location: format!("line {}", line_num + 1),
                            intersection_of: vec![],
                        });
                    }
                    continue;
                }

                if let Some(name) = parse_type_decl(trimmed, "pub enum ") {
                    let sig_is_var = prev_line_has_var_sig;
                    prev_line_has_var_sig = false;
                    // Collect enum variants
                    let mut variants: Vec<(String, Vec<MinedField>)> = Vec::new();
                    let mut block_depth = 1usize;
                    i += 1;
                    while i < all_lines.len() && block_depth > 0 {
                        let inner = all_lines[i].1.trim();
                        let open = inner.chars().filter(|&c| c == '{').count();
                        let close = inner.chars().filter(|&c| c == '}').count();
                        block_depth = block_depth + open - close.min(block_depth + open);

                        if block_depth >= 1 {
                            // Strip comments first, then trailing comma
                            let cleaned = if let Some(cp) = inner.find("//") {
                                inner[..cp].trim()
                            } else { inner };
                            let cleaned = cleaned.trim_end_matches(',').trim();

                            if !cleaned.is_empty() {
                                let first = cleaned.chars().next().unwrap_or(' ');
                                if first.is_ascii_uppercase() {
                                    if let Some(brace_pos) = cleaned.find('{') {
                                        // Struct variant (single-line or multi-line)
                                        let vname: String = cleaned[..brace_pos].trim().to_string();
                                        if !vname.is_empty() && vname.chars().all(|c| c.is_alphanumeric() || c == '_') {
                                            let mut vfields = Vec::new();
                                            // Check if it's a single-line variant: "Foo { field: Type }"
                                            if cleaned.contains('}') {
                                                // Single-line: parse fields from the line itself
                                                let inner = &cleaned[brace_pos + 1..];
                                                let close = inner.find('}').unwrap_or(inner.len());
                                                let fields_str = &inner[..close];
                                                for part in fields_str.split(',') {
                                                    let part = part.trim();
                                                    if let Some(colon) = part.find(':') {
                                                        let fname = part[..colon].trim();
                                                        if !fname.is_empty() && fname.chars().next().map_or(false, |c| c.is_ascii_lowercase()) {
                                                            let type_str = part[colon + 1..].trim();
                                                            if !type_str.is_empty() {
                                                                let (mult, target) = rust_type_to_mult(type_str);
                                                                vfields.push(MinedField { name: fname.to_string(), is_var: false, mult, target, raw_union_type: None });
                                                            }
                                                        }
                                                    }
                                                }
                                                i += 1; // advance past this single-line variant
                                            } else {
                                                // Multi-line: read until closing }
                                                i += 1;
                                                while i < all_lines.len() {
                                                    let vline = all_lines[i].1.trim();
                                                    let vo = vline.chars().filter(|&c| c == '{').count();
                                                    let vc = vline.chars().filter(|&c| c == '}').count();
                                                    block_depth = block_depth + vo - vc.min(block_depth + vo);
                                                    let vclean = vline.trim_end_matches(',').trim_end_matches('}').trim();
                                                    if let Some(colon) = vclean.find(':') {
                                                        let fname = vclean[..colon].trim();
                                                        if !fname.is_empty() && fname.chars().next().map_or(false, |c| c.is_ascii_lowercase()) {
                                                            let type_str = vclean[colon + 1..].trim();
                                                            if !type_str.is_empty() {
                                                                let (mult, target) = rust_type_to_mult(type_str);
                                                                vfields.push(MinedField { name: fname.to_string(), is_var: false, mult, target, raw_union_type: None });
                                                            }
                                                        }
                                                    }
                                                    if vline.contains('}') { i += 1; break; }
                                                    i += 1;
                                                }
                                            }
                                            variants.push((vname, vfields));
                                            continue;
                                        }
                                    } else if let Some(paren_pos) = cleaned.find('(') {
                                        // Tuple variant
                                        let vname: String = cleaned[..paren_pos].trim().to_string();
                                        if !vname.is_empty() && vname.chars().all(|c| c.is_alphanumeric() || c == '_') {
                                            let close_paren = cleaned.rfind(')').unwrap_or(cleaned.len());
                                            let params_str = &cleaned[paren_pos + 1..close_paren];
                                            let vfields: Vec<MinedField> = params_str.split(',')
                                                .enumerate()
                                                .filter_map(|(fi, p)| {
                                                    let p = p.trim();
                                                    if p.is_empty() { return None; }
                                                    let (mult, target) = rust_type_to_mult(p);
                                                    Some(MinedField { name: format!("field{fi}"), is_var: false, mult, target, raw_union_type: None })
                                                })
                                                .collect();
                                            variants.push((vname, vfields));
                                        }
                                    } else if cleaned.chars().all(|c| c.is_alphanumeric() || c == '_') {
                                        // Unit variant
                                        variants.push((cleaned.to_string(), vec![]));
                                    }
                                }
                            }
                        }
                        if block_depth == 0 { i += 1; break; }
                        i += 1;
                    }
                    sigs.push(MinedSig {
                        name: name.clone(),
                        fields: vec![],
                        is_abstract: true,
                        is_var: sig_is_var,
                        parent: None,
                        source_location: format!("line {}", line_num + 1),
                        intersection_of: vec![],
                    });
                    for (vname, vfields) in variants {
                        sigs.push(MinedSig {
                            name: vname,
                            fields: vfields,
                            is_abstract: false,
                            is_var: false,
                            parent: Some(name.clone()),
                            source_location: format!("line {}", line_num + 1),
                            intersection_of: vec![],
                        });
                    }
                    continue;
                }

                // Detect @alloy: var sig annotation for the next declaration
                if trimmed.contains("@alloy: var sig") {
                    prev_line_has_var_sig = true;
                } else {
                    prev_line_has_var_sig = false;
                }
            }

            // Track depth for all lines
            for ch in trimmed.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            i += 1;
        }
    }

    // Pass 2: Extract fact candidates from function bodies
    {
        let mut lines = source.lines().enumerate().peekable();
        while let Some((line_num, line)) = lines.next() {
            let trimmed = line.trim();
            if trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") {
                let body = collect_fn_body(&mut lines);
                extract_facts_from_lines(&body, line_num, &mut fact_candidates);
            }
        }
    }

    // Pass 3: Extract temporal constraint annotations (@temporal markers)
    extract_temporal_annotations(source, &mut fact_candidates);

    MinedModel { sigs, fact_candidates }
}


fn parse_type_decl(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}


fn parse_rust_field(line: &str) -> Option<MinedField> {
    let rest = line.strip_prefix("pub ")?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim().to_string();
    if name.is_empty() { return None; }
    let type_str = rest[colon + 1..].trim().trim_end_matches(',').trim();
    if type_str.is_empty() { return None; }
    let (mult, target) = rust_type_to_mult(type_str);
    Some(MinedField { name, is_var: false, mult, target, raw_union_type: None })
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


fn collect_fn_body(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<(usize, String)> {
    let mut body = Vec::new();
    let mut depth = 1usize;

    for (ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth = depth.saturating_sub(1), _ => {} }
        }
        if depth == 0 { break; }
        body.push((ln, trimmed.to_string()));
    }
    body
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
                depth = depth.saturating_sub(1);
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
                depth = depth.saturating_sub(1);
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
            b')' | b'}' | b']' => depth = depth.saturating_sub(1),
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
            depth = depth.saturating_sub(1);
        }
    }
    None
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
                // Try to translate the condition to Alloy-like syntax
                let alloy_text = translate_rust_condition(&cond);
                facts.push(MinedFactCandidate {
                    alloy_text,
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
        if line.contains(".contains(") && !line.contains(".filter(") {
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

        // Category 2: Guard clause — is_none() guard
        if line.contains(".is_none()") && (line.contains("return Err") || line.contains("return ")) {
            if let Some(pos) = line.find(".is_none()") {
                let before = line[..pos].trim().trim_start_matches("if ").trim();
                facts.push(MinedFactCandidate {
                    alloy_text: format!("some {before} -- field should be one not lone"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "is_none guard".to_string(),
                });
            }
        }

        // Category 2: let Some(x) = field else { return Err(...) }
        if line.contains("let Some(") && line.contains("else") {
            if let Some(field) = extract_let_else_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("some {field} -- presence constraint"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "let-else presence guard".to_string(),
                });
            }
        }

        // Category 3: match exhaustiveness — detect match arms
        if line.contains("match ") {
            // Collect subsequent lines for match arms
            let match_arms = collect_match_variants(body, *ln);
            if !match_arms.is_empty() {
                let variants_str = match_arms.join(", ");
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- enum variants: {variants_str}"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "match exhaustiveness".to_string(),
                });
            }
        }

        // Category 4: if let Some(x) = field
        if line.contains("if let Some(") {
            if let Some(field) = extract_if_let_some_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is lone (optional)"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "if let Some".to_string(),
                });
            }
        }

        // Category 4: .unwrap() — unsafe presence hint
        if line.contains(".unwrap()") && !line.contains("unwrap_or") {
            if let Some(pos) = line.find(".unwrap()") {
                let before = line[..pos].trim();
                // Extract the last identifier/chain before .unwrap()
                let field = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(').next().unwrap_or(before).trim();
                if !field.is_empty() {
                    facts.push(MinedFactCandidate {
                        alloy_text: format!("{field} is one (unsafe unwrap)"),
                        confidence: Confidence::Low,
                        source_location: loc.clone(),
                        source_pattern: "unwrap hint".to_string(),
                    });
                }
            }
        }

        // Category 4: .unwrap_or(default) — lone with default
        if line.contains(".unwrap_or(") || line.contains(".unwrap_or_default()") {
            if let Some(pos) = line.find(".unwrap_or") {
                let before = line[..pos].trim();
                let field = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(').next().unwrap_or(before).trim();
                if !field.is_empty() {
                    facts.push(MinedFactCandidate {
                        alloy_text: format!("{field} is lone (with default)"),
                        confidence: Confidence::Medium,
                        source_location: loc.clone(),
                        source_pattern: "unwrap_or default".to_string(),
                    });
                }
            }
        }

        // Category 5: .filter() — subset relation
        if line.contains(".filter(") || line.contains(".iter().filter(") {
            if let Some(collection) = extract_collection_before_method(line, ".filter(") {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- subset of {collection}"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "filter subset".to_string(),
                });
            }
        }

        // Category 5: .map() — field mapping
        if line.contains(".map(") && (line.contains(".iter().map(") || line.contains(".into_iter().map(")) {
            if let Some(collection) = extract_collection_before_method(line, ".map(")
                .or_else(|| extract_collection_before_method(line, ".iter().map(")) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- field projection from {collection}"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "map projection".to_string(),
                });
            }
        }
    }
}

/// Translate a Rust condition string to Alloy-like syntax.
/// Handles .len() → # and .is_empty() → some/no conversions.
fn translate_rust_condition(cond: &str) -> String {
    let mut result = cond.to_string();
    // .len() → # prefix
    if result.contains(".len()") {
        if let Some(pos) = result.find(".len()") {
            let before = &result[..pos];
            let after = &result[pos + 6..];
            result = format!("#{before}{after}");
        }
    }
    // !xxx.is_empty() → some xxx
    if result.starts_with('!') && result.contains(".is_empty()") {
        let inner = result[1..].trim();
        if let Some(pos) = inner.find(".is_empty()") {
            let field = &inner[..pos];
            return format!("some {field}");
        }
    }
    result
}

/// Extract field name from `let Some(x) = FIELD else { ... }`
fn extract_let_else_field(line: &str) -> Option<String> {
    let eq_pos = line.find(" = ")?;
    let after_eq = &line[eq_pos + 3..];
    let field_end = after_eq.find(" else").or_else(|| after_eq.find(';')).unwrap_or(after_eq.len());
    let field = after_eq[..field_end].trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Extract field name from `if let Some(x) = FIELD { ... }`
fn extract_if_let_some_field(line: &str) -> Option<String> {
    let eq_pos = line.find(" = ")?;
    let after_eq = &line[eq_pos + 3..];
    let field_end = after_eq.find(" {").or_else(|| after_eq.find('{')).unwrap_or(after_eq.len());
    let field = after_eq[..field_end].trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Collect match arm variant names (Type::Variant) from body lines after a match statement.
fn collect_match_variants(body: &[(usize, String)], match_ln: usize) -> Vec<String> {
    let mut variants = Vec::new();
    let mut in_match = false;
    for (ln, line) in body {
        if *ln == match_ln { in_match = true; continue; }
        if !in_match { continue; }
        let trimmed = line.trim();
        if trimmed == "}" { break; }
        // Match arms: "Type::Variant => ..." or "Type::Variant { ... } => ..."
        if trimmed.contains("=>") {
            let arm = trimmed.split("=>").next().unwrap_or("").trim();
            // Extract variant: "Type::Variant" or "Type::Variant { ... }"
            if let Some(variant) = extract_match_variant(arm) {
                variants.push(variant);
            }
        }
    }
    variants
}

/// Extract variant name from a match arm pattern like "Status::Active" or "Status::Active { .. }"
fn extract_match_variant(arm: &str) -> Option<String> {
    let arm = arm.trim();
    if arm == "_" || arm.is_empty() { return None; }
    // "Type::Variant ..." → extract Variant
    if let Some(pos) = arm.find("::") {
        let after = &arm[pos + 2..];
        let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
            return Some(name);
        }
    }
    None
}

/// Extract collection name before a method like .filter( or .map(
fn extract_collection_before_method(line: &str, method: &str) -> Option<String> {
    let pos = line.find(method)?;
    let before = line[..pos].trim();
    // Strip .iter() if present
    let before = before.strip_suffix(".iter()").or_else(|| before.strip_suffix(".into_iter()")).unwrap_or(before);
    // Get the last token (variable name)
    let collection = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{').next().unwrap_or(before).trim();
    if collection.is_empty() { None } else { Some(collection.to_string()) }
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

/// Extract @temporal annotations — delegates to shared implementation.
fn extract_temporal_annotations(source: &str, facts: &mut Vec<MinedFactCandidate>) {
    super::extract_temporal_annotations(source, facts);
}
