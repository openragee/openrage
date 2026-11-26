//! What a map changed of its base is said by object: an entity moved is one move, an entity taken out of the middle is one deletion
//! whatever index the others have now, a copy is one addition, and a twin that moved is ambiguous and never a move.
//!
//! ⚠ The files are the crate's samples and the edits are `or_maps::edit`'s, so the clean checkout runs this.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{
    CBaseArchetypeDef, CBaseArchetypeDefAny, CEntityDef, CEntityDefAny, CMapData, CMapTypes,
    CMloArchetypeDef, CMloInstanceDef, CTimeArchetypeDef,
};
use or_io::meta::{build_library, canonical, write_meta_binary, BinaryFormat, MetaInstance, Value};
use or_maps::changes::{
    entity_changes, map_changes, source_changes, types_changes, EntityChange, Error, MapChanges,
    Matched, PackSources, SourceChanges, Stated,
};
use or_maps::edit::{add_entity, edit_map, place_entity, pose_of, remove_entity, Pose};
use or_maps::facts::Name;
use or_maps::import::{add_files, Incoming};
use or_maps::sample::{child_map_file, map_file, types_file, Archetype, Entity};

fn hash(name: &str) -> u32 {
    Name::from_text(name).hash
}

fn map(entities: &[Entity<'_>]) -> CMapData {
    CMapData::from_bytes(&map_file("harbour", entities, ([0.0; 3], [100.0; 3]))).unwrap()
}

/// Two level-of-detail parents with their children around a crate that has no parent: every link is an index of the list.
fn a_yard() -> Vec<Entity<'static>> {
    vec![
        Entity {
            children: 2,
            ..Entity::at("yard_lod_a", [10.0, 10.0, 0.0], 1)
        },
        Entity {
            parent: 0,
            ..Entity::at("yard_hd_a", [10.0, 12.0, 0.0], 2)
        },
        Entity::at("prop_crate_a", [20.0, 20.0, 0.5], 3),
        Entity {
            children: 2,
            ..Entity::at("yard_lod_b", [30.0, 10.0, 0.0], 4)
        },
        Entity {
            parent: 3,
            ..Entity::at("yard_hd_b", [30.0, 12.0, 0.0], 5)
        },
        Entity {
            parent: 3,
            yaw: 0.6,
            ..Entity::at("yard_hd_b", [30.0, 14.0, 0.0], 6)
        },
        Entity {
            parent: 0,
            ..Entity::at("yard_hd_a", [10.0, 14.0, 0.0], 7)
        },
    ]
}

fn indexes(changes: &MapChanges) -> (Vec<usize>, Vec<usize>, Vec<(usize, usize)>) {
    (
        changes.added.iter().map(|e| e.index).collect(),
        changes.deleted.iter().map(|e| e.index).collect(),
        (changes.changed.iter())
            .map(|k| (k.base.index, k.now.index))
            .collect(),
    )
}

#[test]
fn an_entity_moved_is_one_move_with_where_it_stood_and_where_it_stands() {
    let base = map(&a_yard());
    let mut now = base.clone();
    let pose = pose_of(&now.entities[2]);
    let moved = pose.position + Vec3::new(2.5, -1.0, 0.75);
    place_entity(
        &mut now,
        2,
        Pose {
            position: moved,
            ..pose
        },
    )
    .unwrap();

    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(2, 2)]));
    assert!(changes.ambiguous.is_empty());
    assert_eq!(changes.unchanged, base.entities.len() - 1);
    let kept = &changes.changed[0];
    assert_eq!(kept.matched, Matched::Guid);
    assert_eq!(kept.base.guid, 3);
    assert_eq!(kept.base.archetype.hash, hash("prop_crate_a"));
    assert_eq!(
        kept.changes,
        [EntityChange::Moved {
            from: pose.position.to_array(),
            to: moved.to_array(),
        }],
        "a move is a move and nothing else"
    );
    assert_eq!(kept.base.position, pose.position.to_array());
    assert_eq!(kept.now.position, moved.to_array());
}

#[test]
fn each_thing_an_entity_states_differently_is_named() {
    let base = map(&a_yard());
    let mut now = base.clone();
    let pose = pose_of(&now.entities[1]);
    let turned = Quat::from_rotation_z(1.2) * Quat::from_rotation_x(0.3);
    place_entity(
        &mut now,
        1,
        Pose {
            rotation: turned,
            scale_xy: 2.0,
            scale_z: 3.0,
            ..pose
        },
    )
    .unwrap();
    now.entities[1].archetype_name = HashedString::new("yard_hd_c");
    now.entities[1].lod_dist = 250.0;
    now.entities[1].flags = base.entities[1].flags | 1;
    now.entities[1].parent_index = 3;
    now.entities[1].tint_value = 4;
    now.entities[1].child_lod_dist = 12.0;

    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(1, 1)]));
    let found = &changes.changed[0].changes;
    let stored = pose_of(&now.entities[1]).rotation;
    assert_eq!(
        found,
        &[
            EntityChange::Archetype {
                from: changes.changed[0].base.archetype.clone(),
                to: Name::from_text("yard_hd_c"),
            },
            EntityChange::Turned {
                from: pose.rotation.to_array(),
                to: stored.to_array(),
            },
            EntityChange::Scaled {
                from: [pose.scale_xy, pose.scale_z],
                to: [2.0, 3.0],
            },
            EntityChange::LodDist {
                from: base.entities[1].lod_dist,
                to: 250.0,
            },
            EntityChange::Flags {
                from: base.entities[1].flags,
                to: base.entities[1].flags | 1,
            },
            EntityChange::Parent { from: 0, to: 3 },
            EntityChange::Members {
                names: vec!["childLodDist", "tintValue"],
            },
        ]
    );
    assert!(
        stored.abs_diff_eq(turned, 1e-5),
        "the turn is the rotation that places it"
    );
}

#[test]
fn an_entity_taken_out_of_the_middle_is_one_deletion_though_every_later_index_and_link_shifted() {
    let base = map(&a_yard());
    let mut now = base.clone();
    remove_entity(&mut now, 2).unwrap();
    assert_eq!(
        (now.entities[3].parent_index, now.entities[4].parent_index),
        (2, 2),
        "the links after it are other numbers now"
    );

    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![2], vec![]));
    assert!(changes.ambiguous.is_empty());
    assert_eq!(changes.unchanged, base.entities.len() - 1);
    assert_eq!(changes.deleted[0].guid, 3);
    assert_eq!(changes.deleted[0].position, [20.0, 20.0, 0.5]);
}

#[test]
fn a_child_taken_out_is_one_deletion_and_its_parent_counting_one_child_fewer_is_no_change() {
    let base = map(&a_yard());
    let mut now = base.clone();
    remove_entity(&mut now, 4).unwrap();
    assert_eq!(
        now.entities[3].num_children, 1,
        "the parent counts one fewer"
    );
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![4], vec![]));

    // A count that did not follow the children is a change of the parent's own.
    now.entities[3].num_children = 5;
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![4], vec![(3, 3)]));
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Members {
            names: vec!["numChildren"]
        }]
    );
}

#[test]
fn a_parent_taken_out_leaves_each_child_with_another_parent_and_a_link_moved_is_a_change() {
    let base = map(&a_yard());
    let mut now = base.clone();
    remove_entity(&mut now, 3).unwrap();
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![3], vec![(4, 3), (5, 4)]));
    for kept in &changes.changed {
        assert_eq!(
            kept.changes,
            [EntityChange::Parent { from: 3, to: -1 }],
            "the child has no parent now"
        );
    }

    // The same number naming another entity is a change, and another number naming the same entity is none.
    let mut now = base.clone();
    remove_entity(&mut now, 2).unwrap();
    now.entities[5].parent_index = 2;
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![2], vec![(6, 5)]));
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Parent { from: 0, to: 2 }]
    );
}

