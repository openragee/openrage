//! How a map area's content is split across files, and what of that is derivable.
//!
//! GTA V spreads one area over several `.ymap`: `<area>_strm_N`, `<area>_long_N`,
//! `<area>_critical_N` and `<area>_lod`. Only part of that split can be
//! reproduced from the entities alone — see [`StreamingClass`] — and the
//! measurement that says which part is in
//! `docs/extra/World_Data_Specification.md`.

use std::collections::BTreeMap;

use crate::content_flags::content;
use crate::map_data::MapData;

/// The streaming policy a map declares.
///
/// ⚠ `contentFlags` bit 9 is set on `_critical_` maps and on nothing else;
/// `_strm_` and `_long_` carry identical flag words, so which of those two an
/// entity belongs in is an authoring decision the file does not record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingClass {
    /// Loaded with the area and not unloaded while it is current.
    Critical,
    /// Streamed by distance.
    Streamed,
}

impl StreamingClass {
    pub fn of(map: &MapData<'_>) -> Self {
        if map.content_flags() & content::CRITICAL != 0 {
            StreamingClass::Critical
        } else {
            StreamingClass::Streamed
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StreamingClass::Critical => "critical",
            StreamingClass::Streamed => "streamed",
        }
    }
}

/// Which LOD layer a map's entities put it in.
///
/// Derivable: an entity's `lodLevel` says which layer it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LodLayer {
    /// `LODTYPES_DEPTH_HD` and `_ORPHANHD`.
    Hd,
    /// `LODTYPES_DEPTH_LOD`.
    Lod,
    /// `LODTYPES_DEPTH_SLOD1` and coarser.
    Slod,
}

impl LodLayer {
    /// ⚠ Matched on the variant name, because a `.ymap` carries its own enum
    /// table and the number behind a name is the file's to choose.
    pub fn of_variant(variant: &str) -> Self {
        match variant {
            "LODTYPES_DEPTH_LOD" => LodLayer::Lod,
            "LODTYPES_DEPTH_SLOD1"
            | "LODTYPES_DEPTH_SLOD2"
            | "LODTYPES_DEPTH_SLOD3"
            | "LODTYPES_DEPTH_SLOD4" => LodLayer::Slod,
            _ => LodLayer::Hd,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LodLayer::Hd => "hd",
            LodLayer::Lod => "lod",
            LodLayer::Slod => "slod",
        }
    }
}

/// How a map's entities are spread across LOD layers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerCounts {
    pub hd: u32,
    pub lod: u32,
    pub slod: u32,
}

impl LayerCounts {
    pub fn of(map: &MapData<'_>) -> Self {
        let mut out = Self::default();
        for entity in map.entities() {
            match entity.lod_level().map(LodLayer::of_variant) {
                Some(LodLayer::Hd) | None => out.hd += 1,
                Some(LodLayer::Lod) => out.lod += 1,
                Some(LodLayer::Slod) => out.slod += 1,
            }
        }
        out
    }

    pub fn total(&self) -> u32 {
        self.hd + self.lod + self.slod
    }

    /// Whether every entity is in one layer.
    ///
    /// ⚠ A mixed file is the thing partitioning exists to prevent: the
    /// streamer loads a whole `.ymap` at one range, so an SLOD entity in an
    /// HD file is drawn from much too far away and an HD entity in an SLOD
    /// file pops in late.
    pub fn is_single_layer(&self) -> bool {
        [self.hd, self.lod, self.slod]
            .iter()
            .filter(|n| **n > 0)
            .count()
            <= 1
    }

    pub fn dominant(&self) -> LodLayer {
        if self.slod >= self.hd && self.slod >= self.lod {
            LodLayer::Slod
        } else if self.lod >= self.hd {
            LodLayer::Lod
        } else {
            LodLayer::Hd
        }
    }
}

/// Moving an entity between files renumbers both files.
///
/// ⚠ An entity's position in `entities` is its identity, so moving entity `i`
/// to another file shifts every index above `i` in the source and gives it a
/// new index in the destination, while LOD links, a room's `attachedObjects`
/// and an entity set's `locations` still name the old position. A rewriter
/// consults [`Remap::index_of`] for every stored index.
#[derive(Debug, Clone, Default)]
pub struct Remap {
    /// Old index -> (destination layer, new index).
    moves: BTreeMap<u32, (LodLayer, u32)>,
}

impl Remap {
    /// Plan a partition of one map's entities by their LOD layer.
    ///
    /// Entities keep their relative order within a layer, which is the only
    /// choice that leaves a single-layer map unchanged.
    pub fn by_layer(map: &MapData<'_>) -> Self {
        let mut next: BTreeMap<LodLayer, u32> = BTreeMap::new();
        let mut moves = BTreeMap::new();
        for (i, entity) in map.entities().enumerate() {
            let layer = entity
                .lod_level()
                .map(LodLayer::of_variant)
                .unwrap_or(LodLayer::Hd);
            let slot = next.entry(layer).or_default();
            moves.insert(i as u32, (layer, *slot));
            *slot += 1;
        }
        Self { moves }
    }

    /// Where an entity ends up, or `None` if it was not in the source.
    pub fn index_of(&self, old: u32) -> Option<(LodLayer, u32)> {
        self.moves.get(&old).copied()
    }

