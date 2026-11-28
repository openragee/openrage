//! What a `.ymap` says it contains, derived from what it contains.
//!
//! ⚠ `flags` and `contentFlags` decide whether the streamer looks inside a map
//! at all, so an edit that adds the first interior instance and leaves them
//! alone produces a file the game loads and ignores. A stored bit the contents
//! do not imply is normal — a map group is authored as a set and a member can
//! carry its neighbour's claim — so [`Divergence`] matters in the other
//! direction: a bit the contents imply and the file does not set.

use crate::map_data::MapData;

/// Bit positions in `contentFlags`, named for what sets them.
pub mod content {
    pub const HD: u32 = 1 << 0;
    pub const LOD: u32 = 1 << 1;
    pub const SLOD2_PLUS: u32 = 1 << 2;
    pub const INTERIOR: u32 = 1 << 3;
    pub const SLOD: u32 = 1 << 4;
    pub const OCCLUSION: u32 = 1 << 5;
    pub const PHYSICS: u32 = 1 << 6;
    pub const LOD_LIGHTS: u32 = 1 << 7;
    pub const DISTANT_LOD_LIGHTS: u32 = 1 << 8;
    /// Loaded with the area rather than by distance.
    ///
    /// ⚠ An authoring decision that [`ContentFlags::of`] never sets: measured
    /// over a retail Legacy install, every `_critical_` map sets it and no
    /// `_strm_` or `_long_` map does. Recomputing flags must preserve it, so
    /// an overstated bit is reported rather than cleared.
    pub const CRITICAL: u32 = 1 << 9;
    pub const GRASS: u32 = 1 << 10;
}

/// Bit positions in `flags`.
pub mod map {
    /// Set by anything below HD: the streamer treats the map as a LOD layer.
    pub const SCRIPTED_OR_LOD: u32 = 1 << 1;
}

/// The two flag words a map's contents imply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentFlags {
    pub flags: u32,
    pub content: u32,
}

impl ContentFlags {
    /// What this map's contents imply.
    pub fn of(map: &MapData<'_>) -> Self {
        let mut flags = 0u32;
        let mut content = 0u32;

        for entity in map.entities() {
            // ⚠ An entity whose class states no `lodLevel` is read at the enum's zero, which is HD, as a variant with no name is.
            match entity.lod_level().map_or(LodDepth::Hd, lod_depth) {
                LodDepth::Hd => content |= content::HD,
                LodDepth::Lod => {
                    content |= content::LOD;
                    flags |= map::SCRIPTED_OR_LOD;
                }
                LodDepth::Slod1 => {
                    content |= content::SLOD;
                    flags |= map::SCRIPTED_OR_LOD;
                }
                LodDepth::Slod2Plus => {
                    content |= content::SLOD2_PLUS | content::SLOD;
                    flags |= map::SCRIPTED_OR_LOD;
                }
            }
            if entity.is_interior_instance() {
                content |= content::INTERIOR;
            }
        }

        if map.physics_dictionary_count() > 0 {
            content |= content::PHYSICS;
        }
        if map.grass_batches().next().is_some() {
            content |= content::GRASS;
        }
        if map.has_lod_lights() {
            content |= content::LOD_LIGHTS;
        }
        if map.has_distant_lod_lights() {
            content |= content::DISTANT_LOD_LIGHTS;
            flags |= map::SCRIPTED_OR_LOD;
        }
        if map.box_occluder_count() > 0 || map.occlude_model_count() > 0 {
            content |= content::OCCLUSION;
        }

        Self { flags, content }
    }
}

/// How a map's stored flags differ from what its contents imply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    /// Bits the contents imply that the file does not set.
    ///
    /// ⚠ This is the direction that breaks a map: content the streamer is
    /// not told about is content it does not load.
    pub missing: u32,
    /// Bits the file sets that the contents do not imply.
    pub extra: u32,
}

impl Divergence {
    pub fn of(map: &MapData<'_>) -> Self {
        let implied = ContentFlags::of(map);
        let stored = map.content_flags();
        Self {
            missing: implied.content & !stored,
            extra: stored & !implied.content,
        }
    }

    pub fn is_exact(&self) -> bool {
        self.missing == 0 && self.extra == 0
    }
}

/// Which LOD layer an entity belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LodDepth {
    Hd,
    Lod,
    Slod1,
    Slod2Plus,
}

