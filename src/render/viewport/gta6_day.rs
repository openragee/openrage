//! The light of the day for a GTA VI model: where the sun and the moon stand at an hour, which
//! of them lights, the two samples of the install's weather cycle that stand for the sun's
//! place, and the sky those state.

use std::f32::consts::{PI, TAU};

use glam::Vec3;
use or_io::gta5::data::timecycle::{Keyframe, TimeTable};
use or_renderer::game_shaders::meshlets::Sky;
use or_title::TimecycleData;

use crate::state::drivers::WeatherState;

/// When the sun is highest, as a part of the day: one o'clock by the summer clock and the
/// minutes the first day of spring adds.
const SOLAR_NOON: f32 = (13.0 * 60.0 + 27.0) / 1440.0;
/// The first and the last quarter of an hour of that day at which the sun is found at the
/// horizon, as parts of the day: the clock is bent between them.
const SUNRISE: f32 = 29.0 / 96.0;
const SUNSET: f32 = 78.0 / 96.0;
/// How the clock lingers near sunrise and sunset: the power the way through half a day or half
/// a night is raised to.
const HORIZON_LINGER: f32 = 1.25;
/// The latitude at the middle of the map, in radians.
const LATITUDE: f32 = 25.8 * PI / 180.0;
/// How far the moon's round is rolled from the east-west one, in radians, and how high above
/// that round it rides.
const MOON_ROLL: f32 = 362.0 * PI / 180.0;
const MOON_RISE: f32 = 0.8;
/// How far from straight overhead a body still lights, in degrees, where a cycle states no
/// angle of its own; it fades out over the degree before.
const FADE_ZENITH: f32 = 100.0;
/// The least height of the way to the light, which is raised to it and made a unit again.
const LIGHT_FLOOR: f32 = 0.15;
/// What a cycle states for the cross-fade of sun and moon to leave it to their heights.
const CROSSFADE_BY_HEIGHT: f32 = -10.0;

/// What a lux of sunlight and of moonlight come to in the frame.
// Estimates: the game meters each frame and exposes it on the GPU, by programs not read. These
// two keep a clear noon and a moonlit night where the built-in sky has them.
const SUN_EXPOSURE: f32 = 3.2 / 100_000.0;
const MOON_EXPOSURE: f32 = 0.5;

/// Where a cycle's samples stand on the sun's round, east to west through the top: the angle of
/// the sun in the plane of its path, nought where it rises, and the sample that holds there.
// The half turn is written short, and the round is closed by as short a turn: neither is pi.
#[allow(clippy::approx_constant)]
const SAMPLE_ANGLES: [(f32, usize); 13] = [
    (-3.14, 9),
    (-3.09, 10),
    (-2.77, 11),
    (-2.42, 12),
    (-1.57, 0),
    (-0.45, 1),
    (-0.22, 2),
    (-0.04, 3),
    (0.0, 4),
    (0.3, 5),
    (1.57, 6),
    (2.57, 7),
    (3.09, 8),
];

/// The cycle of a GTA VI install that stands for a weather of the Weather bar.
fn cycle_of(weather: WeatherState) -> &'static str {
    match weather {
        WeatherState::Clear => "Sunny",
        WeatherState::Cloudy => "CLOUDS",
        WeatherState::Fog => "Fog",
        WeatherState::Rain => "Rain",
        WeatherState::Storm => "THUNDERSTORM",
    }
}

/// The clock as the sun follows it: `day` is the part of the day gone by, and each half of the
/// day and of the night is slowed near the horizon.
fn sun_clock(day: f32) -> f32 {
    let (from, to) = match day {
        day if day < SUNRISE => (SUNSET - 1.0, SUNRISE),
        day if day > SUNSET => (SUNSET, SUNRISE + 1.0),
        _ => (SUNRISE, SUNSET),
    };
    let half = (to - from) / 2.0;
    match day - from < half {
        true => from + half * ((day - from) / half).powf(HORIZON_LINGER),
        false => to - half * ((to - day) / half).powf(HORIZON_LINGER),
    }
}

