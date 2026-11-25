//! A ray and a swept sphere against every kind of bound, built by hand, with each answer held to
//! arithmetic done here: a closed form where the geometry gives one, and otherwise the first
//! place along the path that a distance formula says is in the solid.

use glam::{Affine3A, DVec3, Quat, Vec3};
use or_io::gta5::{PhBoundAny, PhBoundGeometry};
use or_world::collision::{BoundCollider, CollisionSet, Hit, Part};

#[path = "support/bounds.rs"]
mod bounds;
use bounds::{first_within, stored, Dice, Solid};

/// A hundredth of a millimetre a metre: what single precision leaves of a distance here.
const CLOSE: f64 = 1e-5;

fn collider(bound: &PhBoundAny) -> BoundCollider {
    BoundCollider::new(bound).expect("a bound built by hand is whole")
}

fn d(v: Vec3) -> DVec3 {
    v.as_dvec3()
}

#[track_caller]
fn assert_hit(hit: &Hit, distance: f64, position: DVec3, normal: DVec3) {
    let scale = distance.abs().max(position.abs().max_element()).max(1.0);
    assert!(
        (f64::from(hit.distance) - distance).abs() <= CLOSE * scale,
        "distance {} for {distance}",
        hit.distance
    );
    assert!(
        (d(hit.position) - position).length() <= CLOSE * scale,
        "position {} for {position}",
        hit.position
    );
    assert!(
        (d(hit.normal) - normal).length() <= 1e-4,
        "normal {} for {normal}",
        hit.normal
    );
}

const WHOLE: Part = Part {
    leaf: 0,
    polygon: None,
};

#[test]
fn a_ray_meets_a_sphere_where_the_sphere_is() {
    let (centre, radius, material) = (Vec3::new(2.0, 3.0, 4.0), 1.5f32, 0x1_0000_0039u64);
    let ball = collider(&bounds::sphere(centre, radius, material));
    let off = 0.9f64;
    let origin = Vec3::new(2.9, 3.0, 10.0);
    let hit = ball.ray_cast(origin, Vec3::NEG_Z, 100.0).expect("a hit");
    let rise = (f64::from(radius).powi(2) - off * off).sqrt();
    let at = d(centre) + DVec3::new(off, 0.0, rise);
    assert_hit(
        &hit,
        f64::from(origin.z) - at.z,
        at,
        (at - d(centre)) / f64::from(radius),
    );
    assert_eq!(hit.material, Some(material));
    assert_eq!(hit.part, WHOLE);

    // The same ray, too short to arrive, and one that passes beside the sphere.
    let short = (f64::from(origin.z) - at.z - 0.01) as f32;
    assert_eq!(ball.ray_cast(origin, Vec3::NEG_Z, short), None);
    let beside = Vec3::new(2.0 + radius + 0.01, 3.0, 10.0);
    assert_eq!(ball.ray_cast(beside, Vec3::NEG_Z, 100.0), None);
    // A direction is a direction whatever its length.
    let long = ball
        .ray_cast(origin, Vec3::NEG_Z * 40.0, 100.0)
        .expect("a hit");
    assert_eq!(long, hit);
}

#[test]
fn a_ray_meets_a_capsule_on_its_side_and_on_its_round_ends() {
    let (radius, half) = (0.5f64, 1.0f64);
    let pill = collider(&bounds::capsule(Vec3::ZERO, radius as f32, half as f32, 7));

    let side = pill
        .ray_cast(Vec3::new(5.0, 0.25, 0.0), Vec3::NEG_X, 100.0)
        .expect("the side");
    assert_hit(&side, 5.0 - radius, DVec3::new(radius, 0.25, 0.0), DVec3::X);

    // Down onto the top end, which is a ball about the end of the axis and no flat face.
    let off = 0.3f64;
    let top = pill
        .ray_cast(Vec3::new(off as f32, 5.0, 0.0), Vec3::NEG_Y, 100.0)
        .expect("the top end");
    let rise = (radius * radius - off * off).sqrt();
    let at = DVec3::new(off, half + rise, 0.0);
    assert_hit(&top, 5.0 - at.y, at, DVec3::new(off, rise, 0.0) / radius);
    assert!(
        at.y < half + radius - 0.05,
        "a flat end would have been met at {}",
        half + radius
    );

    // Across the round end, past where the side stops.
    let up = 0.2f64;
    let across = pill
        .ray_cast(Vec3::new(5.0, (half + up) as f32, 0.0), Vec3::NEG_X, 100.0)
        .expect("the round end from the side");
    let reach = (radius * radius - up * up).sqrt();
    assert_hit(
        &across,
        5.0 - reach,
        DVec3::new(reach, half + up, 0.0),
        DVec3::new(reach, up, 0.0) / radius,
    );
    assert_eq!(across.material, Some(7));
    let over = Vec3::new(5.0, (half + radius) as f32 + 0.01, 0.0);
    assert_eq!(pill.ray_cast(over, Vec3::NEG_X, 100.0), None);
}

#[test]
fn a_ray_meets_a_box_on_the_face_it_enters_by() {
    let (min, max) = (Vec3::new(-1.0, -2.0, -3.0), Vec3::new(1.0, 2.0, 3.0));
    let crate_ = collider(&bounds::boxed(min, max, 0x38));

    let down = crate_
        .ray_cast(Vec3::new(0.5, 0.5, 10.0), Vec3::NEG_Z, 100.0)
        .expect("the top");
    assert_hit(
        &down,
        10.0 - f64::from(max.z),
        DVec3::new(0.5, 0.5, max.z.into()),
        DVec3::Z,
    );

    // Slanted in the XY plane: it reaches the face at x = 1 after four units of x.
    let direction = DVec3::new(-1.0, 0.1, 0.0);
    let slanted = crate_
        .ray_cast(Vec3::new(5.0, 0.0, 0.0), direction.as_vec3(), 100.0)
        .expect("the side");
    let run = 5.0 - f64::from(max.x);
    assert_hit(
        &slanted,
        run * direction.length(),
        DVec3::new(max.x.into(), run * 0.1, 0.0),
        DVec3::X,
    );
    assert_eq!(slanted.material, Some(0x38));

    // The same slant from where it leaves by the side at y = 2 before it reaches x = 1.
    let steep = DVec3::new(-1.0, 1.0, 0.0);
    assert_eq!(
        crate_.ray_cast(Vec3::new(5.0, 0.0, 0.0), steep.as_vec3(), 100.0),
        None
    );
}

#[test]
fn a_ray_meets_a_cylinder_on_its_flat_end_and_on_its_side() {
    let (centre, radius, half) = (Vec3::new(1.0, 1.0, 1.0), 0.75f64, 2.0f64);
    let post = collider(&bounds::cylinder(centre, radius as f32, half as f32, 0x57));

    // Down the axis, off centre: a flat end, where a capsule's would be round.
    let end = post
        .ray_cast(Vec3::new(1.3, 10.0, 1.0), Vec3::NEG_Y, 100.0)
        .expect("the flat end");
    let top = f64::from(centre.y) + half;
    assert_hit(&end, 10.0 - top, DVec3::new(1.3, top, 1.0), DVec3::Y);

    let off = 0.3f64;
    let side = post
        .ray_cast(Vec3::new(10.0, 1.5, 1.3), Vec3::NEG_X, 100.0)
        .expect("the side");
    let reach = (radius * radius - off * off).sqrt();
    assert_hit(
        &side,
        10.0 - (f64::from(centre.x) + reach),
        DVec3::new(f64::from(centre.x) + reach, 1.5, 1.3),
        DVec3::new(reach, 0.0, off) / radius,
    );
    assert_eq!(side.material, Some(0x57));

    let past_the_rim = Vec3::new(1.0 + radius as f32 + 0.01, 10.0, 1.0);
    assert_eq!(post.ray_cast(past_the_rim, Vec3::NEG_Y, 100.0), None);
    let past_the_end = Vec3::new(10.0, 1.0 + half as f32 + 0.01, 1.0);
    assert_eq!(post.ray_cast(past_the_end, Vec3::NEG_X, 100.0), None);
}

