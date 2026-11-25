//! The doors of a pack built by hand: found from its sources and paired, each door rule fired by a map built to break it, and a
//! selection written in every door form, read back where the tree has a reader for the form.

use std::collections::BTreeSet;
use std::f32::consts::PI;
use std::path::{Path, PathBuf};

use glam::Vec3;
use or_maps::door_text::{copies, Copied, DoorCopy, DoorForm, DoorForms, Error as FormError};
use or_maps::doors::{
    built_in_patterns, doors_of, patterns_from_json, Error as PatternsError, SetDoors,
    DOOR_OPENS_FROM, DOOR_RULES,
};
use or_maps::import::{add_files, Incoming};
use or_maps::model::{Maps, Waiver};
use or_maps::place::{read_pasted, Pasted};
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_maps::{check, inspect, rule_of, Depth};

const DOOR: &str = "shop_door_l";
const GATE: &str = "yard_barrier_a";
const SOFA: &str = "sofa_a";
const CRATE: &str = "crate_a";

/// A leaf a metre wide, four centimetres thick and two metres high, whose origin is at the foot of one end.
fn leaf(name: &str) -> Archetype<'_> {
    Archetype::new(name, [0.0, -0.02, 0.0], [1.0, 0.02, 2.0])
}

/// A leaf of a gate as wide and as high, with a box four tenths of a metre thick.
const THICK: &str = "yard_door_thick";

fn types() -> Vec<u8> {
    types_file(
        "shop_types",
        &[
            leaf(DOOR),
            Archetype::new(THICK, [0.0, -0.2, 0.0], [1.0, 0.2, 2.0]),
            // Nothing in its name says door: its archetype states the extension.
            Archetype {
                door: true,
                ..leaf(GATE)
            },
            Archetype::new(SOFA, [-0.5, -0.5, 0.0], [0.5, 0.5, 1.0]),
            Archetype::new(CRATE, [-0.5; 3], [0.5; 3]),
        ],
    )
}

/// A maps module in `dir` whose one set holds a map file that places `entities` and the type file, the pack remembering `names`.
fn module(dir: &Path, entities: &[Entity<'_>], names: &[&str]) -> (PathBuf, Maps) {
    let folder = dir.join("maps");
    let map = map_file("shop", entities, ([-500.0; 3], [500.0; 3]));
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
                bytes: &types(),
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let mut model = Maps::read(&folder).unwrap();
    model.remember(names);
    model.write(&folder).unwrap();
    (folder, model)
}

fn one_set(folder: &Path, model: &Maps) -> SetDoors {
    let patterns = built_in_patterns().expect("the patterns of the tree read");
    let mut sets = doors_of(folder, model, None, &patterns);
    assert_eq!(sets.len(), 1);
    let set = sets.remove(0);
    assert_eq!(set.unread, Vec::<(String, String)>::new());
    set
}

const FIRST: [f32; 3] = [125.5, -340.25, 28.125];
const SECOND: [f32; 3] = [127.5, -340.25, 28.125];

/// Two leaves that face each other across two metres, a gate found by its extension, a crate, and a model nothing names.
fn a_shop(dir: &Path) -> (PathBuf, Maps) {
    module(
        dir,
        &[
            Entity::at(DOOR, FIRST, 1),
            Entity {
                yaw: PI,
                ..Entity::at(DOOR, SECOND, 2)
            },
            Entity::at(GATE, [150.0, -340.0, 28.0], 3),
            Entity::at(CRATE, [160.0, -340.0, 28.0], 4),
            Entity::at("a_model_nothing_names", [170.0, -340.0, 28.0], 5),
        ],
        &[DOOR, CRATE, SOFA],
    )
}

#[test]
fn the_doors_of_a_pack_are_found_from_its_sources_paired_and_opened() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = a_shop(dir.path());
    let set = one_set(&folder, &model);

    assert_eq!(set.entities, 5);
    assert_eq!(set.unnamed, 1, "the model nothing names, and not the gate");
    assert_eq!(set.colliders, 0);
    let found: Vec<(&str, usize, &str, Option<&str>)> = (set.doors.iter())
        .map(|d| (d.model.as_str(), d.entity, d.found_by, d.pattern.as_deref()))
        .collect();
    let gate = format!("hash_{:08X}", or_io::string_hash(GATE.as_bytes()));
    assert_eq!(
        found,
        vec![
            (DOOR, 0, "name", Some("*door*")),
            (DOOR, 1, "name", Some("*door*")),
            (gate.as_str(), 2, "extension", None),
        ]
    );
    assert_eq!(set.pairs, vec![[0, 1]]);
    assert_eq!(
        set.doors.iter().map(|d| d.pair).collect::<Vec<_>>(),
        vec![Some(1), Some(0), None]
    );
    assert_eq!(set.doors[0].position, FIRST);
    // A script is handed no `-0`.
    assert_eq!(set.doors[0].heading.to_bits(), 0.0_f32.to_bits());
    assert!((set.doors[1].heading - 180.0).abs() < 1e-3);
    for door in &set.doors {
        let leaf = door.leaf.as_ref().expect("its box is declared");
        assert_eq!(
            (leaf.motion, leaf.hinge, leaf.width),
            ("swing", Some("low"), 1.0)
        );
        assert_eq!(door.opens.len(), 2);
        for open in &door.opens {
            assert_eq!(
                (open.reaches, &open.blocked_by),
                (90.0, &None),
                "{}",
                door.id()
            );
        }
    }
    assert_eq!(set.findings, vec![]);
}

