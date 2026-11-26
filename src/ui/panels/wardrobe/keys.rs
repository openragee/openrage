//! The keys of the list: Up and Down walk it and Shift spans, Enter opens the model, Space looks at the garment.
//!
//! ⚠ Read while the pointer is over the list and no text is being typed, as the Assets table reads its keys. Delete and F2 are
//! commands of the registry that act on the garments picked, so they are not read here.

use super::WardrobeUi;
use crate::state::AppState;
use crate::ui::commands::{self, Action};
use crate::ui::shortcuts;

pub(super) fn handle(
    ui: &mut egui::Ui,
    state: &mut AppState,
    shown: &mut WardrobeUi,
    module: &str,
) {
    if !ui.rect_contains_pointer(ui.max_rect()) || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (down, up, enter, shift, asked) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::Enter),
            i.modifiers.shift,
            i.events.iter().find_map(|event| {
                shortcuts::action_for_event(crate::ui::commands::Place::Lists, event, &state.prefs)
            }),
        )
    });
    if down || up {
        walk(state, shown, module, down, shift);
    }
    if enter {
        if let [only] = state.picked_garments().as_slice() {
            state.open_garment_model(&only.clone());
        }
    }
    if asked == Some(Action::QuickLook) {
        commands::run_action(state, ui.ctx(), Action::QuickLook);
    }
}

/// Move the garment selected one place down or up the list, or span to it; the first garment when none is selected.
fn walk(state: &mut AppState, shown: &mut WardrobeUi, module: &str, down: bool, span: bool) {
    let Some(order) = shown
        .cache
        .as_ref()
        .map(|c| &c.order)
        .filter(|o| !o.is_empty())
    else {
        return;
    };
    let at = (state.workbench.wardrobe.selected.as_ref())
        .and_then(|(_, id)| order.iter().position(|o| o == id));
    let to = match (at, down) {
        (None, _) => 0,
        (Some(at), true) => (at + 1).min(order.len() - 1),
        (Some(at), false) => at.saturating_sub(1),
    };
    let id = order[to].clone();
    match span && at.is_some() {
        true => {
            let order = order.clone();
            state.span_garments(module, &order, id.clone());
        }
        false => state.select_garment(module, Some(id.clone())),
    }
    shown.view.scroll_to = Some(id);
}
