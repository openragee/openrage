use super::*;

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn floats(out: &mut Vec<u8>, v: [f32; 6]) {
    v.iter()
        .for_each(|f| out.extend_from_slice(&f.to_le_bytes()));
}

const CARRIED: [u8; 7] = [0x41, 0x07, 0xC3, 0x00, 0x5A, 0x19, 0xE2];
// Tile 3 of 36 along X and 5 of 41 along Y.
const TILE_BOUNDS: [f32; 6] = [1_024.0, 2_560.0, -120.0, 1_536.0, 3_072.0, 333.0];

/// A 3 x 2 little-endian height map, built byte by byte.
fn pamh() -> Vec<u8> {
    let mut out = b"PAMH\x01".to_vec();
    out.extend_from_slice(&CARRIED);
    le16(&mut out, 3);
    le16(&mut out, 2);
    floats(&mut out, TILE_BOUNDS);
    le32(&mut out, 2 * 8 + 2 * 6);
    for row in 0..2u32 {
        le16(&mut out, 0);
        le16(&mut out, 3);
        le32(&mut out, row * 3);
    }
    out.extend_from_slice(&[9, 8, 7, 6, 5, 4]);
    out.extend_from_slice(&[1, 2, 3, 0, 1, 2]);
    out
}

#[test]
fn a_little_endian_height_map_reads_carries_its_seven_bytes_and_saves_identical() {
    let bytes = pamh();
    let map = Gta6HeightMap::open(&bytes).expect("opens");
    assert_eq!(map.carried, CARRIED);
    assert_eq!((map.grid.num_cols, map.grid.num_rows), (3, 2));
    assert_eq!(map.grid.max, [9, 8, 7, 6, 5, 4]);
    assert_eq!(map.grid.min, [1, 2, 3, 0, 1, 2]);
    assert_eq!(map.grid.bounds_max, [1_536.0, 3_072.0, 333.0]);
    assert_eq!(map.grid.flags, HeightMap::RLE_DATA);
    assert_eq!(map.tile(), Some((512.0, [2, 5])));
    assert_eq!(map.save().expect("saves"), bytes);
}

#[test]
fn a_height_map_with_one_cell_set_saves_at_its_length_and_reads_back_equal() {
    let bytes = pamh();
    let mut map = Gta6HeightMap::open(&bytes).expect("opens");
    // Column 2 of row 1, which holds 4 over 2.
    map.grid.set_cell(2, 1, 3).expect("sets");
    let saved = map.save().expect("saves");
    assert_eq!(saved.len(), bytes.len());
    let changed: Vec<usize> = (0..saved.len()).filter(|&i| saved[i] != bytes[i]).collect();
    assert_eq!(
        changed,
        [44 + 16 + 5],
        "only the cell's maximum byte changes"
    );
    assert_eq!(Gta6HeightMap::open(&saved).expect("reads back"), map);
}

#[test]
fn a_height_grid_with_a_covered_cell_set_saves_at_its_length_and_reads_back_equal() {
    let bytes = mhiu();
    let mut grid = Gta6HeightGrid::open(&bytes).expect("opens");
    let mut map = grid.height_map().expect("a height map");
    // Row 2's column 1, inside its span: both halves move, as one array holds one height.
    map.set_cell(1, 2, 0x99).expect("sets");
    assert_eq!((map.max[9], map.min[9]), (0x99, 0x99));
    grid.set_height_map(&map).expect("takes it");
    let saved = grid.save().expect("saves");
    assert_eq!(saved.len(), bytes.len());
    let back = Gta6HeightGrid::open(&saved).expect("reads back");
    assert_eq!(back.height_map().expect("a height map"), map);
}

#[test]
fn a_height_map_that_breaks_a_measured_rule_is_refused() {
    let mut version = pamh();
    version[4] = 2;
    let mut longer = pamh();
    longer[40] += 1;
    let mut trimmed = pamh();
    trimmed[44 + 8] = 1;
    let mut cut = pamh();
    cut.pop();
    for bytes in [version, longer, trimmed, cut] {
        let error = Gta6HeightMap::open(&bytes).unwrap_err().to_string();
        assert!(error.contains("GTA VI height map"), "{error}");
    }
    let mut masked = Gta6HeightMap::open(&pamh()).expect("opens");
    masked.grid.water_mask = vec![1];
    assert!(masked.save().is_err());
    masked.grid.water_mask.clear();
    masked.grid.bounds_max[0] += 1.0;
    assert_eq!(masked.tile(), None);
}

