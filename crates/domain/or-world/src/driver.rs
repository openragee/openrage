//! The driver every title's world streams and selects through: a synchronous machine around the
//! forest of [`crate::streaming`]. The maps a view wants go out, their bodies come in, and a
//! selection comes out as instances each body laid out once.
//!
//! ⚠ A map's parents are wanted with it whatever switches them, and a parent the view does not
//! hold yet leaves its children standing alone, each by its own distance.
//!
//! ⚠ An edit session is asked for a map's body before any reader: after an apply the edited map
//! is neither in the mount nor on disk.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::contract::{EditSession, MapBody, Worker, WorldError, WorldInstall};
use crate::model::{
    interior_key, AssetKey, Drawn, Draws, Index, Instances, InteriorPlaced, MapId, Moment,
    Switched, MOST_KEYED,
};
use crate::policy::StreamPolicy;
use crate::streaming::{Forest, ListDraw, Node, NodeDraw, NodeId, NodeState, Selection, View};

/// Why a map is in the forest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The view is where its policy streams it in, and its gate lets it in.
    Own,
    /// It is a parent of this map, which is wanted.
    ParentOf(MapId),
}

/// An interior whose nodes a view brought in or could not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteriorAt {
    /// The map that places it.
    pub map: MapId,
    pub placed: InteriorPlaced,
}

impl InteriorAt {
    /// The key its nodes stand under in the forest.
    pub fn key(&self) -> u64 {
        interior_key(self.map, self.placed.node)
    }
}

/// What streaming for a view came to, each list in the index's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Streamed {
    /// The maps in the forest, each with why.
    pub wanted: Vec<(MapId, Why)>,
    /// Those of them whose body came in for this view.
    pub read: Vec<MapId>,
    /// The maps wanted whose body has not come in.
    pub waiting: Vec<MapId>,
    /// The maps wanted that gave no body, each with why.
    pub unread: Vec<(MapId, String)>,
    /// The interiors within their instance's distance whose nodes stand in the forest.
    pub interiors: Vec<InteriorAt>,
    /// The interiors within their instance's distance that gave no body, each with why.
    pub interiors_unread: Vec<(InteriorAt, String)>,
}

/// What a selection draws and what it waits for.
#[derive(Clone, Default)]
pub struct Selected {
    /// Body after body by forest key, each as its body laid it out.
    pub instances: Instances,
    /// The models in range that are loading, each once, nearest ancestors first.
    pub waiting: Vec<AssetKey>,
    /// Whether a fade in time is under way: the next selection differs though nothing moves.
    pub fading: bool,
    /// The bodies in the forest, each by its key: their lights and occluders ride with the selection.
    pub bodies: Vec<(u64, Arc<dyn MapBody>)>,
    /// What the forest selected, its entities and its lists sorted by node.
    pub selection: Selection,
}

struct Held {
    body: Arc<dyn MapBody>,
    bytes: u64,
    /// The last view that wanted the map, by its number.
    used: u64,
    /// It came from an edit session, not from the reader.
    edited: bool,
}

/// The forest of one opened world near the views asked about.
pub struct Driver {
    install: Arc<dyn WorldInstall>,
    index: Arc<Index>,
    policy: StreamPolicy,
    words: usize,
    forest: Forest,
    held: HashMap<MapId, Held>,
    held_bytes: u64,
    /// The maps in the forest.
    streamed: BTreeSet<MapId>,
    /// What the last view wanted, each with why and how far its entities stand from the eye.
    wanted: BTreeMap<MapId, (Why, f32)>,
    /// The maps whose body came in since the last [`Self::settle`].
    came_in: Vec<MapId>,
    interiors: BTreeMap<u64, (InteriorAt, Arc<dyn MapBody>)>,
    /// ⚠ Kept for the world's life, so a body that does not read is asked for once and not at every view.
    refused: HashMap<MapId, String>,
    refused_interiors: HashMap<u64, String>,
    asked: u64,
    worker: Worker,
}

