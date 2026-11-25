//! Where a document is, in the places that say where.

use super::{DocPayload, Document};

impl Document {
    /// Its file; the two files a comparison sets side by side; else that it was never saved.
    pub fn location(&self) -> String {
        match (&self.path, &self.payload) {
            (Some(path), _) => (crate::state::rpf_archive_open::tree_open::shown(path))
                .map_or_else(|| path.display().to_string(), |p| p.display().to_string()),
            (None, DocPayload::Compare(compared)) => {
                let [left, right] = &compared.places;
                format!("{left} compared with {right}")
            }
            (None, _) => "(unsaved)".to_string(),
        }
    }
}
