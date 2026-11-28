//! A pack texture given another size, format or mip count: its rows planned anew from its shape as
//! the pack's own textures' are, every later texture moved after it, and the header, the table and
//! the directory's entries restated when the pack is saved.

use std::fs::File;
use std::ops::Range;

use super::super::{
    malformed, u32_at, Entry, Format, Row, TexturePack, TtpfError, ENTRY_LEN, HEADER_LEN,
    KIND_ZLIB, ROW_LEN, TEXTURE_LIMIT,
};
use super::{moved_to_page, place, start_at, zlib, Rule, SaveLayout, OFFSET_LIMIT, TOP_ROW};
use crate::codec::texture::{DxgiFormat, GrcTextureFormat};
use crate::gta6::tiling::{chain, chain_lengths, place_planes, Level, Tile};

/// A side is stated in 14 bits, less one.
const SIDE_LIMIT: u32 = 1 << 14;
/// A directory entry states its first page in 21 bits.
const ENTRY_PAGES: u64 = 1 << 21;
/// A table row states its page in 22 bits.
const ROW_PAGES: u64 = 1 << 22;
/// A top mip of at least this many bytes in 64 KiB tiles takes them.
const LARGE_TOP: u64 = 1 << 20;
/// A one-level texture of at least this many bytes in 64 KiB tiles takes them; a chain of several
/// levels whose top mip is at least this many bytes in its own tiles takes flags 3.
const LARGE_LONE: u64 = 128 << 10;

/// What a texture becomes: its size, its format, and its mips, or as many as its chain places when `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub mips: Option<u32>,
}

/// The table of the file on disk, which a pack whose rows moved carries its other rows from.
#[derive(Debug, Clone)]
pub(in crate::gta6) struct Moved {
    rows: Vec<Row>,
    /// The row of that table each row carries; `None` for one a reshape wrote.
    from: Vec<Option<usize>>,
    /// Where the file departs from the 4 KiB start rule or the rule does not decide, whether the file put the row on the next 4 KiB page.
    kept: Vec<Option<bool>>,
}

/// Where a reshaped texture's levels go, and the entry and rows that state them.
struct Planned {
    entry: Entry,
    format: Format,
    tile: Tile,
    levels: Vec<Level>,
    /// Each row's decoded bytes, in virtual order.
    rows: Vec<u64>,
    /// For each slice, each mip written and where in the rows' memory: a cube's earlier rows hold its smaller cubes.
    places: Vec<Vec<(usize, Level)>>,
}

impl Format {
    /// The pack format the tree's `format` is written as, its sRGB form when `srgb` and it has one.
    pub fn of_grc(format: GrcTextureFormat, srgb: bool) -> Option<Self> {
        u8::try_from(format.to_dxgi(srgb).to_raw())
            .ok()
            .and_then(Self::of_dxgi)
    }
}

impl TexturePack {
    /// Whether a texture changed size, format or mips since the pack was opened, so a save moves the textures after it.
    pub fn is_relaid(&self) -> bool {
        self.moved.is_some()
    }

    /// The table the file on disk states.
    pub(in crate::gta6) fn file_rows(&self) -> &[Row] {
        match &self.moved {
            Some(moved) => &moved.rows,
            None => &self.rows,
        }
    }

    /// The rows of the file on disk that bound the stored bytes row `row` carries.
    pub(in crate::gta6) fn file_span(&self, row: usize) -> Result<(Row, Row), TtpfError> {
        let at = match &self.moved {
            Some(moved) => moved.from.get(row).copied().flatten(),
            None => Some(row),
        };
        let rows = self.file_rows();
        at.and_then(|at| Some((*rows.get(at)?, *rows.get(at + 1)?)))
            .ok_or_else(|| {
                malformed(format!(
                    "row {row} holds no bytes of the file the pack was opened from"
                ))
            })
    }

    /// Texture `index` given `shape` and `levels`, its mips top first as linear elements of the shape's format: its rows planned anew and every later texture's moved after them.
    pub fn reshape_levels(
        &mut self,
        index: usize,
        shape: Shape,
        levels: &[Vec<u8>],
    ) -> Result<(), TtpfError> {
        let planned = self.planned(index, shape)?;
        self.relay(index, planned, levels)
    }

