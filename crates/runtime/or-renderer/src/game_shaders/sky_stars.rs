//! GTA VI's point stars: each star of its catalogue a square of two triangles on the sky, its size
//! and colour scaled by the frame, drawn with the star's profile over the pixels nothing was drawn at.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use glam::{Mat3, Mat4};

use super::deferred::{DEPTH_STENCIL_FORMAT, HDR_FORMAT};
use super::sky_view::{Night, SkyMap};

/// The stars of a catalogue as the pass draws them, each eight floats: where it is in the star map's axes at the stars' distance of twenty thousand units, its size in those units, and its colour times its intensity.
#[derive(Debug, Default)]
pub struct PointStars {
    pub stars: Vec<[f32; 8]>,
}

/// A catalogue is read once and held: two are the same only where they are one.
impl PartialEq for PointStars {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

/// The bytes of one star in the instance buffer.
const STAR_BYTES: u64 = 32;

const SHADER: &str = r#"
struct Frame {
    // The projection times the view's turn, the camera at the centre of the sky.
    project: mat4x4<f32>,
    // The rows of the turn from the star map's axes to the frame's (x east, y north, z up).
    turn0: vec4<f32>,
    turn1: vec4<f32>,
    turn2: vec4<f32>,
    // The frame's scale of every star's size and colour, the profile's power, and the exposure.
    point: vec4<f32>,
};
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var profile: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32, @location(0) way: vec4<f32>, @location(1) colour: vec4<f32>) -> Out {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[i];
    let d = normalize(vec3<f32>(
        dot(vec3<f32>(frame.turn0.x, frame.turn1.x, frame.turn2.x), way.xyz),
        dot(vec3<f32>(frame.turn0.y, frame.turn1.y, frame.turn2.y), way.xyz),
        dot(vec3<f32>(frame.turn0.z, frame.turn1.z, frame.turn2.z), way.xyz),
    ));
    var across = cross(vec3<f32>(0.0, 0.0, 1.0), d);
    if (length(across) < 1e-4) {
        across = vec3<f32>(1.0, 0.0, 0.0);
    }
    across = normalize(across);
    let up = cross(d, across);
    // ⚠ Inferred: the size is the square's half width at the stars' distance, as a disc's size is its radius there; the vertex stage that lays it was not read.
    let half = way.w * frame.point.x / 20000.0;
    let p = d + (across * c.x + up * c.y) * half;
    var clip = frame.project * vec4<f32>(p, 1.0);
    // At the depth the frame is cleared to, so that only what nothing was drawn at passes; none under the horizon.
    clip.z = 0.0;
    if (d.z < 0.0) {
        clip = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return Out(clip, c * 0.5 + 0.5, colour.rgb * frame.point.y * frame.point.w);
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let t = textureSampleLevel(profile, linear, in.corner, 0.0).r;
    return vec4<f32>(in.colour * pow(t, 2.0 * frame.point.z), 0.0);
}
"#;

/// The bytes of the pass's uniforms: the projection and four rows of four.
const UNIFORM_BYTES: u64 = 64 + 4 * 16;

/// The pass that adds the point stars to the sky.
pub struct StarsPass {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind: RefCell<Option<wgpu::BindGroup>>,
    /// The catalogue last uploaded and its buffer, and the profile last bound.
    held: RefCell<Option<(Arc<PointStars>, wgpu::Buffer)>>,
    profile: RefCell<Option<*const SkyMap>>,
    /// How many stars the frame draws.
    drawn: Cell<u32>,
}

/// The uniforms of a frame: `project` from the sky's axes to the clip, and the night's turn and scales.
fn words(project: Mat4, night: &Night, exposure: f32) -> Vec<f32> {
    let mut words = project.to_cols_array().to_vec();
    for row in night.turn {
        words.extend(row);
        words.push(0.0);
    }
    words.extend([night.point[0], night.point[1], night.point[2], exposure]);
    words
}

impl StarsPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky-stars"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 16,
                shader_location: 1,
            },
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky-stars"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: STAR_BYTES,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_STENCIL_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Equal),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky-stars"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky-stars"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            device: device.clone(),
            pipeline,
            uniforms,
            sampler,
            bind: RefCell::new(None),
            held: RefCell::new(None),
            profile: RefCell::new(None),
            drawn: Cell::new(0),
        }
    }

    /// Write this frame's stars: `stars` as `night` scales them, seen through `projection` turned by `view`, laid with the map `profile`; none draws nothing.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        (projection, view): (Mat4, Mat4),
        night: Option<&Night>,
        exposure: f32,
        (stars, profile): (Option<&Arc<PointStars>>, Option<&SkyMap>),
    ) {
        self.drawn.set(0);
        let (Some(night), Some(stars), Some(profile)) = (night, stars, profile) else {
            return;
        };
        if night.point[3] < 0.5 || stars.stars.is_empty() {
            return;
        }
        let same = (self.held.borrow().as_ref()).is_some_and(|(held, _)| Arc::ptr_eq(held, stars));
        if !same {
            let bytes: Vec<f32> = stars.stars.iter().flatten().copied().collect();
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sky-stars"),
                size: bytes.len() as u64 * 4,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&bytes));
            *self.held.borrow_mut() = Some((Arc::clone(stars), buffer));
        }
        if *self.profile.borrow() != Some(std::ptr::from_ref(profile)) {
            let texture = super::sky_view::map_texture(&self.device, queue, Some(profile));
            let view = texture.create_view(&Default::default());
            *self.bind.borrow_mut() =
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sky-stars"),
                    layout: &self.pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.uniforms.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                }));
            *self.profile.borrow_mut() = Some(std::ptr::from_ref(profile));
        }
        let turn = Mat4::from_mat3(Mat3::from_mat4(view));
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::cast_slice(&words(projection * turn, night, exposure)),
        );
        self.drawn.set(stars.stars.len() as u32);
    }

    /// Add the stars into `pass`, where a frame was written with some to draw.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        let count = self.drawn.get();
        let held = self.held.borrow();
        let bind = self.bind.borrow();
        if let (true, Some((_, buffer)), Some(bind)) = (count > 0, held.as_ref(), bind.as_ref()) {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, 0..count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_validates_and_its_uniforms_fill_the_frame_it_declares() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("the WGSL parses");
        naga::valid::Validator::new(Default::default(), naga::valid::Capabilities::empty())
            .validate(&module)
            .expect("the WGSL validates");
        let frame = (module.types.iter())
            .find(|(_, t)| t.name.as_deref() == Some("Frame"))
            .map(|(_, t)| t.inner.size(module.to_ctx()))
            .expect("a Frame");
        let words = words(Mat4::IDENTITY, &Night::default(), 1.0);
        assert_eq!(words.len() * 4, frame as usize);
        assert_eq!(u64::from(frame), UNIFORM_BYTES);
    }
}