#[test]
fn a_link_into_the_parent_map_is_compared_as_the_number_it_is() {
    // The number is one an entity of this file has too, and it names none of them.
    let mut entities = a_yard();
    entities[6].parent = 4;
    let mut base = map(&entities);
    base.entities[6].flags |= 1 << 3;
    let mut now = base.clone();
    remove_entity(&mut now, 2).unwrap();
    assert_eq!(
        now.entities[5].parent_index, 4,
        "a deletion here does not renumber the parent map"
    );
    assert_eq!(
        indexes(&map_changes(&base, &now)),
        (vec![], vec![2], vec![])
    );

    now.entities[5].parent_index = 3;
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![2], vec![(6, 5)]));
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Parent { from: 4, to: 3 }]
    );

    // It is no child of the entity of this file that has its number: taking it out takes no child from that one.
    let mut entities = a_yard();
    entities[6].parent = 3;
    let mut base = map(&entities);
    base.entities[6].flags |= 1 << 3;
    let mut now = base.clone();
    remove_entity(&mut now, 6).unwrap();
    assert_eq!(now.entities[3].num_children, 2, "the edit leaves the count");
    assert_eq!(
        indexes(&map_changes(&base, &now)),
        (vec![], vec![6], vec![])
    );
    now.entities[3].num_children = 1;
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![6], vec![(3, 3)]));
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Members {
            names: vec!["numChildren"]
        }]
    );
}

#[test]
fn a_copy_of_a_neighbour_at_another_place_is_one_addition_and_the_neighbour_is_unchanged() {
    let base = map(&a_yard());
    let mut now = base.clone();
    let donor = now.entities[2].clone();
    let pose = pose_of(&donor);
    let stands = Pose {
        position: pose.position + Vec3::new(4.0, 0.0, 0.0),
        ..pose
    };
    let at = add_entity(&mut now, &donor, Name::from_text("prop_crate_a"), stands).unwrap();
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![at], vec![], vec![]));
    assert_eq!(changes.unchanged, base.entities.len());
    assert_eq!(changes.added[0].position, stands.position.to_array());
    assert_ne!(changes.added[0].guid, donor.guid);

    // A copy that kept the guid of its neighbour, and stands before it in the file: the neighbour is the one at its place.
    let mut entities = a_yard();
    entities.insert(0, Entity::at("prop_crate_a", [24.0, 20.0, 0.5], 3));
    for entity in &mut entities[1..] {
        if entity.parent >= 0 {
            entity.parent += 1;
        }
    }
    let changes = map_changes(&base, &map(&entities));
    assert_eq!(indexes(&changes), (vec![0], vec![], vec![]));
    assert!(changes.ambiguous.is_empty());
    assert_eq!(changes.unchanged, base.entities.len());
}

#[test]
fn two_twins_with_one_archetype_and_one_guid_are_ambiguous_when_one_moved_and_never_a_move() {
    let twins = |second: [f32; 3]| {
        map(&[
            Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 9),
            Entity::at("prop_tree", [10.0, 20.0, 0.0], 7),
            Entity::at("prop_tree", second, 7),
        ])
    };
    let base = twins([14.0, 20.0, 0.0]);
    let now = twins([14.0, 23.0, 0.0]);
    let changes = map_changes(&base, &now);
    assert_eq!(
        indexes(&changes),
        (vec![], vec![], vec![]),
        "nothing is said to be added, deleted or moved"
    );
    assert_eq!(changes.unchanged, 2, "the twin that stands where it stood");
    assert_eq!(changes.ambiguous.len(), 1);
    let group = &changes.ambiguous[0];
    assert_eq!(group.archetype.hash, hash("prop_tree"));
    assert_eq!(
        group.base.iter().map(|e| e.index).collect::<Vec<_>>(),
        [2],
        "the index it has in the base"
    );
    assert_eq!(group.now.iter().map(|e| e.index).collect::<Vec<_>>(), [2]);
    assert_eq!(group.base[0].position, [14.0, 20.0, 0.0]);
    assert_eq!(group.now[0].position, [14.0, 23.0, 0.0]);

    // Both moved: neither is told from the other.
    let both = map(&[
        Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 9),
        Entity::at("prop_tree", [10.0, 21.0, 0.0], 7),
        Entity::at("prop_tree", [14.0, 23.0, 0.0], 7),
    ]);
    let changes = map_changes(&base, &both);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![]));
    let group = &changes.ambiguous[0];
    assert_eq!(
        group.base.iter().map(|e| e.index).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        group.now.iter().map(|e| e.index).collect::<Vec<_>>(),
        [1, 2]
    );

    // One twin fewer, and the other where it stood: a deletion, since which of two copies went says nothing.
    let one = map(&[
        Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 9),
        Entity::at("prop_tree", [14.0, 20.0, 0.0], 7),
    ]);
    let changes = map_changes(&base, &one);
    assert_eq!(indexes(&changes), (vec![], vec![1], vec![]));
    assert!(changes.ambiguous.is_empty());

    // One twin fewer, and the other moved: the tree that is left is either, so none is said to be deleted and none to have moved.
    let left = map(&[
        Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 9),
        Entity::at("prop_tree", [14.0, 23.0, 0.0], 7),
    ]);
    let group = |changes: &MapChanges| {
        assert_eq!(indexes(changes), (vec![], vec![], vec![]), "{changes:?}");
        assert_eq!(changes.ambiguous.len(), 1);
        let of = |held: &[or_maps::changes::EntityRef]| -> Vec<usize> {
            held.iter().map(|e| e.index).collect()
        };
        (
            of(&changes.ambiguous[0].base),
            of(&changes.ambiguous[0].now),
        )
    };
    assert_eq!(group(&map_changes(&base, &left)), (vec![1, 2], vec![1]));
    // And the other way, where the twins are the changed map's: a tree that is two now, neither where it stood.
    assert_eq!(group(&map_changes(&left, &base)), (vec![1], vec![1, 2]));
}

#[test]
fn two_entities_at_one_place_are_not_told_apart_by_the_order_they_are_written_in() {
    // Two benches at one place, which shipped maps hold, each with a guid of its own.
    let base = map(&[
        Entity::at("prop_bench", [5.0, 5.0, 0.0], 1),
        Entity::at("prop_bench", [5.0, 5.0, 0.0], 2),
    ]);
    // One states another guid: the other is found by its guid, and this one is the bench that is left at the place.
    let one = map(&[
        Entity::at("prop_bench", [5.0, 5.0, 0.0], 1),
        Entity::at("prop_bench", [5.0, 5.0, 0.0], 102),
    ]);
    let changes = map_changes(&base, &one);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(1, 1)]));
    assert_eq!(changes.changed[0].matched, Matched::Place);
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Guid { from: 2, to: 102 }]
    );

    // Both state another guid: which is which is not said, and the order of the file does not say it.
    let both = map(&[
        Entity::at("prop_bench", [5.0, 5.0, 0.0], 102),
        Entity::at("prop_bench", [5.0, 5.0, 0.0], 101),
    ]);
    let changes = map_changes(&base, &both);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![]), "{changes:?}");
    assert_eq!(changes.unchanged, 0);
    assert_eq!(changes.ambiguous.len(), 1);
    assert_eq!(changes.ambiguous[0].base.len(), 2);
    assert_eq!(changes.ambiguous[0].now.len(), 2);
}

