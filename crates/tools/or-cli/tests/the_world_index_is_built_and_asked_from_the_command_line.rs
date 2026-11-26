//! `openrage-cli world index`, `world at`, `world ground` and `world find` over an install built by
//! hand: what each prints, and where the index is kept when no file is named for it.
//!
//! ⚠ The commands print with `println!`, which a test cannot read back in its own process, so each
//! runs in a child of this test binary that calls `or_cli::run` between two markers. The child is
//! given its own configuration directory, which is where an index with no `--kept` is kept.

#[path = "../../../domain/or-depends/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_title::dlc::{ContentChangeSet, ContentChangeSetGroup};
use or_title::{ContentXml, DataFileEntry, DlcList, SetupData};
use world_fixture::{
    bound_bytes, cube, hash, level_square, manifest_bytes, map_bytes, mesh_bound_bytes,
    placing_map_bytes, types_bytes, write_archive, MapSpec, Placed,
};

const CHILD_ENV: &str = "OPENRAGE_WORLD_INDEX_ARGS";
const CHILD_TEST: &str = "run_world_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_world_for_a_parent() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = args
        .to_string_lossy()
        .split(SEPARATOR)
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

/// What `openrage-cli world <args>` prints with `config` as its configuration directory, and
/// whether it succeeded.
fn world(config: &Path, args: &[&str]) -> (String, bool) {
    let args: Vec<&str> = ["world"].iter().chain(args).copied().collect();
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_SCHEMA", "none")
        .env("OPENRAGE_CONFIG_DIR", config)
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}"));
    let (printed, tail) = rest.split_once(END).unwrap_or_else(|| {
        panic!(
            "the command panicked: {rest}{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (printed.to_string(), tail.starts_with('1'))
}

/// An install under `scratch` named `name`: a town, a shop inside it, and a bound a manifest names
/// for the town.
fn install(scratch: &Path, name: &str) -> PathBuf {
    let root = scratch.join(name);
    std::fs::create_dir(&root).expect("mkdir");
    let shop = MapSpec {
        parent: "m_town",
        ..MapSpec::around(cube([120.0, 120.0, 0.0], 20.0))
    };
    let town = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    let manifest = manifest_bytes(&[("m_town", &["m_town_0"])], &[]);
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", map_bytes("m_town", &town)),
            ("levels/m_shop.ymap", map_bytes("m_shop", &shop)),
            (
                "levels/m_town_0.ybn",
                bound_bytes(cube([110.0, 110.0, 0.0], 50.0)),
            ),
        ],
    );
    root
}

