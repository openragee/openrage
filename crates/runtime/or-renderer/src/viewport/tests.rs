use super::*;

#[test]
fn a_target_past_the_texture_limit_is_scaled_down_keeping_its_aspect() {
    assert_eq!(fit_texture_limit(1920, 1009, 8192), (1920, 1009));
    assert_eq!(fit_texture_limit(1920, 9886, 8192), (1591, 8192));
    assert_eq!(fit_texture_limit(20000, 100, 8192), (8192, 40));
    assert_eq!(fit_texture_limit(100000, 1, 8192), (8192, 1));
}

#[test]
fn perspective_is_reverse_z() {
    let cam = OrbitCamera::default();
    let proj = cam.view_proj(1.0);
    let near_pos = cam.eye() + (cam.target - cam.eye()).normalize() * cam.near;
    let near_clip = proj * glam::Vec4::new(near_pos.x, near_pos.y, near_pos.z, 1.0);
    let near_ndc_z = near_clip.z / near_clip.w;
    assert!(
        (near_ndc_z - 1.0).abs() < 1e-3,
        "reverse-Z should map the near plane to NDC z = 1.0, got {near_ndc_z}"
    );

    let far_pos = cam.eye() + (cam.target - cam.eye()).normalize() * cam.far;
    let far_clip = proj * glam::Vec4::new(far_pos.x, far_pos.y, far_pos.z, 1.0);
    let far_ndc_z = far_clip.z / far_clip.w;
    assert!(
        far_ndc_z.abs() < 1e-3,
        "reverse-Z should map the far plane to NDC z = 0.0, got {far_ndc_z}"
    );
}

#[test]
fn camera_uses_z_up_world_convention() {
    assert_eq!(
        OrbitCamera::WORLD_UP,
        Vec3::Z,
        "RAGE world is Z-up; viewport must agree"
    );
    let cam = OrbitCamera::default();
    assert!(
        cam.eye().z > 0.0,
        "default camera should sit above the ground plane (eye = {:?})",
        cam.eye()
    );
}

#[test]
fn orbit_drag_right_moves_camera_clockwise_from_above() {
    let mut cam = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 1.0,
        fov_y: 60f32.to_radians(),
        near: 0.05,
        far: 100.0,
    };
    assert!((cam.eye().x - 1.0).abs() < 1e-3 && cam.eye().y.abs() < 1e-3);
    cam.orbit(10.0, 0.0);
    assert!(
        cam.eye().y < 0.0,
        "drag-right should sweep the camera to -Y, got eye = {:?}",
        cam.eye()
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn pipelines_compile() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    })) {
        Ok(a) => a,
        Err(e) => {
            or_test_support::skip_device("pipelines_compile", format_args!("no wgpu adapter: {e}"));
            return;
        }
    };
    let (device, queue) =
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("or-renderer-test"),
            required_features: wgpu::Features::empty(),
            required_limits: {
                let mut l = wgpu::Limits::downlevel_defaults();
                l.max_bind_groups = 5;
                l
            },
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        })) {
            Ok(p) => p,
            Err(e) => {
                or_test_support::skip_device(
                    "pipelines_compile",
                    format_args!("device init failed: {e}"),
                );
                return;
            }
        };

    let renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let _layout = renderer.material_bind_group_layout();
}