/// The door ids a list of lines names, in order.
fn doors_in_text(lines: &[String]) -> Vec<String> {
    (lines.iter())
        .filter_map(|line| line.trim_start().strip_prefix("door "))
        .filter_map(|rest| rest.split(' ').next())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_lines_of_a_set_name_the_doors_its_json_lists() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = a_shop(dir.path());
    let set = one_set(&folder, &model);

    let json = serde_json::to_value(&set).unwrap();
    let in_json: Vec<String> = (json["doors"].as_array().unwrap().iter())
        .map(|d| format!("{}#{}", d["source"].as_str().unwrap(), d["entity"]))
        .collect();
    assert_eq!(in_json.len(), 3);
    assert_eq!(doors_in_text(&set.lines()), in_json);
    assert_eq!(json["pairs"], serde_json::json!([[0, 1]]));
    assert_eq!(json["unnamed"], 1);
}

#[test]
fn a_source_that_is_not_there_or_does_not_cook_is_named_and_never_passed_over() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = a_shop(dir.path());
    let patterns = built_in_patterns().expect("the patterns of the tree read");
    let not_read = |set: &SetDoors| -> Vec<String> {
        set.unread
            .iter()
            .map(|(source, _)| source.clone())
            .collect()
    };

    // A type file whose XML no longer compiles: the doors found by name stay, with no box to read, and the gate its extension
    // found is gone with the file that stated it.
    let xml = folder
        .join("shop_types.metadata.src")
        .join(or_source::metadata::XML);
    std::fs::write(&xml, "<CMapTypes>").unwrap();
    let sets = doors_of(&folder, &model, None, &patterns);
    assert_eq!(not_read(&sets[0]), ["shop_types.metadata.src"]);
    assert_eq!(sets[0].doors.len(), 2);
    assert!(sets[0].doors.iter().all(|door| door.leaf.is_none()));
    let lines = sets[0].lines();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("  not read shop_types.metadata.src: ")),
        "{lines:?}"
    );

    // A map source that is no longer in the folder is said too: its doors are doors missed.
    std::fs::remove_dir_all(folder.join("shop.metadata.src")).unwrap();
    let sets = doors_of(&folder, &model, None, &patterns);
    assert_eq!(
        not_read(&sets[0]),
        ["shop.metadata.src", "shop_types.metadata.src"]
    );
    assert_eq!((sets[0].entities, sets[0].doors.len()), (0, 0));
    assert!(sets[0].unread[0].1.contains("not in the folder"));
}

