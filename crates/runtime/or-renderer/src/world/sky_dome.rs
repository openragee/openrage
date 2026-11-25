//! The sky dome's 0x1a0-byte constant block as plain data, the two disc fans, the point stars and the air glow added to the second atmosphere colour, with axes x east, y north, z up (inferred).
//! Not known and so not built: the per-view sky rotation's own arithmetic (it is an argument), the w lane D carries at +0x13c, the w lanes of +0x12c, +0x14c and the inverse rows, and what the pixel program does with the rows named only by offset.

use glam::{Mat3, Vec2, Vec3, Vec4};

/// Bytes in the dome block (26 float4).
pub const BLOCK_BYTES: usize = 0x1a0;

/// Byte offset of the sun disc direction (xyz).
pub const SUN_DIRECTION: usize = 0x00;
/// Byte offset of the twinkle phase.
pub const TWINKLE_PHASE: usize = 0x0c;
/// Byte offset of the moon disc direction (xyz).
pub const MOON_DIRECTION: usize = 0x10;
/// Byte offset of row +0x1c.
pub const ROW_1C: usize = 0x1c;
/// Byte offset of the "cloud" star layer (float4).
pub const CLOUD_LAYER: usize = 0x20;
/// Byte offset of the "dense" star layer (float4).
pub const DENSE_LAYER: usize = 0x30;
/// Byte offset of the "sparse" star layer (float4).
pub const SPARSE_LAYER: usize = 0x40;
/// Byte offset of the sky rotation's four rows (float4 each).
pub const SKY_ROTATION: usize = 0x50;
/// Byte offset of the sky rotation's three inverse rows (float4 each).
pub const SKY_INVERSE: usize = 0x90;
/// Byte offset of the sky matrix's fourth row (float4).
pub const SKY_FOURTH_ROW: usize = 0xc0;
/// Byte offset of the ground point P0 = (cam.x, cam.y, Z0).
pub const GROUND_POINT: usize = 0xd0;
/// Byte offset of the point-star switch (1.0 or 0.0).
pub const POINT_STARS_ON: usize = 0xdc;
/// Byte offset of the first atmosphere colour (xyz).
pub const ATMOSPHERE_A: usize = 0xe0;
/// Byte offset of 1 / (v479 x its tunable).
pub const INVERSE_SCALE_A: usize = 0xec;
/// Byte offset of the second atmosphere colour (xyz).
pub const ATMOSPHERE_B: usize = 0xf0;
/// Byte offset of row +0xfc (v448).
pub const ROW_FC: usize = 0xfc;
/// Byte offset of the sun position (xyz).
pub const SUN_POSITION: usize = 0x100;
/// Byte offset of 1 / (v480 x its tunable).
pub const INVERSE_SCALE_B: usize = 0x10c;
/// Byte offset of the moon position (xyz).
pub const MOON_POSITION: usize = 0x110;
/// Byte offset of row +0x11c.
pub const ROW_11C: usize = 0x11c;
/// Byte offset of the moon disc's right axis (xyz).
pub const MOON_RIGHT: usize = 0x120;
/// Byte offset of the moon disc's forward axis, camera minus moon (xyz).
pub const MOON_FORWARD: usize = 0x130;
/// Byte offset of the moon disc's up axis (xyz).
pub const MOON_UP: usize = 0x140;
/// Byte offset of the cosine of the moon's angular radius.
pub const MOON_COS_RADIUS: usize = 0x160;
/// Byte offset of row +0x164 (v471).
pub const ROW_164: usize = 0x164;
/// Byte offset of row +0x168.
pub const ROW_168: usize = 0x168;
/// Byte offset of row +0x16c.
pub const ROW_16C: usize = 0x16c;
/// Byte offset of the moon's lit direction (xyz).
pub const MOON_LIT_DIRECTION: usize = 0x170;
/// Byte offset of row +0x17c.
pub const ROW_17C: usize = 0x17c;
/// Byte offset of the moon colour (xyz).
pub const MOON_COLOUR: usize = 0x180;
/// Byte offset of row +0x18c.
pub const ROW_18C: usize = 0x18c;
/// Byte offset of row +0x190 (always 0.8).
pub const ROW_190: usize = 0x190;
/// Byte offset of row +0x194 (v461).
pub const ROW_194: usize = 0x194;
/// Byte offset of 1 / (sun disc radius) squared.
pub const SUN_INVERSE_RADIUS_SQUARED: usize = 0x198;
/// Byte offset of 1 / a bound texture's width.
pub const INVERSE_TEXTURE_WIDTH: usize = 0x19c;

