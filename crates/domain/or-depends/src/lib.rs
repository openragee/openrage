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
pub mod census;
pub mod export;
pub mod resolve;
pub mod rules;

// The fixtures of the world index's own tests, which the resolver's and the census's build on.
#[cfg(test)]
#[path = "../../or-world-gta5/src/tests_support.rs"]
// Each of the two crates that include the file builds on a part of it.
#[allow(dead_code)]
mod tests_support;

pub use census::{Census, Coverage, PhysicsRivals, TextureSources};
pub use export::{export, ExportError, Exported, MANIFEST};
pub use resolve::{DependencySet, Edge, Found, Resolver};
pub use rules::{Derivable, Rule, Stated, RULES};
