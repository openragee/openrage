use or_io::gta6::map::interior::{InteriorChild, InteriorRoom, InteriorSet};
use or_io::gta6::map::links::{EntitySetLocation, Portal, Room};
use or_world::streaming::FADE;

use super::super::index::tests_support::index_of;
use super::*;

/// A chair that draws to 20 m and a lamp that draws to 50, each with a model, and a ghost with none.
const CHAIR: u64 = 100;
const LAMP: u64 = 200;
const GHOST: u64 = 300;
const SHELL: u64 = 900;
const KITCHEN: EntitySetLocation = EntitySetLocation::Room(1);

fn index() -> WorldIndex {
    index_of(
        Vec::new(),
        &[
            (CHAIR, 20.0, Some(0x1000)),
            (LAMP, 50.0, Some(0x2000)),
            (GHOST, 20.0, None),
        ],
    )
}

fn child(word: u64, at: [f32; 3], lod_dist: f32) -> InteriorChild {
    InteriorChild {
        archetype_word: word,
        archetype_name: word as u32,
        position: at,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
        lod_dist,
        flags: 0,
        lod_level: 5,
    }
}

fn room(attached_objects: Vec<u32>) -> InteriorRoom {
    InteriorRoom {
        room: Room {
            name: 1,
            bb_min: [-4.0, -3.0, -1.0],
            bb_max: [4.0, 3.0, 3.0],
            flags: 0,
            portal_count: 0,
            floor_id: 0,
            attached_objects,
        },
        blend: 0.0,
        timecycle_name: 0,
        secondary_timecycle_name: 0,
    }
}

/// An interior 8 m by 6 that draws to 30 m: a chair, a ghost and an entity of no archetype in its
/// second room, a lamp in each of two entity sets, and the whole of its east wall open.
fn interior() -> Interior {
    let lamp = |name, at| InteriorSet {
        name,
        locations: vec![KITCHEN],
        entities: vec![child(LAMP, at, 5.0)],
    };
    Interior {
        archetype: 3,
        name: 9,
        name_word: SHELL,
        lod_dist: 30.0,
        bb_min: [-4.0, -3.0, 0.0],
        bb_max: [4.0, 3.0, 0.0],
        mlo_flags: 0,
        entities: vec![
            child(CHAIR, [1.0, 0.0, 0.0], -1.0),
            child(GHOST, [0.0; 3], -1.0),
            child(0xDEAD, [0.0; 3], -1.0),
        ],
        rooms: vec![room(vec![]), room(vec![0, 1, 2])],
        portals: vec![Portal {
            room_from: 1,
            room_to: 0,
            flags: 0,
            corners: [
                [4.0, -3.0, -1.0],
                [4.0, 3.0, -1.0],
                [4.0, 3.0, 3.0],
                [4.0, -3.0, 3.0],
            ],
            attached_objects: vec![],
        }],
        entity_sets: vec![lamp(0xA, [0.0, 2.0, 0.0]), lamp(0xB, [0.0, -2.0, 0.0])],
        time_cycle_modifiers: vec![],
    }
}

/// An instance at (10, 20, 30), turned a quarter about z, that turns the entity set 0xA on.
fn instance() -> InteriorInstance {
    let half = std::f32::consts::FRAC_1_SQRT_2;
    InteriorInstance {
        entity: 0,
        archetype_word: SHELL,
        archetype_name: 9,
        position: [10.0, 20.0, 30.0],
        rotation: [0.0, 0.0, half, half],
        scale_xy: 1.0,
        scale_z: 1.0,
        lod_dist: -1.0,
        group_id: 0,
        floor_id: 0,
        default_entity_sets: vec![0xA],
        num_exit_portals: 0,
        flags: 2,
    }
}

fn origin(rows: &Rows) -> Vec3 {
    Vec3::new(rows[0][3], rows[1][3], rows[2][3])
}

fn near(found: Vec3, wanted: [f32; 3]) -> bool {
    found.distance(Vec3::from(wanted)) < 1e-4
}

fn model(offset: u64) -> ModelRef {
    ModelRef {
        archive: 0,
        offset,
        entry: None,
    }
}

#[test]
fn an_interior_s_entities_stand_in_the_world_under_the_instance_that_places_it() {
    let placed = place(&index(), &instance(), &interior());
    // The ghost has no model, 0xDEAD no archetype, and the set 0xB is off.
    let [chair, lamp] = placed.children.as_slice() else {
        panic!("{} children", placed.children.len());
    };
    assert_eq!((chair.model, lamp.model), (model(0x1000), model(0x2000)));
    // The quarter turn takes the interior's x to the world's y, and its y to the world's −x.
    assert!(near(origin(&chair.rows), [10.0, 21.0, 30.0]));
    assert!(near(origin(&lamp.rows), [8.0, 20.0, 30.0]));
    assert!(near(
        Vec3::new(chair.rows[0][0], chair.rows[1][0], chair.rows[2][0]),
        [0.0, 1.0, 0.0],
    ));
    assert!(near(chair.centre, [10.0, 21.0, 30.0]) && (chair.radius - 1.0).abs() < 1e-4);
    // The chair states no distance and takes its archetype's; the lamp states its own.
    assert_eq!((chair.lod_dist, lamp.lod_dist), (20.0, 5.0));
    assert_eq!(
        (chair.level, lamp.level),
        (Level::OrphanHd, Level::OrphanHd)
    );
    assert_eq!((chair.location, lamp.location), (KITCHEN, KITCHEN));
    assert!(near(placed.centre, [10.0, 20.0, 30.0]));
    assert!((placed.radius - 5.0).abs() < 1e-4);
    assert_eq!(placed.lod_dist, 30.0);
}

