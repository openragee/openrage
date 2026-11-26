//! A document opened from a GTA VI tree's row saves into that row, as a GTA V entry's document
//! saves into its archive: the row takes the body its kind stores, and only the tree's Save as…
//! writes a file.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use or_io::container::rpf::BodyForm;

use crate::state::document::DocumentId;
use crate::state::extract_job::ExtractItem;
use crate::state::gta6_resource_save::EditedSave;
use crate::state::{AppState, DocPayload, Document, RpfEntryBinding};

/// A save into a row that ended on its worker: the document and its path, the edit it was at, the
/// row, and the row's new body or why there is none.
type Ended = (
    DocumentId,
    PathBuf,
    u64,
    RpfEntryBinding,
    Result<Vec<u8>, String>,
);

/// What makes a row's new body on the save's worker.
type Body = Box<dyn FnOnce() -> Result<Vec<u8>, String> + Send>;

// ponytail: one list for the process, taken by the next poll of the app holding the document; two
// saves of one document in flight land in the order they end, add a per-document epoch if that bites.
static ENDED: Mutex<Vec<Ended>> = Mutex::new(Vec::new());

fn ended() -> MutexGuard<'static, Vec<Ended>> {
    ENDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What `doc`'s edits store in its row: a headerless resource's loose form, as the tree holds such a
/// row, any other row's body, made on the worker; refused where the view writes none.
fn row_body(doc: &Document) -> Result<Body, String> {
    if let DocPayload::Meta(meta) = &doc.payload {
        if meta.is_edited() && !meta.is_headerless() {
            // A metadata body's text is parsed and written on the worker, never on the UI thread;
            // a form edit reaches the text there too, so the row holds what the Text view shows.
            let mut meta = meta.clone();
            return Ok(Box::new(move || {
                meta.ensure_text_current();
                let keys = || crate::state::rpf9_scan::rpf9_table::keys().ok();
                crate::state::gta6_meta_save::meta_bytes(&meta, keys)
            }));
        }
    }
    let item = row_item(doc)?;
    Ok(Box::new(move || item.read(BodyForm::Decoded)))
}

fn row_item(doc: &Document) -> Result<ExtractItem, String> {
    let name = String::new();
    match &doc.payload {
        // An edited text body is stored in the encoding and line endings it was read with.
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

impl AppState {
    /// Save `doc_id`, opened from a GTA VI tree's row, into that row on a worker; `None` for any other document.
    pub(in crate::state) fn save_into_tree_row(&mut self, doc_id: DocumentId) -> Option<bool> {
        let path = self.document(doc_id)?.path.clone()?;
        let row = super::row_of(self, &path)?;
        // Only a GTA VI archive opened on its own writes its rows to a new file; a folder's tree
        // leaves its rows to the read-only refusal, which says to open the archive on its own.
        let DocPayload::Rpf(rpf) = &self.document(row.parent_doc_id)?.payload else {
            return None;
        };
        super::super::tree_verbs::refused_tree(&rpf.session)?;
        let doc = self.document(doc_id)?;
        let at = doc.history.at();
        match row_body(doc) {
            Ok(body) => {
                self.say_the_checksum_is_kept(doc_id);
                std::thread::spawn(move || {
                    let body = body();
                    ended().push((doc_id, path, at, row, body));
                });
                Some(true)
            }
            Err(why) => {
                self.show_error_toast(tr!("Not saved into its row: {reason}", reason = why));
                Some(false)
            }
        }
    }

    /// Put each body a save into a row made in that row of its tree, as a GTA V entry's save does,
    /// and bind the document to the row.
    pub(in crate::state) fn poll_tree_row_saves(&mut self) {
        let done: Vec<Ended> = {
            let mut list = ended();
            let (mine, rest) =
                std::mem::take(&mut *list)
                    .into_iter()
                    .partition(|(id, path, _, row, _)| {
                        // Ids repeat across apps of one process: the document must be this app's.
                        let held = self
                            .document(*id)
                            .is_some_and(|d| d.path.as_ref() == Some(path));
                        held && self.document(row.parent_doc_id).is_some()
                    });
            *list = rest;
            mine
        };
        for (doc_id, _, at, row, body) in done {
            let put = body.and_then(|body| {
                self.update_rpf_entry_bytes(row.parent_doc_id, row.entry_id, body)
            });
            if let Err(why) = put {
                self.show_error_toast(tr!("Not saved into its row: {reason}", reason = why));
                continue;
            }
            let archive = (self.document(row.parent_doc_id))
                .and_then(|d| d.path.as_deref())
                .map(crate::state::jobs::file_name)
                .unwrap_or_default();
            let Some(doc) = self.document_mut(doc_id) else {
                continue;
            };
            // Bound as a GTA V entry's document is, the row opens it again and a close of the tree closes it.
            doc.rpf_entry_binding = Some(row);
            doc.saved_as_of(at);
            let name = doc.file_name();
            if let Some(DocPayload::Rpf(rpf)) =
                self.document_mut(row.parent_doc_id).map(|d| &mut d.payload)
            {
                rpf.entry_documents.insert(row.entry_id, doc_id);
            }
            self.show_toast(tr!(
                "{name} is saved into its row of {archive}, and the archive's Save as… writes it to a new file: a patched copy where the body fits its entry's chunks, else a rebuilt archive, which the game refuses because only its maker can sign the table.",
                name = name,
                archive = archive
            ));
        }
    }
}

#[cfg(test)]
#[path = "rpf9_tree_bind_tests.rs"]
mod tests;
