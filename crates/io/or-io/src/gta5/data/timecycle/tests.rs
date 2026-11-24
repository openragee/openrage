use super::*;

const TIME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<time_data version="1.000000" mode="dynamic">
  <suninfo sun_roll="122" sun_yaw="0.0"/>
  <mooninfo moon_roll="-122" moon_wobble_freq="2" moon_wobble_amp="0.2" moon_wobble_offset="0.375"/>
  <sample name="00:00" hour="0" duration="4" uw_tc_mod="underwater"/>
  <sample name="06:00" hour="6" duration="0" uw_tc_mod="underwater"/>
  <sample name="12:00" hour="12" duration="0" uw_tc_mod="underwater"/>
  <sample name="22:00" hour="22" duration="0" uw_tc_mod="underwater"/>
  <region name="URBAN"/>
</time_data>"#;

fn cycle(name: &str, global: [f32; 4], urban: [f32; 4]) -> String {
    let row = |v: [f32; 4]| format!("{} {} {} {}", v[0], v[1], v[2], v[3]);
    format!(
        r#"<timecycle_keyframe_data version="1.000000">
  <cycle name="{name}" regions="2">
    <region name="GLOBAL">
      <light_dir_mult> {} </light_dir_mult>
      <fog_start>
        100.0 100.0 100.0 100.0
      </fog_start>
    </region>
    <region name="URBAN">
      <light_dir_mult> {} </light_dir_mult>
    </region>
  </cycle>
</timecycle_keyframe_data>"#,
        row(global),
        row(urban)
    )
}

fn cycles() -> Timecycle {
    let mut timecycle = Timecycle::new(TIME);
    timecycle.add_cycles(&cycle(
        "CLEAR",
        [0.0, 10.0, 20.0, 30.0],
        [1.0, 1.0, 1.0, 1.0],
    ));
    timecycle.add_cycles(&cycle("RAIN", [0.0, 0.0, 0.0, 0.0], [2.0, 2.0, 2.0, 2.0]));
    timecycle
}

#[test]
fn the_table_reads_its_samples_and_the_angles_of_the_sun_and_moon_in_radians() {
    let table = TimeTable::parse(TIME);
    assert_eq!(
        table.samples,
        [
            TimeSample {
                hour: 0.0,
                duration: 4.0,
                easing: Easing::Linear,
            },
            TimeSample {
                hour: 6.0,
                duration: 0.0,
                easing: Easing::Linear,
            },
            TimeSample {
                hour: 12.0,
                duration: 0.0,
                easing: Easing::Linear,
            },
            TimeSample {
                hour: 22.0,
                duration: 0.0,
                easing: Easing::Linear,
            },
        ]
    );
    assert!((table.sun_roll - 122f32.to_radians()).abs() < 1e-6);
    assert!((table.moon_roll + 122f32.to_radians()).abs() < 1e-6);
    assert_eq!(table.moon_wobble_frequency, 2.0);
    assert_eq!(table.moon_wobble_offset, 0.375);
}

#[test]
fn an_hour_lies_between_two_samples_and_a_sample_holds_for_its_duration_first() {
    let table = TimeTable::parse(TIME);
    // Exactly on a sample
    assert_eq!(table.locate(12.0), Some((2, 3, 0.0)));
    // Halfway from 12 to 22
    let (a, b, t) = table.locate(17.0).unwrap();
    assert_eq!((a, b), (2, 3));
    assert!((t - 0.5).abs() < 1e-6);
    // The first sample holds for four hours, then blends to the one at six
    let (a, b, t) = table.locate(3.0).unwrap();
    assert_eq!((a, b, t), (0, 1, 0.0));
    let (_, _, t) = table.locate(5.0).unwrap();
    assert!((t - 0.5).abs() < 1e-6);
    // After the last sample the day wraps to the first, a day later
    let (a, b, t) = table.locate(23.0).unwrap();
    assert_eq!((a, b), (3, 0));
    assert!((t - 0.5).abs() < 1e-6, "{t}");
    // Before the first: the last sample of the day before, blending in
    let (a, b, _) = table.locate(0.0).unwrap();
    assert_eq!((a, b), (0, 1));
    assert_eq!(TimeTable::default().locate(5.0), None);
}

