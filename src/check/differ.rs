/// Computes structural diff between oxidtr IR and extracted Rust implementation.

use crate::ir::nodes::OxidtrIR;
use crate::parser::ast::Multiplicity;
use super::impl_parser::{ExtractedImpl, ExtractedStruct};

fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_lowercase().next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffItem {
    MissingStruct { name: String },
    ExtraStruct    { name: String },
    MissingField   { struct_name: String, field_name: String },
    ExtraField     { struct_name: String, field_name: String },
    MultiplicityMismatch {
        struct_name: String,
        field_name:  String,
        expected:    Multiplicity,
        actual:      Multiplicity,
    },
    MissingFn { name: String },
    ExtraFn   { name: String },
}

impl std::fmt::Display for DiffItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffItem::MissingStruct { name } =>
                write!(f, "[MISSING_STRUCT] {name}: in model but not in impl"),
            DiffItem::ExtraStruct { name } =>
                write!(f, "[EXTRA_STRUCT] {name}: in impl but not in model"),
            DiffItem::MissingField { struct_name, field_name } =>
                write!(f, "[MISSING_FIELD] {struct_name}.{field_name}: in model but not in impl"),
            DiffItem::ExtraField { struct_name, field_name } =>
                write!(f, "[EXTRA_FIELD] {struct_name}.{field_name}: in impl but not in model"),
            DiffItem::MultiplicityMismatch { struct_name, field_name, expected, actual } =>
                write!(f, "[MULTIPLICITY_MISMATCH] {struct_name}.{field_name}: \
                    expected {expected:?}, got {actual:?}"),
            DiffItem::MissingFn { name } =>
                write!(f, "[MISSING_FN] {name}: pred in model but not in impl"),
            DiffItem::ExtraFn { name } =>
                write!(f, "[EXTRA_FN] {name}: fn in impl but not in model"),
        }
    }
}

pub fn diff(ir: &OxidtrIR, extracted: &ExtractedImpl) -> Vec<DiffItem> {
    let mut diffs = Vec::new();

    // ── structs ────────────────────────────────────────────────────────────────
    let ir_names: std::collections::HashSet<&str> =
        ir.structures.iter().map(|s| s.name.as_str()).collect();
    let impl_map: std::collections::HashMap<&str, &ExtractedStruct> =
        extracted.structs.iter().map(|s| (s.name.as_str(), s)).collect();

    // Missing: in IR but not in impl
    for s in &ir.structures {
        if !impl_map.contains_key(s.name.as_str()) {
            diffs.push(DiffItem::MissingStruct { name: s.name.clone() });
        }
    }

    // Extra: in impl but not in IR
    for s in &extracted.structs {
        if !ir_names.contains(s.name.as_str()) {
            diffs.push(DiffItem::ExtraStruct { name: s.name.clone() });
        }
    }

    // Field-level diff for structs present in both
    for ir_struct in &ir.structures {
        let Some(impl_struct) = impl_map.get(ir_struct.name.as_str()) else { continue };

        let impl_field_map: std::collections::HashMap<&str, _> =
            impl_struct.fields.iter().map(|f| (f.name.as_str(), f)).collect();
        let ir_field_names: std::collections::HashSet<&str> =
            ir_struct.fields.iter().map(|f| f.name.as_str()).collect();

        for ir_field in &ir_struct.fields {
            match impl_field_map.get(ir_field.name.as_str()) {
                None => diffs.push(DiffItem::MissingField {
                    struct_name: ir_struct.name.clone(),
                    field_name:  ir_field.name.clone(),
                }),
                Some(impl_field) if impl_field.mult != ir_field.mult => {
                    diffs.push(DiffItem::MultiplicityMismatch {
                        struct_name: ir_struct.name.clone(),
                        field_name:  ir_field.name.clone(),
                        expected:    ir_field.mult.clone(),
                        actual:      impl_field.mult.clone(),
                    });
                }
                _ => {}
            }
        }

        for impl_field in &impl_struct.fields {
            if !ir_field_names.contains(impl_field.name.as_str()) {
                diffs.push(DiffItem::ExtraField {
                    struct_name: ir_struct.name.clone(),
                    field_name:  impl_field.name.clone(),
                });
            }
        }
    }

    // ── operations ─────────────────────────────────────────────────────────────
    // Pred names are camelCase in Alloy, but generate emits snake_case fn names.
    // Compare by normalizing both sides to snake_case.
    let ir_fn_pairs: Vec<(String, &str)> = ir.operations.iter()
        .map(|o| (to_snake_case(&o.name), o.name.as_str()))
        .collect();
    let impl_fn_snakes: std::collections::HashSet<&str> =
        extracted.fns.iter().map(|f| f.name.as_str()).collect();
    // Also build reverse: snake_case → original pred name, for EXTRA_FN lookup
    let ir_snake_set: std::collections::HashSet<String> =
        ir_fn_pairs.iter().map(|(s, _)| s.clone()).collect();

    for (snake, orig) in &ir_fn_pairs {
        if !impl_fn_snakes.contains(snake.as_str()) {
            diffs.push(DiffItem::MissingFn { name: orig.to_string() });
        }
    }
    for f in &extracted.fns {
        if !ir_snake_set.contains(f.name.as_str()) {
            diffs.push(DiffItem::ExtraFn { name: f.name.clone() });
        }
    }

    diffs
}
