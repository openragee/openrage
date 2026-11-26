//! The module's test of a ray against a node, run here against the function it is written
//! from: every answer equal to the bit, over nodes and rays made up here.

use or_io::gta6::accel::layout::{
    is_box, offset, pointer, BOX_BOUNDS, BOX_BOUNDS_STRIDE, BOX_CHILDREN, HEADER_ROOT, INVALID,
    TRIANGLE_PAIR, TRIANGLE_WORD, TYPE_BOX32, TYPE_INSTANCE,
};
use or_io::gta6::accel::{intersect_ray, word, Arena, Triangle, MISS};

use super::interp::{Calls, Value};
use super::pass_rt::{test_words, traced, MODES, WIDE};
use super::*;

/// A module that holds the test: a program of one test of a node.
fn module() -> naga::Module {
    let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    traced(words).unwrap().module
}

/// A fixed sequence of dwords.
struct Sequence(u64);

impl Sequence {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 24) as u32
    }

    fn below(&mut self, count: u32) -> u32 {
        self.next() % count
    }

    /// A float in `[-size / 2, size / 2)`.
    fn within(&mut self, size: f32) -> f32 {
        ((self.next() >> 8) as f32 / (1 << 24) as f32 - 0.5) * size
    }

    fn point(&mut self, size: f32) -> [f32; 3] {
        std::array::from_fn(|_| self.within(size))
    }
}

/// A ray as the test takes it: its origin, its direction and what a program passes as the
/// direction's inverse.
type Ray = [[f32; 3]; 3];

fn ray(origin: [f32; 3], direction: [f32; 3]) -> Ray {
    [origin, direction, direction.map(|part| 1.0 / part)]
}

/// The ray from `origin` to `target`, stretched by a made-up factor.
fn aimed(sequence: &mut Sequence, origin: [f32; 3], target: [f32; 3]) -> Ray {
    let stretch = [0.25, 1.0, 3.0, 1024.0][sequence.below(4) as usize];
    ray(
        origin,
        std::array::from_fn(|k| (target[k] - origin[k]) * stretch),
    )
}

/// The module's answer for one node and ray, held equal to the reference's.
fn answer(calls: &Calls, words: &[u32], (base, node): (u32, u32), far: f32, ray: Ray) -> [u32; 4] {
    let floats = |part: [f32; 3]| Value::List(part.map(Value::F).to_vec());
    let arguments = vec![
        Value::U(base),
        Value::U(node),
        Value::F(far),
        floats(ray[0]),
        floats(ray[1]),
        floats(ray[2]),
    ];
    let Value::List(got) = calls.call("bvh_intersect", arguments) else {
        panic!("the test answered no four dwords");
    };
    let got: Vec<u32> = got.iter().map(Value::word).collect();
    let want = intersect_ray(words, base << 8, node, far, ray[0], ray[1], ray[2]);
    assert_eq!(
        got,
        want,
        "node {node:#x} at {base:#x}, far {:#x}, ray {:x?}",
        far.to_bits(),
        ray.map(|part| part.map(f32::to_bits))
    );
    want
}

/// A far distance: none, an ordinary one, one that cuts, and ones no extent has.
fn far(sequence: &mut Sequence) -> f32 {
    match sequence.below(8) {
        0 => f32::INFINITY,
        1 => 0.0,
        2 => -1.0,
        3 => f32::NAN,
        4 => sequence.within(2.0).abs() * 1e-3,
        _ => sequence.within(8.0).abs(),
    }
}

/// The structure the made-up nodes are in: the second 256 bytes of the buffer.
const BASE: u32 = 1;

