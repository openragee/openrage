//! `world select` over an install built by hand: what it says of a view, in lines and in JSON, and
//! how it reads its arguments.

use std::collections::BTreeSet;

use crate::forest_fixture::{
    arch, ent, install, manifest_bytes, map_around, map_file, types_file, Arch, Ent, MapOf, LOD,
    NO_LEVEL, ORPHAN,
};
use crate::world_fixture::{cube, mount};

use or_world::state::{flags as group_flags, NOT_BY_DEFAULT};

use super::*;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// Bits 22 and 23 and 0 to 5: the hours of the night.
const NIGHT: u32 = (0b11 << 22) | 0b11_1111;

/// A district: a far model in its own map that stands for two near ones in another, a prop of an
/// archetype no type file declares, a map of the night, and the files of some of their models.
fn district() -> tempfile::TempDir {
    let far = Ent {
        lod_dist: 1000.0,
        child_lod_dist: 100.0,
        level: LOD,
        num_children: 2,
        ..ent("block_lod", [0.0; 3])
    };
    let near = |archetype, x| Ent {
        lod_dist: 40.0,
        parent: Some((0, true)),
        ..ent(archetype, [x, 0.0, 0.0])
    };
    let lost = Ent {
        lod_dist: 300.0,
        ..ent("prop_nowhere", [20.0, 20.0, 0.0])
    };
    let lod = map_around(cube([-200.0, -200.0, -10.0], 400.0), vec![far]);
    let hd = MapOf {
        parent: "m_block_lod",
        ..map_around(
            cube([-200.0, -200.0, -10.0], 400.0),
            vec![near("block_a", 30.0), near("block_b", -30.0), lost],
        )
    };
    let lamp = Ent {
        lod_dist: 300.0,
        ..ent("night_lamp", [10.0, 10.0, 0.0])
    };
    let night = MapOf {
        flags: NOT_BY_DEFAULT,
        ..map_around(cube([-200.0, -200.0, -10.0], 400.0), vec![lamp])
    };
    let types = [
        arch("block_lod", 500.0),
        Arch {
            files: ("block_a", "", "block_txd"),
            ..arch("block_a", 100.0)
        },
        Arch {
            files: ("block_b", "block_dict", "block_txd"),
            ..arch("block_b", 100.0)
        },
        Arch {
            files: ("night_lamp", "", "lamp_txd"),
            ..arch("night_lamp", 100.0)
        },
    ];
    let file = |path: &str| (path.to_string(), vec![0u8; 16]);
    let files = vec![
        map_file("m_block_lod", &lod),
        map_file("m_block", &hd),
        map_file("m_night", &night),
        types_file("block", &types),
        (
            "levels/_manifest.ymf".to_string(),
            manifest_bytes(&[("m_night", group_flags::TIMED, NIGHT, &[])]),
        ),
        // The far model and a near one are files of their own, one of them a fragment; the
        // dictionary of the other near one and the lamp's textures are in no archive.
        file("models/block_lod.ydr"),
        file("models/block_a.yft"),
        file("models/block_txd.ytd"),
        file("models/night_lamp.ydr"),
    ];
    install(&[("x64a.rpf", files)])
}

