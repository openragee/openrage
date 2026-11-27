//! The game's own passes in the viewport: the SPIR-V pack the bridge reads, and the scene of the open drawable it prepares for it.

use std::sync::Arc;

use or_renderer::game_shaders::mesh::{Material, MeshScene, Surface};
use or_renderer::game_shaders::pack::GamePack;
use or_renderer::ShadingMode;

use super::{MeshCacheKey, RenderBridge};
use crate::state::AppState;

/// What the surfaces of the open drawable came to when prepared for the game's own passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameSceneReport {
    /// Surfaces that will draw.
    pub drawn: usize,
    /// Each surface left out: the effect its material names, and why.
    pub left_out: Vec<(String, String)>,
    /// For a GTA VI drawable: whether its programs are being translated, and what Shaded says.
    pub gta6: Option<(bool, String)>,
}

/// What a frame of the active GTA VI drawable's own passes costs the device.
#[derive(Debug, Clone, Copy)]
pub struct ModelCost {
    pub calls: or_renderer::game_shaders::meshlets::MeshletCalls,
    /// The surfaces its own passes draw, and their meshlets.
    pub surfaces: usize,
    pub meshlets: u32,
    /// The surfaces drawn as Material while a worker still builds their pass.
    pub waiting: usize,
    /// The passes built for the device, or being built.
    pub passes: usize,
    /// Whether it casts the sun's shadow under a light that casts one.
    pub casts: bool,
    /// The pixels of the viewport.
    pub size: (u32, u32),
    /// What its scene made on the device, and what came of its maps since Shaded was asked.
    pub made: or_renderer::game_shaders::meshlets::Uploads,
    pub maps: super::gta6_late_maps::LateMaps,
    /// The rays the traced writer of the sun's mask cast in the frame: nought where it casts none.
    pub rays: u64,
    /// Its rays the watchdog left unfinished, each a miss, since its pass was built.
    pub capped: u64,
}

/// No pass of GTA VI's own programs built yet: what the device's driver compiles of them is kept
/// under the configuration folder, where the backend keeps any.
pub(super) fn gta6_pipelines() -> or_renderer::game_shaders::meshlets::MeshletPipelines {
    use or_renderer::game_shaders::meshlets::MeshletPipelines;
    match crate::state::prefs::config_root() {
        Some(root) => MeshletPipelines::kept_in(root.join("gta6_shaders").join("pipelines")),
        None => MeshletPipelines::default(),
    }
}

impl RenderBridge {
    /// Read the game's SPIR-V pack at `root`, which `ShadingMode::Shaded` draws with; `false` where there is none there or it will not read.
    pub fn set_game_pack_path(&mut self, root: &std::path::Path) -> bool {
        match GamePack::open(root) {
            Ok(pack) => {
                // A material names its effect by hash, and the pack's effects are the names that hash back
                or_io::gta5::shaders::shader_names::install(
                    pack.manifest()
                        .effects
                        .iter()
                        .map(|e| (or_io::hash::string_hash_str(&e.name), e.name.clone())),
                );
                tracing::info!(root = %root.display(), effects = pack.manifest().effects.len(), "loaded the game shader pack");
                let pack = Arc::new(pack);
                self.renderer.set_game_pack(Some(pack.clone()));
                self.game_pack = Some(pack);
                self.game_pack_epoch = self.game_pack_epoch.wrapping_add(1);
                // What was built is for the pack that was read before
                self.game_pipelines = Default::default();
                self.renderer.reset_deferred_pipelines();
                self.drop_game_scene();
                true
            }
            Err(e) => {
                tracing::debug!(root = %root.display(), error = %e, "no game shader pack here");
                false
            }
        }
    }

    /// What the last frame of the active GTA VI drawable's own passes asked of the device; none where they draw nothing.
    pub fn model_cost(&self) -> Option<ModelCost> {
        let scene = self.renderer.gta6_scene().filter(|s| s.drawn() > 0)?;
        Some(ModelCost {
            calls: scene.calls(),
            surfaces: scene.drawn(),
            meshlets: scene.meshlets(),
            waiting: scene.waiting.len(),
            passes: self.gta6_pipelines.len(),
            casts: !scene.casts_no_shadow(),
            size: self.renderer.size(),
            made: scene.uploads(),
            maps: self.gta6_maps,
            rays: scene.traced_rays(),
            capped: scene.traced_capped(),
        })
    }

