//! `world models` and `world vehicles` over an install built by hand: what their arguments ask, and that their JSON and
//! their text list the same rows.

use crate::world_fixture;

use or_title::install::vehicles::{LEVEL_VEHICLES, VEHICLE_FILE_TYPE};
use or_title::DataFileEntry;

use super::*;

use world_fixture::{
    cube, dlc_list_bytes, hash, manifest_bytes, mount_all, placing_map_bytes, typed_bytes,
    write_archive, write_pack, ArchetypeSpec, MapSpec, Placed,
};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// A vehicle data file stating `(model, game name, class, type)`.
fn vehicle_file(stated: &[(&str, &str, &str, &str)]) -> Vec<u8> {
    let items: String = (stated.iter())
        .map(|(model, game, class, kind)| {
            format!(
                "<Item><modelName>{model}</modelName><gameName>{game}</gameName>\
                 <type>{kind}</type><vehicleClass>{class}</vehicleClass></Item>"
            )
        })
        .collect();
    format!(
        "<CVehicleModelInfo__InitDataList><InitDatas>{items}</InitDatas>\
         </CVehicleModelInfo__InitDataList>"
    )
    .into_bytes()
}

/// How often the one map of the fixture places each model it places.
const PLACED: &[(&str, u32)] = &[("prop_bench", 3), ("prop_tower", 1)];

/// The base game's models: a bench, a tower, a crate nothing places that is an entry of a dictionary, one no file
/// names, and a shop that holds a crate and states textures no file of the install is named as.
fn base_models() -> Vec<ArchetypeSpec> {
    let sized = |x: f32, y: f32, z: f32| ([0.0; 3], [x, y, z]);
    vec![
        ArchetypeSpec {
            texture_dictionary: "props_txd",
            ..ArchetypeSpec::base("prop_bench", sized(2.0, 1.0, 1.0))
        },
        ArchetypeSpec {
            kind: or_world_gta5::ArchetypeKind::Time,
            texture_dictionary: "props_txd",
            ..ArchetypeSpec::base("prop_tower", sized(4.0, 4.0, 30.0))
        },
        ArchetypeSpec {
            drawable_dictionary: "furniture",
            ..ArchetypeSpec::base("prop_crate", sized(1.0, 1.0, 1.0))
        },
        ArchetypeSpec::base("prop_ghost", sized(3.0, 3.0, 3.0)),
        ArchetypeSpec {
            kind: or_world_gta5::ArchetypeKind::Mlo,
            entities: vec!["prop_crate"],
            texture_dictionary: "txd_nowhere",
            ..ArchetypeSpec::base("v_shop", sized(12.0, 8.0, 4.0))
        },
    ]
}

