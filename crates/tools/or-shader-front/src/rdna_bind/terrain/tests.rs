use std::mem::{offset_of, size_of};

use or_io::gta5::shaders::sgd5::Sgd5Resource;

use super::super::shading::feed;
use super::super::{Class, Native, Unbound};
use super::*;
use crate::rdna::{Loc, Lowered, Program};

#[test]
fn each_default_buffer_states_its_fields_at_the_places_the_layout_gives() {
    let entry = DefaultTexture::entry;
    let (black, flat, clear) = (
        entry(DefaultTexture::Black),
        entry(DefaultTexture::Flat),
        entry(DefaultTexture::Clear),
    );
    // The places the programs read, against the declared records.
    assert_eq!(offset_of!(TileRecord, origin), 48);
    assert_eq!(offset_of!(TileRecord, sub_tiles), 64);
    assert_eq!(offset_of!(TileRecord, weights), 320);
    assert_eq!(offset_of!(TileRecord, indices), 324);
    assert_eq!(offset_of!(TileRecord, tint_a), 332);
    assert_eq!(offset_of!(TileRecord, tint_b), 336);
    assert_eq!(offset_of!(TileRecord, at_356), 356);
    assert_eq!(offset_of!(TileRecord, last_sub_tile), 360);
    assert_eq!(offset_of!(LayerRecord, own_tiling), 16);
    assert_eq!(offset_of!(LayerRecord, sharpness), 40);
    assert_eq!(offset_of!(LayerRecord, tint_a_gain), 52);
    assert_eq!(offset_of!(LayerRecord, at_80), 80);
    assert_eq!(offset_of!(LayerRecord, albedo), 112);
    assert_eq!(offset_of!(Constants, page_table), 12);
    assert_eq!(offset_of!(Constants, level_bias), 20);
    assert_eq!(offset_of!(Constants, switch), 24);

    let record = default_record(entry).words();
    assert_eq!(record.len() * 4, size_of::<TileRecord>());
    let at = |words: &[u32], offset: usize| words[offset / 4];
    let block = offset_of!(TileRecord, sub_tiles);
    for (kind, number) in [black, flat, black, flat].into_iter().enumerate() {
        for sub_tile in 0..16 {
            assert_eq!(at(&record, block + 64 * kind + 4 * sub_tile), number);
        }
    }
    assert_eq!(at(&record, offset_of!(TileRecord, weights)), clear);
    assert_eq!(at(&record, offset_of!(TileRecord, indices)), clear);
    for offset in (offset_of!(TileRecord, at_328)..=offset_of!(TileRecord, at_356)).step_by(4) {
        assert_eq!(at(&record, offset), black, "+{offset}");
    }
    let nought = (0..block).chain(offset_of!(TileRecord, last_sub_tile)..size_of::<TileRecord>());
    assert!(nought.step_by(4).all(|offset| at(&record, offset) == 0));

    let layer = default_layer(entry).words();
    assert_eq!(layer.len() * 4, size_of::<LayerRecord>());
    let float = |offset: usize| f32::from_bits(at(&layer, offset));
    assert_eq!(float(offset_of!(LayerRecord, scale)), 128.0);
    assert_eq!(float(offset_of!(LayerRecord, scale) + 4), 128.0);
    for offset in
        (offset_of!(LayerRecord, own_tiling)..=offset_of!(LayerRecord, height_scale)).step_by(4)
    {
        assert_eq!(float(offset), 1.0, "+{offset}");
    }
    assert_eq!(float(offset_of!(LayerRecord, tint_a_amount)), 0.0);
    assert_eq!(float(offset_of!(LayerRecord, tint_a_gain)), 1.0);
    assert_eq!(float(offset_of!(LayerRecord, at_72) + 4), 24.0);
    assert_eq!(float(offset_of!(LayerRecord, at_80)), 0.2);
    assert_eq!(at(&layer, offset_of!(LayerRecord, albedo)), black);
    assert_eq!(at(&layer, offset_of!(LayerRecord, normal)), flat);
    assert_eq!(at(&layer, offset_of!(LayerRecord, height)), black);

    let constants = default_constants(entry).words();
    assert_eq!(constants.len() * 4, size_of::<Constants>());
    assert!(constants[..5].iter().all(|n| *n == black));
    let bias = f32::from_bits(at(&constants, offset_of!(Constants, level_bias)));
    assert_eq!(bias, 2.0);
    assert_eq!(at(&constants, offset_of!(Constants, switch)), 0);
    // Each default sits at an entry of its own, past the entries a material's numbers pick and
    // within the 255 a stage takes a picked number to.
    assert!(black >= 128 && flat > black && clear > flat && clear < 256);
}

