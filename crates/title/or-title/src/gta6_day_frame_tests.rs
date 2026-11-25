use super::*;

const LAST: usize = FRAME_FLOATS - 1;

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-5
}

fn key(angle: f32, sample: i32) -> SampleKey {
    SampleKey { angle, sample }
}

fn three_keys() -> [SampleKey; 3] {
    [key(-3.0, 9), key(0.0, 4), key(3.0, 8)]
}

fn unit_box() -> RegionBox {
    RegionBox {
        region: 1,
        value: 1.0,
        falloff: 10.0,
        hour_a: 0,
        hour_b: 0,
        strength_id: 0,
        min: Vec3::ZERO,
        max: Vec3::splat(10.0),
        uniform_extent: false,
    }
}

fn noon_weight(region_box: &RegionBox, pos: Vec3) -> f32 {
    box_weight(region_box, pos, 12, 0, 0, &[])
}

#[test]
fn the_pick_takes_the_last_key_at_or_below_x() {
    // Keys -3 (9), 0 (4), 3 (8); x = 1.5 lies between key 0 and key 3: t = 1.5 / 3 = 0.5.
    // x = 0 sits on key 0, so lo is that key: (4, 8, 0); a strict < would give (9, 4, 1).
    let keys = three_keys();
    let pick = pick_samples(1.5, &keys).expect("table is not empty");
    assert_eq!((pick.i0, pick.i1), (4, 8));
    assert!(close(pick.t, 0.5));
    let on = pick_samples(0.0, &keys).expect("table is not empty");
    assert_eq!((on.i0, on.i1), (4, 8));
    assert!(close(on.t, 0.0));
}

#[test]
fn the_last_key_pairs_with_the_first_across_two_pi() {
    // x = 3.1 is past key 3: hi wraps to key -3 + 2π = 3.2831855; t = 0.1 / 0.2831855 = 0.353125.
    let keys = three_keys();
    let pick = pick_samples(3.1, &keys).expect("table is not empty");
    assert_eq!((pick.i0, pick.i1), (8, 9));
    assert!((pick.t - 0.353_125).abs() < 1e-4, "{}", pick.t);
    assert_eq!(pick_samples(0.0, &[]), Err(FrameError::EmptyTable));
}

#[test]
fn a_zero_span_gives_t_zero_not_nan() {
    // One key pairs with itself: b - a = 0, which would make 0/0 = NaN at x = 0.
    let one = [key(0.0, 7)];
    let pick = pick_samples(0.0, &one).expect("table is not empty");
    assert_eq!((pick.i0, pick.i1), (7, 7));
    assert_eq!(pick.t, 0.0);
    assert_eq!(pick_samples(1.0, &one).map(|p| p.t), Ok(0.0));
}

#[test]
fn the_pick_is_eased_by_the_lower_samples_curve() {
    // x = 1.5 picks i0 = 4, i1 = 8, t = 0.5. Sample 4 has curve 5 (t²): 0.25; sample 8's curve 0 would give 0.5.
    // A pick from sample 9, past the 9-entry list, is linear: 0.5, where its i1 = 4 would give 0.25.
    let mut types: Vec<usize> = vec![0; 9];
    types[4] = 5;
    let keys = three_keys();
    let pick = pick_samples(1.5, &keys).expect("table is not empty");
    assert!(close(eased_t(&pick, &types), 0.25));
    let past = SamplePick {
        i0: 9,
        i1: 4,
        t: 0.5,
    };
    assert!(close(eased_t(&past, &types), 0.5));
}

#[test]
fn a_curve_hash_names_its_index() {
    assert_eq!(sample_curve_index(0xe037_3822), 0);
    assert_eq!(sample_curve_index(0xd17d_e093), 1);
    assert_eq!(sample_curve_index(0x87bf_daed), 14);
    assert_eq!(sample_curve_index(0x5ba4_9d43), 16);
    assert_eq!(sample_curve_index(0xcef4_0261), 25);
    assert_eq!(sample_curve_index(0xdead_beef), 0);
}

