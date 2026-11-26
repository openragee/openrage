//! Made-up fixtures of the coder for the tests of the crates above this one: a tables file no
//! coder holds, and BC7 bodies coded through it. Nothing here is of any game.

use super::{bptc, supplied};

/// A file no coder holds, in the layout of a tables file.
pub fn tables_file() -> Vec<u8> {
    supplied::tests::made_up_file()
}

/// `file` with the first two entries of BC7's preset list changed over.
pub fn bc7_entries_swapped(file: &[u8]) -> Vec<u8> {
    supplied::tests::with_entries_swapped(file, 5, 0, 1)
}

/// The stream of `count` made-up BC7 blocks coded through [`tables_file`], which decodes to 16
/// bytes a block: the first `from_presets` of them, at most 127, are each a difference from a
/// preset entry of its own, block 0 from entry 0, and the rest from nothing.
pub fn bc7_body(count: usize, from_presets: usize) -> Vec<u8> {
    let file = tables_file();
    let tables = supplied::read(&file).unwrap_or_else(|refused| panic!("{refused}"));
    bptc::tests::body_from_the_slots(tables.bptc(), count, from_presets)
}
