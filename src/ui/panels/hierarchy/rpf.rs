use super::rows::{begin_rows, finish_rows, select_unless_selected};
use super::rpf_tree::{draw_row, list_rows};
use super::*;
use crate::state::gltf::Model;
use crate::ui::commands::{self, Action, Command, Hits, Target};
use crate::ui::panels::search;

pub(super) fn draw_rpf(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    super::defrag::draw_defrag(ui.ctx(), state);
    let pattern = super::ui_state(state).filter.clone();
    if super::ui_state(state).deep && !pattern.trim().is_empty() {
        draw_deep_find(ui, state, doc_id, pattern.trim());
        return;
    }

    let (file_count, folder_count) = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => (
            rpf.session.file_count(),
            rpf.session.directory_count().saturating_sub(1),
        ),
        _ => return,
    };

    if file_count == 0 && folder_count == 0 {
        empty_centered(ui, tr!("This archive is empty"));
        return;
    }

    let filter = super::ui_state(state).filter.clone();
    if filter.is_empty() {
        let label = egui::RichText::new(tr!(
            "{file_count} files, {folder_count} folders",
            file_count = file_count,
            folder_count = folder_count
        ))
        .small()
        .color(theme::TEXT_SECONDARY);
        ui.add(egui::Label::new(label).selectable(false));
    } else {
        let (shown, total) = filter_readout(state, doc_id, &filter, file_count + folder_count);
        let label = egui::RichText::new(tr!(
            "{shown} of {total} loaded files match",
            shown = shown,
            total = total
        ))
        .small()
        .color(theme::TEXT_SECONDARY);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(label).selectable(false))
                .on_hover_text(tr!(
                    "Counts the files loaded so far. A nested archive's files count \
                     once it has been expanded; the search button reaches the ones \
                     that have not."
                ));
            super::escalate_button(ui, state, &filter);
        });
    }
    let selected_count = state.selection.selected_ids(doc_id).len();
    if selected_count > 1 {
        ui.add(
            egui::Label::new(
                egui::RichText::new(tr!(
                    "{selected_count} selected — right-click one of them to extract them all",
                    selected_count = selected_count
                ))
                .small()
                .color(theme::ACCENT_TEXT),
            )
            .selectable(false),
        );
    }
    ui.add_space(crate::ui::theme::SPACING_HAIR);

    begin_rows(state, doc_id);
    let id_root = ui.make_persistent_id("hierarchy_rpf");
    let mut asked: Option<Command> = None;

    let rows = list_rows(ui.ctx(), state, doc_id, id_root);
    let reveal = (state.navigation.reveal)
        .filter(|at| at.doc == doc_id)
        .and_then(|at| (rows.iter()).position(|row| !row.pending && row.node == at.node));
    if reveal.is_some() {
        state.navigation.reveal = None;
    }
    let salt = ("rpf_tree", doc_id);
    crate::ui::virtual_rows::show(ui, salt, rows.len(), reveal, |ui, index| {
        draw_row(ui, state, doc_id, &rows[index], &mut asked);
    });
    finish_rows(state, doc_id);

    if let Some(command) = asked {
        commands::run(state, ui.ctx(), &command);
    }
}

fn filter_readout(
    state: &mut AppState,
    doc_id: DocumentId,
    filter: &str,
    nodes: usize,
) -> (usize, usize) {
    if let Some(c) = &super::ui_state(state).filter_counts {
        if c.doc_id == doc_id && c.filter == filter && c.nodes == nodes {
            return (c.shown, c.total);
        }
    }
    let (shown, total) = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => explorer::filter_counts(&rpf.session, filter),
        _ => (0, 0),
    };
    super::ui_state(state).filter_counts = Some(super::FilterCounts {
        doc_id,
        filter: filter.to_string(),
        nodes,
        shown,
        total,
    });
    (shown, total)
}

/// How many matches the search of every archive draws; a whole install matches far more.
const SHOWN: usize = 1_000;

