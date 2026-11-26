//! The lights of a frame drawn as volumes of the game's pass for their kind, over the frame the sun lit: each light one instance of a unit sphere the pass shapes.

use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use glam::Vec3;
use or_io::gta5::shaders::spirv_pack::SignatureElement;
use wgpu::util::DeviceExt;

use super::deferred::{
    lighting_values, over_what_was_drawn, provide, Inputs, Screen, DEPTH_STENCIL_FORMAT,
};
use super::engine::EngineConstants;
use super::mesh::scene::MATERIAL_CLEAR;
use super::pack::GamePack;
use super::pipeline::{DrawState, Target};
use super::room_mask::ROOM_BIT;
use super::screen::{vertex_inputs, ScreenError, ScreenPass, Space};

/// The pass that reads a light from a stream of its own, after the twenty-four that read one from the constants and the three that only mark the stencil.
const INSTANCED: usize = 27;

/// How many lights of a kind are drawn at once: the 2,048 a dense street hands the frame, a spot drawn as two cones of eight nested volumes.
pub const MOST: usize = 32_768;

/// How many times each side of the octahedron the sphere is cut into: the volume of a light is that much of a sphere.
const CUTS: u32 = 3;

/// The triangles of a sphere of one unit, three corners each, every one wound to face outward.
pub fn sphere() -> Vec<Vec3> {
    let corners = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    let faces = [
        [0, 2, 4],
        [2, 1, 4],
        [1, 3, 4],
        [3, 0, 4],
        [2, 0, 5],
        [1, 2, 5],
        [3, 1, 5],
        [0, 3, 5],
    ];
    let mut triangles: Vec<[Vec3; 3]> = faces.iter().map(|f| f.map(|i| corners[i])).collect();
    for _ in 0..CUTS {
        triangles = triangles
            .iter()
            .flat_map(|[a, b, c]| {
                let (ab, bc, ca) = (
                    ((*a + *b) * 0.5).normalize(),
                    ((*b + *c) * 0.5).normalize(),
                    ((*c + *a) * 0.5).normalize(),
                );
                [[*a, ab, ca], [ab, *b, bc], [ca, bc, *c], [ab, bc, ca]]
            })
            .collect();
    }
    triangles.into_iter().flatten().collect()
}

/// What the input `semantic` of the vertex stage reads of a corner of the sphere.
pub(super) fn corner_value(semantic: &str, corner: Vec3) -> [f32; 4] {
    match semantic {
        "POSITION" => [corner.x, corner.y, corner.z, 1.0],
        _ => [0.0; 4],
    }
}

