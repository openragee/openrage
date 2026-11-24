//! The world viewer on the device: each model's meshes and each texture kept while they draw, and
//! each frame's instances culled to what the camera sees and laid out a run a model.
//!
//! ⚠ What draws is selected on the world's own thread: a frame reads the last selection and
//! never waits for one, and lays it out again only when the camera, the selection or what is on
//! the device changed.

use std::collections::HashMap;
use std::sync::Arc;

use or_renderer::{Frustum, ShadingMode, WorldDraw, WorldInstance, WorldMesh};
use rayon::prelude::*;

use super::RenderBridge;
use crate::state::gta6_worldview::index::ModelRef;
use crate::state::gta6_worldview::CpuTexture;

/// The bytes of models, and of textures, put on the device in one frame; the rest wait for the next.
const MODELS_A_FRAME: u64 = 48 << 20;
const TEXTURES_A_FRAME: u64 = 8 << 20;
/// The bytes of models, and of textures, kept on the device, past which those drawn longest ago go.
// ponytail: fixed budgets; the device's own memory once a card with less shows it.
const MODELS_KEPT: u64 = 3 << 30;
const TEXTURES_KEPT: u64 = 1 << 30;

/// One mesh of a model, with the key of the texture that colours it and whether it holds it yet.
struct Mesh {
    mesh: Arc<WorldMesh>,
    key: Option<u64>,
    textured: bool,
}

/// A model on the device, or one that did not read.
enum Slot {
    Ready {
        meshes: Vec<Mesh>,
        bytes: u64,
        /// The frame it last drew in.
        drawn: u64,
    },
    Missing,
}

/// A texture on the device, or one no pack holds.
enum Texture {
    Ready {
        group: Arc<wgpu::BindGroup>,
        bytes: u64,
        /// The frame a model that draws last asked for it in.
        used: u64,
    },
    Missing,
}

/// What the world viewer drew last, for the screen to say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldDrawn {
    /// Instances the selection gave, and those of them the camera sees.
    pub selected: usize,
    pub seen: usize,
    pub draws: usize,
    pub models: usize,
    /// Textures on the device, and those no pack holds or that did not read.
    pub textures: usize,
    pub textures_unread: usize,
    pub bytes: u64,
    /// Whether the game's own lighting passes light them.
    pub lit_by_the_game: bool,
}

#[derive(Default)]
pub(super) struct WorldGpu {
    models: HashMap<ModelRef, Slot>,
    model_bytes: u64,
    textures: HashMap<u64, Texture>,
    texture_bytes: u64,
    frame: u64,
    /// Whether the world panel asked for the world this frame.
    pub(super) shown: bool,
    /// Counted up whenever a model or a texture comes to the device or leaves it.
    changes: u64,
    /// The models that came, will never come (`None`) or left (`false`), not yet told to the world's thread.
    told: Vec<(ModelRef, Option<bool>)>,
    /// What the instances on the device were laid out for: the selection, the camera and the changes.
    laid_for: Option<(u64, [u32; 16], u64)>,
    /// The instances the camera sees, by their place in the selection, and where each model's run starts and ends.
    visible: Vec<u32>,
    starts: Vec<u32>,
    ends: Vec<u32>,
    laid: Vec<WorldInstance>,
    /// What a mesh whose texture did not read is drawn with: a mid grey, never the white of no texture.
    grey: Option<Arc<wgpu::BindGroup>>,
    drawn: WorldDrawn,
}

impl WorldGpu {
    /// Let the models and the textures drawn longest ago go until the rest fit their budgets.
    fn evict(&mut self) {
        let now = self.frame;
        if self.model_bytes > MODELS_KEPT {
            let mut idle: Vec<(u64, ModelRef, u64)> = (self.models.iter())
                .filter_map(|(model, slot)| match slot {
                    Slot::Ready { bytes, drawn, .. } if *drawn < now => {
                        Some((*drawn, *model, *bytes))
                    }
                    _ => None,
                })
                .collect();
            idle.sort_unstable_by_key(|(drawn, ..)| *drawn);
            for (_, model, bytes) in idle {
                if self.model_bytes <= MODELS_KEPT {
                    break;
                }
                self.models.remove(&model);
                self.model_bytes -= bytes;
                self.told.push((model, Some(false)));
                self.changes += 1;
            }
        }
        if self.texture_bytes > TEXTURES_KEPT {
            let mut idle: Vec<(u64, u64, u64)> = (self.textures.iter())
                .filter_map(|(key, texture)| match texture {
                    Texture::Ready { bytes, used, .. } if *used < now => {
                        Some((*used, *key, *bytes))
                    }
                    _ => None,
                })
                .collect();
            idle.sort_unstable_by_key(|(used, ..)| *used);
            for (_, key, bytes) in idle {
                if self.texture_bytes <= TEXTURES_KEPT {
                    break;
                }
                // ⚠ A mesh that holds it keeps it until its model goes: only the cache lets it go.
                self.textures.remove(&key);
                self.texture_bytes -= bytes;
            }
        }
    }
}

