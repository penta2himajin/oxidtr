#[allow(unused_imports)]
use super::models::*;
#[allow(unused_imports)]
use super::fixtures::*;
#[allow(unused_imports)]
use super::operations::*;
#[allow(unused_imports)]
use std::collections::BTreeSet;

/// @temporal Liveness property — cannot be fully verified at runtime; static test approximates via implies
#[test]
fn later_pos() {
    let trace: Vec<Vec<P>> = Vec::new();
    assert!(!check_liveness_later_pos(&trace), "empty trace must never satisfy liveness");
}

/// Trace checker for liveness: property holds in at least one future state.
#[allow(dead_code)]
fn check_liveness_later_pos(trace: &[Vec<P>]) -> bool {
    trace.iter().any(|ps| {
        ps.iter().all(|p| { let p = p.clone(); pos(&p) })
    })
}

// --- Anomaly tests: edge-case coverage ---

/// Anomaly: field `x` is not constrained by any fact.
#[test]
fn anomaly_unconstrained_p_x() {
    let instance = default_p();
    let cloned = instance.clone();
    assert_eq!(instance.x, cloned.x, "clone must preserve unconstrained field x");
}

