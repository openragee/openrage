//! The arithmetic of map and LOD lights, their GPU record terms, coronas, the shadow direction and the sun's shadow cascades, with no I/O.
//! Left out: the corona's reflection-phase, bit-11 and mode-2 near fades and its bit 4 and 9 multipliers (runtime values), the cascades' depth extension and margin, and the first-person and vehicle lambda overrides.

use glam::{Vec3, Vec4};

/// Hour a night lamp starts to fade on, before its offset, hours (the build's value at rest).
pub const NIGHT_ON_HOUR: f64 = 18.0;
/// Hour a night lamp has faded off by, before its offset, hours (the build's value at rest).
pub const NIGHT_OFF_HOUR: f64 = 6.0;
/// Least fade duration of the night window, hours.
const NIGHT_MIN_FADE_HOURS: f64 = 1e-6;
/// Light attribute flag: the lamp follows the night window.
pub const ATTR_NIGHT_BIT: u32 = 1 << 15;
/// Light attribute flag: the lamp follows the inverse of the night window.
pub const ATTR_DAY_BIT: u32 = 1 << 16;

/// Largest spot outer angle, degrees.
const SPOT_MAX_OUTER_DEG: f32 = 89.0;
/// Smallest spot inner angle, degrees.
const SPOT_MIN_INNER_DEG: f32 = 1.0;
/// Least gap between a spot's inner and outer angles, degrees.
const SPOT_INNER_GAP_DEG: f32 = 1.0;
/// Least 1 - cos(outer) in the spot normalisation, unitless.
const SPOT_MIN_ONE_MINUS_COS: f32 = 1e-5;
/// Largest cos(inner) - cos(outer) of a degenerate spot, whose record term was not read.
const SPOT_MIN_COS_GAP: f32 = 1e-6;
/// Light flags-2 bit that divides a capsule's intensity by (length + 1) squared.
const CAPSULE_NORMALISE_BIT: u32 = 1 << 17;
/// Least rectangle width or height in a GPU record, metres.
const RECT_MIN_EXTENT: f32 = 0.001;

/// Slot of a GPU record's index when the light has none.
pub const NO_SLOT: u16 = 0xffff;
/// Low byte of a GPU record's index when the light's array is empty.
pub const NO_SLOT_BYTE: u8 = 0xff;

/// Degrees per unit of a LOD spot cone byte.
const LOD_CONE_DEG_PER_UNIT: f32 = 180.0 / 255.0;
/// Metres per unit of a LOD capsule length byte.
const LOD_CAPSULE_M_PER_UNIT: f32 = 140.0 / 255.0;
/// Metres per unit of a LOD rectangle extent byte.
const LOD_RECT_M_PER_UNIT: f32 = 32.0 / 255.0;
/// First bit of the category in a LOD light's word.
const LOD_CATEGORY_SHIFT: u32 = 22;
/// Mask of the three category bits after the shift.
const LOD_CATEGORY_MASK: u32 = 0x7;
/// Number of categories that have a multiplier.
const LOD_CATEGORY_COUNT: usize = 6;
/// Least |f| a LOD light's volume term is divided by.
const LOD_VOLUME_MIN_DIVISOR: f32 = 1e-4;
/// Seconds per millisecond of the animation clock.
const ANIM_SECONDS_PER_MS: f32 = 0.001;
/// Factor on time and phase in the animation u, unitless.
const ANIM_U_RATE: f32 = 30.0;
/// Texel rows per animation row.
const ANIM_ROW_STRIDE: f32 = 12.0;
/// Sign bit of a LOD light's animation row, which selects the raw path.
const ANIM_ROW_RAW_BIT: u16 = 0x8000;

/// Range and added term of LOD spheres 0..2, metres; the added term is a fade width, inferred.
const LOD_SPHERES: [LodSphere; 3] = [
    LodSphere {
        range: 500.0,
        margin: 20.0,
    },
    LodSphere {
        range: 1500.0,
        margin: 100.0,
    },
    LodSphere {
        range: 3500.0,
        margin: 150.0,
    },
];
/// Range of distant-light spheres 0..2, metres.
pub const DISTANT_LIGHT_RANGE: f32 = 5000.0;
/// First day-phase hour of the range cap.
const DAY_CAP_FIRST_HOUR: i64 = 7;
/// Last day-phase hour of the range cap (the cap holds to 16:59).
const DAY_CAP_LAST_HOUR: i64 = 16;
/// Range cap of the LOD and distant culls and the main light cull's per-light reach inside the day hours, metres.
pub const DAY_RANGE_CAP: f32 = 400.0;
/// Minutes in a day.
const MINUTES_PER_DAY: f64 = 1440.0;

/// Corona mode in which the size follows the cone weight and depth.
const CORONA_MODE_DEPTH: u32 = 0;
/// Corona mode that takes the mode-2 size scale.
const CORONA_MODE_SCALED: u32 = 2;
/// Corona mode that never uses screen-constant size.
const CORONA_MODE_WORLD_ONLY: u32 = 4;
/// Denominator constant of the corona cone remap, just above 1 so a zero angle stays finite.
const CORONA_CONE_ONE: f32 = 1.0001;

