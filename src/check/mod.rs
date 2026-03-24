// check module — diff Alloy IR vs implementation (Rust or TypeScript)
pub mod impl_parser;
pub mod differ;

use crate::parser;
use crate::ir;
use crate::extract;
use differ::DiffItem;
use impl_parser::{ExtractedImpl, ExtractedStruct, ExtractedFn, ExtractedField};

use std::path::Path;

#[derive(Debug)]
pub enum CheckError {
    IoError(std::io::Error),
    ParseError(parser::ParseError),
    LoweringError(ir::LoweringError),
    ImplNotFound(String),
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckError::IoError(e) => write!(f, "IO error: {e}"),
            CheckError::ParseError(e) => write!(f, "parse error: {e}"),
            CheckError::LoweringError(e) => write!(f, "lowering error: {e}"),
            CheckError::ImplNotFound(s) => write!(f, "impl file not found: {s}"),
        }
    }
}

impl std::error::Error for CheckError {}

impl From<std::io::Error> for CheckError {
    fn from(e: std::io::Error) -> Self { CheckError::IoError(e) }
}
impl From<parser::ParseError> for CheckError {
    fn from(e: parser::ParseError) -> Self { CheckError::ParseError(e) }
}
impl From<ir::LoweringError> for CheckError {
    fn from(e: ir::LoweringError) -> Self { CheckError::LoweringError(e) }
}

#[derive(Debug)]
pub struct CheckResult {
    pub diffs: Vec<DiffItem>,
}

impl CheckResult {
    pub fn is_ok(&self) -> bool {
        self.diffs.is_empty()
    }
}

pub struct CheckConfig {
    pub impl_dir: String,
}

pub fn run(model_path: &str, config: &CheckConfig) -> Result<CheckResult, CheckError> {
    let source = std::fs::read_to_string(model_path)?;
    let ast = parser::parse(&source)?;
    let ir = ir::lower(&ast)?;

    let impl_dir = Path::new(&config.impl_dir);

    // Auto-detect language by file presence
    let diffs = if impl_dir.join("models.ts").exists() {
        let extracted = extract_mined(impl_dir, "models.ts", "operations.ts", extract::ts_extractor::extract)?;
        let validation_sources = collect_validation_sources_ts(impl_dir)?;
        differ::diff_identity_with_validation(&ir, &extracted, &validation_sources)
    } else if impl_dir.join("Models.kt").exists() {
        let extracted = extract_mined(impl_dir, "Models.kt", "Operations.kt", extract::kotlin_extractor::extract)?;
        let validation_sources = collect_validation_sources_jvm(impl_dir, "Tests.kt")?;
        differ::diff_identity_with_validation(&ir, &extracted, &validation_sources)
    } else if impl_dir.join("Models.java").exists() {
        let extracted = extract_mined(impl_dir, "Models.java", "Operations.java", extract::java_extractor::extract)?;
        let validation_sources = collect_validation_sources_jvm(impl_dir, "Tests.java")?;
        differ::diff_identity_with_validation(&ir, &extracted, &validation_sources)
    } else if impl_dir.join("Models.swift").exists() {
        let extracted = extract_mined(impl_dir, "Models.swift", "Operations.swift", extract::swift_extractor::extract)?;
        let validation_sources = collect_validation_sources_swift(impl_dir)?;
        differ::diff_identity_with_validation(&ir, &extracted, &validation_sources)
    } else if impl_dir.join("models.go").exists() {
        let extracted = extract_mined(impl_dir, "models.go", "operations.go", extract::go_extractor::extract)?;
        let validation_sources = collect_validation_sources_go(impl_dir)?;
        differ::diff_go_with_validation(&ir, &extracted, &validation_sources)
    } else if impl_dir.join("models.rs").exists() {
        let (extracted, validation_sources) = extract_rust(impl_dir)?;
        differ::diff_with_validation(&ir, &extracted, &validation_sources)
    } else {
        // Fallback: use mine to extract from any code in the directory
        match extract::run(config.impl_dir.as_str(), None) {
            Ok(mined) => {
                let extracted = mined_to_extracted(&mined);
                // Collect all source files as validation sources
                let validation_sources = collect_all_sources(impl_dir)?;
                differ::diff_identity_with_validation(&ir, &extracted, &validation_sources)
            }
            Err(_) => {
                return Err(CheckError::ImplNotFound(
                    "no recognized source files found (tried models.rs, models.ts, Models.kt, Models.java, Models.swift, and general extract)".to_string()
                ));
            }
        }
    };

    Ok(CheckResult { diffs })
}

