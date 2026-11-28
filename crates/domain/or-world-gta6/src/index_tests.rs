use super::scan::{container_of, Shard};
use super::*;

fn archetype(word: u64, model: u64, dictionary: u64, name: u32) -> ArchetypeRow {
    ArchetypeRow {
        word,
        model,
        dictionary,
        name,
        lod_dist: 100.0,
        centre: [0.0, 0.0, 1.0],
        radius: 2.0,
        flags: 0,
        hours: ALL_HOURS,
        load_flags: 0,
        pad: 0,
    }
}

fn body(word: u64, archive: u32, offset: u64) -> BodyRow {
    BodyRow {
        word,
        offset,
        archive,
        pad: 0,
    }
}

/// A map of the base game named `name`, held back as `held` says.
fn map(name: u32, held: u32) -> MapRow {
    MapRow {
        archive: 0,
        name,
        block: 7,
        flags: 0,
        content_flags: 1,
        held,
        parents_from: 0,
        parents_len: 0,
        group: 0,
        modes: 1,
        offset: u64::from(name) * 16,
        boxes: [
            -10.0, -10.0, -10.0, 10.0, 10.0, 10.0, -5.0, -5.0, -5.0, 5.0, 5.0, 5.0,
        ],
    }
}

/// An index of two maps, three archetypes and the bodies that draw two of them.
fn made_up(archive: PathBuf) -> WorldIndex {
    let mut second = MapRow {
        flags: 1,
        group: 77,
        ..map(22, 2)
    };
    (second.parents_from, second.parents_len) = (0, 1);
    WorldIndex {
        archives: vec![archive],
        maps: vec![map(11, 0), second],
        parents: vec![11],
        archetypes: vec![
            archetype(300, 900, 0, 1),
            ArchetypeRow {
                hours: 0x0000_00F0,
                load_flags: 0x0001_0081,
                ..archetype(100, 800, 0, 2)
            },
            archetype(200, 0, 700, 3),
        ],
        models: vec![body(900, 0, 0x900), body(800, 0, 0x800)],
        dictionaries: vec![body(700, 0, 0x700)],
        interiors: vec![body(600, 0, 0x650), body(600, 0, 0x600)],
        slod: vec![0, 300],
        switches: vec![GroupRow {
            word: 0xABC,
            id: 77,
            pad: 0,
            hours: [0b101; 8],
            inferred: [0b110; 8],
        }],
        report: Report {
            maps: 2,
            at_rest: 1,
            story: 5,
            permanent_found: 9,
            ..Report::default()
        },
        ..WorldIndex::default()
    }
    .finish()
}

#[test]
fn an_archetype_finds_its_model_by_word_and_a_dictionary_s_by_its_own_name() {
    let index = made_up(PathBuf::from("levels.rpf"));
    let plain = index.archetype(100).expect("the archetype of word 100");
    assert_eq!(
        index.model(plain),
        Some(ModelRef {
            archive: 0,
            offset: 0x800,
            entry: None
        })
    );
    let in_dictionary = index.archetype(200).expect("the archetype of word 200");
    assert_eq!(
        index.model(in_dictionary),
        Some(ModelRef {
            archive: 0,
            offset: 0x700,
            entry: Some(3)
        })
    );
    assert!(index.archetype(101).is_none());
    // An interior names no model, and a word no body states draws nothing.
    assert_eq!(index.model(&archetype(1, 0, 0, 9)), None);
    assert_eq!(index.model(&archetype(1, 555, 0, 9)), None);
    assert_eq!(index.map_named(22), Some(1));
    assert_eq!(index.parents_of(&index.maps[1]), &[11]);
    // The camera starts over the maps that stream at rest alone.
    assert_eq!(index.middle(), Some(Vec3::ZERO));
}

#[test]
fn the_later_of_two_bodies_that_state_one_archetype_stands() {
    let mut index = WorldIndex {
        archetypes: vec![
            archetype(5, 1, 0, 1),
            archetype(4, 9, 0, 1),
            archetype(5, 2, 0, 1),
        ],
        ..WorldIndex::default()
    };
    index = index.finish();
    assert_eq!(index.archetype(5).map(|a| a.model), Some(2));
    assert_eq!(index.report.archetypes, 2);
}

#[test]
fn a_map_streams_when_its_mode_is_on_and_a_script_managed_one_when_its_group_is_too() {
    let at_rest = Enabled::default();
    assert_eq!((at_rest.modes, at_rest.groups.len()), (1, 0));
    let story = MapRow {
        modes: 2,
        ..map(1, 0)
    };
    let managed = MapRow {
        group: 77,
        ..map(2, 2)
    };
    let both = MapRow {
        modes: 3,
        ..map(3, 0)
    };
    let rows = [map(0, 0), story, managed, both, map(4, 1), map(5, 3)];
    let allowed = |enabled: &Enabled| rows.map(|row| enabled.allows(&row));
    assert_eq!(allowed(&at_rest), [true, false, false, true, false, false]);
    let with_story = Enabled {
        modes: 3,
        groups: vec![],
    };
    assert_eq!(
        allowed(&with_story),
        [true, true, false, true, false, false]
    );
    let story_alone = Enabled {
        modes: 2,
        groups: vec![77],
    };
    assert_eq!(
        allowed(&story_alone),
        [false, true, false, true, false, false],
        "a group of a mode that is off stays off"
    );
    let with_group = Enabled {
        modes: 1,
        groups: vec![5, 77],
    };
    assert_eq!(
        allowed(&with_group),
        [true, false, true, true, false, false]
    );
    // A map no pack registers, and one that holds nothing, stream under nothing.
    let all = Enabled {
        modes: 7,
        groups: vec![77, 0],
    };
    assert_eq!(allowed(&all), [true, true, true, true, false, false]);
}

