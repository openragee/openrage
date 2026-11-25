//! Arithmetic of the post chain: the TAA jitter, resolve weights, history weight and block terms, the exposure
//! curve, limits, read-back window and tone members, the tone sets, the light multiplier and the colour grade's
//! packed constants. The grade's pixel arithmetic and the exposure's GPU adaptation were not read, so only their
//! inputs are built here.

use glam::{Vec2, Vec3, Vec4};

/// Floats in one timecycle frame.
pub const FRAME_FLOATS: usize = 1408;

/// One timecycle frame, indexed by variable.
pub type TimecycleFrame = [f32; FRAME_FLOATS];

// Trap: NaN maps to 0, as a shader saturate does; f32::clamp would pass it through.
fn saturate(x: f32) -> f32 {
    if x > 0.0 {
        x.min(1.0)
    } else {
        0.0
    }
}

// Trap: an empty range (high == low) is a step at low, so no 0/0 reaches a caller.
fn ramp(x: f32, low: f32, high: f32) -> f32 {
    if high == low {
        if x >= low {
            1.0
        } else {
            0.0
        }
    } else {
        saturate((x - low) / (high - low))
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Points in the jitter table.
pub const JITTER_POINTS: usize = 16;

/// Jitter period in frames until the build's value is read; inferred.
pub const STANDIN_JITTER_PERIOD: usize = 8;

/// Jitter scale in pixels until the build's value is read; inferred.
pub const STANDIN_JITTER_SCALE: f32 = 0.5;

// Only called with bases 2 and 3: base 0 divides by zero and base 1 never ends.
fn radical_inverse(index: u32, base: u32) -> f32 {
    let b = f64::from(base);
    let mut result: f64 = 0.0;
    let mut fraction: f64 = 1.0;
    let mut rest = index;
    while rest > 0 {
        fraction /= b;
        result += fraction * f64::from(rest % base);
        rest /= base;
    }
    result as f32
}

/// The 16 Halton(2, 3) points mapped to [-1, 1]; entry i - 1 holds index i.
pub fn jitter_table() -> [Vec2; JITTER_POINTS] {
    let mut table = [Vec2::ZERO; JITTER_POINTS];
    for (slot, point) in table.iter_mut().enumerate() {
        let index = slot as u32 + 1;
        let unit = Vec2::new(radical_inverse(index, 2), radical_inverse(index, 3));
        *point = unit * 2.0 - Vec2::ONE;
    }
    table
}

/// One frame's jitter, its change since last frame and the resolve's sub-texel offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JitterStep {
    /// Table point times the scale, in pixels.
    pub jitter: Vec2,
    /// Half the change from last frame's jitter, in pixels.
    pub delta: Vec2,
    /// Offset the resolve weights centre on, (-0.5 x, 0.5 y) of the jitter, in texels.
    pub resolve_offset: Vec2,
}

/// The jitter cycle: table, index, period and scale.
#[derive(Clone, Debug)]
pub struct JitterSequence {
    table: [Vec2; JITTER_POINTS],
    index: usize,
    period: usize,
    scale: f32,
    previous: Vec2,
}

impl JitterSequence {
    /// A cycle of `period` frames (held to 1..=16) scaled by `scale` pixels, at index 0.
    pub fn new(period: usize, scale: f32) -> Self {
        JitterSequence {
            table: jitter_table(),
            index: 0,
            period: period.clamp(1, JITTER_POINTS),
            scale,
            previous: Vec2::ZERO,
        }
    }

    /// Advances one frame and gives its jitter.
    pub fn advance(&mut self) -> JitterStep {
        // The index moves before the read, so the first frame uses table entry 1.
        self.index = (self.index + 1) % self.period;
        let jitter = self.table[self.index] * self.scale;
        let delta = (jitter - self.previous) * 0.5;
        self.previous = jitter;
        JitterStep {
            jitter,
            delta,
            resolve_offset: Vec2::new(-0.5 * jitter.x, 0.5 * jitter.y),
        }
    }
}

/// Falloff of the Gaussian resolve kernel, per squared texel.
pub const GAUSSIAN_FALLOFF: f32 = 2.29;

/// Offsets of the nine neighbourhood taps in texels, in the order their weights are stored.
pub const NEIGHBOURHOOD_TAPS: [[f32; 2]; 9] = [
    [-1.0, -1.0],
    [0.0, -1.0],
    [1.0, -1.0],
    [-1.0, 0.0],
    [0.0, 0.0],
    [1.0, 0.0],
    [-1.0, 1.0],
    [0.0, 1.0],
    [1.0, 1.0],
];

/// Offsets of the five plus taps in texels, in the order their weights are stored.
pub const PLUS_TAPS: [[f32; 2]; 5] = [[0.0, -1.0], [-1.0, 0.0], [0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

/// The resolve's reconstruction kernel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolveKernel {
    /// clamp(1 - d, 0, 1) per axis, the default.
    Tent,
    /// Catmull-Rom per axis.
    CatmullRom,
    /// Mitchell-Netravali with B = C = 1/3 per axis.
    Mitchell,
    /// exp(-2.29 (1 + 0.5 s)^2 r^2) over the squared radius, with parameter s.
    Gaussian(f32),
}

fn tent(d: f32) -> f32 {
    saturate(1.0 - d)
}

// Trap: past d = 2 the polynomial goes negative; the kernel has no zero branch there.
fn catmull_rom(d: f32) -> f32 {
    if d <= 1.0 {
        1.5 * d * d * d - 2.5 * d * d + 1.0
    } else {
        -0.5 * d * d * d + 2.5 * d * d - 4.0 * d + 2.0
    }
}

fn mitchell(d: f32) -> f32 {
    if d < 1.0 {
        (7.0 * d * d * d - 12.0 * d * d + 16.0 / 3.0) / 6.0
    } else if d <= 2.0 {
        (-7.0 / 3.0 * d * d * d + 12.0 * d * d - 20.0 * d + 32.0 / 3.0) / 6.0
    } else {
        0.0
    }
}

impl ResolveKernel {
    /// The kernel's weight at a tap displaced (dx, dy) texels from the sample.
    pub fn weight(self, dx: f32, dy: f32) -> f32 {
        match self {
            ResolveKernel::Tent => tent(dx.abs()) * tent(dy.abs()),
            ResolveKernel::CatmullRom => catmull_rom(dx.abs()) * catmull_rom(dy.abs()),
            ResolveKernel::Mitchell => mitchell(dx.abs()) * mitchell(dy.abs()),
            ResolveKernel::Gaussian(s) => {
                let spread = 1.0 + 0.5 * s;
                (-GAUSSIAN_FALLOFF * spread * spread * (dx * dx + dy * dy)).exp()
            }
        }
    }
}

// Trap: a zero sum is left as it is rather than divided into NaN.
fn normalise(weights: &mut [f32]) {
    let sum: f32 = weights.iter().sum();
    if sum != 0.0 {
        for weight in weights.iter_mut() {
            *weight /= sum;
        }
    }
}

fn tap_weights(kernel: ResolveKernel, offset: Vec2, taps: &[[f32; 2]], weights: &mut [f32]) {
    for (weight, tap) in weights.iter_mut().zip(taps.iter()) {
        *weight = kernel.weight(tap[0] - offset.x, tap[1] - offset.y);
    }
}

/// The nine neighbourhood weights around `offset` texels, normalised to sum 1 when `normalise_sum` holds.
pub fn neighbourhood_weights(kernel: ResolveKernel, offset: Vec2, normalise_sum: bool) -> [f32; 9] {
    let mut weights = [0.0_f32; 9];
    tap_weights(kernel, offset, &NEIGHBOURHOOD_TAPS, &mut weights);
    if normalise_sum {
        normalise(&mut weights);
    }
    weights
}

/// The five plus weights around `offset` texels, normalised to sum 1 unless every weight is 0.
pub fn plus_weights(kernel: ResolveKernel, offset: Vec2) -> [f32; 5] {
    let mut weights = [0.0_f32; 5];
    tap_weights(kernel, offset, &PLUS_TAPS, &mut weights);
    normalise(&mut weights);
    weights
}

/// exp(-dt / T), the share kept after `dt` seconds with time constant `time_constant` seconds.
pub fn decay(dt: f32, time_constant: f32) -> f32 {
    // Trap: every TAA time constant is zero in the image; 0 is the limit for dt > 0 and avoids 0/0.
    if time_constant <= 0.0 {
        return 0.0;
    }
    (-dt / time_constant).exp()
}

/// Tunables of the history weight: time constants in seconds, speeds as the caller measures them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistoryTimes {
    /// Time constant the weight is recomputed from (tunable 0xa40b9617).
    pub base: f32,
    /// Time constant of the second history factor (tunable 0x758b5fdf).
    pub second: f32,
    /// Time constant the weight moves toward with camera speed (tunable 0xb08355ef).
    pub moving: f32,
    /// Speed along the view where the move starts (tunable 0x1bab71d7).
    pub speed_low: f32,
    /// Speed along the view where the move is complete (tunable 0xd226b0ea).
    pub speed_high: f32,
}

/// History weight that a forced frame takes.
pub const FORCED_HISTORY_WEIGHT: f32 = 0.5;

/// sp = -dot(the view-matrix elements at bytes 0x48, 0x58, 0x68, the view vector at +0x130 (not a timecycle variable)); that it is the camera's speed along the view: inferred.
pub fn speed_along_view(view_elements: Vec3, view_130: Vec3) -> f32 {
    -view_elements.dot(view_130)
}

/// This frame's (history weight, second factor) from last frame's pair, `dt` in seconds and `speed_along_view`'s sp.
pub fn history_weight(
    previous: f32,
    previous_second: f32,
    dt: f32,
    times: &HistoryTimes,
    recompute: bool,
    speed_along_view: f32,
    force_half: bool,
) -> (f32, f32) {
    // The second factor when the mode flag is not 1: not stated; keeping last frame's is the viewer's choice.
    let second = if recompute {
        decay(dt, times.second)
    } else {
        previous_second
    };
    if force_half {
        return (FORCED_HISTORY_WEIGHT, second);
    }
    let base = if recompute {
        decay(dt, times.base)
    } else {
        previous
    };
    let k = ramp(speed_along_view, times.speed_low, times.speed_high);
    (lerp(base, decay(dt, times.moving), k), second)
}

/// TAA tunables of the block's response factors and clamp remaps; every one is zero in the image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaaTunables {
    /// Time constants of e1, e2, e3: T(0x2847c733), T(0xea259690), T(0xcbbfcc14).
    pub response_times: [f32; 3],
    /// Ranges (L, H): (T(0xcf503036), T(0x2982b866)) and (T(0x1f2cc217), T(0xc4dc8450)).
    pub remap_ranges: [Vec2; 2],
    /// T(0x144df584), tested against 0.5.
    pub switch: f32,
}

