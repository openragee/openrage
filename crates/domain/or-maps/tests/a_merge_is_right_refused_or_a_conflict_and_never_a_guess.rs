//! The cases where a merge could lose a mapper's work without a sign: each ends in a merged map whose links, counts and guids are
//! right, in a refusal that names its reason, or in a conflict, and a model of the three files says what random edits merge to.
//!
//! ⚠ The files are the crate's samples and the edits are `or_maps::edit`'s, so the clean checkout runs this.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use glam::Vec3;
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{
    CBaseArchetypeDefAny, CEntityDefAny, CMapData, CMapTypes, CMloArchetypeDef, CTimeArchetypeDef,
};
use or_io::meta::{
    build_library, canonical, write_meta_binary, BinaryFormat, MetaDocument, MetaInstance, Value,
};
use or_maps::changes::map_changes;
use or_maps::edit::{add_entity, place_entity, pose_of, remove_entity};
use or_maps::facts::Name;
use or_maps::merge::{
    merge_maps, merge_types, By, Conflict, Did, Disagreement, Error, Merge, Ref, Take,
};
use or_maps::sample::{map_file, map_file_with_container_lods, types_file, Archetype, Entity};
use or_world::transform::Pose;

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

/// A copy of the entity `donor` of `map` that places `archetype` at `at`, under the parent `parent` when one is given.
fn added(map: &mut CMapData, archetype: &str, at: [f32; 3], parent: Option<usize>) -> usize {
    let donor = map.entities[2].clone();
    let pose = Pose {
        position: Vec3::from_array(at),
        ..pose_of(&donor)
    };
    let index = add_entity(map, &donor, Name::from_text(archetype), pose).unwrap();
    if let Some(parent) = parent {
        map.entities[index].parent_index = parent as i32;
        map.entities[parent].num_children += 1;
    }
    index
}

fn merged(base: &CMapData, ours: &CMapData, theirs: &CMapData, take: Take) -> Merge<CMapData> {
    merge_maps(base, ours, theirs, &mut |_| take).unwrap()
}

fn indexes(refs: &[Ref]) -> Vec<usize> {
    refs.iter().map(Ref::index).collect()
}

fn place(entity: &CEntityDefAny) -> [f32; 3] {
    [entity.position.x, entity.position.y, entity.position.z]
}

/// The entity of `map` that stands at `at`, which one entity does.
fn standing(map: &CMapData, at: [f32; 3]) -> usize {
    let found: Vec<usize> = (map.entities.iter().enumerate())
        .filter(|(_, e)| place(e) == at)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(found.len(), 1, "one entity stands at {at:?}");
    found[0]
}

/// Where the parent of the entity at `at` stands, none for an entity that names none.
fn parent_of(map: &CMapData, at: [f32; 3]) -> Option<[f32; 3]> {
    let entity = &map.entities[standing(map, at)];
    usize::try_from(entity.parent_index)
        .ok()
        .map(|parent| place(&map.entities[parent]))
}

/// What `or_world_gta5::validate` finds in `map` as it saves.
fn problems(map: &CMapData) -> Vec<String> {
    let bytes = map.to_bytes().expect("the merged map saves");
    let document = MetaDocument::parse(or_io::meta::default_library(), bytes).expect("reads back");
    let view = or_world_gta5::map_data::MapData::new(&document.instance).expect("a CMapData");
    (or_world_gta5::validate::check(&view).iter())
        .map(ToString::to_string)
        .collect()
}

/// The entities of `map` whose count of children is not the number of entities of `map` that name them: none in a map no child map adds to.
///
/// ⚠ `or_world_gta5::validate` does not judge a count, which a child map adds to, so the counts are judged here.
fn miscounted(map: &CMapData) -> Vec<(usize, u32, usize)> {
    (map.entities.iter().enumerate())
        .filter_map(|(at, entity)| {
            let named = (map.entities.iter())
                .filter(|e| e.flags & (1 << 3) == 0 && usize::try_from(e.parent_index) == Ok(at))
                .count();
            (entity.num_children as usize != named).then_some((at, entity.num_children, named))
        })
        .collect()
}

fn sound(map: &CMapData) {
    assert_eq!(problems(map), Vec::<String>::new());
    assert_eq!(
        miscounted(map),
        Vec::new(),
        "index, count stated, children held"
    );
}

#[test]
fn a_parent_both_sides_deleted_is_one_deletion_and_its_children_name_none() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    remove_entity(&mut ours, 3).unwrap();
    remove_entity(&mut theirs, 3).unwrap();
    // Theirs also took a child of it out, and ours moved the other.
    remove_entity(&mut theirs, 3).unwrap();
    moved(&mut ours, 4, [30.0, 15.0, 0.0]);

    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let said: Vec<(Did, By, Option<usize>)> = (merge.report.applied.iter())
        .map(|a| (a.did, a.by, a.base.as_ref().map(Ref::index)))
        .collect();
    assert_eq!(
        said,
        [
            (Did::Deleted, By::Both, Some(3)),
            (Did::Deleted, By::Theirs, Some(4)),
            (Did::Changed, By::Ours, Some(5)),
        ]
    );
    assert_eq!(
        merge.report.notes,
        Vec::<String>::new(),
        "each side left the children without a parent itself"
    );
    let map = merge.merged.unwrap();
    let guids: Vec<u32> = map.entities.iter().map(|e| e.guid).collect();
    assert_eq!(guids, [1, 2, 3, 6, 7]);
    assert_eq!(parent_of(&map, [30.0, 15.0, 0.0]), None);
    assert_eq!(parent_of(&map, [10.0, 14.0, 0.0]), Some([10.0, 10.0, 0.0]));
    sound(&map);
}

#[test]
fn a_child_one_side_added_under_a_parent_the_other_deleted_is_kept_with_no_parent_and_a_note_that_names_it(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    remove_entity(&mut ours, 3).unwrap();
    let child = added(&mut theirs, "yard_hd_b", [30.0, 16.0, 0.0], Some(3));
    assert_eq!(theirs.entities[3].num_children, 3);

    for (first, second, adder) in [(&ours, &theirs, "theirs"), (&theirs, &ours, "ours")] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        let map = merge.merged.unwrap();
        assert_eq!(
            map.entities.len(),
            base.entities.len(),
            "one went and one came"
        );
        let at = standing(&map, [30.0, 16.0, 0.0]);
        assert_eq!(
            map.entities[at].parent_index, -1,
            "the work is kept, and names no parent"
        );
        sound(&map);
        // The note names the entity, the parent it named, and which side did what: a name by its text where a file gives it.
        let hd = "yard_hd_b";
        let lod = Name::of(Name::from_text("yard_lod_b").hash).shown();
        let deleter = if adder == "theirs" { "ours" } else { "theirs" };
        assert_eq!(
            merge.report.notes,
            [format!(
                "entity {at} of the merged map ({hd} at (30, 16, 0), guid {}) names no LOD parent: {adder} named entity 3 of the base ({lod} at (30, 10, 0), guid 4), which {deleter} deleted",
                map.entities[at].guid
            )]
        );
    }
    let _ = child;
}

#[test]
fn a_move_by_one_side_and_another_parent_by_the_other_are_both_kept_of_one_entity() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours moves a child of the first parent and turns it; theirs gives it the second parent and draws it further.
    moved(&mut ours, 6, [12.0, 14.5, 0.0]);
    ours.entities[6].rotation.z = 0.5;
    theirs.entities[6].parent_index = 3;
    theirs.entities[0].num_children -= 1;
    theirs.entities[3].num_children += 1;
    theirs.entities[6].lod_dist = 180.0;

    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        let map = merge.merged.unwrap();
        let held = &map.entities[6];
        assert_eq!(place(held), [12.0, 14.5, 0.0]);
        assert_eq!(held.rotation.z, 0.5);
        assert_eq!(held.lod_dist, 180.0);
        assert_eq!(held.parent_index, 3);
        assert_eq!(
            (map.entities[0].num_children, map.entities[3].num_children),
            (1, 3)
        );
        sound(&map);
        // The report says each member with the side it is from, the link among them.
        let mover = if std::ptr::eq(first, &ours) {
            By::Ours
        } else {
            By::Theirs
        };
        let linker = if mover == By::Ours {
            By::Theirs
        } else {
            By::Ours
        };
        let said: Vec<(&str, By)> = (merge.report.applied.iter())
            .filter(|a| a.base.as_ref().map(Ref::index) == Some(6))
            .flat_map(|a| a.members.iter().map(|m| (m.member.as_str(), m.by)))
            .collect();
        assert_eq!(
            said,
            [
                ("position", mover),
                ("rotation", mover),
                ("lodDist", linker),
                ("parentIndex", linker),
            ]
        );
    }
}

#[test]
fn many_entities_each_side_added_under_the_same_guids_all_get_one_of_their_own() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Each side counts on from the largest guid of the base, as two editors do, so every guid of one side is a guid of the other.
    for k in 0..40 {
        let a = added(&mut ours, "prop_bench", [40.0 + k as f32, 40.0, 0.0], None);
        let b = added(&mut theirs, "prop_tree", [40.0 + k as f32, 60.0, 0.0], None);
        assert_eq!(ours.entities[a].guid, theirs.entities[b].guid);
    }
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len() + 80);
    let mut guids: Vec<u32> = map.entities.iter().map(|e| e.guid).collect();
    guids.sort_unstable();
    guids.dedup();
    assert_eq!(guids.len(), map.entities.len(), "no guid is stated twice");
    assert!(merge.report.findings.iter().all(|f| f.code != "guid-twice"));
    // Every entity is there, where its side put it, and the report says the guid each was given.
    for k in 0..40 {
        for (y, by) in [(40.0, By::Ours), (60.0, By::Theirs)] {
            let at = standing(&map, [40.0 + k as f32, y, 0.0]);
            let said: Vec<_> = (merge.report.applied.iter())
                .filter(|a| a.merged == Some(at))
                .map(|a| (a.did, a.by, a.guid))
                .collect();
            assert_eq!(said, [(Did::Added, by, Some(map.entities[at].guid))]);
        }
    }
    let mirror = merged(&base, &theirs, &ours, Take::Stop).merged.unwrap();
    assert_eq!(mirror.to_bytes().unwrap(), map.to_bytes().unwrap());
    sound(&map);
}