    /// The meshlets of each surface the active GTA VI drawable's own passes draw, and the vertices one of them is drawn with.
    pub fn model_surfaces(&self) -> Vec<(u32, u32)> {
        let scene = self.renderer.gta6_scene();
        scene.into_iter().flat_map(|scene| scene.fitted()).collect()
    }

    /// Each target the active GTA VI drawable's own passes fill, as a digest of its bytes and how many of them are not nought; none for one that is not read back.
    // ⚠ Waits for the device: for a script's step, never for a frame of the editor. The digest
    // is this build's: it tells two runs of one build apart and nothing else.
    pub fn model_targets(&self) -> Vec<Option<(u64, usize)>> {
        use std::hash::Hasher;
        let targets = (self.renderer.gta6_scene().into_iter())
            .flat_map(|scene| (0..).map_while(|at| scene.target(at)));
        let (width, height) = self.renderer.size();
        let digested = |texture| {
            let read = crate::render::read_back::rgba;
            let pixels = read(&self.device, &self.queue, texture, width, height).ok()?;
            let mut digest = std::hash::DefaultHasher::new();
            digest.write(&pixels);
            let filled = pixels.iter().filter(|byte| **byte != 0).count();
            Some((digest.finish(), filled))
        };
        targets.map(digested).collect()
    }

    /// Have the active GTA VI drawable's own passes draw each surface with the most vertices an instance has, or as fitted to it again.
    pub fn model_at_most(&self, most: bool) {
        if let Some(scene) = self.renderer.gta6_scene() {
            scene.draw_at_most(most);
        }
    }

    /// Forget every pass built of a GTA VI drawable's own programs and the scene made of them: the next frames build them again, with its maps decoded already.
    // ⚠ For a script's step, which times what a model waits for once its maps are in.
    pub fn rebuild_model_passes(&mut self) {
        self.gta6_pipelines = gta6_pipelines();
        self.renderer.set_gta6_scene(None);
        self.game_scene_key = None;
        self.gta6_since = None;
        self.gta6_own = None;
        self.gta6_building = None;
    }

    /// Have the next frame of the active GTA VI drawable's own passes draw every pass again, whatever moved.
    pub fn redraw_model(&self) {
        if let Some(scene) = self.renderer.gta6_scene() {
            scene.forget();
        }
    }

    /// The stages of the last frame as the device timed them, once it has finished it; none on a device that times none.
    // ⚠ Waits for the device: for a script's step, never for a frame of the editor.
    pub fn timed_stages(&self) -> Option<Vec<(&'static str, f32)>> {
        if !(self.device.features()).contains(or_renderer::gpu_timer::FEATURES) {
            return None;
        }
        for _ in 0..8 {
            if self.renderer.collect_gpu_times(&self.device) {
                break;
            }
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        }
        Some(self.renderer.gpu_stages())
    }

    /// Whether there is a pack of the game's shaders to draw with.
    pub fn game_shaders_available(&self) -> bool {
        self.game_pack.is_some()
    }

    /// What the last scene of the game's passes came to; `None` where there is none.
    pub fn game_scene_report(&self) -> Option<&GameSceneReport> {
        self.game_scene_report.as_ref()
    }

    /// Give the game's passes the pose the animation holds, which is by bone of the skeleton the surfaces were bound to, when it has changed.
    fn refresh_skin(&mut self, state: &AppState) {
        let key = (state.anim.skin_revision(), state.anim.skin_matrices.len());
        if self.skin_key == Some(key) {
            return;
        }
        self.skin_key = Some(key);
        self.held.skin_key = None;
        self.renderer.set_skin(state.anim.skin_matrices.clone());
    }

    fn drop_game_scene(&mut self) {
        self.game_scene_key = None;
        self.held.game_key = None;
        self.decoded = Default::default();
        self.game_scene_report = None;
        self.gta6_own = None;
        self.gta6_building = None;
        self.gta6_decoding = false;
        self.gta6_fallback.clear();
        self.renderer.set_game_scene(None);
        self.renderer.set_gta6_scene(None);
        self.renderer.clear_deferred();
    }