/// The TAA block's members computed from the tunables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaaTerms {
    /// e1, e2, e3 = exp(-dt / T), at +0x0f8, +0x100 and +0x10c.
    pub response: [f32; 3],
    /// (L, 1 / (H - L)) of each range, at +0x0f0 and +0x104.
    pub remaps: [Vec2; 2],
    /// +0x110.x: 1 when the switch is above 0.5, else 0.
    pub switch: f32,
}

/// The TAA block's tunable-driven members for `dt` seconds.
pub fn taa_terms(dt: f32, tunables: &TaaTunables) -> TaaTerms {
    let remap = |range: Vec2| -> Vec2 {
        let span = range.y - range.x;
        // Trap: every T is zero in the image, so H - L = 0; 0 stands in for the build's 1/0 (the viewer's choice).
        Vec2::new(range.x, if span == 0.0 { 0.0 } else { 1.0 / span })
    };
    TaaTerms {
        response: tunables.response_times.map(|time| decay(dt, time)),
        remaps: tunables.remap_ranges.map(remap),
        switch: if tunables.switch > 0.5 { 1.0 } else { 0.0 },
    }
}

/// Timecycle variables (x, y) of the exposure curve's seven knots, in order.
pub const EXPOSURE_KNOT_VARIABLES: [[usize; 2]; 7] = [
    [0x306, 0x307],
    [0x30a, 0x30b],
    [0x30e, 0x30f],
    [0x312, 0x313],
    [0x314, 0x315],
    [0x316, 0x317],
    [0x318, 0x319],
];

