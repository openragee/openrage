//! GTA VI's point stars and the discs of its sun and moon, as its sky effect's star and disc
//! stages draw them: each star of its catalogue a square of two triangles on the sky, each disc a
//! fan facing the camera, added over the pixels nothing was drawn at.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use glam::{Mat3, Mat4};

use super::deferred::{DEPTH_STENCIL_FORMAT, HDR_FORMAT};
use super::sky_view::{Night, SkyMap, AIR_SAMPLES};

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

/// What both passes' shaders share: the air's transmittance by the up part of a way, the side of the
/// way every star and disc is laid square to, and where a way meets the face of the star map's cube.
macro_rules! common {
    () => {
        r#"
fn air(up: f32) -> vec3<f32> {
    let x = sqrt(clamp(up, 0.0, 1.0)) * 15.0;
    let i = min(u32(x), 14u);
    return mix(frame.air[i].rgb, frame.air[i + 1u].rgb, x - f32(i));
}

fn across_of(d: vec3<f32>) -> vec3<f32> {
    let across = cross(vec3<f32>(0.0, 0.0, 1.0), d);
    if (length(across) < 1e-4) {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    return normalize(across);
}

// The face's two coordinates over twice the major axis with its sign, plus one half, as the program works them out.
fn cube_face(d: vec3<f32>) -> vec2<f32> {
    let a = abs(d);
    var st = vec2<f32>(0.0);
    var major = 1.0;
    if (a.z >= a.y && a.z >= a.x) {
        major = d.z;
        st = vec2<f32>(select(d.x, -d.x, d.z < 0.0), -d.y);
    } else if (a.y >= a.x) {
        major = d.y;
        st = vec2<f32>(d.x, select(d.z, -d.z, d.y < 0.0));
    } else {
        major = d.x;
        st = vec2<f32>(select(-d.z, d.z, d.x < 0.0), -d.y);
    }
    return st / (2.0 * major) + 0.5;
}
"#
    };
}

const SHADER: &str = concat!(
    r#"
struct Frame {
    // The projection times the view's turn, the camera at the centre of the sky.
    project: mat4x4<f32>,
    // The rows of the turn from the frame's axes (x east, y north, z up) to the star map's.
    turn0: vec4<f32>,
    turn1: vec4<f32>,
    turn2: vec4<f32>,
    // The frame's scale of every star's size and colour, the profile's power, and the exposure.
    point: vec4<f32>,
    // One over the heights the stars and their blink blend in over, the blink's phase, and the cosine inside which the moon hides them.
    heights: vec4<f32>,
    moon: vec4<f32>,
    air: array<vec4<f32>, 16>,
};
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var profile: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;
"#,
    common!(),
    r#"
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
    @location(2) way: vec3<f32>,
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
    let across = across_of(d);
    let up = cross(d, across);
    // The size is the square's half width at the stars' distance.
    let half = way.w * frame.point.x / 20000.0;
    let p = d + (across * c.x + up * c.y) * half;
    // Each corner blinks by a hash of where it falls on a face of the star map's cube, whole at the
    // horizon and less as it climbs through the blink's height.
    let n = normalize(p);
    let s = vec3<f32>(dot(frame.turn0.xyz, n), dot(frame.turn1.xyz, n), dot(frame.turn2.xyz, n));
    let face = cube_face(s);
    let phase = 3.1415927 * fract(43759.0 * sin(12.99 * face.x + 78.233 * face.y)) + frame.heights.z;
    let reach = 1.0 - frame.heights.y * 20000.0 * n.z;
    let blink = 1.0 + reach * reach * (sin(phase) - 1.0);
    var clip = frame.project * vec4<f32>(p, 1.0);
    // At the depth the frame is cleared to, so that only what nothing was drawn at passes; none under the horizon.
    clip.z = 0.0;
    if (d.z < 0.0) {
        clip = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return Out(clip, c * 0.5 + 0.5, colour.rgb * frame.point.y * blink, p);
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let t = textureSampleLevel(profile, linear, in.corner, 0.0).r;
    let d = normalize(in.way);
    let height = frame.heights.x * 20000.0 * d.z;
    let hidden = select(1.0, 0.0, dot(frame.moon.xyz, d) > frame.heights.w || d.z < 0.0);
    let lit = in.colour * pow(t * t, frame.point.z) * (height * height) * air(d.z) * hidden;
    // ⚠ Inferred: a star blinked below nought takes nothing from the sky it is laid on.
    return vec4<f32>(max(lit * frame.point.w, vec3<f32>(0.0)), 0.0);
}
"#
);

/// The bytes of the stars pass's uniforms: the projection, six rows of four and the air's samples.
const UNIFORM_BYTES: u64 = 64 + 6 * 16 + AIR_SAMPLES as u64 * 16;

/// The rows the sky's air is stated in.
fn air_words(night: &Night) -> impl Iterator<Item = f32> + '_ {
    night.air.iter().flatten().copied()
}

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

