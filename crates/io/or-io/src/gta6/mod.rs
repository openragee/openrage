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
pub mod map_names;
pub mod mission_tables;
pub mod move_network;
pub mod names;
pub mod other_bodies;
pub mod save;
pub mod script_bodies;
pub mod text;
pub mod text_language;
pub mod texture_names;
mod tiling;
pub mod ttpf;
pub mod ui_tree;

pub use ttpf::{Entry, Format, Header, Row, RowRole, Texture, TexturePack, TtpfError};
