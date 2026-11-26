//! The level-of-detail forest of a mounted GTA V install: the maps a view streams in, read from the
//! mount as their boxes are wanted and held under a budget of bytes, and what the rules of
//! [`or_world::streaming`] select of them, as instances and the files each needs.
//!
//! ⚠ A map's parent is wanted with it whatever switches the parent, and a parent the view does
//! not hold yet leaves its children standing alone, each by its own distance.
//!
//! ⚠ An interior's entities are no part of the map that places it. They are read from the type
//! file that declares the interior while a view stands within the instance's distance, and each
//! then draws by its own place and distance.

mod read;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{Affine3A, Vec3};
use or_world::state::Gate;
use or_world::streaming::{Forest, Level, MapBox, NodeId, NodeState, View};
use or_world::WorldState;

use crate::world_index::{tree, Map, WorldIndex, WorldSource};
use read::{Placed, ReadMap, What};

/// The bit of a forest key that says it names an interior's entities, not a map's.
const INTERIOR: u64 = 1 << 63;

/// The most entities a map holds for one of them to be an interior's instance the forest can key.
const MOST_KEYED: u32 = 1 << 31;

/// The forest key of the entities of the interior that entity `at` of the map `map` places.
pub fn interior_key(map: u32, at: u32) -> u64 {
    INTERIOR | u64::from(map) << 31 | u64::from(at % MOST_KEYED)
}

/// How far past its box a GTA V map streams in, in metres.
pub const PRESTREAM: f32 = 50.0;

/// Whether a camera at `eye` streams in a GTA V map of `boxes`: within its box grown by [`PRESTREAM`].
// ⚠ The box is the entities' where the streaming one does not hold them. GTA V's rule, kept
// apart from the GTA VI build's sphere that `MapBox::wants` answers by.
pub fn streams_in(boxes: &MapBox, eye: Vec3) -> bool {
    let (s, e) = (boxes.streaming, boxes.entities);
    let holds = s.0.cmple(e.0).all() && s.1.cmpge(e.1).all();
    let (min, max) = if holds { s } else { e };
    let grow = Vec3::splat(PRESTREAM);
    eye.cmpge(min - grow).all() && eye.cmple(max + grow).all()
}

/// The two boxes `map` states, as the streaming rules take them; `None` where it states neither.
pub fn map_box(map: &Map) -> Option<MapBox> {
    let streaming = map.streaming.or(map.entities)?;
    Some(MapBox {
        streaming,
        entities: map.entities.unwrap_or(streaming),
    })
}

/// What asks for a map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request<'a> {
    /// Nothing holds it back: it streams wherever its box is wanted.
    Always,
    /// A manifest's group of its name, by the hour, the weather or both.
    Group(&'a Gate),
    /// It is held back and no group's gate lets it in: only a request by name does.
    Script,
}

/// What asks for `map`, by its own flags and the groups of the mount's manifests.
pub fn request_of<'a>(index: &'a WorldIndex, map: &Map) -> Request<'a> {
    let groups = index.state_groups();
    match groups.gate(map.hash) {
        _ if !groups.is_held_back(map.hash) => Request::Always,
        Some(gate) if *gate != Gate::Scripted => Request::Group(gate),
        _ => Request::Script,
    }
}

/// Why a file gave the forest nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unread {
    #[error("the archive that lists it gave no file")]
    Absent,
    #[error("it does not read as a map")]
    NotAMap,
    #[error("one of its entities did not read, so a place among them names no one")]
    EntityUnread,
    #[error("no type file of the mount declares the interior")]
    Undeclared,
    #[error("the file that declares the interior does not read as a type file")]
    NotATypeFile,
    #[error("the archetype the instance names is no interior")]
    NotAnInterior,
    #[error("the instance stands past the last entity a forest keys an interior by")]
    NotKeyed,
}

/// The files that draw an archetype, each by the hash of its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelFiles {
    /// A drawable or a fragment of this name, or the entry of this name in [`Self::dictionary`].
    pub drawable: u32,
    /// The drawable dictionary that holds it, where its archetype states one.
    pub dictionary: Option<u32>,
    /// Its texture dictionary, where its archetype states one.
    pub textures: Option<u32>,
}

