//! A render pipeline from the vertex and pixel `naga` modules of one pass, with the vertex buffer and colour targets their interfaces ask for.

use std::borrow::Cow;

use std::collections::BTreeMap;

use naga::{Binding, BuiltIn, ScalarKind, ShaderStage, TypeInner};
use thiserror::Error;

use super::bindings::{requirements, BindingError, Requirement};

/// What a pass's modules cannot be built into a pipeline for.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PipelineError {
    #[error("no {0:?} entry point")]
    NoEntryPoint(ShaderStage),
    #[error("an input that is not a 32-bit scalar or vector")]
    InputShape,
    #[error("an input of an unsupported kind")]
    InputKind,
    #[error("an output that is not a 32-bit scalar or vector")]
    OutputShape,
    #[error("an output of an unsupported kind")]
    OutputKind,
    #[error(transparent)]
    Bindings(#[from] BindingError),
    #[error("group {group} binding {binding} is two different resources in the two stages")]
    Conflict { group: u32, binding: u32 },
    /// The adapter's driver is not handed the pass, and why.
    #[error("{0}")]
    Driver(String),
}

/// What a pipeline draws into: the format of the first colour target, which a pass's colour output is written in, and the depth target with the comparison by which a nearer surface wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Target {
    pub colour: wgpu::TextureFormat,
    pub depth: wgpu::TextureFormat,
    pub depth_compare: wgpu::CompareFunction,
    /// How the first colour target takes what is drawn: it replaces what is there where this is `None`, and where it blends, the depth target is tested and not written.
    pub blend: Option<wgpu::BlendState>,
    /// How many colour attachments the pass has, where it has fewer than the pass of the game writes: the device ignores what a shader writes at a location past them. `None` gives every output its own.
    pub colour_attachments: Option<usize>,
}

impl Default for Target {
    fn default() -> Self {
        Self {
            colour: wgpu::TextureFormat::Rgba8Unorm,
            depth: wgpu::TextureFormat::Depth32Float,
            depth_compare: wgpu::CompareFunction::Less,
            blend: None,
            colour_attachments: None,
        }
    }
}

fn float_formats() -> [wgpu::TextureFormat; 4] {
    use wgpu::TextureFormat as F;
    [F::R8Unorm, F::Rg8Unorm, F::Rgba8Unorm, F::Rgba8Unorm]
}

fn uint_formats() -> [wgpu::TextureFormat; 4] {
    use wgpu::TextureFormat as F;
    [F::R8Uint, F::Rg8Uint, F::Rgba8Uint, F::Rgba8Uint]
}

fn sint_formats() -> [wgpu::TextureFormat; 4] {
    use wgpu::TextureFormat as F;
    [F::R8Sint, F::Rg8Sint, F::Rgba8Sint, F::Rgba8Sint]
}

/// The kind and component count of a 32-bit scalar or vector type.
pub fn shape(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Option<(ScalarKind, usize)> {
    match module.types[ty].inner {
        TypeInner::Scalar(s) if s.width == 4 => Some((s.kind, 1)),
        TypeInner::Vector { size, scalar } if scalar.width == 4 => {
            Some((scalar.kind, size as usize))
        }
        _ => None,
    }
}

/// The vertex format of `count` 32-bit components of `kind`.
pub fn vertex_format(kind: ScalarKind, count: usize) -> Option<wgpu::VertexFormat> {
    use wgpu::VertexFormat as V;
    Some(match (kind, count) {
        (ScalarKind::Float, 1) => V::Float32,
        (ScalarKind::Float, 2) => V::Float32x2,
        (ScalarKind::Float, 3) => V::Float32x3,
        (ScalarKind::Float, 4) => V::Float32x4,
        (ScalarKind::Uint, 1) => V::Uint32,
        (ScalarKind::Uint, 2) => V::Uint32x2,
        (ScalarKind::Uint, 3) => V::Uint32x3,
        (ScalarKind::Uint, 4) => V::Uint32x4,
        (ScalarKind::Sint, 1) => V::Sint32,
        (ScalarKind::Sint, 2) => V::Sint32x2,
        (ScalarKind::Sint, 3) => V::Sint32x3,
        (ScalarKind::Sint, 4) => V::Sint32x4,
        _ => return None,
    })
}

/// The one vertex buffer an entry point's inputs read from, each attribute 32-bit components packed one after the other, and its stride.
pub fn vertex_layout(
    module: &naga::Module,
    entry: &naga::EntryPoint,
) -> Result<(Vec<wgpu::VertexAttribute>, u64), PipelineError> {
    let streams = vertex_streams(module, entry, &[])?;
    Ok((streams.vertex, streams.vertex_stride))
}

/// An entry point's inputs split between the buffer read once per vertex and the one read once per instance, each packed in the order the entry point declares them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Streams {
    pub vertex: Vec<wgpu::VertexAttribute>,
    /// At least four bytes, since a buffer has a stride whatever it holds.
    pub vertex_stride: u64,
    pub instance: Vec<wgpu::VertexAttribute>,
    /// Zero where no input is read once per instance.
    pub instance_stride: u64,
}

