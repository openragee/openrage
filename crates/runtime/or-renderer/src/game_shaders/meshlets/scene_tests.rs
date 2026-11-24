//! Frames of a model of two triangles, one over the other, lit onto a target and read back.

use super::tests::{surface, SIZE};
use super::*;
use crate::game_shaders::frame::read_back;

/// A floor across the whole frame at a depth of a tenth, and a small triangle over its middle
/// at six tenths: the camera looks straight down on both.
fn two_levels() -> MeshletSurface {
    let vertices: [[f32; 3]; 6] = [
        [-1.0, -1.0, 0.1],
        [3.0, -1.0, 0.1],
        [-1.0, 3.0, 0.1],
        [-0.3, -0.3, 0.6],
        [0.3, -0.3, 0.6],
        [0.0, 0.3, 0.6],
    ];
    let positions: Vec<u32> = vertices.iter().flatten().map(|v| v.to_bits()).collect();
    let mut surface = surface();
    (surface.feeds).insert(
        (0, 0),
        Feed::Bytes(bytemuck::cast_slice(&positions).to_vec()),
    );
    surface
}

/// The frame `sky` lights the two levels onto, as the bytes of its pixels; none with no device.
fn frame_of(test: &str, sky: Sky) -> Option<Vec<u8>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("meshlet-scene-test"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()?;
    let frame = Target {
        colour: wgpu::TextureFormat::Rgba8Unorm,
        depth: DEPTH,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: Some(1),
    };
    let size = (SIZE, SIZE);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pipelines = MeshletPipelines::default();
    let surfaces = [two_levels()];
    let scene = loop {
        let scene = MeshletScene::prepare(&device, &queue, &mut pipelines, &surfaces, size, &frame);
        match scene.expect("the scene prepares") {
            Some(scene) => break scene,
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    };
    let colour = render_target(
        &device,
        "frame",
        size,
        frame.colour,
        wgpu::TextureUsages::COPY_SRC,
    );
    let depth = render_target(&device, "depth", size, DEPTH, wgpu::TextureUsages::empty());
    let lighting = Lighting {
        view_proj: Mat4::IDENTITY,
        sky,
        camera: Vec3::new(0.0, 0.0, 4.0),
    };
    scene.update(&queue, &lighting);
    let mut encoder = device.create_command_encoder(&Default::default());
    scene.record_targets(&mut encoder);
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &colour,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        scene.record(&mut pass);
    }
    queue.submit([encoder.finish()]);
    let read = read_back(&device, &queue, colour.texture(), frame.colour, SIZE);
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
    Some(read.bytes)
}

/// The pixel of `frame` that shows the place (`x`, `y`) of the clip space.
fn seen_at(frame: &[u8], x: f32, y: f32) -> [u8; 4] {
    let (px, py) = (
        ((x + 1.0) / 2.0 * SIZE as f32) as usize,
        ((1.0 - y) / 2.0 * SIZE as f32) as usize,
    );
    let at = (py * SIZE as usize + px) * 4;
    [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
}

/// A white light that leans towards +x, and nothing from the sky.
fn leaning() -> Sky {
    Sky {
        towards: Vec3::new(0.6, 0.0, 0.8),
        sun: Vec3::splat(std::f32::consts::PI),
        sky: Vec3::ZERO,
        ground: Vec3::ZERO,
        exposure: 1.0,
        tone: None,
        scene: Scene::default(),
    }
}

/// The box the two levels fill.
const BOX: (Vec3, Vec3) = (Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 1.0));

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_upper_triangle_shadows_the_floor_on_the_side_away_from_the_light() {
    const WHAT: &str = "the_upper_triangle_shadows_the_floor";
    // The light leans towards +x, so the shadow falls three eighths of a unit towards -x: on
    // the floor beside the triangle, where the camera sees it.
    let (shadowed, open) = ((-0.5, -0.2), (0.6, -0.2));
    let Some(plain) = frame_of(WHAT, leaning()) else {
        return;
    };
    // With no box stated nothing is cast: the floor is lit alike on both sides.
    assert_eq!(
        seen_at(&plain, shadowed.0, shadowed.1),
        seen_at(&plain, open.0, open.1)
    );
    let mut sky = leaning();
    sky.scene.bounds = Some(BOX);
    let cast = frame_of(WHAT, sky).expect("a device a moment ago");
    let (dark, lit) = (
        seen_at(&cast, shadowed.0, shadowed.1),
        seen_at(&cast, open.0, open.1),
    );
    assert_eq!(
        lit,
        seen_at(&plain, open.0, open.1),
        "the open floor is as it was"
    );
    assert!(lit[0] > 100 && dark[0] < lit[0] / 4, "{dark:?} {lit:?}");
    // The triangle itself faces the light and nothing stands before it.
    assert_eq!(seen_at(&cast, 0.0, -0.1), seen_at(&plain, 0.0, -0.1));
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_clouds_dim_the_directional_light_by_the_share_they_let_through() {
    const WHAT: &str = "the_clouds_dim_the_directional_light";
    // A light dim enough that the frame's curve does not flatten it.
    let dim = Sky {
        sun: Vec3::splat(0.6),
        ..leaning()
    };
    let Some(open) = frame_of(WHAT, dim) else {
        return;
    };
    let mut sky = dim;
    sky.scene.clouds.open = 0.25;
    let clouded = frame_of(WHAT, sky).expect("a device a moment ago");
    let (under, clear) = (seen_at(&clouded, 0.6, -0.2), seen_at(&open, 0.6, -0.2));
    let (dimmed, whole) = (u32::from(under[0]), u32::from(clear[0]));
    assert!(
        whole > 40 && dimmed < whole * 3 / 4 && dimmed > whole / 8,
        "{under:?} {clear:?}"
    );
    // Half hidden, and what is hidden taken all the way towards surroundings that give
    // nothing: half the light is left, and half of that comes through.
    sky.scene.clouds = Clouds {
        open: 0.5,
        haze_blend: 1.0,
        ..Clouds::default()
    };
    let hazed = frame_of(WHAT, sky).expect("a device a moment ago");
    assert_eq!(seen_at(&hazed, 0.6, -0.2), under);
}
