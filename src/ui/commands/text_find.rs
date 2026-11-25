//! The find bar of a text document, which the Find commands open.

use crate::state::AppState;
use crate::ui::panels;

/// The editor of the active text document, else of the first one open, which it makes active.
pub(super) fn text_editor_for(
    ctx: &egui::Context,
    state: &mut AppState,
) -> Option<panels::text_editor::editor_state::EditorStateHandle> {
    let is_text = |state: &AppState, id| {
        state
            .document(id)
            .is_some_and(|d| d.payload.found_in().is_some())
    };
    let target = state
        .active_document
        .filter(|id| is_text(state, *id))
        .or_else(|| {
            state
                .documents
                .iter()
                .find(|d| d.payload.found_in().is_some())
                .map(|d| d.id)
        });
    let doc_id = target?;
    state.set_active_document(doc_id);
    Some(panels::text_editor::editor_state::load_state(ctx, doc_id))
}

/// Opens the find bar of the active text document, else of the first one open.
pub(super) fn find_in_text(ctx: &egui::Context, state: &mut AppState, replace: bool) {
    use panels::text_editor::editor_state::FindMode;

    let Some(handle) = text_editor_for(ctx, state) else {
        return;
    };
    let Ok(mut editor) = handle.lock() else {
        return;
    };
    editor.find.visible = true;
    editor.find.mode = if replace {
        FindMode::Replace
    } else {
        FindMode::Find
    };
    editor.find.want_focus_query = true;
}
