//! The reference walks over an arena: the two walks the game's programs run around the
//! intersection instruction, and the instruction run on every triangle to hold them to.
//!
//! The per-lane walk keeps a short stack that wraps: a child pushed past the ring's size
//! overwrites the oldest, and a pop that finds its entry gone climbs by the parent table instead.
//! The wave-shared walk takes the top level as one wave, with a bounded stack of pairs of a child
//! and the lanes that want it, and leaves each lane an instance's bottom level to itself. Its
//! ray flags decide what a triangle's opacity and face are worth, and its candidate code whether
//! a triangle that is not opaque is committed.

use super::intersect::{intersect_ray, word, MISS};
use super::layout::*;
use super::query::instance_nodes;

/// A ray: it reaches from `t_min` to `t_max` times its direction, which need not be a unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub t_min: f32,
    pub t_max: f32,
}

/// The faces a walk leaves out, before an instance's flags turn or drop the choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cull {
    #[default]
    None,
    Back,
    Front,
}

/// Whose word decides that a triangle is opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Opacity {
    /// The instance's force flags, else the geometry's bit.
    #[default]
    Instance,
    Opaque,
    NonOpaque,
}

/// What one walk is specialised by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    /// The stack's entries a lane, a power of two: 16 in the programs. Fewer than four cannot
    /// hold the three children a box leaves waiting, and loses one.
    pub ring: u32,
    /// The lane of a wave of 32, which sets the low bits of the stack word.
    pub lane: u32,
    /// An instance is entered where its mask shares a bit with this.
    pub mask: u8,
    pub cull: Cull,
    pub opacity: Opacity,
    /// The walk ends at the first hit it commits; otherwise it goes on for the closest.
    pub first_hit: bool,
    /// The walk ends at the turn whose number reaches this.
    pub bound: u32,
}

impl Default for Walk {
    fn default() -> Self {
        Self {
            ring: 16,
            lane: 0,
            mask: 0xFF,
            cull: Cull::None,
            opacity: Opacity::Instance,
            first_hit: false,
            bound: 0xFFFF,
        }
    }
}

/// A triangle a ray meets inside its extent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub t: f32,
    /// The triangle's node pointer in its bottom level.
    pub leaf: u32,
    /// The instance node's pointer in the top level.
    pub instance: u32,
    /// 0xFE for a front face and 0xFF for a back one, after the instance's winding flag.
    pub kind: u8,
    /// The weights of the two corners the triangle's id byte names: the model's second and third.
    pub weights: [f32; 2],
}

/// How a walk ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outcome {
    pub hit: Option<Hit>,
    /// The intersection instructions run.
    pub turns: u32,
    /// The stack word as the walk left it: the address in the low bits, the depth above bit 7,
    /// the depth at which the instance was entered above bit 20.
    pub stack: u32,
    /// The pops that found their entry overwritten and climbed.
    pub climbs: u32,
    /// The greatest depth the stack word reached.
    pub deepest: u32,
}

/// A ray flag: every triangle is opaque, whatever its instance or its geometry says.
pub const RAY_FORCE_OPAQUE: u32 = 0x01;
/// A ray flag: no triangle is opaque.
pub const RAY_FORCE_NON_OPAQUE: u32 = 0x02;
/// A ray flag: back faces are left out, unless the instance culls none.
pub const RAY_CULL_BACK: u32 = 0x10;
/// A ray flag: front faces are left out; it wins where [`RAY_CULL_BACK`] is set too.
pub const RAY_CULL_FRONT: u32 = 0x20;
/// A ray flag: an opaque triangle is passed over, and the second triangle of its node with it.
pub const RAY_SKIP_OPAQUE: u32 = 0x40;
/// A ray flag: a triangle that is not opaque is passed over, and the second of its node with it.
pub const RAY_SKIP_NON_OPAQUE: u32 = 0x80;

fn depth(stack: u32) -> u32 {
    (stack >> 7) & 0xFFF
}

/// One run of the intersection instruction, as a walk asks it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    /// The byte address of the structure the node is of.
    pub base: u32,
    pub node: u32,
    pub t_max: f32,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub inverse: [f32; 3],
}

impl Probe {
    /// What the instruction answers from the bytes of the arena `words`.
    pub fn software(&self, words: &[u32]) -> [u32; 4] {
        let Self {
            base,
            node,
            t_max,
            origin,
            direction,
            inverse,
        } = *self;
        intersect_ray(words, base, node, t_max, origin, direction, inverse)
    }
}

/// Which walk's own arithmetic turns a ray into an instance's space and divides a distance out:
/// the three differ in what they fuse and in what order, and so in the last bits of a distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Form {
    /// The per-lane walk: a row from its first term, the translation added after.
    Lane,
    /// The wave-shared walk whose sums are fused: a row from its diagonal or middle term, an
    /// origin's first product fused onto the translation.
    #[default]
    Wave,
    /// The wave-shared walk that fuses nothing: a row from its middle term, every product
    /// rounded, the translation added last, and a distance by a plain product.
    Plain,
}

