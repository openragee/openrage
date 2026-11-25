//! What every archive row does, whichever view drew it: selection clicks,
//! reveals, and a dragged row let go over a folder.

use super::*;
use crate::ui::panel_registry::PanelId;

/// Start a frame's rows for one archive: forget last frame's order, and work
/// out which folders a pending reveal has to open.
pub(super) fn begin_rows(state: &mut AppState, doc_id: DocumentId) {
    let mut ancestors = std::collections::HashSet::new();
    if let Some(target) = state.navigation.reveal.filter(|l| l.doc == doc_id) {
        if let Some(DocPayload::Rpf(rpf)) = state.document(doc_id).map(|d| &d.payload) {
            let mut at = rpf.session.node(target.node).and_then(|n| n.parent);
            while let Some(parent) = at {
                ancestors.insert(parent);
                at = rpf.session.node(parent).and_then(|n| n.parent);
            }
        }
    }
    let ui = super::ui_state(state);
    ui.visible_rows.insert(doc_id, Vec::new());
    ui.reveal_ancestors = ancestors;
}

/// Apply a shift-click once every row of the frame has been drawn, so the
/// range spans the rows below the click as well as above it.
pub(super) fn finish_rows(state: &mut AppState, doc_id: DocumentId) {
    let ui = super::ui_state(state);
    let Some((_, target)) = ui.pending_range.take_if(|(d, _)| *d == doc_id) else {
        return;
    };
    let order = ui.visible_rows.get(&doc_id).cloned().unwrap_or_default();
    state.selection.select_range(doc_id, target, &order);
}

/// Ctrl toggles a row and shift spans to it; either is a selection edit rather
/// than a navigation. Returns whether the click was one.
pub(super) fn selection_modifier_click(
    ctx: &egui::Context,
    state: &mut AppState,
    doc_id: DocumentId,
    node_id: RpfNodeId,
) -> bool {
    let modifiers = ctx.input(|i| i.modifiers);
    if modifiers.shift {
        super::ui_state(state).pending_range = Some((doc_id, node_id));
    } else if modifiers.command || modifiers.ctrl {
        state.selection.toggle(doc_id, node_id);
    } else {
        return false;
    }
    super::ui_state(state).cancel_pending_rename();
    true
}

/// Select a row and preview it: in its archive where the workspace on screen
/// shows archives, else in the preview tab where it shows the row's kind.
///
/// ⚠ Anywhere else the archive taking over as the active document would carry
/// the user off what they were looking at, such as a drawable in Model; there a
/// click opens only a file of a kind the workspace shows, and in a preview tab.
pub(super) fn select_row(state: &mut AppState, doc_id: DocumentId, node_id: RpfNodeId) {
    state.selection.select(doc_id, node_id);
    state.activate_if_shown(doc_id);
    state.preview_row(doc_id, node_id);
}

pub(super) fn select_and_visit(state: &mut AppState, doc_id: DocumentId, node_id: RpfNodeId) {
    select_row(state, doc_id, node_id);
    state.navigation.visit(Location {
        doc: doc_id,
        node: node_id,
    });
}

/// A right click or a drag on a row outside the selection selects that row alone, as Explorer's
/// do; on a row inside it, the whole selection stays for the menu or the drag to act on.
pub(super) fn select_unless_selected(state: &mut AppState, doc_id: DocumentId, node_id: RpfNodeId) {
    if !state.selection.is_selected(doc_id, node_id) {
        state.selection.select(doc_id, node_id);
        // ⚠ A rename a second click scheduled would open on a row no longer selected.
        super::ui_state(state).cancel_pending_rename();
    }
}

/// A row clicked in the Files view leaves the folder it shows (`shown`) out of the selection.
///
/// ⚠ Going into that folder selected it, and it is not one of the rows listed: held with them it
/// would carry every row in it, the ones not chosen too, or, as the archive's root, none.
pub(super) fn leave_folder_shown(
    state: &mut AppState,
    doc_id: DocumentId,
    shown: Option<RpfNodeId>,
) {
    if let Some(folder) = shown.filter(|&folder| state.selection.is_selected(doc_id, folder)) {
        state.selection.toggle(doc_id, folder);
    }
}

pub(super) fn is_rpf9(state: &AppState, doc: DocumentId, node: RpfNodeId) -> bool {
    matches!(state.document(doc).map(|doc| &doc.payload), Some(DocPayload::Rpf(rpf)) if rpf.session.is_rpf9(node))
}

