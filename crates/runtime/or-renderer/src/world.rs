//! The world a viewport draws: meshes on the device, each drawn once for every instance that places it, in the bucket of its material.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Weak};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::game_shaders::deferred::Sun;
use crate::game_shaders::engine::EngineConstants;
use crate::game_shaders::mesh::scene::{MATERIAL_DEFAULT, MATERIAL_TREE};
use crate::game_shaders::shadow::TILE;
use crate::game_shaders::stand_in::constant_view;
use crate::viewport::{MeshVertex, OrbitCamera, ViewportRenderer};

pub mod cards;
mod lamps;
pub mod light_math;
mod material;
mod pipelines;
mod shader;
pub(crate) mod shadow;
mod shadow_terrain;
pub mod water;
mod water_shader;

pub use lamps::{WorldLamp, WORLD_LAMPS};
pub use material::{
    colour1, coloured, interior, slot, WorldConstants, WorldFamily, WorldSurface, WORLD_MAPS,
};
use material::{uniform_bytes, STAND_INS};
use pipelines::{ByBucket, Pipelines};
pub use shadow::{shadow_reach, ShadowFocus, DISTANCE_MULTIPLIER, SHADOW_FAR};
use shadow::{Cascades, CASCADES};
pub use shadow_terrain::TerrainGrid;

/// How and when a mesh of the world is drawn; declared in the order the buckets draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorldBucket {
    /// Every texel, into the G-buffer of the game's frame.
    #[default]
    Opaque,
    /// As opaque, but for the texels whose coverage is under a half.
    Cutout,
    /// As opaque, and giving light of its own colour.
    Emissive,
    /// Blended by its coverage onto the surface it lies on, which it is held in front of.
    Decal,
    /// Blended by its coverage over the lit frame.
    Blend,
}

impl WorldBucket {
    pub(crate) const COUNT: usize = 5;

    /// Whether it casts the sun's shadow: what blends lets the sun through, and a decal lies on what casts already.
    fn casts(self) -> bool {
        matches!(self, Self::Opaque | Self::Cutout | Self::Emissive)
    }
}

/// What a mesh of the world is drawn with: its maps and constants on the device and its bucket; a clone shares them.
#[derive(Clone)]
pub struct WorldMaterial {
    maps: Arc<wgpu::BindGroup>,
    bucket: WorldBucket,
    /// The material the G-buffer's stencil names it by, which picks the pass of the directional light that lights it.
    stencil: u32,
}

impl PartialEq for WorldMaterial {
    fn eq(&self, other: &Self) -> bool {
        self.bucket == other.bucket && Arc::ptr_eq(&self.maps, &other.maps)
    }
}

impl Eq for WorldMaterial {}

impl WorldMaterial {
    pub fn bucket(&self) -> WorldBucket {
        self.bucket
    }

    /// A number two materials have in common exactly when they draw with the same maps, to sort and group draws by.
    pub fn key(&self) -> usize {
        Arc::as_ptr(&self.maps) as usize
    }
}

/// One mesh of the world on the device: the vertices and indices it draws, and the material it draws them with.
pub struct WorldMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    base_vertex: i32,
    first_index: u32,
    index_count: u32,
    material: WorldMaterial,
}

