//! The placements of one model, nearest first, each with what only its map file states: read from the maps the placement index
//! names, a map at a time, under a budget.

use std::collections::{HashMap, VecDeque};

use glam::Vec3;
use or_maps::edit::Pose;
use or_maps::script_text::{self, heading_of, ScriptForm, ScriptObject};
use or_world::{field, keys, Entity, MapData};

use super::Install;
use crate::placement::{Placed, PlacementIndex};
use crate::world_index::{parse, WorldIndex, WorldSource};

/// How much of a model's placements is listed, and how much is read to say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// The most placements listed.
    pub placements: usize,
    /// Bytes of map files: a map is opened while what was read is under it, so the last one opened may pass it.
    pub bytes: u64,
}

/// Why a placement is listed without its entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unread {
    #[error("the budget of map files was read before its map")]
    Budget,
    #[error("the mount holds no map of that name")]
    NoMap,
    #[error("the archive that lists its map gave no file")]
    NoFile,
    #[error("its file does not read as a map")]
    NotAMap,
    #[error("its map places no such entity there")]
    Moved,
}

/// What a map file states of a placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Detail {
    Entity {
        /// Its place in the map's `entities`.
        index: usize,
        guid: u32,
        /// Degrees from 0 up to 360, the one turn about the vertical axis a script's heading holds.
        heading: f32,
    },
    /// A grass batch, which is no entity.
    Batch,
    Unread(Unread),
}

/// One placement of a model.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementFound {
    /// The `CMapData::name` of the map that places it.
    pub map: u32,
    /// The map file of that name, where the mount holds one.
    pub map_name: Option<String>,
    pub logical: Option<String>,
    pub placed: Placed,
    /// `None` for a grass batch, which the placement index holds no position of.
    pub position: Option<Vec3>,
    /// Metres from the view, where there is a view and a position.
    pub distance: Option<f32>,
    pub detail: Detail,
}

impl PlacementFound {
    /// Degrees, where its map was read.
    pub fn heading(&self) -> Option<f32> {
        match self.detail {
            Detail::Entity { heading, .. } => Some(heading),
            _ => None,
        }
    }

    /// Where it stands as `form` writes a place, with its heading where its map was read; `None` for a grass batch.
    pub fn script(
        &self,
        form: &ScriptForm,
        decimals: usize,
    ) -> Result<Option<String>, script_text::Error> {
        self.position
            .map(|at| form.place(at, self.heading(), decimals))
            .transpose()
    }
}

/// The placements of one model, nearest first.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPlacements {
    pub model: u32,
    /// How many the placement index holds, of which `listed` is the first.
    pub known: usize,
    /// Entities nearest the view first, in the index's order with no view, then grass batches.
    pub listed: Vec<PlacementFound>,
    /// Map files opened.
    pub maps_read: usize,
    pub bytes_read: u64,
}

impl ModelPlacements {
    /// The placements listed that have a position, as the list `form` writes, each called `name`.
    pub fn script(
        &self,
        name: &str,
        form: &ScriptForm,
        decimals: usize,
    ) -> Result<String, script_text::Error> {
        let objects: Vec<ScriptObject> = (self.listed.iter())
            .filter_map(|placement| {
                Some(ScriptObject {
                    name: name.to_string(),
                    position: placement.position?,
                    heading: placement.heading(),
                })
            })
            .collect();
        form.objects(&objects, decimals)
    }
}

/// An entity of a map that places the model asked for.
#[derive(Debug, Clone, Copy)]
struct Stood {
    index: usize,
    guid: u32,
    heading: f32,
}

/// A position as the bits a file holds, which is what one placement is told from another by.
fn bits(position: Vec3) -> [u32; 3] {
    position.to_array().map(f32::to_bits)
}

/// The entities of one map that place one model, by where they stand, each list in the map's order.
type StoodByPlace = HashMap<[u32; 3], VecDeque<Stood>>;

fn stood_in(data: &MapData<'_>, model: u32) -> StoodByPlace {
    let mut stood = StoodByPlace::new();
    // ⚠ Numbered over the array as the file holds it, which is how a `parentIndex` numbers it: a pointer to nothing is no
    // entity and still a place of the array.
    let held = field::array_at(data.instance(), keys::ENTITIES).iter();
    for (index, value) in held.enumerate() {
        let Some(entity) = field::as_struct(value).map(Entity::new) else {
            continue;
        };
        if entity.archetype() != model {
            continue;
        }
        let pose = Pose {
            position: entity.position(),
            rotation: entity.rotation(),
            scale_xy: 1.0,
            scale_z: 1.0,
        };
        stood
            .entry(bits(pose.position))
            .or_default()
            .push_back(Stood {
                index,
                guid: entity.guid(),
                heading: heading_of(&pose),
            });
    }
    stood
}

