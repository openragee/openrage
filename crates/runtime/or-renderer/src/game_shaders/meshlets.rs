//! A GTA VI model's shading pass: its mesh program, lowered to a vertex stage, reads the model's
//! own data blocks, its pixel program writes the targets the game's deferred pass writes, and a
//! last draw lights them onto the frame.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};
use naga::ShaderStage;
use wgpu::util::DeviceExt;

use super::bindings::{arrayed, Requirement, Resource};
use super::mesh::Image;
use super::pipeline::{entry_of, fragment_targets_in, Target};
use super::targets::render_target;

/// The vertices of an instance of a lowered mesh program: three corners for each of the 128
/// threads of a group.
pub const VERTICES: u32 = 384;

/// The depth target the passes draw with: the farthest is nought, as in the viewport.
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The least bytes a buffer is made with.
const LEAST: usize = 16;
/// The features a device binds an array of textures with, picked by a number a draw computes.
const TEXTURE_ARRAYS: wgpu::Features = wgpu::Features::TEXTURE_BINDING_ARRAY
    .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING);
/// The features a device binds an array of buffers with, over those of an array of textures.
const BUFFER_ARRAYS: wgpu::Features =
    wgpu::Features::BUFFER_BINDING_ARRAY.union(wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY);

/// The entries of every array a stage binds, at most: a translated program binds its heap whole,
/// 256 entries an array.
const ARRAY_ENTRIES: u32 = 4096;
/// The buffers and textures of one stage a device is asked for where its adapter has them: a
/// translated program binds each buffer the game fills, which is more than a device gives unasked.
const STAGE_BINDINGS: u32 = 64;

/// A developer's aids for reading what a translated pass does, from the environment variable
/// `OPENRAGE_GTA6_SHADED`: words separated by commas. `target=N` shows a target raw in place
/// of the lit frame (1 the colour, 2 the packed normal, 3 the normal, 4 the specular terms, 5
/// the ambient ones, 6 the fourth channels); `pixel=FILE` and `mesh=FILE` draw with a stage
/// of WGSL of one's own in place of the translated one, such as one `rpf scan-shading --wgsl`
/// wrote, edited to store a value; `record=FILE` draws with the bytes of FILE as the record a
/// material's numbers head, the rest of which only the game fills; `plain-device` asks the
/// device for no optional feature and no raised limit, as a device that has none; `unguarded`
/// has the binder let through a stage whose loop counts to a number only the game fills, and
/// no pass is then built but on a software adapter; `meter=frame` has a frame lit by an
/// install's cycles metered whole, as the game meters one, and not by the model's surfaces alone.
#[derive(Debug, Default, PartialEq)]
struct Aids {
    target: f32,
    pixel: Option<String>,
    mesh: Option<String>,
    record: Option<String>,
    plain_device: bool,
    unguarded: bool,
    whole_frame: bool,
}

impl Aids {
    fn parse(stated: &str) -> Self {
        let mut aids = Self::default();
        for word in stated.split(',').map(str::trim) {
            match word.split_once('=') {
                Some(("target", n)) => aids.target = n.parse().unwrap_or(0.0),
                Some(("pixel", file)) => aids.pixel = Some(file.to_owned()),
                Some(("mesh", file)) => aids.mesh = Some(file.to_owned()),
                Some(("record", file)) => aids.record = Some(file.to_owned()),
                Some(("meter", "frame")) => aids.whole_frame = true,
                None if word == "plain-device" => aids.plain_device = true,
                None if word == "unguarded" => aids.unguarded = true,
                _ => {}
            }
        }
        aids
    }
}

/// The dwords of the file the developer's aid `record=FILE` names, where it is given and reads.
pub fn record_aid() -> Option<Vec<u32>> {
    let bytes = std::fs::read(aids().record.as_ref()?).ok()?;
    let (dwords, _) = bytes.as_chunks::<4>();
    Some(dwords.iter().map(|c| u32::from_le_bytes(*c)).collect())
}

/// Whether the developer's aid `meter=frame` is given.
pub fn whole_frame_meter() -> bool {
    aids().whole_frame
}

fn aids() -> &'static Aids {
    static AIDS: std::sync::OnceLock<Aids> = std::sync::OnceLock::new();
    AIDS.get_or_init(|| Aids::parse(&std::env::var("OPENRAGE_GTA6_SHADED").unwrap_or_default()))
}

/// The stage of WGSL in `file`.
fn stage_of(file: &str) -> Result<naga::Module, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    naga::front::wgsl::parse_str(&text).map_err(|e| e.emit_to_string(&text))
}

/// The features a device draws a translated pass with, of those its adapter has, and `limits`
/// raised to what such a pass binds, as far as `has` reaches.
pub fn device_needs(
    features: wgpu::Features,
    has: &wgpu::Limits,
    limits: &mut wgpu::Limits,
) -> wgpu::Features {
    if aids().plain_device {
        return wgpu::Features::empty();
    }
    // Where a backend keeps what its driver compiled of a pipeline, the passes are built with it.
    let kept = features & wgpu::Features::PIPELINE_CACHE;
    let raised = |asked: u32, has: u32| asked.max(has.min(STAGE_BINDINGS));
    limits.max_storage_buffers_per_shader_stage = raised(
        limits.max_storage_buffers_per_shader_stage,
        has.max_storage_buffers_per_shader_stage,
    );
    limits.max_uniform_buffers_per_shader_stage = raised(
        limits.max_uniform_buffers_per_shader_stage,
        has.max_uniform_buffers_per_shader_stage,
    );
    limits.max_sampled_textures_per_shader_stage = raised(
        limits.max_sampled_textures_per_shader_stage,
        has.max_sampled_textures_per_shader_stage,
    );
    limits.max_samplers_per_shader_stage = raised(
        limits.max_samplers_per_shader_stage,
        has.max_samplers_per_shader_stage,
    );
    // Only a program that picks a texture at a place it computes binds an array: a device
    // without them still draws every other pass.
    if !features.contains(TEXTURE_ARRAYS) {
        return kept;
    }
    limits.max_binding_array_elements_per_shader_stage =
        (has.max_binding_array_elements_per_shader_stage).min(ARRAY_ENTRIES);
    match features.contains(BUFFER_ARRAYS) {
        true => kept | TEXTURE_ARRAYS | BUFFER_ARRAYS,
        false => kept | TEXTURE_ARRAYS,
    }
}

