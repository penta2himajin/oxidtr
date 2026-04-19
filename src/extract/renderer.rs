/// Renders a MinedModel as Alloy source text (.als format).

use super::*;
use std::fmt::Write;
use std::path::PathBuf;

/// A single file emitted by the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedFile {
    pub path: PathBuf,
    pub content: String,
}

/// Legacy single-file renderer. Produces a concatenated string with
/// mid-file `module X` grouping markers — this is **not** Alloy-spec-compliant
/// but is retained for backward compatibility with existing callers/tests.
///
/// For spec-compliant multi-file output, use `render_files`.
pub fn render(model: &MinedModel) -> String {
    let mut out = String::new();

    let mut ungrouped: Vec<&MinedSig> = Vec::new();
    let mut module_order: Vec<String> = Vec::new();
    let mut by_module: std::collections::HashMap<String, Vec<&MinedSig>> = std::collections::HashMap::new();

    for sig in &model.sigs {
        if let Some(m) = &sig.module {
            by_module.entry(m.clone()).or_default().push(sig);
            if !module_order.contains(m) {
                module_order.push(m.clone());
            }
        } else {
            ungrouped.push(sig);
        }
    }

    for sig in &ungrouped {
        render_sig(&mut out, sig);
        writeln!(out).unwrap();
    }

    for module_name in &module_order {
        writeln!(out, "module {module_name}").unwrap();
        writeln!(out).unwrap();
        if let Some(sigs) = by_module.get(module_name) {
            for sig in sigs {
                render_sig(&mut out, sig);
                writeln!(out).unwrap();
            }
        }
    }

    render_fact_candidates(&mut out, &model.fact_candidates);

    out
}

/// Alloy-spec-compliant multi-file renderer.
///
/// - If no sig declares a `module`, returns a single `main.als` file with
///   identical content to the legacy `render()` output.
/// - If sigs are partitioned across modules, emits one file per module
///   (`oxidtr/ast` → `oxidtr/ast.als`) plus a main file with `open` directives.
/// - Cross-module field references produce `open` directives in the dependent
///   module file automatically.
pub fn render_files(model: &MinedModel) -> Vec<RenderedFile> {
    // Partition sigs into (module → sigs) preserving insertion order.
    let mut module_order: Vec<String> = Vec::new();
    let mut by_module: std::collections::BTreeMap<String, Vec<&MinedSig>> =
        std::collections::BTreeMap::new();
    let mut ungrouped: Vec<&MinedSig> = Vec::new();
    for sig in &model.sigs {
        match &sig.module {
            Some(m) => {
                by_module.entry(m.clone()).or_default().push(sig);
                if !module_order.contains(m) {
                    module_order.push(m.clone());
                }
            }
            None => ungrouped.push(sig),
        }
    }

    // No modules → single-file legacy-compat output.
    if module_order.is_empty() {
        let mut out = String::new();
        for sig in &ungrouped {
            render_sig(&mut out, sig);
            writeln!(out).unwrap();
        }
        render_fact_candidates(&mut out, &model.fact_candidates);
        return vec![RenderedFile {
            path: PathBuf::from("main.als"),
            content: out,
        }];
    }

    // Index sig → owning module for cross-reference analysis.
    let sig_owner: std::collections::HashMap<String, String> = model
        .sigs
        .iter()
        .filter_map(|s| s.module.as_ref().map(|m| (s.name.clone(), m.clone())))
        .collect();

    let mut files: Vec<RenderedFile> = Vec::new();

    // Main file — top-level sigs (module = None) + `open` for every sub-module.
    let main_name = "oxidtr";
    let mut main_content = String::new();
    for m in &module_order {
        writeln!(main_content, "open {m}").unwrap();
    }
    if !module_order.is_empty() && (!ungrouped.is_empty() || !model.fact_candidates.is_empty()) {
        writeln!(main_content).unwrap();
    }
    for sig in &ungrouped {
        render_sig(&mut main_content, sig);
        writeln!(main_content).unwrap();
    }
    render_fact_candidates(&mut main_content, &model.fact_candidates);
    files.push(RenderedFile {
        path: PathBuf::from(format!("{main_name}.als")),
        content: main_content,
    });

    // One file per module.
    for m in &module_order {
        let sigs = by_module.get(m).cloned().unwrap_or_default();
        let mut deps: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for sig in &sigs {
            if let Some(parent) = &sig.parent {
                if let Some(parent_mod) = sig_owner.get(parent) {
                    if parent_mod != m {
                        deps.insert(parent_mod.clone());
                    }
                }
            }
            for f in &sig.fields {
                if let Some(target_mod) = sig_owner.get(&f.target) {
                    if target_mod != m {
                        deps.insert(target_mod.clone());
                    }
                }
            }
            for comp in &sig.intersection_of {
                if let Some(comp_mod) = sig_owner.get(comp) {
                    if comp_mod != m {
                        deps.insert(comp_mod.clone());
                    }
                }
            }
        }

        let mut content = String::new();
        writeln!(content, "module {m}").unwrap();
        if !deps.is_empty() {
            writeln!(content).unwrap();
            for dep in &deps {
                writeln!(content, "open {dep}").unwrap();
            }
        }
        writeln!(content).unwrap();
        for sig in &sigs {
            render_sig(&mut content, sig);
            writeln!(content).unwrap();
        }

        // `module foo/bar` → `foo/bar.als`
        let mut path = PathBuf::new();
        for segment in m.split('/') {
            path.push(segment);
        }
        path.set_extension("als");
        files.push(RenderedFile { path, content });
    }

    files
}