/// `texture` as the bind group a mesh's material samples, every level of it, and the bytes it holds.
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    texture: &CpuTexture,
) -> (wgpu::BindGroup, u64) {
    let format = match texture.srgb {
        true => wgpu::TextureFormat::Rgba8UnormSrgb,
        false => wgpu::TextureFormat::Rgba8Unorm,
    };
    let size = |level: u32| wgpu::Extent3d {
        width: (texture.width >> level).max(1),
        height: (texture.height >> level).max(1),
        depth_or_array_layers: 1,
    };
    let on_device = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("world-texture"),
        size: size(0),
        mip_level_count: texture.levels.len().max(1) as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut bytes = 0;
    for (level, pixels) in texture.levels.iter().enumerate() {
        let extent = size(level as u32);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &on_device,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(extent.width * 4),
                rows_per_image: Some(extent.height),
            },
            extent,
        );
        bytes += pixels.len() as u64;
    }
    let view = on_device.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("world-texture-sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        anisotropy_clamp: 8,
        ..Default::default()
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("world-texture-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (group, bytes)
}

impl RenderBridge {
    /// What the world viewer drew last.
    pub fn world_drawn(&self) -> WorldDrawn {
        self.world.drawn
    }

    /// Draw the GTA VI world this frame: put the models and textures that arrived on the device,
    /// and lay out the instances of the last selection the camera sees.
    pub fn refresh_world(&mut self) {
        self.world.shown = true;
        self.world.frame += 1;
        let frame = self.world.frame;
        // Lit by the game's own passes where its pack is read: the frame is made with no surface of its own.
        self.mode = ShadingMode::Shaded;
        if let (Some(pack), false) = (self.game_pack.clone(), self.renderer.has_deferred()) {
            let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let prepared = (self.renderer).prepare_deferred(&self.device, &self.queue, &pack, &[]);
            if pollster::block_on(validation.pop()).is_some() || prepared.is_err() {
                self.renderer.reset_deferred_pipelines();
            }
        }
        let mut viewer = crate::state::gta6_worldview::viewer();
        let world = &mut self.world;
        let mut uploaded = 0u64;
        while uploaded < MODELS_A_FRAME {
            let Some((model, meshes)) = viewer.take_model() else {
                break;
            };
            let slot = match meshes {
                Some(meshes) => {
                    let meshes: Vec<Mesh> = (meshes.iter())
                        .map(|m| Mesh {
                            mesh: Arc::new(WorldMesh::new(
                                &self.device,
                                &m.vertices,
                                &m.indices,
                                None,
                            )),
                            key: m.texture,
                            textured: false,
                        })
                        .collect();
                    let bytes = meshes.iter().map(|m| m.mesh.bytes()).sum();
                    uploaded += bytes;
                    world.model_bytes += bytes;
                    world.told.push((model, Some(true)));
                    Slot::Ready {
                        meshes,
                        bytes,
                        drawn: frame,
                    }
                }
                None => {
                    world.told.push((model, None));
                    Slot::Missing
                }
            };
            world.changes += 1;
            if let Some(Slot::Ready { bytes, .. }) = world.models.insert(model, slot) {
                world.model_bytes -= bytes;
            }
        }
        let mut uploaded = 0u64;
        while uploaded < TEXTURES_A_FRAME {
            let Some((key, texture)) = viewer.take_texture() else {
                break;
            };
            let texture = match texture {
                Some(texture) => {
                    let (group, bytes) =
                        upload(&self.device, &self.queue, &self.material_bgl, &texture);
                    uploaded += bytes;
                    world.texture_bytes += bytes;
                    Texture::Ready {
                        group: Arc::new(group),
                        bytes,
                        used: frame,
                    }
                }
                None => Texture::Missing,
            };
            world.changes += 1;
            if let Some(Texture::Ready { bytes, .. }) = world.textures.insert(key, texture) {
                world.texture_bytes -= bytes;
            }
        }

        let selected = viewer.selected(&mut world.told);
        let (width, height) = self.desired_size;
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        let view_proj = viewer.camera.view_proj(aspect);
        drop(viewer);
        let Some(selected) = selected else {
            if self.renderer.has_world() {
                (self.renderer).set_world(&self.device, &self.queue, &[], Vec::new());
            }
            world.laid_for = None;
            world.drawn = WorldDrawn::default();
            return;
        };
        // Nothing moved, arrived or left: the device holds this frame's instances already.
        let key = (
            selected.version,
            view_proj.to_cols_array().map(f32::to_bits),
            world.changes,
        );
        if world.laid_for == Some(key) {
            return;
        }
        world.laid_for = Some(key);

        // The chunks the camera sees, then the instances of those it sees, counted a model.
        // ⚠ On every core: a selection holds tens of thousands of instances, and a frame that tested
        // each on the UI thread spent most of its time here.
        let frustum = Frustum::of(view_proj);
        let instances = &selected.instances;
        let sees = |at: &u32| {
            let instance = &instances[*at as usize];
            frustum.sees_sphere(instance.centre, instance.radius.max(0.01))
        };
        world.visible = (selected.chunks.par_iter())
            .flat_map_iter(|chunk| {
                // A chunk wholly in view is taken whole, and one out of view not at all.
                let whole = frustum.holds_sphere(chunk.centre, chunk.radius);
                let part = whole || frustum.sees_sphere(chunk.centre, chunk.radius);
                (chunk.from..chunk.to).filter(move |at| part && (whole || sees(at)))
            })
            .collect();
        world.starts.clear();
        world.starts.resize(selected.models.len(), 0);
        for at in &world.visible {
            world.starts[instances[*at as usize].model as usize] += 1;
        }
        // Each model's instances side by side: where its run starts, then each put at its run's end.
        let mut total = 0;
        for start in &mut world.starts {
            total += std::mem::replace(start, total);
        }
        world.ends.clone_from(&world.starts);
        world.laid.clear();
        world.laid.resize(
            total as usize,
            WorldInstance {
                rows: [[0.0; 4]; 3],
                tint: [1.0; 4],
            },
        );
        for at in &world.visible {
            let instance = &selected.instances[*at as usize];
            let end = &mut world.ends[instance.model as usize];
            world.laid[*end as usize].rows = instance.rows;
            *end += 1;
        }

        let grey = Arc::clone(world.grey.get_or_insert_with(|| {
            let pixel = CpuTexture {
                width: 1,
                height: 1,
                srgb: true,
                levels: vec![vec![128, 128, 128, 255]],
            };
            Arc::new(upload(&self.device, &self.queue, &self.material_bgl, &pixel).0)
        }));
        let (mut draws, mut wanted) = (Vec::new(), Vec::new());
        for (at, model) in selected.models.iter().enumerate() {
            let range = world.starts[at]..world.ends[at];
            if range.is_empty() {
                continue;
            }
            let Some(Slot::Ready { meshes, drawn, .. }) = world.models.get_mut(model) else {
                continue;
            };
            *drawn = frame;
            for mesh in meshes {
                // A mesh takes its texture the first frame it draws after the texture arrived.
                match mesh.key.map(|key| (key, world.textures.get_mut(&key))) {
                    Some((_, Some(Texture::Ready { group, used, .. }))) => {
                        *used = frame;
                        if !mesh.textured {
                            mesh.mesh = Arc::new(mesh.mesh.with_texture(Arc::clone(group)));
                            mesh.textured = true;
                        }
                    }
                    Some((_, Some(Texture::Missing))) if !mesh.textured => {
                        mesh.mesh = Arc::new(mesh.mesh.with_texture(Arc::clone(&grey)));
                        mesh.textured = true;
                    }
                    Some((key, None)) if !mesh.textured => wanted.push((0.0, key)),
                    _ => {}
                }
                draws.push(WorldDraw {
                    mesh: Arc::clone(&mesh.mesh),
                    instances: range.clone(),
                });
            }
        }
        wanted.sort_unstable_by_key(|(_, key)| *key);
        wanted.dedup_by_key(|(_, key)| *key);
        crate::state::gta6_worldview::viewer().want_textures(wanted);
        world.drawn = WorldDrawn {
            selected: selected.instances.len(),
            seen: world.visible.len(),
            draws: draws.len(),
            models: world.models.len(),
            textures: (world.textures.values())
                .filter(|t| matches!(t, Texture::Ready { .. }))
                .count(),
            textures_unread: (world.textures.values())
                .filter(|t| matches!(t, Texture::Missing))
                .count(),
            bytes: world.model_bytes + world.texture_bytes,
            lit_by_the_game: self.renderer.has_deferred(),
        };
        (self.renderer).set_world(&self.device, &self.queue, &world.laid, draws);
        world.evict();
    }

    /// Take the world away where no panel asked for it this frame.
    pub(super) fn end_world(&mut self) {
        if !self.world.shown && self.renderer.has_world() {
            (self.renderer).set_world(&self.device, &self.queue, &[], Vec::new());
            self.world.laid_for = None;
        }
    }
}
