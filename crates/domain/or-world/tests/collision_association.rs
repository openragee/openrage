//! Which polygons of the bounds under an object are proposed as the object's own, on bounds built by
//! hand: a box building on a flat floor, one with a floor of its own, one that shares a wall, a fence
//! post, a building turned forty-five degrees, a bound placed by a transform, and the ground asked for.

use std::sync::Arc;

use glam::{Affine3A, Quat, Vec3};
use or_io::gta5::{PhBoundAny, PhPolygon};
use or_world::collision::{
    associate, Association, AssociationOptions, BoundCollider, CollisionSet, LeafAssociation,
    NotJudged, OrientedBox, Reason, BOX_SLACK, GROUND_STEP, JUDGED_FROM, MOST, SHARED_CORNER,
};

#[path = "support/bounds.rs"]
mod bounds;

/// The floor reaches this far either way, in cells of [`CELL`].
const FLOOR_REACH: f32 = 30.0;
const CELL: f32 = 5.0;
/// The building's box in its own space: ten metres by eight, four tall, standing on its origin.
const BB_MIN: Vec3 = Vec3::ZERO;
const BB_MAX: Vec3 = Vec3::new(10.0, 8.0, 4.0);

/// A mesh under construction: polygons are numbered in the order they are added.
#[derive(Default)]
struct Town {
    positions: Vec<Vec3>,
    triangles: Vec<[u16; 3]>,
    primitives: Vec<PhPolygon>,
}

impl Town {
    fn vertex(&mut self, p: Vec3) -> u16 {
        self.positions.push(p);
        (self.positions.len() - 1) as u16
    }

    /// Adds a triangle and says which polygon it is.
    fn triangle(&mut self, corners: [Vec3; 3]) -> u32 {
        let named = corners.map(|p| self.vertex(p));
        self.triangles.push(named);
        (self.triangles.len() - 1) as u32
    }

    /// Adds the two triangles of a rectangle, `a b c d` around it.
    fn quad(&mut self, [a, b, c, d]: [Vec3; 4]) -> [u32; 2] {
        [self.triangle([a, b, c]), self.triangle([a, c, d])]
    }

    /// A level floor at `z` in cells, as far as [`FLOOR_REACH`], each cell the triangle `a b c` and then `a c d`; the polygons it added.
    fn floor(&mut self, z: f32) -> Vec<u32> {
        self.slope(z, 0.0)
    }

    /// [`Town::floor`] that rises `rise` for each metre along x, from `z` over the origin.
    fn slope(&mut self, z: f32, rise: f32) -> Vec<u32> {
        let cells = (2.0 * FLOOR_REACH / CELL) as i32;
        let mut added = Vec::new();
        for i in 0..cells {
            for j in 0..cells {
                let (x, y) = (
                    -FLOOR_REACH + i as f32 * CELL,
                    -FLOOR_REACH + j as f32 * CELL,
                );
                let at = |x: f32, y: f32| Vec3::new(x, y, z + rise * x);
                let [a, b, c, d] = [
                    at(x, y),
                    at(x + CELL, y),
                    at(x + CELL, y + CELL),
                    at(x, y + CELL),
                ];
                // ⚠ Every other cell is wound to face down: which way a triangle is wound is not which way is up.
                if (i + j) % 2 == 0 {
                    added.extend(self.quad([a, b, c, d]));
                } else {
                    added.extend([self.triangle([a, c, b]), self.triangle([a, d, c])]);
                }
            }
        }
        added
    }

    /// The upright wall from `from` to `to` on the ground, `foot` to `top` high, placed by `place`.
    fn wall(
        &mut self,
        place: Affine3A,
        from: [f32; 2],
        to: [f32; 2],
        foot: f32,
        top: f32,
    ) -> [u32; 2] {
        let at = |p: [f32; 2], z: f32| place.transform_point3(Vec3::new(p[0], p[1], z));
        self.quad([at(from, foot), at(to, foot), at(to, top), at(from, top)])
    }

    /// The level rectangle of the building's plan at `z`, placed by `place`.
    fn slab(&mut self, place: Affine3A, z: f32) -> [u32; 2] {
        let at = |x: f32, y: f32| place.transform_point3(Vec3::new(x, y, z));
        self.quad([
            at(BB_MIN.x, BB_MIN.y),
            at(BB_MAX.x, BB_MIN.y),
            at(BB_MAX.x, BB_MAX.y),
            at(BB_MIN.x, BB_MAX.y),
        ])
    }

