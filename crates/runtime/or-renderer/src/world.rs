//! The world a viewport draws: meshes on the device, each drawn once for every instance that places it.

use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::game_shaders::deferred::{DEPTH_STENCIL_FORMAT, GBUFFER_FORMAT, GBUFFER_TARGETS};
use crate::game_shaders::mesh::scene::{writes_material, MATERIAL_DEFAULT};
use crate::viewport::{MeshVertex, ViewportRenderer, MESH_VERTEX_ATTRIBUTES};

/// One mesh of the world on the device: its vertices, indices and the texture its material draws.
pub struct WorldMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    texture: Option<Arc<wgpu::BindGroup>>,
}

impl WorldMesh {
    /// Upload a mesh; `texture` is a bind group of [`ViewportRenderer::material_bind_group_layout`], and none draws it white.
    pub fn new(
        device: &wgpu::Device,
        vertices: &[MeshVertex],
        indices: &[u32],
        texture: Option<Arc<wgpu::BindGroup>>,
    ) -> Self {
        let buffer = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        Self {
            vertices: buffer(
                "world-mesh-vertices",
                bytemuck::cast_slice(vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: buffer(
                "world-mesh-indices",
                bytemuck::cast_slice(indices),
                wgpu::BufferUsages::INDEX,
            ),
            index_count: indices.len() as u32,
            texture,
        }
    }

    /// The bytes of device memory its buffers hold.
    pub fn bytes(&self) -> u64 {
        self.vertices.size() + self.indices.size()
    }

    /// The same mesh drawn with `texture`, sharing its buffers.
    pub fn with_texture(&self, texture: Arc<wgpu::BindGroup>) -> Self {
        Self {
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
            index_count: self.index_count,
            texture: Some(texture),
        }
    }
}

/// Where one instance of a mesh stands: the rows of its 3×4 model matrix, and a colour its surface is multiplied by.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WorldInstance {
    pub rows: [[f32; 4]; 3],
    pub tint: [f32; 4],
}

impl WorldInstance {
    /// The instance `matrix` places, its surface multiplied by `tint`.
    pub fn of(matrix: glam::Mat4, tint: [f32; 4]) -> Self {
        Self {
            rows: [0, 1, 2].map(|row| matrix.row(row).to_array()),
            tint,
        }
    }
}

/// A run of the frame's instances drawn with one mesh.
pub struct WorldDraw {
    pub mesh: Arc<WorldMesh>,
    pub instances: Range<u32>,
}

/// What a viewport keeps of the world: the pipelines that draw it, the instances of this frame and the runs of them each mesh draws.
pub(crate) struct World {
    forward: wgpu::RenderPipeline,
    gbuffer: wgpu::RenderPipeline,
    /// One buffer for every frame, made again only when a frame has more instances than it holds.
    instances: Option<wgpu::Buffer>,
    count: u32,
    draws: Vec<WorldDraw>,
}

impl World {
    /// The pipelines for a viewport whose colour target is of `colour_format`, with its camera and material layouts.
    pub(crate) fn new(
        device: &wgpu::Device,
        camera_bgl: &wgpu::BindGroupLayout,
        material_bgl: &wgpu::BindGroupLayout,
        colour_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport-world-shader"),
            source: wgpu::ShaderSource::Wgsl(WORLD_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-world-pipeline-layout"),
            bind_group_layouts: &[Some(camera_bgl), Some(material_bgl)],
            immediate_size: 0,
        });
        let buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &MESH_VERTEX_ATTRIBUTES,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<WorldInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    5 => Float32x4,
                    6 => Float32x4,
                    7 => Float32x4,
                    8 => Float32x4,
                ],
            },
        ];
        let make = |label, entry, format, targets: usize, stencil| {
            let target = Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_STENCIL_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil,
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &vec![target; targets],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            forward: make(
                "viewport-world-forward-pipeline",
                "fs_forward",
                colour_format,
                1,
                wgpu::StencilState::default(),
            ),
            gbuffer: make(
                "viewport-world-gbuffer-pipeline",
                "fs_gbuffer",
                GBUFFER_FORMAT,
                GBUFFER_TARGETS,
                writes_material(),
            ),
            instances: None,
            count: 0,
            draws: Vec::new(),
        }
    }

    fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[WorldInstance],
        draws: Vec<WorldDraw>,
    ) {
        self.count = instances.len() as u32;
        self.draws = draws;
        if instances.is_empty() {
            return;
        }
        let needed = std::mem::size_of_val(instances) as wgpu::BufferAddress;
        if self.instances.as_ref().is_none_or(|b| b.size() < needed) {
            self.instances = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport-world-instances"),
                size: needed.next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.instances {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(instances));
        }
    }

    fn draws_anything(&self) -> bool {
        self.count > 0 && !self.draws.is_empty()
    }

    fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        camera: &wgpu::BindGroup,
        fallback: &wgpu::BindGroup,
    ) {
        let Some(instances) = self.instances.as_ref().filter(|_| self.draws_anything()) else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_vertex_buffer(1, instances.slice(..));
        for draw in &self.draws {
            // A run past the instances of this frame would read what an earlier frame left in the buffer
            let run = draw.instances.start..draw.instances.end.min(self.count);
            if run.is_empty() || draw.mesh.index_count == 0 {
                continue;
            }
            pass.set_bind_group(1, draw.mesh.texture.as_deref().unwrap_or(fallback), &[]);
            pass.set_vertex_buffer(0, draw.mesh.vertices.slice(..));
            pass.set_index_buffer(draw.mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..draw.mesh.index_count, 0, run);
        }
    }

    /// Draw the world lit as the material mode lights a surface, into a pass over the viewport's colour and depth.
    pub(crate) fn draw_forward(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        camera: &wgpu::BindGroup,
        fallback: &wgpu::BindGroup,
    ) {
        self.draw(pass, &self.forward, camera, fallback);
    }

    /// Draw the world into a pass that fills the G-buffer, for the game's lighting passes to light.
    pub(crate) fn draw_gbuffer(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        camera: &wgpu::BindGroup,
        fallback: &wgpu::BindGroup,
    ) {
        pass.set_stencil_reference(MATERIAL_DEFAULT);
        self.draw(pass, &self.gbuffer, camera, fallback);
    }
}

