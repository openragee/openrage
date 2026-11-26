//! `openrage-cli maps interior` lists the interiors of a project's type files with their rooms, portals and entity sets and what the
//! rules say of each, its JSON lists what its text lists, an interior is named by its source and its index, and `--apply` does a
//! list of operations as `maps apply` does: whole, with nothing written on a dry run, and refused when it was made for another state.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its own process, so what it says is read from a child of
//! this test binary that calls `or_cli::run` between two markers.
//!
//! ⚠ A test binary of its own, and no test of the crate's library, some of whose tests install a schema snapshot: a save lays a
//! file whose shape an edit changed out through the schema its process installed, and the engine's classes state members a sample
//! type file's do not.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_maps::import::{add_files, Incoming};
use or_maps::interior::interiors_of;
use or_maps::operation::{digest_of, Operation, Operations, PortalMembers, RoomMembers};
use or_maps::sample::{map_file, types_file_with_interiors, Entity, Interior, Room, VILLA};
use serde_json::{json, Value};

const VILLA_TYPES: &str = "villa_types.metadata.src";

/// A project in `dir` whose maps content holds a map file of one entity and a type file whose one archetype is `interior`.
fn a_project_with(dir: &Path, interior: Interior<'_>) -> PathBuf {
    std::fs::write(
        dir.join("villa.openrage-proj"),
        r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "villa", "take": ["maps"] }],
  "schema": 4
}"#,
    )
    .unwrap();
    let yard = map_file(
        "yard",
        &[Entity::at("prop_chair", [1.0, 2.0, 3.0], 9)],
        ([-50.0; 3], [50.0; 3]),
    );
    let villa = types_file_with_interiors("villa_types", &[], &[interior]);
    let incoming = |name: &'static str, bytes: Vec<u8>| Incoming {
        name,
        bytes: bytes.leak(),
        from_game: false,
        virtual_path: None,
    };
    let files = [
        incoming("yard.ymap", yard),
        incoming("villa_types.ytyp", villa),
    ];
    let added = add_files(&dir.join("maps"), &files, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    dir.join("villa.openrage-proj")
}

const CHILD_ENV: &str = "OPENRAGE_MAPS_INTERIOR_ARGS";
const CHILD_TEST: &str = "run_maps_interior_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_maps_interior_for_a_parent() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let mut all = vec!["maps".to_string(), "interior".to_string()];
    all.extend(
        (args.to_string_lossy().split(SEPARATOR))
            .filter(|arg| !arg.is_empty())
            .map(str::to_string),
    );
    // ⚠ Flushed: a marker with no line end stays in the buffer, and the command's own lines would come out before it.
    let mark = |marker: &str| {
        print!("{marker}");
        std::io::Write::flush(&mut std::io::stdout()).expect("stdout takes the marker");
    };
    mark(BEGIN);
    let code = or_cli::run(&all);
    mark(END);
    if code != ExitCode::SUCCESS {
        std::process::exit(1);
    }
}

/// What `openrage-cli maps interior <args>` prints, what it says of an error, and whether it succeeded.
fn said(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .output()
        .expect("this test binary runs as a child");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let printed = (stdout.split_once(BEGIN))
        .map(|(_, rest)| rest.split_once(END).map_or(rest, |(printed, _)| printed))
        .unwrap_or_default()
        .to_string();
    (
        printed,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, into);
            } else {
                into.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, &mut found);
    found
}

/// The villa as its file names it: a type file keeps the hash of a name, and the content remembers none.
fn villa_by_hash() -> String {
    format!("hash_{:08X}", or_io::string_hash(b"villa"))
}