/// Tilt of the shadow direction toward the zenith, degrees (a global the build sets once; it is > 0, so the tilt applies).
const SHADOW_TILT_DEG: f32 = 0.265;
/// Shadow filter radius at softness 0.
const SHADOW_FILTER_MIN: f32 = 0.53;
/// Shadow filter radius at softness 1.
const SHADOW_FILTER_MAX: f32 = 10.0;

/// Most sun shadow cascades.
pub const MAX_CASCADES: usize = 4;
/// Near split of the linear set, metres.
const CASCADE_LINEAR_NEAR: f32 = 0.5;
/// Far split of the linear set, metres (176.551712).
const CASCADE_LINEAR_FAR: f32 = f32::from_bits(0x4330_8d3d);
/// Span the linear inner splits are spaced over, metres (176.051712).
const CASCADE_LINEAR_SPAN: f32 = f32::from_bits(0x4330_0d3d);
/// Near split once the log blend runs with more than three cascades, metres.
const CASCADE_BLENDED_NEAR: f32 = 0.05;
/// Log blend lambda when the script sets none, unitless.
pub const CASCADE_DEFAULT_LAMBDA: f32 = 0.9;
/// Least final directional intensity that casts sun shadows, unitless.
pub const SHADOW_MIN_SUN_INTENSITY: f32 = 0.0005;
/// Split factor while the sun casts no shadow, unitless.
const CASCADE_NO_SUN_FACTOR: f32 = 0.001;
/// Factor on the splits from the array size on in the boosted vehicle and camera cases, unitless.
const CASCADE_FAR_BOOST: f32 = 1.75;
/// Least tracked radius of an aircraft, metres.
const AIRCRAFT_MIN_RADIUS: f32 = 2.0;
/// Tracked radius where both ramps start, metres.
const TRACKER_RADIUS_START: f32 = 1.2;
/// Tracked radius where the far scale ramp ends, metres.
const TRACKER_SCALE_RADIUS_END: f32 = 4.0;
/// Tracked radius where the lambda ramp ends, metres.
const TRACKER_LAMBDA_RADIUS_END: f32 = 6.0;
/// Far scale at the end of its ramp, unitless.
const TRACKER_SCALE_MAX: f32 = 2.5;
/// Lambda factor at the start of its ramp, unitless.
const TRACKER_LAMBDA_START: f32 = 0.96;
/// Lambda factor at the end of its ramp, unitless.
const TRACKER_LAMBDA_END: f32 = 0.89;
/// Factor that undoes the default lambda in the tracked lambda (1 / 0.9), unitless.
const TRACKER_LAMBDA_RENORMALISE: f32 = 1.0 / 0.9;
/// Fraction of the way to its target the far scale moves each frame.
const TRACKER_EASE_PER_FRAME: f32 = 0.1;
/// Slope of the steep-camera ramp per unit of |forward z|, unitless.
const STEEP_CAMERA_SLOPE: f32 = 4.0;
/// Offset of the steep-camera ramp; 1.4 / 4 = 0.35 is the |forward z| where it starts, unitless.
const STEEP_CAMERA_OFFSET: f32 = 1.4;
/// Tracker scale taken off at the full steep-camera ramp, unitless.
const STEEP_CAMERA_TRACKER_LOSS: f32 = 0.2;
/// Lambda taken off at the full steep-camera ramp, unitless.
const STEEP_CAMERA_LAMBDA_LOSS: f32 = 0.45;
/// Fewest active cascades.
const MIN_ACTIVE_CASCADES: usize = 2;
/// Fraction of a cascade below which the cascade-count variable's fraction is dropped.
const CASCADE_FRACTION_SNAP: f32 = 0.005;
/// Slope of the far fade per unit of distance over reach; the fade spans the last 1 / slope.
const CASCADE_FADE_SLOPE: f32 = 20.0;

#[allow(clippy::manual_clamp)]
fn saturate(x: f32) -> f32 {
    // max then min maps NaN to 0, as saturate does; f32::clamp keeps NaN.
    x.max(0.0).min(1.0)
}

#[allow(clippy::manual_clamp)]
fn saturate_f64(x: f64) -> f64 {
    // max then min maps NaN to 0, as saturate does; f64::clamp keeps NaN.
    x.max(0.0).min(1.0)
}

fn unit_to_byte(x: f32) -> u8 {
    (x * 255.0 + 0.5).trunc().clamp(0.0, 255.0) as u8
}

/// Smootherstep 6t^5 - 15t^4 + 10t^3 of x saturated to 0..1, unitless.
pub fn smootherstep(x: f64) -> f64 {
    let t = saturate_f64(x);
    t * t * t * (t * (6.0 * t - 15.0) + 10.0)
}

/// Clock hour from whole hours, minutes and seconds, hours.
pub fn clock_hour(hours: i32, minutes: i32, seconds: i32) -> f64 {
    f64::from(hours) + (f64::from(minutes) + f64::from(seconds) / 60.0) / 60.0
}

/// Night amount of a lamp at a clock hour, 0 by day to 1 by night; duration and offset in minutes.
pub fn night_amount(hour: f64, duration_minutes: f32, offset_minutes: f32) -> f64 {
    let duration = f64::from(duration_minutes) / 60.0;
    let offset = f64::from(offset_minutes) / 60.0;
    let span = duration.max(NIGHT_MIN_FADE_HOURS);
    let on = NIGHT_ON_HOUR + offset;
    let off = NIGHT_OFF_HOUR - duration - offset;
    smootherstep((hour - on) / span).max(1.0 - smootherstep((hour - off) / span))
}

