//! Which of a scanned GTA VI cell's neighbours the scan of its archive holds a body of, looked for
//! on a worker: no special link names a cell that has no body, so a deletion takes such a cell as
//! having no link into the edited one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Role, Rpf9ScanFind, Rpf9ZlibRun};
use or_io::gta6::headerless::NavMesh;

use crate::state::rpf9_scan::{Ended, Row};
use crate::state::{AppState, DocPayload};

/// Where a navigation cell's root states its cell.
const CELL: usize = 0xC8;
/// The most a cell's body inflates to.
const BODY_LIMIT: u64 = 1 << 30;

/// What the looks through one archive's scan found: per cell, whether it holds a body of it.
#[derive(Debug, Default)]
struct Looks {
    running: bool,
    found: BTreeMap<u32, bool>,
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

/// Whether the scan `finds` of `archive` holds a body of each of `wanted`; `None` where a body does
/// not read, as it may be one of them.
fn bodies_of(
    archive: &Path,
    finds: &[Rpf9ScanFind],
    wanted: &[u32],
) -> Option<BTreeMap<u32, bool>> {
    let file = File::open(archive).ok()?;
    let mut held = BTreeSet::new();
    for find in finds {
        let (read, bytes) = read_rpf9_body(&file, find.offset, BODY_LIMIT).ok()?;
        let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            ..
        }) = read.kind
        else {
            return None;
        };
        let cell = resource.sections(&bytes).0.get(CELL..CELL + 4)?;
        held.insert(u32::from_le_bytes(cell.try_into().ok()?));
    }
    Some(wanted.iter().map(|c| (*c, held.contains(c))).collect())
}

/// Look through the scan `finds` of `archive` on a worker for a body of each of `wanted`; a look
/// a body refuses records every wanted cell as one that may hold a body, so it is not looked for again.
fn look_through(archive: PathBuf, finds: Vec<Rpf9ScanFind>, wanted: Vec<u32>) {
    std::thread::spawn(move || {
        let found = bodies_of(&archive, &finds, &wanted);
        let mut held = looks();
        let look = held.entry(archive).or_default();
        look.running = false;
        let found = found.unwrap_or_else(|| wanted.iter().map(|c| (*c, true)).collect());
        look.found.extend(found);
    });
}

/// Whether a scan's row is a headerless navigation cell.
fn is_cell(find: &Rpf9ScanFind) -> bool {
    matches!(find.kind, Rpf9BodyKind::Zlib(Rpf9ZlibRun { signature: Some(s), resource: Some(_), .. })
        if s.role() == Some(Rpf9Role::NavMesh))
}

impl AppState {
    /// The cells of `near` whose body is not open and that the scan of the archive the body at
    /// `path` came from may hold a body of; `Err` with the cells a look through that scan, started
    /// here where none runs, has still to find.
    pub(super) fn unopened(
        &self,
        path: &Path,
        near: &[(u32, Option<Arc<NavMesh>>)],
    ) -> Result<Vec<String>, String> {
        let archive = path.parent().unwrap_or(path);
        let mut held = looks();
        let look = held.entry(archive.to_path_buf()).or_default();
        let unknown: Vec<u32> = (near.iter())
            .filter(|(c, body)| body.is_none() && !look.found.contains_key(c))
            .map(|(c, _)| *c)
            .collect();
        if unknown.is_empty() {
            return Ok(closed_of(near, &look.found));
        }
        if !look.running {
            let finds = self.cell_finds(archive);
            // With no scan open, a closed cell may hold a body, as before any look.
            if finds.is_empty() {
                return Ok(closed_of(near, &look.found));
            }
            look.running = true;
            look_through(archive.to_path_buf(), finds, unknown.clone());
        }
        let cells: Vec<String> = unknown.iter().map(u32::to_string).collect();
        Err(cells.join(", "))
    }

    /// The navigation cells the open scans of `archive` found, of those that read the archive whole:
    /// a scan stopped, failed or still running has not met every body.
    fn cell_finds(&self, archive: &Path) -> Vec<Rpf9ScanFind> {
        (self.documents.iter())
            .filter_map(|d| match &d.payload {
                DocPayload::Rpf9(scan) if scan.path == archive => Some(Arc::clone(scan)),
                _ => None,
            })
            .flat_map(|scan| {
                let found = scan.found();
                if found.ended != Some(Ended::Whole) {
                    return Vec::new();
                }
                (found.rows.iter())
                    .filter_map(|row| match row {
                        Row::Body(find) if is_cell(find) => Some(**find),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// Wait for the look through the scan of `archive` to end.
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
    use super::*;

    #[test]
    fn a_closed_cell_the_scan_holds_no_body_of_is_not_waited_for_and_one_it_holds_is() {
        let near: Vec<(u32, Option<Arc<NavMesh>>)> = vec![(1, None), (2, None), (3, None)];
        let found = BTreeMap::from([(1, false), (2, true)]);
        assert_eq!(closed_of(&near, &found), ["2", "3"]);
        assert_eq!(closed_of(&near, &BTreeMap::new()), ["1", "2", "3"]);
    }
}