#[test]
fn the_listing_as_json_lists_what_its_text_lists() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with(dir.path(), VILLA);
    let path = project.to_str().unwrap();
    let (text, errors, clean) = said(&[path]);
    let (data, _, clean_as_data) = said(&[path, "--json"]);
    assert!(clean && clean_as_data, "{text}{errors}");

    let json: Value = serde_json::from_str(&data).unwrap_or_else(|e| panic!("{e}: {data}"));
    assert_eq!(json[0]["module"], "maps");
    let interiors = json[0]["interiors"].as_array().unwrap();
    let [villa] = interiors.as_slice() else {
        panic!("one interior: {data}");
    };
    assert_eq!(villa["source"], VILLA_TYPES);
    assert_eq!(villa["archetype"], 0);
    assert_eq!(villa["name"], villa_by_hash());
    assert_eq!(villa["entities"], VILLA.entities.len());
    assert_eq!(villa["time_cycle_modifiers"], VILLA.modifiers.len());
    let rooms = villa["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 4);
    assert_eq!(
        rooms[1],
        json!({
            "name": "hall", "bb_min": [-5.0, -5.0, 0.0], "bb_max": [5.0, 5.0, 3.0], "flags": 0,
            "floor_id": 0, "portal_count": 2, "attached_objects": [0, 1],
        })
    );
    let portals = villa["portals"].as_array().unwrap();
    assert_eq!(portals.len(), 3);
    assert_eq!(portals[0]["room_from"], 1);
    assert_eq!(portals[0]["room_to"], 0);
    assert_eq!(portals[0]["corners"].as_array().unwrap().len(), 4);
    assert_eq!(portals[0]["attached_objects"], json!([4]));
    let sets = villa["entity_sets"].as_array().unwrap();
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0]["entities"].as_array().unwrap().len(), 2);
    assert_eq!(sets[0]["entities"][1]["location"], 3);
    assert_eq!(villa["broken"], json!([]));
    assert_eq!(villa["findings"], json!([]));

    // The text is the lines of the same listing, under the content and the interior, and holds no other line.
    let source = dir.path().join("maps").join(VILLA_TYPES);
    let listed = &interiors_of(&or_maps::facts::cooked(&source).unwrap(), None).unwrap()[0];
    let mut lines = listed.listed.lines().into_iter();
    let mut expected = vec![
        "content maps: 1 interior(s)".to_string(),
        format!(
            "  {VILLA_TYPES}:0 `{}`: {}",
            villa_by_hash(),
            lines.next().unwrap()
        ),
    ];
    expected.extend(lines.map(|line| format!("    {line}")));
    assert_eq!(text.lines().collect::<Vec<_>>(), expected);
    assert!(
        text.contains("    room 1 `hall`: box -5,-5,0 to 5,5,3"),
        "{text}"
    );
    assert!(text.contains("    portal 0: room 1 to room 0"), "{text}");
    assert!(text.contains("    entity set 1 `"), "{text}");
}

#[test]
fn an_interior_is_named_by_its_source_and_its_index_and_a_name_of_none_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with(dir.path(), VILLA);
    let path = project.to_str().unwrap();
    let (all, _, listed) = said(&[path]);
    assert!(
        listed && all.contains("content maps: 1 interior(s)"),
        "{all}"
    );
    for named in [VILLA_TYPES.to_string(), format!("{VILLA_TYPES}:0")] {
        assert_eq!(said(&[path, &named]).0, all, "{named}");
    }
    // Two names of one interior list it once, and neither is a name of none.
    let by_index = format!("{VILLA_TYPES}:0");
    let (twice, refused, listed) = said(&[path, VILLA_TYPES, &by_index]);
    assert!(listed, "{refused}");
    assert_eq!(twice, all);
    for (named, why) in [
        (
            format!("{VILLA_TYPES}:1"),
            "names no interior of the maps content",
        ),
        (
            "no_such.metadata.src".to_string(),
            "names no interior of the maps content",
        ),
        ("yard.metadata.src".to_string(), "is no type file source"),
        (
            format!("{VILLA_TYPES}:first"),
            "after the `:` comes the index of an archetype",
        ),
    ] {
        let (printed, refused, succeeded) = said(&[path, &named]);
        assert!(!succeeded && printed.is_empty(), "{named}: {printed}");
        assert!(refused.contains(why), "{named}: {refused}");
    }
    for (args, why) in [
        (
            vec![path, "--module", "roads"],
            "has no maps content `roads`",
        ),
        (vec![], "expected a <project.openrage-proj>"),
        (vec![path, "--dry-run"], "--dry-run goes with --apply"),
    ] {
        let (_, refused, succeeded) = said(&args);
        assert!(!succeeded && refused.contains(why), "{args:?}: {refused}");
    }
}

