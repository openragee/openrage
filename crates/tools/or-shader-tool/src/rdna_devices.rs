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
use or_shader_front::rdna::link::{self, Linked};
use or_shader_front::rdna::{self, RdnaError};
use or_shader_front::rdna_bind;
use or_shader_front::rdna_bind::shading::parameter_slots;
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
}

/// Every adapter of every backend the machine has, the software ones too, in wgpu's order; `only`
/// keeps those whose name and backend hold that text, whatever its case.
pub fn open_all(only: Option<&str>) -> Vec<Gpu> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    // A backend may list one adapter once per output it drives.
    let mut seen: Vec<wgpu::AdapterInfo> = Vec::new();
    (adapters.into_iter())
        .filter_map(|adapter| {
            let info = adapter.get_info();
            if seen.contains(&info) {
                return None;
            }
            seen.push(info.clone());
            let named = format!("{} ({:?})", info.name, info.backend).to_lowercase();
            if only.is_some_and(|only| !named.contains(&only.to_lowercase())) {
                return None;
            }
            // An experimental feature is refused unless the device is opened unsafely for it.
            let features = adapter.features() - wgpu::Features::all_experimental_mask();
            let limits = adapter.limits();
            let opened = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("or-shader-tool"),
                required_features: features,
                required_limits: limits.clone(),
                ..Default::default()
            }));
            match opened {
                Ok((device, _queue)) => {
                    device.on_uncaptured_error(Arc::new(|_| {}));
                    Some(Gpu {
                        info,
                        features,
                        limits,
                        device,
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
                wgpu::Backend::Dx12 => (std::env::var("WGPU_DX12_COMPILER").ok())
                    .map_or("fxc".into(), |named| named.to_lowercase()),
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

/// The layout of a pipeline whose modules bind an array: a layout the device derives from a
/// module states no array's length, and takes each for one entry. `None` where no module binds
/// one, or binds something the renderer's own layouts do not take.
fn stated_layout(
    device: &wgpu::Device,
    modules: &[(&naga::Module, wgpu::ShaderStages)],
) -> Option<wgpu::PipelineLayout> {
    let mut groups: BTreeMap<u32, Vec<wgpu::BindGroupLayoutEntry>> = BTreeMap::new();
    let mut arrays = false;
    for (module, visibility) in modules {
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
        for (requirement, count) in arrayed(module).ok()? {
            arrays |= count.is_some();
            let mut entry = (requirement.resource).layout_entry(requirement.binding, *visibility);
            if let wgpu::BindingType::StorageTexture { access, .. } = &mut entry.ty {
                if updated(requirement.group, requirement.binding) {
                    *access = wgpu::StorageTextureAccess::Atomic;
                }
            }
            entry.count = count;
            groups.entry(requirement.group).or_default().push(entry);
        }
    }
    if !arrays {
        return None;
    }
    let last = *groups.keys().next_back()?;
    let layouts: Vec<Option<wgpu::BindGroupLayout>> = (0..=last)
        .map(|group| {
            let entries = groups.get(&group)?;
            Some(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries,
                }),
            )
        })
        .collect();
    let layouts: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Option::as_ref).collect();
    Some(
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &layouts,
            immediate_size: 0,
        }),
    )
}

/// The pipeline of `stages`, its bind group layouts the ones the device derives from what the
/// modules declare, or [`stated_layout`]'s where a module binds an array.
fn create(
    device: &wgpu::Device,
    stages: &Stages,
    cache: Option<&wgpu::PipelineCache>,
) -> Result<(), String> {
    let shader = |module: &naga::Module| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
        })
    };
    let said = |e: or_renderer::game_shaders::pipeline::PipelineError| format!("the check: {e}");
    match stages {
        Stages::Compute(module) => {
            let entry = entry_of(module, ShaderStage::Compute).map_err(said)?;
            let layout = stated_layout(device, &[(module, wgpu::ShaderStages::COMPUTE)]);
            let _pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: layout.as_ref(),
                module: &shader(module),
                entry_point: Some(&entry.name),
                compilation_options: Default::default(),
                cache,
            });
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
            let layout = stated_layout(device, &modules);
            let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: layout.as_ref(),
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
                primitive: wgpu::PrimitiveState::default(),
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
        }
    }
    Ok(())
}