/// Distance of the bodies and the point stars from the ground point, world units.
pub const DOME_DISTANCE: f32 = 20000.0;

/// Disc radius per unit of v459 or v470, world units at the dome distance.
pub const DISC_RADIUS_PER_SIZE: f32 = 250.0;

/// Sun weight of the lit direction (the sun's distance in thousands of km, inferred).
const SUN_DISTANCE: f32 = 149600.0;

/// Moon weight of the lit direction, subtracted (the moon's distance in thousands of km, inferred).
const MOON_DISTANCE: f32 = 384.0;

/// Value at or above which a gate variable is on.
pub const GATE_THRESHOLD: f32 = 1e-4;

/// Lowest z of a disc direction at which its disc is drawn.
pub const DISC_MIN_Z: f32 = -0.1;

/// Divisor of both atmosphere colours (per-metre coefficients stated per kilometre, inferred).
const ATMOSPHERE_DIVISOR: f32 = 1000.0;

/// Seconds per tick of the ms counter.
const SECONDS_PER_TICK: f32 = 0.001;

/// The constant written at row +0x190.
const ROW_190_VALUE: f32 = 0.8;

/// Mask of the call's mode word that gives the mode.
const MODE_MASK: u32 = 0xf;

/// Flag bit A: writes 0 at +0xfc.
const FLAG_SUPPRESS_FC: u32 = 1 << 16;

/// Flag bit B: point stars instead of the air glow in +0xf0.
const FLAG_POINT_STARS: u32 = 1 << 8;

/// Flag word of the main view's dome call: B and the two extra binds of bit 24.
pub const MAIN_VIEW_FLAGS: u32 = 0x0100_0100;

/// Vertices of a disc fan: a centre then 89 rim points, the first and last rim points coincident.
pub const FAN_VERTICES: usize = 90;

/// Angle between rim points, radians: the image's f32 of 2 pi / 88.
const RIM_STEP: f32 = f32::from_bits(0x3d92_3a14);

/// UV of a fan's centre on both axes.
const FAN_UV_CENTRE: f32 = 0.5;

/// UV scale of the sun fan (UV spans 0.5 +/- 1).
const SUN_UV_SCALE: f32 = 1.0;

/// UV scale of the moon fan (the texture fills the disc).
const MOON_UV_SCALE: f32 = 0.5;

/// Most point stars a frame draws.
pub const MAX_POINT_STARS: usize = 1350;

/// Vertices per point star (two triangles).
pub const VERTICES_PER_STAR: usize = 6;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// max(1, W3 x v): the night boost of a colour or a star term.
fn night_boost(night_weight: f32, variable: f32) -> f32 {
    (night_weight * variable).max(1.0)
}

/// The timecycle variables the dome block, the discs and the point stars read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyVariables {
    /// v428, the sun disc gate.
    pub sun_disc_gate: f32,
    /// v430, the moon disc gate.
    pub moon_disc_gate: f32,
    /// v448, row +0xfc.
    pub row_fc: f32,
    /// v450..v452, the first atmosphere colour.
    pub atmosphere_a_colour: Vec3,
    /// v453, the first atmosphere colour's scale.
    pub atmosphere_a_scale: f32,
    /// v454..v456, the second atmosphere colour.
    pub atmosphere_b_colour: Vec3,
    /// v457, the second atmosphere colour's scale.
    pub atmosphere_b_scale: f32,
    /// v458, the night boost of both atmosphere colours.
    pub atmosphere_boost: f32,
    /// v459, the sun disc size.
    pub sun_size: f32,
    /// v461, row +0x194.
    pub row_194: f32,
    /// v467..v469, the moon colour (inferred).
    pub moon_colour: Vec3,
    /// v470, the moon disc size.
    pub moon_size: f32,
    /// v471, row +0x164.
    pub row_164: f32,
    /// v472, stars on when at or above the gate threshold.
    pub stars_on: f32,
    /// v473, the point stars' strength.
    pub point_star_strength: f32,
    /// v474, the point stars' size.
    pub point_star_size: f32,
    /// v475..v477, the strengths of the cloud, dense and sparse star layers.
    pub layer_strength: [f32; 3],
    /// v478, the twinkle phase speed.
    pub twinkle_rate: f32,
    /// v479, divisor term of +0xec.
    pub row_ec: f32,
    /// v480, divisor term of +0x10c.
    pub row_10c: f32,
    /// v481, the night boost of the point stars' strength.
    pub point_star_strength_boost: f32,
    /// v482, the night boost of the point stars' size.
    pub point_star_size_boost: f32,
    /// v483..v485, the night boosts of the cloud, dense and sparse star layers.
    pub layer_boost: [f32; 3],
}

