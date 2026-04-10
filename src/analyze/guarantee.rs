/// Type-system guarantee analysis.
///
/// Determines what level of guarantee each target language's type system
/// can provide for a given constraint, following the spec rule:
/// "Guarantee budget is constant. Stronger type systems reduce test generation;
/// weaker ones increase it."
///
/// Language strength ranking: Rust > Swift ≈ Kotlin > Java > TypeScript

use super::ConstraintInfo;

/// How well the type system can enforce a constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Guarantee {
    /// Type system fully encodes this constraint — no test needed.
    FullyByType,
    /// Type system partially helps — generate a regression test (simpler).
    PartiallyByType,
    /// Type system cannot encode this — generate a full property-based test.
    RequiresTest,
}

/// Target language for code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetLang {
    Rust,
    Swift,
    Kotlin,
    Java,
    Go,
    CSharp,
    TypeScript,
    Lean,
}

impl TargetLang {
    /// Parse from string (matching CLI target values).
    pub fn from_target_str(s: &str) -> Option<Self> {
        match s {
            "rust" => Some(TargetLang::Rust),
            "swift" => Some(TargetLang::Swift),
            "kotlin" | "kt" => Some(TargetLang::Kotlin),
            "java" => Some(TargetLang::Java),
            "go" => Some(TargetLang::Go),
            "csharp" | "cs" => Some(TargetLang::CSharp),
            "typescript" | "ts" => Some(TargetLang::TypeScript),
            "lean" => Some(TargetLang::Lean),
            _ => None,
        }
    }

    /// Whether this language should include schemas.json by default.
    pub fn schema_default(self) -> bool {
        // All languages default off. Use --schema to enable.
        // TS round-trip tests use --schema for lossless cardinality recovery.
        match self {
            TargetLang::Rust | TargetLang::Swift | TargetLang::Kotlin | TargetLang::Java | TargetLang::Go | TargetLang::CSharp | TargetLang::TypeScript | TargetLang::Lean => false,
        }
    }
}

/// Determine what guarantee the target language's type system provides
/// for a given constraint.
///
/// Classification rules from spec:
/// - Null safety: Rust/Kotlin → FullyByType, Java → PartiallyByType, TS → RequiresTest
/// - Enum exhaustiveness: Rust/Kotlin/Java → FullyByType, TS → RequiresTest
/// - Cardinality bounds: Rust (newtype TryFrom) → PartiallyByType, others → RequiresTest
/// - Self-reference exclusion: All → RequiresTest
/// - Acyclicity: All → RequiresTest
/// - Membership: All → RequiresTest
pub fn can_guarantee_by_type(constraint: &ConstraintInfo, lang: TargetLang) -> Guarantee {
    match constraint {
        // Null safety via Option<T> (Rust) or T? (Kotlin)
        ConstraintInfo::Presence { kind: super::PresenceKind::Required, .. } => {
            match lang {
                TargetLang::Rust | TargetLang::Swift | TargetLang::Kotlin | TargetLang::CSharp | TargetLang::Lean => Guarantee::FullyByType,
                TargetLang::Java | TargetLang::Go => Guarantee::PartiallyByType,
                TargetLang::TypeScript => Guarantee::RequiresTest,
            }
        }
        // Absence (no sig.field) — same pattern
        ConstraintInfo::Presence { kind: super::PresenceKind::Absent, .. } => {
            match lang {
                TargetLang::Rust | TargetLang::Swift | TargetLang::Kotlin | TargetLang::CSharp | TargetLang::Lean => Guarantee::FullyByType,
                TargetLang::Java | TargetLang::Go => Guarantee::PartiallyByType,
                TargetLang::TypeScript => Guarantee::RequiresTest,
            }
        }
        // Cardinality bounds: only Rust can partially guarantee via newtype TryFrom
        ConstraintInfo::CardinalityBound { .. } => {
            match lang {
                TargetLang::Rust => Guarantee::PartiallyByType,
                _ => Guarantee::RequiresTest,
            }
        }
        // Self-reference exclusion: no type system can encode this
        ConstraintInfo::NoSelfRef { .. } => Guarantee::RequiresTest,
        // Acyclicity: no type system can encode this
        ConstraintInfo::Acyclic { .. } => Guarantee::RequiresTest,
        // Membership: no type system can encode this
        ConstraintInfo::Membership { .. } => Guarantee::RequiresTest,
        // Iff: no type system can encode biconditional constraints
        ConstraintInfo::Iff { .. } => Guarantee::RequiresTest,
        // Implication: no type system can encode conditional constraints
        ConstraintInfo::Implication { .. } => Guarantee::RequiresTest,
        // Disjoint/Exhaustive: no type system can fully encode partition constraints
        ConstraintInfo::Disjoint { .. } => Guarantee::RequiresTest,
        ConstraintInfo::Exhaustive { .. } => Guarantee::RequiresTest,
        // Field ordering: no type system can encode field ordering
        ConstraintInfo::FieldOrdering { .. } => Guarantee::RequiresTest,
        // Prohibition: no type system can encode negated existentials
        ConstraintInfo::Prohibition { .. } => Guarantee::RequiresTest,
        // Value bounds: no type system can encode range constraints
        ConstraintInfo::ValueBound { .. } => Guarantee::RequiresTest,
        // Named/generic constraints: no type system can encode these
        ConstraintInfo::Named { .. } => Guarantee::RequiresTest,
    }
}

