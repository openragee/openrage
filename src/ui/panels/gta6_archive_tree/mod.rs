//! The GTA VI archive explorer panel: the install's RPF9 archives and the ones opened from disk down
//! one side, and for the chosen one a tree of its table's entries — each entry's recovered name, hash,
//! kind and size, kept by a search over names, hex hashes and kinds. An entry whose body is itself an
//! RPF9 archive expands in place (the nested table is mounted on first expand); a leaf entry opens on
//! a double-click through the app's existing scan-body views, and a row's menu extracts it, copies
//! its name or hash, or replaces it in a new copy of the archive. The data is read through
//! [`crate::state::rpf9_scan::rpf9_table`]; this draws it.

mod actions;
mod state;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) use state::Gta6ArchiveExplorer;

use crate::platform::file_dialogs;
use crate::state::rpf9_scan::rpf9_table::{
    install_root, TableFilter, TableKind, TableNode, TableRow,
};
use crate::state::rpf9_scan::{body_path, View};
use crate::state::AppState;
use crate::ui::panel_registry::{Panel, PanelContext, PanelId, PanelRegistry};
use crate::ui::theme;

/// The explorer's panel id, under which the View menu offers it and a saved layout names it.
pub const ID: PanelId = "gta6_archive_tree";

struct Gta6ArchiveTree;

impl Panel for Gta6ArchiveTree {
    fn id(&self) -> PanelId {
        ID
    }

    fn title(&self, _state: &AppState) -> String {
        crate::i18n::tr("Archives").to_string()
    }

    fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
        draw(ui, ctx.state);
    }
}

static PANEL: Gta6ArchiveTree = Gta6ArchiveTree;

/// Register the explorer into `registry`, so the View menu offers it. Called once as the live panel
/// registry is built, rather than as a builtin, because GTA VI support is read-only and experimental
/// and the panel lists only once an install is set or an archive is opened — a dynamic panel is in no
/// default layout's count.
pub fn register_into(registry: &mut PanelRegistry) {
    registry.register(&PANEL);
}

/// A deferred action from the tree, applied after the read borrow of the explorer is dropped: toggle
/// a nested archive's expansion, open a clear entry's body in a view, or decrypt and open an
/// enciphered one.
enum Action {
    Toggle(Vec<u32>),
    Open(PathBuf),
    /// Decrypt the enciphered leaf `entry` within the node at `node` (a chain of archive entry
    /// indices) whose body is at absolute `body`, then open the plaintext.
    OpenCiphered {
        node: Vec<u32>,
        entry: u32,
        body: u64,
    },
    /// Ask where the row's body goes, then write it there.
    Extract(actions::Row),
    /// Ask for the file to put in the row's place and where the new archive goes, then write it.
    Replace(actions::Row),
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    // Always paint the heading, so the panel draws something even with nothing to list (the
    // self-check fails a panel that paints nothing).
    ui.label(egui::RichText::new(tr!("Archives")).strong());
    actions::poll(state);

    let configured = state.prefs.game_paths.get(or_title::Game::GtaVI).cloned();
    let root = install_root(configured.as_deref());
    let archives = state
        .panels
        .get::<Gta6ArchiveExplorer>()
        .archives(root.as_deref());

    if archives.is_empty() {
        ui.add_space(theme::SPACING_TIGHT);
        ui.label(egui::RichText::new(tr!("Choose the GTA VI folder in Settings, or open one of its archives, to list its archives from their tables.")).weak());
        return;
    }

    // The archives as a wrapped row of selectable chips, named from the install root; clicking one opens it.
    let mut to_open: Option<PathBuf> = None;
    let selected = state.panels.get::<Gta6ArchiveExplorer>().selected.clone();
    ui.horizontal_wrapped(|ui| {
        for path in &archives {
            let is_sel = selected.as_deref() == Some(path.as_path());
            if ui
                .selectable_label(is_sel, chip_name(root.as_deref(), path))
                .clicked()
            {
                to_open = Some(path.clone());
            }
        }
    });
    if let Some(path) = to_open {
        state.panels.get::<Gta6ArchiveExplorer>().open(&path);
    }
    ui.separator();

