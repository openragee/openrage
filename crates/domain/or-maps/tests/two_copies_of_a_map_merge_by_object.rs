//! Two copies of one map merge by object against the file both were made from: a change one side made is taken, a change both
//! made to one value is one change, and the rest is a conflict the policy decides, with the links, the counts and the guids of the
//! merged map right.
//!
//! ⚠ The files are the crate's samples and the edits are `or_maps::edit`'s, so the clean checkout runs this.

use std::collections::BTreeMap;

use glam::Vec3;
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CEntityDef, CEntityDefAny, CMapData, CMapTypes, CMloInstanceDef};
use or_io::meta::{
    build_library, canonical, detect_binary_format, write_meta_binary, BinaryFormat, MetaDocument,
    MetaInstance, Value,
};
use or_maps::changes::{map_changes, EntityChange, SourceChanges};
use or_maps::edit::{add_entity, place_entity, pose_of, remove_entity, Pose};
use or_maps::facts::Name;
use or_maps::import::{add_files, Incoming};
use or_maps::merge::{
    merge_files, merge_maps, merge_source, merge_types, By, Conflict, Did, Disagreement, Error,
    Merge, Ref, Side, Take,
};
use or_maps::sample::{map_file, types_file, Archetype, Entity};

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

fn moved(map: &mut CMapData, index: usize, to: [f32; 3]) {
    let pose = pose_of(&map.entities[index]);
    let position = Vec3::from_array(to);
    place_entity(map, index, Pose { position, ..pose }).unwrap();
}

fn added(map: &mut CMapData, archetype: &str, at: [f32; 3]) -> usize {
    let donor = map.entities[2].clone();
    let pose = Pose {
        position: Vec3::from_array(at),
        ..pose_of(&donor)
    };
    add_entity(map, &donor, Name::from_text(archetype), pose).unwrap()
}

fn position_of(map: &CMapData, guid: u32) -> [f32; 3] {
    let found: Vec<_> = map.entities.iter().filter(|e| e.guid == guid).collect();
    assert_eq!(found.len(), 1, "one entity states the guid {guid}");
    [
        found[0].position.x,
        found[0].position.y,
        found[0].position.z,
    ]
}

fn merged(base: &CMapData, ours: &CMapData, theirs: &CMapData, take: Take) -> Merge<CMapData> {
    merge_maps(base, ours, theirs, &mut |_| take).unwrap()
}

fn indexes(refs: &[Ref]) -> Vec<usize> {
    refs.iter().map(Ref::index).collect()
}

/// The guid of the parent each entity of `map` names, by its own guid; the maps here state each guid once.
fn parents(map: &CMapData) -> BTreeMap<u32, Option<u32>> {
    (map.entities.iter())
        .map(|entity| {
            let parent = usize::try_from(entity.parent_index)
                .ok()
                .map(|parent| map.entities[parent].guid);
            (entity.guid, parent)
        })
        .collect()
}

/// What `or_world::validate` finds in `map` as it saves.
fn problems(map: &CMapData) -> Vec<String> {
    let bytes = map.to_bytes().expect("the merged map saves");
    let document = MetaDocument::parse(or_io::meta::default_library(), bytes).expect("reads back");
    let view = or_world::map_data::MapData::new(&document.instance).expect("a CMapData");
    (or_world::validate::check(&view).iter())
        .map(ToString::to_string)
        .collect()
}

#[test]
fn a_move_by_each_side_of_another_entity_is_both_moves_and_no_conflict() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    moved(&mut ours, 2, [22.0, 21.0, 0.5]);
    moved(&mut theirs, 5, [31.0, 15.0, 0.0]);

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    assert!(merge.report.merged);
    let map = merge.merged.expect("no conflict stopped it");
    assert_eq!(position_of(&map, 3), [22.0, 21.0, 0.5]);
    assert_eq!(position_of(&map, 6), [31.0, 15.0, 0.0]);
    assert_eq!(map.entities.len(), base.entities.len());
    assert_eq!(parents(&map), parents(&base));

    // What each side changed is what the comparison says, and each change is said with the side it is from.
    assert_eq!(
        merge.report.ours,
        SourceChanges::Map(map_changes(&base, &ours))
    );
    assert_eq!(
        merge.report.theirs,
        SourceChanges::Map(map_changes(&base, &theirs))
    );
    let said: Vec<(Did, By, Option<usize>, Option<usize>)> = (merge.report.applied.iter())
        .map(|a| (a.did, a.by, a.base.as_ref().map(Ref::index), a.merged))
        .collect();
    assert_eq!(
        said,
        [
            (Did::Changed, By::Ours, Some(2), Some(2)),
            (Did::Changed, By::Theirs, Some(5), Some(5)),
        ]
    );
    assert_eq!(merge.report.applied[0].members[0].member, "position");
}

#[test]
fn both_moving_one_entity_to_different_places_is_one_conflict_and_each_policy_gives_that_sides_place(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    moved(&mut ours, 2, [22.0, 21.0, 0.5]);
    moved(&mut theirs, 2, [18.0, 19.0, 0.5]);
    // Each also moved an entity the other left alone, which no conflict holds back.
    moved(&mut theirs, 6, [10.0, 16.0, 0.0]);

    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert!(
        stopped.merged.is_none(),
        "a conflict that stops makes no map"
    );
    assert!(!stopped.report.merged);
    assert_eq!(stopped.report.stopped(), 1);
    assert_eq!(stopped.report.conflicts.len(), 1);
    let conflict = &stopped.report.conflicts[0];
    assert_eq!(
        conflict.what,
        Disagreement::Member {
            member: "position".to_string(),
            ours: "(22, 21, 0.5)".to_string(),
            theirs: "(18, 19, 0.5)".to_string(),
        }
    );
    assert_eq!(
        (
            indexes(&conflict.base),
            indexes(&conflict.ours),
            indexes(&conflict.theirs)
        ),
        (vec![2], vec![2], vec![2]),
        "the entity as each file indexes it"
    );
    assert_eq!(conflict.taken, Take::Stop);

    for (take, place) in [
        (Take::Ours, [22.0, 21.0, 0.5]),
        (Take::Theirs, [18.0, 19.0, 0.5]),
    ] {
        let merge = merged(&base, &ours, &theirs, take);
        assert_eq!(merge.report.conflicts.len(), 1);
        assert_eq!(merge.report.conflicts[0].taken, take, "the report says it");
        let map = merge.merged.expect("the policy decided");
        assert_eq!(position_of(&map, 3), place);
        assert_eq!(position_of(&map, 7), [10.0, 16.0, 0.0]);
    }
}

#[test]
fn both_moving_one_entity_to_one_place_is_one_change_and_no_conflict() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    moved(&mut ours, 2, [22.0, 21.0, 0.5]);
    moved(&mut theirs, 2, [22.0, 21.0, 0.5]);
    // Theirs changed another member of it too, which is taken beside the move both made.
    theirs.entities[2].lod_dist = 250.0;

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(position_of(&map, 3), [22.0, 21.0, 0.5]);
    assert_eq!(map.entities[2].lod_dist, 250.0);
    assert_eq!(merge.report.applied.len(), 1, "one change");
    let members: Vec<(&str, By)> = (merge.report.applied[0].members.iter())
        .map(|m| (m.member.as_str(), m.by))
        .collect();
    assert_eq!(members, [("position", By::Both), ("lodDist", By::Theirs)]);
}

