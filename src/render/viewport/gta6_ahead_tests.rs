//! A GTA VI drawable drawn while a worker still builds a pass of its own programs.

use std::collections::BTreeMap;

use or_renderer::game_shaders::meshlets::{Feed, MeshletSurface};
use or_renderer::{MeshDraw, MeshVertex, OrbitCamera};

use super::*;
use crate::render::gta6_meshlets::Meshlets;
use crate::render::gta6_shaded::Own;

/// A stage shaped as a lowered mesh program is: no vertex buffer, three corners a thread and an
/// instance a meshlet. One triangle on the ground about the place its constant states.
const MESH: &str = "
@group(0) @binding(0) var<uniform> view: mat4x4<f32>;
@group(0) @binding(1) var<uniform> about: vec4<f32>;

@vertex fn main(@builtin(vertex_index) v: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2(-0.3, -0.3), vec2(0.3, -0.3), vec2(0.0, 0.3));
    if v >= 3u {
        return vec4<f32>(0.0);
    }
    return view * vec4<f32>(corners[v] + about.xy, 0.0, 1.0);
}";

/// A stage that writes the four targets a frame is lit from: red, and a normal along +Z packed
/// as two 12-bit numbers over three bytes.
const PIXEL: &str = "
struct Targets {
    @location(0) colour: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) terms: vec4<f32>,
}

@fragment fn main() -> Targets {
    let packed = 2047u | (2047u << 12u);
    let bytes = vec3<f32>(f32(packed & 255u), f32((packed >> 8u) & 255u), f32(packed >> 16u));
    return Targets(vec4<f32>(1.0, 0.0, 0.0, 1.0), vec4<f32>(0.5, bytes / 255.0), vec4<f32>(0.0), vec4<f32>(0.0));
}";

/// A surface of one meshlet about (`x`, 0) on the ground, drawn by a pass of its own.
fn surface(x: f32) -> MeshletSurface {
    let stage =
        |text: &str| Arc::new(wgpu::naga::front::wgsl::parse_str(text).expect("the stage parses"));
    let view = Feed::View {
        bytes: vec![0; 64],
        view_proj: Some(0),
    };
    let about = Feed::Bytes(bytemuck::cast_slice(&[x, 0.0, 0.0, 0.0]).to_vec());
    MeshletSurface {
        vertex: stage(MESH),
        fragment: stage(PIXEL),
        feeds: BTreeMap::from([((0, 0), view), ((0, 1), about)]),
        meshlets: 1,
        primitives: 1,
    }
}

/// The triangle [`surface`] draws about (`x`, 0), as a mesh Material draws seen from either side.
fn mesh(device: &wgpu::Device, x: f32) -> MeshDraw {
    use wgpu::util::DeviceExt;
    let corners = [[x - 0.3, -0.3, 0.0], [x + 0.3, -0.3, 0.0], [x, 0.3, 0.0]];
    let vertices = corners.map(|at| MeshVertex::new(at, [0.0, 0.0, 1.0], [0.0, 0.0]));
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    MeshDraw {
        bump: false,
        vertex_buffer: buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX),
        index_buffer: buffer(
            bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 1]),
            wgpu::BufferUsages::INDEX,
        ),
        index_count: 6,
        texture_bind_group: None,
    }
}

/// The pixels a side of the frame a bridge draws.
const PX: u32 = 128;

