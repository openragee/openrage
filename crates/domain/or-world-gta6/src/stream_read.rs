//! A map body as the streamer holds it: the nodes it gives the level-of-detail forest, what draws
//! at each, and where each entity and each instance of its prop and grass lists stands.
//!
//! ⚠ A list of far stand-ins (`IsSLODList`) is a node, so that the lists under it find their
//! parent, and draws no model: its instances are cards, each of the species its id names.

use glam::{Mat4, Quat, Vec3};
use or_io::gta6::map::stream::{StreamGrassList, StreamLink, StreamMap};
use or_world::streaming::{Category, Extent, Kind, Level, Node, ParentRef};

use super::grass;
use crate::index::{ModelRef, WorldIndex, ALL_HOURS};

/// The rows of a 3×4 matrix: the model's axes, with the translation in the fourth column.
pub type Rows = [[f32; 4]; 3];

/// The flags of a prop list of the batch LOD level, and of the batch SLOD level.
const BATCH_LOD: u32 = 1 << 11;
const BATCH: u32 = BATCH_LOD | 1 << 12;

pub(super) fn rows_of(m: Mat4) -> Rows {
    let c = m.to_cols_array_2d();
    std::array::from_fn(|row| [c[0][row], c[1][row], c[2][row], c[3][row]])
}

pub(super) fn place(rows: &Rows, p: Vec3) -> Vec3 {
    let at = |r: [f32; 4]| r[0] * p.x + r[1] * p.y + r[2] * p.z + r[3];
    Vec3::new(at(rows[0]), at(rows[1]), at(rows[2]))
}

/// The length of the longest axis of `rows`, which a sphere's radius grows by.
pub(super) fn stretch(rows: &Rows) -> f32 {
    (0..3)
        .map(|axis| Vec3::new(rows[0][axis], rows[1][axis], rows[2][axis]).length())
        .fold(0.0, f32::max)
}

/// What draws at a node: its model, and the sphere of that model in its own space.
#[derive(Clone, Copy)]
pub(super) struct Drawn {
    /// `None` where no archetype or no body of the index gives one: the node draws nothing.
    pub(super) model: Option<ModelRef>,
    pub(super) centre: Vec3,
    pub(super) radius: f32,
    /// The hours it draws in, bit `h` for the hour from `h`: a time archetype's, else every hour.
    pub(super) hours: u32,
}

impl Drawn {
    /// Its model at the hour `hour`, 0 to 23: none outside the hours of a time archetype.
    pub(super) fn model_at(&self, hour: u32) -> Option<ModelRef> {
        self.model.filter(|_| (self.hours >> hour) & 1 == 1)
    }
}

/// Where the instances of a list stand.
pub(super) enum Instances {
    Placed(Vec<Rows>),
    /// As the body states them: an instance is placed when it draws.
    Grass(StreamGrassList),
    /// A list of far stand-ins: its instances draw as cards.
    Cards(super::slod::CardList),
}

/// A prop or grass list streamed in.
pub(super) struct List {
    pub(super) instances: Instances,
    /// Where an instance starts to fade and where it is gone, in metres from the camera; `None`
    /// for a list that draws whole.
    /// Scaled by the list's node's scale.
    pub(super) reach: Option<(f32, f32)>,
    /// The instance of its parent list each of its instances stands in for; empty for a list with none.
    pub(super) stands_in_for: Vec<u16>,
    /// A prop list of the batch LOD level: while it draws, the instances of its parent prop list
    /// it stands in for are left out, all of them where it names none.
    pub(super) batch_lod: bool,
    /// Each placed instance's tint, its `Tint`'s first three bytes over 255; empty for a list that states none.
    pub(super) tints: Vec<[f32; 3]>,
}

impl List {
    /// How opaque an instance `away` from the camera is with distances scaled by `scale`; `None`
    /// where it is gone.
    pub(super) fn fade(&self, away: f32, scale: f32) -> Option<f32> {
        let Some((start, end)) = self.reach else {
            return Some(1.0);
        };
        let (start, end) = (start * scale, end * scale);
        if away > end {
            return None;
        }
        // A list whose fade starts at its end or past it has no band: an instance is there or gone.
        let fade = match start < end {
            true => ((end - away) / (end - start)).min(1.0),
            false => 1.0,
        };
        (fade > 0.0).then_some(fade)
    }

