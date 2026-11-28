//! What the game leaves, for a plant drawn alone in still air, in the bindings of the wind
//! effects' mesh stages that only the game fills.

use super::shading::Feed;

/// The plant's wind records, 80 bytes each ([`WindRecord`]): a geometry's vertex picks its
/// record, and a record lists the records of the branches it hangs from.
pub const RECORDS: u32 = 0x1948_E9AA;
/// How many records the buffer the game binds for a plant with no wind data of its own holds.
pub const RECORDS_AT_REST: usize = 256;

/// The one-dimensional falloff a render-graph callback binds by the plant's key, from the set
/// a data file of its own fills: 128 texels sampled from a curve, noughts for a curve with no
/// key.
pub const FALLOFF: u32 = 0x569D_8396;

/// One wind record as the game makes it for a plant that has none of its own.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindRecord {
    /// How many branch levels the record hangs from: a stage walks that many records of
    /// `ancestors` and bends the vertex by none where it is nought.
    // ⚠ A stage counts its walk up to this byte and leaves on an exact hit: it is entered only
    // where the byte is not nought.
    pub levels: u8,
    /// Written 2 by the game; what it states is not read.
    pub flags: u8,
    /// A half float, 1.0.
    pub scale: u16,
    /// The record of each level the record hangs from, a byte each, nearest first.
    pub ancestors: [u8; 8],
    /// Two half floats, 1.0 each.
    pub bend: [u16; 2],
    /// Noughts in the game's default record.
    pub rest: [u32; 16],
}

const _: () = assert!(std::mem::size_of::<WindRecord>() == 80);

/// The half float 1.0.
const HALF_ONE: u16 = 0x3C00;

impl WindRecord {
    /// The record the game fills its default buffer with.
    pub const AT_REST: Self = Self {
        levels: 0,
        flags: 2,
        scale: HALF_ONE,
        ancestors: [0; 8],
        bend: [HALF_ONE; 2],
        rest: [0; 16],
    };

    /// The record's bytes as the stage reads them.
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![self.levels, self.flags];
        bytes.extend(self.scale.to_le_bytes());
        bytes.extend(self.ancestors);
        bytes.extend(self.bend.iter().flat_map(|h| h.to_le_bytes()));
        bytes.extend(self.rest.iter().flat_map(|w| w.to_le_bytes()));
        bytes
    }
}

/// The buffer of records the game binds for a plant with no wind data of its own.
pub fn records_at_rest() -> Vec<u8> {
    WindRecord::AT_REST.bytes().repeat(RECORDS_AT_REST)
}

/// What the game leaves in the binding named `hash`, where it is the wind system's and its
/// noughts are not that.
pub(super) fn stated(hash: u32) -> Option<Feed> {
    (hash == RECORDS).then(|| Feed::Stated(records_at_rest()))
}

#[cfg(test)]
mod tests {
    use std::mem::offset_of;

    use or_io::gta5::shaders::sgd5::Sgd5Table;

    use super::*;
    use crate::rdna::ResourceKind;
    use crate::rdna_bind::shading::feed;
    use crate::rdna_bind::trees::tests::stage_reading;

    #[test]
    fn a_wind_record_holds_each_field_at_the_byte_the_stages_read() {
        assert_eq!(offset_of!(WindRecord, levels), 0);
        assert_eq!(offset_of!(WindRecord, scale), 2);
        assert_eq!(offset_of!(WindRecord, ancestors), 4);
        assert_eq!(offset_of!(WindRecord, bend), 12);
        assert_eq!(offset_of!(WindRecord, rest), 16);
        let bytes = WindRecord::AT_REST.bytes();
        assert_eq!(bytes.len(), 80);
        assert_eq!(&bytes[..4], &[0, 2, 0x00, 0x3C]);
        assert_eq!(&bytes[12..16], &[0x00, 0x3C, 0x00, 0x3C]);
        assert!(bytes[16..].iter().all(|b| *b == 0));
    }

    /// A stage that walks a record's levels, as a wind mesh stage does: up to the low byte of
    /// the first dword of the record its vertex picks, past the buffer's header.
    const WALKING: &str = "
@group(0) @binding(0) var<storage> records: array<u32>;
@vertex fn main(@builtin(vertex_index) v: u32) -> @builtin(position) vec4<f32> {
    let levels = records[(v * records[0u]) / 4u + 4u] & 255u;
    var i: u32 = 0u;
    if levels != 0u {
        loop {
            i = i + 1u;
            if i == levels { break; }
        }
    }
    return vec4<f32>(f32(i));
}";

    #[test]
    fn the_records_are_fed_as_the_game_s_default_buffer_and_a_walk_of_their_levels_is_let_through()
    {
        let (native, lowered, table) =
            stage_reading(RECORDS, 80, ResourceKind::Buffer { stride: None }, WALKING);
        let stage = feed(&native, lowered, &table, &Sgd5Table::default()).expect("fed");
        assert_eq!(stage.feeds[0].header, Some(80));
        let Feed::Stated(bytes) = &stage.feeds[0].feed else {
            panic!("the records are not stated: {:?}", stage.feeds[0]);
        };
        assert_eq!(bytes.len(), 80 * RECORDS_AT_REST);
        assert!(bytes.chunks(80).all(|r| r == WindRecord::AT_REST.bytes()));
    }

    #[test]
    fn the_falloff_is_fed_as_the_noughts_of_a_curve_with_no_key() {
        let (native, lowered, table) = stage_reading(
            FALLOFF,
            4,
            ResourceKind::Texture {
                dim: 0,
                compared: false,
            },
            "
@group(0) @binding(0) var falloff: texture_1d<f32>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(1.0 - textureLoad(falloff, 0, 0).z);
}",
        );
        let stage = feed(&native, lowered, &table, &Sgd5Table::default()).expect("fed");
        assert_eq!(stage.feeds[0].feed, Feed::Zero(Some(FALLOFF)));
    }
}
