//! Writing a new RPF9 (GTA VI) archive from an existing one: a copy through decrypt and re-encrypt,
//! an entry replaced, removed or renamed, and a defragment, recursing into nested archives with the
//! nested table written before its parent's row.
//!
//! ⚠ This is a structural tool. The game verifies a signature over the table of contents when it
//! first mounts an archive, and only the game's maker can sign one, so an archive whose table this
//! changes carries a stale signature and the game refuses it. What OpenRage checks is structure: the
//! result reopens, lists and reads back.
//!
//! Rows are addressed through the entry-major index, never by position. A table whose rows keep their
//! count is rewritten in place; one that gains or loses a row or an entry is laid out anew, with the
//! heaps its cell-relative columns point into, which is allowed only where laying out the unedited
//! table reproduces it byte for byte. Preserved as read: the header (its `+0x28` and `+0x2D` among
//! it), the metadata with its type-6 signature and type-9 descriptors, the names heap (opaque; a
//! renamed or added entry gets no heap text), and an unchanged entry's `0xb6f5` content hash. A
//! changed entry's `0xb6f5` is written as zero, its algorithm being unknown, and a replaced entry is
//! stored clear, so its content-cipher row is dropped.
//!
//! A headerless resource (a 20-byte row) is replaced by its loose form, stored as the zero slot and
//! its chunks under a row and a mode-2 chunk table authored from them. Its `0xd07a` and `0x354d`
//! rows, which nothing in the body states, are carried; its dependency list is carried unless the
//! caller names its keys, and a key the type-4 table lacks is added to it. An added entry goes where
//! its name32 (else its name64) keeps the entries ascending, as every measured table holds them, and
//! gets neither of those two rows. A body written into a table whose chunk tables take mode 4 takes
//! mode 4 too, as no measured table mixes it with another.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use byteorder::{ByteOrder, LE};
use flate2::write::ZlibEncoder;
use flate2::Compression;

use super::gta6_archive::{
    Gta6Class, Gta6Entry, GROUP_CHUNK_RECORD, GROUP_CIPHER, GROUP_ENTRY_ROW, GROUP_ENTRY_ROW_20,
    GROUP_NAME32, GROUP_NAME64,
};
use super::gta6_chunk_table::{author, Chunk, Table as ChunkTable};
use super::gta6_form_h;
use super::gta6_mount::{open_at, Gta6Mount};
use super::gta6_write::Rpf9Builder;
use super::rpf9_scan::{read_rpf9_body, RPF9_SLOT, STREAM_CAP};
use super::rpf9_toc::{GtaViKeys, Rpf9Toc};
use super::rpf9_write::{write_rpf9_resource_chunks, Rpf9Sections};
use crate::error::{IoError, Result};
use crate::gta6::hash::{name_hash32, name_hash64};

#[path = "gta6_container_table.rs"]
mod table;
use table::Table;

#[path = "gta6_container_build.rs"]
mod build;
pub use build::{build, Rpf9IndexKey};
use build::{column, insert_entry, set_new_body};

#[path = "gta6_container_deps.rs"]
mod deps;
use deps::{restate_in_mode4, set_dependencies, takes_mode4, MODE4};

/// The inflated size of one chunk of a compressed body.
const CHUNK: usize = 0x10000;
/// A 24-byte row's start field, in 16-byte units, below its class byte at bit 40.
const START_MASK: u64 = (1 << 40) - 1;
/// The chunk-table mode of a 24-byte row.
const ROW24_CHUNK_MODE: u8 = 3;
/// The chunk-table mode of a 20-byte row.
const ROW20_CHUNK_MODE: u8 = 2;

fn bad(why: impl Into<String>) -> IoError {
    IoError::Invalid {
        what: "an RPF9 archive write".into(),
        why: why.into(),
    }
}

fn round8(x: usize) -> usize {
    (x + 7) & !7
}