/// `reciprocal * x` as the walk of `form` multiplies a result out: nought where either factor is,
/// but in the walk that fuses nothing.
pub(super) fn quotient(form: Form, reciprocal: f32, x: f32) -> f32 {
    match form {
        Form::Plain => reciprocal * x,
        Form::Lane | Form::Wave => legacy(reciprocal, x),
    }
}

/// The ray in the space of the instance node at `at`, as the walk of `form` turns it.
fn turned_as(
    form: Form,
    words: &[u32],
    at: u32,
    origin: [f32; 3],
    direction: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    match form {
        Form::Lane => turned(words, at, origin, direction),
        Form::Wave => turned_wave(words, at, origin, direction),
        Form::Plain => turned_plain(words, at, origin, direction),
    }
}

/// The low 29 bits of a sorted child as a node pointer, -1 for none.
fn child(key: u32) -> i32 {
    ((key << 3) as i32) >> 3
}

/// A product that is nought where either factor is, whatever the other.
fn legacy(a: f32, b: f32) -> f32 {
    if a == 0.0 || b == 0.0 {
        0.0
    } else {
        a * b
    }
}

/// The ray in the space of the instance node at `at`: rotated unless the node says its rotation
/// is the identity, then moved. A row rounds its first product alone: the other two are fused
/// onto it.
fn turned(words: &[u32], at: u32, origin: [f32; 3], direction: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let rotate = |v: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|row| {
            let m = |k: u32| {
                f32::from_bits(word(words, at, INSTANCE_ROTATION + 12 * row as u32 + 4 * k))
            };
            m(2).mul_add(v[2], m(1).mul_add(v[1], legacy(m(0), v[0])))
        })
    };
    let (mut o, mut d) = (origin, direction);
    if word(words, at, INSTANCE_ROOT) & INSTANCE_IDENTITY == 0 {
        (o, d) = (rotate(origin), rotate(direction));
    }
    for (k, x) in o.iter_mut().enumerate() {
        *x += f32::from_bits(word(words, at, INSTANCE_TRANSLATION + 4 * k as u32));
    }
    (o, d)
}

/// One lane's walk of the top level at byte `top` of the arena `words`, `candidate` committing or
/// not each hit on a triangle that is not opaque.
pub fn walk_lane(
    words: &[u32],
    top: u32,
    ray: &Ray,
    walk: &Walk,
    candidate: &mut dyn FnMut(&Hit) -> bool,
) -> Outcome {
    let mut body = |probe: &Probe| probe.software(words);
    walk_lane_with(words, top, ray, walk, &mut body, candidate)
}

/// [`walk_lane`] with the instruction answered by `body` in place of the arena's bytes.
pub fn walk_lane_with(
    words: &[u32],
    top: u32,
    ray: &Ray,
    walk: &Walk,
    body: &mut dyn FnMut(&Probe) -> [u32; 4],
    candidate: &mut dyn FnMut(&Hit) -> bool,
) -> Outcome {
    walk_lane_among(words, top, ray, walk, 32, body, candidate)
}

