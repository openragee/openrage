//! What a GTA VI body's own data tells us about its name: a drawable, fragment or drawable
//! dictionary carries the stated model name at offset 0xA8 of its virtual section, which the
//! archive table does not hold. When such a body opens, the leaf's name is kept here by
//! `(archive file, body offset)`, so the tree can show the stated name in place of the
//! `hash_<hex>` it had with no resolved path.
//!
//! ponytail: one map for the process, written once per body on open; a renamed archive file
//! leaves its entries behind, but the read cost is one `HashMap::get` in the hot tree-draw path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use or_io::container::rpf::{debug_name_from_virtual, Rpf9Kind};

/// The learned-names store, keyed by `(archive file, body offset)`.
fn store() -> &'static Mutex<HashMap<(PathBuf, u64), String>> {
    static STORE: OnceLock<Mutex<HashMap<(PathBuf, u64), String>>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

/// Keep the stated name of a body `virtual_section` is the virtual section of, where that kind
/// carries one; `None` for a kind that does not state a name. Returns the stem that was kept.
pub fn learn(
    archive: &Path,
    offset: u64,
    kind: Rpf9Kind,
    virtual_section: &[u8],
) -> Option<String> {
    let name = debug_name_from_virtual(kind, virtual_section)?;
    let stem = leaf_stem(&name);
    if stem.is_empty() {
        return None;
    }
    let mut store = store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    store
        .entry((archive.to_path_buf(), offset))
        .or_insert_with(|| stem.to_owned());
    Some(stem.to_owned())
}

/// The stem learned for the body at `offset` of `archive`, where one was kept; `None` otherwise.
pub fn lookup(archive: &Path, offset: u64) -> Option<String> {
    let store = store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    store.get(&(archive.to_path_buf(), offset)).cloned()
}

/// The last path segment of `name`, with any `.#dr`, `.#ft`, `.#dd` suffix removed.
///
/// The stated name is either a bare stem (`car`) or a path with the platform-independent RAGE
/// suffix (`car.#dr`); either way we keep the leaf name without the suffix.
fn leaf_stem(name: &str) -> &str {
    let after_sep = name.rsplit_once(['/', '\\']).map_or(name, |(_, tail)| tail);
    const SUFFIXES: &[&str] = &[".#dr", ".#ft", ".#dd", ".#ld"];
    for suffix in SUFFIXES {
        if let Some(stem) = after_sep.strip_suffix(suffix) {
            return stem;
        }
    }
    after_sep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leaf_stem_drops_folders_and_a_rage_suffix() {
        assert_eq!(leaf_stem("models/car.#dr"), "car");
        assert_eq!(leaf_stem("car.#dr"), "car");
        assert_eq!(leaf_stem("car.#ft"), "car");
        assert_eq!(leaf_stem("plain_name"), "plain_name");
        // A `#dd` suffix is a dictionary leaf; a hash-name with an unknown suffix stays as given.
        assert_eq!(leaf_stem("peds.#dd"), "peds");
        assert_eq!(leaf_stem("something.#xx"), "something.#xx");
    }

    #[test]
    fn a_learned_name_is_recovered_by_archive_and_offset() {
        let dir = tempfile::tempdir().expect("scratch");
        let archive = dir.path().join("test.rpf");
        // No learned name yet.
        assert_eq!(lookup(&archive, 0x1000), None);
        // A direct insert through the API's own key (bypassing `learn`, which needs valid bytes).
        let mut store = store().lock().unwrap();
        store.insert((archive.clone(), 0x1000), "car".to_owned());
        drop(store);
        assert_eq!(lookup(&archive, 0x1000), Some("car".to_owned()));
        // A row at another offset of the same archive sees nothing.
        assert_eq!(lookup(&archive, 0x2000), None);
    }
}
