//! What a ped part's stages read of its component at rest: the 48 bytes of the component's
//! buffer, the material textures the component's maps take, and the buffer of decorations.

use or_io::gta6::ped_assets::{BLOOD_MASK, MA, MB, MC};

/// The constant buffer of a ped component's decorations; its first dword is how many there are.
// A component with none is bound the game's own default, sixteen bytes of noughts, so a loop
// that counts up to that dword runs no turn.
pub const DECORATIONS: u32 = 0xCEDF_6466;

/// The constant buffer of a ped component's decoration masks, [`DecorationBuffer`].
pub const DECORATION: u32 = 0x243E_BEF2;

/// The buffer of a ped component's decoration layers, read by the byte: [`LayersHeader`], then
/// each category's records.
pub const LAYERS: u32 = 0x40B5_2538;

/// A category word of [`LayersHeader`]: where its records start, a record's size and how many.
const fn category(start: u32, size: u32, count: u32) -> u32 {
    start | size << 16 | count << 28
}

/// The 40 bytes that open [`LAYERS`], a dword a field.
// A component of no kind is bound the default decoration object's, built from a builder with no
// layer: every count nought, every category starting just past these 40 bytes, and nothing after.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayersHeader {
    /// The first enabled layer's opacity times two strengths of its material; nought with none.
    pub strengths: [f32; 2],
    /// Six categories' records: the byte they start at in bits 0 to 15, a record's size in bits
    /// 16 to 27, how many in bits 28 to 31; the pixel program walks none of a count of nought.
    pub categories: [u32; 6],
    /// The byte the next category starts at in bits 16 to 31; its low bits say which hold any.
    pub present: u32,
    /// The last category's records, as [`Self::categories`].
    pub last: u32,
}

const _: () = assert!(std::mem::size_of::<LayersHeader>() == 40);

/// The size of a record of each of [`LayersHeader::categories`], then of [`LayersHeader::last`].
const RECORD_SIZES: [u32; 7] = [0x30, 0x20, 0x5C, 0x40, 0x30, 0xE0, 0x40];

impl LayersHeader {
    /// The header of the default decoration object's buffer: no layer of any category.
    pub const AT_REST: Self = {
        let end = std::mem::size_of::<Self>() as u32;
        let mut categories = [0; 6];
        let mut at = 0;
        while at < categories.len() {
            categories[at] = category(end, RECORD_SIZES[at], 0);
            at += 1;
        }
        Self {
            strengths: [0.0; 2],
            categories,
            present: end << 16,
            last: category(end, RECORD_SIZES[6], 0),
        }
    };

