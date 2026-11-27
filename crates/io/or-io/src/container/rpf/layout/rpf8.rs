//! RPF8: sixteen header bytes, a 256-byte RSA block, twenty-four-byte rows sorted by name hash, the
//! bodies in sixteen-byte units, and a names blob that ends the file.
//!
//! A row is three words: the hash, an encryption config, key id, extension id and the resource
//! and signature bits; the stored size, offset and compressor; and a file's size or a resource's
//! page flags. A table tagged `0xFF` is in the clear, and so is a body whose key id is.
//! A file with an extension the table knows hashes its path without it.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};

use byteorder::{ByteOrder, LE};

use super::{bad, hash_name, name_hash, root_over, Input, Plan, Toc};
use crate::container::rpf::crypto::{CryptoTarget, RpfCryptoProvider};
use crate::container::rpf::entry::{Compressor, RpfBinaryEntry, RpfEntry, RpfResourceEntry};
use crate::container::rpf::header::RpfHeader;
use crate::container::rpf::version::{RpfVersion, RSC8_MAGIC};
use crate::error::{IoError, Result};
use crate::hash::string_hash;

const HEADER: usize = 16;
const SIGNATURE: usize = 256;
const ROW: usize = 24;
const UNIT: u64 = 16;
/// The key id of a body, and a table, in the clear.
pub(in crate::container::rpf) const CLEAR: u32 = 0xFF;
const DIRECTORY_EXT: u8 = 0xFE;
const PC: u16 = b'y' as u16;

const BASE_EXTS: [&str; 14] = [
    "rpf", "#mf", "#dr", "#ft", "#dd", "#td", "#bn", "#bd", "#pd", "#bs", "#sd", "#mt", "#sc",
    "#cs",
];
const EXTRA_EXTS: [&str; 24] = [
    "mrf", "cut", "gfx", "#cd", "#ld", "#pmd", "#pm", "#ed", "#pt", "#map", "#typ", "#ch", "#ldb",
    "#jd", "#ad", "#nv", "#hn", "#pl", "#nd", "#vr", "#wr", "#nh", "#fd", "#as",
];

pub(super) fn prologue(entries: usize) -> u64 {
    (HEADER + SIGNATURE + entries.saturating_sub(1) * ROW) as u64
}

/// An extension's id, with `#` standing for the platform letter; `0xFF` for one the table lacks.
fn ext_id(ext: &str, platform: u8) -> u8 {
    let is = |pattern: &str| {
        pattern.len() == ext.len()
            && pattern
                .bytes()
                .zip(ext.bytes())
                .all(|(p, e)| if p == b'#' { e == platform } else { p == e })
    };
    BASE_EXTS
        .iter()
        .position(|p| is(p))
        .map(|i| i as u8)
        .or_else(|| EXTRA_EXTS.iter().position(|p| is(p)).map(|i| 64 + i as u8))
        .unwrap_or(0xFF)
}

fn ext_of(id: u8, platform: u8) -> Option<String> {
    let pattern = match id {
        0..=13 => BASE_EXTS[usize::from(id)],
        64..=87 => EXTRA_EXTS[usize::from(id - 64)],
        _ => return None,
    };
    Some(pattern.replace('#', &char::from(platform).to_string()))
}

/// Split `path` into the text a row hashes and the extension id it carries.
fn split(path: &str, platform: u8) -> (&str, u8) {
    match path.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains('/') && ext_id(ext, platform) != 0xFF => {
            (stem, ext_id(ext, platform))
        }
        _ => (path, 0xFF),
    }
}

pub(super) fn read(input: &Input) -> Result<Toc> {
    let head = input.bytes(4, (HEADER - 4) as u64)?;
    let (count, names_len) = (
        u64::from(LE::read_u32(&head)),
        u64::from(LE::read_u32(&head[4..])),
    );
    let (tag, platform) = (
        u32::from(LE::read_u16(&head[8..])),
        LE::read_u16(&head[10..]),
    );

    let mut rows = input.bytes((HEADER + SIGNATURE) as u64, count * ROW as u64)?;
    let after_rows = input.size - (HEADER + SIGNATURE) as u64 - rows.len() as u64;
    if names_len > after_rows {
        return Err(IoError::RpfBadHeader {
            message: format!("{names_len} bytes of names in the {after_rows} that follow the rows"),
        });
    }
    let mut names = input.bytes(input.size - names_len, names_len)?;
    let (encryption, outcome) = input.unseal(RpfVersion::Rpf8, tag, &mut rows, &mut names)?;

    let platform = platform as u8;
    let mut entries = Vec::with_capacity(rows.len() / ROW + 1);
    let mut hashes = Vec::with_capacity(rows.len() / ROW);
    entries.push(root_over(rows.len() / ROW));
    for row in rows.as_chunks::<ROW>().0.iter() {
        let mut q = [0u64; 3];
        LE::read_u64_into(row, &mut q);
        hashes.push((q[0] as u32, (q[0] >> 48) as u8));
        entries.push(decode(q)?);
    }

    let known = known_names(&names);
    let mut spelled = vec![String::new()];
    spelled.extend(hashes.iter().map(|&(hash, ext)| {
        known.get(&hash).cloned().unwrap_or_else(|| {
            let mut name = hash_name(hash);
            if let Some(ext) = ext_of(ext, platform) {
                name.push('.');
                name.push_str(&ext);
            }
            name
        })
    }));

    Ok(Toc {
        header: RpfHeader {
            entry_count: entries.len() as u32,
            name_heap_size: names.len() as u32,
            name_shift: 0,
            xcompress: false,
            encryption,
            // The platform letter the header records at `0x0A`, which also decodes the rows'
            // extension ids above.
            platform: Some(platform),
        },
        entries,
        names: spelled,
        name_heap: names,
        outcome,
    })
}

