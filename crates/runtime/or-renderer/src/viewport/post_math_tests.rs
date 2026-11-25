use super::*;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

fn assert_close(actual: f32, expected: f32) {
    assert!(close(actual, expected), "{actual} != {expected}");
}

#[test]
fn the_jitter_table_is_halton_2_3_mapped_to_minus_one_one() {
    // Index 1: (1/2, 1/3) -> (0, -1/3). Index 2: (1/4, 2/3) -> (-1/2, 1/3).
    // Index 3: (3/4, 1/9) -> (1/2, -7/9). Index 4: (1/8, 4/9) -> (-3/4, -1/9).
    let table = jitter_table();
    assert_close(table[0].x, 0.0);
    assert_close(table[0].y, -1.0 / 3.0);
    assert_close(table[1].x, -0.5);
    assert_close(table[1].y, 1.0 / 3.0);
    assert_close(table[2].x, 0.5);
    assert_close(table[2].y, -7.0 / 9.0);
    assert_close(table[3].x, -0.75);
    assert_close(table[3].y, -1.0 / 9.0);
    for point in table.iter() {
        assert!((-1.0..=1.0).contains(&point.x) && (-1.0..=1.0).contains(&point.y));
    }
}

#[test]
fn the_first_frame_uses_entry_one_and_the_delta_is_half_the_change() {
    // Entry 1 = (-1/2, 1/3), scale 0.5: j = (-0.25, 1/6); delta = (j - 0) / 2 = (-0.125, 1/12);
    // resolve offset = (0.125, 1/12). Entry 2 = (1/2, -7/9): j = (0.25, -7/18);
    // delta = ((0.25 + 0.25) / 2, (-7/18 - 3/18) / 2) = (0.25, -5/18).
    let mut sequence = JitterSequence::new(4, 0.5);
    let first = sequence.advance();
    assert_close(first.jitter.x, -0.25);
    assert_close(first.jitter.y, 1.0 / 6.0);
    assert_close(first.delta.x, -0.125);
    assert_close(first.delta.y, 1.0 / 12.0);
    assert_close(first.resolve_offset.x, 0.125);
    assert_close(first.resolve_offset.y, 1.0 / 12.0);
    let second = sequence.advance();
    assert_close(second.delta.x, 0.25);
    assert_close(second.delta.y, -5.0 / 18.0);
}

#[test]
fn the_jitter_repeats_after_its_period() {
    // Period 4: indices 1, 2, 3, 0, then 1 again; frame 4 is entry 0 = (0, -1/3) x 0.5.
    let mut sequence = JitterSequence::new(4, 0.5);
    let first = sequence.advance().jitter;
    sequence.advance();
    sequence.advance();
    let fourth = sequence.advance().jitter;
    assert_close(fourth.x, 0.0);
    assert_close(fourth.y, -1.0 / 6.0);
    assert_eq!(sequence.advance().jitter, first);
}

#[test]
fn the_period_is_held_to_sixteen_and_to_one() {
    // Period 40 is held to 16: frame 17 repeats frame 1. Period 0 is held to 1: always entry 0.
    let mut long = JitterSequence::new(40, 1.0);
    let first = long.advance().jitter;
    for _ in 0..15 {
        long.advance();
    }
    assert_eq!(long.advance().jitter, first);
    let mut short = JitterSequence::new(0, 1.0);
    assert_eq!(short.advance().jitter, jitter_table()[0]);
    assert_eq!(short.advance().jitter, jitter_table()[0]);
}

#[test]
fn tent_weights_split_between_the_two_nearest_taps() {
    // Offset (0.5, 0.5): x weights (0, 0.5, 0.5), y the same; taps (0,0) (1,0) (0,1) (1,1) take 0.25.
    let weights = neighbourhood_weights(ResolveKernel::Tent, Vec2::new(0.5, 0.5), true);
    let expected = [0.0, 0.0, 0.0, 0.0, 0.25, 0.25, 0.0, 0.25, 0.25];
    for (actual, wanted) in weights.iter().zip(expected.iter()) {
        assert_close(*actual, *wanted);
    }
}

