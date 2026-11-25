//! Copy, cut and paste: rows between archives and folders and with Explorer,
//! a field's value into a field of its type, and records into another file.
//!
//! ⚠ **A paste of rows is an import.** Copied rows are held as files, each a
//! row's own file on disk or a copy written under the temporary folder, so a
//! paste here and a paste in Explorer read the same files, and every write a
//! paste makes goes through the import's checks: the edit gate and a name
//! already taken.

mod fields;
mod records;

use std::path::{Path, PathBuf};

pub use fields::field_name;

use super::document::{DocumentId, RpfNodeId};
use super::os_drop::DRAG_OUT_LIMIT;
use super::rpf_session::RpfEditSession;
use super::{
    AppState, ArchiveSource, DocPayload, GateOutcome, PendingRetailEdit, PendingRetailEditPrompt,
    RpfNodeBacking,
};
use crate::platform::clipboard;

/// What Copy or Cut last held for a paste; a copy of one kind replaces a copy of another.
#[derive(Debug, Default)]
pub struct Clipboard {
    held: Option<Held>,
}

impl crate::ui::panel_state::PanelState for Clipboard {
    const KEY: &'static str = "clipboard";
}

#[derive(Debug, Clone)]
enum Held {
    Rows(Rows),
    Field(fields::FieldValue),
    Records(records::Records),
}

/// Copied rows, as the files a paste reads.
#[derive(Debug, Clone)]
struct Rows {
    files: Vec<PathBuf>,
    /// A cut's document and rows, one beside each file, which leave once their paste lands.
    cut: Option<(DocumentId, Vec<RpfNodeId>)>,
}

