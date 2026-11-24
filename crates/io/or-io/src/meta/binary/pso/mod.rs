#[cfg(feature = "gta6")]
pub(crate) mod chks;
pub(crate) mod data_names;
#[cfg(feature = "gta6")]
mod pform_group;
#[cfg(feature = "gta6")]
mod pform_group_grow;
#[cfg(feature = "gta6")]
pub(crate) mod pform_layout;
pub mod reader;
pub mod schema;
pub mod sections;
#[cfg(feature = "gta6")]
pub mod string_blob;
#[cfg(feature = "gta6")]
pub mod string_names;
#[cfg(feature = "gta6")]
mod table_canonical;
pub mod table_form;
#[cfg(test)]
mod table_form_depth;
#[cfg(feature = "gta6")]
pub(crate) mod table_layout;
#[cfg(feature = "gta6")]
mod table_values;
pub mod writer;

/// Whether a save leaves this PSO body's trailing `CHKS` checksum carried as it was read rather
/// than recomputing it, so the game (which recomputes and rejects a mismatch) may refuse an edited
/// file. True for a GTA V/RDR2 `yppp` 0x14 frame and a tagless 0x10 frame, whose rules a save does
/// not reproduce; false for GTA VI's `zppp` 0x14 frame (recomputed on an edit) and a body with no
/// frame. Pure byte inspection, so it answers for any build, feature-gated or not.
pub fn keeps_stale_checksum(body: &[u8]) -> bool {
    let n = body.len();
    let is_14 = n >= 20
        && &body[n - 20..n - 16] == b"CHKS"
        && body[n - 16..n - 12] == 0x14u32.to_be_bytes();
    if is_14 {
        // GTA VI's 0x14 frame, tagged `zppp` at its last four bytes, is the only one recomputed.
        return &body[n - 4..] != b"zppp";
    }
    // A tagless 0x10 frame is carried; so is anything with no frame.
    n >= 16 && &body[n - 16..n - 12] == b"CHKS" && body[n - 12..n - 8] == 0x10u32.to_be_bytes()
}
