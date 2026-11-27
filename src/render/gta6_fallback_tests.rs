use std::collections::HashMap;

use or_io::gta5::shaders::sgd5_pack::pack_effect;
use or_io::gta5::shaders::spirv_pack::{Pass, Production, ShaderStageKind, SpirvPack};
use or_io::gta6::headerless::{
    Drawable, Geometry, Model, Palette, Parameter, ParameterValue, Resource, Shader, Vertices,
};
use or_renderer::game_shaders::pack::GamePack;

use super::*;
use crate::state::{Document, Prefs};

/// A state with a drawable open of one geometry, whose shader sets a constant and names a map.
fn opened() -> AppState {
    let geometry = Geometry {
        vertex_data: Vec::new(),
        index_data: Vec::new(),
        shader: 0,
        vertex_count: 3,
        channels: Vec::new(),
        triangles: vec![[0, 1, 2]],
        positions: Ok(vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        vertices: Vertices::default(),
    };
    let shader = Shader {
        name_hash: 0x0BAD_F00D,
        texture_keys: vec![Some(0x5EED_0000_0000_0001)],
        parameters: vec![
            Parameter {
                hash: 0x11,
                value: ParameterValue::Constant {
                    buffer: 0,
                    offset: 0,
                    words: vec![1.0f32.to_bits(); 4],
                },
            },
            Parameter {
                hash: 0x22,
                value: ParameterValue::Texture { slot: 0 },
            },
        ],
    };
    let drawable = Drawable {
        name: "a_building".to_string(),
        box_min: [0.0; 3],
        box_max: [1.0; 3],
        shaders: vec![shader],
        levels: [
            Some(vec![Model {
                geometries: vec![Ok(geometry)],
                palette: Ok(Palette::Own),
                matrix_index: 0,
            }]),
            None,
            None,
            None,
        ],
        lod_distances: [10.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    };
    let mut state = AppState::from_prefs(Prefs::default());
    let payload = DocPayload::Gta6Drawable(std::sync::Arc::new(Resource::Drawable(drawable)));
    (state.documents).push(Document::new(1, Some("0x10.drawable".into()), payload));
    state.active_document = Some(1);
    state
}

#[test]
fn a_geometry_the_pack_draws_nothing_of_is_left_out_before_its_material_is_made() {
    let state = opened();
    let made = std::cell::Cell::new(0);
    let refused = |_: &str| {
        made.set(made.get() + 1);
        Err("draws nothing".to_string())
    };
    let left = shaded_surfaces(&state, &refused).expect("a drawable is open");
    assert!(left.surfaces.is_empty() && left.decoding.is_empty());
    let effect = effect_name(0x0BAD_F00D);
    assert_eq!(left.left_out, [(effect, "draws nothing".to_string())]);
    assert_eq!(made.get(), 1, "asked once a shader");

    let drawn = shaded_surfaces(&state, &|_| Ok(())).expect("a drawable is open");
    assert!(drawn.left_out.is_empty());
    let [(_, material)] = drawn.surfaces.as_slice() else {
        panic!("one surface");
    };
    assert_eq!(material.vectors.len(), 1, "its constant");
    // No pack the app has read holds the map: nothing decodes it, and nothing waits for it.
    assert!(material.textures.is_empty() && drawn.decoding.is_empty());
    assert!(crate::render::gta6_meshlets::all_in(&[
        0x5EED_0000_0000_0001
    ]));
}

#[test]
fn a_pack_draws_an_effect_only_where_a_technique_of_it_has_a_pass() {
    let stages = [(ShaderStageKind::Vertex, "v".to_string())].into();
    let mut manifest = SpirvPack::new(Production {
        translator: String::new(),
        legalizer: String::new(),
    });
    manifest.effects = vec![
        pack_effect("Own_Only", &[]),
        pack_effect("drawn", &[(Pass { stages }, false)]),
    ];
    let pack = GamePack::in_memory(manifest, HashMap::new());
    assert_eq!(
        pack_draws(&pack, "DRAWN"),
        Ok(()),
        "found as a scene finds it"
    );
    let none = pack_draws(&pack, "own_only").expect_err("no pass");
    assert!(none.contains("own_only"), "{none}");
    let absent = pack_draws(&pack, "elsewhere").expect_err("no effect");
    assert_ne!(none, absent);
}
