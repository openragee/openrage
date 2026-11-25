//! The archive tree of Files: every row listed each frame, and the rows in view drawn.

use super::rows::{
    maybe_drop_into_folder, note_row, select_and_visit, selection_modifier_click, start_drag,
};
use super::rpf::{classify_rename_response, finish_rename, rpf_context_menu, rpf_path_supported};
use super::*;
use crate::ui::commands::{self, Action, Command, Target};
use crate::ui::panels::registry::ids;

/// A row of the tree as one frame lists it.
pub(super) struct ListedRow {
    pub(super) node: RpfNodeId,
    /// Folders above it under the archive's root.
    depth: usize,
    /// What its open or closed state is kept under.
    id: egui::Id,
    open: bool,
    /// The line under a folder still being read, in place of its rows.
    pub(super) pending: bool,
}

fn needs_expand(node: &RpfSessionNode) -> bool {
    node.backing.needs_scan()
}

/// Every row the tree shows, top to bottom: the rows under each open folder, less the files the filter drops.
///
/// ⚠ The whole list each frame and nothing drawn: a row costs a map lookup here and a dozen
/// widgets on screen, so only the rows in view are drawn from it.
pub(super) fn list_rows(
    ctx: &egui::Context,
    state: &mut AppState,
    doc_id: DocumentId,
    id_root: egui::Id,
) -> Vec<ListedRow> {
    let mut rows = Vec::new();
    let mut expand = Vec::new();
    if let Some(DocPayload::Rpf(rpf)) = state.document(doc_id).map(|d| &d.payload) {
        let session = &rpf.session;
        let sort = state.prefs.explorer.sort;
        let files = super::ui_state_ref(state);
        let needle = files.map(|f| f.filter.to_ascii_lowercase());
        let needle = needle.as_deref().unwrap_or_default();
        let pulse = pulse_state_from_ctx(ctx);
        // A row waiting to be listed: its depth, its parent's id, and whether it is shown or only closed.
        let mut waiting: Vec<(RpfNodeId, usize, egui::Id, bool)> =
            explorer::sorted_children(session, session.root_id(), sort)
                .into_iter()
                .rev()
                .map(|child| (child, 0, id_root, true))
                .collect();
        while let Some((id, depth, parent, shown)) = waiting.pop() {
            let Some(node) = session.node(id) else {
                continue;
            };
            let id_here = parent.with(("rpf_node", id));
            if !node.is_dir() {
                if shown && (needle.is_empty() || node.name.to_ascii_lowercase().contains(needle)) {
                    rows.push(ListedRow {
                        node: id,
                        depth,
                        id: id_here,
                        open: false,
                        pending: false,
                    });
                }
                continue;
            }
            let mut kept = egui::collapsing_header::CollapsingState::load_with_default_open(
                ctx, id_here, false,
            );
            let was_open = kept.is_open();
            let revealed = files.is_some_and(|f| f.reveal_ancestors.contains(&id));
            let open = revealed || pulse.unwrap_or(was_open);
            if open != was_open {
                kept.set_open(open);
                kept.store(ctx);
            }
            if shown {
                rows.push(ListedRow {
                    node: id,
                    depth,
                    id: id_here,
                    open,
                    pending: false,
                });
            }
            // ⚠ Collapse closes the folders inside the ones it closes too, so they open closed.
            let closing = was_open && pulse == Some(false);
            let shown = shown && open;
            if !shown && !closing {
                continue;
            }
            if shown && needs_expand(node) {
                expand.push(id);
            }
            let children = explorer::sorted_children(session, id, sort);
            if shown && children.is_empty() && state.expander.is_in_flight(doc_id, id) {
                rows.push(ListedRow {
                    node: id,
                    depth: depth + 1,
                    id: id_here,
                    open: false,
                    pending: true,
                });
            }
            waiting.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, depth + 1, id_here, shown)),
            );
        }
    }
    for id in expand {
        state.request_expand(doc_id, id);
    }
    let order = rows.iter().filter(|row| !row.pending).map(|row| row.node);
    let order = order.collect();
    super::ui_state(state).visible_rows.insert(doc_id, order);
    rows
}

