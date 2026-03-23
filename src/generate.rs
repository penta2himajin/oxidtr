use crate::parser;
use crate::parser::ast::SigMultiplicity;
use crate::ir;
use crate::backend::rust;
use crate::backend::typescript;
use crate::backend::typescript::{TsTestRunner, TsBackendConfig};
use crate::backend::jvm::{kotlin, java};
use crate::backend::swift;
use crate::backend::go;
use crate::backend::schema;
use crate::analyze::guarantee::TargetLang;

use std::path::Path;

#[derive(Debug)]
pub enum GenerateError {
    IoError(std::io::Error),
    ParseError(parser::ParseError),
    LoweringError(ir::LoweringError),
}

impl std::fmt::Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerateError::IoError(e) => write!(f, "IO error: {e}"),
            GenerateError::ParseError(e) => write!(f, "parse error: {e}"),
            GenerateError::LoweringError(e) => write!(f, "lowering error: {e}"),
        }
    }
}

impl std::error::Error for GenerateError {}

impl From<std::io::Error> for GenerateError {
    fn from(e: std::io::Error) -> Self { GenerateError::IoError(e) }
}

impl From<parser::ParseError> for GenerateError {
    fn from(e: parser::ParseError) -> Self { GenerateError::ParseError(e) }
}

impl From<ir::LoweringError> for GenerateError {
    fn from(e: ir::LoweringError) -> Self { GenerateError::LoweringError(e) }
}

#[derive(Debug, Clone)]
pub struct GenerateConfig {
    pub target: String,
    pub output_dir: String,
    pub warnings: WarningLevel,
    pub features: Vec<String>,
    /// Force schema generation (overrides per-language default).
    pub schema: Option<bool>,
    /// Test runner for TypeScript target (default: Bun).
    pub ts_test_runner: TsTestRunner,
}

