//! Sun and moon directions from the in-game clock, the two bodies' light weights and the directional light they make, with axes x east, y north, z up (inferred from the observer rotations).
//! Not known and so not applied: the third body's arithmetic (its direction is an argument), the special-object intensity factor, the camera or interior intensity override, the state flag that forces the pull to 1 and the final lerp toward a second direction.

use glam::Vec3;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Earth's axial tilt, radians (23.44 degrees).
pub const AXIAL_TILT: f64 = 0.409105;

/// Day of year the image starts with, before any setter runs.
pub const DEFAULT_DAY_OF_YEAR: i32 = 80;

/// Days added to the day of year so the season angle is 0 on day 80.
const SEASON_DAY_OFFSET: i32 = 285;

/// Exponent of the clock warp that slows the sun near sunrise and sunset.
pub const WARP_EXPONENT: f64 = 1.25;

/// Clock steps per day of the sunrise and sunset search.
const HORIZON_STEPS: u32 = 96;

/// Distance of the sample angle from 0 or pi that counts as the horizon, radians.
const HORIZON_TOLERANCE: f32 = 0.05;

/// Sample angle below which a turn is added, radians (the typed -3.14, not -pi).
const SAMPLE_ANGLE_WRAP_BELOW: f32 = f32::from_bits(0xc048_f5c3);

/// Turn added to a sample angle below the wrap, radians (the typed 6.28, not 2 pi).
const SAMPLE_ANGLE_TURN: f32 = f32::from_bits(0x40c8_f5c3);

/// Minutes past midnight of noon on a standard-time day.
const STANDARD_NOON_MINUTES: f64 = 720.0;

/// Minutes past midnight of noon on a daylight-saving day.
const DAYLIGHT_NOON_MINUTES: f64 = 780.0;

/// Minutes in a day.
const MINUTES_PER_DAY: f64 = 1440.0;

/// Degrees per radian, as the zenith angle rounds it.
const DEGREES_PER_RADIAN: f32 = 57.29578;

/// Value of v28 that leaves the leader to the zenith angles.
const AUTOMATIC_LEADER: f32 = -10.0;

/// Weight of the sun against the moon and the third body in the light direction, unitless.
const SUN_DIRECTION_WEIGHT: f32 = 100.0;

/// Weight sum at or below which the light has no direction (FLT_EPSILON).
const MIN_DIRECTION_WEIGHT: f32 = f32::EPSILON;

/// Lowest z of the light direction, so the light never comes from below about 8.6 degrees.
pub const MIN_LIGHT_Z: f32 = 0.15;

/// Moon-cycle scale that replaces a negative moon-phase tunable, per cycle.
const PHASE_PER_CYCLE: f32 = 1.0 / 26.0;

/// Intensity, and largest colour channel, below which the directional light is off.
const MIN_LIGHT_INTENSITY: f32 = 1e-6;

/// Day fraction (clock hours / 24) of the in-game clock's four integer fields.
pub fn clock_day_fraction(hours: i32, minutes: i32, seconds: i32, milliseconds: i32) -> f64 {
    let clock_hours = hours as f64
        + minutes as f64 / 60.0
        + seconds as f64 / 3600.0
        + milliseconds as f64 / 3.6e6;
    clock_hours / 24.0
}

/// Tunables that place the observer and the daylight-saving days; a data file may replace any of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunTunables {
    /// World y of the box's south edge, world units.
    pub south_y: f64,
    /// World y of the box's north edge, world units.
    pub north_y: f64,
    /// Latitude at the south edge, degrees north.
    pub south_latitude: f64,
    /// Latitude at the north edge, degrees north.
    pub north_latitude: f64,
    /// World x of the box's west edge, world units.
    pub west_x: f64,
    /// World x of the box's east edge, world units.
    pub east_x: f64,
    /// Longitude at the west edge, degrees east.
    pub west_longitude: f64,
    /// Longitude at the east edge, degrees east.
    pub east_longitude: f64,
    /// First daylight-saving day of the year.
    pub daylight_start_day: i32,
    /// First day of the year after daylight saving.
    pub daylight_end_day: i32,
}

impl SunTunables {
    /// The image defaults, with the east edge's world x supplied because its default was not read.
    pub fn image_defaults(east_x: f64) -> SunTunables {
        SunTunables {
            south_y: -10000.0,
            north_y: 10000.0,
            south_latitude: 24.0,
            north_latitude: 27.6,
            west_x: -11000.0,
            east_x,
            west_longitude: -81.8,
            east_longitude: -79.6,
            daylight_start_day: 79,
            daylight_end_day: 298,
        }
    }
}

