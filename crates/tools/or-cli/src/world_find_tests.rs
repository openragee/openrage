//! `world find` over an install built by hand: what its arguments ask, that its JSON and its text list the same things, and
//! what it copies for a script.

use crate::world_fixture;

use or_maps::place::PastedPlace;

use super::*;

use world_fixture::{
    cube, hash, level_square, manifest_bytes, mesh_bound_bytes, mount, placing_map_bytes,
    types_bytes, write_archive, MapSpec, Placed,
};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// Every archetype the type file declares; `prop_nameless` has no model file.
const MODELS: &[&str] = &[
    "bench",
    "prop_bench",
    "workbench",
    "prop_unplaced",
    "prop_nameless",
];

/// Where the fixture stands its benches, by map, in the order of each map's entities.
fn benches() -> Vec<(&'static str, Placed)> {
    vec![
        (
            "m_town",
            Placed::new("prop_bench", [110.0, 110.0, 10.0], 90.0, 11),
        ),
        (
            "m_dock",
            Placed::new("prop_bench", [410.0, 110.0, 5.0], 45.0, 21),
        ),
        (
            "m_hill",
            Placed::new("prop_bench", [150.0, 450.0, 30.0], 270.0, 31),
        ),
    ]
}

/// An install of three maps that each stand a bench, a type file, a model file for each archetype but one, and a floor.
fn install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let manifest = manifest_bytes(&[], &[]);
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    let unit = cube([-1.0; 3], 2.0);
    let declared: Vec<_> = MODELS.iter().map(|name| (*name, unit, 80.0)).collect();
    let floor = level_square([100.0, 100.0, 10.0], 100.0, 0x21);
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("types/props.ytyp".into(), types_bytes("props", &declared)),
        ("levels/m_town_0.ybn".into(), mesh_bound_bytes(floor)),
    ];
    for (map, bench) in benches() {
        let corner = [bench.position[0] - 10.0, bench.position[1] - 10.0, 0.0];
        let spec = MapSpec::around(cube(corner, 100.0));
        let lamp = Placed::new("workbench", [corner[0], corner[1], 1.0], 0.0, 1);
        files.push((
            format!("levels/{map}.ymap"),
            placing_map_bytes(map, &spec, &[lamp, bench]),
        ));
    }
    for model in MODELS.iter().filter(|model| **model != "prop_nameless") {
        files.push((format!("models/{model}.ydr"), b"a model".to_vec()));
    }
    let files: Vec<(&str, Vec<u8>)> = (files.iter())
        .map(|(path, bytes)| (path.as_str(), bytes.clone()))
        .collect();
    write_archive(&root, "x64a.rpf", &files);
    (scratch, root)
}

/// What `world find <install> <args>` says of `game`.
fn say(game: &MountedGame, root: &Path, kept: &Path, args: &[&str]) -> Said {
    let args: Vec<&str> = ["install"].iter().chain(args).copied().collect();
    let question = question_of(&strings(&args)).expect("a question");
    let forms = ScriptForms::built_in().expect("the forms of the tree");
    let form = (question.copy.as_deref()).map(|id| form_of(&forms, id).expect("a form"));
    let said = said(game, root, kept, false, &question, None, form.as_ref());
    let said = said.expect("the text is read");
    assert!(said.not_kept.is_none(), "{:?}", said.not_kept);
    said
}