/// One record of `stride` bytes, each attribute written from the input at its location, by what `value` reads of the item.
pub(super) fn record_bytes(
    attributes: &[wgpu::VertexAttribute],
    stride: u64,
    inputs: &[SignatureElement],
    value: &dyn Fn(&SignatureElement) -> [f32; 4],
) -> Vec<u8> {
    let mut written = vec![0u8; stride as usize];
    for attribute in attributes {
        let values = inputs
            .iter()
            .find(|i| i.register == attribute.shader_location)
            .map_or([0.0; 4], value);
        let words = (attribute.format.size() / 4) as usize;
        for (i, v) in values.iter().take(words).enumerate() {
            let at = attribute.offset as usize + 4 * i;
            written[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    written
}

/// The records of `items` side by side, each the bytes [`record_bytes`] writes for it, each attribute's input found once for all of them.
fn records<L>(
    attributes: &[wgpu::VertexAttribute],
    stride: u64,
    inputs: &[SignatureElement],
    items: &[L],
    value: &dyn Fn(&SignatureElement, &L) -> [f32; 4],
) -> Vec<u8> {
    let reads: Vec<(usize, usize, Option<&SignatureElement>)> = (attributes.iter())
        .map(|attribute| {
            let input = inputs
                .iter()
                .find(|i| i.register == attribute.shader_location);
            let words = (attribute.format.size() / 4) as usize;
            (attribute.offset as usize, words, input)
        })
        .collect();
    let stride = stride as usize;
    let mut bytes = vec![0u8; stride * items.len()];
    for (record, item) in bytes.chunks_exact_mut(stride.max(1)).zip(items) {
        for &(offset, words, input) in &reads {
            let values = input.map_or([0.0; 4], |input| value(input, item));
            for (k, v) in values.iter().take(words).enumerate() {
                record[offset + 4 * k..offset + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    bytes
}

/// The lights of one kind, drawn into the frame the sun lit.
pub struct LightVolumes<L> {
    pass: ScreenPass,
    /// The same pass drawn only over the pixels a room marked.
    in_room: ScreenPass,
    inputs: Vec<SignatureElement>,
    sphere: wgpu::Buffer,
    corners: u32,
    instances: wgpu::Buffer,
    /// What each input of the vertex stage reads of a light.
    value: fn(&str, u32, &L) -> [f32; 4],
    /// How many lights the frame was written with.
    drawn: AtomicU32,
    /// The lights the instances were last written with.
    written: Mutex<Vec<L>>,
}

impl<L: Copy + PartialEq> LightVolumes<L> {
    /// The pass `effect` and `technique` give for a light that comes from a stream of its own, drawn additively into colour targets of `colour`, over what `frame` holds.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        colour: wgpu::TextureFormat,
        frame: &Inputs<'_>,
        effect: &str,
        technique: &str,
        value: fn(&str, u32, &L) -> [f32; 4],
    ) -> Result<Self, ScreenError> {
        let vertex_inputs = vertex_inputs(pack, effect, technique, INSTANCED).ok_or_else(|| {
            ScreenError::NoPass {
                effect: effect.into(),
                technique: technique.into(),
                pass: INSTANCED,
            }
        })?;
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let target = Target {
            colour,
            depth: DEPTH_STENCIL_FORMAT,
            depth_compare: wgpu::CompareFunction::Always,
            blend: Some(wgpu::BlendState {
                color: add,
                alpha: add,
            }),
            colour_attachments: Some(1),
        };
        let draw = DrawState {
            instance_locations: vertex_inputs
                .iter()
                .filter(|i| i.semantic != "POSITION")
                .map(|i| i.register)
                .collect(),
            // Only the far side of the sphere is drawn, which a camera inside it sees as well as one outside
            cull_mode: Some(wgpu::Face::Front),
            write_mask: Some(wgpu::ColorWrites::COLOR),
            // The depth of the frame is read as it is, and not filtered
            unfiltered: true,
            ..over_what_was_drawn()
        };
        let pass = ScreenPass::new(
            device,
            queue,
            pack,
            effect,
            technique,
            INSTANCED,
            &target,
            &draw,
            Space::Unit,
            &mut |name, requirement| provide(device, frame, name, requirement),
        )?;
        let marked = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Equal,
            ..draw.stencil.front
        };
        let in_room = ScreenPass::new(
            device,
            queue,
            pack,
            effect,
            technique,
            INSTANCED,
            &target,
            &DrawState {
                stencil: wgpu::StencilState {
                    front: marked,
                    back: marked,
                    read_mask: ROOM_BIT,
                    write_mask: 0,
                },
                ..draw.clone()
            },
            Space::Unit,
            &mut |name, requirement| provide(device, frame, name, requirement),
        )?;
        let streams = pass.streams();
        let corners = sphere();
        let mut bytes = Vec::with_capacity(corners.len() * streams.vertex_stride as usize);
        for corner in &corners {
            bytes.extend(record_bytes(
                &streams.vertex,
                streams.vertex_stride,
                &vertex_inputs,
                &|input| corner_value(&input.semantic, *corner),
            ));
        }
        let sphere = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("light-volume-sphere"),
            contents: &bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light-volumes"),
            size: streams.instance_stride.max(4) * MOST as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pass,
            in_room,
            inputs: vertex_inputs,
            sphere,
            corners: corners.len() as u32,
            instances,
            value,
            drawn: AtomicU32::new(0),
            written: Mutex::new(Vec::new()),
        })
    }

    /// Read `frame` anew, for targets of another size.
    pub fn bind(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &Inputs<'_>) {
        for pass in [&mut self.pass, &mut self.in_room] {
            pass.bind(device, queue, &mut |name, requirement| {
                provide(device, frame, name, requirement)
            });
        }
    }

    /// Write the constants of this frame and the first [`MOST`] of `lights`.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        lights: &[L],
    ) {
        let lights = &lights[..lights.len().min(MOST)];
        self.drawn.store(lights.len() as u32, Ordering::Relaxed);
        if lights.is_empty() {
            return;
        }
        let mut values = engine.values();
        values.extend(lighting_values(engine, screen));
        self.pass.write(queue, &values);
        self.in_room.write(queue, &values);
        // ⚠ Thousands of lights, each written attribute by attribute: only when they changed.
        let mut written = self.written.lock().unwrap_or_else(|p| p.into_inner());
        if written.as_slice() == lights {
            return;
        }
        let streams = self.pass.streams();
        let bytes = records(
            &streams.instance,
            streams.instance_stride,
            &self.inputs,
            lights,
            &|input, light| (self.value)(&input.semantic, input.index, light),
        );
        queue.write_buffer(&self.instances, 0, &bytes);
        written.clear();
        written.extend_from_slice(lights);
    }

    /// Draw the lights the frame was written with into `pass`: its colour target of the format this was built for, and the depth and stencil of the frame, which it only reads.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.record_range(pass, 0..u32::MAX, false);
    }

    /// Draw the lights of `range` the frame was written with into `pass`; `in_room` only over the pixels a room marked, the stencil's reference at its bit.
    pub fn record_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>, in_room: bool) {
        let drawn = self.drawn.load(Ordering::Relaxed);
        let range = range.start.min(drawn)..range.end.min(drawn);
        if range.is_empty() {
            return;
        }
        let screen = match in_room {
            true => &self.in_room,
            false => {
                pass.set_stencil_reference(MATERIAL_CLEAR);
                &self.pass
            }
        };
        screen.record_instanced(pass, &self.sphere, self.corners, &self.instances, range);
    }
}

#[cfg(test)]
mod tests;
