use std::collections::HashMap;

use super::super::intersect_ray;
use super::super::scenes::{placed, scene, scene_by, Scene, IDENTITY};
use super::super::{instance_nodes, stated_boxes, word};
use super::*;

type Bounds = ([f32; 3], [f32; 3]);

/// What a walk of every node of one structure met.
#[derive(Default)]
struct Met {
    /// Each triangle's corners in the model's own order, with its geometry and primitive.
    triangles: Vec<([[u32; 3]; 3], u32, u32)>,
    /// Each instance node's number.
    instances: Vec<u32>,
    /// The instance nodes that state their rotation is the identity.
    unturned: usize,
    boxes: usize,
}

fn floats(words: &[u32], at: u32, field: u32) -> [f32; 3] {
    std::array::from_fn(|k| f32::from_bits(word(words, at, field + 4 * k as u32)))
}

fn hull(points: &[[f32; 3]]) -> Bounds {
    let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for point in points {
        for k in 0..3 {
            min[k] = min[k].min(point[k]);
            max[k] = max[k].max(point[k]);
        }
    }
    (min, max)
}

/// The bounds of the node `pointer` after every invariant of its subtree held; an instance's
/// are its corners' own, which its parent states wider.
fn check(scene: &Scene, s: &Structure, pointer: u32, parent: u32, met: &mut Met) -> Bounds {
    let words = scene.arena.words();
    let table = word(words, s.base, HEADER_PARENTS);
    let (at, ty) = (s.base + offset(pointer), pointer & 7);
    let bytes = if ty == 0 { TRIANGLE_BYTES } else { BOX_BYTES };
    assert!(offset(pointer) >= HEADER_BYTES && offset(pointer) + bytes <= table);
    assert!(pointer < 1 << 29, "a pointer keeps 29 bits");
    let entry = (pointer >> 3) - (word(words, s.base, HEADER_FIRST_NODE) >> 6);
    assert_eq!(
        word(words, s.base, table + 4 * entry),
        parent,
        "the parent table"
    );
    match ty {
        TYPE_BOX32 => {
            met.boxes += 1;
            let mut inside = Vec::new();
            for i in 0..4 {
                let child = word(words, at, BOX_CHILDREN + 4 * i);
                if child == INVALID {
                    continue;
                }
                let stated = BOX_BOUNDS + BOX_BOUNDS_STRIDE * i;
                let (min, max) = (floats(words, at, stated), floats(words, at, stated + 12));
                let (low, high) = check(scene, s, child, pointer, met);
                if child & 7 == TYPE_INSTANCE {
                    let wider = (0..3).all(|k| min[k] < low[k] && max[k] > high[k]);
                    assert!(wider, "a box holds its instance's corners with room");
                } else {
                    assert_eq!(
                        (min.map(f32::to_bits), max.map(f32::to_bits)),
                        (low.map(f32::to_bits), high.map(f32::to_bits)),
                        "a box states its child's bounds to the bit"
                    );
                }
                inside.extend([min, max]);
            }
            assert!(inside.len() >= 4, "a box has two children or more");
            hull(&inside)
        }
        0 => {
            let tail = word(words, at, TRIANGLE_WORD);
            let pair = tail & TRIANGLE_PAIR != 0;
            let vertices: Vec<[f32; 3]> = (0..if pair { 4 } else { 3 })
                .map(|n| floats(words, at, TRIANGLE_VERTEX_STRIDE * n))
                .collect();
            for k in 0..if pair { 2 } else { 1 } {
                let id = word(words, at, TRIANGLE_ID) >> (8 * k);
                // The id byte names the places of the model's second and third corner; the
                // first is the one left.
                let (second, third) = ((id & 3) as usize, ((id >> 2) & 3) as usize);
                let first = 3 - second - third;
                let corner = |place: usize| vertices[CORNERS[k as usize][place] as usize];
                let corners = [first, second, third].map(|place| corner(place).map(f32::to_bits));
                let primitive = word(words, at, TRIANGLE_PRIMITIVE + 4 * k);
                met.triangles.push((corners, tail >> 8, primitive));
            }
            hull(&vertices)
        }
        TYPE_INSTANCE => {
            let number = word(words, at, INSTANCE_INDEX);
            met.instances.push(number);
            assert_eq!(word(words, at, INSTANCE_PARENT), parent);
            let instance = &scene.instances[number as usize];
            let rows: [u32; 12] =
                std::array::from_fn(|n| word(words, at, INSTANCE_OBJECT_TO_WORLD + 4 * n as u32));
            let stated = instance.object_to_world.as_flattened();
            assert!(rows.iter().zip(stated).all(|(row, x)| *row == x.to_bits()));
            assert_eq!(word(words, at, INSTANCE_BOTTOM) << 8, instance.bottom.base);
            assert_eq!(
                word(words, at, INSTANCE_ROOT) & INSTANCE_NO_ROOT,
                instance.bottom.root
            );
            let rows = instance.object_to_world;
            let unturned = (0..3).all(|r| (0..3).all(|c| rows[r][c] == f32::from(r == c)));
            let stated = word(words, at, INSTANCE_ROOT) & INSTANCE_IDENTITY != 0;
            assert_eq!(stated, unturned, "the identity bit of instance {number}");
            met.unturned += usize::from(stated);
            // Every corner of the bottom level's box, turned and moved.
            let m = instance.object_to_world;
            let corners: Vec<[f32; 3]> = (0..8)
                .map(|c| {
                    let p: [f32; 3] = std::array::from_fn(|k| match (c >> k) & 1 {
                        0 => instance.bottom.min[k],
                        _ => instance.bottom.max[k],
                    });
                    std::array::from_fn(|k| {
                        m[k][0] * p[0] + m[k][1] * p[1] + m[k][2] * p[2] + m[k][3]
                    })
                })
                .collect();
            hull(&corners)
        }
        other => panic!("a node of type {other} was built"),
    }
}

