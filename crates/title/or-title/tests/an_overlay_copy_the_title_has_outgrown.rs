//! What tells an overlay copy of an archive that the title has since patched the archive it came from.
//!
//! The copy is the user's: saving into it rebuilds the whole archive from decoded entries, so its own
//! bytes, size and time say nothing about the title. Staleness is a fact about the install's archive
//! against a record made when the copy was. Four candidates for that record were weighed, and what
//! each costs and sees is what the tests below hold.
//!
//! | candidate | reads | sees a patch | cries wolf on |
//! |---|---|---|---|
//! | the original's size and modified time | one stat | any rewrite | a touch, a verify, a restore, a move |
//! | a hash of the original's bytes | the whole file | any change | nothing |
//! | the entry table | the table, a few MiB | a changed size, flag or name | nothing |
//! | a digest of every entry | the whole file, decoded | any change | nothing |
//!
//! The record keeps the first and the third. Size and time settle the common case, an archive nobody
//! touched, with a stat. When they differ the table settles it, which reads the table and no body: a
//! touched archive has the table it had, a patched one has not. The hashes cost the gigabytes the
//! roadmap row called out, and the table already holds each entry's size and flags, which a patch
//! that replaces a file moves.
//!
//! Measured by `a_retail_archive_is_told_from_a_touched_one_and_from_another_by_its_table` at
//! `00b079d2` with this change, in the test profile on a warm file cache with other lanes building,
//! over the `common`, `x64a`, `update` and `update2` archives of one Legacy and one Enhanced retail
//! install: the entry table read in 1.5 to 5.7 ms, where reading the file and hashing it a byte at a
//! time took 21 ms to 4.3 s (3.1 s for Legacy's 2.0 GB `update.rpf`, 4.3 s for Enhanced's 3.1 GB).
//! A table holds 624 to 2,211 files, so a record is 70 to 267 KB of JSON. Each archive's record
//! calls the next one in that list stale, and its own touched copy fresh.
//!
//! ⚠ **What the table cannot see**: a patch that changes an entry's contents and no size, flag, name
//! or count. `a_patch_that_moves_nothing_in_the_table_is_not_seen` pins that, so the limit is a test
//! rather than a surprise. Only one patch level of each edition was available to measure against, so
//! whether the title's own updates ever do this is unobserved; the synthetic archives below cover
//! each structural way a patch changes a table.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use or_io::container::rpf::crypto::{NoOpCryptoProvider, RpfCryptoProvider};
use or_io::container::rpf::RpfBuilder;
use or_title::vfs::{
    record_file, refresh_plan, stale_copies, BaseRecord, Change, Discard, StaleCopy, Staleness,
};

const RSC7: u32 = u32::from_le_bytes(*b"RSC7");

fn crypto() -> Arc<dyn RpfCryptoProvider> {
    Arc::new(NoOpCryptoProvider::new())
}

fn text(seed: &str, len: usize) -> Vec<u8> {
    seed.as_bytes().iter().copied().cycle().take(len).collect()
}

/// A resource whose paging words, and so its table row, are the same whatever `marker` is.
fn model(marker: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for word in [RSC7, 0x9A, 0x9000_0042, 0xA000_0007] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend(text("mesh ", 4000));
    out[100] = marker;
    out
}

fn archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut builder = RpfBuilder::default();
    for (path, bytes) in entries {
        builder
            .insert_with(path, bytes.clone(), false)
            .expect("an entry");
    }
    builder.to_bytes().expect("save")
}

/// What the install ships, before the title patches it.
fn shipped() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("data/version.txt", b"1.0.100".to_vec()),
        ("data/handling.meta", text("handling ", 2000)),
        ("data/tuning.dat", text("tuning ", 600)),
        ("models/car.ydr", model(1)),
    ]
}

struct Scene {
    root: PathBuf,
    records: PathBuf,
    base: PathBuf,
    copy: PathBuf,
}