/// What fills one binding of a stage. A binding with no feed is noughts, a white texel or a
/// plain sampler.
#[derive(Debug, Clone)]
pub enum Feed {
    /// The bytes of a buffer, cut or padded with noughts to what its module declares.
    Bytes(Vec<u8>),
    /// A constant buffer each frame writes the view into: its bytes, and where the matrix that
    /// takes the world to the clip space starts.
    View {
        bytes: Vec<u8>,
        view_proj: Option<usize>,
    },
    /// The texture, or each entry of an array of them; `None` is a white texel.
    Textures(Vec<Option<Arc<Image>>>),
}

/// One geometry with the two stages that shade it.
#[derive(Debug, Clone)]
pub struct MeshletSurface {
    /// The lowered mesh program, its bindings in group 0 and its arrays in group 2, linked to
    /// `fragment`: it gives the locations that stage takes.
    pub vertex: Arc<naga::Module>,
    /// The pixel program, its bindings in group 1 and its arrays in group 3.
    pub fragment: Arc<naga::Module>,
    /// By group and binding.
    pub feeds: BTreeMap<(u32, u32), Feed>,
    /// The meshlets of the geometry: an instance draws one.
    pub meshlets: u32,
}

/// A buffer the frame writes the view into, and where.
type ViewWrite = (wgpu::Buffer, usize);

struct Draw {
    pipeline: wgpu::RenderPipeline,
    caster: wgpu::RenderPipeline,
    /// By group number.
    groups: Vec<(u32, wgpu::BindGroup)>,
    meshlets: u32,
    views: Vec<ViewWrite>,
}

/// The surfaces of a model ready to draw, with the targets their pixel programs write.
pub struct MeshletScene {
    draws: Vec<Draw>,
    formats: Vec<Option<wgpu::TextureFormat>>,
    targets: Vec<Option<wgpu::TextureView>>,
    depth: wgpu::TextureView,
    resolve: wgpu::RenderPipeline,
    resolve_layout: wgpu::BindGroupLayout,
    resolve_group: wgpu::BindGroup,
    /// The pass that meters the lit surfaces, the texel it writes and the group the resolve
    /// reads it by.
    meter: wgpu::RenderPipeline,
    metered: wgpu::TextureView,
    metered_group: wgpu::BindGroup,
    light: wgpu::Buffer,
    /// The tile the sun's shadow is cast into, the camera's matrix and the light's, which a
    /// frame copies in turn to where the surfaces read the view, and whether this frame casts.
    shadow_map: wgpu::TextureView,
    matrices: wgpu::Buffer,
    casts: std::sync::atomic::AtomicBool,
    /// The meshes of the model its programs do not draw, which the frame draws as a material.
    pub rest: Vec<usize>,
    /// The surfaces it was prepared from that the device has no pass for, each by its place
    /// among them, and why.
    pub refused: Vec<(usize, String)>,
}

/// The texels of a row no image fills: one row of a palette, which is as wide.
pub const ROW: u32 = 256;

/// A texture of one white texel a side, or of `image` where the view is flat, seen as `dimension`.
// ponytail: one level; build the levels below as `stand_in::uploaded_view` does when a texture seen small shimmers.
fn view_of(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    dimension: wgpu::TextureViewDimension,
    sample: wgpu::TextureSampleType,
    image: Option<&Image>,
    blank: [u8; 4],
) -> wgpu::TextureView {
    use wgpu::TextureViewDimension as D;
    let flat = matches!(dimension, D::D2 | D::D2Array);
    let whole = |i: &&Image| {
        i.width > 0 && i.height > 0 && i.rgba.len() == (i.width as usize) * (i.height as usize) * 4
    };
    let (width, height, rgba, srgb) = match image.filter(|i| flat && whole(i)) {
        Some(image) => (image.width, image.height, image.rgba.as_slice(), image.srgb),
        None => (1, 1, &blank[..], false),
    };
    // A row of texels a program loads by a number of its own, such as a palette a vertex picks
    // a tint from: every texel of it is the blank one, since a load past a row's end is nought.
    let row = blank.repeat(ROW as usize);
    let (width, rgba) = match (dimension, image.is_some() && flat) {
        (D::D1, false) => (ROW, row.as_slice()),
        _ => (width, rgba),
    };
    let layers = match dimension {
        D::Cube | D::CubeArray => 6,
        _ => 1,
    };
    let depth = sample == wgpu::TextureSampleType::Depth;
    let descriptor = wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: match dimension {
            D::D1 => wgpu::TextureDimension::D1,
            D::D3 => wgpu::TextureDimension::D3,
            _ => wgpu::TextureDimension::D2,
        },
        format: match sample {
            wgpu::TextureSampleType::Depth => DEPTH,
            wgpu::TextureSampleType::Sint => wgpu::TextureFormat::Rgba8Sint,
            wgpu::TextureSampleType::Uint => wgpu::TextureFormat::Rgba8Uint,
            _ if srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            _ => wgpu::TextureFormat::Rgba8Unorm,
        },
        usage: if depth {
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT
        } else {
            wgpu::TextureUsages::TEXTURE_BINDING
        },
        view_formats: &[],
    };
    let texture = if depth {
        device.create_texture(&descriptor)
    } else {
        device.create_texture_with_data(
            queue,
            &descriptor,
            wgpu::util::TextureDataOrder::default(),
            &rgba.repeat(layers as usize),
        )
    };
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(dimension),
        ..Default::default()
    })
}

