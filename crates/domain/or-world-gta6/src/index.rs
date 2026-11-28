//! The index of a GTA VI install the world viewer streams from: the maps each pack registers with
//! their boxes, each archetype by its word, and where each model's body lies. It is built by one
//! pass over the world's archives on every core, and kept in the configuration folder by the
//! kept file every title shares: an archive is read again when its size or modification time
//! changes, and no other is.
//!
//! ⚠ A map's entities are not in it: a map's body is read when the map streams in.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use or_title::gta6_mount::{self, HeldBack, MapFact, Mode, SCRIPT_MANAGED};
use or_world::streaming::MapBox;

#[path = "index_install.rs"]
mod install;
#[path = "index_mount.rs"]
mod mount;
#[path = "index_open.rs"]
mod open;
#[path = "index_rows.rs"]
mod rows;
#[path = "index_scan.rs"]
mod scan;
#[path = "index_states.rs"]
mod states;
use or_io::gta6::map::index_types as types;
use or_io::gta6::map::lights_index;

pub use install::{Linked, RULES};
pub use mount::keys;
pub use open::{build, fresh_at, kept_at, open, Opened, GAME, TITLE};
pub use types::ALL_HOURS;

/// A map of the index.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MapRow {
    /// Which archive of the index it lies in.
    pub archive: u32,
    pub name: u32,
    pub block: u32,
    pub flags: u32,
    pub content_flags: u32,
    /// Why it does not stream once its mode is on: 0 where it does, 1 no pack registers it, 2 script managed, 3 empty.
    pub held: u32,
    /// The names of the maps its level-of-detail parents lie in: a run of the index's parent names.
    pub parents_from: u32,
    pub parents_len: u32,
    /// The group a script-managed map is switched with, by the group's id; 0 for a map that is not script managed.
    pub group: u32,
    /// The modes whose packs register it: bit 0 the base game, bit 1 story, bit 2 the shared minigames.
    pub modes: u32,
    pub offset: u64,
    /// The streaming box, lowest corner then highest, then the entities' box.
    pub boxes: [f32; 12],
}

impl MapRow {
    pub fn boxes(&self) -> MapBox {
        let at = |k: usize| Vec3::from_slice(&self.boxes[3 * k..3 * k + 3]);
        MapBox {
            streaming: (at(0), at(1)),
            entities: (at(2), at(3)),
        }
    }
}

/// An archetype of the index, by its word.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct ArchetypeRow {
    pub word: u64,
    /// The word of the drawable or fragment that draws it; 0 for an interior or a dictionary's.
    pub model: u64,
    /// The word of the drawable dictionary that holds it under its name; 0 for none.
    pub dictionary: u64,
    pub name: u32,
    pub lod_dist: f32,
    pub centre: [f32; 3],
    pub radius: f32,
    pub flags: u32,
    /// The hours it is on, bit `h` for the hour `h` of 0 to 23; all 24 for one that is no time archetype.
    pub hours: u32,
    /// The low half of its own load flags: bit 0 terrain, bit 7 environment, bit 16 vegetation.
    pub load_flags: u32,
    pub pad: u32,
}

/// Where a model's body lies, by the word that names it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct BodyRow {
    pub word: u64,
    pub offset: u64,
    pub archive: u32,
    pub pad: u32,
}

/// The body that draws an archetype, and the entry of it where the body is a dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelRef {
    pub archive: u32,
    pub offset: u64,
    pub entry: Option<u32>,
}

/// What the mount and the build came to, for the screen to say.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Report {
    pub packs: u64,
    pub sets_read: u64,
    pub sets_unread: u64,
    pub named_archives: u64,
    pub bound_archives: u64,
    pub maps: u64,
    /// The maps the base game registers, and of those the ones at rest, script managed and empty.
    pub registered: u64,
    pub at_rest: u64,
    pub script_managed: u64,
    pub empty: u64,
    pub archetypes: u64,
    pub models: u64,
    /// Bodies shaped as a map, a map types or a dictionary that did not read.
    pub unread: u64,
    /// The maps the story packs register, and the shared minigames'.
    pub story: u64,
    pub minigames: u64,
    /// The groups of script-managed maps.
    pub groups: u64,
    /// 1 where the archives' tables bound the maps, 0 where the block maps did.
    pub by_table: u64,
    /// The maps of the base game the tables and the block maps bind differently.
    pub disagree: u64,
    /// The level's change sets whose file a table lists under a cipher of its own.
    pub sets_ciphered: u64,
    /// The permanent map-types files the sets name, and those whose archive holds a map-types body.
    pub permanent_named: u64,
    pub permanent_found: u64,
    /// The world-state selectors the game-shape archives hold.
    pub selectors: u64,
}

