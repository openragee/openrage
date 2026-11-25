//! The RPF9 chunk table: the `0xe0c1` cell of a compressed entry and its slice of 8-byte chunk
//! descriptors, decoded into exact per-chunk sizes and authored back bit for bit.
//!
//! The cell is `{u32 u0, u32 u1}`: `u1` holds the chunk count (bits 0..16), a constant `0x08`
//! (bits 16..24) and the mode (bits 24..32); `u0` is a byte offset from the cell to the entry's
//! descriptors, which sit in a heap after the column group. Mode 3 (24-byte rows) packs two
//! complemented 16-bit sizes, a running stored offset and a flag; mode 2 (20-byte rows) packs two
//! 24-bit sizes in 16-byte units, the stored padding and a flag. Mode 4 packs a 28-bit exact inflated
//! size, a 24-bit stored size in 16-byte units, the padding and a flag on either row shape. Chunks
//! lie 16-aligned in the body.

use byteorder::{ByteOrder, LE};

use super::gta6_archive::{Gta6Archive, GROUP_CHUNK_RECORD};
use crate::error::{IoError, Result};

/// `u1` bits 16..24, the same in every row of both modes.
const CELL_MARK: u32 = 0x08;
/// The flag of a deflate-compressed chunk; `0` is a stored chunk.
const FLAG_ZLIB: u64 = 3;
/// Mode 2's bits that hold no field: 24..32 and 56..58.
const MODE2_RESERVED: u64 = (0xFF << 24) | (0x3 << 56);
/// Mode 3's bits that hold no field: 62..64.
const MODE3_RESERVED: u64 = 0x3 << 62;
/// Mode 4's bits that hold no field: 58..64.
const MODE4_RESERVED: u64 = 0x3F << 58;
const MASK24: u64 = 0xFF_FFFF;
const MASK28: u64 = 0x0FFF_FFFF;

/// One chunk of a compressed body: its inflated and exact stored sizes, and whether it is deflated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub inflated: u32,
    pub stored: u32,
    pub zlib: bool,
}

/// A decoded chunk table: its mode (2, 3 or 4) and its chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub mode: u8,
    pub chunks: Vec<Chunk>,
}

fn bad(why: String) -> IoError {
    IoError::Invalid {
        what: "an RPF9 chunk table".into(),
        why,
    }
}

fn ceil16(x: u64) -> u64 {
    (x + 15) & !15
}

/// Decode a chunk table from its `cell` and `heap`, the table bytes from the cell onward.
pub fn decode(cell: [u8; 8], heap: &[u8]) -> Result<Table> {
    let u0 = LE::read_u32(&cell[0..]) as usize;
    let u1 = LE::read_u32(&cell[4..]);
    let count = (u1 & 0xFFFF) as usize;
    let mode = u1 >> 24;
    if (u1 >> 16) & 0xFF != CELL_MARK {
        return Err(bad(format!(
            "cell byte 6 is {:#x}, not 0x08",
            (u1 >> 16) & 0xFF
        )));
    }
    if !(2..=4).contains(&mode) {
        return Err(bad(format!("unknown mode {mode}")));
    }
    let words = u0
        .checked_add(count * 8)
        .and_then(|end| heap.get(u0..end))
        .ok_or_else(|| {
            bad(format!(
                "{count} descriptors at +{u0:#x} run past the table"
            ))
        })?;

    let mut chunks = Vec::with_capacity(count);
    let mut cum = 0u64; // the running ceil16(stored), in bytes
    for (i, word) in words
        .as_chunks::<8>()
        .0
        .iter()
        .map(|w| u64::from_le_bytes(*w))
        .enumerate()
    {
        let (inflated, stored, flag) = if mode == 3 {
            if word & MODE3_RESERVED != 0 {
                return Err(bad(format!("descriptor {i} sets a reserved bit")));
            }
            if (word >> 32) & MASK28 != (cum >> 4) & MASK28 {
                return Err(bad(format!(
                    "descriptor {i} states a stored offset its chunks do not sum to"
                )));
            }
            (
                0x10000 - (word & 0xFFFF),
                0x10000 - ((word >> 16) & 0xFFFF),
                (word >> 60) & 0x3,
            )
        } else {
            let (reserved, inflated, aligned, pad, flag) = if mode == 2 {
                (
                    MODE2_RESERVED,
                    (word & MASK24) << 4,
                    ((word >> 32) & MASK24) << 4,
                    (word >> 58) & 0xF,
                    word >> 62,
                )
            } else {
                (
                    MODE4_RESERVED,
                    word & MASK28,
                    ((word >> 28) & MASK24) << 4,
                    (word >> 52) & 0xF,
                    (word >> 56) & 0x3,
                )
            };
            if word & reserved != 0 {
                return Err(bad(format!("descriptor {i} sets a reserved bit")));
            }
            if pad > aligned {
                return Err(bad(format!("descriptor {i} pads past its stored size")));
            }
            (inflated, aligned - pad, flag)
        };
        if flag != FLAG_ZLIB && flag != 0 {
            return Err(bad(format!("descriptor {i} has unknown flag {flag}")));
        }
        if flag == 0 && inflated != stored {
            return Err(bad(format!(
                "stored chunk {i} inflates to {inflated} bytes from {stored}"
            )));
        }
        chunks.push(Chunk {
            inflated: inflated as u32,
            stored: stored as u32,
            zlib: flag == FLAG_ZLIB,
        });
        cum += ceil16(stored);
    }
    Ok(Table {
        mode: mode as u8,
        chunks,
    })
}