/// The sun's day at the observer: unit direction, the day fractions of sunrise, solar noon and sunset, and the day phase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunDay {
    /// Unit vector toward the sun, x east, y north, z up.
    pub direction: Vec3,
    /// Day fraction of the first horizon step, or -1 when none was found.
    pub sunrise: f64,
    /// Day fraction of solar noon.
    pub noon: f64,
    /// Day fraction of the last horizon step, or -1 when fewer than two were found.
    pub sunset: f64,
    /// The warped day fraction the direction was computed at.
    pub warped_day_fraction: f64,
    /// Day phase P: 0.25 at sunrise, 0.5 at solar noon, 0.75 at sunset.
    pub day_phase: f64,
}

/// The sun for a day fraction, a day of year (clamped to 1..365), the EoT byte table and the observer's world position (the camera or focus, inferred); the warp byte is taken at its image value 1.
pub fn sun_day(
    day_fraction: f64,
    day_of_year: i32,
    equation_of_time: &[u8],
    observer: Vec3,
    tunables: &SunTunables,
) -> SunDay {
    let day = day_of_year.clamp(1, 365);
    // Indexed by the clamped day; index 365 is past the 365-byte table, where the image holds 0.
    let noon_offset_minutes = f64::from(equation_of_time.get(day as usize).copied().unwrap_or(0));
    let noon = solar_noon(day, noon_offset_minutes, tunables);
    let geometry = SunGeometry::new(day, noon, observer, tunables);
    let (sunrise, sunset) = geometry.horizon_crossings();
    let warped = warp_day_fraction(day_fraction, sunrise, sunset);
    SunDay {
        direction: geometry.direction(warped),
        sunrise,
        noon,
        sunset,
        warped_day_fraction: warped,
        day_phase: day_phase(warped, sunrise, noon, sunset),
    }
}

/// Day fraction of solar noon: 12:00 or 13:00 by daylight saving, plus the day's offset in minutes.
fn solar_noon(day_of_year: i32, noon_offset_minutes: f64, tunables: &SunTunables) -> f64 {
    let daylight = (tunables.daylight_start_day..tunables.daylight_end_day).contains(&day_of_year);
    let minutes = if daylight {
        DAYLIGHT_NOON_MINUTES
    } else {
        STANDARD_NOON_MINUTES
    };
    (minutes + noon_offset_minutes) / MINUTES_PER_DAY
}

/// Day phase P: 0.25 at sunrise, 0.5 at solar noon, 0.75 at sunset, piecewise linear in the warped day fraction, at most 1.
pub fn day_phase(warped: f64, sunrise: f64, noon: f64, sunset: f64) -> f64 {
    // Which side of each boundary a value equal to it falls on is not known; it takes the later piece.
    if warped < sunrise {
        0.25 * warped / sunrise
    } else if warped < noon {
        0.25 + 0.25 * (warped - sunrise) / (noon - sunrise)
    } else if warped < sunset {
        0.5 + 0.25 * (warped - noon) / (sunset - noon)
    } else {
        (0.75 + 0.25 * (warped - sunset) / (1.0 - sunset)).min(1.0)
    }
}

/// Unit direction of the sun in the Earth frame for a day of year.
fn season(day_of_year: i32) -> [f64; 3] {
    let angle = TAU * ((day_of_year + SEASON_DAY_OFFSET) % 365) as f64 / 365.0;
    let (sin_angle, cos_angle) = angle.sin_cos();
    [
        cos_angle,
        -sin_angle * AXIAL_TILT.cos(),
        sin_angle * AXIAL_TILT.sin(),
    ]
}

/// The observer's local axes and the day's constants, fixed for one frame.
struct SunGeometry {
    season: [f64; 3],
    east: [f64; 3],
    north: [f64; 3],
    up: [f64; 3],
    meridian_offset: f64,
    noon: f64,
}

