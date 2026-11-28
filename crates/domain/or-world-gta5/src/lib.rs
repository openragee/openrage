//! GTA V's world, for the Legacy and the Enhanced game: its model of what a `.ymap` places and a `.ytyp` declares, its index of an
//! install with the catalogue, the tables and the search over it, and the same world behind the world contract: its title, an
//! opened install over the mount, its index through the kept file every title shares, its map bodies and interiors with their
//! lights and occluders, and its models and textures by the name of their archetype.
//!
//! ⚠ **The model copies no tree**, [`scenario`] apart. `or_io::meta` reads a map and a type file into a value tree, and every
//! type of the model is a borrow over the `MetaInstance` the reader produced, so there is no second model that could disagree
//! with the file and nothing to lose on the way back out. The write path edits the tree itself and re-encodes it against the
//! layout the file arrived with.
//!
//! ⚠ **The model keys on no field's spelling**, which depends on whether a gitignored schema snapshot is staged: the same field
//! is `entities` on one machine and `0xCCAB9307` on another. [`field`] reads by the number both spellings stand for, and [`keys`]
//! holds those numbers as compile-time hashes of their own names.
//!
//! ⚠ A map is keyed by its row of the linked index behind the contract, and by its name in the world index and the forest.

// The fixtures of `tests_support` name this crate by its own name, so the crate that resolves
// what the index states includes the file by path for its tests.
#[cfg(test)]
extern crate self as or_world_gta5;

pub mod archetype_assets;
pub mod archetypes;
pub mod catalogue;
pub mod collision;
pub mod content_flags;
pub mod contents;
pub mod create;
pub mod doors;
pub mod execute;
pub mod extension;
pub mod extents;
pub mod field;
pub mod find;
pub mod generate;
pub mod group;
pub mod keys;
pub mod manifest;
pub mod manifest_build;
pub mod map_data;
pub mod map_types;
pub mod model_catalogue;
pub mod partition;
pub mod placement;
pub mod plan;
pub mod prefab;
pub mod scenario;
pub mod script_text;
pub mod state;
pub mod tables;
pub mod transform;
pub mod validate;
pub mod vehicles;
pub mod world_collision;
pub mod world_forest;
pub mod world_index;
pub mod write;
pub mod xml;

#[cfg(test)]
// Each of the two crates that include the file builds on a part of it.
#[allow(dead_code)]
mod tests_support;

mod beside;
mod body;
mod index;
mod install;
mod models;
mod source;
mod textures;
mod title;

pub use beside::Beside;
pub use body::{Body, BOXED_UNDECLARED, BOXED_UNNAMED, WORDS};
pub use index::{Holding, Linked, RULES};
pub use install::Gta5World;
pub use models::{Bound, Material, Materials, NoModel, Opened, Outcome, BOX};
pub use textures::{NoTexture, EMBEDDED};
pub use title::{Gta5Title, ENHANCED, LEGACY, LEVEL, MODE, MULTIPLAYER, STORY, TITLE};

pub use archetype_assets::{ArchetypeAssetIndex, ArchetypeAssets, Via};
pub use catalogue::{Catalogue, Kind, Located};
pub use contents::ModelContents;
pub use model_catalogue::ModelCatalogue;
pub use placement::{PlacementIndex, Watch};
pub use tables::InstallTables;
pub use vehicles::VehicleCatalogue;
pub use world_collision::WorldCollision;
pub use world_forest::WorldForest;
pub use world_index::{MountedWorld, WorldIndex, WorldSource};

pub use archetypes::ArchetypeIndex;
pub use content_flags::{ContentFlags, Divergence};
pub use extension::{Extension, ExtensionKind};
pub use extents::Extents;
pub use group::{GroupProblem, MapGroup};
pub use manifest::{Manifest, StatedDependencies};
pub use map_data::{
    BoxOccluder, CarGenerator, DistantLodLight, DistantLodLights, Entity, GrassBatch, LodLight,
    LodLights, MapData, TimeCycleModifier,
};
pub use map_types::{
    Archetype, ArchetypeKind, AssetType, Bounds, EntitySet, Interior, MapTypes, Portal, Room,
};
pub use plan::{Edit, Plan, PlanProblem};
pub use state::{Delta, Gate, StateGroup, StateGroups, WorldState};
pub use transform::Placement;
pub use validate::Problem;

/// Whether a value tree is one of the two world roots.
pub fn is_world_root(inst: &or_io::meta::MetaInstance) -> bool {
    field::is_type(inst, keys::types::C_MAP_DATA) || field::is_type(inst, keys::types::C_MAP_TYPES)
}

/// The default library, with the staged snapshot installed when there is one.
#[cfg(test)]
pub(crate) fn test_schema() -> &'static or_io::meta::SchemaLibrary {
    // ⚠ Once a process: finding the snapshot walks the directories above
    // the workspace, and the library is asked for once a field.
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        if let Some(path) = or_test_support::schema_snapshot() {
            if let Err(e) = or_io::meta::install_snapshot(&path) {
                panic!(
                    "the staged schema snapshot {} is unusable: {e}",
                    path.display()
                );
            }
        }
    });
    or_io::meta::default_library()
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::MetaInstance;

    #[test]
    fn both_roots_are_recognised_under_either_spelling() {
        assert!(is_world_root(&MetaInstance::new("CMapData")));
        assert!(is_world_root(&MetaInstance::new("CMapTypes")));
        assert!(is_world_root(&MetaInstance::new("0xD3593FA6")));
        assert!(is_world_root(&MetaInstance::new("0xD98BB561")));
        assert!(!is_world_root(&MetaInstance::new("CPedVariationInfo")));
    }
}
