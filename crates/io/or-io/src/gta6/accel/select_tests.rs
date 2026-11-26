use super::*;
use crate::gta6::headerless::{
    Geometry, HeaderlessError, Model, Palette, TraceGeometry, TraceShader,
};

/// Shader 0 draws in the opaque bucket, 1 in the cutout one, 2 in both, 3 in a bucket the ray
/// tracer leaves out, and 4 in the opaque bucket outside the ray tracer's phase.
const MASKS: [u32; 5] = [0xFF01, 0xFF08, 0xFF09, 0xFF04, 0xDF01];

/// The name hash of shader `shader`.
fn name_of(shader: u16) -> u32 {
    0xE000 + u32::from(shader)
}

fn geometry(shader: u16, key_word: u64) -> TraceGeometry {
    TraceGeometry {
        shader,
        key_word,
        triangle_count: 10 + u32::from(shader),
        vertex_count: 20 + u32::from(shader),
    }
}

fn model(render_mask: u8, geometries: &[TraceGeometry]) -> TraceModel {
    TraceModel {
        render_mask,
        geometries: geometries.to_vec(),
    }
}

fn words(levels: [Option<Vec<TraceModel>>; 4]) -> TraceWords {
    TraceWords {
        level_word: 0,
        shaders: ((0u16..)
            .zip(MASKS)
            .map(|(shader, bucket_mask)| TraceShader {
                name_hash: name_of(shader),
                bucket_mask,
            }))
        .collect(),
        levels,
    }
}

/// A key word whose folded part is `folded`, under low bits the fold drops.
fn key_word(folded: u64) -> u64 {
    folded << 9 | 0x1FF
}

fn once(_: u32) -> bool {
    false
}

#[test]
fn the_level_is_the_stated_one_or_level_0_and_never_before_the_first_a_traced_model_is_in() {
    let traced = || Some(vec![model(0xFF, &[geometry(0, key_word(1))])]);
    let untraced = || Some(vec![model(0xDF, &[geometry(0, key_word(1))])]);

    assert_eq!(
        level(&words([traced(), traced(), None, None]), false),
        Some(0)
    );
    assert_eq!(
        level(&words([untraced(), traced(), None, None]), false),
        Some(1)
    );
    assert_eq!(level(&words([None, None, traced(), None]), false), Some(2));
    // Level 3 alone holds no structure's model, and neither does a drawable no phase traces.
    assert_eq!(level(&words([None, None, None, traced()]), false), None);
    assert_eq!(
        level(&words([untraced(), untraced(), None, None]), true),
        None
    );

    let stating = |stated: u32, levels| TraceWords {
        level_word: stated << 9 | 0x1FF,
        ..words(levels)
    };
    let three = || [traced(), traced(), traced(), None];
    assert_eq!(level(&stating(3, three()), false), Some(2));
    // A level the drawable lacks is not taken, and neither is one before the first traced.
    assert_eq!(
        level(&stating(3, [traced(), traced(), None, None]), false),
        Some(0)
    );
    assert_eq!(
        level(&stating(1, [untraced(), traced(), None, None]), false),
        Some(1)
    );
    let stated = select(&stating(2, three()), false, once);
    assert_eq!(stated.map(|s| s.level), Some(1));
    // The stated level is three bits, less one, and at most 3.
    let four = || [traced(), traced(), traced(), traced()];
    assert_eq!(level(&stating(4, four()), false), Some(3));
    assert_eq!(level(&stating(7, four()), false), Some(3));
}

#[test]
fn a_reduced_model_takes_level_1_where_a_model_of_it_is_traced_and_no_level_is_stated() {
    let traced = || Some(vec![model(0xFF, &[geometry(0, key_word(1))])]);
    let untraced = || Some(vec![model(0xDF, &[geometry(0, key_word(1))])]);
    assert_eq!(
        level(&words([traced(), traced(), traced(), None]), true),
        Some(1)
    );
    assert_eq!(
        level(&words([traced(), untraced(), traced(), None]), true),
        Some(0)
    );
    assert_eq!(
        level(&words([traced(), None, traced(), None]), true),
        Some(0)
    );
    assert_eq!(level(&words([None, None, traced(), None]), true), Some(2));
    let stating_0 = TraceWords {
        level_word: 1 << 9,
        ..words([traced(), traced(), None, None])
    };
    assert_eq!(level(&stating_0, true), Some(0));
}