/// The files kept under the configuration directory `config`, by name.
fn kept_under(config: &Path) -> Vec<String> {
    let Ok(folder) = std::fs::read_dir(config.join("gta5_world_index")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = folder
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_world_index_is_built_kept_and_asked_from_the_command_line() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    let root = install(scratch.path(), "install");
    let game = root.to_string_lossy();

    // The first run reads every archive and keeps the index under the configuration directory.
    let (printed, ok) = world(&config, &["index", "--game", &game]);
    assert!(ok, "{printed}");
    assert!(
        printed.contains("none was read, so every archive was"),
        "{printed}"
    );
    assert!(
        printed.contains("  2 archive(s), 2 read, 4 file(s) opened"),
        "{printed}"
    );
    assert!(
        printed.contains("  2 map(s), 0 that did not read as one"),
        "{printed}"
    );
    let kept = kept_under(&config);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(kept[0].ends_with(".index"), "{kept:?}");

    // The second, of the install spelled another way, reads nothing and keeps no second file.
    let spelled = format!("{game}/../install");
    let (printed, ok) = world(&config, &["index", "--game", &spelled, "--json"]);
    assert!(ok, "{printed}");
    let json: serde_json::Value = serde_json::from_str(printed.trim()).expect("one JSON value");
    assert_eq!(json["kept"], "current");
    assert_eq!(json["read"], serde_json::json!([]));
    assert_eq!(json["opened"], 0);
    assert_eq!(json["maps"], 2);
    assert_eq!(kept_under(&config), kept);

    // A position, as lines and as JSON; the maps come in the order their archive lists them.
    let (printed, ok) = world(&config, &["at", "--game", &game, "130", "130", "10"]);
    assert!(ok, "{printed}");
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(
        lines,
        [
            "At (130, 130, 10): 2 map(s) whose streaming extents hold it, 1 bound(s) whose box does",
            "  map    m_shop  platform:/levels/m_shop.ymap  parent m_town  flags 0x0  contentFlags 0x0",
            "  map    m_town  platform:/levels/m_town.ymap  flags 0x0  contentFlags 0x0",
            "  bound  m_town_0  platform:/levels/m_town_0.ybn",
            "           carried by m_town, by a manifest's map group",
        ]
    );
    let (printed, ok) = world(
        &config,
        &["at", "--json", "--game", &game, "-500", "130", "10"],
    );
    assert!(ok, "{printed}");
    let json: serde_json::Value = serde_json::from_str(printed.trim()).expect("one JSON value");
    assert_eq!(json["at"], serde_json::json!([-500.0, 130.0, 10.0]));
    assert_eq!(json["maps"], serde_json::json!([]));
    assert_eq!(json["bounds"], serde_json::json!([]));
    assert_eq!(
        kept_under(&config),
        kept,
        "asking a position keeps no other file"
    );

    // `--fresh` reads it all again whatever is kept.
    let (printed, ok) = world(&config, &["index", "--game", &game, "--fresh", "--json"]);
    assert!(ok, "{printed}");
    let json: serde_json::Value = serde_json::from_str(printed.trim()).expect("one JSON value");
    assert_eq!(json["kept"], "absent");
    assert_eq!(json["read"], serde_json::json!(["common.rpf", "x64a.rpf"]));

    // Another install has a kept index of its own.
    let other = install(scratch.path(), "other");
    let (printed, ok) = world(&config, &["index", "--game", &other.to_string_lossy()]);
    assert!(ok, "{printed}");
    assert!(
        printed.contains("none was read, so every archive was"),
        "{printed}"
    );
    let both = kept_under(&config);
    assert_eq!(both.len(), 2, "{both:?}");
    assert!(both.contains(&kept[0]), "{both:?}");

    // Two numbers are no position, and a folder that is no install has no index.
    let (printed, ok) = world(&config, &["at", "--game", &game, "130", "130"]);
    assert!(!ok && printed.is_empty(), "{printed}");
    let (printed, ok) = world(&config, &["index", "--game", &config.to_string_lossy()]);
    assert!(!ok && printed.is_empty(), "{printed}");
    assert_eq!(kept_under(&config), both);
}

#[test]
fn the_ground_under_a_position_is_asked_from_the_command_line() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    // A town whose one bound is a floor forty metres square, of a material word of the test's own.
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let floor = level_square([110.0, 110.0, 10.0], 40.0, 0x0507);
    let height = floor.vertex_position(0).expect("a vertex")[2];
    let manifest = manifest_bytes(&[("m_town", &["m_town_0"])], &[]);
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    let town = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", map_bytes("m_town", &town)),
            ("levels/m_town_0.ybn", mesh_bound_bytes(floor)),
        ],
    );
    let game = root.to_string_lossy();

    // No height: the highest ground of the column. The install has no table of materials, so the
    // material is its word and its type.
    let (printed, ok) = world(&config, &["ground", "--game", &game, "120", "130"]);
    assert!(ok, "{printed}");
    let lines: Vec<&str> = printed.lines().collect();
    let expected = [
        format!("Ground under (120, 130), the whole of its column: {height:.3}"),
        format!("  at (120.000, 130.000, {height:.3}), normal (0.000, 0.000, 1.000)"),
        format!("  material {:#018x}, type 7", 0x0507),
        "  from m_town_0  platform:/levels/m_town_0.ybn".to_string(),
        "  looked in 1 bounds file(s); 0 did not read; 0 not loaded, which an interior places"
            .to_string(),
        "    asked     m_town_0  platform:/levels/m_town_0.ybn".to_string(),
    ];
    assert_eq!(lines, expected);
    assert_eq!(kept_under(&config).len(), 1, "the index it built is kept");

    // A height under the floor, as JSON, and a mode, which is said to leave nothing out.
    let under = ["ground", "--json", "--game", &game, "120", "130", "4"];
    let (printed, ok) = world(&config, &under);
    assert!(ok, "{printed}");
    let json: serde_json::Value = serde_json::from_str(printed.trim()).expect("one JSON value");
    assert_eq!(json["ground"], serde_json::json!(null));
    assert_eq!(json["from"], serde_json::json!(4.0));
    assert_eq!(json["lookedIn"][0]["name"], "m_town_0");
    let story = ["ground", "--game", &game, "120", "130", "--mode", "story"];
    let (printed, ok) = world(&config, &story);
    assert!(ok, "{printed}");
    assert_eq!(printed.lines().next(), Some(expected[0].as_str()));
    assert!(
        printed.contains("  note: mounted as story, which leaves out no bounds file"),
        "{printed}"
    );
    assert_eq!(
        kept_under(&config).len(),
        1,
        "a mode keeps no index of its own"
    );

    // One number is no position, a mode is one of two words, and a level the install's list
    // lacks fails the mount: each prints nothing.
    for refused in [
        &["ground", "--game", &game, "120"][..],
        &["ground", "--game", &game, "120", "130", "--mode", "online"],
        &["ground", "--game", &game, "120", "130", "--level", "mpnone"],
        &["ground", "120", "130"],
    ] {
        let (printed, ok) = world(&config, refused);
        assert!(!ok && printed.is_empty(), "{refused:?}: {printed}");
    }
}

/// The archive of the town's ground, with its one bound a floor forty metres square from `x`.
fn write_town(root: &Path, x: f32) {
    let town = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    let floor = level_square([x, 110.0, 10.0], 40.0, 0x0507);
    write_archive(
        root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", map_bytes("m_town", &town)),
            ("levels/m_town_0.ybn", mesh_bound_bytes(floor)),
        ],
    );
}

