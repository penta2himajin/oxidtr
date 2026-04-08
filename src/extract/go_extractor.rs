/// Extracts Alloy model candidates from Go source code.
/// Handles: struct → sig, interface+iota → abstract sig,
/// *T → lone, []T → set/seq, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    // Collect iota-based enums: type X int + const block
    let mut iota_types: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

    // First pass: find iota const blocks
    let source_lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    while i < source_lines.len() {
        let trimmed = source_lines[i].trim();
        if trimmed == "const (" {
            // Parse const block for iota pattern
            let mut first_type = None;
            let mut variants = Vec::new();
            i += 1;
            while i < source_lines.len() {
                let ct = source_lines[i].trim();
                if ct == ")" { break; }
                // "Name Type = iota" or just "Name"
                if ct.contains("= iota") {
                    let parts: Vec<&str> = ct.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let variant = parts[0].to_string();
                        let type_name = parts[1].to_string();
                        first_type = Some(type_name);
                        variants.push(variant);
                    }
                } else if first_type.is_some() && !ct.is_empty() && !ct.starts_with("//") {
                    let name: String = ct.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() {
                        variants.push(name);
                    }
                }
                i += 1;
            }
            if let Some(type_name) = first_type {
                iota_types.insert(type_name, variants);
            }
        }
        i += 1;
    }

    // Second pass: parse structs, interfaces, funcs
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

        // type Foo struct { → sig
        if let Some(name) = parse_go_struct(trimmed) {
            let fields = if is_inline_empty_struct(trimmed) {
                vec![]
            } else {
                collect_go_struct_fields(&mut lines)
            };
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

        // type Foo interface { → abstract sig (with or without marker method)
        if let Some(name) = parse_go_interface(trimmed) {
            if is_inline_empty_interface(trimmed) {
                // Empty interface on same line: `type Foo interface{}`
                sigs.push(MinedSig {
                    name,
                    fields: vec![],
                    is_abstract: true,
                    is_var: sig_is_var,
                    parent: None,
                    source_location: format!("line {}", line_num + 1),
                    intersection_of: vec![],
                });
            } else {
                // Multi-line interface: consume body, check for marker method
                let _marker_method = collect_go_interface_marker(&mut lines);
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
        }

        // type Foo int (enum type)
        if let Some(name) = parse_go_type_alias(trimmed) {
            if let Some(variants) = iota_types.get(&name) {
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
                        name: v.clone(),
                        fields: vec![],
                        is_abstract: false,
                        is_var: false,
                        parent: Some(name.clone()),
                        source_location: format!("line {}", line_num + 1),
                        intersection_of: vec![],
                    });
                }
            }
        }

        // func (t Type) isInterface() {} → variant marker
        if let Some((type_name, interface_name)) = parse_go_marker_method(trimmed) {
            // Mark type_name as child of interface_name
            for sig in &mut sigs {
                if sig.name == type_name && sig.parent.is_none() {
                    sig.parent = Some(interface_name);
                    break;
                }
            }
        }

        // func bodies: extract fact patterns
        // Only collect block if body is NOT entirely on the same line
        if trimmed.starts_with("func ") && !is_single_line_func(trimmed) {
            let body = collect_go_block(&mut lines);
            extract_go_facts(&body, line_num, &mut fact_candidates);
        }
    }

    // Extract @temporal annotations from generated tests
    super::extract_temporal_annotations(source, &mut fact_candidates);

    MinedModel { sigs, fact_candidates }
}

fn parse_go_struct(line: &str) -> Option<String> {
    // "type Foo struct {" or "type Foo struct{}"
    let rest = line.strip_prefix("type ")?;
    let space = rest.find(' ')?;
    let name = &rest[..space];
    if !rest[space..].trim_start().starts_with("struct") { return None; }
    if name.is_empty() || !name.chars().next()?.is_ascii_uppercase() { return None; }
    Some(name.to_string())
}

/// Check if a struct declaration is an inline empty struct (struct{} on same line).
fn is_inline_empty_struct(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.ends_with("struct{}")
}

/// Check if an interface declaration is an inline empty interface (`type X interface{}`).
fn is_inline_empty_interface(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.ends_with("interface{}") || trimmed.ends_with("interface {}")
}

/// Check if a func declaration has its body entirely on the same line (e.g., `func (X) isY() {}`).
fn is_single_line_func(line: &str) -> bool {
    let open_count = line.matches('{').count();
    let close_count = line.matches('}').count();
    open_count > 0 && open_count == close_count
}