/// An install of five models, a map, a pack that declares the bench again and one model of its own, and the vehicles of
/// the level and of the pack.
fn install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let level = LEVEL_VEHICLES.trim_start_matches("common:/");
    let cars = vehicle_file(&[
        ("carone", "CARONE", "VC_SPORTS", "VEHICLE_TYPE_CAR"),
        ("boatone", "BOATONE", "VC_BOAT", "VEHICLE_TYPE_BOAT"),
    ]);
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", dlc_list_bytes(&["alpha"])),
            ("data/_manifest.ymf", manifest_bytes(&[], &[])),
            (level, cars),
        ],
    );
    let placed: Vec<Placed> = (PLACED.iter())
        .flat_map(|(model, count)| {
            (0..*count).map(|n| Placed::new(model, [n as f32, 0.0, 0.0], 0.0, n))
        })
        .collect();
    let town = MapSpec::around(cube([0.0; 3], 100.0));
    write_archive(
        &root,
        "x64a.rpf",
        &[
            ("types/props.ytyp", typed_bytes("props", &base_models())),
            (
                "levels/m_town.ymap",
                placing_map_bytes("m_town", &town, &placed),
            ),
            ("models/prop_bench.ydr", b"a model".to_vec()),
            ("models/prop_tower.yft", b"a model".to_vec()),
            ("models/prop_crate.ydr", b"a model".to_vec()),
            ("models/furniture.ydd", b"models".to_vec()),
            ("models/v_shop.ydr", b"a model".to_vec()),
            ("models/props_txd.ytd", b"textures".to_vec()),
        ],
    );
    let sized = |x: f32| ([0.0; 3], [x, 1.0, 1.0]);
    let alpha = vec![
        ArchetypeSpec::base("prop_bench", sized(2.5)),
        ArchetypeSpec::base("prop_alpha", sized(6.0)),
    ];
    write_pack(
        &root,
        "alpha",
        10,
        // The pack registers a file it holds and one it does not.
        ["vehicles", "missing"]
            .map(|stem| DataFileEntry {
                filename: format!("dlc_alpha:/data/{stem}.meta"),
                file_type: VEHICLE_FILE_TYPE.into(),
                ..Default::default()
            })
            .to_vec(),
        vec![
            ("types/alpha.ytyp", typed_bytes("alpha", &alpha)),
            ("models/prop_alpha.ydr", b"a model".to_vec()),
            (
                "data/vehicles.meta",
                vehicle_file(&[
                    ("alphacar", "ALPHA", "VC_SPORTS", "VEHICLE_TYPE_CAR"),
                    ("carone", "CARONE", "VC_SUPER", "VEHICLE_TYPE_CAR"),
                ]),
            ),
        ],
    );
    (scratch, root)
}

/// What `world models <install> <args>` says of the fixture.
fn models(root: &Path, kept: &Path, args: &[&str]) -> Said {
    let args: Vec<&str> = ["install"].iter().chain(args).copied().collect();
    let asked = models_asked(&strings(&args)).expect("a question");
    let said = models_said(&mount_all(root), root, kept, false, &asked, None);
    let said = said.expect("the text is read");
    assert!(said.not_kept.is_none(), "{:?}", said.not_kept);
    said
}

/// The names of the rows of an answer's JSON, in its order; a row with no name by its hash.
fn names(said: &Said, key: &str) -> Vec<String> {
    let rows = said.json["rows"].as_array().expect("rows");
    let name = |row: &serde_json::Value| match &row[key] {
        serde_json::Value::String(name) => name.clone(),
        _ => row["hash"].as_str().expect("a hash").to_string(),
    };
    rows.iter().map(name).collect()
}

