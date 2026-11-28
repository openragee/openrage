//! Where the PlayStation 5 lays a texture in a pack's image: the Xbox rows end to end, in its own
//! 64 KiB tiles.
//!
//! ⚠ A range runs to the next entry's page and is padded, so a texture's length comes from its
//! entry, never from its range.

use super::bcpack;
use super::tiling::{chain, chain_lengths, volume_levels, Layout, Tile};
use super::ttpf::{
    malformed, row_code_pages, smallest_row_holding, Entry, Format, Texture, TexturePack,
    TtpfError, TEXTURE_LIMIT,
};
use crate::codec::texture::DxgiFormat;

/// Where a texture's levels lie in the image.
pub(super) struct Planned {
    /// The image bytes its levels lie in.
    pub(super) start: u64,
    pub(super) len: u64,
    /// Where the zero bytes after a cube's six faces start; `len` for any other shape.
    pad_from: u64,
    pub(super) layout: Layout,
}

/// Where texture `index`'s levels lie, from its entry alone, or why it cannot be read.
pub(super) fn plan(pack: &TexturePack, index: usize) -> Result<Planned, TtpfError> {
    let count = pack.entries.len();
    let Some(entry) = pack.entries.get(index) else {
        return Err(TtpfError::NoSuchTexture { index, count });
    };
    let range = pack
        .range_of(index)
        .ok_or(TtpfError::RangeNotKnown { index })?;
    let format = entry.format().ok_or(TtpfError::UnknownFormat {
        index,
        code: entry.format_code,
    })?;
    let mut planned = match (entry.dimension, entry.slices) {
        (3, _) => volume(index, entry, format)?,
        (2, 6) => cube(index, entry, format)?,
        (1 | 5, 1) => flat(index, entry, format, 1)?,
        (5, slices) if entry.tile_mode == 0 => flat(index, entry, format, slices.into())?,
        (dimension, slices) => {
            return Err(TtpfError::UnknownShape {
                index,
                dimension,
                slices,
            })
        }
    };
    planned.start += range.start;
    let (held, len) = (range.end - range.start, planned.len);
    if planned.start + len > range.end {
        return Err(malformed(format!(
            "texture {index}'s entry states {len} bytes, and its range holds {held}"
        )));
    }
    // A signed BC6H block of 16 bytes reads as 16 pixels of four floats.
    let grows = if format.dxgi() == DxgiFormat::Bc6hSf16 {
        16
    } else {
        1
    };
    if grows * len > TEXTURE_LIMIT {
        return Err(malformed(format!(
            "texture {index}'s {len} bytes are more than the {TEXTURE_LIMIT} a texture is read into"
        )));
    }
    Ok(planned)
}

/// The levels the entry of a 2D texture or of an array's slice holds.
fn held_levels(entry: &Entry) -> usize {
    let flags = usize::from(entry.format_code & 7);
    match usize::from(entry.mip_count) {
        0 => flags.max(1),
        nibble => nibble + flags,
    }
}

/// A 2D texture, or an array of `slices` in 64 KiB tiles, piece by piece.
fn flat(index: usize, entry: &Entry, format: Format, slices: u64) -> Result<Planned, TtpfError> {
    let tile = Tile::prospero(format, entry.tile_mode == 0);
    let held = held_levels(entry);
    let lengths = chain_lengths(entry.width, entry.height, format, &tile);
    let lone = entry.mip_count <= 1;
    let unit = lengths
        .get(held.min(lengths.len()).wrapping_sub(1))
        .copied();
    let mut levels = unit
        .and_then(|unit| chain(entry.width, entry.height, format, &tile, lone, unit))
        .ok_or_else(|| {
            malformed(format!(
                "texture {index}'s entry states {held} level(s), which no mip chain of its size holds"
            ))
        })?;
    if levels.is_empty() {
        return Err(TtpfError::TopMipInTail {
            index,
            format: format.name(),
        });
    }
    let unit = unit.unwrap_or_default();
    if slices == 1 {
        levels.truncate(held);
    }
    let len = unit * slices;
    Ok(Planned {
        start: 0,
        len,
        pad_from: len,
        layout: Layout {
            format,
            width: entry.width,
            height: entry.height,
            tile,
            levels,
            slices,
            unit,
            piece_major: slices > 1,
            stated: slices == 1,
            depth: None,
        },
    })
}