/// The image `images` fill entry `entry` of a binding with, an array of `entries` where it is
/// one; `None` is the blank texel.
// ⚠ A translated stage takes the last entry of an array for any entry past it, and no device
// checks one: the last is blank, as an entry no image fills is.
fn image_at(images: &[Option<Arc<Image>>], entry: usize, entries: Option<usize>) -> Option<&Image> {
    let last = entries.is_some_and(|entries| entry + 1 >= entries);
    (images.get(entry).and_then(Option::as_deref)).filter(|_| !last)
}

/// What one binding is bound to, by its place in the lists a group is made from.
enum Bound {
    Buffer(usize),
    /// The same buffer at each of so many entries.
    Buffers(usize, usize),
    /// The views from one place to another.
    Views(usize, usize),
    View(usize),
    Sampler(usize),
}

struct Group {
    group: wgpu::BindGroup,
    views: Vec<ViewWrite>,
}

/// What `module` binds in group `group`, each with the entries of an array; empty where it
/// binds nothing there.
fn required_in(
    device: &wgpu::Device,
    module: &naga::Module,
    group: u32,
) -> Result<Vec<(Requirement, Option<NonZeroU32>)>, String> {
    let mut required = arrayed(module).map_err(|e| format!("a binding of a stage is {e}"))?;
    if required
        .iter()
        .any(|(r, _)| r.group % 2 != group % 2 || r.group >= GROUPS)
    {
        return Err("a stage binds outside its groups".to_string());
    }
    required.retain(|(r, _)| r.group == group);
    for (requirement, _) in required.iter().filter(|(_, count)| count.is_some()) {
        let needed = match requirement.resource {
            Resource::Texture { .. } => TEXTURE_ARRAYS,
            _ => TEXTURE_ARRAYS | BUFFER_ARRAYS,
        };
        if !device.features().contains(needed) {
            return Err("the device binds no array of textures or buffers".to_string());
        }
    }
    Ok(required)
}

/// The layout of the group `module` reads as group `group`; `None` where it binds nothing there.
fn layout_of(
    device: &wgpu::Device,
    module: &naga::Module,
    group: u32,
    visibility: wgpu::ShaderStages,
) -> Result<Option<wgpu::BindGroupLayout>, String> {
    let required = required_in(device, module, group)?;
    if required.is_empty() {
        return Ok(None);
    }
    let entries: Vec<wgpu::BindGroupLayoutEntry> = (required.iter())
        .map(|(requirement, count)| {
            let mut entry = (requirement.resource).layout_entry(requirement.binding, visibility);
            entry.count = *count;
            entry
        })
        .collect();
    Ok(Some(device.create_bind_group_layout(
        &wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        },
    )))
}

/// The groups a pass binds: a stage's own, then each stage's arrays.
const GROUPS: u32 = 4;