/// A quarter of what does not go well: a street whose parent map streams elsewhere and is held
/// back, a shop's interior and an instance of an archetype that is no interior, two entities that
/// state no level, an alley whose parent map is in no archive, a yard whose parent map's file is
/// no map, and a map of the rain.
fn quarter() -> tempfile::TempDir {
    let plain = |at: [f32; 3]| Ent {
        lod_dist: 50.0,
        ..ent("plain", at)
    };
    let placing = |archetype, x| Ent {
        lod_dist: 60.0,
        level: ORPHAN,
        interior: true,
        ..ent(archetype, [x, 0.0, 0.0])
    };
    let around = |entities| map_around(cube([-100.0, -100.0, -10.0], 200.0), entities);
    let street = MapOf {
        parent: "m_street_lod",
        ..around(vec![
            placing("v_shop", 0.0),
            placing("plain", 5.0),
            Ent {
                level: NO_LEVEL,
                ..plain([3.0, 0.0, 0.0])
            },
            Ent {
                parent: Some((0, true)),
                ..plain([0.0, 5.0, 0.0])
            },
        ])
    };
    let far = Ent {
        lod_dist: 1000.0,
        child_lod_dist: 100.0,
        level: LOD,
        num_children: 1,
        ..ent("plain_lod", [0.0; 3])
    };
    let street_lod = MapOf {
        flags: NOT_BY_DEFAULT,
        streaming: cube([5000.0, 5000.0, 0.0], 10.0),
        extents: cube([5001.0, 5001.0, 1.0], 1.0),
        ..around(vec![far])
    };
    let alley = MapOf {
        parent: "m_gone",
        ..around(vec![Ent {
            parent: Some((0, true)),
            ..plain([0.0, 10.0, 0.0])
        }])
    };
    let yard = MapOf {
        parent: "m_bad",
        ..around(vec![
            plain([0.0, 20.0, 0.0]),
            Ent {
                level: NO_LEVEL,
                ..plain([0.0, 25.0, 0.0])
            },
        ])
    };
    let rain = MapOf {
        flags: NOT_BY_DEFAULT,
        ..around(vec![plain([0.0, 30.0, 0.0])])
    };
    let counter = Ent {
        lod_dist: 30.0,
        ..ent("shop_counter", [1.0, 0.0, 0.0])
    };
    let types = [
        Arch {
            interior: Some(vec![counter]),
            ..arch("v_shop", 0.0)
        },
        arch("plain", 100.0),
        arch("plain_lod", 500.0),
        arch("shop_counter", 100.0),
    ];
    let files = vec![
        map_file("m_street", &street),
        map_file("m_street_lod", &street_lod),
        map_file("m_alley", &alley),
        map_file("m_yard", &yard),
        map_file("m_rain", &rain),
        ("levels/m_bad.ymap".to_string(), b"no map at all".to_vec()),
        types_file("quarter", &types),
        (
            "levels/_manifest.ymf".to_string(),
            manifest_bytes(&[("m_rain", group_flags::WEATHER, 0, &["RAIN"])]),
        ),
    ];
    install(&[("x64a.rpf", files)])
}

/// What `world select <args>` says of the district: its lines and its JSON.
fn say(dir: &tempfile::TempDir, args: &[&str]) -> (Vec<String>, serde_json::Value) {
    let root = dir.path().to_string_lossy().into_owned();
    let mut all = vec![root.as_str()];
    all.extend(args);
    let (install, asked) = asked_of(&strings(&all)).expect("a view");
    assert_eq!(install, root);
    let game = mount(dir.path());
    let kept = dir.path().join("world.index");
    let (lines, json, not_kept) = said(&game, dir.path(), &kept, false, &asked, None);
    assert!(not_kept.is_none(), "{not_kept:?}");
    (lines, json)
}

/// The lines of `lines` that begin with `label` after their indent, each less the label.
fn labelled<'a>(lines: &'a [String], label: &str) -> Vec<&'a str> {
    let rest = |line: &'a String| line.trim_start().strip_prefix(label).map(str::trim_start);
    lines.iter().filter_map(rest).collect()
}

/// The path the index names a file of `x64a.rpf` by.
fn logical(path: &str) -> String {
    format!("platform:/{path}")
}