#[test]
fn every_sample_curve_matches_its_formula_at_a_quarter() {
    // Worked at t = 0.25: cos(π/8) = 0.9238795, sin(π/8) = 0.3826834, cos(π/4) = 0.7071068;
    // 2^-7.5 = 0.0055243, 2^-2.5 = 0.1767767; sqrt(0.9375) = 0.9682458, sqrt(0.4375) = 0.6614378,
    // sqrt(0.75) = 0.8660254. Bounce: 7.5625 = 121/16, so bounceOut(0.75) = (0.75/11)²·121/16 + 0.9375
    // = 0.97265625 and bounceOut(0.5) = (0.5/11)²·121/16 + 0.75 = 0.765625.
    let expected: [f32; 26] = [
        0.25,
        0.25,
        0.076_120_5,
        0.382_683_4,
        0.146_446_6,
        0.0625,
        0.4375,
        0.125,
        0.015_625,
        0.578_125,
        0.0625,
        0.003_906_25,
        0.683_593_75,
        0.031_25,
        0.015_625,
        0.762_695_3,
        0.015_625,
        0.005_524_3,
        0.823_223_3,
        0.015_625,
        0.031_754_2,
        0.661_437_8,
        0.066_987_3,
        0.027_343_75,
        0.472_656_25,
        0.117_187_5,
    ];
    for (index, value) in expected.iter().enumerate() {
        let got: f32 = sample_curve(index, 0.25);
        assert!(close(got, *value), "curve {index}: {got} != {value}");
    }
}

#[test]
fn the_upper_halves_of_the_split_curves() {
    // Curve 7 at 0.75: (1.5 - 0.75)(1.5 - 1) + 0.5 = 0.875. Curve 24 at 0.95, last segment:
    // (0.95 - 2.625/2.75)² = (0.05/11)², × 121/16 = 0.00015625, + 0.984375 = 0.98453125.
    // Curve 25 at 0.75: 0.5·bounceOut(0.5) + 0.5 = 0.8828125.
    assert!(close(sample_curve(7, 0.75), 0.875));
    assert!(close(sample_curve(24, 0.95), 0.984_531));
    assert!(close(sample_curve(25, 0.75), 0.882_812_5));
}

#[test]
fn an_unknown_curve_is_linear() {
    assert!(close(sample_curve(26, 0.3), 0.3));
    assert!(close(sample_curve(1000, 0.7), 0.7));
}

#[test]
fn a_stored_half_holds_half_the_value() {
    // 0x3c00 = 1.0, 0xc000 = -2.0, 0x0001 = 2^-24 (subnormal), 0x7c00 = +inf.
    assert_eq!(half_to_f32(0x3c00), 1.0);
    assert_eq!(half_to_f32(0xc000), -2.0);
    assert_eq!(half_to_f32(0x0001), 5.960_464_5e-8);
    assert_eq!(half_to_f32(0x7c00), f32::INFINITY);
    assert_eq!(stored_half_value(0x3c00), 2.0);
}

#[test]
fn two_weathers_blend_by_wx_then_t_then_double() {
    // Halves prev 1 (0x3c00) / 2 (0x4000), next 3 (0x4200) / 4 (0x4400); wx = 0.5, t = 0.25:
    // lerp(1, 3, 0.5) = 2, lerp(2, 4, 0.5) = 3, lerp(2, 3, 0.25) = 2.25, × 2 = 4.5.
    // The last variable's prev i0 is 0 (0x0000): lerp(0, 3, 0.5) = 1.5, lerp(1.5, 3, 0.25) = 1.875, × 2 = 3.75.
    let mut prev_i0 = [0x3c00_u16; FRAME_FLOATS];
    prev_i0[LAST] = 0x0000;
    let prev_i1 = [0x4000_u16; FRAME_FLOATS];
    let next_i0 = [0x4200_u16; FRAME_FLOATS];
    let next_i1 = [0x4400_u16; FRAME_FLOATS];
    let prev = CycleSamples {
        at_i0: &prev_i0,
        at_i1: &prev_i1,
    };
    let next = CycleSamples {
        at_i0: &next_i0,
        at_i1: &next_i1,
    };
    let frame = blend_weathers(prev, Some(next), 0.5, 0.25).expect("lengths match");
    assert!(close(frame[0], 4.5), "{}", frame[0]);
    assert!(close(frame[LAST], 3.75), "{}", frame[LAST]);
    let short = CycleSamples {
        at_i0: &next_i0[..1],
        at_i1: &next_i1,
    };
    assert_eq!(
        blend_weathers(prev, Some(short), 0.5, 0.25).err(),
        Some(FrameError::LengthMismatch {
            left: 1,
            right: FRAME_FLOATS
        })
    );
}

