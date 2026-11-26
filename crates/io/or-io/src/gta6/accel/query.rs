//! The device's ray query under the game's walk, on the CPU: what a query would hand over for a
//! ray, and the instruction's answers made from it, so a walk that is answered this way can be
//! held to the same walk answered from the arena's bytes.
//!
//! The walk is not told. It starts at a box the header states as the root, whose answer is the
//! real root and a second box to come back to; each time it comes back the query gives its next
//! candidate, and the walk is led down to that candidate's triangle by answers that name one
//! child each. The mask, the faces, the extent and the commit stay the walk's own.
//!
//! A candidate is stated in the device's own numbers, and the arena is laid out so that they
//! lead to its nodes with no table: an instance's number is its node's place among the instance
//! nodes, which every top-level box states a range of for each child, and a triangle's number is
//! twice its node's unit, and one more for the second triangle of the node.

use super::intersect::{word, MISS};
use super::layout::*;
use super::walk::{candidates_as, leaves, quotient, Form, Hit, Probe, Ray};

/// One triangle a query hands over, as a device states it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    /// The instance's number in the top level the device was given: see [`instance_nodes`].
    pub instance: u32,
    /// The triangle's number in the bottom level the device was given: see [`device_triangles`].
    pub primitive: u32,
    pub t: f32,
    pub front: bool,
    /// The weights of the triangle's second and third corner, as the device was given them.
    pub weights: [f32; 2],
}

impl Shown {
    /// The triangle's node pointer in its bottom level, the second of a pair as type 1.
    pub fn leaf(&self) -> u32 {
        pointer(self.primitive >> 1, self.primitive & 1)
    }
}

/// The two boxes the protocol walks through, where the header of the top level at `top` states
/// the first as its root: the second is known by being its own second child.
pub fn stated_boxes(words: &[u32], top: u32) -> Option<[u32; 2]> {
    let second = |node: u32| word(words, top.wrapping_add(offset(node)), BOX_CHILDREN + 4);
    let first = word(words, top, HEADER_ROOT);
    let again = second(first);
    (is_box(first) && is_box(again) && second(again) == again).then_some([first, again])
}

/// The root of the tree of the top level at `top`, whichever root its header states.
fn tree_root(words: &[u32], top: u32) -> u32 {
    match stated_boxes(words, top) {
        Some([first, _]) => word(words, top.wrapping_add(offset(first)), BOX_CHILDREN),
        None => word(words, top, HEADER_ROOT),
    }
}

/// The instance nodes of the top level at `top` in the order of their numbers, which is the
/// order a device is to be given the instances in: a child before the next, as they were placed.
pub fn instance_nodes(words: &[u32], top: u32) -> Vec<u32> {
    leaves(words, top, tree_root(words, top))
}

/// The triangles of the bottom level at `base` as a device is to be given them, in one geometry:
/// two a unit of the structure, a triangle of no size where a unit holds none, each with its
/// corners in the model's order, so that a device's two weights are the instruction's.
pub fn device_triangles(words: &[u32], base: u32) -> Vec<[[f32; 3]; 3]> {
    let units = word(words, base, HEADER_PARENTS) / UNIT;
    let mut triangles = vec![[[0.0; 3]; 3]; 2 * units as usize];
    for leaf in leaves(words, base, word(words, base, HEADER_ROOT)) {
        let at = base.wrapping_add(offset(leaf));
        let pair = word(words, at, TRIANGLE_WORD) & TRIANGLE_PAIR != 0;
        for k in 0..if pair { 2 } else { 1 } {
            let id = word(words, at, TRIANGLE_ID) >> (8 * k);
            // The places in the node's triangle of the model's three corners.
            let (second, third) = (id & 3, (id >> 2) & 3);
            let places = [3 - second - third, second, third];
            let slot = 2 * (leaf >> 3) + k;
            triangles[slot as usize] = places.map(|place| {
                let vertex = CORNERS[k as usize][place as usize];
                let field = TRIANGLE_VERTEX_STRIDE * vertex;
                std::array::from_fn(|c| f32::from_bits(word(words, at, field + 4 * c as u32)))
            });
        }
    }
    triangles
}

