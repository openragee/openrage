//! Three things were built apart and meet in `or_maps::operation`: a session that holds the sources open, the deletion of an entity
//! with its chain across files, and what an operation does where the pack holds less than it reads. Each is held here through the
//! other two: a chain deletion done through a session and taken back by its inverses leaves every byte, the links of a child map are
//! renumbered so the rule that names a broken one stays silent, and a session says and does what one by one says and does for an
//! archetype the pack does not declare, a type file that does not cook and the first record of a file that holds none.
//!
//! ⚠ The files are the crate's samples and what `or_world::create` makes, so the clean checkout runs this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glam::Vec3;
use or_io::meta::BinaryFormat;
use or_maps::delete;
use or_maps::edit::read_entity;
use or_maps::import::{add_files, Incoming};
use or_maps::model::Maps;
use or_maps::operation::{
    apply, apply_all, inverses, At, Done, Error, Extents, Inserted, Operation, PoseData, Record,
    Session,
};
use or_maps::sample::{child_map_file, map_file, types_file, Archetype, Entity};
use or_maps::{check, inspect, Depth};
use or_world::archetypes::ArchetypeIndex;
use or_world::create;

const SLOD: &str = "city_slod.metadata.src";
const LOD: &str = "city_lod.metadata.src";
const CITY: &str = "city.metadata.src";
const HARBOUR: &str = "harbour.metadata.src";
const YARD: &str = "yard.metadata.src";
const TYPES: &str = "harbour_types.metadata.src";
const BENCH: &str = "prop_bench_01a";

/// A pack of the files `files`, by the name each ships under.
fn pack_of(dir: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
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
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
    }
}

fn at(source: &str, index: usize) -> At {
    At {
        source: source.to_string(),
        index,
    }
}

fn codes(folder: &Path) -> Vec<&'static str> {
    let model = Maps::read(folder).unwrap();
    let facts = inspect(folder, &model, Depth::Open);
    check(&model, &facts, None).iter().map(|f| f.code).collect()
}

/// An entity that names the entity `parent` of the parent map as its LOD parent.
fn under<'a>(archetype: &'a str, position: [f32; 3], guid: u32, parent: i32) -> Entity<'a> {
    Entity {
        parent,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    }
}

/// Three map files, each the parent map of the next. `city_slod` holds a block that stands for the far models of the tower and of
/// the row, and a far model of the shed's far model; `city_lod` holds those three far models, each under `city_slod`; `city` holds
/// the tower, two houses and the shed, each under its far model in `city_lod`, and a length of fence under a far model of its own
/// file.
fn city(dir: &Path) -> PathBuf {
    let extents = ([0.0; 3], [100.0; 3]);
    let slod = map_file(
        "city_slod",
        &[
            Entity {
                children: 2,
                ..Entity::at("slod_block", [10.0, 10.0, 0.0], 101)
            },
            Entity {
                children: 1,
                ..Entity::at("slod_far", [90.0, 10.0, 0.0], 102)
            },
        ],
        extents,
    );
    let lod = child_map_file(
        "city_lod",
        "city_slod",
        &[
            Entity {
                children: 1,
                ..under("lod_tower", [10.0, 10.0, 0.0], 201, 0)
            },
            Entity {
                children: 2,
                ..under("lod_row", [20.0, 10.0, 0.0], 202, 0)
            },
            Entity {
                children: 1,
                ..under("lod_shed", [90.0, 10.0, 0.0], 203, 1)
            },
        ],
        extents,
    );
    let city = child_map_file(
        "city",
        "city_lod",
        &[
            under("tower", [10.0, 12.0, 3.0], 301, 0),
            under("house", [18.0, 10.0, 0.0], 302, 1),
            under("house", [22.0, 10.0, 0.0], 303, 1),
            under("shed", [90.0, 10.0, 0.0], 304, 2),
            Entity {
                children: 1,
                ..Entity::at("fence_far", [70.0, 70.0, 0.0], 305)
            },
            Entity {
                parent: 4,
                ..Entity::at("fence", [70.0, 70.0, 0.0], 306)
            },
        ],
        extents,
    );
    pack_of(
        dir,
        &[
            ("city_slod.ymap", slod),
            ("city_lod.ymap", lod),
            ("city.ymap", city),
        ],
    )
}

