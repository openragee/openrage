//! World-state groups: which maps stream depends on the clock, the weather
//! and what a script asked for, and a manifest says how.
//!
//! A map whose `flags` sets bit 0 streams only when a script requests it by
//! name or when a group gating it on the clock or the weather admits the
//! current state. Each of a manifest's `MapDataGroups` is named for the map it
//! switches and lists in `Bounds` the static collision files that switch with
//! it.
//!
//! ⚠ `Bounds` is collision, not member maps. Over both retail installs
//! (`what_every_world_state_group_holds`) every group's name is a held map
//! that sets bit 0 — 576 of 576 Legacy, 570 of 570 Enhanced — while `Bounds`
//! names a map 2 times in 2,549 and a held `.ybn` 2,549 of 2,549.

use std::collections::{BTreeMap, BTreeSet};

use or_io::meta::{MetaInstance, Value};

use crate::field;
use crate::keys;
use crate::manifest::Manifest;
use crate::map_data::MapData;

/// Bit positions in a group's `Flags`.
pub mod flags {
    /// The group is switched by the clock, per its `HoursOnOff`.
    pub const TIMED: u32 = 1 << 0;
    /// The group is switched by the weather, per its `WeatherTypes`.
    pub const WEATHER: u32 = 1 << 1;
}

/// The bit in `CMapData::flags` that holds a map back until something asks.
pub const NOT_BY_DEFAULT: u32 = 1 << 0;

/// The weather types a group can name, spelt as the game hashes them.
///
/// ⚠ **For naming a number, never for deciding.** A group's weather list is
/// the hashes it stores, and a type missing here still gates correctly; it
/// only prints as a number. One of the 14 types the retail groups name is
/// not in this list, and it stays a number rather than a guess.
pub const WEATHER_NAMES: &[&str] = &[
    "BLIZZARD",
    "CLEAR",
    "CLEARING",
    "CLOUDS",
    "EXTRASUNNY",
    "FOGGY",
    "HALLOWEEN",
    "NEUTRAL",
    "OVERCAST",
    "RAIN",
    "RAIN_HALLOWEEN",
    "SMOG",
    "SNOW",
    "SNOWLIGHT",
    "SNOW_HALLOWEEN",
    "THUNDER",
    "XMAS",
];

/// The weather name a hash stands for, when it is one of [`WEATHER_NAMES`].
pub fn weather_name(hash: u32) -> Option<&'static str> {
    WEATHER_NAMES
        .iter()
        .copied()
        .find(|name| or_io::string_hash(name.as_bytes()) == hash)
}

/// What switches a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// Only a script's request does.
    Scripted,
    /// The clock: on during each hour whose bit is set.
    Timed { hours: u32 },
    /// The weather: on while the current type is one of these.
    Weather { types: Vec<u32> },
    /// Both, and both have to agree.
    TimedWeather { hours: u32, types: Vec<u32> },
}

impl Gate {
    fn of(flags: u32, hours: u32, types: &[u32]) -> Self {
        match (flags & flags::TIMED != 0, flags & flags::WEATHER != 0) {
            (false, false) => Gate::Scripted,
            (true, false) => Gate::Timed { hours },
            (false, true) => Gate::Weather {
                types: types.to_vec(),
            },
            (true, true) => Gate::TimedWeather {
                hours,
                types: types.to_vec(),
            },
        }
    }

    /// Whether the streamer holds this group under `state`, ignoring requests.
    fn admits(&self, hour: u8, weather: u32) -> bool {
        let at = |hours: u32| hour < 24 && hours & (1 << hour) != 0;
        match self {
            Gate::Scripted => false,
            Gate::Timed { hours } => at(*hours),
            Gate::Weather { types } => types.contains(&weather),
            Gate::TimedWeather { hours, types } => at(*hours) && types.contains(&weather),
        }
    }
}