    /// The four walls and the roof of the building, placed by `place`; the polygons they are.
    fn building(&mut self, place: Affine3A) -> Vec<u32> {
        let (lo, hi) = (BB_MIN, BB_MAX);
        let mut own = Vec::new();
        for (from, to) in [
            ([lo.x, lo.y], [hi.x, lo.y]),
            ([hi.x, lo.y], [hi.x, hi.y]),
            ([hi.x, hi.y], [lo.x, hi.y]),
            ([lo.x, hi.y], [lo.x, lo.y]),
        ] {
            own.extend(self.wall(place, from, to, lo.z, hi.z));
        }
        own.extend(self.slab(place, hi.z));
        own
    }

    fn bound(self) -> PhBoundAny {
        let mut g = bounds::geometry(&self.positions, &self.triangles, Vec::new(), &[], &[]);
        g.polygons.extend(self.primitives);
        PhBoundAny::PhBoundGeometry(g)
    }
}

fn set_of(bound: PhBoundAny, place: Affine3A) -> CollisionSet<&'static str> {
    let collider = BoundCollider::new(&bound).expect("a bound built by hand is whole");
    let mut set = CollisionSet::new();
    set.add("area", Arc::new(collider), place)
        .expect("a placement");
    set
}

/// The building's box under `place`.
fn object(place: Affine3A) -> OrientedBox {
    let (_, rotation, position) = place.to_scale_rotation_translation();
    OrientedBox::placed(BB_MIN, BB_MAX, position, rotation, Vec3::ONE)
}

const AS_THE_RULE_HAS_IT: AssociationOptions = AssociationOptions { take_ground: false };

/// The one leaf of the one bound of an answer.
fn leaf<'a>(answer: &'a Association<&'static str>) -> &'a LeafAssociation {
    assert!(answer.not_judged.is_none(), "{:?}", answer.not_judged);
    let [bound] = answer.bounds.as_slice() else {
        panic!(
            "one bound was placed, and {} are answered",
            answer.bounds.len()
        );
    };
    assert_eq!(bound.key, "area");
    let [leaf] = bound.leaves.as_slice() else {
        panic!("the bound is one geometry");
    };
    assert_eq!(leaf.leaf, 0);
    leaf
}

fn proposed(leaf: &LeafAssociation) -> Vec<u32> {
    leaf.proposed.iter().filter_map(|p| p.polygon).collect()
}

fn left_as_ground(leaf: &LeafAssociation) -> Vec<u32> {
    leaf.left
        .iter()
        .filter(|left| left.reason == Reason::Ground)
        .filter_map(|left| left.polygon)
        .collect()
}

/// The floor triangles a level rectangle from `lo` to `hi` reaches: those wholly in it and those it cuts.
///
/// By points and not by the axes the code under test parts shapes with: a triangle is in the rectangle when its corners are, and
/// reaches it when a point of a grid over the rectangle, finer than the slack, lies in the triangle.
fn floor_under(lo: [f32; 2], hi: [f32; 2]) -> (Vec<u32>, Vec<u32>) {
    const STEP: f32 = 0.02;
    const { assert!(STEP < BOX_SLACK / 2.0) };
    let cells = (2.0 * FLOOR_REACH / CELL) as i32;
    let steps = |a: usize| ((hi[a] - lo[a]) / STEP).ceil() as i32;
    let grid: Vec<[f32; 2]> = (0..=steps(0))
        .flat_map(|i| {
            (0..=steps(1)).map(move |j| {
                [
                    (lo[0] + i as f32 * STEP).min(hi[0]),
                    (lo[1] + j as f32 * STEP).min(hi[1]),
                ]
            })
        })
        .collect();
    let side = |p: [f32; 2], a: [f32; 2], b: [f32; 2]| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let holds = |[a, b, c]: [[f32; 2]; 3], p: [f32; 2]| {
        let turns = [side(p, a, b), side(p, b, c), side(p, c, a)];
        turns.iter().all(|t| *t >= 0.0) || turns.iter().all(|t| *t <= 0.0)
    };
    let (mut held, mut cut) = (Vec::new(), Vec::new());
    for i in 0..cells {
        for j in 0..cells {
            let (x, y) = (
                -FLOOR_REACH + i as f32 * CELL,
                -FLOOR_REACH + j as f32 * CELL,
            );
            let [a, b, c, d] = [[x, y], [x + CELL, y], [x + CELL, y + CELL], [x, y + CELL]];
            // Before the grid is walked: most of the floor is nowhere near the rectangle.
            if x > hi[0] || x + CELL < lo[0] || y > hi[1] || y + CELL < lo[1] {
                continue;
            }
            for (k, triangle) in [[a, b, c], [a, c, d]].into_iter().enumerate() {
                let polygon = (2 * (i * cells + j)) as u32 + k as u32;
                let within = triangle
                    .iter()
                    .all(|p| (0..2).all(|a| p[a] >= lo[a] && p[a] <= hi[a]));
                if within {
                    held.push(polygon);
                } else if grid.iter().any(|p| holds(triangle, *p)) {
                    cut.push(polygon);
                }
            }
        }
    }
    (held, cut)
}

