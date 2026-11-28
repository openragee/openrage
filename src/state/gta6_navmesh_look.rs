//! Which of a GTA VI cell's neighbours the table of its archive holds a navigation row of, looked
//! for on a worker, ciphered rows and nested archives among them: no special link names a cell that
//! has no body, so a deletion takes a cell the table holds no row of as having no link into the
//! edited one. A byte scan sees no ciphered body, so with no key every neighbour may hold one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{navigation_cells, Gta6Mount, GtaViKeys};
use or_io::gta6::headerless::NavMesh;

use crate::state::rpf9_scan::rpf9_table;
use crate::state::AppState;

/// What the looks through one archive's table found: per cell, whether it may hold a body of it.
#[derive(Debug, Default)]
struct Looks {
    running: bool,
    found: BTreeMap<u32, bool>,
    /// Whether a look was made with no key, so what it found is dropped once a key is held.
    keyless: bool,
}

impl Looks {
    /// Drop what a look made with no key found, once `keyed` says a key is held now.
    fn refresh(&mut self, keyed: bool) {
        if self.keyless && keyed && !self.running {
            self.found.clear();
            self.keyless = false;
        }
    }
}

// ponytail: an entry per archive a deletion looked through, never emptied, as the cells are.
static LOOKS: Mutex<BTreeMap<PathBuf, Looks>> = Mutex::new(BTreeMap::new());

fn looks() -> MutexGuard<'static, BTreeMap<PathBuf, Looks>> {
    LOOKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The cells of `near` whose body is not open, but those `found` says the archive holds no body of.
fn closed_of(near: &[(u32, Option<Arc<NavMesh>>)], found: &BTreeMap<u32, bool>) -> Vec<String> {
    (near.iter())
        .filter(|(c, body)| body.is_none() && found.get(c) != Some(&false))
        .map(|(c, _)| c.to_string())
        .collect()
}

/// The cells the table of the archive at `archive` holds a navigation row of; `None` where it does
/// not open or a row's cell does not read.
// Only navigation rows are counted: on the build every sized class-0x0B row of a file holding one
// is metadata, never a navigation body (`openrage-cli`'s install-gated navmesh edit test holds it).
fn table_cells(archive: &Path, keys: Result<Arc<GtaViKeys>, String>) -> Option<BTreeSet<u32>> {
    let keys = keys.ok()?;
    let mut file = File::open(archive).ok()?;
    let mount = Gta6Mount::open_root(&mut file, &keys).ok()?;
    navigation_cells(&mut file, &mount, &keys).ok()
}

/// Whether the table of the archive at `archive`, opened with `keys`, may hold a body of each of
/// `wanted`: every one where [`table_cells`] has none.
fn bodies_of(
    archive: &Path,
    keys: Result<Arc<GtaViKeys>, String>,
    wanted: &[u32],
) -> BTreeMap<u32, bool> {
    let held = table_cells(archive, keys);
    (wanted.iter())
        .map(|c| (*c, held.as_ref().is_none_or(|h| h.contains(c))))
        .collect()
}

/// Look through the table of `archive` on a worker for a navigation row of each of `wanted`.
fn look_through(archive: PathBuf, wanted: Vec<u32>) {
    std::thread::spawn(move || {
        let keys = rpf9_table::keys();
        let keyless = keys.is_err();
        let found = bodies_of(&archive, keys, &wanted);
        let mut held = looks();
        let look = held.entry(archive).or_default();
        look.running = false;
        look.keyless |= keyless;
        look.found.extend(found);
    });
}

impl AppState {
    /// The cells of `near` whose body is not open and that the table of the archive the body at
    /// `path` came from may hold a body of; `Err` with the cells a look through that table, started
    /// here where none runs, has still to find.
    pub(super) fn unopened(
        &self,
        path: &Path,
        near: &[(u32, Option<Arc<NavMesh>>)],
    ) -> Result<Vec<String>, String> {
        let archive = path.parent().unwrap_or(path);
        let mut held = looks();
        let look = held.entry(archive.to_path_buf()).or_default();
        if look.keyless {
            look.refresh(rpf9_table::keys().is_ok());
        }
        let unknown: Vec<u32> = (near.iter())
            .filter(|(c, body)| body.is_none() && !look.found.contains_key(c))
            .map(|(c, _)| *c)
            .collect();
        if unknown.is_empty() {
            return Ok(closed_of(near, &look.found));
        }
        if !look.running {
            look.running = true;
            look_through(archive.to_path_buf(), unknown.clone());
        }
        let cells: Vec<String> = unknown.iter().map(u32::to_string).collect();
        Err(cells.join(", "))
    }
}

