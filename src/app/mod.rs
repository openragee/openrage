pub(crate) mod accessibility;
mod editor_window;
mod geometry;
pub(crate) mod gpu;
pub(crate) mod modals;
mod pointer_sync;
mod settings_window;

use anyhow::Result;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::ModifiersState,
    window::WindowId,
};

use crate::state::AppState;
use accessibility::UserEvent;
use editor_window::EditorWindow;
use gpu::SharedGpu;
use settings_window::SettingsWindow;

pub const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.176,
    g: 0.176,
    b: 0.188,
    a: 1.0,
};

pub(crate) const SETTINGS_CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.086,
    g: 0.094,
    b: 0.110,
    a: 1.0,
};

pub(crate) const EDITOR_SIZE: (u32, u32) = (1600, 900);
pub(crate) const EDITOR_MIN_SIZE: (u32, u32) = (1280, 720);

pub(crate) const SETTINGS_WINDOW_SIZE: (u32, u32) = (760, 580);
const SETTINGS_WINDOW_MIN_SIZE: (u32, u32) = (560, 400);

pub fn run(open: Vec<std::path::PathBuf>) -> Result<()> {
    #[cfg(target_os = "linux")]
    if let Some(message) = crate::platform::runtime_libraries::missing_here() {
        anyhow::bail!(message);
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new(event_loop.create_proxy());
    app.app_state.open_from_command_line(open);
    event_loop.run_app(&mut app)?;

    Ok(())
}

struct App {
    gpu: Option<SharedGpu>,
    editor: Option<EditorWindow>,
    settings: Option<SettingsWindow>,
    app_state: AppState,
    modifiers: ModifiersState,
    /// How a window's AccessKit adapter reaches the event loop.
    proxy: EventLoopProxy<UserEvent>,
    /// When a new device is next asked for, while the one the windows drew on is lost.
    device_retry: Option<std::time::Instant>,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            gpu: None,
            editor: None,
            settings: None,
            app_state: AppState::new(),
            modifiers: ModifiersState::empty(),
            proxy,
            device_retry: None,
        }
    }

    fn window_kind_for(&self, id: WindowId) -> Option<WindowKind> {
        if self
            .editor
            .as_ref()
            .map(|e| e.surface.window.id() == id)
            .unwrap_or(false)
        {
            return Some(WindowKind::Editor);
        }
        if self
            .settings
            .as_ref()
            .map(|s| s.surface.window.id() == id)
            .unwrap_or(false)
        {
            return Some(WindowKind::Settings);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowKind {
    Editor,
    Settings,
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor.is_none() {
            if let Err(e) = self.create_editor_window(event_loop) {
                tracing::error!("Failed to create editor window: {}", e);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        let UserEvent::AccessKit(event) = event;
        let surface = match self.window_kind_for(event.window_id) {
            Some(WindowKind::Editor) => self.editor.as_mut().map(|e| &mut e.surface),
            Some(WindowKind::Settings) => self.settings.as_mut().map(|s| &mut s.surface),
            None => None,
        };
        if let Some(surface) = surface {
            if let Some(request) = accessibility::on_event(&surface.egui_ctx, event.window_event) {
                surface.egui_state.on_accesskit_action_request(request);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match self.window_kind_for(window_id) {
            Some(WindowKind::Editor) => self.dispatch_editor_event(event, event_loop),
            Some(WindowKind::Settings) => self.dispatch_settings_event(event, event_loop),
            None => {}
        }
        self.sync_settings_window(event_loop);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(editor) = self.editor.as_ref() {
            editor.surface.window.request_redraw();
        }
        if let Some(settings) = self.settings.as_ref() {
            settings.surface.window.request_redraw();
        }
    }
}
