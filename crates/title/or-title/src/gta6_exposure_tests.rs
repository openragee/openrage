use super::*;

fn keyframe(pairs: &[(&str, f32)]) -> Keyframe {
    let mut k = Keyframe::default();
    for (name, value) in pairs {
        k.set(name, *value);
    }
    k
}

/// Settings of this test's own, not an install's.
fn settings() -> ExposureSettings {
    ExposureSettings {
        min_step: 0.2,
        max_step: 10.0,
        step_mult: 1.0,
        bodies: LightTunables::image_defaults(),
    }
}

/// A day of this test's own: a sun of 100,000 lux, a moon of a third of a lux, a sky light at three tenths.
fn day() -> Keyframe {
    keyframe(&[
        ("sky_scattering_sun_illuminance", 100_000.0),
        ("sky_scattering_moon_illuminance", 0.3),
        ("light_dir_mult", 1.0),
        ("ambient_sky_intensity", 0.3),
        ("postfx_exposure_min", -11.5),
        ("postfx_exposure_max", 20.0),
    ])
}

/// The way towards a body `degrees` from the zenith, in the plane of east and up.
fn at_zenith(degrees: f32) -> [f32; 3] {
    let angle = degrees.to_radians();
    [angle.sin(), 0.0, angle.cos()]
}

#[test]
fn the_steps_keep_their_defaults() {
    let read = ExposureSettings::of(&|_, default| default);
    assert_eq!(
        (read.min_step, read.max_step, read.step_mult),
        (0.15, 15.0, 9.0)
    );
}

#[test]
fn the_target_is_the_key_over_the_luminance_held_before_and_after_the_exposure_is_added() {
    let k = keyframe(&[
        ("postfx_exposure_key_value", 0.18),
        ("postfx_exposure_min", -4.0),
        ("postfx_exposure_max", 4.0),
        ("postfx_exposure", 3.0),
    ]);
    // Six stops over the key: held to -4, then 3 up.
    assert!((target_stops(&k, 0.18 * 64.0) + 1.0).abs() < 1e-5);
    // 3 is within the range, and 3 up from it is held to the most.
    assert_eq!(target_stops(&k, 0.18 / 8.0), 4.0);
    let unheld = target_stops(&Keyframe::default(), 1e9);
    assert!((unheld - (0.18f32 / 1e9).log2()).abs() < 1e-4, "{unheld}");
}

#[test]
fn neither_the_key_value_mode_nor_a_curve_nor_the_compensation_moves_the_target() {
    let key = keyframe(&[("postfx_exposure_key_value", 0.18)]);
    let mut k = keyframe(&[
        ("postfx_exposure_use_key_value_mode", 1.0),
        ("postfx_exposure_key_value", 0.18),
        ("postfx_exposure_blend_curve_weight", 0.25),
    ]);
    assert!((target_stops(&key, 0.72) + 2.0).abs() < 1e-5);
    // A compensation curve whose seven points lie evenly on y = x / 2 is that line.
    for i in 0..7 {
        let x = -12.0 + 2.0 * i as f32;
        k.set(&format!("postfx_exposure_compensation_ev_cp{i}_x"), x);
        k.set(&format!("postfx_exposure_compensation_ev_cp{i}_y"), x / 2.0);
    }
    assert!((compensation(&k, -3.0) + 1.5).abs() < 1e-4);
    assert!((compensation(&k, -9.0) + 4.5).abs() < 1e-4);
    assert!(compensation(&k, 5.0).abs() < 1e-4, "held to the last x");
    assert_eq!(target_stops(&k, 0.72), target_stops(&key, 0.72));
    assert_eq!(compensation(&Keyframe::default(), -3.0), 0.0);
}