#[test]
fn a_box_s_answer_is_the_reference_s_to_the_bit() {
    const NODES: u32 = 160;
    const RAYS: u32 = 48;
    let mut sequence = Sequence(0x9E37_79B9_7F4A_7C15);
    let mut words = vec![0u32; 64 + 32 * NODES as usize];
    let mut boxes = Vec::new();
    for n in 0..NODES {
        let at = 64 + 32 * n as usize;
        let mut bounds: Vec<([f32; 3], [f32; 3])> = Vec::new();
        // Children spread apart, or one over another so a ray meets several.
        let spread = [1.5, 6.0][sequence.below(2) as usize];
        for i in 0..4 {
            let child = match sequence.below(8) {
                0 => INVALID,
                _ => sequence.next() & !0x8000_0000,
            };
            words[at + (BOX_CHILDREN / 4) as usize + i] = child;
            let centre = sequence.point(spread);
            let half = sequence.point(3.0).map(f32::abs);
            let mut least: [f32; 3] = std::array::from_fn(|k| centre[k] - half[k]);
            let mut greatest: [f32; 3] = std::array::from_fn(|k| centre[k] + half[k]);
            let axis = sequence.below(3) as usize;
            match sequence.below(16) {
                // The bounds of the child before: as near, so their places in the node decide.
                0 | 1 if i > 0 => (least, greatest) = bounds[i - 1],
                2 => least[axis] = greatest[axis],
                3 => std::mem::swap(&mut least[axis], &mut greatest[axis]),
                4 => greatest[axis] = f32::NAN,
                5 => greatest[axis] = f32::INFINITY,
                6 => (least[axis], greatest[axis]) = (f32::NEG_INFINITY, f32::INFINITY),
                _ => {}
            }
            bounds.push((least, greatest));
            let first = at + ((BOX_BOUNDS + BOX_BOUNDS_STRIDE * i as u32) / 4) as usize;
            for k in 0..3 {
                words[first + k] = least[k].to_bits();
                words[first + 3 + k] = greatest[k].to_bits();
            }
        }
        boxes.push((pointer(2 * n, TYPE_BOX32), bounds));
    }
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    // How many cases met each count of children, and how many answered two in the node's order
    // for being as near.
    let (mut met, mut level, mut cases) = ([0u32; 5], 0u32, 0u32);
    for (node, bounds) in &boxes {
        for _ in 0..RAYS {
            let (least, greatest) = bounds[sequence.below(4) as usize];
            let inside: [f32; 3] = std::array::from_fn(|k| {
                let share = sequence.within(1.0) + 0.5;
                least[k] + (greatest[k] - least[k]) * share
            });
            let mut origin = sequence.point(14.0);
            let axis = sequence.below(3) as usize;
            let other = (axis + 1) % 3;
            let mut ray = match sequence.below(8) {
                0 => ray(origin, sequence.point(2.0)),
                // Along one axis, or in one plane, at a coordinate inside the box or on its face.
                1 | 2 => {
                    origin[other] =
                        [inside[other], least[other], greatest[other]][sequence.below(3) as usize];
                    let mut direction = sequence.point(2.0);
                    direction[other] = [0.0, -0.0, 1e-40][sequence.below(3) as usize];
                    if sequence.below(2) == 0 {
                        let third = (axis + 2) % 3;
                        origin[third] = inside[third];
                        direction[third] = 0.0;
                    }
                    ray(origin, direction)
                }
                // From a point of the box itself.
                3 => {
                    let target = sequence.point(14.0);
                    aimed(&mut sequence, inside, target)
                }
                _ => aimed(&mut sequence, origin, inside),
            };
            if sequence.below(16) == 0 {
                // The inverse is an operand: whatever the program passes is what is used.
                ray[2] = sequence.point(4.0);
            }
            let far = far(&mut sequence);
            let found = answer(&calls, &words, (BASE, *node), far, ray);
            let count = found.iter().filter(|child| **child != INVALID).count();
            met[count] += 1;
            cases += 1;
            let key = |child: u32| {
                let at = (BASE << 8) + offset(*node);
                let place = (0..4).find(|i| word(&words, at, BOX_CHILDREN + 4 * i) == child);
                place.map(|i| bounds[i as usize])
            };
            level += u32::from(
                (found.windows(2)).any(|pair| pair[1] != INVALID && key(pair[0]) == key(pair[1])),
            );
        }
    }
    assert_eq!(cases, NODES * RAYS);
    assert_eq!((met, level), ([3851, 2158, 1140, 432, 99], 621));
}

/// A node of two triangles: four vertices and its last four dwords.
fn triangle_node(sequence: &mut Sequence, scale: f32) -> [u32; 16] {
    let mut node = [0u32; 16];
    for part in &mut node[..12] {
        *part = (sequence.within(4.0) * scale).to_bits();
    }
    for part in &mut node[12..] {
        *part = sequence.next();
    }
    node
}

