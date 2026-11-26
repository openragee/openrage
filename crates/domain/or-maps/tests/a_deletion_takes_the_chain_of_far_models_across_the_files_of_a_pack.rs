//! A deletion planned over the map files of a pack takes an entity with its chain of far models across files, leaves a far model
//! that stands for another entity and lowers its count, and is one list of operations whose inverses put every file back.
//!
//! The pack is three map files built by hand, each the parent map of the next: `city_slod`, `city_lod` and `city`. After every
//! deletion the files are cooked and read as value trees, and judged by `or_world`: the checks of one file, the checks of the group,
//! and every LOD link by the identity of the two entities it joins, which a renumbering that is off by one does not keep.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_io::meta::{MetaDocument, MetaInstance};
use or_maps::delete::{self, Deletion, Error, Lowered, Note, Why};
use or_maps::import::{add_files, Incoming};
use or_maps::operation::{apply_all, inverses, At, Operation};
use or_maps::sample::{child_map_file, map_file, types_file, Archetype, Entity};
use or_maps::script_text::{ScriptForms, HIDE_RADIUS};
use or_maps::Maps;
use or_world::execute::{identity, links_by_identity, Key};
use or_world::{MapData, MapGroup};

const SLOD: &str = "city_slod.metadata.src";
const LOD: &str = "city_lod.metadata.src";
const CITY: &str = "city.metadata.src";
const TYPES: &str = "city_types.metadata.src";
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
const LAMP_TWIN: usize = 8;
const LAMP_APART: usize = 9;
const FENCE_END: usize = 10;
/// The entities of `city_lod`.
const LOD_TOWER: usize = 0;
const LOD_ROW: usize = 1;
const LOD_SHED: usize = 2;
const LOD_BARN: usize = 3;
/// The entities of `city_slod`.
const SLOD_SIGN: usize = 0;
const SLOD_BLOCK: usize = 1;
const SLOD_FAR: usize = 2;

fn at(source: &str, index: usize) -> At {
    At {
        source: source.to_string(),
        index,
    }
}

/// An entity that names the entity `parent` of the parent map as its LOD parent.
fn under<'a>(archetype: &'a str, position: [f32; 3], guid: u32, parent: i32) -> Entity<'a> {
    Entity {
        parent,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    }
}

