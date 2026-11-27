//! The traced sun's mask on the machine's software adapter: the game's walk of the user's own
//! build, lowered alone, dispatched over a made-up frame and held pixel for pixel to the CPU's
//! reference walk of the same rays. No hardware adapter is asked for.
#![cfg(feature = "device-check")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3, Vec4};
use or_io::gta6::accel::{walk_lane, Hit, Ray, Triangle};
use or_renderer::game_shaders::meshlets::light_for;
use or_renderer::game_shaders::trace::{
    cast_of, query, Frame, PixelView, SceneArena, TracedMask, View, WalkModule,
};
use or_shader_front::rdna_bind::walk_file::{shadow_walk, WALKS};
use wgpu::util::DeviceExt;

const SIZE: (u32, u32) = (64, 48);
/// The roof the sun's shadow falls from, a unit over the ground.
const ROOF: [[f32; 3]; 3] = [[-1.2, -0.9, 1.0], [0.7, -1.1, 1.0], [-0.8, 0.8, 1.0]];
/// A normal straight up as the targets pack it: an octahedral pair of twelve bits each in the
/// last three bytes, both at their middle.
const UP: [u8; 4] = [0, 0x00, 0x08, 0x80];

fn software(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::DX12;
    }
    descriptor.flags = wgpu::InstanceFlags::empty();
    let instance = wgpu::Instance::new(or_renderer::instance::with_dxc(descriptor));
    let asked = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: true,
    };
    let adapter = match pollster::block_on(instance.request_adapter(&asked)) {
        Ok(adapter) if adapter.get_info().device_type == wgpu::DeviceType::Cpu => adapter,
        Ok(adapter) => {
            let name = adapter.get_info().name;
            or_test_support::skip_device(test, format_args!("{name} is no software adapter"));
            return None;
        }
        Err(why) => {
            or_test_support::skip_device(test, format_args!("no software adapter: {why}"));
            return None;
        }
    };
    // Opened with the ray query where the adapter offers it, so that body Q runs too.
    let mut required_limits = adapter.limits();
    let (required_features, experimental_features) =
        query::device_needs(&adapter, &mut required_limits);
    let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rdna-trace-mask"),
        required_features,
        required_limits,
        experimental_features,
        ..Default::default()
    }));
    Some(device.expect("the software adapter's device"))
}

/// Straight down onto the ground: x and y from -2 to 2 across the frame, and a depth of a quarter
/// at z = 0, more the higher.
fn from_above() -> Mat4 {
    Mat4::from_cols(
        Vec4::new(0.5, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.5, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.25, 0.0),
        Vec4::new(0.0, 0.0, 0.25, 1.0),
    )
}

/// The normal [`UP`] decodes to, as the program that lights the frame decodes it.
fn up() -> Vec3 {
    let e = 2048.0 / 2047.5 - 1.0;
    Vec3::new(e, e, 1.0 - 2.0 * e).normalize()
}

fn texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// Whether `ray` crosses the roof's plane within a hair of one of its edges, where the device's
/// rounding and the CPU's may part.
fn near_an_edge(ray: &Ray) -> bool {
    let t = (ROOF[0][2] - ray.origin[2]) / ray.direction[2];
    let p = [0, 1].map(|k| ray.origin[k] + ray.direction[k] * t);
    (0..3).any(|k| {
        let (a, b) = (ROOF[k], ROOF[(k + 1) % 3]);
        let e = [b[0] - a[0], b[1] - a[1]];
        let side = ((p[0] - a[0]) * e[1] - (p[1] - a[1]) * e[0]) / e[0].hypot(e[1]);
        side.abs() < 0.02
    })
}

