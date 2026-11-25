//! GTA VI's water over the frame the game's passes lit: a mesh of its own, drawn over a copy of
//! that frame by its depth along the ray, the day's medium, the sky it reflects and the day's fog.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::shadow::CASCADES;
use crate::game_shaders::deferred::{DEPTH_STENCIL_FORMAT, HDR_FORMAT};
use crate::game_shaders::engine::EngineConstants;
use crate::game_shaders::shadow::MAP_FORMAT;
use crate::game_shaders::stand_in::{constant_view, row_view};
use crate::viewport::{MeshVertex, ViewportRenderer};

use super::water_shader::WATER_SHADER;

/// A far depth, for a frame that made none: the ray then runs to the ground under the water.
const FAR: f32 = 60_000.0;
/// The depth over which a shore fades in, in metres.
const SHORE: f32 = 0.5;
/// How many waves the surface sums.
pub const WAVES: usize = 12;

/// What the water is made of, as the day states it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterLook {
    /// How much of each channel a metre of water takes out.
    pub extinction: [f32; 3],
    /// The share of each channel taken out that the water gives back as its own colour.
    pub albedo: [f32; 3],
    /// What the sky it reflects is multiplied by.
    pub reflection: f32,
    /// How far each channel's scattering leans forward, Henyey and Greenstein's asymmetry: nought scatters evenly.
    pub phase: [f32; 3],
    /// The sea past the quads, where the level states it.
    pub sea: Option<OpenSea>,
}

/// The sea past the level's quads, drawn as a plane out to the horizon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenSea {
    /// The quads' bounds: west, south, east and north, in metres.
    pub bounds: [f32; 4],
    /// The height the sea stands at past them.
    pub height: f32,
    /// Where the quads are drawn about, and how far from it in metres: past that the sea runs on inside the bounds too.
    pub drawn: [f32; 3],
}

/// The medium of the water by place: two grids of texels over the world, each row from the north edge, each west to east; off the grid the water is its `WaterLook`.
#[derive(Debug, Clone, Copy)]
pub struct WaterMedia<'a> {
    /// The grid's west and north edges and its extent along x and y, in metres.
    pub west: f32,
    pub north: f32,
    pub size: [f32; 2],
    pub width: u32,
    pub height: u32,
    /// How much of each channel a metre of water takes out.
    pub extinction: &'a [[f32; 3]],
    /// How much of each channel a metre of water scatters.
    pub scattering: &'a [[f32; 3]],
}

/// The program's `Water` uniform.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WaterUniform {
    extinction: [f32; 4],
    albedo: [f32; 4],
    light: [f32; 4],
    sun: [f32; 4],
    phase: [f32; 4],
    screen: [f32; 4],
    /// The medium's grid: its west and north edges and one over its extent; nought in `z` where there is none.
    grid: [f32; 4],
    /// The finer medium's grid about the camera, the same way.
    near: [f32; 4],
    bounds: [f32; 4],
    /// The height of the sea past the quads, and in `y` one where it is drawn.
    sea: [f32; 4],
    /// Where the quads are drawn about and how far from it.
    drawn: [f32; 4],
    fog: [[f32; 4]; 5],
    fog_colour: [[f32; 4]; 4],
    /// Each wave's number a metre along x and y, its height and its phase.
    waves: [[f32; 4]; WAVES],
    /// The world to each cascade of the sun's shadow, and in `shadow_on.x` one where they are cast.
    shadow: [[[f32; 4]; 4]; CASCADES],
    shadow_on: [f32; 4],
}

/// A grid of the medium as half floats, four to a texel.
fn halves(texels: &[[f32; 3]]) -> Vec<u8> {
    (texels.iter())
        .flat_map(|t| [t[0], t[1], t[2], 0.0])
        .flat_map(|v| or_io::f32_to_half(v).to_le_bytes())
        .collect()
}

/// The water's pipeline, what it reads, and the mesh of this frame.
pub(crate) struct WaterPass {
    pipeline: wgpu::RenderPipeline,
    /// The sea past the quads, over the whole frame.
    sea: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    clamped: wgpu::Sampler,
    /// What the sun's shadow is read with, and the map read where none is cast.
    comparing: wgpu::Sampler,
    no_shadow: wgpu::TextureView,
    fresnel: wgpu::TextureView,
    far: wgpu::TextureView,
    flat: wgpu::TextureView,
    waves: [[f32; 4]; WAVES],
    mesh: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    look: WaterLook,
    inputs: Option<wgpu::BindGroup>,
    /// The medium by place: its extinction and scattering, the grid, and the texels still to upload.
    media: Option<Media>,
    /// The same about the camera, finer.
    near: Option<Media>,
}

