use super::super::reference::{every_triangle, met, Verdict};
use super::super::scenes::{
    crossing, grid, line, placed, ray, scene, scene_by, stacked, Scene, Sequence, IDENTITY,
    RAY_MASK,
};
use super::super::{Arena, Split, Triangle};
use super::*;

/// A candidate's distance by the walk's own quotient.
fn distance(candidate: &Candidate) -> f32 {
    (1.0 / f32::from_bits(candidate.result[1])) * f32::from_bits(candidate.result[0])
}

/// The closest distance inside the ray's extent among `found`, by the walk's own two compares.
fn closest(found: &[Candidate], ray: &Ray) -> Option<f32> {
    let mut best = None;
    for t in found.iter().map(distance) {
        let outside = t < ray.t_min || t >= best.unwrap_or(ray.t_max);
        if !outside {
            best = Some(t);
        }
    }
    best
}

fn walked(scene: &Scene, ray: &Ray, walk: Walk) -> Outcome {
    walk_lane(scene.arena.words(), scene.top.base, ray, &walk, &mut |_| {
        true
    })
}

/// What `rays` rays of the scene gave.
#[derive(Debug, Default, PartialEq, Eq)]
struct Agreement {
    /// At a ring of 16 and of 4: the rays whose closest hit is the instruction's own closest to
    /// the distance's bits, and those whose first hit is one where the instruction finds any.
    bits: [(usize, usize); 2],
    /// Against the test of every triangle that shares nothing with the walk, for the closest
    /// hit, the first hit and the closest front face: the rays that are the same beyond doubt.
    same: [usize; 3],
    /// The rays an unsure triangle could answer, where the walk gave a possible answer.
    possible: [usize; 3],
}

/// The walk at a ring of 16 and of 4 against the instruction run on every triangle, and at 16
/// against the models' own triangles in double precision.
///
/// A ring of two is not held here: it loses a box's fourth child where the first three are
/// triangles, which `a_ring_of_two_loses_the_fourth_child_of_a_box_of_triangles` shows.
fn agreement(rays: usize) -> Agreement {
    agreement_by(Arena::new().split(), rays)
}

/// [`agreement`] over the scene built by `split`.
fn agreement_by(split: Split, rays: usize) -> Agreement {
    let scene = scene_by(split);
    let world = scene.world();
    let (words, top) = (scene.arena.words(), scene.top.base);
    let mut sequence = Sequence(777);
    let mut agree = Agreement::default();
    let mut hits = 0;
    let masked = Walk {
        mask: RAY_MASK,
        ..Walk::default()
    };
    for _ in 0..rays {
        let ray = ray(&mut sequence);
        let found = candidates(words, top, &ray, RAY_MASK);
        let want = closest(&found, &ray).map(f32::to_bits);
        hits += usize::from(want.is_some());
        for (slot, ring) in [16, 4].into_iter().enumerate() {
            let near = walked(&scene, &ray, Walk { ring, ..masked });
            agree.bits[slot].0 += usize::from(near.hit.map(|h| h.t.to_bits()) == want);
            let first_hit = true;
            let any = walked(
                &scene,
                &ray,
                Walk {
                    ring,
                    first_hit,
                    ..masked
                },
            );
            let stated = any
                .hit
                .is_none_or(|h| found.iter().any(|c| distance(c) == h.t));
            agree.bits[slot].1 += usize::from(any.hit.is_some() == want.is_some() && stated);
        }
        let named = |out: Outcome| out.hit.map(|hit| met(words, top, &hit));
        let all = every_triangle(&world, &ray, RAY_MASK, Cull::None);
        let front = every_triangle(&world, &ray, RAY_MASK, Cull::Back);
        let cull = Cull::Back;
        // To the first hit with no triangle opaque, so that the candidate code is asked of every
        // hit the walk could commit, in the order it walked.
        let unforced = Walk {
            first_hit: true,
            opacity: Opacity::NonOpaque,
            ..masked
        };
        let mut offered = Vec::new();
        let first = walk_lane(words, top, &ray, &unforced, &mut |hit| {
            offered.push(met(words, top, hit));
            true
        });
        let verdicts = [
            all.closest(named(walked(&scene, &ray, masked))),
            all.first(named(first), &offered),
            front.closest(named(walked(&scene, &ray, Walk { cull, ..masked }))),
        ];
        for (slot, verdict) in verdicts.into_iter().enumerate() {
            agree.same[slot] += usize::from(verdict == Verdict::Same);
            agree.possible[slot] += usize::from(verdict == Verdict::Possible);
        }
    }
    assert!(hits > rays / 10 && hits < rays, "{hits} of {rays} rays hit");
    println!("{rays} rays, {hits} hit: {agree:?}");
    agree
}

