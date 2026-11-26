//! The global wind: heading and base speed from the weather object, the eased timecycle speeds, up to two moving gusts, the two noise scroll offsets, the maximum speed and a viewer's wind field, with axes x east, y north, z up.
//! Not known and so not applied: the random walk's rate and retarget thresholds (the step and the target are arguments), the wind volumes' fill past the speed it injects, and the shaders' own use of the block; the calm byte (read: it zeroes dir and both timecycle speeds each frame) is not applied because its writer was not found.

use glam::Vec3;
use or_io::gta5::data::timecycle::Keyframe;
use or_io::gta6::weather::WeatherData;
use std::f32::consts::PI;

use crate::gta6_day_frame::{lerp, name_hash};
use crate::gta6_day_vars::VarTable;

/// Frame index of v1103, the weight that pulls the heading toward v1104.
const HEADING_WEIGHT: usize = 1103;

/// Frame index of v1104, the timecycle heading in degrees.
const HEADING_DEGREES: usize = 1104;

/// Frame index of v1105, which the wind volumes' fill multiplies the wind it injects by.
const SPEED_MULT: usize = 1105;

/// Frame index of v1122, the height over which the injected speed goes from the first timecycle speed to the second (m).
const FALLOFF_HEIGHT: usize = 1122;

/// Frame index of v1107, the first of the gust strength pair (km/h).
const GUST_STRENGTH: usize = 1107;

/// Frame index of v1109, the first of the gust duration pair (s).
const GUST_DURATION: usize = 1109;

/// Frame index of v1111, the first of the gust interval pair (s).
const GUST_INTERVAL: usize = 1111;

/// Frame index of v1118, the first of the first timecycle speed pair (km/h).
const SPEED_A: usize = 1118;

/// Frame index of v1120, the first of the second timecycle speed pair (km/h).
const SPEED_B: usize = 1120;

/// Metres per second in one km/h, the build's typed 1/3.6, which the timecycle speeds are multiplied by.
pub const KMH_TO_MS: f32 = f32::from_bits(0x3e8e_38e4);

/// Km/h in one m/s, the divisor of gust strengths and volume speeds; the quotient can differ from a product with KMH_TO_MS by 1 ulp.
const KMH_PER_MS: f32 = 3.6;

/// The volumes' default maximum speed, km/h.
pub const DEFAULT_VOLUME_MAX_SPEED_KMH: f32 = 74.0;

/// Mask of the 23 random bits a unit random number takes.
const RANDOM_MASK: u32 = 0x007f_ffff;

/// Scale of the 23 random bits onto [0, 1), 2^-23.
const RANDOM_SCALE: f32 = 1.0 / 8_388_608.0;

/// Fraction of a duration in one per cent of a gust ramp.
const PER_CENT: f32 = 0.01;

/// Power of the linear gust falloff over its radius, a viewer stand-in: the shader's gust shape was not read.
const GUST_FALLOFF_EXPONENT: f32 = 1.0;

/// The wind numbers of a weather object (the pointer at weather record +0x28): the base speed pair (+0x08, +0x0c) and the heading (+0x10).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherWind {
    /// Base speed at walk 0, m/s (inferred from its sum with gust speeds).
    pub speed_low: f32,
    /// Base speed at walk 1, m/s (inferred).
    pub speed_high: f32,
    /// Heading the wind comes from, degrees clockwise from north (the "from" is inferred).
    pub heading_degrees: f32,
}

/// The timecycle variables the wind reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindVariables {
    /// v1103, the pull toward the timecycle heading; at or below 0 none, above 1 taken as 1.
    pub heading_weight: f32,
    /// v1104, the timecycle heading, degrees.
    pub heading_degrees: f32,
    /// v1107, v1108, the gust strength range, km/h.
    pub gust_strength_kmh: [f32; 2],
    /// v1109, v1110, the gust duration range, s.
    pub gust_duration: [f32; 2],
    /// v1111, v1112, the range of the time to the next gust, s.
    pub gust_interval: [f32; 2],
    /// v1118, v1119, the first timecycle speed range, km/h.
    pub speed_a_kmh: [f32; 2],
    /// v1120, v1121, the second timecycle speed range, km/h.
    pub speed_b_kmh: [f32; 2],
    /// v1105, the multiplier of the wind the volumes are filled with.
    pub speed_mult: f32,
    /// v1122, the height the first timecycle speed has gone over to the second by, m; at or below 0 the first speed at every height.
    pub falloff_height: f32,
}