/// The tower and the shed of `city`: the far model of each goes with it, the far model of the shed's far model too, and the block
/// stays, since it still stands for the row.
fn tower_and_shed() -> [At; 2] {
    [at(CITY, 0), at(CITY, 3)]
}

#[test]
fn a_chain_deletion_done_through_a_session_and_taken_back_by_its_inverses_restores_every_byte() {
    let (_scratch, [one_by_one, held, never_flushed]) = copies(city);
    let before = files(&held);
    let planned = delete::plan(&held, &tower_and_shed()).unwrap();
    assert_eq!(
        planned,
        delete::plan(&one_by_one, &tower_and_shed()).unwrap()
    );
    let names: Vec<&str> = planned.operations.iter().map(Operation::name).collect();
    for name in [
        "remove_entities",
        "set_num_children",
        "renumber_for_removed",
    ] {
        assert!(names.contains(&name), "the plan holds a {name}: {names:?}");
    }
    let applied = delete::apply(&one_by_one, &planned).unwrap();
    let deleted = files(&one_by_one);
    assert_ne!(differing(&before, &deleted), Vec::<PathBuf>::new());

    // Through a session: nothing is written before the flush, and the flush writes what one by one wrote.
    let mut session = Session::open(&held).unwrap();
    let done: Vec<Done> = (planned.operations.iter())
        .map(|operation| {
            session
                .apply(operation)
                .unwrap_or_else(|e| panic!("{} in a session: {e}", operation.name()))
        })
        .collect();
    for (in_session, one) in done.iter().zip(&applied) {
        let mut given = in_session.inverse.clone();
        let mut proven = one.inverse.clone();
        given.set_expects(None);
        proven.set_expects(None);
        assert_eq!(
            given, proven,
            "the session gives the inverse the proof gives"
        );
        assert!(
            in_session.inverse.expects().is_some(),
            "the inverse of {} states what it applies to",
            one.inverse.name()
        );
    }
    assert_eq!(differing(&before, &files(&held)), Vec::<PathBuf>::new());
    let written = session.flush().unwrap();
    assert_eq!(
        written
            .iter()
            .map(|w| w.source.as_str())
            .collect::<Vec<_>>(),
        [CITY, LOD, SLOD]
    );
    assert!(written.iter().all(|source| source.proven), "{written:?}");
    assert_eq!(differing(&deleted, &files(&held)), Vec::<PathBuf>::new());
    // The houses name the row, which stands where the tower's far model stood, and no link of the pack is broken.
    assert_eq!(read_entity(&held.join(CITY), 0).unwrap().parent_index, 0);
    assert_eq!(read_entity(&held.join(LOD), 0).unwrap().parent_index, 0);
    assert_eq!(read_entity(&held.join(SLOD), 0).unwrap().num_children, 1);
    assert!(!codes(&held).contains(&"parent-link-broken"));

    // Taken back newest first, each inverse finds the state it states, and the flush puts every byte back.
    for step in done.iter().rev() {
        session
            .apply(&step.inverse)
            .unwrap_or_else(|e| panic!("the inverse {}: {e}", step.inverse.name()));
    }
    let written = session.flush().unwrap();
    assert!(written.iter().all(|source| source.proven), "{written:?}");
    assert_eq!(differing(&before, &files(&held)), Vec::<PathBuf>::new());

    // One by one, the inverses of the list put every byte back too.
    apply_all(&one_by_one, &inverses(&applied), false).unwrap();
    assert_eq!(
        differing(&before, &files(&one_by_one)),
        Vec::<PathBuf>::new()
    );

    // Taken back before any flush, nothing is left to write and nothing was.
    let mut session = Session::open(&never_flushed).unwrap();
    let done: Vec<Done> = (planned.operations.iter())
        .map(|operation| session.apply(operation).unwrap())
        .collect();
    assert!(session.unwritten());
    for step in done.iter().rev() {
        session.apply(&step.inverse).unwrap();
    }
    assert!(!session.unwritten());
    assert_eq!(session.flush().unwrap(), Vec::new());
    assert_eq!(
        differing(&before, &files(&never_flushed)),
        Vec::<PathBuf>::new()
    );
}

