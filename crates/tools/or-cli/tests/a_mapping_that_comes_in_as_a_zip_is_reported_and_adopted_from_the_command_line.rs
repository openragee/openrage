//! `openrage-cli project adopt <mapping.zip> --into <folder>` prints what a mapping holds before anything is written, as text and as
//! the same report in JSON, adopts it into a project that plans and builds with its locked file as it came, refuses an archive that is
//! not safe to unpack by the entry and writes nothing, and answers with its exit code: 0, 1 for a refusal and for a place with nothing
//! to adopt, 2 for a command line it cannot read.

use std::io::{Cursor, Write};
use std::path::Path;
use std::process::{Command, ExitCode};

use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_project::resource::ESCROW_MARKER;
use or_source::sample::drawable_dictionary;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn run(words: &[&str]) -> ExitCode {
    let all: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    or_cli::run(&all)
}

fn adopt(args: &[&str]) -> ExitCode {
    run(&[&["project", "adopt"], args].concat())
}

const CHILD_TEST: &str = "the_child_that_runs_one_command";
const CHILD_ENV: &str = "OPENRAGE_TEST_ONE_COMMAND";
const BEGIN: &str = "<<begin>>";
const END: &str = "<<end>>";
const SEPARATOR: char = '\u{1f}';

/// Run by `printed` in a child process, so that what the command prints can be read.
#[test]
fn the_child_that_runs_one_command() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args = args.to_string_lossy().into_owned();
    let args: Vec<&str> = args.split(SEPARATOR).collect();
    let mut out = std::io::stdout();
    let _ = write!(out, "{BEGIN}");
    let _ = out.flush();
    let code = adopt(&args);
    let _ = out.flush();
    let _ = write!(out, "{END}{}", u8::from(code == ExitCode::SUCCESS));
    let _ = out.flush();
}

/// Whether `project adopt <args>` exited zero, and what it printed.
fn printed(args: &[&str]) -> (bool, String) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("{args:?}: no start marker: {text}"));
    let (printed, tail) = rest.split_once(END).unwrap_or((rest, ""));
    (tail.starts_with('1'), printed.to_string())
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