/// Whether every ray agrees to the bit, none differs from every triangle, and no more than one
/// ray in twenty is one an unsure triangle could answer.
fn holds(agree: &Agreement, rays: usize) -> bool {
    let apart = |slot: usize| agree.same[slot] + agree.possible[slot] == rays;
    agree.bits == [(rays, rays); 2]
        && (0..3).all(|slot| apart(slot) && 20 * agree.possible[slot] <= rays)
}

#[test]
fn the_walk_finds_what_every_triangle_finds() {
    let agree = agreement(150);
    assert!(holds(&agree, 150), "{agree:?}");
}

#[test]
fn the_walk_finds_what_every_triangle_finds_in_a_tree_of_either_split() {
    for split in [Split::Median, Split::SurfaceArea] {
        let agree = agreement_by(split, 60);
        assert!(holds(&agree, 60), "{split:?}: {agree:?}");
    }
}

/// The same at the counts the plan states, which a default run has no time for.
mod slow {
    #[test]
    fn the_walk_finds_what_every_triangle_finds_over_four_thousand_rays() {
        let agree = super::agreement(4000);
        assert!(super::holds(&agree, 4000), "{agree:?}");
    }

    #[test]
    fn a_ray_down_every_grid_line_hits_at_the_distance_of_every_triangle() {
        assert_eq!(super::down_the_grid(1, false), [552; 4]);
    }

    #[test]
    fn a_ray_down_every_grid_corner_hits_at_the_distance_of_every_triangle() {
        assert_eq!(super::down_the_grid(1, true), [529; 4]);
    }
}

/// A ray that moves along z alone and lies exactly on a widened side of the box that holds the
/// triangle node `leaf` in its parent: whether the parent gives that child for it, and not for
/// the same ray a thousandth farther out. `None` where the node has no parent or no float lies
/// exactly on a side.
///
/// A box is widened by 2^-20 of the greatest distance in its test. From 2^30 up the z axis that
/// is 1024 whatever the box, so the side is a float and the test's own subtraction gives nought
/// there: a slab the rule does not divide by, and takes as closed.
fn on_the_skin(words: &[u32], base: u32, leaf: u32) -> Option<bool> {
    let entry = (leaf >> 3) - (word(words, base, HEADER_FIRST_NODE) >> 6);
    let parent = word(words, base, word(words, base, HEADER_PARENTS) + 4 * entry);
    if parent == INVALID {
        return None;
    }
    let at = base + offset(parent);
    let child = (0..4).find(|i| word(words, at, BOX_CHILDREN + 4 * i) == leaf & !7)?;
    let bound = |k: u32| {
        f32::from_bits(word(
            words,
            at,
            BOX_BOUNDS + BOX_BOUNDS_STRIDE * child + 4 * k,
        ))
    };
    let (far, grow) = (1_073_741_824.0f32, 1024.0f32);
    let meets = |x: f32, y: f32| {
        let direction = [0.0, 0.0, -2.0 * far];
        let inverse = direction.map(|d: f32| 1.0 / d);
        let found = intersect_ray(
            words,
            base,
            parent,
            f32::INFINITY,
            [x, y, far],
            direction,
            inverse,
        );
        found.contains(&(leaf & !7))
    };
    // On x and then on y, below the box and then above it: the first side a float lies on.
    for axis in 0..2u32 {
        let (low, high) = (bound(axis), bound(axis + 3));
        let middle = (bound(1 - axis) + bound(4 - axis)) / 2.0;
        let at = |on: f32| match axis {
            0 => meets(on, middle),
            _ => meets(middle, on),
        };
        for (side, out) in [(low, -1.0f32), (high, 1.0)] {
            // The floats about the side as the sum states it, from two farther out inwards.
            let mut on = side + out * grow;
            for _ in 0..2 {
                on = if out < 0.0 {
                    on.next_down()
                } else {
                    on.next_up()
                };
            }
            for _ in 0..5 {
                if ((side - on) * -out - grow) == 0.0 {
                    return Some(at(on) && !at(on + out * 0.001));
                }
                on = if out < 0.0 {
                    on.next_up()
                } else {
                    on.next_down()
                };
            }
        }
    }
    None
}

