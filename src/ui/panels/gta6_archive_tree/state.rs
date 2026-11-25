//! The Archives panel keeps a view of the same bounded table that Files and previews use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::state::rpf9_scan::rpf9_table::{self, TableFilter, TableNode};
use crate::state::rpf9_scan::rpf9_table_document::Rpf9TableDocument;
use crate::ui::panel_state::PanelState;

type Slot = Arc<Mutex<Option<Result<Arc<Rpf9TableDocument>, String>>>>;

#[derive(Default)]
pub struct Gta6ArchiveExplorer {
    listed: Option<(PathBuf, Vec<PathBuf>)>,
    pub opened: Vec<PathBuf>,
    pub selected: Option<PathBuf>,
    pub table: Option<Arc<Rpf9TableDocument>>,
    pub root: Option<TableNode>,
    pub error: Option<String>,
    pub filter: TableFilter,
    pub kinds: BTreeMap<Vec<u32>, Vec<String>>,
    pending: Option<Slot>,
    revision: Option<u64>,
}

impl PanelState for Gta6ArchiveExplorer {
    const KEY: &'static str = "gta6_archive_tree";
}

impl Gta6ArchiveExplorer {
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

    pub(crate) fn show_table(&mut self, table: Arc<Rpf9TableDocument>) {
        if !self.opened.contains(&table.path) {
            self.opened.push(table.path.clone());
        }
        self.selected = Some(table.path.clone());
        self.table = Some(table);
        self.pending = None;
        self.error = None;
        self.revision = None;
        self.refresh();
    }

    pub fn open(&mut self, path: &Path) {
        self.selected = Some(path.to_path_buf());
        self.table = None;
        self.root = None;
        self.error = None;
        self.kinds.clear();
        self.revision = None;
        let slot = Slot::default();
        let fill = Arc::clone(&slot);
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let result = Rpf9TableDocument::open(&path).and_then(|table| {
                table
                    .map(Arc::new)
                    .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())
            });
            *fill.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
        });
        self.pending = Some(slot);
    }

    fn refresh(&mut self) {
        let Some(table) = &self.table else { return };
        let tree = table.tree();
        if self.revision != Some(tree.revision) {
            self.root = tree.listing();
            self.kinds = tree.kind_labels();
            self.revision = Some(tree.revision);
        }
        self.error = tree.errors.values().next().cloned();
    }

    pub fn poll(&mut self) -> bool {
        let done = self
            .pending
            .as_ref()
            .and_then(|slot| slot.lock().unwrap_or_else(|p| p.into_inner()).take());
        if let Some(done) = done {
            self.pending = None;
            match done {
                Ok(table) => self.show_table(table),
                Err(error) => self.error = Some(error),
            }
        }
        let pending = self.table.as_ref().is_some_and(|table| {
            let mut tree = table.tree();
            tree.poll();
            tree.pending().is_some()
        });
        self.refresh();
        self.pending.is_some() || pending
    }

    pub fn toggle(&mut self, indices: &[u32]) {
        if let Some(table) = &self.table {
            table.toggle_path(indices);
        }
        self.refresh();
    }

    pub(super) fn node_base(&self, indices: &[u32]) -> Result<u64, String> {
        self.table
            .as_ref()
            .and_then(|table| table.tree().base_of(indices))
            .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::rpf9_scan::rpf9_table::{TableRow, BUILD_ENV};

    /// The real size over which a flagged leaf is left for the cheap smoke test: a huge script body
    /// would decipher-and-inflate for seconds.
    const SAMPLE_CAP: u32 = 0x40000;

    /// A flagged root leaf read from the explorer's canonical table has a recognised decoded head.
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
            let table = explorer.table.as_ref().expect("the listed table is held");
            for index in root.rows.iter().filter(|r| small_leaf(r)).map(|r| r.index) {
                let source = table.entry(0, index).expect("the row has a bounded source");
                let body = source
                    .bytes_limited(u64::from(SAMPLE_CAP))
                    .unwrap_or_else(|e| panic!("{}:{index}: {e}", disk.display()));
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
        let (_dir, table) =
            crate::state::rpf9_scan::rpf9_table_document::ui_tests::sample_document();
        let path = table.path.clone();
        let table = Arc::new(table);
        explorer.show_table(Arc::clone(&table));
        explorer.show_table(table);
        assert_eq!(explorer.archives(None), vec![path.clone()]);
        assert_eq!(explorer.selected, Some(path));
        assert!(explorer.root.is_some());
    }
}
