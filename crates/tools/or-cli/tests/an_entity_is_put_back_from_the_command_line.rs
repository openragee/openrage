//! `openrage-cli maps put-back` prints the plan `or_maps::put_back::plan` makes and writes nothing, writes with `--apply` what
//! `or_maps::put_back::apply` writes, keeps the list that takes it back with `--inverse`, prints the plan as data with `--json`, and
//! refuses what is not put back with its sentence and every source as it was.
//!
//! ⚠ The command prints, which a test cannot read back in its own process, so what it says is read from a child of this test
//! binary that calls `or_cli::run`. A process of its own is also what keeps an operation here from the schema snapshot another
//! test of a shared process installs: a record of a sample map is saved by the class its own file states.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_maps::import::{add_files, Incoming};
use or_maps::operation::{apply, At, Operation, Operations, PoseData};
use or_maps::sample::{child_map_file, map_file, Entity};
use or_maps::Maps;
use serde_json::{json, Value};

const FAR: &str = "harbour_lod.metadata.src";
const NEAR: &str = "harbour.metadata.src";

fn maps(verb: &str, args: &[&str]) -> ExitCode {
    let mut all = vec!["maps".to_string(), verb.to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

/// A project whose maps content holds a map of a crane's far model, and a map under it of the crane, a crate, and twin trees.
fn a_project(dir: &Path) -> PathBuf {
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
    let extents = ([0.0; 3], [100.0; 3]);
    let far = map_file(
        "harbour_lod",
        &[Entity {
            children: 1,
            ..Entity::at("crane_lod", [40.0, 40.0, 0.0], 20)
        }],
        extents,
    );
    let near = child_map_file(
        "harbour",
        "harbour_lod",
        &[
            Entity {
                parent: 0,
                in_parent_map: true,
                ..Entity::at("crane", [40.0, 40.0, 0.0], 1)
            },
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 2),
            Entity::at("prop_tree", [30.0, 30.0, 0.0], 7),
            Entity::at("prop_tree", [34.0, 30.0, 0.0], 7),
        ],
        extents,
    );
    let incoming = |name: &'static str, bytes: &'static [u8]| Incoming {
        name,
        bytes,
        from_game: true,
        virtual_path: None,
    };
    let folder = dir.join("maps");
    let added = add_files(
        &folder,
        &[
            incoming("harbour_lod.ymap", far.leak()),
            incoming("harbour.ymap", near.leak()),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    // The names a person typed, which a map file keeps the hash of alone.
    let mut model = Maps::read(&folder).unwrap();
    model.remember(&["crane", "crane_lod", "prop_crate_a"]);
    model.write(&folder).unwrap();
    dir.join("harbour.openrage-proj")
}

/// The XML of each source of the content.
fn sources(dir: &Path) -> BTreeMap<&'static str, Vec<u8>> {
    [FAR, NEAR]
        .into_iter()
        .map(|source| {
            let xml = dir.join("maps").join(source).join("metadata.xml");
            (source, std::fs::read(xml).unwrap())
        })
        .collect()
}

fn pose(position: [f32; 3]) -> PoseData {
    PoseData {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
    }
}

const CHILD_ENV: &str = "OPENRAGE_MAPS_PUT_BACK_ARGS";
const CHILD_TEST: &str = "run_maps_put_back_for_a_parent";
const SEPARATOR: char = '\u{1f}';

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_maps_put_back_for_a_parent() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = (args.to_string_lossy().split(SEPARATOR))
        .map(str::to_string)
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    std::process::exit(match maps("put-back", &args) == ExitCode::SUCCESS {
        true => 0,
        false => 1,
    });
}

/// What `openrage-cli maps put-back <args>` prints and what it says of an error, with whether it succeeded.
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

/// What the command prints for `args`, which it is to do.
fn printed(args: &[&str]) -> String {
    let (printed, error, succeeded) = said(args);
    assert!(succeeded, "{printed}{error}");
    // The harness of the child says which test it runs before the command prints.
    let from = printed.find("content ").or_else(|| printed.find('{'));
    printed[from.unwrap_or(0)..].to_string()
}

/// The JSON the command prints for `args` with `--json`.
fn json_of(args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let (printed, error, succeeded) = said(&all);
    assert!(succeeded, "{printed}{error}");
    let from = printed.find('{').expect("an object is printed");
    let mut read = serde_json::Deserializer::from_str(&printed[from..]).into_iter::<Value>();
    read.next().expect("an object").expect("the JSON reads")
}

/// What the command says of the error it refuses `args` with.
fn refused(args: &[&str]) -> String {
    let (printed, error, succeeded) = said(args);
    assert!(!succeeded, "{printed}");
    error
}

#[test]
fn a_deleted_building_is_planned_by_its_index_in_the_base_written_only_when_asked_and_taken_back_by_the_list_kept(
) {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path());
    let project = text(&file);
    let maps_folder = dir.path().join("maps");
    let made = sources(dir.path());
    let crane = At {
        source: NEAR.into(),
        index: 0,
    };
    let chain = or_maps::delete::plan(&maps_folder, &[crane]).unwrap();
    or_maps::delete::apply(&maps_folder, &chain).unwrap();
    let deleted = sources(dir.path());
    assert_ne!(deleted, made);

    let name = format!("{NEAR}:base:0");
    let plan = printed(&[project, &name]);
    let expected = format!(
        "content maps:\n  \
         comes back  {NEAR} entity 0 of the base, crane at (40, 40, 0), guid 1 as entity 0: chosen\n  \
         comes back  {FAR} entity 0 of the base, crane_lod at (40, 40, 0), guid 20 as entity 0: the far model of what comes back\n\
         would change {NEAR}/metadata.xml\n\
         would change {FAR}/metadata.xml\n\
         content maps: 2 operations would change 2 sources; nothing was written (write it with --apply)\n\
         {NEAR} holds 1 change of its base now, which is what a discard of the map would lose\n"
    );
    assert!(plan.starts_with(&expected), "{plan}");
    assert_eq!(sources(dir.path()), deleted, "a plan writes nothing");
    let json = json_of(&[project, &name]);
    let returns = &json["plan"]["returns"];
    assert_eq!(json["applied"], false);
    assert_eq!(returns[0]["why"], "chosen");
    assert_eq!(returns[0]["base"]["archetype"]["text"], "crane");
    assert_eq!(returns[1]["why"], "far-model");
    assert_eq!(
        (&returns[1]["source"], &returns[1]["index"]),
        (&json!(FAR), &json!(0))
    );
    assert_eq!(json["changes_left"], 1);
    assert_eq!(sources(dir.path()), deleted, "nor does the plan as data");

    let back = dir.path().join("back.json");
    let back = text(&back);
    let done = printed(&[project, &name, "--apply", "--inverse", back]);
    for line in [
        format!("changed {NEAR}/metadata.xml\n"),
        "content maps: 2 operations changed 2 sources\n".to_string(),
        format!("the list that takes it back is {back}: `maps apply` does it\n"),
        format!("{NEAR} states what its base states after this\n"),
    ] {
        assert!(done.contains(&line), "{line:?} in {done}");
    }
    assert_eq!(
        sources(dir.path()),
        made,
        "the building is back as the base has it"
    );

    // The list kept is one `maps apply` takes, and leaves the sources as the deletion left them.
    assert_eq!(maps("apply", &[project, back]), ExitCode::SUCCESS);
    assert_eq!(sources(dir.path()), deleted);

    // Named by its far model, the building comes back under it, and the plan says which entity each stands under.
    let name = format!("{FAR}:base:0");
    let json = json_of(&[project, &name]);
    let returns = &json["plan"]["returns"];
    assert_eq!(
        (&returns[0]["source"], &returns[0]["why"]),
        (&json!(FAR), &json!("chosen"))
    );
    assert_eq!(returns[1]["source"], NEAR);
    assert_eq!(
        returns[1]["why"]["under"],
        json!({ "source": FAR, "index": 0 })
    );
    let plan = printed(&[project, &name]);
    let line = format!(
        "  comes back  {NEAR} entity 0 of the base, crane at (40, 40, 0), guid 1 as entity 0: \
         under entity 0 of the base of {FAR}\n"
    );
    assert!(plan.contains(&line), "{plan}");
}

