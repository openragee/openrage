//! Direct3D 12's compiled stages of a GTA VI pass, kept between sessions: a stage is written as
//! HLSL as the backend writes it for the pass's layout, compiled by DXC once, and handed to the
//! device as DXIL, which the backend takes without compiling it again.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use naga::back::hlsl;
use naga::ShaderStage;
use sha1::{Digest, Sha1};

use super::bindings::arrayed;
use super::pipeline::entry_of;

/// The wgpu-hal whose Direct3D 12 registers [`options`] lays out as it does.
// ⚠ The layout is the backend's own and private: another version may number registers otherwise,
// and a stage of other numbers can be taken by the device and draw from the wrong resources.
const MIRRORED: &str = "29.0.1";
/// The head of a kept stage's file: a change to the file or to what a key holds takes a new one.
const MAGIC: &[u8; 8] = b"ORDXIL\0\x01";
const DIGEST: usize = 20;
/// The highest shader model a stage is compiled for, under what the compiler offers: what every
/// card of the last years takes, and above what a translated stage needs.
const MOST: hlsl::ShaderModel = hlsl::ShaderModel::V6_6;

/// The features a device is asked for where its adapter has them, to be handed a kept stage.
pub(super) const FEATURES: wgpu::Features = wgpu::Features::PASSTHROUGH_SHADERS;

static FOLDER: OnceLock<PathBuf> = OnceLock::new();
/// The file whose being there says that the device refused a stage compiled here, named by its
/// adapter, driver and compiler.
static MARK: OnceLock<PathBuf> = OnceLock::new();
static REFUSED: AtomicBool = AtomicBool::new(false);
static PARTS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// The files of the stages handed for the pass this thread builds.
    static HANDED: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    /// Whether this thread builds a pass again with no stage of this module.
    static OFF: Cell<bool> = const { Cell::new(false) };
}

/// Keep the compiled stages in `folder`; the first call of a process decides.
pub(super) fn keep_in(folder: PathBuf) {
    let _ = FOLDER.set(folder);
}

/// Whether a device of `backend` with `features` is handed stages compiled here.
fn used_on(backend: wgpu::Backend, features: wgpu::Features) -> bool {
    backend == wgpu::Backend::Dx12 && features.contains(FEATURES)
}

/// The module `device` draws `module` with, one of `stages` (the vertex one, the pixel one): DXIL
/// kept or compiled here under Direct3D 12, else the naga module as wgpu compiles it.
pub(super) fn shader(
    device: &wgpu::Device,
    module: &naga::Module,
    stages: (&naga::Module, &naga::Module),
) -> wgpu::ShaderModule {
    if let Some(dxil) = dxil_of(device, module, stages) {
        // SAFETY: the DXIL is what DXC wrote of HLSL laid out for the registers the backend
        // gives this pass's layout, read back whole under its digest; a stage the device refuses
        // is reported in the pass's error scopes, and the pass is then built without it.
        return unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: None,
                dxil: Some(Cow::Owned(dxil)),
                ..Default::default()
            })
        };
    }
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
    })
}

/// Forgets the stages this thread handed for the pass it built: where the device `refused` the
/// pass and it was handed one, their files are deleted and `true` says to build it again in
/// [`without`].
pub(super) fn forget_handed(refused: bool) -> bool {
    let handed = HANDED.with(|handed| std::mem::take(&mut *handed.borrow_mut()));
    if !refused || handed.is_empty() {
        return false;
    }
    for file in &handed {
        let _ = std::fs::remove_file(file);
    }
    tracing::warn!(
        stages = handed.len(),
        "the device refused a pass of Direct3D 12 stages compiled to be kept: they are deleted and the pass is built again"
    );
    true
}

/// `build` with no stage of this module; where it then builds, the device is taken to refuse
/// them, in this session and the next ones, until its driver or the compiler changes.
pub(super) fn without<T, E>(build: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    OFF.with(|off| off.set(true));
    let built = build();
    OFF.with(|off| off.set(false));
    if built.is_ok() {
        REFUSED.store(true, Ordering::Relaxed);
        if let Some(mark) = MARK.get() {
            store(mark, b"");
        }
        tracing::warn!("the device refuses Direct3D 12 stages compiled here: passes are compiled by wgpu until its driver or the compiler changes");
    }
    built
}