/// The corners of triangle `kind` of a node, as floats.
fn corners_of(node: &[u32; 16], kind: u32) -> [[f32; 3]; 3] {
    use or_io::gta6::accel::layout::CORNERS;
    CORNERS[kind as usize]
        .map(|vertex| std::array::from_fn(|k| f32::from_bits(node[3 * vertex as usize + k])))
}

/// A ray at triangle `corners`: into its inside, at an edge, at a corner, along an axis, or
/// past it.
fn at_a_triangle(sequence: &mut Sequence, corners: [[f32; 3]; 3], scale: f32) -> Ray {
    let weights = match sequence.below(8) {
        0 => [0.5, 0.5, 0.0],
        1 => [0.0, 0.25, 0.75],
        2 => [1.0, 0.0, 0.0],
        3 => [1.5, -0.25, -0.25],
        _ => {
            let (a, b) = (sequence.within(1.0) + 0.5, sequence.within(1.0) + 0.5);
            match a + b > 1.0 {
                true => [1.0 - a, 1.0 - b, a + b - 1.0],
                false => [a, b, 1.0 - a - b],
            }
        }
    };
    let target: [f32; 3] = std::array::from_fn(|k| {
        corners[0][k] * weights[0] + corners[1][k] * weights[1] + corners[2][k] * weights[2]
    });
    match sequence.below(4) {
        // Along one axis: the other two components of the direction are nought.
        0 => {
            let axis = sequence.below(3) as usize;
            let mut origin = target;
            origin[axis] += [3.0, -3.0][sequence.below(2) as usize] * scale;
            let mut direction = [[0.0, -0.0][sequence.below(2) as usize]; 3];
            direction[axis] = target[axis] - origin[axis];
            ray(origin, direction)
        }
        _ => {
            let origin = sequence.point(12.0).map(|part| part * scale);
            aimed(sequence, origin, target)
        }
    }
}

#[test]
fn a_triangle_s_answer_is_the_reference_s_to_the_bit() {
    const NODES: u32 = 150;
    const RAYS: u32 = 16;
    let mut sequence = Sequence(0x2545_F491_4F6C_DD1D);
    let nodes: Vec<[u32; 16]> = (0..NODES)
        .map(|_| triangle_node(&mut sequence, 1.0))
        .collect();
    let mut words = vec![0u32; 64];
    words.extend(nodes.iter().flatten());
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    // Hits and misses of each of a node's four triangles.
    let mut found = [[0u32; 2]; 4];
    for (n, node) in nodes.iter().enumerate() {
        for kind in 0..4 {
            for _ in 0..RAYS {
                let ray = at_a_triangle(&mut sequence, corners_of(node, kind), 1.0);
                let far = far(&mut sequence);
                let place = (BASE, pointer(n as u32, kind));
                let hit = answer(&calls, &words, place, far, ray) != MISS;
                found[kind as usize][usize::from(hit)] += 1;
            }
        }
    }
    let cases: u32 = found.iter().flatten().sum();
    assert_eq!(cases, NODES * 4 * RAYS);
    // A corner past the node's fourth vertex is the node's last dwords, which are no numbers
    // here as often as not: the fourth triangle is mostly a miss.
    assert_eq!(
        found,
        [[559, 1841], [603, 1797], [1078, 1322], [1125, 1275]]
    );
}

