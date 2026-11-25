use super::*;

/// `table` with `key` placed as the game places it: the first free slot of the first group with
/// room, from its home group on.
fn insert(table: &mut TextTable, key: u32, item: TextItem) {
    let mask = table.groups - 1;
    let mut group = (key >> 7) & mask;
    loop {
        let used = table.entries.iter().filter(|e| e.group == group).count();
        if used < SLOTS {
            let at = table.entries.partition_point(|e| e.group <= group);
            table.entries.insert(
                at,
                TextEntry {
                    key,
                    group,
                    slot: used as u8,
                    item,
                },
            );
            return;
        }
        group = (group + 1) & mask;
    }
}

fn text(t: &str) -> TextItem {
    TextItem::Text(t.to_string())
}

fn forms() -> TextItem {
    let form = |text: &str, gender, plurality, case, seed: u8| Variant {
        text: text.to_string(),
        gender,
        plurality,
        case,
        hash_34a643c5: std::array::from_fn(|i| [seed + i as u8, seed + 2 * i as u8]),
    };
    TextItem::Variants(Variants {
        gender: 2,
        plurality: 1,
        hash_27d7357f: 10,
        hash_e0237fcf: 1,
        forms: vec![
            form("le chien ~a~", 1, 1, 0, 3),
            form("la chienne", 2, 3, 12, 40),
        ],
    })
}

/// A table of 4 groups whose home group 1 overflows into group 2, with a label-hashed key, a
/// variant entry and texts in several scripts.
fn table() -> TextTable {
    let mut t = TextTable {
        groups: 4,
        entries: Vec::new(),
    };
    for i in 0..17u32 {
        let key = 0x4A00_0000 | 1 << 7 | i << 9 | (i * 5 + 3) & 0x7F;
        insert(
            &mut t,
            key,
            text(&format!("Ligne {i} — 東京 {}", "é".repeat(i as usize))),
        );
    }
    insert(
        &mut t,
        string_hash(b"MADE_UP_LABEL_ONE"),
        text("~INPUT_JUMP~ to jump"),
    );
    insert(&mut t, 0x0000_0181, forms());
    insert(&mut t, 0x7FFF_FE7F, text(""));
    t
}

#[test]
fn a_written_table_reads_back_to_itself_and_writes_again_byte_identical() {
    let t = table();
    assert!(
        t.entries
            .iter()
            .any(|e| e.group == 2 && (e.key >> 7) & 3 == 1),
        "one key is displaced"
    );
    let section = t.to_section().expect("written");
    let (back, layout) = read(&section).expect("it reads");
    assert_eq!(back, t);
    assert_eq!(back.to_section().expect("again"), section);
    assert_eq!(layout.virtual_blocks, 1);
    assert_eq!(layout.length % 16, 0);
    assert!(layout.length <= STREAM);
    let kinds: Vec<u32> = layout.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(kinds, [GROUP, MAP, ENTRY, VARIANT_ENTRY, VARIANT, CHARS]);
    let get = |label: &str| back.get(label).map(|e| &e.item);
    assert_eq!(
        get("made_up_label_one"),
        Some(&text("~INPUT_JUMP~ to jump"))
    );
    assert_eq!(get("MADE_UP_LABEL_TWO"), None);
    let displaced = back
        .entries
        .iter()
        .find(|e| e.group == 2)
        .expect("displaced");
    assert_eq!(back.find(displaced.key), Some(displaced));
    assert_eq!(back.find(0x0000_0181).map(|e| &e.item), Some(&forms()));
}

/// `table()` without its variant entry, which is the last key of its group.
fn plain_table() -> TextTable {
    let mut t = table();
    t.entries.retain(|e| matches!(e.item, TextItem::Text(_)));
    t
}

/// `t`, a table of plain texts, with its blocks numbered as the game numbers them (entries, map,
/// groups, characters) and a byte no rule reads set after the block map.
fn as_the_game_lays_it(t: &TextTable) -> Vec<u8> {
    let (chars, places) = t.pack(usize::MAX).expect("packed");
    let records = t.record_blocks();
    let size = |kind| records.iter().find(|k| k.0 == kind).expect("a block").1;
    let mut kinds = vec![(ENTRY, size(ENTRY)), (MAP, 16), (GROUP, size(GROUP))];
    kinds.extend(chars.iter().map(|c| (CHARS, c.len())));
    let data = t.data_blocks(&kinds, &chars, &places).expect("blocks");
    let mut section = lay_out(&kinds, &data, false).expect("laid out");
    let bm = (u64::from_le_bytes(section[0x08..0x10].try_into().unwrap()) - VBASE) as usize;
    section[bm + 16] = 0xAB;
    section
}

