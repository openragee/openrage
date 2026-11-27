//! The operations that edit an interior keep every index it holds: a room taken out renumbers the rooms the portals and the entity
//! sets name, an entity taken out renumbers the entities the rooms and the portals attach, a portal added or taken out keeps the count
//! each of its rooms states, and the inverses put every index and every order back. What would break an interior is refused with
//! nothing changed, and a list made for another state of its source is refused.
//!
//! That each operation followed by its inverse leaves every source byte for byte is held, with a case for each, by
//! `every_operation_followed_by_its_inverse_leaves_every_source_byte_for_byte_as_it_was`.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_io::meta::typed::{CBaseArchetypeDefAny, CMloArchetypeDef};
use or_maps::edit::read_archetype;
use or_maps::import::{add_files, Incoming};
use or_maps::operation::{
    apply, apply_all, digest_of, inverses, Attached, Error, Operation, PortalMembers, PoseData,
    RoomMembers, Session, NOT_A_ROOM,
};
use or_maps::sample::{
    map_file, types_file_with_interiors, Archetype, Entity, EntitySet, Interior, Portal, VILLA,
};

const VILLA_TYPES: &str = "villa_types.metadata.src";
const YARD: &str = "yard.metadata.src";

/// A pack of one set: a map file of one entity, and a type file whose one archetype is `interior`.
fn pack_of(dir: &Path, interior: Interior<'_>) -> PathBuf {
    let folder = dir.join("maps");
    let yard = map_file(
        "yard",
        &[Entity::at("prop_chair", [1.0, 2.0, 3.0], 9)],
        ([-50.0; 3], [50.0; 3]),
    );
    let villa = types_file_with_interiors("villa_types", &[], &[interior]);
    let incoming = |name: &'static str, bytes: Vec<u8>| Incoming {
        name,
        bytes: bytes.leak(),
        from_game: false,
        virtual_path: None,
    };
    let files = [
        incoming("yard.ymap", yard),
        incoming("villa_types.ytyp", villa),
    ];
    let added = add_files(&folder, &files, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

fn pack(dir: &Path) -> PathBuf {
    pack_of(dir, VILLA)
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

/// The villa as its source cooks to it now.
fn villa(folder: &Path) -> CMloArchetypeDef {
    match read_archetype(&folder.join(VILLA_TYPES), 0).unwrap() {
        CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => mlo,
        other => panic!("the villa is an interior, and reads as {other:?}"),
    }
}

/// What the villa states of its indexes: the two rooms of each portal, the locations of each entity set, the entities each room
/// and each portal attaches, and the portals each room counts.
#[derive(Debug, PartialEq)]
struct Indexes {
    rooms: Vec<String>,
    portals: Vec<(u32, u32)>,
    locations: Vec<Vec<u32>>,
    in_rooms: Vec<Vec<u32>>,
    on_portals: Vec<Vec<u32>>,
    counts: Vec<u32>,
    guids: Vec<u32>,
}

fn indexes(folder: &Path) -> Indexes {
    let villa = villa(folder);
    Indexes {
        rooms: villa.rooms.iter().map(|r| r.name.clone()).collect(),
        portals: (villa.portals.iter())
            .map(|p| (p.room_from, p.room_to))
            .collect(),
        locations: (villa.entity_sets.iter())
            .map(|s| s.locations.clone())
            .collect(),
        in_rooms: (villa.rooms.iter())
            .map(|r| r.attached_objects.clone())
            .collect(),
        on_portals: (villa.portals.iter())
            .map(|p| p.attached_objects.clone())
            .collect(),
        counts: villa.rooms.iter().map(|r| r.portal_count).collect(),
        guids: villa.entities.iter().map(|e| e.guid).collect(),
    }
}

/// The villa as [`VILLA`] states it, which each test starts from.
fn as_made() -> Indexes {
    Indexes {
        rooms: ["limbo", "hall", "kitchen", "cellar"]
            .map(String::from)
            .to_vec(),
        portals: vec![(1, 0), (1, 2), (2, 3)],
        locations: vec![vec![2, 3], vec![1]],
        in_rooms: vec![vec![5], vec![0, 1], vec![2], vec![3]],
        on_portals: vec![vec![4], vec![], vec![]],
        counts: vec![1, 2, 2, 1],
        guids: vec![1, 2, 3, 4, 5, 6],
    }
}

fn remove_portal(portal: usize) -> Operation {
    Operation::RemovePortal {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        portal,
    }
}

fn remove_room(room: usize) -> Operation {
    Operation::RemoveRoom {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        room,
    }
}

fn set_room(room: usize, members: RoomMembers) -> Operation {
    Operation::SetRoom {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        room,
        members,
    }
}

fn attach(entity: usize, room: Option<usize>, portal: Option<usize>) -> Operation {
    Operation::AttachEntity {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        entity,
        room,
        portal,
        at: None,
    }
}

const QUAD: [[f32; 3]; 4] = [
    [12.0, -1.0, -3.0],
    [12.0, 1.0, -3.0],
    [12.0, 1.0, -0.75],
    [12.0, -1.0, -0.75],
];

fn add_portal(from: u32, to: u32, portal: Option<usize>, corners: &[[f32; 3]]) -> Operation {
    Operation::AddPortal {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        portal,
        members: PortalMembers {
            room_from: Some(from),
            room_to: Some(to),
            corners: Some(corners.to_vec()),
            ..PortalMembers::default()
        },
        attached_objects: Vec::new(),
    }
}

/// The list that takes the kitchen, the middle room, out of the villa: the portal into it goes, the portal out of it leads from the
/// hall, the entity a set places in it leaves the set for the cellar, and the room goes.
fn the_kitchen_goes() -> Vec<Operation> {
    vec![
        remove_portal(1),
        Operation::SetPortal {
            source: VILLA_TYPES.into(),
            expects: None,
            archetype: 0,
            portal: 1,
            members: PortalMembers {
                room_from: Some(1),
                ..PortalMembers::default()
            },
        },
        Operation::MoveEntityOutOfSet {
            source: VILLA_TYPES.into(),
            expects: None,
            archetype: 0,
            set: 0,
            entity: 0,
            to: None,
            attached: vec![Attached {
                room: Some(3),
                portal: None,
                at: None,
            }],
        },
        remove_room(2),
    ]
}

#[test]
fn the_sample_villa_states_the_indexes_the_tests_start_from() {
    let scratch = tempfile::tempdir().unwrap();
    assert_eq!(indexes(&pack(scratch.path())), as_made());
}

#[test]
fn removing_a_middle_room_renumbers_the_portals_and_the_entity_sets_exactly_and_the_inverses_restore_them(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);

    let applied = apply_all(&folder, &the_kitchen_goes(), false).unwrap();
    assert_eq!(
        indexes(&folder),
        Indexes {
            rooms: ["limbo", "hall", "cellar"].map(String::from).to_vec(),
            // The cellar was room 3: the portal that leads to it names 2, and the front door names what it named.
            portals: vec![(1, 0), (1, 2)],
            // The crate of the party stood in room 3, and the candle of the night in the hall.
            locations: vec![vec![2], vec![1]],
            // The balloon is entity 6, in the cellar; the table the kitchen attached is attached to nothing.
            in_rooms: vec![vec![5], vec![0, 1], vec![3, 6]],
            on_portals: vec![vec![4], vec![]],
            counts: vec![1, 2, 1],
            guids: vec![1, 2, 3, 4, 5, 6, 20],
        }
    );
    let Operation::AddRoom {
        room,
        members,
        attached_objects,
        ..
    } = &applied[3].inverse
    else {
        panic!("a room is put back by {}", applied[3].inverse.name());
    };
    assert_eq!(*room, Some(2));
    assert_eq!(members.name.as_deref(), Some("kitchen"));
    assert_eq!(members.portal_count, Some(0));
    assert_eq!(attached_objects, &[2]);

    apply_all(&folder, &inverses(&applied), false).unwrap();
    assert_eq!(indexes(&folder), as_made());
    assert!(
        files(&folder) == before,
        "the inverses leave every file as it was"
    );
}

#[test]
fn removing_an_entity_renumbers_every_list_of_entity_indexes_exactly_and_the_inverse_restores_the_order(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    // The chair, entity 1, is the second the hall attaches, and the one entity the portal of the kitchen does.
    apply(&folder, &attach(1, None, Some(1))).unwrap();
    let before = files(&folder);
    let stated = indexes(&folder);
    assert_eq!(stated.on_portals, [vec![4], vec![1], vec![]]);

    let removal = Operation::RemoveInteriorEntity {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        entity: 1,
    };
    let applied = apply(&folder, &removal).unwrap();
    let after = indexes(&folder);
    assert_eq!(after.in_rooms, [vec![4], vec![0], vec![1], vec![2]]);
    assert_eq!(after.on_portals, [vec![3], vec![], vec![]]);
    assert_eq!(after.guids, [1, 3, 4, 5, 6]);
    // No room and no entity set names an entity, so nothing else moved.
    assert_eq!(
        (&after.portals, &after.locations, &after.counts),
        (&stated.portals, &stated.locations, &stated.counts)
    );
    let Operation::InsertInteriorEntity {
        entity, attached, ..
    } = &applied.inverse
    else {
        panic!("an entity is put back by {}", applied.inverse.name());
    };
    let place = |room, portal, at| Attached {
        room,
        portal,
        at: Some(at),
    };
    assert_eq!(*entity, 1);
    assert_eq!(
        attached,
        &[place(Some(1), None, 1), place(None, Some(1), 0)]
    );

    apply(&folder, &applied.inverse).unwrap();
    assert_eq!(indexes(&folder), stated);
    assert!(
        files(&folder) == before,
        "the inverse leaves every file as it was"
    );
}

#[test]
fn a_portal_added_taken_out_or_led_elsewhere_keeps_the_count_each_room_states() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let added = apply(&folder, &add_portal(3, 0, Some(1), &QUAD)).unwrap();
    assert_eq!(added.added, Some(1));
    assert_eq!(
        added.notes,
        Vec::<String>::new(),
        "a portal that leads to the outside is as the installs have it"
    );
    let stated = indexes(&folder);
    assert_eq!(stated.portals, [(1, 0), (3, 0), (1, 2), (2, 3)]);
    assert_eq!(stated.counts, [2, 2, 2, 2]);

    // Led from the kitchen to the hall in place of the cellar to the outside: two rooms count one fewer and two one more.
    let led = Operation::SetPortal {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        portal: 1,
        members: PortalMembers {
            room_from: Some(2),
            room_to: Some(1),
            ..PortalMembers::default()
        },
    };
    let led = apply(&folder, &led).unwrap();
    assert_eq!(indexes(&folder).counts, [1, 3, 3, 1]);
    apply(&folder, &led.inverse).unwrap();
    assert_eq!(indexes(&folder), stated);

    apply(&folder, &remove_portal(1)).unwrap();
    assert_eq!(indexes(&folder), as_made());

    // A portal that leaves the outside is done, and said: no portal of the installs does.
    let outward = apply(&folder, &add_portal(0, 3, None, &QUAD)).unwrap();
    assert!(
        matches!(outward.notes.as_slice(), [note] if note.contains("`room_from`")),
        "{:?}",
        outward.notes
    );

    // A room that counts no portal cannot count one fewer, so the portal that names it stays until its count is stated.
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let uncounted = RoomMembers {
        portal_count: Some(0),
        ..RoomMembers::default()
    };
    apply(&folder, &set_room(3, uncounted)).unwrap();
    let before = files(&folder);
    let refused = apply(&folder, &remove_portal(2)).unwrap_err().to_string();
    assert!(refused.contains("room 3 counts no portal"), "{refused}");
    assert!(files(&folder) == before);
}

