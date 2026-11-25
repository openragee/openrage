//! The timecycle's base frame: the sample pick, the curves, the weather and region blends and their weights, the region boxes, the modifier order and strengths, keyed modifiers and the name hash.
//! A keyed modifier's second and third dimension positions are taken from the caller, because how the game derives them was not read.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::Vec3;

/// The floats of a timecycle frame: one a variable, the variable table's entries and one past them.
pub const FRAME_FLOATS: usize = 1408;
/// One timecycle frame, variable N at float N.
pub type TimecycleFrame = [f32; FRAME_FLOATS];

/// Unitless; 1/(2·√3) as the build stores it, the half extent per unit of x width of a flagged box.
const UNIFORM_EXTENT_SCALE: f32 = f32::from_bits(0x3e93_cd3a);
/// Unitless; how many times a box's height distance counts against its horizontal distance.
const HEIGHT_WEIGHT: f32 = 3.0;
/// Hours per second, as the build stores 1/3600.
const HOURS_PER_SECOND: f32 = f32::from_bits(0x3991_a2b4);
/// Unitless; the smallest span a keyed modifier divides by.
const KEY_SPAN_FLOOR: f32 = 0.001;
/// Unitless; the bounce's first-segment addend, inferred (the standard bounce).
const BOUNCE_FIRST_ADDEND: f32 = 0.0;
/// Unitless; the bounce's last-segment addend, inferred (the standard bounce).
const BOUNCE_LAST_ADDEND: f32 = 0.984375;
/// Unitless; the one-weather blend's x2, inferred from the halves holding value/2.
const ONE_WEATHER_SCALE: f32 = 2.0;
/// Length of an automatic weather change in units of dt; that the units are seconds is assumed.
pub const AUTO_TRANSITION_DURATION: f32 = 60.0;

/// Why a frame could not be made from the given inputs.
#[derive(Debug, PartialEq)]
pub enum FrameError {
    /// Two value lists that must match differ in length.
    LengthMismatch { left: usize, right: usize },
    /// A table that must hold at least one entry is empty.
    EmptyTable,
    /// A keyed modifier's blend sample count is not 1, 2 or 4.
    SampleCount(usize),
    /// An index is past its value list.
    MissingValue { index: usize, len: usize },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LengthMismatch { left, right } => {
                write!(f, "two value lists differ in length: {left} and {right}")
            }
            Self::EmptyTable => write!(f, "the table is empty"),
            Self::SampleCount(n) => write!(
                f,
                "a keyed modifier has {n} blend samples; only 1, 2 or 4 are defined"
            ),
            Self::MissingValue { index, len } => {
                write!(f, "index {index} is past a list of {len} values")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Linear interpolation a + (b - a)·t.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// One key of the sample table: a sun angle in radians and the sample it names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampleKey {
    /// Sun angle, radians.
    pub angle: f32,
    /// Sample index into a cycle region.
    pub sample: i32,
}

/// The two samples a frame blends and the uneased position t (0..1) between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplePick {
    /// Lower sample; its curve eases t.
    pub i0: i32,
    /// Upper sample.
    pub i1: i32,
    /// Position from i0 to i1 before the curve, 0..1.
    pub t: f32,
}