#[test]
fn a_table_written_over_its_own_section_is_byte_identical_where_one_page_is_not() {
    let t = plain_table();
    let game = as_the_game_lays_it(&t);
    let (back, _) = read(&game).expect("it reads");
    assert_eq!(back, t);
    assert_ne!(back.to_section().expect("one page"), game);
    assert_eq!(back.to_section_over(&game).expect("over"), game);

    let variants = table().to_section().expect("written");
    let back = read(&variants).expect("it reads").0;
    assert_eq!(back.to_section_over(&variants).expect("over"), variants);
}

#[test]
fn an_edit_that_keeps_the_texts_lengths_keeps_the_section_and_changes_only_their_bytes() {
    let t = plain_table();
    let game = as_the_game_lays_it(&t);
    let mut edited = t.clone();
    let at = (edited.entries.iter())
        .position(|e| e.item == text("~INPUT_JUMP~ to jump"))
        .expect("the text");
    edited.entries[at].item = text("~INPUT_JUMP~ to hop!");
    let out = edited.to_section_over(&game).expect("written");
    assert_eq!(out.len(), game.len());
    let changed = out.iter().zip(&game).filter(|(a, b)| a != b).count();
    let differ = "jump".bytes().zip("hop!".bytes()).filter(|(a, b)| a != b);
    assert_eq!(changed, differ.count());
    assert_eq!(read(&out).expect("it reads").0, edited);

    // One text a byte shorter and another a byte longer fill the block as before.
    let mut traded = t.clone();
    for (from, to) in [
        ("Ligne 2 — 東京 éé", "Ligne 2 — 東京 éx"),
        ("Ligne 3 — 東京 ééé", "Ligne 3 — 東京 éééy"),
    ] {
        let e = (traded.entries.iter_mut())
            .find(|e| e.item == text(from))
            .expect("the text");
        e.item = text(to);
    }
    let out = traded.to_section_over(&game).expect("written");
    assert_eq!(out.len(), game.len());
    assert_eq!(read(&out).expect("it reads").0, traded);
}

#[test]
fn an_edit_that_changes_a_block_s_size_is_refused_over_the_section() {
    let t = plain_table();
    let game = as_the_game_lays_it(&t);
    let mut longer = t.clone();
    longer.entries[0].item = text("Ligne 0 — 東京, longer");
    let err = longer.to_section_over(&game).expect_err("refused");
    assert!(err.to_string().contains("character blocks"), "{err}");

    let mut added = t.clone();
    insert(&mut added, 0x0000_0201, text(""));
    let err = added.to_section_over(&game).expect_err("refused");
    assert!(
        err.to_string().contains("as many as the section holds"),
        "{err}"
    );
    assert!(matches!(
        t.to_section_over(&game[..0x40]),
        Err(TextTableError::NotText(_))
    ));
}

#[test]
fn texts_past_one_character_block_go_to_the_first_block_with_room() {
    let mut t = TextTable {
        groups: 2,
        entries: Vec::new(),
    };
    // 600,000 + 600,000 do not fit one block, so the second opens a block and the small third
    // goes back into the first.
    for (key, len) in [(0x11u32, 600_000), (0x92, 600_000), (0x13, 1_000)] {
        insert(&mut t, key, text(&"x".repeat(len)));
    }
    let section = t.to_section().expect("written");
    let (back, layout) = read(&section).expect("it reads");
    assert_eq!(back, t);
    let chars: Vec<usize> = (layout.blocks.iter())
        .filter(|b| b.kind == CHARS)
        .map(|b| b.size)
        .collect();
    assert_eq!(chars, [601_002, 600_001]);
    assert_eq!(layout.length % STREAM, 0);
    let phase_a = (layout.blocks.iter().map(|b| b.offset + b.size))
        .max()
        .unwrap_or(0);
    assert!(layout.word_4e as usize >= phase_a.div_ceil(PAGE));
    assert_eq!(back.to_section().expect("again"), section);
}

#[test]
fn a_written_table_saves_as_a_loose_resource_that_opens_as_the_same_table() {
    use crate::container::rpf::{read_rpf9_body, write_rpf9_resource, Rpf9Form, Rpf9Sections};
    let t = table();
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: t.to_section().expect("written"),
        physical: Vec::new(),
    };
    let file = write_rpf9_resource(&sections).expect("a loose resource");
    let (find, bytes) = read_rpf9_body(std::io::Cursor::new(&file), 0, u64::MAX).expect("it reads");
    let again = Rpf9Sections::of(&find, &bytes).expect("a resource");
    assert_eq!(again, sections);
    assert_eq!(read(&again.virtual_section).expect("a table").0, t);
}