#[test]
fn an_inverse_of_a_chain_deletion_is_refused_once_its_source_was_edited_after_it() {
    let (_scratch, [one_by_one, held]) = copies(city);
    let planned = delete::plan(&held, &tower_and_shed()).unwrap();
    let applied = delete::apply(&one_by_one, &planned).unwrap();
    let mut session = Session::open(&held).unwrap();
    let done: Vec<Done> = (planned.operations.iter())
        .map(|operation| session.apply(operation).unwrap())
        .collect();

    // Another edit of `city`, the source the first operation of the plan edited.
    let moved = Operation::PlaceEntity {
        source: CITY.into(),
        expects: None,
        entity: 0,
        pose: pose([19.0, 10.0, 0.0]),
    };
    let of_city = |operation: &&Operation| operation.source() == CITY;
    let in_session = done.iter().map(|step| &step.inverse).find(of_city).unwrap();
    let on_disk = (applied.iter().map(|step| &step.inverse))
        .find(of_city)
        .unwrap();
    session.apply(&moved).unwrap();
    apply(&one_by_one, &moved).unwrap();
    for (refused, way) in [
        (session.apply(in_session).unwrap_err(), "in a session"),
        (apply(&one_by_one, on_disk).unwrap_err(), "one by one"),
    ] {
        assert!(matches!(refused, Error::Stale { .. }), "{way}: {refused:?}");
    }
}

#[test]
fn an_operation_of_a_chain_deletion_a_session_refuses_leaves_its_root_as_it_was() {
    let (_scratch, [held]) = copies(city);
    let mut session = Session::open(&held).unwrap();
    // ⚠ The shed is moved to the front of `city` first, so that the link the refused renumbering works out before it is refused
    // is one that changes: a root half renumbered would hold the shed under the row.
    for operation in delete::plan(&held, &[at(CITY, 0)]).unwrap().operations {
        session.apply(&operation).unwrap();
    }
    let shed = Record::entity(&session.map(CITY).unwrap().entities[2]).unwrap();
    session
        .apply(&Operation::RemoveEntity {
            source: CITY.into(),
            expects: None,
            entity: 2,
            extents: None,
        })
        .unwrap();
    session
        .apply(&Operation::InsertEntity {
            source: CITY.into(),
            expects: None,
            entity: 0,
            record: shed,
            parent_num_children: None,
            children: Vec::new(),
            extents: None,
        })
        .unwrap();
    session.flush().unwrap();
    let before = files(&held);
    let links = |session: &mut Session| -> Vec<i32> {
        (session.map(CITY).unwrap().entities.iter())
            .map(|entity| entity.parent_index)
            .collect()
    };
    // The shed names the far model 1 of `city_lod`, the houses the far model 0, and the fence the far fence of its own file.
    assert_eq!(links(&mut session), [1, 0, 0, -1, 3]);
    let as_read = session.map(LOD).unwrap().clone();
    // The far model of the row is to go while the houses still name it: the renumbering of `city` for it is refused, after the
    // link of the shed before them was worked out.
    let refused = session
        .apply(&Operation::RenumberForRemoved {
            source: CITY.into(),
            expects: None,
            removed: vec![0],
        })
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("take entity 1 out of this map first"),
        "{refused}"
    );
    assert_eq!(links(&mut session), [1, 0, 0, -1, 3]);
    // A link past what an index states, a record that is its own parent and extents that are no number are refused too.
    let tower = Record::entity(&read_entity(&held.join(LOD), 0).unwrap()).unwrap();
    let mut own_parent = read_entity(&held.join(CITY), 4).unwrap();
    own_parent.parent_index = 1;
    let own_parent = Record::entity(&own_parent).unwrap();
    let no_number = Extents {
        entities_extents_min: [f32::NAN, 0.0, 0.0],
        entities_extents_max: [1.0; 3],
        streaming_extents_min: [0.0; 3],
        streaming_extents_max: [1.0; 3],
    };
    let put = |entity, record: &Record| Inserted {
        entity,
        record: record.clone(),
        children: Vec::new(),
    };
    for (operation, said) in [
        (
            Operation::InsertEntities {
                source: CITY.into(),
                expects: None,
                entities: vec![put(0, &tower), put(1, &own_parent)],
                num_children: Vec::new(),
                extents: None,
            },
            "would be its own LOD parent",
        ),
        (
            Operation::RemoveEntities {
                source: LOD.into(),
                expects: None,
                entities: vec![0],
                extents: Some(no_number),
            },
            "not finite",
        ),
        (
            Operation::InsertEntities {
                source: LOD.into(),
                expects: None,
                entities: vec![put(0, &tower)],
                num_children: Vec::new(),
                extents: Some(no_number),
            },
            "not finite",
        ),
    ] {
        let refused = session.apply(&operation).unwrap_err().to_string();
        assert!(refused.contains(said), "{}: {refused}", operation.name());
    }
    assert!(!session.unwritten());
    assert_eq!(session.map(LOD).unwrap(), &as_read);
    for source in [CITY, LOD, SLOD] {
        let held_now = session.map(source).unwrap().clone();
        let cooked = or_maps::facts::cooked(&held.join(source)).unwrap();
        let on_disk: or_io::meta::typed::CMapData = or_io::decl::File::from_bytes(&cooked).unwrap();
        assert_eq!(held_now, on_disk, "{source}");
    }
    assert_eq!(session.flush().unwrap(), Vec::new());
    assert_eq!(differing(&before, &files(&held)), Vec::<PathBuf>::new());
}

