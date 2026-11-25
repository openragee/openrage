use super::*;

fn close(actual: f32, expected: f32, tolerance: f32) -> bool {
    (actual - expected).abs() <= tolerance
}

fn close_f64(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-9
}

#[test]
fn smootherstep_is_the_quintic_of_the_saturated_input() {
    // t = 0.25: 6/1024 - 15/256 + 10/64 = 0.005859375 - 0.05859375 + 0.15625 = 0.103515625.
    assert!(close_f64(smootherstep(0.25), 0.103515625));
    assert!(close_f64(smootherstep(0.5), 0.5));
    assert!(close_f64(smootherstep(-1.0), 0.0));
    assert!(close_f64(smootherstep(2.0), 1.0));
}

#[test]
fn saturate_maps_nan_to_zero() {
    // max(NaN, 0) is 0, then min(0, 1) is 0; clamp would have kept NaN.
    assert_eq!(saturate(f32::NAN), 0.0);
    assert_eq!(saturate_f64(f64::NAN), 0.0);
    // A zero blend width: (200 - 200) / 0 is NaN, so near is 0.
    let flat = CoronaTunables {
        far_blend_width: 0.0,
        ..CoronaTunables::default()
    };
    assert_eq!(corona_near(200.0, &flat), 0.0);
}

#[test]
fn clock_hour_adds_minutes_and_seconds() {
    // 18 + (30 + 30/60) / 60 = 18 + 30.5/60 = 18.508333...
    assert!(close_f64(clock_hour(18, 30, 30), 18.0 + 30.5 / 60.0));
}

#[test]
fn night_window_fades_on_after_eighteen_and_off_before_six() {
    // d = 60 min, o = 0: on = 18, off = 6 - 1 = 5, span 1 h.
    // 18.5: S(0.5) = 0.5. 12: S(-6) = 0 and 1 - S(7) = 0. 3: 1 - S(-2) = 1. 5.5: 1 - S(0.5) = 0.5.
    assert!(close_f64(night_amount(18.5, 60.0, 0.0), 0.5));
    assert!(close_f64(night_amount(12.0, 60.0, 0.0), 0.0));
    assert!(close_f64(night_amount(3.0, 60.0, 0.0), 1.0));
    assert!(close_f64(night_amount(5.5, 60.0, 0.0), 0.5));
}

#[test]
fn night_window_fade_spans_the_duration() {
    // d = 120 min: span 2 h. on = 18, so 19 gives S(0.5) = 0.5, not S(1) = 1.
    // off = 6 - 2 = 4, so 5 gives 1 - S(0.5) = 0.5.
    assert!(close_f64(night_amount(19.0, 120.0, 0.0), 0.5));
    assert!(close_f64(night_amount(5.0, 120.0, 0.0), 0.5));
}

#[test]
fn night_window_offset_shifts_both_ends() {
    // o = 30 min: on = 18.5, so 18.5 gives S(0) = 0; off = 6 - 1 - 0.5 = 4.5, so 5.0 gives 1 - S(0.5) = 0.5.
    assert!(close_f64(night_amount(18.5, 60.0, 30.0), 0.0));
    assert!(close_f64(night_amount(5.0, 60.0, 30.0), 0.5));
}

#[test]
fn lamp_mode_bits_pick_night_day_always_or_never() {
    // At 03:00 with d = 60, o = 0 the night amount is 1; at 12:00 it is 0.
    let night = 1 << 15;
    let day = 1 << 16;
    assert_eq!(lamp_hour_factor(night, 3.0, 60.0, 0.0), 1.0);
    assert_eq!(lamp_hour_factor(night, 12.0, 60.0, 0.0), 0.0);
    assert_eq!(lamp_hour_factor(day, 3.0, 60.0, 0.0), 0.0);
    assert_eq!(lamp_hour_factor(day, 12.0, 60.0, 0.0), 1.0);
    assert_eq!(lamp_hour_factor(0, 3.0, 60.0, 0.0), 1.0);
    // Both bits at 03:00, where the night amount is 1: never on.
    assert_eq!(lamp_hour_factor(night | day, 3.0, 60.0, 0.0), 0.0);
}

