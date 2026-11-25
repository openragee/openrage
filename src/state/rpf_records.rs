//! The records an archive is rebuilt from, each archive nested in it written as
//! one file, what still reads an archive from outside it carried before its
//! save or its delete, and the tree pointed at the file an archive's save
//! wrote, or a rename or move on disk put.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::{BodyForm, RpfArchive, RpfEntry};

use super::document::{ArchiveId, RpfNodeId};
use super::rpf_node::{ArchiveSource, RpfNodeBacking, RpfSessionNodeKind};
use super::rpf_session::RpfEditSession;

impl RpfEditSession {
    /// The bytes of the archive at `id`: its file on disk, or its entry in the archive around it.
    pub fn archive_size(&self, id: RpfNodeId) -> Option<u64> {
        if let RpfNodeBacking::Rpf9Archive { table, owner, .. } = &self.nodes.get(&id)?.backing {
            let tree = table.tree();
            return match owner {
                Some((base, index)) => tree
                    .nodes
                    .get(base)?
                    .mount
                    .archive
                    .entries
                    .get(*index as usize)
                    .and_then(|entry| entry.stored),
                None => tree
                    .nodes
                    .get(&0)
                    .map(|node| node.extent.end - node.extent.start),
            };
        }
        let RpfNodeBacking::Archive { source, .. } = &self.nodes.get(&id)?.backing else {
            return None;
        };
        match source {
            ArchiveSource::Disk(path) => Some(std::fs::metadata(path.as_ref()?).ok()?.len()),
            ArchiveSource::Nested {
                parent_archive_id,
                parent_entry_index,
                ..
            } => {
                let parent = self.archives.get(parent_archive_id)?;
                Some(
                    match parent.archive.entries.get(*parent_entry_index as usize)? {
                        RpfEntry::Binary(b) => b.uncompressed_size as u64,
                        RpfEntry::Resource(r) => r.consumed_size as u64,
                        RpfEntry::Directory(_) => 0,
                    },
                )
            }
        }
    }