/// The placements `install` holds of `model`, nearest `near` first, each with its entity as its map file states it, read from `source` while `budget` allows.
pub fn placements_of(
    model: u32,
    install: &Install<'_>,
    source: &dyn WorldSource,
    near: Option<Vec3>,
    budget: &Budget,
) -> ModelPlacements {
    let world = install.world;
    let known = install.placements.of(model);
    let mut order: Vec<_> = (known.iter())
        .map(|placement| {
            let batch = placement.placed == Placed::GrassBatch;
            let distance = near
                .filter(|_| !batch)
                .map(|near| placement.position.distance(near));
            (placement, batch, distance)
        })
        .collect();
    // A stable sort, so with no view the entities keep the order the index holds them in.
    order.sort_by(|a, b| {
        let by_distance = match (a.2, b.2) {
            (Some(a), Some(b)) => a.total_cmp(&b),
            _ => std::cmp::Ordering::Equal,
        };
        a.1.cmp(&b.1).then(by_distance)
    });
    order.truncate(budget.placements);

    let (mut maps_read, mut bytes_read) = (0usize, 0u64);
    // ⚠ One map's tree at a time: each is folded into the entities that place the model and dropped before the next is opened.
    let mut read: HashMap<u32, Result<StoodByPlace, Unread>> = HashMap::new();
    let mut listed = Vec::with_capacity(order.len());
    for (placement, batch, distance) in order {
        let map = world.map(placement.map);
        let detail = if batch {
            Detail::Batch
        } else {
            let stood = read.entry(placement.map).or_insert_with(|| {
                let map = map.ok_or(Unread::NoMap)?;
                if bytes_read >= budget.bytes {
                    return Err(Unread::Budget);
                }
                let bytes = (source.open(map.archive, &map.logical)).ok_or(Unread::NoFile)?;
                maps_read += 1;
                bytes_read += bytes.len() as u64;
                let root = parse(&bytes).ok_or(Unread::NotAMap)?;
                let data = MapData::new(&root).ok_or(Unread::NotAMap)?;
                Ok(stood_in(&data, model))
            });
            match stood {
                Err(why) => Detail::Unread(*why),
                // ⚠ By its position to the bit, and each entity once: two of a model at one place are two rows.
                Ok(stood) => (stood.get_mut(&bits(placement.position)))
                    .and_then(VecDeque::pop_front)
                    .map_or(Detail::Unread(Unread::Moved), |stood| Detail::Entity {
                        index: stood.index,
                        guid: stood.guid,
                        heading: stood.heading,
                    }),
            }
        };
        listed.push(PlacementFound {
            map: placement.map,
            map_name: map.map(|map| map.name.clone()),
            logical: map.map(|map| map.logical.clone()),
            placed: placement.placed,
            position: (!batch).then_some(placement.position),
            distance,
            detail,
        });
    }
    ModelPlacements {
        model,
        known: known.len(),
        listed,
        maps_read,
        bytes_read,
    }
}

/// Fold every map of `world` that read as one into `index`, each opened from the archive that lists it and dropped before the next; how many gave no map this time.
pub fn place_maps(
    world: &WorldIndex,
    source: &dyn WorldSource,
    index: &mut PlacementIndex,
) -> usize {
    let mut unread = 0;
    for map in world.maps().iter().filter(|map| map.read) {
        let root = (source.open(map.archive, &map.logical)).and_then(|bytes| parse(&bytes));
        match root.as_ref().and_then(MapData::new) {
            Some(data) => index.add(&data),
            None => unread += 1,
        }
    }
    unread
}

#[cfg(test)]
mod tests {
    use or_io::meta::{MetaInstance, Value};

    use super::*;
    use crate::tests_support::map_placing;

    #[test]
    fn an_entity_is_numbered_by_its_place_in_the_array_whatever_stands_before_it() {
        // An array a tool wrote with a pointer to nothing and a value that is no entity in it: the entity after them keeps
        // its place in the array.
        let mut root = map_placing("m", &["prop_x", "prop_y"], &[]);
        let Some(Value::Array(entities)) = root.get_mut("entities") else {
            panic!("the map has an array of entities");
        };
        entities.insert(1, Value::Pointer(None));
        entities.insert(0, Value::UInt(7));
        let held = entities.len();
        let places: Vec<Option<&MetaInstance>> =
            entities.iter().map(or_world::field::as_struct).collect();
        let wanted = places.iter().rposition(Option::is_some).expect("an entity");
        assert_eq!((held, wanted), (4, 3));

        let data = MapData::new(&root).expect("a map");
        let model = or_io::string_hash(b"prop_y");
        let stood: Vec<Stood> = stood_in(&data, model).into_values().flatten().collect();
        let indexes: Vec<usize> = stood.iter().map(|stood| stood.index).collect();
        assert_eq!(indexes, [wanted]);
    }
}