/// The DXIL of `module` for `device`, kept or compiled now; `None` where wgpu compiles it.
fn dxil_of(
    device: &wgpu::Device,
    module: &naga::Module,
    stages: (&naga::Module, &naga::Module),
) -> Option<Vec<u8>> {
    if OFF.with(Cell::get) || REFUSED.load(Ordering::Relaxed) {
        return None;
    }
    let info = device.adapter_info();
    if !used_on(info.backend, device.features()) {
        return None;
    }
    let folder = FOLDER.get()?;
    let dxc = dxc::compiler()?;
    let adapter = format!(
        "{} {:x}:{:x} {} {}",
        info.name, info.vendor, info.device, info.driver, info.driver_info
    );
    let mark = MARK.get_or_init(|| {
        let named = hex(&digest(&[MIRRORED, adapter.as_str(), dxc.identity()]));
        folder.join(format!("refused-{named}"))
    });
    if mark.exists() {
        REFUSED.store(true, Ordering::Relaxed);
        return None;
    }
    let stage = match std::ptr::eq(module, stages.0) {
        true => ShaderStage::Vertex,
        false => ShaderStage::Fragment,
    };
    let written = match hlsl_of(stages, stage, dxc.shader_model()) {
        Ok(written) => written,
        Err(why) => {
            tracing::debug!(%why, "a Direct3D 12 stage is compiled by wgpu");
            return None;
        }
    };
    let half = device.features().contains(wgpu::Features::SHADER_F16);
    let arguments = arguments(&written.entry, &profile(stage, dxc.shader_model()), half);
    let key = Key {
        hlsl: &written.source,
        arguments: &arguments,
        compiler: dxc.identity(),
        adapter: &adapter,
    }
    .digest();
    let file = folder.join(format!("{}.dxil", hex(&key)));
    let kept = std::fs::read(&file).ok().and_then(|bytes| {
        let dxil = decode(&bytes, &key).map(<[u8]>::to_vec);
        if dxil.is_none() {
            tracing::warn!(file = %file.display(), "a kept Direct3D 12 stage is not whole or not this stage's: it is compiled again");
            let _ = std::fs::remove_file(&file);
        }
        dxil
    });
    let dxil = match kept {
        Some(dxil) => dxil,
        None => {
            let started = std::time::Instant::now();
            let dxil = match dxc.compile(&written.source, &arguments) {
                Ok(dxil) => dxil,
                Err(why) => {
                    tracing::debug!(%why, "a Direct3D 12 stage is compiled by wgpu");
                    return None;
                }
            };
            tracing::info!(elapsed = ?started.elapsed(), "a Direct3D 12 stage was compiled to be kept");
            store(&file, &encode(&dxil, &key));
            dxil
        }
    };
    HANDED.with(|handed| handed.borrow_mut().push(file));
    Some(dxil)
}

/// A stage as HLSL, with the name its entry point was written under.
struct Written {
    source: String,
    entry: String,
}

/// The HLSL of `stage` of `stages` as the backend writes it for the pass's layout in `model`: a
/// vertex stage keeps only the outputs the pixel stage reads.
fn hlsl_of(
    stages: (&naga::Module, &naga::Module),
    stage: ShaderStage,
    model: hlsl::ShaderModel,
) -> Result<Written, String> {
    let (vertex, fragment) = stages;
    let module = match stage {
        ShaderStage::Vertex => vertex,
        _ => fragment,
    };
    let options = options(&groups_of(vertex, fragment)?, model)?;
    let entry = (entry_of(module, stage).map_err(|e| e.to_string())?.name).clone();
    let pixel_entry = match stage {
        ShaderStage::Vertex => Some(
            (entry_of(fragment, ShaderStage::Fragment)
                .map_err(|e| e.to_string())?
                .name)
                .clone(),
        ),
        _ => None,
    };
    let pixel = match &pixel_entry {
        Some(name) => Some(
            hlsl::FragmentEntryPoint::new(fragment, name).ok_or("the pixel stage has no entry")?,
        ),
        None => None,
    };
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .subgroup_stages(naga::valid::ShaderStages::all())
    .subgroup_operations(naga::valid::SubgroupOperationSet::all())
    .validate(module)
    .map_err(|e| format!("the stage is not valid: {}", e.as_inner()))?;
    let constants = naga::back::PipelineConstants::default();
    let (module, info) = naga::back::pipeline_constants::process_overrides(
        module,
        &info,
        Some((stage, entry.as_str())),
        &constants,
    )
    .map_err(|e| e.to_string())?;
    let pipeline = hlsl::PipelineOptions {
        entry_point: Some((stage, entry.clone())),
    };
    let mut source = String::new();
    let mut reflection = hlsl::Writer::new(&mut source, &options, &pipeline)
        .write(&module, &info, pixel.as_ref())
        .map_err(|e| e.to_string())?;
    let entry = (reflection.entry_point_names.pop())
        .ok_or("no entry point was written")?
        .map_err(|e| e.to_string())?;
    Ok(Written { source, entry })
}