#[test]
fn a_determinant_at_either_end_of_its_range_is_kept_or_dropped_as_the_reference_does() {
    const NODES: u32 = 24;
    const RAYS: u32 = 12;
    let mut sequence = Sequence(0x0123_4567_89AB_CDEF);
    // A product of two coordinates is the determinant's size: 2^-126 and 2^126 are where it
    // is dropped, so the coordinates are scaled about 2^-63 and 2^63.
    let scales: Vec<f32> = (-67..=-59)
        .chain(59..=67)
        .map(|power| 2.0f32.powi(power))
        .collect();
    let mut nodes = Vec::new();
    for &scale in &scales {
        for _ in 0..NODES {
            nodes.push((triangle_node(&mut sequence, scale), scale));
        }
    }
    let mut words = vec![0u32; 64];
    words.extend(nodes.iter().flat_map(|(node, _)| node));
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    // Hits and misses of the rays aimed inside a triangle, small scales then great ones.
    let mut found = [[0u32; 2]; 2];
    let mut cases = 0;
    for (n, (node, scale)) in nodes.iter().enumerate() {
        let corners = corners_of(node, 0);
        for _ in 0..RAYS {
            let (a, b) = (sequence.within(0.6) + 0.5, sequence.within(0.6) + 0.5);
            let weights = [a * (1.0 - b), (1.0 - a) * (1.0 - b), b];
            let target: [f32; 3] = std::array::from_fn(|k| {
                corners[0][k] * weights[0] + corners[1][k] * weights[1] + corners[2][k] * weights[2]
            });
            let origin = sequence.point(12.0).map(|part| part * scale);
            let ray = aimed(&mut sequence, origin, target);
            let place = (BASE, pointer(n as u32, 0));
            let hit = answer(&calls, &words, place, f32::INFINITY, ray) != MISS;
            found[usize::from(*scale > 1.0)][usize::from(hit)] += 1;
            cases += 1;
        }
    }
    assert_eq!(cases, scales.len() as u32 * NODES * RAYS);
    assert_eq!(found, [[1199, 1393], [1426, 1166]]);
}

#[test]
fn a_node_that_is_neither_answers_no_node() {
    let mut sequence = Sequence(0x0F0F_1234_5678_9ABC);
    let words: Vec<u32> = (0..64 + 16 * 64).map(|_| sequence.next()).collect();
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    let mut cases = 0;
    for unit in 0..64 {
        // An instance, a node of no kind, and a box of halves, which no structure here holds.
        for kind in [TYPE_INSTANCE, 7, 4] {
            let ray = ray(sequence.point(8.0), sequence.point(2.0));
            let found = answer(&calls, &words, (BASE, pointer(unit, kind)), 4.0, ray);
            assert_eq!(found, [INVALID; 4]);
            cases += 1;
        }
    }
    assert_eq!(cases, 192);
    // And a node past the buffer's end is one of noughts, as the reference reads it.
    let beyond = pointer(words.len() as u32, 0);
    let ray = ray([0.0, 0.0, 1.0], [0.0, 0.0, -1.0]);
    assert_eq!(answer(&calls, &words, (BASE, beyond), 4.0, ray), MISS);
}

/// Line `i` of a grid of `n` cells across four units, and the height of the crossing of two.
fn crossing(i: usize, j: usize, n: usize) -> [f32; 3] {
    let line = |i: usize| i as f32 / n as f32 * 4.0 - 2.0;
    let height = 0.06 * ((i * 7 + j * 3) % 6) as f32;
    [line(i), line(j), height]
}

