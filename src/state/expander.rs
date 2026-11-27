use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use or_io::container::rpf::RpfNode;
use or_io::container::{RpfArchive, RpfCryptoProvider};
use or_title::vfs::{ArchiveDevice, ArchiveEntry, ArchiveEntryKind};

use super::document::{DocumentId, RpfNodeId};
use super::rpf_archive_open::open_archive_from_disk;

pub enum ExpansionRequest {
    DiskFolder {
        doc_id: DocumentId,
        node_id: RpfNodeId,
        fs_path: PathBuf,
    },
    Archive {
        doc_id: DocumentId,
        node_id: RpfNodeId,
        source: WorkerArchiveSource,
        crypto: Arc<dyn RpfCryptoProvider>,
    },
}

pub enum WorkerArchiveSource {
    Disk(PathBuf),
    /// A nested archive the VFS listing of its parent already opened.
    Opened(Arc<RpfArchive>),
    Nested {
        parent: ArchiveDevice,
        parent_entry_index: u32,
    },
}

pub enum ExpansionResult {
    DiskFolder {
        doc_id: DocumentId,
        node_id: RpfNodeId,
        items: Vec<DiskScanItem>,
    },
    Archive {
        doc_id: DocumentId,
        node_id: RpfNodeId,
        result: Result<ResolvedArchive, String>,
    },
    Failure {
        doc_id: DocumentId,
        node_id: RpfNodeId,
        error: String,
    },
}

pub struct ResolvedArchive {
    pub archive: Arc<RpfArchive>,
    pub fs_path: Option<PathBuf>,
    pub listing: Vec<ListedEntry>,
}

pub enum DiskScanItem {
    Folder {
        name: String,
        fs_path: PathBuf,
    },
    File {
        name: String,
        fs_path: PathBuf,
        size: u64,
    },
    Archive {
        name: String,
        fs_path: PathBuf,
    },
}

pub struct ExpansionWorker {
    tx: Sender<(u64, ExpansionResult)>,
    rx: Receiver<(u64, ExpansionResult)>,
    in_flight: std::collections::HashSet<(DocumentId, RpfNodeId)>,
    /// How many times a save or a defragment has replaced an archive's file; each listing carries the count it began under.
    replaced: u64,
}

impl Default for ExpansionWorker {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            in_flight: std::collections::HashSet::new(),
            replaced: 0,
        }
    }
}

impl ExpansionWorker {
    pub fn is_in_flight(&self, doc_id: DocumentId, node_id: RpfNodeId) -> bool {
        self.in_flight.contains(&(doc_id, node_id))
    }

    /// Whether any folder or archive of `doc_id` is being listed.
    pub fn is_listing(&self, doc_id: DocumentId) -> bool {
        self.in_flight.iter().any(|(doc, _)| *doc == doc_id)
    }

    pub fn finish(&mut self, doc_id: DocumentId, node_id: RpfNodeId) {
        self.in_flight.remove(&(doc_id, node_id));
    }

    /// A save or a defragment has replaced an archive's file, so every listing in flight read the old one.
    ///
    /// ponytail: one count for every archive, so a save re-lists even an
    /// unrelated archive in flight; count per file if re-listing ever costs.
    pub fn files_replaced(&mut self) {
        self.replaced += 1;
    }

    /// The finished expansions, and the rows whose listing began before a file it read was replaced.
    pub fn drain(&mut self) -> (Vec<ExpansionResult>, Vec<(DocumentId, RpfNodeId)>) {
        let mut fresh = Vec::new();
        let mut stale = Vec::new();
        while let Ok((replaced, result)) = self.rx.try_recv() {
            if replaced == self.replaced {
                fresh.push(result);
            } else {
                stale.push(result.row());
            }
        }
        (fresh, stale)
    }

    pub fn submit(&mut self, request: ExpansionRequest) -> bool {
        let key = match &request {
            ExpansionRequest::DiskFolder {
                doc_id, node_id, ..
            }
            | ExpansionRequest::Archive {
                doc_id, node_id, ..
            } => (*doc_id, *node_id),
        };
        if !self.in_flight.insert(key) {
            return false;
        }
        let (tx, replaced) = (self.tx.clone(), self.replaced);
        thread::spawn(move || {
            let _ = tx.send((replaced, run_request(request)));
        });
        true
    }
}

impl ExpansionResult {
    fn row(&self) -> (DocumentId, RpfNodeId) {
        match self {
            Self::DiskFolder {
                doc_id, node_id, ..
            }
            | Self::Archive {
                doc_id, node_id, ..
            }
            | Self::Failure {
                doc_id, node_id, ..
            } => (*doc_id, *node_id),
        }
    }
}