/// What a device's query hands over for `ray`: every triangle its line meets after nought and
/// before its far end, of every instance whatever its mask, with no face left out, each instance
/// in the order of its node. A device shows no triangle at either end of a ray.
///
/// ⚠ The distances are the software instruction's under the walk of `form`, so a walk answered
/// from them commits the same bits; a device's own differ in their last places.
pub fn handed_over(words: &[u32], top: u32, ray: &Ray, form: Form) -> Vec<Shown> {
    let far = ray.t_max;
    let nodes = instance_nodes(words, top);
    candidates_as(words, top, ray, 0xFF, form)
        .into_iter()
        .filter_map(|candidate| {
            let [numerator, denominator, first, second] = candidate.result.map(f32::from_bits);
            let reciprocal = 1.0 / denominator;
            let t = quotient(form, reciprocal, numerator);
            // The per-lane walk multiplies its weights out plainly.
            let weigh = |x: f32| match form {
                Form::Lane => reciprocal * x,
                Form::Wave | Form::Plain => quotient(form, reciprocal, x),
            };
            let instance = nodes.iter().position(|node| *node == candidate.instance)?;
            (t > 0.0 && t < far).then_some(Shown {
                instance: instance as u32,
                primitive: 2 * (candidate.leaf >> 3) + (candidate.leaf & 1),
                t,
                front: denominator > 0.0,
                weights: [first, second].map(weigh),
            })
        })
        .collect()
}

/// `shown` in another order, which `seed` picks: a device hands its candidates over in no
/// order a walk may rely on.
pub fn shuffled(mut shown: Vec<Shown>, seed: u32) -> Vec<Shown> {
    let mut state = seed;
    for last in (1..shown.len()).rev() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        shown.swap(
            last,
            ((u64::from(state) * (last as u64 + 1)) >> 32) as usize,
        );
    }
    shown
}

/// How the hit of a walk answered from a query stands to the same walk's from the arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreed {
    /// The same to every bit, or no hit in either.
    Same,
    /// Another triangle at the same distance to the bit: the one the device showed first, where
    /// the arena's walk took the one its own order came to first.
    Tied,
    /// The arena's hit lies at nought, where a device shows no triangle.
    AtNought,
    Apart,
}

/// How `query`, the hit of a walk answered from a query, stands to `arena`, the same walk's hit
/// answered from the arena.
pub fn agreed(arena: Option<&Hit>, query: Option<&Hit>) -> Agreed {
    match (arena, query) {
        (arena, query) if arena == query => Agreed::Same,
        (Some(arena), _) if arena.t == 0.0 => Agreed::AtNought,
        (Some(arena), Some(query)) if arena.t.to_bits() == query.t.to_bits() => Agreed::Tied,
        _ => Agreed::Apart,
    }
}

/// One lane's query: what the device hands over for the lane's ray, what of it is yet to be
/// shown, the candidate the walk is being led to, and the distance the walk was seen to commit.
#[derive(Debug, Clone)]
pub struct Query {
    /// The last is handed over first.
    handed: Vec<Shown>,
    waiting: Vec<Shown>,
    /// The candidate in hand, and whether its triangle's answer was given.
    pending: Option<(Shown, bool)>,
    committed: f32,
    /// Whether a candidate the walk committed is confirmed to the device, which then withholds
    /// what lies no nearer. A device may show such a triangle all the same, and one that is
    /// never told does: the walk's own compare refuses it.
    pub confirms: bool,
    /// The queries started, the candidates handed over, the candidates confirmed.
    pub counts: [u32; 3],
    /// The tests of a top-level box that is not on the pending candidate's way, which a walk
    /// that keeps each lane to its own children never makes.
    pub astray: u32,
}