#[test]
fn an_instance_that_states_a_distance_draws_its_interior_to_it() {
    let far = InteriorInstance {
        lod_dist: 80.0,
        ..instance()
    };
    assert_eq!(place(&index(), &far, &interior()).lod_dist, 80.0);
}

/// The models `placed` draws at noon for a camera `north` metres along y from its middle, which
/// the instance's turn makes the east of the interior.
fn drawn(placed: &Placed, north: f32, scale: f32) -> Vec<ModelRef> {
    drawn_at(placed, north, &[scale; Level::COUNT], 12)
}

/// The same with a scale for each level, at the hour `hour`.
fn drawn_at(placed: &Placed, north: f32, scales: &[f32; Level::COUNT], hour: u32) -> Vec<ModelRef> {
    (faded_at(placed, north, scales, hour).into_iter())
        .map(|(model, _)| model)
        .collect()
}

fn view(eye: Vec3, scales: &[f32; Level::COUNT]) -> View {
    View {
        eye,
        velocity: Vec3::ZERO,
        scales: *scales,
        fade: [FADE; Level::COUNT],
    }
}

/// The same with how opaque each draws against the interior, which draws half opaque.
fn faded_at(
    placed: &Placed,
    north: f32,
    scales: &[f32; Level::COUNT],
    hour: u32,
) -> Vec<(ModelRef, f32)> {
    let mut models = Vec::new();
    let eye = placed.centre + Vec3::new(0.0, north, 0.0);
    placed.each(&view(eye, scales), hour, 0.5, |child, away, fade| {
        assert!((away - eye.distance(origin(&child.rows))).abs() < 1e-4);
        models.push((child.model, fade / 0.5));
    });
    models
}

#[test]
fn the_contents_draw_within_the_interior_s_distance_and_each_within_its_own() {
    let placed = place(&index(), &instance(), &interior());
    let (chair, lamp) = (model(0x1000), model(0x2000));
    assert_eq!(drawn(&placed, 0.0, 1.0), [chair, lamp]);
    // 10 m out the lamp is past its 5 m and the 5 m band past it.
    assert_eq!(drawn(&placed, 10.0, 1.0), [chair]);
    // 26.5 m out the chair, a metre north of the middle, is past its 20 m and its band.
    assert!(drawn(&placed, 26.5, 1.0).is_empty());
    // Past the interior's 30 m nothing of it draws, whatever a child's own distance.
    let mut long = placed.clone();
    long.children[0].lod_dist = 500.0;
    assert_eq!(drawn(&long, 29.0, 1.0), [chair]);
    assert!(drawn(&long, 31.0, 1.0).is_empty());
    // Every distance scales.
    assert_eq!(drawn(&placed, 31.0, 2.0), [chair]);
}

#[test]
fn a_camera_inside_the_interior_s_sphere_is_in_range_of_a_shorter_distance() {
    let mut placed = place(&index(), &instance(), &interior());
    placed.lod_dist = 2.0;
    assert_eq!(drawn(&placed, 4.0, 1.0), [model(0x1000), model(0x2000)]);
    assert!(drawn(&placed, 6.0, 1.0).is_empty());
}

#[test]
fn a_child_of_a_time_archetype_draws_inside_its_hours_alone() {
    let mut placed = place(&index(), &instance(), &interior());
    let chair = index().archetype(CHAIR).expect("the chair").hours;
    assert_eq!(placed.children[0].hours, chair);
    placed.children[0].hours = 1 << 20;
    let every = [1.0; Level::COUNT];
    assert_eq!(
        drawn_at(&placed, 0.0, &every, 20),
        [model(0x1000), model(0x2000)]
    );
    assert_eq!(drawn_at(&placed, 0.0, &every, 12), [model(0x2000)]);
}

#[test]
fn a_child_s_distance_scales_by_its_level_and_the_interior_s_by_hd() {
    let placed = place(&index(), &instance(), &interior());
    let mut scales = [1.0; Level::COUNT];
    scales[Level::OrphanHd as usize] = 0.5;
    // The chair, 9 m away, is within half its 20 m; 15 m away it is past that and its band.
    assert_eq!(drawn_at(&placed, 10.0, &scales, 12), [model(0x1000)]);
    assert!(drawn_at(&placed, 16.0, &scales, 12).is_empty());
    // A tenth of the interior's 30 m is within its 5 m sphere: 6 m out nothing draws.
    let mut scales = [1.0; Level::COUNT];
    scales[Level::Hd as usize] = 0.1;
    assert_eq!(
        drawn_at(&placed, 4.0, &scales, 12),
        [model(0x1000), model(0x2000)]
    );
    assert!(drawn_at(&placed, 6.0, &scales, 12).is_empty());
}