/// [`vertex_layout`] with the inputs at `instance_locations` read once per instance from a second buffer.
pub fn vertex_streams(
    module: &naga::Module,
    entry: &naga::EntryPoint,
    instance_locations: &[u32],
) -> Result<Streams, PipelineError> {
    let mut streams = Streams::default();
    for argument in &entry.function.arguments {
        let Some(Binding::Location { location, .. }) = &argument.binding else {
            continue;
        };
        let (kind, count) = shape(module, argument.ty).ok_or(PipelineError::InputShape)?;
        let format = vertex_format(kind, count).ok_or(PipelineError::InputKind)?;
        let (attributes, stride) = if instance_locations.contains(location) {
            (&mut streams.instance, &mut streams.instance_stride)
        } else {
            (&mut streams.vertex, &mut streams.vertex_stride)
        };
        attributes.push(wgpu::VertexAttribute {
            format,
            offset: *stride,
            shader_location: *location,
        });
        *stride += 4 * count as u64;
    }
    streams.vertex_stride = streams.vertex_stride.max(4);
    Ok(streams)
}

/// How a pass is drawn beyond its [`Target`]: which inputs step once per instance, how its vertices make primitives, whether it culls, and what its depth target does.
#[derive(Debug, Clone, Default)]
pub struct DrawState {
    /// The vertex input locations read once per instance, from a second buffer after the per-vertex one.
    pub instance_locations: Vec<u32>,
    pub topology: wgpu::PrimitiveTopology,
    pub cull_mode: Option<wgpu::Face>,
    /// Whether the pass writes depth where it tests it, which a pass that blends does not unless this says so.
    pub depth_write: Option<bool>,
    /// Whether the pipeline has a depth target it never tests, for a pass drawn into a target that has one.
    pub depth_attachment: bool,
    /// Whether the pass tests the depth target by the target's `depth_compare`, and not only has it.
    pub depth_tested: bool,
    /// What the stencil of the depth target does: nothing by default.
    pub stencil: wgpu::StencilState,
    /// The channels of the first colour target the pass writes: all of them by default.
    pub write_mask: Option<wgpu::ColorWrites>,
    /// How the depth a pass writes is pushed away from the surface, by a constant and by how steep the surface is.
    pub depth_bias: wgpu::DepthBiasState,
    /// Whether the pass's float textures are laid out as ones that are not filtered, and the samplers that read them with them, which is what a depth texture needs to be bound as one.
    pub unfiltered: bool,
    /// The bindings, by group and number, that are laid out as unfiltered whatever `unfiltered` says: the one depth map a pass of many textures reads.
    pub unfiltered_at: Vec<(u32, u32)>,
}

/// The colour targets a fragment entry point writes, by location, and whether it writes depth.
pub fn fragment_targets(
    module: &naga::Module,
    entry: &naga::EntryPoint,
) -> Result<(Vec<Option<wgpu::ColorTargetState>>, bool), PipelineError> {
    fragment_targets_in(module, entry, &Target::default())
}

