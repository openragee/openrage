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

fn read_f32(bytes: &[u8; BLOCK_BYTES], offset: usize) -> f32 {
    let mut lane = [0u8; 4];
    lane.copy_from_slice(&bytes[offset..offset + 4]);
    f32::from_le_bytes(lane)
}

/// The variable table's defaults, except a sun size of 2.0 so that the sun and the moon differ.
fn variables() -> SkyVariables {
    SkyVariables {
        sun_disc_gate: 1.0,
        moon_disc_gate: 1.0,
        row_fc: 0.0,
        atmosphere_a_colour: Vec3::new(0.184, 1.0, 0.494),
        atmosphere_a_scale: 0.2,
        atmosphere_b_colour: Vec3::new(0.072, 0.35, 1.0),
        atmosphere_b_scale: 0.1,
        atmosphere_boost: 1.0,
        sun_size: 2.0,
        row_194: 1.0,
        moon_colour: Vec3::ONE,
        moon_size: 2.5,
        row_164: 1.0,
        stars_on: 1.0,
        point_star_strength: 1.0,
        point_star_size: 1.0,
        layer_strength: [1.0; 3],
        twinkle_rate: 1.0,
        row_ec: 1.0,
        row_10c: 1.0,
        point_star_strength_boost: 1.0,
        point_star_size_boost: 1.0,
        layer_boost: [1.0; 3],
    }
}

fn identity_sky() -> SkyMatrix {
    SkyMatrix {
        rotation_rows: [Vec4::X, Vec4::Y, Vec4::Z, Vec4::W],
        fourth_row: Vec4::W,
    }
}

fn view(camera: Vec3, ground_height: f32) -> DomeView {
    DomeView {
        camera,
        ground_height,
        sky: identity_sky(),
        texture_width: None,
    }
}

fn bodies(sun: Vec3, moon: Vec3, night_weight: f32) -> Bodies {
    Bodies {
        sun,
        moon,
        night_weight,
    }
}

fn main_call() -> DomeCall {
    DomeCall::from_words(0, MAIN_VIEW_FLAGS)
}

fn block_at(night_weight: f32, call: DomeCall) -> SkyDomeBlock {
    fill_dome_block(
        &variables(),
        &SkyTunables::image_defaults(),
        &bodies(Vec3::Z, Vec3::Y, night_weight),
        &view(Vec3::ZERO, 0.0),
        call,
        0,
    )
}

fn block_of(v: &SkyVariables, call: DomeCall) -> SkyDomeBlock {
    fill_dome_block(
        v,
        &SkyTunables::image_defaults(),
        &bodies(Vec3::Z, Vec3::Y, 0.0),
        &view(Vec3::ZERO, 0.0),
        call,
        0,
    )
}

#[test]
fn the_main_view_call_sets_point_stars_and_mode_zero() {
    // 0x0100_0100: bit 8 (B) set, bit 16 (A) clear; mode word 0x13 & 0xf = 3.
    let main = main_call();
    assert_eq!(main.mode, 0);
    assert!(main.point_stars);
    assert!(!main.suppress_fc);
    let other = DomeCall::from_words(0x13, 1 << 16);
    assert_eq!(other.mode, 3);
    assert!(other.suppress_fc);
    assert!(!other.point_stars);
}

#[test]
fn the_atmosphere_colours_at_the_defaults() {
    // m = max(1, 0 x 1) = 1. (0.184, 1, 0.494) x 0.2 / 1000 = (3.68e-5, 2.0e-4, 9.88e-5);
    // (0.072, 0.35, 1) x 0.1 / 1000 = (7.2e-6, 3.5e-5, 1.0e-4).
    let block = block_at(0.0, main_call());
    assert_close3(
        block.atmosphere_a,
        Vec3::new(3.68e-5, 2.0e-4, 9.88e-5),
        1e-9,
    );
    assert_close3(block.atmosphere_b, Vec3::new(7.2e-6, 3.5e-5, 1.0e-4), 1e-9);
    // 1 / (1 x 4000) = 2.5e-4; 1 / (1 x 24000) = 4.1666667e-5.
    assert_close(block.inverse_scale_a, 2.5e-4, 1e-10);
    assert_close(block.inverse_scale_b, 4.1666667e-5, 1e-10);
}