#[test]
fn a_child_fades_out_over_its_band_and_with_the_interior() {
    let placed = place(&index(), &instance(), &interior());
    let every = [1.0; Level::COUNT];
    // The chair states 20 m, which fades over 5: 22.5 m away it is half there.
    let faded = faded_at(&placed, 23.5, &every, 12);
    let [(chair, fade)] = faded.as_slice() else {
        panic!("{faded:?}");
    };
    assert_eq!(*chair, model(0x1000));
    assert!((fade - 0.5).abs() < 1e-4, "{fade}");
    // Within its distance it is as opaque as the interior.
    let near = faded_at(&placed, 0.0, &every, 12);
    assert!(near.iter().all(|(_, fade)| (fade - 1.0).abs() < 1e-6));
}

#[test]
fn from_outside_the_contents_draw_seen_through_a_portal_alone() {
    let placed = place(&index(), &instance(), &interior());
    // South of the middle is the interior's west, whose wall has no opening.
    assert!(drawn(&placed, -10.0, 1.0).is_empty());
    assert_eq!(drawn(&placed, 10.0, 1.0), [model(0x1000)]);
}

#[test]
fn the_camera_in_a_room_is_inside_the_interior_and_that_room() {
    let placed = place(&index(), &instance(), &interior());
    let inside = placed.inside(placed.centre).expect("in the second room");
    assert_eq!((inside.interior, inside.room), (9, 1));
    assert_eq!(
        placed.inside(placed.centre + Vec3::new(0.0, 10.0, 0.0)),
        None
    );
}

#[test]
fn a_plane_through_the_camera_in_the_interior_s_space_is_turned_into_the_world_s() {
    let mut placed = place(&index(), &instance(), &interior());
    // Turned a quarter round the vertical and twice as wide as it is high: its own +x is the world's +y.
    let outer = Mat4::from_scale_rotation_translation(
        Vec3::new(2.0, 2.0, 1.0),
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        Vec3::new(5.0, 6.0, 7.0),
    );
    placed.own = outer.inverse();
    let tilted = Vec3::new(1.0, 0.0, 1.0).normalize();
    let world = placed.to_world(&[vec![Vec3::X, tilted]]);
    assert!(world[0][0].abs_diff_eq(Vec3::Y, 1e-5), "{:?}", world[0][0]);
    // A tilted plane leans by the interior's heights over its widths.
    let expected = Vec3::new(0.0, 0.5, 1.0).normalize();
    assert!(world[0][1].abs_diff_eq(expected, 1e-5), "{:?}", world[0][1]);
}

#[test]
fn a_sphere_reaches_into_a_room_where_it_touches_the_room_s_box() {
    let placed = place(&index(), &instance(), &interior());
    assert!(placed.reaches_a_room(placed.centre, 0.1));
    let far = placed.centre + Vec3::new(0.0, 0.0, 1000.0);
    assert!(!placed.reaches_a_room(far, 1.0));
    assert!(placed.reaches_a_room(far, 1000.0));
}

/// A lamp of 10 cd of `kind`, its spot's outer half angle `cone_outer`.
fn ten_candelas(kind: LightKind, cone_outer: f32, colour: [f32; 3]) -> Light {
    Light {
        kind,
        position: [0.0; 3],
        direction: [0.0, 0.0, -1.0],
        colour,
        intensity: 10.0,
        radius: 5.0,
        falloff_exponent: 1.0,
        hours: 0x00FF_FFFF,
        flags: 0,
        cone_inner: 0.0,
        cone_outer,
        length: 0.0,
        corona: [0.0; 3],
    }
}

#[test]
fn a_lamp_s_flux_is_its_intensity_over_the_solid_angle_it_shines_into() {
    use std::f32::consts::PI;
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3 * b;
    let white = [1.0; 3];
    assert!(close(
        flux_of(&ten_candelas(LightKind::Point, 0.0, white)),
        40.0 * PI
    ));
    // A cone of 60° about its axis holds a quarter of the sphere.
    assert!(close(
        flux_of(&ten_candelas(LightKind::Spot, 60.0, white)),
        10.0 * PI
    ));
    assert!(close(
        flux_of(&ten_candelas(LightKind::Area, 0.0, white)),
        10.0 * PI
    ));
    // Green weighs most in what an eye sees of it.
    let green = flux_of(&ten_candelas(LightKind::Point, 0.0, [0.0, 1.0, 0.0]));
    assert!(close(green, 40.0 * PI * 0.7152));
}

#[test]
fn a_light_that_only_draws_a_corona_sends_no_light_round_its_room() {
    let mut light = ten_candelas(LightKind::Point, 0.0, [1.0; 3]);
    assert!(flux_of(&light) > 0.0);
    light.flags = or_io::gta6::map::light_ext::CORONA_ONLY;
    assert_eq!(flux_of(&light), 0.0);
}