#[test]
fn the_arguments_of_models_are_an_install_a_text_and_its_filters() {
    let asked = models_asked(&strings(&["/gta"])).expect("a question");
    assert_eq!(
        asked,
        ModelsAsked {
            install: "/gta".into(),
            text: None,
            pack: None,
            kind: None,
            size: (None, None),
            placed: None,
            sort: None,
            descending: false,
            offset: 0,
            count: COUNT,
        }
    );
    let args = [
        "--pack",
        "dlc_alpha",
        "/gta",
        "prop",
        "bench",
        "--kind",
        "interior",
        "--min-size",
        "1.5",
        "--max-size=20",
        "--unplaced",
        "--sort",
        "size",
        "--desc",
        "--offset",
        "4",
        "--count",
        "2",
        "--level",
        "alpha",
        "--mode",
        "story",
        "--kept",
        "k",
        "--keys",
        "x",
        "--fresh",
        "--json",
    ];
    let asked = models_asked(&strings(&args)).expect("a question");
    assert_eq!(
        asked,
        ModelsAsked {
            install: "/gta".into(),
            text: Some("prop bench".into()),
            pack: Some("dlc_alpha".into()),
            kind: Some(ArchetypeKind::Mlo),
            size: (Some(1.5), Some(20.0)),
            placed: Some(false),
            sort: Some(Sort::Size),
            descending: true,
            offset: 4,
            count: 2,
        }
    );
    // A signed hash is a text and no flag, and the install may be `--game`'s.
    let signed = models_asked(&strings(&["--game", "/gta", "-1234"])).expect("a question");
    assert_eq!(
        (signed.install.as_str(), signed.text.as_deref()),
        ("/gta", Some("-1234"))
    );
    for (kind, word) in [(ArchetypeKind::Base, "base"), (ArchetypeKind::Time, "time")] {
        let asked = models_asked(&strings(&["/gta", "--kind", word])).expect("a question");
        assert_eq!(asked.kind, Some(kind));
        assert_eq!(kind_word(kind), word);
    }
    for (sort, word) in [(Sort::Name, "name"), (Sort::Placed, "placed")] {
        let asked = models_asked(&strings(&["/gta", "--sort", word])).expect("a question");
        assert_eq!(asked.sort, Some(sort));
    }
    let placed = models_asked(&strings(&["/gta", "--placed"])).expect("a question");
    assert_eq!(placed.placed, Some(true));

    for (refused, why) in [
        (&[][..], "<install>"),
        (&["/gta", "--near", "1,2,3"], "`--near` is no flag"),
        (&["/gta", "--json=yes"], "`--json=yes` is no flag"),
        (&["/gta", "--count"], "`--count` takes a value"),
        (&["/gta", "--count", "many"], "--count is a count"),
        (&["/gta", "--offset", "-1"], "--offset is a count"),
        (
            &["/gta", "--kind", "mlo"],
            "--kind is base, time or interior",
        ),
        (&["/gta", "--sort", "lod"], "--sort is name, size or placed"),
        (&["/gta", "--min-size", "big"], "--min-size is a length"),
        (&["/gta", "--max-size", "-2"], "--max-size is a length"),
        (
            &["/gta", "--min-size", "3", "--max-size", "2"],
            "is past --max-size",
        ),
        (&["/gta", "--placed", "--unplaced"], "leave no model"),
    ] {
        let error = models_asked(&strings(refused)).expect_err("refused");
        assert!(error.contains(why), "{refused:?}: {error}");
    }
}

