use std::sync::Arc;

use or_renderer::game_shaders::mesh::{Material as GameMaterial, MeshScene, Surface};
use or_renderer::game_shaders::pack::GamePack;
use or_renderer::{LineVertex, MeshDraw, OrbitCamera, ShadingMode, ViewportRenderer};
use wgpu::util::DeviceExt;

use crate::render::lines::{build_axes, build_grid, document_lines};
use crate::render::meshes::{build_meshes_for_active_document, MeshUpload};
use crate::state::{AppState, DocPayload, DocumentId, MeshSource};

pub(crate) mod bones;
mod day;
mod dressed;
mod framing;
mod game_shaders;
mod gta6_air;
mod gta6_day;
mod photo;
pub mod pick;
mod pick_gta6_bound;
mod pick_lines;
mod posing;
mod retarget;
mod texture_upload;
mod usage;
mod world;

use retarget::{anim_scene_signature, retarget_uploads};

pub(crate) use framing::bound_rows;
pub use framing::{compute_active_doc_bounds, compute_selection_bounds};
pub use game_shaders::GameSceneReport;
pub(crate) use posing::pose_uploads;
pub(crate) use texture_upload::bound_texture;
use texture_upload::upload_texture;

/// Where a photograph's alpha comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    /// What the meshes cover and their textures' alpha keeps, at a half.
    Cut,
    /// What the game's passes drew, which cut and blend as each material says; what the meshes cover under the viewport's own shading.
    Drawn,
    /// What the game's surfaces cover, each as its material says: a blended or cut-out one by its texture's alpha, an opaque one whole; what the meshes cover where there are none.
    ///
    /// ⚠ For a garment, whose ped passes write no alpha: a chain is a strip whose texture's alpha is
    /// the links, and covered whole it was a black band round them.
    Worn,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct CameraInputs {
    pub orbit: egui::Vec2,
    pub pan: egui::Vec2,
    pub zoom: f32,
    pub frame_active: bool,
    pub frame_selection: bool,
    /// Yaw and pitch to snap to, as the axis gizmo asks.
    pub view_angles: Option<(f32, f32)>,
}

impl CameraInputs {
    pub fn merge(&mut self, other: CameraInputs) {
        self.orbit += other.orbit;
        self.pan += other.pan;
        self.zoom += other.zoom;
        self.frame_active |= other.frame_active;
        self.frame_selection |= other.frame_selection;
        if other.view_angles.is_some() {
            self.view_angles = other.view_angles;
        }
    }

