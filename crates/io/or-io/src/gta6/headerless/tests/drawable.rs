use super::*;

fn only_geometry(d: &Drawable) -> &Geometry {
    let models = d.levels[0].as_ref().expect("level 0");
    models[0].geometries[0]
        .as_ref()
        .expect("the geometry reads")
}

/// The parameters [`shader_into`] states, from its tables.
fn expected_parameters() -> Vec<Parameter> {
    let textures = TEXTURES.iter().enumerate().map(|(k, hash)| Parameter {
        hash: *hash,
        value: ParameterValue::Texture { slot: k as u8 },
    });
    let constants = CONSTANTS
        .iter()
        .map(|(hash, buffer, offset, words)| Parameter {
            hash: *hash,
            value: ParameterValue::Constant {
                buffer: *buffer,
                offset: *offset,
                words: (0..usize::from(*words))
                    .map(|i| constant_word(*hash, i))
                    .collect(),
            },
        });
    textures.chain(constants).collect()
}

#[test]
fn a_drawable_reads_to_its_shader_keys_triangles_and_positions() {
    let (img, phys, _) = drawable([1.0, 1.0, 0.0]);
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    assert_eq!(d.name, "box#dr");
    assert_eq!(
        d.shaders,
        [Shader {
            name_hash: SHADER_HASH,
            texture_keys: vec![Some(TEXTURE_KEY), None],
            parameters: expected_parameters(),
        }]
    );
    assert!(d.levels[1..].iter().all(Option::is_none));
    let g = only_geometry(&d);
    assert_eq!((g.shader, g.vertex_count), (0, 3));
    assert_eq!(g.triangles, [[0, 1, 2]]);
    assert_eq!(
        g.positions.as_ref().expect("positions"),
        &TRIANGLE_AT.to_vec()
    );
    assert!(d.embedded.is_none());
}

#[test]
fn the_parameter_block_s_buffers_and_texture_table_lie_where_the_constants_place_them() {
    let (img, _, parts) = drawable([1.0, 1.0, 0.0]);
    let target = |at: usize| u64_at(&img.bytes, at) as usize & 0x0FFF_FFFF;
    let block = target(parts.shader + 0x08);
    let sizes = buffer_sizes();
    assert_eq!(target(block), block + 16);
    assert_eq!(target(block + 8), block + 16 + sizes[0]);
    assert_eq!(
        target(parts.shader + 0x10),
        block + 16 + sizes[0] + sizes[1]
    );

    // A buffer, or the texture table, anywhere else is refused.
    for (at, moved) in [
        (block + 8, block + 16 + sizes[0] + 16),
        (parts.shader + 0x10, block + 16 + sizes[0] + sizes[1] + 8),
    ] {
        let (mut img, phys, _) = drawable([1.0, 1.0, 0.0]);
        img.ptr(at, moved);
        let e = read_drawable(&img.bytes, Some(&phys)).expect_err("a region is moved");
        assert!(matches!(e, HeaderlessError::Disagree { .. }), "{e}");
    }
}

#[test]
fn a_parameter_record_out_of_the_measured_order_kind_or_buffer_is_refused() {
    let word = |infos: usize, k: usize| infos + 12 + 8 * k;
    // Each changes the packed words of the second texture record and the first constant record.
    type Change = fn(u32, u32) -> (u32, u32);
    let cases: [(&str, Change); 4] = [
        ("a constant before a texture", |t, c| (c, t)),
        ("a kind of 2", |t, c| (t, (c & !3) | 2)),
        ("a buffer past the count", |t, c| (t, (c & !0xFC) | 2 << 2)),
        ("a constant of no words", |t, c| (t, c & 0x3F_FFFF)),
    ];
    for (what, change) in cases {
        let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
        let (t, c) = (
            word(parts.parameter_infos, 1),
            word(parts.parameter_infos, TEXTURES.len()),
        );
        let (t_word, c_word) = change(u32_at(&img.bytes, t), u32_at(&img.bytes, c));
        img.u32(t, t_word);
        img.u32(c, c_word);
        let e = read_drawable(&img.bytes, Some(&phys)).expect_err(what);
        assert!(
            matches!(
                e,
                HeaderlessError::Constant { .. } | HeaderlessError::IndexPastCount { .. }
            ),
            "{what}: {e}"
        );
    }
}