/// Hour factor of a map light from its attribute flags (bits 15, 16), 0 off to 1 on.
pub fn lamp_hour_factor(
    attr_flags: u32,
    hour: f64,
    duration_minutes: f32,
    offset_minutes: f32,
) -> f32 {
    let night_bit = (attr_flags & ATTR_NIGHT_BIT) != 0;
    let day_bit = (attr_flags & ATTR_DAY_BIT) != 0;
    let factor = match (night_bit, day_bit) {
        (true, false) => night_amount(hour, duration_minutes, offset_minutes),
        (false, true) => 1.0 - night_amount(hour, duration_minutes, offset_minutes),
        (false, false) => 1.0,
        (true, true) => 0.0,
    };
    factor as f32
}

/// A map light's intensity factors; the bits 18/19 factor, the clock factor, the per-object float and the final factor are undecoded knobs at 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapLightChain {
    /// The light attribute's intensity (attribute +0x2c).
    pub attr_intensity: f32,
    /// Factor of attribute +0xf0 bits 18 and 19: 1 when both are clear, the other cases not decoded.
    pub attr_bits_factor: f32,
    /// Whether the entity's per-object handle (entity +0xbc) is -1, which applies the clock terms.
    pub no_object_handle: bool,
    /// Clock factor of attribute +0x9c, not decoded.
    pub clock_factor: f32,
    /// Night window factor from `lamp_hour_factor`.
    pub hour_factor: f32,
    /// Global light intensity v240; whether it also applies with a handle was not settled.
    pub global_intensity: f32,
    /// v374 when the owner is a vehicle and attribute +0xf0 is not negative.
    pub vehicle: Option<f32>,
    /// v286 when bits 1..5 of entity +0xb4 are 0xf or 0xc.
    pub entity_state: Option<f32>,
    /// v310 when attribute +0xf0 bit 2 is set.
    pub attr_bit2: Option<f32>,
    /// v302 when the setup bit 0x10 is set.
    pub setup_bit: Option<f32>,
    /// Per-object float of the handle entity +0xbc.
    pub per_object: f32,
    /// Undecoded factor on the final intensity.
    pub final_factor: f32,
    /// Owning entity's alpha e+0x90 after stream_math's LOD-tree step (record +0x370 = alpha / 255, a factor on the intensity, inferred); 255 for a light with no owner or an owner in the scan's far list (inferred).
    pub entity_alpha: u8,
}

impl MapLightChain {
    /// Light intensity; the light is added only when it is positive.
    pub fn intensity(&self) -> f32 {
        let mut base = self.attr_bits_factor;
        if self.no_object_handle {
            base *= self.clock_factor * self.hour_factor * self.global_intensity;
        }
        let flagged = [
            self.vehicle,
            self.entity_state,
            self.attr_bit2,
            self.setup_bit,
        ];
        for factor in flagged.into_iter().flatten() {
            base *= factor;
        }
        self.attr_intensity
            * base
            * self.per_object
            * self.final_factor
            * (f32::from(self.entity_alpha) / 255.0)
    }
}

/// Spot cone angles after the clamps, (inner, outer) in degrees.
pub fn clamp_spot_cone(inner_deg: f32, outer_deg: f32) -> (f32, f32) {
    let outer = outer_deg.min(SPOT_MAX_OUTER_DEG);
    let inner = if inner_deg < SPOT_MIN_INNER_DEG {
        SPOT_MIN_INNER_DEG
    } else {
        inner_deg.min(outer - SPOT_INNER_GAP_DEG)
    };
    (inner, outer)
}

/// Cosines (inner, outer) of cone angles in degrees.
pub fn spot_cosines(inner_deg: f32, outer_deg: f32) -> (f32, f32) {
    (inner_deg.to_radians().cos(), outer_deg.to_radians().cos())
}

/// Factor a clamped spot's intensity is multiplied by, lerp(n, 1, k) with n the sphere over the cone; k is 1 for LOD lights and unknown for others, so callers pass it.
pub fn spot_intensity_factor(cos_inner: f32, cos_outer: f32, k: f32) -> f32 {
    let k = saturate(k);
    if k >= 1.0 {
        return 1.0;
    }
    // The floor also on the denominator's 1 - cos(outer) keeps a cone under about 0.25 degrees finite.
    let one_minus_outer = (1.0 - cos_outer).max(SPOT_MIN_ONE_MINUS_COS);
    let solid = 1.0 - cos_inner + (cos_inner - cos_outer) / 3.0;
    let n = 2.0 * one_minus_outer / (solid * one_minus_outer);
    n + k * (1.0 - n)
}

/// Capsule intensity after the length normalisation that flags-2 bit 17 asks for, input units.
pub fn capsule_intensity(intensity: f32, length: f32, flags2: u32) -> f32 {
    if (flags2 & CAPSULE_NORMALISE_BIT) != 0 {
        let span = length + 1.0;
        intensity / (span * span)
    } else {
        intensity
    }
}

/// Record radius, negated when the inside test is on and the camera is within the range-scaled range; pass false for lights other than point and spot.
pub fn signed_range(range: f32, light: Vec3, camera: Vec3, inside_test_on: bool) -> f32 {
    if inside_test_on && (light - camera).length_squared() < range * range {
        -range
    } else {
        range
    }
}

