//! Unit tests for the index decode and the name resolver, on hand-built tables with no game bytes.

use super::*;
use crate::gta6::names::Vocabulary;

/// Write the `[A, B, C]` of a 24-byte entry row from a start (bytes), class, stored (bytes) and real.
fn row24(start: u64, class: u8, stored: u64, real: u64) -> [u8; 24] {
    let a = (u64::from(class) << 40) | (start / 16);
    let b = stored / 16;
    let mut row = [0u8; 24];
    row[0..8].copy_from_slice(&a.to_le_bytes());
    row[8..16].copy_from_slice(&b.to_le_bytes());
    row[16..24].copy_from_slice(&real.to_le_bytes());
    row
}

/// A hand-built decrypted TOC of three entries whose name64 column has two rows (deduplicated) and
/// whose per-entry index is not the identity: entry 1 shares entry 0's name64 row, and takes the
/// second entry row. A reader that paired column row `i` with entry `i` would give entry 1 the wrong
/// name and the wrong row; the index array is the only way to join them.
///
/// Layout: header `[0,0x40)`, an unused group directory `[0x40,0x70)`, the main index array at 0x70
/// (2 columns per entry, 1 byte each), the name64 rows at 0x80 and the 24-byte rows at 0x90.
fn deduplicated_toc(hash_a: u64, hash_b: u64) -> Rpf9Toc {
    let mut bytes = vec![0u8; 0x100];
    let len = bytes.len() as u32;
    // Header fields the index arrays are placed by: no metadata descriptors (n=0, m=0), two columns,
    // no external column, three entries, a one-byte index.
    bytes[0x08..0x0C].copy_from_slice(&len.to_le_bytes()); // split (unused here)
    bytes[0x1A..0x1C].copy_from_slice(&0u16.to_le_bytes()); // n
    bytes[0x1C..0x20].copy_from_slice(&0u32.to_le_bytes()); // m
    bytes[0x20..0x22].copy_from_slice(&2u16.to_le_bytes()); // group_count
    bytes[0x22..0x24].copy_from_slice(&0u16.to_le_bytes()); // external_count
    bytes[0x24..0x28].copy_from_slice(&3u32.to_le_bytes()); // entries
    bytes[0x2C] = 1; // index_width

    // Main index array at 0x70: per entry, [name64 row, row24 row].
    bytes[0x70..0x76].copy_from_slice(&[0, 0, 0, 1, 1, 2]);

    // name64 column rows at 0x80.
    bytes[0x80..0x88].copy_from_slice(&hash_a.to_le_bytes());
    bytes[0x88..0x90].copy_from_slice(&hash_b.to_le_bytes());

    // 24-byte entry rows at 0x90.
    bytes[0x90..0xA8].copy_from_slice(&row24(0x200, 0x0B, 0x100, 0x155));
    bytes[0xA8..0xC0].copy_from_slice(&row24(0x400, 0x00, 0x20, 0x20));
    bytes[0xC0..0xD8].copy_from_slice(&row24(0x800, 0xFF, 0x30, 0x2A));

    Rpf9Toc {
        bytes,
        key_id: 0,
        registry_id: 0,
        entry_count: 3,
        groups: vec![
            Rpf9Group {
                name_hash: GROUP_NAME64,
                count: 2,
                width: 8,
                external: false,
                data: 0x80..0x90,
            },
            Rpf9Group {
                name_hash: GROUP_ENTRY_ROW,
                count: 3,
                width: 24,
                external: false,
                data: 0x90..0xD8,
            },
        ],
    }
}

#[test]
fn the_index_array_joins_a_deduplicated_column_not_by_row_order() {
    let (a, b) = (0x1111_2222_3333_4444, 0x5555_6666_7777_8888);
    let toc = deduplicated_toc(a, b);
    let entries = read_entries(&toc);

    assert_eq!(entries.len(), 3, "one row per entry, not per column row");
    assert_eq!(entries[0].index, 0);
    // Entry 1 shares entry 0's name64 row through the index; a row-order pairing would give it `b`.
    assert_eq!(entries[0].name64, a);
    assert_eq!(
        entries[1].name64, a,
        "the deduplicated row is reached by index, not row order"
    );
    assert_eq!(entries[2].name64, b);
    // Each entry takes its own 24-byte row.
    assert_eq!(entries[0].start, Some(0x200));
    assert_eq!(entries[1].start, Some(0x400));
    assert_eq!(entries[2].start, Some(0x800));
}