#[test]
fn a_constant_s_offset_and_size_are_read_from_their_own_bits() {
    // Exchanging the two fields of the one-word constant at offset 8 names another place.
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let k = TEXTURES.len() + 3;
    assert_eq!(CONSTANTS[3].2, 8);
    let at = parts.parameter_infos + 12 + 8 * k;
    let w = u32_at(&img.bytes, at);
    let (offset, size) = ((w >> 8) & 0x3FFF, w >> 22);
    img.u32(at, (w & 0xFF) | size << 8 | offset << 22);
    match read_drawable(&img.bytes, Some(&phys)) {
        Ok(d) => assert_ne!(d.shaders[0].parameters, expected_parameters()),
        Err(e) => assert!(matches!(e, HeaderlessError::Disagree { .. }), "{e}"),
    }
}

#[test]
fn quantised_positions_interleave_their_lanes_bytes_in_the_order_the_axis_word_names() {
    let (mut img, mut phys, parts) = drawable([1.0, 1.0, 1.0]);
    let origin = [-1.0f32, 0.5, 2.0];
    let step = [0.25f32, 2f32.powi(-16), 2f32.powi(-8)];
    let mut block = [0u8; 0x20];
    for l in 0..3 {
        block[4 * l..4 * l + 4].copy_from_slice(&origin[l].to_le_bytes());
        block[0x10 + 4 * l..0x14 + 4 * l].copy_from_slice(&step[l].to_le_bytes());
    }
    block[0x1C..].copy_from_slice(&4.0f32.to_le_bytes());
    // Axis word 0: lane A is z, lane B is y, lane C is x. Lane B's low byte is the element's
    // first byte, which a reading of three contiguous bit fields gives to lane A.
    let lanes = [[0u32, 0, 0], [0x12_3456, 0, 0], [0, 0xAB_CD01, 0x0203]];
    let stream = Stream {
        channel: 0,
        code: 0x10,
        size: 8,
        bytes: lanes
            .iter()
            .flat_map(|q| pack_lanes(*q).to_le_bytes())
            .collect(),
    };
    put_vertex_block(&mut img, &mut phys, &parts, 3, vec![stream], Some(block));
    let expected: Vec<[f32; 3]> = lanes
        .iter()
        .map(|q| {
            let at =
                |l: usize| (f64::from(origin[l]) + f64::from(q[l]) * f64::from(step[l])) as f32;
            [at(2), at(1), at(0)]
        })
        .collect();
    let corner = |pick: fn(f32, f32) -> f32| {
        std::array::from_fn(|a| {
            expected
                .iter()
                .map(|p| p[a])
                .reduce(pick)
                .expect("vertices")
        })
    };
    img.vec3(parts.geometry_box, corner(f32::min));
    img.vec3(parts.geometry_box + 0x10, corner(f32::max));
    let read = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    assert_eq!(
        only_geometry(&read).positions.as_ref().expect("positions"),
        &expected
    );
}

#[test]
fn positions_that_miss_the_box_are_refused_and_the_triangles_still_read() {
    let (img, phys, _) = drawable([5.0, 5.0, 5.0]);
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let g = only_geometry(&d);
    assert_eq!(g.triangles, [[0, 1, 2]]);
    assert!(matches!(
        g.positions,
        Err(HeaderlessError::Positions { .. })
    ));
}

#[test]
fn a_record_that_is_not_a_drawable_root_is_refused() {
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    img.put(parts.root + 0x7C, &0.0f32.to_le_bytes());
    assert!(matches!(
        read_drawable(&img.bytes, Some(&phys)),
        Err(HeaderlessError::NotADrawable { at: 0 })
    ));
}

#[test]
fn a_declaration_that_is_not_structure_of_arrays_refuses_its_geometry() {
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    img.bytes[parts.declaration + 0x138] &= !1;
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let models = d.levels[0].as_ref().expect("level 0");
    assert!(matches!(
        models[0].geometries[0],
        Err(HeaderlessError::Declaration { .. })
    ));
}