impl AppState {
    /// Copy `rows` of `doc`, or cut them: offered to Explorer as files and held for a paste.
    pub fn copy_rows(
        &mut self,
        doc: DocumentId,
        rows: &[RpfNodeId],
        cut: bool,
    ) -> Result<String, String> {
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return Err(tr!("The archive is no longer open").into());
        };
        if cut {
            for &row in rows {
                self.require_writable_rpf_row(doc, row)?;
            }
        }
        let session = &rpf.session;
        let rows = top_rows(session, rows);
        let named = match rows.as_slice() {
            [] => return Err(tr!("Select a file or folder to copy").into()),
            [one] => session
                .node(*one)
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            many => format!("{} rows", many.len()),
        };
        let scratch = crate::ui::os_drop::staging_dir("openrage-clipboard");
        let files = files_of_rows(session, &rows, &scratch)?;
        let offered = clipboard::offer_files(&files);
        self.panels.get::<Clipboard>().held = Some(Held::Rows(Rows {
            files,
            cut: cut.then(|| (doc, rows.clone())),
        }));
        let verb = if cut { tr!("Cut") } else { tr!("Copied") };
        Ok(match offered {
            Ok(()) if crate::platform::standing_in() => tr!("{verb} {named}: the clipboard's stand-in holds it; paste it into an archive or a folder", verb = verb, named = named),
            Ok(()) => tr!("{verb} {named}: paste it into an archive, a folder or Explorer", verb = verb, named = named),
            Err(why) => {
                tracing::warn!(%why, "the copied rows were not offered to Explorer");
                tr!("{verb} {named}: paste it into an archive or a folder; {why}", verb = verb, named = named, why = why)
            }
        })
    }

    /// Paste into the row `folder` of `doc` the files Explorer copied last, else what Copy or Cut held.
    pub fn paste_rows(&mut self, doc: DocumentId, folder: RpfNodeId) -> Result<(), String> {
        self.require_writable_rpf_row(doc, folder)?;
        let elsewhere = clipboard::files_from_elsewhere();
        let held = self.held_rows();
        let files = match (&held, elsewhere.is_empty()) {
            (_, false) => elsewhere.clone(),
            (Some(held), true) => held.files.clone(),
            (None, true) => {
                return Err(
                    tr!("Nothing to paste: copy or cut files first, here or in Explorer").into(),
                )
            }
        };
        if let Some(why) = self.importing_into_itself(doc, folder, &files) {
            return Err(why);
        }
        if !elsewhere.is_empty() {
            self.import_paths_into_folder(doc, folder, elsewhere);
            return Ok(());
        }
        // ⚠ A cut pasted where its rows already sit moves nothing: it stays held and no step is made
        if let Some((from, rows)) = held.as_ref().and_then(|held| held.cut.as_ref()) {
            let in_place = self.rows_to_place(*from, rows, folder, "paste");
            if *from == doc && in_place.is_ok_and(|rows| rows.is_empty()) {
                self.show_toast(
                    tr!("Already in this folder: nothing was pasted, and the cut is still held")
                        .to_string(),
                );
                return Ok(());
            }
        }
        // ⚠ A cut writes to the archive or folder it leaves as well.
        let mut targets = vec![self.affected_disk_path(doc, folder)];
        if let Some((from, rows)) = held.and_then(|held| held.cut) {
            targets.extend(rows.iter().map(|&row| self.affected_disk_path(from, row)));
        }
        targets.sort();
        targets.dedup();
        let mut ask = None;
        for target in targets {
            match self.gate_permits(target.as_deref()) {
                GateOutcome::Refused => return Err(super::retail_pending_message()),
                GateOutcome::Confirm(path) => ask = ask.or(Some(path)),
                GateOutcome::Proceed => {}
            }
        }
        if let Some(target_path) = ask {
            self.pending_retail_edit = Some(PendingRetailEditPrompt {
                action: PendingRetailEdit::PasteRows {
                    doc_id: doc,
                    folder_id: folder,
                },
                target_path,
                verb: tr!("paste into"),
            });
            return Err(super::retail_pending_message());
        }
        self.paste_rows_inner(doc, folder);
        Ok(())
    }

    /// The paste, once the gate is satisfied; a cut's rows that landed leave the clipboard with their sources.
    pub(super) fn paste_rows_inner(&mut self, doc: DocumentId, folder: RpfNodeId) {
        let Some(held) = self.held_rows() else {
            return;
        };
        if let Err(e) = self.open_archive_row(doc, folder) {
            self.show_error_toast(e);
            return;
        }
        let mut left: Vec<(PathBuf, Option<RpfNodeId>)> = Vec::new();
        let mut failures = Vec::new();
        if let Some(d) = self.document_mut(doc) {
            d.history.begin_batch();
        }
        for (at, file) in held.files.iter().enumerate() {
            let source = held
                .cut
                .as_ref()
                .and_then(|(from, rows)| Some((*from, *rows.get(at)?)));
            if let Err(e) = self.paste_one(doc, folder, file, source) {
                left.push((file.clone(), source.map(|(_, row)| row)));
                failures.push(e);
            }
        }
        let landed = held.files.len() - failures.len();
        if let Some(d) = self.document_mut(doc) {
            d.history
                .end_batch(tr!("Paste {landed} item(s)", landed = landed));
        }
        if let Some((from, _)) = held.cut {
            self.panels.get::<Clipboard>().held = (!left.is_empty()).then(|| {
                Held::Rows(Rows {
                    files: left.iter().map(|(file, _)| file.clone()).collect(),
                    cut: Some((from, left.iter().filter_map(|(_, row)| *row).collect())),
                })
            });
        }
        if failures.is_empty() {
            self.show_toast(tr!("Pasted {landed} item(s)", landed = landed));
        } else {
            self.show_error_toast(tr!(
                "Pasted {landed} item(s); {count} failed: {failures}",
                landed = landed,
                count = failures.len(),
                failures = failures.join("; ")
            ));
        }
    }

    /// One copied file into `folder`; a cut's row moves where the tree can move it, and otherwise leaves once its copy has landed, a row on disk coming back from that copy when the paste is undone.
    fn paste_one(
        &mut self,
        doc: DocumentId,
        folder: RpfNodeId,
        file: &Path,
        source: Option<(DocumentId, RpfNodeId)>,
    ) -> Result<(), String> {
        if let Some((from, row)) = source {
            // A move keeps the row's tab and edits, where an import would read a copy.
            let movable =
                self.disk_path_for_node(from, row).is_some() == self.is_disk_folder(doc, folder);
            if from == doc && movable {
                return self.rpf_move_node(doc, row, folder);
            }
        }
        let (mut files, mut failures) = (0, Vec::new());
        self.import_one(doc, folder, file, &mut files, &mut failures);
        if !failures.is_empty() {
            return Err(failures.join("; "));
        }
        let Some((from, row)) = source else {
            return Ok(());
        };
        let stayed = tr!("pasted, but the cut row stayed where it was");
        // ⚠ Walked while the row is still on disk: each file's copy is what an undo writes back.
        let copies = (self.disk_path_for_node(from, row))
            .map(|_| match self.document(doc).map(|d| &d.payload) {
                Some(DocPayload::Rpf(rpf)) => super::rpf_undo::copies(&rpf.session, folder, file),
                _ => None,
            })
            .map(|copies| {
                copies.ok_or_else(|| {
                    tr!(
                        "{stayed}: its copy is not where it was pasted",
                        stayed = stayed
                    )
                })
            })
            .transpose()?;
        let taken =
            (self.rpf_delete_node_inner(from, row)).map_err(|e| format!("{stayed}: {e}"))?;
        // ⚠ Rows of another document's tree: its ids mean other rows in this one.
        let taken = taken.filter(|_| from == doc).map(|rows| (row, rows));
        let other = (from != doc).then_some(from);
        let undone = copies.and_then(|files| super::rpf_undo::cut(files, taken, other));
        if let (Some(change), Some(d)) = (undone, self.document_mut(doc)) {
            d.record(tr!("Cut {file}", file = file.display()), change);
        }
        Ok(())
    }

    /// Whether Paste has files for a folder: rows copied here, or files Explorer copied since.
    pub fn rows_to_paste(&self) -> bool {
        matches!(self.held(), Some(Held::Rows(_))) || clipboard::holds_files_from_elsewhere()
    }

    fn held(&self) -> Option<&Held> {
        self.panels.peek::<Clipboard>()?.held.as_ref()
    }

    fn held_rows(&self) -> Option<Rows> {
        match self.held() {
            Some(Held::Rows(rows)) => Some(rows.clone()),
            _ => None,
        }
    }
}

