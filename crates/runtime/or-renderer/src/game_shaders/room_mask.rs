//! The rooms an interior's lamp lights: the pixels whose surface lies inside one of its boxes (its room's, those of the rooms its portals open on, and its portals' own) marked in a bit of the stencil above the material's, which the lamp's volumes are then drawn over and nowhere else, a box after another.

use std::ops::Range;

use glam::{Mat4, Vec3};

use super::deferred::DEPTH_STENCIL_FORMAT;

/// The stencil bit a box's pixels are marked in: above the material's three.
// ⚠ One box at a time: boxes overlap, and a pixel in the rooms of two lamps is lit by both.
pub const ROOM_BIT: u32 = 0x80;
/// The bit a pixel already lit by an earlier box of the same lamps keeps, so that a second box does not light it again.
const LIT_BIT: u32 = 0x40;
/// How many boxes a frame marks; the lamps of any past them light unmasked.
pub const MOST_BOXES: usize = 255;

const SHADER: &str = r#"
struct Box {
    // The box as the world from a cube of minus one to one, and the eye in the cube's space.
    bounds: mat4x4<f32>,
    eye: vec4<f32>,
};
struct Boxes {
    view_projection: mat4x4<f32>,
    boxes: array<Box, 255>,
};
@group(0) @binding(0) var<uniform> boxes: Boxes;

// Corner `v` of the cube's twelve triangles, a face a pair, where the face is one the eye sees from outside or not as `near` asks; nothing otherwise.
// ⚠ Chosen by the eye rather than by winding: a mirrored instance turns a box's winding over.
fn corner(v: u32, k: u32, near: bool) -> vec4<f32> {
    var corners = array<u32, 36>(
        0u, 2u, 3u, 0u, 3u, 1u,  4u, 5u, 7u, 4u, 7u, 6u,
        0u, 1u, 5u, 0u, 5u, 4u,  2u, 6u, 7u, 2u, 7u, 3u,
        0u, 4u, 6u, 0u, 6u, 2u,  1u, 3u, 7u, 1u, 7u, 5u,
    );
    let b = boxes.boxes[k];
    let face = v / 6u;
    // The faces stand at z, then y, then x, minus one first.
    let axis = 2u - face / 2u;
    let side = select(-1.0, 1.0, (face & 1u) == 1u);
    let sees = b.eye[axis] * side > 1.0;
    if (sees != near) {
        return vec4<f32>(0.0);
    }
    let c = corners[v];
    let unit = vec3<f32>(f32(c & 1u), f32((c >> 1u) & 1u), f32((c >> 2u) & 1u)) * 2.0 - 1.0;
    return boxes.view_projection * b.bounds * vec4<f32>(unit, 1.0);
}

@vertex
fn vs_far(@builtin(vertex_index) v: u32, @builtin(instance_index) k: u32) -> @invariant @builtin(position) vec4<f32> {
    return corner(v, k, false);
}

@vertex
fn vs_near(@builtin(vertex_index) v: u32, @builtin(instance_index) k: u32) -> @invariant @builtin(position) vec4<f32> {
    return corner(v, k, true);
}
"#;

const UNIFORM_BYTES: u64 = 64 + 80 * MOST_BOXES as u64;

/// A box some lamps light: the world from a cube of minus one to one, and the lamps, as ranges of the frame's lists of points, spots and capsules. Boxes of the same lamps stand together; the lamps of no box light every pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct LightRoom {
    pub bounds: Mat4,
    pub lights: [Range<u32>; 3],
}

/// The passes that mark and clear a box's pixels in the stencil, drawn in a pass of the frame's depth and stencil alone.
pub struct RoomMask {
    /// Marks every pixel no earlier box of the same lamps lit whose surface stands in front of the box's far side.
    mark: wgpu::RenderPipeline,
    /// Clears again those whose surface stands in front of the box's near side: what is left lies inside it.
    unmark_before: wgpu::RenderPipeline,
    /// Keeps what the box marked as lit, for the next box of the same lamps.
    keep: wgpu::RenderPipeline,
    /// Clears the mark wherever the box covers the screen; and both bits.
    clear: wgpu::RenderPipeline,
    clear_all: wgpu::RenderPipeline,
    values: wgpu::Buffer,
    group: wgpu::BindGroup,
}

