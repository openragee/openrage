//! Resolving an entity's `archetypeName` to the bounds it places.
//!
//! A `.ymap` stores a hash and the box it stands for is in some `.ytyp`,
//! usually in another archive, so a caller with only the one file gets `None`
//! rather than a guess.

use std::collections::HashMap;

use crate::map_types::{Bounds, MapTypes};

/// Archetype bounds by name hash, gathered from any number of `.ytyp`s.
#[derive(Debug, Default, Clone)]
pub struct ArchetypeIndex {
    bounds: HashMap<u32, Bounds>,
    lod_dist: HashMap<u32, f32>,
    /// Which `.ytyp` declared each archetype, by that file's own `name`: what a
    /// manifest's `itypDepArray` lists.
    owner: HashMap<u32, u32>,
    redefined: usize,
}

impl ArchetypeIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add every archetype a `.ytyp` declares.
    ///
    /// ⚠ Later wins, as the DLC overlay does, so files must be fed in mount
    /// order; a large [`Self::redefined`] usually means they came in directory
    /// order.
    pub fn add(&mut self, types: MapTypes<'_>) {
        let owner = types.name();
        for arch in types.archetypes() {
            let name = arch.name();
            if name == 0 {
                continue;
            }
            if let Some(bounds) = arch.bounds() {
                if self.bounds.insert(name, bounds).is_some() {
                    self.redefined += 1;
                }
            }
            self.lod_dist.insert(name, arch.lod_dist());
            if owner != 0 {
                self.owner.insert(name, owner);
            }
        }
    }

    /// The `.ytyp` that declared an archetype, by that file's own `name`.
    pub fn owner_of(&self, name: u32) -> Option<u32> {
        self.owner.get(&name).copied()
    }

    pub fn bounds_of(&self, name: u32) -> Option<Bounds> {
        self.bounds.get(&name).copied()
    }

    pub fn lod_dist_of(&self, name: u32) -> Option<f32> {
        self.lod_dist.get(&name).copied()
    }

    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    /// How many archetypes were defined more than once.
    pub fn redefined(&self) -> usize {
        self.redefined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use or_io::meta::{MetaInstance, Value};

    fn types_with(name: &str, extent: f32) -> MetaInstance {
        let mut arch = MetaInstance::new("CBaseArchetypeDef");
        arch.set(
            "name",
            Value::Hash(or_io::meta::instance::value::HashedString::new(name)),
        );
        arch.set("bbMin", Value::Vec3([-extent; 3]));
        arch.set("bbMax", Value::Vec3([extent; 3]));
        arch.set("lodDist", Value::Float((extent * 10.0) as f64));
        let mut root = MetaInstance::new("CMapTypes");
        root.set(
            "archetypes",
            Value::Array(vec![Value::Pointer(Some(Box::new(arch)))]),
        );
        root
    }

    #[test]
    fn a_later_definition_replaces_an_earlier_one() {
        let base = types_with("prop_x", 1.0);
        let dlc = types_with("prop_x", 5.0);
        let mut index = ArchetypeIndex::new();
        index.add(MapTypes::new(&base).unwrap());
        index.add(MapTypes::new(&dlc).unwrap());

        let hash = or_io::string_hash(b"prop_x");
        assert_eq!(index.bounds_of(hash).unwrap().max, Vec3::splat(5.0));
        assert_eq!(index.len(), 1);
        assert_eq!(index.redefined(), 1);
    }

    #[test]
    fn an_unknown_name_resolves_to_nothing_rather_than_a_default() {
        let index = ArchetypeIndex::new();
        assert_eq!(index.bounds_of(0xDEAD_BEEF), None);
        assert!(index.is_empty());
    }
}
