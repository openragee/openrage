//! The sun's shadow of the ground past the cascades: a height map of the terrain marched towards the sun into the height each texel's shadow reaches, and a pass that shades what stands below it.
//!
//! ⚠ GTA VI's own way: its cascades end within a few hundred metres, and past them only the terrain shades, soft by the sun's disc and as dark as the cycle's alpha says.

use std::cell::Cell;

use bytemuck::{Pod, Zeroable};

use crate::game_shaders::deferred::{Sun, GBUFFER_FORMAT};
use crate::game_shaders::engine::EngineConstants;

/// The format each texel's shadow is kept in: the height it reaches and how far away what casts it stands.
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// How many texels towards the sun each texel looks for what casts on it.
const STEPS: u32 = 256;
/// The least angle, in radians, the sun turns through before the shadow is marched again.
const TURN: f32 = 0.002;
/// The cycle's variables the shadow is softened and darkened by.
pub const DISC_SIZE: &str = "terrain_shadows_sun_disc_size";
pub const ALPHA_FACTOR: &str = "terrain_shadows_alpha_factor";

/// A variable of the day's cycle by its name, or the default given where it states none.
pub(crate) type CycleValue<'a> = dyn Fn(&str, f32) -> f32 + 'a;

/// The terrain's heights over the world: rows from the north edge, each west to east, `texel` metres a side.
#[derive(Debug, Clone, Copy)]
pub struct TerrainGrid<'a> {
    pub west: f32,
    pub north: f32,
    pub texel: f32,
    pub width: u32,
    pub height: u32,
    pub heights: &'a [f32],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Bake {
    /// The way towards the sun, and in `w` the texel's side in metres.
    sun: [f32; 4],
    /// The texels across and down, and the steps marched.
    size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Receive {
    /// The grid's west and north edges, and one over its extent along x and y.
    grid: [f32; 4],
    /// What a distance ahead is multiplied by and has added for how whole the cascades are there, the sun's disc and the shadow's alpha.
    fade: [f32; 4],
    /// One over the frame's width and height.
    screen: [f32; 4],
}

/// A triangle over the whole target.
const FULL: &str = r#"
@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const BAKE_SHADER: &str = r#"
struct Bake { sun: vec4<f32>, size: vec4<f32> };

@group(0) @binding(0) var<uniform> bake: Bake;
@group(0) @binding(1) var heights: texture_2d<f32>;

// The highest the sun's shadow of the terrain stands over this texel, and how far away what casts it is, marched a texel at a time towards the sun.
@fragment
fn fs_bake(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let flat = length(bake.sun.xy);
    if (flat < 1e-4) {
        return vec4<f32>(-60000.0, 0.0, 0.0, 1.0);
    }
    // A row runs from the north edge, so north is up the grid.
    let way = vec2<f32>(bake.sun.x, -bake.sun.y) / flat;
    let rise = bake.sun.z / flat * bake.sun.w;
    var reach = -60000.0;
    var away = 0.0;
    let steps = u32(bake.size.z);
    for (var k = 1u; k <= steps; k++) {
        let texel = at.xy + way * f32(k);
        if (any(texel < vec2<f32>(0.0)) || any(texel >= bake.size.xy)) {
            break;
        }
        let height = textureLoad(heights, vec2<i32>(texel), 0).r - rise * f32(k);
        if (height > reach) {
            reach = height;
            away = f32(k) * bake.sun.w;
        }
    }
    return vec4<f32>(reach, away, 0.0, 1.0);
}
"#;

const RECEIVE_SHADER: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos_ws: vec4<f32>,
    camera_misc: vec4<f32>,
    key_light: vec4<f32>,
};
struct Receive { grid: vec4<f32>, fade: vec4<f32>, screen: vec4<f32> };

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> receive: Receive;
@group(1) @binding(1) var shadow: texture_2d<f32>;
@group(1) @binding(2) var clamped: sampler;
@group(1) @binding(3) var depth: texture_depth_2d;

