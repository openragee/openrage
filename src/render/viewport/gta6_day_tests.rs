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
        TUNED,
    ])
}

/// An install's tuning values: the curve a metered frame is exposed by, two filmic curves and
/// how the sun gives way.
const TUNED: &str = "# Visual Configuration Table
Exposure.curve.scale -1.5
Exposure.curve.verticalOffset 7
Exposure.curve.blend.scale -2
Tonemapping.bright.filmic.A 0.25
Tonemapping.bright.filmic.exposure -5
Tonemapping.dark.filmic.A 0.5
Tonemapping.dark.filmic.exposure -2.5
misc.SunFade.ThresholdLength 10
";

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
    let plain = VisualSettings::default();
    let moon = Vec3::new(0.0, 0.6, 0.8);
    let under = |degrees: f32| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Vec3::new(cos, 0.0, -sin)
    };
    let at = |degrees: f32| lighting_body(under(degrees), moon, &frame, &plain);
    assert_eq!(at(-30.0), Some((Body::Sun, 1.0)));
    assert_eq!(at(8.0), Some((Body::Sun, 1.0)));
    let (body, fade) = at(9.5).expect("a light");
    assert!(body == Body::Sun && (fade - 0.5).abs() < 0.01, "{fade}");
    let (body, fade) = at(10.5).expect("a light");
    assert!(body == Body::Moon && (fade - 0.5).abs() < 0.01, "{fade}");
    assert_eq!(at(40.0), Some((Body::Moon, 1.0)));
    // With both far under, nothing lights.
    assert_eq!(lighting_body(-Vec3::Z, -Vec3::Z, &frame, &plain), None);
    // An install's own angle and the degrees it fades over: half gone five degrees before.
    let tuned =
        VisualSettings::parse("misc.SunFade.Threshold 96\nmisc.SunFade.ThresholdLength 10\n");
    let (body, fade) = lighting_body(under(1.0), moon, &frame, &tuned).expect("a light");
    assert!(body == Body::Sun && (fade - 0.5).abs() < 0.01, "{fade}");
    // A cycle's own cross-fade overrules the heights.
    let mut stated = Keyframe::default();
    stated.set("sun_moon_crossfade", -0.25);
    assert_eq!(
        lighting_body(Vec3::Z, moon, &stated, &plain),
        Some((Body::Moon, 0.25))
    );
    // And its own angle moves where the sun gives way, over the install's.
    let mut early = Keyframe::default();
    early.set("sun_fade_threshold", 80.0);
    let low = Vec3::new(0.0, 5f32.to_radians().cos(), 5f32.to_radians().sin());
    assert_eq!(
        lighting_body(low, moon, &early, &plain),
        Some((Body::Moon, 1.0))
    );
}