fn codes(set: &SetDoors) -> Vec<(&'static str, Option<usize>, bool)> {
    (set.findings.iter())
        .map(|f| (f.code, f.place.entity, f.waived.is_some()))
        .collect()
}

#[test]
fn each_door_rule_fires_on_a_map_built_to_break_it() {
    // Two leaves hung three tenths of a metre apart on the same hand: one stands in the other.
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = module(
        dir.path(),
        &[
            Entity::at(DOOR, [0.0, 0.0, 0.0], 1),
            Entity::at(DOOR, [0.3, 0.0, 0.0], 2),
        ],
        &[DOOR],
    );
    let set = one_set(&folder, &model);
    assert_eq!(codes(&set), vec![("doors-overlap", Some(1), false)]);
    assert_eq!(set.pairs, Vec::<[usize; 2]>::new());

    // A sofa each side of a leaf, a tenth of a metre off it: the leaf stops at once both ways.
    let dir = tempfile::tempdir().unwrap();
    let sofas = [
        Entity::at(DOOR, [0.0, 0.0, 0.0], 1),
        Entity::at(SOFA, [0.6, 0.6, 0.0], 2),
        Entity::at(SOFA, [0.6, -0.6, 0.0], 3),
    ];
    let (folder, mut model) = module(dir.path(), &sofas, &[DOOR, SOFA]);
    let set = one_set(&folder, &model);
    assert_eq!(codes(&set), vec![("door-blocked", Some(0), false)]);
    let stops: Vec<&str> = (set.doors[0].opens.iter())
        .map(|open| open.blocked_by.as_deref().expect("stopped"))
        .collect();
    assert_eq!(
        stops,
        vec![
            "`sofa_a` at shop.metadata.src#1",
            "`sofa_a` at shop.metadata.src#2"
        ]
    );

    // A waiver with a reason sets the finding aside, and the rules of the pack know the code it names.
    model.waivers.push(Waiver {
        code: "door-blocked".into(),
        map: None,
        reason: "the sofas are moved by a script".into(),
    });
    let set = one_set(&folder, &model);
    assert_eq!(codes(&set), vec![("door-blocked", Some(0), true)]);
    let lines = set.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("  waived  door-blocked ")
                && line.ends_with("(waived: the sofas are moved by a script)")),
        "{lines:?}"
    );
    // A waiver that gives no reason sets nothing aside, and neither does one of another set; one of this set does.
    let waived_by = |reason: &str, map: Option<String>| {
        let mut model = model.clone();
        model.waivers = vec![Waiver {
            code: "door-blocked".into(),
            map,
            reason: reason.into(),
        }];
        let set = one_set(&folder, &model);
        let lines = set.lines();
        let warned = lines
            .iter()
            .any(|line| line.starts_with("  warning door-blocked "));
        (codes(&set), warned)
    };
    let (blocked, waived) = (
        vec![("door-blocked", Some(0), false)],
        vec![("door-blocked", Some(0), true)],
    );
    assert_eq!(waived_by("  ", None), (blocked.clone(), true));
    let another = Some("a-set-that-is-not-this-one".to_string());
    assert_eq!(waived_by("moved by a script", another), (blocked, true));
    let this = Some(set.set.clone());
    assert_eq!(waived_by("moved by a script", this), (waived, false));
    let facts = inspect(&folder, &model, Depth::Open);
    let of_the_pack = check(&model, &facts, None);
    assert!(!of_the_pack.iter().any(|f| f.code == "waiver-unknown"));
    for rule in DOOR_RULES {
        assert_eq!(rule_of(rule.code), Some(rule));
    }

    // One sofa leaves the leaf a way to open, and the rule is quiet; so is the rule about two doors on a pair that meets.
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = module(dir.path(), &sofas[..2], &[DOOR, SOFA]);
    assert_eq!(codes(&one_set(&folder, &model)), vec![]);
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = a_shop(dir.path());
    assert_eq!(codes(&one_set(&folder, &model)), vec![]);

    // A sofa each side that lets the leaf pass half of a quarter turn before it stops it: the door opens, and the rule is quiet.
    let dir = tempfile::tempdir().unwrap();
    let further = [
        Entity::at(DOOR, [0.0, 0.0, 0.0], 1),
        Entity::at(SOFA, [0.6, 1.3, 0.0], 2),
        Entity::at(SOFA, [0.6, -1.3, 0.0], 3),
    ];
    let (folder, model) = module(dir.path(), &further, &[DOOR, SOFA]);
    let set = one_set(&folder, &model);
    for open in &set.doors[0].opens {
        assert!(open.blocked_by.is_some(), "{open:?}");
        let opens_from = DOOR_OPENS_FROM.to_degrees();
        assert!(open.reaches > opens_from && open.reaches < 90.0, "{open:?}");
    }
    assert_eq!(codes(&set), vec![]);
}

