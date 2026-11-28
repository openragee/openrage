use std::sync::Arc;

use or_job::{Cancel, Progress};

use super::*;
use crate::caps::WorldCaps;
use crate::contract::{
    Keys, Layer, MapReader, ModelReader, Opening, WorldIndexer, WorldLook, WorldTitle,
};
use crate::fake::{self, FakeTitle};
use crate::policy::StreamPolicy;
use crate::slots::{Extras, OptionId, OptionValue};

fn open(title: &FakeTitle, config: &Path) -> Arc<dyn WorldInstall> {
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config,
        keys: Keys::None,
        cancel: &cancel,
        progress: &progress,
    };
    title
        .open(&FakeTitle::spec(), &opening)
        .expect("the fake world opens")
}

/// One open of the fake world, its index kept under `config`.
fn refreshed(title: &FakeTitle, config: &Path) -> (Index, Report) {
    let world = FakeTitle::spec();
    let file = file_for(config, title.id(), &world);
    let install = open(title, config);
    let (index, report) = refresh(title.id(), &world, &*install, &file).expect("the index builds");
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    (index, report)
}

fn rows_read(title: &FakeTitle) -> [usize; 2] {
    [title.counts().rows_of(0), title.counts().rows_of(1)]
}

fn sample() -> Kept {
    let mut first = Section::new();
    first.put_u32(7);
    first.put_text("a row");
    let mut whole = Section::new();
    whole.put_u64(u64::MAX);
    whole.put_f32(1.5);
    Kept {
        head: Head {
            title: "fake".into(),
            world: "a world\nof two lines".into(),
            rules: 3,
            inputs: Inputs {
                archives: vec![
                    Input {
                        name: "one".into(),
                        stamp: Some(Stamp {
                            size: 10,
                            modified: 20,
                        }),
                    },
                    Input {
                        name: "two".into(),
                        stamp: None,
                    },
                ],
                others: vec![Input {
                    name: "beside".into(),
                    stamp: Some(Stamp {
                        size: 1,
                        modified: 2,
                    }),
                }],
            },
        },
        archives: vec![first, Section::new()],
        whole,
    }
}

#[test]
fn a_kept_file_round_trips() {
    let kept = sample();
    let sealed = kept.seal();
    assert_eq!(Kept::unseal(&sealed, 3).expect("it reads back"), kept);
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("deep").join("kept.index");
    kept.keep(&file).expect("it is written");
    assert_eq!(Kept::open(&file, 3).expect("it opens"), Some(kept));
    let beside: Vec<_> = std::fs::read_dir(file.parent().expect("a folder"))
        .expect("the folder lists")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(beside, ["kept.index"], "nothing is left beside the file");
}

#[test]
fn a_section_reads_back_what_was_put_and_refuses_more() {
    let kept = sample();
    let mut from = kept.archives[0].reader();
    assert_eq!(from.u32(), Ok(7));
    assert_eq!(from.text(), Ok("a row"));
    assert_eq!(from.end(), Ok(()));
    assert_eq!(from.u8(), Err(SectionError::CutShort));
    let mut whole = kept.whole.reader();
    assert_eq!(whole.u64(), Ok(u64::MAX));
    assert_eq!(whole.end(), Err(SectionError::BytesLeft));
    assert_eq!(whole.f32(), Ok(1.5));
}

#[test]
fn a_kept_file_of_another_rules_version_is_refused() {
    let sealed = sample().seal();
    match Kept::unseal(&sealed, 4) {
        Err(KeptError::Rules { found: 3, reads: 4 }) => {}
        other => panic!("read under other rules it gave {other:?}"),
    }
}

#[test]
fn a_kept_file_cut_short_altered_or_of_another_format_is_refused() {
    let sealed = sample().seal();
    for cut in 0..sealed.len() {
        assert!(Kept::unseal(&sealed[..cut], 3).is_err(), "cut at {cut}");
    }
    for at in 0..sealed.len() {
        let mut altered = sealed.clone();
        altered[at] ^= 1;
        assert!(Kept::unseal(&altered, 3).is_err(), "altered at {at}");
    }
    let mut longer = sealed.clone();
    longer.push(0);
    assert!(matches!(Kept::unseal(&longer, 3), Err(KeptError::Damaged)));
    let mut altered = sealed.clone();
    *altered.last_mut().expect("a byte") ^= 1;
    assert!(matches!(Kept::unseal(&altered, 3), Err(KeptError::Damaged)));
    let mut other = sealed.clone();
    other[4..8].copy_from_slice(&(FORMAT + 1).to_le_bytes());
    assert!(matches!(
        Kept::unseal(&other, 3),
        Err(KeptError::Format { found }) if found == FORMAT + 1
    ));
    assert!(matches!(
        Kept::unseal(b"not a kept index at all", 3),
        Err(KeptError::NotAKeptIndex)
    ));
}