#[test]
fn a_ray_meets_a_disc_on_its_face_and_on_its_rim() {
    let (radius, half) = (2.0f64, 0.1f64);
    for (margin, why) in [
        (half as f32, "its margin"),
        (0.0, "its box, with no margin"),
    ] {
        let plate = collider(&bounds::disc(
            Vec3::ZERO,
            radius as f32,
            half as f32,
            margin,
            0x74,
        ));

        let face = plate
            .ray_cast(Vec3::new(5.0, 0.5, 0.5), Vec3::NEG_X, 100.0)
            .unwrap_or_else(|| panic!("the face of a disc as thick as {why}"));
        assert_hit(&face, 5.0 - half, DVec3::new(half, 0.5, 0.5), DVec3::X);

        let rim = plate
            .ray_cast(Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z, 100.0)
            .unwrap_or_else(|| panic!("the rim of a disc as thick as {why}"));
        assert_hit(&rim, 10.0 - radius, DVec3::new(0.0, 0.0, radius), DVec3::Z);
        assert_eq!(rim.material, Some(0x74));

        let beside = Vec3::new(half as f32 + 0.05, 0.0, 10.0);
        assert_eq!(
            plate.ray_cast(beside, Vec3::NEG_Z, 100.0),
            None,
            "a disc as thick as {why} stops at {half} from its middle"
        );
    }
}

/// One geometry holding a triangle and each of the four primitives, a material to each polygon.
struct Mixed {
    g: PhBoundGeometry,
    box_half: [Vec3; 3],
}

const MATERIALS: [u64; 3] = [0x38, 0x74, 0x0400_007b];
const TRIANGLE: u32 = 0;
const SPHERE: u32 = 1;
const CAPSULE: u32 = 2;
const BOX: u32 = 3;
const CYLINDER: u32 = 4;
const PRIMITIVE_RADIUS: f32 = 0.5;

fn mixed() -> Mixed {
    let turn = Quat::from_rotation_z(30f32.to_radians());
    let box_half = [
        turn * (Vec3::X * 1.5),
        turn * (Vec3::Y * 0.5),
        Vec3::Z * 0.25,
    ];
    let corners = bounds::far_corners(Vec3::new(30.0, 0.0, 1.0), box_half);
    let positions = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(0.0, 4.0, 0.0),
        Vec3::new(10.0, 0.0, 1.0),
        Vec3::new(20.0, 0.0, 1.0),
        Vec3::new(20.0, 0.0, 3.0),
        corners[0],
        corners[1],
        corners[2],
        corners[3],
        Vec3::new(40.0, -1.0, 2.0),
        Vec3::new(40.0, 1.0, 2.0),
    ];
    let primitives = vec![
        bounds::sphere_primitive(3, PRIMITIVE_RADIUS),
        bounds::capsule_primitive([4, 5], PRIMITIVE_RADIUS),
        bounds::box_primitive([6, 7, 8, 9]),
        bounds::cylinder_primitive([10, 11], PRIMITIVE_RADIUS),
    ];
    Mixed {
        g: bounds::geometry(
            &positions,
            &[[0, 1, 2]],
            primitives,
            &MATERIALS,
            &[0, 1, 2, 0, 1],
        ),
        box_half,
    }
}

fn polygon(index: u32) -> Part {
    Part {
        leaf: 0,
        polygon: Some(index),
    }
}

#[test]
fn a_ray_meets_each_polygon_kind_of_a_geometry_with_its_material() {
    let Mixed { g, box_half } = mixed();
    let soup = collider(&PhBoundAny::PhBoundGeometry(g.clone()));
    assert_eq!(soup.shape_count(), 5);
    let down =
        |x: f64, y: f64| soup.ray_cast(Vec3::new(x as f32, y as f32, 10.0), Vec3::NEG_Z, 100.0);
    let radius = f64::from(PRIMITIVE_RADIUS);

    // The triangle lies at the bound's lowest, which sixteen bits hold to a rounding.
    let hit = down(1.0, 1.0).expect("the triangle");
    assert_hit(&hit, 10.0, DVec3::new(1.0, 1.0, 0.0), DVec3::Z);
    assert_eq!(
        (hit.part, hit.material),
        (polygon(TRIANGLE), Some(MATERIALS[0]))
    );
    assert_eq!(down(3.0, 3.0), None, "past the triangle's long edge");

    let centre = stored(&g, 3);
    let hit = down(centre.x + 0.3, centre.y).expect("the sphere primitive");
    let rise = (radius * radius - 0.09).sqrt();
    assert_hit(
        &hit,
        10.0 - (centre.z + rise),
        centre + DVec3::new(0.3, 0.0, rise),
        DVec3::new(0.3, 0.0, rise) / radius,
    );
    assert_eq!(
        (hit.part, hit.material),
        (polygon(SPHERE), Some(MATERIALS[1]))
    );

    // The capsule stands on end, so from above it is the ball about its upper end.
    let upper = stored(&g, 5);
    let hit = down(upper.x, upper.y + 0.2).expect("the capsule primitive");
    let rise = (radius * radius - 0.04).sqrt();
    assert_hit(
        &hit,
        10.0 - (upper.z + rise),
        upper + DVec3::new(0.0, 0.2, rise),
        DVec3::new(0.0, 0.2, rise) / radius,
    );
    assert_eq!(
        (hit.part, hit.material),
        (polygon(CAPSULE), Some(MATERIALS[2]))
    );

    // The box is turned 30 degrees about Z: its top is level, and its footprint is not its box's.
    // ⚠ Its four corners are sixteen bits an axis each, here 0.6 mm, so the box they make is
    // level to a millimetre and no better.
    let corners: Vec<DVec3> = (6..10).map(|v| stored(&g, v)).collect();
    let middle = corners.iter().sum::<DVec3>() / 4.0;
    let top = (corners[0].z + corners[3].z) * 0.5;
    let along = d(box_half[0]).normalize();
    let inside = middle + along * 1.3;
    let hit = down(inside.x, inside.y).expect("the box primitive, along its long edge");
    assert!(
        (f64::from(hit.distance) - (10.0 - top)).abs() < 2e-3,
        "met at {}",
        hit.distance
    );
    assert!(
        (d(hit.normal) - DVec3::Z).length() < 2e-3,
        "its top faces {}",
        hit.normal
    );
    assert_eq!((hit.part, hit.material), (polygon(BOX), Some(MATERIALS[0])));
    assert_eq!(
        down(middle.x + 1.3, middle.y),
        None,
        "1.3 along the world's X is off a box turned 30 degrees, and on one that is not"
    );

    // The cylinder lies along Y, so from above it is a round side with flat ends.
    let (a, b) = (stored(&g, 10), stored(&g, 11));
    let hit = down(a.x + 0.3, 0.0).expect("the cylinder primitive");
    let rise = (radius * radius - 0.09).sqrt();
    assert_hit(
        &hit,
        10.0 - (a.z + rise),
        DVec3::new(a.x + 0.3, 0.0, a.z + rise),
        DVec3::new(0.3, 0.0, rise) / radius,
    );
    assert_eq!(
        (hit.part, hit.material),
        (polygon(CYLINDER), Some(MATERIALS[1]))
    );
    assert_eq!(
        down(a.x, b.y + 0.05),
        None,
        "a cylinder ends flat where a capsule would round on"
    );
    let end = soup
        .ray_cast(
            Vec3::new(a.x as f32 + 0.2, 10.0, a.z as f32),
            Vec3::NEG_Y,
            100.0,
        )
        .expect("the cylinder's flat end");
    assert_hit(&end, 10.0 - b.y, DVec3::new(a.x + 0.2, b.y, a.z), DVec3::Y);
}

