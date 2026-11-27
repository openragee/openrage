//! The record an instanced draw places each of its instances by, 128 bytes an instance, as the
//! game writes it for an instance that stands where it is placed.

use super::shading::{Fed, Feed};

/// The bytes of one record.
pub const BYTES: u32 = 128;

/// A row-major 3x4 matrix that leaves a vertex where it is.
pub const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// One instance's record.
// ⚠ The rows, the frame before's rows, the noughts and the colour's alpha are what the game
// writes; the fade, the colour, the tint row and the last float are the instance's own, which a
// model opened alone does not state: they hold the values of an instance that is neither faded
// nor tinted, which no frame has shown to be the game's.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceRecord {
    /// Three rows of four floats: the turn and scale in the first three columns, the place in the fourth.
    pub rows: [[f32; 4]; 3],
    /// The same rows as the frame before placed the instance.
    pub rows_before: [[f32; 4]; 3],
    /// The fourth half of the instance's colour times how far it stands into its fade distance, at most one.
    pub fade: f32,
    /// Noughts, as the game writes them for a placed instance.
    pub zero: [u32; 4],
    /// The instance's colour as linear bytes, red lowest, its alpha 255.
    pub colour: u32,
    /// The row of the tint palette the instance takes.
    pub tint_row: i32,
    /// The fourth float of the instance's place in the frame before.
    pub before_w: f32,
}

impl InstanceRecord {
    /// The record of an instance at rest placed by `placement`, a row-major 3x4 matrix.
    pub fn at_rest(placement: [f32; 12]) -> Self {
        let rows: [[f32; 4]; 3] =
            std::array::from_fn(|r| std::array::from_fn(|c| placement[4 * r + c]));
        Self {
            rows,
            rows_before: rows,
            fade: 1.0,
            zero: [0; 4],
            colour: 0xFFFF_FFFF,
            tint_row: 0,
            before_w: 0.0,
        }
    }

    /// The record's bytes, as a buffer of records holds them.
    pub fn bytes(&self) -> Vec<u8> {
        let floats = |row: &[f32; 4]| row.map(f32::to_bits);
        let mut words: Vec<u32> = Vec::with_capacity(BYTES as usize / 4);
        words.extend(self.rows.iter().flat_map(floats));
        words.extend(self.rows_before.iter().flat_map(floats));
        words.push(self.fade.to_bits());
        words.extend(self.zero);
        words.push(self.colour);
        words.push(self.tint_row as u32);
        words.push(self.before_w.to_bits());
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

/// The buffers an instanced mesh stage tints its instances by, by the hashes of their names: 12-byte
/// records and a row texture of colours, which take the record's tint row and colour.
const TINTS: [u32; 2] = [0x8E78_E380, 0xFAC2_2BE4];

/// Whether a stage fed `feeds` reads a field of an instance's record no file of a model opened
/// alone states: one that binds what tints an instance reads its tint row and its colour.
// ponytail: told by what the stage binds, not by the fields it reads; a stage that reads the fade
// alone is let through with the fade at full.
pub(super) fn reads_unsettled(feeds: &[Fed]) -> bool {
    (feeds.iter()).any(|fed| TINTS.iter().any(|hash| fed.feed == Feed::Zero(Some(*hash))))
}

#[cfg(test)]
mod tests {
    use std::mem::{offset_of, size_of};

    use super::*;

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn each_field_of_a_record_lies_where_the_layout_states() {
        assert_eq!(size_of::<InstanceRecord>(), BYTES as usize);
        assert_eq!(offset_of!(InstanceRecord, rows), 0x00);
        assert_eq!(offset_of!(InstanceRecord, rows_before), 0x30);
        assert_eq!(offset_of!(InstanceRecord, fade), 0x60);
        assert_eq!(offset_of!(InstanceRecord, zero), 0x64);
        assert_eq!(offset_of!(InstanceRecord, colour), 0x74);
        assert_eq!(offset_of!(InstanceRecord, tint_row), 0x78);
        assert_eq!(offset_of!(InstanceRecord, before_w), 0x7C);
        // The bytes put each field where the struct does, with values that differ field by field.
        let placement: [f32; 12] = std::array::from_fn(|k| k as f32 + 0.5);
        let record = InstanceRecord {
            fade: 0.25,
            zero: [11, 12, 13, 14],
            colour: 0x1122_3344,
            tint_row: 7,
            before_w: -3.0,
            ..InstanceRecord::at_rest(placement)
        };
        let bytes = record.bytes();
        assert_eq!(bytes.len(), BYTES as usize);
        for (k, value) in placement.iter().enumerate() {
            assert_eq!(word(&bytes, 4 * k), value.to_bits(), "row dword {k}");
            assert_eq!(
                word(&bytes, 0x30 + 4 * k),
                value.to_bits(),
                "frame before's dword {k}"
            );
        }
        assert_eq!(
            word(&bytes, offset_of!(InstanceRecord, fade)),
            0.25f32.to_bits()
        );
        assert_eq!(word(&bytes, offset_of!(InstanceRecord, zero) + 12), 14);
        assert_eq!(
            word(&bytes, offset_of!(InstanceRecord, colour)),
            0x1122_3344
        );
        assert_eq!(word(&bytes, offset_of!(InstanceRecord, tint_row)), 7);
        assert_eq!(
            word(&bytes, offset_of!(InstanceRecord, before_w)),
            (-3.0f32).to_bits()
        );
    }

    #[test]
    fn an_instance_at_rest_stands_where_it_is_placed_with_its_alpha_full() {
        let record = InstanceRecord::at_rest(IDENTITY);
        assert_eq!(
            record.rows,
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0]
            ]
        );
        assert_eq!(record.rows_before, record.rows);
        assert_eq!(record.colour >> 24, 0xFF);
        assert_eq!(record.zero, [0; 4]);
    }

    #[test]
    fn a_stage_that_binds_what_tints_an_instance_reads_an_unsettled_field() {
        let fed = |feed| Fed {
            group: 0,
            binding: 2,
            feed,
            header: None,
        };
        for hash in TINTS {
            assert!(reads_unsettled(&[
                fed(Feed::InstanceRecords),
                fed(Feed::Zero(Some(hash)))
            ]));
        }
        assert!(!reads_unsettled(&[
            fed(Feed::InstanceRecords),
            fed(Feed::Zero(Some(0x1234)))
        ]));
    }
}