/// Take the table of `archive` as holding no navigation row of `cells`, as a look through it finds.
#[cfg(test)]
pub(super) fn given_none_of(archive: &Path, cells: &[u32]) {
    let mut held = looks();
    let look = held.entry(archive.to_path_buf()).or_default();
    look.found.extend(cells.iter().map(|c| (*c, false)));
}

/// Wait for the look through the table of `archive` to end.
#[cfg(test)]
pub(super) fn settled(archive: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while looks().get(archive).is_some_and(|l| l.running) {
        assert!(std::time::Instant::now() < deadline, "the look never ended");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::{write_rpf9_resource, Rpf9Form, Rpf9Sections, NAVIGATION_MESH};

    use super::*;
    use crate::state::opening_an_rpf9_archive::navmesh_cell;
    use crate::state::rpf_archive_open::tree_verbs::tests::synthetic_keys;

    #[test]
    fn a_closed_cell_the_table_holds_no_body_of_is_not_waited_for_and_one_it_holds_is() {
        let near: Vec<(u32, Option<Arc<NavMesh>>)> = vec![(1, None), (2, None), (3, None)];
        let found = BTreeMap::from([(1, false), (2, true)]);
        assert_eq!(closed_of(&near, &found), ["2", "3"]);
        assert_eq!(closed_of(&near, &BTreeMap::new()), ["1", "2", "3"]);
    }

    /// An archive in `dir` of [`navmesh_cell`] at its own grid place and at `others`, each under a
    /// navigation row, its table enciphered under the synthetic key.
    fn cells_at(dir: &Path, others: &[u32]) -> PathBuf {
        let own = navmesh_cell();
        let mut entries = vec![("nav/own.ynv".to_owned(), own.clone())];
        for (i, cell) in others.iter().enumerate() {
            let mut v = own.clone();
            v[0xC8..0xCC].copy_from_slice(&cell.to_le_bytes());
            entries.push((format!("nav/{i}.ynv"), v));
        }
        let entries: Vec<(String, Rpf9NewBody)> = (entries.into_iter())
            .map(|(name, virtual_section)| {
                let s = Rpf9Sections {
                    form: Rpf9Form::BlockMapped,
                    virtual_section,
                    physical: Vec::new(),
                };
                let loose = write_rpf9_resource(&s).expect("written");
                let body = Rpf9NewBody::Resource {
                    type_code: NAVIGATION_MESH,
                    loose,
                };
                (name, body)
            })
            .collect();
        let key = Rpf9IndexKey {
            key_id: 0xDEAD_BEEF,
            iv: [0x33; 16],
        };
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let plan = build(&entries, or_io::platform::Platform::XboxSeries, &key, &keys)
            .expect("the archive builds");
        let mut bytes = Vec::new();
        (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes)).expect("laid out");
        let path = dir.join("cells.rpf");
        std::fs::write(&path, bytes).expect("saved");
        path
    }

    #[test]
    fn a_neighbour_the_table_holds_a_row_of_is_waited_for_and_one_it_does_not_is_not() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (east, west) = (0x1_0001, 0x1_0002);
        let archive = cells_at(dir.path(), &[east]);
        let found = bodies_of(&archive, synthetic_keys(), &[east, west]);
        assert_eq!(found, BTreeMap::from([(east, true), (west, false)]));
        let near = [(east, None), (west, None)];
        assert_eq!(closed_of(&near, &found), [east.to_string()]);
    }

    #[test]
    fn what_a_look_with_no_key_found_is_dropped_once_a_key_is_held() {
        let mut look = Looks {
            running: false,
            found: BTreeMap::from([(1, true)]),
            keyless: true,
        };
        look.refresh(false);
        assert_eq!(look.found.len(), 1, "still no key: the look stands");
        look.refresh(true);
        assert!(
            look.found.is_empty() && !look.keyless,
            "looked for again with the key"
        );
        let mut keyed = Looks {
            found: BTreeMap::from([(1, true)]),
            ..Looks::default()
        };
        keyed.refresh(true);
        assert_eq!(keyed.found.len(), 1, "a look that held a key stands");
    }

    #[test]
    fn with_no_key_or_no_table_every_neighbour_may_hold_a_body() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (east, west) = (0x1_0001, 0x1_0002);
        let archive = cells_at(dir.path(), &[east]);
        let every = BTreeMap::from([(east, true), (west, true)]);
        let no_key = Err("no key".to_owned());
        assert_eq!(bodies_of(&archive, no_key, &[east, west]), every);
        let loose = dir.path().join("loose.rsc");
        std::fs::write(&loose, navmesh_cell()).expect("saved");
        assert_eq!(bodies_of(&loose, synthetic_keys(), &[east, west]), every);
    }
}
