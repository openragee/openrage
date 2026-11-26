//! An entity of a map source is put back as the base its source keeps has it: one both hold by a record in its place, one the map
//! file lacks with the chain of far models its deletion took, as one list of operations whose inverses leave the sources as they
//! were edited. What cannot be told from a twin, what the base does not hold and a base that does not cook are refused, and so is a
//! plan made before another edit of its source.
//!
//! After every plan that is applied the pack is read again: the sources compare to their bases through the pack, the rules are run,
//! and the files are held to the bytes they were made with, which is what an untouched source cooks from.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::CMapData;
use or_io::meta::{build_library, canonical, write_meta_binary, BinaryFormat, MetaInstance, Value};
use or_maps::changes::{EntityChange, PackSources, SourceChanges};
use or_maps::delete;
use or_maps::edit::read_entity;
use or_maps::import::{add_files, Incoming};
use or_maps::operation::{
    apply, apply_all, digest_of, inverses, At, Error as Refused, Operation, PoseData, Record,
    Session,
};
use or_maps::put_back::{self, Error, Named, PutBack, Why};
use or_maps::sample::{child_map_file, map_file, types_file, Archetype, Entity};
use or_maps::{check, inspect, Depth, Maps};

const SLOD: &str = "city_slod.metadata.src";
const LOD: &str = "city_lod.metadata.src";
const CITY: &str = "city.metadata.src";
const MAPS: [&str; 3] = [CITY, LOD, SLOD];

/// The entities of `city`, by their index.
const TOWER: usize = 0;
const HOUSE_A: usize = 1;
const HOUSE_B: usize = 2;
const SHED: usize = 3;
const BARN: usize = 4;
const FENCE_FAR: usize = 5;
const FENCE: usize = 6;
const LAMP: usize = 7;
const FENCE_END: usize = 8;
/// The entities of `city_lod`.
const LOD_TOWER: usize = 0;
const LOD_ROW: usize = 1;
const LOD_SHED: usize = 2;
const LOD_BARN: usize = 3;
/// The entities of `city_slod`.
const SLOD_BLOCK: usize = 1;
const SLOD_FAR: usize = 2;

/// The rules a deletion and what takes it back are held to.
const LINK_RULES: [&str; 4] = [
    "parent-link-broken",
    "lod-children-uncounted",
    "extents-short",
    "hierarchy-broken",
];

fn at(source: &str, index: usize) -> At {
    At {
        source: source.to_string(),
        index,
    }
}

fn now(source: &str, index: usize) -> Named {
    Named::Now(at(source, index))
}

fn base(source: &str, index: usize) -> Named {
    Named::Base(at(source, index))
}

/// An entity that names the entity `parent` of the parent map as its LOD parent.
fn under<'a>(archetype: &'a str, position: [f32; 3], guid: u32, parent: i32) -> Entity<'a> {
    Entity {
        parent,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    }
}

fn incoming<'a>(name: &'a str, bytes: &'a [u8]) -> Incoming<'a> {
    Incoming {
        name,
        bytes,
        from_game: true,
        virtual_path: None,
    }
}

/// A pack of three map files that load together and a type file.
///
/// `city_slod` holds a sign that stands for nothing, a block that stands for the far models of the tower and of the row, and a far
/// model of the shed's far model. `city_lod` holds the far models of the tower, of the row of two houses and of the shed, each under
/// `city_slod`, and the far model of the barn, which has no parent. `city` holds the tower, the two houses, the shed and the barn,
/// each under its far model in `city_lod`, two lengths of fence under a far model of their own file, and a lamp between them.
fn city(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let extents = ([0.0; 3], [100.0; 3]);
    let slod = map_file(
        "city_slod",
        &[
            Entity::at("slod_sign", [1.0, 1.0, 0.0], 100),
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
                ..under("lod_tower", [10.0, 10.0, 0.0], 201, SLOD_BLOCK as i32)
            },
            Entity {
                children: 2,
                ..under("lod_row", [20.0, 10.0, 0.0], 202, SLOD_BLOCK as i32)
            },
            Entity {
                children: 1,
                ..under("lod_shed", [90.0, 10.0, 0.0], 203, SLOD_FAR as i32)
            },
            Entity {
                children: 1,
                ..Entity::at("lod_barn", [50.0, 50.0, 0.0], 204)
            },
        ],
        extents,
    );
    let city = child_map_file(
        "city",
        "city_lod",
        &[
            under("tower", [10.0, 12.0, 3.0], 301, LOD_TOWER as i32),
            under("house", [18.0, 10.0, 0.0], 302, LOD_ROW as i32),
            under("house", [22.0, 10.0, 0.0], 303, LOD_ROW as i32),
            under("shed", [90.0, 10.0, 0.0], 304, LOD_SHED as i32),
            under("barn", [50.0, 50.0, 0.0], 305, LOD_BARN as i32),
            Entity {
                children: 2,
                ..Entity::at("fence_far", [70.0, 70.0, 0.0], 306)
            },
            Entity {
                parent: FENCE_FAR as i32,
                ..Entity::at("fence", [70.0, 70.0, 0.0], 307)
            },
            Entity::at("lamp", [5.0, 5.0, 5.0], 308),
            Entity {
                parent: FENCE_FAR as i32,
                ..Entity::at("fence", [74.0, 70.0, 0.0], 311)
            },
        ],
        extents,
    );
    let types = types_file(
        "city_types",
        &[
            Archetype::new("tower", [-1.0; 3], [1.0; 3]),
            Archetype::new("lamp", [-1.0; 3], [1.0; 3]),
        ],
    );
    let added = add_files(
        &folder,
        &[
            incoming("city_slod.ymap", &slod),
            incoming("city_lod.ymap", &lod),
            incoming("city.ymap", &city),
            incoming("city_types.ytyp", &types),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

/// Every file under `folder`, by its path inside it with `/`.
fn files(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let inside = path.strip_prefix(root).unwrap().to_string_lossy();
                into.insert(inside.replace('\\', "/"), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

/// The files of `one` that `other` does not hold with the same bytes.
fn differing(one: &BTreeMap<String, Vec<u8>>, other: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    (one.iter())
        .filter(|(path, bytes)| other.get(*path) != Some(bytes))
        .map(|(path, _)| path.clone())
        .chain(other.keys().filter(|p| !one.contains_key(*p)).cloned())
        .collect()
}

/// The codes of [`LINK_RULES`] that fire on the pack.
fn link_findings(folder: &Path) -> Vec<&'static str> {
    let model = Maps::read(folder).unwrap();
    let facts = inspect(folder, &model, Depth::Open);
    (check(&model, &facts, None).iter())
        .map(|finding| finding.code)
        .filter(|code| LINK_RULES.contains(code))
        .collect()
}

/// What each of `sources` changed of its base, read through the pack so that a link into a parent map is the entity it names.
fn changes(folder: &Path, sources: &[&str]) -> BTreeMap<String, SourceChanges> {
    let read = PackSources::read(folder, sources.iter().copied());
    (sources.iter())
        .map(|source| (source.to_string(), read.changes(source).expect(source)))
        .collect()
}

/// The sources of `sources` that state something their base does not.
fn changed(folder: &Path, sources: &[&str]) -> Vec<String> {
    (changes(folder, sources).into_iter())
        .filter(|(_, changes)| !changes.is_empty())
        .map(|(source, _)| source)
        .collect()
}

fn map_of(folder: &Path, source: &str) -> CMapData {
    CMapData::from_bytes(&or_maps::facts::cooked(&folder.join(source)).unwrap()).unwrap()
}

/// The corners a map states: the lowest and the highest of its entities, then of what it streams by.
fn extents(map: &CMapData) -> [[f32; 3]; 4] {
    [
        map.entities_extents_min,
        map.entities_extents_max,
        map.streaming_extents_min,
        map.streaming_extents_max,
    ]
    .map(|corner| [corner.x, corner.y, corner.z])
}

fn pose(position: [f32; 3]) -> PoseData {
    PoseData {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
    }
}

/// Stand the entity `entity` of `source` at `position`, and state `flags` as its flags.
fn move_and_flag(folder: &Path, source: &str, entity: usize, position: [f32; 3], flags: u32) {
    let moved = Operation::PlaceEntity {
        source: source.into(),
        expects: None,
        entity,
        pose: pose(position),
    };
    apply(folder, &moved).unwrap();
    let mut flagged = read_entity(&folder.join(source), entity).unwrap();
    flagged.flags = flags;
    let stated = Operation::ReplaceEntity {
        source: source.into(),
        expects: None,
        entity,
        record: Record::entity(&flagged).unwrap(),
        extents: None,
    };
    apply(folder, &stated).unwrap();
}

/// Plan what puts `named` back, apply it, and take it back and apply it again; the plan, with the pack left as the plan leaves it.
///
/// Held for every case: a plan writes nothing, what it says it writes is what is written, its inverses leave every file as it was
/// edited, byte for byte, and applied again it writes the same bytes.
fn put_back(folder: &Path, named: &Named) -> PutBack {
    let edited = files(folder);
    let planned = put_back::plan(folder, named).expect("plans");
    assert_eq!(differing(&edited, &files(folder)), Vec::<String>::new());

    let applied = put_back::apply(folder, &planned).expect("applies");
    let after = files(folder);
    let mut written: Vec<String> = (differing(&edited, &after).iter())
        .map(|path| path.trim_end_matches("/metadata.xml").to_string())
        .collect();
    written.sort();
    assert_eq!(
        written, planned.sources,
        "the plan names the sources written"
    );
    assert!(
        applied.iter().all(|done| done.edited.changed()),
        "the list holds no operation that changes nothing"
    );

    apply_all(folder, &inverses(&applied), false).expect("the inverses apply");
    assert_eq!(
        differing(&edited, &files(folder)),
        Vec::<String>::new(),
        "the inverses leave the sources as they were edited"
    );
    put_back::apply(folder, &planned).expect("applies again");
    assert_eq!(differing(&after, &files(folder)), Vec::<String>::new());
    planned
}

fn names(planned: &PutBack) -> Vec<&'static str> {
    planned.operations.iter().map(Operation::name).collect()
}

/// The entities that come back, each by its source, its index in the base, its index once in, and why.
fn returns(planned: &PutBack) -> Vec<(&str, usize, usize, Why)> {
    (planned.returns.iter())
        .map(|back| {
            (
                back.source.as_str(),
                back.base.index,
                back.index,
                back.why.clone(),
            )
        })
        .collect()
}

fn recounted(planned: &PutBack) -> Vec<(&str, usize, u32, u32)> {
    (planned.recounted.iter())
        .map(|count| (count.source.as_str(), count.entity, count.from, count.to))
        .collect()
}

/// Delete the entities `chosen` with their chains, as `openrage-cli maps delete --apply` does.
fn delete(folder: &Path, chosen: &[At]) -> delete::Deletion {
    let planned = delete::plan(folder, chosen).expect("the deletion plans");
    delete::apply(folder, &planned).expect("the deletion applies");
    planned
}

#[test]
fn an_entity_moved_and_flagged_is_put_back_and_its_source_cooks_to_what_it_cooked_untouched() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    let untouched = or_maps::facts::cooked(&folder.join(CITY)).unwrap();
    let stated = map_of(&folder, CITY);

    // The lamp is moved past the box the map states, which grows to hold it, and its flags are stated anew.
    move_and_flag(&folder, CITY, LAMP, [140.0, 5.0, 5.0], 32 | 1);
    let edited = map_of(&folder, CITY);
    assert_ne!(
        extents(&edited),
        extents(&stated),
        "the move grew the extents"
    );
    let SourceChanges::Map(found) = &changes(&folder, &[CITY])[CITY] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(found.changed.len(), 1);

    let planned = put_back(&folder, &now(CITY, LAMP));
    assert_eq!(planned.returns, Vec::new());
    assert_eq!(names(&planned), ["replace_entity", "set_extents"]);
    let restored = planned.restored.as_ref().expect("the lamp is put back");
    assert_eq!((restored.base.index, restored.now.index), (LAMP, LAMP));
    assert_eq!(
        restored.changes,
        [
            EntityChange::Moved {
                from: [5.0, 5.0, 5.0],
                to: [140.0, 5.0, 5.0],
            },
            EntityChange::Flags { from: 32, to: 33 },
        ]
    );

    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(
        or_maps::facts::cooked(&folder.join(CITY)).unwrap(),
        untouched,
        "the source cooks to what it cooked to untouched"
    );
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());

    // Named by its index in the base, it is the same plan.
    move_and_flag(&folder, CITY, LAMP, [140.0, 5.0, 5.0], 32 | 1);
    let by_base = put_back::plan(&folder, &base(CITY, LAMP)).unwrap();
    assert_eq!(by_base, planned);
}