fn parse_go_interface(line: &str) -> Option<String> {
    let rest = line.strip_prefix("type ")?;
    let space = rest.find(' ')?;
    let name = &rest[..space];
    if !rest[space..].trim_start().starts_with("interface") { return None; }
    if name.is_empty() { return None; }
    Some(name.to_string())
}

fn parse_go_type_alias(line: &str) -> Option<String> {
    // "type Foo int"
    let rest = line.strip_prefix("type ")?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() == 2 && parts[1] == "int" {
        let name = parts[0];
        if !name.is_empty() && name.chars().next()?.is_ascii_uppercase() {
            return Some(name.to_string());
        }
    }
    None
}

fn parse_go_marker_method(line: &str) -> Option<(String, String)> {
    // "func (t Type) isInterfaceName() {}" or "func (Type) isInterfaceName() {}"
    let rest = line.strip_prefix("func (")?;
    let paren_end = rest.find(')')?;
    let receiver = rest[..paren_end].trim();
    // receiver could be "t Type" or "Type"
    let type_name = receiver.split_whitespace().last()?.to_string();
    let after = rest[paren_end + 1..].trim();
    // "isXxx() {}" — after trim, should start with method name
    let method_name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if method_name.starts_with("is") && method_name.len() > 2 {
        let interface_name = method_name[2..].to_string();
        if !interface_name.is_empty() && interface_name.chars().next()?.is_ascii_uppercase() {
            return Some((type_name, interface_name));
        }
    }
    None
}

fn collect_go_struct_fields(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<MinedField> {
    let mut fields = Vec::new();
    let mut depth = 1usize;
    let mut prev_line_has_var = false;
    let mut prev_line_has_seq = false;

    for (_ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }

        // "FieldName Type" — Go struct field
        if let Some(mut field) = parse_go_field(trimmed) {
            if prev_line_has_var {
                field.is_var = true;
            }
            if prev_line_has_seq && field.mult == MinedMultiplicity::Set {
                field.mult = MinedMultiplicity::Seq;
            }
            fields.push(field);
            prev_line_has_var = false;
            prev_line_has_seq = false;
        } else if trimmed.contains("@alloy: var") && !trimmed.contains("@alloy: var sig") {
            prev_line_has_var = true;
        } else if trimmed.contains("@alloy: seq") {
            prev_line_has_seq = true;
        } else {
            prev_line_has_var = false;
            prev_line_has_seq = false;
        }
    }

    fields
}

fn parse_go_field(line: &str) -> Option<MinedField> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("//") || line.starts_with("/*") { return None; }
    // Split on whitespace, skipping empty parts (handles multi-space alignment)
    let mut parts = line.split_whitespace();
    let name = parts.next()?;
    if name.is_empty() || !name.chars().next()?.is_ascii_uppercase() { return None; }
    // Second token is the type; skip embedded types (single word lines like "BaseBlock")
    let type_str = parts.next()?;
    // Remove json tags (backtick-delimited)
    let type_str = if let Some(backtick) = type_str.find('`') {
        type_str[..backtick].trim()
    } else {
        type_str
    };
    if type_str.is_empty() { return None; }

    let (mult, target) = go_type_to_mult(type_str);
    // Convert Go field name (PascalCase) to camelCase for Alloy
    let field_name = to_camel_case(name);
    Some(MinedField { name: field_name, is_var: false, mult, target, raw_union_type: None })
}

fn go_type_to_mult(go_type: &str) -> (MinedMultiplicity, String) {
    let t = go_type.trim();

    // *T → lone
    if let Some(inner) = t.strip_prefix('*') {
        return (MinedMultiplicity::Lone, inner.to_string());
    }

    // []T → set (Go has no native set; slices serve both roles.
    //   Alloy set vs seq distinction is handled by @alloy: seq annotation
    //   or by Set≈Seq equivalence in the differ.)
    if let Some(inner) = t.strip_prefix("[]") {
        return (MinedMultiplicity::Set, inner.to_string());
    }

    // map[K]V → set (key as target)
    if t.starts_with("map[") {
        let bracket_end = t.find(']').unwrap_or(t.len());
        let key = &t[4..bracket_end];
        return (MinedMultiplicity::Set, key.to_string());
    }

    (MinedMultiplicity::One, t.to_string())
}

