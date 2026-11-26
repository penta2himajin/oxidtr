/// Extracts Alloy model candidates from C# source code.
/// Handles: class → sig, abstract class → abstract sig, enum → abstract sig,
/// T? → lone, List<T> → set, T → one.

use super::*;

pub fn extract(source: &str) -> MinedModel {
    let mut model = extract_raw(source);
    split_map_targets(&mut model);
    apply_model_markers(&mut model, &[source]);
    normalize_primitive_types(&mut model, crate::backend::TargetLang::CSharp);
    model
}

fn extract_raw(source: &str) -> MinedModel {
    let mut sigs = Vec::new();
    let mut fact_candidates = Vec::new();
    let mut lines = source.lines().enumerate().peekable();

    let mut prev_line_has_var_sig = false;

    while let Some((line_num, line)) = lines.next() {
        let trimmed = line.trim();

        if trimmed.contains("@alloy: var sig") || trimmed.contains("Alloy var sig:") {
            prev_line_has_var_sig = true;
            continue;
        }

        let sig_is_var = prev_line_has_var_sig;
        prev_line_has_var_sig = false;

        // public enum Foo { ... }
        if let Some(name) = parse_cs_enum(trimmed) {
            let variants = collect_cs_enum_variants(&mut lines);
            sigs.push(MinedSig { multiplicity: crate::parser::ast::SigMultiplicity::Default,
                name: name.clone(),
                fields: vec![],
                is_abstract: true,
                is_var: sig_is_var,
                parent: None,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![], module: None,
            });
            for v in variants {
                sigs.push(MinedSig { multiplicity: crate::parser::ast::SigMultiplicity::Default,
                    name: v,
                    fields: vec![],
                    is_abstract: false,
                    is_var: false,
                    parent: Some(name.clone()),
                    source_location: format!("line {}", line_num + 1),
                    intersection_of: vec![], module: None,
                });
            }
            continue;
        }

        // public abstract class Foo
        // public class Foo : Bar
        // public class Foo
        if let Some((name, is_abstract, parent)) = parse_cs_class(trimmed) {
            let (fields, has_members) = if trimmed.contains('{') && trimmed.contains('}') {
                // Single-line class: `public class Foo : Bar {}`
                (vec![], false)
            } else {
                collect_cs_class_fields(&mut lines)
            };
            // A class with no property but methods, static members or test
            // attributes holds behaviour, not data — the generator's test
            // class is of that kind (#158).
            if fields.is_empty() && parent.is_none() && has_members {
                continue;
            }
            sigs.push(MinedSig { multiplicity: crate::parser::ast::SigMultiplicity::Default,
                name,
                fields,
                is_abstract,
                is_var: sig_is_var,
                parent,
                source_location: format!("line {}", line_num + 1),
                intersection_of: vec![], module: None,
            });
            continue;
        }
    }

    super::extract_temporal_annotations(source, &mut fact_candidates);

    MinedModel { sigs, fact_candidates }
}

fn parse_cs_enum(line: &str) -> Option<String> {
    let rest = line.strip_prefix("public enum ")?;
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() || !name.chars().next()?.is_ascii_uppercase() { return None; }
    Some(name)
}

fn collect_cs_enum_variants(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> Vec<String> {
    let mut variants = Vec::new();
    let mut depth = 0i32;
    let mut started = false;

    for (_ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => { depth += 1; started = true; } '}' => depth -= 1, _ => {} }
        }
        if started && depth <= 0 { break; }
        if depth > 0 {
            let name: String = trimmed.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                variants.push(name);
            }
        }
    }

    variants
}

fn parse_cs_class(line: &str) -> Option<(String, bool, Option<String>)> {
    let rest = line.strip_prefix("public ")?;

    let (rest, is_abstract) = if let Some(r) = rest.strip_prefix("abstract class ") {
        (r, true)
    } else if let Some(r) = rest.strip_prefix("class ") {
        (r, false)
    } else {
        return None;
    };

    // "Foo : Bar" or "Foo" or "Foo {"
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() || !name.chars().next()?.is_ascii_uppercase() { return None; }

    let after_name = rest[name.len()..].trim();
    let parent = if let Some(after_colon) = after_name.strip_prefix(": ") {
        let parent_name: String = after_colon.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if parent_name.is_empty() { None } else { Some(parent_name) }
    } else {
        None
    };

    Some((name, is_abstract, parent))
}

