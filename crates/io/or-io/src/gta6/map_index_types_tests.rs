use super::*;

fn block(data_type: u32, data: Vec<u8>) -> RbmDataBlock {
    RbmDataBlock {
        data_type,
        data,
        data_addr: 0,
    }
}

/// A pointer to `offset` in the block of the 1-based id `id`.
fn pointer(id: u64, offset: u64) -> u64 {
    id | offset << 20
}

/// A map-types body of a base archetype, a time archetype on from `hours`, and a second base
/// archetype in the base archetypes' block: its root, the pointers, then the two blocks of records.
fn types_body(hours: u32) -> Vec<RbmDataBlock> {
    let mut root = vec![0u8; 0x50];
    // The pointers start 8 bytes into their block.
    root[ARCHETYPES..ARCHETYPES + 8].copy_from_slice(&pointer(2, 8).to_le_bytes());
    root[ARCHETYPES + 8..ARCHETYPES + 10].copy_from_slice(&3u16.to_le_bytes());
    let mut list = vec![0u8; 8];
    for raw in [pointer(3, 0), pointer(4, 0x10), pointer(3, 0xC0)] {
        list.extend(raw.to_le_bytes());
    }
    let mut timed = vec![0u8; 0x10 + 0xC0];
    timed[0x10 + 0xB8..0x10 + 0xBC].copy_from_slice(&hours.to_le_bytes());
    vec![
        block(map::C_MAP_TYPES, root),
        block(0, list),
        block(lit(b"CBaseArchetypeDef"), vec![0xEE; 0x180]),
        block(TIME_ARCHETYPE, timed),
    ]
}

#[test]
fn a_time_archetype_is_on_in_its_hours_and_any_other_in_all() {
    let body = types_body(0xFF00_0F0F);
    // The bits past the 24 hours are no hour.
    assert_eq!(
        hours_in(&body, 1, true),
        Some(vec![ALL_HOURS, 0x0000_0F0F, ALL_HOURS])
    );
    // A class table that does not put the hours there gives no hours to trust.
    assert_eq!(hours_in(&body, 1, false), None);
    let mut none = types_body(0);
    none[0].data[ARCHETYPES + 8] = 0;
    assert_eq!(hours_in(&none, 1, false), Some(Vec::new()));
}

#[test]
fn a_map_types_body_whose_pointers_lead_nowhere_gives_no_hours() {
    let body = types_body(1);
    assert_eq!(hours_in(&body, 0, true), None, "no root");
    assert_eq!(hours_in(&body, 9, true), None, "a root past the blocks");
    let mut far = types_body(1);
    far[0].data[ARCHETYPES + 8] = 4;
    assert_eq!(
        hours_in(&far, 1, true),
        None,
        "more pointers than the block"
    );
    let mut cut = types_body(1);
    cut[3].data.truncate(0x10 + 0xBA);
    assert_eq!(hours_in(&cut, 1, true), None, "a record cut short");
    let mut wild = types_body(1);
    wild[1].data[8..16].copy_from_slice(&pointer(7, 0).to_le_bytes());
    assert_eq!(hours_in(&wild, 1, true), None, "a block that is not there");
    let mut odd = types_body(1);
    odd[1].data[8..16].copy_from_slice(&(pointer(3, 0) | 0x1000).to_le_bytes());
    assert_eq!(hours_in(&odd, 1, true), None, "bits 12 to 19 set");
}

#[test]
fn a_map_s_group_is_the_word_at_its_place_in_the_root() {
    let mut root = vec![0u8; 0x100];
    root[0x7C..0x80].copy_from_slice(&0xABCD_1234u32.to_le_bytes());
    root[0x48..0x50].copy_from_slice(&0x0123_4567_89AB_CDEFu64.to_le_bytes());
    let blocks = vec![block(map::C_MAP_DATA, vec![0; 4]), block(0, root)];
    assert_eq!(group_in(&blocks, 2), Some(0xABCD_1234));
    assert_eq!(group_in(&blocks, 1), None, "a root shorter than the member");
    assert_eq!(group_in(&blocks, 3), None);
    assert_eq!(word_in(&blocks, 2), Some(0x0123_4567_89AB_CDEF));
    assert_eq!(word_in(&blocks, 1), None);
}

#[test]
fn an_archetype_s_load_flags_are_the_low_half_where_its_class_states_them() {
    let mut body = types_body(0);
    let flags = 0x0001_0081u64 | 1 << 40;
    body[2].data[0x30..0x38].copy_from_slice(&flags.to_le_bytes());
    body[2].data[0xC0 + 0x30..0xC0 + 0x38].copy_from_slice(&1u64.to_le_bytes());
    body[3].data[0x10 + 0x30..0x10 + 0x38].copy_from_slice(&0x80u64.to_le_bytes());
    let base = lit(b"CBaseArchetypeDef");
    assert_eq!(
        load_flags_in(&body, 1, &|class| class == base || class == TIME_ARCHETYPE),
        Some(vec![0x0001_0081, 0x80, 1])
    );
    // A class whose table puts them elsewhere reads as none.
    assert_eq!(
        load_flags_in(&body, 1, &|class| class == base),
        Some(vec![0x0001_0081, 0, 1])
    );
    assert_eq!(load_flags_in(&body, 0, &|_| true), None, "no root");
}
