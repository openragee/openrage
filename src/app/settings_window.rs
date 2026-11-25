use std::sync::Arc;

use anyhow::Result;
use egui_wgpu::ScreenDescriptor;
use winit::{event::WindowEvent, event_loop::ActiveEventLoop, window::Window};

use super::geometry::{center_inside_bounds, host_window_bounds};
use super::gpu::WindowSurface;
use super::pointer_sync::{reconcile_mouse_buttons, synthesize_pointer_release};
use super::{App, SETTINGS_CLEAR_COLOR, SETTINGS_WINDOW_MIN_SIZE, SETTINGS_WINDOW_SIZE};

pub(super) struct SettingsWindow {
    pub(super) surface: WindowSurface,
}

impl App {
    fn create_settings_window(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self.settings.is_some() {
            return Ok(());
        }
        tracing::debug!("Creating settings window");
        let mut window_attrs = crate::ui::branding::with_taskbar_icon(Window::default_attributes())
            .with_title(tr!("OpenRage — Settings"))
            .with_visible(false)
            .with_window_icon(crate::ui::branding::load_app_logo_window_icon())
            .with_inner_size(winit::dpi::LogicalSize::new(
                SETTINGS_WINDOW_SIZE.0,
                SETTINGS_WINDOW_SIZE.1,
            ))
            .with_min_inner_size(winit::dpi::LogicalSize::new(
                SETTINGS_WINDOW_MIN_SIZE.0,
                SETTINGS_WINDOW_MIN_SIZE.1,
            ));
        let host_bounds = self
            .editor
            .as_ref()
            .and_then(|e| host_window_bounds(&e.surface.window));
        if let Some(pos) = center_inside_bounds(host_bounds, SETTINGS_WINDOW_SIZE, event_loop) {
            window_attrs = window_attrs.with_position(pos);
        }
        let window = Arc::new(event_loop.create_window(window_attrs)?);
        crate::ui::branding::apply_native_window_icons(&window);
        self.init_gpu(&window)?;
        let surface = self.build_window_surface(event_loop, window)?;
        self.settings = Some(SettingsWindow { surface });
        Ok(())
    }

    fn close_settings_window(&mut self) {
        if let Some(settings) = self.settings.take() {
            crate::ui::panels::settings::reset_settings_transient(&settings.surface.egui_ctx);
        }
        crate::ui::panels::settings::set_settings_open(&mut self.app_state, false);
    }

    pub(super) fn sync_settings_window(&mut self, event_loop: &ActiveEventLoop) {
        let want_open = crate::ui::panels::settings::is_settings_open(&self.app_state);
        let raise = crate::ui::panels::settings::take_raise(&mut self.app_state);
        let is_open = self.settings.is_some();
        if want_open && !is_open {
            if let Err(e) = self.create_settings_window(event_loop) {
                tracing::error!("Failed to create settings window: {}", e);
                crate::ui::panels::settings::set_settings_open(&mut self.app_state, false);
            }
        } else if !want_open && is_open {
            self.close_settings_window();
        } else if let (true, Some(settings)) = (raise, self.settings.as_ref()) {
            settings.surface.window.set_minimized(false);
            settings.surface.window.focus_window();
        }
    }

    fn render_settings(&mut self) -> Result<()> {
        // The editor's frame asks for a new device; nothing is drawn on a lost one.
        if self.device_lost() {
            return Ok(());
        }
        let Some(settings) = self.settings.as_mut() else {
            return Ok(());
        };
        let surface = &mut settings.surface;
        let device = self.gpu.as_ref().unwrap().device.clone();
        let queue = self.gpu.as_ref().unwrap().queue.clone();

        let output = match surface.surface.get_current_texture() {
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

        surface.sync_appearance(&self.app_state.prefs.appearance);
        reconcile_mouse_buttons(&surface.egui_ctx, &mut surface.egui_state);

        let raw_input = surface.egui_state.take_egui_input(&surface.window);
        surface.egui_ctx.begin_pass(raw_input);

        crate::ui::panels::settings::draw_settings_window_body(
            &surface.egui_ctx,
            &mut self.app_state,
        );
        // ⚠ Before the pass ends, so the name is in the tree this frame sends.
        crate::ui::accessible_name::name_the_focus(&surface.egui_ctx);

        let full_output = surface.egui_ctx.end_pass();
        surface
            .egui_state
            .handle_platform_output(&surface.window, full_output.platform_output);

        let clipped_primitives = surface
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

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
            label: Some("settings egui encoder"),
        });

        surface.egui_renderer.update_buffers(
            &device,
            &queue,
            &mut encoder,
            &clipped_primitives,
            &screen_descriptor,
        );

        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("settings egui render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SETTINGS_CLEAR_COLOR),
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

        queue.submit(std::iter::once(encoder.finish()));
        output.present();

        for id in &full_output.textures_delta.free {
            surface.egui_renderer.free_texture(id);
        }

        Ok(())
    }

    pub(super) fn dispatch_settings_event(
        &mut self,
        event: WindowEvent,
        _event_loop: &ActiveEventLoop,
    ) {
        let Some(settings) = self.settings.as_mut() else {
            return;
        };

        let response = settings
            .surface
            .egui_state
            .on_window_event(&settings.surface.window, &event);
        if response.consumed {
            return;
        }

        if matches!(
            &event,
            WindowEvent::Focused(false)
                | WindowEvent::CursorLeft { .. }
                | WindowEvent::Occluded(true)
        ) {
            synthesize_pointer_release(&mut settings.surface.egui_state);
        }

        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("Settings window close requested");
                self.close_settings_window();
            }
            WindowEvent::Resized(new_size) if new_size.width > 0 && new_size.height > 0 => {
                let device = self.gpu.as_ref().unwrap().device.clone();
                settings.surface.surface_config.width = new_size.width;
                settings.surface.surface_config.height = new_size.height;
                settings
                    .surface
                    .surface
                    .configure(&device, &settings.surface.surface_config);
            }
            WindowEvent::RedrawRequested => {
                self.app_state.poll_key_extraction();
                if let Err(e) = self.render_settings() {
                    tracing::error!("Settings render error: {}", e);
                }
            }
            _ => {}
        }
    }
}
