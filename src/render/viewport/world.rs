//! The world viewer on the device: each model's levels of detail in pages of vertices and indices
//! that models share, each texture and each material kept while they draw, and each frame's
//! instances culled to what the camera sees and laid out a run a model and level.
//!
//! ⚠ What draws is selected on the world's own thread: a frame reads the last selection and
//! never waits for one, and lays it out again only when the camera, the selection or what is on
//! the device changed.
//!
//! ⚠ A page's bytes are written again once a model leaves it. Whatever puts a model on the
//! device or takes one off counts `changes` up, so that the frame is laid out again before the
//! renderer draws a run of the old bytes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use glam::Vec3;
use or_renderer::world::{WorldConstants, WorldFamily, WorldSurface, WORLD_MAPS};
use or_renderer::{Frustum, ShadingMode, WorldBucket, WorldDraw, WorldMaterial, WorldMesh};

use super::RenderBridge;
use crate::state::drivers::WeatherState;
use crate::state::gta6_world::worldview::index::ModelRef;
use crate::state::gta6_world::worldview::{Bucket, CpuMaterial};

#[path = "world_budget.rs"]
mod budget;
#[path = "world_cards.rs"]
mod cards;
#[path = "world_kept.rs"]
mod kept;
#[path = "world_layout.rs"]
mod layout;
#[path = "world_material.rs"]
mod material;
#[path = "world_pages.rs"]
mod pages;
#[path = "world_ready.rs"]
mod ready;
#[path = "world_textures.rs"]
mod textures;
#[path = "world_water.rs"]
mod water;

pub use budget::WorldCost;
use budget::{ahead, may_take, Budgets};
use layout::{order, Layout, View};
use pages::{Pages, Place};
use textures::{Texture, Textures, ABSENT, MISSING};

/// What a material is told of a map that is a model's own palette.
const OWN_PALETTE: u64 = u64::MAX;

/// One mesh of a level of a model: where it lies in the pages and what draws it.
struct Mesh {
    place: Place,
    /// Which of the materials draws it.
    material: u32,
    /// How often its texture turns over its surface, in turns a metre.
    density: f32,
    /// The mesh as the renderer draws it, and the making of its material it was built with.
    drawn: Option<(Arc<WorldMesh>, u64)>,
}

/// One level of detail of a model, drawn while the camera is nearer than `until` metres.
struct Lod {
    until: f32,
    meshes: Vec<Mesh>,
}

/// A model on the device, or one that did not read.
enum Slot {
    Ready {
        /// From the highest detail down.
        lods: Vec<Lod>,
        bytes: u64,
        /// The frame it last drew in, and the frame a selection last held it in.
        drawn: u64,
        held: u64,
        /// Where the nearest instance of it stood when it last drew.
        at: Vec3,
        /// The keys of the palettes it holds.
        palettes: Vec<u64>,
    },
    Missing,
}

/// What meshes of the same material and bucket draw with: one bind group, made again when a map
/// of it comes to the device, is replaced by a larger one, or leaves.
struct Material {
    /// The key of the map in each slot the renderer binds.
    maps: [Option<u64>; WORLD_MAPS],
    /// Which slots hold a layer's colour, and which are read whole whatever the screen needs.
    colours: [bool; WORLD_MAPS],
    whole: [bool; WORLD_MAPS],
    family: WorldFamily,
    constants: WorldConstants,
    bucket: WorldBucket,
    /// What it draws with, and what each map was when it was made.
    made: Option<(WorldMaterial, [u64; WORLD_MAPS])>,
    /// Counted up each time it is made: a mesh built with an earlier making is built again.
    making: u64,
    /// The frame a mesh that draws last named it in, and for that frame the longest side its
    /// maps are needed at, how soon they are needed and where the nearest thing it draws stands.
    touched: u64,
    pixels: u32,
    priority: f32,
    at: Vec3,
    /// The meshes on the device it draws, and what it was found by: at none it is let go and its slot taken by the next new one.
    meshes: u32,
    of: Option<(CpuMaterial, Bucket)>,
}

/// A run of the layout that draws: a level of a model of the selection, and how far its nearest instance is.
struct Run {
    model: usize,
    level: usize,
    slot: u32,
    away: f32,
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
    /// The bytes of device memory the pages and the textures hold.
    pub bytes: u64,
    /// The pages of vertices and indices the models share.
    pub pages: usize,
    /// Whether the game's own lighting passes light them.
    pub lit_by_the_game: bool,
}

