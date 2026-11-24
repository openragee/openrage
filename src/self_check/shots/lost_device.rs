//! A script's frames on a device, and what becomes of them once it is lost.

use super::*;

/// What a script is told once the device its frames are drawn on is lost.
pub(super) const DEVICE_LOST: &str =
    "the graphics device was lost: nothing more is drawn in this run";

impl Shell {
    /// The editor's frames on `device`, as a script drives them.
    pub(super) fn on(state: &AppState, device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        let mut bridge = RenderBridge::new(device.clone(), queue.clone(), FORMAT);
        // A pack baked elsewhere is named by `OPENRAGE_GAME_PACK`; without one the mode is not offered
        bridge.set_game_pack_path(state.game_pack.root());
        Shell {
            bridge,
            renderer: egui_wgpu::Renderer::new(
                &device,
                FORMAT,
                egui_wgpu::RendererOptions::default(),
            ),
            ctx: crate::app::gpu::configured_egui_context(Some(8192), &state.prefs.appearance),
            device,
            queue,
            size: egui::vec2(1600.0, 900.0),
            time: 0.0,
            target: None,
            dropped: Vec::new(),
            hovering: Vec::new(),
            held: false,
        }
    }

    /// Ends the script with a reason once the device is lost, the view back to Material.
    // ⚠ Asked before anything of a frame is drawn: the painter panics on a buffer a lost
    // device does not write.
    pub(super) fn stop_once_lost(&mut self, state: &mut AppState) -> Result<(), String> {
        if !self.bridge.device_lost() {
            return Ok(());
        }
        state.graphics_device_lost(self.bridge.mode());
        self.bridge.set_mode(ShadingMode::Material);
        Err(DEVICE_LOST.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_ends_with_a_reason_and_no_panic_once_its_device_is_lost() {
        const WHAT: &str = "a_script_ends_with_a_reason_and_no_panic_once_its_device_is_lost";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let _language = crate::i18n::pin("en");
        let mut state = AppState::new();
        let device = Arc::new(device);
        let mut shell = Shell::on(&state, device.clone(), Arc::new(queue));
        shell
            .idle(&mut state, 2)
            .expect("frames on a device that is there");
        assert!(!shell.bridge.device_lost());
        shell.bridge.set_mode(ShadingMode::Shaded);
        // A device destroyed is a device lost: its next poll tells whoever watches it.
        device.destroy();
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        assert!(shell.bridge.device_lost());
        assert_eq!(shell.idle(&mut state, 1), Err(DEVICE_LOST.to_owned()));
        assert_eq!(shell.bridge.mode(), ShadingMode::Material);
        let told = |toast: &crate::state::toasts::Toast| toast.message.contains("device was lost");
        assert!(state.toasts.iter().any(told));
    }
}
