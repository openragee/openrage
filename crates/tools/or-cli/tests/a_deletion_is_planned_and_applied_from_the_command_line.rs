//! `openrage-cli maps delete` prints the plan `or_maps::delete::plan` makes and writes nothing, writes with `--apply` what
//! `or_maps::delete::apply` writes, and refuses an entity that is not there with every source as it was; `maps changes` then reads
//! the deletion as what it took out.
//!
//! ⚠ The command prints with `println!` and `eprintln!`, which a test cannot read back in its own process, so what it says is read
//! from a child of this test binary that calls `or_cli::run`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_maps::import::{add_files, Incoming};
use or_maps::operation::At;
use or_maps::sample::{child_map_file, map_file, Entity};

const FAR: &str = "block_far.metadata.src";
const NEAR: &str = "block.metadata.src";

fn cli(args: &[&str]) -> ExitCode {
    maps("delete", args)
}

fn maps(verb: &str, args: &[&str]) -> ExitCode {
    let mut all = vec!["maps".to_string(), verb.to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

/// An entity under the entity `parent` of the parent map.
fn under<'a>(archetype: &'a str, position: [f32; 3], guid: u32, parent: i32) -> Entity<'a> {
    Entity {
        parent,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    }
}

/// A project in `dir` whose maps content holds two map files: `block_far`, with the far model of a tower and the far model of a row
/// of two houses, and `block`, the map under it, with the tower, the two houses, and two lamps at one pose.
fn a_project_with_maps(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("block.openrage-proj"),
        r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "block", "take": ["maps"] }],
  "schema": 4
}"#,
    )
    .unwrap();
    let far = map_file(
        "block_far",
        &[
            Entity {
                children: 1,
                ..Entity::at("tower_far", [10.0, 20.0, 0.5], 1)
            },
            Entity {
                children: 2,
                ..Entity::at("row_far", [30.0, 20.0, 0.5], 2)
            },
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let near = child_map_file(
        "block",
        "block_far",
        &[
            under("tower", [10.0, 20.0, 0.5], 11, 0),
            under("house", [28.0, 20.0, 0.5], 12, 1),
            under("house", [32.0, 20.0, 0.5], 13, 1),
            Entity::at("lamp", [5.0, 5.0, 0.0], 14),
            Entity::at("lamp", [5.0, 5.0, 0.0], 15),
        ],
        ([0.0; 3], [100.0; 3]),
    );
    let added = add_files(
        &dir.join("maps"),
        &[
            Incoming {
                name: "block_far.ymap",
                bytes: &far,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "block.ymap",
                bytes: &near,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    dir.join("block.openrage-proj")
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

const CHILD_ENV: &str = "OPENRAGE_MAPS_DELETE_ARGS";
/// The verb of `maps` the child runs, where it is not `delete`.
const CHILD_VERB: &str = "OPENRAGE_MAPS_VERB";
const CHILD_TEST: &str = "run_maps_delete_for_a_parent";
const SEPARATOR: char = '\u{1f}';

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_maps_delete_for_a_parent() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = (args.to_string_lossy().split(SEPARATOR))
        .map(str::to_string)
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let verb = std::env::var(CHILD_VERB).unwrap_or_else(|_| "delete".to_string());
    if maps(&verb, &args) != ExitCode::SUCCESS {
        std::process::exit(1);
    }
}

/// What `openrage-cli maps delete <args>` prints and what it says of an error, with whether it succeeded.
fn said(args: &[&str]) -> (String, String, bool) {
    said_by("delete", args)
}

/// [`said`], of the verb `verb` of `maps`.
fn said_by(verb: &str, args: &[&str]) -> (String, String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env(CHILD_VERB, verb)
        .output()
        .expect("this test binary runs as a child");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn the_plan_printed_names_what_goes_what_stays_and_each_file_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let before = files(&dir.path().join("maps"));
    let tower = format!("{NEAR}:0");
    let house = format!("{NEAR}:1");

    let (printed, error, succeeded) = said(&[text(&project), &tower, &house]);
    assert!(succeeded, "{printed}{error}");
    let hash = |name: &str| or_io::string_hash(name.as_bytes());
    let shown = |name: &str| format!("hash_{:08X}", hash(name));
    for line in [
        "content maps: 2 entities named, 3 entities to take out\n".to_string(),
        format!("  goes   {NEAR} entity 0, {} at 10, 20, 0.5: chosen\n", shown("tower")),
        format!("  goes   {NEAR} entity 1, {} at 28, 20, 0.5: chosen\n", shown("house")),
        format!(
            "  goes   {FAR} entity 0, {} at 10, 20, 0.5: left standing for nothing\n",
            shown("tower_far")
        ),
        format!(
            "  stays  {FAR} entity 1, {} at 30, 20, 0.5: 1 entity is still under it; its count of children goes from 2 to 1\n",
            shown("row_far")
        ),
        format!(
            "would change {FAR}/metadata.xml: 1 entity taken out (0), the count of children of entity 1 lowered from 2 to 1\n"
        ),
        format!(
            "would change {NEAR}/metadata.xml: 2 entities taken out (0, 1), 1 LOD link into the parent map renumbered\n"
        ),
        "  stays  the collision and the occluders of what goes: a deletion does not reach them\n".to_string(),
        "or hide by script, which changes no file and takes no collision away (Lua, within 1 m):\n".to_string(),
        format!(
            "  CreateModelHide(10.000, 20.000, 0.500, 1.000, {}, true) -- {}\n",
            hash("tower"),
            shown("tower")
        ),
        "content maps: 4 operations would change 2 sources; nothing was written (write it with --apply)\n".to_string(),
    ] {
        assert!(printed.contains(&line), "{line:?} in {printed}");
    }
    assert!(
        files(&dir.path().join("maps")) == before,
        "a plan writes nothing"
    );

    // The form and the reach of the script line are the ones asked for.
    let (printed, _, succeeded) = said(&[
        text(&project),
        &tower,
        "--script",
        "csharp",
        "--radius",
        "0.25",
    ]);
    assert!(succeeded);
    let line = format!(
        "  API.CreateModelHide(10.000f, 20.000f, 0.500f, 0.250f, {}u, true); // {}\n",
        hash("tower"),
        shown("tower")
    );
    assert!(printed.contains(&line), "{line:?} in {printed}");
    assert!(printed.contains("(C#, within 0.25 m)"), "{printed}");
    assert!(files(&dir.path().join("maps")) == before);
}

#[test]
fn the_changes_of_a_pack_name_what_a_deletion_took_out_and_the_links_a_removal_alone_left_on_other_entities(
) {
    let counts = |printed: &str, source: &str| -> String {
        let line = (printed.lines())
            .find(|line| line.starts_with(&format!("  {source} (")))
            .unwrap_or_else(|| panic!("{source} in {printed}"));
        line.split_once("): ").expect("its counts").1.to_string()
    };

    // The tower deleted with its chain: its far model goes with it, and the links of the two houses are renumbered to the far
    // model they named, which is no change of theirs.
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let (printed, error, succeeded) = said(&[text(&project), &format!("{NEAR}:0"), "--apply"]);
    assert!(succeeded, "{printed}{error}");
    let (printed, error, succeeded) = said_by("changes", &[text(&project)]);
    assert!(succeeded, "{printed}{error}");
    assert_eq!(
        counts(&printed, NEAR),
        "0 added, 1 deleted, 0 changed, 0 ambiguous, 4 unchanged",
        "{printed}"
    );
    assert_eq!(
        counts(&printed, FAR),
        "0 added, 1 deleted, 0 changed, 0 ambiguous, 1 unchanged",
        "{printed}"
    );
    assert!(!printed.contains(" parent "), "{printed}");

    // The far model of the tower taken out alone: the map under it holds the bytes it held, and each of its three links is on
    // another entity, the two of the houses past the end among them.
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let alone = or_maps::operation::Operation::RemoveEntity {
        source: FAR.into(),
        expects: None,
        entity: 0,
        extents: None,
    };
    or_maps::operation::apply(&dir.path().join("maps"), &alone).unwrap();
    let (printed, error, succeeded) = said_by("changes", &[text(&project)]);
    assert!(succeeded, "{printed}{error}");
    assert_eq!(
        counts(&printed, NEAR),
        "0 added, 0 deleted, 3 changed, 0 ambiguous, 2 unchanged",
        "{printed}"
    );
    assert!(printed.contains(" parent 0 is another entity"), "{printed}");
}

#[test]
fn a_deletion_applied_from_the_command_line_and_by_the_function_leaves_the_same_sources() {
    let by_cli = tempfile::tempdir().unwrap();
    let by_function = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(by_cli.path());
    // The same pack twice, to the byte: a pack made again would have another id for its set.
    write(by_function.path(), &files(by_cli.path()));
    let before = files(&by_cli.path().join("maps"));
    let tower = format!("{NEAR}:0");

    let (printed, error, succeeded) = said(&[text(&project), &tower, "--apply"]);
    assert!(succeeded, "{printed}{error}");
    for line in [
        format!("changed {FAR}/metadata.xml: 1 entity taken out (0)\n"),
        format!(
            "changed {NEAR}/metadata.xml: 1 entity taken out (0), 2 LOD links into the parent map renumbered\n"
        ),
        "content maps: 3 operations changed 2 sources\n".to_string(),
    ] {
        assert!(printed.contains(&line), "{line:?} in {printed}");
    }
    assert!(!printed.contains("would change"), "{printed}");
    let after = files(&by_cli.path().join("maps"));
    assert!(after != before, "the deletion changes the sources");

    let folder = by_function.path().join("maps");
    let chosen = [At {
        source: NEAR.to_string(),
        index: 0,
    }];
    let planned = or_maps::delete::plan(&folder, &chosen).unwrap();
    or_maps::delete::apply(&folder, &planned).unwrap();
    let same = files(&folder);
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
fn an_entity_that_is_not_there_is_refused_and_one_with_a_twin_is_said_and_left() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let before = files(&dir.path().join("maps"));
    let tower = format!("{NEAR}:0");
    let past = format!("{NEAR}:5");

    let (printed, error, succeeded) = said(&[text(&project), &tower, &past, "--apply"]);
    assert!(!succeeded);
    assert!(
        error.contains(&format!(
            "error: content `maps`: `{NEAR}` holds 5 entities, so there is no entity 5; no source was changed"
        )),
        "{error}"
    );
    assert!(!printed.contains("changed"), "{printed}");
    assert!(files(&dir.path().join("maps")) == before);

    assert_eq!(
        cli(&[text(&project), "harbour.metadata.src:0", "--apply"]),
        ExitCode::FAILURE
    );
    // What names no entity, no form or no reach is a mistake of usage.
    for wrong in [
        vec![text(&project)],
        vec![text(&project), NEAR],
        vec![text(&project), "block.metadata.src:first"],
        vec![text(&project), ":3"],
        vec![text(&project), &tower, "--script", "python"],
        vec![text(&project), &tower, "--radius", "0"],
        vec![text(&project), &tower, "--radius", "-1"],
        vec![text(&project), &tower, "--radius", "inf"],
        vec![text(&project), &tower, "--radius", "nan"],
        vec![text(&project), &tower, "--radius", "wide"],
        vec![text(&project), &tower, "--force"],
    ] {
        assert_eq!(cli(&wrong), ExitCode::from(2), "{wrong:?}");
    }
    assert_eq!(
        cli(&[text(&project), &tower, "--module", "none", "--apply"]),
        ExitCode::FAILURE
    );
    assert!(files(&dir.path().join("maps")) == before);

    // A lamp with a twin: the plan says so, takes nothing out, and applying it writes nothing.
    let lamp = format!("{NEAR}:3");
    let (printed, error, succeeded) = said(&[text(&project), &lamp, "--apply"]);
    assert!(succeeded, "{printed}{error}");
    let left = format!(
        "  left   {NEAR} entity 3, hash_{:08X} at 5, 5, 0: ambiguous, it cannot be told from entity 4 of {NEAR}",
        or_io::string_hash(b"lamp")
    );
    assert!(printed.contains(&left), "{left:?} in {printed}");
    assert!(
        !printed.contains("the collision and the occluders"),
        "nothing goes, so nothing is said of what would stay of it: {printed}"
    );
    assert!(
        printed.contains("content maps: 1 entity named, 0 entities to take out\n")
            && printed.contains("content maps: 0 operations changed 0 sources\n"),
        "{printed}"
    );
    assert!(files(&dir.path().join("maps")) == before);
    // Both named, both go.
    let twin = format!("{NEAR}:4");
    assert_eq!(
        cli(&[text(&project), &lamp, &twin, "--apply"]),
        ExitCode::SUCCESS
    );
    assert!(files(&dir.path().join("maps")) != before);
}

#[test]
fn a_far_model_named_takes_what_is_under_it_and_the_plan_says_under_which() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with_maps(dir.path());
    let row = format!("{FAR}:1");
    let (printed, error, succeeded) = said(&[text(&project), &row]);
    assert!(succeeded, "{printed}{error}");
    let shown = |name: &str| format!("hash_{:08X}", or_io::string_hash(name.as_bytes()));
    for line in [
        format!(
            "  goes   {FAR} entity 1, {} at 30, 20, 0.5: chosen\n",
            shown("row_far")
        ),
        format!(
            "  goes   {NEAR} entity 1, {} at 28, 20, 0.5: under entity 1 of {FAR}\n",
            shown("house")
        ),
        format!(
            "  goes   {NEAR} entity 2, {} at 32, 20, 0.5: under entity 1 of {FAR}\n",
            shown("house")
        ),
        "content maps: 1 entity named, 3 entities to take out\n".to_string(),
    ] {
        assert!(printed.contains(&line), "{line:?} in {printed}");
    }
}

#[test]
fn a_linked_content_is_planned_from_the_project_that_links_it_and_written_only_from_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    a_project_with_maps(&home);
    let linking = work.join("work.openrage-proj");
    std::fs::write(
        &linking,
        r#"{
  "documents": [],
  "game": "both",
  "modules": [],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "work", "take": [] }],
  "schema": 4
}"#,
    )
    .unwrap();
    let pack = home.join("maps");
    let link = [
        "project",
        "link",
        text(&linking),
        text(&pack),
        "--id",
        "block",
    ];
    assert_eq!(
        or_cli::run(&link.map(str::to_string)),
        ExitCode::SUCCESS,
        "the maps of `home` are linked into `work`"
    );
    let before = files(&home);
    let tower = format!("{NEAR}:0");

    let (printed, error, succeeded) = said(&[text(&linking), &tower]);
    assert!(
        succeeded,
        "a plan writes nothing, so it reads a linked content: {printed}{error}"
    );
    assert!(
        printed.contains("content block: 1 entity named, 2 entities to take out\n"),
        "{printed}"
    );

    let (printed, error, succeeded) = said(&[text(&linking), &tower, "--apply"]);
    assert!(!succeeded, "{printed}");
    assert!(
        error.contains("is in the linked content `block`")
            && error.contains("this project only reads it"),
        "{error}"
    );
    assert!(
        files(&home) == before,
        "the project that links a content does not write it"
    );
}
