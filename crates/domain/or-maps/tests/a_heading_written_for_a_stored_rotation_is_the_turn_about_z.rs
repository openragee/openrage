//! The heading a script form writes for the rotation a map file stores: the turn about Z of the rotation that places the entity, read
//! from a sample map as the Map screen reads it.

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::typed::CMapData;
use or_maps::edit::pose_of;
use or_maps::sample::{map_file, Entity};
use or_world::place::{read_pasted, Pasted, PastedPlace};
use or_world::transform::Pose;
use or_world_gta5::script_text::{heading_of, ScriptForms};
use or_world_gta5::Placement;

fn forms() -> ScriptForms {
    ScriptForms::built_in().expect("the forms of the tree read")
}

/// Half of the last decimal written, and what a 32-bit number of that size cannot tell apart.
fn within(read: f32, written: f32, decimals: usize) -> bool {
    let half = 0.5 * 10f64.powi(-(decimals as i32));
    let grain = f64::from(written.abs().max(1.0)) * f64::from(f32::EPSILON);
    (f64::from(read) - f64::from(written)).abs() <= half + grain
}

fn reads_back(text: &str, position: Vec3, heading: Option<f32>, decimals: usize) {
    let read = read_pasted(text);
    let Ok(Pasted::Place(PastedPlace {
        x,
        y,
        z: Some(z),
        heading: read_heading,
    })) = read
    else {
        panic!("{text:?} read as {read:?}, which is no place with a height");
    };
    for (read, written) in [(x, position.x), (y, position.y), (z, position.z)] {
        assert!(
            within(read, written, decimals),
            "{text:?}: {read} is not {written} to {decimals} decimals"
        );
    }
    match (read_heading, heading) {
        (Some(read), Some(written)) => assert!(
            within(read, written, decimals),
            "{text:?}: the heading {read} is not {written} to {decimals} decimals"
        ),
        (None, None) => {}
        other => panic!("{text:?}: the heading read and the one written are {other:?}"),
    }
}

#[test]
fn a_heading_written_for_a_stored_rotation_is_the_turn_about_z_of_its_quaternion() {
    // An entity stores the conjugate of the rotation that places it; the file is read back as the Map screen reads it.
    for yaw in [0.0_f32, 0.4, 1.2, 2.9, -0.7, -2.5] {
        let entity = Entity {
            yaw,
            ..Entity::at("prop_bench_01a", [125.5, -340.25, 28.0], 1)
        };
        let map = CMapData::from_bytes(&map_file("yard", &[entity], ([0.0; 3], [1.0; 3])))
            .expect("the sample map reads");
        let stored = map.entities[0].rotation;
        let placing = Quat::from_xyzw(stored.x, stored.y, stored.z, stored.w).conjugate();
        // Where the rotation that places it turns the Y axis, as an angle from Y towards -X: the turn about Z.
        let forward = placing * Vec3::Y;
        let expected = (-forward.x).atan2(forward.y).to_degrees().rem_euclid(360.0);
        let pose = pose_of(&map.entities[0]);
        let heading = heading_of(&pose);
        assert!(
            (heading - expected).abs() < 1e-3,
            "{yaw}: {heading} is not {expected}"
        );
        assert!((0.0..360.0).contains(&heading), "{heading}");
        for form in &forms().forms {
            let text = form.place(pose.position, Some(heading), 2).unwrap();
            reads_back(&text, pose.position, Some(expected), 2);
        }
    }
    // A turn a hair to either side of nought is a heading from 0 up to 360 and never 360 itself.
    for hair in [-1.0e-4_f32, -1.0e-5, -1.0e-6, -1.0e-7, -0.0, 0.0, 1.0e-6] {
        let pose = Pose {
            position: Vec3::ZERO,
            rotation: Quat::from_rotation_z(hair.to_radians()),
            scale_xy: 1.0,
            scale_z: 1.0,
        };
        let heading = heading_of(&pose);
        assert!((0.0..360.0).contains(&heading), "{hair}: {heading}");
        let from_nought = heading.min(360.0 - heading);
        assert!(from_nought < 1e-3, "{hair}: {heading}");
    }
    // The rotation of an interior is stored as it is, and the same heading comes of it.
    let turn = Quat::from_rotation_z(1.2);
    let pose = |placement: Placement| Pose {
        position: Vec3::ZERO,
        rotation: placement.rotation_of(placement.store(turn)),
        scale_xy: 1.0,
        scale_z: 1.0,
    };
    let expected = 1.2_f32.to_degrees();
    for placement in [Placement::Entity, Placement::InteriorInstance] {
        assert!((heading_of(&pose(placement)) - expected).abs() < 1e-3);
    }
}
