//! What an operation does where the pack holds less than it reads: an entity of an archetype no type file of the pack declares, the
//! last entity or archetype of a file, and the first of a file that holds none.
//!
//! ⚠ The files are the crate's samples and what `or_world_gta5::create` makes, so the clean checkout runs this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glam::Vec3;
use or_io::meta::BinaryFormat;
use or_maps::edit::{read_archetype, read_entity};
use or_maps::import::{add_files, Incoming};
use or_maps::model::Maps;
use or_maps::operation::{apply, apply_all, At, Operation, PoseData};
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_maps::{check, inspect, Depth};
use or_world_gta5::archetypes::ArchetypeIndex;
use or_world_gta5::create;

const HARBOUR: &str = "harbour.metadata.src";
const YARD: &str = "yard.metadata.src";
const TYPES: &str = "harbour_types.metadata.src";
const BENCH: &str = "prop_bench_01a";

/// A pack of the files `files`, by the name each ships under.
fn pack(dir: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let folder = dir.join("maps");
    let incoming: Vec<Incoming<'_>> = (files.iter())
        .map(|(name, bytes)| Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        })
        .collect();
    let added = add_files(&folder, &incoming, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

/// Every file under `folder`, by its path.
fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, into);
            } else {
                into.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, &mut found);
    found
}

fn pose(position: [f32; 3]) -> PoseData {
    PoseData {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
    }
}

fn crate_types() -> Vec<u8> {
    types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
    )
}

/// The two boxes the map the source `source` cooks to states: its entities' and the one it streams by.
fn extents(folder: &Path, source: &str) -> [(Vec3, Vec3); 2] {
    let model = Maps::read(folder).unwrap();
    let facts = inspect(folder, &model, Depth::Open);
    let map = facts.source(source).and_then(|s| s.map()).expect("a map");
    [map.stored_entities, map.stored_streaming].map(|stored| {
        let (min, max) = stored.expect("the map states extents");
        (Vec3::from(min), Vec3::from(max))
    })
}

fn codes(folder: &Path) -> Vec<&'static str> {
    let model = Maps::read(folder).unwrap();
    let facts = inspect(folder, &model, Depth::Open);
    check(&model, &facts, None).iter().map(|f| f.code).collect()
}

#[test]
fn an_entity_of_an_archetype_the_pack_does_not_declare_is_held_by_the_extents_and_said() {
    let scratch = tempfile::tempdir().unwrap();
    let harbour = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity::at(BENCH, [30.0, 20.0, 0.5], 2),
        ],
        ([0.0, 0.0, -0.5], [100.0; 3]),
    );
    let yard = map_file(
        "yard",
        &[Entity::at("prop_crate_a", [-40.0, 5.0, 2.0], 9)],
        ([-50.0; 3], [50.0; 3]),
    );
    let folder = pack(
        scratch.path(),
        &[
            ("harbour.ymap", harbour),
            ("yard.ymap", yard),
            ("harbour_types.ytyp", crate_types()),
        ],
    );
    assert!(
        !codes(&folder).contains(&"extents-short"),
        "{:?}",
        codes(&folder)
    );
    let far = [5000.0, 5000.0, 0.5];
    let farther = [-7000.0, 300.0, 0.5];
    let before = files(&folder);

    let moved = apply(
        &folder,
        &Operation::PlaceEntity {
            source: HARBOUR.into(),
            expects: None,
            entity: 1,
            pose: pose(far),
        },
    )
    .unwrap();
    // A cooked file states a name by its hash, so that is what the note can say.
    let bench = format!("hash_{:08X}", or_io::string_hash(BENCH.as_bytes()));
    assert!(
        moved.notes.len() == 1 && moved.notes[0].contains(&bench),
        "the operation says whose box it did not have: {:?}",
        moved.notes
    );
    let added = apply(
        &folder,
        &Operation::AddEntity {
            source: HARBOUR.into(),
            expects: None,
            archetype: BENCH.into(),
            pose: pose(farther),
            donor: Some(At {
                source: HARBOUR.into(),
                index: 1,
            }),
        },
    )
    .unwrap();
    assert!(!added.notes.is_empty());

    for (which, (min, max)) in ["entities", "streaming"]
        .iter()
        .zip(extents(&folder, HARBOUR))
    {
        for stands in [far, farther] {
            let stands = Vec3::from(stands);
            assert!(
                stands.cmpge(min).all() && stands.cmple(max).all(),
                "the {which} extents {min}..{max} hold the bench at {stands}"
            );
        }
    }
    // The entity a copy was made of states a stream distance, and the box the map streams by reaches that far past it.
    let reach = read_entity(&folder.join(HARBOUR), 1).unwrap().lod_dist;
    assert!(reach > 0.0);
    let [_, (min, max)] = extents(&folder, HARBOUR);
    assert!(min.x <= farther[0] - reach && max.x >= far[0] + reach);
    assert!(!codes(&folder).contains(&"extents-short"));

    // A map whose every archetype the pack declares gives an operation nothing to say.
    let plain = apply(
        &folder,
        &Operation::PlaceEntity {
            source: YARD.into(),
            expects: None,
            entity: 0,
            pose: pose([-300.0, 20.0, 0.5]),
        },
    )
    .unwrap();
    assert_eq!(plain.notes, Vec::<String>::new());

    // Both are taken back to the byte.
    for applied in [plain, added, moved] {
        apply(&folder, &applied.inverse).unwrap();
    }
    assert!(files(&folder) == before);
}