fn extract_rust(impl_dir: &Path) -> Result<(ExtractedImpl, Vec<String>), CheckError> {
    let models_src = std::fs::read_to_string(impl_dir.join("models.rs"))?;
    let ops_path = impl_dir.join("operations.rs");
    let ops_src = if ops_path.exists() {
        std::fs::read_to_string(&ops_path)?
    } else {
        String::new()
    };

    // Collect validation sources (tests, newtypes) for validation coverage check
    let mut validation_sources = Vec::new();
    let tests_path = impl_dir.join("tests.rs");
    if tests_path.exists() {
        validation_sources.push(std::fs::read_to_string(&tests_path)?);
    }
    let newtypes_path = impl_dir.join("newtypes.rs");
    if newtypes_path.exists() {
        validation_sources.push(std::fs::read_to_string(&newtypes_path)?);
    }

    Ok((impl_parser::parse_impl(&models_src, &ops_src), validation_sources))
}

/// Collect validation source texts from TypeScript impl directory.
fn collect_validation_sources_ts(impl_dir: &Path) -> Result<Vec<String>, CheckError> {
    let mut sources = Vec::new();
    let tests_path = impl_dir.join("tests.ts");
    if tests_path.exists() {
        sources.push(std::fs::read_to_string(&tests_path)?);
    }
    let validators_path = impl_dir.join("validators.ts");
    if validators_path.exists() {
        sources.push(std::fs::read_to_string(&validators_path)?);
    }
    Ok(sources)
}

/// Collect validation source texts from JVM (Kotlin/Java) impl directory.
fn collect_validation_sources_jvm(impl_dir: &Path, tests_file: &str) -> Result<Vec<String>, CheckError> {
    let mut sources = Vec::new();
    let tests_path = impl_dir.join(tests_file);
    if tests_path.exists() {
        sources.push(std::fs::read_to_string(&tests_path)?);
    }
    // Java: also check Models.java for compact constructor assertions
    let models_path = impl_dir.join("Models.java");
    if models_path.exists() {
        sources.push(std::fs::read_to_string(&models_path)?);
    }
    let models_kt_path = impl_dir.join("Models.kt");
    if models_kt_path.exists() {
        sources.push(std::fs::read_to_string(&models_kt_path)?);
    }
    Ok(sources)
}

/// Collect validation source texts from Swift impl directory.
fn collect_validation_sources_swift(impl_dir: &Path) -> Result<Vec<String>, CheckError> {
    let mut sources = Vec::new();
    let tests_path = impl_dir.join("Tests.swift");
    if tests_path.exists() {
        sources.push(std::fs::read_to_string(&tests_path)?);
    }
    Ok(sources)
}

/// Collect validation source texts from Go impl directory.
fn collect_validation_sources_go(impl_dir: &Path) -> Result<Vec<String>, CheckError> {
    let mut sources = Vec::new();
    let tests_path = impl_dir.join("models_test.go");
    if tests_path.exists() {
        sources.push(std::fs::read_to_string(&tests_path)?);
    }
    Ok(sources)
}

