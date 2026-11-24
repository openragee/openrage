//! The light of the day for a GTA VI model: where the sun and the moon stand at an hour, which
//! of them lights, the cycle of the install a weather is drawn with and its two samples that
//! stand for the sun's place, and the light, the sky and the tone those state.

use std::f32::consts::{PI, TAU};

use glam::Vec3;
use or_io::gta5::data::timecycle::{Keyframe, TimeTable};
use or_io::gta5::data::visual_settings::VisualSettings;
use or_renderer::game_shaders::meshlets::{Clouds, Curve, Filmic, Scene, Sky, Tone};
use or_title::TimecycleData;

use super::gta6_air::Air;
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

/// What the ground under a model sends back of the light that falls on it, of every colour
/// alike: the ground that lights the model from below, and the one laid under it.
// A stand-in: the game's ground is its world's own surfaces, and nothing of an install states
// one for a model alone.
pub(super) const GROUND: f32 = 0.3;
/// The tuning value an install states the scale of its exposure curve by, and the stems of that
/// curve's and of the second curve's values.
const EXPOSURE_CURVE: [&str; 3] = [
    "Exposure.curve.scale",
    "Exposure.curve",
    "Exposure.curve.blend",
];

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

/// The name a weather of the Weather bar is asked of a GTA VI install's weather set by.
fn asked_as(weather: WeatherState) -> &'static str {
    match weather {
        WeatherState::Clear => "clear",
        WeatherState::Cloudy => "CLOUDS",
        WeatherState::Fog => "FOG",
        WeatherState::Rain => "Rain",
        WeatherState::Storm => "THUNDERSTORM",
    }
}