#[test]
fn the_night_boost_scales_both_colours_and_never_dims_them() {
    let mut v = variables();
    v.atmosphere_boost = 4.0;
    let t = SkyTunables::image_defaults();
    let fill = |v: &SkyVariables, w3: f32| {
        fill_dome_block(
            v,
            &t,
            &bodies(Vec3::Z, Vec3::Y, w3),
            &view(Vec3::ZERO, 0.0),
            main_call(),
            0,
        )
    };
    // m = max(1, 0.5 x 4) = 2: (3.68e-5, 2e-4, 9.88e-5) x 2 = (7.36e-5, 4e-4, 1.976e-4).
    let night = fill(&v, 0.5);
    assert_close3(
        night.atmosphere_a,
        Vec3::new(7.36e-5, 4.0e-4, 1.976e-4),
        1e-9,
    );
    assert_close3(night.atmosphere_b, Vec3::new(1.44e-5, 7.0e-5, 2.0e-4), 1e-9);
    // v458 = 1, W3 = 0.5: m = max(1, 0.5) = 1, not 0.5.
    let dim = fill(&variables(), 0.5);
    assert_close3(dim.atmosphere_a, Vec3::new(3.68e-5, 2.0e-4, 9.88e-5), 1e-9);
}

#[test]
fn the_air_glow_joins_the_second_colour_only_without_point_stars() {
    let mut t = SkyTunables::image_defaults();
    t.air_glow = Vec3::new(1.0, 0.0, 0.0);
    t.air_glow_galaxy = Vec3::new(0.0, 0.0, 1.0);
    let v = variables();
    let plain = DomeCall::from_words(0, 0);
    // lerp((1, 0, 0), (0, 0, 1), 0.25) = (0.75, 0, 0.25).
    assert_close3(
        air_glow(&v, &t, 0.25, plain),
        Vec3::new(0.75, 0.0, 0.25),
        1e-6,
    );
    // At W3 = 0 the glow is still there: (1, 0, 0).
    assert_close3(air_glow(&v, &t, 0.0, plain), Vec3::X, 1e-6);
    assert_eq!(air_glow(&v, &t, 0.25, main_call()), Vec3::ZERO);
    let mut off = variables();
    off.stars_on = 0.0;
    assert_eq!(air_glow(&off, &t, 0.25, plain), Vec3::ZERO);
    // In the block with v458 = 8: m = max(1, 0.25 x 8) = 2 scales the colour, not the glow:
    // (7.2e-6 x 2 + 0.75, 3.5e-5 x 2, 1.0e-4 x 2 + 0.25) = (0.7500144, 7.0e-5, 0.2502).
    let mut boosted = v;
    boosted.atmosphere_boost = 8.0;
    let block = fill_dome_block(
        &boosted,
        &t,
        &bodies(Vec3::Z, Vec3::Y, 0.25),
        &view(Vec3::ZERO, 0.0),
        plain,
        0,
    );
    assert_close3(
        block.atmosphere_b,
        Vec3::new(0.7500144, 7.0e-5, 0.2502),
        1e-6,
    );
}

#[test]
fn the_point_star_switch_and_row_fc_follow_the_flags() {
    assert_eq!(block_at(0.0, main_call()).point_stars_on, 1.0);
    assert_eq!(
        block_at(0.0, DomeCall::from_words(0, 0)).point_stars_on,
        0.0
    );
    // v472 = 5e-5 < 1e-4 with flag B: the switch is 0 at +0xdc and no star is drawn.
    let mut dark = variables();
    dark.stars_on = 5e-5;
    let block = block_of(&dark, main_call());
    assert_eq!(block.point_stars_on, 0.0);
    assert_eq!(read_f32(&block.to_bytes(), POINT_STARS_ON), 0.0);
    assert!(!point_stars_drawn(&dark, main_call(), 1));
    // v472 exactly 1e-4 is on.
    dark.stars_on = GATE_THRESHOLD;
    assert_eq!(block_of(&dark, main_call()).point_stars_on, 1.0);
    assert!(point_stars_drawn(&dark, main_call(), 1));
    let mut v = variables();
    v.row_fc = 3.0;
    assert_eq!(block_of(&v, main_call()).row_fc, 3.0);
    assert_eq!(block_of(&v, DomeCall::from_words(0, 1 << 16)).row_fc, 0.0);
}

#[test]
fn the_twinkle_phase_wraps_every_two_pi_seconds() {
    // 7000 ms = 7 s; fmod(7, 6.2831853) = 0.7168147; x 1 x 2 = 1.4336294.
    assert_close(twinkle_phase(1.0, 2.0, 7000), 1.4336294, 1e-4);
    // 1000 ms: 1 s, no wrap; x 0.5 x 2 = 1.
    assert_close(twinkle_phase(0.5, 2.0, 1000), 1.0, 1e-5);
}