#[test]
fn an_entity_added_to_an_interior_changes_its_type_file_and_no_map() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);
    let added = Operation::AddInteriorEntity {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        archetype_name: "prop_stool".into(),
        pose: PoseData {
            position: [9.0, 1.0, -3.0],
            rotation: glam::Quat::from_rotation_z(0.5).to_array(),
            scale_xy: 1.0,
            scale_z: 1.0,
        },
        donor: None,
        room: Some(3),
    };
    let applied = apply(&folder, &added).unwrap();
    assert_eq!(applied.added, Some(6));
    let after = files(&folder);
    let changed: Vec<&PathBuf> = (after.iter())
        .filter(|(path, bytes)| before.get(*path) != Some(bytes))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        changed,
        [&folder.join(VILLA_TYPES).join("metadata.xml")],
        "the entity is in the type file, and the map file is as it was"
    );
    assert_eq!(after.len(), before.len());

    let villa = villa(&folder);
    let stool = &villa.entities[6];
    assert!(stool.archetype_name.hash == or_io::string_hash(b"prop_stool"));
    assert_eq!(
        [stool.position.x, stool.position.y, stool.position.z],
        [9.0, 1.0, -3.0]
    );
    // One more than the largest guid of the villa, which an entity of a set states.
    assert_eq!(stool.guid, 23);
    assert_eq!(villa.rooms[3].attached_objects, [3, 6]);
    assert_eq!(added.names(), ["prop_stool"]);
    // The turn is stored as an entity of a map stores one, as its conjugate, and not as an interior instance stores its own.
    let stored = glam::Quat::from_xyzw(
        stool.rotation.x,
        stool.rotation.y,
        stool.rotation.z,
        stool.rotation.w,
    );
    let stated = glam::Quat::from_rotation_z(0.5);
    assert!(
        stored.conjugate().abs_diff_eq(stated, 1e-6) && !stored.abs_diff_eq(stated, 1e-3),
        "{stored:?}"
    );

    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);
}