fn whole(scene: &Scene, s: &Structure) -> Met {
    let words = scene.arena.words();
    assert!(s.base != 0 && s.base.is_multiple_of(STRUCTURE_ALIGN));
    assert_eq!(word(words, s.base, HEADER_ROOT), s.root);
    assert_eq!(word(words, s.base, HEADER_SPACE), 0);
    let mut met = Met::default();
    check(scene, s, s.root, INVALID, &mut met);
    met
}

/// Holds every structure of `scene` to the arena's invariants and its triangles and instances to
/// the scene's own: the instances that state no rotation, of how many.
pub fn invariants(scene: &Scene) -> (usize, usize) {
    for (bottom, triangles) in scene.bottoms.iter().zip(&scene.triangles) {
        let met = whole(scene, bottom);
        assert!(met.instances.is_empty());
        let mut count: HashMap<_, i32> = HashMap::new();
        for t in triangles {
            let corners = t.corners.map(|c| c.map(f32::to_bits));
            *count.entry((corners, t.geometry, t.primitive)).or_default() += 1;
        }
        for found in &met.triangles {
            *count.entry(*found).or_default() -= 1;
        }
        let lost = count.values().filter(|n| **n != 0).count();
        assert_eq!(
            lost, 0,
            "every triangle is in one node, in the model's own corner order"
        );
    }
    let met = whole(scene, &scene.top);
    let mut numbers = met.instances.clone();
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        (0..scene.instances.len() as u32).collect::<Vec<_>>()
    );
    assert!(met.triangles.is_empty() && met.boxes > 0);
    (met.unturned, met.instances.len())
}

#[test]
fn every_structure_holds_the_arena_invariants() {
    let (unturned, instances) = invariants(&scene());
    assert!(unturned > 0 && unturned < instances);
}

#[test]
fn the_grid_is_built_as_pairs_and_the_soup_is_not() {
    let scene = scene();
    let words = scene.arena.words();
    let leaves = |s: &Structure| {
        let table = word(words, s.base, HEADER_PARENTS);
        let boxes = whole(&scene, s).boxes as u32;
        (table - HEADER_BYTES) / UNIT - boxes * (BOX_BYTES / UNIT)
    };
    assert_eq!(
        leaves(&scene.bottoms[0]) as usize * 2,
        scene.triangles[0].len()
    );
    assert_eq!(leaves(&scene.bottoms[1]) as usize, scene.triangles[1].len());
    // A lone triangle is its structure's root.
    assert_eq!(scene.bottoms[2].root, pointer(HEADER_BYTES / UNIT, 0));
}