impl SkyVariables {
    /// Reads the variables from a timecycle frame by index; None when the frame is shorter than 486 floats.
    pub fn from_frame(frame: &[f32]) -> Option<SkyVariables> {
        if frame.len() <= 485 {
            return None;
        }
        Some(SkyVariables {
            sun_disc_gate: frame[428],
            moon_disc_gate: frame[430],
            row_fc: frame[448],
            atmosphere_a_colour: Vec3::new(frame[450], frame[451], frame[452]),
            atmosphere_a_scale: frame[453],
            atmosphere_b_colour: Vec3::new(frame[454], frame[455], frame[456]),
            atmosphere_b_scale: frame[457],
            atmosphere_boost: frame[458],
            sun_size: frame[459],
            row_194: frame[461],
            moon_colour: Vec3::new(frame[467], frame[468], frame[469]),
            moon_size: frame[470],
            row_164: frame[471],
            stars_on: frame[472],
            point_star_strength: frame[473],
            point_star_size: frame[474],
            layer_strength: [frame[475], frame[476], frame[477]],
            twinkle_rate: frame[478],
            row_ec: frame[479],
            row_10c: frame[480],
            point_star_strength_boost: frame[481],
            point_star_size_boost: frame[482],
            layer_boost: [frame[483], frame[484], frame[485]],
        })
    }
}

/// Tunables the dome reads; the image holds only these defaults and a data file may replace any of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyTunables {
    /// Twinkle phase multiplier.
    pub twinkle_scale: f32,
    /// Row +0x1c.
    pub row_1c: f32,
    /// The w lanes of the cloud, dense and sparse layer rows.
    pub layer_w: [f32; 3],
    /// Colours of the cloud, dense and sparse star layers.
    pub layer_colour: [Vec3; 3],
    /// Strengths of the cloud, dense and sparse layers: x at W3 = 0, y at W3 = 1.
    pub layer_night_strength: [Vec2; 3],
    /// Multiplied by v479 to give the divisor of +0xec (an inverse scale height: inferred).
    pub scale_a: f32,
    /// Multiplied by v480 to give the divisor of +0x10c (an inverse scale height: inferred).
    pub scale_b: f32,
    /// Row +0x11c.
    pub row_11c: f32,
    /// Row +0x168.
    pub row_168: f32,
    /// Row +0x16c.
    pub row_16c: f32,
    /// Row +0x17c.
    pub row_17c: f32,
    /// Row +0x18c.
    pub row_18c: f32,
    /// Air glow colour at W3 = 0 (the base ambient star colour).
    pub air_glow: Vec3,
    /// Air glow colour at W3 = 1 (the galaxy's base ambient star colour).
    pub air_glow_galaxy: Vec3,
    /// Point-star size multiplier.
    pub point_star_size: f32,
    /// Point-star strength: x at W3 = 0, y at W3 = 1.
    pub point_star_night_strength: Vec2,
}

impl SkyTunables {
    /// The image defaults; every colour is black there.
    pub fn image_defaults() -> SkyTunables {
        SkyTunables {
            twinkle_scale: 2.0,
            row_1c: 4.0,
            layer_w: [4.0, 4.0, 0.5],
            layer_colour: [Vec3::ZERO; 3],
            layer_night_strength: [
                Vec2::new(0.03, 0.03),
                Vec2::new(7.5, 7.5),
                Vec2::new(120.0, 120.0),
            ],
            scale_a: 4000.0,
            scale_b: 24000.0,
            row_11c: 1.5,
            row_168: 8.0,
            row_16c: 0.5,
            row_17c: 4.0,
            row_18c: 4.0,
            air_glow: Vec3::ZERO,
            air_glow_galaxy: Vec3::ZERO,
            point_star_size: 1.0,
            point_star_night_strength: Vec2::new(1.0, 1.0),
        }
    }
}

