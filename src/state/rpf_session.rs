use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use or_io::container::rpf::{BodyForm, RpfEntry};
use or_io::container::{NoOpCryptoProvider, RpfArchive, RpfCryptoProvider};
use or_title::vfs::{ArchiveDevice, ArchiveEntry, ArchiveEntryKind};

use super::document::{ArchiveId, DocumentId, RpfNodeId};
use super::expander::{list_archive, ListedEntry};
use super::rpf_node::{
    ArchiveSlot, ArchiveSource, RpfNodeBacking, RpfSessionNode, RpfSessionNodeKind,
};

const BYTE_CACHE_CAPACITY: usize = 64;

type SortedChildren = (
    u64,
    super::explorer::SortOrder,
    Vec<RpfNodeId>,
    Arc<[RpfNodeId]>,
);

pub struct RpfEditSession {
    pub(super) nodes: HashMap<RpfNodeId, RpfSessionNode>,
    pub(super) cache: Mutex<LruCache<RpfNodeId, Arc<Vec<u8>>>>,
    pub(super) archives: HashMap<ArchiveId, ArchiveSlot>,
    /// Each folder's children as [`super::explorer::sorted_children_at`] last sorted them, with the
    /// payload epoch, order and children they were sorted from.
    pub(super) sorted: Mutex<HashMap<RpfNodeId, SortedChildren>>,
    pub crypto: Arc<dyn RpfCryptoProvider>,
    root_id: RpfNodeId,
    next_id: RpfNodeId,
    next_archive_id: ArchiveId,
}