fn run_request(request: ExpansionRequest) -> ExpansionResult {
    match request {
        ExpansionRequest::DiskFolder {
            doc_id,
            node_id,
            fs_path,
        } => match scan_disk_folder(&fs_path) {
            Ok(items) => ExpansionResult::DiskFolder {
                doc_id,
                node_id,
                items,
            },
            Err(error) => ExpansionResult::Failure {
                doc_id,
                node_id,
                error,
            },
        },
        ExpansionRequest::Archive {
            doc_id,
            node_id,
            source,
            crypto,
        } => match open_archive(source, crypto) {
            Ok(resolved) => ExpansionResult::Archive {
                doc_id,
                node_id,
                result: Ok(resolved),
            },
            Err(error) => ExpansionResult::Archive {
                doc_id,
                node_id,
                result: Err(error),
            },
        },
    }
}

pub(super) fn scan_disk_folder(path: &std::path::Path) -> Result<Vec<DiskScanItem>, String> {
    let read =
        std::fs::read_dir(path).map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;
    let mut entries = Vec::new();
    for ent in read.flatten() {
        let p = ent.path();
        let Some(name) = p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
        else {
            continue;
        };
        let metadata = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_dir = metadata.is_dir();
        if is_dir {
            entries.push(DiskScanItem::Folder { name, fs_path: p });
        } else if name.to_ascii_lowercase().ends_with(".rpf") {
            entries.push(DiskScanItem::Archive { name, fs_path: p });
        } else {
            entries.push(DiskScanItem::File {
                name,
                fs_path: p,
                size: metadata.len(),
            });
        }
    }
    entries.sort_by(|a, b| {
        let ad = matches!(a, DiskScanItem::Folder { .. });
        let bd = matches!(b, DiskScanItem::Folder { .. });
        match (ad, bd) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => disk_item_name(a)
                .to_ascii_lowercase()
                .cmp(&disk_item_name(b).to_ascii_lowercase()),
        }
    });
    Ok(entries)
}

fn disk_item_name(item: &DiskScanItem) -> &str {
    match item {
        DiskScanItem::Folder { name, .. }
        | DiskScanItem::File { name, .. }
        | DiskScanItem::Archive { name, .. } => name,
    }
}

#[cfg(test)]
thread_local! {
    static ARCHIVE_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts, in tests, an archive opened or listed on the calling thread.
fn note_archive_work() {
    #[cfg(test)]
    ARCHIVE_WORK.with(|n| n.set(n.get() + 1));
}

/// How many archives this thread has opened or listed for the tree.
#[cfg(test)]
fn archive_work_on_this_thread() -> usize {
    ARCHIVE_WORK.with(std::cell::Cell::get)
}

pub(super) fn open_archive(
    source: WorkerArchiveSource,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<ResolvedArchive, String> {
    note_archive_work();
    let (archive, fs_path) = match source {
        WorkerArchiveSource::Disk(path) => (open_archive_from_disk(&path, crypto)?, Some(path)),
        WorkerArchiveSource::Opened(archive) => (archive, None),
        WorkerArchiveSource::Nested {
            parent,
            parent_entry_index,
        } => (
            parent
                .open_nested(parent_entry_index)
                .map_err(|e| format!("nested rpf entry {parent_entry_index}: {e}"))?,
            None,
        ),
    };
    let listing = list_archive(Arc::clone(&archive))?;
    Ok(ResolvedArchive {
        archive,
        fs_path,
        listing,
    })
}

/// One entry of the VFS listing of an archive and, for a folder, what it holds.
#[derive(Debug, Clone)]
pub struct ListedEntry {
    pub entry: ArchiveEntry,
    pub children: Vec<ListedEntry>,
}

/// Every folder of `archive`, listed from its table.
///
/// ⚠ A nested archive is listed and not opened: its table is read and decrypted when a user
/// expands it. Opening each one here made an archive holding hundreds take seconds to show.
pub fn list_archive(archive: Arc<RpfArchive>) -> Result<Vec<ListedEntry>, String> {
    note_archive_work();
    let root = (archive.nodes.first()).ok_or_else(|| "Archive has no root entry".to_string())?;
    Ok(list_folder(&archive, root))
}

fn list_folder(archive: &RpfArchive, dir: &RpfNode) -> Vec<ListedEntry> {
    (dir.children().iter())
        .filter_map(|&at| archive.nodes.get(at as usize))
        .map(|node| {
            let kind = match node {
                RpfNode::Directory { .. } => ArchiveEntryKind::Folder,
                RpfNode::BinaryFile { .. } => ArchiveEntryKind::File,
                RpfNode::ResourceFile { .. } => ArchiveEntryKind::Resource,
            };
            ListedEntry {
                entry: ArchiveEntry {
                    name: node.name().to_owned(),
                    kind,
                    entry_index: node.entry_index(),
                    archive: None,
                },
                children: list_folder(archive, node),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "entering_a_folder_waits_on_no_worker.rs"]
mod entering_a_folder_waits_on_no_worker;
