//! An RPF9 document lists its table and mounts child tables only when expanded.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, Metadata};
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};

use or_io::container::rpf::{
    open_at_bounded, Gta6Archive, Gta6Class, Gta6Entry, GtaViKeys, NameResolver, RPF9_HEADER_SIZE,
    RPF9_MAGIC,
};
use or_io::gta6::names::{vocabulary_file, Vocabulary};

#[path = "rpf9_entry.rs"]
pub mod entry;

pub struct Node {
    pub archive: Gta6Archive,
    pub extent: Range<u64>,
    depth: usize,
    chain: Vec<u32>,
}

struct Pending {
    entry: (u64, u32),
    result: mpsc::Receiver<Result<Node, String>>,
}

#[derive(Default)]
pub struct Tree {
    pub nodes: BTreeMap<u64, Node>,
    pub expanded: BTreeSet<(u64, u32)>,
    pub errors: BTreeMap<(u64, u32), String>,
    pub selected: Option<(u64, u32)>,
    pub filter: String,
    pending: Option<Pending>,
}

impl Tree {
    pub fn pending(&self) -> Option<(u64, u32)> {
        self.pending.as_ref().map(|p| p.entry)
    }

    pub fn poll(&mut self) {
        let Some(pending) = &self.pending else { return };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(tr!("The archive table could not be loaded.").to_string())
            }
        };
        let entry = pending.entry;
        self.pending = None;
        match result {
            Ok(node) => {
                self.nodes.insert(node.extent.start, node);
                self.expanded.insert(entry);
            }
            Err(error) => {
                self.errors.insert(entry, error);
            }
        }
    }
}

pub struct Rpf9TableDocument {
    pub path: PathBuf,
    keys: Arc<GtaViKeys>,
    resolver: Arc<NameResolver>,
    metadata: Metadata,
    tree: Mutex<Tree>,
}

impl Rpf9TableDocument {
    /// Recognise RPF9 by its magic, then open its table or return the error without recovery.
    pub fn open(path: &Path) -> Result<Option<Self>, String> {
        Self::open_using(path, || {
            GtaViKeys::for_install(or_title::config::config_dir().as_deref())
                .map_err(|e| e.to_string())?
                .ok_or_else(|| tr!("Opening a GTA VI archive requires gtavi_magic.dat in the configuration folder or OPENRAGE_GTA6_KEYS.").to_string())
        })
    }

