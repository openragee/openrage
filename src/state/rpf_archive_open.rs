use std::path::Path;
use std::sync::Arc;

use or_io::container::{RpfArchive, RpfCryptoProvider};

pub fn open_archive_from_disk(
    path: &Path,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Arc<RpfArchive>, String> {
    let open = || RpfArchive::open_with(path, Arc::clone(&crypto));
    let opened = match open() {
        // A GTA VI archive is read with the keys this app holds for the game folder that is set.
        Err(or_io::IoError::Rpf9TocEncrypted) if give_gta6_keys() => open(),
        other => other,
    };
    opened.map(Arc::new).map_err(|e| match e {
        or_io::IoError::Rpf9TocEncrypted => super::gta_vi::rpf9_refusal_at(path)
            .unwrap_or_else(|| format!("open '{}': {e}", path.display())),
        _ => format!("open '{}': {e}", path.display()),
    })
}

/// Hands the archive reader the GTA VI keys and names the app holds; `false` when it holds no keys.
fn give_gta6_keys() -> bool {
    use or_io::container::rpf::Gta6Context;
    let Ok(keys) = super::rpf9_scan::rpf9_table::keys() else {
        return false;
    };
    // ponytail: given once per refusal, so keys of a game folder changed later are picked up only
    // by the next archive that is refused. Give them where the folder is set if that shows.
    Gta6Context::give(Gta6Context {
        keys,
        names: super::rpf9_scan::rpf9_table::resolver(),
    });
    true
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
