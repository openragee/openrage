//! A new RPF9 (GTA VI) archive built from nothing: a header and metadata of the shape every measured
//! archive has, a table of the columns its entries need laid out with its heaps, then the bodies.
//!
//! ⚠ Structural only. The type-6 signature descriptor is left empty (zero), as are the type-5 and
//! type-9 descriptors, header `+0x28` and the per-archive id, whose rules are not known; the game
//! verifies the signature when it mounts an archive and only its maker can sign one, so the game
//! refuses a built archive. The names heap is empty.
//!
//! The entries an archive is built from go in as an added entry does: where its name keeps the
//! entries ascending, with the rows its body is stored under.

use byteorder::{ByteOrder, LE};

use super::super::gta6_archive::{
    GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW, GROUP_ENTRY_ROW_20, GROUP_NAME32,
    GROUP_NAME64,
};
use super::super::gta6_chunk_table::{author, Table as ChunkTable};
use super::super::rpf9::RPF9_MAGIC;
use super::table::{Column, Table};
use super::ROW20_CHUNK_MODE;
use super::{bad, headerless, push, round16, stored_chunked, Rpf9NewBody, Rpf9Piece, Rpf9Plan};
use super::{GtaViKeys, Rpf9Builder, Rpf9Toc};
use crate::error::Result;
use crate::gta6::hash::{name_hash32, name_hash64};

/// The metadata descriptors every measured four-descriptor archive holds: kind, the u16 after it,
/// and the payload's offset into the metadata.
const DESCRIPTORS: [(u16, u16, u32); 4] = [(1, 0, 0x00), (5, 0, 0x18), (6, 0, 0x30), (9, 1, 0x140)];
/// The metadata's length after the descriptors under that layout.
const METADATA_LEN: u32 = 0x150;
/// Header `+0x30`: the metadata's last bytes, enciphered under the index key, not the bootstrap one.
const INDEX_KEYED_TAIL: u32 = 0x10;
/// The class byte of an added chunked body: media, markup or raw bytes.
const CLASS_MEDIA: u8 = 0xFF;
/// The class byte of a nested archive.
const CLASS_ARCHIVE: u8 = 0x00;
/// The index cipher type the reader opens.
const INDEX_CIPHER_TYPE: u16 = 16;
/// Header bytes every measured header holds: `+0x19`, `+0x2D` and `+0x2E`.
const HEADER_CONSTANTS: [(usize, u8); 3] = [(0x19, 0x7A), (0x2D, 0x0F), (0x2E, 0x09)];

/// The index key a table is enciphered under: a key id the snapshot holds, and the IV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rpf9IndexKey {
    pub key_id: u32,
    pub iv: [u8; 16],
}

impl Rpf9IndexKey {
    /// The index key `toc` is enciphered under, from its type-1 metadata descriptor.
    pub fn of(toc: &Rpf9Toc) -> Result<Self> {
        let b = &toc.bytes;
        let n = b
            .get(0x1A..0x1C)
            .map(LE::read_u16)
            .ok_or_else(|| bad("the table is shorter than its header"))?;
        let metadata = 0x40 + round16(8 * u64::from(n)) as usize;
        let at = (0..usize::from(n))
            .filter_map(|i| b.get(0x40 + 8 * i..0x48 + 8 * i))
            .find(|d| LE::read_u16(d) == 1)
            .map(|d| metadata + LE::read_u32(&d[4..]) as usize)
            .ok_or_else(|| bad("the table has no index key descriptor"))?;
        let payload = b
            .get(at..at + 22)
            .ok_or_else(|| bad("the index key descriptor runs past the table"))?;
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&payload[6..]);
        Ok(Self {
            key_id: LE::read_u32(payload),
            iv,
        })
    }
}