/// The exposure curve's seven knots (x, y) from a frame; x metered log2 luminance and y EV offset, both inferred.
pub fn exposure_knots(frame: &TimecycleFrame) -> [Vec2; 7] {
    let mut knots = [Vec2::ZERO; 7];
    for (knot, variables) in knots.iter_mut().zip(EXPOSURE_KNOT_VARIABLES.iter()) {
        *knot = Vec2::new(frame[variables[0]], frame[variables[1]]);
    }
    knots
}

/// Stand-in curve value at `ev`: a polyline through the seven knots, held at the end knots outside them.
pub fn exposure_offset(knots: &[Vec2; 7], ev: f32) -> f32 {
    // Stand-in: the build bakes two 4-knot spline pieces (0x306..0x313, 0x312..0x319) into texture 0x723a07e6,
    // in a form not read; this polyline through all seven knots is the viewer's choice, not the build's.
    if ev <= knots[0].x {
        return knots[0].y;
    }
    for pair in knots.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if ev <= end.x {
            let span = end.x - start.x;
            if span <= 0.0 {
                return end.y;
            }
            return lerp(start.y, end.y, (ev - start.x) / span);
        }
    }
    knots[6].y
}

/// B+0x3f0 = (first x, last x, min y, max y) from the end knots only; units inferred.
pub fn exposure_curve_bounds(knots: &[Vec2; 7]) -> Vec4 {
    let first = knots[0];
    let last = knots[6];
    Vec4::new(first.x, last.x, first.y.min(last.y), first.y.max(last.y))
}