#[test]
fn map_light_chain_applies_the_clock_terms_only_without_a_handle() {
    // Without a handle: 2 * (1 * 0.25 * 2) * 3 = 3. With one: 2 * 3 = 6.
    let chain = MapLightChain {
        attr_intensity: 2.0,
        attr_bits_factor: 1.0,
        no_object_handle: true,
        clock_factor: 1.0,
        hour_factor: 0.25,
        global_intensity: 2.0,
        vehicle: Some(3.0),
        entity_state: None,
        attr_bit2: None,
        setup_bit: None,
        per_object: 1.0,
        final_factor: 1.0,
        entity_alpha: 255,
    };
    assert!(close(chain.intensity(), 3.0, 1e-6));
    let handled = MapLightChain {
        no_object_handle: false,
        ..chain
    };
    assert!(close(handled.intensity(), 6.0, 1e-6));
    // v302 and v310 multiply in when flagged: 3 * 0.5 * 4 = 6.
    let flagged = MapLightChain {
        attr_bit2: Some(0.5),
        setup_bit: Some(4.0),
        ..chain
    };
    assert!(close(flagged.intensity(), 6.0, 1e-6));
    // An owner at alpha 51 scales by 51 / 255 = 0.2: 3 * 0.2 = 0.6; alpha 0 puts the lamp out.
    let fading = MapLightChain {
        entity_alpha: 51,
        ..chain
    };
    assert!(close(fading.intensity(), 0.6, 1e-6));
    let gone = MapLightChain {
        entity_alpha: 0,
        ..chain
    };
    assert_eq!(gone.intensity(), 0.0);
}

#[test]
fn spot_cone_clamps_outer_to_89_and_inner_to_one_below_it() {
    // (0.5, 120): outer min(89) = 89, inner below 1 becomes 1. (50, 40): inner min(50, 39) = 39.
    assert_eq!(clamp_spot_cone(0.5, 120.0), (1.0, 89.0));
    assert_eq!(clamp_spot_cone(50.0, 40.0), (39.0, 40.0));
}

#[test]
fn spot_normalisation_lerps_toward_one_by_k() {
    // ci = 0.9, co = 0.8: n = 2 * 0.2 / ((0.1 + 0.1/3) * 0.2) = 0.4 / 0.0266667 = 15.
    // k = 0: 15; k = 0.5: 15 + 0.5 * (1 - 15) = 8; k = 1 and k = 2 (clamped): 1.
    assert!(close(spot_intensity_factor(0.9, 0.8, 0.0), 15.0, 1e-3));
    assert!(close(spot_intensity_factor(0.9, 0.8, 0.5), 8.0, 1e-3));
    assert!(close(spot_intensity_factor(0.9, 0.8, 1.0), 1.0, 1e-5));
    assert!(close(spot_intensity_factor(0.9, 0.8, 2.0), 1.0, 1e-5));
}

#[test]
fn spot_normalisation_stays_finite_for_a_zero_outer_cone() {
    // Outer 0 degrees has cos 1 exactly: k = 1 gives exactly 1, k = 0 a finite n.
    let cos_inner = 1.0_f32.to_radians().cos();
    assert_eq!(spot_intensity_factor(cos_inner, 1.0, 1.0), 1.0);
    assert!(spot_intensity_factor(cos_inner, 1.0, 0.0).is_finite());
}

#[test]
fn capsule_divides_by_length_plus_one_squared_only_with_bit_17() {
    // length 1: 8 / (1 + 1)^2 = 2; without the bit 8 stays.
    assert_eq!(capsule_intensity(8.0, 1.0, 1 << 17), 2.0);
    assert_eq!(capsule_intensity(8.0, 1.0, 1 << 16), 8.0);
}

#[test]
fn record_radius_is_negative_only_inside_with_the_test_on() {
    // Light at the origin, range 2: camera at x = 1 is inside (1 < 4), at x = 3 outside (9 > 4).
    let light = Vec3::ZERO;
    assert_eq!(
        signed_range(2.0, light, Vec3::new(1.0, 0.0, 0.0), true),
        -2.0
    );
    assert_eq!(
        signed_range(2.0, light, Vec3::new(1.0, 0.0, 0.0), false),
        2.0
    );
    assert_eq!(
        signed_range(2.0, light, Vec3::new(3.0, 0.0, 0.0), true),
        2.0
    );
}

#[test]
fn spot_record_terms_and_rectangle_extents() {
    // co = 0.8: sqrt(1 - 0.64) = 0.6; ci = 0.9: 1 / 0.1 = 10; equal cosines have no inverse.
    let (sin_outer, inverse_gap) = spot_record_terms(0.9, 0.8);
    assert!(close(sin_outer, 0.6, 1e-6));
    assert!(inverse_gap.is_some_and(|value| close(value, 10.0, 1e-4)));
    assert_eq!(spot_record_terms(0.8, 0.8).1, None);
    assert_eq!(rect_extent(0.0), 0.001);
    assert_eq!(rect_extent(2.0), 2.0);
}