/// The bind group `module` reads as group `group`, laid out as `layout`, each binding filled
/// by its feed.
fn bind(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    module: &naga::Module,
    group: u32,
    layout: &wgpu::BindGroupLayout,
    feeds: &BTreeMap<(u32, u32), Feed>,
) -> Result<Group, String> {
    let required = required_in(device, module, group)?;

    let (mut buffers, mut textures, mut samplers) = (Vec::new(), Vec::new(), Vec::new());
    let mut bound: Vec<(u32, Bound)> = Vec::new();
    let mut views = Vec::new();
    for (requirement, count) in &required {
        let Requirement {
            binding, resource, ..
        } = requirement;
        let feed = feeds.get(&(group, *binding));
        let bytes = match feed {
            Some(Feed::Bytes(bytes) | Feed::View { bytes, .. }) => bytes.as_slice(),
            _ => &[],
        };
        let entries = count.map_or(1, NonZeroU32::get) as usize;
        let what = match resource {
            Resource::Uniform { size } => {
                let mut held = bytes.to_vec();
                held.resize((*size as usize).max(LEAST), 0);
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: &held,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
                if let Some(Feed::View {
                    view_proj: Some(at),
                    ..
                }) = feed
                {
                    if at + 64 <= held.len() {
                        views.push((buffer.clone(), *at));
                    }
                }
                buffers.push(buffer);
                Bound::Buffer(buffers.len() - 1)
            }
            Resource::Storage { size, .. } => {
                let mut held = bytes.to_vec();
                let least = (*size as usize).max(LEAST).max(held.len());
                held.resize(least.next_multiple_of(4), 0);
                buffers.push(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: &held,
                        usage: wgpu::BufferUsages::STORAGE,
                    }),
                );
                match count {
                    Some(_) => Bound::Buffers(buffers.len() - 1, entries),
                    None => Bound::Buffer(buffers.len() - 1),
                }
            }
            Resource::Texture {
                dimension, sample, ..
            } => {
                let images: &[Option<Arc<Image>>] = match feed {
                    Some(Feed::Textures(images)) => images,
                    _ => &[],
                };
                // A texture only the game fills reads as noughts, as its buffers do; an entry of
                // the material's own that no pack holds is white.
                let empty = match feed {
                    Some(Feed::Textures(_)) => [255; 4],
                    _ => [0; 4],
                };
                let first = textures.len();
                // The entries no image fills share one white texel.
                let mut blank = None;
                for entry in 0..entries {
                    let array = count.map(|_| entries);
                    let view = match image_at(images, entry, array) {
                        Some(image) => {
                            view_of(device, queue, *dimension, *sample, Some(image), empty)
                        }
                        None => blank
                            .get_or_insert_with(|| {
                                view_of(device, queue, *dimension, *sample, None, empty)
                            })
                            .clone(),
                    };
                    textures.push(view);
                }
                match count {
                    Some(_) => Bound::Views(first, textures.len()),
                    None => Bound::View(first),
                }
            }
            Resource::Sampler { comparison } => {
                samplers.push(device.create_sampler(&wgpu::SamplerDescriptor {
                    address_mode_u: wgpu::AddressMode::Repeat,
                    address_mode_v: wgpu::AddressMode::Repeat,
                    address_mode_w: wgpu::AddressMode::Repeat,
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    compare: comparison.then_some(wgpu::CompareFunction::LessEqual),
                    ..Default::default()
                }));
                Bound::Sampler(samplers.len() - 1)
            }
            Resource::StorageTexture { .. } => {
                return Err("a stage writes a texture".to_string());
            }
        };
        bound.push((*binding, what));
    }

    // The lists an array of views or of buffers is bound from, which the entries borrow.
    let view_lists: Vec<Vec<&wgpu::TextureView>> = (bound.iter())
        .map(|(_, what)| match what {
            Bound::Views(from, to) => textures[*from..*to].iter().collect(),
            _ => Vec::new(),
        })
        .collect();
    let buffer_lists: Vec<Vec<wgpu::BufferBinding<'_>>> = (bound.iter())
        .map(|(_, what)| match what {
            Bound::Buffers(at, entries) => (0..*entries)
                .map(|_| buffers[*at].as_entire_buffer_binding())
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let entries: Vec<wgpu::BindGroupEntry<'_>> = (bound.iter().enumerate())
        .map(|(i, (binding, what))| wgpu::BindGroupEntry {
            binding: *binding,
            resource: match what {
                Bound::Buffer(at) => buffers[*at].as_entire_binding(),
                Bound::Buffers(..) => wgpu::BindingResource::BufferArray(&buffer_lists[i]),
                Bound::Views(..) => wgpu::BindingResource::TextureViewArray(&view_lists[i]),
                Bound::View(at) => wgpu::BindingResource::TextureView(&textures[*at]),
                Bound::Sampler(at) => wgpu::BindingResource::Sampler(&samplers[*at]),
            },
        })
        .collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &entries,
    });
    Ok(Group { group, views })
}

/// One pass built for a device: its two stages as they are drawn, the layout of each group and
/// the pipeline.
struct Built {
    vertex: naga::Module,
    fragment: naga::Module,
    layouts: Vec<Option<wgpu::BindGroupLayout>>,
    formats: Vec<Option<wgpu::TextureFormat>>,
    pipeline: wgpu::RenderPipeline,
    /// The pass that draws the vertex stage alone, as the light sees it.
    caster: wgpu::RenderPipeline,
    /// The stages it was built from, kept so that no other pair comes to lie where they do.
    _of: (Arc<naga::Module>, Arc<naga::Module>),
}

/// A pass as far as it is built: nothing while a worker builds it, then the pass or why the
/// device has none.
type Slot = Arc<Mutex<Option<Result<Arc<Built>, String>>>>;

/// What a device's driver compiled of the passes, and the file it is kept in between sessions.
type Compiled = (Arc<wgpu::PipelineCache>, PathBuf);

/// The stack a worker builds a pass on: the validator and the writers recurse a frame for every
/// nested block of a stage, and a translated stage nests deeper than a thread's stack is by default.
const BUILD_STACK: usize = 64 << 20;

/// The passes built so far, by the two stages a surface draws with: a pass is built once for a
/// device, whatever models it then draws, on a worker, so that a frame never waits for one.
#[derive(Default)]
pub struct MeshletPipelines {
    built: std::collections::HashMap<(usize, usize), Slot>,
    /// The folder what the driver compiled is kept in, and that once it was looked for.
    folder: Option<PathBuf>,
    compiled: Option<Option<Compiled>>,
}

impl MeshletPipelines {
    /// Passes whose compiled form is kept in a file of `folder` between sessions, on a backend
    /// that has one to keep.
    pub fn kept_in(folder: PathBuf) -> Self {
        Self {
            folder: Some(folder),
            ..Self::default()
        }
    }

