//! Importing a game file: its bytes become a source folder that a person can edit, beside the project
//! file, and nothing about the file it came from, or a source that is there already, is changed.

mod common;
mod model_fixtures;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::{as_the_game_ships_it, edit_rules, entry, legacy_dictionary, ramp, Spec};
use model_fixtures::{drawable, geometry, ydd, ydr};
use or_io::codec::texture::GrcTextureFormat;
use or_io::container::{RpfArchive, RpfBuilder, RpfEntry};
use or_source::asset::{reid, sha256_hex, How, Source};
use or_source::cook::{describe, Cooker, Recook};
use or_source::game_import::{
    import, import_watched, kind_of, tree_digest, Bounds, Facts, FileReport, Input, Options,
    Report, Verdict, Watch, KINDS,
};
use or_source::{bounds, fragment, git, metadata, models, particles, paths, textures};
use serde_json::{json, Value};

/// How many files of each extension the corpus check imports, spread over the staged ones.
const SAMPLE: usize = 4;

fn ytd(texture: &'static str, salt: u32) -> Vec<u8> {
    legacy_dictionary(&[Spec::new(
        texture,
        GrcTextureFormat::DXT5,
        ramp(16, 16, salt),
    )])
}

fn jacket() -> Vec<u8> {
    ydr(drawable("jacket", vec![geometry(6, false, 0.0)], None))
}

fn dictionary() -> Vec<u8> {
    ydd(vec![
        (
            "jbib_000_u",
            drawable("jbib_000_u", vec![geometry(6, false, 0.0)], None),
        ),
        (
            "jbib_001_u",
            drawable("jbib_001_u", vec![geometry(9, false, 3.0)], None),
        ),
    ])
}

fn game(virtual_path: &str) -> Facts {
    Facts {
        from_game: true,
        virtual_path: Some(virtual_path.to_string()),
        game_build: Some("3258".to_string()),
        ..Facts::default()
    }
}

fn loose() -> Facts {
    Facts::default()
}

/// A folder of files a person took out of the game: no archive path can be known.
fn the_games() -> Facts {
    Facts {
        from_game: true,
        ..Facts::default()
    }
}

fn given<'a>(name: &'a str, bytes: &'a [u8], facts: Facts) -> Input<'a> {
    Input::Bytes { name, bytes, facts }
}