/// The way towards the sun at `hours` of the first day of spring, when it stands on the
/// equator: east is x, north is y.
pub(super) fn sun_towards(hours: f32) -> Vec3 {
    let turn = (sun_clock(hours.rem_euclid(24.0) / 24.0) - SOLAR_NOON) * TAU;
    Vec3::new(
        -turn.sin(),
        -turn.cos() * LATITUDE.sin(),
        turn.cos() * LATITUDE.cos(),
    )
}

/// The way towards the moon at `hours`: once round the compass a day, north at midnight, and
/// always well above the floor.
// The roll and the rise are what the game starts with; an install's settings may state others.
fn moon_towards(hours: f32) -> Vec3 {
    let turn = TAU * (hours.rem_euclid(24.0) / 24.0) - PI;
    let (sin, cos) = MOON_ROLL.sin_cos();
    Vec3::new(
        turn.sin(),
        -cos * turn.cos() - MOON_RISE * sin,
        -sin * turn.cos() + MOON_RISE * cos,
    )
    .normalize()
}

/// The angle of the sun on its round: nought as it rises, a quarter turn at its top.
fn sun_angle(towards: Vec3) -> f32 {
    towards.z.atan2(towards.x)
}

/// The two samples of a cycle the sun at `angle` lies between, and how far from the first to the
/// second it is, bent as the first sample of `table` says.
#[allow(clippy::approx_constant)]
fn samples_at(table: &TimeTable, angle: f32) -> (usize, usize, f32) {
    // The round is closed a little short of a whole turn.
    let angle = if angle < -3.14 { angle + 6.28 } else { angle };
    let first = (SAMPLE_ANGLES.iter())
        .rposition(|(at, _)| *at <= angle)
        .unwrap_or(0);
    let second = (first + 1) % SAMPLE_ANGLES.len();
    let (from, to) = (SAMPLE_ANGLES[first].0, SAMPLE_ANGLES[second].0);
    let to = if to < from { to + TAU } else { to };
    let along = ((angle - from) / (to - from)).clamp(0.0, 1.0);
    let (first, second) = (SAMPLE_ANGLES[first].1, SAMPLE_ANGLES[second].1);
    let easing = table.samples.get(first).map(|s| s.easing);
    (first, second, easing.unwrap_or_default().ease(along))
}

/// Which body lights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Sun,
    Moon,
}

/// The body that lights with the sun at `sun` and the moon at `moon`, and how much of its light
/// is there, from nought to one: the sun until it is some way under the horizon, then the moon,
/// never both; or as the cycle's own cross-fade says.
fn lighting_body(sun: Vec3, moon: Vec3, frame: &Keyframe) -> Option<(Body, f32)> {
    let zenith = |towards: Vec3| towards.z.clamp(-1.0, 1.0).acos().to_degrees();
    let past = |towards: Vec3, name: &str| {
        let stated = frame.get_or(name, -1.0);
        let threshold = if stated < 0.0 { FADE_ZENITH } else { stated };
        (zenith(towards) - threshold).clamp(-1.0, 1.0)
    };
    let stated = frame.get_or("sun_moon_crossfade", CROSSFADE_BY_HEIGHT);
    if stated != CROSSFADE_BY_HEIGHT {
        return match stated.clamp(-1.0, 1.0) {
            fade if fade > 0.0 => Some((Body::Sun, fade)),
            fade if fade < 0.0 => Some((Body::Moon, -fade)),
            _ => None,
        };
    }
    let (sun, moon) = (
        past(sun, "sun_fade_threshold"),
        past(moon, "moon_fade_threshold"),
    );
    match (sun < 0.0, moon < 0.0) {
        (true, _) => Some((Body::Sun, -sun)),
        (false, true) => Some((Body::Moon, sun * -moon)),
        // ponytail: the game lights by its galaxy here, which the moon's round at rest never
        // lets happen; nothing lights until an install's moon is read.
        (false, false) => None,
    }
}