#[test]
fn the_24_byte_row_decodes_start_class_stored_and_real() {
    let toc = deduplicated_toc(1, 2);
    let entries = read_entries(&toc);
    assert_eq!(entries[0].start, Some(0x200));
    assert_eq!(entries[0].class, Some(0x0B));
    assert_eq!(entries[0].stored, Some(0x100));
    assert_eq!(entries[0].real, Some(0x155));
    assert_eq!(entries[0].size(), Some(0x155));
    assert_eq!(entries[2].class, Some(0xFF));
}

#[test]
fn the_20_byte_row_yields_only_its_proven_start() {
    // One entry, one 20-byte row column, no 24-byte column: the start decodes, the sizes stay unknown.
    let mut bytes = vec![0u8; 0x100];
    bytes[0x1A..0x1C].copy_from_slice(&0u16.to_le_bytes()); // n
    bytes[0x1C..0x20].copy_from_slice(&0u32.to_le_bytes()); // m
    bytes[0x20..0x22].copy_from_slice(&1u16.to_le_bytes()); // group_count
    bytes[0x22..0x24].copy_from_slice(&0u16.to_le_bytes()); // external_count
    bytes[0x24..0x28].copy_from_slice(&1u32.to_le_bytes()); // entries
    bytes[0x2C] = 1; // index_width
                     // main_base = 0x40 + 24*1 = 0x58; one column, one entry: index[0] = row 0.
    bytes[0x58] = 0;
    // The 20-byte row at 0x60: [x, 3, size, start/16, 0xB]; start = 0x50*16.
    let row: [u32; 5] = [0xDEAD, 3, 0x99, 0x50, 0xB];
    for (i, w) in row.iter().enumerate() {
        bytes[0x60 + i * 4..0x60 + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    let toc = Rpf9Toc {
        bytes,
        key_id: 0,
        registry_id: 0,
        entry_count: 1,
        groups: vec![Rpf9Group {
            name_hash: GROUP_ENTRY_ROW_20,
            count: 1,
            width: 20,
            external: false,
            data: 0x60..0x74,
        }],
    };
    let entries = read_entries(&toc);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].start, Some(0x500));
    assert_eq!(
        entries[0].stored, None,
        "the 20-byte row's size word is not decoded"
    );
    assert_eq!(entries[0].real, None);
}

#[test]
fn an_entry_without_a_name64_row_still_lists() {
    // group_count 0: no columns at all; every entry lists with a zero hash rather than vanishing.
    let mut bytes = vec![0u8; 0x80];
    bytes[0x20..0x22].copy_from_slice(&0u16.to_le_bytes()); // group_count
    bytes[0x24..0x28].copy_from_slice(&2u32.to_le_bytes()); // entries
    bytes[0x2C] = 1; // index_width
    let toc = Rpf9Toc {
        bytes,
        key_id: 0,
        registry_id: 0,
        entry_count: 2,
        groups: vec![],
    };
    let entries = read_entries(&toc);
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e.name64 == 0 && e.start.is_none()));
}

#[test]
fn read_cipher_reads_the_record_head() {
    let mut row = [0u8; 64];
    row[0..4].copy_from_slice(&0x1000_0029u32.to_le_bytes());
    row[4..6].copy_from_slice(&17u16.to_le_bytes());
    row[6..22].copy_from_slice(&[0xAA; 16]);
    let cipher = read_cipher(&row);
    assert_eq!(cipher.key_id, 0x1000_0029);
    assert_eq!(cipher.cipher_type, 17);
    assert_eq!(cipher.iv, [0xAA; 16]);
}

#[test]
fn parse_hash_reads_both_widths_and_refuses_junk() {
    assert_eq!(parse_hash("0x12FD3D26"), Some(0x12FD3D26));
    assert_eq!(
        parse_hash("0x092DF78629F7833A"),
        Some(0x092D_F786_29F7_833A)
    );
    assert_eq!(parse_hash("12FD3D26"), None);
    assert_eq!(parse_hash("0xZZ"), None);
    assert_eq!(parse_hash("anim"), None);
    assert_eq!(parse_hash("0x"), None);
}

#[test]
fn the_resolver_names_by_name64_and_the_wrong_hash_flavour_misses() {
    use crate::gta6::hash::{name_hash32, name_hash64};
    let mut vocabulary = Vocabulary::default();
    vocabulary.add("config.dat", None);
    let resolver = NameResolver::from_vocabulary(vocabulary);
    assert_eq!(
        resolver.lookup(name_hash64(b"config.dat")),
        Some("config.dat")
    );
    // Case folding discriminates the hash flavour: name_hash64 folds ASCII case, so a mixed-case or
    // upper-case path hashes to the same word and resolves. A case-sensitive Murmur would miss these.
    assert_eq!(
        resolver.lookup(name_hash64(b"Config.Dat")),
        Some("config.dat")
    );
    assert_eq!(
        resolver.lookup(name_hash64(b"CONFIG.DAT")),
        Some("config.dat")
    );
    // The 32-bit hash widened to 64 bits is a different word and must not resolve, so a resolver
    // keyed by name32 rather than name64 would fail here.
    assert_eq!(resolver.lookup(u64::from(name_hash32(b"config.dat"))), None);
    assert!(!resolver.is_empty());
}