/// Extract using an extractor function, then convert MinedModel → ExtractedImpl.
fn extract_mined<F>(
    impl_dir: &Path,
    models_file: &str,
    ops_file: &str,
    extractor: F,
) -> Result<ExtractedImpl, CheckError>
where F: Fn(&str) -> extract::MinedModel {
    let models_src = std::fs::read_to_string(impl_dir.join(models_file))?;
    let ops_path = impl_dir.join(ops_file);
    let ops_src = if ops_path.exists() {
        std::fs::read_to_string(&ops_path)?
    } else {
        String::new()
    };

    let mined = extractor(&models_src);

    let structs = mined.sigs.iter().map(|s| {
        ExtractedStruct {
            name: s.name.clone(),
            fields: s.fields.iter().map(|f| {
                let mult = match f.mult {
                    extract::MinedMultiplicity::One => crate::parser::ast::Multiplicity::One,
                    extract::MinedMultiplicity::Lone => crate::parser::ast::Multiplicity::Lone,
                    extract::MinedMultiplicity::Set => crate::parser::ast::Multiplicity::Set,
                    extract::MinedMultiplicity::Seq => crate::parser::ast::Multiplicity::Seq,
                };
                ExtractedField {
                    name: f.name.clone(),
                    mult,
                    target: f.target.clone(),
                }
            }).collect(),
            is_enum: s.is_abstract,
        }
    }).collect();

    let fns = extract_fns_generic(&ops_src);

    Ok(ExtractedImpl { structs, fns })
}

/// Extract function names from operation files (language-agnostic patterns).
/// Convert a MinedModel to ExtractedImpl for comparison with IR.
fn mined_to_extracted(mined: &extract::MinedModel) -> ExtractedImpl {
    let structs = mined.sigs.iter().map(|s| {
        ExtractedStruct {
            name: s.name.clone(),
            fields: s.fields.iter().map(|f| {
                let mult = match f.mult {
                    extract::MinedMultiplicity::One => crate::parser::ast::Multiplicity::One,
                    extract::MinedMultiplicity::Lone => crate::parser::ast::Multiplicity::Lone,
                    extract::MinedMultiplicity::Set => crate::parser::ast::Multiplicity::Set,
                    extract::MinedMultiplicity::Seq => crate::parser::ast::Multiplicity::Seq,
                };
                ExtractedField {
                    name: f.name.clone(),
                    mult,
                    target: f.target.clone(),
                }
            }).collect(),
            is_enum: s.is_abstract,
        }
    }).collect();

    // Extract function names from fact candidates (validation functions)
    let fns = mined.fact_candidates.iter()
        .filter(|f| f.source_pattern.contains("fn ") || f.source_pattern.contains("function "))
        .filter_map(|f| {
            // Try to extract function name from source_pattern
            let name = f.source_pattern
                .strip_prefix("reverse-translated fn ")?;
            Some(ExtractedFn { name: name.to_string() })
        })
        .collect();

    ExtractedImpl { structs, fns }
}

/// Collect all source file contents from a directory (recursive) for validation checking.
fn collect_all_sources(dir: &Path) -> Result<Vec<String>, CheckError> {
    let mut sources = Vec::new();
    collect_sources_recursive(dir, &mut sources)?;
    Ok(sources)
}

fn collect_sources_recursive(dir: &Path, sources: &mut Vec<String>) -> Result<(), CheckError> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_sources_recursive(&path, sources)?;
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "rs" | "ts" | "kt" | "java" | "go" | "json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        sources.push(content);
                    }
                }
            }
        }
    }
    Ok(())
}

fn extract_fns_generic(src: &str) -> Vec<ExtractedFn> {
    let mut result = Vec::new();
    for line in src.lines() {
        let trimmed = line.trim();
        // TS: "export function name(" / "function name("
        // Kotlin: "fun name("
        // Java: "public static void name(" / "public static boolean name("
        // Swift: "func name("
        let rest = trimmed.strip_prefix("export function ")
            .or_else(|| trimmed.strip_prefix("function "))
            .or_else(|| trimmed.strip_prefix("fun "))
            .or_else(|| trimmed.strip_prefix("func "))
            .or_else(|| {
                // Java: after "public static <return_type> " or "static <return_type> "
                let after = if trimmed.starts_with("public static ") {
                    Some(&trimmed["public static ".len()..])
                } else if trimmed.starts_with("static ") && !trimmed.starts_with("static {") {
                    Some(&trimmed["static ".len()..])
                } else {
                    None
                }?;
                // Skip return type (first word) + space
                let space = after.find(' ')?;
                Some(&after[space + 1..])
            });
        if let Some(rest) = rest {
            let name: String = rest.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                result.push(ExtractedFn { name });
            }
        }
    }
    result
}