/// A test attribute, or a method that is `static`. A fieldless class that
/// declares only these holds behaviour, not data — the generator's test class
/// is of that kind (#158). An instance method does not count: a fieldless
/// domain class may still have one. Nor does the singleton's
/// `static readonly X Instance = new X();`, whose `(` follows the `=`.
fn is_behaviour_member(line: &str) -> bool {
    if line.starts_with("[Fact") || line.starts_with("[Test") || line.starts_with("[Theory") {
        return true;
    }
    let before_init = line.split('=').next().unwrap_or(line);
    before_init.contains('(') && before_init.contains("static ")
}

fn collect_cs_class_fields(
    lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
) -> (Vec<MinedField>, bool) {
    let mut fields = Vec::new();
    let mut has_members = false;
    let mut depth = 0i32;
    let mut started = false;
    let mut prev_line_has_var = false;
    let mut prev_line_has_seq = false;

    for (_ln, line) in lines.by_ref() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch { '{' => { depth += 1; started = true; } '}' => depth -= 1, _ => {} }
        }
        if started && depth <= 0 { break; }

        if let Some(mut field) = parse_cs_property(trimmed) {
            if prev_line_has_var {
                field.is_var = true;
            }
            if prev_line_has_seq && field.mult == MinedMultiplicity::Set {
                field.mult = MinedMultiplicity::Seq;
            }
            fields.push(field);
            prev_line_has_var = false;
            prev_line_has_seq = false;
        } else if depth == 1 && !trimmed.starts_with("//") && is_behaviour_member(trimmed) {
            has_members = true;
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

    (fields, has_members)
}

fn parse_cs_property(line: &str) -> Option<MinedField> {
    // "public Type Name { get; set; }"
    let rest = line.strip_prefix("public ")?.strip_prefix("static ")
        .map_or_else(|| line.strip_prefix("public ").unwrap(), |r| r);
    // Skip if it looks like a method (contains parentheses before '{')
    if let Some(brace_pos) = rest.find('{') {
        let before_brace = &rest[..brace_pos];
        if before_brace.contains('(') { return None; }
    }
    // Must contain "{ get;" to be a property
    if !rest.contains("{ get;") && !rest.contains("{get;") { return None; }

    // Parse "Type Name { get; set; }"
    // Type may contain generics: List<Foo>
    let parts = split_type_and_name(rest)?;
    let (type_str, prop_name) = parts;
    let (mult, target) = cs_type_to_mult(type_str);
    let field_name = to_camel_case(&prop_name);

    Some(MinedField { value_type: None, name: field_name, is_var: false, mult, target, raw_union_type: None })
}

fn split_type_and_name(s: &str) -> Option<(&str, String)> {
    // Find the last identifier before "{ get;"
    let brace_pos = s.find('{')?;
    let before = s[..brace_pos].trim();
    // Last whitespace-separated token is the name
    let last_space = before.rfind(' ')?;
    let name = before[last_space + 1..].trim();
    let type_str = before[..last_space].trim();
    if name.is_empty() || type_str.is_empty() { return None; }
    Some((type_str, name.to_string()))
}

fn cs_type_to_mult(cs_type: &str) -> (MinedMultiplicity, String) {
    let t = cs_type.trim();

    // T? → lone
    if let Some(inner) = t.strip_suffix('?') {
        return (MinedMultiplicity::Lone, inner.to_string());
    }

    // List<T> → set
    if let Some(rest) = t.strip_prefix("List<") {
        if let Some(inner) = rest.strip_suffix('>') {
            return (MinedMultiplicity::Set, inner.to_string());
        }
    }

    // IList<T>, ICollection<T>, IEnumerable<T>, HashSet<T> → set
    for prefix in &["IList<", "ICollection<", "IEnumerable<", "HashSet<", "ISet<"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            if let Some(inner) = rest.strip_suffix('>') {
                return (MinedMultiplicity::Set, inner.to_string());
            }
        }
    }

    // Dictionary<K, V> → a map field `set K -> V`
    for prefix in ["Dictionary<", "IDictionary<"] {
        if let Some(target) = t.strip_prefix(prefix).and_then(|r| r.strip_suffix('>')).and_then(map_target) {
            return (MinedMultiplicity::Set, target);
        }
    }

    (MinedMultiplicity::One, t.to_string())
}

fn to_camel_case(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    }
}