impl GenerateConfig {
    /// Create config with defaults for non-essential fields.
    pub fn new(target: &str, output_dir: &str) -> Self {
        Self {
            target: target.to_string(),
            output_dir: output_dir.to_string(),
            warnings: WarningLevel::Warn,
            features: Vec::new(),
            schema: None,
            ts_test_runner: TsTestRunner::Bun,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningLevel {
    Error,
    Warn,
    Off,
}

#[derive(Debug)]
pub struct Warning {
    pub kind: WarningKind,
    pub message: String,
    pub location: String,
    pub suggestion: Option<String>,
}

#[derive(Debug)]
pub enum WarningKind {
    UnconstrainedSelfRef,
    UnconstrainedCardinality,
    MissingInverse,
    UnreferencedSig,
    UnconstrainedTransitivity,
    UnhandledResponsePattern,
    MissingErrorPropagation,
}

#[derive(Debug)]
pub struct GenerateResult {
    pub files_written: Vec<String>,
    pub warnings: Vec<Warning>,
}

/// Run the generate pipeline: parse → lower → analyze → generate → write.
pub fn run(input_path: &str, config: &GenerateConfig) -> Result<GenerateResult, GenerateError> {
    // Read input
    let source = std::fs::read_to_string(input_path)?;

    // Parse
    let model = parser::parse(&source)?;

    // Lower to IR
    let ir = ir::lower(&model)?;

    // Analyze for warnings
    let warnings = analyze_warnings(&ir);

    // Check warning level
    if config.warnings == WarningLevel::Error && !warnings.is_empty() {
        // Print warnings and fail
        for w in &warnings {
            eprintln!("[WARN] {:?}: {} ({})", w.kind, w.message, w.location);
            if let Some(s) = &w.suggestion {
                eprintln!("  候補: {s}");
            }
        }
        return Err(GenerateError::ParseError(parser::ParseError::InvalidSyntax {
            message: format!("{} warning(s) found with --warnings=error", warnings.len()),
            pos: 0,
        }));
    }

    // Generate target code
    let mut files = match config.target.as_str() {
        "rust" => {
            let rust_config = rust::RustBackendConfig {
                features: config.features.clone(),
            };
            rust::generate_with_config(&ir, &rust_config)
        }
        "typescript" | "ts" => {
            let ts_config = TsBackendConfig {
                test_runner: config.ts_test_runner,
            };
            typescript::generate_with_config(&ir, &ts_config)
        }
        "kotlin" | "kt" => kotlin::generate(&ir),
        "java" => java::generate(&ir),
        "swift" => swift::generate(&ir),
        "go" => go::generate(&ir),
        other => {
            return Err(GenerateError::ParseError(parser::ParseError::InvalidSyntax {
                message: format!("unsupported target: {other}"),
                pos: 0,
            }));
        }
    };

    // Schema generation: based on language default or explicit --schema flag
    let lang = TargetLang::from_target_str(&config.target);
    let should_generate_schema = config.schema.unwrap_or_else(|| {
        lang.map_or(false, |l| l.schema_default())
    });
    if should_generate_schema {
        files.push(schema::generate(&ir));
    }

    // TS-specific: generate runtime validators
    if matches!(config.target.as_str(), "typescript" | "ts") {
        let validators = typescript::generate_validators(&ir);
        if !validators.is_empty() {
            files.push(crate::backend::GeneratedFile {
                path: "validators.ts".to_string(),
                content: validators,
            });
        }
    }

    // Write output files
    let output_dir = Path::new(&config.output_dir);
    std::fs::create_dir_all(output_dir)?;

    let mut files_written = Vec::new();
    for file in &files {
        let path = output_dir.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.content)?;
        files_written.push(path.display().to_string());
    }

    // Print warnings if warn level
    if config.warnings == WarningLevel::Warn {
        for w in &warnings {
            eprintln!("[WARN] {:?}: {} ({})", w.kind, w.message, w.location);
            if let Some(s) = &w.suggestion {
                eprintln!("  候補: {s}");
            }
        }
    }

    Ok(GenerateResult {
        files_written,
        warnings,
    })
}

/// AST-level warning analysis from the README spec.
fn analyze_warnings(ir: &ir::nodes::OxidtrIR) -> Vec<Warning> {
    let mut warnings = Vec::new();

    for s in &ir.structures {
        // UNCONSTRAINED_SELF_REF: field targets self with no constraint
        for f in &s.fields {
            if f.target == s.name {
                let has_constraint = ir.constraints.iter().any(|c| {
                    constraint_references_field(&c.expr, &s.name, &f.name)
                });
                if !has_constraint {
                    warnings.push(Warning {
                        kind: WarningKind::UnconstrainedSelfRef,
                        message: format!("{}.{} は自己参照を含みうる", s.name, f.name),
                        location: format!("sig {}", s.name),
                        suggestion: Some(format!(
                            "fact {{ all x: {} | x not in x.{} }}",
                            s.name, f.name
                        )),
                    });
                }
            }
        }

        // UNCONSTRAINED_CARDINALITY: set field with no cardinality constraint
        for f in &s.fields {
            if f.mult == crate::parser::ast::Multiplicity::Set {
                let has_cardinality = ir.constraints.iter().any(|c| {
                    constraint_has_cardinality(&c.expr, &f.name)
                });
                if !has_cardinality {
                    warnings.push(Warning {
                        kind: WarningKind::UnconstrainedCardinality,
                        message: format!("{}.{} に要素数の制約がない", s.name, f.name),
                        location: format!("sig {}", s.name),
                        suggestion: Some(format!(
                            "fact {{ all x: {} | #x.{} <= N }}",
                            s.name, f.name
                        )),
                    });
                }
            }
        }

        // UNREFERENCED_SIG: no other sig references this one (via fields, parent, or constraints)
        if s.parent.is_none() && !s.is_enum {
            let is_referenced = ir.structures.iter().any(|other| {
                other.name != s.name && other.fields.iter().any(|f| f.target == s.name)
            });
            let is_parent = ir.structures.iter().any(|other| {
                other.parent.as_deref() == Some(&s.name)
            });
            // Also check if sig is referenced in constraint/property quantifier domains
            let is_in_constraint = ir.constraints.iter().any(|c| {
                expr_references_sig_name(&c.expr, &s.name)
            }) || ir.properties.iter().any(|p| {
                expr_references_sig_name(&p.expr, &s.name)
            }) || ir.operations.iter().any(|o| {
                o.params.iter().any(|p| p.type_name == s.name)
            });
            if !is_referenced && !is_parent && !is_in_constraint {
                warnings.push(Warning {
                    kind: WarningKind::UnreferencedSig,
                    message: format!("{} はどこからも参照されていない", s.name),
                    location: format!("sig {}", s.name),
                    suggestion: None,
                });
            }
        }
    }


    // MISSING_INVERSE: A.r → B and B.s → A exist but no constraint references both
    for a in &ir.structures {
        for f_a in &a.fields {
            // Look for sig B = f_a.target that has a back-reference field to A
            for b in &ir.structures {
                if b.name != f_a.target { continue; }
                for f_b in &b.fields {
                    if f_b.target != a.name { continue; }
                    // Found bidirectional: a.f_a -> b, b.f_b -> a
                    let constrained = ir.constraints.iter().any(|c| {
                        constraint_references_field(&c.expr, "", &f_a.name)
                            && constraint_references_field(&c.expr, "", &f_b.name)
                    });
                    if !constrained {
                        warnings.push(Warning {
                            kind: WarningKind::MissingInverse,
                            message: format!(
                                "{}.{} <-> {}.{} の逆関係 fact がない",
                                a.name, f_a.name, b.name, f_b.name
                            ),
                            location: format!("sig {} / sig {}", a.name, b.name),
                            suggestion: Some(format!(
                                "fact {{ all a: {} | all b: {} | a in b.{} iff b in a.{} }}",
                                a.name, b.name, f_b.name, f_a.name
                            )),
                        });
                    }
                }
            }
        }
    }

    // UNCONSTRAINED_TRANSITIVITY: ^field used but no direct fact constrains field
    {
        // Collect all fields used with TransitiveClosure across constraints + properties
        let mut tc_fields: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &ir.constraints {
            collect_tc_fields(&c.expr, &mut tc_fields);
        }
        for p in &ir.properties {
            collect_tc_fields(&p.expr, &mut tc_fields);
        }
        for field in &tc_fields {
            // If no constraint references this field directly (not via TC), warn
            let has_direct_constraint = ir.constraints.iter().any(|c| {
                constraint_references_field_non_tc(&c.expr, field)
            });
            if !has_direct_constraint {
                warnings.push(Warning {
                    kind: WarningKind::UnconstrainedTransitivity,
                    message: format!("^{field} が使われているが {field} への直接 fact がない"),
                    location: format!("field {field}"),
                    suggestion: None,
                });
            }
        }
    }


    // UNHANDLED_RESPONSE_PATTERN / MISSING_ERROR_PROPAGATION
    // Abstract sig (is_enum=true) with 2+ children where some child has no pred.
    {
        // Build parent → children map
        let mut children: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for s in &ir.structures {
            if let Some(parent) = &s.parent {
                children.entry(parent.as_str()).or_default().push(s.name.as_str());
            }
        }
        // Collect all sig names used as pred param types
        // Also collect abstract parent types that have preds (covering all their children)
        let handled: std::collections::HashSet<&str> =
            ir.operations.iter()
              .flat_map(|op| op.params.iter().map(|p| p.type_name.as_str()))
              .collect();
        // Build child → parent map for parent-covers-child check
        let child_to_parent: std::collections::HashMap<&str, &str> =
            ir.structures.iter()
              .filter_map(|s| s.parent.as_deref().map(|p| (s.name.as_str(), p)))
              .collect();

        for s in &ir.structures {
            if !s.is_enum { continue; }
            let variants = match children.get(s.name.as_str()) {
                Some(v) if v.len() >= 2 => v,
                _ => continue, // single-child or no children → skip
            };
            // 全 child が one sig (singleton) なら値 enum → response パターンではない
            // some sig / lone sig children are also considered constrained (not response patterns)
            let all_singletons = variants.iter().all(|&v| {
                ir.structures.iter()
                    .find(|sn| sn.name == v)
                    .map_or(false, |sn| sn.sig_multiplicity != SigMultiplicity::Default)
            });
            if all_singletons { continue; }
            for &variant in variants {
                // Directly handled, or parent abstract type is handled (catch-all pred)
                if handled.contains(variant) { continue; }
                if child_to_parent.get(variant)
                    .map_or(false, |&parent| handled.contains(parent)) { continue; }
                let is_error = is_error_name(variant);
                warnings.push(Warning {
                    kind: if is_error {
                        WarningKind::MissingErrorPropagation
                    } else {
                        WarningKind::UnhandledResponsePattern
                    },
                    message: format!(
                        "{} ({} の sub sig) に対する pred がない",
                        variant, s.name
                    ),
                    location: format!("sig {}", variant),
                    suggestion: Some(format!(
                        "pred handle{}[r: one {}] {{}}",
                        variant, variant
                    )),
                });
            }
        }
    }

    warnings
}

/// Check if an expression references a sig name (e.g. as a quantifier domain).
fn expr_references_sig_name(expr: &crate::parser::ast::Expr, sig_name: &str) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::VarRef(name) => name == sig_name,
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| expr_references_sig_name(&b.domain, sig_name))
                || expr_references_sig_name(body, sig_name)
        }
        Expr::BinaryLogic { left, right, .. } | Expr::Comparison { left, right, .. }
        | Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            expr_references_sig_name(left, sig_name)
                || expr_references_sig_name(right, sig_name)
        }
        Expr::Not(inner) | Expr::Cardinality(inner) | Expr::TransitiveClosure(inner) => {
            expr_references_sig_name(inner, sig_name)
        }
        Expr::FieldAccess { base, .. } => expr_references_sig_name(base, sig_name),
        Expr::IntLiteral(_) => false,
    }
}