    /// Texture `index` encoded from `image` into `format` with the mips `options` asks for, or as many as its chain places; how many it holds.
    #[cfg(feature = "authoring")]
    pub fn reshape(
        &mut self,
        index: usize,
        image: &crate::codec::texture::RgbaImage,
        format: Format,
        options: &crate::authoring::TextureBuildOptions,
    ) -> Result<u32, TtpfError> {
        self.reshape_slices(index, std::slice::from_ref(image), format, options)
    }

    /// Texture `index` encoded from `images`, one for each of its slices and all of one size, into `format` with the mips `options` asks for, or as many as its chain places; how many it holds.
    #[cfg(feature = "authoring")]
    pub fn reshape_slices(
        &mut self,
        index: usize,
        images: &[crate::codec::texture::RgbaImage],
        format: Format,
        options: &crate::authoring::TextureBuildOptions,
    ) -> Result<u32, TtpfError> {
        // A signed BC6H texture would be written from the unsigned blocks the encoder makes.
        if !crate::authoring::is_encodable(format.stored()) || format.dxgi() == DxgiFormat::Bc6hSf16
        {
            return Err(TtpfError::ReshapeFormat {
                index,
                format: format.name(),
            });
        }
        let slices = self.entries.get(index).map_or(1, |e| usize::from(e.slices));
        let one_size = (images.windows(2))
            .all(|pair| (pair[0].width, pair[0].height) == (pair[1].width, pair[1].height));
        let Some(first) = images.first().filter(|_| one_size) else {
            return Err(TtpfError::SliceImages { index, slices });
        };
        let shape = Shape {
            width: first.width,
            height: first.height,
            format,
            mips: options.mip_levels,
        };
        let planned = self.planned(index, shape)?;
        if images.len() != planned.places.len() {
            let slices = planned.places.len();
            return Err(TtpfError::SliceImages { index, slices });
        }
        let count = planned.levels.len() as u32;
        let encode = |e: crate::Error| TtpfError::Encode(e.to_string());
        let mut levels = Vec::with_capacity(images.len() * count as usize);
        for image in images {
            let rgba = crate::codec::texture::generate_mip_chain_with(
                image.width,
                image.height,
                &image.data,
                Some(count),
                options.filter,
                options.srgb,
                options.alpha,
            )
            .map_err(encode)?;
            for (level, rgba) in rgba.iter().enumerate() {
                let side = |pixels: u32| (pixels >> level).max(1);
                let (w, h) = (side(image.width), side(image.height));
                let encoded =
                    crate::authoring::encode_level(format.stored(), w, h, rgba, options.quality);
                levels.push(encoded.map_err(encode)?);
            }
        }
        self.relay(index, planned, &levels)?;
        Ok(count)
    }