#[test]
fn the_shortcut_tables_resolve_and_take_precedence() {
    let mut r = NameResolver::new();
    r.add_hash_name_tsv("0x12FD3D26\tconfig.dat\n0x0019CF52\tmovies/foo.bk2\n");
    assert_eq!(r.lookup(0x12FD3D26), Some("config.dat"));
    assert_eq!(r.lookup(0x0019CF52), Some("movies/foo.bk2"));
    assert_eq!(r.lookup(0xDEAD), None);

    let text = "archive\tentry\tname64\tname32\tname\tsource\n\
                appdata0\t0\t0x092DF78629F7833A\t0x12FD3D26\tconfig.dat\tstrings\n\
                textures\t6\t0x45AF281108824644\t\t\t\n";
    r.add_toc_entries_tsv(text);
    assert_eq!(r.lookup(0x092D_F786_29F7_833A), Some("config.dat"));
    assert_eq!(r.lookup(0x45AF_2811_0882_4644), None);
}

#[test]
fn coverage_counts_the_resolved_entries() {
    let mut r = NameResolver::new();
    r.insert(10, "a.dat");
    let entries: Vec<Gta6Entry> = [10u64, 20, 10]
        .into_iter()
        .enumerate()
        .map(|(i, name64)| Gta6Entry {
            index: i as u32,
            name64,
            name32: None,
            name: None,
            start: None,
            stored: None,
            real: None,
            class: None,
            cipher: None,
        })
        .collect();
    assert_eq!(r.coverage(&entries), (2, 3));
}

#[test]
fn display_name_falls_back_to_the_hash() {
    let mut entry = Gta6Entry {
        index: 0,
        name64: 0x0123_4567_89AB_CDEF,
        name32: None,
        name: None,
        start: None,
        stored: None,
        real: None,
        class: None,
        cipher: None,
    };
    assert_eq!(entry.display_name(), "hash_0123456789abcdef");
    entry.name = Some("real/path.dat".into());
    assert_eq!(entry.display_name(), "real/path.dat");
}

/// A hand-built TOC with a main name64 column and an EXTERNAL content-hash column over two entries,
/// whose content-hash index is deduplicated: entry 1 shares entry 0's hash row through the external
/// index array, so a row-order pairing would give it the wrong hash.
///
/// Layout: header `[0,0x40)`, a group directory `[0x40,0x70)`, the main index array at 0x70, the
/// external index array at 0x80 (`split`), the name64 rows at 0x90 and the content-hash rows at 0xA0.
fn content_hash_toc(hash0: u64, hash1: u64) -> Rpf9Toc {
    let mut bytes = vec![0u8; 0xC0];
    bytes[0x08..0x0C].copy_from_slice(&0x80u32.to_le_bytes()); // split = external index base
    bytes[0x1A..0x1C].copy_from_slice(&0u16.to_le_bytes()); // n
    bytes[0x1C..0x20].copy_from_slice(&0u32.to_le_bytes()); // m
    bytes[0x20..0x22].copy_from_slice(&2u16.to_le_bytes()); // group_count (name64 + content hash)
    bytes[0x22..0x24].copy_from_slice(&1u16.to_le_bytes()); // external_count (the content hash)
    bytes[0x24..0x28].copy_from_slice(&2u32.to_le_bytes()); // entries
    bytes[0x2C] = 1; // index_width

    // Main index array at 0x70 (one main column, name64): entry e -> name64 row e.
    bytes[0x70..0x72].copy_from_slice(&[0, 1]);
    // External index array at 0x80 (one external column): entry 1 shares entry 0's content-hash row.
    bytes[0x80..0x82].copy_from_slice(&[0, 0]);

    bytes[0x90..0x98].copy_from_slice(&1u64.to_le_bytes()); // name64 rows
    bytes[0x98..0xA0].copy_from_slice(&2u64.to_le_bytes());
    bytes[0xA0..0xA8].copy_from_slice(&hash0.to_le_bytes()); // content-hash rows
    bytes[0xA8..0xB0].copy_from_slice(&hash1.to_le_bytes());

    Rpf9Toc {
        bytes,
        key_id: 0,
        registry_id: 0,
        entry_count: 2,
        groups: vec![
            Rpf9Group {
                name_hash: GROUP_NAME64,
                count: 2,
                width: 8,
                external: false,
                data: 0x90..0xA0,
            },
            Rpf9Group {
                name_hash: GROUP_CONTENT_HASH,
                count: 2,
                width: 8,
                external: true,
                data: 0xA0..0xB0,
            },
        ],
    }
}

