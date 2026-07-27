#[allow(unused_imports)]
use super::models::*;
#[allow(unused_imports)]
use super::fixtures::*;
#[allow(unused_imports)]
use std::collections::BTreeSet;

/// @temporal Transition constraint: NestedStep
/// Verifies: pre→post state relationship (always for all c: C | for all d: D | d.v' = d.v)
#[test]
fn transition_nested_step() {
    let cs: Vec<C> = vec![default_c()];
    let next_cs: Vec<C> = cs.clone();
    let ds: Vec<D> = vec![default_d()];
    let next_ds: Vec<D> = ds.clone();
    for (c, next_c) in cs.iter().zip(next_cs.iter()) {
        assert!(ds.iter().all(|d| { let d = d.clone(); next_d.v == d.v }));
    }
}

// --- Anomaly tests: edge-case coverage ---

/// Anomaly: field `v` is not constrained by any fact.
#[test]
fn anomaly_unconstrained_c_v() {
    let instance = default_c();
    let cloned = instance.clone();
    assert_eq!(instance.v, cloned.v, "clone must preserve unconstrained field v");
}