/// The dome call's mode and the two flags the block reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomeCall {
    /// Low four bits of the mode word; the main view uses 0.
    pub mode: u32,
    /// Flag A: +0xfc is written 0.
    pub suppress_fc: bool,
    /// Flag B: point stars are drawn and the air glow is left out of +0xf0.
    pub point_stars: bool,
}

impl DomeCall {
    /// Splits the call's mode word and flag word.
    pub fn from_words(mode_word: u32, flag_word: u32) -> DomeCall {
        DomeCall {
            mode: mode_word & MODE_MASK,
            suppress_fc: flag_word & FLAG_SUPPRESS_FC != 0,
            point_stars: flag_word & FLAG_POINT_STARS != 0,
        }
    }
}

/// The per-view sky matrix as its table holds it; the rotation's own arithmetic from the tilt and azimuth was not read in full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyMatrix {
    /// The four rotation rows, copied to +0x50..+0x8f.
    pub rotation_rows: [Vec4; 4],
    /// The fourth row, copied to +0xc0.
    pub fourth_row: Vec4,
}

/// The two bodies and the night weight the block reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bodies {
    /// Unit sun disc direction: the first result of sun_moon.rs::disc_directions, render copy.
    pub sun: Vec3,
    /// Unit moon disc direction: the second result of sun_moon.rs::disc_directions, render copy.
    pub moon: Vec3,
    /// W3, the weight with neither body up: Weights::third of sun_moon.rs::body_weights.
    pub night_weight: f32,
}

/// What the block takes from the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DomeView {
    /// Camera position, world units.
    pub camera: Vec3,
    /// Z0, a world height the build registers as the water level (inferred).
    pub ground_height: f32,
    /// The view's sky matrix.
    pub sky: SkyMatrix,
    /// Width of the texture row +0x19c divides by, None when it is absent.
    pub texture_width: Option<u16>,
}

/// The dome block, row by row; to_bytes lays it out for upload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyDomeBlock {
    /// +0x00, the sun disc direction.
    pub sun_direction: Vec3,
    /// +0x0c, v478 x tunable x fmod(seconds, 2 pi) (a twinkle: inferred).
    pub twinkle_phase: f32,
    /// +0x10, the moon disc direction.
    pub moon_direction: Vec3,
    /// +0x1c, a tunable.
    pub row_1c: f32,
    /// +0x20, the cloud star layer colour and its w tunable.
    pub cloud_layer: Vec4,
    /// +0x30, the dense star layer colour and its w tunable.
    pub dense_layer: Vec4,
    /// +0x40, the sparse star layer colour and its w tunable.
    pub sparse_layer: Vec4,
    /// +0x50..+0x8f, the sky rotation rows.
    pub sky_rotation: [Vec4; 4],
    /// +0x90..+0xbf, the inverse of the rotation's upper 3x3, by row.
    pub sky_inverse: [Vec3; 3],
    /// +0xc0, the sky matrix's fourth row.
    pub sky_fourth_row: Vec4,
    /// +0xd0, P0 = (cam.x, cam.y, Z0).
    pub ground_point: Vec3,
    /// +0xdc, 1.0 when v472 >= 1e-4 and flag B, else 0.0 (whatever the star count).
    pub point_stars_on: f32,
    /// +0xe0, (v450..v452) x v453 x m / 1000.
    pub atmosphere_a: Vec3,
    /// +0xec, 1 / (v479 x tunable).
    pub inverse_scale_a: f32,
    /// +0xf0, (v454..v456) x v457 x m / 1000 plus the air glow.
    pub atmosphere_b: Vec3,
    /// +0xfc, v448, or 0 with flag A.
    pub row_fc: f32,
    /// +0x100, P0 + 20000 x sun direction.
    pub sun_position: Vec3,
    /// +0x10c, 1 / (v480 x tunable).
    pub inverse_scale_b: f32,
    /// +0x110, P = P0 + 20000 x moon direction.
    pub moon_position: Vec3,
    /// +0x11c, a tunable.
    pub row_11c: f32,
    /// +0x120, R = normalize((0, 0, 1) x D).
    pub moon_right: Vec3,
    /// +0x130, D = normalize(camera - P).
    pub moon_forward: Vec3,
    /// +0x140, D x R.
    pub moon_up: Vec3,
    /// +0x160, cosine of the moon's angular radius.
    pub moon_cos_radius: f32,
    /// +0x164, v471.
    pub row_164: f32,
    /// +0x168, a tunable.
    pub row_168: f32,
    /// +0x16c, a tunable.
    pub row_16c: f32,
    /// +0x170, L = normalize(149600 x sun - 384 x moon).
    pub moon_lit_direction: Vec3,
    /// +0x17c, a tunable.
    pub row_17c: f32,
    /// +0x180, v467..v469.
    pub moon_colour: Vec3,
    /// +0x18c, a tunable.
    pub row_18c: f32,
    /// +0x190, always 0.8.
    pub row_190: f32,
    /// +0x194, v461.
    pub row_194: f32,
    /// +0x198, 1 / (250 x v459)^2.
    pub sun_inverse_radius_squared: f32,
    /// +0x19c, 1 / the texture width, 0 when absent.
    pub inverse_texture_width: f32,
}

