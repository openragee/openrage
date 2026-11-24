//! The GTA VI archive explorer panel: the build's RPF9 archives down one side, and for the chosen
//! one a tree of its deciphered entries — each entry's recovered name, hash and size. An entry whose
//! body is itself an RPF9 archive expands in place (the nested table is mounted on first expand); a
//! leaf entry opens on a double-click through the app's existing scan-body views. The data is read
//! through [`Gta6ArchiveExplorer`]; this only draws it. Its state is a child of this panel module, so
//! declaring it costs `state/mod.rs` no line.

mod state;

use std::path::{Path, PathBuf};

use state::{Gta6ArchiveExplorer, Node};

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
/// and the panel draws only when a build is set — a dynamic panel is in no default layout's count.
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
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    // Always paint the heading, so the panel draws something even with no build set (the self-check
    // fails a panel that paints nothing).
    ui.label(egui::RichText::new(crate::i18n::tr("Archives")).strong());

    let archives: Vec<String> = state
        .panels
        .get::<Gta6ArchiveExplorer>()
        .ensure_listed()
        .to_vec();

    if archives.is_empty() {
        // No build is set (OPENRAGE_GTA6_BUILD), so there is nothing to list; a weak hint names the
        // variable to set. ⚠ Untranslated: a developer-only diagnostic for an experimental, read-only
        // panel. It also keeps the panel painting content of its own when empty, which the self-check
        // requires.
        ui.add_space(theme::SPACING_TIGHT);
        ui.label(
            egui::RichText::new(format!(
                "Set {} to a GTA VI build folder to list its RPF9 archives.",
                state::BUILD_ENV
            ))
            .weak(),
        );
        return;
    }

    // The archives as a wrapped row of selectable chips; clicking one opens it.
    let mut to_open: Option<String> = None;
    let selected = state.panels.get::<Gta6ArchiveExplorer>().selected.clone();
    ui.horizontal_wrapped(|ui| {
        for name in &archives {
            let is_sel = selected.as_deref() == Some(name.as_str());
            if ui.selectable_label(is_sel, name.as_str()).clicked() {
                to_open = Some(name.clone());
            }
        }
    });
    if let Some(name) = to_open {
        state.panels.get::<Gta6ArchiveExplorer>().open(&name);
    }
    ui.separator();

    // Draw the tree, collecting what to do once the explorer borrow is dropped.
    let disk = state.panels.get::<Gta6ArchiveExplorer>().selected_path();
    let mut actions: Vec<Action> = Vec::new();
    {
        let explorer = state.panels.get::<Gta6ArchiveExplorer>();
        // Draw a failed-expand error above the tree without returning, so one bad nested mount does not
        // blank the whole tree; the next toggle clears it.
        if let Some(error) = &explorer.error {
            ui.colored_label(ui.visuals().error_fg_color, error.as_str());
        }
        let (Some(root), Some(disk)) = (&explorer.root, disk.as_deref()) else {
            return;
        };
        ui.monospace(format!(
            "{} entries, index key {:#010X}",
            root.rows.len(),
            root.key_id
        ));
        ui.add_space(theme::SPACING_TIGHT);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draw_node(ui, root, &mut Vec::new(), disk, &mut actions);
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
                let disk = state.panels.get::<Gta6ArchiveExplorer>().selected_path();
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
        }
    }
}

/// Draw `node`'s entries at the depth `path` gives, recursing into the nested archives it has open.
/// `path` is the chain of entry indices from the root to `node`; `disk` is the archive file on disk,
/// against which an entry's absolute offset locates its body.
fn draw_node(
    ui: &mut egui::Ui,
    node: &Node,
    path: &mut Vec<u32>,
    disk: &Path,
    actions: &mut Vec<Action>,
) {
    let indent = path.len() as f32 * 14.0;
    for row in &node.rows {
        let size = row
            .size
            .map(|s| format!("{s:>10}"))
            .unwrap_or_else(|| "         -".to_string());
        let name = row.display_name();
        let cipher = if row.cipher { "  [cipher]" } else { "" };

        ui.horizontal(|ui| {
            ui.add_space(indent);
            if row.is_archive {
                let open = node.open.contains(&row.index);
                let arrow = if open { "▾" } else { "▸" };
                let label = egui::RichText::new(format!("{arrow} {size}  {name}")).monospace();
                let resp = ui.selectable_label(false, label);
                if resp.clicked() {
                    let mut child = path.clone();
                    child.push(row.index);
                    actions.push(Action::Toggle(child));
                }
                if resp.double_clicked() {
                    if let Some(off) = row.absolute_start {
                        actions.push(Action::Open(body_path(disk, off, View::Archive)));
                    }
                }
            } else {
                let label = egui::RichText::new(format!("   {size}  {name}{cipher}")).monospace();
                let resp = ui.selectable_label(false, label);
                // A double-click opens the body: a clear one through the scan-body views, an
                // enciphered one by decrypting it first (the plaintext takes the same views).
                if resp.double_clicked() {
                    if let Some(off) = row.absolute_start {
                        if row.cipher {
                            actions.push(Action::OpenCiphered {
                                node: path.clone(),
                                entry: row.index,
                                body: off,
                            });
                        } else {
                            actions.push(Action::Open(body_path(disk, off, View::Bytes)));
                        }
                    }
                }
            }
        });

        if row.is_archive && node.open.contains(&row.index) {
            if let Some(child) = node.children.get(&row.index) {
                path.push(row.index);
                draw_node(ui, child, path, disk, actions);
                path.pop();
            }
        }
    }
}