#[test]
fn a_box_building_on_a_flat_floor_gives_its_walls_and_its_roof_and_keeps_the_floor() {
    let mut town = Town::default();
    let floor = town.floor(0.0);
    let own = town.building(Affine3A::IDENTITY);
    let set = set_of(town.bound(), Affine3A::IDENTITY);

    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    assert_eq!(answer.height, BB_MAX.z - BB_MIN.z);
    let leaf = leaf(&answer);
    assert_eq!(proposed(leaf), own, "the four walls and the roof");
    assert!(
        leaf.proposed.iter().all(|p| p.share.is_none()),
        "each wholly in the box"
    );

    // The floor under the building, in cells the box holds whole and cells it cuts, is left as ground; the floor around it is not
    // reached, so nothing is said of it.
    let grown = |by: f32| {
        (
            [BB_MIN.x - by, BB_MIN.y - by],
            [BB_MAX.x + by, BB_MAX.y + by],
        )
    };
    let (lo, hi) = grown(BOX_SLACK);
    let (held, cut) = floor_under(lo, hi);
    assert!(
        !held.is_empty() && !cut.is_empty(),
        "the box holds floor and cuts floor"
    );
    let mut under: Vec<u32> = held.iter().chain(&cut).copied().collect();
    under.sort_unstable();
    assert_eq!(left_as_ground(leaf), under);
    assert_eq!(leaf.left.len(), under.len(), "and nothing else is left");
    for left in &leaf.left {
        let polygon = left.polygon.expect("a polygon of a geometry");
        assert_eq!(left.cut, cut.contains(&polygon), "polygon {polygon}");
    }
    assert!(
        under.len() < floor.len(),
        "the floor goes on around the box"
    );
    assert_eq!(answer.proposed(), own.len());
    assert_eq!(answer.left(), under.len());
}

#[test]
fn a_floor_of_the_buildings_own_goes_with_it_once_its_walls_reach_under_it_by_more_than_a_step() {
    // What the rule does of a floor raised above the ground: the walls stand on the ground, so their foot is the ground's height, and
    // a floor higher above it than the step is the building's and is proposed; one within the step is ground and stays.
    let with_floor_at = |z: f32| {
        let mut town = Town::default();
        town.floor(0.0);
        let own = town.building(Affine3A::IDENTITY);
        let raised = town.slab(Affine3A::IDENTITY, z);
        let set = set_of(town.bound(), Affine3A::IDENTITY);
        let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
        let leaf = leaf(&answer).clone();
        (own, raised, leaf)
    };

    let (own, raised, leaf) = with_floor_at(GROUND_STEP * 2.0);
    let mut both = own.clone();
    both.extend(raised);
    assert_eq!(proposed(&leaf), both, "the raised floor is the building's");
    assert!(raised.iter().all(|p| !left_as_ground(&leaf).contains(p)));

    let (own, raised, leaf) = with_floor_at(GROUND_STEP * 0.8);
    assert_eq!(proposed(&leaf), own, "a floor within the step stays");
    assert!(raised.iter().all(|p| left_as_ground(&leaf).contains(p)));

    // The ground under either floor stays, whichever the floor is.
    let (held, _) = floor_under([BB_MIN.x, BB_MIN.y], [BB_MAX.x, BB_MAX.y]);
    assert!(held.iter().all(|p| left_as_ground(&leaf).contains(p)));
}

