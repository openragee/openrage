//! A GTA VI resource document's edits saved as a loose resource, which a read at offset 0 opens
//! again: Save As from a scanned body, Save over a loose resource file.
//!
//! A kind writes its edits by returning them from its capability row's `edited_sections`: the
//! sections it holds, or an edit a worker makes to the body read again; a document whose row has
//! none saves as it did, a copy of what was read.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::Rpf9Sections;

use super::capabilities::EditedSectionsFn;
use super::extract_job::{ExtractItem, OnClash};
use super::gta6_edit::EditSections;
use super::{AppState, DocumentId, GateOutcome, PendingRetailEdit, PendingRetailEditPrompt};

#[path = "gta6_awc_in_extent.rs"]
mod awc_in_extent;

/// What a save writes of a GTA VI resource document's edits.
pub enum EditedSave {
    /// Sections the document holds.
    Sections(Rpf9Sections),
    /// The body at `offset` of `archive` read again on the worker, `edit` made to its sections.
    Body {
        archive: PathBuf,
        offset: u64,
        edit: Arc<EditSections>,
    },
    /// A file's bytes, which its kind's own writer makes on the worker.
    Written(Arc<super::gta6_other_bodies_save::Write>),
    /// A whole file the document wrote, such as an audio container with a stream replaced.
    Bytes(Vec<u8>),
}

/// The audio container row's `edited_sections`: the container laid in its extents, else rewritten.
pub(super) fn edited_awc(doc: &super::Document) -> Option<Result<EditedSave, String>> {
    awc_in_extent::edited(doc)
}

/// A row's `edited_sections` of the sections a payload holds edited.
pub(super) fn held(
    sections: Option<Result<Rpf9Sections, String>>,
) -> Option<Result<EditedSave, String>> {
    sections.map(|s| s.map(EditedSave::Sections))
}

/// The GTA VI drawable row's `edited_sections`: its materials, physics, bones and embedded
/// textures edited in place, then its geometry replaced from glTF.
#[cfg(not(test))]
pub(super) const DRAWABLE: Option<EditedSectionsFn> = Some(super::gta6_gltf_import::edited_save);
#[cfg(test)]
pub(super) const DRAWABLE: Option<EditedSectionsFn> = Some(tests::held);

impl AppState {
    /// What a save writes of document `doc`'s edits, from its row; `None` while it holds none.
    pub(super) fn edited_sections(&self, doc: DocumentId) -> Option<Result<EditedSave, String>> {
        let doc = self.document(doc)?;
        (super::capabilities::for_payload(&doc.payload)?.edited_sections?)(doc)
    }

    /// `name` in `dest`, the edits of `doc` written as a loose resource on a worker; `None` while
    /// it holds none, and `Some(false)` where they cannot be written.
    pub(super) fn save_edited_sections(
        &mut self,
        doc: DocumentId,
        dest: PathBuf,
        name: String,
        reads: PathBuf,
    ) -> Option<bool> {
        let edited = self.edited_sections(doc)?;
        // The neighbouring bodies whose links an edit of a scanned cell renumbered.
        let beside =
            (self.document(doc)).map_or(Vec::new(), super::gta6_navmesh_delete::links::beside);
        Some(self.start_edited_save(edited, dest, name, reads, beside))
    }

    fn start_edited_save(
        &mut self,
        edited: Result<EditedSave, String>,
        dest: PathBuf,
        name: String,
        reads: PathBuf,
        beside: Vec<ExtractItem>,
    ) -> bool {
        let item = match edited {
            Ok(EditedSave::Sections(sections)) => ExtractItem::sections(name.clone(), sections),
            Ok(EditedSave::Body {
                archive,
                offset,
                edit,
            }) => ExtractItem::edited(name.clone(), archive, offset, edit),
            Ok(EditedSave::Written(write)) => ExtractItem::written(name.clone(), write),
            Ok(EditedSave::Bytes(bytes)) => ExtractItem::bytes(name.clone(), bytes),
            Err(e) => {
                self.show_error_toast(tr!("Cannot save: {reason}", reason = e));
                return false;
            }
        };
        let items = std::iter::once(item).chain(beside).collect();
        self.start_scanned_extraction(name, dest, reads, items, OnClash::Save)
    }