#[test]
fn the_mount_gives_a_map_its_modes_its_hold_and_its_group() {
    let fact = |flags, holds| MapFact {
        archive: 0,
        container: None,
        name: 0,
        block: 0,
        flags,
        holds,
    };
    let row = |name, flags, group| MapRow {
        flags,
        group,
        modes: 9,
        ..map(name, 9)
    };
    let mut maps = [
        row(1, 0, 0),
        row(2, 0, 0),
        row(3, 1, 55),
        row(4, 3, 0),
        row(5, 2, 0),
        row(6, 0, 66),
        row(7, 1, 55),
    ];
    let facts = [
        fact(0, true),
        fact(0, true),
        fact(1, true),
        fact(3, false),
        fact(2, false),
        fact(0, true),
        fact(1, true),
    ];
    place(&mut maps, &facts, &[1, 0, 1, 4, 2, 3, 0]);
    let got: Vec<(u32, u32, u32)> = maps.iter().map(|m| (m.modes, m.held, m.group)).collect();
    assert_eq!(
        got,
        [
            (1, 0, 0),
            (0, 1, 0),
            (1, 2, 55),
            // A script-managed map that states no group is a group of its own, by its name.
            (4, 2, 4),
            (2, 3, 0),
            // A group stated by a map that is not script managed is no group.
            (3, 0, 0),
            (0, 1, 55),
        ]
    );
    let report = counted(&maps);
    assert_eq!((report.maps, report.registered, report.at_rest), (7, 3, 2));
    assert_eq!((report.script_managed, report.empty), (1, 0));
    assert_eq!((report.story, report.minigames), (2, 1));

    let index = WorldIndex {
        maps: maps.to_vec(),
        ..WorldIndex::default()
    }
    .finish();
    assert_eq!(
        index.groups(),
        [
            Group {
                id: 4,
                name: 4,
                maps: 1,
                word: 0,
                hours: [0; 8],
                inferred: [0; 8],
            },
            Group {
                id: 55,
                name: 55,
                maps: 2,
                word: 0,
                hours: [0; 8],
                inferred: [0; 8],
            }
        ]
    );
    assert_eq!(index.report.groups, 2);
}

#[test]
fn a_body_lies_in_the_last_nested_archive_whose_names_have_not_ended_it() {
    // The archive's own header at 0, one nested archive ended at 500, one that states no end.
    let archives = [(0, None), (100, Some(500)), (1000, None)];
    assert_eq!(container_of(&archives, 50), None);
    assert_eq!(container_of(&archives, 100), Some(100));
    assert_eq!(container_of(&archives, 499), Some(100));
    assert_eq!(
        container_of(&archives, 500),
        None,
        "past the names that end it"
    );
    assert_eq!(container_of(&archives, 4000), Some(1000));
    assert_eq!(container_of(&[], 4000), None);
}

#[test]
fn a_dictionary_word_is_the_one_body_that_holds_a_drawable_under_every_name() {
    let archetypes = [
        archetype(1, 0, 70, 10),
        archetype(2, 0, 70, 11),
        archetype(3, 0, 80, 12),
        archetype(4, 0, 90, 13),
        archetype(5, 500, 0, 14),
    ];
    let mut keys: HashMap<u32, Vec<BodyRow>> = HashMap::new();
    // Body 0x100 holds 10 and 11; body 0x200 holds 10 alone; 12 is held by two bodies; 13 by none.
    keys.insert(10, vec![body(0, 0, 0x100), body(0, 0, 0x200)]);
    keys.insert(11, vec![body(0, 0, 0x100)]);
    keys.insert(12, vec![body(0, 0, 0x300), body(0, 1, 0x300)]);
    let mut found = dictionaries(&archetypes, &keys);
    found.sort_unstable_by_key(|b| b.word);
    assert_eq!(found, vec![body(70, 0, 0x100)]);
}

#[test]
fn what_a_range_found_before_the_end_of_the_last_body_before_it_is_dropped() {
    let mut shard = Shard {
        models: vec![(1, 90), (2, 120), (3, 300)],
        types: vec![95, 130],
        archives: vec![80, 150],
        ends: vec![(80, 100), (150, 400)],
        offsets: vec![80, 90, 120, 150, 300],
        ..Shard::default()
    };
    shard.cut_before(100);
    assert_eq!(shard.models, vec![(2, 120), (3, 300)]);
    assert_eq!(shard.types, vec![130]);
    assert_eq!(shard.archives, vec![150]);
    assert_eq!(shard.ends, vec![(150, 400)]);
    assert_eq!(shard.offsets, vec![120, 150, 300]);
    // A nested archive with the end its names give it, and one whose names were not found.
    shard.archives.push(140);
    assert_eq!(shard.nested(), [(140, None), (150, Some(400))]);
}
