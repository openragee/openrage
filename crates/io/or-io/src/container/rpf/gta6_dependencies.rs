//! The RPF9 dependency lists: the archive's type-4 key table and each `0x97c2` row's indices into
//! it, decoded and authored back bit for bit.
//!
//! The type-4 metadata descriptor's payload is `{u32 count, u8 width, 3 zero bytes, u64 key[count]}`,
//! the keys strictly ascending. A `0x97c2` cell is `{u32 u0, u32 u1}`: `u0` a byte offset from the
//! cell to the row's `n` ascending indices into that table, `u1 = width << 29 | n`. The column is
//! deduplicated like every other, and its rows' index runs lie packed, in row order, in one heap.

use byteorder::{ByteOrder, LE};

use super::rpf9_toc::Rpf9Toc;
use crate::error::{IoError, Result};

/// The dependency-list column: 8-byte cells, each pointing at a row's indices into the type-4 table.
pub const GROUP_DEPENDENCIES: u64 = 0x97C2_7D97_4803_FFEC;
/// The metadata descriptor kind that holds the dependency key table.
const TABLE_KIND: u16 = 4;
/// `u1` bits 0..29, the row's index count.
const COUNT_MASK: u32 = 0x1FFF_FFFF;
/// The table payload's `{u32 count, u8 width, 3 zero bytes}` head.
const TABLE_HEAD: usize = 8;
/// The most keys a table this writer authors holds: width 4 is read but no stored table uses it.
const MAX_AUTHORED_KEYS: usize = 0x1_0000;

/// An archive's decoded dependency lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependencies {
    /// The type-4 table: unique keys, ascending.
    pub keys: Vec<u64>,
    /// Bytes per index into `keys`: 1, 2 or 4.
    pub width: u8,
    /// Each `0x97c2` row's keys, in row order.
    pub lists: Vec<Vec<u64>>,
    /// Each entry's `0x97c2` row, `None` where it has none.
    pub entry_rows: Vec<Option<usize>>,
}

impl Dependencies {
    /// Entry `e`'s dependency keys, `None` where it has no `0x97c2` row.
    pub fn entry(&self, e: usize) -> Option<&[u64]> {
        let row = (*self.entry_rows.get(e)?)?;
        self.lists.get(row).map(Vec::as_slice)
    }
}

/// One authored `0x97c2` cell: its row's byte offset in the heap and its `u1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListCell {
    pub offset: u32,
    pub u1: u32,
}

impl ListCell {
    /// The cell's bytes when it sits at `cell_at` and the heap at `heap_at`, both table offsets.
    pub fn bytes(&self, cell_at: usize, heap_at: usize) -> Result<[u8; 8]> {
        let u0 = (heap_at + self.offset as usize)
            .checked_sub(cell_at)
            .and_then(|d| u32::try_from(d).ok())
            .ok_or_else(|| {
                bad(format!(
                    "a list at {:#x} cannot be stated from a cell at {cell_at:#x}",
                    heap_at + self.offset as usize
                ))
            })?;
        let mut out = [0u8; 8];
        LE::write_u32(&mut out[0..], u0);
        LE::write_u32(&mut out[4..], self.u1);
        Ok(out)
    }
}

/// The type-4 table payload, one cell per list in order, and the heap of their packed indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authored {
    pub table: Vec<u8>,
    pub cells: Vec<ListCell>,
    pub heap: Vec<u8>,
}

