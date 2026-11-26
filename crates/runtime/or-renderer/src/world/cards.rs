//! The far stand-ins: one camera-facing card a record of a list, its texels taken from the octahedral atlas its species is baked into, drawn where the world's cutouts draw.

use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::game_shaders::deferred::{DEPTH_STENCIL_FORMAT, GBUFFER_FORMAT, GBUFFER_TARGETS};
use crate::game_shaders::mesh::scene::writes_material;
use crate::game_shaders::shadow::MAP_FORMAT;
use crate::viewport::ViewportRenderer;

#[path = "cards_bake.rs"]
mod bake;
#[path = "cards_shader.rs"]
mod shader;

use bake::Bake;
use shader::CARDS_SHADER;

/// One list of far stand-ins as the card pass draws it: its box, its scales, how far its instances draw, its fade, and its 16-byte records as the map states them.
#[derive(Clone)]
pub struct CardBatch {
    pub min: [f32; 3],
    pub size: [f32; 3],
    pub scale_range: [f32; 2],
    /// The furthest from the camera an instance of it draws, in metres.
    pub distance: f32,
    /// How much of the list its fade leaves, from 0 to 1.
    pub fade: f32,
    pub records: Arc<[[u8; 16]]>,
}

/// A species a record names by its id: its model's bounding centre and radius, and the slot of the atlas its frames are in; a species with no slot draws nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardSpecies {
    pub centre: [f32; 3],
    pub radius: f32,
    pub slot: Option<u32>,
}

/// A mesh a species is baked from, in its model's own space: a range of buffers of `MeshVertex` and `u32` indices, and the material the world draws it with, whose arithmetic its colour is worked out by; a cutout's texels of under half coverage are left out.
pub struct CardMesh<'a> {
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub base_vertex: i32,
    pub first_index: u32,
    pub index_count: u32,
    pub material: super::WorldMaterial,
}

/// What the stand-ins' pixel stage takes of the visual settings: the scale its G-buffer's diffuse colour is taken to with distance, and the distance that scaling starts at and the span it takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardLook {
    pub diffuse_scale: [f32; 3],
    pub scale_start: f32,
    pub scale_range: f32,
}

impl Default for CardLook {
    /// The executable's own values, which leave the colour as it is.
    fn default() -> Self {
        Self {
            diffuse_scale: [1.0; 3],
            scale_start: 100.0,
            scale_range: 100.0,
        }
    }
}

/// The look as the pixel stage reads it: the diffuse scale, then the start and one over the span.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuLook {
    diffuse: [f32; 4],
    ramp: [f32; 4],
}

impl GpuLook {
    fn of(look: &CardLook) -> Self {
        let [r, g, b] = look.diffuse_scale;
        Self {
            diffuse: [r, g, b, 0.0],
            // A span of nought scales the colour whole at the start.
            ramp: [look.scale_start, 1.0 / look.scale_range.max(1e-4), 0.0, 0.0],
        }
    }
}

/// The frames of a species' grid along each side.
pub const CARD_FRAMES: u32 = 8;

/// What a species with no slot is given on the device.
const NO_SLOT: u32 = u32::MAX;

/// How far a card that casts is held back by how steeply it meets the light: what the world's own casters are held back by.
const CAST_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 4,
    slope_scale: 2.5,
    clamp: 0.0,
};

/// A list as the vertex stage reads it: the box's least corner and the least scale, the box's size and the most scale, and how far it draws and its fade.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuList {
    low: [f32; 4],
    size: [f32; 4],
    reach: [f32; 4],
}

/// A species as the vertex stage reads it: centre and radius, the card's width and height over the radius, and its slot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuSpecies {
    centre: [f32; 4],
    extent: [f32; 2],
    slot: u32,
    pad: u32,
}

impl GpuSpecies {
    fn of(species: &CardSpecies) -> Self {
        // ⚠ A species of no size would draw a card of none: it is given no slot.
        let slot = species.slot.filter(|_| species.radius > 0.0);
        let [x, y, z] = species.centre;
        Self {
            centre: [x, y, z, species.radius],
            // A baked frame is square: its height over its width is one.
            extent: [1.0, 1.0],
            slot: slot.unwrap_or(NO_SLOT),
            pad: 0,
        }
    }
}

