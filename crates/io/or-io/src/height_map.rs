//! `heightmap.dat`: the world's coarse terrain grid, a maximum and a minimum
//! height byte per cell over a bounding box, stored big-endian.
//!
//! A save writes each row's span the canonical way (trimmed of cells whose
//! two heights are both zero), which every shipped file already is, so a
//! shipped file saves byte-identical.

use crate::error::{IoError, Result};

const TAG: u32 = u32::from_be_bytes(*b"HMAP");
const HEADER_LEN: usize = 44;
const RLE_LEN: usize = 8;

/// One terrain height grid, as its header states it and its cells decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightMap {
    pub version: u8,
    pub flags: u32,
    pub num_cols: u16,
    pub num_rows: u16,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Each cell's maximum height, row by row from `bounds_min`'s Y; 0 for an empty cell.
    pub max: Vec<u8>,
    /// Each cell's minimum height, laid out as [`HeightMap::max`].
    pub min: Vec<u8>,
    /// One bit per cell, present exactly when [`HeightMap::WATER_MASK`] is set.
    pub water_mask: Vec<u8>,
}

impl HeightMap {
    /// The cells are stored as one span per row rather than the whole grid.
    pub const RLE_DATA: u32 = 1 << 0;
    /// A bit per cell follows the heights.
    pub const WATER_MASK: u32 = 1 << 1;

    /// Reads a whole `heightmap.dat`, refusing any length or span the bytes do not bear out.
    pub fn open(bytes: &[u8]) -> Result<Self> {
        let header = bytes
            .get(..HEADER_LEN)
            .ok_or_else(|| refuse(format!("{} bytes is shorter than the header", bytes.len())))?;
        let u32_at = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap());
        let u16_at = |at: usize| u16::from_be_bytes(header[at..at + 2].try_into().unwrap());
        let f32_at = |at: usize| f32::from_bits(u32_at(at));
        let tag = u32_at(0);
        if tag != TAG {
            return Err(refuse(format!(
                "the tag is {tag:#010x}, not big-endian `HMAP`"
            )));
        }
        let cell_size = header[5];
        if cell_size != 1 {
            return Err(refuse(format!(
                "cells are {cell_size} bytes; only one-byte cells are read"
            )));
        }
        if header[6..8] != [0, 0] {
            return Err(refuse("the header's two pad bytes are not zero".into()));
        }
        let flags = u32_at(8);
        let num_cols = u16_at(12);
        let num_rows = u16_at(14);
        let data_len = u32_at(40) as usize;
        let cells = num_cols as usize * num_rows as usize;
        let data = bytes
            .get(HEADER_LEN..)
            .and_then(|rest| rest.get(..data_len))
            .ok_or_else(|| {
                refuse(format!(
                    "the header states {data_len} bytes of heights and {} follow it",
                    bytes.len() - HEADER_LEN
                ))
            })?;

        let mut max = vec![0u8; cells];
        let mut min = vec![0u8; cells];
        if flags & Self::RLE_DATA != 0 {
            let spans_len = num_rows as usize * RLE_LEN;
            let heights = data
                .get(spans_len..)
                .filter(|h| h.len() % 2 == 0)
                .ok_or_else(|| {
                    refuse(format!(
                        "{data_len} bytes of heights do not hold {num_rows} row spans and two equal halves"
                    ))
                })?;
            let (max_data, min_data) = heights.split_at(heights.len() / 2);
            for row in 0..num_rows as usize {
                let rle = &data[row * RLE_LEN..][..RLE_LEN];
                let start = u16::from_be_bytes([rle[0], rle[1]]) as usize;
                let count = u16::from_be_bytes([rle[2], rle[3]]) as usize;
                let offset = i32::from_be_bytes(rle[4..8].try_into().unwrap()) as i64;
                if count == 0 {
                    continue;
                }
                let from = offset + start as i64;
                let span = usize::try_from(from)
                    .ok()
                    .filter(|&from| start + count <= num_cols as usize && from + count <= max_data.len())
                    .ok_or_else(|| {
                        refuse(format!(
                            "row {row}'s span ({count} cells from column {start}, data offset {offset}) \
                             leaves the {num_cols}-column grid or the {}-byte halves",
                            max_data.len()
                        ))
                    })?;
                let at = row * num_cols as usize + start;
                max[at..at + count].copy_from_slice(&max_data[span..span + count]);
                min[at..at + count].copy_from_slice(&min_data[span..span + count]);
            }
        } else {
            if data.len() != 2 * cells {
                return Err(refuse(format!(
                    "{data_len} bytes of heights is not two bytes for each of {cells} cells"
                )));
            }
            max.copy_from_slice(&data[..cells]);
            min.copy_from_slice(&data[cells..]);
        }