/// A pack of three map files that load together and a type file.
///
/// `city_slod` holds a sign that stands for nothing, a block that stands for the tower's and the row's far models, and a far model of
/// the shed's far model. `city_lod` holds the far models of the tower, of the row of two houses and of the shed, each under
/// `city_slod`, and the far model of the barn, which has no parent. `city` holds the tower, the two houses, the shed and the barn, each
/// under its far model in `city_lod`; two lengths of fence under a far model of their own file; and three lamps, two of them at one
/// pose. The tower stands where no two of its coordinates are one number, so a place written with two of them swapped is seen.
fn pack(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
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
        ([0.0; 3], [100.0; 3]),
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
        ([0.0; 3], [100.0; 3]),
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
            Entity::at("lamp", [5.0, 5.0, 0.0], 308),
            Entity::at("lamp", [5.0, 5.0, 0.0], 309),
            Entity::at("lamp", [6.0, 5.0, 0.0], 310),
            Entity {
                parent: FENCE_FAR as i32,
                ..Entity::at("fence", [74.0, 70.0, 0.0], 311)
            },
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let types = types_file(
        "city_types",
        &[Archetype::new("tower", [-1.0; 3], [1.0; 3])],
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
            incoming("city_slod.ymap", slod.leak()),
            incoming("city_lod.ymap", lod.leak()),
            incoming("city.ymap", city.leak()),
            incoming("city_types.ytyp", types.leak()),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    // The names a person typed, which a map file keeps the hash of alone.
    let mut model = Maps::read(&folder).unwrap();
    model.remember(&["tower", "lod_tower", "house"]);
    model.write(&folder).unwrap();
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

/// The value tree each map file of the pack cooks to, by its source.
fn trees(folder: &Path) -> BTreeMap<&'static str, MetaInstance> {
    MAPS.into_iter()
        .map(|source| {
            let cooked = or_maps::facts::cooked(&folder.join(source)).expect(source);
            let document = MetaDocument::parse(or_io::meta::default_library(), cooked).unwrap();
            (source, document.instance)
        })
        .collect()
}

/// What `or_world` reads of the pack: every LOD link by the identity of its two ends, and the children each entity counts.
struct Read {
    links: BTreeMap<Key, Key>,
    counts: BTreeMap<Key, u32>,
    keys: BTreeMap<(&'static str, usize), Key>,
}

/// Read the pack, and hold every file of it to the checks of one map and to the checks of the group.
fn read(folder: &Path) -> Read {
    let trees = trees(folder);
    let mut group = MapGroup::new();
    let mut counts = BTreeMap::new();
    let mut keys = BTreeMap::new();
    for (source, tree) in &trees {
        let map = MapData::new(tree).expect("a map file");
        assert_eq!(
            or_world::validate::check(&map),
            Vec::new(),
            "{source} is held to the checks of one file"
        );
        group.add_from(&map, source);
        for (index, entity) in map.entities().enumerate() {
            let key = identity(entity.instance());
            assert_eq!(
                counts.insert(key, entity.num_children()),
                None,
                "the entities of the pack are told apart by archetype, place and guid"
            );
            keys.insert((*source, index), key);
        }
    }
    assert_eq!(group.len(), MAPS.len());
    assert_eq!(group.check(), Vec::new(), "the group holds");
    assert_eq!(
        group.child_count_disagreements(),
        Vec::new(),
        "every count is the number of entities that name it"
    );
    let (inside, outside) = group.unresolved_links();
    assert_eq!(outside, 0, "of {inside} links into a parent map");
    Read {
        links: links_by_identity(trees.values()),
        counts,
        keys,
    }
}

/// Plan the deletion of `chosen`, apply it, hold what it left to every check, then take it back; the plan, for what a case says of it.
///
/// Held for every case: what goes is what the plan says and nothing else, every link that stays joins the two entities it joined,
/// every count is lowered by the children that went and by no more, the plan lists the files written and no other, no inverse is the
/// XML as it was, and the inverses put every file back byte for byte.
fn delete(chosen: &[At]) -> Deletion {
    delete_emptying(chosen, &[])
}

/// [`delete`], for a deletion that leaves the sources `emptied` with no entity.
///
/// ⚠ A map file with no entity declares no entity, and where no schema snapshot is staged none can be put back into it: the inverse
/// of the removal that emptied a file may be the XML as it was, and of no other.
fn delete_emptying(chosen: &[At], emptied: &[&str]) -> Deletion {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);
    let was = read(&folder);

    let planned = delete::plan(&folder, chosen).expect("plans");
    assert!(files(&folder) == before, "a plan writes nothing");
    let applied = delete::apply(&folder, &planned).expect("applies");
    let after = files(&folder);
    let now = read(&folder);

    let gone: BTreeSet<Key> = (planned.goes.iter())
        .map(|going| {
            let source = MAPS.iter().find(|s| **s == going.entity.at.source).unwrap();
            was.keys[&(*source, going.entity.at.index)]
        })
        .collect();
    assert_eq!(gone.len(), planned.goes.len(), "an entity goes once");
    let left: BTreeSet<Key> = now.counts.keys().copied().collect();
    let kept: BTreeSet<Key> = (was.counts.keys().copied())
        .filter(|key| !gone.contains(key))
        .collect();
    assert_eq!(
        left, kept,
        "what the plan says goes, goes, and nothing else"
    );

    let links: BTreeMap<Key, Key> = (was.links.iter())
        .filter(|(child, _)| !gone.contains(child))
        .map(|(child, parent)| (*child, *parent))
        .collect();
    assert_eq!(
        now.links, links,
        "every link that stays joins the two entities it joined"
    );
    for (key, count) in &now.counts {
        let lost = (was.links.iter())
            .filter(|(child, parent)| *parent == key && gone.contains(child))
            .count();
        assert_eq!(
            *count as usize,
            was.counts[key] as usize - lost,
            "a count is lowered by the children that went, and by no more"
        );
    }

    let written: BTreeSet<&str> = (after.iter())
        .filter(|(path, bytes)| before.get(*path) != Some(bytes))
        .map(|(path, _)| path.as_str())
        .collect();
    let listed: BTreeSet<&str> = planned.files.iter().map(|f| f.file.as_str()).collect();
    assert_eq!(
        listed, written,
        "the plan lists the files written and no other"
    );
    assert_eq!(after.len(), before.len(), "a deletion makes no file");
    let said: BTreeSet<String> = (applied.iter())
        .filter(|done| done.edited.changed())
        .map(|done| {
            let inside = done.edited.file.strip_prefix(&folder).unwrap();
            inside.to_string_lossy().replace('\\', "/")
        })
        .collect();
    assert_eq!(said, listed.iter().map(|s| s.to_string()).collect());
    assert!(
        applied.iter().all(|done| done.edited.changed()),
        "the list holds no operation that changes nothing"
    );

    let back = inverses(&applied);
    for source in MAPS {
        let entities = now.keys.keys().filter(|(held, _)| *held == source);
        assert_eq!(
            entities.count() == 0,
            emptied.contains(&source),
            "{source} is left with no entity only where the case says so"
        );
    }
    assert!(
        (back.iter()).all(
            |inverse| inverse.name() != "rewrite_source" || emptied.contains(&inverse.source())
        ),
        "each inverse says what to put back, not the XML as it was: {:?}",
        (back
            .iter()
            .map(|inverse| (inverse.name(), inverse.source())))
        .collect::<Vec<_>>()
    );
    apply_all(&folder, &back, false).expect("the inverses apply");
    let restored = files(&folder);
    for (path, bytes) in &before {
        assert!(
            restored.get(path) == Some(bytes),
            "{path} is not as it was after the inverses"
        );
    }
    assert_eq!(restored.len(), before.len());
    planned
}

/// The entities the plan takes out, with why, in the order it says them.
fn goes(planned: &Deletion) -> Vec<(At, Why)> {
    (planned.goes.iter())
        .map(|going| (going.entity.at.clone(), going.why.clone()))
        .collect()
}

/// The file `source` as the plan says it changes: the entities removed, the counts lowered, and the links renumbered inside it and into its parent map.
fn change(planned: &Deletion, source: &str) -> Option<(Vec<usize>, Vec<Lowered>, usize, usize)> {
    let file = planned.files.iter().find(|file| file.source == source)?;
    assert_eq!(file.file, format!("{source}/metadata.xml"));
    Some((
        file.removed.clone(),
        file.lowered.clone(),
        file.renumbered,
        file.renumbered_into_parent,
    ))
}

#[test]
fn the_pack_built_by_hand_holds_before_anything_is_deleted() {
    let scratch = tempfile::tempdir().unwrap();
    let read = read(&pack(scratch.path()));
    // Three far models under two of `city_slod`, five buildings under the four of `city_lod`, and two lengths of fence under their own.
    assert_eq!(read.links.len(), 3 + 5 + 2);
}

#[test]
fn a_building_with_one_far_model_goes_with_it_and_no_count_is_lowered() {
    let planned = delete(&[at(CITY, BARN)]);
    assert_eq!(
        goes(&planned),
        [
            (at(CITY, BARN), Why::Chosen),
            (at(LOD, LOD_BARN), Why::LeftWithNoChild),
        ]
    );
    assert_eq!(planned.stays, Vec::new());
    assert_eq!(planned.ambiguous, Vec::new());
    assert_eq!(planned.notes, Vec::new());
    // The two lengths of fence name the entity after the barn, which moves down by one.
    assert_eq!(change(&planned, CITY), Some((vec![BARN], vec![], 2, 0)));
    assert_eq!(change(&planned, LOD), Some((vec![LOD_BARN], vec![], 0, 0)));
    assert_eq!(change(&planned, SLOD), None);
    assert_eq!(planned.files.len(), 2);

    // Named twice, it is chosen once.
    let twice = delete(&[at(CITY, BARN), at(CITY, BARN)]);
    assert_eq!(twice, planned);
}

#[test]
fn one_of_two_buildings_that_share_a_far_model_goes_and_the_far_model_stays_with_one_child_fewer() {
    let planned = delete(&[at(CITY, HOUSE_A)]);
    assert_eq!(goes(&planned), [(at(CITY, HOUSE_A), Why::Chosen)]);
    let [stays] = planned.stays.as_slice() else {
        panic!("one far model stays: {:?}", planned.stays);
    };
    assert_eq!(stays.entity.at, at(LOD, LOD_ROW));
    assert_eq!(
        (stays.children_left, stays.children_elsewhere),
        (1, 0),
        "it still stands for the other house"
    );
    assert_eq!((stays.counted, stays.counts), (2, 1));
    let lowered = Lowered {
        entity: LOD_ROW,
        from: 2,
        to: 1,
    };
    assert_eq!(change(&planned, LOD), Some((vec![], vec![lowered], 0, 0)));
    assert_eq!(change(&planned, CITY), Some((vec![HOUSE_A], vec![], 2, 0)));
    assert_eq!(change(&planned, SLOD), None);
}

#[test]
fn the_last_child_of_a_parent_whose_own_parent_has_other_children_takes_the_first_parent_only() {
    let planned = delete(&[at(CITY, TOWER)]);
    assert_eq!(
        goes(&planned),
        [
            (at(CITY, TOWER), Why::Chosen),
            (at(LOD, LOD_TOWER), Why::LeftWithNoChild),
        ]
    );
    let [stays] = planned.stays.as_slice() else {
        panic!("the block stays: {:?}", planned.stays);
    };
    assert_eq!(stays.entity.at, at(SLOD, SLOD_BLOCK));
    assert_eq!(
        (stays.children_left, stays.counted, stays.counts),
        (1, 2, 1)
    );
    let lowered = Lowered {
        entity: SLOD_BLOCK,
        from: 2,
        to: 1,
    };
    assert_eq!(change(&planned, SLOD), Some((vec![], vec![lowered], 0, 0)));
    // The far models of the row and of the shed stay under the entities of `city_slod`, none of which moved.
    assert_eq!(change(&planned, LOD), Some((vec![LOD_TOWER], vec![], 0, 0)));
    // The two houses, the shed and the barn name far models that each moved down by one; the fence names its own, which did too.
    assert_eq!(change(&planned, CITY), Some((vec![TOWER], vec![], 2, 4)));
    assert_eq!(
        planned
            .operations
            .iter()
            .map(Operation::name)
            .collect::<Vec<_>>(),
        [
            "set_num_children",
            "remove_entities",
            "remove_entities",
            "renumber_for_removed"
        ]
    );
}

#[test]
fn several_entities_go_in_one_list_and_a_block_left_with_no_far_model_goes_too() {
    let chosen = [at(CITY, HOUSE_B), at(CITY, TOWER), at(CITY, HOUSE_A)];
    let planned = delete(&chosen);
    assert_eq!(
        goes(&planned),
        [
            (at(CITY, HOUSE_B), Why::Chosen),
            (at(CITY, TOWER), Why::Chosen),
            (at(CITY, HOUSE_A), Why::Chosen),
            // The far models go in the order of the chosen whose last child each lost.
            (at(LOD, LOD_ROW), Why::LeftWithNoChild),
            (at(LOD, LOD_TOWER), Why::LeftWithNoChild),
            (at(SLOD, SLOD_BLOCK), Why::LeftWithNoChild),
        ]
    );
    assert_eq!(planned.stays, Vec::new());
    assert_eq!(
        change(&planned, CITY),
        Some((vec![TOWER, HOUSE_A, HOUSE_B], vec![], 2, 2))
    );
    // The far model of the shed names the entity of `city_slod` after the block, which moves down by one.
    assert_eq!(
        change(&planned, LOD),
        Some((vec![LOD_TOWER, LOD_ROW], vec![], 0, 1))
    );
    assert_eq!(
        change(&planned, SLOD),
        Some((vec![SLOD_BLOCK], vec![], 0, 0))
    );
    // One removal a file, whatever it loses.
    let removals = (planned.operations.iter()).filter(|op| op.name() == "remove_entities");
    assert_eq!(removals.count(), 3);
}

#[test]
fn a_far_model_that_is_chosen_takes_what_stands_under_it_in_the_map_below() {
    let planned = delete(&[at(LOD, LOD_ROW)]);
    assert_eq!(
        goes(&planned),
        [
            (at(LOD, LOD_ROW), Why::Chosen),
            (at(CITY, HOUSE_A), Why::Under(at(LOD, LOD_ROW))),
            (at(CITY, HOUSE_B), Why::Under(at(LOD, LOD_ROW))),
        ]
    );
    assert_eq!(planned.stays.len(), 1);
    assert_eq!(planned.stays[0].entity.at, at(SLOD, SLOD_BLOCK));

    // Two levels down: the block takes the far models under it, and each of those what stands under it.
    let planned = delete(&[at(SLOD, SLOD_BLOCK)]);
    let under = |source: &str, index: usize, parent: At| (at(source, index), Why::Under(parent));
    assert_eq!(
        goes(&planned),
        [
            (at(SLOD, SLOD_BLOCK), Why::Chosen),
            under(LOD, LOD_TOWER, at(SLOD, SLOD_BLOCK)),
            under(LOD, LOD_ROW, at(SLOD, SLOD_BLOCK)),
            under(CITY, TOWER, at(LOD, LOD_TOWER)),
            under(CITY, HOUSE_A, at(LOD, LOD_ROW)),
            under(CITY, HOUSE_B, at(LOD, LOD_ROW)),
        ]
    );
    assert_eq!(planned.stays, Vec::new());
    assert_eq!(planned.files.len(), 3);
}

#[test]
fn every_entity_of_a_file_goes_and_the_inverses_still_put_every_file_back() {
    let all: Vec<At> = (0..=FENCE_END).map(|index| at(CITY, index)).collect();
    let planned = delete_emptying(&all, &[CITY, LOD]);
    assert_eq!(planned.ambiguous, Vec::new());
    // Every far model of `city_lod` is left with no child, and the two of `city_slod` they stood under after them.
    assert_eq!(
        change(&planned, LOD).map(|(removed, ..)| removed),
        Some(vec![LOD_TOWER, LOD_ROW, LOD_SHED, LOD_BARN])
    );
    assert_eq!(
        change(&planned, SLOD).map(|(removed, ..)| removed),
        Some(vec![SLOD_BLOCK, SLOD_FAR])
    );
    assert_eq!(planned.goes.len(), all.len() + 4 + 2);
}

#[test]
fn a_far_model_in_the_file_of_what_it_stands_for_is_counted_off_by_the_removal_itself() {
    let planned = delete(&[at(CITY, FENCE)]);
    assert_eq!(goes(&planned), [(at(CITY, FENCE), Why::Chosen)]);
    let [stays] = planned.stays.as_slice() else {
        panic!("the far model of the fence stays: {:?}", planned.stays);
    };
    assert_eq!(stays.entity.at, at(CITY, FENCE_FAR));
    assert_eq!(
        (stays.children_left, stays.counted, stays.counts),
        (1, 2, 1)
    );
    let lowered = Lowered {
        entity: FENCE_FAR,
        from: 2,
        to: 1,
    };
    assert_eq!(
        change(&planned, CITY),
        Some((vec![FENCE], vec![lowered], 0, 0))
    );
    // One operation: the removal lowers the count of a parent in its own file, and nothing states it a second time.
    assert_eq!(
        planned
            .operations
            .iter()
            .map(Operation::name)
            .collect::<Vec<_>>(),
        ["remove_entities"]
    );

    let planned = delete(&[at(CITY, FENCE_END), at(CITY, FENCE)]);
    assert_eq!(
        goes(&planned),
        [
            (at(CITY, FENCE_END), Why::Chosen),
            (at(CITY, FENCE), Why::Chosen),
            (at(CITY, FENCE_FAR), Why::LeftWithNoChild),
        ]
    );
    assert_eq!(planned.files.len(), 1);
}

#[test]
fn an_entity_that_stands_for_nothing_moves_the_links_of_the_map_below_which_loses_no_entity() {
    let planned = delete(&[at(SLOD, SLOD_SIGN)]);
    assert_eq!(goes(&planned), [(at(SLOD, SLOD_SIGN), Why::Chosen)]);
    assert_eq!(
        change(&planned, SLOD),
        Some((vec![SLOD_SIGN], vec![], 0, 0))
    );
    // The three far models of `city_lod` name entities of `city_slod` that each moved down by one, and `city` names none of them.
    assert_eq!(change(&planned, LOD), Some((vec![], vec![], 0, 3)));
    assert_eq!(change(&planned, CITY), None);
    assert_eq!(
        planned
            .operations
            .iter()
            .map(Operation::name)
            .collect::<Vec<_>>(),
        ["remove_entities", "renumber_for_removed"]
    );
}

#[test]
fn an_entity_that_is_not_there_is_refused_and_nothing_changes() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);
    let entities = FENCE_END + 1;

    let refused = delete::plan(&folder, &[at(CITY, TOWER), at(CITY, entities)]).unwrap_err();
    assert!(
        matches!(&refused, Error::NoSuchEntity { at, index, len }
            if at == CITY && *index == entities && *len == entities),
        "{refused:?}"
    );
    assert_eq!(
        refused.to_string(),
        format!("`{CITY}` holds {entities} entities, so there is no entity {entities}")
    );
    for (chosen, why) in [
        (at("harbour.metadata.src", 0), "no map file of the pack"),
        (at(TYPES, 0), "no map file of the pack"),
    ] {
        let refused = delete::plan(&folder, &[chosen]).unwrap_err();
        assert!(matches!(refused, Error::NoSuchSource(_)), "{refused:?}");
        assert!(refused.to_string().contains(why), "{refused}");
    }
    assert!(matches!(delete::plan(&folder, &[]), Err(Error::Nothing)));

    // The list of a removal that names an entity past the end is refused whole: the removal before it is not written.
    let list = [
        Operation::RemoveEntities {
            source: LOD.into(),
            expects: None,
            entities: vec![LOD_BARN],
            extents: None,
        },
        Operation::RemoveEntities {
            source: CITY.into(),
            expects: None,
            entities: vec![BARN, entities],
            extents: None,
        },
    ];
    let refused = apply_all(&folder, &list, false).unwrap_err().to_string();
    assert!(
        refused.starts_with(&format!(
            "operation 2 of 2, remove_entities of `{CITY}`: entity {entities} is past the {entities}"
        )),
        "{refused}"
    );
    assert!(files(&folder) == before, "nothing changed");
}