#[test]
fn a_level_surface_with_nothing_standing_in_the_box_is_ground_whatever_its_height() {
    // The other side of the rule: the same level rectangle that is a roof over walls is ground where nothing stands under it.
    let mut town = Town::default();
    town.floor(0.0);
    let deck = town.slab(Affine3A::IDENTITY, BB_MAX.z);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    assert!(leaf.proposed.is_empty(), "{:?}", leaf.proposed);
    assert!(deck.iter().all(|p| left_as_ground(leaf).contains(p)));
}

#[test]
fn a_wall_shared_with_a_neighbour_and_half_in_the_box_is_listed_as_cut_and_not_proposed() {
    let mut town = Town::default();
    town.floor(0.0);
    let (lo, hi) = (BB_MIN, BB_MAX);
    let mut own = Vec::new();
    for (from, to) in [
        ([lo.x, lo.y], [hi.x, lo.y]),
        ([hi.x, hi.y], [lo.x, hi.y]),
        ([lo.x, hi.y], [lo.x, lo.y]),
    ] {
        own.extend(town.wall(Affine3A::IDENTITY, from, to, lo.z, hi.z));
    }
    own.extend(town.slab(Affine3A::IDENTITY, hi.z));
    // The fourth wall goes on as far again along the neighbour.
    let length = 2.0 * (hi.y - lo.y);
    let shared = town.wall(
        Affine3A::IDENTITY,
        [hi.x, lo.y],
        [hi.x, lo.y + length],
        lo.z,
        hi.z,
    );
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    assert_eq!(proposed(leaf), own, "its own three walls and its roof");

    // The box ends halfway along the wall, a slack past it. The triangle that holds the far foot of the wall has in the box the
    // part similar to itself, and the other triangle the rest of the rectangle's near half.
    let along = (hi.y - lo.y + BOX_SLACK) / length;
    let shares = [along * along, 1.0 - (1.0 - along) * (1.0 - along)];
    assert!(shares[0] < 0.5 && shares[1] > 0.5 && shares[1] < MOST);
    for (polygon, share) in shared.iter().zip(shares) {
        let left = leaf
            .left
            .iter()
            .find(|left| left.polygon == Some(*polygon))
            .unwrap_or_else(|| panic!("polygon {polygon} of the shared wall is listed"));
        assert!(left.cut);
        let Reason::MostlyOutside { share: measured } = left.reason else {
            panic!("{:?}", left.reason);
        };
        assert!(
            (measured - share).abs() < 1e-3,
            "polygon {polygon}: {measured} of it in the box for {share}"
        );
    }
}

#[test]
fn a_triangle_all_but_a_sliver_in_the_box_is_proposed_with_its_share() {
    let mut town = Town::default();
    town.floor(0.0);
    let own = town.building(Affine3A::IDENTITY);
    // An upright fin inside the building whose tip pokes through the roof.
    let above = 0.2 + BOX_SLACK;
    let fin = town.triangle([
        Vec3::new(4.0, 4.0, 0.0),
        Vec3::new(6.0, 4.0, 0.0),
        Vec3::new(5.0, 4.0, BB_MAX.z + above),
    ]);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    let mut both = own;
    both.push(fin);
    assert_eq!(proposed(leaf), both);
    let tip = 0.2 / (BB_MAX.z + above);
    let share = leaf
        .proposed
        .iter()
        .find(|p| p.polygon == Some(fin))
        .and_then(|p| p.share)
        .expect("a cut triangle states its share");
    assert!(
        share >= MOST && (share - (1.0 - tip * tip)).abs() < 1e-3,
        "{share}"
    );
}

