//! Every GTA VI archive under a folder scanned in turn as one row of the job list, with progress
//! and Stop: each archive's rows tallied by kind and kept as a few words a body, so that a kind is
//! found across the archives and a row opens its body as the archive's own list opens it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use or_job::{Cancel, Progress};

use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::rpf9_scan::{body_path, opens_as_bytes, Ended, Row, RowKind, Rpf9ScanDocument, View};
use super::AppState;

pub(crate) use or_title::install::gta6_files::{MOST_ARCHIVES, MOST_FOLDERS};
/// The most bodies the scan keeps.
pub(crate) const MOST_HITS: usize = 1 << 23;
/// How often the job looks at the scan of the archive in hand.
const LOOK: Duration = Duration::from_millis(50);

/// A body a scan found: where it starts, its kind, and the view it opens in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    pub offset: u64,
    pub kind: RowKind,
    /// `None` where no view takes it: the archive's own list says why.
    pub view: Option<View>,
}

/// One archive's scan: how many rows of each kind, its bodies, and how the scan ended.
#[derive(Debug)]
pub(crate) struct ArchiveTally {
    pub path: PathBuf,
    pub counts: BTreeMap<RowKind, usize>,
    pub hits: Vec<Hit>,
    pub ended: Ended,
}

/// What the job has found so far.
#[derive(Debug, Default)]
pub(crate) struct InstallFound {
    pub archives: Vec<ArchiveTally>,
    /// How many archives the folder holds, once the walk has found them.
    pub listed: Option<usize>,
    /// Whether the walk stopped at a limit before it found every archive.
    pub cut: bool,
    pub ended: Option<Ended>,
}

struct InstallScan {
    root: PathBuf,
    cancel: Cancel,
    progress: Progress,
    found: Arc<Mutex<InstallFound>>,
    /// Whether the job list has been told how it ended.
    said: bool,
}

impl Drop for InstallScan {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

// ponytail: one scan for the whole app, of the folder it was last started for.
static INSTALL: Mutex<Option<InstallScan>> = Mutex::new(None);

fn install() -> MutexGuard<'static, Option<InstallScan>> {
    INSTALL.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn lock(found: &Mutex<InstallFound>) -> MutexGuard<'_, InstallFound> {
    found.lock().unwrap_or_else(|p| p.into_inner())
}

/// Scan every archive under `root` on a worker, in place of the scan of any folder before.
pub(crate) fn start(root: &Path) {
    let scan = InstallScan {
        root: root.to_path_buf(),
        cancel: Cancel::new(),
        progress: Progress::new(),
        found: Arc::default(),
        said: false,
    };
    let (root, cancel, progress, found) = (
        scan.root.clone(),
        scan.cancel.clone(),
        scan.progress.clone(),
        Arc::clone(&scan.found),
    );
    std::thread::spawn(move || {
        let ended = scan_every_archive(&root, &found, &cancel, &progress);
        lock(&found).ended = Some(ended);
    });
    *install() = Some(scan);
}

/// What the scan of the archives under `root` has found, while it is the scan held.
pub(crate) fn found_under(root: &Path) -> Option<Arc<Mutex<InstallFound>>> {
    let held = install();
    let scan = held.as_ref().filter(|scan| scan.root == root)?;
    Some(Arc::clone(&scan.found))
}

pub(crate) fn stop() {
    if let Some(scan) = install().as_ref() {
        scan.cancel.set();
    }
}

pub(crate) fn stopping() -> bool {
    install().as_ref().is_some_and(|scan| scan.cancel.is_set())
}

/// The scan while it runs, as the job list draws it.
pub(super) fn job() -> Option<RunningJob> {
    let held = install();
    let scan = held.as_ref()?;
    let found = lock(&scan.found);
    if found.ended.is_some() {
        return None;
    }
    let now = scan.progress.snapshot();
    Some(RunningJob {
        kind: JobKind::ArchiveScan,
        doing: tr!(
            "Scanning every GTA VI archive under {folder}",
            folder = scan.root.display()
        ),
        fraction: Some(now.fraction()),
        detail: tr_n!(
            "{count} archive scanned; now {archive}",
            "{count} archives scanned; now {archive}",
            found.archives.len(),
            archive = now.item
        ),
        stopping: scan.cancel.is_set(),
    })
}