    /// The entry, rows and levels of texture `index` given `shape`, as the pack's own textures of that shape have them.
    fn planned(&self, index: usize, shape: Shape) -> Result<Planned, TtpfError> {
        self.edits_rows()?;
        if !self.is_complete() || self.entries.iter().any(|e| !e.on_disk) {
            return Err(TtpfError::NotWhole);
        }
        let count = self.entries.len();
        let was = *self
            .entries
            .get(index)
            .ok_or(TtpfError::NoSuchTexture { index, count })?;
        let flat = matches!((was.dimension, was.slices), (1 | 5, 1));
        let sliced = !flat
            && (matches!((was.dimension, was.slices), (2, 6))
                || (was.dimension == 5 && was.tile_mode == 0));
        let volume = was.dimension == 3;
        if !flat && !sliced && !volume {
            return Err(TtpfError::ReshapeShape { index });
        }
        let Shape {
            width,
            height,
            format,
            mips,
        } = shape;
        let too_large = || TtpfError::ShapeTooLarge {
            index,
            width,
            height,
        };
        if !(1..=SIDE_LIMIT).contains(&width) || !(1..=SIDE_LIMIT).contains(&height) {
            return Err(too_large());
        }
        if sliced {
            return self.planned_slices(index, was, shape);
        }
        if volume {
            return self.planned_volume(index, was, shape);
        }
        let top = chain_lengths(width, height, format, &Tile::of(format, true))
            .first()
            .copied()
            .unwrap_or_default();
        let mips = match mips {
            Some(mips) => mips,
            None => {
                let tile = Tile::of(format, top >= LARGE_TOP);
                let whole = (chain_lengths(width, height, format, &tile).last())
                    .copied()
                    .unwrap_or_default();
                chain(width, height, format, &tile, false, whole)
                    .map_or(1, |levels| levels.len().max(1)) as u32
            }
        };

        // 64 KiB tiles for a large top mip or a large lone level, else 4 KiB tiles.
        let large = top >= LARGE_TOP || (mips == 1 && top >= LARGE_LONE);
        let tile = Tile::of(format, large);
        let lengths = chain_lengths(width, height, format, &tile);
        let explicit = lengths.len().saturating_sub(1);
        let lone = mips == 1 && explicit == 0;
        let whole = lengths.last().copied().unwrap_or_default();
        let all = chain(width, height, format, &tile, lone, whole);
        let all = all.unwrap_or_default();
        let wanted = mips as usize;
        if wanted == 0 || wanted > all.len() {
            return Err(TtpfError::MipsNotPlaced {
                index,
                mips,
                most: all.len(),
            });
        }
        let levels = if wanted <= explicit {
            let explicit = chain(width, height, format, &tile, false, lengths[wanted - 1]);
            explicit.unwrap_or_default()
        } else {
            all[..wanted].to_vec()
        };
        if levels.len() != wanted {
            return Err(malformed(format!(
                "texture {index}'s new chain places {} mips, not {wanted}",
                levels.len()
            )));
        }
        // The pieces smallest first, the tail tile before the explicit levels.
        let mut pieces: Vec<u64> = (levels.iter().rev())
            .filter(|l| !l.tail)
            .map(|l| l.len)
            .collect();
        if levels.iter().any(|l| l.tail) {
            pieces.insert(0, tile.bytes());
        }
        let flags: u16 = match (mips, large) {
            (1, true) => 1,
            (1, false) => 0,
            _ if lengths.first().is_some_and(|&top| top >= LARGE_LONE) => 3,
            _ => flags_of(&pieces),
        };

        // The rows: one for each of the largest `flags` pieces, row 0 for the rest.
        let own_rows = pieces.len().min(usize::from(flags) + 1);
        let first = pieces.len() + 1 - own_rows;
        let (mut rows, own) = (Vec::new(), pieces[first..].chunks(1));
        for group in std::iter::once(&pieces[..first]).chain(own) {
            let size: u64 = group.iter().sum();
            if size <= TOP_ROW {
                rows.push(size);
                continue;
            }
            // Only a level of its own is cut into 4 MiB rows.
            if group.len() > 1 || !size.is_multiple_of(TOP_ROW) {
                return Err(TtpfError::RowNotCoded { index });
            }
            rows.extend(std::iter::repeat_n(TOP_ROW, (size / TOP_ROW) as usize));
        }

        // The nibble counts row 0's levels, or is 0 when there are fewer pieces than rows the flags allow.
        let nibble = if pieces.len() < usize::from(flags) + 1 {
            0
        } else {
            wanted + 1 - own_rows
        };
        let held = if nibble == 0 {
            own_rows
        } else {
            nibble + own_rows - 1
        };
        if held != wanted || nibble > 15 {
            return Err(TtpfError::NibbleCannotState { index, mips });
        }

        // 4 KiB tiles take their tile mode from the row count and the top row's pages.
        let tile_mode = match (large, rows.len(), rows.last().map_or(0, |row| row >> 12)) {
            (true, ..) => 0,
            (false, 1, _) => 1,
            (false, 3, top) if top >= 128 => 1,
            (false, 4, top) if top <= 32 => 3,
            _ => 2,
        };
        let grows = if format.dxgi() == DxgiFormat::Bc6hSf16 {
            16
        } else {
            1
        };
        if grows * rows.iter().sum::<u64>() > TEXTURE_LIMIT {
            return Err(too_large());
        }
        let dxgi = u16::try_from(format.dxgi().to_raw()).unwrap_or_default();
        let entry = Entry {
            width,
            height,
            mip_count: nibble as u8,
            format_code: dxgi << 3 | flags,
            tile_mode,
            row_codes: [0; 4],
            small_rows: 0,
            align_pages: 0,
            ..was
        };
        let places = vec![levels.iter().copied().enumerate().collect()];
        Ok(Planned {
            entry,
            format,
            tile,
            levels,
            rows,
            places,
        })
    }

