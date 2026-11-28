//! `openrage-cli maps doors` lists the doors of a project's map sets with their pairing and what the door rules say, and its JSON
//! lists the same doors as its text. With `--game` it lists the doors of one map file of an install, here one built by hand that
//! holds the map the project holds, as it lists them of the project.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its own process, so what it says is read from a child of
//! this test binary that calls `or_cli::run`.

#[path = "../../../domain/or-world-gta5/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::collections::BTreeMap;
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

/// A map file and the type file of what it places: a double door, a gate whose archetype states the door extension with a sofa
/// each side of it, a door whose archetype no type file declares, and a model nothing names.
fn the_shop() -> (Vec<u8>, Vec<u8>) {
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
    (map, types)
}

/// The models the project of [`a_project_with_doors`] remembers the names of.
const REMEMBERED: [&str; 3] = [DOOR, SOFA, "cellar_door_a"];

/// A project whose maps content holds [`the_shop`].
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
    let (map, types) = the_shop();
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
    model.remember(&REMEMBERED);
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

/// The logical path of the map file of [`an_install_with_the_shop`].
const SHOP_MAP: &str = "platform:/levels/shop.ymap";

/// An install whose one archive of levels holds [`the_shop`], with a model file for each name the project remembers.
fn an_install_with_the_shop(dir: &Path) -> PathBuf {
    let root = dir.join("install");
    std::fs::create_dir(&root).unwrap();
    let (map, types) = the_shop();
    let manifest = world_fixture::manifest_bytes(&[], &[]);
    world_fixture::write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    let models: Vec<String> = (REMEMBERED.iter())
        .map(|name| format!("models/{name}.ydr"))
        .collect();
    let mut files = vec![("levels/shop.ymap", map), ("types/shop_types.ytyp", types)];
    files.extend(
        models
            .iter()
            .map(|path| (path.as_str(), b"a model".to_vec())),
    );
    world_fixture::write_archive(&root, "x64a.rpf", &files);
    root
}

/// What `openrage-cli maps doors <args>` prints with `config` as its configuration directory and no schema snapshot, with whether
/// it succeeded.
fn said_of_an_install(config: &Path, args: &[&str]) -> (String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_SCHEMA", "none")
        .env("OPENRAGE_CONFIG_DIR", config)
        .output()
        .expect("this test binary runs as a child");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.success(),
    )
}

/// Every file under `folder`, by its path, with its bytes and when it was last written.
fn files_under(folder: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut found = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(dir) = folders.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let written = std::fs::metadata(&path).unwrap().modified().unwrap();
                found.insert(path.clone(), (std::fs::read(&path).unwrap(), written));
            }
        }
    }
    found
}

/// The lines a listing says of its one set, the header of the listing left out.
fn set_lines(printed: &str) -> Vec<String> {
    let indented = printed.lines().filter(|line| line.starts_with("  "));
    indented.map(str::to_string).collect()
}

