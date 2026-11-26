//! Each rule of an interior fires on an interior built to break it and on no other, and none fires on a sound one; `check` says them
//! among the findings of a map set, at the interior, with the severity of the rule and set aside by a waiver.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this.

use std::collections::BTreeSet;
use std::path::Path;

use or_maps::import::{add_files, Incoming};
use or_maps::interior::{interiors_of, InFile, INTERIOR_RULES};
use or_maps::operation::NOT_A_ROOM;
use or_maps::sample::{
    types_file_with_interiors, Entity, EntitySet, Interior, Portal, Room, VILLA,
};
use or_maps::{check, inspect, rule_of, Depth, Maps, Severity, Waiver};

/// The text of a name of the sample villa, which a file keeps the hash of alone.
fn named(hash: u32) -> Option<String> {
    [
        "villa",
        "villa_party",
        "villa_night",
        "prop_balloon",
        "prop_crate",
    ]
    .into_iter()
    .find(|name| or_io::string_hash(name.as_bytes()) == hash)
    .map(String::from)
}

/// The one interior of a type file made of `interior`.
fn read(interior: Interior<'_>) -> InFile {
    let bytes = types_file_with_interiors("villa_types", &[], &[interior]);
    let mut interiors = interiors_of(&bytes, Some(&named)).expect("the type file reads");
    assert_eq!(interiors.len(), 1);
    interiors.remove(0)
}

/// The rules that fire on `interior`, each with its sentence.
fn found(interior: Interior<'_>) -> Vec<(&'static str, String)> {
    (read(interior).listed.findings.into_iter())
        .map(|found| (found.code, found.message))
        .collect()
}

const QUAD: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [2.0, 0.0, 0.0],
    [2.0, 0.0, 2.0],
    [0.0, 0.0, 2.0],
];

fn portal(from: u32, to: u32) -> Portal<'static> {
    Portal {
        from,
        to,
        corners: QUAD,
        flags: 0,
        attached: &[],
    }
}

/// The villa with the room `at` as `change` makes it.
fn rooms_with(at: usize, change: impl Fn(Room<'static>) -> Room<'static>) -> Vec<Room<'static>> {
    let mut rooms = VILLA.rooms.to_vec();
    rooms[at] = change(rooms[at]);
    rooms
}

/// The party with its crate in room 4, where the villa holds four rooms.
const MISPLACED: &[EntitySet<'static>] = &[EntitySet {
    name: "villa_party",
    entities: &[
        (2, Entity::at("prop_balloon", [9.0, 2.0, 1.0], 20)),
        (4, Entity::at("prop_crate", [9.0, -2.0, -3.0], 21)),
    ],
}];

/// An interior built to break one rule, with the rule and what its sentence names.
struct Broken {
    code: &'static str,
    said: &'static str,
    interior: Interior<'static>,
}

/// ⚠ A rule with no case here fails `each_rule_fires_on_an_interior_built_to_break_it_and_on_no_other`.
fn cases() -> Vec<Broken> {
    let leak = |rooms: Vec<Room<'static>>| -> &'static [Room<'static>] { rooms.leak() };
    let mut portals = VILLA.portals.to_vec();
    portals.push(portal(2, 2));
    let folded: &'static [Portal<'static>] = portals.leak();

    let mut rooms = VILLA.rooms.to_vec();
    rooms.push(Room {
        name: "vault",
        min: [12.0, -5.0, -3.0],
        max: [15.0, 5.0, 0.0],
        attached: &[],
        portal_count: None,
    });
    let with_a_vault = leak(rooms);

    vec![
        Broken {
            code: "interior-portal-rooms-same",
            said: "1 portal(s) name one room on both sides: #3",
            interior: Interior {
                portals: folded,
                ..VILLA
            },
        },
        Broken {
            code: "interior-portal-count",
            said: "1 room(s) count another number of portals than name them: #2 counts 5 where 2 name it",
            interior: Interior {
                rooms: leak(rooms_with(2, |room| Room {
                    portal_count: Some(5),
                    ..room
                })),
                ..VILLA
            },
        },
        Broken {
            code: "interior-set-room-missing",
            said: "1 entity set(s) place an entity in a room the interior does not hold: set #0 places 1 in a room past the 4 the interior holds, the first in room 4",
            interior: Interior {
                entity_sets: MISPLACED,
                ..VILLA
            },
        },
        Broken {
            code: "interior-entity-in-two-rooms",
            said: "1 entity(ies) are attached to two rooms: #1 to rooms 1 and 2",
            interior: Interior {
                rooms: leak(rooms_with(2, |room| Room {
                    attached: &[2, 1],
                    ..room
                })),
                ..VILLA
            },
        },
        Broken {
            code: "interior-entity-unattached",
            said: "1 entity(ies) are attached to no room and no portal: #2",
            interior: Interior {
                rooms: leak(rooms_with(2, |room| Room {
                    attached: &[],
                    ..room
                })),
                ..VILLA
            },
        },
        Broken {
            code: "interior-room-unreachable",
            said: "1 room(s) are joined to the outside by no chain of portals: #4",
            interior: Interior {
                rooms: with_a_vault,
                ..VILLA
            },
        },
        Broken {
            code: "interior-sealed",
            said: "none of its 2 portal(s) leads to the outside, room 0",
            interior: Interior {
                portals: &VILLA.portals[1..],
                // The front door went with its portal, and stands in the hall.
                rooms: leak(rooms_with(1, |room| Room {
                    attached: &[0, 1, 4],
                    ..room
                })),
                ..VILLA
            },
        },
    ]
}

