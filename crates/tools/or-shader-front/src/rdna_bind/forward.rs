//! A forward pass of a GTA VI model: a pixel program that colours or lights a surface itself and
//! is blended over the lit frame, and what the game leaves in the bindings only it fills while
//! the systems that write them are off.

use or_io::gta5::shaders::sgd5::Sgd5Table;

use super::forward_frame::{frame_of, Frame, VIEW_LIGHTING};
use super::shading::{Fed, Feed, Unfed};
use super::{join, numbered, Class, Join, Place, Record};
use crate::dxil::ShaderKind;
use crate::rdna::{Index, IndexSource, Resource, ResourceKind};

/// The colour targets a forward pixel program exports, by what they hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forward {
    /// One colour, already multiplied by its alpha, laid over what is behind it.
    Emissive,
    /// A light the surface adds and a colour what is behind it is multiplied by, in two targets.
    Lit,
}

/// A factor of a blend, as an effect's blend record numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Factor {
    Zero,
    One,
    OneMinusSourceAlpha,
    /// The second colour the pixel program exports (its target 1), blended with the first.
    SecondColour,
    /// That second colour's alpha.
    SecondAlpha,
}

/// The faces of a surface a forward pass draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Faces {
    /// Those whose corners run counter-clockwise on the screen: the engine's own rasterizer.
    Front,
    /// The others: a pass that draws a glass's inside before its outside.
    Back,
}

/// How a forward pass's colour is laid over the frame.
// The colour's source and destination factors, the alpha's, each summed, and the channels
// written, a bit each from red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blend {
    pub colour: [Factor; 2],
    pub alpha: [Factor; 2],
    pub writes: u8,
    pub faces: Faces,
}

/// The engine's own state for a lit forward pass that states no blend of its own.
// The light the surface adds (target 0) plus the frame times what the surface lets through
// (target 1), every channel written, back faces culled; depth tested and not written.
const ENGINE_LIT: Blend = Blend {
    colour: [Factor::One, Factor::SecondColour],
    alpha: [Factor::One, Factor::SecondAlpha],
    writes: 0b1111,
    faces: Faces::Front,
};

/// The techniques whose lit passes the engine draws with [`ENGINE_LIT`], by name: the view's
/// alpha list (a technique of the plain draw group), and the mirror's forward list.
// Each of the alpha passes' own lists (`alphaPrepass`, `alphaTransmission`, `alphaOit`, their
// spatter and fire forms) and `fire_` set states of their own, which are not read.
const LIT_TECHNIQUES: [&str; 4] = [
    "drawmeshlets",
    "drawskinnedmeshlets",
    "forwardmirror_drawmeshlets",
    "forwardmirror_drawskinnedmeshlets",
];

/// The suffix of a lit pixel program's name that draws the back faces of its surface first.
const BACK_FACES: &str = "_BackFace";

/// `blend` for a pass whose pixel program is named `program`: one named for back faces draws those.
// As the pass's own rasterizer record states: `standard_glass`'s first pass culls the front.
pub fn faces_of(blend: Blend, program: &str) -> Blend {
    match program.ends_with(BACK_FACES) {
        true => Blend {
            faces: Faces::Back,
            ..blend
        },
        false => blend,
    }
}

/// A target written whole, and the first export target that is no colour target.
const WHOLE: u8 = 0xF;
const COLOUR_TARGETS: u8 = 8;

/// The forward pass `exports` are, where they are one: a colour target written whole, or two.
pub fn forward_of(exports: &[(u8, u8)]) -> Option<Forward> {
    let mut colour: Vec<(u8, u8)> = (exports.iter().copied())
        .filter(|(target, _)| *target < COLOUR_TARGETS)
        .collect();
    colour.sort_unstable();
    colour.dedup();
    match colour.as_slice() {
        [(0, WHOLE)] => Some(Forward::Emissive),
        [(0, WHOLE), (1, WHOLE)] => Some(Forward::Lit),
        _ => None,
    }
}

impl Forward {
    /// The blend its pass draws with in the view's own list of forward surfaces.
    // The one colour target is laid on by the blend record `standard_emissive`'s pass names:
    // ONE and ONE less the source's alpha on the colour, ZERO and ONE on the alpha, red, green and
    // blue written. A lit pass names no blend record: it draws with the engine's own state.
    pub fn blend(self) -> Result<Blend, Unfed> {
        self.blend_in(LIT_TECHNIQUES[0])
    }

