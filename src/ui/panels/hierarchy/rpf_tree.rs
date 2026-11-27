//! The archive tree of Files: every row listed each frame, and the rows in view drawn.

use super::rows::{
    maybe_drop_into_folder, note_row, select_and_visit, selection_modifier_click, start_drag,
};
use super::rpf::{classify_rename_response, finish_rename, rpf_context_menu, rpf_path_supported};
use super::*;
use crate::state::explorer::{gta6_kind, row_matches};
use crate::state::RpfEditSession;
use crate::ui::commands::{self, Action, Command, Target};
use crate::ui::panels::registry::ids;
use or_io::container::rpf::Rpf9Kind;
use std::collections::HashSet;
use std::sync::Arc;

/// The name a GTA VI row's icon and support check look at: its own with the Windows (`y`-prefixed)
/// form of the kind's extension, substituted into any stored `#`-prefix the name already carries,
/// or appended where the name has none. `DocumentKind::from_path` only recognises the Windows form,
/// so this is always the `y` letter — the user-facing display uses the build's platform letter.
///
/// ⚠ Only the icon's: the row keeps the name its table states.
pub(super) fn icon_name(name: &str, kind: Option<Rpf9Kind>) -> Option<String> {
    use or_io::container::rpf::substitute_platform_letter;
    let stated = substitute_platform_letter(name, 'y');
    let has_extension = std::path::Path::new(&stated).extension().is_some();
    if has_extension {
        return Some(stated);
    }
    let ext = kind?.extension_pc()?;
    Some(format!("{stated}.{ext}"))
}

/// The name of a row as it is shown, with the build's platform letter substituted for any `#` the
/// stored name carries, for any concrete RAGE platform extension already baked in (`n01.ybn` →
/// `n01.zbn` on an Xbox Series build), and the kind's extension appended where the name has none.
pub(super) fn displayed_name(name: &str, kind: Option<Rpf9Kind>, letter: char) -> String {
    use or_io::container::rpf::substitute_platform_letter;
    let stated = substitute_platform_letter(name, letter);
    let rewritten = rewrite_platform_extension(&stated, letter);
    if std::path::Path::new(&rewritten).extension().is_some() {
        return rewritten;
    }
    match kind.and_then(|k| k.extension_for(letter)) {
        Some(ext) => format!("{rewritten}.{ext}"),
        None => rewritten,
    }
}

/// Swap the first letter of a RAGE platform extension at the end of `path` for `letter`.
///
/// The names vocabulary ships paths with a platform extension baked in (`n01_e01_0.ybn`,
/// `models/car.ydr`), so on an Xbox Series build the row should still read `.zbn` / `.zdr`.
/// Only extensions a RAGE platform letter plus a known stub make (`dr`, `ft`, `td`, `bn`, `map`,
/// `typ`, …) are rewritten; `rpf`, `dat`, `gfx`, `mrf`, `meta`, `cut` and anything else stays.
/// The stub list is `or_io`'s own, shared with the archive's name-based platform detection.
fn rewrite_platform_extension(path: &str, letter: char) -> String {
    use or_io::container::rpf::{platform_letter_of_name, RAGE_STUB_TAILS};
    let Some(dot) = path.rfind('.') else {
        return path.to_string();
    };
    // Reuse the shared matcher so the set of rewritable extensions never drifts from detection.
    let Some(first) = platform_letter_of_name(path) else {
        return path.to_string();
    };
    if first as char == letter {
        return path.to_string();
    }
    let tail = &path[dot + 2..];
    debug_assert!(RAGE_STUB_TAILS.contains(&tail.to_ascii_lowercase().as_str()));
    let mut out = String::with_capacity(path.len());
    out.push_str(&path[..=dot]);
    out.push(letter);
    out.push_str(tail);
    out
}

