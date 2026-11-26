//! The question a GTA VI tree's save asks before its file dialog: which copy it writes, and why.

use crate::state::AppState;

const KEY: &str = "gta6_planned_save";

/// Ask the planned save's question, if one waits.
pub fn draw(ctx: &egui::Context, state: &mut AppState) {
    let Some((title, said)) = state.planned_save_question() else {
        return;
    };
    let mut answer = None;
    super::question_titled(ctx, KEY, title, 520.0, |ui| {
        ui.label(said);
        ui.add_space(crate::ui::theme::SPACING_SNUG);
        ui.horizontal(|ui| {
            if ui.button(tr!("Save as…")).clicked() {
                answer = Some(true);
            }
            if ui.button(tr!("Cancel")).clicked() {
                answer = Some(false);
            }
        });
    });
    if super::question_escaped(ctx, KEY) {
        answer = Some(false);
    }
    if let Some(go) = answer {
        state.answer_planned_save(go);
    }
}