#[test]
fn light_index_shifts_the_slot_over_the_byte() {
    // (2 << 8) + 3 = 0x203; the defaults give 0xffff00 + 0xff = 0xffffff.
    assert_eq!(light_index(2, 3), 0x203);
    assert_eq!(light_index(NO_SLOT, NO_SLOT_BYTE), 0x00ff_ffff);
}

#[test]
fn lod_shape_bytes_scale_to_degrees_and_metres() {
    // 51 * 180/255 = 36; 255 * 180/255 = 180; 255 * 140/255 = 140; 51 * 32/255 = 6.4.
    assert!(close(lod_cone_angle(51), 36.0, 1e-4));
    assert!(close(lod_cone_angle(255), 180.0, 1e-4));
    assert!(close(lod_capsule_length(255), 140.0, 1e-4));
    assert!(close(lod_rectangle_extent(51), 6.4, 1e-4));
}

#[test]
fn lod_category_reads_bits_22_to_24_up_to_five_on_set_one_only() {
    // Bit 25 and the low 22 bits lie outside the field; 6 << 22 is above 5.
    assert_eq!(lod_category(3 << 22), Some(3));
    assert_eq!(lod_category((5 << 22) | (1 << 25) | 0x3f_ffff), Some(5));
    assert_eq!(lod_category(6 << 22), None);
    // Category 2 on set 1 takes multiplier 12; on set 0 it takes none, so 1, not 12.
    let multipliers = [10.0, 11.0, 12.0, 13.0, 14.0, 15.0];
    assert_eq!(lod_category_multiplier(2 << 22, true, &multipliers), 12.0);
    assert_eq!(lod_category_multiplier(2 << 22, false, &multipliers), 1.0);
    assert_eq!(lod_category_multiplier(7 << 22, true, &multipliers), 1.0);
}

#[test]
fn lod_colour_intensity_and_range() {
    // (255, 0, 51) / 255 = (1, 0, 0.2), gain z 2: (1, 0, 0.4); halfway to (0, 1, 0): (0.5, 0.5, 0.2).
    let gain = Vec3::new(1.0, 1.0, 2.0);
    let green = Vec3::new(0.0, 1.0, 0.0);
    let plain = lod_colour([255, 0, 51], gain, Vec3::ONE, green, 0.0);
    assert!(plain.abs_diff_eq(Vec3::new(1.0, 0.0, 0.4), 1e-6));
    let blended = lod_colour([255, 0, 51], gain, Vec3::ONE, green, 0.5);
    assert!(blended.abs_diff_eq(Vec3::new(0.5, 0.5, 0.2), 1e-6));
    // 2 * 3 * 0.5 * 4 * 1.5 = 18; 10 * 2 * 1.5 = 30.
    assert!(close(lod_intensity(2.0, 3.0, 0.5, 4.0, 1.5), 18.0, 1e-5));
    assert!(close(lod_range(10.0, 2.0, 1.5), 30.0, 1e-5));
    assert!(lod_light_skipped(0.0, 0.0));
    assert!(!lod_light_skipped(0.0, 1.0));
    assert!(!lod_light_skipped(1.0, 0.0));
}

#[test]
fn octahedral_tangent_decodes_the_axes() {
    // (0.5, 0.5) -> (0, 0, 1); (1, 0.5) -> (1, 0, 0); (1, 1) -> z = -1, folded to (0, 0, -1).
    assert!(octahedral_tangent([0.5, 0.5]).abs_diff_eq(Vec3::Z, 1e-6));
    assert!(octahedral_tangent([1.0, 0.5]).abs_diff_eq(Vec3::X, 1e-6));
    assert!(octahedral_tangent([1.0, 1.0]).abs_diff_eq(-Vec3::Z, 1e-6));
}

#[test]
fn packed_colour_bytes() {
    // 0xff -> 0x10 | 0x0e | 1 = 0x1f; 0x08 -> 1; 0xe0 -> 0x0e.
    assert_eq!(lod_flag_byte(0xff), 0x1f);
    assert_eq!(lod_flag_byte(0x08), 0x01);
    assert_eq!(lod_flag_byte(0xe0), 0x0e);
    // 1 -> 255.5 -> 255; 0.5 -> 128; -0.2 -> -50 -> 0; 1.5 -> 383 -> 255.
    assert_eq!(pack_rgb8(Vec3::new(1.0, 0.5, -0.2)), [255, 128, 0]);
    assert_eq!(pack_rgb8(Vec3::splat(1.5)), [255, 255, 255]);
}