/// The cell's `u1` and the descriptors for `table`, refusing a size its mode cannot hold.
pub fn author(table: &Table) -> Result<(u32, Vec<u64>)> {
    let (chunks, mode) = (&table.chunks[..], table.mode);
    if !(2..=4).contains(&mode) {
        return Err(bad(format!("unknown mode {mode}")));
    }
    if chunks.len() > 0xFFFF {
        return Err(bad(format!(
            "{} chunks overflow the 16-bit count",
            chunks.len()
        )));
    }
    let mut cum = 0u64; // the running ceil16(stored), in bytes
    for (i, c) in chunks.iter().enumerate() {
        let stored16 = ceil16(u64::from(c.stored)) >> 4;
        let fits = match mode {
            3 => {
                (1..=0x10000).contains(&c.inflated)
                    && (1..=0x10000).contains(&c.stored)
                    && cum >> 4 <= MASK28
            }
            2 => {
                c.inflated.is_multiple_of(16)
                    && u64::from(c.inflated) >> 4 <= MASK24
                    && stored16 <= MASK24
            }
            _ => u64::from(c.inflated) <= MASK28 && stored16 <= MASK24,
        };
        if !fits || (!c.zlib && c.inflated != c.stored) {
            return Err(bad(format!("chunk {i} cannot be stated in mode {mode}")));
        }
        cum += ceil16(u64::from(c.stored));
    }
    Ok(pack(chunks, mode))
}

fn pack(chunks: &[Chunk], mode: u8) -> (u32, Vec<u64>) {
    let u1 = (chunks.len() as u32 & 0xFFFF) | (CELL_MARK << 16) | (u32::from(mode) << 24);
    let mut cum = 0u64;
    let words = chunks
        .iter()
        .map(|c| {
            let (inflated, stored) = (u64::from(c.inflated), u64::from(c.stored));
            let flag = if c.zlib { FLAG_ZLIB } else { 0 };
            let pad = stored.wrapping_neg() & 0xF;
            let stored16 = (ceil16(stored) >> 4) & MASK24;
            let word = match mode {
                3 => {
                    (flag << 60)
                        | (((cum >> 4) & MASK28) << 32)
                        | (((0x10000 - stored) & 0xFFFF) << 16)
                        | ((0x10000 - inflated) & 0xFFFF)
                }
                2 => (flag << 62) | (pad << 58) | (stored16 << 32) | ((inflated >> 4) & MASK24),
                _ => (flag << 56) | (pad << 52) | (stored16 << 28) | (inflated & MASK28),
            };
            cum += ceil16(stored);
            word
        })
        .collect();
    (u1, words)
}

