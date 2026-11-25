//! A species' 64 frames baked from its own model into its slot of the atlas, and the atlas's smaller levels made from them.

use std::num::NonZeroU64;

use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::viewport::MeshVertex;

// The light the world's program reads is bound all nought, so that nothing gives light of its own.
use super::super::lamps::LIGHT_FLOATS;
use super::super::pipelines::WORLD_VERTEX_ATTRIBUTES;
use super::super::{WorldBucket, WorldInstance, CAMERA_FLOATS};
use super::shader::{bake_shader, MIP_SHADER};
use super::{frame_axes, frame_direction, CardMesh, CARD_FRAMES};

/// The side of a layer of the atlas, which holds two by two species, each a grid of frames.
pub(super) const SIDE: u32 = 512;
/// The side of one frame, in texels of the atlas's first level.
const CELL: u32 = SIDE / 2 / CARD_FRAMES;
/// How many texels a side of the bake one texel of the atlas's first level is the mean of: a cutout is cut texel by texel of the bake, as a model is pixel by pixel, and the atlas keeps the share that is covered.
const SUPERSAMPLE: u32 = 4;
/// The side of the picture one species is baked into, and its levels down to the atlas's size.
const BAKED_SIDE: u32 = SIDE / 2 * SUPERSAMPLE;
const BAKED_LEVELS: u32 = SUPERSAMPLE.ilog2() + 1;
/// The levels of the atlas: the least has frames four texels a side.
pub(super) const LEVELS: u32 = 4;
/// The atlas holds linear colour stored as sRGB, with its coverage in alpha.
pub(super) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The bytes of one frame's view, laid out as the viewport's camera uniform.
const VIEW_BYTES: u64 = (CAMERA_FLOATS * size_of::<f32>()) as u64;
const FRAMES: u32 = CARD_FRAMES * CARD_FRAMES;

/// How many species the atlas holds at most on `device`: four a layer.
pub(super) fn slots(device: &wgpu::Device) -> u32 {
    device.limits().max_texture_array_layers.saturating_mul(4)
}

fn atlas_texture(device: &wgpu::Device, layers: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport-cards-atlas"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: layers,
        },
        mip_level_count: LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn every_layer(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("viewport-cards-atlas-view"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

/// One level of one layer, as a picture of its own.
fn level_of(texture: &wgpu::Texture, layer: u32, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("viewport-cards-atlas-level"),
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_mip_level: level,
        mip_level_count: Some(1),
        base_array_layer: layer,
        array_layer_count: Some(1),
        ..Default::default()
    })
}

fn whole_level(texture: &wgpu::Texture, level: u32) -> wgpu::TexelCopyTextureInfo<'_> {
    wgpu::TexelCopyTextureInfo {
        texture,
        mip_level: level,
        origin: wgpu::Origin3d::ZERO,
        aspect: wgpu::TextureAspect::All,
    }
}

/// The view a frame is baked through: the sphere about `centre` of `radius` seen along `-d`, across `right` and up `up`, its depth from nought where it is nearest to one where it is furthest.
pub(super) fn frame_view(centre: Vec3, radius: f32, d: Vec3, (right, up): (Vec3, Vec3)) -> Mat4 {
    let row = |axis: Vec3, scale: f32, offset: f32| {
        (axis * scale).extend(offset - axis.dot(centre) * scale)
    };
    Mat4::from_cols(
        row(right, 1.0 / radius, 0.0),
        row(up, 1.0 / radius, 0.0),
        row(-d, 0.5 / radius, 0.5),
        Vec4::W,
    )
    .transpose()
}

