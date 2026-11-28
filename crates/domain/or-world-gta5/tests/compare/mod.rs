//! One view asked of the core's driver over the adapter and of `WorldForest`'s own loop over the
//! same mount, each from nothing, and what each streamed and selected put in the same terms: a
//! map by the hash of its name, an interior by the entity that places it.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::Vec3;
use or_world::contract::WorldInstall;
use or_world::driver::Driver;
use or_world::model::{is_interior_key, AssetKey, Index, MapId, Moment, Switched};
use or_world::streaming::{NodeState, View};
use or_world_gta5::world_forest::{Model, NoModel};
use or_world_gta5::WorldState;
use or_world_gta5::{Gta5World, Linked, BOX, BOXED_UNDECLARED, BOXED_UNNAMED};
use or_world_gta5::{WorldForest, WorldIndex};

/// Bytes of maps either side may hold let go of: more than one view's.
const ROOMY: u64 = 1 << 30;

/// One instance, in the terms both sides share.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub archetype: u32,
    pub level: u8,
    pub fade: f32,
    pub yields: bool,
    /// 0 where files draw it, else why it stands as a box.
    pub boxed: u32,
    pub rows: [[f32; 4]; 3],
}

/// Where an instance stands: its map by hash, the entity that places its interior, and its place.
pub type Place = (u32, Option<u32>, u32);

/// What one side made of a view.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    /// The maps in the forest by hash, each with the hash of the child it is the parent of where that is why.
    pub wanted: BTreeMap<u32, Option<u32>>,
    pub unread: Vec<u32>,
    /// `(map, entity)` of each interior brought in, then of each that gave none.
    pub interiors: Vec<(u32, u32)>,
    pub interiors_unread: Vec<(u32, u32)>,
    /// One selection after another.
    pub selections: Vec<BTreeMap<Place, Row>>,
    /// The archetypes each selection waits for, in its order.
    pub waiting: Vec<Vec<u32>>,
}

/// Whether the model of `archetype` is taken as loading: one archetype in three.
fn loading(archetype: u32) -> bool {
    archetype.is_multiple_of(3)
}

/// What `WorldForest` makes of `view` over `index`, selected `times` times.
pub fn by_the_forest(
    world: &Gta5World,
    index: &WorldIndex,
    view: &View,
    state: &WorldState,
    hold: bool,
    times: usize,
) -> Seen {
    let mut forest = WorldForest::new(index, world.source(), ROOMY);
    let streamed = forest.stream(view, state);
    let mut seen = Seen::default();
    for wanted in &streamed.wanted {
        let why = match wanted.why {
            or_world_gta5::world_forest::Why::Own => None,
            or_world_gta5::world_forest::Why::ParentOf(child) => Some(child),
        };
        seen.wanted.insert(wanted.map.hash, why);
    }
    seen.unread = streamed.unread.iter().map(|(map, _)| map.hash).collect();
    seen.interiors = (streamed.interiors.iter())
        .map(|at| (at.map, at.entity))
        .collect();
    seen.interiors_unread = (streamed.interiors_unread.iter())
        .map(|(at, _)| (at.map, at.entity))
        .collect();
    for _ in 0..times {
        let ready = |archetype: u32| match hold && loading(archetype) {
            true => NodeState::Loading,
            false => NodeState::Ready,
        };
        let selected = forest.select(view, &ready);
        let mut rows = BTreeMap::new();
        for instance in &selected.instances {
            let m = instance.to_world.matrix3;
            let t = instance.to_world.translation;
            let row = Row {
                archetype: instance.archetype,
                level: instance.level as u8,
                fade: instance.fade,
                yields: instance.yields,
                boxed: match instance.model {
                    Model::Files(_) => 0,
                    Model::Box(NoModel::Unnamed) => BOXED_UNNAMED,
                    Model::Box(NoModel::Undeclared) => BOXED_UNDECLARED,
                },
                rows: [
                    [m.x_axis.x, m.y_axis.x, m.z_axis.x, t.x],
                    [m.x_axis.y, m.y_axis.y, m.z_axis.y, t.y],
                    [m.x_axis.z, m.y_axis.z, m.z_axis.z, t.z],
                ],
            };
            let place = (instance.map, instance.interior, instance.entity);
            assert!(rows.insert(place, row).is_none(), "{place:?} drawn twice");
        }
        seen.selections.push(rows);
        seen.waiting.push(selected.waiting);
    }
    seen
}

