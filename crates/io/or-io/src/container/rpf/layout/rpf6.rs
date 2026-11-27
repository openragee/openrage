//! RPF6: a big-endian header and twenty-byte rows, names as hashes, and after the bodies a debug
//! section that spells them (`debug offset * 8`, one `{name offset, modified}` pair per row, then the text).

use std::io::{Seek, SeekFrom, Write};

use byteorder::{ByteOrder, BE};

use super::{bad, hash_name, name_hash, pad_to, Input, Plan, Toc};
use crate::container::rpf::crypto::{CryptoTarget, RpfCryptoProvider};
use crate::container::rpf::entry::{
    Compressor, RpfBinaryEntry, RpfDirectoryEntry, RpfEntry, RpfResourceEntry,
};
use crate::container::rpf::header::RpfHeader;
use crate::container::rpf::version::RpfVersion;
use crate::error::Result;

const HEADER: u64 = 16;
const ROW: usize = 20;
const DEBUG_ROW: usize = 8;
const DIRECTORY: u32 = 1 << 31;
const COMPRESSED: u32 = 1 << 30;
const SIZE_MASK: u32 = COMPRESSED - 1;
const OFFSET_MASK: u32 = DIRECTORY - 1;
const RESOURCE_OFFSET_MASK: u32 = OFFSET_MASK & !0xFF;
const XENON: u32 = 1 << 8;

/// The rows' bytes rounded up to the cipher's block.
fn sealed_len(count: u64) -> u64 {
    (count * ROW as u64 + 15) & !15
}

pub(super) fn prologue(entries: usize) -> u64 {
    HEADER + sealed_len(entries as u64)
}

pub(super) fn read(input: &Input) -> Result<Toc> {
    let head = input.bytes(4, HEADER - 4)?;
    let mut h = [0u32; 3];
    BE::read_u32_into(&head, &mut h);
    let (count, debug_at, tag) = (u64::from(h[0]), u64::from(h[1]) * 8, h[2]);

    let mut rows = input.bytes(HEADER, sealed_len(count))?;
    let mut debug = if debug_at == 0 {
        Vec::new()
    } else {
        input.bytes(debug_at, input.size.saturating_sub(debug_at))?
    };
    let sealed = debug.len() & !15;
    let (encryption, outcome) =
        input.unseal(RpfVersion::Rpf6, tag, &mut rows, &mut debug[..sealed])?;

    let entries: Vec<RpfEntry> = rows
        .as_chunks::<ROW>()
        .0
        .iter()
        .take(count as usize)
        .map(|row| {
            let mut w = [0u32; 5];
            BE::read_u32_into(row, &mut w);
            decode(w)
        })
        .collect();
    let names = if debug.is_empty() {
        entries.iter().map(|e| hash_name(e.name_offset())).collect()
    } else {
        debug_names(&debug, entries.len())?
    };

    Ok(Toc {
        header: RpfHeader {
            entry_count: count as u32,
            name_heap_size: debug.len() as u32,
            name_shift: 0,
            xcompress: false,
            encryption,
            platform: None,
        },
        entries,
        names,
        name_heap: Vec::new(),
        outcome,
    })
}

/// The text of every row's name, which a debug section keeps after its table.
fn debug_names(debug: &[u8], count: usize) -> Result<Vec<String>> {
    let (pairs, text) = debug
        .split_at_checked(count * DEBUG_ROW)
        .ok_or_else(|| bad("the debug section is shorter than its table"))?;
    pairs
        .as_chunks::<DEBUG_ROW>()
        .0
        .iter()
        .map(|pair| {
            let at = BE::read_u32(pair) as usize;
            let tail = text
                .get(at..)
                .ok_or_else(|| bad("a debug name is out of range"))?;
            let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
            Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
        })
        .collect()
}