/// Each solid as a bound of its own, with the formula that says where it is.
fn solids() -> Vec<(&'static str, PhBoundAny, Solid)> {
    let Mixed { g, .. } = mixed();
    let centre = Vec3::new(1.0, -2.0, 0.5);
    let only = |polygon: usize| {
        let mut one = g.clone();
        one.polygons = vec![g.polygons[polygon].clone()];
        one.poly_mat_index_list = vec![g.poly_mat_index_list[polygon]];
        PhBoundAny::PhBoundGeometry(one)
    };
    let radius = f64::from(PRIMITIVE_RADIUS);
    vec![
        (
            "a sphere bound",
            bounds::sphere(centre, 1.25, 0),
            Solid::Sphere {
                centre: d(centre),
                radius: 1.25,
            },
        ),
        (
            "a capsule bound",
            bounds::capsule(centre, 0.5, 1.5, 0),
            Solid::Capsule {
                a: d(centre) - DVec3::Y * 1.5,
                b: d(centre) + DVec3::Y * 1.5,
                radius: 0.5,
            },
        ),
        (
            "a box bound",
            bounds::boxed(
                centre - Vec3::new(1.0, 0.5, 2.0),
                centre + Vec3::new(1.0, 0.5, 2.0),
                0,
            ),
            Solid::Box {
                centre: d(centre),
                half: DVec3::new(1.0, 0.5, 2.0),
            },
        ),
        (
            "a cylinder bound",
            bounds::cylinder(centre, 0.75, 2.0, 0),
            Solid::Cylinder {
                centre: d(centre),
                axis: DVec3::Y,
                half_length: 2.0,
                radius: 0.75,
            },
        ),
        (
            "a disc bound",
            bounds::disc(centre, 2.0, 0.25, 0.25, 0),
            Solid::Cylinder {
                centre: d(centre),
                axis: DVec3::X,
                half_length: 0.25,
                radius: 2.0,
            },
        ),
        (
            "a sphere primitive",
            only(SPHERE as usize),
            Solid::Sphere {
                centre: stored(&g, 3),
                radius,
            },
        ),
        (
            "a capsule primitive",
            only(CAPSULE as usize),
            Solid::Capsule {
                a: stored(&g, 4),
                b: stored(&g, 5),
                radius,
            },
        ),
        (
            "a cylinder primitive",
            only(CYLINDER as usize),
            Solid::Cylinder {
                centre: (stored(&g, 10) + stored(&g, 11)) * 0.5,
                axis: (stored(&g, 11) - stored(&g, 10)).normalize(),
                half_length: (stored(&g, 11) - stored(&g, 10)).length() * 0.5,
                radius,
            },
        ),
    ]
}

/// The middle of a solid and a length that reaches across it.
fn middle_and_size(solid: &Solid) -> (DVec3, f64) {
    match *solid {
        Solid::Sphere { centre, radius } => (centre, radius),
        Solid::Capsule { a, b, radius } => ((a + b) * 0.5, (b - a).length() * 0.5 + radius),
        Solid::Box { centre, half } => (centre, half.length()),
        Solid::Cylinder {
            centre,
            half_length,
            radius,
            ..
        } => (centre, half_length.hypot(radius)),
    }
}

/// Paths from all round a solid, each aimed at a point inside it or well beside it.
fn paths(solid: &Solid, dice: &mut Dice) -> Vec<(Vec3, Vec3, f32)> {
    let (middle, size) = middle_and_size(solid);
    (0..60)
        .map(|i| {
            let from = middle + d(dice.point(-1.0, 1.0)).normalize() * size * 4.0;
            // Two in three at the solid's inside, one in three past it.
            let aim = if i % 3 == 2 { 1.6 } else { 0.35 };
            let target = middle + d(dice.point(-1.0, 1.0)).normalize() * size * aim;
            let (from, target) = (from.as_vec3(), target.as_vec3());
            (from, (target - from).normalize(), (size * 9.0) as f32)
        })
        .collect()
}

/// What the formula and the collider may differ by: single precision, and the formula's own halving.
const AGREE: f64 = 2e-4;

#[test]
fn a_ray_meets_every_solid_where_a_distance_formula_says_it_starts() {
    let mut dice = Dice(0x5EED);
    let (mut hits, mut misses, mut grazed) = (0usize, 0usize, 0usize);
    for (name, bound, solid) in solids() {
        let shape = collider(&bound);
        for (origin, direction, max) in paths(&solid, &mut dice) {
            let want = first_within(&solid, 0.0, d(origin), d(direction), max.into());
            let got = shape.ray_cast(origin, direction, max);
            match (want, got) {
                (Some(want), Some(got)) => {
                    hits += 1;
                    assert!(
                        (f64::from(got.distance) - want).abs() <= AGREE,
                        "{name}: met at {} where the formula says {want}, from {origin} along {direction}",
                        got.distance
                    );
                    // The normal there leaves the solid: a step along it is outside.
                    let out = d(got.position) + d(got.normal) * 1e-2;
                    assert!(solid.distance(out) > 5e-3, "{name}: the normal points in");
                    assert!(
                        got.normal.dot(direction) <= 0.0,
                        "{name}: the normal faces away"
                    );
                }
                (None, None) => misses += 1,
                (want, got) => {
                    // A path may graze: the formula's steps skip a crossing shorter than one.
                    let grazes = (0..20_000).any(|i| {
                        let t = f64::from(max) * f64::from(i) / 20_000.0;
                        solid.distance(d(origin) + d(direction) * t).abs() < 2e-3
                    });
                    assert!(grazes, "{name}: {got:?} where the formula says {want:?}");
                    grazed += 1;
                }
            }
        }
    }
    assert!(hits > 200 && misses > 60, "{hits} hits and {misses} misses");
    assert!(
        grazed * 50 <= hits,
        "{grazed} paths of {hits} were let off as grazing"
    );
}