/// Spot record terms sqrt(1 - cos(outer)^2) and 1 / (cos(inner) - cos(outer)), the second None at a gap of 1e-6 or less, where the build blends an unread constant.
pub fn spot_record_terms(cos_inner: f32, cos_outer: f32) -> (f32, Option<f32>) {
    let sin_outer = (1.0 - cos_outer * cos_outer).max(0.0).sqrt();
    let gap = cos_inner - cos_outer;
    let inverse_gap = if gap > SPOT_MIN_COS_GAP {
        Some(1.0 / gap)
    } else {
        None
    };
    (sin_outer, inverse_gap)
}

/// Rectangle width or height as its GPU record stores it, metres.
pub fn rect_extent(extent: f32) -> f32 {
    extent.max(RECT_MIN_EXTENT)
}

/// GPU record index (slot << 8) + byte; `NO_SLOT` and `NO_SLOT_BYTE` are the defaults.
pub fn light_index(slot: u16, byte: u8) -> u32 {
    (u32::from(slot) << 8) + u32::from(byte)
}

/// LOD spot cone angle from its byte, degrees.
pub fn lod_cone_angle(byte: u8) -> f32 {
    f32::from(byte) * LOD_CONE_DEG_PER_UNIT
}

/// LOD capsule length from its byte, metres.
pub fn lod_capsule_length(byte: u8) -> f32 {
    f32::from(byte) * LOD_CAPSULE_M_PER_UNIT
}

/// LOD rectangle width or height from its byte, metres.
pub fn lod_rectangle_extent(byte: u8) -> f32 {
    f32::from(byte) * LOD_RECT_M_PER_UNIT
}

/// Category 0..=5 from bits 22..24 of a LOD light's word, None above 5.
pub fn lod_category(word: u32) -> Option<usize> {
    let category = ((word >> LOD_CATEGORY_SHIFT) & LOD_CATEGORY_MASK) as usize;
    if category < LOD_CATEGORY_COUNT {
        Some(category)
    } else {
        None
    }
}

/// Category intensity multiplier from the six frame multipliers, 1 on set 0 or when the category has none.
pub fn lod_category_multiplier(word: u32, set_one: bool, multipliers: &[f32; 6]) -> f32 {
    match lod_category(word) {
        Some(category) if set_one => multipliers[category],
        _ => 1.0,
    }
}

/// LOD light colour lerp(rgb8 / 255 * gain * tint, override, amount); gain is v237..v239, the rest the light's set.
pub fn lod_colour(
    rgb8: [u8; 3],
    gain: Vec3,
    tint: Vec3,
    override_colour: Vec3,
    amount: f32,
) -> Vec3 {
    let rgb = Vec3::new(f32::from(rgb8[0]), f32::from(rgb8[1]), f32::from(rgb8[2]));
    let base = rgb / 255.0 * gain * tint;
    base + (override_colour - base) * amount
}

/// LOD light intensity: v240 x the set's intensity (v286 or v374) x the 0..1 fade x the light's own x its category multiplier.
pub fn lod_intensity(
    global: f32,
    set_intensity: f32,
    fade: f32,
    intensity: f32,
    category_multiplier: f32,
) -> f32 {
    global * set_intensity * fade * intensity * category_multiplier
}

/// LOD light range: the light's x v241 x the call's factor (1, or a global when its byte argument is set), metres.
pub fn lod_range(range: f32, range_scale: f32, call_factor: f32) -> f32 {
    range * range_scale * call_factor
}

/// Unit tangent from a LOD light's octahedral half2; the fold below z = 0 is the standard octahedral fold, assumed.
pub fn octahedral_tangent(h: [f32; 2]) -> Vec3 {
    let x = 2.0 * h[0] - 1.0;
    let y = 2.0 * h[1] - 1.0;
    let z = 1.0 - x.abs() - y.abs();
    let (x, y) = if z < 0.0 {
        ((1.0 - y.abs()) * x.signum(), (1.0 - x.abs()) * y.signum())
    } else {
        (x, y)
    };
    Vec3::new(x, y, z).normalize_or_zero()
}

/// Byte 3 of a GPU record's packed colour from a LOD light's flag source byte.
pub fn lod_flag_byte(b: u8) -> u8 {
    (b & 0x10) | ((b >> 4) & 0x0e) | ((b >> 3) & 1)
}

/// Bytes 0..2 of a GPU record's packed colour, clamp(trunc(c * 255 + 0.5), 0, 255).
pub fn pack_rgb8(colour: Vec3) -> [u8; 3] {
    [
        unit_to_byte(colour.x),
        unit_to_byte(colour.y),
        unit_to_byte(colour.z),
    ]
}

/// A LOD light's 16-byte volume term from its vec4 v and float f, (v.xyz, max(f, 0) + v.w) / max(|f|, 1e-4).
pub fn lod_volume(v: Vec4, f: f32) -> Vec4 {
    Vec4::new(v.x, v.y, v.z, f.max(0.0) + v.w) / f.abs().max(LOD_VOLUME_MIN_DIVISOR)
}

