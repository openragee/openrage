//! The RPF9 archive explorer: an entry listing over a decrypted table of contents.
//!
//! [`Rpf9Toc`] deciphers an RPF9's metadata and column index; this joins the columns into one row per
//! entry. The table is a column store: after the group directory comes one entry-major index array
//! (`entries x non-external-columns x index_width`), and a second array at the split word for the
//! external columns. Each index points into a column's deduplicated rows, or holds the all-ones
//! sentinel (0xFF / 0xFFFF) — a value at or past the column's row count — when the entry has no row
//! there. So entry `e`'s name64, name32, entry row and content-cipher record are read through that
//! index, not by pairing row `e` of one column with row `e` of another: the columns are deduplicated
//! and share no row order, which is why the old row-order pairing listed nothing for the four archives
//! whose name64 column is shorter than their entry count.
//!
//! Two entry-row shapes exist (a table may hold both; an entry has a row in exactly one). The 24-byte
//! row carries start, stored size, real size and a class byte; the 20-byte row, universal in the
//! nested tables, carries only a proven start. A column whose hash this reader does not know is left
//! undecoded rather than read as a row of a guessed shape, so an entry lists what the known columns
//! state and nothing invented.
//!
//! A [`NameResolver`] maps an entry's name64 back to its path, through the recovered-names vocabulary
//! and, as a shortcut, the per-build name tables; nested archive mounting lives in
//! [`super::gta6_mount`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use byteorder::{ByteOrder, LE};

use super::rpf9_toc::{GtaViKeys, Rpf9Group, Rpf9Toc};
use crate::error::Result;
use crate::gta6::hash::name_hash64;
use crate::gta6::names::Vocabulary;

/// The name64 column: 8-byte records, the Murmur3 x64_128 low-64 hash of each entry's lower-case
/// path, the engine's key for it.
pub const GROUP_NAME64: u64 = 0x73BC_B72D_CE74_F11F;
/// The name32 column: 4-byte records, the joaat of the same path.
pub const GROUP_NAME32: u64 = 0xB707_6B62_45D3_5FB5;
/// The 24-byte entry row: three u64 holding the start (16-byte units, with a class byte above bit 40),
/// the stored size (16-byte units, rounded up) and the real (inflated) size.
pub const GROUP_ENTRY_ROW: u64 = 0x33F7_0DFA_F2C5_0989;
/// The 20-byte entry row, the second row shape, universal in the nested tables. Its five little-endian
/// u32 words are `[w0 virtual size, w4 flags, w8 stored size in 16-byte pages, w12 start/16, w16
/// resource type]`; [`Gta6Archive::entry_extras`] decodes the lot into a [`Gta6NestedRow`]. The entry
/// itself still carries only the proven start, so a 20-byte row's body is read by the detector's own
/// measure, not by `w0` (a page-bucket descriptor, not a byte size, for a paged resource).
pub const GROUP_ENTRY_ROW_20: u64 = 0xBA18_0AFB_232D_1B4B;
/// The content-cipher column: 64-byte records whose head is `{u32 keyId, u16 cipherType, byte
/// iv[16]}`. An entry with such a row has an enciphered body; type 17 is opened by
/// [`super::gta6_mount::Gta6Mount::read_entry_decrypted`], other cipher types are not.
pub const GROUP_CIPHER: u64 = 0x2860_30A3_58A7_89FB;
/// The external content-hash column (`0xb6f5`): 8-byte records, a 64-bit content hash of each entry's
/// body — one per entry, near-unique, and colliding only on byte-identical bodies. A reader may show
/// it; an in-place rewrite of a body must recompute it.
pub const GROUP_CONTENT_HASH: u64 = 0xB6F5_BDB2_C66D_A2E9;
/// The chunk record column (`0xe0c1`): 8-byte records, two u32. The second u32's low half is the
/// number of 16-aligned chunks a compressed entry's body is stored in — the count a decrypted body's
/// chunk walk must reproduce. The first u32 is an unresolved byte coordinate into a chunk-length heap.
pub const GROUP_CHUNK_RECORD: u64 = 0xE0C1_2D7A_5D48_84C5;

/// The coarse content class of a 24-byte entry row, from its class byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gta6Class {
    /// `0x00`: a nested RPF9 archive.
    NestedArchive,
    /// `0x0B`: a compressed resource (it carries the chunk/compression record).
    CompressedResource,
    /// `0xFF`: media, markup or raw bytes (Bink, XML, text, a pipeline cache).
    Media,
    /// Any other class byte; `0x0E` is the one observed example.
    Other(u8),
}