/// The entries of each group of the pass of the two stages, as its layout is made of them: an
/// even group the vertex stage's, an odd one the pixel stage's; `None` where neither binds.
fn groups_of(
    vertex: &naga::Module,
    fragment: &naga::Module,
) -> Result<Vec<Option<Vec<wgpu::BindGroupLayoutEntry>>>, String> {
    let mut groups: Vec<Option<Vec<wgpu::BindGroupLayoutEntry>>> = Vec::new();
    for (module, parity, visibility) in [
        (vertex, 0, wgpu::ShaderStages::VERTEX),
        (fragment, 1, wgpu::ShaderStages::FRAGMENT),
    ] {
        for (requirement, count) in arrayed(module).map_err(|e| e.to_string())? {
            if requirement.group % 2 != parity {
                return Err("a stage binds outside its groups".to_string());
            }
            let at = requirement.group as usize;
            if groups.len() <= at {
                groups.resize(at + 1, None);
            }
            let mut entry = (requirement.resource).layout_entry(requirement.binding, visibility);
            entry.count = count;
            groups[at].get_or_insert_with(Vec::new).push(entry);
        }
    }
    Ok(groups)
}

/// The writer's options for a pipeline layout of `groups` and no immediates, as the backend
/// makes them: views numbered on across the groups by kind, a group's samplers as indices into
/// a buffer of its own that follows its views, and the drawing constants after every group.
fn options(
    groups: &[Option<Vec<wgpu::BindGroupLayoutEntry>>],
    shader_model: hlsl::ShaderModel,
) -> Result<hlsl::Options, String> {
    use wgpu::{BindingType as B, BufferBindingType as Buffer};
    let mut binding_map = hlsl::BindingMap::default();
    let mut sampler_buffer_binding_map = hlsl::SamplerIndexBufferBindingMap::default();
    let mut cbv = hlsl::BindTarget::default();
    let mut srv = hlsl::BindTarget::default();
    let mut uav = hlsl::BindTarget::default();
    for (group, entries) in groups.iter().enumerate() {
        let Some(entries) = entries else {
            continue;
        };
        let group = group as u32;
        let mut entries = entries.clone();
        entries.sort_by_key(|entry| entry.binding);
        for entry in &entries {
            let target = match entry.ty {
                B::Sampler(_) => continue,
                B::Buffer {
                    has_dynamic_offset: true,
                    ..
                }
                | B::AccelerationStructure { .. }
                | B::ExternalTexture => {
                    return Err("a binding whose registers are not laid out here".to_string())
                }
                B::Buffer {
                    ty: Buffer::Uniform,
                    ..
                } => &mut cbv,
                B::Buffer {
                    ty: Buffer::Storage { read_only: true },
                    ..
                }
                | B::Texture { .. } => &mut srv,
                B::Buffer {
                    ty: Buffer::Storage { read_only: false },
                    ..
                }
                | B::StorageTexture { .. } => &mut uav,
            };
            binding_map.insert(
                naga::ResourceBinding {
                    group,
                    binding: entry.binding,
                },
                hlsl::BindTarget {
                    binding_array_size: entry.count.map(NonZeroU32::get),
                    dynamic_storage_buffer_offsets_index: None,
                    ..*target
                },
            );
            target.register += entry.count.map_or(1, NonZeroU32::get);
        }
        let mut samplers = 0;
        for entry in (entries.iter()).filter(|entry| matches!(entry.ty, B::Sampler(_))) {
            binding_map.insert(
                naga::ResourceBinding {
                    group,
                    binding: entry.binding,
                },
                hlsl::BindTarget {
                    space: 255,
                    register: samplers,
                    binding_array_size: None,
                    dynamic_storage_buffer_offsets_index: None,
                    restrict_indexing: false,
                },
            );
            samplers += 1;
        }
        if samplers != 0 {
            sampler_buffer_binding_map.insert(hlsl::SamplerIndexBufferKey { group }, srv);
            srv.register += 1;
        }
    }
    let heap = |register| hlsl::BindTarget {
        space: 0,
        register,
        binding_array_size: None,
        dynamic_storage_buffer_offsets_index: None,
        restrict_indexing: false,
    };
    Ok(hlsl::Options {
        shader_model,
        binding_map,
        fake_missing_bindings: false,
        special_constants_binding: Some(cbv),
        immediates_target: None,
        dynamic_storage_buffer_offsets_targets: Default::default(),
        zero_initialize_workgroup_memory: true,
        restrict_indexing: true,
        sampler_heap_target: hlsl::SamplerHeapBindTargets {
            standard_samplers: heap(0),
            comparison_samplers: heap(2048),
        },
        sampler_buffer_binding_map,
        external_texture_binding_map: Default::default(),
        force_loop_bounding: true,
        ray_query_initialization_tracking: true,
    })
}

