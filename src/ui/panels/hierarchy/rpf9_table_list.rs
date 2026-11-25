//! The Files list of a GTA VI archive listed from its table: each entry's name, size and kind, a
//! nested archive expanded in place from its own table, and a leaf opened as the explorer opens it.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::state::rpf9_scan::rpf9_table::{TableFilter, TableKind, TableNode, TableRow};
use crate::state::{AppState, DocPayload, DocumentId};
use crate::ui::theme;

/// One line of the list: an entry, how deep it sits, the header offset of its archive and, for a
/// nested archive, the chain of entry indices that expands it.
struct Line<'a> {
    depth: usize,
    base: u64,
    row: &'a TableRow,
    /// The row's own kind once its table's labels are read.
    label: Option<&'a str>,
    toggle: Option<Vec<u32>>,
    open: bool,
}

enum Action {
    Toggle(Vec<u32>),
    Open {
        base: u64,
        row: TableRow,
        preview: bool,
    },
}

/// The archive's link, the filter and the entries it keeps, laying out only the lines in view.
pub(super) fn draw(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Rpf9Table(table)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let table = Arc::clone(table);
    let on_screen = state.scan_row_on_screen(doc_id);
    // The way back to the archive's own tab from a body of it.
    let parent = ui
        .link(crate::state::jobs::file_name(&table.path))
        .clicked();
    let mut actions = Vec::new();
    {
        let mut tree = table.tree();
        if tree.poll() {
            ui.spinner();
        }
        if let Some(error) = &tree.error {
            ui.colored_label(theme::WARNING, error.as_str());
        }
        draw_filter(ui, &mut tree.filter);
        let mut lines = Vec::new();
        if let Some(root) = &tree.root {
            flatten(
                root,
                &tree.filter,
                &tree.kinds,
                0,
                &mut Vec::new(),
                &mut lines,
            );
        }
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::both()
            .id_salt(("table entries", doc_id))
            .auto_shrink([false, false])
            .show_rows(ui, row_height, lines.len(), |ui, range| {
                for line in &lines[range] {
                    draw_line(ui, line, on_screen, &mut actions);
                }
            });
    }
    if parent {
        state.load_file(table.path.clone());
    }
    for action in actions {
        match action {
            Action::Toggle(path) => table.tree().toggle(&path),
            Action::Open { base, row, preview } => {
                state.open_table_entry(&table.path, base, &row, preview)
            }
        }
    }
}

/// The search field and a toggle per kind, which keep the entries by name, hex hash and kind.
fn draw_filter(ui: &mut egui::Ui, filter: &mut TableFilter) {
    let find = egui::TextEdit::singleline(&mut filter.text)
        .hint_text(tr!("Find by name, or by hash as 0x hex"))
        .desired_width(f32::INFINITY);
    ui.add(find);
    ui.horizontal_wrapped(|ui| {
        for kind in TableKind::ALL {
            let mut on = filter.kinds.contains(&kind);
            if ui.toggle_value(&mut on, kind.label()).changed() {
                if on {
                    filter.kinds.insert(kind);
                } else {
                    filter.kinds.remove(&kind);
                }
            }
        }
    });
    if !filter.is_empty() {
        let note = tr!("Only the archives expanded are searched.");
        ui.label(egui::RichText::new(note).color(theme::TEXT_SECONDARY));
    }
}

/// The lines of `node`, whose header is at `base`, that `filter` keeps, each expanded archive's
/// under its row; `path` is the chain of entry indices from the root to `node`.
fn flatten<'a>(
    node: &'a TableNode,
    filter: &TableFilter,
    kinds: &'a BTreeMap<Vec<u32>, Vec<String>>,
    base: u64,
    path: &mut Vec<u32>,
    out: &mut Vec<Line<'a>>,
) {
    let filtering = !filter.is_empty();
    let labels = kinds.get(path.as_slice());
    for row in &node.rows {
        let open = row.is_archive && node.open.contains(&row.index);
        let child = open.then(|| node.children.get(&row.index)).flatten();
        if filtering && !filter.keeps(row) && !child.is_some_and(|c| filter.keeps_under(c)) {
            continue;
        }
        path.push(row.index);
        out.push(Line {
            depth: path.len() - 1,
            base,
            row,
            label: labels
                .and_then(|l| l.get(row.index as usize))
                .map(String::as_str),
            toggle: row.is_archive.then(|| path.clone()),
            open,
        });
        if let (Some(child), Some(start)) = (child, row.absolute_start) {
            flatten(child, filter, kinds, start, path, out);
        }
        path.pop();
    }
}