/// ⚠ Matched on the variant name rather than the number: a `.ymap` carries its
/// own enum table, so the value behind `LODTYPES_DEPTH_HD` is the file's to
/// choose and two files need not agree.
fn lod_depth(variant: &str) -> LodDepth {
    match variant {
        "LODTYPES_DEPTH_LOD" => LodDepth::Lod,
        "LODTYPES_DEPTH_SLOD1" => LodDepth::Slod1,
        "LODTYPES_DEPTH_SLOD2" | "LODTYPES_DEPTH_SLOD3" | "LODTYPES_DEPTH_SLOD4" => {
            LodDepth::Slod2Plus
        }
        _ => LodDepth::Hd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::{MetaInstance, Value};

    fn map_with(entities: Vec<MetaInstance>, stored_content: u32) -> MetaInstance {
        let mut root = MetaInstance::new("CMapData");
        root.set("contentFlags", Value::UInt(stored_content as u64));
        root.set(
            "entities",
            Value::Array(
                entities
                    .into_iter()
                    .map(|e| Value::Pointer(Some(Box::new(e))))
                    .collect(),
            ),
        );
        root
    }

    fn entity(type_name: &str, lod: &str) -> MetaInstance {
        let mut ent = MetaInstance::new(type_name);
        ent.set(
            "lodLevel",
            Value::Enum {
                type_name: "rage__eLodType".into(),
                variant: lod.into(),
            },
        );
        ent
    }

    #[test]
    fn an_hd_entity_implies_the_hd_bit_and_no_lod_flag() {
        let root = map_with(vec![entity("CEntityDef", "LODTYPES_DEPTH_HD")], 0);
        let f = ContentFlags::of(&MapData::new(&root).unwrap());
        assert_eq!(f.content, content::HD);
        assert_eq!(f.flags, 0);
    }

    /// SLOD2 and below set both `SLOD2_PLUS` and `SLOD`; a map of distant
    /// hills that sets only one is one the streamer half-loads.
    #[test]
    fn slod2_implies_both_of_its_bits() {
        let root = map_with(vec![entity("CEntityDef", "LODTYPES_DEPTH_SLOD3")], 0);
        let f = ContentFlags::of(&MapData::new(&root).unwrap());
        assert_eq!(f.content, content::SLOD2_PLUS | content::SLOD);
        assert_eq!(f.flags, map::SCRIPTED_OR_LOD);
    }

    #[test]
    fn an_interior_instance_implies_the_interior_bit() {
        let root = map_with(vec![entity("CMloInstanceDef", "LODTYPES_DEPTH_HD")], 0);
        let f = ContentFlags::of(&MapData::new(&root).unwrap());
        assert_eq!(f.content, content::HD | content::INTERIOR);
    }

    #[test]
    fn a_missing_bit_is_reported_separately_from_an_extra_one() {
        let root = map_with(
            vec![entity("CEntityDef", "LODTYPES_DEPTH_HD")],
            content::GRASS,
        );
        let d = Divergence::of(&MapData::new(&root).unwrap());
        assert_eq!(d.missing, content::HD);
        assert_eq!(d.extra, content::GRASS);
        assert!(!d.is_exact());
    }

    #[test]
    fn a_map_that_declares_exactly_what_it_holds_diverges_by_nothing() {
        let root = map_with(vec![entity("CEntityDef", "LODTYPES_DEPTH_HD")], content::HD);
        assert!(Divergence::of(&MapData::new(&root).unwrap()).is_exact());
    }

    #[test]
    fn an_entity_that_states_no_lod_level_implies_the_hd_bit() {
        let root = map_with(vec![MetaInstance::new("CEntityDef")], 0);
        let f = ContentFlags::of(&MapData::new(&root).unwrap());
        assert_eq!(f.content, content::HD);
        assert_eq!(f.flags, 0);
    }

    #[test]
    fn a_lod_level_is_read_by_name() {
        assert_eq!(lod_depth("LODTYPES_DEPTH_SLOD1"), LodDepth::Slod1);
        assert_eq!(lod_depth("LODTYPES_DEPTH_ORPHANHD"), LodDepth::Hd);
        assert_eq!(lod_depth("hash_12345678"), LodDepth::Hd);
    }
}