/// The three values of `frame` named `stem` and `_r`, `_g`, `_b`, each one where it states none.
fn rgb(frame: &Keyframe, stem: &str) -> Vec3 {
    let part = |end: &str| frame.get_or(&format!("{stem}_{end}"), 1.0);
    Vec3::new(part("r"), part("g"), part("b"))
}

/// Whether `data` holds what lights a GTA VI model in `weather`: a table of the day's samples
/// and that weather's cycle.
pub(crate) fn lights(data: &TimecycleData, weather: WeatherState) -> bool {
    data.timecycle.table.samples.len() >= SAMPLE_ANGLES.len()
        && data.timecycle.cycle(cycle_of(weather)).is_some()
}

/// The sky a GTA VI model is lit by at `hours` in `weather` by the install's tables of the day,
/// or none where `data` does not hold them. The sun and the moon stand where the game puts them
/// and one of them lights; the weather's cycle, at the two samples the sun's angle lies between,
/// states its illuminance, its fade, its colour and what the sky's ambient is multiplied by.
/// `built_in` gives the sky of an hour of a day whose sun rises at six, under clouds that hide a
/// part of it or the Weather bar's own where none is said.
// Estimates: the two exposures above; the sky's and the ground's colour, which the game works
// out on the GPU from its atmosphere and which are the built-in sky's here, at the hour its
// sun stands as high; the moon's round, which is the game's at rest.
pub(super) fn sky(
    data: &TimecycleData,
    hours: f32,
    weather: WeatherState,
    built_in: impl Fn(f32, Option<f32>) -> Sky,
) -> Option<Sky> {
    if !lights(data, weather) {
        return None;
    }
    let timecycle = &data.timecycle;
    let name = cycle_of(weather);
    let sun = sun_towards(hours);
    let moon = moon_towards(hours);
    let angle = sun_angle(sun);
    let between = samples_at(&timecycle.table, angle);
    let frame = timecycle.keyframe_between(between, name, name, 0.0, 0.0)?;

    let lux = |body: &str| {
        frame.get_or(&format!("sky_scattering_{body}_illuminance"), 0.0)
            * frame.get_or(&format!("sky_scattering_{body}_intensity_multiplier"), 1.0)
    };
    let light = lighting_body(sun, moon, &frame).map(|(body, fade)| {
        let (towards, word, exposure) = match body {
            Body::Sun => (sun, "sun", SUN_EXPOSURE),
            Body::Moon => (moon, "moon", MOON_EXPOSURE),
        };
        let strength = frame.get_or("light_dir_mult", 1.0)
            * fade
            * frame.get_or(&format!("light_dir_{word}_mult"), 1.0);
        let colour = rgb(&frame, "light_dir_col")
            * rgb(&frame, &format!("sky_scattering_{word}_col"))
            * lux(word)
            * strength
            * exposure;
        (towards, colour)
    });
    let (towards, colour) = light.unwrap_or((sun, Vec3::ZERO));

    let cover = (timecycle.cycle(name)).and_then(|cycle| cycle.overall.get("cloud_cover"));
    let mut sky = built_in((6.0 + angle / PI * 12.0).rem_euclid(24.0), cover.copied());
    sky.towards = Vec3::new(towards.x, towards.y, towards.z.max(LIGHT_FLOOR)).normalize();
    sky.sun = colour;
    sky.sky *= rgb(&frame, "sky_scattering_sky_col")
        * rgb(&frame, "ambient_sky_color_mult")
        * frame.get_or("ambient_sky_intensity", 1.0);
    // As the built-in sky lays its ground: a part of the sky and a little of the light, bounced.
    sky.ground = (sky.sky * 0.3 + sky.sun * 0.04) * frame.get_or("ambient_bounce_intensity", 1.0);
    Some(sky)
}

#[cfg(test)]
#[path = "gta6_day_tests.rs"]
mod tests;