/// What the core's driver makes of `view` over the adapter, selected `times` times.
pub fn by_the_driver(
    world: &Arc<Gta5World>,
    index: &Arc<Index>,
    view: &View,
    (hour, weather): (u8, u32),
    hold: bool,
    times: usize,
) -> Seen {
    let linked = index.title::<Linked>().expect("a GTA V link");
    let hash_of = |map: MapId| linked.map(map).expect("a row of the index").hash;
    let install: Arc<dyn WorldInstall> = world.clone();
    let mut driver = Driver::new(install, Arc::clone(index));
    let at = Moment {
        hour: f32::from(hour),
        weather: u64::from(weather),
    };
    let streamed = driver.stream(view, &at, &Switched::default(), None, usize::MAX);
    let mut seen = Seen::default();
    for (map, why) in &streamed.wanted {
        let why = match why {
            or_world::driver::Why::Own => None,
            or_world::driver::Why::ParentOf(child) => Some(hash_of(*child)),
        };
        seen.wanted.insert(hash_of(*map), why);
    }
    assert_eq!(streamed.waiting, [], "every wanted map was read");
    seen.unread = (streamed.unread.iter())
        .map(|(map, _)| hash_of(*map))
        .collect();
    let mut placing: BTreeMap<u64, (u32, u32)> = BTreeMap::new();
    for at in &streamed.interiors {
        placing.insert(at.key(), (hash_of(at.map), at.placed.node));
    }
    seen.interiors = placing.values().copied().collect();
    seen.interiors_unread = (streamed.interiors_unread.iter())
        .map(|(at, _)| (hash_of(at.map), at.placed.node))
        .collect();
    for _ in 0..times {
        let ready = |model: AssetKey| match hold && loading(model.0 as u32) && model != BOX {
            true => NodeState::Loading,
            false => NodeState::Ready,
        };
        let selected = driver.select(view, &at, &ready);
        let of = &selected.instances;
        let mut rows = BTreeMap::new();
        for at in 0..of.len() {
            let (key, entity) = of.node[at];
            let place = match is_interior_key(key) {
                true => {
                    let (map, by) = placing[&key];
                    (map, Some(by), entity)
                }
                false => (hash_of(MapId::of_key(key).expect("a map")), None, entity),
            };
            let words = &of.words[at * of.width()..][..of.width()];
            let boxed = words[1] & !0xff;
            let model = of.model_of(at).expect("a model");
            assert_eq!(model == BOX, boxed != 0, "{place:?}");
            assert!(
                boxed != 0 || model == AssetKey(u64::from(words[0])),
                "{place:?}"
            );
            let row = Row {
                archetype: words[0],
                level: words[1] as u8,
                fade: of.fade[at].abs(),
                yields: of.fade[at] < 0.0,
                boxed,
                rows: of.rows[at],
            };
            assert!(rows.insert(place, row).is_none(), "{place:?} drawn twice");
        }
        seen.selections.push(rows);
        seen.waiting.push(
            selected
                .waiting
                .iter()
                .map(|model| model.0 as u32)
                .collect(),
        );
    }
    seen
}

/// How many instances the last selection of `seen` draws.
pub fn drawn(seen: &Seen) -> usize {
    seen.selections.last().map_or(0, BTreeMap::len)
}

/// The first difference between what the forest and the driver made of one view, in words; `None` where they are the same.
pub fn difference(forest: &Seen, driver: &Seen) -> Option<String> {
    if forest.wanted != driver.wanted {
        let lacking = |a: &Seen, b: &Seen| -> Vec<u32> {
            let maps = a.wanted.keys().filter(|map| !b.wanted.contains_key(map));
            maps.copied().collect()
        };
        let whys = forest
            .wanted
            .iter()
            .filter(|(map, why)| (driver.wanted.get(map)).is_some_and(|theirs| theirs != *why));
        let whys: Vec<String> = whys
            .map(|(map, why)| format!("{map:x}: {why:x?} against {:x?}", driver.wanted[map]))
            .collect();
        return Some(format!(
            "the maps wanted differ: of {} and {}, the forest alone wants {:x?}, the driver alone {:x?}, and {} are wanted for another reason, the first {:?}",
            forest.wanted.len(),
            driver.wanted.len(),
            lacking(forest, driver),
            lacking(driver, forest),
            whys.len(),
            whys.first()
        ));
    }
    let mut sorted = (forest.clone(), driver.clone());
    for seen in [&mut sorted.0, &mut sorted.1] {
        seen.unread.sort_unstable();
        seen.interiors.sort_unstable();
        seen.interiors_unread.sort_unstable();
    }
    let (f, d) = &sorted;
    if (&f.unread, &f.interiors, &f.interiors_unread)
        != (&d.unread, &d.interiors, &d.interiors_unread)
    {
        return Some(format!(
            "what was unread or brought in differs: the forest {:x?} {:x?} {:x?}, the driver {:x?} {:x?} {:x?}",
            f.unread, f.interiors, f.interiors_unread, d.unread, d.interiors, d.interiors_unread
        ));
    }
    for (pass, (theirs, ours)) in f.selections.iter().zip(&d.selections).enumerate() {
        let places = theirs.keys().chain(ours.keys());
        if let Some(place) = places.into_iter().find(|p| theirs.get(*p) != ours.get(*p)) {
            return Some(format!(
                "selection {pass}: at {place:x?} the forest draws {:x?} and the driver {:x?} ({} against {} instances)",
                theirs.get(place),
                ours.get(place),
                theirs.len(),
                ours.len()
            ));
        }
    }
    if f.waiting != d.waiting {
        return Some(format!(
            "the models waited for differ: the forest {:x?}, the driver {:x?}",
            f.waiting, d.waiting
        ));
    }
    None
}

/// Both sides asked about a camera at `eye` at `hour` under `weather`, with every model ready and then with one in three loading; the instances drawn, or the first difference.
pub fn compared(
    world: &Arc<Gta5World>,
    index: &Arc<Index>,
    reference: &WorldIndex,
    eye: Vec3,
    (hour, weather): (u8, &str),
) -> Result<usize, String> {
    let view = View::at(eye, 1.0);
    let state = WorldState::at(hour, weather);
    let mut most = 0;
    for hold in [false, true] {
        let forest = by_the_forest(world, reference, &view, &state, hold, 3);
        let driver = by_the_driver(world, index, &view, (hour, state.weather), hold, 3);
        if let Some(differs) = difference(&forest, &driver) {
            return Err(format!(
                "at {eye}, {hour} h, {weather}, one model in three loading: {hold}: {differs}"
            ));
        }
        most = most.max(drawn(&driver));
    }
    Ok(most)
}