#[test]
fn two_triangles_that_share_an_edge_are_one_node_only_where_they_are_as_opaque() {
    // The units of the nodes of a quad's two triangles, and each triangle's opaque bit by its
    // primitive number.
    let quad = |opaque: [bool; 2]| {
        let corners = [
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0f32],
        ];
        let triangle = |at: [usize; 3], primitive: usize| Triangle {
            corners: at.map(|corner| corners[corner]),
            geometry: 0,
            primitive: primitive as u32,
            opaque: opaque[primitive],
        };
        let mut arena = Arena::new();
        let bottom = (arena.bottom_level(&[triangle([0, 1, 2], 0), triangle([1, 3, 2], 1)]))
            .expect("a bottom level");
        let words = arena.words();
        let units = (word(words, bottom.base, HEADER_PARENTS) - HEADER_BYTES) / UNIT;
        let mut stated = Vec::new();
        for unit in HEADER_BYTES / UNIT..HEADER_BYTES / UNIT + units {
            let at = bottom.base + unit * UNIT;
            let tail = word(words, at, TRIANGLE_WORD);
            let leaf = !is_box(bottom.root) || unit >= (HEADER_BYTES + BOX_BYTES) / UNIT;
            for k in 0..if !leaf {
                0
            } else if tail & TRIANGLE_PAIR != 0 {
                2
            } else {
                1
            } {
                let primitive = word(words, at, TRIANGLE_PRIMITIVE + 4 * k);
                stated.push((primitive, tail & TRIANGLE_OPAQUE != 0));
            }
        }
        stated.sort_unstable();
        (units, stated)
    };
    // One node of two triangles is one unit; two nodes under a box are four.
    for (opaque, units) in [
        ([true, true], 1),
        ([false, false], 1),
        ([true, false], 4),
        ([false, true], 4),
    ] {
        assert_eq!(quad(opaque), (units, vec![(0, opaque[0]), (1, opaque[1])]));
    }
}

#[test]
fn a_top_level_ends_in_the_two_boxes_a_query_is_walked_through() {
    let scene = scene();
    let (top, mut arena) = (scene.top, scene.arena.clone());
    assert_eq!(stated_boxes(arena.words(), top.base), None);
    assert!(arena.state_query_root(&top, true));
    let [first, again] = stated_boxes(arena.words(), top.base).expect("the two boxes");
    assert_ne!(first, again);
    let table = word(arena.words(), top.base, HEADER_PARENTS);
    for node in [first, again] {
        // Inside the structure, under no parent, of the root and the second box, no ray's.
        assert!(offset(node) >= HEADER_BYTES && offset(node) + BOX_BYTES <= table);
        let entry = (node >> 3) - (word(arena.words(), top.base, HEADER_FIRST_NODE) >> 6);
        assert_eq!(word(arena.words(), top.base, table + 4 * entry), INVALID);
        let at = top.base + offset(node);
        let children: [u32; 4] =
            std::array::from_fn(|i| word(arena.words(), at, BOX_CHILDREN + 4 * i as u32));
        assert_eq!(children, [top.root, again, INVALID, INVALID]);
        let inverse = [1.0, 1.0, -1.0];
        let met = intersect_ray(
            arena.words(),
            top.base,
            node,
            1e9,
            [0.0; 3],
            inverse,
            inverse,
        );
        assert_eq!(met, [INVALID; 4]);
    }
    // The tree's own root again, as it was built; and a bottom level has no such boxes.
    assert!(arena.state_query_root(&top, false));
    assert_eq!(arena.words(), scene.arena.words());
    assert!(!arena.state_query_root(&scene.bottoms[0], true));
    assert_eq!(arena.words(), scene.arena.words());
}