#[test]
fn a_member_each_side_changed_of_one_entity_is_both_changes() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    moved(&mut ours, 2, [22.0, 21.0, 0.5]);
    theirs.entities[2].tint_value = 4;
    theirs.entities[2].flags |= 1;

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(position_of(&map, 3), [22.0, 21.0, 0.5]);
    assert_eq!(map.entities[2].tint_value, 4);
    assert_eq!(map.entities[2].flags, base.entities[2].flags | 1);
}

#[test]
fn what_one_side_deleted_and_the_other_moved_is_a_conflict() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    remove_entity(&mut ours, 2).unwrap();
    moved(&mut theirs, 2, [18.0, 19.0, 0.5]);

    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert!(stopped.merged.is_none());
    assert_eq!(stopped.report.conflicts.len(), 1);
    let conflict = &stopped.report.conflicts[0];
    assert_eq!(
        conflict.what,
        Disagreement::ChangedAndDeleted {
            deleted_by: Side::Ours,
            changed: vec!["position".to_string()],
        }
    );
    assert_eq!(
        (
            indexes(&conflict.base),
            indexes(&conflict.ours),
            indexes(&conflict.theirs)
        ),
        (vec![2], vec![], vec![2])
    );

    let gone = merged(&base, &ours, &theirs, Take::Ours).merged.unwrap();
    assert!(gone.entities.iter().all(|e| e.guid != 3), "ours deleted it");
    assert_eq!(gone.entities.len(), base.entities.len() - 1);
    assert_eq!(problems(&gone), Vec::<String>::new());
    let kept = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(position_of(&kept, 3), [18.0, 19.0, 0.5]);
    assert_eq!(kept.entities.len(), base.entities.len());

    // Deleted by both, it is one deletion.
    let mut also = base.clone();
    remove_entity(&mut also, 2).unwrap();
    let merge = merged(&base, &ours, &also, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    assert_eq!(merge.report.applied.len(), 1);
    assert_eq!(
        (merge.report.applied[0].did, merge.report.applied[0].by),
        (Did::Deleted, By::Both)
    );
}

#[test]
fn an_entity_each_side_added_is_in_the_result_with_a_guid_of_its_own_that_the_base_does_not_state()
{
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    let at_ours = added(&mut ours, "prop_bench", [40.0, 40.0, 0.0]);
    let at_theirs = added(&mut theirs, "prop_tree", [60.0, 60.0, 0.0]);
    assert_eq!(
        ours.entities[at_ours].guid, theirs.entities[at_theirs].guid,
        "each side gave its entity one more than the largest guid of the base"
    );

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len() + 2);
    let of = |archetype: &str| -> u32 {
        let found: Vec<_> = (map.entities.iter())
            .filter(|e| e.archetype_name == archetype)
            .collect();
        assert_eq!(found.len(), 1, "{archetype} is in the merged map once");
        found[0].guid
    };
    let (bench, tree) = (of("prop_bench"), of("prop_tree"));
    assert_ne!(bench, tree, "each has a guid of its own");
    for guid in [bench, tree] {
        assert!(
            base.entities.iter().all(|e| e.guid != guid),
            "{guid} is no guid of the base"
        );
    }
    let mut guids: Vec<u32> = map.entities.iter().map(|e| e.guid).collect();
    guids.sort_unstable();
    guids.dedup();
    assert_eq!(guids.len(), map.entities.len(), "no guid is stated twice");
    let said: Vec<(Did, By, Option<u32>)> = (merge.report.applied.iter())
        .map(|a| (a.did, a.by, a.guid))
        .collect();
    assert!(
        said.contains(&(Did::Added, By::Ours, Some(bench))),
        "{said:?}"
    );
    assert!(
        said.contains(&(Did::Added, By::Theirs, Some(tree))),
        "{said:?}"
    );

    // An entity only one side added keeps the guid that side gave it, when nothing else states it.
    let alone = merged(&base, &ours, &base.clone(), Take::Stop)
        .merged
        .unwrap();
    assert_eq!(
        alone.entities[at_ours].guid, ours.entities[at_ours].guid,
        "and the merged map is ours"
    );
}

#[test]
fn an_added_entity_never_keeps_a_guid_the_base_states() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours added a copy of the crate that states the guid of the crate, as a copy made by hand does.
    let copy = added(&mut ours, "prop_crate_a", [40.0, 40.0, 0.5]);
    ours.entities[copy].guid = 3;
    // Ours deleted the last entity, and theirs added a bench that states the guid of that entity.
    remove_entity(&mut ours, 6).unwrap();
    let bench = added(&mut theirs, "prop_bench", [60.0, 60.0, 0.0]);
    theirs.entities[bench].guid = 7;
    assert_eq!(map_changes(&base, &ours).added.len(), 1);
    assert_eq!(map_changes(&base, &theirs).added.len(), 1);

    let map = merged(&base, &ours, &theirs, Take::Stop).merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len() + 1);
    assert!(
        map.entities.iter().all(|e| e.guid != 7),
        "the guid of the entity ours deleted is free in the merged map, and the base states it"
    );
    let crates: Vec<u32> = (map.entities.iter())
        .filter(|e| e.archetype_name == "prop_crate_a")
        .map(|e| e.guid)
        .collect();
    assert_eq!(crates.len(), 2);
    assert_eq!(crates[0], 3, "the crate of the base keeps its guid");
    for entity in &map.entities[base.entities.len() - 1..] {
        assert!(
            base.entities.iter().all(|e| e.guid != entity.guid),
            "an added entity states {}, a guid of the base",
            entity.guid
        );
    }
}

#[test]
fn deletions_by_both_sides_from_the_middle_leave_links_and_counts_that_validate_and_each_entity_its_own_parent(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours takes out the crate and a child of the second parent; theirs takes out a child of the first parent.
    remove_entity(&mut ours, 2).unwrap();
    remove_entity(&mut ours, 3).unwrap();
    remove_entity(&mut theirs, 1).unwrap();
    assert_ne!(
        (ours.entities.iter().map(|e| e.parent_index)).collect::<Vec<_>>(),
        (theirs.entities.iter().map(|e| e.parent_index)).collect::<Vec<_>>(),
        "every later index shifted differently on each side"
    );
    // And each added a child to a parent, which the parent counts.
    let child = added(&mut theirs, "yard_hd_b", [30.0, 16.0, 0.0]);
    theirs.entities[child].parent_index = 2;
    theirs.entities[2].num_children += 1;

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(problems(&map), Vec::<String>::new());

    let guids: Vec<u32> = map.entities.iter().map(|e| e.guid).collect();
    let new = theirs.entities[child].guid;
    assert_eq!(
        guids,
        [1, 4, 6, 7, new],
        "the entities that are left, in the order of the base"
    );
    let was = parents(&base);
    for (guid, parent) in parents(&map) {
        let expected = match guid {
            guid if guid == new => Some(4),
            guid => was[&guid],
        };
        assert_eq!(
            parent, expected,
            "the parent of the entity with guid {guid}"
        );
    }
    let counted = |guid: u32| -> (u32, usize) {
        let at = map.entities.iter().position(|e| e.guid == guid).unwrap();
        let named = (map.entities.iter())
            .filter(|e| usize::try_from(e.parent_index) == Ok(at))
            .count();
        (map.entities[at].num_children, named)
    };
    assert_eq!(counted(1), (1, 1), "one child of the first parent is left");
    assert_eq!(counted(4), (2, 2), "one went and one came");
    assert_eq!(counted(6), (0, 0));
}