/// Entry fade byte from a 0..255 amount, squared when its flag is set, else its square root.
pub fn fade_byte(byte: u8, squared: bool) -> u8 {
    let x = f32::from(byte) / 255.0;
    let g = if squared { x * x } else { x.sqrt() };
    unit_to_byte(g)
}

/// Animation (u, v) of a LOD light at a clock in milliseconds, None unless speed > 0 and the row's sign bit is clear; size is the texture's, floored at 1, and frac is x - floor(x) (assumed for negative x).
pub fn lod_anim_uv(
    ms: f32,
    speed: f32,
    phase: f32,
    row_offset: f32,
    row: u16,
    size: u16,
) -> Option<[f32; 2]> {
    let animated = speed > 0.0 && (row & ANIM_ROW_RAW_BIT) == 0;
    if !animated {
        return None;
    }
    let n = f32::from(size.max(1));
    let t = ms * ANIM_SECONDS_PER_MS;
    let x = t * ANIM_U_RATE * speed / n - ANIM_U_RATE * phase / n;
    let u = x - x.floor();
    let v = 1.0 - (f32::from(row) * ANIM_ROW_STRIDE + 0.5 + row_offset) / n;
    Some([u, v])
}

/// Whether a LOD light is skipped: v281 is 0 and its intensity is not positive.
pub fn lod_light_skipped(keep_dark: f32, intensity: f32) -> bool {
    keep_dark == 0.0 && intensity <= 0.0
}

/// Cull sphere of a LOD light bucket around the camera, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodSphere {
    /// Range, metres.
    pub range: f32,
    /// Term added to the range for the cull, metres.
    pub margin: f32,
}

impl LodSphere {
    /// Radius a bucket is culled against, metres.
    pub fn cull_radius(self) -> f32 {
        self.range + self.margin
    }
}

/// LOD sphere a group's sphere index picks, None for an index whose range is runtime only.
pub fn lod_sphere(index: u16) -> Option<LodSphere> {
    LOD_SPHERES.get(usize::from(index)).copied()
}

/// Whether an AABB bucket lies within a cull radius in metres of the camera, before the frustum test.
pub fn bucket_in_range(min: Vec3, max: Vec3, camera: Vec3, radius: f32) -> bool {
    let centre = (min + max) * 0.5;
    let half_diagonal_sq = ((max - min) * 0.5).length_squared();
    let half_diagonal = if half_diagonal_sq < f32::MIN_POSITIVE {
        0.0
    } else {
        half_diagonal_sq.sqrt()
    };
    let reach = half_diagonal + radius;
    (centre - camera).length_squared() <= reach * reach
}

/// Whole hour of a day phase (0.25 sunrise, 0.5 noon, 0.75 sunset), hours.
pub fn day_phase_hour(day_phase: f64) -> i64 {
    let minutes = (day_phase * MINUTES_PER_DAY).trunc();
    (minutes / 60.0).floor() as i64
}

/// Whether a day phase falls in hours 07:00..16:59, where light ranges are capped.
pub fn in_day_cap(day_phase: f64) -> bool {
    let hour = day_phase_hour(day_phase);
    (DAY_CAP_FIRST_HOUR..=DAY_CAP_LAST_HOUR).contains(&hour)
}

/// LOD or distant cull radius after the day cap, metres; it takes the summed radius (a LOD sphere's `cull_radius`, so 520 becomes 400).
pub fn day_capped_range(range: f32, day_phase: f64) -> f32 {
    if in_day_cap(day_phase) {
        range.min(DAY_RANGE_CAP)
    } else {
        range
    }
}

/// Whether the main light cull processes a light at a position for a day phase.
pub fn light_in_day_reach(light: Vec3, camera: Vec3, day_phase: f64) -> bool {
    !in_day_cap(day_phase) || (light - camera).length_squared() < DAY_RANGE_CAP * DAY_RANGE_CAP
}

/// Corona tunables; the defaults are the build's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoronaTunables {
    /// Size scale of pass 1, unitless.
    pub pass_one_size_scale: f32,
    /// Intensity scale of pass 1, unitless.
    pub pass_one_intensity_scale: f32,
    /// Size per metre of depth in screen-constant mode, unitless.
    pub screen_size_per_metre: f32,
    /// Whether screen-constant size is on.
    pub screen_constant: bool,
    /// Reference tan(half field of view), unitless.
    pub reference_tan_half_fov: f32,
    /// Whether screen-constant size is compensated for the field of view.
    pub fov_compensation: bool,
    /// Size scale in world-size mode, unitless.
    pub world_size_scale: f32,
    /// Size scale of mode 2, unitless.
    pub mode2_size_scale: f32,
    /// Least z bias, metres.
    pub bias_min: f32,
    /// Most z bias, metres.
    pub bias_max: f32,
    /// Depth where the z bias ramp starts, metres.
    pub bias_ramp_start: f32,
    /// Depth where the z bias ramp ends, metres.
    pub bias_ramp_end: f32,
    /// Distance where the far blend starts, metres.
    pub far_blend_start: f32,
    /// Width of the far blend, metres.
    pub far_blend_width: f32,
    /// Divisor of the bit-5 far blend, cos(100 degrees), negative.
    pub far_blend_divisor: f32,
}

