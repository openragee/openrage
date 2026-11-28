//! `openrage-cli maps edit-game` over an install built by hand and a project in a temporary folder: what it prints of an edit it
//! would make, that its JSON lists the same, that `--apply` does it and `maps changes` then reads it, and that the record it keeps
//! takes it back.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its own process, so each run is a child of this test
//! binary that calls `or_cli::run` between two markers, with a configuration directory of its own so that no key of this machine
//! is read.

#[path = "../../../domain/or-world-gta5/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_maps::sample::{child_map_file, map_file, Entity};

const CHILD_ENV: &str = "OPENRAGE_MAPS_EDIT_GAME_ARGS";
const CHILD_TEST: &str = "run_maps_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";
const BOX: ([f32; 3], [f32; 3]) = ([0.0; 3], [100.0; 3]);

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_maps_for_a_parent() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = (args.to_string_lossy().split(SEPARATOR))
        .map(str::to_string)
        .collect();
    let mut out = std::io::stdout();
    let _ = write!(out, "{BEGIN}");
    let _ = out.flush();
    let code = or_cli::run(&args);
    let _ = out.flush();
    let _ = write!(out, "{END}{}", u8::from(code == ExitCode::SUCCESS));
    let _ = out.flush();
}

/// What `openrage-cli maps <args>` prints, what it says of an error, and whether it succeeded.
fn maps(scratch: &Path, args: &[&str]) -> (String, String, bool) {
    let args: Vec<&str> = ["maps"].iter().chain(args).copied().collect();
    cli(scratch, &args)
}

/// The same of `openrage-cli <args>`.
fn cli(scratch: &Path, args: &[&str]) -> (String, String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_SCHEMA", "none")
        .env("OPENRAGE_CONFIG_DIR", scratch.join("config"))
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let error = String::from_utf8_lossy(&output.stderr).into_owned();
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}"));
    let (printed, tail) = rest
        .split_once(END)
        .unwrap_or_else(|| panic!("the command panicked: {rest}{error}"));
    (printed.to_string(), error, tail.starts_with('1'))
}

/// An install of one archive: `m_town`, which stands alone, a shed under its far model in `shed_lod`, and a file that is no map.
fn install(scratch: &Path) -> PathBuf {
    let root = scratch.join("install");
    std::fs::create_dir(&root).unwrap();
    let town = map_file(
        "m_town",
        &[
            Entity::at("bench", [10.0, 20.0, 1.0], 11),
            Entity::at("lamp", [30.0, 20.0, 1.0], 12),
        ],
        BOX,
    );
    let far = Entity {
        children: 1,
        ..Entity::at("lod_shed", [60.0, 60.0, 0.0], 500)
    };
    let near = Entity {
        parent: 0,
        in_parent_map: true,
        ..Entity::at("shed", [60.0, 60.0, 0.0], 501)
    };
    world_fixture::write_archive(&root, "common.rpf", &[("data/empty.txt", Vec::new())]);
    world_fixture::write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", town),
            ("levels/shed_lod.ymap", map_file("shed_lod", &[far], BOX)),
            (
                "levels/shed.ymap",
                child_map_file("shed", "shed_lod", &[near], BOX),
            ),
            ("levels/garbage.ymap", b"not a map file".to_vec()),
        ],
    );
    root
}

/// The logical path of the map called `name` in the install at `root`, as the world index states it.
fn logical(root: &Path, name: &str) -> String {
    let game = world_fixture::mount(root);
    let (index, _) =
        or_world_gta5::WorldIndex::build(&or_world_gta5::MountedWorld::new(&game, root), None);
    let map = index.map(world_fixture::hash(name));
    map.unwrap_or_else(|| panic!("the install holds {name}"))
        .logical
        .clone()
}

/// A project in `scratch` with one empty maps content.
fn project(scratch: &Path) -> PathBuf {
    project_with(
        scratch,
        r#"{ "id": "fivem", "kind": "fivem-resource", "name": "world", "take": ["maps"] }"#,
    )
}

