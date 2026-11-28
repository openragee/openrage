//! What a map set places, as boxes in the world: the query behind the plan view and the 3D scene, with no renderer.
//!
//! Each entity of each map file of a set is a box: the bounding box its archetype's type file gives, turned and scaled as the entity
//! says, at its position. An archetype no type file of the set declares has no box to give, and is drawn as a one metre cube and said to be
//! one ([`Placed::declared`]). A model of the set that an archetype names is found by name ([`Placed::model`]).
//!
//! ⚠ The boxes are the world model's (`or_world::transform`), so the conventions of the file hold: an entity's rotation is stored
//! conjugated and an interior instance's is not.

use std::collections::BTreeMap;

use glam::{Quat, Vec3};
use or_world_gta5::Placement;

use crate::facts::{Facts, Name, Role};
use crate::model::MapSet;
use or_world::gizmo::Obb;

/// The box an entity of an undeclared archetype is drawn as, half a metre either way.
pub const UNKNOWN_HALF: f32 = 0.5;

/// One entity as a box.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The map source it is in, a folder of the module with `/`.
    pub source: String,
    /// Its index among the entities of that map file.
    pub index: usize,
    pub archetype: Name,
    /// Whether a type file of the set gave its box.
    pub declared: bool,
    /// The model source of the set its archetype names, when one does.
    pub model: Option<String>,
    pub obb: Obb,
    pub interior: bool,
    pub lod_dist: f32,
    pub parent_index: i32,
    pub guid: u32,
    /// The level of detail it is for, as the file names it.
    pub lod_level: Option<String>,
}

/// What a set places.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub placed: Vec<Placed>,
    /// The lowest and highest corner of every box, none for a set that places nothing.
    pub bounds: Option<(Vec3, Vec3)>,
}

impl Scene {
    /// The boxes in the order of [`Scene::placed`].
    pub fn obbs(&self) -> Vec<Obb> {
        self.placed.iter().map(|p| p.obb).collect()
    }

    /// The box of the entity `index` of the map file `source`.
    pub fn find(&self, source: &str, index: usize) -> Option<usize> {
        self.placed
            .iter()
            .position(|p| p.source == source && p.index == index)
    }
}

/// What a set places, read from what the disk said of its sources.
pub fn scene(set: &MapSet, facts: &Facts) -> Scene {
    // The archetype boxes and assets of the set's type files; the first to declare a name wins, as the game would pick one.
    let mut declared: BTreeMap<u32, (Vec3, Vec3, Name, Name)> = BTreeMap::new();
    for path in &set.sources {
        let Some(types) = facts.source(path).and_then(|s| s.types()) else {
            continue;
        };
        for archetype in &types.archetypes {
            declared.entry(archetype.name.hash).or_insert_with(|| {
                (
                    Vec3::from(archetype.bb_min),
                    Vec3::from(archetype.bb_max),
                    archetype.asset_name.clone(),
                    archetype.drawable_dictionary.clone(),
                )
            });
        }
    }
    let models: Vec<(u32, &String)> = set
        .sources
        .iter()
        .filter(|path| facts.source(path).and_then(|s| s.role()) == Some(Role::Model))
        .filter_map(|path| {
            let name = &facts.source(path)?.name;
            Some((
                or_io::string_hash(name.to_ascii_lowercase().as_bytes()),
                path,
            ))
        })
        .collect();

    let mut out = Scene::default();
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for path in &set.sources {
        let Some(map) = facts.source(path).and_then(|s| s.map()) else {
            continue;
        };
        for (index, entity) in map.entities.iter().enumerate() {
            let known = declared.get(&entity.archetype.hash);
            let (min, max) = known.map_or(
                (Vec3::splat(-UNKNOWN_HALF), Vec3::splat(UNKNOWN_HALF)),
                |(min, max, ..)| (*min, *max),
            );
            let placement = if entity.interior {
                Placement::InteriorInstance
            } else {
                Placement::Entity
            };
            let obb = Obb {
                position: Vec3::from(entity.position),
                rotation: normalised(placement.rotation_of(entity.rotation)),
                scale: or_world::transform::scale_of(entity.scale_xy, entity.scale_z),
                min,
                max,
            };
            for corner in obb.corners() {
                lo = lo.min(corner);
                hi = hi.max(corner);
            }
            let model = known.and_then(|(_, _, asset, dictionary)| {
                [asset.hash, entity.archetype.hash, dictionary.hash]
                    .into_iter()
                    .filter(|hash| *hash != 0)
                    .find_map(|hash| models.iter().find(|(h, _)| *h == hash))
                    .map(|(_, path)| (*path).clone())
            });
            out.placed.push(Placed {
                source: path.clone(),
                index,
                archetype: entity.archetype.clone(),
                declared: known.is_some(),
                model,
                obb,
                interior: entity.interior,
                lod_dist: entity.lod_dist,
                parent_index: entity.parent_index,
                guid: entity.guid,
                lod_level: entity.lod_level.clone(),
            });
        }
    }
    out.bounds = (!out.placed.is_empty()).then_some((lo, hi));
    out
}

fn normalised(q: Quat) -> Quat {
    if q.is_finite() && q.length_squared() > f32::EPSILON {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}