/// A new archive of `entries`, each a path and a body, its table enciphered under `key`.
// Every piece is bytes, so the plan writes from any source.
pub fn build(
    entries: &[(String, Rpf9NewBody)],
    key: &Rpf9IndexKey,
    keys: &GtaViKeys,
) -> Result<Rpf9Plan> {
    if entries.is_empty() {
        return Err(bad("an archive holds at least one entry"));
    }
    let row24 = entries
        .iter()
        .any(|(_, b)| !matches!(b, Rpf9NewBody::Resource { .. }));
    let row20 = entries
        .iter()
        .any(|(_, b)| matches!(b, Rpf9NewBody::Resource { .. }));
    let chunked = entries
        .iter()
        .any(|(_, b)| !matches!(b, Rpf9NewBody::Archive(_)));
    let wanted = [
        (GROUP_NAME64, 8, false, true),
        (GROUP_NAME32, 4, false, true),
        (GROUP_ENTRY_ROW, 24, false, row24),
        (GROUP_ENTRY_ROW_20, 20, false, row20),
        (GROUP_CHUNK_RECORD, 8, false, chunked),
        (GROUP_CONTENT_HASH, 8, true, true),
    ];
    let columns: Vec<Column> = wanted
        .iter()
        .filter(|w| w.3)
        .map(|&(hash, width, external, _)| Column {
            hash,
            width,
            external,
            data_start: 0,
            rows: Vec::new(),
        })
        .collect();

    let mut prefix = vec![0u8; 0x40 + 8 * DESCRIPTORS.len() + METADATA_LEN as usize];
    LE::write_u32(&mut prefix, RPF9_MAGIC);
    prefix[0x1A] = DESCRIPTORS.len() as u8;
    LE::write_u32(&mut prefix[0x1C..], METADATA_LEN);
    LE::write_u16(&mut prefix[0x20..], columns.len() as u16);
    LE::write_u16(
        &mut prefix[0x22..],
        columns.iter().filter(|c| c.external).count() as u16,
    );
    LE::write_u32(&mut prefix[0x30..], INDEX_KEYED_TAIL);
    for (at, byte) in HEADER_CONSTANTS {
        prefix[at] = byte;
    }
    for (i, (kind, after, offset)) in DESCRIPTORS.iter().enumerate() {
        LE::write_u16(&mut prefix[0x40 + 8 * i..], *kind);
        LE::write_u16(&mut prefix[0x42 + 8 * i..], *after);
        LE::write_u32(&mut prefix[0x44 + 8 * i..], *offset);
    }
    let index_key = 0x40 + 8 * DESCRIPTORS.len();
    LE::write_u32(&mut prefix[index_key..], key.key_id);
    LE::write_u16(&mut prefix[index_key + 4..], INDEX_CIPHER_TYPE);
    prefix[index_key + 6..index_key + 22].copy_from_slice(&key.iv);

    let mut table = Table {
        groups_start: prefix.len(),
        bytes: prefix,
        columns,
        index: Vec::new(),
        width: 1,
        relaid: true,
        faithful: true,
    };
    for (name, body) in entries {
        let e = insert_entry(&mut table, name)?;
        set_new_body(&mut table, e, body).map_err(|err| match err {
            crate::error::IoError::Invalid { why, .. } => bad(format!("{name}: {why}")),
            other => other,
        })?;
    }

    // The table's length does not depend on the starts it states.
    let mut cursor = round16(table.emit()?.len() as u64);
    let mut bodies = Vec::new();
    for (c, r) in table.bodies() {
        let Some(fresh) = table.columns[c].rows[r].body.take() else {
            continue;
        };
        table.set_start(c, r, cursor)?;
        bodies.push((cursor, fresh.pieces));
        cursor = round16(cursor + fresh.len);
    }
    let plain = table.emit()?;
    let toc = Rpf9Toc {
        entry_count: table.index.len() as u32,
        bytes: plain,
        key_id: key.key_id,
        registry_id: (u64::from(key.key_id) << 32) | 0x0200_0000,
        groups: Vec::new(),
    };
    let mut pieces = vec![Rpf9Piece::Bytes(Rpf9Builder::serialize(&toc, keys)?)];
    let mut at = toc.bytes.len() as u64;
    let bodies_changed = bodies.len();
    for (start, body) in bodies {
        push(&mut pieces, Rpf9Piece::Zero(start - at));
        at = start;
        for piece in body {
            at += piece.size();
            push(&mut pieces, piece);
        }
    }
    push(&mut pieces, Rpf9Piece::Zero(round16(at) - at));
    Ok(Rpf9Plan {
        pieces,
        tables_changed: 1,
        bodies_changed,
    })
}