#[test]
fn world_models_lists_each_model_with_what_the_index_states_and_says_it_in_text_too() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let said = models(&root, &kept, &[]);
    let json = &said.json;
    // Five models of the base game and one of the pack, the bench the pack declares again once.
    let declared = base_models().len() + 1;
    assert_eq!(json["models"], declared);
    assert_eq!(json["total"], declared);
    assert_eq!(json["unnamed"], 1);
    assert_eq!(
        json["kinds"],
        json!({ "base": 4, "time": 1, "interior": 1 })
    );
    assert_eq!(
        json["packs"],
        json!([
            { "pack": "common", "models": 0 },
            { "pack": "platform", "models": 4 },
            { "pack": "dlc_alpha", "models": 2 },
        ])
    );
    let ghost = hex(hash("prop_ghost"));
    assert_eq!(
        names(&said, "name"),
        [
            "prop_alpha",
            "prop_bench",
            "prop_crate",
            "prop_tower",
            "v_shop",
            ghost.as_str()
        ],
        "by name, the model no file names last and by its hash"
    );
    let row = |name: &str| -> serde_json::Value {
        let rows = json["rows"].as_array().expect("rows");
        let row = rows.iter().find(|row| row["name"] == name);
        row.unwrap_or_else(|| panic!("{name} is not listed"))
            .clone()
    };
    let bench = row("prop_bench");
    assert_eq!(bench["hash"], hex(hash("prop_bench")));
    assert_eq!(bench["kind"], "base");
    assert_eq!(
        bench["declared"],
        json!({ "typeFile": "alpha", "logical": "dlc_alpha:/types/alpha.ytyp", "pack": "dlc_alpha" })
    );
    assert_eq!(
        bench["replaced"],
        json!([{ "typeFile": "props", "logical": "platform:/types/props.ytyp", "pack": "platform" }])
    );
    assert_eq!(
        bench["box"],
        json!({ "min": [0.0, 0.0, 0.0], "max": [2.5, 1.0, 1.0] })
    );
    assert_eq!(bench["size"], 2.5);
    assert_eq!(bench["sphere"]["radius"], 2.5);
    assert_eq!(bench["lodDist"], 100.0);
    assert_eq!(
        bench["held"],
        json!({ "extension": "ydr", "logical": "platform:/models/prop_bench.ydr", "entry": false })
    );
    let placed_of = |model: &str| PLACED.iter().find(|placed| placed.0 == model).map(|p| p.1);
    assert_eq!(bench["placed"], placed_of("prop_bench").expect("placed"));
    assert_eq!(
        (&bench["inside"], &bench["isPlaced"]),
        (&json!(0), &json!(true))
    );
    // The pack's declaration states no texture dictionary, and the base game's is no longer the bench's.
    assert_eq!(bench["textureDictionary"], json!(null));
    let tower = row("prop_tower");
    assert_eq!(
        (&tower["kind"], &tower["size"]),
        (&json!("time"), &json!(30.0))
    );
    assert_eq!(tower["held"]["extension"], "yft");
    // A dictionary is said by its hash, and by its name where a file of the install has it.
    let named =
        |name: &str, file: bool| json!({ "hash": hex(hash(name)), "name": file.then_some(name) });
    assert_eq!(tower["textureDictionary"], named("props_txd", true));
    assert_eq!(
        row("v_shop")["textureDictionary"],
        named("txd_nowhere", false)
    );
    assert_eq!(tower["drawableDictionary"], json!(null));
    let held = row("prop_crate");
    assert_eq!(
        (&held["placed"], &held["inside"], &held["isPlaced"]),
        (&json!(0), &json!(1), &json!(true))
    );
    // The crate is an entry of the dictionary it states, though a drawable has its name.
    assert_eq!(held["drawableDictionary"], named("furniture", true));
    assert_eq!(
        held["held"],
        json!({ "extension": "ydd", "logical": "platform:/models/furniture.ydd", "entry": true })
    );
    let rows = json["rows"].as_array().expect("rows");
    let unnamed = rows.iter().find(|row| row["hash"] == ghost);
    let unnamed = unnamed.expect("the model no file names");
    assert_eq!(
        (&unnamed["name"], &unnamed["held"], &unnamed["isPlaced"]),
        (&json!(null), &json!(null), &json!(false))
    );
    assert_eq!(
        json["timing"]["filesOpened"], 4,
        "a map, two type files and a manifest"
    );

    // The text lists every row the JSON does, in its order, each with its pack and its file.
    let text = said.lines.join("\n");
    let mut from = 0;
    for name in names(&said, "name") {
        let at = text[from..].find(&format!("\n  {name}  "));
        from += at.unwrap_or_else(|| panic!("{name} is not listed in order:\n{text}"));
    }
    for stated in [
        "6 model(s): 4 base, 1 time, 1 interior; 1 named by no file",
        "6 answer, 6 listed from 0",
        "2.500 m  lod 100  placed 3 time(s)  alpha in dlc_alpha, over platform  held by platform:/models/prop_bench.ydr",
        "never placed",
        "placed 0 time(s), 1 inside an interior",
        "held by no file",
        "held by platform:/models/furniture.ydd  drawables furniture",
        "held by platform:/models/prop_tower.yft  textures props_txd",
    ] {
        assert!(text.contains(stated), "`{stated}` is not said:\n{text}");
    }

    // A second question reads the kept index and opens no file.
    let again = models(&root, &kept, &[]);
    assert_eq!(again.json["timing"]["filesOpened"], 0);
    assert_eq!(again.json["rows"], json["rows"]);
}

