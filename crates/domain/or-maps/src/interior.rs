//! An interior a type file declares, read: the rules that say what is wrong inside it, and what it holds as a listing.
//!
//! A rule is a row of [`INTERIOR_RULES`]. [`check`] reads one interior through `or-world`'s read view and gives a [`Found`] for each
//! rule that applies to it, with the places named in its sentence; [`crate::check`] says them among the findings of a map set, beside
//! `interior-broken`, which holds the indexes a save refuses. The operations that edit an interior are in [`crate::operation`].
//!
//! ⚠ **Each rule is one the installs keep, and the one they do not is a note.** Measured over every interior of the Legacy and of the
//! Enhanced install (`what_the_interior_rules_say_of_the_install`, 2026-11-25): room 0 is the outside, named `limbo` in all 541 and
//! all 556 interiors; no rule that warns fires on one; and 52 of the 541 and 54 of the 556 have no portal to the outside, which is
//! `interior-sealed` and only a note. In those the rooms are joined to nothing a person walks in from, so `interior-room-unreachable`
//! is asked only of an interior that opens to the outside.
//!
//! ⚠ **No rule asks whether an entity stands inside the box of its room**, because the installs do not keep it: the position of the
//! entity is outside the box of the room that attaches it for 35,671 of 64,841 attachments of the Legacy install and 35,782 of 65,220
//! of the Enhanced one (the same suite and date). Nor does one ask whether it stands inside the box of some room: 867 of 67,543 and
//! 867 of 67,971 entities stand outside the box of every room, room 0 among them, but in 135 of the 541 and 135 of the 556 interiors
//! (the same suite, 2026-11-26), more than the note fires on.

use std::collections::{BTreeMap, BTreeSet};

use or_io::meta::MetaDocument;
use or_world_gta5::map_types::Interior;
use or_world_gta5::{keys, MapTypes};
use serde::Serialize;

use crate::facts::{name_at, Name, Names};
use crate::operation::{NOT_A_ROOM, OUTSIDE};
use crate::rules::{Rule, Severity};

use Severity::{Info, Warning};

/// The rules of an interior, in the order a report lists them.
pub const INTERIOR_RULES: &[Rule] = &[
    Rule::new(
        "interior-portal-rooms-same",
        Warning,
        true,
        "a portal of an interior names one room on both sides",
    ),
    Rule::new(
        "interior-portal-count",
        Warning,
        true,
        "a room of an interior counts another number of portals than name it",
    ),
    Rule::new(
        "interior-set-room-missing",
        Warning,
        true,
        "an entity set of an interior places an entity in a room the interior does not hold",
    ),
    Rule::new(
        "interior-entity-in-two-rooms",
        Warning,
        true,
        "an entity of an interior is attached to two rooms",
    ),
    Rule::new(
        "interior-entity-unattached",
        Warning,
        true,
        "an entity of an interior is attached to no room and no portal",
    ),
    Rule::new(
        "interior-room-unreachable",
        Warning,
        true,
        "a room of an interior that opens to the outside is joined to the outside by no chain of portals",
    ),
    Rule::new(
        "interior-sealed",
        Info,
        true,
        "no portal of an interior leads to the outside",
    ),
];

/// One rule that applies to an interior, with the places it applies to in its sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub code: &'static str,
    pub message: String,
}

/// How many places a sentence names before it says how many more there are.
const NAMED: usize = 5;

/// `places` as a sentence names them: the first few, then how many more.
fn some_of(places: &[String]) -> String {
    let named = places[..places.len().min(NAMED)].join(", ");
    match places.len().saturating_sub(NAMED) {
        0 => named,
        more => format!("{named} and {more} more"),
    }
}

fn numbers(of: impl IntoIterator<Item = usize>) -> Vec<String> {
    of.into_iter().map(|index| format!("#{index}")).collect()
}