/// The way the frame at `col`, `row` of a species' grid is baked from, in the card's own frame: east, north and its up axis. The middle of the grid looks down, its edge along the horizon.
pub(crate) fn frame_direction(col: u32, row: u32) -> Vec3 {
    let (col, row) = (col as f32, row as f32);
    // ⚠ East runs from the low row to the high column, so that the frame the vertex stage picks for a way is the one baked from it.
    let a = (col - row) / 7.0;
    let b = (row + col) / 7.0 - 1.0;
    Vec3::new(a, b, (8.0 / 7.0) * (1.0 - a.abs() - b.abs()) - 1.0 / 7.0).normalize()
}

/// The card's right and up in the frame seen from `d`, both square to it; the vertex stage's and the bake's.
pub(crate) fn frame_axes(d: Vec3, east: Vec3, up: Vec3) -> (Vec3, Vec3) {
    let right = up.cross(d);
    let right = if right.length_squared() > 1e-8 {
        right.normalize()
    } else {
        east
    };
    (right, d.cross(right))
}

/// The pipelines and the buffers of the far stand-ins, and the atlas of their species.
pub(crate) struct Cards {
    forward: wgpu::RenderPipeline,
    gbuffer: wgpu::RenderPipeline,
    cast: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The records, the list of each record, the lists, the species and the look, and the bytes of each that are bound.
    buffers: [Option<wgpu::Buffer>; 5],
    bound: [u64; 5],
    look: Option<CardLook>,
    group: Option<wgpu::BindGroup>,
    /// The records of each list last set, by its place, and the records last uploaded.
    spans: Vec<Range<u32>>,
    uploaded: Vec<Arc<[[u8; 16]]>>,
    /// The runs of records of the lists shown, those side by side joined.
    shown: Vec<Range<u32>>,
    bake: Bake,
}

const RECORDS: usize = 0;
const LIST_OF: usize = 1;
const LISTS: usize = 2;
const SPECIES: usize = 3;
const LOOK: usize = 4;
/// The binding each buffer is bound at.
const BINDINGS: [u32; 5] = [0, 1, 2, 3, 6];