    /// Prepare the active drawable for the game's own passes when the mode draws them, and again when what it is made from changes.
    pub fn refresh_game_scene(&mut self, state: &AppState) {
        let revision = state.clip_regions.sky_revision();
        if self.sky_textures_of != revision {
            self.sky_textures_of = revision;
            self.renderer
                .set_sky_textures((*state.clip_regions.sky_textures()).clone());
            self.renderer
                .set_water_textures((*state.clip_regions.water_textures()).clone());
        }
        if self.mode != ShadingMode::Shaded {
            if self.game_scene_key.is_some() {
                self.drop_game_scene();
            }
            return;
        }
        let Some(doc) = state.active_document else {
            return;
        };
        if let Some(hashes) = crate::render::gta6_shaded::shader_hashes(state) {
            self.refresh_gta6_scene(state, doc, &hashes);
            return;
        }
        if self.game_pack.is_none() {
            // What a GTA VI drawable's programs drew is not this document's
            if self.game_scene_key.is_some() {
                self.drop_game_scene();
            }
            return;
        }
        self.refresh_skin(state);
        let key = MeshCacheKey::of(state, doc, self.game_pack_epoch);
        if self.game_scene_key == Some(key) && !self.renderer.deferred_needs_prepare() {
            return;
        }
        self.game_scene_key = Some(key);
        self.held.game_key = None;
        let Some(surfaces) = crate::render::game_scene::surfaces_shown(state, &mut self.decoded)
        else {
            self.renderer.set_game_scene(None);
            self.renderer.clear_deferred();
            self.game_scene_report = None;
            return;
        };
        self.prepare_game_surfaces(surfaces);
    }

    /// The dressed ped for the game's own passes when the mode draws them, and again when what is worn changes; the ped stands at rest.
    pub fn refresh_dressed_game(&mut self, scene: Option<&crate::render::dressed::DressedScene>) {
        if self.mode != ShadingMode::Shaded {
            if self.held.game_key.take().is_some() {
                self.drop_game_scene();
            }
            return;
        }
        let Some(scene) = scene else {
            return;
        };
        if self.game_pack.is_none() {
            return;
        }
        let key = (scene.key, self.game_pack_epoch);
        if self.held.game_key == Some(key) && !self.renderer.deferred_needs_prepare() {
            return;
        }
        self.held.game_key = Some(key);
        self.game_scene_key = None;
        let surfaces = scene.shaded_surfaces();
        if surfaces.is_empty() {
            self.renderer.set_game_scene(None);
            self.renderer.clear_deferred();
            self.game_scene_report = None;
            return;
        }
        self.prepare_game_surfaces(surfaces);
    }

    /// Put the game's passes' ped in the pose `pose` stands for, numbered by its revision, or at rest for none; a pose already held is not written again.
    pub fn pose_dressed_game(&mut self, pose: Option<(u64, &[glam::Mat4])>) {
        let key = pose.map_or((0, 0), |(revision, skin)| (revision, skin.len()));
        if self.held.skin_key == Some(key) {
            return;
        }
        self.held.skin_key = Some(key);
        self.renderer
            .set_skin(pose.map(|(_, skin)| skin.to_vec()).unwrap_or_default());
    }

    /// Prepare `surfaces` for the game's own passes, lit by its deferred passes where the pack has them and by each pass alone where a pass will not build.
    fn prepare_game_surfaces(&mut self, surfaces: Vec<(Surface, Material)>) {
        // What a GTA VI drawable's own programs drew is not this scene's
        if self.gta6_own.take().is_some() {
            self.renderer.set_gta6_scene(None);
        }
        let Some(pack) = &self.game_pack else {
            return;
        };
        // The game's deferred passes light it where the pack has them; a pass the device refuses is reported by its error scope and not by a panic
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let deferred = self
            .renderer
            .prepare_deferred(&self.device, &self.queue, pack, &surfaces);
        match (pollster::block_on(validation.pop()), deferred) {
            (None, Ok((drawn, left_out))) => {
                self.renderer.set_game_scene(None);
                self.game_scene_report = Some(GameSceneReport {
                    drawn,
                    left_out,
                    gta6: None,
                });
                return;
            }
            (error, deferred) => {
                tracing::warn!(device = ?error, prepare = ?deferred.err(), "the game's deferred passes did not prepare, so it is lit by each pass alone");
                // A pipeline the device refused is not one to build the next scene with
                self.renderer.reset_deferred_pipelines();
                self.renderer.clear_deferred();
            }
        }
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let scene = MeshScene::prepare_with(
            &mut self.game_pipelines,
            &self.device,
            &self.queue,
            pack,
            &self.renderer.game_target(),
            &surfaces,
        );
        if let Some(error) = pollster::block_on(validation.pop()) {
            tracing::warn!(%error, "the game's passes did not prepare");
            // A pipeline the device refused is not one to build the next scene with
            self.game_pipelines = Default::default();
            self.renderer.set_game_scene(None);
            self.game_scene_report = None;
            return;
        }
        self.game_scene_report = Some(GameSceneReport {
            drawn: scene.drawn(),
            left_out: scene
                .skipped()
                .iter()
                .map(|(effect, error)| (effect.clone(), error.to_string()))
                .collect(),
            gta6: None,
        });
        self.renderer.set_game_scene(Some(scene));
    }

