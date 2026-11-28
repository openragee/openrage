//! Recomputing the four extents a `.ymap` stores.
//!
//! The engine streams a map by its `streamingExtents` and culls it by its
//! `entitiesExtents`, so an edit that moves an entity outside them produces a
//! file that loads and renders nothing.
//!
//! ⚠ **The entity half is exact and the streaming half is not.** An
//! entity's contribution is its archetype box, scaled, rotated and
//! translated — arithmetic with one right answer. What a *streaming* box
//! adds beyond that differs per content kind and the published reference
//! marks several of its margins as guesses, so [`Extents::streaming`] is
//! offered as a proposal and [`agreement`] is how far it is from what the
//! game shipped, not a claim to have reproduced it.

use glam::Vec3;
use or_world::Aabb;

use crate::archetypes::ArchetypeIndex;
use crate::map_data::MapData;

/// What a recomputation produced, and how much of it it could see.
#[derive(Debug, Clone, Copy)]
pub struct Extents {
    pub entities: Aabb,
    pub streaming: Aabb,
    /// Entities whose archetype the index could not resolve.
    pub unresolved: usize,
    /// Entities placed using a resolved archetype box.
    pub resolved: usize,
}

impl Extents {
    /// Whether every entity contributed a real box.
    ///
    /// ⚠ A map with one unresolved archetype has extents that are a lower
    /// bound, not an answer, and nothing downstream should compare them.
    pub fn is_complete(&self) -> bool {
        self.unresolved == 0
    }
}

/// Recompute both extents for a map, using `index` for archetype bounds.
pub fn compute(map: &MapData<'_>, index: &ArchetypeIndex) -> Extents {
    let mut entities = Aabb::default();
    let mut streaming = Aabb::default();
    let mut unresolved = 0;
    let mut resolved = 0;

    for entity in map.entities() {
        let Some(bounds) = index.bounds_of(entity.archetype()) else {
            unresolved += 1;
            continue;
        };
        resolved += 1;

        let mut lod_dist = entity.lod_dist();
        if lod_dist <= 0.0 {
            lod_dist = index.lod_dist_of(entity.archetype()).unwrap_or(0.0);
        }

        for corner in entity.place(bounds.min, bounds.max) {
            entities.add_point(corner);
        }
        // ⚠ By the size of the scale: an entity mirrored by a scale below nought is drawn as far as one that is not.
        let spread = lod_dist / entity.scale().abs().max_element().max(f32::MIN_POSITIVE);
        for corner in entity.place(bounds.min - spread, bounds.max + spread) {
            streaming.add_point(corner);
        }
    }

    for batch in map.grass_batches() {
        if let Some((min, max)) = batch.bounds() {
            entities.add_box(min, max);
            let lod = batch.lod_dist();
            streaming.add_box(min - lod, max + lod);
        }
    }

    for generator in map.car_generators() {
        let Some(position) = generator.position() else {
            continue;
        };
        let length = generator.perpendicular_length();
        entities.add_box(position - length, position + length);
        streaming.add_box(position - length * 2.0, position + length * 2.0);
    }

    for occluder in map.box_occluders() {
        let half = occluder.size.length() * 0.5;
        entities.add_box(occluder.centre - half, occluder.centre + half);
        streaming.add_box(occluder.centre - half, occluder.centre + half);
    }

    for (min, max) in map.occlude_model_bounds() {
        entities.add_box(min, max);
        streaming.add_box(min, max);
    }

    Extents {
        entities,
        streaming,
        unresolved,
        resolved,
    }
}

