use super::*;

fn table() -> DayTable {
    let samples: String = (0..13)
        .map(|k| {
            let kind = if k == 1 {
                "EASE_QUARTIC_INOUT"
            } else {
                "EASE_LINEAR_IN"
            };
            format!(r#"<sample name="s{k}" hour="{k}" uw_tc_mod="u" interpolation_type="{kind}"/>"#)
        })
        .collect();
    DayTable::parse(&format!(
        r#"<time_data version="1.0" mode="dynamic">{samples}<region name="URBAN"/></time_data>"#
    ))
}

#[test]
fn the_time_table_states_its_samples_their_easing_and_its_regions() {
    let table = table();
    assert_eq!(table.samples.len(), 13);
    assert_eq!(
        (table.samples[3].name.as_str(), table.samples[3].hour),
        ("s3", 3.0)
    );
    assert_eq!(table.samples[1].easing, Easing::QuarticInOut);
    assert_eq!(table.samples[0].easing, Easing::Linear);
    assert_eq!(table.regions, ["URBAN"]);
}

#[test]
fn a_sample_is_found_by_the_suns_angle_and_eased_by_the_first_of_the_two() {
    let table = table();
    // The sun on the eastern horizon is the fifth sample whole, and its top the seventh.
    assert_eq!(table.locate(0.0), Some((4, 5, 0.0)));
    let (first, second, along) = table.locate(1.57).expect("thirteen samples");
    assert_eq!((first, second), (6, 7));
    assert!(along.abs() < 1e-6);
    // Half way from the second sample to the third, bent by the second sample's own easing.
    let (first, second, along) = table
        .locate((-0.45 + -0.22) / 2.0)
        .expect("thirteen samples");
    assert_eq!((first, second), (1, 2));
    assert!(
        (along - 0.5).abs() < 1e-4,
        "the quartic passes through its middle: {along}"
    );
    let (_, _, early) = table.locate(-0.45 + 0.23 * 0.25).expect("thirteen samples");
    assert!((early - 8.0 * 0.25f32.powi(4)).abs() < 1e-3, "{early}");
    // The round closes: past the last row the way runs on to the first, and an angle under the first is a turn on.
    assert_eq!(table.locate(3.12).map(|l| (l.0, l.1)), Some((8, 9)));
    assert_eq!(table.locate(-3.2).map(|l| (l.0, l.1)), Some((7, 8)));
    assert_eq!(DayTable::default().locate(0.0), None);
}

#[test]
fn the_sun_rises_in_the_east_tops_in_the_south_and_is_under_the_floor_at_night() {
    let noon = SOLAR_NOON * 24.0;
    let at = |hours: f32| sun_towards(sun_round(hours), LATITUDE);
    // The clock bends about the middle of sunrise and sunset, which is not noon, so the top is near noon only.
    let top = at(noon);
    assert!(top[0].abs() < 0.01 && top[1] < 0.0, "{top:?}");
    assert!((top[2] - LATITUDE.cos()).abs() < 1e-4, "{top:?}");
    assert!((sun_round(noon) - PI / 2.0).abs() < 0.01);
    assert!((sun_angle(top) - PI / 2.0).abs() < 0.01);
    // At the bend's ends the clock is the day's own, and the sun is on the horizon the search allows.
    let turn = |a: f32, b: f32| ((a - b + PI).rem_euclid(TAU) - PI).abs();
    for part in [SUNRISE, SUNSET] {
        let unbent = PI / 2.0 + (part - SOLAR_NOON) * TAU;
        assert!(turn(sun_round(part * 24.0), unbent) < 1e-4, "{part}");
    }
    let rise = at(SUNRISE * 24.0);
    assert!(rise[0] > 0.99 && rise[2].abs() < 0.05, "{rise:?}");
    let set = at(SUNSET * 24.0);
    assert!(set[0] < -0.99 && set[2].abs() < 0.05, "{set:?}");
    assert!(at(1.0)[2] < 0.0 && at(23.0)[2] < 0.0);
    // It lingers near the horizon: an hour after its rise it has turned less than a twelfth of a half turn.
    assert!(sun_round(SUNRISE * 24.0 + 1.0) - sun_round(SUNRISE * 24.0) < PI / 12.0);
}

#[test]
fn the_frames_hour_puts_its_sun_where_the_round_is() {
    assert_eq!(frame_hours(0.0), 6.0);
    assert_eq!(frame_hours(PI / 2.0), 13.0);
    assert_eq!(frame_hours(PI), 20.0);
    let midnight = frame_hours(-PI / 2.0);
    assert!(!(1e-3..23.999).contains(&midnight), "{midnight}");
    assert!((frame_hours(-PI / 4.0) - 3.0).abs() < 1e-4);
    assert!((frame_hours(-3.0 * PI / 4.0) - 22.0).abs() < 1e-4);
    assert!(sun_lights(6.0) && sun_lights(21.0) && !sun_lights(22.0) && !sun_lights(3.0));
    assert!((sun_roll(0.0) - PI / 2.0).abs() < 1e-6);
}

#[test]
fn the_values_before_a_cycle_and_the_modifiers_of_a_file_are_read() {
    let cycle = r#"<timecycle_keyframe_data version="2.0">
  <High_Level_Vars>
    <cloud_cover> 0.200000</cloud_cover>
    <Fog> 1.5</Fog>
  </High_Level_Vars>
  <cycle name="Sunny" regions="1"><region name="GLOBAL"><fog_start> 1 2</fog_start></region></cycle>
</timecycle_keyframe_data>"#;
    let vars = high_level_vars(cycle);
    assert_eq!(vars.len(), 2);
    assert_eq!((vars["cloud_cover"], vars["Fog"]), (0.2, 1.5));
    assert!(high_level_vars("<timecycle_keyframe_data/>").is_empty());

    let mods = modifiers(
        r#"<timecycle_modifier_data version="1.0">
  <modifier name="a" description="" userFlags="8">
    <light_dir_mult>1.5 1.000000</light_dir_mult>
    <broken>1.0</broken>
  </modifier>
  <modifier name="b" userFlags="0"></modifier>
</timecycle_modifier_data>"#,
    );
    assert_eq!(mods.len(), 2);
    assert_eq!((mods[0].name.as_str(), mods[0].user_flags), ("a", 8));
    assert_eq!(
        mods[0].vars.len(),
        1,
        "a variable with one number is left out"
    );
    assert_eq!(mods[0].vars["light_dir_mult"], [1.5, 1.0]);
    assert!(mods[1].vars.is_empty());
}

fn own(pairs: &[(&str, f32)]) -> Keyframe {
    let mut frame = Keyframe::default();
    for (name, value) in pairs {
        frame.set(name, *value);
    }
    frame
}

fn vars(pairs: &[(&str, [f32; 2])]) -> BTreeMap<String, [f32; 2]> {
    (pairs.iter())
        .map(|(name, values)| (name.to_string(), *values))
        .collect()
}

#[test]
fn a_modifier_meets_each_variable_by_its_kind() {
    assert_eq!(modifier_kind("lod_mult_hd"), ModifierKind::Leave);
    assert_eq!(modifier_kind("light_dir_mult"), ModifierKind::InterpMult);
    assert_eq!(modifier_kind("light_dir_col_g"), ModifierKind::InterpMult);
    assert_eq!(
        modifier_kind("local_atmosphere_fog_density"),
        ModifierKind::Interp
    );
    let mut keyframe = own(&[
        ("local_atmosphere_fog_density", 0.004),
        ("light_dir_mult", 2.0),
        ("lod_mult_hd", 1.0),
    ]);
    apply_modifier(
        &mut keyframe,
        &vars(&[
            // Taken to the first value: the second is not read for this kind.
            ("local_atmosphere_fog_density", [0.0, 0.5]),
            // A quarter of the way towards the first value.
            ("light_dir_mult", [6.0, 0.25]),
            ("lod_mult_hd", [9.0, 1.0]),
            ("sky_new_mie_phase_eccentricity", [0.7, 0.0]),
        ]),
    );
    assert_eq!(keyframe.get("local_atmosphere_fog_density"), Some(0.0));
    assert_eq!(keyframe.get("light_dir_mult"), Some(3.0));
    assert_eq!(keyframe.get("lod_mult_hd"), Some(1.0), "a scale is left");
    assert_eq!(
        keyframe.get("sky_new_mie_phase_eccentricity"),
        Some(0.7),
        "a variable the cycle does not state is the modifier's"
    );
    // A tint the cycle does not state is not blended towards from nothing.
    let mut bare = Keyframe::default();
    apply_modifier(&mut bare, &vars(&[("light_dir_col_r", [0.5, 0.5])]));
    assert_eq!(bare.get("light_dir_col_r"), None);
}

#[test]
fn two_weathers_modifiers_blend_value_by_value_and_one_alone_stands() {
    let a = vars(&[("fog", [1.0, 0.0]), ("only_a", [3.0, 1.0])]);
    let b = vars(&[("fog", [3.0, 1.0]), ("only_b", [5.0, 0.5])]);
    let blend = blend_modifiers(&a, &b, 0.25);
    assert_eq!(blend["fog"], [1.5, 0.25]);
    assert_eq!(blend["only_a"], [3.0, 1.0]);
    assert_eq!(blend["only_b"], [5.0, 0.5]);
    assert_eq!(blend_modifiers(&a, &b, 1.0)["fog"], [3.0, 1.0]);
}

/// A moment lit by a sun of 100,000 lux at the zenith, through clear air, exposed at -12 stops.
fn noon() -> FrameLight {
    let radiance = |r: f32, g: f32, b: f32| [r, g, b];
    FrameLight {
        exposure: (-12f32).exp2(),
        fade: 1.0,
        illuminance: 100_000.0,
        sun_illuminance: 100_000.0,
        body_transmittance: [0.9, 0.8, 0.6],
        sun_transmittance: [0.9, 0.8, 0.6],
        towards_up: 1.0,
        sky: [4000.0, 8000.0, 16000.0],
        dome: [
            radiance(1000.0, 2000.0, 4000.0),
            radiance(1500.0, 2500.0, 4000.0),
            radiance(4000.0, 4000.0, 4000.0),
            radiance(3000.0, 3000.0, 3500.0),
            radiance(2000.0, 2500.0, 3000.0),
        ],
        // East a quarter of the way from its horizon to the transition colour, west beyond it.
        dome_sides: [
            radiance(3375.0, 3625.0, 4000.0),
            radiance(1000.0, 2000.0, 4000.0),
        ],
        fog: [[4000.0, 4000.0, 4000.0]; 4],
        haze: 1.5e-5,
    }
}

#[test]
fn every_light_the_frame_takes_is_physical_times_the_exposure_and_the_frames_own_is_held_at_nought()
{
    let light = noon();
    let lit = light.exposure * FRAME_DIFFUSE;
    let frame = frame_keyframe(
        &own(&[
            ("light_dir_mult", 1.5),
            ("light_dir_col_g", 0.5),
            ("postfx_exposure", 2.0),
            ("postfx_exposure_min", -12.0),
            ("ambient_sky_intensity", 0.25),
            ("ambient_bounce_intensity", 2.0),
        ]),
        &light,
    );
    for name in [
        "postfx_exposure",
        "postfx_exposure_min",
        "postfx_exposure_max",
    ] {
        assert_eq!(frame.get(name), Some(0.0), "{name}");
    }
    assert_eq!(frame.get("postfx_tonemap_filmic_override_dark"), Some(0.0));
    let strength = frame.get("light_dir_mult").expect("worked out");
    assert!(
        (strength - 1.5 * 100_000.0 * lit).abs() < 1e-3,
        "{strength}"
    );
    // A sun of 100,000 lux at -12 stops is a few frame units, not thousands and not a tenth.
    assert!((5.0..15.0).contains(&(strength / 1.5)), "{strength}");
    assert_eq!(frame.get("light_dir_col_g"), Some(0.5 * 0.8));

    // The sky lights from above at its irradiance, and again by the cycle's intensity; the colour's largest part is one.
    let down = frame
        .get("light_natural_amb_down_intensity")
        .expect("worked out");
    assert!((down - 16000.0 * 1.25 * lit).abs() < 1e-4, "{down}");
    assert_eq!(frame.get("light_natural_amb_down_col_b"), Some(1.0));
    assert_eq!(frame.get("light_natural_amb_down_col_r"), Some(0.25));
    // From below: a fifth of what reaches the ground, by the bounce's intensity; red is its largest part.
    let up = frame
        .get("light_natural_amb_up_intensity")
        .expect("worked out");
    let reaching_r = 1.5 * 100_000.0 * 0.9 + 4000.0 * 1.25;
    assert!((up - 0.2 * 2.0 * reaching_r * lit).abs() < 1e-3, "{up}");
    // The artificial light every surface takes a share of is white, an illuminance exposed like the rest.
    for way in ["down", "up"] {
        let intensity = frame.get(&format!("light_artificial_ext_{way}_intensity"));
        assert_eq!(intensity, Some(ARTIFICIAL_AMBIENT_LUX * lit), "{way}");
        assert_eq!(
            frame.get(&format!("light_artificial_ext_{way}_col_g")),
            Some(1.0)
        );
    }
}

#[test]
fn the_dome_is_the_skys_radiance_exposed_and_given_as_a_root_the_frame_squares() {
    let light = noon();
    let frame = frame_keyframe(&Keyframe::default(), &light);
    let inten = frame.get("sky_zenith_col_inten").expect("worked out");
    assert!((inten - 4000.0 * light.exposure).abs() < 1e-5, "{inten}");
    let r = frame.get("sky_zenith_col_r").expect("worked out");
    assert!((r * r - 0.25).abs() < 1e-6, "{r}");
    assert_eq!(frame.get("sky_hdr"), Some(1.0));
    // The zenith is bluer than the eastern horizon the sun whitens.
    let east_r = frame.get("sky_azimuth_east_col_r").expect("worked out");
    assert!(east_r > r);
    // The sun's glow is its tint through the air, given as a root; its strength its illuminance exposed as a light.
    let sun_b = frame.get("sky_sun_col_b").expect("worked out");
    assert!((sun_b * sun_b - 0.6).abs() < 1e-6);
    let sun = frame.get("sky_sun_hdr").expect("worked out");
    assert!((sun - 100_000.0 * light.exposure * FRAME_DIFFUSE).abs() < 1e-3);
    // The disc is a radiance, the illuminance times its multiplier exposed whole.
    let frame = frame_keyframe(
        &own(&[("sky_scattering_sun_disk_intensity_multiplier", 256.0)]),
        &light,
    );
    let disc_b = frame.get("sky_sun_disc_col_b").expect("worked out");
    let drawn = disc_b * disc_b * frame.get("sky_sun_hdr").expect("worked out");
    let radiance = 0.6 * 100_000.0 * 256.0 * light.exposure;
    assert!(
        (drawn - radiance).abs() < 1e-4 * radiance,
        "{drawn} {radiance}"
    );
    // The dome takes its transition colour whole at the height the transition radiance was taken at.
    assert_eq!(
        frame.get("sky_zenith_transition_position"),
        Some(DOME_TRANSITION)
    );
    let east = frame
        .get("sky_zenith_transition_east_blend")
        .expect("worked out");
    assert!((east - 0.25).abs() < 1e-6, "{east}");
    assert_eq!(frame.get("sky_zenith_transition_west_blend"), Some(1.0));
}

#[test]
fn a_night_without_its_body_is_dark_and_the_moon_and_stars_are_exposed_as_lights() {
    let night = FrameLight {
        exposure: 4f32.exp2(),
        fade: 1.0,
        illuminance: 0.25,
        body_transmittance: [0.7, 0.6, 0.5],
        towards_up: 0.6,
        ..FrameLight::default()
    };
    let frame = frame_keyframe(
        &own(&[
            ("light_dir_mult", 0.5),
            ("sky_moon_iten", 100.0),
            ("sky_stars_iten", 0.02),
        ]),
        &night,
    );
    let strength = frame.get("light_dir_mult").expect("worked out");
    assert!(strength > 0.0 && strength < 1.0, "{strength}");
    assert_eq!(frame.get("sky_sun_hdr"), Some(0.0), "the sun's disc is out");
    assert_eq!(frame.get("sky_sun_col_r"), Some(0.0));
    let lit = night.exposure * FRAME_DIFFUSE;
    assert_eq!(frame.get("sky_moon_iten"), Some(100.0 * lit));
    assert_eq!(frame.get("sky_stars_iten"), Some(0.02 * lit));
    assert_eq!(frame.get("light_natural_amb_down_intensity"), Some(0.0));
}

#[test]
fn the_local_atmospheres_fog_is_given_in_the_frames_units() {
    let frame = frame_keyframe(
        &own(&[
            ("local_atmosphere_fog_density", 0.003),
            ("local_atmosphere_fog_height", 40.0),
            ("local_atmosphere_fog_falloff", 250.0),
        ]),
        &noon(),
    );
    let density = frame.get("fog_density").expect("stated");
    assert!((density - 30.0).abs() < 1e-4, "{density}");
    assert_eq!(frame.get("fog_base_height"), Some(40.0));
    assert_eq!(frame.get("fog_falloff"), Some(4.0));
    let clear = frame_keyframe(&own(&[("local_atmosphere_fog_falloff", 0.0)]), &noon());
    assert_eq!(clear.get("fog_falloff"), None);
    assert_eq!(clear.get("fog_density"), None);
    assert_eq!(clear.get("fog_hdr"), Some(1.0), "the colours are exposed");
    // The haze is the air's extinction at the ground, and what it fades to a radiance at the exposure.
    assert_eq!(
        clear.get("fog_haze_density"),
        Some(noon().haze * FRAME_FOG_DENSITY)
    );
    let haze = clear.get("fog_haze_col_r").expect("filled");
    assert!((haze - 4000.0 * noon().exposure).abs() < 1e-6, "{haze}");
}

#[test]
fn what_gta_vi_states_under_the_frames_name_passes_and_the_rest_is_left_to_the_frame() {
    let frame = frame_keyframe(
        &own(&[
            ("postfx_tonemap_filmic_w", 11.2),
            ("far_clip", 7000.0),
            ("sky_new_planet_radius", 6360.0),
        ]),
        &noon(),
    );
    assert_eq!(frame.get("far_clip"), Some(7000.0));
    // The tone curves are the visual settings', not a moment's.
    assert_eq!(frame.get("postfx_tonemap_filmic_w"), None);
    assert_eq!(frame.get("sky_new_planet_radius"), None);

    let mut all: Vec<(&str, f32)> = SAME.iter().map(|name| (*name, 1.0)).collect();
    all.extend([
        ("sky_moon_iten", 1.0),
        ("sky_stars_iten", 1.0),
        ("local_atmosphere_fog_density", 1.0),
        ("local_atmosphere_fog_height", 1.0),
        ("local_atmosphere_fog_falloff", 1.0),
    ]);
    let whole = frame_keyframe(&own(&all), &noon());
    assert_eq!(whole.len(), filled().count());
    for name in filled() {
        assert!(whole.get(name).is_some(), "{name}");
    }
}
