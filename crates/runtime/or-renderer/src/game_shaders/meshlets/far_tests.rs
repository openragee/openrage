//! A stage that reads far past the end of what it binds draws without a fault: a translated
//! program reads at places the game's buffers state, and the editor's noughts or a model's own
//! bytes may send one anywhere.

use super::tests::{surface, MESH, SIZE};
use super::*;

/// Places far past the end of anything a test binds: one whose offset in bytes fits a dword, one
/// whose offset wraps to the start, one with the sign bit and the last.
const FAR: [u32; 4] = [0x0001_0001, 0x1000_0001, 0x8000_0001, 0xFFFF_FFF1];

/// A device as the app opens one for a translated pass, on the adapter the environment picks.
fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::from_env().unwrap_or(descriptor.backends);
    let instance = wgpu::Instance::new(crate::instance::with_dxc(descriptor));
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    let mut required_limits = wgpu::Limits::default();
    let required_features =
        device_needs(adapter.features(), &adapter.limits(), &mut required_limits);
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("far-test"),
        required_features,
        required_limits,
        ..Default::default()
    }))
    .ok()
}

/// [`MESH`] with `declared` among its bindings and `read`, a float read at the place `far`, added
/// to what it shades by; `far` is the seventh dword of the meshlets, so that no compiler knows it.
fn far_mesh(declared: &str, read: &str) -> naga::Module {
    let shade = "out.shade = vec3<f32>(1.0, 0.5, 0.25)";
    let text = MESH
        .replace("struct Out {", &format!("{declared}\nstruct Out {{"))
        .replace(
            &format!("{shade};"),
            &format!("let far = meshlets.words[6];\n    {shade} + vec3<f32>({read});"),
        );
    assert!(text.contains("let far"), "the stage no longer has the line");
    naga::front::wgsl::parse_str(&text).expect("the WGSL parses")
}

/// Draws the targets of a surface whose mesh stage reads `read` of `declared` at `far`, and
/// waits for the device: how many surfaces drew, or why the device has no pass for it.
fn draws_far(
    (device, queue): &(wgpu::Device, wgpu::Queue),
    far: u32,
    declared: &str,
    read: &str,
) -> Result<usize, String> {
    let mut surface = surface();
    surface.vertex = Arc::new(far_mesh(declared, read));
    let mut meshlets: Vec<u32> = vec![0, 1, 2, 3, 4, 5, far];
    meshlets.resize(16, 0);
    surface.feeds.insert(
        (0, 1),
        Feed::Bytes(bytemuck::cast_slice(&meshlets).to_vec()),
    );
    let frame = Target {
        colour: wgpu::TextureFormat::Rgba8Unorm,
        depth: DEPTH,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: Some(1),
    };
    let mut pipelines = MeshletPipelines::default();
    let scene = loop {
        let surfaces = std::slice::from_ref(&surface);
        let size = (SIZE, SIZE);
        match MeshletScene::prepare(device, queue, &mut pipelines, surfaces, size, &frame)? {
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
            Some(scene) => break scene,
        }
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    scene.record_targets(&mut encoder);
    queue.submit([encoder.finish()]);
    let finished = device.poll(wgpu::PollType::wait_indefinitely());
    finished.map(|_| scene.drawn()).map_err(|e| e.to_string())
}

macro_rules! far {
    ($name:ident, $declared:expr, $read:expr) => {
        #[cfg_attr(
            not(feature = "gpu-tests"),
            ignore = "needs a GPU adapter: acquisition can hang, not just fail"
        )]
        #[test]
        fn $name() {
            let Some(gpu) = gpu(stringify!($name)) else {
                return;
            };
            for far in FAR {
                assert_eq!(draws_far(&gpu, far, $declared, $read), Ok(1), "{far:#X}");
            }
        }
    };
}

far!(
    a_constant_buffer_read_far_past_its_rows_draws,
    "@group(0) @binding(3) var<uniform> rows: array<vec4<f32>, 4096>;",
    "rows[far].x"
);
far!(
    a_buffer_read_far_past_its_dwords_draws,
    "",
    "f32(vertices.words[far])"
);
far!(
    a_texture_loaded_far_past_its_texels_and_levels_draws,
    "@group(0) @binding(3) var plain: texture_2d<f32>;",
    "textureLoad(plain, vec2<i32>(i32(far)), 0).x + textureLoad(plain, vec2<i32>(0), i32(far)).x"
);

/// Floats no texture has a texel or a level at: not a number, both infinities and a number past any size.
const WILD: [u32; 4] = [0x7FC0_0000, 0x7F80_0000, 0xFF80_0000, 0x7149_F2CA];

macro_rules! wild {
    ($name:ident, $declared:expr, $read:expr) => {
        #[cfg_attr(
            not(feature = "gpu-tests"),
            ignore = "needs a GPU adapter: acquisition can hang, not just fail"
        )]
        #[test]
        fn $name() {
            let Some(gpu) = gpu(stringify!($name)) else {
                return;
            };
            let read = format!("{}.x", $read.replace("wild", "bitcast<f32>(far)"));
            for wild in WILD {
                assert_eq!(draws_far(&gpu, wild, $declared, &read), Ok(1), "{wild:#X}");
            }
        }
    };
}

wild!(
    a_texture_sampled_at_a_place_and_a_level_that_are_no_numbers_draws,
    "@group(0) @binding(3) var flat: texture_2d<f32>;\n@group(0) @binding(4) var some: sampler;",
    "(textureSampleLevel(flat, some, vec2<f32>(wild), 0.0) + textureSampleLevel(flat, some, vec2<f32>(0.5), wild))"
);
wild!(
    a_row_sampled_at_a_place_and_a_level_that_are_no_numbers_draws,
    "@group(0) @binding(3) var row: texture_1d<f32>;\n@group(0) @binding(4) var some: sampler;",
    "(textureSampleLevel(row, some, wild, 0.0) + textureSampleLevel(row, some, 0.5, wild))"
);
wild!(
    a_volume_sampled_at_a_place_and_a_level_that_are_no_numbers_draws,
    "@group(0) @binding(3) var volume: texture_3d<f32>;\n@group(0) @binding(4) var some: sampler;",
    "(textureSampleLevel(volume, some, vec3<f32>(wild), 0.0) + textureSampleLevel(volume, some, vec3<f32>(0.5), wild))"
);
