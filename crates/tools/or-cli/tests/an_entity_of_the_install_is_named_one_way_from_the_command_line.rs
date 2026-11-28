//! An entity of an install is named one way by the verbs that print one and the verb that takes one: `world find` and
//! `world select` each say the logical path of its map file and its place among that file's entities, `world find` its guid too,
//! and `maps edit-game` edits the entity so named. Over an install built by hand and a project in a temporary folder.
//!
//! The same install holds that each of those verbs, and `world ground`, reads the world index as this build keeps it: a kept
//! file of an older version is refused and built again, never read.
//!
//! ⚠ The commands print with `println!`, which a test cannot read back in its own process, so each run is a child of this test
//! binary that calls `or_cli::run` between two markers, with a configuration directory of its own so that no key of this machine
//! is read.

#[path = "../../../domain/or-world-gta5/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_io::decl::File;
use or_io::meta::typed::CMapData;
use or_maps::sample::{map_file, Entity};

const CHILD_ENV: &str = "OPENRAGE_ONE_NAME_ARGS";
const CHILD_TEST: &str = "run_a_command_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";
const BOX: ([f32; 3], [f32; 3]) = ([0.0; 3], [100.0; 3]);

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_a_command_for_a_parent() {
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

/// What `openrage-cli <args> --json` prints, as the value it is; a command that fails is a failure of the test.
fn json(scratch: &Path, args: &[&str]) -> serde_json::Value {
    let args: Vec<&str> = args.iter().copied().chain(["--json"]).collect();
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_SCHEMA", "none")
        .env("OPENRAGE_CONFIG_DIR", scratch.join("config"))
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let error = String::from_utf8_lossy(&output.stderr);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}"));
    let (printed, tail) = rest
        .split_once(END)
        .unwrap_or_else(|| panic!("the command panicked: {rest}{error}"));
    assert!(tail.starts_with('1'), "{args:?}: {printed}{error}");
    serde_json::from_str(printed.trim())
        .unwrap_or_else(|e| panic!("{args:?} prints one JSON value: {e}: {printed}"))
}

const BENCH_AT: [f32; 3] = [10.0, 20.0, 1.0];
const CRATE_AT: [f32; 3] = [30.0, 20.0, 1.0];
const BENCH_GUID: u32 = 11;

/// An install of one map, `m_town`: a bench a type file declares and a file names, which `world find` finds by its name, then
/// a crate no type file declares, which `world select` says stands as a box.
fn install(scratch: &Path) -> PathBuf {
    let root = scratch.join("install");
    std::fs::create_dir(&root).unwrap();
    let town = map_file(
        "m_town",
        &[
            Entity::at("bench", BENCH_AT, BENCH_GUID),
            Entity::at("crate", CRATE_AT, 12),
        ],
        BOX,
    );
    let unit = world_fixture::cube([-1.0; 3], 2.0);
    world_fixture::write_archive(&root, "common.rpf", &[("data/empty.txt", Vec::new())]);
    world_fixture::write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", town),
            (
                "types/props.ytyp",
                world_fixture::types_bytes("props", &[("bench", unit, 80.0)]),
            ),
            ("models/bench.ydr", b"a model".to_vec()),
        ],
    );
    root
}

/// A project in `scratch` with one empty maps content.
fn project(scratch: &Path) -> PathBuf {
    let folder = scratch.join("project");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("world.openrage-proj");
    let text = r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "world", "take": ["maps"] }],
  "schema": 4
}"#;
    std::fs::write(&file, text).unwrap();
    std::fs::create_dir(folder.join("maps")).unwrap();
    or_maps::Maps::new().write(&folder.join("maps")).unwrap();
    file
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

fn hex(name: &str) -> String {
    format!("{:#010X}", world_fixture::hash(name))
}

/// The archetype and the place of each entity of the source `source` of the maps content of `project`, as it cooks now.
fn entities_of(project: &Path, source: &str) -> Vec<(u32, [f32; 3])> {
    let folder = project.parent().unwrap().join("maps").join(source);
    let cooked = or_maps::facts::cooked(&folder).expect("the source cooks");
    let map = CMapData::from_bytes(&cooked).expect("it cooks to a map file");
    let entities = map.entities.iter();
    entities
        .map(|entity| {
            let at = entity.position;
            (entity.archetype_name.hash, [at.x, at.y, at.z])
        })
        .collect()
}