/// [`fragment_targets`] where a colour of three or four floats written at location zero goes to the colour format of `target`, and so does a single float where that format has one channel.
pub fn fragment_targets_in(
    module: &naga::Module,
    entry: &naga::EntryPoint,
    target: &Target,
) -> Result<(Vec<Option<wgpu::ColorTargetState>>, bool), PipelineError> {
    let mut outputs: Vec<(u32, wgpu::TextureFormat)> = Vec::new();
    let mut depth = false;
    let mut add = |binding: &Option<Binding>,
                   ty: naga::Handle<naga::Type>|
     -> Result<(), PipelineError> {
        match binding {
            Some(Binding::Location { location, .. }) => {
                let (kind, count) = shape(module, ty).ok_or(PipelineError::OutputShape)?;
                let table = match kind {
                    ScalarKind::Float => float_formats(),
                    ScalarKind::Uint => uint_formats(),
                    ScalarKind::Sint => sint_formats(),
                    _ => return Err(PipelineError::OutputKind),
                };
                let format = match (*location, kind, count) {
                    (0, ScalarKind::Float, 3 | 4) => target.colour,
                    (0, ScalarKind::Float, 1) if target.colour.components() == 1 => target.colour,
                    _ => table[count.clamp(1, 4) - 1],
                };
                outputs.push((*location, format));
            }
            Some(Binding::BuiltIn(BuiltIn::FragDepth)) => depth = true,
            _ => {}
        }
        Ok(())
    };
    if let Some(result) = &entry.function.result {
        match &module.types[result.ty].inner {
            TypeInner::Struct { members, .. } => {
                for member in members {
                    add(&member.binding, member.ty)?;
                }
            }
            _ => add(&result.binding, result.ty)?,
        }
    }
    if let Some(attachments) = target.colour_attachments {
        outputs.retain(|(location, _)| (*location as usize) < attachments);
    }
    let slots = outputs
        .iter()
        .map(|(l, _)| *l as usize + 1)
        .max()
        .unwrap_or(0);
    let mut targets: Vec<Option<wgpu::ColorTargetState>> = vec![None; slots];
    for (location, format) in outputs {
        targets[location as usize] = Some(wgpu::ColorTargetState {
            format,
            blend: if location == 0 { target.blend } else { None },
            write_mask: wgpu::ColorWrites::ALL,
        });
    }
    // A pass of several attachments takes only a pipeline that names every one of them, so what the shader writes nothing to is kept as it is
    if let Some(attachments) = target.colour_attachments.filter(|n| *n > 1) {
        targets.resize(attachments, None);
        for slot in targets.iter_mut().filter(|slot| slot.is_none()) {
            *slot = Some(wgpu::ColorTargetState {
                format: target.colour,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            });
        }
    }
    Ok((targets, depth))
}

/// The entry point of `module` that runs at `stage`.
pub fn entry_of(
    module: &naga::Module,
    stage: ShaderStage,
) -> Result<&naga::EntryPoint, PipelineError> {
    module
        .entry_points
        .iter()
        .find(|e| e.stage == stage)
        .ok_or(PipelineError::NoEntryPoint(stage))
}

/// A render pipeline from the vertex stage `vertex` and, when given, the pixel stage `fragment`, with `layout` or one the device derives from the modules.
///
/// The vertex buffer is the packed one [`vertex_layout`] describes and the colour targets are [`fragment_targets`]'s, and a pass that writes no colour gets a depth target, since a pipeline needs one. With `depth_test`, the pipeline tests and writes depth, a nearer surface winning. A pair the device refuses is reported through the device's own error scopes and not here.
pub fn create_render_pipeline(
    device: &wgpu::Device,
    vertex: &naga::Module,
    fragment: Option<&naga::Module>,
    layout: Option<&wgpu::PipelineLayout>,
    depth_test: bool,
) -> Result<wgpu::RenderPipeline, PipelineError> {
    create_render_pipeline_in(
        device,
        vertex,
        fragment,
        layout,
        depth_test,
        &Target::default(),
    )
}

/// [`create_render_pipeline`] for the colour and depth targets of `target`.
pub fn create_render_pipeline_in(
    device: &wgpu::Device,
    vertex: &naga::Module,
    fragment: Option<&naga::Module>,
    layout: Option<&wgpu::PipelineLayout>,
    depth_test: bool,
    target: &Target,
) -> Result<wgpu::RenderPipeline, PipelineError> {
    create_render_pipeline_drawn(
        device,
        vertex,
        fragment,
        layout,
        depth_test,
        target,
        &DrawState::default(),
    )
}