/// A flag word that disagrees with the fields it gates on.
///
/// ⚠ **None ships**: 0 of 576 Legacy and 0 of 570 Enhanced groups, so each
/// is a defect an edit introduced rather than a convention to tolerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateProblem {
    /// `TIMED` without an hour, which is a group that never switches on.
    TimedWithNoHours,
    /// Hours without `TIMED`, which the streamer never reads.
    HoursNotTimed,
    /// `WEATHER` with no weather type.
    WeatherWithNoTypes,
    /// Weather types without `WEATHER`.
    TypesNotWeather,
}

impl std::fmt::Display for GateProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            GateProblem::TimedWithNoHours => "timed, but on during no hour",
            GateProblem::HoursNotTimed => "states hours and is not timed",
            GateProblem::WeatherWithNoTypes => "weather-gated, but names no weather",
            GateProblem::TypesNotWeather => "names weather and is not weather-gated",
        })
    }
}

/// One `CMapDataGroup`, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct StateGroup<'a> {
    inst: &'a MetaInstance,
}

impl<'a> StateGroup<'a> {
    pub fn new(inst: &'a MetaInstance) -> Option<Self> {
        field::is_type(inst, keys::types::C_MAP_DATA_GROUP).then_some(Self { inst })
    }

    pub fn instance(&self) -> &'a MetaInstance {
        self.inst
    }

    /// The group's name, which is the name of the map it switches.
    pub fn name(&self) -> u32 {
        field::hash_at(self.inst, keys::GROUP_NAME).unwrap_or(0)
    }

    /// The static bounds files that switch with the group.
    pub fn bounds(&self) -> Vec<u32> {
        hashes(self.inst, keys::GROUP_BOUNDS)
    }

    pub fn flags(&self) -> u32 {
        field::u32_at(self.inst, keys::GROUP_FLAGS).unwrap_or(0)
    }

    /// One bit per hour of the day, bit 0 for midnight.
    pub fn hours_on_off(&self) -> u32 {
        field::u32_at(self.inst, keys::HOURS_ON_OFF).unwrap_or(0)
    }

    pub fn weather_types(&self) -> Vec<u32> {
        hashes(self.inst, keys::WEATHER_TYPES)
    }

    pub fn gate(&self) -> Gate {
        Gate::of(self.flags(), self.hours_on_off(), &self.weather_types())
    }

    /// Where the flag word and the fields it gates on disagree.
    pub fn problems(&self) -> Vec<GateProblem> {
        problems_of(
            self.flags(),
            self.hours_on_off(),
            !self.weather_types().is_empty(),
        )
    }
}

fn problems_of(flags: u32, hours: u32, has_types: bool) -> Vec<GateProblem> {
    let mut out = Vec::new();
    let timed = flags & flags::TIMED != 0;
    let weather = flags & flags::WEATHER != 0;
    if timed && hours == 0 {
        out.push(GateProblem::TimedWithNoHours);
    }
    if !timed && hours != 0 {
        out.push(GateProblem::HoursNotTimed);
    }
    if weather && !has_types {
        out.push(GateProblem::WeatherWithNoTypes);
    }
    if !weather && has_types {
        out.push(GateProblem::TypesNotWeather);
    }
    out
}

fn hashes(inst: &MetaInstance, key: u32) -> Vec<u32> {
    field::array_at(inst, key)
        .iter()
        .filter_map(|v| match v {
            Value::Hash(h) => Some(h.hash),
            Value::UInt(n) => Some(*n as u32),
            _ => None,
        })
        .collect()
}

impl<'a> Manifest<'a> {
    /// The world-state groups this manifest declares.
    pub fn state_groups(&self) -> impl Iterator<Item = StateGroup<'a>> + 'a {
        field::structs_at(self.instance(), keys::MAP_DATA_GROUPS).filter_map(StateGroup::new)
    }
}

/// Where the world stands: the clock, the weather and what scripts asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldState {
    /// 0 to 23.
    pub hour: u8,
    /// The current weather type's hash.
    pub weather: u32,
    /// Maps a script has requested by name.
    pub requested: BTreeSet<u32>,
}