fn collect_go_interface_marker(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Option<String> {
    let mut depth = 1usize;
    let mut marker = None;

    for (_ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }

        // "isXxx()" method declaration
        let method: String = trimmed.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if method.starts_with("is") && trimmed.contains("()") {
            marker = Some(method);
        }
    }

    marker
}

fn collect_go_block(
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

fn extract_go_facts(
    body: &[(usize, String)],
    _context_line: usize,
    facts: &mut Vec<MinedFactCandidate>,
) {
    for (ln, line) in body {
        let loc = format!("line {}", ln + 1);

        // panic() — precondition
        if line.contains("panic(") && !line.contains("// ") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- panic guard (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "panic".to_string(),
            });
        }

        // len() checks
        if line.contains("len(") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- length check".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "len() check".to_string(),
            });
        }

        // nil checks: "if x == nil" or "if x != nil"
        if line.contains("== nil") || line.contains("!= nil") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- nil check".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "nil check".to_string(),
            });
        }

        // switch statements
        if line.starts_with("switch ") || line == "switch {" {
            facts.push(MinedFactCandidate {
                alloy_text: "-- enum/type switch".to_string(),
                confidence: Confidence::Medium,
                source_location: loc.clone(),
                source_pattern: "switch".to_string(),
            });
        }

        // range — iteration
        if line.contains("for ") && line.contains("range ") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- iteration constraint".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "range iteration".to_string(),
            });
        }

        // errors.New / fmt.Errorf — error propagation
        if line.contains("errors.New(") || line.contains("fmt.Errorf(") {
            facts.push(MinedFactCandidate {
                alloy_text: "-- error creation (review)".to_string(),
                confidence: Confidence::Low,
                source_location: loc.clone(),
                source_pattern: "error creation".to_string(),
            });
        }
    }
}

fn to_camel_case(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => format!("{}{}", c.to_lowercase(), chars.as_str()),
    }
}

/// Reverse-translate a Go expression back to Alloy syntax.
pub fn reverse_translate_expr(code_line: &str) -> Option<String> {
    let s = code_line.trim();
    if s.is_empty() { return None; }

    let s = strip_balanced_parens(s);

    // Tc call: TcField(base) → base.^field
    if let Some(result) = try_reverse_tc_call(s) {
        return Some(result);
    }

    // len(x) → #x
    if let Some(result) = try_reverse_len(s) {
        return Some(result);
    }

    // contains(collection, element) → element in collection
    if let Some(result) = try_reverse_contains(s) {
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
    let rest = s.strip_prefix("Tc")?;
    if rest.is_empty() || !rest.chars().next()?.is_ascii_uppercase() { return None; }
    let paren = rest.find('(')?;
    let field_pascal = &rest[..paren];
    let field = to_camel_case(field_pascal);
    let close = find_matching_close(&rest[paren + 1..])?;
    let args = &rest[paren + 1..paren + 1 + close];
    let base_alloy = reverse_translate_expr(args.trim()).unwrap_or_else(|| args.trim().to_string());
    Some(format!("{base_alloy}.^{field}"))
}

fn try_reverse_len(s: &str) -> Option<String> {
    let inner = s.strip_prefix("len(")?;
    let close = find_matching_close(inner)?;
    let content = inner[..close].trim();
    let content_alloy = reverse_translate_expr(content).unwrap_or_else(|| content.to_string());
    Some(format!("#{content_alloy}"))
}

fn try_reverse_contains(s: &str) -> Option<String> {
    let inner = s.strip_prefix("contains(")?;
    let close = find_matching_close(inner)?;
    let args = &inner[..close];
    let comma = find_top_level_comma(args)?;
    let collection = args[..comma].trim();
    let element = args[comma + 1..].trim();
    let col_alloy = reverse_translate_expr(collection).unwrap_or_else(|| collection.to_string());
    let el_alloy = reverse_translate_expr(element).unwrap_or_else(|| element.to_string());
    Some(format!("{el_alloy} in {col_alloy}"))
}

fn try_reverse_comparison(s: &str) -> Option<String> {
    for (go_op, alloy_op) in &[(" == ", " = "), (" != ", " != "), (" <= ", " <= "),
                                 (" >= ", " >= "), (" < ", " < "), (" > ", " > ")] {
        if let Some(pos) = find_top_level_op(s, go_op) {
            let left = s[..pos].trim();
            let right = s[pos + go_op.len()..].trim();
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