#[test]
fn a_keyframe_blends_the_samples_of_the_hour_and_the_weather() {
    let timecycle = cycles();
    // 9 o'clock is halfway from the 6 sample to the 12 one, and CLEAR is all of the weather
    let frame = timecycle.keyframe(9.0, "CLEAR", "CLEAR", 0.0, 0.0).unwrap();
    assert!((frame.get("light_dir_mult").unwrap() - 15.0).abs() < 1e-5);
    assert_eq!(frame.get("fog_start"), Some(100.0));
    // Half of the way from clear to rain is half of the value
    let mixed = timecycle.keyframe(9.0, "clear", "rain", 0.5, 0.0).unwrap();
    assert!((mixed.get("light_dir_mult").unwrap() - 7.5).abs() < 1e-5);
}

#[test]
fn the_urban_region_is_blended_in_by_its_strength_and_replaces_the_first_at_full_strength() {
    let timecycle = cycles();
    let global = timecycle
        .keyframe(12.0, "CLEAR", "CLEAR", 0.0, 0.0)
        .unwrap();
    let half = timecycle
        .keyframe(12.0, "CLEAR", "CLEAR", 0.0, 0.5)
        .unwrap();
    let urban = timecycle
        .keyframe(12.0, "CLEAR", "CLEAR", 0.0, 1.0)
        .unwrap();
    assert_eq!(global.get("light_dir_mult"), Some(20.0));
    assert_eq!(half.get("light_dir_mult"), Some(10.5));
    assert_eq!(urban.get("light_dir_mult"), Some(1.0));
    // A variable only the first region states stays as it is
    assert_eq!(half.get("fog_start"), Some(100.0));
}

#[test]
fn a_cycle_that_is_not_read_gives_no_keyframe() {
    let timecycle = cycles();
    assert!(timecycle
        .keyframe(12.0, "CLEAR", "SMOG", 0.5, 0.0)
        .is_none());
    assert!(Timecycle::new("")
        .keyframe(12.0, "A", "A", 0.0, 0.0)
        .is_none());
}

#[test]
fn a_bent_blend_starts_and_ends_where_a_straight_one_does_and_lags_it_early() {
    for easing in [Easing::Linear, Easing::QuarticInOut] {
        assert_eq!(easing.ease(0.0), 0.0);
        assert_eq!(easing.ease(1.0), 1.0);
        assert!((easing.ease(0.5) - 0.5).abs() < 1e-6);
    }
    assert_eq!(Easing::Linear.ease(0.25), 0.25);
    assert!((Easing::QuarticInOut.ease(0.25) - 0.03125).abs() < 1e-6);
    assert!((Easing::QuarticInOut.ease(0.75) - 0.96875).abs() < 1e-6);
    // A table that names no bend, as GTA V's, goes straight.
    let table = TimeTable::parse(TIME);
    assert!(table.samples.iter().all(|s| s.easing == Easing::Linear));
}

#[test]
fn a_value_alone_before_a_cycle_is_the_weather_s_and_none_of_its_samples() {
    let mut timecycle = Timecycle::new(TIME);
    timecycle.add_cycles(
        r#"<timecycle_keyframe_data><summary><cover> 0.5</cover><pair> 1 2</pair></summary>
        <cycle name="CLEAR" regions="1"><region name="GLOBAL"><fog_start> 1 2 3 4</fog_start></region></cycle>
        </timecycle_keyframe_data>"#,
    );
    let clear = timecycle.cycle("clear").expect("read");
    assert_eq!(clear.overall.get("cover"), Some(&0.5));
    assert_eq!(
        clear.overall.len(),
        1,
        "a row of several is no single value"
    );
    assert_eq!(clear.regions[0].vars.len(), 1);
    // GTA V's cycles state none.
    assert!(cycles().cycles.iter().all(|cycle| cycle.overall.is_empty()));
}
