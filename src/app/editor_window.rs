use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use egui_wgpu::ScreenDescriptor;
use winit::{
    event::{KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    window::Window,
};

use super::geometry::{centered_position, is_sane_position};
use super::gpu::WindowSurface;
use super::modals;
use super::pointer_sync::{aim_file_drop, reconcile_mouse_buttons, synthesize_pointer_release};
use super::{App, CLEAR_COLOR, EDITOR_MIN_SIZE, EDITOR_SIZE};
use crate::render::RenderBridge;
use crate::state::frame_stats::FramePhase;
use crate::ui::{commands, shortcuts};

pub(super) struct EditorWindow {
    pub(super) surface: WindowSurface,
    pub(super) bridge: RenderBridge,
}

/// The name a Linux desktop matches a window to its launcher and icon by: a Wayland application id
/// and an X11 `WM_CLASS`, which should match the `.desktop` file a package installs. Winit sets
/// neither unless asked, so a window otherwise carries the executable's name on X11 and none on
/// Wayland.
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
fn named(attributes: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
    const APP_ID: &str = "openrage";
    // Both traits name `with_name`, so each is called by its own.
    let attributes = WindowAttributesExtWayland::with_name(attributes, APP_ID, "");
    WindowAttributesExtX11::with_name(attributes, APP_ID, APP_ID)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
)))]
fn named(attributes: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    attributes
}

impl App {
    pub(super) fn create_editor_window(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self.editor.is_some() {
            return Ok(());
        }
        let saved_size = self.app_state.prefs.editor_window.size;
        let saved_pos = self
            .app_state
            .prefs
            .editor_window
            .position
            .filter(|p| is_sane_position(*p));
        let logical_size = saved_size.unwrap_or(EDITOR_SIZE);
        let mut window_attrs = named(crate::ui::branding::with_taskbar_icon(
            Window::default_attributes(),
        ))
        .with_title(self.app_state.window_title())
        .with_visible(false)
        .with_window_icon(crate::ui::branding::load_app_logo_window_icon())
        .with_inner_size(winit::dpi::LogicalSize::new(logical_size.0, logical_size.1))
        .with_min_inner_size(winit::dpi::LogicalSize::new(
            EDITOR_MIN_SIZE.0,
            EDITOR_MIN_SIZE.1,
        ));
        if let Some((x, y)) = saved_pos {
            window_attrs = window_attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        } else if let Some(pos) = centered_position(event_loop, logical_size) {
            window_attrs = window_attrs.with_position(pos);
        }
        let window = Arc::new(event_loop.create_window(window_attrs)?);
        crate::ui::branding::apply_native_window_icons(&window);
        self.init_gpu(&window)?;
        let surface = self.build_window_surface(event_loop, window)?;
        let gpu = self.gpu.as_ref().unwrap();
        let mut bridge =
            RenderBridge::new(gpu.device.clone(), gpu.queue.clone(), gpu.surface_format)
                .with_instance(gpu.instance.clone());

        bridge.set_game_pack_path(self.app_state.game_pack.root());

        self.editor = Some(EditorWindow { surface, bridge });
        Ok(())
    }

    fn persist_editor_geometry(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let window = &editor.surface.window;
        let outer_pos = window
            .outer_position()
            .ok()
            .map(|p| (p.x, p.y))
            .filter(|p| is_sane_position(*p));
        if outer_pos.is_none() {
            return;
        }
        let scale = window.scale_factor();
        let inner = window.inner_size();
        let logical_w = (inner.width as f64 / scale).round() as u32;
        let logical_h = (inner.height as f64 / scale).round() as u32;
        let prefs = &mut self.app_state.prefs;
        let new_size = Some((
            logical_w.max(EDITOR_MIN_SIZE.0),
            logical_h.max(EDITOR_MIN_SIZE.1),
        ));
        let changed =
            prefs.editor_window.size != new_size || prefs.editor_window.position != outer_pos;
        if !changed {
            return;
        }
        prefs.editor_window.size = new_size;
        prefs.editor_window.position = outer_pos;
        crate::state::save_prefs(prefs);
    }

    /// Runs a key press's command, and takes the key from the focused widget.
    fn handle_keyboard_shortcut(&mut self, event: &KeyEvent, typing: bool) {
        let Some(action) = shortcuts::action_for_press(
            &event.logical_key,
            event.physical_key,
            self.modifiers,
            typing,
            &self.app_state.prefs,
        ) else {
            self.handle_plugin_shortcut(event, typing);
            return;
        };
        if commands::spec(action).is_some_and(|spec| spec.panel_key) {
            return;
        }
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        commands::run_action(&mut self.app_state, &editor.surface.egui_ctx, action);
        if let Some(key) = shortcuts::key_of(&event.logical_key, event.physical_key) {
            take_key_press(&mut editor.surface.egui_state, key);
        }
    }

