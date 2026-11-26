//! `openrage-cli project check-resource` reads a resource folder that no project holds and answers with its exit code: 0 for a sound
//! one and for one with only warnings and notes, 1 where a rule finds an error, 2 for a command line it cannot read.
//! What it prints is the report's lines, and with `--json` the same report as data.

use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode};

use or_io::decl::{File, Gen8};
use or_io::gta5::{GtaDrawable, PgDictionary};
use or_maps::sample::{map_file, types_file, Archetype, Entity};

fn check(args: &[&str]) -> ExitCode {
    let mut all = vec!["project".to_string(), "check-resource".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
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
    let code = check(&args);
    let _ = out.flush();
    let _ = write!(out, "{END}{}", u8::from(code == ExitCode::SUCCESS));
    let _ = out.flush();
}

/// Whether `project check-resource <args>` exited zero, and what it printed.
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

fn put(folder: &Path, rel: &str, bytes: &[u8]) {
    let at = folder.join(rel);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, bytes).unwrap();
}

/// A resource of one map, the type file that declares what it places and the model of that archetype, with a manifest that lists one file.
fn a_sound_resource(folder: &Path) {
    put(
        folder,
        "fxmanifest.lua",
        b"fx_version 'cerulean'\ngame 'gta5'\nthis_is_a_map 'yes'\nfiles { 'data/notes.meta' }\n\
          data_file 'DLC_ITYP_REQUEST' 'stream/harbour_types.ytyp'\n",
    );
    put(folder, "data/notes.meta", b"<CExtraData/>\n");
    let map = map_file(
        "harbour",
        &[Entity::at("harbour_crate", [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    );
    put(folder, "stream/harbour.ymap", &map);
    let types = types_file(
        "harbour_types",
        &[Archetype::new("harbour_crate", [-1.0; 3], [1.0; 3])],
    );
    put(folder, "stream/harbour_types.ytyp", &types);
    let model = PgDictionary::<GtaDrawable<Gen8>>::default()
        .to_bytes()
        .expect("an empty dictionary saves");
    put(folder, "stream/harbour_crate.ydd", &model);
}

#[test]
fn a_sound_resource_exits_zero_and_an_error_exits_one_as_text_and_as_json() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("harbour");
    a_sound_resource(&folder);
    let report = or_cli::check_resource_folder(&folder, None, None, Default::default())
        .expect("a folder is read");
    assert_eq!(report.findings, [], "the resource built here is sound");
    assert_eq!(check(&[text(&folder)]), ExitCode::SUCCESS);
    assert_eq!(check(&[text(&folder), "--json"]), ExitCode::SUCCESS);

    // A warning and a note do not fail the check.
    put(&folder, "stream/readme.txt", b"how to install\n");
    std::fs::remove_file(folder.join("stream/harbour_crate.ydd")).unwrap();
    assert_eq!(check(&[text(&folder)]), ExitCode::SUCCESS);
    assert_eq!(
        check(&[text(&folder), "--limits", "file_memory=1"]),
        ExitCode::SUCCESS
    );

    std::fs::remove_file(folder.join("data/notes.meta")).unwrap();
    assert_eq!(check(&[text(&folder)]), ExitCode::from(1));
    assert_eq!(check(&[text(&folder), "--json"]), ExitCode::from(1));
}

#[test]
fn a_command_line_that_cannot_be_read_is_a_usage_error_and_a_path_that_is_no_folder_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("harbour");
    a_sound_resource(&folder);
    assert_eq!(check(&[]), ExitCode::from(2));
    assert_eq!(check(&[text(&folder), text(&folder)]), ExitCode::from(2));
    assert_eq!(check(&[text(&folder), "--strict"]), ExitCode::from(2));
    assert_eq!(
        check(&[text(&folder), "--limits", "file_memory"]),
        ExitCode::from(2)
    );
    assert_eq!(check(&[text(&folder), "--game"]), ExitCode::from(2));
    assert_eq!(
        check(&[text(&folder.join("fxmanifest.lua"))]),
        ExitCode::FAILURE
    );
    assert_eq!(check(&[text(&dir.path().join("gone"))]), ExitCode::FAILURE);
    // A file is refused before anything is read of it, not reported as a folder with no manifest.
    for no_folder in [folder.join("fxmanifest.lua"), dir.path().join("gone")] {
        let refused = or_cli::check_resource_folder(&no_folder, None, None, Default::default());
        assert!(
            refused.is_err_and(|why| why.contains("is not a folder")),
            "{}",
            no_folder.display()
        );
    }
}

#[test]
fn what_is_printed_is_the_report_as_text_and_with_the_flag_the_same_report_as_data() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("harbour");
    a_sound_resource(&folder);
    // An error, a warning and a note.
    std::fs::remove_file(folder.join("data/notes.meta")).unwrap();
    put(&folder, "stream/readme.txt", b"how to install\n");
    std::fs::remove_file(folder.join("stream/harbour_crate.ydd")).unwrap();
    let report = or_cli::check_resource_folder(&folder, None, None, Default::default())
        .expect("a folder is read");
    assert_eq!(report.findings.len(), 3, "{:#?}", report.findings);

    let (passed, as_text) = printed(&[text(&folder)]);
    assert!(!passed);
    assert_eq!(as_text.lines().collect::<Vec<_>>(), report.lines());

    let (passed, as_json) = printed(&[text(&folder), "--json"]);
    assert!(!passed);
    let documents: serde_json::Value =
        serde_json::from_str(&as_json).unwrap_or_else(|e| panic!("{e}: {as_json}"));
    let stated = serde_json::to_value(&report).expect("a report is data");
    assert_eq!(documents, serde_json::Value::Array(vec![stated]));
    let findings = documents[0]["findings"].as_array().expect("a list");
    assert_eq!(findings.len(), report.findings.len());
    for finding in findings {
        let (code, message) = (finding["code"].as_str(), finding["message"].as_str());
        assert!(
            as_text
                .lines()
                .any(|line| line.contains(code.expect("a code"))
                    && line.contains(message.expect("a sentence"))),
            "the text lacks {finding}"
        );
    }

    // The ceiling the flag states reaches the check, and no file is weighed without it.
    assert!(!as_text.contains("file-oversized"), "{as_text}");
    let (_, weighed) = printed(&[text(&folder), "--limits", "file_memory=1"]);
    assert!(weighed.contains("file-oversized"), "{weighed}");
}

#[test]
fn an_install_that_is_asked_and_does_not_mount_fails_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("harbour");
    a_sound_resource(&folder);
    let gone = dir.path().join("no install here");
    assert_eq!(check(&[text(&folder)]), ExitCode::SUCCESS);
    assert_eq!(
        check(&[text(&folder), "--game", text(&gone)]),
        ExitCode::FAILURE
    );
    assert_eq!(
        check(&[text(&folder), "--limits", "file_memroy=1"]),
        ExitCode::from(2),
        "a figure the check does not weigh is not taken in silence"
    );
}
