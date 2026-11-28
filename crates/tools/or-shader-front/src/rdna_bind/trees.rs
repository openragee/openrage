//! What the game leaves, for a model drawn alone, in the buffers a tree's programs find the lights
//! that reach the object by, and in the frame's tree data.

use super::shading::Feed;

/// The draw's state, 68 bytes: dword 5 is the number of the object's record among the frame's
/// object records, set from the object before it is drawn.
pub const DRAW_STATE: u32 = 0x3D71_7C13;
/// The frame's object records, 48 bytes each: the low sixteen bits of dword 9 are the number of
/// the object's list of lights.
pub const OBJECT_RECORDS: u32 = 0xE9DA_8713;
/// The frame's lists of lights, 76 bytes each: dword 0 is how many lights the list holds, then
/// the number of each light's 84-byte record, at most seventeen, then a dword of flags.
pub const LIGHT_LISTS: u32 = 0x9EA6_D244;
/// `gGlobalTreeData`, 44 bytes, which the game writes once a frame.
pub const TREE_DATA: u32 = 0x9FC1_91E6;

/// The buffers only the game fills whose noughts are their state for a model drawn alone: its
/// one object is record nought, whose list is list nought, which holds no light. So a loop that
/// counts a list's lights counts to nought.
// ponytail: stated per buffer, not per field: a loop that counted to another field of the draw's
// state would be let through as well. Only dword 5 of it is established; split it by field when
// such a loop is met.
pub const AT_REST: &[u32] = &[DRAW_STATE, OBJECT_RECORDS, LIGHT_LISTS];

/// The words of the frame's tree data as the game writes them, by dword.
// ⚠ Dword 2 is a bias the game computes from the camera, nought here; no tree program reads it.
// Dword 10 holds 7: its bit 0 makes a tree's pixel program walk the object's list of lights.
pub const TREE_WORDS: [u32; 11] = [
    0,
    0x3C8B_4396,
    0,
    0x4040_0000,
    0x3DB8_51EC,
    0x3EB3_3333,
    0xBE4C_CCCD,
    0x3F4C_CCCD,
    0x3E99_999A,
    2,
    7,
];

/// Where the view's lighting record holds the four words a tree's pixel program fades its
/// coverage by: three switches the game's settings leave on (1.0 each), then the coverage's
/// scale, two to the power of a setting no settings file states (nought): 1.0.
// ⚠ With the four at nought a tree's dithered alpha test discards every pixel of it.
pub const VIEW_COVERAGE: usize = 0x260;
pub const VIEW_COVERAGE_WORDS: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// Where the view's lighting record holds how many probe levels there are: a count of nought
/// wraps where a loop counts it less one, so it states the one level a lit stage is given.
pub const VIEW_LEVELS: usize = 0x72C;
/// The bytes of the view's lighting record.
const VIEW_BYTES: usize = 2044;

/// The map of the world tiles around the camera, 24 bytes: the place of the grid's first corner
/// (two floats), then the heap numbers of the maps of its two by two cells of 512 metres.
pub const WORLD_TILES: u32 = 0x922C_726E;
/// The heap entry the editor puts the game's black texel at where a stage binds the tiles: the
/// last a stage's pick reaches, which a number of a material's would take only as its 255th.
pub const TILE_BLACK_ENTRY: u32 = 255;