#[test]
fn a_view_is_said_with_its_maps_its_instances_by_level_and_the_files_they_need() {
    let dir = district();
    // 10 m from the far model: its two near ones stand in for it.
    let (lines, json) = say(&dir, &["10,0,0"]);
    assert_eq!(
        lines[0],
        "View at (10, 0, 0), 12 h, EXTRASUNNY: 2 map(s) wanted, 0 that gave no entities; \
         0 interior(s) brought in, 0 that gave none"
    );
    assert_eq!(
        labelled(&lines, "map "),
        [
            format!(
                "m_block  {}  the view is within its box",
                logical("levels/m_block.ymap")
            ),
            format!(
                "m_block_lod  {}  the view is within its box",
                logical("levels/m_block_lod.ymap")
            ),
        ]
    );
    assert!(lines.contains(&"  3 instance(s) selected, every model taken as loaded".to_string()));
    assert!(lines
        .contains(&"    HD 3, LOD 0, SLOD1 0, SLOD2 0, SLOD3 0, ORPHAN_HD 0, SLOD4 0".to_string()));
    let nowhere = format!("{:#010X}", or_io::string_hash(b"prop_nowhere"));
    assert_eq!(
        labelled(&lines, "box "),
        [format!(
            "{nowhere}  entity 2 of m_block: no type file of the mount declares its archetype"
        )]
    );
    assert!(lines.contains(&"  3 file(s) needed, 1 that the mount lacks".to_string()));
    let dictionary = format!("{:#010X}", or_io::string_hash(b"block_dict"));
    assert_eq!(
        labelled(&lines, "fragment "),
        [format!("block_a  {}", logical("models/block_a.yft"))]
    );
    assert_eq!(
        labelled(&lines, "drawable dictionary "),
        [format!("{dictionary}  not in the mount")]
    );
    assert_eq!(
        labelled(&lines, "texture dictionary "),
        [format!("block_txd  {}", logical("models/block_txd.ytd"))]
    );

    // 300 m off, the far model alone, which is a drawable of its own name with no textures stated.
    let (lines, json_far) = say(&dir, &["300,0,0"]);
    assert!(lines
        .contains(&"    HD 1, LOD 1, SLOD1 0, SLOD2 0, SLOD3 0, ORPHAN_HD 0, SLOD4 0".to_string()));
    assert_eq!(
        labelled(&lines, "drawable "),
        [format!("block_lod  {}", logical("models/block_lod.ydr"))]
    );
    assert_eq!(json_far["instances"]["byLevel"]["LOD"], json!(1));
    assert_eq!(json["instances"]["byLevel"]["LOD"], json!(0));
}

