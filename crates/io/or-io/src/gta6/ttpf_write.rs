//! Writing a `.ttpf`: every row of a texture left alone is carried as the file stores it, codecs
//! no reader decodes included, and a replaced texture is tiled and written as zlib rows of the
//! sizes its entry states, so the directory, the keys and every virtual address stay as they were.
//! A pack with no rows stores its image as it is: a replaced 2D texture is tiled over its own bytes,
//! and the file is written as it is around them.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;

use super::super::tiling::{Level, Tile};
use super::super::tiling_prospero;
use super::{
    read_at, row_code_pages, Envelope, Plan, Row, TexturePack, TtpfError, HEADER_LEN, KIND_ZLIB,
};

/// A row's offset is stated in 24 bits of 256 bytes.
const OFFSET_LIMIT: u64 = 1 << 32;
/// After 256-byte alignment, a forcible row with less than this left in its 4 KiB page moves to the next page.
const FORCE_REMAINING: u64 = 0x400;
/// A forcible row moves to the next 4 KiB page when this many forcible rows before it did not.
const FORCE_RUN: u32 = 3;
/// A top level over 4 MiB is cut into rows of exactly this many of its tiled bytes.
const TOP_ROW: u64 = 4 << 20;

/// The textures replaced since the pack was opened, each as the zlib rows a save writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replaced(BTreeMap<usize, Vec<Vec<u8>>>);

/// The table a save writes, planned from the table and the replaced rows without reading a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveLayout {
    /// Every row as written, the sentinel last.
    pub rows: Vec<Row>,
    /// Rows where the file starts against the 4 KiB start rule, whose start is carried.
    pub departs: Vec<usize>,
    /// Rows whose start the rule does not decide (the first body, the sentinel, rows of arrays and volumes, every row while an entry is not on disk), carried.
    pub undecided: Vec<usize>,
}

/// What the start rule says of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Start {
    /// The first row of a 2D texture of one or two rows, or the last of one of three or more.
    Forcible,
    /// Any other row of a 2D texture or a cube: 256-byte alignment.
    Aligned,
    Undecided,
}

/// The start rule's state: how many forcible rows since the last that started on 4 KiB did not.
#[derive(Default)]
pub(super) struct Rule(u32);

impl Rule {
    /// Whether a row after data ending at `end` starts on the next 4 KiB page; `None` where the rule does not decide.
    pub(super) fn page(&self, start: Start, end: u64) -> Option<bool> {
        let left = (0x1000 - end.next_multiple_of(0x100) % 0x1000) % 0x1000;
        match start {
            Start::Forcible => Some(left < FORCE_REMAINING || self.0 >= FORCE_RUN),
            Start::Aligned => Some(false),
            Start::Undecided => None,
        }
    }

    pub(super) fn step(&mut self, start: Start, end: u64, offset: u64) {
        let on_page = offset.is_multiple_of(0x1000);
        match start {
            Start::Forcible if on_page => self.0 = 0,
            Start::Forcible => self.0 += 1,
            Start::Undecided if on_page && offset != end.next_multiple_of(0x100) => self.0 = 0,
            _ => {}
        }
    }
}

/// Whether a row at `offset` after data ending at `end` was moved to a 4 KiB page; `None` for a gap that is neither alignment.
fn moved_to_page(end: u64, offset: u64) -> Option<bool> {
    if offset == start_at(false, end) {
        Some(false)
    } else {
        (offset == start_at(true, end)).then_some(true)
    }
}

/// Where a row after data ending at `end` starts: the next 4 KiB page when `page`, else 256-byte alignment.
fn start_at(page: bool, end: u64) -> u64 {
    end.next_multiple_of(if page { 0x1000 } else { 0x100 })
}

fn zlib(bytes: &[u8]) -> Result<Vec<u8>, TtpfError> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).map_err(TtpfError::Write)?;
    encoder.finish().map_err(TtpfError::Write)
}