/// What of the world is switched on: the game modes, and the groups of script-managed maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enabled {
    /// Bit 0 the base game, bit 1 story, bit 2 the shared minigames.
    pub modes: u8,
    /// The ids of the script-managed groups switched on.
    pub groups: Vec<u32>,
}

impl Default for Enabled {
    fn default() -> Self {
        Self {
            modes: Mode::Base.bit(),
            groups: Vec::new(),
        }
    }
}

impl Enabled {
    /// Whether `row` streams: a mode that registers it is on, it holds something, and where it is script managed its group is on.
    pub fn allows(&self, row: &MapRow) -> bool {
        row.modes & u32::from(self.modes) != 0
            && match row.held {
                0 => true,
                2 => self.groups.contains(&row.group),
                _ => false,
            }
    }
}

/// A group of script-managed maps that are switched together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    /// What [`Enabled::groups`] and [`MapRow::group`] name it by.
    pub id: u32,
    /// The 32-bit hash of its name: the id itself, which is the name its maps state, or the map's own name for a map that states none.
    pub name: u32,
    /// How many maps it holds.
    pub maps: u32,
    /// The 64-bit word of its name, which the world-state data names it by; 0 where no map states one.
    pub word: u64,
    /// The hours it is on at rest under each set of modes by the data alone: bit `h` of `hours[modes]` for the hour `h`.
    pub hours: [u32; 8],
    /// The same with what a fresh game is taken to have beyond the data.
    pub inferred: [u32; 8],
}

impl Group {
    /// Whether what a fresh game is taken to have turns it on or off at `hour` under `modes`, against the data alone.
    pub fn by_inference(&self, hour: f32, modes: u8) -> bool {
        let (bit, modes) = (hour_bit(hour), usize::from(modes & 0b111));
        (self.hours[modes] ^ self.inferred[modes]) & bit != 0
    }
}

/// The bit of the whole hour of `hour`, a day past coming round again and a NaN hour being midnight.
fn hour_bit(hour: f32) -> u32 {
    1 << (hour.rem_euclid(24.0) as u32).min(23)
}

/// A group's word and the hours the world-state data has it on, alone and with what a fresh game is taken to have, as the index keeps them.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
struct GroupRow {
    word: u64,
    id: u32,
    pad: u32,
    hours: [u32; 8],
    inferred: [u32; 8],
}

/// A place the world-state data names, as the index keeps it: the hashes of its name and its label, its kind, its run of the index's outline points and its area.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct PlaceRow {
    pub name: u32,
    pub label: u32,
    pub kind: i32,
    from: u32,
    len: u32,
    pub area: f32,
}

/// The index of one install.
#[derive(Debug, Default)]
pub struct WorldIndex {
    pub archives: Vec<PathBuf>,
    pub maps: Vec<MapRow>,
    pub parents: Vec<u32>,
    archetypes: Vec<ArchetypeRow>,
    models: Vec<BodyRow>,
    dictionaries: Vec<BodyRow>,
    interiors: Vec<BodyRow>,
    lights: Vec<lights_index::LightRow>,
    /// The archetype word each far stand-in id names, by the id; 0 for an id no list settles.
    pub slod: Vec<u64>,
    /// Each group's word and hours at rest, by id.
    switches: Vec<GroupRow>,
    /// The places the world-state data names with an outline, and the points of the outlines.
    places: Vec<PlaceRow>,
    outlines: Vec<[f32; 2]>,
    pub report: Report,
    by_name: HashMap<u32, u32>,
    groups: Vec<Group>,
}

fn by_word<T>(rows: &[T], word: u64, of: impl Fn(&T) -> u64) -> Option<&T> {
    let at = rows.binary_search_by_key(&word, of).ok()?;
    rows.get(at)
}

impl WorldIndex {
    /// The places the world-state data names that hold `point`, smallest first.
    pub fn places_at(&self, point: [f32; 2]) -> Vec<&PlaceRow> {
        let mut found: Vec<&PlaceRow> = (self.places.iter())
            .filter(|p| {
                // ⚠ A kept file's run is not trusted: one past the points names no outline.
                let (from, len) = (p.from as usize, p.len as usize);
                let outline = self.outlines.get(from..from.saturating_add(len));
                outline.is_some_and(|o| or_io::gta6::world_state::outline_holds(o, point))
            })
            .collect();
        found.sort_by(|a, b| a.area.total_cmp(&b.area));
        found
    }

    /// Lay `places` as the index keeps them.
    fn lay_places(&mut self, places: Vec<or_io::gta6::world_state::Place>) {
        for place in places {
            self.places.push(PlaceRow {
                name: place.name,
                label: place.label,
                kind: place.kind,
                from: self.outlines.len() as u32,
                len: place.outline.len() as u32,
                area: place.area(),
            });
            self.outlines.extend(place.outline);
        }
    }