/// The platform letter a row of this archive reads under, resolved from the most authoritative
/// source down to a default:
///
/// 1. **The archive's own header**, where its format records the platform. RPF8 keeps the letter
///    at `0x0A` and RPF9 at `0x19`, so an RDR2 or GTA VI archive reports `o`/`p`/`d`/`z`/… straight
///    from the file — no context needed.
/// 2. **The archive's own stored names**, for an older format whose table keeps full names as text
///    (RPF7 and earlier, IMG): `n01.ydr`, `prop.xdr` carry the platform letter in their extension,
///    so the most common one across the table is the archive's platform. Hash-named tables give
///    nothing here.
/// 3. **The active install**, when neither the header nor the names say, and the install's profile
///    ships the archive's container family. The user is browsing their own install.
/// 4. **The canonical title** for the container family (GTA VI for RPF9, RDR2 for RPF8, GTA V for
///    every RPF7-and-before), then `y`, where even that is unknown.
///
/// Steps 1–2 read the platform out of the archive itself; only 3–4 lean on context, and only for a
/// format that neither records the platform nor keeps readable names (a hash-named RPF7).
pub(super) fn platform_letter_for(
    state: &AppState,
    session: &RpfEditSession,
    node: &RpfSessionNode,
) -> char {
    let archive = archive_of(session, node);
    // 1. The archive's header, where the format records a platform (RPF8, RPF9).
    if let Some(letter) = archive.and_then(|a| a.platform_letter()) {
        return letter;
    }
    // 2. The archive's own names, where an older format keeps them as text (RPF7 and earlier, IMG).
    if let Some(letter) = archive.and_then(|a| a.platform_letter_from_names()) {
        return letter;
    }
    let archive_family = archive.map(|a| container_family_for_version(a.version));
    let active = state.prefs.active_game;
    // 3. The active install, when it ships the same container family as the archive.
    if let (Some(family), Some(game)) = (archive_family, active) {
        if game.profile().container() == family {
            return game.platform_letter();
        }
    }
    // 4. The canonical title for the family, then `y` where none is known.
    match archive_family {
        Some(family) => canonical_game_for_container(family).platform_letter(),
        None => active.map(|g| g.platform_letter()).unwrap_or('y'),
    }
}

/// The archive the row sits in, `None` for a row outside one (a disk folder, the install root).
fn archive_of<'a>(
    session: &'a RpfEditSession,
    node: &RpfSessionNode,
) -> Option<&'a or_io::container::RpfArchive> {
    let archive_id = match &node.backing {
        RpfNodeBacking::ArchiveFile { archive_id, .. } => *archive_id,
        _ => return None,
    };
    Some(session.archive(archive_id)?.archive.as_ref())
}

/// The container family (`RpfVersion` has more variants than the profile tracks).
fn container_family_for_version(
    version: or_io::container::rpf::version::RpfVersion,
) -> or_title::ContainerFamily {
    use or_io::container::rpf::version::RpfVersion;
    use or_title::ContainerFamily;
    match version {
        RpfVersion::Rpf9 => ContainerFamily::Rpf9,
        RpfVersion::Rpf8 => ContainerFamily::Rpf8,
        _ => ContainerFamily::Rpf7,
    }
}

/// The title the tree falls back to when no active install matches the archive's container.
fn canonical_game_for_container(family: or_title::ContainerFamily) -> or_title::Game {
    use or_title::{ContainerFamily, Game};
    match family {
        ContainerFamily::Rpf9 => Game::GtaVI,
        ContainerFamily::Rpf8 => Game::Rdr2,
        ContainerFamily::Rpf7 => Game::GtaVLegacy,
    }
}

/// The stem a GTA VI drawable or fragment body stated for `node` on open, where `(archive, offset)`
/// gave [`crate::state::gta6_learned_names::lookup`] one; `None` for a row of any other archive
/// or a row whose body never opened.
pub(super) fn learned_name_for(session: &RpfEditSession, node: &RpfSessionNode) -> Option<String> {
    let (archive_id, entry_index) = match &node.backing {
        RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            ..
        } => (*archive_id, *entry_index),
        _ => return None,
    };
    let archive = &session.archive(archive_id)?.archive;
    let place = archive.rpf9_place(entry_index)?;
    crate::state::gta6_learned_names::lookup(&place.file, place.offset)
}

