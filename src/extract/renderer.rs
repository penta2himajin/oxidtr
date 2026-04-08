/// Renders a MinedModel as Alloy source text (.als format).

use super::*;
use std::fmt::Write;

pub fn render(model: &MinedModel) -> String {
    let mut out = String::new();

    // Group sigs by module, preserving order within each group.
    // Sigs with no module come first (ungrouped), then each module section.
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

    // Render ungrouped sigs first
    for sig in &ungrouped {
        render_sig(&mut out, sig);
        writeln!(out).unwrap();
    }

    // Render each module section
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

    // Render fact candidates (as comments with confidence)
    if !model.fact_candidates.is_empty() {
        writeln!(out, "-- Fact candidates (review required)").unwrap();
        for fact in &model.fact_candidates {
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

    out
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
