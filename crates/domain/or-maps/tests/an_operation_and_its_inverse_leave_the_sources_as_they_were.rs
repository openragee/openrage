//! Every operation of the vocabulary of world edits, followed by the inverse `apply` gives for it, leaves every file of the pack byte for
//! byte as it was; its JSON reads back as itself; and one that cannot be done is refused by name with nothing changed.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_maps::edit::{read_archetype, read_entity};
use or_maps::import::{add_files, Incoming};
use or_maps::operation::{
    apply, apply_all, digest_of, At, Error, Extents, Operation, Operations, PoseData, Record,
    FORMAT,
};
use or_maps::sample::{map_file, types_file, Archetype, Entity};

const HARBOUR: &str = "harbour.metadata.src";
const YARD: &str = "yard.metadata.src";
const TYPES: &str = "harbour_types.metadata.src";

/// A pack of one set: `harbour`, whose entity 0 is the LOD parent of 1 and 2 and whose 3 stands alone, `yard` with one entity, and a
/// type file of two crates. The extents `harbour` states hold none of what stands past 100.
fn pack(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let harbour = map_file(
        "harbour",
        &[
            Entity {
                children: 2,
                ..Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1)
            },
            Entity {
                parent: 0,
                yaw: 0.6,
                ..Entity::at("prop_crate_a", [14.0, 20.0, 0.5], 2)
            },
            Entity {
                parent: 0,
                ..Entity::at("prop_crate_b", [18.0, 20.0, 0.5], 3)
            },
            Entity::at("prop_crate_b", [150.0, 30.0, 0.5], 4),
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let yard = map_file(
        "yard",
        &[Entity {
            scale: 1.5,
            ..Entity::at("prop_crate_b", [-40.0, 5.0, 2.0], 9)
        }],
        ([-50.0; 3], [50.0; 3]),
    );
    let types = types_file(
        "harbour_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        ],
    );
    let incoming = |name: &'static str, bytes: &'static [u8]| Incoming {
        name,
        bytes,
        from_game: false,
        virtual_path: None,
    };
    let added = add_files(
        &folder,
        &[
            incoming("harbour.ymap", harbour.leak()),
            incoming("yard.ymap", yard.leak()),
            incoming("harbour_types.ytyp", types.leak()),
        ],
        None,
    )
    .unwrap();
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
        rotation: glam::Quat::from_rotation_z(1.1).to_array(),
        scale_xy: 2.0,
        scale_z: 0.75,
    }
}

fn place() -> Operation {
    Operation::PlaceEntity {
        source: HARBOUR.into(),
        entity: 1,
        pose: pose([420.0, -66.5, 12.25]),
        expects: None,
    }
}

fn remove(entity: usize) -> Operation {
    Operation::RemoveEntity {
        source: HARBOUR.into(),
        entity,
        extents: None,
        expects: None,
    }
}

/// The inverse of `operation`, once it is applied to the pack in `folder`.
fn inverse_of(folder: &Path, operation: &Operation) -> Operation {
    apply(folder, operation).expect("applies").inverse
}

/// One case: an operation to try on a fresh pack, which a case may first change.
type Case = fn(&Path) -> Operation;