impl WorldState {
    pub fn at(hour: u8, weather: &str) -> Self {
        Self {
            hour,
            weather: or_io::string_hash(weather.as_bytes()),
            requested: BTreeSet::new(),
        }
    }

    pub fn requesting(mut self, map: u32) -> Self {
        self.requested.insert(map);
        self
    }
}

/// What starts and stops streaming between two states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    pub load: BTreeSet<u32>,
    pub unload: BTreeSet<u32>,
    /// Static bounds files a group switches on with its map.
    pub bounds_load: BTreeSet<u32>,
    pub bounds_unload: BTreeSet<u32>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.load.is_empty()
            && self.unload.is_empty()
            && self.bounds_load.is_empty()
            && self.bounds_unload.is_empty()
    }
}

/// What replacing one manifest's groups changed in the index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reload {
    pub added: BTreeSet<u32>,
    pub removed: BTreeSet<u32>,
    /// Groups present before and after whose gate or bounds moved.
    pub changed: BTreeSet<u32>,
    /// What that does to the world under the state it was asked about.
    pub delta: Delta,
}

/// Something wrong between the groups and the maps they switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateProblem {
    /// A group named for a map the index holds no copy of.
    GroupMapAbsent { group: u32 },
    /// A group whose map streams by default, so switching it does nothing.
    GroupMapStreamsByDefault { group: u32 },
    /// A flag word disagreeing with its fields.
    Gate { group: u32, problem: GateProblem },
}

impl std::fmt::Display for StateProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateProblem::GroupMapAbsent { group } => {
                write!(
                    f,
                    "group {group:#010X} switches a map the install does not hold"
                )
            }
            StateProblem::GroupMapStreamsByDefault { group } => write!(
                f,
                "group {group:#010X} switches a map that does not set flags bit 0, \
                 so the map streams whatever the group says"
            ),
            StateProblem::Gate { group, problem } => write!(f, "group {group:#010X}: {problem}"),
        }
    }
}

/// What the index keeps of one group.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Summary {
    gate: Gate,
    bounds: Vec<u32>,
    problems: Vec<GateProblem>,
    origin: String,
}

/// Every group an install declares, and which maps stream by default.
///
/// ⚠ **A summary, not a borrow**, for the reason [`crate::group::MapGroup`]
/// is one: it spans every manifest and every map, and a caller drops each
/// tree once it is folded in.
#[derive(Debug, Clone, Default)]
pub struct StateGroups {
    by_name: BTreeMap<u32, Summary>,
    /// Every map name folded in, and whether it sets [`NOT_BY_DEFAULT`].
    maps: BTreeMap<u32, bool>,
    /// A later declaration of a name replaced an earlier one.
    redeclared: u64,
}

impl StateGroups {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold in every group a manifest declares.
    ///
    /// ⚠ **A later declaration of a name wins**, which is mount order: a DLC
    /// manifest restating a base group replaces it rather than merging.
    /// Over a retail Legacy install 394 of the declarations restate a group
    /// an earlier manifest declared.
    pub fn add_from(&mut self, manifest: Manifest<'_>, origin: &str) {
        for group in manifest.state_groups() {
            self.add_group(
                group.name(),
                (group.flags(), group.hours_on_off()),
                group.weather_types(),
                group.bounds(),
                origin,
            );
        }
    }

    /// Fold in one group by what it states, its `Flags` and `HoursOnOff` as `(flags, hours)`, for a caller that kept those and dropped the manifest.
    pub fn add_group(
        &mut self,
        name: u32,
        (flags, hours): (u32, u32),
        weather_types: Vec<u32>,
        bounds: Vec<u32>,
        origin: &str,
    ) {
        if name == 0 {
            return;
        }
        let summary = Summary {
            gate: Gate::of(flags, hours, &weather_types),
            bounds,
            problems: problems_of(flags, hours, !weather_types.is_empty()),
            origin: origin.to_string(),
        };
        if self.by_name.insert(name, summary).is_some() {
            self.redeclared += 1;
        }
    }

