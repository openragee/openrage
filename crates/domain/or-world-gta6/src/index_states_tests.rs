use or_io::gta6::world_state::{Assumed, Condition, Config, Rule, RuleSet};

use super::super::{tests_support::index_with, MapRow};
use super::*;

const OPEN: u64 = 0xA1;
const CLOSED: u64 = 0xB2;
const STORY_ONLY: u64 = 0xC3;

/// A selector that opens a door by day, and by night from 22 to 6 closes it and opens the other.
fn door() -> Selector {
    Selector {
        key: 1,
        default: 10,
        configs: vec![
            Config {
                key: 10,
                on: vec![OPEN],
                ..Config::default()
            },
            Config {
                key: 20,
                off: vec![OPEN],
                state2: vec![CLOSED],
                ..Config::default()
            },
        ],
        rule_sets: vec![RuleSet {
            priority: 10,
            rules: vec![Rule {
                config: 20,
                condition: Condition::TimeOfDay { start: 22, end: 6 },
            }],
        }],
    }
}

/// A selector of the story that turns on a group of its own and turns the open door off.
fn story() -> Selector {
    Selector {
        key: 2,
        default: 30,
        configs: vec![Config {
            key: 30,
            on: vec![STORY_ONLY],
            off: vec![OPEN],
            ..Config::default()
        }],
        rule_sets: Vec::new(),
    }
}

/// `selector` of the mode bit `mode` in a layer the level loads.
fn at_start(mode: u8, selector: Selector) -> Placed {
    Placed {
        mode,
        layer: (7, 0),
        at_start: true,
        selector,
    }
}

#[test]
fn a_group_is_on_in_the_hours_the_selectors_of_the_modes_on_pick_it() {
    let words = [
        (100, OPEN),
        (200, CLOSED),
        (300, STORY_ONLY),
        (100, OPEN),
        (400, 0xD4),
    ];
    let rows = switches(
        &words,
        &[at_start(1, door()), at_start(2, story())],
        &Assumed::default(),
    );
    let ids: Vec<u32> = rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, [100, 200, 300, 400], "one row a group");
    let day = 0x003F_FFC0u32 & !(1 << 6);
    let night = !day & 0x00FF_FFFF;
    // Base alone: the door is open by day and closed by night.
    assert_eq!(
        [rows[0].hours[1], rows[1].hours[1], rows[2].hours[1]],
        [day, night, 0]
    );
    // With the story, its selector turns the open door off at every hour.
    assert_eq!(
        [rows[0].hours[3], rows[1].hours[3], rows[2].hours[3]],
        [0, night, 0x00FF_FFFF]
    );
    assert_eq!(rows[3].hours, [0; 8], "a group no selector names stays off");
    assert!(rows.iter().all(|r| r.hours[0] == 0), "no mode, no switch");
}

#[test]
fn only_the_layers_the_level_loads_start_loaded() {
    let body = |layer| WorldState {
        layer,
        selectors: vec![door()],
        places: Vec::new(),
    };
    let states = || vec![body(1), body(2), body(3)];
    // 1 the level loads, 2 something else loads when it asks, 3 no manifest names.
    let layers = [(1, true), (2, false)];
    let start: Vec<(u32, bool)> = (placed(states(), Some(&layers), 1, 9))
        .map(|p| (p.layer.1, p.at_start))
        .collect();
    assert_eq!(start, [(1, true), (2, false), (3, true)]);
    assert!(placed(states(), None, 1, 9).all(|p| p.at_start && p.layer.0 == 9));
}

#[test]
fn a_layer_a_picked_configuration_loads_switches_too() {
    // The door's day configuration loads layer 5, whose story selector turns the open door off.
    let mut opener = door();
    opener.configs[0].layers = vec![(7, 5)];
    let later = |at_start| Placed {
        mode: 1,
        layer: (7, 5),
        at_start,
        selector: story(),
    };
    let on = |placed: &[Placed], hour| {
        words_at(
            &placed.iter().collect::<Vec<_>>(),
            hour,
            &Assumed::default(),
        )
    };
    let chained = [at_start(1, opener.clone()), later(false)];
    assert_eq!(on(&chained, 12), HashSet::from([STORY_ONLY]));
    // By night the door's other configuration loads nothing, so the layer stays out.
    assert_eq!(on(&chained, 23), HashSet::from([CLOSED]));
    // A layer no configuration loads stays out at every hour.
    assert_eq!(
        on(&[at_start(1, door()), later(false)], 12),
        HashSet::from([OPEN])
    );
}