impl WorldMesh {
    /// Upload a mesh of its own buffers, drawn with `material`.
    pub fn new(
        device: &wgpu::Device,
        vertices: &[MeshVertex],
        indices: &[u32],
        material: WorldMaterial,
    ) -> Self {
        let buffer = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        Self::shared(
            buffer(
                "world-mesh-vertices",
                bytemuck::cast_slice(vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            buffer(
                "world-mesh-indices",
                bytemuck::cast_slice(indices),
                wgpu::BufferUsages::INDEX,
            ),
            0,
            0,
            indices.len() as u32,
            material,
        )
    }

    /// A mesh that is a range of buffers it does not own: `index_count` indices from `first_index`, each added to `base_vertex`.
    pub fn shared(
        vertices: wgpu::Buffer,
        indices: wgpu::Buffer,
        base_vertex: i32,
        first_index: u32,
        index_count: u32,
        material: WorldMaterial,
    ) -> Self {
        Self {
            vertices,
            indices,
            base_vertex,
            first_index,
            index_count,
            material,
        }
    }

    /// The same mesh drawn with `material`, sharing its buffers.
    pub fn with_material(&self, material: WorldMaterial) -> Self {
        Self {
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
            material,
            ..*self
        }
    }

    pub fn material(&self) -> &WorldMaterial {
        &self.material
    }

    /// The bytes of the buffers it draws from, which a shared mesh does not own.
    pub fn bytes(&self) -> u64 {
        self.vertices.size() + self.indices.size()
    }
}

/// Where one instance of a mesh stands: the rows of its 3×4 model matrix, a tint whose colour its surface is multiplied by and whose alpha is how much of it its fade leaves (below nought, by the inverse dither of what it gives way to), and how much of the natural and of the artificial ambient light it takes; one cast only is not seen and casts the sun's shadow alone.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WorldInstance {
    pub rows: [[f32; 4]; 3],
    pub tint: [f32; 4],
    pub ambient: [f32; 2],
    /// 1 where it casts its shadow and is not seen, else 0.
    pub cast_only: f32,
}

/// The ambient scales of what stands outdoors: the whole of the natural light and none of the artificial.
pub const OUTDOORS: [f32; 2] = [1.0, 0.0];

impl WorldInstance {
    /// The instance `matrix` places, its surface multiplied by `tint`, standing outdoors.
    pub fn of(matrix: glam::Mat4, tint: [f32; 4]) -> Self {
        Self {
            rows: [0, 1, 2].map(|row| matrix.row(row).to_array()),
            tint,
            ambient: OUTDOORS,
            cast_only: 0.0,
        }
    }
}

/// A run of the frame's instances drawn with one mesh.
pub struct WorldDraw {
    pub mesh: Arc<WorldMesh>,
    pub instances: Range<u32>,
}

/// What the world's draws of a frame ask of the device, for a measurement to read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldCalls {
    /// Draws, and the calls they go out in, to the frame and to the four cascades of the shadow together.
    pub draws: usize,
    pub calls: usize,
    pub cast_calls: usize,
    /// Triangles drawn, each instance's counted, to the frame and to the four cascades of the shadow together.
    pub triangles: u64,
    pub cast_triangles: u64,
}

/// The floats of the viewport's camera uniform, two matrices and three vectors: a cascade is seen through a uniform as wide, of which the cast reads the first matrix.
const CAMERA_FLOATS: usize = 44;

/// The bytes one draw's arguments take in the buffer the draws of a frame are issued from.
const ARGS: wgpu::BufferAddress =
    std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>() as wgpu::BufferAddress;

/// What a viewport keeps of the world: the pipelines that draw it, what its materials share, the instances of this frame and the runs of them each mesh draws.
pub(crate) struct World {
    pipelines: Pipelines,
    materials: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// What each slot of a material samples where it names no map.
    stand_ins: [wgpu::TextureView; WORLD_MAPS],
    /// The standard material of no map and the constants most effects state, which every such material shares.
    plain: Arc<wgpu::BindGroup>,
    /// The sun and the ambient light of the game's frame, which what blends over it is lit by.
    light: wgpu::Buffer,
    light_group: wgpu::BindGroup,
    /// The view of each cascade of the sun's shadow, bound where a pass binds its camera.
    cascades: [(wgpu::Buffer, wgpu::BindGroup); CASCADES],
    /// The frame's instances, `per` to a buffer, each buffer made again only when a frame has more instances for it than it holds.
    instances: Vec<Option<wgpu::Buffer>>,
    per: u32,
    count: u32,
    /// The draws of this frame that draw something, in the order of their buckets, and the buffer of instances each draws from.
    draws: Vec<WorldDraw>,
    parts: Vec<u32>,
    /// The runs of `draws` one call issues, for a pass that samples the maps and for the cast of the shadow.
    batches: Vec<Range<u32>>,
    cast_batches: Vec<Range<u32>>,
    /// The arguments of each draw, where the device issues several draws by one call.
    indirect: Option<wgpu::Buffer>,
    multi: bool,
    /// The far stand-ins, drawn where the cutouts draw.
    cards: cards::Cards,
    /// GTA VI's water, drawn over a copy of the lit frame before what blends.
    pub(crate) water: water::WaterPass,
    calls: WorldCalls,
    /// The frame's instances as given, the sphere about each where the caller gave them, and how many sets of them came.
    given: Vec<WorldInstance>,
    spheres: Vec<[f32; 4]>,
    set: u64,
    /// What each cascade of the shadow casts of the frame.
    casting: RefCell<shadow::Casting>,
    /// How the cascades' split is stretched for what the camera rides.
    pub(crate) focus: ShadowFocus,
    /// The terrain's shadow past the cascades, once its heights are given.
    pub(crate) terrain: Option<shadow_terrain::TerrainShadow>,
    /// What a material's own light is multiplied by: the exposure the day's light is given at, 1 where the frame keeps its own day.
    exposure: f32,
    /// When the world was made: the wind moves by the seconds since.
    started: std::time::Instant,
    /// The lamps that light what blends, nearest first.
    lamps: Vec<WorldLamp>,
    /// The maps and constants of each material made, by [`alike`], so that materials alike bind one group and their draws join; and the size past which the dead are let go.
    alike: RefCell<(Alike, usize)>,
}

