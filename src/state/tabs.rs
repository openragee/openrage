//! The tab strip's bookkeeping: the preview tab a single click in Files opens,
//! the order tabs were used in for `Ctrl+Tab`, and the closed tabs
//! `Ctrl+Shift+T` reopens.

use std::path::{Path, PathBuf};

use super::explorer::Place;
use super::navigation::{Arrival, Location};
use super::{
    AfterClose, AppState, DocPayload, Document, DocumentId, DocumentKind, RecentFile,
    RpfEntryBinding, RpfNodeBacking, RpfNodeId,
};
use crate::ui::dock::Workspace;

/// How many closed tabs `Ctrl+Shift+T` can reopen; the oldest is forgotten first.
const CLOSED_KEPT: usize = 32;

#[derive(Debug, Default)]
pub struct Tabs {
    /// The archive row open in the preview tab, which the next single click in Files replaces.
    preview: Option<RpfEntryBinding>,
    /// The archive row a thread is loading.
    pub(super) arriving: Option<RpfEntryBinding>,
    pub(super) queued_rpf9: std::collections::VecDeque<(PathBuf, RpfEntryBinding)>,
    /// A preview replaced while it was arriving, dropped once it arrives.
    abandoned: Option<RpfEntryBinding>,
    /// The body of a scan's list open in the preview tab, by the path the scan names it under.
    scan_preview: Option<PathBuf>,
    /// A scanned body's preview replaced while it was arriving, dropped once it arrives.
    scan_abandoned: Option<PathBuf>,
    /// Every tab, the most recently active first.
    used: Vec<DocumentId>,
    /// How far back in `used` `Ctrl+Tab` has stepped while Ctrl is held.
    cycling: Option<usize>,
    /// Where each closed tab's file is, the one closed last at the end.
    closed: Vec<Place>,
}

impl Tabs {
    /// An open that keeps `row`'s tab: it stops being the preview, and is no longer dropped when it arrives.
    pub(super) fn keep(&mut self, row: RpfEntryBinding) {
        if self.preview == Some(row) {
            self.preview = None;
        }
        if self.abandoned == Some(row) {
            self.abandoned = None;
        }
    }

    /// Whether the load that arrived for `row`, or under `path`, is a replaced preview, to drop; nothing is arriving after it.
    pub(super) fn arrived(&mut self, row: Option<RpfEntryBinding>, path: &Path) -> bool {
        self.arriving = None;
        let replaced = (row.is_some() && self.abandoned == row)
            || self.scan_abandoned.as_deref() == Some(path);
        self.abandoned = None;
        self.scan_abandoned = None;
        replaced
    }

    pub fn can_reopen(&self) -> bool {
        !self.closed.is_empty()
    }
}

/// What holds a file a scan named by its offset (`archive/0x<offset>.<ext>`): the same offset is a
/// body of every archive, so the name alone tells no two of their tabs apart.
fn scan_holder(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    u64::from_str_radix(stem.strip_prefix("0x")?, 16).ok()?;
    Some(path.parent()?.file_name()?.to_string_lossy().into_owned())
}

impl AppState {
    /// Whether `doc` has a tab: every document but a dictionary the app opened from an archive for itself.
    pub fn has_tab(&self, doc: &Document) -> bool {
        doc.rpf_entry_binding.is_none() || !self.auto_opened_texture_docs.contains(&doc.id)
    }

    /// The documents with a tab, in the strip's order.
    pub fn tab_order(&self) -> Vec<DocumentId> {
        self.documents
            .iter()
            .filter(|d| self.has_tab(d))
            .map(|d| d.id)
            .collect()
    }

    /// A tab's name: its file's, followed for a file opened from inside an archive by that archive's.
    pub fn tab_title(&self, doc: &Document) -> String {
        let holder = match doc.rpf_entry_binding {
            Some(row) => self.holder_name(row),
            None => doc.path.as_deref().and_then(scan_holder),
        };
        match holder {
            Some(holder) => format!("{} — {holder}", doc.file_name()),
            None => doc.file_name(),
        }
    }

    /// The nearest archive holding `row`, else the document it was opened from.
    fn holder_name(&self, row: RpfEntryBinding) -> Option<String> {
        let parent = self.document(row.parent_doc_id)?;
        if let DocPayload::Rpf(rpf) = &parent.payload {
            let mut at = rpf.session.node(row.entry_id).and_then(|n| n.parent);
            while let Some(node) = at.and_then(|id| rpf.session.node(id)) {
                if matches!(
                    node.backing,
                    RpfNodeBacking::Archive { .. } | RpfNodeBacking::Rpf9Archive { .. }
                ) {
                    return Some(node.name.clone());
                }
                at = node.parent;
            }
        }
        Some(parent.file_name())
    }

