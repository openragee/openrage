use super::*;

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
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

/// The variable table's defaults for v1103..v1121.
fn variables() -> WindVariables {
    WindVariables {
        heading_weight: 0.0,
        heading_degrees: 0.0,
        gust_strength_kmh: [10.0, 30.0],
        gust_duration: [3.0, 20.0],
        gust_interval: [0.0, 0.0],
        speed_a_kmh: [0.0, 0.0],
        speed_b_kmh: [0.0, 0.0],
        speed_mult: 1.0,
        falloff_height: 0.0,
    }
}

fn randoms(value: f32) -> GustRandoms {
    GustRandoms {
        interval: value,
        strength: value,
        duration: value,
        angle: value,
        x: value,
        y: value,
    }
}

/// A gust at the origin blowing east, 10 s long with 2 s ramps (20 per cent each).
fn gust(life: f32, speed: f32) -> Gust {
    Gust {
        position: Vec3::ZERO,
        weight: 0.0,
        direction: Vec3::X,
        speed,
        strength: 8.0,
        radius: 800.0,
        duration: 10.0,
        ramp_in: 2.0,
        ramp_out: 2.0,
        life,
    }
}

#[test]
fn variables_come_from_their_frame_indices() {
    let frame: Vec<f32> = (0..1408).map(|index| index as f32).collect();
    let v = WindVariables::from_frame(&frame).unwrap();
    assert_eq!((v.heading_weight, v.heading_degrees), (1103.0, 1104.0));
    assert_eq!(v.gust_strength_kmh, [1107.0, 1108.0]);
    assert_eq!(v.gust_duration, [1109.0, 1110.0]);
    assert_eq!(v.gust_interval, [1111.0, 1112.0]);
    assert_eq!(v.speed_a_kmh, [1118.0, 1119.0]);
    assert_eq!(v.speed_b_kmh, [1120.0, 1121.0]);
    assert_eq!((v.speed_mult, v.falloff_height), (1105.0, 1122.0));
    // Index 1122 is the last read: a frame of 1122 floats lacks it.
    assert_eq!(WindVariables::from_frame(&frame[..1122]), None);
}

#[test]
fn the_injected_speed_goes_from_the_first_timecycle_speed_to_the_second_up_the_falloff_height() {
    let mut state = WindState::calm();
    state.timecycle_speeds = [2.0, 6.0];
    let v = WindVariables {
        speed_mult: 0.5,
        falloff_height: 100.0,
        ..variables()
    };
    assert_close(state.injected_speed(&v, -5.0), 1.0, 1e-6);
    assert_close(state.injected_speed(&v, 25.0), 1.5, 1e-6);
    assert_close(state.injected_speed(&v, 400.0), 3.0, 1e-6);
    // No falloff height: the first speed at every height.
    let flat = WindVariables {
        falloff_height: 0.0,
        ..v
    };
    assert_close(state.injected_speed(&flat, 400.0), 1.0, 1e-6);
}

#[test]
fn the_direction_points_away_from_the_heading() {
    // Weight 0: a = 90 degrees, dir = (-sin 90, -cos 90, 0) = (-1, 0, 0): a wind from the east blows west.
    let a = heading(90.0, &variables());
    assert_close(a, std::f32::consts::FRAC_PI_2, 1e-6);
    assert_close3(direction(a), Vec3::new(-1.0, 0.0, 0.0), 1e-6);
    // a = 0: (0, -1, 0).
    assert_close3(direction(0.0), Vec3::new(0.0, -1.0, 0.0), 1e-6);
}

#[test]
fn the_heading_moves_toward_the_timecycle_the_short_way() {
    let mut v = variables();
    v.heading_weight = 0.5;
    v.heading_degrees = 10.0;
    // 350 -> 10: arc = wrap(10 - 350) = +20 degrees = 0.349066; 6.108652 + 0.5 * 0.349066 = 6.283185,
    // wrapped: (6.283185 + pi) % 2 pi - pi = about 0. Without the outer wrap a = 6.283; the long arc gives -pi.
    let a = heading(350.0, &v);
    assert_close(a, 0.0, 1e-5);
    assert_close3(direction(a), Vec3::new(0.0, -1.0, 0.0), 1e-5);
}

#[test]
fn the_heading_weight_is_capped_at_one() {
    let mut v = variables();
    v.heading_weight = 3.0;
    v.heading_degrees = 90.0;
    // min(1, 3) = 1: a = 0 + 90 = 90 degrees, dir (-1, 0, 0); a weight of 3 would give 270, dir (1, 0, 0).
    assert_close3(direction(heading(0.0, &v)), Vec3::new(-1.0, 0.0, 0.0), 1e-5);
    // A negative weight leaves the weather's heading: 30 degrees = pi/6 rad.
    v.heading_weight = -1.0;
    assert_close(heading(30.0, &v), std::f32::consts::FRAC_PI_6, 1e-5);
}