#[test]
fn the_arguments_are_an_install_the_text_and_its_options() {
    let asked = question_of(&strings(&["/gta", "prop_bench"])).expect("a question");
    assert_eq!(
        asked,
        Question {
            install: "/gta".into(),
            text: "prop_bench".into(),
            near: None,
            limit: LIMIT,
            copy: None,
        }
    );
    // A place a shell split into words is joined again, a negative number is no flag, and the options stand anywhere.
    let args = [
        "--limit", "3", "/gta", "-75.5,", "-818.25,", "326", "--near", "1,-2,3", "--json",
        "--copy", "lua", "--level", "mpone", "--mode", "story", "--kept", "k", "--keys", "x",
        "--fresh",
    ];
    let asked = question_of(&strings(&args)).expect("a question");
    assert_eq!(asked.install, "/gta");
    assert_eq!(asked.text, "-75.5, -818.25, 326");
    assert_eq!(asked.near, Some(Vec3::new(1.0, -2.0, 3.0)));
    assert_eq!((asked.limit, asked.copy.as_deref()), (3, Some("lua")));
    let signed = question_of(&strings(&["/gta", "-1234"])).expect("a question");
    assert_eq!(signed.text, "-1234");
    // A value written after `=`, as the verbs beside this one read a flag, is its flag's and no word of the text.
    let args = ["/gta", "bench", "--limit=3", "--near=1,-2,3", "--copy=json"];
    let joined = question_of(&strings(&args)).expect("a question");
    assert_eq!(joined.text, "bench");
    assert_eq!(joined.near, Some(Vec3::new(1.0, -2.0, 3.0)));
    assert_eq!((joined.limit, joined.copy.as_deref()), (3, Some("json")));

    for (refused, why) in [
        (&[][..], "<install>"),
        (&["/gta"], "<text>"),
        (&["/gta", "bench", "--game", "x"], "`--game` is no flag"),
        (&["/gta", "bench", "--game=x"], "`--game=x` is no flag"),
        (&["/gta", "bench", "--json=yes"], "`--json=yes` is no flag"),
        (&["/gta", "bench", "--limit"], "`--limit` takes a value"),
        (&["/gta", "bench", "--limit", "many"], "--limit is a count"),
        (&["/gta", "bench", "--near", "1,2"], "has no height"),
        (&["/gta", "bench", "--near", "here"], "--near is a view"),
    ] {
        let error = question_of(&strings(refused)).expect_err("refused");
        assert!(error.contains(why), "{refused:?}: {error}");
    }
    let forms = ScriptForms::built_in().expect("the forms of the tree");
    for form in &forms.forms {
        assert_eq!(form_of(&forms, &form.id).expect("a form").id, form.id);
    }
    let error = form_of(&forms, "python").expect_err("no such form");
    assert!(
        error.contains("lua, json, csharp") && error.contains("`python`"),
        "{error}"
    );
}

/// Every string a value of the answer's JSON states, where the text must state it too.
fn strings_of(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => out.push(text.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|item| strings_of(item, out)),
        serde_json::Value::Object(members) => {
            members.values().for_each(|member| strings_of(member, out))
        }
        _ => {}
    }
}

/// A position of the JSON as a line writes one.
fn point_of(value: &serde_json::Value) -> String {
    let at = value.as_array().expect("a position is three numbers");
    let at: Vec<f32> = at.iter().map(number_of).collect();
    assert_eq!(at.len(), 3, "{value}");
    point(Vec3::new(at[0], at[1], at[2]))
}

/// A number of the JSON as the 32 bits it was written from.
fn number_of(value: &serde_json::Value) -> f32 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is no number")) as f32
}

