//! A GTA VI model's shading pass: its mesh program, lowered to a vertex stage, reads the model's
//! own data blocks, its pixel program writes the targets the game's deferred pass writes, and a
//! last draw lights them onto the frame.

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

/// The most vertices an instance of a lowered mesh program is drawn with: three corners for
/// each of the 128 threads of a group.
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
/// wrote, edited to store a value, and `over=FILE` the same for a forward pass's pixel stage
/// only; `record=FILE` draws with the bytes of FILE as the record a
/// material's numbers head, the rest of which only the game fills; `plain-device` asks the
/// device for no optional feature and no raised limit, as a device that has none; `unguarded`
/// has the binder let through a stage whose loop counts to a number only the game fills, and
/// no pass is then built but on a software adapter; `meter=frame` has a frame lit by an
/// install's cycles metered whole, as the game meters one, and not by the model's surfaces alone.
/// `traced` and `traced=FILE` have the sun's mask written by the game's walk, which
/// [`super::trace::traced_aid`] reads.
#[derive(Debug, Default, PartialEq)]
struct Aids {
    target: f32,
    pixel: Option<String>,
    over: Option<String>,
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
                Some(("over", file)) => aids.over = Some(file.to_owned()),
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
    // Where a backend keeps what its driver compiled of a pipeline, the passes are built with it;
    // Direct3D 12 is handed the stages `kept_dx12` compiled.
    // A lit forward pass blends the two colours its pixel stage exports into one target.
    let kept = features
        & (wgpu::Features::PIPELINE_CACHE
            | super::kept_dx12::FEATURES
            | wgpu::Features::DUAL_SOURCE_BLENDING);
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
    /// What the frame writes of the light it is lit by, each time that light moves.
    Frame(FrameFeed),
    /// A volume texture, bound where the stage declares a three-dimensional one.
    Volume(Arc<Volume>),
}

/// A volume texture's texels: slice after slice, each row after row, of one level.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    /// Whether a texel is four half floats, as the file stores them; else four bytes of linear colour.
    pub half: bool,
    pub texels: Vec<u8>,
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
    /// The primitives of the largest of them: an instance runs three vertices for each.
    // The thread of a vertex is its primitive's, and a meshlet makes none on a thread past its own.
    pub primitives: u32,
    /// How it is laid over the lit frame where it is a forward surface; `None` for a deferred one.
    // A pass is built once for its two stages and the way they are laid on.
    pub forward: Option<Forward>,
    /// Whether a pass into the targets draws both faces: a terrain cell, whose own program winds
    /// the faces turned to the eye clockwise on the screen.
    pub two_sided: bool,
}

/// A buffer the frame writes the view into, and where.
type ViewWrite = (wgpu::Buffer, usize);

struct Draw {
    /// Its surface's place among those the scene was prepared from: the draws are in that order.
    at: usize,
    pipeline: wgpu::RenderPipeline,
    caster: wgpu::RenderPipeline,
    /// A forward surface's pass into what it lets through of the frame behind it.
    through: Option<wgpu::RenderPipeline>,
    /// By group number.
    groups: Vec<(u32, wgpu::BindGroup)>,
    meshlets: u32,
    /// The vertices of an instance.
    vertices: u32,
    views: Vec<ViewWrite>,
    /// The pass it draws by, and for each of `groups` the digest of the feeds it was bound by and
    /// the views it holds: a map that comes in late binds again only the groups that read it.
    built: Arc<Built>,
    bound_by: Vec<(u64, Vec<ViewWrite>)>,
}

/// What a frame of a model asked of the device, counted where each call is recorded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshletCalls {
    /// The render passes begun: the viewport's own, which the frame is lit in, is not one.
    pub passes: u32,
    /// Every draw, the full-screen ones among them.
    pub draws: u32,
    /// The draws of a surface, into the shadow's tile or into the targets.
    pub surface_draws: u32,
    /// The bind groups set.
    pub binds: u32,
    /// The queue's writes, and the bytes of them.
    pub writes: u32,
    pub written: u64,
    /// The copies from a buffer to a buffer.
    pub copies: u32,
    /// Vertices times instances, over the draws of a surface.
    pub vertices: u64,
}

/// What a recording has to draw again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Stale {
    /// The shadow's tile, which holds the surfaces as the light's matrix places them.
    shadow: bool,
    /// The targets, which hold them as the view matrix does.
    targets: bool,
    /// The mask and the meter, which read the tile, the targets and the light.
    lit: bool,
}