#[test]
fn a_built_structure_s_nodes_answer_as_the_reference_along_shared_edges() {
    const N: usize = 5;
    let mut triangles = Vec::new();
    for i in 0..N {
        for j in 0..N {
            let quad = [
                crossing(i, j, N),
                crossing(i + 1, j, N),
                crossing(i, j + 1, N),
                crossing(i + 1, j + 1, N),
            ];
            for corners in [[quad[0], quad[1], quad[2]], [quad[1], quad[3], quad[2]]] {
                triangles.push(Triangle {
                    corners,
                    geometry: 0,
                    primitive: triangles.len() as u32,
                    opaque: true,
                });
            }
        }
    }
    let mut arena = Arena::new();
    let bottom = arena.bottom_level(&triangles).unwrap();
    let words = arena.words();
    assert_eq!(word(words, bottom.base, HEADER_ROOT), bottom.root);
    // Every node of the structure: the boxes, and each triangle of each leaf.
    let (mut nodes, mut next) = (Vec::new(), vec![bottom.root]);
    while let Some(node) = next.pop() {
        nodes.push(node);
        let at = bottom.base + offset(node);
        if is_box(node) {
            let children = (0..4).map(|i| word(words, at, BOX_CHILDREN + 4 * i));
            next.extend(children.filter(|child| *child != INVALID));
        } else if node & 7 == 0 && word(words, at, TRIANGLE_WORD) & TRIANGLE_PAIR != 0 {
            nodes.push(node | 1);
        }
    }
    let leaves = nodes.iter().filter(|node| !is_box(**node)).count();
    assert_eq!(leaves, triangles.len());
    // Rays at the crossings of the grid's lines and at the middles of its edges, slanted and
    // straight down: every such point is on an edge two triangles share, or on the rim.
    let mut rays = Vec::new();
    for i in 0..=2 * N {
        for j in 0..=2 * N {
            if i % 2 == 1 && j % 2 == 1 {
                continue;
            }
            let ends = [
                crossing(i / 2, j / 2, N),
                crossing(i.div_ceil(2), j.div_ceil(2), N),
            ];
            let target: [f32; 3] = std::array::from_fn(|k| (ends[0][k] + ends[1][k]) * 0.5);
            let inner = (1..2 * N).contains(&i) && (1..2 * N).contains(&j);
            // From above, where no edge of the surface is seen edge on, and from the side.
            let origins = [
                [0.3, -0.2, 5.0],
                [target[0], target[1], 5.0],
                [-7.0, 4.0, 2.0],
            ];
            for (from, origin) in origins.into_iter().enumerate() {
                let direction = std::array::from_fn(|k| target[k] - origin[k]);
                rays.push((ray(origin, direction), inner && from < 2));
            }
        }
    }
    let module = module();
    let calls = Calls::new(&module, &[("arena", words)]);
    // The rays at an edge inside the grid, and those that two triangles or more answered.
    let (mut cases, mut shared, mut twice) = (0u32, 0u32, 0u32);
    for (ray, inner) in &rays {
        let mut hits = 0;
        for node in &nodes {
            let found = answer(&calls, words, (bottom.base >> 8, *node), 2.0, *ray);
            hits += u32::from(!is_box(*node) && found != MISS);
            cases += 1;
        }
        shared += u32::from(*inner);
        twice += u32::from(*inner && hits >= 2);
        // No ray at an edge inside the surface passes between its two triangles.
        assert!(!inner || hits >= 1, "{ray:?}");
    }
    // Every ray against every node of the tree, whichever tree the arena built.
    assert_eq!(cases, (rays.len() * nodes.len()) as u32);
    assert_eq!((shared, twice), (112, 70));
}

/// A float from `10^low` to `10^high`.
fn power(sequence: &mut Sequence, low: f32, high: f32) -> f32 {
    10f32.powf(low + (sequence.within(1.0) + 0.5) * (high - low))
}

/// A node of one triangle: its corners are the node's first three vertices.
fn lone(corners: [[f32; 3]; 3]) -> [u32; 16] {
    let mut node = [0u32; 16];
    for (vertex, corner) in corners.iter().enumerate() {
        for (k, part) in corner.iter().enumerate() {
            node[3 * vertex + k] = part.to_bits();
        }
    }
    node
}

/// The words of a structure of `nodes`, the first at pointer 0 of [`BASE`].
fn structure(nodes: impl IntoIterator<Item = [u32; 16]>) -> Vec<u32> {
    let mut words = vec![0u32; 64];
    words.extend(nodes.into_iter().flatten());
    words
}