#[test]
fn a_second_open_reads_no_archive_and_links_the_same_index() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let (built, first) = refreshed(&title, folder.path());
    assert!(matches!(first.verdict, Verdict::Absent));
    assert_eq!((first.read.clone(), first.derived), (vec![0, 1], true));
    assert_eq!(rows_read(&title), [1, 1]);

    let (kept, second) = refreshed(&title, folder.path());
    assert!(
        matches!(second.verdict, Verdict::Current),
        "{:?}",
        second.verdict
    );
    assert_eq!((second.read.clone(), second.derived), (vec![], false));
    assert_eq!(
        rows_read(&title),
        [1, 1],
        "a kept archive is not read again"
    );
    assert_eq!(kept.maps(), built.maps());
    assert_eq!(kept.named(fake::YARD), built.named(fake::YARD));
}

#[test]
fn a_changed_stamp_reads_that_archive_again_and_leaves_the_others() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let world = FakeTitle::spec();
    let file = file_for(folder.path(), title.id(), &world);
    refreshed(&title, folder.path());
    let before = Kept::open(&file, fake::RULES)
        .expect("it opens")
        .expect("it is there");

    title.touch(1);
    let (_, report) = refreshed(&title, folder.path());
    assert!(
        matches!(report.verdict, Verdict::Changed),
        "{:?}",
        report.verdict
    );
    assert_eq!((report.read.clone(), report.derived), (vec![1], true));
    assert_eq!(
        rows_read(&title),
        [1, 2],
        "only the changed archive is read again"
    );
    let after = Kept::open(&file, fake::RULES)
        .expect("it opens")
        .expect("it is there");
    assert_eq!(after.archives[0], before.archives[0]);
    assert_ne!(
        after.head.inputs.archives[1],
        before.head.inputs.archives[1]
    );
    assert_eq!(
        after.head.inputs.archives[0],
        before.head.inputs.archives[0]
    );

    let (_, settled) = refreshed(&title, folder.path());
    assert!(
        matches!(settled.verdict, Verdict::Current),
        "{:?}",
        settled.verdict
    );
    assert_eq!(rows_read(&title), [1, 2]);
}

#[test]
fn the_same_archives_in_another_order_are_not_read_and_are_derived_across_again() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let (built, _) = refreshed(&title, folder.path());
    title.reverse_archives();
    let (again, report) = refreshed(&title, folder.path());
    assert!(
        matches!(report.verdict, Verdict::Changed),
        "{:?}",
        report.verdict
    );
    assert_eq!((report.read.clone(), report.derived), (vec![], true));
    assert_eq!(
        rows_read(&title),
        [1, 1],
        "an archive that stands is not read again"
    );
    // The rows are the same maps, linked in the order the archives are now listed in.
    assert_eq!(again.maps().len(), built.maps().len());
    assert_eq!(again.named(fake::YARD).map(|id| id.0), Some(2));
    assert_eq!(built.named(fake::YARD).map(|id| id.0), Some(4));
    let (_, settled) = refreshed(&title, folder.path());
    assert!(matches!(settled.verdict, Verdict::Current));
}

#[test]
fn a_kept_file_of_other_rules_or_another_world_serves_nothing() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let world = FakeTitle::spec();
    let file = file_for(folder.path(), title.id(), &world);
    refreshed(&title, folder.path());

    let mut kept = Kept::open(&file, fake::RULES)
        .expect("it opens")
        .expect("it is there");
    kept.head.rules = fake::RULES + 1;
    kept.keep(&file).expect("it is written");
    let (_, report) = refreshed(&title, folder.path());
    assert!(
        matches!(report.verdict, Verdict::Refused(KeptError::Rules { .. })),
        "{:?}",
        report.verdict
    );
    assert_eq!(report.read, [0, 1]);
    assert_eq!(rows_read(&title), [2, 2], "every archive is read again");

    let mut kept = Kept::open(&file, fake::RULES)
        .expect("it opens")
        .expect("it is there");
    kept.head.world.push_str("\n+a layer");
    kept.keep(&file).expect("it is written");
    let (_, report) = refreshed(&title, folder.path());
    assert!(
        matches!(report.verdict, Verdict::Foreign),
        "{:?}",
        report.verdict
    );
    assert_eq!(rows_read(&title), [3, 3]);
}