/// The same, whose one build is `output` as the project file states a build.
fn project_with(scratch: &Path, output: &str) -> PathBuf {
    let folder = scratch.join("project");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("world.openrage-proj");
    let text = r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [OUTPUT],
  "schema": 4
}"#;
    std::fs::write(&file, text.replace("OUTPUT", output)).unwrap();
    std::fs::create_dir(folder.join("maps")).unwrap();
    or_maps::Maps::new().write(&folder.join("maps")).unwrap();
    file
}

/// Every file under `folder`, by its path inside it with `/`.
fn files(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let inside = path.strip_prefix(root).unwrap().to_string_lossy();
                into.insert(inside.replace('\\', "/"), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

#[test]
fn the_verb_prints_what_it_would_import_and_do_and_its_json_lists_what_its_text_lists() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let project = project(scratch.path());
    let kept = scratch.path().join("world.index");
    let shed = logical(&root, "shed");
    let before = files(project.parent().unwrap());
    let args = [
        "edit-game",
        text(&project),
        "--game",
        text(&root),
        "--kept",
        text(&kept),
        "--map",
        &shed,
        "--entity",
        "guid:501",
        "--delete",
    ];

    let (printed, error, ok) = maps(scratch.path(), &args);
    assert!(ok, "{printed}{error}");
    let with_json: Vec<&str> = args.iter().copied().chain(["--json"]).collect();
    let (document, error, ok) = maps(scratch.path(), &with_json);
    assert!(ok, "{document}{error}");
    assert_eq!(
        files(project.parent().unwrap()),
        before,
        "without --apply nothing is written"
    );

    // Each entry of the document is one line of the text that no other entry is, and no line is left over but the two that frame it.
    let json: serde_json::Value = serde_json::from_str(&document).unwrap();
    let lines: Vec<&str> = printed.lines().collect();
    let mut matched = vec![false; lines.len()];
    let mut claim = |is: &dyn Fn(&str) -> bool, what: String| {
        let at: Vec<usize> = (0..lines.len())
            .filter(|at| !matched[*at] && is(lines[*at]))
            .collect();
        assert_eq!(at.len(), 1, "{what} is one line of:\n{lines:#?}");
        matched[at[0]] = true;
    };
    let string = |value: &serde_json::Value| value.as_str().unwrap().to_string();
    claim(&|line| line == "content maps:", "the content".to_string());
    claim(
        &|line| line == "nothing was written: say --apply to do it",
        "that nothing was written".to_string(),
    );
    assert_eq!(json["done"], false);
    assert_eq!(json["edit"]["map"], shed.as_str());
    let entity = format!(
        "entity {} of {}",
        json["entity"]["index"],
        string(&json["entity"]["source"])
    );
    claim(
        &|line| line.contains(&shed) && line.ends_with(&entity),
        entity.clone(),
    );
    let imported = json["imported"].as_array().unwrap();
    assert_eq!(imported.len(), 2);
    for import in imported {
        let begins = format!(
            "would import {} as {}:",
            string(&import["map"]),
            string(&import["source"])
        );
        claim(&|line| line.starts_with(&begins), begins.clone());
    }
    let operations = json["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 2);
    for (at, operation) in operations.iter().enumerate() {
        let whole = format!(
            "would do {}. {} of {}",
            at + 1,
            string(&operation["op"]),
            string(&operation["source"])
        );
        claim(&|line| line == whole, whole.clone());
    }
    for note in json["notes"].as_array().unwrap() {
        let whole = format!("note: {}", string(note));
        claim(&|line| line == whole, whole.clone());
    }
    let plan = json["plan"].as_array().unwrap();
    assert!(
        plan.iter().any(|line| string(line).starts_with("  goes ")),
        "{plan:#?}"
    );
    for line in plan {
        let whole = string(line);
        claim(&|line| line == whole, whole.clone());
    }
    let mut told = 0;
    for part in ["line", "imported", "sharing"] {
        let message = &json["told"][part];
        // The count of each sentence here is two, so the text is the form for several.
        let whole = format!("told: {}", string(&message["other"]))
            .replace("{count}", &message["count"].to_string())
            .replace("{map}", "shed.ymap");
        claim(&|line| line == whole, whole.clone());
        told += 1;
    }
    assert_eq!(told, 3);
    assert!(
        matched.iter().all(|matched| *matched),
        "a line no entry of the document states: {lines:#?}"
    );
}

#[test]
fn with_apply_the_verb_imports_and_edits_and_the_record_it_keeps_takes_both_back() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let project = project(scratch.path());
    let folder = project.parent().unwrap();
    let kept = scratch.path().join("world.index");
    let record = scratch.path().join("record.json");
    let town = logical(&root, "m_town");
    let before = files(folder);

    let (printed, error, ok) = maps(
        scratch.path(),
        &[
            "edit-game",
            text(&project),
            "--game",
            text(&root),
            "--kept",
            text(&kept),
            "--map",
            &town,
            "--entity",
            "0",
            "--move",
            "12,-21.5,1.5",
            "--apply",
            "--record",
            text(&record),
        ],
    );
    assert!(ok, "{printed}{error}");
    for line in [
        format!("move entity 0 of {town} to 12, -21.5, 1.5: entity 0 of m_town.metadata.src\n"),
        format!("imported {town} as m_town.metadata.src: a source taken from the game"),
        "did 1. place_entity of m_town.metadata.src\n".to_string(),
        "told: Moved an object of m_town.ymap\n".to_string(),
        "told: m_town.ymap is now a map of this project\n".to_string(),
        "told: It is a source taken from the game".to_string(),
        "Undo: takes back the edit and the import as one, by 1 inverse operation(s) and the removal of 1 source(s)\n".to_string(),
    ] {
        assert!(printed.contains(&line), "{line:?} is not in:\n{printed}");
    }
    assert!(!printed.contains("nothing was written"), "{printed}");
    let base = folder.join("maps/m_town.metadata.src/base.ymap");
    let shipped = world_fixture::mount(&root).vfs.open(&town).unwrap();
    assert_eq!(std::fs::read(base).unwrap(), shipped);

    // What the content changed of the game is that one move.
    let (changes, error, ok) = maps(scratch.path(), &["changes", text(&project)]);
    assert!(ok, "{changes}{error}");
    assert!(
        changes.contains(
            "m_town.metadata.src (m_town.ymap): 0 added, 0 deleted, 1 changed, 0 ambiguous, 1 unchanged"
        ),
        "{changes}"
    );
    assert!(
        changes.contains("(10, 20, 1) to (12, -21.5, 1.5)"),
        "{changes}"
    );

    // What the person was told of the source is so: the check before sharing names it, and the check against the game finds the
    // file it was taken from where its manifest says the install holds it.
    let (sharing, error, ok) = cli(
        scratch.path(),
        &["project", "check-sharing", text(&project)],
    );
    assert!(
        !ok,
        "a source taken from the game is not the person's to send: {sharing}{error}"
    );
    assert!(
        sharing.contains("maps/m_town.metadata.src is the game's own file"),
        "{sharing}{error}"
    );
    let (original, error, ok) = cli(
        scratch.path(),
        &[
            "project",
            "check-original",
            text(&project),
            "--game",
            text(&root),
        ],
    );
    assert!(
        ok,
        "the install still holds the file the source was taken from: {original}{error}"
    );
    assert!(original.contains("m_town.metadata.src: "), "{original}");

    // The record kept takes the edit and the import back, and the project is as it was.
    let (printed, error, ok) = maps(
        scratch.path(),
        &["edit-game", text(&project), "--take-back", text(&record)],
    );
    assert!(ok, "{printed}{error}");
    assert!(
        printed.contains("removed m_town.metadata.src, which the edit imported\n"),
        "{printed}"
    );
    assert_eq!(files(folder), before);
}

#[test]
fn a_refused_edit_names_the_file_exits_with_an_error_and_writes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let project = project(scratch.path());
    let kept = scratch.path().join("world.index");
    let garbage = logical(&root, "garbage");
    let before = files(project.parent().unwrap());
    let common = [
        "edit-game",
        text(&project),
        "--game",
        text(&root),
        "--kept",
        text(&kept),
        "--entity",
        "0",
    ];
    let with = |more: &[&str], map: &str| -> (String, String, bool) {
        let args: Vec<&str> = (common.iter().copied())
            .chain(["--map", map])
            .chain(more.iter().copied())
            .collect();
        maps(scratch.path(), &args)
    };

    for more in [["--delete", "--apply"], ["--move", "1,2,3"]] {
        let (printed, error, ok) = with(&more, &garbage);
        assert!(!ok, "{printed}");
        assert!(
            error.contains(&format!("`{garbage}` does not read as a map file")),
            "{error}"
        );
    }
    // What the command line states badly is a usage error, before anything is read.
    let town = logical(&root, "m_town");
    let both = "say --move x,y,z or --delete, and not both";
    // ⚠ In the scratch folder: a command that kept it against its own rule would otherwise write beside this crate.
    let record = scratch.path().join("r.json");
    for (more, why) in [
        (&["--delete", "--move", "1,2,3"][..], both),
        (&[][..], both),
        (&["--move", "1,2"][..], "--move takes a position as x,y,z"),
        (
            &["--move", "1,2,3", "--record", text(&record)][..],
            "--record keeps the record of an edit that was done",
        ),
    ] {
        let (printed, error, ok) = with(more, &town);
        assert!(!ok, "{more:?}: {printed}");
        assert!(error.contains(why), "{more:?}: {error}");
    }
    assert!(!record.exists(), "a record of an edit that was not done");
    assert_eq!(files(project.parent().unwrap()), before);
}