/// [`walk_lane_with`] for a lane of a wave of `lanes` lanes, 32 or 64: a stack entry is four bytes
/// a lane, so the stack word of a wave of 64 steps by twice as much.
pub fn walk_lane_among(
    words: &[u32],
    top: u32,
    ray: &Ray,
    walk: &Walk,
    lanes: u32,
    body: &mut dyn FnMut(&Probe) -> [u32; 4],
    candidate: &mut dyn FnMut(&Hit) -> bool,
) -> Outcome {
    // ⚠ The depth is read from bit 7 at either width. In a wave of 64 it is twice the entries
    // held, and for a lane of 32 or more it holds the lane's top bit and is never nought: such a
    // lane does not end on an empty stack, it climbs by the parent table to the root.
    #[derive(PartialEq)]
    enum State {
        Walk,
        Leave,
        Done,
    }
    let read = |at: u32, field: u32| word(words, at, field);
    // The base a node's own structure is read from, and the bias of its parent table.
    let table = |bvh: u32| {
        let base = bvh << 8;
        (read(base, HEADER_FIRST_NODE) >> 6).wrapping_sub(read(base, HEADER_PARENTS) >> 2)
    };
    let entries = walk.ring.clamp(1, 4096).next_power_of_two();
    let step = lanes.clamp(32, 64).next_power_of_two() * 4;
    let mask = (entries * step - 1) & !3;
    let slot = |stack: u32| ((stack & mask) / step) as usize;
    let mut ring = vec![0u32; entries as usize];

    let mut node = read(top, HEADER_ROOT) as i32;
    // The last box or instance gone down into.
    let mut previous = node;
    let mut bvh = top >> 8;
    // Nought while the parent table of `bvh` has not been asked for.
    let mut bias = 0u32;
    let mut instance = 0i32;
    let (mut origin, mut direction) = (ray.origin, ray.direction);
    let mut t_max = ray.t_max;
    let mut stack = walk.lane * 4;
    let mut waiting = [INVALID; 3];
    // What the instance being walked sets: its winding bit, its cull sign, its opacity word.
    let (mut winding, mut cull, mut forced) = (0u8, 0i32, 0u32);
    let mut out = Outcome {
        hit: None,
        turns: 0,
        stack,
        climbs: 0,
        deepest: 0,
    };
    ring[slot(stack)] = INVALID;
    let mut count = 0u32;
    loop {
        let mut state = State::Walk;
        if node <= 0 {
            let popped = std::mem::replace(&mut ring[slot(stack)], INVALID) as i32;
            if popped > 0 {
                // An entry pushed in the top level ends the instance.
                let back = depth(stack) == stack >> 20;
                stack = stack.wrapping_sub(step);
                if back {
                    stack &= 0x7_FFFF;
                    state = State::Leave;
                }
                if popped & 4 != 0 {
                    previous = popped;
                }
                node = popped;
            } else if depth(stack) == 0 {
                state = State::Done;
            } else {
                // The ring overwrote the entry: go on from the parent of the node last entered.
                out.climbs += 1;
                stack = (stack & mask) | 0x4_0000;
                if bias == 0 {
                    bias = table(bvh);
                }
                let parent = ((previous as u32) >> 3).wrapping_sub(bias).wrapping_mul(4);
                node = read(bvh << 8, parent) as i32;
                if node <= 0 {
                    state = State::Done;
                    if instance > 0 {
                        bias = table(top >> 8);
                        let at = top.wrapping_add(offset(instance as u32));
                        node = read(at, INSTANCE_PARENT) as i32;
                        state = State::Leave;
                    }
                    previous = instance;
                }
            }
        }
        count += 1;
        if count >= walk.bound {
            state = State::Done;
        }
        if state == State::Leave {
            bvh = top >> 8;
            (origin, direction) = (ray.origin, ray.direction);
            instance = 0;
            state = State::Walk;
        }
        if state != State::Walk {
            break;
        }

        out.turns += 1;
        let pointer = node as u32;
        let result = body(&Probe {
            base: bvh << 8,
            node: pointer,
            t_max,
            origin,
            direction,
            inverse: direction.map(|x| 1.0 / x),
        });
        let ty = pointer & 7;
        let at = (bvh << 8).wrapping_add(offset(pointer));
        if ty == TYPE_INSTANCE {
            (origin, direction) = (ray.origin, ray.direction);
            instance = node;
        }
        // The children of the box before, the greatest key first so the least is popped first.
        waiting.sort_unstable();
        for key in waiting.iter().rev().filter(|key| **key != INVALID) {
            stack = stack.wrapping_add(step);
            ring[slot(stack)] = key & 0x1FFF_FFFF;
            out.deepest = out.deepest.max(depth(stack));
        }
        waiting = [INVALID; 3];

        if ty & 6 == 6 {
            let mut next = -1;
            let id_mask = read(at, INSTANCE_ID_MASK);
            let flags = read(at, INSTANCE_CONTRIBUTION_FLAGS) >> 24;
            if ty == TYPE_INSTANCE && (id_mask >> 24) & u32::from(walk.mask) != 0 {
                let root = read(at, INSTANCE_ROOT) & INSTANCE_NO_ROOT;
                if root != INSTANCE_NO_ROOT {
                    next = root as i32;
                }
                // The bottom level's root is also the node gone down into: a box met with any
                // other `previous` is taken as one come back up to, and loses its children.
                previous = next;
                stack |= (stack << 13) & 0xFFF0_0000;
                (origin, direction) = turned(words, at, ray.origin, ray.direction);
                winding = (flags & FLAG_FLIP_WINDING != 0) as u8;
                let side = match (flags & FLAG_NO_CULL != 0, winding) {
                    (true, _) => 0,
                    (false, 0) => 1,
                    (false, _) => -1,
                };
                cull = side
                    * match walk.cull {
                        Cull::None => 0,
                        Cull::Back => 1,
                        Cull::Front => -1,
                    };
                forced = match walk.opacity {
                    Opacity::Instance => (flags >> 2) & 3,
                    Opacity::Opaque => 1,
                    Opacity::NonOpaque => 2,
                };
                bias = 0;
                bvh = read(at, INSTANCE_BOTTOM);
            }
            node = next;
        } else if ty < 4 {
            let tail = read(at, TRIANGLE_WORD);
            let next = match ty == 0 && tail & TRIANGLE_PAIR != 0 {
                true => node | 1,
                false => -1,
            };
            let [numerator, denominator, first, second] = result.map(f32::from_bits);
            // The programs' own quotient: a reciprocal, then a product that is nought with nought.
            let reciprocal = 1.0 / denominator;
            let t = quotient(Form::Lane, reciprocal, numerator);
            let side = (denominator > 0.0) as i32 - (denominator < 0.0) as i32;
            // A distance that is not a number would pass these two compares; the instruction
            // returns none.
            let outside = t < ray.t_min || t >= t_max;
            if !outside && cull * side >= 0 {
                let hit = Hit {
                    t,
                    leaf: pointer,
                    instance: instance as u32,
                    kind: (if denominator > 0.0 { 0xFE } else { 0xFF }) ^ winding,
                    weights: [reciprocal * first, reciprocal * second],
                };
                if (tail & TRIANGLE_OPAQUE) | forced == 1 || candidate(&hit) {
                    t_max = t;
                    out.hit = Some(hit);
                    if walk.first_hit {
                        state = State::Done;
                    }
                }
            }
            node = next;
        } else {
            // Triangles before boxes and instances, each in the instruction's order.
            let mut keys = [0, 1, 2, 3].map(|i| match result[i] {
                INVALID => INVALID,
                found => found | (((found & 4) | i as u32) << 29),
            });
            keys.sort_unstable();
            let [mut first, q0, q1, q2] = keys;
            if previous == node {
                waiting = [q0, q1, q2];
            } else if child(first) == previous {
                first = q0;
                waiting = [q1, q2, INVALID];
            } else if child(q0) == previous {
                first = q1;
                waiting = [q2, INVALID, INVALID];
            } else if child(q1) == previous {
                first = q2;
            } else {
                first = INVALID;
            }
            let next = child(first);
            // A box or an instance has bit 31 of its key set and is not the key of no child.
            previous = if (first as i32) < -1 { next } else { node };
            node = next;
        }
        if state != State::Walk {
            break;
        }
    }
    out.stack = stack;
    out
}

