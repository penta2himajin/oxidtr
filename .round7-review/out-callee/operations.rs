use super::models::*;

/// @post: for all q: P | q.x >= 0
pub fn pos(p: &P) -> bool {
    ps.iter().all(|q| { let q = q.clone(); q.x >= 0 })
}