/// What an operation the villa refuses says, with the pack as it was.
fn refusal(prepare: &[Operation], operation: &Operation) -> String {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    apply_all(&folder, prepare, false).unwrap();
    let before = files(&folder);
    let refused = apply(&folder, operation)
        .err()
        .unwrap_or_else(|| panic!("{} is refused", operation.name()));
    assert!(
        matches!(refused, Error::Refused { name, .. } if name == operation.name()),
        "{refused}"
    );
    assert!(
        files(&folder) == before,
        "{}: a refusal changes nothing",
        operation.name()
    );
    refused.to_string()
}

#[test]
fn what_would_break_an_interior_is_refused_by_name_and_changes_nothing() {
    let named = |name: &str| RoomMembers {
        name: Some(name.into()),
        ..RoomMembers::default()
    };
    let add_room = |room, name: &str| Operation::AddRoom {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        room,
        members: named(name),
        attached_objects: Vec::new(),
    };
    let into_set = |location| Operation::MoveEntityIntoSet {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        entity: 2,
        set: 1,
        location,
        at: None,
    };
    let cases: Vec<(Vec<Operation>, Operation, &str)> = vec![
        // A room is taken out once nothing names it.
        (vec![], remove_room(2), "portal(s) 1, 2 name the room"),
        (
            vec![remove_portal(2)],
            remove_room(3),
            "entity set 0 places an entity in the room",
        ),
        (vec![], remove_room(0), "room 0 is the outside"),
        (vec![], remove_room(4), "room 4 is past the 4 rooms"),
        (vec![], add_room(Some(0), "porch"), "room 0 is the outside"),
        (vec![], add_room(Some(5), "porch"), "past the end of the 4"),
        (
            vec![],
            add_room(None, "Hall"),
            "already holds a room of that name",
        ),
        (vec![], add_room(None, " "), "a room has a name"),
        (
            vec![],
            set_room(2, named("limbo")),
            "already holds a room of that name",
        ),
        (
            vec![],
            set_room(2, RoomMembers::default()),
            "no member of the room is stated",
        ),
        (
            vec![],
            set_room(
                2,
                RoomMembers {
                    bb_min: Some([0.0, f32::INFINITY, 0.0]),
                    ..RoomMembers::default()
                },
            ),
            "not a finite number",
        ),
        // A portal joins two rooms the interior holds, by four corners.
        (
            vec![],
            add_portal(2, 2, None, &QUAD),
            "room 2 is named on both sides",
        ),
        (
            vec![],
            add_portal(1, 4, None, &QUAD),
            "would name room 4, past the 4 rooms",
        ),
        (
            vec![],
            add_portal(1, 3, None, &QUAD[..3]),
            "a quad of four corners, and 3 are stated",
        ),
        (
            vec![],
            add_portal(1, 3, Some(4), &QUAD),
            "past the end of the 3",
        ),
        (vec![], remove_portal(3), "portal 3 is past the 3 portals"),
        (
            vec![],
            Operation::SetPortal {
                source: VILLA_TYPES.into(),
                expects: None,
                archetype: 0,
                portal: 1,
                members: PortalMembers {
                    room_to: Some(1),
                    ..PortalMembers::default()
                },
            },
            "room 1 is named on both sides",
        ),
        // An entity is attached once to a list, and to one the interior holds.
        (
            vec![],
            attach(1, Some(1), None),
            "room 1 attaches entity 1 already",
        ),
        (vec![], attach(6, Some(1), None), "entity 6 is past the 6"),
        (
            vec![],
            attach(1, Some(4), None),
            "room 4 is past the 4 rooms",
        ),
        (
            vec![],
            attach(1, None, None),
            "neither a room nor a portal is named",
        ),
        (
            vec![],
            attach(1, Some(2), Some(1)),
            "a room and a portal are both named",
        ),
        (
            vec![],
            Operation::DetachEntity {
                source: VILLA_TYPES.into(),
                expects: None,
                archetype: 0,
                entity: 1,
                room: None,
                portal: Some(0),
            },
            "portal 0 does not attach entity 1",
        ),
        // An entity of a set stands in a room the interior holds.
        (vec![], into_set(4), "names room 4, past the 4 rooms"),
        (vec![], into_set(NOT_A_ROOM | 1), "is no room index"),
        (
            vec![],
            Operation::AddEntitySet {
                source: VILLA_TYPES.into(),
                expects: None,
                archetype: 0,
                set: None,
                name: "villa_night".into(),
                entities: Vec::new(),
            },
            "already holds an entity set of that name",
        ),
        (
            vec![],
            Operation::AddTimeCycleModifier {
                source: VILLA_TYPES.into(),
                expects: None,
                archetype: 0,
                modifier: None,
                name: "villa_noon".into(),
                sphere: [0.0, 0.0, 0.0, f32::NAN],
                percentage: 1.0,
                range: 1.0,
                start_hour: 0,
                end_hour: 1,
            },
            "not a finite number",
        ),
        // An operation of an interior edits an interior of a type file.
        (
            vec![],
            Operation::RemoveRoom {
                source: VILLA_TYPES.into(),
                expects: None,
                archetype: 1,
                room: 1,
            },
            "archetype 1 is past the 1 the type file holds",
        ),
        (
            vec![],
            Operation::RemoveRoom {
                source: YARD.into(),
                expects: None,
                archetype: 0,
                room: 1,
            },
            "remove_room does not edit a map file",
        ),
    ];
    for (prepare, operation, said) in &cases {
        let why = refusal(prepare, operation);
        assert!(
            why.contains(said),
            "{}: `{said}` in {why}",
            operation.name()
        );
    }
}

