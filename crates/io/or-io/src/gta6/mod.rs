//! GTA VI's formats, read as ordinary code.
//!
//! GTA VI has no source here to name a field by, so a name says the role the field was measured to
//! play. Reading comes first: a `.ttpf` texture pack is the one GTA VI file written.

// Its blocks decode through GTA V's channel codec, which `gta5` builds.
#[cfg(feature = "gta5")]
pub mod anim;
#[cfg(feature = "meta")]
pub mod anim_scene;
pub mod audio_data;
pub mod audio_links;
pub mod audio_radio;
pub mod bcpack;
pub mod dialogue;
pub mod hash;
pub mod headerless;
#[cfg(feature = "meta")]
pub mod map;
pub mod map_grid;
pub mod map_names;
#[cfg(feature = "meta")]
pub mod mission_other;
pub mod mission_tables;
pub mod move_network;
#[cfg(test)]
pub(crate) mod mutated_tests;
pub mod names;
pub mod other_bodies;
#[cfg(feature = "meta")]
pub mod ped_catalogue;
pub mod save;
// Its natives list is GTA V's `NativeDb`, which `gta5` builds.
#[cfg(feature = "gta5")]
pub mod script;
pub mod script_bodies;
pub mod slice_sources;
pub mod text;
pub mod text_language;
pub mod texture_names;
mod tiling;
#[cfg(feature = "gta5")]
pub mod timecycle;
pub mod ttpf;
pub mod ui_tree;
#[cfg(all(feature = "gta5", feature = "meta"))]
pub mod water;
#[cfg(feature = "gta5")]
pub mod weather;
#[cfg(feature = "meta")]
pub mod world_state;

pub use ttpf::{Entry, Format, Header, Row, RowRole, Texture, TexturePack, TtpfError};