impl Driver {
    /// No map yet, over `install` and the index linked from its rows.
    pub fn new(install: Arc<dyn WorldInstall>, index: Arc<Index>) -> Self {
        let (policy, words) = (install.policy(), install.caps().instance_words);
        Self {
            install,
            index,
            policy,
            words,
            forest: Forest::waiting_for(policy.children),
            held: HashMap::new(),
            held_bytes: 0,
            streamed: BTreeSet::new(),
            wanted: BTreeMap::new(),
            came_in: Vec::new(),
            interiors: BTreeMap::new(),
            refused: HashMap::new(),
            refused_interiors: HashMap::new(),
            asked: 0,
            worker: Worker::new(),
        }
    }

    pub fn index(&self) -> &Arc<Index> {
        &self.index
    }

    pub fn policy(&self) -> &StreamPolicy {
        &self.policy
    }

    /// The maps `view` wants at `at` under `switched`: those its policy streams in and whose gate lets them in, and their parents.
    fn wanted(&self, view: &View, at: &Moment, switched: &Switched) -> BTreeMap<MapId, (Why, f32)> {
        let mut wanted = BTreeMap::new();
        for (id, row) in (0u32..).map(MapId).zip(self.index.maps()) {
            let Some(boxes) = row.boxes else { continue };
            if (self.policy.streams_in)(&boxes, view.eye) && row.gate.admits(id, at, switched) {
                let (min, max) = boxes.entities;
                // A box stated inside out holds no point to clamp to.
                let away = match min.cmple(max).all() {
                    true => view.eye.distance(view.eye.clamp(min, max)),
                    false => view.eye.distance((min + max) / 2.0),
                };
                wanted.insert(id, (Why::Own, away));
            }
        }
        let mut open: Vec<(MapId, f32)> = (wanted.iter())
            .map(|(id, (_, away))| (*id, *away))
            .collect();
        // Each step takes a map not yet wanted, so parents that name each other end it.
        while let Some((child, away)) = open.pop() {
            let parents = self.index.map(child).map_or(&[][..], |row| &row.parents);
            for parent in parents {
                if !wanted.contains_key(parent) {
                    wanted.insert(*parent, (Why::ParentOf(child), away));
                    open.push((*parent, away));
                }
            }
        }
        wanted
    }

    /// Let go of what stands in the forest of `map`, so that its body is put there again.
    fn unstream(&mut self, map: MapId) {
        self.streamed.remove(&map);
        let under: Vec<u64> = (self.interiors.iter())
            .filter(|(_, (at, _))| at.map == map)
            .map(|(key, _)| *key)
            .collect();
        for key in under {
            self.interiors.remove(&key);
            self.forest.remove(key);
        }
    }

    fn hold(&mut self, map: MapId, body: Arc<dyn MapBody>, edited: bool) {
        let (bytes, used) = (body.bytes(), self.asked);
        if let Some(was) = self.held.insert(
            map,
            Held {
                body,
                bytes,
                used,
                edited,
            },
        ) {
            self.held_bytes -= was.bytes;
        }
        self.held_bytes += bytes;
        self.unstream(map);
    }

    fn drop_held(&mut self, map: MapId) {
        if let Some(was) = self.held.remove(&map) {
            self.held_bytes -= was.bytes;
        }
        self.unstream(map);
        self.forest.remove(map.key());
    }

    /// The maps `view` wants at `at` under `switched` whose body is to be read, the nearest first; a body `edits` holds is taken from it and never read.
    pub fn want(
        &mut self,
        view: &View,
        at: &Moment,
        switched: &Switched,
        edits: Option<&dyn EditSession>,
    ) -> Vec<MapId> {
        self.asked += 1;
        self.wanted = self.wanted(view, at, switched);
        let maps: Vec<MapId> = self.wanted.keys().copied().collect();
        for map in &maps {
            match (
                edits.and_then(|edits| edits.body(*map)),
                self.held.get_mut(map),
            ) {
                (Some(body), Some(held)) if Arc::ptr_eq(&body, &held.body) => {}
                (Some(body), _) => self.hold(*map, body, true),
                // What an edit made of the map is gone from the session: the reader's stands again.
                (None, Some(held)) if held.edited => self.drop_held(*map),
                (None, _) => {}
            }
            // ⚠ Before any map is read: a map of this view that is held must not be the one dropped
            // to make room for another of the same view.
            if let Some(held) = self.held.get_mut(map) {
                held.used = self.asked;
            }
        }
        let mut ask: Vec<(f32, MapId)> = (self.wanted.iter())
            .filter(|(map, _)| !self.held.contains_key(map) && !self.refused.contains_key(map))
            .map(|(map, (_, away))| (*away, *map))
            .collect();
        ask.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        ask.into_iter().map(|(_, map)| map).collect()
    }

