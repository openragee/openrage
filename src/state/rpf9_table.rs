//! A GTA VI archive listed from its decrypted table: each entry's name, hash, size and kind, a nested
//! archive mounted when it is expanded, and the filter the archive explorer applies to what is
//! mounted. An archive opened from disk is listed here first, and by the scan only when no key opens
//! its table.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use or_io::container::rpf::{open_at, Gta6Class, GtaViKeys, NameResolver, RPF9_MAGIC};

use crate::state::jobs::file_name;
use crate::state::DocPayload;

/// A build folder to list archives from in place of the configured install, for a developer's session.
pub(crate) const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";

/// What an entry is, by the table's own facts: its body's header, or its row's class byte.
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

    /// A body that opens with an RPF9 header is an archive; else the 24-byte row's class says, and an
    /// entry with only a 20-byte row is a resource, whose row is a resource's descriptor.
    fn of(is_archive: bool, class: Option<Gta6Class>, nested_row: bool) -> Self {
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
    /// The entry's absolute byte offset in the file, where its row states a start.
    pub absolute_start: Option<u64>,
    /// Whether the body at [`Self::absolute_start`] opens with the clear `9FPR` header.
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

/// Mount the archive whose header is at absolute `base` in `src`: decipher its table, name its
/// entries from `resolver` and read each entry's leading magic, so an archive row can be told from a
/// leaf. Only the header, the table and four bytes per entry are read.
pub(crate) fn mount<R: Read + Seek>(
    src: &mut R,
    base: u64,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<TableNode, String> {
    let mut archive = open_at(src, base, keys).map_err(|e| e.to_string())?;
    archive.resolve_names(resolver);
    if let Err(error) = resolver.cache_local_entries(&archive.entries) {
        tracing::warn!(%error, "could not update the local GTA VI names cache");
    }
    let extras = archive.entry_extras();
    let rows = (archive.entries.iter().enumerate())
        .map(|(at, e)| {
            let absolute_start = e.start.map(|s| base + s);
            let is_archive =
                absolute_start.is_some_and(|off| magic_at(&mut *src, off) == Some(RPF9_MAGIC));
            let nested_row = extras.get(at).is_some_and(|x| x.nested.is_some());
            TableRow {
                index: e.index,
                name64: e.name64,
                name32: e.name32,
                name: e.name.clone(),
                size: e.size(),
                absolute_start,
                is_archive,
                cipher: e.cipher.is_some(),
                kind: TableKind::of(is_archive, e.kind(), nested_row),
            }
        })
        .collect();
    Ok(TableNode {
        key_id: archive.toc.key_id,
        rows,
        ..TableNode::default()
    })
}

/// The 4-byte magic at `offset` in `src`, or `None` when it cannot be read.
fn magic_at<R: Read + Seek>(src: &mut R, offset: u64) -> Option<u32> {
    src.seek(SeekFrom::Start(offset)).ok()?;
    let mut head = [0u8; 4];
    src.read_exact(&mut head).ok()?;
    Some(u32::from_le_bytes(head))
}

/// The archive at `path` listed from its table with `keys`, or, where no key is held or the table does
/// not open, the one sentence that says it is listed by a scan instead.
pub(crate) fn listing(
    path: &Path,
    keys: Result<&GtaViKeys, String>,
    resolver: &NameResolver,
) -> Result<TableNode, String> {
    let scanned = |why: String| {
        tr!(
            "{file} is listed by a scan without its table: {why}",
            file = file_name(path),
            why = why
        )
    };
    let keys = keys.map_err(scanned)?;
    let mut file = File::open(path).map_err(|e| scanned(e.to_string()))?;
    mount(&mut file, 0, keys, resolver).map_err(scanned)
}

/// The archive at `path` mounted from absolute `base`, with the keys and names the app holds.
pub(crate) fn mount_file(path: &Path, base: u64) -> Result<TableNode, String> {
    let keys = keys()?;
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    mount(&mut file, base, &keys, &resolver())
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

/// Bundled names supplemented by `OPENRAGE_GTA6_NAMES` and the local vocabulary, loaded once.
pub(crate) fn resolver() -> Arc<NameResolver> {
    static RESOLVER: OnceLock<Arc<NameResolver>> = OnceLock::new();
    let load = || {
        let local = crate::state::prefs::config_root().map(|root| root.join("gta6_names"));
        let resolver = NameResolver::load_with_sources(None, local.as_deref());
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

/// What the open of each archive found of its table, by its path, until the app that opened it takes it.
// ponytail: one map for the process; an open whose document never arrives leaves its entry behind.
static OFFERS: Mutex<BTreeMap<PathBuf, Result<TableNode, String>>> = Mutex::new(BTreeMap::new());

fn offers() -> MutexGuard<'static, BTreeMap<PathBuf, Result<TableNode, String>>> {
    OFFERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// On the load's worker, list the archive at `path` from its table for the explorer, or say why the
/// scan lists it; nothing for a file that does not open with an RPF9 header.
pub(in crate::state) fn offer(path: &Path) {
    let mut head = [0u8; 4];
    let rpf9 = File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && u32::from_le_bytes(head) == RPF9_MAGIC;
    if !rpf9 {
        return;
    }
    let keys = keys();
    let listing = listing(path, keys.as_deref().map_err(String::clone), &resolver());
    offers().insert(path.to_path_buf(), listing);
}

impl crate::state::AppState {
    /// Show each archive opened from disk in the archive explorer from its table, or say in a toast
    /// that its scan lists it; an offer waits until its archive's tab is open in this app.
    pub(in crate::state) fn poll_table_opens(&mut self) {
        let opened: Vec<PathBuf> = {
            let offers = offers();
            if offers.is_empty() {
                return;
            }
            (offers.keys())
                .filter(|path| {
                    self.documents.iter().any(|d| {
                        d.path.as_deref() == Some(path.as_path())
                            && matches!(&d.payload, DocPayload::Rpf9(scan) if scan.start == 0)
                    })
                })
                .cloned()
                .collect()
        };
        for path in opened {
            match offers().remove(&path) {
                Some(Ok(node)) => {
                    use crate::ui::panels::gta6_archive_tree as tree;
                    self.panels
                        .get::<tree::Gta6ArchiveExplorer>()
                        .show_opened(path, node);
                    let ws = crate::state::DocumentKind::Rpf9.info().workspace;
                    self.workspace_docks.reveal(ws, tree::ID);
                }
                Some(Err(why)) => self.show_info_toast(why),
                None => {}
            }
        }
    }
}

#[path = "gta6_keys_flow.rs"]
pub(crate) mod gta6_keys_flow;

#[path = "gta6_archive_save.rs"]
mod gta6_archive_save;

#[cfg(test)]
#[path = "rpf9_table_tests.rs"]
mod tests;