/// Hold the JSON of one view to its lines: every map, every file that gave nothing, every interior, the counts, every box, every file needed and what did not fit.
fn held_to_its_lines(lines: &[String], json: &serde_json::Value, args: &[&str]) {
    let text = |of: &serde_json::Value, key: &str| of[key].as_str().expect("text").to_string();
    let list = |key: &str| json[key].as_array().expect("a list").clone();
    // Every map, by its name and its path.
    let maps: Vec<String> = (list("maps").iter())
        .map(|m| {
            format!(
                "{}  {}  {}",
                text(m, "name"),
                text(m, "logical"),
                text(m, "why")
            )
        })
        .collect();
    assert_eq!(labelled(lines, "map "), maps, "{args:?}");
    assert!(!maps.is_empty());
    // Every map that gave nothing, with why.
    let unread: Vec<String> = (list("unread").iter())
        .map(|m| {
            format!(
                "{}  {}: {}",
                text(m, "name"),
                text(m, "logical"),
                text(m, "why")
            )
        })
        .collect();
    assert_eq!(labelled(lines, "unread "), unread, "{args:?}");
    // Every interior, brought in or not, by its archetype and the entity that places it.
    let interior = |i: &serde_json::Value| {
        let placed = format!(
            "{}  entity {} of {}",
            text(i, "archetype"),
            i["entity"],
            text(i, "map")
        );
        match i["why"].as_str() {
            Some(why) => format!("{placed}: {why}"),
            None => placed,
        }
    };
    let (brought, not_brought) = (list("interiors"), list("interiorsUnread"));
    assert!(brought.iter().all(|i| i["why"].is_null()));
    assert!(not_brought.iter().all(|i| i["why"].is_string()));
    let interiors: Vec<String> = (brought.iter().chain(&not_brought)).map(interior).collect();
    assert_eq!(labelled(lines, "interior "), interiors, "{args:?}");
    assert!(
        lines[0].ends_with(&format!(
            ": {} map(s) wanted, {} that gave no entities; {} interior(s) brought in, {} that gave none",
            maps.len(),
            unread.len(),
            brought.len(),
            not_brought.len()
        )),
        "{args:?}: {}",
        lines[0]
    );
    // The count at each level, and their sum.
    let by_level = json["instances"]["byLevel"].as_object().expect("levels");
    let counts: Vec<String> = (LEVELS.iter())
        .map(|level| format!("{level} {}", by_level[*level]))
        .collect();
    assert!(
        lines.contains(&format!("    {}", counts.join(", "))),
        "{args:?}"
    );
    let selected: u64 = by_level.values().filter_map(|count| count.as_u64()).sum();
    assert_eq!(json["instances"]["selected"], json!(selected), "{args:?}");
    assert!(
        (lines.iter()).any(|line| line.starts_with(&format!("  {selected} instance(s) selected"))),
        "{args:?}: {lines:?}"
    );
    // Every box, by its archetype, its place and its reason.
    let boxes: Vec<String> = (json["instances"]["boxes"].as_array().expect("boxes").iter())
        .map(|b| {
            format!(
                "{}  {}: {}",
                text(b, "archetype"),
                text(b, "placed"),
                text(b, "why")
            )
        })
        .collect();
    assert_eq!(labelled(lines, "box "), boxes, "{args:?}");
    // Every file, by its kind, its name and where the mount holds it.
    let files = list("files");
    let listed: BTreeSet<String> = (files.iter())
        .map(|f| {
            let logical = f["logical"].as_str().unwrap_or("not in the mount");
            format!("{}  {}  {logical}", text(f, "kind"), text(f, "name"))
        })
        .collect();
    let kinds = [
        "drawable",
        "fragment",
        "drawable dictionary",
        "texture dictionary",
    ];
    let printed: BTreeSet<String> = (kinds.iter())
        .flat_map(|kind| {
            let of = labelled(lines, &format!("{kind}  "));
            of.into_iter().map(move |rest| format!("{kind}  {rest}"))
        })
        .collect();
    assert_eq!(printed, listed, "{args:?}");
    assert_eq!(files.len(), listed.len());
    assert!(
        lines.contains(&format!(
            "  {} file(s) needed, {} that the mount lacks",
            files.len(),
            files.iter().filter(|f| f["logical"].is_null()).count()
        )),
        "{args:?}"
    );
    // What did not fit the rules is a line where there is any, with the two counts.
    let (no_level, lacking) = (&json["noLevel"], &json["parentMapLacking"]);
    let misfits = labelled(lines, "of the maps' entities, ");
    match (no_level.as_u64(), lacking.as_u64()) {
        (Some(0), Some(0)) => assert!(misfits.is_empty(), "{args:?}"),
        _ => assert_eq!(
            misfits,
            [format!(
                "{no_level} state(s) no level and stand(s) as an orphan; {lacking} name(s) a \
                 parent in a map the mount lacks and stand(s) alone"
            )],
            "{args:?}"
        ),
    }
}

#[test]
fn the_json_of_a_view_lists_what_its_lines_list() {
    let (district, quarter) = (district(), quarter());
    for (dir, args) in [
        (&district, vec!["10,0,0"]),
        (&district, vec!["300,0,0"]),
        (&district, vec!["10,0,0", "--hour", "23"]),
        (&district, vec!["10,0,0", "--distance", "25"]),
        (&quarter, vec!["0,0,0"]),
        (
            &quarter,
            vec!["0,0,0", "--weather", "rain", "--distance", "8"],
        ),
    ] {
        let (lines, json) = say(dir, &args);
        held_to_its_lines(&lines, &json, &args);
    }
}