#[test]
fn a_meshlet_list_whose_header_its_record_does_not_state_refuses_its_geometry() {
    let (img, mut phys, parts) = drawable([1.0, 1.0, 0.0]);
    // Six triangles in the header, where the meshlet record states one.
    phys[parts.index_data + 4] = 6;
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let models = d.levels[0].as_ref().expect("level 0");
    assert!(matches!(
        models[0].geometries[0],
        Err(HeaderlessError::Meshlets { .. })
    ));
}

#[test]
fn a_meshlet_list_longer_than_its_parts_refuses_its_geometry() {
    let (mut img, mut phys, parts) = drawable([1.0, 1.0, 0.0]);
    phys.extend([0; 16]);
    img.u32(parts.index_buffer + 0x08, 16);
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let models = d.levels[0].as_ref().expect("level 0");
    assert!(matches!(
        models[0].geometries[0],
        Err(HeaderlessError::Meshlets {
            why: "its length is not the sum of its parts"
        })
    ));
}

#[test]
fn vertex_data_past_the_physical_section_refuses_its_geometry() {
    let (img, _, _) = drawable([1.0, 1.0, 0.0]);
    let d = read_drawable(&img.bytes, Some(&[])).expect("the drawable reads");
    let models = d.levels[0].as_ref().expect("level 0");
    assert!(matches!(
        models[0].geometries[0],
        Err(HeaderlessError::PastEnd {
            section: "physical",
            ..
        })
    ));
}

#[test]
fn a_physical_section_stated_and_not_read_refuses_each_geometry_as_not_read() {
    let (img, _, _) = drawable([1.0, 1.0, 0.0]);
    let d = read_drawable(&img.bytes, None).expect("the drawable reads");
    let models = d.levels[0].as_ref().expect("level 0");
    assert!(matches!(
        models[0].geometries[0],
        Err(HeaderlessError::PhysicalNotRead {
            what: "vertex data",
            ..
        })
    ));
}

#[test]
fn a_drawable_whose_bound_pointer_runs_past_the_body_is_refused() {
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    img.ptr(parts.root + 0xC8, 0x0FFF_FFF0);
    assert!(matches!(
        read_drawable(&img.bytes, Some(&phys)),
        Err(HeaderlessError::PastEnd { what: "bound", .. })
    ));
}

