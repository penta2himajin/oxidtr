/// Renders a MinedModel as Alloy source text (.als format).

use super::*;
use std::fmt::Write;

pub fn render(model: &MinedModel) -> String {
    let mut out = String::new();

    // Render sigs
    for sig in &model.sigs {
        render_sig(&mut out, sig);
        writeln!(out).unwrap();
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
