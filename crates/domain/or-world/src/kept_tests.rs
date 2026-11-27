use std::sync::Arc;

use or_job::{Cancel, Progress};

use super::*;
use crate::contract::{Keys, Opening, WorldTitle};
use crate::fake::{self, FakeTitle};

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
    for cut in [0, 3, 11, sealed.len() - 1] {
        assert!(Kept::unseal(&sealed[..cut], 3).is_err(), "cut at {cut}");
    }
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
    chosen.options.push((
        crate::slots::OptionId(1),
        crate::slots::OptionValue::On(true),
    ));
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
