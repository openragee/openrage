//! Which maps a camera streams in, and which nodes of the loaded maps' level-of-detail forest draw
//! and how opaque: every node draws by its own distance, the children of one parent as opaque as
//! the most opaque of them, a parent with all its children found fades out as they fade in and is
//! held while one that draws is not ready, a far node with no parent fades in a step a selection,
//! and an instance of a list is left out while the entity that stands in for it draws.
//!
//! ⚠ An entity that stands in for the whole of its parent is judged by the distance to its
//! **parent's** position, never its own; a list is judged by its own box's centre. Under a parent
//! that is an entity, a scale below 1 keeps the **parent's** radius.

use std::collections::HashMap;

use glam::Vec3;

use crate::streaming_rules::{distance_alpha, map_streaming_sphere};

/// The band past its distance a level fades out over where the install states none, in metres.
pub const FADE: f32 = 20.0;
/// The band of an orphan that states 20 m or less, in metres.
pub const FADE_SMALL: f32 = 5.0;
/// The farthest an orphan states and still fades over [`FADE_SMALL`], in metres.
pub const SMALL: f32 = 20.0;
/// How much a fade in time gains in one selection, however long since the last.
pub const FADE_STEP: f32 = 51.0 / 255.0;
/// Faster than this nothing fades in time, in metres a second.
pub const FADE_MAX_SPEED: f32 = 80.0;
/// A camera that moved farther than this since the last selection ends every fade in time, in metres.
pub const FADE_MAX_MOVE: f32 = 5.0;
/// A level's scale that changed by this much since the last selection ends every fade in time.
const SCALE_CHANGE: f32 = 0.001;
/// What a terrain archetype's distance is scaled by before its level's scale, by `Level as usize`.
pub const TERRAIN: [f32; Level::COUNT] = [0.5, 0.8, 0.8, 0.8, 0.8, 0.9, 1.0];
/// What an environment archetype's distance is scaled by before its level's scale, by `Level as usize`.
pub const ENVIRONMENT: [f32; Level::COUNT] = [0.8, 0.6, 0.5, 0.5, 0.85, 0.8, 1.0];
/// What a vegetation archetype's distance is scaled by before its level's scale.
pub const VEGETATION: f32 = 1.0;
/// What a grass list of plain instances is scaled by before its level's scale.
pub const GRASS: f32 = 0.8;

/// What a camera's vertical field of view `fov_y`, in radians, multiplies every distance by: 1 at 45° or wider.
pub fn camera_scale(fov_y: f32) -> f32 {
    (std::f32::consts::FRAC_PI_8.tan() / (fov_y / 2.0).tan()).max(1.0)
}

/// What an archetype's own load flags make of the scale of its distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    Other,
    Vegetation,
    Terrain,
    Environment,
}

impl Category {
    /// The category of an archetype of the load flags `load_flags`: of bits 16, 0 and 7 the last set wins.
    pub fn of(load_flags: u32) -> Self {
        let mut category = Self::Other;
        for (bit, then) in [
            (16, Self::Vegetation),
            (0, Self::Terrain),
            (7, Self::Environment),
        ] {
            if load_flags & (1 << bit) != 0 {
                category = then;
            }
        }
        category
    }
}

/// What a node is, as far as the scale of its distance goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// An entity, or a list of far stand-ins.
    #[default]
    Entity,
    /// A prop list: its level's scale never shortens it.
    Props,
    /// A grass list of plain instances.
    Grass,
}

/// A node's level of detail, numbered as a map states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Level {
    Hd = 0,
    Lod = 1,
    Slod1 = 2,
    Slod2 = 3,
    Slod3 = 4,
    #[default]
    OrphanHd = 5,
    Slod4 = 6,
}

impl Level {
    pub const COUNT: usize = 7;

    /// The level a map's number names; a number past the last reads as an orphan.
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Hd,
            1 => Self::Lod,
            2 => Self::Slod1,
            3 => Self::Slod2,
            4 => Self::Slod3,
            6 => Self::Slod4,
            _ => Self::OrphanHd,
        }
    }
}

