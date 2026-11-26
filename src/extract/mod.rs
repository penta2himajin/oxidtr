pub mod rust_extractor;
pub mod ts_extractor;
pub mod kotlin_extractor;
pub mod java_extractor;
pub mod swift_extractor;
pub mod go_extractor;
pub mod csharp_extractor;
pub mod lean_extractor;
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
        // Modular layout: lib.rs at root with module subdirectories
        if path.join("mod.rs").exists() { return Some("rust".to_string()); }
        if path.join("Models.swift").exists() { return Some("swift".to_string()); }
        if path.join("models.go").exists() { return Some("go".to_string()); }
        if path.join("Models.cs").exists() { return Some("csharp".to_string()); }
        if path.join("Types.lean").exists() { return Some("lean".to_string()); }
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
        "swift" => Some("swift".to_string()),
        "go" => Some("go".to_string()),
        "cs" => Some("csharp".to_string()),
        "lean" => Some("lean".to_string()),
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
    let mut all_sources: Vec<String> = Vec::new();

    for (lang, files) in &lang_files {
        let mut lang_model = MinedModel { sigs: Vec::new(), fact_candidates: Vec::new() };
        for file in files {
            let content = std::fs::read_to_string(file)
                .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            let mined = extract_with_lang(&content, lang)?;
            lang_model.sigs.extend(mined.sigs);
            lang_model.fact_candidates.extend(mined.fact_candidates);
            all_sources.push(content);
        }
        sources_used.push(format!("{lang} ({} files)", files.len()));
        per_lang.push((lang.clone(), lang_model));
    }

    // Merge all language models
    let (mut merged, conflicts) = merge_models(per_lang);
    let sources: Vec<&str> = all_sources.iter().map(String::as_str).collect();
    apply_model_markers(&mut merged, &sources);

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

    let swift_files = collect_files(dir, "swift");
    if !swift_files.is_empty() { result.push(("swift".to_string(), swift_files)); }

    let go_files = collect_files(dir, "go");
    if !go_files.is_empty() { result.push(("go".to_string(), go_files)); }

    let cs_files = collect_files(dir, "cs");
    if !cs_files.is_empty() { result.push(("csharp".to_string(), cs_files)); }

    let lean_files = collect_files(dir, "lean");
    if !lean_files.is_empty() { result.push(("lean".to_string(), lean_files)); }

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
                // Prefer var if either source says var
                if sig.is_var {
                    merged_sig.is_var = true;
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
        "swift" => collect_files(dir, "swift"),
        "go" => collect_files(dir, "go"),
        "csharp" | "cs" => collect_files(dir, "cs"),
        "lean" => collect_files(dir, "lean"),
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
    let mut sources: Vec<String> = Vec::new();
    for file in &files_to_mine {
        let content = std::fs::read_to_string(file)
            .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let mined = extract_with_lang(&content, lang)?;
        sources.push(content);

        // Infer module name from subdirectory relative to the base dir.
        // e.g. dir/ast/sig_decl.rs → module "ast"
        // e.g. dir/models.rs → module None
        let inferred_module = infer_module_from_path(file, dir);

        // Dedup: merge same-name sigs (supplement missing fields, prefer more complete)
        for mut sig in mined.sigs {
            // Apply inferred module if not already set
            if sig.module.is_none() {
                sig.module = inferred_module.clone();
            }
            if let Some(existing) = merged.sigs.iter_mut().find(|s| s.name == sig.name) {
                // Supplement missing fields from the new source
                for f in &sig.fields {
                    if !existing.fields.iter().any(|ef| ef.name == f.name) {
                        existing.fields.push(f.clone());
                    }
                }
                // Prefer abstract if either says abstract
                if sig.is_abstract { existing.is_abstract = true; }
                // Prefer var if either says var
                if sig.is_var { existing.is_var = true; }
                // Prefer parent if either provides one
                if existing.parent.is_none() && sig.parent.is_some() {
                    existing.parent = sig.parent.clone();
                }
                // Prefer intersection_of if either provides one
                if existing.intersection_of.is_empty() && !sig.intersection_of.is_empty() {
                    existing.intersection_of = sig.intersection_of.clone();
                }
                // Prefer module if either provides one
                if existing.module.is_none() && sig.module.is_some() {
                    existing.module = sig.module.clone();
                }
            } else {
                merged.sigs.push(sig);
            }
        }
        merged.fact_candidates.extend(mined.fact_candidates);
    }

    // Dedup fact candidates
    merged.fact_candidates.sort_by(|a, b| a.alloy_text.cmp(&b.alloy_text));
    merged.fact_candidates.dedup_by(|a, b| a.alloy_text == b.alloy_text);

    // A file sees only its own sigs and markers; decide again over all of them.
    let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
    apply_model_markers(&mut merged, &sources);

    Ok(merged)
}

