//! The maps streamed in around the camera and what of them draws: each wanted map's body is read
//! on a worker into the nodes of the level-of-detail forest, and a selection gives each entity and
//! each instance that draws with the model that draws it and where it stands.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::{Arc, Condvar, Mutex};

use glam::{Mat4, Quat, Vec3};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::map::stream::{StreamLink, StreamMap};
use or_world::streaming::{Extent, Forest, Node, NodeId, NodeState, ParentRef};

use super::index::{ModelRef, WorldIndex};
use super::pool::Pool;

/// The bytes a map body is read to.
const BODY_LIMIT: u64 = 256 << 20;
/// How far the camera moves before the wanted maps are worked out again, in metres.
const RESTREAM: f32 = 4.0;

/// The rows of a 3×4 matrix: the model's axes, with the translation in the fourth column.
pub type Rows = [[f32; 4]; 3];

fn rows_of(m: Mat4) -> Rows {
    let c = m.to_cols_array_2d();
    std::array::from_fn(|row| [c[0][row], c[1][row], c[2][row], c[3][row]])
}

fn place(rows: &Rows, p: Vec3) -> Vec3 {
    let at = |r: [f32; 4]| r[0] * p.x + r[1] * p.y + r[2] * p.z + r[3];
    Vec3::new(at(rows[0]), at(rows[1]), at(rows[2]))
}

/// The length of the longest axis of `rows`, which a sphere's radius grows by.
fn stretch(rows: &Rows) -> f32 {
    (0..3)
        .map(|axis| Vec3::new(rows[0][axis], rows[1][axis], rows[2][axis]).length())
        .fold(0.0, f32::max)
}

/// What draws at a node: its model, and the sphere of that model in its own space.
#[derive(Clone, Copy)]
struct Drawn {
    /// `None` where no archetype or no body of the index gives one: the node draws nothing.
    model: Option<ModelRef>,
    centre: Vec3,
    radius: f32,
}

/// A map streamed in.
struct Loaded {
    /// One for each node: the entities, then the prop lists.
    drawn: Vec<Drawn>,
    /// Where each entity stands.
    entities: Vec<Rows>,
    /// Where each instance of each prop list stands.
    lists: Vec<Vec<Rows>>,
}

/// What a worker read of a map: its nodes for the forest, and what draws at each.
pub(super) struct ReadMap {
    nodes: Vec<Node>,
    loaded: Loaded,
}

fn link(map: &StreamMap, own: u32, link: Option<StreamLink>) -> (Option<ParentRef>, Option<u32>) {
    let Some(link) = link else {
        return (None, None);
    };
    let name = match link.map {
        Some(selector) => map.lod_parents.get(usize::from(selector)).copied(),
        None => Some(own),
    };
    let parent = name.map(|name| ParentRef {
        map: u64::from(name),
        node: u32::from(link.index),
    });
    (parent, link.instance.map(u32::from))
}