#[test]
fn an_edit_the_bytes_cannot_hold_is_refused_by_the_writer() {
    let mut t = table();
    t.entries[0].item = text("a\0b");
    assert!(matches!(t.to_section(), Err(TextTableError::Text { why, .. }) if why.contains("NUL")));
    t.entries[0].item = text(&"y".repeat(CHAR_BLOCK));
    assert!(
        matches!(t.to_section(), Err(TextTableError::Text { why, .. }) if why.contains("longer"))
    );

    let mut t = table();
    t.entries[3].slot = 16;
    assert!(matches!(t.to_section(), Err(TextTableError::Unwritable(_))));
    let mut t = table();
    let key = t.entries[0].key;
    t.entries[1].key = key;
    assert!(matches!(t.to_section(), Err(TextTableError::Unwritable(_))));
    let mut t = table();
    t.entries.swap(0, 1);
    assert!(matches!(t.to_section(), Err(TextTableError::Unwritable(_))));
    let mut t = table();
    let TextItem::Variants(v) = &mut t
        .entries
        .iter_mut()
        .find(|e| e.key == 0x181)
        .expect("it")
        .item
    else {
        panic!("variants");
    };
    v.forms[1].case = 16;
    assert!(matches!(t.to_section(), Err(TextTableError::Unwritable(_))));
}

/// The section offset of the group block and of the block table in `section`.
fn places(section: &[u8]) -> (usize, usize) {
    let (_, layout) = read(section).expect("it reads");
    let groups = layout
        .blocks
        .iter()
        .find(|b| b.kind == GROUP)
        .expect("groups")
        .offset;
    let bt = (u64::from_le_bytes(section[0x38..0x40].try_into().unwrap()) - VBASE) as usize;
    (groups, bt)
}

#[test]
fn a_body_that_breaks_a_measured_rule_is_refused() {
    let section = table().to_section().expect("written");
    let (groups, bt) = places(&section);
    let class_table =
        (u64::from_le_bytes(section[0x28..0x30].try_into().unwrap()) - VBASE) as usize;
    let chars = read(&section)
        .expect("it reads")
        .1
        .blocks
        .last()
        .expect("chars")
        .offset;
    type Patch = Box<dyn Fn(&mut Vec<u8>)>;
    let cases: Vec<(Patch, &str)> = vec![
        (Box::new(move |v| v[groups] ^= 1), "control byte"),
        (
            Box::new(move |v| v[groups + 16 + 16 * 15 + 3] = 1),
            "is not zero",
        ),
        (Box::new(|v| v[0x4E] ^= 1), "0x4E"),
        (Box::new(|v| v[0x14] = 0x79), "0x14"),
        (
            Box::new(move |v| v[class_table + 4] ^= 1),
            "differs from the one",
        ),
        (Box::new(move |v| v[bt + 4] ^= 0x10), "overlap"),
        (
            Box::new(move |v| v[bt + 16 + 4] = 8),
            "map block is not 16 bytes",
        ),
        (Box::new(move |v| v[chars] = 0xFF), "not UTF-8"),
        (Box::new(move |v| v[groups + 16] ^= 1), "low 7 bits"),
        (Box::new(move |v| v[groups + 16] ^= 0x80), "probing"),
        (Box::new(|v| v[0x20] = 9), "root names block"),
    ];
    for (patch, want) in cases {
        let mut v = section.clone();
        patch(&mut v);
        let err = read(&v).expect_err(want).to_string();
        assert!(err.contains(want), "{want}: {err}");
    }
    let mut not_text = section.clone();
    not_text[0x10] = b'1';
    assert!(matches!(read(&not_text), Err(TextTableError::NotText(_))));
    let mut no_map = section.clone();
    no_map[class_table..class_table + 4].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    assert!(matches!(read(&no_map), Err(TextTableError::NotText(_))));
}

#[test]
fn every_truncation_is_an_error_and_never_a_panic() {
    let section = table().to_section().expect("written");
    // The string table is the last thing a table states; the pad after it is not read.
    let end = (u64::from_le_bytes(section[0x40..0x48].try_into().unwrap()) - VBASE) as usize + 4;
    for len in 0..end {
        assert!(read(&section[..len]).is_err(), "{len} bytes read");
    }
}

#[test]
fn a_class_table_declares_the_map_when_one_of_its_records_names_it() {
    let mut records = vec![0u8; 64];
    assert!(!declares_map(&records));
    records[32..36].copy_from_slice(&MAP.to_le_bytes());
    assert!(declares_map(&records));
    assert!(!declares_map(&records[..60]));
}
