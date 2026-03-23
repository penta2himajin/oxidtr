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
/// For single-lang mode, returns MinedModel directly.
pub fn run(path: &str, lang_override: Option<&str>) -> Result<MinedModel, String> {
    let result = run_merge(path, lang_override)?;
    Ok(result.model)
}

/// Run mine with multi-language merge support.
/// Returns MergeResult with conflicts and source info.
pub fn run_merge(path: &str, lang_override: Option<&str>) -> Result<MergeResult, String> {
    let mut result = run_merge_raw(path, lang_override)?;
    resolve_external_types(&mut result.model);
    Ok(result)
}

fn run_merge_raw(path: &str, lang_override: Option<&str>) -> Result<MergeResult, String> {
    let p = Path::new(path);

    // Single file: detect and extract
    if p.is_file() {
        let lang = match lang_override {
            Some(l) => l.to_string(),
            None => detect_lang_from_file(p)
                .ok_or_else(|| format!("cannot detect language for: {path}"))?,
        };
        let content = std::fs::read_to_string(p)
            .map_err(|e| format!("cannot read {path}: {e}"))?;
        let model = extract_with_lang(&content, &lang)?;
        return Ok(MergeResult {
            model,
            conflicts: vec![],
            sources_used: vec![lang],
        });
    }

    if p.is_dir() {
        return match lang_override {
            // Explicit single-lang mode
            Some(lang) => {
                let model = mine_directory(p, lang)?;
                Ok(MergeResult { model, conflicts: vec![], sources_used: vec![lang.to_string()] })
            }
            // Multi-lang mode: scan all languages and merge
            None => mine_directory_multi_lang(p),
        };
    }

    Err(format!("{path} is neither a file nor a directory"))
}

/// Mine a directory across all detected languages and merge results.
fn mine_directory_multi_lang(dir: &Path) -> Result<MergeResult, String> {
    let lang_files = detect_all_langs(dir);

    if lang_files.is_empty() {
        return Err(format!("no recognized source files in {}", dir.display()));
    }

    let mut per_lang: Vec<(String, MinedModel)> = Vec::new();
    let mut sources_used = Vec::new();

    for (lang, files) in &lang_files {
        let mut lang_model = MinedModel { sigs: Vec::new(), fact_candidates: Vec::new() };
        for file in files {
            let content = std::fs::read_to_string(file)
                .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            let mined = extract_with_lang(&content, lang)?;
            lang_model.sigs.extend(mined.sigs);
            lang_model.fact_candidates.extend(mined.fact_candidates);
        }
        sources_used.push(format!("{lang} ({} files)", files.len()));
        per_lang.push((lang.clone(), lang_model));
    }

    // Merge all language models
    let (merged, conflicts) = merge_models(per_lang);

    Ok(MergeResult { model: merged, conflicts, sources_used })
}

/// Detect all languages present in a directory.
fn detect_all_langs(dir: &Path) -> Vec<(String, Vec<std::path::PathBuf>)> {
    let mut result = Vec::new();

    let rs_files = collect_files(dir, "rs");
    if !rs_files.is_empty() { result.push(("rust".to_string(), rs_files)); }

    let ts_files = collect_files(dir, "ts");
    if !ts_files.is_empty() { result.push(("ts".to_string(), ts_files)); }

    let kt_files = collect_files(dir, "kt");
    if !kt_files.is_empty() { result.push(("kotlin".to_string(), kt_files)); }

    let java_files = collect_files(dir, "java");
    if !java_files.is_empty() { result.push(("java".to_string(), java_files)); }

    // JSON Schema as supplemental
    let schema_path = dir.join("schemas.json");
    if schema_path.exists() {
        result.push(("schema".to_string(), vec![schema_path]));
    } else {
        let json_files = collect_files(dir, "json");
        if !json_files.is_empty() { result.push(("schema".to_string(), json_files)); }
    }

    result
}

