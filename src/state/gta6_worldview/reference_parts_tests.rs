//! The parts of the reference of the GTA VI world, on data written by hand: what differs between
//! two sets of digests, a reference written and read back, what the variables ask, and the script's
//! shots.

use super::*;

/// A world that reads the same holds no difference, and one digest altered, one gone and one more each fail by their key.
#[test]
fn an_altered_digest_a_missing_one_and_one_too_many_each_fail() {
    let world: Digests = [
        ("index.maps", "aa rows=2"),
        ("model.01", "bb"),
        ("shot.a.nodes", "cc n=3"),
    ]
    .map(|(key, digest)| (key.to_string(), digest.to_string()))
    .into();
    assert_eq!(differences(&world, &world), Vec::<String>::new());

    let mut altered = world.clone();
    altered.insert("model.01".into(), "bd".into());
    assert_eq!(
        differences(&world, &altered),
        ["model.01: bb where the reference holds bd"]
    );
    let mut lacking = world.clone();
    lacking.remove("shot.a.nodes");
    assert_eq!(
        differences(&world, &lacking),
        ["shot.a.nodes: cc n=3, which the reference lacks"]
    );
    assert_eq!(
        differences(&lacking, &world),
        ["shot.a.nodes: nothing where the reference holds cc n=3"]
    );
}

#[test]
fn a_reference_reads_back_as_written_is_never_written_over_and_fails_where_it_is_not_there() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("reference.txt");
    let world: Digests = [("index.maps", "aa rows=2"), ("model.01", "bb")]
        .map(|(key, digest)| (key.to_string(), digest.to_string()))
        .into();
    let missing = read(&file).expect_err("no reference yet");
    assert!(missing.contains("no reference at"), "{missing}");

    assert_eq!(write(&file, &world), Ok(()));
    assert_eq!(read(&file), Ok(world.clone()));
    let again = write(&file, &world).expect_err("a reference is there");
    assert!(again.contains("not written over"), "{again}");

    for (text, says) in [
        ("", "no reference"),
        ("index.maps\taa\n", "no reference"),
        (&format!("{HEAD}\n"), "holds no digest"),
        (&format!("{HEAD}\nindex.maps aa\n"), "no key and digest"),
    ] {
        std::fs::write(&file, text).expect("written");
        let wrong = read(&file).expect_err("no reference");
        assert!(wrong.contains(says), "`{text}`: {wrong}");
    }
}

#[test]
fn the_suite_writes_or_holds_and_says_which_variable_it_lacks() {
    let set = |names: &'static [&str]| {
        move |name: &str| names.contains(&name).then(|| PathBuf::from(name))
    };
    let Ok(Asked::Nothing(why)) = asked(&set(&[])) else {
        panic!("no build is named");
    };
    assert!(why.contains(BUILD_ENV), "{why}");
    let Ok(Asked::Nothing(why)) = asked(&set(&[BUILD_ENV])) else {
        panic!("neither file is named");
    };
    assert!(why.contains(WRITE_ENV) && why.contains(HELD_ENV), "{why}");
    assert_eq!(
        asked(&set(&[BUILD_ENV, WRITE_ENV])),
        Ok(Asked::Write(BUILD_ENV.into(), WRITE_ENV.into()))
    );
    assert_eq!(
        asked(&set(&[BUILD_ENV, HELD_ENV])),
        Ok(Asked::Hold(BUILD_ENV.into(), HELD_ENV.into()))
    );
    assert!(asked(&set(&[BUILD_ENV, WRITE_ENV, HELD_ENV])).is_err());
}

#[test]
fn records_digest_alike_in_any_order_and_apart_when_one_changes() {
    let records = |texts: &[&str]| texts.iter().map(|text| text.as_bytes().to_vec()).collect();
    let one = of_records(records(&["a", "bc", "d"]));
    assert_eq!(one, of_records(records(&["d", "a", "bc"])));
    assert!(one.ends_with(" n=3"), "{one}");
    assert_ne!(one, of_records(records(&["a", "bd", "d"])));
    // ⚠ Each record's length is digested with it: `a`, `bc` is not `ab`, `c`.
    assert_ne!(
        of_records(records(&["a", "bc"])),
        of_records(records(&["ab", "c"]))
    );
    assert_eq!(
        evenly(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], 5),
        [&0, &2, &4, &6, &8]
    );
    assert_eq!(evenly(&[0, 1, 2], 5), [&0, &1, &2]);
    assert!(evenly::<u8>(&[], 5).is_empty());
}

#[test]
fn the_reference_script_gives_the_suite_its_cameras_hours_modes_and_weathers() {
    let read = stops(
        "world hour 9\nworld camera 1 2 3 45 -10 60\nshot a\nworld modes base story\n\
         world weather rain\nworld hour 21.5\nshot b\ncompare a ref.png 1 1",
    )
    .expect("a script");
    let (base, story) = (Mode::Base.bit(), Mode::Story.bit());
    let seen: Vec<_> = (read.iter())
        .map(|stop| (stop.name.as_str(), stop.hour, stop.modes, stop.weather))
        .collect();
    assert_eq!(
        seen,
        [
            ("a", 9.0, base, WeatherState::Clear),
            ("b", 21.5, base | story, WeatherState::Rain)
        ]
    );
    assert_eq!(read[0].pose.position, Vec3::new(1.0, 2.0, 3.0));
    assert!(stops("shot a").is_err(), "a shot before any camera");
    assert!(stops("world modes all").is_err());
    assert!(stops("world weather snow").is_err());

    let script = stops(SCRIPT).expect("the script in the tree");
    let names: BTreeSet<&str> = script.iter().map(|stop| stop.name.as_str()).collect();
    assert_eq!(names.len(), script.len(), "no two shots share a name");
    let hours: BTreeSet<u32> = script.iter().map(|stop| stop.hour as u32).collect();
    assert!(hours.len() >= 3, "noon, dusk and night: {hours:?}");
}