/// The filter as a glob over the file index, so it reaches archives the tree
/// has never opened.
///
/// ⚠ **This is a list of paths and not a slice of the tree**, because the
/// matches need not have nodes: a `.ydr` two nested archives down exists in
/// the index and not in `RpfEditSession` until somebody expands its way down.
fn draw_deep_find(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId, pattern: &str) {
    state.deep_find_tick(doc_id, pattern);
    let Some(found) = state.deep_find.found_for(doc_id, pattern) else {
        return;
    };
    let total = found.matches().len();
    let matches: Vec<String> = found.matches().iter().take(SHOWN).cloned().collect();
    let (indexing, read, complete) = (found.indexing, found.read(), found.is_complete());
    let unread = found.warnings.len();
    let some_unread: Vec<String> = found.warnings.iter().take(20).cloned().collect();

    let summary = format!("{total} match{}", if total == 1 { "" } else { "es" });
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(summary)
                    .small()
                    .color(theme::TEXT_SECONDARY),
            )
            .selectable(false),
        );
        if indexing {
            ui.spinner();
            let reading = tr!(
                "still reading the archives, {read} files so far",
                read = read
            );
            ui.label(
                egui::RichText::new(reading)
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    });

    if !indexing && !complete {
        let why = tr!(
            "Not exhaustive — {unread} archive(s) or folder(s) could not be read",
            unread = unread
        );
        ui.add(
            egui::Label::new(egui::RichText::new(why).small().color(theme::WARNING))
                .selectable(false),
        )
        .on_hover_text(some_unread.join("\n"));
    }

    if matches.is_empty() {
        if !indexing {
            empty_centered(ui, tr!("No path matches this glob"));
        }
        return;
    }
    if total > matches.len() {
        let more = tr!(
            "The first {shown} are shown; narrow the glob for the rest",
            shown = SHOWN
        );
        ui.label(
            egui::RichText::new(more)
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    }

    ui.add_space(crate::ui::theme::SPACING_HAIR);
    let rows = matches.len();
    let list = Hits::Inside(doc_id);
    let mut asked = None;
    crate::ui::virtual_rows::show(ui, ("rpf_deep_find", doc_id), rows, None, |ui, row| {
        let path = &matches[row];
        let leaf = path.rsplit('/').next().unwrap_or(path);
        let color = if rpf_path_supported(leaf) {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_SECONDARY
        };
        let text = egui::RichText::new(path).color(color);
        ui.add_space(ui.spacing().indent);
        let selection = &mut state.deep_find.selection;
        let resp = search::result_row(ui, selection, row, rows, text);
        search::hits_menu(&resp, state, list, row, &mut asked);
        if resp.double_clicked() {
            asked = search::open_hit(state, list, row);
        }
    });
    if let Some(command) = asked {
        commands::run(state, ui.ctx(), &command);
    }
}

/// Apply what a frame of a name editor came to: keep editing, rename through
/// the gate and the refusal of a taken name, which reopens the editor, or drop it.
pub(super) fn finish_rename(
    state: &mut AppState,
    doc_id: DocumentId,
    node_id: RpfNodeId,
    old_name: &str,
    rename: RenameState,
    action: InlineRenameAction,
) {
    match action {
        InlineRenameAction::Editing => {
            super::ui_state(state).renaming = Some(rename);
        }
        InlineRenameAction::Commit => {
            let new_name = rename.buffer.trim().to_string();
            if new_name.is_empty() || new_name == old_name {
            } else if let Err(e) = state.rpf_rename_node(doc_id, node_id, &new_name) {
                state.show_error_toast(e);
                super::ui_state(state).renaming = Some(RenameState {
                    focus_pending: true,
                    buffer: new_name,
                    ..rename
                });
            }
        }
        InlineRenameAction::Cancel => {}
    }
}

pub(super) fn classify_rename_response(ui: &egui::Ui, resp: &egui::Response) -> InlineRenameAction {
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if escape && (resp.has_focus() || resp.lost_focus()) {
        return InlineRenameAction::Cancel;
    }
    if resp.lost_focus() {
        return InlineRenameAction::Commit;
    }
    InlineRenameAction::Editing
}

