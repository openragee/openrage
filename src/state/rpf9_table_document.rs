//! A GTA VI archive opened from disk and listed from its decrypted table: its tab and the Files list
//! read the tree held here, and a scan runs only where no key the app holds opens the table.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{GtaViKeys, NameResolver, Rpf9Header, RPF9_HEADER_SIZE, RPF9_MAGIC};

use crate::state::rpf9_scan::rpf9_table::{self, TableNode, TableRow};
use crate::state::rpf9_scan::{body_path, remember_ciphered, Rpf9ScanDocument, View};
use crate::state::{AppState, DocPayload, DocumentId};
use crate::ui::panels::gta6_archive_tree::Gta6ArchiveExplorer;

/// A GTA VI archive as its table lists it.
pub struct Rpf9TableDocument {
    pub path: PathBuf,
    pub header: Rpf9Header,
    /// The mounted tree, what of it is expanded and its filter; a nested table mounts on a worker.
    tree: Mutex<Gta6ArchiveExplorer>,
}

impl Rpf9TableDocument {
    /// The archive at `path` listed from its table with `keys`, or the sentence that says a scan lists it.
    pub(crate) fn open_with(
        path: &Path,
        header: Rpf9Header,
        keys: Result<&GtaViKeys, String>,
        resolver: &NameResolver,
    ) -> Result<Self, String> {
        let node = rpf9_table::listing(path, keys, resolver)?;
        let mut tree = Gta6ArchiveExplorer::default();
        tree.show_opened(path.to_path_buf(), node);
        Ok(Self {
            path: path.to_path_buf(),
            header,
            tree: Mutex::new(tree),
        })
    }

    pub fn tree(&self) -> MutexGuard<'_, Gta6ArchiveExplorer> {
        (self.tree.lock()).unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn held_bytes(&self) -> usize {
        fn rows(node: &TableNode) -> usize {
            node.rows.len() + node.children.values().map(rows).sum::<usize>()
        }
        self.tree().root.as_ref().map_or(0, rows) * std::mem::size_of::<TableRow>()
    }
}

/// What a GTA VI archive opens as: its table where a key the app holds opens it, and then no scan
/// runs; else its scan, once the open has said why no table lists it. `None` for any other file.
pub(crate) fn archive_payload(path: &Path) -> Result<Option<DocPayload>, String> {
    // The resolver reads the harvested names, so it is loaded only once a key is held.
    let table = header_of(path).map(|header| {
        let keys = rpf9_table::keys()?;
        Rpf9TableDocument::open_with(path, header, Ok(&keys), &rpf9_table::resolver())
    });
    payload_of(path, table)
}

/// The table where it opened, else the scan, which a file whose head is not RPF9 has none of.
// ponytail: the fallback's toast lists the archive a second time to say why; one list where keys fail.
fn payload_of(
    path: &Path,
    table: Option<Result<Rpf9TableDocument, String>>,
) -> Result<Option<DocPayload>, String> {
    if let Some(Ok(table)) = table {
        return Ok(Some(DocPayload::Rpf9Table(Arc::new(table))));
    }
    rpf9_table::offer(path);
    let scan = Rpf9ScanDocument::open(path)?;
    Ok(scan.map(|scan| DocPayload::Rpf9(Arc::new(scan))))
}

/// The header the head of the file at `path` states, where it opens with the RPF9 magic.
fn header_of(path: &Path) -> Option<Rpf9Header> {
    let mut head = Vec::with_capacity(RPF9_HEADER_SIZE);
    (File::open(path).ok()?.take(RPF9_HEADER_SIZE as u64))
        .read_to_end(&mut head)
        .ok()?;
    if !head.starts_with(&RPF9_MAGIC.to_le_bytes()) {
        return None;
    }
    Rpf9Header::parse(&head).ok()
}

impl AppState {
    /// The table document listing the tab on screen: that tab's own, or its archive's for a body of one.
    pub fn table_listing_the_active_tab(&self) -> Option<DocumentId> {
        let active = self.active_document()?;
        if matches!(active.payload, DocPayload::Rpf9Table(_)) {
            return Some(active.id);
        }
        let archive = active.path.as_deref()?.parent()?;
        (self.documents.iter())
            .find(|d| {
                matches!(d.payload, DocPayload::Rpf9Table(_)) && d.path.as_deref() == Some(archive)
            })
            .map(|d| d.id)
    }

    /// Open leaf `row` of the table at `base` in `archive`, deciphered on the load's worker where enciphered.
    pub fn open_table_entry(&mut self, archive: &Path, base: u64, row: &TableRow, preview: bool) {
        let Some(body) = row.absolute_start else {
            self.show_info_toast(tr!(
                "This entry's row states no start, so its body cannot be read."
            ));
            return;
        };
        let path = body_path(archive, body, View::Bytes);
        if row.cipher {
            remember_ciphered(&path, base, row.index);
        }
        self.open_scanned_path(path, preview);
    }
}

#[cfg(test)]
#[path = "rpf9_table_document_tests.rs"]
pub(crate) mod tests;