/// The shader model a compiler of version `major.minor` writes, no higher than [`MOST`].
#[cfg_attr(not(windows), allow(dead_code))]
fn shader_model(major: u32, minor: u32) -> hlsl::ShaderModel {
    use hlsl::ShaderModel as M;
    let minor = if major > 1 { u32::MAX } else { minor };
    match minor {
        0 => M::V6_0,
        1 => M::V6_1,
        2 => M::V6_2,
        3 => M::V6_3,
        4 => M::V6_4,
        5 => M::V6_5,
        _ => MOST,
    }
}

/// The target DXC is given for `stage` in `model`, such as `vs_6_6`.
fn profile(stage: ShaderStage, model: hlsl::ShaderModel) -> String {
    format!("{}_{}", stage.to_hlsl_str(), model.to_str())
}

/// What DXC is given, as the backend gives it but for its debugging pair (`-Zi -Od`), which a
/// developer's build asks for and which leaves a stage unoptimised: `half` where the device
/// takes 16-bit numbers.
fn arguments(entry: &str, profile: &str, half: bool) -> Vec<String> {
    let given = [
        "-E",
        entry,
        "-T",
        profile,
        "-HV",
        "2018",
        "-no-warnings",
        "-Ges",
    ];
    let mut arguments = Vec::from(given.map(str::to_owned));
    if half {
        arguments.push("-enable-16bit-types".to_owned());
    }
    arguments
}

/// What the bytes DXC writes of a stage depend on, and the device that takes them.
struct Key<'a> {
    hlsl: &'a str,
    arguments: &'a [String],
    compiler: &'a str,
    adapter: &'a str,
}

impl Key<'_> {
    fn digest(&self) -> [u8; DIGEST] {
        let mut parts = vec![MIRRORED, self.hlsl, self.compiler, self.adapter];
        parts.extend(self.arguments.iter().map(String::as_str));
        digest(&parts)
    }
}

/// The digest of `parts`, each counted, so that no two lists of parts read the same.
fn digest(parts: &[&str]) -> [u8; DIGEST] {
    let mut hasher = Sha1::new();
    hasher.update(MAGIC);
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    let mut out = [0; DIGEST];
    out.copy_from_slice(&hasher.finalize());
    out
}