#[test]
fn catmull_rom_weights_are_normalised_only_when_asked() {
    // f(0.5) = 0.1875 - 0.625 + 1 = 0.5625; f(1.5) = -1.6875 + 5.625 - 6 + 2 = -0.0625.
    // At offset (0.5, 0.5) the axis sum is 1.0625 = 17/16, the 2D sum 289/256, the centre 81/256.
    let offset = Vec2::new(0.5, 0.5);
    let raw = neighbourhood_weights(ResolveKernel::CatmullRom, offset, false);
    assert_close(raw[4], 81.0 / 256.0);
    let sum: f32 = raw.iter().sum();
    assert_close(sum, 289.0 / 256.0);
    let normalised = neighbourhood_weights(ResolveKernel::CatmullRom, offset, true);
    let normalised_sum: f32 = normalised.iter().sum();
    assert_close(normalised_sum, 1.0);
    assert_close(normalised[4], 81.0 / 289.0);
}

#[test]
fn plus_weights_are_always_normalised() {
    // Catmull-Rom at (0.5, 0.5): (0,-1) and (-1,0) = 0.5625 x -0.0625 = -9/256; (0,0), (1,0), (0,1)
    // = 81/256 each; sum = 225/256, so the centre is 81/225 = 0.36.
    let weights = plus_weights(ResolveKernel::CatmullRom, Vec2::new(0.5, 0.5));
    assert_close(weights[2], 0.36);
    assert_close(weights[0], -9.0 / 225.0);
    let sum: f32 = weights.iter().sum();
    assert_close(sum, 1.0);
}

#[test]
fn plus_weights_with_every_tap_zero_stay_zero() {
    // Tent at (2.5, 0): every tap is at least 1.5 texels away in x, so every weight is 0 and the sum is 0.
    let weights = plus_weights(ResolveKernel::Tent, Vec2::new(2.5, 0.0));
    assert_eq!(weights, [0.0; 5]);
}

#[test]
fn mitchell_takes_each_branch_and_is_zero_past_two() {
    // m(0) = (16/3) / 6 = 8/9. m(1) = (7 - 12 + 16/3) / 6 = 1/18.
    // m(0.5) = (0.875 - 3 + 5.333333) / 6 = 0.5347222; x m(0) = 0.4753086.
    // m(1.5) = (-7.875 + 27 - 30 + 10.666667) / 6 = -0.0347222; x m(0) = -0.0308642. m(2.5) = 0.
    assert_close(ResolveKernel::Mitchell.weight(0.0, 0.0), 64.0 / 81.0);
    assert_close(ResolveKernel::Mitchell.weight(1.0, 0.0), 8.0 / 9.0 / 18.0);
    assert_close(
        ResolveKernel::Mitchell.weight(0.5, 0.0),
        0.534_722_2 * 8.0 / 9.0,
    );
    assert_close(
        ResolveKernel::Mitchell.weight(1.5, 0.0),
        -0.034_722_2 * 8.0 / 9.0,
    );
    assert_close(ResolveKernel::Mitchell.weight(2.5, 0.0), 0.0);
}

#[test]
fn the_gaussian_spread_grows_with_its_parameter() {
    // s = 2: (1 + 1)^2 = 4; at (0.5, 0): exp(-2.29 x 4 x 0.25) = exp(-2.29) = 0.101266.
    assert_close(
        ResolveKernel::Gaussian(2.0).weight(0.5, 0.0),
        (-2.29_f32).exp(),
    );
    assert_close(
        ResolveKernel::Gaussian(0.0).weight(0.5, 0.0),
        (-2.29_f32 * 0.25).exp(),
    );
}

#[test]
fn decay_is_exp_of_minus_dt_over_t_and_zero_for_an_unset_constant() {
    // dt = 1 s, T = 1 s: e^-1 = 0.367879. T = 0 (the image's value): 0.
    assert_close(decay(1.0, 1.0), 0.367_879_44);
    assert_close(decay(0.0, 0.0), 0.0);
}