    /// Take the body of `map` as its reader gave it, or why it gave none.
    pub fn give(&mut self, map: MapId, body: Result<Arc<dyn MapBody>, WorldError>) {
        match body {
            Ok(body) => {
                self.hold(map, body, false);
                self.came_in.push(map);
            }
            Err(refused) => {
                tracing::warn!(map = map.0, why = %refused, "a map gave the forest nothing");
                self.refused.insert(map, refused.to_string());
            }
        }
    }

    /// Put the bodies held of the maps wanted in the forest in place of those the view left, and bring in the interiors `view` is within the distance of.
    pub fn settle(&mut self, view: &View) -> Streamed {
        let mut out = Streamed {
            read: std::mem::take(&mut self.came_in),
            ..Streamed::default()
        };
        out.read.retain(|map| self.wanted.contains_key(map));
        out.read.sort_unstable();
        let mut now = BTreeSet::new();
        for (map, (why, _)) in &self.wanted {
            if let Some(refused) = self.refused.get(map) {
                out.unread.push((*map, refused.clone()));
                continue;
            }
            let Some(held) = self.held.get(map) else {
                out.waiting.push(*map);
                continue;
            };
            if self.streamed.insert(*map) {
                self.forest.insert(map.key(), held.body.nodes().to_vec());
            }
            now.insert(*map);
            out.wanted.push((*map, *why));
        }
        let left: Vec<MapId> = self.streamed.difference(&now).copied().collect();
        for map in left {
            self.unstream(map);
            self.forest.remove(map.key());
        }
        self.bring_interiors(view, &mut out);
        self.shed();
        out
    }

    /// [`Self::want`], each body read on this thread through the world's reader, at most `most` of them, then [`Self::settle`].
    pub fn stream(
        &mut self,
        view: &View,
        at: &Moment,
        switched: &Switched,
        edits: Option<&dyn EditSession>,
        most: usize,
    ) -> Streamed {
        let ask = self.want(view, at, switched, edits);
        let install = Arc::clone(&self.install);
        for map in ask.into_iter().take(most) {
            let body = install.maps().read(&self.index, map, &mut self.worker);
            self.give(map, body);
        }
        self.settle(view)
    }