    pub fn apply_to(&self, camera: &mut OrbitCamera) {
        if let Some((yaw, pitch)) = self.view_angles {
            camera.yaw = yaw;
            camera.pitch = pitch;
        }
        camera.orbit(self.orbit.x, self.orbit.y);
        camera.pan(self.pan.x, self.pan.y);
        if self.zoom != 0.0 {
            camera.dolly(self.zoom);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct MeshCacheKey {
    doc: DocumentId,
    payload_epoch: u64,
    game_pack_epoch: u64,
    selected_entry: Option<u32>,
    anim_scene: u64,
    /// ⚠ Part of the key because the channel's value is packed into the
    /// vertex at build time rather than selected in the shader. Leave it
    /// out and switching channel changes nothing on screen.
    debug_channel: Option<crate::render::debug_channel::DebugChannel>,
    lod: usize,
    /// Whether a GTA VI weapon is drawn as its parts.
    assemble_parts: bool,
    /// The garment's variant, which stands in for the diffuse its material names.
    garment_dictionary: Option<DocumentId>,
    /// ⚠ A mesh's texture comes from another tab's file, which a recook reads again without this model's own count moving.
    recooked: u64,
    /// ⚠ A GTA VI mesh's texture can come from an open pack, whose replaced texture moves no count of this model's.
    packs: u64,
    /// ⚠ A GTA VI mesh's texture is looked up by a name a naming job may learn after the mesh is built.
    texture_names: usize,
    /// Whether a ground is laid under a model, or under a particle effect, which is the scene of one.
    ground: bool,
    /// The model a GTA VI animation poses, whose skeleton a GTA VI drawable's meshes are joined to,
    /// and whether it holds a pose.
    gta6_posed_on: Option<(DocumentId, bool)>,
}

impl MeshCacheKey {
    /// What `doc`'s meshes are built from, which builds them again when it changes.
    pub(crate) fn of(state: &AppState, doc: DocumentId, game_pack_epoch: u64) -> Self {
        Self {
            doc,
            payload_epoch: state.document(doc).map_or(0, |d| d.payload_epoch()),
            game_pack_epoch,
            selected_entry: selected_dictionary_entry(state, doc),
            anim_scene: anim_scene_signature(state),
            debug_channel: state.preview_scene.debug_channel,
            lod: state.preview_scene.lod,
            assemble_parts: state.preview_scene.assemble_parts,
            garment_dictionary: state.garment_dictionary(doc),
            recooked: state.recooked,
            packs: crate::state::ttpf_document::packs_epoch(state),
            texture_names: crate::state::ttpf_document::names_known(),
            ground: state.drivers.world_background.ground,
            gta6_posed_on: crate::ui::panels::inspector::gta6_model_drawn(state).or_else(|| {
                crate::ui::panels::inspector::gta6_posed(state).map(|p| (p.model, true))
            }),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LinesCacheKey {
    doc: DocumentId,
    payload_epoch: u64,
    selected_entry: Option<u32>,
    /// The navmesh polygon selected, which its lines outline.
    polygon: Option<usize>,
    lod: usize,
    collision: bool,
    wireframe: bool,
}

impl LinesCacheKey {
    /// What `doc`'s wireframe is built from, which builds it again when it changes.
    pub(crate) fn of(state: &AppState, doc: DocumentId) -> Self {
        Self {
            doc,
            payload_epoch: state.document(doc).map_or(0, |d| d.payload_epoch()),
            selected_entry: selected_dictionary_entry(state, doc),
            polygon: (state.selection.selected(doc))
                .and_then(crate::ui::panels::selection_ids::nav_polygon_of),
            lod: state.preview_scene.lod,
            collision: state.preview_scene.show_collision,
            wireframe: true,
        }
    }
}

pub struct RenderBridge {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,

    renderer: ViewportRenderer,

    thumbnails: crate::render::thumbnails::ThumbnailRenderer,

    /// The active document's camera as of the last `end_frame`; the state owns it.
    camera: OrbitCamera,

    texture_id: Option<egui::TextureId>,

    desired_size: (u32, u32),

    lines: Vec<LineVertex>,
    /// How many of `lines`, at their end, are bones and what plugins draw, which draw in front of the meshes.
    bone_lines: usize,

    cached_doc_lines: Vec<LineVertex>,
    doc_lines_cache_key: Option<LinesCacheKey>,

    mode: ShadingMode,

    cached_mesh_draws: Vec<MeshDraw>,
    offered_channels: Vec<crate::render::debug_channel::DebugChannel>,
    /// What the last mesh build's palette retarget did, for a panel to report.
    palette_retarget: Option<or_anim::PaletteRetarget>,
    mesh_cache_key: Option<MeshCacheKey>,
    cached_mesh_uploads: Vec<MeshUpload>,
    skin_positions: Vec<[f32; 3]>,
    skin_normals: Vec<[f32; 3]>,
    /// The `skin_revision` of the pose `cached_mesh_uploads` hold.
    skinned_revision: Option<u64>,
    /// The game's shaders as a SPIR-V pack, which `ShadingMode::Shaded` draws with.
    game_pack: Option<Arc<GamePack>>,
    game_pack_epoch: u64,
    /// The revision of the sky maps the renderer holds, which the install load moves when it reads them.
    sky_textures_of: u64,
    /// What the scene the viewport holds was prepared from.
    game_scene_key: Option<MeshCacheKey>,
    /// The pose the game's passes hold: its number and how many bones it has.
    skin_key: Option<(u64, usize)>,
    /// The textures the last game scene was made from, as decoded for it.
    decoded: crate::render::game_scene::DecodedTextures,
    game_scene_report: Option<GameSceneReport>,
    /// How the active GTA VI drawable's own mesh and pixel programs came out: the surfaces they
    /// draw and the bindings left at nought, or why it draws without them; `None` where they
    /// were not tried.
    gta6_own: Option<Result<crate::render::gta6_shaded::Own, String>>,
    /// The pipelines of the game's passes built for this pack and this viewport, kept across the scenes an edit makes again.
    game_pipelines: or_renderer::game_shaders::mesh::PipelineCache,
    /// The passes of GTA VI's own programs built so far, kept across the models opened.
    gta6_pipelines: or_renderer::game_shaders::meshlets::MeshletPipelines,
    /// The active GTA VI drawable while a worker still builds a pass of its own programs: asked
    /// for again each frame, and drawn as Material until then.
    gta6_building: Option<crate::render::gta6_meshlets::Meshlets>,
    /// Whether a worker still decodes a map of the active GTA VI drawable: it is read again each
    /// frame, and drawn as Material until then.
    gta6_decoding: bool,
    /// The GTA VI drawable Shaded was last asked for, since when, and whether its own programs
    /// have drawn it since: how long a model takes to its first shaded frame is said once.
    gta6_since: Option<(crate::state::DocumentId, std::time::Instant, bool)>,
    /// What the day the renderer holds was worked out from: the install's timecycle, the hour and the weather.
    day_key: Option<(usize, u32, u32, crate::state::drivers::WeatherState)>,

    gpu_textures: crate::render::gpu_budget::GpuTextures<Arc<wgpu::BindGroup>>,
    /// What the draws are, when they are the dressed ped's and not a document's.
    dressed_key: Option<u64>,
    /// The bind group of each texture the dressed ped is drawn with, by the id its pixels were given.
    dressed_textures: std::collections::HashMap<u64, Arc<wgpu::BindGroup>>,
    /// What the dressed ped's draws are made of, which a pose rewrites and the wireframe view draws.
    held: dressed::Held,
    cache_doc: Option<DocumentId>,
    cache_selected_entry: Option<u32>,
    texture_report: crate::state::TextureResolutionReport,
    material_bgl: Arc<wgpu::BindGroupLayout>,

    particle_textures: crate::render::particles::ParticleTextureCache,
    model_meshes: crate::render::particle_models::ParticleModelMeshCache,

    inputs: CameraInputs,

    visible: bool,

    /// The GTA VI world viewer's models on the device.
    world: world::WorldGpu,

    /// What the device came from, which `with_instance` gives for wgpu to count its objects through.
    instance: Option<wgpu::Instance>,

    /// Set once the device is lost: nothing drawn on it after that is seen.
    lost: Arc<std::sync::atomic::AtomicBool>,
}

impl RenderBridge {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        colour_format: wgpu::TextureFormat,
    ) -> Self {
        let renderer = ViewportRenderer::new(&device, &queue, colour_format, 1, 1);
        let material_bgl = renderer.material_bind_group_layout();
        let lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let told = Arc::clone(&lost);
        device.set_device_lost_callback(move |reason, message| {
            tracing::error!(?reason, %message, "the graphics device was lost");
            told.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        Self {
            lost,
            device,
            queue,
            renderer,
            thumbnails: crate::render::thumbnails::ThumbnailRenderer::new(colour_format),
            camera: OrbitCamera::default(),
            texture_id: None,
            desired_size: (1, 1),
            lines: Vec::new(),
            bone_lines: 0,
            cached_doc_lines: Vec::new(),
            doc_lines_cache_key: None,
            mode: ShadingMode::default(),
            cached_mesh_draws: Vec::new(),
            offered_channels: Vec::new(),
            palette_retarget: None,
            mesh_cache_key: None,
            cached_mesh_uploads: Vec::new(),
            skin_positions: Vec::new(),
            skin_normals: Vec::new(),
            skinned_revision: None,
            game_pack: None,
            game_pack_epoch: 0,
            sky_textures_of: 0,
            game_scene_key: None,
            skin_key: None,
            decoded: Default::default(),
            game_scene_report: None,
            gta6_own: None,
            game_pipelines: Default::default(),
            gta6_pipelines: game_shaders::gta6_pipelines(),
            gta6_building: None,
            gta6_decoding: false,
            gta6_since: None,
            day_key: None,
            gpu_textures: crate::render::gpu_budget::GpuTextures::new(
                crate::state::DEFAULT_GPU_TEXTURE_MIB as usize * 1024 * 1024,
            ),
            dressed_key: None,
            dressed_textures: Default::default(),
            held: Default::default(),
            cache_doc: None,
            cache_selected_entry: None,
            texture_report: Default::default(),
            material_bgl,
            inputs: CameraInputs::default(),
            visible: false,
            world: Default::default(),
            instance: None,
            particle_textures: crate::render::particles::ParticleTextureCache::new(),
            model_meshes: crate::render::particle_models::ParticleModelMeshCache::new(),
        }
    }

    /// The colour of the sun and of the ambient light the game's passes are lit by.
    pub fn set_game_light(&mut self, sun_colour: glam::Vec3, ambient: glam::Vec3) {
        self.renderer.set_game_light(sun_colour, ambient);
    }

    pub fn refresh_particles(&mut self, state: &mut AppState) {
        let cam_eye = self.camera.eye();
        let cam_forward = (self.camera.target - cam_eye).normalize_or(glam::Vec3::Z);
        let world_up = or_renderer::OrbitCamera::WORLD_UP;
        let cam_right = cam_forward.cross(world_up).normalize_or(glam::Vec3::X);
        let cam_up = cam_right.cross(cam_forward).normalize_or(world_up);
        state.sim.manager.env.camera_pos = cam_eye;
        if let Some(eff) = state.sim.manager.effect.as_mut() {
            eff.set_camera_pos(cam_eye);
            eff.set_camera_basis(cam_right, cam_up, cam_forward);
        }
        for eff in state.sim.manager.spawned_effects.iter_mut() {
            eff.set_camera_pos(cam_eye);
            eff.set_camera_basis(cam_right, cam_up, cam_forward);
        }
        use crate::ui::dock::Workspace;
        let want = state.active_workspace == Workspace::Particle;
        let asset = match (want, state.active_document()) {
            (true, Some(doc)) => match &doc.payload {
                DocPayload::Resource(or_io::Asset::Particles(a)) => Some((doc.id, a)),
                _ => None,
            },
            _ => None,
        };
        let Some((doc_id, asset)) = asset else {
            self.renderer.set_particle_batches(Vec::new());
            self.renderer.set_trail_batches(Vec::new());
            self.renderer.set_model_batches(Vec::new());
            crate::render::particle_air::clear(&mut self.renderer);
            return;
        };
        self.particle_textures.invalidate_for_doc(doc_id);
        crate::render::particles::gta6::prime(&mut self.particle_textures, state, doc_id);
        self.model_meshes.invalidate_for_doc(doc_id);

        let clip_regions = state.clip_regions.table();
        let retail_parity = matches!(self.mode, ShadingMode::Shaded);
        let batches = crate::render::particles::build_batches(
            &mut self.particle_textures,
            Some(asset),
            &state.sim.manager,
            &clip_regions,
            retail_parity,
        );
        self.renderer.set_particle_batches(batches);

        let camera_dir = (self.camera.target - self.camera.eye()).normalize_or_zero();
        let camera_pos = self.camera.eye();
        let trail_batches = crate::render::trails::build_trail_batches(
            &mut self.particle_textures,
            Some(asset),
            &state.sim.manager,
            camera_dir,
            camera_pos,
            retail_parity,
        );
        self.renderer.set_trail_batches(trail_batches);

        let model_batches = crate::render::particle_models::build_model_batches(
            &mut self.model_meshes,
            &mut self.particle_textures,
            Some(asset),
            &state.sim.manager,
            self.camera.eye(),
            retail_parity,
        );
        self.renderer.set_model_batches(model_batches);

        crate::render::particle_air::give(&mut self.renderer, &state.ptfx_sinks);
    }

    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture_id
    }

    pub fn camera(&self) -> &OrbitCamera {
        &self.camera
    }

    pub fn mode(&self) -> ShadingMode {
        self.mode
    }

    /// Whether the device this draws on is lost. Its owner draws nothing more on it: a painter
    /// that cannot write a buffer panics.
    pub fn device_lost(&self) -> bool {
        self.lost.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Every channel the meshes now on screen carry, in menu order.
    pub fn debug_channels(&self) -> &[crate::render::debug_channel::DebugChannel] {
        &self.offered_channels
    }

    pub fn debug_channels_available(&self) -> bool {
        !self.offered_channels.is_empty()
    }

    /// What the last mesh build's palette retarget did.
    ///
    /// `None` where no mesh carried the skeleton its palette indexes, which is
    /// a different answer from "nothing resolved".
    pub fn palette_retarget(&self) -> Option<&or_anim::PaletteRetarget> {
        self.palette_retarget.as_ref()
    }

    pub fn set_mode(&mut self, mode: ShadingMode) {
        self.mode = mode;
    }

    pub fn push_inputs(&mut self, inputs: CameraInputs) {
        self.inputs.merge(inputs);
    }

    pub fn set_desired_size(&mut self, width: u32, height: u32) {
        self.desired_size = (width.max(1), height.max(1));
        self.visible = true;
    }

    /// How many line vertices the last `refresh_lines` assembled.
    pub fn drawn_lines(&self) -> usize {
        self.lines.len()
    }

    /// The line vertices the last `refresh_lines` assembled, in the order they are drawn.
    #[cfg(test)]
    pub(crate) fn line_vertices(&self) -> &[LineVertex] {
        &self.lines
    }

    /// The mesh draws and line vertices the active document last built.
    pub fn document_geometry(&self) -> (usize, usize) {
        (self.cached_mesh_draws.len(), self.cached_doc_lines.len())
    }

    pub fn begin_frame(&mut self) {
        self.lines.clear();
        self.inputs = CameraInputs::default();
        self.visible = false;
        self.world.shown = false;
    }

    pub fn refresh_lines(&mut self, state: &AppState) {
        self.lines.clear();
        const GRID_CELLS: i32 = 10;
        const GRID_STEP: f32 = 1.0;
        if state.prefs.viewport.show_grid
            && state.active_workspace != crate::ui::dock::Workspace::Map
        {
            self.lines.extend(build_grid(GRID_CELLS, GRID_STEP));
        }
        // The dressed ped is a clothing preview: the garment is the strongest thing in it, so the modelling axes are left out there.
        if state.prefs.viewport.show_axis
            && state.active_workspace != crate::ui::dock::Workspace::Wardrobe
        {
            let half_extent = GRID_CELLS as f32 * GRID_STEP;
            self.lines.extend(build_axes(half_extent));
        }
        self.refresh_doc_lines_cache(state);
        if !self.cached_doc_lines.is_empty() {
            self.lines.extend_from_slice(&self.cached_doc_lines);
        }
        if self.mode == ShadingMode::Wireframe
            && matches!(
                state.active_workspace,
                crate::ui::dock::Workspace::Wardrobe | crate::ui::dock::Workspace::Weapons
            )
        {
            self.lines.extend(self.dressed_wire_lines());
        }
        crate::render::particle_gizmos::append_gizmo_lines(&mut self.lines, &state.sim.manager);
        let depth_tested = self.lines.len();
        self.append_skeleton_lines(state);
        crate::render::plugin_overlay::append(&mut self.lines, state);
        crate::render::map_overlay::append(&mut self.lines, state);
        self.bone_lines = self.lines.len() - depth_tested;
    }

    fn append_skeleton_lines(&mut self, state: &AppState) {
        self.lines.extend(bones::lines(state));
        if state.active_workspace == crate::ui::dock::Workspace::Wardrobe {
            let ped = state.dressed_ped();
            self.lines
                .extend(crate::render::skeleton_lines::dressed_bones(
                    &state.workbench.wardrobe.dressed,
                    ped,
                ));
        }
    }

    fn refresh_doc_lines_cache(&mut self, state: &AppState) {
        // The dressed ped has no document, and the one left active is not what the Wardrobe shows: its lines are the floor its heels stand on.
        if state.active_workspace == crate::ui::dock::Workspace::Wardrobe {
            self.cached_doc_lines = crate::render::heel_floor::of(state);
            self.doc_lines_cache_key = None;
            return;
        }
        // The assembled weapon has no document either, and draws no lines of its own; nor does the set of maps shown on the Map screen, whose lines are its overlay.
        if matches!(
            state.active_workspace,
            crate::ui::dock::Workspace::Weapons | crate::ui::dock::Workspace::Map
        ) {
            self.cached_doc_lines.clear();
            self.doc_lines_cache_key = None;
            return;
        }
        // A navmesh's lines are its polygons' outlines and its sectors, not a wireframe of the fill, and a cloth record's strands, edges and boxes have no surface, so both draw in every mode.
        let wireframe = matches!(self.mode, ShadingMode::Wireframe)
            || state
                .active_document()
                .is_some_and(|d| d.kind == crate::state::DocumentKind::Ynv)
            || state
                .active_document()
                .is_some_and(|d| crate::render::gta6_cloth::is_cloth(&d.payload));
        let Some(doc_id) = state.active_document else {
            self.cached_doc_lines.clear();
            self.doc_lines_cache_key = None;
            return;
        };
        if !wireframe && !state.preview_scene.show_collision {
            if !self.cached_doc_lines.is_empty() {
                self.cached_doc_lines.clear();
            }
            self.doc_lines_cache_key = None;
            return;
        }
        let key = LinesCacheKey {
            wireframe,
            ..LinesCacheKey::of(state, doc_id)
        };
        if self.doc_lines_cache_key == Some(key) {
            return;
        }
        self.cached_doc_lines = document_lines(state, wireframe);
        self.doc_lines_cache_key = Some(key);
    }

    pub fn refresh_meshes(&mut self, state: &AppState) {
        let active = state.active_document;
        let active_entry = active.and_then(|id| selected_dictionary_entry(state, id));
        if active != self.cache_doc || active_entry != self.cache_selected_entry {
            if active != self.cache_doc {
                self.gpu_textures.clear();
            } else {
                self.gpu_textures.retain(|(entry, _)| entry.is_some());
            }
            self.cache_doc = active;
            self.cache_selected_entry = active_entry;
        }

        let Some(doc_id) = active else {
            self.cached_mesh_draws.clear();
            self.cached_mesh_uploads.clear();
            self.mesh_cache_key = None;
            self.clear_texture_report();
            return;
        };
        let key = MeshCacheKey::of(state, doc_id, 0);
        if self.mesh_cache_key == Some(key) {
            return;
        }
        self.dressed_key = None;

        let mut uploads = build_meshes_for_active_document(state);
        // ⚠ **Here and nowhere else.** A palette is an index into the
        // skeleton it was authored against, and the cache key hashes the
        // skeleton source — so this runs once per pairing, before any pose is
        // applied. Doing it in `refresh_skinning` would compose the mapping
        // with itself on every frame.
        self.palette_retarget = retarget_uploads(&mut uploads, state);
        let any_skinned = uploads.iter().any(|u| u.is_skinned());
        // ⚠ Taken before the uploads are dropped. They are retained only for
        // a skinned asset, so reading the channels off `cached_mesh_uploads`
        // would report none for every static drawable.
        self.offered_channels = {
            let mut all: Vec<_> = uploads
                .iter()
                .flat_map(|upload| upload.channels.iter().copied())
                .collect();
            all.sort();
            all.dedup();
            all
        };
        self.cached_mesh_draws = self.upload_meshes(&uploads, state, None);
        self.cached_mesh_uploads = if any_skinned { uploads } else { Vec::new() };
        self.skinned_revision = None;
        self.mesh_cache_key = Some(key);
    }

    pub fn refresh_skinning(&mut self, state: &AppState) {
        if self.cached_mesh_uploads.is_empty() {
            return;
        }
        if posing::skin_of(state).1.is_empty() {
            if self.skinned_revision.take().is_some() {
                self.write_bind_positions();
            }
            return;
        }

        let moved = pose_uploads(
            &mut self.cached_mesh_uploads,
            &mut self.skinned_revision,
            state,
            &mut self.skin_positions,
            &mut self.skin_normals,
        );
        for index in moved {
            if let (Some(draw), Some(upload)) = (
                self.cached_mesh_draws.get(index),
                self.cached_mesh_uploads.get(index),
            ) {
                self.queue.write_buffer(
                    &draw.vertex_buffer,
                    0,
                    bytemuck::cast_slice(&upload.vertices),
                );
            }
        }
    }

    fn write_bind_positions(&mut self) {
        for (index, upload) in self.cached_mesh_uploads.iter_mut().enumerate() {
            let Some(skinning) = upload.skinning.as_ref() else {
                continue;
            };
            for (vertex, (position, normal)) in upload.vertices.iter_mut().zip(
                skinning
                    .bind_positions
                    .iter()
                    .zip(skinning.bind_normals.iter()),
            ) {
                vertex.position = *position;
                vertex.normal = *normal;
            }
            if let Some(draw) = self.cached_mesh_draws.get(index) {
                self.queue.write_buffer(
                    &draw.vertex_buffer,
                    0,
                    bytemuck::cast_slice(&upload.vertices),
                );
            }
        }
    }

    pub fn end_frame(&mut self, encoder: &mut wgpu::CommandEncoder, state: &mut AppState) -> bool {
        self.camera = state.step_viewport_camera(self.inputs).clone();
        self.gpu_textures
            .set_budget(state.prefs.gpu_texture_budget());
        state.gpu_usage = self.gpu_usage();
        state.gpu_stages = self.renderer.gpu_stages();

        if !self.visible {
            return false;
        }

        let (w, h) = self.desired_size;
        self.renderer.resize(&self.device, w, h);
        self.renderer
            .set_lines(&self.device, &self.queue, &self.lines, self.bone_lines);

        let mesh_draws = if draws_meshes(self.mode) {
            self.cached_mesh_draws.clone()
        } else {
            Vec::new()
        };
        self.renderer.set_meshes(mesh_draws);

        self.renderer
            .set_background(state.drivers.world_background.colour);
        self.renderer
            // The world is seen under its sky, whatever a model's background is set to.
            .set_sky_background(state.drivers.world_background.sky || self.world.shown);
        self.refresh_day(state);
        self.refresh_key_light(state);
        // The sprites are laid out for the camera of this frame
        self.renderer
            .prepare_game_particles(&self.device, &self.queue, &self.camera);
        self.end_world();
        self.renderer.prepare_game_frame(&self.device, &self.queue);
        self.renderer
            .render(encoder, &self.queue, &self.camera, self.mode);
        self.renderer.take_dirty()
    }

    fn upload_meshes(
        &mut self,
        uploads: &[MeshUpload],
        state: &AppState,
        texture_entry: Option<u64>,
    ) -> Vec<MeshDraw> {
        if texture_entry.is_none() {
            self.rebuild_texture_report(uploads, state);
        }
        // ⚠ Each name once for each mesh source: binding a texture hashes every byte it stores.
        let mut bound = std::collections::HashMap::new();
        let mut draws = Vec::with_capacity(uploads.len());
        for upload in uploads {
            if upload.vertices.is_empty() || upload.indices.is_empty() {
                continue;
            }
            let vertex_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("viewport-mesh-vb"),
                    contents: bytemuck::cast_slice(&upload.vertices),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                });
            let index_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("viewport-mesh-ib"),
                    contents: bytemuck::cast_slice(&upload.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            let texture_bind_group = upload.texture_name.as_deref().and_then(|name| {
                (bound.entry((upload.source, name, upload.bump)))
                    .or_insert_with(|| {
                        self.resolve_texture(state, upload.source, name, texture_entry, upload.bump)
                    })
                    .clone()
            });
            let index_count = upload.indices.len() as u32;
            draws.push(MeshDraw {
                // ⚠ Only with its texture: the stand-in of one that did not resolve is no normal map.
                bump: upload.bump && texture_bind_group.is_some(),
                vertex_buffer,
                index_buffer,
                index_count,
                texture_bind_group,
            });
        }
        draws
    }

    /// The texture bound as `name` for thumbnail `entry` or the viewport, uploaded again once its pixels change.
    ///
    /// ⚠ Two mesh sources that embed one name with other pixels share a key, so each build uploads
    /// both again; each draw still binds its own.
    fn resolve_texture(
        &mut self,
        state: &AppState,
        from: Option<MeshSource>,
        name: &str,
        entry: Option<u64>,
        bump: bool,
    ) -> Option<Arc<wgpu::BindGroup>> {
        let name = name.to_ascii_lowercase();
        let found = bound_texture(state, from, entry, &name);
        // ⚠ A normal map is uploaded linear and a colour as sRGB, so one name read both ways is two uploads.
        let key = match bump {
            true => (entry, format!("{name}\0bump")),
            false => (entry, name),
        };
        let Some((texture, stamp)) = found else {
            self.gpu_textures.remove(&key);
            return None;
        };
        if let Some(bg) = self.gpu_textures.get(&key, stamp) {
            return Some(bg);
        }
        let (bg, bytes) = upload_texture(
            &self.device,
            &self.queue,
            &self.material_bgl,
            &texture,
            bump,
        )?;
        let bg = Arc::new(bg);
        self.gpu_textures.insert(key, bg.clone(), bytes, stamp);
        Some(bg)
    }

    pub fn texture_report(&self) -> &crate::state::TextureResolutionReport {
        &self.texture_report
    }

    fn rebuild_texture_report(&mut self, uploads: &[MeshUpload], state: &AppState) {
        let selection = state
            .active_document()
            .and_then(|doc| state.selection.selected(doc.id));
        let wanted = uploads
            .iter()
            .filter_map(|u| Some((u.source, u.texture_name.clone()?)));
        let epoch = self.texture_report.epoch.wrapping_add(1);
        self.texture_report =
            crate::state::texture_resolution::report(state, selection, wanted, epoch);
    }

    fn clear_texture_report(&mut self) {
        let epoch = self.texture_report.epoch.wrapping_add(1);
        self.texture_report = crate::state::TextureResolutionReport {
            epoch,
            ..Default::default()
        };
    }

    pub fn invalidate_texture_resolution(&mut self) {
        self.gpu_textures.clear();
        self.mesh_cache_key = None;
        self.game_scene_key = None;
    }

    pub fn colour_texture_view(&self) -> &wgpu::TextureView {
        self.renderer.colour_view()
    }

    pub fn set_texture_id(&mut self, id: egui::TextureId) {
        self.texture_id = Some(id);
    }
}

fn draws_meshes(mode: ShadingMode) -> bool {
    !matches!(mode, ShadingMode::Wireframe)
}

pub(crate) fn selected_dictionary_entry(state: &AppState, doc_id: DocumentId) -> Option<u32> {
    use crate::ui::panels::selection_ids::{ypt_tag_of, ypt_unpack_model_hash, YPT_MODEL};
    state.document(doc_id)?;
    let sel = state.selection.selected(doc_id)?;
    if ypt_tag_of(sel) != YPT_MODEL {
        return None;
    }
    Some(ypt_unpack_model_hash(sel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::lines::build_lines_for_active_document;
    use crate::state::Document;
    use crate::ui::panels::selection_ids::{dict_entry_id, DOC_ROOT_SEL};

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn a_skinned_drawable_shown_in_shaded_follows_the_pose_the_animation_holds() {
        use or_renderer::game_shaders::mesh::{bone_palette, bounds};
        const WHAT: &str = "a_skinned_drawable_shown_in_shaded_follows_the_pose";
        const PX: u32 = 128;
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let Ok(pack) = std::env::var("OPENRAGE_GAME_PACK") else {
            or_test_support::skip_install(WHAT, format_args!("OPENRAGE_GAME_PACK names no pack"));
            return;
        };
        let device = Arc::new(device);
        let mut bridge = RenderBridge::new(
            device.clone(),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        assert!(bridge.set_game_pack_path(std::path::Path::new(&pack)));
        bridge.mode = ShadingMode::Shaded;
        bridge.renderer.resize(&device, PX, PX);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);

        // The first skinned drawable of the corpus that shows something at rest
        let mut state = AppState::new();
        let mut files = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ydr")
            .target(or_test_support::TARGET_GEN8)
            .find();
        files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX));
        let mut shown = None;
        for path in files {
            let Ok(asset) = or_io::open(&path) else {
                continue;
            };
            let doc = Document::new(0, Some(path.clone()), DocPayload::Resource(asset));
            state.active_document = Some(doc.id);
            state.documents = vec![doc];
            let Some(surfaces) = crate::render::game_scene::surfaces_of_active_document(&state)
            else {
                continue;
            };
            let Some((min, max)) =
                bounds(&surfaces).filter(|_| !bone_palette(&surfaces).is_empty())
            else {
                continue;
            };
            let mut camera = OrbitCamera::default();
            camera.frame_sphere((min + max) * 0.5, (max - min).length() * 0.5);
            bridge.refresh_game_scene(&state);
            let rest = bridge
                .render_pass(&camera, ShadingMode::Shaded, PX)
                .unwrap();
            let first = rest.get_pixel(0, 0);
            if rest.pixels().any(|pixel| pixel != first) {
                shown = Some((path, camera, rest));
                break;
            }
        }
        let Some((path, camera, rest)) = shown else {
            or_test_support::skip_corpus(WHAT, "no skinned .ydr that shows");
            return;
        };
        // The same pose as no pose: every bone where it is
        state.anim.skin_matrices = vec![glam::Mat4::IDENTITY; 512];
        bridge.refresh_game_scene(&state);
        let same = bridge
            .render_pass(&camera, ShadingMode::Shaded, PX)
            .unwrap();
        assert_eq!(same, rest, "{}: a pose at rest moved it", path.display());
        // A pose that carries every bone far off carries the drawable with it; the session numbers its poses, which this test does not, so this one has a bone more
        state.anim.skin_matrices =
            vec![glam::Mat4::from_translation(glam::Vec3::splat(1.0e4)); 513];
        bridge.refresh_game_scene(&state);
        let moved = bridge
            .render_pass(&camera, ShadingMode::Shaded, PX)
            .unwrap();
        let error = pollster::block_on(validation.pop());
        assert!(error.is_none(), "{error:?}");
        assert_ne!(moved, rest, "{}: the pose did not move it", path.display());
    }

    fn state_with_a_dictionary() -> (AppState, DocumentId) {
        let dict = or_io::gta5::PgDictionary {
            codes: vec![0x1111_1111, 0x2222_2222],
            entries: vec![Some(Box::default()), Some(Box::default())],
            ..Default::default()
        };

        let mut state = AppState::new();
        let doc = Document::new(
            0,
            None,
            DocPayload::Resource(or_io::Asset::DrawableDict(dict)),
        );
        let id = doc.id;
        state.documents.push(doc);
        state.active_document = Some(id);
        (state, id)
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn the_bridge_publishes_its_targets_and_thumbnails_to_the_state() {
        const WHAT: &str = "the_bridge_publishes_its_targets_and_thumbnails_to_the_state";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let device = Arc::new(device);
        let mut bridge = RenderBridge::new(
            device.clone(),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let mut state = AppState::new();
        let mut encoder = device.create_command_encoder(&Default::default());
        bridge.set_desired_size(320, 200);
        bridge.end_frame(&mut encoder, &mut state);
        let mut tile_encoder = device.create_command_encoder(&Default::default());
        bridge.thumbnails.render_one(
            &device,
            &bridge.queue,
            &mut tile_encoder,
            Vec::new(),
            &OrbitCamera::default(),
            ShadingMode::Solid,
        );
        bridge.end_frame(&mut encoder, &mut state);

        assert_eq!(
            state.gpu_usage.target_bytes,
            bridge.renderer.target_bytes() as usize
        );
        assert!(state.gpu_usage.target_bytes > 0);
        assert_eq!(
            state.gpu_usage.thumbnail_bytes,
            bridge.thumbnails.bytes() as usize
        );
        assert!(state.gpu_usage.thumbnail_bytes > 0);
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn the_bridge_publishes_the_renderers_buffers_and_default_textures() {
        const WHAT: &str = "the_bridge_publishes_the_renderers_buffers_and_default_textures";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let device = Arc::new(device);
        let mut bridge = RenderBridge::new(
            device.clone(),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let mut state = AppState::new();
        let mut encoder = device.create_command_encoder(&Default::default());
        bridge.set_desired_size(320, 200);
        bridge.end_frame(&mut encoder, &mut state);

        let resident = bridge.renderer.resident_bytes();
        let usage = state.gpu_usage;
        assert_eq!(usage.line_buffer_bytes, resident.line_buffers as usize);
        assert_eq!(
            usage.particle_instance_bytes,
            resident.particle_instance_buffers as usize
        );
        assert_eq!(
            usage.uniform_buffer_bytes,
            resident.uniform_buffers as usize
        );
        assert!(usage.uniform_buffer_bytes > 0);
        assert_eq!(
            usage.default_texture_bytes,
            resident.default_textures as usize
        );
        assert!(usage.default_texture_bytes > 0);
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn the_bridge_publishes_the_drivers_objects_to_the_state() {
        const WHAT: &str = "the_bridge_publishes_the_drivers_objects_to_the_state";
        let Some((instance, device, queue)) = crate::render::test_gpu(WHAT) else {
            return;
        };
        let device = Arc::new(device);
        let mut bridge = RenderBridge::new(
            device.clone(),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
        .with_instance(instance);
        let mut state = AppState::new();
        let mut encoder = device.create_command_encoder(&Default::default());
        bridge.set_desired_size(320, 200);
        bridge.end_frame(&mut encoder, &mut state);
        bridge.end_frame(&mut encoder, &mut state);
        let samplers = state.gpu_usage.driver.samplers;

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        bridge.end_frame(&mut encoder, &mut state);
        assert_eq!(state.gpu_usage.driver.samplers, samplers + 1);

        drop(sampler);
        bridge.end_frame(&mut encoder, &mut state);
        assert_eq!(
            state.gpu_usage.driver.samplers, samplers,
            "a dropped sampler leaves the count the state shows"
        );
    }

    #[test]
    fn every_mode_but_wireframe_hands_the_renderer_the_documents_meshes() {
        for mode in [
            ShadingMode::Solid,
            ShadingMode::Material,
            ShadingMode::Shaded,
            ShadingMode::Channel,
        ] {
            assert!(draws_meshes(mode), "{mode:?} hands the renderer no mesh");
        }
        assert!(!draws_meshes(ShadingMode::Wireframe));
    }

    #[test]
    fn selecting_a_dictionary_entry_changes_the_cache_key() {
        let (mut state, id) = state_with_a_dictionary();

        assert_eq!(
            selected_dictionary_entry(&state, id),
            None,
            "nothing selected yet"
        );

        state.selection.select(id, dict_entry_id(0x1111_1111));
        let first = selected_dictionary_entry(&state, id);
        assert_eq!(first, Some(0x1111_1111));

        state.selection.select(id, dict_entry_id(0x2222_2222));
        let second = selected_dictionary_entry(&state, id);
        assert_eq!(second, Some(0x2222_2222));

        assert_ne!(
            first, second,
            "two entries must produce two keys, or the second one is never drawn"
        );
    }

    #[test]
    fn a_selection_that_is_not_an_entry_keys_nothing() {
        let (mut state, id) = state_with_a_dictionary();
        state.selection.select(id, DOC_ROOT_SEL);
        assert_eq!(selected_dictionary_entry(&state, id), None);
    }

    #[test]
    fn a_particle_model_still_keys_the_way_it_did() {
        let mut state = AppState::new();
        let doc = Document::new(
            0,
            None,
            DocPayload::Resource(or_io::Asset::Particles(Default::default())),
        );
        let id = doc.id;
        state.documents.push(doc);
        state.active_document = Some(id);

        state.selection.select(id, dict_entry_id(0xDEAD_BEEF));
        assert_eq!(selected_dictionary_entry(&state, id), Some(0xDEAD_BEEF));
    }

    #[test]
    fn a_staged_navmesh_draws_its_polygons_and_sectors() {
        const WHAT: &str = "a_staged_navmesh_draws_its_polygons_and_sectors";
        let Some(path) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ynv")
            .find()
            .into_iter()
            .min_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX))
        else {
            or_test_support::skip_corpus(WHAT, "no .ynv staged");
            return;
        };
        let data = std::fs::read(&path).expect("read");
        let mesh = <or_io::gta5::CNavMesh as or_io::gta5::File>::from_bytes(&data)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut state = AppState::new();
        let doc = Document::new(
            0,
            Some(path.clone()),
            DocPayload::Resource(or_io::Asset::NavMesh(mesh)),
        );
        state.active_document = Some(doc.id);
        state.documents.push(doc);

        let meshes = build_meshes_for_active_document(&state);
        assert!(
            meshes.iter().any(|m| !m.indices.is_empty()),
            "{}: no filled polygons",
            path.display()
        );
        assert!(
            !build_lines_for_active_document(&state).is_empty(),
            "{}: no outlines or sectors",
            path.display()
        );
        let (_, radius) = framing::compute_active_doc_bounds(&state)
            .unwrap_or_else(|| panic!("{}: nothing to frame", path.display()));
        assert!(
            radius > 0.0,
            "{}: a degenerate framing sphere",
            path.display()
        );
    }

    /// The hash of the first entry whose first level of detail holds a geometry.
    fn an_entry_with_geometry<T: or_io::gta5::DrawableTarget>(
        dict: &or_io::gta5::PgDictionary<or_io::gta5::GtaDrawable<T>>,
    ) -> Option<u32> {
        dict.codes
            .iter()
            .zip(&dict.entries)
            .find_map(|(code, entry)| {
                let lod = &entry.as_deref()?.lod_group.lods[0];
                lod.models
                    .iter()
                    .any(|m| !m.geometries.is_empty())
                    .then_some(*code)
            })
    }

    #[test]
    fn a_dictionary_of_either_target_draws_and_frames_its_selected_entry() {
        const WHAT: &str = "a_dictionary_of_either_target_draws_and_frames_its_selected_entry";
        for target in [or_test_support::TARGET_GEN8, or_test_support::TARGET_GEN9] {
            let staged = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ydd")
                .target(target)
                .find()
                .into_iter()
                .find_map(|path| {
                    let asset = or_io::open(&path).ok()?;
                    let code = match &asset {
                        or_io::Asset::DrawableDict(dict) => an_entry_with_geometry(dict),
                        or_io::Asset::DrawableDictGen9(dict) => an_entry_with_geometry(dict),
                        _ => None,
                    }?;
                    Some((path, asset, code))
                });
            let Some((path, asset, code)) = staged else {
                or_test_support::skip_corpus(
                    WHAT,
                    format_args!("no {target} .ydd entry with geometry"),
                );
                continue;
            };
            let mut state = AppState::new();
            let doc = Document::new(0, Some(path.clone()), DocPayload::Resource(asset));
            let id = doc.id;
            state.documents.push(doc);
            state.active_document = Some(id);
            state.selection.select(id, dict_entry_id(code));

            assert!(
                !build_meshes_for_active_document(&state).is_empty(),
                "{}: the selected entry draws no mesh",
                path.display()
            );
            assert!(
                !build_lines_for_active_document(&state).is_empty(),
                "{}: the selected entry draws no wireframe",
                path.display()
            );
            let framed = framing::compute_active_doc_bounds(&state);
            assert!(
                framed.is_some_and(|(_, radius)| radius > 0.0),
                "{}: the selected entry frames as {framed:?}",
                path.display()
            );
        }
    }

    #[test]
    fn leaving_channel_mode_leaves_none_of_its_colours_behind() {
        use crate::render::debug_channel::DebugChannel;
        const WHAT: &str = "leaving_channel_mode_leaves_none_of_its_colours_behind";
        let colours = |state: &AppState| -> Vec<[f32; 4]> {
            build_meshes_for_active_document(state)
                .iter()
                .flat_map(|upload| upload.vertices.iter().map(|v| v.colour))
                .collect()
        };
        let mut files = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ydr").find();
        files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX));
        let mut state = AppState::new();
        let staged = files.into_iter().find(|path| {
            let Ok(asset) = or_io::open(path) else {
                return false;
            };
            let doc = Document::new(0, Some(path.clone()), DocPayload::Resource(asset));
            state.active_document = Some(doc.id);
            state.documents = vec![doc];
            build_meshes_for_active_document(&state)
                .iter()
                .any(|upload| upload.channels.contains(&DebugChannel::Normal))
        });
        let Some(path) = staged else {
            or_test_support::skip_corpus(WHAT, "no .ydr with normals");
            return;
        };

        let plain = colours(&state);
        state.preview_scene.debug_channel = Some(DebugChannel::Normal);
        state
            .preview_scene
            .follow_shading_mode(ShadingMode::Channel);
        assert!(
            colours(&state) != plain,
            "{}: Channel mode painted none of the normals' colours",
            path.display()
        );
        for mode in [
            ShadingMode::Wireframe,
            ShadingMode::Solid,
            ShadingMode::Material,
            ShadingMode::Shaded,
        ] {
            state.preview_scene.debug_channel = Some(DebugChannel::Normal);
            state.preview_scene.follow_shading_mode(mode);
            assert!(
                colours(&state) == plain,
                "{}: {mode:?} kept Channel mode's colours",
                path.display()
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn the_wardrobes_light_lights_the_dressed_ped_and_every_other_workspace_keeps_the_first_one() {
        use or_renderer::viewport::{KEY_LIGHT_STRENGTH, KEY_LIGHT_TOWARDS};
        const WHAT: &str = "the_wardrobes_light_lights_the_dressed_ped";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let device = Arc::new(device);
        let mut bridge = RenderBridge::new(
            device.clone(),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let mut state = AppState::new();
        bridge.set_desired_size(64, 64);
        let frame = |bridge: &mut RenderBridge, state: &mut AppState| {
            bridge.begin_frame();
            bridge.set_desired_size(64, 64);
            let mut encoder = device.create_command_encoder(&Default::default());
            bridge.end_frame(&mut encoder, state);
        };

        state.workbench.wardrobe.dressed.light = crate::state::workbench::light::Light {
            azimuth: 90.0,
            elevation: 45.0,
            strength: 1.25,
            sun: false,
        };
        state.active_workspace = crate::ui::dock::Workspace::Browse;
        frame(&mut bridge, &mut state);
        assert_eq!(
            bridge.renderer.key_light(),
            (KEY_LIGHT_TOWARDS, KEY_LIGHT_STRENGTH),
            "a workspace that is not the Wardrobe is lit as the viewport always was"
        );

        state.active_workspace = crate::ui::dock::Workspace::Wardrobe;
        frame(&mut bridge, &mut state);
        let (towards, strength) = bridge.renderer.key_light();
        assert!((towards - state.workbench.wardrobe.dressed.light.way()).length() < 1e-5);
        assert_eq!(strength, 1.25);

        state.active_workspace = crate::ui::dock::Workspace::Browse;
        frame(&mut bridge, &mut state);
        assert_eq!(
            bridge.renderer.key_light(),
            (KEY_LIGHT_TOWARDS, KEY_LIGHT_STRENGTH),
            "leaving the Wardrobe puts the first light back"
        );
    }
}
