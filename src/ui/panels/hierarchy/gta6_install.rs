//! Above a GTA VI folder's Files view: Scan every archive, then each archive's bodies tallied by
//! kind, and the bodies of the kinds toggled on across every archive, a click opening one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::*;
use crate::state::gta6_install_scan::{self as install, Hit, InstallFind, InstallFound, MOST_HITS};
use crate::state::rpf9_scan::{Ended, RowKind};
use crate::ui::panels::inspector::rpf9_row_kind_label as kind_label;

/// Rows a list shows before it scrolls.
const ROWS_IN_VIEW: f32 = 12.0;

/// The bar over the Files view of `doc` while it is a GTA VI install folder; nothing otherwise.
pub(super) fn draw(ui: &mut egui::Ui, state: &mut AppState, doc: DocumentId) {
    let Some(root) = state.document(doc).and_then(|d| d.path.clone()) else {
        return;
    };
    if !is_gta6(ui, &root) {
        return;
    }
    let found = install::found_under(&root);
    let mut open = None;
    egui::Panel::top(ui.id().with("gta6_install_bar"))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
        .show_inside(ui, |ui| {
            names_row(ui, state, &root);
            open = bar(ui, &root, found.as_deref());
        });
    if let Some((archive, hit, preview)) = open {
        state.open_install_hit(&archive, hit, preview);
    }
}

