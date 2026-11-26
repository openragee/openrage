//! A pass of the pack drawn over the whole target by a quad: the game's lighting and post-processing passes, which read what other passes drew.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use or_io::gta5::shaders::spirv_pack::{ConstantBuffer, Program, SignatureElement};
use thiserror::Error;
use wgpu::util::DeviceExt;

use super::bindings::{Requirement, Resource};
use super::constants::Uniform;
use super::link::match_interpolation;
use super::pack::{GamePack, PackError};
use super::pipeline::{
    build_pass_pipeline_drawn, entry_of, vertex_layout, vertex_streams, DrawState, PassPipeline,
    PipelineError, Streams, Target,
};
use super::quad::pair_of;
use super::stand_in::{bind_resources_with, Provided};

#[derive(Debug, Error)]
pub enum ScreenError {
    #[error(transparent)]
    Pack(#[from] PackError),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error("the pack has no effect named {0}")]
    NoEffect(String),
    #[error("{effect} has no technique {technique} with a pass {pass}")]
    NoPass {
        effect: String,
        technique: String,
        pass: usize,
    },
}

/// The range the corners of the quad are written in, which a pass's vertex stage maps to the screen by its own rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// From nought to one on both axes: the lighting passes put it at `(position - 0.5) * 2`.
    Unit,
    /// From minus one to one: where the vertex stage takes the clip position as it is.
    Clip,
}

/// The six vertices of the quad, a triangle each of its two halves.
const CORNERS: [[f32; 2]; 6] = [
    [0.0, 0.0],
    [1.0, 0.0],
    [1.0, 1.0],
    [0.0, 0.0],
    [1.0, 1.0],
    [0.0, 1.0],
];

/// What one input of the vertex stage reads at `corner`, in `space`.
fn corner_value(semantic: &str, corner: [f32; 2], space: Space) -> [f32; 4] {
    let [u, v] = corner;
    match semantic {
        "POSITION" => match space {
            Space::Unit => [u, v, 0.0, 1.0],
            Space::Clip => [u * 2.0 - 1.0, v * 2.0 - 1.0, 0.0, 1.0],
        },
        // The top of the picture is the top of the target
        "TEXCOORD" => [u, 1.0 - v, 0.0, 0.0],
        _ => [0.0; 4],
    }
}