    /// The blend of its pass in the technique `technique`; `Err` where that list's state is not read.
    pub fn blend_in(self, technique: &str) -> Result<Blend, Unfed> {
        match self {
            Self::Emissive => Ok(Blend {
                colour: [Factor::One, Factor::OneMinusSourceAlpha],
                alpha: [Factor::Zero, Factor::One],
                writes: 0b0111,
                faces: Faces::Front,
            }),
            Self::Lit if LIT_TECHNIQUES.contains(&technique) => Ok(ENGINE_LIT),
            Self::Lit => Err(Unfed::Blend),
        }
    }
}

/// The draw's fade record: the entity's fade at byte 0, four weights of a two by two stipple from
/// byte 16, and a pixel whose weight is under one is discarded; the rest at nought switches the
/// alpha system's depth test and the water's clip off.
// A lit program takes the light it adds and the colour it lets through towards nothing and towards
// one by the fade: at nought a lit surface is not there. The engine sets it to one for a draw
// that is not fading.
const FADE: u32 = 0xE688_861E;
/// The fade of an entity drawn whole, and the stipple's four weights.
const FADE_WHOLE: f32 = 1.0;
/// The frame's scale, 16 bytes: what multiplies the colour a pass exports (+4), what it samples
/// of light the frame's size (+0) and the probe's light (+8). One where nothing pre-exposes.
const FRAME_SCALE: u32 = 0xDAAF_2AC8;
/// `g_TransmittanceVolume`, the fog's transmittance, which multiplies the colour.
const TRANSMITTANCE: u32 = 0xBFBD_3BEC;
/// The screen layer's transmittance, which stands in for the fog's where that layer is entered.
const LAYER_TRANSMITTANCE: u32 = 0x8395_00C2;
/// The pools of an entity's records a forward program multiplies its colour by: a light's
/// intensity, colour and two maps (24 bytes a record), and a scale in the high half of +12 (32).
const LIGHT_RECORDS: u32 = 0x7543_300E;
const ENTITY_RECORDS: u32 = 0x598E_AD6B;

/// The first record of the pool `0x7543300E` (24 bytes), which a draw whose record is noughts picks.
// The game makes it at start-up, before any entity takes a record of the pool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightRecord {
    /// What the emissive colour is multiplied by; with its sign bit set the maps are passed over.
    pub intensity: f32,
    /// Red, green, blue and a fourth byte, each over 255, multiplying the colour.
    pub colour: [u8; 4],
    /// The place both maps are sampled at.
    pub at: [f32; 2],
    /// The heap entries of two maps: the first's colour times the second's first channel.
    pub maps: [u32; 2],
}

impl LightRecord {
    /// The record the game makes: one intensity, white, both maps the heap's first entry.
    pub const AT_REST: Self = Self {
        intensity: 1.0,
        colour: [0xFF; 4],
        at: [0.0, -1.0],
        maps: [0, 0],
    };

    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(24);
        bytes.extend(self.intensity.to_le_bytes());
        bytes.extend(self.colour);
        self.at.iter().for_each(|at| bytes.extend(at.to_le_bytes()));
        self.maps
            .iter()
            .for_each(|map| bytes.extend(map.to_le_bytes()));
        bytes
    }
}

/// The first record of the pool `0x598EAD6B` (32 bytes), which a draw whose record is noughts picks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityRecord {
    pub head: u32,
    /// Bit 2 asks for the light taken to grey, 0x8004 for the last fade by the water.
    pub flags: u32,
    /// A word no forward program reads.
    pub third: u32,
    /// Its high half over 65,535 scales the emissive colour.
    pub scales: u32,
    pub rest: [u32; 4],
}

impl EntityRecord {
    /// The record the game makes: no flag, the emissive colour at its whole.
    pub const AT_REST: Self = Self {
        head: 0,
        flags: 0,
        third: 0xFFFF_FF00,
        scales: 0xFFFF_0000,
        rest: [0; 4],
    };

    pub fn bytes(&self) -> Vec<u8> {
        let words = [[self.head, self.flags, self.third, self.scales], self.rest];
        words
            .iter()
            .flatten()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }
}