/// The path each hash in the names blob stands for, keyed as a row would hash it: whole, and without its extension.
fn known_names(blob: &[u8]) -> HashMap<u32, String> {
    let mut known = HashMap::new();
    for path in blob.split(|&b| b == 0).filter(|s| !s.is_empty()) {
        let path = String::from_utf8_lossy(path).into_owned();
        known.insert(string_hash(path.as_bytes()), path.clone());
        if let Some((stem, _)) = path.rsplit_once('.') {
            known.entry(string_hash(stem.as_bytes())).or_insert(path);
        }
    }
    known
}

fn decode(q: [u64; 3]) -> Result<RpfEntry> {
    let (key, config) = ((q[0] >> 40) as u8, (q[0] >> 32) as u8);
    let signed = (q[0] >> 57) & 1;
    let tag = u32::from(key) | u32::from(config) << 8 | (signed as u32) << 16;
    let disk = ((q[1] & 0x0FFF_FFFF) << 4) as u32;
    let file_offset = ((q[1] >> 28) & 0x7FFF_FFFF) * UNIT;
    let compressor = match (q[1] >> 59) as u8 {
        0 => Compressor::None,
        1 => Compressor::Deflate,
        2 => Compressor::Oodle,
        other => return Err(bad(format!("compressor {other} is not one RPF8 has"))),
    };
    let name_offset = q[0] as u32;
    if (q[0] >> 48) as u8 == DIRECTORY_EXT {
        return Err(bad("an RPF8 directory row, which no archive has"));
    }
    if (q[0] >> 56) & 1 != 0 {
        return Ok(RpfEntry::Resource(RpfResourceEntry {
            name_offset,
            consumed_size: disk,
            file_offset,
            system_info: q[2] as u32,
            graphics_info: (q[2] >> 32) as u32,
            compressor,
            tag,
        }));
    }
    let size = u32::try_from(q[2]).map_err(|_| bad("a file larger than 4 GiB"))?;
    Ok(RpfEntry::Binary(RpfBinaryEntry {
        name_offset,
        consumed_size: if compressor == Compressor::None {
            0
        } else {
            disk
        },
        file_offset,
        uncompressed_size: size,
        encrypted: key != CLEAR as u8,
        encrypted_raw: u32::from(key != CLEAR as u8),
        compressor,
        tag,
    }))
}

/// A path's row hash and extension id: the hash of its text less an extension the table knows, or the hash a `hash_XXXXXXXX` name spells.
fn identity(path: &str) -> (u32, u8) {
    if path.starts_with("hash_") {
        let ext = path
            .rsplit_once('.')
            .map_or(0xFF, |(_, e)| ext_id(e, PC as u8));
        (name_hash(path), ext)
    } else {
        let (text, ext) = split(path, PC as u8);
        (string_hash(text.as_bytes()), ext)
    }
}

/// The hash a path's row carries, which is what a table is sorted by.
pub(in crate::container::rpf) fn hash_of(path: &str) -> u32 {
    identity(path).0
}

fn encode(entry: &RpfEntry, path: &str) -> Result<[u64; 3]> {
    let (hash, ext) = identity(path);
    let row = |tag: u32, resource: bool, disk: u32, offset: u64, compressor: u64, last: u64| {
        let first = u64::from(hash)
            | u64::from(tag >> 8 & 0xFF) << 32
            | u64::from(tag & 0xFF) << 40
            | u64::from(ext) << 48
            | u64::from(resource) << 56
            | u64::from(tag >> 16 & 1) << 57;
        let second = u64::from(disk >> 4) | (offset / UNIT) << 28 | compressor << 59;
        [first, second, last]
    };
    let at = |offset: u64| {
        if offset.is_multiple_of(UNIT) && offset / UNIT < 1 << 31 {
            Ok(offset)
        } else {
            Err(bad(format!("offset {offset:#x} does not fit RPF8")))
        }
    };
    Ok(match entry {
        RpfEntry::Directory(_) => return Err(bad("RPF8 has no directories")),
        RpfEntry::Binary(b) => row(
            b.tag,
            false,
            stored_len(b.read_size())?,
            at(b.file_offset)?,
            compressor_id(b.compressor),
            u64::from(b.uncompressed_size),
        ),
        RpfEntry::Resource(r) => row(
            r.tag,
            true,
            stored_len(r.consumed_size)?,
            at(r.file_offset)?,
            compressor_id(r.compressor),
            u64::from(r.system_info) | u64::from(r.graphics_info) << 32,
        ),
    })
}

