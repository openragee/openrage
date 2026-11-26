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
    for note in document["notes"].as_array().unwrap() {
        claim("note: ".to_string(), note.as_str().unwrap());
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

/// What `project adopt --dry-run` prints of the mapping `files` in a zip, and, once it is adopted and built, the folder of the
/// project, the files of the built resource and the manifest the build wrote.
fn adopted_and_built(
    files: &[(&str, Vec<u8>)],
) -> (String, tempfile::TempDir, Vec<String>, String) {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("harbour.zip");
    std::fs::write(&archive, zip_of(files)).unwrap();
    let into = dir.path().join("harbour");
    let (ok, report) = printed(&[text(&archive), "--into", text(&into), "--dry-run"]);
    assert!(ok, "{report}");
    assert_eq!(
        adopt(&[text(&archive), "--into", text(&into)]),
        ExitCode::SUCCESS
    );
    let project = into.join("harbour.openrage-proj");
    assert_eq!(
        run(&["project", "plan", text(&project)]),
        ExitCode::SUCCESS,
        "{report}"
    );
    assert_eq!(
        run(&["project", "build", text(&project)]),
        ExitCode::SUCCESS,
        "{report}"
    );
    let built = into.join("build").join("harbour");
    let manifest = std::fs::read_to_string(built.join("fxmanifest.lua")).unwrap();
    (report, dir, names_under(&built), manifest)
}

/// Every file under `dir` by its path with `/`, sorted.
fn names_under(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let inside = path.strip_prefix(dir).unwrap().to_string_lossy();
                found.push(inside.replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

fn one_map() -> Vec<u8> {
    map_file(
        "harbour",
        &[Entity::at(CRATE, [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    )
}

fn crate_types(name: &str) -> Vec<u8> {
    types_file(name, &[Archetype::new(CRATE, [-1.0; 3], [1.0; 3])])
}

const HEAD: &str = "fx_version 'cerulean'\ngame 'gta5'\nthis_is_a_map 'yes'\n";

/// ⚠ A build lists every file it ships outside the stream folder under `files`, which a server sends to every client, and writes no
/// script line: a script of the server is kept beside the project, where no build ships it.
#[test]
fn a_script_of_the_server_is_kept_beside_the_project_and_no_build_publishes_it() {
    let manifest = format!(
        "{HEAD}server_script 'server/sv_doors.lua'\nserver_scripts {{ 'server/more/*.lua' }}\n\
         shared_script 'config.lua'\nclient_script 'client.lua'\n"
    );
    let (report, dir, built, written) = adopted_and_built(&[
        ("harbour/fxmanifest.lua", manifest.into_bytes()),
        ("harbour/stream/harbour.ymap", one_map()),
        (
            "harbour/server/sv_doors.lua",
            b"local KEY = 'secret'".to_vec(),
        ),
        (
            "harbour/server/more/sv_hooks.lua",
            b"local HOOK = 'secret'".to_vec(),
        ),
        ("harbour/config.lua", b"Config = {}".to_vec()),
        ("harbour/client.lua", b"print('c')".to_vec()),
    ]);
    for script in ["server/sv_doors.lua", "server/more/sv_hooks.lua"] {
        assert!(
            report.contains(&format!(
                "kept beside {script}: a `server_script` line names it"
            )),
            "{report}"
        );
        assert!(!written.contains(script), "{written}");
        assert!(!built.iter().any(|path| path == script), "{built:?}");
        assert!(
            dir.path().join("harbour").join(script).is_file(),
            "it is in the folder of the project"
        );
    }
    assert!(
        written.contains("files {\n  'client.lua',\n  'config.lua',\n}"),
        "{written}"
    );
    let project =
        std::fs::read_to_string(dir.path().join("harbour/harbour.openrage-proj")).unwrap();
    assert!(
        project.contains("server/sv_doors.lua") && project.contains("no build ships it"),
        "the project file records what was not placed: {project}"
    );
}

/// ⚠ The report says of a line what the plan of the adopted project does, so the built manifest is what it is compared with.
#[test]
fn what_the_report_says_a_build_carries_of_a_manifest_is_what_the_built_manifest_states() {
    // A pack of props holds no map file, and its build declares it no map.
    let (report, _dir, _, written) = adopted_and_built(&[
        ("harbour/fxmanifest.lua", HEAD.as_bytes().to_vec()),
        (
            "harbour/stream/harbour_types.ytyp",
            crate_types("harbour_types"),
        ),
        (
            "harbour/stream/harbour_crate.ydd",
            drawable_dictionary(CRATE, 6, 0.0),
        ),
    ]);
    assert!(
        report.contains("manifest line 3 this_is_a_map 'yes': not carried: no map file of the resource becomes a source"),
        "{report}"
    );
    assert!(!written.contains("this_is_a_map"), "{written}");

    // A request for type files by a pattern is carried by each type file it names.
    let by_pattern = format!("{HEAD}data_file 'DLC_ITYP_REQUEST' 'stream/**/*.ytyp'\n");
    let (report, _dir, _, written) = adopted_and_built(&[
        ("harbour/fxmanifest.lua", by_pattern.into_bytes()),
        ("harbour/stream/harbour.ymap", one_map()),
        (
            "harbour/stream/harbour_types.ytyp",
            crate_types("harbour_types"),
        ),
    ]);
    assert!(
        report.contains(
            "data_file 'DLC_ITYP_REQUEST' 'stream/**/*.ytyp': carried: it names 1 file(s)"
        ),
        "{report}"
    );
    assert!(report.contains("this_is_a_map 'yes': carried"), "{report}");
    assert!(
        written.contains("this_is_a_map 'yes'")
            && written.contains("data_file 'DLC_ITYP_REQUEST' 'stream/harbour_types.ytyp'"),
        "{written}"
    );

    // A type file of the name of a map file is kept whole, and the pattern that names it states its type through the Files content.
    let by_pattern = format!("{HEAD}data_file 'DLC_ITYP_REQUEST' 'stream/*.ytyp'\n");
    let (report, _dir, built, written) = adopted_and_built(&[
        ("harbour/fxmanifest.lua", by_pattern.into_bytes()),
        ("harbour/stream/harbour.ymap", one_map()),
        ("harbour/stream/harbour.ytyp", crate_types("harbour")),
    ]);
    assert!(
        report.contains("kept whole stream/harbour.ytyp"),
        "{report}"
    );
    assert!(
        built.iter().any(|path| path == "stream/harbour.ytyp"),
        "{built:?}"
    );
    assert!(
        written.contains("data_file 'DLC_ITYP_REQUEST' 'stream/harbour.ytyp'"),
        "{written}"
    );

    // A word whose text holds a backslash is not carried, since what Lua reads of it is not what was read here.
    let escaped = format!("{HEAD}author 'O\\'Neil'\ndescription 'Line one\\nLine two'\n");
    let (report, _dir, _, written) = adopted_and_built(&[
        ("harbour/fxmanifest.lua", escaped.into_bytes()),
        ("harbour/stream/harbour.ymap", one_map()),
    ]);
    assert!(
        report
            .contains("description 'Line one\\nLine two': not carried: its text holds a backslash"),
        "{report}"
    );
    assert!(
        written.contains("author 'O\\'Neil'") && !written.contains("description"),
        "{written}"
    );
}

/// ⚠ A build cooks into `stream`, and on a disk that does not tell case apart `Stream` is that folder.
#[test]
fn a_stream_folder_spelled_with_a_capital_is_adopted_into_a_project_that_builds() {
    let manifest = format!("{HEAD}data_file 'HANDLING_FILE' 'Stream/notes.txt'\n");
    let (report, dir, built, written) = adopted_and_built(&[
        ("harbour/fxmanifest.lua", manifest.into_bytes()),
        ("harbour/Stream/harbour.ymap", one_map()),
        ("harbour/Stream/notes.txt", b"kept".to_vec()),
        ("harbour/Stream/sealed.ydr", locked()),
    ]);
    assert!(report.contains("kept whole Stream/notes.txt"), "{report}");
    assert_eq!(
        built,
        [
            "fxmanifest.lua",
            "stream/harbour.ymap",
            "stream/notes.txt",
            "stream/sealed.ydr"
        ]
    );
    assert!(
        written.contains("data_file 'HANDLING_FILE' 'stream/notes.txt'"),
        "{written}"
    );
    let kept = names_in(&dir.path().join("harbour/stream"));
    assert_eq!(kept, ["notes.txt", "sealed.ydr"]);
}

/// ⚠ A source cooks into the stream folder by its name alone, so a second file of that name standing there would ship at its path.
#[test]
fn a_second_model_of_a_name_where_the_first_cooks_to_is_left_out_and_the_project_plans() {
    let (first, second) = (
        drawable_dictionary(CRATE, 6, 0.0),
        drawable_dictionary(CRATE, 12, 1.0),
    );
    assert_ne!(first, second);
    let (report, dir, built, _) = adopted_and_built(&[
        ("harbour/fxmanifest.lua", HEAD.as_bytes().to_vec()),
        ("harbour/stream/harbour.ymap", one_map()),
        (
            "harbour/stream/harbour_types.ytyp",
            crate_types("harbour_types"),
        ),
        ("harbour/stream/a/harbour_crate.ydd", first),
        ("harbour/stream/harbour_crate.ydd", second),
    ]);
    assert!(
        report.contains(
            "left out stream/harbour_crate.ydd: `stream/a/harbour_crate.ydd` becomes a source"
        ),
        "{report}"
    );
    assert_eq!(
        built.iter().filter(|path| path.ends_with(".ydd")).count(),
        1,
        "{built:?}"
    );
    assert!(!dir.path().join("harbour/stream/harbour_crate.ydd").exists());
}

/// ⚠ An asset escrow writes `.fxap` beside the files it locks: a name that begins with a dot, and still a file of the resource.
#[test]
fn the_file_an_asset_escrow_writes_beside_what_it_locks_ships_with_the_locked_files() {
    let files = [
        (
            "harbour/.fxap",
            b"what the escrow wrote of the resource".to_vec(),
        ),
        ("harbour/.git/config", b"[core]".to_vec()),
        ("harbour/fxmanifest.lua", HEAD.as_bytes().to_vec()),
        ("harbour/notes.txt", b"kept".to_vec()),
        ("harbour/stream/harbour.ymap", one_map()),
        ("harbour/stream/sealed.ydr", locked()),
    ];
    let (report, _dir, built, written) = adopted_and_built(&files);
    assert!(
        report.contains(
            "kept whole .fxap: the file an asset escrow writes beside the files it locks"
        ),
        "{report}"
    );
    assert!(report.contains("outside harbour/.git: hidden"), "{report}");
    assert!(!report.contains("holds no `.fxap`"), "{report}");
    assert_eq!(
        built,
        [
            ".fxap",
            "fxmanifest.lua",
            "notes.txt",
            "stream/harbour.ymap",
            "stream/sealed.ydr"
        ]
    );
    assert!(
        written.contains("files {\n  'notes.txt',\n}"),
        "a server reads it and sends it to no client: {written}"
    );

    // Without it the report says the locked files have nothing beside them.
    let (report, ..) = adopted_and_built(&files[2..]);
    assert!(
        report.contains(
            "note: 1 file(s) of harbour are locked by an asset escrow and it holds no `.fxap`"
        ),
        "{report}"
    );
}
