use std::path::Path;
use std::sync::Arc;

use or_io::container::{RpfArchive, RpfCryptoProvider};

pub fn open_archive_from_disk(
    path: &Path,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Arc<RpfArchive>, String> {
    // A GTA VI archive is read with the keys and names this app holds, carried into the open; any
    // other archive keeps the crypto it was handed.
    let crypto = match is_rpf9(path) {
        true => gta6_crypto(path).unwrap_or(crypto),
        false => crypto,
    };
    let opened = RpfArchive::open_with(path, crypto);
    opened.map(Arc::new).map_err(|e| {
        super::gta_vi::rpf9_unopened_at(path, &e)
            .unwrap_or_else(|| format!("open '{}': {e}", path.display()))
    })
}

/// A GTA VI archive on disk as the tree every archive opens as, read entry by entry from its file; `None` for any other file, with no key, and, said in a toast, where it does not open.
pub(super) fn gta6_tree(path: &Path) -> Option<super::DocPayload> {
    if !is_rpf9(path) {
        return None;
    }
    let crypto = gta6_crypto(path)?;
    let opened = RpfArchive::open_with(path, crypto)
        .map_err(|e| super::gta_vi::rpf9_unopened_at(path, &e).unwrap_or_else(|| e.to_string()))
        .and_then(super::rpf_session::RpfDocument::new);
    match opened {
        Ok(tree) => Some(super::DocPayload::Rpf(tree)),
        Err(why) => {
            super::rpf9_scan::rpf9_table::offer_unopened(path, why);
            None
        }
    }
}

/// Whether the file at `path` begins with the RPF9 magic, so it is read with the GTA VI session.
fn is_rpf9(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && u32::from_le_bytes(magic) == or_io::container::rpf::RPF9_MAGIC
}

/// The GTA VI key-and-name session this app reads RPF9 archives with, as the provider the open is
/// handed; `None` when the app holds no keys, so the archive is refused rather than read.
// ⚠ Waits until the names are built, so it is reached only from a load's or a tree row's worker,
// whose loading row shows meanwhile.
fn gta6_crypto(path: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    use or_io::container::rpf::Gta6Context;
    // The keys are those of the build whose platform letter the archive's header states at 0x19.
    let mut head = [0u8; 0x1A];
    let read = std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut head));
    let platform = read.ok().and_then(|()| or_io::Platform::from_letter(head[0x19]));
    let keys = super::rpf9_scan::rpf9_table::keys_of(platform).ok()?;
    Some(Arc::new(Gta6Context {
        keys,
        names: super::rpf9_scan::rpf9_table::resolver(),
    }))
}

/// The GTA VI key session a test opens RPF9 archives with, held per thread while its guard lives.
///
/// The production path carries the session in the provider [`open_archive_from_disk`] is handed, so
/// nothing here is reached outside tests. Tests, though, set up their keys apart from the open they
/// drive through the real app, so this test-only thread-local stands in for that provider: it is no
/// process-wide global (gone with R72f), it is per-thread so parallel tests do not collide, and a
/// guard restores the prior session on drop.
#[cfg(test)]
pub(crate) mod gta6_session {
    use std::cell::RefCell;
    use std::sync::Arc;

    use or_io::container::rpf::Gta6Context;
    use or_io::container::{NoOpCryptoProvider, RpfCryptoProvider};

    thread_local! {
        static HELD: RefCell<Option<Arc<Gta6Context>>> = const { RefCell::new(None) };
    }

    /// Restores the session held before it on drop.
    pub(crate) struct Reading(Option<Arc<Gta6Context>>);

    impl Drop for Reading {
        fn drop(&mut self) {
            HELD.with(|held| *held.borrow_mut() = self.0.take());
        }
    }

    /// Hold `context` as this thread's session until the returned guard drops.
    pub(crate) fn hold(context: Gta6Context) -> Reading {
        Reading(HELD.with(|held| held.borrow_mut().replace(Arc::new(context))))
    }

    /// The provider an RPF9 open is handed on this thread: the held session, else a no-op that
    /// refuses an RPF9 table as it would with no keys.
    pub(crate) fn crypto() -> Arc<dyn RpfCryptoProvider> {
        match HELD.with(|held| held.borrow().clone()) {
            Some(session) => session,
            None => Arc::new(NoOpCryptoProvider::new()),
        }
    }
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
                override_bytes,
            } = &rpf.session.node(entry)?.backing
            else {
                return None;
            };
            let archive = &rpf.session.archive(*archive_id)?.archive;
            let place = archive.rpf9_place(*entry_index)?;
            let row_start = place.base + archive.binary_entry(*entry_index)?.file_offset;
            Some((place, row_start, override_bytes.clone()))
        })();
        let Some((place, row_start, edited)) = place else {
            return false;
        };
        let view = super::rpf9_scan::View::Bytes;
        let path = super::rpf9_scan::body_path(&place.file, place.offset, view);
        tree_open::leave(
            &path,
            &place,
            row_start,
            super::rpf9_scan::rpf9_table::keys,
            edited,
        );
        tree_open::name(self, &path, parent, entry);
        self.load_file(path);
        true
    }
}