/// [`create_render_pipeline_in`] drawn as `draw` says; the per-vertex buffer is always slot 0, and the per-instance one slot 1 where an input is read per instance.
pub fn create_render_pipeline_drawn(
    device: &wgpu::Device,
    vertex: &naga::Module,
    fragment: Option<&naga::Module>,
    layout: Option<&wgpu::PipelineLayout>,
    depth_test: bool,
    target: &Target,
    draw: &DrawState,
) -> Result<wgpu::RenderPipeline, PipelineError> {
    let module = |module: &naga::Module| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
        })
    };
    let vertex_entry = entry_of(vertex, ShaderStage::Vertex)?;
    let streams = vertex_streams(vertex, vertex_entry, &draw.instance_locations)?;
    let mut buffers = vec![wgpu::VertexBufferLayout {
        array_stride: streams.vertex_stride,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &streams.vertex,
    }];
    if !streams.instance.is_empty() {
        buffers.push(wgpu::VertexBufferLayout {
            array_stride: streams.instance_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &streams.instance,
        });
    }
    let vertex_shader = module(vertex);
    let fragment_parts = match fragment {
        Some(fragment) => {
            let entry = entry_of(fragment, ShaderStage::Fragment)?;
            let (mut targets, writes_depth) = fragment_targets_in(fragment, entry, target)?;
            if let Some(Some(first)) = targets.first_mut() {
                first.write_mask = draw.write_mask.unwrap_or(wgpu::ColorWrites::ALL);
            }
            Some((module(fragment), entry, targets, writes_depth))
        }
        None => None,
    };
    let writes_depth = fragment_parts
        .as_ref()
        .is_some_and(|(_, _, _, depth)| *depth);
    let no_colour = fragment_parts
        .as_ref()
        .is_none_or(|(_, _, targets, _)| targets.is_empty());
    let depth_stencil = if depth_test {
        Some(wgpu::DepthStencilState {
            format: target.depth,
            depth_write_enabled: Some(draw.depth_write.unwrap_or(target.blend.is_none())),
            depth_compare: Some(target.depth_compare),
            stencil: draw.stencil.clone(),
            bias: draw.depth_bias,
        })
    } else {
        (writes_depth || no_colour || draw.depth_attachment).then(|| wgpu::DepthStencilState {
            format: target.depth,
            depth_write_enabled: Some(writes_depth || draw.depth_write == Some(true)),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: draw.stencil.clone(),
            bias: draw.depth_bias,
        })
    };
    Ok(
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout,
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some(&vertex_entry.name),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: draw.topology,
                cull_mode: draw.cull_mode,
                ..Default::default()
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            fragment: fragment_parts.as_ref().map(|(shader, entry, targets, _)| {
                wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(&entry.name),
                    compilation_options: Default::default(),
                    targets,
                }
            }),
            multiview_mask: None,
            cache: None,
        }),
    )
}

/// A pass's pipeline, with what each of its bind groups must hold.
pub struct PassPipeline {
    pub pipeline: wgpu::RenderPipeline,
    /// The layout of each group either stage uses.
    pub layouts: BTreeMap<u32, wgpu::BindGroupLayout>,
    /// What each of those groups holds, by binding.
    pub requirements: BTreeMap<u32, Vec<Requirement>>,
    /// The bytes between two vertices of the vertex buffer the pipeline reads.
    pub vertex_stride: u64,
    /// The bytes between two instances of the second vertex buffer, which is zero where no input is read once per instance.
    pub instance_stride: u64,
    /// The format of each colour target the pixel stage writes, by location.
    pub colour_formats: Vec<Option<wgpu::TextureFormat>>,
    /// Whether the pipeline has a depth target: for a pass that writes depth or no colour, or one built to test depth.
    pub depth: bool,
    /// Whether the pipeline tests depth, a nearer surface winning.
    pub depth_test: bool,
}

/// Each group's bindings: what they hold, and the stages that see them.
type Groups = BTreeMap<u32, BTreeMap<u32, (Requirement, wgpu::ShaderStages)>>;

/// What each group holds for the stages of a pass, and the stages it is visible to.
fn merge_groups(stages: &[(&naga::Module, wgpu::ShaderStages)]) -> Result<Groups, PipelineError> {
    let mut groups = Groups::new();
    for (module, visibility) in stages {
        for requirement in requirements(module)? {
            let slot = groups.entry(requirement.group).or_default();
            match slot.get_mut(&requirement.binding) {
                None => {
                    slot.insert(requirement.binding, (requirement, *visibility));
                }
                Some((existing, seen)) if existing.resource == requirement.resource => {
                    *seen |= *visibility;
                }
                Some(_) => {
                    return Err(PipelineError::Conflict {
                        group: requirement.group,
                        binding: requirement.binding,
                    })
                }
            }
        }
    }
    Ok(groups)
}

/// [`merge_groups`] for a pass built on the adapter `info`, whose first stage is its vertex
/// stage: refused where that adapter's driver ends the program on such a stage.
fn groups_on(
    info: &wgpu::AdapterInfo,
    stages: &[(&naga::Module, wgpu::ShaderStages)],
) -> Result<Groups, PipelineError> {
    let vertex = stages.first().map(|(vertex, _)| *vertex);
    match vertex.and_then(|vertex| crate::instance::refused_by_driver(info, vertex)) {
        Some(why) => Err(PipelineError::Driver(why)),
        None => merge_groups(stages),
    }
}

