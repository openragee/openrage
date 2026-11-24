use super::super::lights::tests::{array, block, model_of, ptr, put, put_floats};
use super::super::{
    C_ENTITY_DEF, C_MAP_DATA, C_MAP_TYPES, C_MLO_INSTANCE_DEF, ENTITY_SIZE, GRASS_LIST,
    GRASS_LIST_SIZE, MAP_DATA_SIZE, MAP_TYPES_SIZE,
};
use super::*;

/// A quarter turn about z, moved to (10, 20, 30): the model's x points along the world's y.
const TURNED: [[f32; 4]; 3] = [
    [0.0, 1.0, 0.0, 10.0],
    [-1.0, 0.0, 0.0, 20.0],
    [0.0, 0.0, 1.0, 30.0],
];
/// Twice as wide and long and half as high, moved to (1, 2, 3).
const SCALED: [[f32; 4]; 3] = [
    [2.0, 0.0, 0.0, 1.0],
    [0.0, 2.0, 0.0, 2.0],
    [0.0, 0.0, 0.5, 3.0],
];

/// An entity record of `level` with `children` children, whose parent is `parent` of the map
/// `selector` picks and, in a prop list, its `instance`.
fn entity(
    seed: u32,
    level: u8,
    selector: i8,
    parent: i16,
    instance: i16,
    children: u32,
) -> Vec<u8> {
    let mut e = vec![0u8; ENTITY_SIZE as usize];
    put_floats(&mut e, 0x10, &[0.0, 0.0, 0.6, 0.8]);
    put(&mut e, 0x20, &(u64::from(seed) << 32 | 0xB1).to_le_bytes());
    put_floats(&mut e, 0x38, &[seed as f32, 2.0 * seed as f32, -3.0]);
    put(&mut e, 0x44, &(seed | 0xC1).to_le_bytes());
    put(&mut e, 0x48, &0x18_0000u32.to_le_bytes());
    put_floats(&mut e, 0x54, &[1.5, 2.5]);
    put(&mut e, 0x5C, &parent.to_le_bytes());
    put(&mut e, 0x5E, &instance.to_le_bytes());
    put_floats(&mut e, 0x60, &[-1.0, 30.0]);
    put(&mut e, 0x68, &children.to_le_bytes());
    e[0x7E] = level;
    e[0x7F] = selector as u8;
    e
}

fn bytes<const N: usize>(words: &[[u8; N]]) -> Vec<u8> {
    words.concat()
}

