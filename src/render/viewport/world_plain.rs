//! A world drawn through the seam in the renderer's plain pass: whatever session feeds it, each
//! model's levels go to pages of the kind the GTA VI world's do, each mesh draws with one surface
//! of a colour map, and the frame is lit by one sun the session's look states.
//!
//! ⚠ No texture of GTA V states its colour space, so the space is the slot's: a map a surface asks
//! for as its colour is read as sRGB, as the game's own programs read a diffuse map.
//!
//! ⚠ No normal map is bound: how a title packs one is its programs' to say, and a map read the
//! wrong way lights every surface wrongly, where the vertex normal lights it plainly.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use glam::{Mat4, Vec3};
use or_renderer::world::{WorldConstants, WorldFamily, WorldSurface, WORLD_MAPS};
use or_renderer::{
    Frustum, ShadingMode, WorldBucket, WorldDraw, WorldInstance, WorldMaterial, WorldMesh,
};
use or_world::model::{Alpha, AssetKey, Surface, TextureAsk, TextureFormat};
use or_world::streaming::NodeState;

use super::budget::{may_take, taking, Budgets};
use super::layout::{level, order};
use super::pages::{Pages, Place};
use super::textures::upload;
use super::{RenderBridge, WorldDrawn};
use crate::state::gta6_world::worldview::CpuTexture;
use crate::state::world_session::{ArrivedModel, WorldFeed};

/// How many frames a model or a texture goes unused before it may be let go.
const IDLE_FRAMES: u64 = 180;
/// The radius an instance that states none is culled by, in metres.
const LEAST_RADIUS: f32 = 1.0;
/// The longest side a colour map is asked for at, by how far the nearest thing it draws stands: under each distance in metres, that many pixels.
const PIXELS_BY_DISTANCE: [(f32, u32); 4] = [(30.0, 1024), (90.0, 512), (300.0, 256), (900.0, 128)];
/// The longest side asked for past the last of those distances.
const FAR_PIXELS: u32 = 64;

/// A texture by its scope and its name.
type MapKey = (AssetKey, u64);

struct Mesh {
    place: Place,
    material: u32,
    /// The mesh as the renderer draws it, and the making of its material it was built with.
    drawn: Option<(Arc<WorldMesh>, u64)>,
}

struct Lod {
    until: f32,
    meshes: Vec<Mesh>,
}

enum Slot {
    Ready {
        lods: Vec<Lod>,
        bytes: u64,
        /// The frame a selection last held it in.
        held: u64,
    },
    Missing,
}

struct Map {
    /// On the device; `None` for one that did not read or did not go there.
    view: Option<wgpu::TextureView>,
    bytes: u64,
    /// The size it was asked for at, and whether what came is the largest there is.
    got: u32,
    top: bool,
    used: u64,
    /// Counted up each time it comes to the device.
    stamp: u64,
}

struct Material {
    colour: Option<MapKey>,
    bucket: WorldBucket,
    /// What it draws with, and the stamp of its colour map when it was made.
    made: Option<(WorldMaterial, u64)>,
    making: u64,
    /// The frame a run last drew with it, and how near the nearest of that frame stood.
    touched: u64,
    away: f32,
}

/// What a layout was made for: the session, the selection, the camera, what is on the device and the height of the screen.
type LaidFor = (u64, u64, [u32; 16], u64, u32);

/// A world fed through the seam, on the device.
#[derive(Default)]
pub(in crate::render::viewport) struct Plain {
    /// The session whose models and textures are held.
    of: Option<u64>,
    /// Whether a session asked for this path this frame.
    pub(in crate::render::viewport) shown: bool,
    models: HashMap<AssetKey, Slot>,
    model_bytes: u64,
    pages: Pages,
    maps: HashMap<MapKey, Map>,
    map_bytes: u64,
    /// The size each map is waited for at.
    asked: HashMap<MapKey, u32>,
    stamps: u64,
    materials: Vec<Material>,
    material_of: HashMap<(Option<MapKey>, WorldBucket), u32>,
    /// A mid grey of one pixel, made at the first layout; the inner `None` on a device that made none.
    grey: Option<Option<wgpu::TextureView>>,
    frame: u64,
    changes: u64,
    laid_for: Option<LaidFor>,
    laid: Vec<WorldInstance>,
    spheres: Vec<[f32; 4]>,
    /// The sun and the clear colour of the session's look, for the frame's end.
    look: Option<(Vec3, f32, [f32; 4])>,
    was: Option<Instant>,
}