#[test]
fn a_leaf_of_a_double_door_is_not_stopped_by_the_other_leaf() {
    // Two leaves of a gate that meet in the middle, each with a box so thick that the corner of one turns through the box of
    // the other as it opens: the other leaf opens with it, and is no furniture.
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = module(
        dir.path(),
        &[
            Entity::at(THICK, [0.0, 0.0, 0.0], 1),
            Entity {
                yaw: PI,
                ..Entity::at(THICK, [2.0, 0.0, 0.0], 2)
            },
        ],
        &[THICK],
    );
    let set = one_set(&folder, &model);
    assert_eq!(set.pairs, vec![[0, 1]]);
    for door in &set.doors {
        for open in &door.opens {
            assert_eq!((open.reaches, &open.blocked_by), (90.0, &None), "{open:?}");
        }
    }
    assert_eq!(codes(&set), vec![]);

    // A third leaf hung in the arc of the first is no leaf of its door, and stops it the way it turns to it.
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = module(
        dir.path(),
        &[
            Entity::at(THICK, [0.0, 0.0, 0.0], 1),
            Entity {
                yaw: PI,
                ..Entity::at(THICK, [2.0, 0.0, 0.0], 2)
            },
            Entity::at(THICK, [0.0, 0.5, 0.0], 3),
        ],
        &[THICK],
    );
    let set = one_set(&folder, &model);
    assert_eq!(set.pairs, vec![[0, 1]]);
    let stopped: Vec<bool> = (set.doors[0].opens.iter())
        .map(|open| open.blocked_by.is_some())
        .collect();
    assert_eq!(stopped, [true, false], "{:?}", set.doors[0].opens);
}