impl AppState {
    /// Say in the job list how the scan of every archive ended, once, and harvest the archives of a scan that ended whole.
    pub(super) fn poll_install_scan(&mut self) {
        self.read_kept_harvest();
        let (told, harvest) = {
            let mut held = install();
            let Some(scan) = held.as_mut().filter(|scan| !scan.said) else {
                return;
            };
            let found = lock(&scan.found);
            let bodies: usize = (found.archives.iter()).map(|a| a.hits.len()).sum();
            let told = match &found.ended {
                None => return,
                Some(Ended::Whole) => (
                    Outcome::Done(Then::Nothing),
                    tr_n!(
                        "{count} GTA VI archive scanned, {bodies} bodies found",
                        "{count} GTA VI archives scanned, {bodies} bodies found",
                        found.archives.len(),
                        bodies = bodies
                    ),
                ),
                Some(Ended::Stopped) => (
                    Outcome::Stopped(Then::Nothing),
                    tr!("Stopped before it finished").to_string(),
                ),
                Some(Ended::Failed(why)) => (Outcome::Failed, why.clone()),
            };
            let whole = found.ended == Some(Ended::Whole);
            let harvest = whole.then(|| (scan.root.clone(), Arc::clone(&scan.found)));
            drop(found);
            scan.said = true;
            (told, harvest)
        };
        self.jobs.record(JobKind::ArchiveScan, told.0, told.1);
        if let Some((root, found)) = harvest {
            self.harvest_install(&root, &found);
        }
    }

    /// Open `hit` of the archive at `archive` as a row of the archive's own list opens it, in the
    /// preview tab where `preview`; the archive's list where no view takes the body.
    pub(crate) fn open_install_hit(&mut self, archive: &Path, hit: Hit, preview: bool) {
        match hit.view {
            Some(View::Archive) => self.load_file(body_path(archive, hit.offset, View::Archive)),
            Some(view) => self.open_scanned_path(body_path(archive, hit.offset, view), preview),
            None => self.load_file(archive.to_path_buf()),
        }
    }
}

/// Scan each archive under `root` as its own tab scans it, reading the scan kept of one already
/// scanned and keeping a scan that reads one whole, and tally it into `found`.
fn scan_every_archive(
    root: &Path,
    found: &Mutex<InstallFound>,
    cancel: &Cancel,
    progress: &Progress,
) -> Ended {
    let listed = or_title::install::gta6_files::list(root);
    let (archives, cut) = (listed.archive_paths(), listed.truncated);
    let sizes: Vec<u64> = (archives.iter())
        .map(|path| std::fs::metadata(path).map_or(0, |m| m.len()))
        .collect();
    let total = sizes.iter().sum::<u64>().max(1);
    {
        let mut found = lock(found);
        (found.listed, found.cut) = (Some(archives.len()), cut);
    }
    let (mut done, mut held) = (0, 0);
    for (path, &size) in archives.iter().zip(&sizes) {
        if cancel.is_set() {
            return Ended::Stopped;
        }
        progress.now_on(file_name(path));
        let scan = match Rpf9ScanDocument::open(path) {
            Ok(Some(scan)) => scan,
            // A head that is neither RPF9's nor missing: another archive, which no scan reads.
            Ok(None) => {
                done += size;
                progress.reached(done, total);
                continue;
            }
            Err(why) => {
                lock(found).archives.push(ArchiveTally {
                    path: path.clone(),
                    counts: BTreeMap::new(),
                    hits: Vec::new(),
                    ended: Ended::Failed(why),
                });
                done += size;
                progress.reached(done, total);
                continue;
            }
        };
        loop {
            if cancel.is_set() {
                scan.stop();
            }
            let reached = {
                let scanned = scan.found();
                if scanned.ended.is_some() {
                    break;
                }
                scanned.reached
            };
            progress.reached(done + reached.min(size), total);
            std::thread::sleep(LOOK);
        }
        let tally = tally_of(path, &scan, &mut held);
        lock(found).archives.push(tally);
        done += size;
        progress.reached(done, total);
    }
    match cancel.is_set() {
        true => Ended::Stopped,
        false => Ended::Whole,
    }
}

/// How many rows of each kind `scan`, the ended scan of the archive at `path`, lists, and its
/// bodies, while fewer than [`MOST_HITS`] are `held` in all.
fn tally_of(path: &Path, scan: &Rpf9ScanDocument, held: &mut usize) -> ArchiveTally {
    let found = scan.found();
    let (mut counts, mut hits) = (BTreeMap::new(), Vec::new());
    for row in &found.rows {
        let kind = RowKind::of(row);
        *counts.entry(kind).or_insert(0) += 1;
        let Row::Body(find) = row else {
            continue;
        };
        if *held >= MOST_HITS {
            continue;
        }
        let view = match found.view(find) {
            Ok(view) => Some(view),
            Err(_) => (opens_as_bytes(find) && super::gta6_first_entry::note(path, &found, find))
                .then_some(View::Bytes),
        };
        hits.push(Hit {
            offset: find.offset,
            kind,
            view,
        });
        *held += 1;
    }
    ArchiveTally {
        path: path.to_path_buf(),
        counts,
        hits,
        ended: found.ended.clone().unwrap_or(Ended::Stopped),
    }
}