fn run(root: &Path, inputs: &[Input<'_>]) -> Report {
    import(root, inputs, &Options::default())
}

fn only(report: &Report) -> &FileReport {
    assert_eq!(report.files.len(), 1, "{report:#?}");
    &report.files[0]
}

fn made(report: &Report) -> PathBuf {
    match &only(report).verdict {
        Verdict::Made { folder } => folder.clone(),
        other => panic!("expected a source to be made, got {other:?}: {report:#?}"),
    }
}

fn tree(path: &Path) -> String {
    tree_digest(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// What a source compiles to, whichever kind it is.
fn compile(dir: &Path) -> Vec<u8> {
    let source = Source::open(dir).unwrap();
    match source.manifest.kind.as_str() {
        textures::KIND => textures::compile(&source).map(|c| c.bytes),
        _ => models::Inputs::read(&source)
            .and_then(|inputs| inputs.compile())
            .map(|c| c.bytes),
    }
    .unwrap_or_else(|problems| panic!("{}: {problems:#?}", dir.display()))
}

/// The names in `dir`, sorted; none when it is not there.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn write(path: &Path, body: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// Sets a file's modification time to a month ago, so a write after it shows.
fn age(path: &Path) -> SystemTime {
    let then = SystemTime::now() - Duration::from_secs(30 * 86_400);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(then)
        .unwrap();
    fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn the_kind_table_has_a_row_for_each_format_that_becomes_a_source_and_for_no_other() {
    let rows: Vec<(&str, &str)> = KINDS.iter().map(|r| (r.extension, r.kind)).collect();
    assert_eq!(
        rows,
        [
            ("ytd", textures::KIND),
            ("ydr", models::DRAWABLE),
            ("ydd", models::DRAWABLES),
            ("yft", fragment::KIND),
            ("ymt", metadata::KIND),
            ("ymap", metadata::KIND),
            ("ytyp", metadata::KIND),
            ("ypt", particles::KIND),
            ("ybn", bounds::KIND),
            ("ynd", paths::KIND),
        ]
    );
    assert_eq!(kind_of("YTD").map(|r| r.kind), Some("textures"));
    // A `.meta` is a FiveM data file registered by its own path, and a source ships at stream/.
    for none in ["ycd", "awc", "yld", "ynv", "meta", "xml", "txt", ""] {
        assert!(kind_of(none).is_none(), ".{none}");
    }
}

#[test]
fn a_texture_dictionary_a_drawable_and_a_dictionary_become_sources_that_say_where_they_came_from() {
    let project = tempfile::tempdir().unwrap();
    let (a, b, c) = (ytd("bark", 1), jacket(), dictionary());
    let report = run(
        project.path(),
        &[
            given("bark_set.ytd", &a, game("x64a.rpf/textures/bark_set.ytd")),
            given("jacket.ydr", &b, game("x64a.rpf/models/jacket.ydr")),
            given("jbib_u.ydd", &c, loose()),
        ],
    );
    assert_eq!(report.files.len(), 3, "{report:#?}");
    let folders: Vec<PathBuf> = report
        .files
        .iter()
        .map(|f| match &f.verdict {
            Verdict::Made { folder } => folder.clone(),
            other => panic!("{}: {other:?}: {}", f.input, f.reason),
        })
        .collect();
    let sources = project.path().join("sources");
    assert_eq!(
        names(&sources),
        [
            "bark_set.textures.src",
            "jacket.drawable.src",
            "jbib_u.drawables.src"
        ],
        "no staging folder is left"
    );
    assert_eq!(
        folders,
        [
            sources.join("bark_set.textures.src"),
            sources.join("jacket.drawable.src"),
            sources.join("jbib_u.drawables.src")
        ]
    );

    let first = Source::open(&folders[0]).unwrap().manifest;
    assert_eq!(first.name, "bark_set");
    assert_eq!(first.origin.how, How::Extracted);
    assert!(first.origin.from_game);
    assert_eq!(
        first.origin.virtual_path.as_deref(),
        Some("x64a.rpf/textures/bark_set.ytd")
    );
    assert_eq!(first.origin.game_build.as_deref(), Some("3258"));
    assert_eq!(first.origin.sha256, Some(sha256_hex(&a)));

    let third = Source::open(&folders[2]).unwrap().manifest;
    assert_eq!(
        third.origin.how,
        How::Imported,
        "a file nobody says is the game's"
    );
    assert!(!third.origin.from_game);
    assert_eq!(third.origin.sha256, Some(sha256_hex(&c)));
    assert_eq!(third.origin.game_build, None);

    assert_eq!(report.created.len(), 3);
    for (created, folder) in report.created.iter().zip(&folders) {
        assert_eq!(&created.path, folder);
        assert_eq!(created.digest, tree(folder));
    }
}

#[test]
fn a_corpus_ytd_ydr_and_ydd_import_and_compile_back_as_a_direct_extract_does() {
    let mut compared = Vec::new();
    for extension in ["ytd", "ydr", "ydd"] {
        let Some(files) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), extension)
            .all_targets()
            .find_or_skip(&format!("a corpus .{extension} imports and compiles back"))
        else {
            continue;
        };
        let step = (files.len() / SAMPLE).max(1);
        for path in files.iter().step_by(step).take(SAMPLE) {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let name = file.rsplit_once('.').unwrap().0.to_string();
            let bytes = fs::read(path).unwrap();
            let virtual_path = format!("x64a.rpf/{file}");

            let direct = tempfile::tempdir().unwrap();
            let how = textures::Extraction {
                from_game: true,
                virtual_path: Some(&virtual_path),
            };
            let models_how = models::Extraction {
                from_game: true,
                virtual_path: Some(&virtual_path),
                textures: false,
                skeleton: None,
            };
            let extracted = match extension {
                "ytd" => textures::extract(&bytes, &name, direct.path(), &how),
                "ydr" => models::extract_drawable(&bytes, &name, direct.path(), &models_how),
                _ => models::extract_drawables(&bytes, &name, direct.path(), &models_how),
            };

            let project = tempfile::tempdir().unwrap();
            let report = run(project.path(), &[given(&file, &bytes, game(&virtual_path))]);
            match extracted {
                Ok(expected) => {
                    let folder = made(&report);
                    assert_eq!(
                        compile(&folder),
                        compile(&expected),
                        "{file}: the import compiles to something else than a direct extract does"
                    );
                    compared.push(file);
                }
                Err(_) => assert_eq!(
                    only(&report).verdict,
                    Verdict::Refused,
                    "{file}: a direct extract refuses it, so the import has to"
                ),
            }
        }
    }
    if !compared.is_empty() {
        eprintln!("compared {} corpus file(s): {compared:?}", compared.len());
    }
}

#[test]
fn the_same_bytes_twice_are_already_a_source_and_the_second_import_writes_nothing() {
    let project = tempfile::tempdir().unwrap();
    let bytes = ytd("bark", 1);
    let facts = game("x64a.rpf/bark_set.ytd");
    let first = made(&run(
        project.path(),
        &[given("bark_set.ytd", &bytes, facts.clone())],
    ));

    let before = tree(project.path());
    let second = run(project.path(), &[given("bark_set.ytd", &bytes, facts)]);
    assert_eq!(
        only(&second).verdict,
        Verdict::AlreadyASource {
            folder: first.clone()
        },
        "{second:#?}"
    );
    assert!(second.created.is_empty());
    assert!(!only(&second).reason.is_empty());
    assert_eq!(
        tree(project.path()),
        before,
        "an import that adds nothing writes nothing"
    );

    // A loose file is the same file by its hash alone, wherever it is.
    let file = project.path().join("incoming").join("bark_copy.ytd");
    write(&file, &bytes);
    let third = run(
        project.path(),
        &[Input::Path {
            path: &file,
            facts: loose(),
        }],
    );
    assert_eq!(
        only(&third).verdict,
        Verdict::AlreadyASource { folder: first },
        "{third:#?}"
    );
    assert_eq!(names(&project.path().join("sources")).len(), 1);
}

#[test]
fn two_different_files_with_one_name_get_a_numbered_folder_and_the_first_is_untouched() {
    let project = tempfile::tempdir().unwrap();
    let sources = project.path().join("sources");
    let (one, two, three) = (ytd("bark", 1), ytd("bark", 2), ytd("bark", 3));

    let a = made(&run(
        project.path(),
        &[given("bark_set.ytd", &one, game("x64a.rpf/bark_set.ytd"))],
    ));
    let a_before = tree(&a);

    let b = made(&run(
        project.path(),
        &[given("bark_set.ytd", &two, game("update.rpf/bark_set.ytd"))],
    ));
    assert_eq!(b, sources.join("bark_set_2.textures.src"));
    assert_eq!(
        Source::open(&b).unwrap().manifest.name,
        "bark_set",
        "the name is what the game calls the file; only the folder is numbered"
    );
    // The same bytes under another archive path are another origin.
    let c = made(&run(
        project.path(),
        &[given("bark_set.ytd", &one, game("dlc.rpf/bark_set.ytd"))],
    ));
    assert_eq!(c, sources.join("bark_set_3.textures.src"));
    let d = made(&run(
        project.path(),
        &[given("bark_set.ytd", &three, loose())],
    ));
    assert_eq!(d, sources.join("bark_set_4.textures.src"));

    assert_eq!(tree(&a), a_before, "the first source is not touched");

    // Each of them is found again by its origin, whatever its number.
    for (bytes, path, folder) in [
        (&one, "x64a.rpf/bark_set.ytd", &a),
        (&two, "update.rpf/bark_set.ytd", &b),
        (&one, "dlc.rpf/bark_set.ytd", &c),
    ] {
        let again = run(project.path(), &[given("bark_set.ytd", bytes, game(path))]);
        assert_eq!(
            only(&again).verdict,
            Verdict::AlreadyASource {
                folder: folder.clone()
            }
        );
    }
    assert_eq!(names(&sources).len(), 4);
}

#[test]
fn an_import_never_modifies_its_input_a_file_a_folder_or_the_archive_an_entry_came_out_of() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    let input = work.path().join("input");
    let bytes = ytd("bark", 1);
    let file = input.join("bark_set.ytd");
    let kept = input.join("notes").join("readme.txt");
    write(&file, &bytes);
    write(&kept, b"hello");
    let (file_time, kept_time) = (age(&file), age(&kept));
    let folder_time = fs::metadata(&input).unwrap().modified().unwrap();

    let report = run(
        &project,
        &[Input::Path {
            path: &input,
            facts: the_games(),
        }],
    );
    assert_eq!(report.files.len(), 2, "{report:#?}");
    assert_eq!(fs::read(&file).unwrap(), bytes);
    assert_eq!(fs::read(&kept).unwrap(), b"hello");
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), file_time);
    assert_eq!(fs::metadata(&kept).unwrap().modified().unwrap(), kept_time);
    assert_eq!(
        fs::metadata(&input).unwrap().modified().unwrap(),
        folder_time
    );
    assert_eq!(
        names(&input),
        ["bark_set.ytd", "notes"],
        "nothing is added beside it"
    );

    // An entry of an archive: its bytes are handed in, and the archive is left as it was.
    let archive_bytes = RpfBuilder::default()
        .insert_with("x/bark_other.ytd", ytd("moss", 2), true)
        .unwrap()
        .to_bytes()
        .unwrap();
    let rpf = work.path().join("synthetic.rpf");
    write(&rpf, &archive_bytes);
    let rpf_time = age(&rpf);
    let archive = RpfArchive::open(&rpf).unwrap();
    let index = archive
        .entries
        .iter()
        .position(|e| !matches!(e, RpfEntry::Directory(_)))
        .expect("the archive holds the file");
    let entry = archive.read_entry(index as u32).unwrap();

    let report = run(
        &project,
        &[given(
            "bark_other.ytd",
            &entry,
            game("synthetic.rpf/x/bark_other.ytd"),
        )],
    );
    made(&report);
    assert_eq!(fs::read(&rpf).unwrap(), archive_bytes);
    assert_eq!(fs::metadata(&rpf).unwrap().modified().unwrap(), rpf_time);
}

