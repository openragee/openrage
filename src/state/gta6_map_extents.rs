//! A scanned GTA VI map's stored extents grown to hold an entity placed, as GTA V's placement grows
//! its map's: `entitiesExtents` over the archetype's box as placed, `streamingExtents` over the
//! origin give or take the stream distance. Neither shrinks. Where the box or the distance is not
//! known, its extents grow to hold the entity's position only.

use glam::Vec3;
use or_io::gta6::map::{Aabb, Archetype, Entity};
use or_io::hash::literal_string_hash as lit;
use or_io::meta::binary::hash_index::default_hash_index;
use or_io::meta::{MetaInstance, Value};
use or_world::transform::Pose;

const ENTITIES_MIN: u32 = lit(b"entitiesExtentsMin");
const ENTITIES_MAX: u32 = lit(b"entitiesExtentsMax");
const STREAMING_MIN: u32 = lit(b"streamingExtentsMin");
const STREAMING_MAX: u32 = lit(b"streamingExtentsMax");

fn get(tree: &MetaInstance, member: u32) -> Option<[f32; 3]> {
    let _names = or_io::meta::binary::hash_index::title_names();
    match tree.get(&default_hash_index().name_or_literal(member))? {
        Value::Vec3(v) => Some(*v),
        _ => None,
    }
}

/// The `entitiesExtents` and `streamingExtents` a map's tree holds; `None` where it lacks one.
pub(super) fn extents_of(tree: &MetaInstance) -> Option<(Aabb, Aabb)> {
    let entities = Aabb {
        min: get(tree, ENTITIES_MIN)?,
        max: get(tree, ENTITIES_MAX)?,
    };
    let streaming = Aabb {
        min: get(tree, STREAMING_MIN)?,
        max: get(tree, STREAMING_MAX)?,
    };
    Some((entities, streaming))
}

/// The root members that change, with their values, for `entities` and `streaming` to hold `e`
/// of archetype `a` placed at `pose`, and whether one grew to hold the position only because the
/// archetype's box or the stream distance is not known.
pub(super) fn grown(
    (entities, streaming): (Aabb, Aabb),
    e: &Entity,
    a: Option<&Archetype>,
    pose: Pose,
) -> (Vec<(u32, Value)>, bool) {
    let mut out = Vec::new();
    let mut position_only = false;
    let at = [pose.position];
    match a {
        Some(a) => {
            let scale = or_world::transform::scale_of(pose.scale_xy, pose.scale_z);
            let corners = or_world::transform::placed_corners(
                Vec3::from(a.bb_min) * scale,
                Vec3::from(a.bb_max) * scale,
                pose.rotation.normalize(),
                pose.position,
            );
            hold(&mut out, entities, &corners, [ENTITIES_MIN, ENTITIES_MAX]);
        }
        None => position_only |= hold(&mut out, entities, &at, [ENTITIES_MIN, ENTITIES_MAX]),
    }
    // −1 is where the archetype's distance applies.
    let reach = match (e.lod_dist < 0.0, a) {
        (false, _) => Some(e.lod_dist),
        (true, Some(a)) => Some(a.lod_dist),
        (true, None) => None,
    };
    match reach.filter(|d| d.is_finite() && *d >= 0.0) {
        Some(d) => {
            let d = Vec3::splat(d);
            let points = [pose.position - d, pose.position + d];
            hold(&mut out, streaming, &points, [STREAMING_MIN, STREAMING_MAX]);
        }
        None => position_only |= hold(&mut out, streaming, &at, [STREAMING_MIN, STREAMING_MAX]),
    }
    (out, position_only)
}