/// A row starts being dragged: it is selected as a right click selects it, and `panel` takes focus.
pub(super) fn start_drag(
    ctx: &egui::Context,
    state: &mut AppState,
    doc_id: DocumentId,
    node_id: RpfNodeId,
    panel: PanelId,
) {
    if is_rpf9(state, doc_id, node_id)
        || state
            .selection
            .selected_ids(doc_id)
            .iter()
            .any(|&id| is_rpf9(state, doc_id, id))
    {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("rpf_drag", doc_id)), node_id));
    select_unless_selected(state, doc_id, node_id);
    // ⚠ A second click on a row already selected waits to open its name, and would open it mid-drag.
    super::ui_state(state).cancel_pending_rename();
    focus(state, panel);
}

/// Files dropped from outside the window onto `panel`, which `ui` draws, give it focus.
pub(super) fn focus_on_drop(ui: &egui::Ui, state: &mut AppState, panel: PanelId) {
    if ui.ctx().input(|i| !i.raw.dropped_files.is_empty()) && ui.ui_contains_pointer() {
        focus(state, panel);
    }
}

/// Give `panel` focus once the dock is drawn, as a click in it does: a drag or a drop clicks nothing.
fn focus(state: &mut AppState, panel: PanelId) {
    let ws = state.active_workspace;
    // ⚠ Only when another panel has it: a reveal puts back the arrangement under a panel shown alone.
    if state.workspace_docks.focused(ws) != Some(panel) {
        state.workspace_docks.reveal(ws, panel);
    }
}

/// Where a row landed this frame, for a drop from outside the window to find
/// the folder under the pointer.
pub(super) fn note_row(
    state: &mut AppState,
    doc_id: DocumentId,
    node_id: RpfNodeId,
    rect: egui::Rect,
) {
    super::ui_state(state)
        .row_rects
        .push((doc_id, node_id, rect));
}

/// Move the rows being dragged into this folder, or copy them with Ctrl held at the drop, if they
/// were let go over it: the selection, when the row dragged is in it, as a drag out to Explorer takes.
///
/// A release anywhere else is not this folder's to act on; the drag is
/// forgotten once the frame ends, by `ui::os_drop`.
pub(super) fn maybe_drop_into_folder(
    ctx: &egui::Context,
    state: &mut AppState,
    doc_id: DocumentId,
    folder_id: RpfNodeId,
    row: egui::Rect,
) {
    if is_rpf9(state, doc_id, folder_id) {
        return;
    }
    let drag_key = egui::Id::new(("rpf_drag", doc_id));
    let released_here = ctx.input(|i| {
        i.pointer.any_released() && i.pointer.interact_pos().is_some_and(|p| row.contains(p))
    });
    if !released_here {
        return;
    }
    let dragged = ctx.data(|d| d.get_temp::<RpfNodeId>(drag_key));
    let Some(dragged_id) = dragged else {
        return;
    };
    let rows = state.selection.dragged(doc_id, dragged_id);
    // ⚠ Let go over a row it carries, a drag went nowhere, as in Explorer.
    if !rows.contains(&folder_id) {
        let done = if ctx.input(|i| i.modifiers.ctrl) {
            state.rpf_copy_nodes(doc_id, &rows, folder_id)
        } else {
            state.rpf_move_nodes(doc_id, &rows, folder_id)
        };
        if let Err(e) = done {
            state.show_error_toast(e);
        }
    }
    ctx.data_mut(|d| d.remove::<RpfNodeId>(drag_key));
}

/// Start a frame's row rectangles, once per pass whichever view draws first.
pub(super) fn start_row_rects(state: &mut AppState, pass: u64) {
    let ui = super::ui_state(state);
    if ui.rows_pass != pass {
        ui.row_rects.clear();
        ui.rows_pass = pass;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_clicked_in_browse_previews_and_elsewhere_the_document_on_screen_stays() {
        let mut state = AppState::default();
        let archive = state.new_empty_document(DocumentKind::Rpf);
        let Some(DocPayload::Rpf(rpf)) = state.document_mut(archive).map(|d| &mut d.payload) else {
            panic!("an empty archive document holds an archive");
        };
        let root = rpf.session.root_id();
        let row = rpf
            .session
            .add_file_copy(root, "a.meta", b"<Item />".to_vec())
            .expect("a file in the archive");

        state.new_empty_document(DocumentKind::Text);
        state.active_workspace = Workspace::Browse;
        select_and_visit(&mut state, archive, row);
        assert_eq!(
            state.active_document,
            Some(archive),
            "the row's archive is not the active document"
        );
        assert_eq!(state.preview_target(), Some((archive, row)));

        let drawable = state.new_empty_document(DocumentKind::Ydr);
        assert_eq!(state.active_workspace, Workspace::Model);
        select_and_visit(&mut state, archive, row);
        assert_eq!(
            (state.active_document, state.active_workspace),
            (Some(drawable), Workspace::Model),
            "a click in the tree took the drawable off screen"
        );
        assert_eq!(state.selection.selected(archive), Some(row));
    }
}