#[test]
fn one_weather_lerps_its_samples_and_doubles() {
    // lerp(1, 2, 0.5) = 1.5, × 2 = 3.
    let at_i0 = [0x3c00_u16; FRAME_FLOATS];
    let at_i1 = [0x4000_u16; FRAME_FLOATS];
    let prev = CycleSamples {
        at_i0: &at_i0,
        at_i1: &at_i1,
    };
    let frame = blend_weathers(prev, None, 0.9, 0.5).expect("lengths match");
    assert!(close(frame[0], 3.0) && close(frame[LAST], 3.0));
    let short = CycleSamples {
        at_i0: &at_i0,
        at_i1: &at_i1[..1],
    };
    assert_eq!(
        blend_weathers(short, None, 0.0, 0.0).err(),
        Some(FrameError::LengthMismatch {
            left: 1,
            right: FRAME_FLOATS
        })
    );
    let long = vec![0x3c00_u16; FRAME_FLOATS + 1];
    let over = CycleSamples {
        at_i0: &long,
        at_i1: &at_i1,
    };
    assert_eq!(
        blend_weathers(over, None, 0.0, 0.0).err(),
        Some(FrameError::LengthMismatch {
            left: FRAME_FLOATS + 1,
            right: FRAME_FLOATS
        })
    );
}

#[test]
fn the_automatic_transition_is_linear_and_never_goes_back() {
    // 30 of 60: 0.5. An old 0.6 stays 0.6 at 30 of 60. 120 of 60 caps at 1; a finished one stays.
    assert!(close(
        auto_transition_step(0.2, 30.0, AUTO_TRANSITION_DURATION),
        0.5
    ));
    assert!(close(
        auto_transition_step(0.6, 30.0, AUTO_TRANSITION_DURATION),
        0.6
    ));
    assert!(close(
        auto_transition_step(0.9, 120.0, AUTO_TRANSITION_DURATION),
        1.0
    ));
    assert!(close(
        auto_transition_step(1.0, 0.0, AUTO_TRANSITION_DURATION),
        1.0
    ));
}

#[test]
fn the_queued_weather_rises_as_its_time_runs_out() {
    // 22,500 of 45,000 ms left: 1 - 0.5 = 0.5. All left: 0. Past the end (left < 0): 1.
    assert!(close(queued_weather_weight(22_500.0, 45_000.0), 0.5));
    assert!(close(queued_weather_weight(45_000.0, 45_000.0), 0.0));
    assert!(close(queued_weather_weight(-10.0, 45_000.0), 1.0));
}

#[test]
fn only_a_weight_of_exactly_one_takes_region_one_alone() {
    assert_eq!(region_pick(Some(0), 1.0), RegionPick::Only(0));
    assert_eq!(region_pick(None, 1.0), RegionPick::Only(1));
    assert_eq!(region_pick(None, 0.4), RegionPick::Blend(0.4));
    assert_eq!(region_pick(None, 0.0), RegionPick::Only(0));
    // 1.5 is neither 1 nor inside (0, 1), so the frame is region 0.
    assert_eq!(region_pick(None, 1.5), RegionPick::Only(0));
}

#[test]
fn the_override_blends_its_region_one_unless_region_one_was_alone() {
    // w = 1 unforced: region 1, no blend. Forced 0 at w = 0.4 still blends by 0.4 (the quirk).
    // w = 1.5 is not region 1 alone, so it blends by 1.5, unlike region_pick; w = 0 does not blend.
    assert_eq!(override_region_pick(None, 1.0), (1, None));
    assert_eq!(override_region_pick(None, 0.4), (0, Some(0.4)));
    assert_eq!(override_region_pick(Some(0), 0.4), (0, Some(0.4)));
    assert_eq!(override_region_pick(Some(1), 1.0), (1, Some(1.0)));
    assert_eq!(override_region_pick(None, 1.5), (0, Some(1.5)));
    assert_eq!(override_region_pick(None, 0.0), (0, None));
}