#[test]
fn a_list_made_for_another_state_of_its_source_is_refused_and_changes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let xml = folder.join(VILLA_TYPES).join("metadata.xml");
    let made_for = digest_of(&std::fs::read(&xml).unwrap());
    let list: Vec<Operation> = (the_kitchen_goes().into_iter())
        .enumerate()
        .map(|(at, operation)| match at {
            // The first of a list states what the list was made for; the rest follow it.
            0 => operation.expecting(made_for.clone()),
            _ => operation,
        })
        .collect();

    // Another edit of the villa, after the list was made.
    let renamed = RoomMembers {
        name: Some("scullery".into()),
        ..RoomMembers::default()
    };
    let edit = apply(&folder, &set_room(2, renamed)).unwrap();
    let before = files(&folder);
    let found = digest_of(&std::fs::read(&xml).unwrap());

    let refused = apply_all(&folder, &list, false).unwrap_err();
    let Error::InList {
        number: 1,
        of: 4,
        refused: stale,
    } = &refused
    else {
        panic!("{refused}");
    };
    assert!(
        matches!(&**stale, Error::Stale { name: "remove_portal", expects, found: held, .. } if *expects == made_for && *held == found),
        "{stale}"
    );
    let said = refused.to_string();
    assert!(
        said.contains("the source is not as the operation expects it")
            && said.contains("take back what was done after it first"),
        "{said}"
    );
    assert!(files(&folder) == before, "a stale list changes nothing");

    // Once the edit is taken back the list is for the state the source is in.
    apply(&folder, &edit.inverse).unwrap();
    apply_all(&folder, &list, false).unwrap();
    assert_eq!(indexes(&folder).rooms, ["limbo", "hall", "cellar"]);
}