/// `rows` without the root, a row gone, or a row inside another of them, which goes with it.
///
/// ⚠ Nested among the rows carried only: the root, never carried, would take every row beside it with it.
pub(super) fn top_rows(session: &RpfEditSession, rows: &[RpfNodeId]) -> Vec<RpfNodeId> {
    let carried: Vec<RpfNodeId> = rows
        .iter()
        .copied()
        .filter(|&row| row != session.root_id() && session.node(row).is_some())
        .collect();
    carried
        .iter()
        .copied()
        .filter(|&row| {
            !carried
                .iter()
                .any(|&other| other != row && session.is_descendant(row, other))
        })
        .collect()
}

/// `rows` as the files a paste or an import reads: a row's own on disk, else a copy written under `scratch`.
pub(super) fn files_of_rows(
    session: &RpfEditSession,
    rows: &[RpfNodeId],
    scratch: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut written = 0;
    let files: Result<Vec<PathBuf>, String> = rows
        .iter()
        .map(|&row| file_of(session, row, scratch, &mut written))
        .collect();
    files.inspect_err(|_| {
        let _ = std::fs::remove_dir_all(scratch);
    })
}

/// The file a copied row is: its own on disk, else written out under `scratch`.
fn file_of(
    session: &RpfEditSession,
    row: RpfNodeId,
    scratch: &Path,
    written: &mut u64,
) -> Result<PathBuf, String> {
    let node = session
        .node(row)
        .ok_or(tr!("A copied row is no longer in the tree"))?;
    match &node.backing {
        RpfNodeBacking::DiskFolder { fs_path, .. }
        | RpfNodeBacking::DiskFile {
            fs_path,
            override_bytes: None,
            ..
        } => Ok(fs_path.clone()),
        RpfNodeBacking::Archive {
            source: ArchiveSource::Disk(Some(path)),
            archive_id,
            ..
        } => {
            let edited = archive_id
                .and_then(|id| session.archive(id))
                .is_some_and(|slot| slot.dirty);
            if edited {
                return Err(tr!(
                    "{name} has unsaved edits: save it first, so the copy holds them",
                    name = node.name
                ));
            }
            Ok(path.clone())
        }
        _ => {
            if scratch
                .join(super::extract_job::sanitise(&node.name))
                .exists()
            {
                return Err(tr!(
                    "Two of the copied rows are called {name}",
                    name = node.name
                ));
            }
            std::fs::create_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
            write_out(session, row, scratch, written)
        }
    }
}

/// Write a row inside an archive under `into`: a nested archive as one file, a folder with what it holds.
fn write_out(
    session: &RpfEditSession,
    row: RpfNodeId,
    into: &Path,
    written: &mut u64,
) -> Result<PathBuf, String> {
    let node = session
        .node(row)
        .ok_or(tr!("A copied row is no longer in the tree"))?;
    let path = into.join(super::extract_job::sanitise(&node.name));
    let bytes = if matches!(node.backing, RpfNodeBacking::Archive { .. }) {
        session.nested_archive_bytes(row)?
    } else if node.is_dir() {
        std::fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        for child in session.children_of(row) {
            write_out(session, child, &path, written)?;
        }
        return Ok(path);
    } else {
        session
            .bytes_for(row)
            .ok_or_else(|| tr!("{name} could not be read", name = node.name))?
            .to_vec()
    };
    *written += bytes.len() as u64;
    if *written > DRAG_OUT_LIMIT {
        return Err(tr!(
            "More than {size} is too much to copy at once; use Extract… instead",
            size = or_job::human_bytes(DRAG_OUT_LIMIT)
        ));
    }
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}