#[test]
fn frames_lerp_per_variable() {
    // lerp(0, 10, 0.25) = 2.5, lerp(4, 8, 0.25) = 5; the last variable is lerp(0, 10, 0.25) = 2.5.
    let mut from: TimecycleFrame = [0.0; FRAME_FLOATS];
    from[1] = 4.0;
    let mut to: TimecycleFrame = [10.0; FRAME_FLOATS];
    to[1] = 8.0;
    let frame = lerp_frames(&from, &to, 0.25);
    assert_eq!((frame[0], frame[1], frame[LAST]), (2.5, 5.0, 2.5));
}

#[test]
fn a_box_falls_off_linearly_outside_and_is_one_inside() {
    // Box 0..10, F = 10. Inside -> 1. At x = 15 the distance is 15 - 5 - 5 = 5, so 1 - 5/10 = 0.5.
    let region_box = unit_box();
    let inside: f32 = noon_weight(&region_box, Vec3::splat(5.0));
    let outside: f32 = noon_weight(&region_box, Vec3::new(15.0, 5.0, 5.0));
    assert!(close(inside, 1.0));
    assert!(close(outside, 0.5));
}

#[test]
fn height_counts_three_times() {
    // z = 12 is 2 above the box, counted as 6: 1 - 6/10 = 0.4 (0.8 if height counted once).
    // z = 15 is 5 above, counted as 15: 1 - 1.5 = -0.5, left unclamped.
    let region_box = unit_box();
    let two_above: f32 = noon_weight(&region_box, Vec3::new(5.0, 5.0, 12.0));
    let five_above: f32 = noon_weight(&region_box, Vec3::new(5.0, 5.0, 15.0));
    assert!(close(two_above, 0.4));
    assert!(close(five_above, -0.5));
}

#[test]
fn a_flagged_box_takes_its_x_width_on_every_axis() {
    // Box 0..10 by 0..100 by 0..100, flagged: e = 10 × 0.288675 = 2.88675 on all axes.
    // At y = 60 (centre 50): d = 10 - 2.88675 = 7.11325, so 1 - 0.711325 = 0.288675 (1 unflagged).
    let region_box = RegionBox {
        max: Vec3::new(10.0, 100.0, 100.0),
        uniform_extent: true,
        ..unit_box()
    };
    let weight: f32 = noon_weight(&region_box, Vec3::new(5.0, 60.0, 50.0));
    assert!(close(weight, 0.288_675), "{weight}");
}

#[test]
fn a_zero_falloff_is_one_metre_and_strength_defaults_to_one_past_its_list() {
    // F = 0, 0.5 outside: 1 - 0.5 = 0.5. Strength id 0 of [0.5] -> 0.25; id 1 is past the list -> 1.
    let region_box = RegionBox {
        falloff: 0.0,
        ..unit_box()
    };
    let pos: Vec3 = Vec3::new(10.5, 5.0, 5.0);
    assert!(close(box_weight(&region_box, pos, 12, 0, 0, &[]), 0.5));
    assert!(close(box_weight(&region_box, pos, 12, 0, 0, &[0.5]), 0.25));
    let past = RegionBox {
        strength_id: 1,
        ..region_box
    };
    assert!(close(box_weight(&past, pos, 12, 0, 0, &[0.5]), 0.5));
}

#[test]
fn a_modifier_box_queues_weight_times_its_value_only_above_zero() {
    // Inside, value 0.5: 1 × 0.5. At z = 15 the weight is -0.5, so -0.25 is not queued; value 0 is not either.
    let modifier_box = RegionBox {
        value: 0.5,
        ..unit_box()
    };
    let inside: Option<f32> = modifier_box_strength(&modifier_box, Vec3::splat(5.0), 12, 0, 0, &[]);
    assert!(inside.is_some_and(|s| close(s, 0.5)), "{inside:?}");
    let far: Vec3 = Vec3::new(5.0, 5.0, 15.0);
    assert_eq!(
        modifier_box_strength(&modifier_box, far, 12, 0, 0, &[]),
        None
    );
    let zero = RegionBox {
        value: 0.0,
        ..unit_box()
    };
    assert_eq!(
        modifier_box_strength(&zero, Vec3::splat(5.0), 12, 0, 0, &[]),
        None
    );
}

