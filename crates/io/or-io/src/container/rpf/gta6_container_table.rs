//! The decrypted RPF9 table the container writer edits: columns of rows and an entry-major index.
//!
//! Some columns' cells point, from the cell, at a slice of a heap after their region's columns. Each
//! region (the main one before the split, the external one after it) holds its columns, then from a
//! 16-byte boundary one heap per such column in column order, each at its element's alignment and
//! packed in row order; the region then ends at the next 16-byte boundary.

use byteorder::{ByteOrder, LE};

use super::super::gta6_archive::{
    GROUP_CHUNK_RECORD, GROUP_CIPHER, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW, GROUP_ENTRY_ROW_20,
    GROUP_NAME32, GROUP_NAME64,
};
use super::super::gta6_dependencies::GROUP_DEPENDENCIES;
use super::super::rpf9_toc::Rpf9Toc;
use super::{bad, round16, round8, Rpf9Piece, START_MASK};
use crate::error::Result;

/// Cell-relative columns of the nested tables, carried as read: four f32 per resource row.
pub(super) const HEAP_D07A: u64 = 0xD07A_10F0_4686_160D;
/// External cell-relative columns whose cell's second word counts the slice's u32 values.
const HEAP_14DC: u64 = 0x14DC_DA11_3546_FB3E;
const HEAP_2455: u64 = 0x2455_5172_AC4A_2929;
const HEAP_2B55: u64 = 0x2B55_1F38_BCA3_8F34;
const HEAP_F87F: u64 = 0xF87F_70B4_5142_A585;
/// An external cell-relative column whose cell's second word counts the slice's u64 values.
const HEAP_14BB: u64 = 0x14BB_5EE5_D9F2_9CA6;
/// An external cell-relative column of variable-length records, each running to the next.
const HEAP_5A9C: u64 = 0x5A9C_2ED4_8D5E_0959;
/// An external 12-byte column: a word, the offset to the slice, then its count of u32 values.
const HEAP_EA2E: u64 = 0xEA2E_6490_2A50_E4B8;

/// How a cell-relative column's cells reach their slices.
#[derive(Clone, Copy)]
struct Heap {
    /// Where in the cell the u32 offset from the cell's start to its slice sits.
    field: usize,
    /// The slice's length from its cell; `None` where a slice runs to the next one.
    len: Option<fn(&[u8]) -> usize>,
    /// The boundary the heap starts on.
    align: usize,
}

fn word(cell: &[u8], at: usize) -> usize {
    LE::read_u32(&cell[at..]) as usize
}

fn chunk_descriptors(cell: &[u8]) -> usize {
    8 * (word(cell, 4) & 0xFFFF)
}

fn dependency_indices(cell: &[u8]) -> usize {
    (word(cell, 4) & 0x1FFF_FFFF).saturating_mul(word(cell, 4) >> 29)
}

fn u32_count(cell: &[u8]) -> usize {
    word(cell, 4).saturating_mul(4)
}

fn u64_count(cell: &[u8]) -> usize {
    word(cell, 4).saturating_mul(8)
}

fn u32_count_at_8(cell: &[u8]) -> usize {
    word(cell, 8).saturating_mul(4)
}

/// The heap a column of `hash` and `width` keeps its slices in, `None` for a column of plain cells.
fn heap(hash: u64, width: usize) -> Option<Heap> {
    type Len = Option<fn(&[u8]) -> usize>;
    let (field, len, align): (usize, Len, usize) = match (hash, width) {
        (GROUP_CHUNK_RECORD, 8) => (0, Some(chunk_descriptors), 8),
        (GROUP_DEPENDENCIES, 8) => (0, Some(dependency_indices), 1),
        (HEAP_D07A | HEAP_14DC | HEAP_2455 | HEAP_2B55 | HEAP_F87F, 8) => (0, Some(u32_count), 4),
        (HEAP_14BB, 8) => (0, Some(u64_count), 8),
        (HEAP_5A9C, 8) => (0, None, 4),
        (HEAP_EA2E, 12) => (4, Some(u32_count_at_8), 4),
        _ => return None,
    };
    Some(Heap { field, len, align })
}

/// A cell of one column, with the heap slice it points at and the new body a row takes, if any.
pub(super) struct Row {
    pub(super) cell: Vec<u8>,
    pub(super) slice: Vec<u8>,
    /// Where `slice` sat in the table read, for a rewrite that moves nothing.
    pub(super) slice_at: usize,
    pub(super) body: Option<Fresh>,
}