    /// The header's bytes, which at rest are the whole buffer.
    pub fn bytes(&self) -> Vec<u8> {
        let mut words = self.strengths.map(f32::to_bits).to_vec();
        words.extend(self.categories);
        words.extend([self.present, self.last]);
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

/// The pool of 416-byte records an object's draw record picks one of by the high half of its
/// dword 7; a ped's pixel programs multiply the albedo by the float at byte 32 of it.
pub const OBJECT_DATA: u32 = 0x3B31_6B01;

/// One record of [`OBJECT_DATA`], a field per run of dwords the game writes alike.
// The game gives every object whose drawable has a program reading the pool a holder of its own,
// built with these values; the set-up's element nought, which differs (its albedo scale is
// nought), is bound only to objects with no such program. The holder's update then rewrites
// fields from the object's scene data, which a model drawn alone has none of; it keeps the albedo
// scale at one while the holder's darkening byte, nought when built, stays nought. Fields named by
// offset are not read by use.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectData {
    /// -2 each; the pixel program tests bit 0 of the first's bits.
    pub lead: [f32; 2],
    pub zeros_08: [f32; 3],
    /// A colour the deferred pixel program blends in only where bit 1 of [`Self::switches`] is set.
    pub colour: [f32; 3],
    /// What the deferred pixel program multiplies the albedo by, so also the f0 a metal takes.
    pub albedo_scale: f32,
    pub ones_24: [f32; 7],
    /// 0.5, 16, 1, 0.1, 0 and 1.
    pub values_40: [f32; 6],
    pub zeros_58: [f32; 4],
    pub one_68: f32,
    pub zeros_6c: [f32; 2],
    /// Scales the pixel program's bend of the normal; nought skips it.
    pub normal_bend: f32,
    pub minus_one_78: f32,
    pub zero_7c: f32,
    pub none_80: i32,
    pub one_84: f32,
    pub zero_88: f32,
    pub one_8c: f32,
    pub zeros_90: [f32; 3],
    pub floor_9c: f32,
    /// Where the holder's 1,280 bytes of extra data were copied to; -1 until a setter writes any.
    pub extra: i32,
    pub ones_a4: [f32; 3],
    pub zero_b0: f32,
    pub ones_b4: [f32; 3],
    pub zeros_c0: [f32; 43],
    /// Bits that switch branches of the pixel program on; bit 1 blends [`Self::colour`].
    pub switches: u32,
    pub none_170: i32,
    pub zero_174: f32,
    pub ones_178: [f32; 3],
    pub zeros_184: [f32; 2],
    pub ten_18c: f32,
    pub zeros_190: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ObjectData>() == 416);

impl ObjectData {
    /// The record an object's own holder is built with.
    // The constructor writes (0.2, 0.19, 0.08) to the colour, then its setter replaces any value
    // other than (8, 8, 2) / 255 with those.
    pub const AT_REST: Self = Self {
        lead: [-2.0; 2],
        zeros_08: [0.0; 3],
        colour: [8.0 / 255.0, 8.0 / 255.0, 2.0 / 255.0],
        albedo_scale: 1.0,
        ones_24: [1.0; 7],
        values_40: [0.5, 16.0, 1.0, 0.1, 0.0, 1.0],
        zeros_58: [0.0; 4],
        one_68: 1.0,
        zeros_6c: [0.0; 2],
        normal_bend: 1.0,
        minus_one_78: -1.0,
        zero_7c: 0.0,
        none_80: -1,
        one_84: 1.0,
        zero_88: 0.0,
        one_8c: 1.0,
        zeros_90: [0.0; 3],
        floor_9c: -1000.0,
        extra: -1,
        ones_a4: [1.0; 3],
        zero_b0: 0.0,
        ones_b4: [1.0; 3],
        zeros_c0: [0.0; 43],
        switches: 0,
        none_170: -1,
        zero_174: 0.0,
        ones_178: [1.0; 3],
        zeros_184: [0.0; 2],
        ten_18c: 10.0,
        zeros_190: [0.0; 4],
    };

