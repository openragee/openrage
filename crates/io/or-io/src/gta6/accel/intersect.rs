//! The intersection instruction as a pure function of a node's bytes: the reference every other
//! body of it is checked against.
//!
//! The contract is the instruction's; the arithmetic is OpenRage's own, because the hardware's
//! rounding is not public. It is IEEE single precision, one rounded operation where one is
//! written, no product fused into a sum, in the order written here. Every test is in its positive
//! form, so a value that is not a number fails it and the result is a miss.

use super::layout::*;

/// What a triangle that is not hit returns: a distance no ray's extent admits.
pub const MISS: [u32; 4] = [0x7F80_0000, 0x3F80_0000, 0, 0];

/// The least direction component a slab is divided by: the least normal float, 2^-126.
const MIN: f32 = f32::MIN_POSITIVE;
/// The greatest determinant a triangle is kept with, 2^126; the least is [`MIN`].
const DET_MAX: f32 = 1.0 / MIN;
/// The share of a box's greatest distance from the ray's origin it is widened by, 2^-20.
///
/// The triangle test puts a ray inside a triangle it passes within a few units in the last place
/// of, at that distance, which no builder knows; so the test widens the box, not the builder.
const BOX_GROW: f32 = 1.0 / 1_048_576.0;
/// The same for a triangle's own bounds, 2^-21: half a box's, so a box that holds a triangle
/// holds its widened bounds with the rounding of both to spare.
const TRI_GROW: f32 = 1.0 / 2_097_152.0;

/// The dword `field` bytes after byte `at` of the arena; nought past its end.
pub fn word(words: &[u32], at: u32, field: u32) -> u32 {
    (words.get((at.wrapping_add(field) / 4) as usize).copied()).unwrap_or(0)
}

fn float3(words: &[u32], at: u32, field: u32) -> [f32; 3] {
    std::array::from_fn(|k| f32::from_bits(word(words, at, field + 4 * k as u32)))
}

/// One axis of a widened box narrows the stretch of the ray inside it; false where the ray runs
/// along the slab and outside it.
///
/// A direction component below [`MIN`] is not divided by: `1 / 0` is infinite and its product
/// with a plane at the origin's own coordinate is not a number. Such an axis is tested by the
/// origin lying in the closed slab and bounds nothing, so a ray in the plane two boxes share
/// meets both, as the triangle test meets both triangles of a shared edge.
fn narrow(low: f32, high: f32, direction: f32, inverse: f32, stretch: &mut (f32, f32)) -> bool {
    if direction.abs() >= MIN {
        let (p, q) = (low * inverse, high * inverse);
        let (n, f) = if p < q { (p, q) } else { (q, p) };
        if n > stretch.0 {
            stretch.0 = n;
        }
        if f < stretch.1 {
            stretch.1 = f;
        }
        true
    } else {
        low <= 0.0 && high >= 0.0
    }
}

/// The greatest magnitude among `values`, by compares.
fn greatest(values: impl IntoIterator<Item = f32>) -> f32 {
    let mut most = 0.0f32;
    for value in values {
        if value.abs() > most {
            most = value.abs();
        }
    }
    most
}

/// What the instruction returns for the node `node` of the structure at byte `base`.
///
/// A box node gives the pointers of the children whose widened box the ray meets before `t_max`,
/// nearest first and by child number where two are as near, then [`INVALID`]. A triangle gives
/// four floats as bits: the distance's numerator, the denominator, and the numerators of the
/// weights of the two corners its id byte names; or [`MISS`]. The denominator is positive for the
/// face whose corners run counter-clockwise seen from the ray's origin. Any other node gives
/// four [`INVALID`].
///
/// A triangle's denominator is from 2^-126 to 2^126 in size, so its quotients are never not a
/// number, and its distance lies in the stretch of the ray inside the triangle's own bounds, so
/// every box that holds a hit triangle is met no later than the hit.
pub fn intersect_ray(
    words: &[u32],
    base: u32,
    node: u32,
    t_max: f32,
    origin: [f32; 3],
    direction: [f32; 3],
    inverse: [f32; 3],
) -> [u32; 4] {
    let at = base.wrapping_add(offset(node));
    match node & 7 {
        TYPE_BOX32 => {
            let mut found = [(0.0f32, INVALID); 4];
            let mut count = 0;
            for i in 0..4 {
                let child = word(words, at, BOX_CHILDREN + 4 * i);
                if child == INVALID {
                    continue;
                }
                let bounds = BOX_BOUNDS + BOX_BOUNDS_STRIDE * i;
                let (min, max) = (float3(words, at, bounds), float3(words, at, bounds + 12));
                let Some(near) = box_child(min, max, t_max, origin, direction, inverse) else {
                    continue;
                };
                let mut slot = count;
                while slot > 0 && near < found[slot - 1].0 {
                    found[slot] = found[slot - 1];
                    slot -= 1;
                }
                found[slot] = (near, child);
                count += 1;
            }
            found.map(|(_, child)| child)
        }
        k @ 0..=3 => {
            let corners =
                CORNERS[k as usize].map(|n| float3(words, at, TRIANGLE_VERTEX_STRIDE * n));
            let Some([t, determinant, u, v, w]) = triangle(corners, origin, direction, inverse)
            else {
                return MISS;
            };
            // The id byte names the places, in this triangle, of the two corners whose weights
            // the model asks for.
            let id = word(words, at, TRIANGLE_ID) >> (8 * k);
            let weights = [u, v, w];
            let first = weights[(id & 3).min(2) as usize];
            let second = weights[((id >> 2) & 3).min(2) as usize];
            [t, determinant, first, second].map(f32::to_bits)
        }
        _ => [INVALID; 4],
    }
}