/// A click on a row with no modifier held: selects it, `single` then, and `open` on a real double click.
#[allow(clippy::too_many_arguments)]
fn row_clicked(
    ui: &egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    node: &RpfSessionNode,
    resp: &egui::Response,
    sole: bool,
    open: impl FnOnce(&mut AppState),
    single: impl FnOnce(&mut AppState),
) {
    let node_id = node.id;
    if !resp.clicked() || selection_modifier_click(ui.ctx(), state, doc_id, node_id) {
        return;
    }
    let now = ui.ctx().input(|i| i.time);
    let real_multi =
        is_multi_click(resp) && super::ui_state(state).previous_click_was_on(doc_id, node_id, now);
    if real_multi {
        if super::ui_state(state).allow_multi_click(doc_id, node_id, now) {
            super::ui_state(state).note_multi_click(doc_id, node_id, now);
            super::ui_state(state).cancel_pending_rename();
            select_and_visit(state, doc_id, node_id);
            open(state);
        }
    } else {
        select_and_visit(state, doc_id, node_id);
        single(state);
        if sole && !super::rows::is_rpf9(state, doc_id, node_id) {
            super::ui_state(state).schedule_rename(doc_id, node_id, node.name.clone(), now);
        } else {
            super::ui_state(state).cancel_pending_rename();
        }
    }
    super::ui_state(state).note_click(doc_id, node_id, now);
}