/// `map` with its entities in the order `order` gives their indexes, every link renumbered: the same map in another order.
fn reordered(map: &CMapData, order: &[usize]) -> CMapData {
    let mut now_at = vec![0usize; order.len()];
    for (now, &was) in order.iter().enumerate() {
        now_at[was] = now;
    }
    let mut out = map.clone();
    out.entities = order.iter().map(|&was| map.entities[was].clone()).collect();
    for entity in &mut out.entities {
        if let Ok(parent) = usize::try_from(entity.parent_index) {
            entity.parent_index = now_at[parent] as i32;
        }
    }
    out
}

#[test]
fn a_side_that_only_reordered_its_entities_changed_nothing_and_merges_with_no_conflict() {
    let base = map(&a_yard());
    // Ours holds the same entities, the children before their parents.
    let ours = reordered(&base, &[6, 5, 4, 2, 1, 3, 0]);
    sound(&ours);
    assert!(
        map_changes(&base, &ours).is_empty(),
        "an order is no change"
    );
    let mut theirs = base.clone();
    moved(&mut theirs, 5, [31.0, 15.0, 0.0]);
    remove_entity(&mut theirs, 1).unwrap();

    // With the base itself the result is the side as it is, in its order.
    let alone = merged(&base, &ours, &base.clone(), Take::Stop);
    assert_eq!(alone.report.conflicts, Vec::<Conflict>::new());
    assert_eq!(
        alone.merged.unwrap().to_bytes().unwrap(),
        ours.to_bytes().unwrap()
    );

    // With a side that changed something, the result is what that side changed, in the order of the base.
    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        let map = merge.merged.unwrap();
        let guids: Vec<u32> = map.entities.iter().map(|e| e.guid).collect();
        assert_eq!(guids, [1, 3, 4, 5, 6, 7]);
        // The extents apart, which a merge grows, it is theirs.
        assert_eq!(map.entities, theirs.entities);
        sound(&map);
    }
}

#[test]
fn a_guid_one_side_changed_is_kept_beside_what_the_other_changed_of_the_entity() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours.entities[2].guid = 300;
    moved(&mut theirs, 2, [18.0, 19.0, 0.5]);
    theirs.entities[2].lod_dist = 60.0;
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(
        (
            map.entities[2].guid,
            place(&map.entities[2]),
            map.entities[2].lod_dist
        ),
        (300, [18.0, 19.0, 0.5], 60.0)
    );
    assert_eq!(map.entities.len(), base.entities.len());

    // Both giving it another guid is a conflict of that member, which names the two.
    theirs.entities[2].guid = 400;
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    let said: Vec<&Disagreement> = stopped.report.conflicts.iter().map(|c| &c.what).collect();
    assert_eq!(
        said,
        [&Disagreement::Member {
            member: "guid".to_string(),
            ours: "300".to_string(),
            theirs: "400".to_string(),
        }]
    );
    let map = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(
        (map.entities[2].guid, place(&map.entities[2])),
        (400, [18.0, 19.0, 0.5])
    );
}

#[test]
fn a_guid_each_side_gave_another_entity_is_stated_twice_in_the_merged_map_and_is_a_finding_and_no_conflict(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours.entities[2].guid = 300;
    theirs.entities[5].guid = 300;
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!((map.entities[2].guid, map.entities[5].guid), (300, 300));
    let found: Vec<&str> = merge.report.findings.iter().map(|f| f.code).collect();
    assert_eq!(found, ["guid-twice"], "{:?}", merge.report.findings);
}

#[test]
fn entities_both_sides_added_under_one_guid_both_take_a_new_guid_and_merging_a_side_again_meets_a_double(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    let a = added(&mut ours, "prop_bench", [40.0, 40.0, 0.0], None);
    let b = added(&mut theirs, "prop_tree", [60.0, 60.0, 0.0], None);
    let shared = ours.entities[a].guid;
    assert_eq!(shared, theirs.entities[b].guid);
    let map = merged(&base, &ours, &theirs, Take::Stop).merged.unwrap();
    let (bench, tree) = (
        map.entities[standing(&map, [40.0, 40.0, 0.0])].guid,
        map.entities[standing(&map, [60.0, 60.0, 0.0])].guid,
    );
    assert!(
        bench != shared && tree != shared && bench != tree,
        "neither keeps the guid its side gave it"
    );

    // The side still holds its entity under the guid it gave it: merged with the result, the two are a double that differs in its guid.
    let again = merged(&base, &map, &ours, Take::Stop);
    assert!(again.merged.is_none());
    let said: Vec<(&Disagreement, Vec<usize>, Vec<usize>)> = (again.report.conflicts.iter())
        .map(|c| (&c.what, indexes(&c.ours), indexes(&c.theirs)))
        .collect();
    assert_eq!(
        said,
        [(
            &Disagreement::Double {
                differ: vec!["guid".to_string()]
            },
            vec![standing(&map, [40.0, 40.0, 0.0])],
            vec![a]
        )]
    );
    let kept = merged(&base, &map, &ours, Take::Ours).merged.unwrap();
    assert_eq!(
        kept.to_bytes().unwrap(),
        map.to_bytes().unwrap(),
        "taking the merged map's, it is the merged map"
    );
}

#[test]
fn an_entity_both_added_is_kept_once_though_only_one_of_the_two_files_gives_the_text_of_its_name() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    added(&mut ours, "prop_bench", [40.0, 40.0, 0.0], None);
    added(&mut theirs, "prop_bench", [40.0, 40.0, 0.0], None);
    // Theirs as a file gives it back: the name by its hash alone, where ours still holds the text.
    let theirs = CMapData::from_bytes(&theirs.to_bytes().unwrap()).unwrap();
    let (a, b) = (
        &ours.entities[7].archetype_name,
        &theirs.entities[7].archetype_name,
    );
    assert!(
        a.hash == b.hash && a.raw != b.raw,
        "one name, and the text of it in one file only"
    );
    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        assert_eq!(
            merge.merged.unwrap().entities.len(),
            base.entities.len() + 1
        );
    }
}

fn twins(second: [f32; 3]) -> CMapData {
    map(&[
        Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 9),
        Entity::at("prop_tree", [10.0, 20.0, 0.0], 7),
        Entity::at("prop_tree", second, 7),
    ])
}

#[test]
fn a_twin_a_side_moved_is_a_conflict_though_the_other_side_is_the_base_itself_and_taking_that_side_gives_it_byte_for_byte(
) {
    let base = twins([14.0, 20.0, 0.0]);
    let ours = twins([14.0, 23.0, 0.0]);
    for (first, second, mover) in [(&ours, &base, Take::Ours), (&base, &ours, Take::Theirs)] {
        // With no answer nothing is merged, and the report names the twin as each file indexes it and where each has it.
        let stopped = merged(&base, first, second, Take::Stop);
        assert!(stopped.merged.is_none());
        assert_eq!(stopped.report.conflicts.len(), 1);
        let conflict = &stopped.report.conflicts[0];
        assert_eq!(conflict.what, Disagreement::Ambiguous);
        assert_eq!(indexes(&conflict.base), [2]);
        let (of_mover, of_base) = match mover {
            Take::Ours => (&conflict.ours, &conflict.theirs),
            _ => (&conflict.theirs, &conflict.ours),
        };
        let at = |refs: &[Ref]| -> Vec<[f32; 3]> {
            (refs.iter())
                .map(|r| match r {
                    Ref::Entity(entity) => entity.position,
                    Ref::Archetype(_) => panic!("an entity"),
                })
                .collect()
        };
        assert_eq!(at(of_mover), [[14.0, 23.0, 0.0]]);
        assert_eq!(at(of_base), [[14.0, 20.0, 0.0]]);
        // Answered with the side that moved it, the result is that side; with the other, the base.
        let taken = merged(&base, first, second, mover).merged.unwrap();
        assert_eq!(taken.to_bytes().unwrap(), ours.to_bytes().unwrap());
        let other = if mover == Take::Ours {
            Take::Theirs
        } else {
            Take::Ours
        };
        let left = merged(&base, first, second, other).merged.unwrap();
        assert_eq!(left.entities, base.entities);
    }
}

#[test]
fn a_twin_only_one_side_has_is_added_with_a_guid_of_its_own_while_the_entity_it_copies_takes_the_other_sides_move(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours copies the crate with its guid, as a copy made by hand has it; theirs moves the crate.
    let copy = added(&mut ours, "prop_crate_a", [40.0, 40.0, 0.5], None);
    ours.entities[copy].guid = 3;
    moved(&mut theirs, 2, [18.0, 19.0, 0.5]);
    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        let map = merge.merged.unwrap();
        assert_eq!(map.entities.len(), base.entities.len() + 1);
        assert_eq!(
            (map.entities[2].guid, place(&map.entities[2])),
            (3, [18.0, 19.0, 0.5])
        );
        let at = standing(&map, [40.0, 40.0, 0.5]);
        assert_ne!(map.entities[at].guid, 3, "the copy is an entity of its own");
        assert!(merge.report.findings.iter().all(|f| f.code != "guid-twice"));
    }

    // When ours also moved the crate, nothing says which of its two is the crate of the base: a conflict, with every twin named.
    moved(&mut ours, 2, [21.0, 21.0, 0.5]);
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(stopped.report.conflicts.len(), 1);
    let conflict = &stopped.report.conflicts[0];
    assert_eq!(conflict.what, Disagreement::Ambiguous);
    assert_eq!(
        (
            indexes(&conflict.base),
            indexes(&conflict.ours),
            indexes(&conflict.theirs)
        ),
        (vec![2], vec![2, copy], vec![2])
    );
    // Taking ours, both of its crates are in the merged map, the second with a guid of its own that a note states.
    let merge = merged(&base, &ours, &theirs, Take::Ours);
    let map = merge.merged.unwrap();
    assert_eq!(place(&map.entities[2]), [21.0, 21.0, 0.5]);
    let at = standing(&map, [40.0, 40.0, 0.5]);
    assert_eq!(map.entities.len(), base.entities.len() + 1);
    let given = map.entities[at].guid;
    assert_ne!(given, 3);
    assert!(
        merge.report.notes.iter().any(|note| note
            .contains(&format!("entity {at} of the merged map"))
            && note.contains(&format!("the guid {given}"))),
        "{:?}",
        merge.report.notes
    );
    // Taking theirs, the crate is where theirs moved it and neither crate of ours is merged.
    let map = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len());
    assert_eq!(place(&map.entities[2]), [18.0, 19.0, 0.5]);
}