/// A row's menu, in the tree or, with `centre`, in the Files view, where a rename it starts is edited.
pub(super) fn rpf_context_menu(
    response: &egui::Response,
    state: &mut AppState,
    doc_id: DocumentId,
    node: &RpfSessionNode,
    asked: &mut Option<Command>,
    centre: bool,
) {
    let node_id = node.id;
    // ⚠ Before the menu reads the selection, so the menu and the keys after it name the same rows.
    if response.secondary_clicked() {
        select_unless_selected(state, doc_id, node_id);
    }
    let selection = state.selection.selected_ids(doc_id);
    let acts_on_selection = selection.len() > 1 && selection.contains(&node_id);
    response.context_menu(|ui| {
        crate::ui::popup_of_a_document(ui.ctx(), doc_id);
        let mut rows = vec![node_id];
        if acts_on_selection {
            rows.extend(selection.iter().copied().filter(|&row| row != node_id));
        }
        let Some(target) = Target::rows(state, doc_id, rows, centre) else {
            return;
        };
        if let Target::Rows { rows, .. } = &target {
            if rows
                .iter()
                .any(|&id| super::rows::is_rpf9(state, doc_id, id))
            {
                ui.label(tr!("Read-only"));
                if rows.len() == 1 {
                    commands::offer(ui, state, Action::OpenSelection, &target, asked);
                }
                commands::offer(ui, state, Action::Extract, &target, asked);
                commands::offer(ui, state, Action::CopyPath, &target, asked);
                if rows.len() == 1 {
                    if !node.is_dir() {
                        commands::offer(ui, state, Action::ExportAs, &target, asked);
                    }
                    commands::offer(ui, state, Action::Properties, &target, asked);
                    if node.backing.fs_path().is_some() {
                        commands::offer(ui, state, Action::RevealOnDisk, &target, asked);
                    }
                }
                return;
            }
        }
        if acts_on_selection {
            selection_menu(ui, state, node, &target, asked);
            return;
        }
        // ⚠ Open and Import first and as a pair, the two things to do with a game file, with a rule under them: Open looks at it where it is and changes nothing, and Import makes it a source of the project.
        if centre || !node.is_dir() {
            commands::offer(ui, state, Action::OpenSelection, &target, asked);
        }
        commands::offer(ui, state, Action::ImportIntoProject, &target, asked);
        ui.separator();
        if node.backing.fs_path().is_some() {
            commands::offer(ui, state, Action::RevealOnDisk, &target, asked);
        }
        super::uses::submenus(ui, state, doc_id, node);
        if !node.is_dir()
            && node.name.to_ascii_lowercase().ends_with(".ytd")
            && ui
                .button(tr!("Attach as texture dictionary"))
                .on_hover_text(tr!("Consult it for every document's textures, above the \
                     name and folder guesses."))
                .clicked()
        {
            match state.attach_texture_dictionary_from_archive(doc_id, node_id) {
                Ok(_) => state.show_toast(tr!("Attached {name}", name = node.name)),
                Err(message) => state.show_error_toast(message),
            }
            ui.close_kind(egui::UiKind::Menu);
        }
        super::files_menu::add_items(ui, state, &target, asked);
        // ⚠ The three comparisons are one row that opens them, so the menu fits the smallest window.
        if !node.is_dir() || matches!(node.backing, RpfNodeBacking::Archive { .. }) {
            crate::ui::commands::submenu(ui, tr!("Compare"), |ui| {
                for action in [
                    Action::SelectForCompare,
                    Action::CompareWithSelected,
                    Action::CompareWithOriginal,
                ] {
                    commands::offer(ui, state, action, &target, asked);
                }
            });
        }
        ui.separator();
        // ⚠ Every way of taking a copy of the file out stands together: Extract…, then Export as…, glTF and the other edition.
        commands::offer(ui, state, Action::Extract, &target, asked);
        more_extract_menu(ui, state, doc_id, node, vec![node_id]);
        if !node.is_dir() {
            commands::offer(ui, state, Action::ExportAs, &target, asked);
        }
        if state.offers_conversion(doc_id, &[node_id]) {
            commands::offer(ui, state, Action::Convert, &target, asked);
        }
        if state.offers_gltf(Model::Row(doc_id, node_id)) {
            commands::offer(ui, state, Action::ExportGltf, &target, asked);
            commands::offer(ui, state, Action::ImportGltf, &target, asked);
        }
        if matches!(node.backing, RpfNodeBacking::DiskFolder { .. }) {
            commands::offer(ui, state, Action::CreateDlcPack, &target, asked);
        }
        let overlay = node.backing.fs_path().and_then(|archive| {
            let copy = crate::state::retail::overlay_copy_target(archive, &state.prefs)?;
            Some((archive, copy))
        });
        if let Some((archive, copy)) = overlay {
            if copy.is_file() {
                if ui
                    .button(tr!("Show in mods overlay"))
                    .on_hover_text(copy.display().to_string())
                    .clicked()
                {
                    state.reveal_disk_path(&copy);
                    ui.close_kind(egui::UiKind::Menu);
                }
            } else if ui
                .add_enabled(
                    state.prefs.edit_mode.permits_overlay()
                        && state.overlay_copy.running().is_none(),
                    egui::Button::new(crate::ui::commands::behind_gutter(tr!(
                        "Copy to mods overlay…"
                    ))),
                )
                .on_disabled_hover_text(tr!(
                    "Needs the edit mode \"{mode}\", and no overlay copy running",
                    mode = crate::state::EditMode::Overlay.label()
                ))
                .clicked()
            {
                state.offer_overlay_copy(archive);
                ui.close_kind(egui::UiKind::Menu);
            }
        }
        let outgrown = node.backing.fs_path().and_then(|archive| {
            let copy = crate::state::retail::overlay_copy_target(archive, &state.prefs)
                .unwrap_or_else(|| archive.to_path_buf());
            state
                .overlay_copy
                .stale_copy(&copy)
                .map(|stale| stale.copy.clone())
        });
        if let Some(copy) = outgrown {
            if ui
                .add_enabled(
                    state.prefs.edit_mode.permits_overlay()
                        && state.overlay_copy.running().is_none(),
                    egui::Button::new(crate::ui::commands::behind_gutter(tr!(
                        "Refresh from the patched archive…"
                    ))),
                )
                .on_hover_text(tr!(
                    "Replaces this overlay copy with the game's archive as the update left it, \
                     after listing what of yours that discards."
                ))
                .on_disabled_hover_text(tr!(
                    "Needs the edit mode \"{mode}\", and no overlay copy running",
                    mode = crate::state::EditMode::Overlay.label()
                ))
                .clicked()
            {
                state.refresh_overlay_copy(&copy);
                ui.close_kind(egui::UiKind::Menu);
            }
        }
        if state.defrag_target(doc_id, node_id).is_some()
            && ui
                .add_enabled(
                    !state.defrag.is_running() && state.defrag.offer.is_none(),
                    egui::Button::new(crate::ui::commands::behind_gutter(tr!("Defragment…"))),
                )
                .on_hover_text(tr!(
                    "Rebuilds the archive without the gaps edits leave, after showing \
                     its size now and after. The first write keeps the file as it was \
                     beside it as .rpf.bak."
                ))
                .on_disabled_hover_text(tr!("A defragment is already under way"))
                .clicked()
        {
            state.defragment_archive(doc_id, node_id);
            ui.close_kind(egui::UiKind::Menu);
        }
        ui.separator();
        // ⚠ What changes the row and what copies a word of it are each one row that opens them: flat, the menu held more rows than the smallest window is tall.
        crate::ui::commands::submenu(ui, tr!("Edit"), |ui| {
            for action in [Action::Cut, Action::Copy, Action::Paste, Action::Rename] {
                commands::offer(ui, state, action, &target, asked);
            }
            crate::ui::commands::submenu(ui, tr!("Move to"), |ui| {
                let dirs: Vec<(RpfNodeId, String)> = state
                    .document(doc_id)
                    .and_then(|d| match &d.payload {
                        DocPayload::Rpf(rpf) => Some(
                            rpf.session
                                .all_directory_nodes()
                                .into_iter()
                                .filter_map(|id| rpf.session.path_of(id).map(|p| (id, p)))
                                .collect(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_default();
                for (target_id, target_path) in dirs {
                    let label = if target_path.is_empty() {
                        "/".to_string()
                    } else {
                        format!("/{target_path}")
                    };
                    if ui.button(label).clicked() {
                        if let Err(e) = state.rpf_move_node(doc_id, node_id, target_id) {
                            state.show_error_toast(e);
                        }
                        ui.close_kind(egui::UiKind::Menu);
                    }
                }
            });
            commands::offer(ui, state, Action::Delete, &target, asked);
        });
        crate::ui::commands::submenu(ui, tr!("Copy as"), |ui| {
            for action in [Action::CopyPath, Action::CopyName, Action::CopyHash] {
                commands::offer(ui, state, action, &target, asked);
            }
        });
        let at = Location {
            doc: doc_id,
            node: node_id,
        };
        let favourite = super::files_menu::favourite_action(state, at);
        commands::offer(ui, state, favourite, &target, asked);
        ui.separator();
        commands::offer(ui, state, Action::Properties, &target, asked);
    });
}

/// ⚠ **Only the verbs that mean the same thing for every row.** Move to names
/// one row, and a Delete that asked the edit gate once per row could stop
/// halfway with a prompt, so a multi-row menu offers extraction, the clipboard,
/// the paths and, on files a conversion takes, Convert, and says why Rename and
/// Delete… wait for one row.
fn selection_menu(
    ui: &mut egui::Ui,
    state: &mut AppState,
    node: &RpfSessionNode,
    target: &Target,
    asked: &mut Option<Command>,
) {
    let Target::Rows { doc, rows, .. } = target else {
        return;
    };
    ui.label(
        egui::RichText::new(format!("{} selected", rows.len()))
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    ui.separator();
    commands::offer(ui, state, Action::ImportIntoProject, target, asked);
    commands::offer(ui, state, Action::Extract, target, asked);
    more_extract_menu(ui, state, *doc, node, rows.clone());
    for action in [Action::Cut, Action::Copy, Action::Paste, Action::CopyPath] {
        commands::offer(ui, state, action, target, asked);
    }
    if state.offers_conversion(*doc, rows) {
        commands::offer(ui, state, Action::Convert, target, asked);
    }
    ui.separator();
    for action in [Action::Rename, Action::Delete] {
        commands::offer(ui, state, action, target, asked);
    }
}

/// Extracting only one type, or in another form than Extract…'s.
///
/// ⚠ **Every leaf here is a verb, and that is deliberate.** A menu where the
/// body form and the type filter are each a sticky choice makes "extract" mean
/// whatever two earlier clicks left behind, and one of those choices —
/// as-stored — produces files nothing can open.
fn more_extract_menu(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    node: &RpfSessionNode,
    nodes: Vec<RpfNodeId>,
) {
    use crate::state::extract_job::TypeFilter;
    use crate::state::extract_planning::Counting;
    use or_io::container::rpf::BodyForm;

    let node_id = node.id;
    let mut chosen: Option<(BodyForm, TypeFilter)> = None;

    crate::ui::commands::submenu(ui, tr!("More extract options"), |ui| {
        if node.is_dir() && nodes.len() == 1 {
            // ⚠ Clicks stay in the submenu: the click that starts the count must leave it open to show the count grow.
            let keep_open = egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            egui::containers::menu::SubMenuButton::new(tr!("Only one type"))
                .config(keep_open)
                .ui(ui, |ui| {
                    // ⚠ Lists what the tree holds and reads nothing: opening the archives under a row is minutes for an install's root, and only a click asks for it.
                    let (census, counting) = state.census_of(doc_id, node_id);
                    for (ext, count) in &census {
                        let label = if ext.is_empty() {
                            tr!("(no extension) — {count}", count = count)
                        } else {
                            format!(".{ext} — {count}")
                        };
                        if ui.button(label).clicked() {
                            chosen = Some((BodyForm::Decoded, TypeFilter::Only(vec![ext.clone()])));
                            ui.close_kind(egui::UiKind::Menu);
                        }
                    }
                    match counting {
                        Counting::Opening { left } => {
                            ui.label(
                                egui::RichText::new(tr!(
                                    "Counting: {left} more to open…",
                                    left = left
                                ))
                                .small()
                                .color(theme::TEXT_SECONDARY),
                            );
                        }
                        Counting::Idle { unread } if unread > 0 => {
                            if !census.is_empty() {
                                ui.separator();
                            }
                            let asked =
                                ui.button(tr!("Count types in archives…"))
                                    .on_hover_text(tr!(
                                "Opens every archive under this folder to count what is in it: \
                                 for an install's root, minutes of reading."
                            ));
                            if asked.clicked() {
                                state.count_types_under(doc_id, node_id);
                            }
                        }
                        Counting::Idle { .. } if census.is_empty() => {
                            ui.label(
                                egui::RichText::new(tr!("No files")).color(theme::TEXT_SECONDARY),
                            );
                        }
                        Counting::Idle { .. } => {}
                    }
                });
            ui.separator();
        }
        if ui
            .button(tr!("Decrypted…"))
            .on_hover_text(tr!(
                "The key applied and nothing inflated. A binary entry stays \
                 deflated; a resource is the same as decoded."
            ))
            .clicked()
        {
            chosen = Some((BodyForm::Decrypted, TypeFilter::All));
            ui.close_kind(egui::UiKind::Menu);
        }
        if ui
            .button(tr!("As stored (raw)…"))
            .on_hover_text(tr!(
                "Exactly the bytes the archive holds. ⚠ A resource's first \
                 sixteen bytes on disk are filler — its page flags live in \
                 the entry table — so a raw .ydr is the right length and \
                 will not open in anything, this editor included."
            ))
            .clicked()
        {
            chosen = Some((BodyForm::AsStored, TypeFilter::All));
            ui.close_kind(egui::UiKind::Menu);
        }
    });

    let Some((form, filter)) = chosen else {
        return;
    };
    let Some(dest) = file_dialogs::pick_extract_destination() else {
        return;
    };
    state.extract_rpf_nodes(doc_id, nodes, dest, form, filter);
}

pub(super) fn rpf_path_supported(name: &str) -> bool {
    crate::state::DocumentKind::from_path(std::path::Path::new(name)).is_some()
}