impl SunGeometry {
    fn new(day_of_year: i32, noon: f64, observer: Vec3, tunables: &SunTunables) -> SunGeometry {
        let along_x = unit_interval(
            (observer.x as f64 - tunables.west_x) / (tunables.east_x - tunables.west_x),
        );
        let along_y = unit_interval(
            (observer.y as f64 - tunables.south_y) / (tunables.north_y - tunables.south_y),
        );
        let latitude =
            tunables.south_latitude + (tunables.north_latitude - tunables.south_latitude) * along_y;
        let longitude =
            tunables.west_longitude + (tunables.east_longitude - tunables.west_longitude) * along_x;
        let (sin_colatitude, cos_colatitude) = (FRAC_PI_2 - latitude * PI / 180.0).sin_cos();
        let (sin_longitude, cos_longitude) = (longitude * PI / 180.0).sin_cos();
        let season = season(day_of_year);
        // Signed angle from (cos lon, sin lon) to the season's xy, taking `cross` as the z of the
        // 2D cross product (inferred); atan2 ignores the positive scale that normalising that xy would remove.
        let cross = cos_longitude * season[1] - sin_longitude * season[0];
        let along = cos_longitude * season[0] + sin_longitude * season[1];
        SunGeometry {
            season,
            east: [-sin_longitude, cos_longitude, 0.0],
            north: [
                -cos_colatitude * cos_longitude,
                -cos_colatitude * sin_longitude,
                sin_colatitude,
            ],
            up: [
                sin_colatitude * cos_longitude,
                sin_colatitude * sin_longitude,
                cos_colatitude,
            ],
            meridian_offset: cross.atan2(along),
            noon,
        }
    }

    /// Unit vector toward the sun at a day fraction, used as given (no warp).
    fn direction(&self, day_fraction: f64) -> Vec3 {
        let hour_angle = (day_fraction - self.noon) * TAU + self.meridian_offset;
        let (sin_hour, cos_hour) = hour_angle.sin_cos();
        let a = self.season;
        let turned = [
            a[0] * cos_hour + a[1] * sin_hour,
            -a[0] * sin_hour + a[1] * cos_hour,
            a[2],
        ];
        // The two observer rotations reduce to components on the local east, north and up.
        Vec3::new(
            dot(turned, self.east) as f32,
            dot(turned, self.north) as f32,
            dot(turned, self.up) as f32,
        )
    }

    /// Sunrise and sunset day fractions: the first and the last of 96 steps near the horizon, each -1 when unset.
    fn horizon_crossings(&self) -> (f64, f64) {
        let mut sunrise = -1.0;
        let mut sunset = -1.0;
        for step in 0..HORIZON_STEPS {
            let t = step as f64 / HORIZON_STEPS as f64;
            let angle = sample_angle(self.direction(t));
            let near_horizon = angle.abs() < HORIZON_TOLERANCE
                || (angle - std::f32::consts::PI).abs() < HORIZON_TOLERANCE;
            if near_horizon {
                if sunrise < 0.0 {
                    sunrise = t;
                } else {
                    sunset = t;
                }
            }
        }
        (sunrise, sunset)
    }
}

/// Warped day fraction in [0, 1) of any day fraction, wrapped into [0, 1) first: each half of the day or night is eased by the 1.25 power at both ends.
fn warp_day_fraction(day_fraction: f64, sunrise: f64, sunset: f64) -> f64 {
    // Outside [sunset - 1, sunrise + 1] a power of a negative base would be NaN.
    let day_fraction = day_fraction.rem_euclid(1.0);
    // The game's warp without a sunrise and a later sunset was not read; the clock stays unwarped.
    if sunrise < 0.0 || sunset <= sunrise {
        return day_fraction;
    }
    let (start, end) = if day_fraction < sunrise {
        (sunset - 1.0, sunrise)
    } else if day_fraction <= sunset {
        (sunrise, sunset)
    } else {
        (sunset, sunrise + 1.0)
    };
    let half = (end - start) / 2.0;
    let warped = if day_fraction - start < half {
        start + half * ((day_fraction - start) / half).powf(WARP_EXPONENT)
    } else {
        end - half * ((end - day_fraction) / half).powf(WARP_EXPONENT)
    };
    warped.rem_euclid(1.0)
}

/// Sample-pick angle of the sun in radians on the east-up plane: 0 at the east horizon, pi/2 at the meridian, -pi/2 (3 pi/2 unwrapped) at the nadir.
pub fn sample_angle(sun: Vec3) -> f32 {
    let angle = sun.z.atan2(sun.x);
    if angle < SAMPLE_ANGLE_WRAP_BELOW {
        angle + SAMPLE_ANGLE_TURN
    } else {
        angle
    }
}

/// Tunables of the moon's orbit; a data file may replace any of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonTunables {
    /// Orbits per day, unitless.
    pub rate: f64,
    /// First orbit angle, degrees.
    pub orbit_angle: f64,
    /// Added to the first orbit angle to give the orbit's tilt, degrees.
    pub orbit_angle_offset: f64,
    /// Height of the orbit along its normal, unitless.
    pub height: f64,
    /// Swing of that height, unitless.
    pub height_swing: f64,
    /// Swings of the height per orbit, unitless.
    pub height_swing_rate: f64,
}