    /// Puts in the forest the nodes of each interior of a streamed map that `view` is within the distance of, and takes out those of the others.
    fn bring_interiors(&mut self, view: &View, out: &mut Streamed) {
        let mut near = Vec::new();
        for map in &self.streamed {
            let Some(held) = self.held.get(map) else {
                continue;
            };
            let nodes = held.body.nodes();
            for placed in held.body.interiors() {
                let Some(node) = nodes.get(placed.node as usize) else {
                    continue;
                };
                // As far as the instance itself would draw at all: its distance and its band.
                let reach = node.scaled(node.lod_dist, view) + view.fade[node.level as usize];
                if view.eye.distance(node.position) <= reach {
                    near.push(InteriorAt {
                        map: *map,
                        placed: *placed,
                    });
                }
            }
        }
        let now: BTreeSet<u64> = near.iter().map(InteriorAt::key).collect();
        let left: Vec<u64> = (self.interiors.keys().copied())
            .filter(|key| !now.contains(key))
            .collect();
        for key in left {
            self.interiors.remove(&key);
            self.forest.remove(key);
        }
        let install = Arc::clone(&self.install);
        for at in near {
            let key = at.key();
            if self.interiors.contains_key(&key) {
                out.interiors.push(at);
                continue;
            }
            let read = match self.refused_interiors.get(&at.placed.interior) {
                Some(refused) => Err(refused.clone()),
                None if at.placed.node >= MOST_KEYED => {
                    Err("its instance stands past the last node a forest keys an interior by".into())
                }
                None => (install.maps())
                    .interior(&self.index, at.map, &at.placed, &mut self.worker)
                    .map_err(|refused| {
                        tracing::warn!(interior = at.placed.interior, why = %refused, "an interior gave the forest nothing");
                        let why = refused.to_string();
                        self.refused_interiors.insert(at.placed.interior, why.clone());
                        why
                    }),
            };
            match read {
                Ok(body) => {
                    self.forest.insert(key, body.nodes().to_vec());
                    self.interiors.insert(key, (at, body));
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
        while self.held_bytes > self.policy.budget {
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

    /// The body that stands under `key` in the forest.
    fn body(&self, key: u64) -> Option<&Arc<dyn MapBody>> {
        match MapId::of_key(key) {
            Some(map) => self.held.get(&map).map(|held| &held.body),
            None => self.interiors.get(&key).map(|(_, body)| body),
        }
    }

    /// What the forest draws for `view` at `at`; `ready` says where a model stands.
    pub fn select(
        &mut self,
        view: &View,
        at: &Moment,
        ready: &dyn Fn(AssetKey) -> NodeState,
    ) -> Selected {
        let (held, interiors) = (&self.held, &self.interiors);
        let draws = |id: NodeId| -> Draws {
            let body = match MapId::of_key(id.0) {
                Some(map) => held.get(&map).map(|held| &held.body),
                None => interiors.get(&id.0).map(|(_, body)| body),
            };
            body.map_or(Draws::Nothing, |body| body.draws(id.1, at))
        };
        let state = |id: NodeId| match draws(id) {
            Draws::Model(model) => ready(model),
            Draws::StandIn => NodeState::Ready,
            Draws::Nothing => NodeState::Absent,
        };
        let mut selection = self.forest.select(view, &state);
        selection.drawn.sort_unstable_by_key(|draw| draw.node);
        selection.lists.sort_unstable_by_key(|list| list.node);
        let mut seen = BTreeSet::new();
        let waiting = (selection.wanted.iter())
            .filter_map(|id| match draws(*id) {
                Draws::Model(model) => Some(model),
                Draws::StandIn | Draws::Nothing => None,
            })
            .filter(|model| seen.insert(*model))
            .collect();
        let mut instances = Instances::of_width(self.words);
        let (mut nodes, mut lists): (&[NodeDraw], &[ListDraw]) =
            (&selection.drawn, &selection.lists);
        // Body after body by key: each run of the two sorted lists that one key heads.
        while let Some(key) = [
            nodes.first().map(|n| n.node.0),
            lists.first().map(|l| l.node.0),
        ]
        .into_iter()
        .flatten()
        .min()
        {
            let of_nodes = nodes.partition_point(|draw| draw.node.0 == key);
            let of_lists = lists.partition_point(|list| list.node.0 == key);
            if let Some(body) = self.body(key) {
                let drawn = Drawn {
                    key,
                    nodes: &nodes[..of_nodes],
                    lists: &lists[..of_lists],
                    skipped: &selection.skipped,
                };
                body.lay(&drawn, at, &mut instances);
            }
            (nodes, lists) = (&nodes[of_nodes..], &lists[of_lists..]);
        }
        let maps = (self.streamed.iter())
            .filter_map(|map| Some((map.key(), Arc::clone(&self.held.get(map)?.body))));
        let within = (self.interiors.iter()).map(|(key, (_, body))| (*key, Arc::clone(body)));
        Selected {
            instances,
            waiting,
            fading: selection.fading,
            bodies: maps.chain(within).collect(),
            selection,
        }
    }

    /// The node `id` names, where its map or its interior is in the forest.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
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
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
