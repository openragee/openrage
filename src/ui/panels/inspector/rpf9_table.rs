//! The table-backed RPF9 tree and the selected entry's facts.

use crate::state::rpf9::{is_archive, Rpf9TableDocument, Tree};
use crate::ui::theme;

pub(super) fn draw_facts(ui: &mut egui::Ui, doc: &Rpf9TableDocument) {
    let mut tree = doc.tree();
    tree.poll();
    ui.heading(tr!("GTA VI Archive"));
    ui.label(tr!(
        "Entries are listed from the archive table. Expand an archive to load its table."
    ));
    ui.label(tr!(
        "File contents cannot be opened from this table view yet."
    ));
    let Some(root) = tree.nodes.get(&0) else {
        return;
    };
    ui.label(tr_n!(
        "{count} entry",
        "{count} entries",
        root.archive.entries.len()
    ));
    if let Some((base, index)) = tree.selected {
        if let Some(entry) = tree
            .nodes
            .get(&base)
            .and_then(|node| node.archive.entries.get(index as usize))
        {
            ui.separator();
            ui.strong(entry.display_name());
            ui.monospace(format!("#{index} · {:016x}", entry.name64));
            if let Some(start) = entry.start.and_then(|start| base.checked_add(start)) {
                ui.label(tr!("Offset: {offset}", offset = format!("{start:#X}")));
            }
            if let Some(size) = entry.stored {
                ui.label(tr!("Stored size: {size}", size = or_job::human_bytes(size)));
            }
            if entry.cipher.is_some() {
                ui.label(tr!("Encrypted"));
            }
        }
        if let Some(error) = tree.errors.get(&(base, index)) {
            ui.colored_label(theme::WARNING, error);
        }
    }
}

fn visible_rows(
    tree: &Tree,
    base: u64,
    depth: usize,
    filter: &str,
    rows: &mut Vec<(u64, u32, usize)>,
) {
    let Some(node) = tree.nodes.get(&base) else {
        return;
    };
    for entry in &node.archive.entries {
        let key = (base, entry.index);
        let archive = is_archive(entry) || tree.expanded.contains(&key);
        if archive || filter.is_empty() || entry.display_name().to_lowercase().contains(filter) {
            rows.push((base, entry.index, depth));
        }
        if tree.expanded.contains(&key) {
            if let Some(child) = entry.start.and_then(|start| base.checked_add(start)) {
                visible_rows(tree, child, depth + 1, filter, rows);
            }
        }
    }
}

pub(crate) fn draw_tree(ui: &mut egui::Ui, doc: &Rpf9TableDocument) {
    let mut tree = doc.tree();
    tree.poll();
    ui.strong(crate::state::jobs::file_name(&doc.path));
    ui.add(egui::TextEdit::singleline(&mut tree.filter).hint_text(tr!("Find")));
    let mut rows = Vec::new();
    visible_rows(&tree, 0, 0, &tree.filter.to_lowercase(), &mut rows);
    let pending = tree.pending();
    if pending.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(tr!("Loading archive table…"));
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    let mut toggle = None;
    let mut selected = tree.selected;
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    egui::ScrollArea::both()
        .id_salt(("rpf9 table", &doc.path))
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows.len(), |ui, visible| {
            for row in visible {
                let (base, index, depth) = rows[row];
                let key = (base, index);
                let entry = &tree.nodes[&base].archive.entries[index as usize];
                let open = tree.expanded.contains(&key);
                ui.push_id(key, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * ui.spacing().indent);
                        if is_archive(entry) || open {
                            let enabled = pending.is_none() || open;
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(if open { "▾" } else { "▸" }).frame(false),
                                )
                                .clicked()
                            {
                                selected = Some(key);
                                toggle = Some(key);
                            }
                        } else {
                            ui.add_space(ui.spacing().indent);
                        }
                        let response =
                            ui.selectable_label(selected == Some(key), entry.display_name());
                        if response.clicked() {
                            selected = Some(key);
                        }
                        if response.double_clicked()
                            && is_archive(entry)
                            && (pending.is_none() || open)
                        {
                            toggle = Some(key);
                        }
                        // A row without a class can be checked at its exact start on request.
                        if entry.class.is_none() && entry.start.is_some() {
                            response.context_menu(|ui| {
                                if ui
                                    .add_enabled(
                                        pending.is_none(),
                                        egui::Button::new(tr!("Open as archive")),
                                    )
                                    .clicked()
                                {
                                    selected = Some(key);
                                    toggle = Some(key);
                                    ui.close();
                                }
                            });
                        }
                        if pending == Some(key) {
                            ui.spinner();
                        }
                        if entry.cipher.is_some() {
                            ui.weak(tr!("Encrypted"));
                        }
                        if let Some(error) = tree.errors.get(&key) {
                            ui.colored_label(theme::WARNING, "!").on_hover_text(error);
                        }
                    })
                });
            }
        });
    tree.selected = selected;
    drop(tree);
    if let Some((base, index)) = toggle {
        doc.toggle(base, index);
        ui.ctx().request_repaint();
    }
}
