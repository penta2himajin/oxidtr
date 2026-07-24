//! Shared identifier-naming helpers used by both the Rust codegen backend and
//! `check`'s structural differ, so the two stay in sync.

pub fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

/// Alloy predicates in operational style are often verb-named (`addField`,
/// `setIRParent`) even though the generated function is a pure boolean check
/// (immutable refs, `-> bool`) — Alloy predicates have no side effects. The
/// verb form measurably biases small code-completion models into writing
/// code that tries to mutate through the immutable reference. Map known verb
/// prefixes to a check-shaped name instead; anything else falls back to
/// plain snake_case.
pub fn fn_name_for_op(op_name: &str) -> String {
    const VERB_TEMPLATES: &[(&str, &str)] = &[
        ("add", "is_present"),
        ("insert", "is_present"),
        ("remove", "is_absent"),
        ("delete", "is_absent"),
        ("set", "matches"),
    ];
    for (verb, suffix) in VERB_TEMPLATES {
        if let Some(rest) = strip_verb_prefix(op_name, verb) {
            return format!("{}_{}", to_snake_case(&rest), suffix);
        }
    }
    to_snake_case(op_name)
}

/// Strips `verb` from the front of `s` only at a camelCase word boundary
/// (verb followed by an uppercase letter), so e.g. "addField" matches "add"
/// but "address" or "adder" do not.
fn strip_verb_prefix(s: &str, verb: &str) -> Option<String> {
    if s.len() <= verb.len() {
        return None;
    }
    if !s.as_bytes()[..verb.len()].eq_ignore_ascii_case(verb.as_bytes()) {
        return None;
    }
    let rest = &s[verb.len()..];
    let next = rest.chars().next()?;
    if next.is_uppercase() {
        Some(rest.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verb_templates_apply_at_word_boundary() {
        assert_eq!(fn_name_for_op("addField"), "field_is_present");
        assert_eq!(fn_name_for_op("addStructure"), "structure_is_present");
        assert_eq!(fn_name_for_op("addConstraint"), "constraint_is_present");
        assert_eq!(fn_name_for_op("setIRParent"), "i_r_parent_matches");
        assert_eq!(fn_name_for_op("removeField"), "field_is_absent");
    }

    #[test]
    fn non_verb_names_fall_back_to_snake_case() {
        assert_eq!(fn_name_for_op("lowerOneSig"), "lower_one_sig");
        assert_eq!(fn_name_for_op("evalExpr"), "eval_expr");
    }

    #[test]
    fn verb_like_substrings_without_a_word_boundary_are_not_stripped() {
        // "address" starts with "add" but is not followed by an uppercase
        // letter, so it must not be treated as the "add" verb.
        assert_eq!(fn_name_for_op("addressBook"), "address_book");
        assert_eq!(fn_name_for_op("setup"), "setup");
    }
}