/// `g_AmbientCube`: 32 records of 12 bytes, of which a lit program reads one scalar, the x of the
/// eighth (+0x54), that multiplies the directional light.
const AMBIENT_CUBE: u32 = 0xBD93_BF83;
const AMBIENT_CUBE_BYTES: usize = 32 * 12;
/// The air's transmittance by height, a row of texels that multiplies the directional light.
const TRANSMITTANCE_ROW: u32 = 0x136D_4AF8;
// The directional light the frame hands a lit stage is the editor's whole sun, the air's
// transmittance at the model among it: so the cube's scalar is one and the row is white. The
// clouds' shadow `0xC6F8AAEE` stays noughts, a depth of nought through the clouds: what the
// program takes of it is then `saturate(1 + [+0x248])` of the view's record, which carries the
// editor's share.
// The probes' cell words `0xC44BE1D8` and records `0x669A6134` stay noughts: a cell's word of
// nought names the probe of the atlas's first tile, in no room, and the records are read only by
// the block of light from all around, which the second of the two probe levels the frame states
// keeps the pixel from. The visibility `0x856ACFEF` and the light the frame fills.
// The cascades' map `0x78262FB4` stays noughts too: a depth of nought is the game's own texel for
// it while its shadows are off, and every cascade [`CascadeRecord::AT_REST`] states compares in
// front of it.

/// The cascade shadows' constant buffer, 656 bytes.
const CASCADES: u32 = 0xA6D7_79F9;
const CASCADE_BYTES: usize = 656;
/// The frame record, 2,048 bytes.
const FRAME_RECORD: u32 = 0x6E87_25E4;
const FRAME_RECORD_BYTES: usize = 2048;
/// The frame record's scale of the roughness a surface's normal adds over a pixel, plus one: the
/// game's own setting at start-up.
const FRAME_NORMAL_ROUGHNESS: (usize, f32) = (0x620, 0.5);

/// The cascades' buffer for a sun whose map holds nothing nearer to it than any surface: every
/// tap a cascade compares is lit, so a lit surface takes the whole directional light.
// The one tile of the editor's own shadow is not handed to a forward pass: where the program
// places a pixel in a cascade (rows +0x0..+0x2F, then a scale and an offset a cascade) is read,
// what world place it starts from is not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CascadeRecord {
    /// How many cascades the sun casts into (+0x3C), as a float; nought passes the whole
    /// directional light over.
    pub count: f32,
    /// The depth each of the four cascades compares at, before its scale (+0xD8, 16 bytes apart).
    pub depths: [f32; 4],
    /// What the slope bias is divided by (+0x150).
    pub slope_divisor: f32,
    /// A texel of the map across and down, the four taps' spacing (+0x158, +0x15C).
    pub texel: [f32; 2],
}

impl CascadeRecord {
    /// One cascade, every place of each at a depth of -1, under the map's nought: a compared
    /// tap passes where its depth is no greater than the map's.
    pub const AT_REST: Self = Self {
        count: 1.0,
        depths: [-1.0; 4],
        slope_divisor: 1.0,
        texel: [1.0, 1.0],
    };

    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = floats_at(CASCADE_BYTES, 0x3C, &[self.count]);
        for (cascade, depth) in self.depths.iter().enumerate() {
            let at = 0xD8 + 16 * cascade;
            bytes[at..at + 4].copy_from_slice(&depth.to_le_bytes());
        }
        let tail = [self.slope_divisor, 0.0, self.texel[0], self.texel[1]];
        for (i, float) in tail.iter().enumerate() {
            bytes[0x150 + 4 * i..0x154 + 4 * i].copy_from_slice(&float.to_le_bytes());
        }
        bytes
    }
}

/// What the game leaves in a binding only it fills, while the system that writes it is off.
#[derive(Debug, Clone, PartialEq)]
enum Rest {
    /// Bytes, the rest noughts.
    Bytes(Vec<u8>),
    /// The game's one white texel.
    White,
    /// What the renderer fills from the frame it lights.
    Frame(Frame),
}

/// Noughts of `len` bytes with `floats` from byte `at`.
fn floats_at(len: usize, at: usize, floats: &[f32]) -> Vec<u8> {
    let mut bytes = vec![0; len];
    for (i, float) in floats.iter().enumerate() {
        bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&float.to_le_bytes());
    }
    bytes
}

