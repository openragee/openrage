//! `openrage-cli maps doors` lists the doors of a project's map sets with their pairing and what the door rules say, and its JSON
//! lists the same doors as its text.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its own process, so what it says is read from a child of
//! this test binary that calls `or_cli::run`.

use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_maps::import::{add_files, Incoming};
use or_maps::model::Maps;
use or_maps::sample::{map_file, types_file, Archetype, Entity};

fn cli(args: &[&str]) -> ExitCode {
    let mut all = vec!["maps".to_string(), "doors".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

const DOOR: &str = "shop_door_l";
const GATE: &str = "yard_barrier_a";
const SOFA: &str = "sofa_a";

/// A project whose maps content places a double door, a gate whose archetype states the door extension with a sofa each side of
/// it, a door whose archetype no type file declares, and a model nothing names.
fn a_project_with_doors(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("shop.openrage-proj"),
        r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "shop", "take": ["maps"] }],
  "schema": 4
}"#,
    )
    .unwrap();
    let leaf = |name| Archetype::new(name, [0.0, -0.02, 0.0], [1.0, 0.02, 2.0]);
    let map = map_file(
        "shop",
        &[
            Entity::at(DOOR, [10.0, 20.0, 0.0], 1),
            Entity {
                yaw: PI,
                ..Entity::at(DOOR, [12.0, 20.0, 0.0], 2)
            },
            Entity::at(GATE, [30.0, 20.0, 0.0], 3),
            Entity::at(SOFA, [30.6, 20.6, 0.0], 4),
            Entity::at(SOFA, [30.6, 19.4, 0.0], 5),
            Entity::at("cellar_door_a", [50.0, 20.0, 0.0], 6),
            Entity::at("a_model_nothing_names", [60.0, 20.0, 0.0], 7),
        ],
        ([0.0, 10.0, -1.0], [70.0, 30.0, 3.0]),
    );
    let types = types_file(
        "shop_types",
        &[
            leaf(DOOR),
            Archetype {
                door: true,
                ..leaf(GATE)
            },
            Archetype::new(SOFA, [-0.5, -0.5, 0.0], [0.5, 0.5, 1.0]),
        ],
    );
    let folder = dir.join("maps");
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: "shop.ymap",
                bytes: &map,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "shop_types.ytyp",
                bytes: &types,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let mut model = Maps::read(&folder).unwrap();
    model.remember(&[DOOR, SOFA, "cellar_door_a"]);
    model.write(&folder).unwrap();
    dir.join("shop.openrage-proj")
}

const CHILD_ENV: &str = "OPENRAGE_MAPS_DOORS_ARGS";
const CHILD_TEST: &str = "run_maps_doors_for_a_parent";
const SEPARATOR: char = '\u{1f}';

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_maps_doors_for_a_parent() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = (args.to_string_lossy().split(SEPARATOR))
        .map(str::to_string)
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if cli(&args) != ExitCode::SUCCESS {
        std::process::exit(1);
    }
}

/// What `openrage-cli maps doors <args>` prints, with whether it succeeded.
fn said(args: &[&str]) -> (String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .output()
        .expect("this test binary runs as a child");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.success(),
    )
}

/// The JSON the command printed: the harness prints lines of its own around it.
fn json_in(printed: &str) -> serde_json::Value {
    let from = printed.find("[\n").expect("an array begins a line");
    let to = printed.rfind("\n]").expect("and ends one") + 2;
    serde_json::from_str(&printed[from..to]).unwrap_or_else(|e| panic!("{printed}: {e}"))
}