fn history_times() -> HistoryTimes {
    HistoryTimes {
        base: 0.1,
        second: 0.1,
        moving: 0.05,
        speed_low: 0.0,
        speed_high: 10.0,
    }
}

#[test]
fn the_history_weight_moves_toward_the_moving_constant_with_speed() {
    // dt = 0.1: base T = 0.1 gives e^-1 = 0.367879, moving T = 0.05 gives e^-2 = 0.135335.
    // Speed 5 between 0 and 10: k = 0.5, weight = (0.367879 + 0.135335) / 2 = 0.251607.
    let times = history_times();
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, false, -1.0, false).0,
        0.3,
    );
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, true, -1.0, false).0,
        0.367_879_44,
    );
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, true, 5.0, false).0,
        0.251_607_1,
    );
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, true, 20.0, false).0,
        0.135_335_28,
    );
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, true, 5.0, true).0,
        0.5,
    );
}

#[test]
fn the_speed_along_the_view_is_minus_the_dot_product() {
    // (1, 2, 0) . (3, 1, 5) = 5, so sp = -5.
    assert_close(
        speed_along_view(Vec3::new(1.0, 2.0, 0.0), Vec3::new(3.0, 1.0, 5.0)),
        -5.0,
    );
}

#[test]
fn the_second_history_factor_is_recomputed_under_the_same_flag() {
    // dt = 0.1, second T = 0.1: e^-1 = 0.367879 when recomputed; else last frame's 0.2 stands (the viewer's
    // choice). Speed and the forced half move only the first factor.
    let times = history_times();
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, true, 5.0, false).1,
        0.367_879_44,
    );
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, false, 5.0, false).1,
        0.2,
    );
    assert_close(
        history_weight(0.3, 0.2, 0.1, &times, true, 5.0, true).1,
        0.367_879_44,
    );
}

#[test]
fn the_taa_terms_decay_remap_and_switch() {
    // dt = 0.1: T = 0.1 gives e^-1, T = 0 gives 0, T = 0.05 gives e^-2. Range (1, 3): (1, 1/2);
    // the image's empty range (0, 0): (0, 0). Switch 0.6 > 0.5: 1; 0.5 is not above: 0.
    let mut tunables = TaaTunables {
        response_times: [0.1, 0.0, 0.05],
        remap_ranges: [Vec2::new(1.0, 3.0), Vec2::ZERO],
        switch: 0.6,
    };
    let terms = taa_terms(0.1, &tunables);
    assert_close(terms.response[0], 0.367_879_44);
    assert_close(terms.response[1], 0.0);
    assert_close(terms.response[2], 0.135_335_28);
    assert_eq!(terms.remaps[0], Vec2::new(1.0, 0.5));
    assert_eq!(terms.remaps[1], Vec2::ZERO);
    assert_close(terms.switch, 1.0);
    tunables.switch = 0.5;
    assert_close(taa_terms(0.1, &tunables).switch, 0.0);
}

fn test_knots() -> [Vec2; 7] {
    [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 10.0),
        Vec2::new(2.0, 10.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(5.0, 0.0),
        Vec2::new(6.0, -2.0),
        Vec2::new(8.0, -2.0),
    ]
}

#[test]
fn the_exposure_curve_is_linear_between_knots_and_held_outside() {
    // 0.5 is half way 0 -> 10: 5. 3 is half way 10 -> 0: 5. 5.5 is half way 0 -> -2: -1.
    // -5 holds the first knot's 0; 9 holds the last knot's -2.
    let knots = test_knots();
    assert_close(exposure_offset(&knots, 0.5), 5.0);
    assert_close(exposure_offset(&knots, 3.0), 5.0);
    assert_close(exposure_offset(&knots, 5.5), -1.0);
    assert_close(exposure_offset(&knots, -5.0), 0.0);
    assert_close(exposure_offset(&knots, 9.0), -2.0);
}

