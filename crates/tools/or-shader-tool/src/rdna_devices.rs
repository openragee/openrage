//! GTA VI's native programs, translated, built into a pipeline on every adapter the machine has: each
//! adapter's own shader compiler runs over what naga wrote, in child processes a crash only ends one of.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use naga::ShaderStage;
use or_io::gta5::shaders::sgd5::Sgd5File;
use or_io::gta5::shaders::sgd5_pack::vertex_pixel_passes;
use or_renderer::game_shaders::bindings::arrayed;
use or_renderer::game_shaders::pipeline::{entry_of, fragment_targets, vertex_streams};
use or_renderer::instance;
use or_shader_front::rdna::link::{self, Linked};
use or_shader_front::rdna::{self, RdnaError};
use or_shader_front::rdna_bind;
use sha1::{Digest, Sha1};

use crate::device_check::{matching_vertex_shader, one_line};
use crate::rdna_report::{
    missing_feature, Adapter, Cache, Container, Line, Outcome, Report, Run, VERSION,
};

/// The mark of a line a worker means its parent to read: a driver may write to the same stream.
const MARK: &str = "@@";

/// One adapter with a device open on it, taking every feature and limit the adapter offers.
pub struct Gpu {
    pub info: wgpu::AdapterInfo,
    pub features: wgpu::Features,
    pub limits: wgpu::Limits,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Whether it updates a texture of 64-bit texels in a step, which the feature alone does not say.
    pub updates_64_bit_textures: bool,
}

/// Whether a device is opened on the adapter `info`: its name and backend hold `only`, whatever
/// its case, and it is a CPU's where the `software` adapter alone was asked for.
fn wanted(info: &wgpu::AdapterInfo, only: Option<&str>, software: bool) -> bool {
    let named = format!("{} ({:?})", info.name, info.backend).to_lowercase();
    only.is_none_or(|only| named.contains(&only.to_lowercase()))
        && !(software && info.device_type != wgpu::DeviceType::Cpu)
}

/// Every adapter of every backend OpenRage draws through, the software ones too, in wgpu's order,
/// Direct3D 12 compiling as the app's does; `only` keeps those whose name and backend hold that
/// text, whatever its case. No OpenGL adapter is opened: OpenRage does not support it. With
/// [`instance::SOFTWARE_ADAPTER`] set, no device is opened on an adapter that is not a CPU's.
pub fn open_all(only: Option<&str>) -> Vec<Gpu> {
    let software = instance::software_asked();
    let backends = instance::backends_from_env().unwrap_or(wgpu::Backends::empty());
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    descriptor.backends = backends;
    let instance = wgpu::Instance::new(instance::with_dxc(descriptor));
    let adapters = pollster::block_on(instance.enumerate_adapters(backends));
    // A backend may list one adapter once per output it drives.
    let mut seen: Vec<wgpu::AdapterInfo> = Vec::new();
    (adapters.into_iter())
        .filter_map(|adapter| {
            let info = adapter.get_info();
            if seen.contains(&info) {
                return None;
            }
            seen.push(info.clone());
            if !wanted(&info, only, software) {
                return None;
            }
            let named = format!("{} ({:?})", info.name, info.backend);
            // An experimental feature is refused unless the device is opened unsafely for it.
            let features = adapter.features() - wgpu::Features::all_experimental_mask();
            let limits = adapter.limits();
            let updates_64_bit_textures = instance::updates_64_bit_textures(&adapter);
            let opened = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("or-shader-tool"),
                required_features: features,
                required_limits: limits.clone(),
                ..Default::default()
            }));
            match opened {
                Ok((device, queue)) => {
                    device.on_uncaptured_error(Arc::new(|_| {}));
                    Some(Gpu {
                        info,
                        features,
                        limits,
                        device,
                        queue,
                        updates_64_bit_textures,
                    })
                }
                Err(e) => {
                    tracing::warn!("{named} opens no device: {e}");
                    None
                }
            }
        })
        .collect()
}

impl Gpu {
    /// The adapter as a report names it.
    pub fn described(&self) -> Adapter {
        // ponytail: the limits are read off their debug text, which holds one `name: number` a line; a limit that is not a number is left out.
        let limits = (format!("{:#?}", self.limits).lines())
            .filter_map(|line| {
                let (name, value) = line.trim().trim_end_matches(',').split_once(": ")?;
                Some((name.to_owned(), value.parse().ok()?))
            })
            .collect();
        Adapter {
            name: self.info.name.clone(),
            vendor: self.info.vendor,
            device: self.info.device,
            device_type: format!("{:?}", self.info.device_type),
            driver: self.info.driver.clone(),
            driver_info: self.info.driver_info.clone(),
            backend: format!("{:?}", self.info.backend),
            shader_compiler: match self.info.backend {
                wgpu::Backend::Dx12 => match instance::dx12_compiler() {
                    wgpu::Dx12Compiler::Fxc => "fxc".into(),
                    wgpu::Dx12Compiler::Auto => "auto".into(),
                    _ => "dxc".into(),
                },
                _ => String::new(),
            },
            features: (self.features.iter_names())
                .map(|(name, _)| name.to_owned())
                .collect(),
            limits,
        }
    }

    /// The file a pipeline cache of this adapter is saved as, where its backend has one.
    pub fn cache_key(&self) -> Option<String> {
        (self.features.contains(wgpu::Features::PIPELINE_CACHE))
            .then(|| wgpu::util::pipeline_cache_key(&self.info))
            .flatten()
    }
}

/// The modules of one pipeline.
// One is held at a time, so the room a pair takes costs nothing.
#[allow(clippy::large_enum_variant)]
pub enum Stages {
    /// A vertex stage and, where there is one, a pixel stage.
    Render(naga::Module, Option<naga::Module>),
    Compute(naga::Module),
}

/// The entries of each bind group the modules ask for, the bytes a buffer of each holds, and
/// whether a module binds an array. `None` where a module binds something the renderer's own
/// layouts do not take.
type Groups = BTreeMap<u32, Vec<(wgpu::BindGroupLayoutEntry, u64)>>;

/// `module` with its textures of 64-bit texels declared with a format the renderer's layouts
/// name, or `None` where it has none: the layouts know no format of 64-bit texels.
fn narrowed(module: &naga::Module) -> Option<naga::Module> {
    use naga::StorageFormat as Format;
    let wide: Vec<_> = (module.types.iter())
        .filter(|(_, ty)| is_wide(&ty.inner))
        .map(|(handle, ty)| (handle, ty.clone()))
        .collect();
    let mut module = (!wide.is_empty()).then(|| module.clone())?;
    for (handle, mut ty) in wide {
        // A type the module does not hold yet: an arena holds each type once.
        for narrow in [
            Format::R32Uint,
            Format::Rg32Uint,
            Format::Rgba32Uint,
            Format::Rgba16Uint,
        ] {
            if let naga::TypeInner::Image {
                class: naga::ImageClass::Storage { format, .. },
                ..
            } = &mut ty.inner
            {
                *format = narrow;
            }
            if module.types.get(&ty).is_none() {
                module.types.replace(handle, ty);
                break;
            }
        }
    }
    Some(module)
}

fn is_wide(inner: &naga::TypeInner) -> bool {
    matches!(
        inner,
        naga::TypeInner::Image {
            class: naga::ImageClass::Storage {
                format: naga::StorageFormat::R64Uint,
                ..
            },
            ..
        }
    )
}

fn groups(modules: &[(&naga::Module, wgpu::ShaderStages)]) -> Option<(Groups, bool)> {
    use or_renderer::game_shaders::bindings::Resource;
    let mut groups = Groups::new();
    let mut arrays = false;
    for (module, visibility) in modules {
        // The textures of 64-bit texels, which the layout states as they are.
        let wide = |group: u32, binding: u32| {
            module.global_variables.iter().any(|(_, global)| {
                let here = (global.binding.as_ref())
                    .is_some_and(|at| (at.group, at.binding) == (group, binding));
                let ty = match module.types[global.ty].inner {
                    naga::TypeInner::BindingArray { base, .. } => base,
                    _ => global.ty,
                };
                here && is_wide(&module.types[ty].inner)
            })
        };
        let narrowed = narrowed(module);
        // The textures the module updates in a step, which a layout has to say of them.
        let updated = |group: u32, binding: u32| {
            module.global_variables.iter().any(|(_, global)| {
                let here = (global.binding.as_ref())
                    .is_some_and(|at| (at.group, at.binding) == (group, binding));
                let ty = match module.types[global.ty].inner {
                    naga::TypeInner::BindingArray { base, .. } => base,
                    _ => global.ty,
                };
                here && matches!(module.types[ty].inner, naga::TypeInner::Image {
                    class: naga::ImageClass::Storage { access, .. },
                    ..
                } if access.contains(naga::StorageAccess::ATOMIC))
            })
        };
        for (requirement, count) in arrayed(narrowed.as_ref().unwrap_or(module)).ok()? {
            arrays |= count.is_some();
            let mut entry = (requirement.resource).layout_entry(requirement.binding, *visibility);
            if let wgpu::BindingType::StorageTexture { access, format, .. } = &mut entry.ty {
                if updated(requirement.group, requirement.binding) {
                    *access = wgpu::StorageTextureAccess::Atomic;
                }
                if wide(requirement.group, requirement.binding) {
                    *format = wgpu::TextureFormat::R64Uint;
                }
            }
            entry.count = count;
            let size = match requirement.resource {
                Resource::Uniform { size } | Resource::Storage { size, .. } => size,
                _ => 0,
            };
            (groups.entry(requirement.group).or_default()).push((entry, size));
        }
    }
    Some((groups, arrays))
}