/// A pack named `name` in the install at `root`, holding `files`; `change_set` is one that
/// enables `enables`, which multiplayer's map group alone names.
fn write_pack(
    root: &Path,
    name: &str,
    order: i32,
    change_set: Option<(&str, &str)>,
    mut files: Vec<(&str, Vec<u8>)>,
) {
    let mut setup = SetupData {
        name_hash: name.into(),
        device_name: format!("dlc_{name}"),
        dat_file: "content.xml".into(),
        order,
        ..Default::default()
    };
    let mut content = ContentXml::default();
    if let Some((set, enables)) = change_set {
        setup.content_change_set_groups = vec![ContentChangeSetGroup {
            name_hash: "GROUP_MAP".into(),
            content_change_sets: vec![set.to_string().into()],
        }];
        content.data_files = vec![DataFileEntry {
            filename: enables.into(),
            file_type: "RPF_FILE".into(),
            disabled: true,
            ..Default::default()
        }];
        content.content_change_sets = vec![ContentChangeSet {
            name: set.into(),
            files_to_enable: vec![enables.to_string().into()],
            ..Default::default()
        }];
    }
    files.push(("setup2.xml", setup.to_xml().into_bytes()));
    files.push(("content.xml", content.to_xml().into_bytes()));
    let folder = format!("x64/dlcpacks/{name}");
    std::fs::create_dir_all(root.join(&folder)).expect("mkdir");
    write_archive(root, &format!("{folder}/dlc.rpf"), &files);
}

#[test]
fn a_level_a_mode_a_kept_file_and_a_fresh_read_are_taken_from_the_command_line() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    // The town, and two packs its list names after it: the first holds no map, and the second a
    // floor ten metres over the town's, in an archive a change set of multiplayer alone enables.
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let list = DlcList {
        paths: ["alpha", "beta"]
            .iter()
            .map(|pack| format!("dlcpacks:/{pack}/").into())
            .collect(),
    };
    let manifest = manifest_bytes(&[("m_town", &["m_town_0"])], &[]);
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", list.to_xml().into_bytes()),
            ("data/_manifest.ymf", manifest),
        ],
    );
    write_town(&root, 110.0);
    let low = level_square([110.0, 110.0, 10.0], 40.0, 0x0507);
    let raised = level_square([100.0, 100.0, 20.0], 100.0, 0x0507);
    let height = |floor: &or_io::gta5::PhBoundGeometry| {
        serde_json::json!(floor.vertex_position(0).expect("a vertex")[2])
    };
    let beta_map = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    write_pack(&root, "alpha", 10, None, Vec::new());
    write_pack(
        &root,
        "beta",
        20,
        Some(("BETA_MAP", "dlc_beta:/levels.rpf")),
        vec![
            ("levels.rpf/m_beta.ymap", map_bytes("m_beta", &beta_map)),
            ("levels.rpf/m_beta_0.ybn", mesh_bound_bytes(raised.clone())),
        ],
    );
    let game = root.to_string_lossy().into_owned();
    let ask = |more: &[&str]| -> serde_json::Value {
        let args: Vec<&str> = ["ground", "--json", "--game", &game]
            .iter()
            .chain(more)
            .copied()
            .collect();
        let (printed, ok) = world(&config, &args);
        assert!(ok, "{more:?}: {printed}");
        serde_json::from_str(printed.trim()).expect("one JSON value")
    };

    // Every pack: the later pack's floor is the ground.
    let whole = ask(&["120", "130"]);
    assert_eq!(whole["ground"]["height"], height(&raised));
    assert_eq!(whole["notes"], serde_json::json!([]));
    assert_eq!(kept_under(&config).len(), 1);

    // Stopped at the pack before it: the town's floor, the pack left out named, and an index
    // kept in a file of the level's own.
    let level = ask(&["120", "130", "--level", "alpha"]);
    assert_eq!(level["ground"]["height"], height(&low));
    assert_eq!(
        level["notes"],
        serde_json::json!(["the level leaves out 1 of 2 listed pack(s) and their bounds: beta"])
    );
    assert_eq!(kept_under(&config).len(), 2, "a level keeps its own index");

    // A mode reaches the mount: story leaves out the change set multiplayer's group names, and
    // the note says which. No bounds file goes with it, and a mode keeps no index of its own.
    let story = ask(&["120", "130", "--mode", "story"]);
    let note = story["notes"][0].as_str().expect("a note");
    assert!(
        note.ends_with("; 1 change set(s) stated for the other mode: BETA_MAP"),
        "{note}"
    );
    assert_eq!(story["ground"], whole["ground"]);
    let multiplayer = ask(&["120", "130", "--mode", "multiplayer"]);
    let note = multiplayer["notes"][0].as_str().expect("a note");
    assert!(!note.contains("change set(s)"), "{note}");
    assert_eq!(kept_under(&config).len(), 2);

    // `--kept` names the file, and nothing more is kept under the configuration directory.
    let own = scratch.path().join("own.index");
    let own_flag = own.to_string_lossy().into_owned();
    let kept = ask(&["120", "130", "--level", "alpha", "--kept", &own_flag]);
    assert_eq!(kept["ground"], level["ground"]);
    assert!(own.is_file());
    assert_eq!(kept_under(&config).len(), 2);

    // The town's archive is written again with its floor two hundred metres along, at the size
    // and the time it had: a kept index cannot tell, and names the bound where it was.
    let archive = root.join("x64a.rpf");
    let before = std::fs::metadata(&archive).expect("the archive");
    write_town(&root, 310.0);
    let after = std::fs::metadata(&archive).expect("the archive");
    assert_eq!(after.len(), before.len(), "the two archives are one size");
    let time = before.modified().expect("a time");
    let file = std::fs::File::options().write(true).open(&archive);
    file.and_then(|file| file.set_modified(time))
        .expect("the time is set back");
    let moved = ["320", "130", "--level", "alpha", "--kept", &own_flag];
    let stale = ask(&moved);
    assert_eq!(stale["ground"], serde_json::json!(null));
    assert_eq!(stale["lookedIn"], serde_json::json!([]));
    // `--fresh` reads the install again whatever is kept, and keeps what it read.
    let fresh: Vec<&str> = moved.iter().chain(&["--fresh"]).copied().collect();
    let read_again = ask(&fresh);
    assert_eq!(read_again["ground"]["height"], height(&low));
    assert_eq!(read_again["lookedIn"][0]["name"], "m_town_0");
    assert_eq!(ask(&moved)["ground"], read_again["ground"]);
}

