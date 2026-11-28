//! Save into a copy of the archive: a GTA VI document's edits, as its loose save writes them, put
//! in place of the entry it was read from in a new archive file, as a patched copy whose table is
//! byte-identical where the body fits its entry's chunks, else rebuilt, which the game refuses.

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{BodyForm, GtaViKeys, Rpf9ArchiveWriter, Rpf9Place};

use crate::platform::file_dialogs;
use crate::state::document::DocumentId;
use crate::state::extract_job::ExtractItem;
use crate::state::gta6_resource_save::EditedSave;
use crate::state::rpf_archive_open::tree_verbs::{
    as_read_said, patched_said, plan_save, with_kept, write_planned, Replaced, Saved,
};
use crate::state::{AppState, DocPayload, Document, RpfNodeBacking};

/// How a save into a copy of a document's archive ended: the document, and the copy written and
/// how, or why it was refused.
type Ended = (DocumentId, Result<(PathBuf, Saved), String>);

/// What makes the body a save into a copy writes, on the save's worker.
type Body = Box<dyn FnOnce() -> Result<Vec<u8>, String> + Send>;

/// A save into a copy checked before its file is picked: the document, the archive copied, the
/// body's offset there, the row's name and what makes the body.
struct Pending {
    id: DocumentId,
    archive: PathBuf,
    offset: u64,
    name: Option<String>,
    body: Body,
}

// ponytail: one list for the process; a poll takes the saves of the documents its app holds, so a
// save of a document closed before it ended stays in the list unsaid.
static ENDED: Mutex<Vec<Ended>> = Mutex::new(Vec::new());

fn ended() -> MutexGuard<'static, Vec<Ended>> {
    ENDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Where a save finds the keys the archive's tables open with.
type Keys = fn() -> Result<Arc<GtaViKeys>, String>;

/// The archive a document at `path` was read out of and the body's offset there; `None` for a
/// document of a file of its own.
fn archive_origin(path: &Path) -> Option<(PathBuf, u64)> {
    let (archive, offset) = crate::state::rpf9_scan::scanned_body(path)?;
    (archive != path).then(|| (archive.to_path_buf(), offset))
}

