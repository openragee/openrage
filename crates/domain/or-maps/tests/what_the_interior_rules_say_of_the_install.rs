//! What the interiors of a mounted install state, what the rules of an interior say of them, and whether an interior of the game
//! takes an edit and gives it back.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! Two questions. First, of every interior of every `.ytyp` the mount lists: what the operations of `or_maps::operation` hold an
//! edit to is counted and held (the outside is room 0, a portal has four corners and two rooms, a room counts the portals that name
//! it, no entity names a LOD parent, no name is shared), the rules of `or_maps::interior` are run and the interiors each fires on are
//! printed, a rule that warns firing on none, and what no rule was built on is printed with the figure that decided it. Second, a
//! sample of the type files that hold an interior each become a source in a scratch folder, take a room with a portal to the
//! outside, are read back, and are put back by the inverses to the XML and the file they were: once with no schema installed, as the
//! command line runs, and once with the staged schema snapshot installed, as the app installs it, where a file the snapshot states
//! more of than the file does is refused every edit that changes its shape, these and the ones that were there before them.
//!
//! ⚠ The install is opened read-only and nothing here writes into one; the scratch folder is the system's, and is removed.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use or_io::decl::File;
use or_io::meta::typed::{CBaseArchetypeDefAny, CMapTypes, CMloArchetypeDef};
use or_maps::facts::{inside_of, Inside};
use or_maps::import::{add_files, Incoming};
use or_maps::interior::INTERIOR_RULES;
use or_maps::operation::{
    apply_all, inverses, Operation, PortalMembers, RoomMembers, NOT_A_ROOM, OUTSIDE,
};
use or_maps::Severity;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::MountedGame;

const WHAT: &str = "what_the_interior_rules_say_of_the_install";

/// How many threads the type files of an install are spread over, as `what_the_rules_say_of_the_install` spreads its files.
const WORKERS: usize = 12;

/// How many type files the second question samples, evenly over the ones that hold an interior.
const SAMPLED: usize = 48;

/// The install this run was pointed at, mounted with the type files it lists; none, and a skip recorded, when it was pointed at none.
fn mounted() -> Option<(MountedGame, Vec<String>)> {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return None;
    };
    let game = common::mount(&root).expect("the install mounts");
    let mut ytyps = Vec::new();
    game.for_each_file(|logical, _, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ytyp") {
            ytyps.push(logical.to_string());
        }
    });
    ytyps.sort();
    Some((game, ytyps))
}

/// Run `each` over `files` on [`WORKERS`] threads.
fn spread<T: Sync>(files: &[T], each: impl Fn(&T) + Sync) {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| {
                while let Some(file) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    each(file);
                }
            });
        }
    });
}

/// What the interiors of an install state, as counts.
#[derive(Default)]
struct Census {
    type_files: usize,
    interiors: usize,
    rooms: usize,
    portals: usize,
    entities: usize,
    entity_sets: usize,
    set_entities: usize,
    /// Interiors whose room 0 is named `limbo`.
    outside_is_limbo: usize,
    /// Interiors that hold no room beside room 0.
    no_room_inside: usize,
    /// Portals one of whose rooms is room 0, and those of them that name it as the room they lead to.
    portals_outside: usize,
    portals_leading_outside: usize,
    portals_not_a_quad: usize,
    /// Rooms that count other portals than name them.
    rooms_miscounted: usize,
    entities_with_a_lod_parent: usize,
    /// Interiors two of whose rooms, or two of whose entity sets, have one name.
    room_names_shared: usize,
    set_names_shared: usize,
    /// Attachments of an entity to a room that is not room 0, and those whose entity stands outside the box the room states.
    attachments: usize,
    attached_outside_the_box: usize,
    /// Entities whose position is inside the box of no room but room 0, those of them inside the box of no room at all, room 0
    /// among them, and the interiors that hold one of the latter.
    outside_every_room_inside: usize,
    outside_every_room: usize,
    interiors_with_an_entity_outside_every_room: usize,
    /// Locations of entity sets with bit 31 set, and of those the ones whose low bits are below the portal count and at or above the room count.
    locations_flagged: usize,
    flagged_below_the_portals: usize,
    flagged_past_the_rooms: usize,
    interiors_with_a_flagged_location: usize,
    /// How many interiors each rule fires on.
    fired: BTreeMap<&'static str, usize>,
    /// Interiors `interior-broken` fires on.
    broken: usize,
}

