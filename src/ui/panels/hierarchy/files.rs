//! The Files panel: every install, folder and archive open, each as a tree,
//! with the filter, history and places that move through them.

use super::navigation::{
    breadcrumbs, go_to_file_button, history_input, places_menu, preview_pane_button, sort_menu,
};
use super::*;
use crate::ui::panels::registry::ids;

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    super::rows::start_row_rects(state, ui.ctx().cumulative_pass_nr());
    super::rows::focus_on_drop(ui, state, ids::FILES);
    history_input(ui, state);

    // A GTA VI archive a key opens lists its table: its tab, and the tab of a body opened from it.
    if let Some(table) = state.table_listing_the_active_tab() {
        rpf9_table_list::draw(ui, state, table);
        return;
    }
    // One no key opens has no tree: its tab, and the tab of a body opened from it, list what its
    // scan found.
    if let Some(scan) = state.scan_listing_the_active_tab() {
        crate::ui::panels::inspector::draw_scan_list(ui, state, scan);
        return;
    }

    let pulse = if ui_state(state).pulse_expand_all {
        Some(true)
    } else if ui_state(state).pulse_collapse_all {
        Some(false)
    } else {
        None
    };
    write_pulse_to_ctx(ui.ctx(), pulse);
    ui_state(state).pulse_expand_all = false;
    ui_state(state).pulse_collapse_all = false;

    promote_pending_rename(ui.ctx(), state);

    ui.vertical(|ui| {
        toolbar(ui, state);
        filter_row(ui, state);
        // The same words find a file of anything open by its name or its hash, where the tree only narrows what is expanded.
        let (words, deep) = (ui_state(state).filter.clone(), ui_state(state).deep);
        if !deep {
            let (finding, keys) = {
                let files = ui_state(state);
                (files.open_matches, files.filter_keys)
            };
            let found = crate::ui::panels::command_palette::draw_matches_of(
                ui, state, &words, finding, keys,
            );
            let files = ui_state(state);
            // ⚠ The words go with the finding: left in the field they would filter the tree the file was just shown in.
            if found.ended {
                files.open_matches = false;
                files.filter.clear();
            }
            if found.refocus {
                files.focus_filter_next_frame = true;
            }
        }
        breadcrumbs(ui, state);
        ui.separator();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| draw_trees(ui, state));
    });

    write_pulse_to_ctx(ui.ctx(), None);

    if state.navigation.reveal.is_some() {
        ui.ctx().request_repaint();
        let frames = &mut ui_state(state).reveal_frames;
        *frames += 1;
        if *frames > REVEAL_PATIENCE {
            *frames = 0;
            state.navigation.reveal = None;
        }
    } else {
        ui_state(state).reveal_frames = 0;
    }
    if state.navigation.pending_place().is_some() {
        ui.ctx().request_repaint();
    }
}

fn toolbar(ui: &mut egui::Ui, state: &mut AppState) {
    // ⚠ Wrapped: one row overflows the left column's default width, and the
    // overflow widened everything below it past the panel's edge.
    ui.horizontal_wrapped(|ui| {
        // Every button of the row is one height: a small one beside a menu, which is never small, reads as a different control.
        let expand_label =
            egui::RichText::new(tr!("{icon} Expand", icon = ph::ARROWS_OUT_LINE_VERTICAL));
        if ui
            .button(expand_label)
            .on_hover_text(tr!("Expand every collapsed row in Files"))
            .clicked()
        {
            ui_state(state).pulse_expand_all = true;
            ui_state(state).pulse_collapse_all = false;
        }
        let collapse_label =
            egui::RichText::new(tr!("{icon} Collapse", icon = ph::ARROWS_IN_LINE_VERTICAL));
        if ui
            .button(collapse_label)
            .on_hover_text(tr!("Collapse every expanded row in Files"))
            .clicked()
        {
            ui_state(state).pulse_collapse_all = true;
            ui_state(state).pulse_expand_all = false;
        }
        preview_pane_button(ui, state);
        sort_menu(ui, state);
        places_menu(ui, state);
        go_to_file_button(ui, state);
    });
}