/// What a layout was made for: the selection, the camera, what is on the device, the scale of
/// the level-of-detail distances, the height of the screen and how far the sun's shadow is cast.
type LaidFor = (u64, [u32; 16], u64, u32, u32, Option<u32>);

/// What the day was worked out for: the hour, the days, the weathers, the camera's place north
/// in hundreds of metres, whether the install's tables gave one, and the exposure the frame metered.
type DayFor = (
    u32,
    u32,
    crate::state::gta6_world::worldview::sky::WeathersKey,
    i32,
    bool,
    u32,
    usize,
);

#[derive(Default)]
pub(super) struct WorldGpu {
    models: HashMap<ModelRef, Slot>,
    /// The bytes the models take of the pages.
    model_bytes: u64,
    pages: Pages,
    textures: Textures,
    palettes: textures::Palettes,
    materials: Vec<Material>,
    /// The materials by what they are made of, one a bucket, and the slots of those let go.
    material_of: HashMap<CpuMaterial, Vec<(Bucket, u32)>>,
    free_materials: Vec<u32>,
    /// What the device may hold, worked out the first frame.
    budgets: Option<Budgets>,
    frame: u64,
    /// Whether the world panel asked for the world this frame.
    pub(super) shown: bool,
    /// Counted up whenever a model or a texture comes to the device or leaves it.
    changes: u64,
    /// The models that came, will never come (`None`) or left (`false`), not yet told to the world's thread.
    told: Vec<(ModelRef, Option<bool>)>,
    /// The models on the device not yet told, for the maps they sample are not all there.
    pending: HashSet<ModelRef>,
    laid_for: Option<LaidFor>,
    layout: Layout,
    /// When the last frame was and where the camera stood in it.
    was: Option<(Instant, Vec3)>,
    drawn: WorldDrawn,
    /// The day the renderer was last handed while the world was shown.
    day: Option<DayFor>,
    water: water::Water,
    /// The far stand-ins the renderer holds.
    cards: cards::Cards,
    cost: WorldCost,
    /// The models and textures let go, to count those that come back; each set held to a bound by `remember`.
    gone: (HashSet<ModelRef>, HashSet<u64>),
    /// The terrain's heights the renderer was given, by their place in memory.
    terrain: usize,
}

fn bucket_of(bucket: Bucket) -> WorldBucket {
    match bucket {
        Bucket::Opaque => WorldBucket::Opaque,
        Bucket::Cutout => WorldBucket::Cutout,
        Bucket::Blend => WorldBucket::Blend,
        Bucket::Decal => WorldBucket::Decal,
        Bucket::Emissive => WorldBucket::Emissive,
    }
}

/// One ask a key of `wanted`, sorted by key: the soonest of its asks and the largest.
fn merged(mut wanted: Vec<(f32, u64, u32)>) -> Vec<(f32, u64, u32)> {
    wanted.sort_unstable_by_key(|(_, key, _)| *key);
    wanted.dedup_by(|next, kept| {
        let same = next.1 == kept.1;
        if same {
            kept.0 = kept.0.min(next.0);
            kept.2 = kept.2.max(next.2);
        }
        same
    });
    wanted
}

impl RenderBridge {
    /// What the world viewer drew last.
    pub fn world_drawn(&self) -> WorldDrawn {
        self.world.drawn
    }

    /// What the last frame of the world cost the CPU, and what its draws ask of the device.
    pub fn world_cost(&self) -> (WorldCost, or_renderer::world::WorldCalls) {
        (self.world.cost, self.renderer.world_calls())
    }

    /// How many times a model or texture came to the device or left it, or a far stand-in was left to bake.
    pub fn world_changes(&self) -> u64 {
        self.world.changes
    }

    /// Forget the lit frame's temporal history and start its camera offsets from their first.
    pub fn restart_temporal(&self) {
        self.renderer.restart_temporal();
    }

    /// Have the next frame lay the world out again, as though the camera had moved.
    pub fn relay_world(&mut self) {
        self.world.laid_for = None;
    }

    /// Draw the GTA VI world this frame, lit by its own day on the `days`th day in `weather`: put
    /// the models and textures that arrived on the device, and lay out the instances of the last
    /// selection the camera sees.
    pub fn refresh_world(&mut self, days: f32, weather: WeatherState) {
        let started = Instant::now();
        (self.world.cost.laid, self.world.cost.uploaded) = (false, (0, 0));
        self.lay_world(days, weather);
        self.world.cost.refresh = started.elapsed();
    }