/// What bakes a species and makes the atlas's levels, and the atlas.
pub(super) struct Bake {
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    halve: wgpu::RenderPipeline,
    level_layout: wgpu::BindGroupLayout,
    /// The light the world's program is bound with, of nothing, and the one instance a mesh is baked as: where its model puts it, untinted, outdoors.
    light_group: wgpu::BindGroup,
    instance: wgpu::Buffer,
    /// The view of each frame, a step of `stride` bytes apart, read at a dynamic offset.
    views: wgpu::Buffer,
    view_group: wgpu::BindGroup,
    stride: u64,
    /// The supersampled picture a species is baked into, made the first time one is, and its depth.
    baked: Option<(wgpu::Texture, wgpu::TextureView)>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Bake {
    /// What bakes with the world's program, whose materials are bound by `materials`.
    pub(super) fn new(device: &wgpu::Device, materials: &wgpu::BindGroupLayout) -> Self {
        let fragment_texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-cards-bake-view-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(VIEW_BYTES),
                },
                count: None,
            }],
        });
        let light_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-cards-bake-light-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let light = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport-cards-bake-light"),
            contents: bytemuck::cast_slice(&[0.0f32; LIGHT_FLOATS]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let light_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-cards-bake-light-bg"),
            layout: &light_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: light.as_entire_binding(),
            }],
        });
        let instance = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport-cards-bake-instance"),
            contents: bytemuck::bytes_of(&WorldInstance::of(Mat4::IDENTITY, [1.0; 4])),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let level_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-cards-level-bgl"),
            entries: &[fragment_texture(0)],
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
        let stride = VIEW_BYTES.next_multiple_of(align);
        let views = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport-cards-bake-views"),
            size: stride * u64::from(FRAMES),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-cards-bake-view-bg"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &views,
                    offset: 0,
                    size: NonZeroU64::new(VIEW_BYTES),
                }),
            }],
        });

        let module = |label, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let bake = module("viewport-cards-bake-shader", &bake_shader());
        let mip = module("viewport-cards-mip-shader", MIP_SHADER);
        let bake_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-cards-bake-pipeline-layout"),
            bind_group_layouts: &[Some(&view_layout), Some(materials), Some(&light_layout)],
            immediate_size: 0,
        });
        let mip_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-cards-mip-pipeline-layout"),
            bind_group_layouts: &[Some(&level_layout)],
            immediate_size: 0,
        });
        let target = [Some(wgpu::ColorTargetState {
            format: FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        // The world's own vertices and instances, as its pipelines read them
        let vertices = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &WORLD_VERTEX_ATTRIBUTES,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<WorldInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    5 => Float32x4,
                    6 => Float32x4,
                    7 => Float32x4,
                    8 => Float32x4,
                    10 => Float32x2,
                    11 => Float32,
                ],
            },
        ];
        let baker = |label: &str, fragment: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&bake_layout),
                vertex: wgpu::VertexState {
                    module: &bake,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &vertices,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &bake,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &target,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let halve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport-cards-mip-pipeline"),
            layout: Some(&mip_layout),
            vertex: wgpu::VertexState {
                module: &mip,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mip,
                entry_point: Some("fs_half"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            multiview_mask: None,
            cache: None,
        });
        let texture = atlas_texture(device, 1);
        Self {
            opaque: baker("viewport-cards-bake-pipeline", "fs_bake"),
            cutout: baker("viewport-cards-bake-cutout-pipeline", "fs_bake_cutout"),
            halve,
            level_layout,
            light_group,
            instance,
            views,
            view_group,
            stride,
            baked: None,
            view: every_layer(&texture),
            texture,
        }
    }

    /// Every layer and level of the atlas.
    pub(super) fn atlas(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Make the atlas hold at least `layers`, keeping what it holds: twice as many as before where that is more, and never more than the device makes.
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layers: u32) {
        let have = self.texture.depth_or_array_layers();
        if layers <= have {
            return;
        }
        let most = device.limits().max_texture_array_layers;
        let texture = atlas_texture(device, layers.max(have.saturating_mul(2)).min(most));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport-cards-atlas-grow"),
        });
        for level in 0..LEVELS {
            encoder.copy_texture_to_texture(
                whole_level(&self.texture, level),
                whole_level(&texture, level),
                wgpu::Extent3d {
                    width: SIDE >> level,
                    height: SIDE >> level,
                    depth_or_array_layers: have,
                },
            );
        }
        queue.submit([encoder.finish()]);
        self.view = every_layer(&texture);
        self.texture = texture;
    }

    /// Draw `meshes` into the 64 frames of `slot`, each seen along its frame's way from twice the radius off into a picture four times as fine, halved down into the slot, then make the layer's smaller levels again; false where the slot is past what the device holds or the sphere has no size.
    pub(super) fn bake(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: u32,
        (centre, radius): (Vec3, f32),
        meshes: &[CardMesh<'_>],
    ) -> bool {
        let layer = slot / 4;
        if !(radius > 0.0 && radius.is_finite())
            || layer >= device.limits().max_texture_array_layers
        {
            return false;
        }
        self.grow(device, queue, layer + 1);
        let mut views = vec![0u8; (self.stride * u64::from(FRAMES)) as usize];
        for frame in 0..FRAMES {
            let d = frame_direction(frame % CARD_FRAMES, frame / CARD_FRAMES);
            let view = frame_view(centre, radius, d, frame_axes(d, Vec3::X, Vec3::Z));
            // The camera's uniform: the view and its inverse, and the eye twice the radius off along the frame's way
            let mut camera = [0.0f32; CAMERA_FLOATS];
            camera[..16].copy_from_slice(&view.to_cols_array());
            camera[16..32].copy_from_slice(&view.inverse().to_cols_array());
            camera[32..35].copy_from_slice(&(centre + d * 2.0 * radius).to_array());
            let at = (u64::from(frame) * self.stride) as usize;
            views[at..at + VIEW_BYTES as usize].copy_from_slice(bytemuck::cast_slice(&camera));
        }
        queue.write_buffer(&self.views, 0, &views);
        let (baked, depth) = self.baked.get_or_insert_with(|| baked_texture(device));
        let baked = baked.clone();
        let depth = depth.clone();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport-cards-bake"),
        });
        {
            let target = level_of(&baked, 0, 0);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-cards-bake-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
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
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let side = CELL * SUPERSAMPLE;
            for frame in 0..FRAMES {
                let (col, row) = (frame % CARD_FRAMES, frame / CARD_FRAMES);
                let (x, y) = ((col * side) as f32, (row * side) as f32);
                pass.set_viewport(x, y, side as f32, side as f32, 0.0, 1.0);
                let offset = (u64::from(frame) * self.stride) as wgpu::DynamicOffset;
                pass.set_bind_group(0, &self.view_group, &[offset]);
                pass.set_bind_group(2, &self.light_group, &[]);
                pass.set_vertex_buffer(1, self.instance.slice(..));
                for mesh in meshes {
                    pass.set_pipeline(match mesh.material.bucket {
                        WorldBucket::Cutout => &self.cutout,
                        _ => &self.opaque,
                    });
                    pass.set_bind_group(1, &*mesh.material.maps, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    let indices = mesh.first_index..mesh.first_index + mesh.index_count;
                    pass.draw_indexed(indices, mesh.base_vertex, 0..1);
                }
            }
        }
        // The bake halved down to the atlas's size, each texel the share of its own that is covered
        for level in 1..BAKED_LEVELS {
            self.halve(device, &mut encoder, (&baked, 0), level);
        }
        let side = SIDE / 2;
        encoder.copy_texture_to_texture(
            whole_level(&baked, BAKED_LEVELS - 1),
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: (slot & 1) * side,
                    y: ((slot >> 1) & 1) * side,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        for level in 1..LEVELS {
            self.halve(device, &mut encoder, (&self.texture, layer), level);
        }
        queue.submit([encoder.finish()]);
        true
    }

    /// Make level `level` of layer `layer` of `texture` from the level above it.
    fn halve(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        (texture, layer): (&wgpu::Texture, u32),
        level: u32,
    ) {
        let above = level_of(texture, layer, level - 1);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-cards-level-bg"),
            layout: &self.level_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&above),
            }],
        });
        let below = level_of(texture, layer, level);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewport-cards-level-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &below,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.halve);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The picture one species is baked into, its levels down to the atlas's size, and its depth.
fn baked_texture(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: BAKED_SIDE,
        height: BAKED_SIDE,
        depth_or_array_layers: 1,
    };
    let colour = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport-cards-baked"),
        size,
        mip_level_count: BAKED_LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport-cards-bake-depth"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    (colour, depth.create_view(&Default::default()))
}