    // Draw the tree, collecting what to do once the explorer borrow is dropped.
    let mut actions: Vec<Action> = Vec::new();
    {
        let explorer = state.panels.get::<Gta6ArchiveExplorer>();
        if explorer.poll() {
            ui.spinner();
        }
        // Draw a failed-expand error above the tree without returning, so one bad nested mount does not
        // blank the whole tree; the next toggle clears it.
        if let Some(error) = &explorer.error {
            ui.colored_label(ui.visuals().error_fg_color, error.as_str());
        }
        let Some(disk) = explorer.selected.clone() else {
            return;
        };
        draw_filter(ui, &mut explorer.filter);
        let Some(root) = &explorer.root else {
            return;
        };
        ui.monospace(format!(
            "{} entries, index key {:#010X}",
            root.rows.len(),
            root.key_id
        ));
        ui.add_space(theme::SPACING_TIGHT);
        let tree = Tree {
            filter: &explorer.filter,
            kinds: &explorer.kinds,
            disk: &disk,
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draw_node(ui, root, &tree, &mut Vec::new(), &mut actions);
            });
    }

    for action in actions {
        match action {
            Action::Toggle(path) => state.panels.get::<Gta6ArchiveExplorer>().toggle(&path),
            Action::Open(path) => state.load_file(path),
            Action::OpenCiphered { node, entry, body } => {
                // Resolve the node's base and disk cheaply here, then leave a decrypt request for the
                // load worker: the decipher is slow and must not block the UI thread. Each `get` is
                // its own short borrow, dropped before `load_file`.
                let base = state.panels.get::<Gta6ArchiveExplorer>().node_base(&node);
                let disk = state.panels.get::<Gta6ArchiveExplorer>().selected.clone();
                match (base, disk) {
                    (Ok(base), Some(disk)) => {
                        let path = body_path(&disk, body, View::Bytes);
                        crate::state::rpf9_scan::remember_ciphered(&path, base, entry);
                        state.load_file(path);
                    }
                    (Err(e), _) => state.show_error_toast(e),
                    (Ok(_), None) => {}
                }
            }
            Action::Extract(row) => {
                if let Some(out) = file_dialogs::pick_export_path(&file_part(&row.name)) {
                    actions::spawn_extract(row, out);
                }
            }
            Action::Replace(row) => {
                let copy = format!("{}_edited.rpf", file_stem(&row.disk));
                let Some(with) = file_dialogs::ask(tr!("Replace with a file"), |d| d.pick_file())
                else {
                    continue;
                };
                let out = file_dialogs::ask(tr!("Save the new archive"), |d| {
                    d.set_file_name(copy).save_file()
                });
                if let Some(out) = out {
                    actions::spawn_replace(row, with, out);
                }
            }
        }
    }
}

/// The last component of an entry's name, the file name an extract offers.
fn file_part(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    last.chars()
        .map(|c| {
            if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map_or_else(
        || "archive".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    )
}

/// An archive's chip: its path under the install root, or its file name when it is elsewhere.
fn chip_name(root: Option<&Path>, path: &Path) -> String {
    match root.and_then(|root| path.strip_prefix(root).ok()) {
        Some(under) => under.display().to_string(),
        None => crate::state::jobs::file_name(path),
    }
}

/// The search field and a toggle per kind, which keep the tree's rows by name, hex hash and kind.
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
    ui.add_space(theme::SPACING_TIGHT);
}

/// What every row of a tree is drawn against: the filter, each mounted table's kind labels by its
/// entry path, and the archive file on disk, against which an entry's absolute offset locates its
/// body.
struct Tree<'a> {
    filter: &'a TableFilter,
    kinds: &'a BTreeMap<Vec<u32>, Vec<String>>,
    disk: &'a Path,
}