impl Gta6Class {
    /// The class a 24-byte row's class byte names.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::NestedArchive,
            0x0B => Self::CompressedResource,
            0xFF => Self::Media,
            other => Self::Other(other),
        }
    }
}

/// A content-cipher record: the key id, cipher type and IV an enciphered entry's body is under. The
/// body itself is not opened here; this marks the entry and carries what the record states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gta6Cipher {
    pub key_id: u32,
    pub cipher_type: u16,
    pub iv: [u8; 16],
    /// The framing `(F0, F1, F2, F3, F4)` at record bytes 24..64: five little-endian u64 that bound
    /// the enciphered extent of the body (the head, the repeating enciphered/clear windows and the
    /// tail). See [`super::gta6_content::enciphered_ranges`].
    pub framing: [u64; 5],
}

/// One entry of an RPF9 archive, joined from the index columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gta6Entry {
    /// The entry's number, `0..entry_count`. Unlike a column row, it is the archive's own order.
    pub index: u32,
    /// The Murmur3 x64_128 low-64 hash of the entry's lower-case path; `0` for the few entries with
    /// no name64 row.
    pub name64: u64,
    /// The joaat of the same path, where the archive carries a name32 column for the entry.
    pub name32: Option<u32>,
    /// The recovered path, when a [`NameResolver`] has it; the hash alone otherwise.
    pub name: Option<String>,
    /// The entry's start, in bytes, relative to this archive's own header (absolute for a top-level
    /// archive); `None` when the entry has no entry row.
    pub start: Option<u64>,
    /// The stored (on-disk) size in bytes, rounded up to 16; `None` for a 20-byte row, whose size is
    /// not yet decoded, or no row.
    pub stored: Option<u64>,
    /// The real (inflated) size in bytes; `None` for a 20-byte row or no row.
    pub real: Option<u64>,
    /// The class byte of the 24-byte row: a coarse content class ([`Gta6Class`], via [`Self::kind`]) —
    /// `0x00` a nested archive, `0x0B` a compressed resource, `0xFF` media/raw, `0x0E` a lone other.
    /// `None` for a 20-byte row or no row.
    pub class: Option<u8>,
    /// The content-cipher record, when the entry's body is enciphered.
    pub cipher: Option<Gta6Cipher>,
}

impl Gta6Entry {
    /// The entry's real (inflated) size as a `u32`, for the listing views; `None` when it has no
    /// entry row or the size does not fit a `u32`. The full value is [`Self::real`].
    pub fn size(&self) -> Option<u32> {
        self.real.and_then(|r| u32::try_from(r).ok())
    }

    /// The entry's recovered path, or `hash_{name64:016x}` where it never resolved — the fallback a
    /// listing shows, as the engine falls back when its own vocabulary misses.
    pub fn display_name(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("hash_{:016x}", self.name64),
        }
    }

    /// The coarse content class of the entry's 24-byte row, where it has one.
    pub fn kind(&self) -> Option<Gta6Class> {
        self.class.map(Gta6Class::from_byte)
    }
}

/// The 20-byte nested entry row decoded in full: the engine's per-resource descriptor for a headerless
/// resource inside a nested table, keyed to an entry through the same index as the entry row. Kept off
/// [`Gta6Entry`] so the row stays the shape existing consumers build.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gta6NestedRow {
    /// `w0`: the resource virtual size — the flat inflated byte size for a simple resource, a packed
    /// RSC8 virtual-page descriptor (not a plain byte size) for a paged one.
    pub virtual_size: u32,
    /// `w4`: page / allocation flags; the low byte tracks the resource family.
    pub flags: u32,
    /// `w8`: the stored (on-disk) size in 16-byte pages; [`Self::stored_size`] gives the byte size.
    pub stored_pages: u32,
    /// `w16`: the resource type code (drawable `0x02`, dictionary `0x04`, bound `0x06`, cloth `0x58`,
    /// … — the code, reported without splitting any folded version).
    pub resource_type: u32,
}

impl Gta6NestedRow {
    /// The stored on-disk size in bytes: the 16-byte pages scaled up (one of which is a header).
    pub fn stored_size(&self) -> u64 {
        u64::from(self.stored_pages) * 16
    }