/// Two parent maps and three entities, blocks 5 to 7: one of level 2 with no parent, one of level 1
/// under it in the map itself, and an interior under instance 3 of element 5 of the second parent
/// map; one prop list, block 8, of two instances under element 4 of the first; two grass lists.
fn stream_model() -> RbmModel {
    let mut root = vec![0u8; MAP_DATA_SIZE];
    put(&mut root, 0x08, &0x1111_0001u32.to_le_bytes());
    put(&mut root, 0x18, &bytes(&[2u32.to_le_bytes(); 2]));
    put(&mut root, 0x20, &ptr(2, 0).to_le_bytes());
    put(&mut root, 0x50, &0x5050_0005u32.to_le_bytes());
    array(&mut root, 0x58, ptr(3, 0), 2);
    put(&mut root, 0x68, &0x2222_0001u32.to_le_bytes());
    put(&mut root, 0x80, &0x81u32.to_le_bytes());
    put(&mut root, 0x84, &0x401u32.to_le_bytes());
    put_floats(
        &mut root,
        0x90,
        &[-100.0, -200.0, -30.0, 0.0, 100.0, 200.0, 30.0],
    );
    put_floats(&mut root, 0xB0, &[-10.0, -20.0, -3.0, 0.0, 10.0, 20.0, 3.0]);
    array(&mut root, 0xD0, ptr(4, 0), 3);
    array(&mut root, 0x148, ptr(8, 0), 1);
    array(&mut root, 0x158, ptr(10, 0), 2);

    let mut props = vec![0u8; PROP_LIST_SIZE as usize];
    put(&mut props, 0x20, &0xD1D1_0000_0000_00D1u64.to_le_bytes());
    put(&mut props, 0x44, &0xD1u32.to_le_bytes());
    put(&mut props, 0x5C, &4i16.to_le_bytes());
    put(&mut props, 0x5E, &(-1i16).to_le_bytes());
    put_floats(&mut props, 0x60, &[80.0, 20.0]);
    props[0x7E] = 1;
    array(&mut props, 0x90, ptr(9, 0), 2);
    put_floats(&mut props, 0xA0, &[-1.0, -2.0, -3.0, 0.0, 4.0, 5.0, 6.0]);
    let mut instances = vec![0u8; 2 * 0x40];
    put_floats(&mut instances, 0x00, TURNED.as_flattened());
    put_floats(&mut instances, 0x40, SCALED.as_flattened());

    let mut grass = vec![0u8; 2 * GRASS_LIST_SIZE as usize];
    for list in 0..2 {
        let at = list * GRASS_LIST_SIZE as usize + 0x5C;
        put(&mut grass, at, &(-1i16).to_le_bytes());
    }

    model_of(
        super::super::LAYOUTS.iter().chain(links::LAYOUTS),
        1,
        vec![
            block(C_MAP_DATA, root),
            block(
                0x4F,
                bytes(&[0xAAAA_0000_0000_0001u64, 0xBBBB_0000_0000_0002].map(u64::to_le_bytes)),
            ),
            block(
                0x4A,
                bytes(&[0x2222_0001u32, 0x3333_0002].map(u32::to_le_bytes)),
            ),
            block(
                7,
                bytes(&[ptr(5, 0), ptr(6, 0), ptr(7, 0)].map(u64::to_le_bytes)),
            ),
            block(C_ENTITY_DEF, entity(0x100, 2, -1, -1, -1, 1)),
            block(C_ENTITY_DEF, entity(0x200, 1, -1, 0, -1, 1)),
            block(C_MLO_INSTANCE_DEF, entity(0x300, 0, 1, 5, 3, 0)),
            block(PROP_LIST, props),
            block(PROP_INSTANCE, instances),
            block(GRASS_LIST, grass),
        ],
    )
}

fn streamed() -> StreamMap {
    let entity = |seed: u32, rotation, lod_level, num_children, interior, parent| StreamEntity {
        archetype_word: u64::from(seed) << 32 | 0xB1,
        archetype_name: seed | 0xC1,
        position: [seed as f32, 2.0 * seed as f32, -3.0],
        rotation,
        scale_xy: 1.5,
        scale_z: 2.5,
        lod_dist: -1.0,
        child_lod_dist: 30.0,
        num_children,
        lod_level,
        flags: 0x18_0000,
        interior,
        parent,
    };
    let in_the_map = StreamLink {
        map: None,
        index: 0,
        instance: None,
    };
    let in_the_second_parent = StreamLink {
        map: Some(1),
        index: 5,
        instance: Some(3),
    };
    StreamMap {
        name: 0x1111_0001,
        block: 0x5050_0005,
        parent: 0x2222_0001,
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
        lod_parents: vec![0x2222_0001, 0x3333_0002],
        entities: vec![
            entity(0x100, [0.0, 0.0, -0.6, 0.8], 2, 1, false, None),
            entity(0x200, [0.0, 0.0, -0.6, 0.8], 1, 1, false, Some(in_the_map)),
            entity(
                0x300,
                [0.0, 0.0, 0.6, 0.8],
                0,
                0,
                true,
                Some(in_the_second_parent),
            ),
        ],
        prop_lists: vec![StreamPropList {
            archetype_word: 0xD1D1_0000_0000_00D1,
            archetype_name: 0xD1,
            lod_dist: 80.0,
            child_lod_dist: 20.0,
            lod_level: 1,
            parent: Some(StreamLink {
                map: Some(0),
                index: 4,
                instance: None,
            }),
            batch_aabb: Aabb {
                min: [-1.0, -2.0, -3.0],
                max: [4.0, 5.0, 6.0],
            },
            instances: vec![TURNED, SCALED],
        }],
        grass_lists: 2,
    }
}