fn count(census: &mut Census, mlo: &CMloArchetypeDef) {
    census.interiors += 1;
    census.rooms += mlo.rooms.len();
    census.portals += mlo.portals.len();
    census.entities += mlo.entities.len();
    census.entity_sets += mlo.entity_sets.len();
    let rooms = mlo.rooms.len();
    if (mlo.rooms.get(OUTSIDE)).is_some_and(|room| room.name == "limbo") {
        census.outside_is_limbo += 1;
    }
    if rooms < 2 {
        census.no_room_inside += 1;
    }
    let outside = OUTSIDE as u32;
    let mut named = vec![0u32; rooms];
    for portal in &mlo.portals {
        if portal.room_from == outside || portal.room_to == outside {
            census.portals_outside += 1;
            census.portals_leading_outside += usize::from(portal.room_to == outside);
        }
        census.portals_not_a_quad += usize::from(portal.corners.len() != 4);
        for room in BTreeSet::from([portal.room_from, portal.room_to]) {
            if let Some(count) = named.get_mut(room as usize) {
                *count += 1;
            }
        }
    }
    census.rooms_miscounted += (mlo.rooms.iter().zip(&named))
        .filter(|(room, named)| room.portal_count != **named)
        .count();
    census.entities_with_a_lod_parent += (mlo.entities.iter())
        .filter(|e| e.parent_index >= 0)
        .count();
    let names: BTreeSet<String> = (mlo.rooms.iter())
        .map(|room| room.name.to_ascii_lowercase())
        .collect();
    census.room_names_shared += usize::from(names.len() != rooms);
    let sets: BTreeSet<u32> = mlo.entity_sets.iter().map(|set| set.name.hash).collect();
    census.set_names_shared += usize::from(sets.len() != mlo.entity_sets.len());
    let inside = |at: or_io::decl::Vec3, min: or_io::decl::Vec3, max: or_io::decl::Vec3| {
        let (at, min, max) = (
            [at.x, at.y, at.z],
            [min.x, min.y, min.z],
            [max.x, max.y, max.z],
        );
        (0..3).all(|axis| at[axis] >= min[axis] && at[axis] <= max[axis])
    };
    for room in mlo.rooms.iter().skip(1) {
        for entity in &room.attached_objects {
            let Some(entity) = mlo.entities.get(*entity as usize) else {
                continue;
            };
            census.attachments += 1;
            census.attached_outside_the_box +=
                usize::from(!inside(entity.position, room.bb_min, room.bb_max));
        }
    }
    let mut outside_every_room = 0;
    for entity in &mlo.entities {
        let in_a_room = |room: &or_io::meta::typed::CMloRoomDef| {
            inside(entity.position, room.bb_min, room.bb_max)
        };
        census.outside_every_room_inside += usize::from(!mlo.rooms.iter().skip(1).any(in_a_room));
        outside_every_room += usize::from(!mlo.rooms.iter().any(in_a_room));
    }
    census.outside_every_room += outside_every_room;
    census.interiors_with_an_entity_outside_every_room += usize::from(outside_every_room > 0);
    let mut flagged = false;
    for set in &mlo.entity_sets {
        census.set_entities += set.entities.len();
        for location in set.locations.iter().filter(|l| **l & NOT_A_ROOM != 0) {
            let index = (location & !NOT_A_ROOM) as usize;
            flagged = true;
            census.locations_flagged += 1;
            census.flagged_below_the_portals += usize::from(index < mlo.portals.len());
            census.flagged_past_the_rooms += usize::from(index >= rooms);
        }
    }
    census.interiors_with_a_flagged_location += usize::from(flagged);
}