#[test]
fn a_changed_entity_is_named_by_its_index_now_and_the_json_holds_the_plan_with_the_list_that_does_it(
) {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path());
    let project = text(&file);
    let made = sources(dir.path());
    let moved = Operation::PlaceEntity {
        source: NEAR.into(),
        expects: None,
        entity: 1,
        pose: pose([12.0, 20.0, 0.5]),
    };
    apply(&dir.path().join("maps"), &moved).unwrap();

    let name = format!("{NEAR}:1");
    let plan = printed(&[project, &name]);
    let line = format!(
        "  puts back   {NEAR} entity 1 as {NEAR} entity 1 of the base, prop_crate_a at (10, 20, 0.5), guid 2: \
         it undoes moved from (10, 20, 0.5) to (12, 20, 0.5)\n"
    );
    assert!(plan.contains(&line), "{plan}");

    let json = json_of(&[project, &name]);
    assert_eq!(json["content"], "maps");
    assert_eq!(json["applied"], false);
    let plan = &json["plan"];
    assert_eq!(plan["restored"]["source"], NEAR);
    assert_eq!(plan["restored"]["now"]["index"], 1);
    assert_eq!(
        plan["restored"]["base"]["archetype"]["text"], "prop_crate_a",
        "a name the content remembers is given its text"
    );
    assert_eq!(plan["restored"]["changes"][0]["what"], "moved");
    assert_eq!(plan["returns"], json!([]));
    assert_eq!(plan["sources"], json!([NEAR]));
    assert!(json["changes_left"].as_u64().unwrap() >= 1);
    // The list is the file `maps apply` takes: applied, it is what --apply does.
    let list = dir.path().join("list.json");
    std::fs::write(&list, plan["operations"].to_string()).unwrap();
    let read = Operations::from_json(&std::fs::read(&list).unwrap(), "the list").unwrap();
    assert_eq!(read.operations[0].name(), "replace_entity");
    assert!(read.operations[0].expects().is_some());
    assert_eq!(maps("apply", &[project, text(&list)]), ExitCode::SUCCESS);
    assert_eq!(sources(dir.path()), made);

    let error = refused(&[project, &name, "--json"]);
    assert!(error.contains("there is nothing to put back"), "{error}");
}

