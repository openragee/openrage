use std::collections::HashMap;

use super::*;
use crate::meta::binary::rbm::schema::{PsoType, RbmClass, RbmClassEntry};

fn class(name: u32, size: u32, members: &[Member]) -> RbmClass {
    RbmClass {
        name_hash: name,
        signature: 0,
        flags: 0,
        align_power: 4,
        data_size: size,
        major_version: 0,
        entries: members
            .iter()
            .map(|&(hash, offset, code)| {
                RbmClassEntry::synthesised(hash, offset, PsoType::from_byte(code), 0, 0)
            })
            .collect(),
    }
}

/// A pointer inside data: the 1-based block in bits 0 to 11, the offset from bit 20.
fn ptr(block: u64, offset: u64) -> u64 {
    block | offset << 20
}

fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
    buf[at..at + bytes.len()].copy_from_slice(bytes);
}

fn floats(buf: &mut [u8], at: usize, values: &[f32]) {
    for (i, v) in values.iter().enumerate() {
        put(buf, at + 4 * i, &v.to_le_bytes());
    }
}

fn array(buf: &mut [u8], at: usize, pointer: u64, count: u16) {
    put(buf, at, &pointer.to_le_bytes());
    put(buf, at + 8, &count.to_le_bytes());
    put(buf, at + 10, &count.to_le_bytes());
}

fn block(data_type: u32, data: Vec<u8>) -> RbmDataBlock {
    RbmDataBlock {
        data_type,
        data,
        data_addr: 0x5000_0000,
    }
}

fn model(root_block_id: u32, blocks: Vec<RbmDataBlock>) -> RbmModel {
    let classes: HashMap<u32, RbmClass> = LAYOUTS
        .iter()
        .map(|(hash, _, members, size)| (*hash, class(*hash, size.unwrap_or(0x10), members)))
        .collect();
    RbmModel {
        class_order: classes.keys().copied().collect(),
        classes,
        enums: HashMap::new(),
        data_blocks: blocks,
        root_block_id,
        enum_order: Vec::new(),
    }
}

/// Entity block 3 is a `CEntityDef` at `lodLevel` 1, block 4 an interior at 5; one prop list of 3
/// instances and one grass list of 5.
fn map_model() -> RbmModel {
    let mut root = vec![0u8; MAP_DATA_SIZE];
    put(&mut root, 0x08, &0x1111_0001u32.to_le_bytes());
    put(&mut root, 0x10, &0x2222_0000_0000_0002u64.to_le_bytes());
    for (at, value) in [
        (0x50, 0x50u32),
        (0x68, 0x68),
        (0x6C, 0x6C),
        (0x70, 0x70),
        (0x74, 0x74),
    ] {
        put(&mut root, at, &(0x5000_0000 | value).to_le_bytes());
    }
    put(&mut root, 0x80, &0x81u32.to_le_bytes());
    put(&mut root, 0x84, &0x401u32.to_le_bytes());
    floats(
        &mut root,
        0x90,
        &[-100.0, -200.0, -30.0, 0.0, 100.0, 200.0, 30.0],
    );
    floats(&mut root, 0xB0, &[-10.0, -20.0, -3.0, 0.0, 10.0, 20.0, 3.0]);
    array(&mut root, 0xD0, ptr(2, 0), 2);
    array(&mut root, 0x148, ptr(5, 0), 1);
    array(&mut root, 0x158, ptr(7, 0), 1);

    let mut pointers = vec![0u8; 16];
    put(&mut pointers, 0, &ptr(3, 0).to_le_bytes());
    put(&mut pointers, 8, &ptr(4, 0x10).to_le_bytes());

    let entity = |at: usize,
                  rotation: [f32; 4],
                  seed: u32,
                  level: u8,
                  parent: i16,
                  children: u32,
                  lod: f32| {
        let mut e = vec![0u8; at + ENTITY_SIZE as usize];
        let r = &mut e[at..];
        put(r, 0x08, &(u64::from(seed) << 32 | 0xA1).to_le_bytes());
        floats(r, 0x10, &rotation);
        put(r, 0x20, &(u64::from(seed) << 32 | 0xB1).to_le_bytes());
        floats(r, 0x38, &[seed as f32, 2.0 * seed as f32, -3.0]);
        put(r, 0x44, &(seed | 0xC1).to_le_bytes());
        put(r, 0x48, &0x18_0000u32.to_le_bytes());
        floats(r, 0x54, &[1.5, 2.5]);
        put(r, 0x5C, &parent.to_le_bytes());
        put(r, 0x5E, &7i16.to_le_bytes());
        floats(r, 0x60, &[lod, 30.0]);
        put(r, 0x68, &children.to_le_bytes());
        put(r, 0x70, &3u32.to_le_bytes());
        r[0x7E] = level;
        e
    };

    let mut props = vec![0u8; PROP_LIST_SIZE as usize];
    put(&mut props, 0x44, &0xD1u32.to_le_bytes());
    array(&mut props, 0x90, ptr(6, 0), 3);
    floats(&mut props, 0xA0, &[-1.0, -2.0, -3.0, 0.0, 4.0, 5.0, 6.0]);
    let mut grass = vec![0u8; GRASS_LIST_SIZE as usize];
    put(&mut grass, 0x44, &0xE1u32.to_le_bytes());
    floats(&mut grass, 0x80, &[7.0, 8.0, 9.0, 0.0, 17.0, 18.0, 19.0]);
    array(&mut grass, 0x178, ptr(8, 0), 5);

    model(
        1,
        vec![
            block(C_MAP_DATA, root),
            block(7, pointers),
            block(
                C_ENTITY_DEF,
                entity(0, [0.0, 0.0, 0.6, 0.8], 0x100, 1, 4, 2, 120.0),
            ),
            block(
                C_MLO_INSTANCE_DEF,
                entity(0x10, [0.6, 0.0, 0.0, 0.8], 0x200, 5, -1, 0, -1.0),
            ),
            block(PROP_LIST, props),
            block(PROP_INSTANCE, vec![0; 3 * 0x40]),
            block(GRASS_LIST, grass),
            block(GRASS_INSTANCE, vec![0; 5 * 0x10]),
        ],
    )
}