    /// How many passes are built or being built.
    pub fn len(&self) -> usize {
        self.built.len()
    }

    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }

    /// Have a worker start on the pass of each of `surfaces` that none has started on.
    pub fn start(&mut self, device: &wgpu::Device, surfaces: &[MeshletSurface]) {
        for surface in surfaces {
            // What a pass comes to is asked when its surface is drawn.
            let _ = self.of(device, surface);
        }
    }

    /// The pass of `surface`, which a worker starts to build where none has: `None` until it
    /// is built, `Err` where the device has none for it.
    fn of(
        &mut self,
        device: &wgpu::Device,
        surface: &MeshletSurface,
    ) -> Result<Option<Arc<Built>>, String> {
        let key = (
            Arc::as_ptr(&surface.vertex) as usize,
            Arc::as_ptr(&surface.fragment) as usize,
        );
        if !self.built.contains_key(&key) {
            let folder = self.folder.as_deref();
            let compiled = (self.compiled)
                .get_or_insert_with(|| folder.and_then(|folder| compiled_in(device, folder)))
                .clone();
            let slot = Slot::default();
            let (theirs, device) = (Arc::clone(&slot), device.clone());
            let stages = (Arc::clone(&surface.vertex), Arc::clone(&surface.fragment));
            let work = move || {
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    build_checked(&device, stages, compiled.as_ref())
                }))
                .unwrap_or_else(|_| Err("building the pass stopped on an internal error".into()));
                *theirs.lock().unwrap_or_else(|p| p.into_inner()) = Some(built.map(Arc::new));
            };
            (std::thread::Builder::new().name("meshlet-pass".into()))
                .stack_size(BUILD_STACK)
                .spawn(work)
                .map_err(|e| format!("no worker to build the pass on: {e}"))?;
            self.built.insert(key, slot);
        }
        let state = self.built[&key].lock().unwrap_or_else(|p| p.into_inner());
        state.clone().transpose()
    }
}

/// The device's cache of compiled pipelines, filled from the file `folder` holds for its adapter
/// and driver; `None` on a backend that keeps none.
fn compiled_in(device: &wgpu::Device, folder: &std::path::Path) -> Option<Compiled> {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return None;
    }
    let file = folder.join(wgpu::util::pipeline_cache_key(&device.adapter_info())?);
    let data = std::fs::read(&file).ok();
    // SAFETY: the bytes are what a cache of this adapter and driver gave, since the file is named
    // by the key of both; with `fallback` the device starts an empty cache where the driver does
    // not take them.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("meshlet-passes"),
            data: data.as_deref(),
            fallback: true,
        })
    };
    Some((Arc::new(cache), file))
}

/// [`build`], with what the device refuses of it as the reason, and what its driver compiled
/// kept for the next session.
// A scope is its thread's: nothing a frame draws meanwhile is taken for the pass's.
fn build_checked(
    device: &wgpu::Device,
    stages: (Arc<naga::Module>, Arc<naga::Module>),
    compiled: Option<&Compiled>,
) -> Result<Built, String> {
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let started = std::time::Instant::now();
    let built = build(device, stages, compiled.map(|(cache, _)| &**cache));
    let refused = pollster::block_on(validation.pop()).or(pollster::block_on(internal.pop()));
    tracing::info!(elapsed = ?started.elapsed(), refused = refused.is_some(), "a GTA VI pass was built for the device");
    if let Some(error) = refused {
        return Err(error.to_string());
    }
    if let (Ok(_), Some((cache, file))) = (&built, compiled) {
        // Beside its place, then moved there: a session that reads it never sees half a file.
        let partial = file.with_extension(format!("{}.part", std::process::id()));
        let kept = cache.get_data().map(|data| {
            (file.parent().map_or(Ok(()), std::fs::create_dir_all))
                .and_then(|()| std::fs::write(&partial, data))
                .and_then(|()| std::fs::rename(&partial, file))
        });
        if let Some(Err(error)) = kept {
            tracing::debug!(%error, "the compiled passes are not kept for the next session");
        }
    }
    built
}

/// The mesh stage and the pixel stage a pass of `stages` is built from on the adapter `info`:
/// each as it is, or the one a developer's aid of `aids` puts in its place. `Err` says why no
/// pass of theirs is built there.
fn stages_drawn(
    info: &wgpu::AdapterInfo,
    stages: &(Arc<naga::Module>, Arc<naga::Module>),
    aids: &Aids,
) -> Result<(naga::Module, naga::Module), String> {
    // ⚠ A loop the guard would have refused may never end, and a hardware adapter that waits on one is lost.
    if aids.unguarded && info.device_type != wgpu::DeviceType::Cpu {
        return Err("an unguarded stage is drawn on a software adapter only".to_string());
    }
    let fragment = match &aids.pixel {
        Some(file) => stage_of(file)?,
        None => (*stages.1).clone(),
    };
    let vertex = match &aids.mesh {
        Some(file) => stage_of(file)?,
        None => (*stages.0).clone(),
    };
    if let Some(why) = crate::instance::refused_by_driver(info, &vertex) {
        return Err(why);
    }
    Ok((vertex, fragment))
}

/// The pass of the two `stages` built for `device`.
fn build(
    device: &wgpu::Device,
    stages: (Arc<naga::Module>, Arc<naga::Module>),
    compiled: Option<&wgpu::PipelineCache>,
) -> Result<Built, String> {
    // The two stages come linked: the vertex stage gives the locations the pixel stage takes.
    let (vertex, fragment) = stages_drawn(&device.adapter_info(), &stages, aids())?;
    let mut targets = formats_of(&fragment)?;
    // The first target holds linear light, which eight bits band in the darks: it is kept
    // encoded, as a colour target is, and read back as the light the program wrote.
    if let Some(Some(colour)) = targets.first_mut() {
        colour.format = colour.format.add_srgb_suffix();
    }
    let mut layouts = Vec::new();
    for group in 0..GROUPS {
        let (module, visibility) = match group % 2 {
            0 => (&vertex, wgpu::ShaderStages::VERTEX),
            _ => (&fragment, wgpu::ShaderStages::FRAGMENT),
        };
        layouts.push(layout_of(device, module, group, visibility)?);
    }
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &layouts.iter().map(Option::as_ref).collect::<Vec<_>>(),
        immediate_size: 0,
    });
    let shader = |module: &naga::Module| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
        })
    };
    let vertex_entry = entry_of(&vertex, ShaderStage::Vertex).map_err(|e| e.to_string())?;
    let fragment_entry = entry_of(&fragment, ShaderStage::Fragment).map_err(|e| e.to_string())?;
    let vertex_module = shader(&vertex);
    let caster = sun_shadow::caster(
        device,
        &layout,
        &vertex_module,
        &vertex_entry.name,
        compiled,
    );
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("meshlet-pass"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vertex_module,
            entry_point: Some(&vertex_entry.name),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader(&fragment),
            entry_point: Some(&fragment_entry.name),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: compiled,
    });
    Ok(Built {
        formats: (targets.iter())
            .map(|state| state.as_ref().map(|state| state.format))
            .collect(),
        vertex,
        fragment,
        layouts,
        pipeline,
        caster,
        _of: stages,
    })
}