impl WindVariables {
    /// Reads the wind's variables from a timecycle frame by their indices; None when the frame is too short.
    pub fn from_frame(frame: &[f32]) -> Option<WindVariables> {
        Some(WindVariables {
            heading_weight: *frame.get(HEADING_WEIGHT)?,
            heading_degrees: *frame.get(HEADING_DEGREES)?,
            gust_strength_kmh: pair(frame, GUST_STRENGTH)?,
            gust_duration: pair(frame, GUST_DURATION)?,
            gust_interval: pair(frame, GUST_INTERVAL)?,
            speed_a_kmh: pair(frame, SPEED_A)?,
            speed_b_kmh: pair(frame, SPEED_B)?,
            speed_mult: *frame.get(SPEED_MULT)?,
            falloff_height: *frame.get(FALLOFF_HEIGHT)?,
        })
    }

    /// Reads them from a moment's own variables by name, each the table's default where the moment states no name of its hash.
    pub fn of_moment(own: &Keyframe, table: &VarTable) -> Option<WindVariables> {
        let mut frame = table.defaults();
        for (name, value) in own.iter() {
            if let Some(slot) = table
                .index_of(name_hash(name))
                .and_then(|i| frame.get_mut(i))
            {
                *slot = value;
            }
        }
        WindVariables::from_frame(&frame[..])
    }
}

impl WeatherWind {
    /// The wind a weather's data states: its least and most speed and its angle.
    pub fn of(data: &WeatherData) -> WeatherWind {
        WeatherWind {
            speed_low: data.wind_min,
            speed_high: data.wind_max,
            heading_degrees: data.wind_angle,
        }
    }
}

fn pair(frame: &[f32], index: usize) -> Option<[f32; 2]> {
    Some([*frame.get(index)?, *frame.get(index + 1)?])
}

/// The wind manager's configuration; a data file may replace any of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindConfig {
    /// Most gusts alive at once.
    pub max_gusts: usize,
    /// Largest turn of a gust off the wind, degrees.
    pub gust_spread_degrees: f32,
    /// Share of a gust's duration it ramps in over, per cent.
    pub gust_ramp_in_percent: f32,
    /// Share of a gust's duration subtracted from it to give the ramp-out span, per cent.
    pub gust_ramp_out_percent: f32,
    /// Half-width R of the square gusts spawn in and wrap within, m.
    pub gust_range: f32,
    /// Most change of a timecycle speed per second, m/s per s.
    pub speed_ease_rate: f32,
    /// Rates the two noise scroll offsets move along the wind at, per second.
    pub scroll_rates: [f32; 2],
    /// Periods the two noise scroll offsets wrap at.
    pub scroll_periods: [f32; 2],
}

impl WindConfig {
    /// The image's configuration defaults.
    pub fn image_defaults() -> WindConfig {
        WindConfig {
            max_gusts: 2,
            gust_spread_degrees: 0.0,
            gust_ramp_in_percent: 20.0,
            gust_ramp_out_percent: 20.0,
            gust_range: 800.0,
            speed_ease_rate: 2.0,
            scroll_rates: [10.0, 200.0],
            scroll_periods: [4000.0, 4000.0],
        }
    }
}

/// Wraps a value into [-half, half] with period 2 half, by the sign of the value; a half that is not positive leaves it.
// Trap: +half goes to -half and -half to +half, so neither (-half, half] nor [-half, half) holds at both ends.
fn wrap_half_period(value: f32, half: f32) -> f32 {
    if half > 0.0 {
        let period = 2.0 * half;
        if value >= 0.0 {
            (value + half) % period - half
        } else {
            (value - half) % period + half
        }
    } else {
        value
    }
}