#[test]
fn what_is_not_put_back_is_refused_with_its_sentence_and_no_source_is_changed() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path());
    let project = text(&file);
    let maps_folder = dir.path().join("maps");
    // One of the twin trees moves, and a crate is added.
    let moved = Operation::PlaceEntity {
        source: NEAR.into(),
        expects: None,
        entity: 3,
        pose: pose([36.0, 31.0, 0.0]),
    };
    apply(&maps_folder, &moved).unwrap();
    let added = Operation::AddEntity {
        source: NEAR.into(),
        expects: None,
        archetype: "prop_crate_a".into(),
        pose: pose([50.0, 50.0, 0.5]),
        donor: Some(At {
            source: NEAR.into(),
            index: 1,
        }),
    };
    let index = apply(&maps_folder, &added).unwrap().added.unwrap();
    let edited = sources(dir.path());

    for name in [format!("{NEAR}:3"), format!("{NEAR}:base:3")] {
        let error = refused(&[project, &name, "--apply"]);
        assert!(error.contains("cannot be told from a twin"), "{error}");
        assert!(error.contains("; no source was changed"), "{error}");
    }
    let error = refused(&[project, &format!("{NEAR}:{index}"), "--apply"]);
    assert!(
        error.contains("the source added it") && error.contains("`openrage-cli maps delete`"),
        "{error}"
    );
    let error = refused(&[project, &format!("{NEAR}:1"), "--apply"]);
    assert!(error.contains("there is nothing to put back"), "{error}");
    assert_eq!(sources(dir.path()), edited);

    // What names no entity, a second entity and a list kept of nothing written are errors of usage.
    let run = |more: &[&str]| {
        let mut args = vec![project];
        args.extend(more);
        maps("put-back", &args)
    };
    assert_eq!(run(&["harbour"]), ExitCode::from(2));
    assert_eq!(run(&[":base:3"]), ExitCode::from(2));
    assert_eq!(
        run(&[&format!("{NEAR}:1"), &format!("{NEAR}:2")]),
        ExitCode::from(2)
    );
    assert_eq!(
        run(&[&format!("{NEAR}:3"), "--inverse", "back.json"]),
        ExitCode::from(2)
    );
    assert_eq!(run(&[&format!("{NEAR}:3")]), ExitCode::FAILURE);
    assert_eq!(sources(dir.path()), edited);
}