#[test]
fn the_door_patterns_are_data_and_a_file_that_would_name_every_model_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = module(dir.path(), &[Entity::at(CRATE, [0.0; 3], 1)], &[CRATE]);
    assert_eq!(one_set(&folder, &model).doors.len(), 0);

    let file = |doors: &str| {
        format!(r#"{{ "format": "openrage-door-patterns", "schema": 1, "doors": {doors} }}"#)
    };
    let extended = patterns_from_json(&file(r#"["*door*", "crate_*"]"#)).unwrap();
    let sets = doors_of(&folder, &model, None, &extended);
    assert_eq!(sets[0].doors.len(), 1);
    assert_eq!(sets[0].doors[0].pattern.as_deref(), Some("crate_*"));

    assert!(matches!(
        patterns_from_json(&file(r#"["*"]"#)),
        Err(PatternsError::Pattern(_))
    ));
    // The tree's own file says what is no door by a name that holds the word.
    let own = built_in_patterns().expect("the patterns of the tree read");
    assert_eq!(own.door_by("shop_door_l"), Some("*door*"));
    assert_eq!(own.door_by("shop_doorframe_l"), None);
    assert_eq!(own.door_by("shop_door_l_lod"), None);
    let excluding = file(r#"["*door*"], "not_doors": ["*"]"#);
    assert!(matches!(
        patterns_from_json(&excluding),
        Err(PatternsError::Pattern(_))
    ));
    let foreign = file(r#"["*door*"]"#).replace("openrage-door-patterns", "openrage-maps");
    assert!(matches!(
        patterns_from_json(&foreign),
        Err(PatternsError::Format(_))
    ));
    let newer = file(r#"["*door*"]"#).replace("\"schema\": 1", "\"schema\": 2");
    assert!(matches!(
        patterns_from_json(&newer),
        Err(PatternsError::Newer(2))
    ));
}

/// Half of the last decimal written, and what a 32-bit number of that size cannot tell apart.
fn within(read: f32, written: f32, decimals: usize) -> bool {
    let half = 0.5 * 10f64.powi(-(decimals as i32));
    let grain = f64::from(written.abs().max(1.0)) * f64::from(f32::EPSILON);
    (f64::from(read) - f64::from(written)).abs() <= half + grain
}

fn holds(places: &[[f32; 3]], position: [f32; 3], decimals: usize) -> bool {
    (places.iter()).any(|place| (0..3).all(|axis| within(place[axis], position[axis], decimals)))
}

/// Every place a Lua text states as a call of three numbers, read by the reader of pasted places.
fn places_in_lua(text: &str) -> Vec<[f32; 3]> {
    let mut places = Vec::new();
    for call in ["vector3(", "vec3("] {
        let mut rest = text;
        while let Some(at) = rest.find(call) {
            let from = &rest[at..];
            let end = from.find(')').expect("a call closes") + 1;
            // ⚠ `vec3(` is also the end of no other word here, but `vector3(` holds no `vec3(`, so neither is read twice.
            match read_pasted(&from[..end]) {
                Ok(Pasted::Place(place)) => {
                    places.push([place.x, place.y, place.z.expect("three numbers")])
                }
                other => panic!("{:?} read as {other:?}", &from[..end]),
            }
            rest = &from[end..];
        }
    }
    places
}

/// Every place a JSON text states as an object of `x`, `y` and `z`, wherever it is.
fn places_in_json(value: &serde_json::Value, into: &mut Vec<[f32; 3]>) {
    match value {
        serde_json::Value::Object(members) => {
            let axis = |name: &str| members.get(name).and_then(|v| v.as_f64());
            if let (Some(x), Some(y), Some(z)) = (axis("x"), axis("y"), axis("z")) {
                into.push([x as f32, y as f32, z as f32]);
            }
            members.values().for_each(|v| places_in_json(v, into));
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| places_in_json(v, into)),
        _ => {}
    }
}

#[test]
fn every_door_form_writes_both_leaves_of_a_pair_to_the_decimals_asked_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = a_shop(dir.path());
    let set = one_set(&folder, &model);
    // One leaf selects its double door whole, and the gate is a door alone.
    let selection = copies(&set, &[1, 2, 0]);
    assert!(matches!(
        selection.as_slice(),
        [Copied::Pair(a, b), Copied::Single(gate)]
            if a.position == Vec3::from(FIRST) && b.position == Vec3::from(SECOND)
                && gate.hash == or_io::string_hash(GATE.as_bytes())
    ));

    let forms = DoorForms::built_in().expect("the forms of the tree read");
    let ids: Vec<&str> = forms.forms.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(&ids[..2], ["lua", "json"]);
    assert!(
        ids.len() > 2,
        "forms named after door scripts beside the tree's own"
    );
    for form in &forms.forms {
        // The tree vouches for its own two forms and for no other.
        let own = ["lua", "json"].contains(&form.id.as_str());
        assert_eq!(form.unverified.is_none(), own, "{}", form.id);
        for decimals in [0, 1, 2, 4] {
            let text = form.write(&selection, decimals).unwrap();
            for position in [FIRST, SECOND] {
                for value in position {
                    let written = format!("{value:.decimals$}");
                    assert!(text.contains(&written), "{}: {written} in {text}", form.id);
                }
            }
            let places = if form.id == "json" {
                let read: serde_json::Value = serde_json::from_str(&text)
                    .unwrap_or_else(|e| panic!("{text} is not JSON: {e}"));
                let hash = read[0]["double"][0]["hash"].as_u64();
                assert_eq!(hash, Some(u64::from(or_io::string_hash(DOOR.as_bytes()))));
                assert_eq!(read[0]["double"][1]["heading"].as_f64(), Some(180.0));
                let mut places = Vec::new();
                places_in_json(&read, &mut places);
                places
            } else {
                places_in_lua(&text)
            };
            for position in [FIRST, SECOND] {
                assert!(
                    holds(&places, position, decimals),
                    "{}: {position:?} to {decimals} decimals among {places:?}",
                    form.id
                );
            }
            // A double door is one item, written about the point midway between its leaves, and the items count from one.
            if form.id == "qb-doorlock" {
                let middle = (Vec3::from(FIRST) + Vec3::from(SECOND)) * 0.5;
                let [x, y, z] = middle.to_array().map(|v| format!("{v:.decimals$}"));
                let midway = format!("textCoords = vec3({x}, {y}, {z})");
                assert!(text.contains(&midway), "{midway} in {text}");
                assert!(text.contains(&format!("Config.DoorList['{DOOR}_1']")));
                let gate = format!("hash_{:08X}_2", or_io::string_hash(GATE.as_bytes()));
                assert!(text.contains(&format!("Config.DoorList['{gate}']")));
            }
            match (&form.unverified, &form.comment) {
                (Some(why), Some(comment)) => {
                    let first = text.lines().next().unwrap();
                    assert_eq!(first, format!("{comment}{why}"), "{}", form.id);
                    assert!(why.contains("UNVERIFIED"), "{}", form.id);
                }
                (None, _) => assert!(text.starts_with(&form.open), "{}", form.id),
                (Some(_), None) => panic!("{} cannot say it is unverified", form.id),
            }
        }
    }
}

fn a_form(change: impl FnOnce(&mut serde_json::Value)) -> Result<DoorForms, FormError> {
    let mut file = serde_json::json!({
        "format": "openrage-door-forms",
        "schema": 1,
        "forms": [{
            "id": "mine",
            "name": "Mine",
            "open": "",
            "single": "{name} {x} {y} {z}",
            "pair": "{a_x} {a_y} {a_z} {b_x} {b_y} {b_z}",
            "separator": "\n",
            "close": ""
        }]
    });
    change(&mut file["forms"][0]);
    DoorForms::from_json(&file.to_string())
}

#[test]
fn a_door_form_that_drops_a_position_or_cannot_say_it_is_unverified_is_refused() {
    let door = DoorCopy {
        name: "a".into(),
        hash: 7,
        position: Vec3::new(1.0, 2.0, 3.0),
        heading: 90.0,
    };
    let mine = a_form(|_| {}).expect("a form of the same shape reads");
    let form: &DoorForm = mine.form("mine").unwrap();
    assert_eq!(
        form.write(&[Copied::Single(door.clone())], 1).unwrap(),
        "a 1.0 2.0 3.0"
    );
    assert!(matches!(
        a_form(|form| form["pair"] = "{a_x} {a_y} {a_z} {b_x} {b_y}".into()),
        Err(FormError::Lacks { template: "pair", placeholder, .. }) if placeholder == "b_z"
    ));
    assert!(matches!(
        a_form(|form| form["single"] = "{x} {y}".into()),
        Err(FormError::Lacks { template: "single", placeholder, .. }) if placeholder == "z"
    ));
    assert!(matches!(
        a_form(|form| form["single"] = "{x} {y} {z} {b_name}".into()),
        Err(FormError::Stray {
            template: "single",
            ..
        })
    ));
    // A form another project defines has to be able to say so in what it writes.
    assert!(matches!(
        a_form(|form| form["unverified"] = "not checked".into()),
        Err(FormError::Unmarked(id)) if id == "mine"
    ));
    assert!(matches!(
        a_form(|form| {
            form["unverified"] = "not\nchecked".into();
            form["comment"] = "-- ".into();
        }),
        Err(FormError::Mark(_))
    ));
    let marked = a_form(|form| {
        form["unverified"] = "not checked".into();
        form["comment"] = "-- ".into();
    })
    .unwrap();
    let text = marked.forms[0].write(&[Copied::Single(door.clone())], 0);
    assert_eq!(text.unwrap(), "-- not checked\na 1 2 3");

    // A double door's own template is held to the same: a placeholder of a door alone is filled by nothing there.
    for of_a_door in ["name", "hash", "hash_signed", "hash_hex", "heading"] {
        let stray = format!("{{a_x}} {{a_y}} {{a_z}} {{b_x}} {{b_y}} {{b_z}} {{{of_a_door}}}");
        assert!(
            matches!(
                a_form(|form| form["pair"] = stray.into()),
                Err(FormError::Stray { template: "pair", ref placeholder, .. }) if placeholder == of_a_door
            ),
            "{of_a_door}"
        );
    }
    // The point midway between the leaves is its own to write.
    let written = a_form(|form| {
        form["single"] = "{index}: {hash_hex} {x} {y} {z}".into();
        form["pair"] = "{index}: {a_x} {a_y} {a_z} {b_x} {b_y} {b_z} about {x} {y} {z}".into();
    })
    .unwrap();
    let other = DoorCopy {
        position: Vec3::new(2.0, 4.0, 3.0),
        ..door.clone()
    };
    let listed = [
        Copied::Pair(door.clone(), other),
        Copied::Single(door.clone()),
    ];
    assert_eq!(
        written.forms[0].write(&listed, 1).unwrap(),
        format!(
            "1: 1.0 2.0 3.0 2.0 4.0 3.0 about 1.5 3.0 3.0\n2: 0x{:08X} 1.0 2.0 3.0",
            door.hash
        )
    );
    assert!(matches!(
        a_form(|form| {
            form["unverified"] = "not checked".into();
            form["comment"] = "".into();
        }),
        Err(FormError::Unmarked(_))
    ));

    // A name is written inside the quotes of its form, and one that holds a line break is not written.
    let quoted = DoorCopy {
        name: r#"a"b\c'd"#.into(),
        ..door.clone()
    };
    let forms = DoorForms::built_in().expect("the forms of the tree read");
    let json = forms.form("json").unwrap();
    let text = json.write(&[Copied::Single(quoted.clone())], 0).unwrap();
    let read: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(read[0]["model"].as_str(), Some(quoted.name.as_str()));
    let lua = forms.form("lua").unwrap();
    let text = lua.write(&[Copied::Single(quoted.clone())], 0).unwrap();
    assert!(text.contains(r#"model = "a\"b\\c'd","#), "{text}");
    let broken = DoorCopy {
        name: "a\nb".into(),
        ..door.clone()
    };
    assert!(matches!(
        form.write(&[Copied::Single(broken)], 0),
        Err(FormError::Name(_))
    ));

    // A place a selection names that the set does not hold is no door, and a door named twice is written once.
    let dir = tempfile::tempdir().unwrap();
    let (folder, model) = a_shop(dir.path());
    let set = one_set(&folder, &model);
    assert!(matches!(
        copies(&set, &[2, 7, 2]).as_slice(),
        [Copied::Single(gate)] if gate.hash == or_io::string_hash(GATE.as_bytes())
    ));

    let far = DoorCopy {
        position: Vec3::new(f32::INFINITY, 0.0, 0.0),
        ..door.clone()
    };
    assert!(matches!(
        form.write(&[Copied::Single(far)], 2),
        Err(FormError::NotFinite)
    ));
    assert!(matches!(
        form.write(&[Copied::Single(door)], 4000),
        Err(FormError::Decimals(4000))
    ));
    let codes: BTreeSet<&str> = DOOR_RULES.iter().map(|rule| rule.code).collect();
    assert_eq!(
        codes.len(),
        DOOR_RULES.len(),
        "no two door rules share a code"
    );
}