fn bad(why: String) -> IoError {
    IoError::Invalid {
        what: "an RPF9 dependency list".into(),
        why,
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    bytes
        .get(at..at + 2)
        .map(LE::read_u16)
        .ok_or_else(|| bad(format!("the table ends before {at:#x}")))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .map(LE::read_u32)
        .ok_or_else(|| bad(format!("the table ends before {at:#x}")))
}

fn round16(x: usize) -> usize {
    (x + 15) & !15
}

/// The most keys an index of `width` bytes can reach.
fn capacity(width: u8) -> u64 {
    1u64 << (8 * u32::from(width))
}

/// Decode the type-4 table and every `0x97c2` row of `toc`; `None` when it has neither.
pub fn read(toc: &Rpf9Toc) -> Result<Option<Dependencies>> {
    let bytes = &toc.bytes;
    let table = table_range(bytes)?;
    let column = toc
        .groups
        .iter()
        .position(|g| g.name_hash == GROUP_DEPENDENCIES);
    let (table, column) = match (table, column) {
        (None, None) => return Ok(None),
        (Some(t), Some(c)) => (t, c),
        (Some(_), None) => return Err(bad("a type-4 key table with no 0x97c2 column".into())),
        (None, Some(_)) => return Err(bad("a 0x97c2 column with no type-4 key table".into())),
    };
    let (keys, width) = decode_table(bytes, table)?;
    let group = &toc.groups[column];
    if group.width != 8 {
        return Err(bad(format!("a 0x97c2 column {} bytes wide", group.width)));
    }
    let lists = (0..group.count as usize)
        .map(|r| decode_list(bytes, group.data.start + 8 * r, &keys, width))
        .collect::<Result<Vec<_>>>()?;
    let entry_rows = entry_rows(toc, column)?;
    Ok(Some(Dependencies {
        keys,
        width,
        lists,
        entry_rows,
    }))
}

/// The type-4 payload's start and the metadata's end, from the one kind-4 descriptor.
fn table_range(bytes: &[u8]) -> Result<Option<(usize, usize)>> {
    let n = u16_at(bytes, 0x1A)? as usize;
    let metadata_len = u32_at(bytes, 0x1C)? as usize;
    let metadata_start = 0x40 + round16(8 * n);
    let mut found = None;
    for i in 0..n {
        if u16_at(bytes, 0x40 + 8 * i)? != TABLE_KIND {
            continue;
        }
        if found.is_some() {
            return Err(bad("more than one type-4 descriptor".into()));
        }
        let offset = u32_at(bytes, 0x44 + 8 * i)? as usize;
        if offset >= metadata_len {
            return Err(bad("the type-4 descriptor points past the metadata".into()));
        }
        found = Some((metadata_start + offset, metadata_start + metadata_len));
    }
    Ok(found)
}

fn decode_table(bytes: &[u8], (at, end): (usize, usize)) -> Result<(Vec<u64>, u8)> {
    let head = bytes
        .get(at..at + TABLE_HEAD)
        .filter(|_| at + TABLE_HEAD <= end)
        .ok_or_else(|| bad("the type-4 table head runs past the metadata".into()))?;
    let count = LE::read_u32(head) as usize;
    let width = head[4];
    if head[5..] != [0, 0, 0] {
        return Err(bad("the type-4 table head sets a reserved byte".into()));
    }
    if !matches!(width, 1 | 2 | 4) {
        return Err(bad(format!("a type-4 index width of {width}")));
    }
    if count as u64 > capacity(width) {
        return Err(bad(format!(
            "a width-{width} index cannot reach {count} keys"
        )));
    }
    let from = at + TABLE_HEAD;
    let raw = from
        .checked_add(8 * count)
        .filter(|&to| to <= end)
        .and_then(|to| bytes.get(from..to))
        .ok_or_else(|| bad(format!("{count} keys run past the metadata")))?;
    let keys: Vec<u64> = raw
        .as_chunks::<8>()
        .0
        .iter()
        .map(|k| u64::from_le_bytes(*k))
        .collect();
    if keys.windows(2).any(|w| w[0] >= w[1]) {
        return Err(bad("the type-4 keys are not strictly ascending".into()));
    }
    Ok((keys, width))
}

fn decode_list(bytes: &[u8], cell_at: usize, keys: &[u64], width: u8) -> Result<Vec<u64>> {
    let u0 = u32_at(bytes, cell_at)? as usize;
    let u1 = u32_at(bytes, cell_at + 4)?;
    if u1 >> 29 != u32::from(width) {
        return Err(bad(format!(
            "a cell at {cell_at:#x} states width {} over a width-{width} table",
            u1 >> 29
        )));
    }
    let (n, w) = ((u1 & COUNT_MASK) as usize, usize::from(width));
    let from = cell_at + u0;
    let raw = bytes
        .get(from..from + n * w)
        .ok_or_else(|| bad(format!("{n} indices at {from:#x} run past the table")))?;
    let mut out = Vec::with_capacity(n);
    let mut last = None;
    for i in raw.chunks_exact(w) {
        let index = match w {
            1 => usize::from(i[0]),
            2 => usize::from(LE::read_u16(i)),
            _ => LE::read_u32(i) as usize,
        };
        if last.is_some_and(|l| index <= l) {
            return Err(bad(format!("the list at {from:#x} is not ascending")));
        }
        let key = keys
            .get(index)
            .ok_or_else(|| bad(format!("index {index} past a table of {} keys", keys.len())))?;
        out.push(*key);
        last = Some(index);
    }
    Ok(out)
}

/// Each entry's row in column `column`, through the entry-major index arrays.
fn entry_rows(toc: &Rpf9Toc, column: usize) -> Result<Vec<Option<usize>>> {
    let bytes = &toc.bytes;
    let split = u32_at(bytes, 0x08)? as usize;
    let metadata_len = u32_at(bytes, 0x1C)? as usize;
    let index_width = *bytes
        .get(0x2C)
        .ok_or_else(|| bad("the table ends before its index width".into()))?
        as usize;
    if !(1..=2).contains(&index_width) {
        return Err(bad(format!("an entry index width of {index_width}")));
    }
    let metadata_start = 0x40 + round16(8 * u16_at(bytes, 0x1A)? as usize);
    let groups_start = round16(metadata_start + metadata_len);
    let group = &toc.groups[column];
    let kind = toc
        .groups
        .iter()
        .filter(|g| g.external == group.external)
        .count();
    let slot = toc.groups[..column]
        .iter()
        .filter(|g| g.external == group.external)
        .count();
    let base = if group.external {
        split
    } else {
        groups_start + 24 * toc.groups.len()
    };
    (0..toc.entry_count as usize)
        .map(|e| {
            let at = base + (e * kind + slot) * index_width;
            let value = if index_width == 1 {
                bytes.get(at).map(|b| usize::from(*b))
            } else {
                bytes.get(at..at + 2).map(|b| usize::from(LE::read_u16(b)))
            }
            .ok_or_else(|| bad("the entry index runs past the table".into()))?;
            Ok((value < group.count as usize).then_some(value))
        })
        .collect()
}

/// The type-4 table, cells and heap for `lists`, one per `0x97c2` row in row order.
pub fn author(lists: &[Vec<u64>]) -> Result<Authored> {
    let mut sets = Vec::with_capacity(lists.len());
    for (i, list) in lists.iter().enumerate() {
        let mut set = list.clone();
        set.sort_unstable();
        set.dedup();
        if set.is_empty() {
            return Err(bad(format!(
                "list {i} is empty, and an entry with no dependency has no row"
            )));
        }
        sets.push(set);
    }
    let mut keys: Vec<u64> = sets.iter().flatten().copied().collect();
    keys.sort_unstable();
    keys.dedup();
    if keys.len() > MAX_AUTHORED_KEYS {
        return Err(bad(format!(
            "{} keys need a width-4 index, which no stored table uses",
            keys.len()
        )));
    }
    let width: u8 = if keys.len() as u64 <= capacity(1) {
        1
    } else {
        2
    };

    let mut table = Vec::with_capacity(TABLE_HEAD + 8 * keys.len());
    table.extend_from_slice(&(keys.len() as u32).to_le_bytes());
    table.extend_from_slice(&[width, 0, 0, 0]);
    for key in &keys {
        table.extend_from_slice(&key.to_le_bytes());
    }

    let mut heap = Vec::new();
    let mut cells = Vec::with_capacity(sets.len());
    for set in &sets {
        let offset = u32::try_from(heap.len())
            .map_err(|_| bad("the lists overflow a 32-bit heap offset".into()))?;
        for key in set {
            let index = keys.partition_point(|k| k < key);
            if width == 1 {
                heap.push(index as u8);
            } else {
                heap.extend_from_slice(&(index as u16).to_le_bytes());
            }
        }
        cells.push(ListCell {
            offset,
            u1: (u32::from(width) << 29) | set.len() as u32,
        });
    }
    Ok(Authored { table, cells, heap })
}

#[cfg(test)]
#[path = "gta6_dependencies_tests.rs"]
mod tests;