fn put(bytes: &mut [u8; BLOCK_BYTES], offset: usize, values: &[f32]) {
    for (lane, value) in values.iter().enumerate() {
        let at = offset + 4 * lane;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
}

impl SkyDomeBlock {
    /// The block's 0x1a0 little-endian bytes; every byte no row names is 0, as the game zeroes the block first.
    pub fn to_bytes(self) -> [u8; BLOCK_BYTES] {
        let mut bytes = [0u8; BLOCK_BYTES];
        put(&mut bytes, SUN_DIRECTION, &self.sun_direction.to_array());
        put(&mut bytes, TWINKLE_PHASE, &[self.twinkle_phase]);
        put(&mut bytes, MOON_DIRECTION, &self.moon_direction.to_array());
        put(&mut bytes, ROW_1C, &[self.row_1c]);
        put(&mut bytes, CLOUD_LAYER, &self.cloud_layer.to_array());
        put(&mut bytes, DENSE_LAYER, &self.dense_layer.to_array());
        put(&mut bytes, SPARSE_LAYER, &self.sparse_layer.to_array());
        for (row, value) in self.sky_rotation.iter().enumerate() {
            put(&mut bytes, SKY_ROTATION + 16 * row, &value.to_array());
        }
        // Trap: the w lanes of the inverse rows were not read; they stay 0.
        for (row, value) in self.sky_inverse.iter().enumerate() {
            put(&mut bytes, SKY_INVERSE + 16 * row, &value.to_array());
        }
        put(&mut bytes, SKY_FOURTH_ROW, &self.sky_fourth_row.to_array());
        put(&mut bytes, GROUND_POINT, &self.ground_point.to_array());
        put(&mut bytes, POINT_STARS_ON, &[self.point_stars_on]);
        put(&mut bytes, ATMOSPHERE_A, &self.atmosphere_a.to_array());
        put(&mut bytes, INVERSE_SCALE_A, &[self.inverse_scale_a]);
        put(&mut bytes, ATMOSPHERE_B, &self.atmosphere_b.to_array());
        put(&mut bytes, ROW_FC, &[self.row_fc]);
        put(&mut bytes, SUN_POSITION, &self.sun_position.to_array());
        put(&mut bytes, INVERSE_SCALE_B, &[self.inverse_scale_b]);
        put(&mut bytes, MOON_POSITION, &self.moon_position.to_array());
        put(&mut bytes, ROW_11C, &[self.row_11c]);
        // Trap: the game carries a w lane with D at +0x13c whose value was not identified, and the
        // w lanes at +0x12c and +0x14c were not read; all three are written 0.
        put(&mut bytes, MOON_RIGHT, &self.moon_right.to_array());
        put(&mut bytes, MOON_FORWARD, &self.moon_forward.to_array());
        put(&mut bytes, MOON_UP, &self.moon_up.to_array());
        put(&mut bytes, MOON_COS_RADIUS, &[self.moon_cos_radius]);
        put(&mut bytes, ROW_164, &[self.row_164]);
        put(&mut bytes, ROW_168, &[self.row_168]);
        put(&mut bytes, ROW_16C, &[self.row_16c]);
        put(
            &mut bytes,
            MOON_LIT_DIRECTION,
            &self.moon_lit_direction.to_array(),
        );
        put(&mut bytes, ROW_17C, &[self.row_17c]);
        put(&mut bytes, MOON_COLOUR, &self.moon_colour.to_array());
        put(&mut bytes, ROW_18C, &[self.row_18c]);
        put(&mut bytes, ROW_190, &[self.row_190]);
        put(&mut bytes, ROW_194, &[self.row_194]);
        put(
            &mut bytes,
            SUN_INVERSE_RADIUS_SQUARED,
            &[self.sun_inverse_radius_squared],
        );
        put(
            &mut bytes,
            INVERSE_TEXTURE_WIDTH,
            &[self.inverse_texture_width],
        );
        bytes
    }
}

/// The air glow added to +0xf0: lerp(air_glow, air_glow_galaxy, W3) when stars are on and flag B is clear, else 0.
pub fn air_glow(
    variables: &SkyVariables,
    tunables: &SkyTunables,
    night_weight: f32,
    call: DomeCall,
) -> Vec3 {
    if variables.stars_on >= GATE_THRESHOLD && !call.point_stars {
        tunables
            .air_glow
            .lerp(tunables.air_glow_galaxy, night_weight)
    } else {
        Vec3::ZERO
    }
}

/// The twinkle phase v478 x tunable x fmod(ms x 0.001, 2 pi), from the ms counter.
pub fn twinkle_phase(rate: f32, scale: f32, ms_counter: u32) -> f32 {
    // Trap: the counter goes to f32 before x 0.001, so above 2^24 ms (4.7 hours) whole ms are lost;
    // the width of the game's conversion was not read.
    let seconds = ms_counter as f32 * SECONDS_PER_TICK;
    rate * scale * (seconds % std::f32::consts::TAU)
}

/// The moon's lit fraction 0.5 x (1 + L . -moon), from the lit direction at +0x170 and the moon direction.
pub fn moon_lit_fraction(lit_direction: Vec3, moon: Vec3) -> f32 {
    0.5 * (1.0 + lit_direction.dot(-moon))
}

/// Rows of the inverse of a matrix's upper 3x3 by cofactors over the determinant; zero rows when it is singular.
fn inverse_rows(rows: &[Vec4; 4]) -> [Vec3; 3] {
    let matrix =
        Mat3::from_cols(rows[0].truncate(), rows[1].truncate(), rows[2].truncate()).transpose();
    let determinant = matrix.determinant();
    // The game's handling of a singular matrix was not read; a rotation never is one.
    if !determinant.is_finite() || determinant.abs() < f32::MIN_POSITIVE {
        return [Vec3::ZERO; 3];
    }
    let inverse = matrix.inverse();
    [inverse.row(0), inverse.row(1), inverse.row(2)]
}

fn star_layer(
    colour: Vec3,
    night_strength: Vec2,
    boost: f32,
    strength: f32,
    w: f32,
    night_weight: f32,
) -> Vec4 {
    let scale = lerp(night_strength.x, night_strength.y, night_weight)
        * night_boost(night_weight, boost)
        * strength;
    (colour * scale).extend(w)
}

/// Cosine of the moon's angular radius as the block computes it, from the moon position P and v470.
fn moon_cos_radius(moon_position: Vec3, moon_size: f32) -> f32 {
    // n is taken from the world position P, not from P relative to the camera.
    let n = moon_position.normalize_or_zero();
    // The game's test for "n is up" was not read; this falls back only when the cross cannot be normalised.
    let tangent = n.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
    let rim = (moon_position + tangent * DISC_RADIUS_PER_SIZE * moon_size).normalize_or_zero();
    n.dot(rim)
}

/// The dome block for one call, uploaded before the call draws its discs and then its point stars.
pub fn fill_dome_block(
    variables: &SkyVariables,
    tunables: &SkyTunables,
    bodies: &Bodies,
    view: &DomeView,
    call: DomeCall,
    ms_counter: u32,
) -> SkyDomeBlock {
    let w3 = bodies.night_weight;
    let boost = night_boost(w3, variables.atmosphere_boost);
    let ground_point = Vec3::new(view.camera.x, view.camera.y, view.ground_height);
    let sun_position = ground_point + bodies.sun * DOME_DISTANCE;
    let moon_position = ground_point + bodies.moon * DOME_DISTANCE;
    // Trap: with the moon straight overhead D is vertical, so R and D x R are zero; the game's
    // result there was not read.
    let forward = (view.camera - moon_position).normalize_or_zero();
    let right = Vec3::Z.cross(forward).normalize_or_zero();
    let stars_drawn = variables.stars_on >= GATE_THRESHOLD && call.point_stars;
    let sun_radius = DISC_RADIUS_PER_SIZE * variables.sun_size;
    let inverse_texture_width = match view.texture_width {
        // A zero width is taken as absent; the game's handling of it was not read.
        Some(width) if width > 0 => 1.0 / f32::from(width),
        _ => 0.0,
    };
    SkyDomeBlock {
        sun_direction: bodies.sun,
        twinkle_phase: twinkle_phase(variables.twinkle_rate, tunables.twinkle_scale, ms_counter),
        moon_direction: bodies.moon,
        row_1c: tunables.row_1c,
        cloud_layer: star_layer(
            tunables.layer_colour[0],
            tunables.layer_night_strength[0],
            variables.layer_boost[0],
            variables.layer_strength[0],
            tunables.layer_w[0],
            w3,
        ),
        dense_layer: star_layer(
            tunables.layer_colour[1],
            tunables.layer_night_strength[1],
            variables.layer_boost[1],
            variables.layer_strength[1],
            tunables.layer_w[1],
            w3,
        ),
        sparse_layer: star_layer(
            tunables.layer_colour[2],
            tunables.layer_night_strength[2],
            variables.layer_boost[2],
            variables.layer_strength[2],
            tunables.layer_w[2],
            w3,
        ),
        sky_rotation: view.sky.rotation_rows,
        sky_inverse: inverse_rows(&view.sky.rotation_rows),
        sky_fourth_row: view.sky.fourth_row,
        ground_point,
        point_stars_on: if stars_drawn { 1.0 } else { 0.0 },
        atmosphere_a: variables.atmosphere_a_colour * variables.atmosphere_a_scale * boost
            / ATMOSPHERE_DIVISOR,
        // Trap: a zero v479 or v480 gives infinity, as the game's division does.
        inverse_scale_a: 1.0 / (variables.row_ec * tunables.scale_a),
        atmosphere_b: variables.atmosphere_b_colour * variables.atmosphere_b_scale * boost
            / ATMOSPHERE_DIVISOR
            + air_glow(variables, tunables, w3, call),
        row_fc: if call.suppress_fc {
            0.0
        } else {
            variables.row_fc
        },
        sun_position,
        inverse_scale_b: 1.0 / (variables.row_10c * tunables.scale_b),
        moon_position,
        row_11c: tunables.row_11c,
        moon_right: right,
        moon_forward: forward,
        moon_up: forward.cross(right),
        moon_cos_radius: moon_cos_radius(moon_position, variables.moon_size),
        row_164: variables.row_164,
        row_168: tunables.row_168,
        row_16c: tunables.row_16c,
        moon_lit_direction: (bodies.sun * SUN_DISTANCE - bodies.moon * MOON_DISTANCE)
            .normalize_or_zero(),
        row_17c: tunables.row_17c,
        moon_colour: variables.moon_colour,
        row_18c: tunables.row_18c,
        row_190: ROW_190_VALUE,
        row_194: variables.row_194,
        // Trap: a zero v459 gives infinity here too; the game's handling past its division was not read.
        sun_inverse_radius_squared: 1.0 / (sun_radius * sun_radius),
        inverse_texture_width,
    }
}

/// One vertex of a disc fan: position in the units of the dome distance, and UV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FanVertex {
    /// (r cos a, 0, r sin a); the centre is the origin.
    pub position: Vec3,
    /// 0.5 + k (cos a, sin a); the centre is (0.5, 0.5).
    pub uv: Vec2,
}