#[test]
fn a_parent_among_twins_is_the_same_parent_while_the_link_stays_among_them() {
    let yard = |second: [f32; 3], parent: i32| {
        map(&[
            Entity::at("yard_lod", [10.0, 20.0, 0.0], 7),
            Entity::at("yard_lod", second, 7),
            Entity {
                parent,
                ..Entity::at("yard_hd", [14.0, 21.0, 0.0], 9)
            },
        ])
    };
    let base = yard([14.0, 20.0, 0.0], 1);
    // The twin the child names moved, so nothing says which twin it is: the child still names one of them.
    let changes = map_changes(&base, &yard([14.0, 23.0, 0.0], 1));
    assert_eq!(indexes(&changes), (vec![], vec![], vec![]), "{changes:?}");
    assert_eq!(changes.ambiguous.len(), 1);
    assert_eq!(changes.unchanged, 2);

    // The child names the twin that stands where it stood, which is not the one it named.
    let changes = map_changes(&base, &yard([14.0, 23.0, 0.0], 0));
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(2, 2)]));
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Parent { from: 1, to: 0 }]
    );
}

#[test]
fn a_link_is_the_entity_it_names_and_not_its_number() {
    // The two parents changed places in the file, and the child states the number it stated: it names the other parent now.
    let base = map(&[
        Entity::at("yard_lod_a", [10.0, 10.0, 0.0], 1),
        Entity::at("yard_lod_b", [30.0, 10.0, 0.0], 2),
        Entity {
            parent: 0,
            ..Entity::at("yard_hd_a", [10.0, 12.0, 0.0], 3)
        },
    ]);
    let now = map(&[
        Entity::at("yard_lod_b", [30.0, 10.0, 0.0], 2),
        Entity::at("yard_lod_a", [10.0, 10.0, 0.0], 1),
        Entity {
            parent: 0,
            ..Entity::at("yard_hd_a", [10.0, 12.0, 0.0], 3)
        },
    ]);
    assert_eq!(
        base.entities[2], now.entities[2],
        "the child is the same record"
    );
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(2, 2)]));
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Parent { from: 0, to: 0 }],
        "each number is an index of its own file"
    );

    // A link past the end of both files names nothing: it is the same link while it is the same number, whatever was taken out before it.
    let mut entities = a_yard();
    entities[6].parent = 99;
    let base = map(&entities);
    let mut now = base.clone();
    remove_entity(&mut now, 2).unwrap();
    assert_eq!(now.entities[5].parent_index, 98, "the edit renumbers it");
    now.entities[5].parent_index = 99;
    assert_eq!(
        indexes(&map_changes(&base, &now)),
        (vec![], vec![2], vec![])
    );
    now.entities[5].parent_index = 98;
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![2], vec![(6, 5)]));
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Parent { from: 99, to: 98 }]
    );
}

#[test]
fn an_entity_no_guid_finds_is_the_one_of_its_archetype_at_its_place_and_else_the_nearest() {
    let base = map(&[
        Entity::at("prop_tree", [10.0, 20.0, 0.0], 7),
        Entity::at("prop_tree", [50.0, 20.0, 0.0], 8),
        Entity::at("prop_bench", [5.0, 5.0, 0.0], 9),
    ]);
    // Every guid is another one, as a tool that writes its own leaves them; one tree stands 2 m on, and the bench turned.
    let now = map(&[
        Entity::at("prop_tree", [52.0, 20.0, 0.0], 108),
        Entity::at("prop_tree", [10.0, 20.0, 0.0], 107),
        Entity {
            yaw: 0.5,
            ..Entity::at("prop_bench", [5.0, 5.0, 0.0], 109)
        },
    ]);
    let changes = map_changes(&base, &now);
    assert_eq!(
        indexes(&changes),
        (vec![], vec![], vec![(0, 1), (1, 0), (2, 2)])
    );
    assert!(changes.ambiguous.is_empty());
    assert_eq!(changes.changed[0].matched, Matched::Place);
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Guid { from: 7, to: 107 }]
    );
    assert_eq!(changes.changed[1].matched, Matched::Nearest);
    assert_eq!(
        changes.changed[1].changes,
        [
            EntityChange::Moved {
                from: [50.0, 20.0, 0.0],
                to: [52.0, 20.0, 0.0],
            },
            EntityChange::Guid { from: 8, to: 108 },
        ]
    );
    assert_eq!(changes.changed[2].matched, Matched::Nearest);
    assert!(matches!(
        changes.changed[2].changes[0],
        EntityChange::Turned { .. }
    ));

    // Two that stand as near to one: nothing says which it is.
    let near = map(&[
        Entity::at("prop_tree", [10.0, 20.0, 0.0], 107),
        Entity::at("prop_tree", [48.0, 20.0, 0.0], 108),
        Entity::at("prop_tree", [52.0, 20.0, 0.0], 110),
        Entity {
            yaw: 0.5,
            ..Entity::at("prop_bench", [5.0, 5.0, 0.0], 109)
        },
    ]);
    let changes = map_changes(&base, &near);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(0, 0), (2, 3)]));
    assert_eq!(changes.ambiguous.len(), 1);
    assert_eq!(
        changes.ambiguous[0]
            .now
            .iter()
            .map(|e| e.index)
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

/// ⚠ The trap `or_maps::changes` states: the nearest step believes a place over a guid.
#[test]
fn an_entity_taken_out_and_another_of_its_archetype_put_in_read_as_one_moved_with_another_guid() {
    let base = map(&[
        Entity::at("prop_lamp", [0.0, 0.0, 0.0], 1),
        Entity::at("prop_lamp", [10.0, 0.0, 0.0], 2),
        Entity::at("prop_lamp", [20.0, 0.0, 0.0], 3),
    ]);
    // The second lamp is taken out, and a lamp with a guid of its own is put in far from where it stood.
    let now = map(&[
        Entity::at("prop_lamp", [0.0, 0.0, 0.0], 1),
        Entity::at("prop_lamp", [20.0, 0.0, 0.0], 3),
        Entity::at("prop_lamp", [900.0, 0.0, 0.0], 4),
    ]);
    let changes = map_changes(&base, &now);
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(1, 2)]));
    assert_eq!(changes.unchanged, 2);
    // What tells a caller that no guid said so: the step that found it, and the guid that is another one.
    let kept = &changes.changed[0];
    assert_eq!(kept.matched, Matched::Nearest);
    assert_eq!(
        kept.changes,
        [
            EntityChange::Moved {
                from: [10.0, 0.0, 0.0],
                to: [900.0, 0.0, 0.0],
            },
            EntityChange::Guid { from: 2, to: 4 },
        ]
    );
}

#[test]
fn an_entity_whose_position_is_no_number_is_the_nearest_of_none() {
    // Three trees no guid finds, one of them at a position that is not a number and sorts before every number.
    let nowhere = f32::from_bits(0xFFC0_0000);
    assert!(nowhere.is_nan() && nowhere.is_sign_negative());
    let trees = |guids: u32, nudge: f32| {
        [
            entity("prop_tree", guids + 1, [nudge, 0.0, 0.0]),
            entity("prop_tree", guids + 2, [100.0 + nudge, 0.0, 0.0]),
            entity("prop_tree", guids + 3, [nowhere, nudge, 0.0]),
        ]
        .map(CEntityDefAny::Base)
    };
    let changes = entity_changes(&trees(0, 0.0), &trees(10, 1.0));
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(0, 0), (1, 1)]));
    assert!(changes
        .changed
        .iter()
        .all(|k| k.matched == Matched::Nearest));
    assert_eq!(changes.ambiguous.len(), 1, "{changes:?}");
    assert_eq!(changes.ambiguous[0].base[0].index, 2);
    assert_eq!(changes.ambiguous[0].now[0].index, 2);
}