/// What `gpu` makes of `stages`, and the milliseconds it took: the device's refusals are caught
/// through its error scopes, and a panic inside wgpu or naga's writers is a refusal too.
pub fn build(gpu: &Gpu, stages: &Stages, cache: Option<&wgpu::PipelineCache>) -> (Outcome, f64) {
    let start = Instant::now();
    let device = &gpu.device;
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = catch_unwind(AssertUnwindSafe(|| create(device, stages, cache)));
    let validation = pollster::block_on(validation.pop());
    let internal = pollster::block_on(internal.pop());
    let memory = pollster::block_on(memory.pop());
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    // The device frees what was dropped only when it is polled.
    let _ = device.poll(wgpu::PollType::Poll);
    let outcome = match made {
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
    };
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

impl Ready {
    fn settle(&mut self) {
        arrays_apart(&mut self.stages);
        self.bare.as_mut().map(arrays_apart);
        // The pass with no optional part, where there is one, is what a device has to take.
        let least = self.bare.as_ref().unwrap_or(&self.stages);
        self.notes.extend(least.asked());
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
}

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

    pub fn open(dir: &Path) -> Result<Self, String> {
        let said = |e: std::io::Error| format!("{}: {e}", dir.display());
        let text = std::fs::read_to_string(dir.join("plan.json")).map_err(said)?;
        Ok(Self {
            plan: serde_json::from_str(&text).map_err(|e| e.to_string())?,
            blob: Mutex::new(std::fs::File::open(dir.join("programs.bin")).map_err(said)?),
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
            let slots = parameter_slots(&vertex.outputs);
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

/// The note of a pair whose pixel stage reads a parameter its vertex stage does not give.
pub const UNFED: &str =
    "a pair whose pixel stage reads a parameter its vertex stage does not write: read as nought";

/// `ready` on `gpu`, or the feature it needs that `gpu` lacks.
fn tried(gpu: &Gpu, ready: &Ready, cache: Option<&wgpu::PipelineCache>) -> (Outcome, f64) {
    if ready.subgroups && !gpu.features.contains(wgpu::Features::SUBGROUP) {
        return (Outcome::Skipped("SUBGROUP".into()), 0.0);
    }
    let missing = ready.needs - gpu.features;
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter_names().map(|(name, _)| name).collect();
        return (Outcome::Skipped(names.join(" | ")), 0.0);
    }
    let lacks = ready.clips && !gpu.features.contains(wgpu::Features::CLIP_DISTANCES)
        || ready.locations > gpu.limits.max_inter_stage_shader_variables;
    match &ready.bare {
        Some(bare) if lacks => build(gpu, bare, cache),
        _ => build(gpu, &ready.stages, cache),
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
                let (outcome, ms) = tried(gpu, &ready, None);
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
            if let (Outcome::Created, took) = tried(gpu, ready, Some(&cache)) {
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

/// How a check runs.
#[derive(Debug, Clone)]
pub struct Options {
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
    /// Where each adapter's pipeline cache is saved, where its backend has one.
    pub cache_dir: Option<PathBuf>,
    /// The stack a worker's steps run on, in mebibytes; [`stack_mib`]'s where `None`.
    pub stack_mib: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            jobs: std::thread::available_parallelism().map_or(2, |n| (usize::from(n) / 2).max(1)),
            chunk: 128,
            timeout: Duration::from_secs(120),
            units: None,
            adapter: None,
            techniques: Vec::new(),
            cache_dir: None,
            stack_mib: None,
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
    let adapters: Vec<(Adapter, Option<String>)> = (open_all(options.adapter.as_deref()).iter())
        .map(|gpu| (gpu.described(), gpu.cache_key()))
        .collect();
    if adapters.is_empty() {
        return Err("this machine has no adapter a device opens on".into());
    }
    let range = (options.units.clone()).map_or(0..units.len(), |r| r.start..r.end.min(units.len()));
    let scratch = std::env::temp_dir().join(format!("or-rdna-devices-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let only = (!options.techniques.is_empty()).then(|| {
        let names: Vec<&str> = options.techniques.iter().map(String::as_str).collect();
        technique_units(&file, &units, &names)
    });
    Programs::write(&scratch, &file, body, &units, only.as_ref())?;
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
