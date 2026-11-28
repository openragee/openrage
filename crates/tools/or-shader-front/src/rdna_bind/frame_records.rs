//! What the game leaves, for a model drawn alone, in the frame's wind constants and its wind field
//! records: no field, since each lies over a region of the world a model drawn alone is in none of.

/// The frame's wind constants, 680 bytes: dword 0 is how many records of [`RECORDS`] the frame
/// holds, the fields a settings file lists that the world's fields hold.
pub const FRAME: u32 = 0x06AD_2A7A;
/// The frame's wind field records, 144 bytes each ([`WindField`]).
pub const RECORDS: u32 = 0xD108_AEFF;

/// The buffers only the game fills whose noughts are their state for a model drawn alone: no wind
/// field, and a stage skips its walk of the records on a count of nought.
// ponytail: stated per buffer, as `trees::AT_REST` is: only dword 0 of the constants is
// established at rest, so a loop counted to another dword of them would be let through as well.
// Split it by dword when such a loop is met.
pub const AT_REST: &[u32] = &[FRAME];

/// One wind field record as the game writes it each frame, a dword a field.
// A field adds to the wind only inside its box; outside every box the walk adds nothing, which is
// what a count of nought gives.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindField {
    /// The box's near corner in world metres; its height is the field's band's where it has one.
    pub min: [f32; 3],
    /// The box's far corner.
    pub max: [f32; 3],
    /// One over the box's size on each axis.
    pub inverse_size: [f32; 3],
    /// The band's scales of the horizontal wind near its floor and near its top, and of the
    /// vertical wind; ones where the field has no band.
    pub band_scales: [f32; 3],
    /// The height above the ground the band's top fade ends at.
    pub band_top: f32,
    /// One over the depth of the band's top fade; nought where it has none.
    pub band_top_fade: f32,
    /// The height above the ground the band's floor fade starts at.
    pub band_floor: f32,
    /// One over the depth of the band's floor fade; nought where it has none.
    pub band_floor_fade: f32,
    /// What is taken from each channel of the field's map before it is scaled.
    pub map_bias: [f32; 4],
    /// What each channel is scaled by before it is clamped to nought and one.
    pub map_scale: [f32; 4],
    /// The wind each channel gives at nought.
    pub low: [f32; 4],
    /// What the wind each channel gives rises by from nought to one.
    pub range: [f32; 4],
    /// The field's strength, which the weather scales.
    pub strength: f32,
    /// One over the time a slice of the field's map lasts; nought where it has one slice.
    pub inverse_slice_time: f32,
    /// 1 makes the horizontal wind a direction, 2 the whole wind; any other value leaves it.
    pub normalise: u32,
    /// The heap number of the field's map.
    pub map: u32,
}

const _: () = assert!(std::mem::size_of::<WindField>() == 144);

#[cfg(test)]
mod tests {
    use std::mem::offset_of;

    use or_io::gta5::shaders::sgd5::{Sgd5Resource, Sgd5Table};

    use super::*;
    use crate::dxil::ShaderKind;
    use crate::rdna::{Loc, Lowered, Program, Resource, ResourceKind};
    use crate::rdna_bind::shading::{feed, Feed, Unfed};
    use crate::rdna_bind::{Class, Native, Record};

    #[test]
    fn a_wind_field_holds_each_field_at_the_byte_the_stages_read() {
        assert_eq!(offset_of!(WindField, min), 0);
        assert_eq!(offset_of!(WindField, max), 12);
        assert_eq!(offset_of!(WindField, inverse_size), 24);
        assert_eq!(offset_of!(WindField, band_scales), 36);
        assert_eq!(offset_of!(WindField, band_top), 48);
        assert_eq!(offset_of!(WindField, band_top_fade), 52);
        assert_eq!(offset_of!(WindField, band_floor), 56);
        assert_eq!(offset_of!(WindField, band_floor_fade), 60);
        assert_eq!(offset_of!(WindField, map_bias), 64);
        assert_eq!(offset_of!(WindField, map_scale), 80);
        assert_eq!(offset_of!(WindField, low), 96);
        assert_eq!(offset_of!(WindField, range), 112);
        assert_eq!(offset_of!(WindField, strength), 128);
        assert_eq!(offset_of!(WindField, inverse_slice_time), 132);
        assert_eq!(offset_of!(WindField, normalise), 136);
        assert_eq!(offset_of!(WindField, map), 140);
    }

    const TABLE: u8 = 9;
    const AT: u32 = 0x40;

    /// A deferred pixel program whose one record is a constant buffer at register 7.
    fn native() -> Native {
        let exports = [(0u32, 15u32), (1, 15), (2, 15), (3, 15), (4, 3)];
        let words = (exports.iter())
            .flat_map(|(target, en)| [(0x3E << 26) | (target << 4) | en, 0])
            .collect();
        Native {
            program: Program {
                kind: ShaderKind::Pixel,
                slot: 5,
                words,
                wave32: false,
                inputs: Vec::new(),
                vertex_inputs: Vec::new(),
                position_read: 0,
                front_face: false,
                launch: None,
            },
            records: vec![Record {
                class: Class::Cbv,
                register: 7,
                root: TABLE,
                offset: AT,
                dwords: 4,
                stride: 0,
            }],
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// The program's own table, which names register 7's constant buffer `buffer`.
    fn table(buffer: u32) -> Sgd5Table {
        Sgd5Table {
            resources: vec![Sgd5Resource {
                code: 0x30,
                flags: 0,
                slot: 7,
                members: 0,
                buffer_size: 16,
                name_hash: buffer,
                semantic_hash: buffer,
            }],
            ..Sgd5Table::default()
        }
    }

    /// A stage that walks records until its count reaches dword 0 of its one constant buffer,
    /// leaving on an exact hit, after the guard on nought the game's own stages hold.
    fn walking() -> Lowered {
        let module = naga::front::wgsl::parse_str(
            "@group(0) @binding(0) var<uniform> cb0_: vec4<u32>;
@fragment fn main() -> @location(0) vec4<f32> {
var s1_: u32 = 0u;
var s2_: u32 = 0u;
s2_ = cb0_.x;
if s2_ != 0u {
loop {
s1_ = s1_ + 1u;
if s1_ == s2_ { break; }
}
}
return vec4<f32>(f32(s1_));
}",
        )
        .expect("the WGSL parses");
        Lowered {
            module,
            resources: vec![Resource {
                kind: ResourceKind::ConstantBuffer { rows: 1 },
                at: Some(Loc {
                    root: TABLE,
                    path: Vec::new(),
                    offset: AT,
                    index: None,
                }),
                words: None,
                group: 0,
                binding: 0,
                count: None,
                written: false,
            }],
            approximations: Default::default(),
            half_slots: Default::default(),
            subgroups: false,
            intersections: Vec::new(),
            region: None,
        }
    }

    #[test]
    fn a_walk_of_the_wind_fields_is_fed_a_count_of_nought_and_one_to_another_buffer_is_not() {
        let fed = |buffer| feed(&native(), walking(), &table(buffer), &Sgd5Table::default());
        let stage = fed(FRAME).expect("a count to the frame's wind fields is let through");
        let feeds: Vec<Feed> = stage.feeds.into_iter().map(|fed| fed.feed).collect();
        assert_eq!(feeds, [Feed::Zero(Some(FRAME))]);
        // The records themselves, and any other buffer only the game fills, stay refused.
        assert_eq!(fed(RECORDS).err(), Some(Unfed::Loop(RECORDS)));
        let other = 0x1234_5678;
        assert_eq!(fed(other).err(), Some(Unfed::Loop(other)));
    }
}
