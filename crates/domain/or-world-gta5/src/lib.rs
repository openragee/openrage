//! GTA V's world behind the world contract, for the Legacy and the Enhanced game: its title, an
//! opened install over the mount, its index through the kept file every title shares, its map
//! bodies and interiors, and its models by the name of their archetype.
//!
//! ⚠ No file of the world is read here: the index, the reader of a map and the collision of a
//! mount are `or_depends`' as they stand, and this crate turns what they give into the contract's
//! terms. A map is keyed by its row of the linked index, where `or_depends` keys it by its name.

mod body;
mod index;
mod install;
mod models;
mod source;
mod title;

pub use body::{Body, BOXED_UNDECLARED, BOXED_UNNAMED, WORDS};
pub use index::{Linked, RULES};
pub use install::Gta5World;
pub use models::BOX;
pub use title::{Gta5Title, ENHANCED, LEGACY, LEVEL, MODE, MULTIPLAYER, STORY, TITLE};