    /// The record's bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let mut words: Vec<u32> = self.lead.map(f32::to_bits).to_vec();
        let floats = |words: &mut Vec<u32>, values: &[f32]| {
            words.extend(values.iter().map(|v| v.to_bits()));
        };
        floats(&mut words, &self.zeros_08);
        floats(&mut words, &self.colour);
        floats(&mut words, &[self.albedo_scale]);
        floats(&mut words, &self.ones_24);
        floats(&mut words, &self.values_40);
        floats(&mut words, &self.zeros_58);
        floats(&mut words, &[self.one_68]);
        floats(&mut words, &self.zeros_6c);
        floats(
            &mut words,
            &[self.normal_bend, self.minus_one_78, self.zero_7c],
        );
        words.push(self.none_80 as u32);
        floats(&mut words, &[self.one_84, self.zero_88, self.one_8c]);
        floats(&mut words, &self.zeros_90);
        floats(&mut words, &[self.floor_9c]);
        words.push(self.extra as u32);
        floats(&mut words, &self.ones_a4);
        floats(&mut words, &[self.zero_b0]);
        floats(&mut words, &self.ones_b4);
        floats(&mut words, &self.zeros_c0);
        words.extend([self.switches, self.none_170 as u32]);
        floats(&mut words, &[self.zero_174]);
        floats(&mut words, &self.ones_178);
        floats(&mut words, &self.zeros_184);
        floats(&mut words, &[self.ten_18c]);
        floats(&mut words, &self.zeros_190);
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

/// The record an object's own holder states at rest, which the stage is bound as the element
/// its draw record of noughts picks.
pub fn object_data_at_rest() -> Vec<u8> {
    ObjectData::AT_REST.bytes()
}

/// What the game leaves in the ped buffer `hash` for a part drawn alone, where it is stated.
pub(super) fn stated(hash: u32) -> Option<super::shading::Feed> {
    match hash {
        LAYERS => Some(super::shading::Feed::Stated(LayersHeader::AT_REST.bytes())),
        OBJECT_DATA => Some(super::shading::Feed::Stated(object_data_at_rest())),
        _ => None,
    }
}

/// The 48 bytes of [`DECORATION`] for one layer of a decoration preset, a dword a field.
// The game binds the component's own decoration object's, else one its kind's preset builds,
// else 48 bytes of noughts; a component past the ped's list of components is of no kind, so
// a part opened alone takes the noughts, `AT_REST`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecorationBuffer {
    /// The heap number of the layer's mask; nought, and the pixel program samples none.
    pub mask: u32,
    /// Not written.
    pub unused: u32,
    /// The heap number of the texture of the layer's material; nought samples none.
    pub material: u32,
    /// The layer's opacity, or one where its definition or material is opaque.
    pub opacity: f32,
    /// The product of the layer's, its definition's and its material's scales, each axis
    /// negated where one of them flips it.
    pub scale: [f32; 2],
    /// The sine and the cosine of the sum of the three rotations, which are stated in degrees.
    pub turn: [f32; 2],
    /// The sum of the three offsets.
    pub offset: [f32; 2],
    /// The material's strength.
    pub material_strength: f32,
    /// The definition's strength; the game builds no buffer where it or the opacity is nought.
    pub strength: f32,
}

const _: () = assert!(std::mem::size_of::<DecorationBuffer>() == 48);

impl DecorationBuffer {
    /// The 48 noughts the game binds where a component has no decoration.
    pub const AT_REST: Self = Self {
        mask: 0,
        unused: 0,
        material: 0,
        opacity: 0.0,
        scale: [0.0; 2],
        turn: [0.0; 2],
        offset: [0.0; 2],
        material_strength: 0.0,
        strength: 0.0,
    };
}

/// The 48 bytes of the buffer [`super::ped::COMPONENT`], a dword a field.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentBuffer {
    /// One over the height of the component's palette texture.
    pub palette_step: f32,
    /// The palette's three tints, a byte each, bits 24 and 25 the component's flags.
    pub tints: u32,
    /// Three bytes of the component's in its low bits; bit 24 picks bones stated as quaternions.
    pub flags: u32,
    /// Three dwords of the table the component's material entry names: all ones where it names none.
    pub material_table: [u32; 3],
    /// Two bits of the component's flags.
    pub flag_bits: u32,
    /// Where the component's layered material record starts: nought where it has none.
    pub layered_record: u32,
    /// -1 where the component has no wrinkle map: the pixel program then samples none of the three.
    pub wrinkles: i32,
    /// The heap numbers of the wrinkle mask, the wrinkle index and the wrinkle map.
    pub wrinkle_mask: u32,
    pub wrinkle_index: u32,
    pub wrinkle: u32,
}

const _: () = assert!(std::mem::size_of::<ComponentBuffer>() == 48);

impl ComponentBuffer {
    /// The buffer of a component that wears its maps with no palette, no table and no wrinkle map.
    // The palette is then the one-texel default, so its step is one and its tints white; the three
    // picked numbers are not read here, since the binder binds each picked texture itself.
    pub const AT_REST: Self = Self {
        palette_step: 1.0,
        tints: 0x00FF_FFFF,
        flags: 0,
        material_table: [u32::MAX; 3],
        flag_bits: 0,
        layered_record: 0,
        wrinkles: -1,
        wrinkle_mask: 0,
        wrinkle_index: 0,
        wrinkle: 0,
    };

    /// The buffer's bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let mut words = vec![self.palette_step.to_bits(), self.tints, self.flags];
        words.extend(self.material_table);
        words.extend([self.flag_bits, self.layered_record, self.wrinkles as u32]);
        words.extend([self.wrinkle_mask, self.wrinkle_index, self.wrinkle]);
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

/// Which of a component's maps a material texture takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Map {
    /// The texture keyed by the state's albedo word.
    Albedo,
    /// The texture keyed by the state's normal word.
    Normal,
    /// The texture the component asks for by this name.
    Named(u32),
}

