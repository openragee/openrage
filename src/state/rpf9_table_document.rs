//! An RPF9 document lists its table and mounts child tables only when expanded.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, Metadata};
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};

use crate::state::rpf9_scan::rpf9_table::{self, TableKind, TableNode, TableRow};
use crate::state::{AppState, DocPayload};
use or_io::container::rpf::{
    open_at_bounded, Gta6Class, Gta6Entry, Gta6EntryType, Gta6Mount, GtaViKeys, NameResolver,
    Rpf9Header, RPF9_HEADER_SIZE, RPF9_MAGIC,
};

#[path = "document/rpf9_entry.rs"]
pub mod entry;

pub struct Node {
    pub mount: Gta6Mount,
    pub extent: Range<u64>,
    depth: usize,
    chain: Vec<u32>,
    headers: BTreeMap<u32, Gta6EntryType>,
}

impl Node {
    fn open(
        file: &mut File,
        extent: Range<u64>,
        depth: usize,
        chain: Vec<u32>,
        keys: &GtaViKeys,
        resolver: &NameResolver,
        read_headers: bool,
    ) -> Result<Self, String> {
        let mut archive = open_at_bounded(file, extent.clone(), keys).map_err(|e| e.to_string())?;
        archive.resolve_names(resolver);
        let mount = Gta6Mount {
            base: extent.start,
            archive,
        };
        let mut headers = BTreeMap::new();
        for entry in mount.archive.entries.iter().filter(|_| read_headers) {
            let Some(extra) = mount.archive.entry_extra(entry.index as usize) else {
                continue;
            };
            if Gta6EntryType::hint(entry, &extra) != Gta6EntryType::Unknown {
                continue;
            }
            let header = mount
                .read_entry_plan(entry.index, extent.end, u64::MAX)
                .and_then(|read| read.read_header(file, keys));
            if let Ok(Some(header)) = header {
                if let Some(kind) = Gta6EntryType::from_header(&header) {
                    headers.insert(entry.index, kind);
                }
            }
        }
        Ok(Self {
            mount,
            extent,
            depth,
            chain,
            headers,
        })
    }
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
    pub(crate) revision: u64,
    pending: Option<Pending>,
    detected: BTreeMap<(u64, u32), Gta6EntryType>,
}

impl Tree {
    pub fn entry_type(&self, base: u64, index: u32) -> (Gta6EntryType, bool) {
        let hint = self
            .nodes
            .get(&base)
            .and_then(|node| {
                let entry = node.mount.archive.entries.get(index as usize)?;
                let extra = node.mount.archive.entry_extra(index as usize)?;
                Some(Gta6EntryType::hint(entry, &extra))
            })
            .unwrap_or(Gta6EntryType::Unknown);
        if let Some(kind) = self.detected.get(&(base, index)) {
            // The container signature can be shared by several distinct table file types.
            return (
                if hint == Gta6EntryType::Unknown {
                    *kind
                } else {
                    hint
                },
                *kind != Gta6EntryType::Unknown
                    && (hint == Gta6EntryType::Unknown || *kind == hint),
            );
        }
        if let Some(kind) = self
            .nodes
            .get(&base)
            .and_then(|node| node.headers.get(&index))
        {
            return (*kind, true);
        }
        (hint, false)
    }

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
        self.revision = self.revision.wrapping_add(1);
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

#[derive(Clone)]
struct TableContext {
    keys: Arc<GtaViKeys>,
    resolver: Arc<NameResolver>,
}

impl TableContext {
    fn configured() -> Result<Self, String> {
        Ok(Self {
            keys: rpf9_table::keys()?,
            resolver: rpf9_table::resolver(),
        })
    }
}

/// The configured keys and names shared by file roots of one catalog, loaded only after an RPF9 header is recognized.
#[derive(Default)]
pub(crate) struct MetadataOpener {
    context: Option<Result<TableContext, String>>,
}

impl MetadataOpener {
    pub fn open(&mut self, path: &Path) -> Result<Option<Rpf9TableDocument>, String> {
        self.open_using(path, TableContext::configured)
    }