#[test]
fn a_map_compared_with_itself_reports_nothing() {
    // Twins at one place with one guid, and twins apart, which shipped maps hold.
    let mut entities = a_yard();
    entities.push(Entity::at("prop_tree", [40.0, 40.0, 0.0], 7));
    entities.push(Entity::at("prop_tree", [40.0, 40.0, 0.0], 7));
    entities.push(Entity::at("prop_tree", [44.0, 40.0, 0.0], 7));
    entities.push(Entity::at("prop_tree", [48.0, 40.0, 0.0], 0));
    let base = map(&entities);
    let changes = map_changes(&base, &base.clone());
    assert_eq!(
        changes,
        MapChanges {
            unchanged: entities.len(),
            ..MapChanges::default()
        }
    );
    assert!(changes.is_empty());

    // A number that is not one is the same number in both.
    let mut odd = base.clone();
    odd.entities[2].position.x = f32::NAN;
    odd.entities[9].lod_dist = f32::NAN;
    assert!(map_changes(&odd, &odd.clone()).is_empty());
    assert!(map_changes(&CMapData::default(), &CMapData::default()).is_empty());
}

/// A base and a changed map that hold one of everything: a move, a deletion, an addition, a parent taken out, twins of which one moved, and two trees no guid finds that stand nearest to one.
fn one_of_everything() -> (CMapData, CMapData) {
    let mut entities = a_yard();
    entities.push(Entity::at("prop_tree", [40.0, 40.0, 0.0], 7));
    entities.push(Entity::at("prop_tree", [44.0, 40.0, 0.0], 7));
    entities.push(Entity::at("prop_bench", [60.0, 60.0, 0.0], 20));
    entities.push(Entity::at("prop_bench", [61.0, 60.0, 0.0], 21));
    let base = map(&entities);

    let mut now = base.clone();
    let pose = pose_of(&now.entities[1]);
    place_entity(
        &mut now,
        1,
        Pose {
            position: pose.position + Vec3::X,
            ..pose
        },
    )
    .unwrap();
    now.entities[8].position.y = 47.0;
    now.entities[9].guid = 120;
    now.entities[9].position.x = 60.25;
    remove_entity(&mut now, 10).unwrap();
    remove_entity(&mut now, 3).unwrap();
    let donor = now.entities[2].clone();
    let stands = Pose {
        position: Vec3::new(70.0, 70.0, 0.0),
        ..pose_of(&donor)
    };
    add_entity(&mut now, &donor, Name::from_text("prop_crate_b"), stands).unwrap();
    (base, now)
}

#[test]
fn comparing_in_the_other_direction_swaps_what_was_added_and_what_was_deleted_exactly() {
    let (base, now) = one_of_everything();
    let forth = map_changes(&base, &now);
    let back = map_changes(&now, &base);

    assert!(!forth.added.is_empty() && !forth.deleted.is_empty());
    assert!(!forth.changed.is_empty() && !forth.ambiguous.is_empty());
    assert_eq!(forth.added, back.deleted);
    assert_eq!(forth.deleted, back.added);
    assert_eq!(forth.unchanged, back.unchanged);
    assert_eq!(forth.changed.len(), back.changed.len());
    for (kept, mirror) in forth.changed.iter().zip(&back.changed_by_now()) {
        assert_eq!((&kept.base, &kept.now), (&mirror.now, &mirror.base));
        assert_eq!(kept.matched, mirror.matched);
        assert_eq!(
            kept.changes.len(),
            mirror.changes.len(),
            "{kept:?} / {mirror:?}"
        );
        for (change, other) in kept.changes.iter().zip(&mirror.changes) {
            match (change, other) {
                (
                    EntityChange::Moved { from, to },
                    EntityChange::Moved {
                        from: back_from,
                        to: back_to,
                    },
                ) => assert_eq!((from, to), (back_to, back_from)),
                (
                    EntityChange::Parent { from, to },
                    EntityChange::Parent {
                        from: back_from,
                        to: back_to,
                    },
                ) => assert_eq!((from, to), (back_to, back_from)),
                (
                    EntityChange::Guid { from, to },
                    EntityChange::Guid {
                        from: back_from,
                        to: back_to,
                    },
                ) => assert_eq!((from, to), (back_to, back_from)),
                other => panic!("the two directions name another change: {other:?}"),
            }
        }
    }
    assert_eq!(forth.ambiguous.len(), back.ambiguous.len());
    for (group, mirror) in forth.ambiguous.iter().zip(&back.ambiguous) {
        assert_eq!(
            (&group.archetype, &group.base, &group.now),
            (&mirror.archetype, &mirror.now, &mirror.base)
        );
    }

    // What the first direction holds, so the mirror above is a mirror of something.
    assert_eq!(
        indexes(&forth),
        (vec![9], vec![3, 10], vec![(1, 1), (4, 3), (5, 4), (9, 8)])
    );
    assert_eq!(forth.changed[3].matched, Matched::Nearest);
}

/// The kept entities of a comparison in the order of the other direction: by the index each has in the changed map.
trait ByNow {
    fn changed_by_now(&self) -> Vec<or_maps::changes::Kept>;
}

impl ByNow for MapChanges {
    fn changed_by_now(&self) -> Vec<or_maps::changes::Kept> {
        let mut kept = self.changed.clone();
        kept.sort_by_key(|k| k.now.index);
        kept
    }
}

fn entity(archetype: &str, guid: u32, position: [f32; 3]) -> CEntityDef {
    let mut entity = CEntityDef::default();
    entity.archetype_name = HashedString::new(archetype);
    entity.guid = guid;
    entity.position.x = position[0];
    entity.position.y = position[1];
    entity.position.z = position[2];
    entity.rotation.w = 1.0;
    entity.scale_xy = 1.0;
    entity.scale_z = 1.0;
    entity.parent_index = -1;
    entity
}

#[test]
fn a_turn_is_the_rotation_that_places_it_which_an_entity_stores_conjugated_and_an_interior_as_it_is(
) {
    let (was, is) = (
        Quat::from_rotation_z(0.4) * Quat::from_rotation_x(0.2),
        Quat::from_rotation_y(1.1) * Quat::from_rotation_z(-0.7),
    );
    assert!(!was.abs_diff_eq(was.conjugate(), 0.1) && !is.abs_diff_eq(is.conjugate(), 0.1));
    for interior in [false, true] {
        let record = |rotation: Quat| {
            let mut held = entity("v_shop", 1, [0.0; 3]);
            let kept = if interior {
                rotation
            } else {
                rotation.conjugate()
            };
            held.rotation.x = kept.x;
            held.rotation.y = kept.y;
            held.rotation.z = kept.z;
            held.rotation.w = kept.w;
            if interior {
                CEntityDefAny::CMloInstanceDef(CMloInstanceDef {
                    base: held,
                    ..CMloInstanceDef::default()
                })
            } else {
                CEntityDefAny::Base(held)
            }
        };
        let changes = entity_changes(&[record(was)], &[record(is)]);
        let [EntityChange::Turned { from, to }] = changes.changed[0].changes.as_slice() else {
            panic!("{changes:?}");
        };
        assert!(
            Quat::from_array(*from).abs_diff_eq(was, 1e-6),
            "interior {interior}: it was turned {was:?} and is said to have been turned {from:?}"
        );
        assert!(
            Quat::from_array(*to).abs_diff_eq(is, 1e-6),
            "interior {interior}: it is turned {is:?} and is said to be turned {to:?}"
        );
    }

    // An entity that states no rotation, as `0, 0, 0, 1`, is said to have stood at none, with no zero below zero in it.
    let unturned = CEntityDefAny::Base(entity("v_shop", 1, [0.0; 3]));
    let mut turned = unturned.clone();
    turned.rotation.z = was.z;
    turned.rotation.w = was.w;
    let changes = entity_changes(&[unturned], &[turned]);
    let [EntityChange::Turned { from, .. }] = changes.changed[0].changes.as_slice() else {
        panic!("{changes:?}");
    };
    assert_eq!(
        from.map(f32::to_bits),
        [0.0f32, 0.0, 0.0, 1.0].map(f32::to_bits),
        "{from:?}"
    );
}

