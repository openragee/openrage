//! Panel state for the GTA VI archive explorer: the install's archives, the ones opened from disk,
//! and for the one shown a lazily-mounted tree of its table's entries. The listing itself is
//! [`crate::state::rpf9_scan::rpf9_table`]'s; this keeps which archive is shown, what is expanded and the filter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::state::rpf9_scan::rpf9_table::{self, TableFilter, TableNode};
use crate::ui::panel_state::PanelState;

type Slot = Arc<Mutex<Option<Result<TableNode, String>>>>;
type KindsSlot = Arc<Mutex<Option<Vec<String>>>>;

/// Read the kind label of each entry of the table at `base` in `disk` on a worker; a table that
/// does not open leaves its rows their coarse kind, and the mount says why.
fn spawn_kinds(disk: PathBuf, base: u64) -> KindsSlot {
    let slot = KindsSlot::default();
    let fill = Arc::clone(&slot);
    std::thread::spawn(move || {
        let kinds = rpf9_table::keys()
            .and_then(|keys| super::actions::kinds(&disk, base, &keys))
            .unwrap_or_default();
        *fill.lock().unwrap_or_else(|p| p.into_inner()) = Some(kinds);
    });
    slot
}

/// A table mount running on a worker: the entry path its node goes under (`None`: the root), and the
/// slot the worker fills.
struct Pending {
    at: Option<Vec<u32>>,
    slot: Slot,
}

/// Mount the table at `base` in `disk` on a worker: loading the keys can read and hash the whole
/// executable, which must not stall a frame.
fn spawn(at: Option<Vec<u32>>, disk: PathBuf, base: u64) -> Pending {
    let slot = Slot::default();
    let fill = Arc::clone(&slot);
    std::thread::spawn(move || {
        let mounted = rpf9_table::mount_file(&disk, base);
        *fill.lock().unwrap_or_else(|p| p.into_inner()) = Some(mounted);
    });
    Pending { at, slot }
}

/// The explorer's panel state: the archives it offers, which is shown, the tree mounted from it and
/// the filter over that tree.
#[derive(Default)]
pub struct Gta6ArchiveExplorer {
    /// The install root the archive list was walked from, and the list; walked again when the root
    /// changes.
    listed: Option<(PathBuf, Vec<PathBuf>)>,
    /// Archives opened from disk that the explorer lists from their table, in the order they opened.
    pub opened: Vec<PathBuf>,
    /// The archive file currently shown, if any.
    pub selected: Option<PathBuf>,
    /// The root node of [`Self::selected`], when it opened.
    pub root: Option<TableNode>,
    /// Why the last open or expand failed, shown above the tree.
    pub error: Option<String>,
    pub filter: TableFilter,
    /// The open or expand still mounting; a later one replaces it.
    pending: Option<Pending>,
    /// Each mounted table's kind labels by entry index, keyed by the node's entry path.
    pub kinds: BTreeMap<Vec<u32>, Vec<String>>,
    /// The kind labels still being read, by the node's entry path.
    labelling: Vec<(Vec<u32>, KindsSlot)>,
}

impl PanelState for Gta6ArchiveExplorer {
    const KEY: &'static str = "gta6_archive_tree";
}

impl Gta6ArchiveExplorer {
    /// The archives to offer: every `.rpf` under `root`, walked once per root, then the archives
    /// opened from disk that are not among them.
    pub fn archives(&mut self, root: Option<&Path>) -> Vec<PathBuf> {
        let mut list = match root {
            Some(root) => {
                if self.listed.as_ref().map(|(r, _)| r.as_path()) != Some(root) {
                    self.listed = Some((root.to_path_buf(), rpf9_table::archives_in(root)));
                }
                self.listed
                    .as_ref()
                    .map(|(_, l)| l.clone())
                    .unwrap_or_default()
            }
            None => Vec::new(),
        };
        for path in &self.opened {
            if !list.contains(path) {
                list.push(path.clone());
            }
        }
        list
    }

    /// Show `path`, whose table the open already mounted as `node`, and offer it from now on.
    pub fn show_opened(&mut self, path: PathBuf, node: TableNode) {
        if !self.opened.contains(&path) {
            self.opened.push(path.clone());
        }
        self.selected = Some(path.clone());
        self.root = Some(node);
        self.error = None;
        self.pending = None;
        self.kinds.clear();
        self.labelling = vec![(Vec::new(), spawn_kinds(path, 0))];
    }

    /// Open the archive at `path`: its top-level table is mounted on a worker, and [`Self::poll`]
    /// keeps the root node, or the error when it cannot be opened.
    pub fn open(&mut self, path: &Path) {
        self.selected = Some(path.to_path_buf());
        self.root = None;
        self.error = None;
        self.pending = Some(spawn(None, path.to_path_buf(), 0));
        self.kinds.clear();
        self.labelling = vec![(Vec::new(), spawn_kinds(path.to_path_buf(), 0))];
    }