    /// The active GTA VI drawable for the game's own passes, by the programs its effects run as
    /// translated in memory: asked for, prepared again when a new pack of them is made, and drawn
    /// as Material where none of its surfaces draws.
    fn refresh_gta6_scene(
        &mut self,
        state: &AppState,
        doc: crate::state::DocumentId,
        hashes: &[u32],
    ) {
        use crate::state::gta6_shaders;
        // The install's folder where one is set, else the folder above the drawable's own archive.
        let install = || {
            let set = (state.prefs.game_paths.get(or_title::Game::GtaVI))
                .map(|root| root.join("shaders.rpf"));
            set.or_else(|| {
                let path = state.document(doc)?.path.as_deref()?;
                (path.ancestors().skip(1))
                    .map(|folder| folder.join("shaders.rpf"))
                    .find(|archive| archive.is_file())
            })
        };
        gta6_shaders::request(&crate::render::gta6_meshlets::worn(state), install);
        if self.gta6_since.is_none_or(|(of, ..)| of != doc) {
            tracing::info!(doc, "Shaded was asked for a GTA VI drawable");
            self.gta6_since = Some((doc, std::time::Instant::now(), false, false));
            self.gta6_maps = Default::default();
            crate::render::gta6_meshlets::start_decoding(state);
        }
        let running = gta6_shaders::running();
        let pack = gta6_shaders::pack();
        let key = MeshCacheKey::of(state, doc, pack.as_ref().map_or(0, |(number, _)| *number));
        // A map learnt or decoded since the scene was bound is bound where it is read
        self.take_on_gta6_maps(state, key);
        // The pack's passes drew without maps a worker decoded: prepared again once all are in
        let fallen = &mut self.gta6_fallback;
        if !fallen.is_empty() && crate::render::gta6_meshlets::all_in(fallen) {
            fallen.clear();
            self.game_scene_key = None;
        }
        let fresh = self.game_scene_key != Some(key);
        let report = self.game_scene_report.as_ref();
        let was = report.and_then(|r| r.gta6.as_ref()).map(|(was, _)| *was);
        let building = self.gta6_building.is_some();
        let devices = (&*self.device, &*self.queue);
        let traced = crate::render::gta6_trace::refresh(devices, self.renderer.gta6_scene(), state);
        if !fresh && was == Some(running) && !building && !traced {
            return;
        }
        let kept = report
            .filter(|_| !fresh)
            .map(|r| (r.drawn, r.left_out.clone()));
        let (drawn, left_out) = match kept {
            // A worker was building a pass of its own programs: once it has, they draw it
            Some(kept) if building => {
                self.ask_again(|| crate::render::gta6_meshlets::meshlets(state), kept)
            }
            Some(kept) => kept,
            None => {
                self.gta6_building = None;
                self.gta6_decoding = false;
                self.game_scene_key = Some(key);
                self.held.game_key = None;
                self.renderer.clear_deferred();
                self.gta6_own = self.prepare_gta6_meshlets(state);
                self.gta6_fallback.clear();
                let fallback = match (&self.gta6_own, &pack) {
                    // Drawn by its own programs, it needs no pass of the pack; with no pack yet,
                    // there is none to prepare for
                    (Some(Ok(_)), _) | (_, None) => None,
                    (_, Some((_, pack))) => {
                        let draws =
                            |effect: &str| crate::render::gta6_shaded::pack_draws(pack, effect);
                        crate::render::gta6_shaded::shaded_surfaces(state, &draws)
                    }
                };
                match (&pack, fallback) {
                    (Some((_, pack)), Some(fallback)) => {
                        self.gta6_fallback = fallback.decoding;
                        let (drawn, mut left_out) =
                            self.prepare_gta6_surfaces(pack, &fallback.surfaces);
                        left_out.extend(fallback.left_out);
                        (drawn, left_out)
                    }
                    _ => {
                        self.renderer.set_game_scene(None);
                        (0, Vec::new())
                    }
                }
            }
        };
        if let (Some(Ok(_)), Some((_, since, first, every))) =
            (&self.gta6_own, self.gta6_since.as_mut())
        {
            if !std::mem::replace(first, true) {
                tracing::info!(elapsed = ?since.elapsed(), "a GTA VI drawable's first frame by its own programs");
            }
            if self.gta6_building.is_none() && !std::mem::replace(every, true) {
                tracing::info!(elapsed = ?since.elapsed(), "a GTA VI drawable's last surface by its own programs");
            }
        }
        let said = crate::render::gta6_shaded::said(
            &gta6_shaders::status(hashes),
            drawn,
            drawn + left_out.len(),
        );
        let by_cycles = crate::state::gta6_shaders::timecycle::timecycle(state)
            .is_some_and(|tables| super::gta6_day::lights(&tables, state.drivers.weather.state));
        let said = crate::render::gta6_shaded::said_of_own(said, self.gta6_own.as_ref(), by_cycles);
        let said = crate::render::gta6_trace::said(said);
        self.game_scene_report = Some(GameSceneReport {
            drawn,
            left_out,
            gta6: Some((running, said)),
        });
    }