    /// The files under `root` by their path below it, with each archive nested there as one file and each empty folder as its path and a `/`.
    ///
    /// ⚠ No file's path names an empty folder, so a rebuild without its record drops it.
    pub(super) fn records_under(
        &self,
        root: RpfNodeId,
    ) -> Result<Vec<(String, Vec<u8>, bool)>, String> {
        let root_path = self.path_of(root).unwrap_or_default();

        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            if self.is_rpf9(id) {
                return Err(crate::i18n::tr(super::document::rpf9::entry::READ_ONLY).to_string());
            }
            if id != root {
                if matches!(node.backing, RpfNodeBacking::Archive { .. }) {
                    let path =
                        strip_archive_root_path(&self.path_of(id).unwrap_or_default(), &root_path);
                    out.push((path, self.nested_archive_bytes(id)?, false));
                    continue;
                }
                if !node.is_dir() {
                    let path =
                        strip_archive_root_path(&self.path_of(id).unwrap_or_default(), &root_path);
                    let bytes = self.bytes_for(id).ok_or_else(|| {
                        let named = self.path_of(id).unwrap_or_default();
                        format!("'{named}' could not be read, so nothing was saved")
                    })?;
                    let is_resource = matches!(node.kind, RpfSessionNodeKind::ResourceFile);
                    out.push((path, bytes.as_ref().clone(), is_resource));
                } else if node.children.is_empty() {
                    let path =
                        strip_archive_root_path(&self.path_of(id).unwrap_or_default(), &root_path);
                    out.push((format!("{path}/"), Vec::new(), false));
                }
            }
            for child in &node.children {
                stack.push(*child);
            }
        }
        Ok(out)
    }

    /// A nested archive as the archive holding it stores it: the bytes stored
    /// there, or once anything inside it was edited, a rebuild from its own
    /// records under the encryption it was read with.
    pub(super) fn nested_archive_bytes(&self, id: RpfNodeId) -> Result<Vec<u8>, String> {
        let node = self
            .nodes
            .get(&id)
            .ok_or_else(|| format!("RPF node {id} does not exist"))?;
        let RpfNodeBacking::Archive {
            archive_id,
            source:
                ArchiveSource::Nested {
                    parent_archive_id,
                    parent_entry_index,
                    ..
                },
            ..
        } = &node.backing
        else {
            return Err(format!("'{}' is not nested in an archive", node.name));
        };
        if let Some(slot) = archive_id
            .and_then(|nested| self.archives.get(&nested))
            .filter(|slot| slot.dirty)
        {
            let records = self.records_under(id)?;
            return super::document_io::archive_from_records(
                records,
                slot.archive.header.encryption,
                Arc::clone(&self.crypto),
            )
            .map_err(|e| format!("Rebuild '{}': {e}", node.name));
        }
        self.device(*parent_archive_id)
            .ok_or_else(|| format!("'{}': the archive holding it is not open", node.name))?
            .read_entry(*parent_entry_index, BodyForm::Decoded)
            .map_err(|e| format!("Read '{}': {e}", node.name))
    }

    /// Point every node under the archive `archive_id` at its entry in `written`, the archive a save just wrote over its file.
    ///
    /// ⚠ Until then the session reads the replaced file at the offsets it was
    /// listed with, so a second save writes every entry not held in memory, and
    /// every nested archive, from the wrong bytes.
    pub fn adopt_written_archive(
        &mut self,
        archive_id: ArchiveId,
        written: Arc<RpfArchive>,
    ) -> Result<(), String> {
        let root = self
            .archive_root_node(archive_id)
            .ok_or_else(|| format!("Archive {archive_id} has no root node in this session"))?;
        self.adopt(root, archive_id, written)
    }

    fn adopt(
        &mut self,
        root: RpfNodeId,
        archive_id: ArchiveId,
        written: Arc<RpfArchive>,
    ) -> Result<(), String> {
        let entries: HashMap<String, u32> = written
            .nodes
            .iter()
            .map(|n| (n.path().to_ascii_lowercase(), n.entry_index()))
            .collect();
        let root_path = self.path_of(root).unwrap_or_default();
        if let Some(slot) = self.archives.get_mut(&archive_id) {
            slot.archive = Arc::clone(&written);
        }
        let mut stack = self.children_of(root);
        while let Some(id) = stack.pop() {
            let path = strip_archive_root_path(&self.path_of(id).unwrap_or_default(), &root_path);
            let entry = entries
                .get(&path.to_ascii_lowercase())
                .copied()
                .ok_or_else(|| format!("'{path}' is not in the archive just written"));
            let Some(node) = self.nodes.get_mut(&id) else {
                continue;
            };
            match &mut node.backing {
                RpfNodeBacking::Archive {
                    archive_id: listed,
                    source:
                        ArchiveSource::Nested {
                            parent_archive_id,
                            parent_entry_index,
                            opened,
                        },
                    ..
                } => {
                    let entry = entry?;
                    *parent_archive_id = archive_id;
                    *parent_entry_index = entry;
                    *opened = None;
                    if let Some(nested) = *listed {
                        let reopened = written
                            .open_nested(entry)
                            .map_err(|e| format!("'{path}': {e}"))?;
                        self.adopt(id, nested, Arc::new(reopened))?;
                    }
                    continue;
                }
                RpfNodeBacking::ArchiveFile { .. } | RpfNodeBacking::InMemoryFile { .. } => {
                    let entry = entry?;
                    node.kind = match written.entries.get(entry as usize) {
                        Some(RpfEntry::Resource(_)) => RpfSessionNodeKind::ResourceFile,
                        _ => RpfSessionNodeKind::BinaryFile,
                    };
                    node.backing = RpfNodeBacking::ArchiveFile {
                        archive_id,
                        entry_index: entry,
                        override_bytes: None,
                    };
                }
                RpfNodeBacking::ArchiveDir { archive_id: dir } => *dir = archive_id,
                RpfNodeBacking::InMemoryDir => {
                    node.backing = RpfNodeBacking::ArchiveDir { archive_id }
                }
                _ => {}
            }
            stack.extend(node.children.iter().copied());
        }
        Ok(())
    }

    /// Every row outside the archive `archive_id` still read through it: a file or a nested archive moved out of it, or out of an archive nested in it.
    ///
    /// ⚠ Its save reopens every archive opened under it, a nested archive
    /// moved in among them, so a row moved out of that one reads across the
    /// save too.
    fn readers_outside(&self, archive_id: ArchiveId) -> Vec<RpfNodeId> {
        let Some(root) = self.archive_root_node(archive_id) else {
            return Vec::new();
        };
        let reopened: HashSet<ArchiveId> = self
            .nodes
            .values()
            .filter(|node| self.is_descendant(node.id, root))
            .filter_map(|node| match node.backing {
                RpfNodeBacking::Archive { archive_id, .. } => archive_id,
                _ => None,
            })
            .collect();
        let parents: HashMap<ArchiveId, ArchiveId> = self
            .nodes
            .values()
            .filter_map(|node| match &node.backing {
                RpfNodeBacking::Archive {
                    archive_id: Some(nested),
                    source:
                        ArchiveSource::Nested {
                            parent_archive_id, ..
                        },
                    ..
                } => Some((*nested, *parent_archive_id)),
                _ => None,
            })
            .collect();
        let through = |id: ArchiveId| {
            std::iter::successors(Some(id), |id| parents.get(id).copied())
                .any(|id| reopened.contains(&id))
        };
        self.all_nodes()
            .into_iter()
            .filter(|&id| !self.is_descendant(id, root))
            .filter(|id| match self.nodes.get(id).map(|node| &node.backing) {
                Some(RpfNodeBacking::ArchiveFile {
                    archive_id: read,
                    override_bytes: None,
                    ..
                }) => through(*read),
                Some(RpfNodeBacking::Archive {
                    source:
                        ArchiveSource::Nested {
                            parent_archive_id, ..
                        },
                    ..
                }) => through(*parent_archive_id),
                _ => false,
            })
            .collect()
    }

    /// Why the archive `archive_id` cannot be saved yet: a nested archive moved out of it reads it until the archive now holding it is saved.
    pub fn save_refused(&self, archive_id: ArchiveId) -> Option<String> {
        let nested = self.readers_outside(archive_id).into_iter().find(|id| {
            matches!(
                self.nodes.get(id).map(|node| &node.backing),
                Some(RpfNodeBacking::Archive { .. })
            )
        })?;
        let named = self.path_of(nested).unwrap_or_default();
        let left = self
            .archive_root_node(archive_id)
            .and_then(|root| self.path_of(root))
            .unwrap_or_default();
        Some(format!(
            "'{named}' reads '{left}', which it was moved out of, until the archive now \
             holding it is saved: save that one first"
        ))
    }

    /// Read into memory every file outside the archive `archive_id` that is still read through it, before a save replaces it.
    ///
    /// ⚠ A row moved out of an archive reads it at the entry it was listed at,
    /// and once the save points the archive at the file it wrote, that entry is
    /// another file's or none. A nested archive moved out has no bytes of its
    /// own to hold, so the save is refused until the archive now holding it is
    /// saved.
    pub fn carry_readers_of(&mut self, archive_id: ArchiveId) -> Result<(), String> {
        if let Some(refused) = self.save_refused(archive_id) {
            return Err(refused);
        }
        for id in self.readers_outside(archive_id) {
            let bytes = self.bytes_for(id).ok_or_else(|| {
                let named = self.path_of(id).unwrap_or_default();
                format!("'{named}' could not be read, so nothing was saved")
            })?;
            if let Some(RpfNodeBacking::ArchiveFile { override_bytes, .. }) =
                self.nodes.get_mut(&id).map(|node| &mut node.backing)
            {
                *override_bytes = Some(bytes);
            }
        }
        Ok(())
    }

    /// The archives opened at or under the row `node_id`.
    pub fn archives_opened_under(&self, node_id: RpfNodeId) -> Vec<ArchiveId> {
        self.all_nodes()
            .into_iter()
            .filter(|&id| self.is_descendant(id, node_id))
            .filter_map(|id| match self.nodes.get(&id)?.backing {
                RpfNodeBacking::Archive { archive_id, .. } => archive_id,
                _ => None,
            })
            .collect()
    }

    /// Read into memory every file that still reads an archive opened at or under the row `node_id` from outside it, before the row goes.
    pub fn carry_readers_under(&mut self, node_id: RpfNodeId) -> Result<(), String> {
        self.archives_opened_under(node_id)
            .into_iter()
            .try_for_each(|id| self.carry_readers_of(id))
    }

    /// Follow the row `node_id` and every row under it from `from` to `to`, where a rename or move on disk put them, keeping every edit.
    ///
    /// ⚠ An archive opened from a row reads, and saves to, the path it was
    /// opened from, so after a rename its save reads a file that is gone.
    pub fn moved_on_disk(&mut self, node_id: RpfNodeId, from: &Path, to: &Path) {
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get_mut(&id) else {
                continue;
            };
            let Some(rest) = node
                .backing
                .fs_path()
                .and_then(|p| p.strip_prefix(from).ok())
            else {
                continue;
            };
            let moved = if rest.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(rest)
            };
            if matches!(
                node.backing,
                RpfNodeBacking::Rpf9Archive { owner: None, .. }
            ) {
                self.unload_archive(id);
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.backing.set_fs_path(moved);
                }
                continue;
            }
            node.backing.set_fs_path(moved.clone());
            match node.backing {
                RpfNodeBacking::Archive {
                    archive_id: Some(archive_id),
                    ..
                } => self.reopen_at(archive_id, moved),
                RpfNodeBacking::Archive { .. } => {}
                _ => stack.extend(node.children.iter().copied()),
            }
        }
    }

    /// Point the archive `archive_id`, and every archive opened inside it, at its file, now at `path`.
    fn reopen_at(&mut self, archive_id: ArchiveId, path: PathBuf) {
        let Some(slot) = self.archives.get_mut(&archive_id) else {
            return;
        };
        let moved = RpfArchive::open_with(&path, Arc::clone(&self.crypto));
        slot.fs_path = Some(path);
        match moved {
            Ok(moved) => self.reparent(archive_id, Arc::new(moved)),
            Err(error) => {
                tracing::warn!(archive_id, %error, "an archive moved on disk does not reopen");
            }
        }
    }

    /// Give the archive `archive_id` the handle `archive`, and each archive opened inside it one that reads through it.
    ///
    /// ⚠ Wherever its row now sits: a nested archive moved out of this one
    /// reads through it until the archive now holding it is saved.
    fn reparent(&mut self, archive_id: ArchiveId, archive: Arc<RpfArchive>) {
        if let Some(slot) = self.archives.get_mut(&archive_id) {
            slot.archive = Arc::clone(&archive);
        }
        let readers: Vec<(RpfNodeId, u32)> = (self.nodes.values())
            .filter_map(|node| match node.backing {
                RpfNodeBacking::Archive {
                    source:
                        ArchiveSource::Nested {
                            parent_archive_id,
                            parent_entry_index,
                            ..
                        },
                    ..
                } if parent_archive_id == archive_id => Some((node.id, parent_entry_index)),
                _ => None,
            })
            .collect();
        for (id, entry) in readers {
            let Some(RpfNodeBacking::Archive {
                archive_id: listed,
                source: ArchiveSource::Nested { opened, .. },
                ..
            }) = self.nodes.get_mut(&id).map(|node| &mut node.backing)
            else {
                continue;
            };
            *opened = None;
            let Some(nested) = *listed else {
                continue;
            };
            if !self.archives.contains_key(&nested) {
                continue;
            }
            match archive.open_nested(entry) {
                Ok(inner) => self.reparent(nested, Arc::new(inner)),
                Err(error) => {
                    tracing::warn!(nested, %error, "a nested archive does not reopen in its parent");
                }
            }
        }
    }
}

fn strip_archive_root_path(full: &str, root: &str) -> String {
    if root.is_empty() {
        return full.to_string();
    }
    if let Some(rest) = full.strip_prefix(root) {
        rest.trim_start_matches('/').to_string()
    } else {
        full.to_string()
    }
}