    /// Save a copy of the scanned body `doc` shows to `path`: a metadata body as its XML where
    /// `path` ends in `.xml`, any other as a file that opens again; `None` for any other document.
    pub(super) fn save_scanned_body_as(&mut self, doc: DocumentId, path: &Path) -> Option<bool> {
        let id = doc;
        let xml = (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("xml"));
        let over_archive = (self.document(id).and_then(|d| d.path.as_deref()))
            .and_then(super::rpf9_scan::scanned_body)
            .is_some_and(|(archive, _)| super::reload_open_file::same_file(path, archive));
        // An XML export holds no checksum, and a save over the archive is refused below.
        if !xml && !over_archive {
            self.say_the_checksum_is_kept(id);
        }
        // A form edit reaches the text only as the Text panel draws; an XML save reads the text,
        // and any other makes it current on its worker.
        if let Some(doc) = self.document_mut(id).filter(|_| xml) {
            if let super::DocPayload::Meta(meta) = &mut doc.payload {
                meta.ensure_text_current();
            }
        }
        let doc = self.document(doc)?;
        let (archive, offset) = super::rpf9_scan::scanned_body(doc.path.as_deref()?)?;
        let archive = archive.to_path_buf();
        let name = path.file_name()?.to_string_lossy().into_owned();
        let gxt2 = (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("gxt2"));
        // A body shown as the lines its reader reads is never written as those lines.
        let edited_text = doc.dirty
            && matches!(doc.payload, super::DocPayload::Text(_))
            && (doc.path.as_deref()).is_some_and(super::rpf9_scan::opened_as_text);
        let item = match &doc.payload {
            super::DocPayload::Meta(meta) if xml => {
                // A GTA VI form's name comments are display-only; the file is the comment-free XML.
                let bytes = match meta.comment_free_xml() {
                    Some(xml) => xml.into_bytes(),
                    None => meta.text.encode(),
                };
                Ok(ExtractItem::bytes(name.clone(), bytes))
            }
            // An edited text body is written in the encoding and line endings it was read with.
            super::DocPayload::Text(text) if edited_text => {
                Ok(ExtractItem::bytes(name.clone(), text.encode()))
            }
            // A text table exports as GTA V's text table, as a GTA V one saves.
            super::DocPayload::Gta6Text(text) if gxt2 => {
                (text.to_gxt2()).map(|bytes| ExtractItem::bytes(name.clone(), bytes))
            }
            _ => Ok(ExtractItem::scanned(name.clone(), archive.clone(), offset)),
        };
        let item = match item {
            Ok(item) => item,
            Err(e) => {
                self.show_error_toast(tr!("Cannot save: {reason}", reason = e));
                return Some(false);
            }
        };
        let dest = path.parent()?.to_path_buf();
        if super::reload_open_file::same_file(path, &archive) {
            self.show_error_toast(tr!(
                "A body is saved as a file of its own, never over the archive it is read from."
            ));
            return Some(false);
        }
        if !xml && !gxt2 && !edited_text {
            if let Some(started) =
                self.save_edited_meta(id, dest.clone(), name.clone(), archive.clone())
            {
                return Some(started);
            }
            if let Some(started) =
                self.save_edited_sections(id, dest.clone(), name.clone(), archive.clone())
            {
                return Some(started);
            }
            if self.refuse_untaken_edits(id) {
                return Some(false);
            }
        }
        Some(self.start_scanned_extraction(name, dest, archive, vec![item], OnClash::Save))
    }

    /// Whether `doc` holds edits no writer took, said in a toast: a copy of the stored body would
    /// drop them without a word.
    pub(super) fn refuse_untaken_edits(&mut self, doc: DocumentId) -> bool {
        if !self.document(doc).is_some_and(|d| d.dirty) {
            return false;
        }
        self.show_error_toast(tr!(
            "Cannot save: OpenRage writes none of this view's edits to a GTA VI body, so Save As would drop them. Undo them to save the body as it is stored."
        ));
        true
    }