/// Timecycle variable of the lowest exposure.
pub const EXPOSURE_MIN_VARIABLE: usize = 0x31c;

/// Timecycle variable of the highest exposure.
pub const EXPOSURE_MAX_VARIABLE: usize = 0x31d;

/// Timecycle variables summed into the exposure compensation.
pub const EXPOSURE_COMPENSATION_VARIABLES: [usize; 2] = [0x31a, 0x31b];

/// Timecycle variable whose value above 0 resets the exposure window.
pub const EXPOSURE_RESET_VARIABLE: usize = 0x31e;

/// The frame's part of the window reset, v0x31e > 0; the build ORs it with two run-time predicates not read.
pub fn exposure_reset(frame: &TimecycleFrame) -> bool {
    frame[EXPOSURE_RESET_VARIABLE] > 0.0
}

/// The exposure limits and compensation, in EV.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureLimits {
    /// Lowest exposure, in EV.
    pub min_ev: f32,
    /// Highest exposure, in EV.
    pub max_ev: f32,
    /// Compensation added to the exposure, in EV.
    pub compensation_ev: f32,
}

impl ExposureLimits {
    /// The limits a frame states.
    pub fn from_frame(frame: &TimecycleFrame) -> Self {
        ExposureLimits {
            min_ev: frame[EXPOSURE_MIN_VARIABLE],
            max_ev: frame[EXPOSURE_MAX_VARIABLE],
            compensation_ev: frame[EXPOSURE_COMPENSATION_VARIABLES[0]]
                + frame[EXPOSURE_COMPENSATION_VARIABLES[1]],
        }
    }

