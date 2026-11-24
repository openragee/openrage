use super::*;

/// Thirteen samples, the last bent, and one cycle whose rows are `rows` beside a sun of a
/// thousand lux and a moon of two.
fn data(cycle: &str, rows: &str) -> TimecycleData {
    let samples: String = (0..13)
        .map(|hour| match hour {
            12 => format!(r#"<sample hour="{hour}" interpolation_type="EASE_QUARTIC_INOUT"/>"#),
            _ => format!(r#"<sample hour="{hour}"/>"#),
        })
        .collect();
    let thirteen = |value: &str| vec![value; 13].join(" ");
    let cycle = format!(
        r#"<timecycle_keyframe_data><High_Level_Vars><cloud_cover> 0.25</cloud_cover></High_Level_Vars>
        <cycle name="{cycle}" regions="1"><region name="GLOBAL">
        <sky_scattering_sun_illuminance> {}</sky_scattering_sun_illuminance>
        <sky_scattering_moon_illuminance> {}</sky_scattering_moon_illuminance>
        {rows}</region></cycle></timecycle_keyframe_data>"#,
        thirteen("1000"),
        thirteen("2"),
    );
    or_title::timecycle_of_documents([
        format!("<time_data>{samples}</time_data>").as_str(),
        cycle.as_str(),
    ])
}

fn plain(hours: f32, cover: Option<f32>) -> Sky {
    Sky {
        towards: Vec3::Z,
        sun: Vec3::ONE,
        sky: Vec3::splat(hours),
        ground: Vec3::splat(cover.unwrap_or(-1.0)),
        exposure: 1.0,
    }
}

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).length() <= 0.01 * b.length().max(1e-3)
}

#[test]
fn the_sun_rises_in_the_east_tops_in_the_south_and_is_under_the_floor_at_night() {
    // The bent clock is a little late at the sun's own noon, so it stands a hair west of south.
    let noon = sun_towards(SOLAR_NOON * 24.0);
    assert!(close(noon, Vec3::new(-0.0049, -0.4352, 0.9003)), "{noon:?}");
    assert!((noon.z - LATITUDE.cos()).abs() < 1e-3, "{noon:?}");
    // The clock is bent between the two quarters of an hour, so the sun is at the horizon there.
    let (rise, set) = (sun_towards(SUNRISE * 24.0), sun_towards(SUNSET * 24.0));
    assert!(rise.x > 0.99 && rise.z.abs() < 0.1, "{rise:?}");
    assert!(set.x < -0.99 && set.z.abs() < 0.1, "{set:?}");
    assert!(sun_towards(1.0).z < 0.0 && sun_towards(23.0).z < 0.0);
    // Morning and evening, as worked out apart from this code.
    assert!(close(sun_towards(12.0), Vec3::new(0.4405, -0.3907, 0.8083)));
    assert!(close(
        sun_towards(19.0),
        Vec3::new(-0.9984, -0.0247, 0.0512)
    ));
    assert!(close(
        sun_towards(23.0),
        Vec3::new(-0.6836, 0.3177, -0.6571)
    ));
}

#[test]
fn the_sun_lingers_near_the_horizon_and_hurries_through_the_top() {
    let early = sun_clock(SUNRISE + 0.05) - SUNRISE;
    let middle = (SUNRISE + SUNSET) / 2.0;
    let late = middle - sun_clock(middle - 0.05);
    assert!(early < 0.05 && late > 0.05, "{early} and {late}");
    assert!((sun_clock(middle) - middle).abs() < 1e-6);
    // The night closes on the rise of the next day.
    assert!((sun_clock(0.999) - sun_clock(0.001) - 1.0).abs() < 0.01);
}

