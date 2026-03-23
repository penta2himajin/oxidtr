pub mod rust_extractor;
pub mod ts_extractor;
pub mod renderer;

/// Confidence level of a mined element, determined mechanically by pattern type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confidence {
    High,   // Direct structural mapping (struct→sig, enum→abstract sig)
    Medium, // Conditional patterns (.contains, .is_empty, etc.)
    Low,    // Ambiguous patterns (if-return-Err, general comparisons)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MinedMultiplicity {
    One,
    Lone,
    Set,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedField {
    pub name: String,
    pub mult: MinedMultiplicity,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedSig {
    pub name: String,
    pub fields: Vec<MinedField>,
    pub is_abstract: bool,
    pub parent: Option<String>,
    pub source_location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedFactCandidate {
    pub alloy_text: String,
    pub confidence: Confidence,
    pub source_location: String,
    pub source_pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedModel {
    pub sigs: Vec<MinedSig>,
    pub fact_candidates: Vec<MinedFactCandidate>,
}