#[test]
fn a_drawable_reads_its_level_distances_and_the_composite_it_points_at_from_0xc8() {
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let distances = [12.5f32, 40.0, 150.0, 9998.0];
    for (l, d) in distances.iter().enumerate() {
        img.put(parts.root + 0x70 + 4 * l, &d.to_le_bytes());
    }
    let bound = bound_into(&mut img).root;
    img.ptr(parts.root + 0xC8, bound);
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    assert_eq!(d.lod_distances, distances);
    let bound = d.bound.expect("its composite");
    assert_eq!(bound.children.len(), 1);
    assert_eq!(bound.box_max, [1.0, 1.0, 0.0]);

    // A lone bound there reads as a composite of one, placed by the identity; one of no
    // measured type is refused.
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let bound = bound_into(&mut img).root;
    let child = u64_at(
        &img.bytes,
        u64_at(&img.bytes, bound + 0x70) as usize & 0x0FFF_FFFF,
    );
    img.ptr(parts.root + 0xC8, child as usize & 0x0FFF_FFFF);
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let lone = d.bound.expect("its lone bound");
    let [only] = lone.children.as_slice() else {
        panic!("one child, not {:?}", lone.children);
    };
    assert!(matches!(only.bound, Some(Ok(ChildBound::Geometry(_)))));
    assert_eq!(only.matrix[1], [0.0, 1.0, 0.0, 0.0]);
    img.bytes[(child as usize & 0x0FFF_FFFF) + 0x10] = 7;
    assert!(matches!(
        read_drawable(&img.bytes, Some(&phys)),
        Err(HeaderlessError::BoundNotLaidOut { found: 7, .. })
    ));

    // A box there is a composite of it alone, placed where it is.
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let bare = img.alloc(0x70);
    img.bytes[bare + 0x10] = 3;
    let (centre, half) = ([0.5f32, -2.0, 1.0], [0.25f32, 0.5, 1.5]);
    let radius = half.iter().map(|h| h * h).sum::<f32>().sqrt();
    img.put(bare + 0x14, &radius.to_le_bytes());
    img.vec3(bare + 0x20, std::array::from_fn(|a| centre[a] + half[a]));
    img.vec3(bare + 0x30, std::array::from_fn(|a| centre[a] - half[a]));
    img.vec3(bare + 0x40, centre);
    img.u32(bare + 0x4C, 0x7646);
    img.ptr(parts.root + 0xC8, bare);
    let d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let bound = d.bound.expect("its box as a composite");
    assert_eq!((bound.centroid, bound.radius), (centre, radius));
    let [child] = &bound.children[..] else {
        panic!("one child, not {:?}", bound.children);
    };
    assert_eq!(child.matrix[0], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(child.matrix[3], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(child.material_id, Some(0x7646));
    assert!(matches!(
        &child.bound,
        Some(Ok(ChildBound::Box { box_min, .. })) if *box_min == [0.25, -2.5, -0.5]
    ));
}

#[test]
fn a_drawable_that_names_one_model_and_one_geometry_many_times_is_refused() {
    const NAMED: usize = 64;
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let target = |img: &Image, at: usize| u64_at(&img.bytes, at) as usize & 0x0FFF_FFFF;
    let level = target(&img, parts.root + 0x50);
    let model = target(&img, target(&img, level));
    let geometry = target(&img, target(&img, model + 0x08));
    let geometries = img.pointers(&[geometry; NAMED]);
    let boxes = img.alloc(32 * (NAMED + 1));
    for b in 0..=NAMED {
        img.vec3(boxes + 32 * b + 0x10, [1.0, 1.0, 0.0]);
    }
    let shader_indices = img.alloc(2 * NAMED);
    img.ptr(model + 0x08, geometries);
    img.ptr(model + 0x18, boxes);
    img.ptr(model + 0x20, shader_indices);
    img.u16(model + 0x10, NAMED as u16);
    img.u16(model + 0x36, NAMED as u16);
    let models = img.pointers(&[model; NAMED]);
    img.ptr(level, models);
    img.u16(level + 8, NAMED as u16);
    img.u16(level + 0xA, NAMED as u16);
    assert!(matches!(
        read_drawable(&img.bytes, Some(&phys)),
        Err(HeaderlessError::Aliased { .. })
    ));
}

/// The built drawable's one model, found through its level as the reader finds it.
fn model_record(img: &Image, parts: &DrawableParts) -> usize {
    let at = |p: usize| u64_at(&img.bytes, p) as usize & 0x0FFF_FFFF;
    at(at(at(parts.root + 0x50)))
}

/// The built drawable's model with a palette of `count` entries over `words`, read.
fn palette_of(count: u8, words: &[u32]) -> (Result<Palette>, u8) {
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let model = model_record(&img, &parts);
    let list = img.alloc(4 * words.len());
    for (k, w) in words.iter().enumerate() {
        img.u32(list + 4 * k, *w);
    }
    img.ptr(model + 0x28, list);
    img.bytes[model + 0x30] = count;
    img.bytes[model + 0x33] = 2;
    let mut d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let model = d.levels[0].take().expect("level 0").remove(0);
    (model.palette, model.matrix_index)
}

#[test]
fn ascending_bone_hashes_are_a_palette_of_hashes_and_a_mask_or_none_indexes_the_own_skeleton() {
    let hashes = [0x0A3C_77E1, 0x0F00_0002, 0x6D19_C4B5];
    let (palette, matrix_index) = palette_of(3, &hashes);
    assert_eq!(palette.unwrap(), Palette::Hashes(hashes.to_vec()));
    assert_eq!(matrix_index, 2);

    // A mask of 52 bones: two words, the second's bits below 52 - 32, and then another record's.
    let mask = [0x9E27_51C6, 0x0003_B8A9, 0x0000_0001];
    assert_eq!(palette_of(52, &mask).0.unwrap(), Palette::Own);
    // One index under a count of 1 states a mask word of zero.
    assert_eq!(palette_of(1, &[0]).0.unwrap(), Palette::Own);
    // A count of 1 over one hash.
    assert_eq!(
        palette_of(1, &[0x6D19_C4B5]).0.unwrap(),
        Palette::Hashes(vec![0x6D19_C4B5])
    );

    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let model = model_record(&img, &parts);
    img.bytes[model + 0x30] = 9;
    let mut d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
    let model = d.levels[0].take().expect("level 0").remove(0);
    assert_eq!(model.palette.unwrap(), Palette::Own, "no list");
}

#[test]
fn a_list_neither_ascending_nor_a_mask_under_its_count_is_named_and_the_model_still_reads() {
    // A word twice, and the first word has bits at or past 3.
    let (palette, _) = palette_of(3, &[0x6D19_C4B5, 0x0F00_0002, 0x0F00_0002]);
    assert!(
        matches!(palette, Err(HeaderlessError::Palette { count: 3, .. })),
        "{palette:?}"
    );
    // Out of order, with a 0 after the first entry.
    let (palette, _) = palette_of(3, &[0x6D19_C4B5, 0, 0x0A3C_77E1]);
    assert!(
        matches!(palette, Err(HeaderlessError::Palette { count: 3, .. })),
        "{palette:?}"
    );
}

#[test]
fn a_zero_first_ascending_list_and_distinct_hashes_out_of_order_are_hash_palettes() {
    // Up to 32 entries a 0 first word also reads as a mask of no bone past the count.
    let zero_first = [0, 0x0A3C_77E1, 0x0F00_0002, 0x6D19_C4B5];
    assert_eq!(
        palette_of(4, &zero_first).0.unwrap(),
        Palette::Hashes(zero_first.to_vec())
    );
    let unordered = [0x6D19_C4B5, 0x0F00_0002, 0x0A3C_77E1];
    assert_eq!(
        palette_of(3, &unordered).0.unwrap(),
        Palette::Hashes(unordered.to_vec())
    );
}

#[test]
fn a_palette_past_255_entries_takes_its_count_from_the_geometry_whose_low_byte_the_model_keeps() {
    let hashes: Vec<u32> = (0..300).map(|k| 0x0100_0000 + 7 * k).collect();
    // The u16 at +0x2E is another word, so the palette is only the u16 at +0x2C.
    let read = |byte: u8, after: u32| {
        let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
        let model = model_record(&img, &parts);
        let list = img.alloc(4 * hashes.len());
        for (k, w) in hashes.iter().enumerate() {
            img.u32(list + 4 * k, *w);
        }
        img.ptr(model + 0x28, list);
        img.bytes[model + 0x30] = byte;
        let geometry = u64_at(
            &img.bytes,
            u64_at(&img.bytes, model + 0x08) as usize & 0x0FFF_FFFF,
        );
        img.u32(
            (geometry as usize & 0x0FFF_FFFF) + 0x2C,
            hashes.len() as u32 | after << 16,
        );
        let mut d = read_drawable(&img.bytes, Some(&phys)).expect("the drawable reads");
        d.levels[0].take().expect("level 0").remove(0).palette
    };
    assert_eq!(
        read(44, 0).unwrap(),
        Palette::Hashes(hashes.clone()),
        "300 & 0xFF = 44"
    );
    assert_eq!(read(45, 0).unwrap(), Palette::Hashes(hashes[..45].to_vec()));
    assert_eq!(read(44, 1).unwrap(), Palette::Hashes(hashes.clone()));
}

#[test]
fn a_model_s_word_is_the_hash_of_the_name_its_root_states_without_its_ending() {
    let word = |name: &[u8]| {
        let mut img = Image::default();
        let root = img.alloc(0xB0);
        let text = img.alloc(name.len() + 1);
        img.put(text, name);
        img.ptr(root + 0xA8, text);
        stated_model_word(&img.bytes)
    };
    let hash = crate::gta6::hash::name_hash64;
    assert_eq!(word(b"Prop_Bench_01.#dr").unwrap(), hash(b"prop_bench_01"));
    assert_eq!(word(b"prop_door_02.#FT").unwrap(), hash(b"prop_door_02"));
    assert_eq!(word(b"prop_lamp").unwrap(), hash(b"prop_lamp"));
    assert_ne!(word(b"prop_lamp.#tx").unwrap(), hash(b"prop_lamp"));
    let mut img = Image::default();
    img.alloc(0xB0);
    img.ptr(0xA8, 0x0FF0);
    assert!(
        stated_model_word(&img.bytes).is_err(),
        "a name past the section"
    );
}

#[test]
fn a_model_states_its_name_as_written_and_a_section_cut_before_the_name_s_end_states_none() {
    let stated = |name: &[u8]| {
        let mut img = Image::default();
        let root = img.alloc(0xB0);
        let text = img.alloc(name.len() + 1);
        img.put(text, name);
        img.ptr(root + 0xA8, text);
        (img.bytes, text)
    };
    let (ended, _) = stated(b"Zq_Bench_01.#dr");
    assert_eq!(stated_model_name(&ended).unwrap(), "Zq_Bench_01.#dr");
    // A fragment's root states its name with no ending.
    let (fragment, text) = stated(b"zq_door_02");
    assert_eq!(stated_model_name(&fragment).unwrap(), "zq_door_02");
    // What a harvest holds of a body may end at the name, or inside it, where no NUL ends it.
    assert!(stated_model_name(&fragment[..text]).is_err());
    assert!(stated_model_name(&fragment[..text + 4]).is_err());
    assert!(stated_model_name(&fragment[..0xA8]).is_err());
    let (built, _, parts) = drawable([1.0, 1.0, 0.0]);
    assert_eq!(parts.root, 0);
    assert_eq!(stated_model_name(&built.bytes).unwrap(), "box#dr");
    // A root with no name pointer states none.
    let mut null = built.bytes;
    null[0xA8..0xB0].fill(0);
    assert!(stated_model_name(&null).is_err());
}

#[test]
fn the_trace_words_are_the_level_word_each_bucket_mask_render_mask_key_word_and_the_two_counts() {
    let (mut img, _, parts) = drawable([1.0, 1.0, 0.0]);
    let target = |img: &Image, at: usize| u64_at(&img.bytes, at) as usize & 0x0FFF_FFFF;
    let model = target(&img, target(&img, target(&img, parts.root + 0x50)));
    let geometry = target(&img, target(&img, model + 0x08));
    let record = target(&img, geometry + 0x40);
    img.u32(parts.root + 0xB8, 0x0000_0400);
    img.u32(parts.shader + 0x3C, 0xFF08);
    img.bytes[model + 0x34] = 0xA5;
    img.u64(geometry + 0x30, 0x0123_4567_89AB_CDEF);
    let read = |img: &Image| trace_words(Rpf9Role::Drawable, &img.bytes);
    let expected = TraceWords {
        level_word: 0x0000_0400,
        shaders: vec![TraceShader {
            name_hash: SHADER_HASH,
            bucket_mask: 0xFF08,
        }],
        levels: [
            Some(vec![TraceModel {
                render_mask: 0xA5,
                geometries: vec![TraceGeometry {
                    shader: 0,
                    key_word: 0x0123_4567_89AB_CDEF,
                    triangle_count: 1,
                    vertex_count: 3,
                }],
            }]),
            None,
            None,
            None,
        ],
    };
    let mut words = read(&img);
    assert_eq!(words.len(), 1);
    assert_eq!(words.remove(0).expect("the words read"), expected);

    // The triangle count is 23 bits: seven of them lie in the next word's low byte, under a bit
    // of another field.
    img.u32(record + 0x0C, 3 << 8 | 0x82);
    let words = read(&img).remove(0).expect("the words read");
    let geometry_words = words.levels[0].as_ref().expect("level 0")[0].geometries[0];
    assert_eq!(geometry_words.triangle_count, 2 << 16 | 1);

    // A geometry that names a shader the drawable lacks is refused.
    let indices = target(&img, model + 0x20);
    img.u16(indices, 1);
    assert!(matches!(
        read(&img).as_slice(),
        [Err(HeaderlessError::IndexPastCount { .. })]
    ));
}

#[test]
fn a_body_of_another_role_states_no_trace_words_and_a_root_with_no_drawable_is_refused() {
    let (mut img, _, parts) = drawable([1.0, 1.0, 0.0]);
    img.u32(parts.shader + 0x3C, 0xFF01);
    let own = trace_words(Rpf9Role::Drawable, &img.bytes);
    assert!(matches!(own.as_slice(), [Ok(w)] if w.shaders[0].bucket_mask == 0xFF01));
    assert!(trace_words(Rpf9Role::Bound, &img.bytes).is_empty());
    // A fragment's drawable is the record its root points at from 0x18; this root has none.
    assert!(matches!(
        trace_words(Rpf9Role::DrawableWithBound, &img.bytes).as_slice(),
        [Err(_)]
    ));
}