/// A 4 x 3 grid of one array: a trimmed row, an empty one and a full one.
fn mhiu() -> Vec<u8> {
    let mut out = b"MHIU\x01\x00".to_vec();
    le16(&mut out, 4);
    le16(&mut out, 3);
    le16(&mut out, 0);
    floats(
        &mut out,
        [-1_000.0, -2_000.0, -30.0, 3_000.0, 4_000.0, 250.0],
    );
    le32(&mut out, 3 * 8 + 6);
    le32(&mut out, 0x5EED_0042);
    for (start, count, offset) in [(1u16, 2u16, -1i32), (0, 0, 0), (0, 4, 2)] {
        le16(&mut out, start);
        le16(&mut out, count);
        le32(&mut out, offset as u32);
    }
    out.extend_from_slice(&[5, 6, 7, 8, 9, 10]);
    out
}

#[test]
fn a_height_grid_of_one_array_reads_its_packed_rows_and_saves_identical() {
    let bytes = mhiu();
    let grid = Gta6HeightGrid::open(&bytes).expect("opens");
    assert_eq!(grid.num_cols, 4);
    assert_eq!(grid.carried, 0x5EED_0042);
    assert_eq!(grid.bounds_min, [-1_000.0, -2_000.0, -30.0]);
    let rows: Vec<(u16, &[u8])> = grid.rows.iter().map(|r| (r.start, &r.cells[..])).collect();
    assert_eq!(
        rows,
        [(1, &[5u8, 6][..]), (0, &[][..]), (0, &[7, 8, 9, 10][..])]
    );
    assert_eq!(grid.save().expect("saves"), bytes);
}

#[test]
fn a_height_grid_opens_as_a_height_map_whose_uncovered_cells_take_byte_0x28() {
    let grid = Gta6HeightGrid::open(&mhiu()).expect("opens");
    let map = grid.height_map().expect("a height map");
    let fill = 0x42;
    let cells = vec![
        fill, 5, 6, fill, // row 0: columns 1 and 2
        fill, fill, fill, fill, // row 1: empty
        7, 8, 9, 10, // row 2: whole
    ];
    assert_eq!((map.num_cols, map.num_rows), (4, 3));
    assert_eq!(map.max, cells);
    assert_eq!(map.min, cells);
    assert_eq!(map.bounds_min, grid.bounds_min);
    assert_eq!(map.bounds_max, grid.bounds_max);
    assert!(map.water_mask.is_empty());
    let mut wide = grid;
    wide.rows[0].start = 3;
    assert!(wide.height_map().is_err());
}

#[test]
fn a_height_grid_takes_its_height_map_back_growing_a_span_over_a_changed_cell() {
    let bytes = mhiu();
    let mut grid = Gta6HeightGrid::open(&bytes).expect("opens");
    let map = grid.height_map().expect("a height map");
    grid.set_height_map(&map).expect("takes it");
    assert_eq!(
        grid.save().expect("saves"),
        bytes,
        "unchanged, it saves identical"
    );

    let mut edited = map.clone();
    // Row 0's column 3 lies outside its span, row 1 is empty, row 2's column 0 is inside its span.
    for at in [3, 5, 8] {
        edited.max[at] = 0x77;
        edited.min[at] = 0x77;
    }
    edited.bounds_max[2] = 260.0;
    grid.set_height_map(&edited).expect("takes it");
    let back = Gta6HeightGrid::open(&grid.save().expect("saves")).expect("reads back");
    assert_eq!(back, grid);
    assert_eq!(back.height_map().expect("a height map"), edited);
    let rows: Vec<(u16, &[u8])> = back.rows.iter().map(|r| (r.start, &r.cells[..])).collect();
    assert_eq!(
        rows,
        [
            (1, &[5u8, 6, 0x77][..]),
            (1, &[0x77][..]),
            (0, &[0x77, 8, 9, 10][..])
        ]
    );

    let mut two = map.clone();
    two.min[0] = 1;
    assert!(
        grid.set_height_map(&two).is_err(),
        "one array holds one height a cell"
    );
    let mut short = map;
    short.max.pop();
    short.min.pop();
    assert!(grid.set_height_map(&short).is_err());
}

#[test]
fn a_height_grid_whose_spans_are_not_packed_is_refused() {
    let mut unpacked = mhiu();
    unpacked[44 + 16 + 4] = 3;
    let mut empty_with_offset = mhiu();
    empty_with_offset[44 + 8 + 4] = 1;
    let mut cut = mhiu();
    cut.pop();
    let mut wide = mhiu();
    wide[6] = 3;
    for bytes in [unpacked, empty_with_offset, cut, wide] {
        assert!(Gta6HeightGrid::open(&bytes).is_err());
    }
}