/// The bind group made for each material's maps and constants, by [`alike`], held weakly.
type Alike = HashMap<[u64; 2], Weak<wgpu::BindGroup>>;

/// Two hashes of a material's maps and constants, which materials alike have in common.
// ponytail: a material is taken for another of the same two 64-bit hashes; store the views themselves if one in 2^128 is too many.
fn alike(views: &[&wgpu::TextureView; WORLD_MAPS], uniform: &[u8]) -> [u64; 2] {
    use std::hash::{Hash, Hasher};
    [0u8, 1].map(|salt| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        salt.hash(&mut hasher);
        uniform.hash(&mut hasher);
        views.hash(&mut hasher);
        hasher.finish()
    })
}

/// A material as the pipelines bind it: its maps by slot, the one sampler after them, and its constants after that, the family in front.
fn bound(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    maps: [&wgpu::TextureView; WORLD_MAPS],
    (family, constants): (WorldFamily, &WorldConstants),
) -> wgpu::BindGroup {
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("viewport-world-material-constants"),
        contents: &uniform_bytes(family, constants),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let mut entries: Vec<wgpu::BindGroupEntry> = (maps.iter().zip(0u32..))
        .map(|(view, binding)| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        })
        .collect();
    entries.push(wgpu::BindGroupEntry {
        binding: WORLD_MAPS as u32,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: WORLD_MAPS as u32 + 1,
        resource: uniform.as_entire_binding(),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport-world-material-bg"),
        layout,
        entries: &entries,
    })
}