#[test]
fn a_name_has_its_text_only_where_the_text_hashes_to_it() {
    let named = entity("prop_crate_a", 1, [0.0; 3]);
    // A reader's index of names holds two ways of hashing, so a name can arrive with a text that is not the text of its hash.
    let mut misnamed = entity("prop_crate_a", 2, [0.0; 3]);
    misnamed.archetype_name.raw = "prop_crate_b".to_string();
    let mut unnamed = entity("prop_crate_a", 3, [0.0; 3]);
    unnamed.archetype_name = HashedString::from_hash(hash("prop_crate_a"));
    let changes = entity_changes(&[], &[named, misnamed, unnamed].map(CEntityDefAny::Base));
    let names: Vec<&Name> = changes.added.iter().map(|e| &e.archetype).collect();
    assert_eq!(
        names,
        [
            &Name::from_text("prop_crate_a"),
            &Name::of(hash("prop_crate_a")),
            &Name::of(hash("prop_crate_a")),
        ]
    );
}

#[test]
fn every_member_a_comparison_names_is_one_the_typed_record_names() {
    // An interior instance that states another thing in every member the records type.
    let base = CMloInstanceDef {
        base: entity("v_interior", 1, [0.0; 3]),
        ..CMloInstanceDef::default()
    };
    let mut now = base.clone();
    now.base.child_lod_dist = 9.0;
    now.base.num_children = 3;
    now.base.ambient_occlusion_multiplier = 1;
    now.base.artificial_ambient_occlusion = 2;
    now.base.tint_value = 3;
    now.group_id = 4;
    now.floor_id = 5;
    now.default_entity_sets = vec![HashedString::new("set_a")];
    now.num_exit_portals = 6;
    now.mlo_instflags = 7;
    let changes = entity_changes(
        &[CEntityDefAny::CMloInstanceDef(base.clone())],
        &[CEntityDefAny::CMloInstanceDef(now)],
    );
    let [EntityChange::Members { names }] = changes.changed[0].changes.as_slice() else {
        panic!("{changes:?}");
    };
    assert_eq!(names.len(), 10, "one name for each member changed");
    for name in names {
        assert!(
            CEntityDef::MEMBERS.contains(name) || CMloInstanceDef::MEMBERS.contains(name),
            "{name} is no member of an entity"
        );
    }

    // An entity that is an interior instance in one file and not in the other is of another type.
    let changes = entity_changes(
        &[CEntityDefAny::CMloInstanceDef(base.clone())],
        &[CEntityDefAny::Base(base.base.clone())],
    );
    assert_eq!(changes.changed[0].changes, [EntityChange::Other]);

    // An archetype that states another thing in every member.
    let mut held = CTimeArchetypeDef::default();
    held.base.name = HashedString::new("prop_clock");
    let mut other = held.clone();
    other.base.lod_dist = 1.0;
    other.base.flags = 2;
    other.base.special_attribute = 3;
    other.base.bb_min.x = -4.0;
    other.base.bb_max.x = 5.0;
    other.base.bs_centre.y = 6.0;
    other.base.bs_radius = 7.0;
    other.base.hd_texture_dist = 8.0;
    other.base.texture_dictionary = HashedString::new("a");
    other.base.clip_dictionary = HashedString::new("b");
    other.base.drawable_dictionary = HashedString::new("c");
    other.base.physics_dictionary = HashedString::new("d");
    other.base.asset_name = HashedString::new("e");
    other.time_flags = 9;
    let (mut base, mut now) = (CMapTypes::default(), CMapTypes::default());
    base.archetypes
        .push(CBaseArchetypeDefAny::CTimeArchetypeDef(held.clone()));
    now.archetypes
        .push(CBaseArchetypeDefAny::CTimeArchetypeDef(other));
    let changes = types_changes(&base, &now);
    let members: Vec<&str> = (changes.changed[0].changes.iter())
        .map(|c| c.member)
        .collect();
    assert_eq!(members.len(), 14, "one name for each member changed");
    for name in members {
        assert!(
            CBaseArchetypeDef::MEMBERS.contains(&name)
                || CTimeArchetypeDef::MEMBERS.contains(&name),
            "{name} is no member of an archetype"
        );
    }
    assert!(!changes.changed[0].other);

    // A timed archetype that is a plain one now is of another type.
    now.archetypes[0] = CBaseArchetypeDefAny::Base(held.base.clone());
    let changes = types_changes(&base, &now);
    let change = &changes.changed[0].changes[0];
    assert_eq!(change.member, "type");
    assert_eq!(change.from, Stated::Text("CTimeArchetypeDef".into()));
    assert_eq!(change.to, Stated::Text("CBaseArchetypeDef".into()));
}

/// A map of one entity that states `priority` in a member no typed record names, and that states `lods` of itself in another.
fn a_map_with_members_no_record_types(priority: u64, lods: u64) -> CMapData {
    const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <s32 name="parentIndex"/>
    <u32 name="priorityLevel"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <u32 name="containerLods"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;
    let mut entity = MetaInstance::new("CEntityDef");
    entity.set(
        "archetypeName",
        Value::Hash(HashedString::new("prop_crate_a")),
    );
    entity.set("guid", Value::UInt(1));
    entity.set("position", Value::Vec3([1.0, 2.0, 3.0]));
    entity.set("parentIndex", Value::Int(-1));
    entity.set("priorityLevel", Value::UInt(priority));
    assert!(
        !CEntityDef::MEMBERS.contains(&"priorityLevel")
            && !CMapData::MEMBERS.contains(&"containerLods"),
        "the members are ones the typed records carry and do not name"
    );
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new("harbour")));
    root.set("containerLods", Value::UInt(lods));
    root.set(
        "entities",
        Value::Array(vec![Value::Pointer(Some(Box::new(entity)))]),
    );
    let library = build_library([("inline.psc", PSC.as_bytes())]).unwrap();
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).unwrap();
    let bytes = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).unwrap();
    CMapData::from_bytes(&bytes).unwrap()
}

#[test]
fn what_no_typed_member_names_is_said_to_differ_and_not_named() {
    let base = a_map_with_members_no_record_types(1, 1);
    assert!(map_changes(&base, &a_map_with_members_no_record_types(1, 1)).is_empty());
    let changes = map_changes(&base, &a_map_with_members_no_record_types(2, 1));
    assert_eq!(indexes(&changes), (vec![], vec![], vec![(0, 0)]));
    assert_eq!(changes.changed[0].changes, [EntityChange::Other]);
    assert!(changes.root.is_empty() && !changes.other, "{changes:?}");

    // The same of the map itself: its entity states what it stated, and the map is said to differ in what no member names.
    let changes = map_changes(&base, &a_map_with_members_no_record_types(1, 2));
    assert_eq!(indexes(&changes), (vec![], vec![], vec![]));
    assert!(changes.root.is_empty() && changes.other, "{changes:?}");
    assert!(!changes.is_empty());
}