/// A made-up program's table naming, by `hashes`, the record, the layers, the constants and
/// the cells, each in the table of user register 9.
fn table(hashes: [u32; 4]) -> Sgd5Table {
    let resource = |code, slot, name_hash| Sgd5Resource {
        code,
        flags: 0,
        slot,
        members: 0,
        buffer_size: 0,
        name_hash,
        semantic_hash: name_hash,
    };
    Sgd5Table {
        resources: vec![
            resource(0x05, 225, hashes[0]),
            resource(0x05, 226, hashes[1]),
            resource(0x30, 19, hashes[2]),
            resource(0x05, 227, hashes[3]),
        ],
        ..Sgd5Table::default()
    }
}

fn native() -> Native {
    let record = |class, register, offset, stride| Record {
        class,
        register,
        root: 9,
        offset,
        dwords: 4,
        stride,
    };
    Native {
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
        records: vec![
            record(Class::Srv, 225, 32, 372),
            record(Class::Srv, 226, 64, 128),
            record(Class::Cbv, 19, 96, 0),
            record(Class::Srv, 227, 128, 4),
            Record {
                class: Class::RootConstants,
                register: 500,
                root: 13,
                offset: 0,
                dwords: 4,
                stride: 0,
            },
        ],
        inputs: Vec::new(),
        outputs: Vec::new(),
    }
}

/// A mesh program reaches the table of user register 9 through scalar register 17, and the heap,
/// two past the dwords of a draw in 13, through 23.
const TABLE: u8 = 17;
const HEAP: u8 = 23;

fn at(root: u8, offset: u32, index: Option<Index>) -> Option<Loc> {
    Some(Loc {
        root,
        path: Vec::new(),
        offset,
        index,
    })
}

fn bound(kind: ResourceKind, at: Option<Loc>, binding: u32, count: Option<u32>) -> Resource {
    Resource {
        kind,
        at,
        words: None,
        group: 0,
        binding,
        count,
        written: false,
    }
}

/// An entry of the heap picked by the number at `offset` of the buffer at `buffer` of the table.
fn picked(buffer: u32, offset: usize, binding: u32) -> Resource {
    let index = Index {
        source: IndexSource::Data {
            buffer: Box::new(Loc {
                root: TABLE,
                path: Vec::new(),
                offset: buffer,
                index: None,
            }),
            offset: offset as u32,
            plus: None,
        },
        shift: 5,
    };
    let texture = ResourceKind::Texture {
        dim: 1,
        compared: false,
    };
    bound(texture, at(HEAP, 0, Some(index)), binding, None)
}