impl RoomMask {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("room-mask"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let values = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("room-mask"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("room-mask"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("room-mask"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: values.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("room-mask"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // ⚠ The frame's depth is reversed: the nearer, the greater.
        let pipeline = |entry, depth_compare, (compare, read_mask), (pass_op, write_mask)| {
            let face = wgpu::StencilFaceState {
                compare,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op,
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("room-mask"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: None,
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_STENCIL_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare),
                    stencil: wgpu::StencilState {
                        front: face,
                        back: face,
                        read_mask,
                        write_mask,
                    },
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::{CompareFunction as C, StencilOperation as S};
        Self {
            // The far side, where it lies behind the surface or on it and the pixel is not lit yet: the reference's lit bit is clear
            mark: pipeline(
                "vs_far",
                C::LessEqual,
                (C::Equal, LIT_BIT),
                (S::Replace, ROOM_BIT),
            ),
            // The near side, where it lies behind the surface
            unmark_before: pipeline("vs_near", C::Less, (C::Always, 0), (S::Zero, ROOM_BIT)),
            keep: pipeline(
                "vs_far",
                C::Always,
                (C::Equal, ROOM_BIT),
                (S::Replace, LIT_BIT),
            ),
            clear: pipeline("vs_far", C::Always, (C::Always, 0), (S::Zero, ROOM_BIT)),
            clear_all: pipeline(
                "vs_far",
                C::Always,
                (C::Always, 0),
                (S::Zero, ROOM_BIT | LIT_BIT),
            ),
            values,
            group,
        }
    }

    /// Write the frame's view and projection, the eye, and `boxes`, each the world from a cube of minus one to one; the first [`MOST_BOXES`] are kept.
    pub fn write(&self, queue: &wgpu::Queue, view_projection: Mat4, eye: Vec3, boxes: &[Mat4]) {
        let boxes = &boxes[..boxes.len().min(MOST_BOXES)];
        let mut values: Vec<f32> = Vec::with_capacity(16 + 20 * boxes.len());
        values.extend(view_projection.to_cols_array());
        for b in boxes {
            values.extend(b.to_cols_array());
            values.extend(b.inverse().transform_point3(eye).extend(1.0).to_array());
        }
        queue.write_buffer(&self.values, 0, bytemuck::cast_slice(&values));
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline, k: usize) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..36, k as u32..k as u32 + 1);
    }

    /// Mark the pixels of `pass` whose surface lies inside box `k` and that no earlier box of the same lamps lit.
    pub fn mark(&self, pass: &mut wgpu::RenderPass<'_>, k: usize) {
        pass.set_stencil_reference(ROOM_BIT);
        self.draw(pass, &self.mark, k);
        self.draw(pass, &self.unmark_before, k);
    }

    /// Keep what box `k` marked as lit and clear its mark.
    pub fn keep(&self, pass: &mut wgpu::RenderPass<'_>, k: usize) {
        pass.set_stencil_reference(ROOM_BIT | LIT_BIT);
        self.draw(pass, &self.keep, k);
        self.draw(pass, &self.clear, k);
    }

    /// Clear what box `k` marked or kept.
    pub fn clear(&self, pass: &mut wgpu::RenderPass<'_>, k: usize) {
        self.draw(pass, &self.clear_all, k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_validates_and_its_uniform_holds_every_box() {
        let parsed = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{e}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&parsed)
        .unwrap_or_else(|e| panic!("{e:?}"));
        let boxes = (parsed.global_variables.iter())
            .find(|(_, g)| g.name.as_deref() == Some("boxes"))
            .map(|(_, g)| parsed.types[g.ty].inner.size(parsed.to_ctx()))
            .expect("the boxes");
        assert_eq!(u64::from(boxes), UNIFORM_BYTES);
    }
}
