//! GTA VI zlib bodies a scan names by their head: GTA V's height map stored little-endian, the same
//! grid with one array, a two-plane span grid, and the heads of a MoVE network at version 0x13
//! and of an `EDUP` body.

use crate::error::{IoError, Result};
use crate::height_map::HeightMap;

/// What a zlib body's first bytes identify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OtherBody {
    /// GTA V's height map written little-endian: tag bytes `PAMH`, read by [`Gta6HeightMap`].
    HeightMap,
    /// The height map's grid with one array of cells: tag bytes `MHIU`, read by [`Gta6HeightGrid`].
    HeightGrid,
    /// A 128 x 128 tile of two byte planes over row spans, read by [`SpanGrid`].
    SpanGrid,
    /// A MoVE network at version 0x13, whose head [`MoveNetworkHead`] reads.
    MoveNetwork,
    /// A Scaleform movie, which `gfx::GfxMovie` reads.
    Gfx,
    /// `EDUP`, a count, then bytes of high entropy: [`EdupHead`] reads the head, the payload is not read.
    Edup,
    /// An offset table of object trees with no block map, read by `read_object_trees`.
    ObjectTrees,
}

impl OtherBody {
    /// The body `head` starts, of `inflated` bytes in all; `None` where no measured layout names it.
    pub fn of(head: &[u8], inflated: u64) -> Option<Self> {
        let u16_at = |at: usize| {
            head.get(at..at + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
        };
        let grid = [4, 6, 12, 14]
            .iter()
            .all(|at| u16_at(*at) == Some(SPAN_GRID_SIDE));
        // A movie states its length at +4, which sets it apart from text that starts `GFX`.
        let stated = head
            .get(4..8)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let stated_length = stated.is_some_and(|n| u64::from(n) == inflated);
        if starts_offset_table(head) {
            return Some(Self::ObjectTrees);
        }
        Some(match head.get(..4)? {
            b"PAMH" => Self::HeightMap,
            b"MHIU" => Self::HeightGrid,
            &[1, 0, 1, 2] if grid => Self::SpanGrid,
            b"MoVE" => Self::MoveNetwork,
            &[b'G', b'F', b'X', _] if stated_length => Self::Gfx,
            b"EDUP" => Self::Edup,
            _ => return None,
        })
    }

    /// The scan's word for the body.
    pub fn label(self) -> &'static str {
        match self {
            Self::HeightMap => "zlib height map",
            Self::HeightGrid => "zlib height grid",
            Self::SpanGrid => "zlib span grid",
            Self::MoveNetwork => "zlib move network",
            Self::Gfx => "zlib gfx",
            Self::Edup => "zlib edup",
            Self::ObjectTrees => "zlib object trees",
        }
    }
}

/// Zero to 0x28 but a count at 0x14, then that many offsets whose first lies just past them.
fn starts_offset_table(head: &[u8]) -> bool {
    let Some(head) = head.get(..0x30) else {
        return false;
    };
    let count = u64::from(u32::from_le_bytes([
        head[0x14], head[0x15], head[0x16], head[0x17],
    ]));
    let first = u64::from_le_bytes(std::array::from_fn(|k| head[0x28 + k]));
    let zero = head[..0x14]
        .iter()
        .chain(&head[0x18..0x28])
        .all(|b| *b == 0);
    zero && count != 0 && first == 0x28 + 8 * count
}

