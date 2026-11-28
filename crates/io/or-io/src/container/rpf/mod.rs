pub mod archive;
pub mod builder;
pub mod crypto;
pub mod defrag;
pub mod entry;
#[cfg(feature = "gta6")]
pub mod gta6_archive;
#[cfg(feature = "gta6")]
pub mod gta6_chunk_table;
#[cfg(feature = "gta6")]
pub mod gta6_container_write;
#[cfg(feature = "gta6")]
pub mod gta6_content;
#[cfg(feature = "gta6")]
pub mod gta6_dependencies;
#[cfg(feature = "gta6")]
pub mod gta6_extract;
#[cfg(feature = "gta6")]
pub mod gta6_form_h;
#[cfg(feature = "gta6")]
pub mod gta6_keygate;
#[cfg(feature = "gta6")]
pub mod gta6_kinds;
#[cfg(feature = "gta6")]
pub mod gta6_mount;
#[cfg(feature = "gta6")]
pub mod gta6_navigation_rows;
#[cfg(feature = "gta6")]
pub mod gta6_patch;
#[cfg(feature = "gta6")]
pub mod gta6_rpf;
#[cfg(feature = "gta6")]
pub mod gta6_strings;
#[cfg(feature = "gta6")]
pub mod gta6_write;
pub mod header;
mod layout;
mod lzb16;
pub mod oodle;
mod reader;
pub mod rpf9;
#[cfg(feature = "gta6")]
mod rpf9_first_entry;
#[cfg(feature = "gta6")]
pub mod rpf9_grow;
#[cfg(feature = "gta6")]
pub mod rpf9_scan;
#[cfg(feature = "gta6")]
pub mod rpf9_toc;
#[cfg(feature = "gta6")]
pub mod rpf9_write;
pub mod rsc8;
pub mod version;

pub use crate::platform::RAGE_PLATFORM_LETTERS;

/// The tails a platform letter is followed by in RAGE extensions (`#dr`, `#td`, `#map`, …).
pub const RAGE_STUB_TAILS: &[&str] = &[
    "dr", "ft", "dd", "td", "bn", "bd", "bs", "cd", "ld", "sc", "nv", "nh", "jd", "mt", "pt", "ch",
    "ad", "cs", "pd", "pdb", "ldb", "as", "sd", "vr", "wr", "map", "typ", "mf", "ed", "fd", "nm",
    "pm", "pmd", "pfl", "pl", "nd", "cdt", "edt", "gd", "gx", "hm", "pr", "prp", "sf", "sg", "si",
    "sp", "st", "tb", "tx", "vd",
];

/// The platform letter `name`'s extension starts with, where that extension is a RAGE platform
/// letter followed by a known stub (`car.ydr` → `y`, `car.zdr` → `z`); `None` for `rpf`, `meta`,
/// `dat` or any other extension.
pub fn platform_letter_of_name(name: &str) -> Option<u8> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    let (&first, tail) = ext.as_bytes().split_first()?;
    let tail = std::str::from_utf8(tail).ok()?;
    (RAGE_PLATFORM_LETTERS.contains(&first) && RAGE_STUB_TAILS.contains(&tail)).then_some(first)
}