fn bucket_of(alpha: Alpha) -> WorldBucket {
    match alpha {
        Alpha::Opaque => WorldBucket::Opaque,
        Alpha::Cutout(_) => WorldBucket::Cutout,
        Alpha::Blend => WorldBucket::Blend,
    }
}

/// The format the device reads a texture of `format` in, as sRGB where it is a colour.
fn device_format(format: TextureFormat) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as F;
    match format {
        TextureFormat::Rgba8 => F::Rgba8UnormSrgb,
        TextureFormat::Bc1 => F::Bc1RgbaUnormSrgb,
        TextureFormat::Bc3 => F::Bc3RgbaUnormSrgb,
        // One and two channels have no sRGB form: a colour stored so is read as it is.
        TextureFormat::Bc4 => F::Bc4RUnorm,
        TextureFormat::Bc5 => F::Bc5RgUnorm,
        TextureFormat::Bc7 => F::Bc7RgbaUnormSrgb,
    }
}

/// The longest side a colour map is asked for at where the nearest thing it draws stands `away` metres off.
fn pixels_at(away: f32) -> u32 {
    let near = PIXELS_BY_DISTANCE.iter().find(|(under, _)| away < *under);
    near.map_or(FAR_PIXELS, |(_, pixels)| *pixels)
}

/// How strongly a white sun lights the plain pass: a world's colour maps are made for a sun several times a model preview's key, and at that key a street at noon is drawn as dusk.
const SUN_STRENGTH: f32 = 1.5;

/// The light the plain pass's key is given for a sun of the colour `key`: none at night.
fn key_strength(key: [f32; 3]) -> f32 {
    let luminance = 0.2126 * key[0] + 0.7152 * key[1] + 0.0722 * key[2];
    SUN_STRENGTH * luminance.clamp(0.0, 2.0)
}

/// A linear colour as the bytes of a screen hold it, which is what the renderer's background is stated in.
fn shown(linear: [f32; 3]) -> [f32; 4] {
    let encode = |c: f32| match c.clamp(0.0, 1.0) {
        c if c <= 0.003_130_8 => c * 12.92,
        c => 1.055 * c.powf(1.0 / 2.4) - 0.055,
    };
    let [r, g, b] = linear.map(encode);
    [r, g, b, 1.0]
}

impl Plain {
    /// Have the next frame lay the world out again: the renderer no longer holds what the last layout gave it.
    pub(in crate::render::viewport) fn forget_layout(&mut self) {
        self.laid_for = None;
    }

    /// Let go of everything held of the session before, and hold `id`'s from here on; whether another session's was held or none.
    fn begin(&mut self, id: u64) -> bool {
        if self.of == Some(id) {
            return false;
        }
        *self = Self {
            of: Some(id),
            frame: self.frame,
            changes: self.changes + 1,
            ..Self::default()
        };
        true
    }

    fn material(&mut self, surface: &Surface) -> u32 {
        let colour = surface.colour.map(|ask| (ask.scope, ask.name));
        let key = (colour, bucket_of(surface.alpha));
        if let Some(found) = self.material_of.get(&key) {
            return *found;
        }
        let at = self.materials.len() as u32;
        self.materials.push(Material {
            colour,
            bucket: key.1,
            made: None,
            making: 0,
            touched: 0,
            away: f32::INFINITY,
        });
        self.material_of.insert(key, at);
        at
    }

