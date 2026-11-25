//! A GTA VI archive listed from its decrypted table: each entry's name, hash, size and kind, a nested
//! archive mounted when it is expanded, and the filter the archive explorer applies to what is
//! mounted. An unreadable table remains an error.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use or_io::container::rpf::{Gta6Class, GtaViKeys, NameResolver};
use or_io::gta6::names::{vocabulary_file, Vocabulary};

use crate::state::DocPayload;

/// A build folder to list archives from in place of the configured install, for a developer's session.
pub(crate) const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A folder of recovered names, folded into the names the app harvested.
const NAMES_ENV: &str = "OPENRAGE_GTA6_NAMES";

/// An entry's family from its table class and resource descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TableKind {
    Archive,
    Resource,
    Media,
    Other,
    Unknown,
}

impl TableKind {
    pub(crate) const ALL: [TableKind; 5] = [
        TableKind::Archive,
        TableKind::Resource,
        TableKind::Media,
        TableKind::Other,
        TableKind::Unknown,
    ];

    /// The table's family, including a 20-byte resource descriptor.
    pub(crate) fn of(is_archive: bool, class: Option<Gta6Class>, nested_row: bool) -> Self {
        match (is_archive, class) {
            (true, _) | (false, Some(Gta6Class::NestedArchive)) => Self::Archive,
            (false, Some(Gta6Class::CompressedResource)) => Self::Resource,
            (false, Some(Gta6Class::Media)) => Self::Media,
            (false, Some(Gta6Class::Other(_))) => Self::Other,
            (false, None) if nested_row => Self::Resource,
            (false, None) => Self::Unknown,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Archive => tr!("archive"),
            Self::Resource => tr!("resource"),
            Self::Media => tr!("media"),
            Self::Other => tr!("other class"),
            Self::Unknown => tr!("not known"),
        }
    }
}

/// One entry as the table states it.
#[derive(Debug, Clone)]
pub(crate) struct TableRow {
    pub index: u32,
    pub name64: u64,
    pub name32: Option<u32>,
    pub name: Option<String>,
    pub size: Option<u32>,
    /// Whether the table identifies a nested archive.
    pub is_archive: bool,
    /// Whether the body is enciphered; it then opens through the ciphered door.
    pub cipher: bool,
    pub kind: TableKind,
}

impl TableRow {
    /// The recovered path, or `hash_{name64:016x}` where it never resolved.
    pub fn display_name(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("hash_{:016x}", self.name64),
        }
    }
}

/// One mounted archive: its entries and, lazily, the nested archives expanded under it.
#[derive(Debug, Clone, Default)]
pub(crate) struct TableNode {
    pub key_id: u32,
    pub rows: Vec<TableRow>,
    /// The expanded nested archives, keyed by the entry index they sit at; loaded on first expand.
    pub children: BTreeMap<u32, TableNode>,
    /// Which entries are expanded; the child is in [`Self::children`].
    pub open: BTreeSet<u32>,
}

/// Where the keys are loaded from: the executable that unmasks them and the snapshot the
/// environment names.
type KeySource = (Option<PathBuf>, Option<std::ffi::OsString>);

/// GTA VI's keys as the app loads them, held with the failure when none are found: unmasking a
/// snapshot reads and hashes a large executable. They are loaded again once their source changes.
pub(crate) fn keys() -> Result<Arc<GtaViKeys>, String> {
    type Held = Option<(KeySource, Result<Arc<GtaViKeys>, String>)>;
    static KEYS: Mutex<Held> = Mutex::new(None);
    let source: KeySource = (
        gta6_keys_flow::executable(),
        std::env::var_os(or_io::container::rpf::GTA6_SNAPSHOT_ENV),
    );
    let mut held = KEYS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((from, keys)) = held.as_ref() {
        if *from == source {
            return keys.clone();
        }
    }
    let keys = gta6_keys_flow::load_gta6_keys_with(source.0.as_deref()).map(Arc::new);
    *held = Some((source, keys.clone()));
    keys
}