/// ⚠ A variant with no case here fails `every_operation_followed_by_its_inverse_leaves_every_source_byte_for_byte_as_it_was`.
const CASES: &[Case] = &[
    // A pose past the extents: the inverse carries the record and the extents it grew.
    |_| place(),
    // A copy of the map's first entity, placed past the extents.
    |_| Operation::AddEntity {
        source: HARBOUR.into(),
        archetype: "prop_crate_b".into(),
        pose: pose([-300.0, 8.0, 1.0]),
        donor: None,
        expects: None,
    },
    // A copy of an entity of another map file, of an archetype known by its hash alone.
    |_| Operation::AddEntity {
        source: HARBOUR.into(),
        archetype: format!("hash_{:08X}", or_io::string_hash(b"prop_crate_a")),
        pose: pose([20.0, 20.0, 1.0]),
        donor: Some(At {
            source: YARD.into(),
            index: 0,
        }),
        expects: None,
    },
    // The LOD parent of two: both lose their parent and the later ones move down.
    |_| remove(0),
    // A child in the middle: its parent counts one fewer.
    |_| remove(1),
    |folder| inverse_of(folder, &remove(0)),
    |folder| inverse_of(folder, &remove(1)),
    |folder| inverse_of(folder, &place()),
    // An archetype of an entity changed, as the record a person edited: no extents move.
    |folder| {
        let mut entity = read_entity(&folder.join(HARBOUR), 3).unwrap();
        entity.archetype_name = or_io::meta::instance::HashedString::new("prop_crate_a");
        entity.lod_dist = 55.5;
        Operation::ReplaceEntity {
            source: HARBOUR.into(),
            entity: 3,
            record: Record::entity(&entity).unwrap(),
            extents: None,
            expects: None,
        }
    },
    // Entity 3 stands past the extents the map states.
    |_| Operation::GrowExtents {
        source: HARBOUR.into(),
        expects: None,
    },
    |_| Operation::SetExtents {
        source: HARBOUR.into(),
        extents: Extents {
            entities_extents_min: [-1.5, -2.5, -3.5],
            entities_extents_max: [200.0, 201.0, 202.0],
            streaming_extents_min: [-400.25, -400.5, -400.75],
            streaming_extents_max: [0.1, 0.2, 0.3],
        },
        expects: None,
    },
    |_| Operation::AddArchetype {
        source: TYPES.into(),
        name: "prop_new_crate".into(),
        asset: Some("prop_new_crate_lod".into()),
        donor: Some(At {
            source: TYPES.into(),
            index: 1,
        }),
        expects: None,
    },
    |_| Operation::RemoveArchetype {
        source: TYPES.into(),
        archetype: 0,
        expects: None,
    },
    |folder| {
        inverse_of(
            folder,
            &Operation::RemoveArchetype {
                source: TYPES.into(),
                archetype: 0,
                expects: None,
            },
        )
    },
    |folder| {
        let mut archetype = read_archetype(&folder.join(TYPES), 1).unwrap();
        archetype.bb_max.z = 7.125;
        archetype.name = or_io::meta::instance::HashedString::new("prop_crate_c");
        Operation::ReplaceArchetype {
            source: TYPES.into(),
            archetype: 1,
            record: Record::archetype(&archetype).unwrap(),
            expects: None,
        }
    },
    |folder| {
        let xml = std::fs::read_to_string(folder.join(YARD).join("metadata.xml")).unwrap();
        let rewritten = xml.replace("contentFlags value=\"65\"", "contentFlags value=\"66\"");
        assert_ne!(rewritten, xml);
        Operation::RewriteSource {
            source: YARD.into(),
            xml: rewritten,
            expects: None,
        }
    },
];

