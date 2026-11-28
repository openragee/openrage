//! Draws the real shell with no window and saves frames as PNGs, driven by a
//! script of one step per line; `docs/guides/Development_Workflow.md` lists the steps.
//!
//! ⚠ A frame is `render_editor`'s frame: the same polls, `ui::draw`, overlays
//! and render bridge. A `key` step runs the app shortcut it names as the
//! window's winit dispatcher does, `keys::press`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod compare;
mod drag;
mod dumps;
mod keys;
mod lost_device;
mod perf;
mod png;
#[cfg(test)]
mod reference_tests;
mod script;
mod waits;
mod watched;
mod world;
use png::{paint, save_png, target_texture};
use script::{parse, Step};
use waits::OPEN_DEADLINE;
pub(crate) use waits::{activate_game_and_wait, open_and_wait};

use crate::render::RenderBridge;
use crate::state::front_door::Offer;
use crate::state::prefs::Game;
use crate::state::{AfterClose, AppState, LoadingState};
use crate::ui::dock::Workspace;
use crate::ui::panels::settings::SettingsUiState;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Frames drawn before a capture: egui sizes some widgets on a second pass.
const SETTLE_FRAMES: usize = 3;

/// Discovery runs on a worker, so a `game` step with no folder waits for it.
const DISCOVERY_DEADLINE: Duration = Duration::from_secs(30);

/// How long the `picker` step draws the start page before its shot.
const LOGO_SETTLE: Duration = Duration::from_secs(1);

/// How far the fixed clock of a run moves a frame, the step the shell hands egui too.
const FRAME_STEP: Duration = Duration::from_nanos(16_666_667);

/// What every run's random draws start from.
const RANDOM_SEED: u64 = 0x5EED;

/// The exit code.
pub fn run(script: &Path, out: &Path) -> i32 {
    if let Some(limit) = watched::limit() {
        return watched::run(limit);
    }
    match run_script(script, out) {
        Ok(()) => 0,
        Err(why) => {
            println!("{why}");
            1
        }
    }
}

fn run_script(script: &Path, out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(script)
        .map_err(|e| format!("cannot read {}: {e}", script.display()))?;
    // Parsed whole before anything is drawn, so a typo on the last line
    // fails before a long run rather than after it.
    let steps = parse(&text)?;
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    // A frame is a sixtieth of a second and every run draws from one seed, so that two runs draw the same frames.
    or_transport::clock::fix(FRAME_STEP, RANDOM_SEED);

    let mut state = AppState::new();
    let (device, queue) = super::headless_gpu()?;
    let mut shell = Shell::on(&state, device, queue);
    for (number, line, step) in &steps {
        println!("> {line}");
        shell
            .step(&mut state, step, out)
            .map_err(|why| format!("line {number}: {why}"))?;
    }
    match shell.failed.is_empty() {
        true => Ok(()),
        false => Err(format!(
            "{} comparison(s) failed:\n{}",
            shell.failed.len(),
            shell.failed.join("\n")
        )),
    }
}

struct Shell {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    bridge: RenderBridge,
    renderer: egui_wgpu::Renderer,
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    /// Files the next frame receives as dropped from outside the window.
    dropped: Vec<PathBuf>,
    /// Files held over the window by a drag, which every frame reports until the drag ends.
    hovering: Vec<PathBuf>,
    /// The pollers that take a worker's result do not run: a `hold`.
    held: bool,
    /// The comparisons that failed so far, which fail the run at its end so that it judges every one.
    failed: Vec<String>,
    target: Option<(u32, u32, wgpu::Texture)>,
    /// What the last frame took to record its viewports' passes, and to finish and submit them.
    submitted: (std::time::Duration, std::time::Duration),
}