const HEADER_LEN: usize = 44;
const SPAN_LEN: usize = 8;
const SPAN_GRID_SIDE: u16 = 128;
const SPAN_GRID_HEAD: usize = 32;
const MOVE_VERSION: [u8; 16] = [0x13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// The bytes a MoVE body's length word leaves out, besides five per list D entry.
const MOVE_FIXED: u64 = 0x2E;

/// A GTA VI height map: GTA V's grid stored little-endian, every row a full span, no water mask.
#[derive(Debug, Clone, PartialEq)]
pub struct Gta6HeightMap {
    /// The grid, with GTA V's `RLE_DATA` flag and no water mask.
    pub grid: HeightMap,
    /// Bytes 5 to 11, which hold neither GTA V's cell size, pad nor flags: carried as stored.
    pub carried: [u8; 7],
}

impl Gta6HeightMap {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "GTA VI height map";
        let header = bytes
            .get(..HEADER_LEN)
            .ok_or_else(|| short(WHAT, bytes.len()))?;
        if header[..4] != *b"PAMH" || header[4] != 1 {
            return Err(refuse(WHAT, "the head is not `PAMH` and version 1".into()));
        }
        let (cols, rows) = (u16_at(header, 12), u16_at(header, 14));
        let cells = usize::from(cols) * usize::from(rows);
        let data_len = u64::from(u32_at(header, 40));
        let spans = usize::from(rows) * SPAN_LEN;
        if data_len != (spans + 2 * cells) as u64
            || bytes.len() as u64 != HEADER_LEN as u64 + data_len
        {
            return Err(refuse(
                WHAT,
                format!(
                    "{} bytes with {data_len} of data is not {rows} row spans and two halves of {cols} x {rows} cells",
                    bytes.len()
                ),
            ));
        }
        for row in 0..usize::from(rows) {
            let span = &bytes[HEADER_LEN + row * SPAN_LEN..][..SPAN_LEN];
            let (start, count, offset) = (u16_at(span, 0), u16_at(span, 2), i32_at(span, 4));
            if start != 0 || count != cols || i64::from(offset) != (row * usize::from(cols)) as i64
            {
                return Err(refuse(
                    WHAT,
                    format!("row {row}'s span ({start}, {count}, {offset}) is not the whole row"),
                ));
            }
        }
        let (max, min) = bytes[HEADER_LEN + spans..].split_at(cells);
        let mut carried = [0; 7];
        carried.copy_from_slice(&header[5..12]);
        Ok(Self {
            grid: HeightMap {
                version: header[4],
                flags: HeightMap::RLE_DATA,
                num_cols: cols,
                num_rows: rows,
                bounds_min: f32s(header, 16),
                bounds_max: f32s(header, 28),
                max: max.to_vec(),
                min: min.to_vec(),
                water_mask: Vec::new(),
            },
            carried,
        })
    }

    /// Writes the 44-byte header with the carried bytes, a full span per row, then both halves.
    pub fn save(&self) -> Result<Vec<u8>> {
        const WHAT: &str = "GTA VI height map";
        let grid = &self.grid;
        let (cols, rows) = (usize::from(grid.num_cols), usize::from(grid.num_rows));
        if grid.max.len() != cols * rows
            || grid.min.len() != cols * rows
            || !grid.water_mask.is_empty()
        {
            return Err(refuse(
                WHAT,
                format!(
                    "{} maximum and {} minimum heights and a water mask of {} bytes for a {cols} x {rows} grid that holds no mask",
                    grid.max.len(),
                    grid.min.len(),
                    grid.water_mask.len()
                ),
            ));
        }
        let data_len = u32::try_from(rows * SPAN_LEN + 2 * cols * rows)
            .map_err(|_| refuse(WHAT, "the heights pass u32 bytes".into()))?;
        let mut out = Vec::with_capacity(HEADER_LEN + data_len as usize);
        out.extend_from_slice(b"PAMH");
        out.push(grid.version);
        out.extend_from_slice(&self.carried);
        out.extend_from_slice(&grid.num_cols.to_le_bytes());
        out.extend_from_slice(&grid.num_rows.to_le_bytes());
        for v in grid.bounds_min.iter().chain(&grid.bounds_max) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&data_len.to_le_bytes());
        for row in 0..rows {
            let offset = i32::try_from(row * cols)
                .map_err(|_| refuse(WHAT, format!("row {row}'s data offset passes i32")))?;
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&grid.num_cols.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(&grid.max);
        out.extend_from_slice(&grid.min);
        Ok(out)
    }

    /// The square tile's side and its column and row from the world origin, in tiles of that side.
    pub fn tile(&self) -> Option<(f32, [i32; 2])> {
        let (min, max) = (self.grid.bounds_min, self.grid.bounds_max);
        let side = max[0] - min[0];
        if !side.is_finite() || side <= 0.0 || max[1] - min[1] != side {
            return None;
        }
        let at = |axis: usize| {
            let i = min[axis] / side;
            (i.fract() == 0.0 && i.abs() < i32::MAX as f32).then_some(i as i32)
        };
        Some((side, [at(0)?, at(1)?]))
    }
}

/// A row of a [`Gta6HeightGrid`]: its first column and its cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRow {
    pub start: u16,
    pub cells: Vec<u8>,
}