/// A disc to draw: its radius and UV scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiscFan {
    /// Radius in world units at the dome distance.
    pub radius: f32,
    /// UV scale k of the rim.
    pub uv_scale: f32,
}

/// The 90 vertices of a disc fan: the centre, then rim points at i x 2 pi / 88 for i in 0..=88.
pub fn fan_vertices(fan: DiscFan) -> [FanVertex; FAN_VERTICES] {
    let mut vertices = [FanVertex {
        position: Vec3::ZERO,
        uv: Vec2::splat(FAN_UV_CENTRE),
    }; FAN_VERTICES];
    for (index, vertex) in vertices.iter_mut().enumerate().skip(1) {
        let (sin, cos) = ((index - 1) as f32 * RIM_STEP).sin_cos();
        vertex.position = Vec3::new(fan.radius * cos, 0.0, fan.radius * sin);
        vertex.uv = Vec2::splat(FAN_UV_CENTRE) + Vec2::new(cos, sin) * fan.uv_scale;
    }
    vertices
}

fn disc(direction_z: f32, gate: f32, size: f32, uv_scale: f32, call: DomeCall) -> Option<DiscFan> {
    // Every non-zero mode skips both discs (modes 1 to 3 are the ones seen).
    if call.mode == 0 && direction_z >= DISC_MIN_Z && gate >= GATE_THRESHOLD {
        Some(DiscFan {
            radius: DISC_RADIUS_PER_SIZE * size,
            uv_scale,
        })
    } else {
        None
    }
}