#[test]
fn the_doors_of_a_map_file_of_an_install_are_listed_as_those_of_the_same_map_in_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_doors(dir.path());
    let root = an_install_with_the_shop(dir.path());
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let kept = dir.path().join("world.index");
    let install_as_it_was = files_under(&root);
    let of_game = |more: &[&str]| {
        let mut args = vec![
            "--game",
            text(&root),
            "--map",
            SHOP_MAP,
            "--kept",
            text(&kept),
        ];
        args.extend(more);
        said_of_an_install(&config, &args)
    };

    let (of_project, succeeded) = said(&[text(&project)]);
    assert!(succeeded, "{of_project}");
    let (of_install, succeeded) = of_game(&[]);
    assert!(succeeded, "{of_install}");
    let canonical = std::fs::canonicalize(&root).unwrap();
    assert!(
        of_install.contains(&format!(
            "install {}: map {SHOP_MAP}\n",
            canonical.display()
        )),
        "{of_install}"
    );
    assert!(
        of_install.contains("The game's door tuning and the collision of the map are not read.\n"),
        "{of_install}"
    );
    // The set, its doors and its findings, line for line, once the project's source is called as the install calls its file.
    let as_the_install = of_project.replace("shop.metadata.src", SHOP_MAP);
    let (expected, listed) = (set_lines(&as_the_install), set_lines(&of_install));
    assert_eq!(
        listed.len(),
        6,
        "a set, four doors and a finding: {of_install}"
    );
    assert!(
        listed[0].ends_with(
            ": 4 door(s) among 7 entity(ies), 1 double door(s), 1 entity(ies) whose model has no name"
        ),
        "{of_install}"
    );
    assert_eq!(listed[0], "  set shop: 4 door(s) among 7 entity(ies), 1 double door(s), 1 entity(ies) whose model has no name");
    assert_eq!(listed[1..], expected[1..]);

    // As data: the same set, under the install and the map in the place of the content.
    let (of_project, succeeded) = said(&[text(&project), "--json"]);
    assert!(succeeded, "{of_project}");
    let (of_install, succeeded) = of_game(&["--json"]);
    assert!(succeeded, "{of_install}");
    let expected = json_in(&of_project.replace("shop.metadata.src", SHOP_MAP));
    let listed = json_in(&of_install);
    assert_eq!(listed[0]["game"], canonical.display().to_string());
    assert_eq!(listed[0]["map"], SHOP_MAP);
    assert_eq!(listed[0]["sets"].as_array().map(Vec::len), Some(1));
    let mut set = listed[0]["sets"][0].clone();
    let mut of_the_project = expected[0]["sets"][0].clone();
    assert_eq!(set["name"], "shop");
    assert_eq!(set["doors"][0]["source"], SHOP_MAP);
    // A set of a content has an id and a name of its own, and an install's map has neither.
    for named in ["set", "name"] {
        set.as_object_mut().unwrap().remove(named);
        of_the_project.as_object_mut().unwrap().remove(named);
    }
    let id = expected[0]["sets"][0]["set"].as_str().unwrap();
    let id_here = listed[0]["sets"][0]["set"].as_str().unwrap();
    let set: serde_json::Value =
        serde_json::from_str(&set.to_string().replace(id_here, id)).unwrap();
    assert_eq!(set, of_the_project);

    // A name the install lists no file for comes from the list `--names` gives.
    let names = dir.path().join("names.txt");
    std::fs::write(&names, format!("{GATE}\n")).unwrap();
    let (named, succeeded) = of_game(&["--names", text(&names)]);
    assert!(succeeded, "{named}");
    assert!(
        named.contains(&format!("  door {SHOP_MAP}#2 `{GATE}` ")),
        "{named}"
    );

    // ⚠ The install is read and never written: every file of it is as it was, and the one file made is the index kept.
    assert!(files_under(&root) == install_as_it_was);
    assert!(kept.is_file());
    assert_eq!(
        std::fs::read_dir(&config).unwrap().count(),
        0,
        "nothing is kept where `--kept` names the file"
    );
}