impl ViewportRenderer {
    /// The world this frame draws: every instance, and the runs of them each mesh draws; an empty list draws none.
    pub fn set_world(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[WorldInstance],
        draws: Vec<WorldDraw>,
    ) {
        self.world.set(device, queue, instances, draws);
    }

    /// How many instances the world draws.
    pub fn world_instances(&self) -> u32 {
        self.world.count
    }

    /// Whether there is a world to draw.
    pub fn has_world(&self) -> bool {
        self.world.draws_anything()
    }
}

const WORLD_SHADER: &str = r#"
// Every field of the viewport's `CameraUniform`, in order: a uniform struct is addressed by byte offset.
struct Camera {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos_ws: vec4<f32>,
    camera_misc: vec4<f32>,
    key_light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var diffuse_tex: texture_2d<f32>;
@group(1) @binding(1) var diffuse_smp: sampler;

struct VsIn {
    @location(0) position:   vec3<f32>,
    @location(1) normal:     vec3<f32>,
    @location(2) uv:         vec2<f32>,
    @location(3) colour:     vec4<f32>,
    @location(4) depth_bias: f32,
    @location(5) row0:       vec4<f32>,
    @location(6) row1:       vec4<f32>,
    @location(7) row2:       vec4<f32>,
    @location(8) tint:       vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
};

// The pull towards the camera a vertex's depth bias asks for, as the viewport's mesh shader makes it.
const BIAS_MAX_PULL: f32 = 0.05;
const BIAS_DIST_SCALE: f32 = 0.002;

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let local = vec4<f32>(in.position, 1.0);
    let world = vec3<f32>(dot(in.row0, local), dot(in.row1, local), dot(in.row2, local));
    let to_camera = camera.camera_pos_ws.xyz - world;
    let dist_to_cam = length(to_camera);
    let pull_metres = in.depth_bias * min(BIAS_MAX_PULL, dist_to_cam * BIAS_DIST_SCALE);
    let biased = world + to_camera / max(dist_to_cam, 1e-6) * pull_metres;
    out.clip_pos = camera.view_proj * vec4<f32>(biased, 1.0);
    // The rows turn the normal as they turn the surface, which is right for a uniform scale; a non-uniform one shears it slightly.
    out.world_normal = normalize(vec3<f32>(
        dot(in.row0.xyz, in.normal),
        dot(in.row1.xyz, in.normal),
        dot(in.row2.xyz, in.normal),
    ));
    out.uv = in.uv;
    out.colour = in.colour * in.tint;
    out.world_pos = world;
    return out;
}

// Culling is off, so a face seen from behind is shaded by the normal turned towards the viewer.
fn facing_normal(world_normal: vec3<f32>, front_facing: bool) -> vec3<f32> {
    if (front_facing) {
        return normalize(world_normal);
    }
    return -normalize(world_normal);
}

// A cutout texel is drawn or it is not: nothing of the world blends.
const CUTOUT_ALPHA: f32 = 0.5;

@fragment
fn fs_forward(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let surface = textureSample(diffuse_tex, diffuse_smp, in.uv) * in.colour;
    if (surface.a < CUTOUT_ALPHA) {
        discard;
    }
    // The key light, the head light and the ambient light of the viewport's material mode.
    let n = facing_normal(in.world_normal, front_facing);
    let view_dir = normalize(camera.camera_pos_ws.xyz - in.world_pos);
    let key = max(dot(n, camera.key_light.xyz), 0.0) * camera.key_light.w;
    let head = max(dot(n, view_dir), 0.0) * 0.30;
    return vec4<f32>(surface.rgb * (key + head + 0.15), 1.0);
}

// The four targets of the game's G-buffer, as its own default effect fills them for a surface with no specular map.
struct GBufferOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) ambient: vec4<f32>,
};

@fragment
fn fs_gbuffer(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> GBufferOut {
    let surface = textureSample(diffuse_tex, diffuse_smp, in.uv) * in.colour;
    if (surface.a < CUTOUT_ALPHA) {
        discard;
    }
    var out: GBufferOut;
    // The lighting pass squares the colour it reads here, so the target holds the square root of the linear colour.
    out.albedo = vec4<f32>(sqrt(max(surface.rgb, vec3<f32>(0.0))), 1.0);
    // The normal in the world, from -1..1 into 0..1; the default effect leaves the alpha at nought for every normal but one straight up.
    out.normal = vec4<f32>(facing_normal(in.world_normal, front_facing) * 0.5 + 0.5, 0.0);
    // Matte: no specular intensity in red and no exponent in green, the blue the default effect writes, and in alpha a surface the sun reaches whole.
    out.specular = vec4<f32>(0.0, 0.0, 0.98, 1.0);
    // The whole of the natural ambient light in red and none of the artificial in green.
    out.ambient = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    return out;
}
"#;

#[cfg(test)]
mod tests;