/// The cycle that stands for a weather where an install states no weather set.
fn plain_cycle(weather: WeatherState) -> &'static str {
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
/// never both; or as the cycle's own cross-fade says. `settings` state the angle each gives way
/// at and the degrees it fades over, where the cycle states no angle.
fn lighting_body(
    sun: Vec3,
    moon: Vec3,
    frame: &Keyframe,
    settings: &VisualSettings,
) -> Option<(Body, f32)> {
    let zenith = |towards: Vec3| towards.z.clamp(-1.0, 1.0).acos().to_degrees();
    let past = |towards: Vec3, row: &str, body: &str| {
        let stated = frame.get_or(row, -1.0);
        let threshold = match stated < 0.0 {
            true => settings.get_f32(&format!("misc.{body}Fade.Threshold"), FADE_ZENITH),
            false => stated,
        };
        let over = settings.get_f32(&format!("misc.{body}Fade.ThresholdLength"), 1.0);
        ((zenith(towards) - threshold) / over.max(1e-3)).clamp(-1.0, 1.0)
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
        past(sun, "sun_fade_threshold", "Sun"),
        past(moon, "moon_fade_threshold", "Moon"),
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

/// The cycle a weather is drawn with, and each variable its type's modifiers state over it: as
/// the install's weather set says, else the cycle of the weather's own name with nothing over.
fn cycle_of(data: &TimecycleData, weather: WeatherState) -> (&str, Vec<(&str, f32)>) {
    (data.gta6_weather.resolve(asked_as(weather))).unwrap_or((plain_cycle(weather), Vec::new()))
}

/// Whether `data` holds what lights a GTA VI model in `weather`: a table of the day's samples,
/// that weather's cycle, and the curve a metered frame is exposed by.
pub(crate) fn lights(data: &TimecycleData, weather: WeatherState) -> bool {
    data.timecycle.table.samples.len() >= SAMPLE_ANGLES.len()
        && data.timecycle.cycle(cycle_of(data, weather).0).is_some()
        && data.visual_settings.get(EXPOSURE_CURVE[0]).is_some()
}

/// The seven numbers of a filmic curve and the exposure it stands at: a cycle's rows whose
/// names end `ending`, or the install's tuning values of `kind` with the rows where it has none.
fn filmic(
    frame: &Keyframe,
    settings: Option<(&VisualSettings, &str)>,
    ending: &str,
) -> (Filmic, f32) {
    let row = |name: &str| frame.get_or(&format!("postfx_tonemap_filmic_{name}{ending}"), 0.0);
    let value = |name: &str, tuned: &str| match settings {
        Some((settings, kind)) => {
            settings.get_f32(&format!("Tonemapping.{kind}.filmic.{tuned}"), row(name))
        }
        None => row(name),
    };
    let numbers = [
        ("a", "A"),
        ("b", "B"),
        ("c", "C"),
        ("d", "D"),
        ("e", "E"),
        ("f", "F"),
        ("w", "W"),
    ];
    let which = if ending.is_empty() { "_dark" } else { ending };
    let at = frame.get_or(&format!("postfx_tonemap_filmic_exposure{which}"), 0.0);
    let at = match settings {
        Some((settings, kind)) => {
            settings.get_f32(&format!("Tonemapping.{kind}.filmic.exposure"), at)
        }
        None => at,
    };
    (Filmic(numbers.map(|(name, tuned)| value(name, tuned))), at)
}

/// How a frame lit by `frame` is metered and toned, with the install's tuning values; `disc`
/// is the tangent of half the lighting body's disc.
fn tone_of(frame: &Keyframe, settings: &VisualSettings, disc: f32) -> Tone {
    let curve = |stem: &str| Curve {
        scale: settings.get_f32(&format!("{stem}.scale"), 0.0),
        vertical: settings.get_f32(&format!("{stem}.verticalOffset"), 0.0),
        horizontal: settings.get_f32(&format!("{stem}.horizontalOffset"), 0.0),
        bending: settings.get_f32(&format!("{stem}.bending"), 0.0),
    };
    let lowlights = frame.get_or("postfx_exposure_lowlights_percent", 0.0);
    let by_key = frame.get_or("postfx_exposure_use_key_value_mode", 0.0) > 0.0;
    // A cycle overrules the install's two filmic curves with rows of its own: the dark rows
    // alone for both, or both; the dark curve is the rows' wherever either is overruled.
    let overruled =
        |which: &str| frame.get_or(&format!("postfx_tonemap_filmic_override_{which}"), 0.0) > 0.0;
    let ((bright, bright_at), (dark, dark_at)) = match (overruled("dark"), overruled("bright")) {
        (false, false) => (
            filmic(frame, Some((settings, "bright")), "_bright"),
            filmic(frame, Some((settings, "dark")), ""),
        ),
        (true, false) => (filmic(frame, None, ""), filmic(frame, None, "")),
        (true, true) => (filmic(frame, None, "_bright"), filmic(frame, None, "")),
        (false, true) => (
            filmic(frame, Some((settings, "bright")), "_bright"),
            filmic(frame, None, ""),
        ),
    };
    Tone {
        roughness_mod: frame.get_or("direct_light_roughness_mod", 0.0),
        disc,
        lowlights,
        highlights: (frame.get_or("postfx_exposure_highlights_percent", 0.0)).min(0.99 - lowlights),
        curve: [curve(EXPOSURE_CURVE[1]), curve(EXPOSURE_CURVE[2])],
        blend: frame.get_or("postfx_exposure_blend_curve_weight", 0.0),
        key: by_key.then(|| frame.get_or("postfx_exposure_key_value", 0.18)),
        least: frame.get_or("postfx_exposure_min", -11.0),
        most: frame.get_or("postfx_exposure_max", 32.0),
        added: frame.get_or("postfx_exposure", 0.0) + frame.get_or("postfx_exposure_tweak", 0.0),
        bright,
        dark,
        bright_at,
        dark_at,
    }
}

/// The clouds over a model in a weather whose cycle says they cover `cover` of the sky, and
/// what `frame` states of the directional light under them.
// The open share is a stand-in. The game marches the light's way through its field of clouds
// for every place, and a model alone has no field: the clouds are taken to hide the share of
// the ground the cycle says they cover, as deeply as it says a cloud's shadow shows, and the
// model stands in the mean of that.
fn clouds_of(cover: f32, frame: &Keyframe) -> Clouds {
    let shows = frame.get_or("local_atmosphere_cloud_shadow_alpha", 1.0);
    Clouds {
        open: 1.0 - cover.clamp(0.0, 1.0) * shows.clamp(0.0, 1.0),
        roughness_mod: frame.get_or("direct_light_roughness_mod_cloud_shadow", 0.0),
        haze_blend: frame.get_or("direct_light_haze_blend_scale", 0.0),
        haze: frame.get_or("direct_light_haze_intensity", 1.0),
    }
}

/// The sky a GTA VI model is lit by at `hours` in `weather` by the install's tables of the day,
/// or none where `data` does not hold them. The sun and the moon stand where the game puts them
/// and one of them lights, by its lux and the cycle's tint and multipliers, through the air the
/// cycle states; that air's sky is what surrounds the model; and the frame is metered and toned
/// as the game's programs do.
// Estimates left: the ground, a matte one of the editor's own; the clouds' shadow, a mean; the
// moon's round, which is the game's at rest; and what the game adds that a lone model has none
// of: fog, probes and reflections.
pub(super) fn sky(data: &TimecycleData, hours: f32, weather: WeatherState) -> Option<Sky> {
    if !lights(data, weather) {
        return None;
    }
    let (name, stated) = cycle_of(data, weather);
    let sun = sun_towards(hours);
    let moon = moon_towards(hours);
    let between = samples_at(&data.timecycle.table, sun_angle(sun));
    let mut frame = (data.timecycle).keyframe_between(between, name, name, 0.0, 0.0)?;
    for (variable, value) in stated {
        frame.set(variable, value);
    }
    let air = Air::of(&frame);
    let cover = (data.timecycle.cycle(name)).and_then(|cycle| cycle.overall.get("cloud_cover"));
    let clouds = clouds_of(cover.copied().unwrap_or(0.0), &frame);

    let lux = |body: &str| {
        frame.get_or(&format!("sky_scattering_{body}_illuminance"), 0.0)
            * frame.get_or(&format!("sky_scattering_{body}_intensity_multiplier"), 1.0)
    };
    let light = lighting_body(sun, moon, &frame, &data.visual_settings).map(|(body, fade)| {
        let (towards, word) = match body {
            Body::Sun => (sun, "sun"),
            Body::Moon => (moon, "moon"),
        };
        let strength = frame.get_or("light_dir_mult", 1.0)
            * fade
            * frame.get_or(&format!("light_dir_{word}_mult"), 1.0);
        let through = air.light_through(towards)
            * frame.get_or("sky_transmittance_correction_factor", 1.0 / PI);
        (
            towards,
            rgb(&frame, "light_dir_col") * lux(word) * strength * through,
        )
    });
    let (towards, colour) = light.unwrap_or((sun, Vec3::ZERO));
    let towards = Vec3::new(towards.x, towards.y, towards.z.max(LIGHT_FLOOR)).normalize();

    // What a matte surface that faces up sends back of the whole sky, under both bodies.
    let given = air.sky_on_the_ground(sun) * lux("sun") + air.sky_on_the_ground(moon) * lux("moon");
    let around = given / PI
        * rgb(&frame, "sky_scattering_sky_col")
        * rgb(&frame, "ambient_sky_color_mult")
        * frame.get_or("ambient_sky_intensity", 1.0);
    let ground = (around + colour * clouds.open * towards.z / PI)
        * GROUND
        * frame.get_or("ambient_bounce_intensity", 1.0);
    Some(Sky {
        towards,
        sun: colour,
        sky: around,
        ground,
        exposure: 1.0,
        tone: Some(tone_of(&frame, &data.visual_settings, air.disc_tangent())),
        scene: Scene {
            clouds,
            ..Scene::default()
        },
    })
}

#[cfg(test)]
#[path = "gta6_day_tests.rs"]
mod tests;