#[test]
fn two_entities_of_one_guid_in_the_base_merge_like_any_other_while_each_stands_where_it_stood() {
    // A bench and a tree that share the guid 7, and twin trees that share the guid 7 too: the game ships such maps.
    let base = map(&[
        Entity::at("prop_bench", [1.0, 1.0, 0.0], 7),
        Entity {
            children: 1,
            ..Entity::at("prop_tree", [10.0, 20.0, 0.0], 7)
        },
        Entity::at("prop_tree", [14.0, 20.0, 0.0], 7),
        Entity {
            parent: 1,
            ..Entity::at("prop_crate_a", [10.0, 21.0, 0.0], 2)
        },
    ]);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // The bench is the one entity of its archetype, so it is found though its guid is no key; the twins are not moved.
    moved(&mut ours, 0, [2.0, 2.0, 0.0]);
    theirs.entities[0].lod_dist = 40.0;
    moved(&mut theirs, 3, [10.0, 22.0, 0.0]);
    theirs.entities[2].tint_value = 5;
    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        let map = merge.merged.unwrap();
        assert_eq!(
            (place(&map.entities[0]), map.entities[0].lod_dist),
            ([2.0, 2.0, 0.0], 40.0)
        );
        assert_eq!(
            map.entities[2].tint_value, 5,
            "a twin that stands where it stood is itself"
        );
        assert_eq!(parent_of(&map, [10.0, 22.0, 0.0]), Some([10.0, 20.0, 0.0]));
        let guids: Vec<u32> = map.entities.iter().map(|e| e.guid).collect();
        assert_eq!(guids, [7, 7, 7, 2], "no guid of the base is given anew");
        sound(&map);
    }
}

#[test]
fn a_child_of_twins_names_the_twin_its_side_has_it_under_when_that_side_is_taken_for_the_twins() {
    let yard = |first: [f32; 3], second: [f32; 3], swapped: bool, prop: [f32; 3]| {
        let (a, b, parent) = if swapped {
            (second, first, 0)
        } else {
            (first, second, 1)
        };
        map(&[
            Entity {
                children: u32::from(swapped),
                ..Entity::at("yard_lod", a, 7)
            },
            Entity {
                children: u32::from(!swapped),
                ..Entity::at("yard_lod", b, 7)
            },
            Entity {
                parent,
                ..Entity::at("yard_hd", [10.0, 1.0, 0.0], 3)
            },
            Entity::at("prop_crate_a", prop, 4),
        ])
    };
    let base = yard([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], false, [50.0, 5.0, 0.0]);
    // Ours moved both twins and holds them in the other order: its child still names the twin beside it, now the first of the file.
    let ours = yard([0.0, 5.0, 0.0], [10.0, 5.0, 0.0], true, [50.0, 5.0, 0.0]);
    let theirs = yard([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], false, [55.0, 5.0, 0.0]);
    sound(&ours);
    assert_eq!(parent_of(&ours, [10.0, 1.0, 0.0]), Some([10.0, 5.0, 0.0]));

    for (first, second, take) in [(&ours, &theirs, Take::Ours), (&theirs, &ours, Take::Theirs)] {
        let merge = merged(&base, first, second, take);
        assert_eq!(merge.report.conflicts.len(), 1);
        assert_eq!(merge.report.conflicts[0].what, Disagreement::Ambiguous);
        let map = merge.merged.unwrap();
        assert_eq!(
            parent_of(&map, [10.0, 1.0, 0.0]),
            Some([10.0, 5.0, 0.0]),
            "the child names the twin ours has it under, wherever that twin is in the file"
        );
        assert_eq!(
            standing(&map, [55.0, 5.0, 0.0]),
            3,
            "and what theirs moved is moved"
        );
        sound(&map);
    }
    // Taking theirs for the twins, the entities are theirs: its twins are the base's.
    let map = merged(&base, &ours, &theirs, Take::Theirs).merged.unwrap();
    assert_eq!(map.entities, theirs.entities);
}

#[test]
fn a_child_one_side_put_under_a_twin_the_other_side_was_taken_for_says_in_a_note_that_the_twin_is_one_by_its_order(
) {
    let yard = |second: [f32; 3]| {
        map(&[
            Entity::at("yard_lod", [0.0, 0.0, 0.0], 7),
            Entity::at("yard_lod", second, 7),
            Entity::at("prop_crate_a", [50.0, 5.0, 0.0], 4),
        ])
    };
    let base = yard([10.0, 0.0, 0.0]);
    let ours = yard([10.0, 5.0, 0.0]);
    let mut theirs = base.clone();
    let child = added(&mut theirs, "yard_hd", [10.0, 1.0, 0.0], Some(1));
    let merge = merged(&base, &ours, &theirs, Take::Ours);
    let map = merge.merged.unwrap();
    let at = standing(&map, [10.0, 1.0, 0.0]);
    assert_eq!(parent_of(&map, [10.0, 1.0, 0.0]), Some([10.0, 5.0, 0.0]));
    sound(&map);
    let lod = Name::of(Name::from_text("yard_lod").hash).shown();
    assert_eq!(
        merge.report.notes,
        [format!(
            "entity {at} of the merged map (yard_hd at (10, 1, 0), guid {}) names entity 1 of the merged map ({lod} at (10, 5, 0), guid 7) as its LOD parent by the order of the files alone: theirs named a twin, the conflict of the twins took ours, and nothing says which twin of ours that one is",
            map.entities[at].guid
        )]
    );
    // Under the twins of its own side the link is the one that side states, and no note is due.
    let merge = merged(&base, &ours, &theirs, Take::Theirs);
    assert_eq!(merge.report.notes, Vec::<String>::new());
    let map = merge.merged.unwrap();
    assert_eq!(parent_of(&map, [10.0, 1.0, 0.0]), Some([10.0, 0.0, 0.0]));
    let _ = child;

    // The other way about: the side that moved a twin put the child under it, and the side that left the twins alone is taken.
    let mut ours = yard([10.0, 5.0, 0.0]);
    added(&mut ours, "yard_hd", [10.0, 6.0, 0.0], Some(1));
    let merge = merged(&base, &ours, &base.clone(), Take::Theirs);
    let map = merge.merged.unwrap();
    let at = standing(&map, [10.0, 6.0, 0.0]);
    assert_eq!(parent_of(&map, [10.0, 6.0, 0.0]), Some([10.0, 0.0, 0.0]));
    assert_eq!(
        merge.report.notes,
        [format!(
            "entity {at} of the merged map (yard_hd at (10, 6, 0), guid {}) names entity 1 of the merged map ({lod} at (10, 0, 0), guid 7) as its LOD parent by the order of the files alone: ours named a twin, the conflict of the twins took theirs, and nothing says which twin of theirs that one is",
            map.entities[at].guid
        )]
    );
    sound(&map);
}

#[test]
fn a_child_each_side_added_under_one_parent_is_counted_once_each_and_a_child_one_took_out_is_counted_out(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    added(&mut ours, "yard_hd_b", [30.0, 16.0, 0.0], Some(3));
    added(&mut theirs, "yard_hd_b", [30.0, 18.0, 0.0], Some(3));
    remove_entity(&mut theirs, 4).unwrap();
    assert_eq!(
        (
            ours.entities[3].num_children,
            theirs.entities[3].num_children
        ),
        (3, 2)
    );
    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        let map = merge.merged.unwrap();
        assert_eq!(
            map.entities[3].num_children, 3,
            "two of the base less one, and one from each side"
        );
        for y in [16.0, 18.0] {
            assert_eq!(parent_of(&map, [30.0, y, 0.0]), Some([30.0, 10.0, 0.0]));
        }
        sound(&map);
    }
}

#[test]
fn a_parent_with_its_children_that_each_side_added_is_in_the_merged_map_with_its_own_children() {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Each side adds a parent, two children under it, and a child under a parent of the base.
    for (side, x) in [(&mut ours, 40.0), (&mut theirs, 60.0)] {
        let parent = added(side, "yard_lod_new", [x, 10.0, 0.0], None);
        added(side, "yard_hd_new", [x, 12.0, 0.0], Some(parent));
        added(side, "yard_hd_new", [x, 14.0, 0.0], Some(parent));
        added(side, "yard_hd_a", [x, 16.0, 0.0], Some(0));
    }
    // Ours also takes an entity out of the middle, so its indexes are not theirs'.
    remove_entity(&mut ours, 2).unwrap();
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(map.entities.len(), base.entities.len() - 1 + 8);
    for x in [40.0, 60.0] {
        for y in [12.0, 14.0] {
            assert_eq!(parent_of(&map, [x, y, 0.0]), Some([x, 10.0, 0.0]));
        }
        assert_eq!(parent_of(&map, [x, 16.0, 0.0]), Some([10.0, 10.0, 0.0]));
        assert_eq!(map.entities[standing(&map, [x, 10.0, 0.0])].num_children, 2);
    }
    assert_eq!(
        map.entities[0].num_children, 4,
        "two of the base and one from each side"
    );
    sound(&map);
    let mirror = merged(&base, &theirs, &ours, Take::Stop).merged.unwrap();
    assert_eq!(mirror.to_bytes().unwrap(), map.to_bytes().unwrap());
}