    /// Save of a loose resource file whose document holds edited sections, over that file once the
    /// write gate permits it; `None` for any other document, which saves or refuses as it did.
    pub(super) fn save_edited_resource(&mut self, doc: DocumentId) -> Option<bool> {
        if self.document(doc)?.read_only.is_none() {
            self.say_the_checksum_is_kept(doc);
        }
        let path = self.loose_resource_file(doc)?;
        let edited = self.edited_sections(doc)?;
        match self.gate_permits(Some(&path)) {
            GateOutcome::Refused => Some(false),
            GateOutcome::Confirm(target_path) => {
                self.pending_retail_edit = Some(PendingRetailEditPrompt {
                    action: PendingRetailEdit::SaveLooseFile { doc_id: doc },
                    target_path,
                    verb: "save",
                });
                Some(true)
            }
            GateOutcome::Proceed => Some(self.write_edited_resource(doc, path, edited)),
        }
    }

    /// The save [`save_edited_resource`](Self::save_edited_resource) starts once the gate let it
    /// through; `None` for any other document.
    pub(super) fn save_edited_resource_inner(&mut self, doc: DocumentId) -> Option<bool> {
        let path = self.loose_resource_file(doc)?;
        let edited = self.edited_sections(doc)?;
        Some(self.write_edited_resource(doc, path, edited))
    }

    /// The file of its own document `doc` was read from at offset 0.
    fn loose_resource_file(&self, doc: DocumentId) -> Option<PathBuf> {
        let path = self.document(doc)?.path.clone()?;
        // A body read out of an archive is never saved into it: only a file of its own is.
        let (file, 0) = super::rpf9_scan::scanned_body(&path)? else {
            return None;
        };
        (file == path.as_path()).then_some(path)
    }

    fn write_edited_resource(
        &mut self,
        doc: DocumentId,
        path: PathBuf,
        edited: Result<EditedSave, String>,
    ) -> bool {
        let (Some(dest), Some(name)) = (path.parent(), path.file_name()) else {
            return false;
        };
        let (dest, name) = (dest.to_path_buf(), name.to_string_lossy().into_owned());
        let Some(at) = self.document(doc).map(|d| d.history.at()) else {
            return false;
        };
        let started = self.start_edited_save(edited, dest, name, path.clone(), Vec::new());
        if started {
            self.extract_jobs.saving(doc, at);
        }
        started
    }
}

#[cfg(test)]
#[path = "saving_an_edited_gta_vi_text_body.rs"]
mod saving_an_edited_gta_vi_text_body;

#[cfg(test)]
pub(super) mod tests {
    use std::cell::RefCell;
    use std::io::Cursor;
    use std::time::{Duration, Instant};

    use or_io::container::rpf::{read_rpf9_body, Rpf9Form, Rpf9Sections};

    use super::EditedSave;
    use crate::state::opening_an_rpf9_archive::{
        navmesh_cell, row_at, rows, rpf9_head, said, scanned, settle, written, zlib,
    };
    use crate::state::{AppState, DocPayload, Document, DocumentKind, ToastType};

    thread_local! {
        /// The sections a test's GTA VI drawable document holds edited.
        static HELD: RefCell<Option<Rpf9Sections>> = const { RefCell::new(None) };
    }

    /// The sections a test holds, else the drawable's materials edited in place.
    pub(in crate::state) fn held(doc: &Document) -> Option<Result<EditedSave, String>> {
        let DocPayload::Gta6Drawable(_) = doc.payload else {
            return None;
        };
        let sections = HELD.with(|held| held.borrow().clone());
        let edited = || super::super::gta6_gltf_import::edited_save(doc);
        super::held(sections.map(Ok)).or_else(edited)
    }

    fn hold(sections: Option<Rpf9Sections>) {
        HELD.with(|held| *held.borrow_mut() = sections);
    }