/// The buffers, three picked entries, the heap bound whole at binding 6 and, with `cells`, the
/// cells at binding 7.
fn resources(picks: bool, cells: bool) -> Vec<Resource> {
    let buffer = ResourceKind::Buffer { stride: None };
    let mut out = vec![
        bound(buffer, at(TABLE, 32, None), 0, None),
        bound(buffer, at(TABLE, 64, None), 1, None),
        bound(
            ResourceKind::ConstantBuffer { rows: 2 },
            at(TABLE, 96, None),
            2,
            None,
        ),
    ];
    if picks {
        out.push(picked(32, offset_of!(TileRecord, weights), 3));
        out.push(picked(32, offset_of!(TileRecord, sub_tiles) + 64, 4));
        out.push(picked(96, offset_of!(Constants, atlases), 5));
    }
    let runtime = Index {
        source: IndexSource::Runtime { site: 0 },
        shift: 0,
    };
    let texture = ResourceKind::Texture {
        dim: 1,
        compared: false,
    };
    out.push(bound(texture, at(HEAP, 0, Some(runtime)), 6, Some(256)));
    if cells {
        out.push(bound(buffer, at(TABLE, 128, None), 7, None));
    }
    out
}

/// A stage that loads a texel of the heap's entry a number names.
fn lowered(resources: Vec<Resource>) -> Lowered {
    let text = "@group(0) @binding(6) var heap: binding_array<texture_2d<f32>, 256>;
@group(0) @binding(30) var<uniform> number: u32;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return textureLoad(heap[number], vec2<i32>(0), 0);
}";
    Lowered {
        module: naga::front::wgsl::parse_str(text).expect("the WGSL parses"),
        resources,
        approximations: Default::default(),
        half_slots: Default::default(),
        subgroups: false,
        intersections: Vec::new(),
        region: None,
    }
}

const TERRAIN: [u32; 4] = [RECORD, LAYERS, CONSTANTS, CELLS];

#[test]
fn a_terrain_stage_is_fed_the_default_buffers_and_the_textures_their_numbers_name() {
    let stage = feed(
        &native(),
        lowered(resources(true, false)),
        &table(TERRAIN),
        &Sgd5Table::default(),
    )
    .expect("the stage is fed");
    let fed: Vec<_> = (stage.feeds.iter())
        .map(|fed| (fed.binding, fed.feed.clone(), fed.header))
        .collect();
    let texture = |t| Feed::Terrain(Terrain::Texture(t));
    let sub_tile = |k| Feed::Terrain(Terrain::SubTile(k));
    let slot = |at: usize| Feed::Terrain(Terrain::Slot(at as u32));
    assert_eq!(
        fed,
        [
            (0, Feed::Terrain(Terrain::Record), Some(372)),
            (1, Feed::Terrain(Terrain::Layers), Some(128)),
            (2, Feed::Terrain(Terrain::Constants), None),
            // A record's slots are bound by the place their number is read at.
            (3, slot(offset_of!(TileRecord, weights)), None),
            (4, slot(offset_of!(TileRecord, sub_tiles) + 64), None),
            (5, texture(DefaultTexture::Black), None),
            // The heap: its blank entry, the defaults a layer's numbers pick, then a tile's
            // sub-tile textures, which the record's blocks pick.
            (7, Feed::Blank, None),
            (8, texture(DefaultTexture::Black), None),
            (9, texture(DefaultTexture::Flat), None),
            (10, texture(DefaultTexture::Clear), None),
            (11, sub_tile(0), None),
            (12, sub_tile(1), None),
            (13, sub_tile(2), None),
            (14, sub_tile(3), None),
            // The drawn cell's layer textures, an array a kind.
            (15, Feed::Terrain(Terrain::Layer(0)), None),
            (16, Feed::Terrain(Terrain::Layer(1)), None),
            (17, Feed::Terrain(Terrain::Layer(2)), None),
        ]
    );
    let written = super::super::shading::wgsl(&stage.module).expect("the stage is valid");
    let entries = (DefaultTexture::ALL.map(DefaultTexture::entry).into_iter())
        .chain((0..SUB_TILE_KINDS).map(sub_tile_entry));
    for entry in entries {
        // A stage takes a picked number to 255 at most.
        assert!(entry < 256, "entry {entry} is past a pick's reach");
        let entry = format!("== {entry}u");
        assert!(written.contains(&entry), "{entry}: {written}");
    }
    let last = layer_entry(LAYER_KINDS - 1, LAYER_SLICES - 1);
    assert!(last < super::super::trees::TILE_BLACK_ENTRY, "{last}");
    let range = format!(">= {}u", layer_entry(1, 0));
    assert!(written.contains(&range), "{range}: {written}");
    // Where no tile is given, a slot shows what the default record states there.
    assert_eq!(
        default_at(offset_of!(TileRecord, weights) as u32),
        Some(DefaultTexture::Clear)
    );
    assert_eq!(
        default_at(offset_of!(TileRecord, sub_tiles) as u32 + 64),
        Some(DefaultTexture::Flat)
    );
}