    /// `levels` tiled into `planned`'s rows, each kept as a zlib stream, and the table moved, once the reader finds each level where it was written.
    fn relay(
        &mut self,
        index: usize,
        planned: Planned,
        levels: &[Vec<u8>],
    ) -> Result<(), TtpfError> {
        let Planned {
            entry,
            format,
            tile,
            levels: placed,
            rows,
            places,
        } = planned;
        let mips = placed.len();
        if levels.len() != places.len() * mips {
            return Err(malformed(format!(
                "texture {index}'s new chain holds {mips} mips in {} slice(s), not {} in all",
                places.len(),
                levels.len()
            )));
        }
        let element = format.element_bytes();
        let mut memory = vec![0u8; rows.iter().sum::<u64>() as usize];
        // A volume's slice is one z-plane, placed in the z-slab of tiles that holds it.
        let slab = (entry.dimension == 3).then(|| {
            let slabs = u64::from(entry.slices).div_ceil(tile.depth as u64);
            memory.len() as u64 / slabs.max(1)
        });
        for (slice, written) in places.iter().enumerate() {
            for (i, level) in written {
                let fits = (levels.get(slice * mips + i))
                    .filter(|e| e.len() == level.width * level.height * element)
                    .is_some_and(|e| match slab {
                        Some(slab) => {
                            let (size, planes) = ([level.width, level.height], slice..slice + 1);
                            place_planes(&mut memory, level.at, size, planes, e, &tile, slab)
                        }
                        None => place(&mut memory, level, e, &tile, element),
                    });
                if !fits {
                    return Err(malformed(format!(
                        "mip {i} of texture {index}'s new chain is not {}x{} elements",
                        level.width, level.height
                    )));
                }
            }
        }
        let (mut at, mut bodies) = (0usize, Vec::with_capacity(rows.len()));
        for &size in &rows {
            let end = at + size as usize;
            bodies.push(zlib(&memory[at..end])?);
            at = end;
        }
        let mut next = self.clone();
        next.moved_to(index, entry, &rows, bodies)?;
        let read = next.layout_of(index)?;
        let same = |r: &Level, p: &Level| {
            (r.at, r.width, r.height, r.tail) == (p.at, p.width, p.height, p.tail)
        };
        let found = &read.layout.levels;
        // An array's reader reads on through any tail slot that holds data, and the rest are zero.
        let array = places.len() > 1 && entry.dimension == 5;
        let counted = found.len() == placed.len() || (array && found.len() > placed.len());
        let unread = || {
            malformed(format!(
                "texture {index}'s new rows would not be read where they are written"
            ))
        };
        if !counted || !found.iter().zip(&placed).all(|(r, p)| same(r, p)) {
            return Err(unread());
        }
        // A cube's faces and an array's slices are read back whole, since each lies elsewhere.
        if places.len() > 1 && format.dxgi() != DxgiFormat::Bc6hSf16 {
            let back = next.texture(index)?;
            if back.levels as usize != mips || back.pixels != levels.concat() {
                return Err(unread());
            }
        }
        *self = next;
        Ok(())
    }