/// The targets the resolve reads: the colour, the normal, the specular terms and the ambient
/// ones. A device counts eight bytes a sample for each, and takes 32 unasked, so the motion and
/// whatever follows it are drawn to nothing.
const TARGETS: usize = 4;

/// The colour formats the first [`TARGETS`] targets of `fragment` are made in, by location.
fn formats_of(fragment: &naga::Module) -> Result<Vec<Option<wgpu::ColorTargetState>>, String> {
    let entry = entry_of(fragment, ShaderStage::Fragment).map_err(|e| e.to_string())?;
    let target = Target {
        colour: wgpu::TextureFormat::Rgba8Unorm,
        depth: DEPTH,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: None,
    };
    let (mut targets, _) =
        fragment_targets_in(fragment, entry, &target).map_err(|e| e.to_string())?;
    targets.truncate(TARGETS);
    Ok(targets)
}

fn targets_of(
    device: &wgpu::Device,
    formats: &[Option<wgpu::TextureFormat>],
    size: (u32, u32),
) -> (Vec<Option<wgpu::TextureView>>, wgpu::TextureView) {
    let colour = |format: &Option<wgpu::TextureFormat>| {
        format.map(|format| {
            render_target(
                device,
                "meshlet-target",
                size,
                format,
                wgpu::TextureUsages::COPY_SRC,
            )
        })
    };
    let depth = render_target(
        device,
        "meshlet-depth",
        size,
        DEPTH,
        wgpu::TextureUsages::empty(),
    );
    (formats.iter().map(colour).collect(), depth)
}

fn resolve_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    targets: &[Option<wgpu::TextureView>],
    depth: &wgpu::TextureView,
    light: &wgpu::Buffer,
    shadow_map: &wgpu::TextureView,
) -> Result<wgpu::BindGroup, String> {
    let [Some(colour), Some(normal), Some(specular), Some(ambient)] = targets else {
        return Err(
            "the pixel programs do not write the four targets the frame is lit from".into(),
        );
    };
    let view = wgpu::BindingResource::TextureView;
    Ok(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("meshlet-resolve"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: view(colour),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: view(normal),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: view(depth),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: light.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: view(specular),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: view(ambient),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: view(shadow_map),
            },
        ],
    }))
}

/// The sun and the sky a frame is lit by, in linear light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    /// The way towards the sun, or the moon by night.
    pub towards: Vec3,
    /// What it gives a surface that faces it.
    pub sun: Vec3,
    /// What the sky gives from above, and the ground from below.
    pub sky: Vec3,
    pub ground: Vec3,
    pub exposure: f32,
    /// GTA VI's metering and tone curve, where an install's cycle states them: the frame is
    /// then exposed by what its surfaces meter to, times `exposure`.
    pub tone: Option<Tone>,
    pub scene: Scene,
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Sky {
    /// The sky at `hours` of the day under clouds that hide `overcast` of it, from nought to one.
    // Built in, so that a session with no timecycle of an install is lit all the same: a sun
    // that rises at six in the east and sets at eighteen, warm near the horizon, a dim moon
    // opposite it by night, and a sky that greys and dims with the clouds.
    pub fn at(hours: f32, overcast: f32) -> Self {
        let overcast = overcast.clamp(0.0, 1.0);
        let angle = (hours - 6.0) / 12.0 * std::f32::consts::PI;
        let height = angle.sin();
        let towards_sun = Vec3::new(angle.cos(), 0.35, height);
        // Dusk lasts a while after the sun is down, and a night is dim, not black: a model is
        // still there to be looked at.
        let day = smoothstep(-0.2, 0.2, height);
        let up = smoothstep(-0.05, 0.1, height);
        let warm = Vec3::new(1.0, 0.45, 0.2);
        let white = Vec3::new(1.0, 0.96, 0.9);
        let sun = warm.lerp(white, smoothstep(0.0, 0.5, height)) * 3.2 * (1.0 - 0.85 * overcast);
        let moon = Vec3::new(0.55, 0.65, 0.9) * 0.5 * (1.0 - 0.85 * overcast);
        let blue = Vec3::new(0.35, 0.5, 0.8);
        let grey = Vec3::new(0.5, 0.5, 0.52) * (1.0 - 0.5 * overcast);
        let sky = Vec3::new(0.035, 0.05, 0.1).lerp(blue.lerp(grey, overcast), day);
        let (towards, sun) = match height > -0.05 {
            true => (towards_sun, sun * up),
            false => (
                Vec3::new(-towards_sun.x, towards_sun.y, -height),
                moon * (1.0 - day),
            ),
        };
        Self {
            towards: towards.normalize(),
            sun,
            sky,
            ground: sky * 0.3 + sun * 0.04,
            exposure: 1.0,
            tone: None,
            scene: Scene::default(),
        }
    }
}