impl Default for CoronaTunables {
    fn default() -> Self {
        CoronaTunables {
            pass_one_size_scale: 1.0,
            pass_one_intensity_scale: 1.0,
            screen_size_per_metre: 0.03,
            screen_constant: false,
            reference_tan_half_fov: (50.0_f32 * std::f32::consts::PI / 360.0).tan(),
            fov_compensation: true,
            world_size_scale: 1.0,
            mode2_size_scale: 2.0,
            bias_min: 0.0,
            bias_max: 1.0,
            bias_ramp_start: 1.0,
            bias_ramp_end: 2.0,
            far_blend_start: 200.0,
            far_blend_width: 50.0,
            far_blend_divisor: 100.0_f32.to_radians().cos(),
        }
    }
}

/// Far-blend amount `near` of a corona at a camera distance, 0 up to the blend start and 1 past its width, unitless.
pub fn corona_near(distance: f32, tunables: &CoronaTunables) -> f32 {
    saturate((distance - tunables.far_blend_start) / tunables.far_blend_width)
}

/// Directional terms (cone, D) from the corona's unit direction, the unit vector toward the camera, its two cone angles in degrees and its remap width; cone is unclamped, and a zero width gives a non-finite cone that the I > 0 test drops.
pub fn corona_cone(
    direction: Vec3,
    to_camera: Vec3,
    angles_deg: [f32; 2],
    width: f32,
) -> (f32, f32) {
    let alignment = direction.dot(to_camera);
    let weight = |angle_deg: f32| -> f32 {
        let cos_angle = angle_deg.to_radians().cos();
        saturate((alignment - cos_angle) / (CORONA_CONE_ONE - cos_angle))
    };
    let d = (weight(angles_deg[0]) + weight(angles_deg[1])).clamp(0.0, 1.0);
    ((d + width - 1.0) / width, d)
}

/// Bit-5 far blend of (cone, D) toward b by `near`, b = 1 for a negative alignment, else 1 - min(1, alignment / divisor).
pub fn corona_far_blend(
    cone: f32,
    d: f32,
    alignment: f32,
    near: f32,
    tunables: &CoronaTunables,
) -> (f32, f32) {
    // The divisor cos(100 degrees) is negative, so b is at least 1 for a non-negative alignment, as read.
    let b = if alignment < 0.0 {
        1.0
    } else {
        1.0 - (alignment / tunables.far_blend_divisor).min(1.0)
    };
    (cone + (b - cone) * near, d + (b - d) * near)
}

/// Corona intensity before its fades: cone x the runtime scale (a calibration knob) x the pass-1 scale x the item's intensity.
pub fn corona_intensity(
    cone: f32,
    runtime_scale: f32,
    pass_one: bool,
    item_intensity: f32,
    tunables: &CoronaTunables,
) -> f32 {
    let pass_scale = if pass_one {
        tunables.pass_one_intensity_scale
    } else {
        1.0
    };
    cone * runtime_scale * pass_scale * item_intensity
}

/// Whether a corona mode (0, 1, 2 or 4) draws at screen-constant size.
pub fn corona_screen_constant(mode: u32, tunables: &CoronaTunables) -> bool {
    mode != CORONA_MODE_WORLD_ONLY && tunables.screen_constant
}

/// Ratio of a camera's tan(half field of view) to the reference, unitless.
pub fn corona_fov_ratio(tan_half_fov: f32, tunables: &CoronaTunables) -> f32 {
    tan_half_fov / tunables.reference_tan_half_fov
}

/// View depth of a corona along the camera's forward axis, metres.
pub fn corona_depth(position: Vec3, camera: Vec3, forward: Vec3) -> f32 {
    (position - camera).dot(forward)
}

/// Depth a corona is pulled toward the camera, metres; runtime_bias is the mode's runtime offset.
pub fn corona_z_bias(
    depth: f32,
    item_bias: f32,
    runtime_bias: f32,
    tunables: &CoronaTunables,
) -> f32 {
    let ramp = saturate(
        (depth - tunables.bias_ramp_start) / (tunables.bias_ramp_end - tunables.bias_ramp_start),
    );
    (ramp * item_bias)
        .max(tunables.bias_min)
        .min(tunables.bias_max)
        + runtime_bias
}

/// One corona's inputs to its size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoronaSizeInput {
    /// Draw mode: 0, 1, 2 or 4.
    pub mode: u32,
    /// Whether this is the second (pass 1) draw.
    pub pass_one: bool,
    /// The item's size, in its own unit.
    pub item_size: f32,
    /// Directional cone weight D, 1 for an undirected corona, unitless.
    pub cone_weight: f32,
    /// View depth, metres.
    pub depth: f32,
    /// Z bias from `corona_z_bias`, metres.
    pub bias: f32,
    /// Field-of-view ratio from `corona_fov_ratio`, unitless.
    pub fov_ratio: f32,
}

