use or_io::gta6::headerless::{
    Geometry, HeaderlessError, Model, Palette, Parameter, ParameterValue, Shader, TintPalette,
    Vertices,
};

use super::*;

const DIFFUSE: u32 = or_io::gta6::hash::name_hash32(b"DiffuseTex");

/// A triangle drawn with shader `shader`; its positions refused where `read` is false.
fn geometry(shader: u16, read: bool) -> Geometry {
    Geometry {
        shader,
        vertex_count: 3,
        channels: Vec::new(),
        triangles: vec![[0, 1, 2]],
        positions: match read {
            true => Ok(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
            false => Err(HeaderlessError::Positions {
                why: "the decoded vertices do not reach the geometry's box",
            }),
        },
        vertices: Vertices::default(),
        vertex_data: Vec::new(),
        index_data: Vec::new(),
    }
}

fn level(geometries: Vec<Geometry>) -> Option<Vec<Model>> {
    Some(vec![Model {
        geometries: geometries.into_iter().map(Ok).collect(),
        palette: Ok(Palette::Own),
        matrix_index: 0,
    }])
}

fn drawable(
    shaders: Vec<Shader>,
    levels: [Option<Vec<Model>>; 4],
    distances: [f32; 4],
) -> Drawable {
    Drawable {
        name: "box.#dr".to_string(),
        box_min: [0.0; 3],
        box_max: [1.0, 1.0, 0.0],
        shaders,
        levels,
        lod_distances: distances,
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    }
}

/// A shader whose texture records state `records`, each with its key, then `constants`.
fn shader(records: &[(u32, Option<u64>)], constants: &[(u32, f32)]) -> Shader {
    let textures = (records.iter().zip(0u8..)).map(|((hash, _), slot)| Parameter {
        hash: *hash,
        value: ParameterValue::Texture { slot },
    });
    let constants = constants.iter().map(|(hash, value)| Parameter {
        hash: *hash,
        value: ParameterValue::Constant {
            buffer: 0,
            offset: 0,
            words: vec![value.to_bits()],
        },
    });
    Shader {
        name_hash: 1,
        texture_keys: records.iter().map(|(_, key)| *key).collect(),
        parameters: textures.chain(constants).collect(),
    }
}

fn plain() -> Shader {
    shader(&[], &[])
}

const FAR: f32 = 9998.0;

#[test]
fn a_model_holds_every_level_its_body_does_each_drawn_to_its_distance_and_the_last_for_ever() {
    let levels = [
        level(vec![geometry(0, true), geometry(0, true)]),
        level(vec![geometry(0, true)]),
        level(vec![geometry(0, true)]),
        None,
    ];
    let model = model_of(
        &drawable(vec![plain()], levels, [25.0, 80.0, FAR, FAR]),
        &[],
        &|_| None,
    );
    let read: Vec<(f32, usize)> = (model.lods.iter())
        .map(|lod| (lod.until, lod.meshes.len()))
        .collect();
    assert_eq!(read, [(25.0, 2), (80.0, 1), (f32::INFINITY, 1)]);
    assert_eq!(model.lods[0].meshes[0].indices, [0, 1, 2]);
    // Four triangles of three vertices and three indices each.
    let vertex = size_of::<MeshVertex>() as u64;
    assert_eq!(model.bytes(), 4 * (3 * vertex + 3 * 4));

    // One level alone is drawn for ever, whatever distance its body states for it.
    let one = [level(vec![geometry(0, true)]), None, None, None];
    let model = model_of(
        &drawable(vec![plain()], one, [40.0, FAR, FAR, FAR]),
        &[],
        &|_| None,
    );
    assert_eq!(model.lods.len(), 1);
    assert_eq!(model.lods[0].until, f32::INFINITY);
}

#[test]
fn a_level_none_of_whose_geometries_read_is_left_out_and_a_model_with_none_keeps_one_empty_level() {
    let levels = [
        level(vec![geometry(0, true)]),
        level(vec![geometry(0, false)]),
        level(vec![geometry(0, true)]),
        None,
    ];
    let model = model_of(
        &drawable(vec![plain()], levels, [25.0, 80.0, FAR, FAR]),
        &[],
        &|_| None,
    );
    let until: Vec<f32> = model.lods.iter().map(|lod| lod.until).collect();
    assert_eq!(until, [25.0, f32::INFINITY]);

    let none = [level(vec![geometry(0, false)]), None, None, None];
    let model = model_of(&drawable(vec![plain()], none, [FAR; 4]), &[], &|_| None);
    assert_eq!(model.lods.len(), 1);
    assert!(model.lods[0].meshes.is_empty());
    assert_eq!(model.lods[0].until, f32::INFINITY);
    assert_eq!(model.bytes(), 0);
}

#[test]
fn a_mesh_takes_the_maps_and_the_bucket_of_its_own_shader_past_a_geometry_that_was_refused() {
    let shaders = vec![
        shader(&[(DIFFUSE, Some(0xA))], &[]),
        shader(&[(DIFFUSE, Some(0xB))], &[]),
        shader(&[(DIFFUSE, Some(0xC))], &[]),
    ];
    let levels = [
        level(vec![
            geometry(0, true),
            geometry(1, false),
            geometry(2, true),
        ]),
        None,
        None,
        None,
    ];
    let model = model_of(&drawable(shaders, levels, [FAR; 4]), &[0, 2, 3], &|_| None);
    let drawn: Vec<(Option<u64>, Bucket)> = (model.lods[0].meshes.iter())
        .map(|mesh| (mesh.material.layers[0].colour, mesh.bucket))
        .collect();
    assert_eq!(
        drawn,
        [(Some(0xA), Bucket::Opaque), (Some(0xC), Bucket::Cutout)]
    );
}

#[test]
fn a_four_layer_mesh_takes_its_weights_from_colour_channel_25_and_every_mesh_its_second_coordinate()
{
    let four = or_io::gta6::hash::name_hash32(b"DiffuseTexture_layer0");
    let shaders = vec![
        shader(&[(DIFFUSE, Some(0xA))], &[]),
        shader(&[(four, Some(0xB))], &[]),
    ];
    let painted = |shader: u16| {
        let mut geometry = geometry(shader, true);
        geometry.vertices.colours = vec![
            (24, vec![[1.0, 0.0, 0.0, 1.0]; 3]),
            (25, vec![[0.0, 0.0, 1.0, 1.0]; 3]),
        ];
        geometry.vertices.uvs = vec![(28, vec![[0.0, 0.0]; 3]), (29, vec![[0.5, 0.25]; 3])];
        geometry
    };
    let levels = [level(vec![painted(0), painted(1)]), None, None, None];
    let model = model_of(&drawable(shaders, levels, [FAR; 4]), &[0, 0], &|_| None);
    let first = |mesh: usize| {
        model.lods[0].meshes[mesh].vertices[0].colour[2]
            .to_bits()
            .to_le_bytes()
    };
    // The one-layer mesh's first colour is channel 24, the four-layer mesh's channel 25
    assert_eq!(first(0), [255, 0, 0, 255]);
    assert_eq!(first(1), [0, 0, 255, 255]);
    assert_eq!(model.lods[0].meshes[1].vertices[0].colour[..2], [0.5, 0.25]);
}

#[test]
fn each_palette_key_takes_the_models_own_palette_in_the_order_its_shaders_name_them() {
    let palette = or_io::gta6::hash::name_hash32(b"TintPaletteTex");
    let shaders = vec![
        shader(&[(DIFFUSE, Some(0xA)), (palette, Some(0x1))], &[]),
        shader(&[(palette, Some(0x1))], &[]),
        shader(&[(palette, Some(0x2))], &[]),
    ];
    let model = drawable(shaders, [None, None, None, None], [FAR; 4]);
    let row = |colour: u8| TintPalette {
        at: 0,
        colours: vec![[colour, 0, 0, 255]],
    };
    let two = palettes_of(&model, &[row(10), row(20)]);
    assert_eq!(
        two,
        [(0x1, vec![[10, 0, 0, 255]]), (0x2, vec![[20, 0, 0, 255]])]
    );
    // One palette serves every key; none, no key
    let one = palettes_of(&model, &[row(10)]);
    assert_eq!(
        one.iter().map(|(_, c)| c[0][0]).collect::<Vec<_>>(),
        [10, 10]
    );
    assert!(palettes_of(&model, &[]).is_empty());
}

#[test]
fn a_far_stand_in_of_a_plant_draws_no_mesh() {
    let levels = [level(vec![geometry(0, true)]), None, None, None];
    let mut stand_in = drawable(vec![plain()], levels, [FAR; 4]);
    stand_in.name = "veg_tree_oak_01_imposter.#dr".to_string();
    let model = model_of(&stand_in, &[0], &|_| None);
    assert_eq!(model.lods.len(), 1);
    assert!(model.lods[0].meshes.is_empty());
    // The same drawable under another name draws its mesh
    stand_in.name = "veg_tree_oak_01.#dr".to_string();
    assert_eq!(model_of(&stand_in, &[0], &|_| None).lods[0].meshes.len(), 1);
}

#[test]
fn the_bucket_is_the_draw_bucket_byte_under_its_batch_bit_and_a_water_mask_draws_nothing() {
    let of = |byte: u8| Bucket::of(byte, false);
    assert_eq!(of(0), Some(Bucket::Opaque));
    assert_eq!(of(1), Some(Bucket::Blend));
    assert_eq!(of(2), Some(Bucket::Decal));
    assert_eq!(of(3), Some(Bucket::Cutout));
    assert_eq!((of(4), of(5), of(6)), (None, None, None));
    for byte in 0..8 {
        assert_eq!(of(0x80 | byte), of(byte), "{byte}");
    }

    let glow = |strength: f32| shader(&[], &[(GLOW, strength)]);
    let levels = || {
        let geometries = (0..6).map(|shader| geometry(shader, true)).collect();
        [level(geometries), None, None, None]
    };
    let water = shader(&[(DIFFUSE, Some(0xA))], &[]);
    let shaders = vec![glow(5.0), glow(0.0), glow(5.0), glow(5.0), plain(), water];
    let model = model_of(
        &drawable(shaders, levels(), [FAR; 4]),
        &[0x80, 0, 3, 1, 4, 6],
        &|_| None,
    );
    let buckets: Vec<Bucket> = model.lods[0].meshes.iter().map(|m| m.bucket).collect();
    // A glow makes an opaque material emissive and leaves a cutout or blended one as it is; the masks are gone.
    assert_eq!(
        buckets,
        [
            Bucket::Emissive,
            Bucket::Opaque,
            Bucket::Cutout,
            Bucket::Blend
        ]
    );
    // Each mesh carries its own shader's material, classed by its byte.
    let families: Vec<Family> = (model.lods[0].meshes.iter())
        .map(|m| m.material.family)
        .collect();
    assert_eq!(
        families,
        [Family::Other, Family::Other, Family::Other, Family::Glass]
    );
    // Shaders past the bytes read are opaque.
    let model = model_of(
        &drawable(vec![plain(); 6], levels(), [FAR; 4]),
        &[],
        &|_| None,
    );
    assert!(model.lods[0]
        .meshes
        .iter()
        .all(|m| m.bucket == Bucket::Opaque));
    assert_eq!(model.lods[0].meshes.len(), 6);
}