#[test]
fn two_worlds_of_one_install_keep_two_files() {
    let config = Path::new("config");
    let plain = FakeTitle::spec();
    let mut chosen = plain.clone();
    chosen.options.push((OptionId(1), OptionValue::On(true)));
    assert_ne!(plain.identity(), chosen.identity());
    assert_ne!(
        file_for(config, "fake", &plain),
        file_for(config, "fake", &chosen)
    );
    assert_ne!(
        file_for(config, "fake", &plain),
        file_for(config, "other", &plain)
    );
}

/// An opened fake world that lists other inputs than its own.
struct Listed<F> {
    inner: Arc<dyn WorldInstall>,
    alter: F,
}

impl<F: Fn(&mut Inputs) + Send + Sync> WorldInstall for Listed<F> {
    fn caps(&self) -> WorldCaps {
        self.inner.caps()
    }

    fn policy(&self) -> StreamPolicy {
        self.inner.policy()
    }

    fn extras(&self) -> Extras {
        self.inner.extras()
    }

    fn inputs(&self) -> Inputs {
        let mut inputs = self.inner.inputs();
        (self.alter)(&mut inputs);
        inputs
    }

    fn indexer(&self) -> &dyn WorldIndexer {
        self.inner.indexer()
    }

    fn maps(&self) -> &dyn MapReader {
        self.inner.maps()
    }

    fn models(&self) -> &dyn ModelReader {
        self.inner.models()
    }

    fn look(&self) -> &dyn WorldLook {
        self.inner.look()
    }
}

/// One open of the fake world with its inputs as `alter` leaves them, its index kept under `config`.
fn refreshed_as(
    title: &FakeTitle,
    config: &Path,
    alter: impl Fn(&mut Inputs) + Send + Sync,
) -> Report {
    let world = FakeTitle::spec();
    let file = file_for(config, title.id(), &world);
    let listed = Listed {
        inner: open(title, config),
        alter,
    };
    let (_, report) = refresh(title.id(), &world, &listed, &file).expect("the index builds");
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    report
}

#[test]
fn an_archive_with_no_stamp_is_read_at_every_open() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let unstamped = |inputs: &mut Inputs| inputs.archives[1].stamp = None;
    let first = refreshed_as(&title, folder.path(), unstamped);
    assert!(matches!(first.verdict, Verdict::Absent));
    for opens in 2..=3 {
        let report = refreshed_as(&title, folder.path(), unstamped);
        assert!(
            matches!(report.verdict, Verdict::Changed),
            "{:?}",
            report.verdict
        );
        assert_eq!((report.read.clone(), report.derived), (vec![1], true));
        assert_eq!(rows_read(&title), [1, opens]);
    }
}

#[test]
fn what_else_the_index_reads_derives_across_again_when_it_changes_or_bears_no_stamp() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let stamp = |modified: u64| Some(Stamp { size: 1, modified });
    let opens = [
        (stamp(1), "Absent", true),
        (stamp(1), "Current", false),
        (stamp(2), "Changed", true),
        (stamp(2), "Current", false),
        (None, "Changed", true),
        (None, "Changed", true),
    ];
    for (at, (stamp, verdict, derived)) in opens.into_iter().enumerate() {
        let beside = move |inputs: &mut Inputs| {
            inputs.others.push(Input {
                name: "beside".into(),
                stamp,
            });
        };
        let report = refreshed_as(&title, folder.path(), beside);
        assert_eq!(
            (format!("{:?}", report.verdict).as_str(), report.derived),
            (verdict, derived),
            "open {at}"
        );
    }
    assert_eq!(
        rows_read(&title),
        [1, 1],
        "no archive is read again for what stands beside them"
    );
}