#[test]
fn two_entities_of_one_archetype_at_one_pose_are_ambiguous_and_left_unless_both_are_chosen() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);

    let planned = delete::plan(&folder, &[at(CITY, LAMP), at(CITY, BARN)]).unwrap();
    let [left] = planned.ambiguous.as_slice() else {
        panic!("one lamp is ambiguous: {:?}", planned.ambiguous);
    };
    assert_eq!(left.entity.at, at(CITY, LAMP));
    assert_eq!(left.twins, [at(CITY, LAMP_TWIN)]);
    assert_eq!(
        goes(&planned)
            .into_iter()
            .map(|(at, _)| at)
            .collect::<Vec<_>>(),
        [at(CITY, BARN), at(LOD, LOD_BARN)],
        "the lamp is left, and what else was chosen goes"
    );

    // The lamp alone: the plan removes nothing, and applying it writes nothing.
    let planned = delete::plan(&folder, &[at(CITY, LAMP_TWIN)]).unwrap();
    assert_eq!(planned.ambiguous.len(), 1);
    assert_eq!(planned.ambiguous[0].twins, [at(CITY, LAMP)]);
    assert_eq!(planned.goes, Vec::new());
    assert_eq!(planned.operations, Vec::new());
    assert_eq!(planned.files, Vec::new());
    assert_eq!(delete::apply(&folder, &planned).unwrap(), Vec::new());
    assert!(files(&folder) == before);

    // Both chosen, there is nothing to tell apart; and the third lamp stands elsewhere.
    let planned = delete(&[at(CITY, LAMP), at(CITY, LAMP_TWIN)]);
    assert_eq!(planned.ambiguous, Vec::new());
    assert_eq!(planned.goes.len(), 2);
    let planned = delete(&[at(CITY, LAMP_APART)]);
    assert_eq!(planned.ambiguous, Vec::new());
    assert_eq!(goes(&planned), [(at(CITY, LAMP_APART), Why::Chosen)]);
}

