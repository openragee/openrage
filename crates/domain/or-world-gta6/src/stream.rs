//! The maps streamed in around the camera and what of them draws: each wanted map's body is read
//! on a worker into the nodes of the level-of-detail forest, and a selection gives each entity and
//! each instance that draws with the model that draws it, where it stands and how opaque it is.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::streaming_rules::{BAND_INTERIOR_HIGH, BAND_INTERIOR_LOW, BAND_LOW_DETAIL, BAND_OTHER};
use glam::Vec3;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta5::data::visual_settings::VisualSettings;
use or_world::streaming::{Extent, Forest, Kind, Level, NodeId, NodeState, View, FADE};

use crate::index::{Enabled, ModelRef, WorldIndex};
use or_world::pool::Pool;

#[path = "stream_census.rs"]
mod census;
#[path = "stream_grass.rs"]
mod grass;
#[path = "stream_read.rs"]
mod read;
#[path = "stream_slod.rs"]
pub mod slod;
#[path = "stream_world.rs"]
mod world;

pub use census::Census;
pub use read::Rows;
use read::{place, stretch, Drawn, Instances, Loaded};
pub use read::{read_map, ReadMap};
pub use slod::{CardList, Species};
#[cfg(test)]
use world::velocity_at;
pub use world::World;

/// The bytes a map body is read to.
const BODY_LIMIT: u64 = 256 << 20;
/// How far the camera moves before the wanted maps are worked out again, in metres.
const RESTREAM: f32 = 4.0;
/// The hour the world starts at.
const NOON: f32 = 12.0;

/// The band past its distance each level fades out over, as the install's visual settings state it, by `Level as usize`, in metres; [`FADE`] where they do not.
pub fn fade_of(settings: &VisualSettings) -> [f32; Level::COUNT] {
    ["hd", "lod", "slod1", "slod2", "slod3", "orphanhd", "slod4"]
        .map(|level| settings.get_f32(&format!("lod.fadedist.{level}"), FADE))
}

/// One thing a selection draws: its model, where it stands, and its sphere in the world.
#[derive(Clone, Copy)]
pub struct Instance {
    /// Which of the selection's models draws it.
    pub model: u32,
    pub rows: Rows,
    pub centre: Vec3,
    pub radius: f32,
    /// How opaque it is, to 1: below 0 where it fades out for its children, drawn by the inverse of their dither.
    pub fade: f32,
    /// The colour its material's arithmetic tints it by: white but for grass.
    pub tint: [f32; 3],
    /// How much of the natural and of the artificial ambient light it takes: outdoors but inside an interior.
    pub ambient: [f32; 2],
    /// Whether it casts its shadow and is not seen.
    pub cast_only: bool,
}

/// A run of a selection's instances that stand together, and the sphere that holds them all: a camera that does not see the sphere sees none of them.
#[derive(Clone, Copy)]
pub struct Chunk {
    pub centre: Vec3,
    pub radius: f32,
    pub from: u32,
    pub to: u32,
    /// A swap of [`Selected::swaps`] by its place, and whether the chunk draws once the swap's models are ready or until then.
    pub gate: Option<(u32, bool)>,
}

/// The models a run of a selection's instances waits for: once all are ready a frame draws what the next selection will, without waiting for it.
#[derive(Debug, Default)]
pub struct Swap {
    pub waits: Vec<ModelRef>,
    /// The nodes drawn until then and from then, by map name, place and fade.
    pub before: Vec<(u32, u32, f32)>,
    pub after: Vec<(u32, u32, f32)>,
}

/// How the streaming stands, for the screen to say.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Streaming {
    pub maps_wanted: usize,
    pub maps_loaded: usize,
    /// Map bodies asked for and not read yet.
    pub maps_waiting: usize,
    pub nodes: usize,
    pub drawn: usize,
    /// The map bodies read since the world began, the seconds they waited from first asked to read in all and at most, and how long the one waiting longest now has waited.
    pub maps_read: usize,
    pub map_waits: f64,
    pub map_wait_most: f32,
    pub map_oldest: f32,
}