#[test]
fn a_fence_post_under_the_height_limit_is_not_judged_and_the_answer_says_why() {
    let mut town = Town::default();
    town.floor(0.0);
    let post = Affine3A::from_translation(Vec3::new(3.0, 3.0, 0.0));
    let (half, height) = (0.1, 1.2);
    for (from, to) in [
        ([-half, -half], [half, -half]),
        ([half, -half], [half, half]),
        ([half, half], [-half, half]),
        ([-half, half], [-half, -half]),
    ] {
        town.wall(post, from, to, 0.0, height);
    }
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let stands = |height: f32| {
        OrientedBox::placed(
            Vec3::new(-half, -half, 0.0),
            Vec3::new(half, half, height),
            Vec3::new(3.0, 3.0, 0.0),
            Quat::IDENTITY,
            Vec3::ONE,
        )
    };
    assert!(height < JUDGED_FROM);
    let answer = associate(&set, &stands(height), &AS_THE_RULE_HAS_IT);
    assert_eq!(
        answer.not_judged,
        Some(NotJudged::TooLow {
            height,
            limit: JUDGED_FROM
        })
    );
    assert!(answer.bounds.is_empty());
    assert_eq!((answer.proposed(), answer.left()), (0, 0));

    // The same post as tall as the limit is judged, and its four sides are its own.
    let mut town = Town::default();
    town.floor(0.0);
    let mut sides = Vec::new();
    for (from, to) in [
        ([-half, -half], [half, -half]),
        ([half, -half], [half, half]),
        ([half, half], [-half, half]),
        ([-half, half], [-half, -half]),
    ] {
        sides.extend(town.wall(post, from, to, 0.0, JUDGED_FROM));
    }
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &stands(JUDGED_FROM), &AS_THE_RULE_HAS_IT);
    assert_eq!(proposed(leaf(&answer)), sides);

    // A box of no numbers is no box, and is said to be none.
    let broken = OrientedBox {
        centre: Vec3::splat(f32::NAN),
        ..stands(JUDGED_FROM)
    };
    let answer = associate(&set, &broken, &AS_THE_RULE_HAS_IT);
    assert_eq!(answer.not_judged, Some(NotJudged::NoBox));
    assert!(answer.bounds.is_empty());
}

#[test]
fn a_building_turned_forty_five_degrees_gives_the_polygons_the_unturned_one_gives() {
    let turned = Affine3A::from_rotation_translation(
        Quat::from_rotation_z(45f32.to_radians()),
        Vec3::new(-7.0, 3.0, 0.0),
    );
    let mut answers = Vec::new();
    for place in [Affine3A::IDENTITY, turned] {
        let mut town = Town::default();
        town.floor(0.0);
        let own = town.building(place);
        let set = set_of(town.bound(), Affine3A::IDENTITY);
        let answer = associate(&set, &object(place), &AS_THE_RULE_HAS_IT);
        let leaf = leaf(&answer);
        assert_eq!(proposed(leaf), own);
        assert!(
            leaf.left.iter().all(|left| left.reason == Reason::Ground),
            "all that is left is floor"
        );
        answers.push(proposed(leaf));
    }
    assert_eq!(answers[0], answers[1]);
}

#[test]
fn a_bound_placed_by_a_turn_and_a_move_answers_as_the_bound_where_it_was_built() {
    let town = || {
        let mut town = Town::default();
        town.floor(0.0);
        town.building(Affine3A::IDENTITY);
        town.bound()
    };
    let place = Affine3A::from_rotation_translation(
        Quat::from_rotation_z(0.7) * Quat::from_rotation_x(0.01),
        Vec3::new(1200.0, -340.0, 31.0),
    );
    let at_home = associate(
        &set_of(town(), Affine3A::IDENTITY),
        &object(Affine3A::IDENTITY),
        &AS_THE_RULE_HAS_IT,
    );
    let placed = associate(&set_of(town(), place), &object(place), &AS_THE_RULE_HAS_IT);
    assert_eq!(proposed(leaf(&placed)), proposed(leaf(&at_home)));
    assert_eq!(
        left_as_ground(leaf(&placed)),
        left_as_ground(leaf(&at_home))
    );
}

#[test]
fn a_wall_stored_a_little_outside_the_face_of_the_box_is_still_the_objects() {
    let mut town = Town::default();
    town.floor(0.0);
    // Two centimetres past the face its archetype's box ends on: less than the slack, and more than a quantum of this bound.
    let out = 0.02;
    assert!(out < BOX_SLACK);
    let wall = town.wall(
        Affine3A::IDENTITY,
        [BB_MAX.x + out, BB_MIN.y],
        [BB_MAX.x + out, BB_MAX.y],
        BB_MIN.z,
        BB_MAX.z,
    );
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    assert_eq!(proposed(leaf), wall);
    assert!(
        leaf.proposed.iter().all(|p| p.share.is_none()),
        "held whole"
    );
}