/// Corona size after its mode's scaling and the z-bias shrink, in the item size's unit.
pub fn corona_size(input: &CoronaSizeInput, tunables: &CoronaTunables) -> f32 {
    let CoronaSizeInput {
        mode,
        pass_one,
        item_size,
        cone_weight,
        depth,
        bias,
        fov_ratio,
    } = *input;
    let screen = corona_screen_constant(mode, tunables);
    let base = if screen {
        tunables.screen_size_per_metre
    } else {
        tunables.world_size_scale
    };
    let pass_scale = if pass_one {
        tunables.pass_one_size_scale
    } else {
        1.0
    };
    let fov = if screen && tunables.fov_compensation {
        fov_ratio
    } else {
        1.0
    };
    let mut size = pass_scale * base * fov * item_size;
    if mode == CORONA_MODE_DEPTH {
        if screen {
            size *= cone_weight * depth;
        } else {
            // 0.5 * cone_weight in front of the camera; z == 0 gives NaN, which the caller's s > 0 test drops.
            size *= cone_weight * depth / (depth + depth.max(0.0));
        }
    }
    let mode_size = if mode == CORONA_MODE_SCALED {
        tunables.mode2_size_scale
    } else {
        1.0
    };
    let pulled = bias / depth;
    (size - pulled * size) * mode_size
}

/// Corona position pulled toward the camera by its z bias, metres.
pub fn corona_pulled_position(position: Vec3, camera: Vec3, bias: f32, depth: f32) -> Vec3 {
    position - (position - camera) * (bias / depth)
}

/// Shadow light direction from sun_moon::LightState::direction (the unit vector toward the light: the sun's while the sun leads, the moon's while the moon leads), tilted 0.265 degrees toward the zenith, then floored at 0.15 down.
pub fn shadow_direction(toward_light: Vec3) -> Vec3 {
    let zenith_angle = toward_light.z.clamp(-1.0, 1.0).acos();
    let tilted = (zenith_angle - SHADOW_TILT_DEG.to_radians()).max(0.0);
    let horizontal = (toward_light.x * toward_light.x + toward_light.y * toward_light.y).sqrt();
    let scale = if horizontal > 0.0 {
        tilted.sin() / horizontal
    } else {
        0.0
    };
    let tilted_toward_light =
        Vec3::new(toward_light.x * scale, toward_light.y * scale, tilted.cos());
    -floor_light_direction(tilted_toward_light)
}

/// Lowest z of a light direction toward the light, unitless.
const MIN_LIGHT_Z: f32 = 0.15;

// Copied from the day track's sun_moon module, which or-renderer cannot reach: the lander keeps one of the two.
fn floor_light_direction(direction: Vec3) -> Vec3 {
    Vec3::new(direction.x, direction.y, direction.z.max(MIN_LIGHT_Z)).normalize()
}

/// Linear split distances for a cascade count (clamped to 1..=4), padded with the far split, metres.
pub fn linear_splits(count: usize) -> [f32; 5] {
    let n = count.clamp(1, MAX_CASCADES);
    let mut splits = [CASCADE_LINEAR_FAR; 5];
    splits[0] = CASCADE_LINEAR_NEAR;
    for (i, split) in splits.iter_mut().enumerate().take(n).skip(1) {
        *split = CASCADE_LINEAR_NEAR + i as f32 * CASCADE_LINEAR_SPAN / n as f32;
    }
    splits[n] = CASCADE_LINEAR_FAR;
    splits
}

/// Blends the inner splits toward logarithmic ones by lambda, then pulls the near split in above 3 cascades; skipped unless 0 < near < far.
pub fn log_blend_splits(splits: &mut [f32; 5], count: usize, lambda: f32) {
    let n = count.clamp(1, MAX_CASCADES);
    let near = splits[0];
    let far = splits[n];
    if lambda == 0.0 || near <= 0.0 || far <= near {
        return;
    }
    for split in splits.iter_mut().take(n).skip(1) {
        let t = (*split - near) / (far - near);
        *split -= lambda * (*split - near * (far / near).powf(t));
    }
    if n > 3 {
        splits[0] = splits[0].min(CASCADE_BLENDED_NEAR);
    }
}

/// Log blend lambda: the script's value when positive, else 0.9.
pub fn script_lambda(script_value: f32) -> f32 {
    if script_value > 0.0 {
        script_value
    } else {
        CASCADE_DEFAULT_LAMBDA
    }
}

/// Whether the directional light casts sun shadows, from its final intensity and shadow flag.
pub fn sun_casts_shadows(final_intensity: f32, shadow_flag: bool) -> bool {
    shadow_flag && final_intensity >= SHADOW_MIN_SUN_INTENSITY
}