#[test]
fn the_bodies_sit_twenty_thousand_units_from_the_ground_point() {
    let block = fill_dome_block(
        &variables(),
        &SkyTunables::image_defaults(),
        &bodies(Vec3::Z, Vec3::Y, 0.0),
        &view(Vec3::new(100.0, 200.0, 50.0), 10.0),
        main_call(),
        0,
    );
    // P0 = (100, 200, 10), not the camera's z 50.
    assert_eq!(block.ground_point, Vec3::new(100.0, 200.0, 10.0));
    // Sun (0, 0, 1): (100, 200, 20010); moon (0, 1, 0): (100, 20200, 10).
    assert_close3(block.sun_position, Vec3::new(100.0, 200.0, 20010.0), 1e-2);
    assert_close3(block.moon_position, Vec3::new(100.0, 20200.0, 10.0), 1e-2);
}

#[test]
fn the_moon_basis_faces_the_camera() {
    // P = (0, 20000, 0), D = normalize((0, 0, 0) - P) = (0, -1, 0);
    // R = (0, 0, 1) x (0, -1, 0) = (1, 0, 0); D x R = (0, -1, 0) x (1, 0, 0) = (0, 0, 1).
    let block = block_at(0.0, main_call());
    assert_close3(block.moon_forward, Vec3::new(0.0, -1.0, 0.0), 1e-6);
    assert_close3(block.moon_right, Vec3::X, 1e-6);
    assert_close3(block.moon_up, Vec3::Z, 1e-6);
}

#[test]
fn the_moon_radius_cosine_has_a_fallback_tangent_overhead() {
    // n = (0, 1, 0), t = (0, 1, 0) x (0, 0, 1) = (1, 0, 0); rim (625, 20000, 0):
    // 20000 / sqrt(625^2 + 20000^2) = 20000 / 20009.7632 = 0.99951208 = cos(atan 0.03125).
    assert_close(
        moon_cos_radius(Vec3::new(0.0, 20000.0, 0.0), 2.5),
        0.9995121,
        1e-6,
    );
    // n = (0, 0, 1): the cross is zero, t = (1, 0, 0), the same 0.99951208 (NaN without it).
    assert_close(
        moon_cos_radius(Vec3::new(0.0, 0.0, 20000.0), 2.5),
        0.9995121,
        1e-6,
    );
}

#[test]
fn the_lit_direction_leans_from_the_sun_away_from_the_moon() {
    // Sun (0, 0, 1), moon (1, 0, 0): (-384, 0, 149600) / 149600.4928 = (-0.00256684, 0, 0.9999967).
    let block = fill_dome_block(
        &variables(),
        &SkyTunables::image_defaults(),
        &bodies(Vec3::Z, Vec3::X, 0.0),
        &view(Vec3::ZERO, 0.0),
        main_call(),
        0,
    );
    assert_close3(
        block.moon_lit_direction,
        Vec3::new(-0.00256684, 0.0, 0.9999967),
        1e-6,
    );
    // 0.5 x (1 + 0.00256684) = 0.50128342.
    assert_close(
        moon_lit_fraction(block.moon_lit_direction, Vec3::X),
        0.5012834,
        1e-6,
    );
    // Opposite the sun: L = (1, 0, 0), 0.5 x (1 + 1) = 1 (full).
    assert_close(moon_lit_fraction(Vec3::X, -Vec3::X), 1.0, 1e-6);
}

#[test]
fn the_constant_rows_and_the_sun_radius_term() {
    let block = block_at(0.0, main_call());
    // 1 / (250 x 2)^2 = 1 / 250000 = 4.0e-6 (the moon's 2.5 would give 2.56e-6).
    assert_close(block.sun_inverse_radius_squared, 4.0e-6, 1e-12);
    // Moon (0, 1, 0) from the origin, v470 = 2.5: 0.99951208 (the sun's 2.0 would give 0.99968765).
    assert_close(block.moon_cos_radius, 0.9995121, 1e-6);
    assert_eq!(block.row_190, 0.8);
    assert_eq!(block.inverse_texture_width, 0.0);
    let mut v = view(Vec3::ZERO, 0.0);
    v.texture_width = Some(256);
    let with_texture = fill_dome_block(
        &variables(),
        &SkyTunables::image_defaults(),
        &bodies(Vec3::Z, Vec3::Y, 0.0),
        &v,
        main_call(),
        0,
    );
    // 1 / 256 = 0.00390625.
    assert_eq!(with_texture.inverse_texture_width, 0.00390625);
    // v459 = 0: 1 / 0^2 = infinity.
    let mut no_sun = variables();
    no_sun.sun_size = 0.0;
    assert_eq!(
        block_of(&no_sun, main_call()).sun_inverse_radius_squared,
        f32::INFINITY
    );
}