/// The nodes that are no box under the node `pointer` of the structure at `base`, a child before
/// the next.
pub(super) fn leaves(words: &[u32], base: u32, pointer: u32) -> Vec<u32> {
    if !is_box(pointer) {
        return vec![pointer];
    }
    let at = base.wrapping_add(offset(pointer));
    let children = (0..4).map(|i| word(words, at, BOX_CHILDREN + 4 * i));
    let below = children.filter(|child| *child != INVALID);
    below.flat_map(|child| leaves(words, base, child)).collect()
}

/// One triangle a ray's line meets, as the instruction returned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub leaf: u32,
    pub instance: u32,
    /// The instruction's four dwords.
    pub result: [u32; 4],
}

/// Every triangle the line of `ray` meets, of every instance whose mask shares a bit with
/// `mask`: no box is tested and no extent applied, each instance in the order of its node.
pub fn candidates(words: &[u32], top: u32, ray: &Ray, mask: u8) -> Vec<Candidate> {
    candidates_as(words, top, ray, mask, Form::Lane)
}

/// [`candidates`] with the ray turned into each instance's space as the walk of `form` turns it.
pub fn candidates_as(words: &[u32], top: u32, ray: &Ray, mask: u8, form: Form) -> Vec<Candidate> {
    let mut found = Vec::new();
    for instance in instance_nodes(words, top) {
        let at = top.wrapping_add(offset(instance));
        let root = word(words, at, INSTANCE_ROOT) & INSTANCE_NO_ROOT;
        let admitted = (word(words, at, INSTANCE_ID_MASK) >> 24) & u32::from(mask) != 0;
        if instance & 7 != TYPE_INSTANCE || !admitted || root == INSTANCE_NO_ROOT {
            continue;
        }
        let base = word(words, at, INSTANCE_BOTTOM) << 8;
        let (origin, direction) = turned_as(form, words, at, ray.origin, ray.direction);
        let inverse = direction.map(|x| 1.0 / x);
        for leaf in leaves(words, base, root) {
            let tail = word(words, base.wrapping_add(offset(leaf)), TRIANGLE_WORD);
            let pair = tail & TRIANGLE_PAIR != 0;
            for leaf in [Some(leaf), pair.then_some(leaf | 1)].into_iter().flatten() {
                let result =
                    intersect_ray(words, base, leaf, f32::INFINITY, origin, direction, inverse);
                if result != MISS {
                    found.push(Candidate {
                        leaf,
                        instance,
                        result,
                    });
                }
            }
        }
    }
    found
}

/// What a wave-shared walk is specialised by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wave {
    /// The lanes of a wave, a power of two: 32 in every program that walks this way. A wave of
    /// 64 is OpenRage's own widening of it: a pair's lane word is then 64 bits.
    pub lanes: u32,
    /// The entries of a lane's own stack inside an instance, a power of two: 8 in the programs.
    pub ring: u32,
    /// The dwords of the wave's stack of (child, lanes) pairs: 256 in the programs, 128 pairs. A
    /// push past it is dropped and its child never visited.
    pub stack: u32,
    /// An instance is entered where its mask shares a bit with this.
    pub mask: u8,
    /// The ray flags the walk was compiled with, the `RAY_*` bits; a lane's own are added to them.
    pub flags: u32,
    pub form: Form,
    pub near: Near,
    /// A lane is done at the first hit it commits, and leaves the wave when it leaves the
    /// instance; otherwise it goes on for the closest.
    pub first_hit: bool,
    /// A lane is done at the turn inside an instance that brings its count to this.
    pub bound: u32,
}