#[test]
fn the_rules_of_an_interior_are_run_over_every_interior_of_the_install_and_none_that_warns_fires() {
    let Some((game, ytyps)) = mounted() else {
        return;
    };
    let census = Mutex::new(Census::default());
    spread(&ytyps, |logical| {
        let Some(bytes) = game.vfs.open(logical) else {
            return;
        };
        let typed = CMapTypes::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("a type file of the install opens typed: {e}"));
        let Ok(Inside::Types(inside)) = inside_of(&bytes, "ytyp", None) else {
            panic!("a type file of the install reads as one");
        };
        let mut census = census.lock().unwrap();
        census.type_files += 1;
        for archetype in &typed.archetypes {
            if let CBaseArchetypeDefAny::CMloArchetypeDef(mlo) = archetype {
                count(&mut census, mlo);
            }
        }
        // The rules as the rules of a map set run them: off the facts of the file.
        for interior in inside.archetypes.iter().filter_map(|a| a.interior.as_ref()) {
            census.broken += usize::from(!interior.problems.is_empty());
            let fired: BTreeSet<&'static str> =
                interior.findings.iter().map(|found| found.code).collect();
            for code in fired {
                *census.fired.entry(code).or_default() += 1;
            }
        }
    });
    let census = census.into_inner().unwrap();
    let interiors = census.interiors;
    println!(
        "{} type files, {interiors} interiors: {} rooms, {} portals, {} entities, {} entity sets holding {} entities",
        census.type_files,
        census.rooms,
        census.portals,
        census.entities,
        census.entity_sets,
        census.set_entities
    );
    println!(
        "the outside: room 0 is named `limbo` in {} of {interiors} interiors, and {} hold no room beside it; of {} portals that touch room 0, {} name it as the room they lead to",
        census.outside_is_limbo,
        census.no_room_inside,
        census.portals_outside,
        census.portals_leading_outside
    );
    println!(
        "what an edit is held to: {} portal(s) that are not four corners, {} room(s) of {} that count other portals than name them, {} entity(ies) of {} that name a LOD parent, {} interior(s) where two rooms share a name and {} where two entity sets do",
        census.portals_not_a_quad,
        census.rooms_miscounted,
        census.rooms,
        census.entities_with_a_lod_parent,
        census.entities,
        census.room_names_shared,
        census.set_names_shared
    );
    println!(
        "locations with bit 31 set: {} of {} in {} interior(s); the low bits are below the portal count in {} and at or above the room count in {}",
        census.locations_flagged,
        census.set_entities,
        census.interiors_with_a_flagged_location,
        census.flagged_below_the_portals,
        census.flagged_past_the_rooms
    );
    println!(
        "not a rule: the entity stands outside the box of the room that attaches it for {} of {} attachments",
        census.attached_outside_the_box, census.attachments
    );
    println!(
        "not a rule: of {} entities, {} stand outside the box of every room but room 0, and {} outside the box of every room, room 0 among them, in {} of {interiors} interiors",
        census.entities,
        census.outside_every_room_inside,
        census.outside_every_room,
        census.interiors_with_an_entity_outside_every_room
    );
    println!("rules over {interiors} interiors (of which each fires on):");
    println!("  {:30} {}", "interior-broken", census.broken);
    for rule in INTERIOR_RULES {
        let fired = census.fired.get(rule.code).copied().unwrap_or(0);
        println!("  {:30} {fired}", rule.code);
    }

    assert!(interiors > 0, "the install holds interiors");
    assert_eq!(
        census.outside_is_limbo, interiors,
        "room 0 is the outside of every interior"
    );
    assert_eq!(census.no_room_inside, 0);
    assert_eq!(census.portals_leading_outside, census.portals_outside);
    assert_eq!(census.portals_not_a_quad, 0);
    assert_eq!(census.rooms_miscounted, 0);
    assert_eq!(census.entities_with_a_lod_parent, 0);
    assert_eq!((census.room_names_shared, census.set_names_shared), (0, 0));
    assert_eq!(census.broken, 0, "no interior of the game is broken");
    // ⚠ Read as an index of a portal it holds for every one, and as no room for most: what decided that it is not read as a room.
    assert_eq!(census.flagged_below_the_portals, census.locations_flagged);
    assert!(census.flagged_past_the_rooms * 2 > census.locations_flagged);
    // The game's own interiors keep no entity inside the box of its room, so no rule asks it.
    assert!(census.attached_outside_the_box * 4 > census.attachments);
    // ⚠ Nor inside the box of some room, which is another question: few entities stand outside every box, but more than a fifth
    // of the interiors hold one that does, where the one note there is fires on fewer. So no rule and no note asks it.
    assert!(census.outside_every_room * 20 < census.entities);
    assert!(census.interiors_with_an_entity_outside_every_room * 5 > interiors);
    for rule in INTERIOR_RULES {
        let fired = census.fired.get(rule.code).copied().unwrap_or(0);
        match rule.severity {
            // A rule that warns and fires on an interior of the game is wrong, or is a note.
            Severity::Warning | Severity::Error => assert_eq!(fired, 0, "{}", rule.code),
            // The one note: an interior no portal leads into, which the game holds some of.
            Severity::Info => assert!(
                fired > 0 && fired * 5 < interiors,
                "{} fires on {fired} of {interiors}",
                rule.code
            ),
        }
    }
}