/// A body the writer places: its pieces, its length, and the extent it may reuse when it fits.
pub(super) struct Fresh {
    pub(super) pieces: Vec<Rpf9Piece>,
    pub(super) len: u64,
    pub(super) old: Option<(u64, u64)>,
}

pub(super) struct Column {
    pub(super) hash: u64,
    pub(super) width: usize,
    pub(super) external: bool,
    pub(super) data_start: usize,
    pub(super) rows: Vec<Row>,
}

/// A decrypted table as columns of rows and an entry-major index, written back in place or laid
/// out anew.
pub(super) struct Table {
    pub(super) bytes: Vec<u8>,
    pub(super) groups_start: usize,
    pub(super) columns: Vec<Column>,
    /// Each entry's raw index value per column, in directory order.
    pub(super) index: Vec<Vec<u16>>,
    pub(super) width: usize,
    /// Whether an edit added or dropped a row or an entry, so the table must be laid out anew.
    pub(super) relaid: bool,
    /// Whether laying out the unedited table reproduces it, which a re-laid table requires.
    pub(super) faithful: bool,
}

/// The index value that marks an entry with no row in a column.
fn absent(width: usize) -> u16 {
    if width == 1 {
        0xFF
    } else {
        0xFFFF
    }
}

/// Read each cell-relative column's slices, the main region's in `[0, split)` and the external
/// region's in the rest of `b`; a slice of no stated length runs to the next slice of its region.
fn read_slices(columns: &mut [Column], b: &[u8], split: usize) -> Result<()> {
    let start = |col: &Column, r: usize, h: Heap| {
        (col.data_start + r * col.width).saturating_add(word(&col.rows[r].cell, h.field))
    };
    for external in [false, true] {
        let end = if external { b.len() } else { split };
        let heaps: Vec<(usize, Heap)> = (columns.iter().enumerate())
            .filter(|(_, col)| col.external == external)
            .filter_map(|(c, col)| heap(col.hash, col.width).map(|h| (c, h)))
            .collect();
        for (k, &(c, h)) in heaps.iter().enumerate() {
            let next = heaps[k + 1..]
                .iter()
                .find(|(d, _)| !columns[*d].rows.is_empty())
                .map_or(end, |&(d, hd)| start(&columns[d], 0, hd));
            let col = &columns[c];
            let mut slices = Vec::with_capacity(col.rows.len());
            for r in 0..col.rows.len() {
                let from = start(col, r, h);
                let to = match h.len {
                    Some(len) => from.saturating_add(len(&col.rows[r].cell)),
                    None if r + 1 < col.rows.len() => start(col, r + 1, h),
                    None => next,
                };
                let slice = b.get(from..to).filter(|_| to <= end).ok_or_else(|| {
                    bad(format!(
                        "a cell of column {:#018x} points outside its region",
                        col.hash
                    ))
                })?;
                slices.push((slice.to_vec(), from));
            }
            for (row, (slice, at)) in columns[c].rows.iter_mut().zip(slices) {
                row.slice = slice;
                row.slice_at = at;
            }
        }
    }
    Ok(())
}

impl Table {
    pub(super) fn from_toc(toc: &Rpf9Toc) -> Result<Self> {
        let b = &toc.bytes;
        if b.len() < 0x40 {
            return Err(bad("the table is shorter than its header"));
        }
        let n = LE::read_u16(&b[0x1A..]) as usize;
        let m = LE::read_u32(&b[0x1C..]) as usize;
        let split = LE::read_u32(&b[0x08..]) as usize;
        let entries = LE::read_u32(&b[0x24..]) as usize;
        let width = b[0x2C] as usize;
        if !(1..=2).contains(&width) {
            return Err(bad(format!("an index width of {width}")));
        }
        let groups_start = round16((0x40 + round16(8 * n as u64) as usize + m) as u64) as usize;
        let mut columns = Vec::with_capacity(toc.groups.len());
        for g in &toc.groups {
            let w = usize::from(g.width);
            let mut rows = Vec::with_capacity(g.count as usize);
            for r in 0..g.count as usize {
                let at = g.data.start + r * w;
                rows.push(Row {
                    cell: b[at..at + w].to_vec(),
                    slice: Vec::new(),
                    slice_at: 0,
                    body: None,
                });
            }
            columns.push(Column {
                hash: g.name_hash,
                width: w,
                external: g.external,
                data_start: g.data.start,
                rows,
            });
        }
        read_slices(&mut columns, b, split)?;
        let mut table = Self {
            bytes: b.clone(),
            groups_start,
            columns,
            index: Vec::with_capacity(entries),
            width,
            relaid: false,
            faithful: false,
        };
        for e in 0..entries {
            let mut row = Vec::with_capacity(table.columns.len());
            for c in 0..table.columns.len() {
                let at = table.index_at(split, width, e, c);
                let raw = b
                    .get(at..at + width)
                    .ok_or_else(|| bad("the entry index runs past the table"))?;
                row.push(if width == 1 {
                    u16::from(raw[0])
                } else {
                    LE::read_u16(raw)
                });
            }
            table.index.push(row);
        }
        table.faithful = table.layout() == table.bytes;
        Ok(table)
    }

