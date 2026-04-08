/// Extracts Alloy model candidates from TypeScript source code.
/// Handles: interface → sig, discriminated union → abstract sig + sub sigs,
/// T | null → lone, T[] → set, T → one.

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

        // export interface Foo { ... } → sig
        if let Some((name, iface_parent)) = parse_interface_decl(trimmed) {
            // Self-closing interface on one line: "export interface Foo {}"
            let fields = if trimmed.contains('{') && trimmed.contains('}') {
                vec![]
            } else {
                collect_interface_fields(&mut lines)
            };
            // Skip discriminant fields (kind/type with string literal values)
            let real_fields: Vec<MinedField> = fields
                .into_iter()
                .filter(|f| !is_discriminant_field(f))
                .collect();
            sigs.push(MinedSig {
                name,
                fields: real_fields,
                is_abstract: false,
                is_var: sig_is_var,
                parent: iface_parent,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![],
            });
        }

        // export type Foo = A & B & C → sig with intersection_of
        if let Some((name, components)) = parse_type_intersection(trimmed) {
            sigs.push(MinedSig {
                name,
                fields: vec![],
                is_abstract: false,
                is_var: sig_is_var,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: components,
            });
        }

        // export type Foo = "A" | "B" → abstract sig + one sig children (string literal union)
        // export type Foo = A | B     → abstract sig + sub sig children (discriminated union)
        // Also handles multi-line: export type Foo =\n  | A\n  | B;
        if let Some((name, variants)) = parse_type_union(trimmed)
            .or_else(|| parse_multiline_type_union(trimmed, &mut lines))
        {
            let is_string_literal = variants.iter().all(|v| v.starts_with('"'));
            if is_string_literal {
                // String literal union → abstract sig + one sig per literal
                sigs.push(MinedSig {
                    name: name.clone(),
                    fields: vec![],
                    is_abstract: true,
                    is_var: sig_is_var,
                    parent: None,
                    source_location: format!("line {}", line_num + 1),
                    intersection_of: vec![],
                });
                for v in &variants {
                    let vname = v.trim_matches('"').to_string();
                    sigs.push(MinedSig {
                        name: vname,
                        fields: vec![],
                        is_abstract: false,
                        is_var: false,
                        parent: Some(name.clone()),
                        source_location: format!("line {}", line_num + 1),
                        intersection_of: vec![],
                    });
                }
            } else {
                // Discriminated union → mark parent as abstract, set parent on existing sigs
                sigs.push(MinedSig {
                    name: name.clone(),
                    fields: vec![],
                    is_abstract: true,
                    is_var: sig_is_var,
                    parent: None,
                    source_location: format!("line {}", line_num + 1),
                    intersection_of: vec![],
                });
                // Set parent on existing sigs that match variant names
                for v in &variants {
                    if let Some(existing) = sigs.iter_mut().find(|s| s.name == *v) {
                        existing.parent = Some(name.clone());
                    }
                }
            }
        }

        // Function bodies: extract general patterns
        if trimmed.starts_with("export function ") || trimmed.starts_with("function ") {
            let body = collect_block(&mut lines);
            extract_ts_facts(&body, line_num, &mut fact_candidates);
        }
    }

    // Extract @temporal annotations from generated tests
    super::extract_temporal_annotations(source, &mut fact_candidates);

    MinedModel { sigs, fact_candidates }
}

/// A discriminant field is a field named "type" or "kind" whose target
/// looks like a string literal value (e.g. `type: 'heading'`, `kind: "Literal"`).
fn is_discriminant_field(f: &MinedField) -> bool {
    // `kind` fields are always discriminants (oxidtr convention)
    if f.name == "kind" { return true; }
    // `type` fields are discriminants when the target looks like a literal tag
    // (starts with lowercase — e.g. 'heading', 'paragraph')
    if f.name == "type" {
        return f.target.chars().next().map_or(false, |c| c.is_ascii_lowercase());
    }
    false
}