/// A span grid over rows 5 and 6: three live spans and an empty one, four cells a plane.
fn span_grid(kind: u32, a: u32, b: u32) -> Vec<u8> {
    let mut out = vec![1, 0, 1, 2];
    [128, 128].iter().for_each(|v| le16(&mut out, *v));
    le32(&mut out, kind);
    [128, 128, 5, 7].iter().for_each(|v| le16(&mut out, *v));
    [a, b, 4].iter().for_each(|v| le32(&mut out, *v));
    for (start, end, offset) in [(2u16, 4u16, -2i32), (10, 11, -8), (0, 1, 3), (5, 5, 0)] {
        le16(&mut out, start);
        le16(&mut out, end);
        le32(&mut out, offset as u32);
    }
    out.extend_from_slice(&[11, 12, 13, 14, 21, 22, 23, 24]);
    out
}

/// A word whose low half is its high half's low byte with bit 7 set.
const WORD: u32 = 0x3B40_00C0;

/// A kind 2 grid over rows 0..128: one cell a row, at column `row % 128`.
fn full_span_grid(a: u32) -> Vec<u8> {
    let mut out = vec![1, 0, 1, 2];
    [128, 128].iter().for_each(|v| le16(&mut out, *v));
    le32(&mut out, 2);
    [128, 128, 0, 128].iter().for_each(|v| le16(&mut out, *v));
    [a, 0, 128].iter().for_each(|v| le32(&mut out, *v));
    for row in 0..128u16 {
        le16(&mut out, row);
        le16(&mut out, row + 1);
        le32(&mut out, 0);
        out.extend_from_slice(&[0; 8]);
    }
    out.extend((0..=255u8).map(|v| v ^ 0x5A));
    out
}

#[test]
fn a_span_grid_places_its_word_by_kind_and_the_empty_form_has_none() {
    let full = SpanGrid::open(&full_span_grid(WORD)).expect("opens");
    assert_eq!((full.word(), full.a, full.b), (WORD, WORD, 0));
    assert_eq!(
        full.spans[127][0],
        GridSpan {
            start: 127,
            end: 128,
            offset: 0
        }
    );
    let low = SpanGrid::open(&span_grid(0, 0, 0x3A1C_009C)).expect("opens");
    assert_eq!((low.word(), low.a), (0x3A1C_009C, 0));
    let mut empty = vec![1, 0, 1, 2];
    [128, 128].iter().for_each(|v| le16(&mut empty, *v));
    le32(&mut empty, 0);
    [128, 128, 0, 0].iter().for_each(|v| le16(&mut empty, *v));
    [0, 0, 0].iter().for_each(|v| le32(&mut empty, *v));
    let grid = SpanGrid::open(&empty).expect("the empty form opens");
    assert_eq!((grid.word(), grid.spans.len()), (0, 0));
    assert_eq!(grid.save().expect("saves"), empty);
    // A kind 2 grid covers every row, and its word sits at 0x14 only.
    assert!(SpanGrid::open(&full_span_grid(0)).is_err());
    let mut both = full_span_grid(WORD);
    both[0x18..0x1C].copy_from_slice(&WORD.to_le_bytes());
    assert!(SpanGrid::open(&both).is_err());
}

#[test]
fn a_span_grid_reads_its_two_planes_and_saves_identical() {
    let bytes = span_grid(1, 0, WORD);
    assert_eq!(
        OtherBody::of(&bytes, bytes.len() as u64),
        Some(OtherBody::SpanGrid)
    );
    let grid = SpanGrid::open(&bytes).expect("opens");
    assert_eq!((grid.kind, grid.first_row, grid.a, grid.b), (1, 5, 0, WORD));
    assert_eq!(grid.word(), WORD);
    assert_eq!(grid.spans.len(), 2);
    assert_eq!(
        grid.spans[0][1],
        GridSpan {
            start: 10,
            end: 11,
            offset: -8
        }
    );
    assert_eq!(grid.planes, [vec![11, 12, 13, 14], vec![21, 22, 23, 24]]);
    assert_eq!(grid.save().expect("saves"), bytes);
    let mut uneven = grid;
    uneven.planes[1].pop();
    assert!(uneven.save().is_err());
}