/// What a frame need not ask of the device again: the bytes its buffers hold already, and what
/// the tile and the targets were last drawn by. A scene's feeds are bound once and never
/// written after, so these three are all a pass of its surfaces reads that moves.
#[derive(Default)]
struct Kept {
    /// The view matrix the surfaces' buffers hold.
    view: Option<[u8; 64]>,
    /// The light's matrix the tile was last cast by.
    tile: Option<[u8; 64]>,
    /// What the light's buffer holds.
    light: Option<Box<[u8; LIGHT_BYTES]>>,
    /// What is owed until a recording draws it.
    stale: Stale,
    /// Whether the frame last taken in casts a shadow.
    casts: bool,
}

/// What a frame writes of the bytes it is drawn by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Writes {
    /// The view matrix, into each buffer of a surface that holds one.
    view: bool,
    /// The camera's matrix and the light's, which the tile's pass copies to those buffers.
    matrices: bool,
    /// What the frame is lit by.
    light: bool,
}

impl Kept {
    /// Take in the bytes a frame is drawn by: what of them the device lacks. What they make
    /// stale is owed from here on.
    fn take(
        &mut self,
        view: [u8; 64],
        tile: Option<[u8; 64]>,
        light: &[u8; LIGHT_BYTES],
    ) -> Writes {
        let moved = self.view != Some(view);
        // A frame that casts nothing leaves the tile as it was cast last.
        let cast = tile.is_some() && tile != self.tile;
        let relit = self.light.as_deref() != Some(light);
        self.view = Some(view);
        if cast {
            self.tile = tile;
        }
        if relit {
            self.light = Some(Box::new(*light));
        }
        self.casts = tile.is_some();
        self.stale.shadow |= cast;
        self.stale.targets |= moved;
        self.stale.lit |= moved || cast || relit;
        Writes {
            view: moved,
            // The tile's pass copies the camera's matrix back, so it is handed this frame's.
            matrices: self.casts && self.stale.shadow,
            light: relit,
        }
    }

    /// What a recording draws of what is owed, which is owed no longer.
    fn draws(&mut self) -> Stale {
        let mut now = std::mem::take(&mut self.stale);
        // A tile no frame cast into stays owed to the next one that casts.
        if !self.casts {
            self.stale.shadow = std::mem::take(&mut now.shadow);
        }
        now
    }
}

/// The surfaces of a model ready to draw, with the targets their pixel programs write.
pub struct MeshletScene {
    draws: Vec<Draw>,
    /// What the draws are bound to, each made once for every surface that binds it alike.
    made: shared::Made,
    kept: std::cell::RefCell<Kept>,
    /// What the frame last begun by [`MeshletScene::update`] has asked of the device so far.
    calls: std::cell::Cell<MeshletCalls>,
    /// Whether every surface is drawn with [`VERTICES`] and not with those fitted to it.
    at_most: std::cell::Cell<bool>,
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
    /// The screen's mask of the directional light, which the frame is lit through, and the pass
    /// that fills it from the shadow's tile.
    mask: SunMask,
    mask_layout: wgpu::BindGroupLayout,
    mask_writer: wgpu::RenderPipeline,
    /// The mask's second writer, the game's walk over the model's arena, where one is set.
    traced: std::cell::RefCell<Option<super::trace::TracedMask>>,
    /// The tile the sun's shadow is cast into, and the camera's matrix and the light's, which a
    /// frame copies in turn to where the surfaces read the view.
    shadow_map: wgpu::TextureView,
    matrices: wgpu::Buffer,
    /// Its forward surfaces and the lit frame they are laid over, where it has any: the frame is
    /// then lit into that, and toned from it.
    over: Option<forward::Over>,
    /// The meshes of the model its programs do not draw, which the frame draws as a material.
    pub rest: Vec<usize>,
    /// The surfaces it was prepared from that the device has no pass for, each by its place
    /// among them, and why.
    pub refused: Vec<(usize, String)>,
    /// Those whose pass a worker still builds, each by its place among them.
    // The caller draws them as it draws `rest`, and has [`MeshletScene::build_on`] take on those
    // a worker has built the pass of since.
    pub waiting: Vec<usize>,
}

/// The texels of a row no image fills: one row of a palette, which is as wide.
pub const ROW: u32 = 256;

/// The texels of a texture of `rgba` and how many levels it has: with `colour`, every level below
/// the first after it, each the average of four of the one above.
fn with_levels(width: u32, height: u32, rgba: &[u8], srgb: bool, colour: bool) -> (Vec<u8>, u32) {
    let levels = match colour {
        true => super::mips::chain(width, height, rgba, srgb),
        false => Vec::new(),
    };
    let mut texels = rgba.to_vec();
    for (_, _, level) in &levels {
        texels.extend_from_slice(level);
    }
    (texels, 1 + levels.len() as u32)
}