    /// Put `model` in the pages; `None` where none of its meshes found room.
    fn put(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budgets: &Budgets,
        model: &ArrivedModel,
    ) -> Option<Slot> {
        let (mut lods, mut bytes) = (Vec::with_capacity(model.lods.len()), 0);
        for lod in &model.lods {
            let mut meshes = Vec::with_capacity(lod.meshes.len());
            for mesh in &lod.meshes {
                let put = (self.pages).put(device, queue, budgets, &mesh.vertices, &mesh.indices);
                let Some(place) = put else {
                    continue;
                };
                bytes += place.bytes();
                meshes.push(Mesh {
                    place,
                    material: self.material(&mesh.surface),
                    drawn: None,
                });
            }
            lods.push(Lod {
                until: lod.until,
                meshes,
            });
        }
        if bytes == 0 {
            return None;
        }
        self.model_bytes += bytes;
        Some(Slot::Ready {
            lods,
            bytes,
            held: self.frame,
        })
    }

    /// Hold what came for `ask`, its view with its bytes and its longest side, or that nothing came; the bytes that went to the device.
    fn took_map(&mut self, ask: &TextureAsk, made: Option<(wgpu::TextureView, u64, u32)>) -> u64 {
        let key = (ask.scope, ask.name);
        if self.asked.get(&key) == Some(&ask.pixels) {
            self.asked.remove(&key);
        }
        self.stamps += 1;
        let map = match made {
            Some((view, bytes, longest)) => Map {
                view: Some(view),
                bytes,
                got: ask.pixels,
                top: longest < ask.pixels,
                used: self.frame,
                stamp: self.stamps,
            },
            None => {
                // ⚠ A larger answer that did not come leaves the smaller one in place, for good: its bytes are counted already.
                if let Some(held) = self.maps.get_mut(&key).filter(|held| held.view.is_some()) {
                    held.top = true;
                    return 0;
                }
                Map {
                    view: None,
                    bytes: 0,
                    got: ask.pixels,
                    top: true,
                    used: self.frame,
                    stamp: self.stamps,
                }
            }
        };
        let bytes = map.bytes;
        self.map_bytes += bytes;
        if let Some(old) = self.maps.insert(key, map) {
            self.map_bytes -= old.bytes;
        }
        bytes
    }

    fn free(&mut self, model: AssetKey) {
        if let Some(Slot::Ready { lods, bytes, .. }) = self.models.remove(&model) {
            for mesh in lods.into_iter().flat_map(|lod| lod.meshes) {
                self.pages.free(mesh.place);
            }
            self.model_bytes -= bytes;
        }
    }

    /// Let the models no selection held for a while go until the rest fit `budget`, the longest unheld first; those that went.
    fn evict_models(&mut self, budget: u64) -> Vec<AssetKey> {
        if self.model_bytes <= budget {
            return Vec::new();
        }
        let before = self.frame.saturating_sub(IDLE_FRAMES);
        let mut idle: Vec<(u64, AssetKey)> = (self.models.iter())
            .filter_map(|(key, slot)| match slot {
                Slot::Ready { held, .. } if *held < before => Some((*held, *key)),
                _ => None,
            })
            .collect();
        idle.sort_unstable();
        let mut gone = Vec::new();
        for (_, key) in idle {
            if self.model_bytes <= budget {
                break;
            }
            self.free(key);
            gone.push(key);
        }
        gone
    }

    /// Let the maps no material sampled for a while go until the rest fit `budget`, the longest unused first.
    fn evict_maps(&mut self, budget: u64) -> usize {
        if self.map_bytes <= budget {
            return 0;
        }
        let before = self.frame.saturating_sub(IDLE_FRAMES);
        let mut idle: Vec<(u64, MapKey)> = (self.maps.iter())
            .filter(|(_, map)| map.view.is_some() && map.used < before)
            .map(|(key, map)| (map.used, *key))
            .collect();
        idle.sort_unstable();
        let mut gone = 0;
        for (_, key) in idle {
            if self.map_bytes <= budget {
                break;
            }
            if let Some(map) = self.maps.remove(&key) {
                self.map_bytes -= map.bytes;
                gone += 1;
            }
        }
        gone
    }
}