        let tail = &bytes[HEADER_LEN + data_len..];
        let mask_len = if flags & Self::WATER_MASK != 0 {
            cells.div_ceil(8)
        } else {
            0
        };
        if tail.len() != mask_len {
            return Err(refuse(format!(
                "{} bytes follow the heights where the water mask takes {mask_len}",
                tail.len()
            )));
        }

        let map = Self {
            version: header[4],
            flags,
            num_cols,
            num_rows,
            bounds_min: [f32_at(16), f32_at(20), f32_at(24)],
            bounds_max: [f32_at(28), f32_at(32), f32_at(36)],
            max,
            min,
            water_mask: tail.to_vec(),
        };
        Ok(map)
    }

    /// Writes the file, refusing a grid or water mask whose size the header does not state.
    pub fn save(&self) -> Result<Vec<u8>> {
        let cols = self.num_cols as usize;
        let cells = cols * self.num_rows as usize;
        if self.max.len() != cells || self.min.len() != cells {
            return Err(refuse(format!(
                "{} maximum and {} minimum heights for a {}x{} grid",
                self.max.len(),
                self.min.len(),
                self.num_cols,
                self.num_rows
            )));
        }
        let mask_len = if self.flags & Self::WATER_MASK != 0 {
            cells.div_ceil(8)
        } else {
            0
        };
        if self.water_mask.len() != mask_len {
            return Err(refuse(format!(
                "a water mask of {} bytes where the flags call for {mask_len}",
                self.water_mask.len()
            )));
        }

        let mut data = Vec::new();
        if self.flags & Self::RLE_DATA != 0 {
            let mut max_data = Vec::new();
            let mut min_data = Vec::new();
            for row in 0..self.num_rows as usize {
                let max = &self.max[row * cols..][..cols];
                let min = &self.min[row * cols..][..cols];
                let live = |x: &usize| max[*x] != 0 || min[*x] != 0;
                let (start, count, offset) = match ((0..cols).find(live), (0..cols).rfind(live)) {
                    (Some(start), Some(stop)) => {
                        let offset = max_data.len() as i64 - start as i64;
                        max_data.extend_from_slice(&max[start..=stop]);
                        min_data.extend_from_slice(&min[start..=stop]);
                        (start, stop - start + 1, offset)
                    }
                    _ => (0, 0, 0),
                };
                data.extend_from_slice(&(start as u16).to_be_bytes());
                data.extend_from_slice(&(count as u16).to_be_bytes());
                let offset = i32::try_from(offset)
                    .map_err(|_| refuse(format!("row {row}'s data offset {offset} passes i32")))?;
                data.extend_from_slice(&offset.to_be_bytes());
            }
            data.extend_from_slice(&max_data);
            data.extend_from_slice(&min_data);
        } else {
            data.extend_from_slice(&self.max);
            data.extend_from_slice(&self.min);
        }
        let data_len = u32::try_from(data.len())
            .map_err(|_| refuse(format!("{} bytes of heights passes u32", data.len())))?;

        let mut out = Vec::with_capacity(HEADER_LEN + data.len() + mask_len);
        out.extend_from_slice(&TAG.to_be_bytes());
        out.extend_from_slice(&[self.version, 1, 0, 0]);
        out.extend_from_slice(&self.flags.to_be_bytes());
        out.extend_from_slice(&self.num_cols.to_be_bytes());
        out.extend_from_slice(&self.num_rows.to_be_bytes());
        for v in self.bounds_min.iter().chain(&self.bounds_max) {
            out.extend_from_slice(&v.to_bits().to_be_bytes());
        }
        out.extend_from_slice(&data_len.to_be_bytes());
        out.extend_from_slice(&data);
        out.extend_from_slice(&self.water_mask);
        Ok(out)
    }

    /// A cell's world size in X and Y.
    pub fn cell_size(&self) -> [f32; 2] {
        [
            (self.bounds_max[0] - self.bounds_min[0]) / f32::from(self.num_cols.max(1)),
            (self.bounds_max[1] - self.bounds_min[1]) / f32::from(self.num_rows.max(1)),
        ]
    }

    /// Sets the cell at `col`, `row` to the maximum `height`: its minimum follows where it was equal and never stays above it.
    pub fn set_cell(&mut self, col: u16, row: u16, height: u8) -> Result<()> {
        let at = usize::from(row) * usize::from(self.num_cols) + usize::from(col);
        if col >= self.num_cols || row >= self.num_rows || at >= self.max.len().min(self.min.len())
        {
            return Err(refuse(format!(
                "cell {col}, {row} is outside the {}x{} grid",
                self.num_cols, self.num_rows
            )));
        }
        // One array (a grid whose two halves are one) keeps both halves equal.
        if self.min[at] == self.max[at] || self.min[at] > height {
            self.min[at] = height;
        }
        self.max[at] = height;
        Ok(())
    }
}