    /// Take a finished mount and finished kind labels into the tree; true while one still runs.
    pub fn poll(&mut self) -> bool {
        for (at, slot) in std::mem::take(&mut self.labelling) {
            let taken = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            match taken {
                Some(kinds) => {
                    self.kinds.insert(at, kinds);
                }
                None => self.labelling.push((at, slot)),
            }
        }
        let done = match &self.pending {
            None => return !self.labelling.is_empty(),
            Some(p) => p.slot.lock().unwrap_or_else(|e| e.into_inner()).take(),
        };
        let Some(done) = done else {
            return true;
        };
        let Some(Pending { at, .. }) = self.pending.take() else {
            return false;
        };
        match (at, done) {
            (None, Ok(node)) => self.root = Some(node),
            (Some(path), Ok(node)) => {
                if let Some((&index, parent_path)) = path.split_last() {
                    if let Some(parent) = self.node_at_mut(parent_path) {
                        parent.children.insert(index, node);
                        parent.open.insert(index);
                    }
                }
            }
            (_, Err(e)) => self.error = Some(e),
        }
        !self.labelling.is_empty()
    }

    /// Expand or collapse the nested archive at entry `path_indices` within the open tree, mounting
    /// its child node on first expand. `path_indices` is the chain of entry indices from the root.
    pub fn toggle(&mut self, path_indices: &[u32]) {
        // A toggle clears a stale error from an earlier failed expand; a failure below sets it again.
        self.error = None;
        let Some(disk) = self.selected.clone() else {
            return;
        };
        let Some((&entry_index, parent_path)) = path_indices.split_last() else {
            return;
        };
        let base = {
            let Some(parent) = self.node_at(parent_path) else {
                return;
            };
            if parent.open.contains(&entry_index) {
                // Collapsing: the child node is kept so re-expanding is instant.
                None
            } else {
                (parent.rows.iter())
                    .find(|r| r.index == entry_index && r.is_archive)
                    .and_then(|r| r.absolute_start)
            }
        };
        let Some(base) = base else {
            if let Some(parent) = self.node_at_mut(parent_path) {
                parent.open.remove(&entry_index);
            }
            return;
        };
        let mounted = self
            .node_at(parent_path)
            .is_some_and(|p| p.children.contains_key(&entry_index));
        if !mounted {
            let kinds = spawn_kinds(disk.clone(), base);
            self.labelling.push((path_indices.to_vec(), kinds));
            self.pending = Some(spawn(Some(path_indices.to_vec()), disk, base));
            return;
        }
        if let Some(parent) = self.node_at_mut(parent_path) {
            parent.open.insert(entry_index);
        }
    }

    /// The absolute header offset of the node at `node_path`: 0 at the root, then the start of each
    /// archive row the path expands through. A ciphered open resolves the base here, cheaply, so the
    /// decipher itself can run on the load worker rather than the UI thread.
    pub(super) fn node_base(&self, node_path: &[u32]) -> Result<u64, String> {
        let mut base = 0u64;
        let mut node = self.root.as_ref().ok_or("no archive is open")?;
        for &index in node_path {
            let row = (node.rows.iter())
                .find(|r| r.index == index && r.is_archive)
                .ok_or_else(|| format!("entry {index} is not an open nested archive"))?;
            base = row
                .absolute_start
                .ok_or_else(|| format!("entry {index} has no start"))?;
            node = node
                .children
                .get(&index)
                .ok_or_else(|| format!("entry {index} is not mounted"))?;
        }
        Ok(base)
    }

    /// The node at `path_indices` (a chain of entry indices from the root), if mounted.
    pub fn node_at(&self, path_indices: &[u32]) -> Option<&TableNode> {
        let mut node = self.root.as_ref()?;
        for &index in path_indices {
            node = node.children.get(&index)?;
        }
        Some(node)
    }

