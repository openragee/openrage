//! The light of the day for a GTA VI model: where the sun stands at an hour, the two samples of
//! the install's weather cycle that stand for it, and the sky they tint.

use std::f32::consts::{PI, TAU};

use glam::Vec3;
use or_io::gta5::data::timecycle::Timecycle;
use or_renderer::game_shaders::meshlets::Sky;
use or_title::TimecycleData;

use crate::state::drivers::WeatherState;

/// When the sun is highest, as a part of the day: one o'clock by the summer clock and the
/// minutes the first day of spring adds.
const SOLAR_NOON: f32 = (13.0 * 60.0 + 27.0) / 1440.0;
/// The part of a day from the sun's rise to its noon on that day, when it stands on the equator.
const QUARTER_DAY: f32 = 0.25;
/// How the clock lingers near sunrise and sunset: the power the way through half a day or half
/// a night is raised to.
const HORIZON_LINGER: f32 = 1.25;
/// The latitude at the middle of the map, in radians.
const LATITUDE: f32 = 26.64 * PI / 180.0;

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

/// The clock as the sun follows it: `day` is the part of the day gone by, and each half of the
/// day and of the night is slowed near the horizon.
fn sun_clock(day: f32) -> f32 {
    let (rise, set) = (SOLAR_NOON - QUARTER_DAY, SOLAR_NOON + QUARTER_DAY);
    let (from, to) = match day {
        day if day < rise => (set - 1.0, rise),
        day if day > set => (set, rise + 1.0),
        _ => (rise, set),
    };
    let half = (to - from) / 2.0;
    match day - from < half {
        true => from + half * ((day - from) / half).powf(HORIZON_LINGER),
        false => to - half * ((to - day) / half).powf(HORIZON_LINGER),
    }
}

/// The way towards the sun at `hours` of the first day of spring: east is x, north is y.
pub(super) fn sun_towards(hours: f32) -> Vec3 {
    let turn = (sun_clock(hours.rem_euclid(24.0) / 24.0) - SOLAR_NOON) * TAU;
    Vec3::new(
        -turn.sin(),
        -turn.cos() * LATITUDE.sin(),
        turn.cos() * LATITUDE.cos(),
    )
}

/// The angle of the sun on its round: nought as it rises, a quarter turn at its top.
fn sun_angle(towards: Vec3) -> f32 {
    towards.z.atan2(towards.x)
}

/// The two samples of a cycle the sun at `angle` lies between, and how far from the first to the
/// second it is.
#[allow(clippy::approx_constant)]
fn samples_at(angle: f32) -> (usize, usize, f32) {
    // The round is closed a little short of a whole turn.
    let angle = if angle < -3.14 { angle + 6.28 } else { angle };
    let first = (SAMPLE_ANGLES.iter())
        .rposition(|(at, _)| *at <= angle)
        .unwrap_or(0);
    let second = (first + 1) % SAMPLE_ANGLES.len();
    let (from, to) = (SAMPLE_ANGLES[first].0, SAMPLE_ANGLES[second].0);
    let to = if to < from { to + TAU } else { to };
    let along = ((angle - from) / (to - from)).clamp(0.0, 1.0);
    (SAMPLE_ANGLES[first].1, SAMPLE_ANGLES[second].1, along)
}

/// The cycle of `timecycle` that stands for the weather whose cycle GTA V names `wanted`: the
/// one of that name, else the first whose name holds it or is held by it.
fn cycle_for<'a>(timecycle: &'a Timecycle, wanted: &str) -> Option<&'a str> {
    let wanted = wanted.to_ascii_lowercase();
    let named = |exact: bool| {
        timecycle.cycles.iter().find(|cycle| {
            let name = cycle.name.to_ascii_lowercase();
            match exact {
                true => name == wanted,
                false => !name.is_empty() && (name.contains(&wanted) || wanted.contains(&name)),
            }
        })
    };
    (named(true).or_else(|| named(false))).map(|cycle| cycle.name.as_str())
}