#[test]
fn lod_volume_divides_by_the_magnitude_of_f() {
    // f = -2: (1, 2, 3, 0 + 4) / 2; f = 2: (1, 2, 3, 2 + 4) / 2.
    let v = Vec4::new(1.0, 2.0, 3.0, 4.0);
    let negative = lod_volume(v, -2.0);
    assert!(negative.abs_diff_eq(Vec4::new(0.5, 1.0, 1.5, 2.0), 1e-6));
    let positive = lod_volume(v, 2.0);
    assert!(positive.abs_diff_eq(Vec4::new(0.5, 1.0, 1.5, 3.0), 1e-6));
}

#[test]
fn fade_byte_squares_or_takes_the_root() {
    // 128/255 = 0.50196, squared 0.25196, * 255 + 0.5 = 64.75 -> 64.
    // 64/255 = 0.25098, root 0.50098, * 255 + 0.5 = 128.25 -> 128.
    assert_eq!(fade_byte(128, true), 64);
    assert_eq!(fade_byte(64, false), 128);
    assert_eq!(fade_byte(255, false), 255);
    assert_eq!(fade_byte(0, true), 0);
}

#[test]
fn lod_animation_samples_the_curve_texture() {
    // 1000 ms, speed 1, N = 60: u = frac(30 / 60) = 0.5; v = 1 - 0.5 / 60 = 0.9916667.
    let uv = lod_anim_uv(1000.0, 1.0, 0.0, 0.0, 0, 60);
    assert!(uv.is_some_and(|[u, v]| close(u, 0.5, 1e-6) && close(v, 0.9916667, 1e-6)));
    // Phase 1: x = -0.5, frac 0.5.
    let phased = lod_anim_uv(0.0, 1.0, 1.0, 0.0, 0, 60);
    assert!(phased.is_some_and(|[u, _]| close(u, 0.5, 1e-6)));
    assert_eq!(lod_anim_uv(1000.0, 0.0, 0.0, 0.0, 0, 60), None);
    assert_eq!(lod_anim_uv(1000.0, 1.0, 0.0, 0.0, 0x8000, 60), None);
}

#[test]
fn lod_spheres_add_their_margin() {
    // 500 + 20, 3500 + 150; index 3 is runtime only.
    assert_eq!(lod_sphere(0).map(LodSphere::cull_radius), Some(520.0));
    assert_eq!(lod_sphere(2).map(LodSphere::cull_radius), Some(3650.0));
    assert_eq!(lod_sphere(3), None);
}

#[test]
fn bucket_range_adds_the_half_diagonal() {
    // Box (0,0,0)..(2,0,0): centre (1,0,0), half diagonal 1; camera at x = 11 is 10 away.
    // Radius 9: (1 + 9)^2 = 100 >= 100 kept; radius 8.9: 9.9^2 = 98.01 < 100 dropped.
    let min = Vec3::ZERO;
    let max = Vec3::new(2.0, 0.0, 0.0);
    let camera = Vec3::new(11.0, 0.0, 0.0);
    assert!(bucket_in_range(min, max, camera, 9.0));
    assert!(!bucket_in_range(min, max, camera, 8.9));
}

#[test]
fn day_cap_holds_from_seven_to_sixteen_fifty_nine() {
    // 0.29 * 1440 = 417.6 -> 417 min -> hour 6; 0.31 -> 446 -> 7; 0.7076 -> 1018 -> 16; 0.71 -> 1022 -> 17.
    assert!(!in_day_cap(0.29));
    assert!(in_day_cap(0.31));
    assert!(in_day_cap(0.7076));
    assert!(!in_day_cap(0.71));
    assert_eq!(day_capped_range(520.0, 0.5), 400.0);
    assert_eq!(day_capped_range(520.0, 0.9), 520.0);
    assert_eq!(day_capped_range(300.0, 0.5), 300.0);
}

#[test]
fn main_cull_keeps_only_lights_within_400_m_by_day() {
    // Noon (0.5): 399 m kept, 401 m dropped; 400 m dropped too, as 400^2 = 160000 is not < 160000.
    // 0.9 is hour 21, so 401 m is kept.
    let camera = Vec3::ZERO;
    assert!(light_in_day_reach(Vec3::new(399.0, 0.0, 0.0), camera, 0.5));
    assert!(!light_in_day_reach(Vec3::new(401.0, 0.0, 0.0), camera, 0.5));
    assert!(!light_in_day_reach(Vec3::new(400.0, 0.0, 0.0), camera, 0.5));
    assert!(light_in_day_reach(Vec3::new(401.0, 0.0, 0.0), camera, 0.9));
}

