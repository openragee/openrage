//! Save into a copy of the archive: a GTA VI document's edits, as its loose save writes them, put
//! in place of the entry it was read from in a new archive file, as a patched copy whose table is
//! byte-identical where the body fits its entry's chunks, else rebuilt, which the game refuses.

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{BodyForm, GtaViKeys, Rpf9ArchiveWriter};

use crate::state::extract_job::ExtractItem;
use crate::state::gta6_resource_save::EditedSave;
use crate::state::rpf_archive_open::tree_verbs::{
    patched_said, plan_save, with_kept, write_planned, Replaced, Saved,
};
use crate::state::{AppState, DocPayload, Document};

/// How a save into a copy of a document's archive ended: the document, and the copy written and
/// how, or why it was refused.
type Ended = (
    crate::state::document::DocumentId,
    Result<(PathBuf, Saved), String>,
);

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
    /// Whether the active document was read out of a GTA VI archive, so a copy of it can take it.
    pub(crate) fn can_save_into_archive_copy(&self) -> Result<(), &'static str> {
        let doc = self.active_document.and_then(|id| self.document(id));
        match doc.and_then(|d| d.path.as_deref()).and_then(archive_origin) {
            Some(_) => Ok(()),
            None => Err(tr!(
                "Only a document opened from a GTA VI archive is saved into a copy of it"
            )),
        }
    }

    /// The name the dialog offers: the archive's own, marked as an edited copy.
    pub(crate) fn archive_copy_name(&self) -> Option<String> {
        let doc = self.document(self.active_document?)?;
        let (archive, _) = archive_origin(doc.path.as_deref()?)?;
        Some(format!(
            "{}_edited.rpf",
            archive.file_stem()?.to_string_lossy()
        ))
    }

    /// Write the active document's edits into a new copy at `out` of its archive, on a worker.
    pub(crate) fn save_into_archive_copy(&mut self, out: PathBuf) -> bool {
        self.save_into_archive_copy_with(out, super::keys)
    }

    fn save_into_archive_copy_with(&mut self, out: PathBuf, keys: Keys) -> bool {
        let Some(id) = self.active_document else {
            return false;
        };
        // A form edit reaches the text only as the Text panel draws; the save reads the text.
        if let Some(DocPayload::Meta(meta)) = self.document_mut(id).map(|d| &mut d.payload) {
            meta.ensure_text_current();
        }
        let Some(doc) = self.document(id) else {
            return false;
        };
        let origin = doc.path.as_deref().and_then(archive_origin);
        // The row's name as the tree shows it, where the document was opened from a tree's row.
        let name = (doc.path.as_deref())
            .and_then(crate::state::rpf_archive_open::tree_open::shown)
            .and_then(|shown| Some(shown.file_name()?.to_string_lossy().into_owned()));
        let item = archive_item(doc);
        let refused = match (origin, item) {
            (Some((archive, offset)), Ok(item)) => {
                self.say_the_checksum_is_kept(id);
                std::thread::spawn(move || {
                    let said = keys().and_then(|keys| {
                        let body = item.read(BodyForm::Decoded)?;
                        write_copy(&archive, offset, name, body, &out, &keys)
                    });
                    ended().push((id, said.map(|saved| (out, saved))));
                });
                return true;
            }
            (None, _) => {
                tr!("Only a document opened from a GTA VI archive is saved into a copy of it")
                    .to_string()
            }
            (_, Err(why)) => why,
        };
        self.show_error_toast(tr!(
            "Cannot save into a copy of the archive: {reason}",
            reason = refused
        ));
        false
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
            let Saved::Rebuilt {
                tables,
                bodies,
                kept,
            } = saved
            else {
                self.show_toast(patched_said(&file));
                continue;
            };
            let said = match (tables, bodies) {
                (true, _) => tr!(
                    "{file} is a new archive with the edit in its entry's place, which the game refuses because its table's signature is stale.",
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

/// The item the loose save writes of `doc`'s edits, refused where it writes none.
fn archive_item(doc: &Document) -> Result<ExtractItem, String> {
    let name = String::new();
    match &doc.payload {
        DocPayload::Meta(meta) if meta.is_edited() && meta.headerless_sections().is_none() => {
            return crate::state::gta6_meta_save::meta_bytes(meta, || {
                crate::state::rpf9_scan::rpf9_table::keys().ok()
            })
            .map(|bytes| ExtractItem::bytes(name, bytes));
        }
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
