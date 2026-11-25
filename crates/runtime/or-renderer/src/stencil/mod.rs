use crate::viewport::OrbitCamera;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;
pub const WATER_PLANE_WGSL: &str = include_str!("water_plane.wgsl");
pub const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct WaterStencilParams {
    camera_pos: [f32; 3],
    _pad: f32,
    inv_view_proj: [[f32; 4]; 4],
}

pub struct WaterStencilPipeline {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
}

impl WaterStencilPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("water-stencil-shader"),
            source: wgpu::ShaderSource::Wgsl(WATER_PLANE_WGSL.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water-stencil-bgl"),
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
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water-stencil-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water-stencil-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_water_stencil"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: STENCIL_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, bgl }
    }

    pub fn bgl(&self) -> &wgpu::BindGroupLayout {
        &self.bgl
    }
}

pub fn build_water_stencil(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &WaterStencilPipeline,
    target: &wgpu::TextureView,
    camera: &OrbitCamera,
    aspect: f32,
) {
    let view_proj = camera.view_proj(aspect);
    build_water_stencil_with_matrix(
        device,
        queue,
        encoder,
        pipeline,
        target,
        camera.eye(),
        view_proj,
    );
}

pub fn build_water_stencil_with_matrix(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &WaterStencilPipeline,
    target: &wgpu::TextureView,
    camera_pos: Vec3,
    view_proj: Mat4,
) {
    let inv = view_proj.inverse();
    let params = WaterStencilParams {
        camera_pos: camera_pos.to_array(),
        _pad: 0.0,
        inv_view_proj: inv.to_cols_array_2d(),
    };
    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("water-stencil-params"),
        contents: bytemuck::bytes_of(&params),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("water-stencil-bg"),
        layout: pipeline.bgl(),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: params_buffer.as_entire_binding(),
        }],
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("water-stencil-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }
    let _ = (queue, params_buffer, Arc::<u8>::new(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    fn maybe_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: crate::instance::software_asked(),
            ..Default::default()
        }))
        .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("water-stencil-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .ok()
    }

    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "needs a GPU adapter: acquisition can hang, not just fail"
    )]
    #[test]
    fn proj_mode_water_clips_above_z0() {
        let Some((device, queue)) = maybe_device() else {
            or_test_support::skip_device(
                "proj_mode_water_clips_above_z0",
                "no wgpu adapter.proj_mode_water_clips_above_z0",
            );
            return;
        };
        let pipeline = WaterStencilPipeline::new(&device);

        let target_size = 64u32;
        let target_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water-stencil-target"),
            size: wgpu::Extent3d {
                width: target_size,
                height: target_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: STENCIL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target_tex.create_view(&Default::default());

        let camera_pos = Vec3::new(0.0, 0.0, 5.0);
        let view = Mat4::look_at_rh(camera_pos, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
        let view_proj = proj * view;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("water-stencil-test-enc"),
        });
        build_water_stencil_with_matrix(
            &device,
            &queue,
            &mut encoder,
            &pipeline,
            &target_view,
            camera_pos,
            view_proj,
        );

        let bytes_per_row = target_size.div_ceil(256) * 256;
        let total = (bytes_per_row as u64) * (target_size as u64);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water-stencil-readback"),
            size: total,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(target_size),
                },
            },
            wgpu::Extent3d {
                width: target_size,
                height: target_size,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let data = slice.get_mapped_range();

        let mut water_pixels = 0;
        let mut land_pixels = 0;
        for y in 0..target_size {
            for x in 0..target_size {
                let v = data[(y * bytes_per_row + x) as usize];
                if v >= 128 {
                    water_pixels += 1;
                } else {
                    land_pixels += 1;
                }
            }
        }
        assert!(
            water_pixels > land_pixels,
            "camera above water looking down should see mostly water; \
             got {water_pixels} water vs {land_pixels} land"
        );
        let total_px = (target_size * target_size) as usize;
        assert!(
            water_pixels >= total_px * 9 / 10,
            "expected >=90% water, got {water_pixels}/{total_px}"
        );
    }

    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "needs a GPU adapter: acquisition can hang, not just fail"
    )]
    #[test]
    fn proj_mode_water_zero_when_camera_looks_up() {
        let Some((device, queue)) = maybe_device() else {
            return;
        };
        let pipeline = WaterStencilPipeline::new(&device);
        let target_size = 16u32;
        let target_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water-stencil-target-sky"),
            size: wgpu::Extent3d {
                width: target_size,
                height: target_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: STENCIL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target_tex.create_view(&Default::default());
        let camera_pos = Vec3::new(0.0, 0.0, 5.0);
        let view = Mat4::look_at_rh(camera_pos, camera_pos + Vec3::Z, Vec3::Y);
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
        let view_proj = proj * view;
        let mut encoder = device.create_command_encoder(&Default::default());
        build_water_stencil_with_matrix(
            &device,
            &queue,
            &mut encoder,
            &pipeline,
            &target_view,
            camera_pos,
            view_proj,
        );
        let bpr = (target_size).div_ceil(256) * 256;
        let total = (bpr as u64) * (target_size as u64);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging-sky"),
            size: total,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(target_size),
                },
            },
            wgpu::Extent3d {
                width: target_size,
                height: target_size,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let data = slice.get_mapped_range();
        let mut max_v = 0u8;
        for y in 0..target_size {
            for x in 0..target_size {
                max_v = max_v.max(data[(y * bpr + x) as usize]);
            }
        }
        assert_eq!(max_v, 0, "camera looking up: stencil must be all zero");
    }
}