/// A pipeline's layout stated from `groups`, with the layout of each group, `None` at a number
/// no module binds in.
// A layout the device derives from a module states no array's length, and takes each for one entry.
fn stated_layout(
    device: &wgpu::Device,
    groups: &Groups,
) -> (wgpu::PipelineLayout, Vec<Option<wgpu::BindGroupLayout>>) {
    let last = groups.keys().next_back().map_or(0, |last| last + 1);
    let layouts: Vec<Option<wgpu::BindGroupLayout>> = (0..last)
        .map(|group| {
            let entries: Vec<_> = (groups.get(&group)?.iter())
                .map(|(entry, _)| *entry)
                .collect();
            Some(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &entries,
                }),
            )
        })
        .collect();
    let stated: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Option::as_ref).collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &stated,
        immediate_size: 0,
    });
    (layout, layouts)
}

/// What a check does with a pipeline a device creates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Draw {
    /// Nothing.
    #[default]
    No,
    /// A draw of no vertex or a dispatch of no group: the device checks the pipeline against all
    /// it is bound and drawn with, and runs none of it.
    Empty,
    /// [`RUN_VERTICES`] vertices drawn as points, or one group dispatched, so the program runs
    /// on the stand-ins: on a software adapter alone ([`RUN_ON_SOFTWARE`]).
    Run,
}

/// The vertices a program that is run is drawn with.
const RUN_VERTICES: u32 = 3;

/// What a unit is skipped for where it was to be run and the adapter is no CPU's: a program fed
/// stand-ins need not end, and a draw that does not end resets a hardware driver.
pub const RUN_ON_SOFTWARE: &str = "a program is run on a software adapter alone";

/// A pipeline the check created, and what a draw with it takes.
enum Pipeline {
    Render {
        pipeline: wgpu::RenderPipeline,
        vertex_stride: u64,
        colours: Vec<Option<wgpu::TextureFormat>>,
        depth: bool,
    },
    Compute(wgpu::ComputePipeline),
}

struct Made {
    pipeline: Pipeline,
    /// The stated layout's groups, where the layout was stated.
    stated: Option<(Groups, Vec<Option<wgpu::BindGroupLayout>>)>,
}

/// The pipeline of `stages`, its bind group layouts the ones the device derives from what the
/// modules declare, or [`stated_layout`]'s where a module binds an array or `state` asks.
fn create(
    device: &wgpu::Device,
    stages: &Stages,
    cache: Option<&wgpu::PipelineCache>,
    draw: Draw,
) -> Result<Made, String> {
    let state = draw != Draw::No;
    let shader = |module: &naga::Module| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
        })
    };
    let said = |e: or_renderer::game_shaders::pipeline::PipelineError| format!("the check: {e}");
    let stated = |modules: &[(&naga::Module, wgpu::ShaderStages)]| {
        let (groups, arrays) = groups(modules)?;
        (arrays || state).then(|| (stated_layout(device, &groups), groups))
    };
    match stages {
        Stages::Compute(module) => {
            let entry = entry_of(module, ShaderStage::Compute).map_err(said)?;
            let stated = stated(&[(module, wgpu::ShaderStages::COMPUTE)]);
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: stated.as_ref().map(|((layout, _), _)| layout),
                module: &shader(module),
                entry_point: Some(&entry.name),
                compilation_options: Default::default(),
                cache,
            });
            Ok(Made {
                pipeline: Pipeline::Compute(pipeline),
                stated: stated.map(|((_, layouts), groups)| (groups, layouts)),
            })
        }
        Stages::Render(vertex, fragment) => {
            let entry = entry_of(vertex, ShaderStage::Vertex).map_err(said)?;
            let streams = vertex_streams(vertex, entry, &[]).map_err(said)?;
            let pixel = match fragment {
                Some(fragment) => {
                    let entry = entry_of(fragment, ShaderStage::Fragment).map_err(said)?;
                    let (targets, depth) = fragment_targets(fragment, entry).map_err(said)?;
                    Some((shader(fragment), entry, targets, depth))
                }
                None => None,
            };
            let writes_depth = pixel.as_ref().is_some_and(|(_, _, _, depth)| *depth);
            // A pipeline needs a target, so one that writes no colour gets the depth target.
            let no_colour = (pixel.as_ref()).is_none_or(|(_, _, targets, _)| targets.is_empty());
            let mut modules = vec![(vertex, wgpu::ShaderStages::VERTEX)];
            modules.extend((fragment.iter()).map(|pixel| (pixel, wgpu::ShaderStages::FRAGMENT)));
            let stated = stated(&modules);
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: stated.as_ref().map(|((layout, _), _)| layout),
                vertex: wgpu::VertexState {
                    module: &shader(vertex),
                    entry_point: Some(&entry.name),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: streams.vertex_stride,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &streams.vertex,
                    }],
                },
                // Points where the program is run: a stage fed noughts gives three corners at one
                // place, a triangle of which covers no pixel, and a point at the middle covers one.
                primitive: wgpu::PrimitiveState {
                    topology: match draw {
                        Draw::Run => wgpu::PrimitiveTopology::PointList,
                        _ => wgpu::PrimitiveTopology::TriangleList,
                    },
                    ..Default::default()
                },
                depth_stencil: (writes_depth || no_colour).then(|| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(writes_depth),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: (pixel.as_ref()).map(|(module, entry, targets, _)| wgpu::FragmentState {
                    module,
                    entry_point: Some(&entry.name),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache,
            });
            let colours = (pixel.iter().flat_map(|(_, _, targets, _)| targets))
                .map(|target| target.as_ref().map(|target| target.format))
                .collect();
            Ok(Made {
                pipeline: Pipeline::Render {
                    pipeline,
                    vertex_stride: streams.vertex_stride,
                    colours,
                    depth: writes_depth || no_colour,
                },
                stated: stated.map(|((_, layouts), groups)| (groups, layouts)),
            })
        }
    }
}

/// What a draw binds at one entry of a group: stand-ins that hold nothing.
enum Held {
    Buffers(Vec<wgpu::Buffer>),
    Views(Vec<wgpu::TextureView>),
    Samplers(Vec<wgpu::Sampler>),
}

/// A one-texel texture of `format` with `usage`, seen as `dimension`.
fn stand_in_view(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    dimension: wgpu::TextureViewDimension,
    multisampled: bool,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureView {
    use wgpu::TextureViewDimension as V;
    let (texture_dimension, layers) = match dimension {
        V::D1 => (wgpu::TextureDimension::D1, 1),
        V::D2 => (wgpu::TextureDimension::D2, 1),
        V::D2Array => (wgpu::TextureDimension::D2, 2),
        V::Cube | V::CubeArray => (wgpu::TextureDimension::D2, 6),
        V::D3 => (wgpu::TextureDimension::D3, 1),
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: if multisampled { 4 } else { 1 },
        dimension: texture_dimension,
        format,
        usage,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(dimension),
        ..Default::default()
    })
}

/// What a draw binds at `entry`, a buffer of it `size` bytes long.
fn stand_in(
    device: &wgpu::Device,
    entry: &wgpu::BindGroupLayoutEntry,
    size: u64,
) -> Result<Held, String> {
    use wgpu::{BufferBindingType, SamplerBindingType, TextureSampleType, TextureUsages};
    let count = entry.count.map_or(1, |count| count.get() as usize);
    Ok(match entry.ty {
        wgpu::BindingType::Buffer { ty, .. } => {
            let usage = match ty {
                BufferBindingType::Uniform => wgpu::BufferUsages::UNIFORM,
                BufferBindingType::Storage { .. } => {
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
                }
            };
            let buffer = || {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: size.max(16).next_multiple_of(16),
                    usage,
                    mapped_at_creation: false,
                })
            };
            Held::Buffers((0..count).map(|_| buffer()).collect())
        }
        wgpu::BindingType::Sampler(kind) => {
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                compare: (kind == SamplerBindingType::Comparison)
                    .then_some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            });
            Held::Samplers(vec![sampler; count])
        }
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled,
        } => {
            let format = match sample_type {
                TextureSampleType::Float { .. } => wgpu::TextureFormat::Rgba8Unorm,
                TextureSampleType::Sint => wgpu::TextureFormat::Rgba8Sint,
                TextureSampleType::Uint => wgpu::TextureFormat::Rgba8Uint,
                TextureSampleType::Depth => wgpu::TextureFormat::Depth32Float,
            };
            // A texture of several samples can only be one that is drawn to.
            let usage = match multisampled {
                true => TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
                false => TextureUsages::TEXTURE_BINDING,
            };
            let view = stand_in_view(device, format, view_dimension, multisampled, usage);
            Held::Views(vec![view; count])
        }
        wgpu::BindingType::StorageTexture {
            access,
            format,
            view_dimension,
        } => {
            let usage = match access {
                wgpu::StorageTextureAccess::Atomic => {
                    TextureUsages::STORAGE_BINDING | TextureUsages::STORAGE_ATOMIC
                }
                _ => TextureUsages::STORAGE_BINDING,
            };
            let view = stand_in_view(device, format, view_dimension, false, usage);
            Held::Views(vec![view; count])
        }
        _ => return Err("the check has no stand-in for a resource the pipeline binds".into()),
    })
}

/// The bind groups of a draw, by their numbers, and the stand-ins each holds.
struct Bound {
    groups: Vec<(u32, wgpu::BindGroup)>,
    held: Vec<Vec<Held>>,
}