impl RpfEditSession {
    pub fn empty() -> Self {
        let root = RpfSessionNode {
            id: 0,
            parent: None,
            name: String::new(),
            kind: RpfSessionNodeKind::Directory,
            children: Vec::new(),
            backing: RpfNodeBacking::Root,
        };
        let mut nodes = HashMap::new();
        nodes.insert(root.id, root);
        Self {
            nodes,
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(BYTE_CACHE_CAPACITY).expect("non-zero cap"),
            )),
            archives: HashMap::new(),
            sorted: Mutex::new(HashMap::new()),
            crypto: Arc::new(NoOpCryptoProvider::new()),
            root_id: 0,
            next_id: 1,
            next_archive_id: 0,
        }
    }

    pub fn from_archive(archive: RpfArchive) -> Result<Self, String> {
        let mut session = Self::empty();
        session.crypto = Arc::clone(archive.keys());
        let archive = Arc::new(archive);
        let listing = list_archive(archive.clone())?;
        let archive_id = session.register_archive(archive, None)?;
        if let Some(root) = session.nodes.get_mut(&session.root_id) {
            root.backing = RpfNodeBacking::ArchiveDir { archive_id };
        }
        let root_id = session.root_id;
        session.populate_archive_children(root_id, archive_id, &listing)?;
        Ok(session)
    }

    pub fn from_game_root(install_root: PathBuf, crypto: Arc<dyn RpfCryptoProvider>) -> Self {
        let mut session = Self::empty();
        session.crypto = crypto;
        if let Some(root) = session.nodes.get_mut(&session.root_id) {
            root.name = install_root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("GTA V")
                .to_string();
            root.backing = RpfNodeBacking::DiskFolder {
                fs_path: install_root,
                scanned: false,
            };
        }
        session
    }

    pub fn root_id(&self) -> RpfNodeId {
        self.root_id
    }

    pub fn node(&self, id: RpfNodeId) -> Option<&RpfSessionNode> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: RpfNodeId) -> Option<&mut RpfSessionNode> {
        self.nodes.get_mut(&id)
    }

    pub fn children_of(&self, id: RpfNodeId) -> Vec<RpfNodeId> {
        self.nodes
            .get(&id)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    pub fn archive(&self, id: ArchiveId) -> Option<&ArchiveSlot> {
        self.archives.get(&id)
    }

    /// The VFS device over one of this session's archives.
    pub fn device(&self, id: ArchiveId) -> Option<ArchiveDevice> {
        let slot = self.archives.get(&id)?;
        Some(ArchiveDevice::new(Arc::clone(&slot.archive)))
    }

    /// The archives the row `node_id` sits in, innermost first: each opened archive row at or above it.
    ///
    /// ⚠ Not the archive a row's backing reads: a row moved in from another
    /// archive reads that one's file until this one is saved, and belongs to
    /// this one at once.
    fn archives_around(&self, node_id: RpfNodeId) -> impl Iterator<Item = ArchiveId> + '_ {
        std::iter::successors(self.nodes.get(&node_id), |node| {
            node.parent.and_then(|parent| self.nodes.get(&parent))
        })
        .filter_map(|node| match node.backing {
            RpfNodeBacking::Archive { archive_id, .. } => archive_id,
            _ => None,
        })
    }

    /// The archive on disk the row `node_id` is saved into, through every archive it is nested in, and its file.
    pub fn archive_on_disk_around(&self, node_id: RpfNodeId) -> Option<(ArchiveId, PathBuf)> {
        self.archives_around(node_id)
            .find_map(|id| Some((id, self.archives.get(&id)?.fs_path.clone()?)))
    }

    /// Mark the archive holding `node_id` unsaved, with every archive it is nested in.
    ///
    /// ⚠ A nested archive reaches disk only inside the archive on disk holding
    /// it; marked alone, its edit is neither gated nor saved.
    pub fn mark_archive_dirty(&mut self, node_id: RpfNodeId) {
        let around: Vec<ArchiveId> = self.archives_around(node_id).collect();
        for id in around {
            if let Some(slot) = self.archives.get_mut(&id) {
                slot.dirty = true;
            }
        }
    }

    pub fn size_for(&self, id: RpfNodeId) -> Option<u64> {
        let node = self.nodes.get(&id)?;
        match &node.backing {
            RpfNodeBacking::InMemoryFile { bytes } => Some(bytes.len() as u64),
            RpfNodeBacking::DiskFile {
                size_hint,
                override_bytes,
                ..
            } => Some(
                override_bytes
                    .as_ref()
                    .map(|b| b.len() as u64)
                    .unwrap_or(*size_hint),
            ),
            RpfNodeBacking::ArchiveFile {
                archive_id,
                entry_index,
                override_bytes,
            } => {
                if let Some(b) = override_bytes {
                    return Some(b.len() as u64);
                }
                let slot = self.archives.get(archive_id)?;
                let entry = slot.archive.entries.get(*entry_index as usize)?;
                Some(match entry {
                    RpfEntry::Binary(b) => b.uncompressed_size as u64,
                    RpfEntry::Resource(r) => r.consumed_size as u64,
                    RpfEntry::Directory(_) => 0,
                })
            }
            _ => None,
        }
    }

    pub fn bytes_for(&self, id: RpfNodeId) -> Option<Arc<Vec<u8>>> {
        let node = self.nodes.get(&id)?;
        match &node.backing {
            RpfNodeBacking::InMemoryFile { bytes } => Some(Arc::clone(bytes)),
            RpfNodeBacking::DiskFile {
                fs_path,
                override_bytes,
                ..
            } => {
                if let Some(b) = override_bytes {
                    return Some(Arc::clone(b));
                }
                if let Ok(mut c) = self.cache.lock() {
                    if let Some(cached) = c.get(&id).cloned() {
                        tracing::trace!(node_id = id, "disk body cache hit");
                        return Some(cached);
                    }
                }
                let bytes = std::fs::read(fs_path).ok()?;
                let arc = Arc::new(bytes);
                if let Ok(mut c) = self.cache.lock() {
                    c.put(id, Arc::clone(&arc));
                }
                Some(arc)
            }
            RpfNodeBacking::ArchiveFile {
                archive_id,
                entry_index,
                override_bytes,
            } => {
                if let Some(b) = override_bytes {
                    return Some(Arc::clone(b));
                }
                if let Ok(mut c) = self.cache.lock() {
                    if let Some(cached) = c.get(&id).cloned() {
                        tracing::trace!(node_id = id, "rpf body cache hit");
                        return Some(cached);
                    }
                }
                let device = self.device(*archive_id)?;
                match device.read_entry(*entry_index, BodyForm::Decoded) {
                    Ok(bytes) => {
                        tracing::trace!(node_id = id, len = bytes.len(), "rpf body extracted");
                        let arc = Arc::new(bytes);
                        if let Ok(mut c) = self.cache.lock() {
                            c.put(id, Arc::clone(&arc));
                        }
                        Some(arc)
                    }
                    Err(e) => {
                        tracing::warn!(node_id = id, error = %e, "rpf body extract failed");
                        None
                    }
                }
            }
            _ => None,
        }
    }

    pub fn set_bytes_for(&mut self, id: RpfNodeId, bytes: Vec<u8>) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| tr!("RPF node {id} does not exist", id = id))?;
        if node.is_dir() {
            return Err(tr!("Cannot assign file bytes to a folder").to_string());
        }
        let arc = Arc::new(bytes);
        match &mut node.backing {
            RpfNodeBacking::InMemoryFile { bytes } => {
                *bytes = Arc::clone(&arc);
            }
            RpfNodeBacking::DiskFile { override_bytes, .. } => {
                *override_bytes = Some(Arc::clone(&arc));
            }
            RpfNodeBacking::ArchiveFile { override_bytes, .. } => {
                *override_bytes = Some(Arc::clone(&arc));
            }
            _ => {
                return Err(tr!("Node has no file backing").to_string());
            }
        }
        if let Ok(mut c) = self.cache.lock() {
            c.pop(&id);
        }
        self.mark_archive_dirty(id);
        Ok(())
    }

    pub fn path_of(&self, id: RpfNodeId) -> Option<String> {
        let mut parts = Vec::new();
        let mut cur = self.nodes.get(&id)?;
        while let Some(parent) = cur.parent {
            parts.push(cur.name.clone());
            cur = self.nodes.get(&parent)?;
        }
        parts.reverse();
        Some(parts.join("/"))
    }

    pub fn all_nodes(&self) -> Vec<RpfNodeId> {
        let mut out: Vec<RpfNodeId> = self.nodes.keys().copied().collect();
        out.sort_unstable();
        out
    }

    pub fn all_directory_nodes(&self) -> Vec<RpfNodeId> {
        let mut out: Vec<RpfNodeId> = self
            .nodes
            .values()
            .filter(|n| n.is_dir())
            .map(|n| n.id)
            .collect();
        out.sort_unstable();
        out
    }

    pub fn file_records(&self) -> Result<Vec<(String, Vec<u8>, bool)>, String> {
        self.records_under(self.root_id)
    }

    pub fn file_count(&self) -> usize {
        self.nodes.values().filter(|n| !n.is_dir()).count()
    }

    /// The directory half of [`Self::file_count`].
    ///
    /// ⚠ Read every frame by the hierarchy badge and the inspector, which must
    /// not build and sort [`Self::all_directory_nodes`] for one integer. A
    /// session holds only expanded archives' nodes: the largest top-level
    /// archive of a retail Legacy install, `update2.rpf`, has **2,226
    /// entries**, with `common.rpf` at 1,421 and `update.rpf` at 1,644.
    pub fn directory_count(&self) -> usize {
        self.nodes.values().filter(|n| n.is_dir()).count()
    }

    pub fn archive_root_node(&self, archive_id: ArchiveId) -> Option<RpfNodeId> {
        self.nodes
            .iter()
            .find(|(_, n)| {
                matches!(
                    n.backing,
                    RpfNodeBacking::Archive {
                        archive_id: Some(aid),
                        ..
                    } if aid == archive_id
                )
            })
            .map(|(id, _)| *id)
    }

    pub fn file_records_for_archive(
        &self,
        archive_id: ArchiveId,
    ) -> Result<Vec<(String, Vec<u8>, bool)>, String> {
        let root = self.archive_root_node(archive_id).ok_or_else(|| {
            tr!(
                "Archive {archive_id} has no root node in this session",
                archive_id = archive_id
            )
        })?;
        self.records_under(root)
    }

    /// Mark an archive saved, with every archive nested in it, which its save wrote.
    pub fn clear_archive_dirty(&mut self, archive_id: ArchiveId) {
        let node_id = self.archive_root_node(archive_id);
        let mut saved = vec![archive_id];
        let mut stack: Vec<RpfNodeId> = node_id.into_iter().collect();
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            if let RpfNodeBacking::Archive {
                archive_id: Some(nested),
                ..
            } = node.backing
            {
                saved.push(nested);
            }
            stack.extend(&node.children);
        }
        for id in saved {
            if let Some(slot) = self.archives.get_mut(&id) {
                slot.dirty = false;
            }
        }
    }

    pub fn dirty_archive_ids(&self) -> Vec<ArchiveId> {
        self.archives
            .iter()
            .filter(|(_, slot)| slot.dirty)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Whether the row `node_id` is an opened archive holding an edit no save has written.
    pub fn archive_unsaved(&self, node_id: RpfNodeId) -> bool {
        match self.nodes.get(&node_id).map(|node| &node.backing) {
            Some(RpfNodeBacking::Archive {
                archive_id: Some(id),
                ..
            }) => self.archives.get(id).is_some_and(|slot| slot.dirty),
            _ => false,
        }
    }

    pub fn add_folder(&mut self, parent_id: RpfNodeId, name: &str) -> Result<RpfNodeId, String> {
        let id = self.add_directory(parent_id, name.to_string(), RpfNodeBacking::InMemoryDir)?;
        self.mark_archive_dirty(parent_id);
        Ok(id)
    }

    pub fn add_file_copy(
        &mut self,
        parent_id: RpfNodeId,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<RpfNodeId, String> {
        let kind = if looks_like_resource(&bytes) {
            RpfSessionNodeKind::ResourceFile
        } else {
            RpfSessionNodeKind::BinaryFile
        };
        let arc = Arc::new(bytes);
        let id = self.add_file(
            parent_id,
            name.to_string(),
            kind,
            RpfNodeBacking::InMemoryFile { bytes: arc },
        )?;
        self.mark_archive_dirty(parent_id);
        Ok(id)
    }

    pub fn rename_node(&mut self, node_id: RpfNodeId, new_name: &str) -> Result<(), String> {
        if node_id == self.root_id {
            return Err(tr!("Cannot rename RPF root").to_string());
        }
        let normalized = normalize_name(new_name)?;
        let parent_id = self
            .nodes
            .get(&node_id)
            .and_then(|n| n.parent)
            .ok_or_else(|| tr!("Node {node_id} has no parent", node_id = node_id))?;
        self.ensure_unique_name(parent_id, &normalized, Some(node_id))?;
        let node = self
            .nodes
            .get_mut(&node_id)
            .ok_or_else(|| tr!("Node {node_id} does not exist", node_id = node_id))?;
        node.name = normalized;
        // ⚠ Not the row's own: a renamed archive holds what it held, and one on disk has nothing to save.
        self.mark_archive_dirty(parent_id);
        Ok(())
    }

    pub fn move_node(&mut self, node_id: RpfNodeId, new_parent: RpfNodeId) -> Result<(), String> {
        if node_id == self.root_id {
            return Err(tr!("Cannot move RPF root").to_string());
        }
        if node_id == new_parent {
            return Err(tr!("Cannot move a node into itself").to_string());
        }
        let new_parent_node = self.nodes.get(&new_parent).ok_or_else(|| {
            tr!(
                "Target folder {new_parent} does not exist",
                new_parent = new_parent
            )
        })?;
        if !new_parent_node.is_dir() {
            return Err(tr!("Target must be a folder").to_string());
        }
        self.ensure_parent_dir(new_parent)?;
        if self.is_descendant(new_parent, node_id) {
            return Err(tr!("Cannot move a folder into its own descendant").to_string());
        }

        let (old_parent, name) = {
            let node = self
                .nodes
                .get(&node_id)
                .ok_or_else(|| tr!("Node {node_id} does not exist", node_id = node_id))?;
            (
                node.parent
                    .ok_or_else(|| tr!("Node {node_id} has no parent", node_id = node_id))?,
                node.name.clone(),
            )
        };

        if old_parent == new_parent {
            return Ok(());
        }
        self.ensure_unique_name(new_parent, &name, Some(node_id))?;
        self.detach_from_parent(node_id, old_parent)?;
        self.attach_to_parent(node_id, new_parent)?;
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.parent = Some(new_parent);
        }
        self.mark_archive_dirty(old_parent);
        self.mark_archive_dirty(new_parent);
        Ok(())
    }

    pub fn delete_node(&mut self, node_id: RpfNodeId) -> Result<(), String> {
        if node_id == self.root_id {
            return Err(tr!("Cannot delete RPF root").to_string());
        }
        let parent = self
            .nodes
            .get(&node_id)
            .and_then(|n| n.parent)
            .ok_or_else(|| tr!("Node {node_id} has no parent", node_id = node_id))?;
        self.detach_from_parent(node_id, parent)?;

        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.nodes.remove(&id) {
                // ⚠ Kept, an opened archive's slot stays unsaved with no rows to save.
                if let RpfNodeBacking::Archive {
                    archive_id: Some(archive_id),
                    ..
                } = node.backing
                {
                    self.archives.remove(&archive_id);
                }
                stack.extend(node.children);
            }
            if let Ok(mut c) = self.cache.lock() {
                c.pop(&id);
            }
        }
        self.mark_archive_dirty(parent);
        Ok(())
    }

    pub fn preferred_drop_folder(&self, selected: Option<RpfNodeId>) -> RpfNodeId {
        if let Some(id) = selected {
            if let Some(node) = self.nodes.get(&id) {
                if node.is_dir() {
                    return id;
                }
                if let Some(parent) = node.parent {
                    return parent;
                }
            }
        }
        self.root_id
    }

    /// Hold `archive` in a new slot under the next id; `Err` once every id has been used.
    pub fn register_archive(
        &mut self,
        archive: Arc<RpfArchive>,
        fs_path: Option<PathBuf>,
    ) -> Result<ArchiveId, String> {
        let id = self.next_archive_id;
        self.next_archive_id = (self.next_archive_id).checked_add(1).ok_or(tr!(
            "Every archive id of this session is used; close it and open it again"
        ))?;
        self.archives.insert(
            id,
            ArchiveSlot {
                archive,
                fs_path,
                dirty: false,
            },
        );
        Ok(id)
    }

    pub fn populate_archive_children(
        &mut self,
        parent_id: RpfNodeId,
        archive_id: ArchiveId,
        listing: &[ListedEntry],
    ) -> Result<(), String> {
        for listed in listing {
            let ArchiveEntry {
                name,
                kind,
                entry_index,
                archive,
            } = &listed.entry;
            let entry_index = *entry_index;
            let id = self.alloc_id();
            let (kind, backing) = match kind {
                ArchiveEntryKind::Folder => (
                    RpfSessionNodeKind::Directory,
                    RpfNodeBacking::ArchiveDir { archive_id },
                ),
                ArchiveEntryKind::File if name.to_ascii_lowercase().ends_with(".rpf") => (
                    RpfSessionNodeKind::Directory,
                    RpfNodeBacking::Archive {
                        archive_id: None,
                        source: ArchiveSource::Nested {
                            parent_archive_id: archive_id,
                            parent_entry_index: entry_index,
                            opened: archive.clone(),
                        },
                        scanned: false,
                        open_error: None,
                        recovered_hint: None,
                    },
                ),
                ArchiveEntryKind::File => (
                    RpfSessionNodeKind::BinaryFile,
                    RpfNodeBacking::ArchiveFile {
                        archive_id,
                        entry_index,
                        override_bytes: None,
                    },
                ),
                ArchiveEntryKind::Resource => (
                    RpfSessionNodeKind::ResourceFile,
                    RpfNodeBacking::ArchiveFile {
                        archive_id,
                        entry_index,
                        override_bytes: None,
                    },
                ),
            };
            let new_node = RpfSessionNode {
                id,
                parent: Some(parent_id),
                name: name.clone(),
                kind,
                children: Vec::new(),
                backing,
            };
            self.nodes.insert(id, new_node);
            self.attach_to_parent(id, parent_id)?;
            self.populate_archive_children(id, archive_id, &listed.children)?;
        }
        Ok(())
    }

    fn add_directory(
        &mut self,
        parent_id: RpfNodeId,
        name: String,
        backing: RpfNodeBacking,
    ) -> Result<RpfNodeId, String> {
        let normalized = normalize_name(&name)?;
        self.ensure_parent_dir(parent_id)?;
        self.ensure_unique_name(parent_id, &normalized, None)?;

        let id = self.alloc_id();
        let node = RpfSessionNode {
            id,
            parent: Some(parent_id),
            name: normalized,
            kind: RpfSessionNodeKind::Directory,
            children: Vec::new(),
            backing,
        };
        self.nodes.insert(id, node);
        self.attach_to_parent(id, parent_id)?;
        Ok(id)
    }

    fn add_file(
        &mut self,
        parent_id: RpfNodeId,
        name: String,
        kind: RpfSessionNodeKind,
        backing: RpfNodeBacking,
    ) -> Result<RpfNodeId, String> {
        let normalized = normalize_name(&name)?;
        self.ensure_parent_dir(parent_id)?;
        self.ensure_unique_name(parent_id, &normalized, None)?;
        if matches!(kind, RpfSessionNodeKind::Directory) {
            return Err(tr!("File entries cannot use directory kind").to_string());
        }
        let id = self.alloc_id();
        let node = RpfSessionNode {
            id,
            parent: Some(parent_id),
            name: normalized,
            kind,
            children: Vec::new(),
            backing,
        };
        self.nodes.insert(id, node);
        self.attach_to_parent(id, parent_id)?;
        Ok(id)
    }

    pub fn add_disk_child(
        &mut self,
        parent_id: RpfNodeId,
        name: String,
        backing: RpfNodeBacking,
    ) -> Result<RpfNodeId, String> {
        match backing {
            RpfNodeBacking::DiskFolder { .. } => self.add_directory(parent_id, name, backing),
            RpfNodeBacking::DiskFile { .. } => {
                self.add_file(parent_id, name, RpfSessionNodeKind::BinaryFile, backing)
            }
            _ => Err("add_disk_child requires DiskFolder/DiskFile backing".to_string()),
        }
    }

    pub fn add_unopened_disk_archive(
        &mut self,
        parent_id: RpfNodeId,
        name: String,
        fs_path: PathBuf,
    ) -> Result<RpfNodeId, String> {
        let backing = RpfNodeBacking::Archive {
            archive_id: None,
            source: ArchiveSource::Disk(Some(fs_path)),
            scanned: false,
            open_error: None,
            recovered_hint: None,
        };
        self.add_directory(parent_id, name, backing)
    }

    pub fn mark_scanned(&mut self, id: RpfNodeId) {
        if let Some(node) = self.nodes.get_mut(&id) {
            match &mut node.backing {
                RpfNodeBacking::DiskFolder { scanned, .. } => *scanned = true,
                RpfNodeBacking::Archive { scanned, .. } => *scanned = true,
                _ => {}
            }
        }
    }

    /// Forget what was read from an archive node's file, so the next expansion opens it afresh.
    pub fn unload_archive(&mut self, node_id: RpfNodeId) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };
        let mut stack = std::mem::take(&mut node.children);
        if let RpfNodeBacking::Archive {
            archive_id,
            scanned,
            open_error,
            ..
        } = &mut node.backing
        {
            if let Some(id) = archive_id.take() {
                self.archives.remove(&id);
            }
            *scanned = false;
            *open_error = None;
        }
        while let Some(id) = stack.pop() {
            if let Some(child) = self.nodes.remove(&id) {
                if let RpfNodeBacking::Archive {
                    archive_id: Some(nested),
                    ..
                } = child.backing
                {
                    self.archives.remove(&nested);
                }
                stack.extend(child.children);
            }
            if let Ok(mut c) = self.cache.lock() {
                c.pop(&id);
            }
        }
    }

    pub fn needs_expansion(&self, id: RpfNodeId) -> bool {
        self.nodes
            .get(&id)
            .map(|n| n.backing.needs_scan())
            .unwrap_or(false)
    }

    fn alloc_id(&mut self) -> RpfNodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn ensure_parent_dir(&self, parent_id: RpfNodeId) -> Result<(), String> {
        let Some(parent) = self.nodes.get(&parent_id) else {
            return Err(tr!(
                "Parent folder {parent_id} does not exist",
                parent_id = parent_id
            ));
        };
        if !parent.is_dir() {
            return Err(tr!(
                "Parent {parent_id} is not a folder",
                parent_id = parent_id
            ));
        }
        // ⚠ What lands under an archive no listing opened is lost: a save
        // writes that archive as the bytes it was read as.
        if matches!(
            parent.backing,
            RpfNodeBacking::Archive {
                archive_id: None,
                ..
            }
        ) {
            return Err(tr!(
                "'{name}' is not open, so nothing can be added to it",
                name = parent.name
            ));
        }
        Ok(())
    }

    fn ensure_unique_name(
        &self,
        parent_id: RpfNodeId,
        name: &str,
        ignore_node: Option<RpfNodeId>,
    ) -> Result<(), String> {
        let folded = name.to_ascii_lowercase();
        let Some(parent) = self.nodes.get(&parent_id) else {
            return Err(tr!(
                "Parent folder {parent_id} does not exist",
                parent_id = parent_id
            ));
        };
        for child_id in &parent.children {
            if Some(*child_id) == ignore_node {
                continue;
            }
            if let Some(child) = self.nodes.get(child_id) {
                if child.name.eq_ignore_ascii_case(&folded) {
                    return Err(tr!("Sibling '{name}' already exists", name = name));
                }
            }
        }
        Ok(())
    }

    fn detach_from_parent(
        &mut self,
        node_id: RpfNodeId,
        parent_id: RpfNodeId,
    ) -> Result<(), String> {
        let Some(parent) = self.nodes.get_mut(&parent_id) else {
            return Err(tr!(
                "Parent folder {parent_id} does not exist",
                parent_id = parent_id
            ));
        };
        parent.children.retain(|&c| c != node_id);
        Ok(())
    }

    fn attach_to_parent(&mut self, node_id: RpfNodeId, parent_id: RpfNodeId) -> Result<(), String> {
        let Some(parent) = self.nodes.get_mut(&parent_id) else {
            return Err(tr!(
                "Parent folder {parent_id} does not exist",
                parent_id = parent_id
            ));
        };
        parent.children.push(node_id);
        Ok(())
    }

    pub fn is_descendant(&self, candidate: RpfNodeId, ancestor: RpfNodeId) -> bool {
        let mut cur = Some(candidate);
        while let Some(id) = cur {
            if id == ancestor {
                return true;
            }
            cur = self.nodes.get(&id).and_then(|n| n.parent);
        }
        false
    }

    pub fn set_crypto(&mut self, crypto: Arc<dyn RpfCryptoProvider>) {
        self.crypto = crypto;
        if let Ok(mut c) = self.cache.lock() {
            c.clear();
        }
    }
}