#[test]
fn the_curve_bounds_take_only_the_end_knots() {
    // Ends (0, 0) and (8, -2): (0, 8, -2, 0); the inner knot y = 10 does not reach max y.
    let bounds = exposure_curve_bounds(&test_knots());
    assert_eq!(bounds, Vec4::new(0.0, 8.0, -2.0, 0.0));
}

#[test]
fn the_fourth_knot_is_shared_by_both_pieces() {
    // Knot 3 is 0x312/0x313 (end of piece 1), knot 4 is 0x314/0x315 (piece 2's second knot).
    let mut frame = [0.0_f32; FRAME_FLOATS];
    frame[0x312] = 4.0;
    frame[0x313] = -1.0;
    frame[0x314] = 6.0;
    frame[0x315] = -3.0;
    frame[0x318] = 9.0;
    let knots = exposure_knots(&frame);
    assert_eq!(knots[3], Vec2::new(4.0, -1.0));
    assert_eq!(knots[4], Vec2::new(6.0, -3.0));
    assert_eq!(knots[6].x, 9.0);
}

#[test]
fn the_limits_range_clamp_and_compensate() {
    // Limits [-2, 2], compensation 0.25 + 0.25 = 0.5. Range of 0 = 0.5, of 5 = 1.
    // Clamp of 5 = 2 + 0.5 = 2.5; of -9 = -2 + 0.5 = -1.5.
    let mut frame = [0.0_f32; FRAME_FLOATS];
    frame[0x31c] = -2.0;
    frame[0x31d] = 2.0;
    frame[0x31a] = 0.25;
    frame[0x31b] = 0.25;
    let limits = ExposureLimits::from_frame(&frame);
    assert_close(limits.compensation_ev, 0.5);
    assert_close(limits.range(0.0), 0.5);
    assert_close(limits.range(5.0), 1.0);
    assert_close(limits.clamp(5.0), 2.5);
    assert_close(limits.clamp(-9.0), -1.5);
    let empty = ExposureLimits {
        min_ev: 1.0,
        max_ev: 1.0,
        compensation_ev: 0.0,
    };
    assert_close(empty.range(0.5), 0.0);
    assert_close(empty.range(1.0), 1.0);
}

#[test]
fn the_frame_resets_the_window_when_v31e_is_positive() {
    let mut frame = [0.0_f32; FRAME_FLOATS];
    assert!(!exposure_reset(&frame));
    frame[0x31e] = 1.0;
    assert!(exposure_reset(&frame));
}

fn frame_input(exposure: f32) -> WindowInput {
    WindowInput {
        exposure,
        dt: 0.1,
        reset: false,
        hold: false,
        live: false,
        force: false,
    }
}

#[test]
fn the_window_is_a_box_mean_of_the_last_n() {
    // N = 3. Push 1: head 1, count 2, (1 + 0) / 2 = 0.5. Push 2: head 2, count 3,
    // (1 + 2 + 0) / 3 = 1.0, dt (0.1 + 0.1 + 0) / 3. Push 3: head 0, count 3, 6 / 3 = 2.
    // Push 4 replaces 1: (4 + 2 + 3) / 3 = 3.
    let mut window = ExposureWindow::new(3);
    assert_close(window.update(&frame_input(1.0)).exposure, 0.5);
    let two = window.update(&frame_input(2.0));
    assert_close(two.exposure, 1.0);
    assert_close(two.dt, 0.2 / 3.0);
    assert!(!two.live);
    assert_close(window.update(&frame_input(3.0)).exposure, 2.0);
    assert_close(window.update(&frame_input(4.0)).exposure, 3.0);
}

#[test]
fn a_reset_empties_the_window_for_three_frames() {
    // Reset frame and the two after give (0, 1) live; the fourth pushes 5 at slot 0:
    // head 1, count 2, slot 1 never written, so (5 + 0) / 2 = 2.5.
    let mut window = ExposureWindow::new(4);
    window.update(&frame_input(7.0));
    let mut reset = frame_input(7.0);
    reset.reset = true;
    let empty = WindowMean {
        exposure: 0.0,
        dt: 1.0,
        live: true,
    };
    assert_eq!(window.update(&reset), empty);
    assert_eq!(window.update(&frame_input(7.0)), empty);
    assert_eq!(window.update(&frame_input(7.0)), empty);
    assert_close(window.update(&frame_input(5.0)).exposure, 2.5);
}