fn sha1_of(bytes: &[u8]) -> [u8; DIGEST] {
    let mut out = [0; DIGEST];
    out.copy_from_slice(&Sha1::digest(bytes));
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The file a stage of `key` is kept in: the head, the key, the digest of the DXIL and the DXIL.
fn encode(dxil: &[u8], key: &[u8; DIGEST]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MAGIC.len() + 2 * DIGEST + dxil.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(key);
    out.extend_from_slice(&sha1_of(dxil));
    out.extend_from_slice(dxil);
    out
}

/// The DXIL [`encode`] wrote for `key`; `None` for a file cut short, changed, of another form or
/// of another key.
fn decode<'a>(bytes: &'a [u8], key: &[u8; DIGEST]) -> Option<&'a [u8]> {
    let rest = bytes.strip_prefix(MAGIC.as_slice())?;
    let rest = rest.strip_prefix(key.as_slice())?;
    if rest.len() <= DIGEST {
        return None;
    }
    let (sum, dxil) = rest.split_at(DIGEST);
    (sum == sha1_of(dxil).as_slice()).then_some(dxil)
}

/// Writes `bytes` beside `file` and moves them there: a session that reads it never sees half.
fn store(file: &Path, bytes: &[u8]) {
    let part = PARTS.fetch_add(1, Ordering::Relaxed);
    let partial = file.with_extension(format!("{}-{part}.part", std::process::id()));
    let stored = (file.parent().map_or(Ok(()), std::fs::create_dir_all))
        .and_then(|()| std::fs::write(&partial, bytes))
        .and_then(|()| std::fs::rename(&partial, file));
    if let Err(error) = stored {
        let _ = std::fs::remove_file(&partial);
        tracing::debug!(%error, "a compiled Direct3D 12 stage is not kept");
    }
}

#[cfg(windows)]
mod dxc {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use naga::back::hlsl;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Graphics::Direct3D::Dxc;

    type CreateInstance = unsafe extern "system" fn(
        *const windows::core::GUID,
        *const windows::core::GUID,
        *mut *mut core::ffi::c_void,
    ) -> windows::core::HRESULT;

    /// The `dxcompiler.dll` Direct3D 12 compiles with, and what names it in a key.
    pub(super) struct Compiler {
        library: libloading::Library,
        identity: String,
        shader_model: hlsl::ShaderModel,
    }

    static COMPILER: OnceLock<Option<Compiler>> = OnceLock::new();