#[test]
fn a_structure_a_traced_model_holds_its_opaque_and_cutout_geometries_in_file_order() {
    let all = [0, 1, 2, 3, 4].map(|s| geometry(s, key_word(1 + u64::from(s))));
    let level = vec![
        model(0xFF, &all),
        model(0xDF, &all),
        model(0x20, &[geometry(3, key_word(7))]),
        model(0x20, &[geometry(1, key_word(9)), geometry(0, key_word(8))]),
    ];
    let selection = select(&words([Some(level), None, None, None]), false, once);
    let selection = selection.expect("a selection");
    let kept = |geometry, opaque| Selected {
        geometry,
        opaque,
        second: false,
    };
    let structures: Vec<_> = (selection.structures.iter())
        .map(|s| (s.model, s.geometries.clone()))
        .collect();
    assert_eq!(
        structures,
        [
            (0, vec![kept(0, true), kept(1, false), kept(2, false)]),
            // A traced model that keeps no geometry has a structure of none.
            (2, vec![]),
            (3, vec![kept(0, false), kept(1, true)]),
        ]
    );
}

#[test]
fn a_geometry_of_an_effect_described_twice_is_kept_twice_and_folded_once() {
    let level = || {
        Some(vec![model(
            0x20,
            &[geometry(1, key_word(3)), geometry(0, key_word(4))],
        )])
    };
    let twice = select(&words([level(), None, None, None]), false, |name| {
        name == name_of(1)
    })
    .expect("a selection");
    let kept = |geometry, opaque, second| Selected {
        geometry,
        opaque,
        second,
    };
    assert_eq!(
        twice.structures[0].geometries,
        [
            kept(0, false, false),
            kept(0, false, true),
            kept(1, true, false)
        ]
    );
    let description: [u32; 13] = [
        3,
        0,
        INDEX_FORMAT,
        3 * 11,
        21,
        0,
        INDEX_FORMAT,
        3 * 11,
        21,
        1,
        INDEX_FORMAT,
        3 * 10,
        20,
    ];
    assert_eq!(
        twice.structures[0].checksum,
        crc32c(description.iter().flat_map(|w| w.to_le_bytes()))
    );
    let single = select(&words([level(), None, None, None]), false, once).expect("a selection");
    assert_eq!(twice.key, single.key);
    assert_ne!(twice.structures[0].checksum, single.structures[0].checksum);
}

#[test]
fn an_effect_is_described_twice_by_one_annotation_unless_it_holds_one_of_three_others() {
    let word = |key: u32, kind: u32| (key << 2 | kind, 1);
    assert!(described_twice(&[word(0x1234_5678, 0), word(TWICE, 0)]));
    assert!(described_twice(&[word(TWICE, 3)]));
    assert!(!described_twice(&[word(0x1234_5678, 0)]));
    assert!(!described_twice(&[]));
    // The hash of an annotation is 30 bits: the key's top two are not stated.
    assert!(!described_twice(&[(TWICE, 1)]));
    for other in NEVER_TWICE {
        assert!(!described_twice(&[word(TWICE, 0), word(other, 0)]));
    }
}

#[test]
fn a_level_the_drawable_lacks_and_a_shader_it_lacks_select_nothing() {
    let level = || Some(vec![model(0x20, &[geometry(0, key_word(1))])]);
    assert!(select_level(&words([level(), None, None, None]), 0, once).is_some());
    assert_eq!(
        select_level(&words([level(), None, None, None]), 1, once),
        None
    );
    assert_eq!(
        select_level(&words([level(), None, None, None]), 4, once),
        None
    );
    let past = Some(vec![model(0x20, &[geometry(5, key_word(1))])]);
    assert_eq!(
        select_level(&words([past, None, None, None]), 0, once),
        None
    );
}