/// What one frame laid out of a selection: the instances seen, in runs a model and level.
struct Runs {
    /// Each run's model among the selection's, its level, where its instances start and end in the layout, and how near its nearest stands.
    runs: Vec<(u32, u32, std::ops::Range<u32>, f32)>,
    seen: usize,
}

impl RenderBridge {
    /// Draw the world `feed` hands this frame in the plain pass: put what arrived on the device, and lay out the instances of its last selection the camera sees.
    pub fn refresh_world_feed(&mut self, feed: &mut dyn WorldFeed) {
        let started = Instant::now();
        self.world.shown = true;
        self.mode = ShadingMode::Material;
        let bc = (self.device.features()).contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let budgets = *(self.world.budgets).get_or_insert_with(|| {
            let set = std::env::var(super::budget::MEMORY_VARIABLE).ok();
            Budgets::of(
                &self.device.limits(),
                super::budget::memory_set(set.as_deref()),
            )
        });
        if self.world.plain.begin(feed.id()) {
            // ⚠ What GTA VI's world left on the renderer is no part of this one: its far stand-ins and its water draw in the plain pass too.
            self.world.hide_cards(&mut self.renderer);
            (self.world.water).refresh(&mut self.renderer, &self.device, None, Vec3::ZERO);
        }
        let plain = &mut self.world.plain;
        plain.shown = true;
        plain.frame += 1;
        let frame = plain.frame;
        let facts = feed.facts();
        plain.look = Some((facts.sun, key_strength(facts.key), shown(facts.clear)));
        let now = Instant::now();
        let (time, more) = taking(plain.was.map(|then| now.duration_since(then)));
        plain.was = Some(now);

        // The textures first, for half the time: one completes a model already drawn.
        let taken = Instant::now();
        let mut uploaded = 0u64;
        while uploaded < budgets.textures_a_frame * more && may_take(taken, uploaded, time / 2) {
            let Some((ask, read)) = feed.take_texture() else {
                break;
            };
            let made = read.as_ref().and_then(|texture| {
                let cpu = CpuTexture {
                    width: texture.width,
                    height: texture.height,
                    format: device_format(texture.format),
                    levels: texture.levels.clone(),
                };
                let (view, bytes) = upload(&self.device, &self.queue, &cpu, bc, budgets.most_side)?;
                Some((view, bytes, texture.width.max(texture.height)))
            });
            uploaded += plain.took_map(&ask, made);
            plain.changes += 1;
        }
        let mut uploaded = 0u64;
        while uploaded < budgets.models_a_frame * more && may_take(taken, uploaded, time) {
            let Some((model, read)) = feed.take_model() else {
                break;
            };
            plain.free(model);
            let before = plain.model_bytes;
            let slot = read
                .as_ref()
                .and_then(|read| plain.put(&self.device, &self.queue, &budgets, read));
            uploaded += plain.model_bytes - before;
            feed.tell(
                model,
                match slot {
                    Some(_) => NodeState::Ready,
                    None => NodeState::Absent,
                },
            );
            plain.models.insert(model, slot.unwrap_or(Slot::Missing));
            plain.changes += 1;
        }

        let camera = feed.camera();
        let (width, height) = self.desired_size;
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        let view_proj = camera.view_proj(aspect);
        let Some(selection) = feed.selection() else {
            if self.renderer.has_world() {
                (self.renderer).set_world(&self.device, &self.queue, &[], Vec::new());
            }
            plain.laid_for = None;
            self.world.drawn = WorldDrawn::default();
            return;
        };
        // Nothing moved, arrived or left: the device holds this frame's instances already.
        let key = (
            feed.id(),
            selection.version,
            view_proj.to_cols_array().map(f32::to_bits),
            plain.changes,
            height,
        );
        if plain.laid_for == Some(key) {
            self.world.cost.refresh = started.elapsed();
            return;
        }
        plain.laid_for = Some(key);
        let instances = &selection.instances;
        let runs = plain.lay(instances, view_proj, camera.position);

        // Each material a run draws with, with how near it is seen.
        for (model, level, _, away) in &runs.runs {
            let Some(Slot::Ready { lods, .. }) =
                (instances.models.get(*model as usize)).and_then(|key| plain.models.get(key))
            else {
                continue;
            };
            let Some(lod) = lods.get(*level as usize) else {
                continue;
            };
            for mesh in &lod.meshes {
                let material = &mut plain.materials[mesh.material as usize];
                if material.touched != frame {
                    (material.touched, material.away) = (frame, *away);
                } else {
                    material.away = material.away.min(*away);
                }
            }
        }
        let grey = plain
            .grey
            .get_or_insert_with(|| {
                let grey = CpuTexture {
                    width: 1,
                    height: 1,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    levels: vec![vec![128, 128, 128, 255]],
                };
                upload(&self.device, &self.queue, &grey, bc, budgets.most_side)
                    .map(|(view, _)| view)
            })
            .clone();
        // ⚠ Made once: a device that takes no texture of one pixel draws what has no map as white.
        let mut wanted: Vec<(f32, TextureAsk)> = Vec::new();
        let mut asked_now: HashSet<MapKey> = HashSet::new();
        for material in &mut plain.materials {
            if material.touched != frame {
                continue;
            }
            let pixels = pixels_at(material.away);
            let (mut stamp, mut view) = (0, None);
            if let Some(key) = material.colour {
                let mut ask = |pixels: u32| {
                    // ⚠ Held to the size it is waited for at: an ask that grew with every step of the camera would be read once a step.
                    let pixels = plain.asked.get(&key).copied().unwrap_or(pixels);
                    if asked_now.insert(key) {
                        wanted.push((
                            material.away,
                            TextureAsk {
                                scope: key.0,
                                name: key.1,
                                pixels,
                            },
                        ));
                    }
                };
                match plain.maps.get_mut(&key) {
                    Some(map) => {
                        map.used = frame;
                        (stamp, view) = (map.stamp, map.view.clone());
                        if map.view.is_some() && !map.top && pixels > map.got {
                            ask(pixels);
                        }
                    }
                    None => ask(pixels),
                }
            }
            if (material.made.as_ref()).is_none_or(|(_, made_with)| *made_with != stamp) {
                // A colour map that is not on the device is a mid grey, never the white of no map.
                let colour = match material.colour {
                    Some(_) => view.as_ref().or(grey.as_ref()),
                    None => None,
                };
                let mut maps = [None; WORLD_MAPS];
                maps[or_renderer::world::slot::colour(0)] = colour;
                let surface = WorldSurface {
                    family: WorldFamily::Standard,
                    bucket: material.bucket,
                    maps,
                    constants: WorldConstants::default(),
                };
                let made = (self.renderer).world_surface(&self.device, &surface);
                material.made = Some((made, stamp));
                material.making += 1;
            }
        }
        plain.asked = (wanted.iter())
            .map(|(_, ask)| ((ask.scope, ask.name), ask.pixels))
            .collect();
        feed.want_textures(wanted);

        // A draw for each mesh of each run, in the order the renderer sends most of them together.
        let mut keyed = Vec::with_capacity(runs.runs.len());
        for (model, level, range, away) in &runs.runs {
            let Some(named) = instances.models.get(*model as usize) else {
                continue;
            };
            let Some(Slot::Ready { lods, held, .. }) = plain.models.get_mut(named) else {
                continue;
            };
            *held = frame;
            let Some(lod) = lods.get_mut(*level as usize) else {
                continue;
            };
            for mesh in &mut lod.meshes {
                let material = &plain.materials[mesh.material as usize];
                let Some((made, _)) = &material.made else {
                    continue;
                };
                if (mesh.drawn.as_ref()).is_none_or(|(_, making)| *making != material.making) {
                    let Some((vertices, indices)) = plain.pages.buffers(mesh.place.page) else {
                        continue;
                    };
                    let shared = WorldMesh::shared(
                        vertices.clone(),
                        indices.clone(),
                        mesh.place.base_vertex as i32,
                        mesh.place.first_index,
                        mesh.place.indices,
                        made.clone(),
                    );
                    mesh.drawn = Some((Arc::new(shared), material.making));
                }
                let Some((drawn, _)) = &mesh.drawn else {
                    continue;
                };
                keyed.push((
                    order(material.bucket, *away, mesh.place.page, made.key()),
                    WorldDraw {
                        mesh: Arc::clone(drawn),
                        instances: range.clone(),
                    },
                ));
            }
        }
        keyed.sort_unstable_by_key(|(key, _)| *key);
        let draws: Vec<WorldDraw> = keyed.into_iter().map(|(_, draw)| draw).collect();
        // A model the selection names is held though the camera looks away from it.
        for named in &instances.models {
            if let Some(Slot::Ready { held, .. }) = plain.models.get_mut(named) {
                *held = frame;
            }
        }

        let (pages, page_bytes) = plain.pages.held();
        let on_device = plain.maps.values().filter(|map| map.view.is_some()).count();
        self.world.drawn = WorldDrawn {
            selected: instances.len(),
            seen: runs.seen,
            draws: draws.len(),
            models: plain.models.len(),
            textures: on_device,
            textures_unread: plain.maps.len() - on_device,
            bytes: page_bytes + plain.map_bytes,
            pages,
            lit_by_the_game: false,
        };
        (self.renderer).set_world(&self.device, &self.queue, &plain.laid, draws);
        self.renderer.set_world_spheres(&plain.spheres);
        for model in plain.evict_models(budgets.models_kept) {
            feed.tell(model, NodeState::Loading);
            plain.changes += 1;
        }
        if plain.evict_maps(budgets.textures_kept) > 0 {
            plain.changes += 1;
        }
        self.world.cost.refresh = started.elapsed();
    }

