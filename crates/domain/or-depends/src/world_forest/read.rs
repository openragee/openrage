//! A map and an interior as the forest takes them: a node an entity, in the file's own order, and
//! beside each node what it places.

use glam::{Affine3A, Vec3};
use or_world::streaming::{Category, Extent, Kind, Node, ParentRef};

use super::PRESTREAM;
use or_world::{Entity, MapData, MapTypes};

use super::{interior_key, ModelFiles, NoModel, Unread};
use crate::world_index::{self, Archetype, Map, WorldIndex, WorldSource};

/// What an entity is to a selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum What {
    /// The files that draw it.
    Model(ModelFiles),
    /// Nothing says what draws it: it stands as a box.
    Box(NoModel),
    /// An interior's instance: it has no model, and its entities stand under it.
    Interior,
    /// Its archetype names no asset: nothing draws.
    Nothing,
}

/// What a node places.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub archetype: u32,
    /// From the model's own space to the world.
    pub to_world: Affine3A,
    pub what: What,
}

/// A map as read: `nodes[i]` and `placed[i]` are entity `i` of the file.
#[derive(Debug, Default)]
pub struct ReadMap {
    pub nodes: Vec<Node>,
    pub placed: Vec<Placed>,
    /// The entities that are an interior's instance, by their place.
    pub interiors: Vec<u32>,
    /// Entities that state no level, or one that is none.
    pub unleveled: u32,
    /// Entities that name a parent in a parent map the mount lacks.
    pub links_lost: u32,
}

impl ReadMap {
    /// The bytes it holds.
    pub fn bytes(&self) -> u64 {
        let of = |len: usize, each: usize| (len * each) as u64;
        of(self.nodes.len(), std::mem::size_of::<Node>())
            + of(self.placed.len(), std::mem::size_of::<Placed>())
            + of(self.interiors.len(), std::mem::size_of::<u32>())
    }
}

/// What draws the archetype named `archetype`, as the index's row of it says.
pub fn what_of(index: &WorldIndex, archetype: u32, interior: bool) -> What {
    match index.archetype(archetype) {
        _ if archetype == 0 => What::Box(NoModel::Unnamed),
        None => What::Box(NoModel::Undeclared),
        Some(_) if interior => What::Interior,
        Some(row) if row.asset_name == 0 => What::Nothing,
        Some(row) => What::Model(ModelFiles {
            drawable: row.asset_name,
            dictionary: Some(row.drawable_dictionary).filter(|name| *name != 0),
            textures: Some(row.texture_dictionary).filter(|name| *name != 0),
        }),
    }
}

/// How far a camera can stand from anything in the box `map` streams by while the map is wanted, in metres.
fn across(map: &Map) -> f32 {
    let (min, max) = super::map_box(map).map_or((Vec3::ZERO, Vec3::ZERO), |boxes| {
        let (s, e) = (boxes.streaming, boxes.entities);
        (s.0.min(e.0), s.1.max(e.1))
    });
    (max - min + Vec3::splat(2.0 * PRESTREAM)).length()
}

/// The node of `entity`, which stands at `to_world` and draws to `unstated` where neither it nor its archetype states a distance.
fn node_of(
    entity: &Entity<'_>,
    archetype: Option<&Archetype>,
    to_world: &Affine3A,
    unstated: f32,
    (parent, instance): (Option<ParentRef>, Option<u32>),
) -> Node {
    let stated = entity.lod_dist();
    // ⚠ Half the box's diagonal under the largest scale: a bound of the model's sphere, which the
    // index does not keep.
    let radius = archetype.and_then(|a| a.span).map_or(0.0, |(min, max)| {
        (max - min).length() / 2.0 * entity.scale().abs().max_element()
    });
    Node {
        position: Vec3::from(to_world.translation),
        extent: Extent::Point,
        lod_dist: match archetype {
            _ if stated > 0.0 => stated,
            Some(archetype) => archetype.lod_dist,
            None => unstated,
        },
        child_lod_dist: entity.child_lod_dist(),
        parent,
        instance,
        level: entity.level().unwrap_or_default(),
        kind: Kind::Entity,
        category: Category::Other,
        radius,
        num_children: entity.num_children(),
        batch: false,
    }
}