    pub fn len(&self) -> usize {
        self.moves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    /// Whether every entity keeps the index it had.
    ///
    /// ⚠ True exactly when the map was already a single layer, which is what
    /// makes a partition of an already-partitioned map a no-op rather than a
    /// renumbering.
    pub fn is_identity(&self) -> bool {
        self.moves.iter().all(|(&old, &(_, new))| old == new)
            && self
                .moves
                .values()
                .map(|(layer, _)| *layer)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                <= 1
    }

    /// How many entities end up in each layer.
    pub fn counts(&self) -> LayerCounts {
        let mut out = LayerCounts::default();
        for (layer, _) in self.moves.values() {
            match layer {
                LodLayer::Hd => out.hd += 1,
                LodLayer::Lod => out.lod += 1,
                LodLayer::Slod => out.slod += 1,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::{MetaInstance, Value};

    fn map(content_flags: u32, levels: &[&str]) -> MetaInstance {
        let items = levels
            .iter()
            .map(|lod| {
                let mut ent = MetaInstance::new("CEntityDef");
                ent.set(
                    "lodLevel",
                    Value::Enum {
                        type_name: "rage__eLodType".into(),
                        variant: (*lod).into(),
                    },
                );
                Value::Pointer(Some(Box::new(ent)))
            })
            .collect();
        let mut root = MetaInstance::new("CMapData");
        root.set("contentFlags", Value::UInt(content_flags as u64));
        root.set("entities", Value::Array(items));
        root
    }

    #[test]
    fn a_critical_map_is_known_by_content_flag_bit_nine() {
        let critical = map(content::HD | content::CRITICAL, &["LODTYPES_DEPTH_HD"]);
        let streamed = map(content::HD, &["LODTYPES_DEPTH_HD"]);
        assert_eq!(content::CRITICAL, 1 << 9);
        assert_eq!(
            StreamingClass::of(&MapData::new(&critical).unwrap()),
            StreamingClass::Critical
        );
        assert_eq!(
            StreamingClass::of(&MapData::new(&streamed).unwrap()),
            StreamingClass::Streamed
        );
    }

    #[test]
    fn layers_are_counted_by_variant_name() {
        let tree = map(
            0,
            &[
                "LODTYPES_DEPTH_HD",
                "LODTYPES_DEPTH_ORPHANHD",
                "LODTYPES_DEPTH_LOD",
                "LODTYPES_DEPTH_SLOD3",
            ],
        );
        let counts = LayerCounts::of(&MapData::new(&tree).unwrap());
        assert_eq!(counts.hd, 2);
        assert_eq!(counts.lod, 1);
        assert_eq!(counts.slod, 1);
        assert_eq!(counts.total(), 4);
        assert!(!counts.is_single_layer());
    }

    #[test]
    fn a_single_layer_map_says_so() {
        let tree = map(0, &["LODTYPES_DEPTH_HD", "LODTYPES_DEPTH_ORPHANHD"]);
        let counts = LayerCounts::of(&MapData::new(&tree).unwrap());
        assert!(counts.is_single_layer());
        assert_eq!(counts.dominant(), LodLayer::Hd);
    }

    #[test]
    fn partitioning_a_single_layer_map_is_the_identity() {
        let tree = map(0, &["LODTYPES_DEPTH_HD", "LODTYPES_DEPTH_HD"]);
        let remap = Remap::by_layer(&MapData::new(&tree).unwrap());
        assert_eq!(remap.len(), 2);
        assert!(remap.is_identity());
        assert_eq!(remap.index_of(0), Some((LodLayer::Hd, 0)));
        assert_eq!(remap.index_of(1), Some((LodLayer::Hd, 1)));
    }

    #[test]
    fn a_mixed_map_renumbers_within_each_layer() {
        let tree = map(
            0,
            &[
                "LODTYPES_DEPTH_HD",
                "LODTYPES_DEPTH_LOD",
                "LODTYPES_DEPTH_HD",
                "LODTYPES_DEPTH_LOD",
            ],
        );
        let remap = Remap::by_layer(&MapData::new(&tree).unwrap());
        assert!(!remap.is_identity());
        assert_eq!(remap.index_of(0), Some((LodLayer::Hd, 0)));
        assert_eq!(remap.index_of(1), Some((LodLayer::Lod, 0)));
        assert_eq!(remap.index_of(2), Some((LodLayer::Hd, 1)));
        assert_eq!(remap.index_of(3), Some((LodLayer::Lod, 1)));

        let counts = remap.counts();
        assert_eq!(counts.hd, 2);
        assert_eq!(counts.lod, 2);
        assert_eq!(counts.slod, 0);
    }

    #[test]
    fn an_index_that_was_not_in_the_source_remaps_to_nothing() {
        let tree = map(0, &["LODTYPES_DEPTH_HD"]);
        let remap = Remap::by_layer(&MapData::new(&tree).unwrap());
        assert_eq!(remap.index_of(7), None);
    }

    #[test]
    fn an_empty_map_remaps_to_nothing_and_is_the_identity() {
        let tree = map(0, &[]);
        let remap = Remap::by_layer(&MapData::new(&tree).unwrap());
        assert!(remap.is_empty());
        assert!(remap.is_identity());
    }
}