/// The sky a GTA VI model is lit by at `hours` in `weather`. With the install's tables of the
/// day, the sun stands where the game puts it and is tinted by the weather's cycle at the two
/// samples its angle lies between; `built_in` gives the sky of an hour of a day whose sun rises
/// at six, which is all there is without them.
// Estimates, since the cycles state tints and multipliers and nothing absolute: the strength of
// the sun and of the moon, the colour of the sky and what the air takes from a low sun are the
// built-in sky's, at the hour its sun stands as high as the game's; the moon is put opposite
// the sun; a weather takes the cycle whose name is nearest GTA V's for it.
pub(super) fn sky(
    data: Option<&TimecycleData>,
    hours: f32,
    weather: WeatherState,
    built_in: impl Fn(f32) -> Sky,
) -> Sky {
    let tinted = || {
        let timecycle = &data?.timecycle;
        if timecycle.table.samples.len() < SAMPLE_ANGLES.len() {
            return None;
        }
        let cycle = cycle_for(timecycle, weather.cycle_name())?;
        let sun = sun_towards(hours);
        let angle = sun_angle(sun);
        let frame = timecycle.keyframe_between(samples_at(angle), cycle, cycle, 0.0, 0.0)?;
        let tint = Vec3::new(
            frame.get_or("light_dir_col_r", 1.0),
            frame.get_or("light_dir_col_g", 1.0),
            frame.get_or("light_dir_col_b", 1.0),
        ) * frame.get_or("light_dir_mult", 1.0);
        let mut sky = built_in((6.0 + angle / PI * 12.0).rem_euclid(24.0));
        sky.towards = if sun.z > 0.0 { sun } else { -sun };
        sky.sun *= tint;
        Some(sky)
    };
    tinted().unwrap_or_else(|| built_in(hours))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(cycle: &str) -> TimecycleData {
        let samples: String = (0..13)
            .map(|hour| format!(r#"<sample name="s" hour="{hour}"/>"#))
            .collect();
        // Sample 6 is the top of the round, 4 the rise and 0 the middle of the night.
        let cycle = format!(
            r#"<timecycle_keyframe_data><cycle name="{cycle}" regions="1"><region name="GLOBAL">
            <light_dir_col_r> 0 0 0 0 1 1 1 1 1 1 0 0 0</light_dir_col_r>
            <light_dir_col_g> 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5</light_dir_col_g>
            <light_dir_col_b> 0 0 0 0 0 0 0 0 0 0 0 0 0</light_dir_col_b>
            <light_dir_mult> 3 1 1 1 1 1 2 1 1 1 1 1 1</light_dir_mult>
            </region></cycle></timecycle_keyframe_data>"#
        );
        or_title::timecycle_of_documents([
            format!("<time_data>{samples}</time_data>").as_str(),
            cycle.as_str(),
        ])
    }

    fn plain(hours: f32) -> Sky {
        Sky {
            towards: Vec3::Z,
            sun: Vec3::ONE,
            sky: Vec3::splat(hours),
            ground: Vec3::ZERO,
            exposure: 1.0,
        }
    }

    #[test]
    fn the_sun_rises_in_the_east_tops_in_the_south_and_is_under_the_floor_at_night() {
        let noon = sun_towards(SOLAR_NOON * 24.0);
        assert!(noon.x.abs() < 1e-4 && noon.y < 0.0, "{noon:?}");
        assert!((noon.z - LATITUDE.cos()).abs() < 1e-4, "{noon:?}");
        let rise = sun_towards((SOLAR_NOON - QUARTER_DAY) * 24.0);
        assert!(rise.x > 0.99 && rise.z.abs() < 1e-3, "{rise:?}");
        let set = sun_towards((SOLAR_NOON + QUARTER_DAY) * 24.0);
        assert!(set.x < -0.99 && set.z.abs() < 1e-3, "{set:?}");
        assert!(sun_towards(1.0).z < 0.0 && sun_towards(23.0).z < 0.0);
        for hours in [0.0, 5.0, 9.0, 13.0, 18.0, 22.0] {
            assert!((sun_towards(hours).length() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn the_sun_lingers_near_the_horizon_and_hurries_through_the_top() {
        let (rise, noon) = (SOLAR_NOON - QUARTER_DAY, SOLAR_NOON);
        let early = sun_clock(rise + 0.05) - rise;
        let late = noon - sun_clock(noon - 0.05);
        assert!(early < 0.05 && late > 0.05, "{early} and {late}");
        assert!((sun_clock(noon) - noon).abs() < 1e-6);
        // The night closes on the rise of the next day.
        assert!((sun_clock(0.999) - sun_clock(0.001) - 1.0).abs() < 0.01);
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn an_angle_lies_between_two_samples_and_the_round_closes_on_itself() {
        assert_eq!(samples_at(0.0), (4, 5, 0.0));
        assert_eq!(samples_at(1.57), (6, 7, 0.0));
        let (first, second, along) = samples_at(0.15);
        assert_eq!((first, second), (4, 5));
        assert!((along - 0.5).abs() < 1e-5);
        // Past the last sample of the evening the round goes on to the first of the dusk.
        let (first, second, along) = samples_at(3.12);
        assert_eq!((first, second), (8, 9));
        assert!((0.0..=1.0).contains(&along) && along > 0.5, "{along}");
        assert_eq!(samples_at(-PI).0, 8, "a hair under the turn is its end");
        assert_eq!(samples_at(-3.14), (9, 10, 0.0));
        assert_eq!(samples_at(-1.57), (0, 1, 0.0));
    }

    #[test]
    fn the_cycle_tints_the_light_at_the_samples_the_sun_stands_between() {
        let data = data("RAIN");
        let at = |hours: f32| sky(Some(&data), hours, WeatherState::Rain, plain);
        let noon = at(SOLAR_NOON * 24.0);
        assert!(
            (noon.sun - Vec3::new(2.0, 1.0, 0.0)).length() < 0.02,
            "{:?}",
            noon.sun
        );
        assert!(noon.towards.z > 0.8 && noon.towards.y < 0.0);
        assert!(
            (noon.sky.x - 12.0).abs() < 0.01,
            "the built-in sky of its noon"
        );
        let night = at((SOLAR_NOON + 0.5) * 24.0);
        assert!(
            (night.sun - Vec3::new(0.0, 1.5, 0.0)).length() < 0.02,
            "{:?}",
            night.sun
        );
        assert!(night.towards.z > 0.8, "the moon stands opposite the sun");
    }

    #[test]
    fn a_weather_takes_the_cycle_named_for_it_or_the_nearest_name() {
        let named = |cycle: &str, wanted: &str| {
            let data = data(cycle);
            cycle_for(&data.timecycle, wanted).map(str::to_string)
        };
        assert_eq!(named("Rain", "RAIN").as_deref(), Some("Rain"));
        assert_eq!(named("Fog", "FOGGY").as_deref(), Some("Fog"));
        assert_eq!(named("WIDE_CLEAR", "CLEAR").as_deref(), Some("WIDE_CLEAR"));
        assert_eq!(named("Rain", "CLEAR"), None);
    }

    #[test]
    fn without_the_tables_or_the_weather_s_cycle_the_built_in_sky_stands() {
        let unlit = |sky: Sky| sky.sun == Vec3::ONE && sky.sky == Vec3::splat(9.0);
        assert!(unlit(sky(None, 9.0, WeatherState::Rain, plain)));
        let data = data("RAIN");
        assert!(unlit(sky(Some(&data), 9.0, WeatherState::Clear, plain)));
        let empty = TimecycleData::default();
        assert!(unlit(sky(Some(&empty), 9.0, WeatherState::Rain, plain)));
    }
}