/// The height map's grid with one byte array, its rows stored as spans packed in row order.
#[derive(Debug, Clone, PartialEq)]
pub struct Gta6HeightGrid {
    pub num_cols: u16,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Byte 0x28, which a cell outside every span reads as, then bytes 0x29..0x2B: carried.
    pub carried: u32,
    /// One per row, from `bounds_min`'s Y; an empty row has no cells.
    pub rows: Vec<GridRow>,
}

impl Gta6HeightGrid {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "GTA VI height grid";
        let header = bytes
            .get(..HEADER_LEN)
            .ok_or_else(|| short(WHAT, bytes.len()))?;
        if header[..4] != *b"MHIU" || header[4..6] != [1, 0] || u16_at(header, 0x0A) != 0 {
            return Err(refuse(
                WHAT,
                "the head is not `MHIU`, version 1, then zero bytes".into(),
            ));
        }
        let (cols, rows) = (u16_at(header, 6), usize::from(u16_at(header, 8)));
        let data_len = u64::from(u32_at(header, 0x24));
        let spans = rows * SPAN_LEN;
        if bytes.len() as u64 != HEADER_LEN as u64 + data_len || (spans as u64) > data_len {
            return Err(refuse(
                WHAT,
                format!("{} bytes do not hold the {data_len} data bytes the header states, or its {rows} row spans", bytes.len()),
            ));
        }
        let cells = &bytes[HEADER_LEN + spans..];
        let mut out = Vec::with_capacity(rows);
        let mut packed = 0usize;
        for row in 0..rows {
            let span = &bytes[HEADER_LEN + row * SPAN_LEN..][..SPAN_LEN];
            let (start, count, offset) = (
                u16_at(span, 0),
                usize::from(u16_at(span, 2)),
                i32_at(span, 4),
            );
            let fits = if count == 0 {
                start == 0 && offset == 0
            } else {
                usize::from(start) + count <= usize::from(cols)
                    && i64::from(offset) + i64::from(start) == packed as i64
                    && packed + count <= cells.len()
            };
            if !fits {
                return Err(refuse(
                    WHAT,
                    format!("row {row}'s span ({start}, {count}, {offset}) is not packed after the rows before it inside the {cols}-column grid"),
                ));
            }
            out.push(GridRow {
                start,
                cells: cells[packed..packed + count].to_vec(),
            });
            packed += count;
        }
        if packed != cells.len() {
            return Err(refuse(
                WHAT,
                format!(
                    "the spans cover {packed} cells and {} follow them",
                    cells.len()
                ),
            ));
        }
        Ok(Self {
            num_cols: cols,
            bounds_min: f32s(header, 0x0C),
            bounds_max: f32s(header, 0x18),
            carried: u32_at(header, 0x28),
            rows: out,
        })
    }

    /// Writes the header, each row's span packed after the rows before it, then the cells.
    pub fn save(&self) -> Result<Vec<u8>> {
        const WHAT: &str = "GTA VI height grid";
        let rows = u16::try_from(self.rows.len())
            .map_err(|_| refuse(WHAT, format!("{} rows pass u16", self.rows.len())))?;
        let mut spans = Vec::with_capacity(self.rows.len() * SPAN_LEN);
        let mut cells = Vec::new();
        for (i, row) in self.rows.iter().enumerate() {
            let count = u16::try_from(row.cells.len()).ok().filter(|n| {
                *n == 0 || usize::from(row.start) + usize::from(*n) <= usize::from(self.num_cols)
            });
            let offset = i32::try_from(cells.len() as i64 - i64::from(row.start)).ok();
            let (Some(count), Some(offset)) = (count, offset) else {
                return Err(refuse(
                    WHAT,
                    format!("row {i} leaves the {}-column grid", self.num_cols),
                ));
            };
            let (start, offset) = if count == 0 {
                (0, 0)
            } else {
                (row.start, offset)
            };
            spans.extend_from_slice(&start.to_le_bytes());
            spans.extend_from_slice(&count.to_le_bytes());
            spans.extend_from_slice(&offset.to_le_bytes());
            cells.extend_from_slice(&row.cells);
        }
        let data_len = u32::try_from(spans.len() + cells.len())
            .map_err(|_| refuse(WHAT, "the cells pass u32 bytes".into()))?;
        let mut out = Vec::with_capacity(HEADER_LEN + data_len as usize);
        out.extend_from_slice(b"MHIU");
        out.extend_from_slice(&[1, 0]);
        out.extend_from_slice(&self.num_cols.to_le_bytes());
        out.extend_from_slice(&rows.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        for v in self.bounds_min.iter().chain(&self.bounds_max) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&data_len.to_le_bytes());
        out.extend_from_slice(&self.carried.to_le_bytes());
        out.extend_from_slice(&spans);
        out.extend_from_slice(&cells);
        Ok(out)
    }

    /// The grid as GTA V's height map, its one plane in both halves and byte 0x28 where no span is.
    pub fn height_map(&self) -> Result<HeightMap> {
        const WHAT: &str = "GTA VI height grid";
        let cols = usize::from(self.num_cols);
        let rows = u16::try_from(self.rows.len())
            .map_err(|_| refuse(WHAT, format!("{} rows pass u16", self.rows.len())))?;
        let mut cells = vec![self.carried.to_le_bytes()[0]; cols * self.rows.len()];
        for (i, row) in self.rows.iter().enumerate() {
            let from = usize::from(row.start);
            if from + row.cells.len() > cols {
                return Err(refuse(
                    WHAT,
                    format!("row {i} leaves the {cols}-column grid"),
                ));
            }
            let at = i * cols + from;
            cells[at..at + row.cells.len()].copy_from_slice(&row.cells);
        }
        Ok(HeightMap {
            version: 1,
            flags: 0,
            num_cols: self.num_cols,
            num_rows: rows,
            bounds_min: self.bounds_min,
            bounds_max: self.bounds_max,
            max: cells.clone(),
            min: cells,
            water_mask: Vec::new(),
        })
    }

    /// Takes the cells and bounds of `map`, a grid [`Self::height_map`] made, growing a row's span over a cell changed outside it.
    pub fn set_height_map(&mut self, map: &HeightMap) -> Result<()> {
        const WHAT: &str = "GTA VI height grid";
        let cols = usize::from(self.num_cols);
        let cells = cols * self.rows.len();
        if map.num_cols != self.num_cols
            || usize::from(map.num_rows) != self.rows.len()
            || map.max.len() != cells
            || map.min.len() != cells
            || !map.water_mask.is_empty()
        {
            return Err(refuse(
                WHAT,
                format!(
                    "a {} x {} grid of {} maximum and {} minimum heights and a {}-byte water mask is not this {cols} x {} grid of one array with no mask",
                    map.num_cols,
                    map.num_rows,
                    map.max.len(),
                    map.min.len(),
                    map.water_mask.len(),
                    self.rows.len()
                ),
            ));
        }
        if let Some(at) = (0..cells).find(|&i| map.max[i] != map.min[i]) {
            return Err(refuse(
                WHAT,
                format!(
                    "cell {at} has a maximum of {} and a minimum of {}, and this grid holds one height a cell",
                    map.max[at],
                    map.min[at]
                ),
            ));
        }
        let fill = self.carried.to_le_bytes()[0];
        for (i, row) in self.rows.iter_mut().enumerate() {
            let line = &map.max[i * cols..(i + 1) * cols];
            let (start, end) = (
                usize::from(row.start),
                usize::from(row.start) + row.cells.len(),
            );
            if end > cols {
                return Err(refuse(
                    WHAT,
                    format!("row {i} leaves the {cols}-column grid"),
                ));
            }
            let mut span = (!row.cells.is_empty()).then_some((start, end));
            for c in (0..cols).filter(|&c| (c < start || c >= end) && line[c] != fill) {
                span = Some(span.map_or((c, c + 1), |(a, b)| (a.min(c), b.max(c + 1))));
            }
            if let Some((from, to)) = span {
                row.start = u16::try_from(from)
                    .map_err(|_| refuse(WHAT, format!("row {i}'s column {from} passes u16")))?;
                row.cells = line[from..to].to_vec();
            }
        }
        self.bounds_min = map.bounds_min;
        self.bounds_max = map.bounds_max;
        Ok(())
    }
}