fn round16(x: u64) -> u64 {
    (x + 15) & !15
}

/// One run of an output archive, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rpf9Piece {
    /// Bytes the writer produced: an enciphered table or a new body.
    Bytes(Vec<u8>),
    /// `len` bytes of the source archive from `from`, copied unchanged.
    Copy { from: u64, len: u64 },
    /// `len` zero bytes.
    Zero(u64),
}

impl Rpf9Piece {
    /// The bytes this run adds to the output.
    pub fn size(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::Copy { len, .. } | Self::Zero(len) => *len,
        }
    }
}

/// Append `piece`, merging it into the last run where both copy contiguous source or both are zero.
fn push(out: &mut Vec<Rpf9Piece>, piece: Rpf9Piece) {
    if piece.size() == 0 {
        return;
    }
    let merged = match (out.last_mut(), &piece) {
        (
            Some(Rpf9Piece::Copy { from, len }),
            Rpf9Piece::Copy {
                from: next,
                len: more,
            },
        ) if *from + *len == *next => {
            *len += more;
            true
        }
        (Some(Rpf9Piece::Zero(len)), Rpf9Piece::Zero(more)) => {
            *len += more;
            true
        }
        _ => false,
    };
    if !merged {
        out.push(piece);
    }
}

/// An output archive as a run of pieces, and how many tables the edits changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rpf9Plan {
    pub pieces: Vec<Rpf9Piece>,
    /// Tables whose plaintext the edits changed; each such table's signature is stale.
    pub tables_changed: usize,
    /// Bodies the edits wrote anew, nested archives among them.
    pub bodies_changed: usize,
}

impl Rpf9Plan {
    /// The output archive's length in bytes.
    pub fn size(&self) -> u64 {
        self.pieces.iter().map(Rpf9Piece::size).sum()
    }

    /// Write the archive to `out`, copying unchanged runs from `src`, the archive it was planned from.
    pub fn write_to<R: Read + Seek, W: Write>(&self, src: &mut R, out: &mut W) -> Result<u64> {
        for piece in &self.pieces {
            match piece {
                Rpf9Piece::Bytes(bytes) => out.write_all(bytes)?,
                Rpf9Piece::Copy { from, len } => {
                    src.seek(SeekFrom::Start(*from))?;
                    let copied = std::io::copy(&mut Read::take(&mut *src, *len), out)?;
                    if copied != *len {
                        return Err(bad(format!(
                            "the source ended {copied} bytes into a {len}-byte run at {from:#x}"
                        )));
                    }
                }
                Rpf9Piece::Zero(len) => {
                    std::io::copy(&mut Read::take(std::io::repeat(0), *len), out)?;
                }
            }
        }
        Ok(self.size())
    }
}

enum Op {
    Replace(Vec<u8>),
    ReplaceResource {
        loose: Vec<u8>,
        dependencies: Option<Vec<u64>>,
    },
    Remove,
    Rename(String),
    Add {
        name: String,
        body: Rpf9NewBody,
        dependencies: Option<Vec<u64>>,
    },
}

/// The body of an entry the writer adds, and the row it is stored under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rpf9NewBody {
    /// Bytes stored as 64 KiB zlib chunks under a 24-byte row of class `0xFF` and a chunk table.
    Chunked(Vec<u8>),
    /// A loose resource, as `write_rpf9_resource` writes one, of `type_code`, under a 20-byte row.
    Resource { type_code: u32, loose: Vec<u8> },
    /// An RPF9 archive, stored as it is under a 24-byte row of class `0x00`.
    Archive(Vec<u8>),
}

/// Writes RPF9 archives; a table it changes has a stale signature, and the game refuses that archive.
#[derive(Default)]
pub struct Rpf9ArchiveWriter {
    /// Each edit's entry path: entry indices from the top-level table, one per nesting level.
    edits: Vec<(Vec<u32>, Op)>,
    defragment: bool,
}