impl Scene {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("an_overlay_copy_the_title_has_outgrown")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        let base = root.join("update").join("update.rpf");
        std::fs::create_dir_all(base.parent().expect("a parent")).expect("install folders");
        std::fs::write(&base, archive(&shipped())).expect("the shipped archive");
        Self {
            records: root.join("records"),
            copy: root.join("mods").join("update").join("update.rpf"),
            base,
            root,
        }
    }

    /// The copy as the app makes it: the install's bytes, and a record of what they were.
    fn copy_it(&self) {
        let record = BaseRecord::of(&self.base, &crypto()).expect("a record of the original");
        std::fs::create_dir_all(self.copy.parent().expect("a parent")).expect("overlay folders");
        std::fs::copy(&self.base, &self.copy).expect("the copy");
        record
            .write(&record_file(&self.records, &self.copy))
            .expect("the record is kept");
    }

    fn record(&self) -> BaseRecord {
        BaseRecord::read(&record_file(&self.records, &self.copy)).expect("a record was kept")
    }

    /// The title replaces the archive.
    fn patch(&self, entries: &[(&str, Vec<u8>)]) {
        std::fs::write(&self.base, archive(entries)).expect("the patched archive");
    }

    /// The user saves their edits into the copy, which rebuilds it whole.
    fn edit(&self, entries: &[(&str, Vec<u8>)]) {
        std::fs::write(&self.copy, archive(entries)).expect("the edited copy");
    }

    fn stale(&self) -> Vec<StaleCopy> {
        stale_copies(&self.root, &self.records, &crypto())
    }

    fn plan(&self) -> or_title::vfs::RefreshPlan {
        refresh_plan(&self.copy, &self.base, &self.record(), &crypto(), &|| false).expect("a plan")
    }
}

/// What the title's update does to the shipped archive.
fn patched() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("data/version.txt", b"1.0.1000".to_vec()),
        ("data/handling.meta", text("handling ", 2000)),
        ("data/tuning.dat", text("tuning ", 600)),
        ("data/new_in_update.dat", text("fresh ", 300)),
        ("models/car.ydr", model(1)),
    ]
}

fn set_modified(path: &Path, to: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open to touch")
        .set_modified(to)
        .expect("set the time");
}

#[test]
fn a_copy_of_an_archive_nobody_touched_is_fresh() {
    let scene = Scene::new("fresh");
    scene.copy_it();

    assert_eq!(
        scene.record().verdict(&scene.base, &crypto()),
        Staleness::Fresh
    );
    assert!(scene.stale().is_empty());
}

#[test]
fn a_patched_base_makes_its_copy_stale_and_names_it() {
    let scene = Scene::new("patched");
    scene.copy_it();
    scene.patch(&patched());

    assert_eq!(
        scene.record().verdict(&scene.base, &crypto()),
        Staleness::Stale
    );
    assert_eq!(
        scene.stale(),
        vec![StaleCopy {
            copy: scene.copy.clone(),
            base: scene.base.clone(),
        }],
        "the archive the update invalidated is named, as the install holds it and as the overlay does"
    );
}

/// ⚠ A staleness warning that fires on a touch is one a user learns to dismiss.
#[test]
fn a_base_that_was_only_touched_is_not_stale() {
    let scene = Scene::new("touched");
    scene.copy_it();
    std::fs::write(&scene.base, archive(&shipped())).expect("the same archive, written again");
    set_modified(&scene.base, SystemTime::now() + Duration::from_secs(3600));

    let record = scene.record();
    assert_ne!(
        record.modified,
        BaseRecord::of(&scene.base, &crypto())
            .expect("a record now")
            .modified,
        "the touch has to show to a stat, or this test is not about one"
    );
    assert_eq!(record.verdict(&scene.base, &crypto()), Staleness::Fresh);
    assert!(scene.stale().is_empty());
}

#[test]
fn a_copy_nothing_recorded_is_never_reported() {
    let scene = Scene::new("unrecorded");
    std::fs::create_dir_all(scene.copy.parent().expect("a parent")).expect("overlay folders");
    std::fs::copy(&scene.base, &scene.copy).expect("a copy made some other way");
    scene.patch(&patched());

    assert!(
        scene.stale().is_empty(),
        "a copy OpenRage did not make has no record to be stale against, and a guess would be crying wolf"
    );
}

#[test]
fn a_base_the_install_no_longer_holds_is_not_reported() {
    let scene = Scene::new("gone");
    scene.copy_it();
    std::fs::remove_file(&scene.base).expect("the title dropped the archive");

    assert_eq!(
        scene.record().verdict(&scene.base, &crypto()),
        Staleness::Unchecked
    );
    assert!(scene.stale().is_empty());
}

#[test]
fn a_patch_that_moves_nothing_in_the_table_is_not_seen() {
    let scene = Scene::new("blind");
    scene.copy_it();
    let mut same_shape = shipped();
    same_shape[3].1 = model(2);
    scene.patch(&same_shape);
    set_modified(&scene.base, SystemTime::now() + Duration::from_secs(3600));

    assert_eq!(
        scene.record().verdict(&scene.base, &crypto()),
        Staleness::Fresh,
        "the limit the module states: one byte changed inside a resource whose flags are the same"
    );
}

#[test]
fn a_record_is_read_back_whole_and_a_damaged_one_is_no_record() {
    let scene = Scene::new("record");
    scene.copy_it();
    let record = scene.record();
    let file = record_file(&scene.records, &scene.copy);

    assert_eq!(
        record,
        BaseRecord::of(&scene.base, &crypto()).expect("again")
    );
    std::fs::write(&file, b"{\"size\": 12, ").expect("a torn write");
    assert_eq!(BaseRecord::read(&file), None);
    assert_eq!(BaseRecord::read(&file.with_extension("missing")), None);
}