    pub fn archetype(&self, word: u64) -> Option<&ArchetypeRow> {
        by_word(&self.archetypes, word, |a| a.word)
    }

    /// Where the map-types body that states the interior `word` names lies.
    pub fn interior(&self, word: u64) -> Option<&BodyRow> {
        by_word(&self.interiors, word, |b| b.word)
    }

    /// The body that draws `archetype`; `None` for an interior, or a model no body of the index names.
    pub fn model(&self, archetype: &ArchetypeRow) -> Option<ModelRef> {
        let (rows, word, entry) = match archetype.dictionary {
            0 => (&self.models, archetype.model, None),
            word => (&self.dictionaries, word, Some(archetype.name)),
        };
        let body = by_word(rows, word, |b| b.word).filter(|_| word != 0)?;
        Some(ModelRef {
            archive: body.archive,
            offset: body.offset,
            entry,
        })
    }

    /// Each table of the index by name, with how many rows it holds and their bytes.
    #[cfg(any(test, feature = "made-up"))]
    pub fn tables(&self) -> Vec<(&'static str, usize, &[u8])> {
        fn of<'a, T: Pod>(name: &'static str, rows: &'a [T]) -> (&'static str, usize, &'a [u8]) {
            (name, rows.len(), bytemuck::cast_slice(rows))
        }
        vec![
            of("maps", &self.maps),
            of("parents", &self.parents),
            of("archetypes", &self.archetypes),
            of("models", &self.models),
            of("dictionaries", &self.dictionaries),
            of("interiors", &self.interiors),
            of("lights", &self.lights),
            of("slod", &self.slod),
            of("switches", &self.switches),
            of("places", &self.places),
            of("outlines", &self.outlines),
            of("report", std::slice::from_ref(&self.report)),
        ]
    }

    /// Every archetype of the index, by its word.
    #[cfg(any(test, feature = "made-up"))]
    pub fn archetypes(&self) -> &[ArchetypeRow] {
        &self.archetypes
    }

    /// The map of the name `name`, by its place among the maps.
    pub fn map_named(&self, name: u32) -> Option<usize> {
        self.by_name.get(&name).map(|at| *at as usize)
    }

    pub fn parents_of(&self, map: &MapRow) -> &[u32] {
        let from = map.parents_from as usize;
        (self.parents.get(from..from + map.parents_len as usize)).unwrap_or_default()
    }

    /// The groups of script-managed maps, by id.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The ids of the groups the world-state data switches on at `hour` under the selectors of the modes `modes`, as [`Enabled::modes`] states them: with what a fresh game is taken to have, or by the data alone where `proven`.
    pub fn default_on_in(&self, hour: f32, modes: u8, proven: bool) -> Vec<u32> {
        let bit = hour_bit(hour);
        let modes = usize::from(modes & 0b111);
        (self.groups.iter())
            .filter(|g| (if proven { g.hours } else { g.inferred })[modes] & bit != 0)
            .map(|g| g.id)
            .collect()
    }

    /// The lights archetype `word` holds, in its model's space.
    pub fn archetype_lights(&self, word: u64) -> &[lights_index::LightRow] {
        lights_index::of_archetype(&self.lights, word)
    }

    /// The middle of the boxes of the maps that stream at rest, where a camera starts.
    pub fn middle(&self) -> Option<Vec3> {
        let at_rest = Enabled::default();
        let mut sum = glam::DVec3::ZERO;
        let mut count = 0u32;
        for map in self.maps.iter().filter(|m| at_rest.allows(m)) {
            let b = map.boxes();
            let centre = (b.entities.0 + b.entities.1) / 2.0;
            if centre.is_finite() {
                sum += centre.as_dvec3();
                count += 1;
            }
        }
        (count > 0).then(|| (sum / f64::from(count)).as_vec3())
    }

    fn finish(mut self) -> Self {
        self.archetypes.sort_unstable_by_key(|a| a.word);
        // Of two bodies that state one archetype the later one stands.
        self.archetypes.reverse();
        self.archetypes.dedup_by_key(|a| a.word);
        self.archetypes.reverse();
        lights_index::settle(&mut self.lights);
        for rows in [
            &mut self.models,
            &mut self.dictionaries,
            &mut self.interiors,
        ] {
            rows.sort_unstable_by_key(|b| (b.word, b.archive, b.offset));
            rows.dedup_by_key(|b| b.word);
        }
        self.by_name = (self.maps.iter().enumerate())
            .map(|(at, m)| (m.name, at as u32))
            .collect();
        let mut groups = BTreeMap::<u32, u32>::new();
        for map in (self.maps.iter()).filter(|m| m.group != 0) {
            *groups.entry(map.group).or_default() += 1;
        }
        self.switches.sort_unstable_by_key(|s| s.id);
        self.switches.dedup_by_key(|s| s.id);
        let switches = &self.switches;
        self.groups = (groups.into_iter())
            .map(|(id, maps)| {
                let row = by_word(switches, u64::from(id), |s| u64::from(s.id));
                Group {
                    id,
                    name: id,
                    maps,
                    word: row.map_or(0, |s| s.word),
                    hours: row.map_or([0; 8], |s| s.hours),
                    inferred: row.map_or([0; 8], |s| s.inferred),
                }
            })
            .collect();
        self.report.archetypes = self.archetypes.len() as u64;
        self.report.models = (self.models.len() + self.dictionaries.len()) as u64;
        self.report.groups = self.groups.len() as u64;
        self
    }
}