#[test]
fn a_count_of_children_keeps_what_a_child_map_adds_to_it() {
    // The first parent counts five children, two of this file and three of maps that name this one as their parent.
    let mut yard = a_yard();
    yard[0].children = 5;
    let base = map(&yard);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    remove_entity(&mut ours, 1).unwrap();
    remove_entity(&mut theirs, 6).unwrap();
    assert_eq!(
        (
            ours.entities[0].num_children,
            theirs.entities[0].num_children
        ),
        (4, 4)
    );

    let map = merged(&base, &ours, &theirs, Take::Stop).merged.unwrap();
    assert_eq!(
        map.entities[0].num_children, 3,
        "both children of this file went"
    );
}

#[test]
fn twins_one_side_moved_are_a_conflict_that_names_them_and_nothing_is_merged_for_them() {
    let twins = |second: [f32; 3]| {
        map(&[
            Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 9),
            Entity::at("prop_tree", [10.0, 20.0, 0.0], 7),
            Entity::at("prop_tree", second, 7),
        ])
    };
    let base = twins([14.0, 20.0, 0.0]);
    let ours = twins([14.0, 23.0, 0.0]);
    let mut theirs = base.clone();
    moved(&mut theirs, 0, [2.0, 2.0, 0.0]);

    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert!(stopped.merged.is_none());
    assert_eq!(stopped.report.conflicts.len(), 1);
    let conflict = &stopped.report.conflicts[0];
    assert_eq!(conflict.what, Disagreement::Ambiguous);
    assert_eq!(
        (
            indexes(&conflict.base),
            indexes(&conflict.ours),
            indexes(&conflict.theirs)
        ),
        (vec![2], vec![2], vec![2]),
        "the twin as each file indexes it"
    );
    let Ref::Entity(named) = &conflict.ours[0] else {
        panic!("an entity");
    };
    assert_eq!(named.position, [14.0, 23.0, 0.0]);
    assert!(
        stopped
            .report
            .applied
            .iter()
            .all(|a| a.base.as_ref().map(Ref::index) != Some(2)),
        "nothing is said to be merged for the twin"
    );

    // Theirs keeps the twin where the base has it, and ours has it where ours moved it; the crate theirs moved is moved in both.
    let kept = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(
        [kept.entities[2].position.x, kept.entities[2].position.y],
        [14.0, 20.0]
    );
    let taken = merged(&base, &ours, &theirs, Take::Ours).merged.unwrap();
    assert_eq!(
        [taken.entities[2].position.x, taken.entities[2].position.y],
        [14.0, 23.0]
    );
    for map in [&kept, &taken] {
        assert_eq!(map.entities.len(), 3);
        assert_eq!(position_of(map, 9), [2.0, 2.0, 0.0]);
    }

    // The twins are a conflict though the other side is the base itself: nothing says which twin moved.
    let alone = merged(&base, &ours, &base.clone(), Take::Stop);
    assert!(alone.merged.is_none());
    assert_eq!(alone.report.conflicts.len(), 1);
    let whole = merged(&base, &ours, &base.clone(), Take::Ours)
        .merged
        .unwrap();
    assert_eq!(whole.to_bytes().unwrap(), ours.to_bytes().unwrap());
}

#[test]
fn an_entity_one_side_gave_another_archetype_is_never_merged_member_by_member() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours took the last entity out and put a bench in, which took its guid: the comparison reads one entity changed.
    remove_entity(&mut ours, 6).unwrap();
    let at = added(&mut ours, "prop_bench", [40.0, 40.0, 0.0]);
    assert_eq!(ours.entities[at].guid, 7);
    theirs.entities[6].lod_dist = 250.0;

    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert!(stopped.merged.is_none());
    assert_eq!(stopped.report.conflicts.len(), 1);
    assert_eq!(
        stopped.report.conflicts[0].what,
        Disagreement::Record {
            ours: ["archetypeName", "position", "parentIndex"]
                .map(String::from)
                .to_vec(),
            theirs: vec!["lodDist".to_string()],
        },
        "what each side changed of it"
    );
    let bench = merged(&base, &ours, &theirs, Take::Ours).merged.unwrap();
    let held = &bench.entities[6];
    assert!(held.archetype_name == "prop_bench");
    assert_eq!(
        held.lod_dist, ours.entities[at].lod_dist,
        "the bench does not take the distance theirs gave the entity that was there"
    );
    let theirs_kept = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert!(theirs_kept.entities[6].archetype_name == "yard_hd_a");
    assert_eq!(theirs_kept.entities[6].lod_dist, 250.0);
}

#[test]
fn an_entity_both_added_at_one_place_is_a_likely_double_kept_once_only_when_every_member_agrees() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    added(&mut ours, "prop_bench", [40.0, 40.0, 0.0]);
    added(&mut theirs, "prop_bench", [40.0, 40.0, 0.0]);
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len() + 1, "kept once");
    assert_eq!(
        (merge.report.applied[0].did, merge.report.applied[0].by),
        (Did::Added, By::Both)
    );

    // One member apart, the two are a conflict that says so, and a side's is kept.
    let last = theirs.entities.len() - 1;
    theirs.entities[last].lod_dist = 250.0;
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert!(stopped.merged.is_none());
    assert_eq!(stopped.report.conflicts.len(), 1);
    let conflict = &stopped.report.conflicts[0];
    assert_eq!(
        conflict.what,
        Disagreement::Double {
            differ: vec!["lodDist".to_string()]
        },
        "with the member the two differ in"
    );
    assert_eq!(
        (
            indexes(&conflict.base),
            indexes(&conflict.ours),
            indexes(&conflict.theirs)
        ),
        (vec![], vec![7], vec![7])
    );
    let map = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len() + 1);
    assert_eq!(map.entities[7].lod_dist, 250.0);
}

/// Ours and theirs with a move each, a deletion each, an addition each and one entity both moved.
fn two_sides() -> (CMapData, CMapData, CMapData) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    moved(&mut ours, 2, [22.0, 21.0, 0.5]);
    moved(&mut theirs, 2, [18.0, 19.0, 0.5]);
    moved(&mut ours, 4, [31.0, 12.5, 0.0]);
    moved(&mut theirs, 5, [31.0, 15.0, 0.0]);
    added(&mut ours, "prop_bench", [40.0, 40.0, 0.0]);
    added(&mut theirs, "prop_tree", [60.0, 60.0, 0.0]);
    added(&mut theirs, "prop_bench", [45.0, 45.0, 0.0]);
    remove_entity(&mut ours, 6).unwrap();
    remove_entity(&mut theirs, 1).unwrap();
    (base, ours, theirs)
}

#[test]
fn merging_in_the_other_order_gives_the_same_map_under_the_mirrored_policy() {
    let (base, ours, theirs) = two_sides();
    for (take, mirrored) in [(Take::Ours, Take::Theirs), (Take::Theirs, Take::Ours)] {
        let one = merged(&base, &ours, &theirs, take);
        let other = merged(&base, &theirs, &ours, mirrored);
        assert_eq!(one.report.conflicts.len(), 1);
        assert_eq!(other.report.conflicts.len(), 1);
        let (one, other) = (one.merged.unwrap(), other.merged.unwrap());
        assert_eq!(one, other, "taking {take:?}");
        assert_eq!(one.to_bytes().unwrap(), other.to_bytes().unwrap());
        assert_eq!(problems(&one), Vec::<String>::new());
    }
    // The two policies do differ: the entity both moved stands where the side taken put it.
    let (a, b) = (
        merged(&base, &ours, &theirs, Take::Ours).merged.unwrap(),
        merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap(),
    );
    assert_eq!(position_of(&a, 3), [22.0, 21.0, 0.5]);
    assert_eq!(position_of(&b, 3), [18.0, 19.0, 0.5]);
}