#[test]
fn a_span_grid_that_breaks_a_measured_rule_is_refused() {
    let mut more_cells = span_grid(1, 0, WORD);
    more_cells[0x1C] = 5;
    let mut outside = span_grid(1, 0, WORD);
    outside[32 + 2] = 129;
    for bytes in [
        span_grid(5, 0, WORD),
        span_grid(0, WORD, 0),
        span_grid(1, 0, 0),
        span_grid(2, 0, 0),
        span_grid(2, WORD, WORD),
        // The low half is not the high half's low byte with bit 7 set.
        span_grid(1, 0, 0x3B40_00D0),
        span_grid(1, 0, 0x3B40_0040),
        // A zero word outside the empty form, and kind 2 over rows that are not 0..128.
        span_grid(0, 0, 0),
        span_grid(2, WORD, 0),
        more_cells,
        outside,
    ] {
        assert!(SpanGrid::open(&bytes).is_err());
    }
    assert!(SpanGrid::open(&span_grid(1, 0, WORD)).is_ok());
}

/// A MoVE network head with two A hashes, one B hash, two D entries and five bytes after the root.
fn move_network() -> Vec<u8> {
    let mut out = b"MoVE".to_vec();
    out.push(0x13);
    out.extend_from_slice(&[0; 15]);
    le32(&mut out, 0x1234_5678);
    le32(&mut out, 0);
    le32(&mut out, 0);
    [2, 0xA1, 0xA2, 1, 0xB1, 1, 2]
        .iter()
        .for_each(|v| le32(&mut out, *v));
    for (hash, byte) in [(0xD1u32, 0u8), (0xD2, 2)] {
        le32(&mut out, hash);
        out.push(byte);
    }
    out.extend_from_slice(&[0; 10]);
    le16(&mut out, 22);
    le16(&mut out, 0);
    le32(&mut out, 0x0BAD_F00D);
    out.extend_from_slice(&[1, 2, 3, 4, 5]);
    let length = (out.len() - 0x2E - 5 * 2) as u32;
    out[0x18..0x1C].copy_from_slice(&length.to_le_bytes());
    out
}

#[test]
fn a_move_network_head_reads_its_lists_and_root_and_saves_identical() {
    let bytes = move_network();
    let head = MoveNetworkHead::open(&bytes).expect("opens");
    assert_eq!(head.name_hash, 0x1234_5678);
    assert_eq!((head.list_a, head.list_b), (vec![0xA1, 0xA2], vec![0xB1]));
    assert_eq!(head.word_after_b, 1);
    assert_eq!(head.list_d, [(0xD1, 0), (0xD2, 2)]);
    assert_eq!(
        (head.root_type, head.root_index, head.root_id),
        (22, 0, 0x0BAD_F00D)
    );
    assert_eq!(head.rest, [1, 2, 3, 4, 5]);
    assert_eq!(
        MoveNetworkHead::open(&bytes).unwrap().save().unwrap(),
        bytes
    );
}

#[test]
fn a_move_network_that_breaks_a_measured_rule_is_refused() {
    let mut length = move_network();
    length[0x18] += 1;
    let mut version = move_network();
    version[4] = 0x14;
    let mut count = move_network();
    count[0x20..0x24].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    let mut word = move_network();
    word[0x1C] = 1;
    for bytes in [length, version, count, word] {
        assert!(MoveNetworkHead::open(&bytes).is_err());
    }
}

#[test]
fn a_head_names_its_body() {
    assert_eq!(OtherBody::of(&pamh(), 0), Some(OtherBody::HeightMap));
    assert_eq!(OtherBody::of(&mhiu(), 0), Some(OtherBody::HeightGrid));
    assert_eq!(
        OtherBody::of(&move_network(), 0),
        Some(OtherBody::MoveNetwork)
    );
    assert_eq!(OtherBody::of(b"GFX\x08\x08\0\0\0", 8), Some(OtherBody::Gfx));
    // Text that starts `GFX` states no length of its body at +4.
    assert_eq!(OtherBody::of(b"GFX_SCALE=1\n", 12), None);
    assert_eq!(OtherBody::of(b"EDUP\x07\0\0\0", 8), Some(OtherBody::Edup));
    let mut narrow = span_grid(0, 0, 0);
    narrow[12] = 127;
    assert_eq!(OtherBody::of(&narrow, 0), None);
    assert_eq!(OtherBody::of(&[0; 16], 16), None);
    assert_eq!(OtherBody::of(b"GF", 2), None);
}