#[test]
fn a_parent_one_side_gave_an_entity_that_the_other_side_deleted_is_said_as_that_entity_and_who_deleted_it(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours gives the crate the first parent and deletes the second; theirs gives the crate the second parent.
    ours.entities[2].parent_index = 0;
    ours.entities[0].num_children += 1;
    remove_entity(&mut ours, 3).unwrap();
    theirs.entities[2].parent_index = 3;
    theirs.entities[3].num_children += 1;
    let (lod_a, lod_b) = (
        Name::of(Name::from_text("yard_lod_a").hash).shown(),
        Name::of(Name::from_text("yard_lod_b").hash).shown(),
    );
    let merge = merged(&base, &ours, &theirs, Take::Theirs);
    let said: Vec<&Disagreement> = merge.report.conflicts.iter().map(|c| &c.what).collect();
    assert_eq!(
        said,
        [&Disagreement::Member {
            member: "parentIndex".to_string(),
            ours: format!("entity 0 of the base ({lod_a} at (10, 10, 0), guid 1)"),
            theirs: format!(
                "entity 3 of the base ({lod_b} at (30, 10, 0), guid 4), which ours deleted"
            ),
        }]
    );
    // Theirs was taken, and the parent it named is gone: the crate names none, and a note says so.
    let map = merge.merged.unwrap();
    assert_eq!(parent_of(&map, [20.0, 20.0, 0.5]), None);
    assert_eq!(merge.report.notes.len(), 1, "{:?}", merge.report.notes);
    assert!(
        merge.report.notes[0].ends_with(&format!(
            "names no LOD parent: theirs named entity 3 of the base ({lod_b} at (30, 10, 0), guid 4), which ours deleted"
        )),
        "{:?}",
        merge.report.notes
    );
    sound(&map);
    let map = merged(&base, &ours, &theirs, Take::Ours).merged.unwrap();
    assert_eq!(parent_of(&map, [20.0, 20.0, 0.5]), Some([10.0, 10.0, 0.0]));
    sound(&map);
}

#[test]
fn an_entity_given_the_guid_of_one_its_side_deleted_is_read_as_that_one_moved_and_takes_what_the_other_side_changed_of_it(
) {
    // ⚠ What this holds is a wrong merge with no sign, left as it is built: a guid is believed, and the roadmap leaves the reuse
    // of one to the owner. Two crates; ours deletes the second and gives its guid to the first, and theirs draws the second further.
    let base = map(&[
        Entity::at("prop_crate_a", [1.0, 1.0, 0.0], 1),
        Entity::at("prop_crate_a", [5.0, 1.0, 0.0], 2),
        Entity::at("prop_bench", [9.0, 9.0, 0.0], 3),
    ]);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    remove_entity(&mut ours, 1).unwrap();
    ours.entities[0].guid = 2;
    theirs.entities[1].lod_dist = 250.0;
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
    let map = merge.merged.unwrap();
    assert_eq!(map.entities.len(), 2);
    assert_eq!(
        (
            map.entities[0].guid,
            place(&map.entities[0]),
            map.entities[0].lod_dist
        ),
        (2, [1.0, 1.0, 0.0], 250.0),
        "the first crate, under the guid of the second, drawn as far as theirs draws the second"
    );
}

#[test]
fn links_into_the_parent_map_are_the_numbers_their_sides_state_whatever_goes_out_of_this_map() {
    let mut base = map(&[
        Entity::at("yard_lod", [0.0, 0.0, 0.0], 1),
        Entity::at("yard_hd", [0.0, 1.0, 0.0], 2),
        Entity::at("yard_hd", [0.0, 2.0, 0.0], 3),
        Entity::at("prop_crate_a", [5.0, 2.0, 0.0], 4),
    ]);
    // The two in the middle name entities 4 and 9 of the parent map, as their flags say.
    for (index, parent) in [(1, 4), (2, 9)] {
        base.entities[index].flags |= 1 << 3;
        base.entities[index].parent_index = parent;
    }
    let base = CMapData::from_bytes(&base.to_bytes().unwrap()).unwrap();
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours takes the first entity out; theirs names another entity of the parent map, and adds an entity under one more.
    remove_entity(&mut ours, 0).unwrap();
    theirs.entities[2].parent_index = 11;
    let donor = theirs.entities[1].clone();
    let pose = Pose {
        position: Vec3::new(9.0, 9.0, 0.0),
        ..pose_of(&donor)
    };
    let new = add_entity(&mut theirs, &donor, Name::from_text("yard_hd"), pose).unwrap();
    theirs.entities[new].parent_index = 2;
    assert_ne!(
        theirs.entities[new].flags & (1 << 3),
        0,
        "a copy of an entity whose parent is in the parent map"
    );

    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merged(&base, first, second, Take::Stop);
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        // The one link a side changed is said to be that side's number, which no parent map was read for.
        let [note] = merge.report.notes.as_slice() else {
            panic!("{:?}", merge.report.notes);
        };
        assert!(
            note.starts_with("1 link(s) into the parent map are as ")
                && note.contains("read no parent map"),
            "{note}"
        );
        let map = merge.merged.unwrap();
        let links: Vec<i32> = map.entities.iter().map(|e| e.parent_index).collect();
        assert_eq!(
            links,
            [4, 11, -1, 2],
            "no index of the parent map moved with the entity that went out"
        );
        assert!(map.entities.iter().all(|e| e.num_children == 0));
    }
}

/// ⚠ A deletion in the parent map renumbers the links of every map under it, so two sides that each took another entity out of it
/// can both give one link one number, and the merged parent map, which lost both entities, has the entity meant at another.
#[test]
fn a_link_into_the_parent_map_that_both_sides_changed_is_a_conflict_of_its_own_though_they_state_one_number(
) {
    let mut base = map(&[
        Entity::at("yard_lod", [0.0, 0.0, 0.0], 1),
        Entity::at("yard_hd", [0.0, 1.0, 0.0], 2),
        Entity::at("yard_hd", [0.0, 2.0, 0.0], 3),
    ]);
    for (index, parent) in [(1, 4), (2, 9)] {
        base.entities[index].flags |= 1 << 3;
        base.entities[index].parent_index = parent;
    }
    let base = CMapData::from_bytes(&base.to_bytes().unwrap()).unwrap();
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours took entity 0 of the parent map out and theirs entity 6: both renumber the link to 9, to 8, and ours the link to 4 too.
    ours.entities[1].parent_index = 3;
    ours.entities[2].parent_index = 8;
    theirs.entities[2].parent_index = 8;

    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    assert!(
        stopped.merged.is_none(),
        "one number from both sides was taken as agreed"
    );
    let said: Vec<&Disagreement> = stopped.report.conflicts.iter().map(|c| &c.what).collect();
    assert_eq!(
        said,
        [&Disagreement::ParentMapLink {
            ours: "entity 8 of the parent map".to_string(),
            theirs: "entity 8 of the parent map".to_string(),
        }]
    );
    assert!(
        (stopped.report.not_compared.iter())
            .any(|said| said.contains("are not merged file by file")),
        "{:?}",
        stopped.report.not_compared
    );

    // Answered for a side, the links are that side's numbers, and the report says that no parent map was read for them.
    let taken = merged(&base, &ours, &theirs, Take::Ours);
    let map = taken.merged.unwrap();
    let links: Vec<i32> = map.entities.iter().map(|e| e.parent_index).collect();
    assert_eq!(links, [-1, 3, 8]);
    assert!(
        (taken.report.notes.iter())
            .any(|note| note.starts_with("2 link(s) into the parent map are as ours changed them")),
        "{:?}",
        taken.report.notes
    );
}

#[test]
fn a_link_into_the_parent_map_from_one_side_and_into_this_map_from_the_other_is_a_conflict_and_never_a_link_of_both(
) {
    let base = map(&a_yard());
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours says the crate's parent is entity 2 of the parent map, which its flags say too; theirs names the first parent of this map.
    ours.entities[2].flags |= 1 << 3;
    ours.entities[2].parent_index = 2;
    theirs.entities[2].parent_index = 0;
    theirs.entities[0].num_children += 1;
    let stopped = merged(&base, &ours, &theirs, Take::Stop);
    let said: Vec<&Disagreement> = stopped.report.conflicts.iter().map(|c| &c.what).collect();
    let crate_a = Name::of(Name::from_text("yard_lod_a").hash).shown();
    assert_eq!(
        said,
        [&Disagreement::Member {
            member: "parentIndex".to_string(),
            ours: "entity 2 of the parent map".to_string(),
            theirs: format!("entity 0 of the base ({crate_a} at (10, 10, 0), guid 1)"),
        }]
    );
    // Ours whole: the link into the parent map with the flag that says so.
    let map = merged(&base, &ours, &theirs, Take::Ours).merged.unwrap();
    assert_eq!(
        (
            map.entities[2].flags & (1 << 3),
            map.entities[2].parent_index
        ),
        (1 << 3, 2)
    );
    assert_eq!(map.entities[0].num_children, 2);
    sound(&map);
    // Theirs' link with the flag ours set would read entity 0 of the parent map: refused, with the entity named.
    let refused = merge_maps(&base, &ours, &theirs, &mut |_| Take::Theirs).unwrap_err();
    assert!(matches!(refused, Error::Invalid(_)), "{refused}");
    let said = refused.to_string();
    let crate_a = Name::of(Name::from_text("prop_crate_a").hash).shown();
    assert!(
        said.contains(&format!(
            "entity 2 of the base ({crate_a} at (20, 20, 0.5), guid 3)"
        )) && said.contains("which map the parent is in"),
        "{said}"
    );
}