#[test]
fn a_map_whose_extents_leave_out_an_entity_of_an_archetype_nobody_declares_is_short() {
    let scratch = tempfile::tempdir().unwrap();
    let harbour = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity::at(BENCH, [3000.0, 3000.0, 20.0], 2),
        ],
        ([0.0, 0.0, -0.5], [100.0; 3]),
    );
    let folder = pack(
        scratch.path(),
        &[
            ("harbour.ymap", harbour),
            ("harbour_types.ytyp", crate_types()),
        ],
    );
    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let found = check(&model, &facts, None);
    let short = (found.iter())
        .find(|f| f.code == "extents-short")
        .unwrap_or_else(|| panic!("the rule fires: {found:?}"));
    assert!(short.message.contains("at least"), "{}", short.message);
}

/// An empty map made from nothing called `name`.
fn made(name: &str) -> Vec<u8> {
    create::map_file(
        &create::map(name),
        &ArchetypeIndex::new(),
        BinaryFormat::Pso,
    )
    .unwrap()
}

#[test]
fn the_last_entity_and_the_last_archetype_of_a_file_are_taken_out_and_the_inverse_given_puts_them_back(
) {
    let scratch = tempfile::tempdir().unwrap();
    let yard = map_file(
        "yard",
        &[Entity::at("prop_crate_a", [-40.0, 5.0, 2.0], 9)],
        ([-50.0; 3], [50.0; 3]),
    );
    let folder = pack(
        scratch.path(),
        &[("yard.ymap", yard), ("harbour_types.ytyp", crate_types())],
    );
    let before = files(&folder);
    for operation in [
        Operation::RemoveEntity {
            source: YARD.into(),
            expects: None,
            entity: 0,
            extents: None,
        },
        Operation::RemoveArchetype {
            source: TYPES.into(),
            expects: None,
            archetype: 0,
        },
    ] {
        let name = operation.name();
        // A dry run says the inverse a second list is made of.
        let dry = apply_all(&folder, std::slice::from_ref(&operation), true).unwrap();
        let both = [operation.clone(), dry[0].inverse.clone()];
        apply_all(&folder, &both, true).unwrap_or_else(|e| panic!("{name} and its inverse: {e}"));

        let applied = apply(&folder, &operation).unwrap();
        assert_ne!(files(&folder), before, "{name}");
        apply(&folder, &applied.inverse)
            .unwrap_or_else(|e| panic!("the inverse given for {name} is refused: {e}"));
        assert!(files(&folder) == before, "{name} is taken back to the byte");
    }
}