    /// Runs the command of a plugin that a key press names, when no command of the app's own takes the key.
    fn handle_plugin_shortcut(&mut self, event: &KeyEvent, typing: bool) {
        let Some((plugin, command)) = shortcuts::plugin_command_for_press(
            &event.logical_key,
            event.physical_key,
            self.modifiers,
            typing,
            &self.app_state,
        ) else {
            return;
        };
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if crate::ui::panels::question_waits(&editor.surface.egui_ctx) {
            return;
        }
        self.app_state.run_plugin_command(&plugin, &command);
        if let Some(key) = shortcuts::key_of(&event.logical_key, event.physical_key) {
            take_key_press(&mut editor.surface.egui_state, key);
        }
    }

    /// A new device once the one every window drew on is lost, with each window's surface and
    /// painter and the viewport on it; the view goes back to Material and a person is told.
    // ⚠ Never run: it takes a window and a lost device. What is exercised without them is the
    // flag, what the state does, and the windowless frames of a script.
    fn recover_lost_device(&mut self) -> Result<()> {
        let now = Instant::now();
        if self.device_retry.is_some_and(|at| now < at) {
            return Ok(());
        }
        let Some(editor) = self.editor.as_ref() else {
            return Ok(());
        };
        if self.device_retry.is_none() {
            self.app_state.graphics_device_lost(editor.bridge.mode());
        }
        self.device_retry = Some(now + std::time::Duration::from_secs(2));
        let window = editor.surface.window.clone();
        let lost = self.gpu.take();
        if let Err(why) = self.init_gpu(&window) {
            self.gpu = lost;
            return Err(why);
        }
        let (Some(gpu), Some(editor)) = (self.gpu.as_ref(), self.editor.as_mut()) else {
            return Ok(());
        };
        let appearance = &self.app_state.prefs.appearance;
        editor.surface.on_new_device(gpu, appearance)?;
        if let Some(settings) = self.settings.as_mut() {
            settings.surface.on_new_device(gpu, appearance)?;
        }
        let mut bridge =
            RenderBridge::new(gpu.device.clone(), gpu.queue.clone(), gpu.surface_format)
                .with_instance(gpu.instance.clone());
        bridge.set_game_pack_path(self.app_state.game_pack.root());
        bridge.set_mode(or_renderer::ShadingMode::Material);
        editor.bridge = bridge;
        self.device_retry = None;
        Ok(())
    }

    /// Whether the device the windows draw on is lost.
    pub(super) fn device_lost(&self) -> bool {
        (self.editor.as_ref()).is_some_and(|editor| editor.bridge.device_lost())
    }

    fn render_editor(&mut self) -> Result<()> {
        // ⚠ Before anything is drawn: the painter panics on a buffer a lost device does not write.
        if self.device_lost() {
            self.recover_lost_device()?;
            if self.device_lost() {
                return Ok(());
            }
        }
        let Some(editor) = self.editor.as_mut() else {
            return Ok(());
        };
        let surface = &mut editor.surface;
        let device = self.gpu.as_ref().unwrap().device.clone();
        let queue = self.gpu.as_ref().unwrap().queue.clone();

        let started = Instant::now();
        let acquired = surface.surface.get_current_texture();
        self.app_state
            .frame_stats
            .record(FramePhase::Acquire, started.elapsed());
        let output = match acquired {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.surface.configure(&device, &surface.surface_config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        editor.bridge.begin_frame();

        let started = Instant::now();
        self.app_state.tick_previews();
        self.app_state
            .frame_stats
            .record(FramePhase::Simulation, started.elapsed());

        let started = Instant::now();
        surface.sync_appearance(&self.app_state.prefs.appearance);
        reconcile_mouse_buttons(&surface.egui_ctx, &mut surface.egui_state);
        aim_file_drop(&surface.egui_ctx, &mut surface.egui_state, &surface.window);

        let raw_input = surface.egui_state.take_egui_input(&surface.window);
        surface.egui_ctx.begin_pass(raw_input);

        crate::ui::draw(
            &surface.egui_ctx,
            &mut self.app_state,
            Some(&mut editor.bridge),
        );

        modals::draw_editor_overlays(&surface.egui_ctx, &mut self.app_state);

        let full_output = surface.egui_ctx.end_pass();
        surface
            .egui_state
            .handle_platform_output(&surface.window, full_output.platform_output);

        let clipped_primitives = surface
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        self.app_state
            .frame_stats
            .record(FramePhase::UiLayout, started.elapsed());

        let started = Instant::now();
        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [surface.surface_config.width, surface.surface_config.height],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, image_delta) in &full_output.textures_delta.set {
            surface
                .egui_renderer
                .update_texture(&device, &queue, *id, image_delta);
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor egui encoder"),
        });

        surface.egui_renderer.update_buffers(
            &device,
            &queue,
            &mut encoder,
            &clipped_primitives,
            &screen_descriptor,
        );

        let dirty = editor.bridge.end_frame(&mut encoder, &mut self.app_state);
        if dirty || editor.bridge.texture_id().is_none() {
            if let Some(prev) = editor.bridge.texture_id() {
                surface.egui_renderer.free_texture(&prev);
            }
            let tex_id = surface.egui_renderer.register_native_texture(
                &device,
                editor.bridge.colour_texture_view(),
                wgpu::FilterMode::Linear,
            );
            editor.bridge.set_texture_id(tex_id);
        }

        for closed in self.app_state.closed_documents.drain(..) {
            editor
                .bridge
                .forget_document_thumbnails(closed, &mut surface.egui_renderer);
        }

        editor
            .bridge
            .render_thumbnails(&mut encoder, &self.app_state, &mut surface.egui_renderer);

        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("editor egui render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            surface.egui_renderer.render(
                &mut render_pass.forget_lifetime(),
                &clipped_primitives,
                &screen_descriptor,
            );
        }