#[test]
fn where_something_is_is_asked_from_the_command_line() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    // A town that stands a bench, and two packs its list names after it: the first holds no map,
    // and the second a map that stands the same model, in an archive a change set of multiplayer
    // alone enables.
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let list = DlcList {
        paths: ["alpha", "beta"]
            .iter()
            .map(|pack| format!("dlcpacks:/{pack}/").into())
            .collect(),
    };
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", list.to_xml().into_bytes()),
            ("data/_manifest.ymf", manifest_bytes(&[], &[])),
        ],
    );
    let square = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    let in_town = Placed::new("prop_bench", [110.0, 120.0, 10.0], 90.0, 7);
    let in_beta = Placed::new("prop_bench", [150.0, 160.0, 10.0], 180.0, 8);
    let unit = cube([-1.0; 3], 2.0);
    write_archive(
        &root,
        "x64a.rpf",
        &[
            (
                "levels/m_town.ymap",
                placing_map_bytes("m_town", &square, std::slice::from_ref(&in_town)),
            ),
            (
                "types/props.ytyp",
                types_bytes("props", &[("prop_bench", unit, 80.0)]),
            ),
            ("models/prop_bench.ydr", b"a model".to_vec()),
        ],
    );
    write_pack(&root, "alpha", 10, None, Vec::new());
    write_pack(
        &root,
        "beta",
        20,
        Some(("BETA_MAP", "dlc_beta:/levels.rpf")),
        vec![(
            "levels.rpf/m_beta.ymap",
            placing_map_bytes("m_beta", &square, std::slice::from_ref(&in_beta)),
        )],
    );
    let game = root.to_string_lossy().into_owned();
    let ask = |more: &[&str]| -> serde_json::Value {
        let args: Vec<&str> = ["find", &game].iter().chain(more).copied().collect();
        let (printed, ok) = world(&config, &args);
        assert!(ok, "{more:?}: {printed}");
        serde_json::from_str(printed.trim()).expect("one JSON value")
    };
    let maps_of = |answer: &serde_json::Value| -> Vec<String> {
        let listed = answer["placements"]["listed"].as_array().expect("listed");
        let maps = listed.iter().map(|placement| placement["map"].as_str());
        maps.map(|map| map.expect("a map").to_string()).collect()
    };

    // Text that answers nothing prints its empty groups and succeeds.
    let (printed, ok) = world(&config, &["find", &game, "no_such_thing"]);
    assert!(ok, "{printed}");
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines, ["Find `no_such_thing`", "Models: 0", "Maps: 0"]);
    assert_eq!(kept_under(&config).len(), 1, "the index it built is kept");

    // Every pack: the model is placed in the town and in the later pack, nearest the view first.
    let near = ["prop_bench", "--json", "--near", "150,160,10"];
    let whole = ask(&near);
    assert_eq!(whole["models"][0]["placements"], 2);
    assert_eq!(
        whole["models"][0]["hash"],
        format!("0x{:08X}", hash("prop_bench"))
    );
    assert_eq!(maps_of(&whole), ["m_beta", "m_town"]);
    assert_eq!(whole["placements"]["listed"][0]["guid"], in_beta.guid);
    assert_eq!(whole["notes"], serde_json::json!([]));

    // Stopped at the pack before it: the town's alone, the pack left out named, and an index kept
    // in a file of the level's own.
    let level: Vec<&str> = near.iter().chain(&["--level", "alpha"]).copied().collect();
    let level = ask(&level);
    assert_eq!(level["models"][0]["placements"], 1);
    assert_eq!(maps_of(&level), ["m_town"]);
    assert_eq!(
        level["notes"],
        serde_json::json!(["the level leaves out 1 of 2 listed pack(s) and what they place: beta"])
    );
    assert_eq!(kept_under(&config).len(), 2, "a level keeps its own index");

    // A mode reaches the mount, and leaves out no map: the note names the change set it left.
    let story: Vec<&str> = near.iter().chain(&["--mode", "story"]).copied().collect();
    let story = ask(&story);
    assert_eq!(maps_of(&story), ["m_beta", "m_town"]);
    let note = story["notes"][0].as_str().expect("a note");
    assert!(
        note.starts_with("mounted as story, which leaves out no map"),
        "{note}"
    );
    assert!(
        note.ends_with("; 1 change set(s) stated for the other mode: BETA_MAP"),
        "{note}"
    );
    assert_eq!(
        kept_under(&config).len(),
        2,
        "a mode keeps no index of its own"
    );

    // `--copy` prints the copy alone: the list of the model's placements, each as the form writes
    // a place with its heading, and a place as it was typed.
    let (printed, ok) = world(&config, &["find", &game, "prop_bench", "--copy", "lua"]);
    assert!(ok, "{printed}");
    assert!(
        printed.starts_with('{') && printed.trim_end().ends_with('}'),
        "{printed}"
    );
    for placed in [&in_town, &in_beta] {
        let [x, y, z] = placed.position;
        let one = format!("vector4({x:.3}, {y:.3}, {z:.3}, {:.3})", placed.heading);
        assert!(printed.contains(&one), "{printed} lacks {one}");
    }
    let (printed, ok) = world(
        &config,
        &["find", &game, "1.5,", "-2.25,", "3", "--copy", "json"],
    );
    assert!(ok, "{printed}");
    let copied: serde_json::Value = serde_json::from_str(printed.trim()).expect("JSON");
    assert_eq!(
        copied,
        serde_json::json!({ "x": 1.5, "y": -2.25, "z": 3.0 })
    );

    // Nothing to copy fails, as do a flag the verb lacks, a form the tree lacks, a hash too wide
    // for one, no text, and a folder that is no install: each prints nothing.
    for refused in [
        &["find", &game, "no_such_thing", "--copy", "lua"][..],
        &["find", &game, "prop_bench", "--distance", "5"],
        &["find", &game, "prop_bench", "--copy", "python"],
        &["find", &game, "0x123456789"],
        &["find", &game],
        &["find", &config.to_string_lossy(), "prop_bench"],
    ] {
        let (printed, ok) = world(&config, refused);
        assert!(!ok && printed.is_empty(), "{refused:?}: {printed}");
    }
}