#[test]
fn the_star_layers_lerp_by_the_night_weight_and_boost() {
    let mut t = SkyTunables::image_defaults();
    t.layer_colour[0] = Vec3::new(1.0, 0.5, 0.25);
    t.layer_night_strength[0] = Vec2::new(0.03, 0.09);
    t.layer_colour[1] = Vec3::ONE;
    let mut v = variables();
    v.layer_boost[0] = 2.0;
    v.layer_strength[0] = 2.0;
    v.layer_boost[1] = 8.0;
    let block = fill_dome_block(
        &v,
        &t,
        &bodies(Vec3::Z, Vec3::Y, 0.25),
        &view(Vec3::ZERO, 0.0),
        main_call(),
        0,
    );
    // 0.03 + 0.06 x 0.25 = 0.045 (swapped tunables give 0.075); max(1, 0.25 x 2) = 1, not 0.5;
    // x 2: 0.09 -> (0.09, 0.045, 0.0225), w 4.
    assert_close3(
        block.cloud_layer.truncate(),
        Vec3::new(0.09, 0.045, 0.0225),
        1e-6,
    );
    assert_eq!(block.cloud_layer.w, 4.0);
    // Dense: 7.5 x max(1, 0.25 x 8) = 7.5 x 2 x 1 = 15 -> (15, 15, 15), w 4.
    assert_close3(block.dense_layer.truncate(), Vec3::splat(15.0), 1e-5);
    assert_eq!(block.dense_layer.w, 4.0);
    // Black by default: the sparse layer is (0, 0, 0) with w 0.5.
    assert_eq!(block.sparse_layer, Vec4::new(0.0, 0.0, 0.0, 0.5));
}

#[test]
fn the_sky_inverse_is_by_cofactors_not_a_transpose() {
    // 90 degrees about z: rows (0, -1, 0), (1, 0, 0), (0, 0, 1); inverse rows (0, 1, 0), (-1, 0, 0), (0, 0, 1).
    let turn = [
        Vec4::new(0.0, -1.0, 0.0, 0.0),
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::Z,
        Vec4::W,
    ];
    let inverse = inverse_rows(&turn);
    assert_close3(inverse[0], Vec3::Y, 1e-6);
    assert_close3(inverse[1], -Vec3::X, 1e-6);
    assert_close3(inverse[2], Vec3::Z, 1e-6);
    // diag(2, 4, 8): inverse diag(0.5, 0.25, 0.125); a transpose would leave diag(2, 4, 8).
    let scaled = [
        Vec4::new(2.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 4.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 8.0, 0.0),
        Vec4::W,
    ];
    let inverse = inverse_rows(&scaled);
    assert_close3(inverse[0], Vec3::new(0.5, 0.0, 0.0), 1e-6);
    assert_close3(inverse[1], Vec3::new(0.0, 0.25, 0.0), 1e-6);
    assert_close3(inverse[2], Vec3::new(0.0, 0.0, 0.125), 1e-6);
    // A singular matrix gives zero rows, not infinities.
    assert_eq!(inverse_rows(&[Vec4::ZERO; 4]), [Vec3::ZERO; 3]);
}