/// The dimmed words after a file row's name: its kind where its archive's table names one, then its size.
fn detail_text(kind: Option<Rpf9Kind>, size: Option<u64>) -> String {
    let size = size.map(|sz| format!("{:.1} KiB", (sz as f32) / 1024.0));
    match (kind, size) {
        (Some(kind), Some(size)) => format!("{} · {size}", kind.label),
        (Some(kind), None) => kind.label.to_owned(),
        (None, size) => size.unwrap_or_default(),
    }
}

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
    matches!(
        node.backing,
        RpfNodeBacking::DiskFolder { scanned: false, .. }
            | RpfNodeBacking::Archive {
                scanned: false,
                open_error: None,
                ..
            }
    )
}

/// The folders a filter leaves in the tree: each whose name holds the lower-case `needle`, each not yet listed, whose rows the filter has still to reach, and each above either or a loaded row that matches.
fn folders_kept(session: &RpfEditSession, needle: &str) -> HashSet<RpfNodeId> {
    let mut kept = HashSet::new();
    for id in session.all_nodes() {
        let Some(node) = session.node(id) else {
            continue;
        };
        let (hit, mut up) = if node.is_dir() || needs_expand(node) {
            let named = node.name.to_ascii_lowercase().contains(needle);
            (named || needs_expand(node), Some(id))
        } else {
            (
                row_matches(&node.name, gta6_kind(session, node), needle),
                node.parent,
            )
        };
        if !hit {
            continue;
        }
        while let Some(at) = up.filter(|&at| kept.insert(at)) {
            up = session.node(at).and_then(|n| n.parent);
        }
    }
    kept
}

/// [`folders_kept`] for this frame's filter, worked out again only when the filter, the count of
/// rows or the document's `epoch` changes.
///
/// ⚠ Keyed on the epoch as well as the count of rows: a rename changes no count.
fn folders_kept_now(
    ctx: &egui::Context,
    doc_id: DocumentId,
    epoch: u64,
    session: &RpfEditSession,
    needle: &str,
) -> Arc<HashSet<RpfNodeId>> {
    type Kept = (String, usize, u64, Arc<HashSet<RpfNodeId>>);
    let id = egui::Id::new(("rpf_tree_folders_kept", doc_id));
    let nodes = session.file_count() + session.directory_count();
    let cached = ctx.data(|d| d.get_temp::<Kept>(id));
    if let Some((n, count, at, kept)) = cached {
        if n == needle && count == nodes && at == epoch {
            return kept;
        }
    }
    let kept = Arc::new(folders_kept(session, needle));
    let entry: Kept = (needle.to_owned(), nodes, epoch, Arc::clone(&kept));
    ctx.data_mut(|d| d.insert_temp(id, entry));
    kept
}

