//! The index through the kept file every title shares, over a disk made up here: an archive is a
//! text file whose lines a scan turns into rows, and every read of the disk is counted.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use bytemuck::Zeroable;
use or_io::gta6::map::stream::grass::slod::SlodClaim;
use or_io::gta6::world_state::Place;
use or_title::gta6_mount::{Registry, SCRIPT_MANAGED};
use or_world::kept::{Kept, KeptError, Verdict};
use or_world::model::{Gate, MapId};

use super::super::install::RULES;
use super::super::lights_index::LightRow;
use super::super::mount::tests::pack;
use super::super::mount::Mount;
use super::super::rows::{put_pod, Rows};
use super::super::scan::Shard;
use super::super::states::tests::{at_start, door};
use super::super::states::Placed;
use super::super::{ArchetypeRow, MapRow, ALL_HOURS};
use super::*;

/// The word of the group the door of the made-up selector opens by day.
const OPEN: u64 = 0xA1;

/// A disk made up: what a scan gives is read from the archive's own text, a line a row.
#[derive(Default)]
struct Made {
    /// The file name of each archive scanned, in order.
    scans: Mutex<Vec<String>>,
    mounts: AtomicUsize,
    selectors: AtomicUsize,
    /// The archives that do not open, by file name.
    shut: Mutex<HashSet<&'static str>>,
    /// Whether the archive of the world states does not open.
    states_shut: AtomicBool,
    /// The folders the level names its archives under; with none, every archive holds the world.
    level: Mutex<Vec<&'static str>>,
}

impl Made {
    /// The archives scanned since the last call.
    fn scanned(&self) -> Vec<String> {
        std::mem::take(&mut *self.scans.lock().expect("the scans"))
    }

    /// How many times the mount and the selectors were read.
    fn derived(&self) -> [usize; 2] {
        [
            self.mounts.load(Ordering::Relaxed),
            self.selectors.load(Ordering::Relaxed),
        ]
    }
}

fn archetype(word: u64) -> ArchetypeRow {
    ArchetypeRow {
        word,
        model: word + 1,
        dictionary: 0,
        name: word as u32,
        lod_dist: 100.0,
        centre: [0.0, 0.0, 1.0],
        radius: 2.0,
        flags: 0,
        hours: ALL_HOURS,
        load_flags: 0,
        pad: 0,
    }
}

/// What a line `map NAME [GROUP [PARENT]]` of an archive gives: a map, and an archetype with its model.
fn take_map(shard: &mut Shard, numbers: &[u32]) {
    let (name, group) = (numbers[0], numbers.get(1).copied().unwrap_or(0));
    let row = MapRow {
        name,
        block: 7,
        flags: u32::from(group != 0) * SCRIPT_MANAGED,
        content_flags: 1,
        group,
        offset: u64::from(name) * 16,
        boxes: [
            -10.0, -10.0, -10.0, 10.0, 10.0, 10.0, -5.0, -5.0, -5.0, 5.0, 5.0, 5.0,
        ],
        ..MapRow::zeroed()
    };
    (shard.maps).push((row, numbers.get(2).copied().into_iter().collect(), true));
    if group != 0 {
        // The group of the made-up selector's door is 100, by the word it switches.
        let word = if group == 100 { OPEN } else { u64::from(group) };
        shard.words.push((group, word));
    }
    let word = u64::from(name) * 1000;
    shard.archetypes.push(archetype(word));
    shard.models.push((word + 1, u64::from(name) * 16 + 8));
}

/// What a line `rich` gives: a row of every other table, and an archetype stated twice.
fn take_rich(shard: &mut Shard) {
    shard.archetypes.extend([
        archetype(100),
        ArchetypeRow {
            model: 0,
            dictionary: 700,
            name: 3,
            ..archetype(200)
        },
        ArchetypeRow {
            lod_dist: 55.0,
            hours: 0x0000_00F0,
            load_flags: 0x0001_0081,
            ..archetype(100)
        },
    ]);
    shard.interiors.extend([(600, 0x650), (600, 0x600)]);
    shard.keys.push((0x700, vec![3]));
    shard.types.extend([0x650, 0x600]);
    shard.archives.push(0x500);
    shard.unread = 2;
    shard.slod.push(SlodClaim {
        ids: vec![1],
        words: vec![300],
    });
    shard.lights.push(LightRow {
        archetype: 100,
        intensity: 4.0,
        ..LightRow::zeroed()
    });
}