    /// Whether `id` is the preview tab: opened by a single click in Files, and neither kept nor edited since.
    pub fn is_preview(&self, id: DocumentId) -> bool {
        self.document(id).is_some_and(|doc| {
            let of_files =
                doc.rpf_entry_binding.is_some() && doc.rpf_entry_binding == self.tabs.preview;
            let of_a_scan = doc.path.is_some() && doc.path == self.tabs.scan_preview;
            (of_files || of_a_scan) && doc.edit_epoch() == 0 && !self.loses_work(doc)
        })
    }

    /// Keep `id`'s tab open past the next single click in Files, as a double click on it does.
    pub fn keep_tab(&mut self, id: DocumentId) {
        let Some(doc) = self.document(id) else {
            return;
        };
        let (row, of_a_scan) = (
            doc.rpf_entry_binding,
            doc.path.is_some() && doc.path == self.tabs.scan_preview,
        );
        if of_a_scan {
            self.tabs.scan_preview = None;
        }
        if let Some(row) = row {
            self.tabs.keep(row);
        }
    }

    /// Open the body a scan names `path`: in a tab of its own, or as a `preview` in the preview
    /// tab, in place of the scanned body there.
    pub(super) fn open_scanned_path(&mut self, path: PathBuf, preview: bool) {
        // Asked for again, it is wanted when it arrives.
        if self.tabs.scan_abandoned.as_deref() == Some(path.as_path()) {
            self.tabs.scan_abandoned = None;
        }
        if !preview {
            if self.tabs.scan_preview.as_deref() == Some(path.as_path()) {
                self.tabs.scan_preview = None;
            }
        // One already open keeps the tab it has, kept or not.
        } else if self.document_holding(&path, None).is_none() {
            let old = (self.tabs.scan_preview.clone()).filter(|old| *old != path);
            match old.as_deref().map(|old| self.document_holding(old, None)) {
                Some(Some(id)) if self.is_preview(id) => self.close_document(id),
                // ⚠ Still on its way, it has no document to close: one waiting never starts, and the one loading is dropped when it arrives.
                Some(None) => {
                    self.pending_loads
                        .retain(|waiting| Some(waiting) != old.as_ref());
                    let loading = self.load_receiver.as_ref().map(|(loading, _)| loading);
                    if loading == old.as_ref() {
                        self.tabs.scan_abandoned = old;
                    }
                }
                _ => {}
            }
            self.tabs.scan_preview = Some(path.clone());
        }
        self.load_file(path);
    }

    /// The document open for the archive row `row`.
    pub(super) fn entry_document(&self, row: RpfEntryBinding) -> Option<DocumentId> {
        match &self.document(row.parent_doc_id)?.payload {
            DocPayload::Rpf(rpf) => rpf.entry_documents.get(&row.entry_id).copied(),
            _ => None,
        }
    }

    /// The document already open for `path`, or for the archive row `row` names.
    ///
    /// ⚠ Two archive entries are one file only as one row: every archive never
    /// saved names its entries under the same placeholder path.
    pub(super) fn document_holding(
        &self,
        path: &Path,
        row: Option<RpfEntryBinding>,
    ) -> Option<DocumentId> {
        if let Some(open) = row.and_then(|row| self.entry_document(row)) {
            return Some(open);
        }
        // ⚠ Paths compare as written: one typed in another case opens a second tab.
        self.documents
            .iter()
            .find(|d| {
                d.path.as_deref() == Some(path) && (row.is_none() || d.rpf_entry_binding.is_none())
            })
            .map(|d| d.id)
    }