#[test]
fn the_cycle_states_the_light_by_its_lux_its_tint_its_multipliers_and_the_air() {
    // Sample 6 is the top of the round and 0 the middle of the night.
    let rows = "<light_dir_col_g> 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5</light_dir_col_g>
        <light_dir_mult> 3 1 1 1 1 1 2 2 1 1 1 1 3</light_dir_mult>
        <sky_scattering_sun_intensity_multiplier> 0 0 0 0 1 1 0.5 0.5 1 1 0 0 0</sky_scattering_sun_intensity_multiplier>
        <light_dir_moon_mult> 0.5 1 1 1 1 1 1 1 1 1 1 1 0.5</light_dir_moon_mult>
        <ambient_bounce_intensity> 2 2 2 2 2 2 2 2 2 2 2 2 2</ambient_bounce_intensity>";
    let ambient = "<ambient_sky_intensity> 1 1 1 1 1 1 0.5 0.5 1 1 1 1 1</ambient_sky_intensity>
        <ambient_sky_color_mult_r> 1 1 1 1 1 1 0.5 0.5 1 1 1 1 1</ambient_sky_color_mult_r>
        <sky_scattering_sky_col_b> 1 1 1 1 1 1 0.5 0.5 1 1 1 1 1</sky_scattering_sky_col_b>";
    let data = data("Rain", &format!("{rows}{ambient}"));
    let at = |hours: f32| sky(&data, hours, WeatherState::Rain).expect("lit by the cycle");
    let air = Air::of(&Keyframe::default());
    let noon_hours = SOLAR_NOON * 24.0;

    let noon = at(noon_hours);
    // A thousand lux, half of it, twice, by the tint, through the air, with the air's own
    // share of a matte surface's light.
    let through = air.light_through(sun_towards(noon_hours)) / PI;
    let sun = Vec3::new(1.0, 0.5, 1.0) * 1000.0 * 0.5 * 2.0 * through;
    assert!(close(noon.sun, sun), "{:?} against {sun:?}", noon.sun);
    assert!(
        sun.x > sun.z && sun.x > 200.0,
        "the air takes the blue: {sun:?}"
    );
    assert!(close(noon.towards, sun_towards(noon_hours)));
    assert_eq!(noon.exposure, 1.0);

    // The sky is the air's under the sun's lux, by the rows that multiply it: without them it
    // is twice, four times and twice what it is with them.
    let bare = sky(&self::data("Rain", rows), noon_hours, WeatherState::Rain).expect("lit");
    let of_the_air = air.sky_on_the_ground(sun_towards(noon_hours)) * 500.0 / PI;
    assert!(
        close(bare.sky, of_the_air),
        "{:?} against {of_the_air:?}",
        bare.sky
    );
    assert!(
        close(noon.sky * Vec3::new(4.0, 2.0, 4.0), bare.sky),
        "{:?}",
        noon.sky
    );
    assert!(noon.sky.z > noon.sky.x, "blue: {:?}", noon.sky);
    // The clouds cover a quarter of the sky and their shadow shows whole: three quarters of
    // the light comes through, onto the model and onto the ground that lights it from below.
    assert_eq!(noon.scene.clouds, clouds_of(0.25, &Keyframe::default()));
    assert_eq!(noon.scene.clouds.open, 0.75);
    let ground = (noon.sky + noon.sun * 0.75 * noon.towards.z / PI) * GROUND * 2.0;
    assert!(close(noon.ground, ground), "{:?}", noon.ground);
    // A shadow that shows by half hides half as much, and the rows of the light under it are
    // the cycle's.
    let mut stated = Keyframe::default();
    stated.set("local_atmosphere_cloud_shadow_alpha", 0.5);
    stated.set("direct_light_roughness_mod_cloud_shadow", 0.3);
    stated.set("direct_light_haze_blend_scale", 0.2);
    stated.set("direct_light_haze_intensity", 4.0);
    let under = clouds_of(0.8, &stated);
    assert_eq!(
        (
            under.open,
            under.roughness_mod,
            under.haze_blend,
            under.haze
        ),
        (0.6, 0.3, 0.2, 4.0)
    );
    assert_eq!(clouds_of(0.0, &stated).open, 1.0);

    let night = at(noon_hours + 12.0);
    // Two lux of moon, three times, halved by the moon's own multiplier.
    let through = air.light_through(moon_towards(noon_hours + 12.0)) / PI;
    let moon = Vec3::new(1.0, 0.5, 1.0) * 2.0 * 3.0 * 0.5 * through;
    assert!(close(night.sun, moon), "{:?} against {moon:?}", night.sun);
    assert!(close(night.towards, moon_towards(noon_hours + 12.0)));
    assert!(night.sky.y < noon.sky.y * 0.02, "{:?}", night.sky);

    // A sun at the horizon lights from no lower than the floor of the light's way.
    let low = at(SUNSET * 24.0).towards;
    assert!(
        (low.z - LIGHT_FLOOR).abs() < 0.02 && low.x < -0.9,
        "{low:?}"
    );
}