#[test]
fn the_hour_window_ramps_in_before_a_and_out_over_b() {
    // [6, 18]: 05:30 -> 1800/3600 = 0.5; 18:15 -> 1 - 900/3600 = 0.75; 12:00 -> 1; 20:00 -> 0.
    // 06:00 -> 6 <= 6 < 18 gives 1, where a strict start would give 0.
    assert!(close(hour_window(6, 18, 5, 30, 0), 0.5));
    assert!(close(hour_window(6, 18, 18, 15, 0), 0.75));
    assert!(close(hour_window(6, 18, 12, 0, 0), 1.0));
    assert!(close(hour_window(6, 18, 6, 0, 0), 1.0));
    assert!(close(hour_window(6, 18, 20, 0, 0), 0.0));
    assert!(close(hour_window(7, 7, 3, 0, 0), 1.0));
}

#[test]
fn the_hour_window_wraps_past_midnight() {
    // [22, 4]: 22:00 (22 >= 22), 23:00 and 02:00 inside, 12:00 outside, 21:45 ramps in at 2700/3600 = 0.75.
    // [0, 4] at 23:30: (23 + 1) mod 24 = 0 = A, so the ramp gives 0.5.
    assert!(close(hour_window(22, 4, 22, 0, 0), 1.0));
    assert!(close(hour_window(22, 4, 23, 0, 0), 1.0));
    assert!(close(hour_window(22, 4, 2, 0, 0), 1.0));
    assert!(close(hour_window(22, 4, 12, 0, 0), 0.0));
    assert!(close(hour_window(22, 4, 21, 45, 0), 0.75));
    assert!(close(hour_window(0, 4, 23, 30, 0), 0.5));
}

#[test]
fn the_ramp_in_wins_over_the_ramp_out() {
    // [6, 5] at 05:15: hour 5 is both the hour before A and hour B; ramp in = 0.25, ramp out would be 0.75.
    assert!(close(hour_window(6, 5, 5, 15, 0), 0.25));
}

#[test]
fn a_garbage_clock_does_not_overflow() {
    // i32::MAX mod 24 = 7, so (7 + 1) mod 24 = 8 is not A = 6; outside [6, 18): 0.
    assert!(close(hour_window(6, 18, i32::MAX, i32::MAX, i32::MAX), 0.0));
}

#[test]
fn a_region_weight_is_the_largest_positive_box_weight() {
    // Region 1 boxes give 0.5 and 0.8 at x = 15 (F = 10 and F = 25: 1 - 5/25 = 0.8): max 0.8, not 1.3.
    // A region 0 box at 1 - 5/2 = -1.5 is dropped, leaving 0; region 7 is skipped.
    let near = unit_box();
    let far = RegionBox {
        falloff: 25.0,
        ..unit_box()
    };
    let below = RegionBox {
        region: 0,
        falloff: 2.0,
        ..unit_box()
    };
    let stray = RegionBox {
        region: 7,
        ..unit_box()
    };
    let weights = region_weights(
        &[near, far, below, stray],
        Vec3::new(15.0, 5.0, 5.0),
        12,
        0,
        0,
        &[],
    );
    assert!(close(weights[0], 0.0));
    assert!(close(weights[1], 0.8));
}

#[test]
fn a_prev_only_slot_fades_out_with_wx() {
    assert!(close(prev_only_slot_strength(0.25), 0.75));
    assert!(close(prev_only_slot_strength(1.5), 0.0));
    assert!(close(prev_only_slot_strength(-0.5), 1.0));
}