    fn lay_world(&mut self, days: f32, weather: WeatherState) {
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
        let bc = (self.device.features()).contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let world = &mut self.world;
        // ponytail: wgpu 29's adapter states no size of memory, so the kept bytes are the fixed ones or those a person set; a size read from the backend goes here.
        let budgets = *(world.budgets).get_or_insert_with(|| {
            let set = std::env::var(budget::MEMORY_VARIABLE).ok();
            Budgets::of(&self.device.limits(), budget::memory_set(set.as_deref()))
        });
        let mut viewer = crate::state::gta6_world::worldview::viewer();
        let eye = viewer.camera.position;
        let now = Instant::now();
        let lead = world.was.map_or(eye, |(then, was)| {
            ahead(eye, was, now.duration_since(then).as_secs_f32())
        });
        let (time, more) = budget::taking(world.was.map(|(then, _)| now.duration_since(then)));
        world.was = Some((now, eye));

        // The textures first, for half the time: one completes a model already on the device, where a model asks for more.
        let taking = Instant::now();
        let mut uploaded = 0u64;
        while uploaded < budgets.textures_a_frame * more && may_take(taking, uploaded, time / 2) {
            let Some((key, pixels, read)) = viewer.take_texture() else {
                break;
            };
            world.cost.reread.1 += u64::from(world.gone.1.remove(&key));
            uploaded += world.textures.arrive(
                &self.device,
                &self.queue,
                (key, pixels),
                read.as_ref(),
                (bc, budgets.most_side),
                (frame, eye),
            );
            world.changes += 1;
        }
        world.cost.uploaded.1 = uploaded;
        let mut uploaded = 0u64;
        while uploaded < budgets.models_a_frame * more && may_take(taking, uploaded, time) {
            let Some((model, read)) = viewer.take_model() else {
                break;
            };
            world.forget(&model);
            world.cost.reread.0 += u64::from(world.gone.0.remove(&model));
            let slot = match read {
                Some(read) => {
                    let before = world.model_bytes;
                    let slot = world.put(&self.device, &self.queue, &budgets, &read, (frame, eye));
                    uploaded += world.model_bytes - before;
                    world.pending.insert(model);
                    viewer.on_device(model);
                    slot
                }
                None => {
                    world.told.push((model, None));
                    Slot::Missing
                }
            };
            world.changes += 1;
            world.models.insert(model, slot);
        }
        world.cost.uploaded.0 = uploaded;
        world.release_whole(viewer.reads_textures());
        world.cost.taking = taking.elapsed();

        // ⚠ Handing the renderer a day bumps its revision: only when what it was worked out for changed.
        let lighting = Instant::now();
        let metered = self.renderer.metered();
        let weathers = viewer.weathers(weather);
        let (day, lights, exposure) = viewer.sky(days, weathers, metered.as_ref());
        let day_for = (
            viewer.hour.to_bits(),
            days.to_bits(),
            weathers.key(),
            (eye.y / 100.0).round() as i32,
            day.is_some(),
            exposure.to_bits(),
            // A sky table worked out on a worker arrives some frames after the moment that asked for it.
            (day.as_ref().and_then(|d| d.sky_view.as_ref()))
                .map_or(0, |v| Arc::as_ptr(&v.radiance) as usize),
        );
        if world.day != Some(day_for) {
            world.day = Some(day_for);
            self.renderer.set_day(day);
        }
        self.renderer.set_point_lights(lights.points);
        self.renderer.set_spot_lights(lights.spots);
        self.renderer.set_capsule_lights(lights.capsules);
        (self.renderer).set_distant_lights(lights.sprite_look, lights.sprites);
        self.renderer.set_coronas(lights.coronas);
        self.renderer.set_world_lamps(lights.lamps);
        self.renderer.set_world_exposure(exposure);
        self.renderer.set_world_wind(viewer.wind(weathers));
        world.cost.lights = lighting.elapsed();
        // ⚠ The game stretches its cascades by what its camera rides; the viewer's rides nothing, so its cascades are the free camera's, and past them the terrain alone shades, as the game's does.
        if let Some(terrain) = viewer.terrain() {
            let given = Arc::as_ptr(&terrain) as usize;
            if world.terrain != given {
                world.terrain = given;
                let grid = or_renderer::world::TerrainGrid {
                    west: terrain.west,
                    north: terrain.north,
                    texel: terrain.texel,
                    width: terrain.width,
                    height: terrain.height,
                    heights: &terrain.heights,
                };
                (self.renderer).set_world_terrain(&self.device, &self.queue, Some(grid));
            }
        }
        // The GTA V day is worked out again once the world is left.
        self.day_key = None;
        // From a room that sees no portal to the outside, no water.
        let water = viewer.water(weathers).filter(|_| viewer.sees_outside());
        if (world.water).refresh(&mut self.renderer, &self.device, water.as_ref(), eye) {
            world.changes += 1;
        }

        let selected = viewer.selected(&mut world.told);
        let (width, height) = self.desired_size;
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        let view_proj = viewer.camera.view_proj(aspect);
        let (lod_scale, fov_y) = (viewer.lod_scale, viewer.camera.fov_y);
        let index = viewer.index.clone();
        drop(viewer);
        let Some(selected) = selected else {
            if self.renderer.has_world() {
                (self.renderer).set_world(&self.device, &self.queue, &[], Vec::new());
            }
            world.laid_for = None;
            world.drawn = WorldDrawn::default();
            world.hide_cards(&mut self.renderer);
            return;
        };
        // What casts a shadow into the view draws as well, where the sun's shadows are cast.
        let casts = self.renderer.shadows() && self.renderer.has_deferred();
        let reach = casts.then(|| self.renderer.shadow_reach());
        // Nothing moved, arrived or left: the device holds this frame's instances already.
        let key = (
            selected.version,
            view_proj.to_cols_array().map(f32::to_bits),
            world.changes,
            lod_scale.to_bits(),
            height,
            reach.map(f32::to_bits),
        );
        if world.laid_for == Some(key) {
            return;
        }
        world.laid_for = Some(key);
        let laying = Instant::now();

        // A slot for each level of each model of the selection that is on the device.
        let instances = &selected.instances;
        world.layout.clear();
        for model in &selected.models {
            match world.models.get_mut(model) {
                Some(Slot::Ready { lods, held, .. }) => {
                    *held = frame;
                    world.layout.model(lods.iter().map(|lod| lod.until));
                }
                _ => world.layout.model(std::iter::empty()),
            }
        }
        let view = View {
            frustum: Frustum::of(view_proj),
            view_proj,
            eye,
            lod_scale,
            shadow_far: reach,
        };
        let occluding = Instant::now();
        world.layout.occlude(&selected.occluders, &view);
        let occluded = occluding.elapsed();
        world.layout.cull(&selected.chunks, instances, &view);
        world.layout.place(instances);
        let culled = occluding.elapsed() - occluded;

        // The runs that draw, and for each material they name how large and how soon its maps are needed.
        let per_metre = textures::per_metre(height, fov_y);
        let (mut runs, mut touched) = (Vec::new(), Vec::new());
        for (model, named) in selected.models.iter().enumerate() {
            let Some(Slot::Ready {
                lods, drawn, at, ..
            }) = world.models.get_mut(named)
            else {
                continue;
            };
            let mut nearest = f32::INFINITY;
            for (level, slot) in world.layout.slots(model).enumerate() {
                let Some(lod) = lods.get(level) else {
                    continue;
                };
                if world.layout.run(slot).is_empty() {
                    continue;
                }
                let (away, which) = world.layout.nearest[slot as usize];
                *drawn = frame;
                // A run that only casts shadows into the view asks for no map.
                if away.is_infinite() {
                    let away = reach.unwrap_or(or_renderer::world::SHADOW_FAR);
                    runs.push(Run {
                        model,
                        level,
                        slot,
                        away,
                    });
                    continue;
                }
                let instance = &instances[which as usize];
                if away < nearest {
                    (nearest, *at) = (away, instance.centre);
                }
                let rows = &instance.rows;
                let scale = Vec3::new(rows[0][0], rows[1][0], rows[2][0]).length();
                // What is nearest where the camera will shortly be is asked for first.
                let priority = lead.distance(instance.centre);
                let mut blends = false;
                for mesh in &lod.meshes {
                    let material = &mut world.materials[mesh.material as usize];
                    blends |= material.bucket == WorldBucket::Blend;
                    let needed =
                        textures::pixels(mesh.density, away, scale, per_metre, budgets.most_side);
                    if material.touched != frame {
                        touched.push(mesh.material);
                        material.touched = frame;
                        (material.pixels, material.priority) = (needed, priority);
                        material.at = instance.centre;
                    } else {
                        material.pixels = material.pixels.max(needed);
                        if priority < material.priority {
                            (material.priority, material.at) = (priority, instance.centre);
                        }
                    }
                }
                // ⚠ What blends is drawn far to near, the instances of one run too: a pane seen through another is drawn first.
                if blends {
                    world.layout.far_first(slot, eye);
                }
                runs.push(Run {
                    model,
                    level,
                    slot,
                    away,
                });
            }
        }

        // Each of those materials with the maps the device holds now, and the maps it waits for.
        let grey = world.textures.grey(&self.device, &self.queue);
        let room = world.textures.bytes < budgets.textures_kept;
        let mut wanted = Vec::new();
        for id in touched {
            let material = &mut world.materials[id as usize];
            let mut with = [ABSENT; WORLD_MAPS];
            for (at, (stamp, key)) in with.iter_mut().zip(material.maps).enumerate() {
                let Some(key) = key else {
                    continue;
                };
                // A model's own palette came with it and never leaves
                if world.palettes.contains_key(&key) {
                    *stamp = OWN_PALETTE;
                    continue;
                }
                // ⚠ A palette is read texel by texel: a smaller copy of it moves every column.
                let pixels = match material.whole[at] {
                    true => budgets.most_side,
                    false => material.pixels,
                };
                match world.textures.map.get_mut(&key) {
                    Some(Texture::Ready(ready)) => {
                        if ready.used != frame || eye.distance(material.at) < eye.distance(ready.at)
                        {
                            (ready.used, ready.at) = (frame, material.at);
                        }
                        *stamp = ready.stamp;
                        // Seen larger than it was asked for at, it is asked for again while there is room.
                        if !ready.top && pixels > ready.got && room {
                            wanted.push((material.priority, key, pixels));
                        }
                    }
                    Some(Texture::Missing) => {
                        *stamp = match world.cards.stated(key) {
                            Some(_) => cards::stated::STATED,
                            None => MISSING,
                        }
                    }
                    None => wanted.push((material.priority, key, pixels)),
                }
            }
            if (material.made.as_ref()).is_none_or(|(_, made_with)| *made_with != with) {
                // A colour map that is not on the device is a mid grey, or the colour its species' far stand-ins state where no file holds it; a slot that names none samples the renderer's own stand-in.
                let constants = material.constants;
                let maps = std::array::from_fn(|at| {
                    let key = material.maps[at]?;
                    let view = (world.palettes.get(&key)).or(world.textures.of(key).1);
                    match material.colours[at] {
                        true => Some(view.or(world.cards.stated(key)).unwrap_or(&grey)),
                        false => view,
                    }
                });
                let surface = WorldSurface {
                    family: material.family,
                    bucket: material.bucket,
                    maps,
                    constants,
                };
                let made = (self.renderer).world_surface(&self.device, &surface);
                material.made = Some((made, with));
                material.making += 1;
            }
        }
        // The far stand-ins, and the colour maps a species waits for to be baked.
        let model_of = |id: usize| {
            let index = index.as_deref()?;
            crate::state::gta6_world::worldview::stream::slod::model_of(index, id)
        };
        let asks = world.refresh_cards(
            &mut self.renderer,
            (&self.device, &self.queue),
            &selected,
            (&Frustum::of(view_proj), eye),
            &model_of,
        );
        // A species left for the next frame is baked then though nothing else changes.
        if asks.more {
            world.changes += 1;
        }
        wanted.extend(asks.textures);
        let frustum = Frustum::of(view_proj);
        let seen = |model: &ModelRef| {
            let at = selected.waiting_at.get(model);
            at.is_some_and(|at| at.iter().any(|(c, r)| frustum.sees_sphere(*c, *r)))
        };
        wanted.extend(world.ask_whole(
            &selected.wanted,
            seen,
            (eye, lod_scale),
            (per_metre, budgets.most_side),
            frame,
        ));
        let in_view = (selected.wanted.iter())
            .map(|(_, model)| *model)
            .filter(|model| seen(model))
            .collect();
        let mut wanted = merged(wanted);
        world.textures.hold(&mut wanted);
        let mut viewer = crate::state::gta6_world::worldview::viewer();
        viewer.see_models(in_view);
        viewer.want_textures(wanted);
        viewer.want_species_models(asks.models);
        drop(viewer);

        // A draw for each mesh of each run, in the order the renderer sends most of them together.
        let mut keyed = Vec::with_capacity(runs.len());
        for run in &runs {
            let Some(Slot::Ready { lods, .. }) = world.models.get_mut(&selected.models[run.model])
            else {
                continue;
            };
            let Some(lod) = lods.get_mut(run.level) else {
                continue;
            };
            for mesh in &mut lod.meshes {
                let material = &world.materials[mesh.material as usize];
                let Some((made, _)) = &material.made else {
                    continue;
                };
                if (mesh.drawn.as_ref()).is_none_or(|(_, making)| *making != material.making) {
                    let Some((vertices, indices)) = world.pages.buffers(mesh.place.page) else {
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
                    order(material.bucket, run.away, mesh.place.page, made.key()),
                    WorldDraw {
                        mesh: Arc::clone(drawn),
                        instances: world.layout.run(run.slot),
                    },
                ));
            }
        }
        keyed.sort_unstable_by_key(|(key, _)| *key);
        let mut draws: Vec<WorldDraw> = keyed.into_iter().map(|(_, draw)| draw).collect();

        let (pages, page_bytes) = world.pages.held();
        let (textures, textures_unread) = world.textures.counts();
        world.drawn = WorldDrawn {
            selected: instances.len(),
            seen: world.layout.seen.iter().filter(|seen| seen.shown).count(),
            draws: draws.len(),
            models: world.models.len(),
            textures,
            textures_unread,
            bytes: page_bytes + world.textures.bytes,
            pages,
            lit_by_the_game: self.renderer.has_deferred(),
        };
        let laid = world.layout.laid.len();
        world.water.draw(&mut world.layout.laid, &mut draws);
        (self.renderer).set_world(&self.device, &self.queue, &world.layout.laid, draws);
        // The water's instances after the layout's have no sphere, and cast into every cascade.
        self.renderer.set_world_spheres(&world.layout.spheres);
        world.layout.laid.truncate(laid);
        world.evict(&budgets, frame, eye);
        world.compact((&self.device, &self.queue), &budgets);
        let layout = &world.layout;
        world.cost = WorldCost {
            laid: true,
            occlude: occluded,
            cull: culled,
            layout: laying.elapsed(),
            chunks: selected.chunks.len(),
            hidden: layout.hidden,
            occluders: layout.occluders(),
            laid_instances: layout.seen.len(),
            shadow_only: layout.seen.iter().filter(|seen| !seen.shown).count(),
            ..world.cost
        };
    }