impl Shell {
    fn step(&mut self, state: &mut AppState, step: &Step, out: &Path) -> Result<(), String> {
        match step {
            Step::Size(w, h) => self.size = egui::vec2(*w, *h),
            Step::Game(game, folder) => {
                let root = match folder {
                    Some(folder) => folder.clone(),
                    None => self.discovered(state, *game)?,
                };
                activate_game_and_wait(state, *game, &root, |state| self.idle(state, 1))?;
            }
            Step::Wait(how_long) => {
                let until = Instant::now() + *how_long;
                while Instant::now() < until {
                    self.idle(state, 1)?;
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
            Step::Open(path) => self.open(state, path)?,
            Step::Project(path) => self.project(state, path)?,
            // The folder picker is the platform's, so the script names the folder it would return.
            Step::Extract(dest) => {
                let doc = state
                    .active_document
                    .ok_or("nothing is open to extract from")?;
                let rows = state.selection.selected_ids(doc);
                if rows.is_empty() {
                    return Err("no row is selected to extract".into());
                }
                state.extract_rpf_nodes(
                    doc,
                    rows,
                    dest.clone(),
                    or_io::container::rpf::BodyForm::Decoded,
                    crate::state::TypeFilter::All,
                );
            }
            // The file picker is the platform's, so the script names the glTF it would return.
            Step::Gltf(mesh) => {
                let doc = state
                    .active_document
                    .ok_or("no model is open to import into")?;
                state.import_gltf(doc, mesh.clone());
            }
            Step::Pick(path) => crate::platform::file_dialogs::answer_next(path.clone()),
            Step::Drop(path) => {
                // On the panel of the screen that takes drops when it is drawn, else where the pointer last was.
                let panel = match state.active_workspace {
                    Workspace::Weapons => crate::ui::panels::weapons::drop_rect(state),
                    Workspace::Map => crate::ui::panels::maps::drop_rect(state),
                    Workspace::Particle => crate::ui::panels::particles::drop_rect(state),
                    _ => crate::ui::panels::wardrobe::drop_rect(state),
                };
                let at = panel.map_or(egui::pos2(400.0, 300.0), |rect| rect.center());
                self.frame(state, vec![egui::Event::PointerMoved(at)], None)?;
                // A drop ends the drag that held it over the window.
                self.hovering.clear();
                self.dropped.push(path.clone());
                self.idle(state, 1)?;
            }
            Step::DropTile(index, path) => {
                let garment = state
                    .workbench
                    .wardrobe
                    .selected
                    .as_ref()
                    .map(|(_, id)| id.clone());
                let at = garment
                    .and_then(|id| crate::ui::panels::wardrobe::variant_rect(state, &id, *index))
                    .map(|rect| rect.center())
                    .ok_or_else(|| {
                        format!(
                            "no tile {index} is on screen: select a garment in the Wardrobe first"
                        )
                    })?;
                self.frame(state, vec![egui::Event::PointerMoved(at)], None)?;
                self.hovering.clear();
                self.dropped.push(path.clone());
                self.idle(state, 1)?;
            }
            Step::HoverFile(path) => {
                let at = crate::ui::panels::wardrobe::drop_rect(state)
                    .map_or(egui::pos2(400.0, 300.0), |rect| rect.center());
                self.hovering = vec![path.clone()];
                self.frame(state, vec![egui::Event::PointerMoved(at)], None)?;
            }
            Step::HoverEnd => self.hovering.clear(),
            Step::Build => state.build_all(),
            Step::Hold => self.held = true,
            Step::Release => self.held = false,
            Step::AskProject(path) => state.open_project_asking(path.clone()),
            Step::ProjectMenu => {
                state
                    .panels
                    .get::<crate::ui::panels::status_bar::ProjectMenu>()
                    .open = true;
            }
            Step::Workspace(ws) => state.active_workspace = *ws,
            Step::Reset => {
                let ws = state.active_workspace;
                state.workspace_docks.reset(ws);
            }
            Step::Mode(mode) => {
                self.bridge.set_mode(*mode);
                state.remember_shading_mode(*mode);
            }
            Step::Click {
                at,
                button,
                times,
                modifiers,
            } => {
                for _ in 0..*times {
                    let moved = vec![egui::Event::PointerMoved(*at)];
                    self.frame_with(state, moved, *modifiers, None)?;
                    for pressed in [true, false] {
                        let event = egui::Event::PointerButton {
                            pos: *at,
                            button: *button,
                            pressed,
                            modifiers: *modifiers,
                        };
                        self.frame_with(state, vec![event], *modifiers, None)?;
                    }
                }
            }
            Step::Drag {
                from,
                to,
                modifiers,
            } => self.drag(state, *from, *to, *modifiers)?,
            Step::Hover(at) => {
                self.frame(state, vec![egui::Event::PointerMoved(*at)], None)?;
                // Long enough for a tooltip to appear.
                self.idle(state, 30)?;
            }
            Step::Scroll { at, dy } => {
                self.frame(state, vec![egui::Event::PointerMoved(*at)], None)?;
                let wheel = egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, *dy),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                };
                self.frame(state, vec![wheel], None)?;
            }
            Step::Type(text) => self.frame(state, vec![egui::Event::Text(text.clone())], None)?,
            Step::Key { key, modifiers } => {
                // The window takes from egui the press of a key whose command it ran, and egui still sees the release.
                let ran = keys::press(state, &self.ctx, *key, *modifiers);
                let presses: &[bool] = if ran { &[false] } else { &[true, false] };
                for &pressed in presses {
                    let event = egui::Event::Key {
                        key: *key,
                        physical_key: Some(*key),
                        pressed,
                        repeat: false,
                        modifiers: *modifiers,
                    };
                    self.frame_with(state, vec![event], *modifiers, None)?;
                }
            }
            Step::Palette => crate::ui::panels::command_palette::open(state),
            Step::Frames(n) => self.idle(state, *n)?,
            Step::CloseAll => {
                let ids: Vec<_> = state.documents.iter().map(|d| d.id).collect();
                state.request_close(ids, AfterClose::Nothing);
            }
            Step::StartPage => state.show_start_page(),
            Step::CloseProject => state.close_project(),
            Step::NewProject(template, step, folder) => {
                state.start_new_project_on(*template, *step);
                // A folder to adopt is chosen as the picker would choose it.
                if let (Some(folder), Some(draft)) = (folder, state.front_door.new_project.as_mut())
                {
                    draft.pick_place(folder);
                }
            }
            Step::Theme(theme) => {
                state.prefs.appearance.theme = *theme;
                crate::app::gpu::apply_appearance(&self.ctx, &state.prefs.appearance);
            }
            Step::Language(code) => crate::app::gpu::use_language(&self.ctx, state, code.clone()),
            Step::Scale(percent) => {
                state.prefs.appearance.ui_scale_percent = *percent;
                crate::app::gpu::apply_appearance(&self.ctx, &state.prefs.appearance);
            }
            Step::Shot(name) => {
                self.idle(state, SETTLE_FRAMES)?;
                let path = out.join(format!("{name}.png"));
                self.frame(state, Vec::new(), Some(&path))?;
                println!("shot  {}", path.display());
            }
            Step::Picker(name) => {
                let path = out.join(format!("{name}.png"));
                // The start page is what Browse's Document panel draws while nothing is open.
                let open = std::mem::take(&mut state.documents);
                let workspace = std::mem::replace(&mut state.active_workspace, Workspace::Browse);
                // ⚠ Drawn for time, not frames: its logos load from disk on a
                // worker, and a few frames shoot their spinners instead.
                let until = Instant::now() + LOGO_SETTLE;
                let mut shot = Ok(());
                while shot.is_ok() && Instant::now() < until {
                    shot = self.idle(state, 1);
                    std::thread::sleep(Duration::from_millis(30));
                }
                let shot = shot.and_then(|()| self.frame(state, Vec::new(), Some(&path)));
                state.active_workspace = workspace;
                let opened = std::mem::replace(&mut state.documents, open);
                state.documents.extend(opened);
                shot?;
                println!("shot  {}", path.display());
            }
            Step::Settings { name, category } => {
                state.panels.get::<SettingsUiState>().category = *category;
                let path = out.join(format!("{name}.png"));
                self.window_shot(
                    state,
                    crate::app::SETTINGS_WINDOW_SIZE,
                    &path,
                    crate::ui::panels::settings::draw_settings_window_body,
                )?;
                println!("shot  {}", path.display());
            }
            Step::World(step) => self.world(state, step, out)?,
            Step::ModelPerf(frames, name, in_full) => {
                self.model_perf(state, *frames, &out.join(name), *in_full)?;
            }
            Step::ModelPasses => self.bridge.rebuild_model_passes(),
            Step::ModelFrame(picked) => {
                let doc = state
                    .active_document
                    .ok_or("no document is open to frame")?;
                let sphere = crate::render::gta6_drawable::geometries_bounds(state, doc, picked);
                let (centre, radius) = sphere
                    .ok_or_else(|| format!("the open model draws no geometry of {picked:?}"))?;
                state.frame_active_camera(centre, radius);
                println!("framed  {picked:?} about {centre} radius {radius}");
            }
            Step::Compare(step) => {
                if let Err(why) = compare::run(step, out) {
                    println!("FAILED  {why}");
                    self.failed.push(why);
                }
            }
            Step::CompareDump(step) => {
                if let Err(why) = dumps::run(step, out) {
                    println!("FAILED  {why}");
                    self.failed.push(why);
                }
            }
        }
        // Every step ends on drawn frames, so the next one acts on a settled screen.
        self.idle(state, 2)
    }