#[test]
fn modifiers_apply_by_category_before_their_order_key() {
    // Keys: (1, 10) = 1010, (0, 1500) = 1500, (1, 5) = 1005. A sort by key alone gives [2, 0, 1];
    // walking categories in list order gives category 0 first, then 1005 before 1010: [1, 2, 0].
    // Categories -1 and 2 are not in a 2-entry list, so they are never walked.
    let queue = [
        QueuedModifier {
            category: 1,
            priority: 10,
        },
        QueuedModifier {
            category: 0,
            priority: 1500,
        },
        QueuedModifier {
            category: 1,
            priority: 5,
        },
        QueuedModifier {
            category: -1,
            priority: 0,
        },
        QueuedModifier {
            category: 2,
            priority: 0,
        },
    ];
    assert_eq!(modifier_apply_order(&queue, 2), vec![1, 2, 0]);
    assert_eq!(modifier_order_key(3, 42), 3042);
}

#[test]
fn a_modifier_strength_multiplies_its_factors_and_skips_zero() {
    // 0.5 × 2 × 0.5 × 0.5 = 0.25; without the flag and the key, 0.5 × 2 = 1; a queued 0 is skipped.
    assert_eq!(
        modifier_strength(0.5, 2.0, Some(0.5), Some(0.5)),
        Some(0.25)
    );
    assert_eq!(modifier_strength(0.5, 2.0, None, None), Some(1.0));
    assert_eq!(modifier_strength(1.0, 0.0, None, None), None);
    assert_eq!(modifier_strength(-1.0, 1.0, None, None), None);
}

#[test]
fn a_mode_two_entry_lerps_toward_v1() {
    // 2·0.75 + 4·0.25 = 2.5.
    assert!(close(apply_mode2(2.0, 4.0, 0.25), 2.5));
    assert!(close(apply_mode2(4.0, 4.0, 0.5), 4.0));
}

#[test]
fn an_ease_hash_names_its_type() {
    assert_eq!(keyed_ease_type(0x2e2f_833f), 1);
    assert_eq!(keyed_ease_type(0x0293_172d), 7);
    assert_eq!(keyed_ease_type(0x879a_6e76), 9);
    assert_eq!(keyed_ease_type(0), 0);
}

#[test]
fn keyed_eases_match_their_formulas_at_a_quarter() {
    // u = 0.25: 1 - cos(π/8) = 0.0761205, sin(π/8) = 0.3826834, 0.5 - 0.5cos(π/4) = 0.1464466,
    // 0.0625, 1 - 0.5625 = 0.4375, 0.015625, 1 - 0.421875 = 0.578125, 0.00390625, 1 - 0.31640625.
    let expected: [f32; 10] = [
        0.25,
        0.076_120_5,
        0.382_683_4,
        0.146_446_6,
        0.0625,
        0.4375,
        0.015_625,
        0.578_125,
        0.003_906_25,
        0.683_593_75,
    ];
    for (ease_type, value) in expected.iter().enumerate() {
        let got: f32 = keyed_ease(ease_type as u32, 0.25);
        assert!(close(got, *value), "ease {ease_type}: {got} != {value}");
    }
    assert!(close(keyed_ease(10, 0.25), 0.25));
}

#[test]
fn only_the_sine_eases_clamp() {
    // u = 1.5: ease 2 clamps to sin(π/2) = 1 (unclamped sin(0.75π) = 0.7071); ease 4 gives 2.25.
    assert!(close(keyed_ease(2, 1.5), 1.0));
    assert!(close(keyed_ease(4, 1.5), 2.25));
}

#[test]
fn a_driver_is_normalised_unless_its_range_is_zero_to_one() {
    // [0, 10]: 5 -> 0.5, 15 -> 1. [0, 1]: 1.5 stays 1.5. [2, 2]: 3 clamps to 2 -> 0 / 0.001 = 0.
    assert!(close(keyed_driver_position(5.0, 0.0, 10.0), 0.5));
    assert!(close(keyed_driver_position(15.0, 0.0, 10.0), 1.0));
    assert!(close(keyed_driver_position(1.5, 0.0, 1.0), 1.5));
    assert!(close(keyed_driver_position(3.0, 2.0, 2.0), 0.0));
}