/// Returns (name, first_parent) for an interface declaration.
/// Handles `interface Foo`, `interface Foo<T>`, `interface Foo extends Bar`,
/// and `interface Foo extends Bar, Baz` (takes first parent only).
fn parse_interface_decl(line: &str) -> Option<(String, Option<String>)> {
    let rest = line.strip_prefix("export interface ")
        .or_else(|| line.strip_prefix("interface "))?;

    // Extract name (up to '<', ' ', or '{')
    let name: String = rest.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { return None; }

    // Look for `extends` keyword after the name (skip generics <...>)
    let after_name = &rest[name.len()..];
    let after_generics = if after_name.starts_with('<') {
        // Skip generic params: find matching '>'
        let mut depth = 0usize;
        let mut end = 0;
        for (i, ch) in after_name.chars().enumerate() {
            match ch { '<' => depth += 1, '>' => { depth -= 1; if depth == 0 { end = i + 1; break; } } _ => {} }
        }
        &after_name[end..]
    } else {
        after_name
    };

    let parent = if let Some(ext) = after_generics.trim_start().strip_prefix("extends ") {
        // Take first parent (before ',', '<', or '{')
        let first: String = ext.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if first.is_empty() { None } else { Some(first) }
    } else {
        None
    };

    Some((name, parent))
}

fn parse_type_union(line: &str) -> Option<(String, Vec<String>)> {
    let rest = line.strip_prefix("export type ")
        .or_else(|| line.strip_prefix("type "))?;
    let eq_pos = rest.find('=')?;
    let name: String = rest[..eq_pos].trim().to_string();
    if name.is_empty() { return None; }

    let rhs = rest[eq_pos + 1..].trim().trim_end_matches(';').trim();
    // Skip intersection types (handled by parse_type_intersection)
    if rhs.contains("& ") && !rhs.contains('|') { return None; }
    // Skip empty rhs (multi-line union handled by parse_multiline_type_union)
    if rhs.is_empty() { return None; }

    let variants: Vec<String> = rhs.split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if variants.is_empty() { return None; }

    Some((name, variants))
}