#[test]
fn corona_cone_remaps_the_two_angle_weights() {
    // Facing the camera with angles 10 and 20: both weights near 1, D = 1, cone = (1 + 0.5 - 1) / 0.5 = 1.
    // Side-on: both weights 0, D = 0, cone = (0 + 0.5 - 1) / 0.5 = -1.
    let (cone, d) = corona_cone(Vec3::X, Vec3::X, [10.0, 20.0], 0.5);
    assert!(close(cone, 1.0, 1e-6) && close(d, 1.0, 1e-6));
    let (cone, d) = corona_cone(Vec3::X, Vec3::Y, [10.0, 20.0], 0.5);
    assert!(close(cone, -1.0, 1e-6) && close(d, 0.0, 1e-6));
}

#[test]
fn corona_far_blend_and_intensity() {
    // near: 225 m -> (225 - 200) / 50 = 0.5; 100 m -> 0; 300 m -> 1.
    let tunables = CoronaTunables::default();
    assert!(close(tunables.far_blend_divisor, -0.17365, 1e-5));
    assert!(close(corona_near(225.0, &tunables), 0.5, 1e-6));
    assert!(close(corona_near(100.0, &tunables), 0.0, 1e-6));
    assert!(close(corona_near(300.0, &tunables), 1.0, 1e-6));
    // Behind (alignment < 0), b = 1: halfway from (-1, 0) is (0, 0.5).
    let (cone, d) = corona_far_blend(-1.0, 0.0, -0.5, 0.5, &tunables);
    assert!(close(cone, 0.0, 1e-6) && close(d, 0.5, 1e-6));
    // 0.5 * 2 * 4 = 4; pass 1 with I1 = 3: 12.
    assert!(close(
        corona_intensity(0.5, 2.0, false, 4.0, &tunables),
        4.0,
        1e-6
    ));
    let scaled = CoronaTunables {
        pass_one_intensity_scale: 3.0,
        ..tunables
    };
    assert!(close(
        corona_intensity(0.5, 2.0, true, 4.0, &scaled),
        12.0,
        1e-5
    ));
}

#[test]
fn corona_z_bias_ramps_clamps_and_adds_the_runtime_offset() {
    // Ramp 1..2 m: depth 1.5 gives 0.5 * 0.8 = 0.4, + 0.1 = 0.5.
    // Depth 10 gives 1 * 3 = 3, clamped to 1. Depth 0.5 gives 0, + 0.2 = 0.2.
    let tunables = CoronaTunables::default();
    assert!(close(corona_z_bias(1.5, 0.8, 0.1, &tunables), 0.5, 1e-6));
    assert!(close(corona_z_bias(10.0, 3.0, 0.0, &tunables), 1.0, 1e-6));
    assert!(close(corona_z_bias(0.5, 3.0, 0.2, &tunables), 0.2, 1e-6));
}

#[test]
fn corona_world_size_halves_in_mode_zero_and_shrinks_by_the_bias() {
    // Size 2, cone 1, depth 10, bias 1: pulled = 0.1.
    // Mode 0: 2 * 1 * 10 / 20 = 1, * 0.9 = 0.9. Mode 1: 2 * 0.9 = 1.8. Mode 2: 1.8 * 2 = 3.6.
    let tunables = CoronaTunables::default();
    assert!(close(corona_size(&corona(0, 1.0), &tunables), 0.9, 1e-5));
    assert!(close(corona_size(&corona(1, 1.0), &tunables), 1.8, 1e-5));
    assert!(close(corona_size(&corona(2, 1.0), &tunables), 3.6, 1e-5));
}

#[test]
fn corona_pass_one_takes_its_size_scale() {
    // Z1 = 3, mode 1, pass 1: k = 3 * 1 * 1 = 3, s = 3 * 2 = 6, * 0.9 = 5.4.
    let tunables = CoronaTunables {
        pass_one_size_scale: 3.0,
        ..CoronaTunables::default()
    };
    let input = CoronaSizeInput {
        pass_one: true,
        ..corona(1, 1.0)
    };
    assert!(close(corona_size(&input, &tunables), 5.4, 1e-5));
}