fn zip_of(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

const CRATE: &str = "harbour_crate";

fn locked() -> Vec<u8> {
    [
        ESCROW_MARKER.as_slice(),
        b"what the escrow wrote, which no reader opens",
    ]
    .concat()
}

/// A resource of one map, its type file and its model, with a locked model, a script and a manifest that names the script.
fn a_mapping() -> Vec<(&'static str, Vec<u8>)> {
    let manifest = "fx_version 'cerulean'\ngame 'gta5'\nthis_is_a_map 'yes'\nversion '1.2.0'\n\
                    data_file 'DLC_ITYP_REQUEST' 'stream/harbour_types.ytyp'\nclient_script 'client.lua'\n";
    let map = map_file(
        "harbour",
        &[
            Entity::at(CRATE, [10.0, 20.0, 0.5], 1),
            Entity::at("prop_ghost", [12.0, 20.0, 0.5], 2),
        ],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    );
    let types = types_file(
        "harbour_types",
        &[Archetype::new(CRATE, [-1.0; 3], [1.0; 3])],
    );
    vec![
        ("harbour/fxmanifest.lua", manifest.as_bytes().to_vec()),
        ("harbour/client.lua", b"print('harbour')".to_vec()),
        ("harbour/stream/harbour.ymap", map),
        ("harbour/stream/harbour_types.ytyp", types),
        (
            "harbour/stream/harbour_crate.ydd",
            drawable_dictionary(CRATE, 6, 0.0),
        ),
        ("harbour/stream/sealed.ydr", locked()),
    ]
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = (std::fs::read_dir(dir).unwrap().flatten())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_dry_run_prints_the_report_as_text_and_the_same_report_as_json_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("harbour.zip");
    std::fs::write(&archive, zip_of(&a_mapping())).unwrap();
    let into = dir.path().join("project");

    let (ok, said) = printed(&[text(&archive), "--dry-run", "--into", text(&into)]);
    assert!(ok, "{said}");
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(
        lines[0],
        "mapping harbour.zip: 1 resource(s), 0 loose stream folder(s)"
    );
    assert!(
        lines[1].starts_with("resource harbour (fxmanifest.lua): 6 file(s): 3 become source(s), 2 kept whole, 1 locked"),
        "{}",
        lines[1]
    );

    let (ok, json) = printed(&[text(&archive), "--dry-run", "--json"]);
    assert!(ok, "{json}");
    let documents: serde_json::Value = serde_json::from_str(&json).expect("the JSON parses");
    let document = &documents[0];
    assert_eq!(document["format"], "openrage-mapping-report");
    assert_eq!(document["from"], "harbour.zip");
    let root = &document["roots"][0];

    // Every line of the text but the two that count is one entry of the JSON, and every entry is a line.
    let mut matched = vec![false; lines.len()];
    (matched[0], matched[1]) = (true, true);
    let mut claim = |begins: String, holds: &str| {
        let at = (lines.iter())
            .position(|line| line.trim_start().starts_with(&begins) && line.contains(holds))
            .unwrap_or_else(|| panic!("no line begins {begins:?} and holds {holds:?}:\n{said}"));
        assert!(!matched[at], "{begins:?} is said twice");
        matched[at] = true;
    };
    for file in root["files"].as_array().unwrap() {
        let word = match file["becomes"]["as"].as_str().unwrap() {
            "source" => format!("source ({})", file["becomes"]["kind"].as_str().unwrap()),
            "kept" => "kept whole".to_string(),
            other => other.replace('-', " "),
        };
        claim(
            format!("{word} {}", file["path"].as_str().unwrap()),
            file["why"].as_str().unwrap(),
        );
    }
    for line in root["lines"].as_array().unwrap() {
        let carried = if line["carried"].as_bool().unwrap() {
            ": carried: "
        } else {
            ": not carried: "
        };
        claim(
            format!(
                "manifest line {} {} ",
                line["line"],
                line["key"].as_str().unwrap()
            ),
            &format!("{carried}{}", line["how"].as_str().unwrap()),
        );
    }
    for finding in root["findings"].as_array().unwrap() {
        claim(
            format!("note [{}]", finding["code"].as_str().unwrap()),
            finding["message"].as_str().unwrap(),
        );
    }
    assert!(
        matched.iter().all(|m| *m),
        "a line of the text is in no entry of the JSON:\n{said}"
    );
    assert_eq!(root["files"].as_array().unwrap().len(), 6);
    assert_eq!(root["locked"], 1);
    let codes: Vec<&str> = (root["findings"].as_array().unwrap().iter())
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        ["archetype-undeclared"],
        "the map places an archetype nothing declares"
    );

    assert_eq!(
        names_in(dir.path()),
        ["harbour.zip"],
        "a dry run writes nothing"
    );
}

#[test]
fn a_mapping_is_adopted_into_a_project_that_plans_and_builds_with_its_locked_file_as_it_came() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("harbour.zip");
    std::fs::write(&archive, zip_of(&a_mapping())).unwrap();
    let into = dir.path().join("harbour");

    let (ok, said) = printed(&[text(&archive), "--into", text(&into)]);
    assert!(ok, "{said}");
    assert!(
        said.lines()
            .last()
            .is_some_and(|last| last.starts_with("created ")
                && last.contains("3 source(s) made, 2 file(s) kept whole")),
        "{said}"
    );
    assert_eq!(
        names_in(&into),
        [
            "client.lua",
            "fxmanifest.lua",
            "harbour.openrage-proj",
            "maps",
            "stream"
        ],
        "no staging folder is left"
    );
    assert_eq!(names_in(&into.join("stream")), ["sealed.ydr"]);
    let project = into.join("harbour.openrage-proj");

    // A second adoption into the folder is refused: it is not empty any more.
    assert_eq!(
        adopt(&[text(&archive), "--into", text(&into)]),
        ExitCode::FAILURE
    );

    assert_eq!(run(&["project", "plan", text(&project)]), ExitCode::SUCCESS);
    assert_eq!(
        run(&["project", "build", text(&project)]),
        ExitCode::SUCCESS
    );
    let built = into.join("build").join("harbour");
    assert_eq!(
        names_in(&built.join("stream")),
        [
            "harbour.ymap",
            "harbour_crate.ydd",
            "harbour_types.ytyp",
            "sealed.ydr"
        ]
    );
    assert_eq!(
        std::fs::read(built.join("stream/sealed.ydr")).unwrap(),
        locked(),
        "a locked file survives a build byte for byte"
    );
    let manifest = std::fs::read_to_string(built.join("fxmanifest.lua")).unwrap();
    for stated in [
        "this_is_a_map 'yes'",
        "version '1.2.0'",
        "DLC_ITYP_REQUEST",
        "client.lua",
    ] {
        assert!(manifest.contains(stated), "{stated}: {manifest}");
    }
    assert!(
        !manifest.contains("client_script"),
        "the line the report said is not carried is not: {manifest}"
    );
}