/// The heading in radians: the weather's, moved toward v1104 the short way round by min(1, v1103) when v1103 > 0.
pub fn heading(weather_degrees: f32, variables: &WindVariables) -> f32 {
    let weather = weather_degrees.to_radians();
    if variables.heading_weight > 0.0 {
        let toward = variables.heading_degrees.to_radians();
        let arc = wrap_half_period(toward - weather, PI);
        wrap_half_period(weather + variables.heading_weight.min(1.0) * arc, PI)
    } else {
        weather
    }
}

/// Unit wind direction (-sin a, -cos a, 0) of a heading in radians: the way the wind blows.
pub fn direction(heading: f32) -> Vec3 {
    let (sin_heading, cos_heading) = heading.sin_cos();
    Vec3::new(-sin_heading, -cos_heading, 0.0)
}

/// Base speed lerp(low, high, walk), with walk the random walk g in [0, 1).
pub fn base_speed(weather: &WeatherWind, walk: f32) -> f32 {
    lerp(weather.speed_low, weather.speed_high, walk)
}

/// Moves a value toward a target by at most max_step; a negative or NaN step holds it.
pub fn approach(current: f32, target: f32, max_step: f32) -> f32 {
    let step = max_step.max(0.0);
    current + (target - current).clamp(-step, step)
}

/// A unit random number in [0, 1) from a state weather::mwc_next returned: its low 23 bits times 2^-23.
pub fn unit_random(x: u64) -> f32 {
    (x & RANDOM_MASK as u64) as f32 * RANDOM_SCALE
}

/// The two timecycle speeds (m/s), each eased toward lerp(pair, walk) in km/h converted, at the config's rate.
pub fn ease_timecycle_speeds(
    current: [f32; 2],
    variables: &WindVariables,
    walk: f32,
    config: &WindConfig,
    dt: f32,
) -> [f32; 2] {
    let max_step = config.speed_ease_rate * dt;
    let target_a = lerp(variables.speed_a_kmh[0], variables.speed_a_kmh[1], walk) * KMH_TO_MS;
    let target_b = lerp(variables.speed_b_kmh[0], variables.speed_b_kmh[1], walk) * KMH_TO_MS;
    [
        approach(current[0], target_a, max_step),
        approach(current[1], target_b, max_step),
    ]
}

/// The largest speed of the two wind volumes, m/s, from their configured maxima in km/h.
pub fn max_speed(volume_max_kmh: [f32; 2]) -> f32 {
    volume_max_kmh[0].max(volume_max_kmh[1]) / KMH_PER_MS
}

/// A noise scroll offset dt seconds on: moved along the wind at rate, each axis then wrapped at period; a period that is not positive leaves it unwrapped.
pub fn scroll(offset: Vec3, wind_direction: Vec3, dt: f32, rate: f32, period: f32) -> Vec3 {
    let moved = offset + wind_direction * (dt * rate);
    // The wrap's form was not read: fmodf by the period, keeping the value's sign, is assumed.
    if period > 0.0 {
        Vec3::new(moved.x % period, moved.y % period, moved.z % period)
    } else {
        moved
    }
}

/// The unit random numbers a gust spawn draws, each in [0, 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GustRandoms {
    /// Picks the time to the next gust in v1111..v1112.
    pub interval: f32,
    /// Picks the strength in v1107..v1108.
    pub strength: f32,
    /// Picks the duration in v1109..v1110.
    pub duration: f32,
    /// Picks the turn spread x (1 - 2 r) degrees.
    pub angle: f32,
    /// Picks the x of the spawn point, r x 2R - R.
    pub x: f32,
    /// Picks the y of the spawn point, r x 2R - R.
    pub y: f32,
}