impl Default for Sky {
    /// A clear day at ten.
    fn default() -> Self {
        Self::at(10.0, 0.0)
    }
}

/// The bytes of what the resolve reads of a frame's light.
const LIGHT_BYTES: usize = 912;
/// The format of the texel the surfaces are metered into: the stops the frame is exposed by.
const METERED: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What a frame lights the targets by.
#[derive(Debug, Clone, Copy)]
pub struct Lighting {
    /// The matrix that takes the world to the clip space.
    pub view_proj: Mat4,
    pub sky: Sky,
    /// Where the camera stands in the world.
    pub camera: Vec3,
}

fn light_bytes(lighting: &Lighting, casts: bool) -> [u8; LIGHT_BYTES] {
    let sky = &lighting.sky;
    let towards = sky.towards.try_normalize().unwrap_or(Vec3::Z);
    let mut words = [0f32; LIGHT_BYTES / 4];
    words[..4].copy_from_slice(&[towards.x, towards.y, towards.z, 0.0]);
    words[4..8].copy_from_slice(&[sky.sun.x, sky.sun.y, sky.sun.z, sky.exposure]);
    words[8..12].copy_from_slice(&[sky.sky.x, sky.sky.y, sky.sky.z, 0.0]);
    words[12..16].copy_from_slice(&[sky.ground.x, sky.ground.y, sky.ground.z, 0.0]);
    let camera = lighting.camera;
    words[16..20].copy_from_slice(&[camera.x, camera.y, camera.z, aids().target]);
    words[20..36].copy_from_slice(&lighting.view_proj.inverse().to_cols_array());
    words[36..72].copy_from_slice(&Tone::words(sky.tone.as_ref()));
    let to_tile = sun_shadow::cast_of(sky).unwrap_or(Mat4::IDENTITY);
    words[72..88].copy_from_slice(&to_tile.to_cols_array());
    words[88..92].copy_from_slice(&sun_shadow::words(sky, casts));
    words[92..212].copy_from_slice(&sky.scene.words());
    words[212..228].copy_from_slice(&lighting.view_proj.to_cols_array());
    bytemuck::cast(words)
}

impl MeshletScene {
    /// Prepare `surfaces` for targets of `size`, lit onto a frame of `frame`'s formats.
    /// `None` while a worker still builds a pass of theirs: the caller asks again. `Err` says
    /// why the scene is not one to draw; what the device refuses of the groups is reported
    /// through its own error scopes.
    pub fn prepare(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &mut MeshletPipelines,
        surfaces: &[MeshletSurface],
        size: (u32, u32),
        frame: &Target,
    ) -> Result<Option<Self>, String> {
        if surfaces.is_empty() {
            return Err("no surface to draw".to_string());
        }
        // Every pass is asked for before any is waited for, so that they build side by side. A
        // surface whose pass the device has none for is left to the caller; the others draw.
        let (mut passes, mut refused) = (Vec::with_capacity(surfaces.len()), Vec::new());
        for (at, surface) in surfaces.iter().enumerate() {
            match pipelines.of(device, surface) {
                Ok(pass) => passes.push((surface, pass)),
                Err(why) => refused.push((at, why)),
            }
        }
        if passes.is_empty() {
            return Err(refused.swap_remove(0).1);
        }
        let Some(passes) = (passes.into_iter())
            .map(|(surface, pass)| Some((surface, pass?)))
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(None);
        };
        let formats = passes[0].1.formats.clone();
        let mut draws = Vec::with_capacity(surfaces.len());
        for (surface, built) in passes {
            if built.formats != formats {
                return Err("two pixel programs write targets of different shapes".to_string());
            }
            let mut draw = Draw {
                pipeline: built.pipeline.clone(),
                caster: built.caster.clone(),
                groups: Vec::new(),
                meshlets: surface.meshlets,
                views: Vec::new(),
            };
            for (group, layout) in (0..).zip(&built.layouts) {
                let Some(layout) = layout else {
                    continue;
                };
                let module = match group % 2 {
                    0 => &built.vertex,
                    _ => &built.fragment,
                };
                let bound = bind(device, queue, module, group, layout, &surface.feeds)?;
                draw.groups.push((group, bound.group));
                draw.views.extend(bound.views);
            }
            draws.push(draw);
        }

