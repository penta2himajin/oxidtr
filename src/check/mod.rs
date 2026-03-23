// check module — diff Alloy IR vs implementation (Rust or TypeScript)
pub mod impl_parser;
pub mod differ;

use crate::parser;
use crate::ir;
use crate::mine;
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
    let (extracted, use_snake_case) = if impl_dir.join("models.ts").exists() {
        (extract_mined(impl_dir, "models.ts", "operations.ts", mine::ts_extractor::extract)?, false)
    } else if impl_dir.join("Models.kt").exists() {
        (extract_mined(impl_dir, "Models.kt", "Operations.kt", mine::kotlin_extractor::extract)?, false)
    } else if impl_dir.join("Models.java").exists() {
        (extract_mined(impl_dir, "Models.java", "Operations.java", mine::java_extractor::extract)?, false)
    } else if impl_dir.join("models.rs").exists() {
        (extract_rust(impl_dir)?, true)
    } else {
        return Err(CheckError::ImplNotFound(
            "models.rs, models.ts, Models.kt, or Models.java".to_string()
        ));
    };

    let diffs = if use_snake_case {
        differ::diff(&ir, &extracted)
    } else {
        differ::diff_identity(&ir, &extracted)
    };
    Ok(CheckResult { diffs })
}

fn extract_rust(impl_dir: &Path) -> Result<ExtractedImpl, CheckError> {
    let models_src = std::fs::read_to_string(impl_dir.join("models.rs"))?;
    let ops_path = impl_dir.join("operations.rs");
    let ops_src = if ops_path.exists() {
        std::fs::read_to_string(&ops_path)?
    } else {
        String::new()
    };
    Ok(impl_parser::parse_impl(&models_src, &ops_src))
}

/// Extract using a mine extractor function, then convert MinedModel → ExtractedImpl.
fn extract_mined<F>(
    impl_dir: &Path,
    models_file: &str,
    ops_file: &str,
    extractor: F,
) -> Result<ExtractedImpl, CheckError>
where F: Fn(&str) -> mine::MinedModel {
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
                    mine::MinedMultiplicity::One => crate::parser::ast::Multiplicity::One,
                    mine::MinedMultiplicity::Lone => crate::parser::ast::Multiplicity::Lone,
                    mine::MinedMultiplicity::Set => crate::parser::ast::Multiplicity::Set,
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
fn extract_fns_generic(src: &str) -> Vec<ExtractedFn> {
    let mut result = Vec::new();
    for line in src.lines() {
        let trimmed = line.trim();
        // TS: "export function name(" / "function name("
        // Kotlin: "fun name("
        // Java: "public static void name(" / "public static boolean name("
        let rest = trimmed.strip_prefix("export function ")
            .or_else(|| trimmed.strip_prefix("function "))
            .or_else(|| trimmed.strip_prefix("fun "))
            .or_else(|| {
                // Java: after "public static <return_type> "
                if trimmed.starts_with("public static ") {
                    let after = &trimmed["public static ".len()..];
                    // Skip return type (first word) + space
                    let space = after.find(' ')?;
                    Some(&after[space + 1..])
                } else {
                    None
                }
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