/// The child of the top-level box `node` whose range of instances holds the instance numbered
/// `instance`.
fn child_holding(words: &[u32], top: u32, node: u32, instance: u32) -> Option<u32> {
    let at = top.wrapping_add(offset(node));
    (0..4).find_map(|i| {
        let child = word(words, at, BOX_CHILDREN + 4 * i);
        let range = word(words, at, BOX_INSTANCES + 4 * i);
        let holds = (range & 0xFFFF) <= instance && instance <= range >> 16;
        (child != INVALID && holds).then_some(child)
    })
}

impl Query {
    /// A query that hands over `handed`, the candidates of the ray its walk traces.
    pub fn new(handed: Vec<Shown>) -> Self {
        Self {
            handed,
            waiting: Vec::new(),
            pending: None,
            committed: f32::INFINITY,
            confirms: true,
            counts: [0; 3],
            astray: 0,
        }
    }

    /// The next candidate nearer than the distance confirmed, as the first protocol box's two
    /// children; four misses once there is none.
    fn next(&mut self, root: u32, again: u32) -> [u32; 4] {
        self.pending = None;
        while let Some(shown) = self.waiting.pop() {
            if shown.t < self.committed {
                self.pending = Some((shown, false));
                self.counts[1] += 1;
                return [root, again, INVALID, INVALID];
            }
        }
        [INVALID; 4]
    }

    /// What the instruction answers `probe`, asked by a walk of the top level at `top` whose
    /// header states the protocol's first box.
    pub fn answer(&mut self, words: &[u32], top: u32, probe: &Probe) -> [u32; 4] {
        let Some([first, again]) = stated_boxes(words, top) else {
            return [INVALID; 4];
        };
        let root = word(words, top.wrapping_add(offset(first)), BOX_CHILDREN);
        if probe.base == top {
            if probe.node == first {
                // The query starts: its far end is the walk's far operand.
                self.waiting = self.handed.clone();
                self.committed = probe.t_max;
                self.counts[0] += 1;
                return self.next(root, again);
            }
            if probe.node == again {
                // The walk shortened its ray to the candidate shown: it committed it.
                if let (Some((shown, true)), true) = (self.pending, self.confirms) {
                    if probe.t_max.to_bits() == shown.t.to_bits() {
                        self.committed = probe.t_max;
                        self.counts[2] += 1;
                    }
                }
                return self.next(root, again);
            }
            if !is_box(probe.node) {
                return [INVALID; 4];
            }
            // The one child whose instances hold the candidate's.
            let pending = self.pending.map(|(shown, _)| shown.instance);
            let child =
                pending.and_then(|instance| child_holding(words, top, probe.node, instance));
            if child.is_none() {
                self.astray += 1;
            }
            return [child.unwrap_or(INVALID), INVALID, INVALID, INVALID];
        }
        // A bottom level: a box names the candidate's triangle, and that triangle answers once.
        let pending = self.pending.as_mut().filter(|(shown, _)| {
            let mut node = root;
            while is_box(node) {
                match child_holding(words, top, node, shown.instance) {
                    Some(child) => node = child,
                    None => return false,
                }
            }
            probe.base == word(words, top.wrapping_add(offset(node)), INSTANCE_BOTTOM) << 8
        });
        match (probe.node & 7, pending) {
            (TYPE_BOX16 | TYPE_BOX32, Some((shown, _))) => {
                [shown.leaf(), INVALID, INVALID, INVALID]
            }
            (0..=3, Some((shown, given))) if !*given && probe.node == shown.leaf() => {
                *given = true;
                let side = if shown.front { 1.0 } else { -1.0 };
                let [first, second] = shown.weights;
                [shown.t * side, side, first * side, second * side].map(f32::to_bits)
            }
            (0..=3, _) => MISS,
            _ => [INVALID; 4],
        }
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