/// Rays down every `step`th line of the scene's unmoved grid, between two crossings or on each
/// `corner`: how many, how many of them every triangle says hit, how many the walk answers at
/// that distance to the bit, and for how many the box that holds the grid's own triangle under
/// the ray meets a ray that lies on its widened side.
fn down_the_grid(step: usize, corner: bool) -> [usize; 4] {
    let scene = scene();
    let words = scene.arena.words();
    let (n, size) = (24, 10.0);
    let (mut rays, mut hit, mut agree, mut skin) = (0, 0, 0, 0);
    for i in (step..n).step_by(step) {
        for j in usize::from(corner)..n {
            // On a line of the grid in x, where two quads' edges and two boxes' planes lie, and
            // with no movement in x or y: the grid is where it was made, so the instance's space
            // is the world's to the bit.
            let y = match corner {
                true => line(j, n, size),
                false => (line(j, n, size) + line(j + 1, n, size)) / 2.0 + 0.01,
            };
            let ray = Ray {
                origin: [line(i, n, size), y, 5.0],
                direction: [0.0, 0.0, -10.0],
                t_min: 0.0,
                t_max: 1.0,
            };
            let found = candidates(words, scene.top.base, &ray, RAY_MASK);
            let want = closest(&found, &ray);
            let masked = Walk {
                mask: RAY_MASK,
                ..Walk::default()
            };
            let near = walked(&scene, &ray, masked);
            rays += 1;
            hit += usize::from(want.is_some());
            agree += usize::from(near.hit.map(|h| h.t.to_bits()) == want.map(f32::to_bits));
            // The grid where it was made is the last instance placed.
            let unmoved = scene.instances.len() as u32 - 1;
            let number = |c: &&Candidate| {
                word(words, scene.top.base + offset(c.instance), INSTANCE_INDEX) == unmoved
            };
            let held = (found.iter().find(number))
                .and_then(|c| on_the_skin(words, scene.bottoms[0].base, c.leaf));
            skin += usize::from(held == Some(true));
        }
    }
    [rays, hit, agree, skin]
}

#[test]
fn a_ray_down_a_grid_line_or_corner_hits_at_the_distance_of_every_triangle() {
    assert_eq!(down_the_grid(6, false), [72; 4]);
    assert_eq!(down_the_grid(6, true), [69; 4]);
}

#[test]
fn a_stack_deeper_than_the_ring_is_climbed_by_the_parent_table() {
    // Seven levels of boxes whose four children a ray along z all meets leave three children
    // waiting a level: more than the ring's sixteen entries.
    let (arena, top) = stacked(1 << 14, 0.25);
    let top = top.base;
    let mut sequence = Sequence(5);
    let (mut climbed, mut deepest) = (0, 0);
    for index in 0..40 {
        // Every other ray starts its extent three quarters up the stack, so its hit is under
        // entries the ring has lost by the time the walk comes back for them.
        let ray = Ray {
            origin: [sequence.within(3.0), sequence.within(3.0), -1.0],
            direction: [sequence.within(0.01), sequence.within(0.01), 5000.0],
            t_min: if index % 2 == 0 { 0.0 } else { 0.7 },
            t_max: 1.0,
        };
        let found = candidates(arena.words(), top, &ray, 0xFF);
        let want = closest(&found, &ray).map(f32::to_bits);
        for first_hit in [false, true] {
            let walk = Walk {
                first_hit,
                ..Walk::default()
            };
            let out = walk_lane(arena.words(), top, &ray, &walk, &mut |_| true);
            match first_hit {
                false => assert_eq!(out.hit.map(|h| h.t.to_bits()), want, "ray {index}"),
                true => assert_eq!(out.hit.is_some(), want.is_some(), "ray {index}"),
            }
            assert!(out.turns < walk.bound - 1, "ray {index} ran to the bound");
            climbed += out.climbs;
            deepest = deepest.max(out.deepest);
        }
    }
    assert!(
        climbed > 0 && deepest > 16,
        "{climbed} climbs, depth {deepest}"
    );
}