#[test]
fn the_json_lists_the_same_doors_as_the_text_with_their_pairing_and_what_the_rules_say() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_doors(dir.path());

    let (printed, succeeded) = said(&[text(&project)]);
    assert!(succeeded, "{printed}");
    let gate = format!("hash_{:08X}", or_io::string_hash(GATE.as_bytes()));
    for part in [
        "content maps: 1 map set(s)\n",
        "4 door(s) among 7 entity(ies), 1 double door(s), 1 entity(ies) whose model has no name\n",
        "door shop.metadata.src#0 `shop_door_l` ",
        "found by the name pattern `*door*`; read as swing, hinged at the low end of its X, 1.00 m wide; double with shop.metadata.src#1; anticlockwise opens 90.0 degrees; clockwise opens 90.0 degrees\n",
        "double with shop.metadata.src#0",
        &format!("door shop.metadata.src#2 `{gate}` "),
        "found by its door extension; read as swing",
        "on `sofa_a` at shop.metadata.src#3",
        "on `sofa_a` at shop.metadata.src#4",
        "door shop.metadata.src#5 `cellar_door_a` ",
        "not read: no type file handed in declares its archetype, so it has no box to read\n",
        "warning door-blocked ",
        "The game's door tuning and the collision of the map are not read.\n",
    ] {
        assert!(printed.contains(part), "{part:?} in {printed}");
    }
    let in_text: Vec<String> = (printed.lines())
        .filter_map(|line| line.trim_start().strip_prefix("door "))
        .filter_map(|rest| rest.split(' ').next())
        .map(str::to_string)
        .collect();
    assert_eq!(in_text.len(), 4, "{printed}");

    let (printed, succeeded) = said(&[text(&project), "--json"]);
    assert!(succeeded, "{printed}");
    let json = json_in(&printed);
    assert_eq!(json[0]["module"], "maps");
    let set = &json[0]["sets"][0];
    let in_json: Vec<String> = (set["doors"].as_array().expect("doors").iter())
        .map(|d| format!("{}#{}", d["source"].as_str().unwrap(), d["entity"]))
        .collect();
    assert_eq!(in_json, in_text);
    assert_eq!(set["pairs"], serde_json::json!([[0, 1]]));
    assert_eq!(set["unnamed"], 1);
    assert_eq!(set["entities"], 7);
    assert_eq!(set["colliders"], 0);
    let found: Vec<&str> = (set["doors"].as_array().unwrap().iter())
        .map(|d| d["found_by"].as_str().unwrap())
        .collect();
    assert_eq!(found, ["name", "name", "extension", "name"]);
    assert_eq!(set["doors"][2]["hash"], or_io::string_hash(GATE.as_bytes()));
    let rules: Vec<(&str, u64)> = (set["findings"].as_array().unwrap().iter())
        .map(|f| {
            (
                f["code"].as_str().unwrap(),
                f["place"]["entity"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(rules, [("door-blocked", 2)]);
}

#[test]
fn a_pack_that_remembers_no_name_finds_its_doors_by_a_list_of_names_or_by_what_an_operation_states()
{
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_doors(dir.path());
    let folder = dir.path().join("maps");
    // As an import leaves a pack: its files keep hashes, and nobody typed a name.
    let mut model = Maps::read(&folder).unwrap();
    model.names.clear();
    model.write(&folder).unwrap();
    let (printed, succeeded) = said(&[text(&project)]);
    assert!(succeeded, "{printed}");
    assert!(
        printed.contains(
            "1 door(s) among 7 entity(ies), 0 double door(s), 6 entity(ies) whose model has no name\n"
        ),
        "only the gate, by its extension: {printed}"
    );

    let names = dir.path().join("names.txt");
    std::fs::write(&names, format!("{DOOR}\n\n  cellar_door_a  \n{SOFA}\n")).unwrap();
    let (printed, succeeded) = said(&[text(&project), "--names", text(&names)]);
    assert!(succeeded, "{printed}");
    assert!(
        printed.contains(
            "4 door(s) among 7 entity(ies), 1 double door(s), 1 entity(ies) whose model has no name\n"
        ),
        "{printed}"
    );
    let (_, succeeded) = said(&[
        text(&project),
        "--names",
        text(&dir.path().join("gone.txt")),
    ]);
    assert!(!succeeded, "a list that is not there is refused");

    // An operation that adds an entity states its archetype, which the content then remembers.
    let list = dir.path().join("add.json");
    let added = or_maps::operation::Operation::AddEntity {
        source: "shop.metadata.src".into(),
        expects: None,
        archetype: DOOR.into(),
        pose: or_maps::operation::PoseData {
            position: [40.0, 20.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale_xy: 1.0,
            scale_z: 1.0,
        },
        donor: None,
    };
    let hashed = or_maps::operation::Operation::AddEntity {
        source: "shop.metadata.src".into(),
        expects: None,
        archetype: format!("hash_{:08X}", or_io::string_hash(GATE.as_bytes())),
        pose: or_maps::operation::PoseData {
            position: [44.0, 20.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale_xy: 1.0,
            scale_z: 1.0,
        },
        donor: None,
    };
    assert_eq!(added.names(), [DOOR]);
    assert_eq!(
        hashed.names(),
        [] as [&str; 0],
        "a hash spelled out is no name"
    );
    let operations = or_maps::operation::Operations::new(vec![added, hashed]);
    std::fs::write(&list, operations.to_json().unwrap()).unwrap();
    let dry = ["maps", "apply", text(&project), text(&list), "--dry-run"].map(str::to_string);
    assert_eq!(or_cli::run(&dry), ExitCode::SUCCESS);
    assert_eq!(
        Maps::read(&folder).unwrap().names,
        [] as [&str; 0],
        "a dry run writes nothing"
    );
    assert_eq!(or_cli::run(&dry[..4]), ExitCode::SUCCESS);
    assert_eq!(Maps::read(&folder).unwrap().names, [DOOR]);
    let (printed, succeeded) = said(&[text(&project)]);
    assert!(succeeded, "{printed}");
    assert!(
        printed.contains("5 door(s) among 9 entity(ies), 1 double door(s), "),
        "the two leaves, the one added, the gate and the gate added: {printed}"
    );
}

#[test]
fn a_project_with_no_maps_content_and_a_content_that_is_not_there_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_doors(dir.path());
    assert_eq!(cli(&[]), ExitCode::from(2));
    let bare = dir.path().join("bare.openrage-proj");
    std::fs::write(
        &bare,
        r#"{ "documents": [], "game": "both", "modules": [], "open": [], "outputs": [], "schema": 4 }"#,
    )
    .unwrap();
    assert_eq!(cli(&[text(&bare)]), ExitCode::FAILURE);
    assert_eq!(
        cli(&[text(&project), "--module", "none"]),
        ExitCode::FAILURE
    );
    assert_eq!(cli(&[text(&project)]), ExitCode::SUCCESS);
}