#[test]
fn merging_a_side_with_the_base_itself_gives_the_other_side_byte_for_byte() {
    let (base, mut ours, theirs) = two_sides();
    // An entity outside the extents the map states, and one put in the middle of the file: the merge states neither anew.
    moved(&mut ours, 0, [500.0, 10.0, 0.0]);
    let middle = ours.entities[1].clone();
    ours.entities.insert(1, middle);
    ours.entities[1].guid = 90;
    ours.entities[1].position.x = 77.0;
    for entity in ours.entities.iter_mut().skip(2) {
        if entity.parent_index >= 1 {
            entity.parent_index += 1;
        }
    }
    for side in [&ours, &theirs] {
        let saved = side.to_bytes().unwrap();
        let merge = merged(&base, side, &base.clone(), Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        assert_eq!(merge.merged.unwrap().to_bytes().unwrap(), saved);
        let merge = merged(&base, &base.clone(), side, Take::Stop);
        assert_eq!(merge.merged.unwrap().to_bytes().unwrap(), saved);
        // Each change is said with its index in that side.
        let changes = map_changes(&base, side);
        let at: Vec<Option<usize>> = (merge.report.applied.iter())
            .filter(|a| a.did == Did::Added)
            .map(|a| a.merged)
            .collect();
        let mut expected: Vec<Option<usize>> =
            changes.added.iter().map(|e| Some(e.index)).collect();
        let mut at_sorted = at.clone();
        at_sorted.sort();
        expected.sort();
        assert_eq!(at_sorted, expected);
    }
    // A map merged with itself twice is itself.
    let merge = merged(&base, &base.clone(), &base.clone(), Take::Stop);
    assert_eq!(
        merge.merged.unwrap().to_bytes().unwrap(),
        base.to_bytes().unwrap()
    );
    assert!(merge.report.applied.is_empty());
}

#[test]
fn the_extents_of_the_merged_map_hold_what_each_side_placed() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    moved(&mut ours, 2, [-50.0, 20.0, 0.5]);
    moved(&mut theirs, 5, [30.0, 400.0, 0.0]);
    // Theirs states extents that hold what it moved, as an edit grows them; ours states the ones of the base.
    theirs.entities_extents_max.y = 410.0;
    theirs.streaming_extents_max.y = 600.0;

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    let map = merge.merged.unwrap();
    assert_eq!(
        map.entities_extents_min.x, -50.0,
        "grown to hold the entity ours moved"
    );
    assert!(map.streaming_extents_min.x <= -50.0 - base.entities[2].lod_dist);
    assert_eq!(
        map.entities_extents_max.y, 410.0,
        "what theirs states is kept"
    );
    assert_eq!(map.streaming_extents_max.y, 600.0);
    assert_eq!(map.entities_extents_max.x, 100.0, "and nothing shrank");
    assert!(
        merge
            .report
            .findings
            .iter()
            .all(|f| f.code != "extents-short"),
        "{:?}",
        merge.report.findings
    );
    assert!(merge
        .report
        .not_compared
        .iter()
        .any(|said| said.contains("extents")));
}

#[test]
fn what_the_file_states_of_itself_is_taken_from_the_side_that_changed_it_and_is_a_conflict_when_both_did(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours.flags = 2;
    theirs.parent = HashedString::new("harbour_lod");
    moved(&mut theirs, 2, [18.0, 19.0, 0.5]);
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(map.flags, 2);
    assert!(map.parent == "harbour_lod");
    let said: Vec<(&str, By)> = (merge.report.file.iter())
        .map(|m| (m.member.as_str(), m.by))
        .collect();
    assert_eq!(said, [("parent", By::Theirs), ("flags", By::Ours)]);

    theirs.flags = 4;
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert!(stopped.merged.is_none());
    assert_eq!(
        stopped.report.conflicts[0].what,
        Disagreement::File {
            member: "flags".to_string(),
            ours: "2".to_string(),
            theirs: "4".to_string(),
        }
    );
    let map = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(map.flags, 4);
    // The report says what it did not look into.
    let said = stopped.report.not_compared.join("\n");
    for word in ["no typed member names", "flags", "extents"] {
        assert!(said.contains(word), "{said}");
    }
}

#[test]
fn a_merge_that_cannot_be_made_valid_is_refused_with_the_reason_and_returns_nothing() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours makes the crate a child of the last entity, and theirs makes the last entity a child of the crate: a cycle.
    ours.entities[2].parent_index = 6;
    theirs.entities[6].parent_index = 2;
    let refused = merge_maps(&base, &ours, &theirs, &mut |_| Take::Ours).unwrap_err();
    assert!(matches!(refused, Error::Invalid(_)), "{refused}");
    assert!(refused.to_string().contains("cycle"), "{refused}");

    // A side whose own links are broken is named.
    let mut broken = base.clone();
    broken.entities[2].parent_index = 60;
    let refused = merge_maps(&base, &ours, &broken, &mut |_| Take::Ours).unwrap_err();
    assert!(
        matches!(
            &refused,
            Error::Input {
                which: "theirs",
                ..
            }
        ),
        "{refused}"
    );
}

#[test]
fn a_policy_decides_each_conflict_by_itself_and_the_report_says_what_it_took_of_each() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    moved(&mut ours, 2, [22.0, 21.0, 0.5]);
    moved(&mut theirs, 2, [18.0, 19.0, 0.5]);
    moved(&mut ours, 5, [31.0, 15.0, 0.0]);
    moved(&mut theirs, 5, [29.0, 15.0, 0.0]);
    let mut asked = Vec::new();
    let mut policy = |conflict: &Conflict| {
        asked.push(indexes(&conflict.base));
        match conflict.base[0].index() {
            2 => Take::Ours,
            _ => Take::Theirs,
        }
    };
    let merge = merge_maps(&base, &ours, &theirs, &mut policy).unwrap();
    assert_eq!(asked, [vec![2], vec![5]], "each conflict is asked once");
    let taken: Vec<Take> = merge.report.conflicts.iter().map(|c| c.taken).collect();
    assert_eq!(taken, [Take::Ours, Take::Theirs]);
    let map = merge.merged.unwrap();
    assert_eq!(position_of(&map, 3), [22.0, 21.0, 0.5]);
    assert_eq!(position_of(&map, 6), [29.0, 15.0, 0.0]);

    // One conflict answered with stop makes no map, and the others are still asked and listed.
    let mut policy = |conflict: &Conflict| match conflict.base[0].index() {
        2 => Take::Stop,
        _ => Take::Theirs,
    };
    let merge = merge_maps(&base, &ours, &theirs, &mut policy).unwrap();
    assert!(merge.merged.is_none());
    assert_eq!(merge.report.stopped(), 1);
    assert_eq!(merge.report.conflicts.len(), 2);
}