/// Where `entity` stands, by what it states of itself.
fn placement(entity: &Entity<'_>) -> Affine3A {
    Affine3A::from_scale_rotation_translation(entity.scale(), entity.rotation(), entity.position())
}

/// The entities of `map`, read from the archive that lists it.
pub fn map(index: &WorldIndex, source: &dyn WorldSource, map: &Map) -> Result<ReadMap, Unread> {
    let bytes = (source.open(map.archive, &map.logical)).ok_or(Unread::Absent)?;
    let root = world_index::parse(&bytes).ok_or(Unread::NotAMap)?;
    let data = MapData::new(&root).ok_or(Unread::NotAMap)?;
    entities_of(index, map, &data)
}

/// The entities of `map` as `data`, its file read, states them.
pub fn entities_of(index: &WorldIndex, map: &Map, data: &MapData<'_>) -> Result<ReadMap, Unread> {
    // ⚠ The read is lenient and a node's place is its entity's: past an entity that did not read,
    // every index would name the entity before the one it means.
    if !data.entities_are_whole() {
        return Err(Unread::EntityUnread);
    }
    let own = u64::from(map.hash);
    let parent_map = index.parent_of(map).map(|parent| u64::from(parent.hash));
    let unstated = across(map);
    let mut read = ReadMap::default();
    for (at, entity) in data.entities().enumerate() {
        let archetype = entity.archetype();
        let interior = entity.is_interior_instance();
        // ⚠ The index a parent is named by is a place in the parent map's entities where bit 3 of
        // the entity's flags is set, and in this map's own where it is not.
        let parent = entity.parent_index().and_then(|node| {
            let of = match entity.lod_in_parent_map() {
                true => parent_map,
                false => Some(own),
            };
            read.links_lost += u32::from(of.is_none());
            Some(ParentRef {
                map: of?,
                node: u32::try_from(node).ok()?,
            })
        });
        let to_world = placement(&entity);
        read.unleveled += u32::from(entity.level().is_none());
        read.nodes.push(node_of(
            &entity,
            index.archetype(archetype),
            &to_world,
            unstated,
            (parent, None),
        ));
        read.placed.push(Placed {
            archetype,
            to_world,
            what: what_of(index, archetype, interior),
        });
        if interior {
            read.interiors.push(at as u32);
        }
    }
    Ok(read)
}

/// The entities of the interior `archetype`, each placed through the instance that stands at `instance` as entity `at` of the map `map`.
///
/// ⚠ An entity of an interior with no parent of its own stands under the instance as one of its
/// instances does under a list: it draws by its own place and distance, never the interior's.
pub fn interior(
    index: &WorldIndex,
    source: &dyn WorldSource,
    archetype: u32,
    (map, at): (u32, u32),
    (instance, reach): (&Affine3A, f32),
) -> Result<ReadMap, Unread> {
    let row = index.archetype(archetype).ok_or(Unread::Undeclared)?;
    let file = index.type_file(row.type_file).ok_or(Unread::Undeclared)?;
    let bytes = (source.open(file.archive, &file.logical)).ok_or(Unread::Absent)?;
    let root = world_index::parse(&bytes).ok_or(Unread::NotATypeFile)?;
    let types = MapTypes::new(&root).ok_or(Unread::NotATypeFile)?;
    let declared = types.archetypes().find(|a| a.name() == archetype);
    let interior = declared
        .and_then(|a| a.interior())
        .ok_or(Unread::NotAnInterior)?;
    if !interior.entities_are_whole() {
        return Err(Unread::EntityUnread);
    }
    let own = interior_key(map, at);
    let under = ParentRef {
        map: u64::from(map),
        node: at,
    };
    let mut read = ReadMap::default();
    for (place, entity) in interior.entities().enumerate() {
        let name = entity.archetype();
        let link = match entity.parent_index().map(u32::try_from) {
            Some(Ok(node)) => (Some(ParentRef { map: own, node }), None),
            _ => (Some(under), Some(place as u32)),
        };
        let to_world = *instance * placement(&entity);
        read.unleveled += u32::from(entity.level().is_none());
        (read.nodes).push(node_of(
            &entity,
            index.archetype(name),
            &to_world,
            reach,
            link,
        ));
        read.placed.push(Placed {
            archetype: name,
            to_world,
            what: what_of(index, name, false),
        });
    }
    Ok(read)
}