    /// Where `ev` sits between the limits, saturated to [0, 1].
    pub fn range(&self, ev: f32) -> f32 {
        ramp(ev, self.min_ev, self.max_ev)
    }

    /// `ev` held to the limits, then shifted by the compensation, in EV.
    pub fn clamp(&self, ev: f32) -> f32 {
        // Inferred: the shader's order of clamp and compensation was not read.
        // Trap: max then min, not f32::clamp, which panics when min_ev > max_ev.
        ev.max(self.min_ev).min(self.max_ev) + self.compensation_ev
    }
}

/// Frames a reset keeps the window empty.
pub const EXPOSURE_RESET_FRAMES: u32 = 3;

/// One frame's input to the exposure window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowInput {
    /// Read-back exposure, in EV.
    pub exposure: f32,
    /// The value at [0x14abbef4c]; that it is the frame time in seconds: inferred.
    pub dt: f32,
    /// Starts the empty countdown again.
    pub reset: bool,
    /// Keeps the countdown from moving this frame.
    pub hold: bool,
    /// The live value stands and nothing is pushed.
    pub live: bool,
    /// Ignores the countdown this frame.
    pub force: bool,
}

/// The window's mean exposure in EV and mean dt, and whether the live value stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMean {
    /// Mean read-back exposure, in EV.
    pub exposure: f32,
    /// Mean of the inputs' dt; that it is the frame time in seconds: inferred.
    pub dt: f32,
    /// The live value stands instead of the mean.
    pub live: bool,
}

impl WindowMean {
    /// The exposure the range uses, in EV: the live read-back when live, else the mean.
    pub fn metered(&self, live_exposure: f32) -> f32 {
        if self.live {
            live_exposure
        } else {
            self.exposure
        }
    }
}

/// A box mean over the last N read-back exposures, with the reset that empties it.
#[derive(Clone, Debug)]
pub struct ExposureWindow {
    entries: Vec<Vec2>,
    head: usize,
    count: usize,
    countdown: u32,
}

impl ExposureWindow {
    /// An empty window of `length` frames, at least 1.
    pub fn new(length: usize) -> Self {
        ExposureWindow {
            entries: vec![Vec2::ZERO; length.max(1)],
            head: 0,
            count: 0,
            countdown: 0,
        }
    }

    /// Changes the window to `length` frames (at least 1), emptying it when the length changes.
    pub fn set_length(&mut self, length: usize) {
        let length = length.max(1);
        if length != self.entries.len() {
            self.entries = vec![Vec2::ZERO; length];
            self.head = 0;
            self.count = 0;
        }
    }

    /// Takes one frame and gives the window's mean.
    pub fn update(&mut self, input: &WindowInput) -> WindowMean {
        if input.reset {
            self.countdown = EXPOSURE_RESET_FRAMES;
        }
        if self.countdown > 0 && !input.force {
            // Read from this build: the reset zeroes head and count and leaves the entries in place.
            self.head = 0;
            self.count = 0;
            if !input.hold {
                self.countdown -= 1;
            }
            return WindowMean {
                exposure: 0.0,
                dt: 1.0,
                live: true,
            };
        }
        if !input.live {
            self.entries[self.head] = Vec2::new(input.exposure, input.dt);
            self.head = (self.head + 1) % self.entries.len();
            // Trap: the count follows the advanced head, so a part-filled window also sums the slot at head.
            self.count = self.count.max(self.head + 1);
        }
        let mut sum = Vec2::ZERO;
        for entry in &self.entries[..self.count] {
            sum += *entry;
        }
        let mean = sum / (self.count.max(1) as f32);
        WindowMean {
            exposure: mean.x,
            dt: mean.y,
            live: input.live,
        }
    }
}