#[test]
fn the_plan_offers_the_script_line_that_hides_each_model_it_would_take_out() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let forms = ScriptForms::built_in().unwrap();

    let planned = delete::plan(&folder, &[at(CITY, TOWER)]).unwrap();
    let hidden: Vec<(String, [f32; 3])> = (planned.hidden.iter())
        .map(|hidden| (hidden.archetype.shown(), hidden.position))
        .collect();
    assert_eq!(
        hidden,
        [
            ("tower".to_string(), [10.0, 12.0, 3.0]),
            ("lod_tower".to_string(), [10.0, 10.0, 0.0]),
        ]
    );
    let lines = planned
        .script(forms.form("lua").unwrap(), HIDE_RADIUS, 2)
        .unwrap();
    let line = |name: &str, place: &str| {
        format!(
            "CreateModelHide({place}, 1.00, {}, true) -- {name}",
            or_io::string_hash(name.as_bytes())
        )
    };
    assert_eq!(
        lines,
        [
            line("tower", "10.00, 12.00, 3.00"),
            line("lod_tower", "10.00, 10.00, 0.00")
        ]
    );

    // Twins are one line, which hides both; a name the pack does not know is said by its hash.
    let planned = delete::plan(&folder, &[at(CITY, LAMP)]).unwrap();
    assert_eq!(planned.goes, Vec::new());
    let lines = planned.script(forms.form("lua").unwrap(), 0.5, 1).unwrap();
    let lamp = or_io::string_hash(b"lamp");
    assert_eq!(
        lines,
        [format!(
            "CreateModelHide(5.0, 5.0, 0.0, 0.5, {lamp}, true) -- hash_{lamp:08X}"
        )]
    );
    // Both lamps chosen, both go, and the one line still hides the two.
    let planned = delete::plan(&folder, &[at(CITY, LAMP), at(CITY, LAMP_TWIN)]).unwrap();
    assert_eq!(planned.goes.len(), 2);
    assert_eq!(
        planned.script(forms.form("lua").unwrap(), 0.5, 1).unwrap(),
        lines
    );
}

