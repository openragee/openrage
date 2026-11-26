use super::super::build::tests::invariants;
use super::super::layout::{offset, TRIANGLE_VERTEX_STRIDE, TRIANGLE_WORD};
use super::super::reference::met;
use super::super::scenes::{ray, scene, Scene, Sequence, RAY_MASK};
use super::super::walk::leaves;
use super::super::{walk_lane, word, AccelError, Arena, Cull, Instance, Structure, Triangle, Walk};

/// `scene` with every corner where `pose` puts it and every instance as `moved` gives it, each
/// structure rewritten where it is.
fn refitted(
    scene: &Scene,
    pose: &dyn Fn([f32; 3]) -> [f32; 3],
    moved: &dyn Fn(usize, &Instance) -> Instance,
) -> Scene {
    let mut arena = scene.arena.clone();
    let triangles = scene.triangles.clone().map(|list| {
        let posed = |triangle: Triangle| Triangle {
            corners: triangle.corners.map(pose),
            ..triangle
        };
        list.into_iter().map(posed).collect::<Vec<_>>()
    });
    let bottoms = [0, 1, 2].map(|k| {
        (arena.refit_bottom(&scene.bottoms[k], &triangles[k])).expect("a bottom level refitted")
    });
    let instances: Vec<Instance> = (scene.instances.iter().enumerate())
        .map(|(number, instance)| {
            let of = |bottom: &Structure| instance.bottom.base == bottom.base;
            let k = scene.bottoms.iter().position(of).expect("a bottom level");
            Instance {
                bottom: bottoms[k],
                ..moved(number, instance)
            }
        })
        .collect();
    let top = (arena.refit_top(&scene.top, &instances)).expect("a top level refitted");
    Scene {
        arena,
        top,
        triangles,
        bottoms,
        instances,
    }
}

/// The triangles and instances of `posed` built anew, in the same order.
fn rebuilt(posed: &Scene, was: &Scene) -> Scene {
    let mut arena = Arena::new();
    let bottoms =
        [0, 1, 2].map(|k| (arena.bottom_level(&posed.triangles[k])).expect("a bottom level"));
    let instances: Vec<Instance> = (posed.instances.iter())
        .map(|instance| {
            let of = |bottom: &Structure| instance.bottom.base == bottom.base;
            let k = was.bottoms.iter().position(of).expect("a bottom level");
            Instance {
                bottom: bottoms[k],
                ..*instance
            }
        })
        .collect();
    let top = arena.top_level(&instances).expect("a top level");
    Scene {
        arena,
        top,
        triangles: posed.triangles.clone(),
        bottoms,
        instances,
    }
}

/// `instance` with its place in the world moved by `by` times `shift`.
fn shifted(instance: &Instance, shift: [f32; 3], by: f32) -> Instance {
    let mut moved = *instance;
    for (row, shift) in moved.object_to_world.iter_mut().zip(shift) {
        row[3] += shift * by;
    }
    moved
}

#[test]
fn a_refit_of_a_pose_that_keeps_the_tree_is_a_rebuild_to_the_byte() {
    // Every position doubled: a float doubles exactly, so every compare the builder makes
    // falls as it fell, and a rebuild of the posed positions is the same tree.
    let was = scene();
    let twice = |instance: &Instance| {
        let mut moved = *instance;
        for row in &mut moved.object_to_world {
            row[3] *= 2.0;
        }
        moved
    };
    let posed = refitted(&was, &|corner| corner.map(|x| 2.0 * x), &|_, i| twice(i));
    let anew = rebuilt(&posed, &was);
    assert_eq!((posed.top, posed.bottoms), (anew.top, anew.bottoms));
    assert!(
        posed.arena.words() == anew.arena.words(),
        "the arenas differ"
    );
    // What a refit wrote lies in the nodes of the structures it was given, and nowhere else.
    let ranges: Vec<_> = (was.bottoms.iter().chain([&was.top]))
        .map(|structure| was.arena.node_bytes(structure))
        .collect();
    let (before, after) = (was.arena.words(), posed.arena.words());
    let changed: Vec<u32> = (0..before.len() as u32)
        .filter(|at| before[*at as usize] != after[*at as usize])
        .map(|at| at * 4)
        .collect();
    let within = |at: &u32| ranges.iter().any(|range| range.contains(at));
    assert!(changed.len() > 1000 && changed.iter().all(within));
    let nodes: u32 = ranges.iter().map(|range| range.end - range.start).sum();
    println!(
        "{} of the {} dwords of the nodes were rewritten, of {} dwords in the arena",
        changed.len(),
        nodes / 4,
        before.len()
    );
}