/// Write `bytes` to the buffer in `slot`, which is made again where there is none or it holds fewer.
fn fill(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &mut Option<wgpu::Buffer>,
    label: &'static str,
    usage: wgpu::BufferUsages,
    bytes: &[u8],
) {
    let needed = bytes.len() as wgpu::BufferAddress;
    if slot.as_ref().is_none_or(|buffer| buffer.size() < needed) {
        let most = device.limits().max_buffer_size;
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: needed.next_power_of_two().max(4096).min(most),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    if let Some(buffer) = slot {
        queue.write_buffer(buffer, 0, bytes);
    }
}

/// The runs of `draws`, which are in the order of their buckets, that one call can issue: draws side by side of one bucket, the same two buffers, the same buffer of instances by `parts` and the same material. For the cast of the shadow (`cast`) only what casts is taken, and what casts whole needs no map, so any material joins.
fn batches(draws: &[WorldDraw], parts: &[u32], cast: bool) -> Vec<Range<u32>> {
    let mut batches: Vec<Range<u32>> = Vec::new();
    let mut before: Option<(&WorldMesh, u32)> = None;
    for (at, (draw, part)) in draws.iter().zip(parts).enumerate() {
        let (at, mesh) = (at as u32, &*draw.mesh);
        let bucket = mesh.material.bucket;
        if cast && !bucket.casts() {
            continue;
        }
        let joins = before.is_some_and(|(before, was)| {
            before.material.bucket == bucket
                && was == *part
                && before.vertices == mesh.vertices
                && before.indices == mesh.indices
                && ((cast && bucket != WorldBucket::Cutout) || before.material == mesh.material)
        });
        match batches.last_mut().filter(|last| joins && last.end == at) {
            Some(last) => last.end = at + 1,
            None => batches.push(at..at + 1),
        }
        before = Some((mesh, *part));
    }
    batches
}

/// `draws` with each run of instances that crosses the end of a buffer of `per` cut into one draw a buffer, in the same order, and the buffer each draws from.
fn split(draws: Vec<WorldDraw>, per: u32) -> (Vec<WorldDraw>, Vec<u32>) {
    let per = per.max(1);
    let (mut out, mut parts) = (
        Vec::with_capacity(draws.len()),
        Vec::with_capacity(draws.len()),
    );
    for draw in draws {
        let mut start = draw.instances.start;
        while start < draw.instances.end {
            let part = start / per;
            let end = draw.instances.end.min((part + 1).saturating_mul(per));
            out.push(WorldDraw {
                mesh: Arc::clone(&draw.mesh),
                instances: start..end,
            });
            parts.push(part);
            start = end;
        }
    }
    (out, parts)
}

impl World {
    /// The pipelines and what the materials share, for a viewport whose colour target is of `colour_format`, with the layout of its camera.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_bgl: &wgpu::BindGroupLayout,
        colour_format: wgpu::TextureFormat,
    ) -> Self {
        let map = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let mut entries: Vec<wgpu::BindGroupLayoutEntry> =
            (0..WORLD_MAPS as u32).map(map).collect();
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: WORLD_MAPS as u32,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        // The vertex stage reads the family: a mesh the viewer builds keeps its colour as it is.
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: WORLD_MAPS as u32 + 1,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let materials = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-world-material-bgl"),
            entries: &entries,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewport-world-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        // Half floats, so that the normal's half is a half exactly and tilts nothing
        let stand_ins = STAND_INS.map(|texel| constant_view(device, queue, texel));
        let plain = Arc::new(bound(
            device,
            &materials,
            &sampler,
            stand_ins.each_ref(),
            (WorldFamily::Standard, &WorldConstants::default()),
        ));

        let uniform = |label, floats: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (floats * std::mem::size_of::<f32>()) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let group = |label, layout: &wgpu::BindGroupLayout, buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            })
        };
        let light_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-world-light-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // The vertex stage reads the seconds the wind moves by
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let light = uniform("viewport-world-light", lamps::LIGHT_FLOATS);
        let light_group = group("viewport-world-light-bg", &light_layout, &light);
        let cascades = std::array::from_fn(|_| {
            let view = uniform("viewport-world-cascade", CAMERA_FLOATS);
            let seen = group("viewport-world-cascade-bg", camera_bgl, &view);
            (view, seen)
        });
        Self {
            pipelines: Pipelines::new(device, camera_bgl, &materials, &light_layout, colour_format),
            cards: cards::Cards::new(device, queue, camera_bgl, &materials, colour_format),
            water: water::WaterPass::new(device, queue, camera_bgl, &light_layout),
            calls: WorldCalls::default(),
            given: Vec::new(),
            spheres: Vec::new(),
            set: 0,
            casting: RefCell::default(),
            focus: ShadowFocus::FREE,
            terrain: None,
            exposure: 1.0,
            started: std::time::Instant::now(),
            lamps: Vec::new(),
            alike: RefCell::default(),
            materials,
            sampler,
            stand_ins,
            plain,
            light,
            light_group,
            cascades,
            instances: Vec::new(),
            per: 1,
            count: 0,
            draws: Vec::new(),
            parts: Vec::new(),
            batches: Vec::new(),
            cast_batches: Vec::new(),
            indirect: None,
            // An indirect draw may start at an instance other than the first only where the device says so
            multi: device
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
        }
    }

    fn material(&self, device: &wgpu::Device, surface: &WorldSurface<'_>) -> WorldMaterial {
        let plain = surface.maps.iter().all(Option::is_none)
            && surface.family == WorldFamily::Standard
            && surface.constants == WorldConstants::default();
        let maps = match plain {
            true => Arc::clone(&self.plain),
            false => {
                // The four-layer family's slots 9 to 11 are its fourth layer's, and terrain LOD's material A reads its green as a smoothness, which material B's stand-in leaves at nought
                let stand_in = |at: usize| match (surface.family, at) {
                    (WorldFamily::FourLayer, 9..=11) => &self.stand_ins[at - 9],
                    (WorldFamily::TerrainLod, at) if at == slot::material_a(0) => {
                        &self.stand_ins[slot::MATERIAL_B[0]]
                    }
                    _ => &self.stand_ins[at],
                };
                let views = std::array::from_fn(|at| surface.maps[at].unwrap_or(stand_in(at)));
                let constants = (surface.family, &surface.constants);
                // ⚠ A bind group alive holds its views, so a view met again under a live entry is the one it was made with.
                let key = alike(&views, &uniform_bytes(surface.family, &surface.constants));
                let (made, prune_at) = &mut *self.alike.borrow_mut();
                match made.get(&key).and_then(Weak::upgrade) {
                    Some(maps) => maps,
                    None => {
                        let maps = Arc::new(bound(
                            device,
                            &self.materials,
                            &self.sampler,
                            views,
                            constants,
                        ));
                        if made.len() >= *prune_at {
                            made.retain(|_, group| group.strong_count() > 0);
                            *prune_at = (2 * made.len()).max(1024);
                        }
                        made.insert(key, Arc::downgrade(&maps));
                        maps
                    }
                }
            }
        };
        let foliage = surface.constants.flags & WorldConstants::FOLIAGE != 0;
        WorldMaterial {
            maps,
            bucket: surface.bucket,
            stencil: if foliage {
                MATERIAL_TREE
            } else {
                MATERIAL_DEFAULT
            },
        }
    }

    fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[WorldInstance],
        mut draws: Vec<WorldDraw>,
    ) {
        // ⚠ The device makes no buffer larger than its limit: the frame's instances are laid in as many as hold them.
        let most = device.limits().max_buffer_size / std::mem::size_of::<WorldInstance>() as u64;
        self.per = most.clamp(1, u64::from(u32::MAX)) as u32;
        let instances = &instances[..instances.len().min(u32::MAX as usize)];
        self.count = instances.len() as u32;
        // A run past the instances of this frame would read what an earlier frame left in the buffer
        for draw in &mut draws {
            draw.instances.end = draw.instances.end.min(self.count);
        }
        draws.retain(|draw| !draw.instances.is_empty() && draw.mesh.index_count > 0);
        // The sort keeps the order the caller gave the draws of a bucket
        draws.sort_by_key(|draw| draw.mesh.material.bucket);
        let (draws, parts) = split(draws, self.per);
        self.batches = batches(&draws, &parts, false);
        self.cast_batches = batches(&draws, &parts, true);
        let triangles = |draw: &WorldDraw| {
            u64::from(draw.mesh.index_count / 3)
                * u64::from(draw.instances.end - draw.instances.start)
        };
        self.calls = WorldCalls {
            draws: draws.len(),
            calls: match self.multi {
                true => self.batches.len(),
                false => draws.len(),
            },
            triangles: draws.iter().map(triangles).sum(),
            ..WorldCalls::default()
        };
        if !draws.is_empty() {
            let laid: Vec<&[WorldInstance]> = instances.chunks(self.per as usize).collect();
            self.instances.resize_with(laid.len(), || None);
            for (slot, part) in self.instances.iter_mut().zip(laid) {
                fill(
                    device,
                    queue,
                    slot,
                    "viewport-world-instances",
                    wgpu::BufferUsages::VERTEX,
                    bytemuck::cast_slice(part),
                );
            }
        }
        if self.multi && !draws.is_empty() {
            let mut arguments = Vec::with_capacity(draws.len() * ARGS as usize);
            for (draw, part) in draws.iter().zip(&parts) {
                let call = wgpu::util::DrawIndexedIndirectArgs {
                    index_count: draw.mesh.index_count,
                    instance_count: draw.instances.end - draw.instances.start,
                    first_index: draw.mesh.first_index,
                    base_vertex: draw.mesh.base_vertex,
                    first_instance: draw.instances.start - part * self.per,
                };
                arguments.extend_from_slice(call.as_bytes());
            }
            fill(
                device,
                queue,
                &mut self.indirect,
                "viewport-world-draws",
                wgpu::BufferUsages::INDIRECT,
                &arguments,
            );
        }
        (self.draws, self.parts) = (draws, parts);
        // Each cascade casts what of these reaches its tile, once the frame's cascades are known
        self.given.clear();
        self.given.extend_from_slice(instances);
        self.spheres.clear();
        self.set += 1;
        // ⚠ The meshes of a level draw the same run of instances: a cascade may cast an instance once for each.
        let cast: usize = (self.draws.iter())
            .filter(|draw| draw.mesh.material.bucket.casts())
            .map(|draw| draw.instances.len())
            .sum();
        let casting = self.casting.get_mut();
        casting.room(device, cast, self.draws.len());
    }

    fn draws_anything(&self) -> bool {
        !self.draws.is_empty()
    }

    /// Issue `batches` into `pass`, each with the pipeline `pipelines` holds for its bucket: a bucket it holds none for is left out, and a buffer or a material that is bound already is not bound again. With `cast`, each draw's instances are those that cascade casts.
    fn record(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        batches: &[Range<u32>],
        pipelines: &ByBucket,
        cast: Option<(&shadow::Casting, usize)>,
        stencils: bool,
    ) {
        let mut set: Option<(&wgpu::RenderPipeline, &WorldMesh)> = None;
        let mut stencil = MATERIAL_DEFAULT;
        let mut bound = None;
        for batch in batches {
            let run = &self.draws[batch.start as usize..batch.end as usize];
            let Some(mesh) = run.first().map(|draw| &*draw.mesh) else {
                continue;
            };
            let Some(pipeline) = &pipelines[mesh.material.bucket as usize] else {
                continue;
            };
            // A batch draws from one buffer of the frame's instances; the cast binds its own.
            let part = self.parts.get(batch.start as usize).copied().unwrap_or(0);
            if cast.is_none() && bound != Some(part) {
                let Some(Some(instances)) = self.instances.get(part as usize) else {
                    continue;
                };
                pass.set_vertex_buffer(1, instances.slice(..));
                bound = Some(part);
            }
            if !set.is_some_and(|(last, _)| std::ptr::eq(last, pipeline)) {
                pass.set_pipeline(pipeline);
            }
            if !set.is_some_and(|(_, last)| last.vertices == mesh.vertices) {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            }
            if !set.is_some_and(|(_, last)| last.indices == mesh.indices) {
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            }
            let maps = &mesh.material.maps;
            if !set.is_some_and(|(_, last)| Arc::ptr_eq(&last.material.maps, maps)) {
                pass.set_bind_group(1, &**maps, &[]);
            }
            set = Some((pipeline, mesh));
            // A batch is of one material, and so names one material in the stencil
            if stencils && mesh.material.stencil != stencil {
                stencil = mesh.material.stencil;
                pass.set_stencil_reference(stencil);
            }
            // A cascade's arguments follow those of the cascades before it, a draw's for each draw of the frame
            let (arguments, first) = match cast {
                Some((casting, cascade)) => (&casting.indirect, cascade * self.draws.len()),
                None => (&self.indirect, 0),
            };
            match arguments.as_ref().filter(|_| self.multi) {
                Some(arguments) => pass.multi_draw_indexed_indirect(
                    arguments,
                    (first as wgpu::BufferAddress + wgpu::BufferAddress::from(batch.start)) * ARGS,
                    batch.end - batch.start,
                ),
                None => {
                    for (draw, at) in run.iter().zip(batch.start as usize..) {
                        let mesh = &draw.mesh;
                        let instances = match cast {
                            Some((casting, cascade)) => casting.ranges[cascade][at].clone(),
                            None => {
                                let first = part * self.per;
                                draw.instances.start - first..draw.instances.end - first
                            }
                        };
                        pass.draw_indexed(
                            mesh.first_index..mesh.first_index + mesh.index_count,
                            mesh.base_vertex,
                            instances,
                        );
                    }
                }
            }
        }
    }

    /// Draw the buckets `pipelines` holds into `pass`, seen through `camera`.
    fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        camera: &wgpu::BindGroup,
        pipelines: &ByBucket,
        stencils: bool,
    ) {
        if !self.draws_anything() {
            return;
        }
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(2, &self.light_group, &[]);
        self.record(pass, &self.batches, pipelines, None, stencils);
    }

    /// Draw every bucket of the world lit as the plain modes light a surface, into a pass over the viewport's colour and depth.
    pub(crate) fn draw_forward(&self, pass: &mut wgpu::RenderPass<'_>, camera: &wgpu::BindGroup) {
        self.draw(pass, camera, &self.pipelines.forward, false);
        self.cards.draw_forward(pass, camera);
    }

    /// Draw what is opaque, then the decals over it, into a pass that fills the G-buffer for the game's lighting passes to light.
    pub(crate) fn draw_gbuffer(&self, pass: &mut wgpu::RenderPass<'_>, camera: &wgpu::BindGroup) {
        pass.set_stencil_reference(MATERIAL_DEFAULT);
        self.draw(pass, camera, &self.pipelines.gbuffer, true);
        // A far stand-in is a tree's, lit as the tree material its model is lit as
        pass.set_stencil_reference(MATERIAL_TREE);
        self.cards.draw_gbuffer(pass, camera);
    }

    /// Draw what blends into a pass over the frame the game's passes lit, which reads the depth of the frame and writes none: the faces that turn away, then those that face the camera.
    pub(crate) fn draw_lit(&self, pass: &mut wgpu::RenderPass<'_>, camera: &wgpu::BindGroup) {
        // ⚠ The water draws where the world has nothing else to, as over the open sea; leaving the world takes its mesh away.
        self.water.draw(pass, camera, &self.light_group);
        for faces in &self.pipelines.lit {
            self.draw(pass, camera, faces, false);
        }
    }

    /// Write the light of the game's frame, which [`Self::draw_lit`] lights what blends by, and how far the night emissives are on, which every material's own light is blended by.
    pub(crate) fn write_light(&self, queue: &wgpu::Queue, engine: &EngineConstants) {
        let sun = Sun::of(engine);
        let ([down, up], night) = match &engine.lighting {
            Some(lighting) => (
                lighting.natural_ambient.map(|ambient| ambient.truncate()),
                lighting.night_emissive,
            ),
            // With no day the emissives are on, as the game's own scalars have them
            None => ([engine.ambient; 2], 1.0),
        };
        let mut light = [sun.travel.normalize_or_zero(), sun.colour, down, up]
            .map(|vector| vector.extend(0.0).to_array())
            .to_vec();
        let lamps = self.lamps.len().min(WORLD_LAMPS) as f32;
        // ⚠ Seconds are kept under a day so that a float still holds a frame's step.
        let seconds = self.started.elapsed().as_secs_f32() % 86_400.0;
        light.push([night, self.exposure, lamps, seconds]);
        light.extend(lamps::rows(&self.lamps));
        queue.write_buffer(&self.light, 0, bytemuck::cast_slice(&light));
    }

    /// The cascades the sun's shadow of this frame's world is cast into as far as the cycle's `multiplier` takes them, the view of each written for the cast; none where nothing of the world casts or the sun does not shine.
    pub(crate) fn cascades(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        camera: &OrbitCamera,
        aspect: f32,
        multiplier: f32,
    ) -> Option<Cascades> {
        if self.cast_batches.is_empty() && !self.cards.draws_anything() {
            return None;
        }
        let cascades = Cascades::about(engine, camera, aspect, (multiplier, self.focus))?;
        let views = cascades.views();
        for (view, (buffer, _)) in views.iter().zip(&self.cascades) {
            let mut uniform = [0.0f32; CAMERA_FLOATS];
            uniform[..16].copy_from_slice(&view.to_cols_array());
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&uniform));
        }
        let frame = shadow::Frame {
            draws: &self.draws,
            instances: &self.given,
            spheres: &self.spheres,
            batches: &self.cast_batches,
            multi: self.multi,
        };
        (self.casting.borrow_mut()).cast(queue, &views, self.set, &frame);
        Some(cascades)
    }

    /// Cast the world into the map of the sun's shadow, each cascade into its tile, of its instances those that reach it: `pass` is the one the game's map is cast in, and [`Self::cascades`] wrote the views and the lists this frame.
    pub(crate) fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>) {
        let casting = self.casting.borrow();
        let instances = (casting.instances.as_ref()).filter(|_| self.draws_anything());
        if instances.is_none() && !self.cards.draws_anything() {
            return;
        }
        for (tile, (_, view)) in self.cascades.iter().enumerate() {
            // The tiles are one above the other, the nearest cascade at the top
            let top = tile as u32 * TILE;
            pass.set_viewport(0.0, top as f32, TILE as f32, TILE as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, top, TILE, TILE);
            pass.set_bind_group(0, view, &[]);
            if let Some(instances) = instances {
                pass.set_vertex_buffer(1, instances.slice(..));
                let batches = &casting.batches[tile];
                self.record(
                    pass,
                    batches,
                    &self.pipelines.cast,
                    Some((&*casting, tile)),
                    false,
                );
            }
            self.cards.draw_shadow(pass);
        }
    }
}