    /// Where a 20-byte row's enciphered body sits and how long it is stored, given the row's slot
    /// start (`w12 * 16`, already absolute or archive-relative as the caller holds it): the body
    /// begins 16 bytes past the slot (over the zero header slot) and runs `w8 * 16 - 16` bytes. The
    /// length is an upper bound within 15 bytes of the true one; the chunk walk, not it, ends the body.
    pub fn ciphered_body_extent(&self, slot_start: u64) -> (u64, u64) {
        (slot_start + 16, self.stored_size().saturating_sub(16))
    }
}

/// An entry's columns that are not carried on [`Gta6Entry`]: the external content hash and, for a
/// 20-byte nested row, its full descriptor. One per entry, in the archive's own order; read with
/// [`Gta6Archive::entry_extras`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gta6EntryExtra {
    /// The `0xb6f5` external content hash of the entry's body, where the archive carries that column.
    pub content_hash: Option<u64>,
    /// The decoded 20-byte nested row, where the entry has one (`None` for a 24-byte row or no row).
    pub nested: Option<Gta6NestedRow>,
    /// The `0xe0c1` chunk count (the record's second u32, low half) of a compressed entry, where the
    /// archive carries that column: the number of chunks the body's walk should find.
    pub chunk_count: Option<u32>,
}

/// An opened RPF9 archive's listing: its decrypted table of contents and the entries read from it.
#[derive(Debug, Clone)]
pub struct Gta6Archive {
    /// The decrypted table of contents: the header bytes, the key ids, the groups.
    pub toc: Rpf9Toc,
    /// The entries, one per `entry_count`, in the archive's own order.
    pub entries: Vec<Gta6Entry>,
}

impl Gta6Archive {
    /// Open the RPF9 archive at the start of `archive`, deciphering its table of contents with
    /// `keys`. `archive` need only hold the header and table of contents.
    pub fn open(archive: &[u8], keys: &GtaViKeys) -> Result<Self> {
        let toc = Rpf9Toc::decrypt(archive, keys)?;
        let entries = read_entries(&toc);
        Ok(Self { toc, entries })
    }

    /// Fill in each entry's name from `resolver`, where it has the hash. Entries it does not know
    /// keep `None` and show as their hash.
    pub fn resolve_names(&mut self, resolver: &NameResolver) {
        for entry in &mut self.entries {
            if entry.name.is_none() {
                entry.name = resolver.lookup(entry.name64).map(str::to_owned);
            }
        }
    }

    /// The archive's index groups, in order.
    pub fn groups(&self) -> &[Rpf9Group] {
        &self.toc.groups
    }

    /// The columns this reader does not decode into an entry row, by hash and width: the index key,
    /// the per-resource external columns and any column set a future row shape would need. A table
    /// whose entries lack a known entry-row shape shows it here, so a census or a test can name what
    /// was not read rather than silently listing those entries without a start.
    pub fn unknown_columns(&self) -> Vec<(u64, u16)> {
        self.toc
            .groups
            .iter()
            .filter(|g| {
                !matches!(
                    (g.name_hash, g.width),
                    (GROUP_NAME64, 8)
                        | (GROUP_NAME32, 4)
                        | (GROUP_ENTRY_ROW, 24)
                        | (GROUP_ENTRY_ROW_20, 20)
                        | (GROUP_CIPHER, 64)
                        | (GROUP_CONTENT_HASH, 8)
                        | (GROUP_CHUNK_RECORD, 8)
                )
            })
            .map(|g| (g.name_hash, g.width))
            .collect()
    }

    /// The per-entry extra columns — the content hash and the 20-byte nested descriptor — one per
    /// entry in the archive's own order, joined through the same entry-major index as the entries. The
    /// data is kept off [`Gta6Entry`] so the row stays the shape the CLI and app already build.
    pub fn entry_extras(&self) -> Vec<Gta6EntryExtra> {
        read_extras(&self.toc)
    }

    /// The extra columns of one entry by its index, without building the whole table: the content
    /// hash, the 20-byte nested descriptor and the chunk count. `None` when the index is out of range
    /// or the index layout is unreadable. Used by the body reader to resolve a 20-byte row's geometry.
    pub fn entry_extra(&self, index: usize) -> Option<Gta6EntryExtra> {
        let (columns, entries) = locate_columns(&self.toc)?;
        (index < entries).then(|| read_extra_at(&self.toc.bytes, &columns, index))
    }
}

fn round16(x: usize) -> usize {
    (x + 15) & !15
}

