//! A scan that read a whole GTA VI archive is kept in the configuration folder, and the archive is
//! listed from it, unread, until its size or its modification time changes or a person asks for a scan.

use crate::state::Rpf9Document;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use super::finding_a_scanned_body::nested_archive;
use super::jobs::JobKind;
use super::opening_an_rpf9_archive::{archive, files_and_centre, written};
use super::prefs::keep_prefs_in;
use super::rpf9_scan::{Ended, Found, Row, Rpf9ScanDocument};
use super::{AppState, DocPayload};

/// A fresh app whose configuration folder is `config`, and the archive at `path` open in it.
///
/// ⚠ Opened on this thread, whose configuration folder `config` is: a load's worker is another
/// thread, whose scans are kept in the scratch every test of the binary shares.
fn opened(config: &Path, path: &Path) -> (AppState, Arc<Rpf9ScanDocument>) {
    keep_prefs_in(config);
    let mut state = AppState::new();
    let scan = Rpf9ScanDocument::open(path).expect("read");
    let scan = Arc::new(scan.expect("a GTA VI archive"));
    state.insert_document(
        path.to_path_buf(),
        DocPayload::Rpf9(Rpf9Document::Scan(Arc::clone(&scan))),
        None,
        None,
    );
    (state, scan)
}

/// Wait for the list of `scan` to end, and give its rows, its four totals followed by where each
/// nested archive starts and ends, and whether it was kept.
fn listed(scan: &Rpf9ScanDocument) -> (Vec<Row>, Vec<u64>, bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while scan.found().ended.is_none() {
        assert!(Instant::now() < deadline, "the list never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let found = scan.found();
    assert_eq!(found.ended, Some(Ended::Whole));
    assert!(!found.reading_kept, "still reading a kept scan");
    let totals = [
        found.bodies as u64,
        found.reached,
        found.missing,
        found.unclaimed,
    ];
    let ends = found.ends.iter().flat_map(|(at, end)| [*at, *end]);
    let totals = totals.into_iter().chain(ends).collect();
    (found.rows.clone(), totals, found.kept.is_some())
}

/// The files the scans of `config` are kept in.
fn kept_files(config: &Path) -> Vec<PathBuf> {
    let Ok(folder) = std::fs::read_dir(config.join("gta6_scans")) else {
        return Vec::new();
    };
    folder.map(|entry| entry.expect("listed").path()).collect()
}

pub(super) fn modified(path: &Path) -> SystemTime {
    let meta = std::fs::metadata(path).expect("there");
    meta.modified().expect("a modification time")
}

/// Write `bytes` over the file at `path` and state `time` as when it was last modified.
pub(super) fn rewritten(path: &Path, bytes: &[u8], time: SystemTime) {
    std::fs::write(path, bytes).expect("written");
    let file = File::options().write(true).open(path).expect("opened");
    file.set_modified(time).expect("timed");
}

/// `bytes` with nothing after its head, so a scan of it lists no body.
fn emptied(bytes: &[u8]) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    bytes[0x40..].fill(0);
    bytes
}

/// A configuration folder, and an archive of one body of every kind a scan finds, then a nested
/// archive whose names the scan places.
fn scratch() -> (tempfile::TempDir, tempfile::TempDir, Vec<u8>, PathBuf) {
    let config = tempfile::tempdir().expect("a configuration folder");
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (mut bytes, _) = archive();
    bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
    // Past the head and the zeros of the archive that holds it.
    bytes.extend_from_slice(&nested_archive().0[0x1000..]);
    let path = written(dir.path(), &bytes);
    (config, dir, bytes, path)
}

#[test]
fn an_archive_scanned_to_its_end_is_listed_from_its_kept_scan_and_not_read_again() {
    let (config, _dir, bytes, path) = scratch();
    let (_state, scan) = opened(config.path(), &path);
    let (rows, totals, kept) = listed(&scan);
    assert!(!kept, "the first list is a scan");
    assert!(rows.len() > 10, "{rows:?}");
    assert!(totals.len() > 4, "no nested archive's end was placed");
    assert_eq!(kept_files(config.path()).len(), 1);

    // Nothing after the head is left for a scan to find: only the kept scan lists the bodies.
    rewritten(&path, &emptied(&bytes), modified(&path));
    let (state, scan) = opened(config.path(), &path);
    assert_eq!(listed(&scan), (rows, totals, true));
    let jobs = state.running_jobs();
    assert!(jobs.iter().all(|job| job.kind != JobKind::ArchiveScan));
    assert_eq!(kept_files(config.path()).len(), 1);
}

#[test]
fn an_archive_of_another_size_or_time_is_scanned_and_its_kept_scan_replaced() {
    let (config, _dir, bytes, path) = scratch();
    listed(&opened(config.path(), &path).1);
    let written_at = modified(&path);

    // A byte longer, and written when it was.
    let mut longer = bytes.clone();
    longer.push(0);
    rewritten(&path, &longer, written_at);
    let (_, totals, kept) = listed(&opened(config.path(), &path).1);
    assert!(
        !kept,
        "an archive a byte longer was listed from its kept scan"
    );
    assert_eq!(totals[1], longer.len() as u64);
    assert!(listed(&opened(config.path(), &path).1).2, "not kept anew");

    // The same bytes, written ten seconds later.
    rewritten(&path, &longer, written_at + Duration::from_secs(10));
    let (.., kept) = listed(&opened(config.path(), &path).1);
    assert!(
        !kept,
        "an archive written again was listed from its kept scan"
    );
    assert!(listed(&opened(config.path(), &path).1).2, "not kept anew");
    assert_eq!(kept_files(config.path()).len(), 1);
}

#[test]
fn a_stopped_scan_keeps_nothing() {
    let config = tempfile::tempdir().expect("a configuration folder");
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("levels.rpf");
    let file = File::create(&path).expect("created");
    file.set_len(1 << 30).expect("a long file of zeros");
    drop(file);
    let (_state, scan) = opened(config.path(), &path);
    scan.stop();
    let deadline = Instant::now() + Duration::from_secs(30);
    while scan.found().ended.is_none() {
        assert!(Instant::now() < deadline, "the scan never stopped");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(scan.found().ended, Some(Ended::Stopped));
    assert_eq!(kept_files(config.path()), Vec::<PathBuf>::new());
}

#[test]
fn a_kept_scan_cut_in_half_or_of_another_version_is_ignored_and_the_archive_scanned() {
    let (config, _dir, _bytes, path) = scratch();
    let (rows, totals, _) = listed(&opened(config.path(), &path).1);
    let [file] = kept_files(config.path()).try_into().expect("one file");
    let whole = std::fs::read(&file).expect("read");
    let mut another_version = whole.clone();
    another_version[4] += 1;
    for damaged in [&whole[..whole.len() / 2], &another_version] {
        std::fs::write(&file, damaged).expect("written");
        let scanned = listed(&opened(config.path(), &path).1);
        assert_eq!(scanned, (rows.clone(), totals.clone(), false));
        assert!(listed(&opened(config.path(), &path).1).2, "not kept anew");
    }
}

#[test]
fn a_scan_kept_under_other_scan_rules_is_ignored_and_the_archive_scanned() {
    use or_io::container::rpf::{seal_rpf9_scan, unseal_rpf9_scan};
    let (config, _dir, _bytes, path) = scratch();
    let (rows, totals, _) = listed(&opened(config.path(), &path).1);
    let [file] = kept_files(config.path()).try_into().expect("one file");
    let sealed = std::fs::read(&file).expect("read");
    let mut plain = unseal_rpf9_scan(&sealed, 1 << 30).expect("unsealed");
    // The scan's rules are the second word of what a kept scan seals.
    assert_eq!(plain[8..16], super::rpf9_kept::RULES.to_le_bytes());
    plain[8] ^= 1;
    std::fs::write(&file, seal_rpf9_scan(&plain).expect("sealed")).expect("written");
    let scanned = listed(&opened(config.path(), &path).1);
    assert_eq!(scanned, (rows, totals, false));
    assert!(listed(&opened(config.path(), &path).1).2, "not kept anew");
}

#[test]
fn scan_again_on_a_kept_list_scans_the_archive_and_replaces_the_kept_scan() {
    use super::nothing_draws_over_what_it_belongs_to::{painted, Painted, Panel};
    let _serial = crate::ui::theme::serial();
    let (config, _dir, bytes, path) = scratch();
    let (mut state, scan) = opened(config.path(), &path);
    let (rows, ..) = listed(&scan);
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(900.0, 900.0),
    ));
    let mut frames = |state: &mut AppState, events: Vec<egui::Event>| {
        let mut output = panel.frame(events, |ui| files_and_centre(ui, state));
        for _ in 0..2 {
            output = panel.frame(Vec::new(), |ui| files_and_centre(ui, state));
        }
        painted(&output)
    };
    let says =
        |texts: &[Painted], words: &str| texts.iter().any(|painted| painted.text.contains(words));
    let texts = frames(&mut state, Vec::new());
    assert!(says(&texts, "read the whole file"), "{texts:?}");
    assert!(!says(&texts, "Scan again"), "a scan offers a scan again");

    // The archive changes under its size and time: its kept scan lists what is gone.
    rewritten(&path, &emptied(&bytes), modified(&path));
    let (mut state, scan) = opened(config.path(), &path);
    assert_eq!(listed(&scan).0, rows);
    let texts = frames(&mut state, Vec::new());
    assert!(says(&texts, "a scan kept just now"), "{texts:?}");
    let at = (texts.iter())
        .find(|painted| painted.text.trim() == "Scan again")
        .expect("the button is drawn")
        .rect
        .center();
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frames(&mut state, vec![egui::Event::PointerMoved(at), press(true)]);
    frames(&mut state, vec![press(false)]);
    let (again, _, kept) = listed(&scan);
    assert!(!kept, "the list is still the kept scan");
    assert!(again.len() < rows.len(), "{again:?}");
    let (kept_again, _, kept) = listed(&opened(config.path(), &path).1);
    assert!(kept, "the new scan was not kept");
    assert_eq!(kept_again, again);
    assert_eq!(kept_files(config.path()).len(), 1);
}