#[test]
fn an_extraction_that_fails_while_it_writes_leaves_no_folder_and_no_staging() {
    let project = tempfile::tempdir().unwrap();
    // A name no file system holds as a file name: the extraction has written the manifest and the
    // rules when the first DDS fails.
    let long: &'static str = Box::leak("t".repeat(300).into_boxed_str());
    let bytes = ytd(long, 1);
    let report = run(project.path(), &[given("long.ytd", &bytes, loose())]);
    assert_eq!(only(&report).verdict, Verdict::Refused, "{report:#?}");
    assert!(!only(&report).reason.is_empty());
    assert!(report.created.is_empty());
    assert_eq!(names(&project.path().join("sources")), Vec::<String>::new());

    // Bytes that are no texture dictionary, and a protected asset, are refused and say why.
    let report = run(
        project.path(),
        &[
            given("junk.ytd", &[7u8; 64], loose()),
            given("sealed.ydr", b"FXAP\0\0\0\0", loose()),
        ],
    );
    assert_eq!(report.files.len(), 2);
    assert!(report.files.iter().all(|f| f.verdict == Verdict::Refused));
    assert!(
        report.files[0].reason.contains("texture dictionary"),
        "{report:#?}"
    );
    assert!(report.files[1].reason.contains("protected"), "{report:#?}");
    assert_eq!(names(&project.path().join("sources")), Vec::<String>::new());
}