#[test]
fn a_swept_sphere_touches_every_solid_where_a_distance_formula_says_it_first_comes_within_its_radius(
) {
    let mut dice = Dice(0xBA11);
    let (mut hits, mut misses, mut grazed) = (0usize, 0usize, 0usize);
    for (name, bound, solid) in solids() {
        let shape = collider(&bound);
        for (start, direction, max) in paths(&solid, &mut dice) {
            let radius = dice.between(0.05, 0.6);
            let want = first_within(&solid, radius.into(), d(start), d(direction), max.into());
            let got = shape.sweep_sphere(start, direction, max, radius);
            match (want, got) {
                (Some(want), Some(got)) => {
                    hits += 1;
                    assert!(
                        (f64::from(got.distance) - want).abs() <= AGREE,
                        "{name}: touched at {} where the formula says {want}, radius {radius} from {start} along {direction}",
                        got.distance
                    );
                    // The point touched is on the solid, a radius from where the sphere then is.
                    assert!(
                        solid.distance(d(got.position)).abs() <= AGREE,
                        "{name}: the point touched is {} off the solid",
                        solid.distance(d(got.position))
                    );
                    let centre = d(start) + d(direction) * f64::from(got.distance);
                    let from_surface = centre - d(got.position);
                    assert!((from_surface.length() - f64::from(radius)).abs() <= AGREE);
                    assert!(
                        (from_surface.normalize() - d(got.normal)).length() <= 1e-3,
                        "{name}: the normal is not from the point touched to the sphere"
                    );
                }
                (None, None) => misses += 1,
                (want, got) => {
                    let grazes = (0..20_000).any(|i| {
                        let t = f64::from(max) * f64::from(i) / 20_000.0;
                        let gap = solid.distance(d(start) + d(direction) * t) - f64::from(radius);
                        gap.abs() < 2e-3
                    });
                    assert!(grazes, "{name}: {got:?} where the formula says {want:?}");
                    grazed += 1;
                }
            }
        }
    }
    assert!(hits > 200 && misses > 30, "{hits} hits and {misses} misses");
    assert!(
        grazed * 50 <= hits,
        "{grazed} paths of {hits} were let off as grazing"
    );
}

#[test]
fn a_swept_sphere_touches_a_triangle_on_its_face_its_edge_and_its_corner() {
    let positions = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(0.0, 4.0, 0.0),
    ];
    let g = bounds::geometry(&positions, &[[0, 1, 2]], Vec::new(), &[0x99], &[0]);
    let held = [0, 1, 2].map(|v| stored(&g, v));
    let floor = collider(&PhBoundAny::PhBoundGeometry(g));
    let radius = 0.5f64;
    let drop = |x: f64, y: f64| {
        floor
            .sweep_sphere(
                Vec3::new(x as f32, y as f32, 5.0),
                Vec3::NEG_Z,
                100.0,
                radius as f32,
            )
            .unwrap_or_else(|| panic!("a sphere dropped at {x}, {y} lands"))
    };

    let face = drop(1.0, 1.0);
    assert_hit(&face, 5.0 - radius, DVec3::new(1.0, 1.0, 0.0), DVec3::Z);
    assert_eq!(face.material, Some(0x99));

    // 0.3 outside the middle of each edge in turn: it rests on the edge, lower than on the face.
    let out = 0.3f64;
    let rise = (radius * radius - out * out).sqrt();
    for i in 0..3 {
        let (from, to, third) = (held[i], held[(i + 1) % 3], held[(i + 2) % 3]);
        let along = (to - from).normalize();
        let inward = third - from;
        let outward = -(inward - along * inward.dot(along)).normalize();
        let on = (from + to) * 0.5;
        let over = on + outward * out;
        assert_hit(
            &drop(over.x, over.y),
            5.0 - (on.z + rise),
            on,
            (outward * out + DVec3::Z * rise) / radius,
        );
    }

    // 0.3 past each corner in turn, along the line that halves its angle, which is outside
    // both edges that meet there: it rests on the corner.
    for i in 0..3 {
        let (corner, one, other) = (held[i], held[(i + 1) % 3], held[(i + 2) % 3]);
        let past = -((one - corner).normalize() + (other - corner).normalize()).normalize();
        let over = corner + past * out;
        assert_hit(
            &drop(over.x, over.y),
            5.0 - (corner.z + rise),
            corner,
            (past * out + DVec3::Z * rise) / radius,
        );
    }

    let beside = Vec3::new(2.0, -(radius as f32) - 0.01, 5.0);
    assert_eq!(
        floor.sweep_sphere(beside, Vec3::NEG_Z, 100.0, radius as f32),
        None,
        "a sphere a radius and a hundredth outside the edge falls past it"
    );
    let just = Vec3::new(2.0, -(radius as f32) + 0.01, 5.0);
    assert!(floor
        .sweep_sphere(just, Vec3::NEG_Z, 100.0, radius as f32)
        .is_some());
    // From below it meets the same face, whose normal then faces down.
    let under = floor
        .sweep_sphere(Vec3::new(1.0, 1.0, -5.0), Vec3::Z, 100.0, radius as f32)
        .expect("from below");
    assert_hit(
        &under,
        5.0 - radius,
        DVec3::new(1.0, 1.0, 0.0),
        DVec3::NEG_Z,
    );
}

#[test]
fn a_swept_sphere_that_starts_touching_has_moved_no_distance() {
    let crate_ = collider(&bounds::boxed(Vec3::splat(-1.0), Vec3::splat(1.0), 3));
    // Its centre is 0.2 above the top face and its radius 0.5.
    let start = Vec3::new(0.0, 0.0, 1.2);
    let hit = crate_
        .sweep_sphere(start, Vec3::X, 10.0, 0.5)
        .expect("it starts touching");
    assert_hit(&hit, 0.0, DVec3::new(0.0, 0.0, 1.0), DVec3::Z);
    // Deep in the box no surface is nearest, so the normal turns the sphere back.
    let deep = crate_
        .sweep_sphere(Vec3::ZERO, Vec3::X, 10.0, 0.5)
        .expect("it starts inside");
    assert_eq!(deep.distance, 0.0);
    assert_eq!(deep.normal, Vec3::NEG_X);
    // Clear of it by a hundredth, and moving along the face, it touches nothing.
    let clear = Vec3::new(0.0, 0.0, 1.51);
    assert_eq!(crate_.sweep_sphere(clear, Vec3::X, 10.0, 0.5), None);
}

#[test]
fn a_swept_sphere_of_no_radius_is_a_ray_and_one_of_no_number_touches_nothing() {
    let Mixed { g, .. } = mixed();
    let soup = collider(&PhBoundAny::PhBoundGeometry(g));
    let mut dice = Dice(7);
    let mut met = 0;
    for i in 0..200 {
        // Above one of the five polygons in turn, and a little to one side.
        let over = [1.0, 10.0, 20.0, 30.0, 40.0][i % 5];
        let origin = Vec3::new(over + dice.between(-1.0, 1.0), dice.between(-1.0, 1.5), 8.0);
        let direction = Vec3::new(dice.between(-0.1, 0.1), dice.between(-0.1, 0.1), -1.0);
        let ray = soup.ray_cast(origin, direction, 50.0);
        assert_eq!(soup.sweep_sphere(origin, direction, 50.0, 0.0), ray);
        assert_eq!(soup.sweep_sphere(origin, direction, 50.0, -1.0), ray);
        assert_eq!(soup.sweep_sphere(origin, direction, 50.0, f32::NAN), None);
        assert_eq!(
            soup.sweep_sphere(origin, direction, 50.0, f32::INFINITY),
            None
        );
        met += usize::from(ray.is_some());
    }
    assert!(met > 20, "only {met} of the rays met anything");
}