#[test]
fn a_key_pick_interpolates_and_falls_back_to_key_zero_past_the_last() {
    // Keys 0, 10, 20. x = 15: keys 1 and 2, u = 0.5. x = 25: no key >= 25, so key 0 twice.
    // x = -5: key 1 is the first >= -5, so keys 0 and 1 with u clamped to 0.
    let keys = [0.0, 10.0, 20.0];
    let mid = pick_key(&keys, 15.0, 0).expect("keys exist");
    assert_eq!((mid.lo, mid.hi, mid.u), (1, 2, 0.5));
    let past = pick_key(&keys, 25.0, 0).expect("keys exist");
    assert_eq!((past.lo, past.hi, past.u), (0, 0, 1.0));
    let before = pick_key(&keys, -5.0, 0).expect("keys exist");
    assert_eq!((before.lo, before.hi, before.u), (0, 1, 0.0));
    assert_eq!(pick_key(&[], 1.0, 0), Err(FrameError::EmptyTable));
}

#[test]
fn a_key_pick_blends_values_and_weights_linearly() {
    // Values 2 and 6 at u = 0.25: 2 + 4 × 0.25 = 3. Past the last key (key 0 twice) the value is key 0's.
    let pick = KeyPick {
        lo: 0,
        hi: 1,
        u: 0.25,
    };
    assert_eq!(pick.blend(&[2.0, 6.0]), Ok(3.0));
    let past = KeyPick {
        lo: 0,
        hi: 0,
        u: 1.0,
    };
    assert_eq!(past.blend(&[2.0, 6.0]), Ok(2.0));
    assert_eq!(
        pick.blend(&[2.0]),
        Err(FrameError::MissingValue { index: 1, len: 1 })
    );
}

#[test]
fn keyed_samples_combine_by_the_other_dimensions() {
    // S = 2: lerp(1, 3, 0.5) = 2. S = 4: lerp(lerp(0, 2, 0.5), lerp(4, 6, 0.5), 0.25) = lerp(1, 5, 0.25) = 2.
    assert_eq!(combine_samples(&[7.0], 0.5, 0.5), Ok(7.0));
    assert_eq!(combine_samples(&[1.0, 3.0], 0.5, 0.0), Ok(2.0));
    assert_eq!(combine_samples(&[0.0, 2.0, 4.0, 6.0], 0.5, 0.25), Ok(2.0));
    assert_eq!(
        combine_samples(&[0.0, 1.0, 2.0], 0.5, 0.5),
        Err(FrameError::SampleCount(3))
    );
}

#[test]
fn the_name_hash_matches_the_loaders_constant() {
    // Worked by hand: after "name" h = 0xE7ECD7B6; mixed 0x27539566 -> 0x27577F14 -> 0xE6E17F14.
    assert_eq!(name_hash("name"), 0xE6E1_7F14);
}

#[test]
fn the_name_hash_folds_case_and_backslash_and_drops_quotes() {
    assert_eq!(name_hash("NaMe"), 0xE6E1_7F14);
    assert_eq!(name_hash("\"name\"tail"), 0xE6E1_7F14);
    assert_eq!(name_hash("name\0tail"), 0xE6E1_7F14);
    assert_eq!(name_hash("a\\b"), name_hash("a/b"));
    // A quote ends the name only after a leading quote.
    assert_ne!(name_hash("na\"me"), name_hash("na"));
}

#[test]
fn a_value_is_held_as_the_nearest_half_and_read_back_whole() {
    for bits in [
        0x0000_u16, 0x0001, 0x03ff, 0x0400, 0x3c00, 0x3555, 0x7bff, 0x8001, 0xbc00, 0xc4b0,
    ] {
        assert_eq!(f32_to_half(half_to_f32(bits)), bits, "{bits:#06x}");
    }
    assert_eq!(
        f32_to_half(1.0 + 1.0 / 2048.0),
        0x3c00,
        "a tie goes to the even half"
    );
    assert_eq!(f32_to_half(1.0 + 3.0 / 2048.0), 0x3c02);
    assert_eq!(f32_to_half(1e6), 0x7c00);
    assert_eq!(f32_to_half(1e-9), 0);
    assert_eq!(stored_half_value(f32_to_half(14.75 / 2.0)), 14.75);
}