/// One white texel: the default a component's record is reset with in most of its slots.
pub const WHITE: [u8; 4] = [0xFF; 4];
/// The texel the material map `mb` takes where the component has none.
pub const MB_DEFAULT: [u8; 4] = [0x00, 0x00, 0x7E, 0x00];
/// The texel the blood mask takes where the component has none.
pub const BLOOD_DEFAULT: [u8; 4] = [0x80, 0x80, 0x80, 0xFF];

/// The material textures a ped component's maps take, by the +0x14 hash, with each reset texel.
// The game binds by the parameter's own name: 0xC8E8C282, 0xD7C38241, 0xC1C01CC4, 0x0149D3C3,
// 0x12F4EBEB and 0x0814A7FC in this order; the ped effects state each under the hash here.
pub const COMPONENT_MAPS: [(u32, Map, [u8; 4]); 6] = [
    (0xC8E8_C282, Map::Albedo, WHITE),
    (0x1E77_BF4C, Map::Normal, WHITE),
    (0xC6F4_6E79, Map::Named(MA), WHITE),
    (0x739E_3D0D, Map::Named(MB), MB_DEFAULT),
    (0x12F4_EBEB, Map::Named(MC), WHITE),
    (0x0814_A7FC, Map::Named(BLOOD_MASK), BLOOD_DEFAULT),
];

