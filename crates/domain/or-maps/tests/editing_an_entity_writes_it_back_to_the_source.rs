//! An entity moved, turned, scaled, added or taken out is written back to the source it is in: the XML changes, the source cooks to a map
//! file that reads with the change and with nothing else, and the XML as it was puts the file back as it was.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this.

use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::typed::{CMapData, CMapTypes};
use or_maps::edit::{
    add_archetype, add_entity, edit_map, edit_types, grow_extents, place_entity, pose_of,
    remove_archetype, remove_entity,
};
use or_maps::facts::{cooked, Name};
use or_maps::import::{add_files, Incoming};
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_world::transform::Pose;

/// A map set holding a map file of `entities` and a type file of two crates, and the folder of the map file's source.
fn source(dir: &Path, entities: &[Entity<'_>]) -> (PathBuf, PathBuf) {
    let folder = dir.join("maps");
    let map = map_file("harbour", entities, ([0.0; 3], [100.0; 3]));
    let types = types_file(
        "harbour_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        ],
    );
    add_files(
        &folder,
        &[
            Incoming {
                name: "harbour.ymap",
                bytes: &map,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "harbour_types.ytyp",
                bytes: &types,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    (folder.clone(), folder.join("harbour.metadata.src"))
}

fn two_crates() -> Vec<Entity<'static>> {
    vec![
        Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
        Entity {
            yaw: 0.6,
            ..Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 2)
        },
    ]
}

fn map_of(dir: &Path) -> CMapData {
    CMapData::from_bytes(&cooked(dir).unwrap()).unwrap()
}

#[test]
fn moving_an_entity_writes_the_new_position_to_the_source_and_the_xml_as_it_was_puts_it_back() {
    let scratch = tempfile::tempdir().unwrap();
    let (_, dir) = source(scratch.path(), &two_crates());
    let before_bytes = cooked(&dir).unwrap();
    let pose = pose_of(&map_of(&dir).entities[1]);

    let edited = edit_map(&dir, |map| {
        place_entity(
            map,
            1,
            Pose {
                position: pose.position + Vec3::new(2.5, -1.0, 0.75),
                ..pose
            },
        )
    })
    .expect("the edit is written");
    assert!(edited.changed());
    let after = map_of(&dir);
    assert_eq!(after.entities.len(), 2);
    let moved = pose_of(&after.entities[1]);
    assert!(
        (moved.position - Vec3::new(16.5, 19.0, 1.25)).length() < 1e-4,
        "{:?}",
        moved.position
    );
    assert!(
        moved.rotation.abs_diff_eq(pose.rotation, 1e-4),
        "a move leaves the turn alone"
    );
    let other = pose_of(&after.entities[0]);
    assert_eq!(
        other.position,
        Vec3::new(10.0, 20.0, 0.5),
        "the other entity is where it was"
    );
    assert_eq!(after.entities[1].guid, 2);

    // The XML as it was puts the file back as it was: that is the one Undo of the app.
    std::fs::write(&edited.file, &edited.before).unwrap();
    assert_eq!(cooked(&dir).unwrap(), before_bytes);
    std::fs::write(&edited.file, &edited.after).unwrap();
    assert!(
        (pose_of(&map_of(&dir).entities[1]).position - Vec3::new(16.5, 19.0, 1.25)).length() < 1e-4
    );
}

#[test]
fn turning_and_scaling_are_stored_the_way_the_file_stores_them() {
    let scratch = tempfile::tempdir().unwrap();
    let (_, dir) = source(scratch.path(), &two_crates());
    let pose = pose_of(&map_of(&dir).entities[0]);
    let turned = Quat::from_rotation_z(1.2) * Quat::from_rotation_x(0.3);
    edit_map(&dir, |map| {
        place_entity(
            map,
            0,
            Pose {
                rotation: turned,
                scale_xy: 2.0,
                scale_z: 3.0,
                ..pose
            },
        )
    })
    .unwrap();
    let after = pose_of(&map_of(&dir).entities[0]);
    assert!(
        after.rotation.abs_diff_eq(turned, 1e-4) || after.rotation.abs_diff_eq(-turned, 1e-4),
        "{:?} vs {turned:?}",
        after.rotation
    );
    assert_eq!((after.scale_xy, after.scale_z), (2.0, 3.0));
    // The file stores the conjugate for an entity.
    let stored = map_of(&dir).entities[0].rotation;
    let conjugate = turned.conjugate();
    assert!((stored.x - conjugate.x).abs() < 1e-4 && (stored.w - conjugate.w).abs() < 1e-4);
}

#[test]
fn a_pose_that_is_not_a_number_is_refused_and_the_source_is_as_it_was() {
    let scratch = tempfile::tempdir().unwrap();
    let (_, dir) = source(scratch.path(), &two_crates());
    let before = std::fs::read(dir.join("metadata.xml")).unwrap();
    let pose = pose_of(&map_of(&dir).entities[0]);
    let refused = edit_map(&dir, |map| {
        place_entity(
            map,
            0,
            Pose {
                position: Vec3::new(f32::NAN, 0.0, 0.0),
                ..pose
            },
        )
    });
    assert!(refused.unwrap_err().contains("finite"));
    let past = edit_map(&dir, |map| place_entity(map, 9, pose));
    assert!(past.unwrap_err().contains("past the 2"));
    assert_eq!(std::fs::read(dir.join("metadata.xml")).unwrap(), before);
}

#[test]
fn an_entity_is_added_as_a_copy_with_a_guid_of_its_own_and_taken_out_with_the_links_renumbered() {
    let scratch = tempfile::tempdir().unwrap();
    // 0 is the LOD parent of 1 and 2; 3 stands alone.
    let entities = vec![
        Entity {
            children: 2,
            ..Entity::at("prop_crate_a", [0.0, 0.0, 0.0], 1)
        },
        Entity {
            parent: 0,
            ..Entity::at("prop_crate_a", [1.0, 0.0, 0.0], 2)
        },
        Entity {
            parent: 0,
            ..Entity::at("prop_crate_b", [2.0, 0.0, 0.0], 3)
        },
        Entity::at("prop_crate_b", [3.0, 0.0, 0.0], 4),
    ];
    let (_, dir) = source(scratch.path(), &entities);

    let pose = Pose {
        position: Vec3::new(50.0, 60.0, 0.0),
        rotation: Quat::IDENTITY,
        scale_xy: 1.0,
        scale_z: 1.0,
    };
    edit_map(&dir, |map| {
        let donor = map.entities[3].clone();
        let at = add_entity(map, &donor, Name::from_text("prop_crate_a"), pose)?;
        assert_eq!(at, 4);
        Ok(())
    })
    .unwrap();
    let map = map_of(&dir);
    assert_eq!(map.entities.len(), 5);
    let added = &map.entities[4];
    assert!(added.archetype_name == "prop_crate_a");
    assert_eq!(added.guid, 5, "a guid of its own: one past the largest");
    assert_eq!((added.parent_index, added.num_children), (-1, 0));
    assert_eq!(added.position.x, 50.0);

    // Taking out the middle child: the parent has one child fewer, the later ones move down.
    edit_map(&dir, |map| remove_entity(map, 1)).unwrap();
    let map = map_of(&dir);
    assert_eq!(map.entities.len(), 4);
    assert_eq!(map.entities[0].num_children, 1);
    assert_eq!(map.entities[1].guid, 3);
    assert_eq!(map.entities[1].parent_index, 0);

    // Taking out the parent: its child has none.
    edit_map(&dir, |map| remove_entity(map, 0)).unwrap();
    let map = map_of(&dir);
    assert_eq!(map.entities[0].guid, 3);
    assert_eq!(
        map.entities[0].parent_index, -1,
        "what named the parent as its own has none now"
    );
    assert!(edit_map(&dir, |map| remove_entity(map, 40))
        .unwrap_err()
        .contains("past the"));
}

#[test]
fn an_archetype_is_added_as_a_copy_under_a_name_nobody_holds_and_taken_out() {
    let scratch = tempfile::tempdir().unwrap();
    let (_, map_dir) = source(scratch.path(), &two_crates());
    let types_dir = map_dir.parent().unwrap().join("harbour_types.metadata.src");
    let types = || CMapTypes::from_bytes(&cooked(&types_dir).unwrap()).unwrap();

    let donor = or_maps::edit::read_archetype(&types_dir, 1).unwrap();
    edit_types(&types_dir, |t| {
        add_archetype(
            t,
            Some(&donor),
            "prop_new_crate",
            Some("prop_new_crate_lod"),
        )
        .map(drop)
    })
    .unwrap();
    let after = types();
    assert_eq!(after.archetypes.len(), 3);
    assert!(after.archetypes[2].name == "prop_new_crate");
    assert!(after.archetypes[2].asset_name == "prop_new_crate_lod");
    assert_eq!(
        after.archetypes[2].bb_max.x, 2.0,
        "the copy keeps the box of its donor"
    );

    let twice = edit_types(&types_dir, |t| {
        add_archetype(t, None, "PROP_NEW_CRATE", None).map(drop)
    });
    assert!(twice.unwrap_err().contains("resolves a name once"));
    edit_types(&types_dir, |t| remove_archetype(t, 0)).unwrap();
    assert_eq!(types().archetypes.len(), 2);
}

#[test]
fn the_extents_grow_to_hold_a_moved_entity_and_never_shrink() {
    let scratch = tempfile::tempdir().unwrap();
    let (_, dir) = source(scratch.path(), &two_crates());
    let pose = pose_of(&map_of(&dir).entities[0]);
    edit_map(&dir, |map| {
        place_entity(
            map,
            0,
            Pose {
                position: Vec3::new(150.0, 20.0, 0.5),
                ..pose
            },
        )?;
        grow_extents(map, |hash| {
            (hash == or_io::string_hash(b"prop_crate_a")).then_some((
                Vec3::splat(-1.0),
                Vec3::splat(1.0),
                0.0,
            ))
        });
        Ok(())
    })
    .unwrap();
    let map = map_of(&dir);
    assert!(
        map.entities_extents_max.x >= 151.0,
        "{:?}",
        map.entities_extents_max
    );
    assert_eq!(
        map.entities_extents_min.x, 0.0,
        "the minimum the file stated stays"
    );
}