    fn finished(state: &mut AppState) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.extract_jobs.is_running() {
            state.poll_extract_job();
            assert!(Instant::now() < deadline, "the save never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The edited sections a test holds: a navigation cell's, so a file of them opens as one.
    fn edited() -> Rpf9Sections {
        Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: navmesh_cell(),
            physical: Vec::new(),
        }
    }

    /// The sections a read at offset 0 of the file at `path` returns.
    fn read_back(path: &std::path::Path) -> Rpf9Sections {
        let bytes = std::fs::read(path).expect("the file");
        let (find, read) = read_rpf9_body(Cursor::new(bytes), 0, u64::MAX).expect("it reads");
        Rpf9Sections::of(&find, &read).expect("a resource")
    }

    /// The document opened from `path`, other than `not`.
    fn opened(
        state: &mut AppState,
        path: std::path::PathBuf,
        not: crate::state::DocumentId,
    ) -> crate::state::DocumentId {
        state.load_file(path);
        settle(state);
        (state.active_document)
            .filter(|&d| d != not)
            .unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message))
    }

    #[test]
    fn edited_sections_save_as_a_loose_resource_from_a_scanned_body_and_over_a_loose_file() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1010, 0);
        bytes.extend(zlib(&crate::state::opening_a_gta_vi_body::bound_body()));
        let mut state = AppState::new();
        let doc = scanned(&mut state, written(dir.path(), &bytes));
        state.open_scanned_body(doc, row_at(&rows(&state, doc), 0x1010));
        settle(&mut state);
        let body = (state.active_document)
            .filter(|&d| d != doc)
            .unwrap_or_else(|| panic!("the body: {}", state.error_dialog_message));
        assert_eq!(
            state.document(body).map(|d| d.kind),
            Some(DocumentKind::Gta6Drawable)
        );

        // Nothing edited: Save As writes the copy it did, and Save refuses a body of an archive.
        hold(None);
        let copy = dir.path().join("copy.rsc");
        assert!(state.save_active_document_as(copy.clone()));
        finished(&mut state);
        assert_ne!(read_back(&copy), edited());
        hold(Some(edited()));
        state.toasts.clear();
        assert!(!state.save_active_document());
        assert!(said(&state, ToastType::Warning, &["read-only"]));

        // Edited: Save As writes the sections, which open again through the reader of their kind.
        let saved = dir.path().join("saved.rsc");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        let mut expected = edited();
        expected
            .virtual_section
            .resize(expected.virtual_section.len().next_multiple_of(16), 0);
        assert_eq!(read_back(&saved), expected);
        let cell = opened(&mut state, saved, doc);
        assert_eq!(
            state.document(cell).map(|d| d.kind),
            Some(DocumentKind::Ynv)
        );

        // Save over a loose resource file the copy opened from.
        hold(None);
        let loose = opened(&mut state, copy.clone(), doc);
        assert!(!state.save_active_document(), "nothing edited");
        hold(Some(edited()));
        state.active_document = Some(loose);
        // Refused while a defragment holds the file, as every Save is.
        let before = std::fs::read(&copy).expect("the copy");
        state
            .defrag
            .start(copy.clone(), "a defragment".into(), |_, cancel| {
                while !cancel.is_set() {
                    std::thread::sleep(Duration::from_millis(2));
                }
                crate::state::defrag_job::Outcome::Stopped
            });
        state.toasts.clear();
        assert!(!state.save_active_document());
        assert!(said(&state, ToastType::Error, &["being defragmented"]));
        state.defrag.cancel();
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.defrag.is_running() {
            state.poll_defrag();
            assert!(Instant::now() < deadline, "the defragment never stopped");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read(&copy).expect("the copy"), before);
        assert!(state.save_active_document());
        finished(&mut state);
        assert_eq!(read_back(&copy), expected);
        // The file it replaced is kept beside it, as every Save keeps one.
        let backup = std::fs::read(copy.with_extension("rsc.bak")).expect("a backup");
        assert_eq!(backup, before);
        hold(None);
    }
}
