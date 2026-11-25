use byteorder::{ByteOrder, LE};

use super::*;
use crate::container::rpf::rpf9_toc::Rpf9Group;

const K0: u64 = 0x30;
const K1: u64 = 0x10_0000_0000;
const K2: u64 = 0xFFFF_FFFF_FFFF_FFF0;
/// Where [`toc`] puts the type-4 payload.
const TABLE_AT: usize = 0x60;

/// A decrypted TOC with a kind-1 and a kind-4 descriptor, `table` as the type-4 payload, one
/// `0x97c2` column of `cells` with `heap` right after them, and the 1-byte entry index `index`.
fn toc(table: &[u8], cells: &[ListCell], heap: &[u8], index: &[u8]) -> Rpf9Toc {
    let mut b = vec![0u8; TABLE_AT];
    LE::write_u16(&mut b[0x1A..], 2);
    LE::write_u16(&mut b[0x40..], 1); // kind 1 at metadata offset 0
    LE::write_u16(&mut b[0x48..], TABLE_KIND);
    LE::write_u32(&mut b[0x4C..], (TABLE_AT - 0x50) as u32);
    b.extend_from_slice(table);
    let metadata_len = b.len() - 0x50;
    LE::write_u32(&mut b[0x1C..], metadata_len as u32);
    LE::write_u16(&mut b[0x20..], 1);
    LE::write_u32(&mut b[0x24..], index.len() as u32);
    b[0x2C] = 1;
    // The group record, which `read` takes from `groups`, then the index.
    b.resize(round16(b.len()) + 24, 0);
    b.extend_from_slice(index);
    let cells_at = (b.len() + 7) & !7;
    b.resize(cells_at, 0);
    let heap_at = cells_at + 8 * cells.len();
    for (r, cell) in cells.iter().enumerate() {
        b.extend_from_slice(&cell.bytes(cells_at + 8 * r, heap_at).expect("a cell"));
    }
    b.extend_from_slice(heap);
    let split = b.len() as u32;
    LE::write_u32(&mut b[0x08..], split); // no external column
    Rpf9Toc {
        bytes: b,
        key_id: 0,
        registry_id: 0,
        entry_count: index.len() as u32,
        groups: vec![Rpf9Group {
            name_hash: GROUP_DEPENDENCIES,
            count: cells.len() as u32,
            width: 8,
            external: false,
            data: cells_at..heap_at,
        }],
    }
}

/// Three keys, width 1; row 0 holds keys 0 and 2, row 1 key 1; written by hand, not by `author`.
fn hand_built() -> Authored {
    let mut table = vec![3, 0, 0, 0, 1, 0, 0, 0];
    for k in [K0, K1, K2] {
        table.extend_from_slice(&k.to_le_bytes());
    }
    Authored {
        table,
        cells: vec![
            ListCell {
                offset: 0,
                u1: (1 << 29) | 2,
            },
            ListCell {
                offset: 2,
                u1: (1 << 29) | 1,
            },
        ],
        heap: vec![0, 2, 1],
    }
}

fn good() -> Rpf9Toc {
    let a = hand_built();
    toc(&a.table, &a.cells, &a.heap, &[1, 0xFF, 0, 1])
}

#[test]
fn a_hand_built_table_reads_and_authors_back_bit_for_bit() {
    let deps = read(&good()).expect("reads").expect("has lists");
    assert_eq!(deps.keys, [K0, K1, K2]);
    assert_eq!(deps.width, 1);
    assert_eq!(deps.lists, [vec![K0, K2], vec![K1]]);
    assert_eq!(deps.entry_rows, [Some(1), None, Some(0), Some(1)]);
    assert_eq!(
        deps.entry(0),
        Some(&[K1][..]),
        "entries 0 and 3 share row 1"
    );
    assert_eq!(deps.entry(1), None);
    assert_eq!(deps.entry(4), None);
    assert_eq!(author(&deps.lists).expect("authors"), hand_built());
    // An unordered list with a repeat is the same key set.
    assert_eq!(
        author(&[vec![K2, K0, K2], vec![K1]]).expect("authors"),
        hand_built()
    );
}