/// One column of the decrypted index, with the entry-major index-array parameters that locate the
/// row of a given entry. Reading is bounds-checked: a wrong offset yields `None`, never a panic.
struct Column {
    /// The column's records, `[data_start, data_start + count*width)` within the TOC bytes.
    data_start: usize,
    width: usize,
    count: usize,
    /// The start of this column's kind's index array (the main array, or the split for an external
    /// column).
    index_base: usize,
    /// Bytes an entry occupies in that index array (`columns_of_this_kind * index_width`).
    index_stride: usize,
    /// This column's position among its kind, in records.
    slot: usize,
    index_width: usize,
}

impl Column {
    /// Entry `e`'s record in this column, through the index array; `None` when the entry has no row
    /// (the sentinel, a value at or past the row count) or any offset falls outside the TOC bytes.
    fn row<'a>(&self, bytes: &'a [u8], e: usize) -> Option<&'a [u8]> {
        let at = self.index_base + e * self.index_stride + self.slot * self.index_width;
        let raw = bytes.get(at..at + self.index_width)?;
        let row = match self.index_width {
            1 => raw[0] as usize,
            _ => LE::read_u16(raw) as usize,
        };
        if row >= self.count {
            return None;
        }
        let o = self.data_start + row * self.width;
        bytes.get(o..o + self.width)
    }
}

/// The columns this reader knows, located for the archive's index arrays.
#[derive(Default)]
struct KnownColumns {
    name64: Option<Column>,
    name32: Option<Column>,
    row24: Option<Column>,
    row20: Option<Column>,
    cipher: Option<Column>,
    content_hash: Option<Column>,
    chunk_record: Option<Column>,
}

/// Locate every known column for the archive's entry-major index arrays, with the entry count. An
/// archive whose index layout is unreadable (an entry count of zero, an index width other than 1 or 2)
/// yields `None` rather than a guess.
fn locate_columns(toc: &Rpf9Toc) -> Option<(KnownColumns, usize)> {
    let bytes = &toc.bytes;
    // The header fields the index arrays are placed by; all lie in the first 0x40 bytes the decrypt
    // guaranteed present.
    let read_u16 = |at: usize| bytes.get(at..at + 2).map(LE::read_u16);
    let read_u32 = |at: usize| bytes.get(at..at + 4).map(LE::read_u32);
    let (split, n, m, group_count, external_count, entries) = (
        read_u32(0x08)?,
        read_u16(0x1A)?,
        read_u32(0x1C)?,
        read_u16(0x20)?,
        read_u16(0x22)?,
        read_u32(0x24)?,
    );
    let index_width = bytes.get(0x2C).copied().unwrap_or(0) as usize;
    let (entries, group_count, external_count) = (
        entries as usize,
        group_count as usize,
        external_count as usize,
    );
    if entries == 0 || !(1..=2).contains(&index_width) || external_count > group_count {
        return None;
    }

    let metadata_start = 0x40 + round16(8 * n as usize);
    let groups_start = round16(metadata_start + m as usize);
    let main_base = groups_start + 24 * group_count;
    let main_stride = (group_count - external_count) * index_width;
    let ext_base = split as usize;
    let ext_stride = external_count * index_width;

    // Each group's position among its kind, in directory order, gives its slot in the index array.
    let mut columns = KnownColumns::default();
    let (mut main_slot, mut ext_slot) = (0usize, 0usize);
    for group in &toc.groups {
        let slot = if group.external { ext_slot } else { main_slot };
        let (index_base, index_stride) = if group.external {
            (ext_base, ext_stride)
        } else {
            (main_base, main_stride)
        };
        let column = Column {
            data_start: group.data.start,
            width: group.width as usize,
            count: group.count as usize,
            index_base,
            index_stride,
            slot,
            index_width,
        };
        match (group.name_hash, group.width) {
            (GROUP_NAME64, 8) => columns.name64 = Some(column),
            (GROUP_NAME32, 4) => columns.name32 = Some(column),
            (GROUP_ENTRY_ROW, 24) => columns.row24 = Some(column),
            (GROUP_ENTRY_ROW_20, 20) => columns.row20 = Some(column),
            (GROUP_CIPHER, 64) => columns.cipher = Some(column),
            (GROUP_CONTENT_HASH, 8) => columns.content_hash = Some(column),
            (GROUP_CHUNK_RECORD, 8) => columns.chunk_record = Some(column),
            _ => {}
        }
        if group.external {
            ext_slot += 1;
        } else {
            main_slot += 1;
        }
    }

    Some((columns, entries))
}