#[test]
fn a_parent_each_side_gave_one_entity_is_a_conflict_and_the_counts_follow_the_side_taken() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // The crate, which has no parent, is made a child of the first parent by ours and of the second by theirs.
    for (side, parent) in [(&mut ours, 0), (&mut theirs, 3)] {
        side.entities[2].parent_index = parent as i32;
        side.entities[parent].num_children += 1;
    }
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(
        stopped.report.conflicts.len(),
        1,
        "{:?}",
        stopped.report.conflicts
    );
    assert_eq!(
        stopped.report.conflicts[0].what,
        Disagreement::Member {
            member: "parentIndex".to_string(),
            ours: format!(
                "entity 0 of the base ({} at (10, 10, 0), guid 1)",
                Name::of(Name::from_text("yard_lod_a").hash).shown()
            ),
            theirs: format!(
                "entity 3 of the base ({} at (30, 10, 0), guid 4)",
                Name::of(Name::from_text("yard_lod_b").hash).shown()
            ),
        }
    );
    for (take, parent, counts) in [(Take::Ours, 1, (3, 2)), (Take::Theirs, 4, (2, 3))] {
        let map = merged(&base, &ours, &theirs, take).merged.unwrap();
        assert_eq!(parents(&map)[&3], Some(parent));
        assert_eq!(
            (map.entities[0].num_children, map.entities[3].num_children),
            counts,
            "taking {take:?}"
        );
        assert_eq!(problems(&map), Vec::<String>::new());
    }

    // Both giving it one parent is one change.
    let merge = merged(&base, &ours, &ours.clone(), Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(parents(&map)[&3], Some(1));
    assert_eq!(map.entities[0].num_children, 3);
}

#[test]
fn an_entity_one_side_named_as_a_parent_and_the_other_deleted_leaves_no_link_and_a_note() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours makes the crate a child of the last entity, and gives the last entity a child to count; theirs deletes the last entity.
    ours.entities[2].parent_index = 6;
    ours.entities[6].num_children = 1;
    remove_entity(&mut theirs, 6).unwrap();
    // A count that follows the children is no change of the last entity, so its deletion is no conflict: it goes.
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len() - 1);
    assert_eq!(parents(&map)[&3], None, "the crate names no parent");
    assert_eq!(problems(&map), Vec::<String>::new());
    let (named, parent) = (
        Name::of(Name::from_text("prop_crate_a").hash).shown(),
        Name::of(Name::from_text("yard_hd_a").hash).shown(),
    );
    assert_eq!(
        merge.report.notes,
        [format!(
            "entity 2 of the merged map ({named} at (20, 20, 0.5), guid 3) names no LOD parent: ours named entity 6 of the base ({parent} at (10, 14, 0), guid 7), which theirs deleted"
        )]
    );
    assert_eq!(
        map.entities[0].num_children, 1,
        "the first parent lost the child theirs deleted"
    );
}

#[test]
fn a_child_a_deletion_left_without_its_parent_is_no_change_of_the_child() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours deletes the second parent, which leaves its two children without one; theirs moves that parent and deletes a child.
    remove_entity(&mut ours, 3).unwrap();
    assert_eq!(
        (ours.entities[3].parent_index, ours.entities[4].parent_index),
        (-1, -1)
    );
    moved(&mut theirs, 3, [30.0, 11.0, 0.0]);
    remove_entity(&mut theirs, 5).unwrap();
    theirs.entities[3].num_children = 1;

    // The parent is the one conflict: the child theirs deleted is one deletion, though ours left it without a parent.
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    let said: Vec<(&Disagreement, Vec<usize>)> = (stopped.report.conflicts.iter())
        .map(|c| (&c.what, indexes(&c.base)))
        .collect();
    assert_eq!(
        said,
        [(
            &Disagreement::ChangedAndDeleted {
                deleted_by: Side::Ours,
                changed: vec!["position".to_string()],
            },
            vec![3]
        )]
    );
    // Kept as theirs has it, the parent keeps the child that is left, which ours did not change.
    let kept = merged(&base, &ours, &theirs, Take::Theirs);
    let map = kept.merged.unwrap();
    assert_eq!(position_of(&map, 4), [30.0, 11.0, 0.0]);
    assert_eq!(
        parents(&map)[&5],
        Some(4),
        "the child still names the parent that was kept"
    );
    let at = map.entities.iter().position(|e| e.guid == 4).unwrap();
    assert_eq!(map.entities[at].num_children, 1);
    assert_eq!(kept.report.notes, Vec::<String>::new());
    // Deleted as ours has it, the child names no parent, as ours left it, and that is no news.
    let gone = merged(&base, &ours, &theirs, Take::Ours);
    let map = gone.merged.unwrap();
    assert!(map.entities.iter().all(|e| e.guid != 4 && e.guid != 6));
    assert_eq!(parents(&map)[&5], None);
    assert_eq!(gone.report.notes, Vec::<String>::new());
    assert_eq!(problems(&map), Vec::<String>::new());
    // What ours is said to have changed of the child names no member: the removal of its parent is the change.
    assert!(
        stopped.report.applied.iter().all(|a| a.did != Did::Changed),
        "{:?}",
        stopped.report.applied
    );
}

#[test]
fn the_twins_of_the_side_taken_stand_in_the_places_of_the_base_and_a_child_still_names_one() {
    let yard = |second: [f32; 3], child: [f32; 3]| {
        map(&[
            Entity {
                children: 0,
                ..Entity::at("yard_lod", [10.0, 20.0, 0.0], 7)
            },
            Entity {
                children: 1,
                ..Entity::at("yard_lod", second, 7)
            },
            Entity {
                parent: 1,
                ..Entity::at("yard_hd", child, 9)
            },
            Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 3),
        ])
    };
    let base = yard([14.0, 20.0, 0.0], [14.0, 21.0, 0.0]);
    let ours = yard([14.0, 23.0, 0.0], [14.0, 21.0, 0.0]);
    let theirs = yard([14.0, 20.0, 0.0], [15.0, 21.0, 0.0]);
    let merge = merged(&base, &ours, &theirs, Take::Ours);
    assert_eq!(merge.report.conflicts.len(), 1);
    assert_eq!(merge.report.conflicts[0].what, Disagreement::Ambiguous);
    let map = merge.merged.unwrap();
    let guids: Vec<u32> = map.entities.iter().map(|e| e.guid).collect();
    assert_eq!(guids, [7, 7, 9, 3], "every entity is at the index it had");
    assert_eq!(map.entities[1].position.y, 23.0, "the twin ours moved");
    assert_eq!(map.entities[2].position.x, 15.0, "the child theirs moved");
    assert_eq!(
        map.entities[2].parent_index, 1,
        "which still names the twin"
    );
    assert_eq!(map.entities[1].num_children, 1);
    assert_eq!(problems(&map), Vec::<String>::new());
}

/// A map of two entities that state `priority` in a member no typed record names, and that states `generators` of itself in another.
fn a_map_with_members_no_record_types(priority: [u64; 2], generators: u64) -> CMapData {
    a_map_with_members_no_record_types_in(BinaryFormat::Rbm, priority, generators)
}

