//! GTA VI's keys and names as the app holds them, and why an archive opened from disk is listed by
//! its scan rather than its table: no key is held, or its table does not open with the one held.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use or_io::container::rpf::{open_at, GtaViKeys, NameResolver, GTA6_NAMES_ENV, RPF9_MAGIC};

use crate::state::jobs::file_name;
use crate::state::DocPayload;

/// Whether the table of the archive at `path` opens with `keys`, else the sentence that says a scan lists it.
pub(crate) fn table_opens(path: &Path, keys: Result<&GtaViKeys, String>) -> Result<(), String> {
    let scanned = |why: String| {
        tr!(
            "{file} is listed by a scan without its table: {why}",
            file = file_name(path),
            why = why
        )
    };
    let keys = keys.map_err(scanned)?;
    let mut file = File::open(path).map_err(|e| scanned(e.to_string()))?;
    open_at(&mut file, 0, keys).map_err(|e| scanned(e.to_string()))?;
    Ok(())
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

/// The names of the [`GTA6_NAMES_ENV`] folder and of the config dir's harvest, then the local
/// resolved names and the bundled words, built once a run.
///
/// ⚠ The first call reads millions of names and every other caller waits on it, so it runs only on
/// a worker; [`warm_names`] starts it there as soon as a GTA VI folder or key is met.
pub(crate) fn resolver() -> Arc<NameResolver> {
    static RESOLVER: OnceLock<Arc<NameResolver>> = OnceLock::new();
    let load = || {
        let dir = std::env::var_os(GTA6_NAMES_ENV).map(PathBuf::from);
        let config = crate::state::prefs::config_root();
        let mut names = names_cache::load(dir.as_deref(), config.as_deref());
        // A hash keeps the first name given it, so the folder's and the harvest's names stand.
        let local = config.map(|root| root.join("gta6_names").join("resolved-names.tsv"));
        if let Some(text) = local.and_then(|file| std::fs::read_to_string(file).ok()) {
            names.add_hash_name_tsv(&text);
        }
        names.merge_vocabulary(&or_io::gta6::names::Vocabulary::bundled());
        Arc::new(names)
    };
    Arc::clone(RESOLVER.get_or_init(load))
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
        offers().insert(path.to_path_buf(), why);
    }
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

#[path = "gta6_keys_flow.rs"]
pub(crate) mod gta6_keys_flow;

#[path = "rpf9_names_cache.rs"]
mod names_cache;

#[path = "gta6_archive_save.rs"]
mod gta6_archive_save;

#[cfg(test)]
#[path = "rpf9_table_tests.rs"]
mod tests;