/// Build one row per entry by joining the known columns through the entry-major index arrays. An
/// archive whose index layout is unreadable lists nothing rather than guessing.
fn read_entries(toc: &Rpf9Toc) -> Vec<Gta6Entry> {
    let Some((columns, entries)) = locate_columns(toc) else {
        return Vec::new();
    };
    (0..entries)
        .map(|e| join_entry(&toc.bytes, &columns, e))
        .collect()
}

/// Build the per-entry extra columns (the content hash and the 20-byte nested descriptor), joined
/// through the same index arrays as [`read_entries`].
fn read_extras(toc: &Rpf9Toc) -> Vec<Gta6EntryExtra> {
    let Some((columns, entries)) = locate_columns(toc) else {
        return Vec::new();
    };
    (0..entries)
        .map(|e| read_extra_at(&toc.bytes, &columns, e))
        .collect()
}

/// Entry `e`'s extra columns, read through the located index arrays.
fn read_extra_at(bytes: &[u8], columns: &KnownColumns, e: usize) -> Gta6EntryExtra {
    Gta6EntryExtra {
        content_hash: columns
            .content_hash
            .as_ref()
            .and_then(|c| c.row(bytes, e))
            .map(LE::read_u64),
        nested: columns
            .row20
            .as_ref()
            .and_then(|c| c.row(bytes, e))
            .map(read_nested_row),
        // The chunk count is the record's second u32's low half.
        chunk_count: columns
            .chunk_record
            .as_ref()
            .and_then(|c| c.row(bytes, e))
            .map(|row| LE::read_u32(&row[4..]) & 0xFFFF),
    }
}

/// Decode a 20-byte nested entry row's five little-endian u32 words into its descriptor.
fn read_nested_row(row: &[u8]) -> Gta6NestedRow {
    Gta6NestedRow {
        virtual_size: LE::read_u32(&row[0..]),
        flags: LE::read_u32(&row[4..]),
        stored_pages: LE::read_u32(&row[8..]),
        resource_type: LE::read_u32(&row[16..]),
    }
}

/// Join entry `e`'s columns into one row.
fn join_entry(bytes: &[u8], columns: &KnownColumns, e: usize) -> Gta6Entry {
    let name64 = columns
        .name64
        .as_ref()
        .and_then(|c| c.row(bytes, e))
        .map(LE::read_u64)
        .unwrap_or(0);
    let name32 = columns
        .name32
        .as_ref()
        .and_then(|c| c.row(bytes, e))
        .map(LE::read_u32);
    let cipher = columns
        .cipher
        .as_ref()
        .and_then(|c| c.row(bytes, e))
        .map(read_cipher);

    // An entry has a row in exactly one of the two entry-row shapes; the 24-byte row is read in full,
    // the 20-byte row for its proven start alone.
    let mut start = None;
    let mut stored = None;
    let mut real = None;
    let mut class = None;
    if let Some(row) = columns.row24.as_ref().and_then(|c| c.row(bytes, e)) {
        let a = LE::read_u64(&row[0..]);
        let b = LE::read_u64(&row[8..]);
        let c = LE::read_u64(&row[16..]);
        start = Some((a & ((1u64 << 40) - 1)) * 16);
        class = Some(((a >> 40) & 0xFF) as u8);
        // A corrupt row can state a stored size that overflows 16-byte units; a malformed file must
        // not panic under overflow checks, so an unrepresentable size reads as absent.
        stored = b.checked_mul(16);
        real = Some(c);
    } else if let Some(row) = columns.row20.as_ref().and_then(|c| c.row(bytes, e)) {
        let start_units = u64::from(LE::read_u32(&row[12..]));
        start = Some(start_units * 16);
    }

    Gta6Entry {
        index: e as u32,
        name64,
        name32,
        name: None,
        start,
        stored,
        real,
        class,
        cipher,
    }
}

/// Read a content-cipher record from its 64-byte row: the key id, cipher type and IV at the head, then
/// the five little-endian u64 framing words at bytes 24..64.
fn read_cipher(row: &[u8]) -> Gta6Cipher {
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&row[6..22]);
    let mut framing = [0u64; 5];
    for (i, f) in framing.iter_mut().enumerate() {
        *f = LE::read_u64(&row[24 + i * 8..]);
    }
    Gta6Cipher {
        key_id: LE::read_u32(&row[0..]),
        cipher_type: LE::read_u16(&row[4..]),
        iv,
        framing,
    }
}