#[test]
fn what_a_file_states_of_itself_is_named_member_by_member_beside_what_it_holds() {
    let base = map(&a_yard());
    let mut now = base.clone();
    now.content_flags = 66;
    now.entities_extents_max.x = 120.0;
    let changes = map_changes(&base, &now);
    assert!(!changes.is_empty());
    assert_eq!(indexes(&changes), (vec![], vec![], vec![]));
    assert_eq!(changes.unchanged, base.entities.len());
    let members: Vec<&str> = changes.root.iter().map(|c| c.member).collect();
    assert_eq!(members, ["contentFlags", "entitiesExtentsMax"]);
    assert_eq!(
        (&changes.root[0].from, &changes.root[0].to),
        (&Stated::Number(65), &Stated::Number(66))
    );
    assert_eq!(
        (&changes.root[1].from, &changes.root[1].to),
        (
            &Stated::Vector([100.0; 3]),
            &Stated::Vector([120.0, 100.0, 100.0])
        )
    );
    assert!(!changes.other);

    // Every member the root types but its entities, each by a name the record has.
    now.name = HashedString::new("quay");
    now.parent = HashedString::new("quay_lod");
    now.flags = 2;
    now.streaming_extents_min.x = -9.0;
    now.streaming_extents_max.y = 109.0;
    now.entities_extents_min.z = -1.0;
    now.physics_dictionaries
        .push(HashedString::new("quay_collision"));
    let changes = map_changes(&base, &now);
    // ⚠ The records a map holds are typed and not named member by member: its entities are compared apart, and the rest
    // is said to differ.
    const HELD: [&str; 7] = [
        "entities",
        "boxOccluders",
        "occludeModels",
        "timeCycleModifiers",
        "carGenerators",
        "LODLightsSOA",
        "DistantLODLightsSOA",
    ];
    assert_eq!(changes.root.len(), CMapData::MEMBERS.len() - HELD.len());
    for change in &changes.root {
        assert!(
            CMapData::MEMBERS.contains(&change.member) && !HELD.contains(&change.member),
            "{} is no member of a map",
            change.member
        );
    }
    assert!(HELD.iter().all(|held| CMapData::MEMBERS.contains(held)));
    assert_eq!(indexes(&changes), (vec![], vec![], vec![]));
    assert!(!changes.other);

    let mut parked = base.clone();
    parked.car_generators.push(or_io::meta::CCarGen::default());
    let changes = map_changes(&base, &parked);
    assert!(changes.root.is_empty() && changes.other, "{changes:?}");

    // And of a type file: its name and the files it depends on.
    let base = types(&[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])]);
    let mut now = base.clone();
    now.name = HashedString::new("quay_types");
    now.dependencies.push(HashedString::new("harbour_props"));
    let changes = types_changes(&base, &now);
    assert!(!changes.is_empty() && !changes.other);
    assert_eq!(changes.unchanged, 1);
    assert_eq!(changes.root.len(), CMapTypes::MEMBERS.len() - 1);
    let members: Vec<&str> = changes.root.iter().map(|c| c.member).collect();
    assert_eq!(members, ["name", "dependencies"]);
    assert_eq!(changes.root[1].to, Stated::Text("harbour_props".into()));
    let other = types_changes(
        &a_type_file_with_members_no_record_types(1, 1),
        &a_type_file_with_members_no_record_types(1, 2),
    );
    assert!(other.other && other.root.is_empty() && other.changed.is_empty());
    assert_eq!(other.unchanged, 1);
}

fn types(archetypes: &[Archetype<'_>]) -> CMapTypes {
    CMapTypes::from_bytes(&types_file("harbour_types", archetypes)).unwrap()
}

#[test]
fn a_type_file_says_which_archetype_was_added_which_deleted_and_which_member_of_one_changed() {
    let base = types(&[
        Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
        Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        Archetype::new("prop_crate_c", [-3.0; 3], [3.0; 3]),
    ]);
    // The first is gone, so the others have another index; one of them has a taller box, and one is new.
    let now = types(&[
        Archetype::new("prop_crate_b", [-2.0; 3], [2.0, 2.0, 4.5]),
        Archetype::new("prop_crate_c", [-3.0; 3], [3.0; 3]),
        Archetype::new("prop_crate_d", [-1.0; 3], [1.0; 3]),
    ]);
    let changes = types_changes(&base, &now);
    assert_eq!(changes.unchanged, 1);
    assert!(changes.ambiguous.is_empty());
    assert_eq!(
        (changes.deleted.len(), changes.deleted[0].index),
        (1, 0),
        "{changes:?}"
    );
    assert_eq!(changes.deleted[0].name.hash, hash("prop_crate_a"));
    assert_eq!((changes.added.len(), changes.added[0].index), (1, 2));
    assert_eq!(changes.added[0].name.hash, hash("prop_crate_d"));

    assert_eq!(changes.changed.len(), 1);
    let kept = &changes.changed[0];
    assert_eq!((kept.base.index, kept.now.index), (1, 0));
    assert_eq!(kept.base.name.hash, hash("prop_crate_b"));
    assert!(!kept.other && kept.entities.is_none());
    // The sample states the centre of the box too, so a taller box moves it.
    let members: Vec<&str> = kept.changes.iter().map(|c| c.member).collect();
    assert_eq!(members, ["bbMax", "bsCentre"]);
    assert_eq!(kept.changes[0].from, Stated::Vector([2.0; 3]));
    assert_eq!(kept.changes[0].to, Stated::Vector([2.0, 2.0, 4.5]));

    let same = types_changes(&base, &base.clone());
    assert!(same.is_empty());
    assert_eq!(same.unchanged, 3);

    // A name declared twice is told apart by nothing once one of the two changed.
    let twice = types(&[
        Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
        Archetype::new("prop_crate_a", [-2.0; 3], [2.0; 3]),
    ]);
    assert!(types_changes(&twice, &twice.clone()).is_empty());
    let mut other = twice.clone();
    other.archetypes[1].bb_max.z = 9.0;
    let changes = types_changes(&twice, &other);
    assert!(changes.changed.is_empty() && changes.added.is_empty());
    assert_eq!(changes.ambiguous.len(), 1);
    assert_eq!(changes.ambiguous[0].base, [0, 1]);
    assert_eq!(changes.ambiguous[0].now, [0, 1]);
}

#[test]
fn an_interior_says_what_its_entities_changed_and_its_own_members_beside_them() {
    let mut held = CMloArchetypeDef::default();
    held.base.name = HashedString::new("v_shop");
    held.mlo_flags = 1;
    held.entities = vec![
        CEntityDefAny::Base(entity("prop_chair", 1, [0.0, 0.0, 0.0])),
        CEntityDefAny::Base(entity("prop_table", 2, [1.0, 0.0, 0.0])),
        CEntityDefAny::Base(entity("prop_lamp", 3, [2.0, 0.0, 0.0])),
    ];
    let mut other = held.clone();
    other.entities[2].position.z = 1.5;
    other.entities.remove(0);
    let (mut base, mut now) = (CMapTypes::default(), CMapTypes::default());
    base.archetypes
        .push(CBaseArchetypeDefAny::CMloArchetypeDef(held.clone()));
    now.archetypes
        .push(CBaseArchetypeDefAny::CMloArchetypeDef(other.clone()));

    // Only its entities differ: no member of the interior is named, and nothing else of it is said to differ.
    let changes = types_changes(&base, &now);
    assert_eq!(changes.changed.len(), 1, "{changes:?}");
    let kept = &changes.changed[0];
    assert!(kept.changes.is_empty() && !kept.other, "{kept:?}");
    let inside = kept.entities.as_ref().expect("its entities changed");
    assert_eq!(indexes(inside), (vec![], vec![0], vec![(2, 1)]));
    assert_eq!(inside.unchanged, 1);
    assert_eq!(
        inside.changed[0].changes,
        [EntityChange::Moved {
            from: [2.0, 0.0, 0.0],
            to: [2.0, 0.0, 1.5],
        }]
    );

    // Only a member of its own differs: it is named, and its entities are not said to have changed.
    other.entities = held.entities.clone();
    other.mlo_flags = 2;
    now.archetypes[0] = CBaseArchetypeDefAny::CMloArchetypeDef(other);
    let changes = types_changes(&base, &now);
    let kept = &changes.changed[0];
    let members: Vec<&str> = kept.changes.iter().map(|c| c.member).collect();
    assert_eq!(members, ["mloFlags"]);
    assert!(CMloArchetypeDef::MEMBERS.contains(&"mloFlags"));
    assert_eq!(
        (&kept.changes[0].from, &kept.changes[0].to),
        (&Stated::Number(1), &Stated::Number(2))
    );
    assert!(kept.entities.is_none() && !kept.other, "{kept:?}");
}

/// A type file of one archetype that states `asset_type` in a member no typed record names, and that states `composites` of itself in another.
fn a_type_file_with_members_no_record_types(asset_type: u64, composites: u64) -> CMapTypes {
    const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CBaseArchetypeDef">
    <string name="name" type="atHashString"/>
    <u32 name="assetType"/>
  </structdef>
  <structdef type="CMapTypes">
    <string name="name" type="atHashString"/>
    <u32 name="compositeEntityTypes"/>
    <array name="archetypes" type="atArray"><pointer type="CBaseArchetypeDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;
    let mut archetype = MetaInstance::new("CBaseArchetypeDef");
    archetype.set("name", Value::Hash(HashedString::new("prop_crate_a")));
    archetype.set("assetType", Value::UInt(asset_type));
    assert!(
        !CBaseArchetypeDef::MEMBERS.contains(&"assetType")
            && !CMapTypes::MEMBERS.contains(&"compositeEntityTypes"),
        "the members are ones the typed records carry and do not name"
    );
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new("harbour_types")));
    root.set("compositeEntityTypes", Value::UInt(composites));
    root.set(
        "archetypes",
        Value::Array(vec![Value::Pointer(Some(Box::new(archetype)))]),
    );
    let library = build_library([("inline.psc", PSC.as_bytes())]).unwrap();
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).unwrap();
    let bytes = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).unwrap();
    CMapTypes::from_bytes(&bytes).unwrap()
}