/// `harbour` places a crate the type file declares and a bench no type file of the pack declares; `yard` places a crate.
fn harbour_with_a_bench(dir: &Path) -> PathBuf {
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
    let types = types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
    );
    pack_of(
        dir,
        &[
            ("harbour.ymap", harbour),
            ("yard.ymap", yard),
            ("harbour_types.ytyp", types),
        ],
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

#[test]
fn an_entity_of_an_archetype_the_pack_does_not_declare_placed_through_a_session_grows_the_extents_and_says_so(
) {
    let (_scratch, [one_by_one, held]) = copies(harbour_with_a_bench);
    let before = files(&held);
    let far = [5000.0, 5000.0, 0.5];
    let farther = [-7000.0, 300.0, 0.5];
    let list = [
        Operation::PlaceEntity {
            source: HARBOUR.into(),
            expects: None,
            entity: 1,
            pose: pose(far),
        },
        Operation::AddEntity {
            source: HARBOUR.into(),
            expects: None,
            archetype: BENCH.into(),
            pose: pose(farther),
            donor: Some(at(HARBOUR, 1)),
        },
        // A map whose every archetype the pack declares gives an operation nothing to say.
        Operation::PlaceEntity {
            source: YARD.into(),
            expects: None,
            entity: 0,
            pose: pose([-300.0, 20.0, 0.5]),
        },
    ];
    let bench = format!("hash_{:08X}", or_io::string_hash(BENCH.as_bytes()));
    let mut session = Session::open(&held).unwrap();
    let mut done = Vec::new();
    for (operation, says) in list.iter().zip([true, true, false]) {
        let applied = apply(&one_by_one, operation).unwrap();
        let in_session = session.apply(operation).unwrap();
        assert_eq!(
            in_session.notes,
            applied.notes,
            "{}: a session says what one by one says",
            operation.name()
        );
        assert_eq!(
            in_session.notes.len() == 1 && in_session.notes[0].contains(&bench),
            says,
            "{:?}",
            in_session.notes
        );
        done.push(in_session);
    }
    session.flush().unwrap();
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new()
    );
    for (which, (min, max)) in ["entities", "streaming"]
        .iter()
        .zip(extents(&held, HARBOUR))
    {
        for stands in [far, farther] {
            let stands = Vec3::from(stands);
            assert!(
                stands.cmpge(min).all() && stands.cmple(max).all(),
                "the {which} extents {min}..{max} hold the bench at {stands}"
            );
        }
    }
    assert!(!codes(&held).contains(&"extents-short"));

    for step in done.iter().rev() {
        session.apply(&step.inverse).unwrap();
    }
    session.flush().unwrap();
    assert_eq!(differing(&before, &files(&held)), Vec::<PathBuf>::new());
}