impl Made {
    /// A stand-in at every binding of the pipeline.
    fn bound(&self, device: &wgpu::Device) -> Result<Bound, String> {
        let (groups, layouts) = (self.stated.as_ref())
            .ok_or("the check states no layout for a resource the pipeline binds")?;
        let mut bound = Bound {
            groups: Vec::new(),
            held: Vec::new(),
        };
        for (group, layout) in layouts.iter().enumerate() {
            let (Some(layout), Some(entries)) = (layout, groups.get(&(group as u32))) else {
                continue;
            };
            let held: Vec<Held> = (entries.iter())
                .map(|(entry, size)| stand_in(device, entry, *size))
                .collect::<Result<_, _>>()?;
            // An array's entries are named through references that outlive the bind group's own.
            let views: Vec<Vec<&wgpu::TextureView>> = (held.iter())
                .map(|held| match held {
                    Held::Views(views) => views.iter().collect(),
                    _ => Vec::new(),
                })
                .collect();
            let samplers: Vec<Vec<&wgpu::Sampler>> = (held.iter())
                .map(|held| match held {
                    Held::Samplers(samplers) => samplers.iter().collect(),
                    _ => Vec::new(),
                })
                .collect();
            let buffers: Vec<Vec<wgpu::BufferBinding<'_>>> = (held.iter())
                .map(|held| match held {
                    Held::Buffers(buffers) => (buffers.iter())
                        .map(wgpu::Buffer::as_entire_buffer_binding)
                        .collect(),
                    _ => Vec::new(),
                })
                .collect();
            let resources: Vec<wgpu::BindGroupEntry<'_>> = (entries.iter().enumerate())
                .map(|(at, (entry, _))| {
                    let array = entry.count.is_some();
                    let resource = match &held[at] {
                        Held::Buffers(_) if array => {
                            wgpu::BindingResource::BufferArray(&buffers[at])
                        }
                        Held::Buffers(held) => held[0].as_entire_binding(),
                        Held::Views(_) if array => {
                            wgpu::BindingResource::TextureViewArray(&views[at])
                        }
                        Held::Views(held) => wgpu::BindingResource::TextureView(&held[0]),
                        Held::Samplers(_) if array => {
                            wgpu::BindingResource::SamplerArray(&samplers[at])
                        }
                        Held::Samplers(held) => wgpu::BindingResource::Sampler(&held[0]),
                    };
                    wgpu::BindGroupEntry {
                        binding: entry.binding,
                        resource,
                    }
                })
                .collect();
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout,
                entries: &resources,
            });
            bound.groups.push((group as u32, bind_group));
            bound.held.push(held);
        }
        Ok(bound)
    }

    /// A draw into targets of its own, or a dispatch, with every binding holding a stand-in: the
    /// device validates a pipeline against what it is bound and drawn with only here.
    // With `run` false no vertex is drawn and no group dispatched, so the device runs none of the
    // program: fed stand-ins, a program's loops need not end, and a draw that does not end has a
    // hardware driver reset the device under every process that draws on it. With `run` three
    // vertices are drawn as points, or one group dispatched: [`tried`] asks that of a CPU alone.
    fn draw(&self, gpu: &Gpu, bound: &Bound, run: bool) -> Result<(), String> {
        let device = &gpu.device;
        let bound = &bound.groups;
        let mut encoder = device.create_command_encoder(&Default::default());
        match &self.pipeline {
            Pipeline::Compute(pipeline) => {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(pipeline);
                for (group, bind_group) in bound {
                    pass.set_bind_group(*group, bind_group, &[]);
                }
                let groups = u32::from(run);
                pass.dispatch_workgroups(groups, groups, groups);
            }
            Pipeline::Render {
                pipeline,
                vertex_stride,
                colours,
                depth,
            } => {
                let target = |format: wgpu::TextureFormat| {
                    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
                    stand_in_view(device, format, wgpu::TextureViewDimension::D2, false, usage)
                };
                let colours: Vec<Option<wgpu::TextureView>> =
                    colours.iter().map(|format| format.map(target)).collect();
                let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = (colours
                    .iter())
                .map(|view| {
                    view.as_ref().map(|view| wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                })
                .collect();
                let depth = depth.then(|| target(wgpu::TextureFormat::Depth32Float));
                let vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: (*vertex_stride * u64::from(RUN_VERTICES)).max(4),
                    usage: wgpu::BufferUsages::VERTEX,
                    mapped_at_creation: false,
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &attachments,
                    depth_stencil_attachment: depth.as_ref().map(|view| {
                        wgpu::RenderPassDepthStencilAttachment {
                            view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                for (group, bind_group) in bound {
                    pass.set_bind_group(*group, bind_group, &[]);
                }
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.draw(0..if run { RUN_VERTICES } else { 0 }, 0..1);
            }
        }
        gpu.queue.submit([encoder.finish()]);
        (device.poll(wgpu::PollType::wait_indefinitely()))
            .map(|_| ())
            .map_err(|e| format!("the device did not finish the draw: {e}"))
    }
}

/// What the device said of `work`: its refusals are caught through its error scopes, and a panic
/// inside wgpu or naga's writers is a refusal too.
fn scoped(device: &wgpu::Device, work: impl FnOnce() -> Result<(), String>) -> Outcome {
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = catch_unwind(AssertUnwindSafe(work));
    let validation = pollster::block_on(validation.pop());
    let internal = pollster::block_on(internal.pop());
    let memory = pollster::block_on(memory.pop());
    match made {
        Err(panic) => {
            let said = (panic.downcast_ref::<String>().map(String::as_str))
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("with no message");
            Outcome::Rejected(format!("panicked: {}", one_line(said)))
        }
        Ok(Err(why)) => Outcome::Rejected(why),
        Ok(Ok(())) => match validation.or(internal).or(memory) {
            None => Outcome::Created,
            Some(wgpu::Error::Validation { description, .. }) => {
                Outcome::Rejected(one_line(&description))
            }
            Some(wgpu::Error::Internal { description, .. }) => {
                Outcome::Rejected(format!("internal: {}", one_line(&description)))
            }
            Some(error) => Outcome::Rejected(one_line(&error.to_string())),
        },
    }
}

/// What `gpu` makes of `stages`, and the milliseconds it took. With a `draw`, a pipeline it
/// creates is then drawn with ([`Made::draw`]), and a refusal of the draw is the unit's.
pub fn build(
    gpu: &Gpu,
    stages: &Stages,
    cache: Option<&wgpu::PipelineCache>,
    draw: Draw,
) -> (Outcome, f64) {
    let start = Instant::now();
    let device = &gpu.device;
    let mut made = None;
    let mut outcome = scoped(device, || {
        made = Some(create(device, stages, cache, draw)?);
        Ok(())
    });
    if let (Outcome::Created, true, Some(made)) = (&outcome, draw != Draw::No, &made) {
        let drawn = scoped(device, || {
            made.draw(gpu, &made.bound(device)?, draw == Draw::Run)
        });
        if let Outcome::Rejected(why) = drawn {
            outcome = Outcome::Rejected(format!("the draw: {why}"));
        }
    }
    drop(made);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    // The device frees what was dropped only when it is polled.
    let _ = device.poll(wgpu::PollType::Poll);
    (outcome, ms)
}

/// One thing the check builds a pipeline of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Unit {
    /// The container's program at this index, each of its blocks alone, with a partner that does nothing.
    Program(usize),
    /// The two programs, by index, a pass of an effect draws with.
    Pair { vertex: usize, pixel: usize },
}

impl Unit {
    /// `12`, `12.1` for a program's second block, `v12+p34`.
    pub fn name(&self, block: usize) -> String {
        match self {
            Self::Program(index) if block == 0 => index.to_string(),
            Self::Program(index) => format!("{index}.{block}"),
            Self::Pair { vertex, pixel } => format!("v{vertex}+p{pixel}"),
        }
    }
}

/// Every program of `file`, then each distinct vertex and pixel pair a pass of its effects names.
pub fn units(file: &Sgd5File) -> Vec<Unit> {
    let index: HashMap<usize, usize> = (file.programs.iter().enumerate())
        .map(|(i, program)| (program.at, i))
        .collect();
    let mut out: Vec<Unit> = (0..file.programs.len()).map(Unit::Program).collect();
    let mut seen = BTreeSet::new();
    for effect in &file.effects {
        for (_, vertex, pixel) in vertex_pixel_passes(file, effect) {
            let (Some(&vertex), Some(&pixel)) = (index.get(&vertex.at), index.get(&pixel.at))
            else {
                continue;
            };
            if seen.insert((vertex, pixel)) {
                out.push(Unit::Pair { vertex, pixel });
            }
        }
    }
    out
}

/// Every unit of `file` on a line: its number, its name as a report gives it, then a program's
/// kind and name, or each effect and technique a pass of which draws with the pair.
pub fn listing(file: &Sgd5File) -> String {
    let index: HashMap<usize, usize> = (file.programs.iter().enumerate())
        .map(|(i, program)| (program.at, i))
        .collect();
    let mut passes: HashMap<Unit, BTreeSet<String>> = HashMap::new();
    for effect in &file.effects {
        let mut names = Vec::new();
        for technique in &effect.techniques {
            names.extend((0..technique.passes).map(|_| technique.name.as_str()));
        }
        for (pass, vertex, pixel) in vertex_pixel_passes(file, effect) {
            let (Some(&vertex), Some(&pixel)) = (index.get(&vertex.at), index.get(&pixel.at))
            else {
                continue;
            };
            let technique = names.get(pass).copied().unwrap_or_default();
            (passes.entry(Unit::Pair { vertex, pixel }).or_default())
                .insert(format!("{}/{technique}", effect.name));
        }
    }
    let mut out = String::new();
    for (number, unit) in units(file).iter().enumerate() {
        let what = match unit {
            Unit::Program(at) => {
                format!("{}\t{}", file.programs[*at].kind, file.programs[*at].name)
            }
            Unit::Pair { .. } => {
                let of: Vec<&str> = (passes.get(unit).into_iter().flatten())
                    .map(String::as_str)
                    .collect();
                format!("pair\t{}", of.join(" "))
            }
        };
        out.push_str(&format!("{number}\t{}\t{what}\n", unit.name(0)));
    }
    out
}

/// The places in `units` of the pairs a pass of a technique named in `names` draws with.
pub fn technique_units(file: &Sgd5File, units: &[Unit], names: &[&str]) -> BTreeSet<usize> {
    let index: HashMap<usize, usize> = (file.programs.iter().enumerate())
        .map(|(i, program)| (program.at, i))
        .collect();
    let place: HashMap<Unit, usize> = units.iter().enumerate().map(|(i, u)| (*u, i)).collect();
    let mut out = BTreeSet::new();
    for effect in &file.effects {
        // A technique's passes follow those of the techniques before it.
        let mut wanted = BTreeSet::new();
        let mut first = 0;
        for technique in &effect.techniques {
            let passes = first..first + usize::from(technique.passes);
            first = passes.end;
            if names.contains(&technique.name.as_str()) {
                wanted.extend(passes);
            }
        }
        for (pass, vertex, pixel) in vertex_pixel_passes(file, effect) {
            let (Some(&vertex), Some(&pixel)) = (index.get(&vertex.at), index.get(&pixel.at))
            else {
                continue;
            };
            if wanted.contains(&pass) {
                out.extend(place.get(&Unit::Pair { vertex, pixel }));
            }
        }
    }
    out
}

/// Names the stack, in mebibytes, a worker's steps run on.
pub const STACK_ENV: &str = "OR_RDNA_DEVICES_STACK_MIB";
/// The stack a worker's steps run on unless [`STACK_ENV`] says otherwise: naga's HLSL and GLSL
/// writers recurse once per nested expression, and a large program overflows a thread's usual stack.
const DEFAULT_STACK_MIB: usize = 256;

/// The mebibytes of stack a worker's steps run on.
pub fn stack_mib() -> usize {
    (std::env::var(STACK_ENV).ok())
        .and_then(|mib| mib.parse().ok())
        .unwrap_or(DEFAULT_STACK_MIB)
}

/// `work` on a thread of [`stack_mib`] of stack.
pub fn on_worker_stack<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        (std::thread::Builder::new().stack_size(stack_mib() << 20))
            .spawn_scoped(scope, work)
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|_| "the worker panicked".to_owned())
    })
}