fn bytes_test_device(what: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    let adapter =
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        {
            Ok(a) => a,
            Err(e) => {
                or_test_support::skip_device(what, format_args!("no wgpu adapter: {e}"));
                return None;
            }
        };
    match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bytes-test"),
        // The stages of a frame are timed where the device can, as the app's is
        required_features: adapter.features() & crate::gpu_timer::FEATURES,
        required_limits: wgpu::Limits {
            max_bind_groups: 5,
            ..wgpu::Limits::default()
        },
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    })) {
        Ok(p) => Some(p),
        Err(e) => {
            or_test_support::skip_device(what, format_args!("device init failed: {e}"));
            None
        }
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn target_bytes_follow_the_targets_the_renderer_makes_and_resizes() {
    let Some((device, queue)) =
        bytes_test_device("target_bytes_follow_the_targets_the_renderer_makes_and_resizes")
    else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let forward = renderer.target_bytes();
    assert_eq!(
        forward,
        texture_bytes(&renderer.colour) + texture_bytes(renderer.depth_view.texture())
    );
    assert!(forward > 0);

    renderer.resize(&device, 128, 128);
    assert!(
        renderer.target_bytes() > forward,
        "the targets follow the size"
    );
    renderer.resize(&device, 64, 64);
    assert_eq!(
        renderer.target_bytes(),
        forward,
        "a replaced target is not counted twice"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn lines_drawn_in_front_show_through_a_mesh_nearer_the_camera() {
    const WHAT: &str = "lines_drawn_in_front_show_through_a_mesh_nearer_the_camera";
    const RED: [u8; 4] = [255, 0, 0, 255];
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = ViewportRenderer::new(&device, &queue, format, 64, 64);
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: 0.0,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    // A square facing the camera, nearer it than the line through the middle of the view.
    let corners = [
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [1.0, 1.0, 1.0],
        [1.0, -1.0, 1.0],
    ];
    let vertices = corners.map(|at| MeshVertex::new(at, [1.0, 0.0, 0.0], [0.0, 0.0]));
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    renderer.set_meshes(vec![MeshDraw {
        bump: false,
        vertex_buffer: buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX),
        index_buffer: buffer(
            bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            wgpu::BufferUsages::INDEX,
        ),
        index_count: 6,
        texture_bind_group: None,
    }]);
    let line = [
        LineVertex::new([0.0, 0.0, -0.5], [1.0, 0.0, 0.0, 1.0]),
        LineVertex::new([0.0, 0.0, 0.5], [1.0, 0.0, 0.0, 1.0]),
    ];
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // Whether the middle row, across the line, shows it.
    let mut shows_the_line = |in_front: usize| {
        renderer.set_lines(&device, &queue, &line, in_front);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &queue, &camera, ShadingMode::Solid);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &renderer.colour,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 28, y: 32, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 8,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let shown = slice.get_mapped_range()[..32]
            .as_chunks::<4>()
            .0
            .contains(&RED);
        readback.unmap();
        shown
    };
    assert!(!shows_the_line(0), "the square hides the line behind it");
    assert!(shows_the_line(2), "a line drawn in front is hidden");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_normal_map_tilts_the_light_a_material_takes_and_gives_it_no_colour() {
    const WHAT: &str = "a_normal_map_tilts_the_light_a_material_takes_and_gives_it_no_colour";
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, 64, 64);
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: 0.0,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    // A square facing the camera, its texture coordinate running across it.
    let corners = [
        ([1.0, -1.0, -1.0], [0.0, 0.0]),
        ([1.0, 1.0, -1.0], [1.0, 0.0]),
        ([1.0, 1.0, 1.0], [1.0, 1.0]),
        ([1.0, -1.0, 1.0], [0.0, 1.0]),
    ];
    let vertices = corners.map(|(at, uv)| MeshVertex::new(at, [1.0, 0.0, 0.0], uv));
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    let layout = renderer.material_bind_group_layout();
    // A texture of one linear texel.
    let texel = |rgba: [u8; 4]| {
        let size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            size,
        );
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        }))
    };
    let square = MeshDraw {
        vertex_buffer: buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX),
        index_buffer: buffer(
            bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            wgpu::BufferUsages::INDEX,
        ),
        index_count: 6,
        texture_bind_group: None,
        bump: false,
    };
    // The pixel in the middle of the square, drawn as a material.
    let mut middle = |rgba: Option<[u8; 4]>, bump: bool| {
        renderer.set_meshes(vec![MeshDraw {
            texture_bind_group: rgba.map(texel),
            bump,
            ..square.clone()
        }]);
        image_after(&device, &queue, &renderer, &camera, ShadingMode::Material)[32 * 64 + 32]
    };
    let near = |a: [u8; 4], b: [u8; 4]| (0..3).all(|c| a[c].abs_diff(b[c]) <= 3);
    const FLAT: [u8; 4] = [128, 128, 255, 255];
    let untextured = middle(None, false);
    let flat = middle(Some(FLAT), true);
    assert!(
        near(flat, untextured),
        "a flat normal map leaves the light as it was and its blue tints nothing: {flat:?} against {untextured:?}"
    );
    let tinted = middle(Some(FLAT), false);
    assert!(
        u16::from(tinted[0]) + 16 < u16::from(tinted[2]),
        "the same texel drawn as a colour is blue: {tinted:?}"
    );
    // A normal lying in the square takes none of the light from the camera, and to one side or
    // the other none of the key light.
    let tilted = [[255, 128, 128, 255], [0, 128, 128, 255]].map(|t| middle(Some(t), true)[0]);
    let darkest = tilted[0].min(tilted[1]);
    assert!(
        u16::from(darkest) + 16 < u16::from(flat[0]),
        "a normal tilted into the surface is lit less: {tilted:?} against {flat:?}"
    );
}