#[test]
fn a_terrain_stage_that_reads_the_tile_s_cells_is_fed_them_at_their_place() {
    let stage = feed(
        &native(),
        lowered(resources(true, true)),
        &table(TERRAIN),
        &Sgd5Table::default(),
    )
    .expect("the stage is fed");
    let cells = (stage.feeds.iter()).find(|fed| fed.binding == 7);
    let cells = cells.map(|fed| (fed.feed.clone(), fed.header));
    assert_eq!(cells, Some((Feed::Terrain(Terrain::Cells), Some(4))));
    // The heap's entries move past the cells' binding.
    assert!((stage.feeds.iter()).any(|fed| fed.binding == 8 && fed.feed == Feed::Blank));
}

#[test]
fn a_tile_s_own_record_states_its_corner_its_sub_tiles_and_the_numbers_of_its_slots() {
    let corner = [-9728.0, 1536.0, -299.75];
    // Slots 3 (weights), 7 (second tint) and kind 1 (slot 1) are held; the drawn sub-tile is 5.
    let held = |slot: usize| matches!(slot, 1 | 3 | 7);
    let record = tile_record(corner, true, 5, held).words();
    let at = |offset: usize| record[offset / 4];
    let float = |offset: usize| f32::from_bits(at(offset));
    assert_eq!(
        [0, 20, 40].map(float),
        [1.0; 3],
        "three rows of an identity"
    );
    let origin = offset_of!(TileRecord, origin);
    assert_eq!([0, 4, 8].map(|o| float(origin + o)), corner);
    // The fourth dword packs the whole 32 m steps toward nought: -304 and 48.
    assert_eq!(at(origin + 12), (-304i16 as u16 as u32) | 48 << 16);
    let block = offset_of!(TileRecord, sub_tiles);
    let clear = DefaultTexture::Clear.entry();
    assert_eq!(
        at(block + 64 + 4 * 5),
        sub_tile_entry(1),
        "the held kind at the drawn sub-tile"
    );
    assert_eq!(
        at(block + 64 + 4 * 4),
        clear,
        "another sub-tile is not this view's"
    );
    assert_eq!(at(block + 4 * 5), clear, "a kind not held");
    assert_eq!(at(offset_of!(TileRecord, last_sub_tile)), 15);
    let held_number = at(offset_of!(TileRecord, weights));
    assert!(held_number > sub_tile_entry(SUB_TILE_KINDS - 1));
    assert_ne!(
        at(offset_of!(TileRecord, tint_b)),
        held_number,
        "each held slot its own"
    );
    assert_eq!(at(offset_of!(TileRecord, tint_a)), clear);
    assert_eq!(
        at(offset_of!(TileRecord, at_340)),
        DefaultTexture::Flat.entry()
    );

    // With no sub-tiles the first entry of each block is the slot's, and an unheld slot 11 is nought.
    let record = tile_record(corner, false, 5, |slot| slot == 0).words();
    let at = |offset: usize| record[offset / 4];
    assert_eq!(at(block), sub_tile_entry(0));
    assert_eq!(at(block + 64), clear);
    assert_eq!(at(block + 192), 0);
    assert_eq!(at(offset_of!(TileRecord, last_sub_tile)), 0);
    for (slot, offset) in SLOT_OFFSETS.iter().enumerate() {
        assert_eq!(slot_at(*offset), Some(slot));
    }
}