impl Cards {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_bgl: &wgpu::BindGroupLayout,
        materials: &wgpu::BindGroupLayout,
        colour_format: wgpu::TextureFormat,
    ) -> Self {
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-cards-bgl"),
            entries: &[
                storage(0),
                storage(1),
                storage(2),
                storage(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ..storage(BINDINGS[LOOK])
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewport-cards-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport-cards-shader"),
            source: wgpu::ShaderSource::Wgsl(CARDS_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-cards-pipeline-layout"),
            bind_group_layouts: &[Some(camera_bgl), Some(&layout)],
            immediate_size: 0,
        });
        let build = |label: &str,
                     (vertex, fragment): (&str, &str),
                     targets: &[Option<wgpu::ColorTargetState>],
                     depth_stencil: wgpu::DepthStencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(depth_stencil),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // The depth of the viewport and of the game's frame, where the nearer a surface the greater its depth
        let frame = |stencil| wgpu::DepthStencilState {
            format: DEPTH_STENCIL_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil,
            bias: wgpu::DepthBiasState::default(),
        };
        let target = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let forward = build(
            "viewport-cards-forward-pipeline",
            ("vs_main", "fs_forward"),
            &[target(colour_format)],
            frame(wgpu::StencilState::default()),
        );
        let gbuffer = build(
            "viewport-cards-gbuffer-pipeline",
            ("vs_main", "fs_gbuffer"),
            &vec![target(GBUFFER_FORMAT); GBUFFER_TARGETS],
            frame(writes_material()),
        );
        // A tile of the map: the nearer the sun a surface the less its depth, and no colour is written
        let cast = build(
            "viewport-cards-cast-pipeline",
            ("vs_cast", "fs_cast"),
            &[],
            wgpu::DepthStencilState {
                format: MAP_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: CAST_BIAS,
            },
        );
        let mut cards = Self {
            forward,
            gbuffer,
            cast,
            layout,
            sampler,
            buffers: [None, None, None, None, None],
            bound: [0; 5],
            look: None,
            group: None,
            spans: Vec::new(),
            uploaded: Vec::new(),
            shown: Vec::new(),
            bake: Bake::new(device, materials),
        };
        cards.write(
            device,
            queue,
            SPECIES,
            bytemuck::bytes_of(&GpuSpecies::of(&NONE)),
        );
        for at in [RECORDS, LIST_OF, LISTS] {
            cards.write(device, queue, at, &[0; 64]);
        }
        cards.set_look(device, queue, CardLook::default());
        cards
    }

    /// Hand the pixel stage `look` where it differs from the one it holds.
    fn set_look(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, look: CardLook) {
        if self.look == Some(look) {
            return;
        }
        self.look = Some(look);
        self.write(device, queue, LOOK, bytemuck::bytes_of(&GpuLook::of(&look)));
        self.regroup(device);
    }

    /// Write `bytes` to buffer `at`, which is bound at their length.
    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, at: usize, bytes: &[u8]) {
        let label = [
            "cards-records",
            "cards-list-of",
            "cards-lists",
            "cards-species",
            "cards-look",
        ][at];
        super::fill(
            device,
            queue,
            &mut self.buffers[at],
            label,
            wgpu::BufferUsages::STORAGE,
            bytes,
        );
        self.bound[at] = bytes.len() as u64;
    }

    fn regroup(&mut self, device: &wgpu::Device) {
        let mut entries = Vec::with_capacity(7);
        for (at, (buffer, bound)) in self.buffers.iter().zip(self.bound).enumerate() {
            let (Some(buffer), Some(size)) = (buffer, NonZeroU64::new(bound)) else {
                return;
            };
            entries.push(wgpu::BindGroupEntry {
                binding: BINDINGS[at],
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: 0,
                    size: Some(size),
                }),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::TextureView(self.bake.atlas()),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 5,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        });
        self.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-cards-bg"),
            layout: &self.layout,
            entries: &entries,
        }));
    }

    /// Upload `lists`, each record with the place of its list; a list past the most a storage binding holds is left out. None is shown.
    // ⚠ Lists whose records are the very ones uploaded last, in the same order, write only their distances and fades: a fade changes them every selection.
    fn set_lists(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, lists: &[CardBatch]) {
        let most = device.limits().max_storage_buffer_binding_size / 16;
        let held: usize = lists.iter().map(|list| list.records.len()).sum();
        let same_spans = lists.len() == self.uploaded.len()
            && (lists.iter().zip(&self.uploaded))
                .all(|(list, was)| list.records.len() == was.len());
        // Lists laid out as before rewrite only the records of those that changed: a list with instances hidden is a new one every selection.
        if same_spans && self.buffers[RECORDS].is_some() {
            for (at, list) in lists.iter().enumerate() {
                let (Some(span), Some(buffer)) = (self.spans.get(at), &self.buffers[RECORDS])
                else {
                    continue;
                };
                if !Arc::ptr_eq(&list.records, &self.uploaded[at])
                    && span.len() == list.records.len()
                {
                    let offset = u64::from(span.start) * 16;
                    queue.write_buffer(buffer, offset, bytemuck::cast_slice(&list.records[..]));
                    self.uploaded[at] = Arc::clone(&list.records);
                }
            }
        }
        let again = lists.len() == self.uploaded.len()
            && (lists.iter().zip(&self.uploaded))
                .all(|(list, was)| Arc::ptr_eq(&list.records, was));
        let mut records: Vec<u8> = Vec::with_capacity(if again { 0 } else { held * 16 });
        let mut list_of: Vec<u32> = Vec::with_capacity(if again { 0 } else { held });
        let mut gpu = Vec::with_capacity(lists.len());
        self.spans.clear();
        self.shown.clear();
        let mut start = 0u32;
        for (at, list) in lists.iter().enumerate() {
            let end = u64::from(start) + list.records.len() as u64;
            if end > most || end > u64::from(u32::MAX) {
                tracing::warn!(
                    lists = lists.len() - at,
                    "far stand-ins past what the device binds are left out"
                );
                break;
            }
            if !again {
                records.extend_from_slice(bytemuck::cast_slice(&list.records[..]));
                list_of.extend(std::iter::repeat_n(at as u32, list.records.len()));
            }
            let [x, y, z] = list.min;
            let [w, h, d] = list.size;
            let [least, most_scale] = list.scale_range;
            gpu.push(GpuList {
                low: [x, y, z, least],
                size: [w, h, d, most_scale],
                reach: [list.distance, list.fade, 0.0, 0.0],
            });
            self.spans.push(start..end as u32);
            start = end as u32;
        }
        // ⚠ A binding is never empty: nothing reads past the records the draws name.
        if gpu.is_empty() {
            gpu.push(GpuList::zeroed());
        }
        if !again {
            if records.is_empty() {
                records.resize(16, 0);
                list_of.resize(4, 0);
            }
            self.write(device, queue, RECORDS, &records);
            self.write(device, queue, LIST_OF, bytemuck::cast_slice(&list_of));
            // Held, so that no other records come to lie where these did and pass for them.
            self.uploaded = lists.iter().map(|list| Arc::clone(&list.records)).collect();
        }
        self.write(device, queue, LISTS, bytemuck::cast_slice(&gpu));
        self.regroup(device);
    }

    fn set_species(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, species: &[CardSpecies]) {
        let mut gpu: Vec<GpuSpecies> = species.iter().map(GpuSpecies::of).collect();
        if gpu.is_empty() {
            gpu.push(GpuSpecies::of(&NONE));
        }
        self.write(device, queue, SPECIES, bytemuck::cast_slice(&gpu));
        self.regroup(device);
    }

    fn show(&mut self, shown: &[u32]) {
        self.shown.clear();
        for span in shown.iter().filter_map(|at| self.spans.get(*at as usize)) {
            match self.shown.last_mut().filter(|last| last.end == span.start) {
                Some(last) => last.end = span.end,
                None if !span.is_empty() => self.shown.push(span.clone()),
                None => {}
            }
        }
    }

    pub(crate) fn draws_anything(&self) -> bool {
        !self.shown.is_empty()
    }

    fn record(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        let Some(group) = self.group.as_ref().filter(|_| self.draws_anything()) else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, group, &[]);
        for run in &self.shown {
            pass.draw(0..4, run.clone());
        }
    }

    /// Draw the cards lit as the plain modes light a surface, into a pass over the viewport's colour and depth.
    pub(crate) fn draw_forward(&self, pass: &mut wgpu::RenderPass<'_>, camera: &wgpu::BindGroup) {
        pass.set_bind_group(0, camera, &[]);
        self.record(pass, &self.forward);
    }

    /// Draw the cards into the G-buffer, the pass's stencil reference already set as the world's.
    pub(crate) fn draw_gbuffer(&self, pass: &mut wgpu::RenderPass<'_>, camera: &wgpu::BindGroup) {
        pass.set_bind_group(0, camera, &[]);
        self.record(pass, &self.gbuffer);
    }

    /// Cast the cards into the tile of the cascade whose view is bound already.
    pub(crate) fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.record(pass, &self.cast);
    }

    fn instances(&self) -> u32 {
        self.shown.iter().map(|run| run.end - run.start).sum()
    }
}

