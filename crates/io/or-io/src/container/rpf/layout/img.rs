//! The image formats, which list files with no directories and count their offsets in 2048-byte sectors.
//!
//! IMG1 is a `.dir` of 32-byte rows beside the `.img` of bodies, and IMG2 puts the same rows after
//! an eight-byte header; neither says a file's exact size, only its sectors. IMG3 has sixteen-byte
//! rows with exact sizes, then the names one after another.

use std::io::{Seek, SeekFrom, Write};

use byteorder::{ByteOrder, LE};

use super::{bad, root_over, Input, Plan, Toc};
use crate::container::rpf::crypto::TocDecryptOutcome;
use crate::container::rpf::entry::{Compressor, RpfBinaryEntry, RpfEntry, RpfResourceEntry};
use crate::container::rpf::header::{RpfEncryption, RpfHeader};
use crate::container::rpf::version::RpfVersion;
use crate::error::{IoError, Result};

const SECTOR: u64 = 2048;
const ROW32: usize = 32;
const NAME: usize = 24;
const ROW16: usize = 16;
const IMG3_HEADER: u64 = 0x14;
const RESOURCE: u16 = 0x2000;
const OLD_RESOURCE: u16 = 0x4000;
const PADDING: u16 = 0x7FF;
/// A generic file's resource type in a GTA IV image.
const GENERIC: u32 = 1;

pub(super) fn prologue(version: RpfVersion, entries: usize, names: usize) -> u64 {
    let files = entries.saturating_sub(1);
    match version {
        RpfVersion::Img2 => 8 + (files * ROW32) as u64,
        RpfVersion::Img3 => IMG3_HEADER + (files * ROW16 + names) as u64,
        _ => 0,
    }
}

fn toc(entries: Vec<RpfEntry>, names: Vec<String>, heap: usize) -> Toc {
    Toc {
        header: RpfHeader {
            entry_count: entries.len() as u32,
            name_heap_size: heap as u32,
            name_shift: 0,
            xcompress: false,
            encryption: RpfEncryption::None,
            platform: None,
        },
        entries,
        names,
        name_heap: Vec::new(),
        outcome: TocDecryptOutcome::Nominal,
    }
}

/// A file of `sectors` sectors at `sector`, whose true length the row does not give.
fn sectors(sector: u32, count: u32) -> RpfEntry {
    RpfEntry::Binary(RpfBinaryEntry {
        name_offset: 0,
        consumed_size: 0,
        file_offset: u64::from(sector) * SECTOR,
        uncompressed_size: count.saturating_mul(SECTOR as u32),
        encrypted: false,
        encrypted_raw: 0,
        compressor: Compressor::None,
        tag: 0,
    })
}

fn row_name(row: &[u8]) -> String {
    let end = row.iter().position(|&b| b == 0).unwrap_or(row.len());
    String::from_utf8_lossy(&row[..end]).into_owned()
}

/// IMG1's table, which is the whole of its `.dir`.
pub(in crate::container::rpf) fn read_img1(dir: &[u8]) -> Result<Toc> {
    if !dir.len().is_multiple_of(ROW32) {
        return Err(IoError::RpfBadHeader {
            message: format!("a {} byte directory is not whole 32-byte rows", dir.len()),
        });
    }
    let mut entries = vec![root_over(dir.len() / ROW32)];
    let mut names = vec![String::new()];
    for row in dir.as_chunks::<ROW32>().0.iter() {
        entries.push(sectors(LE::read_u32(row), LE::read_u32(&row[4..])));
        names.push(row_name(&row[8..]));
    }
    Ok(toc(entries, names, 0))
}

pub(super) fn read(version: RpfVersion, input: &Input) -> Result<Toc> {
    if version == RpfVersion::Img3 {
        return read_img3(input);
    }
    let count = u64::from(LE::read_u32(&input.bytes(4, 4)?));
    let rows = input.bytes(8, count * ROW32 as u64)?;
    let mut entries = vec![root_over(count as usize)];
    let mut names = vec![String::new()];
    for row in rows.as_chunks::<ROW32>().0.iter() {
        entries.push(sectors(
            LE::read_u32(row),
            u32::from(LE::read_u16(&row[4..])),
        ));
        names.push(row_name(&row[8..]));
    }
    Ok(toc(entries, names, 0))
}