impl Rpf9ArchiveWriter {
    /// A writer with no edits, which copies an archive through decrypt and re-encrypt.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the body of the entry at `path` (entry indices, top level first) with `body`.
    pub fn replace(&mut self, path: &[u32], body: Vec<u8>) -> &mut Self {
        self.edits.push((path.to_vec(), Op::Replace(body)));
        self
    }

    /// Replace the headerless resource at `path` by its loose form; `None` keys carry its list.
    pub fn replace_resource(
        &mut self,
        path: &[u32],
        loose: Vec<u8>,
        dependencies: Option<Vec<u64>>,
    ) -> &mut Self {
        self.edits.push((
            path.to_vec(),
            Op::ReplaceResource {
                loose,
                dependencies,
            },
        ));
        self
    }

    /// Add an entry `name` holding `body` to the archive at `archive`, empty for the top level.
    pub fn add(&mut self, archive: &[u32], name: &str, body: Rpf9NewBody) -> &mut Self {
        self.push_add(archive, name, body, None)
    }

    /// As [`add`](Self::add), the new entry's dependency list the keys `dependencies`.
    pub fn add_with_dependencies(
        &mut self,
        archive: &[u32],
        name: &str,
        body: Rpf9NewBody,
        dependencies: Vec<u64>,
    ) -> &mut Self {
        self.push_add(archive, name, body, Some(dependencies))
    }

    fn push_add(
        &mut self,
        archive: &[u32],
        name: &str,
        body: Rpf9NewBody,
        dependencies: Option<Vec<u64>>,
    ) -> &mut Self {
        self.edits.push((
            archive.to_vec(),
            Op::Add {
                name: name.to_owned(),
                body,
                dependencies,
            },
        ));
        self
    }

    /// Remove the entry at `path`.
    pub fn remove(&mut self, path: &[u32]) -> &mut Self {
        self.edits.push((path.to_vec(), Op::Remove));
        self
    }

    /// Rename the entry at `path`: its name64 and name32 become the engine's hashes of `name`.
    pub fn rename(&mut self, path: &[u32], name: &str) -> &mut Self {
        self.edits
            .push((path.to_vec(), Op::Rename(name.to_owned())));
        self
    }

    /// Pack the top-level bodies in order, closing every gap; only starts and the table change.
    pub fn defragment(&mut self) -> &mut Self {
        self.defragment = true;
        self
    }

    /// Plan the new archive from `src`, the archive the edits address. Nothing is written.
    pub fn plan<R: Read + Seek>(&self, src: &mut R, keys: &GtaViKeys) -> Result<Rpf9Plan> {
        let len = src.seek(SeekFrom::End(0))?;
        let edits: Vec<(&[u32], &Op)> = self.edits.iter().map(|(p, o)| (p.as_slice(), o)).collect();
        let mut changed = Changes::default();
        let pieces = plan_archive(
            src,
            keys,
            &mut changed,
            (0, len),
            &edits,
            self.defragment,
            true,
        )?;
        Ok(Rpf9Plan {
            pieces,
            tables_changed: changed.tables,
            bodies_changed: changed.bodies,
        })
    }
}

/// How many tables and bodies an archive's edits changed, nested archives included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Changes {
    tables: usize,
    bodies: usize,
}

