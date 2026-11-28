use super::*;

fn structure(fields: Vec<(u32, Value)>) -> Value {
    let mut of = MetaInstance::new("made_up");
    of.fields = fields
        .into_iter()
        .map(|(hash, v)| (name(hash), v))
        .collect();
    Value::Struct(Box::new(of))
}

fn root(value: Value) -> MetaInstance {
    match value {
        Value::Struct(of) => *of,
        _ => unreachable!(),
    }
}

fn aabb(min: [f32; 3], max: [f32; 3]) -> Value {
    structure(vec![
        (MIN, Value::Vec4([min[0], min[1], min[2], 0.0])),
        (MAX, Value::Vec3(max)),
    ])
}

fn bounds_tree(cells: usize) -> MetaInstance {
    let cell = |i: usize| aabb([i as f32, 0.0, -1.0], [i as f32 + 64.0, 64.0, 1.0]);
    root(structure(vec![
        (NAME, Value::String("n04_w19".into())),
        (
            PLACE,
            structure(vec![
                (CORNER, Value::Vec3([-9728.0, 1536.0, -299.5])),
                (PLACE_SECOND, Value::Vec3([1600.0, 40.0, 0.0])),
            ]),
        ),
        (
            BOX,
            aabb([-9728.0, 1536.0, -299.5], [-9216.0, 2048.0, -0.5]),
        ),
        (CELL_BOXES, Value::Array((0..cells).map(cell).collect())),
    ]))
}

#[test]
fn a_tile_s_bounds_are_read_from_its_members() {
    let bounds = TileBounds::from_tree(&bounds_tree(CELLS)).expect("the bounds read");
    assert_eq!(bounds.name, "n04_w19");
    assert_eq!(bounds.corner, [-9728.0, 1536.0, -299.5]);
    assert_eq!(bounds.place_second, [1600.0, 40.0, 0.0]);
    assert_eq!(bounds.bounds.max, [-9216.0, 2048.0, -0.5]);
    assert_eq!(bounds.cells.len(), CELLS);
    assert_eq!(bounds.cells[5].min, [5.0, 0.0, -1.0]);
    assert_eq!(bounds.cells[5].max, [69.0, 64.0, 1.0]);
    // A tile is eight by eight cells: any other count is refused.
    assert!(TileBounds::from_tree(&bounds_tree(63)).is_err());
}

fn layer(first: f32) -> Value {
    let members = LAYER_MEMBERS.iter().enumerate().map(|(i, (hash, floats))| {
        let v = first + i as f32;
        let value = match floats {
            4 => Value::Vec4([v, v + 0.25, v + 0.5, v + 0.75]),
            _ => Value::Float(f64::from(v)),
        };
        (*hash, value)
    });
    structure(members.collect())
}

fn material_tree(masks: usize) -> MetaInstance {
    let mut keys: Vec<Value> = (1..=11).map(|k| Value::UInt(0x100 + k)).collect();
    keys[7] = Value::UInt(name_hash64(b"n04_w19_tint"));
    keys.push(Value::Null);
    let names: Vec<Value> = (1..=12).map(|k| Value::UInt(k * 3)).collect();
    let textures = |i: u64| {
        let fields = (0..3).flat_map(|k| {
            [
                (TEXTURE_NAMES[k], Value::UInt(i * 10 + k as u64)),
                (TEXTURE_KEYS[k], Value::UInt(0xABC0 + i * 10 + k as u64)),
            ]
        });
        structure(fields.collect())
    };
    let mask = |i: u64| structure(vec![(MASK, Value::UInt(1 << i))]);
    root(structure(vec![
        (SLOT_NAMES, Value::Array(names)),
        (SLOT_KEYS, Value::Array(keys)),
        (LAYERS, Value::Array(vec![layer(1.0), layer(100.0)])),
        (LAYER_TEXTURES, Value::Array((0..2).map(textures).collect())),
        (
            LAYER_MASKS,
            Value::Array((0..masks as u64).map(mask).collect()),
        ),
    ]))
}

