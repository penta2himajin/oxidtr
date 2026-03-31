/// Parses oxidtr-generated Rust files to extract structural information.
/// Uses format-specific lightweight parsing (not syn) since we know the output format.

use crate::backend::{TargetLang, reverse_native_type};
use crate::parser::ast::Multiplicity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedField {
    pub name: String,
    pub mult: Multiplicity,
    pub target: String,
    pub is_var: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedStruct {
    pub name: String,
    pub fields: Vec<ExtractedField>,
    pub is_enum: bool,
    pub is_var: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFn {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedImpl {
    pub structs: Vec<ExtractedStruct>,
    pub fns: Vec<ExtractedFn>,
}

pub fn parse_impl(models_src: &str, ops_src: &str) -> ExtractedImpl {
    let structs = parse_structs(models_src);
    let fns = parse_fns(ops_src);
    ExtractedImpl { structs, fns }
}

fn parse_structs(src: &str) -> Vec<ExtractedStruct> {
    let mut result = Vec::new();
    let mut lines = src.lines().peekable();
    let mut pending_var_sig = false;
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        // Detect @alloy: var sig annotation on comment lines before struct/enum
        if trimmed.contains("@alloy: var sig") {
            pending_var_sig = true;
            continue;
        }
        if let Some(name) = parse_type_decl(trimmed, "pub struct ") {
            // Unit struct (`pub struct Foo;`) has no brace block.
            // Calling collect_fields on a unit struct would set depth=1 and
            // consume all subsequent lines until the next `}` — eating other structs.
            let fields = if trimmed.ends_with(';') {
                vec![]
            } else {
                collect_fields(&mut lines)
            };
            result.push(ExtractedStruct { name, fields, is_enum: false, is_var: pending_var_sig });
            pending_var_sig = false;
            continue;
        }
        if let Some(name) = parse_type_decl(trimmed, "pub enum ") {
            let variant_structs = collect_enum_variants(&mut lines);
            result.push(ExtractedStruct { name, fields: vec![], is_enum: true, is_var: pending_var_sig });
            pending_var_sig = false;
            // Each enum variant corresponds to a child sig in the IR.
            result.extend(variant_structs);
        }
    }
    result
}

fn parse_fns(src: &str) -> Vec<ExtractedFn> {
    let mut result = Vec::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub fn ") {
            if let Some(name) = extract_fn_name(trimmed) {
                result.push(ExtractedFn { name });
            }
        }
    }
    result
}

fn parse_type_decl(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

fn collect_fields(lines: &mut std::iter::Peekable<std::str::Lines<'_>>) -> Vec<ExtractedField> {
    let mut fields = Vec::new();
    let mut depth = 1usize;
    let mut pending_var = false;
    for line in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => depth += 1, '}' => depth -= 1, _ => {} }
        }
        if depth == 0 { break; }
        // Detect @alloy: var annotation on comment lines
        if trimmed.contains("@alloy: var") {
            pending_var = true;
            continue;
        }
        if let Some(mut field) = parse_field_line(trimmed) {
            field.is_var = pending_var;
            fields.push(field);
            pending_var = false;
        }
    }
    fields
}

/// Parse enum variants, producing an ExtractedStruct per variant.
/// Handles both unit variants (`Foo,`) and struct variants (`Foo { field: Type, },`).
fn collect_enum_variants(lines: &mut std::iter::Peekable<std::str::Lines<'_>>) -> Vec<ExtractedStruct> {
    let mut result = Vec::new();
    let mut depth = 1usize; // already inside the enum `{`

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        // Track braces for the outer enum block
        let open = trimmed.chars().filter(|&c| c == '{').count();
        let close = trimmed.chars().filter(|&c| c == '}').count();
        depth = depth + open - close;
        if depth == 0 { break; }

        // Try to detect a variant line at depth 1
        // Unit variant: "VariantName," at depth 1
        // Struct variant start: "VariantName {" at depth 2 (open brace just counted)
        let cleaned = trimmed.trim_end_matches(',');
        if cleaned.is_empty() { continue; }
        let first = cleaned.chars().next().unwrap_or(' ');
        if !first.is_ascii_uppercase() { continue; }

        // Check for struct variant: "VariantName {" or "VariantName {"
        if let Some(brace_pos) = cleaned.find('{') {
            let name: String = cleaned[..brace_pos].trim().to_string();
            if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.is_empty() {
                // Collect fields until we return to the variant's depth
                let fields = collect_variant_fields(lines, &mut depth);
                result.push(ExtractedStruct { name, fields, is_enum: false, is_var: false });
            }
        } else if cleaned.chars().all(|c| c.is_alphanumeric() || c == '_') {
            // Unit variant
            result.push(ExtractedStruct { name: cleaned.to_string(), fields: vec![], is_enum: false, is_var: false });
        }
    }
    result
}