/// Check if an expression references a specific field of a sig.
fn constraint_references_field(expr: &crate::parser::ast::Expr, _sig: &str, field: &str) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::FieldAccess { base, field: f } => {
            f == field || constraint_references_field(base, _sig, field)
        }
        Expr::Comparison { left, right, .. } => {
            constraint_references_field(left, _sig, field)
                || constraint_references_field(right, _sig, field)
        }
        Expr::BinaryLogic { left, right, .. } => {
            constraint_references_field(left, _sig, field)
                || constraint_references_field(right, _sig, field)
        }
        Expr::Not(inner) => constraint_references_field(inner, _sig, field),
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| constraint_references_field(&b.domain, _sig, field))
                || constraint_references_field(body, _sig, field)
        }
        Expr::Cardinality(inner) => constraint_references_field(inner, _sig, field),
        Expr::TransitiveClosure(inner) => constraint_references_field(inner, _sig, field),
        Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            constraint_references_field(left, _sig, field)
                || constraint_references_field(right, _sig, field)
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
}

/// Check if an expression contains a cardinality operator on a field.
fn constraint_has_cardinality(expr: &crate::parser::ast::Expr, field: &str) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::Cardinality(inner) => constraint_references_field(inner, "", field),
        Expr::Comparison { left, right, .. } => {
            constraint_has_cardinality(left, field)
                || constraint_has_cardinality(right, field)
        }
        Expr::BinaryLogic { left, right, .. } => {
            constraint_has_cardinality(left, field)
                || constraint_has_cardinality(right, field)
        }
        Expr::Not(inner) => constraint_has_cardinality(inner, field),
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| constraint_has_cardinality(&b.domain, field))
                || constraint_has_cardinality(body, field)
        }
        _ => false,
    }
}