/// [`a_map_with_members_no_record_types`], saved in the container `format` names.
fn a_map_with_members_no_record_types_in(
    format: BinaryFormat,
    priority: [u64; 2],
    generators: u64,
) -> CMapData {
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
    <u32 name="numChildren"/>
    <u32 name="priorityLevel"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="carGenerators"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;
    assert!(
        !CEntityDef::MEMBERS.contains(&"priorityLevel")
            && !CMapData::MEMBERS.contains(&"carGenerators"),
        "the members are ones the typed records carry and do not name"
    );
    let entities = (priority.iter().enumerate())
        .map(|(at, priority)| {
            let mut entity = MetaInstance::new("CEntityDef");
            let name = ["prop_crate_a", "prop_crate_b"][at];
            entity.set("archetypeName", Value::Hash(HashedString::new(name)));
            entity.set("flags", Value::UInt(32));
            entity.set("guid", Value::UInt(at as u64 + 1));
            entity.set("position", Value::Vec3([1.0 + at as f32 * 4.0, 2.0, 3.0]));
            entity.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
            entity.set("scaleXY", Value::Float(1.0));
            entity.set("scaleZ", Value::Float(1.0));
            entity.set("parentIndex", Value::Int(-1));
            entity.set("lodDist", Value::Float(100.0));
            entity.set("numChildren", Value::UInt(0));
            entity.set("priorityLevel", Value::UInt(*priority));
            Value::Pointer(Some(Box::new(entity)))
        })
        .collect();
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new("harbour")));
    root.set("flags", Value::UInt(0));
    root.set("carGenerators", Value::UInt(generators));
    for (corner, at) in [
        ("streamingExtentsMin", [-200.0; 3]),
        ("streamingExtentsMax", [200.0; 3]),
        ("entitiesExtentsMin", [0.0; 3]),
        ("entitiesExtentsMax", [100.0; 3]),
    ] {
        root.set(corner, Value::Vec3(at));
    }
    root.set("entities", Value::Array(entities));
    let library = build_library([("inline.psc", PSC.as_bytes())]).unwrap();
    let layout = canonical::layout_for(&library, &root, format, None).unwrap();
    let bytes = write_meta_binary(format, Some(&layout), &root).unwrap();
    assert_eq!(detect_binary_format(&bytes), Some(format));
    CMapData::from_bytes(&bytes).unwrap()
}

#[test]
fn what_both_sides_changed_to_one_value_in_two_containers_merges_into_the_container_of_the_base_in_either_order(
) {
    let base = a_map_with_members_no_record_types([1, 1], 1);
    // Both sides state the same of what no typed member names, of the map and of an entity: ours in an RBM file, theirs in a PSO.
    let mut ours = a_map_with_members_no_record_types_in(BinaryFormat::Rbm, [2, 1], 5);
    let mut theirs = a_map_with_members_no_record_types_in(BinaryFormat::Pso, [2, 1], 5);
    moved(&mut ours, 0, [3.0, 2.0, 3.0]);
    moved(&mut theirs, 1, [9.0, 2.0, 3.0]);
    let one = merged(&base, &ours, &theirs, Take::Stop);
    let other = merged(&base, &theirs, &ours, Take::Stop);
    assert_eq!(one.report.conflicts, Vec::<Conflict>::new());
    assert_eq!(other.report.conflicts, Vec::<Conflict>::new());
    let (one, other) = (
        one.merged.unwrap().to_bytes().unwrap(),
        other.merged.unwrap().to_bytes().unwrap(),
    );
    assert_eq!(detect_binary_format(&one), Some(BinaryFormat::Rbm));
    assert_eq!(detect_binary_format(&other), Some(BinaryFormat::Rbm));
    assert_eq!(one, other, "the same file whichever side is called ours");
    let map = CMapData::from_bytes(&one).unwrap();
    assert!(!map_changes(&ours, &map).other && !map_changes(&theirs, &map).other);
    assert_eq!(position_of(&map, 1), [3.0, 2.0, 3.0]);
    assert_eq!(position_of(&map, 2), [9.0, 2.0, 3.0]);

    // What only the side in the other container changed of the file cannot be put into the container of the base: refused, with
    // the side and the two containers named, in either order.
    let mut ours = a_map_with_members_no_record_types_in(BinaryFormat::Rbm, [1, 1], 1);
    let mut theirs = a_map_with_members_no_record_types_in(BinaryFormat::Pso, [1, 1], 7);
    moved(&mut ours, 0, [3.0, 2.0, 3.0]);
    moved(&mut theirs, 1, [9.0, 2.0, 3.0]);
    for (first, second, which) in [(&ours, &theirs, "theirs"), (&theirs, &ours, "ours")] {
        let refused = merge_maps(&base, first, second, &mut |_| Take::Stop).unwrap_err();
        assert!(
            matches!(&refused, Error::Input { which: named, why } if *named == which && why.contains("is a PSO file where the base is a RBM file")),
            "{refused}"
        );
    }
    // Merged with the base itself it is that side as it is, in its own container.
    let alone = merged(&base, &theirs, &base.clone(), Take::Stop)
        .merged
        .unwrap();
    assert_eq!(alone.to_bytes().unwrap(), theirs.to_bytes().unwrap());
}

#[test]
fn what_no_typed_member_names_is_taken_whole_from_the_side_that_changed_it_and_never_merged_inside()
{
    let base = a_map_with_members_no_record_types([1, 1], 1);
    // Ours changes what no member names of the first entity, and theirs moves that entity and changes the same of the map.
    let ours = a_map_with_members_no_record_types([2, 1], 1);
    let mut theirs = a_map_with_members_no_record_types([1, 1], 5);
    moved(&mut theirs, 0, [3.0, 2.0, 3.0]);
    assert_eq!(
        map_changes(&base, &ours).changed[0].changes,
        [EntityChange::Other]
    );
    assert!(map_changes(&base, &theirs).other);

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = CMapData::from_bytes(&merge.merged.unwrap().to_bytes().unwrap()).unwrap();
    // The comparison is the witness: of ours the merged map differs by the move and by what theirs states of the map, and of
    // theirs by what ours states of the entity.
    let of_ours = map_changes(&ours, &map);
    assert!(
        of_ours.other,
        "what theirs states of the map is in the merged map"
    );
    assert!(
        matches!(
            of_ours.changed[0].changes.as_slice(),
            [EntityChange::Moved { .. }]
        ),
        "{of_ours:?}"
    );
    let of_theirs = map_changes(&theirs, &map);
    assert!(!of_theirs.other);
    assert_eq!(of_theirs.changed[0].changes, [EntityChange::Other]);
    let said: Vec<(&str, By)> = (merge.report.file.iter())
        .map(|m| (m.member.as_str(), m.by))
        .collect();
    assert_eq!(said, [("what no typed member names", By::Theirs)]);

    // Both changing it of one entity, or of the map, to something else is a conflict; to the same thing it is one change.
    let theirs = a_map_with_members_no_record_types([3, 1], 1);
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(stopped.report.conflicts.len(), 1);
    assert!(
        matches!(&stopped.report.conflicts[0].what, Disagreement::Member { member, .. } if member == "what no typed member names"),
        "{:?}",
        stopped.report.conflicts
    );
    let taken = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert!(map_changes(&theirs, &taken).is_empty(), "theirs, whole");
    let (ours, theirs) = (
        a_map_with_members_no_record_types([1, 1], 2),
        a_map_with_members_no_record_types([1, 4], 3),
    );
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(
        stopped.report.conflicts[0].what,
        Disagreement::File {
            member: "what no typed member names".to_string(),
            ours: "changed".to_string(),
            theirs: "changed".to_string(),
        }
    );
    let same = a_map_with_members_no_record_types([1, 4], 2);
    let merge = merged(&base, &ours, &same, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    assert!(map_changes(&same, &merge.merged.unwrap()).is_empty());
}

/// A map that places one interior, with the members of an interior instance, and one crate.
fn a_map_with_an_interior() -> CMapData {
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
    <u32 name="numChildren"/>
  </structdef>
  <structdef type="CMloInstanceDef" base="CEntityDef">
    <u32 name="groupId"/>
    <u32 name="floorId"/>
    <u32 name="numExitPortals"/>
    <u32 name="MLOInstflags"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;
    let entity = |class: &str, name: &str, guid: u64, at: [f32; 3]| {
        let mut entity = MetaInstance::new(class);
        entity.set("archetypeName", Value::Hash(HashedString::new(name)));
        entity.set("flags", Value::UInt(32));
        entity.set("guid", Value::UInt(guid));
        entity.set("position", Value::Vec3(at));
        entity.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
        entity.set("scaleXY", Value::Float(1.0));
        entity.set("scaleZ", Value::Float(1.0));
        entity.set("parentIndex", Value::Int(-1));
        entity.set("lodDist", Value::Float(100.0));
        entity.set("numChildren", Value::UInt(0));
        entity
    };
    let mut interior = entity("CMloInstanceDef", "harbour_office", 1, [10.0, 10.0, 0.0]);
    for (member, stated) in [
        ("groupId", 1),
        ("floorId", 2),
        ("numExitPortals", 3),
        ("MLOInstflags", 4),
    ] {
        interior.set(member, Value::UInt(stated));
    }
    let entities = [
        interior,
        entity("CEntityDef", "prop_crate_a", 2, [20.0, 20.0, 0.5]),
    ];
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new("harbour")));
    for (corner, at) in [
        ("streamingExtentsMin", [-200.0; 3]),
        ("streamingExtentsMax", [200.0; 3]),
        ("entitiesExtentsMin", [0.0; 3]),
        ("entitiesExtentsMax", [100.0; 3]),
    ] {
        root.set(corner, Value::Vec3(at));
    }
    root.set(
        "entities",
        Value::Array(
            (entities.into_iter())
                .map(|e| Value::Pointer(Some(Box::new(e))))
                .collect(),
        ),
    );
    let library = build_library([("inline.psc", PSC.as_bytes())]).unwrap();
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).unwrap();
    let bytes = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).unwrap();
    CMapData::from_bytes(&bytes).unwrap()
}