#[test]
fn the_wrap_follows_the_sign_of_the_value() {
    // 810 with R 800: (810 + 800) % 1600 - 800 = 10 - 800 = -790.
    assert_close(wrap_half_period(810.0, 800.0), -790.0, 1e-3);
    // -810: (-1610) % 1600 + 800 = -10 + 800 = 790.
    assert_close(wrap_half_period(-810.0, 800.0), 790.0, 1e-3);
    // Inside the range it stays: 300.
    assert_close(wrap_half_period(300.0, 800.0), 300.0, 1e-3);
    assert_close(wrap_half_period(300.0, 0.0), 300.0, 0.0);
    // -800: (-1600) % 1600 + 800 = -0 + 800 = 800; a Euclidean wrap would give 0 - 800 = -800.
    assert_close(wrap_half_period(-800.0, 800.0), 800.0, 0.0);
}

#[test]
fn a_half_turn_arc_wraps_to_minus_pi() {
    // Pins the formula's choice at d = pi: (pi + pi) % 2 pi - pi = 0 - pi = -pi. The notes' stated interval
    // (-pi, pi] would give +pi; which the build does is unsettled.
    assert_close(wrap_half_period(PI, PI), -PI, 0.0);
}

#[test]
fn the_base_speed_lerps_by_the_walk() {
    let weather = WeatherWind {
        speed_low: 2.0,
        speed_high: 6.0,
        heading_degrees: 0.0,
    };
    // 2 + (6 - 2) * 0.25 = 3.
    assert_close(base_speed(&weather, 0.25), 3.0, 1e-6);
    let mut state = WindState::calm();
    state.apply_weather(&weather, &variables(), 0.25);
    assert_close(state.base_speed, 3.0, 1e-6);
    assert_close3(state.direction, Vec3::new(0.0, -1.0, 0.0), 1e-6);
}

#[test]
fn approach_moves_at_most_one_step() {
    // 0 toward 5 by 2: 2; 4.5 toward 5 by 2: 5; 5 toward 0 by 2: 3.
    assert_close(approach(0.0, 5.0, 2.0), 2.0, 0.0);
    assert_close(approach(4.5, 5.0, 2.0), 5.0, 0.0);
    assert_close(approach(5.0, 0.0, 2.0), 3.0, 0.0);
    // A negative or NaN step holds the value.
    assert_close(approach(5.0, 0.0, -1.0), 5.0, 0.0);
    assert_close(approach(5.0, 0.0, f32::NAN), 5.0, 0.0);
}

#[test]
fn a_unit_random_takes_the_low_23_bits() {
    // 0x00c00000 & 0x7fffff = 0x400000 = 2^22: 2^22 / 2^23 = 0.5.
    assert_close(unit_random(0x00c0_0000), 0.5, 0.0);
    // u64::MAX & 0x7fffff = 0x7fffff; 0x7fffff / 2^23 = 1 - 2^-23 = 0.99999988: the carry half is dropped.
    assert_close(unit_random(u64::MAX), 1.0 - RANDOM_SCALE, 0.0);
}

#[test]
fn the_timecycle_speeds_are_km_h_eased_at_the_rate() {
    let mut v = variables();
    v.speed_a_kmh = [36.0, 72.0];
    v.speed_b_kmh = [7.2, 7.2];
    let config = WindConfig::image_defaults();
    // Targets lerp(36, 72, 0.5) = 54 km/h = 15 m/s and 7.2 km/h = 2 m/s; step 2 * 0.5 = 1.
    let eased = ease_timecycle_speeds([0.0, 0.0], &v, 0.5, &config, 0.5);
    assert_close(eased[0], 1.0, 1e-6);
    assert_close(eased[1], 1.0, 1e-6);
    // Step 2 * 10 = 20 reaches both: 15 and 2 (54 unconverted would reach 20).
    let settled = ease_timecycle_speeds([0.0, 0.0], &v, 0.5, &config, 10.0);
    assert_close(settled[0], 15.0, 1e-4);
    assert_close(settled[1], 2.0, 1e-5);
}