#[test]
fn a_shape_wholly_in_the_box_is_proposed_and_one_the_box_cuts_is_left() {
    let mut town = Town::default();
    town.floor(0.0);
    let own = town.building(Affine3A::IDENTITY);
    let triangles = town.triangles.len() as u32;
    let inside = town.vertex(Vec3::new(5.0, 4.0, 1.0));
    let ends = [
        town.vertex(Vec3::new(9.0, 4.0, 1.0)),
        town.vertex(Vec3::new(12.0, 4.0, 1.0)),
    ];
    town.primitives = vec![
        bounds::sphere_primitive(inside, 0.5),
        bounds::capsule_primitive(ends, 0.25),
    ];
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    let mut both = own;
    both.push(triangles);
    assert_eq!(
        proposed(leaf),
        both,
        "the sphere with the walls and the roof"
    );
    let capsule = leaf
        .left
        .iter()
        .find(|left| left.polygon == Some(triangles + 1))
        .expect("the capsule is listed");
    assert!(capsule.cut);
    assert_eq!(capsule.reason, Reason::CutShape);
}

#[test]
fn the_ground_is_proposed_too_once_it_is_asked_for() {
    let mut town = Town::default();
    town.floor(0.0);
    let own = town.building(Affine3A::IDENTITY);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(
        &set,
        &object(Affine3A::IDENTITY),
        &AssociationOptions { take_ground: true },
    );
    let leaf = leaf(&answer);
    let (held, cut) = floor_under(
        [BB_MIN.x - BOX_SLACK, BB_MIN.y - BOX_SLACK],
        [BB_MAX.x + BOX_SLACK, BB_MAX.y + BOX_SLACK],
    );
    let mut all: Vec<u32> = held.iter().chain(&own).copied().collect();
    all.sort_unstable();
    assert_eq!(
        proposed(leaf),
        all,
        "the floor the box holds whole goes too"
    );
    // A cell the box cuts is judged by its share like any triangle, and none here is most in the box.
    for polygon in cut {
        let left = leaf
            .left
            .iter()
            .find(|left| left.polygon == Some(polygon))
            .unwrap_or_else(|| panic!("floor polygon {polygon} is listed"));
        assert!(
            matches!(left.reason, Reason::MostlyOutside { share } if share < MOST),
            "{:?}",
            left.reason
        );
    }
    assert!(left_as_ground(leaf).is_empty());
}

#[test]
fn the_ground_uphill_of_a_building_on_a_slope_is_left_with_the_ground_downhill() {
    // Fifteen degrees: the ground is at the foot of the walls on the low side of the box and more than two metres up them on the
    // high side, where the walls go on under it.
    let rise = 15f32.to_radians().tan();
    let mut town = Town::default();
    town.slope(0.0, rise);
    let own = town.building(Affine3A::IDENTITY);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    assert!(rise * BB_MAX.x > BB_MIN.z + 8.0 * GROUND_STEP && rise * BB_MAX.x < BB_MAX.z);
    assert_eq!(
        proposed(leaf),
        own,
        "the four walls and the roof, and no ground"
    );
    let (held, cut) = floor_under(
        [BB_MIN.x - BOX_SLACK, BB_MIN.y - BOX_SLACK],
        [BB_MAX.x + BOX_SLACK, BB_MAX.y + BOX_SLACK],
    );
    assert!(!held.is_empty() && !cut.is_empty());
    let mut under: Vec<u32> = held.iter().chain(&cut).copied().collect();
    under.sort_unstable();
    assert_eq!(
        left_as_ground(leaf),
        under,
        "the slope under the box, high and low"
    );
    assert_eq!(leaf.left.len(), under.len());
}

