//! Made-up scenes the reference walks are tested on: a bumpy grid of quads, a soup of loose
//! triangles, instances of them turned and moved about, a stack of triangles and a chain of boxes
//! that fills a wave's stack.

use super::layout::*;
use super::reference::{world, Placed};
use super::{Arena, Instance, Ray, Split, Structure, Triangle};

/// A fixed sequence of floats in `[0, 1)`.
pub struct Sequence(pub u32);

impl Sequence {
    pub fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1 << 24) as f32
    }

    /// A float in `[-size / 2, size / 2)`.
    pub fn within(&mut self, size: f32) -> f32 {
        (self.next() - 0.5) * size
    }
}

/// The coordinate of line `i` of a grid of `n` cells across `size`.
pub fn line(i: usize, n: usize, size: f32) -> f32 {
    i as f32 / n as f32 * size - size / 2.0
}

/// Where lines `i` and `j` of a grid cross, lifted by a bump of the grid's own.
pub fn crossing(i: usize, j: usize, n: usize, size: f32, bump: f32) -> [f32; 3] {
    let height = bump * (i as f32 * 0.7).sin() * (j as f32 * 0.5).cos();
    [line(i, n, size), line(j, n, size), height]
}

/// `n` by `n` quads across `size`, each two triangles that share its diagonal.
pub fn grid(n: usize, size: f32, bump: f32) -> Vec<Triangle> {
    let at = |i: usize, j: usize| crossing(i, j, n, size, bump);
    let mut triangles = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let quad = [at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1)];
            for mut corners in [[quad[0], quad[1], quad[2]], [quad[1], quad[3], quad[2]]] {
                // Each triangle starts at another of its corners, so the node of a pair holds
                // triangles turned from the model's own order.
                corners.rotate_left(triangles.len() % 3);
                triangles.push(Triangle {
                    corners,
                    geometry: 0,
                    primitive: triangles.len() as u32,
                    opaque: true,
                });
            }
        }
    }
    triangles
}

/// `n` triangles of about `edge` across, scattered through a cube of `size`.
pub fn soup(sequence: &mut Sequence, n: usize, size: f32, edge: f32) -> Vec<Triangle> {
    (0..n)
        .map(|primitive| {
            let centre: [f32; 3] = std::array::from_fn(|_| sequence.within(size));
            Triangle {
                corners: std::array::from_fn(|_| centre.map(|x| x + sequence.within(edge))),
                geometry: 1,
                primitive: primitive as u32,
                opaque: true,
            }
        })
        .collect()
}

/// `count` loose triangles stacked along z, `step` apart, each across the z axis and none
/// opaque: the builder splits them along z where they reach farther along it than across.
pub fn stack(count: usize, step: f32) -> Vec<Triangle> {
    (0..count)
        .map(|k| {
            let z = k as f32 * step;
            Triangle {
                corners: [[-1.0, -1.0, z], [2.0, -1.0, z], [-1.0, 2.0, z]],
                geometry: 0,
                primitive: k as u32,
                opaque: false,
            }
        })
        .collect()
}

/// A [`stack`] placed once, unturned, with its top level.
pub fn stacked(count: usize, step: f32) -> (Arena, Structure) {
    let mut arena = Arena::new();
    let bottom = (arena.bottom_level(&stack(count, step))).expect("a bottom level");
    let top = (arena.top_level(&[placed(bottom, IDENTITY, [0.0; 3], 0xFF)])).expect("a top level");
    (arena, top)
}

pub const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn rotation(sequence: &mut Sequence) -> [[f32; 3]; 3] {
    let turn = std::f32::consts::TAU;
    let (a, b) = (sequence.next() * turn, sequence.next() * turn);
    let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
    [
        [ca, -sa * cb, sa * sb],
        [sa, ca * cb, -ca * sb],
        [0.0, sb, cb],
    ]
}

pub fn placed(bottom: Structure, turn: [[f32; 3]; 3], at: [f32; 3], mask: u8) -> Instance {
    Instance {
        bottom,
        object_to_world: std::array::from_fn(|r| [turn[r][0], turn[r][1], turn[r][2], at[r]]),
        id: 0,
        mask,
        flags: 0,
        contribution: 0,
    }
}

pub struct Scene {
    pub arena: Arena,
    pub top: Structure,
    pub triangles: [Vec<Triangle>; 3],
    pub bottoms: [Structure; 3],
    pub instances: Vec<Instance>,
}

/// The mask the scene's rays carry; one instance in seven answers to another.
pub const RAY_MASK: u8 = 0x40;
/// The mask of the instances the scene's rays leave out: a bit no walk read so far asks for.
pub const OTHER_MASK: u8 = 0x02;

impl Scene {
    /// Every triangle of the scene in the world, for the test of every triangle.
    pub fn world(&self) -> Vec<Placed> {
        let models: Vec<_> = (self.instances.iter())
            .map(|instance| {
                let of = |bottom: &Structure| bottom.base == instance.bottom.base;
                let bottom = self.bottoms.iter().position(of).expect("a bottom level");
                (&self.triangles[bottom][..], instance)
            })
            .collect();
        world(&models)
    }
}

/// A grid, a soup and a lone triangle, 24 instances of them, and the grid once more where it
/// was made: unturned and unmoved. One instance in four culls no face and one in four has its
/// winding turned.
pub fn scene() -> Scene {
    scene_by(Arena::new().split())
}