/// The uniforms of a frame: `project` from the sky's axes to the clip, and the night's turn, scales and air.
fn words(project: Mat4, night: &Night, exposure: f32) -> Vec<f32> {
    let mut words = project.to_cols_array().to_vec();
    for row in night.turn {
        words.extend(row);
        words.push(0.0);
    }
    words.extend([night.point[0], night.point[1], night.point[2], exposure]);
    words.extend([
        night.scales[2],
        night.scales[3],
        night.blink[3],
        night.blink[2],
    ]);
    words.extend(night.moon);
    words.push(0.0);
    words.extend(air_words(night));
    words
}

/// Light added to what is there: the sky's own colour target, tested for a depth nothing was drawn at.
fn added(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    buffers: &[wgpu::VertexBufferLayout<'_>],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: None,
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers,
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
            module,
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
    })
}

/// A bind group of `uniforms`, the map `texture` and `sampler`.
fn bind_of(
    device: &wgpu::Device,
    pipeline: &wgpu::RenderPipeline,
    uniforms: &wgpu::Buffer,
    texture: &wgpu::Texture,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = texture.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sky-stars"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn linear_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("sky-stars"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
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
        let pipeline = added(
            device,
            "sky-stars",
            &module,
            &[wgpu::VertexBufferLayout {
                array_stride: STAR_BYTES,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attributes,
            }],
        );
        Self {
            device: device.clone(),
            pipeline,
            uniforms: uniform_buffer(device, "sky-stars", UNIFORM_BYTES),
            sampler: linear_sampler(device),
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
            *self.bind.borrow_mut() = Some(bind_of(
                &self.device,
                &self.pipeline,
                &self.uniforms,
                &texture,
                &self.sampler,
            ));
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

/// The triangles of a disc's fan: a centre and 89 rim points, the first and last the same.
const FAN_TRIANGLES: u32 = 88;

const DISC_SHADER: &str = concat!(
    r#"
struct Frame {
    project: mat4x4<f32>,
    // xyz the way to the sun, w its disc's radius at the stars' distance, none at nought
    sun: vec4<f32>,
    // rgb its colour, w how much its limb darkens
    sun_colour: vec4<f32>,
    moon: vec4<f32>,
    // xyz the way the moon's light comes from
    moon_lit: vec4<f32>,
    // rgb its colour, w the exposure
    moon_colour: vec4<f32>,
    air: array<vec4<f32>, 16>,
};
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var face: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;
"#,
    common!(),
    r#"
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) way: vec3<f32>,
    @location(2) @interpolate(flat) which: u32,
};

// The basis a disc's fan is laid in: across, towards the camera, and up.
fn basis(way: vec3<f32>) -> mat3x3<f32> {
    let forward = -way;
    let across = across_of(forward);
    return mat3x3<f32>(across, forward, cross(forward, across));
}

@vertex
fn vs(@builtin(vertex_index) i: u32, @builtin(instance_index) which: u32) -> Out {
    let k = i % 3u;
    var rim = vec2<f32>(0.0);
    if (k != 0u) {
        let a = f32(i / 3u + k - 1u) * 0.071399833;
        rim = vec2<f32>(cos(a), sin(a));
    }
    let disc = select(frame.sun, frame.moon, which == 1u);
    let b = basis(disc.xyz);
    let p = disc.xyz + (b[0] * rim.x + b[2] * rim.y) * (disc.w / 20000.0);
    var clip = frame.project * vec4<f32>(p, 1.0);
    clip.z = 0.0;
    if (disc.w <= 0.0) {
        clip = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    // The sun's texture coordinates run one each way from its centre to its rim, the moon's a half.
    let uv = 0.5 + rim * select(1.0, 0.5, which == 1u);
    return Out(clip, uv, p, which);
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let d = normalize(in.way);
    let seen = air(d.z) * select(1.0, 0.0, d.z < 0.0) * frame.moon_colour.w;
    if (in.which == 0u) {
        let r2 = dot(in.uv - 0.5, in.uv - 0.5);
        let limb = 1.0 - (1.0 - sqrt(max(1.0 - r2, 0.0))) * frame.sun_colour.w;
        return vec4<f32>(frame.sun_colour.rgb * limb * seen, 0.0);
    }
    // The program reads its map at the coordinates turned upside down.
    let uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    let texel = textureSampleLevel(face, linear, uv, 0.0);
    // The face as a ball: across, out towards the camera, and down the map.
    // ⚠ Inferred: the across term as twice the coordinate's distance from the centre, as the down term is.
    let a = 2.0 * uv.x - 1.0;
    let down = 2.0 * uv.y - 1.0;
    let out = sqrt(max(1.0 - a * a - down * down, 0.0));
    let b = basis(frame.moon.xyz);
    let normal = b[0] * a + b[1] * out - b[2] * down;
    let lit = max(dot(normal, frame.moon_lit.xyz), 0.0);
    return vec4<f32>(frame.moon_colour.rgb * texel.rgb * texel.a * lit * seen, 0.0);
}
"#
);

/// The bytes of the discs pass's uniforms: the projection, five rows of four and the air's samples.
const DISC_UNIFORM_BYTES: u64 = 64 + 5 * 16 + AIR_SAMPLES as u64 * 16;

/// The pass that adds the discs of the sun and the moon to the sky.
pub struct DiscsPass {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind: RefCell<Option<wgpu::BindGroup>>,
    /// The face of the moon last bound: a black texel stands for one not read.
    face: RefCell<Option<Option<*const SkyMap>>>,
    shown: Cell<bool>,
}

/// The uniforms of the discs of a frame looking through `project`, at `exposure`.
fn disc_words(project: Mat4, night: &Night, exposure: f32) -> Vec<f32> {
    let mut words = project.to_cols_array().to_vec();
    words.extend(night.sun_disc);
    words.extend(night.sun_colour);
    words.extend(night.moon);
    words.push(night.moon_disc[3]);
    words.extend(&night.moon_disc[..3]);
    words.push(0.0);
    words.extend(&night.moon_colour[..3]);
    words.push(exposure);
    words.extend(air_words(night));
    words
}

impl DiscsPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky-discs"),
            source: wgpu::ShaderSource::Wgsl(DISC_SHADER.into()),
        });
        Self {
            device: device.clone(),
            pipeline: added(device, "sky-discs", &module, &[]),
            uniforms: uniform_buffer(device, "sky-discs", DISC_UNIFORM_BYTES),
            sampler: linear_sampler(device),
            bind: RefCell::new(None),
            face: RefCell::new(None),
            shown: Cell::new(false),
        }
    }

    /// Write this frame's discs as `night` states them, seen through `projection` turned by `view`, the moon's laid with `face`.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        (projection, view): (Mat4, Mat4),
        night: Option<&Night>,
        exposure: f32,
        face: Option<&SkyMap>,
    ) {
        let night = night.filter(|n| n.sun_disc[3] > 0.0 || n.moon_disc[3] > 0.0);
        self.shown.set(night.is_some());
        let Some(night) = night else {
            return;
        };
        let held = face.map(std::ptr::from_ref);
        if *self.face.borrow() != Some(held) {
            let texture = super::sky_view::map_texture(&self.device, queue, face);
            *self.bind.borrow_mut() = Some(bind_of(
                &self.device,
                &self.pipeline,
                &self.uniforms,
                &texture,
                &self.sampler,
            ));
            *self.face.borrow_mut() = Some(held);
        }
        let turn = Mat4::from_mat3(Mat3::from_mat4(view));
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::cast_slice(&disc_words(projection * turn, night, exposure)),
        );
    }

    /// Add the discs into `pass`, where a frame was written with one to draw.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        let bind = self.bind.borrow();
        if let (true, Some(bind)) = (self.shown.get(), bind.as_ref()) {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..FAN_TRIANGLES * 3, 0..2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_size(shader: &str) -> u32 {
        let module = naga::front::wgsl::parse_str(shader).expect("the WGSL parses");
        naga::valid::Validator::new(Default::default(), naga::valid::Capabilities::empty())
            .validate(&module)
            .expect("the WGSL validates");
        let (_, frame) = (module.types.iter())
            .find(|(_, t)| t.name.as_deref() == Some("Frame"))
            .expect("a Frame");
        frame.inner.size(module.to_ctx())
    }

    #[test]
    fn the_shaders_validate_and_their_uniforms_fill_the_frames_they_declare() {
        let frame = frame_size(SHADER);
        assert_eq!(
            words(Mat4::IDENTITY, &Night::default(), 1.0).len() * 4,
            frame as usize
        );
        assert_eq!(u64::from(frame), UNIFORM_BYTES);
        let frame = frame_size(DISC_SHADER);
        let words = disc_words(Mat4::IDENTITY, &Night::default(), 1.0);
        assert_eq!(words.len() * 4, frame as usize);
        assert_eq!(u64::from(frame), DISC_UNIFORM_BYTES);
    }

    #[test]
    fn a_fan_steps_round_its_rim_as_the_builds_does() {
        // The shader's step is the build's f32 of 2 pi / 88.
        let step = crate::world::sky_dome::fan_vertices(crate::world::sky_dome::DiscFan {
            radius: 1.0,
            uv_scale: 1.0,
        })[2]
            .position;
        assert!((step.z.atan2(step.x) - std::f32::consts::TAU / 88.0).abs() < 1e-6);
        assert_eq!(
            FAN_TRIANGLES as usize + 2,
            crate::world::sky_dome::FAN_VERTICES
        );
    }
}