/// The near end of a wave's rays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Near {
    /// Nought whatever the ray states: the walk compares with a literal.
    #[default]
    Nought,
    /// Each lane's own ray's.
    Ray,
}

impl Default for Wave {
    fn default() -> Self {
        Self {
            lanes: 32,
            ring: 8,
            stack: 256,
            mask: 0xFF,
            flags: 0,
            form: Form::Wave,
            near: Near::Nought,
            first_hit: false,
            bound: 0xFFFF,
        }
    }
}

/// How a wave's walk ended.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveOutcome {
    /// One a lane; `None` for a lane with no ray or no hit. ⚠ The instance is stated for every
    /// hit; a program may keep the leaf and the distance alone.
    pub hits: Vec<Option<Hit>>,
    /// Each lane's turn count: the wave's turns in the top level, whether or not the lane is
    /// still tracing, and the lane's own below it.
    pub counts: Vec<u32>,
    /// Each lane's own stack word as it left the last instance it walked: the byte address of
    /// its top entry, below nought while it holds none. A program keeps it no longer than that.
    pub stacks: Vec<i32>,
    /// The wave's turns in the top level.
    pub turns: u32,
    /// The most pairs the wave's stack held.
    pub deepest: u32,
    /// The pushes dropped at the stack's bound.
    pub dropped: u32,
    /// The most entries a lane's own stack held, and the pops that climbed instead.
    pub ring_deepest: u32,
    pub climbs: u32,
}

/// The ray in the space of the instance node at `at`, in the order the wave-shared walk computes
/// it: each row starts from its diagonal term, or from its middle term in the last row, and the
/// other two are fused onto it. A direction's first product is rounded; an origin's is fused
/// onto the translation.
fn turned_wave(
    words: &[u32],
    at: u32,
    origin: [f32; 3],
    direction: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let m =
        |row: u32, k: u32| f32::from_bits(word(words, at, INSTANCE_ROTATION + 12 * row + 4 * k));
    let t = |k: u32| f32::from_bits(word(words, at, INSTANCE_TRANSLATION + 4 * k));
    if word(words, at, INSTANCE_ROOT) & INSTANCE_IDENTITY != 0 {
        return (std::array::from_fn(|k| t(k as u32) + origin[k]), direction);
    }
    // The term each row starts from, then the other two in their order.
    const ORDER: [[usize; 3]; 3] = [[0, 1, 2], [1, 0, 2], [1, 0, 2]];
    let d = std::array::from_fn(|row| {
        let [a, b, c] = ORDER[row];
        let r = row as u32;
        let first = legacy(m(r, a as u32), direction[a]);
        m(r, c as u32).mul_add(direction[c], m(r, b as u32).mul_add(direction[b], first))
    });
    let o = std::array::from_fn(|row| {
        let [a, b, c] = ORDER[row];
        let r = row as u32;
        let first = m(r, a as u32).mul_add(origin[a], t(r));
        m(r, c as u32).mul_add(origin[c], m(r, b as u32).mul_add(origin[b], first))
    });
    (o, d)
}

/// The ray in the space of the instance node at `at`, as the walk that fuses nothing computes
/// it: each row from its middle term, every product rounded, the translation added last.
fn turned_plain(
    words: &[u32],
    at: u32,
    origin: [f32; 3],
    direction: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let m =
        |row: u32, k: u32| f32::from_bits(word(words, at, INSTANCE_ROTATION + 12 * row + 4 * k));
    let t = |k: u32| f32::from_bits(word(words, at, INSTANCE_TRANSLATION + 4 * k));
    if word(words, at, INSTANCE_ROOT) & INSTANCE_IDENTITY != 0 {
        return (std::array::from_fn(|k| t(k as u32) + origin[k]), direction);
    }
    let row = |r: u32, v: [f32; 3]| m(r, 1) * v[1] + m(r, 0) * v[0] + m(r, 2) * v[2];
    (
        std::array::from_fn(|r| t(r as u32) + row(r as u32, origin)),
        std::array::from_fn(|r| row(r as u32, direction)),
    )
}

/// The cull sign of ray flags: 1 where back faces are left out, -1 where front faces are.
fn cull_of(flags: u32) -> i32 {
    // Two bits, signed, and no less than -1.
    ((((flags >> 4) & 3) as i32) << 30 >> 30).max(-1)
}

