//! A drag of the pointer, as a script plays one.

use super::Shell;
use crate::state::AppState;

/// The frames the pointer takes from where it is pressed to where it is let go.
const STEPS: u16 = 8;

impl Shell {
    /// Presses the primary button at `from`, moves to `to` and lets go, `modifiers` held throughout.
    pub(super) fn drag(
        &mut self,
        state: &mut AppState,
        from: egui::Pos2,
        to: egui::Pos2,
        modifiers: egui::Modifiers,
    ) -> Result<(), String> {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        self.frame_with(
            state,
            vec![egui::Event::PointerMoved(from)],
            modifiers,
            None,
        )?;
        self.frame_with(state, vec![button(from, true)], modifiers, None)?;
        for step in 1..=STEPS {
            let at = from + (to - from) * (f32::from(step) / f32::from(STEPS));
            self.frame_with(state, vec![egui::Event::PointerMoved(at)], modifiers, None)?;
        }
        self.frame_with(state, vec![button(to, false)], modifiers, None)
    }
}