#[test]
fn the_extents_put_back_are_those_of_the_base_grown_to_hold_what_still_stands_otherwise() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let stated = extents(&map_of(&folder, CITY));

    // The lamp goes far to the east and the tower far to the north; the tower has a box, a metre each way.
    move_and_flag(&folder, CITY, LAMP, [140.0, 5.0, 5.0], 32);
    move_and_flag(&folder, CITY, TOWER, [10.0, 180.0, 3.0], 32 | 8);
    let grown = extents(&map_of(&folder, CITY));
    assert_eq!(grown[1], [141.0, 181.0, 100.0]);

    put_back(&folder, &now(CITY, LAMP));
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    let held = extents(&map_of(&folder, CITY));
    assert_eq!(
        held[1],
        [100.0, 181.0, 100.0],
        "the east is the base's again, and the north still holds the tower"
    );
    assert_eq!(held[0], stated[0]);

    put_back(&folder, &now(CITY, TOWER));
    assert_eq!(extents(&map_of(&folder, CITY)), stated);
    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
}

#[test]
fn a_building_deleted_with_its_far_models_comes_back_whole_as_one_list_and_the_rules_are_silent() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());

    // The tower goes with its far model, and the block that stands for the row too counts one child fewer.
    let deleted = delete(&folder, &[at(CITY, TOWER)]);
    assert_eq!(deleted.goes.len(), 2);
    assert_eq!(changed(&folder, &MAPS), [CITY, LOD, SLOD]);

    let planned = put_back(&folder, &base(CITY, TOWER));
    assert_eq!(planned.restored, None);
    assert_eq!(
        returns(&planned),
        [
            (CITY, TOWER, TOWER, Why::Chosen),
            (LOD, LOD_TOWER, LOD_TOWER, Why::FarModel),
        ]
    );
    assert_eq!(recounted(&planned), [(SLOD, SLOD_BLOCK, 1, 2)]);
    assert_eq!(planned.left, Vec::new());
    assert_eq!(planned.notes, Vec::<String>::new());
    assert_eq!(planned.sources, [CITY, LOD, SLOD]);
    assert_eq!(
        names(&planned),
        [
            // The links of `city` into `city_lod` move on for the far model that comes back before what they name.
            "renumber_for_inserted",
            "insert_entities",
            "insert_entities",
            "set_num_children",
        ]
    );

    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());

    // The shed's chain is one entity in each of the three files.
    let deleted = delete(&folder, &[at(CITY, SHED)]);
    assert_eq!(deleted.goes.len(), 3);
    let planned = put_back(&folder, &base(CITY, SHED));
    assert_eq!(
        returns(&planned),
        [
            (CITY, SHED, SHED, Why::Chosen),
            (LOD, LOD_SHED, LOD_SHED, Why::FarModel),
            (SLOD, SLOD_FAR, SLOD_FAR, Why::FarModel),
        ]
    );
    assert_eq!(recounted(&planned), []);
    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());

    // With the tower still out, the barn and its far model come back at other indexes than the base gives them, and the link
    // of the one names the other where it stands.
    delete(&folder, &[at(CITY, TOWER)]);
    delete(&folder, &[at(CITY, BARN - 1)]);
    let planned = put_back(&folder, &base(CITY, BARN));
    assert_eq!(
        returns(&planned),
        [
            (CITY, BARN, BARN - 1, Why::Chosen),
            (LOD, LOD_BARN, LOD_BARN - 1, Why::FarModel),
        ]
    );
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    let barn = &map_of(&folder, CITY).entities[BARN - 1];
    assert_eq!(barn.parent_index, LOD_BARN as i32 - 1);
    let left = changes(&folder, &MAPS);
    let SourceChanges::Map(city) = &left[CITY] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(
        (city.deleted.len(), city.count()),
        (1, 1),
        "the tower is all that is still out of `city`"
    );
    put_back(&folder, &base(CITY, TOWER));
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn one_of_two_buildings_comes_back_with_the_far_model_they_share_which_counts_what_stands_under_it()
{
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    let deleted = delete(&folder, &[at(CITY, HOUSE_A), at(CITY, HOUSE_B)]);
    assert_eq!(deleted.goes.len(), 3, "the two houses and their row");

    let planned = put_back(&folder, &base(CITY, HOUSE_B));
    assert_eq!(
        returns(&planned),
        [
            // The first house is still out, so the second stands where the first stood.
            (CITY, HOUSE_B, HOUSE_A, Why::Chosen),
            (LOD, LOD_ROW, LOD_ROW, Why::FarModel),
        ]
    );
    assert_eq!(recounted(&planned), [(SLOD, SLOD_BLOCK, 1, 2)]);
    let row = &map_of(&folder, LOD).entities[LOD_ROW];
    assert_eq!(
        row.num_children, 1,
        "the row counts the house that stands under it, where the base counts two"
    );
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    // What is left to say of the pack is the house that is still out, and the row that counts one child for it.
    let left = changes(&folder, &MAPS);
    let (SourceChanges::Map(city), SourceChanges::Map(lod)) = (&left[CITY], &left[LOD]) else {
        panic!("a map source compares as a map");
    };
    assert_eq!(
        (city.deleted.len(), city.changed.len(), city.count()),
        (1, 0, 1)
    );
    // ⚠ The comparison counts the children of an entity in its own file, so a count that follows a child of the map under it reads as a change.
    assert_eq!(lod.count(), 1);
    assert_eq!(
        lod.changed[0].changes,
        [EntityChange::Members {
            names: vec!["numChildren"]
        }]
    );
    assert!(left[SLOD].is_empty());

    // The other house comes back under the row that is there, which counts it.
    let planned = put_back(&folder, &base(CITY, HOUSE_A));
    assert_eq!(returns(&planned), [(CITY, HOUSE_A, HOUSE_A, Why::Chosen)]);
    assert_eq!(recounted(&planned), [(LOD, LOD_ROW, 1, 2)]);
    assert_eq!(names(&planned), ["insert_entities", "set_num_children"]);
    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_far_model_put_back_brings_what_the_base_places_under_it() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    // The row is chosen: the two houses under it go with it.
    let deleted = delete(&folder, &[at(LOD, LOD_ROW)]);
    assert_eq!(deleted.goes.len(), 3);

    let planned = put_back(&folder, &base(LOD, LOD_ROW));
    let under = Why::Under {
        source: LOD.to_string(),
        index: LOD_ROW,
    };
    assert_eq!(
        returns(&planned),
        [
            (LOD, LOD_ROW, LOD_ROW, Why::Chosen),
            (CITY, HOUSE_A, HOUSE_A, under.clone()),
            (CITY, HOUSE_B, HOUSE_B, under),
        ]
    );
    assert_eq!(recounted(&planned), [(SLOD, SLOD_BLOCK, 1, 2)]);
    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_far_model_taken_out_alone_comes_back_over_the_entities_it_left_with_no_parent() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    // The removal alone leaves the two lengths of fence with no parent, which a deletion planned would have taken with it.
    let alone = Operation::RemoveEntity {
        source: CITY.into(),
        expects: None,
        entity: FENCE_FAR,
        extents: None,
    };
    apply(&folder, &alone).unwrap();
    let left = map_of(&folder, CITY);
    assert_eq!(
        (
            left.entities[FENCE - 1].parent_index,
            left.entities[FENCE_END - 1].parent_index
        ),
        (-1, -1)
    );

    let planned = put_back(&folder, &base(CITY, FENCE_FAR));
    assert_eq!(
        returns(&planned),
        [(CITY, FENCE_FAR, FENCE_FAR, Why::Chosen)]
    );
    assert_eq!(names(&planned), ["insert_entities"]);
    let back = map_of(&folder, CITY);
    assert_eq!(back.entities[FENCE_FAR].num_children, 2);
    assert_eq!(
        (
            back.entities[FENCE].parent_index,
            back.entities[FENCE_END].parent_index
        ),
        (FENCE_FAR as i32, FENCE_FAR as i32)
    );
    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn an_entity_comes_back_under_a_far_model_of_its_own_file_which_counts_it_again() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    // One length of fence goes, and the far model of the two counts one.
    let deleted = delete(&folder, &[at(CITY, FENCE)]);
    assert_eq!(deleted.goes.len(), 1);
    assert_eq!(map_of(&folder, CITY).entities[FENCE_FAR].num_children, 1);

    let planned = put_back(&folder, &base(CITY, FENCE));
    assert_eq!(returns(&planned), [(CITY, FENCE, FENCE, Why::Chosen)]);
    assert_eq!(recounted(&planned), [(CITY, FENCE_FAR, 1, 2)]);
    // The count goes in with the entity, as the removal that takes the entity out again takes it off.
    assert_eq!(names(&planned), ["insert_entities"]);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn an_entity_put_back_under_a_far_model_that_went_brings_the_far_model_with_it() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);

    // A length of fence is put under no parent by hand, with the count of the far model it left as it was.
    let mut loose = read_entity(&folder.join(CITY), FENCE).unwrap();
    loose.parent_index = -1;
    let stated = Operation::ReplaceEntity {
        source: CITY.into(),
        expects: None,
        entity: FENCE,
        record: Record::entity(&loose).unwrap(),
        extents: None,
    };
    apply(&folder, &stated).unwrap();
    let planned = put_back(&folder, &now(CITY, FENCE));
    assert_eq!(names(&planned), ["replace_entity", "set_num_children"]);
    assert_eq!(
        recounted(&planned),
        [(CITY, FENCE_FAR, 2, 3)],
        "the far model counts one more for the entity put back under it, whatever it counted"
    );
    // The count that was left wrong is put back with the far model itself.
    let planned = put_back(&folder, &now(CITY, FENCE_FAR));
    assert_eq!(names(&planned), ["replace_entity"]);
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());

    // The far model of the fence goes alone, and a length of fence it left with no parent is put back: the far model comes
    // back for it, over both lengths.
    let alone = Operation::RemoveEntity {
        source: CITY.into(),
        expects: None,
        entity: FENCE_FAR,
        extents: None,
    };
    apply(&folder, &alone).unwrap();
    let planned = put_back(&folder, &now(CITY, FENCE - 1));
    assert_eq!(
        planned.restored.as_ref().map(|put| put.base.index),
        Some(FENCE)
    );
    assert_eq!(
        returns(&planned),
        [(CITY, FENCE_FAR, FENCE_FAR, Why::FarModel)]
    );
    assert_eq!(names(&planned), ["insert_entities"]);
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_link_a_removal_alone_left_on_another_entity_is_put_back_with_the_entity_that_states_it() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    // The far model of the shed goes alone from `city_lod`: the shed's link then names the barn's far model, and the barn's
    // names nothing. The far model above it is made to count what is left under it.
    let alone = Operation::RemoveEntity {
        source: LOD.into(),
        expects: None,
        entity: LOD_SHED,
        extents: None,
    };
    apply(&folder, &alone).unwrap();
    let counted = Operation::SetNumChildren {
        source: SLOD.into(),
        expects: None,
        entity: SLOD_FAR,
        num_children: 0,
    };
    apply(&folder, &counted).unwrap();
    assert_eq!(link_findings(&folder), ["parent-link-broken"]);

    let planned = put_back(&folder, &now(CITY, SHED));
    assert!(planned.restored.is_some());
    assert_eq!(
        returns(&planned),
        [(LOD, LOD_SHED, LOD_SHED, Why::FarModel)]
    );
    assert_eq!(
        recounted(&planned),
        [
            // The barn's far model was named by the shed's link, and is named by none now.
            (LOD, LOD_BARN, 1, 0),
            (SLOD, SLOD_FAR, 0, 1),
        ]
    );
    // ⚠ The barn's link was on no entity before and is on none now: it is the barn's to put back.
    assert_eq!(link_findings(&folder), ["parent-link-broken"]);
    let left = changes(&folder, &MAPS);
    let SourceChanges::Map(city) = &left[CITY] else {
        panic!("a map source compares as a map");
    };
    let listed: Vec<usize> = city.changed.iter().map(|kept| kept.now.index).collect();
    assert_eq!(listed, [BARN]);

    let planned = put_back(&folder, &now(CITY, BARN));
    assert_eq!(recounted(&planned), [(LOD, LOD_BARN, 0, 1)]);
    assert_eq!(changed(&folder, &MAPS), Vec::<String>::new());
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn an_entity_whose_far_model_is_in_a_map_the_pack_does_not_hold_is_put_back_with_the_link_the_base_states(
) {
    // The pack holds `city` alone, as a first edit of one of its objects leaves a project: `city_lod` is the game's.
    let scratch = tempfile::tempdir().unwrap();
    let whole = city(scratch.path());
    let cooked = or_maps::facts::cooked(&whole.join(CITY)).unwrap();
    let folder = scratch.path().join("alone");
    let added = add_files(&folder, &[incoming("city.ymap", &cooked)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let made = files(&folder);

    move_and_flag(&folder, CITY, SHED, [91.0, 10.0, 0.0], 32 | 8);
    let planned = put_back(&folder, &now(CITY, SHED));
    assert_eq!(planned.notes, Vec::<String>::new());
    assert_eq!(planned.returns, Vec::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());

    // Taken out, it comes back alone, naming the index of the game's map that its base names.
    let gone = Operation::RemoveEntity {
        source: CITY.into(),
        expects: None,
        entity: SHED,
        extents: None,
    };
    apply(&folder, &gone).unwrap();
    let planned = put_back(&folder, &base(CITY, SHED));
    assert_eq!(returns(&planned), [(CITY, SHED, SHED, Why::Chosen)]);
    assert_eq!(planned.notes, Vec::<String>::new());
    assert_eq!(
        map_of(&folder, CITY).entities[SHED].parent_index,
        LOD_SHED as i32
    );
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

/// A pack of one map of two crates, two trees that state one guid, and a bench.
fn harbour(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let map = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 2),
            Entity::at("prop_tree", [30.0, 30.0, 0.0], 7),
            Entity::at("prop_tree", [34.0, 30.0, 0.0], 7),
            Entity::at("prop_bench", [18.0, 20.0, 0.5], 8),
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(&folder, &[incoming("harbour.ymap", &map)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

const HARBOUR: &str = "harbour.metadata.src";

#[test]
fn an_entity_that_cannot_be_told_from_a_twin_is_refused_and_nothing_is_written() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = harbour(scratch.path());
    // One of the two trees moves: nothing says which of the two it was.
    let moved = Operation::PlaceEntity {
        source: HARBOUR.into(),
        expects: None,
        entity: 3,
        pose: pose([36.0, 31.0, 0.0]),
    };
    apply(&folder, &moved).unwrap();
    let edited = files(&folder);
    let SourceChanges::Map(found) = &changes(&folder, &[HARBOUR])[HARBOUR] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(found.ambiguous.len(), 1);
    assert_eq!(
        found.count(),
        1 + found.root.len(),
        "a group nothing tells apart counts once"
    );

    for named in [now(HARBOUR, 3), base(HARBOUR, 3)] {
        let refused = put_back::plan(&folder, &named).unwrap_err();
        assert!(
            matches!(
                &refused,
                Error::Ambiguous { at, index: 3, base, now, .. }
                    if at == HARBOUR && base == &[3] && now == &[3]
            ),
            "{refused:?}"
        );
        let said = refused.to_string();
        assert!(said.contains("cannot be told from a twin"), "{said}");
        assert!(said.contains("none is put back by a guess"), "{said}");
    }
    // The twin that stands where it stood is itself, and states what the base states.
    let refused = put_back::plan(&folder, &now(HARBOUR, 2)).unwrap_err();
    assert!(matches!(refused, Error::Unchanged { .. }), "{refused:?}");
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());
}

#[test]
fn an_entity_the_base_does_not_hold_is_refused_as_what_a_deletion_takes_out() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = harbour(scratch.path());
    let added = Operation::AddEntity {
        source: HARBOUR.into(),
        expects: None,
        archetype: "prop_crate_b".into(),
        pose: pose([50.0, 50.0, 0.5]),
        donor: None,
    };
    let index = apply(&folder, &added).unwrap().added.unwrap();
    let edited = files(&folder);

    let refused = put_back::plan(&folder, &now(HARBOUR, index)).unwrap_err();
    assert!(
        matches!(&refused, Error::Added { at, index: at_index } if at == HARBOUR && *at_index == index),
        "{refused:?}"
    );
    let said = refused.to_string();
    assert!(said.contains("the source added it"), "{said}");
    assert!(said.contains("`openrage-cli maps delete`"), "{said}");

    // An entity that is not there is refused by the number the file holds, in the base and now.
    let past = put_back::plan(&folder, &now(HARBOUR, index + 1)).unwrap_err();
    assert_eq!(
        past.to_string(),
        format!(
            "`{HARBOUR}` holds {} entities, so there is no entity {}",
            index + 1,
            index + 1
        )
    );
    let past = put_back::plan(&folder, &base(HARBOUR, index)).unwrap_err();
    assert_eq!(
        past.to_string(),
        format!("the base of `{HARBOUR}` holds {index} entities, so there is no entity {index}")
    );
    let unknown = put_back::plan(&folder, &now("quay.metadata.src", 0)).unwrap_err();
    assert!(matches!(unknown, Error::NoSuchSource(_)), "{unknown:?}");
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_base_that_does_not_cook_is_refused_by_the_name_of_its_source() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = harbour(scratch.path());
    move_and_flag(&folder, HARBOUR, 0, [11.0, 20.0, 0.5], 32);
    put_back::plan(&folder, &now(HARBOUR, 0)).expect("the source is put back while its base cooks");

    // The base is cut short, and its manifest is made to state the digest of what is left, so that only cooking it shows it.
    let dir = folder.join(HARBOUR);
    let whole = std::fs::read(dir.join("base.ymap")).unwrap();
    let cut = &whole[..whole.len() / 2];
    std::fs::write(dir.join("base.ymap"), cut).unwrap();
    let manifest = std::fs::read_to_string(dir.join("asset.json")).unwrap();
    let (was, is) = (
        or_source::asset::sha256_hex(&whole),
        or_source::asset::sha256_hex(cut),
    );
    assert!(manifest.contains(&was));
    std::fs::write(dir.join("asset.json"), manifest.replace(&was, &is)).unwrap();
    let edited = files(&folder);

    let refused = put_back::plan(&folder, &now(HARBOUR, 0)).unwrap_err();
    let said = refused.to_string();
    assert!(
        matches!(refused, Error::Compare { .. } | Error::Unread { .. }),
        "{refused:?}"
    );
    assert!(said.contains("base"), "{said}");
    assert!(said.contains(HARBOUR), "{said}");
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_plan_made_before_another_edit_of_its_source_is_refused_whole() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    delete(&folder, &[at(CITY, TOWER)]);
    let planned = put_back::plan(&folder, &base(CITY, TOWER)).unwrap();
    // Each source states the XML the plan was made of, once.
    for source in MAPS {
        let stating: Vec<&str> = (planned.operations.iter())
            .filter(|operation| operation.source() == source)
            .filter_map(Operation::expects)
            .collect();
        let xml = std::fs::read(folder.join(source).join("metadata.xml")).unwrap();
        assert_eq!(stating, [digest_of(&xml)], "{source}");
    }

    // `city_slod` is edited after the plan was made: it is the last source the list reaches.
    let other = Operation::PlaceEntity {
        source: SLOD.into(),
        expects: None,
        entity: 0,
        pose: pose([2.0, 1.0, 0.0]),
    };
    apply(&folder, &other).unwrap();
    let edited = files(&folder);

    let refused = put_back::apply(&folder, &planned).unwrap_err();
    let Refused::InList { refused: why, .. } = &refused else {
        panic!("{refused:?}");
    };
    assert!(
        matches!(&**why, Refused::Stale { at, .. } if at == SLOD),
        "{refused}"
    );
    assert_eq!(
        differing(&edited, &files(&folder)),
        Vec::<String>::new(),
        "no source of the list was written"
    );

    // A session refuses it the same way, and holds nothing of it.
    let mut session = Session::open(&folder).unwrap();
    let refused = put_back::apply_in(&mut session, &planned).unwrap_err();
    assert!(
        matches!(&refused, Refused::InList { refused: why, .. } if matches!(&**why, Refused::Stale { .. })),
        "{refused}"
    );
    assert_eq!(
        session.flush().unwrap(),
        Vec::new(),
        "it has nothing to write"
    );
    drop(session);
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());

    // Planned again, it applies, and what is left is the other edit.
    put_back(&folder, &base(CITY, TOWER));
    assert_eq!(changed(&folder, &MAPS), [SLOD]);
}

#[test]
fn a_session_does_a_plan_whole_and_writes_what_the_list_writes() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    delete(&folder, &[at(CITY, SHED)]);
    let edited = files(&folder);
    let planned = put_back::plan(&folder, &base(CITY, SHED)).unwrap();

    let mut session = Session::open(&folder).unwrap();
    let done = put_back::apply_in(&mut session, &planned).unwrap();
    assert_eq!(done.len(), planned.operations.len());
    assert_eq!(
        differing(&edited, &files(&folder)),
        Vec::<String>::new(),
        "a session writes nothing until it is flushed"
    );
    session.flush().unwrap();
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());

    // Undo is the inverses the session gave, the last first.
    for step in done.iter().rev() {
        session.apply(&step.inverse).unwrap();
    }
    session.flush().unwrap();
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());

    // ⚠ A plan is of the sources as the disk holds them: a session that holds an edit of one of them it has not written
    // refuses the plan, and keeps the edit.
    let other = Operation::PlaceEntity {
        source: LOD.into(),
        expects: None,
        entity: 0,
        pose: pose([11.0, 10.0, 0.0]),
    };
    session.apply(&other).unwrap();
    let refused = put_back::apply_in(&mut session, &planned).unwrap_err();
    let Refused::InList { refused: why, .. } = &refused else {
        panic!("{refused:?}");
    };
    assert!(
        matches!(&**why, Refused::Stale { at, .. } if at == LOD),
        "{refused}"
    );
    let written = session.flush().unwrap();
    assert_eq!(written.len(), 1, "the edit the session held, and no other");
    drop(session);
    assert_eq!(
        differing(&edited, &files(&folder)),
        [format!("{LOD}/metadata.xml")]
    );
}