/// The samples around sun angle x (radians, gta6_day_sun::sample_angle of the ephemeris sun SunDay::direction) in keys sorted ascending; the last key pairs with the first across 2π.
pub fn pick_samples(x: f32, keys: &[SampleKey]) -> Result<SamplePick, FrameError> {
    if keys.is_empty() {
        return Err(FrameError::EmptyTable);
    }
    let last: usize = keys.len() - 1;
    let mut lo: usize = 0;
    for (k, key) in keys.iter().enumerate() {
        if key.angle <= x {
            lo = k;
        }
    }
    let hi: usize = if lo == last { 0 } else { lo + 1 };
    let a: f32 = keys[lo].angle;
    let mut b: f32 = keys[hi].angle;
    if b < a {
        b += TAU;
    }
    // A zero span (one key, or two equal angles) would divide 0 by 0 into NaN.
    let span: f32 = b - a;
    let t: f32 = if span > 0.0 {
        ((x - a) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Ok(SamplePick {
        i0: keys[lo].sample,
        i1: keys[hi].sample,
        t,
    })
}

/// The pick's t eased by curve `interpolation_types[i0]`, the lower sample's; a sample past the list is linear.
pub fn eased_t(pick: &SamplePick, interpolation_types: &[usize]) -> f32 {
    let curve: usize = usize::try_from(pick.i0)
        .ok()
        .and_then(|i| interpolation_types.get(i))
        .copied()
        .unwrap_or(0);
    sample_curve(curve, pick.t)
}

/// The sample curve index named by an interpolation_type hash; an unknown hash is 0 (linear).
pub fn sample_curve_index(hash: u32) -> usize {
    const HASHES: [u32; 26] = [
        0xe037_3822,
        0xd17d_e093,
        0x92cd_4243,
        0x0cb0_ae52,
        0x660c_a82f,
        0x87de_d884,
        0x813b_eb09,
        0x7d5e_e926,
        0x077c_de24,
        0xae86_da96,
        0xd233_273f,
        0x10d7_7a29,
        0x7198_7e2a,
        0x6c27_c5fd,
        0x87bf_daed,
        0x9145_eb81,
        0x5ba4_9d43,
        0x278e_9952,
        0xb249_7316,
        0x7b87_f36a,
        0x1273_6ad9,
        0x6839_0109,
        0x3174_512f,
        0xb44c_9e4a,
        0x6189_557a,
        0xcef4_0261,
    ];
    HASHES.iter().position(|&h| h == hash).unwrap_or(0)
}

/// Sample curve `index` (0..25) at t (0..1); an index past 25 is linear.
pub fn sample_curve(index: usize, t: f32) -> f32 {
    match index {
        2 => 1.0 - (t * FRAC_PI_2).cos(),
        3 => (t * FRAC_PI_2).sin(),
        4 => 0.5 - 0.5 * (PI * t).cos(),
        5 => t * t,
        6 => (2.0 - t) * t,
        7 => {
            if 2.0 * t < 1.0 {
                2.0 * t * t
            } else {
                (1.5 - t) * (2.0 * t - 1.0) + 0.5
            }
        }
        8 => t * t * t,
        9 => (t - 1.0).powi(3) + 1.0,
        10 => {
            if 2.0 * t < 1.0 {
                4.0 * t.powi(3)
            } else {
                0.5 * (2.0 * t - 2.0).powi(3) + 1.0
            }
        }
        11 => t.powi(4),
        12 => 1.0 - (t - 1.0).powi(4),
        13 => {
            if 2.0 * t < 1.0 {
                8.0 * t.powi(4)
            } else {
                1.0 - 0.5 * (2.0 * t - 2.0).powi(4)
            }
        }
        14 | 16 => {
            if 2.0 * t < 1.0 {
                16.0 * t.powi(5)
            } else {
                0.5 * (2.0 * t - 2.0).powi(5) + 1.0
            }
        }
        15 => (t - 1.0).powi(5) + 1.0,
        17 => {
            if t > 0.0 {
                (10.0 * t - 10.0).exp2()
            } else {
                0.0
            }
        }
        18 => {
            if t < 1.0 {
                1.0 - (-10.0 * t).exp2()
            } else {
                1.0
            }
        }
        19 => expo_in_out(t),
        20 => 1.0 - (1.0 - t * t).sqrt(),
        21 => (1.0 - (t - 1.0) * (t - 1.0)).sqrt(),
        22 => circ_in_out(t),
        23 => 1.0 - bounce_out(1.0 - t),
        24 => bounce_out(t),
        25 => {
            if t < 0.5 {
                0.5 * (1.0 - bounce_out(1.0 - 2.0 * t))
            } else {
                0.5 * bounce_out(2.0 * t - 1.0) + 0.5
            }
        }
        _ => t,
    }
}

// Inferred: the standard exponential in-out; only the start of the build's curve 19 was read.
fn expo_in_out(t: f32) -> f32 {
    if t <= 0.0 {
        0.0
    } else if t >= 1.0 {
        1.0
    } else if 2.0 * t < 1.0 {
        0.5 * (20.0 * t - 10.0).exp2()
    } else {
        1.0 - 0.5 * (10.0 - 20.0 * t).exp2()
    }
}

// Inferred: the standard circular in-out; only the start of the build's curve 22 was read.
fn circ_in_out(t: f32) -> f32 {
    if 2.0 * t < 1.0 {
        let u: f32 = 2.0 * t;
        -0.5 * ((1.0 - u * u).sqrt() - 1.0)
    } else {
        let u: f32 = 2.0 * t - 2.0;
        0.5 * ((1.0 - u * u).sqrt() + 1.0)
    }
}

fn bounce_out(t: f32) -> f32 {
    let scale: f32 = 7.5625;
    if t < 1.0 / 2.75 {
        scale * t * t + BOUNCE_FIRST_ADDEND
    } else if t < 2.0 / 2.75 {
        let u: f32 = t - 1.5 / 2.75;
        scale * u * u + 0.75
    } else if t < 2.5 / 2.75 {
        let u: f32 = t - 2.25 / 2.75;
        scale * u * u + 0.9375
    } else {
        let u: f32 = t - 2.625 / 2.75;
        scale * u * u + BOUNCE_LAST_ADDEND
    }
}

/// An IEEE half from its bits, as f32.
pub fn half_to_f32(bits: u16) -> f32 {
    let wide: u32 = bits as u32;
    let sign: u32 = (wide & 0x8000) << 16;
    let exponent: u32 = (wide >> 10) & 0x1f;
    let mantissa: u32 = wide & 0x3ff;
    if exponent == 0 {
        // Zero or subnormal: mantissa · 2^-24.
        let magnitude: f32 = mantissa as f32 * f32::from_bits(0x3380_0000);
        return if sign != 0 { -magnitude } else { magnitude };
    }
    if exponent == 0x1f {
        return f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13));
    }
    f32::from_bits(sign | ((exponent + 112) << 23) | (mantissa << 13))
}