/// A texture of one white texel a side, or of `image` where the view is flat, seen as `dimension`,
/// with the levels below its first where it is an image of colour.
// ⚠ A texture repeated many times across a surface (a terrain layer, 128 times a cell) is read
// one texel in many at a distance: with no levels below, each pixel lands on a texel of its own.
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
    let colour = image.is_some() && flat && matches!(sample, wgpu::TextureSampleType::Float { .. });
    let (texels, levels) = with_levels(width, height, rgba, srgb, colour);
    let descriptor = wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: levels,
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
            &texels.repeat(layers as usize),
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
    made: &mut shared::Made,
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
        let framed = forward_frame::bound(
            device,
            feed,
            resource,
            *count,
            made,
            &mut buffers,
            &mut textures,
        );
        if let Some(what) = framed {
            bound.push((*binding, what));
            continue;
        }
        let what = match resource {
            Resource::Uniform { size } => {
                let mut held = bytes.to_vec();
                held.resize((*size as usize).max(LEAST), 0);
                // A frame writes the view into its own buffer of each surface.
                let shared = !matches!(feed, Some(Feed::View { .. }));
                let usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
                let buffer = made.buffer(device, usage, &held, shared);
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
            Resource::Storage { size, read_only } => {
                let mut held = bytes.to_vec();
                let least = (*size as usize).max(LEAST).max(held.len());
                held.resize(least.next_multiple_of(4), 0);
                // A buffer a stage may write is its surface's own.
                let usage = wgpu::BufferUsages::STORAGE;
                buffers.push(made.buffer(device, usage, &held, *read_only));
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
                if let (Some(Feed::Volume(volume)), wgpu::TextureViewDimension::D3) =
                    (feed, dimension)
                {
                    textures.push(made.volume(device, queue, volume));
                    bound.push((*binding, Bound::View(first)));
                    continue;
                }
                let sliced = count.is_none() && *dimension == wgpu::TextureViewDimension::D2Array;
                if let (true, Some(view)) = (sliced, made.slices(device, queue, images)) {
                    textures.push(view);
                    bound.push((*binding, Bound::View(first)));
                    continue;
                }
                // The entries no image fills share one white texel, and an image bound twice
                // is one texture: the scene makes each once.
                for entry in 0..entries {
                    let array = count.map(|_| entries);
                    let image = image_at(images, entry, array)
                        .and(images.get(entry).and_then(Option::as_ref));
                    textures.push(made.view(device, queue, *dimension, *sample, image, empty));
                }
                match count {
                    Some(_) => Bound::Views(first, textures.len()),
                    None => Bound::View(first),
                }
            }
            Resource::Sampler { comparison } => {
                samplers.push(made.sampler(device, *comparison));
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
    made.uploads.groups += 1;
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
    /// A forward pass's own into what its surface lets through of the frame behind it.
    through: Option<wgpu::RenderPipeline>,
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
/// A pass by the places of its two stages, for a forward one how it is laid on, and whether it
/// draws both faces.
type PassKey = (
    usize,
    usize,
    Option<(wgpu::BlendState, wgpu::ColorWrites, Option<wgpu::Face>)>,
    bool,
);

#[derive(Default)]
pub struct MeshletPipelines {
    built: std::collections::HashMap<PassKey, Slot>,
    /// The folder what the driver compiled is kept in, and that once it was looked for.
    folder: Option<PathBuf>,
    compiled: Option<Option<Compiled>>,
    /// The workers that build the passes, made with the first of them.
    workers: Option<or_job::Pool>,
    /// What a worker runs before it builds a pass, where a test watches the builds.
    #[cfg(test)]
    before: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// The workers that build passes, `limit` at once and no more: a compiler holds hundreds of
/// megabytes a stage. Each time the last of them finds no pass left to build, what `compiled`
/// gives of the driver's work is kept in `file`: once a batch, where a save after each pass
/// writes the whole of it as many times as there are passes.
fn pass_workers(
    limit: usize,
    file: Option<PathBuf>,
    compiled: impl Fn() -> Option<Vec<u8>> + Send + Sync + 'static,
) -> or_job::Pool {
    or_job::Pool::new(limit, BUILD_STACK, move || {
        if let Some(file) = &file {
            keep(file, compiled());
        }
    })
}

/// Keep `data`, what a driver compiled of the passes, in `file` for the next session.
fn keep(file: &std::path::Path, data: Option<Vec<u8>>) {
    // ⚠ One writer at a time in the process: the file beside it is named by the process alone.
    static KEEPING: Mutex<()> = Mutex::new(());
    let Some(data) = data else {
        return;
    };
    let _one = KEEPING.lock().unwrap_or_else(|p| p.into_inner());
    // Beside its place, then moved there: a session that reads it never sees half a file.
    let partial = file.with_extension(format!("{}.part", std::process::id()));
    let kept = (file.parent().map_or(Ok(()), std::fs::create_dir_all))
        .and_then(|()| std::fs::write(&partial, &data))
        .and_then(|()| std::fs::rename(&partial, file));
    match kept {
        Ok(()) => {
            tracing::info!(
                bytes = data.len(),
                "the passes a driver compiled were kept for the next session"
            );
        }
        Err(error) => {
            tracing::debug!(%error, "the compiled passes are not kept for the next session");
        }
    }
}

impl MeshletPipelines {
    /// Passes whose compiled form is kept in a file of `folder` between sessions, on a backend
    /// that has one to keep.
    pub fn kept_in(folder: PathBuf) -> Self {
        super::kept_dx12::keep_in(folder.join("dx12"));
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

    /// Take `surface`'s pass as one a worker still builds, or with `held` false let the next ask start one.
    // How a test, of this crate or of one that asks for `test-support`, sees a frame drawn while
    // a pass builds: no worker is started for a pass that is held.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold(&mut self, surface: &MeshletSurface, held: bool) {
        let key = Self::key(surface);
        match held {
            true => self.built.insert(key, Slot::default()),
            false => self.built.remove(&key),
        };
    }

    fn key(surface: &MeshletSurface) -> PassKey {
        (
            Arc::as_ptr(&surface.vertex) as usize,
            Arc::as_ptr(&surface.fragment) as usize,
            surface.forward.as_ref().map(Forward::state),
            surface.two_sided,
        )
    }

    /// The pass of `surface`, which a worker starts to build where none has: `None` until it
    /// is built, `Err` where the device has none for it.
    fn of(
        &mut self,
        device: &wgpu::Device,
        surface: &MeshletSurface,
    ) -> Result<Option<Arc<Built>>, String> {
        let key = Self::key(surface);
        if !self.built.contains_key(&key) {
            let folder = self.folder.as_deref();
            let compiled = (self.compiled)
                .get_or_insert_with(|| folder.and_then(|folder| compiled_in(device, folder)))
                .clone();
            let slot = Slot::default();
            let (theirs, device) = (Arc::clone(&slot), device.clone());
            let stages = (Arc::clone(&surface.vertex), Arc::clone(&surface.fragment));
            let (forward, two_sided) = (surface.forward, surface.two_sided);
            // No more at once than the machine spares.
            let limit = or_job::workers();
            let workers = self.workers.get_or_insert_with(|| match compiled.clone() {
                Some((cache, file)) => pass_workers(limit, Some(file), move || cache.get_data()),
                None => pass_workers(limit, None, || None),
            });
            #[cfg(test)]
            let before = self.before.clone();
            let work = move || {
                #[cfg(test)]
                if let Some(before) = &before {
                    before();
                }
                let cache = compiled.as_ref().map(|(cache, _)| &**cache);
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    build_checked(&device, stages, cache, forward, two_sided)
                }))
                .unwrap_or_else(|_| Err("building the pass stopped on an internal error".into()));
                *theirs.lock().unwrap_or_else(|p| p.into_inner()) = Some(built.map(Arc::new));
            };
            (workers.run(work)).map_err(|e| format!("no worker to build the pass on: {e}"))?;
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

/// [`build`], with what the device refuses of it as the reason.
// A scope is its thread's: nothing a frame draws meanwhile is taken for the pass's.
fn build_checked(
    device: &wgpu::Device,
    stages: (Arc<naga::Module>, Arc<naga::Module>),
    compiled: Option<&wgpu::PipelineCache>,
    forward: Option<Forward>,
    two_sided: bool,
) -> Result<Built, String> {
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let started = std::time::Instant::now();
    let built = build(device, stages.clone(), compiled, forward, two_sided);
    let refused = pollster::block_on(validation.pop()).or(pollster::block_on(internal.pop()));
    tracing::info!(elapsed = ?started.elapsed(), refused = refused.is_some(), "a GTA VI pass was built for the device");
    if super::kept_dx12::forget_handed(refused.is_some()) {
        return super::kept_dx12::without(|| {
            build_checked(device, stages, compiled, forward, two_sided)
        });
    }
    if let Some(error) = refused {
        return Err(error.to_string());
    }
    built
}

/// The mesh stage and the pixel stage a pass of `stages`, `forward` or not, is built from on the adapter `info`:
/// each as it is, or the one a developer's aid of `aids` puts in its place. `Err` says why no
/// pass of theirs is built there.
fn stages_drawn(
    info: &wgpu::AdapterInfo,
    stages: &(Arc<naga::Module>, Arc<naga::Module>),
    aids: &Aids,
    forward: bool,
) -> Result<(naga::Module, naga::Module), String> {
    // ⚠ A loop the guard would have refused may never end, and a hardware adapter that waits on one is lost.
    if aids.unguarded && info.device_type != wgpu::DeviceType::Cpu {
        return Err("an unguarded stage is drawn on a software adapter only".to_string());
    }
    let mut fragment = match (&aids.pixel, aids.over.as_ref().filter(|_| forward)) {
        (_, Some(file)) | (Some(file), None) => stage_of(file)?,
        (None, None) => (*stages.1).clone(),
    };
    super::pipeline::position_w_as_direct3d(&mut fragment, info.backend);
    let vertex = match &aids.mesh {
        Some(file) => stage_of(file)?,
        None => (*stages.0).clone(),
    };
    if let Some(why) = crate::instance::refused_by_driver(info, &vertex) {
        return Err(why);
    }
    Ok((vertex, fragment))
}

/// The faces a pass culls: a forward pass's own; a pass into the targets the back faces, as the
/// game's draw lists do, unless it is `two_sided` or its pixel stage reads which face it is on.
// ⚠ A model's card seen from both sides is two coincident faces wound apart: drawn with neither
// culled, under `GreaterEqual` the face turned away wins the tie and is lit from behind.
// ponytail: a placement that mirrors (negative determinant) winds its faces the other way; such an
// instance would need the front face turned per draw.
fn culled(
    fragment: &naga::Module,
    forward: Option<&Forward>,
    two_sided: bool,
) -> Option<wgpu::Face> {
    let facing = naga::Binding::BuiltIn(naga::BuiltIn::FrontFacing);
    let reads_facing = (fragment.entry_points.iter())
        .flat_map(|entry| &entry.function.arguments)
        .any(|argument| argument.binding.as_ref() == Some(&facing));
    match forward {
        Some(forward) => forward.cull,
        None if reads_facing || two_sided => None,
        None => Some(wgpu::Face::Back),
    }
}

/// The pass of the two `stages` built for `device`: into the targets, or with `forward` over the
/// lit frame, tested against the targets' depth and not writing it.
fn build(
    device: &wgpu::Device,
    stages: (Arc<naga::Module>, Arc<naga::Module>),
    compiled: Option<&wgpu::PipelineCache>,
    forward: Option<Forward>,
    two_sided: bool,
) -> Result<Built, String> {
    // The two stages come linked: the vertex stage gives the locations the pixel stage takes.
    let (mut vertex, mut fragment) =
        stages_drawn(&device.adapter_info(), &stages, aids(), forward.is_some())?;
    if forward.as_ref().is_some_and(Forward::dual) {
        forward::dual_sourced(&mut fragment);
    }
    if device.adapter_info().backend == wgpu::Backend::Dx12 {
        super::kept_dx12::typed_literals(&mut vertex);
        super::kept_dx12::typed_literals(&mut fragment);
    }
    let mut targets = match &forward {
        Some(forward) => vec![Some(forward::target(forward))],
        None => formats_of(&fragment)?,
    };
    // The first target holds linear light, which eight bits band in the darks: it is kept
    // encoded, as a colour target is, and read back as the light the program wrote.
    if let (None, Some(Some(colour))) = (&forward, targets.first_mut()) {
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
    let shader =
        |module: &naga::Module| super::kept_dx12::shader(device, module, (&vertex, &fragment));
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
    let fragment_module = shader(&fragment);
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("meshlet-pass"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vertex_module,
            entry_point: Some(&vertex_entry.name),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            front_face: forward::FRONT,
            cull_mode: culled(&fragment, forward.as_ref(), two_sided),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(forward.is_none()),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fragment_module,
            entry_point: Some(&fragment_entry.name),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: compiled,
    });
    let through = forward.as_ref().and_then(|forward| {
        forward::through_pipeline(
            device,
            &layout,
            (&vertex_module, vertex_entry.name.as_str()),
            (&fragment_module, fragment_entry.name.as_str()),
            forward,
            compiled,
        )
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
        through,
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
    mask: &SunMask,
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
                binding: 7,
                resource: view(&mask.roots),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: view(&mask.tiles),
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
const LIGHT_BYTES: usize = 912 + 16 * env_fit::SIDE * env_fit::SIDE;
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
    let (fits, _) = words[228..].as_chunks_mut::<4>();
    for (held, fit) in fits.iter_mut().zip(env_fit::table()) {
        held[..3].copy_from_slice(fit);
    }
    bytemuck::cast(words)
}

/// Why no scene draws two surfaces whose passes do not write the same targets.
const SHAPES: &str = "two pixel programs write targets of different shapes";

/// What draws `surface`, the one at `at`, by its pass `built`: each group bound to its feeds.
fn draw_of(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    at: usize,
    surface: &MeshletSurface,
    built: &Arc<Built>,
    made: &mut shared::Made,
) -> Result<Draw, String> {
    let mut draw = Draw {
        at,
        pipeline: built.pipeline.clone(),
        caster: built.caster.clone(),
        through: built.through.clone(),
        groups: Vec::new(),
        meshlets: surface.meshlets,
        vertices: 3 * surface.primitives.min(VERTICES / 3),
        views: Vec::new(),
        built: Arc::clone(built),
        bound_by: Vec::new(),
    };
    for (group, layout) in (0..).zip(&built.layouts) {
        let Some(layout) = layout else {
            continue;
        };
        let module = match group % 2 {
            0 => &built.vertex,
            _ => &built.fragment,
        };
        let bound = bind(device, queue, module, group, layout, &surface.feeds, made)?;
        let digest = shared::digest(&surface.feeds, group);
        draw.bound_by.push((digest, bound.views.clone()));
        draw.groups.push((group, bound.group));
        draw.views.extend(bound.views);
    }
    Ok(draw)
}

impl MeshletScene {
    /// Prepare `surfaces` for targets of `size`, lit onto a frame of `frame`'s formats.
    /// `None` while no pass of theirs is built and a worker builds one: the caller asks again.
    /// `Err` says why the scene is not one to draw; what the device refuses of the groups is
    /// reported through its own error scopes.
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
        // surface whose pass the device has none for, or none yet, is left to the caller; the
        // others draw.
        let (mut passes, mut waiting, mut refused) = (Vec::new(), Vec::new(), Vec::new());
        for (at, surface) in surfaces.iter().enumerate() {
            match pipelines.of(device, surface) {
                Ok(Some(pass)) => passes.push((at, surface, pass)),
                Ok(None) => waiting.push(at),
                Err(why) => refused.push((at, why)),
            }
        }
        // The forward surfaces are laid over what the others light.
        let (laid, passes): (Vec<_>, Vec<_>) =
            (passes.into_iter()).partition(|(_, surface, _)| surface.forward.is_some());
        if passes.is_empty() {
            return match (waiting.is_empty(), refused.is_empty()) {
                (false, _) => Ok(None),
                (true, false) => Err(refused.swap_remove(0).1),
                (true, true) => Err(forward::ALONE.to_string()),
            };
        }
        let formats = passes[0].2.formats.clone();
        let mut draws = Vec::with_capacity(surfaces.len());
        let mut made = shared::Made::default();
        for (at, surface, built) in passes {
            if built.formats != formats {
                return Err(SHAPES.to_string());
            }
            draws.push(draw_of(device, queue, at, surface, &built, &mut made)?);
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
                texture(7, unfiltered),
                texture(8, wgpu::TextureSampleType::Uint),
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
        let mask_layout = sun_mask::layout(device);
        let mask_writer = sun_mask::writer(device, &mask_layout, &resolve_shader);
        let mask = SunMask::new(
            device,
            &mask_layout,
            size,
            (targets.as_slice(), &depth),
            &light,
            &shadow_map,
        )?;
        let resolve_group =
            resolve_group(device, &resolve_layout, &targets, &depth, &light, &mask)?;
        // Lit into a frame of its own from the first where any surface is a forward one, those
        // whose pass a worker still builds among them.
        let over = match surfaces.iter().any(|surface| surface.forward.is_some()) {
            true => Some(forward::Over::new(
                device,
                queue,
                forward::Layouts {
                    resolve: &resolve_layout,
                    metered: &metered_layout,
                },
                frame,
                size,
                &laid,
                &mut made,
            )?),
            false => None,
        };
        Ok(Some(Self {
            draws,
            made,
            over,
            kept: Default::default(),
            calls: Default::default(),
            at_most: Default::default(),
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
            mask,
            mask_layout,
            mask_writer,
            traced: Default::default(),
            shadow_map,
            matrices,
            rest: Vec::new(),
            refused,
            waiting,
        }))
    }

    /// Draw as well each surface waited for whose pass a worker has built since; whether fewer wait.
    // `surfaces` are those the scene was prepared from. A surface taken on is bound alone, where a
    // prepare makes every buffer and texture of every surface anew, and is drawn in its place
    // among the others, so that the scene ends as one prepared with every pass built.
    pub fn build_on(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &mut MeshletPipelines,
        surfaces: &[MeshletSurface],
    ) -> Result<bool, String> {
        let waited = std::mem::take(&mut self.waiting);
        for at in &waited {
            let Some(surface) = surfaces.get(*at) else {
                continue;
            };
            match pipelines.of(device, surface) {
                Ok(Some(built)) if surface.forward.is_some() => match &mut self.over {
                    Some(over) => {
                        over.take_on(device, queue, *at, surface, &built, &mut self.made)?
                    }
                    None => self.refused.push((*at, forward::ALONE.to_string())),
                },
                Ok(Some(built)) if built.formats != self.formats => return Err(SHAPES.to_string()),
                Ok(Some(built)) => {
                    let draw = draw_of(device, queue, *at, surface, &built, &mut self.made)?;
                    let place = self.draws.partition_point(|drawn| drawn.at < *at);
                    self.draws.insert(place, draw);
                }
                Ok(None) => self.waiting.push(*at),
                Err(why) => self.refused.push((*at, why)),
            }
        }
        let fewer = self.waiting.len() < waited.len();
        if fewer {
            self.refused.sort_by_key(|(at, _)| *at);
            // The buffers of a surface taken on hold no view yet, and the targets lack it.
            self.forget();
        }
        Ok(fewer)
    }

    /// Make the targets anew for a frame of `size`.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (targets, depth) = targets_of(device, &self.formats, size);
        let Ok(mask) = SunMask::new(
            device,
            &self.mask_layout,
            size,
            (targets.as_slice(), &depth),
            &self.light,
            &self.shadow_map,
        ) else {
            return;
        };
        let lit_by = &self.resolve_layout;
        if let Ok(group) = resolve_group(device, lit_by, &targets, &depth, &self.light, &mask) {
            self.resolve_group = group;
            self.targets = targets;
            self.depth = depth;
            self.mask = mask;
            if let Some(over) = &mut self.over {
                over.resize(device, size);
            }
            if let Some(traced) = self.traced.get_mut() {
                traced.resized();
            }
            self.forget();
        }
    }

    /// Forget what was drawn and written: the next frame asks the device for all of it again.
    // ⚠ Whatever writes a buffer or a texture of the scene's behind its back calls this, a
    // writer of the mask that is no pass of the scene's among them: the meter reads the mask.
    pub fn forget(&self) {
        self.kept.take();
    }

    /// Draw every surface with the most vertices an instance has, or with `most` false as fitted to it.
    // What a bench holds the fitted draw against: the targets hold the same bytes either way
    // unless a mesh program makes a primitive on a thread past those its record states.
    pub fn draw_at_most(&self, most: bool) {
        self.at_most.set(most);
        self.forget();
    }

    /// How many surfaces draw.
    pub fn drawn(&self) -> usize {
        self.draws.len() + self.drawn_forward()
    }

    /// How many of them draw by a forward pass, laid over the lit frame.
    pub fn drawn_forward(&self) -> usize {
        self.over.as_ref().map_or(0, forward::Over::drawn)
    }

    /// The meshlets of the surfaces that draw.
    pub fn meshlets(&self) -> u32 {
        self.draws.iter().map(|draw| draw.meshlets).sum()
    }

    /// The meshlets of each surface that draws, and the vertices one of them is drawn with.
    pub fn fitted(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (self.draws.iter()).map(|draw| (draw.meshlets, draw.vertices))
    }

    /// What the last frame asked of the device.
    pub fn calls(&self) -> MeshletCalls {
        self.calls.get()
    }

    fn count(&self, add: impl FnOnce(&mut MeshletCalls)) {
        let mut calls = self.calls.get();
        add(&mut calls);
        self.calls.set(calls);
    }

    fn write(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, at: u64, bytes: &[u8]) {
        queue.write_buffer(buffer, at, bytes);
        self.count(|calls| {
            calls.writes += 1;
            calls.written += bytes.len() as u64;
        });
    }

    /// Draw every surface into `pass`: by its caster for the shadow's tile, else by its own pass.
    fn draw_surfaces(&self, pass: &mut wgpu::RenderPass<'_>, cast: bool) {
        let (mut binds, mut vertices) = (0, 0);
        for draw in &self.draws {
            pass.set_pipeline(if cast { &draw.caster } else { &draw.pipeline });
            for (group, bound) in &draw.groups {
                pass.set_bind_group(*group, bound, &[]);
                binds += 1;
            }
            // One range for the draw and for the count of it: the two never part.
            let fitted = match self.at_most.get() {
                true => VERTICES,
                false => draw.vertices,
            };
            let (corners, meshlets) = (0..fitted, 0..draw.meshlets);
            vertices += u64::from(corners.end) * u64::from(meshlets.end);
            pass.draw(corners, meshlets);
        }
        self.count(|calls| {
            calls.passes += 1;
            calls.draws += self.draws.len() as u32;
            calls.surface_draws += self.draws.len() as u32;
            calls.binds += binds;
            calls.vertices += vertices;
        });
    }

    /// Count a full-screen draw that set `binds` groups, in `passes` passes of its own.
    fn count_full_screen(&self, passes: u32, binds: u32) {
        self.count(|calls| {
            calls.passes += passes;
            calls.draws += 1;
            calls.binds += binds;
        });
    }

    /// The texture of target `index`, for the bytes the pixel programs wrote.
    pub fn target(&self, index: usize) -> Option<&wgpu::Texture> {
        Some(self.targets.get(index)?.as_ref()?.texture())
    }

    /// Whether the model casts no shadow whatever lights it, which a person is told.
    // A buffer is copied to at a multiple of four bytes: a surface that reads the view elsewhere
    // cannot be handed the light's, and a model is not cast by some of its surfaces alone.
    pub fn casts_no_shadow(&self) -> bool {
        (self.draws.iter().flat_map(|draw| &draw.views)).any(|(_, at)| at % 4 != 0)
    }

    /// Write what moved of this frame's view and light, and take in what a recording draws again for it.
    pub fn update(&self, queue: &wgpu::Queue, lighting: &Lighting) {
        self.calls.set(MeshletCalls::default());
        let cast = sun_shadow::cast_of(&lighting.sky).filter(|_| !self.casts_no_shadow());
        // Compared as bytes: a nought of the other sign is another matrix to a program.
        let bytes = |matrix: Mat4| -> [u8; 64] { bytemuck::cast(matrix.to_cols_array()) };
        let (view, tile) = (bytes(lighting.view_proj), cast.map(bytes));
        let light = light_bytes(lighting, cast.is_some());
        let writes = self.kept.borrow_mut().take(view, tile, &light);
        if writes.view {
            for (buffer, at) in self.draws.iter().flat_map(|draw| &draw.views) {
                self.write(queue, buffer, *at as u64, &view);
            }
            for (buffer, at) in self.over.iter().flat_map(|over| over.views()) {
                self.write(queue, buffer, *at as u64, &view);
            }
        }
        if let Some(over) = &self.over {
            over.update(lighting.camera);
        }
        if let (Some(tile), true) = (tile, writes.matrices) {
            self.write(
                queue,
                &self.matrices,
                0,
                bytemuck::cast_slice(&[view, tile]),
            );
        }
        if writes.light {
            self.write(queue, &self.light, 0, &light);
            let (writes, written) = self.made.frame.write(queue, &lighting.sky);
            self.count(|calls| {
                calls.writes += writes;
                calls.written += written;
            });
        }
        self.update_traced(queue, lighting);
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
        self.draw_surfaces(&mut pass, true);
        drop(pass);
        for (buffer, at) in views() {
            encoder.copy_buffer_to_buffer(&self.matrices, 0, buffer, *at as u64, 64);
        }
        self.count(|calls| calls.copies += 2 * views().count() as u32);
    }

    /// Draw what the last [`MeshletScene::update`] left stale, and tell `mark` where each stage ends.
    // The surfaces into the shadow's tile, the surfaces into the targets, then the mask and what
    // they meter to, each in a pass of its own; a stage is marked whether it drew or not.
    pub fn record_targets(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        mut mark: impl FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        let mut now = self.kept.borrow_mut().draws();
        if now.shadow {
            self.record_shadow(encoder);
        }
        mark(encoder, "model shadow");
        if now.targets {
            self.record_surfaces(encoder);
        }
        mark(encoder, "model targets");
        // The traced mask moves with no stale of its own: what reads it is drawn again after it.
        now.lit |= self.record_traced(encoder);
        if now.lit {
            self.record_lit(encoder);
        }
        mark(encoder, "model meter");
        if let Some(over) = &self.over {
            if forward::redraws(now) {
                over.record(self, encoder);
            }
            mark(encoder, "model forward");
        }
    }

    /// Draw the surfaces into the targets, in a pass of their own.
    fn record_surfaces(&self, encoder: &mut wgpu::CommandEncoder) {
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
        self.draw_surfaces(&mut pass, false);
    }

    /// Fill the mask from the tile and the targets, and meter what the surfaces come to.
    fn record_lit(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.traced_writes() && self.mask.record(encoder, &self.mask_writer) {
            self.count_full_screen(1, 1);
        }
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
        self.count_full_screen(1, 1);
    }

    /// Light what the targets hold onto the frame `pass` draws, with its depth.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(over) = &self.over {
            return over.show(self, pass);
        }
        pass.set_pipeline(&self.resolve);
        pass.set_bind_group(0, &self.resolve_group, &[]);
        pass.set_bind_group(1, &self.metered_group, &[]);
        pass.draw(0..3, 0..1);
        self.count_full_screen(0, 2);
    }
}

mod env_fit;
mod forward;
mod forward_frame;
mod resolve;
mod scene;
mod shared;
mod sun_mask;
mod sun_shadow;
mod tone;
mod traced;
pub use forward::Forward;
pub use forward_frame::FrameFeed;
use resolve::RESOLVE;
pub use scene::{falls_on, nine_of, Clouds, Dome, Scene};
pub use shared::Uploads;
use sun_mask::SunMask;
pub use sun_shadow::to_tile;
pub use tone::{metered, Curve, Filmic, Tone};
pub use traced::light_for;
pub(crate) use traced::resolve_program;

#[cfg(test)]
mod calls_tests;
#[cfg(test)]
mod far_tests;
#[cfg(test)]
mod mask_tests;
#[cfg(test)]
mod scene_tests;
#[cfg(test)]
mod skin_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod traced_tests;
#[cfg(test)]
mod translucency_tests;