/// Hold the lines of an answer to its JSON, each line written again here from the row of the JSON it states: a name, a path,
/// a hash, a count or a number the text states and the JSON lacks, or the JSON states and the text lacks, is a line that is
/// not there, and the text has no line the JSON has no row for.
fn the_text_lists_what_the_json_lists(said: &Said) {
    let (lines, json) = (&said.lines, &said.json);
    let text = lines.join("\n");
    let rows = |key: &str| json[key].as_array().cloned().unwrap_or_default();
    let holds = |line: String| assert!(lines.contains(&line), "no line is {line:?}:\n{text}");
    let mut stated = Vec::new();
    for key in ["place", "hash", "models", "placements", "maps", "notes"] {
        strings_of(&json[key], &mut stated);
    }
    for string in &stated {
        assert!(
            text.contains(string.as_str()),
            "the text lacks {string:?}:\n{text}"
        );
    }
    // One line for the question, and then each group's.
    let mut expected = 1;
    holds(format!("Find `{}`", json["text"].as_str().expect("a text")));
    if let Some(place) = json["place"].as_object() {
        let maps = place["maps"].as_array().expect("maps");
        let bounds = place["bounds"].as_array().expect("bounds");
        expected += 5 + maps.len() + bounds.len();
        let (x, y) = (number_of(&place["x"]), number_of(&place["y"]));
        let typed = match &place["z"] {
            serde_json::Value::Null => format!("({x}, {y}), with no height"),
            z => point(Vec3::new(x, y, number_of(z))),
        };
        let heading = match &place["heading"] {
            serde_json::Value::Null => String::new(),
            heading => format!(", heading {}", number_of(heading)),
        };
        holds(format!("Place {typed}{heading}"));
        match place["ground"].as_object() {
            Some(ground) => {
                let from = match ground["bound"].as_str() {
                    Some(bound) => format!(", from {bound}"),
                    None => String::new(),
                };
                holds(format!(
                    "  ground {:.3}{from}",
                    number_of(&ground["height"])
                ));
            }
            None => holds("  ground none: no bounds file holds ground there".to_string()),
        }
        match &place["goTo"] {
            serde_json::Value::Null => {
                holds("  go to nowhere: it has no height and no ground".to_string())
            }
            at => holds(format!("  go to {}", point_of(at))),
        }
        holds(format!(
            "  {} map(s) whose streaming extents hold it, {} named",
            place["mapsHeld"],
            maps.len()
        ));
        holds(format!(
            "  {} bound(s) whose box holds it, {} named",
            place["boundsHeld"],
            bounds.len()
        ));
        let named = |file: &serde_json::Value| {
            let name = file["name"].as_str().expect("a name");
            format!("{name}  {}", file["logical"].as_str().expect("a path"))
        };
        maps.iter()
            .for_each(|map| holds(format!("    map    {}", named(map))));
        bounds
            .iter()
            .for_each(|bound| holds(format!("    bound  {}", named(bound))));
        assert_eq!(json["models"], serde_json::json!([]));
        assert_eq!(json["maps"], serde_json::json!([]));
    } else {
        let more = |key: &str| if json[key] == true { ", and more" } else { "" };
        let models = rows("models");
        holds(format!("Models: {}{}", models.len(), more("moreModels")));
        for model in &models {
            let name = model["name"]
                .as_str()
                .or(model["hash"].as_str())
                .expect("a name or a hash");
            let declared = match model["typeFile"].as_object() {
                Some(file) => format!(
                    "declared in {}  {}",
                    file["name"].as_str().expect("a name"),
                    file["logical"].as_str().expect("a path")
                ),
                None => "declared in no type file read".to_string(),
            };
            let count = model["placements"].as_u64().expect("a count");
            let placed = match (count, model["nearest"].as_object()) {
                (0, _) => "0 placement(s): no map places it, so it opens as a model".to_string(),
                (count, Some(nearest)) => format!(
                    "{count} placement(s), the nearest {:.3} m away at {} in {}",
                    number_of(&nearest["distance"]),
                    point_of(&nearest["position"]),
                    nearest["map"].as_str().expect("a map")
                ),
                (count, None) => format!("{count} placement(s)"),
            };
            holds(format!(
                "  {name}  {}  {}  {declared}  {placed}",
                model["hash"].as_str().expect("a hash"),
                model["rank"].as_str().expect("a rank")
            ));
        }
        let maps = rows("maps");
        holds(format!("Maps: {}{}", maps.len(), more("moreMaps")));
        for map in &maps {
            let centre = match &map["centre"] {
                serde_json::Value::Null => "no extents".to_string(),
                centre => format!("centre {}", point_of(centre)),
            };
            holds(format!(
                "  {}  {}  {}  {centre}  {}",
                map["name"].as_str().expect("a name"),
                map["hash"].as_str().expect("a hash"),
                map["rank"].as_str().expect("a rank"),
                map["logical"].as_str().expect("a path")
            ));
        }
        expected += 2 + models.len() + maps.len();
        if let Some(placements) = json["placements"].as_object() {
            let listed = placements["listed"].as_array().expect("listed");
            holds(format!(
                "Placements of {}: {} known, {} listed, {} map file(s) read",
                placements["model"].as_str().expect("a model"),
                placements["known"],
                listed.len(),
                placements["mapsRead"]
            ));
            for placement in listed {
                let at = match &placement["position"] {
                    serde_json::Value::Null => "no position".to_string(),
                    at => point_of(at),
                };
                let detail = match (placement["entity"].as_u64(), placement["notRead"].as_str()) {
                    (Some(entity), _) => format!(
                        "entity {entity}  guid {}  heading {:.3}",
                        placement["guid"],
                        number_of(&placement["heading"])
                    ),
                    (None, Some(why)) => format!("not read: {why}"),
                    (None, None) => "a grass batch".to_string(),
                };
                let away = match &placement["distance"] {
                    serde_json::Value::Null => String::new(),
                    distance => format!("  {:.3} m", number_of(distance)),
                };
                holds(format!(
                    "    {}  {}  {at}  {detail}{away}",
                    placement["map"].as_str().expect("a map"),
                    placement["logical"].as_str().unwrap_or("no file")
                ));
            }
            expected += 1 + listed.len();
        }
    }
    if let Some(hash) = json["hash"].as_object() {
        let names = hash["names"].as_array().expect("names");
        expected += 1 + names.len().max(1);
        let spelled = |key: &str| hash[key].as_str().expect("a spelling");
        let name = match hash["name"].as_str() {
            Some(name) => format!("name {name}"),
            None => "no name stated by the install".to_string(),
        };
        holds(format!(
            "Hash {}  unsigned {}  signed {}  {name}",
            spelled("hexadecimal"),
            spelled("unsigned"),
            spelled("signed")
        ));
        if names.is_empty() {
            holds("  names nothing in the install".to_string());
        }
        for named in names {
            let logical = || named["logical"].as_str().expect("a path");
            holds(match named["is"].as_str().expect("what it names") {
                "archetype" => match named["typeFile"].as_object() {
                    Some(file) => format!(
                        "  an archetype, declared in {}  {}",
                        file["name"].as_str().expect("a name"),
                        file["logical"].as_str().expect("a path")
                    ),
                    None => "  an archetype".to_string(),
                },
                "map" => format!("  a map  {}", logical()),
                "file" => format!(
                    "  a file ({})  {}",
                    named["extension"].as_str().expect("an extension"),
                    logical()
                ),
                other => panic!("{other} is nothing a hash names"),
            });
        }
    }
    let notes = rows("notes");
    expected += notes.len();
    for note in &notes {
        holds(format!("  note: {}", note.as_str().expect("a note")));
    }
    assert_eq!(
        lines.len(),
        expected,
        "the text has a line the JSON has no row for, or lacks one:\n{text}"
    );
}