/// One row of the tree, in the strip `ui` is.
pub(super) fn draw_row(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    row: &ListedRow,
    asked: &mut Option<Command>,
) {
    let node_id = row.node;
    let indent = ui.spacing().indent;
    // The line egui draws down the left of an indented block, a level at a time.
    let strip = ui.max_rect();
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let gap = ui.spacing().item_spacing.y;
    for level in 0..row.depth {
        let x = strip.left() + (level as f32 + 0.5) * indent;
        ui.painter()
            .vline(x, strip.top()..=strip.bottom() + gap, stroke);
    }
    ui.add_space(row.depth as f32 * indent);
    if row.pending {
        ui.label(
            egui::RichText::new("…")
                .small()
                .color(theme::TEXT_SECONDARY),
        );
        return;
    }
    let Some((node, unsaved, size, bound_doc_id)) =
        state.document(doc_id).and_then(|d| match &d.payload {
            DocPayload::Rpf(rpf) => {
                let node = rpf.session.node(node_id)?.clone();
                let size = (!node.is_dir())
                    .then(|| rpf.session.size_for(node_id))
                    .flatten();
                Some((
                    node,
                    rpf.session.archive_unsaved(node_id),
                    size,
                    rpf.entry_documents.get(&node_id).copied(),
                ))
            }
            _ => None,
        })
    else {
        return;
    };
    let selected = state.selection.is_selected(doc_id, node_id);
    let sole = state.selection.selected_ids(doc_id) == [node_id];
    let in_flight = state.expander.is_in_flight(doc_id, node_id);

    let mut taken_rename: Option<RenameState> = if super::ui_state(state)
        .is_renaming_rpf(doc_id, node_id)
        && !super::rows::is_rpf9(state, doc_id, node_id)
    {
        super::ui_state(state).renaming.take()
    } else {
        None
    };
    let renaming = taken_rename.is_some();
    let mut rename_action = InlineRenameAction::Editing;

    if node.is_dir() {
        let (archive_open_error, archive_recovered_hint) = match &node.backing {
            RpfNodeBacking::Archive {
                open_error,
                recovered_hint,
                ..
            } => (open_error.as_deref(), recovered_hint.as_deref()),
            RpfNodeBacking::Rpf9Archive { open_error, .. } => (open_error.as_deref(), None),
            _ => (None, None),
        };
        let outgrown = node
            .backing
            .fs_path()
            .is_some_and(|path| state.overlay_copy.stale_copy(path).is_some());
        let label_color = if selected {
            theme::ACCENT_TEXT
        } else if archive_open_error.is_some() {
            theme::TEXT_SECONDARY
        } else if outgrown {
            theme::WARNING
        } else {
            theme::TEXT_PRIMARY
        };

        let mut kept = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            row.id,
            false,
        );
        let spacing = std::mem::take(&mut ui.spacing_mut().item_spacing.x);
        let toggle = kept.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
        ui.spacing_mut().item_spacing.x = spacing;
        if toggle.clicked() {
            kept.store(ui.ctx());
        }
        crate::ui::accessible_name::named(
            toggle,
            if row.open {
                tr!("Collapse")
            } else {
                tr!("Expand")
            },
        );
        let resp = if let Some(rename) = taken_rename.as_mut() {
            let r = inline_rename_text_edit(ui, rename);
            rename_action = classify_rename_response(ui, &r);
            r
        } else {
            let icon = icons::rpf_node_icon(&node, unsaved, in_flight, true, row.open);
            let job = icons::icon_label(&icon, &node.name, label_color.into());
            ui.selectable_label(selected, job)
        };
        note_row(state, doc_id, node_id, resp.rect);
        let resp = resp.interact(egui::Sense::click_and_drag());
        let resp = if let Some((index, hash)) = node.rpf9_entry() {
            resp.on_hover_text(format!(
                "{}\n#{index} · {hash:016X}",
                crate::state::files_view::type_in_words(&node)
            ))
        } else {
            resp
        };
        if !renaming {
            let table_error = matches!(node.backing, RpfNodeBacking::Rpf9Archive { .. })
                || archive_open_error.is_some_and(crate::state::gta_vi::is_rpf9_refusal);
            let resp = if let Some(err) = archive_open_error.filter(|_| table_error) {
                resp.on_hover_text(err)
            } else if let Some(err) = archive_open_error {
                resp.on_hover_ui(|ui| {
                    ui.set_max_width(ui.spacing().tooltip_width);
                    ui.label(tr!(
                        "This archive did not open:\n{err}\n\n\
                         No key the game ships reads its list of files. It may be \
                         damaged, encrypted with a key of its own, or renamed in a \
                         way no key OpenRage tries can undo.",
                        err = err
                    ));
                })
            } else if let Some(hint) = archive_recovered_hint {
                resp.on_hover_ui(|ui| {
                    ui.set_max_width(ui.spacing().tooltip_width);
                    ui.label(tr!(
                        "Opened with a key found by trying every one.\n{hint}\n\n\
                         An archive's key follows from its name and size, so a \
                         renamed archive needs another of the game's 101 keys; \
                         OpenRage tried each and kept the one that reads its files.",
                        hint = hint
                    ));
                })
            } else if outgrown {
                resp.on_hover_text(tr!(
                    "The game was updated since this copy was made, so it hides what the \
                     update changed. Right-click to refresh it from the patched archive."
                ))
            } else {
                resp
            };
            let (ctx, state_id) = (ui.ctx().clone(), row.id);
            let open = |_: &mut AppState| toggle_collapsing(&ctx, state_id, false);
            row_clicked(ui, state, doc_id, &node, &resp, sole, open, |_| {});
            if resp.drag_started() {
                start_drag(ui.ctx(), state, doc_id, node_id, ids::FILES);
            }
            maybe_drop_into_folder(ui.ctx(), state, doc_id, node_id, resp.rect);
            rpf_context_menu(&resp, state, doc_id, &node, asked, false);
        }
    } else {
        let supported = node.rpf9_type().is_some() || rpf_path_supported(&node.name);
        let color = if !supported {
            theme::TEXT_SECONDARY
        } else if selected {
            theme::ACCENT_TEXT
        } else {
            theme::TEXT_PRIMARY
        };
        ui.add_space(indent);
        let resp = if let Some(rename) = taken_rename.as_mut() {
            let r = inline_rename_text_edit(ui, rename);
            rename_action = classify_rename_response(ui, &r);
            r
        } else {
            let icon = icons::rpf_node_icon(&node, unsaved, in_flight, supported, false);
            let label = icons::icon_label(&icon, &node.name, color.into());
            let r = selectable_header(ui, selected, label);
            let detail = size.map(|sz| format!("{:.1} KiB", (sz as f32) / 1024.0));
            ui.add(detail_label(detail.unwrap_or_default()));
            r
        };
        note_row(state, doc_id, node_id, resp.rect);
        let resp = resp.interact(egui::Sense::click_and_drag());
        let resp = if let Some((index, hash)) = node.rpf9_entry() {
            resp.on_hover_text(format!(
                "{}\n#{index} · {hash:016X}",
                crate::state::files_view::type_in_words(&node)
            ))
        } else {
            resp
        };
        if !renaming {
            // ⚠ An entry open as its own document is a leaf here: its tree is the Outline's.
            let open = |state: &mut AppState| match bound_doc_id {
                Some(bound) => state.set_active_document(bound),
                None => {
                    *asked = Target::rows(state, doc_id, vec![node_id], false)
                        .and_then(|row| commands::on(state, Action::OpenSelection, row));
                }
            };
            let single = |state: &mut AppState| {
                if let Some(bound) = bound_doc_id {
                    state.activate_if_shown(bound);
                }
            };
            row_clicked(ui, state, doc_id, &node, &resp, sole, open, single);
            if resp.drag_started() {
                start_drag(ui.ctx(), state, doc_id, node_id, ids::FILES);
            }
            rpf_context_menu(&resp, state, doc_id, &node, asked, false);
        }
    }

    if let Some(rename) = taken_rename {
        finish_rename(state, doc_id, node_id, &node.name, rename, rename_action);
    }
}
