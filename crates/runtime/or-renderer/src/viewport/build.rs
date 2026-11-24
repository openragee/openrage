//! How a viewport renderer is built: its pipelines, bind groups and targets.

use super::*;

impl ViewportRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        colour_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let (width, height) = (width.max(1), height.max(1));

        let camera_uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            inv_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos_ws: [0.0, 0.0, 0.0, 1.0],
            camera_misc: [0.05, 5000.0, width as f32, height as f32],
            key_light: key_light_uniform(KEY_LIGHT_TOWARDS, KEY_LIGHT_STRENGTH),
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport-camera-uniform"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bgl = Arc::new(device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("viewport-camera-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            },
        ));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-camera-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport-line-shader"),
            source: wgpu::ShaderSource::Wgsl(LINE_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-line-pipeline-layout"),
            bind_group_layouts: &[Some(&camera_bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport-line-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x4,
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: colour_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let material_bgl = Arc::new(device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("viewport-material-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            },
        ));

        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport-mesh-shader"),
            source: wgpu::ShaderSource::Wgsl(MESH_SHADER.into()),
        });

        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-mesh-pipeline-layout"),
            bind_group_layouts: &[Some(&camera_bgl), Some(&material_bgl)],
            immediate_size: 0,
        });

        let mesh_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_VERTEX_ATTRIBUTES,
        };

        let make_mesh_pipeline = |label: &str, fs_entry: &str, depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(&mesh_vertex_layout),
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &mesh_shader,
                    entry_point: Some(fs_entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: colour_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let solid_pipeline = make_mesh_pipeline("viewport-solid-pipeline", "fs_solid", true);
        let material_pipeline =
            make_mesh_pipeline("viewport-material-pipeline", "fs_material", true);
        let material_blend_pipeline = make_mesh_pipeline(
            "viewport-material-blend-pipeline",
            "fs_material_blend",
            false,
        );
        let bump_pipeline = make_mesh_pipeline("viewport-bump-pipeline", "fs_bump", true);
        let channel_pipeline = make_mesh_pipeline("viewport-channel-pipeline", "fs_channel", true);

        let (fallback_material_bg, fallback_material_bytes) =
            create_fallback_material_bind_group(device, queue, &material_bgl);

        let world = crate::world::World::new(device, &camera_bgl, &material_bgl, colour_format);

        let (colour, colour_view, colour_display_view, depth_view) =
            create_targets(device, colour_format, width, height);
        let front_line_pipeline = build_overlay_line_pipeline(
            device,
            &camera_bgl,
            colour_format,
            wgpu::CompareFunction::Always,
        );
        Self {
            colour_format,
            width,
            height,
            dirty: true,
            colour,
            colour_view,
            colour_display_view,
            depth_view,
            pipeline,
            bind_group,
            camera_buffer,
            camera_bgl,
            line_buffer: None,
            line_count: 0,
            lines_in_front: 0,
            line_buffer_capacity: 0,
            front_line_pipeline,
            solid_pipeline,
            material_pipeline,
            material_blend_pipeline,
            bump_pipeline,
            channel_pipeline,
            material_bgl,
            fallback_material_bg: Arc::new(fallback_material_bg),
            fallback_material_bytes,
            meshes: Vec::new(),
            world,
            game_scene: None,
            deferred: None,
            deferred_scene: None,
            refracts: false,
            refraction_stale: false,
            deferred_pipelines: Default::default(),
            game_light: {
                let day = crate::game_shaders::engine::EngineConstants::default();
                (day.sun_colour, day.ambient)
            },
            game_sun: None,
            key_light: (
                crate::viewport::KEY_LIGHT_TOWARDS,
                crate::viewport::KEY_LIGHT_STRENGTH,
            ),
            sky_background: false,
            shadows: true,
            day: None,
            day_revision: 0,
            game_particles: None,
            gpu_timer: crate::gpu_timer::GpuTimer::new(device, queue),
            particle_batches: Vec::new(),
            trail_batches: Vec::new(),
            model_batches: Vec::new(),
            point_lights: Vec::new(),
            spot_lights: Vec::new(),
            fog_volumes: Vec::new(),
            sky_textures: Default::default(),
            water_textures: Default::default(),
            skin: Vec::new(),
            background: CLEAR_COLOR,
        }
    }
}
