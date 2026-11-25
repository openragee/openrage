//! What a row's menu adds to the verbs every row has: a new file of a kind,
//! a new folder that asks for its name in place, which favourites command a
//! row offers, and the Properties window.

use super::*;
use crate::state::files_view::{row_facts, RowFacts};
use crate::ui::commands::{self, Action, Command, Target};

/// Add to or Remove from favourites, whichever `at` takes.
pub(super) fn favourite_action(state: &AppState, at: Location) -> Action {
    match state.place_of(at) {
        Some(place) if state.prefs.explorer.is_favourite(&place) => Action::RemoveFromFavourites,
        _ => Action::AddToFavourites,
    }
}

/// `New folder`, or `New folder (2)` and on, whichever neither the folder nor the disk holds.
fn free_folder_name(state: &AppState, doc_id: DocumentId, parent: RpfNodeId) -> String {
    let Some(DocPayload::Rpf(rpf)) = state.document(doc_id).map(|d| &d.payload) else {
        return tr!("New folder").to_string();
    };
    let on_disk = rpf.session.node(parent).and_then(|n| match &n.backing {
        RpfNodeBacking::DiskFolder { fs_path, .. } => Some(fs_path.clone()),
        _ => None,
    });
    let taken = |name: &str| {
        rpf.session.children_of(parent).iter().any(|&id| {
            rpf.session
                .node(id)
                .is_some_and(|n| n.name.eq_ignore_ascii_case(name))
        }) || on_disk.as_ref().is_some_and(|dir| dir.join(name).exists())
    };
    std::iter::once(tr!("New folder").to_string())
        .chain((2..).map(|n| tr!("New folder ({n})", n = n)))
        .find(|name| !taken(name))
        .unwrap_or_else(|| "New folder".to_string())
}

/// New folder, New file and Copy into archive… into the folder `target` adds to.
pub(super) fn add_items(
    ui: &mut egui::Ui,
    state: &mut AppState,
    target: &Target,
    asked: &mut Option<Command>,
) {
    let &Target::Rows {
        doc: doc_id,
        into: parent,
        ..
    } = target
    else {
        return;
    };
    commands::offer(ui, state, Action::NewFolder, target, asked);
    let mut new_file_kind: Option<DocumentKind> = None;
    crate::ui::commands::submenu(ui, tr!("New file"), |ui| {
        super::new_document_items(ui, &mut new_file_kind);
    });
    if let Some(kind) = new_file_kind {
        match state.rpf_add_empty_file(doc_id, parent, kind) {
            Ok(added) => {
                let name = match state.document(doc_id).map(|d| &d.payload) {
                    Some(DocPayload::Rpf(rpf)) => rpf.session.node(added).map(|n| n.name.clone()),
                    _ => None,
                };
                state.show_toast(tr!("Added {name}", name = name.unwrap_or_default()));
            }
            Err(e) => state.show_error_toast(e),
        }
    }
    commands::offer(ui, state, Action::ImportFile, target, asked);
}

/// What a right click on the Files view's empty space offers: adds into the folder shown, and its properties.
pub(super) fn background_menu(
    resp: &egui::Response,
    state: &mut AppState,
    doc_id: DocumentId,
    folder: RpfNodeId,
    asked: &mut Option<Command>,
) {
    resp.context_menu(|ui| {
        crate::ui::popup_of_a_document(ui.ctx(), doc_id);
        let Some(target) = Target::rows(state, doc_id, vec![folder], true) else {
            return;
        };
        add_items(ui, state, &target, asked);
        commands::offer(ui, state, Action::Paste, &target, asked);
        ui.separator();
        commands::offer(ui, state, Action::Properties, &target, asked);
    });
}

/// Make a folder under `parent` and put its name up for editing where the row shows.
pub(crate) fn new_folder(
    state: &mut AppState,
    doc_id: DocumentId,
    parent: RpfNodeId,
    centre: bool,
) {
    let name = free_folder_name(state, doc_id, parent);
    match state.rpf_add_folder(doc_id, parent, &name) {
        Ok(id) => {
            state.selection.select(doc_id, id);
            let ui = ui_state(state);
            if centre {
                ui.start_rename_in_centre(doc_id, id, &name);
            } else {
                ui.start_rename_rpf(doc_id, id, &name);
                state.navigation.reveal = Some(Location {
                    doc: doc_id,
                    node: id,
                });
            }
        }
        Err(e) => state.show_error_toast(e),
    }
}

/// The properties of the row picked from a menu, at the top of Details: its columns and its path.
pub(crate) fn draw_properties(ui: &mut egui::Ui, state: &mut AppState) {
    let Some((doc_id, node_id)) = state
        .panels
        .peek::<FilesViewUi>()
        .and_then(|v| v.properties)
    else {
        return;
    };
    let shown = state.document(doc_id).and_then(|d| match &d.payload {
        DocPayload::Rpf(rpf) => {
            let node = rpf.session.node(node_id)?;
            let facts = row_facts(&rpf.session, node_id)?;
            let inner = rpf.session.path_of(node_id).unwrap_or_default();
            let path = match (&d.path, node.backing.fs_path()) {
                (_, Some(disk)) => disk.display().to_string(),
                (Some(root), None) => format!("{}\\{}", root.display(), inner.replace('/', "\\")),
                (None, None) => format!("/{inner}"),
            };
            let hash = crate::state::rpf9_scan::rpf9_tree_actions::copied_hash(&rpf.session, node);
            Some((node.name.clone(), facts, path, hash))
        }
        _ => None,
    });
    let Some((name, facts, path, hash)) = shown else {
        state.panels.get::<FilesViewUi>().properties = None;
        return;
    };
    let mut open = true;
    ui.horizontal(|ui| {
        ui.strong(tr!("{name} Properties", name = name));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui.small_button(egui_phosphor::regular::X);
            if close.on_hover_text(tr!("Close")).clicked() {
                open = false;
            }
        });
    });
    egui::Grid::new("files_view_properties_grid")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for (label, value) in property_rows(&name, &facts, &path, hash.clone()) {
                ui.label(egui::RichText::new(label).color(theme::TEXT_SECONDARY));
                // ⚠ Wrapped: a path is wider than the column Details stands in.
                ui.add(egui::Label::new(value).selectable(true).wrap());
                ui.end_row();
            }
        });
    ui.separator();
    if !open {
        state.panels.get::<FilesViewUi>().properties = None;
    }
}

/// What the Properties window lists, in order; `hash` is the one Copy hash gives.
pub(super) fn property_rows(
    name: &str,
    facts: &RowFacts,
    path: &str,
    hash: String,
) -> Vec<(&'static str, String)> {
    let bytes = |n: Option<u64>| match n {
        Some(n) => tr!("{size} ({n} bytes)", n = n, size = or_job::human_bytes(n)),
        None => "—".to_string(),
    };
    let text = |t: Option<&str>| t.unwrap_or("—").to_string();
    vec![
        (tr!("Name"), name.to_string()),
        (tr!("Type"), facts.kind.clone()),
        (tr!("Size"), bytes(facts.size)),
        (tr!("Stored size"), bytes(facts.stored)),
        (tr!("Compression"), text(facts.compression)),
        (tr!("Encryption"), text(facts.encryption)),
        (tr!("Edition"), text(facts.edition)),
        (
            tr!("Version"),
            facts.version.map_or("—".to_string(), |v| v.to_string()),
        ),
        (tr!("Hash"), hash),
        (tr!("Path"), path.to_string()),
    ]
}
