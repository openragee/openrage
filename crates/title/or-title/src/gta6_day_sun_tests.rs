use super::*;

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

fn assert_close64(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

fn assert_close3(actual: Vec3, expected: Vec3, tolerance: f32) {
    assert!(
        (actual - expected).abs().max_element() <= tolerance,
        "{actual:?} is not within {tolerance} of {expected:?}"
    );
}

/// The variable table's defaults for v0..v28.
fn variables() -> LightVariables {
    LightVariables {
        day_colour: Vec3::new(0.89, 0.675, 0.392),
        day_intensity: 14.75,
        sun_multiplier: 1.0,
        moon_multiplier: 1.0,
        third_colour: Vec3::new(0.89, 0.675, 0.392),
        third_intensity: 14.75,
        pull_amount: 0.0,
        disc_pull_scale: 1.0,
        sun_pull: 0.0,
        moon_pull: 0.0,
        sun_target: Vec3::new(0.0, -1.0, 0.0),
        moon_target: Vec3::new(0.0, -1.0, 0.0),
        sun_zenith_threshold: -1.0,
        moon_zenith_threshold: -1.0,
        leader_override: -10.0,
    }
}

fn tunables() -> SunTunables {
    // x1 is not in the image; 11000 is a test value.
    SunTunables::image_defaults(11000.0)
}

/// Observer at y 0 (latitude 25.8) and the west edge (longitude -81.8).
fn camera() -> Vec3 {
    Vec3::new(-11000.0, 0.0, 0.0)
}

#[test]
fn clock_day_fraction_counts_every_field() {
    // 18 + 30/60 + 36/3600 = 18.51 h, / 24 = 0.77125.
    assert_close64(clock_day_fraction(18, 30, 36, 0), 0.77125, 1e-12);
    // 900 ms = 0.00025 h, / 24 = 1.0416667e-5.
    assert_close64(clock_day_fraction(0, 0, 0, 900), 1.0416667e-5, 1e-11);
}

#[test]
fn season_is_zero_on_day_80_and_tilted_at_the_solstice() {
    // (80 + 285) mod 365 = 0: angle 0, A = (1, 0, 0).
    let equinox = season(80);
    assert_close64(equinox[0], 1.0, 1e-12);
    assert_close64(equinox[1], 0.0, 1e-12);
    assert_close64(equinox[2], 0.0, 1e-12);
    // (172 + 285) mod 365 = 92: angle 2 pi 92/365 = 1.583707 = pi/2 + 0.012911, sin = 0.999917;
    // z = 0.999917 * sin(0.409105) = 0.999917 * 0.397788 = 0.397755; x = cos = -0.012911.
    let solstice = season(172);
    assert_close64(solstice[2], 0.397755, 1e-5);
    assert_close64(solstice[0], -0.012911, 1e-5);
}

#[test]
fn noon_moves_an_hour_on_daylight_saving_days() {
    let t = tunables();
    // Day 100 is in 79..297: (780 + 10)/1440 = 0.548611.
    assert_close64(solar_noon(100, 10.0, &t), 0.548611, 1e-6);
    // Day 79 is the first daylight-saving day.
    assert_close64(solar_noon(79, 10.0, &t), 0.548611, 1e-6);
    // Days 298 and 300 are standard time: (720 + 10)/1440 = 0.506944.
    assert_close64(solar_noon(298, 10.0, &t), 0.506944, 1e-6);
    assert_close64(solar_noon(300, 10.0, &t), 0.506944, 1e-6);
}

#[test]
fn sun_day_reads_the_noon_offset_at_the_clamped_day() {
    let mut table = [0u8; 365];
    table[1] = 10;
    table[80] = 30;
    // Day 80 reads byte 80: (780 + 30)/1440 = 0.5625.
    assert_close64(
        sun_day(0.5, 80, &table, camera(), &tunables()).noon,
        0.5625,
        1e-12,
    );
    // Day -5 clamps to 1, standard time: (720 + 10)/1440 = 0.506944.
    assert_close64(
        sun_day(0.5, -5, &table, camera(), &tunables()).noon,
        0.506944,
        1e-6,
    );
    // Day 400 clamps to 365, past the table: 720/1440 = 0.5.
    assert_close64(
        sun_day(0.5, 400, &table, camera(), &tunables()).noon,
        0.5,
        1e-12,
    );
}

#[test]
fn the_equinox_sun_crosses_the_meridian_at_noon_and_rises_due_east() {
    let noon = solar_noon(80, 0.0, &tunables());
    let geometry = SunGeometry::new(80, noon, camera(), &tunables());
    // Latitude 24 + 3.6 * 0.5 = 25.8: noon sun south at elevation 64.2,
    // (0, -sin 25.8, cos 25.8) = (0, -0.435231, 0.900319).
    assert_close3(
        geometry.direction(noon),
        Vec3::new(0.0, -0.435231, 0.900319),
        1e-5,
    );
    // Six hours earlier the hour angle is a quarter turn less: due east on the horizon.
    assert_close3(
        geometry.direction(noon - 0.25),
        Vec3::new(1.0, 0.0, 0.0),
        1e-5,
    );
}

#[test]
fn the_solstice_noon_sun_uses_the_season_height() {
    // A = (-0.012910, -0.917401, 0.397755), |A.xy| = 0.917492, colatitude 64.2:
    // north = -0.435231 * 0.917492 + 0.900319 * 0.397755 = -0.041215,
    // up = 0.900319 * 0.917492 + 0.435231 * 0.397755 = 0.999151.
    let noon = solar_noon(172, 0.0, &tunables());
    let sun = SunGeometry::new(172, noon, camera(), &tunables()).direction(noon);
    assert_close3(sun, Vec3::new(0.0, -0.041215, 0.999151), 1e-4);
}

#[test]
fn the_observer_latitude_is_clamped_to_the_box() {
    let noon = solar_noon(80, 0.0, &tunables());
    // y far north clamps to latitude 27.6: noon sun (0, -sin 27.6, cos 27.6) = (0, -0.463296, 0.886204).
    let north = SunGeometry::new(80, noon, Vec3::new(-11000.0, 1.0e6, 0.0), &tunables());
    assert_close3(
        north.direction(noon),
        Vec3::new(0.0, -0.463296, 0.886204),
        1e-5,
    );
    // y far south clamps to latitude 24.0: noon sun (0, -sin 24, cos 24) = (0, -0.406737, 0.913545).
    let south = SunGeometry::new(80, noon, Vec3::new(-11000.0, -1.0e6, 0.0), &tunables());
    assert_close3(
        south.direction(noon),
        Vec3::new(0.0, -0.406737, 0.913545),
        1e-5,
    );
}

#[test]
fn sunrise_and_sunset_are_the_first_and_last_horizon_steps() {
    // Equinox, noon 780/1440 = 52/96: the sun is on the horizon exactly at steps 28 and 76.
    // One step either side it is 3.75 degrees of hour angle away: angle
    // atan2(0.900319 * sin 3.75deg, cos 3.75deg) = 0.0589 > 0.05, so not a horizon step.
    let noon = solar_noon(80, 0.0, &tunables());
    let geometry = SunGeometry::new(80, noon, camera(), &tunables());
    let (sunrise, sunset) = geometry.horizon_crossings();
    assert_close64(sunrise, 28.0 / 96.0, 1e-12);
    assert_close64(sunset, 76.0 / 96.0, 1e-12);
    // Noon 787.5/1440 = 52.5/96, horizon at steps 28.5 and 76.5. At steps 28 and 29 the angle
    // is -/+ atan(0.900319 * tan 1.875deg) = -/+ 0.0295, both horizon steps. Step 76 gives
    // pi - 0.0295, a horizon step. Step 77 gives atan2(-, -) = -3.1121, not below -3.14, so it
    // stays unwrapped and is not one. Steps 27 and 30 are at -/+ 0.0884. The horizon steps are
    // 28, 29 and 76: a sunset taken from the second step would be 29/96.
    let geometry = SunGeometry::new(80, solar_noon(80, 7.5, &tunables()), camera(), &tunables());
    let (sunrise, sunset) = geometry.horizon_crossings();
    assert_close64(sunrise, 28.0 / 96.0, 1e-12);
    assert_close64(sunset, 76.0 / 96.0, 1e-12);
}

#[test]
fn the_warp_eases_both_ends_of_each_half() {
    // Day [0.25, 0.75], half 0.25; d 0.3: 0.25 + 0.25 * 0.2^1.25 = 0.25 + 0.25 * 0.133748 = 0.283437.
    assert_close64(warp_day_fraction(0.3, 0.25, 0.75), 0.283437, 1e-6);
    // Night [0.75, 1.25]; d 0.9: 0.75 + 0.25 * 0.6^1.25 = 0.75 + 0.25 * 0.528067 = 0.882017.
    assert_close64(warp_day_fraction(0.9, 0.25, 0.75), 0.882017, 1e-6);
    // Night [-0.25, 0.25]; d 0.1 is past the midpoint: 0.25 - 0.25 * 0.6^1.25 = 0.117983.
    assert_close64(warp_day_fraction(0.1, 0.25, 0.75), 0.117983, 1e-6);
    // Night [-0.1, 0.2], half 0.15; d 0: -0.1 + 0.15 * (2/3)^1.25 = -0.1 + 0.15 * 0.602401
    // = -0.0096398, wrapped to 0.9903602.
    assert_close64(warp_day_fraction(0.0, 0.2, 0.9), 0.9903602, 1e-6);
}

#[test]
fn the_warp_wraps_a_day_fraction_outside_the_day_first() {
    // -0.3 is 0.7 and 1.4 is 0.4: without the wrap both would be powers of negative bases, NaN.
    assert_close64(
        warp_day_fraction(-0.3, 0.25, 0.75),
        warp_day_fraction(0.7, 0.25, 0.75),
        1e-12,
    );
    assert_close64(
        warp_day_fraction(1.4, 0.25, 0.75),
        warp_day_fraction(0.4, 0.25, 0.75),
        1e-12,
    );
}

#[test]
fn sun_day_warps_the_clock_before_the_direction() {
    let noon = 780.0 / 1440.0;
    // At noon the warp holds the day's midpoint (28 + 76)/2/96 = noon: the meridian sun.
    let at_noon = sun_day(noon, 80, &[], camera(), &tunables());
    assert_close64(at_noon.warped_day_fraction, noon, 1e-9);
    assert_close64(at_noon.day_phase, 0.5, 1e-9);
    assert_close3(at_noon.direction, Vec3::new(0.0, -0.435231, 0.900319), 1e-5);
    // d 0.4: 28/96 + 0.25 * (0.108333/0.25)^1.25 = 0.291667 + 0.25 * 0.351583 = 0.379563.
    let morning = sun_day(0.4, 80, &[], camera(), &tunables());
    assert_close64(morning.warped_day_fraction, 0.379563, 1e-5);
    let unwarped = SunGeometry::new(80, noon, camera(), &tunables()).direction(0.379563);
    assert_close3(morning.direction, unwarped, 1e-4);
}

#[test]
fn the_day_phase_is_a_quarter_per_piece() {
    // Sunrise 0.2, noon 0.5, sunset 0.9.
    // 0.25 * 0.1/0.2 = 0.125; 0.25 + 0.25 * 0.15/0.3 = 0.375.
    assert_close64(day_phase(0.1, 0.2, 0.5, 0.9), 0.125, 1e-12);
    assert_close64(day_phase(0.35, 0.2, 0.5, 0.9), 0.375, 1e-12);
    // 0.5 + 0.25 * 0.2/0.4 = 0.625; 0.75 + 0.25 * 0.05/0.1 = 0.875.
    assert_close64(day_phase(0.7, 0.2, 0.5, 0.9), 0.625, 1e-12);
    assert_close64(day_phase(0.95, 0.2, 0.5, 0.9), 0.875, 1e-12);
    // The boundaries: 0.25, 0.5 and 0.75.
    assert_close64(day_phase(0.2, 0.2, 0.5, 0.9), 0.25, 1e-12);
    assert_close64(day_phase(0.5, 0.2, 0.5, 0.9), 0.5, 1e-12);
    assert_close64(day_phase(0.9, 0.2, 0.5, 0.9), 0.75, 1e-12);
}

#[test]
fn the_sample_angle_runs_on_through_the_night() {
    // Meridian by day: atan2(0.9, 0) = pi/2; by night: atan2(-0.9, 0) = -pi/2.
    let quarter = std::f32::consts::FRAC_PI_2;
    assert_close(sample_angle(Vec3::new(0.0, -0.4, 0.9)), quarter, 1e-5);
    assert_close(sample_angle(Vec3::new(0.0, 0.4, -0.9)), -quarter, 1e-5);
    // atan2(-0.001, -1) = -3.140593 < -3.14: + 6.28 = 3.139407.
    assert_close(sample_angle(Vec3::new(-1.0, 0.0, -0.001)), 3.139407, 1e-5);
    // atan2(-0.002, -1) = -3.139593 is not below -3.14 and stays.
    assert_close(sample_angle(Vec3::new(-1.0, 0.0, -0.002)), -3.139593, 1e-5);
}

#[test]
fn the_moon_cycle_is_the_fraction_of_scaled_days() {
    // (3 + 0.25) * 1 = 3.25 -> 0.25; (3 + 0.25) * 2 = 6.5 -> 0.5.
    assert_close64(moon_cycle(3, 0.25, 1.0), 0.25, 1e-12);
    assert_close64(moon_cycle(3, 0.25, 2.0), 0.5, 1e-12);
}

#[test]
fn the_moon_turns_once_a_day_and_never_sets() {
    let t = MoonTunables::image_defaults();
    // Tilt 362 deg = 2 deg: sin 0.0348995, cos 0.9993908; length sqrt(0.8^2 + 1) = 1.2806248.
    // Midnight, turn -pi: (0, 0.9993908 - 0.8 * 0.0348995, 0.8 * 0.9993908 + 0.0348995)
    // = (0, 0.9714712, 0.8344121) / 1.2806248 = (0, 0.758592, 0.651566).
    assert_close3(
        moon_direction(0.0, &t),
        Vec3::new(0.0, 0.758592, 0.651566),
        1e-5,
    );
    // 06:00, turn -pi/2: (-1, -0.0279196, 0.7995126) / 1.2806248 = (-0.780869, -0.021802, 0.624314).
    assert_close3(
        moon_direction(0.25, &t),
        Vec3::new(-0.780869, -0.021802, 0.624314),
        1e-5,
    );
    // Noon, turn 0: (0, -1.0273104, 0.7646131) / 1.2806248 = (0, -0.802195, 0.597062).
    assert_close3(
        moon_direction(0.5, &t),
        Vec3::new(0.0, -0.802195, 0.597062),
        1e-5,
    );
    // z = (0.7995126 - cos(turn) * 0.0348995) / 1.2806248 stays in [0.597062, 0.651566].
    for hour in 0..24 {
        let z = moon_direction(hour as f64 / 24.0, &t).z;
        assert!((0.597..0.6516).contains(&z), "hour {hour}: z {z}");
    }
}

#[test]
fn the_moon_height_swings_with_the_cycle() {
    let mut t = MoonTunables::image_defaults();
    t.height_swing_rate = 1.0;
    // Cycle 0.25: height 0.8 + 0.1 * sin(pi/2) = 0.9; (-1, -0.9 * 0.0348995, 0.9 * 0.9993908)
    // = (-1, -0.0314096, 0.8994517), length sqrt(1.81) = 1.3453624: (-0.743294, -0.023347, 0.668557).
    assert_close3(
        moon_direction(0.25, &t),
        Vec3::new(-0.743294, -0.023347, 0.668557),
        1e-5,
    );
}

#[test]
fn zenith_is_in_degrees_and_clamped() {
    // acos(0.5) = 1.0471976 rad = 60 degrees.
    assert_close(zenith_degrees(0.5), 60.0, 1e-3);
    assert_close(zenith_degrees(2.0), 0.0, 1e-3);
    assert_close(zenith_degrees(-2.0), 180.0, 1e-3);
}

#[test]
fn the_hand_over_ramps_linearly_with_a_dark_point_at_minus_ten() {
    let v = variables();
    let t = LightTunables::image_defaults();
    // Moon zenith 50: (50 - 100)/1 = -50 clamps to -1, so the moon takes any weight the sun leaves.
    let at = |elevation: f32| body_weights(90.0 - elevation, 50.0, &v, &t);
    // Elevation -8: zenith 98, (98 - 100)/1 = -2 -> -1: sun 1.
    assert_close(at(-8.0).sun, 1.0, 1e-5);
    // -9.5: zenith 99.5 -> -0.5: sun 0.5.
    let dusk = at(-9.5);
    assert_close(dusk.sun, 0.5, 1e-5);
    assert_close(dusk.moon, 0.0, 1e-6);
    // -10: zenith 100 -> 0: the moon gets min(0, 1) * 1 = 0, every weight 0.
    assert_eq!(
        at(-10.0),
        Weights {
            sun: 0.0,
            moon: 0.0,
            third: 0.0
        }
    );
    // -10.5: 0.5 -> moon 0.5; -12: 2 -> min(2, 1) = moon 1.
    let night = at(-10.5);
    assert_close(night.moon, 0.5, 1e-5);
    assert_close(night.sun, 0.0, 1e-6);
    assert_close(at(-12.0).moon, 1.0, 1e-6);
}

#[test]
fn the_override_and_thresholds_replace_the_automatic_weights() {
    let t = LightTunables::image_defaults();
    let mut v = variables();
    v.leader_override = 0.4;
    assert_close(body_weights(170.0, 50.0, &v, &t).sun, 0.4, 1e-6);
    v.leader_override = 2.0;
    assert_close(body_weights(170.0, 50.0, &v, &t).sun, 1.0, 1e-6);
    v.leader_override = -3.0;
    let moon = body_weights(10.0, 50.0, &v, &t);
    assert_close(moon.moon, 1.0, 1e-6);
    assert_close(moon.sun, 0.0, 1e-6);
    v.leader_override = 0.0;
    assert_eq!(
        body_weights(10.0, 50.0, &v, &t),
        Weights {
            sun: 0.0,
            moon: 0.0,
            third: 0.0
        }
    );
    // v26 = 80 replaces 100: (79.5 - 80)/1 = -0.5 -> sun 0.5 (the default would give 1).
    let mut v = variables();
    v.sun_zenith_threshold = 80.0;
    assert_close(body_weights(79.5, 50.0, &v, &t).sun, 0.5, 1e-5);
}

#[test]
fn the_third_body_takes_weight_only_when_both_bodies_are_past_their_thresholds() {
    let v = variables();
    let mut t = LightTunables::image_defaults();
    t.moon_zenith_start = 40.0;
    // Sun (100.5 - 100)/1 = 0.5; moon (50.5 - 40)/1 = 10.5 -> 1: third = min(0.5, 1) * min(10.5, 1) = 0.5.
    let w = body_weights(100.5, 50.5, &v, &t);
    assert_close(w.third, 0.5, 1e-5);
    assert_close(w.moon, 0.0, 1e-6);
    assert_eq!(w.leader(), Leader::Neither);
}

#[test]
fn the_sun_counts_a_hundred_times_in_the_direction() {
    let v = variables();
    let w = Weights {
        sun: 0.01,
        moon: 1.0,
        third: 0.0,
    };
    // Sum 100 * 0.01 + 1 = 2, shares 0.5 and 0.5: (0.5, 0, 0.5) normalised = (1/sqrt 2, 0, 1/sqrt 2).
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let d = light_direction(Vec3::X, Vec3::Z, Vec3::Y, &w, &v);
    assert_close3(d, Vec3::new(half, 0.0, half), 1e-5);
}

#[test]
fn the_light_direction_is_floored_at_z_0_15() {
    let v = variables();
    let none = Weights {
        sun: 0.0,
        moon: 0.0,
        third: 0.0,
    };
    // No weight: direction 0, floored to (0, 0, 0.15), normalised to (0, 0, 1).
    assert_close3(
        light_direction(Vec3::X, Vec3::Z, Vec3::Y, &none, &v),
        Vec3::Z,
        1e-6,
    );
    // Sum 1e-8 <= FLT_EPSILON: no direction, so (0, 0, 1). The moon's share of 1 would give
    // (1, 0, 0.15) / 1.0111874 = (0.988936, 0, 0.148340).
    let tiny = Weights {
        sun: 0.0,
        moon: 1e-8,
        third: 0.0,
    };
    assert_close3(
        light_direction(Vec3::Z, Vec3::X, Vec3::Y, &tiny, &v),
        Vec3::Z,
        1e-6,
    );
    // Sun (1, 0, 0.05)/1.0012492 = (0.998752, 0, 0.0499376); z -> 0.15, length 1.0099535:
    // (0.988909, 0, 0.148522).
    let sun = Vec3::new(1.0, 0.0, 0.05).normalize();
    let w = Weights {
        sun: 1.0,
        moon: 0.0,
        third: 0.0,
    };
    assert_close3(
        light_direction(sun, Vec3::Z, Vec3::Y, &w, &v),
        Vec3::new(0.988909, 0.0, 0.148522),
        1e-4,
    );
}

#[test]
fn the_light_pull_ignores_the_disc_scale() {
    let mut v = variables();
    v.sun_pull = 1.0;
    v.disc_pull_scale = 0.5;
    let w = Weights {
        sun: 1.0,
        moon: 0.0,
        third: 0.0,
    };
    // Light: pull max(1, 0) = 1 goes all the way to the target (0, -1, 0); floored:
    // (0, -1, 0.15) / 1.011187 = (0, -0.988936, 0.148340).
    assert_close3(
        light_direction(Vec3::X, Vec3::Z, Vec3::Y, &w, &v),
        Vec3::new(0.0, -0.988936, 0.148340),
        1e-5,
    );
    // Disc: pull 0.5 * 1: (1, 0, 0) + 0.5 * ((0, -1, 0) - (1, 0, 0)) = (0.5, -0.5, 0) -> (1, -1, 0)/sqrt 2.
    // The moon's pull max(0, 0) = 0 leaves it where it is.
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let (sun_disc, moon_disc) = disc_directions(Vec3::X, Vec3::Z, &v);
    assert_close3(sun_disc, Vec3::new(half, -half, 0.0), 1e-5);
    assert_close3(moon_disc, Vec3::Z, 1e-6);
}

#[test]
fn the_moon_phase_dims_only_with_a_floor_below_one() {
    let v = variables();
    let moon = Weights {
        sun: 0.0,
        moon: 1.0,
        third: 0.0,
    };
    // Defaults (phase 0.5, floor 1): 1 * (1 - 1 * 0 * 0) = 1.
    assert_close(
        intensity_weight(&moon, 0.3, &v, &LightTunables::image_defaults()),
        1.0,
        1e-6,
    );
    // Phase 0.25, floor 0.2: 1 - 1 * 0.8 * |0.5 - 1| = 0.6.
    let mut t = LightTunables::image_defaults();
    t.moon_phase = 0.25;
    t.moon_floor = 0.2;
    assert_close(intensity_weight(&moon, 0.3, &v, &t), 0.6, 1e-6);
    // Negative phase takes cycle / 26 = 0.5/26 = 0.0192308; floor 0: 1 - |0.0384615 - 1| = 0.0384615.
    t.moon_phase = -1.0;
    t.moon_floor = 0.0;
    assert_close(intensity_weight(&moon, 0.5, &v, &t), 0.0384615, 1e-6);
}

#[test]
fn the_moonlight_is_the_day_set_times_the_moon_multiplier() {
    let mut v = variables();
    v.moon_multiplier = 0.5;
    v.third_colour = Vec3::new(0.1, 0.2, 0.9);
    let t = LightTunables::image_defaults();
    let moon = moon_direction(0.0, &MoonTunables::image_defaults());
    // Sun z -0.5: zenith 120, (120 - 100)/1 -> 1; moon zenith about 49.3 -> -1: moon weight 1.
    let sun = Vec3::new(0.866025, 0.0, -0.5);
    let light = directional_light(sun, moon, Vec3::Y, 0.0, &v, &t);
    assert_close(light.weights.moon, 1.0, 1e-6);
    assert_close3(light.colour, v.day_colour, 0.0);
    // 14.75 * 1 * 0.5 = 7.375.
    assert_close(light.intensity, 7.375, 1e-5);
    // Only the moon has weight and nothing pulls: the light comes from the moon (z 0.65 > 0.15).
    assert_close3(light.direction, moon, 1e-5);
}

#[test]
fn the_sun_takes_its_multiplier_and_the_third_body_its_own_set() {
    let mut v = variables();
    v.sun_multiplier = 2.0;
    v.moon_multiplier = 0.5;
    v.third_colour = Vec3::new(0.1, 0.2, 0.9);
    v.third_intensity = 3.0;
    let moon = moon_direction(0.0, &MoonTunables::image_defaults());
    // Sun (0.6, 0, 0.8): zenith 36.87 -> sun 1; 14.75 * 1 * 2 = 29.5.
    let day = directional_light(
        Vec3::new(0.6, 0.0, 0.8),
        moon,
        Vec3::Y,
        0.0,
        &v,
        &LightTunables::image_defaults(),
    );
    assert_close(day.intensity, 29.5, 1e-4);
    // Moon start 40: moon (49.3 - 40) -> 1, sun (120 - 100) -> 1: third 1, which takes no
    // multiplier: colour v9..v11, 3 * 1 = 3.
    let mut t = LightTunables::image_defaults();
    t.moon_zenith_start = 40.0;
    let third = directional_light(Vec3::new(0.866025, 0.0, -0.5), moon, Vec3::Y, 0.0, &v, &t);
    assert_close(third.weights.third, 1.0, 1e-6);
    assert_close3(third.colour, v.third_colour, 0.0);
    assert_close(third.intensity, 3.0, 1e-5);
}

#[test]
fn a_dark_colour_or_a_tiny_intensity_switches_the_light_off() {
    let moon = moon_direction(0.0, &MoonTunables::image_defaults());
    let sun = Vec3::new(0.6, 0.0, 0.8);
    let t = LightTunables::image_defaults();
    let mut v = variables();
    v.day_colour = Vec3::ZERO;
    assert_eq!(
        directional_light(sun, moon, Vec3::Y, 0.0, &v, &t).intensity,
        0.0
    );
    let mut v = variables();
    v.day_intensity = 1e-7;
    assert_eq!(
        directional_light(sun, moon, Vec3::Y, 0.0, &v, &t).intensity,
        0.0
    );
}

#[test]
fn light_variables_come_from_their_frame_indices() {
    let mut frame = [0.0f32; crate::gta6_day_frame::FRAME_FLOATS];
    for (index, value) in frame.iter_mut().enumerate() {
        *value = index as f32;
    }
    let v = LightVariables::from_frame(&frame);
    assert_eq!(v.day_colour, Vec3::new(0.0, 1.0, 2.0));
    assert_eq!(
        (v.day_intensity, v.sun_multiplier, v.moon_multiplier),
        (3.0, 4.0, 5.0)
    );
    assert_eq!(v.third_colour, Vec3::new(9.0, 10.0, 11.0));
    assert_eq!(
        (v.third_intensity, v.pull_amount, v.disc_pull_scale),
        (12.0, 13.0, 14.0)
    );
    assert_eq!((v.sun_pull, v.moon_pull), (18.0, 19.0));
    assert_eq!(v.sun_target, Vec3::new(20.0, 21.0, 22.0));
    assert_eq!(v.moon_target, Vec3::new(23.0, 24.0, 25.0));
    assert_eq!(
        (
            v.sun_zenith_threshold,
            v.moon_zenith_threshold,
            v.leader_override
        ),
        (26.0, 27.0, 28.0)
    );
}