#[test]
fn an_archive_that_is_not_safe_to_unpack_is_refused_by_the_entry_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("evil.zip");
    let mut files = a_mapping();
    files.push(("harbour/../../outside.lua", b"print('out')".to_vec()));
    std::fs::write(&archive, zip_of(&files)).unwrap();
    let into = dir.path().join("project");

    assert_eq!(
        adopt(&[text(&archive), "--into", text(&into)]),
        ExitCode::FAILURE
    );
    assert_eq!(adopt(&[text(&archive), "--dry-run"]), ExitCode::FAILURE);
    let (ok, json) = printed(&[text(&archive), "--into", text(&into), "--json"]);
    assert!(!ok);
    let documents: serde_json::Value = serde_json::from_str(&json).expect("the JSON parses");
    let refused = &documents[0]["refused"];
    assert_eq!(refused["code"], "climbs");
    assert_eq!(refused["entry"], "harbour/../../outside.lua");
    assert!(
        documents[0].get("roots").is_none(),
        "nothing of a refused archive is reported"
    );
    let (ok, said) = printed(&[text(&archive), "--into", text(&into)]);
    assert!(
        !ok && said.is_empty(),
        "the refusal is the error, and no report is printed: {said}"
    );

    // A limit is an argument: the same mapping is refused once fewer entries are taken than it states.
    let sound = dir.path().join("sound.zip");
    std::fs::write(&sound, zip_of(&a_mapping())).unwrap();
    assert_eq!(
        adopt(&[text(&sound), "--dry-run", "--limits", "entries=6"]),
        ExitCode::SUCCESS
    );
    assert_eq!(
        adopt(&[text(&sound), "--dry-run", "--limits", "entries=5"]),
        ExitCode::FAILURE
    );
    assert_eq!(
        adopt(&[text(&sound), "--into", text(&into), "--limits", "entries=5"]),
        ExitCode::FAILURE
    );

    assert_eq!(
        names_in(dir.path()),
        ["evil.zip", "sound.zip"],
        "nothing is written"
    );
    assert!(!dir.path().parent().unwrap().join("outside.lua").exists());
}

#[test]
fn a_place_with_nothing_to_adopt_exits_one_and_a_command_line_that_cannot_be_read_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("loose.zip");
    std::fs::write(
        &archive,
        zip_of(&[
            ("harbour.ymap", b"a map".to_vec()),
            ("readme.txt", b"hello".to_vec()),
        ]),
    )
    .unwrap();
    let into = dir.path().join("project");

    let (ok, said) = printed(&[text(&archive), "--dry-run"]);
    assert!(!ok);
    assert!(said.contains("outside harbour.ymap: a map file"), "{said}");
    assert!(
        said.contains("no resource and no stream folder, so there is nothing to adopt"),
        "{said}"
    );
    assert_eq!(
        adopt(&[text(&archive), "--into", text(&into)]),
        ExitCode::FAILURE
    );
    assert!(!into.exists());

    assert_eq!(
        adopt(&[text(&archive)]),
        ExitCode::from(2),
        "an adoption needs --into"
    );
    assert_eq!(
        adopt(&[text(&archive), "--dry-run", "--strict"]),
        ExitCode::from(2)
    );
    assert_eq!(
        adopt(&[text(&archive), "--dry-run", "--limits", "files=3"]),
        ExitCode::from(2)
    );
    assert_eq!(
        adopt(&[text(&archive), text(&archive), "--dry-run"]),
        ExitCode::from(2)
    );
    assert_eq!(
        adopt(&[text(&archive), "--into", text(&dir.path().join("a b?"))]),
        ExitCode::from(2)
    );
    assert_eq!(
        adopt(&[text(&dir.path().join("gone.zip")), "--dry-run"]),
        ExitCode::FAILURE
    );
    assert_eq!(names_in(dir.path()), ["loose.zip"]);
}