/// The camera a selection is made for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub eye: Vec3,
    /// Metres a second: past [`FADE_MAX_SPEED`] nothing fades in time. Nothing is asked for ahead of it.
    pub velocity: Vec3,
    /// What the distances of each level are multiplied by, by `Level as usize`: the camera's scale times the level's own.
    pub scales: [f32; Level::COUNT],
    /// The band past its distance each level fades out over, by `Level as usize`, in metres.
    pub fade: [f32; Level::COUNT],
}

impl View {
    /// A camera at rest at `eye`, every level scaled by `scale` and fading out over [`FADE`].
    pub fn at(eye: Vec3, scale: f32) -> Self {
        Self {
            eye,
            velocity: Vec3::ZERO,
            scales: [scale; Level::COUNT],
            fade: [FADE; Level::COUNT],
        }
    }
}

/// The two boxes a map states: the one it streams in by, and the one its entities stand in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapBox {
    pub streaming: (Vec3, Vec3),
    pub entities: (Vec3, Vec3),
}

impl MapBox {
    /// Whether a camera at `at` streams the map in, at any speed: within its streaming box's sphere.
    // ⚠ The sphere of the entities' box streams in almost nothing: a few dozen maps of thousands, at a city camera.
    pub fn wants(&self, at: Vec3) -> bool {
        let (min, max) = self.streaming;
        map_streaming_sphere(min, max).is_some_and(|sphere| sphere.contains(at))
    }
}

/// A node's parent: the map it lies in, by the caller's key, and its place among that map's nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentRef {
    pub map: u64,
    pub node: u32,
}

/// What a node is: one thing, or a list of instances in a box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Extent {
    Point,
    /// The box of a list of instances, the lowest corner and the highest.
    // ⚠ A list's distance is measured to its position, the box's centre, never to the box: a list
    // of far stand-ins states its half diagonal plus its instances' distance.
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
    pub level: Level,
    pub kind: Kind,
    /// Its archetype's.
    pub category: Category,
    /// What a scale below 1 leaves of its distance, in metres: a list's half diagonal, an entity's
    /// sphere; under a parent that is an entity, the parent's is taken in its place.
    pub radius: f32,
    /// How many children it states, the entities that stand in for its instances among them: it
    /// fades out as they fade in only once that many are loaded.
    pub num_children: u32,
    /// A prop list of the batch levels: it never fades out for its children, which leave out the
    /// instances they stand in for instead.
    pub batch: bool,
}

impl Node {
    /// What its distances are multiplied by for `view`.
    pub fn scale(&self, view: &View) -> f32 {
        let level = self.level as usize;
        // ⚠ A later rule replaces an earlier one, never multiplies it.
        let own = match self.category {
            Category::Vegetation => VEGETATION,
            Category::Terrain => TERRAIN[level],
            Category::Environment => ENVIRONMENT[level],
            Category::Other if self.kind == Kind::Grass => GRASS,
            Category::Other => 1.0,
        };
        let map = view.scales[level];
        own * match self.kind {
            Kind::Props => map.max(1.0),
            _ => map,
        }
    }

    /// How far it draws whole for `view` where it states `dist`: a scale below 1 shortens no more than what lies past its radius.
    pub fn scaled(&self, dist: f32, view: &View) -> f32 {
        match self.scale(view) {
            s if s < 1.0 => self.radius + s * (dist - self.radius),
            s => s * dist,
        }
    }
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

/// An entity that draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeDraw {
    pub node: NodeId,
    /// How opaque, above 0 and to 1.
    pub fade: f32,
    /// It fades out for its children: drawn by the inverse of their dither, the two leave no pixel bare.
    pub yields: bool,
}

/// A list that draws, and the instances left out of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ListDraw {
    pub node: NodeId,
    /// How opaque every instance of it drawn is, above 0 and to 1.
    pub fade: f32,
    /// It fades out for its children, as [`NodeDraw::yields`].
    pub yields: bool,
    /// A range of [`Selection::skipped`].
    pub skipped: std::ops::Range<usize>,
}