/// The names of the operations, as the type itself lists them when it refuses a name.
fn variants() -> BTreeSet<String> {
    let refused = serde_json::from_str::<Operation>(r#"{ "op": "no such operation" }"#)
        .unwrap_err()
        .to_string();
    let (_, listed) = refused
        .split_once("expected one of")
        .unwrap_or_else(|| panic!("the refusal lists the operations: {refused}"));
    listed
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn every_operation_followed_by_its_inverse_leaves_every_source_byte_for_byte_as_it_was() {
    let mut covered = BTreeSet::new();
    for (number, case) in CASES.iter().enumerate() {
        let scratch = tempfile::tempdir().unwrap();
        let folder = pack(scratch.path());
        let operation = case(&folder);
        let name = operation.name();
        covered.insert(name.to_string());

        let before = files(&folder);
        let applied =
            apply(&folder, &operation).unwrap_or_else(|e| panic!("case {number} ({name}): {e}"));
        let after = files(&folder);
        assert_ne!(after, before, "case {number} ({name}) changes a source");
        assert!(applied.edited.changed(), "case {number} ({name})");
        assert_eq!(
            after.get(&applied.edited.file),
            Some(&applied.edited.after),
            "case {number} ({name}) says what it wrote"
        );
        // The inverse says what to put back, and is the whole XML only for an operation that states XML.
        assert_eq!(
            applied.inverse.name() == "rewrite_source",
            name == "rewrite_source",
            "case {number} ({name}) is taken back by {}",
            applied.inverse.name()
        );

        let undone = apply(&folder, &applied.inverse)
            .unwrap_or_else(|e| panic!("the inverse of case {number} ({name}): {e}"));
        let restored = files(&folder);
        for (path, bytes) in &before {
            assert!(
                restored.get(path) == Some(bytes),
                "case {number} ({name}): {} is not as it was after its inverse",
                path.display()
            );
        }
        assert_eq!(
            restored.len(),
            before.len(),
            "case {number} ({name}) leaves no file behind"
        );

        // The inverse of the inverse does the operation again.
        apply(&folder, &undone.inverse)
            .unwrap_or_else(|e| panic!("case {number} ({name}) done again: {e}"));
        assert!(
            files(&folder) == after,
            "case {number} ({name}) done again writes what it wrote"
        );
    }
    let variants = variants();
    assert!(
        variants.contains("place_entity") && variants.len() > 1,
        "{variants:?}"
    );
    assert_eq!(
        covered, variants,
        "every operation has a case that its inverse takes back"
    );
}

#[test]
fn the_json_of_every_operation_reads_back_as_itself_and_another_schema_is_refused() {
    let mut covered = BTreeSet::new();
    for case in CASES {
        let scratch = tempfile::tempdir().unwrap();
        let folder = pack(scratch.path());
        let operation = case(&folder);
        let inverse = inverse_of(&folder, &operation);
        covered.insert(operation.name().to_string());
        let list = Operations::new(vec![operation.clone(), inverse]);
        let text = list.to_json().unwrap();
        assert!(text.ends_with("}\n") && !text.contains('\r'));
        let stated = format!("\"op\": \"{}\"", operation.name());
        assert!(text.contains(&stated), "{stated} in {text}");
        assert_eq!(
            Operations::from_json(text.as_bytes(), "x").unwrap(),
            list,
            "{}",
            operation.name()
        );
    }
    assert_eq!(covered, variants());

    let text = Operations::new(vec![place()]).to_json().unwrap();
    assert!(text.contains(&format!("\"format\": \"{FORMAT}\"")) && text.contains("\"schema\": 1"));
    for other in ["\"schema\": 2", "\"schema\": 0", "\"schema\": \"1\""] {
        let changed = text.replace("\"schema\": 1", other);
        assert!(
            matches!(
                Operations::from_json(changed.as_bytes(), "x"),
                Err(Error::Schema(..))
            ),
            "{other}"
        );
    }
    let foreign = text.replace(FORMAT, "openrage-maps");
    assert!(matches!(
        Operations::from_json(foreign.as_bytes(), "x"),
        Err(Error::Format(..))
    ));
    for broken in [
        text.replace("place_entity", "launch_entity"),
        text.replace("\"entity\": 1", "\"entity\": 1, \"speed\": 3"),
        text.replace("\"entity\": 1", "\"entity\": -1"),
    ] {
        assert!(
            matches!(
                Operations::from_json(broken.as_bytes(), "x"),
                Err(Error::Json(..))
            ),
            "{broken}"
        );
    }
}

#[test]
fn an_operation_that_names_what_is_not_there_is_refused_by_name_and_changes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);
    let record = Record::entity(&read_entity(&folder.join(HARBOUR), 0).unwrap()).unwrap();
    let nowhere = "../elsewhere.metadata.src";
    let refused: Vec<(Operation, &str)> = vec![
        (
            Operation::PlaceEntity {
                source: HARBOUR.into(),
                entity: 9,
                pose: pose([0.0; 3]),
                expects: None,
            },
            "entity 9 is past the 4",
        ),
        (remove(4), "entity 4 is past the 4"),
        (
            Operation::ReplaceEntity {
                source: HARBOUR.into(),
                entity: 7,
                record: record.clone(),
                extents: None,
                expects: None,
            },
            "entity 7 is past the 4",
        ),
        (
            Operation::InsertEntity {
                source: HARBOUR.into(),
                entity: 6,
                record,
                parent_num_children: None,
                children: Vec::new(),
                extents: None,
                expects: None,
            },
            "entity 6 is past the end of the 4",
        ),
        (
            Operation::AddEntity {
                source: HARBOUR.into(),
                archetype: "prop_crate_a".into(),
                pose: pose([0.0; 3]),
                donor: Some(At {
                    source: YARD.into(),
                    index: 3,
                }),
                expects: None,
            },
            "entity 3 is past the 1",
        ),
        (
            Operation::AddEntity {
                source: HARBOUR.into(),
                archetype: "  ".into(),
                pose: pose([0.0; 3]),
                donor: None,
                expects: None,
            },
            "an entity places an archetype, and none is named",
        ),
        (
            Operation::RemoveArchetype {
                source: TYPES.into(),
                archetype: 2,
                expects: None,
            },
            "archetype 2 is past the 2",
        ),
        (
            Operation::AddArchetype {
                source: TYPES.into(),
                name: "PROP_CRATE_A".into(),
                asset: None,
                donor: None,
                expects: None,
            },
            "resolves a name once",
        ),
        (
            Operation::PlaceEntity {
                source: HARBOUR.into(),
                entity: 0,
                pose: PoseData {
                    position: [f32::NAN, 0.0, 0.0],
                    ..pose([0.0; 3])
                },
                expects: None,
            },
            "finite",
        ),
        (
            Operation::PlaceEntity {
                source: HARBOUR.into(),
                entity: 0,
                pose: PoseData {
                    rotation: [0.0; 4],
                    ..pose([0.0; 3])
                },
                expects: None,
            },
            "rotation of no length",
        ),
        (
            Operation::PlaceEntity {
                source: TYPES.into(),
                entity: 0,
                pose: pose([0.0; 3]),
                expects: None,
            },
            "does not edit a type file",
        ),
        (
            Operation::RemoveArchetype {
                source: HARBOUR.into(),
                archetype: 0,
                expects: None,
            },
            "does not edit a map file",
        ),
        (
            Operation::GrowExtents {
                source: nowhere.into(),
                expects: None,
            },
            "no map set of the pack names this source",
        ),
        (
            Operation::RewriteSource {
                source: HARBOUR.into(),
                xml: "<CMapData><entities>".into(),
                expects: None,
            },
            "metadata.xml",
        ),
    ];
    for (operation, why) in &refused {
        let said = match apply(&folder, operation) {
            Err(Error::Refused { name, at, why }) => {
                assert_eq!(name, operation.name());
                assert_eq!(at, operation.source());
                format!("{name} of `{at}`: {why}")
            }
            other => panic!("{} was not refused: {other:?}", operation.name()),
        };
        assert!(said.contains(why), "{said} says {why}");
        assert_eq!(
            apply(&folder, operation).unwrap_err().to_string(),
            said,
            "the refusal names the operation and its source"
        );
        assert!(
            files(&folder) == before,
            "{} changed a source",
            operation.name()
        );
    }

    // In a list, the one refused is named by its place, and the ones before it are not written.
    let list = [place(), remove(0), refused[1].0.clone(), place()];
    let failed = apply_all(&folder, &list, false).unwrap_err();
    assert!(
        matches!(
            &failed,
            Error::InList {
                number: 3,
                of: 4,
                ..
            }
        ),
        "{failed:?}"
    );
    assert!(
        failed
            .to_string()
            .starts_with("operation 3 of 4, remove_entity of `harbour.metadata.src`: entity 4"),
        "{failed}"
    );
    assert!(files(&folder) == before, "a refused list writes nothing");
}