/// The component's map and reset texel of the material texture the effect states as `stated`.
pub fn component_map(stated: u32) -> Option<(Map, [u8; 4])> {
    (COMPONENT_MAPS.iter())
        .find_map(|(hash, map, texel)| (*hash == stated).then_some((*map, *texel)))
}

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
    fn the_component_s_buffer_holds_each_field_at_the_dword_the_pixel_program_reads() {
        assert_eq!(offset_of!(ComponentBuffer, palette_step), 0);
        assert_eq!(offset_of!(ComponentBuffer, tints), 4);
        assert_eq!(offset_of!(ComponentBuffer, flags), 8);
        assert_eq!(offset_of!(ComponentBuffer, material_table), 12);
        assert_eq!(offset_of!(ComponentBuffer, flag_bits), 24);
        assert_eq!(offset_of!(ComponentBuffer, layered_record), 28);
        assert_eq!(offset_of!(ComponentBuffer, wrinkles), 32);
        assert_eq!(offset_of!(ComponentBuffer, wrinkle_mask), 36);
        assert_eq!(offset_of!(ComponentBuffer, wrinkle), 44);
        let bytes = ComponentBuffer::AT_REST.bytes();
        let dword = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(bytes.len(), 48);
        // Each field is written where `offset_of!` puts it, not in another order.
        assert_eq!(
            f32::from_bits(dword(offset_of!(ComponentBuffer, palette_step))),
            1.0
        );
        assert_eq!(dword(offset_of!(ComponentBuffer, tints)), 0x00FF_FFFF);
        for at in [12, 16, 20] {
            assert_eq!(dword(at), u32::MAX);
        }
        assert_eq!(dword(offset_of!(ComponentBuffer, wrinkles)), u32::MAX);
        assert_eq!(dword(offset_of!(ComponentBuffer, layered_record)), 0);
        assert_eq!(dword(offset_of!(ComponentBuffer, wrinkle)), 0);
    }

    #[test]
    fn the_decoration_buffer_holds_each_field_at_the_byte_the_game_writes_it_and_none_at_rest() {
        assert_eq!(offset_of!(DecorationBuffer, mask), 0);
        assert_eq!(offset_of!(DecorationBuffer, material), 8);
        assert_eq!(offset_of!(DecorationBuffer, opacity), 12);
        assert_eq!(offset_of!(DecorationBuffer, scale), 16);
        assert_eq!(offset_of!(DecorationBuffer, turn), 24);
        assert_eq!(offset_of!(DecorationBuffer, offset), 32);
        assert_eq!(offset_of!(DecorationBuffer, material_strength), 40);
        assert_eq!(offset_of!(DecorationBuffer, strength), 44);
        // The pixel program samples a decoration texture only where its number is not nought.
        let rest = DecorationBuffer::AT_REST;
        assert_eq!((rest.mask, rest.material), (0, 0));
        assert_eq!(rest.strength, 0.0);
    }

    #[test]
    fn a_component_s_maps_are_the_material_textures_the_game_binds_them_to() {
        assert_eq!(component_map(0xC8E8_C282), Some((Map::Albedo, WHITE)));
        assert_eq!(component_map(0x1E77_BF4C), Some((Map::Normal, WHITE)));
        assert_eq!(
            component_map(0x739E_3D0D),
            Some((Map::Named(MB), MB_DEFAULT))
        );
        assert_eq!(
            component_map(0x0814_A7FC),
            Some((Map::Named(BLOOD_MASK), BLOOD_DEFAULT))
        );
        // A material texture the component does not write, such as a texture array of the head's.
        assert_eq!(component_map(0x0CD2_7763), None);
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

    /// A stage that walks records while its count is under the first dword of its one constant
    /// buffer, testing before each turn as the head's own walk of its decorations does.
    fn counting() -> Lowered {
        walking("if s1_ >= s2_ { break; } s1_ = s1_ + 1u;")
    }

    /// The same walk leaving on an exact hit after its first step, which never meets nought.
    fn hitting() -> Lowered {
        walking("s1_ = s1_ + 1u; if s1_ == s2_ { break; }")
    }

    fn walking(turn: &str) -> Lowered {
        let text = format!(
            "@group(0) @binding(0) var<uniform> cb0_: vec4<u32>;
@fragment fn main() -> @location(0) vec4<f32> {{
var s1_: u32 = 0u;
var s2_: u32 = 0u;
s2_ = cb0_.x;
loop {{ {turn} }}
return vec4<f32>(f32(s1_));
}}"
        );
        let module = naga::front::wgsl::parse_str(&text).expect("the WGSL parses");
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
    fn a_count_to_the_decorations_of_a_component_with_none_is_a_stated_nought() {
        let fed = |buffer| feed(&native(), counting(), &table(buffer), &Sgd5Table::default());
        // The decorations' buffer is the game's default at rest: noughts, which the stage is fed.
        let stage = fed(DECORATIONS).expect("a count to the decorations is let through");
        let feeds: Vec<Feed> = stage.feeds.into_iter().map(|fed| fed.feed).collect();
        assert_eq!(feeds, [Feed::Zero(Some(DECORATIONS))]);
        // The same loop to a number of any other buffer only the game fills stays refused.
        let other = 0x1234_5678;
        assert_eq!(fed(other).err(), Some(Unfed::Loop(other)));
        // An exact hit after the first step never meets the nought: refused, not let through.
        let hit = feed(
            &native(),
            hitting(),
            &table(DECORATIONS),
            &Sgd5Table::default(),
        );
        assert_eq!(
            hit.err(),
            Some(Unfed::Endless(crate::rdna_bind::loops::MOST_TURNS))
        );
    }

    #[test]
    fn the_layers_buffer_at_rest_states_every_category_empty_just_past_its_header() {
        assert_eq!(offset_of!(LayersHeader, categories), 8);
        assert_eq!(offset_of!(LayersHeader, present), 32);
        assert_eq!(offset_of!(LayersHeader, last), 36);
        let bytes = LayersHeader::AT_REST.bytes();
        let dword = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(bytes.len(), 40);
        assert_eq!((dword(0), dword(4)), (0, 0));
        // The first category: records of 48 bytes from byte 40, none of them.
        assert_eq!(dword(8), 40 | 0x30 << 16);
        assert_eq!(dword(8) >> 28, 0);
        assert_eq!(dword(28), 40 | 0xE0 << 16);
        assert_eq!(dword(32), 40 << 16);
        assert_eq!(dword(36), 40 | 0x40 << 16);
        // A stage that counts by the buffer is fed those bytes, so the guard lets it through.
        let stage = feed(&native(), counting(), &table(LAYERS), &Sgd5Table::default())
            .expect("a count to the layers at rest is let through");
        let feeds: Vec<Feed> = stage.feeds.into_iter().map(|fed| fed.feed).collect();
        assert_eq!(feeds, [Feed::Stated(bytes)]);
    }

    #[test]
    fn the_object_data_holds_each_field_where_the_holder_writes_it() {
        let at = [
            (offset_of!(ObjectData, lead), 0x00),
            (offset_of!(ObjectData, zeros_08), 0x08),
            (offset_of!(ObjectData, colour), 0x14),
            (offset_of!(ObjectData, albedo_scale), 0x20),
            (offset_of!(ObjectData, ones_24), 0x24),
            (offset_of!(ObjectData, values_40), 0x40),
            (offset_of!(ObjectData, zeros_58), 0x58),
            (offset_of!(ObjectData, one_68), 0x68),
            (offset_of!(ObjectData, zeros_6c), 0x6C),
            (offset_of!(ObjectData, normal_bend), 0x74),
            (offset_of!(ObjectData, minus_one_78), 0x78),
            (offset_of!(ObjectData, zero_7c), 0x7C),
            (offset_of!(ObjectData, none_80), 0x80),
            (offset_of!(ObjectData, one_84), 0x84),
            (offset_of!(ObjectData, zero_88), 0x88),
            (offset_of!(ObjectData, one_8c), 0x8C),
            (offset_of!(ObjectData, zeros_90), 0x90),
            (offset_of!(ObjectData, floor_9c), 0x9C),
            (offset_of!(ObjectData, extra), 0xA0),
            (offset_of!(ObjectData, ones_a4), 0xA4),
            (offset_of!(ObjectData, zero_b0), 0xB0),
            (offset_of!(ObjectData, ones_b4), 0xB4),
            (offset_of!(ObjectData, zeros_c0), 0xC0),
            (offset_of!(ObjectData, switches), 0x16C),
            (offset_of!(ObjectData, none_170), 0x170),
            (offset_of!(ObjectData, zero_174), 0x174),
            (offset_of!(ObjectData, ones_178), 0x178),
            (offset_of!(ObjectData, zeros_184), 0x184),
            (offset_of!(ObjectData, ten_18c), 0x18C),
            (offset_of!(ObjectData, zeros_190), 0x190),
        ];
        for (offset, byte) in at {
            assert_eq!(offset, byte);
        }
        // `bytes` writes each field at its `offset_of!` place, not in another order.
        let bytes = ObjectData::AT_REST.bytes();
        let float = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let dword = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(bytes.len(), 416);
        assert_eq!(float(offset_of!(ObjectData, albedo_scale)), 1.0);
        assert_eq!(float(offset_of!(ObjectData, colour) + 8), 2.0 / 255.0);
        assert_eq!(float(offset_of!(ObjectData, values_40) + 4), 16.0);
        assert_eq!(float(offset_of!(ObjectData, normal_bend)), 1.0);
        assert_eq!(float(offset_of!(ObjectData, floor_9c)), -1000.0);
        assert_eq!(dword(offset_of!(ObjectData, extra)), u32::MAX);
        assert_eq!(dword(offset_of!(ObjectData, switches)), 0);
        assert_eq!(dword(offset_of!(ObjectData, none_170)), u32::MAX);
        assert_eq!(float(offset_of!(ObjectData, ten_18c)), 10.0);
        assert_eq!(float(0x19C), 0.0);
    }

    #[test]
    fn an_object_s_own_data_is_stated_with_an_albedo_scale_of_one() {
        let bytes = object_data_at_rest();
        let float = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(bytes.len(), 416);
        assert_eq!(float(32), 1.0, "the float a ped's albedo is multiplied by");
        assert_eq!((float(0x68), float(0x84), float(0x180)), (1.0, 1.0, 1.0));
        let stage = feed(
            &native(),
            // Read, not counted to: its first dword is no count.
            walking("break;"),
            &table(OBJECT_DATA),
            &Sgd5Table::default(),
        )
        .expect("a stage reading the object data is let through");
        let feeds: Vec<Feed> = stage.feeds.into_iter().map(|fed| fed.feed).collect();
        assert_eq!(feeds, [Feed::Stated(bytes)]);
    }
}