/// The byte offset in `archive.toc.bytes` of entry `index`'s `0xe0c1` cell, where it has one.
pub fn cell_offset(archive: &Gta6Archive, index: usize) -> Option<usize> {
    let bytes = &archive.toc.bytes;
    let read_u16 = |at: usize| bytes.get(at..at + 2).map(LE::read_u16);
    let read_u32 = |at: usize| bytes.get(at..at + 4).map(LE::read_u32);
    let split = read_u32(0x08)? as usize;
    let metadata_len = read_u32(0x1C)? as usize;
    let group_count = read_u16(0x20)? as usize;
    let external_count = read_u16(0x22)? as usize;
    let entries = read_u32(0x24)? as usize;
    let index_width = *bytes.get(0x2C)? as usize;
    if index >= entries || !(1..=2).contains(&index_width) || external_count > group_count {
        return None;
    }
    let metadata_start = 0x40 + ceil16(8 * u64::from(read_u16(0x1A)?)) as usize;
    let groups_start = ceil16((metadata_start + metadata_len) as u64) as usize;

    // A column's slot is its position among the columns of its kind (main or external).
    let at = archive
        .toc
        .groups
        .iter()
        .position(|g| g.name_hash == GROUP_CHUNK_RECORD && g.width == 8)?;
    let group = &archive.toc.groups[at];
    let slot = archive.toc.groups[..at]
        .iter()
        .filter(|g| g.external == group.external)
        .count();
    let (base, stride) = if group.external {
        (split, external_count * index_width)
    } else {
        (
            groups_start + 24 * group_count,
            (group_count - external_count) * index_width,
        )
    };
    let o = base + index * stride + slot * index_width;
    let raw = bytes.get(o..o + index_width)?;
    let row = if index_width == 1 {
        raw[0] as usize
    } else {
        LE::read_u16(raw) as usize
    };
    if row >= group.count as usize {
        return None;
    }
    let cell = group.data.start + row * 8;
    bytes.get(cell..cell + 8).map(|_| cell)
}

/// Entry `index`'s decoded chunk table and mode, absent when it has no `0xe0c1` row.
pub fn entry_table(archive: &Gta6Archive, index: usize) -> Result<Option<Table>> {
    let Some(at) = cell_offset(archive, index) else {
        return Ok(None);
    };
    let heap = &archive.toc.bytes[at..];
    let mut cell = [0u8; 8];
    cell.copy_from_slice(&heap[..8]);
    decode(cell, heap).map(Some)
}

/// Entry `index`'s chunk descriptors, absent when it has no `0xe0c1` row.
pub fn entry_chunks(archive: &Gta6Archive, index: usize) -> Result<Option<Vec<Chunk>>> {
    entry_table(archive, index).map(|table| table.map(|table| table.chunks))
}