#[test]
fn the_edition_of_the_install_the_verb_was_given_is_what_a_build_of_the_other_refuses() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    // ⚠ The install holds no executable, which is how one of GTA V Legacy is told apart here: the build is for the other edition.
    let project = project_with(
        scratch.path(),
        r#"{ "id": "fivem", "kind": "fivem-resource", "name": "world", "take": ["maps"], "target": "enhanced" }"#,
    );
    let kept = scratch.path().join("world.index");
    let town = logical(&root, "m_town");
    let before = files(project.parent().unwrap());

    let (printed, error, ok) = maps(
        scratch.path(),
        &[
            "edit-game",
            text(&project),
            "--game",
            text(&root),
            "--kept",
            text(&kept),
            "--map",
            &town,
            "--entity",
            "0",
            "--move",
            "1,2,3",
            "--apply",
        ],
    );
    assert!(!ok, "{printed}");
    assert!(
        error.contains(&format!("`{town}` is a map of a GTA V Legacy install")),
        "{error}"
    );
    assert!(
        error.contains("the build `world` of this content is for GTA V Enhanced"),
        "{error}"
    );
    assert_eq!(files(project.parent().unwrap()), before);
}

/// ⚠ `world find` and `world select` mount as `--level` and `--mode` say, and a path and a place they print go to this verb.
#[test]
fn the_verb_mounts_at_the_level_and_in_the_mode_the_verbs_that_print_an_entity_mount_at() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let project = project(scratch.path());
    let kept = scratch.path().join("world.index");
    let town = logical(&root, "m_town");
    let common = [
        "edit-game",
        text(&project),
        "--game",
        text(&root),
        "--kept",
        text(&kept),
        "--map",
        town.as_str(),
        "--entity",
        "0",
        "--move",
        "1,2,3",
    ];
    let with = |more: &[&str]| -> (String, String, bool) {
        let args: Vec<&str> = common.iter().chain(more).copied().collect();
        maps(scratch.path(), &args)
    };
    let (plain, error, ok) = with(&[]);
    assert!(ok, "{error}");
    let (in_mode, error, ok) = with(&["--mode", "story"]);
    assert!(ok, "{error}");
    assert_eq!(
        in_mode, plain,
        "an install with no pack is one map in either mode"
    );
    // A level the install's list does not name is the mount's refusal, with the level, and no unknown flag.
    let (printed, error, ok) = with(&["--level", "nowhere"]);
    assert!(!ok, "{printed}");
    assert!(
        error.contains("nowhere") && !error.contains("unknown flag"),
        "{error}"
    );
}
