//! Why an edge is in a dependency set, and what each archetype of an install declares of its assets: the two the world index names.

use std::collections::BTreeMap;

use crate::{AssetType, MapTypes};

/// Why an edge is in a dependency set, so a caller exporting an asset can
/// tell which dependencies a file states and which this crate infers
/// ([`Via::is_stated`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Via {
    /// `assetName`, with `assetType` saying which family.
    ArchetypeAsset,
    ArchetypeTextureDictionary,
    ArchetypeDrawableDictionary,
    ArchetypePhysicsDictionary,
    ArchetypeClipDictionary,
    /// The type file the archetype is declared in.
    DeclaringTypeFile,
    /// `gtxd.meta` says this dictionary falls through to that one.
    TextureDictionaryParent,
    /// The manifest binds a high-detail dictionary to this asset.
    ManifestHdTexture,
    /// A filename convention rather than a reference.
    HighDetailCompanion,
    /// The manifest's `MapDataGroups` names this static bound for the map.
    ManifestMapDataGroup,
    /// A filename convention: a `.ybn` named `<map>_N`, `hi@<map>_N` or
    /// `ma@<map>_N` for a map no group names, with whether its box lies inside
    /// the map's streaming box (`None` where either body was not read).
    MapStemConvention {
        inside_streaming_box: Option<bool>,
    },
    /// The manifest's `Interiors` names this bound for the interior archetype.
    ManifestInterior,
}

impl Via {
    /// Whether a file states this edge, as against this crate inferring it.
    pub fn is_stated(&self) -> bool {
        !matches!(
            self,
            Via::HighDetailCompanion | Via::MapStemConvention { .. }
        )
    }
}

/// What one archetype declares, kept as scalars so the tree can go.
///
/// ⚠ Scalars, never the instance: a retail install's `.ytyp` trees do not
/// fit in memory together, so the caller drops each tree at the end of its
/// own iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchetypeAssets {
    pub asset_name: u32,
    pub asset_type: AssetType,
    pub texture_dictionary: u32,
    pub drawable_dictionary: u32,
    pub physics_dictionary: u32,
    pub clip_dictionary: u32,
    /// The `.ytyp` that declared it.
    pub owner: u32,
}

/// Every archetype's asset fields, by name hash.
#[derive(Debug, Default, Clone)]
pub struct ArchetypeAssetIndex {
    by_name: BTreeMap<u32, ArchetypeAssets>,
    redefined: usize,
}

impl ArchetypeAssetIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add every archetype a `.ytyp` declares. Later wins, as the mount does.
    pub fn add(&mut self, types: MapTypes<'_>) {
        let owner = types.name();
        for arch in types.archetypes() {
            let name = arch.name();
            if name == 0 {
                continue;
            }
            let assets = ArchetypeAssets {
                asset_name: arch.asset_name(),
                asset_type: arch.asset_type(),
                texture_dictionary: arch.texture_dictionary(),
                drawable_dictionary: arch.drawable_dictionary(),
                physics_dictionary: arch.physics_dictionary(),
                clip_dictionary: arch.clip_dictionary(),
                owner,
            };
            if self.by_name.insert(name, assets).is_some() {
                self.redefined += 1;
            }
        }
    }

    pub fn get(&self, name: u32) -> Option<&ArchetypeAssets> {
        self.by_name.get(&name)
    }

    /// The archetype named `name`, else the first whose `assetName` it is.
    pub fn archetype_of(&self, name: u32) -> Option<u32> {
        if self.by_name.contains_key(&name) {
            return Some(name);
        }
        (self.by_name.iter())
            .find(|(_, a)| a.asset_name == name)
            .map(|(n, _)| *n)
    }

    pub fn names(&self) -> impl Iterator<Item = u32> + '_ {
        self.by_name.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn redefined(&self) -> usize {
        self.redefined
    }

    /// Roughly how many bytes this holds, for a caller reporting its cost.
    pub fn footprint(&self) -> usize {
        self.by_name.len() * (std::mem::size_of::<ArchetypeAssets>() + 48)
    }
}