#[test]
fn adaptation_moves_at_a_held_rate_and_never_past_its_target() {
    let settings = settings();
    let k = keyframe(&[("postfx_exposure_adaption_step_mult", 0.5)]);
    let step = adapt(&settings, &k, 0.0, 4.0, 0.1);
    assert!((step - 0.2).abs() < 1e-5, "{step}");
    // Slower than the least step: the least step, held at the target.
    assert_eq!(adapt(&settings, &k, 3.99, 4.0, 1.0), 4.0);
    assert!((adapt(&settings, &k, 3.0, 4.0, 0.1) - 3.05).abs() < 1e-5);
    // Faster than the most: the most.
    let fast = keyframe(&[("postfx_exposure_adaption_step_mult", 1.0)]);
    assert_eq!(adapt(&settings, &fast, 0.0, -50.0, 1.0), -10.0);
    assert_eq!(
        adapt(&settings, &k, 1.0, 4.0, 0.0),
        4.0,
        "no time, no history"
    );
}

#[test]
fn the_sun_lights_until_its_threshold_and_then_the_moon_by_its_own() {
    let k = keyframe(&[("light_dir_sun_mult", 0.5), ("light_dir_moon_mult", 0.8)]);
    let moon = at_zenith(40.0);
    let image = LightTunables::image_defaults();
    let low_sun = bodies(&k, at_zenith(95.0), moon, &image);
    assert!(low_sun.sun_leads);
    assert_eq!((low_sun.sun_fade, low_sun.fade), (1.0, 0.5));
    assert!(
        (low_sun.towards[2] - crate::gta6_day_sun::MIN_LIGHT_Z).abs() < 0.02,
        "{:?}",
        low_sun.towards
    );
    let under = bodies(&k, at_zenith(100.5), moon, &image);
    assert!(!under.sun_leads);
    assert_eq!(under.sun_fade, 0.0);
    assert!((under.moon_fade - 0.5).abs() < 1e-3, "{}", under.moon_fade);
    assert!((under.fade - 0.4).abs() < 1e-3);
    assert_eq!(under.body, moon);
    // A stated threshold and a crossfade.
    let early = keyframe(&[("sun_fade_threshold", 80.0)]);
    assert_eq!(bodies(&early, at_zenith(85.0), moon, &image).sun_fade, 0.0);
    let set = |c: f32| {
        bodies(
            &keyframe(&[("sun_moon_crossfade", c)]),
            at_zenith(10.0),
            moon,
            &image,
        )
    };
    assert_eq!((set(0.3).sun_fade, set(0.3).moon_fade), (0.3, 0.0));
    assert_eq!((set(-0.4).sun_fade, set(-0.4).moon_fade), (0.0, 0.4));
}

#[test]
fn noon_is_exposed_darker_than_dusk_and_dusk_than_midnight() {
    let (k, settings) = (day(), settings());
    let moon = at_zenith(45.0);
    let (noon, noon_stops) = light(&k, at_zenith(15.0), moon, &settings);
    let (dusk, dusk_stops) = light(&k, at_zenith(89.0), moon, &settings);
    let (night, night_stops) = light(&k, at_zenith(150.0), moon, &settings);
    assert_eq!(noon_stops, -11.5, "noon is held at the day's least");
    assert!(noon_stops < dusk_stops - 2.0, "{noon_stops} {dusk_stops}");
    assert!(dusk_stops < night_stops - 3.0, "{dusk_stops} {night_stops}");
    assert_eq!(noon.exposure, noon_stops.exp2());
    assert!(night.illuminance < 1.0 && dusk.illuminance > 1000.0);
    // Through the low air the dusk sun reaches the ground at a small part of the noon sun.
    assert!(dusk.body_transmittance[1] < 0.2 * noon.body_transmittance[1]);
    // The frame's sun at noon is a few units, and the moon's light, exposed by its key, less than it.
    let frame = |light: &FrameLight| {
        or_io::gta6::timecycle::frame_keyframe(&k, light)
            .get("light_dir_mult")
            .unwrap_or(0.0)
    };
    assert!((5.0..15.0).contains(&frame(&noon)), "{}", frame(&noon));
    assert!(frame(&night) < frame(&noon), "{}", frame(&night));
}