/// Merge MinedModels from multiple languages.
/// Same-name sigs are unified; field conflicts are reported.
fn merge_models(per_lang: Vec<(String, MinedModel)>) -> (MinedModel, Vec<MergeConflict>) {
    let mut merged_sigs: Vec<MinedSig> = Vec::new();
    let mut all_facts: Vec<MinedFactCandidate> = Vec::new();
    let mut conflicts: Vec<MergeConflict> = Vec::new();

    // Track which sigs we've seen: name → (sig, source_lang)
    let mut seen: std::collections::HashMap<String, (MinedSig, String)> = std::collections::HashMap::new();

    for (lang, model) in per_lang {
        all_facts.extend(model.fact_candidates);

        for sig in model.sigs {
            if let Some((existing, existing_lang)) = seen.get(&sig.name) {
                // Same sig name from different language — check for conflicts
                let field_conflicts = compare_fields(
                    &sig.name, &existing.fields, &existing_lang,
                    &sig.fields, &lang,
                );
                conflicts.extend(field_conflicts);

                // Merge: supplement missing fields from new source
                let mut merged_sig = existing.clone();
                for f in &sig.fields {
                    if !merged_sig.fields.iter().any(|ef| ef.name == f.name) {
                        merged_sig.fields.push(f.clone());
                    }
                }
                // Prefer abstract if either source says abstract
                if sig.is_abstract {
                    merged_sig.is_abstract = true;
                }
                // Prefer parent if either source provides one
                if merged_sig.parent.is_none() && sig.parent.is_some() {
                    merged_sig.parent = sig.parent.clone();
                }
                // Update source_location to show both
                merged_sig.source_location = format!("{}, {}: {}",
                    merged_sig.source_location, lang, sig.source_location);
                seen.insert(sig.name.clone(), (merged_sig, format!("{existing_lang}+{lang}")));
            } else {
                seen.insert(sig.name.clone(), (sig, lang.clone()));
            }
        }
    }

    // Collect merged sigs in insertion order (approximated by HashMap iteration)
    for (_, (sig, _)) in seen {
        merged_sigs.push(sig);
    }
    merged_sigs.sort_by(|a, b| a.name.cmp(&b.name));

    // Deduplicate fact candidates by alloy_text
    all_facts.sort_by(|a, b| a.alloy_text.cmp(&b.alloy_text));
    all_facts.dedup_by(|a, b| a.alloy_text == b.alloy_text);

    (MinedModel { sigs: merged_sigs, fact_candidates: all_facts }, conflicts)
}

/// Compare fields between two sources of the same sig.
fn compare_fields(
    sig_name: &str,
    fields_a: &[MinedField], lang_a: &str,
    fields_b: &[MinedField], lang_b: &str,
) -> Vec<MergeConflict> {
    let mut conflicts = Vec::new();

    for fa in fields_a {
        if let Some(fb) = fields_b.iter().find(|f| f.name == fa.name) {
            // Same field name exists in both — check multiplicity and target
            if fa.mult != fb.mult {
                conflicts.push(MergeConflict {
                    sig_name: sig_name.to_string(),
                    field_name: fa.name.clone(),
                    sources: vec![
                        format!("{lang_a}: {:?} {}", fa.mult, fa.target),
                        format!("{lang_b}: {:?} {}", fb.mult, fb.target),
                    ],
                    description: "multiplicity mismatch".to_string(),
                });
            } else if fa.target != fb.target {
                conflicts.push(MergeConflict {
                    sig_name: sig_name.to_string(),
                    field_name: fa.name.clone(),
                    sources: vec![
                        format!("{lang_a}: {}", fa.target),
                        format!("{lang_b}: {}", fb.target),
                    ],
                    description: "target type mismatch".to_string(),
                });
            }
        }
    }

    conflicts
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
    collect_files_recursive(dir, ext, &mut files);
    files.sort();
    files
}

fn collect_files_recursive(dir: &Path, ext: &str, files: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursive(&path, ext, files);
            } else if path.is_file() && path.extension().map_or(false, |e| e == ext) {
                files.push(path);
            }
        }
    }
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
    Seq,
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

/// Add placeholder sigs for types referenced in fields but not defined as sigs.
/// This ensures the mined .als output is self-contained valid Alloy.
/// Only adds sigs for valid Alloy identifiers (no qualified paths, generics, or tuples).
pub fn resolve_external_types(model: &mut MinedModel) {
    let defined: std::collections::HashSet<String> = model.sigs.iter()
        .map(|s| s.name.clone())
        .collect();

    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for sig in &model.sigs {
        for field in &sig.fields {
            if !defined.contains(&field.target) && is_valid_sig_name(&field.target) {
                referenced.insert(field.target.clone());
            }
        }
    }

    for name in referenced {
        model.sigs.push(MinedSig {
            name,
            fields: vec![],
            is_abstract: false,
            parent: None,
            source_location: "external type".to_string(),
        });
    }
}

/// A valid Alloy sig name is a simple identifier: alphanumeric + underscore, starts with a letter.
fn is_valid_sig_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().next().map_or(false, |c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// A conflict detected when merging sigs from multiple languages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflict {
    pub sig_name: String,
    pub field_name: String,
    pub sources: Vec<String>,   // e.g. ["rust: Set<User>", "ts: User[]"]
    pub description: String,
}

/// Result of a multi-language mine operation.
#[derive(Debug)]
pub struct MergeResult {
    pub model: MinedModel,
    pub conflicts: Vec<MergeConflict>,
    pub sources_used: Vec<String>, // e.g. ["rust (3 files)", "ts (2 files)", "schema (1 file)"]
}