fn corona(mode: u32, fov_ratio: f32) -> CoronaSizeInput {
    CoronaSizeInput {
        mode,
        pass_one: false,
        item_size: 2.0,
        cone_weight: 1.0,
        depth: 10.0,
        bias: 1.0,
        fov_ratio,
    }
}

#[test]
fn corona_screen_constant_size_grows_with_depth_and_field_of_view() {
    // tan(half fov) twice the reference: ratio 2. Mode 0: 0.03 * 2 * 2 = 0.12, * 1 * 10 = 1.2, * 0.9 = 1.08.
    // Mode 4 never uses screen-constant size: 1 * 2 = 2, * 0.9 = 1.8.
    let tunables = CoronaTunables {
        screen_constant: true,
        ..CoronaTunables::default()
    };
    let ratio = corona_fov_ratio(2.0 * tunables.reference_tan_half_fov, &tunables);
    assert!(close(ratio, 2.0, 1e-6));
    assert!(close(corona_size(&corona(0, ratio), &tunables), 1.08, 1e-5));
    assert!(close(corona_size(&corona(4, ratio), &tunables), 1.8, 1e-5));
    // Compensation off ignores the ratio: 0.03 * 2 = 0.06, * 10 = 0.6, * 0.9 = 0.54, not 1.08.
    let uncompensated = CoronaTunables {
        fov_compensation: false,
        ..tunables
    };
    assert!(close(
        corona_size(&corona(0, 2.0), &uncompensated),
        0.54,
        1e-5
    ));
}

#[test]
fn corona_is_pulled_toward_the_camera_by_bias_over_depth() {
    // Position (0,0,10), camera at the origin, bias 1, depth 10: 10 - 10 * 0.1 = 9.
    let pulled = corona_pulled_position(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, 1.0, 10.0);
    assert!(close(pulled.z, 9.0, 1e-6));
    assert!(close(
        corona_depth(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Z),
        10.0,
        1e-6
    ));
}

#[test]
fn shadow_direction_tilts_toward_the_zenith_and_floors_at_0_15_down() {
    // Light from the zenith: straight down. 45 degrees: z = -cos(45 - 0.265 degrees).
    assert!(shadow_direction(Vec3::Z).abs_diff_eq(-Vec3::Z, 1e-6));
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let tilted = shadow_direction(Vec3::new(half, 0.0, half));
    assert!(close(tilted.z, -(44.735_f32.to_radians().cos()), 1e-5));
    assert!(tilted.x < 0.0);
    // Horizon light: tilted z = cos(89.735 degrees) = 0.0046, floored to 0.15, so -0.15 / |(1, 0, 0.15)| = -0.1483.
    let low = shadow_direction(Vec3::X);
    assert!(low.z < -0.14 && low.z > -0.15 && low.x < 0.0);
    // A zero vector: acos(0) tilts to z = 0.0046, floored and normalised to (0, 0, 1), so straight down.
    assert!(shadow_direction(Vec3::ZERO).abs_diff_eq(-Vec3::Z, 1e-6));
}

#[test]
fn linear_splits_space_the_span_evenly_and_pad_with_the_far_split() {
    // Count 2: 0.5, 0.5 + 176.0517 / 2 = 88.5259, then 176.5517 three times.
    let splits = linear_splits(2);
    assert!(close(splits[0], 0.5, 1e-5));
    assert!(close(splits[1], 88.5259, 1e-3));
    assert!(close(splits[2], 176.5517, 1e-3));
    assert!(close(splits[4], 176.5517, 1e-3));
    // Count 0 is taken as 1, so the near split stays 0.5.
    assert_eq!(linear_splits(0)[0], 0.5);
}

#[test]
fn free_camera_splits_match_the_build() {
    // n = 0.5, f = 176.5517, f/n = 353.1; split1 = 0.1 * 44.5129 + 0.9 * 0.5 * 353.1^0.25 = 4.451 + 1.951 = 6.40;
    // split2 = 8.853 + 0.9 * 9.396 = 17.31; split3 = 13.254 + 0.9 * 40.73 = 49.91; near min(0.5, 0.05).
    let splits = cascade_splits(4, 1.0, CASCADE_DEFAULT_LAMBDA, 1.0, true, false, false);
    let expected = [0.05, 6.40, 17.31, 49.91, 176.55];
    for (actual, wanted) in splits.iter().zip(expected.iter()) {
        assert!(close(*actual, *wanted, 0.01), "{actual} against {wanted}");
    }
}