/// The town with a bench and a floor, and two packs its list names after it: the first holds no map, and the second a map that
/// stands the same bench under a floor ten metres over the town's.
fn install_of_two_packs(scratch: &Path) -> PathBuf {
    let root = scratch.join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let list = DlcList {
        paths: ["alpha", "beta"]
            .iter()
            .map(|pack| format!("dlcpacks:/{pack}/").into())
            .collect(),
    };
    // ⚠ No map group names the town: a group lets its maps into a view by the hour, and this one would state none.
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", list.to_xml().into_bytes()),
            ("data/_manifest.ymf", manifest_bytes(&[], &[])),
        ],
    );
    let square = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    let in_town = Placed::new("prop_bench", [110.0, 120.0, 10.0], 90.0, 7);
    let in_beta = Placed::new("prop_bench", [150.0, 160.0, 10.0], 180.0, 8);
    write_archive(
        &root,
        "x64a.rpf",
        &[
            (
                "levels/m_town.ymap",
                placing_map_bytes("m_town", &square, std::slice::from_ref(&in_town)),
            ),
            (
                "levels/m_town_0.ybn",
                mesh_bound_bytes(level_square([110.0, 110.0, 10.0], 40.0, 0x0507)),
            ),
            (
                "types/props.ytyp",
                types_bytes("props", &[("prop_bench", cube([-1.0; 3], 2.0), 80.0)]),
            ),
            ("models/prop_bench.ydr", b"a model".to_vec()),
        ],
    );
    write_pack(&root, "alpha", 10, None, Vec::new());
    write_pack(
        &root,
        "beta",
        20,
        Some(("BETA_MAP", "dlc_beta:/levels.rpf")),
        vec![
            (
                "levels.rpf/m_beta.ymap",
                placing_map_bytes("m_beta", &square, std::slice::from_ref(&in_beta)),
            ),
            (
                "levels.rpf/m_beta_0.ybn",
                mesh_bound_bytes(level_square([100.0, 100.0, 20.0], 100.0, 0x0507)),
            ),
        ],
    );
    root
}

/// One verb that reads the world index: its name, how it is asked of the install `game`, and what its answer names.
struct Asking {
    verb: &'static str,
    args: fn(&str) -> Vec<String>,
    /// Every map or bound the answer names, those it says it could not read among them.
    names: fn(&serde_json::Value) -> Vec<String>,
}