#[test]
fn a_triangle_of_any_size_thin_or_wide_answers_as_the_reference_to_the_bit() {
    const CASES: u32 = 4000;
    let mut sequence = Sequence(0x517C_C1B7_2722_0A95);
    let mut cases = Vec::new();
    for _ in 0..CASES {
        // A triangle from far below a unit to far above it across, every other one thin: there a
        // distance is a quotient of two small numbers, and the stretch of the ray inside the
        // triangle's own bounds is what it is brought into.
        let size = power(&mut sequence, -15.0, 15.0);
        let (away, shrink) = (
            power(&mut sequence, -3.0, 3.0),
            power(&mut sequence, -6.0, 0.0),
        );
        let mut corners: [[f32; 3]; 3] = std::array::from_fn(|_| sequence.point(size));
        if sequence.below(2) == 0 {
            let thin = power(&mut sequence, -7.0, -3.0);
            let along = sequence.within(1.0) + 0.5;
            let [a, b, _] = corners;
            corners[2] = std::array::from_fn(|k| {
                a[k] + along * (b[k] - a[k]) + sequence.within(thin * size)
            });
        }
        let (wb, wc) = (
            (sequence.within(1.0) + 0.5) * 0.5,
            (sequence.within(1.0) + 0.5) * 0.5,
        );
        let [a, b, c] = corners;
        let on: [f32; 3] = std::array::from_fn(|k| a[k] + wb * (b[k] - a[k]) + wc * (c[k] - a[k]));
        let origin: [f32; 3] = std::array::from_fn(|k| on[k] + sequence.within(away * size));
        let direction = std::array::from_fn(|k| (on[k] - origin[k]) * shrink);
        cases.push((lone(corners), ray(origin, direction)));
    }
    let words = structure(cases.iter().map(|(node, _)| *node));
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    let mut kept = 0;
    for (n, (_, ray)) in cases.iter().enumerate() {
        let place = (BASE, pointer(n as u32, 0));
        kept += u32::from(answer(&calls, &words, place, f32::INFINITY, *ray) != MISS);
    }
    assert_eq!((cases.len() as u32, kept), (CASES, 3602));
}

#[test]
fn a_needle_that_points_at_a_ray_from_its_side_is_dropped_as_the_reference_drops_it() {
    const CASES: u32 = 4000;
    let mut sequence = Sequence(0x0BAD_5EED_0DDB_A115);
    let mut cases = Vec::new();
    for _ in 0..CASES {
        // Three corners all but in line with a point of the ray, a hundred units to its side
        // and farther: sheared, their edges' areas against the ray are nought or of one sign by
        // chance, and only the triangle's own bounds say the ray passes beside it.
        let mut direction = sequence.point(2.0);
        // Every other ray runs in a plane of the axes, or along one: there no distance says the
        // ray is beside the needle on that axis, only where its origin lies.
        for _ in 0..[0, 0, 1, 2][sequence.below(4) as usize] {
            direction[sequence.below(3) as usize] = 0.0;
        }
        let origin = sequence.point(50.0);
        let along = sequence.within(1.0) + 0.5;
        let through: [f32; 3] = std::array::from_fn(|k| origin[k] + along * direction[k]);
        let side = sequence.point(2.0);
        let width = (sequence.within(1.0) + 0.5) * 1e-4;
        let corners: [[f32; 3]; 3] = std::array::from_fn(|n| {
            let out = 100.0 * (1.0 + 0.35 * n as f32);
            std::array::from_fn(|k| through[k] + out * side[k] + sequence.within(width))
        });
        // Whether the ray's whole line stays well clear of the needle.
        let reach = side.iter().map(|x| x * x).sum::<f32>().sqrt();
        let lean = (0..3).map(|k| side[k] * direction[k]).sum::<f32>().abs();
        let length = direction.iter().map(|x| x * x).sum::<f32>().sqrt();
        let clear = reach >= 0.1 && lean <= 0.5 * reach * length;
        cases.push((lone(corners), ray(origin, direction), clear));
    }
    let words = structure(cases.iter().map(|(node, ..)| *node));
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    let (mut clear, mut kept) = (0, 0);
    for (n, (_, ray, stays)) in cases.iter().enumerate() {
        let place = (BASE, pointer(n as u32, 0));
        let hit = answer(&calls, &words, place, f32::INFINITY, *ray) != MISS;
        clear += u32::from(*stays);
        kept += u32::from(*stays && hit);
    }
    assert_eq!((cases.len() as u32, clear, kept), (CASES, 1971, 0));
}

/// A float's place among the floats, the infinities at its ends.
fn place_of(float: f32) -> i64 {
    let bits = float.to_bits() as i32;
    i64::from(bits ^ (((bits >> 31) as u32) >> 1) as i32)
}

/// The float at a place of [`place_of`].
fn float_at(place: i64) -> f32 {
    let place = place as i32;
    f32::from_bits((place ^ (((place >> 31) as u32) >> 1) as i32) as u32)
}

