//! A session holds the sources of a pack open as typed roots: a list of operations done through it and flushed once leaves every file
//! byte for byte as doing the list one by one leaves it, an operation it refuses and a flush that is refused change nothing, and an
//! inverse is refused when its source is not as the inverse states it.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_maps::edit::{read_archetype, read_entity};
use or_maps::import::{add_files, Incoming};
use or_maps::operation::{
    apply, apply_all, digest_of, At, Error, Extents, Inserted, Operation, PoseData, Record, Session,
};
use or_maps::sample::{child_map_file, map_file, types_file, Archetype, Entity};

const HARBOUR: &str = "harbour.metadata.src";
const YARD: &str = "yard.metadata.src";
const PIER: &str = "pier.metadata.src";
const TYPES: &str = "harbour_types.metadata.src";

fn incoming(name: &'static str, bytes: Vec<u8>) -> Incoming<'static> {
    Incoming {
        name,
        bytes: bytes.leak(),
        from_game: false,
        virtual_path: None,
    }
}

/// A pack of one set made of `files`.
fn pack_of(dir: &Path, files: Vec<Incoming<'static>>) -> PathBuf {
    let folder = dir.join("maps");
    let added = add_files(&folder, &files, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

/// A pack of one set: `harbour`, whose entity 0 is the LOD parent of 1 and 2 and whose 3 stands alone past the extents it states,
/// `yard` with one entity, and a type file of two crates.
fn pack(dir: &Path) -> PathBuf {
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
    pack_of(
        dir,
        vec![
            incoming("harbour.ymap", harbour),
            incoming("yard.ymap", yard),
            incoming("harbour_types.ytyp", types),
        ],
    )
}

/// The pack of [`pack`] with `pier`, whose parent map is `harbour` and whose two entities name its entities 0 and 3 as their LOD parents.
fn pack_with_a_child_map(dir: &Path) -> PathBuf {
    let folder = pack(dir);
    let pier = child_map_file(
        "pier",
        "harbour",
        &[
            Entity {
                parent: 0,
                in_parent_map: true,
                ..Entity::at("prop_crate_a", [30.0, 20.0, 0.5], 21)
            },
            Entity {
                parent: 3,
                in_parent_map: true,
                ..Entity::at("prop_crate_b", [34.0, 20.0, 0.5], 22)
            },
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(&folder, &[incoming("pier.ymap", pier)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

/// Every file under `folder`, by its path inside it.
fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let inside = path.strip_prefix(root).unwrap().to_path_buf();
                into.insert(inside, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

/// A pack `make` makes, `N` times over as the same files: an import gives each source an id of its own, so the copies are of one import.
fn copies<const N: usize>(make: fn(&Path) -> PathBuf) -> ([tempfile::TempDir; N], [PathBuf; N]) {
    let scratch = [(); N].map(|()| tempfile::tempdir().unwrap());
    let first = make(scratch[0].path());
    let made = files(&first);
    let folders = std::array::from_fn(|at| {
        let folder = scratch[at].path().join("maps");
        for (inside, bytes) in &made {
            let file = folder.join(inside);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        folder
    });
    (scratch, folders)
}

/// The files of `one` that `other` does not hold with the same bytes, and the ones only `other` holds.
fn differing(one: &BTreeMap<PathBuf, Vec<u8>>, other: &BTreeMap<PathBuf, Vec<u8>>) -> Vec<PathBuf> {
    (one.iter())
        .filter(|(path, bytes)| other.get(*path) != Some(bytes))
        .map(|(path, _)| path.clone())
        .chain(other.keys().filter(|p| !one.contains_key(*p)).cloned())
        .collect()
}

fn pose(position: [f32; 3]) -> PoseData {
    PoseData {
        position,
        rotation: glam::Quat::from_rotation_z(1.1).to_array(),
        scale_xy: 2.0,
        scale_z: 0.75,
    }
}

fn place(source: &str, entity: usize, position: [f32; 3]) -> Operation {
    Operation::PlaceEntity {
        source: source.into(),
        entity,
        pose: pose(position),
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

fn no_number() -> Extents {
    Extents {
        entities_extents_min: [f32::NAN, 0.0, 0.0],
        entities_extents_max: [1.0; 3],
        streaming_extents_min: [0.0; 3],
        streaming_extents_max: [1.0; 3],
    }
}

/// `operation` with no digest stated: what two ways of doing an operation give alike.
fn bare(operation: &Operation) -> Operation {
    let mut bare = operation.clone();
    bare.set_expects(None);
    bare
}

/// `record` with its member `name` set to `value`.
fn with(record: &Record, name: &str, value: Value) -> Record {
    let mut tree = record.0.clone();
    match &mut tree {
        Value::Pointer(Some(inst)) | Value::Struct(inst) => inst.set(name, value),
        other => panic!("a record is a struct: {other:?}"),
    }
    Record(tree)
}

/// The record of an interior whose one room attaches an object the interior does not hold: read as an archetype, and refused by a save.
fn a_broken_interior(folder: &Path) -> Record {
    let archetype = read_archetype(&folder.join(TYPES), 0).unwrap();
    let mut tree = Record::archetype(&archetype).unwrap().0;
    match &mut tree {
        Value::Pointer(Some(inst)) | Value::Struct(inst) => {
            inst.type_name = "CMloArchetypeDef".into()
        }
        other => panic!("a record is a struct: {other:?}"),
    }
    let mut room = MetaInstance::new("CMloRoomDef");
    room.set("attachedObjects", Value::Array(vec![Value::UInt(5)]));
    let rooms = Value::Array(vec![Value::Struct(Box::new(room))]);
    with(&Record(tree), "rooms", rooms)
}

/// One step of a list: the operation to do next, which may read the pack as the steps before it left it on disk.
type Step = fn(&Path) -> Operation;

/// ⚠ A variant with no step here fails `a_list_done_through_a_session_and_flushed_once_leaves_what_doing_it_one_by_one_leaves`.
const STEPS: &[Step] = &[
    // Past the extents, with the boxes of the type file as it was imported.
    |_| place(HARBOUR, 1, [420.0, -66.5, 12.25]),
    |_| Operation::AddEntity {
        source: HARBOUR.into(),
        archetype: "prop_crate_b".into(),
        pose: pose([-300.0, 8.0, 1.0]),
        donor: None,
        expects: None,
    },
    // A copy of an entity of another source, which the session holds too.
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
    // The LOD parent of two.
    |_| remove(0),
    |folder| Operation::InsertEntity {
        source: HARBOUR.into(),
        entity: 1,
        record: Record::entity(&read_entity(&folder.join(YARD), 0).unwrap()).unwrap(),
        parent_num_children: None,
        children: Vec::new(),
        extents: None,
        expects: None,
    },
    |folder| {
        let mut entity = read_entity(&folder.join(HARBOUR), 2).unwrap();
        entity.archetype_name = HashedString::new("prop_crate_a");
        entity.lod_dist = 55.5;
        Operation::ReplaceEntity {
            source: HARBOUR.into(),
            entity: 2,
            record: Record::entity(&entity).unwrap(),
            extents: None,
            expects: None,
        }
    },
    |_| Operation::SetExtents {
        source: HARBOUR.into(),
        extents: Extents {
            entities_extents_min: [-1.5, -2.5, -3.5],
            entities_extents_max: [2.0, 2.5, 3.0],
            streaming_extents_min: [-4.25, -4.5, -4.75],
            streaming_extents_max: [0.1, 0.2, 0.3],
        },
        expects: None,
    },
    // The box of a crate grows: what the extents grow to hold after this is not what they would have held before.
    |folder| {
        let mut archetype = read_archetype(&folder.join(TYPES), 1).unwrap();
        archetype.bb_max.z = 77.125;
        Operation::ReplaceArchetype {
            source: TYPES.into(),
            archetype: 1,
            record: Record::archetype(&archetype).unwrap(),
            expects: None,
        }
    },
    |_| Operation::GrowExtents {
        source: HARBOUR.into(),
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
    // Of the archetype the step before declared.
    |_| Operation::AddEntity {
        source: HARBOUR.into(),
        archetype: "prop_new_crate".into(),
        pose: pose([900.0, 0.0, 0.0]),
        donor: None,
        expects: None,
    },
    |_| Operation::RemoveArchetype {
        source: TYPES.into(),
        archetype: 0,
        expects: None,
    },
    |folder| {
        let mut archetype = read_archetype(&folder.join(TYPES), 0).unwrap();
        archetype.name = HashedString::new("prop_crate_d");
        archetype.bb_min.x = -9.5;
        Operation::InsertArchetype {
            source: TYPES.into(),
            archetype: 0,
            record: Record::archetype(&archetype).unwrap(),
            expects: None,
        }
    },
    |_| place(HARBOUR, 0, [-777.0, 5.0, 3.0]),
    |_| place(YARD, 0, [60.0, 61.0, 1.0]),
    // The XML of a source the session holds with an operation done to it.
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
    |_| place(YARD, 0, [-90.0, 61.0, 1.0]),
    // Two entities at once, named last first, with one renumbering.
    |_| Operation::RemoveEntities {
        source: HARBOUR.into(),
        entities: vec![3, 1],
        extents: None,
        expects: None,
    },
    // Two copies of an entity of another source put in at once.
    |folder| {
        let record = Record::entity(&read_entity(&folder.join(YARD), 0).unwrap()).unwrap();
        let put = |entity| Inserted {
            entity,
            record: record.clone(),
            children: Vec::new(),
        };
        Operation::InsertEntities {
            source: HARBOUR.into(),
            entities: vec![put(2), put(1)],
            num_children: Vec::new(),
            extents: None,
            expects: None,
        }
    },
    |_| Operation::SetNumChildren {
        source: HARBOUR.into(),
        entity: 0,
        num_children: 7,
        expects: None,
    },
    // The links of `pier` into its parent map, for the entity 1 taken out of that map: the link to 3 names 2.
    |_| Operation::RenumberForRemoved {
        source: PIER.into(),
        removed: vec![1],
        expects: None,
    },
    // And for an entity put in at its start: both links name the one after.
    |_| Operation::RenumberForInserted {
        source: PIER.into(),
        inserted: vec![0],
        expects: None,
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
    (listed.split('`').skip(1).step_by(2))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_list_done_through_a_session_and_flushed_once_leaves_what_doing_it_one_by_one_leaves() {
    let (_scratch, [one_by_one, flushed_once, flushed_each]) = copies(pack_with_a_child_map);
    let imported = files(&one_by_one);
    assert_eq!(
        differing(&imported, &files(&flushed_once)),
        Vec::<PathBuf>::new(),
        "the three packs start as the same files"
    );
    let mut once = Session::open(&flushed_once).unwrap();
    let mut each = Session::open(&flushed_each).unwrap();
    let mut covered = BTreeSet::new();
    for (number, step) in STEPS.iter().enumerate() {
        let operation = step(&one_by_one);
        let name = operation.name();
        covered.insert(name.to_string());
        let applied = apply(&one_by_one, &operation)
            .unwrap_or_else(|e| panic!("step {number} ({name}), one by one: {e}"));
        assert!(
            applied.edited.changed(),
            "step {number} ({name}) changes its source"
        );
        for (session, way) in [
            (&mut once, "flushed once"),
            (&mut each, "flushed each time"),
        ] {
            let done = session
                .apply(&operation)
                .unwrap_or_else(|e| panic!("step {number} ({name}), {way}: {e}"));
            assert_eq!(done.added, applied.added, "step {number} ({name}), {way}");
            assert_eq!(
                bare(&done.inverse),
                bare(&applied.inverse),
                "step {number} ({name}), {way}: the session gives the inverse the proof gives"
            );
            assert_eq!(
                done.inverse
                    .expects()
                    .map(|d| d == applied.inverse.expects().unwrap()),
                // What a rewrite leaves is an XML, which has its digest, and so has what several entities put in at once leave, an
                // operation no session holds to be sure and does as `apply` does it; a root an operation changed has none yet.
                Some(matches!(name, "rewrite_source" | "insert_entities")),
                "step {number} ({name}), {way}"
            );
        }
        assert!(once.unwritten(), "step {number} ({name})");
        assert_eq!(
            differing(&imported, &files(&flushed_once)),
            Vec::<PathBuf>::new(),
            "step {number} ({name}): a session writes nothing before its flush"
        );
        let written = each.flush().unwrap();
        assert!(!each.unwritten());
        assert_eq!(
            differing(&files(&one_by_one), &files(&flushed_each)),
            Vec::<PathBuf>::new(),
            "step {number} ({name}): flushed after each operation"
        );
        let [written] = written.as_slice() else {
            panic!("step {number} ({name}) wrote {written:?}");
        };
        assert_eq!(written.source, operation.source());
        assert_eq!(
            written.file,
            flushed_each.join(operation.source()).join("metadata.xml")
        );
        assert_eq!(written.digest, digest_of(&applied.edited.after));
        assert_eq!(Some(written.digest.as_str()), applied.inverse.expects());
        assert!(written.proven, "step {number} ({name})");
    }
    assert_eq!(covered, variants(), "every operation has a step");

    let written = once.flush().unwrap();
    assert_eq!(
        differing(&files(&one_by_one), &files(&flushed_once)),
        Vec::<PathBuf>::new(),
        "one flush after the whole list"
    );
    assert_eq!(
        written
            .iter()
            .map(|w| w.source.as_str())
            .collect::<Vec<_>>(),
        [HARBOUR, TYPES, PIER, YARD]
    );
    for source in &written {
        assert_eq!(
            source.digest,
            digest_of(&std::fs::read(&source.file).unwrap())
        );
        assert!(source.proven, "{}", source.source);
    }
    assert!(!once.unwritten());
    assert_eq!(
        once.flush().unwrap(),
        [],
        "a flush with nothing to write writes nothing"
    );
}

/// A root is kept across the operations, where one by one it is read again from the XML each time: a map and a type file grown to
/// many times what they were read as are written alike.
#[test]
fn sources_grown_far_past_what_they_were_read_as_are_written_as_one_by_one_writes_them() {
    let (_scratch, [one_by_one, held]) = copies(pack);
    let mut session = Session::open(&held).unwrap();
    for step in 0..96 {
        let entity = Operation::AddEntity {
            source: HARBOUR.into(),
            archetype: format!("prop_grown_{}", step / 2),
            pose: pose([step as f32 * 3.0, -40.0, 1.0]),
            donor: None,
            expects: None,
        };
        let archetype = Operation::AddArchetype {
            source: TYPES.into(),
            name: format!("prop_grown_{step}"),
            asset: None,
            donor: None,
            expects: None,
        };
        for operation in [archetype, entity] {
            let applied = apply(&one_by_one, &operation).unwrap();
            let done = session.apply(&operation).unwrap();
            assert_eq!(bare(&done.inverse), bare(&applied.inverse), "step {step}");
            assert_eq!(done.added, applied.added, "step {step}");
        }
        // Written part of the way, so that the root goes on from what a flush left.
        if step == 40 {
            session.flush().unwrap();
        }
    }
    let written = session.flush().unwrap();
    assert!(written.iter().all(|source| source.proven), "{written:?}");
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new()
    );
    assert_eq!(session.map(HARBOUR).unwrap().entities.len(), 100);
    assert_eq!(session.types(TYPES).unwrap().archetypes.len(), 98);
}

/// What a session holds of the three sources, as the files they would save as.
fn roots(session: &mut Session) -> [Vec<u8>; 3] {
    [
        session.map(HARBOUR).unwrap().to_bytes().unwrap(),
        session.map(YARD).unwrap().to_bytes().unwrap(),
        session.types(TYPES).unwrap().to_bytes().unwrap(),
    ]
}

#[test]
fn an_operation_refused_in_the_middle_of_a_list_leaves_the_roots_and_the_disk_as_they_were() {
    let (_scratch, [one_by_one, held]) = copies(pack);
    let imported = files(&held);
    let before = [place(HARBOUR, 1, [420.0, -66.5, 12.25]), remove(0)];
    let after = place(HARBOUR, 2, [-5.0, 5.0, 5.0]);
    let record = Record::entity(&read_entity(&held.join(HARBOUR), 3).unwrap()).unwrap();
    let insert = |parent_num_children, extents| Operation::InsertEntity {
        source: HARBOUR.into(),
        entity: 1,
        record: record.clone(),
        parent_num_children,
        children: vec![0],
        extents,
        expects: None,
    };
    let replace = |record: Record, extents| Operation::ReplaceEntity {
        source: HARBOUR.into(),
        entity: 1,
        record,
        extents,
        expects: None,
    };
    let refused: Vec<(Operation, &str)> = vec![
        (place(HARBOUR, 9, [0.0; 3]), "entity 9 is past the 3"),
        (remove(3), "entity 3 is past the 3"),
        (place(TYPES, 0, [0.0; 3]), "does not edit a type file"),
        (
            Operation::GrowExtents {
                source: "../elsewhere.metadata.src".into(),
                expects: None,
            },
            "no map set of the pack names this source",
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
            Operation::RewriteSource {
                source: HARBOUR.into(),
                xml: "<CMapData><entities>".into(),
                expects: None,
            },
            "metadata.xml",
        ),
        // Each of these was refused only after it had changed the map, which a session keeps.
        (
            Operation::RemoveEntity {
                source: HARBOUR.into(),
                entity: 1,
                extents: Some(no_number()),
                expects: None,
            },
            "finite",
        ),
        (replace(record.clone(), Some(no_number())), "finite"),
        (insert(None, Some(no_number())), "finite"),
        (insert(Some(4), None), "names no LOD parent"),
        (
            Operation::InsertEntity {
                source: HARBOUR.into(),
                entity: 1,
                record: with(&record, "parentIndex", Value::Int(1)),
                parent_num_children: None,
                children: Vec::new(),
                extents: None,
                expects: None,
            },
            "entity 1 would be its own LOD parent",
        ),
        (
            Operation::InsertEntity {
                source: HARBOUR.into(),
                entity: 1,
                record: with(&record, "parentIndex", Value::Int(4)),
                parent_num_children: None,
                children: Vec::new(),
                extents: None,
                expects: None,
            },
            "past the 4 entities",
        ),
        // A LOD parent a save refuses is refused when the record is put in, in a session as one by one.
        (
            replace(with(&record, "parentIndex", Value::Int(1)), None),
            "entity 1 would be its own LOD parent",
        ),
        (
            replace(with(&record, "parentIndex", Value::Int(3)), None),
            "past the 3 entities",
        ),
    ];

    for operation in &before {
        apply(&one_by_one, operation).unwrap();
    }
    let mut session = Session::open(&held).unwrap();
    for operation in &before {
        session.apply(operation).unwrap();
    }
    let kept = roots(&mut session);
    for (operation, why) in &refused {
        match session.apply(operation) {
            Err(Error::Refused {
                name,
                at,
                why: said,
            }) => {
                assert_eq!((name, at.as_str()), (operation.name(), operation.source()));
                assert!(said.contains(why), "{name}: {said} says {why}");
            }
            other => panic!("{} was not refused: {other:?}", operation.name()),
        }
        assert!(
            roots(&mut session) == kept,
            "{} ({why}) changed a root it was refused on",
            operation.name()
        );
        assert_eq!(
            differing(&imported, &files(&held)),
            Vec::<PathBuf>::new(),
            "{} ({why}) wrote",
            operation.name()
        );
        let one = apply(&one_by_one, operation).unwrap_err().to_string();
        assert!(one.contains(why), "one by one, {one} says {why}");
    }

    // The operations before and after the refused ones are done as if none had been tried.
    apply(&one_by_one, &after).unwrap();
    session.apply(&after).unwrap();
    session.flush().unwrap();
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn a_flush_the_disk_refuses_puts_back_what_it_wrote_and_the_roots_are_kept() {
    let (_scratch, [one_by_one, held]) = copies(pack);
    let imported = files(&held);
    let list = [
        place(HARBOUR, 1, [420.0, -66.5, 12.25]),
        place(YARD, 0, [60.0, 61.0, 1.0]),
    ];
    let mut session = Session::open(&held).unwrap();
    for operation in &list {
        apply(&one_by_one, operation).unwrap();
        session.apply(operation).unwrap();
    }
    // ⚠ A folder where the writer puts the XML before it renames it: `yard` is written after `harbour`, and cannot be.
    let in_the_way = held.join(YARD).join("metadata.xml.part");
    std::fs::create_dir(&in_the_way).unwrap();

    let refused = session.flush().unwrap_err();
    assert!(matches!(refused, Error::Write(_)), "{refused:?}");
    assert!(refused.to_string().contains("metadata.xml"), "{refused}");
    assert_eq!(
        differing(&imported, &files(&held)),
        Vec::<PathBuf>::new(),
        "the source written before the one that could not be is put back"
    );
    assert!(session.unwritten());

    std::fs::remove_dir(&in_the_way).unwrap();
    assert_eq!(session.flush().unwrap().len(), 2);
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn a_flush_writes_over_no_source_that_was_changed_since_the_session_read_it() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let mut session = Session::open(&folder).unwrap();
    session
        .apply(&place(HARBOUR, 1, [420.0, -66.5, 12.25]))
        .unwrap();
    session.apply(&place(YARD, 0, [60.0, 61.0, 1.0])).unwrap();
    // Something that is not the session edits a source the session holds.
    apply(&folder, &place(YARD, 0, [-7.0, 8.0, 9.0])).unwrap();
    let edited = files(&folder);

    let refused = session.flush().unwrap_err();
    assert!(
        matches!(&refused, Error::Changed { at } if at == YARD),
        "{refused:?}"
    );
    assert!(refused
        .to_string()
        .contains("was changed on disk since the session read it"));
    assert_eq!(
        differing(&edited, &files(&folder)),
        Vec::<PathBuf>::new(),
        "neither the source that changed nor the one that did not is written"
    );
    drop(session);
    assert_eq!(
        differing(&edited, &files(&folder)),
        Vec::<PathBuf>::new(),
        "nor by the drop"
    );
}

#[test]
fn a_flush_that_cannot_write_an_operation_names_it_and_writes_nothing() {
    let (_scratch, [one_by_one, held]) = copies(pack);
    let imported = files(&held);
    let add = |name: &str| Operation::AddArchetype {
        source: TYPES.into(),
        name: name.into(),
        asset: None,
        donor: None,
        expects: None,
    };
    let broken = Operation::ReplaceArchetype {
        source: TYPES.into(),
        archetype: 0,
        record: a_broken_interior(&held),
        expects: None,
    };
    let said = "attaches object 5, past its 0 entities";
    // One by one, the save refuses it at the operation.
    let one = apply(&one_by_one, &broken).unwrap_err().to_string();
    assert!(one.contains(said), "{one}");

    let writable = [place(HARBOUR, 1, [420.0, -66.5, 12.25]), add("prop_one")];
    let mut session = Session::open(&held).unwrap();
    for operation in &writable {
        apply(&one_by_one, operation).unwrap();
        session.apply(operation).unwrap();
    }
    let second = session.apply(&broken).unwrap();
    let third = session.apply(&add("prop_two")).unwrap();

    let refused = session.flush().unwrap_err();
    match &refused {
        Error::Unwritten {
            at,
            why,
            number,
            of,
            name,
        } => {
            assert_eq!(
                (at.as_str(), *number, *of, *name),
                (TYPES, 2, 3, "replace_archetype")
            );
            assert!(why.contains(said), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        refused
            .to_string()
            .contains("operation 2 of the 3 done to it since it was last written"),
        "{refused}"
    );
    assert_eq!(
        differing(&imported, &files(&held)),
        Vec::<PathBuf>::new(),
        "a refused flush writes no source, the map file that could be written no more than the type file"
    );
    assert!(session.unwritten());

    // The operations from the one named on are taken back, and what is left is written.
    session.apply(&third.inverse).unwrap();
    session.apply(&second.inverse).unwrap();
    assert_eq!(session.flush().unwrap().len(), 2);
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn an_inverse_applied_after_another_edit_is_refused_and_applied_in_order_puts_every_byte_back() {
    let first = place(HARBOUR, 1, [420.0, -66.5, 12.25]);
    let second = remove(3);
    let sentence = "the source is not as the operation expects it, so it was edited since the operation was made";

    // One by one: the inverse states the digest of the XML its operation wrote.
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let imported = files(&folder);
    let a = apply(&folder, &first).unwrap();
    assert_eq!(
        a.inverse.expects(),
        Some(digest_of(&a.edited.after).as_str())
    );
    let b = apply(&folder, &second).unwrap();
    let edited = files(&folder);
    let stale = apply(&folder, &a.inverse).unwrap_err();
    match &stale {
        Error::Stale {
            name,
            at,
            expects,
            found,
        } => {
            assert_eq!((*name, at.as_str()), ("replace_entity", HARBOUR));
            assert_eq!(Some(expects.as_str()), a.inverse.expects());
            assert_eq!(Some(found.as_str()), b.inverse.expects());
        }
        other => panic!("{other:?}"),
    }
    assert!(stale.to_string().contains(sentence), "{stale}");
    let in_a_list = apply_all(&folder, &[b.inverse.clone(), b.inverse.clone()], false).unwrap_err();
    assert!(
        in_a_list
            .to_string()
            .starts_with("operation 2 of 2, insert_entity")
            && in_a_list.to_string().contains(sentence),
        "{in_a_list}"
    );
    assert_eq!(differing(&edited, &files(&folder)), Vec::<PathBuf>::new());
    apply_all(&folder, &[b.inverse.clone(), a.inverse.clone()], false).unwrap();
    assert_eq!(differing(&imported, &files(&folder)), Vec::<PathBuf>::new());

    // In a session, with nothing written: the inverse states a token of the session.
    let mut session = Session::open(&folder).unwrap();
    let a = session.apply(&first).unwrap();
    let b = session.apply(&second).unwrap();
    let kept = roots(&mut session);
    let stale = session.apply(&a.inverse).unwrap_err();
    assert!(matches!(stale, Error::Stale { .. }), "{stale:?}");
    assert!(stale.to_string().contains(sentence), "{stale}");
    assert!(roots(&mut session) == kept);
    let b_back = session.apply(&b.inverse).unwrap();
    let a_back = session.apply(&a.inverse).unwrap();
    assert!(
        !session.unwritten(),
        "operations taken back in order leave nothing to write"
    );
    assert_eq!(session.flush().unwrap(), []);
    assert_eq!(differing(&imported, &files(&folder)), Vec::<PathBuf>::new());

    // Forward again by the inverses of the inverses, each giving the inverse it gave the first time.
    let out_of_order = session.apply(&b_back.inverse).unwrap_err();
    assert!(
        matches!(out_of_order, Error::Stale { .. }),
        "{out_of_order:?}"
    );
    assert_eq!(session.apply(&a_back.inverse).unwrap().inverse, a.inverse);
    assert_eq!(session.apply(&b_back.inverse).unwrap().inverse, b.inverse);
    let [written] = session.flush().unwrap().try_into().unwrap();
    assert_eq!(differing(&edited, &files(&folder)), Vec::<PathBuf>::new());

    // Across the flush the session still holds each inverse to its order.
    let stale = session.apply(&a.inverse).unwrap_err();
    assert!(matches!(stale, Error::Stale { .. }), "{stale:?}");
    // What reads the disk knows no token: the inverse is refused there, and done once it states the digest the flush gave.
    let unknown = apply(&folder, &b.inverse).unwrap_err();
    assert!(matches!(unknown, Error::Stale { .. }), "{unknown:?}");
    assert_eq!(differing(&edited, &files(&folder)), Vec::<PathBuf>::new());
    session
        .apply(&b.inverse.clone().expecting(written.digest))
        .unwrap();
    session.apply(&a.inverse).unwrap();
    session.flush().unwrap();
    assert_eq!(differing(&imported, &files(&folder)), Vec::<PathBuf>::new());
    drop(session);

    // Another session knows no token of the first.
    let mut session = Session::open(&folder).unwrap();
    let foreign = session.apply(&b.inverse).unwrap_err();
    assert!(matches!(foreign, Error::Stale { .. }), "{foreign:?}");
    let a = session.apply(&first).unwrap();
    let [written] = session.flush().unwrap().try_into().unwrap();
    drop(session);
    apply(&folder, &a.inverse.expecting(written.digest)).unwrap();
    assert_eq!(differing(&imported, &files(&folder)), Vec::<PathBuf>::new());
}

#[test]
fn a_session_dropped_with_operations_pending_writes_them() {
    let (_scratch, [one_by_one, held]) = copies(pack);
    let imported = files(&held);
    let list = [
        place(HARBOUR, 1, [420.0, -66.5, 12.25]),
        remove(0),
        place(YARD, 0, [1.0; 3]),
    ];
    {
        let mut session = Session::open(&held).unwrap();
        for operation in &list {
            apply(&one_by_one, operation).unwrap();
            session.apply(operation).unwrap();
        }
        assert!(session.unwritten());
        assert_eq!(differing(&imported, &files(&held)), Vec::<PathBuf>::new());
    }
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new(),
        "the drop wrote what no flush had"
    );

    // A drop whose flush is refused writes nothing: the refusal is logged, which is all a drop can do with it.
    let broken = Operation::ReplaceArchetype {
        source: TYPES.into(),
        archetype: 0,
        record: a_broken_interior(&held),
        expects: None,
    };
    let written = files(&held);
    {
        let mut session = Session::open(&held).unwrap();
        session.apply(&place(HARBOUR, 2, [7.0; 3])).unwrap();
        session.apply(&broken).unwrap();
    }
    assert_eq!(differing(&written, &files(&held)), Vec::<PathBuf>::new());
}

/// Do `operation` one by one in a pack and through a session in another, both made by `make`: the two give one inverse, and each
/// way's, applied newest first to what that way wrote, puts every byte back.
fn the_two_ways_give_one_inverse(
    make: fn(&Path) -> PathBuf,
    operations: &[&dyn Fn(&Path) -> Operation],
    taken_back_by: &[&str],
) {
    let (_scratch, [one_by_one, held]) = copies(make);
    let before = files(&held);
    let mut session = Session::open(&held).unwrap();
    let mut inverses = Vec::new();
    let mut on_disk = Vec::new();
    for (operation, by) in operations.iter().zip(taken_back_by) {
        let operation = operation(&one_by_one);
        let applied = apply(&one_by_one, &operation).unwrap();
        let done = session.apply(&operation).unwrap();
        assert_eq!(applied.inverse.name(), *by, "{}", operation.name());
        assert_eq!(
            bare(&done.inverse),
            bare(&applied.inverse),
            "{}",
            operation.name()
        );
        inverses.push(done.inverse);
        on_disk.push(applied.inverse);
    }
    session.flush().unwrap();
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new()
    );
    for inverse in inverses.iter().rev() {
        session.apply(inverse).unwrap();
    }
    session.flush().unwrap();
    assert_eq!(differing(&before, &files(&held)), Vec::<PathBuf>::new());
    // ⚠ Each one by one reads the source again from the XML the operation before it wrote, which is not the file the inverse was proven on.
    for inverse in on_disk.iter().rev() {
        apply(&one_by_one, inverse)
            .unwrap_or_else(|e| panic!("the inverse {} one by one: {e}", inverse.name()));
    }
    assert_eq!(
        differing(&before, &files(&one_by_one)),
        Vec::<PathBuf>::new()
    );
}

/// A file read again from the XML an operation wrote names no class it holds no record of, and saves no record of one: the inverse of
/// the operation that took the last record of a class out is the XML, in a session as one by one.
#[test]
fn the_last_record_of_a_class_taken_out_of_a_file_is_put_back_by_the_xml() {
    let take_out = |source: &'static str, entity| {
        move |_: &Path| Operation::RemoveEntity {
            source: source.into(),
            entity,
            extents: None,
            expects: None,
        }
    };
    let archetype_out = |archetype| {
        move |_: &Path| Operation::RemoveArchetype {
            source: TYPES.into(),
            archetype,
            expects: None,
        }
    };
    // The one entity of `yard`; in `harbour`, every entity but the last is put back as a record.
    the_two_ways_give_one_inverse(pack, &[&take_out(YARD, 0)], &["rewrite_source"]);
    the_two_ways_give_one_inverse(
        pack,
        &[
            &take_out(HARBOUR, 3),
            &take_out(HARBOUR, 0),
            &take_out(HARBOUR, 1),
            &take_out(HARBOUR, 0),
        ],
        &[
            "insert_entity",
            "insert_entity",
            "insert_entity",
            "rewrite_source",
        ],
    );
    the_two_ways_give_one_inverse(
        pack,
        &[&archetype_out(1), &archetype_out(0)],
        &["insert_archetype", "rewrite_source"],
    );

    // What the file no longer names, neither way adds to it: a session keeps no root that still could. The first entity of a map
    // that holds none is built and no copy, and these samples are RBM, whose class carries a signature nothing derives.
    let (_scratch, [one_by_one, held]) = copies(pack);
    let add = Operation::AddEntity {
        source: YARD.into(),
        archetype: "prop_crate_a".into(),
        pose: pose([1.0, 2.0, 3.0]),
        donor: Some(At {
            source: HARBOUR.into(),
            index: 0,
        }),
        expects: None,
    };
    let mut session = Session::open(&held).unwrap();
    apply(&one_by_one, &take_out(YARD, 0)(&one_by_one)).unwrap();
    session.apply(&take_out(YARD, 0)(&held)).unwrap();
    session.flush().unwrap();
    let emptied = files(&held);
    assert_eq!(
        differing(&files(&one_by_one), &emptied),
        Vec::<PathBuf>::new()
    );
    let one = apply(&one_by_one, &add).unwrap_err().to_string();
    assert!(one.contains("signature"), "{one}");
    // ⚠ At the operation and not at the flush: a record built here is saved before it is kept, since nothing else shows the file
    // lays it out, and a root left holding one it cannot save could not give the record its removal carries.
    let refused = session.apply(&add).unwrap_err();
    assert!(
        matches!(
            &refused,
            Error::Refused { name: "add_entity", at, why } if at == YARD && why.contains("signature")
        ),
        "{refused:?}"
    );
    assert!(!session.unwritten());
    assert_eq!(session.flush().unwrap(), Vec::new());
    assert_eq!(differing(&emptied, &files(&held)), Vec::<PathBuf>::new());
}

#[test]
fn where_an_inverse_does_not_hold_by_construction_a_session_gives_what_the_proof_gives() {
    // XML typed by hand: the first edit rewrites it whole, and the second edits what the first wrote.
    fn typed_by_hand(dir: &Path) -> PathBuf {
        let folder = pack(dir);
        let file = folder.join(HARBOUR).join("metadata.xml");
        let exported = std::fs::read_to_string(&file).unwrap();
        let typed = exported.replacen("<entities", "\n\n      <entities", 1);
        assert_ne!(typed, exported);
        std::fs::write(&file, typed).unwrap();
        folder
    }
    the_two_ways_give_one_inverse(
        typed_by_hand,
        &[&|_| place(HARBOUR, 1, [420.0, -66.5, 12.25]), &|_| {
            place(HARBOUR, 2, [1.0, 2.0, 3.0])
        }],
        &["rewrite_source", "replace_entity"],
    );

    // An insertion that claims a child another entity has, after an operation the session has not written; and one that claims a free one.
    let claims = |children: Vec<usize>| {
        move |folder: &Path| Operation::InsertEntity {
            source: HARBOUR.into(),
            entity: 4,
            record: Record::entity(&read_entity(&folder.join(HARBOUR), 0).unwrap()).unwrap(),
            parent_num_children: None,
            children: children.clone(),
            extents: None,
            expects: None,
        }
    };
    the_two_ways_give_one_inverse(
        pack,
        &[&|_| place(HARBOUR, 2, [1.0, 2.0, 3.0]), &claims(vec![1])],
        &["replace_entity", "rewrite_source"],
    );
    the_two_ways_give_one_inverse(pack, &[&claims(vec![3])], &["remove_entity"]);
    // A count its parent would not get back from a removal.
    the_two_ways_give_one_inverse(
        pack,
        &[&|folder| Operation::InsertEntity {
            source: HARBOUR.into(),
            entity: 4,
            record: Record::entity(&read_entity(&folder.join(HARBOUR), 1).unwrap()).unwrap(),
            parent_num_children: Some(7),
            children: Vec::new(),
            extents: None,
            expects: None,
        }],
        &["rewrite_source"],
    );

    // A map with an entity that is its own LOD parent, which no save writes: taking it out is taken back by the XML.
    fn looped(dir: &Path) -> PathBuf {
        let quay = map_file(
            "harbour",
            &[
                Entity {
                    parent: 0,
                    ..Entity::at("prop_crate_a", [1.0, 2.0, 0.5], 1)
                },
                Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 2),
            ],
            ([0.0; 3], [100.0; 3]),
        );
        pack_of(dir, vec![incoming("harbour.ymap", quay)])
    }
    the_two_ways_give_one_inverse(
        looped,
        &[&|_| remove(0), &|_| place(HARBOUR, 0, [3.0; 3])],
        &["rewrite_source", "replace_entity"],
    );

    // A type file that declares a name twice: an archetype taken out or renamed is not put back by an insertion or a replacement.
    fn twice(dir: &Path) -> PathBuf {
        let types = types_file(
            "harbour_types",
            &[
                Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
                Archetype::new("prop_crate_a", [-2.0; 3], [2.0; 3]),
                Archetype::new("prop_crate_b", [-3.0; 3], [3.0; 3]),
            ],
        );
        pack_of(dir, vec![incoming("harbour_types.ytyp", types)])
    }
    let take_out = |archetype| {
        move |_: &Path| Operation::RemoveArchetype {
            source: TYPES.into(),
            archetype,
            expects: None,
        }
    };
    the_two_ways_give_one_inverse(twice, &[&take_out(1)], &["rewrite_source"]);
    the_two_ways_give_one_inverse(twice, &[&take_out(2)], &["insert_archetype"]);
    the_two_ways_give_one_inverse(
        twice,
        &[&|folder| {
            let mut archetype = read_archetype(&folder.join(TYPES), 0).unwrap();
            archetype.name = HashedString::new("prop_crate_c");
            Operation::ReplaceArchetype {
                source: TYPES.into(),
                archetype: 0,
                record: Record::archetype(&archetype).unwrap(),
                expects: None,
            }
        }],
        &["rewrite_source"],
    );
}

/// Numbers that are the same at every run, so a list made of them is the same list.
struct Dice(u64);

impl Dice {
    fn roll(&mut self, sides: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % sides.max(1) as u64) as usize
    }

    /// A length within a kilometre either way, to the millimetre.
    fn metres(&mut self) -> f32 {
        self.roll(2_000_001) as f32 / 1000.0 - 1000.0
    }

    fn corner(&mut self) -> [f32; 3] {
        [self.metres(), self.metres(), self.metres()]
    }

    fn pose(&mut self) -> PoseData {
        let turn = glam::EulerRot::XYZ;
        PoseData {
            position: self.corner(),
            rotation: glam::Quat::from_euler(turn, self.metres(), self.metres(), self.metres())
                .to_array(),
            scale_xy: 0.5 + self.roll(4) as f32,
            scale_z: 0.25 + self.roll(3) as f32,
        }
    }
}

/// The next operation of a list made at random: an index may be one past the end, a name may be declared already, and an insertion
/// names any parent and any children, so that some are refused and some are taken back by the XML. `written` is the pack as one by
/// one has written it, whose XML a rewrite edits.
fn at_random(dice: &mut Dice, session: &mut Session, written: &Path) -> Operation {
    let names = ["prop_crate_a", "prop_crate_b", "Prop_New_1", "prop_new_2"];
    let source = [HARBOUR, YARD][dice.roll(2)].to_string();
    let entities = session.map(&source).unwrap().entities.len();
    let archetypes = session.types(TYPES).unwrap().archetypes.len();
    match dice.roll(11) {
        0 | 1 => Operation::PlaceEntity {
            source,
            entity: dice.roll(entities + 1),
            pose: dice.pose(),
            expects: None,
        },
        2 => Operation::AddEntity {
            archetype: names[dice.roll(names.len())].into(),
            pose: dice.pose(),
            donor: match dice.roll(3) {
                0 => None,
                1 => Some(At {
                    source: source.clone(),
                    index: dice.roll(entities + 1),
                }),
                _ => Some(At {
                    source: [HARBOUR, YARD][dice.roll(2)].into(),
                    index: 0,
                }),
            },
            source,
            expects: None,
        },
        3 => Operation::RemoveEntity {
            source,
            entity: dice.roll(entities + 1),
            extents: None,
            expects: None,
        },
        4 => Operation::GrowExtents {
            source,
            expects: None,
        },
        5 => Operation::SetExtents {
            source,
            extents: Extents {
                entities_extents_min: dice.corner(),
                entities_extents_max: dice.corner(),
                streaming_extents_min: dice.corner(),
                streaming_extents_max: dice.corner(),
            },
            expects: None,
        },
        6 => Operation::AddArchetype {
            source: TYPES.into(),
            name: names[dice.roll(names.len())].into(),
            asset: None,
            donor: None,
            expects: None,
        },
        7 => Operation::RemoveArchetype {
            source: TYPES.into(),
            archetype: dice.roll(archetypes + 1),
            expects: None,
        },
        8 if archetypes > 0 => {
            let archetype = dice.roll(archetypes);
            let mut record = session.types(TYPES).unwrap().archetypes[archetype].clone();
            record.bb_max.z = dice.metres().abs();
            if dice.roll(4) == 0 {
                record.name = HashedString::new(names[dice.roll(names.len())]);
            }
            Operation::ReplaceArchetype {
                source: TYPES.into(),
                archetype,
                record: Record::archetype(&record).unwrap(),
                expects: None,
            }
        }
        9 if entities > 0 => {
            let mut record = session.map(&source).unwrap().entities[dice.roll(entities)].clone();
            record.parent_index = dice.roll(entities + 2) as i32 - 1;
            record.num_children = dice.roll(3) as u32;
            Operation::InsertEntity {
                source,
                entity: dice.roll(entities + 1),
                record: Record::entity(&record).unwrap(),
                parent_num_children: (dice.roll(2) == 0).then(|| dice.roll(4) as u32),
                children: (0..dice.roll(3)).map(|_| dice.roll(entities)).collect(),
                extents: None,
                expects: None,
            }
        }
        10 => {
            let xml = std::fs::read_to_string(written.join(&source).join("metadata.xml")).unwrap();
            let (from, to) = [("65", "66"), ("66", "65")][usize::from(xml.contains("\"66\""))];
            let flags = |value: &str| format!("contentFlags value=\"{value}\"");
            Operation::RewriteSource {
                source,
                xml: xml.replace(&flags(from), &flags(to)),
                expects: None,
            }
        }
        _ => Operation::GrowExtents {
            source,
            expects: None,
        },
    }
}

/// Lists no person wrote find what a list a person wrote does not: one made at random, with steps taken back and made again as
/// Undo and Redo do, is refused, added to and taken back alike both ways, and at each flush the files are the same.
#[test]
fn lists_made_at_random_with_steps_taken_back_and_made_again_are_written_alike_both_ways() {
    let not_named = "is not in the schema";
    for seed in 1..=16u64 {
        let (_scratch, [one_by_one, held]) = copies(pack);
        let mut session = Session::open(&held).unwrap();
        let mut dice = Dice(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        // Each step both ways: the session's inverse and the one that reads the disk.
        let (mut back, mut again) = (Vec::new(), Vec::new());
        for step in 0..60 {
            let history = match dice.roll(5) {
                0 => Some(true),
                1 => Some(false),
                _ => None,
            };
            let (in_session, on_disk) = match history {
                Some(true) if !back.is_empty() => back.pop().unwrap(),
                Some(false) if !again.is_empty() => again.pop().unwrap(),
                Some(_) => continue,
                None => {
                    let operation = at_random(&mut dice, &mut session, &one_by_one);
                    (operation.clone(), operation)
                }
            };
            let said = format!("seed {seed}, step {step}, {}", in_session.name());
            match (apply(&one_by_one, &on_disk), session.apply(&in_session)) {
                (Ok(applied), Ok(done)) => {
                    assert_eq!(done.added, applied.added, "{said}");
                    assert_eq!(done.inverse.name(), applied.inverse.name(), "{said}");
                    let inverses = (done.inverse, applied.inverse);
                    match history {
                        Some(true) => again.push(inverses),
                        Some(false) => back.push(inverses),
                        None => {
                            again.clear();
                            back.push(inverses);
                        }
                    }
                }
                // A record of a class the file no longer names: one by one the save refuses it, and a session's flush does.
                (Err(refused), Ok(done)) if refused.to_string().contains(not_named) => {
                    assert!(history.is_none(), "{said}: a step of history: {refused}");
                    let flush = session.flush().unwrap_err().to_string();
                    assert!(flush.contains(not_named), "{said}: {flush}");
                    session.apply(&done.inverse).unwrap();
                }
                (Err(one), Err(other)) => {
                    assert!(history.is_none(), "{said}: a step of history: {one}");
                    assert_eq!(one.to_string(), other.to_string(), "{said}");
                }
                (one, other) => panic!(
                    "{said}: one by one {:?}, and in a session {:?}",
                    one.map(|applied| applied.inverse),
                    other.map(|done| done.inverse)
                ),
            }
            if dice.roll(5) == 0 || step == 59 {
                let written = session.flush().unwrap_or_else(|e| panic!("{said}: {e}"));
                assert!(written.iter().all(|source| source.proven), "{said}");
                assert_eq!(
                    differing(&files(&one_by_one), &files(&held)),
                    Vec::<PathBuf>::new(),
                    "{said}"
                );
            }
        }
    }
}

/// Two states of a source may be alike: an entity put back by hand where it stood, a step of a drag that moved nothing. The step
/// taken back from the state between them and the step made again from it are then one operation, and each still leads where its
/// own step led, so every inverse given before is done in its order.
#[test]
fn inverses_are_done_in_their_order_though_two_states_of_their_source_were_alike() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let far = place(YARD, 0, [-45.0, 5.0, 2.0]);
    let near = place(YARD, 0, [-42.0, 5.0, 2.0]);
    let other = Operation::SetExtents {
        source: YARD.into(),
        extents: Extents {
            entities_extents_min: [-90.0; 3],
            entities_extents_max: [90.0; 3],
            streaming_extents_min: [-190.0; 3],
            streaming_extents_max: [190.0; 3],
        },
        expects: None,
    };
    // The extents hold the entity at both places from here on, so a move between the two grows nothing.
    for operation in [&far, &near, &far] {
        apply(&folder, operation).unwrap();
    }
    let start = files(&folder);

    // Moved, and moved back by hand to where it stood.
    let mut session = Session::open(&folder).unwrap();
    let moved = session.apply(&near).unwrap();
    let back_by_hand = session.apply(&far).unwrap();
    let again = session.apply(&back_by_hand.inverse).unwrap();
    assert_eq!(
        bare(&again.inverse),
        bare(&moved.inverse),
        "making the second move again and taking the first back are one operation"
    );
    let made_again = session.apply(&again.inverse).unwrap();
    let after = session.apply(&other).unwrap();
    for inverse in [&after.inverse, &made_again.inverse, &moved.inverse] {
        session
            .apply(inverse)
            .unwrap_or_else(|e| panic!("{} in its order: {e}", inverse.name()));
    }
    assert!(!session.unwritten());
    drop(session);

    // A step made again from the state the entity was put back by hand to, which the session knows by another name: the one it gave
    // the state the entity first stood in.
    let mut session = Session::open(&folder).unwrap();
    let moved = session.apply(&near).unwrap();
    let back_by_hand = session.apply(&far).unwrap();
    let after = session.apply(&other).unwrap();
    let after_again = session.apply(&after.inverse).unwrap();
    let again = session.apply(&back_by_hand.inverse).unwrap();
    let mut made_again = Vec::new();
    for step in [&again.inverse, &after_again.inverse] {
        let done = session
            .apply(step)
            .unwrap_or_else(|e| panic!("{} made again: {e}", step.name()));
        made_again.push(done.inverse);
    }
    for inverse in [&made_again[1], &made_again[0], &moved.inverse] {
        session
            .apply(inverse)
            .unwrap_or_else(|e| panic!("{} in its order: {e}", inverse.name()));
    }
    assert!(!session.unwritten());
    drop(session);

    // A drag whose steps after the first move nothing.
    let mut session = Session::open(&folder).unwrap();
    let steps = [(); 3].map(|()| session.apply(&near).unwrap());
    let again = session.apply(&steps[2].inverse).unwrap();
    let made_again = session.apply(&again.inverse).unwrap();
    let after = session.apply(&other).unwrap();
    for inverse in [
        &after.inverse,
        &made_again.inverse,
        &steps[1].inverse,
        &steps[0].inverse,
    ] {
        session
            .apply(inverse)
            .unwrap_or_else(|e| panic!("{} in its order: {e}", inverse.name()));
    }
    assert!(!session.unwritten());
    assert_eq!(session.flush().unwrap(), []);
    assert_eq!(differing(&start, &files(&folder)), Vec::<PathBuf>::new());
}

/// A source knows the way of the inverses of its last thousand operations and of the thousand that take those back, and no more: a
/// thousand steps of a longer drag are taken back in their order.
#[test]
fn a_thousand_steps_of_a_longer_drag_are_taken_back_in_their_order() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let along = |step: usize| step as f32 * 0.25 - 40.0;
    let mut session = Session::open(&folder).unwrap();
    let given: Vec<Operation> = (0..1500)
        .map(|step| {
            let moved = session.apply(&place(YARD, 0, [along(step), 5.0, 2.0]));
            moved.unwrap().inverse
        })
        .collect();
    for (taken, inverse) in given.iter().rev().take(1000).enumerate() {
        session
            .apply(inverse)
            .unwrap_or_else(|e| panic!("step {taken} taken back: {e}"));
    }
    assert_eq!(
        session.map(YARD).unwrap().entities[0].position.x,
        along(499)
    );
}