#[test]
fn each_flag_of_world_models_filters_orders_or_pages_the_list() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let ghost = hex(hash("prop_ghost"));
    let asked = |args: &[&str]| {
        let said = models(&root, &kept, args);
        let total = said.json["total"].as_u64().expect("a total");
        (names(&said, "name"), total)
    };
    assert_eq!(
        asked(&["--kind", "interior"]),
        (vec!["v_shop".to_string()], 1)
    );
    assert_eq!(asked(&["--kind", "time"]).0, ["prop_tower"]);
    assert_eq!(
        asked(&["--pack", "dlc_alpha"]).0,
        ["prop_alpha", "prop_bench"]
    );
    assert_eq!(
        asked(&["--pack", "DLC_Alpha", "--placed"]).0,
        ["prop_bench"]
    );
    assert_eq!(
        asked(&["--unplaced"]).0,
        ["prop_alpha", "v_shop", ghost.as_str()]
    );
    assert_eq!(
        asked(&["--min-size", "2.5", "--max-size", "6"]).0,
        ["prop_alpha", "prop_bench", ghost.as_str()]
    );
    assert_eq!(
        asked(&["--sort", "size", "--desc"]).0,
        [
            "prop_tower",
            "v_shop",
            "prop_alpha",
            ghost.as_str(),
            "prop_bench",
            "prop_crate"
        ]
    );
    assert_eq!(
        asked(&["--sort", "placed", "--desc", "--count", "2"]).0,
        ["prop_bench", "prop_crate"]
    );
    assert_eq!(
        asked(&["prop_", "--sort", "size"]).0,
        ["prop_crate", "prop_bench", "prop_alpha", "prop_tower"]
    );
    // A page is a slice of the answer, and the total is the answer's.
    assert_eq!(
        asked(&["--offset", "1", "--count", "2"]),
        (vec!["prop_bench".to_string(), "prop_crate".to_string()], 6)
    );
    assert_eq!(asked(&["--offset", "9"]), (Vec::new(), 6));

    // A text is ranked as `world find` ranks it, and a name no file states is found typed in full and by its hash.
    let ranked = |text: &str| {
        let said = models(&root, &kept, &[text]);
        let rows = said.json["rows"].as_array().expect("rows").clone();
        let ranks = rows.iter().map(|row| row["rank"].as_str().expect("a rank"));
        let ranks: Vec<String> = ranks.map(str::to_string).collect();
        names(&said, "name")
            .into_iter()
            .zip(ranks)
            .collect::<Vec<_>>()
    };
    let row = |name: &str, rank: &str| (name.to_string(), rank.to_string());
    assert_eq!(ranked("bench"), [row("prop_bench", "part")]);
    assert_eq!(ranked("prop_bench"), [row("prop_bench", "exact")]);
    assert_eq!(ranked("prop_ghost"), [row(&ghost, "exact")]);
    assert_eq!(ranked(&ghost), [row(&ghost, "exact")]);

    // A pack the mount lacks answers nothing, and the note names the packs it has.
    let nowhere = models(&root, &kept, &["--pack", "dlc_gamma"]);
    assert_eq!(nowhere.json["total"], 0);
    let notes = nowhere.json["notes"].as_array().expect("notes");
    let note = notes[0].as_str().expect("a note");
    assert!(
        note.contains("no pack `dlc_gamma`") && note.contains("common, platform, dlc_alpha"),
        "{note}"
    );
    assert!(nowhere.lines.join("\n").contains(note));
    assert_eq!(
        models(&root, &kept, &["--pack", "platform"]).json["notes"],
        json!([])
    );

    // A text that has the shape of a hash and is none is refused.
    let wide = models_asked(&strings(&["install", "0x1234567890"])).expect("a question");
    let refused = models_said(&mount_all(&root), &root, &kept, false, &wide, None);
    assert!(refused.is_err_and(|e| e.contains("does not fit a hash")));
}