#[test]
fn a_level_surface_that_goes_on_past_the_box_is_left_and_one_that_ends_in_it_goes() {
    let mut town = Town::default();
    town.floor(0.0);
    let own = town.building(Affine3A::IDENTITY);
    // A deck two metres up that crosses the building from far on one side to far on the other, in cells two metres long.
    let (span, width, up) = (2.0, [3.0, 5.0], 2.0);
    let mut deck = Vec::new();
    let mut inside = Vec::new();
    for k in -10..15 {
        let x = k as f32 * span;
        // Every other cell is stored four millimetres aside, as a second bound would store it: its corners are still its neighbours'.
        let aside = if k % 2 == 0 { 0.0 } else { 0.004 };
        assert!(aside < SHARED_CORNER);
        let cell = town.quad([
            Vec3::new(x, width[0] + aside, up),
            Vec3::new(x + span, width[0] + aside, up),
            Vec3::new(x + span, width[1] + aside, up),
            Vec3::new(x, width[1] + aside, up),
        ]);
        if x >= BB_MIN.x && x + span <= BB_MAX.x {
            inside.extend(cell);
        }
        deck.extend(cell);
    }
    assert_eq!(
        inside.len(),
        10,
        "five cells of the deck are wholly in the box"
    );
    // A landing at the same height that ends inside the building, and shares no edge with the deck.
    let landing = town.quad([
        Vec3::new(1.0, 6.0, up),
        Vec3::new(3.0, 6.0, up),
        Vec3::new(3.0, 7.0, up),
        Vec3::new(1.0, 7.0, up),
    ]);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    let mut both = own.clone();
    both.extend(landing);
    assert_eq!(proposed(leaf), both, "the building and its landing");
    let ground = left_as_ground(leaf);
    assert!(
        inside.iter().all(|p| ground.contains(p)),
        "the deck is not the building's, in the box or past it"
    );
    assert!(
        leaf.left
            .iter()
            .filter(|left| left.polygon.is_some_and(|p| deck.contains(&p)))
            .all(|left| left.reason == Reason::Ground),
        "and the cells the box cuts are left for the same reason"
    );

    // Asked for, the ground goes by its share as it did: the deck in the box with it.
    let answer = associate(
        &set,
        &object(Affine3A::IDENTITY),
        &AssociationOptions { take_ground: true },
    );
    let taken = proposed(self::leaf(&answer));
    assert!(inside.iter().all(|p| taken.contains(p)));
}

#[test]
fn a_roof_is_the_objects_at_a_shared_corner_and_ground_along_a_shared_edge() {
    let mut town = Town::default();
    town.floor(0.0);
    let own = town.building(Affine3A::IDENTITY);
    // The roof next door, as high, which touches the building's roof at the one corner the two have in common.
    let corner = BB_MAX;
    town.quad([
        corner,
        corner + Vec3::new(5.0, 0.0, 0.0),
        corner + Vec3::new(5.0, 5.0, 0.0),
        corner + Vec3::new(0.0, 5.0, 0.0),
    ]);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    assert_eq!(
        proposed(leaf(&answer)),
        own,
        "an edge is shared, or nothing is"
    );

    // ⚠ What the rule does of a terrace: a roof next door that is as high and shares the whole edge makes one surface of the two,
    // which goes on past the box, so the building's roof is left as ground with it.
    let mut town = Town::default();
    town.floor(0.0);
    let own = town.building(Affine3A::IDENTITY);
    town.quad([
        Vec3::new(BB_MAX.x, BB_MIN.y, BB_MAX.z),
        Vec3::new(BB_MAX.x + 5.0, BB_MIN.y, BB_MAX.z),
        Vec3::new(BB_MAX.x + 5.0, BB_MAX.y, BB_MAX.z),
        BB_MAX,
    ]);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    assert_eq!(proposed(leaf), own[..8], "the four walls");
    assert!(own[8..].iter().all(|p| left_as_ground(leaf).contains(p)));
}