#[test]
fn what_no_typed_member_of_an_archetype_names_is_said_to_differ_and_not_named() {
    let base = a_type_file_with_members_no_record_types(1, 1);
    assert!(types_changes(&base, &a_type_file_with_members_no_record_types(1, 1)).is_empty());
    let changes = types_changes(&base, &a_type_file_with_members_no_record_types(2, 1));
    assert_eq!(changes.changed.len(), 1, "{changes:?}");
    assert!(changes.changed[0].other);
    assert!(changes.changed[0].changes.is_empty() && changes.changed[0].entities.is_none());
    assert!(changes.root.is_empty() && !changes.other);
}

#[test]
fn the_archetypes_of_a_type_file_are_listed_in_the_order_of_their_file() {
    let names = ["prop_a", "prop_b", "prop_c", "prop_d", "prop_e", "prop_f"];
    let ascending = |of: &[&str]| of.windows(2).all(|pair| hash(pair[0]) < hash(pair[1]));
    assert!(
        !ascending(&names[..3]) && !ascending(&names[3..]),
        "the order of the names is not the order of their hashes, which would hide a list kept in that order"
    );
    let boxed = |name: &'static str, top: f32| Archetype::new(name, [-1.0; 3], [1.0, 1.0, top]);
    let first = types(&names[..3].iter().map(|n| boxed(n, 1.0)).collect::<Vec<_>>());
    let second = types(&names[3..].iter().map(|n| boxed(n, 1.0)).collect::<Vec<_>>());
    let changes = types_changes(&first, &second);
    let at = |held: &[or_maps::changes::ArchetypeRef]| -> Vec<usize> {
        held.iter().map(|a| a.index).collect()
    };
    assert_eq!(at(&changes.deleted), [0, 1, 2]);
    assert_eq!(at(&changes.added), [0, 1, 2]);

    let taller = types(&names[..3].iter().map(|n| boxed(n, 2.0)).collect::<Vec<_>>());
    let changes = types_changes(&first, &taller);
    let kept: Vec<usize> = changes.changed.iter().map(|k| k.base.index).collect();
    assert_eq!(kept, [0, 1, 2]);
}