/// Exposure tunables B carries; `Default` gives the image defaults, which the tunables file may change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureTunables {
    /// T(0x92715b9d), B+0x3a8; image default 0.15.
    pub b3a8: f32,
    /// T(0x443c77e0), B+0x3ac; image default 15.0.
    pub b3ac: f32,
    /// T(0x41da28d4), B+0x434; image default 1e-6.
    pub b434: f32,
    /// T(0x0e688cd6), B+0x438; image default 100000.
    pub b438: f32,
}

impl Default for ExposureTunables {
    fn default() -> Self {
        ExposureTunables {
            b3a8: 0.15,
            b3ac: 15.0,
            b434: 1e-6,
            b438: 100_000.0,
        }
    }
}

/// B's tone members that come from the timecycle or the window; the rest of the section is tunables, splines or 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneMembers {
    /// B+0x3a0: (mean E, mean dt, T(0x92715b9d), T(0x443c77e0)).
    pub b3a0: Vec4,
    /// B+0x3b0: v0x330.
    pub b3b0: f32,
    /// B+0x3e8: v0x332 unchanged; that it is the local exposure weight: inferred.
    pub b3e8: f32,
    /// B+0x3f0: the curve bounds.
    pub b3f0: Vec4,
    /// B+0x400: v0x300.
    pub b400: f32,
    /// B+0x404: v0x322.
    pub b404: f32,
    /// B+0x410: v0x2ff > 0.
    pub b410: bool,
    /// B+0x420: (v0x31c, v0x31d, v0x31a + v0x31b, 0).
    pub b420: Vec4,
    /// B+0x430: the constant -11.0; that it is a metering EV floor or offset: inferred.
    pub b430: f32,
    /// B+0x434, +0x438: T(0x41da28d4), T(0x0e688cd6).
    pub b434: Vec2,
    /// B+0x500: v0x36b > 0.
    pub b500: bool,
    /// B+0x514: v0x286 > 0.
    pub b514: bool,
    /// B+0x51c: (v0x36c, v0x36d).
    pub b51c: Vec2,
    /// B+0x524: |v0x36c - 1| > 0.001.
    pub b524: bool,
}

/// B's tone members from a frame, the window's mean and the exposure tunables.
pub fn tone_members(
    frame: &TimecycleFrame,
    mean: &WindowMean,
    tunables: &ExposureTunables,
) -> ToneMembers {
    let limits = ExposureLimits::from_frame(frame);
    ToneMembers {
        b3a0: Vec4::new(mean.exposure, mean.dt, tunables.b3a8, tunables.b3ac),
        b3b0: frame[0x330],
        b3e8: frame[0x332],
        b3f0: exposure_curve_bounds(&exposure_knots(frame)),
        b400: frame[0x300],
        b404: frame[0x322],
        b410: frame[0x2ff] > 0.0,
        b420: Vec4::new(limits.min_ev, limits.max_ev, limits.compensation_ev, 0.0),
        b430: -11.0,
        b434: Vec2::new(tunables.b434, tunables.b438),
        b500: frame[0x36b] > 0.0,
        b514: frame[0x286] > 0.0,
        b51c: Vec2::new(frame[0x36c], frame[0x36d]),
        b524: (frame[0x36c] - 1.0).abs() > 0.001,
    }
}

/// Float of the blended tone set that scales CPU-side light: lane 2 of its second vec4.
pub const TONE_LIGHT_LANE: usize = 6;

/// t = saturate((x - a) / (b - a)), divisor 1 when b == a; x the exposure reader's value (that it is E: inferred), a and b engine values of unknown source.
pub fn tone_blend_weight(x: f32, a: f32, b: f32) -> f32 {
    let span = if b == a { 1.0 } else { b - a };
    saturate((x - a) / span)
}