#[test]
fn a_map_the_install_lacks_and_an_install_flag_with_no_install_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_doors(dir.path());
    let root = an_install_with_the_shop(dir.path());
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let kept = dir.path().join("world.index");
    let refused = |args: &[&str]| {
        let (printed, succeeded) = said_of_an_install(&config, args);
        assert!(!succeeded, "{args:?}: {printed}");
        assert!(!printed.contains("door(s) among"), "{args:?}: {printed}");
    };
    let (game, kept) = (text(&root), text(&kept));
    // A map the install does not hold, a type file named as a map, and no map named.
    refused(&[
        "--game",
        game,
        "--kept",
        kept,
        "--map",
        "platform:/levels/none.ymap",
    ]);
    refused(&[
        "--game",
        game,
        "--kept",
        kept,
        "--map",
        "platform:/types/shop_types.ytyp",
    ]);
    refused(&["--game", game, "--kept", kept]);
    // An install and a project, or a content of one, at once.
    refused(&[
        text(&project),
        "--game",
        game,
        "--kept",
        kept,
        "--map",
        SHOP_MAP,
    ]);
    refused(&[
        "--game", game, "--kept", kept, "--map", SHOP_MAP, "--module", "maps",
    ]);
    // What says something of an install, with none named.
    refused(&[text(&project), "--map", SHOP_MAP]);
    refused(&[text(&project), "--kept", kept]);
    refused(&[text(&project), "--fresh"]);
    // A folder that is no install.
    refused(&["--game", text(&config), "--kept", kept, "--map", SHOP_MAP]);
    // The map named in another case is the same file.
    let shouted = SHOP_MAP.to_ascii_uppercase();
    let (printed, succeeded) = said_of_an_install(
        &config,
        &["--game", game, "--kept", kept, "--map", &shouted],
    );
    assert!(succeeded, "{printed}");
    assert!(printed.contains(&format!("map {SHOP_MAP}\n")), "{printed}");
}

#[test]
fn a_door_an_interior_holds_is_read_by_the_type_file_that_declares_it_which_the_map_names_nowhere()
{
    use world_fixture::{cube, placing_map_bytes, write_archive, MapSpec};
    const BANK: &str = "platform:/levels/bank.ymap";
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("install");
    std::fs::create_dir(&root).unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    // A map that places an interior and nothing else; the interior holds a door, which a second type file declares.
    let stands = [500.0, 600.0, 30.0];
    let bank = MapSpec {
        interiors: vec!["bank_interior"],
        ..MapSpec::around(cube(stands, 20.0))
    };
    let leaf = Archetype::new("vault_door", [0.0, -0.02, 0.0], [1.0, 0.02, 2.0]);
    let manifest = world_fixture::manifest_bytes(&[], &[]);
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/bank.ymap", placing_map_bytes("bank", &bank, &[])),
            (
                "types/bank_interior.ytyp",
                world_fixture::interior_types_bytes(
                    "bank_interior",
                    "bank_interior",
                    "vault_door",
                    [1.0, 2.0, 0.5],
                ),
            ),
            ("types/bank_props.ytyp", types_file("bank_props", &[leaf])),
            ("models/vault_door.ydr", b"a model".to_vec()),
        ],
    );
    let kept = dir.path().join("world.index");
    let (printed, succeeded) = said_of_an_install(
        &config,
        &[
            "--game",
            text(&root),
            "--map",
            BANK,
            "--kept",
            text(&kept),
            "--json",
        ],
    );
    assert!(succeeded, "{printed}");
    let set = &json_in(&printed)[0]["sets"][0];
    assert_eq!(
        set["entities"], 2,
        "the interior and the door it holds: {set}"
    );
    assert_eq!(set["unread"], serde_json::Value::Null, "{set}");
    let doors = set["doors"].as_array().expect("doors");
    assert_eq!(doors.len(), 1, "{set}");
    let door = &doors[0];
    assert_eq!(door["source"], BANK);
    assert_eq!((&door["entity"], &door["inside"]), (&0.into(), &0.into()));
    assert_eq!(door["model"], "vault_door");
    assert_eq!(door["found_by"], "name");
    // Where the interior stands, and where the door stands in it.
    let expected = [stands[0] + 1.0, stands[1] + 2.0, stands[2] + 0.5];
    assert_eq!(door["position"], serde_json::json!(expected));
    // ⚠ Its box is in the type file of the props, which no entity of the map places a model of.
    assert_eq!(door["leaf"]["motion"], "swing", "{door}");
    assert_eq!(door["leaf"]["width"], serde_json::json!(1.0));
    assert_eq!(door["unread"], serde_json::Value::Null);
}