#[test]
fn a_type_file_that_does_not_cook_is_said_by_a_session_as_one_by_one_says_it() {
    let (_scratch, [one_by_one, held]) = copies(harbour_with_a_bench);
    for folder in [&one_by_one, &held] {
        std::fs::write(folder.join(TYPES).join("metadata.xml"), "no type file").unwrap();
    }
    let moved = Operation::PlaceEntity {
        source: YARD.into(),
        expects: None,
        entity: 0,
        pose: pose([-300.0, 20.0, 0.5]),
    };
    let applied = apply(&one_by_one, &moved).unwrap();
    let mut session = Session::open(&held).unwrap();
    // Said at each operation, of boxes the session read once.
    for position in [-300.0, -310.0] {
        let done = session
            .apply(&Operation::PlaceEntity {
                source: YARD.into(),
                expects: None,
                entity: 0,
                pose: pose([position, 20.0, 0.5]),
            })
            .unwrap();
        assert!(
            (done.notes.iter()).any(|note| note.contains(TYPES) && note.contains("does not cook")),
            "{:?}",
            done.notes
        );
        if position == -300.0 {
            assert_eq!(done.notes, applied.notes);
        }
    }
    session.flush().unwrap();
    let [(min, _), _] = extents(&held, YARD);
    assert!(min.x <= -310.0, "the entity is held where it stands");
}

/// An empty map made from nothing called `name`, which is a PSO.
fn made(name: &str) -> Vec<u8> {
    create::map_file(
        &create::map(name),
        &ArchetypeIndex::new(),
        BinaryFormat::Pso,
    )
    .unwrap()
}

/// Two maps and a type file that hold nothing, made from nothing, beside a map and a type file that hold one record each.
fn a_pack_with_files_that_hold_nothing(dir: &Path) -> PathBuf {
    let yard = map_file(
        "yard",
        &[Entity {
            scale: 1.5,
            ..Entity::at("prop_crate_a", [-40.0, 5.0, 2.0], 9)
        }],
        ([-50.0; 3], [50.0; 3]),
    );
    let types = types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
    );
    let empty_types = create::types_file(&create::types("quay_types"), BinaryFormat::Pso).unwrap();
    pack_of(
        dir,
        &[
            ("harbour.ymap", made("harbour")),
            ("quay.ymap", made("quay")),
            ("yard.ymap", yard),
            ("harbour_types.ytyp", types),
            ("quay_types.ytyp", empty_types),
        ],
    )
}