fn refuse(why: String) -> IoError {
    IoError::InvalidData {
        context: format!("heightmap: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> HeightMap {
        HeightMap {
            version: 1,
            flags: HeightMap::RLE_DATA,
            num_cols: 4,
            num_rows: 3,
            bounds_min: [-100.0, -50.0, 0.0],
            bounds_max: [100.0, 100.0, 800.0],
            max: vec![0, 9, 0, 7, 0, 0, 0, 0, 5, 6, 0, 0],
            min: vec![0, 1, 0, 2, 0, 0, 0, 0, 0, 3, 0, 0],
            water_mask: Vec::new(),
        }
    }

    #[test]
    fn a_grid_saves_its_spans_and_reopens_equal() {
        let map = grid();
        let bytes = map.save().expect("saves");
        // Row 0 spans columns 1..=3 (the zero cell inside stays), row 1 is
        // empty, row 2 spans 0..=1 and its offset counts back to column 0.
        let spans: Vec<[u8; 8]> = (0..3)
            .map(|r| bytes[44 + 8 * r..][..8].try_into().unwrap())
            .collect();
        assert_eq!(spans[0], [0, 1, 0, 3, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(spans[1], [0; 8]);
        assert_eq!(spans[2], [0, 0, 0, 2, 0, 0, 0, 3]);
        assert_eq!(&bytes[44 + 24..], &[9, 0, 7, 5, 6, 1, 0, 2, 0, 3]);
        assert_eq!(HeightMap::open(&bytes).expect("reopens"), map);
        assert_eq!(map.cell_size(), [50.0, 50.0]);
    }

    #[test]
    fn a_whole_grid_and_a_water_mask_round_trip() {
        let mut map = grid();
        map.flags = HeightMap::WATER_MASK;
        map.water_mask = vec![0b1010_0101, 0b0000_1111];
        let bytes = map.save().expect("saves");
        assert_eq!(bytes.len(), 44 + 24 + 2);
        assert_eq!(HeightMap::open(&bytes).expect("reopens"), map);
    }

    #[test]
    fn a_cell_set_moves_its_minimum_only_where_it_was_equal_or_above_and_reopens_equal() {
        let mut map = grid();
        // Cell 1, 0 holds 9 over 1: the minimum stays. Cell 0, 2 holds 5 over 0: 0 is below 3.
        map.set_cell(1, 0, 4).expect("sets");
        map.set_cell(0, 2, 3).expect("sets");
        // Cell 1, 2 holds 6 over 3: the new maximum 2 is below it, so the minimum comes down.
        map.set_cell(1, 2, 2).expect("sets");
        assert_eq!(map.max, [0, 4, 0, 7, 0, 0, 0, 0, 3, 2, 0, 0]);
        assert_eq!(map.min, [0, 1, 0, 2, 0, 0, 0, 0, 0, 2, 0, 0]);
        let mut one = HeightMap {
            min: grid().max,
            ..grid()
        };
        one.set_cell(3, 0, 200).expect("sets");
        assert_eq!(
            (one.max[3], one.min[3]),
            (200, 200),
            "equal halves stay equal"
        );
        let bytes = map.save().expect("saves");
        assert_eq!(bytes.len(), grid().save().expect("saves").len());
        assert_eq!(HeightMap::open(&bytes).expect("reopens"), map);
        for (col, row) in [(4, 0), (0, 3)] {
            let error = map.set_cell(col, row, 1).unwrap_err().to_string();
            assert!(error.contains("outside the 4x3 grid"), "{error}");
        }
    }

    #[test]
    fn damaged_bytes_are_refused_by_name() {
        let bytes = grid().save().expect("saves");
        let mut short = bytes.clone();
        short.pop();
        assert!(HeightMap::open(&short)
            .unwrap_err()
            .to_string()
            .contains("heightmap"));
        let mut wide = bytes.clone();
        wide[44 + 2..44 + 4].copy_from_slice(&9u16.to_be_bytes());
        let error = HeightMap::open(&wide).unwrap_err().to_string();
        assert!(error.contains("row 0"), "{error}");
        let mut little = bytes;
        little[..4].copy_from_slice(b"PAMH");
        assert!(HeightMap::open(&little).is_err());
        let mut wrong = grid();
        wrong.max.pop();
        assert!(wrong.save().is_err());
    }
}