#[test]
fn a_tile_s_material_states_its_slots_and_its_layers_in_record_order() {
    let material = TileMaterial::from_tree(&material_tree(2)).expect("the material reads");
    assert_eq!(material.slot_names[0], 3);
    assert_eq!(material.slot_keys[0], 0x101);
    assert_eq!(material.slot_keys[11], 0, "an empty slot is nought");
    assert_eq!(material.layers.len(), 2);
    // The first member is four floats, the scale and offset; the next starts at the fifth.
    assert_eq!(&material.layers[0][..5], &[1.0, 1.25, 1.5, 1.75, 2.0]);
    // The last three members are four floats each, ending the 28.
    assert_eq!(material.layers[1][27], 115.75);
    assert_eq!(material.textures[1].names, [10, 11, 12]);
    assert_eq!(material.textures[1].keys, [0xABCA, 0xABCB, 0xABCC]);
    assert_eq!(material.masks, [1, 2]);
    assert!(material.is_of("n04_w19"));
    assert!(!material.is_of("n04_w18"));
    // Every layer has its textures and its mask, or the body is refused.
    assert!(TileMaterial::from_tree(&material_tree(1)).is_err());
}

#[test]
fn bytes_that_are_no_table_form_document_are_refused() {
    assert!(TileBounds::open(b"PSIN but nothing after it").is_err());
    assert!(TileMaterial::open(&[0u8; 64]).is_err());
}

#[test]
fn a_cell_s_drawable_names_its_tile_and_its_number() {
    assert_eq!(cell_of("n04_w19_027_hd.#dr"), Some(("n04_w19", 27)));
    assert_eq!(cell_of("n04_w19_064_hd"), Some(("n04_w19", 64)));
    assert_eq!(cell_of("n04_w19_065_hd"), None);
    assert_eq!(cell_of("n04_w19_027_lod"), None);
    assert_eq!(cell_of("n04_w19_slod2_children"), None);
    // Cells 1, 2, 9 and 10 share the first sub-tile; 64 is in the last.
    assert_eq!([1, 2, 9, 10].map(sub_tile_of), [0; 4]);
    assert_eq!(sub_tile_of(3), 1);
    assert_eq!(sub_tile_of(17), 4);
    assert_eq!(sub_tile_of(64), 15);
    assert_eq!(
        sub_tile_key("n04_w19", 0, 0),
        name_hash64(b"n04_w19_lod_001_ab")
    );
    assert_eq!(
        sub_tile_key("n04_w19", 15, 3),
        name_hash64(b"n04_w19_lod_016_mb")
    );
    let material = TileMaterial::from_tree(&material_tree(2)).expect("the material reads");
    assert_eq!(slot_key(&material, "n04_w19", 3), 0x104);
    assert_eq!(
        slot_key(&material, "n04_w19", 11),
        name_hash64(b"n04_w19_lod_mb")
    );
    assert_eq!(
        slot_key(&material, "n04_w19", 12),
        name_hash64(b"n04_w19_grass_mask")
    );
    assert_eq!(slot_key(&material, "n04_w19", 13), 0);
    assert_eq!(
        images_beside("levels/americas/map/terrain/n01_w17_n04_w20/n04_w19").as_deref(),
        Some("levels/americas/map/terrain/n01_w17_n04_w20/n01_w17_n04_w20_terrain_images_merged")
    );
}

#[test]
fn a_layer_reads_whichever_way_its_members_are_spelled() {
    // A decoder with a names snapshot spells some members as words and others as hashes.
    let spelled = |of: Value, word: bool| match of {
        Value::Struct(mut of) => {
            for ((n, _), (hash, _)) in of.fields.iter_mut().zip(LAYER_MEMBERS) {
                *n = match (word, hash) {
                    (true, h) if h == lit(b"bumpiness") => "bumpiness".into(),
                    (true, h) if h == lit(b"translucency") => "translucency".into(),
                    _ => format!("hash_{hash:08X}"),
                };
            }
            Value::Struct(of)
        }
        other => other,
    };
    for word in [true, false] {
        let mut tree = material_tree(2);
        let at = (tree.fields.iter())
            .position(|(n, _)| *n == name(LAYERS))
            .expect("layers");
        tree.fields[at].1 =
            Value::Array(vec![spelled(layer(1.0), word), spelled(layer(100.0), word)]);
        let material = TileMaterial::from_tree(&tree).expect("the material reads");
        assert_eq!(material.layers[0][4], 2.0, "spelled as words: {word}");
    }
}