/// A node whose model is loading, as it draws once the model is ready.
#[derive(Debug, Clone, PartialEq)]
pub struct Waiting {
    pub node: NodeId,
    /// Its parent; itself for a root.
    pub parent: NodeId,
    pub fade: f32,
    pub yields: bool,
    /// For a list, a range of [`Selection::skipped`]; empty for an entity.
    pub skipped: std::ops::Range<usize>,
}

/// A parent drawn whole for a child on its way, and how it draws once every such child is ready.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub node: NodeId,
    /// How opaque it draws then: nought where it is gone.
    pub then: f32,
    pub yields: bool,
    /// For a list, a range of [`Selection::skipped`] left out then besides its own.
    pub skipped: std::ops::Range<usize>,
}

/// What a selection draws and what it waits for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    /// The entities that draw.
    pub drawn: Vec<NodeDraw>,
    /// The lists that draw.
    pub lists: Vec<ListDraw>,
    /// The instances left out of the lists, list after list.
    pub skipped: Vec<u32>,
    /// The nodes in range whose model is loading, nearest ancestors first.
    pub wanted: Vec<NodeId>,
    /// The nodes waited for, so that a frame can draw one the moment its model is ready.
    pub waiting: Vec<Waiting>,
    /// The parents of [`Self::drawn`] and [`Self::lists`] held whole for a child of `waiting`.
    pub held: Vec<Held>,
    /// Whether a fade in time is under way: the next selection differs though nothing moves.
    pub fading: bool,
}

const NONE: u32 = u32::MAX;

struct Loaded {
    key: u64,
    nodes: Vec<Node>,
    /// How opaque each node drew at the selection before this one, and at this one.
    was: Vec<f32>,
    now: Vec<f32>,
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
    /// What a scale below 1 leaves of each node's distance: its parent's where that is an entity.
    radius: Vec<f32>,
    roots: Vec<u32>,
}

impl Links {
    /// The map of node `at`, by its place among the maps, and the node's place in that map.
    fn place(&self, at: usize) -> (usize, usize) {
        let slot = self.slot[at] as usize;
        (slot, at - self.base[slot] as usize)
    }

    /// Make a root of each node `most` links below a root, so that no chain is longer.
    // ⚠ A parent is named by a map's own bytes, and a selection walks a chain by recursion.
    fn cut_deeper_than(&mut self, most: usize) {
        let mut open: Vec<(u32, usize)> = self.roots.iter().map(|root| (*root, 0)).collect();
        while let Some((at, depth)) = open.pop() {
            let mut child = self.first_child[at as usize];
            if depth + 1 >= most {
                self.first_child[at as usize] = NONE;
            }
            while child != NONE {
                let next = self.next_sibling[child as usize];
                match depth + 1 >= most {
                    true => {
                        (
                            self.parent[child as usize],
                            self.next_sibling[child as usize],
                        ) = (NONE, NONE);
                        self.roots.push(child);
                        open.push((child, 0));
                    }
                    false => open.push((child, depth + 1)),
                }
                child = next;
            }
        }
    }
}

/// The longest chain of parents a selection walks: far past the levels of detail a map states.
const DEEPEST: usize = 64;

/// The level-of-detail forest of the maps streamed in.
#[derive(Default)]
pub struct Forest {
    maps: Vec<Loaded>,
    slots: HashMap<u64, usize>,
    links: Links,
    stale: bool,
    /// Where the camera stood at the last selection, and the scales it had.
    last: Option<(Vec3, [f32; Level::COUNT])>,
    /// A parent waits for exactly the children it states, never raised to those loaded.
    exact_children: bool,
}

impl Forest {
    pub fn new() -> Self {
        Self::default()
    }