#[test]
fn the_moon_goes_round_the_compass_above_the_floor_and_is_north_at_midnight() {
    for hours in [0.0, 3.0, 6.0, 12.0, 18.0, 23.0] {
        let moon = moon_towards(hours);
        assert!((moon.length() - 1.0).abs() < 1e-5);
        let high = moon.z.asin().to_degrees();
        assert!((36.0..41.0).contains(&high), "{high} at {hours}");
    }
    let midnight = moon_towards(0.0);
    assert!(midnight.y > 0.7 && midnight.x.abs() < 1e-4, "{midnight:?}");
    assert!(moon_towards(6.0).x < -0.7, "west at six");
}

#[test]
#[allow(clippy::approx_constant)]
fn an_angle_lies_between_two_samples_and_the_round_closes_on_itself() {
    let table = data("Rain", "").timecycle.table;
    assert_eq!(samples_at(&table, 0.0), (4, 5, 0.0));
    assert_eq!(samples_at(&table, 1.57), (6, 7, 0.0));
    let (first, second, along) = samples_at(&table, 0.15);
    assert_eq!((first, second), (4, 5));
    assert!((along - 0.5).abs() < 1e-5);
    // Past the last sample of the evening the round goes on to the first of the dusk.
    let (first, second, along) = samples_at(&table, 3.12);
    assert_eq!((first, second), (8, 9));
    assert!((0.0..=1.0).contains(&along) && along > 0.5, "{along}");
    assert_eq!(
        samples_at(&table, -PI).0,
        8,
        "a hair under the turn is its end"
    );
    assert_eq!(samples_at(&table, -3.14), (9, 10, 0.0));
    assert_eq!(samples_at(&table, -1.57), (0, 1, 0.0));
    // A quarter of the way from the bent sample is far less than a quarter once bent.
    let (first, _, along) = samples_at(&table, -2.42 + 0.25 * (2.42 - 1.57));
    assert_eq!(first, 12);
    assert!((along - 8.0 * 0.25f32.powi(4)).abs() < 1e-4, "{along}");
}

#[test]
fn the_sun_lights_until_it_is_ten_degrees_under_then_the_moon_and_never_both() {
    let frame = Keyframe::default();
    let moon = Vec3::new(0.0, 0.6, 0.8);
    let at = |under: f32| {
        let (sin, cos) = under.to_radians().sin_cos();
        lighting_body(Vec3::new(cos, 0.0, -sin), moon, &frame)
    };
    assert_eq!(at(-30.0), Some((Body::Sun, 1.0)));
    assert_eq!(at(8.0), Some((Body::Sun, 1.0)));
    let (body, fade) = at(9.5).expect("a light");
    assert!(body == Body::Sun && (fade - 0.5).abs() < 0.01, "{fade}");
    let (body, fade) = at(10.5).expect("a light");
    assert!(body == Body::Moon && (fade - 0.5).abs() < 0.01, "{fade}");
    assert_eq!(at(40.0), Some((Body::Moon, 1.0)));
    // With both far under, nothing lights.
    assert_eq!(lighting_body(-Vec3::Z, -Vec3::Z, &frame), None);
    // A cycle's own cross-fade overrules the heights.
    let mut stated = Keyframe::default();
    stated.set("sun_moon_crossfade", -0.25);
    assert_eq!(
        lighting_body(Vec3::Z, moon, &stated),
        Some((Body::Moon, 0.25))
    );
    // And its own angle moves where the sun gives way.
    let mut early = Keyframe::default();
    early.set("sun_fade_threshold", 80.0);
    let low = Vec3::new(0.0, 5f32.to_radians().cos(), 5f32.to_radians().sin());
    assert_eq!(lighting_body(low, moon, &early), Some((Body::Moon, 1.0)));
}