        self.app_state
            .frame_stats
            .record(FramePhase::RenderEncode, started.elapsed());

        let started = Instant::now();
        queue.submit(std::iter::once(encoder.finish()));
        output.present();
        self.app_state
            .frame_stats
            .record(FramePhase::Submit, started.elapsed());

        for id in &full_output.textures_delta.free {
            surface.egui_renderer.free_texture(id);
        }

        Ok(())
    }

    pub(super) fn dispatch_editor_event(
        &mut self,
        event: WindowEvent,
        event_loop: &ActiveEventLoop,
    ) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };

        let typing = editor.surface.egui_ctx.egui_wants_keyboard_input();
        let response = editor
            .surface
            .egui_state
            .on_window_event(&editor.surface.window, &event);
        // ⚠ egui reports every key consumed while a widget has keyboard focus;
        // the shortcuts decide which of those keys the app still takes.
        if response.consumed && !matches!(event, WindowEvent::KeyboardInput { .. }) {
            return;
        }

        if matches!(
            &event,
            WindowEvent::Focused(false)
                | WindowEvent::CursorLeft { .. }
                | WindowEvent::Occluded(true)
        ) {
            synthesize_pointer_release(&mut editor.surface.egui_state);
        }

        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("Editor close requested");
                self.app_state.request_exit();
            }
            WindowEvent::Resized(new_size) if new_size.width > 0 && new_size.height > 0 => {
                let device = self.gpu.as_ref().unwrap().device.clone();
                editor.surface.surface_config.width = new_size.width;
                editor.surface.surface_config.height = new_size.height;
                editor
                    .surface
                    .surface
                    .configure(&device, &editor.surface.surface_config);
                tracing::debug!("Editor resized to {}x{}", new_size.width, new_size.height);
                self.persist_editor_geometry();
            }
            WindowEvent::Moved(_) => {
                self.persist_editor_geometry();
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                self.handle_keyboard_shortcut(&event, typing);
            }
            WindowEvent::RedrawRequested => {
                let started = Instant::now();
                self.app_state.poll_load_result();
                self.app_state.poll_save_result();
                self.app_state
                    .drive_close(crate::ui::panels::menu::save_dialog_for);
                self.app_state.poll_key_extraction();
                self.app_state.poll_material_palette_load();
                self.app_state.poll_clip_regions_load();
                self.app_state.poll_shader_definitions();
                self.app_state.poll_game_discovery();
                self.app_state.poll_expansions();
                self.app_state.tick_navigation();
                self.app_state.poll_extract_job();
                self.app_state.poll_jobs();
                let game_pack = self
                    .app_state
                    .maybe_auto_bake_game_pack()
                    .or_else(|| self.app_state.poll_game_pack());
                if let (Some(root), Some(editor)) = (game_pack, self.editor.as_mut()) {
                    editor.bridge.set_game_pack_path(&root);
                }
                self.app_state.load_plugin_surfaces();
                self.app_state.update_toasts();
                self.app_state
                    .frame_stats
                    .record(FramePhase::Updates, started.elapsed());
                self.app_state.tick_frame_stats();
                if let Err(e) = self.render_editor() {
                    tracing::error!("Editor render error: {}", e);
                }
                if let (Some(title), Some(editor)) =
                    (self.app_state.changed_window_title(), self.editor.as_ref())
                {
                    editor.surface.window.set_title(&title);
                }
                let staged = crate::ui::os_drop::take_drag_out(&mut self.app_state);
                if let (false, Some(editor)) = (staged.is_empty(), self.editor.as_ref()) {
                    if let Err(e) =
                        crate::platform::drag_drop::drag_files_out(&editor.surface.window, &staged)
                    {
                        self.app_state.show_error_toast(e);
                    }
                }
            }
            _ => {}
        }
        if std::mem::take(&mut self.app_state.should_exit) {
            tracing::info!("Exiting");
            crate::ui::dock_layout::flush(&mut self.app_state);
            event_loop.exit();
        }
    }
}

/// Drops the press egui queued for a key the app took.
///
/// ⚠ One press runs one command: an egui text field also reads `Ctrl+W`, as
/// delete the previous word.
fn take_key_press(state: &mut egui_winit::State, key: egui::Key) {
    let events = &mut state.egui_input_mut().events;
    let queued = events
        .iter()
        .rposition(|e| matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key));
    if let Some(at) = queued {
        events.remove(at);
    }
}
