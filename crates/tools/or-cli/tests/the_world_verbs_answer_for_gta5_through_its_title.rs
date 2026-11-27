//! `openrage-cli world index`, `at`, `ground`, `find` and `select` with `--neutral` over a GTA V
//! install built by hand: the install is found through the registry of titles, opened through
//! GTA V's title, and each verb answers by the world contract what GTA V's own verb answers.
//!
//! ⚠ The commands print with `println!`, which a test cannot read back in its own process, so each
//! runs in a child of this test binary that calls `or_cli::run` between two markers. The child is
//! given its own configuration directory, which is where the index is kept.

#[path = "../../../domain/or-depends/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use world_fixture::{
    cube, dlc_list_bytes, level_square, manifest_bytes, mesh_bound_bytes, placing_map_bytes,
    write_archive, write_pack, MapSpec, Placed,
};

const CHILD_ENV: &str = "OPENRAGE_WORLD_NEUTRAL_ARGS";
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

/// What `openrage-cli world <args>` prints with `config` as its configuration directory, what it says of an error, and whether it succeeded.
fn world(config: &Path, args: &[&str]) -> (String, String, bool) {
    let args: Vec<&str> = ["world"].iter().chain(args).copied().collect();
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_SCHEMA", "none")
        .env("OPENRAGE_CONFIG_DIR", config)
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let errors = String::from_utf8_lossy(&output.stderr).into_owned();
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}"));
    let (printed, tail) = rest
        .split_once(END)
        .unwrap_or_else(|| panic!("the command panicked: {rest}{errors}"));
    (printed.to_string(), errors, tail.starts_with('1'))
}

/// The height of the floor under the town.
const FLOOR: f32 = 10.0;

/// An install under `scratch`: a town of two benches no type file declares, over a floor, and a pack `beta` that adds a map 2,000 m east.
fn install(scratch: &Path) -> PathBuf {
    let root = scratch.join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let list = dlc_list_bytes(&["beta"]);
    // ⚠ No map group names the town: a group that states no hour and no weather lets its maps in only when asked.
    let manifest = manifest_bytes(&[], &[]);
    write_archive(
        &root,
        "common.rpf",
        &[("data/dlclist.xml", list), ("data/_manifest.ymf", manifest)],
    );
    let town = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    let benches = [
        Placed::new("bench", [120.0, 120.0, FLOOR], 0.0, 1),
        Placed::new("bench", [140.0, 120.0, FLOOR], 90.0, 2),
    ];
    write_archive(
        &root,
        "x64a.rpf",
        &[
            (
                "levels/m_town.ymap",
                placing_map_bytes("m_town", &town, &benches),
            ),
            (
                "levels/m_town_0.ybn",
                mesh_bound_bytes(level_square([110.0, 110.0, FLOOR], 40.0, 0x0507)),
            ),
        ],
    );
    let far = MapSpec::around(cube([2100.0, 100.0, 0.0], 100.0));
    let bench = [Placed::new("bench", [2120.0, 120.0, FLOOR], 0.0, 3)];
    let beta = placing_map_bytes("m_beta", &far, &bench);
    write_pack(
        &root,
        "beta",
        10,
        Vec::new(),
        vec![("levels/m_beta.ymap", beta)],
    );
    root
}

/// The number before ` instance(s) selected` in what a select verb printed.
fn selected(printed: &str) -> Option<usize> {
    let line = printed
        .lines()
        .find(|l| l.contains(" instance(s) selected"))?;
    line.split_whitespace().next()?.parse().ok()
}