#[test]
fn the_cycle_states_the_light_by_its_lux_its_fade_its_colours_and_its_multipliers() {
    // Sample 6 is the top of the round and 0 the middle of the night.
    let rows = "<light_dir_col_g> 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5</light_dir_col_g>
        <light_dir_mult> 3 1 1 1 1 1 2 2 1 1 1 1 3</light_dir_mult>
        <sky_scattering_sun_intensity_multiplier> 0 0 0 0 1 1 0.5 0.5 1 1 0 0 0</sky_scattering_sun_intensity_multiplier>
        <sky_scattering_sun_col_b> 1 1 1 1 1 1 0.25 0.25 1 1 1 1 1</sky_scattering_sun_col_b>
        <light_dir_moon_mult> 0.5 1 1 1 1 1 1 1 1 1 1 1 0.5</light_dir_moon_mult>
        <ambient_sky_intensity> 1 1 1 1 1 1 0.5 0.5 1 1 1 1 1</ambient_sky_intensity>
        <ambient_sky_color_mult_r> 1 1 1 1 1 1 0.5 0.5 1 1 1 1 1</ambient_sky_color_mult_r>
        <ambient_bounce_intensity> 2 2 2 2 2 2 2 2 2 2 2 2 2</ambient_bounce_intensity>";
    let data = data("Rain", rows);
    let at = |hours: f32| sky(&data, hours, WeatherState::Rain, plain).expect("lit by the cycle");

    let noon = at(SOLAR_NOON * 24.0);
    // A thousand lux, half of it, twice, by the colours.
    let sun = Vec3::new(1.0, 0.5, 0.25) * 1000.0 * 0.5 * 2.0 * SUN_EXPOSURE;
    assert!(close(noon.sun, sun), "{:?} against {sun:?}", noon.sun);
    assert!(close(noon.towards, sun_towards(SOLAR_NOON * 24.0)));
    // The built-in sky of its own noon, under the cover the cycle states, by the ambient rows.
    assert!(close(noon.sky, Vec3::new(3.0, 6.0, 6.0)), "{:?}", noon.sky);
    assert!(close(noon.ground, (noon.sky * 0.3 + noon.sun * 0.04) * 2.0));

    let night = at(SOLAR_NOON * 24.0 + 12.0);
    // Two lux of moon, three times, halved by the moon's own multiplier.
    let moon = Vec3::new(1.0, 0.5, 1.0) * 2.0 * 3.0 * 0.5 * MOON_EXPOSURE;
    assert!(close(night.sun, moon), "{:?} against {moon:?}", night.sun);
    assert!(close(night.towards, moon_towards(SOLAR_NOON * 24.0 + 12.0)));

    // A sun at the horizon lights from no lower than the floor of the light's way.
    let low = at(SUNSET * 24.0).towards;
    assert!(
        (low.z - LIGHT_FLOOR).abs() < 0.02 && low.x < -0.9,
        "{low:?}"
    );
}

#[test]
fn the_cover_a_cycle_states_reaches_the_built_in_sky() {
    let data = data("Rain", "");
    let seen = std::cell::Cell::new(None);
    let built_in = |hours, cover| {
        seen.set(cover);
        plain(hours, cover)
    };
    sky(&data, 12.0, WeatherState::Rain, built_in).expect("lit by the cycle");
    assert_eq!(seen.get(), Some(0.25));
}

#[test]
fn each_weather_takes_a_cycle_and_without_it_or_the_table_nothing_is_stated() {
    for (weather, cycle) in [
        (WeatherState::Clear, "SUNNY"),
        (WeatherState::Cloudy, "clouds"),
        (WeatherState::Fog, "FOG"),
        (WeatherState::Rain, "RAIN"),
        (WeatherState::Storm, "ThunderStorm"),
    ] {
        let data = data(cycle, "");
        assert!(lights(&data, weather), "{weather:?}");
        assert!(sky(&data, 9.0, weather, plain).is_some());
    }
    let rain = data("Rain", "");
    assert!(!lights(&rain, WeatherState::Clear));
    assert!(sky(&rain, 9.0, WeatherState::Clear, plain).is_none());
    assert!(!lights(&TimecycleData::default(), WeatherState::Rain));
    assert!(sky(&TimecycleData::default(), 9.0, WeatherState::Rain, plain).is_none());
}
