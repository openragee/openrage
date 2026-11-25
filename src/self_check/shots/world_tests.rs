use super::*;

const IDLE: Found = Found {
    indexed: true,
    waiting: [0; 4],
    standing: [40, 40, 900, 700],
    adapting: false,
    water: false,
    selecting: false,
    fading: false,
    device: 0,
    caught_up: true,
};

/// A time into a wait: the rule counts frames, so a time only orders them.
const TICK: Duration = Duration::from_millis(1500);

fn at(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

#[test]
fn the_steps_of_the_world_are_read() {
    assert_eq!(
        parse("camera -120.5 340 62.25 135 -20 60"),
        Ok(WorldStep::Camera(Pose {
            position: glam::Vec3::new(-120.5, 340.0, 62.25),
            yaw: 135.0,
            pitch: -20.0,
            fov: Some(60.0),
        }))
    );
    assert_eq!(parse("hour 17.5"), Ok(WorldStep::Hour(17.5)));
    assert_eq!(parse("hour 0"), Ok(WorldStep::Hour(0.0)));
    assert_eq!(parse("clean on"), Ok(WorldStep::Clean(true)));
    assert_eq!(parse("clean off"), Ok(WorldStep::Clean(false)));
    assert_eq!(parse("settle"), Ok(WorldStep::Settle(SETTLE_DEADLINE)));
    assert_eq!(parse("settle 2.5"), Ok(WorldStep::Settle(at(2500))));
    assert_eq!(
        parse("converge"),
        Ok(WorldStep::Converge(CONVERGE_DEADLINE))
    );
    assert_eq!(parse("converge 30"), Ok(WorldStep::Converge(at(30_000))));
    assert_eq!(
        parse("dump truth-01.json"),
        Ok(WorldStep::Dump("truth-01.json".into()))
    );
    assert_eq!(parse("modes base story"), Ok(WorldStep::Modes(3)));
    assert_eq!(parse("modes minigames"), Ok(WorldStep::Modes(4)));
    assert_eq!(
        parse("fly 30 0 -1.5 2"),
        Ok(WorldStep::Fly(glam::Vec3::new(30.0, 0.0, -1.5), at(2000)))
    );
    assert_eq!(
        parse("perf cost-01.json 30"),
        Ok(WorldStep::Perf("cost-01.json".into(), 30))
    );
    assert_eq!(
        parse("move 1 walk.json 50 0 10 0 -5 60"),
        Ok(WorldStep::Move(
            Pose {
                position: glam::Vec3::new(50.0, 0.0, 10.0),
                yaw: 0.0,
                pitch: -5.0,
                fov: Some(60.0),
            },
            at(1000),
            "walk.json".into()
        ))
    );
    assert_eq!(
        parse("group 0D93912D on"),
        Ok(WorldStep::Group(0x0D93_912D, true))
    );
    assert_eq!(parse("group 0x1 off"), Ok(WorldStep::Group(1, false)));
    assert_eq!(
        parse("weather cloudy"),
        Ok(WorldStep::Weather(WeatherState::Cloudy))
    );
}

#[test]
fn a_census_is_written_by_kind_and_level_and_names_as_sorted_hexadecimal() {
    let mut counts = [[0; 7]; 4];
    (counts[0][0], counts[2][5], counts[3][2]) = (12, 3, 1);
    let written = by_level(&counts);
    assert_eq!(written.len(), 3, "a count of none is left out: {written:?}");
    assert_eq!(written["entity HD"], 12);
    assert_eq!(written["grass list ORPHANHD"], 3);
    assert_eq!(written["far grass list SLOD1"], 1);
    assert_eq!(
        hex(&[0xABC, 0x1, 0xFFFF_FFFF]),
        ["00000001", "00000ABC", "FFFFFFFF"]
    );
}

#[test]
fn a_wrong_world_step_is_refused() {
    for bad in [
        "",
        "fly",
        "camera",
        "camera 1 2 3",
        "camera 1 2 3 0 95",
        "hour",
        "hour 24",
        "hour -1",
        "hour noon",
        "clean",
        "clean yes",
        "settle 0",
        "settle -3",
        "settle soon",
        "settle 1 2",
        "settle 1e30",
        "converge 0",
        "converge soon",
        "converge 1 2",
        "dump",
        "dump ../out.json",
        "dump C:/out.json",
        "dump two words",
        "modes",
        "modes base online",
        "weather",
        "weather sunny",
        "group",
        "group 0D93912D",
        "group zz on",
        "group 1 maybe",
        "fly 1 2 3",
        "fly 1 2 3 0",
        "fly 1 2 3 soon",
        "perf",
        "perf a.json",
        "perf a.json 0",
        "perf ../a.json 3",
        "move",
        "move 1 a.json",
        "move 0 a.json 1 2 3 0 0",
        "move 1 ../a.json 1 2 3 0 0",
    ] {
        assert!(parse(bad).is_err(), "`world {bad}` should be refused");
    }
}

#[test]
fn the_world_settles_once_nothing_is_waited_for_and_nothing_has_changed_for_the_quiet_frames() {
    let mut rule = Settle::default();
    assert!(!rule.settled(at(0), IDLE), "one frame is no quiet time");
    assert!(!rule.settled(at(1), IDLE));
    assert!(
        rule.settled(at(2), IDLE),
        "settled on the frame the quiet frames end, however little time they took"
    );
}

#[test]
fn a_far_node_still_fading_in_never_settles() {
    // Nothing is waited for and the counts stand still, yet each selection draws the far nodes a step more opaque.
    let fading = Found {
        fading: true,
        ..IDLE
    };
    let mut rule = Settle::default();
    for frame in 0..10 {
        assert!(!rule.settled(TICK * frame, fading));
    }
    assert_eq!(rule.still(), "far nodes were still fading in");
    assert!(!rule.settled(TICK * 10, IDLE));
    assert!(!rule.settled(TICK * 11, IDLE));
    assert!(rule.settled(TICK * 12, IDLE));
}

#[test]
fn something_put_on_the_device_starts_the_quiet_frames_again() {
    // The counts stand still, yet a texture came and a far stand-in bakes from it.
    let mut rule = Settle::default();
    assert!(!rule.settled(at(0), IDLE));
    assert!(!rule.settled(at(1), IDLE));
    let came = Found { device: 1, ..IDLE };
    assert!(!rule.settled(at(2), came));
    assert!(!rule.settled(at(3), came));
    assert!(rule.settled(at(4), came));
}

#[test]
fn an_exposure_still_adapting_never_settles() {
    let adapting = Found {
        adapting: true,
        ..IDLE
    };
    let mut rule = Settle::default();
    for frame in 0..10 {
        assert!(!rule.settled(TICK * frame, adapting));
    }
    assert_eq!(rule.still(), "the exposure was still adapting");
    assert!(!rule.settled(TICK * 10, IDLE));
    assert!(!rule.settled(TICK * 11, IDLE));
    assert!(rule.settled(TICK * 12, IDLE));
}

#[test]
fn a_selection_made_for_the_last_place_of_the_camera_never_settles() {
    // Nothing is waited for yet: the world's thread has not chosen for the new place.
    let behind = Found {
        caught_up: false,
        ..IDLE
    };
    let mut rule = Settle::default();
    assert!(!rule.settled(at(0), behind));
    assert!(!rule.settled(TICK * 10, behind));
    assert!(!rule.settled(TICK * 10 + at(1), IDLE));
    assert!(!rule.settled(TICK * 11 + at(1), IDLE));
    assert!(rule.settled(TICK * 12 + at(1), IDLE));
}

#[test]
fn something_waited_for_or_changed_starts_the_quiet_time_again() {
    let waiting = Found {
        waiting: [0, 3, 0, 0],
        ..IDLE
    };
    let grown = Found {
        standing: [40, 40, 900, 701],
        ..IDLE
    };
    let mut rule = Settle::default();
    assert!(!rule.settled(at(0), IDLE));
    // A model asked for after a quiet stretch: the camera jumped and the selection caught up.
    assert!(!rule.settled(TICK - at(1), waiting));
    assert!(
        !rule.settled(TICK * 3, waiting),
        "never while something is waited for"
    );
    assert!(!rule.settled(TICK * 3 + at(1), IDLE));
    assert_eq!(
        rule.last_waits(),
        "maps 0.0 s, models 4.5 s, textures 0.0 s, lights 0.0 s, exposure 0.0 s"
    );
    assert!(
        !rule.settled(TICK * 4, IDLE),
        "the quiet frames run from the last change, not from the first frame"
    );
    // Nothing is waited for, yet what stands streamed in moved: a parent gave way to its children.
    assert!(!rule.settled(TICK * 4 + at(1), grown));
    assert!(!rule.settled(TICK * 5, grown));
    assert!(rule.settled(TICK * 5 + at(1), grown));
}

#[test]
fn a_world_with_no_index_never_settles_and_the_wait_says_what_was_still_arriving() {
    let unindexed = Found {
        indexed: false,
        ..Found::default()
    };
    let mut rule = Settle::default();
    assert_eq!(rule.still(), "no frame was drawn");
    assert!(!rule.settled(at(0), unindexed));
    assert!(
        !rule.settled(TICK * 10, unindexed),
        "no index is nothing to shoot"
    );
    assert_eq!(rule.still(), "no index of the install is held");
    let waiting = Found {
        waiting: [2, 0, 7, 0],
        ..IDLE
    };
    assert!(!rule.settled(TICK * 11, waiting));
    assert_eq!(
        rule.still(),
        "2 maps, 0 models, 7 textures and 0 maps' lights were still waited for"
    );
    let water = Found {
        water: true,
        ..IDLE
    };
    assert!(!rule.settled(TICK * 12, water));
    assert!(
        !rule.settled(TICK * 20, water),
        "never while the water is read"
    );
    assert_eq!(
        rule.still(),
        "the water's ground or fog was still being read"
    );
    let selecting = Found {
        selecting: true,
        ..IDLE
    };
    assert!(!rule.settled(TICK * 21, selecting));
    assert!(
        !rule.settled(TICK * 30, selecting),
        "never while the world's thread has not selected on what it was told"
    );
    assert_eq!(
        rule.still(),
        "the world's thread had not yet selected on what it was told"
    );
    assert!(!rule.settled(TICK * 31, IDLE));
    assert_eq!(rule.still(), "what is streamed in was still changing");
}