#[test]
fn a_far_model_in_a_map_that_is_not_among_the_sources_is_said_and_a_parent_that_counts_children_elsewhere_stays(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    // The shed's far model in `city_slod` is made to count three children, of which the sources hold one.
    let counts = Operation::SetNumChildren {
        source: SLOD.into(),
        expects: None,
        entity: SLOD_FAR,
        num_children: 3,
    };
    apply_all(&folder, &[counts], false).unwrap();
    let planned = delete::plan(&folder, &[at(CITY, SHED)]).unwrap();
    assert_eq!(
        goes(&planned),
        [
            (at(CITY, SHED), Why::Chosen),
            (at(LOD, LOD_SHED), Why::LeftWithNoChild),
        ]
    );
    let [stays] = planned.stays.as_slice() else {
        panic!("the far model of the far model stays: {:?}", planned.stays);
    };
    assert_eq!(stays.entity.at, at(SLOD, SLOD_FAR));
    assert_eq!((stays.children_left, stays.children_elsewhere), (0, 2));
    assert_eq!((stays.counted, stays.counts), (3, 2));
    assert_eq!(planned.notes, Vec::new());

    // The pack with `city_lod` alone: what one file says of a chain whose other files were not handed over.
    let mut model = Maps::read(&folder).unwrap();
    for set in &mut model.maps {
        set.sources.retain(|source| source == LOD);
    }
    model.write(&folder).unwrap();
    let before = files(&folder);
    let planned = delete::plan(&folder, &[at(LOD, LOD_TOWER)]).unwrap();
    assert_eq!(goes(&planned), [(at(LOD, LOD_TOWER), Why::Chosen)]);
    let slod = or_io::string_hash(b"city_slod");
    assert!(
        matches!(planned.notes.as_slice(), [
            Note::ParentMapNotAmongSources { entity, parent },
            Note::ChildrenNotAmongSources { entity: counting, counted: 1, found: 0 },
        ] if *entity == at(LOD, LOD_TOWER) && parent.hash == slod && *counting == at(LOD, LOD_TOWER)),
        "{:?}",
        planned.notes
    );
    assert_eq!(planned.stays, Vec::new());
    assert_eq!(planned.files.len(), 1);
    assert!(files(&folder) == before);
}