impl Disk for Made {
    fn mount(
        &self,
        every: &[PathBuf],
        _keys: Option<&GtaViKeys>,
        _stopped: &dyn Fn() -> bool,
    ) -> Option<Mount> {
        self.mounts.fetch_add(1, Ordering::Relaxed);
        let level = self.level.lock().expect("the level");
        // Sixteen archives under a folder are what names the folder's archive the level's.
        let files: Vec<String> = (level.iter())
            .flat_map(|folder| (0..16).map(move |k| format!("platform:/{folder}/part{k}.rpf")))
            .collect();
        let files: Vec<&str> = files.iter().map(String::as_str).collect();
        let registry = match (files.is_empty(), every.first()) {
            (false, Some(first)) => Registry::of(vec![(first.clone(), pack("core", &files, true))]),
            _ => Registry::default(),
        };
        Some(Mount {
            registry,
            tables: Vec::new(),
        })
    }

    fn scan(
        &self,
        path: &Path,
        _cancel: &Cancel,
        reached: &(dyn Fn(u64) + Sync),
    ) -> io::Result<Option<Shard>> {
        let name = path.file_name().expect("a file").to_string_lossy();
        self.scans.lock().expect("the scans").push(name.to_string());
        if self.shut.lock().expect("the shut").contains(&*name) {
            return Err(io::Error::other("the archive is shut"));
        }
        let text = std::fs::read_to_string(path)?;
        let mut shard = Shard::default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let kind = words.next();
            let numbers: Vec<u32> = words.map(|n| n.parse().expect("a number")).collect();
            match kind {
                Some("map") => take_map(&mut shard, &numbers),
                Some("rich") => take_rich(&mut shard),
                _ => {}
            }
        }
        reached(text.len() as u64);
        Ok(Some(shard))
    }

    fn selectors(
        &self,
        _mounted: &Mount,
        _every: &[PathBuf],
        _cancel: &Cancel,
    ) -> Option<(Vec<Placed>, Vec<Place>, bool)> {
        self.selectors.fetch_add(1, Ordering::Relaxed);
        let shut = self.states_shut.load(Ordering::Relaxed);
        if shut {
            return Some((Vec::new(), Vec::new(), false));
        }
        let place = Place {
            name: 9,
            label: 8,
            kind: 1,
            outline: vec![[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]],
        };
        Some((vec![at_start(1, door())], vec![place], true))
    }
}

/// An install of two archives and a configuration folder, on a disk made up.
struct Scene {
    folder: tempfile::TempDir,
    disk: Made,
}

impl Scene {
    fn new() -> Self {
        let scene = Self {
            folder: tempfile::tempdir().expect("a scratch folder"),
            disk: Made::default(),
        };
        std::fs::create_dir_all(scene.root()).expect("the install");
        scene.write("east.rpf", "map 11\nmap 22 100 11\nrich\n");
        scene.write("west.rpf", "map 33\n");
        scene
    }

    fn root(&self) -> PathBuf {
        self.folder.path().join("install")
    }

    fn config(&self) -> PathBuf {
        self.folder.path().join("config")
    }

    fn write(&self, archive: &str, text: &str) {
        std::fs::write(self.root().join(archive), text).expect("the archive");
    }

    fn file(&self) -> PathBuf {
        kept_at(&self.config(), &self.root())
    }

    fn opened(&self, afresh: bool) -> Opened {
        let (cancel, progress) = (Cancel::new(), Progress::new());
        let (root, config) = (self.root(), self.config());
        let job = (&cancel, &progress);
        open_through(&self.disk, &root, Some(&config), None, afresh, job)
            .expect("the index opens")
            .expect("no open is cancelled")
    }

    fn open(&self) -> Opened {
        self.opened(false)
    }
}