#[test]
fn the_dome_is_lit_by_the_body_that_leads_and_the_disc_by_the_sun_alone() {
    let (k, settings) = (day(), settings());
    let (noon, _) = light(&k, at_zenith(15.0), at_zenith(45.0), &settings);
    assert!(noon.dome.iter().all(|c| c.iter().all(|v| *v > 0.0)));
    let (night, _) = light(&k, at_zenith(150.0), at_zenith(45.0), &settings);
    assert_eq!(
        night.sun_transmittance, [0.0; 3],
        "the sun is under the ground"
    );
    assert!(night.dome[0][2] < 1e-3 * noon.dome[0][2]);
    assert!(night.dome[0][2] > 0.0, "the moon lights the sky");
}

#[test]
fn the_ground_takes_the_sky_of_a_sun_under_the_horizon_where_it_stands() {
    let (k, settings) = (day(), settings());
    let moon = at_zenith(45.0);
    let (setting, _) = light(&k, at_zenith(91.0), moon, &settings);
    let (twilight, _) = light(&k, at_zenith(98.0), moon, &settings);
    assert!(twilight.illuminance > 1000.0, "the sun still leads");
    // The sky darkens as its sun sinks, the ground's with the dome's zenith.
    assert!(
        twilight.sky[1] < 0.5 * setting.sky[1],
        "{:?} {:?}",
        twilight.sky,
        setting.sky
    );
    let ground = twilight.sky[1] / setting.sky[1];
    let dome = twilight.dome[0][1] / setting.dome[0][1];
    assert!((ground / dome - 1.0).abs() < 0.5, "{ground} {dome}");
}

#[test]
fn a_storm_shades_the_ground_from_its_sun_and_lights_it_by_what_its_clouds_scatter() {
    let settings = settings();
    let (clear, _) = light(&day(), at_zenith(15.0), at_zenith(45.0), &settings);
    let mut k = day();
    k.set("cloud_cover", 1.0);
    k.set("cloud_type", 1.0);
    let (storm, _) = light(&k, at_zenith(15.0), at_zenith(45.0), &settings);
    // The sun is as strong above the clouds; what reaches the ground straight is none of it.
    assert_eq!(storm.illuminance, clear.illuminance);
    assert!(storm.body_transmittance[1] < 1e-6 * clear.body_transmittance[1]);
    // What the clouds scatter down is a small part of a clear noon's light, and not none.
    let sun = clear.illuminance * clear.body_transmittance[1] * clear.towards_up;
    let whole = sun + clear.sky[1];
    assert!(
        (0.05 * whole..0.5 * whole).contains(&storm.sky[1]),
        "{:?} {whole}",
        storm.sky
    );
}

#[test]
fn the_light_hands_over_through_a_dark_point_at_the_settings_ramps() {
    let stated = |name: &str, default: f32| match name {
        "misc.SunFade.ThresholdLength" => 10.0,
        "misc.MoonFade.ThresholdLength" => 5.0,
        _ => default,
    };
    let tunables = ExposureSettings::of(&stated).bodies;
    assert_eq!(
        (tunables.sun_zenith_ramp, tunables.moon_zenith_ramp),
        (10.0, 5.0)
    );
    let (k, moon) = (keyframe(&[]), at_zenith(50.0));
    let dusk = bodies(&k, at_zenith(95.0), moon, &tunables);
    assert!(dusk.sun_leads && (dusk.fade - 0.5).abs() < 1e-5, "{dusk:?}");
    let dark = bodies(&k, at_zenith(100.0), moon, &tunables);
    assert_eq!((dark.fade, dark.towards), (0.0, [0.0, 0.0, 1.0]));
    let rising = bodies(&k, at_zenith(105.0), moon, &tunables);
    assert!(
        !rising.sun_leads && (rising.moon_fade - 0.5).abs() < 1e-5,
        "{rising:?}"
    );
    assert_eq!(rising.body, moon);
}
