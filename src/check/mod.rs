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

    // Auto-detect language: check for models.ts first, then models.rs
    let is_ts = impl_dir.join("models.ts").exists();
    let extracted = if is_ts {
        extract_ts(impl_dir)?
    } else if impl_dir.join("models.rs").exists() {
        extract_rust(impl_dir)?
    } else {
        return Err(CheckError::ImplNotFound(
            "models.rs or models.ts".to_string()
        ));
    };

    // TS backend preserves camelCase fn names; Rust backend converts to snake_case
    let diffs = if is_ts {
        differ::diff_identity(&ir, &extracted)
    } else {
        differ::diff(&ir, &extracted)
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

fn extract_ts(impl_dir: &Path) -> Result<ExtractedImpl, CheckError> {
    let models_src = std::fs::read_to_string(impl_dir.join("models.ts"))?;
    let ops_path = impl_dir.join("operations.ts");
    let ops_src = if ops_path.exists() {
        std::fs::read_to_string(&ops_path)?
    } else {
        String::new()
    };

    // Use mine ts_extractor for structural extraction, then convert to ExtractedImpl
    let mined_models = mine::ts_extractor::extract(&models_src);

    let structs = mined_models.sigs.iter().map(|s| {
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

    // Extract fn names from operations.ts using line-based parsing
    let fns = extract_ts_fns(&ops_src);

    Ok(ExtractedImpl { structs, fns })
}

fn extract_ts_fns(src: &str) -> Vec<ExtractedFn> {
    let mut result = Vec::new();
    for line in src.lines() {
        let trimmed = line.trim();
        // "export function fooBar(" or "function fooBar("
        let rest = trimmed.strip_prefix("export function ")
            .or_else(|| trimmed.strip_prefix("function "));
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