#[test]
fn the_first_record_of_a_file_that_holds_none_is_built_through_a_session_as_one_by_one_builds_it() {
    let (_scratch, [one_by_one, held]) = copies(a_pack_with_files_that_hold_nothing);
    let before = files(&held);
    let list = [
        // With no donor, with a donor of another map file, and the first archetype of a type file as a copy of one of another.
        Operation::AddEntity {
            source: HARBOUR.into(),
            expects: None,
            archetype: "prop_crate_a".into(),
            pose: pose([60.0, 70.0, 0.5]),
            donor: None,
        },
        Operation::AddEntity {
            source: "quay.metadata.src".into(),
            expects: None,
            archetype: "prop_crate_a".into(),
            pose: pose([6.0, 7.0, 0.5]),
            donor: Some(at(YARD, 0)),
        },
        Operation::AddArchetype {
            source: "quay_types.metadata.src".into(),
            expects: None,
            name: "prop_quay_crate".into(),
            asset: None,
            donor: Some(at(TYPES, 0)),
        },
        // And a second entity of the map that held none, which is a copy of the first.
        Operation::AddEntity {
            source: HARBOUR.into(),
            expects: None,
            archetype: "prop_crate_a".into(),
            pose: pose([64.0, 70.0, 0.5]),
            donor: None,
        },
    ];
    let mut session = Session::open(&held).unwrap();
    let mut done = Vec::new();
    let mut applied = Vec::new();
    for operation in &list {
        let name = operation.name();
        let one = apply(&one_by_one, operation).unwrap_or_else(|e| panic!("{name}: {e}"));
        let in_session = session
            .apply(operation)
            .unwrap_or_else(|e| panic!("{name} in a session: {e}"));
        assert_eq!(in_session.added, one.added, "{name}");
        assert_eq!(in_session.inverse.name(), one.inverse.name(), "{name}");
        done.push(in_session);
        applied.push(one);
    }
    session.flush().unwrap();
    assert_eq!(
        differing(&files(&one_by_one), &files(&held)),
        Vec::<PathBuf>::new()
    );
    let first = read_entity(&held.join(HARBOUR), 0).unwrap();
    assert_eq!((first.guid, first.parent_index), (1, -1));

    for step in done.iter().rev() {
        session
            .apply(&step.inverse)
            .unwrap_or_else(|e| panic!("the inverse {}: {e}", step.inverse.name()));
    }
    session.flush().unwrap();
    assert_eq!(differing(&before, &files(&held)), Vec::<PathBuf>::new());
    apply_all(&one_by_one, &inverses(&applied), false).unwrap();
    assert_eq!(
        differing(&before, &files(&one_by_one)),
        Vec::<PathBuf>::new()
    );
}

/// `harbour` with four crates, and `yard`, whose parent map is `harbour` and whose one entity names the last entity of `harbour`
/// as its LOD parent.
fn a_yard_under_the_harbour(dir: &Path) -> PathBuf {
    let harbour = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 2),
            Entity::at("prop_crate_a", [18.0, 20.0, 0.5], 3),
            Entity {
                children: 1,
                ..Entity::at("prop_crate_a", [22.0, 20.0, 0.5], 4)
            },
        ],
        ([0.0, 0.0, -0.5], [100.0; 3]),
    );
    let yard = child_map_file(
        "yard",
        "harbour",
        &[under("prop_crate_a", [20.0, 5.0, 2.0], 9, 3)],
        ([0.0, 0.0, -0.5], [100.0; 3]),
    );
    pack_of(dir, &[("harbour.ymap", harbour), ("yard.ymap", yard)])
}

/// What the review left to the chain deletion: an entity taken out of a parent map by the plan of a deletion has the links of the
/// child map renumbered, where the one operation that takes it out alone leaves them for the rule `parent-link-broken` to name.
#[test]
fn a_deletion_planned_in_a_parent_map_renumbers_the_link_the_bare_removal_leaves_past_its_end() {
    let (_scratch, [planned_in, bare]) = copies(a_yard_under_the_harbour);
    assert!(!codes(&bare).contains(&"parent-link-broken"));

    let planned = delete::plan(&planned_in, &[at(HARBOUR, 1)]).unwrap();
    assert_eq!(planned.goes.len(), 1, "{:?}", planned.goes);
    let mut session = Session::open(&planned_in).unwrap();
    for operation in &planned.operations {
        session.apply(operation).unwrap();
    }
    session.flush().unwrap();
    assert_eq!(
        read_entity(&planned_in.join(YARD), 0).unwrap().parent_index,
        2,
        "the link of the child map names the entity it named"
    );
    assert!(!codes(&planned_in).contains(&"parent-link-broken"));

    apply(
        &bare,
        &Operation::RemoveEntity {
            source: HARBOUR.into(),
            expects: None,
            entity: 1,
            extents: None,
        },
    )
    .unwrap();
    assert_eq!(read_entity(&bare.join(YARD), 0).unwrap().parent_index, 3);
    assert!(codes(&bare).contains(&"parent-link-broken"));
}
