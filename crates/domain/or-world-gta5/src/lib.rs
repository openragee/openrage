//! GTA V's world behind the world contract, for the Legacy and the Enhanced game: its title, an
//! opened install over the mount, its index through the kept file every title shares, its map
//! bodies and interiors with their lights and occluders, and its models and textures by the name
//! of their archetype.
//!
//! ⚠ The index, the entities of a map and the collision of a mount are `or_depends`' as they
//! stand, and this crate turns what they give into the contract's terms; what a map states beside
//! its entities, a model and a texture are read here, through `or_io`. A map is keyed by its row
//! of the linked index, where `or_depends` keys it by its name.

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
