//! The terrain's shadow: its programs, and a wall of the terrain shading the ground past it.

use super::*;

#[test]
fn the_terrain_shadow_programs_are_valid_and_read_their_uniforms_at_the_size_written() {
    for (stage, name, size) in [
        (BAKE_SHADER, "Bake", std::mem::size_of::<Bake>()),
        (RECEIVE_SHADER, "Receive", std::mem::size_of::<Receive>()),
    ] {
        let source = format!("{FULL}{stage}");
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{name} parses: {}", e.emit_to_string(&source)));
        let flags = naga::valid::ValidationFlags::all();
        naga::valid::Validator::new(flags, naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name} validates: {e:?}"));
        let declared = (module.types.iter())
            .find(|(_, t)| t.name.as_deref() == Some(name))
            .map(|(_, t)| t.inner.size(module.to_ctx()));
        assert_eq!(declared, Some(size as u32), "{name}");
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_wall_of_the_terrain_shades_the_ground_on_its_far_side_from_the_sun_as_far_as_its_height_reaches(
) {
    use glam::{Mat4, Vec3, Vec4};
    const SIDE: u32 = 64;
    let Some((device, queue)) = crate::game_shaders::test_pack::gpu("terrain_shadow") else {
        return;
    };
    // A wall 20 m high from x = 30 to 33 over flat ground, a texel a metre; the sun low in the west, its rise half its run.
    let heights: Vec<f32> = (0..SIDE * SIDE)
        .map(|k| match k % SIDE {
            30..=32 => 20.0,
            _ => 0.0,
        })
        .collect();
    let grid = TerrainGrid {
        west: 0.0,
        north: SIDE as f32,
        texel: 1.0,
        width: SIDE,
        height: SIDE,
        heights: &heights,
    };
    let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let terrain = TerrainShadow::new(&device, &queue, &camera_layout, &grid).expect("a grid");
    // Straight down onto the grid, which fills the frame.
    let half = SIDE as f32 / 2.0;
    let eye = Vec3::new(half, half, 100.0);
    let view = Mat4::look_at_rh(eye, eye.with_z(0.0), Vec3::Y);
    let view_proj = Mat4::orthographic_rh(-half, half, -half, half, 1.0, 200.0) * view;
    let mut floats = [0.0f32; 44];
    floats[..16].copy_from_slice(&view_proj.to_cols_array());
    floats[16..32].copy_from_slice(&view_proj.inverse().to_cols_array());
    let uniform = wgpu::util::DeviceExt::create_buffer_init(
        &device,
        &wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&floats),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &camera_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let target = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: usage | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let shadows = target(GBUFFER_FORMAT, wgpu::TextureUsages::COPY_SRC);
    let depth = target(
        crate::game_shaders::deferred::DEPTH_STENCIL_FORMAT,
        wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let (shadows_view, depth_view) = (
        shadows.create_view(&Default::default()),
        depth.create_view(&Default::default()),
    );
    // The ground everywhere: the depth a point of it has.
    let ground = view_proj * Vec4::new(0.0, 0.0, 0.0, 1.0);
    let mut encoder = device.create_command_encoder(&Default::default());
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &shadows_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(ground.z / ground.w),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    let engine = EngineConstants {
        sun_direction: Vec3::new(-2.0, 0.0, 1.0).normalize(),
        sun_colour: Vec3::ONE,
        lighting: None,
        ..EngineConstants::default()
    };
    let day = |name: &str, default: f32| match name {
        ALPHA_FACTOR => 1.0,
        _ => default,
    };
    terrain.record(
        &queue,
        &mut encoder,
        &engine,
        (&camera, &depth_view),
        &shadows_view,
        ([0.0; 2], &day),
        (SIDE, SIDE),
    );
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIDE * SIDE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        shadows.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIDE * 4),
                rows_per_image: Some(SIDE),
            },
        },
        shadows.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device finishes");
    let bytes = read.slice(..).get_mapped_range().to_vec();
    // The alpha of the middle row at column x.
    let alpha = |x: u32| bytes[((SIDE / 2 * SIDE + x) * 4 + 3) as usize];
    for x in [2, 10, 20, 27] {
        assert_eq!(alpha(x), 255, "lit on the sun's side, at {x}");
    }
    // 20 m high at a rise of a half, the shadow runs 40 m past the wall.
    for x in [36, 45, 55, 60] {
        assert_eq!(alpha(x), 0, "shaded past the wall, at {x}");
    }
}