    /// Light the plain pass by the session's sun and clear to its sky's colour, where a session drew through the seam this frame.
    pub(in crate::render::viewport) fn light_world_feed(&mut self) {
        if let (true, Some((sun, strength, clear))) =
            (self.world.plain.shown, self.world.plain.look)
        {
            self.renderer.set_key_light(sun, strength);
            self.renderer.set_background(clear);
            self.renderer.set_sky_background(false);
        }
    }

    /// Have the next frame lay a fed world out again, as though the camera had moved.
    pub fn relay_world_feed(&mut self) {
        self.world.plain.forget_layout();
    }

    /// How many times a model or a texture of a fed world came to the device or left it.
    pub fn world_feed_changes(&self) -> u64 {
        self.world.plain.changes
    }
}

impl Plain {
    /// Cull `instances` to what `view_proj` sees from `eye`, each at the level its distance draws, laid a run a model and level.
    fn lay(&mut self, instances: &or_world::model::Instances, view_proj: Mat4, eye: Vec3) -> Runs {
        let frustum = Frustum::of(view_proj);
        // The distances each model's levels draw under, for those on the device.
        let untils: Vec<Option<Vec<f32>>> = (instances.models.iter())
            .map(|key| match self.models.get(key) {
                Some(Slot::Ready { lods, .. }) => Some(lods.iter().map(|lod| lod.until).collect()),
                _ => None,
            })
            .collect();
        let mut seen: Vec<(u64, f32, u32)> = Vec::new();
        for (at, (model, sphere)) in instances.model.iter().zip(&instances.sphere).enumerate() {
            let Some(Some(untils)) = untils.get(*model as usize) else {
                continue;
            };
            let centre = Vec3::new(sphere[0], sphere[1], sphere[2]);
            let radius = sphere[3].max(LEAST_RADIUS);
            if !frustum.sees_sphere(centre, radius) {
                continue;
            }
            let away = (centre.distance(eye) - radius).max(0.0);
            let level = level(untils, away, 1.0) as u64;
            seen.push((u64::from(*model) << 8 | level.min(255), away, at as u32));
        }
        seen.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        self.laid.clear();
        self.spheres.clear();
        let mut runs: Vec<(u32, u32, std::ops::Range<u32>, f32)> = Vec::new();
        for (key, away, at) in &seen {
            let (model, level) = ((key >> 8) as u32, (key & 255) as u32);
            let here = self.laid.len() as u32;
            match runs.last_mut() {
                Some((m, l, range, _)) if (*m, *l) == (model, level) => range.end = here + 1,
                _ => runs.push((model, level, here..here + 1, *away)),
            }
            let at = *at as usize;
            self.laid.push(WorldInstance {
                rows: instances.rows[at],
                tint: [1.0, 1.0, 1.0, instances.fade[at]],
                ambient: or_renderer::world::OUTDOORS,
                cast_only: 0.0,
            });
            self.spheres.push(instances.sphere[at]);
        }
        Runs {
            runs,
            seen: seen.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_map_is_read_as_srgb_wherever_its_format_has_such_a_form() {
        for format in [
            TextureFormat::Rgba8,
            TextureFormat::Bc1,
            TextureFormat::Bc3,
            TextureFormat::Bc7,
        ] {
            assert!(device_format(format).is_srgb(), "{format:?}");
        }
        for format in [TextureFormat::Bc4, TextureFormat::Bc5] {
            assert!(!device_format(format).is_srgb(), "{format:?}");
        }
    }

    #[test]
    fn a_map_is_asked_for_smaller_the_farther_it_is_seen() {
        let sizes: Vec<u32> = [0.0, 29.9, 30.0, 299.0, 899.0, 5000.0]
            .into_iter()
            .map(pixels_at)
            .collect();
        assert_eq!(sizes, [1024, 1024, 512, 256, 128, FAR_PIXELS]);
    }

    #[test]
    fn a_white_sun_lights_at_the_worlds_strength_and_night_not_at_all() {
        assert_eq!(key_strength([1.0; 3]), SUN_STRENGTH);
        assert_eq!(key_strength([0.0; 3]), 0.0);
        // A clear colour is stated to the renderer as a screen holds it: a linear mid grey is brighter than half there.
        let grey = shown([0.5; 3]);
        assert!((grey[0] - 0.7354).abs() < 1e-3, "{grey:?}");
        assert_eq!(
            (shown([0.0; 3]), shown([1.0; 3])[3]),
            ([0.0, 0.0, 0.0, 1.0], 1.0)
        );
    }

    #[test]
    fn instances_are_laid_a_run_a_model_and_level_and_what_the_camera_does_not_see_is_left_out() {
        use or_world::model::{rows_at, Instance, Instances};
        let mut plain = Plain::default();
        let lods = |untils: &[f32]| Slot::Ready {
            lods: (untils.iter())
                .map(|until| Lod {
                    until: *until,
                    meshes: Vec::new(),
                })
                .collect(),
            bytes: 0,
            held: 0,
        };
        plain.models.insert(AssetKey(7), lods(&[50.0, 500.0]));
        plain.models.insert(AssetKey(9), lods(&[9000.0]));
        let mut instances = Instances::of_width(0);
        // Along +Y ahead of a camera at the origin: near and far of one model, one of another, one behind, and one of a model not on the device.
        for (node, model, y) in [
            (0, 7, 20.0),
            (1, 9, 100.0),
            (2, 7, 200.0),
            (3, 7, -100.0),
            (4, 5, 30.0),
            (5, 7, 300.0),
        ] {
            instances.push(Instance {
                node: (0, node),
                model: AssetKey(model),
                rows: rows_at(Vec3::new(0.0, y, 0.0)),
                sphere: [0.0, y, 0.0, 2.0],
                fade: 1.0 - node as f32 * 0.1,
                words: &[],
            });
        }
        let camera = or_renderer::FlyCamera {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 1.0,
            fov_y: 60f32.to_radians(),
            near: 0.5,
            far: 5000.0,
        };
        let runs = plain.lay(&instances, camera.view_proj(1.0), Vec3::ZERO);
        assert_eq!(
            runs.seen, 4,
            "the one behind and the one not on the device are left out"
        );
        let stated: Vec<(u32, u32, std::ops::Range<u32>)> = (runs.runs.iter())
            .map(|(model, level, range, _)| (*model, *level, range.clone()))
            .collect();
        // Model 7 is the first the selection names and 9 the second; 7 draws its near level once and its far level twice.
        assert_eq!(stated, [(0, 0, 0..1), (0, 1, 1..3), (1, 0, 3..4)]);
        let fades: Vec<f32> = plain.laid.iter().map(|laid| laid.tint[3]).collect();
        assert_eq!(
            fades,
            [1.0, 0.8, 0.5, 0.9],
            "each keeps its own fade, the nearer of a run first"
        );
        assert_eq!(plain.spheres.len(), plain.laid.len());
    }

    #[test]
    fn a_larger_map_that_did_not_come_leaves_the_smaller_one_counted_once() {
        const WHAT: &str = "a_larger_map_that_did_not_come_leaves_the_smaller_one_counted_once";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let pixel = CpuTexture {
            width: 1,
            height: 1,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            levels: vec![vec![128, 128, 128, 255]],
        };
        let (view, bytes) = upload(&device, &queue, &pixel, false, 4096).expect("one pixel");
        let ask = |pixels| TextureAsk {
            scope: AssetKey(3),
            name: 5,
            pixels,
        };
        let key = (AssetKey(3), 5);
        let mut plain = Plain::default();
        plain.asked.insert(key, 64);
        // Asked at 64 a side and come at 64: a larger one may exist.
        assert_eq!(plain.took_map(&ask(64), Some((view, bytes, 64))), bytes);
        assert!(bytes > 0 && plain.map_bytes == bytes && plain.asked.is_empty());
        assert!(!plain.maps[&key].top);
        // The larger did not read: the smaller stays, is the largest there will be, and weighs what it did.
        assert_eq!(plain.took_map(&ask(256), None), 0);
        let held = &plain.maps[&key];
        assert!(held.view.is_some() && held.top && held.got == 64);
        assert_eq!(plain.map_bytes, bytes);
        // One that never read is held as nothing, so that it is not asked for again.
        let other = TextureAsk { name: 6, ..ask(64) };
        assert_eq!(plain.took_map(&other, None), 0);
        let never = &plain.maps[&(AssetKey(3), 6)];
        assert!(never.view.is_none() && never.top);
        assert_eq!(plain.map_bytes, bytes);
        assert_eq!(plain.evict_maps(0), 0, "a map used this frame is kept");
        plain.frame += IDLE_FRAMES + 2;
        assert_eq!(plain.evict_maps(0), 1);
        assert_eq!(plain.map_bytes, 0);
    }

    #[test]
    fn the_models_longest_unheld_are_let_go_until_the_rest_fit_and_one_held_lately_stays() {
        let mut plain = Plain::default();
        let slot = |bytes, held| Slot::Ready {
            lods: Vec::new(),
            bytes,
            held,
        };
        plain.frame = 1000;
        for (key, held) in [(1, 10), (2, 5), (3, 990), (4, 700)] {
            plain.models.insert(AssetKey(key), slot(100, held));
            plain.model_bytes += 100;
        }
        plain.models.insert(AssetKey(9), Slot::Missing);
        assert_eq!(plain.evict_models(400), [], "what fits is kept");
        // The longest unheld first, and no more than the budget asks.
        assert_eq!(plain.evict_models(250), [AssetKey(2), AssetKey(1)]);
        assert_eq!(plain.model_bytes, 200);
        // The one a selection held inside the idle frames stays whatever the budget.
        assert_eq!(plain.evict_models(0), [AssetKey(4)]);
        assert!(plain.models.contains_key(&AssetKey(3)) && plain.model_bytes == 100);
        assert!(matches!(plain.models[&AssetKey(9)], Slot::Missing));
    }
}
