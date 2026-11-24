//! Which maps a focus streams in, and which nodes of the loaded maps' level-of-detail forest draw:
//! a parent or its children by the distance to the parent, a parent held while a child is not
//! ready, and an instance of a list left out while the entity that stands in for it draws.
//!
//! ⚠ A child is judged by the distance to its **parent's** position, never its own, so the
//! children of one parent appear and go together.

use std::collections::HashMap;

use glam::Vec3;

/// How far past its box a map streams in, in metres.
pub const PRESTREAM: f32 = 50.0;

/// The two boxes a map states: the one it streams in by, and the one its entities stand in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapBox {
    pub streaming: (Vec3, Vec3),
    pub entities: (Vec3, Vec3),
}

impl MapBox {
    /// Whether a focus at `at` streams the map in.
    pub fn wants(&self, at: Vec3) -> bool {
        let (s, e) = (self.streaming, self.entities);
        // A streaming box that does not hold the entities is not the one the map streams by.
        let holds = s.0.cmple(e.0).all() && s.1.cmpge(e.1).all();
        let (min, max) = if holds { s } else { e };
        let grow = Vec3::splat(PRESTREAM);
        at.cmpge(min - grow).all() && at.cmple(max + grow).all()
    }
}

/// A node's parent: the map it lies in, by the caller's key, and its place among that map's nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentRef {
    pub map: u64,
    pub node: u32,
}

/// What a node's distance is measured to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Extent {
    /// Its position.
    Point,
    /// The box of a list of instances, the lowest corner and the highest: nought inside it.
    Box(Vec3, Vec3),
}

/// A node of the forest: an entity, or a whole list of instances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub position: Vec3,
    pub extent: Extent,
    /// How far it draws, in metres, the archetype's where the entity states none.
    pub lod_dist: f32,
    /// Within this its children draw in its place; not above 0 where it states none.
    pub child_lod_dist: f32,
    pub parent: Option<ParentRef>,
    /// The instance of its parent list it stands in for, where its parent is a list and it names one.
    pub instance: Option<u32>,
}

/// Where a node's model stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    /// On the device: it draws.
    Ready,
    /// Asked for or not yet: its parent draws in its place until it arrives.
    Loading,
    /// It has no model and never will: it draws nothing and holds nothing back.
    Absent,
}

/// A node by its map's key and its place among the map's nodes.
pub type NodeId = (u64, u32);

/// A list that draws, and the instances left out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListDraw {
    pub node: NodeId,
    /// A range of [`Selection::skipped`].
    pub skipped: std::ops::Range<usize>,
}

/// What a selection draws and what it waits for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The entities that draw.
    pub drawn: Vec<NodeId>,
    /// The lists that draw.
    pub lists: Vec<ListDraw>,
    /// The instances left out of the lists, list after list.
    pub skipped: Vec<u32>,
    /// The nodes in range whose model is loading, nearest ancestors first.
    pub wanted: Vec<NodeId>,
}

const NONE: u32 = u32::MAX;

struct Loaded {
    key: u64,
    nodes: Vec<Node>,
    /// Whether every wanted map that names this one as a parent is loaded.
    complete: bool,
}

/// The links of the loaded nodes, numbered map after map; made again when a map comes or goes.
#[derive(Default)]
struct Links {
    /// The first node of each map.
    base: Vec<u32>,
    /// The map of each node, by its place among the maps.
    slot: Vec<u32>,
    parent: Vec<u32>,
    first_child: Vec<u32>,
    next_sibling: Vec<u32>,
    /// How far each node draws once its parent's `child_lod_dist` has taken the place of its own.
    range: Vec<f32>,
    /// Within this a node's children draw in its place: its own, else the farthest of its children's.
    child_range: Vec<f32>,
    roots: Vec<u32>,
}

/// The level-of-detail forest of the maps streamed in.
#[derive(Default)]
pub struct Forest {
    maps: Vec<Loaded>,
    slots: HashMap<u64, usize>,
    links: Links,
    stale: bool,
    /// Whether each node gives way to its children this selection: 0 not yet known, 1 yes, 2 no.
    yields: Vec<u8>,
}

impl Forest {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the map `key` is loaded.
    pub fn holds(&self, key: u64) -> bool {
        self.slots.contains_key(&key)
    }