/// The pieces of one archive, `[base, base + len)` of `src`, after `edits` (paths relative to it).
fn plan_archive<R: Read + Seek>(
    src: &mut R,
    keys: &GtaViKeys,
    changed: &mut Changes,
    (base, len): (u64, u64),
    edits: &[(&[u32], &Op)],
    defrag: bool,
    top: bool,
) -> Result<Vec<Rpf9Piece>> {
    let archive = open_at(src, base, keys)?;
    let toc_end = archive.toc.bytes.len() as u64;
    let names = u64::from(LE::read_u32(&archive.toc.bytes[0x0C..]));
    let names_at = len
        .checked_sub(names)
        .filter(|&at| at >= toc_end)
        .ok_or_else(|| bad("the names heap overlaps the table of contents"))?;
    if edits.is_empty() && !defrag {
        return Ok(if top {
            vec![
                Rpf9Piece::Bytes(Rpf9Builder::serialize(&archive.toc, keys)?),
                Rpf9Piece::Copy {
                    from: base + toc_end,
                    len: len - toc_end,
                },
            ]
        } else {
            vec![Rpf9Piece::Copy { from: base, len }]
        });
    }

    let mut table = Table::from_toc(&archive.toc)?;
    let mode4 = takes_mode4(&table);
    let mount = Gta6Mount { base, archive };
    let entries = table.index.len();
    let mut direct = Vec::new();
    let mut adds = Vec::new();
    let mut nested: BTreeMap<usize, Vec<(&[u32], &Op)>> = BTreeMap::new();
    for &(path, op) in edits {
        // An add names the archive it goes into, every other edit an entry.
        let (&head, rest) = match (path.split_first(), op) {
            (
                None,
                Op::Add {
                    name,
                    body,
                    dependencies,
                },
            ) => {
                adds.push((name.as_str(), body, dependencies.as_deref()));
                continue;
            }
            (None, _) => return Err(bad("an edit names no entry")),
            (Some(split), _) => split,
        };
        let head = head as usize;
        if head >= entries {
            return Err(bad(format!(
                "the archive has {entries} entries, so it has no entry {head}"
            )));
        }
        if rest.is_empty() && !matches!(op, Op::Add { .. }) {
            direct.push((head, op));
        } else {
            nested.entry(head).or_default().push((rest, op));
        }
    }

    // A nested archive is written first; its parent's row then states its new extent.
    for (head, sub) in nested {
        let entry = &mount.archive.entries[head];
        let (Some(c), Some(Gta6Class::NestedArchive), Some(start), Some(stored)) = (
            table.col(GROUP_ENTRY_ROW, 24),
            entry.kind(),
            entry.start,
            entry.stored,
        ) else {
            return Err(bad(format!("entry {head} is not a nested archive")));
        };
        let before = *changed;
        let child = (base + start, stored);
        let pieces = plan_archive(src, keys, changed, child, &sub, false, false)?;
        let new_len: u64 = pieces.iter().map(Rpf9Piece::size).sum();
        if *changed == before && new_len == stored {
            continue;
        }
        let mut cell = table
            .cell(head, c)
            .ok_or_else(|| bad(format!("entry {head} has no entry row")))?
            .to_vec();
        LE::write_u64(&mut cell[8..], new_len / 16);
        LE::write_u64(&mut cell[16..], new_len);
        table.set_body(head, c, cell, pieces, new_len, (start, stored));
        table.zero_hash(head);
    }

    let mut removed = Vec::new();
    for (e, op) in direct {
        match op {
            Op::Remove => removed.push(e),
            Op::Rename(name) => rename(&mut table, e, name)?,
            Op::Replace(body) => replace(&mut table, &mount, src, keys, e, body, mode4)?,
            Op::ReplaceResource {
                loose,
                dependencies,
            } => replace_resource(
                &mut table,
                &mount,
                src,
                e,
                loose,
                dependencies.as_deref(),
                mode4,
            )?,
            Op::Add { .. } => return Err(bad("an add names an archive, not an entry")),
        }
    }
    removed.sort_unstable();
    removed.dedup();
    if removed.len() >= entries && adds.is_empty() {
        return Err(bad("an archive must keep at least one entry"));
    }
    // Last first, so each index still names the entry it named in the table read.
    for &e in removed.iter().rev() {
        table.remove(e);
    }
    for (name, body, dependencies) in adds {
        let e = insert_entry(&mut table, name)?;
        set_new_body(&mut table, e, body)?;
        if mode4 {
            restate_in_mode4(&mut table, e)?;
        }
        if let Some(keys) = dependencies {
            set_dependencies(&mut table, e, keys)?;
        }
    }

    // The table's length does not depend on the starts it states.
    let toc_len = table.emit()?.len() as u64;
    let mut places = Vec::new();
    for (c, r) in table.bodies() {
        let (at, extent) = table.extent(c, r);
        let place = match &table.columns[c].rows[r].body {
            Some(fresh) => Place {
                c,
                r,
                // An added row states no start, as no body starts at 0; it goes after the rest.
                at: if at == 0 { u64::MAX } else { at },
                len: fresh.len,
                keep: fresh
                    .old
                    .filter(|&(_, ext)| fresh.len <= ext)
                    .map(|(s, _)| s),
                copy: false,
                new: 0,
            },
            None => {
                if at < toc_end || at.checked_add(extent).is_none_or(|end| end > names_at) {
                    return Err(bad(format!(
                        "an entry row states a body at {at:#x} of {extent} bytes, outside the \
                         archive"
                    )));
                }
                Place {
                    c,
                    r,
                    at,
                    len: extent,
                    keep: Some(at),
                    copy: true,
                    new: 0,
                }
            }
        };
        places.push(place);
    }
    changed.bodies += places.iter().filter(|p| !p.copy).count();
    let first = places
        .iter()
        .map(|p| p.at)
        .filter(|&at| at != u64::MAX)
        .min();
    // A table that outgrew the space before the first body takes the archive packed.
    let pack = defrag || first.is_some_and(|f| toc_len > f);
    let names_new = if pack {
        places.sort_by_key(|p| (p.at, p.c, p.r));
        let mut cursor = round16(toc_len).max(first.unwrap_or(0));
        for p in &mut places {
            p.new = cursor;
            cursor = round16(cursor + p.len);
        }
        cursor
    } else {
        let tail = places
            .iter()
            .filter_map(|p| p.keep.map(|s| s + p.len))
            .max()
            .unwrap_or(0);
        let mut cursor = round16(tail.max(toc_len));
        for p in &mut places {
            p.new = match p.keep {
                Some(at) => at,
                None => {
                    let at = cursor;
                    cursor = round16(at + p.len);
                    at
                }
            };
        }
        let end = places.iter().map(|p| p.new + p.len).max().unwrap_or(0);
        if end > names_at {
            round16(end)
        } else {
            names_at
        }
    };
    for p in &places {
        table.set_start(p.c, p.r, p.new)?;
    }

    let plain = table.emit()?;
    if plain != mount.archive.toc.bytes {
        changed.tables += 1;
    }
    let mut head = Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: plain,
            key_id: mount.archive.toc.key_id,
            registry_id: mount.archive.toc.registry_id,
            entry_count: table.index.len() as u32,
            groups: Vec::new(),
        },
        keys,
    )?;
    // In place, a shorter table's old ciphertext tail reads as zero rather than stale bytes.
    if !pack && (head.len() as u64) < toc_end {
        head.resize(toc_end as usize, 0);
    }

    let mut segments: Vec<(u64, Vec<Rpf9Piece>)> = vec![(0, vec![Rpf9Piece::Bytes(head)])];
    for p in &places {
        let pieces = if p.copy {
            vec![Rpf9Piece::Copy {
                from: base + p.at,
                len: p.len,
            }]
        } else {
            table.columns[p.c].rows[p.r]
                .body
                .take()
                .map(|fresh| fresh.pieces)
                .unwrap_or_default()
        };
        segments.push((p.new, pieces));
    }
    segments.push((
        names_new,
        vec![Rpf9Piece::Copy {
            from: base + names_at,
            len: names,
        }],
    ));
    segments.sort_by_key(|s| s.0);

    // A gap keeps the source's bytes where nothing moved, and is zero when packed or past the old end.
    let mut out = Vec::new();
    let mut cur = 0u64;
    for (at, pieces) in segments {
        if at < cur {
            return Err(bad(format!("two bodies overlap at {at:#x}")));
        }
        let copy_end = if pack { cur } else { at.min(names_at).max(cur) };
        push(
            &mut out,
            Rpf9Piece::Copy {
                from: base + cur,
                len: copy_end - cur,
            },
        );
        push(&mut out, Rpf9Piece::Zero(at - copy_end));
        cur = at;
        for piece in pieces {
            cur += piece.size();
            push(&mut out, piece);
        }
    }
    Ok(out)
}