        let (targets, depth) = targets_of(device, &formats, size);
        let resolve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet-resolve"),
            source: wgpu::ShaderSource::Wgsl(RESOLVE.into()),
        });
        let texture =
            |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet-resolve"),
            entries: &[
                texture(0, unfiltered),
                texture(1, unfiltered),
                texture(2, wgpu::TextureSampleType::Depth),
                texture(4, unfiltered),
                texture(5, unfiltered),
                texture(6, wgpu::TextureSampleType::Depth),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let light = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet-light"),
            contents: &[0; LIGHT_BYTES],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let metered_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet-metered"),
            entries: &[texture(0, unfiltered)],
        });
        let metered = render_target(
            device,
            "meshlet-metered",
            (1, 1),
            METERED,
            wgpu::TextureUsages::empty(),
        );
        let metered_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet-metered"),
            layout: &metered_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&metered),
            }],
        });
        let meter_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet-meter"),
            bind_group_layouts: &[Some(&resolve_layout)],
            immediate_size: 0,
        });
        let meter = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshlet-meter"),
            layout: Some(&meter_layout),
            vertex: wgpu::VertexState {
                module: &resolve_shader,
                entry_point: Some("at_corner"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &resolve_shader,
                entry_point: Some("metered"),
                compilation_options: Default::default(),
                targets: &[Some(METERED.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet-resolve"),
            bind_group_layouts: &[Some(&resolve_layout), Some(&metered_layout)],
            immediate_size: 0,
        });
        let resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshlet-resolve"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &resolve_shader,
                entry_point: Some("at_corner"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: frame.depth,
                depth_write_enabled: Some(true),
                depth_compare: Some(frame.depth_compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &resolve_shader,
                entry_point: Some("lit"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: frame.colour,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let shadow_map = sun_shadow::tile(device);
        let matrices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet-matrices"),
            size: 128,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let resolve_group = resolve_group(
            device,
            &resolve_layout,
            &targets,
            &depth,
            &light,
            &shadow_map,
        )?;
        Ok(Some(Self {
            draws,
            formats,
            targets,
            depth,
            resolve,
            resolve_layout,
            resolve_group,
            meter,
            metered,
            metered_group,
            light,
            shadow_map,
            matrices,
            casts: Default::default(),
            rest: Vec::new(),
            refused,
        }))
    }

    /// Make the targets anew for a frame of `size`.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (targets, depth) = targets_of(device, &self.formats, size);
        if let Ok(group) = resolve_group(
            device,
            &self.resolve_layout,
            &targets,
            &depth,
            &self.light,
            &self.shadow_map,
        ) {
            self.resolve_group = group;
            self.targets = targets;
            self.depth = depth;
        }
    }

    /// How many surfaces draw.
    pub fn drawn(&self) -> usize {
        self.draws.len()
    }

    /// The texture of target `index`, for the bytes the pixel programs wrote.
    pub fn target(&self, index: usize) -> Option<&wgpu::Texture> {
        Some(self.targets.get(index)?.as_ref()?.texture())
    }

    /// Write this frame's view and what the last draw lights by.
    pub fn update(&self, queue: &wgpu::Queue, lighting: &Lighting) {
        let matrix = lighting.view_proj.to_cols_array();
        for (buffer, at) in self.draws.iter().flat_map(|draw| &draw.views) {
            queue.write_buffer(buffer, *at as u64, bytemuck::cast_slice(&matrix));
        }
        // A buffer is copied to at a multiple of four bytes: a surface that reads the view
        // elsewhere casts nothing, and nor does the model it is of.
        let aligned = (self.draws.iter().flat_map(|draw| &draw.views)).all(|(_, at)| at % 4 == 0);
        let cast = sun_shadow::cast_of(&lighting.sky).filter(|_| aligned);
        if let Some(to_tile) = cast {
            let both = [matrix, to_tile.to_cols_array()];
            queue.write_buffer(&self.matrices, 0, bytemuck::cast_slice(&both));
        }
        (self.casts).store(cast.is_some(), std::sync::atomic::Ordering::Relaxed);
        queue.write_buffer(&self.light, 0, &light_bytes(lighting, cast.is_some()));
    }

    /// Draw the surfaces as the light sees them into the tile: the light's matrix is copied to
    /// where each reads the view, and the camera's back once the tile is drawn.
    fn record_shadow(&self, encoder: &mut wgpu::CommandEncoder) {
        let views = || self.draws.iter().flat_map(|draw| &draw.views);
        for (buffer, at) in views() {
            encoder.copy_buffer_to_buffer(&self.matrices, 64, buffer, *at as u64, 64);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet-shadow"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.shadow_map,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for draw in &self.draws {
            pass.set_pipeline(&draw.caster);
            for (group, bound) in &draw.groups {
                pass.set_bind_group(*group, bound, &[]);
            }
            pass.draw(0..VERTICES, 0..draw.meshlets);
        }
        drop(pass);
        for (buffer, at) in views() {
            encoder.copy_buffer_to_buffer(&self.matrices, 0, buffer, *at as u64, 64);
        }
    }

    /// Draw the surfaces into the targets, in a pass of their own.
    pub fn record_targets(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.casts.load(std::sync::atomic::Ordering::Relaxed) {
            self.record_shadow(encoder);
        }
        let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = (self.targets.iter())
            .map(|target| {
                target.as_ref().map(|view| wgpu::RenderPassColorAttachment {
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
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet-pass"),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for draw in &self.draws {
            pass.set_pipeline(&draw.pipeline);
            for (group, bound) in &draw.groups {
                pass.set_bind_group(*group, bound, &[]);
            }
            pass.draw(0..VERTICES, 0..draw.meshlets);
        }
        drop(pass);
        // What the surfaces meter to, before the frame is lit by it.
        let mut meter = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet-meter"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.metered,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        meter.set_pipeline(&self.meter);
        meter.set_bind_group(0, &self.resolve_group, &[]);
        meter.draw(0..3, 0..1);
    }

    /// Light what the targets hold onto the frame `pass` draws, with its depth.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.resolve);
        pass.set_bind_group(0, &self.resolve_group, &[]);
        pass.set_bind_group(1, &self.metered_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

mod resolve;
mod scene;
mod sun_shadow;
mod tone;
use resolve::RESOLVE;
pub use scene::{falls_on, nine_of, Clouds, Dome, Scene};
pub use sun_shadow::to_tile;
pub use tone::{metered, Curve, Filmic, Tone};

#[cfg(test)]
mod far_tests;
#[cfg(test)]
mod scene_tests;
#[cfg(test)]
mod tests;