pub use archive::{RpfArchive, RpfNode};
pub use builder::RpfBuilder;
#[cfg(feature = "crypto")]
pub use crypto::extractor::ExeFingerprint;
pub use crypto::{
    default_provider, CryptoTarget, NoOpCryptoProvider, RpfCryptoProvider, TocDecryptOutcome,
};
#[cfg(feature = "crypto")]
pub use crypto::{
    extract_keys, extract_keys_with, fingerprint_executable, EncryptCache, EncryptDeriveProgress,
    ExtractionMode, ExtractionProgress, Gta5Keys, Gta5NgEncryptState, Gta5RetailCryptoProvider,
    KeysCache, KeysCacheMetadata, Rdr2CryptoProvider, Rdr2Keys, ENCRYPT_CACHE_SCHEMA,
    KEYS_CACHE_SCHEMA, RDR2_SNAPSHOT_ENV, RDR2_SNAPSHOT_FILE_NAME,
};
pub use defrag::Defrag;
pub use entry::{
    Compressor, FiPackEntry, FiPackEntryDirectory, FiPackEntryFile, FiPackEntryResource,
    FiPackEntryU, RpfBinaryEntry, RpfDirectoryEntry, RpfEntry, RpfEntryKind, RpfResourceEntry,
};
#[cfg(feature = "gta6")]
pub use gta6_archive::{
    Gta6Archive, Gta6Cipher, Gta6Class, Gta6Entry, Gta6EntryExtra, Gta6NestedRow, NameResolver,
    GROUP_CHUNK_RECORD, GROUP_CIPHER, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW, GROUP_ENTRY_ROW_20,
    GROUP_NAME32, GROUP_NAME64,
};
#[cfg(feature = "gta6")]
pub use gta6_container_write::{Rpf9ArchiveWriter, Rpf9Piece, Rpf9Plan};
#[cfg(feature = "gta6")]
pub use gta6_keygate::{decode_masked, encode_masked, gate_key, is_masked, MASKED_MAGIC};
#[cfg(feature = "gta6")]
pub use gta6_mount::{open_at, Gta6DecryptedBody, Gta6Mount};
#[cfg(feature = "gta6")]
pub use gta6_navigation_rows::{navigation_cell, navigation_cells, NAVIGATION_MESH};
#[cfg(feature = "gta6")]
pub use gta6_patch::{
    plan_patch, write_patched_copy, write_planned_copy, BodyPatch, ChunkPatch, PatchPlan,
    PatchRefusal, PatchReport,
};
#[cfg(feature = "gta6")]
pub use gta6_rpf::{
    debug_name_from_virtual, default_names_dir, shipped_dir, shipped_script_names_dir,
    substitute_platform_letter, Gta6Context, Rpf9Kind, Rpf9Place, GTA6_NAMES_ENV,
    GTA6_SCRIPT_NAMES_ENV,
};
#[cfg(feature = "gta6")]
pub use gta6_write::{Rpf9Builder, Rpf9CipherCore};
pub use header::{
    FiPackHeader, RpfEncryption, RpfHeader, RPF7_MAGIC, RPF_AES_MAGIC, RPF_DIRECTORY_OFFSET_MARKER,
    RPF_FILE_ALIGNMENT, RPF_FILE_ALIGNMENT_SHIFT, RPF_MAX_RESOURCE_CONSUMED_SIZE,
    RPF_NAME_OFFSET_MAX_SHIFT, RPF_NG_MAGIC, RPF_OPEN_MAGIC, RPF_TRAILING_PAD,
};
pub use reader::BodyForm;
pub use rpf9::RPF9_MAGIC;
#[cfg(feature = "gta6")]
pub use rpf9::{Rpf9Header, RPF9_HEADER_SIZE};
#[cfg(feature = "gta6")]
pub use rpf9_first_entry::rpf9_first_entry_end;
#[cfg(feature = "gta6")]
pub use rpf9_grow::{grow_section, Rpf9GrowError, Rpf9Section};
#[cfg(feature = "gta6")]
pub use rpf9_scan::EMPTY_ARCHIVE_PLACEHOLDER;
#[cfg(feature = "gta6")]
pub use rpf9_scan::{
    extract_rpf9_body, read_rpf9_body, seal_rpf9_scan, starts_stored_rpf9_resource,
    unseal_rpf9_scan, Rpf9Block, Rpf9BodyKind, Rpf9Content, Rpf9Family, Rpf9Resource, Rpf9Role,
    Rpf9Scan, Rpf9ScanCarry, Rpf9ScanError, Rpf9ScanFind, Rpf9Signature, Rpf9Walk, Rpf9ZlibRun,
    RPF9_MISSING_RUN, RPF9_SLOT,
};
#[cfg(feature = "gta6")]
pub use rpf9_scan::{rpf9_block_streams, rpf9_names_by_count_at, Rpf9NamesByCount};
#[cfg(feature = "gta6")]
pub use rpf9_toc::{
    GtaViKeys, Rpf9Group, Rpf9Toc, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV, GTA6_SNAPSHOT_FILE_NAME,
};
#[cfg(feature = "gta6")]
pub use rpf9_write::write_rpf9_resource_capped;
#[cfg(feature = "gta6")]
pub use rpf9_write::{write_rpf9_resource, Rpf9Form, Rpf9Sections, Rpf9WriteError};
pub use version::RpfVersion;

#[cfg(test)]
mod platform_letter_tests {
    use super::platform_letter_of_name;

    #[test]
    fn a_platform_extension_names_its_letter_and_nothing_else_does() {
        assert_eq!(platform_letter_of_name("models/car.ydr"), Some(b'y'));
        assert_eq!(platform_letter_of_name("prop.xdr"), Some(b'x'));
        assert_eq!(platform_letter_of_name("tex.ztd"), Some(b'z'));
        assert_eq!(platform_letter_of_name("area.pmap"), Some(b'p'));
        // Case is folded before matching.
        assert_eq!(platform_letter_of_name("CAR.YDR"), Some(b'y'));
        // A bare extension with no platform letter, and one whose tail is no stub.
        assert_eq!(platform_letter_of_name("packs/base.rpf"), None);
        assert_eq!(platform_letter_of_name("world/heightmap.dat"), None);
        assert_eq!(platform_letter_of_name("audio.awc"), None);
        assert_eq!(platform_letter_of_name("hash_00ab"), None);
        // `d` is a RAGE letter (Durango) but `at` is no stub, so `.dat` is left alone.
        assert_eq!(platform_letter_of_name("x.dat"), None);
    }
}
