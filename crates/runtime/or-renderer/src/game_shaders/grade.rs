//! GTA VI's colour grade over the lit frame, by the moment's own values: two gradients across the
//! screen, top to bottom and left to right, each three colours about a midpoint as the build packs
//! them.

use super::deferred::HDR_FORMAT;

/// The grade's gamma at the display setting the build rests at: shadow and highlight colours go to the power of one over it.
const GRADE_GAMMA: f32 = 2.4;
/// The least magnitude of a pivot's divisor, so that a midpoint of nought or one stays finite: the viewer's guard, the build has none.
const PIVOT_MARGIN: f32 = 1e-4;

/// One gradient as the build packs it: each colour (`w` one less twice its region's midpoint, the middle's `w` the midpoint), one over the midpoint's two spans, how far the middle is drawn to the ends' mean, and the least a colour is held to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradeSet {
    pub first: [f32; 4],
    pub middle: [f32; 4],
    pub last: [f32; 4],
    pub pivot: [f32; 4],
}

impl Default for GradeSet {
    /// What a moment that states nothing packs: white at every point, the midpoints at the half.
    fn default() -> Self {
        Self {
            first: [1.0, 1.0, 1.0, 0.0],
            middle: [1.0, 1.0, 1.0, 0.5],
            last: [1.0, 1.0, 1.0, 0.0],
            pivot: [2.0, 2.0, 0.0, 0.0],
        }
    }
}

/// The composite's shift of the darkest pixels towards a rod's sight: the tint, its strength, and the weight's scale and bias over the pixel's luminance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scotopic {
    pub colour: [f32; 3],
    pub intensity: f32,
    pub scale: f32,
    pub bias: f32,
}

impl Scotopic {
    /// The shift `get` reads from a moment's keyframe by name, each with the variable table's default.
    pub fn of(get: &dyn Fn(&str, f32) -> f32) -> Self {
        Self {
            colour: [
                get("postfx_scotopic_col_r", 0.368),
                get("postfx_scotopic_col_g", 0.537),
                get("postfx_scotopic_col_b", 1.0),
            ],
            intensity: get("postfx_scotopic_blue_shift_intensity", 0.7),
            scale: get("postfx_scotopic_weight_scale", 0.3),
            bias: get("postfx_scotopic_weight_bias", -0.15),
        }
    }

    /// The words the sharpening pass reads after its amount: the tint and strength, then the weight's scale and bias.
    pub(crate) fn words(&self) -> [f32; 8] {
        let [r, g, b] = self.colour;
        [r, g, b, self.intensity, self.scale, self.bias, 0.0, 0.0]
    }
}

/// The grade of a moment: the gradient top to bottom and the one left to right.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Grade {
    pub vertical: GradeSet,
    pub horizontal: GradeSet,
}

// To be merged with the post track's `post_math::grade_set` when both land.
fn away_from_zero(x: f32) -> f32 {
    if x.abs() < PIVOT_MARGIN {
        PIVOT_MARGIN.copysign(x)
    } else {
        x
    }
}

impl GradeSet {
    /// The set `get` reads by name: the colours of the three points `points`, the midpoint `centre` and the two regions' midpoints `widths`, as the build packs them.
    // To be merged with the post track's `post_math::grade_set` when both land.
    fn of(
        get: &dyn Fn(&str, f32) -> f32,
        points: [&str; 3],
        centre: &str,
        widths: [&str; 2],
    ) -> Self {
        let smooth = get(&format!("postfx_grad_{}_col_smooth_t", points[1]), 0.0);
        let least = get("postfx_grad_color_min", 0.0);
        let k = 1.0 / GRADE_GAMMA;
        let point = |name: &str| {
            let fade = get(&format!("postfx_grad_{name}_col_fade"), 1.0);
            let mult = get(&format!("postfx_grad_{name}_col_mult"), 1.0);
            let colour =
                ["r", "g", "b"].map(|c| get(&format!("postfx_grad_{name}_col_{c}"), 1.0) * fade);
            (colour, mult)
        };
        let power = |(colour, mult): ([f32; 3], f32)| colour.map(|c| c.max(0.0).powf(k) * mult);
        let p = get(centre, 0.5);
        let (a, c) = (power(point(points[0])), power(point(points[2])));
        let (mid, mid_mult) = point(points[1]);
        Self {
            first: [a[0], a[1], a[2], 1.0 - 2.0 * get(widths[0], 0.5)],
            middle: [mid[0] * mid_mult, mid[1] * mid_mult, mid[2] * mid_mult, p],
            last: [c[0], c[1], c[2], 1.0 - 2.0 * get(widths[1], 0.5)],
            pivot: [
                1.0 / away_from_zero(1.0 - p),
                1.0 / away_from_zero(p),
                smooth,
                least,
            ],
        }
    }
}

