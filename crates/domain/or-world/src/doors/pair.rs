//! Which two swinging leaves are one double door.
//!
//! Two leaves pair where their hinge lines are parallel, their closed leaves lie in one plane at
//! one height, each reaches from its hinge toward the other's, and the opening between the two
//! hinges is no narrower than the longer reach and no wider than the two reaches together. Where
//! a leaf could pair with more than one other, the pair whose opening its two reaches fill most
//! nearly is kept, and each leaf is in one pair at most.

use super::leaf::{Hinge, Leaf};

/// How far a hinge may stand out of the other leaf's plane, or above or below the other hinge, in metres.
pub const PAIR_PLANE_TOLERANCE: f32 = 0.1;

/// How far two hinge lines may be from parallel, in radians.
pub const PAIR_ANGLE_TOLERANCE: f32 = 5.0 * std::f32::consts::PI / 180.0;

/// How much wider than the two reaches together an opening may be, in metres: the gap two closed leaves leave between them.
pub const PAIR_GAP_SLACK: f32 = 0.15;

/// Two leaves that are one double door, by their places in the slice handed in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pair {
    /// The lower of the two places.
    pub a: usize,
    pub b: usize,
    /// How far apart the two hinges are, in metres.
    pub opening: f32,
}

/// The opening two hinges close, none where the two leaves are no double door.
fn opening_of(a: &Hinge, b: &Hinge) -> Option<f32> {
    // ⚠ Two leaves whose hinges each stand in the other's plane, and that reach toward each other, point at each other
    // already, so only the hinge lines are held to an angle.
    let aligned = PAIR_ANGLE_TOLERANCE.cos();
    if a.axis.dot(b.axis) < aligned {
        return None;
    }
    let between = b.position - a.position;
    let rise = between.dot(a.axis);
    let out_of_a = between.dot(a.axis.cross(a.toward_free_edge));
    let out_of_b = between.dot(b.axis.cross(b.toward_free_edge));
    if [rise, out_of_a, out_of_b]
        .iter()
        .any(|off| off.abs() > PAIR_PLANE_TOLERANCE)
    {
        return None;
    }
    // Each reaches toward the other's hinge, never away from it.
    if between.dot(a.toward_free_edge) <= 0.0 || between.dot(b.toward_free_edge) >= 0.0 {
        return None;
    }
    let opening = (between - a.axis * rise).length();
    let fits = opening >= a.reach.max(b.reach) && opening <= a.reach + b.reach + PAIR_GAP_SLACK;
    fits.then_some(opening)
}

/// Pairs the leaves that are double doors; a place that holds no leaf, or a leaf that does not swing, pairs with nothing.
pub fn pair(leaves: &[Option<Leaf>]) -> Vec<Pair> {
    let hinges: Vec<Option<Hinge>> = leaves
        .iter()
        .map(|leaf| leaf.and_then(|leaf| leaf.hinge))
        .collect();
    // Each candidate with how far its opening is from the two reaches together.
    let mut candidates: Vec<(f32, Pair)> = Vec::new();
    for (a, first) in hinges.iter().enumerate() {
        let Some(first) = first else { continue };
        for (b, second) in hinges.iter().enumerate().skip(a + 1) {
            let Some(second) = second else { continue };
            if let Some(opening) = opening_of(first, second) {
                let misfit = (opening - (first.reach + second.reach)).abs();
                candidates.push((misfit, Pair { a, b, opening }));
            }
        }
    }
    // ⚠ Ties fall to the lower places, so what pairs does not depend on the order the candidates were met in.
    candidates.sort_by(|(x, p), (y, q)| x.total_cmp(y).then((p.a, p.b).cmp(&(q.a, q.b))));
    let mut paired = vec![false; leaves.len()];
    let mut pairs = Vec::new();
    for (_, candidate) in candidates {
        if paired[candidate.a] || paired[candidate.b] {
            continue;
        }
        paired[candidate.a] = true;
        paired[candidate.b] = true;
        pairs.push(candidate);
    }
    pairs.sort_by_key(|p| p.a);
    pairs
}