fn read_img3(input: &Input) -> Result<Toc> {
    let head = input.bytes(0, IMG3_HEADER)?;
    let count = u64::from(LE::read_u32(&head[8..]));
    let table_len = u64::from(LE::read_u32(&head[12..]));
    if LE::read_u16(&head[16..]) as usize != ROW16 {
        return Err(IoError::RpfBadHeader {
            message: "an IMG3 row that is not sixteen bytes".into(),
        });
    }
    let rows_len = count * ROW16 as u64;
    let table = input.bytes(IMG3_HEADER, table_len.max(rows_len))?;
    let (rows, text) = table.split_at(rows_len as usize);

    let mut names = vec![String::new()];
    names.extend(
        text.split(|&b| b == 0)
            .take(count as usize)
            .map(|n| String::from_utf8_lossy(n).into_owned()),
    );
    if names.len() as u64 != count + 1 {
        return Err(bad("fewer IMG3 names than rows"));
    }

    let mut entries = vec![root_over(count as usize)];
    for row in rows.as_chunks::<ROW16>().0.iter() {
        let flags = LE::read_u16(&row[14..]);
        let file_offset = u64::from(LE::read_u32(&row[8..])) * SECTOR;
        let disk = (u32::from(LE::read_u16(&row[12..])) << 11)
            .checked_sub(u32::from(flags & PADDING))
            .ok_or_else(|| bad("an IMG3 file shorter than its padding"))?;
        let tag = u32::from(flags & !PADDING);
        entries.push(if flags & RESOURCE != 0 && flags & OLD_RESOURCE == 0 {
            RpfEntry::Resource(RpfResourceEntry {
                name_offset: 0,
                consumed_size: disk,
                file_offset,
                system_info: LE::read_u32(row),
                graphics_info: LE::read_u32(&row[4..]),
                compressor: Compressor::None,
                tag,
            })
        } else {
            RpfEntry::Binary(RpfBinaryEntry {
                name_offset: 0,
                consumed_size: 0,
                file_offset,
                uncompressed_size: disk,
                encrypted: false,
                encrypted_raw: 0,
                compressor: Compressor::None,
                tag,
            })
        });
    }
    Ok(toc(entries, names, text.len()))
}

/// The sectors a body of `len` bytes takes, as a 16-bit count.
fn sector_count(len: u32) -> Result<u16> {
    u16::try_from(u64::from(len).div_ceil(SECTOR))
        .map_err(|_| bad(format!("{len} bytes are more sectors than a row counts")))
}

fn sector_of(offset: u64) -> Result<u32> {
    u32::try_from(offset / SECTOR).map_err(|_| bad(format!("offset {offset:#x} is past 8 TiB")))
}

fn row32(entry: &RpfEntry, name: &str, version: RpfVersion) -> Result<[u8; ROW32]> {
    let RpfEntry::Binary(b) = entry else {
        return Err(bad(format!("{version} holds binary files only")));
    };
    if name.len() >= NAME {
        return Err(bad(format!(
            "{name:?} does not fit {version}'s 23-character name"
        )));
    }
    let mut row = [0u8; ROW32];
    LE::write_u32(&mut row, sector_of(b.file_offset)?);
    let count = sector_count(b.uncompressed_size)?;
    if version == RpfVersion::Img1 {
        LE::write_u32(&mut row[4..], u32::from(count));
    } else {
        LE::write_u16(&mut row[4..], count);
    }
    row[8..8 + name.len()].copy_from_slice(name.as_bytes());
    Ok(row)
}

fn row16(entry: &RpfEntry) -> Result<[u8; ROW16]> {
    let (disk, flags, first, second, offset) = match entry {
        RpfEntry::Binary(b) => (
            b.uncompressed_size,
            b.tag as u16,
            b.uncompressed_size,
            GENERIC,
            b.file_offset,
        ),
        RpfEntry::Resource(r) => (
            r.consumed_size,
            r.tag as u16,
            r.system_info,
            r.graphics_info,
            r.file_offset,
        ),
        RpfEntry::Directory(_) => return Err(bad("IMG3 has no directories")),
    };
    let used = sector_count(disk)?;
    let mut row = [0u8; ROW16];
    LE::write_u32(&mut row, first);
    LE::write_u32(&mut row[4..], second);
    LE::write_u32(&mut row[8..], sector_of(offset)?);
    LE::write_u16(&mut row[12..], used);
    LE::write_u16(
        &mut row[14..],
        (u32::from(used) * SECTOR as u32 - disk) as u16 | flags,
    );
    Ok(row)
}

fn write_rows<W: Write>(w: &mut W, plan: &Plan, version: RpfVersion) -> Result<()> {
    for (entry, name) in plan.entries.iter().zip(plan.names).skip(1) {
        w.write_all(&row32(entry, name, version)?)?;
    }
    Ok(())
}

/// IMG1's `.dir`, one row per file in the order the bodies lie.
pub(in crate::container::rpf) fn write_img1<W: Write>(dir: &mut W, plan: &Plan) -> Result<()> {
    write_rows(dir, plan, RpfVersion::Img1)
}

pub(super) fn write<W: Write + Seek>(version: RpfVersion, w: &mut W, plan: &Plan) -> Result<()> {
    let files = plan.entries.len() - 1;
    w.seek(SeekFrom::Start(0))?;
    if version == RpfVersion::Img2 {
        w.write_all(&version.magic().unwrap_or(0).to_le_bytes())?;
        w.write_all(&(files as u32).to_le_bytes())?;
        return write_rows(w, plan, version);
    }
    let rows = plan.entries[1..]
        .iter()
        .map(row16)
        .collect::<Result<Vec<_>>>()?;
    let names: Vec<u8> = plan.names[1..]
        .iter()
        .flat_map(|n| n.bytes().chain([0]))
        .collect();
    let mut head = [0u8; IMG3_HEADER as usize];
    LE::write_u32(&mut head, version.magic().unwrap_or(0));
    LE::write_u32(&mut head[4..], 3);
    LE::write_u32(&mut head[8..], files as u32);
    LE::write_u32(&mut head[12..], (files * ROW16 + names.len()) as u32);
    LE::write_u16(&mut head[16..], ROW16 as u16);
    w.write_all(&head)?;
    for row in rows {
        w.write_all(&row)?;
    }
    w.write_all(&names)?;
    Ok(())
}