#[test]
fn what_the_rules_find_is_listed_and_what_a_save_refuses_fails_the_command() {
    // A vault no portal names, in an interior that opens to the outside.
    let mut rooms = VILLA.rooms.to_vec();
    rooms.push(Room {
        name: "vault",
        min: [12.0, -5.0, -3.0],
        max: [15.0, 5.0, 0.0],
        attached: &[],
        portal_count: None,
    });
    let dir = tempfile::tempdir().unwrap();
    let cut_off = Interior {
        rooms: rooms.leak(),
        ..VILLA
    };
    let project = a_project_with(dir.path(), cut_off);
    let (text, _, clean) = said(&[project.to_str().unwrap()]);
    assert!(clean, "a warning does not fail the command: {text}");
    let line = (text.lines())
        .find(|line| line.contains("interior-room-unreachable"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(line.trim_start().starts_with("warning"), "{line}");
    assert!(line.ends_with("no chain of portals: #4"), "{line}");
    let (data, _, _) = said(&[project.to_str().unwrap(), "--json"]);
    let json: Value = serde_json::from_str(&data).unwrap();
    let findings = &json[0]["interiors"][0]["findings"];
    assert_eq!(findings[0]["code"], "interior-room-unreachable");
    assert_eq!(findings.as_array().unwrap().len(), 1);

    // A room that attaches an entity the interior does not hold: the rule `interior-broken`, an error.
    let mut rooms = VILLA.rooms.to_vec();
    rooms[1].attached = &[0, 1, 9];
    let dir = tempfile::tempdir().unwrap();
    let broken = Interior {
        rooms: rooms.leak(),
        ..VILLA
    };
    let project = a_project_with(dir.path(), broken);
    let (text, _, clean) = said(&[project.to_str().unwrap()]);
    assert!(!clean, "{text}");
    assert!(
        text.contains("error   interior-broken")
            && text.contains("room 1 attaches object 9, past the 6 entities in this interior"),
        "{text}"
    );
}

/// A list that gives the villa an attic over the hall, with a portal from it to the outside.
fn an_attic(expects: Option<String>) -> Vec<Operation> {
    vec![
        Operation::AddRoom {
            source: VILLA_TYPES.into(),
            expects,
            archetype: 0,
            room: None,
            members: RoomMembers {
                name: Some("attic".into()),
                bb_min: Some([-5.0, -5.0, 3.0]),
                bb_max: Some([5.0, 5.0, 5.5]),
                timecycle_name: Some("villa_attic_dust".into()),
                ..RoomMembers::default()
            },
            attached_objects: Vec::new(),
        },
        Operation::AddPortal {
            source: VILLA_TYPES.into(),
            expects: None,
            archetype: 0,
            portal: None,
            members: PortalMembers {
                room_from: Some(4),
                room_to: Some(0),
                corners: Some(vec![
                    [-1.0, -5.0, 3.0],
                    [1.0, -5.0, 3.0],
                    [1.0, -5.0, 5.0],
                    [-1.0, -5.0, 5.0],
                ]),
                ..PortalMembers::default()
            },
            attached_objects: Vec::new(),
        },
    ]
}

#[test]
fn a_list_is_applied_whole_a_dry_run_writes_nothing_and_a_stale_list_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = a_project_with(dir.path(), VILLA);
    let path = project.to_str().unwrap();
    let maps = dir.path().join("maps");
    let xml = maps.join(VILLA_TYPES).join("metadata.xml");
    let made_for = digest_of(&std::fs::read(&xml).unwrap());
    let list = dir.path().join("attic.json");
    let list_path = list.to_str().unwrap();
    let json = Operations::new(an_attic(Some(made_for))).to_json().unwrap();
    assert!(json.contains("\"op\": \"add_room\"") && json.contains("\"op\": \"add_portal\""));
    std::fs::write(&list, json).unwrap();

    let before = files(&maps);
    let (text, errors, succeeded) = said(&[path, "--apply", list_path, "--dry-run"]);
    assert!(succeeded, "{errors}");
    assert!(
        text.contains("  1. add_room of villa_types.metadata.src: added at 4\n")
            && text.contains("  2. add_portal of villa_types.metadata.src: added at 3\n")
            && text.contains("would change villa_types.metadata.src/metadata.xml\n")
            && text.ends_with(
                "content maps: 2 operation(s) would change 1 source(s); nothing was written\n"
            ),
        "{text}"
    );
    assert!(files(&maps) == before, "a dry run writes nothing");

    let (text, errors, succeeded) = said(&[path, "--apply", list_path]);
    assert!(succeeded, "{errors}");
    assert!(
        text.ends_with("content maps: 2 operation(s) changed 1 source(s)\n"),
        "{text}"
    );
    let (listing, _, clean) = said(&[path, &format!("{VILLA_TYPES}:0")]);
    assert!(clean, "{listing}");
    assert!(listing.contains("5 room(s), 4 portal(s)"), "{listing}");
    assert!(
        listing.contains(
            "room 4 `attic`: box -5,-5,3 to 5,5,5.5, flags 0, floor 0, counts 1 portal(s)"
        ),
        "{listing}"
    );
    assert!(listing.contains("room 0 `limbo`") && listing.contains("counts 2 portal(s)"));
    assert!(listing.contains("portal 3: room 4 to room 0"), "{listing}");
    assert!(
        !listing.contains("interior-"),
        "nothing is found in it: {listing}"
    );
    // The name the list states is remembered in the content, since the file keeps its hash alone.
    let model = or_maps::Maps::read(&maps).unwrap();
    assert_eq!(
        model
            .name_of(or_io::string_hash(b"villa_attic_dust"))
            .as_deref(),
        Some("villa_attic_dust")
    );

    // The list was made for the villa as it was, and is refused for the villa as it is.
    let before = files(&maps);
    let (printed, refused, succeeded) = said(&[path, "--apply", list_path]);
    assert!(!succeeded && printed.is_empty(), "{printed}");
    assert!(
        refused.contains("operation 1 of 2, add_room of `villa_types.metadata.src`")
            && refused.contains("the source is not as the operation expects it")
            && refused.contains("; no source was changed"),
        "{refused}"
    );
    assert!(files(&maps) == before);

    for (args, why) in [
        (
            vec![path, "--apply", list_path, "--json"],
            "--json is the form of the listing",
        ),
        (
            vec![path, VILLA_TYPES, "--apply", list_path],
            "--apply takes a list",
        ),
    ] {
        let (_, refused, succeeded) = said(&args);
        assert!(!succeeded && refused.contains(why), "{args:?}: {refused}");
    }
    assert!(files(&maps) == before);
}