/// The eight tone floats lerped from set A to set B by `t`.
pub fn blend_tone_sets(set_a: &[f32; 8], set_b: &[f32; 8], t: f32) -> [f32; 8] {
    let mut blended = [0.0_f32; 8];
    for (lane, value) in blended.iter_mut().enumerate() {
        *value = lerp(set_a[lane], set_b[lane], t);
    }
    blended
}

/// Timecycle variables of the two tone override flags.
pub const TONE_OVERRIDE_FLAGS: [usize; 2] = [0x344, 0x34d];

/// First timecycle variables of the two eight-float tone override sets.
pub const TONE_OVERRIDE_SETS: [usize; 2] = [0x345, 0x34e];

/// The override sets in variable order when either flag is above 0; which one is set A, and their lane order: not read.
pub fn tone_override_sets(frame: &TimecycleFrame) -> Option<[[f32; 8]; 2]> {
    if !TONE_OVERRIDE_FLAGS.iter().any(|&flag| frame[flag] > 0.0) {
        return None;
    }
    let mut sets = [[0.0_f32; 8]; 2];
    for (set, &first) in sets.iter_mut().zip(TONE_OVERRIDE_SETS.iter()) {
        set.copy_from_slice(&frame[first..first + 8]);
    }
    Some(sets)
}

/// The CPU light multiplier exp2(-E) times the blended tone set's light lane, for read-back E in EV.
pub fn light_scale(exposure: f32, blended_tone_set: &[f32; 8]) -> f32 {
    (-exposure).exp2() * blended_tone_set[TONE_LIGHT_LANE]
}

/// First timecycle variable of grade set 1.
pub const GRADE_SET_1: usize = 0x2af;

/// First timecycle variable of grade set 2.
pub const GRADE_SET_2: usize = 0x2c5;

/// Timecycle variable of the blend weight between the grade sets.
pub const GRADE_BLEND_VARIABLE: usize = 0x2e0;

/// Floats in the packed grade block (0x94 bytes).
pub const GRADE_BLOCK_FLOATS: usize = 37;

/// Gamma added per step of the display setting, about 0.053333.
const GRADE_GAMMA_STEP: f32 = f32::from_bits(0x3d5a_740f);

/// Gamma at a display setting of 0.
const GRADE_GAMMA_BASE: f32 = 1.6;

/// Least magnitude of the pivot reciprocals' divisors, so p = 0 and p = 1 stay finite; the viewer's guard, the build has none.
pub const GRADE_PIVOT_MARGIN: f32 = 1e-4;

/// The grade's gamma g from the display setting: 2.4 at 15.
pub fn grade_gamma(setting: u32) -> f32 {
    setting as f32 * GRADE_GAMMA_STEP + GRADE_GAMMA_BASE
}

/// One packed grade set; lane names follow the block's c0..c4.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeSet {
    /// c0: shadow colour x intensity, to the power 1/g, x scale 0; w = 1 - 2 x shadow width.
    pub shadow: Vec4,
    /// c1: mid colour x intensity x scale 1, linear; w = the pivot p.
    pub mid: Vec4,
    /// c2: highlight colour x intensity, to the power 1/g, x scale 2; w = 1 - 2 x highlight width.
    pub highlight: Vec4,
    /// c3: (1 / (1 - p), 1 / p, extra 0, extra 1).
    pub pivot: Vec4,
    /// c4: extra 2.
    pub extra: f32,
    /// The set's trailing scalar, packed after both sets.
    pub scalar: f32,
}

impl GradeSet {
    /// This set lerped toward `other` by `t`, member by member.
    pub fn lerp(&self, other: &GradeSet, t: f32) -> GradeSet {
        GradeSet {
            shadow: self.shadow.lerp(other.shadow, t),
            mid: self.mid.lerp(other.mid, t),
            highlight: self.highlight.lerp(other.highlight, t),
            pivot: self.pivot.lerp(other.pivot, t),
            extra: lerp(self.extra, other.extra, t),
            scalar: lerp(self.scalar, other.scalar, t),
        }
    }
}