#[test]
fn a_roof_that_overhangs_the_box_by_a_sliver_is_still_the_objects() {
    let mut town = Town::default();
    town.floor(0.0);
    let (lo, hi) = (BB_MIN, BB_MAX);
    let mut own = Vec::new();
    for (from, to) in [
        ([lo.x, lo.y], [hi.x, lo.y]),
        ([hi.x, lo.y], [hi.x, hi.y]),
        ([hi.x, hi.y], [lo.x, hi.y]),
        ([lo.x, hi.y], [lo.x, lo.y]),
    ] {
        own.extend(town.wall(Affine3A::IDENTITY, from, to, lo.z, hi.z));
    }
    // The roof ends thirty centimetres past the face of the box: the box cuts both of its triangles and holds most of each.
    let eave = hi.x + 0.3;
    let roof = town.quad([
        Vec3::new(lo.x, lo.y, hi.z),
        Vec3::new(eave, lo.y, hi.z),
        Vec3::new(eave, hi.y, hi.z),
        Vec3::new(lo.x, hi.y, hi.z),
    ]);
    own.extend(roof);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(Affine3A::IDENTITY), &AS_THE_RULE_HAS_IT);
    let leaf = leaf(&answer);
    assert_eq!(
        proposed(leaf),
        own,
        "a surface most of which the box holds does not go on past it"
    );
    for polygon in roof {
        let share = (leaf.proposed.iter())
            .find(|p| p.polygon == Some(polygon))
            .and_then(|p| p.share)
            .expect("a cut triangle states its share");
        assert!((MOST..1.0).contains(&share), "{share}");
    }
}

#[test]
fn a_bound_placed_at_another_size_answers_as_the_bound_where_it_was_built() {
    let town = || {
        let mut town = Town::default();
        town.floor(0.0);
        town.building(Affine3A::IDENTITY);
        town.bound()
    };
    let at_home = associate(
        &set_of(town(), Affine3A::IDENTITY),
        &object(Affine3A::IDENTITY),
        &AS_THE_RULE_HAS_IT,
    );
    // Twice the size and three quarters of it: the box in the bound's own space is smaller than in the world, and then larger. The
    // quarter turn puts the long side of the box along the bound's other axis.
    for (size, turn) in [
        (2.0, Quat::from_rotation_z(-1.1)),
        (0.75, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
    ] {
        let at = Vec3::new(-300.0, 75.0, 12.0);
        let place = Affine3A::from_scale_rotation_translation(Vec3::splat(size), turn, at);
        let sized = OrientedBox::placed(BB_MIN, BB_MAX, at, turn, Vec3::splat(size));
        let placed = associate(&set_of(town(), place), &sized, &AS_THE_RULE_HAS_IT);
        assert!((placed.height - size * at_home.height).abs() < 1e-4);
        assert_eq!(proposed(leaf(&placed)), proposed(leaf(&at_home)), "{size}");
        assert_eq!(
            left_as_ground(leaf(&placed)),
            left_as_ground(leaf(&at_home)),
            "{size}"
        );
    }
}

#[test]
fn a_box_turned_onto_its_side_is_judged_by_how_tall_it_stands_in_the_world() {
    let mut town = Town::default();
    town.floor(0.0);
    // A quarter turn about x lays the building on the wall it had along y = 0: it stands eight metres tall and four deep.
    let on_its_side = Affine3A::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let own = town.building(on_its_side);
    let set = set_of(town.bound(), Affine3A::IDENTITY);
    let answer = associate(&set, &object(on_its_side), &AS_THE_RULE_HAS_IT);
    assert!((answer.height - BB_MAX.y).abs() < 1e-4, "{}", answer.height);
    let leaf = leaf(&answer);
    // The wall it lies on is level and at the foot of the others, which is ground; the wall that was opposite is its roof now.
    assert_eq!(proposed(leaf), own[2..]);
    assert!(own[..2].iter().all(|p| left_as_ground(leaf).contains(p)));

    // A slab a metre thick is too low lying down and tall enough on its edge, whatever its own up is.
    let slab = |turn: Quat| {
        OrientedBox::placed(
            Vec3::ZERO,
            Vec3::new(10.0, 8.0, 1.0),
            Vec3::new(0.0, 0.0, 0.0),
            turn,
            Vec3::ONE,
        )
    };
    let lying = associate(&set, &slab(Quat::IDENTITY), &AS_THE_RULE_HAS_IT);
    assert!(matches!(lying.not_judged, Some(NotJudged::TooLow { .. })));
    let on_edge = associate(
        &set,
        &slab(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)),
        &AS_THE_RULE_HAS_IT,
    );
    assert_eq!(on_edge.not_judged, None);
    assert!((on_edge.height - 10.0).abs() < 1e-4, "{}", on_edge.height);
}