#[test]
fn a_tile_s_layer_takes_its_floats_in_record_order_and_the_default_textures() {
    let params: [f32; 28] = std::array::from_fn(|i| i as f32 + 0.5);
    let layer = tile_layer(params, unheld_layer()).words();
    for (i, word) in layer[..28].iter().enumerate() {
        assert_eq!(*word, params[i].to_bits(), "dword {i}");
    }
    assert_eq!(
        layer[offset_of!(LayerRecord, albedo) / 4],
        DefaultTexture::Black.entry()
    );
    assert_eq!(
        layer[offset_of!(LayerRecord, normal) / 4],
        DefaultTexture::Flat.entry()
    );
    assert_eq!(
        layer[offset_of!(LayerRecord, height) / 4],
        DefaultTexture::Black.entry()
    );
}

#[test]
fn a_stage_of_other_buffers_is_fed_as_before() {
    const OTHER: [u32; 4] = [0xA000_0001, 0xA000_0002, 0xA000_0003, 0xA000_0004];
    let stage = feed(
        &native(),
        lowered(resources(false, true)),
        &table(OTHER),
        &Sgd5Table::default(),
    )
    .expect("the stage is fed");
    let fed: Vec<_> = (stage.feeds.iter())
        .map(|fed| (fed.binding, fed.feed.clone(), fed.header))
        .collect();
    assert_eq!(
        fed,
        [
            (0, Feed::Zero(Some(OTHER[0])), Some(372)),
            (1, Feed::Zero(Some(OTHER[1])), Some(128)),
            (2, Feed::Zero(Some(OTHER[2])), None),
            (8, Feed::Blank, None),
            (7, Feed::Zero(Some(OTHER[3])), Some(4)),
        ]
    );
    // A number of a buffer that is none of the terrain's still picks nothing the editor holds.
    let picks = feed(
        &native(),
        lowered(resources(true, false)),
        &table(OTHER),
        &Sgd5Table::default(),
    );
    assert_eq!(picks.map(|_| ()), Err(Unfed::Unbound(Unbound::Picked)));
}

#[test]
fn the_feedback_texture_s_writes_are_taken_out_and_a_read_of_it_is_refused() {
    let text = "@group(1) @binding(29) var feedback: texture_storage_2d<rgba16float, write>;
@fragment fn main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    if at.x > 2.0 {
        textureStore(feedback, vec2<i32>(at.xy), vec4<f32>(1.0));
    }
    return vec4<f32>(0.5);
}";
    let mut module = naga::front::wgsl::parse_str(text).unwrap();
    assert_eq!(unwritten(&mut module, (1, 29)), Ok(()));
    let written = super::super::shading::wgsl(&module).expect("the stage is valid");
    assert!(!written.contains("textureStore"), "{written}");
    assert!(written.contains("texture_2d<f32>"), "{written}");

    let read = text.replace("write>", "read_write>").replace(
        "return vec4<f32>(0.5);",
        "return textureLoad(feedback, vec2<i32>(0));",
    );
    let mut module = naga::front::wgsl::parse_str(&read).unwrap();
    assert_eq!(unwritten(&mut module, (1, 29)), Err(Unfed::TerrainFeedback));
    // Through the feeds: only a binding fed as the feedback is touched.
    let feeds = [Fed {
        group: 1,
        binding: 29,
        feed: Feed::Terrain(Terrain::Feedback),
        header: None,
    }];
    let mut module = naga::front::wgsl::parse_str(text).unwrap();
    assert_eq!(held(&mut module, &feeds), Ok(()));
    let written = super::super::shading::wgsl(&module).unwrap();
    assert!(!written.contains("textureStore"), "{written}");
}
