//! Names placed by counting the entries a nested archive's header states: added to a whole
//! archive's list once its scan ends, and ending the list of a nested archive opened from it.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use or_io::container::rpf::{rpf9_names_by_count_at, Rpf9BodyKind, Rpf9NamesByCount, Rpf9ScanFind};
use or_job::{Cancel, Progress};

use super::rpf9_scan::{Ended, Found, Row, MAX_ROWS};

/// The names each nested archive opened from its parent's list ends with, by archive and header.
// ponytail: one map for the whole app, which no close empties; a find is a few dozen bytes.
static NAMES: Mutex<BTreeMap<(PathBuf, u64), Rpf9ScanFind>> = Mutex::new(BTreeMap::new());

fn known() -> MutexGuard<'static, BTreeMap<(PathBuf, u64), Rpf9ScanFind>> {
    (NAMES.lock()).unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock(found: &Mutex<Found>) -> MutexGuard<'_, Found> {
    found
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keep the names `found`, the list of `archive`, holds for the archive nested at `offset`.
pub(super) fn remember(archive: &Path, found: &Found, offset: u64) {
    let own = Rpf9BodyKind::Names { archive: offset };
    let names = found.rows.iter().find_map(|row| match row {
        Row::Body(find) if find.kind == own => Some(**find),
        _ => None,
    });
    if let Some(names) = names {
        known().insert((archive.to_path_buf(), offset), names);
    }
}

/// The scan of `path` from `start`, with the names placed by count added once a whole file's
/// scan ends; a nested archive whose names are known is scanned to them and ended there.
pub(super) fn counted_scan(
    path: &Path,
    start: u64,
    len: u64,
    found: &Mutex<Found>,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Ended, String> {
    let key = (path.to_path_buf(), start);
    let names = known().get(&key).copied().filter(|_| start > 0);
    let Some(names) = names else {
        let ended = super::rpf9_scan::scan(path, start, len, found, cancel, progress)?;
        if start == 0 && ended == Ended::Whole && !cancel.is_set() {
            if let Err(error) = place(path, found) {
                tracing::warn!(%error, path = %path.display(), "no names were placed by count");
            }
        }
        // Opened by its path, no list gave it names: they are placed by counting its entries.
        if start > 0 && matches!(ended, Ended::Failed(_)) && !cancel.is_set() {
            if let Some(names) = by_count(path, start) {
                return Ok(end_at(found, start, names, ended));
            }
        }
        return Ok(ended);
    };
    let to = names.offset.min(len);
    let ended = super::rpf9_scan::scan(path, start, to, found, cancel, progress)?;
    Ok(end_at(found, start, names, ended))
}

/// The names of the archive nested at `start` of the file at `path`, placed by counting the
/// entries its header states; `None` where no count places them.
// ponytail: a second scan of the archive's entries with no stop, run only after a scan failed.
fn by_count(path: &Path, start: u64) -> Option<Rpf9ScanFind> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()));
    let placed =
        file.and_then(|file| rpf9_names_by_count_at(&file, start).map_err(|e| e.to_string()));
    placed.unwrap_or_else(|error| {
        tracing::warn!(%error, path = %path.display(), "no names were placed by count");
        None
    })
}

/// `ended`, the scan into `found` of the archive nested at `start`, ended at `names`, its names,
/// where it went past them.
fn end_at(found: &Mutex<Found>, start: u64, names: Rpf9ScanFind, ended: Ended) -> Ended {
    let mut found = lock(found);
    // The scan goes on past the names to a header or the file's end, where it fails.
    match ended {
        Ended::Failed(_) if found.reached >= names.offset => {}
        ended => return ended,
    }
    let before = |row: &Row| match row {
        Row::Body(find) => find.offset < names.offset,
        Row::Missing { from, .. } => *from < names.offset,
    };
    found.rows.retain(before);
    found.rows.push(Row::Body(Box::new(names)));
    found.bodies = bodies(&found.rows);
    let end = names.offset + names.span;
    found.ends.insert(start, end);
    found.reached = end;
    Ended::Whole
}

fn bodies(rows: &[Row]) -> usize {
    let body = |row: &&Row| match row {
        Row::Body(find) => !matches!(find.kind, Rpf9BodyKind::Archive(_)),
        Row::Missing { .. } => false,
    };
    rows.iter().filter(body).count()
}

