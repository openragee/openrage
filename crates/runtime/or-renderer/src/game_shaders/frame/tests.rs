use super::*;
use crate::game_shaders::bindings::Resource;
use crate::game_shaders::pipeline::{build_pass_pipeline_with, PassPipeline};
use crate::game_shaders::stand_in::{bind_groups_with, Provided};

fn wgsl(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the WGSL parses")
}

fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("frame-test"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()
}

/// What `body` made the device refuse, if anything.
fn refused<T>(device: &wgpu::Device, body: impl FnOnce() -> T) -> (T, Option<String>) {
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = body();
    let validation = pollster::block_on(validation.pop());
    let internal = pollster::block_on(internal.pop());
    (made, validation.or(internal).map(|e| e.to_string()))
}

const VERTEX: &str = "
@vertex fn main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(position, 1.0);
}";

const PIXEL: &str = "
@group(1) @binding(0) var<uniform> colour: vec4<f32>;
@fragment fn main() -> @location(0) vec4<f32> { return colour; }";

const SIZE: u32 = 8;

/// The pass of `PIXEL`, built to test depth or not.
fn pass(device: &wgpu::Device, pixel: &str, depth_test: bool) -> PassPipeline {
    build_pass_pipeline_with(device, &wgsl(VERTEX), Some(&wgsl(pixel)), depth_test)
        .expect("the pass builds")
}

fn groups(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pass: &PassPipeline,
    colour: [f32; 4],
) -> BTreeMap<u32, wgpu::BindGroup> {
    bind_groups_with(device, queue, pass, &mut |requirement| {
        matches!(requirement.resource, Resource::Uniform { .. })
            .then(|| Provided::Bytes(bytemuck::cast_slice(&colour).to_vec()))
    })
}

/// A quad that covers the target at depth `z`, as six positions of three floats.
fn quad(z: f32) -> Vec<u8> {
    let corners = [
        [-1.0f32, -1.0, z],
        [1.0, -1.0, z],
        [1.0, 1.0, z],
        [-1.0, -1.0, z],
        [1.0, 1.0, z],
        [-1.0, 1.0, z],
    ];
    bytemuck::cast_slice(&corners).to_vec()
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// The pixel in the middle of the frame.
fn middle(frame: &Frame, device: &wgpu::Device, queue: &wgpu::Queue) -> [u8; 4] {
    let rendered = frame.read_back(device, queue);
    let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    [
        rendered.bytes[at],
        rendered.bytes[at + 1],
        rendered.bytes[at + 2],
        rendered.bytes[at + 3],
    ]
}

/// `colour` quads drawn in order at the depths given, into a fresh frame.
fn drawn(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    depth_test: bool,
    order: &[(f32, [f32; 4])],
) -> [u8; 4] {
    let pass = pass(device, PIXEL, depth_test);
    let frame = Frame::new(device, SIZE);
    frame.clear(device, queue);
    for (z, colour) in order {
        let groups = groups(device, queue, &pass, *colour);
        frame.draw(device, queue, &pass, &groups, &quad(*z), 6);
    }
    middle(&frame, device, queue)
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_nearer_surface_wins_whichever_is_drawn_first_when_the_pass_tests_depth() {
    let Some((device, queue)) = gpu("a_nearer_surface_wins") else {
        return;
    };
    let (shown, error) = refused(&device, || {
        (
            drawn(&device, &queue, true, &[(0.2, RED), (0.8, GREEN)]),
            drawn(&device, &queue, true, &[(0.8, GREEN), (0.2, RED)]),
        )
    });
    assert_eq!(error, None);
    assert_eq!(shown.0, [255, 0, 0, 255], "the near one drawn first");
    assert_eq!(shown.1, [255, 0, 0, 255], "the near one drawn last");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn without_a_depth_test_the_last_draw_wins() {
    let Some((device, queue)) = gpu("without_a_depth_test") else {
        return;
    };
    let shown = drawn(&device, &queue, false, &[(0.2, RED), (0.8, GREEN)]);
    assert_eq!(shown, [0, 255, 0, 255]);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn clearing_the_frame_empties_its_colour_and_resets_its_depth() {
    let Some((device, queue)) = gpu("clearing_the_frame") else {
        return;
    };
    let pass = pass(&device, PIXEL, true);
    let frame = Frame::new(&device, SIZE);
    assert_eq!(frame.size(), SIZE);
    frame.clear(&device, &queue);
    assert_eq!(frame.read_back(&device, &queue).touched(), 0);
    let near = groups(&device, &queue, &pass, RED);
    frame.draw(&device, &queue, &pass, &near, &quad(0.2), 6);
    assert_eq!(
        frame.read_back(&device, &queue).touched(),
        (SIZE * SIZE) as usize
    );
    // Cleared again, a farther surface shows where a nearer one had been
    frame.clear(&device, &queue);
    assert_eq!(frame.read_back(&device, &queue).touched(), 0);
    let far = groups(&device, &queue, &pass, GREEN);
    frame.draw(&device, &queue, &pass, &far, &quad(0.8), 6);
    assert_eq!(middle(&frame, &device, &queue), [0, 255, 0, 255]);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_pass_with_more_than_one_target_draws_its_first_into_the_frame_and_throws_the_rest_away() {
    let Some((device, queue)) = gpu("a_pass_with_more_than_one_target") else {
        return;
    };
    let pixel = "struct Out { @location(0) a: vec4<f32>, @location(1) b: vec4<f32> }
        @group(1) @binding(0) var<uniform> colour: vec4<f32>;
        @fragment fn main() -> Out { var o: Out; o.a = colour; o.b = vec4<f32>(0.0, 0.0, 1.0, 1.0); return o; }";
    let pass = pass(&device, pixel, true);
    assert_eq!(pass.colour_formats.len(), 2);
    let frame = Frame::new(&device, SIZE);
    frame.clear(&device, &queue);
    let (_, error) = refused(&device, || {
        let groups = groups(&device, &queue, &pass, RED);
        frame.draw(&device, &queue, &pass, &groups, &quad(0.5), 6);
    });
    assert_eq!(error, None);
    assert_eq!(middle(&frame, &device, &queue), [255, 0, 0, 255]);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_pass_whose_first_target_is_another_format_leaves_the_frames_colour_alone() {
    let Some((device, queue)) = gpu("a_pass_whose_first_target_is_another_format") else {
        return;
    };
    let pixel = "@group(1) @binding(0) var<uniform> colour: vec4<f32>;
        @fragment fn main() -> @location(0) f32 { return colour.x; }";
    let pass = pass(&device, pixel, true);
    assert_eq!(
        pass.colour_formats,
        vec![Some(wgpu::TextureFormat::R8Unorm)]
    );
    let frame = Frame::new(&device, SIZE);
    frame.clear(&device, &queue);
    let (_, error) = refused(&device, || {
        let groups = groups(&device, &queue, &pass, RED);
        frame.draw(&device, &queue, &pass, &groups, &quad(0.5), 6);
    });
    assert_eq!(error, None);
    assert_eq!(frame.read_back(&device, &queue).touched(), 0);
}