#[test]
fn the_json_lists_what_the_text_lists() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let game = mount(&root);
    let bench = format!("0x{:08X}", hash("prop_bench"));
    let nameless = hash("prop_nameless").to_string();
    let town = (hash("m_town") as i32).to_string();
    let questions: Vec<Vec<&str>> = vec![
        vec!["prop_bench", "--near", "400,100,5"],
        vec!["prop_bench", "--near", "400,100,5", "--limit", "2"],
        vec!["bench"],
        vec!["bench", "--limit", "1"],
        vec!["prop_unplaced"],
        vec![&bench],
        vec![&nameless],
        vec![&town],
        vec!["0x00000001"],
        vec!["m_"],
        vec!["dock", "--limit", "0"],
        vec!["150", "150"],
        vec!["vector4(150.0, 150.0, 10.5, 90.0)"],
        vec!["-5000,", "-5000,", "0"],
        vec!["no_such_thing"],
    ];
    let mut seen = (0, 0, 0, 0, 0);
    for question in &questions {
        let said = say(&game, &root, &kept, question);
        the_text_lists_what_the_json_lists(&said);
        let json = &said.json;
        seen.0 += usize::from(!json["place"].is_null());
        seen.1 += usize::from(!json["hash"].is_null());
        seen.2 += json["models"].as_array().map_or(0, Vec::len);
        seen.3 += json["placements"]["listed"].as_array().map_or(0, Vec::len);
        seen.4 += json["maps"].as_array().map_or(0, Vec::len);
    }
    // ⚠ An answer that listed nothing would agree with itself: each group was listed, and more than once.
    assert!(
        seen.0 >= 3 && seen.1 >= 4 && seen.2 >= 8 && seen.3 >= 6 && seen.4 >= 4,
        "{seen:?}"
    );

    // What the groups hold, said once from the fixture's own lists.
    let said = say(&game, &root, &kept, &["prop_bench", "--near", "400,100,5"]);
    let json = &said.json;
    assert_eq!(json["models"][0]["name"], "prop_bench");
    assert_eq!(json["models"][0]["hash"], bench.as_str());
    assert_eq!(json["models"][0]["placements"], benches().len());
    assert_eq!(json["models"][0]["typeFile"]["name"], "props");
    assert_eq!(json["models"][0]["nearest"]["map"], "m_dock");
    let listed = json["placements"]["listed"].as_array().expect("listed");
    let maps: Vec<&str> = listed.iter().filter_map(|p| p["map"].as_str()).collect();
    assert_eq!(maps, ["m_dock", "m_town", "m_hill"]);
    for placement in listed {
        let (_, bench) = (benches().into_iter())
            .find(|(map, _)| placement["map"] == *map)
            .expect("a map of the fixture");
        // The bench is the second entity of its map, after the workbench.
        assert_eq!(placement["entity"], 1);
        assert_eq!(placement["guid"], bench.guid);
        assert_eq!(placement["position"], serde_json::json!(bench.position));
    }
    let nothing = say(&game, &root, &kept, &["no_such_thing"]);
    assert_eq!(nothing.json["models"], serde_json::json!([]));
    assert_eq!(nothing.json["maps"], serde_json::json!([]));
    assert!(nothing.json["place"].is_null() && nothing.json["hash"].is_null());
    assert!(
        nothing.lines.contains(&"Models: 0".to_string()),
        "{:?}",
        nothing.lines
    );
    let unplaced = say(&game, &root, &kept, &["prop_unplaced"]);
    assert!(
        unplaced
            .lines
            .iter()
            .any(|line| line.contains("0 placement(s): no map places it")),
        "{:?}",
        unplaced.lines
    );
}