/// The tiles' record as the game writes it with no tile streamed in: the grid at the origin and
/// each cell the black texel.
pub fn world_tiles_at_rest() -> Vec<u8> {
    let mut words = vec![0u32; 2];
    words.extend([TILE_BLACK_ENTRY; 4]);
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The heap entries a stage that binds the tiles takes beside its own: the black texel, unless
/// a number of the material's, `numbers`, takes that entry.
pub(super) fn tile_defaults(tiles: bool, numbers: &[u32]) -> Vec<(u32, Feed)> {
    use super::terrain::{DefaultTexture, Terrain};
    match tiles && !numbers.contains(&(TILE_BLACK_ENTRY - 1)) {
        true => vec![(
            TILE_BLACK_ENTRY,
            Feed::Terrain(Terrain::Texture(DefaultTexture::Black)),
        )],
        false => Vec::new(),
    }
}

/// The frame record, 2,048 bytes.
pub const FRAME_RECORD: u32 = 0x6E87_25E4;
/// Where the frame record holds the near fade of foliage: the depth it starts at and one over
/// the depth it takes, so that a pixel's coverage is `saturate(scale * (depth - start))`.
pub const FRAME_NEAR_FADE: usize = 0x5E8;
/// The game's for a camera that does not move: the fade starts at 0.15 and ends at 0.2 metres.
// The executable's constants 0.15 and 0.2, blended towards wider ones only while the camera moves.
pub const FRAME_NEAR_FADE_WORDS: [f32; 2] = [0.15, 1.0 / (0.2 - 0.15)];
/// Where the frame record holds a range in linear light, its start then its end: a ped's deferred
/// pixel program takes `saturate((x - start) / (end - start))` of the largest of three channels.
pub const FRAME_RAMP: usize = 0x6C0;
/// The game's two settings at rest for that range, as bytes of sRGB the frame decodes.
// ⚠ Stated as noughts, one over the range is infinite and the ramp steps on noise: a speckle.
pub const FRAME_RAMP_BYTES: [u8; 2] = [5, 20];
const FRAME_BYTES: usize = 2048;

/// A byte of sRGB in linear light, by the frame's arithmetic.
fn linear_of(byte: u8) -> f32 {
    let x = f32::from(byte) / 255.0;
    match x {
        x if x <= 0.04045 => x / 12.92,
        x => ((x + 0.055) / 1.055).powf(2.4),
    }
}

/// The words of the frame record the game is known to leave, by byte: the near fade and the ramp.
pub(super) fn frame_words() -> Vec<(u32, u32)> {
    let ramp = FRAME_RAMP_BYTES.map(linear_of);
    let at = |start: usize, words: [f32; 2]| {
        (words.into_iter().zip(0..)).map(move |(word, k)| ((start + 4 * k) as u32, word.to_bits()))
    };
    at(FRAME_NEAR_FADE, FRAME_NEAR_FADE_WORDS)
        .chain(at(FRAME_RAMP, ramp))
        .collect()
}

/// The frame record a deferred pass is given: [`frame_words`], noughts elsewhere.
// ⚠ With the fade at nought every pixel of a tree's card has no coverage.
pub(super) fn frame_at_rest() -> Feed {
    let mut bytes = vec![0u8; FRAME_BYTES];
    for (at, word) in frame_words() {
        let at = at as usize;
        bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    Feed::Stated(bytes)
}

/// The view's lighting record a deferred pass is given: the game's coverage words, one probe
/// level, noughts elsewhere.
pub(super) fn view_at_rest() -> Feed {
    let mut bytes = vec![0u8; VIEW_BYTES];
    for (k, word) in VIEW_COVERAGE_WORDS.iter().enumerate() {
        let at = VIEW_COVERAGE + 4 * k;
        bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes[VIEW_LEVELS..VIEW_LEVELS + 4].copy_from_slice(&1u32.to_le_bytes());
    Feed::Stated(bytes)
}

/// What the game leaves in the binding named `hash`, where it is a buffer of a tree's and its
/// noughts are not that.
pub(super) fn stated(hash: u32) -> Option<Feed> {
    match hash {
        TREE_DATA => Some(Feed::Stated(
            TREE_WORDS.iter().flat_map(|w| w.to_le_bytes()).collect(),
        )),
        WORLD_TILES => Some(Feed::Stated(world_tiles_at_rest())),
        _ => (super::batches::stated(hash))
            .or_else(|| super::ped_layers::stated(hash))
            .or_else(|| super::wind::stated(hash)),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use or_io::gta5::shaders::sgd5::{Sgd5Resource, Sgd5Table};

    use super::super::shading::{feed, Unfed};
    use super::super::{Class, Native, Record};
    use super::*;
    use crate::dxil::ShaderKind;
    use crate::rdna::{Loc, Lowered, Program, Resource, ResourceKind};

    /// A buffer of the game's another stage might count to.
    const OTHER: u32 = 0xA000_0006;

    /// A mesh stage that binds the buffer `hash` of `stride`-byte elements at register 16 as
    /// `kind`, its lowered module the WGSL `text`, which binds it at group 0, binding 0.
    pub(in crate::rdna_bind) fn stage_reading(
        hash: u32,
        stride: u32,
        kind: ResourceKind,
        text: &str,
    ) -> (Native, Lowered, Sgd5Table) {
        let record = Record {
            class: Class::Srv,
            register: 16,
            root: 9,
            offset: 512,
            dwords: 4,
            stride,
        };
        let native = Native {
            program: Program {
                kind: ShaderKind::Mesh,
                slot: 8,
                words: Vec::new(),
                wave32: false,
                inputs: Vec::new(),
                vertex_inputs: Vec::new(),
                position_read: 0,
                front_face: false,
                launch: None,
            },
            records: vec![record],
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        // A mesh program reaches the table of user register 9 through scalar register 17.
        let resource = Resource {
            kind,
            at: Some(Loc {
                root: 17,
                path: Vec::new(),
                offset: record.offset,
                index: None,
            }),
            words: None,
            group: 0,
            binding: 0,
            count: None,
            written: false,
        };
        let lowered = Lowered {
            module: naga::front::wgsl::parse_str(text).expect("the WGSL parses"),
            resources: vec![resource],
            approximations: Default::default(),
            half_slots: Default::default(),
            subgroups: false,
            intersections: Vec::new(),
            region: None,
        };
        let table = Sgd5Table {
            resources: vec![Sgd5Resource {
                code: 0x05,
                flags: 0,
                slot: 16,
                members: 0,
                buffer_size: stride,
                name_hash: hash,
                semantic_hash: hash,
            }],
            ..Sgd5Table::default()
        };
        (native, lowered, table)
    }

    /// A stage that walks as many entries as dword 5 of the buffer at binding 0 states.
    const COUNTING: &str = "
@group(0) @binding(0) var<uniform> state: array<vec4<u32>, 2>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    var i: u32 = 0u;
    loop {
        if i >= state[1].y { break; }
        i = i + 1u;
    }
    return vec4<f32>(f32(i));
}";

    #[test]
    fn a_loop_counted_to_a_number_the_feed_states_at_rest_is_fed_and_one_of_another_buffer_is_not()
    {
        let rows = ResourceKind::ConstantBuffer { rows: 2 };
        let fed = |hash: u32| {
            let (native, lowered, table) = stage_reading(hash, 68, rows, COUNTING);
            feed(&native, lowered, &table, &Sgd5Table::default()).map(|stage| stage.feeds)
        };
        for hash in AT_REST {
            let feeds = fed(*hash).unwrap_or_else(|why| panic!("0x{hash:08X}: {why}"));
            assert_eq!(feeds[0].feed, Feed::Zero(Some(*hash)), "fed as its noughts");
        }
        // The same loop counted to a number of a buffer whose rest is not known stays refused.
        assert_eq!(fed(OTHER).map(|_| ()), Err(Unfed::Loop(OTHER)));
    }

    #[test]
    fn the_tree_data_is_fed_as_the_game_writes_it() {
        let (native, lowered, table) = stage_reading(
            TREE_DATA,
            44,
            ResourceKind::ConstantBuffer { rows: 3 },
            "
@group(0) @binding(0) var<uniform> trees: array<vec4<u32>, 3>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(trees[2].z & 1u));
}",
        );
        let stage = feed(&native, lowered, &table, &Sgd5Table::default()).expect("fed");
        let Feed::Stated(bytes) = &stage.feeds[0].feed else {
            panic!("the tree data is not stated: {:?}", stage.feeds[0]);
        };
        assert_eq!(bytes.len(), 44);
        let word = |at: usize| u32::from_le_bytes(bytes[4 * at..4 * at + 4].try_into().unwrap());
        assert_eq!(f32::from_bits(word(3)), 3.0);
        assert_eq!(word(10) & 1, 1, "the list of lights is walked");
        assert_eq!(word(9), 2);
    }

    #[test]
    fn the_world_tiles_are_fed_as_no_tile_streamed_in_and_their_black_entry_is_kept_free() {
        let (native, lowered, table) = stage_reading(
            WORLD_TILES,
            24,
            ResourceKind::Buffer { stride: None },
            "
@group(0) @binding(0) var<storage> tiles: array<u32>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(tiles[6u]));
}",
        );
        let stage = feed(&native, lowered, &table, &Sgd5Table::default()).expect("fed");
        assert_eq!(stage.feeds[0].header, Some(24));
        let Feed::Stated(bytes) = &stage.feeds[0].feed else {
            panic!("the tiles are not stated: {:?}", stage.feeds[0]);
        };
        let word = |at: usize| u32::from_le_bytes(bytes[4 * at..4 * at + 4].try_into().unwrap());
        assert_eq!((word(0), word(1)), (0, 0), "the grid at the origin");
        assert!(
            (2..6).all(|k| word(k) == TILE_BLACK_ENTRY),
            "each cell the black texel"
        );
        assert_eq!(tile_defaults(true, &[0, 3]).len(), 1);
        assert!(tile_defaults(true, &[TILE_BLACK_ENTRY - 1]).is_empty());
        assert!(tile_defaults(false, &[]).is_empty());
    }

    #[test]
    fn a_deferred_stage_is_given_the_view_s_coverage_words_and_one_probe_level() {
        let (native, lowered, table) = stage_reading(
            super::super::forward_frame::VIEW_LIGHTING,
            2044,
            ResourceKind::ConstantBuffer { rows: 128 },
            "
@group(0) @binding(0) var<uniform> view: array<vec4<u32>, 128>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(bitcast<f32>(view[38].w));
}",
        );
        let stage = feed(&native, lowered, &table, &Sgd5Table::default()).expect("fed");
        let Feed::Stated(bytes) = &stage.feeds[0].feed else {
            panic!("the view's record is not stated: {:?}", stage.feeds[0]);
        };
        assert_eq!(bytes.len(), 2044);
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        // The tree's alpha test reads row 38: its fourth dword scales the coverage.
        for k in 0..4 {
            assert_eq!(
                f32::from_bits(word(38 * 16 + 4 * k)),
                1.0,
                "coverage word {k}"
            );
        }
        assert_eq!(word(VIEW_LEVELS), 1);
        // The frame record's near fade, read at row 94's third and fourth dwords.
        let (native, lowered, table) = stage_reading(
            FRAME_RECORD,
            2048,
            ResourceKind::ConstantBuffer { rows: 128 },
            "
@group(0) @binding(0) var<uniform> frame: array<vec4<u32>, 128>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(bitcast<f32>(frame[94].z), bitcast<f32>(frame[94].w), 0.0, 1.0);
}",
        );
        let stage = feed(&native, lowered, &table, &Sgd5Table::default()).expect("fed");
        let Feed::Stated(frame) = &stage.feeds[0].feed else {
            panic!("the frame record is not stated: {:?}", stage.feeds[0]);
        };
        let float = |at: usize| f32::from_le_bytes(frame[at..at + 4].try_into().unwrap());
        assert_eq!(float(94 * 16 + 8), 0.15);
        assert!(
            (float(94 * 16 + 12) - 20.0).abs() < 1e-3,
            "{}",
            float(94 * 16 + 12)
        );
        // The ramp at row 108: 5 and 20 of 255 in linear light, so one over its width is finite.
        let (start, end) = (float(108 * 16), float(108 * 16 + 4));
        assert!((start - 5.0 / 255.0 / 12.92).abs() < 1e-7, "{start}");
        assert!(
            (end - ((20.0 / 255.0 + 0.055) / 1.055f32).powf(2.4)).abs() < 1e-7,
            "{end}"
        );
        assert!((1.0 / (end - start)).is_finite() && end > start);
        // Its count is not stated: a loop to a number of it stays refused.
        let (native, lowered, table) = stage_reading(
            FRAME_RECORD,
            2048,
            ResourceKind::ConstantBuffer { rows: 2 },
            COUNTING,
        );
        let fed = feed(&native, lowered, &table, &Sgd5Table::default());
        assert_eq!(fed.map(|_| ()), Err(Unfed::Loop(FRAME_RECORD)));
        let others = (0..2044)
            .step_by(4)
            .filter(|at| !(0x260..0x270).contains(at));
        assert_eq!(
            others.filter(|at| word(*at) != 0).count(),
            1,
            "the level count alone"
        );
    }
}