#[test]
fn a_dry_run_says_what_a_list_would_change_and_writes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);
    let list = [
        place(),
        remove(2),
        Operation::AddArchetype {
            source: TYPES.into(),
            name: "prop_new_crate".into(),
            asset: None,
            donor: None,
            expects: None,
        },
        // Reads the type file as the operation before it left it.
        Operation::AddEntity {
            source: HARBOUR.into(),
            archetype: "prop_new_crate".into(),
            pose: pose([900.0, 0.0, 0.0]),
            donor: None,
            expects: None,
        },
    ];
    let dry = apply_all(&folder, &list, true).unwrap();
    assert_eq!(dry.len(), list.len());
    assert!(dry.iter().all(|applied| applied.edited.changed()));
    assert_eq!(dry[3].added, Some(3));
    assert!(files(&folder) == before, "a dry run writes nothing");

    let done = apply_all(&folder, &list, false).unwrap();
    assert_eq!(done, dry, "a dry run says what the run does");
    let after = files(&folder);
    assert_eq!(
        after.get(&done[3].edited.file),
        Some(&done[3].edited.after),
        "the last operation on a source is what the source holds"
    );
    assert_eq!(after.get(&done[2].edited.file), Some(&done[2].edited.after));

    // The inverses, last first, put the pack back.
    let back: Vec<Operation> = done.iter().rev().map(|a| a.inverse.clone()).collect();
    apply_all(&folder, &back, false).unwrap();
    assert!(files(&folder) == before);
}

#[test]
fn xml_typed_by_hand_is_put_back_as_it_was_typed() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let file = folder.join(HARBOUR).join("metadata.xml");
    let exported = std::fs::read_to_string(&file).unwrap();
    // The same document, spaced as no export spaces it.
    let typed = exported.replacen("<entities", "\n\n      <entities", 1);
    assert_ne!(typed, exported);
    std::fs::write(&file, &typed).unwrap();
    let before = files(&folder);

    let applied = apply(&folder, &place()).unwrap();
    assert_eq!(
        applied.inverse,
        Operation::RewriteSource {
            source: HARBOUR.into(),
            xml: typed,
            expects: Some(digest_of(&std::fs::read(&file).unwrap())),
        },
        "an edit rewrites the XML whole, so what takes it back is the XML as it was typed, stated for the XML the edit wrote"
    );
    assert_ne!(files(&folder), before);
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);
}