#[test]
fn a_source_of_another_kind_is_passed_over_and_one_that_leaves_the_pack_is_refused() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    // A source that is no metadata, beside the maps: the manifest of the type file under another kind and name.
    let manifest = std::fs::read_to_string(folder.join(TYPES).join("asset.json")).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    manifest["kind"] = "sound".into();
    manifest["name"] = "siren".into();
    let other = folder.join("siren.sound.src");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("asset.json"), manifest.to_string()).unwrap();
    let mut model = Maps::read(&folder).unwrap();
    model.maps[0].sources.push("siren.sound.src".into());
    model.write(&folder).unwrap();
    let planned =
        delete::plan(&folder, &[at(CITY, BARN)]).expect("a source of another kind is no map file");
    assert_eq!(planned.goes.len(), 2);
    let refused = delete::plan(&folder, &[at("siren.sound.src", 0)]).unwrap_err();
    assert!(matches!(refused, Error::NoSuchSource(_)), "{refused:?}");

    // A name that leaves the folder of the pack is not opened, whatever is there.
    let outside = scratch.path().join("outside.metadata.src");
    std::fs::create_dir(&outside).unwrap();
    for entry in std::fs::read_dir(folder.join(CITY)).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, outside.join(path.file_name().unwrap())).unwrap();
    }
    model.maps[0].sources.push("../outside.metadata.src".into());
    model.write(&folder).unwrap();
    let before = files(scratch.path());
    let refused = delete::plan(&folder, &[at(CITY, BARN)]).unwrap_err();
    assert!(
        matches!(&refused, Error::Source(source, why)
            if source == "../outside.metadata.src" && why.contains("leaves the folder of the pack")),
        "{refused:?}"
    );
    assert!(files(scratch.path()) == before);
}