fn smoothstep_unit(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

fn tracked_radius(radius: f32, aircraft: bool) -> f32 {
    if aircraft {
        radius.max(AIRCRAFT_MIN_RADIUS)
    } else {
        radius
    }
}

/// Target far scale for a tracked entity's radius in metres (a bounding radius, inferred), 1 to 2.5.
pub fn tracker_scale_target(radius: f32, aircraft: bool) -> f32 {
    let r = tracked_radius(radius, aircraft).clamp(TRACKER_RADIUS_START, TRACKER_SCALE_RADIUS_END);
    let x = (r - TRACKER_RADIUS_START) / (TRACKER_SCALE_RADIUS_END - TRACKER_RADIUS_START);
    1.0 + (TRACKER_SCALE_MAX - 1.0) * smoothstep_unit(x)
}

/// Log blend lambda for a tracked entity's radius in metres (a bounding radius, inferred).
pub fn tracked_lambda(lambda: f32, radius: f32, aircraft: bool) -> f32 {
    let r = tracked_radius(radius, aircraft).clamp(TRACKER_RADIUS_START, TRACKER_LAMBDA_RADIUS_END);
    let x = (r - TRACKER_RADIUS_START) / (TRACKER_LAMBDA_RADIUS_END - TRACKER_RADIUS_START);
    let factor =
        TRACKER_LAMBDA_START + (TRACKER_LAMBDA_END - TRACKER_LAMBDA_START) * smoothstep_unit(x);
    lambda * TRACKER_LAMBDA_RENORMALISE * factor
}

/// Far scale eased one frame toward its target, or set to it on a snap, unitless.
pub fn ease_tracker_scale(previous: f32, target: f32, snap: bool) -> f32 {
    if snap {
        target
    } else {
        previous + (target - previous) * TRACKER_EASE_PER_FRAME
    }
}

/// Steep-camera factors (tracker scale, lambda) from the camera's forward z, both 1 at |z| <= 0.35.
pub fn steep_camera_factors(forward_z: f32) -> (f32, f32) {
    let u = saturate(STEEP_CAMERA_SLOPE * forward_z.abs() - STEEP_CAMERA_OFFSET);
    (
        1.0 - STEEP_CAMERA_TRACKER_LOSS * u,
        1.0 - STEEP_CAMERA_LAMBDA_LOSS * u,
    )
}

/// A frame's split distances, metres; scale is bounds times tracker scale, far_boost the 1.75 case.
pub fn cascade_splits(
    count: usize,
    scale: f32,
    lambda: f32,
    distance_multiplier: f32,
    sun_casts: bool,
    far_boost: bool,
    alternate: bool,
) -> [f32; 5] {
    let n = count.clamp(1, MAX_CASCADES);
    let mut splits = linear_splits(n);
    for split in splits.iter_mut() {
        *split *= scale;
    }
    log_blend_splits(&mut splits, n, lambda);
    if far_boost {
        for split in splits.iter_mut().skip(n) {
            *split *= CASCADE_FAR_BOOST;
        }
    }
    // The alternate mode moves only the last two splits by m; that the sun factor still applies to all is inferred.
    let m = if alternate {
        splits[3] *= distance_multiplier / 3.0 + 2.0 / 3.0;
        splits[4] *= 2.0 * distance_multiplier / 3.0 + 1.0 / 3.0;
        1.0
    } else {
        distance_multiplier
    };
    let sun = if sun_casts {
        1.0
    } else {
        CASCADE_NO_SUN_FACTOR
    };
    for split in splits.iter_mut() {
        *split *= m * sun;
    }
    splits
}

/// Active cascade count and the fraction the last one's far end is moved by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveCascades {
    /// Number of active cascades, 2..=4.
    pub count: usize,
    /// Fraction 0..1 of the last cascade kept, 0 for all of it.
    pub far_fraction: f32,
}

/// Active cascades from the frame's cascade-count variable; the fraction comes from the unclamped value as read, so 4.5 gives four cascades and 0.5.
pub fn active_cascades(value: f32) -> ActiveCascades {
    let ceiling = value.ceil();
    let count = if ceiling.is_nan() || ceiling < MIN_ACTIVE_CASCADES as f32 {
        MIN_ACTIVE_CASCADES
    } else if ceiling > MAX_CASCADES as f32 {
        MAX_CASCADES
    } else {
        ceiling as usize
    };
    let mut far_fraction = value - value.floor();
    if far_fraction > 0.0 && far_fraction < CASCADE_FRACTION_SNAP {
        far_fraction = 0.0;
    }
    ActiveCascades {
        count,
        far_fraction,
    }
}

/// Far split of the last active cascade, metres, moved between its two splits by the far fraction; the read lerp(split[N-1], split[N]) names the array size N, which equals the active count only at four, so taking the active count is inferred.
pub fn active_far_split(splits: &[f32; 5], active: ActiveCascades) -> f32 {
    let last = active.count.clamp(1, MAX_CASCADES);
    let fraction = active.far_fraction;
    if fraction > 0.0 && fraction < 1.0 {
        splits[last - 1] + (splits[last] - splits[last - 1]) * fraction
    } else {
        splits[last]
    }
}

/// Shadow filter radius from the softness variable, lerp(0.53, 10, max(v, 0)).
pub fn shadow_filter_radius(softness: f32) -> f32 {
    SHADOW_FILTER_MIN + (SHADOW_FILTER_MAX - SHADOW_FILTER_MIN) * softness.max(0.0)
}

/// View depth of a radial split distance, metres; tx and ty are the camera's two projection terms.
pub fn split_view_depth(split: f32, tx: f32, ty: f32) -> f32 {
    split / (1.0 + tx * tx + ty * ty).sqrt()
}

/// Far reach F of the last active cascade, metres: its bounding sphere's centre in the shadow frame plus sqrt(3) times its radius.
pub fn cascade_far_reach(centre_in_shadow_frame: Vec3, radius: f32) -> f32 {
    centre_in_shadow_frame.length() + 3.0_f32.sqrt() * radius
}

/// Sun shadow far fade at a distance in metres, 0 within 0.95 of the far reach to 1 at it.
pub fn cascade_far_fade(distance: f32, far_reach: f32) -> f32 {
    saturate(
        CASCADE_FADE_SLOPE * distance / far_reach.max(f32::MIN_POSITIVE)
            - (CASCADE_FADE_SLOPE - 1.0),
    )
}

#[cfg(test)]
#[path = "light_math_tests.rs"]
mod tests;