/// Why an entity stands as a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, thiserror::Error)]
pub enum NoModel {
    #[error("the entity names no archetype")]
    Unnamed,
    #[error("no type file of the mount declares its archetype")]
    Undeclared,
}

/// What draws an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Files(ModelFiles),
    /// Nothing says what draws it: a box stands in its place, and why.
    Box(NoModel),
}

/// What draws the archetype named `archetype`; `None` where its archetype names no asset.
pub fn model_of(index: &WorldIndex, archetype: u32) -> Option<Model> {
    match read::what_of(index, archetype, false) {
        What::Model(files) => Some(Model::Files(files)),
        What::Box(why) => Some(Model::Box(why)),
        What::Interior | What::Nothing => None,
    }
}

/// Why a map is in the forest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The view is within the box it streams by, and what asks for it holds.
    Own,
    /// It is the parent of this map, which is wanted.
    ParentOf(u32),
}

/// One map a view wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wanted<'a> {
    pub map: &'a Map,
    pub why: Why,
}

/// An interior whose entities a view brought in or could not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteriorAt {
    /// The map that places it, by its hash.
    pub map: u32,
    /// Its instance's place among that map's entities.
    pub entity: u32,
    pub archetype: u32,
}

/// What streaming for a view came to, each list in the mount's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Streamed<'a> {
    /// The maps in the forest.
    pub wanted: Vec<Wanted<'a>>,
    /// Those of them read from the mount for this view, the nearest first.
    pub read: Vec<&'a Map>,
    /// The maps wanted that a later call reads.
    pub waiting: Vec<&'a Map>,
    /// The maps wanted that gave no entities, each with why.
    pub unread: Vec<(&'a Map, Unread)>,
    /// The interiors within their distance whose entities stand in the forest.
    pub interiors: Vec<InteriorAt>,
    /// The interiors within their distance that gave no entities, each with why.
    pub interiors_unread: Vec<(InteriorAt, Unread)>,
}

/// One thing a selection draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub node: NodeId,
    /// The map that places it, by its hash; an interior's entity is placed by the map of the interior's instance.
    pub map: u32,
    /// Its place among the map's entities, or among its interior's.
    pub entity: u32,
    /// The entity of [`Self::map`] that is the instance of its interior, for an interior's entity.
    pub interior: Option<u32>,
    pub archetype: u32,
    /// From the model's own space to the world.
    pub to_world: Affine3A,
    pub level: Level,
    /// How opaque, above 0 and to 1.
    pub fade: f32,
    /// It fades out for its children, as [`or_world::streaming::NodeDraw::yields`].
    pub yields: bool,
    pub model: Model,
}

/// The files a selection needs, each kind by the hashes of its names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Needed {
    /// Drawables and fragments that are files of their own.
    pub drawables: BTreeSet<u32>,
    pub drawable_dictionaries: BTreeSet<u32>,
    pub texture_dictionaries: BTreeSet<u32>,
}

impl Needed {
    /// Take the files of one model.
    pub fn add(&mut self, files: &ModelFiles) {
        match files.dictionary {
            Some(dictionary) => self.drawable_dictionaries.insert(dictionary),
            None => self.drawables.insert(files.drawable),
        };
        self.texture_dictionaries.extend(files.textures);
    }
}

/// What a selection draws and what it waits for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selected {
    /// By the forest's key of its map or interior, then its place there.
    pub instances: Vec<Instance>,
    /// The archetypes in range whose model is loading, each once, nearest ancestors first.
    pub waiting: Vec<u32>,
    /// Whether a fade in time is under way: the next selection differs though nothing moves.
    pub fading: bool,
}

impl Selected {
    /// How many instances draw at each level, by `Level as usize`.
    pub fn by_level(&self) -> [usize; Level::COUNT] {
        let mut count = [0; Level::COUNT];
        for instance in &self.instances {
            count[instance.level as usize] += 1;
        }
        count
    }

    /// The files of what draws.
    pub fn files(&self) -> Needed {
        let mut needed = Needed::default();
        for instance in &self.instances {
            if let Model::Files(files) = &instance.model {
                needed.add(files);
            }
        }
        needed
    }
}

struct Held {
    read: ReadMap,
    bytes: u64,
    /// The last view that wanted the map, by its number.
    used: u64,
}