/// A two-entry archive whose entry 0 has its cell at 0x60, the head of `cell_and_heap`; entry 1 has none.
#[cfg(test)]
pub(crate) fn one_entry_archive(cell_and_heap: &[u8]) -> Gta6Archive {
    use super::rpf9_toc::{Rpf9Group, Rpf9Toc};
    let mut bytes = vec![0u8; 0x60];
    LE::write_u32(&mut bytes[0x08..], 0x60); // split; no external column
    LE::write_u16(&mut bytes[0x20..], 1); // one group
    LE::write_u32(&mut bytes[0x24..], 2); // two entries
    bytes[0x2C] = 1; // 1-byte index
    bytes[0x58] = 0; // entry 0 -> row 0
    bytes[0x59] = 0xFF; // entry 1 -> no row
    bytes.extend_from_slice(cell_and_heap);
    Gta6Archive {
        toc: Rpf9Toc {
            bytes,
            key_id: 0,
            registry_id: 0,
            entry_count: 2,
            groups: vec![Rpf9Group {
                name_hash: GROUP_CHUNK_RECORD,
                count: 1,
                width: 8,
                external: false,
                data: 0x60..0x68,
            }],
        },
        entries: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cell `{u0, u1}` followed by `gap` zero bytes and the descriptors.
    fn table(mode: u32, count: u32, gap: usize, words: &[u64]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(8 + gap as u32).to_le_bytes());
        out.extend_from_slice(&(count | (0x08 << 16) | (mode << 24)).to_le_bytes());
        out.resize(8 + gap, 0);
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    fn cell(t: &[u8]) -> [u8; 8] {
        t[..8].try_into().unwrap()
    }

    fn m3(inflated: u64, stored: u64, cum16: u64, flag: u64) -> u64 {
        (flag << 60)
            | (cum16 << 32)
            | (((0x10000 - stored) & 0xFFFF) << 16)
            | ((0x10000 - inflated) & 0xFFFF)
    }

    fn m2(units_inflated: u64, units_aligned: u64, pad: u64, flag: u64) -> u64 {
        (flag << 62) | (pad << 58) | (units_aligned << 32) | units_inflated
    }

    #[test]
    fn a_mode_3_table_decodes_and_authors_back_bit_for_bit() {
        // A full 64 KiB zlib chunk, a 64 KiB stored chunk, then a short zlib tail.
        let words = [
            m3(0x10000, 30001, 0, 3),
            m3(0x10000, 0x10000, 30016 / 16, 0),
            m3(1234, 700, (30016 + 0x10000) / 16, 3),
        ];
        let t = table(3, 3, 8, &words);
        let decoded = decode(cell(&t), &t).expect("decodes");
        assert_eq!(decoded.mode, 3);
        assert_eq!(
            decoded.chunks,
            [
                Chunk {
                    inflated: 0x10000,
                    stored: 30001,
                    zlib: true
                },
                Chunk {
                    inflated: 0x10000,
                    stored: 0x10000,
                    zlib: false
                },
                Chunk {
                    inflated: 1234,
                    stored: 700,
                    zlib: true
                },
            ]
        );
        let u1 = LE::read_u32(&t[4..]);
        assert_eq!(author(&decoded).unwrap(), (u1, words.to_vec()));
    }

    #[test]
    fn a_mode_2_table_decodes_and_authors_back_bit_for_bit() {
        // A 128 KiB zlib chunk stored in 4001 bytes (pad 15 to 4016), then a 48-byte stored chunk.
        let words = [m2(0x20000 / 16, 4016 / 16, 15, 3), m2(3, 3, 0, 0)];
        let t = table(2, 2, 0, &words);
        let decoded = decode(cell(&t), &t).expect("decodes");
        assert_eq!(decoded.mode, 2);
        assert_eq!(
            decoded.chunks,
            [
                Chunk {
                    inflated: 0x20000,
                    stored: 4001,
                    zlib: true
                },
                Chunk {
                    inflated: 48,
                    stored: 48,
                    zlib: false
                },
            ]
        );
        let u1 = LE::read_u32(&t[4..]);
        assert_eq!(author(&decoded).unwrap(), (u1, words.to_vec()));
    }

    #[test]
    fn a_mode_2_table_of_small_chunks_authors_back_bit_for_bit() {
        // The 20-byte row shape: mode 2 although every chunk inflates to 64 KiB or less.
        let words = [m2(4, 2, 0, 3), m2(0x1000, 0x400, 7, 3)];
        let t = table(2, 2, 0, &words);
        let decoded = decode(cell(&t), &t).expect("decodes");
        assert_eq!(decoded.mode, 2);
        assert!(decoded.chunks.iter().all(|c| c.inflated <= 0x10000));
        assert_eq!(
            author(&decoded).unwrap(),
            (LE::read_u32(&t[4..]), words.to_vec())
        );
    }

    fn m4(inflated: u64, units_aligned: u64, pad: u64, flag: u64) -> u64 {
        (flag << 56) | (pad << 52) | (units_aligned << 28) | inflated
    }

    #[test]
    fn a_mode_4_table_decodes_and_authors_back_bit_for_bit() {
        // A 1 MiB zlib chunk in 70001 bytes (pad 15), an odd-sized stored chunk, an odd zlib tail.
        let words = [
            m4(0x10_0000, 70016 / 16, 15, 3),
            m4(1000, 1008 / 16, 8, 0),
            m4(0x0FFF_FFFF, 0xFF_FFFF, 1, 3),
        ];
        let t = table(4, 3, 8, &words);
        let decoded = decode(cell(&t), &t).expect("decodes");
        assert_eq!(decoded.mode, 4);
        assert_eq!(
            decoded.chunks,
            [
                Chunk {
                    inflated: 0x10_0000,
                    stored: 70001,
                    zlib: true
                },
                Chunk {
                    inflated: 1000,
                    stored: 1000,
                    zlib: false
                },
                Chunk {
                    inflated: 0x0FFF_FFFF,
                    stored: 0x0FFF_FFEF,
                    zlib: true
                },
            ]
        );
        assert_eq!(
            author(&decoded).unwrap(),
            (LE::read_u32(&t[4..]), words.to_vec())
        );
    }

    #[test]
    fn a_malformed_mode_4_table_is_refused() {
        let ok = m4(4000, 63, 8, 3);
        assert!(decode(cell(&table(4, 1, 0, &[ok])), &table(4, 1, 0, &[ok])).is_ok());
        let refused = |t: Vec<u8>| decode(cell(&t), &t).is_err();
        for bit in 58..64 {
            assert!(
                refused(table(4, 1, 0, &[ok | (1 << bit)])),
                "reserved bit {bit}"
            );
        }
        assert!(
            refused(table(4, 1, 0, &[m4(4000, 63, 8, 1)])),
            "unknown flag"
        );
        assert!(
            refused(table(4, 1, 0, &[m4(2000, 63, 8, 0)])),
            "a stored chunk that grows"
        );
        assert!(
            refused(table(4, 1, 0, &[m4(16, 0, 3, 3)])),
            "padding past the stored size"
        );
        assert!(
            refused(table(4, 2, 0, &[ok])),
            "a count that overruns the heap"
        );
        let of = |c: Chunk| Table {
            mode: 4,
            chunks: vec![c],
        };
        let too_big = Chunk {
            inflated: 0x1000_0000,
            stored: 9000,
            zlib: true,
        };
        assert!(
            author(&of(too_big)).is_err(),
            "mode 4 states at most 28 bits inflated"
        );
        let too_long = Chunk {
            inflated: 100,
            stored: 0x1000_0000,
            zlib: true,
        };
        assert!(
            author(&of(too_long)).is_err(),
            "mode 4 states at most 24 bits of stored units"
        );
    }

    #[test]
    fn a_malformed_table_is_refused() {
        let ok3 = [m3(100, 50, 0, 3)];
        let ok2 = [m2(4, 2, 0, 3)];
        assert!(decode(cell(&table(3, 1, 0, &ok3)), &table(3, 1, 0, &ok3)).is_ok());
        let refused = |t: Vec<u8>| decode(cell(&t), &t).is_err();
        // An unknown mode.
        assert!(refused(table(5, 1, 0, &ok3)));
        assert!(refused(table(1, 1, 0, &ok3)));
        // A reserved bit in either mode.
        assert!(refused(table(3, 1, 0, &[ok3[0] | (1 << 62)])));
        assert!(refused(table(2, 1, 0, &[ok2[0] | (1 << 24)])));
        assert!(refused(table(2, 1, 0, &[ok2[0] | (1 << 56)])));
        // A count that overruns the heap.
        assert!(refused(table(3, 2, 0, &ok3)));
        // A cell mark other than 0x08.
        let mut t = table(3, 1, 0, &ok3);
        t[6] = 0x09;
        assert!(refused(t));
        // A mode-3 stored offset its chunks do not sum to, an unknown flag, a stored chunk that grows.
        assert!(refused(table(3, 1, 0, &[m3(100, 50, 1, 3)])));
        assert!(refused(table(3, 1, 0, &[m3(100, 50, 0, 1)])));
        assert!(refused(table(2, 1, 0, &[m2(4, 2, 0, 0)])));
        // Padding past the stored size.
        assert!(refused(table(2, 1, 0, &[m2(4, 0, 3, 3)])));
    }

    #[test]
    fn a_size_the_mode_cannot_hold_is_refused_when_authoring() {
        let odd = Chunk {
            inflated: 1000,
            stored: 1000,
            zlib: false,
        };
        let of = |mode: u8, c: Chunk| Table {
            mode,
            chunks: vec![c],
        };
        assert!(
            author(&of(2, odd)).is_err(),
            "mode 2 states sizes in 16-byte units"
        );
        let big = Chunk {
            inflated: 0x10010,
            stored: 9000,
            zlib: true,
        };
        assert!(author(&of(3, big)).is_err(), "mode 3 states at most 64 KiB");
        assert!(author(&of(5, big)).is_err(), "no mode 5");
        assert!(author(&of(4, big)).is_ok());
        assert!(author(&of(2, big)).is_ok());
        let empty = Chunk {
            inflated: 0,
            stored: 0,
            zlib: true,
        };
        assert!(author(&of(3, empty)).is_err(), "mode 3 cannot state 0");
        let unaligned = Chunk {
            inflated: 0x10008,
            stored: 9000,
            zlib: true,
        };
        assert!(author(&of(2, unaligned)).is_err());
    }

    #[test]
    fn the_reader_finds_an_entry_cell_through_the_index() {
        let words = [m3(4000, 900, 0, 3)];
        let archive = one_entry_archive(&table(3, 1, 0, &words));
        assert_eq!(cell_offset(&archive, 0), Some(0x60));
        assert_eq!(
            entry_chunks(&archive, 0).unwrap(),
            Some(vec![Chunk {
                inflated: 4000,
                stored: 900,
                zlib: true
            }])
        );
        // No row, and an index past the entry count.
        assert_eq!(entry_chunks(&archive, 1).unwrap(), None);
        assert_eq!(cell_offset(&archive, 2), None);
    }
}