#[test]
fn entry_extras_joins_the_external_content_hash_through_the_index() {
    let (h0, h1) = (0xAABB_CCDD_1122_3344, 0x9988_7766_5544_3322);
    let archive = Gta6Archive {
        toc: content_hash_toc(h0, h1),
        entries: Vec::new(),
    };
    let extras = archive.entry_extras();
    assert_eq!(extras.len(), 2, "one extra per entry");
    assert_eq!(extras[0].content_hash, Some(h0));
    // Entry 1's external index points at row 0, so it shares entry 0's hash, not h1 — the external
    // columns are deduplicated and joined by index, exactly like the entry columns.
    assert_eq!(extras[1].content_hash, Some(h0));
    assert!(
        extras.iter().all(|e| e.nested.is_none()),
        "no 20-byte column, so no nested descriptor"
    );
}

#[test]
fn entry_extras_decodes_the_full_20_byte_nested_descriptor() {
    // One entry, one 20-byte row column: the whole descriptor decodes, while the entry itself still
    // carries only its proven start (so a paged body is not read against the page-bucket `w0`).
    let mut bytes = vec![0u8; 0x100];
    bytes[0x20..0x22].copy_from_slice(&1u16.to_le_bytes()); // group_count
    bytes[0x24..0x28].copy_from_slice(&1u32.to_le_bytes()); // entries
    bytes[0x2C] = 1; // index_width
                     // main_base = 0x40 + 24*1 = 0x58; one column, one entry: index[0] = row 0.
    bytes[0x58] = 0;
    // The 20-byte row at 0x60: [w0 virtual, w4 flags, w8 stored pages, w12 start/16, w16 type].
    let row: [u32; 5] = [0x1234, 0x0004_0006, 0x11, 0x50, 0x58];
    for (i, w) in row.iter().enumerate() {
        bytes[0x60 + i * 4..0x60 + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    let toc = Rpf9Toc {
        bytes,
        key_id: 0,
        registry_id: 0,
        entry_count: 1,
        groups: vec![Rpf9Group {
            name_hash: GROUP_ENTRY_ROW_20,
            count: 1,
            width: 20,
            external: false,
            data: 0x60..0x74,
        }],
    };
    let archive = Gta6Archive {
        toc,
        entries: Vec::new(),
    };
    let extras = archive.entry_extras();
    assert_eq!(extras.len(), 1);
    let nested = extras[0]
        .nested
        .expect("the 20-byte row decodes to a descriptor");
    assert_eq!(nested.virtual_size, 0x1234);
    assert_eq!(nested.flags, 0x0004_0006);
    assert_eq!(nested.stored_pages, 0x11);
    assert_eq!(nested.resource_type, 0x58);
    assert_eq!(
        nested.stored_size(),
        0x11 * 16,
        "16-byte pages scaled to bytes"
    );
    assert!(extras[0].content_hash.is_none());
}

#[test]
fn the_class_byte_maps_to_an_entry_kind() {
    // deduplicated_toc builds the three 24-byte rows with class 0x0B, 0x00, 0xFF.
    let entries = read_entries(&deduplicated_toc(1, 2));
    assert_eq!(entries[0].kind(), Some(Gta6Class::CompressedResource));
    assert_eq!(entries[1].kind(), Some(Gta6Class::NestedArchive));
    assert_eq!(entries[2].kind(), Some(Gta6Class::Media));
    assert_eq!(Gta6Class::from_byte(0x0E), Gta6Class::Other(0x0E));
    // A row with no class byte (a 20-byte row or no row) has no kind.
    let rowless = Gta6Entry {
        class: None,
        ..entries[0].clone()
    };
    assert_eq!(rowless.kind(), None);
}

#[test]
fn the_content_hash_column_is_known_and_an_unknown_column_is_reported() {
    let mut toc = content_hash_toc(1, 2);
    // A genuinely unknown column: its hash and width are reported, the content hash is not.
    toc.groups.push(Rpf9Group {
        name_hash: 0x1234_5678_9ABC_DEF0,
        count: 1,
        width: 28,
        external: true,
        data: 0xB0..0xB0,
    });
    let archive = Gta6Archive {
        toc,
        entries: Vec::new(),
    };
    assert_eq!(archive.unknown_columns(), vec![(0x1234_5678_9ABC_DEF0, 28)]);
}