#[test]
fn no_rule_fires_on_a_sound_interior() {
    assert_eq!(found(VILLA), Vec::<(&str, String)>::new());
    // A location with bit 31 set is no room index, as six of each install are, and is held to no room.
    let flagged = Interior {
        entity_sets: &[EntitySet {
            name: "villa_party",
            entities: &[(
                NOT_A_ROOM | 41,
                Entity::at("prop_crate", [9.0, -2.0, -3.0], 21),
            )],
        }],
        ..VILLA
    };
    assert_eq!(found(flagged), Vec::<(&str, String)>::new());
}

#[test]
fn each_rule_fires_on_an_interior_built_to_break_it_and_on_no_other() {
    let mut covered = BTreeSet::new();
    for broken in cases() {
        covered.insert(broken.code);
        assert_eq!(
            found(broken.interior),
            [(broken.code, broken.said.to_string())],
            "{}",
            broken.code
        );
    }
    let rules: BTreeSet<&str> = INTERIOR_RULES.iter().map(|rule| rule.code).collect();
    assert_eq!(rules.len(), INTERIOR_RULES.len(), "a code names one rule");
    assert_eq!(
        covered, rules,
        "every rule has an interior built to break it"
    );
}

#[test]
fn a_room_cut_off_is_said_only_of_an_interior_that_opens_to_the_outside() {
    // The portal to the outside goes, and a room no portal names is there: the interior is sealed, and nothing is unreachable.
    let mut rooms = rooms_with(1, |room| Room {
        attached: &[0, 1, 4],
        ..room
    });
    rooms.push(Room {
        name: "vault",
        min: [12.0, -5.0, -3.0],
        max: [15.0, 5.0, 0.0],
        attached: &[],
        portal_count: None,
    });
    let sealed = Interior {
        portals: &VILLA.portals[1..],
        rooms: rooms.leak(),
        ..VILLA
    };
    let codes: Vec<&str> = found(sealed).into_iter().map(|(code, _)| code).collect();
    assert_eq!(codes, ["interior-sealed"]);

    // Two rooms behind a portal that leads nowhere near the outside are both said, by their numbers.
    let mut portals = vec![VILLA.portals[0]];
    portals.push(portal(2, 3));
    let cut = Interior {
        portals: portals.leak(),
        ..VILLA
    };
    assert_eq!(
        found(cut),
        [(
            "interior-room-unreachable",
            "2 room(s) are joined to the outside by no chain of portals: #2, #3".to_string()
        )]
    );
}

#[test]
fn what_a_save_refuses_stays_the_rule_it_was_and_fires_none_of_these() {
    // A portal to a room past the rooms is `interior-broken`: it counts for no room and joins nothing.
    let mut portals = VILLA.portals.to_vec();
    portals.push(portal(3, 9));
    let rooms = rooms_with(3, |room| Room {
        portal_count: Some(2),
        ..room
    });
    let broken = Interior {
        portals: portals.leak(),
        rooms: rooms.leak(),
        ..VILLA
    };
    assert_eq!(found(broken), Vec::<(&str, String)>::new());
    let bytes = types_file_with_interiors("villa_types", &[], &[broken]);
    let inside = or_maps::facts::inside_of(&bytes, "ytyp", None).unwrap();
    let or_maps::facts::Inside::Types(types) = inside else {
        panic!("a type file");
    };
    let interior = types.archetypes[0].interior.as_ref().unwrap();
    assert_eq!(
        interior.problems,
        ["portal 3 names room 9, past the 4 rooms in this interior"]
    );
    assert!(interior.findings.is_empty());
}