#[test]
fn world_vehicles_lists_each_vehicle_with_its_pack_and_filters_by_class_type_pack_and_text() {
    let (_scratch, root) = install();
    let game = mount_all(&root);
    let say = |args: &[&str]| {
        let args: Vec<&str> = ["install"].iter().chain(args).copied().collect();
        let asked = vehicles_asked(&strings(&args)).expect("a question");
        vehicles_said(&game, &root, &asked, None)
    };
    let all = say(&[]);
    let json = &all.json;
    assert_eq!(json["vehicles"], 3);
    assert_eq!(json["total"], 3);
    assert_eq!(names(&all, "modelName"), ["alphacar", "boatone", "carone"]);
    assert_eq!(
        json["files"],
        json!([
            { "pack": "common", "path": LEVEL_VEHICLES, "vehicles": 2, "notRead": null },
            { "pack": "dlc_alpha", "path": "dlc_alpha:/data/vehicles.meta", "vehicles": 2, "notRead": null },
            {
                "pack": "dlc_alpha",
                "path": "dlc_alpha:/data/missing.meta",
                "vehicles": null,
                "notRead": "the mount holds no such file",
            },
        ])
    );
    // The car the level states is the pack's, which states it again in another class.
    assert_eq!(
        json["rows"][2],
        json!({
            "modelName": "carone",
            "gameName": "CARONE",
            "class": "VC_SUPER",
            "type": "VEHICLE_TYPE_CAR",
            "pack": "dlc_alpha",
            "file": "dlc_alpha:/data/vehicles.meta",
            "replaced": ["common"],
            "rank": null,
        })
    );
    assert_eq!(
        json["classes"],
        json!([
            { "class": "VC_BOAT", "vehicles": 1 },
            { "class": "VC_SPORTS", "vehicles": 1 },
            { "class": "VC_SUPER", "vehicles": 1 },
        ])
    );
    let text = all.lines.join("\n");
    for stated in [
        "3 vehicle(s) from 3 data file(s), 1 that gave none",
        "\n  note: dlc_alpha:/data/missing.meta of dlc_alpha gave no vehicle: the mount holds no such file",
        "classes: VC_BOAT: 1, VC_SPORTS: 1, VC_SUPER: 1",
        "types: VEHICLE_TYPE_BOAT: 1, VEHICLE_TYPE_CAR: 2",
        "  carone  CARONE  VC_SUPER  VEHICLE_TYPE_CAR  in dlc_alpha, over common  dlc_alpha:/data/vehicles.meta",
        "  boatone  BOATONE  VC_BOAT  VEHICLE_TYPE_BOAT  in common  common:/data/levels/gta5/vehicles.meta",
    ] {
        assert!(text.contains(stated), "`{stated}` is not said:\n{text}");
    }

    let listed = |args: &[&str]| names(&say(args), "modelName");
    assert_eq!(listed(&["--class", "VC_SPORTS"]), ["alphacar"]);
    assert_eq!(listed(&["--class", "vc_super"]), ["carone"]);
    assert_eq!(
        listed(&["--type", "VEHICLE_TYPE_CAR"]),
        ["alphacar", "carone"]
    );
    assert_eq!(listed(&["--pack", "common"]), ["boatone"]);
    assert_eq!(listed(&["one"]), ["boatone", "carone"]);
    assert_eq!(listed(&["one", "--type", "VEHICLE_TYPE_BOAT"]), ["boatone"]);
    assert_eq!(listed(&["--offset", "1", "--count", "1"]), ["boatone"]);
    assert_eq!(say(&["--offset", "1", "--count", "1"]).json["total"], 3);

    let asked = vehicles_asked(&strings(&[
        "--game", "/gta", "car", "--class", "VC_X", "--type", "T", "--pack=p", "--offset", "2",
        "--count", "7", "--level", "alpha", "--mode", "story", "--keys", "k", "--json",
    ]));
    assert_eq!(
        asked.expect("a question"),
        VehiclesAsked {
            install: "/gta".into(),
            text: Some("car".into()),
            class: Some("VC_X".into()),
            vehicle_type: Some("T".into()),
            pack: Some("p".into()),
            offset: 2,
            count: 7,
        }
    );
    for (refused, why) in [
        (&[][..], "<install>"),
        (&["/gta", "--kind", "base"], "`--kind` is no flag"),
        (&["/gta", "--class"], "`--class` takes a value"),
        (&["/gta", "--count", "x"], "--count is a count"),
    ] {
        let error = vehicles_asked(&strings(refused)).expect_err("refused");
        assert!(error.contains(why), "{refused:?}: {error}");
    }
}