/// One entry: a click expands an archive, and previews a leaf, which a double click keeps.
fn draw_line(ui: &mut egui::Ui, line: &Line, on_screen: Option<u64>, actions: &mut Vec<Action>) {
    let row = line.row;
    let mark = match (&line.toggle, line.open) {
        (Some(_), true) => "▾",
        (Some(_), false) => "▸",
        (None, _) => " ",
    };
    let size = row.size.map_or_else(|| "-".to_string(), |s| s.to_string());
    let cipher = if row.cipher { "  [cipher]" } else { "" };
    let text = format!(
        "{mark} {size:>10}  {:<10}  {}{cipher}",
        line.label.unwrap_or(row.kind.label()),
        row.display_name()
    );
    ui.horizontal(|ui| {
        ui.add_space(line.depth as f32 * 14.0);
        let selected = row.absolute_start.is_some() && row.absolute_start == on_screen;
        let response = ui.selectable_label(selected, egui::RichText::new(text).monospace());
        match &line.toggle {
            Some(path) => {
                let hover = tr!("Click to expand or collapse this nested archive, listed from its own table.");
                if response.on_hover_text(hover).clicked() {
                    actions.push(Action::Toggle(path.clone()));
                }
            }
            None => {
                let hover = tr!("Click to open it in the preview tab, and double-click to keep it in a tab of its own; an enciphered body is deciphered first.");
                let response = response.on_hover_text(hover);
                if response.clicked() || response.double_clicked() {
                    actions.push(Action::Open {
                        base: line.base,
                        row: row.clone(),
                        preview: !response.double_clicked(),
                    });
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::rpf9_scan::rpf9_table_document::tests::{sample_document, state_showing};

    #[test]
    fn an_expanded_archive_lists_its_entries_under_its_row_and_the_filter_keeps_them() {
        let (_dir, doc) = sample_document();
        let mut root = doc.tree().root.clone().expect("the root is mounted");
        let nested = root.rows[2].index;
        let start = root.rows[2]
            .absolute_start
            .expect("the nested archive's start");
        let child = TableNode {
            rows: vec![root.rows[0].clone()],
            ..TableNode::default()
        };
        root.children.insert(nested, child);

        let none = BTreeMap::new();
        let lines = |root: &TableNode, filter: &TableFilter| {
            let mut out = Vec::new();
            flatten(root, filter, &none, 0, &mut Vec::new(), &mut out);
            (out.iter())
                .map(|l| (l.depth, l.base, l.toggle.clone()))
                .collect::<Vec<_>>()
        };
        let all = TableFilter::default();
        assert_eq!(
            lines(&root, &all).len(),
            3,
            "a collapsed archive lists nothing under it"
        );
        root.open.insert(nested);
        let shown = lines(&root, &all);
        assert_eq!(shown.len(), 4);
        assert_eq!(shown[2], (0, 0, Some(vec![nested])));
        assert_eq!(
            shown[3],
            (1, start, None),
            "a nested entry stands at its archive's header"
        );

        let alpha = TableFilter {
            text: "alpha".into(),
            ..TableFilter::default()
        };
        assert_eq!(
            lines(&root, &alpha).len(),
            3,
            "the root's match, the archive that holds one and its match"
        );

        // A row takes its table's own label, by the chain of the table it is in.
        let kinds = BTreeMap::from([(vec![nested], vec!["drawable".to_string(); 8])]);
        let mut out = Vec::new();
        flatten(&root, &all, &kinds, 0, &mut Vec::new(), &mut out);
        let labels: Vec<_> = out.iter().map(|l| l.label).collect();
        assert_eq!(
            labels[..3],
            [None, None, None],
            "the root's labels are not read"
        );
        assert_eq!(labels[3], Some("drawable"));
    }

    #[test]
    fn the_files_list_draws_the_table_s_names() {
        let _serial = crate::ui::theme::serial();
        let (_dir, doc) = sample_document();
        let (mut state, id) = state_showing(doc);
        let output = egui::Context::default().run_ui(Default::default(), |ui| {
            draw(ui, &mut state, id);
        });
        let shapes = format!("{:?}", output.shapes);
        assert!(shapes.contains("common/data/alpha.meta"), "{shapes}");
        assert!(shapes.contains("levels/inner.rpf"), "{shapes}");
        assert!(shapes.contains("hash_"), "{shapes}");
    }
}