#[test]
fn a_kept_file_another_title_wrote_serves_nothing() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let file = file_for(folder.path(), title.id(), &FakeTitle::spec());
    refreshed(&title, folder.path());
    let mut kept = Kept::open(&file, fake::RULES)
        .expect("it opens")
        .expect("it is there");
    kept.head.title = "another".into();
    kept.keep(&file).expect("it is written");
    let (_, report) = refreshed(&title, folder.path());
    assert!(
        matches!(report.verdict, Verdict::Foreign),
        "{:?}",
        report.verdict
    );
    assert_eq!(rows_read(&title), [2, 2]);
}

#[test]
fn a_world_differs_by_its_layers_and_their_order_and_by_no_order_of_its_options() {
    let plain = FakeTitle::spec();
    let layered = |paths: &[&str]| WorldSpec {
        layers: (paths.iter())
            .map(|path| Layer { path: path.into() })
            .collect(),
        ..plain.clone()
    };
    let worlds = [
        plain.clone(),
        layered(&["a"]),
        layered(&["a", "b"]),
        layered(&["b", "a"]),
        // One layer whose name holds a line break and the mark of a layer is not two layers.
        layered(&["a\n+b"]),
        // An install whose name holds a line break and the mark of a layer is no install with a layer.
        WorldSpec::of(fake::GAME, "fake-install\n+a"),
        // Nor is one whose name ends as a quoted layer is written.
        WorldSpec::of(fake::GAME, "fake-install\n+\"a\""),
    ];
    for (at, one) in worlds.iter().enumerate() {
        for other in &worlds[at + 1..] {
            assert_ne!(one.identity(), other.identity(), "{one:?} and {other:?}");
        }
    }

    let (mut one, mut other) = (plain.clone(), plain);
    let (mode, pack) = (
        (OptionId(1), OptionValue::On(true)),
        (OptionId(2), OptionValue::Named("a pack".into())),
    );
    one.options = vec![mode.clone(), pack.clone()];
    other.options = vec![pack, mode];
    assert_eq!(one.identity(), other.identity());
}

#[test]
fn a_stamp_tells_a_file_written_again_from_the_one_read_before() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("an archive");
    assert_eq!(Stamp::of(&file), None, "nothing can be asked of no file");
    std::fs::write(&file, b"four").expect("it is written");
    let first = Stamp::of(&file).expect("a file has a stamp");
    assert_eq!(first.size, 4);
    assert_eq!(
        Stamp::of(&file),
        Some(first),
        "a file that stands keeps its stamp"
    );

    // As many bytes, written a second later: the time alone tells the two apart.
    let written = std::fs::metadata(&file).and_then(|meta| meta.modified());
    let later = written.expect("a time of writing") + std::time::Duration::from_secs(1);
    let open = std::fs::File::options().write(true).open(&file);
    (open.and_then(|file| file.set_modified(later))).expect("the time is set");
    let second = Stamp::of(&file).expect("a file has a stamp");
    assert_eq!(second.size, first.size);
    assert!(
        second.modified > first.modified,
        "{second:?} after {first:?}"
    );

    std::fs::write(&file, b"seven..").expect("it is written again");
    assert_eq!(Stamp::of(&file).map(|stamp| stamp.size), Some(7));
}

#[test]
fn an_archive_whose_stamp_went_backwards_is_read_again() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    title.touch(1);
    refreshed(&title, folder.path());
    let older = |inputs: &mut Inputs| {
        let stamp = inputs.archives[1].stamp.as_mut().expect("a stamp");
        stamp.modified -= 1;
    };
    let report = refreshed_as(&title, folder.path(), older);
    assert!(
        matches!(report.verdict, Verdict::Changed),
        "{:?}",
        report.verdict
    );
    assert_eq!(report.read, [1]);
    assert_eq!(rows_read(&title), [1, 2]);
}

/// A kept file of one archive whose section is `len` bytes of `byte`.
fn filled(byte: u8, len: usize) -> Kept {
    let mut section = Section::new();
    section.put_bytes(&vec![byte; len]);
    let mut kept = sample();
    kept.head.inputs.archives.truncate(1);
    kept.archives = vec![section];
    kept
}