/// `ty` as a binding that is never filtered: a float texture that is not, and a sampler that does not.
fn unfiltered(ty: wgpu::BindingType) -> wgpu::BindingType {
    match ty {
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { .. },
            view_dimension,
            multisampled,
        } => wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension,
            multisampled,
        },
        wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering) => {
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering)
        }
        other => other,
    }
}

/// A pass's pipeline with the bind group layouts its modules ask for, which are the shaders' own and so cannot disagree with them.
pub fn build_pass_pipeline(
    device: &wgpu::Device,
    vertex: &naga::Module,
    fragment: Option<&naga::Module>,
) -> Result<PassPipeline, PipelineError> {
    build_pass_pipeline_with(device, vertex, fragment, false)
}

/// [`build_pass_pipeline`], testing and writing depth when `depth_test`.
pub fn build_pass_pipeline_with(
    device: &wgpu::Device,
    vertex: &naga::Module,
    fragment: Option<&naga::Module>,
    depth_test: bool,
) -> Result<PassPipeline, PipelineError> {
    build_pass_pipeline_in(device, vertex, fragment, depth_test, &Target::default())
}

/// [`build_pass_pipeline_with`] for the colour and depth targets of `target`.
pub fn build_pass_pipeline_in(
    device: &wgpu::Device,
    vertex: &naga::Module,
    fragment: Option<&naga::Module>,
    depth_test: bool,
    target: &Target,
) -> Result<PassPipeline, PipelineError> {
    build_pass_pipeline_drawn(
        device,
        vertex,
        fragment,
        depth_test,
        target,
        &DrawState::default(),
    )
}

/// [`build_pass_pipeline_in`] drawn as `draw` says.
pub fn build_pass_pipeline_drawn(
    device: &wgpu::Device,
    vertex: &naga::Module,
    fragment: Option<&naga::Module>,
    depth_test: bool,
    target: &Target,
    draw: &DrawState,
) -> Result<PassPipeline, PipelineError> {
    let mut stages = vec![(vertex, wgpu::ShaderStages::VERTEX)];
    if let Some(fragment) = fragment {
        stages.push((fragment, wgpu::ShaderStages::FRAGMENT));
    }
    let groups = groups_on(&device.adapter_info(), &stages)?;
    let mut layouts = BTreeMap::new();
    let mut required = BTreeMap::new();
    for (group, bindings) in &groups {
        let entries: Vec<_> = bindings
            .values()
            .map(|(requirement, visibility)| {
                let mut entry = requirement
                    .resource
                    .layout_entry(requirement.binding, *visibility);
                if draw.unfiltered || draw.unfiltered_at.contains(&(*group, requirement.binding)) {
                    entry.ty = unfiltered(entry.ty);
                }
                entry
            })
            .collect();
        layouts.insert(
            *group,
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &entries,
            }),
        );
        required.insert(
            *group,
            bindings
                .values()
                .map(|(r, _)| r.clone())
                .collect::<Vec<_>>(),
        );
    }
    // A group no stage uses is a gap in the list, which the device allows
    let slots = groups
        .keys()
        .next_back()
        .map_or(0, |last| *last as usize + 1);
    let in_order: Vec<Option<&wgpu::BindGroupLayout>> =
        (0..slots as u32).map(|g| layouts.get(&g)).collect();
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &in_order,
        immediate_size: 0,
    });
    let pipeline = create_render_pipeline_drawn(
        device,
        vertex,
        fragment,
        Some(&pipeline_layout),
        depth_test,
        target,
        draw,
    )?;
    let vertex_entry = entry_of(vertex, ShaderStage::Vertex)?;
    let streams = vertex_streams(vertex, vertex_entry, &draw.instance_locations)?;
    let (colour_formats, writes_depth) = match fragment {
        Some(fragment) => {
            let (targets, depth) =
                fragment_targets_in(fragment, entry_of(fragment, ShaderStage::Fragment)?, target)?;
            (
                targets
                    .iter()
                    .map(|t| t.as_ref().map(|t| t.format))
                    .collect(),
                depth,
            )
        }
        None => (Vec::new(), false),
    };
    let depth = depth_test || writes_depth || colour_formats.is_empty() || draw.depth_attachment;
    Ok(PassPipeline {
        pipeline,
        layouts,
        requirements: required,
        vertex_stride: streams.vertex_stride,
        instance_stride: streams.instance_stride,
        colour_formats,
        depth,
        depth_test,
    })
}

#[cfg(test)]
mod tests;
