pub mod kotlin;
pub mod java;
pub mod expr_translator;

use crate::ir::nodes::*;
use crate::parser::ast::Multiplicity;
use std::collections::{HashMap, HashSet};

/// Shared JVM type mapping context computed from IR.
pub struct JvmContext {
    pub children: HashMap<String, Vec<String>>,
    pub enum_parents: HashSet<String>,
    pub variant_names: HashSet<String>,
    pub struct_map: HashMap<String, StructureNode>,
    pub cyclic_fields: HashSet<(String, String)>,
}

impl JvmContext {
    pub fn from_ir(ir: &OxidtrIR) -> Self {
        let children: HashMap<String, Vec<String>> = {
            let mut map: HashMap<String, Vec<String>> = HashMap::new();
            for s in &ir.structures {
                if let Some(parent) = &s.parent {
                    map.entry(parent.clone()).or_default().push(s.name.clone());
                }
            }
            map
        };

        let enum_parents: HashSet<String> = ir
            .structures.iter()
            .filter(|s| s.is_enum)
            .map(|s| s.name.clone())
            .collect();

        let variant_names: HashSet<String> = ir
            .structures.iter()
            .filter(|s| s.parent.as_ref().map_or(false, |p| enum_parents.contains(p)))
            .map(|s| s.name.clone())
            .collect();

        let struct_map: HashMap<String, StructureNode> = ir
            .structures.iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect();

        let cyclic_fields = super::rust::find_cyclic_fields(ir);

        JvmContext { children, enum_parents, variant_names, struct_map, cyclic_fields }
    }

    pub fn is_variant(&self, name: &str) -> bool {
        self.variant_names.contains(name)
    }
}

/// Abstract JVM type representation for a field.
pub fn jvm_type_name(target: &str, mult: &Multiplicity) -> (String, bool) {
    // Returns (type_string_without_wrapper, is_collection)
    match mult {
        Multiplicity::One => (target.to_string(), false),
        Multiplicity::Lone => (target.to_string(), false), // nullable handling is language-specific
        Multiplicity::Set | Multiplicity::Seq => (target.to_string(), true),
    }
}