/// The `member` of each item of the list at `pointer` of `found`, in the order of their names.
fn names_at(found: &serde_json::Value, pointer: &str, member: &str) -> Vec<String> {
    let listed = found.pointer(pointer).and_then(|listed| listed.as_array());
    let listed = listed.unwrap_or_else(|| panic!("{pointer} of {found}"));
    let mut names: Vec<String> = (listed.iter())
        .map(|one| one[member].as_str().expect("a name").to_string())
        .collect();
    names.sort();
    names
}

/// The four verbs of `world` that answer from the index, each asked of the column the town and the later pack both cover.
///
/// ⚠ A verb that reads a file the index names through the mount says so when the mount lacks it, and that is counted as a name:
/// an index that held a file of another level would otherwise answer rightly by failing to open it.
const ASKINGS: [Asking; 4] = [
    Asking {
        verb: "at",
        args: |game| strings(&["at", "--json", "--game", game, "120", "130", "12"]),
        names: |found| names_at(found, "/maps", "name"),
    },
    Asking {
        verb: "ground",
        args: |game| strings(&["ground", "--json", "--game", game, "120", "130"]),
        names: |found| {
            let ground = found["ground"]["bound"]["name"].as_str().expect("ground");
            let mut names = vec![format!("ground on {ground}")];
            names.extend(names_at(found, "/lookedIn", "name"));
            names
        },
    },
    Asking {
        verb: "find",
        args: |game| strings(&["find", game, "prop_bench", "--json"]),
        names: |found| {
            let mut names = names_at(found, "/placements/listed", "map");
            let notes = found["notes"].as_array().expect("notes").iter();
            let unread = notes.filter_map(|note| note.as_str());
            names.extend(
                unread
                    .filter(|note| note.contains("gave no map"))
                    .map(str::to_string),
            );
            names
        },
    },
    Asking {
        verb: "select",
        args: |game| strings(&["select", game, "120,130,12", "--json"]),
        names: |found| {
            let mut names = names_at(found, "/maps", "name");
            let unread = names_at(found, "/unread", "name");
            names.extend(unread.iter().map(|name| format!("unread {name}")));
            names
        },
    },
];

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

#[test]
fn every_world_verb_that_reads_the_index_answers_from_the_index_of_the_level_it_is_asked_at() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    let root = install_of_two_packs(scratch.path());
    let game = root.to_string_lossy().into_owned();
    let ask = |asking: &Asking, more: &[&str]| -> Vec<String> {
        let mut args = (asking.args)(&game);
        args.extend(strings(more));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (printed, ok) = world(&config, &args);
        assert!(ok, "{} {more:?}: {printed}", asking.verb);
        let found = serde_json::from_str(printed.trim()).expect("one JSON value");
        (asking.names)(&found)
    };
    // What each verb names with every pack, and stopped at the pack before the one that holds a map.
    let with_beta = |verb: &str| match verb {
        "ground" => strings(&["ground on m_beta_0", "m_beta_0", "m_town_0"]),
        _ => strings(&["m_beta", "m_town"]),
    };
    let at_alpha = |verb: &str| match verb {
        "ground" => strings(&["ground on m_town_0", "m_town_0"]),
        _ => strings(&["m_town"]),
    };

    // ⚠ One file handed to every verb at every level: what one level kept is what the next is offered.
    let one = scratch.path().join("one.index");
    let one = one.to_string_lossy().into_owned();
    // Every verb is asked before any is judged, so a verb that answers wrongly does not hide the next.
    let wrong: std::cell::RefCell<Vec<String>> = Default::default();
    let held = |verb: &str, what: &str, said: Vec<String>, expected: Vec<String>| {
        if said != expected {
            let said = format!("{verb} {what}: {said:?}, not {expected:?}");
            wrong.borrow_mut().push(said);
        }
    };
    for asking in &ASKINGS {
        let verb = asking.verb;
        let kept = |level: &[&str]| -> Vec<String> {
            let more: Vec<&str> = level.iter().chain(&["--kept", &one]).copied().collect();
            ask(asking, &more)
        };
        held(verb, "of every pack", kept(&[]), with_beta(verb));
        let level = kept(&["--level", "alpha"]);
        let what = "at the level of alpha, from a file kept of every pack";
        held(verb, what, level, at_alpha(verb));
        let what = "of every pack, from a file kept at the level of alpha";
        held(verb, what, kept(&[]), with_beta(verb));
        let level = kept(&["--level", "ALPHA"]);
        held(verb, "at a level named in capitals", level, at_alpha(verb));
        let what = "at the last pack of the list, which is every pack";
        held(verb, what, kept(&["--level", "beta"]), with_beta(verb));
    }
    assert_eq!(kept_under(&config), Vec::<String>::new());

    // With no file named, a level's index is kept in a file of its own, the same for every verb.
    for asking in &ASKINGS {
        let verb = asking.verb;
        held(verb, "of every pack", ask(asking, &[]), with_beta(verb));
        let level = ask(asking, &["--level", "alpha"]);
        held(verb, "at the level of alpha", level, at_alpha(verb));
        held(
            verb,
            "of every pack again",
            ask(asking, &[]),
            with_beta(verb),
        );
        let files = kept_under(&config).len();
        if files != 2 {
            wrong.borrow_mut().push(format!(
                "{verb} left {files} kept file(s), not one of every pack and one of the level"
            ));
        }
    }
    let wrong = wrong.into_inner();
    assert!(wrong.is_empty(), "{wrong:#?}");
    let kept = kept_under(&config);

    // `world index` builds the index of a level, and says the maps of that level alone.
    let maps = |more: &[&str]| -> serde_json::Value {
        let args: Vec<&str> = ["index", "--json", "--game", &game]
            .iter()
            .chain(more)
            .copied()
            .collect();
        let (printed, ok) = world(&config, &args);
        assert!(ok, "{more:?}: {printed}");
        let said: serde_json::Value = serde_json::from_str(printed.trim()).expect("JSON");
        said["maps"].clone()
    };
    assert_eq!(maps(&[]), 2);
    assert_eq!(maps(&["--level", "alpha"]), 1);
    assert_eq!(maps(&["--level", "alpha", "--kept", &one]), 1);
    assert_eq!(maps(&["--kept", &one]), 2);
    assert_eq!(
        kept_under(&config),
        kept,
        "an index is kept where its verbs keep it"
    );

    // A level the install's list lacks fails the mount of every verb, and a level with no value is refused.
    for asking in &ASKINGS {
        for refused in [&["--level", "mpnone"][..], &["--level"]] {
            let mut args = (asking.args)(&game);
            args.extend(strings(refused));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let (printed, ok) = world(&config, &args);
            assert!(
                !ok && printed.is_empty(),
                "{} {refused:?}: {printed}",
                asking.verb
            );
        }
    }

    // `world new-map` makes a map from nothing: it reads no index, of any level, and keeps none.
    let other = scratch.path().join("elsewhere");
    std::fs::create_dir(&other).expect("mkdir");
    let made = scratch.path().join("m_new.ymap");
    let (printed, ok) = world(
        &other,
        &["new-map", "--name", "m_new", &made.to_string_lossy()],
    );
    assert!(ok && made.is_file(), "{printed}");
    assert_eq!(kept_under(&other), Vec::<String>::new());
}

