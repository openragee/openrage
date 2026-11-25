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
    /// Route a generic archive row through the same table/index address as Files.
    pub(super) fn open_gta6_archive_entry(
        &mut self,
        parent: super::DocumentId,
        entry: super::RpfNodeId,
    ) -> bool {
        let address = (|| {
            let super::DocPayload::Rpf(rpf) = &self.document(parent)?.payload else {
                return None;
            };
            let node = rpf.session.node(entry)?;
            let super::RpfNodeBacking::ArchiveFile {
                archive_id,
                entry_index,
                override_bytes: None,
            } = &node.backing
            else {
                return None;
            };
            let address = rpf
                .session
                .archive(*archive_id)?
                .archive
                .rpf9_entry_address(*entry_index)?;
            Some(super::document::rpf9::entry::indexed_path(
                &address.file,
                &address.indices,
                &node.name,
            ))
        })();
        let Some(path) = address else { return false };
        self.load_file(path);
        true
    }
}