/// The names harvest of the folder at `root`: its button, or its Stop while it runs, and how many
/// names are held.
fn names_row(ui: &mut egui::Ui, state: &mut AppState, root: &Path) {
    use crate::state::gta6_names;
    ui.horizontal_wrapped(|ui| {
        ui.strong(tr!("Names"));
        if gta6_names::harvesting() {
            ui.spinner();
            if ui.button(tr!("Stop")).clicked() {
                state.cancel_job(crate::state::jobs::JobKind::TextureNames);
            }
            ui.label(
                egui::RichText::new(tr!("Harvesting the names the archives spell…"))
                    .color(theme::TEXT_SECONDARY),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        let button = ui.button(tr!("Harvest names")).on_hover_text(tr!("Reads every GTA VI archive under this folder once, as a job of the job list, for the names its bodies spell. They are kept, and from then on a hash a map states, a texture pack's key and a scanned drawable are shown by name."));
        if button.clicked() {
            state.harvest_gta6_names(root);
        }
        let held = gta6_names::vocabulary().len();
        let said = match held {
            0 => tr!("No name is held yet").to_string(),
            _ => tr_n!("{count} name held", "{count} names held", held),
        };
        ui.label(egui::RichText::new(said).color(theme::TEXT_SECONDARY));
    });
}

/// Whether the folder at `root` holds GTA VI, asked of the disk once a folder.
fn is_gta6(ui: &egui::Ui, root: &Path) -> bool {
    let id = egui::Id::new(("gta6_install_folder", root));
    if let Some(known) = ui.data(|d| d.get_temp::<bool>(id)) {
        return known;
    }
    let game = or_title::Game::detect_from_root(root);
    let known = root.is_dir() && crate::state::gta_vi::opens_archives_individually(game);
    ui.data_mut(|d| d.insert_temp(id, known));
    known
}

/// The scan's button and state, its tallies and the rows of the kinds toggled on; the row a
/// click opens, and whether in the preview tab.
fn bar(
    ui: &mut egui::Ui,
    root: &Path,
    found: Option<&Mutex<InstallFound>>,
) -> Option<(PathBuf, Hit, bool)> {
    let running = found.is_some_and(|f| install::lock(f).ended.is_none());
    ui.horizontal_wrapped(|ui| {
        ui.strong(tr!("Every archive of this folder"));
        if running {
            ui.spinner();
            let stop = ui.add_enabled(!install::stopping(), egui::Button::new(tr!("Stop")));
            if stop.clicked() {
                install::stop();
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        let label = match found {
            Some(_) => tr!("Scan every archive again"),
            None => tr!("Scan every archive"),
        };
        let button = ui.button(label).on_hover_text(tr!("Scans each GTA VI archive under this folder in turn, as a job of the job list: an archive scanned before is read from its kept scan. Its bodies are then tallied by kind, archive by archive, and the bodies of a kind are listed across them."));
        if button.clicked() {
            install::start(root);
        }
    });
    let found = found?;
    let key = found as *const Mutex<InstallFound> as usize;
    let found = install::lock(found);
    ui.label(egui::RichText::new(status(&found)).color(theme::TEXT_SECONDARY));
    if found.cut {
        ui.colored_label(
            theme::WARNING,
            tr!(
                "The walk stops at {folders} folders and {archives} archives, so the archives past them are not scanned.",
                folders = install::MOST_FOLDERS,
                archives = install::MOST_ARCHIVES
            ),
        );
    }
    if found.archives.is_empty() {
        return None;
    }
    let row_height = ui.spacing().interact_size.y;
    egui::CollapsingHeader::new(tr!("Bodies by kind, archive by archive"))
        .id_salt("gta6_install_tallies")
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("gta6_install_tally_rows")
                .max_height(row_height * ROWS_IN_VIEW)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, found.archives.len(), |ui, range| {
                    for archive in &found.archives[range] {
                        tally_row(ui, root, archive);
                    }
                });
        });
    find_by_kind(ui, key, &found, root)
}

fn status(found: &InstallFound) -> String {
    let done = found.archives.len();
    match &found.ended {
        None => match found.listed {
            Some(total) => tr!(
                "Scanning: {done} of {total} archives scanned",
                done = done,
                total = total
            ),
            None => tr!("Looking for the archives under this folder…").to_string(),
        },
        Some(Ended::Whole) => tr_n!("{count} archive scanned", "{count} archives scanned", done),
        Some(Ended::Stopped) => tr_n!(
            "Stopped after {count} archive",
            "Stopped after {count} archives",
            done
        ),
        Some(Ended::Failed(why)) => why.clone(),
    }
}

/// One archive on one row, as the list scrolls by rows: its path under the folder, how many of
/// its rows are of each kind, and why its scan failed.
fn tally_row(ui: &mut egui::Ui, root: &Path, archive: &install::ArchiveTally) {
    let name = archive.path.strip_prefix(root).unwrap_or(&archive.path);
    let counts: Vec<String> = (RowKind::ALL.iter())
        .filter_map(|kind| {
            let count = archive.counts.get(kind)?;
            Some(format!("{} {count}", kind_label(*kind)))
        })
        .collect();
    ui.horizontal(|ui| {
        ui.strong(name.display().to_string());
        ui.label(
            egui::RichText::new(counts.join(", "))
                .small()
                .color(theme::TEXT_SECONDARY),
        );
        if let Ended::Failed(why) = &archive.ended {
            ui.colored_label(theme::WARNING, why).on_hover_text(why);
        }
    });
}

/// A body's line in the list across the archives, in which the Find field's words are sought.
fn line(root: &Path, archive: &install::ArchiveTally, hit: &Hit) -> String {
    let name = archive.path.strip_prefix(root).unwrap_or(&archive.path);
    let kind = kind_label(hit.kind);
    format!("{}  {:#012X}  {kind}", name.display(), hit.offset)
}

/// The Find field, a toggle for each kind with its count across the archives, and the bodies
/// the field and the kinds on keep.
fn find_by_kind(
    ui: &mut egui::Ui,
    key: usize,
    found: &InstallFound,
    root: &Path,
) -> Option<(PathBuf, Hit, bool)> {
    let mut totals: BTreeMap<RowKind, usize> = BTreeMap::new();
    for (kind, count) in found.archives.iter().flat_map(|a| &a.counts) {
        *totals.entry(*kind).or_insert(0) += count;
    }
    let text_id = egui::Id::new(("gta6_install_find_text", root));
    let mut text: String = ui.data(|d| d.get_temp(text_id)).unwrap_or_default();
    let field = egui::TextEdit::singleline(&mut text)
        .hint_text(tr!("Find by archive, offset or kind, or go to 0x offset"))
        .desired_width(f32::INFINITY);
    if ui.add(field).changed() {
        ui.data_mut(|d| d.insert_temp(text_id, text.clone()));
    }
    let kinds_id = egui::Id::new(("gta6_install_kinds", root));
    let mut kinds: BTreeSet<RowKind> = ui.data(|d| d.get_temp(kinds_id)).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(tr!("Find by kind:"));
        // A missing run is no body, so no row of the list.
        for kind in RowKind::ALL.into_iter().filter(|k| *k != RowKind::Missing) {
            let count = totals.get(&kind).copied().unwrap_or(0);
            let mut on = kinds.contains(&kind);
            if count == 0 && !on {
                continue;
            }
            let toggle = ui.toggle_value(&mut on, format!("{} {count}", kind_label(kind)));
            if toggle.changed() {
                match on {
                    true => kinds.insert(kind),
                    false => kinds.remove(&kind),
                };
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(kinds_id, kinds.clone()));
    if kinds.is_empty() && text.trim().is_empty() {
        return None;
    }
    let held: usize = found.archives.iter().map(|a| a.hits.len()).sum();
    if held >= MOST_HITS {
        ui.colored_label(
            theme::WARNING,
            tr!(
                "The list keeps at most {count} bodies across the archives: the rest are counted and not listed.",
                count = MOST_HITS
            ),
        );
    }
    // Kept for the scan in hand: an archive the scan adds is examined alone, and a change of the
    // field or the kinds examines every body again, a budget a frame.
    let id = egui::Id::new(("gta6_install_rows", root));
    let shared = match ui.data(|d| d.get_temp::<(usize, Arc<Mutex<InstallFind>>)>(id)) {
        Some((scan, shared)) if scan == key => shared,
        _ => {
            let shared = Arc::new(Mutex::new(InstallFind::default()));
            ui.data_mut(|d| d.insert_temp(id, (key, Arc::clone(&shared))));
            shared
        }
    };
    let mut find = shared.lock().unwrap_or_else(|p| p.into_inner());
    let (rows, examining) = find.shown(found, &text, &kinds, |a, h| line(root, a, h));
    if let Some(offset) = install::offset_in(&text) {
        ui.label(
            egui::RichText::new(tr!(
                "Each archive's body that starts last at or before {offset}",
                offset = format!("{offset:#X}")
            ))
            .color(theme::TEXT_SECONDARY),
        );
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(tr_n!(
                "{count} body of those kinds: a click shows it, a double click keeps its tab",
                "{count} bodies of those kinds: a click shows one, a double click keeps its tab",
                rows.len()
            ))
            .color(theme::TEXT_SECONDARY),
        );
        if examining {
            ui.spinner();
            ui.ctx().request_repaint();
        }
    });
    let (mut open, row_height) = (None, ui.spacing().interact_size.y);
    egui::ScrollArea::vertical()
        .id_salt(("gta6_install_found", root))
        .max_height(row_height * ROWS_IN_VIEW)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            for &(a, h) in &rows[range] {
                let (archive, hit) = (&found.archives[a], found.archives[a].hits[h]);
                let response = ui.selectable_label(false, line(root, archive, &hit));
                if response.double_clicked() {
                    open = Some((archive.path.clone(), hit, false));
                } else if response.clicked() {
                    open = Some((archive.path.clone(), hit, true));
                }
            }
        });
    open
}