/// The distance at which the ray enters a box's child, or `None` where it does not before `t_max`.
fn box_child(
    min: [f32; 3],
    max: [f32; 3],
    t_max: f32,
    origin: [f32; 3],
    direction: [f32; 3],
    inverse: [f32; 3],
) -> Option<f32> {
    let low: [f32; 3] = std::array::from_fn(|k| min[k] - origin[k]);
    let high: [f32; 3] = std::array::from_fn(|k| max[k] - origin[k]);
    // The origin's own coordinates are in it for the rounding of a ray turned into an
    // instance's space, which is of their size.
    let sides = (0..3).flat_map(|k| [low[k], high[k]]);
    let grow = greatest(origin.into_iter().chain(sides)) * BOX_GROW;
    let mut stretch = (f32::NEG_INFINITY, f32::INFINITY);
    for k in 0..3 {
        let (low, high) = (low[k] - grow, high[k] + grow);
        // False for bounds that are not numbers, as an absent child's are, or inside out.
        if !(low <= high && narrow(low, high, direction[k], inverse[k], &mut stretch)) {
            return None;
        }
    }
    let (near, far) = stretch;
    (far >= 0.0 && far >= near && near < t_max).then_some(near)
}

/// A triangle's distance numerator, determinant and three weight numerators, by a test that
/// leaves no crack along an edge two triangles share.
///
/// The corners are taken from the origin and sheared so the ray runs along its greatest axis;
/// each edge then has a signed area against the ray, the difference of two products. An edge's
/// two corners give two triangles the same two products the other way round, so a ray is never
/// outside both, and equal products are on the edge and inside. The side is read by comparing
/// the products, never from the difference's sign, which a fused product would change.
fn triangle(
    corners: [[f32; 3]; 3],
    origin: [f32; 3],
    direction: [f32; 3],
    inverse: [f32; 3],
) -> Option<[f32; 5]> {
    // The axis the ray runs most along, the lowest on a tie.
    let mut kz = 0;
    if direction[1].abs() > direction[0].abs() {
        kz = 1;
    }
    if direction[2].abs() > direction[kz].abs() {
        kz = 2;
    }
    let moving = direction[kz].abs() >= MIN;
    if !moving {
        return None;
    }
    // The other two, exchanged where the ray runs down its axis so a winding stays a winding.
    let (kx, ky) = match direction[kz] < 0.0 {
        false => ((kz + 1) % 3, (kz + 2) % 3),
        true => ((kz + 2) % 3, (kz + 1) % 3),
    };
    let (sx, sy) = (direction[kx] / direction[kz], direction[ky] / direction[kz]);
    let sz = 1.0 / direction[kz];
    // Each corner from the origin on the three axes, then sheared.
    let q = corners.map(|corner| [kx, ky, kz].map(|k| corner[k] - origin[k]));
    let [p0, p1, p2] = q.map(|q| [q[0] - sx * q[2], q[1] - sy * q[2], sz * q[2]]);
    // An edge's two products, the edge opposite corner 0 first.
    let products = [
        (p2[0] * p1[1], p2[1] * p1[0]),
        (p0[0] * p2[1], p0[1] * p2[0]),
        (p1[0] * p0[1], p1[1] * p0[0]),
    ];
    let inside = products.iter().all(|(a, b)| a >= b) || products.iter().all(|(a, b)| a <= b);
    if !inside {
        return None;
    }
    let [u, v, w] = products.map(|(a, b)| a - b);
    let determinant = (u + v) + w;
    if !(determinant.abs() >= MIN && determinant.abs() <= DET_MAX) {
        return None;
    }
    let mut numerator = (u * p0[2] + v * p1[2]) + w * p2[2];
    let finite = numerator.abs() <= f32::MAX;
    if !finite {
        return None;
    }
    // The triangle's own bounds from the origin, widened, and the stretch of the ray inside
    // them: a ray that passes beside them is no hit, however thin a triangle points at it.
    let mut stretch = (f32::NEG_INFINITY, f32::INFINITY);
    let mut bounds = [(0.0f32, 0.0f32); 3];
    for (j, bound) in bounds.iter_mut().enumerate() {
        let (mut low, mut high) = (q[0][j], q[0][j]);
        for corner in &q[1..] {
            if corner[j] < low {
                low = corner[j];
            }
            if corner[j] > high {
                high = corner[j];
            }
        }
        *bound = (low, high);
    }
    let grow = greatest(bounds.iter().flat_map(|(low, high)| [*low, *high])) * TRI_GROW;
    for (j, k) in [kx, ky, kz].into_iter().enumerate() {
        let (low, high) = (bounds[j].0 - grow, bounds[j].1 + grow);
        if !narrow(low, high, direction[k], inverse[k], &mut stretch) {
            return None;
        }
    }
    let (near, far) = stretch;
    let crossed = far >= near;
    if !crossed {
        return None;
    }
    // The distance brought into that stretch, so no box of a holding box contradicts it,
    // whatever the two roundings.
    let t = numerator / determinant;
    if t < near {
        numerator = near * determinant;
    } else if t > far {
        numerator = far * determinant;
    }
    Some([numerator, determinant, u, v, w])
}

#[cfg(test)]
#[path = "intersect_tests.rs"]
mod tests;