#[test]
fn a_side_whose_own_links_are_a_cycle_is_named_as_what_cannot_be_merged() {
    let base = map(&a_yard());
    let mut ours = base.clone();
    // Ours alone makes two entities each other's parent.
    ours.entities[2].parent_index = 6;
    ours.entities[6].parent_index = 2;
    let mut theirs = base.clone();
    moved(&mut theirs, 5, [31.0, 15.0, 0.0]);
    for (first, second, which) in [(&ours, &theirs, "ours"), (&theirs, &ours, "theirs")] {
        let refused = merge_maps(&base, first, second, &mut |_| Take::Ours).unwrap_err();
        assert!(
            matches!(&refused, Error::Input { which: named, why } if *named == which && why.contains("cycle")),
            "{refused}"
        );
    }
    // A cycle neither side holds, which the two make together, is the merged file's.
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    ours.entities[2].parent_index = 6;
    theirs.entities[6].parent_index = 2;
    let refused = merge_maps(&base, &ours, &theirs, &mut |_| Take::Ours).unwrap_err();
    assert!(matches!(refused, Error::Invalid(_)), "{refused}");
}

fn types(archetypes: &[Archetype<'_>]) -> CMapTypes {
    CMapTypes::from_bytes(&types_file("harbour_types", archetypes)).unwrap()
}

fn four_crates() -> CMapTypes {
    types(&[
        Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
        Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        Archetype::new("prop_crate_c", [-3.0; 3], [3.0; 3]),
        Archetype::new("prop_crate_d", [-4.0; 3], [4.0; 3]),
    ])
}

#[test]
fn a_box_both_sides_gave_one_archetype_is_a_conflict_of_each_corner_that_names_the_archetype_and_the_two_boxes(
) {
    let base = four_crates();
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours takes the first archetype out, so the one both change is at another index in each file.
    ours.archetypes.remove(0);
    ours.archetypes[1].bb_max = or_io::decl::Vec3 {
        x: 5.0,
        y: 5.0,
        z: 5.0,
    };
    theirs.archetypes[2].bb_max = or_io::decl::Vec3 {
        x: 6.0,
        y: 6.0,
        z: 6.0,
    };
    theirs.archetypes[2].bb_min = or_io::decl::Vec3 {
        x: -6.0,
        y: -6.0,
        z: -6.0,
    };
    theirs.archetypes[2].bs_radius = 11.0;

    let stopped = merge_types(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
    assert!(stopped.merged.is_none());
    assert_eq!(
        stopped.report.conflicts.len(),
        1,
        "{:?}",
        stopped.report.conflicts
    );
    let conflict = &stopped.report.conflicts[0];
    assert_eq!(
        conflict.what,
        Disagreement::Member {
            member: "bbMax".to_string(),
            ours: "(5, 5, 5)".to_string(),
            theirs: "(6, 6, 6)".to_string(),
        }
    );
    assert_eq!(
        (
            indexes(&conflict.base),
            indexes(&conflict.ours),
            indexes(&conflict.theirs)
        ),
        (vec![2], vec![1], vec![2])
    );
    let Ref::Archetype(named) = &conflict.base[0] else {
        panic!("an archetype");
    };
    assert_eq!(named.name.hash, Name::from_text("prop_crate_c").hash);

    for (take, mirrored, corner) in [
        (Take::Ours, Take::Theirs, 5.0),
        (Take::Theirs, Take::Ours, 6.0),
    ] {
        let merge = merge_types(&base, &ours, &theirs, &mut |_| take).unwrap();
        assert_eq!(merge.report.conflicts[0].taken, take);
        let merged = merge.merged.unwrap();
        let held = &merged.archetypes[1];
        assert_eq!(held.bb_max.x, corner);
        // What only theirs changed of it is theirs' whichever side the corner is from.
        assert_eq!((held.bb_min.x, held.bs_radius), (-6.0, 11.0));
        assert_eq!(merged.archetypes.len(), 3);
        let mirror = merge_types(&base, &theirs, &ours, &mut |_| mirrored).unwrap();
        assert_eq!(
            mirror.merged.unwrap().to_bytes().unwrap(),
            merged.to_bytes().unwrap()
        );
    }
}

#[test]
fn archetypes_of_a_name_a_file_declares_twice_are_one_conflict_and_a_side_is_taken_for_all_of_them()
{
    let base = types(&[
        Archetype::new("prop_a", [-1.0; 3], [1.0; 3]),
        Archetype::new("prop_b", [-2.0; 3], [2.0; 3]),
        Archetype::new("prop_b", [-3.0; 3], [3.0; 3]),
        Archetype::new("prop_c", [-4.0; 3], [4.0; 3]),
    ]);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    // Ours changes the second of the name; theirs takes the first of it out, and changes another archetype.
    ours.archetypes[2].lod_dist = 9.0;
    theirs.archetypes[3].lod_dist = 7.0;
    theirs.archetypes.remove(1);

    let stopped = merge_types(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
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
        (vec![1, 2], vec![1, 2], vec![1]),
        "every archetype of the name, as each file indexes it"
    );
    let held = |take: Take| -> Vec<(f32, f32)> {
        let merge = merge_types(&base, &ours, &theirs, &mut |_| take).unwrap();
        let merged = merge.merged.unwrap();
        (merged.archetypes.iter())
            .map(|a| (a.lod_dist, a.bb_max.x))
            .collect()
    };
    // Each archetype by how far it is drawn and how wide its box is: the name's are the side's, and the last is theirs' in both.
    assert_eq!(
        held(Take::Ours),
        [(100.0, 1.0), (100.0, 2.0), (9.0, 3.0), (7.0, 4.0)]
    );
    assert_eq!(held(Take::Theirs), [(100.0, 1.0), (100.0, 3.0), (7.0, 4.0)]);
}

#[test]
fn a_type_file_merged_with_the_base_itself_is_the_other_side_byte_for_byte_in_that_sides_order() {
    let base = four_crates();
    let mut ours = base.clone();
    // An archetype put in the middle, one changed, and two that changed places: the order of the base would give another file.
    let mut tree = ours.archetypes[0].clone();
    tree.name = HashedString::new("prop_tree");
    ours.archetypes.insert(1, tree);
    ours.archetypes[3].lod_dist = 250.0;
    ours.archetypes.swap(0, 4);
    let saved = ours.to_bytes().unwrap();
    for (first, second) in [(&ours, &base), (&base, &ours)] {
        let merge = merge_types(&base, first, second, &mut |_| Take::Stop).unwrap();
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        assert_eq!(merge.merged.unwrap().to_bytes().unwrap(), saved);
        // Each change is said at its index in that side.
        let said: Vec<(Did, Option<usize>)> = (merge.report.applied.iter())
            .map(|a| (a.did, a.merged))
            .collect();
        assert_eq!(said, [(Did::Changed, Some(3)), (Did::Added, Some(1))]);
    }
}

#[test]
fn archetypes_each_side_added_are_in_one_order_whichever_side_is_called_ours() {
    let base = four_crates();
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    for (side, names) in [
        (&mut ours, ["prop_bench", "prop_tree", "prop_lamp"]),
        (&mut theirs, ["prop_sign", "prop_bin", "prop_post"]),
    ] {
        for name in names {
            let mut new = side.archetypes[0].clone();
            new.name = HashedString::new(name);
            side.archetypes.insert(0, new);
        }
    }
    let one = merge_types(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
    let other = merge_types(&base, &theirs, &ours, &mut |_| Take::Stop).unwrap();
    let (one, other) = (one.merged.unwrap(), other.merged.unwrap());
    assert_eq!(one.archetypes.len(), 10);
    assert_eq!(one.to_bytes().unwrap(), other.to_bytes().unwrap());
}

/// A type file that declares an interior of three entities and an archetype that is there at some hours of the day.
fn an_interior_and_a_clock() -> CMapTypes {
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
  <structdef type="CBaseArchetypeDef">
    <float name="lodDist"/>
    <u32 name="flags"/>
    <string name="name" type="atHashString"/>
    <string name="assetName" type="atHashString"/>
  </structdef>
  <structdef type="CTimeArchetypeDef" base="CBaseArchetypeDef">
    <u32 name="timeFlags"/>
  </structdef>
  <structdef type="CMloArchetypeDef" base="CBaseArchetypeDef">
    <u32 name="mloFlags"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CMapTypes">
    <string name="name" type="atHashString"/>
    <array name="archetypes" type="atArray"><pointer type="CBaseArchetypeDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;
    let entity = |name: &str, guid: u64, at: [f32; 3]| {
        let mut entity = MetaInstance::new("CEntityDef");
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
        Value::Pointer(Some(Box::new(entity)))
    };
    let archetype = |class: &str, name: &str| {
        let mut archetype = MetaInstance::new(class);
        archetype.set("lodDist", Value::Float(100.0));
        archetype.set("flags", Value::UInt(0));
        archetype.set("name", Value::Hash(HashedString::new(name)));
        archetype.set("assetName", Value::Hash(HashedString::new(name)));
        archetype
    };
    let mut office = archetype("CMloArchetypeDef", "harbour_office");
    office.set("mloFlags", Value::UInt(1));
    office.set(
        "entities",
        Value::Array(vec![
            entity("prop_chair", 1, [0.0, 0.0, 0.0]),
            entity("prop_table", 2, [1.0, 0.0, 0.0]),
            entity("prop_lamp", 3, [2.0, 0.0, 0.0]),
        ]),
    );
    let mut clock = archetype("CTimeArchetypeDef", "prop_clock");
    clock.set("timeFlags", Value::UInt(5));
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new("harbour_types")));
    root.set(
        "archetypes",
        Value::Array(vec![
            Value::Pointer(Some(Box::new(office))),
            Value::Pointer(Some(Box::new(clock))),
        ]),
    );
    let library = build_library([("inline.psc", PSC.as_bytes())]).unwrap();
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).unwrap();
    let bytes = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).unwrap();
    CMapTypes::from_bytes(&bytes).unwrap()
}