    /// How many maps and how many nodes are loaded.
    pub fn len(&self) -> (usize, usize) {
        let nodes = self.maps.iter().map(|m| m.nodes.len()).sum();
        (self.maps.len(), nodes)
    }

    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    /// Load the nodes of the map `key`, in place of any it held.
    pub fn insert(&mut self, key: u64, nodes: Vec<Node>) {
        let loaded = Loaded {
            key,
            nodes,
            complete: true,
        };
        match self.slots.get(&key) {
            Some(at) => self.maps[*at] = loaded,
            None => {
                self.slots.insert(key, self.maps.len());
                self.maps.push(loaded);
            }
        }
        self.stale = true;
    }

    /// Let the map `key` go.
    pub fn remove(&mut self, key: u64) {
        let Some(at) = self.slots.remove(&key) else {
            return;
        };
        self.maps.swap_remove(at);
        if let Some(moved) = self.maps.get(at) {
            self.slots.insert(moved.key, at);
        }
        self.stale = true;
    }

    /// Say whether every wanted map that names the map `key` as a parent is loaded: until then none
    /// of its nodes gives way to its children.
    pub fn set_complete(&mut self, key: u64, complete: bool) {
        if let Some(at) = self.slots.get(&key) {
            self.maps[*at].complete = complete;
        }
    }

    /// The node `id` names, where its map is loaded.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.maps
            .get(*self.slots.get(&id.0)?)?
            .nodes
            .get(id.1 as usize)
    }

    fn relink(&mut self) {
        let mut links = Links::default();
        let mut total = 0u32;
        for map in &self.maps {
            links.base.push(total);
            total += map.nodes.len() as u32;
        }
        let count = total as usize;
        links.slot = Vec::with_capacity(count);
        links.parent = vec![NONE; count];
        links.first_child = vec![NONE; count];
        links.next_sibling = vec![NONE; count];
        links.range = Vec::with_capacity(count);
        links.child_range = Vec::with_capacity(count);
        for (slot, map) in self.maps.iter().enumerate() {
            for node in &map.nodes {
                links.slot.push(slot as u32);
                links.range.push(node.lod_dist);
                links.child_range.push(node.child_lod_dist);
            }
        }
        let mut at = 0usize;
        for map in &self.maps {
            for node in &map.nodes {
                let parent = node.parent.and_then(|p| {
                    let slot = *self.slots.get(&p.map)?;
                    ((p.node as usize) < self.maps[slot].nodes.len())
                        .then(|| links.base[slot] + p.node)
                });
                // A node that names itself, or a parent not loaded, stands as a root.
                match parent.filter(|p| *p as usize != at) {
                    Some(parent) => {
                        links.parent[at] = parent;
                        links.next_sibling[at] = links.first_child[parent as usize];
                        links.first_child[parent as usize] = at as u32;
                    }
                    None => links.roots.push(at as u32),
                }
                at += 1;
            }
        }
        // A parent that states no distance for its children takes the farthest of theirs, and one
        // that states one gives it to each child that stands in for the whole of it.
        for child in 0..count {
            let parent = links.parent[child];
            if parent == NONE || self.node_at(&links, child).instance.is_some() {
                continue;
            }
            let stated = self.node_at(&links, parent as usize).child_lod_dist;
            if stated > 0.0 {
                links.range[child] = stated;
            } else {
                let far = links.child_range[parent as usize].max(links.range[child]);
                links.child_range[parent as usize] = far;
            }
        }
        self.links = links;
        self.stale = false;
    }

    fn node_at(&self, links: &Links, at: usize) -> &Node {
        let slot = links.slot[at] as usize;
        &self.maps[slot].nodes[at - links.base[slot] as usize]
    }

    fn id_of(&self, at: u32) -> NodeId {
        let slot = self.links.slot[at as usize] as usize;
        (self.maps[slot].key, at - self.links.base[slot])
    }

    /// The nodes that draw for a camera at `eye` with distances scaled by `scale`, and those
    /// waited for; `state` says where the model of a node stands.
    pub fn select(
        &mut self,
        eye: Vec3,
        scale: f32,
        state: &dyn Fn(NodeId) -> NodeState,
    ) -> Selection {
        if self.stale {
            self.relink();
        }
        self.yields.clear();
        self.yields.resize(self.links.slot.len(), 0);
        let mut pass = Pass {
            forest: self,
            eye,
            scale,
            state,
            out: Selection::default(),
        };
        for k in 0..pass.forest.links.roots.len() {
            let root = pass.forest.links.roots[k];
            pass.draw(root);
        }
        pass.out
    }
}