/// The vertices of the quad, `stride` bytes each, with each attribute written from the input at its location.
fn quad_vertices(
    attributes: &[wgpu::VertexAttribute],
    stride: u64,
    inputs: &[SignatureElement],
    space: Space,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(CORNERS.len() * stride as usize);
    for corner in CORNERS {
        let mut written = vec![0u8; stride as usize];
        for attribute in attributes {
            let values = inputs
                .iter()
                .find(|i| i.register == attribute.shader_location)
                .map_or([0.0; 4], |i| corner_value(&i.semantic, corner, space));
            let words = (attribute.format.size() / 4) as usize;
            for (i, value) in values.iter().take(words).enumerate() {
                let at = attribute.offset as usize + 4 * i;
                written[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        bytes.extend(written);
    }
    bytes
}

/// The constants of a pass whose vertex stage lays a unit quad over the whole of its target, the first corner at the top left, showing the whole of what it reads.
pub fn unit_quad() -> [(&'static str, Vec<f32>); 3] {
    [
        ("QuadPosition", vec![2.0, -2.0, -1.0, 1.0]),
        ("QuadTexCoords", vec![1.0, 1.0, 0.0, 0.0]),
        ("QuadScale", vec![1.0; 4]),
    ]
}

/// What the constants of a pass were last written for, so that a frame in which none of it changed writes none.
pub struct Written<K>(Mutex<Option<K>>);

impl<K> Default for Written<K> {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl<K: PartialEq + Copy> Written<K> {
    /// Whether `key` is not what was last written, and from then on it is.
    pub fn changed(&self, key: K) -> bool {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if *held == Some(key) {
            return false;
        }
        *held = Some(key);
        true
    }

    /// Forget what was written, for buffers that were made again.
    pub fn reset(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// One pass of an effect, ready to draw as a quad: its pipeline built, its resources bound and its buffers kept for a frame to write.
pub struct ScreenPass {
    pass: Arc<PassPipeline>,
    /// What each binding of the pass is called, by group and binding.
    names: BTreeMap<(u32, u32), String>,
    /// The layout of each constant buffer of the pass, by name.
    layouts: BTreeMap<String, ConstantBuffer>,
    /// What the effect declares for its variables, by name, as 32-bit words.
    defaults: BTreeMap<String, Vec<u32>>,
    groups: BTreeMap<u32, wgpu::BindGroup>,
    uniforms: Vec<Uniform>,
    vertices: wgpu::Buffer,
    streams: Streams,
}

/// The inputs of the vertex stage of pass `pass` of `technique` of `effect`, which a caller reads to say which of them step once per instance.
pub fn vertex_inputs(
    pack: &GamePack,
    effect: &str,
    technique: &str,
    pass: usize,
) -> Option<Vec<SignatureElement>> {
    let manifest = pack.manifest();
    let (vertex_key, _) = manifest
        .effects
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(effect))?
        .techniques
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(technique))?
        .passes
        .get(pass)
        .and_then(pair_of)?;
    Some(manifest.programs.get(vertex_key)?.reflection.inputs.clone())
}

impl ScreenPass {
    /// Pass number `pass` of technique `technique` of `effect`, drawn as `draw` says into `target`.
    ///
    /// `provide` is asked for each texture and sampler the pass binds, with the name the game's reflection gives it, and what it does not give is a stand-in.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        effect: &str,
        technique: &str,
        pass: usize,
        target: &Target,
        draw: &DrawState,
        space: Space,
        provide: &mut dyn FnMut(&str, &Requirement) -> Option<Provided>,
    ) -> Result<Self, ScreenError> {
        let manifest = pack.manifest();
        let found = manifest
            .effects
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(effect))
            .ok_or_else(|| ScreenError::NoEffect(effect.to_string()))?;
        let no_pass = || ScreenError::NoPass {
            effect: effect.to_string(),
            technique: technique.to_string(),
            pass,
        };
        let (vertex_key, pixel_key) = found
            .techniques
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(technique))
            .and_then(|t| t.passes.get(pass))
            .and_then(pair_of)
            .ok_or_else(no_pass)?;
        let vertex_module = pack.module(vertex_key)?;
        let pixel_module = pixel_key.map(|key| pack.module(key)).transpose()?;
        let mut vertex = vertex_module.clone();
        if let Some(pixel) = &pixel_module {
            match_interpolation(&mut vertex, pixel);
        }
        let built = build_pass_pipeline_drawn(
            device,
            &vertex,
            pixel_module.as_ref(),
            draw.depth_tested,
            target,
            draw,
        )?;

        let programs: Vec<&Program> = [Some(vertex_key), pixel_key]
            .into_iter()
            .flatten()
            .filter_map(|k| manifest.programs.get(k))
            .collect();
        let mut names = BTreeMap::new();
        let mut layouts = BTreeMap::new();
        for program in &programs {
            for binding in &program.reflection.bindings {
                names.insert((binding.set, binding.binding), binding.name.clone());
            }
            for layout in manifest.constant_buffers_of(program) {
                layouts.insert(layout.name.clone(), layout.clone());
            }
        }

        let entry = entry_of(&vertex_module, naga::ShaderStage::Vertex)?;
        let (attributes, stride) = vertex_layout(&vertex_module, entry)?;
        let streams = vertex_streams(&vertex_module, entry, &draw.instance_locations)?;
        let inputs = manifest
            .programs
            .get(vertex_key)
            .map(|p| p.reflection.inputs.as_slice())
            .unwrap_or_default();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &quad_vertices(&attributes, stride, inputs, space),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut screen = Self {
            pass: Arc::new(built),
            names,
            layouts,
            defaults: found.defaults.clone(),
            groups: BTreeMap::new(),
            uniforms: Vec::new(),
            vertices,
            streams,
        };
        screen.bind(device, queue, provide);
        Ok(screen)
    }

    /// The same pass bound to other resources: its pipeline is shared, and its buffers and bindings are its own.
    pub fn forked(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        provide: &mut dyn FnMut(&str, &Requirement) -> Option<Provided>,
    ) -> Self {
        let mut other = Self {
            pass: self.pass.clone(),
            names: self.names.clone(),
            layouts: self.layouts.clone(),
            defaults: self.defaults.clone(),
            groups: BTreeMap::new(),
            uniforms: Vec::new(),
            vertices: self.vertices.clone(),
            streams: self.streams.clone(),
        };
        other.bind(device, queue, provide);
        other
    }

    /// Bind the pass's resources again, for `provide` to give others, such as the targets of another size; the pipeline is kept and the buffers are new, so a frame writes them anew.
    pub fn bind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        provide: &mut dyn FnMut(&str, &Requirement) -> Option<Provided>,
    ) {
        let names = &self.names;
        let mut given = |requirement: &Requirement| -> Option<Provided> {
            let name = names.get(&(requirement.group, requirement.binding))?;
            match requirement.resource {
                Resource::Texture { .. } | Resource::Sampler { .. } => provide(name, requirement),
                _ => None,
            }
        };
        let bound = bind_resources_with(device, queue, &self.pass, &mut given);
        let mut uniforms = Vec::new();
        for requirements in self.pass.requirements.values() {
            for requirement in requirements {
                let Resource::Uniform { .. } = requirement.resource else {
                    continue;
                };
                let key = (requirement.group, requirement.binding);
                let (Some(buffer), Some(layout)) = (
                    bound.buffers.get(&key),
                    self.names.get(&key).and_then(|n| self.layouts.get(n)),
                ) else {
                    continue;
                };
                let defaults = layout
                    .variables
                    .iter()
                    .filter_map(|variable| {
                        let words = self.defaults.get(&variable.name)?;
                        let room = (variable.size / 4) as usize;
                        Some((
                            variable.name.clone(),
                            words[..words.len().min(room)].to_vec(),
                        ))
                    })
                    .collect();
                uniforms.push(Uniform::new(
                    buffer.clone(),
                    layout.clone(),
                    defaults,
                    Vec::new(),
                ));
            }
        }
        self.groups = bound.groups;
        self.uniforms = uniforms;
    }

    /// Write the constants of this frame: what the effect declares, then `values` over it.
    pub fn write(&self, queue: &wgpu::Queue, values: &BTreeMap<&'static str, Vec<f32>>) {
        for uniform in &self.uniforms {
            uniform.write(queue, values);
        }
    }

    /// Draw the quad into `pass`, which must have the targets this was built for.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pass.pipeline);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for (group, bind_group) in &self.groups {
            pass.set_bind_group(*group, bind_group, &[]);
        }
        pass.draw(0..CORNERS.len() as u32, 0..1);
    }

    /// Draw `vertices` vertices of `geometry` into `pass` in place of the quad, for a pass with no input that steps once per instance.
    pub fn record_mesh(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        geometry: &wgpu::Buffer,
        vertices: u32,
    ) {
        pass.set_pipeline(&self.pass.pipeline);
        pass.set_vertex_buffer(0, geometry.slice(..));
        for (group, bind_group) in &self.groups {
            pass.set_bind_group(*group, bind_group, &[]);
        }
        pass.draw(0..vertices, 0..1);
    }

    /// The vertex buffers the pass reads: the attributes of the one read once per vertex and of the one read once per instance.
    pub fn streams(&self) -> &Streams {
        &self.streams
    }

    /// Draw the instances of `range` of `vertices` vertices of `geometry` into `pass`, in place of the quad, with the per-instance inputs read from `instances`.
    pub fn record_instanced(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        geometry: &wgpu::Buffer,
        vertices: u32,
        instances: &wgpu::Buffer,
        range: std::ops::Range<u32>,
    ) {
        pass.set_pipeline(&self.pass.pipeline);
        pass.set_vertex_buffer(0, geometry.slice(..));
        pass.set_vertex_buffer(1, instances.slice(..));
        for (group, bind_group) in &self.groups {
            pass.set_bind_group(*group, bind_group, &[]);
        }
        pass.draw(0..vertices, range);
    }

    /// The format of each colour target the pass writes, by location.
    pub fn colour_formats(&self) -> &[Option<wgpu::TextureFormat>] {
        &self.pass.colour_formats
    }
}

#[cfg(test)]
mod tests;
