//! The maps streamed in around the camera and what of them draws: each wanted map's body is read
//! on a worker into the nodes of the level-of-detail forest, and a selection gives each entity and
//! each instance that draws with the model that draws it, where it stands and how opaque it is.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use glam::Vec3;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta5::data::visual_settings::VisualSettings;
use or_world::streaming::{Extent, Forest, Kind, Level, NodeId, NodeState, View, FADE};

use super::index::{Enabled, ModelRef, WorldIndex};
use super::pool::Pool;

#[path = "stream_census.rs"]
mod census;
#[path = "stream_grass.rs"]
mod grass;
#[path = "stream_read.rs"]
mod read;
#[path = "stream_slod.rs"]
pub mod slod;

pub use census::Census;
pub use read::Rows;
use read::{place, stretch, Drawn, Instances, Loaded};
pub(super) use read::{read_map, ReadMap};
pub use slod::{CardList, Species};

/// The bytes a map body is read to.
const BODY_LIMIT: u64 = 256 << 20;
/// How far the camera moves before the wanted maps are worked out again, in metres.
const RESTREAM: f32 = 4.0;
/// The hour the world starts at.
const NOON: f32 = 12.0;

/// The band past its distance each level fades out over, as the install's visual settings state
/// it, by `Level as usize`, in metres; [`FADE`] where they do not.
pub(super) fn fade_of(settings: &VisualSettings) -> [f32; Level::COUNT] {
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

/// A run of a selection's instances that stand together, and the sphere that holds them all: a
/// camera that does not see the sphere sees none of them.
#[derive(Clone, Copy)]
pub struct Chunk {
    pub centre: Vec3,
    pub radius: f32,
    pub from: u32,
    pub to: u32,
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
}

/// What draws for one place of the camera: every instance, in chunks, and the models they draw with.
#[derive(Default)]
pub struct Selected {
    /// Counted up with each selection, so that one already laid out is not laid out again.
    pub version: u64,
    /// Where the camera stood when it was made: a frame or more behind a camera in flight.
    pub eye: Vec3,
    pub models: Vec<ModelRef>,
    pub chunks: Vec<Chunk>,
    pub instances: Vec<Instance>,
    /// The lists of far stand-ins that draw, as cards.
    pub cards: Vec<CardList>,
    /// Each far stand-in id's species, by the id a card's record bears.
    pub species: Arc<[Species]>,
    /// Each model waited for, with how far the nearest thing that waits for it is.
    pub wanted: Vec<(f32, ModelRef)>,
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
    pub inside: Option<super::interior::Inside>,
    /// The rooms the camera sees of each interior it looks into.
    pub(super) rooms_seen: super::interior::RoomsSeen,
    /// From the room the camera stands in, the pyramids through it the outside is seen through, as
    /// unit normals in the world; `None` where nothing of the outside is hidden.
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
pub(super) struct Streamer {
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
    occluders: super::occluders::Occluders,
    pub(super) streaming: Streaming,
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
    if out.instances.len() > from {
        let (centre, radius) = bound(&out.instances[from..]);
        out.chunks.push(Chunk {
            centre,
            radius,
            from: from as u32,
            to: out.instances.len() as u32,
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

/// The hour of the day `hour` falls in, 0 to 23.
fn hour_of(hour: f32) -> u32 {
    (hour.rem_euclid(24.0) as u32).min(23)
}

impl Streamer {
    #[cfg(test)]
    pub(super) fn new(index: Arc<WorldIndex>) -> Self {
        Self::skipping(index, HashSet::new(), Arc::default())
    }

    /// The maps around the camera of `index`, `skipped` never taken in, saying in `unproven` the maps it takes in until a selection is made whole.
    pub(super) fn skipping(
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
                    super::interior::read_placed(&shared, files, &map, virtual_section, physical);
                Some(read)
            },
        );
        let species = slod::species(&index);
        let occluders = super::occluders::Occluders::new(Arc::clone(&index));
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
            unproven,
        }
    }

    /// Switch on what `enabled` names in place of what was on; whether that is a change.
    pub(super) fn set_enabled(&mut self, enabled: Enabled) -> bool {
        if self.enabled == enabled {
            return false;
        }
        self.enabled = enabled;
        // The wanted maps are worked out again though the camera stands.
        self.streamed_at = None;
        true
    }

    /// Set the hour of the day, 0.0 to 24.0; whether a time archetype may come or go by it.
    pub(super) fn set_hour(&mut self, hour: f32) -> bool {
        let hour = hour_of(hour);
        let changed = self.hour != hour;
        self.hour = hour;
        changed
    }

    /// Work out the maps the camera of `view` wants, ask for those not loaded and let go of the
    /// rest; whether a map came or went.
    pub(super) fn stream(&mut self, view: &View) -> bool {
        let mut changed = false;
        for (name, read) in self.readers.take(|| true) {
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
            if !boxes.wanted(view, map.content_flags) {
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

    /// What draws for the camera of `view`; `ready` says whether a model is on the device, and
    /// `None` that it never will be.
    pub(super) fn select(
        &mut self,
        view: &View,
        ready: &dyn Fn(ModelRef) -> Option<bool>,
    ) -> Selected {
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
        let mut models = Models::default();
        let instance = |model: u32, rows: Rows, drawn: &Drawn, fade: f32, tint| Instance {
            model,
            rows,
            centre: place(&rows, drawn.centre),
            radius: drawn.radius * stretch(&rows),
            fade,
            tint,
            ambient: or_renderer::OUTDOORS,
            cast_only: false,
        };
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
            let Some((map, drawn)) = drawn_of(id) else {
                continue;
            };
            let rows = map.entities.get(id.1 as usize);
            if let (Some(model), Some(rows)) = (drawn.model_at(hour), rows) {
                let model = models.ix(model);
                (out.instances).push(instance(
                    model,
                    *rows,
                    drawn,
                    signed(draw.fade, draw.yields),
                    [1.0; 3],
                ));
            } else if interior_of(id).is_some() {
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
            let Some((map, drawn)) = drawn_of(draw.node) else {
                continue;
            };
            let list = (draw.node.1 as usize)
                .checked_sub(map.entities.len())
                .and_then(|at| map.lists.get(at));
            let (Some(model), Some(list)) = (drawn.model_at(hour), list) else {
                continue;
            };
            let model = models.ix(model);
            let mut skipped = hidden.remove(&draw.node).unwrap_or_default();
            skipped.extend(&selection.skipped[draw.skipped.clone()]);
            skipped.sort_unstable();
            let from = out.instances.len();
            // ⚠ An instance's own distances take the list's whole scale, grass's included.
            let scale = (self.forest.node(draw.node)).map_or(1.0, |node| node.scale(view));
            // An instance that fades by its own distance fades with its list too.
            list.each(eye, scale, &skipped, |rows, fade, tint| {
                (out.instances).push(instance(
                    model,
                    rows,
                    drawn,
                    fade * signed(draw.fade, draw.yields),
                    tint,
                ));
            });
            if matches!(list.instances, Instances::Grass(_)) {
                grass.push(from..out.instances.len());
            }
            close(&mut out, from);
        }
        // A far stand-in yields to the model of its tree where one is drawn, and draws where none is.
        let standing = slod::Standing::of(
            (out.instances.iter().enumerate())
                .filter(|(at, _)| !grass.iter().any(|run| run.contains(at)))
                .map(|(_, i)| Vec3::new(i.rows[0][3], i.rows[1][3], i.rows[2][3])),
        );
        for (cards, scale, fade) in stand_ins {
            (out.cards).push(cards.seen(scale, fade, &cards.handed_off(&standing)));
        }
        let list_instances = out.instances.len() - listed;
        // Each model waited for once, by the nearest node that waits for it; one the camera's
        // velocity brings into range after those in range, by the way to it.
        let mut waited: HashMap<ModelRef, f32> = HashMap::new();
        let lead = eye.distance(view.lead());
        let now = selection.wanted.iter().map(|id| (id, 0.0));
        for (id, later) in now.chain(selection.ahead.iter().map(|id| (id, lead))) {
            let (Some(model), Some(node)) = (model_of(*id), self.forest.node(*id)) else {
                continue;
            };
            let away = eye.distance(node.position) + later;
            let nearest = waited.entry(model).or_insert(away);
            *nearest = nearest.min(away);
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
        let mut through = None;
        for (name, map) in loaded {
            for interior in &map.interiors {
                let inside = interior.inside(eye);
                out.inside = out.inside.or(inside);
                let drawn = interiors.get(&(*name, interior.entity)).copied();
                let Some(fade) = drawn.or(inside.map(|_| 1.0)) else {
                    continue;
                };
                let from = out.instances.len();
                let seen = interior.each(view, hour, fade, |child, away, fade| {
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
                            let at = out.waiting_at.entry(child.model).or_default();
                            at.push((child.centre, child.radius));
                        }
                        None => {}
                    }
                });
                if let Some(seen) = seen {
                    if let (Some(_), None, Some(pyramids)) = (inside, &through, &seen.outside) {
                        through = Some((interior, interior.to_world(pyramids)));
                    }
                    (out.rooms_seen).insert((*name, interior.entity as u32), seen.rooms);
                }
                close(&mut out, from);
            }
        }
        // ⚠ GTA V's rule: from a room the outside is seen through the portals to it alone; what stands
        // out of their sight still casts the sun's shadow in.
        if let Some((interior, through)) = &through {
            // ⚠ Measured: a map places a room's walls and roof, which reach into its box.
            let sees = |i: &Instance| {
                interior.reaches_a_room(i.centre, i.radius)
                    || (through.iter())
                        .any(|p| super::interior_rooms::reaches(p, eye, i.centre, i.radius))
            };
            for instance in &mut out.instances[..outdoors] {
                if instance.ambient == or_renderer::OUTDOORS && !sees(instance) {
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

/// How far the camera moves before what draws is selected again, in metres.
const RESELECT: f32 = 0.5;
/// How long the world's thread waits for a change before it looks for map bodies that arrived.
const TICK: std::time::Duration = std::time::Duration::from_millis(20);
/// The shortest time between two passes a speed is measured over, in seconds.
const SPEED_STEP: f32 = 0.001;
/// How long a camera told no new place keeps its speed: the thread passes more often than frames come.
// ponytail: a second, longer than a frame of the shots run under load; tell the thread each frame's own velocity if a
// camera that stops must count as still sooner.
const STILL: std::time::Duration = std::time::Duration::from_secs(1);

/// The camera's velocity at `now` where it is at `eye`, given where and when it last moved (`moved`)
/// and the velocity it had then.
// ⚠ A pass between two frames finds the camera where it was: measured there, a camera in flight
// would read as still, and a map that arrives then would fade in as if it were.
fn velocity_at(
    moved: &mut Option<(Vec3, Instant)>,
    velocity: Vec3,
    eye: Vec3,
    now: Instant,
) -> Vec3 {
    match *moved {
        None => {
            *moved = Some((eye, now));
            Vec3::ZERO
        }
        Some((at, then)) if at != eye => {
            let step = now.duration_since(then).as_secs_f32();
            if step < SPEED_STEP {
                return velocity;
            }
            *moved = Some((eye, now));
            (eye - at) / step
        }
        // A camera at rest that moves again is measured from this pass, not from when it stopped.
        Some((at, then)) if now.duration_since(then) >= STILL => {
            *moved = Some((at, now));
            Vec3::ZERO
        }
        Some(_) => velocity,
    }
}

/// What the screen tells the world's thread.
#[derive(Default)]
struct Told {
    eye: Vec3,
    scale: f32,
    hour: f32,
    /// The band past its distance each level fades out over, by `Level as usize`, in metres.
    fade: [f32; Level::COUNT],
    /// What of the game to switch on, where that was told since the thread last took it.
    enabled: Option<Enabled>,
    /// Models that came to or left the device since the last selection, with whether each is
    /// there now and `None` for one that will never be.
    models: Vec<(ModelRef, Option<bool>)>,
    /// Whether anything was told since the thread last took what was.
    changed: bool,
    /// How many times something was told.
    times: u64,
    stopped: bool,
}

struct Shared {
    told: Mutex<Told>,
    woken: Condvar,
    latest: Mutex<Arc<Selected>>,
    /// The maps the thread took in since its last whole selection.
    unproven: Arc<Mutex<Vec<u32>>>,
    /// Up to how many tellings the thread has taken and, where they called for one, selected after.
    done: std::sync::atomic::AtomicU64,
}

/// The world as a thread of its own streams and selects it, so that a frame only reads what was
/// selected last and never waits for the selection.
pub(super) struct World {
    shared: Arc<Shared>,
    thread: std::thread::JoinHandle<()>,
}

impl World {
    /// The world of `index` about a camera at `eye`, its distances `scale`d, that never takes in the maps `skipped`.
    pub(super) fn new(
        index: Arc<WorldIndex>,
        eye: Vec3,
        scale: f32,
        skipped: HashSet<u32>,
    ) -> Self {
        let shared = Arc::new(Shared {
            told: Mutex::new(Told {
                eye,
                scale,
                hour: NOON,
                fade: [FADE; Level::COUNT],
                ..Told::default()
            }),
            woken: Condvar::new(),
            latest: Mutex::default(),
            unproven: Arc::default(),
            done: Default::default(),
        });
        let theirs = Arc::clone(&shared);
        let thread = std::thread::spawn(move || run(index, skipped, &theirs));
        Self { shared, thread }
    }

    /// The maps the thread took in since its last whole selection: for a thread that died, what it is taken to have died on.
    pub(super) fn unproven(&self) -> Vec<u32> {
        (self
            .shared
            .unproven
            .lock()
            .unwrap_or_else(|p| p.into_inner()))
        .clone()
    }

    /// Say the thread took `map` in, as the thread does before it takes one.
    #[cfg(test)]
    pub(super) fn took(&self, map: u32) {
        (self
            .shared
            .unproven
            .lock()
            .unwrap_or_else(|p| p.into_inner()))
        .push(map);
    }

    /// Whether the thread ended though it was not stopped: a panic in a build that unwinds.
    pub(super) fn died(&self) -> bool {
        self.thread.is_finished()
    }

    /// End the thread as a panic on it would.
    #[cfg(test)]
    pub(super) fn end(&self) {
        (self.shared.told.lock().unwrap_or_else(|p| p.into_inner())).stopped = true;
        self.shared.woken.notify_one();
    }

    /// Tell the thread what `tell` changes, and wake it where that is a change.
    fn tell_of(&self, tell: impl FnOnce(&mut Told) -> bool) {
        let mut told = self.shared.told.lock().unwrap_or_else(|p| p.into_inner());
        let changed = tell(&mut told);
        told.changed |= changed;
        told.times += u64::from(changed);
        drop(told);
        if changed {
            self.shared.woken.notify_one();
        }
    }

    /// Say where the camera is and the scale of its distances.
    pub(super) fn look(&self, eye: Vec3, scale: f32) {
        self.tell_of(|told| {
            let changed = (told.eye, told.scale) != (eye, scale);
            (told.eye, told.scale) = (eye, scale);
            changed
        });
    }

    /// Say the hour of the day, 0.0 to 24.0: a time archetype draws inside its hours alone.
    pub(super) fn set_hour(&self, hour: f32) {
        self.tell_of(|told| {
            let changed = hour_of(told.hour) != hour_of(hour);
            told.hour = hour;
            changed
        });
    }

    /// Say the band past its distance each level fades out over, by `Level as usize`, in metres:
    /// [`fade_of`] the install's visual settings.
    pub(super) fn set_fade(&self, fade: [f32; Level::COUNT]) {
        self.tell_of(|told| {
            let changed = told.fade != fade;
            told.fade = fade;
            changed
        });
    }

    /// Say what of the game is switched on: a map it does not allow is not streamed in.
    pub(super) fn set_enabled(&self, enabled: Enabled) {
        self.tell_of(|told| {
            told.enabled = Some(enabled);
            true
        });
    }

    /// Say which models came to the device, will never come, or left it.
    pub(super) fn tell(&self, models: &mut Vec<(ModelRef, Option<bool>)>) {
        if models.is_empty() {
            return;
        }
        self.tell_of(|told| {
            told.models.append(models);
            true
        });
    }

    /// Whether something was told the thread that the last selection does not stand on yet.
    pub(super) fn unselected(&self) -> bool {
        let times = (self.shared.told.lock().unwrap_or_else(|p| p.into_inner())).times;
        self.shared.done.load(std::sync::atomic::Ordering::Acquire) != times
    }

    /// What was selected last.
    pub(super) fn latest(&self) -> Arc<Selected> {
        Arc::clone(&self.shared.latest.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl Drop for World {
    fn drop(&mut self) {
        (self.shared.told.lock().unwrap_or_else(|p| p.into_inner())).stopped = true;
        self.shared.woken.notify_one();
    }
}

/// The world's thread: stream the maps around the camera, and select again whenever the camera
/// moved, a map came or went, a model did, the hour, the fade bands or what is switched on
/// changed, or a fade in time is under way.
fn run(index: Arc<WorldIndex>, skipped: HashSet<u32>, shared: &Shared) {
    let mut streamer = Streamer::skipping(index, skipped, Arc::clone(&shared.unproven));
    let mut ready: HashMap<ModelRef, Option<bool>> = HashMap::new();
    let mut selected_at: Option<(Vec3, f32, [f32; Level::COUNT])> = None;
    let mut moved_at: Option<(Vec3, Instant)> = None;
    let (mut velocity, mut fading, mut version) = (Vec3::ZERO, false, 0);
    loop {
        let (eye, scale, hour, fade, enabled, models, times) = {
            let mut told = shared.told.lock().unwrap_or_else(|p| p.into_inner());
            // What was told while the last selection was made is taken at once.
            if !told.changed && !told.stopped {
                (told, _) =
                    (shared.woken.wait_timeout(told, TICK)).unwrap_or_else(|p| p.into_inner());
            }
            if told.stopped {
                return;
            }
            told.changed = false;
            let (enabled, models) = (told.enabled.take(), std::mem::take(&mut told.models));
            (
                told.eye, told.scale, told.hour, told.fade, enabled, models, told.times,
            )
        };
        velocity = velocity_at(&mut moved_at, velocity, eye, Instant::now());
        let mut stale = !models.is_empty() || fading;
        for (model, state) in models {
            match state {
                // A model that left the device is waited for again where it is wanted.
                Some(false) => ready.remove(&model),
                state => ready.insert(model, state),
            };
        }
        stale |= enabled.is_some_and(|enabled| streamer.set_enabled(enabled));
        stale |= streamer.set_hour(hour);
        let mut view = View::at(eye, scale);
        view.velocity = velocity;
        view.fade = fade;
        stale |= streamer.stream(&view);
        stale |= selected_at.is_none_or(|(at, by, faded)| {
            at.distance(eye) >= RESELECT || by != scale || faded != fade
        });
        if stale {
            let state = |model: ModelRef| ready.get(&model).copied().unwrap_or(Some(false));
            let mut selected = streamer.select(&view, &state);
            version += 1;
            selected.version = version;
            fading = selected.fading;
            selected_at = Some((eye, scale, fade));
            *shared.latest.lock().unwrap_or_else(|p| p.into_inner()) = Arc::new(selected);
            // What was taken in has been through a whole selection.
            (shared.unproven.lock().unwrap_or_else(|p| p.into_inner())).clear();
        }
        shared
            .done
            .store(times, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