#[test]
fn a_box_entered_exactly_at_the_far_distance_is_not_met() {
    const BOXES: u32 = 300;
    let mut sequence = Sequence(0xFA57_0FF5_E7C0_FFEE);
    let mut words = vec![0u32; 64 + 32 * BOXES as usize];
    let mut boxes = Vec::new();
    for n in 0..BOXES {
        let at = 64 + 32 * n as usize;
        let children = at + (BOX_CHILDREN / 4) as usize;
        words[children..children + 4].fill(INVALID);
        words[children] = 8 * (n + 1);
        let centre = sequence.point(6.0);
        let half = sequence.point(3.0).map(|part| part.abs() + 0.01);
        let first = at + (BOX_BOUNDS / 4) as usize;
        for k in 0..3 {
            words[first + k] = (centre[k] - half[k]).to_bits();
            words[first + 3 + k] = (centre[k] + half[k]).to_bits();
        }
        boxes.push((pointer(2 * n, TYPE_BOX32), centre, half));
    }
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    let (mut cases, mut ahead) = (0, 0);
    for (node, centre, half) in &boxes {
        for _ in 0..2 {
            let inside: [f32; 3] =
                std::array::from_fn(|k| centre[k] + sequence.within(1.6) * half[k]);
            let origin = sequence.point(40.0);
            let ray = aimed(&mut sequence, origin, inside);
            let met = |far: f32| {
                intersect_ray(&words, BASE << 8, *node, far, ray[0], ray[1], ray[2])[0] != INVALID
            };
            if !met(f32::INFINITY) || met(f32::NEG_INFINITY) {
                continue;
            }
            // The least far distance the box is met with is the float after the distance the
            // ray enters it at: a box is met where it is entered before the far distance.
            let (mut low, mut high) = (place_of(f32::NEG_INFINITY), place_of(f32::INFINITY));
            while high - low > 1 {
                let middle = low + (high - low) / 2;
                match met(float_at(middle)) {
                    true => high = middle,
                    false => low = middle,
                }
            }
            let (entry, after) = (float_at(low), float_at(high));
            let at_entry = answer(&calls, &words, (BASE, *node), entry, ray);
            let past_it = answer(&calls, &words, (BASE, *node), after, ray);
            assert_eq!((at_entry[0], past_it[0] != INVALID), (INVALID, true));
            cases += 2;
            ahead += u32::from(entry > 0.0);
        }
    }
    assert_eq!((cases, ahead), (1200, 600));
}

#[test]
fn a_ray_as_long_on_two_axes_takes_the_lower_of_them_as_the_reference_does() {
    const NODES: u32 = 100;
    const RAYS: u32 = 30;
    let mut sequence = Sequence(0x7E57_AB1E_0F7E_571E);
    let nodes: Vec<[u32; 16]> = (0..NODES)
        .map(|_| triangle_node(&mut sequence, 1.0))
        .collect();
    let words = structure(nodes.iter().copied());
    let module = module();
    let calls = Calls::new(&module, &[("arena", &words)]);
    let (mut cases, mut hits) = (0, 0);
    for (n, node) in nodes.iter().enumerate() {
        let corners = corners_of(node, 0);
        for _ in 0..RAYS {
            let (a, b) = (sequence.within(0.8) + 0.5, sequence.within(0.8) + 0.5);
            let weights = [a * (1.0 - b), (1.0 - a) * (1.0 - b), b];
            let target: [f32; 3] = std::array::from_fn(|k| {
                corners[0][k] * weights[0] + corners[1][k] * weights[1] + corners[2][k] * weights[2]
            });
            // Two components of one size, of either sign, and a third no greater: one time in
            // four as great too.
            let size = sequence.within(4.0).abs() + 0.25;
            let mut signed = |size: f32| size * [1.0, -1.0][sequence.below(2) as usize];
            let mut direction = [signed(size), signed(size), signed(size)];
            if sequence.below(4) != 0 {
                direction[2] = sequence.within(2.0) * size;
            }
            direction.swap(sequence.below(3) as usize, 2);
            let origin = std::array::from_fn(|k| target[k] - direction[k]);
            let place = (BASE, pointer(n as u32, 0));
            hits += u32::from(answer(&calls, &words, place, 8.0, ray(origin, direction)) != MISS);
            cases += 1;
        }
    }
    assert_eq!((cases, hits), (NODES * RAYS, 3000));
}