#[test]
fn the_key_folds_the_kept_key_words_a_model_at_a_time_and_adds_its_bias() {
    let key = |level: Vec<TraceModel>| {
        (select(&words([Some(level), None, None, None]), false, once).expect("a selection")).key
    };
    let (a, b, c) = (0x12_3456_789A, 0x0F_EDCB_A987, 0x05_5555_AAAA);
    let g = |folded| geometry(0, key_word(folded));

    // One geometry's key is its word without its low nine bits, plus 6.
    assert_eq!(key(vec![model(0x20, &[g(a)])]), a + 6);
    assert_eq!(key(vec![model(0x20, &[g(a), g(b)])]), fold(a, b) + KEY_BIAS);
    assert_ne!(fold(a, b), fold(b, a));
    assert_eq!(
        key(vec![model(0x20, &[g(a)]), model(0x20, &[g(b), g(c)])]),
        fold(a, fold(b, c)).wrapping_add(KEY_BIAS)
    );
    assert_eq!(
        key(vec![model(0x20, &[g(a), g(b)]), model(0x20, &[g(c)])]),
        fold(fold(a, b), c).wrapping_add(KEY_BIAS)
    );
    assert_ne!(fold(a, fold(b, c)), fold(fold(a, b), c));

    // A geometry left out and a model left out fold nothing.
    let left_out = geometry(3, key_word(b));
    assert_eq!(key(vec![model(0x20, &[left_out, g(a)])]), a + KEY_BIAS);
    assert_eq!(
        key(vec![model(0xDF, &[g(b)]), model(0x20, &[g(a)])]),
        a + KEY_BIAS
    );
    // A traced model that keeps no geometry folds nought: after a model that kept some it
    // changes the key, and before one it is the first word folded and leaves it.
    let empty = || model(0x20, &[geometry(3, key_word(b))]);
    assert_eq!(
        key(vec![model(0x20, &[g(a)]), empty()]),
        fold(a, 0).wrapping_add(KEY_BIAS)
    );
    assert_ne!(fold(a, 0), a);
    assert_eq!(key(vec![empty(), model(0x20, &[g(a)])]), a + KEY_BIAS);
}

#[test]
fn a_structure_s_checksum_is_the_crc_32c_of_its_count_and_four_words_a_kept_geometry() {
    let level = vec![
        model(
            0x20,
            &[
                geometry(0, key_word(1)),
                geometry(3, key_word(2)),
                geometry(1, key_word(3)),
            ],
        ),
        model(0x20, &[geometry(2, key_word(4))]),
    ];
    let selection = select(&words([Some(level), None, None, None]), false, once);
    let selection = selection.expect("a selection");
    let checksum = |description: &[u32]| crc32c(description.iter().flat_map(|w| w.to_le_bytes()));
    assert_eq!(
        selection.structures[0].checksum,
        checksum(&[2, 1, INDEX_FORMAT, 3 * 10, 20, 0, INDEX_FORMAT, 3 * 11, 21])
    );
    assert_eq!(
        selection.structures[1].checksum,
        checksum(&[1, 0, INDEX_FORMAT, 3 * 12, 22])
    );
}

#[test]
fn the_crc_is_castagnoli_s_from_a_zero_register_with_no_inversion() {
    // CRC-32C's published check value is taken from a register of all ones and inverted; a
    // register of all ones is a zero register over a message whose first four bytes are inverted.
    let mut message = *b"123456789";
    message[..4].iter_mut().for_each(|b| *b = !*b);
    assert_eq!(!crc32c(message.into_iter()), 0xE306_9283);
    assert_eq!(crc32c([0u8; 8].into_iter()), 0);
}

/// A geometry of `count` triangles in a fan about the origin, all of shader `shader`.
fn fan(shader: u16, count: u32) -> Result<Geometry, HeaderlessError> {
    let rim = |k: u32| [k as f32, 1.0, shader.into()];
    Ok(Geometry {
        shader,
        vertex_count: count + 2,
        channels: Vec::new(),
        triangles: (0..count).map(|k| [0, k + 1, k + 2]).collect(),
        positions: Ok((std::iter::once([0.0; 3]).chain((0..=count).map(rim))).collect()),
        vertices: Default::default(),
        vertex_data: Vec::new(),
        index_data: Vec::new(),
    })
}