#[test]
fn a_ray_that_starts_inside_a_solid_meets_it_at_once_and_inside_a_mesh_meets_its_far_wall() {
    for (name, bound, solid) in solids() {
        let shape = collider(&bound);
        let (middle, _) = middle_and_size(&solid);
        // Off its middle, where no direction is special, and well inside.
        let within = middle + DVec3::new(0.1, -0.08, 0.05);
        assert!(
            solid.distance(within) < -0.05,
            "{name}: the start is inside"
        );
        for direction in [Vec3::new(0.3, -0.5, 0.8), Vec3::new(-0.3, 0.5, -0.8)] {
            let direction = direction.normalize();
            let hit = shape
                .ray_cast(within.as_vec3(), direction, 100.0)
                .unwrap_or_else(|| panic!("{name}: a ray from inside it"));
            assert_eq!(hit.distance, 0.0, "{name}");
            assert_eq!(hit.position, within.as_vec3(), "{name}");
            assert!(
                (hit.normal + direction).length() < 1e-6,
                "{name}: it is turned back"
            );
        }
    }

    // A triangle has no inside: from within a closed mesh a ray reaches the wall ahead of it.
    let (low, high) = (Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 3.0));
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { low.x } else { high.x },
            if i & 2 == 0 { low.y } else { high.y },
            if i & 4 == 0 { low.z } else { high.z },
        )
    };
    let positions: Vec<Vec3> = (0..8).map(corner).collect();
    let faces: [[u16; 3]; 12] = [
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    let room = collider(&PhBoundAny::PhBoundGeometry(bounds::triangles(
        &positions, &faces,
    )));
    let from = Vec3::new(0.25, -0.5, 0.0);
    let up = room.ray_cast(from, Vec3::Z, 100.0).expect("the ceiling");
    assert_hit(
        &up,
        f64::from(high.z - from.z),
        DVec3::new(0.25, -0.5, high.z.into()),
        DVec3::NEG_Z,
    );
    let side = room.ray_cast(from, Vec3::NEG_Y, 100.0).expect("the wall");
    assert_hit(
        &side,
        f64::from(from.y - low.y),
        DVec3::new(0.25, low.y.into(), 0.0),
        DVec3::Y,
    );
}

/// ⚠ The tree turns away a ray that leaves a shape's box behind, so what the shape itself must
/// turn away starts inside that box and outside the shape, with the shape behind it.
#[test]
fn a_ray_that_runs_away_from_a_solid_meets_nothing() {
    let mut tried = std::collections::BTreeSet::new();
    for (name, bound, solid) in solids() {
        let shape = collider(&bound);
        let (middle, size) = middle_and_size(&solid);
        let boxed = shape.local_box();
        let far = (size * 9.0) as f32;
        for corner in 0..8 {
            let corner = Vec3::new(
                if corner & 1 == 0 {
                    boxed.min.x
                } else {
                    boxed.max.x
                },
                if corner & 2 == 0 {
                    boxed.min.y
                } else {
                    boxed.max.y
                },
                if corner & 4 == 0 {
                    boxed.min.z
                } else {
                    boxed.max.z
                },
            );
            // Nine tenths of the way from the solid's middle to a corner of its box.
            let origin = middle.as_vec3().lerp(corner, 0.9);
            if solid.distance(d(origin)) < 0.02 * size {
                // A box fills its box: no corner of it is outside the solid.
                continue;
            }
            tried.insert(name);
            let away = (origin - middle.as_vec3()).normalize();
            assert!(
                shape.ray_cast(origin, -away, far).is_some(),
                "{name}: the line from {origin} along {away} goes through it"
            );
            for max in [far, f32::INFINITY] {
                assert_eq!(
                    shape.ray_cast(origin, away, max),
                    None,
                    "{name}: a ray from {origin} along {away} leaves it behind"
                );
            }
        }
    }
    let round: std::collections::BTreeSet<&str> = solids()
        .iter()
        .map(|(name, ..)| *name)
        .filter(|name| !name.contains("box"))
        .collect();
    assert_eq!(
        tried, round,
        "every solid that does not fill its box was tried"
    );

    // A tilted triangle, from just off its face and inside its box.
    let tilted = [
        Vec3::new(0.3, -1.2, 0.4),
        Vec3::new(3.1, 0.7, 1.9),
        Vec3::new(-0.8, 2.6, 2.2),
    ];
    let g = bounds::triangles(&tilted, &[[0, 1, 2]]);
    let [a, b, c] = [0, 1, 2].map(|v| stored(&g, v).as_vec3());
    let slope = collider(&PhBoundAny::PhBoundGeometry(g));
    let off = (b - a).cross(c - a).normalize();
    let origin = (a + b + c) / 3.0 + off * 0.1;
    let boxed = slope.local_box();
    assert!(
        origin.cmpgt(boxed.min).all() && origin.cmplt(boxed.max).all(),
        "the start is inside the triangle's box"
    );
    assert!(slope.ray_cast(origin, -off, 100.0).is_some());
    assert_eq!(slope.ray_cast(origin, off, f32::INFINITY), None);
}

/// ⚠ A triangle's corner is on the faces of the triangle's box, and the tree tests the box
/// with other arithmetic than the triangle: a box held to its last bit turns such a ray away.
#[test]
fn a_slanted_ray_at_a_triangles_corner_meets_it() {
    let mut dice = Dice(0xC0A);
    let mut rays = 0;
    for _ in 0..300 {
        let middle = dice.point(-50.0, 50.0);
        let positions = [0; 3].map(|_| middle + dice.point(-3.0, 3.0));
        let g = bounds::triangles(&positions, &[[0, 1, 2]]);
        let held = [0, 1, 2].map(|v| stored(&g, v).as_vec3());
        let sheet = collider(&PhBoundAny::PhBoundGeometry(g));
        let normal = (held[1] - held[0]).cross(held[2] - held[0]).normalize();
        for corner in held {
            for _ in 0..20 {
                let from = corner + normal * dice.between(2.0, 6.0) + dice.point(-3.0, 3.0);
                let towards = corner - from;
                // Well off the triangle's plane, where a ray is seen edge on.
                if towards.normalize().dot(normal).abs() < 0.2 {
                    continue;
                }
                rays += 1;
                let hit = sheet
                    .ray_cast(from, towards, 100.0)
                    .unwrap_or_else(|| panic!("a ray from {from} at the corner {corner}"));
                assert!(
                    (hit.position - corner).length() < 1e-3,
                    "met at {} for {corner}",
                    hit.position
                );
            }
        }
    }
    assert!(rays > 10_000, "{rays} rays were cast");
}