struct Media {
    extinction: (wgpu::Texture, wgpu::TextureView),
    scattering: (wgpu::Texture, wgpu::TextureView),
    grid: [f32; 4],
    pending: Option<(Vec<u8>, Vec<u8>)>,
}

impl WaterPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &wgpu::BindGroupLayout,
        light: &wgpu::BindGroupLayout,
    ) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-water-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                texture(2),
                texture(3),
                texture(4),
                sampler(6),
                texture(8),
                texture(9),
                texture(10),
                texture(11),
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport-water-shader"),
            source: wgpu::ShaderSource::Wgsl(WATER_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-water-pipeline-layout"),
            bind_group_layouts: &[Some(camera), Some(&layout), Some(light)],
            immediate_size: 0,
        });
        // The position, and the colour after the normal and the texture coordinate.
        let attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: std::mem::offset_of!(MeshVertex, colour) as wgpu::BufferAddress,
                shader_location: 3,
            },
        ];
        let mesh = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        }];
        let pipeline_of = |label: &'static str,
                           (vertex, fragment): (&'static str, &'static str),
                           buffers: &[wgpu::VertexBufferLayout<'_>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState::default(),
                // The nearer a surface the greater its depth; the water writes none, so what blends after it still shows.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_STENCIL_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                // It replaces what it covers, which it read from a copy, and keeps the word the frame holds in its alpha.
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = pipeline_of(
            "viewport-water-pipeline",
            ("vs_water", "fs_water"),
            &mesh[..],
        );
        let sea = pipeline_of("viewport-water-sea-pipeline", ("vs_sea", "fs_sea"), &[]);
        let sampler_of = |address_mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("viewport-water-sampler"),
                address_mode_u: address_mode,
                address_mode_v: address_mode,
                address_mode_w: address_mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        Self {
            pipeline,
            sea,
            layout,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport-water-uniform"),
                size: std::mem::size_of::<WaterUniform>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            clamped: sampler_of(wgpu::AddressMode::ClampToEdge),
            comparing: crate::game_shaders::samplers::comparing(device),
            no_shadow: crate::game_shaders::targets::render_target(
                device,
                "viewport-water-no-shadow",
                (1, 1),
                MAP_FORMAT,
                wgpu::TextureUsages::empty(),
            ),
            fresnel: row_view(device, queue, &crate::game_shaders::water::blend_row()),
            far: constant_view(device, queue, [FAR, 0.0, 0.0, 0.0]),
            flat: constant_view(device, queue, [0.5, 0.5, 1.0, 1.0]),
            waves: [[0.0; 4]; WAVES],
            mesh: None,
            look: WaterLook {
                extinction: [0.0; 3],
                albedo: [0.0; 3],
                reflection: 1.0,
                phase: [0.0; 3],
                sea: None,
            },
            inputs: None,
            media: None,
            near: None,
        }
    }

    /// The medium by place, uploaded with the next frame's constants; `None` leaves the day's one medium everywhere.
    fn set_media(&mut self, device: &wgpu::Device, media: Option<WaterMedia<'_>>) {
        self.media = media_of(device, media);
    }

    /// The medium about the camera at the atlas's own size, over the coarse one; `None` leaves the coarse one everywhere.
    fn set_near(&mut self, device: &wgpu::Device, media: Option<WaterMedia<'_>>) {
        self.near = media_of(device, media);
    }
}