/// A unit translated and its stages joined: its modules, and what they ask of a device.
pub struct Ready {
    pub stages: Stages,
    /// The unit for a device that clips by no distance, or that links fewer locations than
    /// `stages` cross with: the distances a discard in the pixel stage, and no parameter that
    /// nothing reads.
    pub bare: Option<Stages>,
    /// The locations the vertex stage of `stages` gives, and whether it gives clip distances.
    pub locations: u32,
    pub clips: bool,
    /// Whether the modules hold subgroup operations.
    pub subgroups: bool,
    /// The device features the textures the modules declare cannot do without.
    pub needs: wgpu::Features,
    /// What the check did to the unit that a reader of its outcome should know.
    pub notes: Vec<String>,
}

/// The device features `module` cannot do without, by the textures it declares: one updated in a
/// step, and one of 64-bit texels, which the module then computes with 64-bit integers. A device
/// without them refuses the module in words that name no feature.
fn needs(module: &naga::Module) -> wgpu::Features {
    let mut needs = wgpu::Features::empty();
    for (_, ty) in module.types.iter() {
        let naga::TypeInner::Image {
            class: naga::ImageClass::Storage { format, access },
            ..
        } = ty.inner
        else {
            continue;
        };
        if access.contains(naga::StorageAccess::ATOMIC) {
            needs |= wgpu::Features::TEXTURE_ATOMIC;
        }
        if format == naga::StorageFormat::R64Uint {
            needs |= wgpu::Features::TEXTURE_INT64_ATOMIC | wgpu::Features::SHADER_INT64;
        }
    }
    needs
}

/// The capabilities naga's validator has to be given before it takes `module`, by their names:
/// what the module asks of any device, whichever one builds it.
pub fn asked(module: &naga::Module) -> Vec<String> {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    // What the textures need is given first: without 64-bit integers the validator stops at a
    // cast, in words that name no capability, and would never reach the rest.
    let mut names: Vec<String> = (needs(module).iter_names())
        .map(|(name, _)| name.to_owned())
        .collect();
    let mut given = (names
        .iter()
        .filter_map(|name| Capabilities::from_name(name)))
    .fold(Capabilities::empty(), |all, one| all | one);
    // The validator names one at a time.
    while let Err(refusal) = Validator::new(ValidationFlags::all(), given).validate(module) {
        let Some(named) = missing_feature(&format!("{:?}", refusal.into_inner())) else {
            break;
        };
        let more = (named.split(" | ").filter_map(Capabilities::from_name))
            .fold(Capabilities::empty(), |all, one| all | one);
        if given.contains(more) {
            break;
        }
        given |= more;
        names.extend(named.split(" | ").map(str::to_owned));
    }
    names
}

impl Stages {
    /// What the modules ask of any device, as notes of a report.
    fn asked(&self) -> Vec<String> {
        let mut names = match self {
            Self::Compute(module) | Self::Render(module, None) => asked(module),
            Self::Render(vertex, Some(pixel)) => [asked(vertex), asked(pixel)].concat(),
        };
        names.sort();
        names.dedup();
        names
            .into_iter()
            .map(|name| format!("needs {name}"))
            .collect()
    }

    fn needs(&self) -> wgpu::Features {
        match self {
            Self::Compute(module) | Self::Render(module, None) => needs(module),
            Self::Render(vertex, Some(pixel)) => needs(vertex) | needs(pixel),
        }
    }
}

/// The locations a vertex stage gives, and whether it gives clip distances.
fn given(vertex: &naga::Module) -> (u32, bool) {
    let result = (vertex.entry_points.first()).and_then(|entry| entry.function.result.as_ref());
    let members = match result.map(|result| &vertex.types[result.ty].inner) {
        Some(naga::TypeInner::Struct { members, .. }) => members.as_slice(),
        _ => &[],
    };
    let is = |member: &naga::StructMember, clip: bool| match member.binding {
        Some(naga::Binding::Location { .. }) => !clip,
        Some(naga::Binding::BuiltIn(naga::BuiltIn::ClipDistance)) => clip,
        _ => false,
    };
    let locations = members.iter().filter(|member| is(member, false)).count();
    (
        locations as u32,
        members.iter().any(|member| is(member, true)),
    )
}

/// The groups past a stage's own that its binding arrays go to, as the renderer places them: wgpu
/// takes no group that holds both a binding array and a constant buffer.
const ARRAY_GROUPS: u32 = 2;

fn arrays_apart(stages: &mut Stages) {
    let apart = |module: &mut naga::Module| {
        let arrays: Vec<_> = (module.global_variables.iter())
            .filter(|(_, global)| {
                matches!(
                    module.types[global.ty].inner,
                    naga::TypeInner::BindingArray { .. }
                )
            })
            .map(|(handle, _)| handle)
            .collect();
        for handle in arrays {
            if let Some(binding) = &mut module.global_variables[handle].binding {
                binding.group += ARRAY_GROUPS;
            }
        }
    };
    match stages {
        Stages::Compute(module) | Stages::Render(module, None) => apart(module),
        Stages::Render(vertex, Some(pixel)) => {
            apart(vertex);
            apart(pixel);
        }
    }
}

/// The locations every device links: a vertex stage that gives more is also built as a pass that reads none of them.
const LOCATIONS: u32 = 16;

/// A pixel stage that reads and writes nothing, as a pass that only fills the depth runs.
fn no_pixel() -> Result<naga::Module, String> {
    naga::front::wgsl::parse_str("@fragment fn main() {}").map_err(|e| one_line(&e.to_string()))
}

fn joined(
    vertex: &naga::Module,
    pixel: &naga::Module,
    options: &link::Options,
) -> Result<(Stages, Linked), String> {
    let (mut vertex, mut pixel) = (vertex.clone(), pixel.clone());
    let linked = link::link(&mut vertex, &mut pixel, options)
        .map_err(|why| format!("the stages do not join: {why}"))?;
    for module in [&vertex, &pixel] {
        rdna::check(module)
            .map_err(|why| format!("naga refuses a joined module: {}", one_line(&why)))?;
    }
    Ok((Stages::Render(vertex, Some(pixel)), linked))
}

fn refusal(error: &RdnaError) -> String {
    match error {
        RdnaError::NotTranslated(why) => format!("not translated: {}", why.name()),
        RdnaError::Cfg(_) => "not translated: graph".into(),
        RdnaError::Container(_) => "the program's container does not read".into(),
    }
}

fn checked(module: &naga::Module) -> Result<(), String> {
    rdna::check(module).map_err(|why| format!("naga refuses the module: {}", one_line(&why)))
}

/// `module` alone as a pipeline: a pixel module gets a vertex stage that feeds every input it takes.
pub fn alone(mut module: naga::Module, subgroups: bool) -> Result<Ready, String> {
    let stage = module.entry_points.first().map(|entry| entry.stage);
    let (mut bare, mut locations, mut clips) = (None, 0, false);
    let stages = match stage {
        Some(ShaderStage::Compute) => Stages::Compute(module),
        Some(ShaderStage::Vertex) => {
            (locations, clips) = given(&module);
            // A stage the translator did not write joins nothing, and is built as it stands.
            if clips || locations > LOCATIONS {
                let joined = joined(&module, &no_pixel()?, &link::Options::default());
                bare = joined.ok().map(|(stages, _)| stages);
            }
            Stages::Render(module, None)
        }
        Some(ShaderStage::Fragment) => {
            // Its inputs packed as a pass packs them: as lowered, a parameter's high halves are past what a device links.
            // One the translator did not write is left as it is.
            if let Ok(packed) = link::pack(&mut module) {
                locations = packed.locations;
                rdna::check(&module)
                    .map_err(|why| format!("naga refuses a packed module: {}", one_line(&why)))?;
            }
            let partner = matching_vertex_shader(&module, &module.entry_points[0])
                .and_then(|source| {
                    naga::front::wgsl::parse_str(&source).map_err(|e| one_line(&e.to_string()))
                })
                .map_err(|why| format!("the check writes no vertex stage for it: {why}"))?;
            Stages::Render(partner, Some(module))
        }
        _ => return Err("the module has no entry point of a stage the check builds".into()),
    };
    let mut ready = Ready {
        needs: stages.needs(),
        stages,
        bare,
        locations,
        clips,
        subgroups,
        notes: Vec::new(),
    };
    ready.settle();
    Ok(ready)
}

