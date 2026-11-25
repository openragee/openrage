//! `openrage-cli world index` and `world at` over an install built by hand: what each prints, and
//! where the index is kept when no file is named for it.
//!
//! ⚠ The commands print with `println!`, which a test cannot read back in its own process, so each
//! runs in a child of this test binary that calls `or_cli::run` between two markers. The child is
//! given its own configuration directory, which is where an index with no `--kept` is kept.

#[path = "../../../domain/or-depends/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use world_fixture::{bound_bytes, cube, manifest_bytes, map_bytes, write_archive, MapSpec};

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