/// The variable value a stored cycle half holds: the half times 2.
pub fn stored_half_value(bits: u16) -> f32 {
    2.0 * half_to_f32(bits)
}

/// One weather's cycle at the two picked samples of one region: f16 bits, one per variable.
#[derive(Debug, Clone, Copy)]
pub struct CycleSamples<'a> {
    /// The region's halves at sample i0.
    pub at_i0: &'a [u16],
    /// The region's halves at sample i1.
    pub at_i1: &'a [u16],
}

/// One region's base frame per variable: 2·lerp(lerp(A0, B0, wx), lerp(A1, B1, wx), t); with one weather 2·lerp(A0, A1, t), the x2 inferred.
pub fn blend_weathers(
    prev: CycleSamples<'_>,
    next: Option<CycleSamples<'_>>,
    wx: f32,
    t: f32,
) -> Result<Box<TimecycleFrame>, FrameError> {
    same_length(prev.at_i0.len(), FRAME_FLOATS)?;
    same_length(prev.at_i1.len(), FRAME_FLOATS)?;
    let mut frame: Box<TimecycleFrame> = Box::new([0.0; FRAME_FLOATS]);
    match next {
        None => {
            for i in 0..FRAME_FLOATS {
                let value: f32 = lerp(half_to_f32(prev.at_i0[i]), half_to_f32(prev.at_i1[i]), t);
                frame[i] = ONE_WEATHER_SCALE * value;
            }
        }
        Some(next) => {
            same_length(next.at_i0.len(), FRAME_FLOATS)?;
            same_length(next.at_i1.len(), FRAME_FLOATS)?;
            for i in 0..FRAME_FLOATS {
                let at_i0: f32 = lerp(half_to_f32(prev.at_i0[i]), half_to_f32(next.at_i0[i]), wx);
                let at_i1: f32 = lerp(half_to_f32(prev.at_i1[i]), half_to_f32(next.at_i1[i]), wx);
                frame[i] = 2.0 * lerp(at_i0, at_i1, t);
            }
        }
    }
    Ok(frame)
}

fn same_length(left: usize, right: usize) -> Result<(), FrameError> {
    if left == right {
        Ok(())
    } else {
        Err(FrameError::LengthMismatch { left, right })
    }
}

/// The automatic transition's new weather weight wx from the old one and the advanced elapsed time; it never goes back.
pub fn auto_transition_step(t_old: f32, elapsed: f32, duration: f32) -> f32 {
    if t_old < 1.0 {
        t_old.max((elapsed / duration).min(1.0))
    } else {
        t_old
    }
}

/// The queued weather's weight wo from the milliseconds left and its total: 1 once left is below 0.
pub fn queued_weather_weight(left_ms: f32, total_ms: f32) -> f32 {
    let r: f32 = left_ms / total_ms;
    if r < 0.0 {
        1.0
    } else {
        1.0 - r.min(1.0)
    }
}

/// Which region frame a frame is made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegionPick {
    /// That region's frame alone.
    Only(usize),
    /// Region 0's frame lerped toward region 1's by this weight.
    Blend(f32),
}