/// The locations wgpu's Metal backend links between stages on iOS: 60 components, four a location.
const IOS_LOCATIONS: u32 = 15;

/// The note of a unit whose stages cross with more than [`IOS_LOCATIONS`].
pub const PAST_IOS: &str =
    "crosses its stages with more than 15 locations, the most wgpu's Metal backend links on iOS";

/// The start of the note of a unit the renderer refuses to build on an AMD adapter under
/// Direct3D 12 ([`instance::refused_by_driver`]), whatever adapter the check runs on.
pub const AMD_REFUSED: &str = "on an AMD adapter under Direct3D 12 the renderer refuses it";

/// An adapter's description that holds nothing but its vendor, its backend and its kind.
fn described_as(vendor: u32, backend: wgpu::Backend, kind: wgpu::DeviceType) -> wgpu::AdapterInfo {
    wgpu::AdapterInfo {
        name: String::new(),
        vendor,
        device: 0,
        device_type: kind,
        device_pci_bus_id: String::new(),
        driver: String::new(),
        driver_info: String::new(),
        backend,
        subgroup_min_size: 4,
        subgroup_max_size: 4,
        transient_saves_memory: false,
    }
}

impl Stages {
    /// Why the renderer builds no pass of these stages on the adapter `info`, where it does not.
    fn refused_on(&self, info: &wgpu::AdapterInfo) -> Option<String> {
        match self {
            Self::Render(vertex, _) => instance::refused_by_driver(info, vertex),
            Self::Compute(_) => None,
        }
    }
}

impl Ready {
    fn settle(&mut self) {
        arrays_apart(&mut self.stages);
        self.bare.as_mut().map(arrays_apart);
        // The pass with no optional part, where there is one, is what a device has to take.
        let least = self.bare.as_ref().unwrap_or(&self.stages);
        self.notes.extend(least.asked());
        const AMD: u32 = 0x1002;
        let amd = described_as(AMD, wgpu::Backend::Dx12, wgpu::DeviceType::IntegratedGpu);
        let refused = self.stages.refused_on(&amd);
        (self.notes).extend(refused.map(|why| format!("{AMD_REFUSED}: {why}")));
        if self.locations > IOS_LOCATIONS {
            self.notes.push(PAST_IOS.to_owned());
        }
    }
}

/// What the parent lists once for its workers: the units, and where each program's DXIL container
/// lies in the file of them all, `None` where its stream does not inflate.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Plan {
    units: Vec<Unit>,
    /// The units a worker checks of its range; all of them where `None`.
    #[serde(default)]
    only: Option<BTreeSet<usize>>,
    programs: Vec<Option<(u64, usize)>>,
}

/// A container's programs inflated into one file in a folder, with the units beside it: reading
/// the container itself takes a worker seconds and a gigabyte, and a worker is started often.
pub struct Programs {
    plan: Plan,
    blob: Mutex<std::fs::File>,
    /// Per adapter, in the order they open, the units not to build on it ([`Options::avoid`]).
    avoid: Vec<BTreeSet<String>>,
}

/// What a unit is skipped for on an adapter it ended the process on before.
pub const AVOIDED: &str = "ended the process on this adapter and driver in an earlier run";

impl Programs {
    /// Writes `file`'s programs and `units` into `dir`.
    pub fn write(
        dir: &Path,
        file: &Sgd5File,
        body: &[u8],
        units: &[Unit],
        only: Option<&BTreeSet<usize>>,
    ) -> Result<(), String> {
        let said = |e: std::io::Error| format!("{}: {e}", dir.display());
        let mut blob =
            std::io::BufWriter::new(std::fs::File::create(dir.join("programs.bin")).map_err(said)?);
        let mut plan = Plan {
            units: units.to_vec(),
            only: only.cloned(),
            programs: Vec::with_capacity(file.programs.len()),
        };
        let mut at = 0u64;
        for program in &file.programs {
            let container =
                (body.get(program.stream.clone())).and_then(|stream| program.inflate(stream).ok());
            plan.programs.push(match container {
                Some(container) => {
                    blob.write_all(&container).map_err(said)?;
                    at += container.len() as u64;
                    Some((at - container.len() as u64, container.len()))
                }
                None => None,
            });
        }
        blob.flush().map_err(said)?;
        let text = serde_json::to_string(&plan).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("plan.json"), text).map_err(said)
    }

    /// Writes into `dir` the units each adapter is not to build.
    fn write_avoided(dir: &Path, avoid: &[BTreeSet<String>]) -> Result<(), String> {
        let text = serde_json::to_string(avoid).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("avoid.json"), text).map_err(|e| format!("{}: {e}", dir.display()))
    }

    pub fn open(dir: &Path) -> Result<Self, String> {
        let said = |e: std::io::Error| format!("{}: {e}", dir.display());
        let text = std::fs::read_to_string(dir.join("plan.json")).map_err(said)?;
        let avoid = (std::fs::read_to_string(dir.join("avoid.json")).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Ok(Self {
            plan: serde_json::from_str(&text).map_err(|e| e.to_string())?,
            blob: Mutex::new(std::fs::File::open(dir.join("programs.bin")).map_err(said)?),
            avoid,
        })
    }

    pub fn units(&self) -> &[Unit] {
        &self.plan.units
    }

    /// The DXIL container of the program at `index`.
    fn container(&self, index: usize) -> Result<Vec<u8>, String> {
        let (at, len) = (self.plan.programs.get(index))
            .ok_or("no such program")?
            .ok_or("the program's stream does not inflate")?;
        let mut container = vec![0; len];
        let mut blob = self.blob.lock().map_err(|e| e.to_string())?;
        (blob.seek(SeekFrom::Start(at)))
            .and_then(|_| blob.read_exact(&mut container))
            .map_err(|e| e.to_string())?;
        Ok(container)
    }
}

/// How many blocks `unit` has, each translated and built apart.
fn blocks(programs: &Programs, unit: Unit) -> Result<usize, String> {
    match unit {
        Unit::Pair { .. } => Ok(1),
        Unit::Program(index) => rdna::unwrap(&programs.container(index)?)
            .map(|blocks| blocks.len())
            .map_err(|_| "the program's container does not read".to_owned()),
    }
}

/// Block `block` of `unit` through the translator and naga's validator.
pub fn translated(programs: &Programs, unit: Unit, block: usize) -> Result<Ready, String> {
    let inflated = |index| programs.container(index);
    match unit {
        Unit::Program(index) => {
            let programs = rdna::unwrap(&inflated(index)?)
                .map_err(|_| "the program's container does not read".to_owned())?;
            let program = programs.get(block).ok_or("no such block")?;
            let lowered = rdna::translate(program).map_err(|e| refusal(&e))?;
            checked(&lowered.module)?;
            alone(lowered.module, lowered.subgroups)
        }
        Unit::Pair { vertex, pixel } => {
            let stage = |index, stage: &str| {
                rdna_bind::native(&inflated(index)?)
                    .map_err(|e| format!("pair, {stage}: {}", e.name()))
            };
            let (vertex, pixel) = (stage(vertex, "vertex")?, stage(pixel, "pixel")?);
            let fragment = rdna_bind::lower(&pixel, &BTreeSet::new())
                .map_err(|e| format!("pair, pixel: {}", e.name()))?;
            // The pixel stage reads parameter `n` at the `n`th register of the signature that is no
            // system value, as the binder pairs them; a parameter past the signature at its own number.
            let registers: Vec<link::Register> = (vertex.outputs.iter())
                .map(|element| link::Register {
                    number: element.register,
                    system_value: element.system_value,
                    halves: element.component_type == FLOAT16,
                })
                .collect();
            // Lowered once more, for its count, only where the signature reads two ways.
            let count = || {
                (rdna_bind::lower(&vertex, &BTreeSet::new()).ok())
                    .map_or(0, |lowered| link::written(&lowered.module))
            };
            let slots = link::parameter_slots(&registers, count);
            let parameter = |slot: &u8| match slots.iter().position(|s| *s == u32::from(*slot)) {
                Some(at) => u8::try_from(at).ok(),
                None => (usize::from(*slot) >= slots.len()).then_some(*slot),
            };
            // The vertex stage exports as halves what the pixel stage reads as halves.
            let halves: BTreeSet<u8> = fragment.half_slots.iter().filter_map(parameter).collect();
            let geometry = rdna_bind::lower(&vertex, &halves)
                .map_err(|e| format!("pair, vertex: {}", e.name()))?;
            checked(&geometry.module)?;
            checked(&fragment.module)?;
            // Each stage numbers its bindings from zero in group 0: the pixel stage's go to group 1, where the binder puts them.
            let mut pixel = fragment.module;
            for (_, global) in pixel.global_variables.iter_mut() {
                if let Some(binding) = &mut global.binding {
                    binding.group = 1;
                }
            }
            let (_, clips) = given(&geometry.module);
            let options = |clip_distances| link::Options {
                slots: slots.clone(),
                clip_distances,
            };
            let (stages, linked) = joined(&geometry.module, &pixel, &options(true))?;
            let bare = match clips {
                true => Some(joined(&geometry.module, &pixel, &options(false))?.0),
                false => None,
            };
            let unfed = linked.carried.iter().any(|carried| !carried.fed);
            let mut ready = Ready {
                subgroups: geometry.subgroups || fragment.subgroups,
                needs: stages.needs(),
                stages,
                bare,
                locations: linked.locations,
                clips,
                notes: (unfed.then(|| UNFED.to_owned()).into_iter()).collect(),
            };
            ready.settle();
            Ok(ready)
        }
    }
}

/// The component type a signature gives a register of 16-bit floats.
const FLOAT16: u32 = 6;

/// The note of a pair whose pixel stage reads a parameter its vertex stage does not give.
pub const UNFED: &str =
    "a pair whose pixel stage reads a parameter its vertex stage does not write: read as nought";