/// `held` grown over `points`, each corner that moves pushed as the member it is; whether one moved.
fn hold(out: &mut Vec<(u32, Value)>, held: Aabb, points: &[Vec3], [min, max]: [u32; 2]) -> bool {
    let (was_lo, was_hi) = (Vec3::from(held.min), Vec3::from(held.max));
    let (lo, hi) = (points.iter()).fold((was_lo, was_hi), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    if lo != was_lo {
        out.push((min, Value::Vec3(lo.to_array())));
    }
    if hi != was_hi {
        out.push((max, Value::Vec3(hi.to_array())));
    }
    lo != was_lo || hi != was_hi
}

#[cfg(test)]
mod tests {
    use glam::Quat;
    use or_io::gta6::map::ArchetypeClass;

    use super::*;

    fn entity(lod_dist: f32) -> Entity {
        Entity {
            interior: false,
            id: 1,
            rotation: [0.0, 0.0, 0.0, 1.0],
            archetype_word: 2,
            position: [0.0; 3],
            archetype_name: 3,
            flags: 0,
            scale_xy: 1.0,
            scale_z: 1.0,
            parent_index: -1,
            hash_e74c3c23: 0,
            lod_dist,
            child_lod_dist: 0.0,
            num_children: 0,
            tint_value: 0,
            priority_level: 0,
            lod_level: 5,
        }
    }

    fn archetype(lod_dist: f32) -> Archetype {
        Archetype {
            class: ArchetypeClass::Base,
            name: 3,
            name_word: 2,
            texture_dictionary: 0,
            drawable_dictionary: 0,
            drawable: 2,
            fragment: 0,
            lod_dist,
            special_attribute: 0,
            bb_min: [-1.0, -2.0, 0.0],
            bb_max: [1.0, 2.0, 3.0],
            bs_centre: [0.0; 3],
            bs_radius: 3.0,
            flags: 0,
            hd_texture_dist: 0.0,
            clip_dictionary: 0,
            physics_dictionary: 0,
            asset_type: 2,
            asset_name: 0,
        }
    }

    fn cube(x: f32) -> Aabb {
        Aabb {
            min: [-x; 3],
            max: [x; 3],
        }
    }

    #[test]
    fn a_placement_past_the_extents_grows_the_corners_it_passes_and_none_inside() {
        let held = (cube(50.0), cube(500.0));
        // Turned a quarter about z and doubled across, the box reaches 4 m either side in x.
        let pose = Pose {
            position: Vec3::new(100.0, 0.0, 0.0),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale_xy: 2.0,
            scale_z: 1.0,
        };
        let a = archetype(450.0);
        let (out, position_only) = grown(held, &entity(-1.0), Some(&a), pose);
        assert!(!position_only);
        assert_eq!(out.len(), 2, "{out:?}");
        let (member, Value::Vec3(max)) = &out[0] else {
            panic!("a corner")
        };
        assert_eq!(*member, ENTITIES_MAX);
        let want = [104.0, 50.0, 50.0];
        assert!((0..3).all(|k| (max[k] - want[k]).abs() < 1e-4), "{max:?}");
        // The archetype's 450 m from x = 100 reaches past 500 m in x only.
        assert_eq!(out[1], (STREAMING_MAX, Value::Vec3([550.0, 500.0, 500.0])));

        // Inside both, with or without its archetype: nothing changes.
        let inside = Pose {
            position: Vec3::ZERO,
            ..pose
        };
        assert_eq!(
            grown(held, &entity(10.0), Some(&a), inside),
            (Vec::new(), false)
        );
        assert_eq!(
            grown(held, &entity(-1.0), None, inside),
            (Vec::new(), false)
        );
        // An archetype not found: the entities extents hold the position only, and say so.
        let at_x = (ENTITIES_MAX, Value::Vec3([100.0, 50.0, 50.0]));
        assert_eq!(
            grown(held, &entity(-1.0), None, pose),
            (vec![at_x.clone()], true)
        );
        // The entity's own distance still applies to the streaming extents without its archetype.
        let out = grown(held, &entity(420.0), None, pose);
        let reach = (STREAMING_MAX, Value::Vec3([520.0, 500.0, 500.0]));
        assert_eq!(out, (vec![at_x, reach], true));
    }

    #[test]
    fn a_stream_distance_not_known_grows_the_streaming_extents_to_the_position_only() {
        let held = (cube(50.0), cube(500.0));
        let far = Pose {
            position: Vec3::new(0.0, -700.0, 0.0),
            rotation: Quat::IDENTITY,
            scale_xy: 1.0,
            scale_z: 1.0,
        };
        // The archetype's box is known, but neither the entity nor its archetype states a distance.
        let a = archetype(-1.0);
        let (out, position_only) = grown(held, &entity(-1.0), Some(&a), far);
        assert!(position_only, "{out:?}");
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0], (ENTITIES_MIN, Value::Vec3([-50.0, -702.0, -50.0])));
        assert_eq!(
            out[1],
            (STREAMING_MIN, Value::Vec3([-500.0, -700.0, -500.0]))
        );
    }

    #[test]
    fn the_extents_are_read_from_the_tree_and_a_tree_lacking_one_states_none() {
        let mut tree = MetaInstance::new("CMapData");
        let name = |h| default_hash_index().name_or_literal(h);
        for (member, v) in [
            (ENTITIES_MIN, -1.0),
            (ENTITIES_MAX, 1.0),
            (STREAMING_MIN, -2.0),
        ] {
            tree.set(name(member), Value::Vec3([v; 3]));
        }
        assert!(extents_of(&tree).is_none());
        tree.set(name(STREAMING_MAX), Value::Vec3([2.0; 3]));
        assert_eq!(extents_of(&tree), Some((cube(1.0), cube(2.0))));
    }
}