impl MoonTunables {
    /// The image defaults.
    pub fn image_defaults() -> MoonTunables {
        MoonTunables {
            rate: 1.0,
            orbit_angle: 122.0,
            orbit_angle_offset: 240.0,
            height: 0.8,
            height_swing: 0.1,
            height_swing_rate: 0.0,
        }
    }
}

/// The moon's cycle position p = fmod((days + day fraction) * rate, 1), from a whole-day count and the unwarped day fraction.
pub fn moon_cycle(days: i64, day_fraction: f64, rate: f64) -> f64 {
    ((days as f64 + day_fraction) * rate) % 1.0
}

/// Unit vector toward the moon (x east, y north, z up) at a cycle position.
pub fn moon_direction(cycle: f64, tunables: &MoonTunables) -> Vec3 {
    let (sin_turn, cos_turn) = (TAU * cycle - PI).sin_cos();
    let (sin_tilt, cos_tilt) =
        ((tunables.orbit_angle + tunables.orbit_angle_offset) * PI / 180.0).sin_cos();
    let height =
        tunables.height + tunables.height_swing * (TAU * cycle * tunables.height_swing_rate).sin();
    // height * (0, -sin tilt, cos tilt) - (-sin turn, cos turn cos tilt, cos turn sin tilt);
    // the two are orthogonal, so the length is sqrt(height^2 + 1) and never 0.
    let toward = [
        sin_turn,
        -height * sin_tilt - cos_turn * cos_tilt,
        height * cos_tilt - cos_turn * sin_tilt,
    ];
    let length = dot(toward, toward).sqrt();
    Vec3::new(
        (toward[0] / length) as f32,
        (toward[1] / length) as f32,
        (toward[2] / length) as f32,
    )
}

/// Zenith angle in degrees (0 overhead, 180 at the nadir) of a unit vector's z.
pub fn zenith_degrees(z: f32) -> f32 {
    z.clamp(-1.0, 1.0).acos() * DEGREES_PER_RADIAN
}

/// The timecycle variables the sun, the moon and the directional light read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightVariables {
    /// v0..v2, the light colour unless the third body has weight.
    pub day_colour: Vec3,
    /// v3, the light intensity unless the third body has weight.
    pub day_intensity: f32,
    /// v4, multiplier on the intensity weight when the sun leads.
    pub sun_multiplier: f32,
    /// v5, multiplier on the intensity weight when the moon leads.
    pub moon_multiplier: f32,
    /// v9..v11, the light colour when the third body has weight.
    pub third_colour: Vec3,
    /// v12, the light intensity when the third body has weight.
    pub third_intensity: f32,
    /// v13, the pull amount while the state flag is off.
    pub pull_amount: f32,
    /// v14, the scale of the discs' pull (not the light's).
    pub disc_pull_scale: f32,
    /// v18, the sun's pull amount.
    pub sun_pull: f32,
    /// v19, the moon's pull amount.
    pub moon_pull: f32,
    /// v20..v22, the vector the sun is pulled toward.
    pub sun_target: Vec3,
    /// v23..v25, the vector the moon is pulled toward.
    pub moon_target: Vec3,
    /// v26, the sun's zenith threshold in degrees, used when >= 0.
    pub sun_zenith_threshold: f32,
    /// v27, the moon's zenith threshold in degrees, used when >= 0.
    pub moon_zenith_threshold: f32,
    /// v28, the leader override: -10 automatic, > 0 sun, < 0 moon, 0 none.
    pub leader_override: f32,
}

impl LightVariables {
    /// Reads the light's variables from a timecycle frame by their indices.
    pub fn from_frame(frame: &crate::gta6_day_frame::TimecycleFrame) -> LightVariables {
        LightVariables {
            day_colour: Vec3::new(frame[0], frame[1], frame[2]),
            day_intensity: frame[3],
            sun_multiplier: frame[4],
            moon_multiplier: frame[5],
            third_colour: Vec3::new(frame[9], frame[10], frame[11]),
            third_intensity: frame[12],
            pull_amount: frame[13],
            disc_pull_scale: frame[14],
            sun_pull: frame[18],
            moon_pull: frame[19],
            sun_target: Vec3::new(frame[20], frame[21], frame[22]),
            moon_target: Vec3::new(frame[23], frame[24], frame[25]),
            sun_zenith_threshold: frame[26],
            moon_zenith_threshold: frame[27],
            leader_override: frame[28],
        }
    }
}