#[test]
fn the_bytes_sit_at_the_block_offsets() {
    let mut v = view(Vec3::new(100.0, 200.0, 50.0), 10.0);
    v.sky.fourth_row = Vec4::new(1.0, 2.0, 3.0, 4.0);
    let block = fill_dome_block(
        &variables(),
        &SkyTunables::image_defaults(),
        &bodies(Vec3::Z, Vec3::Y, 0.0),
        &v,
        main_call(),
        0,
    );
    let bytes = block.to_bytes();
    assert_eq!(bytes.len(), 0x1a0);
    // sun (0, 0, 1) at 0x00..0x0c; moon (0, 1, 0): y at 0x14.
    assert_eq!(read_f32(&bytes, 0x08), 1.0);
    assert_eq!(read_f32(&bytes, 0x14), 1.0);
    // Rotation row 1 = (0, 1, 0, 0): y at 0x64; inverse row 2 z at 0xb8; fourth row w at 0xcc.
    assert_eq!(read_f32(&bytes, 0x64), 1.0);
    assert_eq!(read_f32(&bytes, 0xb8), 1.0);
    assert_eq!(read_f32(&bytes, 0xcc), 4.0);
    // P0 = (100, 200, 10) at 0xd0..0xdc, the point-star switch at 0xdc.
    assert_eq!(read_f32(&bytes, 0xd0), 100.0);
    assert_eq!(read_f32(&bytes, 0xd8), 10.0);
    assert_eq!(read_f32(&bytes, 0xdc), 1.0);
    // 1/4000 at 0xec, the sun's z 20010 at 0x108, 0.8 at 0x190, 1 / 500^2 = 4.0e-6 at 0x198.
    assert_eq!(read_f32(&bytes, 0xec), 2.5e-4);
    assert_close(read_f32(&bytes, 0x108), 20010.0, 1e-2);
    assert_eq!(read_f32(&bytes, 0x190), 0.8);
    assert_eq!(read_f32(&bytes, 0x198), 4.0e-6);
    // 0x150..0x160 is no row: zero.
    assert!(bytes[0x150..0x160].iter().all(|byte| *byte == 0));
}

#[test]
fn a_fan_is_a_centre_and_a_closed_rim() {
    let vertices = fan_vertices(DiscFan {
        radius: 625.0,
        uv_scale: 0.5,
    });
    assert_eq!(vertices.len(), 90);
    assert_eq!(vertices[0].position, Vec3::ZERO);
    assert_eq!(vertices[0].uv, Vec2::splat(0.5));
    // Rim 0 at angle 0: (625, 0, 0), UV (1, 0.5).
    assert_close3(vertices[1].position, Vec3::new(625.0, 0.0, 0.0), 1e-3);
    assert_close(vertices[1].uv.x, 1.0, 1e-6);
    // Rim 22 at 22 x 2 pi / 88 = pi / 2: (0, 0, 625), UV (0.5, 1).
    assert_close3(vertices[23].position, Vec3::new(0.0, 0.0, 625.0), 1e-2);
    assert_close(vertices[23].uv.y, 1.0, 1e-5);
    // Rim 88 at 2 pi closes on rim 0.
    assert_close3(vertices[89].position, vertices[1].position, 1e-2);
}

#[test]
fn the_discs_draw_only_through_their_gates() {
    let v = variables();
    let main = main_call();
    let sun = Some(DiscFan {
        radius: 500.0,
        uv_scale: 1.0,
    });
    let moon = Some(DiscFan {
        radius: 625.0,
        uv_scale: 0.5,
    });
    // Sun r = 250 x 2 = 500, k = 1; moon r = 250 x 2.5 = 625, k = 0.5.
    assert_eq!(sun_disc(&v, Vec3::new(0.0, 0.0, 1.0), main), sun);
    assert_eq!(moon_disc(&v, Vec3::new(0.0, 0.99875, -0.05), main), moon);
    // z exactly -0.1 is drawn; -0.1001 and -0.2 are not.
    assert_eq!(
        moon_disc(&v, Vec3::new(0.0, 0.99499, DISC_MIN_Z), main),
        moon
    );
    assert_eq!(moon_disc(&v, Vec3::new(0.0, 0.99499, -0.1001), main), None);
    assert_eq!(moon_disc(&v, Vec3::new(0.0, 0.98, -0.2), main), None);
    // Mode 1: not drawn.
    assert_eq!(
        sun_disc(&v, Vec3::Z, DomeCall::from_words(1, MAIN_VIEW_FLAGS)),
        None
    );
    // Gates exactly 1e-4: drawn.
    let mut edge = variables();
    edge.sun_disc_gate = GATE_THRESHOLD;
    edge.moon_disc_gate = GATE_THRESHOLD;
    assert_eq!(sun_disc(&edge, Vec3::Z, main), sun);
    assert_eq!(moon_disc(&edge, Vec3::Z, main), moon);
    // Gate v430 = 0 < 1e-4: the moon is not drawn, the sun still is.
    let mut no_moon = variables();
    no_moon.moon_disc_gate = 0.0;
    assert_eq!(moon_disc(&no_moon, Vec3::Z, main), None);
    assert_eq!(sun_disc(&no_moon, Vec3::Z, main), sun);
    // Gate v428 = 0 < 1e-4: the sun is not drawn, the moon still is.
    let mut no_sun = variables();
    no_sun.sun_disc_gate = 0.0;
    assert_eq!(sun_disc(&no_sun, Vec3::Z, main), None);
    assert_eq!(moon_disc(&no_sun, Vec3::Z, main), moon);
    // atan(0.0125 x 2.5) = atan(0.03125) = 0.031239833 rad (1.79 degrees).
    assert_close(disc_angular_radius(2.5), 0.0312398, 1e-7);
}