const fn compressor_id(compressor: Compressor) -> u64 {
    match compressor {
        Compressor::None => 0,
        Compressor::Deflate => 1,
        Compressor::Oodle => 2,
    }
}

/// A stored size as the row holds it: in sixteen-byte units, so rounded up to one.
fn stored_len(len: u32) -> Result<u32> {
    len.checked_next_multiple_of(UNIT as u32)
        .filter(|&n| n >> 4 < 1 << 28)
        .ok_or_else(|| bad(format!("{len} bytes do not fit an RPF8 row")))
}

pub(super) fn write<W: Write + Seek>(
    w: &mut W,
    plan: &Plan,
    crypto: &dyn RpfCryptoProvider,
) -> Result<()> {
    let files = plan.entries.len() - 1;
    let mut rows = vec![0u8; files * ROW];
    let mut names = Vec::new();
    for ((entry, path), row) in plan.entries[1..]
        .iter()
        .zip(&plan.names[1..])
        .zip(rows.as_chunks_mut::<ROW>().0.iter_mut())
    {
        LE::write_u64_into(&encode(entry, path)?, row);
        if !path.starts_with("hash_") {
            names.extend_from_slice(path.as_bytes());
            names.push(0);
        }
    }
    // Retail archives end their list of names with four zero bytes, which the length counts.
    if !names.is_empty() {
        names.extend_from_slice(&[0; 4]);
    }
    let names_len = names.len() as u32;

    let tag = RpfVersion::Rpf8.tag_of(plan.encryption)?;
    if tag != CLEAR {
        let target = CryptoTarget::Toc {
            archive_name: None,
            archive_size: u32::try_from(plan.body_end).unwrap_or(u32::MAX),
        };
        crypto.encrypt(plan.encryption, target.clone(), &mut rows)?;
        crypto.encrypt(plan.encryption, target, &mut names)?;
    }

    w.seek(SeekFrom::Start(plan.body_end))?;
    w.write_all(&names)?;

    let mut head = [0u8; HEADER];
    head[..4].copy_from_slice(&RpfVersion::Rpf8.magic().unwrap_or(0).to_le_bytes());
    LE::write_u32(&mut head[4..], files as u32);
    LE::write_u32(&mut head[8..], names_len);
    LE::write_u16(&mut head[12..], tag as u16);
    LE::write_u16(&mut head[14..], PC);
    w.seek(SeekFrom::Start(0))?;
    w.write_all(&head)?;
    w.write_all(&[0u8; SIGNATURE])?;
    w.write_all(&rows)?;
    Ok(())
}

/// Whether a row's key id names a cipher.
pub(in crate::container::rpf) fn is_encrypted(tag: u32) -> bool {
    tag & 0xFF != CLEAR
}

/// The sixteen bytes that lead an RPF8 resource file: its magic, what its body is, and its page flags.
///
/// `clear` says the body was deciphered. The flags word is the one community tools write: the
/// resource id at bit 0, the compressor less one at 8, the signature flag at 13, the key id plus
/// one at 16 and the config at 24.
pub(in crate::container::rpf) fn resource_header(
    entry: &RpfResourceEntry,
    clear: bool,
) -> [u8; 16] {
    let (key, config) = if clear {
        (CLEAR, 0)
    } else {
        (entry.tag & 0xFF, entry.tag >> 8 & 0xFF)
    };
    let compressor = match entry.compressor {
        Compressor::None => 0x1F,
        Compressor::Deflate => 0,
        Compressor::Oodle => 1,
    };
    let flags = (entry.system_info & 0xF) << 4
        | (entry.graphics_info & 0xF)
        | compressor << 8
        | (entry.tag >> 16 & 1) << 13
        | ((key + 1) & 0xFF) << 16
        | config << 24;
    let mut head = [0u8; 16];
    LE::write_u32(&mut head, RSC8_MAGIC);
    LE::write_u32(&mut head[4..], flags);
    LE::write_u32(&mut head[8..], entry.system_info);
    LE::write_u32(&mut head[12..], entry.graphics_info);
    head
}

/// A resource's row, from the sixteen bytes that lead its file; none if they are not RSC8's.
pub(in crate::container::rpf) fn resource_from_file(file: &[u8]) -> Option<RpfResourceEntry> {
    let head = file.get(..16)?;
    if LE::read_u32(head) != RSC8_MAGIC {
        return None;
    }
    let flags = LE::read_u32(&head[4..]);
    let compressor = match flags >> 8 & 0x1F {
        0x1F => Compressor::None,
        0 => Compressor::Deflate,
        1 => Compressor::Oodle,
        _ => return None,
    };
    let key = (flags >> 16 & 0xFF).wrapping_sub(1) & 0xFF;
    Some(RpfResourceEntry {
        name_offset: 0,
        consumed_size: u32::try_from(file.len()).ok()?,
        file_offset: 0,
        system_info: LE::read_u32(&head[8..]),
        graphics_info: LE::read_u32(&head[12..]),
        compressor,
        tag: key | (flags >> 24) << 8 | (flags >> 13 & 1) << 16,
    })
}