/// A cube: its six faces lie in the last of the rows its row codes state, in 4 KiB tiles whatever its tile mode.
fn cube(index: usize, entry: &Entry, format: Format) -> Result<Planned, TtpfError> {
    if format.element_side() == 1 && format.element_bytes() != 4 {
        let bytes = format.element_bytes();
        return Err(TtpfError::CubeNotMeasured { index, bytes });
    }
    let flags = entry.format_code & 7;
    let rows = (entry.row_codes.get(..=usize::from(flags)))
        .and_then(|codes| {
            let pages = codes.iter().map(|&code| row_code_pages(code));
            pages.collect::<Option<Vec<u32>>>()
        })
        .ok_or_else(|| {
            malformed(format!(
                "texture {index}'s row-size codes do not state the {} row(s) of its cube",
                flags + 1
            ))
        })?;
    let bytes = |pages: &u32| u64::from(*pages) << 12;
    let (before, last) = match rows.split_last() {
        Some((last, before)) => (before.iter().map(bytes).sum::<u64>(), bytes(last)),
        None => (0, 0),
    };
    let tile = Tile::of(format, false);
    let lone = entry.mip_count <= 1;
    let lengths = chain_lengths(entry.width, entry.height, format, &tile);
    let unit = if lone {
        lengths.first()
    } else {
        lengths.last()
    };
    let unit = (unit.copied()).filter(|&unit| smallest_row_holding(6 * unit) == Some(last));
    let mut levels = unit
        .and_then(|unit| chain(entry.width, entry.height, format, &tile, lone, unit))
        .ok_or_else(|| {
            malformed(format!(
                "texture {index}'s last row of {last} bytes is no 6 mip chains of its size"
            ))
        })?;
    let unit = unit.unwrap_or_default();
    // Each face holds its lone level 0, or as many levels as its nibble and one more for each
    // row after the first.
    let held = if lone {
        1
    } else {
        usize::from(entry.mip_count) + usize::from(flags)
    };
    let explicit = levels.iter().filter(|l| !l.tail).count();
    if held < explicit || held > levels.len() {
        return Err(malformed(format!(
            "texture {index}'s nibble and flags count {held} levels in each face, and its last row \
             places {explicit} outside its tail and {} in all",
            levels.len()
        )));
    }
    levels.truncate(held);
    Ok(Planned {
        start: before,
        len: last,
        pad_from: 6 * unit,
        layout: Layout {
            format,
            width: entry.width,
            height: entry.height,
            tile,
            levels,
            slices: 6,
            unit,
            piece_major: false,
            stated: true,
            depth: None,
        },
    })
}

/// A volume: z-slabs of 4 KiB tiles, each holding the chain.
fn volume(index: usize, entry: &Entry, format: Format) -> Result<Planned, TtpfError> {
    let tile = Tile::volume(format)
        .filter(|_| entry.tile_mode == 1)
        .ok_or(TtpfError::Volume {
            index,
            tile_mode: entry.tile_mode,
        })?;
    let (width, height, depth) = (entry.width, entry.height, u32::from(entry.slices));
    let mips = u32::from(entry.mip_count.max(1));
    let (levels, unit) = volume_levels([width, height, depth], format, &tile, mips);
    let len = unit * u64::from(depth).div_ceil(tile.depth as u64);
    Ok(Planned {
        start: 0,
        len,
        pad_from: len,
        layout: Layout {
            format,
            width,
            height,
            tile,
            levels,
            slices: depth.into(),
            unit,
            piece_major: false,
            stated: true,
            depth: Some(depth),
        },
    })
}

/// Texture `index` as `TexturePack::texture_in_tiles` gives one of the Xbox tiles, out of the image.
pub(super) fn texture(
    pack: &TexturePack,
    index: usize,
) -> Result<(Texture, bcpack::Decode), TtpfError> {
    let Planned {
        start,
        len,
        pad_from,
        layout,
    } = plan(pack, index)?;
    let memory = pack.image(start, len)?;
    if memory[pad_from as usize..].iter().any(|&b| b != 0) {
        return Err(malformed(format!(
            "texture {index}'s last row holds bytes other than zero past its six faces"
        )));
    }
    let (levels, pixels) = layout.read(&memory, 0).ok_or_else(|| {
        malformed(format!(
            "texture {index}'s top mip does not lie in its range"
        ))
    })?;
    let texture = Texture {
        width: layout.width,
        height: layout.height,
        format: layout.format,
        levels,
        faces: layout.slices as u32,
        pixels,
    };
    Ok((texture, bcpack::Decode::default()))
}