#[test]
fn two_writers_at_once_leave_a_whole_file_of_one_of_them() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("kept.index");
    let (one, other) = (filled(1, 300_000), filled(2, 200_000));
    let done = std::sync::atomic::AtomicBool::new(false);
    let read = || match Kept::open(&file, 3) {
        Ok(None) => {}
        Ok(Some(kept)) => assert!(kept == one || kept == other, "a file of neither writer"),
        Err(e) => panic!("a reader found a file that is no whole kept index: {e}"),
    };
    std::thread::scope(|scope| {
        let writers: Vec<_> = [&one, &other]
            .into_iter()
            .map(|kept| {
                scope.spawn(|| {
                    let kept: Vec<_> = (0..40).map(|_| kept.keep(&file)).collect();
                    kept.into_iter().filter_map(Result::err).count()
                })
            })
            .collect();
        scope.spawn(|| {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                read();
            }
        });
        let failed: usize = (writers.into_iter())
            .map(|writer| writer.join().expect("a writer ends"))
            .sum();
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        // ⚠ Elsewhere than Unix a file a reader holds open may refuse to be replaced: the write
        // then says so, and the file that stood stands.
        assert!(!cfg!(unix) || failed == 0, "{failed} writes failed");
    });
    read();
    assert!(Kept::open(&file, 3).expect("it opens").is_some());
    let beside: Vec<_> = std::fs::read_dir(folder.path())
        .expect("the folder lists")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(beside, ["kept.index"], "nothing is left beside the file");
}

#[test]
fn a_file_past_the_most_a_kept_index_is_read_to_is_refused_unread() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("kept.index");
    // No kept index by its first bytes: read, it would be refused as none, and not as damaged.
    let made = std::fs::File::create(&file).expect("the file is made");
    made.set_len(MOST_BYTES + 1)
        .expect("the file is lengthened");
    drop(made);
    assert!(matches!(Kept::open(&file, 3), Err(KeptError::Damaged)));
}

#[test]
fn a_file_that_cannot_be_replaced_is_left_with_nothing_written_beside_it() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    // A folder that holds a file stands where the kept file is to go: nothing moves over it.
    let file = folder.path().join("kept.index");
    std::fs::create_dir(&file).expect("the folder is made");
    std::fs::write(file.join("held"), b"held").expect("the file is written");
    assert!(sample().keep(&file).is_err());
    let beside: Vec<_> = std::fs::read_dir(folder.path())
        .expect("the folder lists")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(beside, ["kept.index"]);
}

/// An indexer of one archive, whose rows read in part while it is shut.
#[derive(Default)]
struct Halting {
    shut: std::sync::atomic::AtomicBool,
    reads: std::sync::atomic::AtomicUsize,
}

impl WorldIndexer for Halting {
    fn rules(&self) -> u32 {
        1
    }

    fn rows(&self, _archive: usize, out: &mut Section) -> Result<(), WorldError> {
        use std::sync::atomic::Ordering;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let shut = self.shut.load(Ordering::Relaxed);
        out.put_u8(u8::from(!shut));
        if shut {
            out.in_part();
        }
        Ok(())
    }

    fn whole(&self, _archives: &[Section], _out: &mut Section) -> Result<(), WorldError> {
        Ok(())
    }

    fn link(&self, archives: &[Section], _whole: &Section) -> Result<Index, WorldError> {
        let read = archives[0].reader().u8()?;
        let row = crate::model::MapRow {
            name: u64::from(read),
            label: String::new(),
            boxes: None,
            parents: Vec::new(),
            gate: crate::model::Gate::Always,
            archive: 0,
        };
        Ok(Index::new(vec![row])?)
    }

    fn placements(
        &self,
        _index: &Index,
        _map: crate::model::MapId,
        _out: &mut Vec<crate::model::PlacementRow>,
    ) -> Result<(), WorldError> {
        Err(WorldError::Lacks("placement rows"))
    }
}

#[test]
fn an_archive_read_in_part_keeps_no_stamp_and_is_read_again_at_the_next_open() {
    use std::sync::atomic::Ordering;
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("kept.index");
    let world = FakeTitle::spec();
    let stamp = Some(Stamp {
        size: 10,
        modified: 20,
    });
    let inputs = || Inputs {
        archives: vec![Input {
            name: "one".into(),
            stamp,
        }],
        others: Vec::new(),
    };
    let indexer = Halting::default();
    indexer.shut.store(true, Ordering::Relaxed);
    let open = || refresh_with("halting", &world, inputs(), &indexer, &file).expect("it opens");
    let kept_stamp = || {
        let kept = Kept::open(&file, 1)
            .expect("it reads")
            .expect("it is there");
        kept.head.inputs.archives[0].stamp
    };

    let (index, report) = open();
    assert_eq!(index.maps()[0].name, 0, "what could be read is linked");
    assert_eq!(report.read, [0]);
    assert_eq!(
        kept_stamp(),
        None,
        "the stamp taken before the read is not kept"
    );

    // The archive stands as it was, and its rows are asked for again all the same.
    indexer.shut.store(false, Ordering::Relaxed);
    let (index, report) = open();
    assert_eq!((index.maps()[0].name, report.read), (1, vec![0]));
    assert!(matches!(report.verdict, Verdict::Changed));
    assert_eq!(kept_stamp(), stamp);
    let (_, report) = open();
    assert!(matches!(report.verdict, Verdict::Current));
    assert_eq!(indexer.reads.load(Ordering::Relaxed), 2);
}