/// The nodes of `map` and what draws at each, by the archetypes and models of `index`.
pub(super) fn read_map(index: &WorldIndex, map: &StreamMap) -> ReadMap {
    let count = map.entities.len() + map.prop_lists.len();
    let (mut nodes, mut drawn) = (Vec::with_capacity(count), Vec::with_capacity(count));
    let of = |word: u64| {
        let archetype = index.archetype(word);
        let model = archetype.and_then(|a| index.model(a));
        let (centre, radius, lod_dist) = archetype.map_or((Vec3::ZERO, 0.0, 0.0), |a| {
            (Vec3::from(a.centre), a.radius, a.lod_dist)
        });
        (
            Drawn {
                model,
                centre,
                radius,
            },
            lod_dist,
        )
    };
    let mut entities = Vec::with_capacity(map.entities.len());
    for e in &map.entities {
        let (what, archetype_dist) = of(e.archetype_word);
        let (parent, instance) = link(map, map.name, e.parent);
        nodes.push(Node {
            position: Vec3::from(e.position),
            extent: Extent::Point,
            // An entity that states no distance takes its archetype's.
            lod_dist: if e.lod_dist > 0.0 {
                e.lod_dist
            } else {
                archetype_dist
            },
            child_lod_dist: e.child_lod_dist,
            parent,
            instance,
        });
        // ⚠ An interior's contents stand in its own space, which nothing here places: it draws nothing.
        drawn.push(Drawn {
            model: what.model.filter(|_| !e.interior),
            ..what
        });
        entities.push(rows_of(Mat4::from_scale_rotation_translation(
            Vec3::new(e.scale_xy, e.scale_xy, e.scale_z),
            Quat::from_array(e.rotation).normalize(),
            Vec3::from(e.position),
        )));
    }
    let mut lists = Vec::with_capacity(map.prop_lists.len());
    for list in &map.prop_lists {
        let (what, archetype_dist) = of(list.archetype_word);
        let (parent, _) = link(map, map.name, list.parent);
        let (min, max) = (
            Vec3::from(list.batch_aabb.min),
            Vec3::from(list.batch_aabb.max),
        );
        nodes.push(Node {
            position: (min + max) / 2.0,
            extent: Extent::Box(min, max),
            lod_dist: if list.lod_dist > 0.0 {
                list.lod_dist
            } else {
                archetype_dist
            },
            child_lod_dist: list.child_lod_dist,
            parent,
            instance: None,
        });
        drawn.push(what);
        // A list states each instance as its three axes, each with one part of the translation.
        let rows = |a: &[[f32; 4]; 3]| -> Rows {
            std::array::from_fn(|row| [a[0][row], a[1][row], a[2][row], a[row][3]])
        };
        lists.push(list.instances.iter().map(rows).collect());
    }
    ReadMap {
        nodes,
        loaded: Loaded {
            drawn,
            entities,
            lists,
        },
    }
}

/// One thing a selection draws: its model, where it stands, and its sphere in the world.
#[derive(Clone, Copy)]
pub struct Instance {
    /// Which of the selection's models draws it.
    pub model: u32,
    pub rows: Rows,
    pub centre: Vec3,
    pub radius: f32,
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
    pub models: Vec<ModelRef>,
    pub chunks: Vec<Chunk>,
    pub instances: Vec<Instance>,
    /// Each model waited for, with how far the nearest thing that waits for it is.
    pub wanted: Vec<(f32, ModelRef)>,
    pub streaming: Streaming,
}

