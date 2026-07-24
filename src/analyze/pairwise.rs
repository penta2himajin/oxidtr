/// Deterministic pairwise-covering-array generation.
///
/// Given N dimensions each with a pool of candidate values, returns a small
/// set of tuples such that every pair of (dimension, value) combinations
/// across any 2 dimensions appears together in at least one tuple. Used to
/// turn per-field fixture-diversity pools (Phase 3a/3b) into a bounded set
/// of test instances for facts quantifying N variables over the same sig,
/// instead of either a single vacuous fixture or a full K^N cross product.
///
/// For N <= 2 this is necessarily equal to full enumeration (there is only
/// one dimension, or only one pair of dimensions to cover) — reduction only
/// kicks in at N >= 3.
pub fn pairwise_covering_tuples<T: Clone>(pools: &[Vec<T>]) -> Vec<Vec<T>> {
    let n = pools.len();
    if n == 0 || pools.iter().any(|p| p.is_empty()) {
        return Vec::new();
    }
    if n == 1 {
        return pools[0].iter().map(|v| vec![v.clone()]).collect();
    }

    let mut uncovered: Vec<(usize, usize, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            for a in 0..pools[i].len() {
                for b in 0..pools[j].len() {
                    uncovered.push((i, a, j, b));
                }
            }
        }
    }

    let mut tuples: Vec<Vec<usize>> = Vec::new();

    while let Some(&(i0, a0, j0, b0)) = uncovered.first() {
        let mut chosen: Vec<Option<usize>> = vec![None; n];
        chosen[i0] = Some(a0);
        chosen[j0] = Some(b0);

        for k in 0..n {
            if chosen[k].is_some() {
                continue;
            }
            let mut best_idx = 0;
            let mut best_covered = 0usize;
            for v in 0..pools[k].len() {
                let covered = chosen
                    .iter()
                    .enumerate()
                    .filter_map(|(d, c)| c.map(|idx| (d, idx)))
                    .filter(|&(d, idx)| pair_uncovered(&uncovered, d, idx, k, v))
                    .count();
                if v == 0 || covered > best_covered {
                    best_covered = covered;
                    best_idx = v;
                }
            }
            chosen[k] = Some(best_idx);
        }

        let full: Vec<usize> = chosen.into_iter().map(|c| c.unwrap()).collect();
        uncovered.retain(|&(i, a, j, b)| !(full[i] == a && full[j] == b));
        tuples.push(full);
    }

    tuples
        .into_iter()
        .map(|idxs| {
            idxs.iter()
                .enumerate()
                .map(|(d, &idx)| pools[d][idx].clone())
                .collect()
        })
        .collect()
}

fn pair_uncovered(
    uncovered: &[(usize, usize, usize, usize)],
    d1: usize,
    v1: usize,
    d2: usize,
    v2: usize,
) -> bool {
    let (i, a, j, b) = if d1 < d2 { (d1, v1, d2, v2) } else { (d2, v2, d1, v1) };
    uncovered
        .iter()
        .any(|&(ui, ua, uj, ub)| ui == i && ua == a && uj == j && ub == b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every pair of (dimension, value) combinations across any 2 dimensions
    /// must appear together in at least one output tuple.
    fn assert_full_pairwise_coverage<T: Clone + PartialEq + std::fmt::Debug>(
        pools: &[Vec<T>],
        tuples: &[Vec<T>],
    ) {
        let n = pools.len();
        for i in 0..n {
            for j in (i + 1)..n {
                for a in &pools[i] {
                    for b in &pools[j] {
                        let covered = tuples.iter().any(|t| t[i] == *a && t[j] == *b);
                        assert!(
                            covered,
                            "pair (dim {i}={a:?}, dim {j}={b:?}) not covered by any tuple"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn single_dimension_is_full_enumeration() {
        let pools = vec![vec![1, 2, 3]];
        let result = pairwise_covering_tuples(&pools);
        assert_eq!(result, vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn two_dimensions_equals_full_cross_product() {
        let pools = vec![vec![1, 2], vec![10, 20, 30]];
        let result = pairwise_covering_tuples(&pools);

        let expected: HashSet<(i32, i32)> = pools[0]
            .iter()
            .flat_map(|&a| pools[1].iter().map(move |&b| (a, b)))
            .collect();
        let actual: HashSet<(i32, i32)> = result.iter().map(|t| (t[0], t[1])).collect();
        assert_eq!(actual, expected);
        assert_eq!(result.len(), pools[0].len() * pools[1].len());
    }

    #[test]
    fn three_dimensions_covers_all_pairs_with_fewer_than_full_cross_product() {
        // Matches the real Assoc law's arity: all a, b, c: Money | ...
        let pools = vec![vec![0, 1], vec![0, 1], vec![0, 1]];
        let result = pairwise_covering_tuples(&pools);

        assert_full_pairwise_coverage(&pools, &result);
        assert!(
            result.len() < 8,
            "expected pairwise reduction below the full 2^3 cross product, got {} tuples",
            result.len()
        );
    }

    #[test]
    fn uneven_pool_sizes_still_cover_all_pairs() {
        let pools = vec![vec!["a", "b"], vec!["x", "y", "z"], vec!["p", "q"]];
        let result = pairwise_covering_tuples(&pools);
        assert_full_pairwise_coverage(&pools, &result);
    }

    #[test]
    fn four_dimensions_still_cover_all_pairs() {
        let pools = vec![vec![0, 1], vec![0, 1], vec![0, 1], vec![0, 1]];
        let result = pairwise_covering_tuples(&pools);
        assert_full_pairwise_coverage(&pools, &result);
    }

    #[test]
    fn is_deterministic_across_calls() {
        let pools = vec![vec![0, 1], vec![0, 1, 2], vec![0, 1]];
        let first = pairwise_covering_tuples(&pools);
        let second = pairwise_covering_tuples(&pools);
        assert_eq!(first, second);
    }

    #[test]
    fn empty_pool_yields_no_tuples() {
        let pools: Vec<Vec<i32>> = vec![vec![1, 2], vec![]];
        assert!(pairwise_covering_tuples(&pools).is_empty());
    }
}