impl ViewportRenderer {
    /// A standard material of `bucket` that samples these maps, `specular` as its material A; a map left out is white, the vertex normal, and fully rough.
    pub fn world_material(
        &self,
        device: &wgpu::Device,
        colour: Option<&wgpu::TextureView>,
        normal: Option<&wgpu::TextureView>,
        specular: Option<&wgpu::TextureView>,
        bucket: WorldBucket,
    ) -> WorldMaterial {
        let mut maps = [None; WORLD_MAPS];
        maps[slot::colour(0)] = colour;
        maps[slot::normal(0)] = normal;
        maps[slot::material_a(0)] = specular;
        let surface = WorldSurface {
            family: WorldFamily::Standard,
            bucket,
            maps,
            constants: WorldConstants::default(),
        };
        self.world.material(device, &surface)
    }

    /// A material that shades by `surface`; a clone shares its maps and constants.
    pub fn world_surface(
        &self,
        device: &wgpu::Device,
        surface: &WorldSurface<'_>,
    ) -> WorldMaterial {
        self.world.material(device, surface)
    }

    /// The world this frame draws: every instance, and the runs of them each mesh draws, in any order; an empty list draws none.
    pub fn set_world(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[WorldInstance],
        draws: Vec<WorldDraw>,
    ) {
        self.world.set(device, queue, instances, draws);
    }