/// The archives and every table of an index, to compare two indexes by.
type Tables = (Vec<PathBuf>, Vec<(&'static str, Vec<u8>)>);

fn tables(index: &WorldIndex) -> Tables {
    let tables = index.tables().into_iter();
    let tables = tables.map(|(name, _, bytes)| (name, bytes.to_vec()));
    (index.archives.clone(), tables.collect())
}

fn names(index: &WorldIndex) -> Vec<u32> {
    index.maps.iter().map(|map| map.name).collect()
}

fn report(opened: &Opened) -> &Report {
    opened.kept.as_ref().expect("an index kept")
}

const NONE: [&str; 0] = [];

#[test]
fn a_second_start_opens_no_archive() {
    let scene = Scene::new();
    let first = scene.open();
    assert!(matches!(report(&first).verdict, Verdict::Absent));
    assert_eq!(report(&first).read, [0, 1]);
    assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);
    assert_eq!(scene.disk.derived(), [1, 1]);
    assert_eq!(names(&first.index), [11, 22, 33]);

    let second = scene.open();
    assert!(
        matches!(report(&second).verdict, Verdict::Current),
        "{:?}",
        report(&second)
    );
    assert_eq!(scene.disk.scanned(), NONE);
    assert_eq!(
        scene.disk.derived(),
        [1, 1],
        "neither the mount nor the selectors were read again"
    );
    assert_eq!(tables(&second.index), tables(&first.index));
}

#[test]
fn a_kept_index_reads_back_as_it_was_until_an_archive_changes() {
    let scene = Scene::new();
    let built = scene.open();
    let read = scene.open();
    assert!(matches!(report(&read).verdict, Verdict::Current));
    assert_eq!(tables(&read.index), tables(&built.index));
    for index in [&built.index, &read.index] {
        let archives = ["east.rpf", "west.rpf"].map(|name| scene.root().join(name));
        assert_eq!(index.archives, archives);
        assert_eq!(index.map_named(22), Some(1));
        assert_eq!(index.parents_of(&index.maps[1]), [11]);
        assert_eq!(index.maps[2].archive, 1);
        // Of two bodies that state one archetype the later stands, with its hours and its load flags.
        let later = index
            .archetype(100)
            .map(|a| (a.lod_dist, a.hours, a.load_flags));
        assert_eq!(later, Some((55.0, 0xF0, 0x0001_0081)));
        assert_eq!(index.interior(600).map(|b| b.offset), Some(0x600));
        assert_eq!(index.interior(601), None);
        let dictionary = index.archetype(200).and_then(|a| index.model(a));
        assert_eq!(
            dictionary.map(|m| (m.offset, m.entry)),
            Some((0x700, Some(3)))
        );
        assert_eq!(index.slod, [0, 300]);
        assert_eq!(index.archetype_lights(100).len(), 1);
        assert_eq!(index.places_at([1.0, 1.0]).len(), 1);
        assert_eq!((index.report.maps, index.report.unread), (3, 2));
        // The groups are worked out again from the maps, with their words and hours at rest.
        assert_eq!(index.groups().len(), 1);
        assert_eq!(index.groups()[0].word, OPEN);
        assert_eq!(index.default_on_in(13.0, 1, true), [100]);
        assert_eq!(index.default_on_in(23.0, 1, true), Vec::<u32>::new());
    }

    // An archive of another size is another archive: it is read again, and no other is.
    scene.disk.scanned();
    scene.write("west.rpf", "map 33\nmap 44\n");
    let changed = scene.open();
    assert!(matches!(report(&changed).verdict, Verdict::Changed));
    assert_eq!(scene.disk.scanned(), ["west.rpf"]);
    assert_eq!(names(&changed.index), [11, 22, 33, 44]);
}

#[test]
fn a_changed_archive_alone_is_read_again() {
    let scene = Scene::new();
    scene.open();
    scene.disk.scanned();
    scene.write("east.rpf", "map 11\nmap 55\n");
    let changed = scene.open();
    assert!(matches!(report(&changed).verdict, Verdict::Changed));
    assert_eq!(report(&changed).read, [0]);
    assert_eq!(scene.disk.scanned(), ["east.rpf"]);
    assert_eq!(
        names(&changed.index),
        [11, 55, 33],
        "the rows of the archive that stands are the kept ones"
    );
    // What is derived across archives is made again, from a mount read once this open.
    assert_eq!(scene.disk.derived(), [2, 2]);
    let after = scene.open();
    assert!(matches!(report(&after).verdict, Verdict::Current));
    assert_eq!(scene.disk.scanned(), NONE);
    assert_eq!(tables(&after.index), tables(&changed.index));
}

/// A request that closes the made-up door from 12 to 19, which the data alone leaves open.
const FRESH_GAME: &str = r#"{"flags_clear": true, "requests": [{"selector": "1", "config": "0x14", "priority": 1000, "hours": [12, 19]}]}"#;

#[test]
fn a_changed_fresh_game_derives_again_and_scans_no_archive() {
    let scene = Scene::new();
    let before = scene.open();
    assert_eq!(before.index.default_on_in(13.0, 1, false), [100]);
    scene.disk.scanned();

    let fresh = fresh_at(&scene.config());
    assert_eq!(
        fresh.parent(),
        scene.file().parent(),
        "beside the kept index"
    );
    std::fs::write(&fresh, FRESH_GAME).expect("the fresh game");
    let with = scene.open();
    assert!(matches!(report(&with).verdict, Verdict::Changed));
    assert!(report(&with).derived && report(&with).read.is_empty());
    assert_eq!(scene.disk.scanned(), NONE);
    assert_eq!(with.index.default_on_in(13.0, 1, true), [100]);
    assert_eq!(
        with.index.default_on_in(13.0, 1, false),
        Vec::<u32>::new(),
        "the request of the file is taken"
    );
    assert!(matches!(report(&scene.open()).verdict, Verdict::Current));

    // An edit of it derives again, and so does its going.
    std::fs::write(&fresh, "{}").expect("the fresh game, edited");
    let edited = scene.open();
    assert!(matches!(report(&edited).verdict, Verdict::Changed));
    assert_eq!(edited.index.default_on_in(13.0, 1, false), [100]);
    std::fs::remove_file(&fresh).expect("gone");
    assert!(matches!(report(&scene.open()).verdict, Verdict::Changed));
    assert!(matches!(report(&scene.open()).verdict, Verdict::Current));
    assert_eq!(scene.disk.scanned(), NONE);
}

#[test]
fn an_old_kept_file_is_deleted_and_its_fresh_game_moved() {
    let scene = Scene::new();
    let old = old_kept_at(&scene.config(), &scene.root());
    let folder = old.parent().expect("the old folder");
    std::fs::create_dir_all(folder).expect("the old folder");
    std::fs::write(&old, b"ORWIDX\0\0 an index of an earlier build").expect("the old index");
    std::fs::write(folder.join(FRESH), FRESH_GAME).expect("the old fresh game");
    // What the Map screen keeps in the same folder, and the index of another install.
    let (theirs, other) = (folder.join("0123.world"), folder.join("0123.index"));
    std::fs::write(&theirs, b"theirs").expect("another file");
    std::fs::write(&other, b"another install").expect("another index");

    let opened = scene.open();
    assert!(!old.exists(), "the old kept index is deleted");
    assert!(
        !folder.join(FRESH).exists(),
        "the fresh game left its old place"
    );
    let moved = std::fs::read_to_string(fresh_at(&scene.config())).expect("the moved file");
    assert_eq!(moved, FRESH_GAME);
    assert!(theirs.exists() && other.exists());
    assert!(matches!(report(&opened).verdict, Verdict::Absent));
    assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);
    assert_eq!(
        opened.index.default_on_in(13.0, 1, false),
        Vec::<u32>::new(),
        "the index was built with the file moved from its old place"
    );
    let again = scene.open();
    assert!(matches!(report(&again).verdict, Verdict::Current));
    assert_eq!(tables(&again.index), tables(&opened.index));
}