/// Every rule of [`INTERIOR_RULES`] that applies to `interior`, one finding a rule.
pub fn check(interior: &Interior<'_>) -> Vec<Found> {
    let mut found = Vec::new();
    let mut say = |code: &'static str, message: String| found.push(Found { code, message });
    let rooms = interior.room_count();
    let entities = interior.entity_count();
    let portals: Vec<(usize, usize)> = (interior.portals())
        .map(|portal| (portal.room_from() as usize, portal.room_to() as usize))
        .collect();

    let same = (portals.iter().enumerate())
        .filter(|(_, (from, to))| from == to)
        .map(|(portal, _)| portal);
    let same = numbers(same);
    if !same.is_empty() {
        say(
            "interior-portal-rooms-same",
            format!(
                "{} portal(s) name one room on both sides: {}",
                same.len(),
                some_of(&same)
            ),
        );
    }

    // The portals that name each room, a portal once.
    let mut named = vec![0u32; rooms];
    for (from, to) in &portals {
        for room in BTreeSet::from([*from, *to]) {
            if let Some(count) = named.get_mut(room) {
                *count += 1;
            }
        }
    }
    let miscounted: Vec<String> = (interior.rooms().zip(&named).enumerate())
        .filter(|(_, (room, named))| room.portal_count() != **named)
        .map(|(at, (room, named))| {
            format!("#{at} counts {} where {named} name it", room.portal_count())
        })
        .collect();
    if !miscounted.is_empty() {
        say(
            "interior-portal-count",
            format!(
                "{} room(s) count another number of portals than name them: {}",
                miscounted.len(),
                some_of(&miscounted)
            ),
        );
    }

    let mut misplaced = Vec::new();
    for (set, held) in interior.entity_sets().enumerate() {
        // ⚠ A location with bit 31 set is no room index, and is not held to the rooms.
        let past = (held.locations())
            .filter(|l| l & NOT_A_ROOM == 0 && *l as usize >= rooms)
            .collect::<Vec<_>>();
        if let Some(first) = past.first() {
            misplaced.push(format!(
                "set #{set} places {} in a room past the {rooms} the interior holds, the first in room {first}",
                past.len()
            ));
        }
    }
    if !misplaced.is_empty() {
        say(
            "interior-set-room-missing",
            format!(
                "{} entity set(s) place an entity in a room the interior does not hold: {}",
                misplaced.len(),
                some_of(&misplaced)
            ),
        );
    }

    // The rooms that attach each entity, and whether a portal does. An index past the entities is `interior-broken`.
    let mut in_rooms: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut attached = vec![false; entities];
    for (room, held) in interior.rooms().enumerate() {
        for entity in held.attached_objects().map(|e| e as usize) {
            if let Some(attached) = attached.get_mut(entity) {
                *attached = true;
                in_rooms.entry(entity).or_default().insert(room);
            }
        }
    }
    for entity in interior.portals().flat_map(|p| p.attached_objects()) {
        if let Some(attached) = attached.get_mut(entity as usize) {
            *attached = true;
        }
    }
    let twice: Vec<String> = (in_rooms.iter())
        .filter(|(_, rooms)| rooms.len() > 1)
        .map(|(entity, rooms)| {
            let rooms: Vec<String> = rooms.iter().map(ToString::to_string).collect();
            format!("#{entity} to rooms {}", rooms.join(" and "))
        })
        .collect();
    if !twice.is_empty() {
        say(
            "interior-entity-in-two-rooms",
            format!(
                "{} entity(ies) are attached to two rooms: {}",
                twice.len(),
                some_of(&twice)
            ),
        );
    }
    let loose = (attached.iter().enumerate())
        .filter(|(_, attached)| !**attached)
        .map(|(entity, _)| entity);
    let loose = numbers(loose);
    if !loose.is_empty() {
        say(
            "interior-entity-unattached",
            format!(
                "{} entity(ies) are attached to no room and no portal: {}",
                loose.len(),
                some_of(&loose)
            ),
        );
    }

    // The rooms a chain of portals joins to the outside. A portal that names a room past the rooms is `interior-broken`.
    let mut reached = vec![false; rooms];
    let mut next = vec![OUTSIDE];
    while let Some(room) = next.pop() {
        if room >= rooms || std::mem::replace(&mut reached[room], true) {
            continue;
        }
        for (from, to) in &portals {
            if *from == room {
                next.push(*to);
            }
            if *to == room {
                next.push(*from);
            }
        }
    }
    let opens = (portals.iter()).any(|(from, to)| {
        (*from == OUTSIDE || *to == OUTSIDE) && from != to && *from.max(to) < rooms
    });
    let cut_off = numbers((0..rooms).filter(|room| !reached[*room]));
    if opens && !cut_off.is_empty() {
        say(
            "interior-room-unreachable",
            format!(
                "{} room(s) are joined to the outside by no chain of portals: {}",
                cut_off.len(),
                some_of(&cut_off)
            ),
        );
    }
    if !opens && rooms > 1 {
        say(
            "interior-sealed",
            format!(
                "none of its {} portal(s) leads to the outside, room {OUTSIDE}",
                portals.len()
            ),
        );
    }
    found
}