#[test]
fn a_box_s_children_are_walked_triangles_first_then_boxes_each_nearest_first() {
    // Five triangles four apart: the root holds the lowest three and a box of the upper two. A
    // ray from above meets the box first, and the walk still takes the root's triangles before
    // it, the nearest first.
    let (arena, top) = stacked(5, 4.0);
    let (words, top, lane) = (arena.words(), top.base, 3);
    let ray = Ray {
        origin: [0.0, 0.0, 20.0],
        direction: [0.0, 0.0, -40.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let mut order = Vec::new();
    let walk = Walk {
        lane,
        ..Walk::default()
    };
    let out = walk_lane(words, top, &ray, &walk, &mut |hit| {
        order.push(met(words, top, hit).primitive);
        false
    });
    assert_eq!(order, [2, 1, 0, 4, 3]);
    // The instance, the root, its three triangles, the box and its two; an emptied stack is back
    // at the lane's own address.
    assert_eq!((out.hit, out.turns, out.stack), (None, 8, lane * 4));
    // The first hit a walk ends on is the first of that order.
    let first = Walk {
        first_hit: true,
        ..walk
    };
    let out = walk_lane(words, top, &ray, &first, &mut |_| true);
    let named = out.hit.map(|hit| met(words, top, &hit).primitive);
    assert_eq!((named, out.turns), (Some(2), 3));
}

#[test]
fn a_ring_of_two_loses_the_fourth_child_of_a_box_of_triangles() {
    // Four triangles under one box, and an extent that holds the farthest alone. The three that
    // wait are pushed onto two entries, the third over the first, and nothing is left to climb
    // back from: the walk never comes to the fourth.
    let (arena, top) = stacked(4, 4.0);
    let top = top.base;
    let ray = Ray {
        origin: [0.0, 0.0, -4.0],
        direction: [0.0, 0.0, 20.0],
        t_min: 0.7,
        t_max: 1.0,
    };
    let hit = |ring: u32| {
        let walk = Walk {
            ring,
            ..Walk::default()
        };
        let out = walk_lane(arena.words(), top, &ray, &walk, &mut |_| true);
        let named = out.hit.map(|hit| met(arena.words(), top, &hit).primitive);
        (named, out.climbs > 0)
    };
    assert_eq!(hit(16), (Some(3), false));
    assert_eq!(hit(4), (Some(3), false));
    assert_eq!(hit(2), (None, true));
}

/// One triangle facing +z, placed once with `flags`, and a ray down onto its front.
fn one(flags: u8, opaque: bool) -> (Arena, u32, Ray) {
    let mut arena = Arena::new();
    let triangle = Triangle {
        corners: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque,
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let mut instance = placed(bottom, IDENTITY, [10.0, 0.0, 0.0], 0x04);
    instance.flags = flags;
    let top = arena.top_level(&[instance]).expect("a top level");
    let ray = Ray {
        origin: [10.25, 0.25, 1.0],
        direction: [0.0, 0.0, -1.0],
        t_min: 0.0,
        t_max: 4.0,
    };
    (arena, top.base, ray)
}

#[test]
fn a_hit_is_held_to_the_extent_the_mask_the_face_and_the_opacity() {
    let hit = |flags: u8, opaque: bool, walk: Walk, ray: &dyn Fn(Ray) -> Ray, commit: bool| {
        let (arena, top, down) = one(flags, opaque);
        let mut asked = 0;
        let out = walk_lane(arena.words(), top, &ray(down), &walk, &mut |_| {
            asked += 1;
            commit
        });
        (out.hit.map(|h| (h.t, h.kind, h.weights)), asked)
    };
    let same = |ray: Ray| ray;
    let any = Walk::default();
    assert_eq!(
        hit(0, true, any, &same, true),
        (Some((1.0, 0xFE, [0.25, 0.25])), 0)
    );
    // The extent: its far end is not inside it, its near end is.
    assert_eq!(
        hit(0, true, any, &|r| Ray { t_max: 1.0, ..r }, true).0,
        None
    );
    assert_eq!(
        hit(0, true, any, &|r| Ray { t_min: 1.0, ..r }, true)
            .0
            .map(|h| h.0),
        Some(1.0)
    );
    assert_eq!(
        hit(0, true, any, &|r| Ray { t_min: 1.5, ..r }, true).0,
        None
    );
    // The mask: the instance answers to 0x04.
    assert_eq!(
        hit(0, true, Walk { mask: 0xFB, ..any }, &same, true).0,
        None
    );
    assert_eq!(
        hit(0, true, Walk { mask: 0x04, ..any }, &same, true)
            .0
            .map(|h| h.0),
        Some(1.0)
    );
    // The face: a ray from above meets the front, and the instance's flags turn or drop the cull.
    let (back, front) = (
        Walk {
            cull: Cull::Back,
            ..any
        },
        Walk {
            cull: Cull::Front,
            ..any
        },
    );
    let flip = FLAG_FLIP_WINDING as u8;
    assert_eq!(hit(0, true, back, &same, true).0.map(|h| h.1), Some(0xFE));
    assert_eq!(hit(0, true, front, &same, true).0, None);
    assert_eq!(hit(flip, true, any, &same, true).0.map(|h| h.1), Some(0xFF));
    assert_eq!(hit(flip, true, back, &same, true).0, None);
    assert_eq!(
        hit(flip, true, front, &same, true).0.map(|h| h.1),
        Some(0xFF)
    );
    assert_eq!(
        hit(FLAG_NO_CULL as u8, true, front, &same, true)
            .0
            .map(|h| h.1),
        Some(0xFE)
    );
    // The opacity: a triangle that is not opaque is the candidate's to commit.
    assert_eq!(
        hit(0, false, any, &same, true),
        (Some((1.0, 0xFE, [0.25, 0.25])), 1)
    );
    assert_eq!(hit(0, false, any, &same, false), (None, 1));
    assert_eq!(hit(FLAG_FORCE_OPAQUE as u8, false, any, &same, false).1, 0);
    assert_eq!(
        hit(FLAG_FORCE_NON_OPAQUE as u8, true, any, &same, false),
        (None, 1)
    );
    let forced = |opacity| Walk { opacity, ..any };
    assert_eq!(hit(0, false, forced(Opacity::Opaque), &same, false).1, 0);
    assert_eq!(
        hit(0, true, forced(Opacity::NonOpaque), &same, false),
        (None, 1)
    );
}

#[test]
fn an_instance_that_states_no_rotation_turns_a_ray_by_its_translation_alone() {
    let (arena, top, ray) = one(0, true);
    let mut words = arena.words().to_vec();
    let at = top + offset(word(&words, top, HEADER_ROOT));
    let hit = |words: &[u32]| {
        let out = walk_lane(words, top, &ray, &Walk::default(), &mut |_| true);
        out.hit.map(|h| h.t)
    };
    // Rows that are not the identity are not read while the node says its rotation is.
    assert_ne!(word(&words, at, INSTANCE_ROOT) & INSTANCE_IDENTITY, 0);
    for k in 0..9 {
        words[((at + INSTANCE_ROTATION) / 4 + k) as usize] = 2.0f32.to_bits();
    }
    assert_eq!(hit(&words), Some(1.0));
    words[((at + INSTANCE_ROOT) / 4) as usize] &= !INSTANCE_IDENTITY;
    assert_ne!(hit(&words), Some(1.0));
}

#[test]
fn an_instance_the_mask_leaves_out_is_still_the_instance_last_met() {
    // Four instances under one box, none of which answers to the ray's mask, and a ring of two:
    // the third push overwrites the first, the pop that finds it gone climbs to the box and goes
    // on to the fourth instance, and two more climbs end at the root's parent. The fourth
    // instance, though never entered, is still the instance last met there: the walk goes to its
    // parent, the box, once more before it ends.
    let mut arena = Arena::new();
    let triangle = Triangle {
        corners: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let instances = [0.0, 10.0, 20.0, 30.0].map(|x| placed(bottom, IDENTITY, [x, 0.0, 0.0], 0x01));
    let top = arena.top_level(&instances).expect("a top level");
    let ray = Ray {
        origin: [-5.0, 0.25, 0.0],
        direction: [50.0, 0.0, 0.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let walk = Walk {
        ring: 2,
        mask: 0x02,
        ..Walk::default()
    };
    let out = walk_lane(arena.words(), top.base, &ray, &walk, &mut |_| true);
    assert_eq!((out.hit, out.turns, out.climbs), (None, 8, 4));
}

#[test]
fn a_walk_ends_at_its_bound_and_states_its_stack_word() {
    let scene = scene();
    let mut sequence = Sequence(777);
    let ray = ray(&mut sequence);
    let walk = Walk {
        mask: RAY_MASK,
        lane: 5,
        ..Walk::default()
    };
    let whole = walked(&scene, &ray, walk);
    // An emptied stack is back at the lane's own address, at no depth, in no instance.
    assert!(whole.turns > 3 && whole.stack == 5 * 4, "{whole:?}");
    let short = walked(&scene, &ray, Walk { bound: 4, ..walk });
    assert_eq!(short.turns, 3);
}

#[test]
fn a_ray_at_an_edge_or_a_corner_inside_a_sheet_meets_the_sheet() {
    let (n, size, bump) = (24, 10.0, 0.4);
    let mut arena = Arena::new();
    let bottom = arena
        .bottom_level(&grid(n, size, bump))
        .expect("a bottom level");
    let top = (arena.top_level(&[placed(bottom, IDENTITY, [0.0; 3], 0xFF)])).expect("a top level");
    let mut sequence = Sequence(11);
    let (rays, mut met) = (4000, 0);
    for _ in 0..rays {
        // A corner no edge of the sheet's rim touches, and a point along one of the three kinds
        // of edge that leave it, or the corner itself.
        let i = 1 + (sequence.next() * (n - 2) as f32) as usize;
        let j = 1 + (sequence.next() * (n - 2) as f32) as usize;
        let (to, along) = match (sequence.next() * 4.0) as usize {
            0 => ((i, j), 0.0),
            1 => ((i + 1, j), sequence.next()),
            2 => ((i, j + 1), sequence.next()),
            _ => ((i - 1, j + 1), sequence.next()),
        };
        let (from, to) = (
            crossing(i, j, n, size, bump),
            crossing(to.0, to.1, n, size, bump),
        );
        let aim: [f32; 3] = std::array::from_fn(|k| from[k] + along * (to[k] - from[k]));
        // From steeply above: steeper than any face, so the ray goes through the sheet there
        // and does not only touch a ridge from outside.
        let origin = [
            aim[0] + sequence.within(20.0),
            aim[1] + sequence.within(20.0),
            aim[2] + 40.0 + sequence.next() * 60.0,
        ];
        let ray = Ray {
            origin,
            direction: std::array::from_fn(|k| (aim[k] - origin[k]) * 2.0),
            t_min: 0.0,
            t_max: 1.0,
        };
        met += usize::from(!candidates(arena.words(), top.base, &ray, 0xFF).is_empty());
    }
    assert_eq!(met, rays, "rays that met the sheet");
}

/// An instance node alone at address nought that states `rows` and `translation`.
fn node(rows: [[f32; 3]; 3], translation: [f32; 3]) -> Vec<u32> {
    let mut words = vec![0u32; (INSTANCE_BYTES / 4) as usize];
    for (k, x) in rows.iter().flatten().enumerate() {
        words[(INSTANCE_ROTATION / 4) as usize + k] = x.to_bits();
    }
    for (k, x) in translation.iter().enumerate() {
        words[(INSTANCE_TRANSLATION / 4) as usize + k] = x.to_bits();
    }
    words
}

/// How a term of a row is put onto the sum before it.
#[derive(Clone, Copy, PartialEq)]
enum Put {
    /// The product rounded, then the sum rounded.
    Rounded,
    /// The product and the sum rounded once.
    Fused,
}

/// A row's sum as a walk states it, in double precision with no fused operation: from `start`
/// where one is given, each term of `order` put on by its own rule, `end` added last.
///
/// Every factor here is a float of 1 to 2 either side of nought, so a product is a whole number
/// of 2^-46 below 4 and every sum of them is whole in a double: a double's one rounding to a
/// float is the fused operation's.
fn row(
    m: [f32; 3],
    v: [f32; 3],
    order: [usize; 3],
    start: Option<f32>,
    put: [Put; 3],
    end: Option<f32>,
) -> f32 {
    let mut sum = start;
    for (k, put) in order.into_iter().zip(put) {
        let product = f64::from(m[k]) * f64::from(v[k]);
        sum = Some(match (sum, put) {
            (None, _) => product as f32,
            (Some(sum), Put::Fused) => (product + f64::from(sum)) as f32,
            (Some(sum), Put::Rounded) => product as f32 + sum,
        });
    }
    sum.expect("three terms") + end.unwrap_or(0.0)
}

/// The order of a row's terms in the wave-shared walk.
const WAVE_ORDER: [[usize; 3]; 3] = [[0, 1, 2], [1, 0, 2], [1, 0, 2]];

#[test]
fn a_ray_s_turn_into_an_instance_fuses_the_products_of_a_row() {
    use Put::{Fused, Rounded};
    let mut sequence = Sequence(31);
    let mut factor = || (1.0 + sequence.next()) * if sequence.next() < 0.5 { -1.0 } else { 1.0 };
    let cases = 400;
    // The cases in which a row computed another way differs from the walk's own: a test that
    // cannot tell the two apart holds nothing of that step.
    let mut apart = [0usize; 12];
    for _ in 0..cases {
        let rows: [[f32; 3]; 3] = [0; 3].map(|_| [0; 3].map(|_| factor()));
        let translation = [0; 3].map(|_| factor());
        let (o, d) = ([0; 3].map(|_| factor()), [0; 3].map(|_| factor()));
        let words = node(rows, translation);
        let each = |f: &dyn Fn(usize) -> f32| -> [f32; 3] { std::array::from_fn(f) };

        // The per-lane walk rounds a row's first product, fuses the second and the third onto
        // it, and adds the translation last.
        let lane = |v: [f32; 3], t: Option<[f32; 3]>, put: [Put; 3]| {
            each(&|r| row(rows[r], v, [0, 1, 2], None, put, t.map(|t| t[r])))
        };
        let own = [Rounded, Fused, Fused];
        let want = (lane(o, Some(translation), own), lane(d, None, own));
        assert_eq!(turned(&words, 0, o, d), want);
        for (slot, put) in [[Rounded, Rounded, Fused], [Rounded, Fused, Rounded]]
            .into_iter()
            .enumerate()
        {
            apart[slot] += usize::from(lane(d, None, put) != want.1);
        }

        // The wave-shared walk starts its second and third rows from their middle term; a
        // direction's first product is rounded, an origin's is fused onto the translation.
        let wave = |v: [f32; 3], t: Option<[f32; 3]>, order: [[usize; 3]; 3], put: [Put; 3]| {
            each(&|r| row(rows[r], v, order[r], t.map(|t| t[r]), put, None))
        };
        let fused = [Fused; 3];
        let want = (
            wave(o, Some(translation), WAVE_ORDER, fused),
            wave(d, None, WAVE_ORDER, own),
        );
        assert_eq!(turned_wave(&words, 0, o, d), want);
        let swapped = [[0, 1, 2], [1, 2, 0], [1, 2, 0]];
        let others = [
            wave(d, None, WAVE_ORDER, [Rounded, Fused, Rounded]) != want.1,
            wave(d, None, swapped, own) != want.1,
            wave(d, None, [[0, 1, 2]; 3], own) != want.1,
            wave(o, Some(translation), WAVE_ORDER, [Fused, Fused, Rounded]) != want.0,
            wave(o, Some(translation), WAVE_ORDER, [Rounded, Fused, Fused]) != want.0,
            wave(o, Some(translation), swapped, fused) != want.0,
        ];
        for (slot, differs) in others.into_iter().enumerate() {
            apart[2 + slot] += usize::from(differs);
        }

        // The walk that fuses nothing rounds every product, takes a row's first two terms
        // before its third (the two commute) and adds the translation last.
        let plain = |v: [f32; 3], t: Option<[f32; 3]>, order: [usize; 3], put: [Put; 3]| {
            each(&|r| row(rows[r], v, order, None, put, t.map(|t| t[r])))
        };
        let (middle, none) = ([1, 0, 2], [Rounded; 3]);
        let want = (
            plain(o, Some(translation), middle, none),
            plain(d, None, middle, none),
        );
        assert_eq!(turned_plain(&words, 0, o, d), want);
        let others = [
            plain(d, None, [1, 2, 0], none) != want.1,
            plain(d, None, middle, [Rounded, Fused, Rounded]) != want.1,
            plain(d, None, middle, [Rounded, Rounded, Fused]) != want.1,
            each(&|r| row(rows[r], o, middle, Some(translation[r]), none, None)) != want.0,
        ];
        for (slot, differs) in others.into_iter().enumerate() {
            apart[8 + slot] += usize::from(differs);
        }
    }
    println!("of {cases} cases, a row computed another way differs in {apart:?}");
    assert!(
        apart.iter().all(|count| 4 * count > cases),
        "{apart:?} of {cases}"
    );

    // A first product with nought is nought whatever its other factor; a fused one is not.
    let words = node([[0.0, 1.0, 0.0]; 3], [0.0; 3]);
    let endless = [f32::INFINITY, 2.0, 3.0];
    assert_eq!(turned(&words, 0, [0.0; 3], endless).1, [2.0; 3]);
    assert_eq!(turned_wave(&words, 0, [0.0; 3], endless).1[0], 2.0);
    // The wave's second and third rows start from their middle term.
    let words = node([[1.0, 0.0, 0.0]; 3], [0.0; 3]);
    let (_, direction) = turned_wave(&words, 0, [0.0; 3], [5.0, f32::INFINITY, 3.0]);
    assert_eq!(direction[1..], [5.0; 2]);
    // The walk that fuses nothing multiplies plainly: nought times no number is no number.
    let words = node([[0.0, 1.0, 0.0]; 3], [0.0; 3]);
    assert!(turned_plain(&words, 0, [0.0; 3], endless).1[0].is_nan());
    // An instance that states no rotation is moved alone, in every form.
    let mut words = node([[2.0; 3]; 3], [1.0, 2.0, 3.0]);
    words[(INSTANCE_ROOT / 4) as usize] = INSTANCE_IDENTITY;
    for form in [Form::Lane, Form::Wave, Form::Plain] {
        let moved = turned_as(form, &words, 0, [10.0, 20.0, 30.0], [1.0, 2.0, 4.0]);
        assert_eq!(moved, ([11.0, 22.0, 33.0], [1.0, 2.0, 4.0]), "{form:?}");
    }
}

#[test]
fn a_lane_of_a_wave_of_sixty_four_steps_its_stack_word_by_twice_as_much() {
    // The five triangles of the order test, none opaque and none committed.
    let (arena, top) = stacked(5, 4.0);
    let (words, top) = (arena.words(), top.base);
    let ray = Ray {
        origin: [0.0, 0.0, 20.0],
        direction: [0.0, 0.0, -40.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let run = |lanes: u32, lane: u32, bound: u32| {
        let walk = Walk {
            lane,
            bound,
            ..Walk::default()
        };
        let mut order = Vec::new();
        let mut body = |probe: &Probe| probe.software(words);
        let out = walk_lane_among(words, top, &ray, &walk, lanes, &mut body, &mut |hit| {
            order.push(met(words, top, hit).primitive);
            false
        });
        (order, out)
    };
    for (lanes, step) in [(32, 0x80), (64, 0x100)] {
        for lane in [0, 5, 31] {
            // Stopped after the instance, the root and one triangle: of the three children that
            // waited the pop took one back, and two are left, each an entry of four bytes a
            // lane above the lane's own address.
            let (_, short) = run(lanes, lane, 4);
            assert_eq!((short.turns, short.stack), (3, lane * 4 + 2 * step));
            let (order, whole) = run(lanes, lane, 0xFFFF);
            assert_eq!(order, [2, 1, 0, 4, 3]);
            let ended = (whole.turns, whole.stack, whole.climbs);
            assert_eq!(ended, (8, lane * 4, 0), "{lanes} lanes");
        }
    }
    // A lane of 32 or more has its top bit where the depth is read: the depth it entered the
    // instance at is one, its emptied stack is not seen as empty, and it ends by climbing to
    // the root and out of the instance.
    for lane in [32, 40, 63] {
        let (_, short) = run(64, lane, 4);
        assert_eq!(short.stack, (lane * 4 + 2 * 0x100) | 1 << 20, "lane {lane}");
        let (order, whole) = run(64, lane, 0xFFFF);
        assert_eq!(order, [2, 1, 0, 4, 3], "lane {lane}");
        assert!(
            whole.climbs > 0 && whole.turns > 8,
            "lane {lane}: {whole:?}"
        );
    }
}