/// Add to `found`, a whole scan of the file at `path`, the names placed by count.
fn place(path: &Path, found: &Mutex<Found>) -> Result<(), String> {
    let mut count = Rpf9NamesByCount::default();
    for row in &lock(found).rows {
        if let Row::Body(find) = row {
            count.push(find);
        }
    }
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let placed = count.place(&file).map_err(|e| e.to_string())?;
    let mut found = lock(found);
    for names in placed {
        let Rpf9BodyKind::Names { archive } = names.kind else {
            continue;
        };
        let at = found
            .rows
            .partition_point(|row| offset(row) <= names.offset);
        found.rows.insert(at, Row::Body(Box::new(names)));
        found.ends.insert(archive, names.offset + names.span);
        found.bodies += 1;
    }
    found.rows.truncate(MAX_ROWS);
    Ok(())
}

fn offset(row: &Row) -> u64 {
    match row {
        Row::Body(find) => find.offset,
        Row::Missing { from, .. } => *from,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::opening_an_rpf9_archive::{
        row_at, rows, rpf9_head, scanned, settle, written, zlib,
    };
    use super::super::{AppState, DocPayload};
    use super::*;

    const NESTED: u64 = 0x1000;

    fn xml(root: &str) -> Vec<u8> {
        format!("<?xml version=\"1.0\"?>\n<{root}/>\n").into_bytes()
    }

    fn offsets(rows: &[Row]) -> Vec<u64> {
        rows.iter().map(offset).collect()
    }

    /// A nested archive of one entry whose names stand before a top-level body, no header, and
    /// where its names start.
    fn counted_archive() -> (Vec<u8>, u64) {
        // Two entries' index takes 32 bytes after parts A to C's 0x190.
        let mut bytes = rpf9_head(2, 0x1B0);
        bytes.resize(NESTED as usize, 0);
        bytes.extend(rpf9_head(1, 0x1B0));
        bytes.extend(zlib(&xml("CMapTypes")));
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        let names_at = bytes.len() as u64;
        bytes.extend([0x33; 0x40]);
        bytes.extend(zlib(&xml("CMapData")));
        (bytes, names_at)
    }

    #[test]
    fn a_nested_archive_opened_by_its_path_ends_at_the_names_placed_by_count() {
        use super::super::rpf9_scan::{body_path, View};
        use or_io::container::rpf::Rpf9Scan;

        let (bytes, names_at) = counted_archive();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = written(dir.path(), &bytes);
        let file = File::open(&path).expect("open");
        let streamed = (Rpf9Scan::new(&file, 0..u64::MAX).expect("a scan"))
            .map(|find| find.expect("a find"))
            .any(|find| matches!(find.kind, Rpf9BodyKind::Names { .. }));
        assert!(!streamed, "the stream scan places no names here");

        // A fresh state: no list of the parent was opened, so no names were kept for it.
        let mut state = AppState::new();
        let nested = scanned(&mut state, body_path(&path, NESTED, View::Archive));
        let Some(DocPayload::Rpf9(scan)) = state.document(nested).map(|d| &d.payload) else {
            panic!("not a GTA VI archive document");
        };
        assert_eq!(scan.start, NESTED);
        let found = scan.found();
        assert_eq!(found.ended, Some(Ended::Whole), "{:?}", found.ended);
        assert_eq!(found.reached, names_at + 0x40);
        assert_eq!(offsets(&found.rows), [NESTED, NESTED + 0x1B0, names_at]);
        assert_eq!(found.bodies, 2);
        assert_eq!(found.ends.get(&NESTED), Some(&(names_at + 0x40)));
    }

    #[test]
    fn names_placed_by_count_are_listed_and_end_the_nested_archive_opened_from_the_list() {
        let (bytes, names_at) = counted_archive();
        let top_level = names_at + 0x40;
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = written(dir.path(), &bytes);
        let mut state = AppState::new();
        let parent = scanned(&mut state, path.clone());
        let listed = rows(&state, parent);
        let entry = NESTED + 0x1B0;
        assert_eq!(
            offsets(&listed),
            [0, NESTED, entry, names_at, top_level],
            "{listed:?}"
        );

        state.open_scanned_body(parent, row_at(&listed, NESTED));
        settle(&mut state);
        let nested = state.active_document.expect("an open document");
        assert_ne!(nested, parent);
        let Some(DocPayload::Rpf9(scan)) = state.document(nested).map(|d| &d.payload) else {
            panic!(
                "not a GTA VI archive document: {}",
                state.error_dialog_message
            );
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while scan.found().ended.is_none() {
            assert!(Instant::now() < deadline, "the scan never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        let found = scan.found();
        assert_eq!(found.ended, Some(Ended::Whole));
        assert_eq!(found.reached, names_at + 0x40);
        assert_eq!(offsets(&found.rows), [NESTED, entry, names_at]);
        assert_eq!(found.bodies, 2);
    }
}