    /// Ask again about the drawable a worker was building for, which `read` gives where none
    /// builds for one any more: what the pack's passes draw of it, `kept` where its own draw none.
    fn ask_again(
        &mut self,
        read: impl FnOnce() -> Option<Result<crate::render::gta6_meshlets::Meshlets, String>>,
        kept: (usize, Vec<(String, String)>),
    ) -> (usize, Vec<(String, String)>) {
        let drew = matches!(self.gta6_own, Some(Ok(_)));
        self.gta6_own = self.prepare_gta6_meshlets_of(read);
        match self.gta6_own {
            Some(Ok(_)) => {
                self.renderer.set_game_scene(None);
                (0, Vec::new())
            }
            // What some of its passes drew and none does now is prepared from nothing by the
            // next frame, as a drawable they never drew is
            _ if drew => {
                self.game_scene_key = None;
                kept
            }
            _ => kept,
        }
    }

    /// Prepare the active GTA VI drawable for its own mesh and pixel programs: the surfaces they
    /// draw and the bindings left at nought, or why it draws without them; `None` where the
    /// document shows no such drawable.
    fn prepare_gta6_meshlets(
        &mut self,
        state: &AppState,
    ) -> Option<Result<crate::render::gta6_shaded::Own, String>> {
        self.prepare_gta6_meshlets_of(|| crate::render::gta6_meshlets::meshlets(state))
    }