    fn open_using(
        path: &Path,
        keys: impl FnOnce() -> Result<GtaViKeys, String>,
    ) -> Result<Option<Self>, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut magic = [0; 4];
        if file.metadata().map_err(|e| e.to_string())?.len() < magic.len() as u64 {
            return Ok(None);
        }
        file.read_exact(&mut magic).map_err(|e| e.to_string())?;
        if magic != RPF9_MAGIC.to_le_bytes() {
            return Ok(None);
        }
        let keys = Arc::new(keys()?);
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let resolver = Arc::new(load_resolver());
        let mut archive =
            open_at_bounded(&mut file, 0..metadata.len(), &keys).map_err(|e| e.to_string())?;
        archive.resolve_names(&resolver);
        let mut tree = Tree::default();
        tree.nodes.insert(
            0,
            Node {
                archive,
                extent: 0..metadata.len(),
                depth: 0,
                chain: Vec::new(),
            },
        );
        Ok(Some(Self {
            path: path.to_path_buf(),
            keys,
            resolver,
            metadata,
            tree: Mutex::new(tree),
        }))
    }

    pub fn tree(&self) -> MutexGuard<'_, Tree> {
        self.tree.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn job(&self) -> Option<crate::state::jobs::RunningJob> {
        self.tree().pending()?;
        Some(crate::state::jobs::RunningJob {
            kind: crate::state::jobs::JobKind::ArchiveTable,
            doing: tr!("Loading archive table…").to_string(),
            fraction: None,
            detail: crate::state::jobs::file_name(&self.path),
            stopping: false,
        })
    }

    pub fn stop(&self) {
        // The worker finishes its bounded table read, but its result is discarded.
        self.tree().pending = None;
    }

    pub fn held_bytes(&self) -> usize {
        self.tree()
            .nodes
            .values()
            .map(|node| {
                node.archive.toc.bytes.len()
                    + std::mem::size_of_val(node.archive.entries.as_slice())
                    + node
                        .archive
                        .entries
                        .iter()
                        .filter_map(|entry| entry.name.as_ref())
                        .map(String::len)
                        .sum::<usize>()
            })
            .sum()
    }

    /// Toggle a child archive, loading its table on a worker on the first expansion.
    pub fn toggle(&self, base: u64, index: u32) {
        let mut tree = self.tree();
        let key = (base, index);
        if tree.expanded.remove(&key) {
            return;
        }
        let Some(parent) = tree.nodes.get(&base) else {
            return;
        };
        let extent = match child_extent(parent, index) {
            Ok(extent) => extent,
            Err(error) => {
                tree.errors.insert(key, error);
                return;
            }
        };
        let depth = parent.depth + 1;
        let mut chain = parent.chain.clone();
        chain.push(index);
        if let Some(node) = tree.nodes.get(&extent.start) {
            if node.extent == extent {
                tree.expanded.insert(key);
            } else {
                tree.errors.insert(
                    key,
                    tr!("This entry does not describe a bounded nested archive.").to_string(),
                );
            }
            return;
        }
        if tree.pending.is_some() {
            return;
        }
        tree.errors.remove(&key);
        let (tx, result) = mpsc::channel();
        let (path, keys, resolver, metadata) = (
            self.path.clone(),
            Arc::clone(&self.keys),
            Arc::clone(&self.resolver),
            self.metadata.clone(),
        );
        tree.pending = Some(Pending { entry: key, result });
        std::thread::spawn(move || {
            let loaded = (|| {
                let mut file = File::open(&path).map_err(|e| e.to_string())?;
                let now = file.metadata().map_err(|e| e.to_string())?;
                if now.len() != metadata.len() || now.modified().ok() != metadata.modified().ok() {
                    return Err(
                        tr!("The archive changed on disk. Reopen it to read its table.")
                            .to_string(),
                    );
                }
                let mut archive =
                    open_at_bounded(&mut file, extent.clone(), &keys).map_err(|e| e.to_string())?;
                archive.resolve_names(&resolver);
                Ok(Node {
                    archive,
                    extent,
                    depth,
                    chain,
                })
            })();
            let _ = tx.send(loaded);
        });
    }
}

pub fn is_archive(entry: &Gta6Entry) -> bool {
    entry.kind() == Some(Gta6Class::NestedArchive)
}

fn child_extent(parent: &Node, index: u32) -> Result<Range<u64>, String> {
    let invalid = || tr!("This entry does not describe a bounded nested archive.").to_string();
    let entry = parent
        .archive
        .entries
        .get(index as usize)
        .ok_or_else(invalid)?;
    if entry.cipher.is_some() {
        return Err(tr!("This nested archive is encrypted and cannot be expanded.").to_string());
    }
    if parent.depth >= 32 {
        return Err(tr!("The archive nesting limit was reached.").to_string());
    }
    let relative = entry.start.ok_or_else(invalid)?;
    let size = entry
        .stored
        .or_else(|| {
            parent
                .archive
                .entry_extra(index as usize)?
                .nested
                .map(|row| row.stored_size())
        })
        .ok_or_else(invalid)?;
    if relative < parent.archive.toc.bytes.len() as u64 || size < RPF9_HEADER_SIZE as u64 {
        return Err(invalid());
    }
    let start = parent
        .extent
        .start
        .checked_add(relative)
        .ok_or_else(invalid)?;
    let end = start
        .checked_add(size)
        .filter(|end| *end <= parent.extent.end)
        .ok_or_else(invalid)?;
    Ok(start..end)
}

fn load_resolver() -> NameResolver {
    let mut resolver = std::env::var_os("OPENRAGE_GTA6_NAMES")
        .map(PathBuf::from)
        .map(|dir| NameResolver::load(&dir))
        .unwrap_or_default();
    if let Some(config) = or_title::config::config_dir() {
        resolver.merge_vocabulary(&Vocabulary::load(&vocabulary_file(&config)));
    }
    resolver
}

#[cfg(test)]
#[path = "rpf9_tests.rs"]
mod tests;
