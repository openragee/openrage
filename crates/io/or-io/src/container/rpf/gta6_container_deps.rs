//! An entry's dependency list, set against the archive's type-4 key table and growing it by a key it
//! lacks; and the mode a written chunk table takes in a table whose rows take mode 4.

use byteorder::{ByteOrder, LE};

use super::super::gta6_archive::GROUP_CHUNK_RECORD;
use super::super::gta6_chunk_table::{author, decode, Table as ChunkTable};
use super::super::gta6_dependencies::{self, GROUP_DEPENDENCIES};
use super::bad;
use super::build::column;
use super::table::Table;
use crate::error::Result;

/// The chunk-table mode of every row of a table that holds one in it.
pub(super) const MODE4: u8 = 4;

/// Set entry `e`'s dependency list to `keys`; no keys leaves it no row. A key the type-4 table
/// lacks is added to it, which re-indexes every list and widens each past 256 keys; a key no list
/// names any more stays.
pub(super) fn set_dependencies(table: &mut Table, e: usize, keys: &[u64]) -> Result<()> {
    let (mut known, mut width) = gta6_dependencies::table_keys(&table.bytes[..table.groups_start])?
        .ok_or_else(|| bad("the archive has no dependency lists to set an entry's in"))?;
    let c = column(table, GROUP_DEPENDENCIES, 8, "dependency-list")?;
    let mut set = keys.to_vec();
    set.sort_unstable();
    set.dedup();
    if set.is_empty() {
        table.set_cell(e, c, None, Vec::new());
        return Ok(());
    }
    let missing: Vec<u64> = (set.iter().copied())
        .filter(|k| known.binary_search(k).is_err())
        .collect();
    if !missing.is_empty() {
        let mut grown = known.clone();
        grown.extend(missing);
        grown.sort_unstable();
        let grown_width = gta6_dependencies::width_for(grown.len())?;
        for row in &mut table.columns[c].rows {
            let named = listed(&row.slice, width, &known)?;
            row.slice = indices(&named, &grown, grown_width)?;
            let u1 = (u32::from(grown_width) << 29) | named.len() as u32;
            LE::write_u32(&mut row.cell[4..], u1);
        }
        let prefix = gta6_dependencies::grow_table(&table.bytes[..table.groups_start], &grown)?;
        table.bytes = [&prefix[..], &table.bytes[table.groups_start..]].concat();
        table.groups_start = prefix.len();
        table.relaid = true;
        (known, width) = (grown, grown_width);
    }
    // The offset word is kept, as a table rewritten in place keeps the slice where it was.
    let mut cell = (table.cell(e, c).map(<[u8]>::to_vec)).unwrap_or_else(|| vec![0; 8]);
    LE::write_u32(&mut cell[4..], (u32::from(width) << 29) | set.len() as u32);
    let slice = indices(&set, &known, width)?;
    table.set_cell(e, c, Some(cell), slice);
    Ok(())
}

/// The keys a list's `slice` of `width`-byte indices names in `keys`.
fn listed(slice: &[u8], width: u8, keys: &[u64]) -> Result<Vec<u64>> {
    (slice.chunks_exact(usize::from(width)))
        .map(|i| {
            let index = match width {
                1 => usize::from(i[0]),
                2 => usize::from(LE::read_u16(i)),
                _ => LE::read_u32(i) as usize,
            };
            keys.get(index).copied().ok_or_else(|| {
                bad(format!(
                    "a dependency index {index} past a table of {} keys",
                    keys.len()
                ))
            })
        })
        .collect()
}

/// `set`'s `width`-byte indices into `keys`, which holds every one of them.
fn indices(set: &[u64], keys: &[u64], width: u8) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(set.len() * usize::from(width));
    for key in set {
        let i = keys
            .binary_search(key)
            .map_err(|_| bad(format!("key {key:#018x} is not in the dependency table")))?;
        match width {
            1 => out.push(i as u8),
            2 => out.extend_from_slice(&(i as u16).to_le_bytes()),
            _ => out.extend_from_slice(&(i as u32).to_le_bytes()),
        }
    }
    Ok(out)
}

/// Whether the table's chunk tables take mode 4: no measured table holds it beside another mode.
pub(super) fn takes_mode4(table: &Table) -> bool {
    table
        .col(GROUP_CHUNK_RECORD, 8)
        .is_some_and(|k| (table.columns[k].rows.iter()).any(|r| r.cell.get(7) == Some(&MODE4)))
}

/// Restate new entry `e`'s chunk table, where it has one, in mode 4.
pub(super) fn restate_in_mode4(table: &mut Table, e: usize) -> Result<()> {
    let Some(k) = table.col(GROUP_CHUNK_RECORD, 8) else {
        return Ok(());
    };
    let Some(r) = table.row_of(e, k) else {
        return Ok(());
    };
    let row = &table.columns[k].rows[r];
    // Decoded as a cell whose descriptors follow it.
    let mut heap = row.cell.clone();
    LE::write_u32(&mut heap[..4], 8);
    heap.extend_from_slice(&row.slice);
    let mut cell = [0u8; 8];
    cell.copy_from_slice(&heap[..8]);
    let chunks = decode(cell, &heap)?.chunks;
    let (u1, words) = author(&ChunkTable {
        mode: MODE4,
        chunks,
    })?;
    table.set_chunks(e, k, u1, &words);
    Ok(())
}
