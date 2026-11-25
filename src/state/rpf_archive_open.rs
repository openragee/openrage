use std::path::Path;
use std::sync::Arc;

use or_io::container::{RpfArchive, RpfCryptoProvider};

pub fn open_archive_from_disk(
    path: &Path,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Arc<RpfArchive>, String> {
    use std::io::Read;
    let mut magic = [0; 4];
    // A GTA VI archive is read with the keys and names this app holds, given before the open: keys
    // the environment names would open it too, with names that lack the local ones.
    if std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && u32::from_le_bytes(magic) == or_io::container::rpf::RPF9_MAGIC
    {
        give_gta6_keys();
    }
    let opened = RpfArchive::open_with(path, crypto);
    opened.map(Arc::new).map_err(|e| match e {
        or_io::IoError::Rpf9TocEncrypted => super::gta_vi::rpf9_refusal_at(path)
            .unwrap_or_else(|| format!("open '{}': {e}", path.display())),
        _ => format!("open '{}': {e}", path.display()),
    })
}

/// A GTA VI archive on disk as the tree every archive opens as, read entry by entry from its file; `None` for any other file, and with no key.
pub(super) fn gta6_tree(
    path: &Path,
    crypto: &Arc<dyn RpfCryptoProvider>,
) -> Option<super::DocPayload> {
    use std::io::Read;
    let mut magic = [0u8; 4];
    (std::fs::File::open(path).ok()?)
        .read_exact(&mut magic)
        .ok()?;
    if magic != or_io::container::rpf::RPF9_MAGIC.to_le_bytes() || !give_gta6_keys() {
        return None;
    }
    let archive = RpfArchive::open_with(path, Arc::clone(crypto)).ok()?;
    let tree = super::rpf_session::RpfDocument::new(archive).ok()?;
    Some(super::DocPayload::Rpf(tree))
}

/// Hands the archive reader the GTA VI keys and names the app holds; `false` when it holds no keys.
// ⚠ Waits until the names are built, so it is reached only from a load's or a tree row's worker,
// whose loading row shows meanwhile.
fn give_gta6_keys() -> bool {
    use or_io::container::rpf::Gta6Context;
    let Ok(keys) = super::rpf9_scan::rpf9_table::keys() else {
        return false;
    };
    Gta6Context::give(Gta6Context {
        keys,
        names: super::rpf9_scan::rpf9_table::resolver(),
    });
    true
}

#[path = "rpf9_tree_open.rs"]
pub(super) mod tree_open;

#[path = "rpf9_tree_verbs.rs"]
pub(super) mod tree_verbs;

impl super::AppState {
    /// Opens an entry of a GTA VI archive by what its body is, read by its table row on the load worker, since its name states no kind; `false` for any other entry.
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
            let archive = &rpf.session.archive(*archive_id)?.archive;
            let place = archive.rpf9_place(*entry_index)?;
            let row_start = place.base + archive.binary_entry(*entry_index)?.file_offset;
            Some((place, row_start))
        })();
        let Some((place, row_start)) = place else {
            return false;
        };
        let view = super::rpf9_scan::View::Bytes;
        let path = super::rpf9_scan::body_path(&place.file, place.offset, view);
        tree_open::leave(&path, &place, row_start, super::rpf9_scan::rpf9_table::keys);
        self.load_file(path);
        true
    }
}