/// Every row the tree shows, top to bottom: the rows under each open folder, less the rows the filter drops.
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
    let doc = state.document(doc_id);
    if let Some((DocPayload::Rpf(rpf), epoch)) = doc.map(|d| (&d.payload, d.payload_epoch())) {
        let session = &rpf.session;
        let sort = state.prefs.explorer.sort;
        let files = super::ui_state_ref(state);
        let needle = files.map(|f| f.filter.to_ascii_lowercase());
        let needle = needle.as_deref().unwrap_or_default();
        // While a filter is set, a folder stays only where it leads to a match.
        let kept =
            (!needle.is_empty()).then(|| folders_kept_now(ctx, doc_id, epoch, session, needle));
        let pulse = pulse_state_from_ctx(ctx);
        // A row waiting to be listed: its depth, its parent's id, and whether it is shown or only closed.
        let mut waiting: Vec<(RpfNodeId, usize, egui::Id, bool)> =
            explorer::sorted_children_at(session, epoch, session.root_id(), sort)
                .iter()
                .rev()
                .map(|&child| (child, 0, id_root, true))
                .collect();
        while let Some((id, depth, parent, shown)) = waiting.pop() {
            let Some(node) = session.node(id) else {
                continue;
            };
            let id_here = parent.with(("rpf_node", id));
            if !node.is_dir() {
                if shown
                    && (needle.is_empty()
                        || row_matches(&node.name, gta6_kind(session, node), needle))
                {
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
            let shown = shown && kept.as_ref().is_none_or(|kept| kept.contains(&id));
            let mut collapsing = egui::collapsing_header::CollapsingState::load_with_default_open(
                ctx, id_here, false,
            );
            let was_open = collapsing.is_open();
            let revealed = files.is_some_and(|f| f.reveal_ancestors.contains(&id));
            let open = revealed || pulse.unwrap_or(was_open);
            if open != was_open {
                collapsing.set_open(open);
                collapsing.store(ctx);
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
            let children = explorer::sorted_children_at(session, epoch, id, sort);
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
                    .iter()
                    .rev()
                    .map(|&child| (child, depth + 1, id_here, shown)),
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
        if sole {
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
    let Some((node, unsaved, size, bound_doc_id, kind, learned, letter)) =
        state.document(doc_id).and_then(|d| match &d.payload {
            DocPayload::Rpf(rpf) => {
                let node = rpf.session.node(node_id)?.without_children();
                let size = (!node.is_dir())
                    .then(|| rpf.session.size_for(node_id))
                    .flatten();
                let kind = gta6_kind(&rpf.session, &node);
                // The stem of the stated model name a drawable or fragment body states on open,
                // where its row's name was never resolved; `None` for any other row.
                let learned = kind
                    .filter(|_| node.name.starts_with("hash_"))
                    .and_then(|_| learned_name_for(&rpf.session, &node));
                // The platform letter rows of this archive read under, so a Windows-form name
                // the vocabulary ships (`n01.ybn`) renders as `.zbn` on an Xbox Series archive.
                let letter = platform_letter_for(state, &rpf.session, &node);
                Some((
                    node,
                    rpf.session.archive_unsaved(node_id),
                    size,
                    rpf.entry_documents.get(&node_id).copied(),
                    kind,
                    learned,
                    letter,
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

    let mut taken_rename: Option<RenameState> =
        if super::ui_state(state).is_renaming_rpf(doc_id, node_id) {
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
            _ => (None, None),
        };
        // ⚠ GTA VI's refusal is no failure: the archive opens as a scan, and is drawn as one that opens.
        let scans = archive_open_error.is_some_and(crate::state::gta_vi::is_rpf9_refusal);
        let outgrown = node
            .backing
            .fs_path()
            .is_some_and(|path| state.overlay_copy.stale_copy(path).is_some());
        let label_color = if selected {
            theme::ACCENT_TEXT
        } else if archive_open_error.is_some() && !scans {
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
        if !renaming {
            // ⚠ GTA VI's refusal says why itself; the GTA V key advice below is not true of it.
            // The words are made only once the pointer rests on the row.
            let resp = if let Some(err) = archive_open_error.filter(|_| scans) {
                resp.on_hover_ui(|ui| {
                    ui.set_max_width(ui.spacing().tooltip_width);
                    ui.label(tr!(
                        "{why}\n\nIt opens as a scan of what it holds, in a tab of its own.",
                        why = err
                    ));
                })
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
        // The name used for the icon and the label. Where a drawable body stated its model name
        // on open, that stem stands in for the row's `hash_<hex>`.
        let row_name = learned.clone().unwrap_or_else(|| node.name.clone());
        // A GTA VI row's kind picks its icon as a GTA V row's extension does; the editor's
        // DocumentKind lookup works on the Windows form, which is what `icon_name` substitutes.
        let shown = icon_name(&row_name, kind).map(|name| RpfSessionNode {
            name,
            ..node.clone()
        });
        let shown = shown.as_ref().unwrap_or(&node);
        // A GTA VI row whose table names its kind is counted as supported even where no
        // `DocumentKind` yet claims its extension: the row is a known RAGE format, the icon
        // should not say "blocked" and the row should not dim like a stray file.
        let supported = rpf_path_supported(&shown.name) || kind.is_some();
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
            let icon = icons::rpf_node_icon(shown, unsaved, in_flight, supported, false);
            // The visible row name carries the build's platform letter (`#dr` → `ydr` on PC,
            // `zdr` on Xbox Series), or the kind's extension where the row states no name.
            // Where the body stated its own model name on open, that stem replaces the hash.
            let display = displayed_name(&row_name, kind, letter);
            let label = icons::icon_label(&icon, &display, color.into());
            let r = selectable_header(ui, selected, label);
            ui.add(detail_label(detail_text(kind, size)));
            r
        };
        note_row(state, doc_id, node_id, resp.rect);
        let resp = resp.interact(egui::Sense::click_and_drag());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RpfSessionNodeKind;

    const DRAWABLE: Rpf9Kind = Rpf9Kind {
        label: "drawable",
        // `Rpf9Kind::extension` stores the platform-independent RAGE stub (`#dr`); the Windows
        // form (`ydr`) is what `extension_pc()` and `DocumentKind::from_path` see.
        extension: Some("#dr"),
    };
    const MEDIA: Rpf9Kind = Rpf9Kind {
        label: "media",
        extension: None,
    };

    fn file(name: &str) -> RpfSessionNode {
        RpfSessionNode {
            id: 1,
            parent: None,
            name: name.to_owned(),
            kind: RpfSessionNodeKind::BinaryFile,
            children: Vec::new(),
            backing: RpfNodeBacking::InMemoryFile {
                bytes: std::sync::Arc::new(Vec::new()),
            },
        }
    }

    #[test]
    fn a_gta_vi_row_shows_its_kind_where_the_size_is_and_takes_the_icon_of_its_gta_v_kind() {
        let name = "hash_00000000000000ab";
        assert_eq!(
            detail_text(Some(DRAWABLE), Some(2048)),
            "drawable · 2.0 KiB"
        );
        assert_eq!(detail_text(Some(MEDIA), None), "media");
        let shown = file(&icon_name(name, Some(DRAWABLE)).expect("a drawable has an extension"));
        assert!(rpf_path_supported(&shown.name));
        let icon = icons::rpf_node_icon(&shown, false, false, true, false);
        assert_eq!(
            icon.glyph,
            icons::doc_kind_icon(DocumentKind::Ydr, false).glyph
        );
        assert_eq!(
            icon_name(name, Some(MEDIA)),
            None,
            "media names no GTA V format"
        );
    }

    #[test]
    fn a_displayed_name_substitutes_the_platform_letter_and_appends_a_kind_s_extension() {
        // A hash-form row gets the kind's extension with the build's letter: `ydr` on PC, `zdr`
        // on Xbox Series.
        assert_eq!(
            displayed_name("hash_000000000000ab00", Some(DRAWABLE), 'y'),
            "hash_000000000000ab00.ydr"
        );
        assert_eq!(
            displayed_name("hash_000000000000ab00", Some(DRAWABLE), 'z'),
            "hash_000000000000ab00.zdr"
        );
        // A path that already carries the `#dr` stub resolves to the platform's letter.
        assert_eq!(
            displayed_name("models/car.#dr", Some(DRAWABLE), 'y'),
            "models/car.ydr"
        );
        assert_eq!(
            displayed_name("models/car.#dr", Some(DRAWABLE), 'z'),
            "models/car.zdr"
        );
        // A path already in its concrete form stays as is for the matching letter.
        assert_eq!(
            displayed_name("models/car.ydr", Some(DRAWABLE), 'y'),
            "models/car.ydr"
        );
        // A `y`-prefixed extension the names vocabulary ships rewrites to the build's letter.
        assert_eq!(
            displayed_name("models/car.ydr", Some(DRAWABLE), 'z'),
            "models/car.zdr"
        );
        assert_eq!(
            displayed_name("terrain/n01_e01_0.ybn", None, 'z'),
            "terrain/n01_e01_0.zbn"
        );
        // A bare extension like `.rpf` / `.dat` / `.meta` isn't RAGE-shaped, so it stays.
        assert_eq!(
            displayed_name("packs/base.rpf", None, 'z'),
            "packs/base.rpf"
        );
        assert_eq!(
            displayed_name("world/heightmap.dat", None, 'z'),
            "world/heightmap.dat"
        );
        // A row with no kind keeps its name whatever letter is asked.
        assert_eq!(displayed_name("plain_file", None, 'z'), "plain_file");
    }

    #[test]
    fn the_filter_finds_a_gta_vi_row_by_its_kind_or_its_gta_v_extension() {
        let name = "hash_00000000000000ab";
        for needle in ["drawable", "draw", ".ydr", "ydr", "00ab"] {
            assert!(row_matches(name, Some(DRAWABLE), needle), "{needle}");
        }
        assert!(!row_matches(name, Some(DRAWABLE), "texture"));
        assert!(!row_matches(name, Some(MEDIA), ".ydr"));
        assert!(row_matches(name, None, "0xab"));
        assert!(!row_matches(name, None, "0xcd"));
        assert!(!row_matches("0xab.ydr", None, "0xcd"));
    }

    #[test]
    fn a_filtered_tree_lists_the_match_and_the_folders_above_it_only() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let levels = session.add_folder(root, "levels").expect("levels");
        let gta5 = session.add_folder(levels, "gta5").expect("gta5");
        session
            .add_file_copy(gta5, "props.ydr", vec![1])
            .expect("ydr");
        session
            .add_file_copy(gta5, "props.ytd", vec![1])
            .expect("ytd");
        let audio = session.add_folder(root, "audio").expect("audio");
        session
            .add_file_copy(audio, "sfx.awc", vec![1])
            .expect("awc");
        session
            .add_file_copy(root, "readme.txt", vec![1])
            .expect("txt");
        let mut state = AppState::default();
        let doc = state.new_empty_document(DocumentKind::Rpf);
        let Some(DocPayload::Rpf(rpf)) = state.document_mut(doc).map(|d| &mut d.payload) else {
            panic!("an empty archive document holds an archive");
        };
        rpf.session = session;
        let ctx = egui::Context::default();
        write_pulse_to_ctx(&ctx, Some(true));
        let listed = |state: &mut AppState, filter: &str| -> Vec<String> {
            crate::ui::panels::hierarchy::ui_state(state).filter = filter.to_owned();
            let rows = list_rows(&ctx, state, doc, egui::Id::new("tree"));
            let Some(DocPayload::Rpf(rpf)) = state.document(doc).map(|d| &d.payload) else {
                return Vec::new();
            };
            rows.iter()
                .filter_map(|row| Some(rpf.session.node(row.node)?.name.clone()))
                .collect()
        };
        assert_eq!(listed(&mut state, "YDR"), ["levels", "gta5", "props.ydr"]);
        assert_eq!(listed(&mut state, "gta5"), ["levels", "gta5"]);
        assert_eq!(listed(&mut state, "nothing"), Vec::<String>::new());
        assert_eq!(
            listed(&mut state, "").len(),
            7,
            "with no filter every row is listed"
        );
        // A folder not yet listed stays under any filter, so that the filter reaches its rows.
        let Some(DocPayload::Rpf(rpf)) = state.document_mut(doc).map(|d| &mut d.payload) else {
            panic!("the archive document");
        };
        let root = rpf.session.root_id();
        let unlisted = rpf.session.add_folder(root, "x64a").expect("x64a");
        rpf.session.node_mut(unlisted).expect("x64a").backing = RpfNodeBacking::DiskFolder {
            fs_path: std::path::PathBuf::from("x64a"),
            scanned: false,
        };
        let listed_now = listed(&mut state, "ydr");
        assert!(listed_now.iter().any(|n| n == "x64a"), "{listed_now:?}");
        assert!(
            listed_now.iter().any(|n| n == "props.ydr"),
            "{listed_now:?}"
        );
        let nothing = listed(&mut state, "nothing");
        assert!(
            !nothing.is_empty() && nothing.iter().all(|n| n == "x64a"),
            "{nothing:?}"
        );
    }

    #[test]
    fn a_row_with_no_kind_draws_and_filters_as_it_did() {
        let name = "hash_00000000000000ab";
        assert_eq!(detail_text(None, Some(2048)), "2.0 KiB");
        assert_eq!(detail_text(None, None), "");
        assert_eq!(icon_name(name, None), None);
        assert!(row_matches(name, None, "hash_"));
        assert!(!row_matches(name, None, "drawable"));
        assert!(row_matches("a.ydr", None, ".ydr"));
    }
}