impl Clone for RpfEditSession {
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(BYTE_CACHE_CAPACITY).expect("non-zero cap"),
            )),
            archives: self.archives.clone(),
            sorted: Mutex::new(HashMap::new()),
            crypto: Arc::clone(&self.crypto),
            root_id: self.root_id,
            next_id: self.next_id,
            next_archive_id: self.next_archive_id,
        }
    }
}

impl std::fmt::Debug for RpfEditSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpfEditSession")
            .field("nodes", &self.nodes.len())
            .field("archives", &self.archives.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpfEntryBinding {
    pub parent_doc_id: DocumentId,
    pub entry_id: RpfNodeId,
}

pub struct RpfDocument {
    pub session: RpfEditSession,
    pub expanded: HashSet<u32>,
    pub focused_node: Option<RpfNodeId>,
    pub is_game_root: bool,
    pub entry_documents: HashMap<RpfNodeId, DocumentId>,
}

impl RpfDocument {
    pub fn new(archive: RpfArchive) -> Result<Self, String> {
        let mut expanded = HashSet::new();
        expanded.insert(0);
        Ok(Self {
            session: RpfEditSession::from_archive(archive)?,
            expanded,
            focused_node: None,
            is_game_root: false,
            entry_documents: HashMap::new(),
        })
    }

    pub fn new_game_root(install_root: PathBuf, crypto: Arc<dyn RpfCryptoProvider>) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(0);
        Self {
            session: RpfEditSession::from_game_root(install_root, crypto),
            expanded,
            focused_node: None,
            is_game_root: true,
            entry_documents: HashMap::new(),
        }
    }

    pub fn new_folder(folder: PathBuf, crypto: Arc<dyn RpfCryptoProvider>) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(0);
        Self {
            session: RpfEditSession::from_game_root(folder, crypto),
            expanded,
            focused_node: None,
            is_game_root: false,
            entry_documents: HashMap::new(),
        }
    }
}

