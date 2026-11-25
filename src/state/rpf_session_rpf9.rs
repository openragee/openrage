use super::*;
use crate::state::document::rpf9::{is_archive, Rpf9TableDocument};
use or_io::container::rpf::Gta6EntryType;

impl RpfEditSession {
    /// The table already mounted for an archive's own disk path.
    pub(crate) fn rpf9_table(&self, path: &std::path::Path) -> Option<&Arc<Rpf9TableDocument>> {
        self.nodes.values().find_map(|node| match &node.backing {
            RpfNodeBacking::Rpf9Archive {
                table, owner: None, ..
            } if table.path == path => Some(table),
            _ => None,
        })
    }

    pub fn is_rpf9(&self, node_id: RpfNodeId) -> bool {
        std::iter::successors(self.nodes.get(&node_id), |node| {
            node.parent.and_then(|parent| self.nodes.get(&parent))
        })
        .any(|node| {
            matches!(
                node.backing,
                RpfNodeBacking::Rpf9Archive { .. }
                    | RpfNodeBacking::Rpf9Dir { .. }
                    | RpfNodeBacking::Rpf9File { .. }
            )
        })
    }

    pub(super) fn ensure_not_rpf9(&self, node_id: RpfNodeId) -> Result<(), String> {
        if self.is_rpf9(node_id) {
            return Err(
                crate::i18n::tr(crate::state::document::rpf9::entry::READ_ONLY).to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn rpf9_size_for(&self, id: RpfNodeId) -> Option<u64> {
        let (table, base, index) = match &self.node(id)?.backing {
            RpfNodeBacking::Rpf9File { table, base, index }
            | RpfNodeBacking::Rpf9Archive {
                table,
                owner: Some((base, index)),
                ..
            } => (table, base, index),
            _ => return None,
        };
        let tree = table.tree();
        let archive = &tree.nodes.get(base)?.mount.archive;
        let entry = archive.entries.get(*index as usize)?;
        entry.real.or_else(|| {
            archive
                .entry_extra(*index as usize)?
                .nested
                .map(|row| row.stored_size())
        })
    }

    pub(super) fn rpf9_bytes_for(&self, id: RpfNodeId) -> Option<Arc<Vec<u8>>> {
        let RpfNodeBacking::Rpf9File { table, base, index } = &self.node(id)?.backing else {
            return None;
        };
        match table.entry(*base, *index).and_then(|entry| entry.bytes()) {
            Ok(bytes) => Some(Arc::new(bytes)),
            Err(error) => {
                tracing::warn!(node_id = id, error = %error, "RPF9 table entry read failed");
                None
            }
        }
    }

    pub(super) fn unload_rpf9_table(&mut self, id: RpfNodeId) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if let RpfNodeBacking::Rpf9Archive {
            table,
            owner,
            opened_base,
            open_error,
        } = &mut node.backing
        {
            if owner.is_none() {
                let path = table.path.clone();
                node.backing = RpfNodeBacking::Archive {
                    archive_id: None,
                    source: ArchiveSource::Disk(Some(path)),
                    scanned: false,
                    open_error: None,
                    recovered_hint: None,
                };
            } else {
                *opened_base = None;
                *open_error = None;
            }
        }
    }

    /// Populate known paths and root-level hashes without changing the archive's entry indices.
    pub fn populate_rpf9_children(
        &mut self,
        parent_id: RpfNodeId,
        table: Arc<Rpf9TableDocument>,
        base: u64,
    ) -> Result<(), String> {
        let parent = self.nodes.get(&parent_id).ok_or_else(|| {
            tr!(
                "Parent folder {parent_id} does not exist",
                parent_id = parent_id
            )
        })?;
        if !parent.is_dir() {
            return Err(tr!(
                "Parent {parent_id} is not a folder",
                parent_id = parent_id
            ));
        }
        if !parent.children.is_empty() {
            return Ok(());
        }
        let rows = {
            let tree = table.tree();
            let archive = &tree
                .nodes
                .get(&base)
                .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())?
                .mount
                .archive;
            archive
                .entries
                .iter()
                .map(|entry| {
                    let path = entry
                        .name
                        .as_deref()
                        .and_then(safe_path)
                        .unwrap_or_else(|| vec![format!("hash_{:016x}", entry.name64)]);
                    (
                        entry.index,
                        path,
                        is_archive(entry),
                        tree.entry_type(base, entry.index).0,
                    )
                })
                .collect::<Vec<_>>()
        };
        // Folders are allocated first, so an entry cannot shadow a known path's directory.
        let mut directories = HashMap::new();
        for (_, path, _, _) in &rows {
            let mut parent = parent_id;
            for component in &path[..path.len() - 1] {
                let key = (parent, component.to_ascii_lowercase());
                parent = *directories.entry(key).or_insert_with(|| {
                    self.insert_rpf9_node(
                        parent,
                        component.clone(),
                        RpfSessionNodeKind::Directory,
                        RpfNodeBacking::Rpf9Dir {
                            table: Arc::clone(&table),
                        },
                    )
                });
            }
        }
        let mut names = directories.keys().cloned().collect::<HashSet<_>>();
        for (index, path, archive, kind) in rows {
            let mut parent = parent_id;
            for component in &path[..path.len() - 1] {
                parent = directories[&(parent, component.to_ascii_lowercase())];
            }
            let name = path.last().cloned().unwrap_or_default();
            let mut unique = name.clone();
            let mut ordinal = 0u64;
            while !names.insert((parent, unique.to_ascii_lowercase())) {
                unique = if ordinal == 0 {
                    format!("{name} [{index}]")
                } else {
                    format!("{name} [{index}-{ordinal}]")
                };
                ordinal += 1;
            }
            let (kind, backing) = if archive {
                (
                    RpfSessionNodeKind::Directory,
                    RpfNodeBacking::Rpf9Archive {
                        table: Arc::clone(&table),
                        owner: Some((base, index)),
                        opened_base: None,
                        open_error: None,
                    },
                )
            } else {
                let kind = match kind {
                    Gta6EntryType::TextureDictionary | Gta6EntryType::Resource(_) => {
                        RpfSessionNodeKind::ResourceFile
                    }
                    _ => RpfSessionNodeKind::BinaryFile,
                };
                (
                    kind,
                    RpfNodeBacking::Rpf9File {
                        table: Arc::clone(&table),
                        base,
                        index,
                    },
                )
            };
            self.insert_rpf9_node(parent, unique, kind, backing);
        }
        Ok(())
    }

    fn insert_rpf9_node(
        &mut self,
        parent: RpfNodeId,
        name: String,
        kind: RpfSessionNodeKind,
        backing: RpfNodeBacking,
    ) -> RpfNodeId {
        let id = self.alloc_id();
        self.nodes.insert(
            id,
            RpfSessionNode {
                id,
                parent: Some(parent),
                name,
                kind,
                children: Vec::new(),
                backing,
            },
        );
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.push(id);
        }
        id
    }
}

fn safe_path(name: &str) -> Option<Vec<String>> {
    let normalized = name.replace('\\', "/");
    let parts = normalized.split('/').map(str::to_owned).collect::<Vec<_>>();
    parts
        .iter()
        .all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part
                    .chars()
                    .any(|c| c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
                && part.trim() == part
        })
        .then_some(parts)
}