/// A room of an interior, as a listing states it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListedRoom {
    pub name: String,
    pub bb_min: [f32; 3],
    pub bb_max: [f32; 3],
    pub flags: u32,
    pub floor_id: i32,
    pub portal_count: u32,
    /// The entities of the interior it attaches, by their index.
    pub attached_objects: Vec<u32>,
}

/// A portal of an interior, as a listing states it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListedPortal {
    pub room_from: u32,
    pub room_to: u32,
    pub flags: u32,
    pub opacity: u32,
    pub corners: Vec<[f32; 3]>,
    pub attached_objects: Vec<u32>,
}

/// An entity an entity set holds: the archetype it places, as its text or `hash_XXXXXXXX`, and the location the set states for it, none where its two arrays differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedSetEntity {
    pub archetype: String,
    pub location: Option<u32>,
}

/// An entity set of an interior, as a listing states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedSet {
    /// Its name, as its text or `hash_XXXXXXXX`.
    pub name: String,
    pub entities: Vec<ListedSetEntity>,
}

/// What an interior holds, with what the rules of [`INTERIOR_RULES`] find in it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listed {
    pub entities: usize,
    pub rooms: Vec<ListedRoom>,
    pub portals: Vec<ListedPortal>,
    pub entity_sets: Vec<ListedSet>,
    pub time_cycle_modifiers: usize,
    /// What a save refuses in it, a sentence each: the rule `interior-broken`.
    pub broken: Vec<String>,
    pub findings: Vec<Found>,
}

/// The name `hash` is, with its text where `names` has one that hashes to it.
fn shown(hash: u32, names: Names<'_>) -> String {
    let text = names
        .and_then(|names| names(hash))
        .filter(|text| or_io::string_hash(text.as_bytes()) == hash);
    Name { hash, text }.shown()
}

/// What `interior` holds; a hash is shown by its text where `names` has it.
pub fn listed(interior: &Interior<'_>, names: Names<'_>) -> Listed {
    Listed {
        entities: interior.entity_count(),
        rooms: (interior.rooms())
            .map(|room| {
                let (min, max) = room.bounds().unwrap_or_default();
                ListedRoom {
                    name: room.name().unwrap_or_default(),
                    bb_min: min.to_array(),
                    bb_max: max.to_array(),
                    flags: room.flags(),
                    floor_id: room.floor_id(),
                    portal_count: room.portal_count(),
                    attached_objects: room.attached_objects().collect(),
                }
            })
            .collect(),
        portals: (interior.portals())
            .map(|portal| ListedPortal {
                room_from: portal.room_from(),
                room_to: portal.room_to(),
                flags: portal.flags(),
                opacity: portal.opacity(),
                corners: portal.corners().iter().map(|c| c.to_array()).collect(),
                attached_objects: portal.attached_objects().collect(),
            })
            .collect(),
        entity_sets: (interior.entity_sets())
            .map(|set| {
                // ⚠ By place, and never by a zip: a set whose two arrays differ is listed with what each entity has.
                let locations: Vec<u32> = set.locations().collect();
                ListedSet {
                    name: shown(set.name(), names),
                    entities: (set.entities().enumerate())
                        .map(|(at, entity)| ListedSetEntity {
                            archetype: shown(entity.archetype(), names),
                            location: locations.get(at).copied(),
                        })
                        .collect(),
                }
            })
            .collect(),
        time_cycle_modifiers: interior.time_cycle_modifier_count(),
        broken: (or_world_gta5::validate::check_interior(interior).iter())
            .map(ToString::to_string)
            .collect(),
        findings: check(interior),
    }
}

