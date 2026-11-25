//! A file that changed under the tabs open on it: each is read again in place.

use super::document_io::parse_payload_from_path;
use super::{AppState, DocPayload, DocumentId, DocumentKind};
use std::path::Path;
use std::sync::Arc;

/// Whether two paths name one file: as written, or once the system has resolved both.
pub(super) fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

impl AppState {
    /// Read again every tab open on `path`, which a recook wrote; the number read.
    ///
    /// A tab keeps its id and its place, loses the steps that hold a value of the old contents, and the previews drawn from it are built again.
    /// ⚠ One with unsaved edits is not read again until `over_edits`: it asks, as a `.ypt` does.
    /// A file the reader refuses leaves every tab as it was and is the error. The read is on the
    /// calling thread, like the open of a dictionary a model asks for.
    pub fn reload_open_file(&mut self, path: &Path, over_edits: bool) -> Result<usize, String> {
        self.reload_open_file_where(path, |_| over_edits)
    }

    /// [`Self::reload_open_file`], reading a tab over its edits only where `over_edits` holds for it.
    pub(super) fn reload_open_file_where(
        &mut self,
        path: &Path,
        over_edits: impl Fn(DocumentId) -> bool,
    ) -> Result<usize, String> {
        let open: Vec<(DocumentId, DocumentKind, bool)> = (self.documents.iter())
            .filter(|d| d.rpf_entry_binding.is_none() && d.kind != DocumentKind::Ypt)
            .filter(|d| d.path.as_deref().is_some_and(|open| same_file(open, path)))
            .map(|d| (d.id, d.kind, d.dirty))
            .collect();
        let mut reloaded = 0;
        for (id, kind, dirty) in open {
            if dirty && !over_edits(id) {
                if !self.reload_conflicts.iter().any(|p| p == path) {
                    self.reload_conflicts.push(path.to_path_buf());
                }
                continue;
            }
            let explicit_scan = matches!(
                self.document(id).map(|doc| &doc.payload),
                Some(DocPayload::Rpf9(_))
            ) && super::rpf9_scan::scanned_body(path).is_none();
            let (payload, resource) = if explicit_scan {
                let scan = super::rpf9_scan::Rpf9ScanDocument::open(path)?
                    .ok_or_else(|| tr!("Could not open the file").to_string())?;
                (DocPayload::Rpf9(Arc::new(scan)), None)
            } else {
                parse_payload_from_path(path, kind, self.keys.provider())?
            };
            if let Some(doc) = self.document_mut(id) {
                doc.reloaded(payload, resource);
            }
            self.texture_cache.forget_document(id);
            reloaded += 1;
        }
        if reloaded > 0 {
            self.recooked = self.recooked.wrapping_add(1);
        }
        Ok(reloaded)
    }

    /// Replace a `.ypt` with what is on disk; one with unsaved edits waits on the user unless `over_edits`.
    pub(super) fn reload_ypt_from_disk(&mut self, path: &Path, over_edits: bool) {
        let Some((doc_id, dirty)) = self
            .documents
            .iter()
            .find(|d| matches!(d.kind, DocumentKind::Ypt) && d.path.as_deref() == Some(path))
            .map(|d| (d.id, d.dirty))
        else {
            return;
        };
        if dirty && !over_edits {
            if !self.reload_conflicts.iter().any(|p| p == path) {
                self.reload_conflicts.push(path.to_path_buf());
            }
            return;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(?path, error = %err, "live reload: failed to read file");
                return;
            }
        };
        match <or_io::gta5::PtxFxList as or_io::gta5::File>::from_bytes(&bytes) {
            Ok(mut list) => {
                or_particle::list::canonical(&mut list);
                let resource = super::document_io::resource_image_for(DocumentKind::Ypt, &bytes);
                if let Some(doc) = self.document_mut(doc_id) {
                    doc.payload = super::DocPayload::Resource(or_io::Asset::Particles(list));
                    doc.set_resource(resource);
                    doc.history.clear();
                    doc.mark_saved();
                }
                self.bump_sim_revision();
                self.sim.manager.snapshots.clear();
                tracing::info!(?path, "reloaded YPT from disk");
            }
            Err(err) => {
                tracing::warn!(?path, error = %err, "live reload failed");
            }
        }
    }
}