/// What draws for one place of the camera: every instance, in chunks, and the models they draw with.
#[derive(Default)]
pub struct Selected {
    /// Counted up with each selection, so that one already laid out is not laid out again.
    pub version: u64,
    /// Where the camera stood when it was made: a frame or more behind a camera in flight.
    pub eye: Vec3,
    /// How long the streaming and the selection that made it took on the world's thread.
    pub took: std::time::Duration,
    /// How far into the selection what it waits for was asked for.
    pub asked: std::time::Duration,
    /// How long the far stand-ins took to yield to the models drawn where their trees stand.
    pub handed: std::time::Duration,
    pub models: Vec<ModelRef>,
    pub chunks: Vec<Chunk>,
    pub instances: Vec<Instance>,
    /// What a frame swaps in as the models a held parent waits for arrive, by the place its chunks name.
    pub swaps: Vec<Swap>,
    /// The lists of far stand-ins that draw, as cards.
    pub cards: Vec<CardList>,
    /// Each far stand-in id's species, by the id a card's record bears.
    pub species: Arc<[Species]>,
    /// Each model waited for, with how far the nearest thing that waits for it is.
    pub wanted: Vec<(f32, ModelRef)>,
    /// The request band of each model waited for where nothing that waits for it is in view.
    pub bands: HashMap<ModelRef, f32>,
    /// The sphere of each thing that waits for a model, by the model.
    pub waiting_at: HashMap<ModelRef, Vec<(Vec3, f32)>>,
    pub streaming: Streaming,
    /// What it holds, counted for the check against what the game loads (`world dump`).
    pub census: Census,
    /// Whether a fade in time is under way: the next selection differs though nothing moves.
    pub fading: bool,
    /// The occluders of the occluder maps around the camera; the same list until one comes or goes.
    pub occluders: Arc<[Arc<or_io::gta6::map::occluders::MapOccluders>]>,
    /// The interior and room the camera stands in, where it stands in one.
    pub inside: Option<crate::interior::Inside>,
    /// The rooms the camera sees of each interior it looks into.
    pub rooms_seen: crate::interior::RoomsSeen,
    /// From the room the camera stands in, the pyramids through it the outside is seen through, as unit normals in the world; `None` where nothing of the outside is hidden.
    pub outside_through: Option<Vec<Vec<Vec3>>>,
}

/// The models of a selection, each once, in the order they were first drawn with.
#[derive(Default)]
struct Models {
    list: Vec<ModelRef>,
    places: HashMap<ModelRef, u32>,
}

impl Models {
    fn ix(&mut self, model: ModelRef) -> u32 {
        *self.places.entry(model).or_insert_with(|| {
            self.list.push(model);
            self.list.len() as u32 - 1
        })
    }
}

/// The maps around the camera.
pub struct Streamer {
    index: Arc<WorldIndex>,
    /// Each far stand-in id's species, worked out once from the index.
    species: Arc<[Species]>,
    forest: Forest,
    loaded: HashMap<u32, Loaded>,
    /// The maps the camera wants, by name, with their distance; their parents among them.
    wanted: HashMap<u32, f32>,
    /// A map that did not read is not asked for again.
    failed: HashSet<u32>,
    /// A script-managed map that read and holds nothing the forest draws is not wanted again.
    empty: HashSet<u32>,
    /// The maps wanted only as the parents of others, which are wanted whatever they hold.
    linked: HashSet<u32>,
    /// Where the wanted maps were last worked out.
    streamed_at: Option<Vec3>,
    /// What of the game is switched on: a map it does not allow is not wanted.
    enabled: Enabled,
    /// The hour of the day, 0 to 23: a time archetype draws inside its hours alone.
    hour: u32,
    readers: Pool<u32, Option<ReadMap>>,
    /// The occluder maps, streamed by a tier of their own: they hold nothing the forest draws.
    occluders: crate::occluders::Occluders,
    pub streaming: Streaming,
    /// When each map body waited for was first asked for.
    asked_at: HashMap<u32, Instant>,
    /// The maps taken in since the last selection that was made whole: what a thread that dies is taken to have died on.
    unproven: Arc<Mutex<Vec<u32>>>,
}

/// The sphere that holds the spheres of `instances`.
fn bound(instances: &[Instance]) -> (Vec3, f32) {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for i in instances {
        min = min.min(i.centre - Vec3::splat(i.radius));
        max = max.max(i.centre + Vec3::splat(i.radius));
    }
    ((min + max) / 2.0, (max - min).length() / 2.0)
}

/// End a chunk of the instances of `out` from `from` on, where there are any.
fn close(out: &mut Selected, from: usize) {
    close_gated(out, from, None);
}