/// Resolves shared word candidates and user-only name tables; build paths and file extensions stay local.
#[derive(Debug, Default, Clone)]
pub struct NameResolver {
    /// What a kept names cache holds, searched where it lies; every name it holds was named first.
    kept: Arc<cache::Kept>,
    /// The names held beyond `kept`, and the names of hashes neither holds.
    vocabulary: Vocabulary,
    shortcut: HashMap<u64, String>,
    /// `kept`'s held names and `vocabulary` together, built the first time they are asked for.
    whole: OnceLock<Vocabulary>,
}

impl NameResolver {
    /// An empty resolver, which names nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// A resolver over `vocabulary`, the recovered-names corpus hashed by `name_hash64` at load.
    pub fn from_vocabulary(vocabulary: Vocabulary) -> Self {
        Self {
            vocabulary,
            ..Self::default()
        }
    }

    /// A resolver over just the `vocabulary.txt` under `dir`, with no shortcut tables. The shortcut
    /// tables are derived from one build's own tables and so resolve by construction; a caller that
    /// must measure resolution against the corpus alone uses this.
    pub fn load_vocabulary(dir: &Path) -> Self {
        Self::from_vocabulary(read_vocabulary(&dir.join("vocabulary.txt")))
    }

    /// A resolver over `dir`: the recovered-names `vocabulary.txt`, plus the per-build name tables as
    /// a shortcut where present. Any file that is absent is passed over, so a partial corpus works.
    pub fn load(dir: &Path) -> Self {
        let mut resolver = Self::load_vocabulary(dir);
        if let Ok(text) = std::fs::read_to_string(dir.join("toc-entries-resolved.tsv")) {
            resolver.add_toc_entries_tsv(&text);
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("resolved-names.tsv")) {
            resolver.add_hash_name_tsv(&text);
        }
        resolver
    }

    /// Shared words supplemented by `dir`, or the folder [`super::GTA6_NAMES_ENV`] names.
    pub fn load_with_defaults(dir: Option<&Path>) -> Self {
        Self::load_with_sources(dir, None)
    }