struct Pass<'a> {
    forest: &'a mut Forest,
    eye: Vec3,
    scale: f32,
    state: &'a dyn Fn(NodeId) -> NodeState,
    out: Selection,
}

impl Pass<'_> {
    fn node(&self, at: u32) -> Node {
        *self.forest.node_at(&self.forest.links, at as usize)
    }

    /// How far the camera is from what node `at` is measured to.
    fn away(&self, at: u32) -> f32 {
        let node = self.node(at);
        match node.extent {
            Extent::Point => self.eye.distance(node.position),
            Extent::Box(min, max) => self.eye.distance(self.eye.clamp(min, max)),
        }
    }

    /// Whether node `at` is within the distance it draws to: measured to its parent where it
    /// stands in for the whole of one, to itself otherwise.
    fn in_range(&self, at: u32) -> bool {
        let parent = self.forest.links.parent[at as usize];
        let whole_child = parent != NONE && self.node(at).instance.is_none();
        let away = self.away(if whole_child { parent } else { at });
        away <= self.forest.links.range[at as usize] * self.scale
    }

    fn first_child(&self, at: u32) -> u32 {
        self.forest.links.first_child[at as usize]
    }

    fn next_sibling(&self, at: u32) -> u32 {
        self.forest.links.next_sibling[at as usize]
    }

    /// Whether node `at` gives way to the children that stand in for the whole of it: it has some,
    /// its map's child maps are loaded, the camera is within their distance and each is ready.
    fn yields(&mut self, at: u32) -> bool {
        match self.forest.yields[at as usize] {
            1 => return true,
            2 => return false,
            _ => {}
        }
        let complete = self.forest.maps[self.forest.links.slot[at as usize] as usize].complete;
        let near = self.away(at) <= self.forest.links.child_range[at as usize] * self.scale;
        let (mut any, mut ready) = (false, true);
        let mut child = if near { self.first_child(at) } else { NONE };
        // Every child is asked, so that each one still loading is waited for.
        while child != NONE {
            if self.node(child).instance.is_none() {
                any = true;
                ready &= self.ready(child);
            }
            child = self.next_sibling(child);
        }
        let yields = any && ready && complete;
        self.forest.yields[at as usize] = if yields { 1 } else { 2 };
        yields
    }

    /// Whether node `at` can take its parent's place: its model is on the device, or it gives way
    /// to children that can.
    fn ready(&mut self, at: u32) -> bool {
        if self.yields(at) {
            return true;
        }
        match (self.state)(self.forest.id_of(at)) {
            NodeState::Ready | NodeState::Absent => true,
            NodeState::Loading => {
                self.out.wanted.push(self.forest.id_of(at));
                false
            }
        }
    }

    /// Draw node `at`, whose parent has given way to it or which has none.
    fn draw(&mut self, at: u32) {
        if !self.in_range(at) {
            return;
        }
        if self.yields(at) {
            let mut child = self.first_child(at);
            while child != NONE {
                if self.node(child).instance.is_none() {
                    self.draw(child);
                }
                child = self.next_sibling(child);
            }
            return;
        }
        let id = self.forest.id_of(at);
        if matches!(self.node(at).extent, Extent::Box(..)) {
            self.draw_list(at, id);
            return;
        }
        match (self.state)(id) {
            NodeState::Ready => self.out.drawn.push(id),
            NodeState::Loading => self.out.wanted.push(id),
            NodeState::Absent => {}
        }
    }

    /// Draw the list `at` less each instance whose own entity draws in its place.
    fn draw_list(&mut self, at: u32, id: NodeId) {
        match (self.state)(id) {
            NodeState::Ready => {}
            NodeState::Loading => return self.out.wanted.push(id),
            NodeState::Absent => return,
        }
        let from = self.out.skipped.len();
        let mut child = self.first_child(at);
        while child != NONE {
            if let Some(instance) = self.node(child).instance.filter(|_| self.in_range(child)) {
                let child_id = self.forest.id_of(child);
                match (self.state)(child_id) {
                    NodeState::Ready => {
                        self.out.drawn.push(child_id);
                        self.out.skipped.push(instance);
                    }
                    NodeState::Loading => self.out.wanted.push(child_id),
                    // An entity with no model leaves its instance drawn.
                    NodeState::Absent => {}
                }
            }
            child = self.next_sibling(child);
        }
        self.out.lists.push(ListDraw {
            node: id,
            skipped: from..self.out.skipped.len(),
        });
    }
}

#[cfg(test)]
#[path = "streaming_tests.rs"]
mod tests;
