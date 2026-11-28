//! Bounds placed in a set and asked in world space: the ground under a point, what a box holds
//! of a bound, the keys and the placing transforms, and the bounds that are refused.

use std::sync::Arc;

use glam::{Affine3A, DVec3, Mat3, Quat, Vec3};
use or_io::gta5::{BoundType, PhBoundAny, PhBoundGeometry, PhBoundPlane};
use or_world::collision::{
    BoundCollider, BoxOverlap, CollisionError, CollisionSet, GroundProbe, OrientedBox, Part,
};
use or_world_gta5::collision::OfBound;

#[path = "support/bounds.rs"]
mod bounds;
use bounds::{stored, Dice};

fn collider(bound: &PhBoundAny) -> Arc<BoundCollider> {
    Arc::new(BoundCollider::new(bound).expect("a bound built by hand is whole"))
}

fn set_of(bounds: Vec<(&'static str, PhBoundAny, Affine3A)>) -> CollisionSet<&'static str> {
    let mut set = CollisionSet::new();
    for (key, bound, place) in bounds {
        set.add(key, collider(&bound), place).expect("a placement");
    }
    set
}

const FLOOR: u64 = 0x10;
const ROOF: u64 = 0x20;
const WALL: u64 = 0x30;
const STEEP: u64 = 0x40;
/// The steep triangle's three vertices, by their place in the building's list.
const STEEP_VERTICES: [usize; 3] = [11, 12, 13];

/// A floor at 0 and a roof at 3 over ten metres square, an upright wall in the plane x = 5, and
/// a triangle some 84 degrees from level that stands over the point 2, 2.
fn building() -> PhBoundGeometry {
    let positions = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(10.0, 10.0, 0.0),
        Vec3::new(0.0, 10.0, 0.0),
        Vec3::new(0.0, 0.0, 3.0),
        Vec3::new(10.0, 0.0, 3.0),
        Vec3::new(10.0, 10.0, 3.0),
        Vec3::new(0.0, 10.0, 3.0),
        Vec3::new(5.0, 4.0, 0.0),
        Vec3::new(5.0, 6.0, 0.0),
        Vec3::new(5.0, 5.0, 3.0),
        Vec3::new(1.9, 1.0, 0.5),
        Vec3::new(2.1, 1.0, 2.5),
        Vec3::new(2.0, 3.0, 1.5),
    ];
    // ⚠ The roof is wound to face down: which way a triangle is wound is not which way is up.
    let triangles = [
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [8, 9, 10],
        [11, 12, 13],
    ];
    bounds::geometry(
        &positions,
        &triangles,
        Vec::new(),
        &[FLOOR, ROOF, WALL, STEEP],
        &[0, 0, 1, 1, 2, 3],
    )
}

/// Holds two places to one, to a hundredth of a millimetre a metre from the origin.
#[track_caller]
fn assert_at(got: Vec3, want: Vec3) {
    let scale = want.abs().max_element().max(1.0);
    assert!((got - want).length() < 1e-5 * scale, "at {got} for {want}");
}

fn walker(max_rise: f32, max_slope_degrees: f32) -> GroundProbe {
    GroundProbe {
        max_rise,
        max_drop: 10.0,
        max_slope: max_slope_degrees.to_radians(),
    }
}

/// The height of the steep triangle's plane over a point, from the vertices the bound holds.
fn steep_height(g: &PhBoundGeometry, x: f64, y: f64) -> (f64, DVec3) {
    let [a, b, c] = STEEP_VERTICES.map(|v| stored(g, v));
    let n = (b - a).cross(c - a);
    let z = a.z - (n.x * (x - a.x) + n.y * (y - a.y)) / n.z;
    let up = if n.z > 0.0 { n } else { -n };
    (z, up.normalize())
}

#[test]
fn the_ground_between_a_floor_and_a_roof_is_the_floor() {
    let g = building();
    let set = set_of(vec![(
        "building",
        PhBoundAny::PhBoundGeometry(g),
        Affine3A::IDENTITY,
    )]);
    let between = Vec3::new(7.0, 7.0, 1.5);

    let (key, floor) = set
        .ground(between, &walker(1.0, 45.0))
        .expect("a floor below");
    assert_eq!(*key, "building");
    assert_at(floor.position, Vec3::new(7.0, 7.0, 0.0));
    assert!(
        (floor.distance - (1.0 + 1.5)).abs() < 1e-5,
        "from the top of the probe"
    );
    assert_at(floor.normal, Vec3::Z);
    assert_eq!(floor.material, Some(FLOOR));

    // Allowed to rise past the roof, the highest surface below that height is the roof, and
    // its normal faces up though the triangle is wound to face down.
    let (_, roof) = set.ground(between, &walker(2.0, 45.0)).expect("the roof");
    assert_at(roof.position, Vec3::new(7.0, 7.0, 3.0));
    assert_at(roof.normal, Vec3::Z);
    assert_eq!(roof.material, Some(ROOF));
    // A step short of it, the roof is out of reach again.
    let (_, under) = set
        .ground(Vec3::new(7.0, 7.0, 2.9), &walker(0.05, 45.0))
        .expect("the floor");
    assert_eq!(under.material, Some(FLOOR));
    // And from on top of the roof with no rise at all, the roof.
    let (_, on) = set
        .ground(Vec3::new(7.0, 7.0, 3.5), &walker(0.0, 45.0))
        .expect("the roof");
    assert_eq!(on.material, Some(ROOF));
}

#[test]
fn the_ground_is_never_a_wall() {
    let g = building();
    let set = set_of(vec![(
        "building",
        PhBoundAny::PhBoundGeometry(g.clone()),
        Affine3A::IDENTITY,
    )]);

    // The upright wall is there: a level ray meets it.
    let (_, wall) = set
        .ray_cast(Vec3::new(0.0, 5.0, 1.0), Vec3::X, 100.0)
        .expect("the wall");
    assert_eq!(wall.material, Some(WALL));
    // Straight down its plane it is seen edge on, and the ground under it is the floor.
    let (_, floor) = set
        .ground(Vec3::new(5.0, 5.0, 1.5), &walker(1.0, 45.0))
        .expect("the floor");
    assert_eq!(floor.material, Some(FLOOR));
    assert_at(floor.position, Vec3::new(5.0, 5.0, 0.0));

    // The steep triangle is in the way of the probe: a ray down meets it first.
    let over = Vec3::new(2.0, 2.0, 2.0);
    let (height, up) = steep_height(&g, 2.0, 2.0);
    assert!(
        height > 1.0 && height < 2.0,
        "it stands between the point and the floor: {height}"
    );
    let slope = up.z.acos().to_degrees();
    assert!(
        slope > 80.0 && slope < 89.0,
        "it is steep and not upright: {slope}"
    );
    let (_, first) = set
        .ray_cast(over + Vec3::Z * 0.4, Vec3::NEG_Z, 100.0)
        .expect("the steep triangle");
    assert_eq!(first.material, Some(STEEP));

    // A walker's ground passes it for the floor beneath.
    let (_, floor) = set.ground(over, &walker(0.4, 45.0)).expect("the floor");
    assert_eq!(floor.material, Some(FLOOR));
    assert_at(floor.position, Vec3::new(2.0, 2.0, 0.0));
    // Only a probe that takes a slope steeper than it stands on it.
    let (_, steep) = set
        .ground(over, &walker(0.4, slope as f32 + 1.0))
        .expect("the steep triangle");
    assert_eq!(steep.material, Some(STEEP));
    assert!(
        (f64::from(steep.position.z) - height).abs() < 1e-4,
        "{}",
        steep.position.z
    );
    assert!((steep.normal.as_dvec3() - up).length() < 1e-4);
    let (_, floor) = set
        .ground(over, &walker(0.4, slope as f32 - 1.0))
        .expect("the floor");
    assert_eq!(floor.material, Some(FLOOR));
}

/// ⚠ A point on the seam of two triangles is in both to the last bit or in neither, and a
/// vertical probe on a seam runs along the faces of the tree's boxes: both are where a floor
/// lets something through.
#[test]
fn the_ground_is_found_on_every_seam_and_corner_of_a_tiled_floor() {
    const CELLS: u16 = 16;
    let across = CELLS + 1;
    let positions: Vec<Vec3> = (0..across)
        .flat_map(|j| (0..across).map(move |i| Vec3::new(i.into(), j.into(), 0.0)))
        .collect();
    let mut triangles = Vec::new();
    for j in 0..CELLS {
        for i in 0..CELLS {
            let at = j * across + i;
            triangles.push([at, at + 1, at + across + 1]);
            triangles.push([at, at + across + 1, at + across]);
        }
    }
    let g = bounds::triangles(&positions, &triangles);
    // The vertices where sixteen bits put them, the middles of the edges between them, and the
    // middles of the diagonals.
    let held: Vec<Vec3> = (0..positions.len())
        .map(|v| stored(&g, v).as_vec3())
        .collect();
    let at = |i: u16, j: u16| held[usize::from(j * across + i)];
    let mut probes = held.clone();
    for j in 0..CELLS {
        for i in 0..CELLS {
            probes.push((at(i, j) + at(i + 1, j)) * 0.5);
            probes.push((at(i, j) + at(i, j + 1)) * 0.5);
            probes.push((at(i, j) + at(i + 1, j + 1)) * 0.5);
        }
    }
    let far = Affine3A::from_translation(Vec3::new(4096.0, -2048.0, 32.0));
    let set = set_of(vec![
        (
            "here",
            PhBoundAny::PhBoundGeometry(g.clone()),
            Affine3A::IDENTITY,
        ),
        ("far", PhBoundAny::PhBoundGeometry(g), far),
    ]);
    for probe in &probes {
        for (key, place) in [("here", Affine3A::IDENTITY), ("far", far)] {
            let over = place.transform_point3(*probe + Vec3::Z * 1.5);
            let (found, hit) = set
                .ground(over, &walker(0.5, 45.0))
                .unwrap_or_else(|| panic!("nothing under {over}, which is over the floor"));
            assert_eq!(*found, key);
            assert_at(hit.position, place.transform_point3(*probe));
            assert_at(hit.normal, Vec3::Z);
        }
    }
    assert_eq!(probes.len(), 17 * 17 + 3 * 16 * 16);
}

/// Jittered heights over a jittered grid: no seam runs along an axis, and no face is level.
fn rough_terrain(cells: u16, dice: &mut Dice) -> PhBoundGeometry {
    let across = cells + 1;
    let positions: Vec<Vec3> = (0..across)
        .flat_map(|j| (0..across).map(move |i| (i, j)))
        .map(|(i, j)| {
            Vec3::new(
                f32::from(i) * 1.37 + dice.between(-0.3, 0.3),
                f32::from(j) * 1.37 + dice.between(-0.3, 0.3),
                dice.between(0.0, 0.8),
            )
        })
        .collect();
    let mut triangles = Vec::new();
    for j in 0..cells {
        for i in 0..cells {
            let at = j * across + i;
            triangles.push([at, at + 1, at + across + 1]);
            triangles.push([at, at + across + 1, at + across]);
        }
    }
    bounds::triangles(&positions, &triangles)
}

/// ⚠ On a slanted seam the two triangles work out which side a point is on from different
/// corners, and each may round it out of itself: a triangle held to its edge to the last bit
/// lets some of these through.
#[test]
fn the_ground_is_found_along_every_seam_of_a_rough_terrain() {
    const CELLS: u16 = 24;
    let across = CELLS + 1;
    let mut dice = Dice(0x7E44);
    let g = rough_terrain(CELLS, &mut dice);
    let held: Vec<Vec3> = (0..g.compressed_vertices.len())
        .map(|v| stored(&g, v).as_vec3())
        .collect();
    let at = |i: u16, j: u16| held[usize::from(j * across + i)];
    let set = set_of(vec![(
        "terrain",
        PhBoundAny::PhBoundGeometry(g),
        Affine3A::IDENTITY,
    )]);
    let mut probes = 0;
    for j in 1..CELLS - 1 {
        for i in 1..CELLS - 1 {
            for (from, to) in [
                (at(i, j), at(i + 1, j)),
                (at(i, j), at(i, j + 1)),
                (at(i, j), at(i + 1, j + 1)),
            ] {
                for _ in 0..8 {
                    let on = from.lerp(to, dice.roll());
                    probes += 1;
                    let (_, hit) = set
                        .ground(on + Vec3::Z * 2.0, &walker(0.5, 89.0))
                        .unwrap_or_else(|| panic!("nothing under {on}, which is on a seam"));
                    assert!(
                        (hit.position.z - on.z).abs() < 1e-3,
                        "under {on} the ground is at {}",
                        hit.position.z
                    );
                }
            }
        }
    }
    assert!(probes > 10_000, "{probes} probes");
}

/// ⚠ What the ground passes over must not shorten the search. Each sheet here is too steep to
/// stand on and as long as the floor, so in the tree it comes before the floor under some
/// points and after it under others.
#[test]
fn what_the_ground_passes_over_does_not_hide_the_floor_beneath() {
    const CELLS: u16 = 12;
    let across = CELLS + 1;
    let lift = 80f32.to_radians().tan();
    let mut positions: Vec<Vec3> = (0..across)
        .flat_map(|j| (0..across).map(move |i| Vec3::new(i.into(), j.into(), 0.0)))
        .collect();
    let mut triangles = Vec::new();
    for j in 0..CELLS {
        for i in 0..CELLS {
            let at = j * across + i;
            triangles.push([at, at + 1, at + across + 1]);
            triangles.push([at, at + across + 1, at + across]);
        }
    }
    let floors = triangles.len();
    let wide = f32::from(CELLS);
    for j in 0..CELLS {
        let y = f32::from(j);
        let at = positions.len() as u16;
        positions.extend([
            Vec3::new(0.0, y, 2.0),
            Vec3::new(wide, y, 2.0),
            Vec3::new(wide, y + 1.0, 2.0 + lift),
            Vec3::new(0.0, y + 1.0, 2.0 + lift),
        ]);
        triangles.push([at, at + 1, at + 2]);
        triangles.push([at, at + 2, at + 3]);
    }
    let materials: Vec<u8> = (0..triangles.len())
        .map(|t| u8::from(t >= floors))
        .collect();
    let g = bounds::geometry(
        &positions,
        &triangles,
        Vec::new(),
        &[FLOOR, STEEP],
        &materials,
    );
    let level = stored(&g, 0).z as f32;
    let set = set_of(vec![(
        "hall",
        PhBoundAny::PhBoundGeometry(g),
        Affine3A::IDENTITY,
    )]);
    let top = 2.0 + lift + 1.0;
    for j in 0..CELLS {
        for i in 0..CELLS {
            let over = Vec3::new(f32::from(i) + 0.4, f32::from(j) + 0.6, top);
            let (_, first) = set.ray_cast(over, Vec3::NEG_Z, 100.0).expect("the sheet");
            assert_eq!(first.material, Some(STEEP), "over {over}");
            let (_, ground) = set
                .ground(over, &walker(0.0, 45.0))
                .unwrap_or_else(|| panic!("no ground under {over}"));
            assert_eq!(ground.material, Some(FLOOR), "under {over}");
            assert!(
                (ground.position.z - level).abs() < 1e-3,
                "the floor under {over} is at {}",
                ground.position.z
            );
        }
    }
}

/// ⚠ A bound's own up is not the world's: a slope is judged after the bound is placed.
#[test]
fn a_slope_is_judged_as_the_bound_is_placed() {
    let level = bounds::triangles(
        &[
            Vec3::new(-4.0, -4.0, 0.0),
            Vec3::new(4.0, -4.0, 0.0),
            Vec3::new(0.0, 4.0, 0.0),
        ],
        &[[0, 1, 2]],
    );
    let upright = bounds::triangles(
        &[
            Vec3::new(-4.0, 0.0, -4.0),
            Vec3::new(4.0, 0.0, -4.0),
            Vec3::new(0.0, 0.0, 4.0),
        ],
        &[[0, 1, 2]],
    );
    let tip = 60f32;
    let tipped = Affine3A::from_rotation_x(tip.to_radians());
    let laid = Affine3A::from_rotation_x(90f32.to_radians());
    let over = Vec3::new(0.0, 0.0, 5.0);
    let ground = |g: &PhBoundGeometry, place: Affine3A, slope: f32| {
        let set = set_of(vec![("it", PhBoundAny::PhBoundGeometry(g.clone()), place)]);
        set.ground(over, &walker(0.0, slope)).map(|(_, hit)| hit)
    };

    // Level in its own space and tipped by its placing: ground only to what stands that steep.
    assert!(ground(&level, Affine3A::IDENTITY, 45.0).is_some());
    assert_eq!(ground(&level, tipped, tip - 1.0), None);
    let steep = ground(&level, tipped, tip + 1.0).expect("a walker of steep slopes");
    assert!(
        (steep.normal.z - tip.to_radians().cos()).abs() < 1e-5,
        "its normal in the world: {}",
        steep.normal
    );
    assert!(steep.position.length() < 1e-4, "it turns about the origin");

    // Upright in its own space and laid flat by its placing: ground to anyone.
    assert_eq!(ground(&upright, Affine3A::IDENTITY, 45.0), None);
    let flat = ground(&upright, laid, 1.0).expect("it lies level");
    assert_at(flat.normal, Vec3::Z);
}

#[test]
fn the_ground_stops_where_the_probe_does() {
    let set = set_of(vec![
        (
            "building",
            PhBoundAny::PhBoundGeometry(building()),
            Affine3A::IDENTITY,
        ),
        (
            "crate",
            bounds::boxed(Vec3::new(20.0, 4.0, 0.0), Vec3::new(22.0, 6.0, 1.0), 0x50),
            Affine3A::IDENTITY,
        ),
        (
            "cellar",
            bounds::boxed(Vec3::new(20.0, 4.0, -3.0), Vec3::new(22.0, 6.0, -2.0), 0x60),
            Affine3A::IDENTITY,
        ),
    ]);
    let between = Vec3::new(7.0, 7.0, 1.5);
    let reach = |max_drop: f32| GroundProbe {
        max_rise: 0.0,
        max_drop,
        max_slope: 45f32.to_radians(),
    };
    assert_eq!(
        set.ground(between, &reach(1.0)),
        None,
        "the floor is 1.5 below"
    );
    assert!(set.ground(between, &reach(1.5)).is_some());
    // The drop is counted from the point, whatever the rise above it.
    let lifted = |max_drop: f32| GroundProbe {
        max_rise: 1.0,
        ..reach(max_drop)
    };
    assert_eq!(set.ground(between, &lifted(1.4)), None);
    assert!(set.ground(between, &lifted(1.5)).is_some());
    assert_eq!(
        set.ground(Vec3::new(50.0, 50.0, 1.0), &reach(100.0)),
        None,
        "nothing is there"
    );

    // Standing on the crate, its top; standing in it, what lies beneath it.
    let (key, top) = set
        .ground(Vec3::new(21.0, 5.0, 1.0), &walker(0.5, 45.0))
        .expect("its top");
    assert_eq!(
        (*key, top.position, top.normal),
        ("crate", Vec3::new(21.0, 5.0, 1.0), Vec3::Z)
    );
    let (key, below) = set
        .ground(Vec3::new(21.0, 5.0, 0.5), &walker(0.2, 45.0))
        .expect("the cellar");
    assert_eq!(
        (*key, below.position.z, below.material),
        ("cellar", -2.0, Some(0x60))
    );

    for broken in [
        GroundProbe {
            max_rise: -1.0,
            ..reach(10.0)
        },
        GroundProbe {
            max_rise: f32::NAN,
            ..reach(10.0)
        },
        reach(-1.0),
        reach(f32::NAN),
        reach(f32::INFINITY),
    ] {
        assert_eq!(set.ground(between, &broken), None, "{broken:?}");
    }
    assert_eq!(
        set.ground(Vec3::new(f32::NAN, 7.0, 1.5), &reach(10.0)),
        None
    );
}

#[test]
fn a_placed_bound_answers_where_its_transform_puts_it_and_at_its_scale() {
    let g = building();
    // Twice the size, a quarter turn about Z, and moved.
    let place = Affine3A::from_scale_rotation_translation(
        Vec3::splat(2.0),
        Quat::from_rotation_z(90f32.to_radians()),
        Vec3::new(100.0, 0.0, 50.0),
    );
    let set = set_of(vec![(
        "building",
        PhBoundAny::PhBoundGeometry(g.clone()),
        place,
    )]);
    let world = |local: Vec3| place.transform_point3(local);

    // Between the floor and the roof, which are now 6 apart: 3 above the floor.
    let between = world(Vec3::new(7.0, 7.0, 1.5));
    let (_, floor) = set.ground(between, &walker(2.0, 45.0)).expect("the floor");
    let want = world(Vec3::new(7.0, 7.0, 0.0));
    assert!(
        (floor.position - want).length() < 1e-4,
        "{} for {want}",
        floor.position
    );
    assert!(
        (floor.distance - (2.0 + 3.0)).abs() < 1e-4,
        "metres of the world: {}",
        floor.distance
    );
    assert!((floor.normal - Vec3::Z).length() < 1e-6);
    assert_eq!(floor.material, Some(FLOOR));

    // A level ray at the wall, which the turn has put across Y.
    let from = world(Vec3::new(0.0, 5.0, 1.0));
    let along = place.transform_vector3(Vec3::X).normalize();
    let (_, wall) = set.ray_cast(from, along, 100.0).expect("the wall");
    assert!(
        (wall.distance - 2.0 * 5.0).abs() < 1e-3,
        "{}",
        wall.distance
    );
    assert!(
        (wall.normal + along).length() < 1e-4,
        "it faces the ray: {}",
        wall.normal
    );
    assert!((wall.position - world(Vec3::new(5.0, 5.0, 1.0))).length() < 1e-3);

    // A sphere of 0.5 dropped on the floor rests 0.5 above it in the world's metres.
    let (_, rest) = set
        .sweep_sphere(between, Vec3::NEG_Z, 100.0, 0.5)
        .expect("the floor");
    assert!(
        (rest.distance - (3.0 - 0.5)).abs() < 1e-4,
        "{}",
        rest.distance
    );
    assert!((rest.position - want).length() < 1e-4);

    // How far a query may go is in the world's metres too: the floor is 3 below.
    assert_eq!(set.ray_cast(between, Vec3::NEG_Z, 2.9), None);
    assert!(set.ray_cast(between, Vec3::NEG_Z, 3.1).is_some());
    assert_eq!(set.sweep_sphere(between, Vec3::NEG_Z, 2.4, 0.5), None);
    assert!(set.sweep_sphere(between, Vec3::NEG_Z, 2.6, 0.5).is_some());

    // The box of the placed bound is the box of its turned and scaled corners.
    let boxed = set.world_box(&"building").expect("it is in the set");
    let corners = [Vec3::ZERO, Vec3::new(10.0, 10.0, 3.0)].map(world);
    let (low, high) = (corners[0].min(corners[1]), corners[0].max(corners[1]));
    assert!(
        (boxed.min - low).length() < 1e-3 && (boxed.max - high).length() < 1e-3,
        "{boxed:?}"
    );
}

#[test]
fn a_set_answers_with_the_key_of_the_nearest_bound_and_forgets_what_is_removed() {
    let ball = collider(&bounds::sphere(Vec3::ZERO, 1.0, 9));
    let at = |x: f32| Affine3A::from_translation(Vec3::new(x, 0.0, 0.0));
    let mut set: CollisionSet<u32> = CollisionSet::new();
    assert!(set.is_empty());
    assert_eq!(
        set.ray_cast(Vec3::new(-10.0, 0.0, 0.0), Vec3::X, 100.0),
        None
    );

    // One collider, placed three times: the tree is built once and shared.
    for (key, x) in [(30, 30.0), (10, 10.0), (20, 20.0)] {
        set.add(key, ball.clone(), at(x)).expect("a placement");
    }
    assert_eq!(Arc::strong_count(&ball), 4);
    assert_eq!(set.len(), 3);
    let boxes: Vec<(u32, Vec3, Vec3)> = set
        .world_boxes()
        .map(|(key, b)| (*key, b.min, b.max))
        .collect();
    let around = |x: f32| (Vec3::new(x - 1.0, -1.0, -1.0), Vec3::new(x + 1.0, 1.0, 1.0));
    assert_eq!(
        boxes,
        [10.0f32, 20.0, 30.0].map(|x| (x as u32, around(x).0, around(x).1)),
        "in key order, each about where it was placed"
    );

    let from = Vec3::new(0.0, 0.0, 0.0);
    let (key, hit) = set.ray_cast(from, Vec3::X, 100.0).expect("the nearest");
    assert_eq!((*key, hit.distance, hit.material), (10, 9.0, Some(9)));
    let (key, hit) = set
        .ray_cast(Vec3::new(40.0, 0.0, 0.0), Vec3::NEG_X, 100.0)
        .expect("the nearest");
    assert_eq!((*key, hit.distance), (30, 9.0));
    let (key, hit) = set
        .sweep_sphere(from, Vec3::X, 100.0, 0.5)
        .expect("the nearest");
    assert_eq!((*key, hit.distance), (10, 8.5));
    // A sphere whose middle passes beside a bound's box touches the bound all the same:
    // 1.3 to one side, where radii of 1 and 0.5 reach 1.5.
    let beside = 1.3f32;
    let (key, hit) = set
        .sweep_sphere(Vec3::new(0.0, beside, 0.0), Vec3::X, 100.0, 0.5)
        .expect("it brushes the nearest");
    let short = (1.5f32 * 1.5 - beside * beside).sqrt();
    assert_eq!(*key, 10);
    assert!(
        (hit.distance - (10.0 - short)).abs() < 1e-5,
        "{}",
        hit.distance
    );
    // A sphere of no radius is a ray, and one of no number touches nothing.
    let aside = Vec3::new(0.0, 0.5, 0.0);
    let ray = set.ray_cast(aside, Vec3::X, 100.0);
    assert!(ray.is_some());
    for radius in [0.0, -1.0] {
        assert_eq!(set.sweep_sphere(aside, Vec3::X, 100.0, radius), ray);
    }
    for radius in [f32::NAN, f32::INFINITY] {
        assert_eq!(set.sweep_sphere(aside, Vec3::X, 100.0, radius), None);
    }

    assert!(set.remove(&10));
    assert!(!set.remove(&10), "it is gone");
    assert_eq!(set.world_box(&10), None);
    let (key, hit) = set.ray_cast(from, Vec3::X, 100.0).expect("the next");
    assert_eq!((*key, hit.distance), (20, 19.0));

    // A key added again takes the new place.
    set.add(20, ball.clone(), at(-5.0)).expect("a placement");
    assert_eq!(set.len(), 2);
    let (key, hit) = set
        .ray_cast(from, Vec3::X, 100.0)
        .expect("what is left ahead");
    assert_eq!((*key, hit.distance), (30, 29.0));
    let (key, hit) = set
        .ray_cast(from, Vec3::NEG_X, 100.0)
        .expect("the moved one");
    assert_eq!((*key, hit.distance), (20, 4.0));

    // Two bounds met at one distance: the lower key, whichever was added first.
    for order in [[1, 2], [2, 1]] {
        let mut twins: CollisionSet<u32> = CollisionSet::new();
        for key in order {
            twins.add(key, ball.clone(), at(10.0)).expect("a placement");
        }
        let (key, _) = twins.ray_cast(from, Vec3::X, 100.0).expect("either");
        assert_eq!(*key, 1);
    }
}

/// Points all through one polygon of a geometry, its extremes among them, from the vertices the bound holds.
fn samples(g: &PhBoundGeometry, polygon: usize) -> Vec<DVec3> {
    let p = &g.polygons[polygon];
    let (named, radius) = g.polygon_vertices(p).expect("a polygon of a known kind");
    let v: Vec<DVec3> = named.iter().map(|i| stored(g, *i)).collect();
    let radius = f64::from(radius);
    let steps = |n: usize| (0..=n).map(move |i| i as f64 / n as f64);
    // Directions all round, and the same shrunk towards the middle.
    let ball = |centre: DVec3, radius: f64| -> Vec<DVec3> {
        let mut out = vec![centre];
        for shell in [1.0, 0.6, 0.3] {
            for a in steps(48) {
                for b in steps(24) {
                    let (lon, lat) = (a * std::f64::consts::TAU, (b - 0.5) * std::f64::consts::PI);
                    let dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
                    out.push(centre + dir * radius * shell);
                }
            }
        }
        out
    };
    match p.primitive() {
        0 => {
            // No farther apart than two centimetres, so a corner of the box a tenth deep is seen.
            let longest = (v[1] - v[0]).length().max((v[2] - v[0]).length());
            let fine = ((longest / 0.02).ceil() as usize).clamp(120, 4000);
            steps(fine)
                .flat_map(|a| steps(fine).map(move |b| (a, b)))
                .filter(|(a, b)| a + b <= 1.0)
                .map(|(a, b)| v[0] + (v[1] - v[0]) * a + (v[2] - v[0]) * b)
                .collect()
        }
        1 => ball(v[0], radius),
        2 => steps(40)
            .flat_map(|t| ball(v[0].lerp(v[1], t), radius))
            .collect(),
        3 => {
            let centre = (v[0] + v[1] + v[2] + v[3]) * 0.25;
            let half = [
                (v[0] + v[1] - v[2] - v[3]) * 0.25,
                (v[0] + v[2] - v[1] - v[3]) * 0.25,
                (v[0] + v[3] - v[1] - v[2]) * 0.25,
            ];
            let side = || steps(12).map(|t| t * 2.0 - 1.0);
            side()
                .flat_map(|a| side().flat_map(move |b| side().map(move |c| (a, b, c))))
                .map(|(a, b, c)| centre + half[0] * a + half[1] * b + half[2] * c)
                .collect()
        }
        4 => {
            let axis = (v[1] - v[0]).normalize();
            let across = axis.any_orthonormal_vector();
            let other = axis.cross(across);
            let mut out = Vec::new();
            for t in steps(40) {
                for ring in [1.0, 0.6, 0.3, 0.0] {
                    for a in steps(96) {
                        let turn = a * std::f64::consts::TAU;
                        let out_by = (across * turn.cos() + other * turn.sin()) * radius * ring;
                        out.push(v[0].lerp(v[1], t) + out_by);
                    }
                }
            }
            out
        }
        kind => panic!("polygon {polygon} is of kind {kind}"),
    }
}

/// What a box holds of a geometry, by asking of each polygon's points whether the box holds them.
fn sorted_by_points(g: &PhBoundGeometry, region: &OrientedBox) -> BoxOverlap {
    let axes = [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| (region.rotation * axis).as_dvec3());
    let holds = |p: DVec3| {
        let from = p - region.centre.as_dvec3();
        (0..3).all(|i| from.dot(axes[i]).abs() <= f64::from(region.half_extents[i]))
    };
    let mut out = BoxOverlap::default();
    for polygon in 0..g.polygons.len() {
        let points = samples(g, polygon);
        let held = points.iter().filter(|p| holds(**p)).count();
        let part = Part {
            leaf: 0,
            polygon: Some(polygon as u32),
        };
        if held == points.len() {
            out.inside.push(part);
        } else if held > 0 {
            out.crossing.push(part);
        }
    }
    out
}

/// A point of the plane z = `z` from its place along the long and the short edge of a box turned 45 degrees about Z.
fn turned(along: f32, across: f32, z: f32) -> Vec3 {
    let h = std::f32::consts::FRAC_1_SQRT_2;
    Vec3::new((along - across) * h, (along + across) * h, z)
}

/// A box turned 45 degrees about Z, with triangles and primitives in it, across it and beside it.
///
/// ⚠ The pairs near its corner are the ones only a whole test tells apart: each lies within
/// the box's reach along all three of its edges, and one is in the box and the other beside it
/// by less than a tenth. What parts the one beside it is, in turn, an edge of a triangle, the
/// plane of a triangle, the roundness of a cylinder and of a sphere, and two edges of two boxes.
/// The pairs beside its long face are parted by that face alone: a tilted box that shares no
/// edge direction with it, and a cylinder no line to a corner parts. The cylinder past its
/// other corner is parted by the line to that corner alone, and the next lies along the
/// world's X where the box's upright edges are off both of its ends. The last pair is a tilted
/// box with a face of its own turned to the box's corner, which that face alone parts.
fn sorted_scene() -> (PhBoundGeometry, OrientedBox, Vec<u32>, Vec<u32>) {
    let region = OrientedBox {
        centre: Vec3::ZERO,
        rotation: Quat::from_rotation_z(45f32.to_radians()),
        half_extents: Vec3::new(2.0, 1.0, 1.0),
    };
    // A vector of the box's own frame, in the world's.
    let framed = |v: Vec3| turned(v.x, v.y, v.z);
    let mut positions: Vec<Vec3> = Vec::new();
    let mut put = |p: Vec3| {
        positions.push(p);
        (positions.len() - 1) as u16
    };
    let mut triangle = |a: Vec3, b: Vec3, c: Vec3| [put(a), put(b), put(c)];
    // A triangle eight across, in the plane that faces the box's top corner from `away`.
    let facing = |away: f32| {
        let normal = Vec3::ONE.normalize();
        let (right, up) = (
            Vec3::new(1.0, -1.0, 0.0).normalize(),
            Vec3::new(1.0, 1.0, -2.0).normalize(),
        );
        let middle = normal * away;
        [
            framed(middle + right * 8.0),
            framed(middle - right * 4.0 + up * 7.0),
            framed(middle - right * 4.0 - up * 7.0),
        ]
    };
    // The corner at 2, 1, 1 is this far from the box's middle along the plane's normal.
    let corner_away = Vec3::new(2.0, 1.0, 1.0).dot(Vec3::ONE.normalize());
    let (clear, cut) = (facing(corner_away + 0.09), facing(corner_away - 0.09));
    let triangles = vec![
        // 0: in the box.
        triangle(
            Vec3::new(-0.3, -0.3, 0.0),
            Vec3::new(0.3, -0.3, 0.0),
            Vec3::new(0.0, 0.3, 0.2),
        ),
        // 1: one corner in, two out.
        triangle(
            Vec3::new(0.0, 0.0, 0.5),
            Vec3::new(5.0, 5.0, 0.5),
            Vec3::new(5.0, 6.0, 0.5),
        ),
        // 2: beside the box, in the box of its corners.
        triangle(
            Vec3::new(1.85, -1.9, 0.0),
            Vec3::new(1.95, -1.9, 0.0),
            Vec3::new(1.9, -1.8, 0.0),
        ),
        // 3: far away.
        triangle(
            Vec3::new(20.0, 20.0, 0.0),
            Vec3::new(21.0, 20.0, 0.0),
            Vec3::new(20.0, 21.0, 0.0),
        ),
        // 4: every corner out, and the box through its middle.
        triangle(
            Vec3::new(-50.0, -50.0, 0.5),
            Vec3::new(50.0, -50.0, 0.5),
            Vec3::new(0.0, 60.0, 0.5),
        ),
        // 5: past the corner at 2 along and 1 back, its near edge 0.14 clear of it.
        triangle(
            turned(2.4, -0.8, 0.0),
            turned(1.8, -1.4, 0.0),
            turned(2.6, -1.6, 0.0),
        ),
        // 6: the same, its near edge 0.14 into the corner.
        triangle(
            turned(2.2, -0.6, 0.0),
            turned(1.6, -1.2, 0.0),
            turned(2.6, -1.6, 0.0),
        ),
        // 7: facing the top corner, its plane 0.09 clear of it; 8: its plane 0.09 into it.
        triangle(clear[0], clear[1], clear[2]),
        triangle(cut[0], cut[1], cut[2]),
    ];
    let mut primitives = vec![
        // 9: a sphere in the box.
        bounds::sphere_primitive(put(Vec3::new(0.5, 0.5, 0.0)), 0.3),
        // 10: a capsule out of its top; 11: one far away.
        bounds::capsule_primitive([put(Vec3::ZERO), put(Vec3::new(0.0, 0.0, 3.0))], 0.1),
        bounds::capsule_primitive(
            [
                put(Vec3::new(10.0, 10.0, 0.0)),
                put(Vec3::new(10.0, 10.0, 3.0)),
            ],
            0.1,
        ),
        // 12: a cylinder in the box; 13: one out of its top.
        bounds::cylinder_primitive(
            [
                put(Vec3::new(-0.5, 0.5, -0.3)),
                put(Vec3::new(-0.5, 0.5, 0.3)),
            ],
            0.2,
        ),
        bounds::cylinder_primitive(
            [put(Vec3::new(0.0, 0.0, 0.8)), put(Vec3::new(0.0, 0.0, 1.6))],
            0.3,
        ),
        // 14: an upright cylinder 0.21 from the corner and 0.2 round; 15: one 0.14 from it.
        bounds::cylinder_primitive(
            [
                put(turned(2.15, -1.15, -0.5)),
                put(turned(2.15, -1.15, 0.5)),
            ],
            0.2,
        ),
        bounds::cylinder_primitive(
            [put(turned(2.1, -1.1, -0.5)), put(turned(2.1, -1.1, 0.5))],
            0.2,
        ),
    ];
    // 16: a box in the box; 17: one out of its top; 18: one far away.
    let small = [Vec3::X * 0.1, Vec3::Y * 0.1, Vec3::Z * 0.1];
    let big = [Vec3::X * 0.3, Vec3::Y * 0.3, Vec3::Z * 0.3];
    for (centre, half) in [
        (Vec3::new(-1.0, 0.0, 0.0), small),
        (Vec3::new(0.0, 0.0, 1.0), big),
        (Vec3::new(5.0, -5.0, 0.0), small),
    ] {
        primitives.push(bounds::box_primitive(
            bounds::far_corners(centre, half).map(&mut put),
        ));
    }
    // 19: a sphere 0.21 from the corner and 0.2 round; 20: one 0.14 from it.
    primitives.push(bounds::sphere_primitive(put(turned(2.15, -1.15, 0.0)), 0.2));
    primitives.push(bounds::sphere_primitive(put(turned(2.1, -1.1, 0.0)), 0.2));
    // 21: a beam laid over the box's top edge, square in section and turned so that an edge of
    // its own crosses that edge 0.08 clear of it; 22: the same, 0.08 into it.
    let lean = Vec3::new(0.0, 1.0, 1.0).normalize();
    let (length, side) = (Vec3::new(0.0, 1.0, -1.0).normalize() * 1.5, 0.3f32);
    let faces = [(Vec3::X + lean).normalize(), (Vec3::X - lean).normalize()];
    for gap in [0.08f32, -0.08] {
        let middle = Vec3::new(0.0, 1.0, 1.0) + lean * (side * 2f32.sqrt() + gap);
        let half = [
            framed(length),
            framed(faces[0] * side),
            framed(faces[1] * side),
        ];
        primitives.push(bounds::box_primitive(
            bounds::far_corners(framed(middle), half).map(&mut put),
        ));
    }
    // 23: a tilted box beside the long face, its nearest corner 0.08 clear; 24: 0.08 into it.
    let tilt = Quat::from_euler(glam::EulerRot::XYZ, 0.5, 0.7, 0.9);
    let edges = [
        tilt * Vec3::X * 0.3,
        tilt * Vec3::Y * 0.2,
        tilt * Vec3::Z * 0.1,
    ];
    let reach: f32 = edges.iter().map(|edge| edge.y.abs()).sum();
    for gap in [0.08f32, -0.08] {
        let middle = Vec3::new(0.4, region.half_extents.y + reach + gap, 0.1);
        primitives.push(bounds::box_primitive(
            bounds::far_corners(framed(middle), edges.map(framed)).map(&mut put),
        ));
    }
    // An upright cylinder 0.2 round, its axis this far from the box and at this place of it.
    let mut upright = |along: f32, across: f32| {
        bounds::cylinder_primitive(
            [
                put(turned(along, across, -0.5)),
                put(turned(along, across, 0.5)),
            ],
            0.2,
        )
    };
    // 25: 0.21 from the middle of the long face; 26: 0.14 from it.
    primitives.push(upright(0.3, 1.21));
    primitives.push(upright(0.3, 1.14));
    // 27: 0.21 from the corner at 2 along and 1 across, 20 degrees round it from the long
    // edge's line, so nearer than 0.2 to the lines of both faces; 28: 0.14 from it.
    let (sin, cos) = 20f32.to_radians().sin_cos();
    primitives.push(upright(2.0 + 0.21 * cos, 1.0 + 0.21 * sin));
    primitives.push(upright(2.0 + 0.14 * cos, 1.0 + 0.14 * sin));
    // 29: a cylinder along the world's X, beside the box where it narrows to its corner.
    primitives.push(bounds::cylinder_primitive(
        [
            put(Vec3::new(1.6, -0.5, 0.0)),
            put(Vec3::new(2.0, -0.5, 0.0)),
        ],
        0.3,
    ));
    // 30: a tilted box whose face looks at the corner at 2, 1, 1 from 0.08 off; 31: the corner
    // 0.08 through that face.
    let facing = Vec3::new(1.0, 0.8, 0.6).normalize();
    let tilt = Quat::from_rotation_arc(Vec3::X, facing) * Quat::from_rotation_x(0.4);
    // ⚠ The face is wide: under a narrow one an edge of it parts the two as well.
    let edges = [
        tilt * Vec3::X * 0.3,
        tilt * Vec3::Y * 0.6,
        tilt * Vec3::Z * 0.5,
    ];
    for gap in [0.08f32, -0.08] {
        let middle = region.half_extents + facing * (0.3 + gap);
        primitives.push(bounds::box_primitive(
            bounds::far_corners(framed(middle), edges.map(framed)).map(&mut put),
        ));
    }
    let g = bounds::geometry(&positions, &triangles, primitives, &[], &[]);
    (
        g,
        region,
        vec![0, 9, 12, 16],
        vec![1, 4, 6, 8, 10, 13, 15, 17, 20, 22, 24, 26, 28, 31],
    )
}

fn polygons(parts: &[Part]) -> Vec<u32> {
    parts
        .iter()
        .map(|part| part.polygon.expect("a polygon of the geometry"))
        .collect()
}

#[test]
fn a_box_sorts_the_polygons_of_a_bound_into_those_it_holds_those_it_cuts_and_the_rest() {
    let (g, region, inside, crossing) = sorted_scene();
    assert_eq!(g.polygons.len(), 32);
    // The scene is what it was drawn to be: its points say so before the collider does.
    let by_points = sorted_by_points(&g, &region);
    assert_eq!(polygons(&by_points.inside), inside);
    assert_eq!(polygons(&by_points.crossing), crossing);

    let bound = BoundCollider::new(&PhBoundAny::PhBoundGeometry(g)).expect("a whole bound");
    let sorted = bound.overlap_box(&region);
    assert_eq!(polygons(&sorted.inside), inside, "wholly in the box");
    assert_eq!(polygons(&sorted.crossing), crossing, "partly in the box");
    assert_eq!(sorted, by_points);

    // A rotation is a direction whatever its length, and a box of no numbers holds nothing.
    let long = OrientedBox {
        rotation: region.rotation * 3.0,
        ..region
    };
    assert_eq!(bound.overlap_box(&long), sorted);
    // Half an extent is a size, whichever sign it is given.
    let negated = OrientedBox {
        half_extents: -region.half_extents,
        ..region
    };
    assert_eq!(bound.overlap_box(&negated), sorted);
    let broken = OrientedBox {
        centre: Vec3::splat(f32::NAN),
        ..region
    };
    assert_eq!(bound.overlap_box(&broken), BoxOverlap::default());

    // A box that holds nothing, and one that holds everything.
    let nowhere = OrientedBox {
        centre: Vec3::new(0.0, 0.0, 30.0),
        ..region
    };
    assert_eq!(bound.overlap_box(&nowhere), BoxOverlap::default());
    let everywhere = OrientedBox {
        half_extents: Vec3::splat(200.0),
        ..region
    };
    let all = bound.overlap_box(&everywhere);
    assert_eq!(polygons(&all.inside), (0..32).collect::<Vec<u32>>());
    assert!(all.crossing.is_empty());
}

#[test]
fn a_box_sorts_many_scattered_polygons_as_their_points_say() {
    let mut dice = bounds::Dice(0xB0C5);
    let mut positions: Vec<Vec3> = Vec::new();
    let mut put = |p: Vec3| {
        positions.push(p);
        (positions.len() - 1) as u16
    };
    let mut triangles = Vec::new();
    let mut primitives = Vec::new();
    for _ in 0..80 {
        let at = dice.point(-4.0, 4.0);
        triangles.push([0; 3].map(|_| put(at + dice.point(-1.0, 1.0))));
    }
    for i in 0..60 {
        let at = dice.point(-4.0, 4.0);
        let radius = dice.between(0.2, 0.7);
        let other = at + dice.point(-1.0, 1.0);
        primitives.push(match i % 4 {
            0 => bounds::sphere_primitive(put(at), radius),
            1 => bounds::capsule_primitive([put(at), put(other)], radius),
            2 => bounds::cylinder_primitive([put(at), put(other)], radius),
            _ => {
                let turn = Quat::from_euler(
                    glam::EulerRot::XYZ,
                    dice.between(0.0, 3.0),
                    dice.between(0.0, 3.0),
                    dice.between(0.0, 3.0),
                );
                let half = [
                    turn * Vec3::X * radius,
                    turn * Vec3::Y * 0.5,
                    turn * Vec3::Z * 0.3,
                ];
                bounds::box_primitive(bounds::far_corners(at, half).map(&mut put))
            }
        });
    }
    let g = bounds::geometry(&positions, &triangles, primitives, &[], &[]);
    let bound = BoundCollider::new(&PhBoundAny::PhBoundGeometry(g.clone())).expect("a whole bound");

    let (mut inside, mut crossing, mut doubtful) = (0, 0, 0);
    for _ in 0..12 {
        let region = OrientedBox {
            centre: dice.point(-2.0, 2.0),
            rotation: Quat::from_euler(
                glam::EulerRot::XYZ,
                dice.between(0.0, 3.0),
                dice.between(0.0, 3.0),
                dice.between(0.0, 3.0),
            ),
            half_extents: Vec3::new(
                dice.between(1.5, 4.0),
                dice.between(1.5, 4.0),
                dice.between(1.5, 4.0),
            ),
        };
        let got = bound.overlap_box(&region);
        let want = sorted_by_points(&g, &region);
        inside += got.inside.len();
        crossing += got.crossing.len();
        // ⚠ Points are a finite sample: a polygon that dips into the box between two of them
        // is cut and looks clear, and one whose last sliver is out looks held. So the points
        // may only err towards less contact, never towards more.
        for part in &want.inside {
            assert!(
                got.inside.contains(part) || got.crossing.contains(part),
                "{part:?} has every point in the box and was left out"
            );
        }
        for part in &want.crossing {
            assert!(
                got.crossing.contains(part),
                "{part:?} has points in and out of the box"
            );
        }
        for part in &got.inside {
            assert!(
                want.inside.contains(part),
                "{part:?} is held, with a point outside"
            );
        }
        doubtful += got
            .crossing
            .iter()
            .filter(|part| !want.crossing.contains(part))
            .count();
    }
    assert!(
        inside > 100 && crossing > 100,
        "{inside} held and {crossing} cut"
    );
    assert!(
        doubtful * 20 <= crossing,
        "{doubtful} of {crossing} cut polygons have no point that says so"
    );
}

#[test]
fn a_set_sorts_each_bound_a_box_reaches_in_the_bounds_own_space() {
    let (g, region, inside, crossing) = sorted_scene();
    let scene = collider(&PhBoundAny::PhBoundGeometry(g));
    let place = Affine3A::from_scale_rotation_translation(
        Vec3::splat(3.0),
        Quat::from_rotation_x(40f32.to_radians()),
        Vec3::new(500.0, -200.0, 30.0),
    );
    let mut set: CollisionSet<&str> = CollisionSet::new();
    set.add("here", scene.clone(), Affine3A::IDENTITY)
        .expect("a placement");
    set.add("there", scene.clone(), place).expect("a placement");
    set.add(
        "elsewhere",
        scene,
        Affine3A::from_translation(Vec3::new(0.0, 0.0, 5000.0)),
    )
    .expect("a placement");
    // In the corner of the box's own box and beside the box: reached, and nothing of it held.
    let ball = collider(&bounds::sphere(Vec3::new(1.9, -1.9, 0.0), 0.2, 0));
    set.add("beside", ball, Affine3A::IDENTITY)
        .expect("a placement");

    let found = set.overlap_box(&region);
    assert_eq!(found.len(), 1, "one bound is under the box");
    assert_eq!(*found[0].0, "here");
    assert_eq!(polygons(&found[0].1.inside), inside);

    // The same box carried by the second bound's transform sorts that bound the same way.
    let (scale, rotation, translation) = place.to_scale_rotation_translation();
    let carried = OrientedBox {
        centre: translation,
        rotation: rotation * region.rotation,
        half_extents: region.half_extents * scale,
    };
    let found = set.overlap_box(&carried);
    assert_eq!(found.len(), 1);
    assert_eq!(*found[0].0, "there");
    assert_eq!(polygons(&found[0].1.inside), inside);
    assert_eq!(polygons(&found[0].1.crossing), crossing);
}

fn refusal(bound: &PhBoundAny) -> CollisionError {
    BoundCollider::new(bound).expect_err("a bound that names what it does not hold")
}

fn floor_triangle() -> PhBoundGeometry {
    bounds::geometry(
        &[
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(0.0, 4.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        ],
        &[[0, 1, 2]],
        Vec::new(),
        &[0x11, 0x22],
        &[1],
    )
}

#[test]
fn a_polygon_that_names_what_its_bound_does_not_hold_is_refused() {
    let whole = floor_triangle();
    let vertices = whole.compressed_vertices.len();
    assert!(BoundCollider::new(&PhBoundAny::PhBoundGeometry(whole.clone())).is_ok());

    let mut g = whole.clone();
    g.polygons[0].vertex_indices[2] = 999;
    assert_eq!(
        refusal(&PhBoundAny::PhBoundGeometry(g)),
        CollisionError::Vertex {
            leaf: 0,
            polygon: 0,
            vertex: 999,
            vertices
        }
    );
    // The top bit of an index is not part of it, so it names no vertex past the end.
    let mut g = whole.clone();
    g.polygons[0].vertex_indices[2] |= 0x8000;
    assert!(BoundCollider::new(&PhBoundAny::PhBoundGeometry(g)).is_ok());

    for (primitive, named) in [
        (bounds::sphere_primitive(40, 0.5), 40),
        (bounds::capsule_primitive([3, 41], 0.5), 41),
        (bounds::cylinder_primitive([42, 3], 0.5), 42),
        (bounds::box_primitive([0, 1, 2, 43]), 43),
    ] {
        let mut g = whole.clone();
        g.polygons.push(primitive);
        g.poly_mat_index_list.push(0);
        assert_eq!(
            refusal(&PhBoundAny::PhBoundGeometry(g)),
            CollisionError::Vertex {
                leaf: 0,
                polygon: 1,
                vertex: named,
                vertices
            }
        );
    }

    let mut g = whole.clone();
    g.poly_mat_index_list[0] = 7;
    assert_eq!(
        refusal(&PhBoundAny::PhBoundGeometry(g)),
        CollisionError::Material {
            leaf: 0,
            polygon: 0,
            material: 7,
            materials: 2
        }
    );
    let mut g = whole.clone();
    g.poly_mat_index_list.push(0);
    assert_eq!(
        refusal(&PhBoundAny::PhBoundGeometry(g)),
        CollisionError::MaterialIndices {
            leaf: 0,
            indices: 2,
            polygons: 1
        }
    );

    let mut g = whole.clone();
    let area = &mut g.polygons[0].area;
    *area = f32::from_bits(area.to_bits() | 0b101);
    assert_eq!(
        refusal(&PhBoundAny::PhBoundGeometry(g)),
        CollisionError::UnknownPrimitive {
            leaf: 0,
            polygon: 0,
            bits: 0b101
        }
    );

    // The second bound of a composite is named as the second.
    let mut g = whole.clone();
    g.polygons[0].vertex_indices[0] = 500;
    let nested = bounds::composite(vec![
        (Some(bounds::sphere(Vec3::ZERO, 1.0, 0)), Affine3A::IDENTITY),
        (Some(PhBoundAny::PhBoundGeometry(g)), Affine3A::IDENTITY),
    ]);
    assert!(matches!(
        refusal(&nested),
        CollisionError::Vertex {
            leaf: 1,
            vertex: 500,
            ..
        }
    ));
}

#[test]
fn a_bound_of_sizes_and_places_that_are_no_numbers_is_refused() {
    let number =
        |bound: &PhBoundAny| matches!(refusal(bound), CollisionError::Number { leaf: 0, .. });

    let mut g = floor_triangle();
    g.un_quantize_factor.x = f32::NAN;
    assert!(number(&PhBoundAny::PhBoundGeometry(g)));
    let mut g = floor_triangle();
    g.bounding_box_center.z = f32::INFINITY;
    assert!(number(&PhBoundAny::PhBoundGeometry(g)));
    // ⚠ Corners past what a number holds beside one in place: the least and the greatest of
    // the three pass over what is no number, so their box alone says nothing is wrong.
    let lost = |polygon: Option<or_io::gta5::PhPolygon>| {
        let mut g = floor_triangle();
        g.un_quantize_factor.x = f32::MAX;
        for (vertex, x) in [(0, -2), (1, 0), (2, -2), (3, 0)] {
            g.compressed_vertices[vertex].x = x;
        }
        if let Some(polygon) = polygon {
            g.polygons = vec![polygon];
        }
        PhBoundAny::PhBoundGeometry(g)
    };
    assert!(number(&lost(None)), "a triangle with two corners lost");
    assert!(
        number(&lost(Some(bounds::capsule_primitive([1, 0], 0.5)))),
        "a capsule with one end lost"
    );
    // The same vertices with the lost ones left out make a bound that is whole.
    assert!(BoundCollider::new(&lost(Some(bounds::capsule_primitive([1, 3], 0.5)))).is_ok());
    let mut g = floor_triangle();
    g.polygons.push(bounds::sphere_primitive(3, -0.5));
    g.poly_mat_index_list.push(0);
    assert!(number(&PhBoundAny::PhBoundGeometry(g)));
    let mut g = floor_triangle();
    g.polygons.push(bounds::capsule_primitive([0, 3], f32::NAN));
    g.poly_mat_index_list.push(0);
    assert!(number(&PhBoundAny::PhBoundGeometry(g)));
    // A sphere in its place and of no end: every number it states is one, and its reach is not.
    let mut g = floor_triangle();
    g.polygons.push(bounds::sphere_primitive(3, f32::INFINITY));
    g.poly_mat_index_list.push(0);
    assert!(number(&PhBoundAny::PhBoundGeometry(g)));

    assert!(number(&bounds::sphere(Vec3::ZERO, -1.0, 0)));
    assert!(number(&bounds::sphere(
        Vec3::new(f32::NAN, 0.0, 0.0),
        1.0,
        0
    )));
    // A capsule whose radius around its centroid is less than its own radius.
    assert!(number(&bounds::capsule(Vec3::ZERO, 0.5, -0.75, 0)));
    // A box whose minimum is past its maximum.
    assert!(number(&bounds::boxed(
        Vec3::splat(1.0),
        Vec3::splat(-1.0),
        0
    )));
    assert!(number(&bounds::cylinder(Vec3::ZERO, -0.5, 1.0, 0)));
    // A cylinder whose box runs backwards along its axis.
    assert!(number(&bounds::cylinder(Vec3::ZERO, 0.5, -1.0, 0)));
    // ⚠ A cylinder is as wide as the wider of its box's two widths, and one of them is no
    // number: the wider of the two would be the other.
    let mut post = bounds::cylinder(Vec3::ZERO, 0.5, 1.0, 0);
    if let PhBoundAny::PhBoundCylinder(c) = &mut post {
        c.base.bounding_box_max_xyz_margin_w.x = f32::NAN;
    }
    assert!(number(&post));
    // A disc whose margin is no number states no thickness, where one of no margin states its box's.
    let mut plate = bounds::disc(Vec3::ZERO, 2.0, 0.1, 0.1, 0);
    if let PhBoundAny::PhBoundDisc(d) = &mut plate {
        d.base.bounding_box_max_xyz_margin_w.w = f32::NAN;
    }
    assert!(number(&plate));
    assert!(number(&bounds::disc(
        Vec3::ZERO,
        f32::INFINITY,
        0.1,
        0.1,
        0
    )));
}

#[test]
fn a_composite_with_a_child_it_cannot_place_is_refused() {
    let ball = || Some(bounds::sphere(Vec3::ZERO, 1.0, 0));
    let mut short = bounds::composite(vec![
        (ball(), Affine3A::IDENTITY),
        (ball(), Affine3A::IDENTITY),
    ]);
    let PhBoundAny::PhBoundComposite(c) = &mut short else {
        unreachable!("a composite was built");
    };
    c.current_matrices.truncate(1);
    assert_eq!(
        refusal(&short),
        CollisionError::ChildMatrix {
            child: 1,
            bounds: 2,
            matrices: 1
        }
    );

    // Three axes of a real turn, so that none lies along the world's, then put out of true.
    let turn = Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.5, 0.8);
    let [x, y, z] = Vec3::AXES.map(|axis| turn * axis);
    let axes = |x: Vec3, y: Vec3, z: Vec3| Affine3A::from_mat3(Mat3::from_cols(x, y, z));
    // One axis leant a degree towards another, its length kept.
    let (lean, upright) = 1f32.to_radians().sin_cos();
    let flat = Affine3A::from_scale(Vec3::ZERO);
    let nowhere = Affine3A::from_translation(Vec3::new(f32::NAN, 0.0, 0.0));
    for (place, what) in [
        (
            axes(x, y * 1.01, z),
            "a hundredth longer along its second axis",
        ),
        (
            axes(x, y, z * 1.01),
            "a hundredth longer along its third axis",
        ),
        (
            axes(x, y * upright + x * lean, z),
            "with its second axis leaning on its first",
        ),
        (
            axes(x, y, z * upright + x * lean),
            "with its third axis leaning on its first",
        ),
        (
            axes(x, y, z * upright + y * lean),
            "with its third axis leaning on its second",
        ),
        (flat, "of no size"),
        (nowhere, "nowhere"),
    ] {
        let built = bounds::composite(vec![(ball(), Affine3A::IDENTITY), (ball(), place)]);
        assert_eq!(
            refusal(&built),
            CollisionError::ChildTransform { child: 1 },
            "a child placed {what}"
        );
        // A set refuses the same of the transform a bound is added with, and holds nothing of it.
        let mut set: CollisionSet<u8> = CollisionSet::new();
        let whole = collider(&bounds::sphere(Vec3::ZERO, 1.0, 0));
        assert_eq!(
            set.add(0, whole, place),
            Err(CollisionError::Placement),
            "placed {what}"
        );
        assert!(set.is_empty());
    }
    // What rounding leaves of a turn, a ten-thousandth out of true, is a turn still.
    for place in [
        axes(x, y, z),
        axes(x, y * 1.0001, z),
        axes(x, y, z * upright + x * 1e-4),
    ] {
        let built = bounds::composite(vec![(ball(), place)]);
        assert_eq!(BoundCollider::new(&built).map(|b| b.shape_count()), Ok(1));
    }
    // A mirror keeps a sphere a sphere, and a child placed by one is kept.
    let mirrored = Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    let built = bounds::composite(vec![(ball(), mirrored)]);
    assert_eq!(BoundCollider::new(&built).map(|b| b.shape_count()), Ok(1));
}

#[test]
fn what_states_no_shape_is_counted_and_never_met() {
    let plane = PhBoundAny::PhBoundPlane(PhBoundPlane {
        base: bounds::header(
            BoundType::Plane,
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
            0.0,
            1.0,
            0,
        ),
        ..Default::default()
    });
    let bare = PhBoundAny::Base(bounds::header(
        BoundType::Triangle,
        Vec3::splat(-1.0),
        Vec3::splat(1.0),
        0.0,
        1.0,
        0,
    ));
    // A cylinder primitive whose two ends are one vertex has no axis.
    let mut g = floor_triangle();
    g.polygons.push(bounds::cylinder_primitive([3, 3], 0.5));
    g.poly_mat_index_list.push(0);
    let built = bounds::composite(vec![
        (Some(plane), Affine3A::IDENTITY),
        (Some(bare), Affine3A::IDENTITY),
        (Some(PhBoundAny::PhBoundGeometry(g)), Affine3A::IDENTITY),
    ]);
    let bound = BoundCollider::new(&built).expect("nothing here is broken");
    assert_eq!((bound.shape_count(), bound.shapeless()), (1, 3));

    // Only the triangle is there to meet, and it is the third bound's first polygon.
    let down = |x: f32, y: f32| bound.ray_cast(Vec3::new(x, y, 5.0), Vec3::NEG_Z, 100.0);
    let hit = down(0.5, 0.5).expect("the triangle");
    assert_eq!(
        hit.part,
        Part {
            leaf: 2,
            polygon: Some(0)
        }
    );
    assert_eq!(
        down(-0.5, -0.5),
        None,
        "in the plane's and the bare header's box"
    );

    // A bound of no shape at all has an empty box and is asked nothing.
    let empty = collider(&bounds::composite(Vec::new()));
    assert!(empty.local_box().is_empty());
    let mut set: CollisionSet<u8> = CollisionSet::new();
    set.add(0, empty, Affine3A::IDENTITY).expect("a placement");
    assert!(set.world_box(&0).expect("it is in the set").is_empty());
    assert_eq!(set.ray_cast(Vec3::ZERO, Vec3::X, 100.0), None);
}

#[test]
fn a_polygon_takes_the_material_its_index_names_or_the_first_or_none() {
    let down = |g: PhBoundGeometry| {
        BoundCollider::new(&PhBoundAny::PhBoundGeometry(g))
            .expect("a whole bound")
            .ray_cast(Vec3::new(1.0, 1.0, 5.0), Vec3::NEG_Z, 100.0)
            .expect("the triangle")
            .material
    };
    assert_eq!(
        down(floor_triangle()),
        Some(0x22),
        "the one its index names"
    );
    let mut g = floor_triangle();
    g.poly_mat_index_list.clear();
    assert_eq!(down(g), Some(0x11), "the first, with no index");
    let mut g = floor_triangle();
    g.poly_mat_index_list.clear();
    g.material_ids.clear();
    assert_eq!(down(g), None, "none, where the bound lists none");
}

/// A strip of `count` triangles along X.
fn strip(count: u16) -> PhBoundAny {
    let positions: Vec<Vec3> = (0..count + 2)
        .map(|v| Vec3::new(f32::from(v / 2), f32::from(v % 2), 0.0))
        .collect();
    let triangles: Vec<[u16; 3]> = (0..count).map(|t| [t, t + 1, t + 2]).collect();
    PhBoundAny::PhBoundGeometry(bounds::triangles(&positions, &triangles))
}

#[test]
fn a_collider_says_how_many_bytes_it_holds() {
    let bytes = |bound: &PhBoundAny| collider(bound).size_in_bytes();
    let none = bytes(&PhBoundAny::PhBoundPlane(PhBoundPlane {
        base: bounds::header(
            BoundType::Plane,
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
            0.0,
            1.0,
            0,
        ),
        ..Default::default()
    }));
    assert_eq!(
        none,
        std::mem::size_of::<BoundCollider>(),
        "a bound of no shape holds itself alone"
    );
    let (thousand, two_thousand) = (bytes(&strip(1000)), bytes(&strip(2000)));
    // A triangle is three corners of three floats, and one place in the tree's order.
    let least = 3 * std::mem::size_of::<Vec3>() + std::mem::size_of::<u32>();
    assert!(
        thousand - none >= 1000 * least,
        "{} bytes for a thousand triangles",
        thousand - none
    );
    // Twice the triangles are twice the bytes, to the few nodes a tree's last level rounds by.
    let (first, second) = ((thousand - none) as f64, (two_thousand - thousand) as f64);
    assert!(
        (second / first - 1.0).abs() < 0.1,
        "{first} bytes for the first thousand, {second} for the second"
    );
}

/// A level floor at height nought of `cells` by `cells` squares of `cell` metres, its lowest corner at `corner`.
fn level_grid(corner: Vec3, cell: f32, cells: usize) -> PhBoundGeometry {
    let side = cells + 1;
    let mut positions = Vec::new();
    for y in 0..side {
        for x in 0..side {
            positions.push(corner + Vec3::new(x as f32 * cell, y as f32 * cell, 0.0));
        }
    }
    let mut triangles = Vec::new();
    for y in 0..cells {
        for x in 0..cells {
            let at = (y * side + x) as u16;
            let side = side as u16;
            triangles.push([at, at + 1, at + side + 1]);
            triangles.push([at, at + side + 1, at + side]);
        }
    }
    let material = vec![0; triangles.len()];
    bounds::geometry(&positions, &triangles, Vec::new(), &[FLOOR], &material)
}

#[test]
fn a_level_floor_is_ground_to_a_probe_that_takes_no_slope_at_all() {
    // ⚠ The normal of a level triangle comes out of its cross product an ulp under one, at these cell sizes and places among others.
    let far = Vec3::new(3000.0, -3000.0, 0.0);
    for (corner, cell) in [(Vec3::ZERO, 3.3), (far, 0.37), (far, 7.77), (far, 1.0)] {
        let set = set_of(vec![(
            "floor",
            PhBoundAny::PhBoundGeometry(level_grid(corner, cell, 4)),
            Affine3A::IDENTITY,
        )]);
        for max_slope in [0.0, 1e-4] {
            let probe = GroundProbe {
                max_rise: 1.0,
                max_drop: 10.0,
                max_slope,
            };
            let mut missed = Vec::new();
            for y in 0..4 {
                for x in 0..4 {
                    // Inside each of the two triangles of a square, off the edge they share.
                    for (u, v) in [(0.7, 0.2), (0.2, 0.7)] {
                        let over =
                            corner + Vec3::new((x as f32 + u) * cell, (y as f32 + v) * cell, 0.5);
                        if set.ground(over, &probe).is_none() {
                            missed.push(over);
                        }
                    }
                }
            }
            assert!(
                missed.is_empty(),
                "cells of {cell} m at {corner}, a slope of {max_slope}: no ground under {} of 32 points, the first {:?}",
                missed.len(),
                missed.first()
            );
        }
    }
    // A slope is still refused by a probe that takes none: the steep triangle of the building is no ground to it.
    let g = building();
    let set = set_of(vec![(
        "building",
        PhBoundAny::PhBoundGeometry(g),
        Affine3A::IDENTITY,
    )]);
    let (_, hit) = set
        .ground(Vec3::new(2.0, 2.0, 2.9), &walker(0.0, 0.0))
        .expect("the floor under the steep triangle");
    assert_at(hit.position, Vec3::new(2.0, 2.0, 0.0));
}

/// ⚠ A point the ground query answered lies a rounding either side of the plane it was found on, and the probe of a second ask
/// with no rise starts at the point itself.
#[test]
fn the_ground_asked_at_the_height_it_answered_with_no_rise_is_the_same_ground() {
    let mut dice = Dice(7);
    let (mut asked, mut missed) = (0, 0);
    let mut furthest = 0.0f32;
    for _ in 0..200 {
        // One sloped triangle, far out on the map.
        let corners = [
            Vec3::new(-30.0, -30.0, dice.between(-4.0, 4.0)),
            Vec3::new(30.0, -30.0, dice.between(-4.0, 4.0)),
            Vec3::new(0.0, 40.0, dice.between(-4.0, 4.0)),
        ];
        let far = Vec3::new(
            dice.between(-7000.0, 7000.0),
            dice.between(-7000.0, 7000.0),
            dice.between(0.0, 600.0),
        );
        let set = set_of(vec![(
            "terrain",
            PhBoundAny::PhBoundGeometry(bounds::triangles(&corners, &[[0, 1, 2]])),
            Affine3A::from_translation(far),
        )]);
        let probe = GroundProbe {
            max_rise: 0.0,
            max_drop: 50.0,
            max_slope: 60f32.to_radians(),
        };
        for _ in 0..20 {
            let over = Vec3::new(dice.between(-8.0, 8.0), dice.between(-12.0, 8.0), 20.0);
            let (_, first) = set
                .ground(far + over, &probe)
                .expect("the terrain is under the point");
            asked += 1;
            match set.ground(first.position, &probe) {
                Some((_, again)) => {
                    furthest = furthest.max((again.position.z - first.position.z).abs());
                }
                None => missed += 1,
            }
        }
    }
    assert_eq!(missed, 0, "of {asked} asks at the ground's own height");
    assert!(
        furthest < 0.02,
        "the ground moved {furthest} between two asks"
    );
}
