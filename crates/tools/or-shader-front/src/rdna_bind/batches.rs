//! A batch's instances as the game binds them where it draws a batch's programs with no batch of
//! its own: one instance, in place.

use super::shading::Feed;

/// The list of the instances a draw takes, a dword each: the low sixteen bits are the instance's
/// record, the next eight a byte the mesh program takes as well.
pub const LIST: u32 = 0x1AB3_F264;
/// The instances' records, 64 bytes each, and the same records as the frame before placed them.
pub const RECORDS: u32 = 0x9707_1119;
pub const RECORDS_BEFORE: u32 = 0xC978_0FFF;
/// Dwords the pass that culls a batch's instances writes, nought at rest.
pub const CULLED: u32 = 0xF0B6_7341;

/// The bytes of one record.
pub const BYTES: u32 = 64;
/// The entries of the list the game binds at rest.
pub const LIST_ENTRIES: u32 = 0x2000;
/// What the game puts above the instance's number in each entry of the list it binds at rest.
pub const LIST_HIGH: u32 = 0xFFFF_0000;
/// The dwords of the culling pass's buffer the game binds at rest.
pub const CULLED_WORDS: usize = 256;

/// One instance's record.
// ⚠ The words past the rows hold what the game's own record for a draw with no batch holds;
// what they state is not read, but for the top byte of `word13`: a flag in its bit 7 and a
// number in its low seven bits, which the mesh program takes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchRecord {
    /// Three rows of four floats that place the instance.
    pub rows: [[f32; 4]; 3],
    /// One at rest.
    pub word12: f32,
    /// All ones at rest.
    pub word13: u32,
    /// One at rest.
    pub word14: f32,
    /// Nought at rest.
    pub word15: u32,
}

impl BatchRecord {
    /// The record the game binds for a draw with no batch: the instance where it is placed.
    pub const AT_REST: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        word12: 1.0,
        word13: u32::MAX,
        word14: 1.0,
        word15: 0,
    };

    /// The record's bytes, as a buffer of records holds them.
    pub fn bytes(&self) -> Vec<u8> {
        let mut words: Vec<u32> = self.rows.iter().flatten().map(|f| f.to_bits()).collect();
        words.extend([
            self.word12.to_bits(),
            self.word13,
            self.word14.to_bits(),
            self.word15,
        ]);
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

/// The list the game binds at rest: entry `i` names instance `i`.
// A mesh program takes the entry of its group over the geometry's meshlets, so every group of a
// draw of one geometry takes entry nought.
pub fn list_at_rest() -> Vec<u32> {
    (0..LIST_ENTRIES).map(|i| LIST_HIGH | i).collect()
}

/// What the game binds at `hash` for a draw with no batch, where it is one of a batch's buffers.
pub(super) fn stated(hash: u32) -> Option<Feed> {
    let words =
        |words: Vec<u32>| -> Vec<u8> { words.iter().flat_map(|w| w.to_le_bytes()).collect() };
    match hash {
        LIST => Some(Feed::Stated(words(list_at_rest()))),
        RECORDS | RECORDS_BEFORE => Some(Feed::Stated(BatchRecord::AT_REST.bytes())),
        CULLED => Some(Feed::Stated(vec![0; 4 * CULLED_WORDS])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{offset_of, size_of};

    use or_io::gta5::shaders::sgd5::Sgd5Table;

    use super::super::shading::feed;
    use super::super::trees::tests::stage_reading;
    use super::*;
    use crate::rdna::ResourceKind;

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn each_field_of_a_batch_record_lies_where_the_layout_states() {
        assert_eq!(size_of::<BatchRecord>(), BYTES as usize);
        assert_eq!(offset_of!(BatchRecord, rows), 0);
        assert_eq!(offset_of!(BatchRecord, word12), 48);
        assert_eq!(offset_of!(BatchRecord, word13), 52);
        assert_eq!(offset_of!(BatchRecord, word14), 56);
        assert_eq!(offset_of!(BatchRecord, word15), 60);
        let record = BatchRecord {
            rows: std::array::from_fn(|r| std::array::from_fn(|c| (4 * r + c) as f32 + 0.5)),
            word12: 2.0,
            word13: 0x1122_3344,
            word14: -3.0,
            word15: 7,
        };
        let bytes = record.bytes();
        assert_eq!(bytes.len(), BYTES as usize);
        for k in 0..12 {
            assert_eq!(
                word(&bytes, 4 * k),
                (k as f32 + 0.5).to_bits(),
                "row dword {k}"
            );
        }
        assert_eq!(
            word(&bytes, offset_of!(BatchRecord, word12)),
            2f32.to_bits()
        );
        assert_eq!(word(&bytes, offset_of!(BatchRecord, word13)), 0x1122_3344);
        assert_eq!(
            word(&bytes, offset_of!(BatchRecord, word14)),
            (-3f32).to_bits()
        );
        assert_eq!(word(&bytes, offset_of!(BatchRecord, word15)), 7);
    }

    #[test]
    fn the_batch_at_rest_is_one_instance_where_it_is_placed() {
        let rest = BatchRecord::AT_REST.bytes();
        assert_eq!(word(&rest, 0), 1f32.to_bits());
        assert_eq!(word(&rest, 20), 1f32.to_bits());
        assert_eq!(word(&rest, 40), 1f32.to_bits());
        assert_eq!(word(&rest, 12), 0, "no move");
        assert_eq!(word(&rest, 52) >> 24, 0xFF);
        let list = list_at_rest();
        assert_eq!(list.len(), LIST_ENTRIES as usize);
        assert_eq!(list[0] & 0xFFFF, 0, "the first entry names the one record");
        assert_eq!(list[5], 0xFFFF_0005);
    }

    /// A stage that places a vertex by the record its list's first entry names.
    const PLACING: &str = "
@group(0) @binding(0) var<storage> list: array<u32>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(list[4u] & 65535u));
}";

    #[test]
    fn a_batch_stage_is_fed_its_one_instance_and_drawn() {
        let buffer = ResourceKind::Buffer { stride: None };
        let fed = |hash, stride| {
            let (native, lowered, table) = stage_reading(hash, stride, buffer, PLACING);
            let stage = feed(&native, lowered, &table, &Sgd5Table::default())
                .unwrap_or_else(|why| panic!("0x{hash:08X}: {why}"));
            stage.feeds[0].feed.clone()
        };
        // The list is no longer left at nought, so the stage is not refused as a batch.
        let Feed::Stated(list) = fed(LIST, 4) else {
            panic!("the list is not stated");
        };
        assert_eq!(list.len(), 4 * LIST_ENTRIES as usize);
        assert_eq!(word(&list, 0) & 0xFFFF, 0);
        // A record of 64 bytes is the batch's, not an object's matrix told by its shape.
        for hash in [RECORDS, RECORDS_BEFORE] {
            assert_eq!(fed(hash, BYTES), Feed::Stated(BatchRecord::AT_REST.bytes()));
        }
        assert_eq!(fed(CULLED, 4), Feed::Stated(vec![0; 4 * CULLED_WORDS]));
    }
}