/// How far a recomputed box is from a stored one, in world units.
///
/// The answer is the largest single-axis disagreement over both corners, so
/// zero means identical and a small number means a rounding-scale difference
/// rather than a structural one.
pub fn agreement(computed: &Aabb, stored: (Vec3, Vec3)) -> f32 {
    if computed.is_empty() {
        return f32::INFINITY;
    }
    let (min, max) = stored;
    (computed.min - min)
        .abs()
        .max((computed.max - max).abs())
        .max_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::value::HashedString;
    use or_io::meta::{MetaInstance, Value};

    fn map_with_one_entity(rotation: [f32; 4], position: [f32; 3]) -> MetaInstance {
        let mut ent = MetaInstance::new("CEntityDef");
        ent.set("archetypeName", Value::Hash(HashedString::new("prop_x")));
        ent.set("position", Value::Vec3(position));
        ent.set("rotation", Value::Vec4(rotation));
        ent.set("scaleXY", Value::Float(1.0));
        ent.set("scaleZ", Value::Float(1.0));
        ent.set("lodDist", Value::Float(0.0));
        let mut root = MetaInstance::new("CMapData");
        root.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(ent)))]),
        );
        root
    }

    fn index_with(min: [f32; 3], max: [f32; 3]) -> ArchetypeIndex {
        let mut arch = MetaInstance::new("CBaseArchetypeDef");
        arch.set("name", Value::Hash(HashedString::new("prop_x")));
        arch.set("bbMin", Value::Vec3(min));
        arch.set("bbMax", Value::Vec3(max));
        arch.set("lodDist", Value::Float(0.0));
        let mut types = MetaInstance::new("CMapTypes");
        types.set(
            "archetypes",
            Value::Array(vec![Value::Pointer(Some(Box::new(arch)))]),
        );
        let mut index = ArchetypeIndex::new();
        index.add(crate::map_types::MapTypes::new(&types).unwrap());
        index
    }

    #[test]
    fn an_unrotated_entity_contributes_its_own_box() {
        let root = map_with_one_entity([0.0, 0.0, 0.0, 1.0], [10.0, 0.0, 0.0]);
        let map = MapData::new(&root).unwrap();
        let out = compute(&map, &index_with([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]));
        assert!(out.is_complete());
        assert_eq!(out.entities.min, Vec3::new(9.0, -2.0, -3.0));
        assert_eq!(out.entities.max, Vec3::new(11.0, 2.0, 3.0));
    }

    /// ⚠ A quarter turn about Z swaps the box's X and Y half-extents. With
    /// the wrong rotation convention it swaps them the same way, which is
    /// exactly why an AABB check cannot tell the conventions apart on a
    /// symmetric box — the corpus measurement uses asymmetric ones.
    #[test]
    fn a_quarter_turn_swaps_the_horizontal_extents() {
        let q = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let root = map_with_one_entity([q.x, q.y, q.z, q.w], [0.0, 0.0, 0.0]);
        let map = MapData::new(&root).unwrap();
        let out = compute(&map, &index_with([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]));
        assert!((out.entities.max.x - 2.0).abs() < 1e-5);
        assert!((out.entities.max.y - 1.0).abs() < 1e-5);
        assert!((out.entities.max.z - 3.0).abs() < 1e-5);
    }

    #[test]
    fn an_unresolved_archetype_is_reported_rather_than_guessed() {
        let root = map_with_one_entity([0.0, 0.0, 0.0, 1.0], [10.0, 0.0, 0.0]);
        let map = MapData::new(&root).unwrap();
        let out = compute(&map, &ArchetypeIndex::new());
        assert_eq!(out.unresolved, 1);
        assert_eq!(out.resolved, 0);
        assert!(!out.is_complete());
        assert!(out.entities.is_empty());
    }

    #[test]
    fn an_empty_box_never_agrees_with_a_stored_one() {
        let empty = Aabb::default();
        assert!(agreement(&empty, (Vec3::ZERO, Vec3::ZERO)).is_infinite());
        assert!(empty.expanded(100.0).is_empty());
    }

    #[test]
    fn agreement_is_the_worst_single_axis() {
        let a = Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(1.0, 1.0, 1.0),
        };
        assert_eq!(agreement(&a, (Vec3::ZERO, Vec3::ONE)), 0.0);
        assert_eq!(agreement(&a, (Vec3::ZERO, Vec3::new(1.0, 1.0, 4.0))), 3.0);
    }
}