#[test]
fn a_fresh_game_already_at_its_new_place_stands_and_the_old_one_is_left() {
    let scene = Scene::new();
    let old = old_kept_at(&scene.config(), &scene.root()).with_file_name(FRESH);
    std::fs::create_dir_all(old.parent().expect("a folder")).expect("the old folder");
    std::fs::write(&old, "{}").expect("the old fresh game");
    let new = fresh_at(&scene.config());
    std::fs::create_dir_all(new.parent().expect("a folder")).expect("the new folder");
    std::fs::write(&new, FRESH_GAME).expect("the new fresh game");
    let opened = scene.open();
    assert_eq!(
        opened.index.default_on_in(13.0, 1, false),
        Vec::<u32>::new()
    );
    assert_eq!(std::fs::read_to_string(&old).expect("left"), "{}");
}

/// The kept file of `scene`, rewritten as `change` leaves it.
fn rewrite(scene: &Scene, change: impl FnOnce(&mut Kept)) {
    let mut kept = (Kept::open(&scene.file(), RULES).expect("it reads")).expect("it is there");
    change(&mut kept);
    kept.keep(&scene.file()).expect("it is written");
}

#[test]
fn a_kept_file_of_other_rules_or_another_title_is_refused_and_built_again() {
    let scene = Scene::new();
    let first = scene.open();
    scene.disk.scanned();

    rewrite(&scene, |kept| kept.head.rules = RULES + 1);
    let rules = scene.open();
    match &report(&rules).verdict {
        Verdict::Refused(KeptError::Rules { found, reads }) => {
            assert_eq!((*found, *reads), (RULES + 1, RULES))
        }
        other => panic!("a file of other rules gave {other:?}"),
    }
    assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);
    assert_eq!(tables(&rules.index), tables(&first.index));
    assert!(matches!(report(&scene.open()).verdict, Verdict::Current));

    rewrite(&scene, |kept| kept.head.title = "gta5".into());
    let title = scene.open();
    assert!(matches!(report(&title).verdict, Verdict::Foreign));
    assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);
    assert_eq!(tables(&title.index), tables(&first.index));
    let kept = (Kept::open(&scene.file(), RULES).expect("it reads")).expect("it is there");
    assert_eq!((kept.head.title.as_str(), kept.head.rules), (TITLE, RULES));
    assert_eq!(kept.head.world, world(&scene.root()).identity());
}