    /// The compiler the instance was given, loaded once; `None` where it was given FXC or none.
    pub(super) fn compiler() -> Option<&'static Compiler> {
        let loaded = COMPILER.get_or_init(|| {
            let path = path()?;
            (Compiler::load(&path))
                .map_err(|why| tracing::debug!(%why, "Direct3D 12 stages are not kept"))
                .ok()
        });
        loaded.as_ref()
    }

    /// The `dxcompiler.dll` [`crate::instance::dx12_compiler`] gives an instance.
    fn path() -> Option<PathBuf> {
        match wgpu::Dx12Compiler::from_env() {
            Some(wgpu::Dx12Compiler::DynamicDxc { dxc_path }) => Some(PathBuf::from(dxc_path)),
            Some(wgpu::Dx12Compiler::Auto) | None => crate::instance::find_dxc(),
            Some(_) => None,
        }
    }

    impl Compiler {
        fn load(path: &Path) -> Result<Self, String> {
            let meta = std::fs::metadata(path).ok();
            let length = meta.as_ref().map_or(0, |meta| meta.len());
            let stamp = (meta.and_then(|meta| meta.modified().ok()))
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_nanos());
            // SAFETY: the library is the compiler the instance loaded from the same file, whose
            // initialisers have run in this process already.
            let library = unsafe { libloading::Library::new(path) }.map_err(|e| e.to_string())?;
            let mut compiler = Self {
                library,
                identity: String::new(),
                shader_model: hlsl::ShaderModel::V6_0,
            };
            // A compiler older than its version's interface is 1.0.
            let (mut major, mut minor) = (1, 0);
            if let Ok(version) = compiler.instance()?.cast::<Dxc::IDxcVersionInfo>() {
                // SAFETY: both point at numbers that outlive the call.
                if unsafe { version.GetVersion(&mut major, &mut minor) }.is_err() {
                    (major, minor) = (1, 0);
                }
            }
            compiler.shader_model = super::shader_model(major, minor);
            compiler.identity = format!("{} {length} {stamp} {major}.{minor}", path.display());
            Ok(compiler)
        }

        pub(super) fn identity(&self) -> &str {
            &self.identity
        }

        pub(super) fn shader_model(&self) -> hlsl::ShaderModel {
            self.shader_model
        }

        /// A compiler of its own for each stage: one is not shared between threads.
        fn instance(&self) -> Result<Dxc::IDxcCompiler3, String> {
            // SAFETY: every `dxcompiler.dll` exports `DxcCreateInstance` of this signature.
            let create = unsafe { self.library.get::<CreateInstance>(b"DxcCreateInstance") }
                .map_err(|e| e.to_string())?;
            let mut made: Option<Dxc::IDxcCompiler3> = None;
            // SAFETY: `made` is where the interface of the identifier asked for is written.
            unsafe {
                create(
                    &Dxc::CLSID_DxcCompiler,
                    &<Dxc::IDxcCompiler3 as Interface>::IID,
                    (&mut made as *mut Option<Dxc::IDxcCompiler3>).cast(),
                )
            }
            .ok()
            .map_err(|e| e.to_string())?;
            made.ok_or_else(|| "DXC made no compiler".to_owned())
        }

        /// The DXIL of `source` compiled with `arguments`; `Err` holds what DXC said.
        pub(super) fn compile(
            &self,
            source: &str,
            arguments: &[String],
        ) -> Result<Vec<u8>, String> {
            let compiler = self.instance()?;
            let wide: Vec<Vec<u16>> = (arguments.iter())
                .map(|argument| argument.encode_utf16().chain(Some(0)).collect())
                .collect();
            let pointers: Vec<PCWSTR> = wide.iter().map(|wide| PCWSTR(wide.as_ptr())).collect();
            let buffer = Dxc::DxcBuffer {
                Ptr: source.as_ptr().cast(),
                Size: source.len(),
                Encoding: Dxc::DXC_CP_UTF8.0,
            };
            // SAFETY: the source and the arguments, each ending in a nought, outlive the call.
            let result: Dxc::IDxcResult =
                unsafe { compiler.Compile(&buffer, Some(pointers.as_slice()), None) }
                    .map_err(|e| e.to_string())?;
            let errors: Dxc::IDxcBlobUtf8 = output(&result, Dxc::DXC_OUT_ERRORS)?;
            // SAFETY: a blob holds so many bytes from its pointer while it lives.
            let said = unsafe { errors.GetStringLength() };
            if said != 0 {
                let said = unsafe { std::slice::from_raw_parts(errors.GetStringPointer().0, said) };
                return Err(format!("DXC: {}", String::from_utf8_lossy(said)));
            }
            let object: Dxc::IDxcBlob = output(&result, Dxc::DXC_OUT_OBJECT)?;
            // SAFETY: as above.
            let length = unsafe { object.GetBufferSize() };
            if length == 0 {
                return Err("DXC wrote nothing".to_owned());
            }
            let dxil = unsafe {
                std::slice::from_raw_parts(object.GetBufferPointer().cast::<u8>(), length)
            };
            Ok(dxil.to_vec())
        }
    }

    fn output<T: Interface>(
        result: &Dxc::IDxcResult,
        kind: Dxc::DXC_OUT_KIND,
    ) -> Result<T, String> {
        let mut out: Option<T> = None;
        // SAFETY: `out` is where the output, of the interface `T`, is written.
        unsafe { result.GetOutput::<T>(kind, &mut None, &mut out) }.map_err(|e| e.to_string())?;
        out.ok_or_else(|| "DXC gave no output of that kind".to_owned())
    }
}

#[cfg(not(windows))]
mod dxc {
    /// No compiler: DXC is Direct3D 12's, which only Windows has.
    pub(super) enum Compiler {}

    impl Compiler {
        pub(super) fn identity(&self) -> &str {
            match *self {}
        }

        pub(super) fn shader_model(&self) -> naga::back::hlsl::ShaderModel {
            match *self {}
        }

        pub(super) fn compile(&self, _: &str, _: &[String]) -> Result<Vec<u8>, String> {
            match *self {}
        }
    }

    pub(super) fn compiler() -> Option<&'static Compiler> {
        None
    }
}

#[cfg(test)]
mod tests;