fn drawable(models: Vec<Vec<Result<Geometry, HeaderlessError>>>) -> Drawable {
    let models = models.into_iter().map(|geometries| Model {
        geometries,
        palette: Ok(Palette::Own),
        matrix_index: 0,
    });
    Drawable {
        name: String::new(),
        box_min: [0.0; 3],
        box_max: [0.0; 3],
        shaders: Vec::new(),
        levels: [Some(models.collect()), None, None, None],
        lod_distances: [0.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    }
}

#[test]
fn a_structure_s_triangles_are_its_kept_geometries_numbered_by_description_and_by_triangle() {
    // Model 0 keeps a cutout geometry described twice and an opaque one, and leaves one out;
    // model 1 is not traced; model 2 keeps nothing; model 3 keeps one.
    let level = vec![
        model(
            0x20,
            &[
                geometry(1, key_word(1)),
                geometry(3, key_word(2)),
                geometry(0, key_word(3)),
            ],
        ),
        model(0xDF, &[geometry(0, key_word(4))]),
        model(0x20, &[geometry(3, key_word(5))]),
        model(0x20, &[geometry(0, key_word(6))]),
    ];
    let selection = select(&words([Some(level), None, None, None]), false, |name| {
        name == name_of(1)
    })
    .expect("a selection");
    let read = drawable(vec![
        vec![fan(1, 2), fan(3, 5), fan(0, 3)],
        vec![fan(0, 7)],
        vec![fan(3, 1)],
        vec![fan(0, 1)],
    ]);
    let traced = triangles(&selection, &read).expect("the drawable the words are of");
    let stated: Vec<Vec<(u32, u32, bool)>> = (traced.iter())
        .map(|t| {
            (t.triangles
                .iter()
                .map(|t| (t.geometry, t.primitive, t.opaque)))
            .collect()
        })
        .collect();
    assert_eq!(
        stated,
        [
            vec![
                (0, 0, false),
                (0, 1, false),
                (1, 0, false),
                (1, 1, false),
                (2, 0, true),
                (2, 1, true),
                (2, 2, true),
            ],
            vec![(0, 0, true)],
        ]
    );
    assert_eq!(traced.iter().map(|t| t.model).collect::<Vec<_>>(), [0, 3]);
    // The corners are the positions the triangle names, in its own order, and the second
    // description's are the first's.
    let corners = |traced: &Traced, n: usize| traced.triangles[n].corners;
    assert_eq!(
        corners(&traced[0], 1),
        [[0.0; 3], [1.0, 1.0, 1.0], [2.0, 1.0, 1.0]]
    );
    assert_eq!(corners(&traced[0], 3), corners(&traced[0], 1));

    // A geometry that did not read and a triangle that names a vertex its geometry lacks are
    // counted, and a drawable that lacks a geometry the selection names is not its drawable.
    let mut cut = fan(0, 3).expect("a geometry");
    cut.triangles[1] = [0, 2, 9];
    let unread = Err(HeaderlessError::PhysicalNotRead {
        what: "geometry",
        at: 0,
    });
    let partly = drawable(vec![
        vec![fan(1, 2), fan(3, 5), Ok(cut)],
        vec![],
        vec![fan(3, 1)],
        vec![unread],
    ]);
    let traced = triangles(&selection, &partly).expect("the drawable the words are of");
    let counts: Vec<_> = (traced.iter())
        .map(|t| (t.triangles.len(), t.unread, t.left_out))
        .collect();
    assert_eq!(counts, [(6, 0, 1), (0, 1, 0)]);
    let other = drawable(vec![vec![fan(1, 2)], vec![], vec![], vec![fan(0, 1)]]);
    assert_eq!(triangles(&selection, &other), None);
    assert_eq!(triangles(&selection, &drawable(Vec::new())), None);
}
