use serde_json::json;

use super::*;

/// A dump with every field `world dump` writes, of a camera that draws two nodes.
fn dump() -> Value {
    json!({
        "camera": { "pos": [1.0, 2.0, 3.0], "yaw": 0.0, "pitch": -10.0, "fov": 60.0 },
        "selected_eye": [1.0, 2.0, 3.0],
        "selection_ms": 4.5,
        "asked_ms": 0.5,
        "handed_ms": 0.25,
        "hour": 12.0,
        "ground": 1.5,
        "lod_scale": 1.0,
        "modes": 1,
        "groups_on": ["0000000A"],
        "streaming": {
            "maps_wanted": 2,
            "maps_loaded": 2,
            "maps_waiting": 0,
            "nodes": 9,
            "drawn": 2,
            "maps_read": 7,
            "map_waits_s": 0.75,
            "map_wait_most_s": 0.5,
            "map_oldest_s": 0.0,
        },
        "wanted_map_names": ["00000001", "00000002"],
        "loaded_map_names": ["00000001", "00000002"],
        "nodes_by_level": { "entity HD": 9 },
        "drawn_by_level": { "entity HD": 2 },
        "list_instances": 0,
        "drawn_nodes": ["00000001:0:1.000", "00000002:3:0.400"],
        "swaps": { "held": 0, "open": 0 },
        "jumps": 1,
        "cards": { "lists": 0, "records": 0 },
        "device_textures": ["0000000000000001:256:256x256"],
        "device_stand_ins": [],
        "frame": {
            "lit_by_the_game": true,
            "selected": 2,
            "seen": 2,
            "draws": 2,
            "models": 2,
            "textures": 1,
            "textures_unread": 0,
            "pages": 1,
            "bytes": 4096,
        },
        "models_whole": { "count": 2, "waits_s": 0.1, "most_s": 0.1 },
        "models_taken": { "count": 2, "waits_s": 0.1, "most_s": 0.1, "stale": 0 },
        "textures_taken": { "count": 1, "waits_s": 0.1, "most_s": 0.1, "stale": 0 },
    })
}

/// `dump` with the field at `path` (`OBJECT.MEMBER` for a member) set to `value`.
fn with(mut dump: Value, path: &str, value: Value) -> Value {
    match path.split_once('.') {
        Some((object, member)) => dump[object][member] = value,
        None => dump[path] = value,
    }
    dump
}

#[test]
fn the_step_names_a_dump_and_a_reference() {
    assert_eq!(
        parse("city.json C:\\ref frames\\city.json"),
        Ok(CompareDump {
            dump: "city.json".into(),
            reference: PathBuf::from("C:\\ref frames\\city.json"),
        })
    );
    for bad in ["", "city.json", "../city.json C:\\ref\\city.json"] {
        assert!(parse(bad).is_err(), "`comparedump {bad}` should be refused");
    }
}

#[test]
fn a_dump_is_no_different_from_itself_and_every_field_it_writes_is_in_one_list() {
    assert_eq!(differences(&dump(), &dump()), Ok(Vec::new()));
    let both: BTreeSet<&str> = COMPARED.iter().chain(&LEFT_OUT).copied().collect();
    assert_eq!(
        both.len(),
        COMPARED.len() + LEFT_OUT.len(),
        "no field is both compared and left out"
    );
    let written: BTreeSet<String> = (fields(dump().as_object().expect("an object")).into_iter())
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        written,
        both.iter().map(|name| name.to_string()).collect(),
        "the lists name what a dump holds and nothing else"
    );
}

#[test]
fn each_compared_field_fails_alone_and_each_field_left_out_fails_nothing() {
    for name in COMPARED {
        let changed = with(dump(), name, json!("changed"));
        let found = differences(&changed, &dump()).expect("two dumps");
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        assert!(found[0].starts_with(name), "{found:?}");
    }
    for name in LEFT_OUT {
        let changed = with(dump(), name, json!(123_456.5));
        assert_eq!(differences(&changed, &dump()), Ok(Vec::new()), "{name}");
    }
}

#[test]
fn a_node_that_draws_less_opaque_and_one_that_no_longer_draws_are_each_said() {
    let nodes = |nodes: &[&str]| with(dump(), "drawn_nodes", json!(nodes));
    let faded = nodes(&["00000001:0:1.000", "00000002:3:0.800"]);
    let found = differences(&faded, &dump()).expect("two dumps");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("00000002:3:0.800") && found[0].contains("00000002:3:0.400"),
        "the node is said as each holds it: {found:?}"
    );
    let gone = nodes(&["00000001:0:1.000"]);
    let found = differences(&gone, &dump()).expect("two dumps");
    assert!(
        found[0].contains("0 entries the reference lacks")
            && found[0].contains("1 it holds alone")
            && found[0].contains("00000002:3:0.400"),
        "{found:?}"
    );
}

#[test]
fn a_field_no_list_names_and_a_field_that_is_gone_each_fail() {
    let more = with(dump(), "lights", json!(3));
    let found = differences(&more, &more).expect("two dumps");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("lights") && found[0].contains("neither"));

    let mut less = dump();
    (less.as_object_mut().expect("an object")).remove("drawn_nodes");
    let found = differences(&less, &dump()).expect("two dumps");
    assert_eq!(found, ["drawn_nodes: the dump does not hold it"]);
    let found = differences(&dump(), &less).expect("two dumps");
    assert_eq!(found, ["drawn_nodes: the reference does not hold it"]);
    assert!(differences(&json!([1]), &dump()).is_err(), "no object");
}

#[test]
fn a_dump_with_no_reference_fails_and_so_does_one_that_was_not_written() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let reference = folder.path().join("reference.json");
    let step = CompareDump {
        dump: "city.json".into(),
        reference: reference.clone(),
    };
    let none = run(&step, folder.path()).expect_err("no dump yet");
    assert!(none.contains("city.json"), "{none}");

    let write = |path: &Path, dump: &Value| {
        std::fs::write(path, dump.to_string()).expect("a dump is written");
    };
    write(&folder.path().join("city.json"), &dump());
    let missing = run(&step, folder.path()).expect_err("no reference");
    assert!(missing.contains("no reference"), "{missing}");
    assert!(!reference.exists(), "the step never writes a reference");

    write(&reference, &with(dump(), "selection_ms", json!(99.0)));
    assert_eq!(run(&step, folder.path()), Ok(()), "a timing fails nothing");
    write(&reference, &with(dump(), "streaming.drawn", json!(3)));
    let differs = run(&step, folder.path()).expect_err("another count of nodes drawn");
    assert!(
        differs.contains("city.json") && differs.contains("streaming.drawn"),
        "{differs}"
    );
    std::fs::write(&reference, "not json").expect("written");
    assert!(
        run(&step, folder.path()).is_err(),
        "a reference that does not read"
    );
}