/// The forest of a mount's maps near the views asked about.
pub struct WorldForest<'a> {
    index: &'a WorldIndex,
    source: &'a dyn WorldSource,
    /// Bytes of the maps read and held, in the forest or let go of.
    budget: u64,
    forest: Forest,
    held: HashMap<u32, Held>,
    held_bytes: u64,
    /// The maps in the forest, by hash.
    streamed: BTreeSet<u32>,
    /// The interiors whose entities are in the forest, by their forest key: what each node places, the nodes being the forest's.
    interiors: BTreeMap<u64, (InteriorAt, ReadMap)>,
    /// ⚠ Kept for the mount's life, so a file that does not read is opened once and not at every view.
    refused: HashMap<u32, Unread>,
    refused_interiors: HashMap<u32, Unread>,
    /// The maps the tree of streaming extents cannot answer for, by their place in the index: each is asked at every view.
    loose: Vec<u32>,
    asked: u64,
}

impl<'a> WorldForest<'a> {
    /// No map yet, over `index` and the mount it was built from, holding at most `budget` bytes of maps read, those in the forest among them, unless one view's own maps pass it.
    pub fn new(index: &'a WorldIndex, source: &'a dyn WorldSource, budget: u64) -> Self {
        // ⚠ The tree is of the streaming extents, and a map streams by its entities' extents
        // where the streaming ones do not hold them.
        let in_tree = |map: &Map| {
            let streaming = map
                .streaming
                .filter(|(min, max)| tree::is_a_box(*min, *max));
            match (streaming, map.entities) {
                (Some(s), Some(e)) => s.0.cmple(e.0).all() && s.1.cmpge(e.1).all(),
                (streaming, _) => streaming.is_some(),
            }
        };
        let loose = (index.maps().iter().enumerate())
            .filter(|(_, map)| map_box(map).is_some() && !in_tree(map))
            .map(|(at, _)| at as u32)
            .collect();
        Self {
            index,
            source,
            budget,
            forest: Forest::new(),
            held: HashMap::new(),
            held_bytes: 0,
            streamed: BTreeSet::new(),
            interiors: BTreeMap::new(),
            refused: HashMap::new(),
            refused_interiors: HashMap::new(),
            loose,
            asked: 0,
        }
    }

    /// The maps `view` wants under `state`, by their place in the index, each with why and how far its entities stand from the eye: those whose box the view is within and that something asks for, and their parents.
    fn wanted(&self, view: &View, state: &WorldState) -> BTreeMap<u32, (&'a Map, Why, f32)> {
        let index = self.index;
        let groups = index.state_groups();
        // ⚠ A box is grown along each axis and the tree asks by a circle: the circle about the
        // grown box's corner takes every box the rules would. A tier that streams from farther
        // than `PRESTREAM` needs its own radius here.
        let reach = PRESTREAM * std::f32::consts::SQRT_2;
        let near = index.maps_near_column(view.eye.x, view.eye.y, reach);
        let loose = (self.loose.iter()).map(|at| &index.maps()[*at as usize]);
        let mut wanted = BTreeMap::new();
        for map in near.into_iter().chain(loose) {
            let Some(boxes) = map_box(map) else {
                continue;
            };
            let asked = streams_in(&boxes, view.eye) && groups.is_active(map.hash, state);
            if let Some(at) = index.place_of_map(map.hash).filter(|_| map.read && asked) {
                let (min, max) = boxes.entities;
                let away = view.eye.distance(view.eye.clamp(min, max));
                wanted.insert(at, (map, Why::Own, away));
            }
        }
        let own: Vec<(&Map, f32)> = (wanted.values())
            .map(|(map, _, away)| (*map, *away))
            .collect();
        for (map, away) in own {
            let mut child = map;
            // Each step takes a map not yet wanted, so parents that name each other end it.
            while let Some((at, parent)) = (index.parent_of(child))
                .and_then(|parent| Some((index.place_of_map(parent.hash)?, parent)))
                .filter(|(at, _)| !wanted.contains_key(at))
            {
                wanted.insert(at, (parent, Why::ParentOf(child.hash), away));
                child = parent;
            }
        }
        wanted
    }