#[test]
fn hold_keeps_the_countdown_and_force_skips_it() {
    // Held frames do not count down: after five held frames three more are still empty.
    let mut window = ExposureWindow::new(4);
    let mut held = frame_input(1.0);
    held.reset = true;
    held.hold = true;
    for _ in 0..5 {
        assert!(window.update(&held).live);
    }
    for _ in 0..3 {
        assert!(window.update(&frame_input(1.0)).live);
    }
    assert!(!window.update(&frame_input(1.0)).live);
    // Force pushes through a fresh reset: 6 at slot 0, head 1, count 2, so (6 + 0) / 2 = 3.
    let mut forced = frame_input(6.0);
    forced.reset = true;
    forced.force = true;
    let mut fresh = ExposureWindow::new(4);
    assert_close(fresh.update(&forced).exposure, 3.0);
}

#[test]
fn a_live_frame_pushes_nothing_and_meters_the_live_value() {
    // After 1, 2 (N = 3) the mean is (1 + 2 + 0) / 3 = 1.0; a live frame of 9 leaves it at 1.0
    // and meters 9.
    let mut window = ExposureWindow::new(3);
    window.update(&frame_input(1.0));
    let pushed = window.update(&frame_input(2.0));
    let mut live = frame_input(9.0);
    live.live = true;
    let mean = window.update(&live);
    assert_close(mean.exposure, 1.0);
    assert_close(mean.metered(9.0), 9.0);
    assert_close(pushed.metered(9.0), 1.0);
}

#[test]
fn a_new_length_empties_the_window() {
    // 8, then a length change, then 2 at slot 0: head 1, count 2, (2 + 0) / 2 = 1.0; had 8 stayed in
    // slot 0 and 2 gone to slot 1, the mean would be (8 + 2 + 0) / 3 = 3.33.
    let mut window = ExposureWindow::new(3);
    window.update(&frame_input(8.0));
    window.set_length(5);
    assert_close(window.update(&frame_input(2.0)).exposure, 1.0);
}

#[test]
fn the_tone_members_come_from_the_frame_the_mean_and_the_tunables() {
    // Default knots (-14, 2) and (10, -4) give bounds (-14, 10, -4, 2). v0x36c = 1.0005 is within 0.001
    // of 1, so +0x524 is false; v0x2ff = 1 sets +0x410.
    let mut frame = [0.0_f32; FRAME_FLOATS];
    frame[0x306] = -14.0;
    frame[0x307] = 2.0;
    frame[0x318] = 10.0;
    frame[0x319] = -4.0;
    frame[0x332] = 0.2;
    frame[0x2ff] = 1.0;
    frame[0x36c] = 1.0005;
    frame[0x36d] = 0.18;
    frame[0x31c] = -3.5;
    frame[0x31d] = 3.0;
    let mean = WindowMean {
        exposure: 1.5,
        dt: 0.02,
        live: false,
    };
    let members = tone_members(&frame, &mean, &ExposureTunables::default());
    assert_eq!(members.b3a0, Vec4::new(1.5, 0.02, 0.15, 15.0));
    assert_close(members.b3e8, 0.2);
    assert_eq!(members.b3f0, Vec4::new(-14.0, 10.0, -4.0, 2.0));
    assert_eq!(members.b420, Vec4::new(-3.5, 3.0, 0.0, 0.0));
    assert!(members.b410);
    assert!(!members.b500);
    assert!(!members.b524);
    assert_eq!(members.b51c, Vec2::new(1.0005, 0.18));
    assert_close(members.b430, -11.0);
}