/// The sun disc of a call, None when its gates leave it undrawn.
pub fn sun_disc(variables: &SkyVariables, sun: Vec3, call: DomeCall) -> Option<DiscFan> {
    disc(
        sun.z,
        variables.sun_disc_gate,
        variables.sun_size,
        SUN_UV_SCALE,
        call,
    )
}

/// The moon disc of a call, None when its gates leave it undrawn.
pub fn moon_disc(variables: &SkyVariables, moon: Vec3, call: DomeCall) -> Option<DiscFan> {
    disc(
        moon.z,
        variables.moon_disc_gate,
        variables.moon_size,
        MOON_UV_SCALE,
        call,
    )
}

/// Angular radius in radians of a disc of size v (v459 or v470): atan(250 v / 20000).
pub fn disc_angular_radius(size: f32) -> f32 {
    (DISC_RADIUS_PER_SIZE * size / DOME_DISTANCE).atan()
}

/// One catalogue star: colour, polar angle A from up, azimuth B, size and intensity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointStar {
    /// RGB.
    pub colour: Vec3,
    /// Polar angle from up, radians.
    pub polar: f32,
    /// Azimuth, radians.
    pub azimuth: f32,
    /// Size before the frame's scales.
    pub size: f32,
    /// Intensity; a star at or below 0 is skipped.
    pub intensity: f32,
}