    /// Reads the maps `view` wants under `state` that are not held, puts them in the forest in place of those it left, and brings in the interiors within their distance.
    pub fn stream(&mut self, view: &View, state: &WorldState) -> Streamed<'a> {
        self.stream_some(view, state, usize::MAX)
    }

    /// [`Self::stream`], reading at most `most` maps, the nearest first: the rest wait for a later call.
    pub fn stream_some(&mut self, view: &View, state: &WorldState, most: usize) -> Streamed<'a> {
        self.asked += 1;
        let wanted = self.wanted(view, state);
        let mut out = Streamed::default();
        // ⚠ Before any map is read: a map of this view that is held must not be the one dropped
        // to make room for another of the same view.
        for (map, ..) in wanted.values() {
            if let Some(held) = self.held.get_mut(&map.hash) {
                held.used = self.asked;
            }
        }
        let mut ask: Vec<(f32, u32, &'a Map)> = (wanted.iter())
            .filter(|(_, (map, ..))| {
                !self.held.contains_key(&map.hash) && !self.refused.contains_key(&map.hash)
            })
            .map(|(at, (map, _, away))| (*away, *at, *map))
            .collect();
        ask.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, _, map) in ask.into_iter().take(most) {
            match read::map(self.index, self.source, map) {
                Ok(read) => {
                    let (bytes, used) = (read.bytes(), self.asked);
                    self.held_bytes += bytes;
                    self.held.insert(map.hash, Held { read, bytes, used });
                    out.read.push(map);
                }
                Err(refused) => {
                    tracing::warn!(file = %map.logical, why = %refused, "a map gave the forest nothing");
                    self.refused.insert(map.hash, refused);
                }
            }
        }
        let mut now = BTreeSet::new();
        for (map, why, _) in wanted.into_values() {
            if let Some(refused) = self.refused.get(&map.hash) {
                out.unread.push((map, *refused));
                continue;
            }
            let Some(held) = self.held.get(&map.hash) else {
                out.waiting.push(map);
                continue;
            };
            if self.streamed.insert(map.hash) {
                (self.forest).insert(u64::from(map.hash), held.read.nodes.clone());
            }
            now.insert(map.hash);
            out.wanted.push(Wanted { map, why });
        }
        let left: Vec<u32> = self.streamed.difference(&now).copied().collect();
        for map in left {
            self.streamed.remove(&map);
            self.forest.remove(u64::from(map));
        }
        self.bring_interiors(view, &mut out);
        self.shed();
        out
    }

    /// Puts in the forest the entities of each interior of a streamed map that `view` is within the distance of, and takes out those of the others.
    fn bring_interiors(&mut self, view: &View, out: &mut Streamed<'a>) {
        let mut near: Vec<(InteriorAt, Affine3A, f32)> = Vec::new();
        for map in &self.streamed {
            let Some(held) = self.held.get(map) else {
                continue;
            };
            for &entity in &held.read.interiors {
                let (node, placed) = (
                    &held.read.nodes[entity as usize],
                    &held.read.placed[entity as usize],
                );
                // As far as the instance itself would draw at all: its distance and its band.
                let reach = node.scaled(node.lod_dist, view) + view.fade[node.level as usize];
                if view.eye.distance(node.position) <= reach {
                    let at = InteriorAt {
                        map: *map,
                        entity,
                        archetype: placed.archetype,
                    };
                    near.push((at, placed.to_world, reach));
                }
            }
        }
        let now: BTreeSet<u64> = (near.iter())
            .map(|(at, ..)| interior_key(at.map, at.entity))
            .collect();
        let left: Vec<u64> = (self.interiors.keys().copied())
            .filter(|key| !now.contains(key))
            .collect();
        for key in left {
            self.interiors.remove(&key);
            self.forest.remove(key);
        }
        for (at, to_world, reach) in near {
            let key = interior_key(at.map, at.entity);
            if self.interiors.contains_key(&key) {
                out.interiors.push(at);
                continue;
            }
            let read = match self.refused_interiors.get(&at.archetype).copied() {
                Some(refused) => Err(refused),
                None if at.entity >= MOST_KEYED => Err(Unread::NotKeyed),
                None => read::interior(
                    self.index,
                    self.source,
                    at.archetype,
                    (at.map, at.entity),
                    (&to_world, reach),
                )
                .inspect_err(|refused| {
                    tracing::warn!(archetype = at.archetype, why = %refused, "an interior gave the forest nothing");
                    self.refused_interiors.insert(at.archetype, *refused);
                }),
            };
            match read {
                Ok(mut read) => {
                    self.forest.insert(key, std::mem::take(&mut read.nodes));
                    self.interiors.insert(key, (at, read));
                    out.interiors.push(at);
                }
                Err(refused) => out.interiors_unread.push((at, refused)),
            }
        }
    }

    /// Drops the map wanted longest ago until the budget holds.
    ///
    /// ⚠ Never a map in the forest: a view whose maps pass the budget keeps them all until the next.
    fn shed(&mut self) {
        while self.held_bytes > self.budget {
            let oldest = (self.held.iter())
                .filter(|(map, _)| !self.streamed.contains(map))
                .min_by_key(|(map, held)| (held.used, **map))
                .map(|(map, _)| *map);
            let Some(map) = oldest else { break };
            if let Some(held) = self.held.remove(&map) {
                self.held_bytes -= held.bytes;
            }
        }
    }

    /// What the forest draws for `view`; `ready` says where the model of an archetype stands, by the archetype's name.
    pub fn select(&mut self, view: &View, ready: &dyn Fn(u32) -> NodeState) -> Selected {
        let (held, interiors) = (&self.held, &self.interiors);
        let placed = |id: NodeId| -> Option<&Placed> {
            let read = match id.0 & INTERIOR {
                0 => &held.get(&u32::try_from(id.0).ok()?)?.read,
                _ => &interiors.get(&id.0)?.1,
            };
            read.placed.get(id.1 as usize)
        };
        // A box waits for nothing, and an interior's instance has no model of its own.
        let state = |id: NodeId| match placed(id) {
            Some(placed) => match placed.what {
                What::Model(_) => ready(placed.archetype),
                What::Box(_) => NodeState::Ready,
                What::Interior | What::Nothing => NodeState::Absent,
            },
            None => NodeState::Absent,
        };
        let mut selection = self.forest.select(view, &state);
        selection.drawn.sort_unstable_by_key(|draw| draw.node);
        let mut out = Selected {
            fading: selection.fading,
            ..Selected::default()
        };
        for draw in &selection.drawn {
            let id = draw.node;
            let (Some(placed), Some(node)) = (placed(id), self.forest.node(id)) else {
                continue;
            };
            let model = match placed.what {
                What::Model(files) => Model::Files(files),
                What::Box(why) => Model::Box(why),
                What::Interior | What::Nothing => continue,
            };
            let within = interiors.get(&id.0).map(|(at, _)| *at);
            out.instances.push(Instance {
                node: id,
                map: within.map_or(id.0 as u32, |at| at.map),
                entity: id.1,
                interior: within.map(|at| at.entity),
                archetype: placed.archetype,
                to_world: placed.to_world,
                level: node.level,
                fade: draw.fade,
                yields: draw.yields,
                model,
            });
        }
        let archetypes = |nodes: &[NodeId]| {
            let mut seen = BTreeSet::new();
            let named = nodes.iter().filter_map(|id| placed(*id));
            let named = named.map(|placed| placed.archetype);
            named.filter(|name| seen.insert(*name)).collect()
        };
        out.waiting = archetypes(&selection.wanted);
        out
    }

    /// The node `id` names, where its map or its interior is in the forest.
    pub fn node(&self, id: NodeId) -> Option<&or_world::streaming::Node> {
        self.forest.node(id)
    }

    /// How many maps and interiors are in the forest, and how many nodes.
    pub fn len(&self) -> (usize, usize) {
        self.forest.len()
    }

    pub fn is_empty(&self) -> bool {
        self.forest.is_empty()
    }

    /// How many maps are held, in the forest or let go of.
    pub fn held_maps(&self) -> usize {
        self.held.len()
    }

    /// The bytes of the maps held.
    pub fn held_bytes(&self) -> u64 {
        self.held_bytes
    }

    /// The entities of the maps in the forest that state no level, and those that name a parent in a parent map the mount lacks.
    pub fn misfits(&self) -> (u64, u64) {
        let held = (self.streamed.iter()).filter_map(|map| self.held.get(map));
        held.fold((0, 0), |(unleveled, lost), held| {
            (
                unleveled + u64::from(held.read.unleveled),
                lost + u64::from(held.read.links_lost),
            )
        })
    }
}
