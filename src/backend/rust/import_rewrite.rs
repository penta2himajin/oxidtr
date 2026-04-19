/// Rewrites `use super::models::*;` in a generated Rust source string so that
/// it imports types from per-module sources instead of the monolithic
/// `models.rs`.
///
/// When `has_models` is true, the original `use super::models::*;` is kept
/// alongside the per-module imports — `models.rs` still exists for ungrouped
/// types in that case.
pub(super) fn rewrite_models_import(
    content: String,
    modules: &[String],
    has_models: bool,
) -> String {
    let mut imports: Vec<String> = modules
        .iter()
        .map(|m| format!("use super::{m}::*;"))
        .collect();
    if has_models {
        imports.push("use super::models::*;".to_string());
    }
    content.replace("use super::models::*;", &imports.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::rewrite_models_import;

    #[test]
    fn replaces_models_import_with_module_imports() {
        let input = "use super::models::*;\nfn f() {}\n".to_string();
        let modules = vec!["a".to_string(), "b".to_string()];
        let out = rewrite_models_import(input, &modules, false);
        assert_eq!(out, "use super::a::*;\nuse super::b::*;\nfn f() {}\n");
    }

    #[test]
    fn keeps_models_import_when_has_models() {
        let input = "use super::models::*;\n".to_string();
        let modules = vec!["a".to_string()];
        let out = rewrite_models_import(input, &modules, true);
        assert_eq!(out, "use super::a::*;\nuse super::models::*;\n");
    }

    #[test]
    fn no_modules_only_models() {
        let input = "use super::models::*;\n".to_string();
        let out = rewrite_models_import(input, &[], true);
        assert_eq!(out, "use super::models::*;\n");
    }

    #[test]
    fn no_modules_no_models_strips_import() {
        let input = "use super::models::*;\nfn f() {}\n".to_string();
        let out = rewrite_models_import(input, &[], false);
        assert_eq!(out, "\nfn f() {}\n");
    }

    #[test]
    fn leaves_other_content_intact() {
        let input = "fn f() {}\n".to_string();
        let modules = vec!["a".to_string()];
        let out = rewrite_models_import(input.clone(), &modules, false);
        assert_eq!(out, input);
    }
}