#[test]
fn an_entity_added_after_the_one_with_the_largest_guid_went_is_put_back_as_that_entity() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = harbour(scratch.path());
    let made = files(&folder);
    let (crate_a, bench) = (0, 4);
    assert_eq!(map_of(&folder, HARBOUR).entities[bench].guid, 8);
    let gone = |entity: usize| Operation::RemoveEntity {
        source: HARBOUR.into(),
        expects: None,
        entity,
        extents: None,
    };
    let added = Operation::AddEntity {
        source: HARBOUR.into(),
        expects: None,
        archetype: "prop_crate_b".into(),
        pose: pose([60.0, 60.0, 0.5]),
        donor: None,
    };
    // The bench states the largest guid, one more than the next. It goes, and the crate added next takes its guid.
    apply(&folder, &gone(bench)).unwrap();
    let index = apply(&folder, &added).unwrap().added.unwrap();
    assert_eq!(map_of(&folder, HARBOUR).entities[index].guid, 8);
    assert_eq!(index, bench, "it is added last, where the bench stood last");
    let SourceChanges::Map(read) = &changes(&folder, &[HARBOUR])[HARBOUR] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(
        (read.added.len(), read.deleted.len(), read.changed.len()),
        (0, 0, 1),
        "the comparison reads one entity changed"
    );

    // So that is what is put back: the bench, in the place of the crate, which is gone as an edit of the bench is.
    let planned = put_back(&folder, &now(HARBOUR, index));
    assert_eq!(planned.returns, Vec::new());
    assert_eq!(planned.operations[0].name(), "replace_entity");
    assert_eq!(changed(&folder, &[HARBOUR]), Vec::<String>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());

    // An entity added while the largest guid stands states one more, and is an addition beside the deletion.
    apply(&folder, &gone(crate_a)).unwrap();
    let index = apply(&folder, &added).unwrap().added.unwrap();
    assert_eq!(map_of(&folder, HARBOUR).entities[index].guid, 9);
    let refused = put_back::plan(&folder, &now(HARBOUR, index)).unwrap_err();
    assert!(matches!(refused, Error::Added { .. }), "{refused:?}");
    let planned = put_back(&folder, &base(HARBOUR, crate_a));
    assert_eq!(
        returns(&planned),
        [(HARBOUR, crate_a, crate_a, Why::Chosen)]
    );
}