/// What a unit is skipped for on an adapter that offers `TEXTURE_INT64_ATOMIC` and whose textures
/// of 64-bit texels cannot be updated in a step.
pub const NO_64_BIT_UPDATE: &str =
    "TEXTURE_INT64_ATOMIC (offered, but no R64Uint texture of the adapter is updated in a step)";

/// The versions of Metal's shading language the text is written for, each with the first macOS
/// and iOS that take it: what wgpu asks naga for on a Metal device of that system. The second
/// is the oldest on which wgpu's Metal backend updates a texture in a step.
pub const METAL_VERSIONS: [((u8, u8), &str); 2] =
    [((3, 0), "macOS 13, iOS 16"), ((3, 1), "macOS 14, iOS 17")];

/// What Metal binds a stage, as wgpu's Metal backend states it of a Mac: buffers, textures and
/// samplers.
const METAL_STAGE: [(&str, u32); 3] = [("buffers", 31), ("textures", 128), ("samplers", 16)];

/// Where each resource of a stage of `module` goes in Metal's tables, numbered as wgpu's Metal
/// backend numbers a pipeline layout: in the order of the groups and of the bindings in each, an
/// array as one buffer of arguments, then a buffer of sizes. `None` where the renderer's layouts
/// do not take the module, which is then written with made-up numbers.
///
/// # Errors
/// Where the stage binds more than [`METAL_STAGE`], which the backend refuses the layout for.
fn metal_resources(
    module: &naga::Module,
    stage: wgpu::ShaderStages,
) -> Result<Option<naga::back::msl::EntryPointResources>, String> {
    use naga::back::msl::{BindSamplerTarget, BindTarget};
    let Some((groups, _)) = groups(&[(module, stage)]) else {
        return Ok(None);
    };
    let mut resources = naga::back::msl::EntryPointResources::default();
    let (mut bound, mut sized) = ([0u32; 3], stage == wgpu::ShaderStages::VERTEX);
    let mut next = |kind: usize| {
        bound[kind] += 1;
        (bound[kind] - 1) as u8
    };
    for (group, entries) in &groups {
        let mut entries: Vec<_> = entries.iter().map(|(entry, _)| entry).collect();
        entries.sort_by_key(|entry| entry.binding);
        for entry in entries {
            let mut target = BindTarget::default();
            match entry.ty {
                _ if entry.count.is_some() => target.buffer = Some(next(0)),
                wgpu::BindingType::Buffer { ty, .. } => {
                    target.buffer = Some(next(0));
                    if let wgpu::BufferBindingType::Storage { read_only } = ty {
                        target.mutable = !read_only;
                        sized = true;
                    }
                }
                wgpu::BindingType::Sampler(_) => {
                    target.sampler = Some(BindSamplerTarget::Resource(next(2)));
                }
                wgpu::BindingType::StorageTexture { access, .. } => {
                    target.texture = Some(next(1));
                    target.mutable = access != wgpu::StorageTextureAccess::ReadOnly;
                }
                _ => target.texture = Some(next(1)),
            }
            let at = naga::ResourceBinding {
                group: *group,
                binding: entry.binding,
            };
            resources.resources.insert(at, target);
        }
    }
    if sized {
        resources.sizes_buffer = Some(next(0));
    }
    // The vertex buffer a render stage is drawn with takes one of the same table.
    bound[0] += u32::from(stage == wgpu::ShaderStages::VERTEX);
    match (METAL_STAGE.iter().zip(bound)).find(|((_, most), bound)| bound > most) {
        Some(((what, most), bound)) => {
            let stage = match stage {
                wgpu::ShaderStages::VERTEX => "vertex",
                wgpu::ShaderStages::FRAGMENT => "pixel",
                _ => "compute",
            };
            Err(format!(
                "Metal binds {most} {what} a stage, and the {stage} stage binds {bound}"
            ))
        }
        None => Ok(Some(resources)),
    }
}

/// `module` as Metal's shading language of `version`, written with the checks a Metal device's
/// writer is given and with its resources at `resources`.
fn metal_source(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    version: (u8, u8),
    resources: Option<naga::back::msl::EntryPointResources>,
) -> Result<String, String> {
    use naga::back::msl;
    use naga::proc::{BoundsCheckPolicies, BoundsCheckPolicy};
    let entry = module.entry_points.first().map(|entry| entry.name.clone());
    let options = msl::Options {
        lang_version: version,
        fake_missing_bindings: resources.is_none(),
        per_entry_point_map: (entry.zip(resources).into_iter()).collect(),
        bounds_check_policies: BoundsCheckPolicies {
            index: BoundsCheckPolicy::Restrict,
            buffer: BoundsCheckPolicy::Restrict,
            image_load: BoundsCheckPolicy::Restrict,
            binding_array: BoundsCheckPolicy::Unchecked,
        },
        zero_initialize_workgroup_memory: true,
        force_loop_bounding: true,
        ..Default::default()
    };
    let (text, written) =
        msl::write_string(module, info, &options, &msl::PipelineOptions::default())
            .map_err(|e| one_line(&e.to_string()))?;
    // The writer gives the text and says apart which entry points it could not write.
    match written.entry_point_names.into_iter().find_map(Result::err) {
        Some(e) => Err(format!(
            "an entry point is not written: {}",
            one_line(&e.to_string())
        )),
        None => Ok(text),
    }
}

/// Whether naga writes `stages` as Metal's shading language, validated for what its writer
/// supports and counted against what Metal binds a stage: all of Metal that a machine with no
/// Metal device can check. A unit that updates a texture in a step is skipped by the version of
/// [`METAL_VERSIONS`] that does; naga writes the same text for both. With `keep`, a folder and the unit's name, each stage's text is saved
/// there under the version it was written for, for a Mac's own compiler.
// ponytail: the text is not compiled here; only Metal's compiler says whether it builds.
fn metal_text(stages: &Stages, keep: Option<(&Path, &str)>) -> Outcome {
    use naga::back::msl;
    use naga::valid::{ValidationFlags, Validator};
    let write = |module: &naga::Module, stage: wgpu::ShaderStages, tag: &str| {
        let info = Validator::new(ValidationFlags::all(), msl::supported_capabilities())
            .validate(module)
            .map_err(|refusal| {
                let said = format!("{:?}", refusal.into_inner());
                match missing_feature(&said) {
                    Some(capability) => Outcome::Skipped(capability),
                    None => Outcome::Rejected(one_line(&said)),
                }
            })?;
        let resources = metal_resources(module, stage).map_err(Outcome::Rejected)?;
        let at = usize::from(needs(module).contains(wgpu::Features::TEXTURE_ATOMIC));
        let (version, _) = METAL_VERSIONS[at];
        let text = metal_source(module, &info, version, resources).map_err(Outcome::Rejected)?;
        if let Some((dir, name)) = keep {
            let dir = dir.join(format!("{}.{}", version.0, version.1));
            let file = dir.join(format!("{name}.{tag}.metal"));
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(&file, text))
                .map_err(|e| Outcome::Rejected(format!("{}: {e}", file.display())))?;
        }
        Ok(at)
    };
    let written = catch_unwind(AssertUnwindSafe(|| match stages {
        Stages::Compute(module) => write(module, wgpu::ShaderStages::COMPUTE, "compute"),
        Stages::Render(module, None) => write(module, wgpu::ShaderStages::VERTEX, "vertex"),
        Stages::Render(vertex, Some(pixel)) => {
            let vertex = write(vertex, wgpu::ShaderStages::VERTEX, "vertex")?;
            Ok(vertex.max(write(pixel, wgpu::ShaderStages::FRAGMENT, "pixel")?))
        }
    }));
    match written {
        Ok(Ok(0)) => Outcome::Created,
        Ok(Ok(at)) => {
            let ((major, minor), systems) = METAL_VERSIONS[at];
            Outcome::Skipped(format!("Metal's language {major}.{minor} ({systems})"))
        }
        Ok(Err(outcome)) => outcome,
        Err(_) => Outcome::Rejected("naga's MSL writer panicked".into()),
    }
}

/// The adapter a run of [`Ways::msl`] is counted under.
pub fn metal_text_adapter() -> Adapter {
    Adapter {
        name: "naga's MSL writer, no device".into(),
        backend: "Metal".into(),
        shader_compiler: "text".into(),
        ..Default::default()
    }
}

/// `ready` on `gpu`, or the feature it needs that `gpu` lacks.
fn tried(
    gpu: &Gpu,
    ready: &Ready,
    cache: Option<&wgpu::PipelineCache>,
    draw: Draw,
) -> (Outcome, f64) {
    if draw == Draw::Run && gpu.info.device_type != wgpu::DeviceType::Cpu {
        return (Outcome::Skipped(RUN_ON_SOFTWARE.into()), 0.0);
    }
    if ready.subgroups && !gpu.features.contains(wgpu::Features::SUBGROUP) {
        return (Outcome::Skipped("SUBGROUP".into()), 0.0);
    }
    let missing = ready.needs - gpu.features;
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter_names().map(|(name, _)| name).collect();
        return (Outcome::Skipped(names.join(" | ")), 0.0);
    }
    if (ready.needs).contains(wgpu::Features::TEXTURE_INT64_ATOMIC) && !gpu.updates_64_bit_textures
    {
        return (Outcome::Skipped(NO_64_BIT_UPDATE.into()), 0.0);
    }
    let lacks = ready.clips && !gpu.features.contains(wgpu::Features::CLIP_DISTANCES)
        || ready.locations > gpu.limits.max_inter_stage_shader_variables;
    let stages = match &ready.bare {
        Some(bare) if lacks => bare,
        _ => &ready.stages,
    };
    // What the renderer refuses for the adapter's driver is not put to the driver here either.
    match stages.refused_on(&gpu.info) {
        Some(why) => (Outcome::Skipped(why), 0.0),
        None => build(gpu, stages, cache, draw),
    }
}