#[test]
fn the_max_speed_is_the_larger_volume_in_m_s() {
    // 74 km/h / 3.6 = 20.555556 m/s.
    assert_close(
        max_speed([DEFAULT_VOLUME_MAX_SPEED_KMH, 36.0]),
        20.555_556,
        1e-4,
    );
    assert_close(max_speed([36.0, 74.0]), 20.555_556, 1e-4);
}

#[test]
fn a_scroll_offset_moves_along_the_wind_and_wraps_by_its_sign() {
    // (0, -3998, 0) + (0, -1, 0) * 1 * 10 = -4008; -4008 % 4000 = -8 (a Euclidean wrap would give 3992).
    let wrapped = scroll(Vec3::new(0.0, -3998.0, 0.0), -Vec3::Y, 1.0, 10.0, 4000.0);
    assert_close3(wrapped, Vec3::new(0.0, -8.0, 0.0), 1e-3);
    // A period of 0 leaves it unwrapped: -4008.
    let unwrapped = scroll(Vec3::new(0.0, -3998.0, 0.0), -Vec3::Y, 1.0, 10.0, 0.0);
    assert_close3(unwrapped, Vec3::new(0.0, -4008.0, 0.0), 1e-3);
}

#[test]
fn a_step_moves_both_scroll_offsets_at_their_rates() {
    let config = WindConfig::image_defaults();
    let mut state = WindState::calm();
    state.direction = Vec3::new(0.0, -1.0, 0.0);
    state.step(0.0, 0.5, &variables(), 0.0, &randoms(0.5), &config);
    // A: -1 * 0.5 * 10 = -5; B: -1 * 0.5 * 200 = -100.
    assert_close3(state.scroll_offsets[0], Vec3::new(0.0, -5.0, 0.0), 1e-5);
    assert_close3(state.scroll_offsets[1], Vec3::new(0.0, -100.0, 0.0), 1e-4);
}

#[test]
fn a_gust_spawns_from_its_variables() {
    let r = GustRandoms {
        interval: 0.0,
        strength: 0.5,
        duration: 1.0,
        angle: 0.5,
        x: 0.75,
        y: 0.25,
    };
    let mut config = WindConfig::image_defaults();
    config.gust_ramp_out_percent = 30.0;
    let g = spawn_gust(Vec3::X, 2.0, &variables(), &r, &config).unwrap();
    // Strength lerp(10, 30, 0.5) = 20 km/h / 3.6 = 5.555556 m/s; speed 5.555556 + 2 = 7.555556.
    assert_close(g.strength, 5.555_556, 1e-5);
    assert_close(g.speed, 7.555_556, 1e-5);
    // Duration lerp(3, 20, 1) = 20; ramp in 20 * 0.01 * 20 = 4, ramp out 20 * 0.01 * 30 = 6; life 20.
    assert_close(g.duration, 20.0, 0.0);
    assert_close(g.ramp_in, 4.0, 1e-5);
    assert_close(g.ramp_out, 6.0, 1e-5);
    assert_close(g.life, 20.0, 0.0);
    // Position (0.75 * 1600 - 800, 0.25 * 1600 - 800, 0) = (400, -400, 0); spread 0 leaves the direction.
    assert_close3(g.position, Vec3::new(400.0, -400.0, 0.0), 1e-3);
    assert_close3(g.direction, Vec3::X, 1e-6);
    assert_close(g.radius, 800.0, 0.0);
    assert_close(g.weight, 0.0, 0.0);
}

#[test]
fn a_gust_turns_by_the_spread() {
    let mut config = WindConfig::image_defaults();
    config.gust_spread_degrees = 90.0;
    let mut r = randoms(0.5);
    // r 0: 90 * (1 - 0) = 90 degrees: (1, 0, 0) -> (cos 90, sin 90, 0) = (0, 1, 0).
    r.angle = 0.0;
    let left = spawn_gust(Vec3::X, 0.0, &variables(), &r, &config).unwrap();
    assert_close3(left.direction, Vec3::Y, 1e-6);
    // r 1: -90 degrees: (0, -1, 0).
    r.angle = 1.0;
    let right = spawn_gust(Vec3::X, 0.0, &variables(), &r, &config).unwrap();
    assert_close3(right.direction, -Vec3::Y, 1e-6);
}

#[test]
fn no_gust_without_strength_or_duration() {
    let config = WindConfig::image_defaults();
    let mut v = variables();
    v.gust_strength_kmh = [0.0, 0.0];
    assert_eq!(spawn_gust(Vec3::X, 2.0, &v, &randoms(0.5), &config), None);
    let mut v = variables();
    v.gust_duration = [0.0, 0.0];
    assert_eq!(spawn_gust(Vec3::X, 2.0, &v, &randoms(0.5), &config), None);
}