/// [`close`], the chunk drawn as `gate` says.
fn close_gated(out: &mut Selected, from: usize, gate: Option<(u32, bool)>) {
    if out.instances.len() > from {
        let (centre, radius) = bound(&out.instances[from..]);
        out.chunks.push(Chunk {
            centre,
            radius,
            from: from as u32,
            to: out.instances.len() as u32,
            gate,
        });
    }
}

/// An instance's fade as the renderer takes it: below nought where it gives way to its children.
fn signed(fade: f32, yields: bool) -> f32 {
    if yields {
        -fade
    } else {
        fade
    }
}

/// What a selection hands what it waits for to: by model, the distance and the band.
type Asking<'a> = dyn FnMut(&HashMap<ModelRef, f32>, &HashMap<ModelRef, f32>) + 'a;

/// Raise the band `model` is asked for in to `band`.
fn raise(bands: &mut HashMap<ModelRef, f32>, model: ModelRef, band: f32) {
    let at = bands.entry(model).or_insert(band);
    *at = at.max(band);
}

/// The hour of the day `hour` falls in, 0 to 23.
fn hour_of(hour: f32) -> u32 {
    (hour.rem_euclid(24.0) as u32).min(23)
}

impl Streamer {
    #[cfg(any(test, feature = "made-up"))]
    pub fn new(index: Arc<WorldIndex>) -> Self {
        Self::skipping(index, HashSet::new(), Arc::default())
    }