/// The column of `hash` and `width`, refused by `what` it holds where the table has none.
pub(super) fn column(table: &Table, hash: u64, width: usize, what: &str) -> Result<usize> {
    table
        .col(hash, width)
        .ok_or_else(|| bad(format!("the archive has no {what} column")))
}

/// Insert an entry named `name` where its name32 (else its name64) keeps the entries ascending,
/// with its name rows and a zero content hash; its index.
pub(super) fn insert_entry(table: &mut Table, name: &str) -> Result<usize> {
    if name.is_empty() {
        return Err(bad("an entry cannot be added with an empty name"));
    }
    let c64 = column(table, GROUP_NAME64, 8, "name64")?;
    let h64 = name_hash64(name.as_bytes()).to_le_bytes();
    if (0..table.index.len()).any(|o| table.cell(o, c64) == Some(&h64[..])) {
        return Err(bad(format!("another entry is already named {name:?}")));
    }
    let c32 = table.col(GROUP_NAME32, 4);
    let h32 = name_hash32(name.as_bytes()).to_le_bytes();
    let key = |table: &Table, o: usize| match c32 {
        Some(c) => table.cell(o, c).map(|k| u64::from(LE::read_u32(k))),
        None => table.cell(o, c64).map(LE::read_u64),
    };
    let new = match c32 {
        Some(_) => u64::from(LE::read_u32(&h32)),
        None => LE::read_u64(&h64),
    };
    let e = (0..table.index.len())
        .position(|o| key(table, o).is_some_and(|k| k > new))
        .unwrap_or(table.index.len());
    table.index.insert(e, vec![u16::MAX; table.columns.len()]);
    table.relaid = true;
    table.set_cell(e, c64, Some(h64.to_vec()), Vec::new());
    if let Some(c) = c32 {
        table.set_cell(e, c, Some(h32.to_vec()), Vec::new());
    }
    if let Some(c) = table.col(GROUP_CONTENT_HASH, 8) {
        table.set_cell(e, c, Some(vec![0; 8]), Vec::new());
    }
    Ok(e)
}

/// Give new entry `e` its body and the rows it is stored under.
pub(super) fn set_new_body(table: &mut Table, e: usize, body: &Rpf9NewBody) -> Result<()> {
    let row24 = |class: u8, stored: &[u8], real: usize| {
        let mut cell = vec![0u8; 24];
        LE::write_u64(&mut cell, u64::from(class) << 40);
        LE::write_u64(&mut cell[8..], stored.len() as u64 / 16);
        LE::write_u64(&mut cell[16..], real as u64);
        cell
    };
    let (c, cell, stored) = match body {
        Rpf9NewBody::Chunked(bytes) => {
            let c = column(table, GROUP_ENTRY_ROW, 24, "24-byte entry-row")?;
            let k = column(table, GROUP_CHUNK_RECORD, 8, "chunk-record")?;
            let (stored, u1, words) = stored_chunked(bytes)?;
            table.set_chunks(e, k, u1, &words);
            (c, row24(CLASS_MEDIA, &stored, bytes.len()), stored)
        }
        Rpf9NewBody::Archive(bytes) => {
            if bytes.get(..4).map(LE::read_u32) != Some(RPF9_MAGIC) {
                return Err(bad("an added archive does not begin with an RPF9 header"));
            }
            let c = column(table, GROUP_ENTRY_ROW, 24, "24-byte entry-row")?;
            let mut stored = bytes.clone();
            stored.resize(round16(stored.len() as u64) as usize, 0);
            (c, row24(CLASS_ARCHIVE, &stored, bytes.len()), stored)
        }
        Rpf9NewBody::Resource { type_code, loose } => {
            let c = column(table, GROUP_ENTRY_ROW_20, 20, "20-byte entry-row")?;
            let k = column(table, GROUP_CHUNK_RECORD, 8, "chunk-record")?;
            let (row, stored, chunks) = headerless(*type_code, loose)?;
            let (u1, words) = author(&ChunkTable {
                mode: ROW20_CHUNK_MODE,
                chunks,
            })?;
            table.set_chunks(e, k, u1, &words);
            (c, row.to_vec(), stored)
        }
    };
    let len = stored.len() as u64;
    table.set_body(e, c, cell, vec![Rpf9Piece::Bytes(stored)], len, (0, 0));
    Ok(())
}
