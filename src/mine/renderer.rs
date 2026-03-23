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
        };
        let comma = if i < sig.fields.len() - 1 { "," } else { "" };
        writeln!(out, "  {}: {mult} {}{comma}", f.name, f.target).unwrap();
    }
    writeln!(out, "}}").unwrap();
}