#[test]
fn the_index_widens_past_256_keys_and_round_trips() {
    for (n, width) in [(256u64, 1u8), (257, 2)] {
        let keys: Vec<u64> = (0..n).map(|k| k * 7 + 1).collect();
        let lists = vec![keys[..200].to_vec(), keys[100..].to_vec(), vec![keys[3]]];
        let a = author(&lists).expect("authors");
        assert_eq!(a.table[4], width, "{n} keys");
        assert_eq!(a.cells[1].u1 >> 29, u32::from(width));
        assert_eq!(
            a.heap.len(),
            (200 + (n as usize - 100) + 1) * usize::from(width)
        );
        let deps = read(&toc(&a.table, &a.cells, &a.heap, &[2, 0, 1]))
            .expect("reads")
            .expect("has lists");
        assert_eq!((deps.width, &deps.keys), (width, &keys));
        assert_eq!(deps.lists, lists);
        assert_eq!(author(&deps.lists).expect("authors"), a);
    }
}

#[test]
fn a_table_with_neither_reads_as_none() {
    let mut t = good();
    LE::write_u16(&mut t.bytes[0x48..], 2);
    t.groups[0].name_hash = 1;
    assert_eq!(read(&t).expect("reads"), None);
}

/// Whether `read` refuses [`good`] after `edit`.
fn refused(edit: impl FnOnce(&mut Rpf9Toc)) -> bool {
    let mut t = good();
    edit(&mut t);
    read(&t).is_err()
}

#[test]
fn a_malformed_table_or_list_is_refused() {
    let heap_at = good().groups[0].data.end;
    let u1_at = good().groups[0].data.start + 4;
    assert!(!refused(|_| {}));
    // The table head: an unknown width, a reserved byte, more keys than the metadata holds.
    assert!(refused(|t| t.bytes[TABLE_AT + 4] = 3));
    assert!(refused(|t| t.bytes[TABLE_AT + 5] = 1));
    assert!(refused(|t| LE::write_u32(&mut t.bytes[TABLE_AT..], 4)));
    // Keys out of order.
    assert!(refused(|t| LE::write_u64(&mut t.bytes[TABLE_AT + 8..], K1)));
    // Two type-4 descriptors, or one past the metadata.
    assert!(refused(|t| LE::write_u16(&mut t.bytes[0x40..], TABLE_KIND)));
    assert!(refused(|t| LE::write_u32(&mut t.bytes[0x4C..], 0x1000)));
    // The column without the table, the table without the column, a column of the wrong width.
    assert!(refused(|t| LE::write_u16(&mut t.bytes[0x48..], 2)));
    assert!(refused(|t| t.groups[0].name_hash = 1));
    assert!(refused(|t| t.groups[0].width = 4));
    // A cell of another width, an index past the table, a list out of order, one past the bytes.
    assert!(refused(|t| LE::write_u32(
        &mut t.bytes[u1_at..],
        (2 << 29) | 2
    )));
    assert!(refused(|t| t.bytes[heap_at + 1] = 3));
    assert!(refused(|t| t.bytes[heap_at] = 2));
    assert!(refused(|t| LE::write_u32(
        &mut t.bytes[u1_at..],
        (1 << 29) | 100
    )));
    // An entry index width the table cannot have.
    assert!(refused(|t| t.bytes[0x2C] = 3));
}

#[test]
fn a_width_1_table_of_257_keys_is_refused() {
    let keys: Vec<u64> = (0..257).collect();
    let a = author(&[keys]).expect("authors at width 2");
    let mut t = toc(&a.table, &a.cells, &a.heap, &[0]);
    assert!(read(&t).is_ok());
    t.bytes[TABLE_AT + 4] = 1;
    assert!(read(&t).is_err(), "a 1-byte index cannot reach 257 keys");
}

#[test]
fn what_the_writer_cannot_state_is_refused() {
    assert!(
        author(&[vec![K0], vec![]]).is_err(),
        "an empty list has no row"
    );
    let wide: Vec<u64> = (0..=MAX_AUTHORED_KEYS as u64).collect();
    assert!(author(&[wide]).is_err(), "width 4 is not authored");
    let cell = ListCell { offset: 0, u1: 1 };
    assert!(cell.bytes(0x100, 0x80).is_err(), "a list before its cell");
    assert!(cell.bytes(0x80, 0x100).is_ok());
}