    /// Call `draw` with where each instance that draws for a camera at `eye` stands, how opaque
    /// it is and the colour it is tinted by, less the instances `skipped` names in rising order.
    // ponytail: every instance of a list that draws is measured at each selection; a grid of the list's box if the walk shows.
    pub(super) fn each(
        &self,
        eye: Vec3,
        scale: f32,
        skipped: &[u32],
        mut draw: impl FnMut(Rows, f32, [f32; 3]),
    ) {
        match &self.instances {
            Instances::Placed(placed) => {
                for (at, rows) in placed.iter().enumerate() {
                    if skipped.binary_search(&(at as u32)).is_ok() {
                        continue;
                    }
                    let origin = Vec3::new(rows[0][3], rows[1][3], rows[2][3]);
                    // ⚠ The tint colours only a material that names no colour map: the renderer leaves it out of every other.
                    let tint = self.tints.get(at).copied().unwrap_or([1.0; 3]);
                    if let Some(fade) = self.fade(eye.distance(origin), scale) {
                        draw(*rows, fade, tint);
                    }
                }
            }
            Instances::Grass(list) => {
                for (at, instance) in list.instances.iter().enumerate() {
                    if skipped.binary_search(&(at as u32)).is_ok() {
                        continue;
                    }
                    let origin = Vec3::from(list.position_of(instance));
                    if let Some(fade) = self.fade(eye.distance(origin), scale) {
                        let rows = grass::rows(list, instance, origin);
                        draw(rows, fade, grass::tint(instance));
                    }
                }
            }
            // Cards are drawn by the card pass, never as instances.
            Instances::Cards(_) => {}
        }
    }
}

/// A map streamed in.
pub(super) struct Loaded {
    /// One for each node: the entities, then the prop lists, then the grass lists.
    pub(super) drawn: Vec<Drawn>,
    /// Where each entity stands.
    pub(super) entities: Vec<Rows>,
    /// The prop lists, then the grass lists.
    pub(super) lists: Vec<List>,
    /// The interiors the map places, each with its contents in the world.
    pub(super) interiors: Vec<crate::interior::Placed>,
}