/// Every pixel of the viewport, row by row, after `renderer` draws in `mode`: its width is a multiple of 64, so a row is a whole number of copy rows.
fn image_after(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &ViewportRenderer,
    camera: &OrbitCamera,
    mode: ShadingMode,
) -> Vec<[u8; 4]> {
    let (width, height) = (renderer.width, renderer.height);
    let row = width * 4;
    assert_eq!(row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(&mut encoder, queue, camera, mode);
    encoder.copy_texture_to_buffer(
        renderer.colour.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let pixels = slice
        .get_mapped_range()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect();
    readback.unmap();
    pixels
}

/// The colour of the middle pixel after `renderer` draws in `mode`.
fn middle_pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &ViewportRenderer,
    camera: &OrbitCamera,
    mode: ShadingMode,
) -> [u8; 4] {
    let image = image_after(device, queue, renderer, camera, mode);
    image[(renderer.height / 2 * renderer.width + renderer.width / 2) as usize]
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_translucent_surface_in_the_material_mode_leaves_the_one_behind_it_visible() {
    const WHAT: &str = "a_translucent_surface_in_the_material_mode_leaves_the_one_behind";
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let camera = OrbitCamera {
        pitch: 1.5,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    let quad_at = |z: f32, colour: [f32; 4]| {
        let corners = [
            [-1.0, -1.0, z],
            [1.0, -1.0, z],
            [1.0, 1.0, z],
            [-1.0, 1.0, z],
        ];
        let vertices =
            corners.map(|at| MeshVertex::new_coloured(at, [0.0, 0.0, 1.0], [0.0, 0.0], colour));
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
                bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
                wgpu::BufferUsages::INDEX,
            ),
            index_count: 6,
            texture_bind_group: None,
        }
    };
    // A half-transparent red shell, nearer the camera and listed first, over an opaque blue core
    let shell = || quad_at(0.0, [1.0, 0.0, 0.0, 0.5]);
    let core = || quad_at(-0.5, [0.0, 0.0, 1.0, 1.0]);
    let middle = |renderer: &ViewportRenderer| {
        middle_pixel(&device, &queue, renderer, &camera, ShadingMode::Material)
    };
    renderer.set_meshes(vec![shell()]);
    let shell_alone = middle(&renderer);
    renderer.set_meshes(vec![shell(), core()]);
    let both = middle(&renderer);
    assert!(
        both[2] > shell_alone[2] + 40,
        "the core shows through the shell: {both:?} against {shell_alone:?}"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_game_shaders_mode_draws_a_prepared_scene_and_without_one_draws_as_a_material_does() {
    use crate::game_shaders::mesh::MeshScene;
    use crate::game_shaders::pack::GamePack;
    use crate::game_shaders::synthetic::{material, pack_on_disk, quad};
    const WHAT: &str = "the_game_shaders_mode_draws_a_prepared_scene";
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let dir = pack_on_disk();
    let pack = GamePack::open(dir.path()).unwrap();
    // The colour target is sRGB, as a window's is, and a half of linear light reads back as 188
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let camera = OrbitCamera {
        pitch: 1.5,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    let surfaces = [(quad(0.0), material([0.5, 0.5, 0.5, 1.0], Some([255; 4])))];
    let scene = MeshScene::prepare(&device, &queue, &pack, &renderer.game_target(), &surfaces);
    assert_eq!(scene.drawn(), 1);
    assert!(!renderer.has_game_scene());
    // A surface of the viewport's own, in the middle of the view, for the modes that draw it
    let corners = [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    let vertices = corners.map(|at| MeshVertex::new(at, [0.0, 0.0, 1.0], [0.0, 0.0]));
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    renderer.set_meshes(vec![MeshDraw {
        bump: false,
        vertex_buffer: buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX),
        index_buffer: buffer(
            bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            wgpu::BufferUsages::INDEX,
        ),
        index_count: 6,
        texture_bind_group: None,
    }]);
    renderer.set_game_scene(Some(scene));
    assert!(renderer.has_game_scene());

    let drawn = middle_pixel(&device, &queue, &renderer, &camera, ShadingMode::Shaded);
    assert!(drawn[..3].iter().all(|c| c.abs_diff(188) <= 2), "{drawn:?}");
    // Another mode draws the surface of the viewport's own, and leaves the scene alone
    let material_mode = middle_pixel(&device, &queue, &renderer, &camera, ShadingMode::Material);
    let wireframe_mode = middle_pixel(&device, &queue, &renderer, &camera, ShadingMode::Wireframe);
    assert_ne!(material_mode, drawn);
    assert_ne!(
        material_mode, wireframe_mode,
        "a mode that draws no surface differs, so the fallback draws one"
    );

    // With the scene taken away the mode draws what the material mode does
    renderer.set_game_scene(None);
    let without = middle_pixel(&device, &queue, &renderer, &camera, ShadingMode::Shaded);
    assert_eq!(without, material_mode);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_game_shaders_see_the_aspect_of_the_view_and_a_sun_over_the_viewers_shoulder() {
    use crate::game_shaders::mesh::{light_towards, MeshScene};
    use crate::game_shaders::pack::GamePack;
    use crate::game_shaders::synthetic::{material, pack_on_disk, quad};
    const WHAT: &str = "the_game_shaders_see_the_aspect_of_the_view";
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let dir = pack_on_disk();
    let pack = GamePack::open(dir.path()).unwrap();
    // Twice as wide as high, and a target that holds the colours as they are written
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, 128, 64);
    let camera = OrbitCamera {
        yaw: 0.7,
        pitch: 1.5,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    let mut sunlit = material([1.0; 4], None);
    sunlit.effect = "sunlit".into();
    let surfaces = [(quad(0.0), sunlit)];
    let scene = MeshScene::prepare(&device, &queue, &pack, &renderer.game_target(), &surfaces);
    renderer.set_game_scene(Some(scene));
    let image = image_after(&device, &queue, &renderer, &camera, ShadingMode::Shaded);
    // The corner is the clear colour, which is opaque too
    let covered = |at: usize| image[at] != image[0];
    let (width, height) = (128usize, 64usize);
    let across = (0..width)
        .filter(|x| covered(height / 2 * width + x))
        .count();
    let down = (0..height)
        .filter(|y| covered(y * width + width / 2))
        .count();
    // A square seen square covers as many columns as rows, whatever the shape of the view
    assert!(across.abs_diff(down) <= 2, "{across} columns, {down} rows");
    assert!(across > 8, "the square is seen at all: {across}");
    // The sun it is lit by is the one over the shoulder of this camera, its light travelling away from it, as a colour
    let sun = light_towards(camera.eye(), camera.target, OrbitCamera::WORLD_UP);
    let got = image[height / 2 * width + width / 2];
    for (channel, component) in got.iter().zip((-sun).to_array()) {
        let want = (component * 0.5 + 0.5) * 255.0;
        assert!((f32::from(*channel) - want).abs() <= 2.5, "{got:?} {sun:?}");
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn resident_bytes_count_each_class_once_it_is_allocated() {
    let Some((device, queue)) =
        bytes_test_device("resident_bytes_count_each_class_once_it_is_allocated")
    else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let start = renderer.resident_bytes();
    assert_eq!(start.line_buffers, 0);
    assert_eq!(start.particle_instance_buffers, 0);
    assert_eq!(start.uniform_buffers, renderer.camera_buffer.size());
    assert!(start.default_textures > 0, "the material fallback");

    renderer.set_lines(
        &device,
        &queue,
        &[LineVertex::new([0.0; 3], [1.0; 4]); 2],
        0,
    );
    assert!(renderer.resident_bytes().line_buffers > 0);
}

/// The colour of a pixel of a `size`-square render of `renderer`, row by row from the top left.
fn pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &ViewportRenderer,
    at: (u32, u32),
) -> [u8; 4] {
    let (size, _) = renderer.size();
    let read = crate::game_shaders::frame::read_back(
        device,
        queue,
        renderer.colour_texture(),
        renderer.colour_format,
        size,
    );
    let i = ((at.1 * size + at.0) * 4) as usize;
    [
        read.bytes[i],
        read.bytes[i + 1],
        read.bytes[i + 2],
        read.bytes[i + 3],
    ]
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn shaded_draws_the_drawable_through_the_deferred_passes_and_keeps_the_background_around_it() {
    use crate::game_shaders::mesh::{Image, Material};
    use crate::game_shaders::synthetic::quad;
    let test = "shaded_draws_the_drawable_through_the_deferred_passes";
    let Some((device, queue)) = bytes_test_device(test) else {
        return;
    };
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return;
    };
    let pack = Arc::new(crate::game_shaders::pack::GamePack::open(root).expect("the pack opens"));
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    renderer.set_background([0.1, 0.2, 0.3, 1.0]);
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    let surfaces = [(
        quad(0.0),
        Material {
            effect: "default".into(),
            textures: [("DiffuseSampler".to_string(), white)].into(),
            ..Material::default()
        },
    )];
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let (drawn, left_out) = renderer
        .prepare_deferred(&device, &queue, &pack, &surfaces)
        .expect("the deferred passes prepare");
    assert_eq!((drawn, left_out.len()), (1, 0), "{left_out:?}");
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: std::f32::consts::FRAC_PI_2 - 0.2,
        ..OrbitCamera::default()
    };
    let draw = |renderer: &ViewportRenderer| {
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
        queue.submit([encoder.finish()]);
    };
    // sRGB of 0.1, 0.2 and 0.3, as the target stores them
    let background = [89, 124, 149, 255];
    let near = |got: [u8; 4], want: [u8; 4]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 2);

    draw(&renderer);
    let middle = pixel(&device, &queue, &renderer, (32, 32));
    let corner = pixel(&device, &queue, &renderer, (1, 1));
    assert!(!near(middle, background), "the drawable shows: {middle:?}");
    assert!(near(corner, background), "the background shows: {corner:?}");

    // The targets made again are read again, and the frame is as before
    renderer.resize(&device, 96, 96);
    renderer.prepare_game_frame(&device, &queue);
    draw(&renderer);
    let middle = pixel(&device, &queue, &renderer, (48, 48));
    let corner = pixel(&device, &queue, &renderer, (1, 1));
    assert!(!near(middle, background), "{middle:?}");
    assert!(near(corner, background), "{corner:?}");
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every pass"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn shaded_draws_the_games_sky_round_the_drawable_when_asked_to_and_the_background_colour_otherwise()
{
    use crate::game_shaders::lighting::Day;
    use crate::game_shaders::mesh::{Image, Material};
    use crate::game_shaders::synthetic::quad;
    let test = "shaded_draws_the_games_sky_round_the_drawable";
    let Some((device, queue)) = bytes_test_device(test) else {
        return;
    };
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return;
    };
    let pack = Arc::new(crate::game_shaders::pack::GamePack::open(root).expect("the pack opens"));
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    renderer.set_background([0.1, 0.2, 0.3, 1.0]);
    let mut keyframe = or_io::gta5::data::timecycle::Keyframe::default();
    for (name, value) in [
        ("sky_azimuth_east_col_b", 1.0),
        ("sky_azimuth_east_col_g", 0.6),
        ("sky_azimuth_east_col_inten", 1.0),
        ("sky_azimuth_west_col_b", 1.0),
        ("sky_azimuth_west_col_g", 0.6),
        ("sky_azimuth_west_col_inten", 1.0),
        ("sky_azimuth_transition_col_b", 0.9),
        ("sky_azimuth_transition_col_g", 0.7),
        ("sky_azimuth_transition_col_inten", 1.0),
        ("sky_azimuth_transition_position", 0.5),
        ("sky_zenith_col_b", 0.5),
        ("sky_zenith_col_g", 0.2),
        ("sky_zenith_col_inten", 1.0),
        ("sky_zenith_transition_position", 0.8),
        ("sky_hdr", 2.0),
        ("postfx_exposure_min", 0.0),
        ("postfx_exposure_max", 0.0),
    ] {
        keyframe.set(name, value);
    }
    renderer.set_day(Some(Day {
        keyframe,
        ..Day::default()
    }));
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    let surfaces = [(
        quad(0.0),
        Material {
            effect: "default".into(),
            textures: [("DiffuseSampler".to_string(), white)].into(),
            ..Material::default()
        },
    )];
    renderer
        .prepare_deferred(&device, &queue, &pack, &surfaces)
        .expect("the deferred passes prepare");
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: std::f32::consts::FRAC_PI_2 - 0.2,
        ..OrbitCamera::default()
    };
    let draw = |renderer: &ViewportRenderer| {
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
        queue.submit([encoder.finish()]);
    };
    let background = [89, 124, 149, 255];
    let near = |got: [u8; 4], want: [u8; 4]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 2);

    draw(&renderer);
    assert!(near(pixel(&device, &queue, &renderer, (1, 1)), background));

    renderer.set_sky_background(true);
    renderer.prepare_game_frame(&device, &queue);
    draw(&renderer);
    let corner = pixel(&device, &queue, &renderer, (1, 1));
    assert!(!near(corner, background), "the sky shows: {corner:?}");
    assert!(
        corner[2] >= corner[0],
        "a clear sky is blue the most: {corner:?}"
    );
    let middle = pixel(&device, &queue, &renderer, (32, 32));
    assert!(
        !near(middle, corner),
        "the drawable is not the sky: {middle:?}"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_cutout_surface_of_the_peds_hair_effect_shows_in_shaded_where_its_alpha_keeps_it() {
    use crate::game_shaders::mesh::{Coverage, Image, Material};
    use crate::game_shaders::synthetic::quad;
    let test = "a_cutout_surface_of_the_peds_hair_effect_shows_in_shaded";
    let Some((device, queue)) = bytes_test_device(test) else {
        return;
    };
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return;
    };
    let pack = Arc::new(crate::game_shaders::pack::GamePack::open(root).expect("the pack opens"));
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    renderer.set_background([0.1, 0.2, 0.3, 1.0]);
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    let surfaces = [(
        crate::game_shaders::mesh::Surface {
            skinned: true,
            ..quad(0.0)
        },
        Material {
            effect: "ped_hair_spiked".into(),
            textures: [("DiffuseSampler".to_string(), white)].into(),
            coverage: Coverage::Cutout,
            ..Material::default()
        },
    )];
    let (drawn, left_out) = renderer
        .prepare_deferred(&device, &queue, &pack, &surfaces)
        .expect("the deferred passes prepare");
    assert_eq!((drawn, left_out.len()), (1, 0), "{left_out:?}");
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: std::f32::consts::FRAC_PI_2 - 0.2,
        ..OrbitCamera::default()
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
    queue.submit([encoder.finish()]);
    // sRGB of 0.1, 0.2 and 0.3, as the target stores them
    let background = [89, 124, 149, 255];
    let middle = pixel(&device, &queue, &renderer, (32, 32));
    assert!(
        middle
            .iter()
            .zip(background)
            .any(|(g, w)| g.abs_diff(w) > 2),
        "the hair shows: {middle:?}"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_cutout_whose_clipping_pass_writes_one_target_leaves_the_frame_valid_and_the_rest_drawn() {
    use crate::game_shaders::mesh::{Coverage, Image, Material, Surface};
    use crate::game_shaders::synthetic::quad;
    let test = "a_cutout_whose_clipping_pass_writes_one_target";
    let Some((device, queue)) = bytes_test_device(test) else {
        return;
    };
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return;
    };
    let pack = Arc::new(crate::game_shaders::pack::GamePack::open(root).expect("the pack opens"));
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    renderer.set_background([0.1, 0.2, 0.3, 1.0]);
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    // The vehicles' clipping technique writes the albedo alone, where the G-buffer has four targets
    let surfaces = [
        (
            Surface {
                skinned: true,
                ..quad(0.0)
            },
            Material {
                effect: "vehicle_cutout".into(),
                textures: [("DiffuseSampler".to_string(), white.clone())].into(),
                coverage: Coverage::Cutout,
                ..Material::default()
            },
        ),
        (
            quad(0.0),
            Material {
                effect: "default".into(),
                textures: [("DiffuseSampler".to_string(), white)].into(),
                ..Material::default()
            },
        ),
    ];
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let (drawn, left_out) = renderer
        .prepare_deferred(&device, &queue, &pack, &surfaces)
        .expect("the deferred passes prepare");
    assert_eq!((drawn, left_out.len()), (2, 0), "{left_out:?}");
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: std::f32::consts::FRAC_PI_2 - 0.2,
        ..OrbitCamera::default()
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
    queue.submit([encoder.finish()]);
    let error = pollster::block_on(validation.pop());
    assert!(error.is_none(), "the device refused a pass: {error:?}");
    let background = [89, 124, 149, 255];
    let middle = pixel(&device, &queue, &renderer, (32, 32));
    assert!(
        middle
            .iter()
            .zip(background)
            .any(|(g, w)| g.abs_diff(w) > 2),
        "the other surface shows: {middle:?}"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_clear_glass_beside_what_is_opaque_leaves_the_background_showing_through_it() {
    use crate::game_shaders::mesh::{Coverage, Image, Material, Surface};
    use crate::game_shaders::synthetic::quad;
    let test = "a_clear_glass_beside_what_is_opaque";
    let Some((device, queue)) = bytes_test_device(test) else {
        return;
    };
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return;
    };
    let pack = Arc::new(crate::game_shaders::pack::GamePack::open(root).expect("the pack opens"));
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    renderer.set_background([0.1, 0.2, 0.3, 1.0]);
    let image = |rgba: [u8; 4]| Image {
        width: 1,
        height: 1,
        rgba: rgba.to_vec(),
        srgb: false,
    };
    let opaque = (
        quad(0.0),
        Material {
            effect: "default".into(),
            textures: [("DiffuseSampler".to_string(), image([255; 4]))].into(),
            ..Material::default()
        },
    );
    // A pane clear everywhere, which a vehicle's glass is where its mesh holds no damage, to the side of the opaque quad
    let mut pane = Surface {
        skinned: true,
        ..quad(0.0)
    };
    for position in &mut pane.positions {
        position[0] += 2.0;
    }
    pane.colours
        .insert(0, vec![[1.0, 1.0, 1.0, 0.0]; pane.positions.len()]);
    let glass = (
        pane,
        Material {
            effect: "vehicle_vehglass".into(),
            textures: [("DiffuseSampler".to_string(), image([255, 255, 255, 0]))].into(),
            coverage: Coverage::Blended,
            ..Material::default()
        },
    );
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: std::f32::consts::FRAC_PI_2 - 0.2,
        ..OrbitCamera::default()
    };
    let mut frame = |surfaces: &[(Surface, Material)]| {
        renderer
            .prepare_deferred(&device, &queue, &pack, surfaces)
            .expect("the deferred passes prepare");
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
        queue.submit([encoder.finish()]);
        let (size, _) = renderer.size();
        crate::game_shaders::frame::read_back(
            &device,
            &queue,
            renderer.colour_texture(),
            renderer.colour_format,
            size,
        )
        .bytes
    };
    let alone = frame(std::slice::from_ref(&opaque));
    let with_glass = frame(&[opaque.clone(), glass]);
    let differing =
        |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(a, b)| a.abs_diff(**b) > 3).count();
    assert_eq!(
        differing(&alone, &with_glass),
        0,
        "bytes the clear glass changed"
    );

    // A decal beside it is the other way about: it is a layer meant to be seen where it lies
    let mut layer = quad(0.0);
    for position in &mut layer.positions {
        position[0] += 2.0;
    }
    let decal = (
        layer,
        Material {
            effect: "decal".into(),
            textures: [("DiffuseSampler".to_string(), image([255; 4]))].into(),
            coverage: Coverage::Blended,
            ..Material::default()
        },
    );
    let with_decal = frame(&[opaque, decal]);
    assert!(
        differing(&alone, &with_decal) > 0,
        "the decal beside the opaque quad shows"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn water_reads_the_frame_behind_it_and_how_far_that_lies_and_is_prepared_again_with_the_targets() {
    use crate::game_shaders::deferred::HDR_FORMAT;
    use crate::game_shaders::frame::read_back;
    use crate::game_shaders::mesh::{Coverage, Image, Material, Surface};
    use crate::game_shaders::synthetic::quad;
    const WHAT: &str = "water_reads_the_frame_behind_it";
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let Some(pack) = crate::game_shaders::test_pack::real_pack(WHAT) else {
        return;
    };
    let water = Material {
        effect: "water_shallow".into(),
        coverage: Coverage::Blended,
        ..Material::default()
    };
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    let ground = Material {
        effect: "default".into(),
        textures: [("DiffuseSampler".to_string(), white)].into(),
        ..Material::default()
    };
    let camera = OrbitCamera {
        pitch: 1.5,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    // The pixel a point of the floor beside the water is drawn at
    let projection = perspective_reverse_z_rh(camera.fov_y, 1.0, camera.near, camera.far);
    let clip = projection * camera.view() * glam::Vec4::new(0.0, 1.6, -1.0, 1.0);
    let (x, y) = (clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    let beside = (y * 64.0) as usize * 64 + (x * 64.0) as usize;
    // What the water reads after it draws `surfaces`: the light beside it and the distance at its middle
    let read = |renderer: &mut ViewportRenderer, surfaces: &[(Surface, Material)]| {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        renderer
            .prepare_deferred(&device, &queue, &pack, surfaces)
            .expect("the scene is prepared");
        renderer.prepare_game_frame(&device, &queue);
        middle_pixel(&device, &queue, renderer, &camera, ShadingMode::Shaded);
        let error = pollster::block_on(validation.pop());
        assert!(error.is_none(), "{error:?}");
        let frame = renderer.deferred.as_ref().expect("the frame is made");
        let half = |bytes: &[u8], at: usize| {
            or_io::half::half_to_f32(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
        };
        let light = read_back(
            &device,
            &queue,
            frame.refraction.as_ref().expect("a copy is made").texture(),
            HDR_FORMAT,
            64,
        );
        let depth = read_back(
            &device,
            &queue,
            frame
                .passes
                .half_depth()
                .expect("the depth is made")
                .texture(),
            wgpu::TextureFormat::R16Float,
            32,
        );
        // The light beside the water, where nothing hides it, and the distance at the middle, where the water lies over the ground
        (
            half(&light.bytes, beside * 8),
            half(&depth.bytes, (16 * 32 + 16) * 2),
        )
    };
    let (alone_light, alone_depth) = read(&mut renderer, &[(quad(0.0), water.clone())]);
    // A floor three times as wide as the water, which shows beside it
    let mut floor = quad(-1.0);
    for at in &mut floor.positions {
        at[0] *= 3.0;
        at[1] *= 3.0;
    }
    let behind = [(quad(0.0), water.clone()), (floor, ground)];
    let (light, depth) = read(&mut renderer, &behind);
    // The ground is lit and is what lies behind the water there, and the nearest thing along the view is the ground
    assert!(light > alone_light + 0.05, "{light} against {alone_light}");
    let expected = -(camera.view() * glam::Vec4::new(0.0, 0.0, -1.0, 1.0)).z;
    assert!(
        (depth - expected).abs() < 0.03 * expected,
        "{depth} against {expected} metres"
    );
    assert!(
        alone_depth > depth,
        "nothing there is further than the ground"
    );
    // What it reflects is the map of the sky the frame is lit by, not a colour of its own
    let dressed = renderer.dress_water(&device, &queue, &behind);
    let reflected = dressed[0].1.views["ReflectionSampler"].texture();
    let sky = renderer.deferred.as_ref().expect("the frame is made");
    assert_eq!(reflected, sky.reflection.view().texture());
    // How thick it lies is what the fog pass wrote beside the colour, and how much a ray reflects is the game's curve
    let thick = dressed[0].1.views["LightingSampler"].texture();
    assert_eq!(thick, sky.fog_sink.as_ref().expect("a sink").texture());
    let curve = dressed[0].1.views["BlendSampler"].texture();
    assert_eq!(
        (curve.width(), curve.height()),
        (crate::game_shaders::water::BLEND_WIDTH as u32, 1)
    );
    // The ripples and the foam of the install are bound where it gives them, the second ripple map to both samplers the game reads it by, and a scene that holds water is asked to be prepared again to read them
    for name in [
        "StaticBumpSampler",
        "WaterBumpSampler",
        "WaterBumpSampler2",
        "StaticFoamSampler",
    ] {
        assert!(!dressed[0].1.views.contains_key(name), "{name}");
    }
    let map = |side: u32| crate::game_shaders::mesh::Image {
        width: side,
        height: side,
        rgba: vec![128; (side * side * 4) as usize],
        srgb: false,
    };
    renderer.set_water_textures(crate::game_shaders::water::WaterTextures {
        bump: Some(map(4)),
        bump2: Some(map(2)),
        foam: Some(map(8)),
    });
    assert!(renderer.deferred_needs_prepare());
    let rippled = renderer.dress_water(&device, &queue, &behind);
    let side = |name: &str| rippled[0].1.views[name].texture().width();
    assert_eq!(
        [
            side("StaticBumpSampler"),
            side("WaterBumpSampler"),
            side("WaterBumpSampler2"),
            side("StaticFoamSampler")
        ],
        [4, 2, 2, 8]
    );
    renderer
        .prepare_deferred(&device, &queue, &pack, &behind)
        .expect("the scene is prepared with them");
    // The copy is of the size of the frame, so a frame of another size has the scene made again
    assert!(!renderer.deferred_needs_prepare());
    renderer.resize(&device, 128, 64);
    assert!(renderer.deferred_needs_prepare());
    renderer
        .prepare_deferred(&device, &queue, &pack, &behind)
        .expect("the scene is prepared again");
    assert!(!renderer.deferred_needs_prepare());
    let frame = renderer.deferred.as_ref().expect("the frame is made");
    let size = frame
        .refraction
        .as_ref()
        .expect("a copy is made")
        .texture()
        .size();
    assert_eq!((size.width, size.height), (128, 64));
}

#[test]
fn the_key_light_is_sent_as_a_unit_way_and_a_strength_and_a_way_of_no_length_is_the_first_light() {
    let first = key_light_uniform(KEY_LIGHT_TOWARDS, KEY_LIGHT_STRENGTH);
    let way = Vec3::new(first[0], first[1], first[2]);
    assert!((way.length() - 1.0).abs() < 1e-6, "{first:?}");
    assert_eq!(first[3], KEY_LIGHT_STRENGTH);
    assert_eq!(key_light_uniform(Vec3::ZERO, KEY_LIGHT_STRENGTH), first);
    assert_eq!(
        key_light_uniform(Vec3::new(0.0, 0.0, 5.0), 2.0),
        [0.0, 0.0, 1.0, 2.0]
    );
    assert_eq!(
        key_light_uniform(Vec3::Z, -3.0)[3],
        0.0,
        "no light is not a negative one"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_skinned_surface_is_drawn_in_the_pose_it_is_given_through_the_bones_it_is_weighted_to() {
    use crate::game_shaders::mesh::{Image, Material};
    use crate::game_shaders::synthetic::quad;
    const WHAT: &str = "a_skinned_surface_is_drawn_in_the_pose_it_is_given";
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let Some(pack) = crate::game_shaders::test_pack::real_pack(WHAT) else {
        return;
    };
    // A floor weighted wholly to a bone far down the skeleton
    let mut floor = quad(0.0);
    floor.skinned = true;
    floor.joints = vec![[9, 0, 0, 0]; 4];
    // Another skinned surface, far out of the view, weighted to a bone before it: the floor's bone is the second of the table, as a bone of a ped is
    let mut spare = quad(1.0e4);
    spare.skinned = true;
    spare.joints = vec![[3, 0, 0, 0]; 4];
    spare.weights = vec![[1.0, 0.0, 0.0, 0.0]; 4];
    floor.weights = vec![[1.0, 0.0, 0.0, 0.0]; 4];
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    let material = Material {
        effect: "default".into(),
        textures: [("DiffuseSampler".to_string(), white)].into(),
        ..Material::default()
    };
    let camera = OrbitCamera {
        pitch: 1.5,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    renderer
        .prepare_deferred(
            &device,
            &queue,
            &pack,
            &[(floor, material.clone()), (spare, material)],
        )
        .expect("the scene is prepared");
    renderer.prepare_game_frame(&device, &queue);
    let middle = |renderer: &ViewportRenderer| {
        middle_pixel(&device, &queue, renderer, &camera, ShadingMode::Shaded)
    };
    // At rest the floor lies under the middle of the view, and the same under a pose that leaves its bone where it is
    let rest = middle(&renderer);
    // A pose is not a change of the targets: whoever shows the frame registers its texture anew when they are made again, and a pose moves every frame of a clip
    renderer.take_dirty();
    renderer.set_skin(vec![glam::Mat4::IDENTITY; 10]);
    assert_eq!(middle(&renderer), rest);
    assert!(
        !renderer.take_dirty(),
        "a pose flagged the targets as made again"
    );
    // A pose that carries its bone a hundred metres off carries the floor with it, and leaves the view empty
    let mut pose = vec![glam::Mat4::IDENTITY; 10];
    pose[9] = glam::Mat4::from_translation(glam::Vec3::new(100.0, 0.0, 0.0));
    renderer.set_skin(pose);
    let moved = middle(&renderer);
    let error = pollster::block_on(validation.pop());
    assert!(error.is_none(), "{error:?}");
    assert_ne!(moved, rest, "the floor stayed where it was");
    // A pose that turns the bone a quarter and moves it half a metre draws what the same turn and move of the floor's own corners draw
    let turn = glam::Mat4::from_translation(glam::Vec3::new(0.5, 0.25, 0.0))
        * glam::Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let mut pose = vec![glam::Mat4::IDENTITY; 10];
    pose[9] = turn;
    renderer.set_skin(pose);
    let posed = image_after(&device, &queue, &renderer, &camera, ShadingMode::Shaded);
    let mut carried = quad(0.0);
    for at in &mut carried.positions {
        *at = turn.transform_point3((*at).into()).into();
    }
    carried.normals = vec![turn.transform_vector3(glam::Vec3::Z).into(); 4];
    let mut still =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let plain = Material {
        effect: "default".into(),
        textures: [(
            "DiffuseSampler".to_string(),
            Image {
                width: 1,
                height: 1,
                rgba: vec![255; 4],
                srgb: false,
            },
        )]
        .into(),
        ..Material::default()
    };
    still
        .prepare_deferred(&device, &queue, &pack, &[(carried, plain)])
        .expect("the carried floor is prepared");
    still.prepare_game_frame(&device, &queue);
    let expected = image_after(&device, &queue, &still, &camera, ShadingMode::Shaded);
    assert_eq!(
        posed, expected,
        "the pose does not carry the floor as a turn and a move does"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_plain_modes_are_lit_by_the_key_light_that_was_set_and_by_the_first_one_until_then() {
    const WHAT: &str = "the_plain_modes_are_lit_by_the_key_light_that_was_set";
    let Some((device, queue)) = bytes_test_device(WHAT) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let camera = OrbitCamera {
        pitch: 1.5,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    let corners = [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    let vertices = corners.map(|at| MeshVertex::new(at, [0.0, 0.0, 1.0], [0.0, 0.0]));
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    renderer.set_meshes(vec![MeshDraw {
        bump: false,
        vertex_buffer: buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX),
        index_buffer: buffer(
            bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            wgpu::BufferUsages::INDEX,
        ),
        index_count: 6,
        texture_bind_group: None,
    }]);
    let lit = |renderer: &ViewportRenderer| {
        middle_pixel(&device, &queue, renderer, &camera, ShadingMode::Solid)[0]
    };

    let first = lit(&renderer);
    renderer.set_key_light(KEY_LIGHT_TOWARDS, KEY_LIGHT_STRENGTH);
    assert_eq!(
        lit(&renderer),
        first,
        "the light it began with is the one it is set to"
    );
    renderer.set_key_light(Vec3::Z, 1.0);
    let facing = lit(&renderer);
    renderer.set_key_light(-Vec3::Z, 1.0);
    let away = lit(&renderer);
    renderer.set_key_light(Vec3::Z, 0.0);
    let none = lit(&renderer);
    assert!(
        facing > away + 40,
        "a light on the surface lights it: {facing} against {away}"
    );
    assert_eq!(
        away, none,
        "a light behind the surface lights it as no light does"
    );
    assert!(away > 0, "the head light and the ambient light stay");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_sun_shadows_a_surface_below_a_raised_one_in_shaded_until_shadows_are_set_off() {
    use crate::game_shaders::mesh::{Image, Material};
    use crate::game_shaders::synthetic::quad;
    let test = "the_sun_shadows_a_surface_below_a_raised_one";
    let Some((device, queue)) = bytes_test_device(test) else {
        return;
    };
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return;
    };
    let pack = Arc::new(crate::game_shaders::pack::GamePack::open(root).expect("the pack opens"));
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    let material = Material {
        effect: "default".into(),
        textures: [("DiffuseSampler".to_string(), white)].into(),
        ..Material::default()
    };
    // A floor, and a top over half of it that the sun, travelling across and down, throws a shadow of onto the floor beside it.
    let (mut floor, mut top) = (quad(0.0), quad(1.0));
    top.positions = vec![
        [-0.5, -1.0, 1.0],
        [0.5, -1.0, 1.0],
        [0.5, 1.0, 1.0],
        [-0.5, 1.0, 1.0],
    ];
    for surface in [&mut floor, &mut top] {
        surface.indices.extend([0, 2, 1, 0, 3, 2]);
    }
    renderer
        .prepare_deferred(
            &device,
            &queue,
            &pack,
            &[(floor, material.clone()), (top, material)],
        )
        .expect("the deferred passes prepare");
    renderer.set_game_sun(Some(Vec3::new(1.0, 0.0, 2.0).normalize()));
    let camera = OrbitCamera {
        yaw: 0.0,
        pitch: std::f32::consts::FRAC_PI_2 - 0.2,
        ..OrbitCamera::default()
    };
    let picture = |renderer: &ViewportRenderer| {
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &queue, &camera, ShadingMode::Shaded);
        queue.submit([encoder.finish()]);
        let (size, _) = renderer.size();
        crate::game_shaders::frame::read_back(
            &device,
            &queue,
            renderer.colour_texture(),
            renderer.colour_format,
            size,
        )
        .bytes
    };
    let shadowed = picture(&renderer);
    renderer.set_shadows(false);
    let bare = picture(&renderer);
    let differing = shadowed
        .chunks(4)
        .zip(bare.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        differing > 16,
        "the shadow on the floor is drawn, and setting shadows off takes it away: {differing} pixels differ"
    );
    renderer.set_shadows(true);
    assert_eq!(
        picture(&renderer),
        shadowed,
        "shadows on again is the frame as it was"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_shaded_frame_reports_how_long_each_of_its_stages_took_on_the_device() {
    use crate::game_shaders::mesh::{Image, Material};
    use crate::game_shaders::synthetic::quad;
    let test = "a_shaded_frame_reports_how_long_each_of_its_stages_took_on_the_device";
    let Some((device, queue)) = bytes_test_device(test) else {
        return;
    };
    if !device.features().contains(crate::gpu_timer::FEATURES) {
        or_test_support::skip_device(test, format_args!("the adapter has no timestamp queries"));
        return;
    }
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return;
    };
    let pack = Arc::new(crate::game_shaders::pack::GamePack::open(root).expect("the pack opens"));
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    let surfaces = [(
        quad(0.0),
        Material {
            effect: "default".into(),
            textures: [("DiffuseSampler".to_string(), white)].into(),
            ..Material::default()
        },
    )];
    renderer
        .prepare_deferred(&device, &queue, &pack, &surfaces)
        .expect("the deferred passes prepare");
    assert!(
        renderer.gpu_stages().is_empty(),
        "nothing is timed before a frame is"
    );

    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        &mut encoder,
        &queue,
        &OrbitCamera::default(),
        ShadingMode::Shaded,
    );
    queue.submit([encoder.finish()]);
    let mut read = false;
    for _ in 0..1000 {
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if renderer.collect_gpu_times(&device) {
            read = true;
            break;
        }
    }
    assert!(read, "the timestamps were never read");

    let stages = renderer.gpu_stages();
    let names: Vec<_> = stages.iter().map(|(name, _)| *name).collect();
    for stage in ["g-buffer", "lighting", "resolve", "lines and models"] {
        assert!(names.contains(&stage), "{names:?}");
    }
    let order = |name| names.iter().position(|n| *n == name).expect("a stage");
    assert!(order("g-buffer") < order("lighting") && order("lighting") < order("resolve"));
    assert!(stages.iter().all(|(_, ms)| *ms >= 0.0));
    assert!(
        stages.iter().map(|(_, ms)| ms).sum::<f32>() > 0.0,
        "{stages:?}"
    );
}
