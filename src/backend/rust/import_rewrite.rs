//! Import rewriter for the Rust backend's modular codegen layout.
//!
//! The non-modular generators (`generate_helpers`, `generate_tests`,
//! `generate_fixtures`, `generate_newtypes`) always emit `use super::models::*;`
//! at the top of each produced file. When codegen runs in modular mode,
//! sigs tagged with an inline `module X` directive live in `X/*.rs` instead
//! of `models.rs`, so those imports must be rewritten.
//!
//! A previous implementation replaced the `models` import wholesale with
//! submodule imports, which worked only if *every* sig was module-tagged —
//! as soon as a single ungrouped sig existed, `models.rs` was still
//! produced but no longer imported, causing downstream compile failures.

/// Rewrite the `use super::models::*;` import so modular output correctly
/// imports both the root `models` module (when any ungrouped sig exists)
/// and every submodule.
pub fn rewrite_models_import(
    content: String,
    modules: &[String],
    has_ungrouped: bool,
) -> String {
    let module_imports = modules
        .iter()
        .map(|m| format!("use super::{m}::*;"))
        .collect::<Vec<_>>()
        .join("\n");
    let replacement = match (has_ungrouped, modules.is_empty()) {
        // Only module-tagged sigs → drop models import, use submodule imports.
        (false, false) => module_imports,
        // Only ungrouped sigs (no submodules) → keep models import as-is.
        (true, true) => "use super::models::*;".to_string(),
        // Mixed → keep models import AND add per-module imports.
        (true, false) => format!("use super::models::*;\n{module_imports}"),
        // Degenerate: no sigs at all. Leave whatever was there.
        (false, true) => "use super::models::*;".to_string(),
    };
    content.replace("use super::models::*;", &replacement)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: &str = "#[allow(unused_imports)]\nuse super::models::*;\n\nfn x() {}\n";

    #[test]
    fn mixed_keeps_models_and_adds_submodule() {
        let out = rewrite_models_import(INPUT.to_string(), &["sub".to_string()], true);
        assert!(out.contains("use super::models::*;"), "got:\n{out}");
        assert!(out.contains("use super::sub::*;"), "got:\n{out}");
    }

    #[test]
    fn module_only_drops_models_import() {
        let out = rewrite_models_import(INPUT.to_string(), &["sub".to_string()], false);
        assert!(!out.contains("use super::models::*;"), "got:\n{out}");
        assert!(out.contains("use super::sub::*;"), "got:\n{out}");
    }

    #[test]
    fn ungrouped_only_keeps_models_unchanged() {
        let out = rewrite_models_import(INPUT.to_string(), &[], true);
        assert!(out.contains("use super::models::*;"), "got:\n{out}");
        assert!(!out.contains("use super::sub::*;"), "got:\n{out}");
    }
}