/// A map set holding a map file of `entities` and a type file of two crates, and the folders of the two sources.
fn sources(dir: &Path, entities: &[Entity<'_>]) -> (PathBuf, PathBuf) {
    let folder = dir.join("maps");
    let map = map_file("harbour", entities, ([0.0; 3], [100.0; 3]));
    let types = types_file(
        "harbour_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        ],
    );
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: "harbour.ymap",
                bytes: &map,
                from_game: true,
                virtual_path: Some("x64a.rpf/levels/harbour.ymap"),
            },
            Incoming {
                name: "harbour_types.ytyp",
                bytes: &types,
                from_game: true,
                virtual_path: Some("x64a.rpf/levels/harbour_types.ytyp"),
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    (
        folder.join("harbour.metadata.src"),
        folder.join("harbour_types.metadata.src"),
    )
}

#[test]
fn a_source_compared_with_the_base_it_keeps_says_what_its_xml_changed() {
    let scratch = tempfile::tempdir().unwrap();
    let (map_dir, types_dir) = sources(scratch.path(), &a_yard());
    let SourceChanges::Map(untouched) = source_changes(&map_dir).unwrap() else {
        panic!("a map source compares as a map");
    };
    assert_eq!(
        untouched,
        MapChanges {
            unchanged: a_yard().len(),
            ..MapChanges::default()
        },
        "a source as it was imported states what its base states"
    );
    assert!(source_changes(&types_dir).unwrap().is_empty());

    // One moved, one taken out, one added: exactly those three.
    let mut stands = None;
    edit_map(&map_dir, |map| {
        let pose = pose_of(&map.entities[6]);
        place_entity(
            map,
            6,
            Pose {
                position: pose.position + Vec3::new(0.0, 3.0, 0.0),
                ..pose
            },
        )?;
        remove_entity(map, 2)?;
        let donor = map.entities[0].clone();
        let pose = Pose {
            position: Vec3::new(50.0, 50.0, 0.0),
            ..pose_of(&donor)
        };
        stands = Some(pose.position);
        add_entity(map, &donor, Name::from_text("prop_crate_b"), pose).map(|_| ())
    })
    .unwrap();
    let SourceChanges::Map(changes) = source_changes(&map_dir).unwrap() else {
        panic!("a map source compares as a map");
    };
    assert_eq!(indexes(&changes), (vec![6], vec![2], vec![(6, 5)]));
    assert_eq!(changes.unchanged, a_yard().len() - 2);
    assert!(changes.ambiguous.is_empty());
    assert_eq!(
        changes.changed[0].changes,
        [EntityChange::Moved {
            from: [10.0, 14.0, 0.0],
            to: [10.0, 17.0, 0.0],
        }]
    );
    assert_eq!(changes.added[0].position, stands.unwrap().to_array());

    // A source that is no source, and one that cooks to no map, are refused with the reason.
    let refused = source_changes(&scratch.path().join("maps")).unwrap_err();
    assert!(!refused.to_string().is_empty());
    std::fs::write(map_dir.join("metadata.xml"), "not a map").unwrap();
    let refused = source_changes(&map_dir).unwrap_err();
    assert!(
        refused.to_string().starts_with("the source does not cook"),
        "{refused}"
    );

    // A metadata source of another kind of file is named as neither, whatever its bytes are.
    let other = or_source::metadata::extract(
        &map_file("harbour", &a_yard(), ([0.0; 3], [100.0; 3])),
        "harbour",
        "ymt",
        scratch.path(),
        &or_source::models::Extraction {
            from_game: true,
            virtual_path: None,
            textures: false,
            skeleton: None,
        },
    )
    .unwrap();
    let refused = source_changes(&other).unwrap_err();
    assert!(
        matches!(&refused, Error::Kind(file) if file == "harbour.ymt"),
        "{refused}"
    );
    assert_eq!(
        refused.to_string(),
        "harbour.ymt is neither a map file nor a type file"
    );
}

/// Trees scattered over a square of ground whose side grows with their number, each with a guid of its own.
fn scattered(count: usize, guid_of: impl Fn(usize) -> u32, nudge: f32) -> Vec<CEntityDefAny> {
    let side = (count as f32).sqrt() * 10.0;
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    (0..count)
        .map(|i| {
            let position = [next() * side + nudge, next() * side, 0.0];
            let archetype = ["prop_tree_a", "prop_tree_b", "prop_tree_c"][i % 3];
            CEntityDefAny::Base(entity(archetype, guid_of(i), position))
        })
        .collect()
}

/// A comparison a measure runs.
type Compare = Box<dyn Fn() -> MapChanges>;

/// The shortest run of each of `few` and `many` over nine rounds at most, timed turn about so that both meet the same load.
fn shortest(few: &dyn Fn() -> MapChanges, many: &dyn Fn() -> MapChanges) -> (Duration, Duration) {
    let time = |compare: &dyn Fn() -> MapChanges| {
        let start = Instant::now();
        let changes = compare();
        let took = start.elapsed();
        assert!(changes.added.is_empty() && changes.ambiguous.is_empty());
        took
    };
    // ⚠ No round is started after a second, so a comparison that is slow reaches the bound and is not left to the test's timeout.
    let started = Instant::now();
    let (mut a, mut b) = (time(few), time(many));
    for _ in 1..9 {
        if started.elapsed() > Duration::from_secs(1) {
            break;
        }
        (a, b) = (a.min(time(few)), b.min(time(many)));
    }
    (a, b)
}

/// Four times the entities must not cost sixteen times the time: the bound is ten times, where a comparison of each entity with each
/// other measures sixteen and this one between four and six. Built by the `dev` profile, which optimises the workspace's crates at
/// `opt-level = 1` (with LLVM on Apple silicon and Cranelift elsewhere).
///
/// ⚠ The machine is shared, so a time is the shortest of nine runs, the two sizes are timed turn about, and a measure past the bound
/// is taken again, twice at most: a comparison that grows with the square is past it every time.
#[test]
fn several_thousand_entities_compare_in_time_that_does_not_grow_with_the_square_of_their_number() {
    let (small, large) = (4_000, 16_000);
    // Found by its guid, with every third one moved: the common case.
    let by_guid = |count: usize| {
        let base = scattered(count, |i| i as u32 + 1, 0.0);
        let mut now = base.clone();
        for entity in now.iter_mut().step_by(3) {
            entity.position.z += 1.0;
        }
        move || {
            let changes = entity_changes(&base, &now);
            assert_eq!(changes.changed.len(), count.div_ceil(3));
            assert_eq!(changes.unchanged, count - count.div_ceil(3));
            changes
        }
    };
    // No guid finds any, and every one stands a little further on: each is found as the nearest of its archetype.
    let by_place = |count: usize| {
        let base = scattered(count, |i| i as u32 + 1, 0.0);
        let now = scattered(count, |i| (i + count) as u32 + 1, 0.01);
        move || {
            let changes = entity_changes(&base, &now);
            assert_eq!(changes.changed.len(), count, "each tree is found");
            assert!(changes.deleted.is_empty());
            assert!((changes.changed.iter())
                .all(|k| k.matched == Matched::Nearest && k.base.index == k.now.index));
            changes
        }
    };
    let cases: [(&str, Compare, Compare); 2] = [
        (
            "by guid",
            Box::new(by_guid(small)),
            Box::new(by_guid(large)),
        ),
        (
            "by place",
            Box::new(by_place(small)),
            Box::new(by_place(large)),
        ),
    ];
    for (what, few, many) in &cases {
        let mut measures = 1;
        let (mut few_took, mut many_took) = shortest(few, many);
        while many_took >= few_took * 10 && measures < 3 {
            (few_took, many_took) = shortest(few, many);
            measures += 1;
        }
        assert!(
            many_took < few_took * 10,
            "{what}: {small} entities took {few_took:?} and {large} took {many_took:?}, more than ten times as long, in each of {measures} measures"
        );
    }
}

/// A pack of `block_lod`, three far models, and `block`, the map under it: a house under the first far model, two under the
/// second and a shed under the third.
fn a_block_under_its_far_models(dir: &Path) -> PathBuf {
    let extents = ([0.0; 3], [100.0; 3]);
    let far = |name, x: f32, guid, children| Entity {
        children,
        ..Entity::at(name, [x, 0.0, 0.0], guid)
    };
    let under = |name, x: f32, guid, parent| Entity {
        parent,
        in_parent_map: true,
        ..Entity::at(name, [x, 5.0, 0.0], guid)
    };
    let lod = map_file(
        "block_lod",
        &[
            far("far_a", 10.0, 1, 1),
            far("far_b", 20.0, 2, 2),
            far("far_c", 30.0, 3, 1),
        ],
        extents,
    );
    let block = child_map_file(
        "block",
        "block_lod",
        &[
            under("house_a", 10.0, 11, 0),
            under("house_b", 20.0, 12, 1),
            under("house_c", 22.0, 13, 1),
            under("shed", 30.0, 14, 2),
        ],
        extents,
    );
    let folder = dir.join("maps");
    let incoming =
        [("block_lod.ymap", &lod), ("block.ymap", &block)].map(|(name, bytes)| Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        });
    let added = add_files(&folder, &incoming, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

#[test]
fn a_link_into_the_parent_map_of_a_pack_is_the_entity_it_names_there_and_not_its_number() {
    const FAR: &str = "block_lod.metadata.src";
    const NEAR: &str = "block.metadata.src";
    let map_changes_of = |folder: &Path, path: &str| match PackSources::read(folder, [FAR, NEAR])
        .changes(path)
        .unwrap()
    {
        SourceChanges::Map(changes) => changes,
        SourceChanges::Types(_) => panic!("{path} is a map file"),
    };

    // The first house deleted with its chain: its far model goes, and the links of the others are renumbered to the far models
    // they named, which is no change of theirs.
    let scratch = tempfile::tempdir().unwrap();
    let folder = a_block_under_its_far_models(scratch.path());
    let chosen = or_maps::operation::At {
        source: NEAR.to_string(),
        index: 0,
    };
    let planned = or_maps::delete::plan(&folder, &[chosen]).unwrap();
    or_maps::delete::apply(&folder, &planned).unwrap();
    let near = map_changes_of(&folder, NEAR);
    assert_eq!(indexes(&near), (vec![], vec![0], vec![]), "{near:?}");
    assert_eq!(near.unchanged, 3);
    let far = map_changes_of(&folder, FAR);
    assert_eq!(indexes(&far), (vec![], vec![0], vec![]), "{far:?}");
    // Compared alone, a source has no parent map in hand and its links are their numbers.
    let SourceChanges::Map(alone) = source_changes(&folder.join(NEAR)).unwrap() else {
        panic!("a map file");
    };
    assert_eq!(
        indexes(&alone),
        (vec![], vec![0], vec![(1, 0), (2, 1), (3, 2)])
    );

    // The first far model taken out alone: no byte of the map under it changed, and each of its entities names another far model,
    // the house of the far model that went and the shed whose link is past the end among them.
    let scratch = tempfile::tempdir().unwrap();
    let folder = a_block_under_its_far_models(scratch.path());
    let before = std::fs::read(folder.join(NEAR).join("metadata.xml")).unwrap();
    let alone = or_maps::operation::Operation::RemoveEntity {
        source: FAR.into(),
        expects: None,
        entity: 0,
        extents: None,
    };
    or_maps::operation::apply(&folder, &alone).unwrap();
    assert_eq!(
        std::fs::read(folder.join(NEAR).join("metadata.xml")).unwrap(),
        before
    );
    let near = map_changes_of(&folder, NEAR);
    assert_eq!(
        indexes(&near),
        (vec![], vec![], vec![(0, 0), (1, 1), (2, 2), (3, 3)]),
        "{near:?}"
    );
    for (kept, link) in near.changed.iter().zip([0, 1, 1, 2]) {
        let same_number = EntityChange::Parent {
            from: link,
            to: link,
        };
        assert_eq!(kept.changes, [same_number]);
    }
}