#[test]
fn a_view_says_what_gave_nothing_what_an_interior_brought_and_why_a_parent_is_wanted() {
    let dir = quarter();
    let (lines, json) = say(&dir, &["0,0,0"]);
    assert_eq!(
        lines[0],
        "View at (0, 0, 0), 12 h, EXTRASUNNY: 4 map(s) wanted, 1 that gave no entities; \
         1 interior(s) brought in, 1 that gave none"
    );
    // The street's parent streams nowhere near and waits to be asked for: it comes with the street.
    assert_eq!(
        labelled(&lines, "map "),
        [
            format!(
                "m_alley  {}  the view is within its box",
                logical("levels/m_alley.ymap")
            ),
            format!(
                "m_street  {}  the view is within its box",
                logical("levels/m_street.ymap")
            ),
            format!(
                "m_street_lod  {}  the parent of m_street",
                logical("levels/m_street_lod.ymap")
            ),
            format!(
                "m_yard  {}  the view is within its box",
                logical("levels/m_yard.ymap")
            ),
        ]
    );
    assert_eq!(
        labelled(&lines, "unread "),
        [format!(
            "m_bad  {}: it does not read as a map",
            logical("levels/m_bad.ymap")
        )]
    );
    let hex = |name: &str| format!("{:#010X}", or_io::string_hash(name.as_bytes()));
    assert_eq!(
        labelled(&lines, "interior "),
        [
            format!("{}  entity 0 of m_street", hex("v_shop")),
            format!(
                "{}  entity 1 of m_street: the archetype the instance names is no interior",
                hex("plain")
            ),
        ]
    );
    // The alley's entity, the street's two, the yard's two and the shop's counter; the far model
    // of the street gives way to the one that names it.
    assert!(lines.contains(&"  6 instance(s) selected, every model taken as loaded".to_string()));
    assert!(lines
        .contains(&"    HD 4, LOD 0, SLOD1 0, SLOD2 0, SLOD3 0, ORPHAN_HD 2, SLOD4 0".to_string()));
    assert!(lines.contains(
        &"  of the maps' entities, 2 state(s) no level and stand(s) as an orphan; 1 name(s) a \
          parent in a map the mount lacks and stand(s) alone"
            .to_string()
    ));
    assert_eq!(
        (json["noLevel"].clone(), json["parentMapLacking"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(json["interiors"][0]["entity"], json!(0));
    assert_eq!(json["unread"][0]["name"], json!("m_bad"));

    // The weather lets the map of the rain in, whatever the hour.
    let names = |json: &serde_json::Value| -> Vec<String> {
        let maps = json["maps"].as_array().expect("maps").iter();
        maps.map(|m| m["name"].as_str().expect("a name").to_string())
            .collect()
    };
    assert!(!names(&json).contains(&"m_rain".to_string()));
    for hour in ["3", "15"] {
        let (_, rain) = say(&dir, &["0,0,0", "--weather", "rain", "--hour", hour]);
        assert!(names(&rain).contains(&"m_rain".to_string()), "{hour} h");
        assert_eq!(rain["instances"]["selected"], json!(7));
    }
}

#[test]
fn the_hour_lets_a_group_s_map_in_and_the_distance_lists_only_what_stands_within_it() {
    let dir = district();
    let names = |json: &serde_json::Value| -> Vec<String> {
        let maps = json["maps"].as_array().expect("maps").iter();
        maps.map(|m| m["name"].as_str().expect("a name").to_string())
            .collect()
    };
    let (_, noon) = say(&dir, &["10,0,0"]);
    let (lines, night) = say(&dir, &["10,0,0", "--hour", "23", "--weather", "rain"]);
    // In the mount's order, which within an archive is its entries'.
    assert_eq!(names(&noon), ["m_block", "m_block_lod"]);
    assert_eq!(names(&night), ["m_block", "m_block_lod", "m_night"]);
    assert!(lines[0].starts_with("View at (10, 0, 0), 23 h, RAIN: 3 map(s) wanted"));
    assert_eq!(
        (night["hour"].clone(), night["weather"].clone()),
        (json!(23), json!("RAIN"))
    );
    assert_eq!(night["instances"]["selected"], json!(4));
    assert_eq!(noon["instances"]["pastDistance"], json!(null));

    // Within 21 m of the place stand the near model 20 m off and the lamp 10 m off; the prop of
    // no archetype stands past it, and the other near model 40 m off.
    let nowhere = Vec3::new(20.0, 20.0, 0.0).distance(Vec3::new(10.0, 0.0, 0.0));
    assert!(nowhere > 21.0 && nowhere < 25.0);
    let (lines, near) = say(&dir, &["10,0,0", "--hour", "23", "--distance", "21"]);
    assert_eq!(near["distance"], json!(21.0));
    assert_eq!(near["instances"]["selected"], json!(2));
    assert_eq!(near["instances"]["pastDistance"], json!(2));
    assert!(lines.contains(
        &"  2 instance(s) selected within 21 m, 2 more past it, every model taken as loaded"
            .to_string()
    ));
    assert!(labelled(&lines, "box ").is_empty());
    // At 25 m the prop is within it, and is said as the box it stands as.
    let (lines, wider) = say(&dir, &["10,0,0", "--hour", "23", "--distance", "25"]);
    assert_eq!(wider["instances"]["selected"], json!(3));
    assert_eq!(labelled(&lines, "box ").len(), 1);
}

#[test]
fn the_install_comes_first_or_after_its_flag_and_a_place_is_three_numbers() {
    let asked = |args: &[&str]| asked_of(&strings(args));
    let place = Vec3::new(-75.0, -818.5, 326.0);
    let plain = Asked {
        at: place,
        distance: None,
        hour: HOUR,
        weather: WEATHER.to_string(),
    };
    let expected = Ok(("an install".to_string(), plain.clone()));
    assert_eq!(asked(&["an install", "-75,-818.5,326"]), expected);
    assert_eq!(asked(&["an install", "-75", "-818.5", "326"]), expected);
    assert_eq!(asked(&["--game", "an install", "-75,-818.5,326"]), expected);
    assert_eq!(asked(&["-75,-818.5,326", "--game", "an install"]), expected);
    assert_eq!(
        asked(&["--json", "an install", "-75, -818.5, 326"]),
        expected
    );
    // A flag's value is neither the install nor a coordinate.
    let narrowed = Asked {
        distance: Some(150.0),
        hour: 3,
        weather: "THUNDER".to_string(),
        ..plain
    };
    assert_eq!(
        asked(&[
            "--distance",
            "150",
            "an install",
            "--hour",
            "3",
            "-75",
            "-818.5",
            "326",
            "--weather",
            "thunder",
            "--kept",
            "a file",
        ]),
        Ok(("an install".to_string(), narrowed))
    );
    for (args, why) in [
        (
            vec!["an install", "1", "2"],
            "a place is three numbers, x,y,z, and 2 were given",
        ),
        (vec!["--json"], "the install's folder comes first"),
        (
            vec!["an install", "1,2,3", "--far"],
            "`--far` is no flag of this verb",
        ),
        (
            vec!["an install", "1,2,3", "--hour"],
            "`--hour` takes a value",
        ),
        (
            vec!["an install", "1,2,3", "--hour", "24"],
            "--hour is an hour of the day, 0 to 23, not `24`",
        ),
        (
            vec!["an install", "1,2,3", "--distance", "-1"],
            "--distance is metres, 0 or more, not `-1`",
        ),
    ] {
        assert_eq!(asked(&args), Err(why.to_string()), "{args:?}");
    }
}