    /// [`Self::prepare_gta6_meshlets`] of the drawable `read` gives where no worker builds for one.
    pub(super) fn prepare_gta6_meshlets_of(
        &mut self,
        read: impl FnOnce() -> Option<Result<crate::render::gta6_meshlets::Meshlets, String>>,
    ) -> Option<Result<crate::render::gta6_shaded::Own, String>> {
        use or_renderer::game_shaders::meshlets::MeshletScene;
        // The drawable a worker builds for is the one asked about again, as it was read then
        let meshlets = match self.gta6_building.take() {
            Some(meshlets) => meshlets,
            None => {
                self.renderer.set_gta6_scene(None);
                // A map decoded from here on is bound by a later frame
                self.gta6_maps.seen = crate::render::gta6_maps::arrived();
                match read()? {
                    Ok(meshlets) => meshlets,
                    Err(why) => return Some(Err(why)),
                }
            }
        };
        // A surface draws by its pass once it is built, a map still decoded as its blank
        // texel; the map is bound into the scene as it comes in
        self.gta6_decoding = meshlets.decoding;
        // What some of its passes draw takes on the surfaces a worker has built the pass of
        // since, each alone: a prepare makes every buffer and texture of every surface anew, on
        // this thread
        let live = match self.gta6_own {
            Some(Ok(_)) => self.renderer.take_gta6_scene(),
            _ => None,
        };
        // What the device refuses is reported by its error scope and not by a panic: a stage
        // its shader compiler does not take is an internal error, not a validation one
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let started = std::time::Instant::now();
        let built = self.gta6_pipelines.len();
        let (device, queue, pipelines) = (&self.device, &self.queue, &mut self.gta6_pipelines);
        let mut more = true;
        let anew = live.is_none();
        let scene = match live {
            Some(mut scene) => {
                (scene.build_on(device, queue, pipelines, &meshlets.surfaces)).map(|fewer| {
                    more = fewer;
                    Some(scene)
                })
            }
            None => MeshletScene::prepare(
                device,
                queue,
                pipelines,
                &meshlets.surfaces,
                self.renderer.size(),
                &self.renderer.game_target(),
            ),
        };
        let refused = pollster::block_on(validation.pop());
        let refused = refused.or(pollster::block_on(internal.pop()));
        self.gta6_maps.prepares += u32::from(anew && matches!(scene, Ok(Some(_))));
        if more && !matches!(scene, Ok(None)) {
            tracing::info!(
                elapsed = ?started.elapsed(),
                surfaces = meshlets.surfaces.len(),
                decoding = meshlets.decoding,
                drawn = scene.as_ref().map_or(0, |scene| scene.as_ref().map_or(0, MeshletScene::drawn)),
                passes = self.gta6_pipelines.len(),
                "a GTA VI drawable's own passes were prepared"
            );
        }
        if self.gta6_pipelines.len() > built {
            tracing::info!(
                passes = self.gta6_pipelines.len() - built,
                "a worker builds a GTA VI drawable's own passes"
            );
        }
        let building = tr!("a worker is still building its passes for this device");
        let own = match (scene, refused) {
            (Ok(None), None) => {
                self.gta6_building = Some(meshlets);
                return Some(Err(building.to_string()));
            }
            (Ok(Some(mut scene)), None) => {
                // A surface the device has no pass for, or none yet, is drawn as Material beside
                // the others
                let (mut rest, mut why) = (meshlets.rest.clone(), meshlets.why.clone());
                let refused = scene.refused.iter().map(|(at, why)| (at, why.as_str()));
                let waiting = scene.waiting.iter().map(|at| (at, building));
                for (at, reason) in refused.chain(waiting) {
                    let Some((mesh, effect)) = meshlets.of.get(*at) else {
                        continue;
                    };
                    rest.push(*mesh);
                    why.get_or_insert_with(|| format!("{effect}: {reason}"));
                }
                let own = crate::render::gta6_shaded::Own {
                    drawn: scene.drawn(),
                    forward: scene.drawn_forward(),
                    zeroed: meshlets.zeroed,
                    rest: rest.len(),
                    why,
                    unshadowed: scene.casts_no_shadow(),
                };
                self.said_all_in(meshlets.decoding);
                // The drawable is asked about again until every pass of its surfaces is built
                if !scene.waiting.is_empty() {
                    self.gta6_building = Some(meshlets);
                }
                scene.rest = rest;
                self.renderer.set_gta6_scene(Some(scene));
                return Some(Ok(own));
            }
            (Err(why), None) => Err(why),
            (_, Some(error)) => {
                tracing::warn!(%error, "a GTA VI drawable's own programs did not prepare");
                // A pipeline the device refused is not one to draw the next model with
                self.gta6_pipelines = gta6_pipelines();
                Err(error.to_string())
            }
        };
        // What drew by some of its passes draws by none of them now
        self.renderer.set_gta6_scene(None);
        Some(own)
    }

    /// Prepare `surfaces` with `pack`'s forward passes alone, since GTA VI's effects hold none of
    /// the lighting passes GTA V's deferred route draws with; a scene that draws nothing is not
    /// set, so Shaded draws as Material.
    fn prepare_gta6_surfaces(
        &mut self,
        pack: &GamePack,
        surfaces: &[(Surface, Material)],
    ) -> (usize, Vec<(String, String)>) {
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let scene = MeshScene::prepare_with(
            &mut self.game_pipelines,
            &self.device,
            &self.queue,
            pack,
            &self.renderer.game_target(),
            surfaces,
        );
        if let Some(error) = pollster::block_on(validation.pop()) {
            tracing::warn!(%error, "the GTA VI programs' passes did not prepare");
            // A pipeline the device refused is not one to build the next scene with
            self.game_pipelines = Default::default();
            self.renderer.set_game_scene(None);
            return (0, Vec::new());
        }
        let left_out = (scene.skipped().iter())
            .map(|(effect, error)| (effect.clone(), error.to_string()))
            .collect();
        let drawn = scene.drawn();
        self.renderer.set_game_scene((drawn > 0).then_some(scene));
        (drawn, left_out)
    }
}

#[cfg(test)]
#[path = "gta6_ahead_tests.rs"]
mod tests;