#[test]
fn an_inverse_of_a_session_is_refused_once_another_edit_of_the_interior_follows_it() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let before = files(&folder);
    let mut session = Session::open(&folder).unwrap();
    let removed = session.apply(&remove_portal(1)).unwrap();
    let attached = session.apply(&attach(1, None, Some(0))).unwrap();
    let stale = session.apply(&removed.inverse).unwrap_err();
    assert!(
        matches!(
            stale,
            Error::Stale {
                name: "add_portal",
                ..
            }
        ),
        "{stale}"
    );
    // In their order, each finds the state it states.
    session.apply(&attached.inverse).unwrap();
    session.apply(&removed.inverse).unwrap();
    session.flush().unwrap();
    assert!(files(&folder) == before);
}

#[test]
fn an_interior_whose_entity_set_states_what_is_no_room_takes_a_portal_last_and_gives_none_up() {
    // The party places its crate at a location with bit 31 set, as six locations of each install are.
    let flagged = NOT_A_ROOM | 1;
    let interior = Interior {
        entity_sets: &[EntitySet {
            name: "villa_party",
            entities: &[
                (2, Entity::at("prop_balloon", [9.0, 2.0, 1.0], 20)),
                (flagged, Entity::at("prop_crate", [9.0, -2.0, -3.0], 21)),
            ],
        }],
        ..VILLA
    };
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack_of(scratch.path(), interior);
    let before = files(&folder);
    for refused in [add_portal(3, 0, Some(1), &QUAD), remove_portal(2)] {
        let why = apply(&folder, &refused).unwrap_err().to_string();
        assert!(why.contains("a location that is no room"), "{why}");
        assert!(files(&folder) == before);
    }
    let added = apply(&folder, &add_portal(3, 0, None, &QUAD)).unwrap();
    assert_eq!(added.added, Some(3));
    assert_eq!(indexes(&folder).counts, [2, 2, 2, 2]);
    // No portal is given up here, so what takes the portal back is the XML as it was.
    assert_eq!(added.inverse.name(), "rewrite_source");
    apply(&folder, &added.inverse).unwrap();
    assert!(files(&folder) == before);
    assert_eq!(indexes(&folder).locations, [vec![2, flagged]]);

    // A room put in before the kitchen renumbers the location that is a room, and leaves the other.
    let attic = Operation::AddRoom {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        room: Some(1),
        members: RoomMembers {
            name: Some("attic".into()),
            ..RoomMembers::default()
        },
        attached_objects: Vec::new(),
    };
    apply(&folder, &attic).unwrap();
    assert_eq!(indexes(&folder).locations, [vec![3, flagged]]);
}