/// The species of no size, which draws nothing.
const NONE: CardSpecies = CardSpecies {
    centre: [0.0; 3],
    radius: 0.0,
    slot: None,
};

impl ViewportRenderer {
    /// The lists of far stand-ins there are, uploaded once; none are shown until [`Self::show_card_lists`].
    pub fn set_card_lists(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lists: &[CardBatch],
    ) {
        self.world.cards.set_lists(device, queue, lists);
    }

    /// The species the records name, by their id.
    pub fn set_card_species(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        species: &[CardSpecies],
    ) {
        self.world.cards.set_species(device, queue, species);
    }

    /// What the stand-ins take of the visual settings.
    pub fn set_card_look(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, look: CardLook) {
        self.world.cards.set_look(device, queue, look);
    }

    /// Which of the lists last set draw this frame, by their place in that list.
    pub fn show_card_lists(&mut self, shown: &[u32]) {
        self.world.cards.show(shown);
    }

    /// Render the 64 frames of a species from `meshes` into atlas slot `slot`; false where the device holds no such slot.
    pub fn bake_card_slot(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: u32,
        centre: [f32; 3],
        radius: f32,
        meshes: &[CardMesh<'_>],
    ) -> bool {
        let cards = &mut self.world.cards;
        let baked = (cards.bake).bake(device, queue, slot, (Vec3::from(centre), radius), meshes);
        cards.regroup(device);
        baked
    }

    /// How many slots the atlas can hold on this device.
    pub fn card_slots(&self, device: &wgpu::Device) -> u32 {
        bake::slots(device)
    }

    /// How many card instances the shown lists hold.
    pub fn card_instances(&self) -> u32 {
        self.world.cards.instances()
    }
}

#[cfg(test)]
#[path = "cards_tests.rs"]
mod tests;