/// Writes a level's linear elements into `memory` in tiles from the level's place, where `untile` reads them.
pub(in crate::gta6) fn place(
    memory: &mut [u8],
    level: &Level,
    elements: &[u8],
    tile: &Tile,
    element: usize,
) -> bool {
    let mut along = [vec![0usize; tile.width], vec![0usize; tile.height]];
    for (bit, (axis, from)) in tile.bits().enumerate() {
        let Some(spread) = along.get_mut(usize::from(axis - b'x')) else {
            return false;
        };
        for (coordinate, index) in spread.iter_mut().enumerate() {
            *index |= ((coordinate >> from) & 1) << bit;
        }
    }
    let (across, tile_bytes) = (level.width.div_ceil(tile.width), tile.bytes() as usize);
    for (i, block) in elements.chunks_exact(element).enumerate() {
        let (x, y) = (i % level.width, i / level.width);
        let tile_at = (y / tile.height) * across + x / tile.width;
        let index = along[0][x % tile.width] | along[1][y % tile.height];
        let to = level.at as usize + tile_at * tile_bytes + index * element;
        let Some(slot) = memory.get_mut(to..to + element) else {
            return false;
        };
        slot.copy_from_slice(block);
    }
    true
}

impl TexturePack {
    /// The replaced textures, which an undo swaps.
    pub fn replaced_mut(&mut self) -> &mut Replaced {
        &mut self.replaced
    }

    /// Refuses, by its platform's name, an edit of a pack with no rows: every edit is written as rows.
    pub(super) fn edits_rows(&self) -> Result<(), TtpfError> {
        match self.envelope {
            Envelope::Rows => Ok(()),
            Envelope::Plain => Err(TtpfError::EditPlain {
                platform: self.facts.platform.label(),
            }),
        }
    }

    /// The stored bytes a replacement wrote for row `row`.
    // An empty body is a row of the texture the replacement left as the file stores it: no zlib
    // stream is empty.
    pub(super) fn replaced_row(&self, row: usize) -> Option<&[u8]> {
        self.replaced.0.iter().find_map(|(&index, bodies)| {
            let rows = self.rows_of(index)?;
            bodies
                .get(row.checked_sub(rows.start)?)
                .filter(|body| rows.contains(&row) && !body.is_empty())
                .map(Vec::as_slice)
        })
    }