/// `corners` cut into `n` by `n` triangles of the same plane and outline.
fn cut_in(corners: [[f32; 3]; 3], n: usize) -> Vec<[[f32; 3]; 3]> {
    let [a, b, c] = corners.map(Vec3::from);
    let at = |i: usize, j: usize| {
        (a + (b - a) * i as f32 / n as f32 + (c - a) * j as f32 / n as f32).to_array()
    };
    let mut cut = Vec::new();
    for i in 0..n {
        for j in 0..n - i {
            cut.push([at(i, j), at(i + 1, j), at(i, j + 1)]);
            if i + j + 1 < n {
                cut.push([at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
    }
    cut
}

#[test]
fn the_traced_mask_of_a_made_up_frame_is_the_reference_walk_s_pixel_for_pixel() {
    let test = "the traced sun's mask on the software adapter";
    if std::env::var_os(WALKS).is_none() {
        or_test_support::skip_install(test, "OPENRAGE_RDNA_WALKS names no file of a walk");
        return;
    }
    let Some((device, queue)) = software(test) else {
        return;
    };
    // Body S, its nodes tested by the arena's arithmetic, then body Q, by the device's query.
    held_to_the_reference(&device, &queue, false, false);
    // The control: a bound of nought holds the walk to four turns, which a ray that meets the roof
    // passes; each such ray is capped, counted, and left lit.
    held_to_the_reference(&device, &queue, false, true);
    if query::offered(device.adapter_info().backend, device.features()) {
        held_to_the_reference(&device, &queue, true, false);
    } else {
        or_test_support::skip_device(test, "the adapter offers no ray query: body Q is not run");
    }
}

fn held_to_the_reference(device: &wgpu::Device, queue: &wgpu::Queue, query: bool, cut: bool) {
    let (device, queue) = (device.clone(), queue.clone());
    let lowered = shadow_walk(None, query).expect("a shadow ray's walk of the build");
    let walk = Arc::new(WalkModule {
        module: Arc::new(lowered.module.clone()),
        given: lowered.given.clone(),
        left: lowered.left,
        threads: lowered.threads,
        shared_dwords: lowered.shared_dwords,
        bound: if cut { 0 } else { lowered.walk.bound },
        unbounded: lowered.loops_not_shown_to_end,
    });
    let corner = |x: f32, y: f32| [x, y, 0.0];
    let triangle = |corners: [[f32; 3]; 3], primitive: u32| Triangle {
        corners,
        geometry: 0,
        primitive,
        opaque: true,
    };
    let mut scene = vec![
        triangle(
            [corner(-2.0, -2.0), corner(2.0, -2.0), corner(-2.0, 2.0)],
            0,
        ),
        triangle([corner(2.0, -2.0), corner(2.0, 2.0), corner(-2.0, 2.0)], 1),
    ];
    // The control's roof is the same triangle cut in 256, so that a ray meets it only boxes deep.
    let pieces = if cut { 16 } else { 1 };
    for (k, corners) in cut_in(ROOF, pieces).into_iter().enumerate() {
        scene.push(triangle(corners, 2 + k as u32));
    }
    let arena = Arc::new(SceneArena::build(&scene).expect("the scene's arena"));
    let mut traced = TracedMask::new(&device, &queue, walk, Arc::clone(&arena), 1)
        .expect("the device runs the walk");
    let started = Instant::now();
    loop {
        match traced.state() {
            Some(Ok(())) => break,
            Some(Err(why)) => panic!("the device refused the traced pass: {why}"),
            None if started.elapsed() > Duration::from_secs(600) => panic!("no pass in 600 s"),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }

    let depth = texture(
        &device,
        wgpu::TextureFormat::Depth32Float,
        SIZE,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let depth_view = depth.create_view(&Default::default());
    let normal = texture(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        SIZE,
        wgpu::TextureUsages::COPY_DST,
    );
    queue.write_texture(
        normal.as_image_copy(),
        &UP.repeat((SIZE.0 * SIZE.1) as usize),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE.0 * 4),
            rows_per_image: Some(SIZE.1),
        },
        normal.size(),
    );
    let roots = texture(
        &device,
        wgpu::TextureFormat::R32Float,
        SIZE,
        wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
    );
    let tiles = texture(
        &device,
        wgpu::TextureFormat::R32Uint,
        (SIZE.0.div_ceil(16), SIZE.1.div_ceil(16)),
        wgpu::TextureUsages::COPY_DST,
    );
    let towards = Vec3::new(0.3, 0.2, 1.0).normalize();
    let camera = Vec3::new(0.0, 0.0, 10.0);
    let light = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &light_for(
            from_above().to_cols_array(),
            towards.to_array(),
            camera.to_array(),
        ),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let view = View {
        view_proj: from_above().to_cols_array(),
        towards: towards.to_array(),
        disc: 0.01,
        clouds: 1.0,
    };
    let normal_view = normal.create_view(&Default::default());
    let frame = Frame {
        depth: &depth_view,
        normal: &normal_view,
        light: &light,
        roots: &roots,
    };
    // The first frame is one that moved, the second the first sample of the view standing still.
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    for drawn in 0..2 {
        traced.update(&queue, &view, SIZE, &tiles);
        let mut encoder = device.create_command_encoder(&Default::default());
        if drawn == 0 {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.25),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            drop(pass);
        }
        assert!(
            traced.record(&mut encoder, &frame),
            "frame {drawn} casts rays"
        );
        queue.submit([encoder.finish()]);
    }
    assert_eq!(traced.rays(), u64::from(SIZE.0 * SIZE.1));
    let rows = 256 * SIZE.1 as u64;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: rows,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        roots.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(SIZE.1),
            },
        },
        roots.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the frames run to their end");
    let refused = pollster::block_on(validation.pop());
    assert!(refused.is_none(), "{refused:?}");
    let mapped = read.slice(..).get_mapped_range();
    let at = |x: u32, y: u32| {
        let i = (y * 256 + x * 4) as usize;
        f32::from_le_bytes(mapped[i..i + 4].try_into().unwrap())
    };

    let pixel_view = PixelView {
        unproject: from_above().inverse(),
        towards,
        camera,
        disc: 0.01,
        clouds: 1.0,
    };
    let (mut shadowed, mut lit, mut edges, mut differing) = (0, 0, 0, Vec::new());
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            let cast = cast_of(&pixel_view, ([x, y], SIZE), (0.25, up()), 0, arena.sphere());
            let ray = cast
                .ray
                .expect("every pixel shows the ground, which faces the sun");
            if near_an_edge(&ray) {
                edges += 1;
                continue;
            }
            let hit = walk_lane(
                arena.words(),
                arena.top.base,
                &ray,
                &lowered.walk,
                &mut |_: &Hit| true,
            )
            .hit;
            let wanted = if hit.is_some() {
                0.0
            } else {
                cast.share.sqrt()
            };
            match hit {
                Some(_) => shadowed += 1,
                None => lit += 1,
            }
            if at(x, y) != wanted {
                differing.push(([x, y], at(x, y), wanted));
            }
        }
    }
    assert!(shadowed > 0 && lit > 0, "{shadowed} shadowed, {lit} lit");
    assert!(
        edges < (SIZE.0 * SIZE.1) as usize / 4,
        "{edges} pixels near an edge"
    );
    eprintln!(
        "query {query}, cut {cut}: {shadowed} shadowed, {lit} lit, {edges} near an edge, {} differ",
        differing.len()
    );
    if cut {
        let lit_where_shadowed =
            |(_, got, wanted): &([u32; 2], f32, f32)| *wanted == 0.0 && *got > 0.0;
        assert!(
            !differing.is_empty() && differing.iter().all(lit_where_shadowed),
            "{differing:?}"
        );
    } else {
        assert!(
            differing.is_empty(),
            "query {query}: {} differ, the first {:?}",
            differing.len(),
            &differing[..differing.len().min(6)]
        );
    }
    drop(mapped);
    // The count of capped rays the first frame copied is mapped by the next update and read by the one after.
    for _ in 0..2 {
        traced.update(&queue, &view, SIZE, &tiles);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the count is mapped");
    }
    eprintln!("query {query}, cut {cut}: {} capped", traced.capped());
    match cut {
        true => assert!(traced.capped() > 0, "the control capped no ray"),
        false => assert_eq!(
            traced.capped(),
            0,
            "query {query}: a real walk reached the cap"
        ),
    }
}