/// A project whose maps content holds a far model of a stand of trees, under it a map of two trees that state one guid, a
/// bench and a post that stands for itself, and a map with no parent whose crate says its far model is in one.
fn a_grove(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("grove.openrage-proj"),
        r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "grove", "take": ["maps"] }],
  "schema": 4
}"#,
    )
    .unwrap();
    let extents = ([0.0; 3], [100.0; 3]);
    let under = |archetype: &'static str, position: [f32; 3], guid: u32| Entity {
        parent: 0,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    };
    let far = map_file(
        "grove_lod",
        &[Entity {
            children: 3,
            ..Entity::at("lod_trees", [32.0, 30.0, 0.0], 50)
        }],
        extents,
    );
    let near = child_map_file(
        "grove",
        "grove_lod",
        &[
            under("prop_tree", [30.0, 30.0, 0.0], 7),
            under("prop_tree", [34.0, 30.0, 0.0], 7),
            under("prop_bench", [32.0, 32.0, 0.5], 8),
            Entity::at("prop_post", [5.0, 5.0, 0.0], 9),
        ],
        extents,
    );
    let lonely = map_file(
        "lonely",
        &[under("prop_crate_a", [10.0, 20.0, 0.5], 1)],
        extents,
    );
    let incoming = |name: &'static str, bytes: &'static [u8]| Incoming {
        name,
        bytes,
        from_game: true,
        virtual_path: None,
    };
    let added = add_files(
        &dir.join("maps"),
        &[
            incoming("grove_lod.ymap", far.leak()),
            incoming("grove.ymap", near.leak()),
            incoming("lonely.ymap", lonely.leak()),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let mut model = Maps::read(&dir.join("maps")).unwrap();
    model.remember(&["prop_tree"]);
    model.write(&dir.join("maps")).unwrap();
    dir.join("grove.openrage-proj")
}

#[test]
fn the_plan_says_each_count_that_changes_each_entity_it_leaves_out_and_each_note() {
    const GROVE: &str = "grove.metadata.src";
    const GROVE_LOD: &str = "grove_lod.metadata.src";
    const LONELY: &str = "lonely.metadata.src";
    let dir = tempfile::tempdir().unwrap();
    let file = a_grove(dir.path());
    let project = text(&file);
    let maps_folder = dir.path().join("maps");
    let at = |source: &str, index: usize| At {
        source: source.into(),
        index,
    };

    // The bench goes, and the far model of the stand counts two: put back, it counts three again.
    let bench = or_maps::delete::plan(&maps_folder, &[at(GROVE, 2)]).unwrap();
    or_maps::delete::apply(&maps_folder, &bench).unwrap();
    let plan = printed(&[project, &format!("{GROVE}:base:2")]);
    let line =
        format!("  counts      {GROVE_LOD} entity 0: its count of children goes from 2 to 3\n");
    assert!(plan.contains(&line), "{plan}");
    let json = json_of(&[project, &format!("{GROVE}:base:2")]);
    assert_eq!(
        json["plan"]["recounted"],
        json!([{ "source": GROVE_LOD, "entity": 0, "from": 2, "to": 3 }])
    );
    let done = printed(&[project, &format!("{GROVE}:base:2"), "--apply"]);
    assert!(done.contains(&line), "{done}");

    // The far model goes with what stands under it, and a tree of another guid is planted beside the post: the two trees of the
    // base cannot be told from it, so the far model comes back with the bench and without them.
    let stand = or_maps::delete::plan(&maps_folder, &[at(GROVE_LOD, 0)]).unwrap();
    or_maps::delete::apply(&maps_folder, &stand).unwrap();
    let planted = Operation::AddEntity {
        source: GROVE.into(),
        expects: None,
        archetype: "prop_tree".into(),
        pose: pose([60.0, 60.0, 0.0]),
        donor: None,
    };
    apply(&maps_folder, &planted).unwrap();
    let plan = printed(&[project, &format!("{GROVE_LOD}:base:0")]);
    for tree in [0, 1] {
        let x = [30, 34][tree];
        let line = format!(
            "  left        {GROVE} entity {tree} of the base, prop_tree at ({x}, 30, 0), guid 7: \
             it cannot be told from a twin, so it is not put back by a guess\n"
        );
        assert!(plan.contains(&line), "{line:?} in {plan}");
    }
    let json = json_of(&[project, &format!("{GROVE_LOD}:base:0")]);
    assert_eq!(json["plan"]["left"].as_array().map(Vec::len), Some(2));
    assert_eq!(json["plan"]["left"][1]["base"]["index"], 1);

    // The crate of the map with no parent is moved: put back, its link into a parent map is stated as the base states it.
    let moved = Operation::PlaceEntity {
        source: LONELY.into(),
        expects: None,
        entity: 0,
        pose: pose([12.0, 20.0, 0.5]),
    };
    apply(&maps_folder, &moved).unwrap();
    let plan = printed(&[project, &format!("{LONELY}:0")]);
    let line = format!(
        "  note        {LONELY} entity 0 of the base names a far model in a parent map, and its map names none: \
         the link is stated as the base states it\n"
    );
    assert!(plan.contains(&line), "{plan}");
    let json = json_of(&[project, &format!("{LONELY}:0")]);
    assert_eq!(json["plan"]["notes"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_linked_content_is_planned_from_the_project_that_links_it_and_written_only_from_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    a_project(&home);
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
    let moved = Operation::PlaceEntity {
        source: NEAR.into(),
        expects: None,
        entity: 1,
        pose: pose([12.0, 20.0, 0.5]),
    };
    apply(&pack, &moved).unwrap();
    let edited = sources(&home);
    let name = format!("{NEAR}:1");

    let (printed, error, succeeded) = said(&[text(&linking), &name]);
    assert!(
        succeeded,
        "a plan writes nothing, so it reads a linked content: {printed}{error}"
    );
    assert!(
        printed.contains("content block:\n  puts back   ")
            && printed.contains(" would change 1 source;"),
        "{printed}"
    );

    let (printed, error, succeeded) = said(&[text(&linking), &name, "--apply"]);
    assert!(!succeeded, "{printed}");
    assert!(
        error.contains("is in the linked content `block`")
            && error.contains("this project only reads it"),
        "{error}"
    );
    assert_eq!(
        sources(&home),
        edited,
        "the project that links a content does not write it"
    );
}