#[test]
fn an_inverse_that_would_not_put_the_file_back_is_the_xml_as_it_was() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    // An insertion at the end that claims an entity as its LOD child: a removal leaves what named the removed entity with no parent.
    let record = Record::entity(&read_entity(&folder.join(HARBOUR), 0).unwrap()).unwrap();
    let claims = |children: Vec<usize>| Operation::InsertEntity {
        source: HARBOUR.into(),
        entity: 4,
        record: record.clone(),
        parent_num_children: None,
        children,
        extents: None,
        expects: None,
    };

    // Entity 1 names 0 as its parent: once it is claimed by the inserted one, a removal cannot say whose it was.
    let before = files(&folder);
    let applied = apply(&folder, &claims(vec![1])).unwrap();
    assert_eq!(applied.inverse.name(), "rewrite_source");
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);

    // Entity 3 has no parent, which is what a removal leaves it with: the plain inverse is proven and given.
    let applied = apply(&folder, &claims(vec![3])).unwrap();
    assert_eq!(applied.inverse.name(), "remove_entity");
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);
}

const QUAY: &str = "quay.metadata.src";

/// A pack of one map file, `quay`, whose entity 0 stands alone and whose entity 1 is the LOD parent of 2 and 3: a parent that stands
/// after the first entity, which `harbour` has none of.
fn quay(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let quay = map_file(
        "quay",
        &[
            Entity::at("prop_crate_a", [1.0, 2.0, 0.5], 1),
            Entity {
                children: 2,
                ..Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 2)
            },
            Entity {
                parent: 1,
                ..Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 3)
            },
            Entity {
                parent: 1,
                ..Entity::at("prop_crate_b", [18.0, 20.0, 0.5], 4)
            },
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(
        &folder,
        &[Incoming {
            name: "quay.ymap",
            bytes: &quay,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

/// The guid, the LOD parent and the child count of every entity of the source `source`, in order.
fn links(folder: &Path, source: &str) -> Vec<(u32, i32, u32)> {
    (0..)
        .map_while(|at| read_entity(&folder.join(source), at).ok())
        .map(|e| (e.guid, e.parent_index, e.num_children))
        .collect()
}

#[test]
fn an_entity_put_in_before_a_lod_parent_moves_the_links_that_name_the_parent() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = quay(scratch.path());
    let before = files(&folder);
    assert_eq!(
        links(&folder, QUAY),
        [(1, -1, 0), (2, -1, 2), (3, 1, 0), (4, 1, 0)]
    );
    let take_first = Operation::RemoveEntity {
        source: QUAY.into(),
        entity: 0,
        extents: None,
        expects: None,
    };

    // Taken out, the parent stands first and its children name it there.
    let applied = apply(&folder, &take_first).unwrap();
    assert_eq!(links(&folder, QUAY), [(2, -1, 2), (3, 0, 0), (4, 0, 0)]);
    assert_eq!(
        applied.inverse.name(),
        "insert_entity",
        "the removal is taken back by what it removed, not by the whole XML"
    );
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);

    // An entity put in where the parent stands: the parent moves down one and its children follow it.
    let record = Record::entity(&read_entity(&folder.join(QUAY), 0).unwrap()).unwrap();
    let put_in = Operation::InsertEntity {
        source: QUAY.into(),
        entity: 1,
        record,
        parent_num_children: None,
        children: Vec::new(),
        extents: None,
        expects: None,
    };
    let applied = apply(&folder, &put_in).unwrap();
    assert_eq!(
        links(&folder, QUAY),
        [(1, -1, 0), (1, -1, 0), (2, -1, 2), (3, 2, 0), (4, 2, 0)]
    );
    assert_eq!(applied.inverse.name(), "remove_entity");
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);
}

#[test]
fn an_entity_that_names_itself_as_its_lod_parent_is_taken_out_and_no_other_count_changes() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let looped = map_file(
        "quay",
        &[
            // Its own parent: what `hierarchy-broken` finds, and what a person takes out to mend the map.
            Entity {
                parent: 0,
                ..Entity::at("prop_crate_a", [1.0, 2.0, 0.5], 1)
            },
            Entity {
                children: 1,
                ..Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 2)
            },
            Entity {
                parent: 1,
                ..Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 3)
            },
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(
        &folder,
        &[Incoming {
            name: "quay.ymap",
            bytes: &looped,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let before = files(&folder);

    let applied = apply(
        &folder,
        &Operation::RemoveEntity {
            source: QUAY.into(),
            entity: 0,
            extents: None,
            expects: None,
        },
    )
    .unwrap();
    assert_eq!(
        links(&folder, QUAY),
        [(2, -1, 1), (3, 0, 0)],
        "the entity that moved into its place keeps the child it has"
    );
    // The typed root refuses to save an entity that is its own parent, so no record put back restores the file: the XML as it was does.
    assert_eq!(applied.inverse.name(), "rewrite_source");
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);
}

#[test]
fn a_source_that_leaves_the_folder_of_the_pack_is_refused_and_is_not_written() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    // A source beside the pack's folder, which the pack's own file is made to name.
    let outside = scratch.path().join("outside.metadata.src");
    std::fs::create_dir(&outside).unwrap();
    for entry in std::fs::read_dir(folder.join(YARD)).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, outside.join(path.file_name().unwrap())).unwrap();
    }
    let named = "../outside.metadata.src";
    let model = folder.join("maps.json");
    let stated = std::fs::read_to_string(&model).unwrap();
    let escaping = stated.replacen(
        &format!("\"{YARD}\""),
        &format!("\"{YARD}\",\n        \"{named}\""),
        1,
    );
    assert_ne!(escaping, stated);
    std::fs::write(&model, escaping).unwrap();
    let before = files(scratch.path());

    let xml = std::fs::read_to_string(outside.join("metadata.xml")).unwrap();
    let rewritten = xml.replace("contentFlags value=\"65\"", "contentFlags value=\"66\"");
    assert_ne!(rewritten, xml);
    for operation in [
        Operation::RewriteSource {
            source: named.into(),
            xml: rewritten,
            expects: None,
        },
        Operation::PlaceEntity {
            source: named.into(),
            entity: 0,
            pose: pose([900.0, 0.0, 0.0]),
            expects: None,
        },
    ] {
        match apply(&folder, &operation) {
            Err(Error::Refused { why, .. }) => {
                assert!(why.contains("leaves the folder of the pack"), "{why}")
            }
            Err(other) => panic!("{}: {other}", operation.name()),
            Ok(_) => panic!("{} was not refused", operation.name()),
        }
        assert!(
            files(scratch.path()) == before,
            "{} wrote past the pack",
            operation.name()
        );
    }

    // The sources inside are edited as before, with the one outside named beside them.
    apply(&folder, &place()).unwrap();
    let after = files(scratch.path());
    assert_ne!(after, before);
    assert!(
        (after.iter()).all(|(path, bytes)| path.starts_with(folder.join(HARBOUR))
            || before.get(path) == Some(bytes)),
        "only the source placed in changes"
    );
}