impl AppState {
    /// Where the body at `path` sits in the archive its open tree's row reads, which a Save as…
    /// made the copy, and the row's path in the tree; `None` for a body no open tree's row holds.
    fn tree_row_place(&self, path: &Path) -> Option<(Rpf9Place, String)> {
        let row = crate::state::rpf_archive_open::tree_open::row_of(self, path)?;
        let DocPayload::Rpf(rpf) = &self.document(row.parent_doc_id)?.payload else {
            return None;
        };
        let RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            ..
        } = &rpf.session.node(row.entry_id)?.backing
        else {
            return None;
        };
        let place = rpf
            .session
            .archive(*archive_id)?
            .archive
            .rpf9_place(*entry_index)?;
        // The row's path inside the file on disk: the tree's path is from its root, which may be a
        // folder holding the archive.
        let shown = crate::state::rpf_archive_open::tree_open::shown(path);
        let inner = (shown
            .as_deref()
            .and_then(|s| s.strip_prefix(&place.file).ok()))
        .map(|p| {
            (p.iter().map(|c| c.to_string_lossy()))
                .collect::<Vec<_>>()
                .join("/")
        })
        .or_else(|| rpf.session.path_of(row.entry_id))?;
        Some((place, inner))
    }

    /// The archive a save into a copy of the body at `path` copies and the body's offset there:
    /// its tree row's archive, else the archive it was read out of.
    fn copy_origin(&self, path: &Path) -> Option<(PathBuf, u64)> {
        match self.tree_row_place(path) {
            Some((place, _)) => Some((place.file, place.offset)),
            None => archive_origin(path),
        }
    }

    /// Whether the active document was read out of a GTA VI archive, so a copy of it can take it.
    pub(crate) fn can_save_into_archive_copy(&self) -> Result<(), &'static str> {
        let doc = self.active_document.and_then(|id| self.document(id));
        match doc
            .and_then(|d| d.path.as_deref())
            .and_then(|p| self.copy_origin(p))
        {
            Some(_) => Ok(()),
            None => Err(tr!(
                "Only a document opened from a GTA VI archive is saved into a copy of it"
            )),
        }
    }

    /// The name the dialog offers: the archive's own, marked as an edited copy.
    pub(crate) fn archive_copy_name(&self) -> Option<String> {
        let doc = self.document(self.active_document?)?;
        let (archive, _) = self.copy_origin(doc.path.as_deref()?)?;
        Some(format!(
            "{}_edited.rpf",
            archive.file_stem()?.to_string_lossy()
        ))
    }

    /// Ask where the copy of the active document's archive goes, then write its edits there on a
    /// worker; the toast after says which copy it is.
    pub(crate) fn ask_and_save_into_archive_copy(&mut self) -> bool {
        self.ask_and_save_with(super::keys)
    }

    fn ask_and_save_with(&mut self, keys: Keys) -> bool {
        let Some(name) = self.archive_copy_name() else {
            return false;
        };
        // Nothing to save is said before a dialog asks where a copy goes.
        let pending = match self.pending_copy() {
            Ok(pending) => pending,
            Err(why) => return self.refused_copy(why),
        };
        // Which copy is written is planned once the file is picked, so the title names neither.
        let Some(out) = file_dialogs::save_gta6_archive_copy(&name, tr!("Save a copy")) else {
            return false;
        };
        self.write_copy_on_worker(pending, out, keys)
    }

    #[cfg(test)]
    pub(in crate::state) fn save_into_archive_copy_with(
        &mut self,
        out: PathBuf,
        keys: Keys,
    ) -> bool {
        match self.pending_copy() {
            Ok(pending) => self.write_copy_on_worker(pending, out, keys),
            Err(why) => self.refused_copy(why),
        }
    }

    /// The active document's save into a copy of its archive, its body still to be made; refused
    /// where it is no archive's or nothing in it is to be saved.
    fn pending_copy(&mut self) -> Result<Pending, String> {
        let refused =
            || tr!("Only a document opened from a GTA VI archive is saved into a copy of it");
        let id = self.active_document.ok_or_else(|| refused().to_string())?;
        let doc = self.document(id).ok_or_else(|| refused().to_string())?;
        let (archive, offset) = (doc.path.as_deref())
            .and_then(|p| self.copy_origin(p))
            .ok_or_else(|| refused().to_string())?;
        // The row's name as the tree shows it, where the document was opened from a tree's row.
        let name = (doc.path.as_deref())
            .and_then(crate::state::rpf_archive_open::tree_open::shown)
            .and_then(|shown| Some(shown.file_name()?.to_string_lossy().into_owned()));
        Ok(Pending {
            id,
            archive,
            offset,
            name,
            body: archive_body(doc)?,
        })
    }

    fn write_copy_on_worker(&mut self, pending: Pending, out: PathBuf, keys: Keys) -> bool {
        let Pending {
            id,
            archive,
            offset,
            name,
            body,
        } = pending;
        self.say_the_checksum_is_kept(id);
        std::thread::spawn(move || {
            let said = keys().and_then(|keys| {
                let body = body()?;
                write_copy(&archive, offset, name, body, &out, &keys)
            });
            ended().push((id, said.map(|saved| (out, saved))));
        });
        true
    }

    fn refused_copy(&mut self, why: String) -> bool {
        self.show_error_toast(tr!(
            "Cannot save into a copy of the archive: {reason}",
            reason = why
        ));
        false
    }

    /// The archive the document `doc_id` was opened from a GTA VI tree's row of, and the row's
    /// path there; `None` for a document opened any other way.
    pub(crate) fn tree_row_origin(&self, doc_id: DocumentId) -> Option<(PathBuf, String)> {
        let path = self.document(doc_id)?.path.as_deref()?;
        // The tree's own archive, which a Save as… made the copy, while the tree is open.
        if let Some((place, inner)) = self.tree_row_place(path) {
            return Some((place.file, inner));
        }
        let shown = crate::state::rpf_archive_open::tree_open::shown(path)?;
        let (archive, _) = crate::state::rpf9_scan::scanned_body(path)?;
        let inner = match shown.strip_prefix(archive) {
            Ok(inner) => (inner.iter().map(|p| p.to_string_lossy()))
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => shown.display().to_string(),
        };
        Some((archive.to_path_buf(), inner))
    }

    /// Say how each save into a copy of an archive ended.
    pub(in crate::state) fn poll_archive_saves(&mut self) {
        self.poll_tree_saves();
        let done: Vec<Ended> = {
            let mut list = ended();
            let (mine, rest) = std::mem::take(&mut *list)
                .into_iter()
                .partition(|(id, _)| self.document(*id).is_some());
            *list = rest;
            mine
        };
        for (_, said) in done {
            let (out, saved) = match said {
                Ok(written) => written,
                Err(why) => {
                    self.show_error_toast(tr!(
                        "Cannot save into a copy of the archive: {reason}",
                        reason = why
                    ));
                    continue;
                }
            };
            let file = crate::state::jobs::file_name(&out);
            let (tables, bodies, kept) = match saved {
                Saved::AsRead => {
                    self.show_toast(as_read_said(&file));
                    continue;
                }
                Saved::Patched => {
                    self.show_toast(patched_said(&file));
                    continue;
                }
                Saved::Rebuilt {
                    tables,
                    bodies,
                    kept,
                } => (tables, bodies, kept),
            };
            let said = match (tables, bodies) {
                (true, _) => tr!(
                    "{file} is a rebuilt archive with the document's edit, which the game refuses because only its maker can sign the table it changed.",
                    file = file
                ),
                (false, true) => tr!(
                    "{file} is a new archive with the edit in its entry's place and its table as it was read.",
                    file = file
                ),
                (false, false) => tr!(
                    "{file} is a new archive the same as the one it was read from: the edit writes the bytes the entry holds.",
                    file = file
                ),
            };
            self.show_toast(with_kept(said, &kept));
        }
    }
}