#[test]
fn a_result_is_copied_as_a_script_takes_it_and_reads_back() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let game = mount(&root);
    let forms = ScriptForms::built_in().expect("the forms of the tree");
    for form in &forms.forms {
        // A place typed with its height and heading is copied as it was typed, to the decimals written.
        let said = say(
            &game,
            &root,
            &kept,
            &["12.5, -340.25, 28.125, 271.5", "--copy", &form.id],
        );
        let text = said
            .copy
            .expect("asked to copy")
            .expect("a place has a copy");
        let read = read_pasted(&text).expect("the copy reads");
        let wanted = PastedPlace {
            x: 12.5,
            y: -340.25,
            z: Some(28.125),
            heading: Some(271.5),
        };
        assert_eq!(read, Pasted::Place(wanted), "{text}");
        assert_eq!(said.json["copy"]["form"], form.id.as_str());
        assert_eq!(said.json["copy"]["text"], text.as_str());

        // Two numbers over the town's floor are copied at the floor's height, and over nothing there is nothing to copy.
        let floor = level_square([100.0, 100.0, 10.0], 100.0, 0x21);
        let height = floor.vertex_position(0).expect("a vertex")[2];
        let said = say(&game, &root, &kept, &["150 150", "--copy", &form.id]);
        let text = said
            .copy
            .expect("asked to copy")
            .expect("the ground gives a height");
        let Ok(Pasted::Place(read)) = read_pasted(&text) else {
            panic!("{text:?} is no place");
        };
        assert_eq!((read.x, read.y), (150.0, 150.0));
        assert!(
            (read.z.expect("a height") - height).abs() <= 0.0005,
            "{text}"
        );
        let said = say(&game, &root, &kept, &["-5000 -5000", "--copy", &form.id]);
        let why = said.copy.expect("asked to copy").expect_err("no height");
        assert!(why.contains("no height and no ground"), "{why}");
        assert!(said.json.get("copy").is_none());

        // A model named in full is copied as the list of its placements, nearest first, each reading back to where it stands.
        let args = ["prop_bench", "--near", "400,100,5", "--copy", &form.id];
        let said = say(&game, &root, &kept, &args);
        let text = said
            .copy
            .expect("asked to copy")
            .expect("a model has a copy");
        let mut rest = text.as_str();
        for map in ["m_dock", "m_town", "m_hill"] {
            let (_, bench) = (benches().into_iter())
                .find(|(m, _)| *m == map)
                .expect("a map");
            let one = form
                .place(Vec3::from(bench.position), Some(bench.heading), DECIMALS)
                .expect("written");
            let at = rest
                .find(&one)
                .unwrap_or_else(|| panic!("{text} lacks {one}"));
            rest = &rest[at + one.len()..];
            let Ok(Pasted::Place(read)) = read_pasted(&one) else {
                panic!("{one:?} is no place");
            };
            assert_eq!([read.x, read.y, read.z.expect("a height")], bench.position);
            assert_eq!(read.heading, Some(bench.heading));
        }
        assert_eq!(text.matches("\"prop_bench\"").count(), 3, "{text}");

        // A model nothing places, and a name that is no model's whole name, have nothing to copy.
        let said = say(&game, &root, &kept, &["prop_unplaced", "--copy", &form.id]);
        let why = said
            .copy
            .expect("asked to copy")
            .expect_err("nothing placed");
        assert!(why.contains("no map places prop_unplaced"), "{why}");
        let said = say(&game, &root, &kept, &["benc", "--copy", &form.id]);
        let why = said
            .copy
            .expect("asked to copy")
            .expect_err("no model named");
        assert!(why.contains("names no model in full"), "{why}");
    }
}

#[test]
fn text_with_the_shape_of_a_hash_that_is_none_is_refused() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let game = mount(&root);
    let question = question_of(&strings(&["install", "0x123456789"])).expect("a question");
    let Err(refused) = said(&game, &root, &kept, false, &question, None, None) else {
        panic!("a hash of nine digits was read");
    };
    assert!(refused.contains("0x123456789"), "{refused}");
}