#[test]
fn a_gust_ramps_up_over_a_fifth_and_down_over_the_rest() {
    // Duration 10, ramps 2 and 2: up = (10 - life)/2, down = life/(10 - 2), each in [0, 1].
    // Life 10 -> 9: up 0.5, down min(1, 9/8) = 1: 0.5.
    assert_close(gust(10.0, 0.0).aged(1.0).weight, 0.5, 1e-6);
    // Life 10 -> 8: up 1, down 1: 1.
    assert_close(gust(10.0, 0.0).aged(2.0).weight, 1.0, 1e-6);
    // Life 10 -> 4: up 1, down 0.5.
    assert_close(gust(10.0, 0.0).aged(6.0).weight, 0.5, 1e-6);
    // Life 10 -> 2: 0.25.
    assert_close(gust(10.0, 0.0).aged(8.0).weight, 0.25, 1e-6);
    // A ramp-in of 0 gives 1 at once: at elapsed 0 up is 1 by the guard (0/0 would be NaN),
    // down min(1, 10/8) = 1; at life 9 down min(1, 9/8) = 1.
    let mut sudden = gust(10.0, 0.0);
    sudden.ramp_in = 0.0;
    assert_close(sudden.aged(0.0).weight, 1.0, 1e-6);
    assert_close(sudden.aged(1.0).weight, 1.0, 1e-6);
}

#[test]
fn a_ramp_out_of_zero_ramps_down_over_the_whole_life() {
    // Pins the reading chosen: ramp out 0 gives span 10 - 0 = 10; life 10 -> 5: up 1, down 5/10 = 0.5.
    // The reading "1 if the ramp-out is 0" would give 1.
    let mut g = gust(10.0, 0.0);
    g.ramp_out = 0.0;
    assert_close(g.aged(5.0).weight, 0.5, 1e-6);
}

#[test]
fn a_gust_moves_with_the_wind_and_wraps_to_its_square() {
    let mut g = gust(10.0, 20.0);
    g.position = Vec3::new(790.0, 5.0, 0.0);
    // 790 + 20 * 1 = 810, wrapped to -790; y stays 5; life 10 - 1 = 9.
    let aged = g.aged(1.0);
    assert_close3(aged.position, Vec3::new(-790.0, 5.0, 0.0), 1e-3);
    assert_close(aged.life, 9.0, 0.0);
}

#[test]
fn a_step_spawns_then_ages_and_waits_for_the_interval() {
    let mut v = variables();
    v.gust_interval = [4.0, 8.0];
    let config = WindConfig::image_defaults();
    let r = GustRandoms {
        interval: 0.25,
        ..randoms(0.5)
    };
    let mut state = WindState::calm();
    state.direction = Vec3::new(0.0, -1.0, 0.0);
    state.base_speed = 2.0;
    state.step(1.0, 0.5, &v, 0.0, &r, &config);
    // Next = 1 + lerp(4, 8, 0.25) = 1 + 5 = 6; any other draw (0.5) would give 7.
    assert_close(state.next_gust_time, 6.0, 0.0);
    assert_eq!(state.gusts.len(), 1);
    let g = state.gusts[0];
    // Duration lerp(3, 20, 0.5) = 11.5, ramps 2.3; aged 0.5 in the same frame: life 11,
    // up 0.5/2.3 = 0.217391, down min(1, 11/9.2) = 1.
    assert_close(g.life, 11.0, 1e-5);
    assert_close(g.weight, 0.217_391, 1e-5);
    // Spawned at (0, 0), speed 5.555556 + 2: y = -7.555556 * 0.5 = -3.777778.
    assert_close3(g.position, Vec3::new(0.0, -3.777_778, 0.0), 1e-3);
    // Clock 2 is not past 6: no second gust.
    state.step(2.0, 0.5, &v, 0.0, &r, &config);
    assert_eq!(state.gusts.len(), 1);
    assert_close(state.next_gust_time, 6.0, 0.0);
}

#[test]
fn a_full_list_spawns_nothing_and_a_spent_gust_goes() {
    let mut v = variables();
    v.gust_interval = [4.0, 8.0];
    let mut config = WindConfig::image_defaults();
    config.max_gusts = 1;
    let mut state = WindState::calm();
    state.gusts.push(gust(10.0, 0.0));
    // Clock 10 > 0: the timer resets to 10 + 6 = 16, but the list is full.
    state.step(10.0, 0.5, &v, 0.0, &randoms(0.5), &config);
    assert_eq!(state.gusts.len(), 1);
    assert_close(state.gusts[0].life, 9.5, 0.0);
    assert_close(state.next_gust_time, 16.0, 0.0);
    // Life 0.5 - 1 = -0.5 <= 0: removed.
    state.gusts[0].life = 0.5;
    state.step(11.0, 1.0, &v, 0.0, &randoms(0.5), &config);
    assert!(state.gusts.is_empty());
}