/// What the loose save writes of `doc`'s edits, made on the save's worker; refused where it
/// writes none.
fn archive_body(doc: &Document) -> Result<Body, String> {
    if let DocPayload::Meta(meta) = &doc.payload {
        if meta.is_edited() && !meta.is_headerless() {
            // A metadata body's text is parsed and written on the worker, never on the UI thread;
            // a form edit reaches the text there too.
            let mut meta = meta.clone();
            return Ok(Box::new(move || {
                meta.ensure_text_current();
                let keys = || crate::state::rpf9_scan::rpf9_table::keys().ok();
                crate::state::gta6_meta_save::meta_bytes(&meta, keys)
            }));
        }
    }
    let item = archive_item(doc)?;
    Ok(Box::new(move || item.read(BodyForm::Decoded)))
}

/// The item the loose save writes of `doc`'s edits other than a metadata body's, refused where it
/// writes none.
fn archive_item(doc: &Document) -> Result<ExtractItem, String> {
    let name = String::new();
    match &doc.payload {
        // An edited text body is written in the encoding and line endings it was read with.
        DocPayload::Text(text)
            if doc.dirty
                && (doc.path.as_deref()).is_some_and(crate::state::rpf9_scan::opened_as_text) =>
        {
            return Ok(ExtractItem::bytes(name, text.encode()));
        }
        _ => {}
    }
    let edited = crate::state::capabilities::for_payload(&doc.payload)
        .and_then(|row| row.edited_sections)
        .and_then(|edited| edited(doc));
    match edited {
        Some(Ok(EditedSave::Sections(sections))) => Ok(ExtractItem::sections(name, sections)),
        Some(Ok(EditedSave::Body {
            archive,
            offset,
            edit,
        })) => Ok(ExtractItem::edited(name, archive, offset, edit)),
        Some(Ok(EditedSave::Written(write))) => Ok(ExtractItem::written(name, write)),
        Some(Ok(EditedSave::Bytes(bytes))) => Ok(ExtractItem::bytes(name, bytes)),
        Some(Err(why)) => Err(why),
        None => Err(tr!(
            "nothing this view holds is edited, or OpenRage writes none of its edits to a GTA VI body"
        )
        .to_string()),
    }
}

/// Write `body` in place of the entry whose body is at `offset` of the archive at `archive`, into
/// the new file `out`, as a patched copy where it fits the entry's chunks, else rebuilt, planned
/// before anything is written; the row is named `name`, else by its offset. A headerless resource's entry takes `body` as its loose form, any
/// other its bytes, and the writer refuses what it would not read back.
fn write_copy(
    archive: &Path,
    offset: u64,
    name: Option<String>,
    body: Vec<u8>,
    out: &Path,
    keys: &GtaViKeys,
) -> Result<Saved, String> {
    let mut file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let wanted = BTreeSet::from([offset]);
    let mut found = crate::state::rpf_archive_open::tree_verbs::chains(&mut file, &wanted, keys)?;
    drop(file);
    let at = found.remove(&offset).unwrap_or_default();
    let mut writer = Rpf9ArchiveWriter::new();
    if at.headerless {
        writer.replace_resource(&at.chain, body.clone(), None);
    } else {
        writer.replace(&at.chain, body.clone());
    }
    let row = Replaced {
        name: name.unwrap_or_else(|| format!("{offset:#X}")),
        at,
        body,
    };
    let planned = plan_save(archive, keys, &[row], Vec::new(), writer)?;
    write_planned(archive, out, keys, &planned)
}

#[cfg(test)]
#[path = "gta6_archive_save_tests.rs"]
mod tests;