    /// Record whether a map streams by default.
    ///
    /// ⚠ A map is several files under one name, so the bit is folded with
    /// OR: one part holding the map back holds all of it back.
    pub fn add_map(&mut self, map: &MapData<'_>) {
        self.add_map_flags(map.name(), map.flags());
    }

    /// [`Self::add_map`] for a map known by its name and its `flags` alone.
    pub fn add_map_flags(&mut self, name: u32, flags: u32) {
        if name == 0 {
            return;
        }
        let held = flags & NOT_BY_DEFAULT != 0;
        *self.maps.entry(name).or_default() |= held;
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn redeclared(&self) -> u64 {
        self.redeclared
    }

    pub fn names(&self) -> impl Iterator<Item = u32> + '_ {
        self.by_name.keys().copied()
    }

    pub fn contains(&self, group: u32) -> bool {
        self.by_name.contains_key(&group)
    }

    pub fn gate(&self, group: u32) -> Option<&Gate> {
        self.by_name.get(&group).map(|s| &s.gate)
    }

    pub fn bounds(&self, group: u32) -> &[u32] {
        self.by_name.get(&group).map_or(&[], |s| &s.bounds)
    }

    /// Maps folded in with [`Self::add_map`] that something has to ask for.
    pub fn held_back(&self) -> impl Iterator<Item = u32> + '_ {
        self.maps.iter().filter(|(_, held)| **held).map(|(m, _)| *m)
    }

    /// Whether a map waits to be asked for.
    ///
    /// ⚠ A group's own map counts even when no map was folded in, because
    /// the group is the claim: a caller holding only manifests still gets the
    /// answer every shipped group agrees with.
    pub fn is_held_back(&self, map: u32) -> bool {
        self.maps.get(&map).copied().unwrap_or(false) || self.by_name.contains_key(&map)
    }

    /// Whether a group's gate admits `state`, or a script requested it.
    pub fn is_on(&self, group: u32, state: &WorldState) -> bool {
        let Some(summary) = self.by_name.get(&group) else {
            return false;
        };
        state.requested.contains(&group) || summary.gate.admits(state.hour, state.weather)
    }

    /// Whether a map streams under `state`.
    pub fn is_active(&self, map: u32, state: &WorldState) -> bool {
        !self.is_held_back(map) || state.requested.contains(&map) || self.is_on(map, state)
    }

    /// What starts and stops streaming going from `before` to `after`.
    pub fn transition(&self, before: &WorldState, after: &WorldState) -> Delta {
        let candidates: BTreeSet<u32> = before
            .requested
            .symmetric_difference(&after.requested)
            .copied()
            .chain(
                self.by_name
                    .iter()
                    .filter(|(_, s)| {
                        s.gate.admits(before.hour, before.weather)
                            != s.gate.admits(after.hour, after.weather)
                    })
                    .map(|(n, _)| *n),
            )
            .collect();
        let mut delta = Delta::default();
        for map in candidates {
            self.record(&mut delta, map, before, after);
        }
        delta
    }

    fn record(&self, delta: &mut Delta, map: u32, before: &WorldState, after: &WorldState) {
        let (was, now) = (self.is_active(map, before), self.is_active(map, after));
        let bounds = self.bounds(map);
        match (was, now) {
            (false, true) => {
                delta.load.insert(map);
                delta.bounds_load.extend(bounds);
            }
            (true, false) => {
                delta.unload.insert(map);
                delta.bounds_unload.extend(bounds);
            }
            _ => {}
        }
    }

    /// Request or release a map, and say what that streams.
    pub fn toggle(&self, state: &mut WorldState, map: u32, on: bool) -> Delta {
        let before = state.clone();
        if on {
            state.requested.insert(map);
        } else {
            state.requested.remove(&map);
        }
        self.transition(&before, state)
    }

    /// Replace every group `origin` declared with what `manifest` declares now.
    ///
    /// Answers which groups moved and what that streams under `state` without
    /// touching the rest of the index or reading any world file. `None`
    /// withdraws `origin` entirely, as for a deleted manifest.
    pub fn reload(
        &mut self,
        origin: &str,
        manifest: Option<Manifest<'_>>,
        state: &WorldState,
    ) -> Reload {
        let before = self.clone();
        let old: BTreeMap<u32, Summary> = self
            .by_name
            .iter()
            .filter(|(_, s)| s.origin == origin)
            .map(|(n, s)| (*n, s.clone()))
            .collect();
        for name in old.keys() {
            self.by_name.remove(name);
        }
        if let Some(manifest) = manifest {
            let redeclared = self.redeclared;
            self.add_from(manifest, origin);
            self.redeclared = redeclared;
        }

        let mut reload = Reload::default();
        for (name, summary) in &self.by_name {
            if summary.origin != origin {
                continue;
            }
            match old.get(name) {
                None => {
                    reload.added.insert(*name);
                }
                Some(prev) if prev != summary => {
                    reload.changed.insert(*name);
                }
                Some(_) => {}
            }
        }
        for name in old.keys() {
            if !self.by_name.contains_key(name) {
                reload.removed.insert(*name);
            }
        }

        let touched: BTreeSet<u32> = reload
            .added
            .iter()
            .chain(&reload.removed)
            .chain(&reload.changed)
            .copied()
            .collect();
        for map in touched {
            let (was, now) = (before.is_active(map, state), self.is_active(map, state));
            if was != now {
                if now {
                    reload.delta.load.insert(map);
                } else {
                    reload.delta.unload.insert(map);
                }
            }
            let old_bounds: BTreeSet<u32> = if was {
                before.bounds(map).iter().copied().collect()
            } else {
                BTreeSet::new()
            };
            let new_bounds: BTreeSet<u32> = if now {
                self.bounds(map).iter().copied().collect()
            } else {
                BTreeSet::new()
            };
            reload
                .delta
                .bounds_load
                .extend(new_bounds.difference(&old_bounds));
            reload
                .delta
                .bounds_unload
                .extend(old_bounds.difference(&new_bounds));
        }
        reload
    }

    /// What is wrong between the groups and the maps folded in.
    ///
    /// ⚠ The map-side checks run only once maps were folded in: an index of
    /// manifests alone cannot tell an absent map from one it was not given.
    pub fn check(&self) -> Vec<StateProblem> {
        let mut out = Vec::new();
        for (name, summary) in &self.by_name {
            for problem in &summary.problems {
                out.push(StateProblem::Gate {
                    group: *name,
                    problem: *problem,
                });
            }
            if self.maps.is_empty() {
                continue;
            }
            match self.maps.get(name) {
                None => out.push(StateProblem::GroupMapAbsent { group: *name }),
                Some(false) => out.push(StateProblem::GroupMapStreamsByDefault { group: *name }),
                Some(true) => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::value::HashedString;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    fn group(name: &str, bounds: &[&str], flags: u32, hours: u32, weather: &[&str]) -> Value {
        let list = |items: &[&str]| {
            Value::Array(
                items
                    .iter()
                    .map(|s| Value::Hash(HashedString::new(*s)))
                    .collect(),
            )
        };
        let mut inst = MetaInstance::new("CMapDataGroup");
        inst.set("Name", Value::Hash(HashedString::new(name)));
        inst.set("Bounds", list(bounds));
        inst.set("Flags", Value::UInt(flags as u64));
        inst.set("WeatherTypes", list(weather));
        inst.set("HoursOnOff", Value::UInt(hours as u64));
        Value::Struct(Box::new(inst))
    }

    fn manifest(groups: Vec<Value>) -> MetaInstance {
        let mut root = MetaInstance::new("CPackFileMetaData");
        root.set("MapDataGroups", Value::Array(groups));
        root
    }

    fn map(name: &str, flags: u32) -> MetaInstance {
        let mut root = MetaInstance::new("CMapData");
        root.set("name", Value::Hash(HashedString::new(name)));
        root.set("flags", Value::UInt(flags as u64));
        root
    }

    const DAYTIME: u32 = 0x000F_FFC0;

    fn sample() -> MetaInstance {
        manifest(vec![
            group("carrier", &["carrier_col"], 0, 0, &[]),
            group("day_market", &[], flags::TIMED, DAYTIME, &[]),
            group(
                "puddles",
                &["puddles_col"],
                flags::TIMED | flags::WEATHER,
                DAYTIME,
                &["RAIN", "THUNDER"],
            ),
        ])
    }

    fn index_of(tree: &MetaInstance) -> StateGroups {
        let mut groups = StateGroups::new();
        groups.add_from(Manifest::new(tree).unwrap(), "base");
        groups
    }

    #[test]
    fn a_manifest_declares_its_groups_and_their_gates() {
        let tree = sample();
        let m = Manifest::new(&tree).unwrap();
        let groups: Vec<StateGroup<'_>> = m.state_groups().collect();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].gate(), Gate::Scripted);
        assert_eq!(groups[0].bounds(), vec![hash("carrier_col")]);
        assert_eq!(groups[1].gate(), Gate::Timed { hours: DAYTIME });
        assert_eq!(
            groups[2].gate(),
            Gate::TimedWeather {
                hours: DAYTIME,
                types: vec![hash("RAIN"), hash("THUNDER")]
            }
        );
        assert!(groups.iter().all(|g| g.problems().is_empty()));
    }

    #[test]
    fn groups_kept_as_what_they_state_gate_as_the_manifest_that_stated_them() {
        let tree = sample();
        let read = index_of(&tree);
        let mut kept = StateGroups::new();
        for group in Manifest::new(&tree).unwrap().state_groups() {
            kept.add_group(
                group.name(),
                (group.flags(), group.hours_on_off()),
                group.weather_types(),
                group.bounds(),
                "base",
            );
        }
        // A group named nothing is no group, as a manifest's is none.
        kept.add_group(0, (flags::TIMED, DAYTIME), vec![], vec![], "base");
        assert_eq!(kept.len(), read.len());
        // ⚠ Held to what each group says of itself: a manifest is folded in through the same
        // `add_group`, so the two indexes agree whatever it does.
        for group in Manifest::new(&tree).unwrap().state_groups() {
            let name = group.name();
            assert_eq!(kept.gate(name), Some(&group.gate()), "{name:#010X}");
            assert_eq!(kept.bounds(name), group.bounds(), "{name:#010X}");
        }
        let puddles = Gate::TimedWeather {
            hours: DAYTIME,
            types: vec![hash("RAIN"), hash("THUNDER")],
        };
        assert_eq!(kept.gate(hash("puddles")), Some(&puddles));
        assert_eq!(kept.bounds(hash("puddles")), [hash("puddles_col")]);
        assert!(kept.check().is_empty());
        // A flag word that disagrees with its fields is said of a group kept so, as of one read.
        let mut odd = StateGroups::new();
        odd.add_group(hash("never"), (flags::TIMED, 0), vec![], vec![], "base");
        assert_eq!(
            odd.check(),
            [StateProblem::Gate {
                group: hash("never"),
                problem: GateProblem::TimedWithNoHours
            }]
        );
        // And a map known by its flags alone is held back as the map read whole is.
        let held = map("day_market", NOT_BY_DEFAULT);
        let mut by_tree = index_of(&tree);
        by_tree.add_map(&MapData::new(&held).unwrap());
        kept.add_map_flags(hash("day_market"), NOT_BY_DEFAULT);
        kept.add_map_flags(hash("plain"), 0);
        // One no group names is held back by its own bit alone.
        kept.add_map_flags(hash("loose_ipl"), NOT_BY_DEFAULT);
        assert!(kept.is_held_back(hash("loose_ipl")) && !kept.is_held_back(hash("plain")));
        let (noon, night) = (WorldState::at(12, "CLEAR"), WorldState::at(2, "CLEAR"));
        for state in [&noon, &night] {
            assert_eq!(
                kept.is_active(hash("day_market"), state),
                by_tree.is_active(hash("day_market"), state)
            );
        }
        assert!(kept.is_active(hash("day_market"), &noon));
        assert!(!kept.is_active(hash("day_market"), &night));
        assert!(kept.is_active(hash("plain"), &night));
        assert!(!kept.is_active(hash("loose_ipl"), &noon));
    }

    #[test]
    fn what_streams_follows_the_bit_the_groups_and_the_requests() {
        let tree = sample();
        let mut groups = index_of(&tree);
        let loose = map("loose_ipl", NOT_BY_DEFAULT);
        let plain = map("plain", 0);
        groups.add_map(&MapData::new(&loose).unwrap());
        groups.add_map(&MapData::new(&plain).unwrap());

        let night = WorldState::at(2, "CLEAR");
        assert!(!groups.is_active(hash("carrier"), &night));
        assert!(!groups.is_active(hash("loose_ipl"), &night));
        assert!(groups.is_active(hash("plain"), &night));
        assert!(groups.is_active(hash("never_folded_in"), &night));

        let asked = night
            .clone()
            .requesting(hash("carrier"))
            .requesting(hash("loose_ipl"));
        assert!(groups.is_active(hash("carrier"), &asked));
        assert!(groups.is_active(hash("loose_ipl"), &asked));
    }

    #[test]
    fn the_clock_and_the_weather_both_have_to_agree() {
        let tree = sample();
        let groups = index_of(&tree);
        let puddles = hash("puddles");
        assert!(!groups.is_on(puddles, &WorldState::at(12, "CLEAR")));
        assert!(!groups.is_on(puddles, &WorldState::at(2, "RAIN")));
        assert!(groups.is_on(puddles, &WorldState::at(12, "RAIN")));
        assert!(groups.is_on(hash("day_market"), &WorldState::at(6, "CLEAR")));
        assert!(!groups.is_on(hash("day_market"), &WorldState::at(5, "CLEAR")));
        assert!(!groups.is_on(hash("day_market"), &WorldState::at(20, "CLEAR")));
    }

    #[test]
    fn requesting_a_group_streams_its_map_and_its_bounds() {
        let tree = sample();
        let groups = index_of(&tree);
        let mut state = WorldState::at(2, "CLEAR");
        let on = groups.toggle(&mut state, hash("carrier"), true);
        assert_eq!(on.load, BTreeSet::from([hash("carrier")]));
        assert_eq!(on.bounds_load, BTreeSet::from([hash("carrier_col")]));
        assert!(on.unload.is_empty());
        let off = groups.toggle(&mut state, hash("carrier"), false);
        assert_eq!(off.unload, on.load);
        assert_eq!(off.bounds_unload, on.bounds_load);
        assert!(off.load.is_empty());
    }

    #[test]
    fn requesting_what_is_already_streaming_is_no_change() {
        let tree = sample();
        let groups = index_of(&tree);
        let mut state = WorldState::at(12, "CLEAR");
        assert!(groups
            .toggle(&mut state, hash("day_market"), true)
            .is_empty());
        assert!(groups
            .toggle(&mut state, hash("plain_map"), true)
            .is_empty());
    }

    #[test]
    fn moving_the_clock_streams_the_timed_groups() {
        let tree = sample();
        let groups = index_of(&tree);
        let delta = groups.transition(&WorldState::at(5, "RAIN"), &WorldState::at(6, "RAIN"));
        assert_eq!(
            delta.load,
            BTreeSet::from([hash("day_market"), hash("puddles")])
        );
        assert_eq!(delta.bounds_load, BTreeSet::from([hash("puddles_col")]));
    }

    #[test]
    fn reloading_a_manifest_reports_what_moved() {
        let base = sample();
        let mut groups = index_of(&base);
        let dlc = manifest(vec![group("dlc_only", &[], 0, 0, &[])]);
        groups.add_from(Manifest::new(&dlc).unwrap(), "dlc");

        let state = WorldState::at(12, "RAIN").requesting(hash("carrier"));
        let edited = manifest(vec![
            group("carrier", &["carrier_col", "carrier_col2"], 0, 0, &[]),
            group("day_market", &[], flags::TIMED, DAYTIME, &[]),
        ]);
        let reload = groups.reload("base", Manifest::new(&edited), &state);
        assert_eq!(reload.changed, BTreeSet::from([hash("carrier")]));
        assert_eq!(reload.removed, BTreeSet::from([hash("puddles")]));
        assert!(reload.added.is_empty());
        // `puddles` was streaming (noon, raining); with no group and no map
        // folded in, nothing now holds it back, so it stays and only its
        // bounds go. The requested carrier gains a bounds file.
        assert!(reload.delta.load.is_empty() && reload.delta.unload.is_empty());
        assert_eq!(
            reload.delta.bounds_load,
            BTreeSet::from([hash("carrier_col2")])
        );
        assert_eq!(
            reload.delta.bounds_unload,
            BTreeSet::from([hash("puddles_col")])
        );
        assert!(groups.contains(hash("dlc_only")));

        let gone = groups.reload("base", None, &state);
        assert_eq!(
            gone.removed,
            BTreeSet::from([hash("carrier"), hash("day_market")])
        );
        assert!(groups.contains(hash("dlc_only")));
    }

    #[test]
    fn a_reload_delta_is_the_difference_of_two_full_answers() {
        let base = sample();
        let mut groups = index_of(&base);
        for name in ["carrier", "day_market", "puddles", "loose"] {
            groups.add_map(&MapData::new(&map(name, NOT_BY_DEFAULT)).unwrap());
        }
        let state = WorldState::at(12, "CLEAR");
        let before = groups.clone();
        let edited = manifest(vec![group("loose", &[], flags::TIMED, DAYTIME, &[])]);
        let reload = groups.reload("base", Manifest::new(&edited), &state);
        for m in ["carrier", "day_market", "puddles", "loose"] {
            let m = hash(m);
            let (was, now) = (before.is_active(m, &state), groups.is_active(m, &state));
            assert_eq!(reload.delta.load.contains(&m), !was && now, "{m:#X}");
            assert_eq!(reload.delta.unload.contains(&m), was && !now, "{m:#X}");
        }
        assert_eq!(reload.delta.load, BTreeSet::from([hash("loose")]));
        assert_eq!(reload.delta.unload, BTreeSet::from([hash("day_market")]));
    }

    #[test]
    fn a_flag_word_that_disagrees_with_its_fields_is_reported() {
        assert_eq!(
            problems_of(flags::TIMED, 0, false),
            vec![GateProblem::TimedWithNoHours]
        );
        assert_eq!(problems_of(0, 1, false), vec![GateProblem::HoursNotTimed]);
        assert_eq!(
            problems_of(flags::WEATHER, 0, false),
            vec![GateProblem::WeatherWithNoTypes]
        );
        assert_eq!(problems_of(0, 0, true), vec![GateProblem::TypesNotWeather]);
        assert!(problems_of(flags::TIMED | flags::WEATHER, 1, true).is_empty());
    }

    #[test]
    fn a_group_whose_map_streams_anyway_is_reported() {
        let tree = sample();
        let mut groups = index_of(&tree);
        assert!(
            groups.check().is_empty(),
            "no maps folded in, nothing to say"
        );
        groups.add_map(&MapData::new(&map("carrier", 0)).unwrap());
        groups.add_map(&MapData::new(&map("day_market", NOT_BY_DEFAULT)).unwrap());
        let problems = groups.check();
        assert!(problems.contains(&StateProblem::GroupMapStreamsByDefault {
            group: hash("carrier")
        }));
        assert!(problems.contains(&StateProblem::GroupMapAbsent {
            group: hash("puddles")
        }));
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn a_weather_hash_names_itself() {
        assert_eq!(weather_name(hash("EXTRASUNNY")), Some("EXTRASUNNY"));
        assert_eq!(weather_name(hash("not_a_weather")), None);
    }
}