#[test]
fn a_truncated_kept_file_is_refused_and_built_again() {
    let scene = Scene::new();
    let first = scene.open();
    scene.disk.scanned();
    let whole = std::fs::read(scene.file()).expect("the kept bytes");
    std::fs::write(scene.file(), &whole[..whole.len() - 9]).expect("cut");
    let cut = scene.open();
    assert!(
        matches!(report(&cut).verdict, Verdict::Refused(KeptError::Damaged)),
        "{:?}",
        report(&cut)
    );
    assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);
    assert_eq!(tables(&cut.index), tables(&first.index));
    assert_eq!(std::fs::read(scene.file()).expect("kept again"), whole);
}

#[test]
fn an_archive_that_did_not_open_is_read_again_at_the_next_open() {
    let scene = Scene::new();
    scene.disk.shut.lock().expect("the shut").insert("west.rpf");
    let without = scene.open();
    assert_eq!(names(&without.index), [11, 22]);
    assert_eq!(without.index.archives.len(), 2, "it is still the world's");
    assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);

    // Its stamp stood while its rows were never read: it alone is asked again.
    scene.disk.shut.lock().expect("the shut").clear();
    let with = scene.open();
    assert_eq!(report(&with).read, [1]);
    assert_eq!(scene.disk.scanned(), ["west.rpf"]);
    assert_eq!(names(&with.index), [11, 22, 33]);
    assert!(matches!(report(&scene.open()).verdict, Verdict::Current));
    assert_eq!(scene.disk.scanned(), NONE);
}