    /// Texture `index` replaced from `image`, encoded into its format with as many mips as its chain holds, which it returns; the texture keeps its key, size, format and rows.
    #[cfg(feature = "authoring")]
    pub fn replace(
        &mut self,
        index: usize,
        image: &crate::codec::texture::RgbaImage,
        options: &crate::authoring::TextureBuildOptions,
    ) -> Result<u32, TtpfError> {
        let layout = match self.envelope {
            Envelope::Rows => self.writable(index)?.layout,
            Envelope::Plain => self.writable_plain(index)?.layout,
        };
        let format = layout.format;
        if !crate::authoring::is_encodable(format.stored()) {
            return Err(TtpfError::ReplaceFormat {
                index,
                format: format.name(),
            });
        }
        let (width, height) = (layout.width, layout.height);
        if (image.width, image.height) != (width, height) {
            return Err(TtpfError::ImageSize {
                index,
                width,
                height,
                got: (image.width, image.height),
            });
        }
        let count = layout.levels.len() as u32;
        let encode = |e: crate::Error| TtpfError::Encode(e.to_string());
        let chain = crate::codec::texture::generate_mip_chain_with(
            width,
            height,
            &image.data,
            Some(count),
            options.filter,
            options.srgb,
            options.alpha,
        )
        .map_err(encode)?;
        let levels = (chain.iter().enumerate())
            .map(|(level, rgba)| {
                let side = |pixels: u32| (pixels >> level).max(1);
                let (w, h) = (side(width), side(height));
                crate::authoring::encode_level(format.stored(), w, h, rgba, options.quality)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(encode)?;
        self.replace_levels(index, &levels)?;
        Ok(count)
    }

    /// Texture `index` replaced by `levels`, its mips top first as linear elements of its format, one for each mip its chain holds.
    pub fn replace_levels(&mut self, index: usize, levels: &[Vec<u8>]) -> Result<(), TtpfError> {
        if self.envelope == Envelope::Plain {
            return self.put_plain(index, levels);
        }
        let plan = self.writable(index)?;
        self.put(index, plan, levels)
    }

    /// Where the levels of 2D texture `index` of a pack with no rows lie in its image, once the pack is whole.
    fn writable_plain(&self, index: usize) -> Result<tiling_prospero::Planned, TtpfError> {
        if !self.is_complete() {
            return Err(TtpfError::NotWhole);
        }
        let count = self.entries.len();
        let entry = (self.entries.get(index)).ok_or(TtpfError::NoSuchTexture { index, count })?;
        if !matches!((entry.dimension, entry.slices), (1 | 5, 1)) {
            return Err(TtpfError::ReplaceShape { index });
        }
        tiling_prospero::plan(self, index)
    }

    /// Tiles `levels` over texture `index`'s bytes of the image, which a save writes in their place: a pack with no rows stores them as they are, so nothing moves.
    fn put_plain(&mut self, index: usize, levels: &[Vec<u8>]) -> Result<(), TtpfError> {
        let tiling_prospero::Planned {
            start, len, layout, ..
        } = self.writable_plain(index)?;
        if levels.len() != layout.levels.len() {
            return Err(super::malformed(format!(
                "texture {index}'s chain holds {} mips, not {}",
                layout.levels.len(),
                levels.len()
            )));
        }
        let element = layout.format.element_bytes();
        // The bytes no level covers stay as the file has them.
        let mut memory = self.image(start, len)?;
        for (i, (level, elements)) in layout.levels.iter().zip(levels).enumerate() {
            if elements.len() != level.width * level.height * element
                || !place(&mut memory, level, elements, &layout.tile, element)
            {
                return Err(super::malformed(format!(
                    "mip {i} of texture {index}'s replacement is not {}x{} elements of its chain",
                    level.width, level.height
                )));
            }
        }
        self.replaced.0.insert(index, vec![memory]);
        Ok(())
    }

    /// Each replaced texture of a pack with no rows: where its bytes start in the image, and the bytes, in the image's order.
    pub(super) fn replaced_plain(&self) -> Vec<(u64, &[u8])> {
        let mut found: Vec<(u64, &[u8])> = (self.replaced.0.iter())
            .filter(|_| self.envelope == Envelope::Plain)
            .filter_map(|(&index, bodies)| {
                Some((self.range_of(index)?.start, bodies.first()?.as_slice()))
            })
            .collect();
        found.sort_by_key(|(start, _)| *start);
        found
    }

    /// Where texture `index`'s replacement goes, once its shape, its tail and its rows are ones the writer covers.
    fn writable(&self, index: usize) -> Result<Plan, TtpfError> {
        self.edits_rows()?;
        if !self.is_complete() {
            return Err(TtpfError::NotWhole);
        }
        let count = self.entries.len();
        let entry = *self
            .entries
            .get(index)
            .ok_or(TtpfError::NoSuchTexture { index, count })?;
        if !matches!((entry.dimension, entry.slices), (1 | 5, 1)) {
            return Err(TtpfError::ReplaceShape { index });
        }
        let plan = self.layout_of(index)?;
        let top = self.top_row(&plan);
        if top + 1 < plan.rows.end && (top..plan.rows.end).any(|row| self.size(row) != TOP_ROW) {
            return Err(TtpfError::SplitTopMip {
                index,
                rows: plan.rows.end - top,
            });
        }
        if plan
            .held
            .is_some_and(|held| plan.layout.levels.len() < held)
        {
            return Err(TtpfError::TailSlotNotMeasured { index });
        }
        let stated = (entry.row_codes, entry.small_rows, entry.align_pages);
        if self.codes_by_rule(index) != Some(stated) {
            return Err(TtpfError::CodesNotByRule { index });
        }
        Ok(plan)
    }

    /// The first row of the texture's top level, which is its last row unless the top level is cut into several.
    fn top_row(&self, plan: &Plan) -> usize {
        self.row_holding(&plan.rows, plan.start, plan.layout.levels[0].from())
    }

    /// Tiles `levels` into texture `index`'s range and keeps each row of it as a zlib stream.
    fn put(&mut self, index: usize, plan: Plan, levels: &[Vec<u8>]) -> Result<(), TtpfError> {
        let Plan {
            rows,
            start,
            len,
            layout,
            ..
        } = plan;
        if levels.len() != layout.levels.len() {
            return Err(super::malformed(format!(
                "texture {index}'s chain holds {} mips, not {}",
                layout.levels.len(),
                levels.len()
            )));
        }
        let element = layout.format.element_bytes();
        let mut memory = vec![0u8; len as usize];
        for (i, (level, elements)) in layout.levels.iter().zip(levels).enumerate() {
            if elements.len() != level.width * level.height * element
                || !place(&mut memory, level, elements, &layout.tile, element)
            {
                return Err(super::malformed(format!(
                    "mip {i} of texture {index}'s replacement is not {}x{} elements of its chain",
                    level.width, level.height
                )));
            }
        }
        let bodies = rows
            .map(|row| {
                let at = (self.rows[row].virtual_address - start) as usize;
                zlib(&memory[at..at + self.size(row) as usize])
            })
            .collect::<Result<_, _>>()?;
        self.replaced.0.insert(index, bodies);
        Ok(())
    }

    /// The row-size codes, `small_rows` and `align_pages` texture `index`'s row sizes give by the rule; `None` for an array or a volume, rows not known, more than four coded rows or a size no code states.
    pub fn codes_by_rule(&self, index: usize) -> Option<([u8; 4], u8, u16)> {
        let entry = self.entries.get(index)?;
        let flat = matches!((entry.dimension, entry.slices), (1 | 5, 1));
        if !flat && entry.dimension != 2 {
            return None;
        }
        let pages: Vec<u64> = self
            .rows_of(index)?
            .map(|row| self.size(row) >> 12)
            .collect();
        let mut codes = [0; 4];
        if entry.dimension == 2 || (pages.len() >= 2 && entry.tile_mode != 0) {
            if pages.len() > codes.len() {
                return None;
            }
            for (code, &pages) in codes.iter_mut().zip(&pages) {
                *code = (1..32).find(|&c| row_code_pages(c).map(u64::from) == Some(pages))?;
            }
        }
        if !flat || entry.tile_mode == 0 || pages.len() < 3 {
            return Some((codes, 0, 0));
        }
        let small = pages.iter().take_while(|&&p| p < 16).count();
        let align = (16 - pages[..small].iter().sum::<u64>() % 16) % 16;
        Some((codes, small as u8, align as u16))
    }

    /// What the start rule says of each row.
    pub(super) fn starts(&self) -> Vec<Start> {
        let mut starts = vec![Start::Undecided; self.rows.len()];
        let Some(ranges) = (0..self.entries.len())
            .map(|k| self.rows_of(k))
            .collect::<Option<Vec<_>>>()
        else {
            return starts;
        };
        for (k, (entry, rows)) in self.entries.iter().zip(ranges).enumerate() {
            let flat = matches!((entry.dimension, entry.slices), (1 | 5, 1));
            if !flat && matches!(entry.dimension, 3 | 5) {
                continue;
            }
            // A top level cut into several rows is forced at the first of them.
            let forcible = if rows.len() <= 2 {
                rows.start
            } else {
                (self.layout_of(k)).map_or(rows.end - 1, |plan| self.top_row(&plan))
            };
            for row in rows {
                starts[row] = if flat && row == forcible {
                    Start::Forcible
                } else {
                    Start::Aligned
                };
            }
        }
        starts
    }

    /// The table a save writes: each row after the one before it at 256-byte alignment or on the next 4 KiB page as the start rule says, and as the file has it where the rule does not decide or the file departs from it in a row no replacement wrote.
    pub fn save_layout(&self) -> Result<SaveLayout, TtpfError> {
        self.edits_rows()?;
        if let Some(moved) = self.moved.as_deref() {
            return self.relaid_layout(moved);
        }
        let n = self.rows.len();
        let starts = self.starts();
        let written: Vec<Option<&[u8]>> = (0..n).map(|row| self.replaced_row(row)).collect();
        let stored = |row: usize| match written[row] {
            Some(bytes) => Ok(bytes.len() as u64),
            None => self.stored_len(row),
        };
        let mut rows = vec![self.rows[0]];
        let (mut shipped, mut rule) = (Rule::default(), Rule::default());
        let (mut departs, mut undecided) = (Vec::new(), Vec::new());
        for i in 1..n {
            let start = starts[i];
            // The rule decides from the second body on; the first, which follows the directory
            // wherever a save leaves it, still counts toward the run.
            let decides = |rule: &Rule, end| (i > 1).then(|| rule.page(start, end)).flatten();
            let shipped_end = self.rows[i - 1].offset + self.stored_len(i - 1)?;
            let offset = self.rows[i].offset;
            let moved = moved_to_page(shipped_end, offset)
                .ok_or(TtpfError::GapNotCarried { row: i - 1 })?;
            let agrees = decides(&shipped, shipped_end).map(|page| start_at(page, shipped_end))
                == Some(offset);
            shipped.step(start, shipped_end, offset);

            let end = rows[i - 1].offset + stored(i - 1)?;
            let page = match decides(&rule, end) {
                Some(page) if agrees || written[i].is_some() => page,
                Some(_) => {
                    departs.push(i);
                    moved
                }
                None => {
                    undecided.push(i);
                    moved
                }
            };
            let offset = start_at(page, end);
            rule.step(start, end, offset);
            // Less than one 4 KiB page, which the pad's 12 bits hold.
            rows[i - 1].pad = (offset - end) as u16;
            let kind = if written[i].is_some() {
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

    /// Writes a pack with no rows to `out` as the image its file is, each replaced texture's bytes in place of the file's.
    fn save_plain(&self, out: &mut impl Write) -> Result<(), TtpfError> {
        use std::io::{Read, Seek, SeekFrom};
        if self.moved.is_some() {
            return self.edits_rows();
        }
        let mut file = File::open(&self.path).map_err(|source| TtpfError::Io {
            path: self.path.clone(),
            source,
        })?;
        let mut at = 0u64;
        for (start, memory) in self.replaced_plain() {
            let before = start.saturating_sub(at);
            std::io::copy(&mut (&mut file).take(before), out).map_err(TtpfError::Write)?;
            out.write_all(memory).map_err(TtpfError::Write)?;
            at = start + memory.len() as u64;
            (file.seek(SeekFrom::Start(at))).map_err(TtpfError::Write)?;
        }
        (std::io::copy(&mut file, out).map(drop)).map_err(TtpfError::Write)
    }

    /// Writes the pack with its replaced textures to `out`, in the form saving it again writes byte for byte.
    pub fn save_to(&self, out: &mut impl Write) -> Result<(), TtpfError> {
        if !self.is_complete() || self.entries.iter().any(|e| !e.on_disk) {
            return Err(TtpfError::NotWhole);
        }
        if self.envelope == Envelope::Plain {
            return self.save_plain(out);
        }
        let layout = self.save_layout()?;
        let mut file = File::open(&self.path).map_err(|source| TtpfError::Io {
            path: self.path.clone(),
            source,
        })?;
        let mut head = read_at(&mut file, &self.path, 0, HEADER_LEN)?;
        relayout::restate_head(&mut head, &layout.rows);
        for row in &layout.rows {
            let page = row.virtual_address >> 12;
            let lo =
                (row.offset >> 8) as u32 | u32::from(row.kind) << 26 | ((page & 3) as u32) << 30;
            let hi = (page >> 2) as u32 | u32::from(row.pad) << 20;
            head.extend(lo.to_le_bytes());
            head.extend(hi.to_le_bytes());
        }
        head.resize(layout.rows[0].offset as usize, 0);
        out.write_all(&head).map_err(TtpfError::Write)?;
        let zeros = [0u8; 0x1000];
        for (row, planned) in layout.rows[..layout.rows.len() - 1].iter().enumerate() {
            let carried;
            let bytes = match self.replaced_row(row) {
                Some(bytes) => bytes,
                None => {
                    carried = self.carried(&mut file, row)?;
                    carried.as_slice()
                }
            };
            out.write_all(bytes).map_err(TtpfError::Write)?;
            out.write_all(&zeros[..usize::from(planned.pad)])
                .map_err(TtpfError::Write)?;
        }
        Ok(())
    }
}

#[path = "ttpf_relayout.rs"]
mod relayout;
#[path = "ttpf_slice.rs"]
mod slice;
pub(super) use relayout::Moved;
pub use relayout::Shape;