/// The scene bent and its instances moved apart, refitted, and the same built anew.
fn bent() -> (Scene, Scene) {
    let was = scene();
    let bend = |p: [f32; 3]| {
        [
            p[0] + 0.3 * (p[1] * 0.7).sin(),
            p[1] - 0.1 * p[0],
            p[2] + 0.2 * (p[0] * 0.5).cos() + 0.1 * p[1],
        ]
    };
    let apart = |number: usize, instance: &Instance| {
        shifted(instance, [0.5, -0.25, 1.0], (number % 3) as f32)
    };
    let posed = refitted(&was, &bend, &apart);
    let anew = rebuilt(&posed, &was);
    (posed, anew)
}

#[test]
fn a_refit_s_boxes_hold_the_posed_triangles_to_the_bit() {
    let (posed, anew) = bent();
    // Every box states its children's bounds to the bit, every node holds its model's posed
    // triangle, and every instance node its instance as it is now: what holds of a built arena.
    let (unturned, instances) = invariants(&posed);
    assert!(unturned > 0 && unturned < instances);
    // The structures bound what a rebuild's bound.
    assert_eq!(
        posed.bottoms.map(|bottom| (bottom.min, bottom.max)),
        anew.bottoms.map(|bottom| (bottom.min, bottom.max))
    );
    assert_eq!((posed.top.min, posed.top.max), (anew.top.min, anew.top.max));
}

/// What `rays` rays gave over a refitted scene and over the same scene built anew.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Walked {
    pub hits: usize,
    /// The rays whose closest hit is the same triangle of the same instance at the same
    /// distance, face and weights, and those whose first hit is one where the other's is.
    pub closest: usize,
    pub first: usize,
    /// The turns of the closest-hit walks over the refit, and over the rebuild.
    pub turns: [u32; 2],
}

pub fn walked(rays: usize) -> Walked {
    let (posed, anew) = bent();
    let mut sequence = Sequence(1917);
    let mut walked = Walked::default();
    for _ in 0..rays {
        let ray = ray(&mut sequence);
        for first_hit in [false, true] {
            let walk = Walk {
                mask: RAY_MASK,
                cull: Cull::Back,
                first_hit,
                ..Walk::default()
            };
            let over = |scene: &Scene| {
                let (words, top) = (scene.arena.words(), scene.top.base);
                let out = walk_lane(words, top, &ray, &walk, &mut |_| true);
                (out.hit.map(|hit| met(words, top, &hit)), out.turns)
            };
            let ((refit, turns), (rebuild, anew_turns)) = (over(&posed), over(&anew));
            match first_hit {
                false => {
                    walked.hits += usize::from(rebuild.is_some());
                    walked.closest += usize::from(refit == rebuild);
                    walked.turns[0] += turns;
                    walked.turns[1] += anew_turns;
                }
                true => walked.first += usize::from(refit.is_some() == rebuild.is_some()),
            }
        }
    }
    println!("{rays} rays over a refit and over a rebuild: {walked:?}");
    walked
}

#[test]
fn a_walk_over_a_refit_finds_what_a_walk_over_a_rebuild_finds() {
    let walked = walked(300);
    assert_eq!((walked.closest, walked.first), (300, 300), "{walked:?}");
    assert!(walked.hits > 15, "{walked:?}");
}