    /// Texture `index` stated by `entry` over rows of `sizes` bytes written as `bodies`, every later row and entry moved by the change in length, and the header and row 0 restated.
    fn moved_to(
        &mut self,
        index: usize,
        mut entry: Entry,
        sizes: &[u64],
        bodies: Vec<Vec<u8>>,
    ) -> Result<(), TtpfError> {
        let old = self
            .rows_of(index)
            .ok_or(TtpfError::RangeNotKnown { index })?;
        let start = self.rows[old.start].virtual_address;
        let was = self.rows[old.end].virtual_address - start;
        let now: u64 = sizes.iter().sum();
        let mut moved = match self.moved.take() {
            Some(moved) => *moved,
            None => Moved {
                kept: self.kept()?,
                from: (0..self.rows.len()).map(Some).collect(),
                rows: self.rows.clone(),
            },
        };
        let mut va = start;
        let written: Vec<Row> = (sizes.iter())
            .map(|&size| {
                let row = Row {
                    offset: 0,
                    kind: KIND_ZLIB,
                    virtual_address: va,
                    pad: 0,
                };
                va += size;
                row
            })
            .collect();
        let added = written.len();
        self.rows = spliced(&self.rows, &old, &written);
        for row in &mut self.rows[old.start + added..] {
            row.virtual_address = row.virtual_address - was + now;
        }
        moved.from = spliced(&moved.from, &old, &vec![None; added]);
        moved.kept = spliced(&moved.kept, &old, &vec![None; added]);
        for first in self.first_rows[index + 1..].iter_mut().flatten() {
            *first = *first - old.len() + added;
        }
        entry.page = self.entries[index].page;
        self.entries[index] = entry;
        for later in &mut self.entries[index + 1..] {
            let page = ((u64::from(later.page) << 12) - was + now) >> 12;
            if page >= ENTRY_PAGES {
                return Err(TtpfError::PagePastField(page));
            }
            later.page = page as u32;
        }
        let n = self.rows.len();
        let end = self.rows[n - 1].virtual_address;
        if end >> 12 >= ROW_PAGES {
            return Err(TtpfError::PagePastField(end >> 12));
        }
        let too_many = |_| malformed(format!("{n} rows do not fit the header"));
        self.header.row_count = u32::try_from(n).map_err(too_many)?;
        self.header.virtual_size = end;
        self.rows[0].offset = (HEADER_LEN + ROW_LEN * n as u64).next_multiple_of(0x100);
        self.replaced.0.insert(index, bodies);
        self.moved = Some(Box::new(moved));
        // The row-size codes, small_rows and align_pages from the rows; an array in 64 KiB tiles
        // and a volume state none, as a 2D texture in them does not.
        let (dimension, slices) = (self.entries[index].dimension, self.entries[index].slices);
        let array = dimension == 3 || (dimension == 5 && slices > 1);
        let coded = match array {
            true => Some(([0; 4], 0, 0)),
            false => self.codes_by_rule(index),
        };
        let (codes, small, align) = coded.ok_or(TtpfError::RowNotCoded { index })?;
        let entry = &mut self.entries[index];
        (entry.row_codes, entry.small_rows, entry.align_pages) = (codes, small, align);
        self.rows = self.save_layout()?.rows;
        Ok(())
    }

    /// For each row of a pack whose rows have not moved, the file's own start where the file departs from the start rule or the rule does not decide.
    fn kept(&self) -> Result<Vec<Option<bool>>, TtpfError> {
        let starts = self.starts();
        let (mut rule, mut kept) = (Rule::default(), vec![None; self.rows.len()]);
        for i in 1..self.rows.len() {
            let end = self.rows[i - 1].offset + self.stored_len(i - 1)?;
            let offset = self.rows[i].offset;
            let moved =
                moved_to_page(end, offset).ok_or(TtpfError::GapNotCarried { row: i - 1 })?;
            let decided = (i > 1).then(|| rule.page(starts[i], end)).flatten();
            if decided.map(|page| start_at(page, end)) != Some(offset) {
                kept[i] = Some(moved);
            }
            rule.step(starts[i], end, offset);
        }
        Ok(kept)
    }

    /// The bytes a save writes for row `row` before its pad.
    fn written_len(&self, row: usize) -> Result<u64, TtpfError> {
        match self.replaced_row(row) {
            Some(bytes) => Ok(bytes.len() as u64),
            None => self.stored_len(row),
        }
    }