/// One span of a [`SpanGrid`] row: columns `start..end` at data index `offset + column`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSpan {
    pub start: u16,
    pub end: u16,
    pub offset: i32,
}

/// A 128 x 128 tile of two byte planes over land, two spans a row; what the planes hold is not known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanGrid {
    /// The word at 0x08, 0 to 3.
    pub kind: u32,
    /// The first row the spans cover; they cover `first_row..first_row + spans.len()`.
    pub first_row: u16,
    /// The word at 0x14: [`SpanGrid::word`] for kind 2 and 3, else zero.
    pub a: u32,
    /// The word at 0x18: [`SpanGrid::word`] for kind 0 and 1, else zero.
    pub b: u32,
    pub spans: Vec<[GridSpan; 2]>,
    /// A byte per covered cell each, at the indices the spans state.
    pub planes: [Vec<u8>; 2],
}

impl SpanGrid {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "GTA VI span grid";
        let head = bytes
            .get(..SPAN_GRID_HEAD)
            .ok_or_else(|| short(WHAT, bytes.len()))?;
        if OtherBody::of(head, bytes.len() as u64) != Some(OtherBody::SpanGrid) {
            return Err(refuse(
                WHAT,
                "the head is not 01 00 01 02 with 128 x 128 twice".into(),
            ));
        }
        let (kind, first, end) = (u32_at(head, 8), u16_at(head, 0x10), u16_at(head, 0x12));
        let (a, b, len) = (u32_at(head, 0x14), u32_at(head, 0x18), u32_at(head, 0x1C));
        if kind > 3 || first > end || end > SPAN_GRID_SIDE {
            return Err(refuse(
                WHAT,
                format!("kind {kind} or rows {first}..{end} out of range"),
            ));
        }
        let words = match kind {
            0 => a == 0,
            1 => a == 0 && b != 0,
            _ => a != 0 && b == 0,
        };
        if !words {
            return Err(refuse(
                WHAT,
                format!("kind {kind} with words {a:#X} and {b:#X}"),
            ));
        }
        let word = if kind >= 2 { a } else { b };
        let form = if word == 0 {
            kind == 0 && (first, end, len) == (0, 0, 0)
        } else {
            (word & 0xFFFF) == (((word >> 16) & 0xFF) | 0x80) && end > 0
        };
        if !form || (kind >= 2 && ((first, end) != (0, SPAN_GRID_SIDE) || len == 0)) {
            return Err(refuse(
                WHAT,
                format!("kind {kind} with word {word:#010X} over rows {first}..{end} and {len} cells is no form a grid takes"),
            ));
        }
        let rows = usize::from(end - first);
        let len = len as usize;
        if bytes.len() as u64 != (SPAN_GRID_HEAD + 16 * rows) as u64 + 2 * len as u64 {
            return Err(refuse(
                WHAT,
                format!(
                    "{} bytes is not 32 + 16 x {rows} rows + 2 x {len} cells",
                    bytes.len()
                ),
            ));
        }
        let mut spans = Vec::with_capacity(rows);
        let mut covered = 0usize;
        for row in 0..rows {
            let at = SPAN_GRID_HEAD + 16 * row;
            let span = |s: usize| GridSpan {
                start: u16_at(bytes, at + 8 * s),
                end: u16_at(bytes, at + 8 * s + 2),
                offset: i32_at(bytes, at + 8 * s + 4),
            };
            let pair = [span(0), span(1)];
            for (s, span) in pair.iter().enumerate() {
                let (start, stop) = (i64::from(span.start), i64::from(span.end));
                let inside = start <= stop
                    && span.end <= SPAN_GRID_SIDE
                    && (start == stop
                        || (i64::from(span.offset) + start >= 0
                            && i64::from(span.offset) + stop <= len as i64));
                if !inside {
                    return Err(refuse(
                        WHAT,
                        format!("row {row} span {s} ({}, {}, {}) leaves the grid or the {len}-byte planes", span.start, span.end, span.offset),
                    ));
                }
                covered += usize::from(span.end - span.start);
            }
            spans.push(pair);
        }
        if covered != len {
            return Err(refuse(
                WHAT,
                format!("the spans cover {covered} cells of {len}"),
            ));
        }
        let planes = &bytes[SPAN_GRID_HEAD + 16 * rows..];
        Ok(Self {
            kind,
            first_row: first,
            a,
            b,
            spans,
            planes: [planes[..len].to_vec(), planes[len..].to_vec()],
        })
    }

    /// The word its kind places: low half = high half's low byte | 0x80, zero in the empty form.
    pub fn word(&self) -> u32 {
        if self.kind >= 2 {
            self.a
        } else {
            self.b
        }
    }

    /// Writes the head with the cell count the planes hold, the spans, then both planes.
    pub fn save(&self) -> Result<Vec<u8>> {
        let end = u16::try_from(self.spans.len())
            .ok()
            .and_then(|n| self.first_row.checked_add(n))
            .unwrap_or(u16::MAX);
        let len = u32::try_from(self.planes[0].len()).unwrap_or(u32::MAX);
        let mut out =
            Vec::with_capacity(SPAN_GRID_HEAD + 16 * self.spans.len() + 2 * self.planes[0].len());
        out.extend_from_slice(&[1, 0, 1, 2]);
        for word in [SPAN_GRID_SIDE, SPAN_GRID_SIDE] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&self.kind.to_le_bytes());
        for word in [SPAN_GRID_SIDE, SPAN_GRID_SIDE, self.first_row, end] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        for word in [self.a, self.b, len] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        for span in self.spans.iter().flatten() {
            out.extend_from_slice(&span.start.to_le_bytes());
            out.extend_from_slice(&span.end.to_le_bytes());
            out.extend_from_slice(&span.offset.to_le_bytes());
        }
        out.extend_from_slice(&self.planes[0]);
        out.extend_from_slice(&self.planes[1]);
        // What a save writes is what an open accepts: rows, spans and the two planes' lengths.
        Self::open(&out)?;
        Ok(out)
    }
}