#[test]
fn an_operation_of_an_interior_edits_the_archetype_it_names_and_is_refused_one_that_is_no_interior()
{
    // A crate before the villa: archetype 0 is no interior, and the villa is archetype 1.
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("maps");
    let crates = [Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])];
    let types = types_file_with_interiors("villa_types", &crates, &[VILLA]);
    let incoming = [Incoming {
        name: "villa_types.ytyp",
        bytes: types.leak(),
        from_game: false,
        virtual_path: None,
    }];
    let added = add_files(&folder, &incoming, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let before = files(&folder);
    let renamed = |archetype| Operation::SetRoom {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype,
        room: 2,
        members: RoomMembers {
            name: Some("scullery".into()),
            ..RoomMembers::default()
        },
    };
    let refused = apply(&folder, &renamed(0)).unwrap_err().to_string();
    assert!(refused.contains("archetype 0 is no interior"), "{refused}");
    assert!(files(&folder) == before, "a refusal changes nothing");

    apply(&folder, &renamed(1)).unwrap();
    let CBaseArchetypeDefAny::CMloArchetypeDef(villa) =
        read_archetype(&folder.join(VILLA_TYPES), 1).unwrap()
    else {
        panic!("archetype 1 is the villa");
    };
    assert_eq!(villa.rooms[2].name, "scullery");
}