/// Collect fields inside a struct enum variant.
/// We are inside the variant's `{`, so depth has already been incremented.
/// Read lines until depth returns to the level before the variant opened.
fn collect_variant_fields(
    lines: &mut std::iter::Peekable<std::str::Lines<'_>>,
    depth: &mut usize,
) -> Vec<ExtractedField> {
    let target_depth = *depth - 1; // depth when we exit the variant block
    let mut fields = Vec::new();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        let open = trimmed.chars().filter(|&c| c == '{').count();
        let close = trimmed.chars().filter(|&c| c == '}').count();
        *depth = *depth + open - close;

        // Try to parse field lines (no `pub` prefix in enum variant fields)
        if let Some(field) = parse_variant_field_line(trimmed) {
            fields.push(field);
        }

        if *depth <= target_depth { break; }
    }
    fields
}

/// Parse a field line inside an enum variant (no `pub` prefix).
/// Format: "field_name: Type,"
fn parse_variant_field_line(line: &str) -> Option<ExtractedField> {
    let trimmed = line.trim().trim_end_matches(',').trim_end_matches('}').trim();
    let colon = trimmed.find(':')?;
    let name = trimmed[..colon].trim().to_string();
    if name.is_empty() || !name.chars().next()?.is_ascii_lowercase() { return None; }
    let type_str = trimmed[colon + 1..].trim().to_string();
    if type_str.is_empty() { return None; }
    let (mult, target) = type_to_mult(&type_str);
    Some(ExtractedField { name, mult, target, is_var: false })
}

fn parse_field_line(line: &str) -> Option<ExtractedField> {
    let rest = line.strip_prefix("pub ")?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim().to_string();
    if name.is_empty() { return None; }
    let type_str = rest[colon + 1..].trim().trim_end_matches(',').trim().to_string();
    if type_str.is_empty() { return None; }
    let (mult, target) = type_to_mult(&type_str);
    Some(ExtractedField { name, mult, target, is_var: false })
}

fn extract_fn_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("pub fn ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Reverse-maps a Rust type string to Multiplicity.
/// e.g. "Option<Foo>" -> Lone, "Vec<Foo>" -> Set, "Foo" -> One
/// Also reverse-maps native types: "String" -> "Str", "i64" -> "Int", etc.
pub fn type_to_mult(rust_type: &str) -> (Multiplicity, String) {
    let t = rust_type.trim();
    if let Some(inner) = strip_wrapper(t, "BTreeSet<", ">") {
        return (Multiplicity::Set, reverse_rust_native(inner));
    }
    if let Some(inner) = strip_wrapper(t, "Vec<", ">") {
        return (Multiplicity::Seq, reverse_rust_native(inner));
    }
    if let Some(inner) = strip_wrapper(t, "Option<Box<", ">>") {
        return (Multiplicity::Lone, reverse_rust_native(inner));
    }
    if let Some(inner) = strip_wrapper(t, "Option<", ">") {
        return (Multiplicity::Lone, reverse_rust_native(inner));
    }
    if let Some(inner) = strip_wrapper(t, "Box<", ">") {
        return (Multiplicity::One, reverse_rust_native(inner));
    }
    (Multiplicity::One, reverse_rust_native(t))
}

/// Reverse-map a Rust native type back to the Alloy alias if applicable.
fn reverse_rust_native(name: &str) -> String {
    reverse_native_type(TargetLang::Rust, name)
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

fn strip_wrapper<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)?.strip_suffix(suffix)
}