/// A body's place in a written archive: its row, its start in the table read, its length, the start
/// it keeps (an unchanged body, or a new one that fits its old extent), and where it goes.
struct Place {
    c: usize,
    r: usize,
    at: u64,
    len: u64,
    keep: Option<u64>,
    copy: bool,
    new: u64,
}

/// Rename entry `e`: name64 and name32 change together; an entry with no name32 row keeps none.
fn rename(table: &mut Table, e: usize, name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(bad("an entry cannot be renamed to an empty name"));
    }
    let c64 = table
        .col(GROUP_NAME64, 8)
        .ok_or_else(|| bad("the archive has no name64 column"))?;
    let h64 = name_hash64(name.as_bytes()).to_le_bytes();
    if (0..table.index.len()).any(|o| o != e && table.cell(o, c64) == Some(&h64[..])) {
        return Err(bad(format!("another entry is already named {name:?}")));
    }
    table.set_cell(e, c64, Some(h64.to_vec()), Vec::new());
    if let Some(c32) = table.col(GROUP_NAME32, 4) {
        if table.row_of(e, c32).is_some() {
            let h32 = name_hash32(name.as_bytes()).to_le_bytes();
            table.set_cell(e, c32, Some(h32.to_vec()), Vec::new());
        }
    }
    Ok(())
}