/// Where a worker starts in its first unit: the blocks before `block` are passed over, and in `block` the adapters before `adapter`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Resume {
    pub block: usize,
    pub adapter: usize,
}

/// The units of `range` of `programs`, each on every adapter, written to `out` a line a step: a
/// [`Line::Begin`] before anything that can end the process, so the parent knows what did.
pub fn worker(
    programs: &Programs,
    range: Range<usize>,
    resume: Resume,
    only: Option<&str>,
    ways: &Ways,
    out: &mut dyn Write,
) -> Result<(), String> {
    let mut say = |line: Line| -> Result<(), String> {
        let text = serde_json::to_string(&line).map_err(|e| e.to_string())?;
        writeln!(out, "{MARK}{text}")
            .and_then(|()| out.flush())
            .map_err(|e| e.to_string())
    };
    let gpus = open_all(only);
    for unit in range.clone() {
        let Some(&what) = programs.units().get(unit) else {
            break;
        };
        if (programs.plan.only.as_ref()).is_some_and(|only| !only.contains(&unit)) {
            continue;
        }
        let first = unit == range.start;
        let count = match blocks(programs, what) {
            Ok(count) => count,
            Err(why) => {
                if !first || resume == Resume::default() {
                    say(Line::Front {
                        unit,
                        block: 0,
                        why,
                    })?;
                }
                continue;
            }
        };
        for block in (0..count).skip(if first { resume.block } else { 0 }) {
            say(Line::Begin {
                unit,
                block,
                adapter: None,
            })?;
            let ready = match translated(programs, what, block) {
                Ok(ready) => ready,
                Err(why) => {
                    say(Line::Front { unit, block, why })?;
                    continue;
                }
            };
            for what in &ready.notes {
                let what = what.clone();
                say(Line::Note { unit, block, what })?;
            }
            let passed = match first && block == resume.block {
                true => resume.adapter,
                false => 0,
            };
            for (adapter, gpu) in gpus.iter().enumerate().skip(passed) {
                say(Line::Begin {
                    unit,
                    block,
                    adapter: Some(adapter),
                })?;
                let avoided = (programs.avoid.get(adapter))
                    .is_some_and(|avoid| avoid.contains(&what.name(block)));
                let (outcome, ms) = match avoided {
                    true => (Outcome::Skipped(AVOIDED.into()), 0.0),
                    false => tried(gpu, &ready, None, ways.draw),
                };
                say(Line::Device {
                    unit,
                    block,
                    adapter,
                    outcome,
                    ms,
                })?;
            }
            // The text is one more adapter, after the devices.
            if ways.msl && passed <= gpus.len() {
                let adapter = gpus.len();
                say(Line::Begin {
                    unit,
                    block,
                    adapter: Some(adapter),
                })?;
                let start = Instant::now();
                let name = what.name(block);
                let keep = (ways.msl_dir.as_deref()).map(|dir| (dir, name.as_str()));
                let outcome = metal_text(&ready.stages, keep);
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                say(Line::Device {
                    unit,
                    block,
                    adapter,
                    outcome,
                    ms,
                })?;
            }
        }
    }
    say(Line::Done)
}

/// The `(unit, block)` steps of `list` built on adapter `adapter` with a pipeline cache: an empty
/// one saved to `file` afterwards, or with `load` the one `file` holds. Gives the pipelines built,
/// the milliseconds the device took over them, and the cache's bytes.
pub fn cache_worker(
    programs: &Programs,
    adapter: usize,
    only: Option<&str>,
    list: &[(usize, usize)],
    file: &Path,
    load: bool,
) -> Result<(usize, f64, usize), String> {
    let gpus = open_all(only);
    let gpu = gpus.get(adapter).ok_or("no such adapter")?;
    let saved = match load {
        true => Some(std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?),
        false => None,
    };
    // SAFETY: the data is what `get_data` gave for this adapter, saved under wgpu's key for it; a
    // cache the driver no longer takes falls back to an empty one.
    let cache = unsafe {
        gpu.device
            .create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: None,
                data: saved.as_deref(),
                fallback: true,
            })
    };
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let (mut built, mut ms) = (0usize, 0f64);
    // Translated a batch at a time on every core, so the modules of a container are never all held at once.
    for batch in list.chunks(threads * 16) {
        let per = batch.len().div_ceil(threads);
        let ready: Vec<Option<Ready>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (batch.chunks(per))
                .filter_map(|part| {
                    let translate = move || {
                        (part.iter())
                            .map(|&(unit, block)| {
                                translated(programs, *programs.units().get(unit)?, block).ok()
                            })
                            .collect::<Vec<_>>()
                    };
                    (std::thread::Builder::new().stack_size(stack_mib() << 20))
                        .spawn_scoped(scope, translate)
                        .ok()
                })
                .collect();
            (handles.into_iter())
                .flat_map(|handle| handle.join().unwrap_or_default())
                .collect()
        });
        for ready in ready.iter().flatten() {
            if let (Outcome::Created, took) = tried(gpu, ready, Some(&cache), Draw::No) {
                built += 1;
                ms += took;
            }
        }
    }
    let bytes = match load {
        true => saved.map_or(0, |data| data.len()),
        false => {
            let data = cache.get_data().ok_or("the device gave no cache data")?;
            std::fs::write(file, &data).map_err(|e| format!("{}: {e}", file.display()))?;
            data.len()
        }
    };
    Ok((built, ms, bytes))
}

/// What a check does with a unit beyond creating its pipeline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ways {
    /// What is done with each pipeline a device creates.
    pub draw: Draw,
    /// Write each unit as Metal's shading language too, counted as an adapter of its own.
    pub msl: bool,
    /// Where each unit's Metal text is kept, a folder a language version.
    pub msl_dir: Option<PathBuf>,
}

/// How a check runs.
#[derive(Debug, Clone)]
pub struct Options {
    pub ways: Ways,
    /// The worker processes at once.
    pub jobs: usize,
    /// The units a worker is given.
    pub chunk: usize,
    /// How long a worker may say nothing before it is ended and the step it was on counted as timed out.
    pub timeout: Duration,
    /// The units checked, of [`units`]; all of them where `None`.
    pub units: Option<Range<usize>>,
    pub adapter: Option<String>,
    /// The techniques whose passes' pairs are checked, and nothing else; every unit where empty.
    pub techniques: Vec<String>,
    /// The units checked and nothing else, by their numbers; every unit where `None`.
    pub picked: Option<BTreeSet<usize>>,
    /// Where each adapter's pipeline cache is saved, where its backend has one.
    pub cache_dir: Option<PathBuf>,
    /// The stack a worker's steps run on, in mebibytes; [`stack_mib`]'s where `None`.
    pub stack_mib: Option<usize>,
    /// An earlier report: a unit that ended the process on an adapter of it is not built again
    /// on that adapter and driver, and is counted as skipped for [`AVOIDED`].
    pub avoid: Option<Report>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            ways: Ways::default(),
            jobs: std::thread::available_parallelism().map_or(2, |n| (usize::from(n) / 2).max(1)),
            chunk: 128,
            timeout: Duration::from_secs(120),
            units: None,
            adapter: None,
            techniques: Vec::new(),
            picked: None,
            cache_dir: None,
            stack_mib: None,
            avoid: None,
        }
    }
}

/// A step of a worker: the unit, the block, and the adapter, `None` while the translator runs.
type Step = (usize, usize, Option<usize>);

/// One worker over `range` from `resume`; its lines go to `results`. Gives where the next worker
/// starts, or `None` once the range is done. `again` is the step that last ended a worker.
fn one_worker(
    exe: &Path,
    body: &Path,
    range: Range<usize>,
    resume: Resume,
    options: &Options,
    results: &mpsc::Sender<Line>,
    again: &mut Option<Step>,
) -> Result<Option<(usize, Resume)>, String> {
    let mut command = Command::new(exe);
    command.args(["rdna-devices", "--worker"]).arg(body).args([
        range.start.to_string(),
        range.end.to_string(),
        resume.block.to_string(),
        resume.adapter.to_string(),
    ]);
    if let Some(only) = &options.adapter {
        command.args(["--adapter", only]);
    }
    match options.ways.draw {
        Draw::No => {}
        Draw::Empty => drop(command.arg("--draw")),
        Draw::Run => drop(command.arg("--run")),
    }
    if options.ways.msl {
        command.arg("--msl");
    }
    if let Some(dir) = &options.ways.msl_dir {
        command.arg("--msl-dir").arg(dir);
    }
    if let Some(mib) = options.stack_mib {
        command.env(STACK_ENV, mib.to_string());
    }
    let mut child = (command.stdout(Stdio::piped()).stderr(Stdio::null()))
        .spawn()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let stdout = child.stdout.take().ok_or("the worker has no output")?;
    let (lines, read) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).split(b'\n') {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    // The step begun and not ended, and the last unit a line named.
    let (mut begun, mut reached, mut done, mut timed_out) = (None, range.start, false, false);
    loop {
        let text = match read.recv_timeout(options.timeout) {
            Ok(Ok(text)) => text,
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                timed_out = true;
                break;
            }
        };
        let text = String::from_utf8_lossy(&text);
        let Some(line) = (text.trim().strip_prefix(MARK))
            .and_then(|text| serde_json::from_str::<Line>(text).ok())
        else {
            continue;
        };
        match line {
            Line::Begin {
                unit,
                block,
                adapter,
            } => {
                begun = Some((unit, block, adapter));
                reached = unit;
            }
            Line::Done => done = true,
            line => {
                begun = None;
                let _ = results.send(line);
            }
        }
    }
    let _ = child.kill();
    let status = child.wait().map_err(|e| e.to_string())?;
    if done {
        return Ok(None);
    }
    tracing::debug!(?begun, timed_out, %status, "a worker stopped");
    // A driver under load ends a process now and then: a step is counted once it has ended two.
    if let Some(step) = begun.filter(|step: &Step| !timed_out && *again != Some(*step)) {
        *again = Some(step);
        let resume = Resume {
            block: step.1,
            adapter: step.2.unwrap_or(0),
        };
        return Ok(Some((step.0, resume)));
    }
    let next = match begun {
        Some((unit, block, None)) => {
            let why = match timed_out {
                true => "the translator ran past the time limit".to_owned(),
                false => format!("the translator ended the process ({status})"),
            };
            let _ = results.send(Line::Front { unit, block, why });
            let resume = Resume {
                block: block + 1,
                adapter: 0,
            };
            (unit, resume)
        }
        Some((unit, block, Some(adapter))) => {
            let outcome = match timed_out {
                true => Outcome::TimedOut,
                false => Outcome::Crashed(status.to_string()),
            };
            let _ = results.send(Line::Device {
                unit,
                block,
                adapter,
                outcome,
                ms: 0.0,
            });
            let resume = Resume {
                block,
                adapter: adapter + 1,
            };
            (unit, resume)
        }
        // It ended between two steps: the unit it was on is given up, so a worker that never starts ends the range.
        None => {
            let why = format!("the worker ended outside a step ({status})");
            let _ = results.send(Line::Front {
                unit: reached,
                block: 0,
                why,
            });
            (reached + 1, Resume::default())
        }
    };
    Ok(Some(next))
}

