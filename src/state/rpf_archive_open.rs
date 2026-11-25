use std::path::Path;
use std::sync::Arc;

use or_io::container::{RpfArchive, RpfCryptoProvider};

pub fn open_archive_from_disk(
    path: &Path,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Arc<RpfArchive>, String> {
    RpfArchive::open_with(path, crypto)
        .map(Arc::new)
        .map_err(|e| match e {
            or_io::IoError::Rpf9TocEncrypted => super::gta_vi::rpf9_refusal_at(path)
                .unwrap_or_else(|| format!("open '{}': {e}", path.display())),
            _ => format!("open '{}': {e}", path.display()),
        })
}

impl super::AppState {
    /// Opens an entry of a GTA VI archive by what its body is, read at its place in the file, since its name states no kind; `false` for any other entry.
    pub(super) fn open_gta6_archive_entry(
        &mut self,
        parent: super::DocumentId,
        entry: super::RpfNodeId,
    ) -> bool {
        let place = (|| {
            let super::DocPayload::Rpf(rpf) = &self.document(parent)?.payload else {
                return None;
            };
            let super::RpfNodeBacking::ArchiveFile {
                archive_id,
                entry_index,
                override_bytes: None,
            } = &rpf.session.node(entry)?.backing
            else {
                return None;
            };
            (rpf.session.archive(*archive_id)?.archive).rpf9_place(*entry_index)
        })();
        let Some(place) = place else {
            return false;
        };
        let view = super::rpf9_scan::View::Bytes;
        let path = super::rpf9_scan::body_path(&place.file, place.offset, view);
        if place.ciphered {
            super::rpf9_scan::remember_ciphered(&path, place.base, place.entry);
        }
        self.load_file(path);
        true
    }
}