#[test]
fn point_stars_take_the_frame_scales_and_skip_dark_stars() {
    let mut v = variables();
    v.point_star_size = 1.5;
    v.point_star_strength = 0.5;
    v.point_star_size_boost = 8.0;
    let mut t = SkyTunables::image_defaults();
    t.point_star_night_strength = Vec2::new(1.0, 3.0);
    let scales = point_star_scales(&v, &t, 0.25);
    // Size: max(1, 0.25 x 8) x 1.5 x 1 = 3; colour: max(1, 0.25 x 1) x 0.5 x lerp(1, 3, 0.25)
    // = 1 x 0.5 x 1.5 = 0.75 (swapped tunables give 1.25).
    assert_close(scales.size, 3.0, 1e-6);
    assert_close(scales.colour, 0.75, 1e-6);
    let star = PointStar {
        colour: Vec3::ONE,
        polar: std::f32::consts::FRAC_PI_2,
        azimuth: 0.0,
        size: 2.0,
        intensity: 2.0,
    };
    // A = pi/2, B = 0: (20000, 0, 0), w = 2 x 3 = 6; colour 1 x 2 x 0.75 = 1.5, w 0.
    let (position, colour) = point_star_vertex(&star, &scales).unwrap();
    assert_close3(position.truncate(), Vec3::new(20000.0, 0.0, 0.0), 1e-2);
    assert_close(position.w, 6.0, 1e-6);
    assert_close3(colour.truncate(), Vec3::splat(1.5), 1e-6);
    assert_eq!(colour.w, 0.0);
    let dark = PointStar {
        intensity: 0.0,
        ..star
    };
    assert_eq!(point_star_vertex(&dark, &scales), None);
    assert!(point_stars_drawn(&v, main_call(), 1));
    assert!(!point_stars_drawn(&v, main_call(), 0));
    assert!(!point_stars_drawn(&v, DomeCall::from_words(0, 0), 1));
    let mut off = v;
    off.stars_on = 0.0;
    assert!(!point_stars_drawn(&off, main_call(), 1));
    assert_eq!(point_star_count(2000), 1350);
    assert_eq!(point_star_count(12), 12);
}

#[test]
fn sky_variables_come_from_their_frame_indices() {
    let frame: Vec<f32> = (0..1408).map(|index| index as f32).collect();
    let v = SkyVariables::from_frame(&frame).unwrap();
    assert_eq!(
        (v.sun_disc_gate, v.moon_disc_gate, v.row_fc),
        (428.0, 430.0, 448.0)
    );
    assert_eq!(v.atmosphere_a_colour, Vec3::new(450.0, 451.0, 452.0));
    assert_eq!(v.atmosphere_b_colour, Vec3::new(454.0, 455.0, 456.0));
    assert_eq!(
        (
            v.atmosphere_a_scale,
            v.atmosphere_b_scale,
            v.atmosphere_boost
        ),
        (453.0, 457.0, 458.0)
    );
    assert_eq!(
        (v.sun_size, v.row_194, v.moon_size, v.row_164),
        (459.0, 461.0, 470.0, 471.0)
    );
    assert_eq!(v.moon_colour, Vec3::new(467.0, 468.0, 469.0));
    assert_eq!(
        (
            v.stars_on,
            v.point_star_strength,
            v.point_star_size,
            v.twinkle_rate
        ),
        (472.0, 473.0, 474.0, 478.0)
    );
    assert_eq!(v.layer_strength, [475.0, 476.0, 477.0]);
    assert_eq!((v.row_ec, v.row_10c), (479.0, 480.0));
    assert_eq!(
        (v.point_star_strength_boost, v.point_star_size_boost),
        (481.0, 482.0)
    );
    assert_eq!(v.layer_boost, [483.0, 484.0, 485.0]);
    assert_eq!(SkyVariables::from_frame(&frame[..485]), None);
}