#[test]
fn what_cannot_be_built_is_refused() {
    let mut arena = Arena::new();
    assert_eq!(arena.bottom_level(&[]), Err(AccelError::Empty));
    assert_eq!(arena.top_level(&[]), Err(AccelError::Empty));
    let mut triangle = Triangle {
        corners: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        geometry: 1 << 24,
        primitive: 0,
        opaque: true,
    };
    assert_eq!(
        arena.bottom_level(&[triangle]),
        Err(AccelError::Geometry(0))
    );
    triangle.geometry = 0;
    triangle.corners[2][1] = f32::NAN;
    assert_eq!(
        arena.bottom_level(&[triangle]),
        Err(AccelError::NotFinite(0))
    );
    triangle.corners[2][1] = 1.0;
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let flat = Instance {
        bottom,
        object_to_world: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0; 4]],
        id: 0,
        mask: 0xFF,
        flags: 0,
        contribution: 0,
    };
    assert_eq!(arena.top_level(&[flat]), Err(AccelError::Singular(0)));
}

#[test]
fn a_top_level_box_states_the_instances_under_each_child() {
    let scene = scene();
    let (words, top) = (scene.arena.words(), scene.top.base);
    let nodes = instance_nodes(words, top);
    assert_eq!(nodes.len(), scene.instances.len());
    // An instance's number is its node's place among the instance nodes by address.
    assert!(nodes.windows(2).all(|pair| pair[0] < pair[1]), "{nodes:?}");
    // The numbers of the instances under `node`.
    fn below(words: &[u32], top: u32, node: u32, nodes: &[u32], boxes: &mut u32) -> Vec<u32> {
        if !is_box(node) {
            let number = nodes.iter().position(|other| *other == node);
            return vec![number.expect("an instance node") as u32];
        }
        *boxes += 1;
        let mut all = Vec::new();
        for i in 0..4 {
            let at = top + offset(node);
            let child = word(words, at, BOX_CHILDREN + 4 * i);
            let range = word(words, at, BOX_INSTANCES + 4 * i);
            if child == INVALID {
                assert_eq!(range, INVALID);
                continue;
            }
            let under = below(words, top, child, nodes, boxes);
            let (first, last) = (range & 0xFFFF, range >> 16);
            assert_eq!(
                under,
                (first..=last).collect::<Vec<u32>>(),
                "child {i} of {node:#x}"
            );
            all.extend(under);
        }
        all
    }
    let mut boxes = 0;
    let all = below(
        words,
        top,
        word(words, top, HEADER_ROOT),
        &nodes,
        &mut boxes,
    );
    assert_eq!(all, (0..nodes.len() as u32).collect::<Vec<u32>>());
    assert!(boxes > 3, "{boxes} boxes");
    // A bottom level's boxes keep those bytes nought.
    let bottom = scene.bottoms[0];
    assert!(is_box(bottom.root));
    for i in 0..4 {
        assert_eq!(
            word(
                words,
                bottom.base + offset(bottom.root),
                BOX_INSTANCES + 4 * i
            ),
            0
        );
    }
}