/// A bridge that draws frames of [`PX`] a side, with its device; none with no device.
fn bridged(what: &str) -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>, RenderBridge)> {
    let (device, queue) = crate::render::test_device(what)?;
    let (device, queue) = (Arc::new(device), Arc::new(queue));
    let mut bridge = RenderBridge::new(
        device.clone(),
        queue.clone(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
    // Nothing a driver compiled is read from, or kept in, the configuration's folder.
    bridge.gta6_pipelines = Default::default();
    bridge.renderer.resize(&device, PX, PX);
    Some((device, queue, bridge))
}

/// The pixel of `frame` that shows the middle of the triangle about (`x`, 0) through `camera`.
fn seen_at(frame: &image::RgbaImage, camera: &OrbitCamera, x: f32) -> [u8; 4] {
    let at = (camera.view_proj(1.0)).project_point3(glam::Vec3::new(x, -0.1, 0.0));
    let side = PX as f32;
    frame
        .get_pixel(
            ((at.x + 1.0) / 2.0 * side) as u32,
            ((1.0 - at.y) / 2.0 * side) as u32,
        )
        .0
}

/// Whether a pixel is the red a surface's own pass lights it.
fn red(pixel: [u8; 4]) -> bool {
    pixel[0] > 100 && pixel[1] < pixel[0] / 2 && pixel[2] < pixel[0] / 2
}

/// A pixel stage that writes three targets, where a frame is lit from the four of [`PIXEL`].
const THREE: &str = "
struct Targets {
    @location(0) colour: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
}

@fragment fn main() -> Targets {
    return Targets(vec4<f32>(1.0, 0.0, 0.0, 1.0), vec4<f32>(0.5), vec4<f32>(0.0));
}";

#[test]
#[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
fn a_drawable_some_passes_drew_and_none_can_once_all_are_built_ends_as_one_they_never_drew() {
    const WHAT: &str = "a_drawable_some_passes_drew_and_none_can_once_all_are_built";
    let Some((device, queue, mut bridge)) = bridged(WHAT) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // The pass held back writes other targets than the one ahead: no scene draws both, which
    // is known only once the held one is built.
    let (held, ahead) = (-0.8, 0.8);
    let mut other = surface(held);
    other.fragment = Arc::new(wgpu::naga::front::wgsl::parse_str(THREE).expect("the stage parses"));
    let surfaces = [other, surface(ahead)];
    bridge
        .renderer
        .set_meshes(vec![mesh(&device, held), mesh(&device, ahead)]);
    let read = || {
        Some(Ok(Meshlets {
            surfaces: surfaces.to_vec(),
            of: vec![(0, "held".to_string()), (1, "ahead".to_string())],
            zeroed: 0,
            rest: Vec::new(),
            why: None,
            decoding: false,
        }))
    };
    // What the pack's passes draw of it, as a report keeps it, and the key its scene was made by.
    let kept = (3, vec![("held".to_string(), "kept".to_string())]);
    let key = MeshCacheKey::of(&crate::state::AppState::default(), 7, 0);
    bridge.game_scene_key = Some(key);

    bridge.gta6_pipelines.hold(&surfaces[0], true);
    for _ in 0..4000 {
        if matches!(bridge.gta6_own, Some(Ok(_))) {
            break;
        }
        // Its own passes draw it: the pack's draw none of it.
        let drawn = bridge.ask_again(read, kept.clone());
        assert!(drawn == kept || drawn == (0, Vec::new()));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let own = bridge.gta6_own.clone().expect("a drawable").expect("drawn");
    assert_eq!((own.drawn, own.rest), (1, 1));
    assert_eq!(bridge.game_scene_key, Some(key));

    // Once the other pass is built the two cannot be one scene: the drawable is drawn by none
    // of its passes, and the next frame prepares it from nothing.
    bridge.gta6_pipelines.hold(&surfaces[0], false);
    let mut drawn = (0, Vec::new());
    for _ in 0..4000 {
        if bridge.gta6_building.is_none() {
            break;
        }
        drawn = bridge.ask_again(read, kept.clone());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let shapes = "two pixel programs write targets of different shapes";
    assert_eq!(bridge.gta6_own, Some(Err(shapes.to_string())));
    assert_eq!(drawn, kept);
    assert_eq!(bridge.game_scene_key, None);
    assert!(bridge.renderer.gta6_scene().is_none() && bridge.gta6_building.is_none());
    // As that frame asks, it is the drawable every pass of which was built before it was
    // prepared: the same reason, and no scene of its own.
    assert_eq!(
        bridge.prepare_gta6_meshlets_of(read),
        Some(Err(shapes.to_string()))
    );
    assert!(bridge.renderer.gta6_scene().is_none() && bridge.gta6_building.is_none());
    let camera = OrbitCamera::default();
    let mut encoder = device.create_command_encoder(&Default::default());
    (bridge.renderer).render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
    queue.submit([encoder.finish()]);
    let texture = bridge.renderer.colour_texture();
    let frame = crate::render::read_back::rgba(&device, &queue, texture, PX, PX).expect("a frame");
    let (left, right) = (
        seen_at(&frame, &camera, held),
        seen_at(&frame, &camera, ahead),
    );
    assert!(!red(left) && !red(right), "{left:?} {right:?}");
    assert!(left != seen_at(&frame, &camera, 0.0), "no mesh is drawn");
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}

#[test]
#[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
fn a_surface_draws_by_its_own_pass_and_the_one_whose_pass_is_still_built_as_material() {
    const WHAT: &str = "a_surface_draws_by_its_own_pass_and_the_one_whose_pass_is_still_built";
    let Some((device, queue, mut bridge)) = bridged(WHAT) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);

    // Two surfaces side by side, each the mesh of its number in the model view.
    let (held, ahead) = (-0.8, 0.8);
    let surfaces = [surface(held), surface(ahead)];
    bridge
        .renderer
        .set_meshes(vec![mesh(&device, held), mesh(&device, ahead)]);
    let read = || {
        Some(Ok(Meshlets {
            surfaces: surfaces.to_vec(),
            of: vec![(0, "held".to_string()), (1, "ahead".to_string())],
            zeroed: 0,
            rest: Vec::new(),
            why: None,
            decoding: false,
        }))
    };
    let camera = OrbitCamera::default();
    // The frame as its pixels, and the pixel that shows the middle of the triangle about `x`.
    let frame = |bridge: &mut RenderBridge| {
        let mut encoder = device.create_command_encoder(&Default::default());
        (bridge.renderer).render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
        queue.submit([encoder.finish()]);
        let texture = bridge.renderer.colour_texture();
        crate::render::read_back::rgba(&device, &queue, texture, PX, PX).expect("a frame")
    };
    let seen = |frame: &image::RgbaImage, x: f32| seen_at(frame, &camera, x);
    // Ask as a frame of the app does, until `done` or for twenty seconds.
    let ask = |bridge: &mut RenderBridge, done: &dyn Fn(&RenderBridge) -> bool| {
        for _ in 0..4000 {
            bridge.gta6_own = bridge.prepare_gta6_meshlets_of(read);
            if done(bridge) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the drawable never came to it: {:?}", bridge.gta6_own);
    };

    bridge.gta6_pipelines.hold(&surfaces[0], true);
    ask(&mut bridge, &|bridge| {
        matches!(bridge.gta6_own, Some(Ok(_)))
    });
    let building = "held: a worker is still building its passes for this device";
    let one = Own {
        drawn: 1,
        zeroed: 0,
        rest: 1,
        why: Some(building.to_string()),
        unshadowed: false,
    };
    assert_eq!(bridge.gta6_own, Some(Ok(one.clone())));
    assert!(bridge.gta6_building.is_some(), "it is asked about again");
    let scene = bridge.renderer.gta6_scene().expect("a scene");
    assert_eq!((scene.drawn(), scene.rest.as_slice()), (1, &[0][..]));
    let partly = frame(&mut bridge);
    // Asked again with no pass built since, it is the scene that drew that frame still.
    let drew = bridge.renderer.gta6_scene().expect("a scene").calls();
    assert!(drew.surface_draws > 0);
    bridge.gta6_own = bridge.prepare_gta6_meshlets_of(read);
    assert_eq!(bridge.gta6_own, Some(Ok(one)));
    assert_eq!(bridge.renderer.gta6_scene().map(|s| s.calls()), Some(drew));

    bridge.gta6_pipelines.hold(&surfaces[0], false);
    ask(&mut bridge, &|bridge| bridge.gta6_building.is_none());
    let both = Own {
        drawn: 2,
        zeroed: 0,
        rest: 0,
        why: None,
        unshadowed: false,
    };
    assert_eq!(bridge.gta6_own, Some(Ok(both)));
    let scene = bridge.renderer.gta6_scene().expect("a scene");
    assert_eq!(
        (scene.drawn(), scene.rest.len(), scene.waiting.len()),
        (2, 0, 0)
    );
    let whole = frame(&mut bridge);

    // The surface ahead is lit red by its own pass in both frames; the held one is the plain
    // mesh while its pass is built, and red as the other once it is.
    let (own, material) = (seen(&partly, ahead), seen(&partly, held));
    assert!(red(own), "{own:?}");
    assert!(
        !red(material) && material != seen(&partly, 0.0),
        "{material:?}"
    );
    let (ahead, held) = (seen(&whole, ahead), seen(&whole, held));
    assert!(red(ahead) && red(held), "{ahead:?} {held:?}");
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}