/// An indexer of one archive, whose derived section is made in part while it is shut.
#[derive(Default)]
struct Wavering {
    shut: std::sync::atomic::AtomicBool,
    reads: std::sync::atomic::AtomicUsize,
    derived: std::sync::atomic::AtomicUsize,
}

impl WorldIndexer for Wavering {
    fn rules(&self) -> u32 {
        1
    }

    fn rows(&self, _archive: usize, _out: &mut Section) -> Result<(), WorldError> {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn whole(&self, _archives: &[Section], out: &mut Section) -> Result<(), WorldError> {
        use std::sync::atomic::Ordering;
        self.derived.fetch_add(1, Ordering::Relaxed);
        let shut = self.shut.load(Ordering::Relaxed);
        out.put_u8(u8::from(!shut));
        if shut {
            out.in_part();
        }
        Ok(())
    }

    fn link(&self, _archives: &[Section], whole: &Section) -> Result<Index, WorldError> {
        let row = crate::model::MapRow {
            name: u64::from(whole.reader().u8()?),
            label: String::new(),
            boxes: None,
            parents: Vec::new(),
            gate: crate::model::Gate::Always,
            archive: 0,
        };
        Ok(Index::new(vec![row])?)
    }

    fn placements(
        &self,
        _index: &Index,
        _map: crate::model::MapId,
        _out: &mut Vec<crate::model::PlacementRow>,
    ) -> Result<(), WorldError> {
        Err(WorldError::Lacks("placement rows"))
    }
}

#[test]
fn a_derived_section_made_in_part_is_made_again_at_the_next_open_and_no_archive_is_read() {
    use std::sync::atomic::Ordering;
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("kept.index");
    let world = FakeTitle::spec();
    let stamp = Some(Stamp {
        size: 10,
        modified: 20,
    });
    // With another input and with none: neither says a section made in part stands.
    for others in [Vec::new(), vec![("else", stamp)]] {
        let inputs = || Inputs {
            archives: vec![Input {
                name: "one".into(),
                stamp,
            }],
            others: (others.iter())
                .map(|(name, stamp)| Input {
                    name: (*name).into(),
                    stamp: *stamp,
                })
                .collect(),
        };
        let _ = std::fs::remove_file(&file);
        let indexer = Wavering::default();
        indexer.shut.store(true, Ordering::Relaxed);
        let open =
            || refresh_with("wavering", &world, inputs(), &indexer, &file).expect("it opens");
        let counts = || {
            (
                indexer.reads.load(Ordering::Relaxed),
                indexer.derived.load(Ordering::Relaxed),
            )
        };

        let (index, _) = open();
        assert_eq!(index.maps()[0].name, 0, "what could be made is linked");
        assert_eq!(counts(), (1, 1));
        // While it is made in part it is made at every open, from the rows kept.
        let (_, report) = open();
        assert!(report.derived && report.read.is_empty());
        assert_eq!(counts(), (1, 2));

        indexer.shut.store(false, Ordering::Relaxed);
        let (index, report) = open();
        assert_eq!(index.maps()[0].name, 1);
        assert!(matches!(report.verdict, Verdict::Changed));
        assert!(report.derived && report.read.is_empty());
        let (index, report) = open();
        assert_eq!(index.maps()[0].name, 1);
        assert!(matches!(report.verdict, Verdict::Current));
        assert_eq!(counts(), (1, 3));
        let kept = (Kept::open(&file, 1).expect("it reads")).expect("it is there");
        assert_eq!(
            kept.head.inputs,
            inputs(),
            "the inputs alone, once it is whole"
        );
    }
}