/// The maps around the camera.
pub(super) struct Streamer {
    index: Arc<WorldIndex>,
    forest: Forest,
    loaded: HashMap<u32, Loaded>,
    /// The maps the camera wants, by name, with their distance; their parents among them.
    wanted: HashMap<u32, f32>,
    /// A map that did not read is not asked for again.
    failed: HashSet<u32>,
    streamed_at: Option<Vec3>,
    readers: Pool<u32, Option<ReadMap>>,
    pub(super) streaming: Streaming,
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

impl Streamer {
    pub(super) fn new(index: Arc<WorldIndex>) -> Self {
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
                Some(read_map(&shared, &map))
            },
        );
        Self {
            index,
            forest: Forest::new(),
            loaded: HashMap::new(),
            wanted: HashMap::new(),
            failed: HashSet::new(),
            streamed_at: None,
            readers,
            streaming: Streaming::default(),
        }
    }

    /// Work out the maps a camera at `eye` wants, ask for those not loaded and let go of the
    /// rest; whether a map came or went.
    pub(super) fn stream(&mut self, eye: Vec3) -> bool {
        let mut changed = false;
        for (name, read) in self.readers.take(|| true) {
            match read {
                Some(read) => changed |= self.insert(name, read),
                None => {
                    self.failed.insert(name);
                }
            }
        }
        let moved = self
            .streamed_at
            .is_none_or(|at| at.distance(eye) >= RESTREAM);
        if moved {
            self.streamed_at = Some(eye);
            self.rewant(eye);
        }
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
        // A map whose child map is wanted and not loaded yet holds its nodes in place.
        let mut held: HashSet<u32> = HashSet::new();
        for (_, name) in &asked {
            if let Some(at) = self.index.map_named(*name) {
                held.extend(self.index.parents_of(&self.index.maps[at]));
            }
        }
        for name in self.loaded.keys() {
            (self.forest).set_complete(u64::from(*name), !held.contains(name));
        }
        self.streaming.maps_wanted = self.wanted.len();
        self.streaming.maps_loaded = self.loaded.len();
        self.streaming.maps_waiting = asked.len();
        self.streaming.nodes = self.forest.len().1;
        self.readers.want(asked);
        changed
    }

    // ponytail: every map's box is tested each time the camera has moved; a grid of the boxes if the walk shows.
    fn rewant(&mut self, eye: Vec3) {
        self.wanted.clear();
        let mut parents = Vec::new();
        for map in self.index.maps.iter().filter(|m| m.held == 0) {
            let boxes = map.boxes();
            if !boxes.wants(eye) {
                continue;
            }
            let (min, max) = boxes.entities;
            let away = eye.distance(eye.clamp(min, max));
            self.wanted.insert(map.name, away);
            parents.extend(self.index.parents_of(map).iter().map(|p| (*p, away)));
        }
        // A wanted map's parents are wanted with it, and theirs.
        while let Some((name, away)) = parents.pop() {
            let Some(at) = self.index.map_named(name) else {
                continue;
            };
            let map = &self.index.maps[at];
            // A parent the mount holds back is no part of the world at rest.
            if matches!(map.held, 1 | 2) || self.wanted.contains_key(&name) {
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
        self.forest.insert(u64::from(name), read.nodes);
        self.loaded.insert(name, read.loaded);
        true
    }

    /// What draws for a camera at `eye` with distances scaled by `scale`; `ready` says whether a
    /// model is on the device, and `None` that it never will be.
    pub(super) fn select(
        &mut self,
        eye: Vec3,
        scale: f32,
        ready: &dyn Fn(ModelRef) -> Option<bool>,
    ) -> Selected {
        let loaded = &self.loaded;
        let drawn_of = |id: NodeId| {
            let map = loaded.get(&(id.0 as u32))?;
            Some((map, map.drawn.get(id.1 as usize)?))
        };
        let model_of = |id: NodeId| drawn_of(id).and_then(|(_, drawn)| drawn.model);
        let state = |id: NodeId| match model_of(id).map(ready) {
            None | Some(None) => NodeState::Absent,
            Some(Some(true)) => NodeState::Ready,
            Some(Some(false)) => NodeState::Loading,
        };
        let mut selection = self.forest.select(eye, scale, &state);
        let mut out = Selected::default();
        let mut models: HashMap<ModelRef, u32> = HashMap::new();
        let mut model_ix = |model: ModelRef, out: &mut Selected| {
            *models.entry(model).or_insert_with(|| {
                out.models.push(model);
                out.models.len() as u32 - 1
            })
        };
        let instance = |model: u32, rows: &Rows, drawn: &Drawn| Instance {
            model,
            rows: *rows,
            centre: place(rows, drawn.centre),
            radius: drawn.radius * stretch(rows),
        };
        // The entities of one map stand together: a chunk a map.
        selection.drawn.sort_unstable();
        let (mut from, mut of_map) = (0, None);
        for id in &selection.drawn {
            if of_map != Some(id.0) {
                close(&mut out, from);
                (from, of_map) = (out.instances.len(), Some(id.0));
            }
            let Some((map, drawn)) = drawn_of(*id) else {
                continue;
            };
            if let (Some(model), Some(rows)) = (drawn.model, map.entities.get(id.1 as usize)) {
                let model = model_ix(model, &mut out);
                out.instances.push(instance(model, rows, drawn));
            }
        }
        close(&mut out, from);
        // And the instances of one list: a chunk a list.
        for list in &selection.lists {
            let Some((map, drawn)) = drawn_of(list.node) else {
                continue;
            };
            let instances = (list.node.1 as usize)
                .checked_sub(map.entities.len())
                .and_then(|at| map.lists.get(at));
            let (Some(model), Some(instances)) = (drawn.model, instances) else {
                continue;
            };
            let model = model_ix(model, &mut out);
            let skipped = &selection.skipped[list.skipped.clone()];
            let from = out.instances.len();
            for (at, rows) in instances.iter().enumerate() {
                if !skipped.contains(&(at as u32)) {
                    out.instances.push(instance(model, rows, drawn));
                }
            }
            close(&mut out, from);
        }
        // Each model waited for once, by the nearest node that waits for it.
        let mut waited: HashMap<ModelRef, f32> = HashMap::new();
        for id in &selection.wanted {
            let (Some(model), Some(node)) = (model_of(*id), self.forest.node(*id)) else {
                continue;
            };
            let away = eye.distance(node.position);
            let nearest = waited.entry(model).or_insert(away);
            *nearest = nearest.min(away);
        }
        out.wanted = (waited.into_iter().map(|(model, away)| (away, model))).collect();
        self.streaming.drawn = out.instances.len();
        out.streaming = self.streaming;
        out
    }
}

/// How far the camera moves before what draws is selected again, in metres.
const RESELECT: f32 = 0.5;
/// How long the world's thread waits for a change before it looks for map bodies that arrived.
const TICK: std::time::Duration = std::time::Duration::from_millis(20);

/// What the screen tells the world's thread.
#[derive(Default)]
struct Told {
    eye: Vec3,
    scale: f32,
    /// Models that came to or left the device since the last selection, with whether each is
    /// there now and `None` for one that will never be.
    models: Vec<(ModelRef, Option<bool>)>,
    /// Whether anything was told since the thread last took what was.
    changed: bool,
    stopped: bool,
}

struct Shared {
    told: Mutex<Told>,
    woken: Condvar,
    latest: Mutex<Arc<Selected>>,
}

/// The world as a thread of its own streams and selects it, so that a frame only reads what was
/// selected last and never waits for the selection.
pub(super) struct World {
    shared: Arc<Shared>,
}

impl World {
    pub(super) fn new(index: Arc<WorldIndex>, eye: Vec3, scale: f32) -> Self {
        let shared = Arc::new(Shared {
            told: Mutex::new(Told {
                eye,
                scale,
                ..Told::default()
            }),
            woken: Condvar::new(),
            latest: Mutex::default(),
        });
        let theirs = Arc::clone(&shared);
        std::thread::spawn(move || run(index, &theirs));
        Self { shared }
    }

    /// Say where the camera is and the scale of its distances.
    pub(super) fn look(&self, eye: Vec3, scale: f32) {
        let mut told = self.shared.told.lock().unwrap_or_else(|p| p.into_inner());
        told.changed |= (told.eye, told.scale) != (eye, scale);
        (told.eye, told.scale) = (eye, scale);
        drop(told);
        self.shared.woken.notify_one();
    }

    /// Say which models came to the device, will never come, or left it.
    pub(super) fn tell(&self, models: &mut Vec<(ModelRef, Option<bool>)>) {
        if models.is_empty() {
            return;
        }
        let mut told = self.shared.told.lock().unwrap_or_else(|p| p.into_inner());
        told.models.append(models);
        told.changed = true;
        drop(told);
        self.shared.woken.notify_one();
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
/// moved, a map came or went, or a model did.
fn run(index: Arc<WorldIndex>, shared: &Shared) {
    let mut streamer = Streamer::new(index);
    let mut ready: HashMap<ModelRef, Option<bool>> = HashMap::new();
    let mut selected_at: Option<(Vec3, f32)> = None;
    let mut version = 0;
    loop {
        let (eye, scale, models) = {
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
            (told.eye, told.scale, std::mem::take(&mut told.models))
        };
        let mut stale = !models.is_empty();
        for (model, state) in models {
            match state {
                // A model that left the device is waited for again where it is wanted.
                Some(false) => ready.remove(&model),
                state => ready.insert(model, state),
            };
        }
        stale |= streamer.stream(eye);
        stale |= selected_at.is_none_or(|(at, by)| at.distance(eye) >= RESELECT || by != scale);
        if stale {
            let state = |model: ModelRef| ready.get(&model).copied().unwrap_or(Some(false));
            let mut selected = streamer.select(eye, scale, &state);
            version += 1;
            selected.version = version;
            selected_at = Some((eye, scale));
            *shared.latest.lock().unwrap_or_else(|p| p.into_inner()) = Arc::new(selected);
        }
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