/// The most bodies the Find of the list across the archives examines in one frame, so that
/// millions are filtered over several frames and never in one.
const FIND_BUDGET: usize = 50_000;

/// The bodies the list across the archives shows: those of the kinds toggled on, every kind when
/// none is, whose line holds the Find field's words; where the field states an offset, each
/// archive's body that starts last at or before it.
#[derive(Debug, Default)]
pub(crate) struct InstallFind {
    /// The offset the field states, else its words lowercased, and the kinds the rows are for.
    built: (Option<u64>, String, BTreeSet<RowKind>),
    /// The archive, and the body of it, examined next.
    next: (usize, usize),
    shown: Vec<(usize, usize)>,
}

impl InstallFind {
    /// The bodies of `found` shown for `text` and `kinds`, each as its archive's index and its own
    /// in scan order, and whether any are still to be examined; `line` is a body's line in the list.
    pub(crate) fn shown(
        &mut self,
        found: &InstallFound,
        text: &str,
        kinds: &BTreeSet<RowKind>,
        line: impl Fn(&ArchiveTally, &Hit) -> String,
    ) -> (&[(usize, usize)], bool) {
        let key = match offset_in(text) {
            Some(offset) => (Some(offset), String::new(), kinds.clone()),
            None => (None, text.trim().to_lowercase(), kinds.clone()),
        };
        if key != self.built || self.next.0 > found.archives.len() {
            *self = Self {
                built: key,
                ..Self::default()
            };
        }
        let Self {
            built: (offset, words, kinds),
            next,
            shown,
        } = self;
        let admits = |hit: &Hit| kinds.is_empty() || kinds.contains(&hit.kind);
        let mut spent = 0;
        while spent < FIND_BUDGET {
            let Some(archive) = found.archives.get(next.0) else {
                break;
            };
            match *offset {
                Some(offset) => {
                    // A scan lists its bodies in the order they start.
                    let before = archive.hits.partition_point(|h| h.offset <= offset);
                    let at = archive.hits[..before].iter().rposition(admits);
                    spent += 1 + before - at.unwrap_or(0);
                    if let Some(at) = at {
                        shown.push((next.0, at));
                    }
                    *next = (next.0 + 1, 0);
                }
                None => {
                    let hits = &archive.hits[next.1.min(archive.hits.len())..];
                    let take = hits.len().min(FIND_BUDGET - spent);
                    for (i, hit) in hits[..take].iter().enumerate() {
                        let holds = words.is_empty()
                            || line(archive, hit).to_lowercase().contains(words.as_str());
                        if admits(hit) && holds {
                            shown.push((next.0, next.1 + i));
                        }
                    }
                    spent += take.max(1);
                    *next = match take == hits.len() {
                        true => (next.0 + 1, 0),
                        false => (next.0, next.1 + take),
                    };
                }
            }
        }
        (shown.as_slice(), next.0 < found.archives.len())
    }
}

