// check module — diff Alloy IR vs Rust implementation
pub mod impl_parser;
pub mod differ;

use crate::parser;
use crate::ir;
use differ::DiffItem;

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

    let models_path = impl_dir.join("models.rs");
    if !models_path.exists() {
        return Err(CheckError::ImplNotFound("models.rs".to_string()));
    }
    let models_src = std::fs::read_to_string(&models_path)?;

    let ops_path = impl_dir.join("operations.rs");
    let ops_src = if ops_path.exists() {
        std::fs::read_to_string(&ops_path)?
    } else {
        String::new()
    };

    let extracted = impl_parser::parse_impl(&models_src, &ops_src);
    let diffs = differ::diff(&ir, &extracted);

    Ok(CheckResult { diffs })
}