#[test]
fn the_members_of_an_interior_instance_merge_one_by_one() {
    let base = a_map_with_an_interior();
    let interior = |map: &mut CMapData, change: &dyn Fn(&mut CMloInstanceDef)| {
        let CEntityDefAny::CMloInstanceDef(interior) = &mut map.entities[0] else {
            panic!("the first entity is an interior");
        };
        change(interior);
    };
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    interior(&mut ours, &|i| i.group_id = 7);
    interior(&mut theirs, &|i| i.floor_id = 8);
    moved(&mut theirs, 0, [12.0, 10.0, 0.0]);
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = CMapData::from_bytes(&merge.merged.unwrap().to_bytes().unwrap()).unwrap();
    let held = map.entities[0]
        .as_mlo_instance()
        .expect("an interior still");
    assert_eq!(
        (
            held.group_id,
            held.floor_id,
            held.num_exit_portals,
            held.mlo_instflags
        ),
        (7, 8, 3, 4)
    );
    assert_eq!(position_of(&map, 1), [12.0, 10.0, 0.0]);

    // One member both changed to different values is a conflict that names it by the engine's name.
    interior(&mut theirs, &|i| i.group_id = 9);
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(
        stopped.report.conflicts[0].what,
        Disagreement::Member {
            member: "groupId".to_string(),
            ours: "7".to_string(),
            theirs: "9".to_string(),
        }
    );
    let map = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(
        map.entities[0].as_mlo_instance().map(|i| i.group_id),
        Some(9)
    );
}

#[test]
fn a_copy_in_another_container_merges_into_the_container_of_the_base() {
    let base = map(&[
        Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 1),
        Entity::at("prop_crate_b", [5.0, 1.0, 0.0], 2),
    ]);
    let mut ours = base.clone();
    moved(&mut ours, 0, [2.0, 1.0, 0.0]);
    // Theirs is a map made from nothing, which is a PSO where the base is an RBM file: the two crates by their guids, one moved,
    // and a bench.
    let mut built = or_world::create::map("harbour");
    for (name, at) in [
        ("prop_crate_a", [1.0, 1.0, 0.0]),
        ("prop_crate_b", [7.0, 1.0, 0.0]),
        ("prop_bench", [9.0, 9.0, 0.0]),
    ] {
        let placed = or_world::create::Placed {
            flags: 32,
            ..or_world::create::Placed::at(HashedString::new(name), Vec3::from_array(at), 100.0)
        };
        or_world::create::place(&mut built, &placed).unwrap();
    }
    let saved = built.to_bytes().unwrap();
    assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Pso));
    let theirs = CMapData::from_bytes(&saved).unwrap();

    let map = merged(&base, &ours, &theirs, Take::Stop).merged.unwrap();
    let saved = map.to_bytes().unwrap();
    assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Rbm));
    let back = CMapData::from_bytes(&saved).unwrap();
    assert_eq!(back.entities.len(), 3);
    assert_eq!(position_of(&back, 1), [2.0, 1.0, 0.0]);
    assert_eq!(position_of(&back, 2), [7.0, 1.0, 0.0]);
    assert!(back.entities[2].archetype_name == "prop_bench");
}