// Trap: the build's pow has no guard for negatives; they are held at 0 here instead of giving NaN.
fn power(colour: Vec3, k: f32) -> Vec3 {
    Vec3::new(
        colour.x.max(0.0).powf(k),
        colour.y.max(0.0).powf(k),
        colour.z.max(0.0).powf(k),
    )
}

// Only a divisor nearer 0 than the margin moves, to the margin with its sign kept; every other pivot is the build's.
fn away_from_zero(x: f32) -> f32 {
    if x.abs() < GRADE_PIVOT_MARGIN {
        GRADE_PIVOT_MARGIN.copysign(x)
    } else {
        x
    }
}

// Offsets from `base`: colours 0..9, pivot 9, widths 10 and 11, scales 12..15, intensities 15..18,
// scalar 18, extras 19..22.
fn grade_set(frame: &TimecycleFrame, base: usize, gamma: f32) -> GradeSet {
    let k = 1.0 / gamma;
    let value = |offset: usize| -> f32 { frame[base + offset] };
    let shadow = Vec3::new(value(0), value(1), value(2)) * value(0x0f);
    let mid = Vec3::new(value(3), value(4), value(5)) * value(0x10);
    let highlight = Vec3::new(value(6), value(7), value(8)) * value(0x11);
    let pivot = value(9);
    GradeSet {
        shadow: (power(shadow, k) * value(12)).extend(1.0 - 2.0 * value(10)),
        mid: (mid * value(13)).extend(pivot),
        highlight: (power(highlight, k) * value(14)).extend(1.0 - 2.0 * value(11)),
        pivot: Vec4::new(
            1.0 / away_from_zero(1.0 - pivot),
            1.0 / away_from_zero(pivot),
            value(0x13),
            value(0x14),
        ),
        extra: value(0x15),
        scalar: value(0x12),
    }
}

fn write_grade_set(out: &mut [f32; GRADE_BLOCK_FLOATS], start: usize, set: &GradeSet) {
    let vectors = [set.shadow, set.mid, set.highlight, set.pivot];
    for (i, vector) in vectors.iter().enumerate() {
        let at = start + 4 * i;
        out[at..at + 4].copy_from_slice(&vector.to_array());
    }
    out[start + 16] = set.extra;
}

/// Both grade sets and the weight that blends them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeBlock {
    /// Set 1.
    pub first: GradeSet,
    /// Set 2.
    pub second: GradeSet,
    /// Blend weight t from set 1 toward set 2.
    pub blend: f32,
}

impl GradeBlock {
    /// Both grade sets and their blend weight from a frame, for gamma g.
    pub fn from_frame(frame: &TimecycleFrame, gamma: f32) -> Self {
        GradeBlock {
            first: grade_set(frame, GRADE_SET_1, gamma),
            second: grade_set(frame, GRADE_SET_2, gamma),
            blend: frame[GRADE_BLEND_VARIABLE],
        }
    }

    /// The 37 floats of the packed block: set 1 at 0, set 2 at 17, the scalars at 34 and 35, t at 36.
    pub fn to_floats(&self) -> [f32; GRADE_BLOCK_FLOATS] {
        let mut out = [0.0_f32; GRADE_BLOCK_FLOATS];
        write_grade_set(&mut out, 0, &self.first);
        write_grade_set(&mut out, 17, &self.second);
        out[34] = self.first.scalar;
        out[35] = self.second.scalar;
        out[36] = self.blend;
        out
    }

    /// Set 1 lerped toward set 2 by the blend weight.
    pub fn blended(&self) -> GradeSet {
        // Inferred: the shader blends the sets by t; where in its arithmetic was not read.
        self.first.lerp(&self.second, self.blend)
    }
}

#[cfg(test)]
#[path = "post_math_tests.rs"]
mod tests;