/// The offset `text` states, written in hexadecimal after `0x`, as an archive's own list reads it.
pub(crate) fn offset_in(text: &str) -> Option<u64> {
    let text = text.trim();
    let hex = (text.strip_prefix("0x")).or_else(|| text.strip_prefix("0X"))?;
    u64::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_archives_under_a_folder_are_scanned_in_turn_and_tallied_apart() {
        let dir = tempfile::tempdir().expect("a folder");
        let deep = dir.path().join("x64");
        std::fs::create_dir_all(&deep).expect("a folder");
        // Two archives whose heads have not arrived, of two sizes, and two files of no archive.
        let (first, second) = (deep.join("a.rpf"), dir.path().join("b.RPF"));
        std::fs::write(&first, vec![0u8; 0x400]).expect("written");
        std::fs::write(&second, vec![0u8; 0x900]).expect("written");
        std::fs::write(
            dir.path().join("c.rpf"),
            b"RPF7 and more than a head of other bytes",
        )
        .expect("written");
        std::fs::write(dir.path().join("notes.txt"), vec![0u8; 0x400]).expect("written");

        let (found, cancel, progress) = (Mutex::default(), Cancel::new(), Progress::new());
        let ended = scan_every_archive(dir.path(), &found, &cancel, &progress);
        assert_eq!(ended, Ended::Whole);
        let found = lock(&found);
        assert_eq!(found.listed, Some(3));
        let tallied: Vec<&Path> = found.archives.iter().map(|a| a.path.as_path()).collect();
        let mut scanned = vec![first.as_path(), second.as_path()];
        scanned.sort();
        assert_eq!(tallied, scanned, "the other archive is not scanned");
        for archive in &found.archives {
            let alone = Rpf9ScanDocument::open(&archive.path)
                .expect("read")
                .expect("scanned");
            while alone.found().ended.is_none() {
                std::thread::sleep(LOOK);
            }
            let rows = alone.found().rows.len();
            assert_eq!(
                archive.counts.values().sum::<usize>(),
                rows,
                "{}: every row its own list holds is counted once",
                archive.path.display()
            );
            let bodies = (alone.found().rows.iter())
                .filter(|r| matches!(r, Row::Body(_)))
                .count();
            assert_eq!(archive.hits.len(), bodies);
        }
        assert_eq!(progress.snapshot().fraction(), 1.0);

        let stopped = Cancel::new();
        stopped.set();
        let (none, progress) = (Mutex::default(), Progress::new());
        assert_eq!(
            scan_every_archive(dir.path(), &none, &stopped, &progress),
            Ended::Stopped
        );
        assert!(
            lock(&none).archives.is_empty(),
            "a stop before the first scans none"
        );
    }

    fn tally(name: &str, hits: &[(u64, RowKind)]) -> ArchiveTally {
        ArchiveTally {
            path: PathBuf::from(name),
            counts: BTreeMap::new(),
            hits: (hits.iter())
                .map(|&(offset, kind)| Hit {
                    offset,
                    kind,
                    view: None,
                })
                .collect(),
            ended: Ended::Whole,
        }
    }

    fn line(archive: &ArchiveTally, hit: &Hit) -> String {
        format!("{}  {:#012X}", archive.path.display(), hit.offset)
    }

    #[test]
    fn the_find_across_the_archives_keeps_words_and_goes_to_an_offset_in_each() {
        let (model, text) = (RowKind::Model, RowKind::Text);
        let mut found = InstallFound::default();
        found.archives.push(tally(
            "a.rpf",
            &[(0x100, model), (0x200, text), (0x300, model)],
        ));
        found
            .archives
            .push(tally("b.rpf", &[(0x50, text), (0x250, model)]));
        let none = BTreeSet::new();
        let models = BTreeSet::from([model]);
        let texts = BTreeSet::from([text]);
        let mut find = InstallFind::default();
        let mut shown = |text: &str, kinds: &BTreeSet<RowKind>| {
            let (rows, examining) = find.shown(&found, text, kinds, line);
            assert!(!examining, "{text}: five bodies take one frame");
            rows.to_vec()
        };
        assert_eq!(shown("0x260", &none), [(0, 1), (1, 1)]);
        assert_eq!(shown(" 0X260 ", &models), [(0, 0), (1, 1)]);
        assert!(
            shown("0x10", &none).is_empty(),
            "no body starts at or before it"
        );
        assert_eq!(shown("B.RPF", &none), [(1, 0), (1, 1)]);
        assert_eq!(shown("300", &none), [(0, 2)]);
        assert_eq!(shown("", &texts), [(0, 1), (1, 0)]);
        assert_eq!(shown("a.rpf", &texts), [(0, 1)]);
        assert_eq!(offset_in("0x1F"), Some(0x1F));
        assert_eq!(offset_in("1F"), None);
        assert_eq!(offset_in("0xZZ"), None);
    }

    #[test]
    fn the_find_across_the_archives_examines_a_budget_a_frame_and_an_added_archive_alone() {
        let many: Vec<(u64, RowKind)> = (0..FIND_BUDGET as u64 + 5)
            .map(|i| (i * 0x10, RowKind::Model))
            .collect();
        let mut found = InstallFound::default();
        found.archives.push(tally("a.rpf", &many));
        let kinds = BTreeSet::from([RowKind::Model]);
        let mut find = InstallFind::default();
        let (rows, examining) = find.shown(&found, "", &kinds, line);
        assert_eq!((rows.len(), examining), (FIND_BUDGET, true));
        let (rows, examining) = find.shown(&found, "", &kinds, line);
        assert_eq!((rows.len(), examining), (many.len(), false));

        found.archives.push(tally(
            "b.rpf",
            &[(0x10, RowKind::Model), (0x20, RowKind::Text)],
        ));
        let (rows, examining) = find.shown(&found, "", &kinds, line);
        assert!(!examining);
        assert_eq!(rows.len(), many.len() + 1, "only the added archive's body");
        assert_eq!(rows.last(), Some(&(1, 0)));

        let (rows, _) = find.shown(&found, "", &BTreeSet::new(), line);
        assert_eq!(
            rows.len(),
            FIND_BUDGET,
            "a change of the kinds examines every body again"
        );
    }
}