/// Draw `node`'s entries at the depth `path` gives, recursing into the nested archives it has open.
/// Under a filter, a row shows when the filter keeps it or an expanded archive under it holds one it
/// keeps. `path` is the chain of entry indices from the root to `node`.
fn draw_node(
    ui: &mut egui::Ui,
    node: &TableNode,
    tree: &Tree<'_>,
    path: &mut Vec<u32>,
    actions: &mut Vec<Action>,
) {
    let indent = path.len() as f32 * 14.0;
    let filter = tree.filter;
    let filtering = !filter.is_empty();
    let labels = tree.kinds.get(path.as_slice());
    for row in &node.rows {
        let child = (row.is_archive && node.open.contains(&row.index))
            .then(|| node.children.get(&row.index))
            .flatten();
        if filtering && !filter.keeps(row) && !child.is_some_and(|c| filter.keeps_under(c)) {
            continue;
        }
        let size = row
            .size
            .map(|s| format!("{s:>10}"))
            .unwrap_or_else(|| "         -".to_string());
        let name = row.display_name();
        // The row's own kind, once its table's labels are read; the coarse kind until then.
        let label = labels.and_then(|l| l.get(row.index as usize));
        let kind = format!("{:<22}", label.map_or(row.kind.label(), String::as_str));
        let cipher = if row.cipher { "  [cipher]" } else { "" };

        ui.horizontal(|ui| {
            ui.add_space(indent);
            let text = if row.is_archive {
                let arrow = if node.open.contains(&row.index) {
                    "▾"
                } else {
                    "▸"
                };
                format!("{arrow} {size}  {kind}  {name}")
            } else {
                format!("   {size}  {kind}  {name}{cipher}")
            };
            let resp = ui.selectable_label(false, egui::RichText::new(text).monospace());
            if row.is_archive && resp.clicked() {
                let mut child = path.clone();
                child.push(row.index);
                actions.push(Action::Toggle(child));
            }
            if resp.double_clicked() {
                actions.extend(open_action(row, path, tree.disk));
            }
            resp.context_menu(|ui| row_menu(ui, row, path, tree.disk, actions));
        });

        if let Some(child) = child {
            path.push(row.index);
            draw_node(ui, child, tree, path, actions);
            path.pop();
        }
    }
}

/// What opening `row` does: an archive opens as one, a clear body through the scan-body views, an
/// enciphered one by deciphering it first (the plaintext takes the same views).
fn open_action(row: &TableRow, node: &[u32], disk: &Path) -> Option<Action> {
    let off = row.absolute_start?;
    Some(if row.is_archive {
        Action::Open(body_path(disk, off, View::Archive))
    } else if row.cipher {
        Action::OpenCiphered {
            node: node.to_vec(),
            entry: row.index,
            body: off,
        }
    } else {
        Action::Open(body_path(disk, off, View::Bytes))
    })
}

/// A row's menu: open it, extract it, copy its name or hash, or replace it in a new archive.
fn row_menu(
    ui: &mut egui::Ui,
    row: &TableRow,
    node: &[u32],
    disk: &Path,
    actions: &mut Vec<Action>,
) {
    let target = || actions::Row {
        disk: disk.to_path_buf(),
        node: node.to_vec(),
        entry: row.index,
        name: row.display_name(),
    };
    if ui.button(tr!("Open")).clicked() {
        actions.extend(open_action(row, node, disk));
        ui.close();
    }
    if ui.button(tr!("Extract to a file…")).clicked() {
        actions.push(Action::Extract(target()));
        ui.close();
    }
    if ui.button(tr!("Copy name")).clicked() {
        ui.ctx().copy_text(row.display_name());
        ui.close();
    }
    if ui.button(tr!("Copy hash")).clicked() {
        ui.ctx().copy_text(format!("0x{:016x}", row.name64));
        ui.close();
    }
    ui.separator();
    let replace = ui.button(tr!("Replace with a file…")).on_hover_text(tr!(
        "Writes a new copy of the archive with this entry replaced, which the game refuses: only its maker can sign the table."
    ));
    if replace.clicked() {
        actions.push(Action::Replace(target()));
        ui.close();
    }
}
