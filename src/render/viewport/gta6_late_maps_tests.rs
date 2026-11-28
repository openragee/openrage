//! A GTA VI drawable drawn by its own passes while its maps are still decoded.

use std::collections::BTreeMap;
use std::sync::Arc;

use or_renderer::game_shaders::meshlets::{Feed, MeshletSurface};

use super::*;
use crate::render::gta6_meshlets::Meshlets;

/// A stage shaped as a lowered mesh program is: one triangle about the middle of the view.
const MESH: &str = "
@group(0) @binding(0) var<uniform> view: mat4x4<f32>;

@vertex fn main(@builtin(vertex_index) v: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2(-0.3, -0.3), vec2(0.3, -0.3), vec2(0.0, 0.3));
    if v >= 3u {
        return vec4<f32>(0.0);
    }
    return view * vec4<f32>(corners[v], 0.0, 1.0);
}";

/// A stage that writes the four targets a frame is lit from, the first by its map.
const PIXEL: &str = "
@group(1) @binding(0) var map: texture_2d<f32>;

struct Targets {
    @location(0) colour: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) terms: vec4<f32>,
}

@fragment fn main() -> Targets {
    let texel = textureLoad(map, vec2<i32>(0), 0);
    return Targets(texel, vec4<f32>(0.5), vec4<f32>(0.0), vec4<f32>(0.0));
}";

#[test]
#[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
fn a_surface_whose_pass_is_built_draws_by_it_while_its_map_is_still_decoded() {
    const WHAT: &str = "a_surface_whose_pass_is_built_draws_by_it_while_its_map_is_still_decoded";
    let Some((device, queue)) = crate::render::test_device(WHAT) else {
        return;
    };
    let (device, queue) = (Arc::new(device), Arc::new(queue));
    let mut bridge = RenderBridge::new(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
    // Nothing a driver compiled is read from, or kept in, the configuration's folder.
    bridge.gta6_pipelines = Default::default();
    bridge.renderer.resize(&device, 64, 64);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let stage =
        |text: &str| Arc::new(wgpu::naga::front::wgsl::parse_str(text).expect("the stage parses"));
    let view = Feed::View {
        bytes: vec![0; 64],
        view_proj: Some(0),
    };
    // Its map is not in yet: the heap's entry is empty, which binds the blank texel.
    let surface = MeshletSurface {
        vertex: stage(MESH),
        fragment: stage(PIXEL),
        feeds: BTreeMap::from([((0, 0), view), ((1, 0), Feed::Textures(vec![None]))]),
        meshlets: 1,
        primitives: 1,
        two_sided: false,
        forward: None,
    };
    let read = || {
        Some(Ok(Meshlets {
            surfaces: vec![surface.clone()],
            of: vec![(0, "decoding".to_string())],
            zeroed: 0,
            rest: Vec::new(),
            why: None,
            decoding: true,
        }))
    };
    for _ in 0..4000 {
        bridge.gta6_own = bridge.prepare_gta6_meshlets_of(read);
        if bridge.gta6_building.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let own = bridge.gta6_own.clone().expect("a drawable");
    let own = own.expect("drawn by its own pass while its map is decoded");
    assert_eq!((own.drawn, own.rest, own.why), (1, 0, None));
    let scene = bridge.renderer.gta6_scene().expect("a scene");
    assert_eq!((scene.drawn(), scene.waiting.len()), (1, 0));
    // It waits for the map, and was prepared once.
    assert!(bridge.gta6_decoding);
    assert_eq!(bridge.gta6_maps.prepares, 1);
    assert!(!bridge.gta6_maps.all_in, "a map is still decoded");
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}
