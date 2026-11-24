//! The world editor's model: what a `.ymap` places and what a `.ytyp` declares.
//!
//! `or_io::meta` reads both (19,392 and 2,763 of them in a retail Legacy install
//! decode and round-trip) into a value tree that says nothing about entities,
//! archetypes, interiors or extents. This crate is that layer, and
//! [`scenario`] names a scenario region's points over its typed root.
//!
//! ⚠ **Nothing here copies the tree**, [`scenario`] apart. Every type is a borrow over the
//! `MetaInstance` the reader produced, so there is no second model that
//! could disagree with the file and nothing to lose on the way back out.
//! The write path edits the tree itself and re-encodes it against the
//! layout the file arrived with.
//!
//! ⚠ **Nothing here keys on a field's spelling**, which depends on whether a
//! gitignored schema snapshot is staged: the same field is `entities` on one
//! machine and `0xCCAB9307` on another. [`field`] reads by the number both
//! spellings stand for, and [`keys`] holds those numbers as compile-time
//! hashes of their own names.

pub mod archetypes;
pub mod content_flags;
pub mod execute;
pub mod extension;
pub mod extents;
pub mod field;
pub mod generate;
pub mod group;
pub mod keys;
pub mod manifest;
pub mod map_data;
pub mod map_types;
pub mod partition;
pub mod plan;
pub mod prefab;
pub mod scenario;
pub mod state;
pub mod streaming;
pub mod validate;
pub mod write;
pub mod xml;

pub use archetypes::ArchetypeIndex;
pub use content_flags::{ContentFlags, Divergence};
pub use extension::{Extension, ExtensionKind};
pub use extents::{Aabb, Extents};
pub use group::{GroupProblem, MapGroup};
pub use manifest::{Manifest, StatedDependencies};
pub use map_data::{BoxOccluder, Entity, GrassBatch, MapData};
pub use map_types::{
    Archetype, ArchetypeKind, AssetType, Bounds, EntitySet, Interior, MapTypes, Portal, Room,
};
pub use plan::{Edit, Plan, PlanProblem};
pub use state::{Delta, Gate, StateGroup, StateGroups, WorldState};
pub use transform::Placement;
pub use validate::Problem;

pub mod transform;

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