#[test]
fn the_tone_blend_weight_divides_by_one_on_an_empty_range() {
    // (1 - 0) / 4 = 0.25. a = b = 2: (2.3 - 2) / 1 = 0.3. (10 - 0) / 4 saturates to 1. NaN maps to 0.
    assert_close(tone_blend_weight(1.0, 0.0, 4.0), 0.25);
    assert_close(tone_blend_weight(2.3, 2.0, 2.0), 0.3);
    assert_close(tone_blend_weight(10.0, 0.0, 4.0), 1.0);
    assert_close(tone_blend_weight(f32::NAN, 0.0, 4.0), 0.0);
    let blended = blend_tone_sets(&[0.0; 8], &[4.0; 8], 0.25);
    assert_close(blended[7], 1.0);
}

#[test]
fn the_override_sets_stand_only_while_a_flag_is_set() {
    // Flags 0: none. v0x34d = 1: the sets at 0x345.. and 0x34e.. in that order.
    let mut frame = [0.0_f32; FRAME_FLOATS];
    frame[0x345] = 1.0;
    frame[0x34c] = 4.0;
    frame[0x34e] = -2.0;
    frame[0x355] = 3.0;
    assert_eq!(tone_override_sets(&frame), None);
    frame[0x34d] = 1.0;
    let sets = tone_override_sets(&frame);
    assert_eq!(
        sets,
        Some([
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 4.0],
            [-2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 3.0],
        ])
    );
}

#[test]
fn the_light_scale_is_exp2_of_minus_e_times_lane_six() {
    // Lane 6 = 2: E = 1 gives 0.5 x 2 = 1; E = -2 gives 4 x 2 = 8. Lane 2 = 100 must not count.
    let mut set = [0.0_f32; 8];
    set[6] = 2.0;
    set[2] = 100.0;
    assert_close(light_scale(1.0, &set), 1.0);
    assert_close(light_scale(-2.0, &set), 8.0);
}

#[test]
fn the_grade_gamma_is_two_point_four_at_fifteen() {
    // 15 x 0.0533333 + 1.6 = 2.4; 0 gives 1.6.
    assert_close(grade_gamma(15), 2.4);
    assert_close(grade_gamma(0), 1.6);
}

fn neutral_grade(frame: &mut TimecycleFrame, base: usize) {
    for offset in 0..9 {
        frame[base + offset] = 1.0;
    }
    frame[base + 9] = 0.5;
    frame[base + 10] = 0.5;
    frame[base + 11] = 0.5;
    for offset in 12..18 {
        frame[base + offset] = 1.0;
    }
}

#[test]
fn shadow_and_highlight_take_the_power_and_mid_stays_linear() {
    // g = 2, k = 0.5. Shadow (1, 1, 1) x intensity 4 = 4, to the 0.5 = 2, x scale 0 (2) = 4.
    // Mid (0.25, 1, 1) x scale 1 (3): 0.75, no power. Highlight r 0.25 to the 0.5 = 0.5, x scale 2 (1).
    // Shadow width 0.25: w = 1 - 0.5 = 0.5. Pivot 0.25: c3 = (4/3, 4), c1.w = 0.25.
    let mut frame = [0.0_f32; FRAME_FLOATS];
    neutral_grade(&mut frame, GRADE_SET_1);
    frame[GRADE_SET_1 + 0x0f] = 4.0;
    frame[GRADE_SET_1 + 12] = 2.0;
    frame[GRADE_SET_1 + 3] = 0.25;
    frame[GRADE_SET_1 + 13] = 3.0;
    frame[GRADE_SET_1 + 6] = 0.25;
    frame[GRADE_SET_1 + 10] = 0.25;
    frame[GRADE_SET_1 + 9] = 0.25;
    let set = GradeBlock::from_frame(&frame, 2.0).first;
    assert_close(set.shadow.x, 4.0);
    assert_close(set.shadow.w, 0.5);
    assert_close(set.mid.x, 0.75);
    assert_close(set.mid.w, 0.25);
    assert_close(set.highlight.x, 0.5);
    assert_close(set.highlight.y, 1.0);
    assert_close(set.highlight.w, 0.0);
    assert_close(set.pivot.x, 4.0 / 3.0);
    assert_close(set.pivot.y, 4.0);
}