#[test]
fn an_index_no_file_can_count_past_is_left_to_the_save_to_refuse_and_wraps_to_nothing_the_interior_holds(
) {
    // One more than the largest number a file states is nought: the outside, and the first entity.
    let last = u32::MAX;
    let attic = Operation::AddRoom {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        room: Some(1),
        members: RoomMembers {
            name: Some("attic".into()),
            ..RoomMembers::default()
        },
        attached_objects: Vec::new(),
    };
    let mut portals = VILLA.portals.to_vec();
    portals.push(Portal {
        from: 2,
        to: last,
        corners: QUAD,
        flags: 0,
        attached: &[],
    });
    let scratch = tempfile::tempdir().unwrap();
    let names_no_room = Interior {
        portals: portals.leak(),
        ..VILLA
    };
    let folder = pack_of(scratch.path(), names_no_room);
    let before = files(&folder);
    let why = apply(&folder, &attic).unwrap_err().to_string();
    assert!(
        why.contains(&format!("portal 3 names room {last}, past the 5 rooms")),
        "{why}"
    );
    assert!(files(&folder) == before, "a refusal changes nothing");

    // The chair as a removal carries it, to put into an interior whose kitchen attaches an entity no file counts past.
    let scratch = tempfile::tempdir().unwrap();
    let sound = pack(scratch.path());
    let removal = Operation::RemoveInteriorEntity {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        entity: 1,
    };
    // For any state of its source: it was made for the sound villa.
    let mut put_back = apply(&sound, &removal).unwrap().inverse;
    put_back.set_expects(None);
    assert_eq!(put_back.name(), "insert_interior_entity");
    let out_of_the_set = Operation::MoveEntityOutOfSet {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        set: 0,
        entity: 0,
        to: Some(0),
        attached: Vec::new(),
    };
    let mut rooms = VILLA.rooms.to_vec();
    rooms[2].attached = &[2, u32::MAX];
    let scratch = tempfile::tempdir().unwrap();
    let attaches_no_entity = Interior {
        rooms: rooms.leak(),
        ..VILLA
    };
    let folder = pack_of(scratch.path(), attaches_no_entity);
    let before = files(&folder);
    for operation in [put_back, out_of_the_set] {
        let why = apply(&folder, &operation).unwrap_err().to_string();
        assert!(
            why.contains(&format!(
                "room 2 attaches object {last}, past the 7 entities"
            )),
            "{}: {why}",
            operation.name()
        );
        assert!(files(&folder) == before, "a refusal changes nothing");
    }
}

#[test]
fn a_session_asked_to_name_a_room_that_has_no_name_is_left_able_to_write() {
    let mut rooms = VILLA.rooms.to_vec();
    rooms[2].name = "";
    let scratch = tempfile::tempdir().unwrap();
    let nameless = Interior {
        rooms: rooms.leak(),
        ..VILLA
    };
    let folder = pack_of(scratch.path(), nameless);
    let named = RoomMembers {
        name: Some("kitchen".into()),
        ..RoomMembers::default()
    };
    let mut session = Session::open(&folder).unwrap();
    // ⚠ No operation states a room with no name, so `set_room` does not take this one back, and a session that held it to do so
    // could neither write the name, where the file has no storage for one, nor take it back. It is refused when it is asked, as
    // `apply` refuses it, or it is taken back by what the session gives.
    match session.apply(&set_room(2, named.clone())) {
        Ok(done) => {
            let back = session.apply(&done.inverse);
            back.unwrap_or_else(|e| panic!("the inverse, {}: {e}", done.inverse.name()));
        }
        Err(refused) => {
            assert!(
                matches!(
                    refused,
                    Error::Refused {
                        name: "set_room",
                        ..
                    }
                ),
                "{refused}"
            );
            apply(&folder, &set_room(2, named)).expect_err("`apply` refuses it too");
        }
    }
    let floor = RoomMembers {
        floor_id: Some(-1),
        ..RoomMembers::default()
    };
    session.apply(&set_room(3, floor)).unwrap();
    let written = session.flush().unwrap();
    assert_eq!(written.len(), 1, "{written:?}");
    let villa = villa(&folder);
    assert_eq!(
        (villa.rooms[2].name.as_str(), villa.rooms[3].floor_id),
        ("", -1)
    );
}