/// What became of one sampled type file.
enum Outcome {
    /// Edited, read back and put back; whether the file its source cooks to is the file the game ships, byte for byte.
    Restored { ships_as_cooked: bool },
    /// It did not become a source, with why.
    NotASource(String),
    /// The type file holds no portal, so its table states no class a portal is laid out by, and the first one was refused, with why.
    NoPortalClass(String),
    /// The schema installed in the process states a member a class of the file does not, and the save refused the edit as it
    /// refuses an archetype taken out of the same file; what the save said.
    SchemaStatesMore(String),
    /// The edit was refused, with why.
    Refused(String),
}

/// The first interior of `types`, by its index among the archetypes.
fn first_interior(types: &CMapTypes) -> Option<(usize, &CMloArchetypeDef)> {
    (types.archetypes.iter().enumerate()).find_map(|(at, archetype)| match archetype {
        CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => Some((at, mlo)),
        _ => None,
    })
}

/// The rules that fire on the interior `archetype` of the type file `bytes`.
fn fired(bytes: &[u8], archetype: usize) -> BTreeSet<&'static str> {
    let Ok(Inside::Types(inside)) = inside_of(bytes, "ytyp", None) else {
        panic!("the type file reads as one");
    };
    let interior = inside.archetypes[archetype].interior.as_ref().unwrap();
    assert_eq!(interior.problems, Vec::<String>::new());
    interior.findings.iter().map(|found| found.code).collect()
}