fn map_data(model: &RbmModel) -> Result<MapData> {
    match read(model)? {
        MapBody::Data(map) => Ok(map),
        other => panic!("a map, not {other:?}"),
    }
}

#[test]
fn a_map_reads_its_own_fields_its_entities_and_its_lists_counted() {
    let map = map_data(&map_model()).expect("it reads");
    let placed = |interior, seed: u32, rotation, level, parent, children, lod_dist| Entity {
        interior,
        id: u64::from(seed) << 32 | 0xA1,
        rotation,
        archetype_word: u64::from(seed) << 32 | 0xB1,
        position: [seed as f32, 2.0 * seed as f32, -3.0],
        archetype_name: seed | 0xC1,
        flags: 0x18_0000,
        scale_xy: 1.5,
        scale_z: 2.5,
        parent_index: parent,
        hash_e74c3c23: 7,
        lod_dist,
        child_lod_dist: 30.0,
        num_children: children,
        tint_value: 3,
        priority_level: 0,
        lod_level: level,
    };
    let expected = MapData {
        name: 0x1111_0001,
        name_word: 0x2222_0000_0000_0002,
        parent: 0x5000_0068,
        hash_6c0f9c8c: 0x5000_0050,
        hash_9e8d32a0: 0x5000_006C,
        hash_ec16451d: 0x5000_0070,
        hash_36c898aa: 0x5000_0074,
        flags: 0x81,
        content_flags: 0x401,
        streaming_extents: Aabb {
            min: [-100.0, -200.0, -30.0],
            max: [100.0, 200.0, 30.0],
        },
        entities_extents: Aabb {
            min: [-10.0, -20.0, -3.0],
            max: [10.0, 20.0, 3.0],
        },
        entities: vec![
            placed(false, 0x100, [0.0, 0.0, 0.6, 0.8], 1, 4, 2, 120.0),
            placed(true, 0x200, [0.6, 0.0, 0.0, 0.8], 5, -1, 0, -1.0),
        ],
        prop_lists: vec![InstanceList {
            archetype_name: 0xD1,
            instances: 3,
            batch_aabb: Aabb {
                min: [-1.0, -2.0, -3.0],
                max: [4.0, 5.0, 6.0],
            },
        }],
        grass_lists: vec![InstanceList {
            archetype_name: 0xE1,
            instances: 5,
            batch_aabb: Aabb {
                min: [7.0, 8.0, 9.0],
                max: [17.0, 18.0, 19.0],
            },
        }],
    };
    assert_eq!(map, expected);
    assert_eq!(map.entities[0].lod_level_name(), "LODTYPES_DEPTH_LOD");
    assert_eq!(map.entities[1].lod_level_name(), "LODTYPES_DEPTH_ORPHANHD");
}

#[test]
fn an_entity_turns_its_model_by_the_conjugate_and_an_interior_by_its_own_quaternion() {
    let map = map_data(&map_model()).expect("it reads");
    assert_eq!(map.entities[0].model_rotation(), [-0.0, -0.0, -0.6, 0.8]);
    assert_eq!(map.entities[1].model_rotation(), [0.6, 0.0, 0.0, 0.8]);
}

