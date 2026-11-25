//! Texture keys and named entries addressed only through bounded RPF9 tables.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use or_io::container::rpf::Gta6EntryType;
use or_job::{Cancel, Progress};

use super::document::rpf9::{entry::EntrySource, MetadataOpener, Rpf9TableDocument};

/// An already mounted table, or an archive whose table the worker opens.
pub(super) enum Root {
    Mounted(Arc<Rpf9TableDocument>),
    File(PathBuf),
}

impl Root {
    fn path(&self) -> &Path {
        match self {
            Self::Mounted(table) => &table.path,
            Self::File(path) => path,
        }
    }
}

#[derive(Clone, Copy)]
struct Limits {
    tables: usize,
    rows: usize,
    dictionaries: usize,
    keys: usize,
    virtual_bytes: u64,
    total_virtual_bytes: u64,
}

const LIMITS: Limits = Limits {
    tables: 8192,
    rows: 2_000_000,
    dictionaries: 65_536,
    keys: 1_000_000,
    virtual_bytes: 8 << 20,
    total_virtual_bytes: 512 << 20,
};

type Stamp = Option<(u64, Option<SystemTime>)>;

/// A bounded catalog of tables, whose names and types are metadata and whose texture bodies stay unread.
#[derive(Default)]
pub(super) struct Tables {
    roots: Vec<Arc<Rpf9TableDocument>>,
    observed: Vec<(PathBuf, Stamp)>,
    nodes: Vec<(Arc<Rpf9TableDocument>, u64)>,
    dictionaries: Vec<(EntrySource, u64)>,
    pub failures: Vec<(PathBuf, String)>,
    pub failed: usize,
    pub truncated: bool,
}

impl Tables {
    fn fail(&mut self, path: PathBuf, error: String) {
        self.failed += 1;
        if self.failures.len() < 64 {
            self.failures.push((path, error));
        }
    }

    pub fn is_current(&self) -> bool {
        self.roots.iter().all(|table| table.check_current().is_ok())
            && self.observed.iter().all(|(path, was)| stamp(path) == *was)
    }

    /// Candidates for a loose file's name hash and type, without reading their bodies.
    pub fn entries_named(&self, hash: u64, extension: &str) -> Vec<EntrySource> {
        let mut found = Vec::new();
        for (table, base) in &self.nodes {
            let addresses = {
                let tree = table.tree();
                let Some(node) = tree.nodes.get(base) else {
                    continue;
                };
                node.mount
                    .archive
                    .entries
                    .iter()
                    .filter(|entry| {
                        entry.name64 == hash
                            && tree.entry_type(*base, entry.index).0.extension() == extension
                    })
                    .map(|entry| entry.index)
                    .collect::<Vec<_>>()
            };
            found.extend(
                addresses
                    .into_iter()
                    .filter_map(|entry| table.entry(*base, entry).ok()),
            );
        }
        found
    }

    /// Open and traverse only table-addressed child archives; `None` once cancelled.
    pub fn open(roots: Vec<Root>, cancel: &Cancel, progress: &Progress) -> Option<Self> {
        Self::open_with_limits(roots, cancel, progress, LIMITS)
    }

    fn open_with_limits(
        roots: Vec<Root>,
        cancel: &Cancel,
        progress: &Progress,
        limits: Limits,
    ) -> Option<Self> {
        let mut catalog = Self::default();
        let mut files = MetadataOpener::default();
        let mut seen = HashSet::new();
        let (mut tables, mut rows) = (0usize, 0usize);
        for root in roots {
            if cancel.is_set() {
                return None;
            }
            if !seen.insert(root.path().to_path_buf()) {
                continue;
            }
            if tables >= limits.tables {
                catalog.truncated = true;
                return Some(catalog);
            }
            catalog
                .observed
                .push((root.path().to_path_buf(), stamp(root.path())));
            let table = match root {
                Root::Mounted(table) => match table.detached_metadata() {
                    Ok(snapshot) => Arc::new(snapshot),
                    Err(error) => {
                        catalog.fail(table.path.clone(), error);
                        continue;
                    }
                },
                Root::File(path) => match files.open(&path) {
                    Ok(Some(table)) => Arc::new(table),
                    Ok(None) => continue,
                    Err(error) => {
                        catalog.fail(path, error);
                        continue;
                    }
                },
            };
            if let Err(error) = table.check_current() {
                catalog.fail(table.path.clone(), error);
                continue;
            }
            catalog.roots.push(Arc::clone(&table));
            let mut pending = VecDeque::from([0]);
            let mut visited = HashSet::new();
            while let Some(base) = pending.pop_front() {
                if cancel.is_set() {
                    return None;
                }
                if !visited.insert(base) {
                    continue;
                }
                tables += 1;
                progress.reached(tables as u64, (tables + pending.len()) as u64);
                let entries = {
                    let tree = table.tree();
                    let Some(node) = tree.nodes.get(&base) else {
                        catalog.fail(
                            table.path.clone(),
                            tr!("The archive table could not be loaded.").to_string(),
                        );
                        continue;
                    };
                    if node.mount.archive.entries.len() > limits.rows.saturating_sub(rows) {
                        catalog.truncated = true;
                        return Some(catalog);
                    }
                    node.mount
                        .archive
                        .entries
                        .iter()
                        .map(|entry| {
                            let size = node
                                .mount
                                .archive
                                .entry_extra(entry.index as usize)
                                .and_then(|extra| extra.nested)
                                .map(|row| u64::from(row.virtual_size & !15));
                            (entry.index, tree.entry_type(base, entry.index).0, size)
                        })
                        .collect::<Vec<_>>()
                };
                catalog.nodes.push((Arc::clone(&table), base));
                rows += entries.len();
                for (entry, kind, size) in entries {
                    if cancel.is_set() {
                        return None;
                    }
                    if kind == Gta6EntryType::Archive {
                        if tables + pending.len() >= limits.tables {
                            catalog.truncated = true;
                            return Some(catalog);
                        }
                        match table.expand_table(base, entry) {
                            Ok(child) => pending.push_back(child),
                            Err(error) => catalog.fail(table.path.clone(), error),
                        }
                        continue;
                    }
                    if kind != Gta6EntryType::TextureDictionary {
                        continue;
                    }
                    if catalog.dictionaries.len() >= limits.dictionaries {
                        catalog.truncated = true;
                        return Some(catalog);
                    }
                    match table.entry(base, entry) {
                        Ok(source) => catalog.dictionaries.push((source, size.unwrap_or(0))),
                        Err(error) => catalog.fail(table.path.clone(), error),
                    }
                }
                if let Err(error) = table.check_current() {
                    catalog.fail(table.path.clone(), error);
                    return Some(catalog);
                }
            }
        }
        Some(catalog)
    }
}