/// The textures of `media`, its texels still to upload; `None` where it has none or they do not fill its grid.
fn media_of(device: &wgpu::Device, media: Option<WaterMedia<'_>>) -> Option<Media> {
    media
        .filter(|m| {
            let texels = m.width as usize * m.height as usize;
            texels > 0 && m.extinction.len() == texels && m.scattering.len() == texels
        })
        .map(|m| {
            let texture = || {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("viewport-water-medium"),
                    size: wgpu::Extent3d {
                        width: m.width,
                        height: m.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                (texture, view)
            };
            Media {
                extinction: texture(),
                scattering: texture(),
                grid: [m.west, m.north, 1.0 / m.size[0], 1.0 / m.size[1]],
                pending: Some((halves(m.extinction), halves(m.scattering))),
            }
        })
}

impl WaterPass {
    /// The mesh to draw and what it is made of; `None` draws no water.
    fn set(&mut self, device: &wgpu::Device, water: Option<(&[MeshVertex], &[u32], WaterLook)>) {
        match water {
            Some((_, _, look)) => self.look = look,
            None => self.look.sea = None,
        }
        let mesh = water.map(|(vertices, indices, _)| (vertices, indices));
        self.mesh = mesh
            .filter(|(_, indices)| !indices.is_empty())
            .map(|(vertices, indices)| {
                let buffer = |label, contents: &[u8], usage| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents,
                        usage,
                    })
                };
                (
                    buffer(
                        "viewport-water-vertices",
                        bytemuck::cast_slice(vertices),
                        wgpu::BufferUsages::VERTEX,
                    ),
                    buffer(
                        "viewport-water-indices",
                        bytemuck::cast_slice(indices),
                        wgpu::BufferUsages::INDEX,
                    ),
                    indices.len() as u32,
                )
            });
    }

    /// Whether there is water to draw, which then reads a copy of the frame and its depth.
    pub(crate) fn drawn(&self) -> bool {
        self.mesh.is_some() || self.look.sea.is_some()
    }

    /// Bind what the water reads this frame: the copy of the lit frame, its depth where the frame made one, the map of the sky, and the map of the sun's shadow where one is cast.
    pub(crate) fn bind(
        &mut self,
        device: &wgpu::Device,
        behind: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        sky: &wgpu::TextureView,
        shadow: Option<&wgpu::TextureView>,
    ) {
        let views = [behind, depth.unwrap_or(&self.far), sky, &self.fresnel];
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: self.uniform.as_entire_binding(),
        }];
        entries.extend(
            (1u32..)
                .zip(views)
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        );
        entries.push(wgpu::BindGroupEntry {
            binding: 6,
            resource: wgpu::BindingResource::Sampler(&self.clamped),
        });
        // Without a medium by place the program reads the day's one medium, and these are not read.
        fn media_views<'a>(
            media: Option<&'a Media>,
            flat: &'a wgpu::TextureView,
        ) -> (&'a wgpu::TextureView, &'a wgpu::TextureView) {
            media.map_or((flat, flat), |m| (&m.extinction.1, &m.scattering.1))
        }
        let ((extinction, scattering), (near_extinction, near_scattering)) = (
            media_views(self.media.as_ref(), &self.flat),
            media_views(self.near.as_ref(), &self.flat),
        );
        for (binding, view) in [
            (8, extinction),
            (9, scattering),
            (10, near_extinction),
            (11, near_scattering),
        ] {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 12,
            resource: wgpu::BindingResource::TextureView(shadow.unwrap_or(&self.no_shadow)),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 13,
            resource: wgpu::BindingResource::Sampler(&self.comparing),
        });
        self.inputs = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-water-bg"),
            layout: &self.layout,
            entries: &entries,
        }));
    }

    /// Upload the medium by place where it is new.
    pub(crate) fn upload(&mut self, queue: &wgpu::Queue) {
        for media in [&mut self.media, &mut self.near].into_iter().flatten() {
            if let Some((extinction, scattering)) = media.pending.take() {
                for ((texture, _), bytes) in [
                    (&media.extinction, extinction),
                    (&media.scattering, scattering),
                ] {
                    let size = texture.size();
                    queue.write_texture(
                        texture.as_image_copy(),
                        &bytes,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(size.width * 8),
                            rows_per_image: Some(size.height),
                        },
                        size,
                    );
                }
            }
        }
    }

    /// Write what the water is lit and fogged by this frame, for a frame `width` by `height`, with the world to each cascade of the sun's shadow where it is cast.
    pub(crate) fn write(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        (width, height): (u32, u32),
        shadow: Option<[glam::Mat4; CASCADES]>,
    ) {
        let rgb = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
        // Without a moment of the day there is no fog: nothing hides the water.
        let (fog, fog_colour) = match &engine.lighting {
            Some(lighting) => (
                lighting.fog_params.map(|p| p.to_array()),
                lighting.fog_colours.map(|c| c.to_array()),
            ),
            None => ([[0.0; 4]; 5], [[0.0; 4]; 4]),
        };
        // The sky's light and the sun's, apart: the sun's alone is turned by the phase.
        let (sun, ambient) = match &engine.lighting {
            Some(lighting) => (
                lighting.directional_colour.truncate() * (-lighting.directional_light.z).max(0.0),
                lighting.natural_ambient[1].truncate(),
            ),
            None => (
                engine.sun_colour * engine.sun_direction.z.max(0.0),
                engine.ambient,
            ),
        };
        let sea = self.look.sea;
        let uniform = WaterUniform {
            extinction: rgb(self.look.extinction, 0.0),
            albedo: rgb(self.look.albedo, self.look.reflection),
            light: rgb(ambient.to_array(), SHORE),
            sun: rgb(sun.to_array(), 0.0),
            phase: rgb(self.look.phase, 0.0),
            screen: [
                1.0 / width.max(1) as f32,
                1.0 / height.max(1) as f32,
                0.0,
                0.0,
            ],
            grid: self.media.as_ref().map_or([0.0; 4], |m| m.grid),
            near: self.near.as_ref().map_or([0.0; 4], |m| m.grid),
            bounds: sea.map_or([0.0; 4], |s| s.bounds),
            sea: sea.map_or([0.0; 4], |s| [s.height, 1.0, 0.0, 0.0]),
            drawn: sea.map_or([0.0; 4], |s| [s.drawn[0], s.drawn[1], s.drawn[2], 0.0]),
            fog,
            fog_colour,
            waves: self.waves,
            shadow: shadow.map_or([[[0.0; 4]; 4]; CASCADES], |views| {
                views.map(|v| v.to_cols_array_2d())
            }),
            shadow_on: [f32::from(u8::from(shadow.is_some())), 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    /// Draw the water into `pass`, seen through `camera` and lit by `light`.
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        camera: &wgpu::BindGroup,
        light: &wgpu::BindGroup,
    ) {
        let Some(inputs) = &self.inputs else {
            return;
        };
        let bind = |pass: &mut wgpu::RenderPass<'_>, pipeline| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, camera, &[]);
            pass.set_bind_group(1, inputs, &[]);
            pass.set_bind_group(2, light, &[]);
        };
        if let Some((vertices, indices, count)) = &self.mesh {
            bind(pass, &self.pipeline);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..*count, 0, 0..1);
        }
        if self.look.sea.is_some() {
            bind(pass, &self.sea);
            pass.draw(0..3, 0..1);
        }
    }
}