    fn node_at_mut(&mut self, path_indices: &[u32]) -> Option<&mut TableNode> {
        let mut node = self.root.as_mut()?;
        for &index in path_indices {
            node = node.children.get_mut(&index)?;
        }
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::rpf9_scan::rpf9_table::{TableRow, BUILD_ENV};

    /// The real size over which a flagged leaf is left for the cheap smoke test: a huge script body
    /// would decipher-and-inflate for seconds.
    const SAMPLE_CAP: u32 = 0x40000;

    /// A flagged root leaf of a real build decrypts to a recognised inflated head, never ciphertext:
    /// [`crate::state::rpf9_scan::decipher_entry`], which the ciphered-open path runs on the load
    /// worker, reproduces the mount's deciphered body. Install-gated — it needs the key snapshot and
    /// the build, and skips cleanly without them.
    #[test]
    fn a_flagged_root_entry_decrypts_to_a_recognised_head() {
        let have = |v: &str| {
            std::env::var_os(v)
                .map(PathBuf::from)
                .is_some_and(|p| p.exists())
        };
        if !(have(BUILD_ENV) && have("OPENRAGE_GTA6_KEYS")) {
            or_test_support::skip_install(
                "gta6 archive explorer ciphered open",
                format_args!("set OPENRAGE_GTA6_KEYS (with content keys) and {BUILD_ENV}"),
            );
            return;
        }
        let mut explorer = Gta6ArchiveExplorer::default();
        let root = rpf9_table::install_root(None);
        for disk in explorer.archives(root.as_deref()) {
            explorer.open(&disk);
            while explorer.poll() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let Some(root) = explorer.root.clone() else {
                continue;
            };
            let small_leaf = |r: &TableRow| {
                r.cipher && !r.is_archive && r.size.is_some_and(|s| s > 0 && s <= SAMPLE_CAP)
            };
            for index in root.rows.iter().filter(|r| small_leaf(r)).map(|r| r.index) {
                let Ok(body) = crate::state::rpf9_scan::decipher_entry(&disk, 0, index) else {
                    continue;
                };
                if body.is_empty() {
                    continue;
                }
                let head_ok = body.starts_with(b"PSIN")
                    || body.starts_with(b"RBF0")
                    || body.first().is_some_and(|&b| b == b'<' || b == 0xEF)
                    || (body.len() >= 8 && body[..8].iter().all(|&b| b == 0));
                assert!(
                    head_ok,
                    "a decrypted flagged entry's head {:02x?} is not a recognised inflated kind",
                    &body[..body.len().min(4)]
                );
                return;
            }
        }
        or_test_support::skip_install(
            "gta6 archive explorer ciphered open",
            format_args!("no small flagged root entry decrypted in the build"),
        );
    }

    /// Every table of a real build, nested ones included, labels as many rows height map and span
    /// grid as the census measured of their class bytes. Install-gated.
    #[test]
    fn the_build_labels_its_height_maps_and_span_grids_as_measured() {
        use or_io::container::rpf::gta6_kinds::entry_kinds;
        use or_io::container::rpf::{Gta6Mount, GtaViKeys};

        let keys_set =
            std::env::var_os("OPENRAGE_GTA6_KEYS").is_some_and(|p| Path::new(&p).exists());
        let root = rpf9_table::install_root(None).filter(|_| keys_set);
        let keys = root.as_ref().and_then(|_| {
            GtaViKeys::for_install(or_title::config::config_dir().as_deref())
                .ok()
                .flatten()
        });
        let (Some(root), Some(keys)) = (root, keys) else {
            or_test_support::skip_install(
                "gta6 explorer kind labels",
                format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}"),
            );
            return;
        };
        let (mut height_maps, mut span_grids) = (0usize, 0usize);
        for disk in rpf9_table::archives_in(&root) {
            let Ok(mut file) = std::fs::File::open(&disk) else {
                continue;
            };
            let Ok(top) = Gta6Mount::open_root(&mut file, &keys) else {
                continue;
            };
            let mut stack = vec![(top, 0usize)];
            while let Some((mount, depth)) = stack.pop() {
                let kinds = entry_kinds(&mount.archive);
                for (entry, kind) in mount.archive.entries.iter().zip(kinds) {
                    match &*kind {
                        "height map" => height_maps += 1,
                        "span grid" => span_grids += 1,
                        _ => {}
                    }
                    let nested = mount
                        .entry_begins_archive(&mut file, entry)
                        .unwrap_or(false);
                    if nested && depth < 8 {
                        if let Ok(child) = mount.open_nested(&mut file, entry, &keys) {
                            stack.push((child, depth + 1));
                        }
                    }
                }
            }
        }
        assert_eq!((height_maps, span_grids), (1_476, 1_476));
    }

    #[test]
    fn an_open_mounts_on_a_worker_and_its_failure_is_kept_once_it_ends() {
        let mut explorer = Gta6ArchiveExplorer::default();
        explorer.open(Path::new("no such folder/missing.rpf"));
        assert!(explorer.root.is_none() && explorer.error.is_none());
        while explorer.poll() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(explorer.root.is_none());
        assert!(explorer.error.is_some(), "a missing archive is an error");
        assert!(!explorer.poll(), "nothing runs once it is taken");
    }

    #[test]
    fn an_archive_opened_from_disk_is_offered_and_shown_beside_the_install() {
        let mut explorer = Gta6ArchiveExplorer::default();
        let path = PathBuf::from("elsewhere/opened.rpf");
        explorer.show_opened(path.clone(), TableNode::default());
        explorer.show_opened(path.clone(), TableNode::default());
        assert_eq!(explorer.archives(None), vec![path.clone()]);
        assert_eq!(explorer.selected, Some(path));
        assert!(explorer.root.is_some());
    }
}