#[test]
fn a_refresh_lists_what_it_would_discard() {
    let scene = Scene::new("plan");
    scene.copy_it();

    // The user changes a text file and a model without changing either's size or flags, adds a file
    // and drops one; the title patches the same archive.
    let mut edited = shipped();
    edited[1].1 = text("hand-tuned ", 2000);
    edited[3].1 = model(7);
    edited.remove(2);
    edited.push(("data/mine.txt", b"mine".to_vec()));
    scene.edit(&edited);
    scene.patch(&patched());

    let plan = scene.plan();
    assert_eq!(
        plan.yours,
        vec![
            Discard::new("data/handling.meta", Change::Edited),
            Discard::new("data/mine.txt", Change::Added),
            Discard::new("data/tuning.dat", Change::Removed),
            Discard::new("models/car.ydr", Change::Edited),
        ],
        "every change of the user's, same-size ones included, and nothing the update did"
    );
    assert_eq!(
        plan.by_update,
        vec!["data/version.txt".to_string()],
        "the one entry the update changed that the copy still holds the old version of"
    );
}

#[test]
fn a_copy_nobody_edited_has_nothing_of_the_users_to_discard() {
    let scene = Scene::new("plan_untouched");
    scene.copy_it();
    scene.patch(&patched());

    let plan = scene.plan();
    assert_eq!(plan.yours, Vec::<Discard>::new());
    assert_eq!(plan.by_update, vec!["data/version.txt".to_string()]);
}

#[test]
fn a_plan_that_is_stopped_says_so_and_decides_nothing() {
    let scene = Scene::new("plan_stopped");
    scene.copy_it();
    scene.patch(&patched());

    let stopped = refresh_plan(
        &scene.copy,
        &scene.base,
        &scene.record(),
        &crypto(),
        &|| true,
    );
    assert!(stopped.is_err());
}

/// A pass over the whole file, as a hash of it would make: its read, plus a byte-at-a-time FNV-1a.
fn read_and_hash(path: &Path) -> (u64, Duration) {
    use std::io::Read;
    let started = std::time::Instant::now();
    let mut file = std::fs::File::open(path).expect("open");
    let mut chunk = vec![0u8; 1 << 20];
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    loop {
        let read = file.read(&mut chunk).expect("read");
        if read == 0 {
            return (hash, started.elapsed());
        }
        for byte in &chunk[..read] {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// What each candidate costs on a retail install, and that the two that decide tell real archives apart.
#[test]
fn a_retail_archive_is_told_from_a_touched_one_and_from_another_by_its_table() {
    use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
    use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            "a_retail_archive_is_told_from_a_touched_one_and_from_another_by_its_table",
            format_args!("no install; set {GAME_ROOT_ENV}"),
        );
        return;
    }
    for root in roots {
        let edition = or_title::Game::detect_from_root(&root).expect("a game's install");
        let exe = edition
            .find_executable(&root)
            .expect("an install has its executable");
        let keys = extract_keys(&exe, &|_| {}).expect("keys from the install's executable");
        let crypto: Arc<dyn RpfCryptoProvider> =
            Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));

        eprintln!("\n=== {} at {} ===", edition.label(), root.display());
        let mut earlier: Option<(PathBuf, BaseRecord)> = None;
        for name in [
            "common.rpf",
            "x64a.rpf",
            "update/update.rpf",
            "update/update2.rpf",
        ] {
            let base = root.join(name);
            if !base.is_file() {
                continue;
            }
            let started = std::time::Instant::now();
            let record = BaseRecord::of(&base, &crypto).expect("a record of a retail archive");
            let table = started.elapsed();
            let (_, whole) = read_and_hash(&base);
            eprintln!(
                "{name}: {} bytes, {} entries; entry table {table:?}; whole-file read and hash {whole:?}; record {} bytes",
                record.size,
                record.entries.len(),
                serde_json::to_vec(&record).expect("json").len()
            );

            assert_eq!(record.verdict(&base, &crypto), Staleness::Fresh);
            let touched = BaseRecord {
                modified: record.modified + 1,
                ..record.clone()
            };
            assert_eq!(
                touched.verdict(&base, &crypto),
                Staleness::Fresh,
                "{name}: a touched archive is the one it was"
            );
            if let Some((other, other_record)) = &earlier {
                assert_eq!(
                    other_record.verdict(&base, &crypto),
                    Staleness::Stale,
                    "{name} is not {}: the table tells two real archives apart",
                    other.display()
                );
            }
            earlier = Some((base, record));
        }
    }
}