#[test]
fn near_plane_is_pulled_in_only_above_three_cascades_and_only_when_blending() {
    // Three cascades keep 0.5; lambda 0 leaves the linear set whole.
    assert!(close(
        cascade_splits(3, 1.0, 0.9, 1.0, true, false, false)[0],
        0.5,
        1e-6
    ));
    assert_eq!(
        cascade_splits(4, 1.0, 0.0, 1.0, true, false, false),
        linear_splits(4)
    );
}

#[test]
fn a_zero_scale_collapses_the_splits_without_nan() {
    // Every split scales to 0, so near = far = 0 and the blend is skipped.
    let splits = cascade_splits(4, 0.0, 0.9, 1.0, true, false, false);
    assert!(splits.iter().all(|split| *split == 0.0));
}

#[test]
fn splits_take_the_distance_multiplier_and_collapse_without_sun() {
    // Far 176.5517 * 2 = 353.1034; without sun 176.5517 * 0.001 = 0.1765517.
    assert!(close(
        cascade_splits(4, 1.0, 0.9, 2.0, true, false, false)[4],
        353.1034,
        1e-3
    ));
    assert!(close(
        cascade_splits(4, 1.0, 0.9, 1.0, false, false, false)[4],
        0.1765517,
        1e-5
    ));
    assert!(sun_casts_shadows(0.0005, true));
    assert!(!sun_casts_shadows(0.0004, true));
    assert!(!sun_casts_shadows(1.0, false));
}

#[test]
fn far_boost_scales_the_splits_from_the_count_on() {
    // Count 4: split4 = 176.5517 * 1.75 = 308.9655, split3 stays 49.91.
    // Count 2: split2 = 308.9655; split1 = 88.5259 - 0.9 * (88.5259 - 0.5 * 353.1034^0.5) = 17.31, unboosted.
    let four = cascade_splits(4, 1.0, 0.9, 1.0, true, true, false);
    assert!(close(four[4], 308.9655, 1e-3));
    assert!(close(four[3], 49.91, 0.01));
    let two = cascade_splits(2, 1.0, 0.9, 1.0, true, true, false);
    assert!(close(two[2], 308.9655, 1e-3));
    assert!(close(two[1], 17.31, 0.01));
}

#[test]
fn alternate_mode_scales_only_the_last_two_splits() {
    // m = 2: split3 *= 2/3 + 2/3 = 4/3, 49.91 -> 66.55; split4 *= 4/3 + 1/3 = 5/3, 176.5517 -> 294.2529;
    // split1 stays 6.40 rather than doubling.
    let splits = cascade_splits(4, 1.0, 0.9, 2.0, true, false, true);
    assert!(close(splits[1], 6.40, 0.01));
    assert!(close(splits[3], 66.55, 0.02));
    assert!(close(splits[4], 294.2529, 1e-3));
}

#[test]
fn script_lambda_is_used_only_when_positive() {
    // 0 is not > 0, so the default 0.9 applies.
    assert_eq!(script_lambda(-1.0), 0.9);
    assert_eq!(script_lambda(0.0), 0.9);
    assert_eq!(script_lambda(0.5), 0.5);
}

#[test]
fn tracker_scale_ramps_from_one_to_two_and_a_half() {
    // r = 2.5: x = 1.3 / 2.8 = 0.4642857, smooth = 0.2155612 * 2.0714286 = 0.4465197, 1 + 1.5 * that = 1.6697796.
    // Aircraft floor r = 2: x = 0.2857143, smooth = 0.0816327 * 2.4285714 = 0.1982507, scale = 1.2973761.
    assert!(close(tracker_scale_target(1.0, false), 1.0, 1e-6));
    assert!(close(tracker_scale_target(4.0, false), 2.5, 1e-6));
    assert!(close(tracker_scale_target(2.5, false), 1.6697796, 1e-5));
    assert!(close(tracker_scale_target(1.0, true), 1.2973761, 1e-5));
    // Far split for r = 2.5: 176.5517 * 1.6697796 = 294.80.
    let far = cascade_splits(
        4,
        tracker_scale_target(2.5, false),
        0.9,
        1.0,
        true,
        false,
        false,
    )[4];
    assert!(close(far, 294.80, 0.05));
}

#[test]
fn tracked_lambda_ramps_from_0_96_to_0_89() {
    // r = 2.5: x = 1.3 / 4.8 = 0.2708333, smooth = 0.0733507 * 2.4583333 = 0.1803205,
    // factor = 0.96 - 0.07 * 0.1803205 = 0.9473776, times 0.9 / 0.9. r = 6: 0.89.
    assert!(close(tracked_lambda(0.9, 2.5, false), 0.9473776, 1e-5));
    assert!(close(tracked_lambda(0.9, 6.0, false), 0.89, 1e-5));
}