#[test]
fn a_grazing_ray_meets_what_it_touches_and_misses_what_it_passes() {
    // A tangent to a sphere, in numbers single precision holds exactly.
    let ball = collider(&bounds::sphere(Vec3::ZERO, 1.0, 0));
    let tangent = ball
        .ray_cast(Vec3::new(-5.0, 1.0, 0.0), Vec3::X, 100.0)
        .expect("a tangent touches");
    assert_hit(&tangent, 5.0, DVec3::Y, DVec3::Y);
    let past = Vec3::new(-5.0, 1.001, 0.0);
    assert_eq!(ball.ray_cast(past, Vec3::X, 100.0), None);

    // Down through a triangle's corner, along its edge's line, and through its edge.
    let positions = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(0.0, 4.0, 0.0),
    ];
    let sheet = collider(&PhBoundAny::PhBoundGeometry(bounds::triangles(
        &positions,
        &[[0, 1, 2]],
    )));
    for (x, y, what) in [
        (0.0, 0.0, "a corner"),
        (2.0, 0.0, "an edge"),
        (2.0, 2.0, "the long edge"),
    ] {
        let hit = sheet
            .ray_cast(Vec3::new(x, y, 3.0), Vec3::NEG_Z, 100.0)
            .unwrap_or_else(|| panic!("a ray through {what} touches the triangle"));
        assert_hit(&hit, 3.0, DVec3::new(x.into(), y.into(), 0.0), DVec3::Z);
    }
    for (x, y) in [(-0.001, 0.0), (2.0, -0.001), (2.001, 2.001)] {
        assert_eq!(
            sheet.ray_cast(Vec3::new(x, y, 3.0), Vec3::NEG_Z, 100.0),
            None,
            "a thousandth outside at {x}, {y}"
        );
    }

    // Along a box's edge, which is on two of its faces at once.
    let crate_ = collider(&bounds::boxed(Vec3::splat(-1.0), Vec3::splat(1.0), 0));
    let along = crate_
        .ray_cast(Vec3::new(-5.0, 1.0, 1.0), Vec3::X, 100.0)
        .expect("a ray along an edge touches the box");
    assert_eq!(along.distance, 4.0);
    assert_eq!(along.position, Vec3::new(-1.0, 1.0, 1.0));
    assert_eq!(along.normal, Vec3::NEG_X);
    let clear = Vec3::new(-5.0, 1.001, 1.0);
    assert_eq!(crate_.ray_cast(clear, Vec3::X, 100.0), None);
}

#[test]
fn a_ray_parallel_to_a_face_meets_only_what_lies_in_its_way() {
    let positions = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(0.0, 4.0, 0.0),
    ];
    let sheet = collider(&PhBoundAny::PhBoundGeometry(bounds::triangles(
        &positions,
        &[[0, 1, 2]],
    )));
    // In the triangle's own plane, and just above it: a face seen edge on is no surface.
    for z in [0.0, 0.5] {
        assert_eq!(
            sheet.ray_cast(Vec3::new(-5.0, 1.0, z), Vec3::X, 100.0),
            None,
            "a ray along the plane at z = {z}"
        );
    }

    // A tilted triangle, and rays from a point of its plane beside it to a point of its plane
    // on it: single precision leaves each a hair off the plane, and it is still seen edge on.
    let tilted = [
        Vec3::new(0.3, -1.2, 0.4),
        Vec3::new(3.1, 0.7, 1.9),
        Vec3::new(-0.8, 2.6, 2.2),
    ];
    let g = bounds::triangles(&tilted, &[[0, 1, 2]]);
    let [a, b, c] = [0, 1, 2].map(|v| stored(&g, v).as_vec3());
    let slope = collider(&PhBoundAny::PhBoundGeometry(g));
    let mut dice = Dice(0x51DE);
    for _ in 0..200 {
        let beside = a + (b - a) * dice.between(-2.0, -0.5) + (c - a) * dice.between(-1.0, 2.0);
        let on = a + (b - a) * dice.between(0.1, 0.4) + (c - a) * dice.between(0.1, 0.4);
        assert_eq!(
            slope.ray_cast(beside, on - beside, 100.0),
            None,
            "a ray in the plane, from {beside} to {on}"
        );
    }
    // The same triangle from off its plane is there to be met.
    let above = (a + b + c) / 3.0 + (b - a).cross(c - a).normalize() * 2.0;
    let face = slope
        .ray_cast(above, (a + b + c) / 3.0 - above, 100.0)
        .expect("its face");
    assert!((face.distance - 2.0).abs() < 1e-4, "{}", face.distance);

    let crate_ = collider(&bounds::boxed(Vec3::splat(-1.0), Vec3::splat(1.0), 0));
    // Above the top face and level with it.
    let above = Vec3::new(-5.0, 0.0, 1.5);
    assert_eq!(crate_.ray_cast(above, Vec3::X, 100.0), None);
    // Level with the top face and in its plane: the box is closed, so the ray meets its side.
    let level = crate_
        .ray_cast(Vec3::new(-5.0, 0.0, 1.0), Vec3::X, 100.0)
        .expect("a ray in the plane of the top face");
    assert_eq!((level.distance, level.normal), (4.0, Vec3::NEG_X));
    // Between two faces and level with both.
    let through = crate_
        .ray_cast(Vec3::new(-5.0, 0.25, -0.5), Vec3::X, 100.0)
        .expect("a ray between the faces");
    assert_eq!((through.distance, through.normal), (4.0, Vec3::NEG_X));

    // Along a cylinder's axis, beside it and through its flat end.
    let post = collider(&bounds::cylinder(Vec3::ZERO, 1.0, 2.0, 0));
    let beside = Vec3::new(1.5, 10.0, 0.0);
    assert_eq!(post.ray_cast(beside, Vec3::NEG_Y, 100.0), None);
    // ⚠ Beside its round and in the corner of its box, where the box does not say it is missed.
    let cornered = Vec3::new(0.8, 10.0, 0.8);
    assert_eq!(post.ray_cast(cornered, Vec3::NEG_Y, 100.0), None);
    let end = post
        .ray_cast(Vec3::new(0.5, 10.0, 0.0), Vec3::NEG_Y, 100.0)
        .expect("down the axis");
    assert_eq!((end.distance, end.normal), (8.0, Vec3::Y));
    // Across it and level with its flat end, a hundredth above and in the end's plane.
    let over = Vec3::new(-5.0, 2.01, 0.0);
    assert_eq!(post.ray_cast(over, Vec3::X, 100.0), None);
    let on = post
        .ray_cast(Vec3::new(-5.0, 2.0, 0.0), Vec3::X, 100.0)
        .expect("in the plane of the flat end");
    assert_eq!(on.distance, 4.0);
}

#[test]
fn a_ray_of_no_length_meets_only_what_it_starts_in() {
    let ball = collider(&bounds::sphere(Vec3::ZERO, 1.0, 5));
    let (outside, inside) = (Vec3::new(3.0, 0.0, 0.0), Vec3::new(0.25, 0.0, 0.0));

    // No direction: there is no ray.
    for origin in [outside, inside] {
        assert_eq!(ball.ray_cast(origin, Vec3::ZERO, 100.0), None);
        assert_eq!(ball.sweep_sphere(origin, Vec3::ZERO, 100.0, 0.5), None);
        assert_eq!(ball.ray_cast(origin, Vec3::splat(f32::NAN), 100.0), None);
    }
    // No length: only where it stands.
    assert_eq!(ball.ray_cast(outside, Vec3::NEG_X, 0.0), None);
    let within = ball
        .ray_cast(inside, Vec3::NEG_X, 0.0)
        .expect("it stands in the sphere");
    assert_eq!(
        (within.distance, within.position, within.material),
        (0.0, inside, Some(5))
    );
    // A sphere not moved touches what it already overlaps: 1.4 from the centre, where radii of 1 and 0.5 reach 1.5.
    let near = Vec3::new(1.4, 0.0, 0.0);
    assert!(ball.sweep_sphere(near, Vec3::NEG_X, 0.0, 0.5).is_some());
    // A length that is no length, and a start that is no place.
    for max in [-1.0, f32::NAN] {
        assert_eq!(ball.ray_cast(outside, Vec3::NEG_X, max), None);
        assert_eq!(ball.ray_cast(inside, Vec3::NEG_X, max), None);
        assert_eq!(ball.sweep_sphere(outside, Vec3::NEG_X, max, 0.5), None);
        assert_eq!(ball.sweep_sphere(near, Vec3::NEG_X, max, 0.5), None);
    }
    let nowhere = Vec3::new(f32::NAN, 0.0, 0.0);
    assert_eq!(ball.ray_cast(nowhere, Vec3::NEG_X, 100.0), None);
    assert_eq!(ball.sweep_sphere(nowhere, Vec3::NEG_X, 100.0, 0.5), None);
    assert_eq!(ball.sweep_sphere(outside, Vec3::NEG_X, 0.0, 0.5), None);
    // And a ray to just short of the sphere stops short.
    assert_eq!(ball.ray_cast(outside, Vec3::NEG_X, 1.999), None);
    assert!(ball.ray_cast(outside, Vec3::NEG_X, 2.0).is_some());
}