/// What the game leaves in the binding named `hash` of a `forward` program where noughts are not it.
// The emissive program reads of the view's lighting record and the frame record only the fog's
// lookup and the frame's size, count and flags, whose noughts pass the fog and the water's clip
// over; the lit program lights by them.
fn rest_of(forward: Forward, hash: u32) -> Option<Rest> {
    let lit = forward == Forward::Lit;
    match hash {
        FADE => {
            let mut bytes = floats_at(128, 16, &[FADE_WHOLE; 4]);
            bytes[..4].copy_from_slice(&FADE_WHOLE.to_le_bytes());
            Some(Rest::Bytes(bytes))
        }
        FRAME_SCALE => Some(Rest::Bytes(floats_at(16, 0, &[1.0; 3]))),
        TRANSMITTANCE | LAYER_TRANSMITTANCE => Some(Rest::White),
        LIGHT_RECORDS => Some(Rest::Bytes(LightRecord::AT_REST.bytes())),
        ENTITY_RECORDS => Some(Rest::Bytes(EntityRecord::AT_REST.bytes())),
        AMBIENT_CUBE if lit => Some(Rest::Bytes(floats_at(AMBIENT_CUBE_BYTES, 0x54, &[1.0]))),
        TRANSMITTANCE_ROW if lit => Some(Rest::White),
        CASCADES if lit => Some(Rest::Bytes(CascadeRecord::AT_REST.bytes())),
        FRAME_RECORD if lit => {
            let (at, scale) = FRAME_NORMAL_ROUGHNESS;
            Some(Rest::Bytes(floats_at(FRAME_RECORD_BYTES, at, &[scale])))
        }
        _ if lit => frame_of(hash).map(Rest::Frame),
        _ => None,
    }
}

/// `feed`, one binding of a `forward` stage, as the game leaves it at rest: a binding only the
/// game fills whose noughts are its rest stays noughts. `Err` names one it is not given.
pub(super) fn at_rest(forward: Forward, feed: Feed) -> Result<Feed, Unfed> {
    let Feed::Zero(Some(hash)) = feed else {
        return Ok(feed);
    };
    match rest_of(forward, hash) {
        Some(Rest::Bytes(bytes)) => Ok(Feed::Stated(bytes)),
        Some(Rest::White) => Ok(Feed::White),
        Some(Rest::Frame(frame)) => Ok(Feed::Frame(frame)),
        None => Ok(feed),
    }
}

/// The places of the view's lighting record a lit stage may read, by byte, half open: those the
/// frame states, and those the stated record keeps a stage from reaching.
// Stated: the way to the light (+0xF8 is loaded with it and not used), the roughness modifiers,
// the haze, the disc, the fog's lookup, the clouds' share, the alpha fade, the probes' jitter,
// grid and first level, the level count, the glass's remap, the ambient map's place and the
// reflection's scale, and the probes' atlas, number, tile and cells. Not reached: +0x0 and
// +0xDC..+0xEB in the local lights' loop (no light).
const VIEW_LIGHTING_READ: [(u32, u32); 16] = [
    (0x0, 0x10),
    (0xDC, 0xFC),
    (0x108, 0x120),
    (0x230, 0x238),
    (0x240, 0x258),
    (0x3DC, 0x3E4),
    (0x3E8, 0x3EC),
    (0x408, 0x410),
    (0x4B0, 0x4C0),
    (0x50C, 0x510),
    (0x550, 0x558),
    (0x568, 0x56C),
    (0x588, 0x594),
    (0x72C, 0x730),
    (0x750, 0x760),
    (0x7CC, 0x7DC),
];

/// The byte a place `reads` gives of a constant buffer stands for: a row of four dwords and a
/// dword of it, or a dword; `None` for a place a computed index picks.
fn byte_of(place: &str) -> Option<u32> {
    let numbers: Vec<u32> = place
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match numbers.as_slice() {
        [row, dword] => Some(row * 16 + dword * 4),
        [dword] => Some(dword * 4),
        _ => None,
    }
}

/// Why a lit stage `module` is refused for a place of the view's lighting record it reads at a
/// stated index that the frame neither states nor keeps it from reaching.
// ponytail: the constant-buffer form alone; the record's twin read by address starts after a
// header, and its stated places are not checked. Check them when a stage reads one.
pub(super) fn unestablished(module: &naga::Module, feeds: &[Fed]) -> Option<Unfed> {
    let read = super::shading::reads(module);
    let viewed = (feeds.iter())
        .filter(|fed| fed.feed == Feed::Frame(Frame::ViewLighting) && fed.header.is_none());
    let allowed =
        |byte: u32| (VIEW_LIGHTING_READ.iter()).any(|(from, to)| (*from..*to).contains(&byte));
    for fed in viewed {
        let places = read.get(&(fed.group, fed.binding)).into_iter().flatten();
        let unread: Vec<String> = (places.filter_map(|place| byte_of(place)))
            .filter(|byte| !allowed(*byte))
            .map(|byte| format!("+{byte:#X}"))
            .collect();
        if !unread.is_empty() {
            tracing::info!(
                places = unread.join(" "),
                "a lit stage reads places of the view's lighting record nothing states"
            );
            return Some(Unfed::Unread(VIEW_LIGHTING));
        }
    }
    None
}