/// Tunables of the light weights and the moon-phase dimming; a data file may replace any of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightTunables {
    /// Sun zenith where its weight starts to fall, degrees (v26 replaces it when >= 0).
    pub sun_zenith_start: f32,
    /// Sun zenith span of a weight ramp, degrees.
    pub sun_zenith_ramp: f32,
    /// Moon zenith where its weight starts to fall, degrees (v27 replaces it when >= 0).
    pub moon_zenith_start: f32,
    /// Moon zenith span of a weight ramp, degrees.
    pub moon_zenith_ramp: f32,
    /// Moon phase in [0, 1]; a negative value takes the moon cycle / 26.
    pub moon_phase: f32,
    /// Fraction of the moonlight kept at the darkest phase.
    pub moon_floor: f32,
}

impl LightTunables {
    /// The image defaults.
    pub fn image_defaults() -> LightTunables {
        LightTunables {
            sun_zenith_start: 100.0,
            sun_zenith_ramp: 1.0,
            moon_zenith_start: 100.0,
            moon_zenith_ramp: 1.0,
            moon_phase: 0.5,
            moon_floor: 1.0,
        }
    }
}

/// The three bodies' light weights, each in [0, 1], at most one of them non-zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// The sun's weight.
    pub sun: f32,
    /// The moon's weight.
    pub moon: f32,
    /// The third body's weight.
    pub third: f32,
}

/// Which body's multiplier the intensity weight takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leader {
    /// The sun's weight is above both others.
    Sun,
    /// The moon's weight is above the third's and not below the sun's.
    Moon,
    /// No multiplier applies.
    Neither,
}

impl Weights {
    /// The leading body by the multiplier rule.
    pub fn leader(&self) -> Leader {
        if self.sun > self.moon && self.sun > self.third {
            Leader::Sun
        } else if self.moon > self.third {
            Leader::Moon
        } else {
            Leader::Neither
        }
    }
}

/// The three weights from the sun's and the moon's zenith angles in degrees.
pub fn body_weights(
    sun_zenith: f32,
    moon_zenith: f32,
    variables: &LightVariables,
    tunables: &LightTunables,
) -> Weights {
    let mut weights = Weights {
        sun: 0.0,
        moon: 0.0,
        third: 0.0,
    };
    let leader = variables.leader_override;
    if leader != AUTOMATIC_LEADER {
        if leader > 0.0 {
            weights.sun = leader.min(1.0);
        } else if leader < 0.0 {
            weights.moon = (-leader).min(1.0);
        }
    } else {
        let sun_start = if variables.sun_zenith_threshold >= 0.0 {
            variables.sun_zenith_threshold
        } else {
            tunables.sun_zenith_start
        };
        let moon_start = if variables.moon_zenith_threshold >= 0.0 {
            variables.moon_zenith_threshold
        } else {
            tunables.moon_zenith_start
        };
        // A ramp of 0 gives NaN at the start zenith, which no comparison below takes; the game's
        // handling of a zero ramp was not read.
        let sun_past = (sun_zenith - sun_start) / tunables.sun_zenith_ramp;
        let moon_past = (moon_zenith - moon_start) / tunables.moon_zenith_ramp;
        let sun_clamped = sun_past.clamp(-1.0, 1.0);
        let moon_clamped = moon_past.clamp(-1.0, 1.0);
        if sun_clamped < 0.0 {
            weights.sun = -sun_clamped;
        } else if moon_clamped < 0.0 {
            weights.moon = sun_past.min(1.0) * -moon_clamped;
        } else {
            weights.third = sun_past.min(1.0) * moon_past.min(1.0);
        }
    }
    weights.sun = weights.sun.clamp(0.0, 1.0);
    weights.moon = weights.moon.clamp(0.0, 1.0);
    weights.third = weights.third.clamp(0.0, 1.0);
    weights
}

