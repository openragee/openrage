use super::*;

/// A table of `ENTRIES` made-up entries, entry `i` hashing to `i * 7 + 1`, with `bad` broken by a wrong index.
fn table_bytes(bad: Option<usize>) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..ENTRIES {
        let own = if bad == Some(i) { i + 1 } else { i } as u32;
        let words = [own, i as u32 * 7 + 1, (i as f32 * 0.5).to_bits(), 1, 1, 2];
        out.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    }
    out
}

#[test]
fn the_variable_table_is_found_by_its_shape_and_refused_where_an_entry_is_not_its_own() {
    let mut image = vec![0xcc_u8; 4096];
    image.extend(table_bytes(None));
    image.extend([0xcc; 64]);
    let table = VarTable::find(&image).expect("the table is found");
    assert_eq!(table.entries.len(), ENTRIES);
    assert_eq!(table.index_of(3 * 7 + 1), Some(3));
    assert_eq!(table.entries[3].default, 1.5);
    assert_eq!(table.defaults()[FRAME_FLOATS - 1], 0.0);
    assert_eq!(table.defaults()[10], 5.0);
    assert!(VarTable::parse(&table_bytes(Some(500))).is_none());
    // Two tables in one image name no one table.
    let mut twice = image.clone();
    twice.extend(table_bytes(None));
    assert!(VarTable::find(&twice).is_none());
}

#[test]
fn the_noon_table_is_a_smooth_year_of_bytes_then_nought() {
    let year: Vec<u8> = (0..NOON_DAYS)
        .map(|d| (30.0 + 12.0 * (d as f32 / 58.0).sin()) as u8)
        .collect();
    let mut image = vec![200_u8, 3, 90];
    image.extend(&year);
    image.extend([0, 0, 0, 0]);
    assert_eq!(find_noon_table(&image).as_deref(), Some(&year[..]));
    // A flat run is not the table, nor one that jumps.
    let mut flat = vec![7_u8; NOON_DAYS + 10];
    flat.push(0);
    assert_eq!(find_noon_table(&flat), None);
    let mut jumps = year.clone();
    jumps[100] = jumps[99].saturating_add(9);
    jumps.push(0);
    assert_eq!(find_noon_table(&jumps), None);
}