#[test]
fn a_triangle_is_posed_by_its_geometry_and_its_primitive() {
    // Two geometries of one quad each, three units apart along x, their triangles numbered from
    // nought in each as a model's are; the pose lifts each geometry to a height of its own.
    let quad = |geometry: u32, z: f32| {
        let x = 3.0 * geometry as f32;
        let corners = [
            [x, 0.0, z],
            [x + 1.0, 0.0, z],
            [x, 1.0, z],
            [x + 1.0, 1.0, z],
        ];
        [[0, 1, 2], [1, 3, 2]].into_iter().zip(0..).map(
            move |(at, primitive): ([usize; 3], u32)| Triangle {
                corners: at.map(|corner| corners[corner]),
                geometry,
                primitive,
                opaque: true,
            },
        )
    };
    let model = |heights: [f32; 2]| -> Vec<Triangle> {
        (0..2)
            .flat_map(|geometry| quad(geometry, heights[geometry as usize]))
            .collect()
    };
    let mut arena = Arena::new();
    let bottom = (arena.bottom_level(&model([0.0, 0.0]))).expect("a bottom level");
    let posed = (arena.refit_bottom(&bottom, &model([1.0, 2.0]))).expect("a bottom level refitted");
    assert_eq!((posed.min, posed.max), ([0.0, 0.0, 1.0], [4.0, 1.0, 2.0]));
    let words = arena.words();
    let nodes = leaves(words, bottom.base, bottom.root);
    assert_eq!(nodes.len(), 2, "a node a quad");
    for node in nodes {
        let at = bottom.base + offset(node);
        let geometry = (word(words, at, TRIANGLE_WORD) >> 8) as f32;
        // The node's four vertices lie over their own geometry's quad, at its own height.
        for vertex in 0..4 {
            let [x, _, z]: [f32; 3] = std::array::from_fn(|k| {
                f32::from_bits(word(
                    words,
                    at,
                    TRIANGLE_VERTEX_STRIDE * vertex + 4 * k as u32,
                ))
            });
            let over = (3.0 * geometry..=3.0 * geometry + 1.0).contains(&x);
            assert!(over && z == 1.0 + geometry, "geometry {geometry}: {x}, {z}");
        }
    }
}

#[test]
fn a_pose_a_structure_cannot_take_is_refused() {
    let was = scene();
    let grid = &was.triangles[0];
    let refit = |triangles: &[Triangle]| {
        was.arena
            .clone()
            .refit_bottom(&was.bottoms[0], triangles)
            .map(|_| ())
    };
    assert_eq!(refit(grid), Ok(()));
    // The grid's triangles share their nodes two by two: one moved alone tears its node.
    let mut torn = grid.clone();
    torn[0].corners = torn[0].corners.map(|corner| corner.map(|x| x + 0.5));
    assert!(
        matches!(refit(&torn), Err(AccelError::Torn(_))),
        "{:?}",
        refit(&torn)
    );
    let (geometry, primitive) = (grid[7].geometry, grid[7].primitive);
    let mut fewer = grid.clone();
    fewer.remove(7);
    assert_eq!(refit(&fewer), Err(AccelError::Unposed(geometry, primitive)));
    let mut endless = grid.clone();
    endless[3].corners[1][2] = f32::NAN;
    assert_eq!(refit(&endless), Err(AccelError::NotFinite(3)));

    let top = |instances: &[Instance]| was.arena.clone().refit_top(&was.top, instances).map(|_| ());
    assert_eq!(top(&was.instances), Ok(()));
    let last = was.instances.len() - 1;
    assert_eq!(top(&was.instances[..last]), Err(AccelError::Unplaced(last)));
    let mut flat = was.instances.clone();
    flat[2].object_to_world[1] = [0.0; 4];
    assert_eq!(top(&flat), Err(AccelError::Singular(2)));
}

/// The count a default run has no time for.
pub mod slow {
    #[test]
    fn four_thousand_walks_over_a_refit_find_what_they_find_over_a_rebuild() {
        let walked = super::walked(4000);
        assert_eq!((walked.closest, walked.first), (4000, 4000), "{walked:?}");
        assert!(walked.hits > 400, "{walked:?}");
    }
}