#[test]
fn a_folder_is_walked_without_build_vfs_hidden_nested_sources_and_project_files() {
    let work = tempfile::tempdir().unwrap();
    let (project, input) = (work.path().join("project"), work.path().join("input"));
    let valid = |salt| ytd("tex", salt);
    write(&input.join("a.ytd"), &valid(1));
    write(&input.join("sub/b.ytd"), &valid(2));
    write(&input.join("sub/notes.md"), b"notes");
    write(&input.join("build/skipped_build.ytd"), &valid(3));
    write(&input.join("VFS/skipped_vfs.ytd"), &valid(4));
    write(&input.join(".hidden/skipped_hidden.ytd"), &valid(5));
    write(&input.join(".skipped_dot.ytd"), &valid(6));
    write(
        &input.join("nested.textures.src/skipped_nested.ytd"),
        &valid(7),
    );
    write(&input.join("mine.openrage-proj"), b"{}");

    let report = run(
        &project,
        &[Input::Path {
            path: &input,
            facts: loose(),
        }],
    );
    let seen: Vec<String> = report
        .files
        .iter()
        .map(|f| {
            Path::new(&f.input)
                .strip_prefix(&input)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(seen, ["a.ytd", "sub/b.ytd", "sub/notes.md"], "{report:#?}");
    assert!(report.left_out.is_empty(), "{report:#?}");
    assert_eq!(
        report.notes,
        ["Kept as compiled files, not sources: 1 .md"],
        "the walk hit no bound, and the report names the kind it kept"
    );
    assert_eq!(
        names(&project.join("sources")),
        ["a.textures.src", "b.textures.src"]
    );
    // A file with no source form is kept where it was found in the folder.
    assert_eq!(
        fs::read(project.join("files/sub/notes.md")).unwrap(),
        b"notes"
    );
    assert!(matches!(
        report.files[2].verdict,
        Verdict::Kept { copied_to: Some(_) }
    ));
}

#[test]
fn a_data_meta_is_kept_under_files_at_its_own_folder_path_not_made_a_stream_source() {
    let work = tempfile::tempdir().unwrap();
    let (project, input) = (work.path().join("project"), work.path().join("incoming"));
    write(&project.join("mine.openrage-proj"), b"{}");
    write(&input.join("data/handling.meta"), b"<CHandlingDataMgr/>");

    let report = run(
        &project,
        &[Input::Path {
            path: &input,
            facts: loose(),
        }],
    );
    assert!(
        matches!(only(&report).verdict, Verdict::Kept { copied_to: Some(_) }),
        "{report:#?}"
    );
    assert_eq!(
        fs::read(project.join("files/data/handling.meta")).unwrap(),
        b"<CHandlingDataMgr/>"
    );
    assert!(!project.join("sources").exists(), "{report:#?}");
}

#[test]
fn the_projects_own_folder_imported_again_writes_nothing_and_nests_no_files() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    write(&project.join("mine.openrage-proj"), b"{}");
    write(&project.join("files/hull.ybn"), b"hull");
    write(&project.join("loose.ybn"), b"loose");
    write(&project.join("a.ytd"), &ytd("tex", 1));

    let twice = |walked: &Path| {
        let input = Input::Path {
            path: walked,
            facts: loose(),
        };
        run(&project, std::slice::from_ref(&input));
        let before = tree(&project);
        let again = run(&project, std::slice::from_ref(&input));
        assert!(again.created.is_empty(), "{again:#?}");
        assert_eq!(tree(&project), before, "{again:#?}");
    };

    // The project's folder itself: its files/ is not game content.
    twice(&project);
    assert_eq!(names(&project.join("files")), ["hull.ybn", "loose.ybn"]);
    assert_eq!(names(&project.join("sources")), ["a.textures.src"]);

    // A folder above it: the project's files/ is skipped there too, and only the loose file is kept, under its path.
    twice(work.path());
    assert_eq!(
        names(&project.join("files")),
        ["hull.ybn", "loose.ybn", "project"]
    );
    assert_eq!(names(&project.join("files/project")), ["loose.ybn"]);
}

#[test]
fn a_bound_of_the_walk_leaves_files_out_and_says_what_it_left_out() {
    let work = tempfile::tempdir().unwrap();
    let (project, input) = (work.path().join("project"), work.path().join("input"));
    write(&input.join("a.ytd"), &ytd("tex", 1));
    write(&input.join("readme.txt"), b"r");
    write(&input.join("sub/b.ytd"), &ytd("tex", 2));
    write(&input.join("deep/l1/l2/c.ytd"), &ytd("tex", 3));

    let options = Options {
        bounds: Bounds { depth: 1, files: 2 },
        ..Options::default()
    };
    let report = import(
        &project,
        &[Input::Path {
            path: &input,
            facts: loose(),
        }],
        &options,
    );
    assert_eq!(report.files.len(), 2, "{report:#?}");
    let left: Vec<&str> = report.left_out.iter().map(|l| l.path.as_str()).collect();
    assert_eq!(left, ["deep/l1", "sub/b.ytd"], "{report:#?}");
    assert!(report.left_out.iter().all(|l| !l.why.is_empty()));
    assert!(
        report.notes.iter().any(|n| n.contains("not imported")),
        "{:?}",
        report.notes
    );
    assert!(!project.join("sources/b.textures.src").exists());
    assert!(!project.join("sources/c.textures.src").exists());
}

#[test]
fn a_source_folder_is_taken_as_what_it_is_and_not_walked_for_loose_files() {
    let project = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let source = made(&run(
        elsewhere.path(),
        &[given("a.ytd", &ytd("tex", 1), loose())],
    ));
    let report = run(
        project.path(),
        &[Input::Path {
            path: &source,
            facts: loose(),
        }],
    );
    assert_eq!(
        only(&report).verdict,
        Verdict::Made {
            folder: project
                .path()
                .join("sources")
                .join(source.file_name().unwrap())
        },
        "{report:#?}"
    );
    assert!(only(&report).reason.contains("as it is"), "{report:#?}");
    assert!(
        !project.path().join("files").exists(),
        "its DDS files are its own, not loose files"
    );
}

#[test]
fn a_file_with_no_source_form_is_kept_as_a_compiled_file_once_and_never_overwritten() {
    let project = tempfile::tempdir().unwrap();
    let files = project.path().join("files");
    let first = run(
        project.path(),
        &[given("collision.ycd", b"one", game("x/collision.ycd"))],
    );
    assert_eq!(
        only(&first).verdict,
        Verdict::Kept {
            copied_to: Some(files.join("collision.ycd"))
        }
    );
    assert!(only(&first).reason.contains("compiled file"), "{first:#?}");
    assert_eq!(fs::read(files.join("collision.ycd")).unwrap(), b"one");
    assert_eq!(first.created.len(), 1);
    assert!(
        !project.path().join("sources").exists(),
        "no source is made for it"
    );

    let before = tree(project.path());
    let again = run(project.path(), &[given("collision.ycd", b"one", loose())]);
    assert_eq!(
        only(&again).verdict,
        Verdict::Kept {
            copied_to: Some(files.join("collision.ycd"))
        }
    );
    assert!(again.created.is_empty());
    assert_eq!(tree(project.path()), before);

    let other = run(project.path(), &[given("collision.ycd", b"two", loose())]);
    assert_eq!(
        only(&other).verdict,
        Verdict::Kept {
            copied_to: Some(files.join("collision_2.ycd"))
        }
    );
    assert_eq!(fs::read(files.join("collision.ycd")).unwrap(), b"one");

    let not_copied = import(
        project.path(),
        &[given("elsewhere.ycd", b"x", loose())],
        &Options {
            keep: false,
            ..Options::default()
        },
    );
    assert_eq!(only(&not_copied).verdict, Verdict::Kept { copied_to: None });
    assert!(!files.join("elsewhere.ycd").exists());
}

#[test]
fn what_an_import_made_is_undone_while_it_is_untouched_and_not_otherwise() {
    let project = tempfile::tempdir().unwrap();
    let report = run(
        project.path(),
        &[
            given("a.ytd", &ytd("one", 1), loose()),
            given("b.ytd", &ytd("two", 2), loose()),
            given("c.ybn", b"c", loose()),
        ],
    );
    assert_eq!(report.created.len(), 3, "{report:#?}");
    assert!(report.created.iter().all(|c| c.untouched()));
    write(
        &report.created[0]
            .path
            .join("textures/added_by_a_person.dds"),
        b"dds",
    );
    assert!(!report.created[0].untouched());

    let undone: Vec<bool> = report.created.iter().map(|c| c.undo().unwrap()).collect();
    assert_eq!(undone, [false, true, true]);
    assert!(
        report.created[0].path.exists(),
        "a source somebody edited stays"
    );
    assert!(!report.created[1].path.exists() && !report.created[2].path.exists());
}

#[test]
fn editing_notes_leaves_a_recook_unchanged_and_editing_a_rule_does_not() {
    let project = tempfile::tempdir().unwrap();
    let dir = made(&run(
        project.path(),
        &[given("a.ytd", &ytd("tex", 1), loose())],
    ));
    let mut cooker = Cooker::new(project.path().join("build").join("shadow"));
    let manifest = dir.join("asset.json");
    assert!(matches!(cooker.recook(&manifest), Recook::Cooked { .. }));

    let mut source = Source::open(&dir).unwrap();
    source.manifest.notes = "the jacket for the harbor crew".to_string();
    source.write_manifest().unwrap();
    assert!(
        matches!(cooker.recook(&manifest), Recook::Unchanged { .. }),
        "a note is for a person; no compile reads it"
    );

    // A rule the DDS can honour: its colour space is a flag of the header. One it cannot, such as
    // another format, is refused and cooks nothing.
    edit_rules(&dir, |rules| {
        entry(rules, "tex")["encode"] = json!({ "colour_space": "srgb" });
    });
    assert!(matches!(
        cooker.recook(&dir.join("textures.json")),
        Recook::Cooked { .. }
    ));
}

#[test]
fn a_save_of_an_unchanged_source_changes_no_byte_and_a_save_makes_every_json_file_canonical() {
    let project = tempfile::tempdir().unwrap();
    let textures_dir = made(&run(
        project.path(),
        &[given("a.ytd", &ytd("tex", 1), loose())],
    ));
    let drawables_dir = made(&run(
        project.path(),
        &[given("d.ydd", &dictionary(), loose())],
    ));
    for dir in [&textures_dir, &drawables_dir] {
        let before = tree(dir);
        Source::open(dir).unwrap().save().unwrap();
        assert_eq!(tree(dir), before, "{}", dir.display());
    }

    // A person's editor writes the rules some other way; the next save writes them canonically.
    let rules = textures_dir.join("textures.json");
    let canonical = fs::read_to_string(&rules).unwrap();
    let value: Value = serde_json::from_str(&canonical).unwrap();
    fs::write(&rules, serde_json::to_string(&value).unwrap()).unwrap();
    let nested = fs::read_dir(drawables_dir.join("drawables"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("drawable.json");
    let nested_canonical = fs::read_to_string(&nested).unwrap();
    fs::write(
        &nested,
        serde_json::to_string(&serde_json::from_str::<Value>(&nested_canonical).unwrap()).unwrap(),
    )
    .unwrap();
    assert_ne!(fs::read_to_string(&rules).unwrap(), canonical);

    Source::open(&textures_dir).unwrap().save().unwrap();
    Source::open(&drawables_dir).unwrap().save().unwrap();
    assert_eq!(fs::read_to_string(&rules).unwrap(), canonical);
    assert_eq!(fs::read_to_string(&nested).unwrap(), nested_canonical);
    let settled = tree(&textures_dir);
    Source::open(&textures_dir).unwrap().save().unwrap();
    assert_eq!(tree(&textures_dir), settled);

    // A file that is no JSON stops the whole save: nothing else is written.
    fs::write(&rules, "{ not json").unwrap();
    let mut source = Source::open(&textures_dir).unwrap();
    let manifest_before = fs::read_to_string(textures_dir.join("asset.json")).unwrap();
    source.manifest.notes = "changed".to_string();
    let problem = source.save().unwrap_err();
    assert_eq!(problem.file.as_deref(), Some("textures.json"), "{problem}");
    assert_eq!(
        fs::read_to_string(textures_dir.join("asset.json")).unwrap(),
        manifest_before
    );
}

#[test]
fn a_copied_source_is_given_an_id_of_its_own_and_nothing_else_changes() {
    let project = tempfile::tempdir().unwrap();
    let dir = made(&run(
        project.path(),
        &[given("a.ytd", &ytd("tex", 1), loose())],
    ));
    let copy = project.path().join("copy of a.textures.src");
    copy_tree(&dir, &copy);

    let old = Source::open(&dir).unwrap().manifest;
    assert_eq!(Source::open(&copy).unwrap().manifest.id, old.id);
    let new = reid(&copy).unwrap();
    assert_ne!(new, old.id);
    let reread = Source::open(&copy).unwrap().manifest;
    assert_eq!(reread.id, new);
    assert_eq!(Source::open(&dir).unwrap().manifest.id, old.id);
    let mut expected = old;
    expected.id = new;
    assert_eq!(reread, expected);
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The lines between the markers of `text`.
fn block(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let begin = lines
        .iter()
        .position(|l| *l == git::BEGIN)
        .expect("a begin marker");
    let end = lines
        .iter()
        .position(|l| *l == git::END)
        .expect("an end marker");
    lines[begin + 1..end]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

#[test]
fn the_ignore_blocks_keep_game_content_out_and_a_persons_lines_in_and_are_idempotent() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    // A file a person wrote, with Windows line ends and no final newline.
    fs::write(root.join(".gitignore"), "target/\r\n*.log").unwrap();
    fs::write(
        root.join(".gitattributes"),
        "*.psd filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();

    let report = run(
        root,
        &[
            given("a.ytd", &ytd("one", 1), game("x64a.rpf/a.ytd")),
            given("b.ytd", &ytd("two", 2), loose()),
        ],
    );
    assert_eq!(report.created.len(), 2, "{report:#?}");
    let ignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(ignore.starts_with("target/\r\n*.log\r\n"), "{ignore:?}");
    assert!(
        !ignore.replace("\r\n", "").contains('\n'),
        "one kind of line end: {ignore:?}"
    );
    assert_eq!(
        block(&ignore),
        ["/build/", "/vfs/", "/sources/a.textures.src/"],
        "the game's source is ignored and the person's own is not"
    );
    let attributes = fs::read_to_string(root.join(".gitattributes")).unwrap();
    assert!(attributes.starts_with("*.psd filter=lfs diff=lfs merge=lfs -text\n"));
    let lines = block(&attributes);
    assert!(lines.iter().any(|l| l == "*.dds binary"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "*.glb binary"), "{lines:?}");
    // A base is hashed into `asset.json`, so a checkout that converted its line ends would not compile.
    for base in [
        "base.ydr",
        "base.ydd",
        "base.yft",
        "base.ymt",
        "base.ymap",
        "base.ytyp",
        "base.meta",
        "base.ybn",
        "base.ynd",
    ] {
        assert!(
            lines.iter().any(|l| *l == format!("{base} binary")),
            "{base}: {lines:?}"
        );
    }
    assert!(
        !lines.iter().any(|l| l.contains("lfs")),
        "LFS is the person's to ask for"
    );

    // Again: nothing to change, so no byte is.
    assert!(!git::update(root).unwrap());
    assert_eq!(fs::read_to_string(root.join(".gitignore")).unwrap(), ignore);
    assert_eq!(
        fs::read_to_string(root.join(".gitattributes")).unwrap(),
        attributes
    );

    // A line a person adds after the block stays, and one they add inside it goes.
    fs::write(
        root.join(".gitignore"),
        ignore.replace("/vfs/", "/vfs/\r\nnot-mine-to-keep") + "dist/\r\n",
    )
    .unwrap();
    assert!(git::update(root).unwrap());
    let fixed = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert_eq!(fixed, ignore.clone() + "dist/\r\n");

    // LFS, when asked for, is for the images and not for the DDS, and an import keeps it.
    assert!(git::write(root, true).unwrap());
    let with = block(&fs::read_to_string(root.join(".gitattributes")).unwrap());
    assert!(
        with.iter().any(|l| l.starts_with("*.png filter=lfs")),
        "{with:?}"
    );
    assert!(with.iter().any(|l| l == "*.dds binary"), "{with:?}");
    run(
        root,
        &[given("c.ytd", &ytd("three", 3), game("x64a.rpf/c.ytd"))],
    );
    let after = block(&fs::read_to_string(root.join(".gitattributes")).unwrap());
    assert_eq!(after, with, "an import does not undo the person's choice");
    assert!(block(&fs::read_to_string(root.join(".gitignore")).unwrap())
        .contains(&"/sources/c.textures.src/".to_string()));
    assert!(git::write(root, false).unwrap());
    assert!(
        !block(&fs::read_to_string(root.join(".gitattributes")).unwrap())
            .iter()
            .any(|l| l.contains("lfs"))
    );
}

#[test]
fn a_block_whose_end_marker_is_gone_is_refused_and_the_file_is_not_touched() {
    let project = tempfile::tempdir().unwrap();
    let damaged = format!("mine\n{}\n/build/\n", git::BEGIN);
    fs::write(project.path().join(".gitignore"), &damaged).unwrap();
    let why = git::write(project.path(), false).unwrap_err();
    assert!(why.to_string().contains(".gitignore"), "{why}");
    assert_eq!(
        fs::read_to_string(project.path().join(".gitignore")).unwrap(),
        damaged
    );
    assert!(
        !project.path().join(".gitattributes").exists(),
        "nothing is written when one file is refused"
    );
}

/// The `origin.cooked_sha256` an `asset.json` states, read as the text it is.
fn recorded_cook(folder: &Path) -> Option<String> {
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(folder.join("asset.json")).unwrap()).unwrap();
    manifest["origin"]["cooked_sha256"]
        .as_str()
        .map(str::to_string)
}

#[test]
fn a_game_import_records_what_its_own_first_cook_made_and_another_import_records_nothing() {
    let project = tempfile::tempdir().unwrap();
    let (a, b) = (
        as_the_game_ships_it(&ytd("bark", 1)),
        as_the_game_ships_it(&ytd("moss", 2)),
    );
    let report = run(
        project.path(),
        &[
            given("bark_set.ytd", &a, game("x64a.rpf/bark_set.ytd")),
            given("moss_set.ytd", &b, loose()),
        ],
    );
    let folder = |at: usize| match &report.files[at].verdict {
        Verdict::Made { folder } => folder.clone(),
        other => panic!("{other:?}: {report:#?}"),
    };

    let taken = folder(0);
    let cooked = compile(&taken);
    assert_ne!(cooked, a, "the stand-in cooks back to its own bytes");
    assert_eq!(
        recorded_cook(&taken),
        Some(sha256_hex(&cooked)),
        "the import did not record the digest of its own first cook"
    );
    assert_eq!(
        describe(&taken).unwrap().game,
        [sha256_hex(&a), sha256_hex(&cooked)],
        "a build compares a cook with the game's file and with the import's own"
    );

    let kept = folder(1);
    assert_eq!(recorded_cook(&kept), None, "{kept:?} is not the game's");
    assert!(describe(&kept).unwrap().game.is_empty());
}

/// The names of the files a report lists, below the folder `base` that was walked, with `/` between the parts.
fn seen_below(report: &Report, base: &Path) -> Vec<String> {
    report
        .files
        .iter()
        .map(|f| {
            Path::new(&f.input)
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn a_walk_leaves_an_archive_out_and_says_so_instead_of_keeping_it_whole() {
    let work = tempfile::tempdir().unwrap();
    let (project, input) = (work.path().join("project"), work.path().join("input"));
    write(&input.join("a.ytd"), &ytd("tex", 1));
    write(&input.join("pack/Models.RPF"), b"an archive of gigabytes");
    write(&input.join("pack/lamp.ybn"), b"hull");

    let report = run(
        &project,
        &[Input::Path {
            path: &input,
            facts: the_games(),
        }],
    );

    assert_eq!(
        seen_below(&report, &input),
        ["a.ytd", "pack/lamp.ybn"],
        "{report:#?}"
    );
    assert!(
        !project.join("files/pack/Models.RPF").exists(),
        "the archive was copied whole: {:?}",
        names(&project.join("files/pack"))
    );
    let left: Vec<&str> = report.left_out.iter().map(|l| l.path.as_str()).collect();
    assert_eq!(left, ["pack/Models.RPF"], "{report:#?}");
    assert!(
        report.left_out[0].why.contains("archive") && report.left_out[0].why.contains("open it"),
        "{report:#?}"
    );
    assert!(
        report.notes.iter().any(|n| n.contains("archive")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_walk_names_its_bounds_in_the_report_and_one_file_alone_does_not() {
    let work = tempfile::tempdir().unwrap();
    let (project, input) = (work.path().join("project"), work.path().join("input"));
    write(&input.join("a.ytd"), &ytd("tex", 1));

    let walked = run(
        &project,
        &[Input::Path {
            path: &input,
            facts: loose(),
        }],
    );
    assert_eq!(walked.bounds, Some(Options::default().bounds));
    assert!(walked.left_out.is_empty() && walked.notes.is_empty());

    let alone = run(
        &project,
        &[Input::Path {
            path: &input.join("a.ytd"),
            facts: loose(),
        }],
    );
    assert_eq!(alone.bounds, None, "{alone:#?}");
}

#[test]
fn a_cancelled_import_keeps_the_files_it_made_and_lists_the_ones_it_did_not_reach() {
    let work = tempfile::tempdir().unwrap();
    let (project, input) = (work.path().join("project"), work.path().join("input"));
    for (n, name) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
        write(
            &input.join(format!("{name}.ytd")),
            &ytd("tex", n as u32 + 1),
        );
    }
    let inputs = [Input::Path {
        path: &input,
        facts: loose(),
    }];

    // Asked before each file, it says to go on twice and then to stop.
    let asked = std::cell::Cell::new(0);
    let cancelled = || {
        asked.set(asked.get() + 1);
        asked.get() > 2
    };
    let said = std::cell::RefCell::new(Vec::new());
    let progress = |done: usize, total: usize, name: &str| {
        said.borrow_mut().push((done, total, name.to_string()));
    };
    let report = import_watched(
        &project,
        &inputs,
        &Options::default(),
        &Watch {
            cancelled: Some(&cancelled),
            progress: Some(&progress),
        },
    );

    assert!(report.cancelled, "{report:#?}");
    assert_eq!(seen_below(&report, &input), ["a.ytd", "b.ytd"]);
    assert!(report
        .files
        .iter()
        .all(|f| matches!(f.verdict, Verdict::Made { .. })));
    assert_eq!(
        names(&project.join("sources")),
        ["a.textures.src", "b.textures.src"],
        "what was made stays"
    );
    assert_eq!(report.created.len(), 2, "Undo can take back what was made");
    let left: Vec<&str> = report.left_out.iter().map(|l| l.path.as_str()).collect();
    assert_eq!(left, ["c.ytd", "d.ytd", "e.ytd"], "{report:#?}");
    assert!(
        report.notes.iter().any(|n| n.contains("2 of 5")),
        "{:?}",
        report.notes
    );
    assert_eq!(
        *said.borrow(),
        [(0, 5, "a.ytd".to_string()), (1, 5, "b.ytd".to_string())],
        "progress is told before each file, with the total found before the first"
    );

    // The same import run to the end adds the rest and says the first two are sources already.
    let rest = run(&project, &inputs);
    assert!(!rest.cancelled && rest.left_out.is_empty());
    let verdicts: Vec<bool> = rest
        .files
        .iter()
        .map(|f| matches!(f.verdict, Verdict::AlreadyASource { .. }))
        .collect();
    assert_eq!(verdicts, [true, true, false, false, false], "{rest:#?}");
}

#[test]
fn an_import_cancelled_before_its_first_file_makes_nothing_and_lists_every_file() {
    let work = tempfile::tempdir().unwrap();
    let (project, input) = (work.path().join("project"), work.path().join("input"));
    write(&input.join("a.ytd"), &ytd("tex", 1));
    write(&input.join("notes.md"), b"notes");

    let report = import_watched(
        &project,
        &[Input::Path {
            path: &input,
            facts: loose(),
        }],
        &Options::default(),
        &Watch {
            cancelled: Some(&|| true),
            progress: None,
        },
    );

    assert!(report.cancelled && report.files.is_empty() && report.created.is_empty());
    assert_eq!(report.left_out.len(), 2, "{report:#?}");
    assert!(!project.join("sources").exists() && !project.join("files").exists());
}

/// The bones a glb states for its skin, by the source folder the door made.
fn skin_of(folder: &Path) -> (usize, usize) {
    let scene = or_interchange::gltf::read::load(
        Path::new("model.glb"),
        &fs::read(folder.join("model.glb")).unwrap(),
    )
    .unwrap();
    (
        scene.objects[0].bones.len(),
        scene.primitives().filter(|p| !p.joints.is_empty()).count(),
    )
}

#[test]
fn a_ped_component_imported_with_its_reference_skeleton_has_its_skin_and_one_it_does_not_fit_still_comes_in(
) {
    use model_fixtures::reference;
    let root = tempfile::tempdir().unwrap();
    // The door skips a file it has made a source of already, so each import is of other bytes.
    let component = |lift: f32| ydr(drawable("jbib_000_u", vec![geometry(6, true, lift)], None));
    let made = |facts: Facts, name: &str, lift: f32| {
        let bytes = component(lift);
        let report = import(
            root.path(),
            &[Input::Bytes {
                name,
                bytes: &bytes,
                facts,
            }],
            &Options::default(),
        );
        let Verdict::Made { folder } = &report.files[0].verdict else {
            panic!("{:?}", report.files);
        };
        folder.clone()
    };

    // With nothing to bind to, a ped component's glb states no skin: its weights are the base's.
    let bare = made(loose(), "jbib_000_u.ydr", 0.0);
    assert_eq!(skin_of(&bare), (0, 0));

    // With the skeleton of the ped, the glb has the bones and the weights of every primitive.
    let skeleton = reference(8);
    let facts = Facts {
        skeleton: or_source::game_import::Skeleton::of(skeleton.clone()),
        ..loose()
    };
    let skinned = made(facts, "jbib_001_u.ydr", 3.0);
    assert_eq!(skin_of(&skinned), (8, 1));
    let source = Source::open(&skinned).unwrap();
    assert_eq!(
        source.manifest.origin.extra["reference_skeleton"],
        json!("embedded")
    );
    assert!(
        models::compile(&source).is_ok(),
        "the source still compiles"
    );

    // A skeleton the palette outgrows does not refuse the file: it comes in as it did, and the source says why.
    let too_small = Facts {
        skeleton: or_source::game_import::Skeleton::of(reference(2)),
        ..loose()
    };
    let unfit = made(too_small, "jbib_002_u.ydr", 6.0);
    assert_eq!(skin_of(&unfit), (0, 0));
    let said = Source::open(&unfit).unwrap().manifest.origin.extra["reference_skeleton"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(said.starts_with("not embedded: "), "{said}");
}