/// The dword of the view's lighting record that numbers the heap entry of the probes' light.
// Sampled only in the second probe block, the reflection's, at the place the record's tile and
// atlas fields give: the frame fills it as it fills the record.
const PROBE_TEXTURE: u32 = 0x568;
/// The image instruction's dimension field of a plain two-dimensional texture.
const DIM_2D: u8 = 1;
/// A number shifted by this reaches its heap entry.
const ENTRY_SHIFT: u8 = 5;

/// What fills the texture `resource` a lit `kind` program picks from the heap, the table two
/// past the draw's dwords, by a number of the view's lighting record; `None` for any other pick.
pub(super) fn picked(
    resource: &Resource,
    kind: ShaderKind,
    records: &[Record],
    table: &Sgd5Table,
) -> Option<Feed> {
    let (ResourceKind::Texture { dim: DIM_2D, .. }, None) = (&resource.kind, resource.count) else {
        return None;
    };
    let at = resource.at.as_ref()?;
    let draw = records.iter().find(|r| r.class == Class::RootConstants)?;
    let heap = match numbered(Place::Words { root: at.root }, kind) {
        Place::Words { root } => root,
        _ => at.root,
    };
    if !at.path.is_empty() || heap != draw.root.wrapping_add(2) {
        return None;
    }
    let Some(Index {
        source:
            IndexSource::Data {
                buffer,
                offset: PROBE_TEXTURE,
                plus: None,
            },
        shift: ENTRY_SHIFT,
    }) = &at.index
    else {
        return None;
    };
    if !buffer.path.is_empty() || buffer.index.is_some() {
        return None;
    }
    let place = Place::Table {
        root: buffer.root,
        offset: buffer.offset,
    };
    match join(numbered(place, kind), records, table) {
        Join::Named { named, .. } if named.name_hash == VIEW_LIGHTING => {
            Some(Feed::Frame(Frame::ProbeRadiance))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lit_stage_s_probe_texture_picked_by_the_view_s_record_is_the_frame_s_probe_light() {
        use crate::rdna::{Loc, Lowered, Program};
        use crate::rdna_bind::shading::feed;
        use crate::rdna_bind::{Native, Unbound};
        use or_io::gta5::shaders::sgd5::Sgd5Resource;

        const DRAW: u8 = 12;
        const TABLE: u8 = 9;
        let loc = |root, offset, index| Loc {
            root,
            path: Vec::new(),
            offset,
            index,
        };
        let native = |exports: &[(u32, u32)]| Native {
            program: Program {
                kind: ShaderKind::Pixel,
                slot: 5,
                words: (exports.iter())
                    .flat_map(|(target, en)| [(0x3E << 26) | (target << 4) | en, 0])
                    .collect(),
                wave32: false,
                inputs: Vec::new(),
                vertex_inputs: Vec::new(),
                position_read: 0,
                front_face: false,
                launch: None,
            },
            records: [(Class::Cbv, 7, TABLE), (Class::RootConstants, 0, DRAW)]
                .map(|(class, register, root)| Record {
                    class,
                    register,
                    root,
                    offset: 0,
                    dwords: 4,
                    stride: 0,
                })
                .to_vec(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let table = |name| Sgd5Table {
            resources: vec![Sgd5Resource {
                code: 0x30,
                flags: 0,
                slot: 7,
                members: 0,
                buffer_size: 2044,
                name_hash: name,
                semantic_hash: name,
            }],
            ..Sgd5Table::default()
        };
        // The record, and a plain texture of the heap picked by its dword at `byte`.
        let lowered = |byte| {
            let number = Index {
                source: IndexSource::Data {
                    buffer: Box::new(loc(TABLE, 0, None)),
                    offset: byte,
                    plus: None,
                },
                shift: ENTRY_SHIFT,
            };
            let resource = |kind, at, binding| Resource {
                kind,
                at: Some(at),
                words: None,
                group: 0,
                binding,
                count: None,
                written: false,
            };
            let texture = ResourceKind::Texture {
                dim: DIM_2D,
                compared: false,
            };
            Lowered {
                module: naga::Module::default(),
                resources: vec![
                    resource(
                        ResourceKind::ConstantBuffer { rows: 128 },
                        loc(TABLE, 0, None),
                        0,
                    ),
                    resource(texture, loc(DRAW + 2, 0, Some(number)), 1),
                ],
                approximations: Default::default(),
                half_slots: Default::default(),
                subgroups: false,
                intersections: Vec::new(),
                region: None,
            }
        };
        let lit = [(9, 0), (0, 15), (1, 15)];
        let fed = |exports: &[(u32, u32)], byte, name| {
            let stage = feed(&native(exports), lowered(byte), &table(name), &table(name))?;
            Ok::<_, Unfed>(
                stage
                    .feeds
                    .into_iter()
                    .map(|fed| fed.feed)
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            fed(&lit, PROBE_TEXTURE, VIEW_LIGHTING),
            Ok(vec![
                Feed::Frame(Frame::ViewLighting),
                Feed::Frame(Frame::ProbeRadiance)
            ])
        );
        // Another dword, another record, or a program that is not lit: a number nothing states.
        let refused = Err(Unfed::Unbound(Unbound::Picked));
        assert_eq!(fed(&lit, PROBE_TEXTURE + 4, VIEW_LIGHTING), refused);
        assert_eq!(fed(&lit, PROBE_TEXTURE, 0x1234_5678), refused);
        assert_eq!(fed(&[(0, 15)], PROBE_TEXTURE, VIEW_LIGHTING), refused);
    }

    #[test]
    fn a_forward_pass_is_told_by_its_colour_targets_and_a_deferred_one_is_none() {
        // The emissive program's: target 0 whole, and the export of no target.
        assert_eq!(forward_of(&[(0, 15), (9, 0)]), Some(Forward::Emissive));
        // The lit alpha program's, the null export first and a target exported twice.
        let lit = [(9, 0), (0, 15), (1, 15), (0, 15)];
        assert_eq!(forward_of(&lit), Some(Forward::Lit));
        let deferred = [(0, 15), (1, 15), (2, 15), (3, 15), (4, 3)];
        assert_eq!(forward_of(&deferred), None);
        assert_eq!(forward_of(&[(0, 7)]), None, "a target not written whole");
        assert_eq!(forward_of(&[(9, 0)]), None, "no colour");
    }

    #[test]
    fn only_a_blend_an_effect_or_the_engine_states_is_drawn_with() {
        let emissive = Forward::Emissive.blend().expect("stated");
        assert_eq!(emissive.colour, [Factor::One, Factor::OneMinusSourceAlpha]);
        assert_eq!(emissive.alpha, [Factor::Zero, Factor::One]);
        assert_eq!(
            emissive.writes, 0b0111,
            "red, green and blue; the alpha kept"
        );
        // A lit pass of the view's alpha list: its light plus the frame times what it lets through.
        let lit = Forward::Lit.blend_in("drawmeshlets").expect("the engine's");
        assert_eq!(lit.colour, [Factor::One, Factor::SecondColour]);
        assert_eq!(lit.alpha, [Factor::One, Factor::SecondAlpha]);
        assert_eq!((lit.writes, lit.faces), (0b1111, Faces::Front));
        assert_eq!(
            Forward::Lit.blend(),
            Ok(lit),
            "the view's list is the plain one"
        );
        assert_eq!(Forward::Lit.blend_in("drawskinnedmeshlets"), Ok(lit));
        // The alpha passes' own lists set states that are not read.
        for technique in [
            "alphaTransmission_drawmeshlets",
            "alphaPrepass_drawmeshlets",
            "fire_drawmeshlets",
        ] {
            assert_eq!(
                Forward::Lit.blend_in(technique),
                Err(Unfed::Blend),
                "{technique}"
            );
        }
        assert_eq!(
            Forward::Emissive.blend_in("cube_drawmeshlets"),
            Ok(emissive)
        );
        // A glass's first pass draws its inside.
        assert_eq!(
            faces_of(lit, "PS_Textured_Eight_BackFace").faces,
            Faces::Back
        );
        assert_eq!(faces_of(lit, "PS_Textured_Eight"), lit);
    }

    #[test]
    fn each_pool_s_first_record_is_what_the_game_makes_at_its_offsets() {
        let word = |bytes: &[u8], at: usize| {
            u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
        };
        let light = LightRecord::AT_REST.bytes();
        assert_eq!(light.len(), 24);
        assert_eq!(word(&light, 0), 1.0f32.to_bits(), "the intensity at +0");
        assert_eq!(word(&light, 4), 0xFFFF_FFFF, "white at +4");
        assert_eq!(word(&light, 8), 0.0f32.to_bits());
        assert_eq!(word(&light, 12), (-1.0f32).to_bits(), "the place at +8");
        assert_eq!(
            [word(&light, 16), word(&light, 20)],
            [0, 0],
            "the maps at +16"
        );
        let entity = EntityRecord::AT_REST.bytes();
        assert_eq!(entity.len(), 32);
        assert_eq!(word(&entity, 4), 0, "no flag at +4");
        assert_eq!(word(&entity, 8), 0xFFFF_FF00);
        assert_eq!(
            word(&entity, 12) >> 16,
            0xFFFF,
            "the emissive scale whole at +12"
        );
        assert!(entity[16..].iter().all(|b| *b == 0));
    }

    #[test]
    fn a_lit_stage_is_handed_the_view_s_record_the_cascades_at_rest_and_the_probes_noughts() {
        // The emissive program reads the view's record for its fog alone: noughts are its rest.
        let view = Feed::Zero(Some(0x0048_742C));
        assert_eq!(at_rest(Forward::Emissive, view.clone()), Ok(view.clone()));
        // A lit one is handed the frame's, and the environment fit's table.
        assert_eq!(
            at_rest(Forward::Lit, view),
            Ok(Feed::Frame(Frame::ViewLighting))
        );
        let fit = Feed::Zero(Some(0x410A_2FC7));
        assert_eq!(
            at_rest(Forward::Lit, fit),
            Ok(Feed::Frame(Frame::EnvironmentFit))
        );
        // The probes' cell words and records and the cascades' map stay noughts; the probes'
        // visibility is the frame's.
        for nought in [0xC44B_E1D8, 0x669A_6134, 0x7826_2FB4] {
            let fed = at_rest(Forward::Lit, Feed::Zero(Some(nought)));
            assert_eq!(fed, Ok(Feed::Zero(Some(nought))), "0x{nought:08X}");
        }
        assert_eq!(
            at_rest(Forward::Lit, Feed::Zero(Some(0x856A_CFEF))),
            Ok(Feed::Frame(Frame::ProbeVisibility))
        );
        assert_eq!(
            at_rest(Forward::Lit, Feed::Zero(Some(CASCADES))),
            Ok(Feed::Stated(CascadeRecord::AT_REST.bytes()))
        );
        let Ok(Feed::Stated(frame)) = at_rest(Forward::Lit, Feed::Zero(Some(FRAME_RECORD))) else {
            panic!("the frame record is not stated");
        };
        assert_eq!(frame.len(), 2048);
        assert_eq!(
            frame[0x620..0x624],
            0.5f32.to_le_bytes(),
            "the normal's roughness scale"
        );
        let mut elsewhere = frame[..0x620].iter().chain(&frame[0x624..]);
        assert!(
            elsewhere.all(|b| *b == 0),
            "the switches, flags and count at nought"
        );
    }

    #[test]
    fn the_cascades_at_rest_state_one_cascade_whose_every_tap_is_lit() {
        let float = |bytes: &[u8], at: usize| {
            f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
        };
        let bytes = CascadeRecord::AT_REST.bytes();
        assert_eq!(bytes.len(), 656);
        // Nought cascades pass the directional light over whole.
        assert_eq!(float(&bytes, 0x3C), 1.0, "one cascade");
        for cascade in 0..4 {
            assert_eq!(
                float(&bytes, 0xD8 + 16 * cascade),
                -1.0,
                "cascade {cascade}'s depth"
            );
        }
        assert_eq!(
            float(&bytes, 0x150),
            1.0,
            "the slope bias's divisor, kept finite"
        );
        assert_eq!([float(&bytes, 0x158), float(&bytes, 0x15C)], [1.0, 1.0]);
        // The rows that place a pixel in a cascade, the biases and the clouds' matrix: noughts.
        let stated = [0x3C, 0xD8, 0xE8, 0xF8, 0x108, 0x150, 0x158, 0x15C];
        let mut others = (0..656).filter(|b| !stated.iter().any(|at| (*at..*at + 4).contains(b)));
        assert!(others.all(|b| bytes[b] == 0));
        // The game's map at rest holds a depth of nought; every cascade compares under it.
        assert!(CascadeRecord::AT_REST
            .depths
            .iter()
            .all(|depth| *depth <= 0.0));
    }

    #[test]
    fn a_place_of_the_view_s_record_is_read_by_its_row_and_dword() {
        assert_eq!(byte_of("114.3"), Some(0x72C));
        assert_eq!(byte_of("459"), Some(0x72C));
        assert_eq!(byte_of("*.1"), None);
        assert_eq!(byte_of("*"), None);
    }

    #[test]
    fn a_lit_stage_is_handed_the_frame_s_directional_light_and_light_from_all_around() {
        let at_rest = |hash| at_rest(Forward::Lit, Feed::Zero(Some(hash)));
        assert_eq!(
            at_rest(0xA96F_368C),
            Ok(Feed::Frame(Frame::DirectionalLight))
        );
        assert_eq!(at_rest(0x3094_0D63), Ok(Feed::Frame(Frame::AmbientMap)));
        // The sun the frame hands is whole: the cube's scalar is one and the air's row white.
        let Ok(Feed::Stated(cube)) = at_rest(AMBIENT_CUBE) else {
            panic!("the ambient cube is not stated");
        };
        assert_eq!(cube.len(), 384);
        assert_eq!(cube[0x54..0x58], 1.0f32.to_le_bytes());
        assert!(cube[..0x54].iter().chain(&cube[0x58..]).all(|b| *b == 0));
        assert_eq!(at_rest(TRANSMITTANCE_ROW), Ok(Feed::White));
        // A depth of nought through the clouds: the view's record carries their share.
        assert_eq!(at_rest(0xC6F8_AAEE), Ok(Feed::Zero(Some(0xC6F8_AAEE))));
        // The emissive program reads none of them: they stay as they come.
        let cube = Feed::Zero(Some(AMBIENT_CUBE));
        assert_eq!(super::at_rest(Forward::Emissive, cube.clone()), Ok(cube));
    }

    #[test]
    fn a_forward_stage_is_given_the_game_s_rest_and_an_emissive_one_is_fed_whole() {
        let at_rest = |feed| at_rest(Forward::Emissive, feed);
        let float = |bytes: &[u8], at: usize| {
            f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
        };
        // Every pixel is discarded under a stipple weight of nought: the rest is four ones.
        let Ok(Feed::Stated(fade)) = at_rest(Feed::Zero(Some(FADE))) else {
            panic!("the fade record is not stated");
        };
        assert_eq!(fade.len(), 128);
        assert!((16..32).step_by(4).all(|at| float(&fade, at) == 1.0));
        // A lit surface faded to nought adds no light and lets everything through: the engine's
        // fade for a draw that is not fading is one.
        assert_eq!(float(&fade, 0), 1.0, "the entity's fade");
        assert!(fade[4..16].iter().chain(&fade[32..]).all(|b| *b == 0));
        let Ok(Feed::Stated(scale)) = at_rest(Feed::Zero(Some(FRAME_SCALE))) else {
            panic!("the frame's scale is not stated");
        };
        assert_eq!(
            [0, 4, 8, 12].map(|at| float(&scale, at)),
            [1.0, 1.0, 1.0, 0.0]
        );
        assert_eq!(at_rest(Feed::Zero(Some(TRANSMITTANCE))), Ok(Feed::White));
        // An entity's records: the first of each pool, as the game makes them.
        assert_eq!(
            at_rest(Feed::Zero(Some(LIGHT_RECORDS))),
            Ok(Feed::Stated(LightRecord::AT_REST.bytes()))
        );
        assert_eq!(
            at_rest(Feed::Zero(Some(ENTITY_RECORDS))),
            Ok(Feed::Stated(EntityRecord::AT_REST.bytes()))
        );
        // Noughts that are the rest, and what the editor fills itself, are left as they are.
        assert_eq!(
            at_rest(Feed::Zero(Some(0x4D5A_93EB))),
            Ok(Feed::Zero(Some(0x4D5A_93EB)))
        );
        assert_eq!(at_rest(Feed::Instance), Ok(Feed::Instance));
        assert_eq!(at_rest(Feed::Zero(None)), Ok(Feed::Zero(None)));
    }
}