/// Infer module name from file path relative to the base directory.
/// e.g. base_dir/ast/sig_decl.rs → Some("ast")
/// e.g. base_dir/models.rs → None
/// e.g. base_dir/src/ir/nodes.rs → Some("ir") (innermost directory)
fn infer_module_from_path(file: &Path, base_dir: &Path) -> Option<String> {
    let relative = file.strip_prefix(base_dir).ok()?;
    let parent = relative.parent()?;
    // Only infer module if the file is in a subdirectory (not at root level)
    if parent.as_os_str().is_empty() {
        return None;
    }
    // Use the first path component as the module name
    // e.g. "ast/sig_decl.rs" → "ast"
    let module_name = parent.components().next()?;
    let name = module_name.as_os_str().to_str()?;
    // Skip common non-module directories
    if name == "src" || name == "Sources" || name == "Tests" || name == "__tests__" {
        // If there's a deeper component, use that instead
        let rest: std::path::PathBuf = parent.components().skip(1).collect();
        if !rest.as_os_str().is_empty() {
            return rest.components().next()
                .and_then(|c| c.as_os_str().to_str())
                .map(|s| s.to_string());
        }
        return None;
    }
    Some(name.to_string())
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
        "swift" => Ok(swift_extractor::extract(content)),
        "go" => Ok(go_extractor::extract(content)),
        "csharp" | "cs" => Ok(csharp_extractor::extract(content)),
        "lean" => Ok(lean_extractor::extract(content)),
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
    pub is_var: bool, // Alloy 6: mutable field
    pub mult: MinedMultiplicity,
    pub target: String,
    /// `Some(V)` for a map field `K -> V`; `target` then holds `K`.
    pub value_type: Option<String>,
    /// Raw union type string preserved from source (e.g. "number | string").
    pub raw_union_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedSig {
    pub name: String,
    pub fields: Vec<MinedField>,
    pub is_abstract: bool,
    pub is_var: bool, // Alloy 6: mutable sig (var sig)
    /// `one` / `lone` / `some`, or `Default` for a plain `sig`.
    pub multiplicity: crate::parser::ast::SigMultiplicity,
    pub parent: Option<String>,
    pub source_location: String,
    /// Components for intersection type aliases (e.g. ["A", "B", "C"] for `type T = A & B & C`).
    pub intersection_of: Vec<String>,
    /// Module grouping inferred from directory structure or package declarations.
    pub module: Option<String>,
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

/// Returns true if the name is a language primitive type that should not be
/// added as a placeholder sig. Covers Rust, TypeScript, Kotlin, Java, Swift, and Go.
pub fn is_language_primitive(name: &str) -> bool {
    matches!(name,
        // Rust
        "String" | "str" | "bool" | "char"
        | "i8" | "i16" | "i32" | "i64" | "i128"
        | "u8" | "u16" | "u32" | "u64" | "u128"
        | "f32" | "f64" | "usize" | "isize"
        // TypeScript
        | "string" | "number" | "boolean" | "any" | "unknown"
        | "never" | "void" | "null" | "undefined" | "object"
        | "bigint"
        // Kotlin / Java boxed
        | "Int" | "Long" | "Short" | "Byte" | "Float" | "Double"
        | "Boolean" | "Char" | "Unit" | "Nothing" | "Any"
        | "Integer" | "Character"
        // Java primitives (lowercase)
        | "int" | "long" | "short" | "byte" | "float" | "double"
        // Swift
        | "Int8" | "Int16" | "Int32" | "Int64"
        | "UInt" | "UInt8" | "UInt16" | "UInt32" | "UInt64"
        | "Bool" | "Void" | "AnyObject" | "Never"
        // C#
        | "decimal" | "nint" | "nuint" | "dynamic"
        // Go
        | "int8" | "int16" | "int32" | "int64"
        | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "rune" | "error"
        | "complex64" | "complex128"
        // Boolean literal types (subtypes of Boolean, shared across all languages)
        | "true" | "false"
        // Go/TS shared (already covered above: any, bool, string, int, byte)
    )
}

/// The Alloy name (`Str` / `Int` / `Float` / `Bool`) of a language primitive,
/// or `None` if `native` is not one.
///
/// The generator's own table (`backend::native_type_for`) is consulted first,
/// so whatever a backend emits reads back as the alias it came from. The
/// synonyms after it are the other widths a hand-written program uses for
/// the same Alloy type (`i32`, `Integer`, `uint32`, ...).
///
/// TypeScript's `number` stands for both `Int` and `Float`; it reads back as
/// `Int`, the alias the backend table lists first.
pub fn alloy_primitive(lang: crate::backend::TargetLang, native: &str) -> Option<&'static str> {
    use crate::backend::TargetLang as L;
    if let Some(alias) = crate::backend::reverse_native_type(lang, native) {
        return Some(alias);
    }
    let alias = match (lang, native) {
        (L::Rust, "i8" | "i16" | "i32" | "i128" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "isize") => "Int",
        (L::Rust, "f32") => "Float",
        (L::Rust, "str") => "Str",
        (L::TypeScript, "bigint") => "Int",
        (L::Kotlin, "Int" | "Short" | "Byte" | "UInt" | "ULong" | "UShort" | "UByte") => "Int",
        (L::Kotlin, "Float") => "Float",
        (L::Java, "int" | "Integer" | "Long" | "short" | "Short" | "byte" | "Byte") => "Int",
        (L::Java, "Double" | "float" | "Float") => "Float",
        (L::Java, "Boolean") => "Bool",
        (L::Swift, "Int8" | "Int16" | "Int32" | "Int64" | "UInt" | "UInt8" | "UInt16" | "UInt32" | "UInt64") => "Int",
        (L::Swift, "Float") => "Float",
        (L::Go, "int" | "int8" | "int16" | "int32" | "uint" | "uint8" | "uint16" | "uint32" | "uint64" | "byte") => "Int",
        (L::Go, "float32") => "Float",
        (L::CSharp, "int" | "short" | "byte" | "sbyte" | "uint" | "ulong" | "ushort" | "Int32" | "Int64") => "Int",
        (L::CSharp, "float" | "decimal" | "Double" | "Single") => "Float",
        (L::CSharp, "String") => "Str",
        (L::CSharp, "Boolean") => "Bool",
        (L::Lean, "Nat") => "Int",
        _ => return None,
    };
    Some(alias)
}

/// Split the arguments of a generic type at its top-level commas:
/// `String, Map<K, V>` → [`String`, `Map<K, V>`].
pub fn split_type_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

/// The target an extractor gives a map field whose key and value are the two
/// type arguments in `args`: `K -> V`, which `split_map_targets` turns into
/// `target` and `value_type`. `None` if `args` are not two types.
pub fn map_target(args: &str) -> Option<String> {
    match split_type_args(args).as_slice() {
        [k, v] => Some(format!("{k} -> {v}")),
        _ => None,
    }
}

/// Move the value type of every `K -> V` target into `value_type`.
/// A pointer key (`*Holder`, Go's map key for a struct) names its sig.
pub fn split_map_targets(model: &mut MinedModel) {
    for sig in &mut model.sigs {
        for field in &mut sig.fields {
            if let Some((k, v)) = field.target.split_once(" -> ") {
                let key = k.trim().trim_start_matches('*').to_string();
                field.value_type = Some(v.trim().to_string());
                field.target = key;
            }
        }
    }
}

/// Undo `backend::escape_keyword` on every field name.
pub fn unescape_keyword_fields(model: &mut MinedModel, is_keyword: fn(&str) -> bool) {
    for sig in &mut model.sigs {
        for field in &mut sig.fields {
            field.name = crate::backend::unescape_keyword(&field.name, is_keyword);
        }
    }
}

/// Rewrite every field type that is a language primitive to its Alloy name,
/// so the mined model names the same types the original model did.
pub fn normalize_primitive_types(model: &mut MinedModel, lang: crate::backend::TargetLang) {
    for sig in &mut model.sigs {
        for field in &mut sig.fields {
            if let Some(alias) = alloy_primitive(lang, &field.target) {
                field.target = alias.to_string();
            }
            if let Some(alias) = field.value_type.as_deref().and_then(|v| alloy_primitive(lang, v)) {
                field.value_type = Some(alias.to_string());
            }
        }
    }
}

/// Recover from the `oxidtr:` markers in `sources` what the generated types
/// cannot say, and set every sig's multiplicity.
///
/// In order:
/// 1. `Derived extends Base`: the parent relation, and the removal of the
///    fields `flatten_concrete_inheritance` repeated from `Base` (#157).
/// 2. `Shape declares label`: the fields an abstract sig's cases repeat move
///    back to it (#157).
/// 3. The multiplicity the code implies — `one` for a case of an abstract sig
///    that carries no data, plain otherwise, the same rule as
///    `backend::natural_sig_multiplicity` (#156).
/// 4. `lone sig Cache`: a multiplicity the code cannot imply (#156).
/// 5. `field Holder.red: one Red`: a field's multiplicity and type where its
///    generated type is wider — a map (#153), a case of an enum (#157).
/// 6. `abstract sig Material`: an abstract sig with no case, which Go writes
///    as a bare `type Material int` (#157).
///
/// Markers name what they are about, so this runs again over every file of
/// a directory once its sigs are merged; every step is idempotent.
pub fn apply_model_markers(model: &mut MinedModel, sources: &[&str]) {
    use crate::parser::ast::SigMultiplicity;
    let markers: Vec<Marker> = sources.iter().flat_map(|s| model_markers(s)).collect();

    for m in &markers {
        if let Marker::Abstract { name } = m {
            match model.sigs.iter_mut().find(|s| &s.name == name) {
                Some(s) => s.is_abstract = true,
                None => model.sigs.push(MinedSig {
                    name: name.clone(),
                    fields: vec![],
                    is_abstract: true,
                    is_var: false,
                    multiplicity: SigMultiplicity::Default,
                    parent: None,
                    source_location: "oxidtr marker".to_string(),
                    intersection_of: vec![],
                    module: None,
                }),
            }
        }
    }

    let declared: std::collections::HashMap<String, Vec<String>> = model.sigs.iter()
        .map(|s| (s.name.clone(), s.fields.iter().map(|f| f.name.clone()).collect()))
        .collect();
    for m in &markers {
        if let Marker::Extends { child, parent } = m {
            let inherited = declared.get(parent).cloned().unwrap_or_default();
            if let Some(s) = model.sigs.iter_mut().find(|s| &s.name == child) {
                s.parent = Some(parent.clone());
                s.fields.retain(|f| !inherited.contains(&f.name));
            }
        }
    }

    for m in &markers {
        if let Marker::Declares { parent, fields } = m {
            for name in fields {
                let mut found: Option<MinedField> = None;
                for s in model.sigs.iter_mut().filter(|s| s.parent.as_deref() == Some(parent.as_str())) {
                    if let Some(i) = s.fields.iter().position(|f| &f.name == name) {
                        let f = s.fields.remove(i);
                        found.get_or_insert(f);
                    }
                }
                let Some(p) = model.sigs.iter_mut().find(|s| &s.name == parent) else { continue };
                if let Some(f) = found {
                    if !p.fields.iter().any(|pf| &pf.name == name) {
                        p.fields.push(f);
                    }
                }
            }
        }
    }

    let natural: Vec<SigMultiplicity> = model.sigs.iter().map(|s| {
        let find = |name: &str| model.sigs.iter().find(|x| x.name == name);
        let Some(parent) = s.parent.as_deref().and_then(find) else {
            return SigMultiplicity::Default;
        };
        if !parent.is_abstract || !s.fields.is_empty() {
            return SigMultiplicity::Default;
        }
        let mut ancestor = Some(parent);
        let mut seen = 0;
        while let Some(a) = ancestor {
            if !a.fields.is_empty() || seen > model.sigs.len() {
                return SigMultiplicity::Default;
            }
            seen += 1;
            ancestor = a.parent.as_deref().and_then(find);
        }
        SigMultiplicity::One
    }).collect();
    for (s, mult) in model.sigs.iter_mut().zip(natural) {
        s.multiplicity = mult;
    }

    for m in markers {
        match m {
            Marker::Sig { name, mult } => {
                if let Some(s) = model.sigs.iter_mut().find(|s| s.name == name) {
                    s.multiplicity = mult;
                }
            }
            Marker::Field { sig, field, mult, target, value } => {
                // The field is declared on `sig`, but a target that copies a
                // parent's fields into each case shows it on the children. A
                // backend that escaped a keyword (`use_`) names it escaped.
                for s in model.sigs.iter_mut()
                    .filter(|s| s.name == sig || s.parent.as_deref() == Some(sig.as_str()))
                {
                    let exact = s.fields.iter().any(|f| f.name == field);
                    let unescaped = field.strip_suffix('_').unwrap_or(&field);
                    for f in s.fields.iter_mut()
                        .filter(|f| f.name == field || (!exact && f.name == unescaped))
                    {
                        f.mult = mult.clone();
                        f.target = target.clone();
                        f.value_type = value.clone();
                    }
                }
            }
            Marker::Abstract { .. } | Marker::Extends { .. } | Marker::Declares { .. } => {}
        }
    }
}

/// A fact about the model that a generated `oxidtr:` comment states.
enum Marker {
    Sig { name: String, mult: crate::parser::ast::SigMultiplicity },
    Field { sig: String, field: String, mult: MinedMultiplicity, target: String, value: Option<String> },
    Extends { child: String, parent: String },
    Declares { parent: String, fields: Vec<String> },
    Abstract { name: String },
}

/// The markers in `source`. Each is an `oxidtr:` comment whose text up to an
/// em dash is one of
/// `<mult> sig <Name>`, `abstract sig <Name>`, `<Child> extends <Parent>`,
/// `<Parent> declares <f>, <g>` or `field <Sig>.<f>: <mult> <T> [-> <V>]`.
fn model_markers(source: &str) -> Vec<Marker> {
    use crate::parser::ast::SigMultiplicity;
    let mut out = Vec::new();
    for line in source.lines() {
        let Some(at) = line.find("oxidtr: ") else { continue };
        let text = &line[at + "oxidtr: ".len()..];
        let text = text.split(" — ").next().unwrap_or(text);
        let words: Vec<&str> = text.split_whitespace().collect();
        match words.as_slice() {
            ["field", path, mult, ty, rest @ ..] => {
                let Some((sig, field)) = path.trim_end_matches(':').split_once('.') else { continue };
                let mult = match *mult {
                    "one" => MinedMultiplicity::One,
                    "lone" => MinedMultiplicity::Lone,
                    "set" => MinedMultiplicity::Set,
                    "seq" => MinedMultiplicity::Seq,
                    _ => continue,
                };
                let value = match rest {
                    ["->", v, ..] => Some(v.to_string()),
                    _ => None,
                };
                out.push(Marker::Field {
                    sig: sig.to_string(), field: field.to_string(), mult,
                    target: ty.to_string(), value,
                });
            }
            ["abstract", "sig", name, ..] if is_valid_sig_name(name) => {
                out.push(Marker::Abstract { name: name.to_string() });
            }
            [child, "extends", parent, ..] if is_valid_sig_name(child) && is_valid_sig_name(parent) => {
                out.push(Marker::Extends { child: child.to_string(), parent: parent.to_string() });
            }
            [parent, "declares", ..] if is_valid_sig_name(parent) => {
                let fields = text.split_once("declares").map(|(_, f)| f).unwrap_or("")
                    .split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
                out.push(Marker::Declares { parent: parent.to_string(), fields });
            }
            _ => {
                let (mult, rest) = match words.as_slice() {
                    ["one", "sig", rest @ ..] => (SigMultiplicity::One, rest),
                    ["lone", "sig", rest @ ..] => (SigMultiplicity::Lone, rest),
                    ["some", "sig", rest @ ..] => (SigMultiplicity::Some, rest),
                    ["sig", rest @ ..] => (SigMultiplicity::Default, rest),
                    _ => continue,
                };
                if let Some(name) = rest.first().filter(|n| is_valid_sig_name(n)) {
                    out.push(Marker::Sig { name: name.to_string(), mult });
                }
            }
        }
    }
    out
}

/// Returns true if the name looks like a generic type parameter.
/// Matches single uppercase letters (T, S, K, V, ...) and all-uppercase 2-char names (IO, ID, ...).
/// Mixed-case 2-char names (Go, Of, Io) are treated as user-defined types.
pub fn is_type_parameter(name: &str) -> bool {
    let len = name.len();
    if len == 1 {
        name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
    } else if len == 2 {
        name.chars().all(|c| c.is_ascii_uppercase())
    } else {
        false
    }
}

/// Extract @temporal annotations from generated test code.
/// Works across all languages by looking for `@temporal` in comment lines.
pub fn extract_temporal_annotations(source: &str, facts: &mut Vec<MinedFactCandidate>) {
    for (i, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        // Strip common comment prefixes: /// // /** * -- #
        let content = trimmed
            .strip_prefix("///")
            .or_else(|| trimmed.strip_prefix("//"))
            .or_else(|| trimmed.strip_prefix("/**"))
            .or_else(|| trimmed.strip_prefix("*"))
            .or_else(|| trimmed.strip_prefix("--"))
            .unwrap_or(trimmed)
            .trim();

        if let Some(rest) = content.strip_prefix("@temporal ") {
            let loc = format!("line {}", i + 1);
            if let Some(name) = rest.strip_prefix("Transition constraint: ") {
                let name = name.trim().trim_end_matches("*/").trim();
                facts.push(MinedFactCandidate {
                    alloy_text: format!("always all s | s.field' = ... -- {name}"),
                    confidence: Confidence::High,
                    source_location: loc,
                    source_pattern: format!("@temporal Transition constraint: {name}"),
                });
            } else if let Some(name) = rest.strip_prefix("Invariant constraint: ") {
                let name = name.trim().trim_end_matches("*/").trim();
                facts.push(MinedFactCandidate {
                    alloy_text: format!("always all s | ... -- {name}"),
                    confidence: Confidence::High,
                    source_location: loc,
                    source_pattern: format!("@temporal Invariant constraint: {name}"),
                });
            }
        }
    }
}

/// Add placeholder sigs for types referenced in fields but not defined as sigs.
/// This ensures the mined .als output is self-contained valid Alloy.
/// Filters out language primitives and generic type parameters.
pub fn resolve_external_types(model: &mut MinedModel) {
    let defined: std::collections::HashSet<String> = model.sigs.iter()
        .map(|s| s.name.clone())
        .collect();

    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for sig in &model.sigs {
        for field in &sig.fields {
            for ty in std::iter::once(&field.target).chain(field.value_type.as_ref()) {
                if !defined.contains(ty)
                    && is_valid_sig_name(ty)
                    && !is_language_primitive(ty)
                    && !crate::backend::is_native_type_alias(ty)
                    && !is_type_parameter(ty)
                {
                    referenced.insert(ty.clone());
                }
            }
        }
    }

    for name in referenced {
        model.sigs.push(MinedSig { multiplicity: crate::parser::ast::SigMultiplicity::Default,
            name,
            fields: vec![],
            is_abstract: false,
            is_var: false,
            parent: None,
            source_location: "external type".to_string(),
            intersection_of: vec![], module: None,
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