const QUAY: &str = "quay.metadata.src";

/// A pack of one map whose entity with the largest guid stands first, and a lamp after it.
fn quay(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let map = map_file(
        "quay",
        &[
            Entity::at("prop_bollard", [10.0, 20.0, 0.5], 3),
            Entity::at("prop_lamp", [14.0, 20.0, 0.5], 2),
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(&folder, &[incoming("quay.ymap", &map)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

#[test]
fn an_entity_put_back_by_its_guid_stands_at_the_index_of_the_one_that_took_the_guid() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = quay(scratch.path());
    let made = files(&folder);
    let stated = map_of(&folder, QUAY);

    let gone = Operation::RemoveEntity {
        source: QUAY.into(),
        expects: None,
        entity: 0,
        extents: None,
    };
    apply(&folder, &gone).unwrap();
    let added = Operation::AddEntity {
        source: QUAY.into(),
        expects: None,
        archetype: "prop_crate_b".into(),
        pose: pose([60.0, 60.0, 0.5]),
        donor: None,
    };
    let index = apply(&folder, &added).unwrap().added.unwrap();
    assert_eq!((index, map_of(&folder, QUAY).entities[index].guid), (1, 3));

    put_back(&folder, &now(QUAY, index));
    let back = map_of(&folder, QUAY);
    assert_eq!(changed(&folder, &[QUAY]), Vec::<String>::new());
    assert!(
        back.entities[1] == stated.entities[0] && back.entities[0] == stated.entities[1],
        "every entity states what the base states, in the order the edits left"
    );
    // ⚠ So the source is not the bytes it was made with: only a discard of the map reaches those.
    assert_eq!(
        differing(&made, &files(&folder)),
        [format!("{QUAY}/metadata.xml")]
    );
}

#[test]
fn a_map_file_emptied_gets_its_entities_back_one_at_a_time() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = quay(scratch.path());
    let made = files(&folder);
    let all = Operation::RemoveEntities {
        source: QUAY.into(),
        expects: None,
        entities: vec![0, 1],
        extents: None,
    };
    apply(&folder, &all).unwrap();
    assert_eq!(map_of(&folder, QUAY).entities.len(), 0);

    // ⚠ A map file with no entity states no class of one, so where no schema snapshot is staged it takes no record: the plan is
    // then the XML of the file its base saves with the entity in, and is an insertion where a snapshot is.
    let planned = put_back(&folder, &base(QUAY, 1));
    assert_eq!(returns(&planned), [(QUAY, 1, 0, Why::Chosen)]);
    assert!(
        matches!(
            names(&planned).as_slice(),
            ["insert_entities"] | ["rewrite_source"]
        ),
        "{:?}",
        names(&planned)
    );
    let held = map_of(&folder, QUAY);
    assert_eq!(held.entities.len(), 1);
    assert_eq!(held.entities[0].guid, 2);

    let planned = put_back(&folder, &base(QUAY, 0));
    assert_eq!(returns(&planned), [(QUAY, 0, 0, Why::Chosen)]);
    assert_eq!(changed(&folder, &[QUAY]), Vec::<String>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn what_a_discard_of_a_map_would_lose_is_counted_from_the_comparison() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let count = |folder: &Path| -> usize {
        changes(folder, &MAPS)
            .values()
            .map(SourceChanges::count)
            .sum()
    };
    assert_eq!(count(&folder), 0);

    // One entity changed in two ways is one change, and each corner of the extents that grew with it is one more.
    move_and_flag(&folder, CITY, LAMP, [140.0, 5.0, 5.0], 32 | 1);
    let SourceChanges::Map(found) = &changes(&folder, &[CITY])[CITY] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(found.changed.len(), 1);
    assert!(!found.root.is_empty(), "the extents grew");
    let grown = found.root.len();
    assert_eq!(found.count(), 1 + grown);

    // A building deleted is an entity in each file its chain crosses, and a far model that counts a child fewer.
    delete(&folder, &[at(CITY, TOWER)]);
    let found = changes(&folder, &MAPS);
    assert_eq!(
        [found[CITY].count(), found[LOD].count(), found[SLOD].count()],
        [2 + grown, 1, 1]
    );
    put_back(&folder, &base(CITY, TOWER));
    assert_eq!(count(&folder), 1 + grown);
    put_back(&folder, &now(CITY, LAMP));
    assert_eq!(count(&folder), 0);
}

/// The index in `source` of the entity that states `guid`, when it holds one.
fn standing(folder: &Path, source: &str, guid: u32) -> Option<usize> {
    (map_of(folder, source).entities.iter()).position(|entity| entity.guid == guid)
}

/// Delete each of the `len` entities of `source` in turn with its chain, and put it back: what went comes back, the rules are
/// silent, and the pack is the bytes it was made with.
fn each_deleted_and_put_back(source: &str, len: usize) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    let made = files(&folder);
    assert_eq!(map_of(&folder, source).entities.len(), len);
    for index in 0..len {
        let deleted = delete(&folder, &[at(source, index)]);
        assert!(!deleted.goes.is_empty(), "{source} {index}");
        let planned = put_back(&folder, &base(source, index));
        assert_eq!(
            planned.returns.len(),
            deleted.goes.len(),
            "{source} {index}: what went comes back"
        );
        assert_eq!(
            link_findings(&folder),
            Vec::<&str>::new(),
            "{source} {index}"
        );
        assert_eq!(
            changed(&folder, &MAPS),
            Vec::<String>::new(),
            "{source} {index}"
        );
        assert_eq!(
            differing(&made, &files(&folder)),
            Vec::<String>::new(),
            "{source} {index}"
        );
    }
}

// ⚠ A test for each map file, so that none of the three runs long in the default tier.
#[test]
fn each_entity_of_a_map_deleted_with_its_chain_is_put_back_to_the_bytes_the_pack_was_made_with() {
    each_deleted_and_put_back(CITY, 9);
}

#[test]
fn each_far_model_deleted_with_what_is_under_it_is_put_back_to_the_bytes_the_pack_was_made_with() {
    each_deleted_and_put_back(LOD, 4);
}

#[test]
fn each_farthest_model_deleted_with_what_is_under_it_is_put_back_to_the_bytes_the_pack_was_made_with(
) {
    each_deleted_and_put_back(SLOD, 3);
}

const YARD: &str = "yard.metadata.src";

/// A pack of one map whose chain is inside it, each far model listed after what stands under it: a block over two rows, each over
/// a hut, and a post. The first hut is turned and scaled.
fn yard(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let map = map_file(
        "yard",
        &[
            Entity {
                parent: 3,
                yaw: 0.7,
                scale: 2.0,
                ..Entity::at("hut", [10.0, 10.0, 0.0], 1)
            },
            Entity::at("post", [5.0, 5.0, 0.0], 2),
            Entity {
                parent: 5,
                ..Entity::at("hut", [30.0, 10.0, 0.0], 3)
            },
            Entity {
                parent: 4,
                children: 1,
                ..Entity::at("row", [10.0, 10.0, 0.0], 4)
            },
            Entity {
                children: 2,
                ..Entity::at("block", [20.0, 10.0, 0.0], 5)
            },
            Entity {
                parent: 4,
                children: 1,
                ..Entity::at("row", [30.0, 10.0, 0.0], 6)
            },
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(&folder, &[incoming("yard.ymap", &map)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

#[test]
fn a_chain_inside_one_file_comes_back_in_the_order_of_the_base_with_each_count_of_what_stands_under_it(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = yard(scratch.path());
    let made = files(&folder);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    for index in 0..6 {
        let deleted = delete(&folder, &[at(YARD, index)]);
        let planned = put_back(&folder, &base(YARD, index));
        assert_eq!(planned.returns.len(), deleted.goes.len(), "{index}");
        assert_eq!(link_findings(&folder), Vec::<&str>::new(), "{index}");
        assert_eq!(changed(&folder, &[YARD]), Vec::<String>::new(), "{index}");
        assert_eq!(
            differing(&made, &files(&folder)),
            Vec::<String>::new(),
            "{index}"
        );
    }

    // The first hut goes with its row, and the block counts one; then the second goes with its row and the block, and the post
    // is all that stands.
    delete(&folder, &[at(YARD, 0)]);
    let hut = standing(&folder, YARD, 3).expect("the second hut stands");
    delete(&folder, &[at(YARD, hut)]);
    assert_eq!(map_of(&folder, YARD).entities.len(), 1);

    // The first hut comes back with its row and the block, which counts the one row that stands under it.
    let planned = put_back(&folder, &base(YARD, 0));
    assert_eq!(
        returns(&planned),
        [
            (YARD, 0, 0, Why::Chosen),
            (YARD, 3, 2, Why::FarModel),
            (YARD, 4, 3, Why::FarModel),
        ]
    );
    let held = map_of(&folder, YARD);
    assert_eq!(
        (held.entities[0].parent_index, held.entities[2].parent_index),
        (2, 3),
        "each names its far model where that one stands"
    );
    assert_eq!(held.entities[3].num_children, 1);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());

    // The second hut comes back with its row under the block that is there, which counts both again.
    let planned = put_back(&folder, &base(YARD, 2));
    assert_eq!(
        returns(&planned),
        [(YARD, 2, 2, Why::Chosen), (YARD, 5, 5, Why::FarModel)]
    );
    assert_eq!(recounted(&planned), [(YARD, 4, 1, 2)]);
    assert_eq!(names(&planned), ["insert_entities"]);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    assert_eq!(changed(&folder, &[YARD]), Vec::<String>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn an_entity_turned_scaled_and_moved_is_put_back_to_the_rotation_its_file_stores() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = yard(scratch.path());
    let made = files(&folder);
    let stored = map_of(&folder, YARD).entities[0].rotation;
    let turned = Operation::PlaceEntity {
        source: YARD.into(),
        expects: None,
        entity: 0,
        pose: PoseData {
            position: [11.0, 12.0, 1.0],
            rotation: [0.0, 0.6, 0.0, 0.8],
            scale_xy: 3.0,
            scale_z: 0.5,
        },
    };
    apply(&folder, &turned).unwrap();
    assert_ne!(map_of(&folder, YARD).entities[0].rotation, stored);

    let planned = put_back(&folder, &now(YARD, 0));
    let undone: Vec<&str> = (planned.restored.as_ref().expect("the hut is put back"))
        .changes
        .iter()
        .map(|change| match change {
            EntityChange::Moved { .. } => "moved",
            EntityChange::Turned { .. } => "turned",
            EntityChange::Scaled { .. } => "scaled",
            _ => "another",
        })
        .collect();
    assert_eq!(undone, ["moved", "turned", "scaled"]);
    assert_eq!(map_of(&folder, YARD).entities[0].rotation, stored);
    assert_eq!(changed(&folder, &[YARD]), Vec::<String>::new());
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_building_comes_back_into_a_parent_map_that_gained_an_entity_before_its_far_model() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    delete(&folder, &[at(CITY, TOWER)]);
    // A sign is put first into `city_lod`, and the links of `city` into it move on.
    let mut sign = read_entity(&folder.join(LOD), 2).unwrap();
    sign.guid = 999;
    sign.archetype_name = HashedString::new("lod_sign");
    sign.parent_index = -1;
    sign.flags = 32;
    sign.num_children = 0;
    let list = [
        Operation::InsertEntity {
            source: LOD.into(),
            expects: None,
            entity: 0,
            record: Record::entity(&sign).unwrap(),
            parent_num_children: None,
            children: Vec::new(),
            extents: None,
        },
        Operation::RenumberForInserted {
            source: CITY.into(),
            expects: None,
            inserted: vec![0],
        },
    ];
    apply_all(&folder, &list, false).unwrap();
    assert_eq!(link_findings(&folder), Vec::<&str>::new());

    // The far model of the tower comes back first, as the base has it, and the sign stands after it.
    let planned = put_back(&folder, &base(CITY, TOWER));
    assert_eq!(
        returns(&planned),
        [
            (CITY, TOWER, TOWER, Why::Chosen),
            (LOD, LOD_TOWER, LOD_TOWER, Why::FarModel),
        ]
    );
    assert_eq!(recounted(&planned), [(SLOD, SLOD_BLOCK, 1, 2)]);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    let lod = map_of(&folder, LOD);
    assert_eq!((lod.entities[0].guid, lod.entities[1].guid), (201, 999));
    let links: Vec<i32> = (map_of(&folder, CITY).entities[..5].iter())
        .map(|entity| entity.parent_index)
        .collect();
    assert_eq!(links, [0, 2, 2, 3, 4], "each link names what it named");
    let left = changes(&folder, &MAPS);
    let SourceChanges::Map(lod) = &left[LOD] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(
        (lod.added.len(), lod.deleted.len(), lod.changed.len()),
        (1, 0, 0),
        "the sign is all `city_lod` holds that its base does not"
    );
    assert!(left[CITY].is_empty() && left[SLOD].is_empty());
}

#[test]
fn an_entity_comes_back_under_a_far_model_that_was_moved_which_stays_where_it_was_moved_to() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    delete(&folder, &[at(CITY, HOUSE_A)]);
    move_and_flag(&folder, LOD, LOD_ROW, [21.0, 11.0, 0.0], 32 | 8);
    let planned = put_back(&folder, &base(CITY, HOUSE_A));
    assert_eq!(returns(&planned), [(CITY, HOUSE_A, HOUSE_A, Why::Chosen)]);
    assert_eq!(recounted(&planned), [(LOD, LOD_ROW, 1, 2)]);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    let left = changes(&folder, &MAPS);
    assert!(left[CITY].is_empty(), "{:?}", left[CITY]);
    let SourceChanges::Map(lod) = &left[LOD] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(lod.changed.len(), 1);
    assert_eq!(
        lod.changed[0].changes,
        [EntityChange::Moved {
            from: [20.0, 10.0, 0.0],
            to: [21.0, 11.0, 0.0],
        }],
        "the move of the far model is all that is left of it"
    );
}

#[test]
fn the_extents_hold_an_entity_the_source_added_and_a_twin_that_moved_once_another_is_put_back() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = city(scratch.path());
    // A lamp is added far to the east, and the lamp of the base goes far to the north.
    let added = Operation::AddEntity {
        source: CITY.into(),
        expects: None,
        archetype: "lamp".into(),
        pose: pose([150.0, 5.0, 5.0]),
        donor: Some(at(CITY, LAMP)),
    };
    apply(&folder, &added).unwrap();
    move_and_flag(&folder, CITY, LAMP, [5.0, 160.0, 5.0], 32);
    assert_eq!(
        extents(&map_of(&folder, CITY))[1],
        [151.0, 161.0, 100.0],
        "the box of a lamp is a metre each way"
    );
    put_back(&folder, &now(CITY, LAMP));
    assert_eq!(
        extents(&map_of(&folder, CITY))[1],
        [151.0, 100.0, 100.0],
        "the north is the base's again, and the east still holds the lamp that was added"
    );
    assert_eq!(link_findings(&folder), Vec::<&str>::new());

    // One of the twin trees goes far to the east, where nothing says which of the two it was, and a crate far to the north.
    let scratch = tempfile::tempdir().unwrap();
    let folder = harbour(scratch.path());
    for (entity, position) in [(3, [130.0, 31.0, 0.0]), (0, [10.0, 150.0, 0.5])] {
        let moved = Operation::PlaceEntity {
            source: HARBOUR.into(),
            expects: None,
            entity,
            pose: pose(position),
        };
        apply(&folder, &moved).unwrap();
    }
    assert_eq!(
        extents(&map_of(&folder, HARBOUR))[1],
        [130.0, 150.0, 100.0],
        "no type file gives a box, so each is held where it stands"
    );
    put_back(&folder, &now(HARBOUR, 0));
    assert_eq!(extents(&map_of(&folder, HARBOUR))[1], [130.0, 100.0, 100.0]);
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
}

const GROVE: &str = "grove.metadata.src";
const GROVE_LOD: &str = "grove_lod.metadata.src";

/// A pack of a far model of a stand of trees, and under it a map of two trees that state one guid, and a bench.
fn grove(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let extents = ([0.0; 3], [100.0; 3]);
    let lod = map_file(
        "grove_lod",
        &[Entity {
            children: 2,
            ..Entity::at("lod_trees", [32.0, 30.0, 0.0], 50)
        }],
        extents,
    );
    let near = child_map_file(
        "grove",
        "grove_lod",
        &[
            under("prop_tree", [30.0, 30.0, 0.0], 7, 0),
            under("prop_tree", [34.0, 30.0, 0.0], 7, 0),
            Entity::at("prop_bench", [18.0, 20.0, 0.5], 8),
        ],
        extents,
    );
    let added = add_files(
        &folder,
        &[
            incoming("grove_lod.ymap", &lod),
            incoming("grove.ymap", &near),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

#[test]
fn what_the_base_places_under_an_entity_that_comes_back_is_left_where_it_cannot_be_told_from_a_twin(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = grove(scratch.path());
    // The far model goes with the two trees under it, and a tree of another guid is planted: nothing says whether it is one of
    // the two.
    let deleted = delete(&folder, &[at(GROVE_LOD, 0)]);
    assert_eq!(deleted.goes.len(), 3);
    let planted = Operation::AddEntity {
        source: GROVE.into(),
        expects: None,
        archetype: "prop_tree".into(),
        pose: pose([60.0, 60.0, 0.0]),
        donor: None,
    };
    apply(&folder, &planted).unwrap();
    let SourceChanges::Map(found) = &changes(&folder, &[GROVE])[GROVE] else {
        panic!("a map source compares as a map");
    };
    assert_eq!((found.ambiguous.len(), found.deleted.len()), (1, 0));

    let planned = put_back(&folder, &base(GROVE_LOD, 0));
    assert_eq!(returns(&planned), [(GROVE_LOD, 0, 0, Why::Chosen)]);
    let left: Vec<(&str, usize, u32)> = (planned.left.iter())
        .map(|left| (left.source.as_str(), left.base.index, left.base.guid))
        .collect();
    assert_eq!(left, [(GROVE, 0, 7), (GROVE, 1, 7)]);
    assert_eq!(
        map_of(&folder, GROVE_LOD).entities[0].num_children,
        0,
        "the far model counts no tree while none stands under it"
    );
    assert_eq!(link_findings(&folder), Vec::<&str>::new());
    // ⚠ The comparison counts the children of an entity in its own file, so the far model reads as changed while it counts fewer.
    assert_eq!(changed(&folder, &[GROVE, GROVE_LOD]), [GROVE, GROVE_LOD]);

    // Each of the two is refused by itself, by its index in the base.
    let refused = put_back::plan(&folder, &base(GROVE, 1)).unwrap_err();
    assert!(
        matches!(&refused, Error::Ambiguous { index: 1, base, .. } if base == &[0, 1]),
        "{refused:?}"
    );
    assert!(
        refused.to_string().contains("of the base of `"),
        "{refused}"
    );
}

const PIER: &str = "pier.metadata.src";
const PIER_LOD: &str = "pier_lod.metadata.src";

/// A pack of two far models of posts that state one guid, and under it a map of the two posts, each under its own.
fn pier(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let extents = ([0.0; 3], [100.0; 3]);
    let lod = map_file(
        "pier_lod",
        &[
            Entity {
                children: 1,
                ..Entity::at("lod_post", [10.0, 10.0, 0.0], 60)
            },
            Entity {
                children: 1,
                ..Entity::at("lod_post", [20.0, 10.0, 0.0], 60)
            },
        ],
        extents,
    );
    let near = child_map_file(
        "pier",
        "pier_lod",
        &[
            under("post", [10.0, 10.0, 0.0], 1, 0),
            under("post", [20.0, 10.0, 0.0], 2, 1),
        ],
        extents,
    );
    let added = add_files(
        &folder,
        &[
            incoming("pier_lod.ymap", &lod),
            incoming("pier.ymap", &near),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

#[test]
fn an_entity_whose_far_model_cannot_be_told_from_a_twin_keeps_the_one_it_names_and_is_refused_once_it_names_none(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pier(scratch.path());
    // The second of the twin far models moves: it is one of a group nothing tells apart.
    let moved = Operation::PlaceEntity {
        source: PIER_LOD.into(),
        expects: None,
        entity: 1,
        pose: pose([22.0, 11.0, 0.0]),
    };
    apply(&folder, &moved).unwrap();
    let SourceChanges::Map(found) = &changes(&folder, &[PIER_LOD])[PIER_LOD] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(found.ambiguous.len(), 1);

    // The post under it is edited and put back: its link stays among the twins, so it keeps the far model it names.
    move_and_flag(&folder, PIER, 1, [21.0, 10.0, 0.0], 32 | 8 | 1);
    let planned = put_back(&folder, &now(PIER, 1));
    assert_eq!(names(&planned)[0], "replace_entity");
    assert_eq!(planned.returns, Vec::new());
    assert_eq!(map_of(&folder, PIER).entities[1].parent_index, 1);
    assert_eq!(changed(&folder, &[PIER]), Vec::<String>::new());

    // Taken out, it names nothing, and which of the twins to put it back under is not known.
    let gone = Operation::RemoveEntity {
        source: PIER.into(),
        expects: None,
        entity: 1,
        extents: None,
    };
    apply(&folder, &gone).unwrap();
    let edited = files(&folder);
    let refused = put_back::plan(&folder, &base(PIER, 1)).unwrap_err();
    assert!(
        matches!(
            &refused,
            Error::FarModelAmbiguous { at, index: 1, above, parent: 1 }
                if at == PIER && above == PIER_LOD
        ),
        "{refused:?}"
    );
    let said = refused.to_string();
    assert!(said.contains("cannot be told from a twin"), "{said}");
    assert!(
        said.contains("it is not known which entity to put it back under"),
        "{said}"
    );
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());
}

/// Move the source `source` of the pack in `folder` into its folder `into`, and name it there.
fn move_source(folder: &Path, source: &str, into: &str) -> String {
    let moved = format!("{into}/{source}");
    std::fs::create_dir_all(folder.join(into)).unwrap();
    std::fs::rename(folder.join(source), folder.join(into).join(source)).unwrap();
    let mut model = Maps::read(folder).unwrap();
    for set in &mut model.maps {
        for named in set.sources.iter_mut().filter(|named| *named == source) {
            *named = moved.clone();
        }
    }
    model.write(folder).unwrap();
    moved
}

#[test]
fn a_map_two_sources_each_hold_entities_of_is_refused_by_the_names_of_both() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let extents = ([0.0; 3], [100.0; 3]);
    // Two sources that each cook to `quay.ymap`, in two folders of the pack: each holds entities, so an index into the map
    // names neither.
    let part = |archetype: &str, guid: u32| {
        map_file(
            "quay",
            &[
                Entity::at(archetype, [10.0, 20.0, 0.5], guid),
                Entity::at(archetype, [14.0, 20.0, 0.5], guid + 1),
            ],
            extents,
        )
    };
    let (first, second) = (part("prop_bollard", 1), part("prop_lamp", 11));
    for (into, bytes) in [("east", &first), ("west", &second)] {
        let added = add_files(&folder, &[incoming("quay.ymap", bytes)], None).unwrap();
        assert_eq!(added.refused, Vec::<(String, String)>::new());
        move_source(&folder, QUAY, into);
    }
    let (east, west) = (format!("east/{QUAY}"), format!("west/{QUAY}"));
    let gone = Operation::RemoveEntity {
        source: east.clone(),
        expects: None,
        entity: 0,
        extents: None,
    };
    apply(&folder, &gone).unwrap();
    let edited = files(&folder);

    let refused = put_back::plan(&folder, &base(&east, 0)).unwrap_err();
    let Error::SplitMap(sources) = &refused else {
        panic!("{refused:?}");
    };
    assert_eq!(sources, &[east.clone(), west]);
    assert!(
        refused
            .to_string()
            .contains("each hold entities of one map"),
        "{refused}"
    );
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_file_that_states_the_name_of_another_map_is_no_part_of_it_and_an_entity_comes_back_into_its_own_file(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let extents = ([0.0; 3], [100.0; 3]);
    // ⚠ `depot.ymap` states the name `quay_lod`, as a file renamed without its name does: it is the map `depot` all the same,
    // and the link of `quay` leads into the file called `quay_lod`.
    let far = |archetype: &'static str, guid: u32| Entity {
        children: 1,
        ..Entity::at(archetype, [10.0, 20.0, 0.5], guid)
    };
    let depot = map_file("quay_lod", &[far("depot_far", 71)], extents);
    let quay_lod = map_file("quay_lod", &[far("crane_far", 20)], extents);
    let quay = child_map_file(
        "quay",
        "quay_lod",
        &[
            under("crane", [10.0, 20.0, 0.5], 1, 0),
            Entity::at("prop_crate_a", [30.0, 20.0, 0.5], 2),
        ],
        extents,
    );
    let added = add_files(
        &folder,
        &[
            incoming("depot.ymap", &depot),
            incoming("quay_lod.ymap", &quay_lod),
            incoming("quay.ymap", &quay),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let made = files(&folder);
    let (depot, quay_lod) = ("depot.metadata.src", "quay_lod.metadata.src");

    // The crane goes with its far model in `quay_lod.ymap`, and `depot.ymap` keeps what it holds.
    let deleted = delete(&folder, &[at(QUAY, 0)]);
    assert_eq!(deleted.goes.len(), 2);
    assert_eq!(map_of(&folder, depot).entities.len(), 1);

    // Two files state the name and one is called so: no map is said to be split, and the far model comes back into its file.
    let planned = put_back(&folder, &base(QUAY, 0));
    assert_eq!(
        returns(&planned),
        [(QUAY, 0, 0, Why::Chosen), (quay_lod, 0, 0, Why::FarModel)]
    );
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_link_of_the_base_that_names_nothing_is_stated_as_it_is_and_said() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let extents = ([0.0; 3], [100.0; 3]);
    // A map that names no parent, whose crate says its far model is in one; and a map whose crate names the tenth entity of a
    // parent map that holds one.
    let lonely = map_file(
        "lonely",
        &[under("prop_crate_a", [10.0, 20.0, 0.5], 1, 0)],
        extents,
    );
    let far = map_file(
        "cove_lod",
        &[Entity::at("lod_cove", [10.0, 20.0, 0.5], 20)],
        extents,
    );
    let cove = child_map_file(
        "cove",
        "cove_lod",
        &[under("prop_crate_a", [10.0, 20.0, 0.5], 2, 9)],
        extents,
    );
    let added = add_files(
        &folder,
        &[
            incoming("lonely.ymap", &lonely),
            incoming("cove_lod.ymap", &far),
            incoming("cove.ymap", &cove),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let made = files(&folder);

    for (source, said) in [
        (
            "lonely.metadata.src",
            "names a far model in a parent map, and its map names none",
        ),
        (
            "cove.metadata.src",
            "names entity 9 of its parent map, whose base holds 1",
        ),
    ] {
        move_and_flag(&folder, source, 0, [12.0, 20.0, 0.5], 32 | 8 | 1);
        let planned = put_back(&folder, &now(source, 0));
        assert_eq!(planned.notes.len(), 1, "{:?}", planned.notes);
        let note = &planned.notes[0];
        assert!(note.contains(said), "{note}");
        assert!(
            note.starts_with(&format!("{source} entity 0 of the base "))
                && note.ends_with("the link is stated as the base states it"),
            "{note}"
        );
        assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
    }
}

const SHORE: &str = "shore.metadata.src";

/// A map file of two crates that states `lods` of itself in a member no typed record names.
fn a_map_that_states_what_no_record_types(lods: u64) -> Vec<u8> {
    const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
    <float name="scaleXY"/>
    <float name="scaleZ"/>
    <s32 name="parentIndex"/>
    <float name="lodDist"/>
    <float name="childLodDist"/>
    <u32 name="numChildren"/>
    <s32 name="ambientOcclusionMultiplier"/>
    <s32 name="artificialAmbientOcclusion"/>
    <u32 name="tintValue"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <string name="parent" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="contentFlags"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <u32 name="containerLods"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;
    assert!(
        !CMapData::MEMBERS.contains(&"containerLods"),
        "the member is one the typed record carries and does not name"
    );
    let entity = |guid: u64, x: f32| {
        let mut inst = MetaInstance::new("CEntityDef");
        inst.set(
            "archetypeName",
            Value::Hash(HashedString::new("prop_crate_a")),
        );
        inst.set("flags", Value::UInt(32));
        inst.set("guid", Value::UInt(guid));
        inst.set("position", Value::Vec3([x, 20.0, 0.5]));
        inst.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
        inst.set("scaleXY", Value::Float(1.0));
        inst.set("scaleZ", Value::Float(1.0));
        inst.set("parentIndex", Value::Int(-1));
        inst.set("lodDist", Value::Float(100.0));
        inst.set("childLodDist", Value::Float(0.0));
        inst.set("numChildren", Value::UInt(0));
        inst.set("ambientOcclusionMultiplier", Value::Int(255));
        inst.set("artificialAmbientOcclusion", Value::Int(255));
        inst.set("tintValue", Value::UInt(0));
        Value::Pointer(Some(Box::new(inst)))
    };
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new("shore")));
    root.set("parent", Value::Hash(HashedString::from_hash(0)));
    root.set("flags", Value::UInt(0));
    root.set("contentFlags", Value::UInt(65));
    root.set("streamingExtentsMin", Value::Vec3([0.0; 3]));
    root.set("streamingExtentsMax", Value::Vec3([100.0; 3]));
    root.set("entitiesExtentsMin", Value::Vec3([0.0; 3]));
    root.set("entitiesExtentsMax", Value::Vec3([100.0; 3]));
    root.set("containerLods", Value::UInt(lods));
    root.set(
        "entities",
        Value::Array(vec![entity(1, 10.0), entity(2, 14.0)]),
    );
    let library = build_library([("inline.psc", PSC.as_bytes())]).unwrap();
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).unwrap();
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).unwrap()
}

#[test]
fn a_file_that_takes_no_record_and_states_what_no_member_names_otherwise_is_refused_and_not_written_as_its_base(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let map = a_map_that_states_what_no_record_types(1);
    let added = add_files(&folder, &[incoming("shore.ymap", &map)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());

    // The map is emptied, so its file states no class of an entity, and what it states of itself in a member no record types
    // is changed in its XML.
    let all = Operation::RemoveEntities {
        source: SHORE.into(),
        expects: None,
        entities: vec![0, 1],
        extents: None,
    };
    apply(&folder, &all).unwrap();
    let xml = std::fs::read_to_string(folder.join(SHORE).join("metadata.xml")).unwrap();
    let stated = "<containerLods value=\"1\"";
    assert_eq!(xml.matches(stated).count(), 1, "{xml}");
    let otherwise = Operation::RewriteSource {
        source: SHORE.into(),
        expects: None,
        xml: xml.replace(stated, "<containerLods value=\"2\""),
    };
    apply(&folder, &otherwise).unwrap();
    let SourceChanges::Map(found) = &changes(&folder, &[SHORE])[SHORE] else {
        panic!("a map source compares as a map");
    };
    assert!(found.other && found.deleted.len() == 2, "{found:?}");
    let edited = files(&folder);

    // ⚠ The file of the base would take the record, and states the member as the base does: written so, the change would be lost.
    let refused = put_back::plan(&folder, &base(SHORE, 0)).unwrap_err();
    assert!(
        matches!(&refused, Error::Unwritable { at, .. } if at == SHORE),
        "{refused:?}"
    );
    let said = refused.to_string();
    assert!(
        said.starts_with(&format!(
            "`{SHORE}` cannot be written with the entity put back: "
        )),
        "{said}"
    );
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());
}

const LOT: &str = "lot.metadata.src";

/// The flags the parked vehicle of [`a_map_that_parks_a_vehicle`] states, which no other number of the file is.
const PARKED: u64 = 7717;

/// A map of two crates that parks one vehicle.
fn a_map_that_parks_a_vehicle() -> Vec<u8> {
    const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
    <float name="scaleXY"/>
    <float name="scaleZ"/>
    <s32 name="parentIndex"/>
    <float name="lodDist"/>
    <float name="childLodDist"/>
    <u32 name="numChildren"/>
    <s32 name="ambientOcclusionMultiplier"/>
    <s32 name="artificialAmbientOcclusion"/>
    <u32 name="tintValue"/>
  </structdef>
  <structdef type="CCarGen">
    <Vector3 name="position"/>
    <string name="carModel" type="atHashString"/>
    <u32 name="flags"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <string name="parent" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="contentFlags"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
    <array name="carGenerators" type="atArray"><struct type="CCarGen"/></array>
  </structdef>
</ParserSchema>"#;
    let entity = |guid: u64, x: f32| {
        let mut inst = MetaInstance::new("CEntityDef");
        inst.set(
            "archetypeName",
            Value::Hash(HashedString::new("prop_crate_a")),
        );
        inst.set("flags", Value::UInt(32));
        inst.set("guid", Value::UInt(guid));
        inst.set("position", Value::Vec3([x, 20.0, 0.5]));
        inst.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
        inst.set("scaleXY", Value::Float(1.0));
        inst.set("scaleZ", Value::Float(1.0));
        inst.set("parentIndex", Value::Int(-1));
        inst.set("lodDist", Value::Float(100.0));
        inst.set("childLodDist", Value::Float(0.0));
        inst.set("numChildren", Value::UInt(0));
        inst.set("ambientOcclusionMultiplier", Value::Int(255));
        inst.set("artificialAmbientOcclusion", Value::Int(255));
        inst.set("tintValue", Value::UInt(0));
        Value::Pointer(Some(Box::new(inst)))
    };
    let mut parked = MetaInstance::new("CCarGen");
    parked.set("position", Value::Vec3([50.0, 50.0, 0.5]));
    parked.set("carModel", Value::Hash(HashedString::new("a_saloon")));
    parked.set("flags", Value::UInt(PARKED));
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new("lot")));
    root.set("parent", Value::Hash(HashedString::from_hash(0)));
    root.set("flags", Value::UInt(0));
    root.set("contentFlags", Value::UInt(65));
    root.set("streamingExtentsMin", Value::Vec3([-200.0; 3]));
    root.set("streamingExtentsMax", Value::Vec3([300.0; 3]));
    root.set("entitiesExtentsMin", Value::Vec3([0.0; 3]));
    root.set("entitiesExtentsMax", Value::Vec3([100.0; 3]));
    root.set(
        "entities",
        Value::Array(vec![entity(1, 10.0), entity(2, 14.0)]),
    );
    root.set(
        "carGenerators",
        Value::Array(vec![Value::Struct(Box::new(parked))]),
    );
    let library = build_library([("inline.psc", PSC.as_bytes())]).unwrap();
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).unwrap();
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).unwrap()
}

#[test]
fn an_entity_put_back_leaves_the_parked_vehicle_the_source_changed_as_the_source_states_it() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let map = a_map_that_parks_a_vehicle();
    let added = add_files(&folder, &[incoming("lot.ymap", &map)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    assert_eq!(map_of(&folder, LOT).car_generators.len(), 1);

    // The source states other flags of its parked vehicle, a record the map types beside its entities.
    let xml = std::fs::read_to_string(folder.join(LOT).join("metadata.xml")).unwrap();
    let stated = format!("<flags value=\"{PARKED}\"");
    assert_eq!(xml.matches(&stated).count(), 1, "{xml}");
    let otherwise = Operation::RewriteSource {
        source: LOT.into(),
        expects: None,
        xml: xml.replace(&stated, &format!("<flags value=\"{}\"", PARKED + 1)),
    };
    apply(&folder, &otherwise).unwrap();
    // What is left of the source's changes once its entities are put back: the vehicle, as the source states it.
    let only_the_vehicle = |when: &str| {
        let SourceChanges::Map(found) = &changes(&folder, &[LOT])[LOT] else {
            panic!("a map source compares as a map");
        };
        assert_eq!(found.records, ["carGenerators"], "{when}: {found:?}");
        assert_eq!(found.count(), 1, "{when}: {found:?}");
        let parked = map_of(&folder, LOT).car_generators;
        assert_eq!(parked.len(), 1, "{when}");
        assert_eq!(u64::from(parked[0].flags), PARKED + 1, "{when}");
    };
    only_the_vehicle("as edited");

    // A crate moved and put back by one record in its place.
    move_and_flag(&folder, LOT, 0, [40.0, 40.0, 0.5], 32);
    put_back(&folder, &now(LOT, 0));
    only_the_vehicle("a moved crate put back");

    // A crate deleted and put back by an insertion.
    let gone = |entities: Vec<usize>| Operation::RemoveEntities {
        source: LOT.into(),
        expects: None,
        entities,
        extents: None,
    };
    apply(&folder, &gone(vec![1])).unwrap();
    put_back(&folder, &base(LOT, 1));
    only_the_vehicle("a deleted crate put back");

    // ⚠ The map emptied states no class of an entity, so where no schema snapshot is installed the plan writes the source
    // whole, from the file of its base: the vehicle that file states is the base's, and the one written is the source's.
    apply(&folder, &gone(vec![0, 1])).unwrap();
    let planned = put_back(&folder, &base(LOT, 0));
    assert!(
        matches!(
            names(&planned).as_slice(),
            ["insert_entities"] | ["rewrite_source"]
        ),
        "{:?}",
        names(&planned)
    );
    put_back(&folder, &base(LOT, 1));
    only_the_vehicle("an emptied map put back");
}

#[test]
fn a_far_model_comes_back_into_the_file_of_its_map_that_held_it_where_another_file_of_that_map_holds_none(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let extents = ([0.0; 3], [100.0; 3]);
    // Two sources of the pack each cook to `dock_lod.ymap`, in two folders: one holds its one entity and the other none, and
    // `dock` is under it.
    let far = map_file(
        "dock_lod",
        &[Entity {
            children: 1,
            ..Entity::at("lod_crane", [40.0, 40.0, 0.0], 20)
        }],
        extents,
    );
    let empty = map_file("dock_lod", &[], extents);
    let near = child_map_file(
        "dock",
        "dock_lod",
        &[
            under("crane", [40.0, 40.0, 0.0], 1, 0),
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 2),
        ],
        extents,
    );
    for (into, bytes) in [("held", &far), ("empty", &empty)] {
        let added = add_files(&folder, &[incoming("dock_lod.ymap", bytes)], None).unwrap();
        assert_eq!(added.refused, Vec::<(String, String)>::new());
        move_source(&folder, "dock_lod.metadata.src", into);
    }
    let added = add_files(&folder, &[incoming("dock.ymap", &near)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let made = files(&folder);
    let (dock, dock_lod) = ("dock.metadata.src", "held/dock_lod.metadata.src");
    // ⚠ The rules read a map of two files their own way, and say the same of this pack before and after.
    let as_made = link_findings(&folder);

    // The crane goes with its far model, and then neither file of `dock_lod` holds an entity.
    let deleted = delete(&folder, &[at(dock, 0)]);
    assert_eq!(deleted.goes.len(), 2);
    assert_eq!(map_of(&folder, dock_lod).entities.len(), 0);

    let planned = put_back(&folder, &base(dock, 0));
    assert_eq!(
        returns(&planned),
        [(dock, 0, 0, Why::Chosen), (dock_lod, 0, 0, Why::FarModel),],
        "the far model comes back into the file whose base holds it"
    );
    assert_eq!(link_findings(&folder), as_made);
    assert_eq!(differing(&made, &files(&folder)), Vec::<String>::new());
}

#[test]
fn an_entity_whose_base_names_a_far_model_past_the_entities_of_its_file_is_refused_with_the_sentence(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    // The game's file is wrong as it is: the crate names the tenth entity of a map of two.
    let map = map_file(
        "wharf",
        &[
            Entity {
                parent: 9,
                ..Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1)
            },
            Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 2),
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(&folder, &[incoming("wharf.ymap", &map)], None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let wharf = "wharf.metadata.src";
    // The crate is moved in the XML of its source, as a person types it.
    let dir = folder.join(wharf).join("metadata.xml");
    let xml = std::fs::read_to_string(&dir).unwrap();
    let stood = "<position x=\"10.0\"";
    assert_eq!(xml.matches(stood).count(), 1, "{xml}");
    std::fs::write(&dir, xml.replace(stood, "<position x=\"11.0\"")).unwrap();
    let SourceChanges::Map(found) = &changes(&folder, &[wharf])[wharf] else {
        panic!("a map source compares as a map");
    };
    assert_eq!(found.changed.len(), 1);
    let edited = files(&folder);

    // ⚠ No record goes into a map with a link the game would follow past its entities, so the entity is not put back.
    let refused = put_back::plan(&folder, &now(wharf, 0)).unwrap_err();
    assert!(
        matches!(&refused, Error::Refused { name: "replace_entity", at, .. } if at == wharf),
        "{refused:?}"
    );
    let said = refused.to_string();
    assert!(
        said.contains("would name LOD parent 9, past the 2 entities of the map"),
        "{said}"
    );
    assert_eq!(differing(&edited, &files(&folder)), Vec::<String>::new());
}