impl Clone for RpfDocument {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            expanded: self.expanded.clone(),
            focused_node: self.focused_node,
            is_game_root: self.is_game_root,
            entry_documents: self.entry_documents.clone(),
        }
    }
}

fn normalize_name(name: &str) -> Result<String, String> {
    let mut out = name.replace('\\', "/");
    out = out.trim_matches('/').trim().to_string();
    if out.is_empty() {
        return Err(tr!("Name cannot be empty").to_string());
    }
    if out.contains('/') {
        return Err(tr!("Name cannot contain path separators").to_string());
    }
    Ok(out)
}

fn looks_like_resource(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    bytes[..4] == *b"RSC7"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_registered_once_every_id_is_used_is_refused_and_nothing_is_registered() {
        let bytes = crate::state::nested_archives_survive_a_save::archive(&[("a.txt", b"a")]);
        let archive = RpfArchive::from_bytes(bytes).expect("opens");
        let mut session = RpfEditSession::empty();
        session.next_archive_id = ArchiveId::MAX;

        let refused = session.register_archive(Arc::new(archive), None);
        assert!(refused.is_err(), "{refused:?}");
        assert!(session.archives.is_empty(), "a refused archive was held");
    }

    /// The VFS listing fills the tree: every archive file keeps its entry
    /// index and an empty override slot, and a nested archive waits, unopened,
    /// for a click.
    #[test]
    fn the_tree_is_filled_from_the_vfs_listing() {
        use or_io::container::rpf::RpfBuilder;

        fn archive_of(entries: &[(&str, &[u8])]) -> RpfBuilder {
            let mut builder = RpfBuilder::default();
            for (path, bytes) in entries {
                builder.insert_with(path, *bytes, false).expect("add");
            }
            builder
        }

        let mid = archive_of(&[("mid.ydr", b"two")]).to_bytes().expect("save");
        let archive = archive_of(&[
            ("data/also.ydr", b"one-deeper"),
            ("top.ydr", b"one"),
            ("mid.rpf", &mid),
        ])
        .build()
        .expect("opens");
        let mut session = RpfEditSession::from_archive(archive).expect("session");

        let mut files = 0;
        for id in session.all_nodes() {
            let node = session.node(id).expect("node");
            match &node.backing {
                RpfNodeBacking::ArchiveFile {
                    archive_id,
                    entry_index,
                    override_bytes,
                } => {
                    let slot = session.archive(*archive_id).expect("slot");
                    let named = slot
                        .archive
                        .node_for_entry(*entry_index)
                        .expect("the entry index names an entry");
                    assert_eq!(named.name(), node.name);
                    assert!(override_bytes.is_none());
                    files += 1;
                }
                RpfNodeBacking::Archive {
                    source: ArchiveSource::Nested { opened, .. },
                    scanned,
                    ..
                } => {
                    assert_eq!(node.name, "mid.rpf");
                    assert!(!scanned, "a nested archive waits for a click");
                    assert!(opened.is_none(), "the listing leaves it unopened");
                    assert!(node.children.is_empty());
                }
                _ => {}
            }
        }
        assert_eq!(files, 2, "data/also.ydr and top.ydr");

        let also = session
            .all_nodes()
            .into_iter()
            .find(|&id| session.path_of(id).as_deref() == Some("data/also.ydr"))
            .expect("a folder's files are listed under it");
        assert_eq!(
            session.bytes_for(also).expect("bytes").as_slice(),
            b"one-deeper"
        );
        session
            .set_bytes_for(also, b"edited".to_vec())
            .expect("edit");
        assert_eq!(
            session.bytes_for(also).expect("bytes").as_slice(),
            b"edited"
        );
    }

    #[test]
    fn session_add_rename_move_delete_roundtrip() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let a = session.add_folder(root, "a").expect("add folder a");
        let b = session.add_folder(root, "b").expect("add folder b");
        let file = session
            .add_file_copy(a, "x.ypt", vec![1, 2, 3, 4])
            .expect("add file");
        assert_eq!(session.path_of(file).as_deref(), Some("a/x.ypt"));

        session.rename_node(file, "renamed.ypt").expect("rename");
        assert_eq!(session.path_of(file).as_deref(), Some("a/renamed.ypt"));

        session.move_node(file, b).expect("move");
        assert_eq!(session.path_of(file).as_deref(), Some("b/renamed.ypt"));

        session.delete_node(b).expect("delete folder b");
        assert!(session.node(file).is_none());
    }

    #[test]
    fn session_disallows_duplicate_sibling_names_case_insensitive() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        session.add_folder(root, "Folder").expect("add folder");
        let err = session
            .add_folder(root, "folder")
            .expect_err("duplicate must fail");
        assert!(err.contains("already exists"));
    }

    #[test]
    fn session_disallows_move_into_descendant() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let a = session.add_folder(root, "a").expect("add a");
        let b = session.add_folder(a, "b").expect("add b");
        let err = session
            .move_node(a, b)
            .expect_err("move into descendant must fail");
        assert!(err.contains("descendant"));
    }

    #[test]
    fn in_memory_file_bytes_roundtrip() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let id = session
            .add_file_copy(root, "x.bin", vec![9, 8, 7])
            .expect("add file");
        let bytes = session.bytes_for(id).expect("bytes present");
        assert_eq!(bytes.as_slice(), &[9, 8, 7]);
        assert_eq!(session.size_for(id), Some(3));
    }
}