/// The region choice: the forced region, region 1 at weight exactly 1, a blend inside (0, 1), else region 0.
#[allow(clippy::float_cmp)]
pub fn region_pick(forced_region: Option<usize>, region1_weight: f32) -> RegionPick {
    if let Some(region) = forced_region {
        return RegionPick::Only(region);
    }
    if region1_weight == 1.0 {
        RegionPick::Only(1)
    } else if region1_weight > 0.0 && region1_weight < 1.0 {
        RegionPick::Blend(region1_weight)
    } else {
        RegionPick::Only(0)
    }
}

/// The queued weather's base region and, when region 1 was not taken alone and w > 0, its own region-1 blend weight w; the forced path blends too.
#[allow(clippy::float_cmp)]
pub fn override_region_pick(
    forced_region: Option<usize>,
    region1_weight: f32,
) -> (usize, Option<f32>) {
    let region1_alone: bool = forced_region.is_none() && region1_weight == 1.0;
    let base: usize = forced_region.unwrap_or(if region1_alone { 1 } else { 0 });
    let blend: Option<f32> = if !region1_alone && region1_weight > 0.0 {
        Some(region1_weight)
    } else {
        None
    };
    (base, blend)
}

/// Per-variable lerp of two frames by w: region 0 toward region 1, or the base toward a queued weather.
pub fn lerp_frames(from: &TimecycleFrame, to: &TimecycleFrame, w: f32) -> Box<TimecycleFrame> {
    let mut frame: Box<TimecycleFrame> = Box::new([0.0; FRAME_FLOATS]);
    for (out, (a, b)) in frame.iter_mut().zip(from.iter().zip(to.iter())) {
        *out = lerp(*a, *b, w);
    }
    frame
}

/// A region or modifier box as the frame builder weighs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionBox {
    /// Box +0x08: region index (0 or 1) for a region box, modifier index for a modifier box.
    pub region: i32,
    /// Box +0x0c: the constructor value × 0.01; a modifier box's strength scale.
    pub value: f32,
    /// Falloff distance outside the box, metres; 0 means 1 m.
    pub falloff: f32,
    /// First full hour of the window.
    pub hour_a: i32,
    /// Hour over which the window ramps out.
    pub hour_b: i32,
    /// Index into the caller's strength list.
    pub strength_id: u32,
    /// Lower corner, metres.
    pub min: Vec3,
    /// Upper corner, metres.
    pub max: Vec3,
    /// Bit 0 of the box flags: every half extent is the x width times 1/(2·√3).
    pub uniform_extent: bool,
}

/// The hour window weight (0..1) of [hour_a, hour_b] at h:m:s, ramping in over the hour before A and out over hour B.
pub fn hour_window(hour_a: i32, hour_b: i32, hours: i32, minutes: i32, seconds: i32) -> f32 {
    if hour_a == hour_b {
        return 1.0;
    }
    // Wrapping, so a clock far outside 0..23 and 0..59 cannot overflow.
    let into_hour: f32 = minutes.wrapping_mul(60).wrapping_add(seconds) as f32 * HOURS_PER_SECOND;
    // The ramp in is tested before the ramp out, so it wins when hour B is the hour before A.
    if (hours.rem_euclid(24) + 1) % 24 == hour_a {
        return into_hour;
    }
    if hours == hour_b {
        return 1.0 - into_hour;
    }
    let inside: bool = if hour_a < hour_b {
        (hour_a..hour_b).contains(&hours)
    } else {
        hours >= hour_a || hours < hour_b
    };
    if inside {
        1.0
    } else {
        0.0
    }
}

/// A box's weight at `pos` (metres) and clock h:m:s; unclamped, so a value <= 0 is out of reach.
pub fn box_weight(
    region_box: &RegionBox,
    pos: Vec3,
    hours: i32,
    minutes: i32,
    seconds: i32,
    strengths: &[f32],
) -> f32 {
    let centre: Vec3 = (region_box.min + region_box.max) * 0.5;
    let half_extent: Vec3 = if region_box.uniform_extent {
        Vec3::splat((region_box.max.x - region_box.min.x) * UNIFORM_EXTENT_SCALE)
    } else {
        (region_box.max - region_box.min) * 0.5
    };
    let outside: Vec3 =
        ((pos - centre).abs() - half_extent).max(Vec3::ZERO) * Vec3::new(1.0, 1.0, HEIGHT_WEIGHT);
    let distance: f32 = outside.length();
    let reach: f32 = if region_box.falloff == 0.0 {
        1.0 - distance
    } else {
        1.0 - distance / region_box.falloff
    };
    if reach == 0.0 {
        return 0.0;
    }
    let window: f32 = hour_window(
        region_box.hour_a,
        region_box.hour_b,
        hours,
        minutes,
        seconds,
    );
    if window == 0.0 {
        return 0.0;
    }
    let strength: f32 = strengths
        .get(region_box.strength_id as usize)
        .copied()
        .unwrap_or(1.0);
    reach * window * strength
}