    fn open_using(
        &mut self,
        path: &Path,
        context: impl FnOnce() -> Result<TableContext, String>,
    ) -> Result<Option<Rpf9TableDocument>, String> {
        Rpf9TableDocument::open_with_context(
            path,
            || self.context.get_or_insert_with(context).clone(),
            false,
        )
    }
}

pub struct Rpf9TableDocument {
    pub path: PathBuf,
    pub header: Rpf9Header,
    keys: Arc<GtaViKeys>,
    resolver: Arc<NameResolver>,
    metadata: Metadata,
    opened_at: std::time::Instant,
    tree: Arc<Mutex<Tree>>,
    read_headers: bool,
}

impl std::fmt::Debug for Rpf9TableDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rpf9TableDocument")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Rpf9TableDocument {
    /// Recognise RPF9 by its magic, then open its table or return the error without recovery.
    pub fn open(path: &Path) -> Result<Option<Self>, String> {
        Self::open_with_context(path, TableContext::configured, true)
    }

    #[cfg(test)]
    pub(crate) fn open_using(
        path: &Path,
        keys: impl FnOnce() -> Result<GtaViKeys, String>,
    ) -> Result<Option<Self>, String> {
        Self::open_with_headers(path, keys, true)
    }

    #[cfg(test)]
    pub(crate) fn open_using_names(
        path: &Path,
        keys: impl FnOnce() -> Result<GtaViKeys, String>,
        resolver: NameResolver,
    ) -> Result<Option<Self>, String> {
        Self::open_with_context(
            path,
            || {
                Ok(TableContext {
                    keys: Arc::new(keys()?),
                    resolver: Arc::new(resolver),
                })
            },
            true,
        )
    }

    #[cfg(test)]
    fn open_with_headers(
        path: &Path,
        keys: impl FnOnce() -> Result<GtaViKeys, String>,
        read_headers: bool,
    ) -> Result<Option<Self>, String> {
        Self::open_with_context(
            path,
            || {
                Ok(TableContext {
                    keys: Arc::new(keys()?),
                    resolver: rpf9_table::resolver(),
                })
            },
            read_headers,
        )
    }

    fn open_with_context(
        path: &Path,
        context: impl FnOnce() -> Result<TableContext, String>,
        read_headers: bool,
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
        let TableContext { keys, resolver } = context()?;
        let opened_at = std::time::Instant::now();
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let node = Node::open(
            &mut file,
            0..metadata.len(),
            0,
            Vec::new(),
            &keys,
            &resolver,
            read_headers,
        )?;
        let header = Rpf9Header::parse(&node.mount.archive.toc.bytes).map_err(|e| e.to_string())?;
        let mut tree = Tree::default();
        tree.nodes.insert(0, node);
        let opened = Self {
            path: path.to_path_buf(),
            header,
            keys,
            resolver,
            metadata,
            opened_at,
            tree: Arc::new(Mutex::new(tree)),
            read_headers,
        };
        opened.check_current()?;
        Ok(Some(opened))
    }

