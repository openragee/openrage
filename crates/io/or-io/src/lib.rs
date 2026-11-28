//! GTA V's file formats: the containers, the descriptions and the canonical
//! writer, and the declarations `#[record]` turns into zero-copy structs.

extern crate self as or_io;

#[cfg(feature = "gta5")]
pub mod asset;
pub mod audio_world_sectors;
#[cfg(feature = "authoring")]
pub mod authoring;
#[cfg(feature = "awc")]
pub mod awc;
pub mod cache_loader;
pub mod codec;
pub mod constants;
pub mod container;
#[cfg(feature = "dds")]
pub mod dds;
pub mod decl;
pub mod desc;
pub mod diagnostics;
pub mod distant_lights;
pub mod error;
pub mod generation;
#[cfg(feature = "gfx")]
pub mod gfx;
#[cfg(feature = "gta5")]
pub mod gta5;
#[cfg(feature = "gta6")]
pub mod gta6;
pub mod half;
pub mod hash;
pub mod height_map;
#[cfg(feature = "meta")]
pub mod meta;
pub mod mrf;
pub mod names;
pub mod nav_precalc;
pub mod needle;
pub mod platform;
#[cfg(feature = "archives")]
pub mod rdr2;
#[cfg(feature = "rel")]
pub mod rel;
pub mod social_club_metadata;
pub mod water_map;
pub mod world_blanket;
pub mod write;

#[cfg(feature = "gta5")]
pub use asset::{from_bytes, open, Asset};
pub use diagnostics::{Arg, Diagnostic, Diagnostics, Severity};
pub use error::{Error, ErrorMode, IoError, Result};
pub use generation::{Generation, TargetDescriptor, TargetId};
pub use half::{f32_to_half, half_to_f32};
pub use hash::{
    finalize_hash, literal_string_hash, partial_string_hash, string_hash, string_hash_str,
};
pub use names::{Collision, HashFlavour, NameBook, NameIndex, Resolution};
pub use needle::{hex_preview, Needle, NeedleKind};
pub use platform::Platform;