/// Replace entry `e`'s body: a chunk-record entry is stored as 64 KiB zlib chunks with an authored
/// chunk table, in mode 4 where the table's take it, any other 24-byte row raw. A new body equal to
/// the old one changes nothing.
fn replace<R: Read + Seek>(
    table: &mut Table,
    mount: &Gta6Mount,
    src: &mut R,
    keys: &GtaViKeys,
    e: usize,
    body: &[u8],
    mode4: bool,
) -> Result<()> {
    if table
        .col(GROUP_ENTRY_ROW_20, 20)
        .is_some_and(|c| table.row_of(e, c).is_some())
    {
        return Err(bad(format!(
            "entry {e} is a headerless resource with a 20-byte row: replace it with its loose form"
        )));
    }
    let entry = &mount.archive.entries[e];
    let (Some(c), Some(start), Some(stored), Some(class)) = (
        table
            .col(GROUP_ENTRY_ROW, 24)
            .filter(|&c| table.row_of(e, c).is_some()),
        entry.start,
        entry.stored,
        entry.class,
    ) else {
        return Err(bad(format!("entry {e} has no entry row")));
    };
    if Gta6Class::from_byte(class) == Gta6Class::NestedArchive {
        return Err(bad(format!(
            "entry {e} is a nested archive: edit the entries inside it"
        )));
    }
    let chunked = table
        .col(GROUP_CHUNK_RECORD, 8)
        .filter(|&k| table.row_of(e, k).is_some());
    if entry.real == Some(body.len() as u64)
        && old_body(mount, src, keys, entry, chunked.is_some()).as_deref() == Some(body)
    {
        return Ok(());
    }
    let stored_bytes = match chunked {
        Some(k) => {
            let mode = if mode4 { MODE4 } else { ROW24_CHUNK_MODE };
            let (bytes, u1, words) = stored_chunked_in(body, mode)?;
            table.set_chunks(e, k, u1, &words);
            bytes
        }
        None => {
            let mut bytes = body.to_vec();
            bytes.resize(round16(bytes.len() as u64) as usize, 0);
            check_reads_back(&bytes, body)?;
            bytes
        }
    };
    let len = stored_bytes.len() as u64;
    let mut cell = table
        .cell(e, c)
        .ok_or_else(|| bad(format!("entry {e} has no entry row")))?
        .to_vec();
    LE::write_u64(&mut cell[8..], len / 16);
    LE::write_u64(&mut cell[16..], body.len() as u64);
    table.set_body(
        e,
        c,
        cell,
        vec![Rpf9Piece::Bytes(stored_bytes)],
        len,
        (start, stored),
    );
    changed_clear(table, e);
    Ok(())
}