/// The cache worker over `list`, and what it printed.
fn one_cache_worker(
    exe: &Path,
    body: &Path,
    adapter: usize,
    list: &Path,
    file: &Path,
    load: bool,
    options: &Options,
) -> Result<(usize, f64, usize), String> {
    let mut command = Command::new(exe);
    (command.args(["rdna-devices", "--cache-worker"]).arg(body))
        .arg(adapter.to_string())
        .arg(list)
        .arg(file)
        .arg(if load { "load" } else { "save" });
    if let Some(only) = &options.adapter {
        command.args(["--adapter", only]);
    }
    if let Some(mib) = options.stack_mib {
        command.env(STACK_ENV, mib.to_string());
    }
    let output = command.output().map_err(|e| e.to_string())?;
    (String::from_utf8_lossy(&output.stdout).lines())
        .find_map(|line| serde_json::from_str(line.trim().strip_prefix(MARK)?).ok())
        .ok_or_else(|| {
            let said = String::from_utf8_lossy(&output.stderr);
            let last = said.lines().last().unwrap_or_default();
            format!(
                "the cache worker gave no result ({}): {last}",
                output.status
            )
        })
}

/// The container `body` on every adapter of this machine, through worker processes of the
/// program `exe`; `progress` is told the units done and their count.
pub fn run(
    exe: &Path,
    body: &[u8],
    options: &Options,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Report, String> {
    let file = Sgd5File::open(body).map_err(|e| e.to_string())?;
    let units = units(&file);
    let digest: String = (Sha1::digest(body).iter())
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut adapters: Vec<(Adapter, Option<String>)> = (open_all(options.adapter.as_deref())
        .iter())
    .map(|gpu| (gpu.described(), gpu.cache_key()))
    .collect();
    if options.ways.msl {
        adapters.push((metal_text_adapter(), None));
    }
    if adapters.is_empty() {
        let unsupported = instance::UNSUPPORTED;
        return Err(format!("no adapter a device opens on. {unsupported}"));
    }
    let range = (options.units.clone()).map_or(0..units.len(), |r| r.start..r.end.min(units.len()));
    let scratch = std::env::temp_dir().join(format!("or-rdna-devices-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let only = (!options.techniques.is_empty()).then(|| {
        let names: Vec<&str> = options.techniques.iter().map(String::as_str).collect();
        technique_units(&file, &units, &names)
    });
    let only = match (only, &options.picked) {
        (Some(only), Some(picked)) => Some(&only & picked),
        (only, picked) => only.or_else(|| picked.clone()),
    };
    Programs::write(&scratch, &file, body, &units, only.as_ref())?;
    if let Some(earlier) = &options.avoid {
        let same = |run: &&Run, adapter: &Adapter| {
            let (a, b) = (&run.adapter, adapter);
            (&a.name, &a.backend, &a.driver, &a.driver_info)
                == (&b.name, &b.backend, &b.driver, &b.driver_info)
        };
        let avoid: Vec<BTreeSet<String>> = (adapters.iter())
            .map(|(adapter, _)| {
                (earlier.runs.iter().filter(|run| same(run, adapter)))
                    .flat_map(|run| run.crashed.values().flatten().cloned())
                    .collect()
            })
            .collect();
        Programs::write_avoided(&scratch, &avoid)?;
    }
    // The ranges of the units checked: the range asked for, or the runs of a technique's pairs in it.
    let checked = match &only {
        None => vec![(range.start, range.end)],
        Some(only) => {
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for &unit in only.range(range.clone()) {
                match runs.last_mut() {
                    Some(run) if run.1 == unit => run.1 += 1,
                    _ => runs.push((unit, unit + 1)),
                }
            }
            runs
        }
    };
    let body_path = scratch.clone();

    let mut container = Container {
        programs: file.programs.len(),
        pairs: units.len() - file.programs.len(),
        front: BTreeMap::new(),
        notes: BTreeMap::new(),
    };
    let mut runs: Vec<Run> = (adapters.iter())
        .map(|(adapter, _)| Run {
            container: digest.clone(),
            adapter: adapter.clone(),
            stack_mib: options.stack_mib.unwrap_or_else(stack_mib),
            units: checked.clone(),
            drawn: options.ways.draw != Draw::No && adapter.shader_compiler != "text",
            ran: options.ways.draw == Draw::Run && adapter.shader_compiler != "text",
            ..Default::default()
        })
        .collect();
    let mut created: Vec<Vec<(usize, usize)>> = vec![Vec::new(); adapters.len()];
    // A worker is given a run of the units to check, so a technique's few pairs start few workers.
    let todo: Vec<usize> = match &only {
        Some(only) => only.range(range.clone()).copied().collect(),
        None => range.clone().collect(),
    };
    let (cursor, finished) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let failed = std::thread::scope(|scope| {
        let (results, lines) = mpsc::channel::<Line>();
        let handles: Vec<_> = (0..options.jobs.max(1))
            .map(|_| {
                let results = results.clone();
                let (cursor, finished, todo, body_path) = (&cursor, &finished, &todo, &body_path);
                scope.spawn(move || -> Result<(), String> {
                    loop {
                        let chunk = options.chunk.max(1);
                        let first = cursor.fetch_add(chunk, Ordering::Relaxed);
                        let Some(given) = todo.get(first..(first + chunk).min(todo.len())) else {
                            return Ok(());
                        };
                        let (Some(&start), Some(&last)) = (given.first(), given.last()) else {
                            return Ok(());
                        };
                        let end = last + 1;
                        let (mut at, mut again) = (Some((start, Resume::default())), None);
                        while let Some((from, resume)) = at.filter(|(from, _)| *from < end) {
                            let (range, again) = (from..end, &mut again);
                            at = one_worker(
                                exe, body_path, range, resume, options, &results, again,
                            )?;
                        }
                        let done = finished.fetch_add(given.len(), Ordering::Relaxed);
                        progress(done + given.len(), todo.len());
                    }
                })
            })
            .collect();
        drop(results);
        for line in lines {
            match line {
                Line::Front { unit, block, why } => {
                    let name = units[unit].name(block);
                    container.front.entry(why).or_default().push(name);
                }
                Line::Device {
                    unit,
                    block,
                    adapter,
                    outcome,
                    ms,
                } => {
                    if let Some(run) = runs.get_mut(adapter) {
                        if outcome == Outcome::Created {
                            created[adapter].push((unit, block));
                        }
                        run.count(&units[unit].name(block), &outcome, ms);
                    }
                }
                Line::Note { unit, block, what } => {
                    let name = units[unit].name(block);
                    container.notes.entry(what).or_default().push(name);
                }
                Line::Begin { .. } | Line::Done => {}
            }
        }
        (handles.into_iter())
            .filter_map(|handle| handle.join().ok()?.err())
            .next()
    });
    let caches = (|| -> Result<(), String> {
        if let (None, Some(dir)) = (&failed, &options.cache_dir) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            for (adapter, (_, key)) in adapters.iter().enumerate() {
                let Some(key) = key else { continue };
                created[adapter].sort_unstable();
                let list = scratch.join("created.json");
                let text = serde_json::to_string(&created[adapter]).map_err(|e| e.to_string())?;
                std::fs::write(&list, text).map_err(|e| e.to_string())?;
                let name = format!("{}-{key}", &digest[..12]);
                let cache_file = dir.join(&name);
                let pass = |load| {
                    one_cache_worker(exe, &body_path, adapter, &list, &cache_file, load, options)
                };
                match pass(false).and_then(|empty| Ok((empty, pass(true)?))) {
                    Ok(((pipelines, empty_ms, bytes), (_, loaded_ms, _))) => {
                        runs[adapter].cache = Some(Cache {
                            file: name,
                            bytes,
                            pipelines,
                            empty_ms,
                            loaded_ms,
                        });
                    }
                    Err(why) => tracing::warn!("no pipeline cache for adapter {adapter}: {why}"),
                }
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    if let Some(why) = failed {
        return Err(why);
    }
    caches?;
    // The workers' lines arrive in no order: a shorter name first, so numbers sort as numbers.
    let sorted = |units: &mut Vec<String>| units.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
    container.front.values_mut().for_each(sorted);
    container.notes.values_mut().for_each(sorted);
    // A unit a worker was started again on was translated again, and noted again.
    container.notes.values_mut().for_each(Vec::dedup);
    for run in &mut runs {
        run.rejected.values_mut().for_each(sorted);
        run.skipped.values_mut().for_each(sorted);
        run.crashed.values_mut().for_each(sorted);
        sorted(&mut run.timed_out);
    }
    Ok(Report {
        version: VERSION,
        containers: BTreeMap::from([(digest, container)]),
        runs,
    })
}

#[cfg(test)]
mod tests;