fn types_model() -> RbmModel {
    let mut root = vec![0u8; MAP_TYPES_SIZE];
    array(&mut root, 0x18, ptr(2, 0), 2);
    put(&mut root, 0x28, &0x7777_0001u32.to_le_bytes());
    let mut pointers = vec![0u8; 16];
    put(&mut pointers, 0, &ptr(3, 0).to_le_bytes());
    put(&mut pointers, 8, &ptr(4, 0).to_le_bytes());
    let archetype = |seed: u64, asset_type: u32| {
        let mut a = vec![0u8; 0x220];
        put(&mut a, 0x08, &(seed << 8 | 0x08).to_le_bytes());
        put(&mut a, 0x10, &(seed << 8 | 0x10).to_le_bytes());
        if asset_type == 2 {
            put(&mut a, 0x20, &(seed << 8 | 0x08).to_le_bytes());
        }
        floats(&mut a, 0x38, &[150.0]);
        put(&mut a, 0x3C, &(seed as u32 | 0x3C).to_le_bytes());
        floats(
            &mut a,
            0x40,
            &[-1.0, -2.0, -3.0, 4.0, 5.0, 6.0, 0.5, 1.5, 2.5, 7.5],
        );
        put(&mut a, 0x68, &(seed as u32 | 0x68).to_le_bytes());
        floats(&mut a, 0x70, &[60.0]);
        for at in [0x78usize, 0x7C, 0x80, 0x88] {
            put(&mut a, at, &(seed as u32 | at as u32).to_le_bytes());
        }
        put(&mut a, 0x84, &asset_type.to_le_bytes());
        a
    };
    model(
        1,
        vec![
            block(C_MAP_TYPES, root),
            block(7, pointers),
            block(C_BASE_ARCHETYPE_DEF, archetype(0x100, 2)),
            block(C_MLO_ARCHETYPE_DEF, archetype(0x200, 0)),
        ],
    )
}

#[test]
fn a_map_types_body_reads_its_archetypes_and_the_model_word_its_asset_type_names() {
    let MapBody::Types(types) = read(&types_model()).expect("it reads") else {
        panic!("a map types body");
    };
    let archetype = |class, seed: u64, asset_type, drawable| Archetype {
        class,
        name: seed as u32 | 0x78,
        name_word: seed << 8 | 0x08,
        texture_dictionary: seed << 8 | 0x10,
        drawable_dictionary: 0,
        drawable,
        fragment: 0,
        lod_dist: 150.0,
        special_attribute: seed as u32 | 0x3C,
        bb_min: [-1.0, -2.0, -3.0],
        bb_max: [4.0, 5.0, 6.0],
        bs_centre: [0.5, 1.5, 2.5],
        bs_radius: 7.5,
        flags: seed as u32 | 0x68,
        hd_texture_dist: 60.0,
        clip_dictionary: seed as u32 | 0x7C,
        physics_dictionary: seed as u32 | 0x80,
        asset_type,
        asset_name: seed as u32 | 0x88,
    };
    let expected = MapTypes {
        name: 0x7777_0001,
        archetypes: vec![
            archetype(ArchetypeClass::Base, 0x100, 2, 0x100 << 8 | 0x08),
            archetype(ArchetypeClass::Interior, 0x200, 0, 0),
        ],
    };
    assert_eq!(types, expected);
    assert_eq!(types.archetypes[0].model_word(), 0x100 << 8 | 0x08);
    assert_eq!(types.archetypes[0].asset_type_name(), "ASSET_TYPE_DRAWABLE");
    assert_eq!(types.archetypes[1].model_word(), 0);
}

type Patch = Box<dyn Fn(&mut RbmModel)>;
type Built = fn() -> RbmModel;

fn set(block: usize, at: usize, bytes: Vec<u8>) -> Patch {
    Box::new(move |m: &mut RbmModel| put(&mut m.data_blocks[block - 1].data, at, &bytes))
}

