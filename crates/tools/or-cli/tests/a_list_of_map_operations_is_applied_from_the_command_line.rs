//! `openrage-cli maps apply` does a list of operations to the sources of a maps content exactly as `or_maps::operation::apply_all`
//! does, says what a dry run would change without writing, and refuses a list whole.
//!
//! ⚠ The command prints with `println!` and `eprintln!`, which a test cannot read back in its own process, so what it says is read
//! from a child of this test binary that calls `or_cli::run`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_maps::import::{add_files, Incoming};
use or_maps::operation::{apply_all, digest_of, At, Operation, Operations, PoseData};
use or_maps::sample::{map_file, types_file, Archetype, Entity};

const HARBOUR: &str = "harbour.metadata.src";
const TYPES: &str = "harbour_types.metadata.src";

fn cli(args: &[&str]) -> ExitCode {
    let mut all = vec!["maps".to_string(), "apply".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

/// A project in `dir` whose maps content holds a map file of three crates, the first the LOD parent of the second, and a type file.
fn a_project_with_maps(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("harbour.openrage-proj"),
        r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "harbour", "take": ["maps"] }],
  "schema": 4
}"#,
    )
    .unwrap();
    let map = map_file(
        "harbour",
        &[
            Entity {
                children: 1,
                ..Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1)
            },
            Entity {
                parent: 0,
                ..Entity::at("prop_crate_a", [14.0, 20.0, 0.5], 2)
            },
            Entity::at("prop_crate_b", [18.0, 20.0, 0.5], 3),
        ],
        ([8.0, 18.0, -1.0], [20.0, 22.0, 2.0]),
    );
    let types = types_file(
        "harbour_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        ],
    );
    let added = add_files(
        &dir.join("maps"),
        &[
            Incoming {
                name: "harbour.ymap",
                bytes: &map,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "harbour_types.ytyp",
                bytes: &types,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    dir.join("harbour.openrage-proj")
}

/// Every file under `folder`, by its path inside it.
fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let inside = path.strip_prefix(root).unwrap().to_path_buf();
                into.insert(inside, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

/// Write every file of `files` under `folder`.
fn write(folder: &Path, files: &BTreeMap<PathBuf, Vec<u8>>) {
    for (inside, bytes) in files {
        let path = folder.join(inside);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

fn pose(position: [f32; 3]) -> PoseData {
    PoseData {
        position,
        rotation: [0.0, 0.0, 0.6_f32.sin(), 0.6_f32.cos()],
        scale_xy: 1.25,
        scale_z: 1.0,
    }
}

fn a_list() -> Vec<Operation> {
    vec![
        Operation::PlaceEntity {
            source: HARBOUR.into(),
            entity: 2,
            pose: pose([240.0, -31.5, 4.0]),
            expects: None,
        },
        Operation::AddArchetype {
            source: TYPES.into(),
            name: "prop_new_crate".into(),
            asset: None,
            donor: Some(At {
                source: TYPES.into(),
                index: 1,
            }),
            expects: None,
        },
        Operation::AddEntity {
            source: HARBOUR.into(),
            archetype: "prop_new_crate".into(),
            pose: pose([-90.0, 12.0, 0.0]),
            donor: None,
            expects: None,
        },
        Operation::RemoveEntity {
            source: HARBOUR.into(),
            entity: 0,
            extents: None,
            expects: None,
        },
        Operation::RemoveArchetype {
            source: TYPES.into(),
            archetype: 0,
            expects: None,
        },
    ]
}

#[test]
fn a_list_applied_from_the_command_line_and_by_the_function_leaves_the_same_sources() {
    let by_cli = tempfile::tempdir().unwrap();
    let by_function = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(by_cli.path());
    // The same pack twice, to the byte: a pack made again would have another id for its set.
    let before = files(by_cli.path());
    write(by_function.path(), &before);

    let list = by_cli.path().join("operations.json");
    let operations = a_list();
    std::fs::write(
        &list,
        Operations::new(operations.clone()).to_json().unwrap(),
    )
    .unwrap();
    let before = files(&by_cli.path().join("maps"));

    assert_eq!(
        cli(&[text(&project), text(&list), "--dry-run"]),
        ExitCode::SUCCESS
    );
    assert!(
        files(&by_cli.path().join("maps")) == before,
        "a dry run writes nothing"
    );

    assert_eq!(cli(&[text(&project), text(&list)]), ExitCode::SUCCESS);
    let after = files(&by_cli.path().join("maps"));
    assert!(after != before, "the list changes the sources");

    apply_all(&by_function.path().join("maps"), &operations, false).unwrap();
    let same = files(&by_function.path().join("maps"));
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        same.keys().collect::<Vec<_>>()
    );
    for (inside, bytes) in &after {
        assert!(
            same.get(inside) == Some(bytes),
            "{} differs between the command line and the function",
            inside.display()
        );
    }
}

#[test]
fn a_list_with_an_operation_that_cannot_be_done_is_refused_whole_and_so_is_one_of_another_schema() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let before = files(&dir.path().join("maps"));
    let list = dir.path().join("operations.json");

    let mut operations = a_list();
    operations.push(Operation::RemoveEntity {
        source: HARBOUR.into(),
        entity: 40,
        extents: None,
        expects: None,
    });
    let good = Operations::new(a_list()).to_json().unwrap();
    std::fs::write(&list, Operations::new(operations).to_json().unwrap()).unwrap();
    assert_eq!(cli(&[text(&project), text(&list)]), ExitCode::FAILURE);
    assert!(
        files(&dir.path().join("maps")) == before,
        "a refused list changes no source"
    );

    std::fs::write(&list, good.replace("\"schema\": 1", "\"schema\": 2")).unwrap();
    assert_eq!(cli(&[text(&project), text(&list)]), ExitCode::FAILURE);
    std::fs::write(&list, "not json").unwrap();
    assert_eq!(cli(&[text(&project), text(&list)]), ExitCode::FAILURE);
    assert_eq!(
        cli(&[text(&project), text(&dir.path().join("gone.json"))]),
        ExitCode::FAILURE
    );
    assert_eq!(cli(&[text(&project)]), ExitCode::from(2));
    assert_eq!(
        cli(&[text(&project), text(&list), "--module", "none"]),
        ExitCode::FAILURE
    );
    assert!(files(&dir.path().join("maps")) == before);
}

const CHILD_ENV: &str = "OPENRAGE_MAPS_APPLY_ARGS";
const CHILD_TEST: &str = "run_maps_apply_for_a_parent";
const SEPARATOR: char = '\u{1f}';

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_maps_apply_for_a_parent() {
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

/// What `openrage-cli maps apply <args>` prints and what it says of an error, with whether it succeeded.
fn said(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .output()
        .expect("this test binary runs as a child");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn a_dry_run_names_each_operation_and_file_and_a_refusal_names_the_operation_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let before = files(&dir.path().join("maps"));
    let list = dir.path().join("operations.json");
    std::fs::write(&list, Operations::new(a_list()).to_json().unwrap()).unwrap();

    let (printed, _, succeeded) = said(&[text(&project), text(&list), "--dry-run"]);
    assert!(succeeded, "{printed}");
    for line in [
        "  1. place_entity of harbour.metadata.src\n",
        "  2. add_archetype of harbour_types.metadata.src: added at 2\n",
        "  3. add_entity of harbour.metadata.src: added at 3\n",
        "  5. remove_archetype of harbour_types.metadata.src\n",
        "would change harbour.metadata.src/metadata.xml\n",
        "would change harbour_types.metadata.src/metadata.xml\n",
        "content maps: 5 operation(s) would change 2 source(s); nothing was written\n",
    ] {
        assert!(printed.contains(line), "{line:?} in {printed}");
    }
    assert!(files(&dir.path().join("maps")) == before);

    // The refusal is the function's own, with the operation's place in the list, and says that nothing changed.
    let mut operations = a_list();
    operations.push(Operation::RemoveEntity {
        source: HARBOUR.into(),
        entity: 40,
        extents: None,
        expects: None,
    });
    let refused = apply_all(&dir.path().join("maps"), &operations, true)
        .unwrap_err()
        .to_string();
    assert!(
        refused.starts_with("operation 6 of 6, remove_entity of `harbour.metadata.src`: "),
        "{refused}"
    );
    std::fs::write(&list, Operations::new(operations).to_json().unwrap()).unwrap();
    let (printed, error, succeeded) = said(&[text(&project), text(&list)]);
    assert!(!succeeded);
    assert!(
        error.contains(&format!("error: {refused}; no source was changed")),
        "{error}"
    );
    assert!(!printed.contains("changed"), "{printed}");
    assert!(files(&dir.path().join("maps")) == before);
}

#[test]
fn a_project_with_two_maps_contents_is_asked_which_and_only_the_one_named_changes() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    // The same sources again as a second content of the project.
    write(&dir.path().join("more"), &files(&dir.path().join("maps")));
    let stated = std::fs::read_to_string(&project).unwrap();
    let both = stated.replace(
        r#"[{ "id": "maps", "path": "maps", "type": "maps" }]"#,
        r#"[{ "id": "maps", "path": "maps", "type": "maps" }, { "id": "more", "path": "more", "type": "maps" }]"#,
    );
    assert_ne!(both, stated);
    std::fs::write(&project, both).unwrap();
    let before = files(&dir.path().join("maps"));
    let list = dir.path().join("operations.json");
    std::fs::write(&list, Operations::new(a_list()).to_json().unwrap()).unwrap();

    assert_eq!(cli(&[text(&project), text(&list)]), ExitCode::FAILURE);
    assert_eq!(
        cli(&[text(&project), text(&list), "--module", "none"]),
        ExitCode::FAILURE
    );
    assert!(files(&dir.path().join("maps")) == before);
    assert!(files(&dir.path().join("more")) == before);

    assert_eq!(
        cli(&[text(&project), text(&list), "--module", "more"]),
        ExitCode::SUCCESS
    );
    assert!(
        files(&dir.path().join("maps")) == before,
        "the content that was not named is as it was"
    );
    assert!(files(&dir.path().join("more")) != before);
}

/// A list is a file a person keeps, and the source it was made for may have been edited since: an operation that states the XML its
/// source holds is done to that XML and to no other.
#[test]
fn an_operation_that_states_the_xml_of_its_source_is_refused_once_the_source_holds_another() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let folder = dir.path().join("maps");
    let list = dir.path().join("operations.json");
    let imported = std::fs::read(folder.join(HARBOUR).join("metadata.xml")).unwrap();
    let place = Operation::PlaceEntity {
        source: HARBOUR.into(),
        entity: 2,
        pose: pose([30.0, 20.0, 0.5]),
        expects: None,
    }
    .expecting(digest_of(&imported));
    // The operation that takes it back states the XML it leaves, so the two in one list leave the source as it was.
    let [back] = apply_all(&folder, std::slice::from_ref(&place), true)
        .unwrap()
        .try_into()
        .unwrap();
    let both = Operations::new(vec![place.clone(), back.inverse]);
    std::fs::write(&list, both.to_json().unwrap()).unwrap();
    let before = files(&folder);
    let (printed, error, succeeded) = said(&[text(&project), text(&list)]);
    assert!(succeeded, "{error}");
    assert!(
        printed.contains("content maps: 2 operation(s) changed 0 source(s)"),
        "{printed}"
    );
    assert!(files(&folder) == before);

    let written = Operations::new(vec![place]).to_json().unwrap();
    assert!(written.contains("\"expects\": \""), "{written}");
    std::fs::write(&list, written).unwrap();
    let (_, error, succeeded) = said(&[text(&project), text(&list)]);
    assert!(succeeded, "{error}");
    let moved = files(&folder);
    assert!(moved != before);

    // The same list a second time: its source holds what the first time wrote.
    let (printed, error, succeeded) = said(&[text(&project), text(&list)]);
    assert!(!succeeded, "{printed}");
    assert!(
        error.contains(
            "error: operation 1 of 1, place_entity of `harbour.metadata.src`: the source is not as the operation expects it"
        ) && error.contains("; no source was changed"),
        "{error}"
    );
    assert!(files(&folder) == moved);
}