#[test]
fn an_interior_and_a_time_archetype_merge_their_own_members_one_by_one_and_an_interior_takes_its_entities_whole(
) {
    let base = an_interior_and_a_clock();
    let office = |types: &mut CMapTypes, change: &dyn Fn(&mut CMloArchetypeDef)| {
        let CBaseArchetypeDefAny::CMloArchetypeDef(office) = &mut types.archetypes[0] else {
            panic!("the first archetype is an interior");
        };
        change(office);
    };
    let clock = |types: &mut CMapTypes, change: &dyn Fn(&mut CTimeArchetypeDef)| {
        let CBaseArchetypeDefAny::CTimeArchetypeDef(clock) = &mut types.archetypes[1] else {
            panic!("the second archetype is a time archetype");
        };
        change(clock);
    };
    let held = |types: &CMapTypes| -> (u32, Vec<f32>, u32, f32) {
        let saved = CMapTypes::from_bytes(&types.to_bytes().unwrap()).unwrap();
        let (
            CBaseArchetypeDefAny::CMloArchetypeDef(office),
            CBaseArchetypeDefAny::CTimeArchetypeDef(clock),
        ) = (&saved.archetypes[0], &saved.archetypes[1])
        else {
            panic!("an interior and a time archetype still");
        };
        (
            office.mlo_flags,
            office.entities.iter().map(|e| e.position.z).collect(),
            clock.time_flags,
            clock.base.lod_dist,
        )
    };
    // Ours moves an entity of the interior and draws the clock nearer; theirs changes a member of each archetype's own type.
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    office(&mut ours, &|o| o.entities[2].position.z = 1.5);
    clock(&mut ours, &|c| c.base.lod_dist = 44.0);
    office(&mut theirs, &|o| o.mlo_flags = 9);
    clock(&mut theirs, &|c| c.time_flags = 77);
    for (first, second) in [(&ours, &theirs), (&theirs, &ours)] {
        let merge = merge_types(&base, first, second, &mut |_| Take::Stop).unwrap();
        assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
        assert_eq!(
            held(&merge.merged.unwrap()),
            (9, vec![0.0, 0.0, 1.5], 77, 44.0)
        );
    }

    // Both changing the entities of the interior, to other things, is one conflict: they are taken whole from a side, and what
    // only one side changed of the interior's members is kept whichever side that is.
    office(&mut theirs, &|o| {
        o.entities.remove(0);
    });
    let stopped = merge_types(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
    let said: Vec<&Disagreement> = stopped.report.conflicts.iter().map(|c| &c.what).collect();
    assert_eq!(
        said,
        [&Disagreement::Member {
            member: "its entities and what no typed member names".to_string(),
            ours: "changed".to_string(),
            theirs: "changed".to_string(),
        }]
    );
    for (take, mirrored, entities) in [
        (Take::Ours, Take::Theirs, vec![0.0, 0.0, 1.5]),
        (Take::Theirs, Take::Ours, vec![0.0, 0.0]),
    ] {
        let merged = merge_types(&base, &ours, &theirs, &mut |_| take)
            .unwrap()
            .merged
            .unwrap();
        assert_eq!(held(&merged), (9, entities, 77, 44.0), "taking {take:?}");
        let mirror = merge_types(&base, &theirs, &ours, &mut |_| mirrored)
            .unwrap()
            .merged
            .unwrap();
        assert_eq!(mirror.to_bytes().unwrap(), merged.to_bytes().unwrap());
    }
}

/// A small generator of numbers that gives the same ones on every machine.
struct Rng(u64);

impl Rng {
    fn of(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// How many random cases each test of random edits runs: `MERGE_ROUNDS` in the environment gives another number.
fn rounds() -> u64 {
    (std::env::var("MERGE_ROUNDS").ok())
        .and_then(|n| n.parse().ok())
        .unwrap_or(250)
}

/// A container LOD names an entity by its index, which nothing renumbers: a merge that takes out an entity before the one it
/// names would leave it naming another, and is refused as an edit that takes that entity out is; one that takes out an entity
/// after it is made.
#[test]
fn a_merge_that_takes_out_an_entity_before_one_a_container_lod_names_is_refused_and_one_after_it_is_made(
) {
    let extents = ([0.0; 3], [100.0; 3]);
    let base = map_file_with_container_lods("harbour", &a_yard(), &[3], extents);
    let base = CMapData::from_bytes(&base).unwrap();
    let mut theirs = base.clone();
    moved(&mut theirs, 5, [30.0, 15.0, 0.0]);

    let mut ours = base.clone();
    remove_entity(&mut ours, 2).unwrap();
    let refused = merge_maps(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap_err();
    assert!(matches!(refused, Error::Invalid(_)), "{refused}");
    let refused = refused.to_string();
    assert!(
        refused.contains("container LODs") && refused.contains("entity 3"),
        "{refused}"
    );

    let mut ours = base.clone();
    remove_entity(&mut ours, 6).unwrap();
    let merge = merged(&base, &ours, &theirs, Take::Stop);
    let map = merge.merged.expect("no conflict");
    assert_eq!(map.entities.len(), 6);
    assert_eq!(place(&map.entities[5]), [30.0, 15.0, 0.0]);
    assert_eq!(map.entities[0].num_children, 1);
    sound(&map);
}

/// Who an entity is across the three files: an entity of the base by its guid, or one a side added by where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Who {
    Base(u32),
    Added([u32; 3]),
}

/// What the model keeps of an entity: where it stands, how far it is drawn, and who its parent is.
#[derive(Debug, Clone, PartialEq)]
struct State {
    position: [u32; 3],
    lod: u32,
    parent: Option<Who>,
}

/// ⚠ The random edits add every entity at an `x` of a thousand or more and leave every entity of the base below it.
fn who(entity: &CEntityDefAny) -> Who {
    match entity.position.x >= 1000.0 {
        true => Who::Added(place(entity).map(f32::to_bits)),
        false => Who::Base(entity.guid),
    }
}

fn model(map: &CMapData) -> BTreeMap<Who, State> {
    let mut out = BTreeMap::new();
    for entity in &map.entities {
        let parent = usize::try_from(entity.parent_index)
            .ok()
            .map(|p| who(&map.entities[p]));
        let state = State {
            position: place(entity).map(f32::to_bits),
            lod: entity.lod_dist.to_bits(),
            parent,
        };
        assert!(out.insert(who(entity), state).is_none(), "one of each");
    }
    out
}

/// Four parents with three children each, and four props with no parent: every guid once.
fn a_base_of_parents() -> CMapData {
    let mut list = Vec::new();
    let names = ["lod_a", "lod_b", "lod_c", "lod_d"];
    let mut guid = 1;
    for (p, name) in names.iter().enumerate() {
        let at = list.len() as i32;
        list.push(Entity {
            children: 3,
            ..Entity::at(name, [p as f32 * 10.0, 0.0, 0.0], guid)
        });
        guid += 1;
        for c in 0..3 {
            list.push(Entity {
                parent: at,
                ..Entity::at("hd", [p as f32 * 10.0, 1.0 + c as f32, 0.0], guid)
            });
            guid += 1;
        }
    }
    for l in 0..4 {
        list.push(Entity::at("prop", [50.0 + l as f32, 5.0, 0.0], guid));
        guid += 1;
    }
    map(&list)
}

fn is_parent(map: &CMapData, index: usize) -> bool {
    [1u32, 5, 9, 13].contains(&map.entities[index].guid)
}

/// `steps` random edits of `map`, as `side` makes them: moves, distances, deletions, additions (at places the two sides share at
/// times) and new parents.
fn edit(map: &mut CMapData, rng: &mut Rng, side: usize, steps: usize) {
    let mut added = 0;
    for _ in 0..steps {
        if map.entities.is_empty() {
            break;
        }
        let index = rng.below(map.entities.len());
        let of_base = map.entities[index].position.x < 1000.0;
        match rng.below(6) {
            0 if of_base => {
                let to = [rng.below(90) as f32, rng.below(90) as f32, 0.0];
                moved(map, index, to);
            }
            1 => map.entities[index].lod_dist = [50.0, 150.0, 250.0][rng.below(3)],
            2 => remove_entity(map, index).unwrap(),
            3 => {
                let donor = map.entities[index].clone();
                let shared = rng.below(4) == 0;
                let x = 1000.0
                    + match shared {
                        true => rng.below(3) as f32,
                        false => 10.0 + (side * 100 + added) as f32,
                    };
                if map.entities.iter().any(|e| e.position.x == x) {
                    continue;
                }
                let pose = Pose {
                    position: Vec3::new(x, 0.0, 0.0),
                    ..pose_of(&donor)
                };
                let at = add_entity(map, &donor, Name::from_text("prop_new"), pose).unwrap();
                map.entities[at].guid = 100 + added as u32;
                added += 1;
                if rng.below(2) == 0 {
                    let parents: Vec<usize> = (0..map.entities.len())
                        .filter(|&i| i != at && is_parent(map, i))
                        .collect();
                    if !parents.is_empty() {
                        let parent = parents[rng.below(parents.len())];
                        map.entities[at].parent_index = parent as i32;
                        map.entities[parent].num_children += 1;
                    }
                }
            }
            4 if !is_parent(map, index) => {
                if let Ok(old) = usize::try_from(map.entities[index].parent_index) {
                    map.entities[old].num_children -= 1;
                }
                let parents: Vec<usize> = (0..map.entities.len())
                    .filter(|&i| i != index && is_parent(map, i))
                    .collect();
                if rng.below(4) == 0 || parents.is_empty() {
                    map.entities[index].parent_index = -1;
                } else {
                    let parent = parents[rng.below(parents.len())];
                    map.entities[index].parent_index = parent as i32;
                    map.entities[parent].num_children += 1;
                }
            }
            _ => {}
        }
    }
}

/// What the merge holds when every conflict is answered for ours, from what the three files state of each entity alone.
fn expected(base: &CMapData, ours: &CMapData, theirs: &CMapData) -> BTreeMap<Who, State> {
    let (b, o, t) = (model(base), model(ours), model(theirs));
    let mut out: BTreeMap<Who, State> = BTreeMap::new();
    // Losing a parent the side deleted is no change of the child.
    let parent_changed = |was: &State, now: &State, side: &BTreeMap<Who, State>| {
        now.parent != was.parent
            && !(now.parent.is_none() && was.parent.is_some_and(|p| !side.contains_key(&p)))
    };
    let changed = |was: &State, now: &State, side: &BTreeMap<Who, State>| {
        was.position != now.position || was.lod != now.lod || parent_changed(was, now, side)
    };
    for (who, was) in &b {
        match (o.get(who), t.get(who)) {
            // Ours deleted it: gone, also where theirs changed it, since the conflict takes ours.
            (None, _) => {}
            // Theirs deleted it: gone, unless ours changed it and the conflict keeps ours.
            (Some(mine), None) => {
                if changed(was, mine, &o) {
                    out.insert(*who, mine.clone());
                }
            }
            (Some(mine), Some(yours)) => {
                let pick = |m: bool, y: bool| if m || !y { mine } else { yours };
                let state = State {
                    position: pick(
                        was.position != mine.position,
                        was.position != yours.position,
                    )
                    .position,
                    lod: pick(was.lod != mine.lod, was.lod != yours.lod).lod,
                    parent: if parent_changed(was, mine, &o) {
                        mine.parent
                    } else if parent_changed(was, yours, &t) {
                        yours.parent
                    } else {
                        was.parent
                    },
                };
                out.insert(*who, state);
            }
        }
    }
    // An entity each added at one place is a double, of which ours is kept.
    for side in [&o, &t] {
        for (who, state) in side {
            if matches!(who, Who::Added(_)) && !out.contains_key(who) {
                out.insert(*who, state.clone());
            }
        }
    }
    // A parent that is not in the merged map is no parent.
    let held: Vec<Who> = out.keys().copied().collect();
    for state in out.values_mut() {
        if state.parent.is_some_and(|p| !held.contains(&p)) {
            state.parent = None;
        }
    }
    out
}

#[test]
fn random_edits_of_two_sides_merge_to_what_a_model_of_the_three_files_says_with_links_counts_and_guids_right(
) {
    let base = a_base_of_parents();
    sound(&base);
    let mut conflicts = 0;
    for seed in 1..=rounds() {
        let mut rng = Rng::of(seed);
        let (mut ours, mut theirs) = (base.clone(), base.clone());
        let steps = 1 + rng.below(6);
        edit(&mut ours, &mut rng, 0, steps);
        let steps = 1 + rng.below(6);
        edit(&mut theirs, &mut rng, 1, steps);
        sound(&ours);
        sound(&theirs);

        let merge = merge_maps(&base, &ours, &theirs, &mut |_| Take::Ours)
            .unwrap_or_else(|why| panic!("seed {seed}: {why}"));
        conflicts += merge.report.conflicts.len();
        let merged = merge.merged.expect("ours was taken");
        assert_eq!(problems(&merged), Vec::<String>::new(), "seed {seed}");
        assert_eq!(miscounted(&merged), Vec::new(), "seed {seed}");
        assert_eq!(
            model(&merged),
            expected(&base, &ours, &theirs),
            "seed {seed}: {:?}",
            merge.report.conflicts
        );
        let mut guids: Vec<u32> = merged.entities.iter().map(|e| e.guid).collect();
        guids.sort_unstable();
        guids.dedup();
        assert_eq!(
            guids.len(),
            merged.entities.len(),
            "seed {seed}: a guid twice"
        );

        // In the other order under the mirrored policy it is the same file.
        let mirror = merge_maps(&base, &theirs, &ours, &mut |_| Take::Theirs).unwrap();
        assert_eq!(
            mirror.merged.unwrap().to_bytes().unwrap(),
            merged.to_bytes().unwrap(),
            "seed {seed}"
        );
        // And each side merged with the base itself is that side, whichever of the two it is called.
        for side in [&ours, &theirs] {
            let saved = side.to_bytes().unwrap();
            let alone = merge_maps(&base, side, &base.clone(), &mut |_| Take::Stop).unwrap();
            assert_eq!(
                alone.merged.map(|m| m.to_bytes().unwrap()),
                Some(saved.clone()),
                "seed {seed}"
            );
            let alone = merge_maps(&base, &base.clone(), side, &mut |_| Take::Stop).unwrap();
            assert_eq!(
                alone.merged.map(|m| m.to_bytes().unwrap()),
                Some(saved),
                "seed {seed}"
            );
        }
    }
    assert!(conflicts > 0, "the cases hold conflicts too");
}

/// Twin parents with children, twin props, and entities that state the guid nought, as shipped maps do.
fn a_base_of_twins() -> CMapData {
    map(&[
        Entity {
            children: 3,
            ..Entity::at("lod_a", [0.0, 0.0, 0.0], 7)
        },
        Entity {
            children: 2,
            ..Entity::at("lod_a", [10.0, 0.0, 0.0], 7)
        },
        Entity {
            parent: 0,
            ..Entity::at("hd", [0.0, 1.0, 0.0], 0)
        },
        Entity {
            parent: 0,
            ..Entity::at("hd", [0.0, 2.0, 0.0], 0)
        },
        Entity {
            parent: 1,
            ..Entity::at("hd", [10.0, 1.0, 0.0], 0)
        },
        Entity {
            parent: 1,
            ..Entity::at("hd", [10.0, 2.0, 0.0], 0)
        },
        Entity {
            parent: 0,
            ..Entity::at("hd2", [0.0, 3.0, 0.0], 3)
        },
        Entity::at("prop", [50.0, 5.0, 0.0], 4),
        Entity::at("prop", [51.0, 5.0, 0.0], 4),
        Entity::at("prop", [52.0, 5.0, 0.0], 5),
    ])
}

/// `steps` random edits of a map of twins: the edits of [`edit`], and guids given anew, entities that change places in the file, and
/// counts of children that are not the children.
fn edit_twins(map: &mut CMapData, rng: &mut Rng, side: usize, steps: usize) {
    let mut added = 0;
    let is_lod =
        |entity: &CEntityDefAny| entity.archetype_name.hash == Name::from_text("lod_a").hash;
    for _ in 0..steps {
        if map.entities.is_empty() {
            break;
        }
        let index = rng.below(map.entities.len());
        match rng.below(8) {
            0 => {
                let to = [rng.below(20) as f32, rng.below(20) as f32, 0.0];
                moved(map, index, to);
            }
            1 => map.entities[index].lod_dist = [50.0, 150.0, 250.0][rng.below(3)],
            2 => remove_entity(map, index).unwrap(),
            3 => {
                let donor = map.entities[index].clone();
                let x = 1000.0
                    + match rng.below(3) == 0 {
                        true => rng.below(3) as f32,
                        false => 10.0 + (side * 100 + added) as f32,
                    };
                let pose = Pose {
                    position: Vec3::new(x, 0.0, 0.0),
                    ..pose_of(&donor)
                };
                let name = ["prop_new", "prop", "hd", "lod_a"][rng.below(4)];
                let at = add_entity(map, &donor, Name::from_text(name), pose).unwrap();
                if rng.below(2) == 0 {
                    map.entities[at].guid = [0, 4, 7, 100][rng.below(4)];
                }
                added += 1;
                if rng.below(2) == 0 {
                    let parents: Vec<usize> = (0..map.entities.len())
                        .filter(|&i| {
                            i != at && is_lod(&map.entities[i]) && map.entities[i].parent_index < 0
                        })
                        .collect();
                    if !parents.is_empty() {
                        let parent = parents[rng.below(parents.len())];
                        map.entities[at].parent_index = parent as i32;
                        map.entities[parent].num_children += 1;
                    }
                }
            }
            4 if !is_lod(&map.entities[index]) => {
                if let Ok(old) = usize::try_from(map.entities[index].parent_index) {
                    map.entities[old].num_children -= 1;
                }
                let parents: Vec<usize> = (0..map.entities.len())
                    .filter(|&i| i != index && is_lod(&map.entities[i]))
                    .collect();
                if rng.below(4) == 0 || parents.is_empty() {
                    map.entities[index].parent_index = -1;
                } else {
                    let parent = parents[rng.below(parents.len())];
                    map.entities[index].parent_index = parent as i32;
                    map.entities[parent].num_children += 1;
                }
            }
            5 => map.entities[index].guid = [0, 3, 4, 5, 7, 9, 100][rng.below(7)],
            6 => {
                let other = rng.below(map.entities.len());
                map.entities.swap(index, other);
                for entity in &mut map.entities {
                    if entity.parent_index == index as i32 {
                        entity.parent_index = other as i32;
                    } else if entity.parent_index == other as i32 {
                        entity.parent_index = index as i32;
                    }
                }
            }
            _ => map.entities[index].num_children += u32::from(rng.below(5) == 0),
        }
    }
}

#[test]
fn random_edits_of_a_map_of_twins_merge_to_a_valid_map_that_is_the_same_in_the_other_order_and_a_side_with_the_base_is_that_side(
) {
    let base = a_base_of_twins();
    sound(&base);
    let (mut merges, mut with_conflicts) = (0, 0);
    for seed in 1..=rounds() {
        let mut rng = Rng::of(seed);
        let (mut ours, mut theirs) = (base.clone(), base.clone());
        let steps = 1 + rng.below(5);
        edit_twins(&mut ours, &mut rng, 0, steps);
        let steps = 1 + rng.below(5);
        edit_twins(&mut theirs, &mut rng, 1, steps);
        if !problems(&ours).is_empty() || !problems(&theirs).is_empty() {
            continue;
        }
        for (take, mirrored) in [(Take::Ours, Take::Theirs), (Take::Theirs, Take::Ours)] {
            let one = merge_maps(&base, &ours, &theirs, &mut |_| take);
            let other = merge_maps(&base, &theirs, &ours, &mut |_| mirrored);
            let (one, other) = match (one, other) {
                (Ok(one), Ok(other)) => (one, other),
                (Err(_), Err(_)) => continue,
                (one, other) => panic!(
                    "seed {seed} {take:?}: merged one way and refused the other: {:?} / {:?}",
                    one.err().map(|e| e.to_string()),
                    other.err().map(|e| e.to_string())
                ),
            };
            merges += 1;
            with_conflicts += usize::from(!one.report.conflicts.is_empty());
            let merged = one.merged.expect("a side was taken");
            assert_eq!(
                problems(&merged),
                Vec::<String>::new(),
                "seed {seed} {take:?}"
            );
            assert_eq!(
                other.merged.unwrap().to_bytes().unwrap(),
                merged.to_bytes().unwrap(),
                "seed {seed} {take:?}"
            );
        }
        // A side merged with the base itself is that side to the byte, once its twins, if it moved any, are answered for it.
        for side in [&ours, &theirs] {
            let alone = merge_maps(&base, side, &base.clone(), &mut |_| Take::Ours)
                .unwrap_or_else(|why| panic!("seed {seed}: {why}"));
            assert_eq!(
                alone.merged.map(|m| m.to_bytes().unwrap()),
                Some(side.to_bytes().unwrap()),
                "seed {seed}: {:?}",
                alone.report.conflicts
            );
            // Unanswered, it stops only for twins, and says which.
            let asked = merge_maps(&base, side, &base.clone(), &mut |_| Take::Stop).unwrap();
            assert!(
                (asked.report.conflicts.iter())
                    .all(|c| c.what == Disagreement::Ambiguous && !c.ours.is_empty()),
                "seed {seed}: {:?}",
                asked.report.conflicts
            );
            assert_eq!(
                asked.merged.is_none(),
                !asked.report.conflicts.is_empty(),
                "seed {seed}"
            );
        }
    }
    assert!(
        merges > 100 && with_conflicts > 50,
        "{merges} merges, {with_conflicts} with conflicts"
    );
}

#[test]
fn files_with_broken_links_counts_and_places_merge_to_a_valid_map_or_are_refused_and_never_panic() {
    let twins = a_base_of_twins();
    let (mut merged_or_stopped, mut refused) = (0, 0);
    for seed in 1..=rounds() {
        let mut rng = Rng::of(seed);
        let mut files = [twins.clone(), twins.clone(), twins.clone()];
        for (which, file) in files.iter_mut().enumerate() {
            let steps = if which == 0 {
                rng.below(2)
            } else {
                1 + rng.below(4)
            };
            edit_twins(file, &mut rng, which, steps);
            for _ in 0..rng.below(3) {
                if file.entities.is_empty() {
                    break;
                }
                let index = rng.below(file.entities.len());
                match rng.below(4) {
                    0 => {
                        file.entities[index].parent_index =
                            [-5, -1, 0, 3, 40, i32::MAX, index as i32][rng.below(7)];
                    }
                    1 => file.entities[index].flags ^= 1 << 3,
                    2 => file.entities[index].num_children = [0, 1, 7, u32::MAX][rng.below(4)],
                    _ => {
                        file.entities[index].position.x =
                            [f32::NAN, f32::INFINITY, -0.0, 1e30][rng.below(4)];
                    }
                }
            }
        }
        let [base, ours, theirs] = files;
        for take in [Take::Ours, Take::Theirs, Take::Stop] {
            match merge_maps(&base, &ours, &theirs, &mut |_| take) {
                Ok(merge) => {
                    merged_or_stopped += 1;
                    if let Some(merged) = merge.merged {
                        assert_eq!(
                            problems(&merged),
                            Vec::<String>::new(),
                            "seed {seed} {take:?}"
                        );
                    }
                }
                Err(why) => {
                    refused += 1;
                    assert!(!why.to_string().is_empty());
                }
            }
        }
    }
    assert!(
        merged_or_stopped > 100 && refused > 20,
        "{merged_or_stopped} merged or stopped, {refused} refused"
    );
}

#[test]
fn random_edits_of_a_type_file_merge_to_the_same_file_in_the_other_order_and_a_side_with_the_base_is_that_side(
) {
    let names: Vec<String> = (0..8).map(|i| format!("prop_{i}")).collect();
    let list: Vec<Archetype<'_>> = (names.iter())
        .map(|n| Archetype::new(n, [-1.0; 3], [1.0; 3]))
        .collect();
    let base = types(&list);
    let (mut merges, mut refusals) = (0, 0);
    for seed in 1..=rounds() {
        let mut rng = Rng::of(seed);
        let mut sides = [base.clone(), base.clone()];
        for side in sides.iter_mut() {
            for _ in 0..1 + rng.below(5) {
                if side.archetypes.is_empty() {
                    break;
                }
                let index = rng.below(side.archetypes.len());
                match rng.below(7) {
                    0 => side.archetypes[index].lod_dist = [10.0, 20.0, 30.0][rng.below(3)],
                    1 => side.archetypes[index].flags = rng.below(3) as u32,
                    2 => {
                        side.archetypes.remove(index);
                    }
                    3 => {
                        let mut new = side.archetypes[index].clone();
                        new.name = HashedString::new(["new_a", "new_b", "prop_3"][rng.below(3)]);
                        new.bs_radius = rng.below(2) as f32;
                        side.archetypes.push(new);
                    }
                    4 => {
                        side.archetypes[index].name =
                            HashedString::new(["prop_1", "ren_a", "ren_b"][rng.below(3)]);
                    }
                    5 => {
                        let other = rng.below(side.archetypes.len());
                        side.archetypes.swap(index, other);
                    }
                    _ => side.archetypes[index].special_attribute = rng.below(3) as u32,
                }
            }
        }
        let [ours, theirs] = sides;
        for (take, mirrored) in [(Take::Ours, Take::Theirs), (Take::Theirs, Take::Ours)] {
            let one = merge_types(&base, &ours, &theirs, &mut |_| take);
            let other = merge_types(&base, &theirs, &ours, &mut |_| mirrored);
            match (one, other) {
                (Ok(one), Ok(other)) => {
                    merges += 1;
                    assert_eq!(
                        one.merged.unwrap().to_bytes().unwrap(),
                        other.merged.unwrap().to_bytes().unwrap(),
                        "seed {seed} {take:?}"
                    );
                }
                (Err(_), Err(_)) => refusals += 1,
                (one, other) => panic!(
                    "seed {seed} {take:?}: merged one way and refused the other: {:?} / {:?}",
                    one.err().map(|e| e.to_string()),
                    other.err().map(|e| e.to_string())
                ),
            }
        }
        for side in [&ours, &theirs] {
            // A side that declares one name twice is a conflict of that name, which taking the side answers.
            let alone = merge_types(&base, side, &base.clone(), &mut |_| Take::Ours)
                .unwrap_or_else(|why| panic!("seed {seed}: {why}"));
            assert_eq!(
                alone.merged.map(|m| m.to_bytes().unwrap()),
                Some(side.to_bytes().unwrap()),
                "seed {seed}"
            );
        }
    }
    assert!(
        merges > 100 && refusals > 0,
        "{merges} merges, {refusals} refusals"
    );
}

/// A map of `count` entities of fifty archetypes, and two sides: ours deleted `out_of_ten` of each ten and changed a third of the
/// rest, and theirs changed a half and added `added` for each entity the map held.
fn a_large_map_and_two_sides(
    count: usize,
    out_of_ten: usize,
    added: usize,
) -> (CMapData, CMapData, CMapData) {
    let names: Vec<String> = (0..50).map(|i| format!("prop_{i}")).collect();
    let list: Vec<Entity<'_>> = (0..count)
        .map(|i| {
            Entity::at(
                &names[i % 50],
                [i as f32, (i % 97) as f32, 0.0],
                i as u32 + 1,
            )
        })
        .collect();
    let base = map(&list);
    let (mut ours, mut theirs) = (base.clone(), base.clone());
    for i in (0..count).step_by(3) {
        ours.entities[i].position.z = 3.0;
    }
    for i in (0..count).step_by(20) {
        theirs.entities[i].lod_dist = 60.0;
    }
    let gone: Vec<usize> = (0..count)
        .filter(|i| i % 10 < out_of_ten && i % 20 != 0)
        .collect();
    // No entity here names a parent, so the ones left are the map a removal of each would leave.
    let mut index = 0;
    ours.entities.retain(|_| {
        index += 1;
        gone.binary_search(&(index - 1)).is_err()
    });
    let donor = theirs.entities[0].clone();
    for k in 0..count * added {
        let mut new = donor.clone();
        new.archetype_name = HashedString::new("prop_new");
        new.position.x = -10.0 - k as f32;
        new.guid = (count + k) as u32 + 1;
        theirs.entities.push(new);
    }
    (base, ours, theirs)
}

/// The time of a merge does not grow with the square of the entities: four times the entities take well under sixteen times as
/// long, both where a side added many and where a side deleted most.
///
/// Measured 2026-11-25 on an Apple M5 Ultra while other lanes built on it, the least of three merges under the test profile:
/// 0.23 s for a base of 2,000 entities and 0.91 s for one of 8,000 where theirs added four for each and ours deleted half, and
/// 0.06 s for 6,000 and 0.25 s for 24,000 where ours deleted nine of ten. With one pass over the map for each entity deleted, the
/// first pair took 0.27 s and 1.76 s, and with one for each entity added 0.30 s and 2.07 s: either fails this test.
#[test]
fn a_merge_of_four_times_the_entities_takes_nowhere_near_sixteen_times_as_long() {
    let timed = |count: usize, out_of_ten: usize, added: usize| -> Duration {
        let (base, ours, theirs) = a_large_map_and_two_sides(count, out_of_ten, added);
        let mut least = Duration::MAX;
        for _ in 0..3 {
            let started = Instant::now();
            let merge = merge_maps(&base, &ours, &theirs, &mut |_| Take::Stop).unwrap();
            least = least.min(started.elapsed());
            assert_eq!(merge.report.conflicts, Vec::<Conflict>::new());
            let merged = merge.merged.unwrap();
            assert_eq!(merged.entities.len(), ours.entities.len() + count * added);
        }
        least
    };
    for (what, small, out_of_ten, added) in [("added", 2_000, 5, 4), ("deleted", 6_000, 9, 0)] {
        let (few, many) = (
            timed(small, out_of_ten, added),
            timed(small * 4, out_of_ten, added),
        );
        println!(
            "{what}: {small} entities in {few:?}, {} in {many:?}",
            small * 4
        );
        assert!(
            many < few * 6,
            "{what}: {small} entities merge in {few:?} and {} in {many:?}: more than six times as long",
            small * 4
        );
    }
}