/// A vehicle data file stating `(model, class)`, each a car.
fn vehicle_file(stated: &[(&str, &str)]) -> Vec<u8> {
    let items: String = (stated.iter())
        .map(|(model, class)| {
            format!(
                "<Item><modelName>{model}</modelName><gameName>{model}</gameName>\
                 <type>VEHICLE_TYPE_CAR</type><vehicleClass>{class}</vehicleClass></Item>"
            )
        })
        .collect();
    format!(
        "<CVehicleModelInfo__InitDataList><InitDatas>{items}</InitDatas>\
         </CVehicleModelInfo__InitDataList>"
    )
    .into_bytes()
}

#[test]
fn the_models_and_the_vehicles_of_an_install_are_listed_from_the_command_line_at_a_level() {
    use or_title::install::vehicles::{LEVEL_VEHICLES, VEHICLE_FILE_TYPE};
    use world_fixture::{dlc_list_bytes, typed_bytes, ArchetypeSpec};

    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    // A bench the town places twice, and two packs after it that each declare a model and state a vehicle.
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", dlc_list_bytes(&["alpha", "beta"])),
            ("data/_manifest.ymf", manifest_bytes(&[], &[])),
            (
                LEVEL_VEHICLES.trim_start_matches("common:/"),
                vehicle_file(&[("carone", "VC_SPORTS")]),
            ),
        ],
    );
    let unit = cube([0.0; 3], 2.0);
    let town = MapSpec::around(cube([0.0; 3], 100.0));
    let benches = [
        Placed::new("prop_bench", [1.0, 1.0, 0.0], 0.0, 1),
        Placed::new("prop_bench", [5.0, 1.0, 0.0], 0.0, 2),
    ];
    write_archive(
        &root,
        "x64a.rpf",
        &[
            (
                "types/props.ytyp",
                typed_bytes("props", &[ArchetypeSpec::base("prop_bench", unit)]),
            ),
            (
                "levels/m_town.ymap",
                placing_map_bytes("m_town", &town, &benches),
            ),
            ("models/prop_bench.ydr", b"a model".to_vec()),
        ],
    );
    for (order, pack) in ["alpha", "beta"].into_iter().enumerate() {
        let model: &'static str = if pack == "alpha" {
            "prop_alpha"
        } else {
            "prop_beta"
        };
        let registered = DataFileEntry {
            filename: format!("dlc_{pack}:/data/vehicles.meta"),
            file_type: VEHICLE_FILE_TYPE.into(),
            ..Default::default()
        };
        let (types, drawable) = (format!("types/{pack}.ytyp"), format!("models/{model}.ydr"));
        world_fixture::write_pack(
            &root,
            pack,
            10 * (order as i32 + 1),
            vec![registered],
            vec![
                (
                    types.as_str(),
                    typed_bytes(pack, &[ArchetypeSpec::base(model, unit)]),
                ),
                (drawable.as_str(), b"a model".to_vec()),
                (
                    "data/vehicles.meta",
                    vehicle_file(&[(&format!("{pack}car"), "VC_SUPER")]),
                ),
            ],
        );
    }
    let game = root.to_string_lossy().into_owned();
    let ask = |verb: &str, more: &[&str]| -> serde_json::Value {
        let args: Vec<&str> = [verb, "--json", "--game", &game]
            .iter()
            .chain(more)
            .copied()
            .collect();
        let (printed, ok) = world(&config, &args);
        assert!(ok, "{verb} {more:?}: {printed}");
        serde_json::from_str(printed.trim()).expect("one JSON value")
    };
    let rows = |answer: &serde_json::Value, key: &str| -> Vec<String> {
        let rows = answer["rows"].as_array().expect("rows").iter();
        rows.map(|row| row[key].as_str().expect("a name").to_string())
            .collect()
    };

    // Every pack: three models, of which the town places one, and three vehicles.
    let whole = ask("models", &[]);
    assert_eq!(
        rows(&whole, "name"),
        ["prop_alpha", "prop_bench", "prop_beta"]
    );
    assert_eq!((&whole["total"], &whole["models"]), (&3.into(), &3.into()));
    assert_eq!(whole["rows"][1]["placed"], benches.len());
    assert_eq!(whole["notes"], serde_json::json!([]));
    assert_eq!(kept_under(&config).len(), 1);
    // The index is kept, so the question after it opens no file, and `--fresh` reads every file again.
    let opened = |answer: &serde_json::Value| answer["timing"]["filesOpened"].as_u64();
    assert!(opened(&whole) > Some(0), "{:?}", whole["timing"]);
    assert_eq!(opened(&ask("models", &[])), Some(0));
    let fresh = ask("models", &["--fresh"]);
    assert_eq!(opened(&fresh), opened(&whole));
    assert_eq!(fresh["rows"], whole["rows"]);
    assert_eq!(rows(&ask("models", &["--placed"]), "name"), ["prop_bench"]);
    assert_eq!(
        rows(&ask("models", &["alpha", "--pack", "dlc_alpha"]), "name"),
        ["prop_alpha"]
    );
    let vehicles = ask("vehicles", &[]);
    assert_eq!(
        rows(&vehicles, "modelName"),
        ["alphacar", "betacar", "carone"]
    );
    assert_eq!(
        rows(&ask("vehicles", &["--class", "VC_SUPER"]), "modelName"),
        ["alphacar", "betacar"]
    );

    // Stopped at the first pack: the second pack's model and vehicle are left out, the pack is named, and the index
    // of the level is kept in a file of its own.
    let level = ask("models", &["--level", "alpha"]);
    assert_eq!(rows(&level, "name"), ["prop_alpha", "prop_bench"]);
    assert_eq!(level["total"], 2);
    let note = level["notes"][0].as_str().expect("a note");
    assert!(
        note.contains("leaves out 1 of 2 listed pack(s)") && note.ends_with("beta"),
        "{note}"
    );
    assert_eq!(kept_under(&config).len(), 2);
    let level = ask("vehicles", &["--level", "alpha"]);
    assert_eq!(rows(&level, "modelName"), ["alphacar", "carone"]);
    let note = level["notes"][0].as_str().expect("a note");
    assert!(
        note.contains("the vehicles they state") && note.ends_with("beta"),
        "{note}"
    );

    // A mode is said in a note of each verb, in the words of what that verb reads.
    for (verb, reads) in [("models", "type file"), ("vehicles", "vehicle data file")] {
        let answer = ask(verb, &["--mode", "story"]);
        let notes = answer["notes"].as_array().expect("notes").iter();
        let notes: Vec<&str> = notes.filter_map(|note| note.as_str()).collect();
        let said = |note: &&str| note.starts_with("mounted as ") && note.contains(reads);
        assert!(notes.iter().any(said), "{verb}: {notes:?}");
    }
    assert_eq!(kept_under(&config).len(), 2);

    // As text, the install first; and a flag the verb lacks, a count that is none and a folder that is no install fail.
    let (printed, ok) = world(&config, &["models", &game, "bench"]);
    assert!(ok && printed.starts_with("Models of "), "{printed}");
    assert!(printed.contains("\n  prop_bench  "), "{printed}");
    let (printed, ok) = world(&config, &["vehicles", &game, "--class", "VC_SPORTS"]);
    assert!(ok && printed.starts_with("Vehicles of "), "{printed}");
    assert!(
        printed.contains("\n  carone  carone  VC_SPORTS  VEHICLE_TYPE_CAR  in common"),
        "{printed}"
    );
    for refused in [
        &["models", &game, "--near", "1,2,3"][..],
        &["models", &game, "--count", "some"],
        &["models", &config.to_string_lossy()],
        &["vehicles", &game, "--kind", "base"],
        &["vehicles", &config.to_string_lossy()],
    ] {
        let (printed, ok) = world(&config, refused);
        assert!(!ok && printed.is_empty(), "{refused:?}: {printed}");
    }
}