/// Make a source of the type file `bytes`, add a room with a portal to the outside to its first interior, read it back, take the
/// two back, and say what became of it.
fn edit_and_put_back(leaf: &str, bytes: &[u8]) -> Outcome {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let folder = scratch.path().join("maps");
    let incoming = [Incoming {
        name: leaf,
        bytes,
        from_game: true,
        virtual_path: None,
    }];
    let added = match add_files(&folder, &incoming, None) {
        Ok(added) => added,
        Err(why) => return Outcome::NotASource(why),
    };
    if let Some((_, why)) = added.refused.first() {
        return Outcome::NotASource(why.clone());
    }
    let [dir] = added.made.as_slice() else {
        panic!("{} sources were made of one file", added.made.len());
    };
    let source = dir.file_name().unwrap().to_string_lossy().into_owned();
    let xml = dir.join(or_source::metadata::XML);
    let xml_before = std::fs::read(&xml).unwrap();
    let cooked_before = or_maps::facts::cooked(dir).expect("the source cooks");
    let typed = CMapTypes::from_bytes(&cooked_before).expect("what it cooks to opens typed");
    let (archetype, mlo) = first_interior(&typed).expect("the type file holds an interior");
    let (rooms, portals) = (mlo.rooms.len(), mlo.portals.len());
    let portals_of_the_file: usize = (typed.archetypes.iter())
        .map(|archetype| match archetype {
            CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => mlo.portals.len(),
            _ => 0,
        })
        .sum();
    let was_sealed = fired(&cooked_before, archetype).contains("interior-sealed");

    let list = [
        Operation::AddRoom {
            source: source.clone(),
            expects: None,
            archetype,
            room: None,
            members: RoomMembers {
                name: Some("a room no interior of the game has".into()),
                bb_min: Some([-1.0, -1.0, 0.0]),
                bb_max: Some([1.0, 1.0, 2.5]),
                ..RoomMembers::default()
            },
            attached_objects: Vec::new(),
        },
        Operation::AddPortal {
            source: source.clone(),
            expects: None,
            archetype,
            portal: None,
            members: PortalMembers {
                room_from: Some(rooms as u32),
                room_to: Some(OUTSIDE as u32),
                corners: Some(vec![
                    [-1.0, -1.0, 0.0],
                    [1.0, -1.0, 0.0],
                    [1.0, -1.0, 2.25],
                    [-1.0, -1.0, 2.25],
                ]),
                ..PortalMembers::default()
            },
            attached_objects: Vec::new(),
        },
    ];
    let applied = match apply_all(&folder, &list, false) {
        Ok(applied) => applied,
        Err(why) => {
            assert!(
                std::fs::read(&xml).unwrap() == xml_before,
                "{leaf}: a refusal changes nothing"
            );
            let why = why.to_string();
            // ⚠ `or-io` declares no class of a portal for a file that states none, so the first portal of a file is refused: by
            // the save where no schema is installed, and by the source, whose base states no such class, where one is.
            let first_portal = portals_of_the_file == 0 && why.contains("add_portal of");
            let (_, said) = why.split_once(".metadata.src`: ").unwrap_or(("", &why));
            if first_portal {
                let (_, said) = said.rsplit_once("element 0: ").unwrap_or(("", said));
                return Outcome::NoPortalClass(said.to_string());
            }
            // ⚠ A save lays a file whose shape an edit changed out through the schema the process installed, and refuses it
            // where that schema states a member the file's own class does not. It is no refusal of these operations: an
            // archetype taken out of the same file, an operation that was there before them, is refused the same way.
            const STATES_MORE: &str = "that its schema declares";
            let taken_out = Operation::RemoveArchetype {
                source: source.clone(),
                expects: None,
                archetype,
            };
            let as_any_edit = apply_all(&folder, &[taken_out], true)
                .is_err_and(|refused| refused.to_string().contains(STATES_MORE));
            return match why.contains(STATES_MORE) && as_any_edit {
                true => {
                    let (_, said) = said.rsplit_once(": ").unwrap_or(("", said));
                    Outcome::SchemaStatesMore(said.to_string())
                }
                false => Outcome::Refused(why),
            };
        }
    };
    assert_ne!(std::fs::read(&xml).unwrap(), xml_before, "{leaf}");

    // Read back: the room and its portal are there, each of the two rooms counts one portal more, and nothing else is said of
    // the interior but, where no portal led outside before, that its rooms are now cut off from the one that does.
    let cooked = or_maps::facts::cooked(dir).expect("the edited source cooks");
    let edited = CMapTypes::from_bytes(&cooked).expect("the edited file opens typed");
    let (_, now) = first_interior(&edited).unwrap();
    assert_eq!(
        (now.rooms.len(), now.portals.len()),
        (rooms + 1, portals + 1),
        "{leaf}"
    );
    let added = &now.rooms[rooms];
    assert_eq!(added.name, "a room no interior of the game has");
    assert_eq!(added.portal_count, 1);
    assert_eq!(
        now.rooms[OUTSIDE].portal_count,
        mlo.rooms[OUTSIDE].portal_count + 1
    );
    let door = &now.portals[portals];
    assert_eq!(
        (door.room_from as usize, door.room_to as usize),
        (rooms, OUTSIDE)
    );
    let expected: BTreeSet<&str> = match was_sealed {
        true => BTreeSet::from(["interior-room-unreachable"]),
        false => BTreeSet::new(),
    };
    assert_eq!(fired(&cooked, archetype), expected, "{leaf}");

    // Undone: the inverses, the last first, each an operation of the vocabulary and none the XML as it was.
    let back = inverses(&applied);
    assert_eq!(
        back.iter().map(Operation::name).collect::<Vec<_>>(),
        ["remove_portal", "remove_room"],
        "{leaf}"
    );
    apply_all(&folder, &back, false)
        .unwrap_or_else(|e| panic!("{leaf}: the inverses are refused: {e}"));
    assert!(
        std::fs::read(&xml).unwrap() == xml_before,
        "{leaf}: the XML is not as it was"
    );
    let cooked_after = or_maps::facts::cooked(dir).expect("the source cooks again");
    assert!(
        cooked_after == cooked_before,
        "{leaf}: the file is not the one the source cooked to"
    );
    Outcome::Restored {
        ships_as_cooked: cooked_before == bytes,
    }
}

#[test]
fn a_room_with_a_portal_to_the_outside_is_added_to_sampled_interiors_read_back_and_put_back() {
    // With no schema installed, as `openrage-cli maps apply` runs.
    assert!(!or_io::meta::has_default_library());
    let Some((restored, states_more, sample)) = sampled() else {
        return;
    };
    assert_eq!(states_more, 0, "no schema is installed to state more");
    assert!(
        restored * 2 > sample,
        "{restored} of {sample} were edited and put back"
    );
}