#[test]
fn a_dry_run_says_what_would_stop_the_adoption_it_is_asked_about_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("harbour.zip");
    std::fs::write(&archive, zip_of(&a_mapping())).unwrap();
    let held = dir.path().join("held");
    std::fs::create_dir_all(&held).unwrap();
    std::fs::write(held.join("mine.txt"), b"mine").unwrap();
    let new = dir.path().join("new");

    // A folder that holds something: the report is printed whole, and the answer is the one the adoption would give.
    let (ok, said) = printed(&[text(&archive), "--dry-run", "--into", text(&held)]);
    assert!(!ok, "{said}");
    assert!(
        said.starts_with("mapping harbour.zip: 1 resource(s)"),
        "{said}"
    );
    assert_eq!(
        adopt(&[text(&archive), "--into", text(&held)]),
        ExitCode::FAILURE
    );

    // A root the mapping does not hold, with a folder named and with none.
    for args in [
        vec![text(&archive), "--dry-run", "--root", "north"],
        vec![
            text(&archive),
            "--dry-run",
            "--root",
            "north",
            "--into",
            text(&new),
        ],
    ] {
        assert_eq!(adopt(&args), ExitCode::FAILURE, "{args:?}");
    }
    assert_eq!(
        adopt(&[
            text(&archive),
            "--dry-run",
            "--root",
            "harbour",
            "--into",
            text(&new)
        ]),
        ExitCode::SUCCESS
    );

    // A path the listing holds and the folder it would be unpacked into does not, as the document states a refusal.
    let longest = "harbour/stream/harbour_types.ytyp";
    let limit = format!("path={}", longest.len());
    let (ok, json) = printed(&[
        text(&archive),
        "--dry-run",
        "--json",
        "--into",
        text(&new),
        "--limits",
        &limit,
    ]);
    assert!(!ok, "{json}");
    let documents: serde_json::Value = serde_json::from_str(&json).expect("the JSON parses");
    let refused = &documents[0]["adopted"]["refused"];
    assert_eq!(refused["code"], "path-too-long", "{json}");
    assert!(refused["entry"].as_str().is_some(), "{json}");
    assert_eq!(
        documents[0]["roots"][0]["files"].as_array().map(Vec::len),
        Some(6),
        "the report is whole beside the refusal"
    );
    assert_eq!(
        adopt(&[text(&archive), "--into", text(&new), "--limits", &limit]),
        ExitCode::FAILURE,
        "the adoption is refused as the dry run said"
    );

    // A folder whose name is no project name is a command line the adoption cannot read, and the dry run answers the same.
    let unnamed = dir.path().join("a b?");
    assert_eq!(
        adopt(&[text(&archive), "--dry-run", "--into", text(&unnamed)]),
        ExitCode::from(2)
    );
    assert_eq!(
        adopt(&[
            text(&archive),
            "--dry-run",
            "--into",
            text(&unnamed),
            "--name",
            "harbour"
        ]),
        ExitCode::SUCCESS
    );

    assert_eq!(names_in(dir.path()), ["harbour.zip", "held"]);
    assert_eq!(names_in(&held), ["mine.txt"]);
}

#[test]
fn a_model_no_reader_opens_is_kept_whole_and_the_build_stops_at_it_as_the_adoption_said() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("harbour.zip");
    let files = vec![(
        "stream/harbour_crate.ydr",
        b"no drawable a reader opens".to_vec(),
    )];
    std::fs::write(&archive, zip_of(&files)).unwrap();
    let into = dir.path().join("harbour");

    let (ok, said) = printed(&[text(&archive), "--into", text(&into)]);
    assert!(ok, "{said}");
    let adopted = (said.lines())
        .find(|line| line.starts_with("adopted as kept whole stream/harbour_crate.ydr"))
        .unwrap_or_else(|| panic!("the adoption says what it made of the model: {said}"));
    assert!(
        adopted.contains("stops at it by name if it cannot read it back"),
        "{adopted}"
    );
    assert_eq!(
        std::fs::read(into.join("stream/harbour_crate.ydr")).unwrap(),
        files[0].1
    );

    // The sentence is true of this file: the build reads back what it wrote, and does not swap in what it cannot read.
    let project = into.join("harbour.openrage-proj");
    assert_eq!(run(&["project", "plan", text(&project)]), ExitCode::SUCCESS);
    assert_eq!(
        run(&["project", "build", text(&project)]),
        ExitCode::FAILURE
    );
    assert!(!into.join("build").join("harbour").exists());
}