/// The head of a MoVE network at version 0x13, and the bytes after its root node's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveNetworkHead {
    /// The word at 0x14, inferred to be the network's name hash.
    pub name_hash: u32,
    /// Hashes, inferred to be the networks this one refers to.
    pub list_a: Vec<u32>,
    pub list_b: Vec<u32>,
    /// The word after list B, 0 to 2, with no entries after it.
    pub word_after_b: u32,
    /// A hash and a byte each.
    pub list_d: Vec<(u32, u8)>,
    /// The ten bytes before the root node.
    pub before_root: [u8; 10],
    pub root_type: u16,
    pub root_index: u16,
    pub root_id: u32,
    /// Everything after the root node's id, as stored; `super::move_network` reads its nodes.
    pub rest: Vec<u8>,
}

impl MoveNetworkHead {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "GTA VI MoVE network";
        if bytes.get(..4) != Some(&b"MoVE"[..]) || bytes.get(4..20) != Some(&MOVE_VERSION[..]) {
            return Err(refuse(
                WHAT,
                "the head is not `MoVE` at version 0x13, 0, 0, 0".into(),
            ));
        }
        let word = |at: usize| {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| short(WHAT, bytes.len()))
        };
        if word(0x1C)? != 0 {
            return Err(refuse(WHAT, "the word at 0x1C is not zero".into()));
        }
        let list = |at: usize, size: usize, name: &str| -> Result<(usize, usize)> {
            let count = word(at)? as usize;
            let end = (at as u64 + 4) + count as u64 * size as u64;
            if end > bytes.len() as u64 {
                return Err(refuse(
                    WHAT,
                    format!("list {name} of {count} runs past the body"),
                ));
            }
            Ok((count, end as usize))
        };
        let (a, at_b) = list(0x20, 4, "A")?;
        let (b, at_x) = list(at_b, 4, "B")?;
        let word_after_b = word(at_x)?;
        let (d, at_ten) = list(at_x + 4, 5, "D")?;
        let root = at_ten + 10;
        if bytes.len() < root + 8 {
            return Err(short(WHAT, bytes.len()));
        }
        let length = u64::from(word(0x18)?);
        if length + MOVE_FIXED + 5 * d as u64 != bytes.len() as u64 {
            return Err(refuse(
                WHAT,
                format!(
                    "the length word {length} is not the body's {} bytes less 0x2E and 5 x {d}",
                    bytes.len()
                ),
            ));
        }
        let hashes = |from: usize, n: usize| -> Vec<u32> {
            (0..n).map(|i| u32_at(bytes, from + 4 * i)).collect()
        };
        let mut before_root = [0; 10];
        before_root.copy_from_slice(&bytes[at_ten..root]);
        Ok(Self {
            name_hash: word(0x14)?,
            list_a: hashes(0x24, a),
            list_b: hashes(at_b + 4, b),
            word_after_b,
            list_d: (0..d)
                .map(|i| (u32_at(bytes, at_x + 8 + 5 * i), bytes[at_x + 12 + 5 * i]))
                .collect(),
            before_root,
            root_type: u16_at(bytes, root),
            root_index: u16_at(bytes, root + 2),
            root_id: u32_at(bytes, root + 4),
            rest: bytes[root + 8..].to_vec(),
        })
    }

    /// Writes the head with its length word computed, then the carried bytes after the root's id.
    pub fn save(&self) -> Result<Vec<u8>> {
        const WHAT: &str = "GTA VI MoVE network";
        let mut out = Vec::new();
        out.extend_from_slice(b"MoVE");
        out.extend_from_slice(&MOVE_VERSION);
        out.extend_from_slice(&self.name_hash.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        for list in [&self.list_a, &self.list_b] {
            out.extend_from_slice(&count(list.len())?.to_le_bytes());
            list.iter()
                .for_each(|h| out.extend_from_slice(&h.to_le_bytes()));
        }
        out.extend_from_slice(&self.word_after_b.to_le_bytes());
        out.extend_from_slice(&count(self.list_d.len())?.to_le_bytes());
        for (hash, byte) in &self.list_d {
            out.extend_from_slice(&hash.to_le_bytes());
            out.push(*byte);
        }
        out.extend_from_slice(&self.before_root);
        out.extend_from_slice(&self.root_type.to_le_bytes());
        out.extend_from_slice(&self.root_index.to_le_bytes());
        out.extend_from_slice(&self.root_id.to_le_bytes());
        out.extend_from_slice(&self.rest);
        let length = (out.len() as u64) - MOVE_FIXED - 5 * self.list_d.len() as u64;
        let length =
            u32::try_from(length).map_err(|_| refuse(WHAT, "the body passes u32 bytes".into()))?;
        out[0x18..0x1C].copy_from_slice(&length.to_le_bytes());
        Ok(out)
    }
}