#[test]
fn a_map_streams_its_own_fields_and_each_entity_and_prop_list_with_its_parent_and_placing() {
    assert_eq!(read(&stream_model()).expect("it reads"), streamed());
}

#[test]
fn an_instance_s_rows_are_its_model_s_axes_and_their_fourth_column_moves_it() {
    let model = stream_model();
    let map = read(&model).expect("it reads");
    let [turned, scaled] = map.prop_lists[0].instances[..] else {
        panic!("two instances");
    };
    assert_eq!(place(&turned, [1.0, 0.0, 0.0]), [10.0, 21.0, 30.0]);
    assert_eq!(place(&turned, [0.0, 1.0, 0.0]), [9.0, 20.0, 30.0]);
    assert_eq!(place(&turned, [0.0, 0.0, 1.0]), [10.0, 20.0, 31.0]);
    assert_eq!(place(&scaled, [1.0, 1.0, 2.0]), [3.0, 4.0, 4.0]);
    let Links::Data(links) = links::read(&model).expect("its links read") else {
        panic!("a map's links");
    };
    let origins = [turned, scaled].map(|instance| place(&instance, [0.0; 3]));
    assert_eq!(
        links.prop_points[0], origins,
        "the link reader's point is where the model's origin lands"
    );
}

#[test]
fn an_index_into_the_own_map_that_names_no_earlier_entity_is_no_parent() {
    for itself_or_later in [1i16, 2, 9] {
        let mut m = stream_model();
        put(
            &mut m.data_blocks[5].data,
            0x5C,
            &itself_or_later.to_le_bytes(),
        );
        let map = read(&m).expect("the body still reads");
        assert_eq!(
            map.entities[1].parent, None,
            "parentIndex {itself_or_later}"
        );
    }
    let mut m = stream_model();
    m.data_blocks[7].data[0x7F] = 0xFF;
    let map = read(&m).expect("it reads");
    assert_eq!(
        map.prop_lists[0].parent, None,
        "a prop list that picks no map"
    );
}

type Patch = Box<dyn Fn(&mut RbmModel)>;

#[test]
fn a_body_of_another_root_and_one_a_reader_refuses_are_refused() {
    let cases: Vec<(Patch, &str)> = vec![
        (
            Box::new(|m: &mut RbmModel| m.data_blocks[0].data_type = 0x1234_5678),
            "the root structure 0x12345678 is neither",
        ),
        (
            Box::new(|m: &mut RbmModel| {
                m.data_blocks[0] = block(C_MAP_TYPES, vec![0; MAP_TYPES_SIZE])
            }),
            "it is a CMapTypes, where a streamer takes a CMapData",
        ),
        (
            Box::new(|m: &mut RbmModel| m.data_blocks[4].data[0x7F] = 2),
            "selector 2 is past the 2 maps",
        ),
        (
            Box::new(|m: &mut RbmModel| m.data_blocks[7].data[0x7E] = 7),
            "PropInstanceList[0]: its lodLevel 7 is no LODTYPES_DEPTH value",
        ),
        (
            Box::new(|m: &mut RbmModel| {
                put_floats(&mut m.data_blocks[8].data, 0x44, &[f32::INFINITY])
            }),
            "PropInstanceList[0]: the number at 0x4 is not finite",
        ),
        (
            Box::new(|m: &mut RbmModel| {
                let class = m.classes.get_mut(&PROP_LIST).expect("declared");
                class.entries.retain(|member| member.offset != 0x60);
            }),
            "rage__fwPropInstanceListDef's member",
        ),
    ];
    for (patch, want) in cases {
        let mut m = stream_model();
        patch(&mut m);
        let err = read(&m).expect_err(want).to_string();
        assert!(err.contains(want), "{want}: {err}");
    }
    assert!(open(&[], &[]).is_err(), "two empty sections");
}

#[test]
fn every_truncated_block_is_an_error_and_never_a_panic() {
    let whole = stream_model();
    for b in 0..whole.data_blocks.len() {
        for len in 0..whole.data_blocks[b].data.len() {
            let mut m = whole.clone();
            m.data_blocks[b].data.truncate(len);
            let _ = read(&m);
        }
    }
}