    /// Where entry `e`'s index value for column `c` sits in a table laid out with external index at
    /// `split` and index width `width`: entry-major, one slot per column of its kind.
    pub(super) fn index_at(&self, split: usize, width: usize, e: usize, c: usize) -> usize {
        let external = self.columns[c].external;
        let kind = self
            .columns
            .iter()
            .filter(|k| k.external == external)
            .count();
        let slot = self.columns[..c]
            .iter()
            .filter(|k| k.external == external)
            .count();
        let base = if external {
            split
        } else {
            self.groups_start + 24 * self.columns.len()
        };
        base + (e * kind + slot) * width
    }

    pub(super) fn col(&self, hash: u64, width: usize) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.hash == hash && c.width == width)
    }

    pub(super) fn row_of(&self, e: usize, c: usize) -> Option<usize> {
        let v = usize::from(self.index[e][c]);
        (v < self.columns[c].rows.len()).then_some(v)
    }

    pub(super) fn cell(&self, e: usize, c: usize) -> Option<&[u8]> {
        self.row_of(e, c).map(|r| &self.columns[c].rows[r].cell[..])
    }

    /// Set entry `e`'s cell in column `c` (`None` to give it no row), overwriting its row when no
    /// other entry shares it and adding one otherwise; whether the row was overwritten in place.
    pub(super) fn set_cell(
        &mut self,
        e: usize,
        c: usize,
        cell: Option<Vec<u8>>,
        slice: Vec<u8>,
    ) -> bool {
        let current = self.row_of(e, c);
        let shared = current.is_some_and(|r| {
            (0..self.index.len())
                .filter(|&o| self.row_of(o, c) == Some(r))
                .count()
                > 1
        });
        let in_place = match (cell, current) {
            (None, _) => {
                // Past every row at any width, where a width-1 sentinel names a row past 255.
                self.index[e][c] = u16::MAX;
                false
            }
            (Some(cell), Some(r)) if !shared => {
                let row = &mut self.columns[c].rows[r];
                if row.slice.len() != slice.len() {
                    self.relaid = true;
                }
                row.cell = cell;
                row.slice = slice;
                true
            }
            (Some(cell), _) => {
                let rows = &mut self.columns[c].rows;
                rows.push(Row {
                    cell,
                    slice,
                    slice_at: 0,
                    body: None,
                });
                self.index[e][c] = (rows.len() - 1) as u16;
                self.relaid = true;
                false
            }
        };
        self.gc();
        in_place
    }

    /// Set entry `e`'s entry row to `cell` and give that row the new body `pieces` of `len` bytes,
    /// which may reuse the extent `old` when the row was its own.
    pub(super) fn set_body(
        &mut self,
        e: usize,
        c: usize,
        cell: Vec<u8>,
        pieces: Vec<Rpf9Piece>,
        len: u64,
        old: (u64, u64),
    ) {
        let in_place = self.set_cell(e, c, Some(cell), Vec::new());
        if let Some(r) = self.row_of(e, c) {
            self.columns[c].rows[r].body = Some(Fresh {
                pieces,
                len,
                old: in_place.then_some(old),
            });
        }
    }

    /// Set entry `e`'s chunk record: `u1` and the descriptors, keeping the cell's offset word, which
    /// a re-laid table recomputes.
    pub(super) fn set_chunks(&mut self, e: usize, k: usize, u1: u32, words: &[u64]) {
        let mut cell = self
            .cell(e, k)
            .map(<[u8]>::to_vec)
            .unwrap_or_else(|| vec![0; 8]);
        LE::write_u32(&mut cell[4..], u1);
        let slice = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.set_cell(e, k, Some(cell), slice);
    }

    /// Write a changed entry's `0xb6f5` content hash as zero, where it has one.
    pub(super) fn zero_hash(&mut self, e: usize) {
        if let Some(c) = self.col(GROUP_CONTENT_HASH, 8) {
            if self.row_of(e, c).is_some() {
                self.set_cell(e, c, Some(vec![0; 8]), Vec::new());
            }
        }
    }

    pub(super) fn remove(&mut self, e: usize) {
        self.index.remove(e);
        self.relaid = true;
        self.gc();
    }

    /// Drop every row no entry references, renumbering the index.
    pub(super) fn gc(&mut self) {
        for (c, col) in self.columns.iter_mut().enumerate() {
            let len = col.rows.len();
            let mut used = vec![false; len];
            for row in &self.index {
                if let Some(u) = used.get_mut(usize::from(row[c])) {
                    *u = true;
                }
            }
            if used.iter().all(|&u| u) {
                continue;
            }
            let mut map = vec![0u16; len];
            let mut next = 0u16;
            for (to, &u) in map.iter_mut().zip(&used) {
                if u {
                    *to = next;
                    next += 1;
                }
            }
            let mut r = 0;
            col.rows.retain(|_| {
                r += 1;
                used[r - 1]
            });
            for row in &mut self.index {
                if let Some(&to) = map.get(usize::from(row[c])) {
                    row[c] = to;
                }
            }
            self.relaid = true;
        }
    }

    /// The rows of the 24- and 20-byte entry-row columns some entry references: one body each.
    pub(super) fn bodies(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for c in [
            self.col(GROUP_ENTRY_ROW, 24),
            self.col(GROUP_ENTRY_ROW_20, 20),
        ]
        .into_iter()
        .flatten()
        {
            let mut used = vec![false; self.columns[c].rows.len()];
            for e in 0..self.index.len() {
                if let Some(r) = self.row_of(e, c) {
                    used[r] = true;
                }
            }
            out.extend(
                used.iter()
                    .enumerate()
                    .filter(|&(_, &u)| u)
                    .map(|(r, _)| (c, r)),
            );
        }
        out
    }

    /// The start and stored extent, in bytes, an entry row states.
    pub(super) fn extent(&self, c: usize, r: usize) -> (u64, u64) {
        let cell = &self.columns[c].rows[r].cell;
        if self.columns[c].hash == GROUP_ENTRY_ROW {
            (
                (LE::read_u64(cell) & START_MASK) * 16,
                LE::read_u64(&cell[8..]).saturating_mul(16),
            )
        } else {
            (
                u64::from(LE::read_u32(&cell[12..])) * 16,
                u64::from(LE::read_u32(&cell[8..])) * 16,
            )
        }
    }

    pub(super) fn set_start(&mut self, c: usize, r: usize, start: u64) -> Result<()> {
        let units = start / 16;
        let row24 = self.columns[c].hash == GROUP_ENTRY_ROW;
        let cell = &mut self.columns[c].rows[r].cell;
        if row24 {
            if units > START_MASK {
                return Err(bad(format!("a body start of {start:#x} overflows its row")));
            }
            let a = LE::read_u64(cell);
            LE::write_u64(cell, (a & !START_MASK) | units);
        } else {
            let units = u32::try_from(units)
                .map_err(|_| bad(format!("a body start of {start:#x} overflows its row")))?;
            LE::write_u32(&mut cell[12..], units);
        }
        Ok(())
    }

    /// The plaintext table: rewritten in place, or laid out anew after a row or an entry changed.
    pub(super) fn emit(&self) -> Result<Vec<u8>> {
        if !self.relaid {
            return Ok(self.patch());
        }
        if !self.faithful {
            let known = [
                GROUP_NAME64,
                GROUP_NAME32,
                GROUP_ENTRY_ROW,
                GROUP_ENTRY_ROW_20,
                GROUP_CIPHER,
                GROUP_CONTENT_HASH,
                GROUP_CHUNK_RECORD,
            ];
            let unknown: Vec<String> = self
                .columns
                .iter()
                .filter(|c| !known.contains(&c.hash) && heap(c.hash, c.width).is_none())
                .map(|c| format!("{:#018x}", c.hash))
                .collect();
            return Err(bad(format!(
                "the table holds data past its columns that this writer cannot lay out again \
                 (columns {}), so it takes no edit that adds or drops a row or an entry",
                unknown.join(", ")
            )));
        }
        if self.index.len() > 0xFFFF {
            return Err(bad(format!(
                "{} entries, more than a 2-byte index holds",
                self.index.len()
            )));
        }
        Ok(self.layout())
    }

    /// The table read, with every row, chunk slice and index value written back where it was.
    pub(super) fn patch(&self) -> Vec<u8> {
        let mut out = self.bytes.clone();
        for col in &self.columns {
            for (r, row) in col.rows.iter().enumerate() {
                let at = col.data_start + r * col.width;
                out[at..at + col.width].copy_from_slice(&row.cell);
                out[row.slice_at..row.slice_at + row.slice.len()].copy_from_slice(&row.slice);
            }
        }
        let split = LE::read_u32(&self.bytes[0x08..]) as usize;
        for (e, row) in self.index.iter().enumerate() {
            for (c, &v) in row.iter().enumerate() {
                let at = self.index_at(split, self.width, e, c);
                if self.width == 1 {
                    out[at] = v as u8;
                } else {
                    LE::write_u16(&mut out[at..], v);
                }
            }
        }
        out
    }

    /// The table laid out from its columns: directory and main index, the main region, then at the
    /// 16-aligned split the external index and the external region.
    pub(super) fn layout(&self) -> Vec<u8> {
        let entries = self.index.len();
        let width = if entries <= 0xFF { 1 } else { 2 };
        let main = self.columns.iter().filter(|c| !c.external).count();
        let mut out = self.bytes[..self.groups_start].to_vec();
        out.resize(
            round8(self.groups_start + 24 * self.columns.len() + main * entries * width),
            0,
        );
        let mut offsets = vec![0usize; self.columns.len()];
        self.place(&mut out, &mut offsets, false);
        let split = round16(out.len() as u64) as usize;
        let external = self.columns.len() - main;
        out.resize(round8(split + external * entries * width), 0);
        self.place(&mut out, &mut offsets, true);
        out.resize(round16(out.len() as u64) as usize, 0);

        for (i, col) in self.columns.iter().enumerate() {
            let at = self.groups_start + 24 * i;
            LE::write_u64(&mut out[at..], col.hash);
            LE::write_u32(&mut out[at + 8..], col.rows.len() as u32);
            LE::write_u32(&mut out[at + 12..], offsets[i] as u32);
            LE::write_u32(
                &mut out[at + 16..],
                col.width as u32 | (u32::from(col.external) << 16),
            );
            LE::write_u32(&mut out[at + 20..], 0);
        }
        for (e, row) in self.index.iter().enumerate() {
            for (c, &v) in row.iter().enumerate() {
                let at = self.index_at(split, width, e, c);
                let v = if usize::from(v) < self.columns[c].rows.len() {
                    v
                } else {
                    absent(width)
                };
                if width == 1 {
                    out[at] = v as u8;
                } else {
                    LE::write_u16(&mut out[at..], v);
                }
            }
        }
        let toc_end = out.len() as u32;
        LE::write_u32(&mut out[0x04..], toc_end);
        LE::write_u32(&mut out[0x08..], split as u32);
        LE::write_u32(&mut out[0x24..], entries as u32);
        out[0x2C] = width as u8;
        out
    }

    /// Append the main (or external) region to `out`: its columns' rows, each column 8-aligned from
    /// the region's start (each one's offset recorded), then from a 16-byte boundary the heaps.
    pub(super) fn place(&self, out: &mut Vec<u8>, offsets: &mut [usize], external: bool) {
        let start = out.len();
        let mut cells = vec![0usize; self.columns.len()];
        for (c, col) in self.columns.iter().enumerate() {
            if col.external == external {
                out.resize(start + round8(out.len() - start), 0);
                offsets[c] = out.len() - start;
                cells[c] = out.len();
                for row in &col.rows {
                    out.extend_from_slice(&row.cell);
                }
            }
        }
        out.resize(round16(out.len() as u64) as usize, 0);
        for (c, col) in self.columns.iter().enumerate() {
            let Some(h) = heap(col.hash, col.width).filter(|_| col.external == external) else {
                continue;
            };
            out.resize(out.len().next_multiple_of(h.align), 0);
            for (r, row) in col.rows.iter().enumerate() {
                let cell = cells[c] + r * col.width;
                let u0 = (out.len() - cell) as u32;
                LE::write_u32(&mut out[cell + h.field..], u0);
                out.extend_from_slice(&row.slice);
            }
        }
    }
}
