//! GTA VI's keys and names as the app holds them, and why an archive opened from disk is listed by
//! its scan rather than its table: no key is held, or its table does not open with the one held.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use or_io::container::rpf::{open_at, GtaViKeys, NameResolver, RPF9_MAGIC};

use crate::state::jobs::file_name;
use crate::state::DocPayload;

/// Whether the table of the archive at `path` opens with `keys`, else the sentence that says a scan lists it.
pub(crate) fn table_opens(path: &Path, keys: Result<&GtaViKeys, String>) -> Result<(), String> {
    // A table cut short is said so before any key is asked for.
    if let Some(short) = crate::state::gta_vi::rpf9_cut_short_at(path) {
        return Err(scanned(path, short));
    }
    let keys = keys.map_err(|why| scanned(path, why))?;
    let mut file = File::open(path).map_err(|e| scanned(path, e.to_string()))?;
    open_at(&mut file, 0, keys).map_err(|e| scanned(path, e.to_string()))?;
    Ok(())
}

/// The sentence that says a scan lists the archive at `path` without its table, and `why`.
fn scanned(path: &Path, why: String) -> String {
    tr!(
        "{file} is listed by a scan without its table: {why}",
        file = file_name(path),
        why = why
    )
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
    if keys.is_ok() {
        warm_names();
    }
    keys
}

/// The names of the [`or_io::container::rpf::default_names_dir`] folder, of the config dir's harvest and local resolved names, of the
/// default hash index and of the bundled words, built once a run.
///
/// ⚠ The first call reads millions of names and every other caller waits on it, so it runs only on
/// a worker; [`warm_names`] starts it there as soon as a GTA VI folder or key is met.
pub(crate) fn resolver() -> Arc<NameResolver> {
    static RESOLVER: OnceLock<Arc<NameResolver>> = OnceLock::new();
    let load = || {
        let dir = or_io::container::rpf::default_names_dir();
        let config = crate::state::prefs::config_root();
        Arc::new(names_from(dir.as_deref(), config.as_deref()))
    };
    Arc::clone(RESOLVER.get_or_init(load))
}

/// The names of `dir` and `config`'s harvest, then each name the default hash index holds with no game data, under its GTA VI path hash, then the bundled words.
fn names_from(dir: Option<&Path>, config: Option<&Path>) -> NameResolver {
    // Installs the schema snapshot, which the default index reads once and keeps.
    crate::state::meta_schema();
    let mut names = NameResolver::load_cached(dir, config);
    // `insert` keeps the first name a hash is given, so the user's own names win.
    for name in or_io::meta::binary::hash_index::default_hash_index().names() {
        names.insert(or_io::gta6::hash::name_hash64(name.as_bytes()), name);
    }
    names.merge_vocabulary(&or_io::gta6::names::Vocabulary::bundled());
    names
}

/// Builds [`resolver`] on a worker once a run where keys are held, before an archive asks for it.
pub(crate) fn warm_names() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            // Names are read only for a run that holds keys: an archive opens with none otherwise.
            if keys().is_ok() {
                resolver();
            }
        });
    });
}

/// Why the scan lists each archive opened from disk, by its path, until the app that opened it says so.
// ponytail: one map for the process; an open whose document never arrives leaves its entry behind.
static OFFERS: Mutex<BTreeMap<PathBuf, String>> = Mutex::new(BTreeMap::new());

fn offers() -> MutexGuard<'static, BTreeMap<PathBuf, String>> {
    OFFERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// On the load's worker, keep why the scan lists the RPF9 archive at `path` where its table does not open.
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
    if let Err(why) = table_opens(path, keys.as_deref().map_err(Clone::clone)) {
        // What `offer_unopened` kept of the tree's own open says more, and stays.
        offers().entry(path.to_path_buf()).or_insert(why);
    }
}

/// On the load's worker, keep `why` the RPF9 archive at `path` did not open as a tree with the keys held, for the toast its scan's tab says.
pub(in crate::state) fn offer_unopened(path: &Path, why: String) {
    offers().insert(path.to_path_buf(), scanned(path, why));
}

impl crate::state::AppState {
    /// Say in a toast why the scan lists each archive whose tab is open, once that tab is open.
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
            if let Some(why) = offers().remove(&path) {
                self.show_info_toast(why);
            }
        }
    }
}

#[path = "gta6_keys_held.rs"]
pub(crate) mod gta6_keys_held;

#[path = "gta6_keys_flow.rs"]
pub(crate) mod gta6_keys_flow;

#[path = "rpf9_names_cache.rs"]
mod names_cache;

#[path = "gta6_archive_save.rs"]
mod gta6_archive_save;

#[cfg(test)]
#[path = "rpf9_table_tests.rs"]
mod tests;

#[cfg(test)]
mod default_names {
    use super::*;
    use or_io::gta6::hash::name_hash64;

    #[test]
    fn with_no_names_folder_a_row_is_named_by_a_name_the_default_index_holds() {
        let index = or_io::meta::binary::hash_index::default_hash_index();
        let name = index
            .names()
            .next()
            .expect("the default index holds a name");
        let hash = name_hash64(name.as_bytes());
        let named = names_from(None, None);
        assert!(
            named
                .lookup(hash)
                .is_some_and(|n| n.eq_ignore_ascii_case(name)),
            "{name:?} names no row: {:?}",
            named.lookup(hash)
        );

        // A name the user's folder gives the same hash wins over the index's.
        let dir = tempfile::tempdir().expect("scratch");
        let mine = format!("mine/{name}");
        let table = format!("0x{hash:016x}\t{mine}\n");
        std::fs::write(dir.path().join("resolved-names.tsv"), table).expect("write");
        let named = names_from(Some(dir.path()), None);
        assert_eq!(named.lookup(hash), Some(mine.as_str()));
    }
}