#[test]
fn an_archive_the_mount_newly_names_is_read_though_it_did_not_change() {
    let scene = Scene::new();
    scene.write("common.rpf", "the documents\n");
    *scene.disk.level.lock().expect("the level") = vec!["east"];
    let first = scene.open();
    assert_eq!(first.index.archives, [scene.root().join("east.rpf")]);
    assert_eq!(scene.disk.scanned(), ["east.rpf"], "the level's alone");
    assert_eq!(names(&first.index), [11, 22]);

    // The documents change and name another archive, which itself stands as it was.
    scene.write("common.rpf", "the documents, which name another folder\n");
    *scene.disk.level.lock().expect("the level") = vec!["east", "west"];
    let second = scene.open();
    assert_eq!(report(&second).read, [0], "the documents' archive alone");
    assert_eq!(scene.disk.scanned(), ["west.rpf"]);
    assert_eq!(names(&second.index), [11, 22, 33]);
    assert_eq!(second.index.maps[2].archive, 1);
    let third = scene.open();
    assert!(matches!(report(&third).verdict, Verdict::Current));
    assert_eq!(scene.disk.scanned(), NONE);
    assert_eq!(tables(&third.index), tables(&second.index));

    // Once it changes, its rows lie in its own section like any other's.
    scene.write("west.rpf", "map 33\nmap 44\n");
    let fourth = scene.open();
    assert_eq!(scene.disk.scanned(), ["west.rpf"]);
    assert_eq!(names(&fourth.index), [11, 22, 33, 44]);
}

#[test]
fn an_index_built_again_reads_every_archive() {
    let scene = Scene::new();
    let first = scene.open();
    scene.disk.scanned();
    let again = scene.opened(true);
    assert!(matches!(report(&again).verdict, Verdict::Absent));
    assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);
    assert_eq!(tables(&again.index), tables(&first.index));
}

#[test]
fn an_index_of_no_map_is_not_kept() {
    let scene = Scene::new();
    scene.write("east.rpf", "");
    scene.write("west.rpf", "");
    let opened = scene.open();
    assert_eq!(opened.index.report.maps, 0);
    assert!(!scene.file().exists());
}

#[test]
fn an_index_built_with_nothing_kept_is_the_index_the_kept_file_links() {
    let scene = Scene::new();
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let job = (&cancel, &progress);
    let built = build_through(&scene.disk, &scene.root(), None, None, job)
        .expect("it builds")
        .expect("no build is cancelled");
    assert!(!scene.config().exists(), "nothing is written");
    assert_eq!(tables(&built), tables(&scene.open().index));
    assert_eq!(tables(&built), tables(&scene.open().index));
    // The progress of a build is counted against the bytes of the world's archives.
    let sizes = ["east.rpf", "west.rpf"]
        .map(|name| std::fs::metadata(scene.root().join(name)).map(|meta| meta.len()));
    let total: u64 = sizes.into_iter().map(|size| size.expect("a size")).sum();
    let reached = progress.snapshot();
    assert_eq!((reached.attempted, reached.total), (total, total));
}

/// A disk that reads nothing once the job is cancelled.
struct Stopped;

impl Disk for Stopped {
    fn mount(
        &self,
        _: &[PathBuf],
        _: Option<&GtaViKeys>,
        stopped: &dyn Fn() -> bool,
    ) -> Option<Mount> {
        assert!(stopped(), "the open was cancelled before it began");
        None
    }

    fn scan(&self, _: &Path, _: &Cancel, _: &(dyn Fn(u64) + Sync)) -> io::Result<Option<Shard>> {
        Ok(None)
    }

    fn selectors(
        &self,
        _: &Mount,
        _: &[PathBuf],
        _: &Cancel,
    ) -> Option<(Vec<Placed>, Vec<Place>, bool)> {
        None
    }
}

#[test]
fn a_cancelled_open_gives_nothing_and_keeps_nothing() {
    let scene = Scene::new();
    let (cancel, progress) = (Cancel::new(), Progress::new());
    cancel.set();
    let (root, config) = (scene.root(), scene.config());
    let job = (&cancel, &progress);
    let opened = open_through(&Stopped, &root, Some(&config), None, false, job);
    assert!(matches!(opened, Ok(None)));
    assert!(!scene.file().exists());
}