    /// Clone the root table into an independent metadata catalog whose children never change the UI's tree.
    pub(crate) fn detached_metadata(&self) -> Result<Self, String> {
        self.check_current()?;
        let root = {
            let tree = self.tree();
            let root = tree
                .nodes
                .get(&0)
                .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())?;
            Node {
                mount: root.mount.clone(),
                extent: root.extent.clone(),
                depth: root.depth,
                chain: root.chain.clone(),
                headers: BTreeMap::new(),
            }
        };
        let mut tree = Tree::default();
        tree.nodes.insert(0, root);
        Ok(Self {
            path: self.path.clone(),
            header: self.header,
            keys: Arc::clone(&self.keys),
            resolver: Arc::clone(&self.resolver),
            metadata: self.metadata.clone(),
            opened_at: self.opened_at,
            tree: Arc::new(Mutex::new(tree)),
            read_headers: false,
        })
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
                node.mount.archive.toc.bytes.len()
                    + std::mem::size_of_val(node.mount.archive.entries.as_slice())
                    + node
                        .mount
                        .archive
                        .entries
                        .iter()
                        .filter_map(|entry| entry.name.as_ref())
                        .map(String::len)
                        .sum::<usize>()
            })
            .sum()
    }

    /// Reuse this snapshot or open its replacement without changing existing entry sources.
    pub(crate) fn current_or_reopen(self: &Arc<Self>) -> Result<Arc<Self>, String> {
        if self.check_current().is_ok() {
            return Ok(Arc::clone(self));
        }
        Self::open_with_context(
            &self.path,
            || {
                Ok(TableContext {
                    keys: Arc::clone(&self.keys),
                    resolver: Arc::clone(&self.resolver),
                })
            },
            self.read_headers,
        )?
        .map(Arc::new)
        .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())
    }

    pub(crate) fn check_current(&self) -> Result<(), String> {
        let now = std::fs::metadata(&self.path).map_err(|e| e.to_string())?;
        if now.len() != self.metadata.len() || now.modified().ok() != self.metadata.modified().ok()
        {
            return Err(
                tr!("The archive changed on disk. Reopen it to read its table.").to_string(),
            );
        }
        Ok(())
    }

    /// Load a bounded child table synchronously on an archive expansion worker.
    pub fn expand_table(&self, base: u64, index: u32) -> Result<u64, String> {
        self.check_current()?;
        let (extent, depth, chain) = {
            let tree = self.tree();
            let parent = tree
                .nodes
                .get(&base)
                .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())?;
            if !parent
                .mount
                .archive
                .entries
                .get(index as usize)
                .is_some_and(is_archive)
            {
                return Err(
                    tr!("This entry does not describe a bounded nested archive.").to_string(),
                );
            }
            let extent = child_extent(parent, index)?;
            if let Some(node) = tree.nodes.get(&extent.start) {
                return if node.extent == extent {
                    Ok(extent.start)
                } else {
                    Err(tr!("This entry does not describe a bounded nested archive.").to_string())
                };
            }
            let mut chain = parent.chain.clone();
            chain.push(index);
            (extent, parent.depth + 1, chain)
        };
        let loaded: Result<Node, String> = (|| {
            let mut file = File::open(&self.path).map_err(|e| e.to_string())?;
            let check = |file: &File| -> Result<(), String> {
                let now = file.metadata().map_err(|e| e.to_string())?;
                if now.len() != self.metadata.len()
                    || now.modified().ok() != self.metadata.modified().ok()
                {
                    return Err(
                        tr!("The archive changed on disk. Reopen it to read its table.")
                            .to_string(),
                    );
                }
                Ok(())
            };
            check(&file)?;
            let node = Node::open(
                &mut file,
                extent.clone(),
                depth,
                chain,
                &self.keys,
                &self.resolver,
                self.read_headers,
            )?;
            check(&file)?;
            Ok(node)
        })();
        let mut tree = self.tree();
        match loaded {
            Ok(node) => {
                if tree
                    .nodes
                    .get(&extent.start)
                    .is_some_and(|old| old.extent != extent)
                {
                    return Err(
                        tr!("This entry does not describe a bounded nested archive.").to_string(),
                    );
                }
                tree.nodes.entry(extent.start).or_insert(node);
                tree.revision = tree.revision.wrapping_add(1);
                tree.errors.remove(&(base, index));
                Ok(extent.start)
            }
            Err(error) => {
                tree.errors.insert((base, index), error.clone());
                Err(error)
            }
        }
    }

    /// Toggle a child archive, loading its table on a worker on the first expansion.
    pub fn toggle(&self, base: u64, index: u32) {
        let mut tree = self.tree();
        let key = (base, index);
        tree.revision = tree.revision.wrapping_add(1);
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
        let read_headers = self.read_headers;
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
                Node::open(
                    &mut file,
                    extent,
                    depth,
                    chain,
                    &keys,
                    &resolver,
                    read_headers,
                )
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
        .mount
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
                .mount
                .archive
                .entry_extra(index as usize)?
                .nested
                .map(|row| row.stored_size())
        })
        .ok_or_else(invalid)?;
    if relative < parent.mount.archive.toc.bytes.len() as u64 || size < RPF9_HEADER_SIZE as u64 {
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

impl Tree {
    pub(crate) fn listing(&self) -> Option<TableNode> {
        self.listing_at(0)
    }

    fn listing_at(&self, base: u64) -> Option<TableNode> {
        let node = self.nodes.get(&base)?;
        let archive = &node.mount.archive;
        let mut shown = TableNode {
            key_id: archive.toc.key_id,
            ..TableNode::default()
        };
        for entry in &archive.entries {
            let kind = self.entry_type(base, entry.index).0;
            let is_archive = kind == Gta6EntryType::Archive;
            let absolute_start = entry.start.and_then(|start| base.checked_add(start));
            let nested = archive
                .entry_extra(entry.index as usize)
                .and_then(|extra| extra.nested);
            shown.rows.push(TableRow {
                index: entry.index,
                name64: entry.name64,
                name32: entry.name32,
                name: Some(
                    crate::state::files_view::rpf9_display_name(&entry.display_name(), kind)
                        .into_owned(),
                ),
                size: entry.size().or_else(|| {
                    nested
                        .map(|row| row.stored_size())
                        .and_then(|size| u32::try_from(size).ok())
                }),
                is_archive,
                cipher: entry.cipher.is_some(),
                kind: TableKind::of(is_archive, entry.kind(), nested.is_some()),
            });
            if is_archive {
                if let Some(child) = absolute_start.and_then(|start| self.listing_at(start)) {
                    shown.children.insert(entry.index, child);
                }
                if self.expanded.contains(&(base, entry.index)) {
                    shown.open.insert(entry.index);
                }
            }
        }
        Some(shown)
    }

    pub(crate) fn kind_labels(&self) -> BTreeMap<Vec<u32>, Vec<String>> {
        self.nodes
            .values()
            .map(|node| {
                let labels = or_io::container::rpf::gta6_kinds::entry_kinds(&node.mount.archive)
                    .into_iter()
                    .map(|kind| kind.into_owned())
                    .collect();
                (node.chain.clone(), labels)
            })
            .collect()
    }

    pub(crate) fn base_of(&self, indices: &[u32]) -> Option<u64> {
        self.nodes
            .iter()
            .find_map(|(&base, node)| (node.chain == indices).then_some(base))
    }
}

impl Rpf9TableDocument {
    pub(crate) fn toggle_path(&self, indices: &[u32]) {
        let Some((&index, parent)) = indices.split_last() else {
            return;
        };
        let base = self.tree().base_of(parent);
        if let Some(base) = base {
            self.toggle(base, index);
        }
    }
}

/// An RPF9 opens from its bounded table; a missing key or invalid table stays an error.
pub(crate) fn archive_payload(path: &Path) -> Result<Option<DocPayload>, String> {
    archive_payload_with(path, TableContext::configured)
}

fn archive_payload_with(
    path: &Path,
    context: impl FnOnce() -> Result<TableContext, String>,
) -> Result<Option<DocPayload>, String> {
    Rpf9TableDocument::open_with_context(path, context, true)
        .map(|table| table.map(|table| DocPayload::Rpf9Table(Arc::new(table))))
}

impl AppState {
    /// A racing open of the same file snapshot keeps the table already shown by the app.
    pub(crate) fn share_rpf9_table(
        &self,
        incoming: Arc<Rpf9TableDocument>,
    ) -> Arc<Rpf9TableDocument> {
        let held = self
            .mounted_rpf9_tables(&incoming.path)
            .filter(|held| !Arc::ptr_eq(held, &incoming))
            .max_by_key(|table| table.opened_at)
            .filter(|held| {
                (held.metadata.len() == incoming.metadata.len()
                    && held.metadata.modified().ok() == incoming.metadata.modified().ok())
                    || held.opened_at > incoming.opened_at
            })
            .cloned();
        held.unwrap_or(incoming)
    }

    pub(crate) fn mounted_rpf9_table(&self, path: &Path) -> Option<Arc<Rpf9TableDocument>> {
        self.mounted_rpf9_tables(path)
            .max_by_key(|table| table.opened_at)
            .cloned()
    }

    fn mounted_rpf9_tables<'a>(
        &'a self,
        path: &'a Path,
    ) -> impl Iterator<Item = &'a Arc<Rpf9TableDocument>> + 'a {
        let explorer = self
            .panels
            .peek::<crate::ui::panels::gta6_archive_tree::Gta6ArchiveExplorer>()
            .and_then(|explorer| explorer.table.as_ref())
            .filter(|table| table.path == path);
        self.documents
            .iter()
            .filter_map(move |doc| match &doc.payload {
                DocPayload::Rpf9Table(table) if table.path == path => Some(table),
                DocPayload::Rpf(rpf) => rpf.session.rpf9_table(path),
                _ => None,
            })
            .chain(explorer)
    }
}

#[cfg(test)]
#[path = "document/rpf9_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rpf9_table_document_tests.rs"]
pub(crate) mod ui_tests;

#[cfg(test)]
#[path = "rpf9_table_shared_tests.rs"]
mod shared_tests;