/// A modifier box's queued strength, weight × +0x0c, or None when it is not above 0.
pub fn modifier_box_strength(
    modifier_box: &RegionBox,
    pos: Vec3,
    hours: i32,
    minutes: i32,
    seconds: i32,
    strengths: &[f32],
) -> Option<f32> {
    let s: f32 =
        box_weight(modifier_box, pos, hours, minutes, seconds, strengths) * modifier_box.value;
    if s > 0.0 {
        Some(s)
    } else {
        None
    }
}

/// The two region weights at `pos` (metres) and clock h:m:s: per region, the largest box weight above 0, else 0.
pub fn region_weights(
    boxes: &[RegionBox],
    pos: Vec3,
    hours: i32,
    minutes: i32,
    seconds: i32,
    strengths: &[f32],
) -> [f32; 2] {
    let mut weights: [f32; 2] = [0.0, 0.0];
    for region_box in boxes {
        // A region other than 0 or 1 is skipped: the game's weight array holds two.
        if region_box.region != 0 && region_box.region != 1 {
            continue;
        }
        let weight: f32 = box_weight(region_box, pos, hours, minutes, seconds, strengths);
        let slot: usize = region_box.region as usize;
        if weight > 0.0 && weight > weights[slot] {
            weights[slot] = weight;
        }
    }
    weights
}

/// The strength of a per-weather modifier slot when only the previous weather has one.
pub fn prev_only_slot_strength(wx: f32) -> f32 {
    (1.0 - wx).clamp(0.0, 1.0)
}

/// A queued modifier's place in a pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueuedModifier {
    /// Position of the modifier's category in the category list.
    pub category: i32,
    /// The modifier's priority attribute.
    pub priority: i32,
}

/// The order key category·1000 + priority, wrapping on overflow.
pub fn modifier_order_key(category: i32, priority: i32) -> i32 {
    category.wrapping_mul(1000).wrapping_add(priority)
}

/// Queue positions in apply order: categories in list order, then by order key inside a category; a category outside 0..category_count is never walked.
pub fn modifier_apply_order(queue: &[QueuedModifier], category_count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..queue.len())
        .filter(|&i| usize::try_from(queue[i].category).is_ok_and(|c| c < category_count))
        .collect();
    order.sort_by_key(|&i| {
        let entry: QueuedModifier = queue[i];
        let key: i32 = modifier_order_key(entry.category, entry.priority);
        (entry.category, key)
    });
    order
}

/// A queued modifier's strength, or None when it is not above 0 and the modifier is skipped.
pub fn modifier_strength(
    strength_attr: f32,
    queued: f32,
    global_factor: Option<f32>,
    keyed_weight: Option<f32>,
) -> Option<f32> {
    // global_factor is Some only when modifier flag 0x200 is set; keyed_weight only for a keyed modifier.
    let s: f32 =
        strength_attr * queued * global_factor.unwrap_or(1.0) * keyed_weight.unwrap_or(1.0);
    if s > 0.0 {
        Some(s)
    } else {
        None
    }
}

/// A mode-2 entry: cur·(1 - s) + v1·s, left alone when cur already equals v1.
#[allow(clippy::float_cmp)]
pub fn apply_mode2(cur: f32, v1: f32, s: f32) -> f32 {
    if cur != v1 {
        cur * (1.0 - s) + v1 * s
    } else {
        cur
    }
}

/// The keyed ease type named by the hash of an ease attribute's value; an unknown hash is 0 (linear).
pub fn keyed_ease_type(value_hash: u32) -> u32 {
    match value_hash {
        0x2e2f_833f => 1,
        0x6de1_3d08 => 2,
        0x1b66_8e8a => 3,
        0x209f_7361 => 4,
        0xf6c0_b95d => 5,
        0xb823_ac10 => 6,
        0x0293_172d => 7,
        0xe418_ed97 => 8,
        0x879a_6e76 => 9,
        _ => 0,
    }
}