// What of the sun reaches a pixel past the cascades, by the terrain's shadow over it: whole above the height the shadow reaches, none below, over a band the sun's disc spreads at that distance.
@fragment
fn fs_receive(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let scene = textureLoad(depth, vec2<i32>(at.xy), 0);
    if (scene <= 0.0) {
        return vec4<f32>(1.0);
    }
    let ndc = vec2<f32>(at.x * receive.screen.x * 2.0 - 1.0, 1.0 - at.y * receive.screen.y * 2.0);
    let far = camera.inv_view_proj * vec4<f32>(ndc, scene, 1.0);
    let p = far.xyz / far.w;
    let ahead = (camera.view_proj * vec4<f32>(p, 1.0)).w;
    let cascaded = clamp(ahead * receive.fade.x + receive.fade.y, 0.0, 1.0);
    let uv = vec2<f32>((p.x - receive.grid.x) * receive.grid.z, (receive.grid.y - p.y) * receive.grid.w);
    if (cascaded >= 1.0 || any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(1.0);
    }
    let shade_at = textureSampleLevel(shadow, clamped, uv, 0.0);
    let band = max(shade_at.g * receive.fade.z, 1.0);
    let lit = clamp((p.z - shade_at.r) / band + 0.5, 0.0, 1.0);
    let shade = 1.0 - receive.fade.w * (1.0 - lit);
    return vec4<f32>(mix(shade, 1.0, cascaded));
}
"#;

/// The terrain's shadow: its heights, the shadow marched from them for the last sun, and the passes that march and receive it.
pub(crate) struct TerrainShadow {
    bake: wgpu::RenderPipeline,
    receive: wgpu::RenderPipeline,
    receive_layout: wgpu::BindGroupLayout,
    bake_uniform: wgpu::Buffer,
    receive_uniform: wgpu::Buffer,
    bake_group: wgpu::BindGroup,
    shadow: wgpu::TextureView,
    clamped: wgpu::Sampler,
    grid: [f32; 4],
    texel: f32,
    size: [u32; 2],
    /// The way towards the sun the shadow was last marched for.
    marched: Cell<Option<glam::Vec3>>,
    device: wgpu::Device,
}