#[test]
fn a_reload_of_an_archive_s_tab_scans_the_archive_again() {
    let (config, _dir, bytes, path) = scratch();
    let (mut state, scan) = opened(config.path(), &path);
    let (rows, ..) = listed(&scan);

    // The archive changes on disk, to one a scan lists no body of.
    let later = modified(&path) + Duration::from_secs(10);
    rewritten(&path, &emptied(&bytes), later);
    assert_eq!(state.reload_open_file(&path, false), Ok(1));
    let tab = state.documents.iter().find_map(|d| match &d.payload {
        DocPayload::Rpf9(Rpf9Document::Scan(scan)) => Some(Arc::clone(scan)),
        _ => None,
    });
    let again = tab.expect("the tab is still the archive's scan");
    assert!(!Arc::ptr_eq(&scan, &again), "the tab kept the scan it had");
    let (listed_again, _, kept) = listed(&again);
    assert!(
        !kept,
        "an archive written again was listed from its kept scan"
    );
    assert!(listed_again.len() < rows.len(), "{listed_again:?}");
}

#[test]
fn a_kept_scan_being_read_says_so_and_is_no_scan_job() {
    use super::nothing_draws_over_what_it_belongs_to::{painted, Panel};
    let _serial = crate::ui::theme::serial();
    let (config, _dir, _bytes, path) = scratch();
    let (mut state, scan) = opened(config.path(), &path);
    listed(&scan);
    // As the list is from its open until the worker has read the kept scan.
    *scan.found() = Found {
        reading_kept: true,
        ..Found::default()
    };
    assert!(
        scan.job().is_none(),
        "reading a kept scan is listed as a scan"
    );
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(900.0, 900.0),
    ));
    let mut output = None;
    for _ in 0..3 {
        output = Some(panel.frame(Vec::new(), |ui| files_and_centre(ui, &mut state)));
    }
    let texts = painted(&output.expect("drawn"));
    let says = |words: &str| texts.iter().any(|painted| painted.text.contains(words));
    assert!(says("Reading the kept scan…"), "{texts:?}");
    assert!(!says("Scanned"), "{texts:?}");
}