/// The recovered-names folder [`NAMES_ENV`] names, folded together with the vocabulary the app
/// harvested into the config dir, loaded once.
pub(crate) fn resolver() -> Arc<NameResolver> {
    static RESOLVER: OnceLock<Arc<NameResolver>> = OnceLock::new();
    let load = || {
        let mut resolver = match std::env::var_os(NAMES_ENV).map(PathBuf::from) {
            Some(dir) => NameResolver::load(&dir),
            None => NameResolver::new(),
        };
        if let Some(config) = or_title::config::config_dir() {
            resolver.merge_vocabulary(&Vocabulary::load(&vocabulary_file(&config)));
        }
        Arc::new(resolver)
    };
    Arc::clone(RESOLVER.get_or_init(load))
}

/// The folder the explorer lists archives from: [`BUILD_ENV`] where it names a folder, else the GTA VI
/// install chosen in Settings.
pub(crate) fn install_root(configured: Option<&Path>) -> Option<PathBuf> {
    std::env::var_os(BUILD_ENV)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| configured.filter(|p| p.is_dir()).map(Path::to_path_buf))
}

/// Every `.rpf` under `root`, in path order, as the install's own walk finds them.
pub(crate) fn archives_in(root: &Path) -> Vec<PathBuf> {
    crate::state::gta6_install_scan::archives_under(root).0
}

/// The words, kinds and hashes the explorer keeps rows by.
#[derive(Debug, Clone, Default)]
pub(crate) struct TableFilter {
    pub text: String,
    pub kinds: BTreeSet<TableKind>,
}

impl TableFilter {
    pub(crate) fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.kinds.is_empty()
    }

    /// Whether `row` is of a kind toggled on, where any is, and its name holds the text; hex text
    /// also finds the name64 or name32 hash, and text written `0x…` finds a hash only.
    pub(crate) fn keeps(&self, row: &TableRow) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&row.kind) {
            return false;
        }
        let text = self.text.trim().to_lowercase();
        if text.is_empty() {
            return true;
        }
        let (hex, hash_only) = match text.strip_prefix("0x") {
            Some(rest) => (rest, true),
            None => (text.as_str(), false),
        };
        let is_hex = !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
        let hash = is_hex
            && (format!("{:016x}", row.name64).contains(hex)
                || row.name32.is_some_and(|h| format!("{h:08x}").contains(hex)));
        hash || (!hash_only && row.display_name().to_lowercase().contains(&text))
    }

    /// Whether `node` or a nested archive expanded under it holds a row the filter keeps.
    pub(crate) fn keeps_under(&self, node: &TableNode) -> bool {
        node.rows.iter().any(|row| {
            self.keeps(row)
                || (node.open.contains(&row.index)
                    && node
                        .children
                        .get(&row.index)
                        .is_some_and(|c| self.keeps_under(c)))
        })
    }
}

impl crate::state::AppState {
    /// Share the already mounted tables with the archive explorer.
    pub(in crate::state) fn poll_table_opens(&mut self) {
        let mut paths = std::collections::BTreeSet::new();
        for doc in &self.documents {
            match &doc.payload {
                DocPayload::Rpf9Table(table) => {
                    paths.insert(table.path.clone());
                }
                DocPayload::Rpf(rpf) => {
                    for node in rpf.session.nodes.values() {
                        if let crate::state::RpfNodeBacking::Rpf9Archive {
                            table,
                            owner: None,
                            ..
                        } = &node.backing
                        {
                            paths.insert(table.path.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        let tables: Vec<_> = paths
            .into_iter()
            .filter_map(|path| self.mounted_rpf9_table(&path))
            .collect();
        let explorer = self
            .panels
            .get::<crate::ui::panels::gta6_archive_tree::Gta6ArchiveExplorer>();
        for table in tables {
            if !explorer.opened.contains(&table.path)
                || (explorer.selected.as_deref() == Some(table.path.as_path())
                    && explorer
                        .table
                        .as_ref()
                        .is_none_or(|old| !Arc::ptr_eq(old, &table)))
            {
                explorer.show_table(table);
            }
        }
    }
}

#[path = "gta6_keys_flow.rs"]
pub(crate) mod gta6_keys_flow;

#[path = "gta6_archive_save.rs"]
mod gta6_archive_save;
pub(crate) use gta6_archive_save::write_entry;

#[cfg(test)]
#[path = "rpf9_table_tests.rs"]
mod tests;