#[test]
fn roots_built_here_and_never_saved_are_refused_with_the_reason_and_merge_once_saved() {
    let mut base = or_world::create::map("harbour");
    for (name, at) in [
        ("prop_crate_a", [1.0, 1.0, 0.0]),
        ("prop_crate_b", [7.0, 1.0, 0.0]),
    ] {
        let placed =
            or_world::create::Placed::at(HashedString::new(name), Vec3::from_array(at), 100.0);
        or_world::create::place(&mut base, &placed).unwrap();
    }
    let sides = |base: &CMapData| {
        let (mut ours, mut theirs) = (base.clone(), base.clone());
        moved(&mut ours, 0, [2.0, 1.0, 0.0]);
        moved(&mut theirs, 1, [9.0, 1.0, 0.0]);
        (ours, theirs)
    };
    // ⚠ An operation carries a record as the value tree its file saves it as, and a record built here has none until it is saved.
    let (ours, theirs) = sides(&base);
    let refused = merge_maps(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap_err();
    assert!(refused.to_string().contains("cannot be saved"), "{refused}");

    let base = CMapData::from_bytes(&base.to_bytes().unwrap()).unwrap();
    let (ours, theirs) = sides(&base);
    let map = merged(&base, &ours, &theirs, Take::Stop).merged.unwrap();
    assert_eq!(position_of(&map, 1), [2.0, 1.0, 0.0]);
    assert_eq!(position_of(&map, 2), [9.0, 1.0, 0.0]);
}

fn types(archetypes: &[Archetype<'_>]) -> CMapTypes {
    CMapTypes::from_bytes(&types_file("harbour_types", archetypes)).unwrap()
}

#[test]
fn a_type_file_merges_by_the_names_of_its_archetypes() {
    let crates = [
        Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
        Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        Archetype::new("prop_crate_c", [-3.0; 3], [3.0; 3]),
        Archetype::new("prop_crate_d", [-4.0; 3], [4.0; 3]),
    ];
    let base = types(&crates);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours takes the first out, so every later index shifts, changes a distance and adds a bench.
    ours.archetypes.remove(0);
    ours.archetypes[0].lod_dist = 250.0;
    let mut bench = ours.archetypes[0].clone();
    bench.name = HashedString::new("prop_bench");
    bench.asset_name = HashedString::new("prop_bench");
    ours.archetypes.push(bench);
    // Theirs changes another member of the one ours changed, a member of another, and adds a tree.
    theirs.archetypes[1].special_attribute = 3;
    theirs.archetypes[2].bs_radius = 9.0;
    let mut tree = theirs.archetypes[0].clone();
    tree.name = HashedString::new("prop_tree");
    theirs.archetypes.insert(0, tree);

    let merge = merge_types(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let merged = merge.merged.unwrap();
    let names: Vec<u32> = merged.archetypes.iter().map(|a| a.name.hash).collect();
    let hash = |name: &str| Name::from_text(name).hash;
    let mut expected = vec![
        hash("prop_crate_b"),
        hash("prop_crate_c"),
        hash("prop_crate_d"),
    ];
    let mut added = vec![hash("prop_bench"), hash("prop_tree")];
    added.sort_unstable();
    expected.extend(added);
    assert_eq!(
        names, expected,
        "the base's in their order, then the ones added, by name"
    );
    assert_eq!(merged.archetypes[0].lod_dist, 250.0, "ours");
    assert_eq!(
        merged.archetypes[0].special_attribute, 3,
        "theirs, of the same archetype"
    );
    assert_eq!(merged.archetypes[1].bs_radius, 9.0);
    merged.to_bytes().expect("the merged type file saves");

    // Both change one member of one archetype to different values: one conflict that names it as each file indexes it.
    theirs.archetypes[2].lod_dist = 300.0;
    let stopped = merge_types(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
    assert!(stopped.merged.is_none());
    assert_eq!(stopped.report.conflicts.len(), 1);
    let conflict = &stopped.report.conflicts[0];
    assert_eq!(
        conflict.what,
        Disagreement::Member {
            member: "lodDist".to_string(),
            ours: "250".to_string(),
            theirs: "300".to_string(),
        }
    );
    assert_eq!(
        (
            indexes(&conflict.base),
            indexes(&conflict.ours),
            indexes(&conflict.theirs)
        ),
        (vec![1], vec![0], vec![2])
    );
    for (take, distance) in [(Take::Ours, 250.0), (Take::Theirs, 300.0)] {
        let merged = merge_types(&base, &ours, &theirs, &mut |_| take)
            .unwrap()
            .merged
            .unwrap();
        assert_eq!(merged.archetypes[0].lod_dist, distance);
        assert_eq!(merged.archetypes[0].special_attribute, 3);
    }

    // One side changed what the other deleted; and both added one name with other members.
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours.archetypes.remove(3);
    theirs.archetypes[3].lod_dist = 12.0;
    for (side, radius) in [(&mut ours, 1.0), (&mut theirs, 2.0)] {
        let mut bench = side.archetypes[0].clone();
        bench.name = HashedString::new("prop_bench");
        bench.bs_radius = radius;
        side.archetypes.push(bench);
    }
    let stopped = merge_types(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
    let said: Vec<&Disagreement> = stopped.report.conflicts.iter().map(|c| &c.what).collect();
    assert_eq!(
        said,
        [
            &Disagreement::ChangedAndDeleted {
                deleted_by: Side::Ours,
                changed: vec!["lodDist".to_string()],
            },
            &Disagreement::Double {
                differ: vec!["bsRadius".to_string()]
            }
        ]
    );
    let merged = merge_types(&base, &ours, &theirs, &mut |_| Take::Theirs)
        .unwrap()
        .merged
        .unwrap();
    assert_eq!(merged.archetypes.len(), 5);
    assert_eq!(merged.archetypes[3].lod_dist, 12.0);
    assert_eq!(merged.archetypes[4].bs_radius, 2.0);

    // With the base itself, the other side byte for byte; and in the other order, the same file.
    let alone = merge_types(&base, &ours, &base.clone(), &mut |_| Take::Stop).unwrap();
    assert_eq!(
        alone.merged.unwrap().to_bytes().unwrap(),
        ours.to_bytes().unwrap()
    );
    let mirrored = merge_types(&base, &theirs, &ours, &mut |_| Take::Ours)
        .unwrap()
        .merged
        .unwrap();
    assert_eq!(merged.to_bytes().unwrap(), mirrored.to_bytes().unwrap());
}

#[test]
fn three_files_merge_as_the_roots_do_and_a_file_of_another_kind_is_refused() {
    let (base, ours, theirs) = two_sides();
    let bytes = |map: &CMapData| map.to_bytes().unwrap();
    let merge = merge_files(
        "ymap",
        &bytes(&base),
        &bytes(&ours),
        &bytes(&theirs),
        &mut |_| Take::Ours,
    )
    .unwrap();
    let roots = merged(&base, &ours, &theirs, Take::Ours);
    assert_eq!(merge.merged.unwrap(), bytes(&roots.merged.unwrap()));
    assert_eq!(merge.report.conflicts, roots.report.conflicts);

    let refused = merge_files("ymt", &[], &[], &[], &mut |_| Take::Ours).unwrap_err();
    assert!(matches!(refused, Error::Kind(_)), "{refused}");
    let refused = merge_files(
        "ymap",
        &bytes(&base),
        b"no map",
        &bytes(&theirs),
        &mut |_| Take::Ours,
    )
    .unwrap_err();
    assert!(
        refused
            .to_string()
            .starts_with("ours does not read as a map file"),
        "{refused}"
    );
}

#[test]
fn another_copy_of_a_map_merges_into_its_source_with_the_base_the_source_keeps() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let game = map_file("harbour", &a_yard(), ([0.0; 3], [100.0; 3]));
    let added = add_files(
        &folder,
        &[Incoming {
            name: "harbour.ymap",
            bytes: &game,
            from_game: true,
            virtual_path: Some("x64a.rpf/levels/harbour.ymap"),
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let dir = folder.join("harbour.metadata.src");

    // The project moved the crate; another resource ships a copy of the game's map with a child moved and one taken out.
    or_maps::edit::edit_map(&dir, |map| {
        moved(map, 2, [22.0, 21.0, 0.5]);
        Ok(())
    })
    .unwrap();
    let mut theirs = CMapData::from_bytes(&game).unwrap();
    moved(&mut theirs, 5, [31.0, 15.0, 0.0]);
    remove_entity(&mut theirs, 1).unwrap();
    let theirs = theirs.to_bytes().unwrap();

    let done = merge_source(&dir, &theirs, &mut |_| Take::Stop).unwrap();
    assert_eq!(done.report.conflicts, Vec::<Conflict>::new());
    assert!(done.edited.expect("the source was written").changed());
    let now = CMapData::from_bytes(&or_maps::facts::cooked(&dir).unwrap()).unwrap();
    assert_eq!(now.entities.len(), 6);
    assert_eq!(position_of(&now, 3), [22.0, 21.0, 0.5]);
    assert_eq!(position_of(&now, 6), [31.0, 15.0, 0.0]);
    assert!(now.entities.iter().all(|e| e.guid != 2));
    // The base the source keeps is still the game's file: what the source changed of it now is all three.
    let SourceChanges::Map(changes) = or_maps::changes::source_changes(&dir).unwrap() else {
        panic!("a map source");
    };
    assert_eq!(
        (
            changes.changed.len(),
            changes.deleted.len(),
            changes.added.len()
        ),
        (2, 1, 0)
    );

    // A conflict that stops leaves the source as it was.
    let before = std::fs::read(dir.join("metadata.xml")).unwrap();
    let mut again = CMapData::from_bytes(&game).unwrap();
    moved(&mut again, 2, [1.0, 1.0, 0.5]);
    let stopped = merge_source(&dir, &again.to_bytes().unwrap(), &mut |_| Take::Stop).unwrap();
    assert!(stopped.edited.is_none());
    assert_eq!(stopped.report.conflicts.len(), 1);
    assert_eq!(std::fs::read(dir.join("metadata.xml")).unwrap(), before);
}