/// Mark entry `e`'s body changed and stored clear: its content hash zero, its cipher row dropped.
fn changed_clear(table: &mut Table, e: usize) {
    table.zero_hash(e);
    if let Some(k) = table.col(GROUP_CIPHER, 64) {
        if table.row_of(e, k).is_some() {
            table.set_cell(e, k, None, Vec::new());
        }
    }
}

/// Replace headerless resource `e` with `loose`: its row and chunk table (mode 2, or 4 where the
/// table's take it) authored from the chunks, its dependency list set from `dependencies` where
/// given. The same stored body and row change nothing.
fn replace_resource<R: Read + Seek>(
    table: &mut Table,
    mount: &Gta6Mount,
    src: &mut R,
    e: usize,
    loose: &[u8],
    dependencies: Option<&[u64]>,
    mode4: bool,
) -> Result<()> {
    let (c, r) = table
        .col(GROUP_ENTRY_ROW_20, 20)
        .and_then(|c| table.row_of(e, c).map(|r| (c, r)))
        .ok_or_else(|| {
            bad(format!(
                "entry {e} has no 20-byte row, so it is no headerless resource"
            ))
        })?;
    let old = table.columns[c].rows[r].cell.clone();
    let (start, extent) = table.extent(c, r);
    let (mut row, stored, chunks) = headerless(LE::read_u32(&old[16..]), loose)?;
    if let Some(keys) = dependencies {
        set_dependencies(table, e, keys)?;
    }
    row[12..16].copy_from_slice(&old[12..16]);
    let mut held = vec![0u8; stored.len()];
    let unchanged = row[..] == old[..]
        && src.seek(SeekFrom::Start(mount.base + start)).is_ok()
        && src.read_exact(&mut held).is_ok()
        && held == stored;
    if unchanged {
        return Ok(());
    }
    let k = column(table, GROUP_CHUNK_RECORD, 8, "chunk-record")?;
    let (u1, words) = author(&ChunkTable {
        mode: if mode4 { MODE4 } else { ROW20_CHUNK_MODE },
        chunks,
    })?;
    table.set_chunks(e, k, u1, &words);
    let len = stored.len() as u64;
    table.set_body(
        e,
        c,
        row.to_vec(),
        vec![Rpf9Piece::Bytes(stored)],
        len,
        (start, extent),
    );
    changed_clear(table, e);
    Ok(())
}

/// `loose`, a loose resource, stored under a 20-byte row of `type_code`: the row (its start left
/// zero), the body (the zero slot, then the chunks) and the chunks its table states.
fn headerless(type_code: u32, loose: &[u8]) -> Result<([u8; 20], Vec<u8>, Vec<Chunk>)> {
    let sections_at = |bytes: &[u8], at: u64| {
        let (find, read) = read_rpf9_body(Cursor::new(bytes), at, u64::MAX).ok()?;
        Rpf9Sections::of(&find, &read)
    };
    let sections = sections_at(loose, 0)
        .ok_or_else(|| bad("the loose file does not read as a resource from its start"))?;
    let written =
        write_rpf9_resource_chunks(&sections.virtual_section, &sections.physical, STREAM_CAP)
            .map_err(|e| bad(format!("the loose resource cannot be stored: {e}")))?;
    let row = gta6_form_h::row(
        type_code,
        written.virtual_len,
        written.physical_len,
        &written.chunks,
    )?;
    let stored = gta6_form_h::stored_body(&[&written.bytes]);
    // Read past its slot, the stored body is the resource its loose form reads as.
    let back = sections_at(&stored, RPF9_SLOT);
    if back.is_none() || back != sections_at(&written.bytes, 0) {
        return Err(bad(
            "the resource would be stored in a form OpenRage's reader does not read back",
        ));
    }
    Ok((row, stored, written.chunks))
}