/// What a worker read of a map: its nodes for the forest, and what draws at each.
pub struct ReadMap {
    pub(super) nodes: Vec<Node>,
    pub(super) loaded: Loaded,
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

/// `stated` where it is above 0, else the archetype's, in whole metres.
fn distance(stated: f32, archetype: f32) -> f32 {
    (if stated > 0.0 { stated } else { archetype }).trunc()
}

/// The node of a list in the box `min` to `max`, scaled as a prop list: it is measured to the box's
/// centre, and one that states no distance takes its archetype's.
fn list_node(
    (min, max): (Vec3, Vec3),
    (lod_dist, archetype_dist): (f32, f32),
    (child_lod_dist, num_children): (f32, u32),
    parent: Option<ParentRef>,
    (level, category): (u8, Category),
) -> Node {
    Node {
        num_children,
        batch: false,
        position: (min + max) / 2.0,
        extent: Extent::Box(min, max),
        lod_dist: distance(lod_dist, archetype_dist),
        child_lod_dist: child_lod_dist.trunc(),
        parent,
        instance: None,
        level: Level::from_raw(level),
        kind: Kind::Props,
        category,
        radius: (max - min).length() / 2.0,
    }
}

/// The nodes of `map` and what draws at each, by the archetypes and models of `index`.
pub fn read_map(index: &WorldIndex, map: &StreamMap) -> ReadMap {
    let count = map.entities.len() + map.prop_lists.len() + map.grass.len();
    let (mut nodes, mut drawn) = (Vec::with_capacity(count), Vec::with_capacity(count));
    let of = |word: u64| {
        let archetype = index.archetype(word);
        let model = archetype.and_then(|a| index.model(a));
        let (centre, radius, lod_dist, hours, category) =
            archetype.map_or((Vec3::ZERO, 0.0, 0.0, ALL_HOURS, Category::Other), |a| {
                let category = Category::of(a.load_flags);
                (
                    Vec3::from(a.centre),
                    a.radius,
                    a.lod_dist,
                    a.hours,
                    category,
                )
            });
        (
            Drawn {
                model,
                centre,
                radius,
                hours,
            },
            lod_dist,
            category,
        )
    };
    let mut entities = Vec::with_capacity(map.entities.len());
    for e in &map.entities {
        let (what, archetype_dist, category) = of(e.archetype_word);
        let (parent, instance) = link(map, map.name, e.parent);
        nodes.push(Node {
            position: Vec3::from(e.position),
            extent: Extent::Point,
            // An entity that states no distance takes its archetype's, and no orphan draws farther.
            lod_dist: distance(e.lod_dist, archetype_dist),
            child_lod_dist: e.child_lod_dist.trunc(),
            parent,
            instance,
            level: Level::from_raw(e.lod_level),
            kind: Kind::Entity,
            category,
            // ponytail: the sphere's radius alone, the index keeps no box to measure the sphere's
            // offset from; add `|M·(box centre − sphere centre)|` once the archetype row has one.
            radius: what.radius,
            num_children: e.num_children,
            batch: false,
        });
        // An interior has no model of its own: its contents are placed by `interior::read_placed`.
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
    let mut lists = Vec::with_capacity(map.prop_lists.len() + map.grass.len());
    for list in &map.prop_lists {
        let (what, archetype_dist, category) = of(list.archetype_word);
        let (parent, _) = link(map, map.name, list.parent);
        let corners = (
            Vec3::from(list.batch_aabb.min),
            Vec3::from(list.batch_aabb.max),
        );
        nodes.push(Node {
            batch: list.flags & BATCH != 0,
            ..list_node(
                corners,
                (list.lod_dist, archetype_dist),
                (list.child_lod_dist, list.num_children),
                parent,
                (list.lod_level, category),
            )
        });
        drawn.push(what);
        // A list states each instance as its three axes, each with one part of the translation.
        let rows = |a: &[[f32; 4]; 3]| -> Rows {
            std::array::from_fn(|row| [a[0][row], a[1][row], a[2][row], a[row][3]])
        };
        // ⚠ Only a list that nothing stands in for draws by its instances' own distance: where
        // a parent draws a list's far instances, leaving them out one by one would leave holes.
        let alone = parent.is_none()
            && Level::from_raw(list.lod_level) == Level::OrphanHd
            && list.instance_lod_dist > 0.0;
        lists.push(List {
            instances: Instances::Placed(list.instances.iter().map(rows).collect()),
            reach: alone.then_some((list.instance_lod_dist, list.instance_lod_dist)),
            stands_in_for: list.parent_instances.clone(),
            batch_lod: list.flags & BATCH_LOD != 0,
            tints: (list.tints.iter())
                .map(|t| [t[0], t[1], t[2]].map(|c| f32::from(c) / 255.0))
                .collect(),
        });
    }
    for list in &map.grass {
        let (what, archetype_dist, category) = of(list.archetype_word);
        let corners = (
            Vec3::from(list.batch_aabb.min),
            Vec3::from(list.batch_aabb.max),
        );
        // A grass list names no parent and states no distance for children.
        let node = list_node(
            corners,
            (list.lod_dist, archetype_dist),
            (0.0, list.num_children),
            None,
            (list.lod_level, category),
        );
        // A list of far stand-ins draws at least as far as its farthest instance does, and scales
        // as an entity; a plain one keeps the distance it states.
        nodes.push(match list.slod {
            true => Node {
                lod_dist: (node.lod_dist).max((node.radius + list.instance_lod_dist).trunc()),
                kind: Kind::Entity,
                ..node
            },
            false => Node {
                kind: Kind::Grass,
                ..node
            },
        });
        drawn.push(Drawn {
            model: what.model.filter(|_| !list.slod),
            ..what
        });
        lists.push(List {
            instances: match list.slod {
                true => Instances::Cards(super::slod::CardList::of(list, index)),
                false => Instances::Grass(list.clone()),
            },
            reach: Some((list.lod_fade_start_dist, list.instance_lod_dist)),
            stands_in_for: Vec::new(),
            batch_lod: false,
            tints: Vec::new(),
        });
    }
    ReadMap {
        nodes,
        loaded: Loaded {
            drawn,
            entities,
            lists,
            interiors: Vec::new(),
        },
    }
}