    /// Stretch the sun's cascades of the world for a camera that rides what `focus` is of.
    pub fn set_shadow_focus(&mut self, focus: ShadowFocus) {
        self.world.focus = focus;
    }

    /// Multiply every material's own light by `exposure`, the exposure the day's light is given at; 1 leaves it as the material states it.
    pub fn set_world_exposure(&mut self, exposure: f32) {
        self.world.exposure = exposure;
    }

    /// Light what blends over the lit frame by `lamps`, nearest first, of which the first [`WORLD_LAMPS`] count.
    pub fn set_world_lamps(&mut self, lamps: Vec<WorldLamp>) {
        self.world.lamps = lamps;
    }

    /// How many instances the world draws.
    pub fn world_instances(&self) -> u32 {
        self.world.count
    }

    /// What the world's draws of the last frame set ask of the device, the shadow's as last cast.
    pub fn world_calls(&self) -> WorldCalls {
        let world = &self.world;
        let (cast_calls, cast_triangles) =
            world
                .casting
                .borrow()
                .cost(&world.draws, world.multi, world.set);
        WorldCalls {
            cast_calls,
            cast_triangles,
            ..world.calls
        }
    }

    /// The sphere about each instance the last [`Self::set_world`] gave, `[x, y, z, radius]`, so that a cascade of the shadow casts only those that reach its tile; one with none is cast into every cascade.
    pub fn set_world_spheres(&mut self, spheres: &[[f32; 4]]) {
        self.world.spheres.clear();
        self.world.spheres.extend_from_slice(spheres);
        self.world.set += 1;
    }

    /// Whether there is a world to draw.
    pub fn has_world(&self) -> bool {
        self.world.draws_anything()
    }
}

#[cfg(test)]
mod tests;
