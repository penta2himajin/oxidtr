pub mod rust_extractor;
pub mod ts_extractor;
pub mod kotlin_extractor;
pub mod java_extractor;
pub mod schema_extractor;
pub mod renderer;

use std::path::Path;

/// Auto-detect language from file extension or directory contents.
/// Returns None if detection fails.
pub fn detect_lang(path: &Path) -> Option<String> {
    if path.is_file() {
        return detect_lang_from_file(path);
    }
    if path.is_dir() {
        // Priority order matches check command: ts > kt > java > rs > schema
        if path.join("models.ts").exists() { return Some("ts".to_string()); }
        if path.join("Models.kt").exists() { return Some("kt".to_string()); }
        if path.join("Models.java").exists() { return Some("java".to_string()); }
        if path.join("models.rs").exists() { return Some("rust".to_string()); }
        if path.join("schemas.json").exists() { return Some("schema".to_string()); }

        // Scan for any matching extension
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Some(lang) = detect_lang_from_file(&entry.path()) {
                    return Some(lang);
                }
            }
        }
    }
    None
}

fn detect_lang_from_file(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "rs" => Some("rust".to_string()),
        "ts" => Some("ts".to_string()),
        "kt" => Some("kotlin".to_string()),
        "java" => Some("java".to_string()),
        "json" => Some("schema".to_string()),
        _ => None,
    }
}

/// Run mine on a path (file or directory), auto-detecting language.
/// If `lang_override` is Some, uses that instead of auto-detection.
pub fn run(path: &str, lang_override: Option<&str>) -> Result<MinedModel, String> {
    let p = Path::new(path);

    let lang = match lang_override {
        Some(l) => l.to_string(),
        None => detect_lang(p).ok_or_else(|| format!("cannot detect language for: {path}"))?,
    };

    if p.is_file() {
        let content = std::fs::read_to_string(p)
            .map_err(|e| format!("cannot read {path}: {e}"))?;
        return Ok(extract_with_lang(&content, &lang)?);
    }

    if p.is_dir() {
        return mine_directory(p, &lang);
    }

    Err(format!("{path} is neither a file nor a directory"))
}

fn mine_directory(dir: &Path, lang: &str) -> Result<MinedModel, String> {
    // Find the relevant files based on language
    let files_to_mine: Vec<std::path::PathBuf> = match lang {
        "rust" | "rs" => collect_files(dir, "rs"),
        "typescript" | "ts" => collect_files(dir, "ts"),
        "kotlin" | "kt" => collect_files(dir, "kt"),
        "java" => collect_files(dir, "java"),
        "schema" | "json" => {
            let schema_path = dir.join("schemas.json");
            if schema_path.exists() { vec![schema_path] } else { collect_files(dir, "json") }
        }
        _ => return Err(format!("unsupported language: {lang}")),
    };

    if files_to_mine.is_empty() {
        return Err(format!("no {lang} files found in {}", dir.display()));
    }

    // Mine each file and merge results
    let mut merged = MinedModel { sigs: Vec::new(), fact_candidates: Vec::new() };
    for file in &files_to_mine {
        let content = std::fs::read_to_string(file)
            .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let mined = extract_with_lang(&content, lang)?;
        merged.sigs.extend(mined.sigs);
        merged.fact_candidates.extend(mined.fact_candidates);
    }

    Ok(merged)
}

fn collect_files(dir: &Path, ext: &str) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == ext) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_with_lang(content: &str, lang: &str) -> Result<MinedModel, String> {
    match lang {
        "rust" | "rs" => Ok(rust_extractor::extract(content)),
        "typescript" | "ts" => Ok(ts_extractor::extract(content)),
        "kotlin" | "kt" => Ok(kotlin_extractor::extract(content)),
        "java" => Ok(java_extractor::extract(content)),
        "schema" | "json" => Ok(schema_extractor::extract(content)),
        other => Err(format!("unsupported language: {other}")),
    }
}

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
