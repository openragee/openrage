use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use or_io::container::RpfArchive;

use super::document::{ArchiveId, RpfNodeId};
use super::memory::hash_table_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpfSessionNodeKind {
    Directory,
    BinaryFile,
    ResourceFile,
}

#[derive(Debug, Clone)]
pub enum RpfNodeBacking {
    Root,
    InMemoryDir,
    InMemoryFile {
        bytes: Arc<Vec<u8>>,
    },
    DiskFolder {
        fs_path: PathBuf,
        scanned: bool,
    },
    DiskFile {
        fs_path: PathBuf,
        size_hint: u64,
        override_bytes: Option<Arc<Vec<u8>>>,
    },
    Archive {
        archive_id: Option<ArchiveId>,
        source: ArchiveSource,
        scanned: bool,
        open_error: Option<String>,
        recovered_hint: Option<String>,
    },
    ArchiveDir {
        archive_id: ArchiveId,
    },
    ArchiveFile {
        archive_id: ArchiveId,
        entry_index: u32,
        override_bytes: Option<Arc<Vec<u8>>>,
    },
}

impl RpfNodeBacking {
    /// The bytes the row holds in memory in place of reading them again.
    pub fn memory_bytes(&self) -> usize {
        match self {
            RpfNodeBacking::InMemoryFile { bytes } => bytes.len(),
            RpfNodeBacking::DiskFile { override_bytes, .. }
            | RpfNodeBacking::ArchiveFile { override_bytes, .. } => {
                override_bytes.as_ref().map_or(0, |bytes| bytes.len())
            }
            _ => 0,
        }
    }

    pub fn needs_scan(&self) -> bool {
        matches!(
            self,
            RpfNodeBacking::DiskFolder { scanned: false, .. }
                | RpfNodeBacking::Archive {
                    scanned: false,
                    open_error: None,
                    ..
                }
        )
    }

    pub fn fs_path(&self) -> Option<&std::path::Path> {
        match self {
            RpfNodeBacking::DiskFolder { fs_path, .. }
            | RpfNodeBacking::DiskFile { fs_path, .. } => Some(fs_path.as_path()),
            RpfNodeBacking::Archive {
                source: ArchiveSource::Disk(Some(path)),
                ..
            } => Some(path.as_path()),
            _ => None,
        }
    }

    pub fn set_fs_path(&mut self, new_path: std::path::PathBuf) {
        match self {
            RpfNodeBacking::DiskFolder { fs_path, .. }
            | RpfNodeBacking::DiskFile { fs_path, .. } => *fs_path = new_path,
            RpfNodeBacking::Archive {
                source: ArchiveSource::Disk(slot),
                ..
            } => *slot = Some(new_path),
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub enum ArchiveSource {
    Disk(Option<PathBuf>),
    Nested {
        parent_archive_id: ArchiveId,
        parent_entry_index: u32,
        /// The archive the VFS listing opened; `None` when that open failed.
        opened: Option<Arc<RpfArchive>>,
    },
}

#[derive(Debug, Clone)]
pub struct RpfSessionNode {
    pub id: RpfNodeId,
    pub parent: Option<RpfNodeId>,
    pub name: String,
    pub kind: RpfSessionNodeKind,
    pub children: Vec<RpfNodeId>,
    pub backing: RpfNodeBacking,
}

impl RpfSessionNode {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, RpfSessionNodeKind::Directory)
    }

    /// A copy of the row with no children: what a drawn row needs, without a folder's list of tens of thousands copied each frame.
    pub fn without_children(&self) -> Self {
        Self {
            id: self.id,
            parent: self.parent,
            name: self.name.clone(),
            kind: self.kind,
            children: Vec::new(),
            backing: self.backing.clone(),
        }
    }

    /// What the row allocates besides the node: its name, its children and the paths its backing names.
    pub fn row_bytes(&self) -> usize {
        let backing = match &self.backing {
            RpfNodeBacking::DiskFolder { fs_path, .. }
            | RpfNodeBacking::DiskFile { fs_path, .. } => fs_path.capacity(),
            RpfNodeBacking::Archive {
                source,
                open_error,
                recovered_hint,
                ..
            } => {
                let path = match source {
                    ArchiveSource::Disk(Some(path)) => path.capacity(),
                    _ => 0,
                };
                path + open_error.as_ref().map_or(0, String::capacity)
                    + recovered_hint.as_ref().map_or(0, String::capacity)
            }
            _ => 0,
        };
        self.name.capacity() + self.children.capacity() * std::mem::size_of::<RpfNodeId>() + backing
    }
}

#[derive(Debug, Clone)]
pub struct ArchiveSlot {
    pub archive: Arc<RpfArchive>,
    pub fs_path: Option<PathBuf>,
    pub dirty: bool,
}

impl ArchiveSlot {
    /// The bytes of the archive when it was read into memory, which its tables of contents leave out.
    pub fn memory_bytes(&self) -> usize {
        let in_memory = self.archive.path().is_none() && self.archive.parent().is_none();
        if in_memory {
            self.archive.size() as usize
        } else {
            0
        }
    }
}

impl super::rpf_session::RpfEditSession {
    /// Entry and archive bytes held in memory, the read cache among them; the tables of contents are `table_bytes`.
    pub fn held_bytes(&self) -> usize {
        let entries: usize = self
            .nodes
            .values()
            .map(|node| node.backing.memory_bytes())
            .sum();
        let cached: usize = self
            .cache
            .lock()
            .map_or(0, |c| c.iter().map(|(_, b)| b.len()).sum());
        let archives: usize = self.archives.values().map(ArchiveSlot::memory_bytes).sum();
        entries + cached + archives
    }

    /// The rows of the tree: each node, its name, its children and the paths it reads from.
    pub fn tree_bytes(&self) -> usize {
        let rows: usize = self.nodes.values().map(RpfSessionNode::row_bytes).sum();
        hash_table_bytes::<(RpfNodeId, RpfSessionNode)>(self.nodes.capacity()) + rows
    }

    /// The tables of contents of the archives this tree holds, each once however many rows share it.
    ///
    /// ⚠ An archive the listing of its parent opened is held by that row before it is expanded, so
    /// a table counts from the first listing; `seen` carries the archives another tree counted.
    pub fn table_bytes(&self, seen: &mut HashSet<*const RpfArchive>) -> usize {
        let slots = self.archives.values().map(|slot| &slot.archive);
        let listed = self.nodes.values().filter_map(|node| match &node.backing {
            RpfNodeBacking::Archive {
                source:
                    ArchiveSource::Nested {
                        opened: Some(archive),
                        ..
                    },
                ..
            } => Some(archive),
            _ => None,
        });
        let mut total = 0;
        for archive in slots.chain(listed) {
            let mut at = Some(archive);
            while let Some(table) = at.filter(|table| seen.insert(Arc::as_ptr(table))) {
                total += table.table_bytes();
                at = table.parent();
            }
        }
        total
    }
}