#[test]
fn the_tone_is_the_cycle_s_rows_and_the_install_s_tuning_values() {
    let rows = "<postfx_exposure_lowlights_percent> 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9</postfx_exposure_lowlights_percent>
        <postfx_exposure_highlights_percent> 0.2 0.2 0.2 0.2 0.2 0.2 0.2 0.2 0.2 0.2 0.2 0.2 0.2</postfx_exposure_highlights_percent>
        <postfx_exposure_min> -9 -9 -9 -9 -9 -9 -9 -9 -9 -9 -9 -9 -9</postfx_exposure_min>
        <postfx_exposure> 1 1 1 1 1 1 1 1 1 1 1 1 1</postfx_exposure>
        <postfx_exposure_tweak> 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5</postfx_exposure_tweak>
        <direct_light_roughness_mod> 0.4 0.4 0.4 0.4 0.4 0.4 0.4 0.4 0.4 0.4 0.4 0.4 0.4</direct_light_roughness_mod>
        <postfx_tonemap_filmic_a> 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1</postfx_tonemap_filmic_a>
        <postfx_tonemap_filmic_b> 0.3 0.3 0.3 0.3 0.3 0.3 0.3 0.3 0.3 0.3 0.3 0.3 0.3</postfx_tonemap_filmic_b>
        <postfx_tonemap_filmic_exposure_dark> 1 1 1 1 1 1 1 1 1 1 1 1 1</postfx_tonemap_filmic_exposure_dark>";
    let tone_in = |rows: &str| {
        let sky = sky(&data("Rain", rows), 12.0, WeatherState::Rain).expect("lit by the cycle");
        sky.tone.expect("a cycle that lights states a tone")
    };
    let tone = tone_in(rows);
    // The brightest share is cut to what the darkest leaves of all but a hundredth.
    assert_eq!((tone.lowlights, tone.roughness_mod), (0.9, 0.4));
    assert!((tone.highlights - 0.09).abs() < 1e-6, "{}", tone.highlights);
    assert_eq!((tone.least, tone.most, tone.added), (-9.0, 32.0, 1.5));
    assert_eq!((tone.key, tone.blend), (None, 0.0));
    assert_eq!((tone.curve[0].scale, tone.curve[0].vertical), (-1.5, 7.0));
    assert_eq!((tone.curve[1].scale, tone.curve[1].vertical), (-2.0, 0.0));
    assert!((0.008..0.009).contains(&tone.disc), "{}", tone.disc);
    // The install's two curves, a number it does not state being the cycle's row.
    assert_eq!((tone.bright.0[0], tone.dark.0[0]), (0.25, 0.5));
    assert_eq!((tone.bright.0[1], tone.dark.0[1]), (0.0, 0.3));
    assert_eq!((tone.bright_at, tone.dark_at), (-5.0, -2.5));

    // A cycle that overrules the dark curve gives its rows to both, and the place they state.
    let dark = "<postfx_tonemap_filmic_override_dark> 1 1 1 1 1 1 1 1 1 1 1 1 1</postfx_tonemap_filmic_override_dark>";
    let overruled = tone_in(&format!("{rows}{dark}"));
    assert_eq!(overruled.bright, overruled.dark);
    assert_eq!((overruled.dark.0[0], overruled.dark_at), (0.1, 1.0));
    // And a cycle that meters by its key says the key.
    let key = "<postfx_exposure_use_key_value_mode> 1 1 1 1 1 1 1 1 1 1 1 1 1</postfx_exposure_use_key_value_mode>
        <postfx_exposure_key_value> 0.25 0.25 0.25 0.25 0.25 0.25 0.25 0.25 0.25 0.25 0.25 0.25 0.25</postfx_exposure_key_value>";
    assert_eq!(tone_in(key).key, Some(0.25));
}

#[test]
fn a_weather_is_drawn_with_the_cycle_its_type_names_under_that_type_s_modifiers() {
    let mut data = data("Layered", "");
    let lit = |data: &TimecycleData| sky(data, SOLAR_NOON * 24.0, WeatherState::Rain);
    assert!(lit(&data).is_none(), "no cycle of the weather's own name");
    data.gta6_weather.add_types(
        "<WeatherTypeDatas><Item><Name>Wet</Name><TimeCycleName>LAYERED</TimeCycleName></Item></WeatherTypeDatas>
        <WeatherTypes><Item><Name>AM_Rain</Name><DataName>Wet</DataName>
        <ModAtmosphere>M_air</ModAtmosphere></Item></WeatherTypes>
        <WeatherTypeIndirectionList><Item><From>Rain</From><To>AM_Rain</To></Item></WeatherTypeIndirectionList>",
    );
    assert!(lights(&data, WeatherState::Rain) && !lights(&data, WeatherState::Clear));
    let plain = lit(&data).expect("the type's cycle lights");
    data.gta6_weather.add_modifiers(
        r#"<modifier name="M_air"><light_dir_mult>4.000000 0.000000</light_dir_mult></modifier>"#,
    );
    let modified = lit(&data).expect("the type's cycle lights");
    assert!(close(modified.sun, plain.sun * 4.0), "{:?}", modified.sun);
}

#[test]
fn each_weather_takes_a_cycle_and_without_it_the_table_or_the_curve_nothing_is_stated() {
    for (weather, cycle) in [
        (WeatherState::Clear, "SUNNY"),
        (WeatherState::Cloudy, "clouds"),
        (WeatherState::Fog, "FOG"),
        (WeatherState::Rain, "RAIN"),
        (WeatherState::Storm, "ThunderStorm"),
    ] {
        let data = data(cycle, "");
        assert!(lights(&data, weather), "{weather:?}");
        assert!(sky(&data, 9.0, weather).is_some());
    }
    let mut rain = data("Rain", "");
    assert!(!lights(&rain, WeatherState::Clear));
    assert!(sky(&rain, 9.0, WeatherState::Clear).is_none());
    assert!(!lights(&TimecycleData::default(), WeatherState::Rain));
    assert!(sky(&TimecycleData::default(), 9.0, WeatherState::Rain).is_none());
    // An install that states no curve to expose a metered frame by is not lit by its cycles.
    rain.visual_settings = VisualSettings::default();
    assert!(!lights(&rain, WeatherState::Rain));
    assert!(sky(&rain, 9.0, WeatherState::Rain).is_none());
}