#[test]
fn an_entity_set_whose_two_arrays_differ_is_listed_with_what_each_entity_has_and_is_taken_out_to_mend_the_interior(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    // A document a person typed: the party keeps its two entities and the location of the first alone.
    let xml = folder.join(VILLA_TYPES).join("metadata.xml");
    let typed = std::fs::read_to_string(&xml).unwrap();
    let first = "<locations>\n            <Item value=\"2\"/>\n";
    let both = format!("{first}            <Item value=\"3\"/>\n");
    assert_eq!(typed.matches(&both).count(), 1, "{typed}");
    std::fs::write(&xml, typed.replace(&both, first)).unwrap();
    let before = files(&folder);

    let listed = |folder: &Path| {
        let cooked = or_maps::facts::cooked(&folder.join(VILLA_TYPES)).unwrap();
        or_maps::interior::interiors_of(&cooked, None)
            .unwrap()
            .remove(0)
            .listed
    };
    let broken = listed(&folder);
    assert_eq!(
        broken.broken,
        ["entity set 0 has 1 locations for 2 entities"]
    );
    let locations: Vec<Option<u32>> = (broken.entity_sets[0].entities.iter())
        .map(|entity| entity.location)
        .collect();
    assert_eq!(locations, [Some(2), None]);
    let lines = broken.lines();
    let party = (lines.iter())
        .find(|line| line.starts_with("entity set 0 "))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(
        party.contains(" in 2, ") && party.ends_with(" with no location"),
        "{party}"
    );

    // No entity of the set has a place to be moved from, and nothing is changed.
    let moved = Operation::MoveEntityOutOfSet {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        set: 0,
        entity: 1,
        to: None,
        attached: Vec::new(),
    };
    let why = apply(&folder, &moved).unwrap_err().to_string();
    assert!(
        why.contains("entity set 0 has 1 locations for 2 entities"),
        "{why}"
    );
    assert!(files(&folder) == before, "a refusal changes nothing");

    // The set is taken out, which mends the interior; what takes that back is the document as the person typed it.
    let removal = Operation::RemoveEntitySet {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        set: 0,
    };
    let applied = apply(&folder, &removal).unwrap();
    let mended = listed(&folder);
    assert_eq!(mended.broken, Vec::<String>::new());
    assert_eq!(mended.entity_sets.len(), 1);
    assert_eq!(applied.inverse.name(), "rewrite_source");
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before, "the document is as it was typed");
}

/// The class of an entity follows its archetype: an interior of the pack is placed by an instance alone, and holds no interior.
#[test]
fn an_interior_the_pack_declares_is_placed_by_no_plain_entity_and_holds_no_interior() {
    let pose = PoseData {
        position: [10.0, 10.0, 3.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
    };
    let on_the_yard = |archetype: &str| Operation::AddEntity {
        source: YARD.into(),
        expects: None,
        archetype: archetype.into(),
        pose,
        donor: None,
    };
    let said = refusal(&[], &on_the_yard("villa"));
    assert!(
        said.contains("`villa` is an interior a type file of the pack declares"),
        "{said}"
    );
    let inside = Operation::AddInteriorEntity {
        source: VILLA_TYPES.into(),
        expects: None,
        archetype: 0,
        archetype_name: "villa".into(),
        pose,
        donor: None,
        room: Some(1),
    };
    let said = refusal(&[], &inside);
    assert!(said.contains("an interior holds no interior"), "{said}");

    // What is no interior is added as it was.
    let scratch = tempfile::tempdir().unwrap();
    let folder = pack(scratch.path());
    let added = apply(&folder, &on_the_yard("prop_table")).unwrap();
    assert_eq!(added.added, Some(1));
}

/// A room is found by its name, so two names apart only by the white space around them are one.
#[test]
fn a_room_is_refused_a_name_another_has_but_for_the_white_space_around_it() {
    for name in [" hall", "hall ", "hall\t", "HALL"] {
        let named = RoomMembers {
            name: Some(name.to_string()),
            ..RoomMembers::default()
        };
        let said = refusal(&[], &set_room(3, named));
        assert!(
            said.contains("already holds a room of that name"),
            "{name:?}: {said}"
        );
    }
}