fn stamp(path: &Path) -> Stamp {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}

/// Keys found so far, with a cursor that resumes at the next unread dictionary.
pub(super) struct Index {
    pub tables: Tables,
    keys: BTreeMap<u64, Vec<usize>>,
    next: usize,
    indexed: usize,
    virtual_bytes: u64,
    limited: bool,
}

impl Index {
    pub fn new(tables: Tables) -> Self {
        Self {
            tables,
            keys: BTreeMap::new(),
            next: 0,
            indexed: 0,
            virtual_bytes: 0,
            limited: false,
        }
    }

    pub fn dictionaries(&self) -> usize {
        self.indexed
    }

    pub fn holds(&self, key: u64) -> bool {
        self.keys.contains_key(&key)
    }

    pub fn candidates(&self, key: u64) -> impl Iterator<Item = &EntrySource> {
        self.keys
            .get(&key)
            .into_iter()
            .flatten()
            .filter_map(|&index| {
                self.tables
                    .dictionaries
                    .get(index)
                    .map(|(source, _)| source)
            })
    }

    /// Remove a dictionary that failed to open so the same keys can seek later candidates.
    pub fn reject(&mut self, path: &Path) {
        self.keys.retain(|_, candidates| {
            candidates.retain(|&at| self.tables.dictionaries[at].0.path != path);
            !candidates.is_empty()
        });
    }

    pub fn is_current(&self) -> bool {
        self.tables.is_current()
    }

    pub fn exhausted(&self) -> bool {
        self.next == self.tables.dictionaries.len() || self.limited
    }

    /// Read virtual sections until every requested key is found, a limit is reached, or cancellation returns `false`.
    #[cfg(test)]
    pub fn extend_for(
        &mut self,
        wanted: &HashSet<u64>,
        cancel: &Cancel,
        progress: &Progress,
    ) -> bool {
        self.extend_with_limits(wanted, cancel, progress, LIMITS)
    }

    /// Read until at least one requested key has a candidate, leaving the remaining dictionaries for a later request.
    pub fn extend_until_any(
        &mut self,
        wanted: &HashSet<u64>,
        cancel: &Cancel,
        progress: &Progress,
    ) -> bool {
        self.extend(wanted, cancel, progress, LIMITS, true)
    }

    #[cfg(test)]
    fn extend_with_limits(
        &mut self,
        wanted: &HashSet<u64>,
        cancel: &Cancel,
        progress: &Progress,
        limits: Limits,
    ) -> bool {
        self.extend(wanted, cancel, progress, limits, false)
    }

    fn extend(
        &mut self,
        wanted: &HashSet<u64>,
        cancel: &Cancel,
        progress: &Progress,
        limits: Limits,
        any: bool,
    ) -> bool {
        let found = |index: &Self| {
            wanted.is_empty()
                || if any {
                    wanted.iter().any(|key| index.holds(*key))
                } else {
                    wanted.iter().all(|key| index.holds(*key))
                }
        };
        while !found(self) && !self.exhausted() {
            if cancel.is_set() {
                return false;
            }
            let (source, size) = &self.tables.dictionaries[self.next];
            if self.virtual_bytes.saturating_add(*size) > limits.total_virtual_bytes {
                self.tables.truncated = true;
                self.limited = true;
                break;
            }
            let result = source.texture_keys_limited(limits.virtual_bytes);
            let path = source.path.clone();
            self.virtual_bytes = self.virtual_bytes.saturating_add(*size);
            let at = self.next;
            self.next += 1;
            progress.reached(self.next as u64, self.tables.dictionaries.len() as u64);
            match result {
                Ok(keys) => {
                    let new_keys = keys
                        .iter()
                        .filter(|key| !self.keys.contains_key(*key))
                        .count();
                    if self.keys.len().saturating_add(new_keys) > limits.keys {
                        self.tables.truncated = true;
                        self.limited = true;
                        break;
                    }
                    self.indexed += 1;
                    for key in keys {
                        let candidates = self.keys.entry(key).or_default();
                        if candidates.last() != Some(&at) {
                            candidates.push(at);
                        }
                    }
                }
                Err(error) => self.tables.fail(path, error),
            }
        }
        !cancel.is_set()
    }
}

#[cfg(test)]
#[path = "rpf9_texture_index_tests.rs"]
pub(super) mod tests;