/// [`scene`] with its structures built by `split`.
pub fn scene_by(split: Split) -> Scene {
    let mut sequence = Sequence(12345);
    let mut arena = Arena::with_split(split);
    let triangles = [
        grid(24, 10.0, 0.4),
        soup(&mut sequence, 1500, 8.0, 0.6),
        soup(&mut sequence, 1, 2.0, 1.0),
    ];
    let bottoms = [0, 1, 2].map(|k| arena.bottom_level(&triangles[k]).expect("a bottom level"));
    let mut instances: Vec<Instance> = (0..24)
        .map(|i| {
            let turn = match i % 5 {
                0 => IDENTITY,
                _ => rotation(&mut sequence),
            };
            let at = std::array::from_fn(|_| sequence.within(40.0));
            let mask = if i % 7 == 0 { OTHER_MASK } else { RAY_MASK };
            let flags = [0, FLAG_NO_CULL, FLAG_FLIP_WINDING, 0][i % 4] as u8;
            Instance {
                flags,
                ..placed(bottoms[i % 3], turn, at, mask)
            }
        })
        .collect();
    instances.push(placed(bottoms[0], IDENTITY, [0.0; 3], RAY_MASK));
    let top = arena.top_level(&instances).expect("a top level");
    Scene {
        arena,
        top,
        triangles,
        bottoms,
        instances,
    }
}

/// A ray from somewhere about the scene to somewhere inside it, its extent the whole way.
pub fn ray(sequence: &mut Sequence) -> Ray {
    let origin: [f32; 3] = std::array::from_fn(|_| sequence.within(60.0));
    let direction = origin.map(|from| {
        let to = sequence.within(40.0) - from;
        if to == 0.0 {
            1e-9
        } else {
            to
        }
    });
    Ray {
        origin,
        direction,
        t_min: 0.0,
        t_max: 1.0,
    }
}

/// A top level written node by node over a bottom level of one triangle.
pub struct Chain {
    /// The arena's dwords.
    pub words: Vec<u32>,
    /// The top level's byte address.
    pub top: u32,
    /// The instances in the order of their numbers.
    pub instances: Vec<Instance>,
    /// The bottom level's triangles.
    pub triangles: Vec<Triangle>,
}

/// A chain of `boxes` boxes along z, each of the next box and three instances but the last, of
/// three instances alone. A box states its children over the whole footprint, so every ray along
/// z meets every child, the next box before its instances; each instance's triangle lies in a
/// cell of its own. Forty-three boxes leave 128 pairs waiting: a wave's whole stack.
pub fn chain(boxes: usize) -> Chain {
    let mut arena = Arena::new();
    let triangle = Triangle {
        corners: [[0.0; 3], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let mut words = arena.words().to_vec();
    let top = words.len() as u32 * 4;
    // Two units a node after the header's two; box `k` first, then its three instances.
    let unit = |k: usize, child: usize| (2 + 8 * k + 2 * child) as u32;
    let mut tree = vec![0u32; 16 * unit(boxes, 0) as usize];
    fn put(tree: &mut [u32], unit: u32, field: u32, value: u32) {
        tree[(unit * 16 + field / 4) as usize] = value;
    }
    /// Child `i` of the box at `unit`: `to`, stated over the whole footprint from `z[0]` to `z[1]`.
    fn child(tree: &mut [u32], unit: u32, i: u32, to: u32, z: [f32; 2]) {
        put(tree, unit, BOX_CHILDREN + 4 * i, to);
        let bounds = [-100.0, -100.0, z[0], 100.0, 100.0, z[1]];
        for (k, x) in bounds.into_iter().enumerate() {
            let field = BOX_BOUNDS + BOX_BOUNDS_STRIDE * i + 4 * k as u32;
            put(tree, unit, field, x.to_bits());
        }
    }
    put(&mut tree, 0, HEADER_ROOT, pointer(unit(0, 0), TYPE_BOX32));
    put(&mut tree, 0, HEADER_FIRST_NODE, HEADER_BYTES);
    let mut instances = Vec::new();
    let end = 10.0 * boxes as f32;
    for k in 0..boxes {
        let (me, level) = (unit(k, 0), 10.0 * k as f32);
        match k + 1 < boxes {
            true => {
                let next = pointer(unit(k + 1, 0), TYPE_BOX32);
                child(&mut tree, me, 3, next, [level + 1.0, end]);
            }
            false => child(&mut tree, me, 3, INVALID, [f32::NAN; 2]),
        }
        for i in 0..3 {
            let number = instances.len();
            let at = [
                2.0 * (number % 12) as f32,
                2.0 * (number / 12) as f32,
                level + 3.0 + 2.0 * i as f32,
            ];
            let instance = placed(bottom, IDENTITY, at, 0xFF);
            let node = unit(k, 1 + i);
            let slab = [at[2] - 0.5, at[2] + 0.5];
            child(&mut tree, me, i as u32, pointer(node, TYPE_INSTANCE), slab);
            for (n, x) in instance.object_to_world.as_flattened().iter().enumerate() {
                let field = INSTANCE_OBJECT_TO_WORLD + 4 * n as u32;
                put(&mut tree, node, field, x.to_bits());
            }
            let mask = u32::from(instance.mask) << 24;
            put(&mut tree, node, INSTANCE_ID_MASK, mask);
            let root = bottom.root | INSTANCE_IDENTITY;
            put(&mut tree, node, INSTANCE_ROOT, root);
            put(&mut tree, node, INSTANCE_BOTTOM, bottom.base >> 8);
            for (n, x) in at.iter().enumerate() {
                let field = INSTANCE_TRANSLATION + 4 * n as u32;
                put(&mut tree, node, field, (-x).to_bits());
            }
            put(&mut tree, node, INSTANCE_INDEX, number as u32);
            put(&mut tree, node, INSTANCE_PARENT, pointer(me, TYPE_BOX32));
            instances.push(instance);
        }
    }
    words.extend(tree);
    Chain {
        words,
        top,
        instances,
        triangles: vec![triangle],
    }
}