impl Grade {
    /// The grade `get` reads from a moment's keyframe by name, each with the variable table's default.
    pub fn of(get: &dyn Fn(&str, f32) -> f32) -> Self {
        Self {
            vertical: GradeSet::of(
                get,
                ["top", "middle", "bottom"],
                "postfx_grad_midpoint",
                [
                    "postfx_grad_top_middle_midpoint",
                    "postfx_grad_middle_bottom_midpoint",
                ],
            ),
            horizontal: GradeSet::of(
                get,
                ["left", "center", "right"],
                "postfx_grad_centerpoint",
                [
                    "postfx_grad_left_center_midpoint",
                    "postfx_grad_center_right_midpoint",
                ],
            ),
        }
    }

    /// The pass's uniforms.
    fn words(&self) -> Vec<f32> {
        let mut words = Vec::with_capacity(32);
        for set in [self.vertical, self.horizontal] {
            for row in [set.first, set.middle, set.last, set.pivot] {
                words.extend(row);
            }
        }
        words
    }
}

const SHADER: &str = r#"
struct Set {
    first: vec4<f32>,
    middle: vec4<f32>,
    last: vec4<f32>,
    pivot: vec4<f32>,
};
struct Values {
    vertical: Set,
    horizontal: Set,
};
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> values: Values;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// The colours held to the least, the middle drawn to the ends' mean, a ramp from the first to the
// middle and one from the middle to the last, each moved by its region's term, then the two ramps
// mixed by the place itself.
// ⚠ The dynamic multipliers times the contrast heuristic's value are left out: both are nought in every moment read.
fn gradient(t: f32, s: Set) -> vec3<f32> {
    let first = max(s.first.rgb, vec3<f32>(s.pivot.w));
    let last = max(s.last.rgb, vec3<f32>(s.pivot.w));
    let middle = mix(max(s.middle.rgb, vec3<f32>(s.pivot.w)), (first + last) * 0.5, s.pivot.z);
    let up = mix(first, middle, saturate(s.first.w + saturate(t * s.pivot.y)));
    let down = mix(middle, last, saturate(saturate(s.pivot.x * saturate(t - s.middle.w)) - s.last.w));
    return mix(up, down, t);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source));
    let texel = textureLoad(source, vec2<i32>(position.xy), 0);
    let at = position.xy / size;
    // The frame is graded before the bloom is laid over it, top to bottom and left to right.
    let c = texel.rgb * gradient(at.y, values.vertical) * gradient(at.x, values.horizontal);
    return vec4<f32>(c, texel.a);
}
"#;

/// The bytes of the pass's uniforms.
const UNIFORM_BYTES: u64 = 32 * 4;

/// The pass that grades the lit frame in place.
pub(crate) struct GradePass {
    pipeline: wgpu::RenderPipeline,
    values: wgpu::Buffer,
    /// The target the grade is drawn into, its size, and the bindings of the frame it reads.
    scratch: Option<((u32, u32), wgpu::TextureView, wgpu::BindGroup)>,
}

impl GradePass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grade"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grade"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let values = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grade"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            values,
            scratch: None,
        }
    }

    /// Make the target for a frame `hdr` where it is of another size, and bind `hdr`.
    pub(crate) fn prepare(&mut self, device: &wgpu::Device, hdr: &wgpu::TextureView) {
        let size = hdr.texture().size();
        let size = (size.width, size.height);
        let target = match self.scratch.take() {
            Some((was, target, _)) if was == size => target,
            _ => super::targets::render_target(
                device,
                "grade",
                size,
                HDR_FORMAT,
                wgpu::TextureUsages::COPY_SRC,
            ),
        };
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grade"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.values.as_entire_binding(),
                },
            ],
        });
        self.scratch = Some((size, target, bind));
    }

    /// Grade the lit frame `hdr` by `grade`, leaving the result in `hdr`.
    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        hdr: &wgpu::TextureView,
        grade: &Grade,
    ) {
        let Some((_, target, bind)) = &self.scratch else {
            return;
        };
        queue.write_buffer(&self.values, 0, bytemuck::cast_slice(&grade.words()));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grade"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_texture(
            target.texture().as_image_copy(),
            hdr.texture().as_image_copy(),
            target.texture().size(),
        );
    }
}

#[cfg(test)]
#[path = "grade_tests.rs"]
mod tests;