#[test]
fn a_composite_places_each_child_by_its_own_matrix() {
    let slab = |material| {
        bounds::boxed(
            Vec3::new(-1.0, -0.5, -0.25),
            Vec3::new(1.0, 0.5, 0.25),
            material,
        )
    };
    let quarter_turn = Affine3A::from_rotation_translation(
        Quat::from_rotation_z(90f32.to_radians()),
        Vec3::new(10.0, 0.0, 2.0),
    );
    let aside = Affine3A::from_translation(Vec3::new(-10.0, 0.0, 0.0));
    let no_matrix = Affine3A::from_cols_array(&[f32::NAN; 12]);
    let built = bounds::composite(vec![
        (Some(slab(1)), quarter_turn),
        (None, no_matrix),
        (Some(bounds::sphere(Vec3::ZERO, 1.0, 2)), aside),
    ]);
    let both = collider(&built);
    assert_eq!(both.shape_count(), 2);
    let down = |x: f32, y: f32| both.ray_cast(Vec3::new(x, y, 20.0), Vec3::NEG_Z, 100.0);

    // The slab's long edge now runs along Y: its corners, turned and moved, say where its top is.
    let corners: Vec<Vec3> = [-1.0f32, 1.0]
        .iter()
        .flat_map(|x| [-0.5f32, 0.5].map(|y| quarter_turn.transform_point3(Vec3::new(*x, y, 0.25))))
        .collect();
    let (low, high) = corners.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(low, high), c| (low.min(*c), high.max(*c)),
    );
    assert!(
        (high.y - low.y) > (high.x - low.x),
        "the turn swapped its edges"
    );
    let along = Vec3::new((low.x + high.x) * 0.5, high.y - 0.2, 0.0);
    let hit = down(along.x, along.y).expect("the turned slab, towards the end of its long edge");
    assert_hit(
        &hit,
        f64::from(20.0 - high.z),
        DVec3::new(along.x.into(), along.y.into(), high.z.into()),
        DVec3::Z,
    );
    assert_eq!((hit.part, hit.material), (WHOLE, Some(1)));
    assert_eq!(
        down(high.x + 0.3, 0.0),
        None,
        "past the short edge of the turned slab, which an unturned one would cover"
    );
    assert_eq!(
        down(0.0, 0.0),
        None,
        "where the slab was before it was moved"
    );

    // The empty slot is no bound, so the sphere is the second of them.
    let ball = down(-10.0, 0.0).expect("the sphere, moved aside");
    assert_hit(&ball, 19.0, DVec3::new(-10.0, 0.0, 1.0), DVec3::Z);
    assert_eq!(
        (ball.part, ball.material),
        (
            Part {
                leaf: 1,
                polygon: None
            },
            Some(2)
        )
    );

    // A composite in a composite: the matrices multiply, the outer one last.
    let lifted = Affine3A::from_rotation_translation(
        Quat::from_rotation_x(90f32.to_radians()),
        Vec3::new(0.0, 0.0, 50.0),
    );
    let nested = collider(&bounds::composite(vec![(Some(built), lifted)]));
    // The quarter turn about X stands the slab's underside up to face along Y.
    let want = lifted.transform_point3(quarter_turn.transform_point3(Vec3::new(0.0, 0.0, -0.25)));
    let from = want + Vec3::Y * 5.0;
    let hit = nested
        .ray_cast(from, Vec3::NEG_Y, 100.0)
        .expect("the slab, placed twice");
    assert!(
        (hit.position - want).length() < 1e-4 && (hit.distance - 5.0).abs() < 1e-4,
        "met at {} for {want}",
        hit.position
    );
    assert!((hit.normal - Vec3::Y).length() < 1e-4);
}

/// ⚠ A composite's child is moved into the composite's space once, when the collider is built,
/// and a bound placed in a set is left where it is and the query moved to it: two routes to
/// one answer, and here the matrix turns, moves and doubles every kind of polygon.
#[test]
fn a_composites_child_is_where_a_set_places_the_same_bound() {
    let Mixed { g, .. } = mixed();
    let place = Affine3A::from_scale_rotation_translation(
        Vec3::splat(2.0),
        Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.7, 1.1),
        Vec3::new(-30.0, 12.0, 5.0),
    );
    let child = PhBoundAny::PhBoundGeometry(g);
    let baked = collider(&bounds::composite(vec![(Some(child.clone()), place)]));
    let mut set = CollisionSet::new();
    set.add((), std::sync::Arc::new(collider(&child)), place)
        .expect("a placement");

    let mut dice = Dice(0xC0);
    let (mut rays, mut sweeps, mut edge) = (0, 0, 0);
    let mut kinds = std::collections::BTreeSet::new();
    for i in 0..400 {
        // At one of the five polygons in turn, from a few metres off it.
        let over = [1.0, 10.0, 20.0, 30.0, 40.0][i % 5];
        let target = place.transform_point3(Vec3::new(over, 0.3, 1.0) + dice.point(-0.6, 0.6));
        let origin = target + dice.point(-1.0, 1.0).normalize() * 12.0;
        let direction = target - origin;
        let radius = dice.between(0.1, 0.5);

        let queries = [
            (
                baked.ray_cast(origin, direction, 40.0),
                set.ray_cast(origin, direction, 40.0),
            ),
            (
                baked.sweep_sphere(origin, direction, 40.0, radius),
                set.sweep_sphere(origin, direction, 40.0, radius),
            ),
        ];
        for (swept, (baked, placed)) in queries.into_iter().enumerate() {
            let placed = placed.map(|(_, hit)| hit);
            let (Some(baked), Some(placed)) = (baked, placed) else {
                // A path along the very edge of a polygon may fall either side on each route.
                edge += usize::from(baked.is_some() != placed.is_some());
                continue;
            };
            assert!(
                (baked.distance - placed.distance).abs() < 2e-3,
                "{} baked and {} placed, from {origin} along {direction}",
                baked.distance,
                placed.distance
            );
            assert!((baked.position - placed.position).length() < 2e-3);
            assert!((baked.normal - placed.normal).length() < 2e-3);
            assert_eq!((baked.part, baked.material), (placed.part, placed.material));
            kinds.insert(baked.part.polygon);
            if swept == 0 {
                rays += 1;
            } else {
                sweeps += 1;
            }
        }
    }
    assert_eq!(kinds.len(), 5, "every polygon was met: {kinds:?}");
    assert!(
        rays > 150 && sweeps > 200,
        "{rays} rays and {sweeps} sweeps met something"
    );
    assert!(
        edge <= 2,
        "{edge} paths met something by one route and nothing by the other"
    );
}