#[test]
fn what_world_find_and_world_select_print_of_an_entity_is_what_maps_edit_game_takes() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let project = project(scratch.path());
    let kept = scratch.path().join("world.index");
    let (game, kept) = (text(&root), text(&kept));

    // `world find` names the bench by the file of its map, its place there and its guid.
    let found = json(
        scratch.path(),
        &["world", "find", game, "bench", "--kept", kept],
    );
    let listed = found["placements"]["listed"].as_array().expect("listed");
    assert_eq!(listed.len(), 1, "{found:#}");
    let bench = &listed[0];
    assert_eq!(bench["map"], "m_town");
    assert_eq!(
        (&bench["entity"], &bench["guid"]),
        (&0.into(), &BENCH_GUID.into())
    );
    let bench_file = bench["logical"].as_str().expect("its map file");

    // `world select` names the crate, which stands as a box, by the file of its map and its place there.
    let [x, y, z] = CRATE_AT;
    let view = format!("{x},{y},{z}");
    let selected = json(
        scratch.path(),
        &["world", "select", game, &view, "--kept", kept],
    );
    let boxes = selected["instances"]["boxes"].as_array().expect("boxes");
    assert_eq!(boxes.len(), 1, "{selected:#}");
    let crate_box = &boxes[0];
    assert_eq!(crate_box["archetype"], hex("crate"));
    assert_eq!(
        (&crate_box["map"], &crate_box["entity"]),
        (&"m_town".into(), &1.into())
    );
    assert!(crate_box["interior"].is_null());
    let crate_file = crate_box["logical"].as_str().expect("its map file");
    // One file, said one way by both, and as the list of maps the view wants says it.
    assert_eq!(crate_file, bench_file);
    assert_eq!(selected["maps"][0]["logical"], bench_file);

    let edit = |entity: &str, to: &str, apply: bool| -> serde_json::Value {
        let mut args = vec![
            "maps",
            "edit-game",
            text(&project),
            "--game",
            game,
            "--kept",
            kept,
            "--map",
            bench_file,
            "--entity",
            entity,
            "--move",
            to,
        ];
        args.extend(apply.then_some("--apply"));
        json(scratch.path(), &args)
    };
    // The bench by its guid and by its place is one entity, the one `world find` named.
    let by_guid = edit(&format!("guid:{BENCH_GUID}"), "11,21,2", false);
    let by_place = edit(&bench["entity"].to_string(), "11,21,2", false);
    assert_eq!(by_guid["entity"], by_place["entity"]);
    assert_eq!(by_guid["entity"]["index"], bench["entity"]);

    // Done, it is the bench that moved and the crate that stayed.
    let done = edit(&bench["entity"].to_string(), "11,21,2", true);
    let source = done["entity"]["source"].as_str().expect("the source");
    let (bench_model, crate_model) = (world_fixture::hash("bench"), world_fixture::hash("crate"));
    assert_eq!(
        entities_of(&project, source),
        [(bench_model, [11.0, 21.0, 2.0]), (crate_model, CRATE_AT)]
    );
    // And the place `world select` gave the crate by moves the crate, in the source the map now is.
    let done = edit(&crate_box["entity"].to_string(), "31,22,3", true);
    assert_eq!(done["entity"]["source"], source);
    assert_eq!(
        entities_of(&project, source),
        [
            (bench_model, [11.0, 21.0, 2.0]),
            (crate_model, [31.0, 22.0, 3.0])
        ]
    );
}

/// The version a kept world index states, which follows its four bytes of magic.
fn version_of(kept: &Path) -> u32 {
    let bytes = std::fs::read(kept).expect("the kept index");
    assert_eq!(&bytes[..4], b"ORWI");
    u32::from_le_bytes(bytes[4..8].try_into().unwrap())
}

#[test]
fn each_verb_that_reads_the_world_index_builds_again_a_kept_file_of_an_older_version() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let project = project(scratch.path());
    let kept_file = scratch.path().join("world.index");
    let (game, kept) = (text(&root), text(&kept_file));
    let [x, y, z] = CRATE_AT;
    let view = format!("{x},{y},{z}");
    let (xs, ys) = (x.to_string(), y.to_string());

    let find = ["world", "find", game, "bench", "--kept", kept];
    let select = ["world", "select", game, &view, "--kept", kept];
    let ground = ["world", "ground", "--game", game, &xs, &ys, "--kept", kept];
    let edit = [
        "maps",
        "edit-game",
        text(&project),
        "--game",
        game,
        "--kept",
        kept,
        "--map",
        "platform:/levels/m_town.ymap",
        "--entity",
        "0",
        "--move",
        "11,21,2",
    ];
    // The first builds and keeps the index, at the version this build writes.
    let first = json(scratch.path(), &find);
    let current = version_of(&kept_file);
    assert_eq!(first["placements"]["listed"][0]["logical"], edit[8]);

    for verb in [&find[..], &select[..], &ground[..], &edit[..]] {
        let answer = json(scratch.path(), verb);
        // The same file, stated to be of the version before: nothing of it is taken, and it is kept again as this build's.
        let mut older = std::fs::read(&kept_file).unwrap();
        older[4..8].copy_from_slice(&(current - 1).to_le_bytes());
        std::fs::write(&kept_file, &older).unwrap();
        assert_eq!(version_of(&kept_file), current - 1);
        assert_eq!(json(scratch.path(), verb), answer, "{verb:?}");
        assert_eq!(version_of(&kept_file), current, "{verb:?}");
    }
}