#[test]
fn each_entity_of_the_pack_chosen_alone_leaves_a_pack_that_holds_to_every_check() {
    let all = (0..=FENCE_END)
        .map(|index| at(CITY, index))
        .chain((0..=LOD_BARN).map(|index| at(LOD, index)))
        .chain((0..=SLOD_FAR).map(|index| at(SLOD, index)));
    let (mut named, mut taken) = (0, 0);
    for chosen in all {
        named += 1;
        let planned = delete(std::slice::from_ref(&chosen));
        // The two lamps at one pose are left; any other entity goes, first of what its plan takes.
        let twin = chosen == at(CITY, LAMP) || chosen == at(CITY, LAMP_TWIN);
        assert_eq!(planned.ambiguous.len(), usize::from(twin), "{chosen:?}");
        let first = planned.goes.first().map(|going| &going.entity.at);
        assert_eq!(first, (!twin).then_some(&chosen), "{chosen:?}");
        taken += planned.goes.len();
    }
    assert!(
        taken > named,
        "some entity took its chain with it: {taken} of {named}"
    );
}

/// The LOD link and the count of children of each entity of the source `source`, in order.
fn links_of(folder: &Path, source: &str) -> Vec<(i32, u32)> {
    (0..)
        .map_while(|index| or_maps::edit::read_entity(&folder.join(source), index).ok())
        .map(|entity| (entity.parent_index, entity.num_children))
        .collect()
}

