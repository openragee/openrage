//! RPF0, RPF2, RPF3 and RPF4: sixteen-byte little-endian rows at `0x800`, then the name heap.
//!
//! RPF2 and RPF4 name an entry by an offset into the heap and RPF3 by a hash; RPF4 counts its
//! offsets in eight-byte units. RPF0 has no resources, and its directory repeats its count.

use std::io::{Seek, SeekFrom, Write};

use byteorder::{ByteOrder, LE};

use super::{bad, hash_name, name_hash, pad_to, Input, Plan, Toc};
use crate::container::rpf::crypto::{CryptoTarget, RpfCryptoProvider};
use crate::container::rpf::entry::{
    Compressor, RpfBinaryEntry, RpfDirectoryEntry, RpfEntry, RpfResourceEntry,
};
use crate::container::rpf::header::RpfHeader;
use crate::container::rpf::reader::build_names;
use crate::container::rpf::version::RpfVersion;
use crate::error::{IoError, Result};

const TABLE_AT: u64 = 0x800;
const ROW: usize = 16;
const DIRECTORY: u32 = 1 << 31;
const COMPRESSED: u32 = 1 << 30;
const SIZE_MASK: u32 = COMPRESSED - 1;
const OFFSET_MASK: u32 = DIRECTORY - 1;
const RESOURCE_OFFSET_MASK: u32 = OFFSET_MASK & !0xFF;

const fn header_len(version: RpfVersion) -> usize {
    if matches!(version, RpfVersion::Rpf0) {
        12
    } else {
        24
    }
}

/// An offset's unit, as a shift.
const fn shift(version: RpfVersion) -> u32 {
    if matches!(version, RpfVersion::Rpf4) {
        3
    } else {
        0
    }
}

pub(super) fn prologue(entries: usize, heap: usize) -> u64 {
    TABLE_AT + (entries * ROW + heap) as u64
}

pub(super) fn read(version: RpfVersion, input: &Input) -> Result<Toc> {
    let head = input.bytes(0, header_len(version) as u64)?;
    let mut h = [0u32; 6];
    LE::read_u32_into(&head, &mut h[..head.len() / 4]);
    let (table_len, count) = (u64::from(h[1]), u64::from(h[2]));
    let tag = if version == RpfVersion::Rpf0 { 0 } else { h[4] };
    if count * ROW as u64 > table_len {
        return Err(IoError::RpfBadHeader {
            message: format!("{count} rows do not fit the {table_len} byte table"),
        });
    }

    let mut rows = input.bytes(TABLE_AT, table_len)?;
    let mut heap = rows.split_off(count as usize * ROW);
    let (encryption, outcome) = input.unseal(version, tag, &mut rows, &mut heap)?;

    let entries: Vec<RpfEntry> = rows
        .as_chunks::<ROW>()
        .0
        .iter()
        .map(|row| {
            let mut w = [0u32; 4];
            LE::read_u32_into(row, &mut w);
            decode(version, w)
        })
        .collect();
    let names = if version == RpfVersion::Rpf3 {
        entries.iter().map(|e| hash_name(e.name_offset())).collect()
    } else {
        build_names(&entries, &heap, 0)?
    };

    Ok(Toc {
        header: RpfHeader {
            entry_count: count as u32,
            name_heap_size: heap.len() as u32,
            name_shift: 0,
            xcompress: false,
            encryption,
            platform: None,
        },
        entries,
        names,
        name_heap: heap,
        outcome,
    })
}

fn decode(version: RpfVersion, w: [u32; 4]) -> RpfEntry {
    let sized = |compressed: bool, disk: u32| {
        if compressed {
            (disk, Compressor::Deflate)
        } else {
            (0, Compressor::None)
        }
    };
    if version == RpfVersion::Rpf0 {
        let name_offset = w[0] & OFFSET_MASK;
        if w[0] & DIRECTORY != 0 {
            return RpfEntry::Directory(RpfDirectoryEntry {
                name_offset,
                first_entry_index: w[1],
                entry_count: w[2],
            });
        }
        let (consumed_size, compressor) = sized(w[2] != w[3], w[2]);
        return RpfEntry::Binary(RpfBinaryEntry {
            name_offset,
            consumed_size,
            file_offset: u64::from(w[1]),
            uncompressed_size: w[3],
            encrypted: false,
            encrypted_raw: 0,
            compressor,
            tag: 0,
        });
    }

    let unit = shift(version);
    if w[2] & DIRECTORY != 0 {
        RpfEntry::Directory(RpfDirectoryEntry {
            name_offset: w[0],
            first_entry_index: w[2] & OFFSET_MASK,
            entry_count: w[3] & SIZE_MASK,
        })
    } else if w[3] & DIRECTORY != 0 {
        RpfEntry::Resource(RpfResourceEntry {
            name_offset: w[0],
            consumed_size: w[1],
            file_offset: u64::from(w[2] & RESOURCE_OFFSET_MASK) << unit,
            system_info: w[3],
            graphics_info: 0,
            compressor: Compressor::None,
            tag: w[2] & 0xFF,
        })
    } else {
        let (consumed_size, compressor) = sized(w[3] & COMPRESSED != 0, w[3] & SIZE_MASK);
        RpfEntry::Binary(RpfBinaryEntry {
            name_offset: w[0],
            consumed_size,
            file_offset: u64::from(w[2] & OFFSET_MASK) << unit,
            uncompressed_size: w[1],
            encrypted: false,
            encrypted_raw: 0,
            compressor,
            tag: 0,
        })
    }
}