/// Many triangles and primitives scattered in a block, enough for a tree several levels deep.
fn scatter(dice: &mut Dice) -> PhBoundGeometry {
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    let mut primitives = Vec::new();
    let next = |positions: &mut Vec<Vec3>, p: Vec3| {
        positions.push(p);
        (positions.len() - 1) as u16
    };
    for _ in 0..300 {
        let at = dice.point(-8.0, 8.0);
        let corners = [0; 3].map(|_| next(&mut positions, at + dice.point(-1.5, 1.5)));
        triangles.push(corners);
    }
    for i in 0..60 {
        let at = dice.point(-8.0, 8.0);
        let radius = dice.between(0.2, 0.8);
        let other = at + dice.point(-1.5, 1.5);
        primitives.push(match i % 4 {
            0 => bounds::sphere_primitive(next(&mut positions, at), radius),
            1 => bounds::capsule_primitive(
                [next(&mut positions, at), next(&mut positions, other)],
                radius,
            ),
            2 => bounds::cylinder_primitive(
                [next(&mut positions, at), next(&mut positions, other)],
                radius,
            ),
            _ => {
                let turn = Quat::from_euler(
                    glam::EulerRot::XYZ,
                    dice.between(0.0, 3.0),
                    dice.between(0.0, 3.0),
                    dice.between(0.0, 3.0),
                );
                let half = [
                    turn * Vec3::X * radius,
                    turn * Vec3::Y,
                    turn * Vec3::Z * 0.4,
                ];
                bounds::box_primitive(
                    bounds::far_corners(at, half).map(|c| next(&mut positions, c)),
                )
            }
        });
    }
    bounds::geometry(&positions, &triangles, primitives, &[], &[])
}

/// ⚠ The tree only chooses what is tested, so a bound's answer is the best of its polygons'
/// answers one at a time. A polygon alone is the same bound with the others taken out, so its
/// vertices stay where sixteen bits put them.
#[test]
fn the_tree_gives_the_answer_of_asking_every_polygon_in_turn() {
    let mut dice = Dice(0x7EE);
    let g = scatter(&mut dice);
    let whole = collider(&PhBoundAny::PhBoundGeometry(g.clone()));
    assert_eq!(whole.shape_count(), 360);
    let alone: Vec<BoundCollider> = g
        .polygons
        .iter()
        .map(|p| {
            let mut one = g.clone();
            one.polygons = vec![p.clone()];
            collider(&PhBoundAny::PhBoundGeometry(one))
        })
        .collect();
    let renamed = |hit: Hit, polygon: usize| Hit {
        part: Part {
            leaf: 0,
            polygon: Some(polygon as u32),
        },
        ..hit
    };
    // Where several polygons are met at one distance, as a sphere that starts against two is, the
    // tree may name any of them.
    let agrees = |got: Option<Hit>, found: Vec<Hit>| -> bool {
        let Some(least) = found.iter().map(|hit| hit.distance).min_by(f32::total_cmp) else {
            return got.is_none();
        };
        found
            .iter()
            .filter(|hit| hit.distance == least)
            .any(|hit| Some(*hit) == got)
    };
    let (mut rays, mut sweeps) = (0, 0);
    for _ in 0..300 {
        let origin = dice.point(-11.0, 11.0);
        let direction = dice.point(-1.0, 1.0);
        let max = dice.between(2.0, 30.0);
        let radius = dice.between(0.1, 1.0);

        let each: Vec<Hit> = alone
            .iter()
            .enumerate()
            .filter_map(|(i, one)| {
                one.ray_cast(origin, direction, max)
                    .map(|hit| renamed(hit, i))
            })
            .collect();
        rays += usize::from(!each.is_empty());
        let got = whole.ray_cast(origin, direction, max);
        assert!(
            agrees(got, each),
            "a ray from {origin} along {direction}: {got:?}"
        );

        let each: Vec<Hit> = alone
            .iter()
            .enumerate()
            .filter_map(|(i, one)| {
                one.sweep_sphere(origin, direction, max, radius)
                    .map(|hit| renamed(hit, i))
            })
            .collect();
        sweeps += usize::from(!each.is_empty());
        let got = whole.sweep_sphere(origin, direction, max, radius);
        assert!(
            agrees(got, each),
            "a sphere of {radius} from {origin} along {direction}: {got:?}"
        );
    }
    assert!(
        rays > 40 && sweeps > 100,
        "{rays} rays and {sweeps} sweeps met something"
    );
}

/// ⚠ The two triangles of a seam share their corners to the bit, and a ray's place in each is computed from its origin: from far
/// off the rounding of that is larger than any fixed slack past an edge, and both would turn the ray away.
#[test]
fn a_slanted_ray_from_hundreds_of_metres_meets_the_corner_and_the_edge_it_is_aimed_at() {
    // A height field of quarter-metre cells far from the origin, each cell two triangles.
    let (o, n, cell) = (Vec3::new(-7900.13, 7411.9, 912.4), 24usize, 0.25f32);
    let mut dice = Dice(4);
    let mut positions = Vec::new();
    for i in 0..=n {
        for j in 0..=n {
            let across = Vec3::new(i as f32 - n as f32 * 0.5, j as f32 - n as f32 * 0.5, 0.0);
            positions.push(o + across * cell + Vec3::Z * dice.between(0.0, 0.1));
        }
    }
    let at = |i: usize, j: usize| (i * (n + 1) + j) as u16;
    let mut faces = Vec::new();
    for i in 0..n {
        for j in 0..n {
            faces.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
            faces.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    let field = bounds::triangles(&positions, &faces);
    let collider = collider(&PhBoundAny::PhBoundGeometry(field.clone()));
    let corner = |i: usize, j: usize| stored(&field, usize::from(at(i, j)));

    let (mut asked, mut missed) = (0usize, Vec::new());
    for i in 2..n - 1 {
        for j in 2..n - 1 {
            // The corner six triangles share, and a point of the edge to the next corner.
            let (here, next) = (corner(i, j), corner(i + 1, j + 1));
            for aim in [here, here + (next - here) * 0.37] {
                for _ in 0..3 {
                    let away = DVec3::new(
                        f64::from(dice.between(-400.0, 400.0)),
                        f64::from(dice.between(-400.0, 400.0)),
                        f64::from(dice.between(150.0, 400.0)),
                    );
                    let from = (aim + away).as_vec3();
                    let to = aim.as_vec3() - from;
                    asked += 1;
                    match collider.ray_cast(from, to, to.length() * 2.0) {
                        // A slanted ray may meet a rise of the field before the point it is aimed at, a little short of it.
                        Some(hit) => assert!(
                            (d(hit.position) - aim).length() < 1.0,
                            "a ray aimed at {aim} met the field at {}",
                            hit.position
                        ),
                        None => missed.push((from, aim)),
                    }
                }
            }
        }
    }
    assert!(asked > 2000, "{asked} rays");
    assert!(
        missed.is_empty(),
        "{} of {asked} rays passed through a seam, the first from {:?}",
        missed.len(),
        missed.first()
    );
}