#[test]
fn a_pivot_of_zero_or_one_stays_finite() {
    // p = 0 and p = 1 would give 1/0; the margin keeps both reciprocals finite.
    let mut frame = [0.0_f32; FRAME_FLOATS];
    neutral_grade(&mut frame, GRADE_SET_1);
    neutral_grade(&mut frame, GRADE_SET_2);
    frame[GRADE_SET_1 + 9] = 0.0;
    frame[GRADE_SET_2 + 9] = 1.0;
    let block = GradeBlock::from_frame(&frame, 2.4);
    assert!(block.first.pivot.y.is_finite());
    assert!(block.second.pivot.x.is_finite());
    assert_close(block.first.mid.w, 0.0);
}

#[test]
fn a_pivot_outside_zero_one_keeps_the_builds_reciprocals() {
    // p = 1.5: (1/(1 - 1.5), 1/1.5) = (-2, 2/3). p = -0.5: (1/1.5, 1/(-0.5)) = (2/3, -2).
    let mut frame = [0.0_f32; FRAME_FLOATS];
    neutral_grade(&mut frame, GRADE_SET_1);
    neutral_grade(&mut frame, GRADE_SET_2);
    frame[GRADE_SET_1 + 9] = 1.5;
    frame[GRADE_SET_2 + 9] = -0.5;
    let block = GradeBlock::from_frame(&frame, 2.4);
    assert_close(block.first.pivot.x, -2.0);
    assert_close(block.first.pivot.y, 2.0 / 3.0);
    assert_close(block.second.pivot.x, 2.0 / 3.0);
    assert_close(block.second.pivot.y, -2.0);
}

#[test]
fn the_packed_block_places_set_two_at_float_seventeen() {
    // Set 2 c0 at byte 0x44 = float 17, c4 at 0x84 = float 33; scalars at 34, 35; t at 36.
    let mut frame = [0.0_f32; FRAME_FLOATS];
    neutral_grade(&mut frame, GRADE_SET_1);
    neutral_grade(&mut frame, GRADE_SET_2);
    frame[GRADE_SET_2 + 0x0f] = 9.0;
    frame[GRADE_SET_2 + 0x15] = 7.0;
    frame[GRADE_SET_1 + 0x12] = 5.0;
    frame[GRADE_SET_2 + 0x12] = 6.0;
    frame[GRADE_SET_1 + 0x15] = 8.0;
    frame[GRADE_BLEND_VARIABLE] = 0.75;
    let floats = GradeBlock::from_frame(&frame, 2.0).to_floats();
    // Set 2 shadow (1, 1, 1) x 9, to the 0.5 = 3.
    assert_close(floats[17], 3.0);
    assert_close(floats[16], 8.0);
    assert_close(floats[33], 7.0);
    assert_close(floats[34], 5.0);
    assert_close(floats[35], 6.0);
    assert_close(floats[36], 0.75);
    // Set 1 c3 = (2, 2, 0, 0) at floats 12..16 for p = 0.5.
    assert_close(floats[12], 2.0);
    assert_close(floats[13], 2.0);
}

#[test]
fn the_blend_weight_lerps_set_one_toward_set_two() {
    // Set 1 scalar 0, set 2 scalar 4, t = 0.25: 1. t = 0 would give set 1's 0.
    let mut frame = [0.0_f32; FRAME_FLOATS];
    neutral_grade(&mut frame, GRADE_SET_1);
    neutral_grade(&mut frame, GRADE_SET_2);
    frame[GRADE_SET_2 + 0x12] = 4.0;
    frame[GRADE_BLEND_VARIABLE] = 0.25;
    let block = GradeBlock::from_frame(&frame, 2.0);
    assert_close(block.blended().scalar, 1.0);
    let mut still = block;
    still.blend = 0.0;
    assert_eq!(still.blended(), block.first);
}