    /// Take the world, its lights and its day away where no panel asked for it this frame.
    pub(super) fn end_world(&mut self) {
        if !self.world.shown {
            (self.world.water).refresh(&mut self.renderer, &self.device, None, Vec3::ZERO);
        }
        if !self.world.shown && self.renderer.has_world() {
            (self.renderer).set_world(&self.device, &self.queue, &[], Vec::new());
            self.renderer.set_point_lights(Vec::new());
            self.renderer.set_spot_lights(Vec::new());
            self.renderer.set_capsule_lights(Vec::new());
            (self.renderer).set_distant_lights(Default::default(), Default::default());
            self.renderer.set_coronas(None);
            self.renderer.set_world_lamps(Vec::new());
            self.world.hide_cards(&mut self.renderer);
            (self.world.laid_for, self.world.day) = (None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_wanted_twice_is_asked_for_once_at_its_soonest_and_largest() {
        let wanted = vec![
            (40.0, 7, 256),
            (9.0, 3, 128),
            (12.0, 7, 1024),
            (80.0, 7, 512),
        ];
        let once: Vec<(f32, u64, u32)> = vec![(9.0, 3, 128), (12.0, 7, 1024)];
        assert_eq!(merged(wanted), once);
        assert!(merged(Vec::new()).is_empty());
    }

    #[test]
    fn every_bucket_of_a_model_is_the_renderers_of_the_same_name() {
        let pairs = [
            (Bucket::Opaque, WorldBucket::Opaque),
            (Bucket::Cutout, WorldBucket::Cutout),
            (Bucket::Decal, WorldBucket::Decal),
            (Bucket::Blend, WorldBucket::Blend),
            (Bucket::Emissive, WorldBucket::Emissive),
        ];
        for (model, renderer) in pairs {
            assert_eq!(bucket_of(model), renderer);
        }
    }
}
