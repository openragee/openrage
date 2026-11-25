//! What an asset needs, and where a model is placed.
//!
//! Both are the same graph read in opposite directions, and both are
//! questions about an install rather than a file: a `.ymap` places an
//! archetype by name and never says which `.ytyp` declares it, so this crate
//! sits above the title layer and takes a mount.
//!
//! ⚠ The ceiling is per format and is data: [`rules`] says what each format
//! states, what only the install states, what is a filename convention and
//! what nothing states, and its suite fails when a format this crate can
//! classify carries no ruling.
//!
//! ⚠ Nothing here builds a second model of map data: the placement half keeps
//! a position and a map name per hit, the archetype half a few scalars per
//! archetype and the world index a few per map, type file and bound, because
//! a retail install's value trees do not fit in memory together.

pub mod audio;
pub mod catalogue;
pub mod census;
pub mod contents;
pub mod export;
pub mod find;
pub mod placement;
pub mod resolve;
pub mod rules;
pub mod tables;
pub mod world_collision;
pub mod world_index;

#[cfg(test)]
mod tests_support;

pub use catalogue::{Catalogue, Kind, Located};
pub use census::{Census, Coverage, PhysicsRivals, TextureSources};
pub use contents::ModelContents;
pub use export::{export, ExportError, Exported, MANIFEST};
pub use placement::{Placed, Placement, PlacementIndex, Watch};
pub use resolve::{
    ArchetypeAssetIndex, ArchetypeAssets, DependencySet, Edge, Found, Resolver, Via,
};
pub use rules::{Derivable, Rule, Stated, RULES};
pub use tables::InstallTables;
pub use world_collision::WorldCollision;
pub use world_index::{MountedWorld, WorldIndex, WorldSource};