/// Parse multi-line type union:
///   export type Foo =
///     | A
///     | B;
fn parse_multiline_type_union(
    line: &str,
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Option<(String, Vec<String>)> {
    let rest = line.strip_prefix("export type ")
        .or_else(|| line.strip_prefix("type "))?;
    let eq_pos = rest.find('=')?;
    let name: String = rest[..eq_pos].trim().to_string();
    if name.is_empty() { return None; }
    let rhs = rest[eq_pos + 1..].trim();
    // Only match if rhs is empty or starts with `|` (beginning of multi-line union)
    if !rhs.is_empty() && !rhs.starts_with('|') { return None; }

    let mut variants: Vec<String> = Vec::new();
    // Collect initial rhs if it starts with `|`
    if rhs.starts_with('|') {
        let v = rhs.trim_start_matches('|').trim().trim_end_matches(';').trim().to_string();
        if !v.is_empty() { variants.push(v); }
    }
    // Collect subsequent lines starting with `|`
    while let Some((_, next_line)) = lines.peek() {
        let trimmed = next_line.trim();
        if let Some(rest) = trimmed.strip_prefix('|') {
            let v = rest.trim().trim_end_matches(';').trim().to_string();
            if !v.is_empty() { variants.push(v); }
            lines.next();
        } else {
            break;
        }
    }
    if variants.is_empty() { return None; }
    Some((name, variants))
}

/// Detect intersection type aliases: "export type Foo = A & B & C"
/// Returns Some((name, components)) if this is an intersection type.
fn parse_type_intersection(line: &str) -> Option<(String, Vec<String>)> {
    let rest = line.strip_prefix("export type ")
        .or_else(|| line.strip_prefix("type "))?;
    let eq_pos = rest.find('=')?;
    let name: String = rest[..eq_pos].trim()
        .chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { return None; }

    let rhs = rest[eq_pos + 1..].trim().trim_end_matches(';').trim();
    // Must contain & but not |
    if !rhs.contains('&') || rhs.contains('|') { return None; }

    let components: Vec<String> = rhs.split('&')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if components.len() < 2 { return None; }

    Some((name, components))
}

fn collect_interface_fields(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<MinedField> {
    let mut fields = Vec::new();
    let mut depth = 1usize;
    let mut prev_line_has_var = false;
    // Track inline object depth separately to avoid mis-counting type annotations.
    // When depth==1 and we encounter a field line whose type part has unbalanced
    // braces, we accumulate them here and skip those lines for field parsing.
    let mut inline_depth: isize = 0;

    for (_ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        let depth_before = depth;

        // Single-line JSDoc /** ... */ or block comment content: skip brace counting entirely.
        // These lines cannot be field declarations and their braces (e.g. `{ y: 10 }`)
        // must not affect the interface depth.
        if (trimmed.starts_with("/**") && trimmed.ends_with("*/"))
            || trimmed.starts_with("* ")
            || trimmed == "*"
            || trimmed.starts_with("*/")
        {
            continue;
        }

        if inline_depth > 0 {
            // Consuming lines inside a multi-line inline object type.
            for ch in trimmed.chars() {
                match ch {
                    '{' => inline_depth += 1,
                    '}' => inline_depth -= 1,
                    _ => {}
                }
            }
            if inline_depth <= 0 {
                inline_depth = 0;
                // The closing '}' of the inline object is consumed.
                // Depth of the outer interface is unchanged.
            }
            continue;
        }

        // Count braces, but be careful about inline object types.
        // For a field line at depth==1 (e.g. `bounds?: { x?: number; };`),
        // braces in the type annotation are balanced on the same line.
        // For a multi-line inline object (open brace not closed on same line),
        // we enter inline_depth mode.
        if depth_before == 1 {
            let colon_pos = trimmed.find(':');
            if let Some(cp) = colon_pos {
                if !trimmed.starts_with('}') {
                    let before = &trimmed[..cp];
                    let type_part = &trimmed[cp..];
                    // Count pre-colon braces (e.g., the outer `}` only)
                    for ch in before.chars() {
                        match ch { '{' => depth += 1, '}' => { if depth > 0 { depth -= 1; } } _ => {} }
                    }
                    // Net brace count in the type part
                    let open_t = type_part.chars().filter(|&c| c == '{').count() as isize;
                    let close_t = type_part.chars().filter(|&c| c == '}').count() as isize;
                    let net = open_t - close_t;
                    if net > 0 {
                        // Multi-line inline object: enter inline tracking mode
                        inline_depth = net;
                    }
                    // Balanced or over-closed: no depth change for outer interface
                } else {
                    // Line starts with '}': closing the interface or a nested block
                    for ch in trimmed.chars() {
                        match ch { '{' => depth += 1, '}' => { if depth > 0 { depth -= 1; } } _ => {} }
                    }
                }
            } else {
                for ch in trimmed.chars() {
                    match ch { '{' => depth += 1, '}' => { if depth > 0 { depth -= 1; } } _ => {} }
                }
            }
        } else {
            for ch in trimmed.chars() {
                match ch { '{' => depth += 1, '}' => { if depth > 0 { depth -= 1; } } _ => {} }
            }
        }

        if depth == 0 { break; }

        if depth_before == 1 && inline_depth == 0 {
            if trimmed.contains("@alloy: var") {
                prev_line_has_var = true;
            } else if let Some(mut field) = parse_ts_field(trimmed) {
                if prev_line_has_var {
                    field.is_var = true;
                }
                fields.push(field);
                prev_line_has_var = false;
            } else {
                prev_line_has_var = false;
            }
        }
    }
    fields
}

fn parse_ts_field(line: &str) -> Option<MinedField> {
    let trimmed = line.trim().trim_end_matches(';').trim_end_matches(',').trim();
    if trimmed.is_empty() || trimmed.starts_with("//") { return None; }

    // Issue 1: Skip JSDoc comment lines (/** ..., * ..., */)
    if trimmed.starts_with("/**")
        || trimmed.starts_with("*/")
        || trimmed.starts_with("* ")
        || trimmed == "*"
    {
        return None;
    }

    // readonly fields are non-var; absence of readonly means var (mutable across states)
    let has_readonly = trimmed.starts_with("readonly ");
    let rest = if has_readonly { &trimmed["readonly ".len()..] } else { trimmed };

    let colon = rest.find(':')?;

    // Issue 2: Skip method signatures — '(' before ':' means it's a method, not a property
    if rest[..colon].contains('(') { return None; }

    let mut name = rest[..colon].trim().to_string();
    let optional = name.ends_with('?');
    if optional {
        name = name.trim_end_matches('?').to_string();
    }
    if name.is_empty() { return None; }

    let type_str = rest[colon + 1..].trim();
    if type_str.is_empty() { return None; }

    // Issue 4: Single string literal discriminant (kind: "Foo" or type: 'foo').
    // Must NOT be a union — "frame" | "scene" starts/ends with '"' but contains " | ".
    let is_double_quoted = type_str.starts_with('"') && type_str.ends_with('"') && !type_str.contains(" | ");
    let is_single_quoted = type_str.starts_with('\'') && type_str.ends_with('\'') && !type_str.contains(" | ");
    if is_double_quoted || is_single_quoted {
        let quote_char = if is_double_quoted { '"' } else { '\'' };
        return Some(MinedField {
            name,
            is_var: !has_readonly,
            mult: MinedMultiplicity::One,
            target: type_str.trim_matches(quote_char).to_string(),
            raw_union_type: None,
        });
    }

    // Skip callback/function types: `(args) => ReturnType`
    // These are not data fields — they are method signatures embedded as properties
    if is_callback_type(type_str) {
        return None;
    }

    let raw_union = detect_union_type(type_str);
    let (mult, target) = ts_type_to_mult(type_str, optional);
    Some(MinedField { name, is_var: !has_readonly, mult, target, raw_union_type: raw_union })
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

/// Detect field-level union types like "number | string".
/// Returns Some(raw) if it is a non-null union, None otherwise.
fn detect_union_type(ts_type: &str) -> Option<String> {
    let t = ts_type.trim();
    // Must contain " | " but not be a null-union (already handled as lone)
    if !t.contains(" | ") { return None; }
    if t.contains("null") { return None; }
    // Skip callback types that happen to contain "|" inside parameter lists
    if is_callback_type(t) { return None; }
    Some(t.to_string())
}

/// Detect callback/function type signatures: `(args) => ReturnType`
/// Also handles union of callback + other: `string | (item: S) => T`
fn is_callback_type(ts_type: &str) -> bool {
    let t = ts_type.trim();
    // Direct callback: starts with ( and contains =>
    if t.starts_with('(') && t.contains("=>") { return true; }
    // Union containing a callback: any variant starts with (
    if t.contains(" | ") {
        for part in t.split(" | ") {
            let p = part.trim();
            if p.starts_with('(') && p.contains("=>") { return true; }
        }
    }
    false
}

fn strip_wrapper_ts<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)?.strip_suffix(suffix)
}

fn collect_block(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<(usize, String)> {
    let mut body = Vec::new();
    // depth starts at 0: we haven't seen the opening '{' yet.
    // For single-line trigger (e.g. `function f() {`), the caller's line already
    // contained the '{', so we start at depth=1 only when the trigger line had one.
    // Here we conservatively start at 0 and increment on the first '{' we see.
    let mut depth = 0usize;
    let mut started = false;

    for (ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch {
                '{' => { depth += 1; started = true; }
                '}' => { if depth > 0 { depth -= 1; } }
                _ => {}
            }
        }
        if started && depth == 0 { break; }
        body.push((ln, trimmed.to_string()));
    }
    body
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

fn extract_ts_facts(
    body: &[(usize, String)],
    _context_line: usize,
    facts: &mut Vec<MinedFactCandidate>,
) {
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
        if line.contains(".length") && !line.contains("?.") {
            facts.push(MinedFactCandidate {
                alloy_text: extract_length_fact(line),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: ".length check".to_string(),
            });
        }

        // Category 1: if (x === null) throw → null guard with presence info
        if (line.contains("=== null") || line.contains("!== null")) && line.contains("throw") {
            let field = extract_null_guard_field(line);
            facts.push(MinedFactCandidate {
                alloy_text: format!("some {field} -- presence constraint"),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "null guard throw".to_string(),
            });
        } else if line.contains("=== null") || line.contains("!== null") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- null check (lone field constraint)".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "null check".to_string(),
            });
        }

        // Category 1: if (x === undefined) throw → presence guard
        if line.contains("=== undefined") && line.contains("throw") {
            let field = extract_null_guard_field(line);
            facts.push(MinedFactCandidate {
                alloy_text: format!("some {field} -- presence constraint"),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "null guard throw".to_string(),
            });
        }

        // Category 1: Array.isArray type guard
        if line.contains("Array.isArray(") && line.contains("throw") {
            if let Some(pos) = line.find("Array.isArray(") {
                let rest = &line[pos + "Array.isArray(".len()..];
                let end = rest.find(')').unwrap_or(rest.len());
                let arg = rest[..end].trim();
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{arg} is seq -- type guard"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "type guard Array.isArray".to_string(),
                });
            }
        }

        // throw new Error → precondition guard (Low) — only if not already handled above
        if (line.contains("throw new Error") || line.contains("throw "))
            && !line.contains("=== null") && !line.contains("=== undefined")
            && !line.contains("Array.isArray") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- precondition guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "throw pattern".to_string(),
            });
        }

        // Category 3: switch on discriminant → variant evidence
        if line.contains("switch (") || line.contains("switch(") {
            let variants = collect_switch_variants(body, *ln);
            if !variants.is_empty() {
                let variants_str = variants.join(", ");
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- enum variants: {variants_str}"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "switch exhaustiveness".to_string(),
                });
            }
        }

        // Category 4: optional chaining ?.
        if line.contains("?.") && !line.contains("??") {
            if let Some(field) = extract_optional_chain_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is lone (optional)"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "optional chaining".to_string(),
                });
            }
        }

        // Category 4: nullish coalescing ??
        if line.contains("??") {
            if let Some(field) = extract_nullish_coalescing_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is lone (with default)"),
                    confidence: Confidence::Medium,
                    source_location: loc.clone(),
                    source_pattern: "nullish coalescing".to_string(),
                });
            }
        }

        // Category 4: non-null assertion x!
        if line.contains("!.") {
            if let Some(field) = extract_non_null_assertion_field(line) {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("{field} is one (non-null assertion)"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "non-null assertion".to_string(),
                });
            }
        }

        // Category 5: .filter() — subset
        if line.contains(".filter(") {
            if let Some(collection) = extract_ts_collection_before(line, ".filter(") {
                facts.push(MinedFactCandidate {
                    alloy_text: format!("-- subset of {collection}"),
                    confidence: Confidence::Low,
                    source_location: loc.clone(),
                    source_pattern: "filter subset".to_string(),
                });
            }
        }

        // Category 5: .map() — field projection
        if line.contains(".map(") && !line.contains(".filter(") {
            if let Some(collection) = extract_ts_collection_before(line, ".map(") {
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

/// Extract the field name from a null guard like "if (x === null)" or "if (x === null || x === undefined)"
fn extract_null_guard_field(line: &str) -> String {
    // Try to find identifier before === null
    if let Some(pos) = line.find("=== null") {
        let before = line[..pos].trim();
        let field = before.rsplit(|c: char| c == '(' || c == ' ' || c == '!').next().unwrap_or(before).trim();
        if !field.is_empty() {
            return field.to_string();
        }
    }
    "unknown".to_string()
}

/// Collect switch case variant names from body lines.
fn collect_switch_variants(body: &[(usize, String)], switch_ln: usize) -> Vec<String> {
    let mut variants = Vec::new();
    let mut in_switch = false;
    for (ln, line) in body {
        if *ln == switch_ln { in_switch = true; continue; }
        if !in_switch { continue; }
        let trimmed = line.trim();
        if trimmed == "}" { break; }
        // case "Variant": or case Variant:
        if trimmed.starts_with("case ") {
            let rest = &trimmed[5..];
            let end = rest.find(':').unwrap_or(rest.len());
            let variant = rest[..end].trim().trim_matches('"');
            if !variant.is_empty() {
                variants.push(variant.to_string());
            }
        }
    }
    variants
}

/// Extract field before ?. operator
fn extract_optional_chain_field(line: &str) -> Option<String> {
    let pos = line.find("?.")?;
    let before = line[..pos].trim();
    let field = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{' || c == ',').next().unwrap_or(before).trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Extract field before ?? operator
fn extract_nullish_coalescing_field(line: &str) -> Option<String> {
    let pos = line.find("??")?;
    let before = line[..pos].trim();
    let field = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{').next().unwrap_or(before).trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Extract field before !. non-null assertion
fn extract_non_null_assertion_field(line: &str) -> Option<String> {
    let pos = line.find("!.")?;
    let before = line[..pos].trim();
    let field = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{' || c == ',').next().unwrap_or(before).trim();
    if field.is_empty() { None } else { Some(field.to_string()) }
}

/// Extract collection name before a method call
fn extract_ts_collection_before(line: &str, method: &str) -> Option<String> {
    let pos = line.find(method)?;
    let before = line[..pos].trim();
    let collection = before.rsplit(|c: char| c == ' ' || c == '=' || c == '(' || c == '{').next().unwrap_or(before).trim();
    if collection.is_empty() { None } else { Some(collection.to_string()) }
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