/// The bytes of an `EDUP` body's head: the tag and the count.
const EDUP_HEAD: usize = 8;

/// The head of an `EDUP` body; its payload has no record width its count divides and is not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdupHead {
    /// The word at +4.
    pub count: u32,
    /// The bytes after the 8-byte head.
    pub payload_len: u64,
    /// With a count of zero, the word at +8, which states the length of the bytes after it.
    pub stated_len: Option<u32>,
}

impl EdupHead {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "GTA VI EDUP body";
        let head = bytes
            .get(..EDUP_HEAD)
            .ok_or_else(|| short(WHAT, bytes.len()))?;
        if head[..4] != *b"EDUP" {
            return Err(refuse(WHAT, "the head is not `EDUP`".into()));
        }
        let count = u32_at(head, 4);
        let payload_len = (bytes.len() - EDUP_HEAD) as u64;
        let stated_len = if count == 0 {
            let after = bytes.len().checked_sub(EDUP_HEAD + 4).ok_or_else(|| {
                refuse(
                    WHAT,
                    format!(
                        "a count of zero with {payload_len} payload bytes leaves no length word"
                    ),
                )
            })?;
            let stated = u32_at(bytes, EDUP_HEAD);
            if u64::from(stated) != after as u64 {
                return Err(refuse(
                    WHAT,
                    format!("a count of zero, and its length word states {stated} bytes where {after} follow it"),
                ));
            }
            Some(stated)
        } else {
            None
        };
        Ok(Self {
            count,
            payload_len,
            stated_len,
        })
    }
}

fn count(n: usize) -> Result<u32> {
    u32::try_from(n).map_err(|_| refuse("GTA VI MoVE network", format!("a list of {n} passes u32")))
}

// The callers bound every offset by the body's length first.
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
    u32_at(bytes, at) as i32
}

fn f32s(bytes: &[u8], at: usize) -> [f32; 3] {
    [0, 4, 8].map(|i| f32::from_bits(u32_at(bytes, at + i)))
}

fn short(what: &str, len: usize) -> IoError {
    refuse(what, format!("{len} bytes is shorter than its head"))
}

fn refuse(what: &str, why: String) -> IoError {
    IoError::InvalidData {
        context: format!("{what}: {why}"),
    }
}

#[cfg(test)]
#[path = "other_bodies_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "other_bodies_edup_tests.rs"]
mod edup_tests;
