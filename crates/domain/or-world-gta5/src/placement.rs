//! Where in the world a model is put.
//!
//! The reverse of a dependency edge, over the same graph: a map places an
//! archetype by name, so "every placement of this model" is that relation
//! read backwards.

use std::collections::{BTreeMap, HashSet};

use crate::MapData;
use glam::Vec3;

/// What kind of thing put the model there.
///
/// ⚠ A grass batch is a placement and not an entity: it lives in a different
/// array with its own archetype field, so a query that walked only
/// `entities` would report zero placements for every plant in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Placed {
    Entity,
    GrassBatch,
    InteriorEntity,
}

/// One placement of one model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The `CMapData::name` of the map that places it.
    pub map: u32,
    pub position: Vec3,
    pub placed: Placed,
}

/// Which archetypes an index is collecting positions for.
///
/// `All` over a retail install is tens of megabytes of positions, so a
/// targeted query asks for [`Watch::These`]. Both count every placement, so a
/// run's coverage stays checkable.
#[derive(Debug, Clone, Default)]
pub enum Watch {
    #[default]
    All,
    These(HashSet<u32>),
}

impl Watch {
    fn wants(&self, archetype: u32) -> bool {
        match self {
            Watch::All => true,
            Watch::These(set) => set.contains(&archetype),
        }
    }
}

/// Every placement of every watched archetype.
#[derive(Debug, Default)]
pub struct PlacementIndex {
    by_archetype: BTreeMap<u32, Vec<Placement>>,
    watch: Watch,
    maps: usize,
    /// Every placement seen, watched or not.
    seen: u64,
    /// Placements whose `archetypeName` is zero, which name nothing.
    nameless: u64,
}

impl PlacementIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn watching(names: impl IntoIterator<Item = u32>) -> Self {
        Self {
            watch: Watch::These(names.into_iter().collect()),
            ..Self::default()
        }
    }

    /// Fold one map in. The tree is not retained.
    pub fn add(&mut self, map: &MapData<'_>) {
        let name = map.name();
        self.maps += 1;
        for entity in map.entities() {
            let placed = if entity.is_interior_instance() {
                Placed::InteriorEntity
            } else {
                Placed::Entity
            };
            self.record(name, entity.archetype(), entity.position(), placed);
        }
        for batch in map.grass_batches() {
            self.record(name, batch.archetype(), Vec3::ZERO, Placed::GrassBatch);
        }
    }

    fn record(&mut self, map: u32, archetype: u32, position: Vec3, placed: Placed) {
        self.seen += 1;
        if archetype == 0 {
            self.nameless += 1;
            return;
        }
        if !self.watch.wants(archetype) {
            return;
        }
        self.by_archetype
            .entry(archetype)
            .or_default()
            .push(Placement {
                map,
                position,
                placed,
            });
    }

    pub fn of(&self, archetype: u32) -> &[Placement] {
        self.by_archetype
            .get(&archetype)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The distinct maps that place an archetype.
    pub fn maps_placing(&self, archetype: u32) -> Vec<u32> {
        let mut maps: Vec<u32> = self.of(archetype).iter().map(|p| p.map).collect();
        maps.sort_unstable();
        maps.dedup();
        maps
    }

    pub fn archetypes(&self) -> impl Iterator<Item = u32> + '_ {
        self.by_archetype.keys().copied()
    }

    pub fn maps(&self) -> usize {
        self.maps
    }

    /// Every placement the walk visited, watched or not.
    pub fn seen(&self) -> u64 {
        self.seen
    }

    pub fn nameless(&self) -> u64 {
        self.nameless
    }

    /// Placements actually held.
    pub fn held(&self) -> u64 {
        self.by_archetype.values().map(|v| v.len() as u64).sum()
    }

    pub fn footprint(&self) -> usize {
        self.by_archetype
            .values()
            .map(|v| v.capacity() * std::mem::size_of::<Placement>())
            .sum::<usize>()
            + self.by_archetype.len() * 48
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::map_placing;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    #[test]
    fn a_model_is_found_in_every_map_that_places_it() {
        let a = map_placing("m_a", &["prop_x", "prop_y", "prop_x"], &[]);
        let b = map_placing("m_b", &["prop_x"], &[]);
        let mut index = PlacementIndex::new();
        index.add(&MapData::new(&a).unwrap());
        index.add(&MapData::new(&b).unwrap());

        assert_eq!(index.of(hash("prop_x")).len(), 3);
        assert_eq!(
            index.maps_placing(hash("prop_x")),
            vec![hash("m_a"), hash("m_b")]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert_eq!(index.of(hash("prop_y")).len(), 1);
        assert_eq!(index.of(hash("absent")).len(), 0);
        assert_eq!(index.maps(), 2);
        assert_eq!(index.seen(), 4);
    }

    #[test]
    fn a_grass_batch_counts_as_a_placement() {
        let tree = map_placing("m", &["prop_x"], &["grass_a", "grass_a"]);
        let mut index = PlacementIndex::new();
        index.add(&MapData::new(&tree).unwrap());

        assert_eq!(index.of(hash("grass_a")).len(), 2);
        assert!(index
            .of(hash("grass_a"))
            .iter()
            .all(|p| p.placed == Placed::GrassBatch));
        assert_eq!(index.seen(), 3);
    }

    #[test]
    fn a_watched_index_counts_what_it_declines_to_hold() {
        let tree = map_placing("m", &["prop_x", "prop_y", "prop_z"], &[]);
        let mut index = PlacementIndex::watching([hash("prop_y")]);
        index.add(&MapData::new(&tree).unwrap());

        assert_eq!(index.held(), 1);
        assert_eq!(index.seen(), 3);
        assert_eq!(index.of(hash("prop_x")).len(), 0);
    }

    #[test]
    fn a_placement_naming_nothing_is_counted_rather_than_indexed() {
        let tree = map_placing("m", &["", "prop_x"], &[]);
        let mut index = PlacementIndex::new();
        index.add(&MapData::new(&tree).unwrap());

        assert_eq!(index.nameless(), 1);
        assert_eq!(index.held(), 1);
    }
}