/// Keyed-modifier ease `ease_type` (1..9) at u; 0 or any other type is linear.
pub fn keyed_ease(ease_type: u32, u: f32) -> f32 {
    // Only the sine eases clamp u to 0..1.
    let clamped: f32 = u.clamp(0.0, 1.0);
    match ease_type {
        1 => 1.0 - (clamped * FRAC_PI_2).cos(),
        2 => (clamped * FRAC_PI_2).sin(),
        3 => 0.5 - 0.5 * (clamped * PI).cos(),
        4 => u * u,
        5 => 1.0 - (1.0 - u) * (1.0 - u),
        6 => u * u * u,
        7 => 1.0 - (1.0 - u).powi(3),
        8 => u.powi(4),
        9 => 1.0 - (1.0 - u).powi(4),
        _ => u,
    }
}

/// A driver value mapped into a keyed dimension: unchanged for range (0, 1), else (clamp(x) - min) / max(max - min, 0.001).
#[allow(clippy::float_cmp)]
pub fn keyed_driver_position(driver: f32, min: f32, max: f32) -> f32 {
    if min == 0.0 && max == 1.0 {
        return driver;
    }
    // max then min, not f32::clamp, which panics when min > max.
    (driver.max(min).min(max) - min) / (max - min).max(KEY_SPAN_FLOOR)
}

/// The two keys a keyed modifier interpolates between and the eased position u (0..1) from lo to hi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyPick {
    /// Lower key.
    pub lo: usize,
    /// Upper key.
    pub hi: usize,
    /// Eased position from lo to hi.
    pub u: f32,
}

impl KeyPick {
    /// The per-key value list lerped from key lo to key hi by u: an entry's v1, or the key weights.
    pub fn blend(&self, per_key: &[f32]) -> Result<f32, FrameError> {
        let len: usize = per_key.len();
        let missing = |index: usize| FrameError::MissingValue { index, len };
        let low: f32 = *per_key.get(self.lo).ok_or_else(|| missing(self.lo))?;
        let high: f32 = *per_key.get(self.hi).ok_or_else(|| missing(self.hi))?;
        Ok(low + (high - low) * self.u)
    }
}

/// The keys around position x: the first key from index 1 at or above x and the one before it, or key 0 twice past the last key.
pub fn pick_key(keys: &[f32], x: f32, ease_type: u32) -> Result<KeyPick, FrameError> {
    if keys.is_empty() {
        return Err(FrameError::EmptyTable);
    }
    let mut lo: usize = 0;
    let mut hi: usize = 0;
    for (i, key) in keys.iter().enumerate().skip(1) {
        if *key >= x {
            lo = i - 1;
            hi = i;
            break;
        }
    }
    let span: f32 = (keys[hi] - keys[lo]).max(KEY_SPAN_FLOOR);
    let u: f32 = keyed_ease(ease_type, ((x - keys[lo]) / span).clamp(0.0, 1.0));
    Ok(KeyPick { lo, hi, u })
}

/// A keyed entry's value from its 1, 2 or 4 blend samples, with a and b the other dimensions' eased positions.
pub fn combine_samples(samples: &[f32], a: f32, b: f32) -> Result<f32, FrameError> {
    match samples {
        [s0] => Ok(*s0),
        [s0, s1] => Ok(lerp(*s0, *s1, a)),
        [s0, s1, s2, s3] => Ok(lerp(lerp(*s0, *s1, a), lerp(*s2, *s3, a), b)),
        _ => Err(FrameError::SampleCount(samples.len())),
    }
}

/// Case-folded Jenkins one-at-a-time hash, seed 0: A..Z to a..z, `\` to `/`; a leading `"` is dropped and the next `"` or a NUL ends the name.
pub fn name_hash(name: &str) -> u32 {
    let bytes: &[u8] = name.as_bytes();
    let (body, quoted): (&[u8], bool) = match bytes.strip_prefix(b"\"") {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    let mut h: u32 = 0;
    for &c in body {
        if c == 0 || (quoted && c == b'"') {
            break;
        }
        let folded: u8 = if c == b'\\' {
            b'/'
        } else {
            c.to_ascii_lowercase()
        };
        h = h.wrapping_add(folded as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

#[cfg(test)]
#[path = "gta6_day_frame_tests.rs"]
mod tests;