    /// Loads external names, then the local cache and vocabulary, then the shared word candidates.
    pub fn load_with_sources(dir: Option<&Path>, local: Option<&Path>) -> Self {
        let external = dir
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os(super::GTA6_NAMES_ENV).map(Into::into));
        let mut names = external.as_deref().map(Self::load).unwrap_or_default();
        if let Some(local) = local {
            match cache::read_cache(local) {
                Ok(cached) => {
                    for (hash, name) in cached {
                        if names.lookup(hash).is_none() {
                            names.insert(hash, name);
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "could not read the local GTA VI names cache"),
            }
            names.merge_vocabulary(&read_vocabulary(&local.join("vocabulary.txt")));
        }
        names.merge_vocabulary(&Vocabulary::bundled());
        names
    }

    /// The vocabulary including verified cache names, for metadata and texture hash lookups.
    // ⚠ Over a kept cache this copies every name it holds, once: the lookups never need it.
    pub fn vocabulary(&self) -> &Vocabulary {
        if self.kept.is_empty() {
            return &self.vocabulary;
        }
        self.whole.get_or_init(|| {
            let mut whole = self.kept.vocabulary();
            whole.merge(&self.vocabulary);
            whole
        })
    }

    /// Fold `other` into this resolver's vocabulary, so names a caller harvested elsewhere (its own
    /// config-dir vocabulary) resolve alongside the ones loaded from a folder.
    pub fn merge_vocabulary(&mut self, other: &Vocabulary) {
        self.whole = OnceLock::new();
        self.vocabulary.merge(other);
    }

    /// Whether the resolver knows no names.
    pub fn is_empty(&self) -> bool {
        self.kept.is_empty() && self.vocabulary.is_empty() && self.shortcut.is_empty()
    }

    /// How many names the resolver holds across both sources.
    pub fn len(&self) -> usize {
        // `vocabulary` may hold again a word `kept` holds; `shortcut` holds none `kept` names.
        let words = if self.kept.is_empty() {
            self.vocabulary.len()
        } else {
            (self.vocabulary.names())
                .filter(|name| self.kept.get(name_hash64(name.as_bytes())).is_none())
                .count()
        };
        let shortcut =
            (self.shortcut.keys()).filter(|hash| self.vocabulary.name64(**hash).is_none());
        self.kept.len() + words + shortcut.count()
    }

    /// The path whose name64 is `hash`: the shortcut first, then the vocabulary.
    pub fn lookup(&self, hash: u64) -> Option<&str> {
        if let Some((name, _)) = self.kept.get(hash) {
            return Some(name);
        }
        if let Some(name) = self.shortcut.get(&hash) {
            return Some(name);
        }
        self.vocabulary.name64(hash)
    }

    /// The path whose name32 is `hash`, from the vocabulary; `None` where two held names share it.
    pub fn lookup_name32(&self, hash: u32) -> Option<&str> {
        self.vocabulary().name32(hash)
    }

    /// How many of `entries` the resolver names, and how many there are: a coverage report.
    pub fn coverage(&self, entries: &[Gta6Entry]) -> (usize, usize) {
        let resolved = entries
            .iter()
            .filter(|e| self.lookup(e.name64).is_some())
            .count();
        (resolved, entries.len())
    }

    /// Record a `hash -> name` shortcut, keeping the first name seen for a hash.
    pub fn insert(&mut self, hash: u64, name: impl Into<String>) {
        let name = name.into();
        self.whole = OnceLock::new();
        // ⚠ A hash the vocabulary already names keeps that name, and a hash the kept cache names
        // keeps its name: the first name a hash is given wins either way.
        let kept = self.kept.get(hash).map(|(_, held)| held);
        let held = kept == Some(true) || self.vocabulary.name64(hash).is_some();
        if kept != Some(true) && name_hash64(name.as_bytes()) == hash {
            self.vocabulary.add(&name, None);
        }
        if !held && kept.is_none() {
            self.shortcut.entry(hash).or_insert(name);
        }
    }

    /// Add the entries of a `hash<TAB>name` table, as `resolved-names.tsv` holds them: the first
    /// field a `0x…` hash (8 or 16 hex digits), the second the path. Lines that do not parse are
    /// skipped, so a header row or a comment does no harm.
    pub fn add_hash_name_tsv(&mut self, text: &str) {
        for line in text.lines() {
            let mut fields = line.split('\t');
            let (Some(hash), Some(name)) = (fields.next(), fields.next()) else {
                continue;
            };
            if let (Some(hash), false) = (parse_hash(hash), name.is_empty()) {
                self.insert(hash, name);
            }
        }
    }

    /// Add the entries of the per-archive name table (`toc-entries-resolved.tsv`): a header row,
    /// then `archive<TAB>entry<TAB>name64<TAB>name32<TAB>name<TAB>source`. The name64 (column 3) is
    /// mapped to the name (column 5) when both are present; other rows are skipped.
    pub fn add_toc_entries_tsv(&mut self, text: &str) {
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let [_, _, name64, _, name, ..] = fields.as_slice() else {
                continue;
            };
            if let (Some(hash), false) = (parse_hash(name64), name.is_empty()) {
                self.insert(hash, *name);
            }
        }
    }
}

/// The vocabulary at `file`: the kept `openrage gta6 names 1` format where it is one, else a plain
/// list of one name per line (the shape the owner's recovered-names corpora take), each hashed with
/// `name_hash64` as [`Vocabulary::add`] takes it. Blank lines and `//` comments are skipped. A file
/// that is present but yields no name is warned about, since it then resolves nothing.
fn read_vocabulary(file: &Path) -> Vocabulary {
    let kept = Vocabulary::load(file);
    if !kept.is_empty() {
        return kept;
    }
    let Ok(text) = std::fs::read_to_string(file) else {
        return kept; // Absent or unreadable: an empty vocabulary, silently.
    };
    let mut vocabulary = Vocabulary::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        vocabulary.add(line, None);
    }
    if vocabulary.is_empty() {
        tracing::warn!(file = %file.display(), "names vocabulary held no usable names");
    }
    vocabulary
}

/// A `0x…` hash of 1 to 16 hex digits as a `u64`; `None` for anything else.
fn parse_hash(field: &str) -> Option<u64> {
    let digits = field
        .strip_prefix("0x")
        .or_else(|| field.strip_prefix("0X"))?;
    if !matches!(digits.len(), 1..=16) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
#[path = "gta6_archive_tests.rs"]
mod tests;

#[path = "gta6_name_cache.rs"]
mod cache;