    fn discovered(&mut self, state: &mut AppState, game: Game) -> Result<PathBuf, String> {
        let until = Instant::now() + DISCOVERY_DEADLINE;
        loop {
            if let Some(root) = state.prefs.game_paths.get(game) {
                return Ok(root.clone());
            }
            if Instant::now() > until {
                return Err(format!(
                    "no {} install was found; give its folder after the game",
                    game.label()
                ));
            }
            self.idle(state, 1)?;
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn open(&mut self, state: &mut AppState, path: &Path) -> Result<(), String> {
        open_and_wait(state, path, |state| self.idle(state, 1))?;
        // The banner over an opened folder says what the folder holds once a worker has read it.
        let until = Instant::now() + DISCOVERY_DEADLINE;
        while let Some(Offer::Make(folder)) = state.adoption_offer() {
            if state.front_door.previews.of(&folder).is_some() || Instant::now() > until {
                break;
            }
            self.idle(state, 1)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Opens a project as Open Project does once its file is picked, and waits for its files to be read.
    fn project(&mut self, state: &mut AppState, path: &Path) -> Result<(), String> {
        state.load_project_from(path.to_path_buf());
        let until = Instant::now() + OPEN_DEADLINE;
        while state.project_path.as_deref() != Some(path)
            || !state.workbench.is_scanned()
            || matches!(state.loading, LoadingState::Loading)
        {
            if Instant::now() > until {
                return Err(format!(
                    "{} was still opening after three minutes",
                    path.display()
                ));
            }
            self.idle(state, 1)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn idle(&mut self, state: &mut AppState, frames: usize) -> Result<(), String> {
        for _ in 0..frames {
            self.frame(state, Vec::new(), None)?;
        }
        Ok(())
    }

    fn frame(
        &mut self,
        state: &mut AppState,
        events: Vec<egui::Event>,
        capture: Option<&Path>,
    ) -> Result<(), String> {
        self.frame_with(state, events, egui::Modifiers::NONE, capture)
    }

    /// ⚠ Polls as the editor's redraw does, except the shader-pack bake: a
    /// bake downloads its tools and runs for minutes, so a script never starts one.
    fn frame_with(
        &mut self,
        state: &mut AppState,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
        capture: Option<&Path>,
    ) -> Result<(), String> {
        self.stop_once_lost(state)?;
        or_transport::clock::tick();
        state.poll_load_result();
        state.poll_save_result();
        // No dialog opens in a script, so a document never saved stays unsaved and its close is called off.
        state.drive_close(|_, _| None);
        state.poll_key_extraction();
        state.poll_material_palette_load();
        state.poll_clip_regions_load();
        state.poll_shader_definitions();
        state.poll_game_discovery();
        state.poll_expansions();
        state.tick_navigation();
        state.poll_extract_job();
        // ⚠ A hold skips every poller of `poll_jobs`: the workbench's takes the build's and the cook's results.
        if !self.held {
            state.poll_jobs();
        }
        state.update_toasts();

        self.bridge.begin_frame();
        state.tick_previews();
        self.time += 1.0 / 60.0;
        self.ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            max_texture_side: Some(8192),
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            modifiers,
            events,
            focused: true,
            hovered_files: (self.hovering.iter())
                .map(|path| egui::HoveredFile {
                    path: Some(path.clone()),
                    mime: String::new(),
                })
                .collect(),
            dropped_files: std::mem::take(&mut self.dropped)
                .into_iter()
                .map(|path| egui::DroppedFile {
                    path: Some(path),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        crate::ui::draw(&self.ctx, state, Some(&mut self.bridge));
        crate::app::modals::draw_editor_overlays(&self.ctx, state);
        let output = self.ctx.end_pass();
        // The editor's window puts copied text on the system clipboard; a script only says it.
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                tracing::info!("the clipboard's stand-in was asked to copy {text:?}");
            }
        }
        let ppp = output.pixels_per_point;
        let primitives = self.ctx.tessellate(output.shapes, ppp);
        let (device, queue) = (self.device.clone(), self.queue.clone());
        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(&device, &queue, *id, delta);
        }

        let w = (self.size.x * ppp).round() as u32;
        let h = (self.size.y * ppp).round() as u32;
        if !matches!(&self.target, Some((tw, th, _)) if (*tw, *th) == (w, h)) {
            self.target = Some((w, h, target_texture(&device, w, h)));
        }
        let Some((_, _, target)) = &self.target else {
            return Err("no render target".into());
        };
        let view = target.create_view(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: ppp,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        self.renderer
            .update_buffers(&device, &queue, &mut encoder, &primitives, &screen);
        let recording = std::time::Instant::now();
        let dirty = self.bridge.end_frame(&mut encoder, state);
        self.submitted.0 = recording.elapsed();
        if dirty || self.bridge.texture_id().is_none() {
            if let Some(previous) = self.bridge.texture_id() {
                self.renderer.free_texture(&previous);
            }
            let id = self.renderer.register_native_texture(
                &device,
                self.bridge.colour_texture_view(),
                wgpu::FilterMode::Linear,
            );
            self.bridge.set_texture_id(id);
        }
        for closed in state.closed_documents.drain(..) {
            self.bridge
                .forget_document_thumbnails(closed, &mut self.renderer);
        }
        self.bridge
            .render_thumbnails(&mut encoder, state, &mut self.renderer);
        paint(
            &mut self.renderer,
            &mut encoder,
            &view,
            crate::app::CLEAR_COLOR,
            &primitives,
            &screen,
        );
        let submitting = std::time::Instant::now();
        queue.submit(std::iter::once(encoder.finish()));
        self.submitted.1 = submitting.elapsed();
        // A software adapter draws when it is waited for: a frame it never finishes stops here, where the watch sees it.
        if or_renderer::instance::software_asked() {
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }
        watched::beat();
        let saved = match capture {
            Some(path) => save_png(&device, &queue, target, w, h, path),
            None => Ok(()),
        };
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        saved
    }

    /// A window the editor opens on its own surface, Settings, drawn with its
    /// own context and renderer as that surface has.
    fn window_shot(
        &mut self,
        state: &mut AppState,
        size: (u32, u32),
        path: &Path,
        mut draw: impl FnMut(&egui::Context, &mut AppState),
    ) -> Result<(), String> {
        if self.bridge.device_lost() {
            return Err(lost_device::DEVICE_LOST.to_owned());
        }
        let ctx = crate::app::gpu::configured_egui_context(Some(8192), &state.prefs.appearance);
        let mut renderer =
            egui_wgpu::Renderer::new(&self.device, FORMAT, egui_wgpu::RendererOptions::default());
        let points = egui::vec2(size.0 as f32, size.1 as f32);
        for frame in 0..=SETTLE_FRAMES {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
                max_texture_side: Some(8192),
                time: Some(frame as f64 / 60.0),
                focused: true,
                ..Default::default()
            });
            draw(&ctx, state);
            let output = ctx.end_pass();
            let ppp = output.pixels_per_point;
            let primitives = ctx.tessellate(output.shapes, ppp);
            for (id, delta) in &output.textures_delta.set {
                renderer.update_texture(&self.device, &self.queue, *id, delta);
            }
            let w = (points.x * ppp).round() as u32;
            let h = (points.y * ppp).round() as u32;
            let target = target_texture(&self.device, w, h);
            let view = target.create_view(&Default::default());
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [w, h],
                pixels_per_point: ppp,
            };
            let mut encoder = self.device.create_command_encoder(&Default::default());
            renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &primitives,
                &screen,
            );
            paint(
                &mut renderer,
                &mut encoder,
                &view,
                crate::app::SETTINGS_CLEAR_COLOR,
                &primitives,
                &screen,
            );
            self.queue.submit(std::iter::once(encoder.finish()));
            if frame == SETTLE_FRAMES {
                save_png(&self.device, &self.queue, &target, w, h, path)?;
            }
            for id in &output.textures_delta.free {
                renderer.free_texture(id);
            }
        }
        Ok(())
    }
}