#[test]
fn the_wind_at_a_point_adds_each_gust_by_its_falloff() {
    let mut state = WindState::calm();
    state.direction = Vec3::new(0.0, -1.0, 0.0);
    state.base_speed = 3.0;
    // No gust: dir x s = (0, -3, 0).
    assert_close3(
        state.wind_at(Vec3::ZERO, 0.0),
        Vec3::new(0.0, -3.0, 0.0),
        1e-6,
    );
    // Gust life 6 (weight up 1, down 6/8 = 0.75), strength 8, at the origin; point 400 m east:
    // falloff 1 - 400/800 = 0.5, so + 8 * 0.75 * 0.5 = 3 along x.
    state.gusts.push(gust(6.0, 0.0));
    assert_close3(
        state.wind_at(Vec3::new(400.0, 0.0, 0.0), 0.0),
        Vec3::new(3.0, -3.0, 0.0),
        1e-5,
    );
    // 1200 m east wraps to -400 in the gust's square: the same 0.5.
    assert_close3(
        state.wind_at(Vec3::new(1200.0, 0.0, 0.0), 0.0),
        Vec3::new(3.0, -3.0, 0.0),
        1e-5,
    );
    // 900 m north wraps to -700: falloff 1/8, + 8 * 0.75 * 0.125 = 0.75.
    assert_close3(
        state.wind_at(Vec3::new(0.0, 900.0, 0.0), 0.0),
        Vec3::new(0.75, -3.0, 0.0),
        1e-4,
    );
}

#[test]
fn the_wind_at_a_later_time_moves_and_ramps_the_gusts() {
    let mut state = WindState::calm();
    // Gust moving east at 100 m/s, life 5; 4 s on it is at 400 m with life 1:
    // weight up 1, down 1/8 = 0.125; falloff at 400 m east is 1: 8 * 0.125 = 1.
    state.gusts.push(gust(5.0, 100.0));
    assert_close3(
        state.wind_at(Vec3::new(400.0, 0.0, 0.0), 4.0),
        Vec3::new(1.0, 0.0, 0.0),
        1e-4,
    );
    // 6 s on its life is -1: weight 0, nothing.
    assert_close3(
        state.wind_at(Vec3::new(600.0, 0.0, 0.0), 6.0),
        Vec3::ZERO,
        1e-6,
    );
}

#[test]
fn a_dead_gust_adds_nothing_even_at_full_weight() {
    let mut state = WindState::calm();
    state.direction = Vec3::new(0.0, -1.0, 0.0);
    state.base_speed = 3.0;
    // Ramp out 10 = duration: span 0, so down is 1. Life 1, 5 s on: -4, up ramp(14, 2) = 1, weight 1;
    // kept, it would add 8 * 1 * 1 along x at the origin. Dead, only dir x s = (0, -3, 0).
    let mut g = gust(1.0, 0.0);
    g.ramp_out = 10.0;
    state.gusts.push(g);
    assert_close3(
        state.wind_at(Vec3::ZERO, 5.0),
        Vec3::new(0.0, -3.0, 0.0),
        1e-6,
    );
}

#[test]
fn a_moments_variables_come_by_their_names_hash_else_the_tables_default() {
    // Entry i hashes to i * 7 + 1 and defaults to i / 2, but entry 1104 hashes to the name the moment states.
    let named = name_hash("made_up_heading");
    let mut bytes = Vec::new();
    for i in 0..1407u32 {
        let hash = if i == 1104 { named } else { i * 7 + 1 };
        for word in [i, hash, (i as f32 * 0.5).to_bits(), 1, 1, 2] {
            bytes.extend(word.to_le_bytes());
        }
    }
    let table = VarTable::parse(&bytes).expect("a table");
    let mut own = Keyframe::default();
    own.set("made_up_heading", 90.0);
    own.set("a_name_no_entry_hashes_to", 5.0);
    let v = WindVariables::of_moment(&own, &table).expect("the variables");
    assert_eq!(v.heading_degrees, 90.0);
    // 1103 / 2 and (1107 / 2, 1108 / 2): the defaults.
    assert_eq!(v.heading_weight, 551.5);
    assert_eq!(v.gust_strength_kmh, [553.5, 554.0]);
}