    /// The maps around the camera of `index`, `skipped` never taken in, saying in `unproven` the maps it takes in until a selection is made whole.
    pub fn skipping(
        index: Arc<WorldIndex>,
        skipped: HashSet<u32>,
        unproven: Arc<Mutex<Vec<u32>>>,
    ) -> Self {
        let shared = Arc::clone(&index);
        let readers = Pool::new(
            4,
            HashMap::<u32, File>::new,
            move |files, name: u32| -> Option<ReadMap> {
                let row = shared.maps.get(shared.map_named(name)?)?;
                let file = match files.entry(row.archive) {
                    std::collections::hash_map::Entry::Occupied(open) => open.into_mut(),
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        let path = shared.archives.get(row.archive as usize)?;
                        slot.insert(File::open(path).ok()?)
                    }
                };
                let (find, bytes) = read_rpf9_body(&*file, row.offset, BODY_LIMIT).ok()?;
                let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                    resource: Some(resource),
                    ..
                }) = find.kind
                else {
                    return None;
                };
                let (virtual_section, physical) = resource.sections(&bytes);
                let map = or_io::gta6::map::stream::open(virtual_section, physical).ok()?;
                let mut read = read_map(&shared, &map);
                read.loaded.interiors =
                    crate::interior::read_placed(&shared, files, &map, virtual_section, physical);
                Some(read)
            },
        );
        let species = slod::species(&index);
        let occluders = crate::occluders::Occluders::new(Arc::clone(&index));
        Self {
            index,
            species,
            forest: Forest::new(),
            loaded: HashMap::new(),
            wanted: HashMap::new(),
            failed: skipped,
            empty: HashSet::new(),
            linked: HashSet::new(),
            streamed_at: None,
            enabled: Enabled::default(),
            hour: hour_of(NOON),
            readers,
            occluders,
            streaming: Streaming::default(),
            asked_at: HashMap::new(),
            unproven,
        }
    }

    /// How many map bodies and occluder maps are asked for and not read yet.
    #[cfg(any(test, feature = "made-up"))]
    pub fn waiting(&self) -> usize {
        self.streaming.maps_waiting + self.occluders.waiting()
    }

    /// Switch on what `enabled` names in place of what was on; whether that is a change.
    pub fn set_enabled(&mut self, enabled: Enabled) -> bool {
        if self.enabled == enabled {
            return false;
        }
        self.enabled = enabled;
        // The wanted maps are worked out again though the camera stands.
        self.streamed_at = None;
        true
    }

    /// Set the hour of the day, 0.0 to 24.0; whether a time archetype may come or go by it.
    pub fn set_hour(&mut self, hour: f32) -> bool {
        let hour = hour_of(hour);
        let changed = self.hour != hour;
        self.hour = hour;
        changed
    }

    /// Work out the maps the camera of `view` wants, ask for those not loaded and let go of the rest; whether a map came or went.
    pub fn stream(&mut self, view: &View) -> bool {
        let mut changed = false;
        let now = Instant::now();
        for (name, read) in self.readers.take(|| true) {
            if let Some(asked) = self.asked_at.remove(&name) {
                let waited = now.duration_since(asked).as_secs_f32();
                self.streaming.maps_read += 1;
                self.streaming.map_waits += f64::from(waited);
                self.streaming.map_wait_most = self.streaming.map_wait_most.max(waited);
            }
            match read {
                Some(read) => {
                    // ⚠ Said before it is taken in: what it holds may stop the thread on its way in.
                    (self.unproven.lock().unwrap_or_else(|p| p.into_inner())).push(name);
                    changed |= self.insert(name, read);
                }
                None => {
                    self.failed.insert(name);
                }
            }
        }
        let moved = (self.streamed_at).is_none_or(|at| at.distance(view.eye) >= RESTREAM);
        if moved {
            self.streamed_at = Some(view.eye);
            self.rewant(view);
        }
        changed |= self.occluders.stream(view, &self.enabled, moved);
        // What is loaded and no longer wanted goes.
        let gone: Vec<u32> = (self.loaded.keys().copied())
            .filter(|name| !self.wanted.contains_key(name))
            .collect();
        for name in gone {
            self.loaded.remove(&name);
            self.forest.remove(u64::from(name));
            changed = true;
        }
        let asked: Vec<(f32, u32)> = (self.wanted.iter())
            .filter(|(name, _)| !self.loaded.contains_key(name) && !self.failed.contains(name))
            .map(|(name, away)| (*away, *name))
            .collect();
        // What is no longer asked for has stopped waiting.
        let mut still = HashMap::with_capacity(asked.len());
        for (_, name) in &asked {
            still.insert(*name, *self.asked_at.get(name).unwrap_or(&now));
        }
        self.asked_at = still;
        self.streaming.map_oldest = (self.asked_at.values())
            .map(|at| now.duration_since(*at).as_secs_f32())
            .fold(0.0, f32::max);
        self.streaming.maps_wanted = self.wanted.len();
        self.streaming.maps_loaded = self.loaded.len();
        self.streaming.maps_waiting = asked.len();
        self.streaming.nodes = self.forest.len().1;
        self.readers.want(asked);
        changed
    }

    // ponytail: every map's box is tested each time the camera has moved; a grid of the boxes if the walk shows.
    fn rewant(&mut self, view: &View) {
        self.wanted.clear();
        let eye = view.eye;
        let mut parents = Vec::new();
        for map in (self.index.maps.iter()).filter(|m| self.enabled.allows(m)) {
            let boxes = map.boxes();
            if !boxes.wants(eye) {
                continue;
            }
            let (min, max) = boxes.entities;
            let away = eye.distance(eye.clamp(min, max));
            parents.extend(self.index.parents_of(map).iter().map(|p| (*p, away)));
            // ⚠ A script-managed map is held back before an empty one is, so one that holds
            // nothing is found empty only once it is read.
            if !self.empty.contains(&map.name) {
                self.wanted.insert(map.name, away);
            }
        }
        // A wanted map's parents are wanted with it, and theirs.
        self.linked.clear();
        while let Some((name, away)) = parents.pop() {
            let Some(at) = self.index.map_named(name) else {
                continue;
            };
            let map = &self.index.maps[at];
            // ⚠ A parent that is not switched on is no part of the world, but one that holds
            // nothing (`held` 3) is still a link to its own parents.
            let off = !self.enabled.allows(map) && map.held != 3;
            if off {
                continue;
            }
            // ⚠ A map wanted for itself may be a parent too: it is linked though already wanted.
            self.linked.insert(name);
            if self.wanted.contains_key(&name) {
                continue;
            }
            self.wanted.insert(name, away);
            parents.extend(self.index.parents_of(map).iter().map(|p| (*p, away)));
        }
    }

    /// Take the map `name` in where it is still wanted; whether it was.
    fn insert(&mut self, name: u32, read: ReadMap) -> bool {
        if !self.wanted.contains_key(&name) {
            return false;
        }
        let managed = (self.index.map_named(name)).is_some_and(|at| self.index.maps[at].held == 2);
        if read.nodes.is_empty() && managed && !self.linked.contains(&name) {
            self.empty.insert(name);
            self.wanted.remove(&name);
            return false;
        }
        self.forest.insert(u64::from(name), read.nodes);
        self.loaded.insert(name, read.loaded);
        true
    }

    /// What draws for the camera of `view`; `ready` says whether a model is on the device, and `None` that it never will be.
    #[cfg(any(test, feature = "made-up"))]
    pub fn select(&mut self, view: &View, ready: &dyn Fn(ModelRef) -> Option<bool>) -> Selected {
        self.select_asking(view, ready, &mut |_, _| {})
    }

    /// [`Self::select`], handing `asking` what the maps' nodes wait for, by model with its distance and its band, as soon as it is known.
    fn select_asking(
        &mut self,
        view: &View,
        ready: &dyn Fn(ModelRef) -> Option<bool>,
        asking: &mut Asking<'_>,
    ) -> Selected {
        let began = Instant::now();
        let (loaded, hour, eye) = (&self.loaded, self.hour, view.eye);
        let drawn_of = |id: NodeId| {
            let map = loaded.get(&(id.0 as u32))?;
            Some((map, map.drawn.get(id.1 as usize)?))
        };
        // A time archetype outside its hours has no model for the while: it draws nothing, is
        // not waited for and holds nothing back.
        let model_of = |id: NodeId| drawn_of(id).and_then(|(_, drawn)| drawn.model_at(hour));
        // A list of far stand-ins waits for no model: its cards draw once its species' atlas is there.
        let cards_of = |id: NodeId| {
            let (map, _) = drawn_of(id)?;
            let list = map
                .lists
                .get((id.1 as usize).checked_sub(map.entities.len())?)?;
            match &list.instances {
                Instances::Cards(cards) => Some(cards),
                _ => None,
            }
        };
        // An interior's instance draws its contents, which wait for nothing as a whole.
        let interior_of = |id: NodeId| {
            let map = loaded.get(&(id.0 as u32))?;
            (map.interiors.iter()).find(|p| p.entity == id.1 as usize)
        };
        let state = |id: NodeId| match model_of(id).map(ready) {
            _ if cards_of(id).is_some() => NodeState::Ready,
            None if interior_of(id).is_some() => NodeState::Ready,
            None | Some(None) => NodeState::Absent,
            Some(Some(true)) => NodeState::Ready,
            Some(Some(false)) => NodeState::Loading,
        };
        let mut selection = self.forest.select(view, &state);
        let mut out = Selected {
            fading: selection.fading,
            eye,
            ..Selected::default()
        };
        // Each model waited for once, by the nearest node that waits for it, with the highest band of
        // what waits for it as though out of view: a far level's, or the rest's.
        let mut waited: HashMap<ModelRef, f32> = HashMap::new();
        for id in &selection.wanted {
            let (Some(model), Some(node)) = (model_of(*id), self.forest.node(*id)) else {
                continue;
            };
            let away = eye.distance(node.position);
            let nearest = waited.entry(model).or_insert(away);
            *nearest = nearest.min(away);
            let band = match node.level {
                Level::Hd | Level::OrphanHd => BAND_OTHER,
                _ => BAND_LOW_DETAIL,
            };
            raise(&mut out.bands, model, band);
            let sphere = match node.extent {
                Extent::Box(min, max) => Some(((min + max) / 2.0, (max - min).length() / 2.0)),
                Extent::Point => drawn_of(*id).and_then(|(map, drawn)| {
                    let rows = map.entities.get(id.1 as usize)?;
                    Some((place(rows, drawn.centre), drawn.radius * stretch(rows)))
                }),
            };
            if let Some(sphere) = sphere {
                out.waiting_at.entry(model).or_default().push(sphere);
            }
        }
        // ⚠ Asked for before the instances and the far stand-ins are laid out, which is most of a selection's time.
        asking(&waited, &out.bands);
        out.asked = began.elapsed();
        let mut models = Models::default();
        let instance = |model: u32, rows: Rows, drawn: &Drawn, fade: f32, tint| Instance {
            model,
            rows,
            centre: place(&rows, drawn.centre),
            radius: drawn.radius * stretch(&rows),
            fade,
            tint,
            ambient: crate::ambient::OUTDOORS,
            cast_only: false,
        };
        // An entity's instance, `fade` signed; whether it has one.
        let lay_entity = |out: &mut Selected, models: &mut Models, id: NodeId, fade: f32| {
            let Some((map, drawn)) = drawn_of(id) else {
                return false;
            };
            let rows = map.entities.get(id.1 as usize);
            let (Some(model), Some(rows)) = (drawn.model_at(hour), rows) else {
                return false;
            };
            let model = models.ix(model);
            (out.instances).push(instance(model, *rows, drawn, fade, [1.0; 3]));
            true
        };
        // A list's instances less `skipped`, `fade` signed; whether they are grass.
        let lay_list =
            |out: &mut Selected, models: &mut Models, id: NodeId, fade, skipped: &[u32]| {
                let Some((map, drawn)) = drawn_of(id) else {
                    return false;
                };
                let list = (id.1 as usize)
                    .checked_sub(map.entities.len())
                    .and_then(|at| map.lists.get(at));
                let (Some(model), Some(list)) = (drawn.model_at(hour), list) else {
                    return false;
                };
                let model = models.ix(model);
                // ⚠ An instance's own distances take the list's whole scale, grass's included.
                let scale = (self.forest.node(id)).map_or(1.0, |node| node.scale(view));
                // An instance that fades by its own distance fades with its list too.
                list.each(eye, scale, skipped, |rows, own, tint| {
                    (out.instances).push(instance(model, rows, drawn, own * fade, tint));
                });
                matches!(list.instances, Instances::Grass(_))
            };
        // The parents held for a child on its way draw in chunks of their own, which a frame swaps.
        let held_at: HashMap<NodeId, usize> = (selection.held.iter().enumerate())
            .map(|(at, held)| (held.node, at))
            .collect();
        // The entities of one map stand together: a chunk a map.
        selection.drawn.sort_unstable_by_key(|d| d.node);
        // How opaque the forest draws each interior's instance, by its map and place.
        let mut interiors: HashMap<(u32, usize), f32> = HashMap::new();
        let (mut from, mut of_map) = (0, None);
        for draw in &selection.drawn {
            let id = draw.node;
            if of_map != Some(id.0) {
                close(&mut out, from);
                (from, of_map) = (out.instances.len(), Some(id.0));
            }
            if held_at.contains_key(&id) && model_of(id).is_some() {
                continue;
            }
            if !lay_entity(&mut out, &mut models, id, signed(draw.fade, draw.yields))
                && interior_of(id).is_some()
            {
                interiors.insert((id.0 as u32, id.1 as usize), draw.fade);
            }
        }
        close(&mut out, from);
        // A prop list of the batch LOD level that draws leaves out the instances of its parent prop
        // list it stands in for, all of them where it names none; the stand-ins' own come after.
        let mut hidden: HashMap<NodeId, Vec<u32>> = HashMap::new();
        for draw in &selection.lists {
            let Some((map, _)) = drawn_of(draw.node) else {
                continue;
            };
            let list = (draw.node.1 as usize)
                .checked_sub(map.entities.len())
                .and_then(|at| map.lists.get(at));
            let parent = (self.forest.node(draw.node)).and_then(|node| node.parent);
            let Some((list, parent)) = list.zip(parent).filter(|(list, _)| list.batch_lod) else {
                continue;
            };
            let at = (parent.map, parent.node);
            let of_props = (self.forest.node(at)).is_some_and(|node| node.kind == Kind::Props);
            let count = drawn_of(at)
                .and_then(|(map, _)| {
                    map.lists
                        .get((at.1 as usize).checked_sub(map.entities.len())?)
                })
                .map_or(0, |parent| match &parent.instances {
                    Instances::Placed(placed) => placed.len(),
                    _ => 0,
                });
            if of_props {
                let left_out = hidden.entry(at).or_default();
                match list.stands_in_for.is_empty() {
                    true => left_out.extend(0..count as u32),
                    false => left_out.extend(list.stands_in_for.iter().map(|i| u32::from(*i))),
                }
            }
        }
        // And the instances of one list: a chunk a list.
        let listed = out.instances.len();
        // The far stand-ins drawn, laid out once every model is placed, and the instances of grass
        let (mut stand_ins, mut grass) = (Vec::new(), Vec::new());
        for draw in &selection.lists {
            if let Some(cards) = cards_of(draw.node) {
                let scale = (self.forest.node(draw.node)).map_or(1.0, |node| node.scale(view));
                stand_ins.push((cards, scale, draw.fade));
                continue;
            }
            if held_at.contains_key(&draw.node) {
                continue;
            }
            let mut skipped = hidden.remove(&draw.node).unwrap_or_default();
            skipped.extend(&selection.skipped[draw.skipped.clone()]);
            skipped.sort_unstable();
            let from = out.instances.len();
            let fade = signed(draw.fade, draw.yields);
            if lay_list(&mut out, &mut models, draw.node, fade, &skipped) {
                grass.push(from..out.instances.len());
            }
            close(&mut out, from);
        }
        // Each held parent until its children are ready and from then, and they from then; a child
        // whose parent does not draw, from then alone. What draws from then stands for no far stand-in.
        let mut children: HashMap<NodeId, Vec<&or_world::streaming::Waiting>> = HashMap::new();
        for waiting in &selection.waiting {
            children.entry(waiting.parent).or_default().push(waiting);
        }
        let was_drawn = (selection
            .drawn
            .iter()
            .map(|d| (d.node, (d.fade, d.yields, 0..0))))
        .chain((selection.lists.iter()).map(|l| (l.node, (l.fade, l.yields, l.skipped.clone()))))
        .filter(|(node, _)| held_at.contains_key(node));
        let was_drawn: HashMap<NodeId, (f32, bool, std::ops::Range<usize>)> = was_drawn.collect();
        let is_list =
            |id: NodeId| (self.forest.node(id)).is_some_and(|n| n.extent != Extent::Point);
        // A held parent's or a child's instances, less `own` and what batch children leave out; where they start.
        let lay = |out: &mut Selected, models: &mut Models, id, fade, own: Vec<u32>| {
            let from = out.instances.len();
            let mut skipped = hidden.get(&id).cloned().unwrap_or_default();
            skipped.extend(own);
            skipped.sort_unstable();
            match is_list(id) {
                true => lay_list(out, models, id, fade, &skipped),
                false => lay_entity(out, models, id, fade),
            };
            from
        };
        let parents = (selection.held.iter()).map(|held| (held.node, Some(held)));
        let mut orphans: Vec<NodeId> = (children.keys().copied())
            .filter(|parent| !held_at.contains_key(parent))
            .collect();
        orphans.sort_unstable();
        let mut after = Vec::new();
        for (parent, held) in parents.chain(orphans.into_iter().map(|parent| (parent, None))) {
            let gate = out.swaps.len() as u32;
            let kids = children.remove(&parent).unwrap_or_default();
            let mut swap = Swap {
                waits: kids.iter().filter_map(|w| model_of(w.node)).collect(),
                ..Swap::default()
            };
            let name = |id: NodeId, fade: f32| (id.0 as u32, id.1, fade);
            // An interior's instance and a list of far stand-ins drew as they do, and are not swapped.
            let held = held.filter(|_| model_of(parent).is_some() && cards_of(parent).is_none());
            if let (Some(held), Some((fade, yields, own))) = (held, was_drawn.get(&parent)) {
                let skipped = selection.skipped[own.clone()].to_vec();
                let from = lay(
                    &mut out,
                    &mut models,
                    parent,
                    signed(*fade, *yields),
                    skipped.clone(),
                );
                close_gated(&mut out, from, Some((gate, false)));
                swap.before.push(name(parent, *fade));
                if held.then > 0.0 {
                    let mut then = skipped;
                    then.extend(&selection.skipped[held.skipped.clone()]);
                    let from = lay(
                        &mut out,
                        &mut models,
                        parent,
                        signed(held.then, held.yields),
                        then,
                    );
                    close_gated(&mut out, from, Some((gate, true)));
                    after.push(from..out.instances.len());
                    swap.after.push(name(parent, held.then));
                }
            }
            for kid in kids {
                let skipped = selection.skipped[kid.skipped.clone()].to_vec();
                let from = lay(
                    &mut out,
                    &mut models,
                    kid.node,
                    signed(kid.fade, kid.yields),
                    skipped,
                );
                close_gated(&mut out, from, Some((gate, true)));
                after.push(from..out.instances.len());
                swap.after.push(name(kid.node, kid.fade));
            }
            out.swaps.push(swap);
        }
        // A far stand-in yields to the model of its tree where one is drawn, and draws where none is.
        // ⚠ Each instance once against a mark, not against every run of grass: a city camera's thousands of runs made this seconds of each selection.
        let handing = Instant::now();
        let mut of_grass = vec![false; out.instances.len()];
        for run in grass.into_iter().chain(after) {
            of_grass[run].fill(true);
        }
        let standing = slod::Standing::of(
            (out.instances.iter().zip(&of_grass))
                .filter(|(_, grass)| !**grass)
                .map(|(i, _)| Vec3::new(i.rows[0][3], i.rows[1][3], i.rows[2][3])),
        );
        for (cards, scale, fade) in stand_ins {
            (out.cards).push(cards.seen(scale, fade, &cards.handed_off(&standing)));
        }
        out.handed = handing.elapsed();
        let list_instances = out.instances.len() - listed;
        // What the maps place inside an interior near the camera takes the interior's ambient light.
        // ponytail: every selection looks again at each instance near an interior in range; held
        // per map body if the selection's time shows it.
        for interior in loaded.values().flat_map(|map| &map.interiors) {
            if interior.in_range(eye, view) {
                interior.shade(&out.chunks, &mut out.instances);
            }
        }
        // The contents of each interior whose instance draws, or that the camera stands in: a
        // chunk an interior, as opaque as its instance.
        let outdoors = out.instances.len();
        // The interiors in range, in one order whatever the maps' order, and what the camera sees of them.
        let mut near: Vec<(u32, &crate::interior::Placed)> = (loaded.iter())
            .flat_map(|(name, map)| map.interiors.iter().map(move |i| (*name, i)))
            .filter(|(_, i)| i.in_range(eye, view))
            .collect();
        near.sort_by_key(|(name, i)| (*name, i.entity));
        let shells: Vec<_> = near.iter().map(|(_, i)| i.shell()).collect();
        let walked = crate::interior_rooms::walk(&shells, eye);
        out.inside = (walked.inside).and_then(|(k, room)| near.get(k)?.1.inside(room));
        let mut through = None;
        for (k, ((name, interior), seen)) in near.iter().zip(&walked.seen).enumerate() {
            let inside = walked.standing.is_some_and(|(at, _)| at == k);
            let drawn = interiors.get(&(*name, interior.entity)).copied();
            let Some(fade) = drawn.or(inside.then_some(1.0)) else {
                continue;
            };
            let from = out.instances.len();
            interior.each(view, hour, fade, seen, |child, away, fade| {
                match ready(child.model) {
                    Some(true) => {
                        let model = models.ix(child.model);
                        (out.instances).push(Instance {
                            model,
                            rows: child.rows,
                            centre: child.centre,
                            radius: child.radius,
                            fade,
                            tint: [1.0; 3],
                            ambient: child.ambient,
                            cast_only: false,
                        });
                    }
                    Some(false) => {
                        let nearest = waited.entry(child.model).or_insert(away);
                        *nearest = nearest.min(away);
                        // ponytail: the build picks 4, 3 or 2 by the interior and room the camera stands in, its mapping not read: 4 inside this interior, 2 outside it.
                        let band = match inside {
                            true => BAND_INTERIOR_HIGH,
                            false => BAND_INTERIOR_LOW,
                        };
                        raise(&mut out.bands, child.model, band);
                        let at = out.waiting_at.entry(child.model).or_default();
                        at.push((child.centre, child.radius));
                    }
                    None => {}
                }
            });
            if let (true, None, Some(pyramids)) = (inside, &through, &seen.outside) {
                through = Some((*interior, pyramids.clone()));
            }
            (out.rooms_seen).insert((*name, interior.entity as u32), seen.rooms.clone());
            close(&mut out, from);
        }
        // ⚠ GTA V's rule: from a room the outside is seen through the portals to it alone; what stands
        // out of their sight still casts the sun's shadow in.
        if let Some((interior, through)) = &through {
            // ⚠ Measured: a map places a room's walls and roof, which reach into its box.
            let sees = |i: &Instance| {
                interior.reaches_a_room(i.centre, i.radius)
                    || (through.iter())
                        .any(|p| crate::interior_rooms::reaches(p, eye, i.centre, i.radius))
            };
            for instance in &mut out.instances[..outdoors] {
                if instance.ambient == crate::ambient::OUTDOORS && !sees(instance) {
                    instance.cast_only = true;
                }
            }
        }
        out.outside_through = through.map(|(_, through)| through);
        out.occluders = self.occluders.held();
        out.wanted = (waited.into_iter().map(|(model, away)| (away, model))).collect();
        out.species = Arc::clone(&self.species);
        out.models = models.list;
        self.streaming.drawn = out.instances.len();
        out.streaming = self.streaming;
        out.census = census::count(
            &self.wanted,
            loaded,
            &self.forest,
            &selection,
            list_instances,
        );
        out
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