    /// Open a file's row in the preview tab, in place of the preview there, where the workspace on screen shows its kind.
    pub fn preview_row(&mut self, doc: DocumentId, node: RpfNodeId) {
        // ⚠ Browse previews the selected row in its Document panel and opens nothing.
        if self.active_workspace == Workspace::Browse {
            return;
        }
        let kind = match self.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => {
                rpf.session
                    .node(node)
                    .filter(|n| !n.is_dir())
                    .and_then(|n| match n.rpf9_type() {
                        Some(or_io::container::rpf::Gta6EntryType::TextureDictionary) => {
                            Some(DocumentKind::Ytd)
                        }
                        Some(or_io::container::rpf::Gta6EntryType::Resource(
                            or_io::container::rpf::Rpf9Role::Drawable
                            | or_io::container::rpf::Rpf9Role::DrawableWithBound
                            | or_io::container::rpf::Rpf9Role::Dictionary
                            | or_io::container::rpf::Rpf9Role::Bound,
                        )) => Some(DocumentKind::Gta6Drawable),
                        Some(or_io::container::rpf::Gta6EntryType::File("ymap" | "ytyp")) => {
                            Some(DocumentKind::Meta)
                        }
                        Some(_) => Some(DocumentKind::Hex),
                        None => DocumentKind::from_path(Path::new(&n.name)),
                    })
            }
            _ => None,
        };
        if !kind.is_some_and(|kind| self.active_workspace.handles_kind(kind)) {
            return;
        }
        let row = RpfEntryBinding {
            parent_doc_id: doc,
            entry_id: node,
        };
        if let Some(open) = self.entry_document(row) {
            self.activate_if_shown(open);
            return;
        }
        if self.tabs.arriving == Some(row) {
            return;
        }
        if let Some(old) = self.tabs.preview {
            match self.entry_document(old) {
                Some(id) if self.is_preview(id) => self.close_document(id),
                None if self.tabs.arriving == Some(old) => self.tabs.abandoned = Some(old),
                _ => {}
            }
        }
        self.open_rpf_entry(doc, node);
        self.tabs.preview = Some(row);
    }

    /// The tabs' upkeep once a frame: the active tab moves to the front of the order they were used in, unless `Ctrl+Tab` is stepping back with Ctrl held.
    pub fn tick_tabs(&mut self, ctrl_held: bool) {
        if !ctrl_held {
            self.tabs.cycling = None;
        }
        let tabs = self.tab_order();
        let known = self.tabs.used.len();
        self.tabs.used.retain(|id| tabs.contains(id));
        if self.tabs.used.len() != known {
            self.tabs.cycling = None;
        }
        for id in &tabs {
            if !self.tabs.used.contains(id) {
                self.tabs.used.push(*id);
            }
        }
        if self.tabs.cycling.is_none() {
            if let Some(active) = self.active_document.filter(|id| tabs.contains(id)) {
                self.tabs.used.retain(|id| *id != active);
                self.tabs.used.insert(0, active);
            }
        }
        // ⚠ Kept for good once edited: undoing pose keys must not make it a preview again.
        if let Some(row) = self.tabs.preview {
            if self
                .entry_document(row)
                .is_some_and(|id| !self.is_preview(id))
            {
                self.tabs.preview = None;
            }
        }
    }

    /// Step one tab further back in the order they were used, as `Ctrl+Tab` does each time it is pressed with Ctrl held.
    pub fn cycle_tabs(&mut self) {
        self.tick_tabs(true);
        let count = self.tabs.used.len();
        if count < 2 {
            return;
        }
        let at = self.tabs.cycling.map_or(1, |at| (at + 1) % count);
        self.tabs.cycling = Some(at);
        let id = self.tabs.used[at];
        self.set_active_document(id);
    }

    /// Close every document but `keep`, the install and the documents `keep` was opened from.
    pub fn close_other_tabs(&mut self, keep: DocumentId) {
        let others = self.other_tabs(keep);
        self.request_close(others, AfterClose::Nothing);
        self.set_active_document(keep);
    }

    /// The documents [`AppState::close_other_tabs`] closes.
    pub fn other_tabs(&self, keep: DocumentId) -> Vec<DocumentId> {
        // ⚠ Closing an archive closes what was opened from it, `keep` among them.
        let mut kept = vec![keep];
        while let Some(row) = kept
            .last()
            .and_then(|id| self.document(*id))
            .and_then(|d| d.rpf_entry_binding)
        {
            if kept.contains(&row.parent_doc_id) {
                break;
            }
            kept.push(row.parent_doc_id);
        }
        self.documents
            .iter()
            .filter(|d| !kept.contains(&d.id) && d.kind != DocumentKind::GameRoot)
            .map(|d| d.id)
            .collect()
    }

    /// Remember where a closing tab's file is, for `Ctrl+Shift+T`; a preview is not remembered.
    pub(super) fn remember_closed(&mut self, id: DocumentId) {
        let has_tab = self.document(id).is_some_and(|d| self.has_tab(d));
        if !has_tab || self.is_preview(id) {
            return;
        }
        if let Some(place) = self.place_of(Location::document(id)) {
            if self.tabs.closed.len() == CLOSED_KEPT {
                self.tabs.closed.remove(0);
            }
            self.tabs.closed.push(place);
        }
    }

    /// Open the tab closed last again, from disk or from its archive.
    pub fn reopen_closed_tab(&mut self) {
        if let Some(place) = self.tabs.closed.pop() {
            self.walk_to(Path::new(&place.source), vec![place.inner], Arrival::Open);
        }
    }

    /// Put `id`'s file first among the recent ones, unless it is a preview.
    pub(super) fn remember_recent(&mut self, id: DocumentId) {
        if self.is_preview(id) {
            return;
        }
        if let Some(place) = self.place_of(Location::document(id)) {
            self.prefs.touch_recent_file(place);
            super::save_prefs(&self.prefs);
        }
    }

    /// Open a recent file again, from disk or from what holds it.
    pub fn open_recent(&mut self, recent: &RecentFile) {
        let source = Path::new(&recent.source);
        self.walk_to(source, vec![recent.inner.clone()], Arrival::Open);
    }
}