    /// A forest whose parents wait for exactly the children they state: GTA V's rule.
    pub fn with_exact_children() -> Self {
        Self {
            exact_children: true,
            ..Self::default()
        }
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
            was: vec![0.0; nodes.len()],
            now: vec![0.0; nodes.len()],
            nodes,
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
        links.radius = Vec::with_capacity(count);
        for (slot, map) in self.maps.iter().enumerate() {
            for node in &map.nodes {
                links.slot.push(slot as u32);
                links.range.push(node.lod_dist);
                links.radius.push(node.radius);
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
                if let Some(parent) = parent.filter(|p| *p as usize != at) {
                    links.parent[at] = parent;
                }
                at += 1;
            }
        }
        self.cut_loops(&mut links);
        for at in 0..count {
            match links.parent[at] {
                NONE => links.roots.push(at as u32),
                parent => {
                    links.next_sibling[at] = links.first_child[parent as usize];
                    links.first_child[parent as usize] = at as u32;
                }
            }
        }
        links.cut_deeper_than(DEEPEST);
        // A parent that states a distance for its children gives it to each child that stands in
        // for the whole of it.
        for child in 0..count {
            let parent = links.parent[child];
            if parent == NONE {
                continue;
            }
            // ⚠ Under a parent that is an entity, a child's scale keeps the parent's radius, never
            // its own: siblings that state one distance draw to one distance.
            let of = self.node_at(&links, parent as usize);
            if of.extent == Extent::Point {
                links.radius[child] = of.radius;
            }
            if self.node_at(&links, child).instance.is_some() {
                continue;
            }
            let stated = self.node_at(&links, parent as usize).child_lod_dist;
            if stated > 0.0 {
                links.range[child] = stated;
            }
        }
        self.links = links;
        self.stale = false;
    }

    /// Make a root of the node that draws farthest in each loop of the links' parents: a loop
    /// reaches no root, and nothing in or under it would draw.
    // ⚠ A parent is named by a map's own bytes, so maps can name each other round a loop.
    fn cut_loops(&self, links: &mut Links) {
        // 0 not walked yet, 1 on the walk under way, 2 walked.
        let mut state = vec![0u8; links.parent.len()];
        let mut walk = Vec::new();
        for start in 0..links.parent.len() {
            let mut at = start as u32;
            while at != NONE && state[at as usize] == 0 {
                state[at as usize] = 1;
                walk.push(at);
                at = links.parent[at as usize];
            }
            if at != NONE && state[at as usize] == 1 {
                let dist = |k: u32| self.node_at(links, k as usize).lod_dist;
                let (mut root, mut on) = (at, links.parent[at as usize]);
                while on != at {
                    if dist(on) > dist(root) {
                        root = on;
                    }
                    on = links.parent[on as usize];
                }
                links.parent[root as usize] = NONE;
            }
            for k in walk.drain(..) {
                state[k as usize] = 2;
            }
        }
    }

    fn node_at(&self, links: &Links, at: usize) -> &Node {
        let (slot, node) = links.place(at);
        &self.maps[slot].nodes[node]
    }

    fn id_of(&self, at: u32) -> NodeId {
        let (slot, node) = self.links.place(at as usize);
        (self.maps[slot].key, node as u32)
    }

    /// The nodes that draw for `view` and how opaque each is, and those waited for, by where `state` says each model stands.
    pub fn select(&mut self, view: &View, state: &dyn Fn(NodeId) -> NodeState) -> Selection {
        if self.stale {
            self.relink();
        }
        for map in &mut self.maps {
            std::mem::swap(&mut map.was, &mut map.now);
            map.now.fill(0.0);
        }
        // A first selection, a jump, a fast camera or a change of scale is a cut: what the
        // distances allow is there at once.
        let steady = self.last.is_some_and(|(eye, scales)| {
            eye.distance(view.eye) <= FADE_MAX_MOVE
                && (scales.iter().zip(&view.scales)).all(|(a, b)| (a - b).abs() < SCALE_CHANGE)
        });
        let timed = steady && view.velocity.length() <= FADE_MAX_SPEED;
        self.last = Some((view.eye, view.scales));
        let mut pass = Pass {
            forest: self,
            view,
            timed,
            state,
            out: Selection::default(),
        };
        for k in 0..pass.forest.links.roots.len() {
            let root = pass.forest.links.roots[k];
            pass.draw(root, None);
        }
        pass.out
    }
}