#[test]
fn a_new_game_settles_the_player_conditions() {
    let rule = |condition| Selector {
        rule_sets: vec![RuleSet {
            priority: 10,
            rules: vec![Rule {
                config: 20,
                condition,
            }],
        }],
        ..door()
    };
    // A rule that holds for a new game picks; one that does not leaves the default.
    let wanted = Condition::NewGame(false);
    let not_wanted = Condition::Not(Box::new(Condition::NewGame(false)));
    assert_eq!(rule(wanted).pick(12).map(|c| c.key), Some(10));
    assert_eq!(rule(not_wanted).pick(12).map(|c| c.key), Some(20));
}

/// A script-managed map of the group `group`.
fn managed(name: u32, group: u32) -> MapRow {
    MapRow {
        archive: 0,
        name,
        block: 0,
        flags: 1,
        content_flags: 0,
        held: 2,
        parents_from: 0,
        parents_len: 0,
        group,
        modes: 1,
        offset: u64::from(name),
        boxes: [0.0; 12],
    }
}

#[test]
fn the_index_says_which_groups_are_on_at_an_hour_and_keeps_their_words() {
    let maps = vec![managed(1, 100), managed(2, 200), managed(3, 300)];
    let mut index = index_with(Vec::new(), &[], maps, Vec::new());
    let words = [(100, OPEN), (200, CLOSED), (300, STORY_ONLY)];
    index.switches = switches(
        &words,
        &[at_start(1, door()), at_start(2, story())],
        &Assumed::default(),
    );
    let index = index.finish();
    assert_eq!(index.groups()[0].word, OPEN);
    assert_eq!(index.default_on_in(12.0, 1, true), [100]);
    assert_eq!(index.default_on_in(23.5, 1, true), [200]);
    assert_eq!(index.default_on_in(12.0, 3, true), [300]);
    assert_eq!(index.default_on_in(12.0, 0b111, true), [300]);
    // An hour past the day comes round again.
    assert_eq!(index.default_on_in(36.0, 1, true), [100]);
    assert_eq!(index.default_on_in(-1.0, 1, true), [200]);
}

#[test]
fn a_request_taken_of_a_fresh_game_is_kept_apart_from_what_the_data_proves() {
    let dir = std::env::temp_dir().join(format!("or-fresh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let file = dir.join("fresh_game.json");
    // The script asks for the night configuration from 12 to 19, above the rule set.
    let text = r#"{"flags_clear": true, "requests": [{"selector": "1", "config": "0x14", "priority": 1000, "hours": [12, 19], "evidence": "made up"}]}"#;
    std::fs::write(&file, text).expect("written");
    let (assumed, word) = assumed(&file);
    assert_ne!(word, 0);
    assert_eq!(assumed.flags, Some(HashSet::new()));
    assert_eq!(assumed.requests[0].hours, 0x000F_F000);
    let rows = switches(
        &[(100, OPEN), (200, CLOSED)],
        &[at_start(1, door())],
        &assumed,
    );
    let base = |r: &GroupRow| (r.hours[1], r.inferred[1]);
    let day = 0x003F_FFC0u32 & !(1 << 6);
    assert_eq!(
        base(&rows[0]),
        (day, day & !0x000F_F000),
        "the data alone keeps the door open by day"
    );
    assert_eq!(
        base(&rows[1]),
        (!day & 0x00FF_FFFF, (!day & 0x00FF_FFFF) | 0x000F_F000)
    );
    let maps = vec![managed(1, 100), managed(2, 200)];
    let mut index = index_with(Vec::new(), &[], maps, Vec::new());
    index.switches = rows;
    let index = index.finish();
    assert_eq!(index.default_on_in(13.0, 1, true), [100]);
    assert_eq!(index.default_on_in(13.0, 1, false), [200]);
    assert!(index.groups()[0].by_inference(13.0, 1));
    assert!(!index.groups()[0].by_inference(9.0, 1));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        super::assumed(&file),
        (Assumed::default(), 0),
        "no file, nothing taken"
    );
}