fn decode(w: [u32; 5]) -> RpfEntry {
    if w[2] & DIRECTORY != 0 {
        RpfEntry::Directory(RpfDirectoryEntry {
            name_offset: w[0],
            first_entry_index: w[2] & OFFSET_MASK,
            entry_count: w[3] & SIZE_MASK,
        })
    } else if w[3] & DIRECTORY != 0 {
        RpfEntry::Resource(RpfResourceEntry {
            name_offset: w[0],
            consumed_size: w[1] & OFFSET_MASK,
            file_offset: u64::from(w[2] & RESOURCE_OFFSET_MASK) << 3,
            system_info: w[3],
            graphics_info: w[4],
            compressor: Compressor::None,
            tag: (w[2] & 0xFF) | if w[1] & DIRECTORY != 0 { XENON } else { 0 },
        })
    } else {
        let compressed = w[3] & COMPRESSED != 0;
        let disk = w[1] & OFFSET_MASK;
        RpfEntry::Binary(RpfBinaryEntry {
            name_offset: w[0],
            consumed_size: if compressed { disk } else { 0 },
            file_offset: u64::from(w[2] & OFFSET_MASK) << 3,
            uncompressed_size: if compressed { w[3] & SIZE_MASK } else { disk },
            encrypted: false,
            encrypted_raw: 0,
            compressor: if compressed {
                Compressor::Deflate
            } else {
                Compressor::None
            },
            tag: 0,
        })
    }
}

fn encode(entry: &RpfEntry, hash: u32) -> Result<[u32; 5]> {
    let at = |offset: u64, mask: u32| {
        u32::try_from(offset >> 3)
            .ok()
            .filter(|n| n & !mask == 0)
            .ok_or_else(|| bad(format!("offset {offset:#x} does not fit RPF6")))
    };
    Ok(match entry {
        RpfEntry::Directory(d) => [hash, 0, DIRECTORY | d.first_entry_index, d.entry_count, 0],
        RpfEntry::Binary(b) => {
            let compressed = b.consumed_size != 0;
            [
                hash,
                if compressed {
                    b.consumed_size
                } else {
                    b.uncompressed_size
                },
                at(b.file_offset, OFFSET_MASK)?,
                if compressed {
                    COMPRESSED | b.uncompressed_size
                } else {
                    0
                },
                0,
            ]
        }
        RpfEntry::Resource(r) => [
            hash,
            r.consumed_size | if r.tag & XENON != 0 { DIRECTORY } else { 0 },
            at(r.file_offset, RESOURCE_OFFSET_MASK)? | (r.tag & 0xFF),
            r.system_info | DIRECTORY,
            r.graphics_info,
        ],
    })
}

pub(super) fn write<W: Write + Seek>(
    w: &mut W,
    plan: &Plan,
    crypto: &dyn RpfCryptoProvider,
) -> Result<()> {
    let mut rows = vec![0u8; plan.entries.len() * ROW];
    for ((entry, name), row) in plan
        .entries
        .iter()
        .zip(plan.names)
        .zip(rows.as_chunks_mut::<ROW>().0.iter_mut())
    {
        BE::write_u32_into(&encode(entry, name_hash(name))?, row);
    }

    let mut debug = Vec::with_capacity(plan.names.len() * DEBUG_ROW);
    let mut text = Vec::new();
    for name in plan.names {
        debug.extend_from_slice(&(text.len() as u32).to_be_bytes());
        debug.extend_from_slice(&0u32.to_be_bytes());
        text.extend_from_slice(name.as_bytes());
        text.push(0);
    }
    debug.extend_from_slice(&text);

    let tag = RpfVersion::Rpf6.tag_of(plan.encryption)?;
    rows.resize(sealed_len(plan.entries.len() as u64) as usize, 0);
    if tag != 0 {
        let target = CryptoTarget::Toc {
            archive_name: None,
            archive_size: u32::try_from(plan.body_end).unwrap_or(u32::MAX),
        };
        crypto.encrypt(plan.encryption, target.clone(), &mut rows)?;
        let sealed = debug.len() & !15;
        crypto.encrypt(plan.encryption, target, &mut debug[..sealed])?;
    }

    let debug_at = plan.body_end.next_multiple_of(8);
    w.seek(SeekFrom::Start(plan.body_end))?;
    pad_to(w, debug_at)?;
    w.write_all(&debug)?;

    let mut head = [0u8; HEADER as usize];
    head[..4].copy_from_slice(&RpfVersion::Rpf6.magic().unwrap_or(0).to_le_bytes());
    BE::write_u32_into(
        &[plan.entries.len() as u32, (debug_at / 8) as u32, tag],
        &mut head[4..],
    );
    w.seek(SeekFrom::Start(0))?;
    w.write_all(&head)?;
    w.write_all(&rows)?;
    Ok(())
}