/// Give each map what the mount says of it: the modes that register it, why it is held back, and
/// a group of its own where it is script managed and states none.
fn place(maps: &mut [MapRow], facts: &[MapFact], modes: &[u8]) {
    for ((row, fact), modes) in maps.iter_mut().zip(facts).zip(modes) {
        row.modes = u32::from(*modes);
        row.held = match gta6_mount::held_back(*modes != 0, fact) {
            None => 0,
            Some(HeldBack::NotRegistered) => 1,
            Some(HeldBack::ScriptManaged) => 2,
            Some(HeldBack::Empty) => 3,
        };
        let managed = row.flags & SCRIPT_MANAGED != 0;
        row.group = match (managed, row.group) {
            (false, _) => 0,
            (true, 0) => row.name,
            (true, group) => group,
        };
    }
}

/// What the maps count to, mode by mode.
fn counted(maps: &[MapRow]) -> Report {
    let at_rest = Enabled::default();
    let mut report = Report {
        maps: maps.len() as u64,
        ..Report::default()
    };
    for map in maps {
        let in_mode = |mode: Mode| u64::from(map.modes & u32::from(mode.bit()) != 0);
        let base = in_mode(Mode::Base);
        report.registered += base;
        report.at_rest += u64::from(at_rest.allows(map));
        report.script_managed += base * u64::from(map.held == 2);
        report.empty += base * u64::from(map.held == 3);
        report.story += in_mode(Mode::Story);
        report.minigames += in_mode(Mode::Minigames);
    }
    report
}

/// The one dictionary body that holds a drawable under every name the archetypes of a dictionary
/// word bear, for each such word.
fn dictionaries(archetypes: &[ArchetypeRow], keys: &HashMap<u32, Vec<BodyRow>>) -> Vec<BodyRow> {
    let mut names: HashMap<u64, Vec<u32>> = HashMap::new();
    for a in archetypes.iter().filter(|a| a.dictionary != 0) {
        names.entry(a.dictionary).or_default().push(a.name);
    }
    let mut found = Vec::new();
    for (word, names) in names {
        let mut holders: Option<HashSet<(u32, u64)>> = None;
        for name in &names {
            let bodies: HashSet<(u32, u64)> = (keys.get(name).into_iter().flatten())
                .map(|b| (b.archive, b.offset))
                .collect();
            holders = Some(match holders {
                None => bodies,
                Some(held) => held.intersection(&bodies).copied().collect(),
            });
        }
        let holders = holders.unwrap_or_default();
        if let (1, Some((archive, offset))) = (holders.len(), holders.iter().next()) {
            found.push(BodyRow {
                word,
                offset: *offset,
                archive: *archive,
                pad: 0,
            });
        }
    }
    found
}

/// Indexes made up for the tests of this folder.
#[cfg(any(test, feature = "made-up"))]
pub mod tests_support {
    use super::*;

    /// An index of `archives` with an archetype for each word, the distance it draws to and the
    /// offset of its model where it has one, and the maps `maps` with the parent names `parents`.
    pub fn index_with(
        archives: Vec<PathBuf>,
        archetypes: &[(u64, f32, Option<u64>)],
        maps: Vec<MapRow>,
        parents: Vec<u32>,
    ) -> WorldIndex {
        let mut index = WorldIndex {
            archives,
            maps,
            parents,
            ..WorldIndex::default()
        };
        for (word, lod_dist, model) in archetypes {
            index.archetypes.push(ArchetypeRow {
                word: *word,
                model: model.map_or(0, |_| word + 1),
                dictionary: 0,
                name: *word as u32,
                lod_dist: *lod_dist,
                centre: [0.0; 3],
                radius: 1.0,
                flags: 0,
                hours: ALL_HOURS,
                load_flags: 0,
                pad: 0,
            });
            index.models.extend(model.map(|offset| BodyRow {
                word: word + 1,
                offset,
                archive: 0,
                pad: 0,
            }));
        }
        index.finish()
    }

    pub fn index_of(archives: Vec<PathBuf>, archetypes: &[(u64, f32, Option<u64>)]) -> WorldIndex {
        index_with(archives, archetypes, Vec::new(), Vec::new())
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