#[test]
fn a_top_level_of_more_instances_than_a_node_numbers_is_refused() {
    let mut arena = Arena::new();
    let triangle = Triangle {
        corners: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let instance = placed(bottom, IDENTITY, [0.0; 3], 0xFF);
    let many = vec![instance; (1 << 16) + 1];
    assert_eq!(arena.top_level(&many), Err(AccelError::Instances(65537)));
    assert!(arena.top_level(&many[1..]).is_ok());
}

#[test]
fn a_plane_parts_two_clusters_where_the_median_cuts_through_one() {
    // Sixteen unit boxes along `axis`, the farthest first: three by the origin and thirteen far
    // off. Along the axis after it they lie in four rows, so that axis has planes of its own.
    let boxes = |axis: usize| -> Vec<Item> {
        (0..16)
            .rev()
            .map(|leaf| {
                let mut min = [0.0f32; 3];
                min[axis] = if leaf < 3 { 0.0 } else { 1000.0 } + 2.0 * leaf as f32;
                min[(axis + 1) % 3] = 3.0 * (leaf % 4) as f32;
                Item {
                    min,
                    max: min.map(|x| x + 1.0),
                    leaf,
                }
            })
            .collect()
    };
    // Whichever axis the clusters lie along, the plane between them is the cheapest of all three.
    for axis in 0..3 {
        let mut items = boxes(axis);
        let (near, far) = halve(&mut items, Split::SurfaceArea);
        let mut near: Vec<u32> = near.iter().map(|item| item.leaf).collect();
        near.sort_unstable();
        assert_eq!((near, far.len()), (vec![0, 1, 2], 13), "along axis {axis}");
        let mut items = boxes(axis);
        assert_eq!(halve(&mut items, Split::Median).0.len(), 8);
    }
    // Boxes about one centre are parted by no plane, and are halved all the same.
    let mut one: Vec<Item> = boxes(0)
        .into_iter()
        .map(|item| Item { leaf: 0, ..item })
        .collect();
    for item in &mut one {
        (item.min, item.max) = ([5.0; 3], [6.0; 3]);
    }
    assert_eq!(cheapest(&mut one), None);
    assert_eq!(halve(&mut one, Split::SurfaceArea).0.len(), 8);
    // One box, one more ten along and twelve in a row from twenty: by the surfaces alone the
    // lone box is cut off as cheaply as the two; what each part holds keeps the two together.
    let places = [0.0, 10.0].into_iter().chain((20..32).map(|x| x as f32));
    let mut row: Vec<Item> = (places.zip(0..))
        .map(|(x, leaf)| Item {
            min: [x, 0.0, 0.0],
            max: [x + 1.0, 1.0, 1.0],
            leaf,
        })
        .collect();
    row.reverse();
    assert_eq!(cheapest(&mut row), Some(2));
    assert!(row[..2].iter().all(|item| item.leaf < 2));
}

#[test]
fn a_tree_of_either_split_holds_the_arena_invariants() {
    let scenes = [Split::Median, Split::SurfaceArea].map(scene_by);
    for scene in &scenes {
        let (unturned, instances) = invariants(scene);
        assert!(unturned > 0 && unturned < instances);
    }
    assert!(
        scenes[0].arena.words() != scenes[1].arena.words(),
        "the two splits built one tree"
    );
    // What names no split of its own takes the one a ray walks fewer nodes of, and so does an
    // arena that is given none, where the tests' own variable names none.
    assert_eq!(Split::default(), Split::SurfaceArea);
    if std::env::var_os("OPENRAGE_ACCEL_SPLIT").is_none() {
        let given_none = [Arena::new().split(), Arena::default().split()];
        assert_eq!(given_none, [Split::default(); 2]);
    }
}

#[test]
fn a_top_level_is_parted_about_the_median_whatever_the_arena_s_split() {
    // Instances each twice as far along x as the one before: by surface area the farthest is
    // cut off alone at every box, and the tree is as deep as half the instances.
    let built = |split: Split| {
        let mut arena = Arena::with_split(split);
        let triangle = Triangle {
            corners: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            geometry: 0,
            primitive: 0,
            opaque: true,
        };
        let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
        let instances: Vec<Instance> = (0..40)
            .map(|n| placed(bottom, IDENTITY, [2f32.powi(n), 0.0, 0.0], 0xFF))
            .collect();
        let top = arena.top_level(&instances).expect("a top level");
        (arena.words().to_vec(), top)
    };
    let (median, top) = built(Split::Median);
    assert!(built(Split::SurfaceArea) == (median.clone(), top));
    // Forty instances four to a box: no instance lies more than three boxes down.
    fn depth(words: &[u32], top: u32, node: u32) -> u32 {
        let children = (0..4).map(|i| word(words, top + offset(node), BOX_CHILDREN + 4 * i));
        let below = children.filter(|child| is_box(*child));
        1 + below
            .map(|child| depth(words, top, child))
            .max()
            .unwrap_or(0)
    }
    assert_eq!(depth(&median, top.base, top.root), 3);
}