#[test]
fn a_body_that_breaks_a_measured_rule_is_refused_with_the_rule() {
    let cases: Vec<(Built, Patch, &str)> = vec![
        (
            map_model,
            Box::new(|m: &mut RbmModel| m.data_blocks[0].data_type = 0x35AB_C248),
            "the root structure 0x35ABC248 is neither",
        ),
        (
            map_model,
            Box::new(|m: &mut RbmModel| {
                let c = m.classes.get_mut(&C_ENTITY_DEF).expect("declared");
                c.entries[3].offset = 0x30;
            }),
            "CEntityDef's member",
        ),
        (
            map_model,
            Box::new(|m: &mut RbmModel| {
                m.classes
                    .get_mut(&C_ENTITY_DEF)
                    .expect("declared")
                    .data_size = 0x80
            }),
            "CEntityDef is 0x80 bytes, where every measured body's is 0x90",
        ),
        (
            map_model,
            set(2, 0, (ptr(3, 0) | 1 << 12).to_le_bytes().to_vec()),
            "sets bits 12 to 19",
        ),
        (
            map_model,
            set(2, 0, ptr(9, 0).to_le_bytes().to_vec()),
            "names block 9 of 8",
        ),
        (
            map_model,
            set(2, 0, ptr(3, 0x900).to_le_bytes().to_vec()),
            "is past its 144-byte block",
        ),
        (
            map_model,
            set(2, 8, ptr(1, 0).to_le_bytes().to_vec()),
            "no entity class",
        ),
        (
            map_model,
            set(1, 0xDA, 1u16.to_le_bytes().to_vec()),
            "count 2 is over its capacity 1",
        ),
        (
            map_model,
            set(3, 0x1C, 2.0f32.to_le_bytes().to_vec()),
            "norm is",
        ),
        (
            map_model,
            set(3, 0x38, f32::NAN.to_le_bytes().to_vec()),
            "is not finite",
        ),
        (
            map_model,
            set(3, 0x7E, vec![7]),
            "lodLevel 7 is no LODTYPES_DEPTH value",
        ),
        (
            map_model,
            set(3, 0x7E, vec![5]),
            "an ORPHANHD entity has parentIndex 4",
        ),
        (
            map_model,
            set(3, 0x68, 0u32.to_le_bytes().to_vec()),
            "a lodLevel 1 entity has no children",
        ),
        (
            map_model,
            set(3, 0x5C, (-2i16).to_le_bytes().to_vec()),
            "below -1",
        ),
        (
            map_model,
            set(5, 0xA0, 9.0f32.to_le_bytes().to_vec()),
            "BatchAABB's min is over its max",
        ),
        (
            map_model,
            set(5, 0x98, vec![4, 0, 4, 0]),
            "do not fit a block of their type",
        ),
        (
            map_model,
            Box::new(|m: &mut RbmModel| m.data_blocks[0].data.truncate(0x400)),
            "fewer than CMapData's 0x460",
        ),
        (
            types_model,
            set(3, 0x84, 5u32.to_le_bytes().to_vec()),
            "assetType 5 is none",
        ),
        (
            types_model,
            set(3, 0x84, 1u32.to_le_bytes().to_vec()),
            "disagrees with the model words",
        ),
        (
            types_model,
            set(3, 0x20, 9u64.to_le_bytes().to_vec()),
            "differs from its name word",
        ),
        (
            types_model,
            set(3, 0x40, 9.0f32.to_le_bytes().to_vec()),
            "bbMin is over its bbMax",
        ),
        (
            types_model,
            set(3, 0x78, 0u32.to_le_bytes().to_vec()),
            "its name is 0",
        ),
        (
            types_model,
            Box::new(|m: &mut RbmModel| m.data_blocks[3].data_type = C_ENTITY_DEF),
            "no archetype class",
        ),
    ];
    for (built, patch, want) in cases {
        let mut m = built();
        patch(&mut m);
        let err = read(&m).expect_err(want).to_string();
        assert!(err.contains(want), "{want}: {err}");
    }
}

#[test]
fn every_truncated_block_is_an_error_and_never_a_panic() {
    for built in [map_model as Built, types_model] {
        let whole = built();
        for b in 0..whole.data_blocks.len() {
            for len in 0..whole.data_blocks[b].data.len() {
                let mut m = whole.clone();
                m.data_blocks[b].data.truncate(len);
                let _ = read(&m);
            }
        }
    }
}

#[test]
fn a_map_s_name_word_is_read_from_its_root_alone_and_a_map_types_body_states_none() {
    let model = map_model();
    let word = map_data(&model).expect("the map reads").name_word;
    assert_ne!(word, 0);
    assert_eq!(name_word_of(&model), Some(word));
    assert_eq!(name_word_of(&types_model()), None);
}

#[test]
fn the_root_type_is_read_from_the_header_and_the_block_table_alone() {
    let mut v = vec![0u8; 0x100];
    put(&mut v, 0x20, &2u32.to_le_bytes());
    put(&mut v, 0x38, &0x5000_0080u64.to_le_bytes());
    put(&mut v, 0x4C, &2u16.to_le_bytes());
    put(&mut v, 0x80, &C_MAP_DATA.to_le_bytes());
    put(&mut v, 0x90, &C_MAP_TYPES.to_le_bytes());
    assert_eq!(root_type(&v), Some(C_MAP_TYPES));
    put(&mut v, 0x20, &3u32.to_le_bytes());
    assert_eq!(root_type(&v), None, "a root past the table");
    assert_eq!(root_type(&v[..0x40]), None);
}

#[test]
fn every_changed_byte_is_read_or_refused_and_never_a_panic() {
    for whole in [map_model(), types_model()] {
        crate::gta6::mutated_tests::every_block_change(&whole, |m| {
            let _ = (read(m), lights::read(m), links::read(m));
        });
    }
}