fn render_fact_candidates(out: &mut String, facts: &[MinedFactCandidate]) {
    if facts.is_empty() {
        return;
    }
    writeln!(out, "-- Fact candidates (review required)").unwrap();
    for fact in facts {
        let conf = match fact.confidence {
            Confidence::High => "HIGH",
            Confidence::Medium => "MEDIUM",
            Confidence::Low => "LOW",
        };
        writeln!(out, "-- [{conf}] from: {} ({})", fact.source_pattern, fact.source_location).unwrap();
        writeln!(out, "-- fact {{ {} }}", fact.alloy_text).unwrap();
        writeln!(out).unwrap();
    }
}

fn render_sig(out: &mut String, sig: &MinedSig) {
    // Intersection type alias: render as a comment + empty sig
    if !sig.intersection_of.is_empty() {
        let components = sig.intersection_of.join(" & ");
        writeln!(out, "-- intersection: {} = {}", sig.name, components).unwrap();
        writeln!(out, "sig {} {{", sig.name).unwrap();
        writeln!(out, "}}").unwrap();
        return;
    }

    if sig.is_abstract {
        write!(out, "abstract ").unwrap();
    }
    if sig.is_var {
        write!(out, "var ").unwrap();
    }
    write!(out, "sig {}", sig.name).unwrap();
    if let Some(parent) = &sig.parent {
        write!(out, " extends {parent}").unwrap();
    }
    writeln!(out, " {{").unwrap();
    for (i, f) in sig.fields.iter().enumerate() {
        let mult = match f.mult {
            MinedMultiplicity::One => "one",
            MinedMultiplicity::Lone => "lone",
            MinedMultiplicity::Set => "set",
            MinedMultiplicity::Seq => "seq",
        };
        let comma = if i < sig.fields.len() - 1 { "," } else { "" };
        // Use raw_union_type as a comment annotation; target holds the first variant
        // for Alloy compatibility (Alloy cannot express field-level union types)
        let var_prefix = if f.is_var { "var " } else { "" };
        if let Some(raw) = &f.raw_union_type {
            let first_type = raw.split(" | ").next().unwrap_or(&f.target).trim();
            writeln!(out, "  {var_prefix}{}: {mult} {}{comma} -- union: {raw}", f.name, first_type).unwrap();
        } else {
            writeln!(out, "  {var_prefix}{}: {mult} {}{comma}", f.name, f.target).unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
}