/// The frame's scales of every point star's size and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointStarScales {
    /// max(1, W3 x v482) x v474 x tunable.
    pub size: f32,
    /// max(1, W3 x v481) x v473 x lerp of two tunables by W3.
    pub colour: f32,
}

/// The point stars' scales for a frame.
pub fn point_star_scales(
    variables: &SkyVariables,
    tunables: &SkyTunables,
    night_weight: f32,
) -> PointStarScales {
    PointStarScales {
        size: night_boost(night_weight, variables.point_star_size_boost)
            * variables.point_star_size
            * tunables.point_star_size,
        colour: night_boost(night_weight, variables.point_star_strength_boost)
            * variables.point_star_strength
            * lerp(
                tunables.point_star_night_strength.x,
                tunables.point_star_night_strength.y,
                night_weight,
            ),
    }
}

/// Whether a call draws point stars: stars on, flag B and a non-zero draw count (whether that is the list length or the stars written after the intensity skip was not read).
pub fn point_stars_drawn(variables: &SkyVariables, call: DomeCall, count: usize) -> bool {
    variables.stars_on >= GATE_THRESHOLD && call.point_stars && count > 0
}

/// At most MAX_POINT_STARS from a list of this length; whether the cap counts records read or stars written was not read.
pub fn point_star_count(list_length: usize) -> usize {
    list_length.min(MAX_POINT_STARS)
}

/// A star's position (w = size) and colour (w = 0), None when its intensity is at or below 0.
pub fn point_star_vertex(star: &PointStar, scales: &PointStarScales) -> Option<(Vec4, Vec4)> {
    if star.intensity <= 0.0 {
        return None;
    }
    let (sin_polar, cos_polar) = star.polar.sin_cos();
    let (sin_azimuth, cos_azimuth) = star.azimuth.sin_cos();
    let position =
        Vec3::new(cos_azimuth * sin_polar, sin_azimuth * sin_polar, cos_polar) * DOME_DISTANCE;
    let colour = star.colour * star.intensity * scales.colour;
    Some((position.extend(star.size * scales.size), colour.extend(0.0)))
}

#[cfg(test)]
#[path = "sky_dome_tests.rs"]
mod tests;