/// Unit vector toward the light: the weighted, pulled blend of the bodies, floored at z 0.15.
fn light_direction(
    sun: Vec3,
    moon: Vec3,
    third: Vec3,
    weights: &Weights,
    variables: &LightVariables,
) -> Vec3 {
    let sum = SUN_DIRECTION_WEIGHT * weights.sun + weights.moon + weights.third;
    let blended = if sum <= MIN_DIRECTION_WEIGHT {
        Vec3::ZERO
    } else {
        let sun_share = SUN_DIRECTION_WEIGHT * weights.sun / sum;
        let moon_share = weights.moon / sum;
        let third_share = weights.third / sum;
        let base = sun * sun_share + moon * moon_share + third * third_share;
        let target = variables.sun_target * sun_share + variables.moon_target * moon_share;
        let pull = sun_share * variables.sun_pull.max(variables.pull_amount)
            + moon_share * variables.moon_pull.max(variables.pull_amount);
        (base + (target - base) * pull).normalize_or_zero()
    };
    floor_light_direction(blended)
}

/// Raises a direction's z to at least 0.15 and normalises it; a zero direction becomes (0, 0, 1).
pub fn floor_light_direction(direction: Vec3) -> Vec3 {
    Vec3::new(direction.x, direction.y, direction.z.max(MIN_LIGHT_Z)).normalize()
}

/// Intensity weight W: the largest weight, dimmed by the moon phase, times the leader's multiplier.
fn intensity_weight(
    weights: &Weights,
    moon_cycle: f32,
    variables: &LightVariables,
    tunables: &LightTunables,
) -> f32 {
    let largest = weights.sun.max(weights.moon).max(weights.third);
    let phase = if tunables.moon_phase < 0.0 {
        moon_cycle * PHASE_PER_CYCLE
    } else {
        tunables.moon_phase
    };
    let dimmed =
        largest * (1.0 - weights.moon * (1.0 - tunables.moon_floor) * (2.0 * phase - 1.0).abs());
    match weights.leader() {
        Leader::Sun => dimmed * variables.sun_multiplier,
        Leader::Moon => dimmed * variables.moon_multiplier,
        Leader::Neither => dimmed,
    }
}

/// Sky-disc unit directions (sun, moon), each pulled toward its target by v14 times its pull amount.
pub fn disc_directions(sun: Vec3, moon: Vec3, variables: &LightVariables) -> (Vec3, Vec3) {
    let sun_pull = variables.disc_pull_scale * variables.sun_pull.max(variables.pull_amount);
    let moon_pull = variables.disc_pull_scale * variables.moon_pull.max(variables.pull_amount);
    (
        (sun + (variables.sun_target - sun) * sun_pull).normalize_or_zero(),
        (moon + (variables.moon_target - moon) * moon_pull).normalize_or_zero(),
    )
}

/// The frame's sun and moon light: weights, zenith angles, direction, intensity weight, colour and intensity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightState {
    /// The three bodies' weights.
    pub weights: Weights,
    /// The sun's zenith angle, degrees.
    pub sun_zenith: f32,
    /// The moon's zenith angle, degrees.
    pub moon_zenith: f32,
    /// Unit vector toward the light; the light itself shines along -direction.
    pub direction: Vec3,
    /// The intensity weight W, unitless.
    pub intensity_weight: f32,
    /// The light colour, as the frame holds it.
    pub colour: Vec3,
    /// The light intensity in the frame's own units, 0 when off.
    pub intensity: f32,
}

/// The directional light from the raw ephemeris sun and raw moon (not the pulled disc directions of disc_directions), the third body's vector, the moon cycle p and the frame's variables.
pub fn directional_light(
    sun: Vec3,
    moon: Vec3,
    third: Vec3,
    moon_cycle: f32,
    variables: &LightVariables,
    tunables: &LightTunables,
) -> LightState {
    let sun_zenith = zenith_degrees(sun.z);
    let moon_zenith = zenith_degrees(moon.z);
    let weights = body_weights(sun_zenith, moon_zenith, variables, tunables);
    let direction = light_direction(sun, moon, third, &weights, variables);
    let weight = intensity_weight(&weights, moon_cycle, variables, tunables);
    let third_set = weights.third > 0.0;
    let colour = if third_set {
        variables.third_colour
    } else {
        variables.day_colour
    };
    let base = if third_set {
        variables.third_intensity
    } else {
        variables.day_intensity
    };
    let mut intensity = base * weight;
    if intensity < MIN_LIGHT_INTENSITY || colour.max_element() < MIN_LIGHT_INTENSITY {
        intensity = 0.0;
    }
    LightState {
        weights,
        sun_zenith,
        moon_zenith,
        direction,
        intensity_weight: weight,
        colour,
        intensity,
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Clamps to [0, 1]; NaN, from an empty box, counts as 0.
fn unit_interval(value: f64) -> f64 {
    if value > 0.0 {
        value.min(1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "gta6_day_sun_tests.rs"]
mod tests;