#[test]
fn a_rotation_that_is_not_of_length_one_is_stored_as_the_rotation_it_turns_by() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let turn = glam::Quat::from_rotation_z(0.6);
    let stretched = PoseData {
        rotation: (turn * 3.0).to_array(),
        ..pose([20.0, 20.0, 1.0])
    };
    let length_of = |entity: usize| {
        let stored = read_entity(&folder.join(HARBOUR), entity).unwrap().rotation;
        glam::Vec4::new(stored.x, stored.y, stored.z, stored.w).length()
    };

    apply(
        &folder,
        &Operation::PlaceEntity {
            source: HARBOUR.into(),
            entity: 3,
            pose: stretched,
            expects: None,
        },
    )
    .unwrap();
    let added = apply(
        &folder,
        &Operation::AddEntity {
            source: HARBOUR.into(),
            archetype: "prop_crate_a".into(),
            pose: stretched,
            donor: None,
            expects: None,
        },
    )
    .unwrap()
    .added
    .expect("an entity is added");
    for entity in [3, added] {
        assert!(
            (length_of(entity) - 1.0).abs() < 1.0e-6,
            "entity {entity} stores a rotation of length {}",
            length_of(entity)
        );
        let placed = read_entity(&folder.join(HARBOUR), entity).unwrap();
        let held = or_maps::edit::pose_of(&placed).rotation;
        assert!(
            held.dot(turn).abs() > 1.0 - 1.0e-6,
            "{held} turns as {turn}"
        );
    }
}