#[test]
fn a_gta5_install_is_found_through_the_registry_and_indexed_through_its_title() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    let root = install(scratch.path());
    let game = root.to_string_lossy();

    let (printed, errors, ok) = world(&config, &["index", &game, "--neutral"]);
    assert!(ok, "{printed}{errors}");
    let lines: Vec<&str> = printed.lines().collect();
    assert!(lines[0].ends_with("(gta5, gta-v-legacy)"), "{printed}");
    assert!(lines[1].ends_with("none was kept, so every archive was read"));
    assert_eq!(lines[2], "  3 archive(s), 3 read");
    assert!(lines[3].starts_with("  2 map(s): "), "{printed}");
    assert_eq!(
        lines[4],
        "  it states: a height of the ground, a collision query, placement rows, a weather gate"
    );
    assert_eq!(
        lines[5..],
        [
            "  option 1  DLC pack: a name of the install's",
            "  option 2  Mode: 0 (Story), 1 (Multiplayer)"
        ]
    );
    // The index is kept under the title's own folder, and a second start reads no archive.
    let kept = std::fs::read_dir(config.join("worlds").join("gta5")).expect("the kept folder");
    assert_eq!(kept.count(), 1);
    let (printed, _, ok) = world(&config, &["index", "--neutral", "--game", &game]);
    assert!(
        ok && printed.contains("  3 archive(s), 0 read"),
        "{printed}"
    );

    // At the level of no pack past the base there is no such level; at `beta` every map; a mode
    // by its label; and each world keeps a file of its own.
    let level = ["index", &game, "--neutral", "--option", "1=beta"];
    let (printed, errors, ok) = world(&config, &level);
    assert!(ok && printed.contains("  2 map(s): "), "{printed}{errors}");
    let mode = ["index", &game, "--neutral", "--option", "2=multiplayer"];
    let (printed, errors, ok) = world(&config, &mode);
    assert!(ok && printed.contains("  2 map(s): "), "{printed}{errors}");
    let kept = std::fs::read_dir(config.join("worlds").join("gta5")).expect("the kept folder");
    assert_eq!(kept.count(), 3);

    // An option the title lacks, a choice it lacks and a pack the list lacks are refused, and print nothing.
    for (refused, why) in [
        ("9=x", "the title has no option 9"),
        ("2=online", "option 2 has no choice `online`"),
        ("1=mpnone", "mpnone"),
    ] {
        let args = ["index", &game, "--neutral", "--option", refused];
        let (printed, errors, ok) = world(&config, &args);
        assert!(!ok && printed.is_empty(), "{refused}: {printed}");
        assert!(errors.contains(why), "{refused}: {errors}");
    }
}

#[test]
fn each_neutral_verb_answers_of_a_gta5_install_what_gta5s_own_verb_answers() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).expect("mkdir");
    let root = install(scratch.path());
    let game = root.to_string_lossy();

    // The same instances as GTA V's own verb selects: both benches in the town, one by the far map, none elsewhere.
    for (place, instances) in [("130,120,12", 2), ("2120,130,12", 1), ("-9000,-9000,0", 0)] {
        let (own, errors, ok) = world(&config, &["select", &game, place]);
        assert!(ok, "{own}{errors}");
        let (neutral, errors, ok) = world(&config, &["select", &game, place, "--neutral"]);
        assert!(ok, "{neutral}{errors}");
        assert_eq!(selected(&own), Some(instances), "{own}");
        assert_eq!(selected(&neutral), Some(instances), "{neutral}");
    }
    let (printed, _, ok) = world(&config, &["select", &game, "130,120,12", "--neutral"]);
    assert!(ok, "{printed}");
    assert!(
        printed.contains("  map       m_town  the view is within its box"),
        "{printed}"
    );
    let turned = printed
        .lines()
        .filter(|line| line.contains("at (140, 120, 10)"));
    assert_eq!(turned.count(), 1, "{printed}");

    let (printed, _, ok) = world(&config, &["at", &game, "130,120,12", "--neutral"]);
    assert!(ok, "{printed}");
    assert_eq!(
        printed.lines().collect::<Vec<_>>(),
        [
            "At (130, 120, 12), 12 h: 1 map(s) stream by their box",
            "  map       m_town  always (in)"
        ]
    );

    // The floor from far above, from just above, and from under it, where a probe that rises nothing finds none.
    let floor = format!("Ground under (120, 130): {FLOOR} (the surface faces 0, 0, 1)");
    for (place, said) in [
        (&["120", "130"][..], floor.as_str()),
        (&["120", "130", "14"], floor.as_str()),
        (&["120", "130", "4"], "Ground under (120, 130): none"),
        (&["900", "900"], "Ground under (900, 900): none"),
    ] {
        let args: Vec<&str> = (["ground", &game, "--neutral"].iter())
            .chain(place)
            .copied()
            .collect();
        let (printed, errors, ok) = world(&config, &args);
        assert!(ok, "{printed}{errors}");
        assert_eq!(printed.trim_end(), said, "{place:?}");
    }

    let (printed, _, ok) = world(&config, &["find", &game, "TOWN", "--neutral"]);
    assert!(ok, "{printed}");
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines[0], "`TOWN`: 1 map(s)");
    assert!(
        lines[1].starts_with("  map       m_town  0x") && lines[1].ends_with("  places 2"),
        "{printed}"
    );

    // One number is no place, and a flag of GTA V's own verb is none of the neutral one.
    for refused in [
        &["select", &game, "130", "--neutral"][..],
        &["ground", &game, "130", "--neutral"],
        &["at", &game, "130,120,12", "--neutral", "--level", "beta"],
        &["find", &game, "--neutral"],
    ] {
        let (printed, _, ok) = world(&config, refused);
        assert!(!ok && printed.is_empty(), "{refused:?}: {printed}");
    }
}