fn encode(version: RpfVersion, entry: &RpfEntry, name: u32) -> Result<[u32; 4]> {
    let unit = shift(version);
    let at = |offset: u64, mask: u32| {
        u32::try_from(offset >> unit)
            .ok()
            .filter(|n| n & !mask == 0)
            .ok_or_else(|| bad(format!("offset {offset:#x} does not fit {version}")))
    };
    Ok(match (version, entry) {
        (RpfVersion::Rpf0, RpfEntry::Directory(d)) => [
            DIRECTORY | name,
            d.first_entry_index,
            d.entry_count,
            d.entry_count,
        ],
        (RpfVersion::Rpf0, RpfEntry::Binary(b)) => [
            name,
            at(b.file_offset, u32::MAX)?,
            if b.consumed_size != 0 {
                b.consumed_size
            } else {
                b.uncompressed_size
            },
            b.uncompressed_size,
        ],
        (RpfVersion::Rpf0, RpfEntry::Resource(_)) => return Err(bad("RPF0 holds no resources")),
        (_, RpfEntry::Directory(d)) => [name, 0, DIRECTORY | d.first_entry_index, d.entry_count],
        (_, RpfEntry::Binary(b)) => [
            name,
            b.uncompressed_size,
            at(b.file_offset, OFFSET_MASK)?,
            if b.consumed_size != 0 {
                COMPRESSED | b.consumed_size
            } else {
                b.uncompressed_size
            },
        ],
        (_, RpfEntry::Resource(r)) => [
            name,
            r.consumed_size,
            at(r.file_offset, RESOURCE_OFFSET_MASK)? | (r.tag & 0xFF),
            r.system_info | DIRECTORY,
        ],
    })
}

pub(super) fn write<W: Write + Seek>(
    version: RpfVersion,
    w: &mut W,
    plan: &Plan,
    crypto: &dyn RpfCryptoProvider,
) -> Result<()> {
    let hashed = version == RpfVersion::Rpf3;
    let heap = if hashed { &[][..] } else { plan.name_heap };
    let mut rows = vec![0u8; plan.entries.len() * ROW];
    for ((entry, name), row) in plan
        .entries
        .iter()
        .zip(plan.names)
        .zip(rows.as_chunks_mut::<ROW>().0.iter_mut())
    {
        let name = if hashed {
            name_hash(name)
        } else {
            entry.name_offset()
        };
        LE::write_u32_into(&encode(version, entry, name)?, row);
    }

    let tag = version.tag_of(plan.encryption)?;
    let mut heap = heap.to_vec();
    if tag != 0 {
        let target = CryptoTarget::Toc {
            archive_name: None,
            archive_size: u32::try_from(plan.body_end).unwrap_or(u32::MAX),
        };
        crypto.encrypt(plan.encryption, target.clone(), &mut rows)?;
        crypto.encrypt(plan.encryption, target, &mut heap)?;
    }

    let mut head = [0u32; 6];
    head[0] = version.magic().unwrap_or(0);
    head[1] = (rows.len() + heap.len()) as u32;
    head[2] = plan.entries.len() as u32;
    head[4] = tag;
    let mut head_bytes = [0u8; 24];
    LE::write_u32_into(&head, &mut head_bytes);

    w.seek(SeekFrom::Start(0))?;
    w.write_all(&head_bytes[..header_len(version)])?;
    pad_to(w, TABLE_AT)?;
    w.write_all(&rows)?;
    w.write_all(&heap)?;
    Ok(())
}