/// Whether laying `toc` out anew reproduces it byte for byte, as an edit adding a row needs.
pub fn relays_identically(toc: &Rpf9Toc) -> Result<bool> {
    Ok(Table::from_toc(toc)?.faithful)
}

/// The entry's current body, inflated (and deciphered) as the readers return it; `None` when it
/// cannot be read, which counts as different.
fn old_body<R: Read + Seek>(
    mount: &Gta6Mount,
    src: &mut R,
    keys: &GtaViKeys,
    entry: &Gta6Entry,
    chunked: bool,
) -> Option<Vec<u8>> {
    if chunked && entry.cipher.is_some() {
        return mount.read_entry_decrypted(&mut *src, entry, keys).ok();
    }
    // A clear chunked body is walked by its chunk table: read by what it holds, it can end short of
    // the table's last chunk.
    let len = if chunked { entry.stored? } else { entry.real? };
    let mut body = vec![0u8; usize::try_from(len).ok()?];
    src.seek(SeekFrom::Start(mount.absolute_start(entry)?))
        .ok()?;
    src.read_exact(&mut body).ok()?;
    if chunked {
        return mount.inflate_deciphered_stored(&body, entry).ok();
    }
    Some(body)
}

/// `body` stored as chunks under a mode-3 table: the stored bytes, the cell's `u1` and the
/// descriptors; refused where the reader would not read it back.
fn stored_chunked(body: &[u8]) -> Result<(Vec<u8>, u32, Vec<u64>)> {
    stored_chunked_in(body, ROW24_CHUNK_MODE)
}

/// As [`stored_chunked`], under a table of `mode`.
fn stored_chunked_in(body: &[u8], mode: u8) -> Result<(Vec<u8>, u32, Vec<u64>)> {
    let (bytes, chunks) = chunk_body(body)?;
    let (u1, words) = author(&ChunkTable { mode, chunks })?;
    check_reads_back(&bytes, body)?;
    Ok((bytes, u1, words))
}

/// A clear body is read by what it holds, so one the reader does not recognise could not be read.
fn check_reads_back(stored: &[u8], body: &[u8]) -> Result<()> {
    let read = read_rpf9_body(Cursor::new(stored), 0, body.len() as u64);
    if read.ok().map(|(_, bytes)| bytes).as_deref() != Some(body) {
        return Err(bad(
            "the new body would be stored in a form OpenRage's reader does not read back",
        ));
    }
    Ok(())
}

/// `body` as 64 KiB chunks, each zlib unless that would not shrink it, each 16-aligned; and the
/// chunks the table states.
fn chunk_body(body: &[u8]) -> Result<(Vec<u8>, Vec<Chunk>)> {
    let mut out = Vec::new();
    let mut chunks = Vec::new();
    for piece in body.chunks(CHUNK) {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(piece)?;
        let deflated = encoder.finish()?;
        let zlib = deflated.len() < piece.len();
        let stored = if zlib { &deflated[..] } else { piece };
        chunks.push(Chunk {
            inflated: piece.len() as u32,
            stored: stored.len() as u32,
            zlib,
        });
        out.extend_from_slice(stored);
        out.resize(round16(out.len() as u64) as usize, 0);
    }
    Ok((out, chunks))
}

#[cfg(test)]
#[path = "gta6_container_write_tests.rs"]
mod tests;