/// One interior of a type file: its index among the archetypes of the file, its name and what it holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InFile {
    pub archetype: usize,
    /// Its name, as its text or `hash_XXXXXXXX`.
    pub name: String,
    #[serde(flatten)]
    pub listed: Listed,
}

/// The interiors the type file `bytes` declares, in the order of its archetypes.
pub fn interiors_of(bytes: &[u8], names: Names<'_>) -> Result<Vec<InFile>, String> {
    let document = MetaDocument::parse(or_io::meta::default_library(), bytes.to_vec())
        .map_err(|e| format!("does not read as a metadata document: {e}"))?;
    let types = MapTypes::new(&document.instance)
        .ok_or_else(|| "is no type file: its root is not a CMapTypes".to_string())?;
    let interiors = types
        .archetypes()
        .enumerate()
        .filter_map(|(at, archetype)| {
            let interior = archetype.interior()?;
            Some(InFile {
                archetype: at,
                name: name_at(archetype.instance(), keys::NAME, names).shown(),
                listed: listed(&interior, names),
            })
        });
    Ok(interiors.collect())
}

impl Listed {
    /// The listing as lines of text, a room, a portal, an entity set and a finding each.
    pub fn lines(&self) -> Vec<String> {
        let box_of = |[x, y, z]: [f32; 3]| format!("{x},{y},{z}");
        let attached = |attached: &[u32]| match attached {
            [] => String::new(),
            held => format!(
                ", attaches {}",
                some_of(&numbers(held.iter().map(|e| *e as usize)))
            ),
        };
        let mut lines = vec![format!(
            "{} entity(ies), {} room(s), {} portal(s), {} entity set(s), {} time cycle modifier(s)",
            self.entities,
            self.rooms.len(),
            self.portals.len(),
            self.entity_sets.len(),
            self.time_cycle_modifiers
        )];
        for (at, room) in self.rooms.iter().enumerate() {
            lines.push(format!(
                "room {at} `{}`: box {} to {}, flags {}, floor {}, counts {} portal(s){}",
                room.name,
                box_of(room.bb_min),
                box_of(room.bb_max),
                room.flags,
                room.floor_id,
                room.portal_count,
                attached(&room.attached_objects)
            ));
        }
        for (at, portal) in self.portals.iter().enumerate() {
            let corners: Vec<String> = portal.corners.iter().map(|c| box_of(*c)).collect();
            lines.push(format!(
                "portal {at}: room {} to room {}, flags {}, opacity {}, corners {}{}",
                portal.room_from,
                portal.room_to,
                portal.flags,
                portal.opacity,
                corners.join(" | "),
                attached(&portal.attached_objects)
            ));
        }
        for (at, set) in self.entity_sets.iter().enumerate() {
            let held: Vec<String> = (set.entities.iter())
                .map(|entity| match entity.location {
                    Some(location) => format!("`{}` in {location}", entity.archetype),
                    None => format!("`{}` with no location", entity.archetype),
                })
                .collect();
            lines.push(format!(
                "entity set {at} `{}`: {} entity(ies){}{}",
                set.name,
                held.len(),
                if held.is_empty() { "" } else { ": " },
                some_of(&held)
            ));
        }
        for broken in &self.broken {
            lines.push(format!("{:<7} {:<28} {broken}", "error", "interior-broken"));
        }
        for finding in &self.findings {
            let word = match INTERIOR_RULES.iter().find(|rule| rule.code == finding.code) {
                Some(rule) if rule.severity == Info => "note",
                Some(rule) if rule.severity == Warning => "warning",
                _ => "error",
            };
            lines.push(format!(
                "{word:<7} {:<28} {}",
                finding.code, finding.message
            ));
        }
        lines
    }
}
