use super::*;

fn time() -> String {
    let samples: String = (0..13)
        .map(|k| format!(r#"<sample name="s{k}" hour="{k}" interpolation_type="EASE_LINEAR_IN"/>"#))
        .collect();
    format!(r#"<?xml version="1.0"?><time_data mode="dynamic">{samples}</time_data>"#)
}

/// A cycle whose sun fades with the sample: sample 6 is the top of the round, 4 the rise, 0 the middle of the night.
fn cycle(name: &str, mult: f32) -> String {
    let all = |value: f32| vec![value.to_string(); 13].join(" ");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<timecycle_keyframe_data version="2.000000">
  <High_Level_Vars><cloud_cover> 0.25</cloud_cover></High_Level_Vars>
  <cycle name="{name}" regions="2">
    <region name="GLOBAL">
      <light_dir_mult> {mult} </light_dir_mult>
      <sky_scattering_sun_intensity_multiplier> 0 0 0 0 1 1 1 1 1 1 0 0 0</sky_scattering_sun_intensity_multiplier>
      <sky_scattering_sun_illuminance> {sun} </sky_scattering_sun_illuminance>
      <sky_scattering_moon_illuminance> {moon} </sky_scattering_moon_illuminance>
      <postfx_exposure_min> {least} </postfx_exposure_min>
      <postfx_exposure_max> {most} </postfx_exposure_max>
      <far_clip> 100 100 100 100 100 100 300 100 100 100 100 100 100</far_clip>
    </region>
    <region name="URBAN"><light_dir_mult> 9 9 9 9 9 9 9 9 9 9 9 9 9</light_dir_mult></region>
  </cycle>
</timecycle_keyframe_data>"#,
        mult = all(mult),
        sun = all(100_000.0),
        moon = all(0.5),
        least = all(-12.0),
        most = all(16.0),
    )
}

const WEATHER: &str = r#"<CContentsOfWeatherXmlFile>
  <WeatherTypeDatas>
    <Item><Name>WET</Name><Rain value="1.000000" /><TimeCycleName>Rainy</TimeCycleName></Item>
    <Item><Name>CLEAR</Name><TimeCycleName>ClearSky</TimeCycleName></Item>
  </WeatherTypeDatas>
  <WeatherTypes>
    <Item><Name>AM_Wet</Name><DataName>WET</DataName><ModAtmosphere>m</ModAtmosphere></Item>
    <Item><Name>AM_Sunny</Name><DataName>CLEAR</DataName></Item>
  </WeatherTypes>
  <WeatherTypeIndirectionList><Item><From>SUNNY</From><To>AM_Sunny</To></Item></WeatherTypeIndirectionList>
</CContentsOfWeatherXmlFile>"#;

const SETTINGS: &str = "# Visual Configuration Table\nmisc.MoonDimMult\t0.3\nTonemapping.dark.filmic.W 11.2\nsky.latitude.YPosMin\t-100.0\nsky.latitude.YPosMax\t100.0\nsky.latitude.rangeMin\t20.0\nsky.latitude.rangeMax\t30.0\nmisc.Moon.RollOffset 90\nmisc.Sun.Roll 270\nExposure.curve.scale -1.0\nExposure.curve.verticalOffset 6.0\nAdaptation.step.size.mult 1.0\n";

fn tables() -> DayTables {
    let (time, dry, wet) = (time(), cycle("Sunny", 1.0), cycle("Rainy", 0.5));
    let clear = cycle("ClearSky", 0.25);
    let mods = r#"<timecycle_modifier_data><modifier name="m" userFlags="0"><far_clip>1 1</far_clip><fog_start>2 2</fog_start></modifier><modifier name="s" userFlags="0"><far_clip>3 3</far_clip></modifier></timecycle_modifier_data>"#;
    DayTables::of_texts([
        wet.as_str(),
        "<some_other_table/>",
        SETTINGS,
        time.as_str(),
        "plain text of another kind",
        WEATHER,
        dry.as_str(),
        clear.as_str(),
        mods,
    ])
}

#[test]
fn nameless_texts_are_told_apart_by_what_they_open_with_in_any_order() {
    let tables = tables();
    assert!(!tables.is_empty());
    assert_eq!(tables.table.samples.len(), 13);
    assert_eq!(tables.weathers(), ["Rainy", "Sunny", "ClearSky"]);
    assert_eq!(tables.high_level["sunny"]["cloud_cover"], 0.25);
    assert_eq!(tables.weather.datas.len(), 2);
    assert_eq!(tables.modifiers.len(), 2);
    assert_eq!(tables.visual_settings.get_f32("misc.MoonDimMult", 1.0), 0.3);
    assert!(DayTables::of_texts(["<other/>", "words"]).is_empty());
}

#[test]
fn the_latitude_is_the_settings_range_at_the_cameras_place_and_the_maps_middle_without_one() {
    let tables = tables();
    assert!((tables.latitude(0.0) - 25f32.to_radians()).abs() < 1e-6);
    assert!((tables.latitude(-500.0) - 20f32.to_radians()).abs() < 1e-6);
    assert!((tables.latitude(100.0) - 30f32.to_radians()).abs() < 1e-6);
    assert_eq!(DayTables::default().latitude(0.0), LATITUDE);
}

#[test]
fn a_moment_blends_the_two_samples_the_sun_stands_between_and_exposes_its_light() {
    let tables = tables();
    // Solar noon by the summer clock, with no noon table to move it: the top of the round, the seventh sample almost whole.
    let noon = tables
        .moment(13.0, "Rainy", 0.0)
        .expect("a cycle of that name");
    assert!(
        (noon.frame_hours - 13.0).abs() < 0.05,
        "{}",
        noon.frame_hours
    );
    assert!((noon.own.get("far_clip").expect("stated") - 300.0).abs() < 2.0);
    assert_eq!(noon.keyframe.get("far_clip"), noon.own.get("far_clip"));
    assert!(noon.sun[2] > 0.9 && noon.sun[1] < 0.0, "{:?}", noon.sun);
    assert!(
        (noon.table.sun_roll - (std::f32::consts::FRAC_PI_2 + 25f32.to_radians())).abs() < 1e-6
    );
    assert!((noon.table.moon_roll - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert_eq!((noon.moon_dim, noon.wetness), (0.3, 0.0));
    // The frame's own exposure is held at nought: the light it takes is exposed already.
    assert_eq!(noon.keyframe.get("postfx_exposure_max"), Some(0.0));
    assert_eq!(
        noon.light_scale,
        noon.stops.exp2() * or_io::gta6::timecycle::FRAME_DIFFUSE
    );

    // The middle of the night: the moon lights, and the frame is exposed far brighter.
    let night = tables
        .moment(1.45, "Rainy", 0.0)
        .expect("a cycle of that name");
    assert!(night.sun[2] < -0.5, "{:?}", night.sun);
    assert!(night.moon[2] > 0.5, "{:?}", night.moon);
    assert!(
        !(1.0..23.0).contains(&night.frame_hours),
        "{}",
        night.frame_hours
    );
    assert!(
        noon.stops < night.stops - 5.0,
        "{} {}",
        noon.stops,
        night.stops
    );
    let moon = night.keyframe.get("light_dir_mult").expect("worked out");
    let sun = noon.keyframe.get("light_dir_mult").expect("worked out");
    assert!(moon > 0.0 && moon < sun, "{moon} {sun}");
}

#[test]
fn a_weather_is_its_types_cycle_with_its_modifiers_and_a_name_that_is_a_cycle_too_is_not_that_cycle(
) {
    let tables = tables();
    let mult = |weather: &str| {
        let moment = tables.moment(13.45, weather, 0.0).expect(weather);
        moment.own.get("light_dir_mult").expect("stated")
    };
    // The default weather goes through its indirection to its data's cycle, not to the cycle of its name.
    assert_eq!(mult("Sunny"), 0.25);
    assert_eq!(mult("AM_Wet"), 0.5);
    assert_eq!(mult("rainy"), 0.5);
    // The type's modifier is laid over its cycle; the cycle alone has none.
    let far = |weather: &str| {
        tables
            .moment(13.45, weather, 0.0)
            .and_then(|m| m.own.get("far_clip"))
    };
    assert_eq!(far("AM_Wet"), Some(1.0));
    assert!(far("rainy").is_some_and(|f| f > 200.0));
    assert_eq!(
        tables.moment(13.45, "AM_Wet", 0.0).map(|m| m.wetness),
        Some(1.0)
    );
    assert_eq!(
        tables.moment(13.45, "rainy", 0.0).map(|m| m.wetness),
        Some(0.0)
    );
    assert!(tables.moment(13.45, "Snow", 0.0).is_none());
    assert!(DayTables::default().moment(13.45, "Sunny", 0.0).is_none());
}

#[test]
fn half_way_between_two_weathers_their_cycles_and_rain_are_half_of_each() {
    let tables = tables();
    let half = tables
        .moment_between(13.45, "AM_Wet", "Sunny", 0.5, 0.0)
        .expect("both are read");
    let mult = half.own.get("light_dir_mult").expect("stated");
    assert!((mult - 0.375).abs() < 1e-6, "{mult}");
    assert_eq!(half.wetness, 0.5);
    // The modifier only one of them names is laid whole.
    assert_eq!(half.own.get("far_clip"), Some(1.0));
}

#[test]
fn a_queued_weather_is_laid_over_the_base_by_its_weight_with_its_modifiers() {
    let tables = tables();
    let at = |weight: f32| {
        let weathers = WeatherBlend {
            queued: Some(("AM_Wet", weight)),
            ..WeatherBlend::steady("Sunny")
        };
        (tables.moment_in(13.45, weathers, 0.0, None)).expect("both are read")
    };
    let none = at(0.0);
    assert_eq!(
        none.own,
        tables.moment(13.45, "Sunny", 0.0).expect("read").own
    );
    let half = at(0.5);
    let mult = half.own.get("light_dir_mult").expect("stated");
    assert!((mult - 0.375).abs() < 1e-6, "{mult}");
    assert_eq!(half.wetness, 0.5);
    let whole = at(1.0);
    assert_eq!(whole.own.get("light_dir_mult"), Some(0.5));
    assert_eq!(whole.own.get("far_clip"), Some(1.0));
    assert_eq!(whole.wetness, 1.0);
}

#[test]
fn a_metered_frame_is_exposed_by_the_curve_and_adapted_towards_it_at_the_settings_pace() {
    let tables = tables();
    let night = tables.moment(1.45, "Rainy", 0.0).expect("a cycle");
    let settings = tables.exposure_settings();
    let previous = night.stops;
    // A frame drawn at the night's stops that meters a thousand cd/m² of scene.
    let metered = 1000.0 * previous.exp2();
    let at_once = night.metered(&settings, metered, previous, 0.0);
    let want = -(1000f32.ln()) + 6.0 - 10.0;
    assert!(
        (at_once.stops - want).abs() < 1e-3,
        "{} {want}",
        at_once.stops
    );
    let strength = |m: &Moment| m.keyframe.get("light_dir_mult").expect("worked out");
    let ratio = strength(&at_once) / strength(&night);
    assert!((ratio - (at_once.stops - night.stops).exp2()).abs() < 1e-3 * ratio);
    assert_eq!(at_once.own, night.own);
    // A tenth of a second goes a tenth of the way, the rate being within the settings' steps.
    let step = night.metered(&settings, metered, previous, 0.1);
    let tenth = previous + (want - previous) * 0.1;
    assert!((step.stops - tenth).abs() < 1e-2, "{} {tenth}", step.stops);
}

#[test]
fn a_room_lays_its_modifier_over_the_day_by_its_blend_and_one_not_read_changes_nothing() {
    let tables = tables();
    let day = tables.moment(12.0, "Sunny", 0.0).expect("a moment");
    let clip = |m: &Moment| m.own.get("far_clip").expect("a far clip");
    // The room names the modifier `m` by its hash, whatever the case.
    let room = or_io::hash::string_hash(b"M");
    let whole = tables
        .moment_inside(12.0, "Sunny", 0.0, [room, 0], 1.0)
        .expect("a moment");
    let half = tables
        .moment_inside(12.0, "Sunny", 0.0, [room, 0], 0.5)
        .expect("a moment");
    assert_eq!(clip(&whole), 1.0);
    assert!(
        (clip(&half) - (clip(&day) + 1.0) / 2.0).abs() < 1e-4,
        "{}",
        clip(&half)
    );
    let unread = tables
        .moment_inside(12.0, "Sunny", 0.0, [room ^ 1, 0], 1.0)
        .expect("a moment");
    assert_eq!(unread, day);
    // A second modifier stands in for the first where it states a variable, and leaves the rest.
    let second = or_io::hash::string_hash(b"s");
    let both = tables
        .moment_inside(12.0, "Sunny", 0.0, [room, second], 1.0)
        .expect("a moment");
    assert_eq!(clip(&both), 3.0);
    assert_eq!(both.own.get("fog_start"), Some(2.0));
    let alone = tables
        .moment_inside(12.0, "Sunny", 0.0, [room ^ 1, second], 1.0)
        .expect("a moment");
    assert_eq!(alone, day);
}