    /// The table a save of a pack whose rows moved writes, each row by the start rule from row 1 on, or as the file had it where it departs from the rule or the rule does not decide.
    pub(super) fn relaid_layout(&self, moved: &Moved) -> Result<SaveLayout, TtpfError> {
        let n = self.rows.len();
        let starts = self.starts();
        let mut rows = vec![self.rows[0]];
        let mut rule = Rule::default();
        let (mut departs, mut undecided) = (Vec::new(), Vec::new());
        for i in 1..n {
            let end = rows[i - 1].offset + self.written_len(i - 1)?;
            let written = self.replaced_row(i).is_some();
            let decided = rule.page(starts[i], end);
            // A row a replacement wrote takes the rule, except row 1 and a row the rule does not
            // decide, as a save that moves nothing has it.
            let kept = moved.kept.get(i).copied().flatten();
            let carried = kept.filter(|_| i == 1 || !written || decided.is_none());
            let page = match carried {
                Some(page) => {
                    match decided {
                        None => undecided.push(i),
                        Some(by_rule) if by_rule != page => departs.push(i),
                        Some(_) => {}
                    }
                    page
                }
                // A row a reshape wrote that the rule does not decide, an array's, takes 256-byte
                // alignment.
                None => decided.unwrap_or(false),
            };
            let offset = start_at(page, end);
            rule.step(starts[i], end, offset);
            // Less than one 4 KiB page, which the pad's 12 bits hold.
            rows[i - 1].pad = (offset - end) as u16;
            let kind = if written {
                KIND_ZLIB
            } else {
                self.rows[i].kind
            };
            rows.push(Row {
                offset,
                kind,
                virtual_address: self.rows[i].virtual_address,
                pad: 0,
            });
        }
        let last = rows[n - 2];
        let sentinel = &mut rows[n - 1];
        (sentinel.kind, sentinel.pad) = (last.kind, last.pad);
        if sentinel.offset > OFFSET_LIMIT - 0x100 {
            return Err(TtpfError::TooLarge(sentinel.offset));
        }
        Ok(SaveLayout {
            rows,
            departs,
            undecided,
        })
    }

    /// Row `row`'s bytes as the file stores them, the directory with every entry restated once rows have moved.
    pub(super) fn carried(&self, file: &mut File, row: usize) -> Result<Vec<u8>, TtpfError> {
        let mut bytes = self.stored(file, row)?;
        if row == 0 && self.moved.is_some() {
            for (k, entry) in self.entries.iter().enumerate() {
                let at = (HEADER_LEN + ENTRY_LEN * k as u64) as usize;
                let end = at + ENTRY_LEN as usize;
                let short = || malformed("the directory is shorter than its entries".into());
                let words = bytes.get_mut(at..end).ok_or_else(short)?;
                restate(words, entry);
            }
        }
        Ok(bytes)
    }
}

/// The file header with the row count and the virtual size of `rows`, the table a save writes.
pub(super) fn restate_head(head: &mut [u8], rows: &[Row]) {
    let size = rows.last().map_or(0, |sentinel| sentinel.virtual_address);
    head[4..8].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    head[8..12].copy_from_slice(&(size as u32).to_le_bytes());
    head[0x0C] = (size >> 32) as u8;
}

/// Writes `entry`'s fields into its 16 bytes, keeping every bit no field reads.
fn restate(words: &mut [u8], entry: &Entry) {
    let packed = (entry.row_codes.iter().enumerate()).fold(0u32, |packed, (j, &code)| {
        packed | u32::from(code) << (5 * j)
    });
    let w0 = (entry.width - 1) | (entry.height - 1) << 14 | u32::from(entry.mip_count) << 28;
    // Bit 26 marks a cube in tile mode 0.
    let cube = u32::from(entry.dimension == 2);
    let w4 = u32_at(words, 4) & !(0x3FF | 3 << 24 | cube << 26)
        | u32::from(entry.format_code)
        | u32::from(entry.tile_mode) << 24
        | (cube & u32::from(entry.tile_mode == 0)) << 26;
    let w8 = entry.page | packed << 21;
    let w12 = u32_at(words, 12) & !(0x1FF | 3 << 19 | 0x7FF << 21)
        | packed >> 11
        | u32::from(entry.small_rows) << 19
        | u32::from(entry.align_pages) << 21;
    for (i, word) in [w0, w4, w8, w12].into_iter().enumerate() {
        words[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
}

/// The flags of a 2D chain of several levels in 4 KiB tiles, from its tiled pieces (the tail tile
/// and each explicit level): 0 when the largest is under 16 KiB, 3 when the second largest is
/// 32 KiB or more, else 1.
fn flags_of(pieces: &[u64]) -> u16 {
    let mut sorted = pieces.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    match (sorted.first(), sorted.get(1)) {
        (Some(&largest), _) if largest < 16 << 10 => 0,
        (_, Some(&second)) if second >= 32 << 10 => 3,
        _ => 1,
    }
}

/// `all` with the items of `old` replaced by `new`.
fn spliced<T: Clone>(all: &[T], old: &Range<usize>, new: &[T]) -> Vec<T> {
    [&all[..old.start], new, &all[old.end..]].concat()
}

#[path = "ttpf_reshape_slices.rs"]
mod slices;
#[path = "ttpf_reshape_volume.rs"]
mod volume;