#[test]
fn a_map_and_a_type_file_that_hold_nothing_take_their_first_through_an_operation() {
    let scratch = tempfile::tempdir().unwrap();
    let yard = map_file(
        "yard",
        &[Entity {
            scale: 1.5,
            ..Entity::at("prop_crate_a", [-40.0, 5.0, 2.0], 9)
        }],
        ([-50.0; 3], [50.0; 3]),
    );
    let empty_types = create::types_file(&create::types("quay_types"), BinaryFormat::Pso).unwrap();
    let folder = pack(
        scratch.path(),
        &[
            ("harbour.ymap", made("harbour")),
            ("quay.ymap", made("quay")),
            ("yard.ymap", yard),
            ("harbour_types.ytyp", crate_types()),
            ("quay_types.ytyp", empty_types),
        ],
    );
    let before = files(&folder);

    // With no donor: the entity is built here.
    let first = apply(
        &folder,
        &Operation::AddEntity {
            source: HARBOUR.into(),
            expects: None,
            archetype: "prop_crate_a".into(),
            pose: pose([60.0, 70.0, 0.5]),
            donor: None,
        },
    )
    .unwrap_or_else(|e| panic!("the first entity of a map made from nothing: {e}"));
    assert_eq!(first.added, Some(0));
    let entity = read_entity(&folder.join(HARBOUR), 0).unwrap();
    assert!(entity.archetype_name == "prop_crate_a");
    assert_eq!((entity.guid, entity.parent_index), (1, -1));

    // With a donor of another map file, which is what a screen sends: what the donor states of itself is kept.
    let donor = read_entity(&folder.join(YARD), 0).unwrap();
    let second = apply(
        &folder,
        &Operation::AddEntity {
            source: "quay.metadata.src".into(),
            expects: None,
            archetype: "prop_crate_a".into(),
            pose: pose([6.0, 7.0, 0.5]),
            donor: Some(At {
                source: YARD.into(),
                index: 0,
            }),
        },
    )
    .unwrap_or_else(|e| panic!("the first entity, as a copy of one of another file: {e}"));
    let copy = read_entity(&folder.join("quay.metadata.src"), 0).unwrap();
    assert_eq!(
        (copy.flags, copy.lod_dist, copy.child_lod_dist),
        (donor.flags, donor.lod_dist, donor.child_lod_dist)
    );
    assert_eq!((copy.position.x, copy.guid), (6.0, 1));

    // The first archetype of a type file, as a copy of one of another.
    let third = apply(
        &folder,
        &Operation::AddArchetype {
            source: "quay_types.metadata.src".into(),
            expects: None,
            name: "prop_quay_crate".into(),
            asset: None,
            donor: Some(At {
                source: TYPES.into(),
                index: 0,
            }),
        },
    )
    .unwrap_or_else(|e| panic!("the first archetype, as a copy of one of another file: {e}"));
    let archetype = read_archetype(&folder.join("quay_types.metadata.src"), 0).unwrap();
    let copied = read_archetype(&folder.join(TYPES), 0).unwrap();
    assert!(archetype.name == "prop_quay_crate" && archetype.asset_name == "prop_quay_crate");
    assert_eq!(archetype.bb_max, copied.bb_max);

    for applied in [third, second, first] {
        apply(&folder, &applied.inverse).unwrap();
    }
    assert!(files(&folder) == before);
}

/// ⚠ The reading the owner is to rule on, held as it stands so a change of it is seen: a new entity takes one more than the largest
/// guid of its map, so the one added after the entity with the largest guid went takes that guid, and the comparison, which trusts a
/// guid one entity of each file states, reads a deletion and an addition as one entity kept.
#[test]
fn an_entity_added_after_the_one_with_the_largest_guid_went_takes_its_guid_and_compares_as_that_entity_changed(
) {
    use or_maps::changes::{source_changes, EntityChange, Matched, SourceChanges};

    let scratch = tempfile::tempdir().unwrap();
    let harbour = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity::at("prop_crate_a", [14.0, 20.0, 0.5], 2),
            Entity::at("prop_tree", [18.0, 20.0, 0.5], 3),
        ],
        ([0.0, 0.0, -0.5], [100.0; 3]),
    );
    let folder = pack(
        scratch.path(),
        &[
            ("harbour.ymap", harbour),
            ("harbour_types.ytyp", crate_types()),
        ],
    );
    let list = [
        Operation::RemoveEntity {
            source: HARBOUR.into(),
            expects: None,
            entity: 2,
            extents: None,
        },
        Operation::AddEntity {
            source: HARBOUR.into(),
            expects: None,
            archetype: "prop_crate_b".into(),
            pose: pose([60.0, 70.0, 0.5]),
            donor: None,
        },
    ];
    apply_all(&folder, &list, false).unwrap();
    assert_eq!(read_entity(&folder.join(HARBOUR), 2).unwrap().guid, 3);

    let SourceChanges::Map(changes) = source_changes(&folder.join(HARBOUR)).unwrap() else {
        panic!("a map file");
    };
    assert_eq!((changes.added.len(), changes.deleted.len()), (0, 0));
    let [kept] = changes.changed.as_slice() else {
        panic!("one entity kept and changed: {:?}", changes.changed);
    };
    assert_eq!(kept.matched, Matched::Guid);
    assert_eq!((kept.base.guid, kept.now.guid), (3, 3));
    assert!(kept
        .changes
        .iter()
        .any(|change| matches!(change, EntityChange::Archetype { .. })));
}