/// The key a wave orders a box's children by: how many lanes named the child first, second,
/// third and fourth, as digits of base 32 cut to 24 bits before each is added.
// A wave of 64 lanes can count 32 or more in one place, which then carries into the place before.
fn vote(named: [u32; 4]) -> u64 {
    (named.into_iter()).fold(0, |key, count| (key & 0xFF_FFFF) * 32 + u64::from(count))
}

/// A lane of a wave inside one instance.
struct Lane {
    lane: usize,
    /// The walk's ray flags with the lane's own.
    flags: u32,
    /// The cull sign after the instance's flags, and the opacity word: 1 forces every triangle
    /// opaque, 2 and 3 none.
    cull: i32,
    forced: u32,
    origin: [f32; 3],
    direction: [f32; 3],
    inverse: [f32; 3],
    /// Below nought while the lane's own stack is empty.
    stack: i32,
    node: i32,
    previous: i32,
    waiting: [u32; 3],
}

/// What the lanes of a wave share inside one instance.
struct Below<'a> {
    words: &'a [u32],
    wave: &'a Wave,
    rays: &'a [Option<Ray>],
    /// The bottom level's byte address and the bias of its parent table.
    base: u32,
    bias: u32,
    /// The instance node's pointer and its winding bit.
    instance: u32,
    winding: u8,
    step: i32,
    ring_mask: i32,
}

impl Below<'_> {
    /// One turn of `lane`; `Some` once the lane is done with the instance, true where it is done
    /// with the walk too.
    fn turn(
        &self,
        lane: &mut Lane,
        rings: &mut [i32],
        t_max: &mut [f32],
        out: &mut WaveOutcome,
        code: &mut Code<'_>,
    ) -> Option<bool> {
        let index = lane.lane;
        let pointer = lane.node as u32;
        let probe = Probe {
            base: self.base,
            node: pointer,
            t_max: t_max[index],
            origin: lane.origin,
            direction: lane.direction,
            inverse: lane.inverse,
        };
        let result = (code.body)(index, &probe);
        out.counts[index] += 1;
        // The children of the box before, the greatest key first so the least is popped first.
        lane.waiting.sort_unstable();
        for key in lane.waiting.iter().rev().filter(|key| **key != INVALID) {
            lane.stack += self.step;
            rings[((lane.stack & self.ring_mask) >> 2) as usize] = (key & 0x1FFF_FFFF) as i32;
            let held = (lane.stack - (index as i32) * 4) / self.step + 1;
            out.ring_deepest = out.ring_deepest.max(held as u32);
        }
        lane.waiting = [INVALID; 3];
        let mut done = false;
        if is_box(pointer) {
            let mut keys = [0, 1, 2, 3].map(|i| match result[i] {
                INVALID => INVALID,
                found => found | (((found & 4) | i as u32) << 29),
            });
            keys.sort_unstable();
            let [mut first, q0, q1, q2] = keys;
            if lane.previous == lane.node {
                lane.waiting = [q0, q1, q2];
            } else if child(first) == lane.previous {
                first = q0;
                lane.waiting = [q1, q2, INVALID];
            } else if child(q0) == lane.previous {
                first = q1;
                lane.waiting = [q2, INVALID, INVALID];
            } else if child(q1) == lane.previous {
                first = q2;
            } else {
                first = INVALID;
            }
            let next = child(first);
            lane.previous = if (first as i32) < -1 { next } else { lane.node };
            lane.node = next;
        } else {
            let mut next = -1;
            let at = self.base.wrapping_add(offset(pointer));
            let tail = word(self.words, at, TRIANGLE_WORD);
            let opaque = (tail & TRIANGLE_OPAQUE) | lane.forced == 1;
            let skip = match opaque {
                true => RAY_SKIP_OPAQUE,
                false => RAY_SKIP_NON_OPAQUE,
            };
            // A triangle the flags pass over is not looked at, nor is the second of its node.
            if pointer & 7 != 7 && lane.flags & skip == 0 {
                if pointer & 7 == 0 && tail & TRIANGLE_PAIR != 0 {
                    next = lane.node | 1;
                }
                let form = self.wave.form;
                let [numerator, denominator, first, second] = result.map(f32::from_bits);
                let reciprocal = 1.0 / denominator;
                let t = quotient(form, reciprocal, numerator);
                // The face by the high word of the denominator, signed: nought for a value no
                // triangle returns.
                let face = lane.cull * ((denominator.to_bits() as i32) >> 16);
                let t_min = match self.wave.near {
                    Near::Nought => 0.0,
                    Near::Ray => self.rays[index].map_or(0.0, |ray| ray.t_min),
                };
                if t_min <= t && face > -1 && t < t_max[index] {
                    let hit = Hit {
                        t,
                        leaf: pointer,
                        instance: self.instance,
                        kind: (if denominator > 0.0 { 0xFE } else { 0xFF }) ^ self.winding,
                        weights: [first, second].map(|x| quotient(form, reciprocal, x)),
                    };
                    // The walk's own candidate code has the last word on what is not opaque.
                    if opaque || (code.candidate)(index, &hit) {
                        t_max[index] = t;
                        out.hits[index] = Some(hit);
                        done = self.wave.first_hit;
                    }
                }
            }
            lane.node = next;
        }
        if lane.node <= 0 && lane.stack >= 0 {
            let at = ((lane.stack & self.ring_mask) >> 2) as usize;
            let popped = std::mem::replace(&mut rings[at], -1);
            if popped > 0 {
                lane.stack -= self.step;
                if popped & 4 != 0 {
                    lane.previous = popped;
                }
                lane.node = popped;
            } else {
                // The ring overwrote the entry: go on from the parent of the node last entered,
                // with the stack word as it is. The root's parent is no node, and ends the lane.
                out.climbs += 1;
                let parent = ((lane.previous as u32) >> 3)
                    .wrapping_sub(self.bias)
                    .wrapping_mul(4);
                lane.node = word(self.words, self.base, parent) as i32;
            }
        }
        let bound = out.counts[index] >= self.wave.bound;
        out.stacks[index] = lane.stack;
        (bound || done || lane.node < 1).then_some(bound || done)
    }
}