#[test]
fn the_linked_index_names_each_map_once_with_its_parents_its_gate_and_its_archive() {
    let scene = Scene::new();
    // A name borne twice: the index answers for the last, and that one alone is a row.
    scene.write("west.rpf", "map 33\nmap 11\n");
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let root = scene.root();
    let install = Install::new(&root, None, None, (&cancel, &progress), &scene.disk);
    let mut archives = Vec::new();
    for archive in 0..2 {
        let mut section = Section::new();
        install.rows(archive, &mut section).expect("rows");
        archives.push(section);
    }
    let mut whole = Section::new();
    install.whole(&archives, &mut whole).expect("derived");
    let index = install.link(&archives, &whole).expect("it links");
    let linked = index.title::<Linked>().expect("the tables of the link");
    assert_eq!(names(&linked.world), [11, 22, 33, 11]);
    let rows: Vec<u64> = index.maps().iter().map(|row| row.name).collect();
    assert_eq!(rows, [22, 33, 11]);
    let places: Vec<_> = (0..4).map(|id| linked.place(MapId(id))).collect();
    assert_eq!(places, [Some(1), Some(2), Some(3), None]);
    let row = &index.maps()[0];
    assert_eq!(row.parents, [index.named(11).expect("the parent")]);
    assert_eq!((row.archive, index.maps()[2].archive), (0, 1));
    assert_eq!(row.boxes, Some(linked.world.maps[1].boxes()));
    // No pack registers a map of this disk: each is held, with no switch to let it in.
    let held = Gate::Held { switch: None };
    assert!(index.maps().iter().all(|row| row.gate == held));
}

#[test]
fn the_rows_of_an_archive_read_back_as_they_were_written_and_a_section_out_of_step_is_refused() {
    let mut shard = Shard::default();
    take_map(&mut shard, &[22, 100, 11]);
    take_map(&mut shard, &[90]);
    take_rich(&mut shard);
    let rows = Rows::of(shard);
    assert_eq!(rows.maps[0].parents, [11]);
    assert_eq!(rows.maps[0].container, None, "before the nested archive");
    assert_eq!(rows.maps[1].container, Some(0x500));
    assert_eq!(rows.types, [0x500, 0x500], "each by the archive it lies in");
    let mut section = Section::new();
    rows.put(&mut section);
    assert_eq!(Rows::read(&section).expect("it reads"), Some(rows));

    let mut none = Section::new();
    Rows::put_none(&mut none);
    assert_eq!(Rows::read(&none).expect("it reads"), None);

    // A byte more and a kind no build writes are each refused.
    let mut longer = section.clone();
    longer.put_u8(0);
    assert!(Rows::read(&longer).is_err());
    let mut other = Section::new();
    other.put_u8(7);
    assert!(Rows::read(&other).is_err());
    assert!(Rows::read(&Section::new()).is_err());
}

#[test]
fn an_archive_the_mount_newly_names_that_did_not_open_is_read_again_at_the_next_open() {
    let scene = Scene::new();
    scene.write("common.rpf", "the documents\n");
    *scene.disk.level.lock().expect("the level") = vec!["east"];
    scene.open();
    scene.write("common.rpf", "the documents, which name another folder\n");
    *scene.disk.level.lock().expect("the level") = vec!["east", "west"];
    scene.disk.shut.lock().expect("the shut").insert("west.rpf");
    scene.disk.scanned();
    let without = scene.open();
    assert_eq!(scene.disk.scanned(), ["west.rpf"]);
    assert_eq!(names(&without.index), [11, 22]);

    // Its stamp stands and its own section holds no rows: the derived section alone is made again.
    scene.disk.shut.lock().expect("the shut").clear();
    let with = scene.open();
    assert!(report(&with).derived && report(&with).read.is_empty());
    assert_eq!(scene.disk.scanned(), ["west.rpf"]);
    assert_eq!(names(&with.index), [11, 22, 33]);
    assert!(matches!(report(&scene.open()).verdict, Verdict::Current));
    assert_eq!(scene.disk.scanned(), NONE);
}