/// One gust: a patch of extra wind moving with the wind inside a square of half-width radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gust {
    /// Centre, m, z 0, in a frame (world or camera-relative) that was not read.
    pub position: Vec3,
    /// Ramp weight in [0, 1].
    pub weight: f32,
    /// Unit direction it moves and blows along.
    pub direction: Vec3,
    /// Speed it moves at, its strength plus the base speed at spawn, m/s.
    pub speed: f32,
    /// Extra wind speed it adds, m/s.
    pub strength: f32,
    /// Half-width R of the square it wraps within, m.
    pub radius: f32,
    /// Length of its life, s.
    pub duration: f32,
    /// Span of the ramp in, s.
    pub ramp_in: f32,
    /// Subtracted from the duration to give the ramp-out span, s.
    pub ramp_out: f32,
    /// Seconds of life left; the gust goes once this is at or below 0.
    pub life: f32,
}

/// A ramp factor elapsed / span in [0, 1]; a span that is not positive gives 1.
// Whether a ramp-out of 0 (rather than a span D - ramp_out of 0) gives 1 was not settled; the span is tested.
fn ramp(elapsed: f32, span: f32) -> f32 {
    if span > 0.0 {
        (elapsed / span).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

impl Gust {
    /// The gust dt seconds on: moved along its direction, wrapped to its square, life spent and weight ramped.
    pub fn aged(&self, dt: f32) -> Gust {
        let life = self.life - dt;
        let moved = self.position + self.direction * (dt * self.speed);
        let position = Vec3::new(
            wrap_half_period(moved.x, self.radius),
            wrap_half_period(moved.y, self.radius),
            moved.z,
        );
        let ramp_up = ramp(self.duration - life, self.ramp_in);
        let ramp_down = ramp(life, self.duration - self.ramp_out);
        Gust {
            position,
            weight: ramp_up * ramp_down,
            life,
            ..*self
        }
    }

    /// Share of the gust's strength felt at a point, a viewer stand-in: linear over the radius in xy, the offset wrapped to the gust's square.
    pub fn falloff(&self, point: Vec3) -> f32 {
        if self.radius > 0.0 {
            let dx = wrap_half_period(point.x - self.position.x, self.radius);
            let dy = wrap_half_period(point.y - self.position.y, self.radius);
            let distance = (dx * dx + dy * dy).sqrt();
            (1.0 - distance / self.radius)
                .max(0.0)
                .powf(GUST_FALLOFF_EXPONENT)
        } else {
            0.0
        }
    }
}

/// A new gust along the wind, or None when its strength or duration is not positive; its weight starts at 0.
pub fn spawn_gust(
    wind_direction: Vec3,
    base_speed: f32,
    variables: &WindVariables,
    randoms: &GustRandoms,
    config: &WindConfig,
) -> Option<Gust> {
    let strength = lerp(
        variables.gust_strength_kmh[0],
        variables.gust_strength_kmh[1],
        randoms.strength,
    ) / KMH_PER_MS;
    let duration = lerp(
        variables.gust_duration[0],
        variables.gust_duration[1],
        randoms.duration,
    );
    // Written as a positive test so a NaN strength or duration spawns nothing.
    let alive = strength > 0.0 && duration > 0.0;
    if !alive {
        return None;
    }
    let turn = (config.gust_spread_degrees * (1.0 - 2.0 * randoms.angle)).to_radians();
    let (sin_turn, cos_turn) = turn.sin_cos();
    let direction = Vec3::new(
        cos_turn * wind_direction.x - sin_turn * wind_direction.y,
        sin_turn * wind_direction.x + cos_turn * wind_direction.y,
        wind_direction.z,
    );
    let range = config.gust_range;
    Some(Gust {
        position: Vec3::new(
            randoms.x * 2.0 * range - range,
            randoms.y * 2.0 * range - range,
            0.0,
        ),
        weight: 0.0,
        direction,
        speed: strength + base_speed,
        strength,
        radius: range,
        duration,
        ramp_in: duration * PER_CENT * config.gust_ramp_in_percent,
        ramp_out: duration * PER_CENT * config.gust_ramp_out_percent,
        life: duration,
    })
}

/// The global wind and its gusts.
#[derive(Clone, Debug, PartialEq)]
pub struct WindState {
    /// Unit direction the wind blows along.
    pub direction: Vec3,
    /// Base speed, m/s (inferred).
    pub base_speed: f32,
    /// The two eased timecycle speeds, m/s.
    pub timecycle_speeds: [f32; 2],
    /// The live gusts, at most the config's maximum.
    pub gusts: Vec<Gust>,
    /// Clock time after which the next gust may spawn, s.
    pub next_gust_time: f32,
    /// The two noise scroll offsets, A and B.
    pub scroll_offsets: [Vec3; 2],
}

impl WindState {
    /// No wind, no gusts and a gust due at once (the game's starting values were not read).
    pub fn calm() -> WindState {
        WindState {
            direction: Vec3::ZERO,
            base_speed: 0.0,
            timecycle_speeds: [0.0, 0.0],
            gusts: Vec::new(),
            // A gust is due once the clock is past this, so 0 would hold the first back from a clock of 0.
            next_gust_time: f32::NEG_INFINITY,
            scroll_offsets: [Vec3::ZERO; 2],
        }
    }

    /// Sets the direction and base speed from a weather object's wind, as the game does on a weather change.
    pub fn apply_weather(&mut self, weather: &WeatherWind, variables: &WindVariables, walk: f32) {
        self.direction = direction(heading(weather.heading_degrees, variables));
        self.base_speed = base_speed(weather, walk);
    }

    /// One frame at clock seconds after dt seconds: the timecycle speeds eased, a gust spawned when due, every gust aged and the dead removed, then both scroll offsets moved.
    pub fn step(
        &mut self,
        clock: f32,
        dt: f32,
        variables: &WindVariables,
        walk: f32,
        randoms: &GustRandoms,
        config: &WindConfig,
    ) {
        self.timecycle_speeds =
            ease_timecycle_speeds(self.timecycle_speeds, variables, walk, config, dt);
        if clock > self.next_gust_time {
            self.next_gust_time = clock
                + lerp(
                    variables.gust_interval[0],
                    variables.gust_interval[1],
                    randoms.interval,
                );
            // Whether a full list still resets the timer was not read; here it does.
            if self.gusts.len() < config.max_gusts {
                if let Some(gust) =
                    spawn_gust(self.direction, self.base_speed, variables, randoms, config)
                {
                    self.gusts.push(gust);
                }
            }
        }
        let mut index = 0;
        while index < self.gusts.len() {
            let aged = self.gusts[index].aged(dt);
            if aged.life > 0.0 {
                self.gusts[index] = aged;
                index += 1;
            } else {
                self.gusts.swap_remove(index);
            }
        }
        for (offset, (rate, period)) in self
            .scroll_offsets
            .iter_mut()
            .zip(config.scroll_rates.iter().zip(config.scroll_periods.iter()))
        {
            *offset = scroll(*offset, self.direction, dt, *rate, *period);
        }
    }

    /// The speed the wind volumes' fill injects along the wind at height `z` (m/s): the two eased timecycle speeds blended by how far `z` is up the falloff height, times v1105.
    pub fn injected_speed(&self, variables: &WindVariables, z: f32) -> f32 {
        let up = if variables.falloff_height > 0.0 {
            (z / variables.falloff_height).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let [bottom, top] = self.timecycle_speeds;
        lerp(bottom, top, up) * variables.speed_mult
    }

    /// Wind velocity (m/s) at a point, a viewer stand-in: dir x s read from this build, plus each live gust's direction x strength x weight x falloff, a sum the build was not seen to make.
    pub fn wind_at(&self, point: Vec3, since_step: f32) -> Vec3 {
        let mut wind = self.direction * self.base_speed;
        for gust in &self.gusts {
            let aged = gust.aged(since_step);
            if aged.life > 0.0 {
                wind += aged.direction * (aged.strength * aged.weight * aged.falloff(point));
            }
        }
        wind
    }
}

#[cfg(test)]
#[path = "gta6_day_wind_tests.rs"]
mod tests;