/// Classify all constraints for a given language.
/// Returns (constraint, guarantee) pairs.
pub fn classify_all(constraints: &[ConstraintInfo], lang: TargetLang) -> Vec<(&ConstraintInfo, Guarantee)> {
    constraints.iter()
        .map(|c| (c, can_guarantee_by_type(c, lang)))
        .collect()
}

/// Check if any constraint for a sig is FullyByType for the given language.
/// Used to decide whether to skip generating a test.
pub fn has_type_guarantee(constraints: &[ConstraintInfo], lang: TargetLang, sig_name: &str) -> bool {
    constraints.iter().any(|c| {
        let matches_sig = match c {
            ConstraintInfo::Presence { sig_name: s, .. } => s == sig_name,
            _ => false,
        };
        matches_sig && can_guarantee_by_type(c, lang) == Guarantee::FullyByType
    })
}

/// Check if enum exhaustiveness is guaranteed by the type system for a language.
/// Rust (match), Kotlin (when), Java (switch) → FullyByType. TS → RequiresTest.
pub fn enum_exhaustiveness_guarantee(lang: TargetLang) -> Guarantee {
    match lang {
        TargetLang::Rust | TargetLang::Swift | TargetLang::Kotlin | TargetLang::Java | TargetLang::CSharp | TargetLang::Lean => Guarantee::FullyByType,
        TargetLang::Go | TargetLang::TypeScript => Guarantee::RequiresTest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::{ConstraintInfo, PresenceKind, BoundKind};

    #[test]
    fn null_safety_rust_fully_guaranteed() {
        let c = ConstraintInfo::Presence {
            sig_name: "User".to_string(),
            field_name: "name".to_string(),
            kind: PresenceKind::Required,
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Rust), Guarantee::FullyByType);
    }

    #[test]
    fn null_safety_kotlin_fully_guaranteed() {
        let c = ConstraintInfo::Presence {
            sig_name: "User".to_string(),
            field_name: "name".to_string(),
            kind: PresenceKind::Required,
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Kotlin), Guarantee::FullyByType);
    }

    #[test]
    fn null_safety_java_partially_guaranteed() {
        let c = ConstraintInfo::Presence {
            sig_name: "User".to_string(),
            field_name: "name".to_string(),
            kind: PresenceKind::Required,
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Java), Guarantee::PartiallyByType);
    }

    #[test]
    fn null_safety_ts_requires_test() {
        let c = ConstraintInfo::Presence {
            sig_name: "User".to_string(),
            field_name: "name".to_string(),
            kind: PresenceKind::Required,
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::TypeScript), Guarantee::RequiresTest);
    }

    #[test]
    fn cardinality_rust_partially_guaranteed() {
        let c = ConstraintInfo::CardinalityBound {
            sig_name: "User".to_string(),
            field_name: "roles".to_string(),
            bound: BoundKind::AtMost(5),
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Rust), Guarantee::PartiallyByType);
    }

    #[test]
    fn cardinality_ts_requires_test() {
        let c = ConstraintInfo::CardinalityBound {
            sig_name: "User".to_string(),
            field_name: "roles".to_string(),
            bound: BoundKind::AtMost(5),
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::TypeScript), Guarantee::RequiresTest);
    }

    #[test]
    fn cardinality_kotlin_requires_test() {
        let c = ConstraintInfo::CardinalityBound {
            sig_name: "User".to_string(),
            field_name: "roles".to_string(),
            bound: BoundKind::AtMost(5),
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Kotlin), Guarantee::RequiresTest);
    }

    #[test]
    fn no_self_ref_always_requires_test() {
        let c = ConstraintInfo::NoSelfRef {
            sig_name: "Node".to_string(),
            field_name: "parent".to_string(),
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Rust), Guarantee::RequiresTest);
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Kotlin), Guarantee::RequiresTest);
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Java), Guarantee::RequiresTest);
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Go), Guarantee::RequiresTest);
        assert_eq!(can_guarantee_by_type(&c, TargetLang::TypeScript), Guarantee::RequiresTest);
    }

    #[test]
    fn acyclic_always_requires_test() {
        let c = ConstraintInfo::Acyclic {
            sig_name: "Node".to_string(),
            field_name: "parent".to_string(),
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Rust), Guarantee::RequiresTest);
        assert_eq!(can_guarantee_by_type(&c, TargetLang::TypeScript), Guarantee::RequiresTest);
    }

    #[test]
    fn membership_always_requires_test() {
        let c = ConstraintInfo::Membership {
            sig_name: "User".to_string(),
            field_name: "roles".to_string(),
        };
        assert_eq!(can_guarantee_by_type(&c, TargetLang::Rust), Guarantee::RequiresTest);
        assert_eq!(can_guarantee_by_type(&c, TargetLang::TypeScript), Guarantee::RequiresTest);
    }

    #[test]
    fn enum_exhaustiveness_by_lang() {
        assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Rust), Guarantee::FullyByType);
        assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Kotlin), Guarantee::FullyByType);
        assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Java), Guarantee::FullyByType);
        assert_eq!(enum_exhaustiveness_guarantee(TargetLang::Go), Guarantee::RequiresTest);
        assert_eq!(enum_exhaustiveness_guarantee(TargetLang::TypeScript), Guarantee::RequiresTest);
    }

    #[test]
    fn target_lang_from_str() {
        assert_eq!(TargetLang::from_target_str("rust"), Some(TargetLang::Rust));
        assert_eq!(TargetLang::from_target_str("kotlin"), Some(TargetLang::Kotlin));
        assert_eq!(TargetLang::from_target_str("kt"), Some(TargetLang::Kotlin));
        assert_eq!(TargetLang::from_target_str("java"), Some(TargetLang::Java));
        assert_eq!(TargetLang::from_target_str("typescript"), Some(TargetLang::TypeScript));
        assert_eq!(TargetLang::from_target_str("ts"), Some(TargetLang::TypeScript));
        assert_eq!(TargetLang::from_target_str("go"), Some(TargetLang::Go));
        assert_eq!(TargetLang::from_target_str("python"), None);
    }

    #[test]
    fn schema_defaults() {
        assert!(!TargetLang::Rust.schema_default());
        assert!(!TargetLang::Kotlin.schema_default());
        assert!(!TargetLang::Java.schema_default());
        assert!(!TargetLang::Go.schema_default());
        assert!(!TargetLang::TypeScript.schema_default());
    }

    #[test]
    fn classify_all_mixed_constraints() {
        let constraints = vec![
            ConstraintInfo::Presence {
                sig_name: "User".to_string(),
                field_name: "name".to_string(),
                kind: PresenceKind::Required,
            },
            ConstraintInfo::CardinalityBound {
                sig_name: "User".to_string(),
                field_name: "roles".to_string(),
                bound: BoundKind::AtMost(5),
            },
            ConstraintInfo::Acyclic {
                sig_name: "Node".to_string(),
                field_name: "parent".to_string(),
            },
        ];

        // Rust: 1 FullyByType, 1 PartiallyByType, 1 RequiresTest
        let rust_results = classify_all(&constraints, TargetLang::Rust);
        assert_eq!(rust_results[0].1, Guarantee::FullyByType);
        assert_eq!(rust_results[1].1, Guarantee::PartiallyByType);
        assert_eq!(rust_results[2].1, Guarantee::RequiresTest);

        // TS: 0 FullyByType, 0 PartiallyByType, 3 RequiresTest
        let ts_results = classify_all(&constraints, TargetLang::TypeScript);
        assert!(ts_results.iter().all(|(_, g)| *g == Guarantee::RequiresTest));
    }

    #[test]
    fn rust_generates_fewer_tests_than_ts() {
        let constraints = vec![
            ConstraintInfo::Presence {
                sig_name: "User".to_string(),
                field_name: "name".to_string(),
                kind: PresenceKind::Required,
            },
            ConstraintInfo::Presence {
                sig_name: "User".to_string(),
                field_name: "email".to_string(),
                kind: PresenceKind::Required,
            },
            ConstraintInfo::CardinalityBound {
                sig_name: "User".to_string(),
                field_name: "roles".to_string(),
                bound: BoundKind::AtMost(5),
            },
            ConstraintInfo::Acyclic {
                sig_name: "Node".to_string(),
                field_name: "parent".to_string(),
            },
        ];

        let rust_tests = classify_all(&constraints, TargetLang::Rust)
            .iter()
            .filter(|(_, g)| *g == Guarantee::RequiresTest)
            .count();
        let ts_tests = classify_all(&constraints, TargetLang::TypeScript)
            .iter()
            .filter(|(_, g)| *g == Guarantee::RequiresTest)
            .count();

        assert!(rust_tests < ts_tests, "Rust ({}) should generate fewer tests than TS ({})", rust_tests, ts_tests);
    }
}