impl TerrainShadow {
    /// The passes and textures for `grid`, read with the camera's layout `camera`; `None` where the grid is empty or its heights do not fill it.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &wgpu::BindGroupLayout,
        grid: &TerrainGrid<'_>,
    ) -> Option<Self> {
        let texels = grid.width as usize * grid.height as usize;
        let limit = device.limits().max_texture_dimension_2d;
        let fits = grid.width > 0
            && grid.height > 0
            && grid.width <= limit
            && grid.height <= limit
            && grid.heights.len() == texels
            && grid.texel > 0.0;
        if !fits {
            return None;
        }
        let texture = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: grid.width,
                    height: grid.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        // ⚠ Half floats: a quarter of a metre at 500 m, under the least band a shadow softens over.
        let halves: Vec<u8> = (grid.heights.iter())
            .flat_map(|h| or_io::f32_to_half(*h).to_le_bytes())
            .collect();
        let heights = texture(
            "world-terrain-heights",
            wgpu::TextureFormat::R16Float,
            wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            heights.as_image_copy(),
            &halves,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(grid.width * 2),
                rows_per_image: Some(grid.height),
            },
            heights.size(),
        );
        let shadow = texture(
            "world-terrain-shadow",
            SHADOW_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
        .create_view(&Default::default());
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let texture_of = |sample_type| wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let bake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world-terrain-bake-bgl"),
            entries: &[
                entry(0, fragment, uniform),
                entry(
                    1,
                    fragment,
                    texture_of(wgpu::TextureSampleType::Float { filterable: false }),
                ),
            ],
        });
        let receive_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world-terrain-receive-bgl"),
            entries: &[
                entry(0, fragment, uniform),
                entry(
                    1,
                    fragment,
                    texture_of(wgpu::TextureSampleType::Float { filterable: true }),
                ),
                entry(
                    2,
                    fragment,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(3, fragment, texture_of(wgpu::TextureSampleType::Depth)),
            ],
        });
        let buffer = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let bake_uniform = buffer("world-terrain-bake", std::mem::size_of::<Bake>());
        let receive_uniform = buffer("world-terrain-receive", std::mem::size_of::<Receive>());
        let heights_view = heights.create_view(&Default::default());
        let bake_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world-terrain-bake-bg"),
            layout: &bake_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bake_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&heights_view),
                },
            ],
        });
        let module = |label, stage: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{FULL}{stage}").into()),
            })
        };
        let (baking, receiving) = (
            module("world-terrain-bake-shader", BAKE_SHADER),
            module("world-terrain-receive-shader", RECEIVE_SHADER),
        );
        let pipeline = |label,
                        shader: &wgpu::ShaderModule,
                        layouts: &[Option<&wgpu::BindGroupLayout>],
                        entry_point,
                        target| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    targets: &[Some(target)],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let bake = pipeline(
            "world-terrain-bake",
            &baking,
            &[Some(&bake_layout)],
            "fs_bake",
            wgpu::ColorTargetState {
                format: SHADOW_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
        );
        // As the game's own pass: only the alpha, as what was there times what the sun lets through.
        let receive = pipeline(
            "world-terrain-receive",
            &receiving,
            &[Some(camera), Some(&receive_layout)],
            "fs_receive",
            wgpu::ColorTargetState {
                format: GBUFFER_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent::REPLACE,
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::SrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALPHA,
            },
        );
        let size = [
            grid.width as f32 * grid.texel,
            grid.height as f32 * grid.texel,
        ];
        Some(Self {
            bake,
            receive,
            receive_layout,
            bake_uniform,
            receive_uniform,
            bake_group,
            shadow,
            clamped: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("world-terrain-sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            grid: [grid.west, grid.north, 1.0 / size[0], 1.0 / size[1]],
            texel: grid.texel,
            size: [grid.width, grid.height],
            marched: Cell::new(None),
            device: device.clone(),
        })
    }

    /// Shade the self-shadow of the G-buffer, `shadows`, by the terrain's shadow where the cascades, which end `fade` gives, do not reach: marched again first where the sun of `engine` turned. `day` gives the cycle's variables; nothing is drawn where the sun does not shine.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        engine: &EngineConstants,
        (camera, depth): (&wgpu::BindGroup, &wgpu::TextureView),
        shadows: &wgpu::TextureView,
        (fade, day): ([f32; 2], &CycleValue<'_>),
        (width, height): (u32, u32),
    ) {
        let sun = Sun::of(engine);
        let towards = -sun.travel.normalize_or_zero();
        if !sun.shines() || towards.z <= 0.0 {
            return;
        }
        let turned = (self.marched.get()).is_none_or(|was| was.dot(towards) < TURN.cos());
        if turned {
            self.marched.set(Some(towards));
            let bake = Bake {
                sun: [towards.x, towards.y, towards.z, self.texel],
                size: [self.size[0] as f32, self.size[1] as f32, STEPS as f32, 0.0],
            };
            queue.write_buffer(&self.bake_uniform, 0, bytemuck::bytes_of(&bake));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world-terrain-bake"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.shadow,
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
            pass.set_pipeline(&self.bake);
            pass.set_bind_group(0, &self.bake_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let receive = Receive {
            grid: self.grid,
            fade: [
                fade[0],
                fade[1],
                day(DISC_SIZE, 0.0).max(0.0),
                day(ALPHA_FACTOR, 1.0).clamp(0.0, 1.0),
            ],
            screen: [
                1.0 / width.max(1) as f32,
                1.0 / height.max(1) as f32,
                0.0,
                0.0,
            ],
        };
        queue.write_buffer(&self.receive_uniform, 0, bytemuck::bytes_of(&receive));
        let depth = depth.texture().create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world-terrain-receive-bg"),
            layout: &self.receive_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.receive_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.clamped),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world-terrain-receive"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: shadows,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.receive);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl crate::viewport::ViewportRenderer {
    /// The terrain's heights the sun's shadow past the cascades is marched over; `None`, or a grid the device cannot hold, shades nothing past them.
    pub fn set_world_terrain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: Option<TerrainGrid<'_>>,
    ) {
        let camera = self.camera_bind_group_layout();
        self.world.terrain =
            grid.and_then(|grid| TerrainShadow::new(device, queue, &camera, &grid));
    }

    /// Whether the terrain's heights were given and taken.
    pub fn has_world_terrain(&self) -> bool {
        self.world.terrain.is_some()
    }
}

#[cfg(test)]
#[path = "shadow_terrain_tests.rs"]
mod tests;