/// The record a hit's geometry has in a table of geometries, as candidate code that reads one
/// finds it: the low `bits` bits of its instance's contribution, and the geometry's number.
pub fn record_of(words: &[u32], top: u32, hit: &Hit, bits: u32) -> u32 {
    let instance = top.wrapping_add(offset(hit.instance));
    let kept = (1u32 << bits.min(24)) - 1;
    let contribution = word(words, instance, INSTANCE_CONTRIBUTION_FLAGS) & kept;
    let leaf = (word(words, instance, INSTANCE_BOTTOM) << 8).wrapping_add(offset(hit.leaf));
    contribution.wrapping_add(word(words, leaf, TRIANGLE_WORD) >> 8)
}

/// What a wave's walk asks of its caller: the instruction for a lane's number, and the walk's own
/// candidate code, which commits or not a lane's hit on a triangle that is not opaque.
pub struct Code<'a> {
    pub body: &'a mut dyn FnMut(usize, &Probe) -> [u32; 4],
    pub candidate: &'a mut dyn FnMut(usize, &Hit) -> bool,
}

/// A wave's walk of the top level at byte `top` of the arena `words`: a ray a lane, `None` for a
/// lane that does not trace, and each lane's own ray flags, none for a lane past the end of
/// `flags`; `code` answers the instruction and decides the hits on what is not opaque.
pub fn walk_wave(
    words: &[u32],
    top: u32,
    rays: &[Option<Ray>],
    flags: &[u32],
    wave: &Wave,
    code: &mut Code<'_>,
) -> WaveOutcome {
    let lanes = wave.lanes.clamp(1, 64).next_power_of_two() as usize;
    let rays = &rays[..rays.len().min(lanes)];
    let step = (lanes * 4) as i32;
    let ring = wave.ring.clamp(1, 4096).next_power_of_two() as i32;
    let ring_mask = (ring * step - 1) & !3;
    let mut rings = vec![0i32; ring as usize * lanes];
    let mut out = WaveOutcome {
        hits: vec![None; rays.len()],
        counts: vec![0; rays.len()],
        stacks: (0..rays.len() as i32).map(|lane| lane * 4 - step).collect(),
        turns: 0,
        deepest: 0,
        dropped: 0,
        ring_deepest: 0,
        climbs: 0,
    };
    let bit = |lane: usize| 1u64 << lane;
    let tracing = (0..rays.len())
        .filter(|lane| rays[*lane].is_some())
        .fold(0, |set, lane| set | bit(lane));
    let root = word(words, top, HEADER_ROOT) as i32;
    if root < 1 || tracing == 0 {
        return out;
    }
    let mut t_max: Vec<f32> = (rays.iter())
        .map(|ray| ray.map_or(0.0, |ray| ray.t_max))
        .collect();
    let mut active = tracing;
    let (mut node, mut set) = (root, active);
    // A table of the box before: four (child, lanes) pairs from slot 0, three tags from slot 12.
    let mut slot = [0u64; 16];
    let mut stack: Vec<(u32, u64)> = Vec::new();
    let pairs = (wave.stack / 2) as usize;
    loop {
        out.turns += 1;
        let pointer = node as u32;
        let instance = pointer & 7 == TYPE_INSTANCE;
        let mut results = vec![[INVALID; 4]; rays.len()];
        for (lane, ray) in rays.iter().enumerate() {
            let Some(ray) = ray else { continue };
            out.counts[lane] += 1;
            if !instance && set & bit(lane) != 0 {
                let probe = Probe {
                    base: top,
                    node: pointer,
                    t_max: t_max[lane],
                    origin: ray.origin,
                    direction: ray.direction,
                    inverse: ray.direction.map(|x| 1.0 / x),
                };
                results[lane] = (code.body)(lane, &probe);
            }
        }
        // One load for the wave: a box's four child pointers, or an instance node's four dwords
        // from its id and mask to its bottom level.
        let at = top.wrapping_add(offset(pointer));
        let from = if instance {
            INSTANCE_ID_MASK
        } else {
            BOX_CHILDREN
        };
        let loaded: [u32; 4] = std::array::from_fn(|k| word(words, at, from + 4 * k as u32));
        // The pairs of the box before, the least tag first so the best ends on top.
        let mut tags = [slot[12], slot[13], slot[14]];
        tags.sort_unstable();
        for tag in tags.into_iter().filter(|tag| *tag >= 8) {
            if stack.len() >= pairs {
                out.dropped += 1;
                continue;
            }
            let i = (tag & 7) as usize;
            stack.push((slot[i] as u32, slot[i + 1]));
            out.deepest = out.deepest.max(stack.len() as u32);
        }
        slot = [0; 16];
        if instance {
            let [id_mask, contribution_flags, root, bottom] = loaded;
            let into = match (id_mask >> 24) & u32::from(wave.mask) != 0 {
                true => set,
                false => 0,
            };
            let root = root & INSTANCE_NO_ROOT;
            if into != 0 && root != INSTANCE_NO_ROOT && root > 0 {
                let stated = contribution_flags >> 24;
                let winding = (stated & FLAG_FLIP_WINDING != 0) as u8;
                let side = match (stated & FLAG_NO_CULL != 0, winding) {
                    (true, _) => 0,
                    (false, 0) => 1,
                    (false, _) => -1,
                };
                let base = bottom << 8;
                let below = Below {
                    words,
                    wave,
                    rays,
                    base,
                    bias: (word(words, base, HEADER_FIRST_NODE) >> 6)
                        .wrapping_sub(word(words, base, HEADER_PARENTS) >> 2),
                    instance: pointer,
                    winding,
                    step,
                    ring_mask,
                };
                let mut live: Vec<Lane> = (0..rays.len())
                    .filter(|lane| into & bit(*lane) != 0)
                    .filter_map(|lane| {
                        let ray = rays[lane].as_ref()?;
                        let (origin, direction) =
                            turned_as(wave.form, words, at, ray.origin, ray.direction);
                        let own = wave.flags | flags.get(lane).copied().unwrap_or(0);
                        // The ray's two force bits where it has one, else the instance's.
                        let forced = match own & 3 {
                            0 => (stated >> 2) & 3,
                            ray => ray,
                        };
                        Some(Lane {
                            lane,
                            flags: own,
                            cull: side * cull_of(own),
                            forced,
                            origin,
                            direction,
                            inverse: direction.map(|x| 1.0 / x),
                            stack: (lane as i32) * 4 - step,
                            node: root as i32,
                            // The bottom level's root is also the node gone down into.
                            previous: root as i32,
                            waiting: [INVALID; 3],
                        })
                    })
                    .collect();
                // The lanes run a turn each in step, and meet only in the rings' memory.
                while !live.is_empty() {
                    live.retain_mut(|lane| {
                        match below.turn(lane, &mut rings, &mut t_max, &mut out, code) {
                            Some(left) => {
                                if left {
                                    active &= !bit(lane.lane);
                                }
                                false
                            }
                            None => true,
                        }
                    });
                }
            }
            set = 0;
        } else {
            // For each child, the lanes that named it first, second, third and fourth: the child
            // most lanes want soonest is taken, and the other three wait.
            let mut tag = [0u64; 4];
            for i in 0..4 {
                let child = (loaded[i] as i32).max(0) as u32;
                // The lanes whose result of place `k` is the child.
                let naming = |k: usize| {
                    let named = |lane: &usize| results[*lane][k] == child;
                    let lanes = (0..rays.len()).filter(|lane| tracing & bit(*lane) != 0);
                    lanes.filter(named).fold(0, |set, lane| set | bit(lane))
                };
                let named = [0, 1, 2, 3].map(naming);
                let wanting = named.into_iter().fold(0, |set, lanes| set | lanes);
                let key = vote(named.map(u64::count_ones));
                tag[i] = key * 8 + (6 - 2 * i as u64);
                slot[6 - 2 * i] = child.into();
                slot[7 - 2 * i] = wanting;
            }
            let mut rest = [tag[1], tag[2], tag[3]];
            rest.sort_unstable();
            let best = tag[0].max(rest[2]);
            (slot[12], slot[13], slot[14]) = (tag[0].min(rest[2]), rest[1], rest[0]);
            node = slot[(best & 7) as usize] as i32;
            set = slot[(best & 7) as usize + 1];
        }
        if set == 0 {
            node = -1;
            while let Some((child, wanting)) = stack.pop() {
                if wanting & active != 0 {
                    (node, set) = (child as i32, wanting & active);
                    break;
                }
            }
        }
        if node < 1 {
            break;
        }
    }
    out
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "wave_tests.rs"]
mod wave_tests;