struct Pass<'a> {
    forest: &'a mut Forest,
    view: &'a View,
    /// Whether anything fades in time this selection.
    timed: bool,
    state: &'a dyn Fn(NodeId) -> NodeState,
    out: Selection,
}

impl Pass<'_> {
    fn node(&self, at: u32) -> Node {
        *self.forest.node_at(&self.forest.links, at as usize)
    }

    /// How far a camera at `eye` is from what node `at` is judged by: its parent where it is an
    /// entity that stands in for the whole of one, itself otherwise; a list's place is its box's centre.
    fn judged(&self, at: u32, eye: Vec3) -> f32 {
        let parent = self.forest.links.parent[at as usize];
        let node = self.node(at);
        let from_parent = parent != NONE && node.instance.is_none() && node.extent == Extent::Point;
        eye.distance(self.node(if from_parent { parent } else { at }).position)
    }

    /// How far node `at` draws whole, by its scale.
    fn reach(&self, at: u32) -> f32 {
        let links = &self.forest.links;
        let node = Node {
            radius: links.radius[at as usize],
            ..self.node(at)
        };
        node.scaled(links.range[at as usize], self.view)
    }

    /// The band past its distance over which node `at` fades out.
    fn band(&self, at: u32) -> f32 {
        let level = self.node(at).level;
        // ⚠ The short band is picked by the distance the node states, before any scale.
        match level == Level::OrphanHd && self.forest.links.range[at as usize] <= SMALL {
            true => FADE_SMALL,
            false => self.view.fade[level as usize],
        }
    }

    /// How opaque the distance alone makes node `at`: whole within the distance it draws to, and
    /// gone its band past it, in steps of 1/255.
    // ⚠ The step is taken down from a fifth of a hundredth of a step above: a node just short of its band's end draws nothing.
    fn by_distance(&self, at: u32) -> f32 {
        let away = self.judged(at, self.view.eye);
        f32::from(distance_alpha(away, self.reach(at), self.band(at))) / 255.0
    }

    fn first_child(&self, at: u32) -> u32 {
        self.forest.links.first_child[at as usize]
    }

    fn next_sibling(&self, at: u32) -> u32 {
        self.forest.links.next_sibling[at as usize]
    }

    /// Whether node `at` fades in by time: one with no parent that is neither of the high level
    /// nor an orphan.
    fn fades_in_time(&self, at: u32) -> bool {
        !matches!(self.node(at).level, Level::Hd | Level::OrphanHd)
            && self.forest.links.parent[at as usize] == NONE
    }

    /// How opaque node `at` draws now where `target` is what the distances allow, kept as [`Self::settle`] does.
    fn stepped(&self, at: u32, target: f32) -> f32 {
        let (slot, node) = self.forest.links.place(at as usize);
        let was = self.forest.maps[slot].was[node];
        match self.timed && target > was && self.fades_in_time(at) {
            true => (was + FADE_STEP).min(target),
            false => target,
        }
    }

    /// How opaque node `at` draws now where `target` is what the distances allow: what drew less
    /// a selection ago gains one step, and the rest is there at once.
    fn settle(&mut self, at: u32, target: f32) -> f32 {
        let fade = self.stepped(at, target);
        let (slot, node) = self.forest.links.place(at as usize);
        self.out.fading |= fade < target;
        self.forest.maps[slot].now[node] = fade;
        fade
    }

    /// Draw node `at` and every node under it, each by its own distance; `shared` is how opaque the
    /// most opaque child of its parent is, where it stands in for the whole of that parent.
    fn draw(&mut self, at: u32, shared: Option<f32>) {
        // Over its children, those that stand in for an instance among them: the most opaque, how
        // many are loaded, and whether one that draws at all is not on the device yet.
        let (mut most, mut found, mut held) = (0.0f32, 0u32, false);
        let mut child = self.first_child(at);
        while child != NONE {
            let alpha = self.by_distance(child);
            let id = self.forest.id_of(child);
            let loading = (self.state)(id) == NodeState::Loading;
            // ⚠ A child that holds its parent is waited for though its own children leave it nothing to draw.
            if alpha > 0.0 && loading {
                self.out.wanted.push(id);
            }
            (most, found, held) = (most.max(alpha), found + 1, held || (alpha > 0.0 && loading));
            child = self.next_sibling(child);
        }
        let mut child = self.first_child(at);
        while child != NONE {
            let whole = self.node(child).instance.is_none();
            self.draw(child, whole.then_some(most));
            child = self.next_sibling(child);
        }
        let own = self.by_distance(at);
        if own <= 0.0 {
            return;
        }
        let node = self.node(at);
        // ⚠ Only a parent the distance leaves whole fades out for its children, and only once every
        // child it states is loaded. What an entity states is raised to what is loaded, none
        // included; a list's is not, as a list is no entity that a child attaches to (inferred).
        let stated = match node.extent {
            Extent::Point if !self.forest.exact_children => node.num_children.max(found),
            _ => node.num_children,
        };
        let crosses = !node.batch && stated > 0 && found == stated;
        // How it draws once no child holds it.
        let (free, free_yields) = match crosses && own >= 1.0 {
            true => (1.0 - most, true),
            // A child is as opaque as the most opaque of its parent's.
            false => (shared.unwrap_or(own), false),
        };
        let (target, yields) = match held {
            true => (1.0, false),
            false => (free, free_yields),
        };
        if target <= 0.0 {
            return;
        }
        let id = self.forest.id_of(at);
        let parent = self.forest.links.parent[at as usize];
        match (self.state)(id) {
            NodeState::Ready => {}
            // A root was not waited for by a parent's pass. Either is drawn by the frame its model is
            // ready in, as the next selection will draw it: a root that fades in time a step in.
            NodeState::Loading => {
                let parent = match parent {
                    NONE => {
                        self.out.wanted.push(id);
                        id
                    }
                    parent => self.forest.id_of(parent),
                };
                let skipped = match node.extent {
                    Extent::Box(..) => self.left_out(at).0,
                    Extent::Point => 0..0,
                };
                let fade = self.stepped(at, target);
                self.out.waiting.push(Waiting {
                    node: id,
                    parent,
                    fade,
                    yields,
                    skipped,
                });
                return;
            }
            NodeState::Absent => return,
        }
        let fade = self.settle(at, target);
        let later = match node.extent {
            Extent::Box(..) => {
                let (now, later) = self.left_out(at);
                self.out.lists.push(ListDraw {
                    node: id,
                    fade,
                    yields,
                    skipped: now,
                });
                later
            }
            Extent::Point => {
                self.out.drawn.push(NodeDraw {
                    node: id,
                    fade,
                    yields,
                });
                0..0
            }
        };
        if held {
            let then = match free > 0.0 {
                true => self.stepped(at, free),
                false => 0.0,
            };
            self.out.held.push(Held {
                node: id,
                then,
                yields: free_yields,
                skipped: later,
            });
        }
    }

    /// The instances of the list `at` left out as their own entity draws in their place, then those
    /// left out once the entities on their way are ready, as two ranges of `skipped`.
    // An entity that stands in for an instance draws by its own place, which is the instance's,
    // and its instance is left out while it draws at all; one with no model leaves it drawn.
    fn left_out(&mut self, at: u32) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let from = self.out.skipped.len();
        let mut later = Vec::new();
        let mut child = self.first_child(at);
        while child != NONE {
            let alone = self.by_distance(child);
            if let Some(instance) = self.node(child).instance.filter(|_| alone > 0.0) {
                match (self.state)(self.forest.id_of(child)) {
                    NodeState::Ready => self.out.skipped.push(instance),
                    NodeState::Loading => later.push(instance),
                    NodeState::Absent => {}
                }
            }
            child = self.next_sibling(child);
        }
        let mid = self.out.skipped.len();
        self.out.skipped.extend(later);
        (from..mid, mid..self.out.skipped.len())
    }
}

#[cfg(test)]
#[path = "streaming_tests.rs"]
mod tests;