fn filter_row(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        let deep = ui_state(state).deep;
        let hint = if deep {
            tr!("Glob (**/*.ydr)…")
        } else {
            tr!("Filter (substring)…")
        };
        // ⚠ Room is kept for the two buttons after the field: at the panel's whole width the field pushed them past its edge, where they were neither seen nor clicked.
        let button = egui::TextStyle::Button.resolve(ui.style()).size
            + 2.0 * ui.spacing().button_padding.x
            + ui.spacing().item_spacing.x;
        let edit = egui::TextEdit::singleline(&mut ui_state(state).filter)
            .id_salt("hierarchy_filter_input")
            .hint_text(hint)
            .desired_width((ui.available_width() - 2.0 * button).max(40.0));
        let resp = ui.add(edit);
        // The list under the field takes the arrows and Enter only while they are the field's.
        ui_state(state).filter_keys = resp.has_focus() || resp.lost_focus();
        if resp.changed() {
            crate::ui::panels::command_palette::first_row(state);
        }
        if ui_state(state).focus_filter_next_frame {
            // ⚠ Not in the frame of the click that asked for it: the keyboard given there is taken back as that click ends, and what is typed next goes nowhere.
            if ui.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
                ui.ctx().request_repaint();
            } else {
                resp.request_focus();
                ui_state(state).focus_filter_next_frame = false;
            }
        }
        if !ui_state(state).filter.is_empty()
            && crate::ui::accessible_name::named(ui.small_button(ph::X), tr!("Clear filter"))
                .on_hover_text(tr!("Clear filter"))
                .clicked()
        {
            ui_state(state).filter.clear();
            state.deep_find.clear();
        }
        let mut toggled = deep;
        let search = crate::ui::accessible_name::named(
            ui.toggle_value(&mut toggled, ph::MAGNIFYING_GLASS),
            tr!("Search inside archives"),
        );
        if search
            .on_hover_text(tr!("Search inside archives.\n\n\
                 Reads the box as a path glob and resolves it through the \
                 virtual filesystem, so it reaches files in nested archives \
                 that have never been opened. `**/*.ydr` finds every drawable \
                 at any depth; `*` stops at a separator."))
            .changed()
        {
            ui_state(state).deep = toggled;
            state.deep_find.clear();
        }
    });
}

/// Each install, folder and archive open; a file of any other kind is the Outline's.
fn draw_trees(ui: &mut egui::Ui, state: &mut AppState) {
    let mut outcome = RowOutcome::default();
    let doc_ids: Vec<DocumentId> = state
        .documents
        .iter()
        .filter(|d| d.rpf_entry_binding.is_none() && matches!(d.payload, DocPayload::Rpf(_)))
        .map(|d| d.id)
        .collect();
    if doc_ids.is_empty() {
        let scope = ui.scope(|ui| {
            empty_centered(
                ui,
                tr!("No install, folder or archive open\n\nRight-click here to make a new file"),
            );
        });
        let bg = ui.interact(
            scope.response.rect,
            ui.id().with("hierarchy_empty_ctx"),
            egui::Sense::click(),
        );
        bg.context_menu(|ui| {
            new_document_menu(ui, &mut outcome.create);
        });
        outcome.apply(state, ui.ctx());
        return;
    }

    for doc_id in doc_ids {
        document_row(ui, state, doc_id, &mut outcome);
    }

    let trailing = crate::ui::accessible_name::named(
        ui.allocate_response(
            egui::Vec2::new(ui.available_width(), 64.0),
            egui::Sense::click(),
        ),
        tr!("Empty space in Files"),
    );
    trailing.context_menu(|ui| {
        new_document_menu(ui, &mut outcome.create);
    });
    outcome.apply(state, ui.ctx());
}

#[path = "rpf9_table_list.rs"]
mod rpf9_table_list;