#[test]
fn tracker_scale_eases_a_tenth_a_frame_unless_snapped() {
    // 1 + (2.5 - 1) * 0.1 = 1.15.
    assert!(close(ease_tracker_scale(1.0, 2.5, false), 1.15, 1e-6));
    assert_eq!(ease_tracker_scale(1.0, 2.5, true), 2.5);
}

#[test]
fn steep_camera_shrinks_tracker_and_lambda_past_0_35() {
    // z = 0.3: 1.2 - 1.4 < 0, u = 0: (1, 1). z = -0.475: u = 1.9 - 1.4 = 0.5: (0.9, 0.775).
    // z = 0.9: 3.6 - 1.4 = 2.2 clamped to 1: (0.8, 0.55).
    let (tracker, lambda) = steep_camera_factors(0.3);
    assert!(close(tracker, 1.0, 1e-6) && close(lambda, 1.0, 1e-6));
    let (tracker, lambda) = steep_camera_factors(-0.475);
    assert!(close(tracker, 0.9, 1e-5) && close(lambda, 0.775, 1e-5));
    let (tracker, lambda) = steep_camera_factors(0.9);
    assert!(close(tracker, 0.8, 1e-5) && close(lambda, 0.55, 1e-5));
}

#[test]
fn active_cascades_round_up_clamp_and_drop_small_fractions() {
    // 4 -> (4, 0); 1 -> (2, 0); 2.5 -> (3, 0.5); 3.003 -> ceil 4, fraction 0.003 < 0.005 dropped.
    let active = |count: usize, far_fraction: f32| ActiveCascades {
        count,
        far_fraction,
    };
    assert_eq!(active_cascades(4.0), active(4, 0.0));
    assert_eq!(active_cascades(1.0), active(2, 0.0));
    assert_eq!(active_cascades(2.5), active(3, 0.5));
    assert_eq!(active_cascades(3.003), active(4, 0.0));
}

#[test]
fn far_fraction_moves_the_far_end_between_the_last_two_splits() {
    // Default splits, four active, fraction 0.5: (49.91 + 176.55) / 2 = 113.23.
    let splits = cascade_splits(4, 1.0, 0.9, 1.0, true, false, false);
    let half = ActiveCascades {
        count: 4,
        far_fraction: 0.5,
    };
    let whole = ActiveCascades {
        count: 4,
        far_fraction: 0.0,
    };
    assert!(close(active_far_split(&splits, half), 113.23, 0.01));
    assert!(close(active_far_split(&splits, whole), 176.55, 0.01));
}

#[test]
fn softness_radial_depth_and_far_reach() {
    // Softness 0 -> 0.53, 1 -> 10, -1 -> 0.53, 0.5 -> 0.53 + 9.47 / 2 = 5.265.
    assert!(close(shadow_filter_radius(0.0), 0.53, 1e-6));
    assert!(close(shadow_filter_radius(1.0), 10.0, 1e-5));
    assert!(close(shadow_filter_radius(-1.0), 0.53, 1e-6));
    assert!(close(shadow_filter_radius(0.5), 5.265, 1e-5));
    // 1 + 1 + 2 = 4, so 10 / 2 = 5.
    assert!(close(
        split_view_depth(10.0, 1.0, 2.0_f32.sqrt()),
        5.0,
        1e-5
    ));
    // |(3, 4, 0)| = 5, + sqrt(3) * 1 = 6.732051.
    assert!(close(
        cascade_far_reach(Vec3::new(3.0, 4.0, 0.0), 1.0),
        6.732051,
        1e-5
    ));
}

#[test]
fn far_fade_runs_over_the_last_five_percent() {
    // F = 200: 190 -> 20 * 0.95 - 19 = 0; 195 -> 19.5 - 19 = 0.5; 200 -> 1; 100 -> saturate(-9) = 0.
    assert!(close(cascade_far_fade(190.0, 200.0), 0.0, 1e-5));
    assert!(close(cascade_far_fade(195.0, 200.0), 0.5, 1e-5));
    assert!(close(cascade_far_fade(200.0, 200.0), 1.0, 1e-5));
    assert!(close(cascade_far_fade(100.0, 200.0), 0.0, 1e-6));
    // A zero reach stays finite: 0 / MIN_POSITIVE - 19 saturates to 0.
    assert_eq!(cascade_far_fade(0.0, 0.0), 0.0);
}