/// A pack of the map files `files`, each a file name and its bytes.
fn pack_of(dir: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let folder = dir.join("maps");
    let incoming: Vec<Incoming> = (files.iter())
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

#[test]
fn a_link_into_a_parent_map_leads_into_the_file_called_so_and_never_into_one_that_states_its_name()
{
    let extents = ([0.0; 3], [1000.0; 3]);
    // `depot.ymap` states the name `quay`, as a file renamed without its name does: it is the map `depot` all the same.
    let depot = || {
        let hut = Entity::at("prop_depot_hut", [500.0, 500.0, 0.5], 71);
        let shed = Entity::at("prop_depot_shed", [520.0, 500.0, 0.5], 72);
        map_file("quay", &[hut, shed], extents)
    };
    let quay_hd = || {
        let crane = under("prop_crane", [10.0, 20.0, 0.5], 21, 0);
        child_map_file("quay_hd", "quay", &[crane], extents)
    };
    let crane = [at("quay_hd.metadata.src", 0)];

    // The parent map is not in the pack: the crane goes alone, the plan says where its far model is, and the depot keeps its two.
    let scratch = tempfile::tempdir().unwrap();
    let files = [("depot.ymap", depot()), ("quay_hd.ymap", quay_hd())];
    let folder = pack_of(scratch.path(), &files);
    let planned = delete::plan(&folder, &crane).unwrap();
    assert_eq!(goes(&planned), [(crane[0].clone(), Why::Chosen)]);
    assert!(
        matches!(
            planned.notes.as_slice(),
            [Note::ParentMapNotAmongSources { parent, .. }] if parent.hash == or_io::string_hash(b"quay")
        ),
        "{:?}",
        planned.notes
    );
    delete::apply(&folder, &planned).unwrap();
    assert_eq!(links_of(&folder, "depot.metadata.src"), [(-1, 0), (-1, 0)]);

    // The parent map is in the pack beside it: the far model in `quay.ymap` goes with the crane, and no file is said to split a map.
    let scratch = tempfile::tempdir().unwrap();
    let far = Entity {
        children: 1,
        ..Entity::at("prop_crane_far", [10.0, 20.0, 0.5], 1)
    };
    let quay = map_file("quay", &[far], extents);
    let files = [
        ("depot.ymap", depot()),
        ("quay.ymap", quay),
        ("quay_hd.ymap", quay_hd()),
    ];
    let folder = pack_of(scratch.path(), &files);
    let planned = delete::plan(&folder, &crane).unwrap();
    assert_eq!(
        goes(&planned),
        [
            (crane[0].clone(), Why::Chosen),
            (at("quay.metadata.src", 0), Why::LeftWithNoChild)
        ]
    );
    assert_eq!(planned.notes, Vec::new());
    delete::apply(&folder, &planned).unwrap();
    assert_eq!(links_of(&folder, "depot.metadata.src"), [(-1, 0), (-1, 0)]);
    assert_eq!(links_of(&folder, "quay.metadata.src"), []);
}

#[test]
fn an_entity_taken_out_of_a_map_renumbers_the_links_of_the_maps_under_its_file_and_of_no_other() {
    let extents = ([0.0; 3], [100.0; 3]);
    let far = |archetype: &'static str, x: f32, guid: u32, children: u32| Entity {
        children,
        ..Entity::at(archetype, [x, 10.0, 0.0], guid)
    };
    // `f_lod.ymap` states the name `x_lod`: the maps under it are those that name `f_lod`, and `x` is under another file.
    let f_lod = map_file(
        "x_lod",
        &[
            far("f_solo", 1.0, 10, 0),
            far("f_far_a", 10.0, 11, 1),
            far("f_far_b", 20.0, 12, 1),
        ],
        extents,
    );
    let houses = [
        under("house_a", [10.0, 10.0, 0.0], 21, 1),
        under("house_b", [20.0, 10.0, 0.0], 22, 2),
    ];
    let shops = [
        under("shop_a", [60.0, 60.0, 0.0], 41, 1),
        under("shop_b", [70.0, 60.0, 0.0], 42, 2),
    ];
    let scratch = tempfile::tempdir().unwrap();
    let files = [
        ("f_lod.ymap", f_lod),
        ("f.ymap", child_map_file("f", "f_lod", &houses, extents)),
        ("x.ymap", child_map_file("x", "x_lod", &shops, extents)),
    ];
    let folder = pack_of(scratch.path(), &files);
    let planned = delete::plan(&folder, &[at("f_lod.metadata.src", 0)]).unwrap();
    assert_eq!(planned.goes.len(), 1);
    delete::apply(&folder, &planned).unwrap();
    assert_eq!(
        links_of(&folder, "f.metadata.src"),
        [(0, 0), (1, 0)],
        "the houses name the far models they named"
    );
    assert_eq!(
        links_of(&folder, "x.metadata.src"),
        [(1, 0), (2, 0)],
        "the shops name entities of `x_lod.ymap`, which lost nothing"
    );
    // The rules name the file that states another name than its own.
    let model = Maps::read(&folder).unwrap();
    let facts = or_maps::inspect(&folder, &model, or_maps::Depth::Open);
    let found = or_maps::rules::check(&model, &facts, None);
    let named: Vec<&str> = (found.iter())
        .filter(|finding| finding.code == "map-named-otherwise")
        .filter_map(|finding| finding.place.source.as_deref())
        .collect();
    assert_eq!(named, ["f_lod.metadata.src"]);
    assert!(
        found
            .iter()
            .all(|f| f.code != "lod-children-uncounted" && f.code != "parent-link-broken"),
        "the links and the counts of the pack hold: {found:?}"
    );
}