/// A pack of one set whose one source is a type file of `interior`.
fn pack(dir: &Path, interior: Interior<'_>) -> std::path::PathBuf {
    let folder = dir.join("maps");
    let bytes = types_file_with_interiors("villa_types", &[], &[interior]);
    let files = [Incoming {
        name: "villa_types.ytyp",
        bytes: bytes.leak(),
        from_game: false,
        virtual_path: None,
    }];
    let added = add_files(&folder, &files, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

#[test]
fn the_rules_of_a_map_set_say_each_at_its_interior_with_its_severity_and_a_waiver_sets_it_aside() {
    for broken in cases() {
        let scratch = tempfile::tempdir().unwrap();
        let folder = pack(scratch.path(), broken.interior);
        let mut model = Maps::read(&folder).unwrap();
        let rule = rule_of(broken.code).expect("the rule is one `rule_of` finds");
        let said = |model: &Maps| {
            let facts = inspect(&folder, model, Depth::Open);
            let found: Vec<_> = (check(model, &facts, None).into_iter())
                .filter(|finding| finding.code.starts_with("interior-"))
                .collect();
            let [finding] = found.as_slice() else {
                panic!("{}: {found:?}", broken.code);
            };
            finding.clone()
        };
        let finding = said(&model);
        assert_eq!(finding.code, broken.code);
        assert_eq!(finding.severity, rule.severity);
        assert_eq!(
            finding.place.source.as_deref(),
            Some("villa_types.metadata.src")
        );
        assert_eq!(
            (finding.place.archetype, finding.place.entity),
            (Some(0), None)
        );
        assert!(
            finding.message.starts_with("the interior `")
                && finding.message.ends_with(&format!(": {}", broken.said)),
            "{}",
            finding.message
        );
        assert_eq!(finding.blocks(), rule.severity == Severity::Error);
        assert!(!finding.blocks(), "no rule of an interior stops a build");

        model.waivers.push(Waiver {
            code: broken.code.to_string(),
            map: None,
            reason: "the room is entered by a script".to_string(),
        });
        assert_eq!(
            said(&model).waived.as_deref(),
            Some("the room is entered by a script"),
            "{}",
            broken.code
        );
    }

    // And a sound interior adds nothing to what the rules say of its set.
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path(), VILLA);
    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let interior: Vec<_> = (check(&model, &facts, None).into_iter())
        .filter(|finding| finding.code.starts_with("interior-"))
        .collect();
    assert_eq!(interior, Vec::new());
}

#[test]
fn the_listing_of_an_interior_states_what_it_holds_and_its_lines_say_the_same() {
    let villa = read(VILLA);
    assert_eq!((villa.archetype, villa.name.as_str()), (0, "villa"));
    let listed = &villa.listed;
    assert_eq!(listed.entities, VILLA.entities.len());
    assert_eq!(listed.time_cycle_modifiers, VILLA.modifiers.len());
    let rooms: Vec<(&str, u32, &[u32])> = (listed.rooms.iter())
        .map(|room| {
            (
                room.name.as_str(),
                room.portal_count,
                &room.attached_objects[..],
            )
        })
        .collect();
    assert_eq!(
        rooms,
        [
            ("limbo", 1, &[5][..]),
            ("hall", 2, &[0, 1][..]),
            ("kitchen", 2, &[2][..]),
            ("cellar", 1, &[3][..])
        ]
    );
    assert_eq!(listed.rooms[2].bb_min, VILLA.rooms[2].min);
    assert_eq!(listed.rooms[2].bb_max, VILLA.rooms[2].max);
    let portals: Vec<(u32, u32, usize, &[u32])> = (listed.portals.iter())
        .map(|p| {
            (
                p.room_from,
                p.room_to,
                p.corners.len(),
                &p.attached_objects[..],
            )
        })
        .collect();
    assert_eq!(
        portals,
        [(1, 0, 4, &[4][..]), (1, 2, 4, &[][..]), (2, 3, 4, &[][..])]
    );
    assert_eq!(listed.portals[0].corners, VILLA.portals[0].corners);
    let sets: Vec<(&str, Vec<u32>)> = (listed.entity_sets.iter())
        .map(|set| {
            let locations = set.entities.iter().filter_map(|e| e.location).collect();
            (set.name.as_str(), locations)
        })
        .collect();
    assert_eq!(
        sets,
        [("villa_party", vec![2, 3]), ("villa_night", vec![1])]
    );
    assert_eq!(listed.entity_sets[0].entities[0].archetype, "prop_balloon");

    let lines = listed.lines();
    assert_eq!(
        lines[0],
        "6 entity(ies), 4 room(s), 3 portal(s), 2 entity set(s), 2 time cycle modifier(s)"
    );
    assert_eq!(
        lines[2],
        "room 1 `hall`: box -5,-5,0 to 5,5,3, flags 0, floor 0, counts 2 portal(s), attaches #0, #1"
    );
    assert_eq!(
        lines[5],
        "portal 0: room 1 to room 0, flags 0, opacity 0, corners -1,-5,0 | 1,-5,0 | 1,-5,2.25 | -1,-5,2.25, attaches #4"
    );
    assert_eq!(
        lines[8],
        "entity set 0 `villa_party`: 2 entity(ies): `prop_balloon` in 2, `prop_crate` in 3"
    );
    assert_eq!(lines.len(), 1 + 4 + 3 + 2, "and no finding");

    // A finding is a line of its own, with the word of its severity.
    let sealed = &cases()[6];
    let lines = read(sealed.interior).listed.lines();
    let said: Vec<&str> = lines.last().unwrap().split_whitespace().collect();
    assert_eq!(said[..2], ["note", "interior-sealed"]);
    assert_eq!(
        said[2..].join(" "),
        "none of its 2 portal(s) leads to the outside, room 0"
    );
}