/// ⚠ The first schema installed in a process is the one it keeps, so this is a test of its own: under nextest, a process of its own.
#[test]
fn the_same_edit_is_taken_and_put_back_with_the_schema_snapshot_installed_as_the_app_installs_it() {
    if game_roots().is_empty() {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    }
    let Some(snapshot) = or_test_support::schema_snapshot() else {
        or_test_support::skip_schema(WHAT);
        return;
    };
    or_io::meta::install_snapshot(&snapshot).expect("the staged schema snapshot installs");
    assert!(or_io::meta::has_default_library());
    let Some((restored, states_more, sample)) = sampled() else {
        return;
    };
    // ⚠ One snapshot is staged, and it states the classes of one edition: the files of the edition it does not state take no
    // edit that changes their shape while it is installed, and the files of the other take this one.
    assert!(
        (states_more == 0 && restored * 2 > sample) || (restored == 0 && states_more == sample),
        "{restored} of {sample} were edited and put back, and {states_more} were refused by the schema installed"
    );
}

/// Sample the type files of the install that hold an interior, edit each and put it back, and hold that each was put back or was
/// refused unchanged for a reason that is not these operations'; how many were put back, how many the installed schema refused,
/// and how many were sampled; none, and a skip recorded, when the run was pointed at no install.
fn sampled() -> Option<(usize, usize, usize)> {
    let (game, ytyps) = mounted()?;
    // The type files that hold an interior, in the order of their paths.
    let holding = Mutex::new(Vec::new());
    spread(&ytyps, |logical| {
        let Some(bytes) = game.vfs.open(logical) else {
            return;
        };
        let held =
            CMapTypes::from_bytes(&bytes).is_ok_and(|typed| first_interior(&typed).is_some());
        if held {
            holding.lock().unwrap().push(logical.clone());
        }
    });
    let mut holding = holding.into_inner().unwrap();
    holding.sort();
    let stride = (holding.len() / SAMPLED).max(1);
    let sample: Vec<&String> = holding.iter().step_by(stride).take(SAMPLED).collect();

    let outcomes = Mutex::new(Vec::new());
    spread(&sample, |logical| {
        let bytes = game.vfs.open(logical).expect("the file was read before");
        let leaf = logical.rsplit('/').next().unwrap_or(logical);
        let outcome = edit_and_put_back(leaf, &bytes);
        outcomes.lock().unwrap().push(outcome);
    });
    let outcomes = outcomes.into_inner().unwrap();
    let (mut restored, mut as_shipped) = (0, 0);
    let mut no_portal_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut states_more: BTreeMap<String, usize> = BTreeMap::new();
    let mut not_sources: BTreeMap<String, usize> = BTreeMap::new();
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for outcome in &outcomes {
        match outcome {
            Outcome::Restored { ships_as_cooked } => {
                restored += 1;
                as_shipped += usize::from(*ships_as_cooked);
            }
            Outcome::NotASource(why) => *not_sources.entry(why.clone()).or_default() += 1,
            Outcome::NoPortalClass(why) => *no_portal_class.entry(why.clone()).or_default() += 1,
            Outcome::SchemaStatesMore(why) => *states_more.entry(why.clone()).or_default() += 1,
            Outcome::Refused(why) => *refused.entry(why.clone()).or_default() += 1,
        }
    }
    println!(
        "{} type files hold an interior; {} sampled: {restored} took a room with a portal to the outside, read back and were put back to the XML and the file their source cooked to, which for {as_shipped} of them is the file the game ships byte for byte",
        holding.len(),
        sample.len()
    );
    for (why, count) in &no_portal_class {
        println!(
            "  {count} hold no portal in the whole file, so the first one was refused and nothing was changed: {why}"
        );
    }
    for (why, count) in &states_more {
        println!(
            "  {count} were refused unchanged by the save, as an archetype taken out of the same file is, since the schema installed states more than the file: {why}"
        );
    }
    for (why, count) in &not_sources {
        println!("  {count} did not become a source: {why}");
    }
    for (why, count) in &refused {
        println!("  {count} refused the edit: {why}");
    }
    assert_eq!(outcomes.len(), sample.len());
    assert!(sample.len() >= SAMPLED.min(holding.len()));
    assert_eq!(
        refused,
        BTreeMap::new(),
        "an interior of the game takes a room and a portal"
    );
    // A type file that is no source is the import's to refuse, and is no edit that failed.
    Some((restored, states_more.values().sum(), sample.len()))
}