#[test]
fn world_states_whose_archive_did_not_open_are_read_again_at_the_next_open() {
    let scene = Scene::new();
    scene.disk.states_shut.store(true, Ordering::Relaxed);
    let without = scene.open();
    assert_eq!(
        without.index.default_on_in(13.0, 1, true),
        Vec::<u32>::new()
    );
    scene.disk.scanned();

    scene.disk.states_shut.store(false, Ordering::Relaxed);
    let with = scene.open();
    assert!(report(&with).derived && report(&with).read.is_empty());
    assert_eq!(
        scene.disk.scanned(),
        NONE,
        "no archive of the world is read"
    );
    assert_eq!(with.index.default_on_in(13.0, 1, true), [100]);
    let derived = scene.disk.derived();
    assert!(matches!(report(&scene.open()).verdict, Verdict::Current));
    assert_eq!(scene.disk.derived(), derived);
}

#[test]
fn a_kept_file_whose_sections_do_not_link_is_deleted_and_the_index_built_again() {
    let scene = Scene::new();
    let first = scene.open();
    let whole = std::fs::read(scene.file()).expect("the kept bytes");
    let changes: [fn(&mut Kept); 3] = [
        // An archive's rows that count other maps than the derived section places.
        |kept| {
            kept.archives[1] = Section::new();
            Rows::default().put(&mut kept.archives[1]);
        },
        // A derived section with a byte past its last table.
        |kept| kept.whole.put_u8(0),
        // A section of no kind.
        |kept| kept.archives[0] = Section::new(),
    ];
    for change in changes {
        rewrite(&scene, change);
        scene.disk.scanned();
        let again = scene.open();
        assert!(matches!(report(&again).verdict, Verdict::Absent));
        assert_eq!(scene.disk.scanned(), ["east.rpf", "west.rpf"]);
        assert_eq!(tables(&again.index), tables(&first.index));
        assert_eq!(std::fs::read(scene.file()).expect("kept again"), whole);
    }
}

#[test]
fn the_rows_of_a_section_whose_tables_count_other_maps_are_refused() {
    let mut shard = Shard::default();
    take_map(&mut shard, &[22, 100, 11]);
    take_map(&mut shard, &[90]);
    let rows = Rows::of(shard);
    // One map fewer in the table of what each holds, and one parent fewer than the maps count.
    let mut holds = rows.clone();
    holds.maps.pop();
    let mut parents = rows.clone();
    parents.maps[0].parents.clear();
    // The same section written table by table reads; then each table out of step with the maps.
    for (holds_of, parents_of) in [(&rows, &rows), (&holds, &rows), (&rows, &parents)] {
        let mut section = Section::new();
        section.put_u8(1);
        let all: Vec<MapRow> = rows.maps.iter().map(|m| m.row).collect();
        put_pod(&mut section, &all);
        let counts: Vec<u32> = (rows.maps.iter()).map(|m| m.parents.len() as u32).collect();
        put_pod(&mut section, &counts);
        let named: Vec<u32> = (parents_of.maps.iter())
            .flat_map(|m| m.parents.iter().copied())
            .collect();
        put_pod(&mut section, &named);
        let held: Vec<u8> = holds_of.maps.iter().map(|m| u8::from(m.holds)).collect();
        section.put_bytes(&held);
        put_pod(&mut section, &vec![0u64; rows.maps.len()]);
        put_pod(&mut section, &rows.archetypes);
        // No light, the words of the groups, then no types body, no model, no interior, no
        // dictionary, nothing unread and no far stand-in.
        section.put_bytes(&[]);
        let words: Vec<u64> = (rows.words.iter())
            .flat_map(|(group, word)| [u64::from(*group), *word])
            .collect();
        put_pod(&mut section, &words);
        section.put_bytes(&[]);
        let models: Vec<u64> = rows.models.iter().flat_map(|(a, b)| [*a, *b]).collect();
        put_pod(&mut section, &models);
        section.put_bytes(&[]);
        for _ in 0..3 {
            section.put_u64(0);
        }
        let read = Rows::read(&section);
        match std::ptr::eq(holds_of, parents_of) {
            true => assert_eq!(read.expect("it reads"), Some(rows.clone())),
            false => assert!(read.is_err()),
        }
    }
}