/// Collect all field names that appear directly under TransitiveClosure.
fn collect_tc_fields(expr: &crate::parser::ast::Expr, out: &mut std::collections::HashSet<String>) {
    use crate::parser::ast::Expr;
    match expr {
        Expr::TransitiveClosure(inner) => {
            collect_direct_fields(inner, out);
            collect_tc_fields(inner, out);
        }
        Expr::Comparison { left, right, .. } => {
            collect_tc_fields(left, out); collect_tc_fields(right, out);
        }
        Expr::BinaryLogic { left, right, .. } => {
            collect_tc_fields(left, out); collect_tc_fields(right, out);
        }
        Expr::Not(inner) | Expr::Cardinality(inner) => collect_tc_fields(inner, out),
        Expr::Quantifier { bindings, body, .. } => {
            for b in bindings { collect_tc_fields(&b.domain, out); }
            collect_tc_fields(body, out);
        }
        Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            collect_tc_fields(left, out); collect_tc_fields(right, out);
        }
        Expr::FieldAccess { base, .. } => collect_tc_fields(base, out),
        Expr::VarRef(_) | Expr::IntLiteral(_) => {}
    }
}

/// Collect field names directly accessed (not inside TC).
fn collect_direct_fields(expr: &crate::parser::ast::Expr, out: &mut std::collections::HashSet<String>) {
    use crate::parser::ast::Expr;
    match expr {
        Expr::FieldAccess { base, field } => {
            out.insert(field.clone());
            collect_direct_fields(base, out);
        }
        _ => {}
    }
}

/// Check if an expression references a field WITHOUT going through TransitiveClosure.
fn constraint_references_field_non_tc(expr: &crate::parser::ast::Expr, field: &str) -> bool {
    use crate::parser::ast::Expr;
    match expr {
        Expr::TransitiveClosure(_) => false, // skip inside TC
        Expr::FieldAccess { base, field: f } => {
            f == field || constraint_references_field_non_tc(base, field)
        }
        Expr::Comparison { left, right, .. } => {
            constraint_references_field_non_tc(left, field)
                || constraint_references_field_non_tc(right, field)
        }
        Expr::BinaryLogic { left, right, .. } => {
            constraint_references_field_non_tc(left, field)
                || constraint_references_field_non_tc(right, field)
        }
        Expr::Not(inner) | Expr::Cardinality(inner) => {
            constraint_references_field_non_tc(inner, field)
        }
        Expr::Quantifier { bindings, body, .. } => {
            bindings.iter().any(|b| constraint_references_field_non_tc(&b.domain, field))
                || constraint_references_field_non_tc(body, field)
        }
        Expr::SetOp { left, right, .. } | Expr::Product { left, right } => {
            constraint_references_field_non_tc(left, field)
                || constraint_references_field_non_tc(right, field)
        }
        Expr::VarRef(_) | Expr::IntLiteral(_) => false,
    }
}

/// Returns true if a sig name indicates an error/failure variant.
fn is_error_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("error")
        || lower.contains("err")
        || lower.contains("fail")
        || lower.contains("exception")
        || lower.contains("reject")
        || lower.contains("denied")
}