/// ⚠ An operation edits one file, and a link into a parent map is another file's (`flags` bit 3): taking an entity out of the parent
/// map goes through, and what the files then show of it is a rule's to say.
#[test]
fn an_entity_taken_out_of_a_parent_map_leaves_a_link_past_its_end_which_a_rule_names() {
    use or_io::decl::File;
    use or_io::meta::instance::HashedString;
    use or_io::meta::typed::CMapData;

    let scratch = tempfile::tempdir().unwrap();
    let harbour = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity::at("prop_crate_a", [14.0, 20.0, 0.5], 2),
            Entity::at("prop_crate_a", [18.0, 20.0, 0.5], 3),
            Entity::at("prop_crate_a", [22.0, 20.0, 0.5], 4),
        ],
        ([0.0, 0.0, -0.5], [100.0; 3]),
    );
    // `yard` names `harbour` as its parent, and its entity names the last entity of `harbour` as its LOD parent.
    let mut yard = CMapData::from_bytes(&map_file(
        "yard",
        &[Entity::at("prop_crate_a", [20.0, 5.0, 2.0], 9)],
        ([0.0, 0.0, -0.5], [100.0; 3]),
    ))
    .unwrap();
    yard.parent = HashedString::new("harbour");
    yard.entities[0].flags |= 1 << 3;
    yard.entities[0].parent_index = 3;
    let folder = pack(
        scratch.path(),
        &[
            ("harbour.ymap", harbour),
            ("yard.ymap", yard.to_bytes().unwrap()),
            ("harbour_types.ytyp", crate_types()),
        ],
    );
    assert!(!codes(&folder).contains(&"parent-link-broken"));

    apply(
        &folder,
        &Operation::RemoveEntity {
            source: HARBOUR.into(),
            expects: None,
            entity: 1,
            extents: None,
        },
    )
    .unwrap();
    assert_eq!(
        read_entity(&folder.join(YARD), 0).unwrap().parent_index,
        3,
        "the link of the other file is left as it was"
    );
    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let found = check(&model, &facts, None);
    let broken = (found.iter())
        .find(|f| f.code == "parent-link-broken")
        .unwrap_or_else(|| panic!("the rule fires: {found:?}"));
    assert_eq!(
        (broken.place.source.as_deref(), broken.place.entity),
        (Some(YARD), Some(0))
    );
    assert!(
        broken.message.contains("past the 3 entities"),
        "{}",
        broken.message
    );
    assert!(!broken.blocks(), "a warning, which a waiver sets aside");
}

#[test]
fn a_type_file_that_does_not_cook_is_said_and_an_rbm_map_that_holds_nothing_refuses_its_first_by_name(
) {
    let scratch = tempfile::tempdir().unwrap();
    let yard = map_file(
        "yard",
        &[Entity::at("prop_crate_a", [-40.0, 5.0, 2.0], 9)],
        ([-50.0; 3], [50.0; 3]),
    );
    // The crate's sample writes RBM, as the game's own maps are.
    let empty = map_file("harbour", &[], ([0.0; 3], [0.0; 3]));
    let folder = pack(
        scratch.path(),
        &[
            ("yard.ymap", yard),
            ("harbour.ymap", empty),
            ("harbour_types.ytyp", crate_types()),
        ],
    );

    // An RBM class carries a signature nothing derives, and the refusal says that and never that a class is missing from a schema.
    let before = files(&folder);
    let refused = apply(
        &folder,
        &Operation::AddEntity {
            source: HARBOUR.into(),
            expects: None,
            archetype: "prop_crate_a".into(),
            pose: pose([1.0, 2.0, 3.0]),
            donor: Some(At {
                source: YARD.into(),
                index: 0,
            }),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(
        refused.contains("signature") && !refused.contains("synth"),
        "{refused}"
    );
    assert!(files(&folder) == before);

    // A type file whose XML no longer compiles declares no box, and the operation that grows the extents says which file.
    std::fs::write(folder.join(TYPES).join("metadata.xml"), "no type file").unwrap();
    let moved = apply(
        &folder,
        &Operation::PlaceEntity {
            source: YARD.into(),
            expects: None,
            entity: 0,
            pose: pose([-300.0, 20.0, 0.5]),
        },
    )
    .unwrap();
    assert!(
        (moved.notes.iter()).any(|note| note.contains(TYPES) && note.contains("does not cook")),
        "{:?}",
        moved.notes
    );
    let [(min, _), _] = extents(&folder, YARD);
    assert!(min.x <= -300.0, "the entity is held where it stands");
}