impl ViewportRenderer {
    /// The water the game's frame draws over what it lit: its mesh, each vertex's colour holding in red how deep the ground lies under it, and what it is made of; `None` draws none.
    pub fn set_world_water(
        &mut self,
        device: &wgpu::Device,
        water: Option<(&[MeshVertex], &[u32], WaterLook)>,
    ) {
        self.world.water.set(device, water);
    }

    /// The waves the water's surface sums this frame: each one's number a metre along x and y, its height and its phase.
    pub fn set_world_waves(&mut self, waves: [[f32; 4]; WAVES]) {
        self.world.water.waves = waves;
    }

    /// The water's medium by place; `None` leaves the day's one medium everywhere.
    pub fn set_world_water_media(&mut self, device: &wgpu::Device, media: Option<WaterMedia<'_>>) {
        self.world.water.set_media(device, media);
    }

    /// The water's medium about the camera at the atlas's own size, over the coarse one; `None` leaves the coarse one everywhere.
    pub fn set_world_water_near(&mut self, device: &wgpu::Device, media: Option<WaterMedia<'_>>) {
        self.world.water.set_near(device, media);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_water_program_is_valid_and_its_uniform_is_the_size_the_renderer_writes() {
        let module = naga::front::wgsl::parse_str(WATER_SHADER).expect("the WGSL parses");
        let flags = naga::valid::ValidationFlags::all();
        let mut validator = naga::valid::Validator::new(flags, naga::valid::Capabilities::all());
        validator
            .validate(&module)
            .expect("the program is valid, its derivatives in uniform control flow");
        let water = (module.types.iter())
            .find(|(_, t)| t.name.as_deref() == Some("Water"))
            .map(|(_, t)| t.inner.size(module.to_ctx()))
            .expect("a Water uniform");
        assert_eq!(water as usize, std::mem::size_of::<WaterUniform>());
    }
}
