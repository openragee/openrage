//! The day the GTA VI world is lit by: the install's own tables, read once on a worker and held,
//! as the day the frame takes in place of a GTA V install's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use glam::Vec3;
use or_renderer::game_shaders::lighting::{night_emissive, Day, Stated};
use or_renderer::game_shaders::sky_view::{Clouds, SkyView};
use or_renderer::game_shaders::tone::{Gta6Post, ToneSettings};
use or_renderer::viewport::Metered;
use or_title::gta6_cloud::{CloudLayer, EXTINCTION};
use or_title::gta6_day::{read_day, DayTables, Moment};
use or_title::gta6_sky::Atmosphere;

use crate::state::drivers::WeatherState;

/// The archive of an install the tables of the day are in.
const ARCHIVE: &str = "common.rpf";
/// How far into the moon's cycle the first day stands: near its full, as the GTA V day has it.
const NEAR_FULL_MOON: f32 = 13.0;
/// What a light is scaled by while no GTA VI day is read and the frame keeps its own day.
const NO_DAY_LIGHT_SCALE: f32 = 0.1;
/// How far north, in metres, a camera moves before its moment is worked out again.
const LATITUDE_STEP: f32 = 100.0;
/// The least change of exposure, in stops, the frame is given a new day for.
const STEADY: f32 = 0.01;
/// The up part of the way to a body below which it lights no sky: some twenty degrees under the horizon.
const LOWEST_BODY: f32 = -0.35;
/// How near, in stops, the exposure comes to what the curve wants before it counts as arrived: the meter's own noise from frame to frame is less.
const REACHED: f32 = 0.1;
/// How far, in metres, the camera climbs or falls before the sky's table is worked out again from its new height.
const HEIGHT_STEP: f32 = 50.0;

type Read = Arc<OnceLock<Arc<DayTables>>>;

// ponytail: read again each session, about a second; kept beside the world index if it shows.
static HELD: Mutex<Vec<(PathBuf, Read)>> = Mutex::new(Vec::new());

/// The cycle of the install that stands for the Weather bar's `state`.
pub fn weather_name(state: WeatherState) -> &'static str {
    match state {
        WeatherState::Clear => "Sunny",
        WeatherState::Cloudy => "CLOUDS",
        WeatherState::Fog => "Fog",
        WeatherState::Rain => "Rain",
        WeatherState::Storm => "THUNDERSTORM",
    }
}

/// The day's tables of the install at `root`, read once on a worker and held: `None` until they are read, and where the install states none.
pub fn tables(root: &Path) -> Option<Arc<DayTables>> {
    let archive = root.join(ARCHIVE);
    let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
    let slot = match held.iter().find(|(path, _)| *path == archive) {
        Some((_, slot)) => Arc::clone(slot),
        None => {
            let slot = Read::default();
            held.push((archive.clone(), Arc::clone(&slot)));
            let filled = Arc::clone(&slot);
            std::thread::spawn(move || {
                let read = || {
                    read_day(&archive, &|| false).unwrap_or_else(|error| {
                        tracing::warn!(archive = %archive.display(), %error, "GTA VI's tables of the day are not read");
                        DayTables::default()
                    })
                };
                let tables = super::pool::or_on_panic("the day", read, DayTables::default());
                tracing::info!(
                    cycles = tables.timecycle.cycles.len(),
                    samples = tables.table.samples.len(),
                    "GTA VI's tables of the day were read"
                );
                let _ = filled.set(Arc::new(tables));
            });
            slot
        }
    };
    let tables = slot.get()?;
    (!tables.is_empty()).then(|| Arc::clone(tables))
}

/// The moment of GTA VI's day at `hour` in `weather` for a camera `y` metres north on the map, which a frame takes its day, its lights' scale and its exposure from; `None` where the tables hold no cycle for it.
pub fn moment(tables: &DayTables, hour: f32, weather: WeatherState, y: f32) -> Option<Moment> {
    type Key = (usize, u32, WeatherState, i32);
    // ponytail: the last moment held, as a frame asks for the same one many times over.
    static LAST: Mutex<Option<(Key, Option<Moment>)>> = Mutex::new(None);
    // The latitude moves the sun by a hundredth of a degree over a hundred metres.
    let rows = (y / LATITUDE_STEP).round();
    let key = (
        std::ptr::from_ref(tables) as usize,
        hour.to_bits(),
        weather,
        rows as i32,
    );
    let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, moment)) = last.as_ref().filter(|(held, _)| *held == key) {
        return moment.clone();
    }
    let moment = tables.moment(hour, weather_name(weather), rows * LATITUDE_STEP);
    *last = Some((key, moment.clone()));
    moment
}

/// Where the frame's exposure stands: the moment it was metered for, the camera's cut it began at, its stops and when they were set, whether the next reading is the first of that moment, and whether the last reading had reached what the curve wants.
struct Adapting {
    of: (u32, WeatherState),
    cut: u32,
    stops: f32,
    at: Instant,
    fresh: bool,
    reached: bool,
}

// ponytail: one exposure for the one World view; a second view would share it.
static ADAPTING: Mutex<Option<Adapting>> = Mutex::new(None);

/// Whether the frame's exposure is still on its way: no reading of its own moment taken yet, or the last short of what the curve wants of it.
pub fn adapting() -> bool {
    let held = ADAPTING.lock().unwrap_or_else(|p| p.into_inner());
    held.as_ref().is_some_and(|a| a.fresh || !a.reached)
}

/// `moment` exposed by the frame's own luminance, `metered` as last read from a frame drawn under the day set now, for a camera last cut to its place at `cut`: adapted from frame to frame at the pace of the settings, taken whole at the first reading of a new hour or weather or after a cut, and held where nothing was read.
// ⚠ A cut is the camera put down somewhere, never how far it went: a camera flown fast adapts as it flies.
pub fn exposed(
    tables: &DayTables,
    moment: Moment,
    (hour, weather): (f32, WeatherState),
    cut: u32,
    metered: Option<&Metered>,
) -> Moment {
    let of = (hour.to_bits(), weather);
    let now = Instant::now();
    let mut held = ADAPTING.lock().unwrap_or_else(|p| p.into_inner());
    let kept = held.as_mut().filter(|a| a.of == of && a.cut == cut);
    let Some(adapting) = kept else {
        // ⚠ What is read now was drawn under the hour or the place before: the new one waits for its own frame.
        *held = Some(Adapting {
            of,
            cut,
            stops: moment.stops,
            at: now,
            fresh: true,
            reached: false,
        });
        return moment;
    };
    // ⚠ Inferred: the build names these percentages; they are taken as parts of the frame.
    let low = moment.own.get_or("postfx_exposure_lowlights_percent", 0.0);
    let high = moment.own.get_or("postfx_exposure_highlights_percent", 0.0);
    let Some(luminance) = metered.and_then(|m| m.average(low, high)) else {
        return moment.exposed_at(adapting.stops);
    };
    let seconds = match std::mem::take(&mut adapting.fresh) {
        true => 0.0,
        false => now.duration_since(adapting.at).as_secs_f32(),
    };
    let settings = tables.exposure_settings();
    let scene = luminance / adapting.stops.exp2();
    let target = or_title::gta6_exposure::target_stops(&settings, &moment.own, scene);
    let exposed = moment.metered(&settings, luminance, adapting.stops, seconds);
    adapting.reached = (target - exposed.stops).abs() < REACHED;
    // ponytail: a step too small to see is not taken, so that a still frame keeps its day.
    if (exposed.stops - adapting.stops).abs() < STEADY {
        return moment.exposed_at(adapting.stops);
    }
    (adapting.stops, adapting.at) = (exposed.stops, now);
    exposed
}
/// The day the frame is lit by at `hour` of the `days`th day in `weather`, for a camera `y` metres north on the map; `None` where the tables hold no cycle for it.
#[cfg(test)]
pub fn day_of(
    tables: &DayTables,
    hour: f32,
    days: f32,
    weather: WeatherState,
    y: f32,
) -> Option<Day> {
    let moment = moment(tables, hour, weather, y)?;
    Some(day_of_moment(
        tables,
        moment,
        (hour, days),
        weather,
        Vec3::new(0.0, y, 0.0),
    ))
}

/// The day the frame is lit by from `moment`, at `hour` of the `days`th day in `weather`, for a camera at `eye` (z metres above the sea, y metres north).
pub fn day_of_moment(
    tables: &DayTables,
    moment: Moment,
    (hour, days): (f32, f32),
    weather: WeatherState,
    eye: Vec3,
) -> Day {
    let height = eye.z;
    let visual = &tables.visual_settings;
    // The moment's light is stated whole, faded already by its own bodies.
    let stated = Stated {
        sun: Vec3::from(moment.sun),
        moon: Vec3::from(moment.moon),
        towards_light: Vec3::from(moment.bodies.towards),
        sun_fade: moment.bodies.sun_fade,
        moon_fade: moment.bodies.moon_fade,
        exposure: moment.light.exposure,
    };
    let radiance = sky_radiance(&moment, height);
    let sky_view = SkyView {
        clouds: clouds_of(&moment, &radiance),
        radiance,
        exposure: moment.light.exposure,
        night: Some(super::sky_night::night_of(
            tables,
            &moment,
            tables.latitude(eye.y),
        )),
        maps: super::sky_night::maps(),
    };
    let post = post_of(&|name, default| visual.get_f32(name, default), &moment.own);
    Day {
        keyframe: moment.keyframe,
        table: moment.table,
        hours: moment.frame_hours,
        cycle_time: NEAR_FULL_MOON + days + hour / 24.0,
        moon_dim: moment.moon_dim,
        // The Weather bar's own wetness where the install's weather states no rain.
        wetness: moment.wetness.max(weather.wetness()),
        // How far a material's glow has gone to its night value: the cycle's own blend, else on by the night hours.
        night_emissive: (moment.own.get("global_emissive_scale_blend_value")).unwrap_or_else(
            || {
                night_emissive(
                    hour,
                    visual.get_f32("emissive.night.start.time", 20.0) as u32,
                    visual.get_f32("emissive.night.end.time", 7.0) as u32,
                )
            },
        ),
        // ⚠ GTA VI's exposure curve has the frame's name and another shape (a natural logarithm, a
        // bending and offsets in place of a power): the moment is exposed by it already and the
        // frame's own exposure held at nought, so the frame is never given it.
        tone: ToneSettings::from_visual(
            &|name, default| match name.starts_with("Exposure.curve.") {
                true => default,
                false => visual.get_f32(name, default),
            },
        ),
        stated: Some(stated),
        sky_view: Some(sky_view),
        post: Some(post),
    }
}

/// The passes after the frame is lit, from the install's visual settings `visual` and the moment's own keyframe `own`.
fn post_of(
    visual: &dyn Fn(&str, f32) -> f32,
    own: &or_io::gta5::data::timecycle::Keyframe,
) -> Gta6Post {
    use or_renderer::game_shaders::tone::{
        LocalExposure, Temporal, BLOOM_SIZES, COMPENSATION_SAMPLES,
    };
    let get = |name: &str, default: f32| own.get_or(name, default);
    let mut bloom_weights = [0.0; BLOOM_SIZES];
    for (i, weight) in bloom_weights.iter_mut().enumerate() {
        *weight = visual(&format!("bloom.airy.weight{}", 2u32 << i), 0.5);
    }
    let input = [
        get("postfx_exposure_compensation_input_ev_min", -14.0),
        get("postfx_exposure_compensation_input_ev_max", 10.0),
    ];
    let mut compensation = [0.0; COMPENSATION_SAMPLES];
    for (i, sample) in compensation.iter_mut().enumerate() {
        let along = i as f32 / (COMPENSATION_SAMPLES - 1) as f32;
        let ev = input[0] + (input[1] - input[0]) * along;
        *sample = or_title::gta6_exposure::compensation(own, ev);
    }
    let curve = |name: &str| visual(&format!("Exposure.curve.perPixel.{name}"), 0.0);
    Gta6Post {
        bloom_weights,
        bloom_mix: get("postfx_intensity_bloom", 0.0),
        local: LocalExposure {
            amount: get("postfx_exposure_perpixel_amount", 0.0),
            scale: curve("scale"),
            horizontal_offset: curve("horizontalOffset"),
            vertical_offset: curve("verticalOffset"),
            bending: curve("bending"),
            least: get("postfx_exposure_min", -12.0),
            most: get("postfx_exposure_max", 32.0),
            offset: get("postfx_exposure", 0.0) + get("postfx_exposure_tweak", 0.0),
            contrast: get("postfx_exposure_contrast_comp_amount", 0.0),
            compensation,
            input,
            output: [
                get("postfx_exposure_compensation_ev_min", 4.0),
                get("postfx_exposure_compensation_ev_max", -4.0),
            ],
        },
        temporal: Temporal {
            near: visual("temporalaa.convergenceTime", 0.15),
            far: visual("temporalaa.convergenceTimeFar", 1.0),
            far_depth: visual("temporalaa.convergenceTimeDepthMax", 250.0),
            gamma: visual("temporalaa.varianceGammaMin", 1.25),
        },
        sharpen: visual("sharpen.intensity", 0.15),
    }
}

/// The radiance of each texel of a sky table, in cd/m².
type Radiance = Arc<Vec<[f32; 3]>>;

/// The clouds `moment` states, lit by its leading body through the air and by the sky of `radiance` over them, in the frame's units; none where it states no cover.
fn clouds_of(moment: &Moment, radiance: &[[f32; 3]]) -> Option<Clouds> {
    let layer = CloudLayer::of(&moment.own);
    if layer.cover <= 0.0 {
        return None;
    }
    let lit = &moment.bodies;
    let exposure = moment.light.exposure;
    let through = Atmosphere::of(&moment.own).transmittance(lit.body[2]);
    let strength = lit.fade * lit.illuminance * exposure;
    Some(Clouds {
        layer: [layer.base, layer.top, layer.cover, layer.kind],
        // ponytail: the clouds stand still; the wind's drift waits for a speed the build states.
        shape: [layer.sphere, EXTINCTION, layer.anisotropy, 0.0],
        towards: lit.body,
        light: through.map(|t| t * strength),
        sky: hemisphere_radiance(radiance).map(|c| c * exposure),
        haze: moment.light.haze,
        planet: moment.own.get_or("sky_new_planet_radius", 6360.0) * 1000.0,
    })
}

/// The even radiance that lights level ground as the table's upper half does: its irradiance over pi.
fn hemisphere_radiance(radiance: &[[f32; 3]]) -> [f32; 3] {
    use or_renderer::game_shaders::sky_view::{direction, AROUND, UP};
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let mut sum = [0.0f32; 3];
    for (i, texel) in radiance.iter().enumerate() {
        let (x, y) = (i as u32 % AROUND, i as u32 / AROUND);
        let up = direction(x, y).z;
        // A row is `v² π/2` up, so it spans `π v dv` of elevation.
        let v = (y as f32 + 0.5) / UP as f32;
        let solid =
            (TAU / AROUND as f32) * (2.0 * FRAC_PI_2 * v / UP as f32) * (1.0 - up * up).sqrt();
        sum = [0, 1, 2].map(|c| sum[c] + texel[c] * up * solid);
    }
    sum.map(|c| c / PI)
}

/// What the sky's radiance table is worked out from.
#[derive(Clone, PartialEq)]
struct SkyOf {
    air: Atmosphere,
    sun: [f32; 3],
    moon: [f32; 3],
    illuminance: [f32; 2],
    tint: [f32; 3],
    /// The camera's height above the sea, in steps of [`HEIGHT_STEP`].
    height: i32,
}

/// The table last worked out, and the worker's: the table it is working out and the one asked for next.
static TABLE: Mutex<Option<(SkyOf, Radiance)>> = Mutex::new(None);
static WORK: Mutex<(Option<SkyOf>, Option<SkyOf>)> = Mutex::new((None, None));

/// The sky's radiance in every way of the table at `moment` for a camera `height` metres above the sea: what the air scatters of the sun and of the moon, each of its own illuminance, tinted by the moment's sky colour. The first table is worked out at once; a later one on a worker, the last shown until it is ready.
fn sky_radiance(moment: &Moment, height: f32) -> Radiance {
    let own = &moment.own;
    let of = SkyOf {
        height: (height.max(0.0) / HEIGHT_STEP).round() as i32,
        air: Atmosphere::of(own),
        sun: moment.sun,
        moon: moment.moon,
        illuminance: [
            moment.light.sun_illuminance,
            own.get_or("sky_scattering_moon_illuminance", 0.0)
                * own.get_or("sky_scattering_moon_intensity_multiplier", 1.0),
        ],
        tint: ["r", "g", "b"].map(|c| own.get_or(&format!("sky_scattering_sky_col_{c}"), 1.0)),
    };
    let mut held = TABLE.lock().unwrap_or_else(|p| p.into_inner());
    let last = match held.as_ref() {
        Some((was, table)) if *was == of => return Arc::clone(table),
        Some((_, table)) => Arc::clone(table),
        None => {
            let table = worked_out(&of);
            *held = Some((of, Arc::clone(&table)));
            return table;
        }
    };
    drop(held);
    let mut work = WORK.lock().unwrap_or_else(|p| p.into_inner());
    let (working, wanted) = &mut *work;
    if working.as_ref() != Some(&of) && wanted.as_ref() != Some(&of) {
        *wanted = Some(of);
        if working.is_none() {
            std::thread::spawn(work_through);
        }
    }
    last
}

/// Work out the tables asked for, one after another, until none is.
fn work_through() {
    loop {
        let next = {
            let mut work = WORK.lock().unwrap_or_else(|p| p.into_inner());
            let next = work.1.take();
            work.0.clone_from(&next);
            next
        };
        let Some(of) = next else {
            return;
        };
        let table = worked_out(&of);
        *TABLE.lock().unwrap_or_else(|p| p.into_inner()) = Some((of, table));
    }
}

/// The table `of` states, a few tenths of a second's work.
fn worked_out(of: &SkyOf) -> Radiance {
    // A body far under the horizon lights no air the camera sees.
    let lit = |body: [f32; 3], lux: f32| lux > 0.0 && body[2] > LOWEST_BODY;
    let bodies: Vec<([f32; 3], f32)> = [(of.sun, of.illuminance[0]), (of.moon, of.illuminance[1])]
        .into_iter()
        .filter(|(body, lux)| lit(*body, *lux))
        .collect();
    let metres = f64::from(of.height) * f64::from(HEIGHT_STEP);
    SkyView::of(
        |view| {
            let mut sum = [0.0f32; 3];
            for (body, lux) in &bodies {
                let seen = of.air.radiance_from(metres, view.to_array(), *body, *lux);
                sum = [0, 1, 2].map(|c| sum[c] + seen[c]);
            }
            [0, 1, 2].map(|c| sum[c] * of.tint[c])
        },
        1.0,
    )
    .radiance
}

/// What a light the game states in its own unit is multiplied by to be the frame's at `hour` in `weather` for a camera `y` metres north: the moment's exposure over pi, and the old stand-in where no GTA VI day is read.
#[cfg(test)]
pub fn light_scale(tables: Option<&DayTables>, hour: f32, weather: WeatherState, y: f32) -> f32 {
    scale_of(
        tables
            .and_then(|tables| moment(tables, hour, weather, y))
            .as_ref(),
    )
}

/// What a light the game states in its own unit is multiplied by under `moment`: its exposure over pi, and the old stand-in where no GTA VI day is read.
pub fn scale_of(moment: Option<&Moment>) -> f32 {
    moment.map_or(NO_DAY_LIGHT_SCALE, |moment| moment.light_scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_renderer::game_shaders::lighting::View;

    fn tables() -> DayTables {
        let samples: String = (0..13)
            .map(|k| format!(r#"<sample name="s{k}" hour="{k}"/>"#))
            .collect();
        let time = format!("<time_data>{samples}</time_data>");
        let cycle = r#"<timecycle_keyframe_data><cycle name="Sunny" regions="1"><region name="GLOBAL">
            <light_dir_mult> 1 1 1 1 1 1 1 1 1 1 1 1 1</light_dir_mult>
            </region></cycle></timecycle_keyframe_data>"#;
        let settings = "Tonemapping.dark.filmic.W 11.2\nExposure.curve.scale -2.0\nemissive.night.start.time 22\nemissive.night.end.time 4\n";
        DayTables::of_texts([time.as_str(), cycle, settings])
    }

    #[test]
    fn the_frames_sun_stands_where_gta_vis_does_at_every_hour_of_its_day() {
        let tables = tables();
        let view = View {
            camera_height: 0.0,
            projection: glam::Mat4::IDENTITY,
            near: 0.1,
        };
        for hour in [7.5f32, 9.0, 13.45, 16.0, 19.0, 23.0, 2.0] {
            let moment = tables
                .moment(hour, "Sunny", 0.0)
                .expect("the cycle is read");
            let day = day_of(&tables, hour, 0.0, WeatherState::Clear, 0.0).expect("a day");
            let (sun, _) = day.lighting(&view).sun_and_moon();
            let own = glam::Vec3::from(moment.sun);
            assert!(
                (sun - own).length() < 1e-3,
                "at {hour}: {sun:?} and {own:?}"
            );
        }
    }

    #[test]
    fn the_day_takes_the_installs_curve_and_night_hours_and_leaves_its_exposure_curve() {
        let tables = tables();
        let day = day_of(&tables, 23.0, 2.0, WeatherState::Rain, 0.0);
        assert!(day.is_none(), "no cycle stands for rain in these tables");
        let day = day_of(&tables, 23.0, 2.0, WeatherState::Clear, 0.0).expect("a day");
        assert_eq!(day.tone.dark.w, 11.2);
        assert_eq!(day.tone.curve_scale, ToneSettings::default().curve_scale);
        assert_eq!(day.night_emissive, 1.0);
        assert_eq!(day.cycle_time, NEAR_FULL_MOON + 2.0 + 23.0 / 24.0);
        let noon = day_of(&tables, 12.0, 0.0, WeatherState::Clear, 0.0).expect("a day");
        assert_eq!(noon.night_emissive, 0.0);
    }

    #[test]
    fn the_frame_maps_the_moment_at_nought_stops_and_the_lights_take_its_exposure() {
        let tables = tables();
        let view = View {
            camera_height: 0.0,
            projection: glam::Mat4::IDENTITY,
            near: 0.1,
        };
        for hour in [13.45f32, 1.0] {
            let day = day_of(&tables, hour, 0.0, WeatherState::Clear, 0.0).expect("a day");
            assert_eq!(day.tone(&day.lighting(&view)).stops, 0.0, "at {hour}");
            let moment = tables.moment(hour, "Sunny", 0.0).expect("a moment");
            let scale = light_scale(Some(&tables), hour, WeatherState::Clear, 0.0);
            assert_eq!(scale, moment.light_scale);
            assert_eq!(scale_of(Some(&moment)), scale);
            let exposed = moment.stops.exp2() / std::f32::consts::PI;
            assert!(
                (scale - exposed).abs() <= 1e-6 * exposed,
                "{scale} {exposed}"
            );
            let once = day_of_moment(
                &tables,
                moment,
                (hour, 0.0),
                WeatherState::Clear,
                Vec3::ZERO,
            );
            assert_eq!(once.hours, day.hours, "at {hour}");
        }
        let none = light_scale(None, 12.0, WeatherState::Clear, 0.0);
        assert_eq!(none, NO_DAY_LIGHT_SCALE);
        assert_eq!(scale_of(None), NO_DAY_LIGHT_SCALE);
    }

    #[test]
    fn the_frame_is_exposed_by_its_own_luminance_from_the_first_reading_of_its_own_hour() {
        let tables = tables();
        let at = |metered: Option<&Metered>| {
            let moment = moment(&tables, 9.5, WeatherState::Clear, 0.0).expect("a moment");
            exposed(&tables, moment, (9.5, WeatherState::Clear), 0, metered)
        };
        let unread = moment(&tables, 9.5, WeatherState::Clear, 0.0).expect("a moment");
        // What is read in the first frame of an hour was drawn under the hour before.
        let bright = Metered::of(vec![8.0; 16]);
        let grey = at(Some(&bright));
        assert_eq!(grey.stops, unread.stops);
        // The first reading of its own is taken whole: the curve over the scene's luminance.
        let first = at(Some(&bright));
        let scene = 8.0 / grey.stops.exp2();
        let curve = -2.0 * scene.ln() - 10.0;
        assert!(
            (first.stops - curve).abs() < 1e-3,
            "{} {curve}",
            first.stops
        );
        assert_eq!(first.light_scale, first.stops.exp2() / std::f32::consts::PI);
        // A frame that reads the same scene keeps its exposure; nothing read holds it.
        let same = Metered::of(vec![scene * first.stops.exp2(); 16]);
        assert_eq!(at(Some(&same)).stops, first.stops);
        assert_eq!(at(None).stops, first.stops);
        // A scene four times as bright is moved towards, not jumped to, a moment later.
        let brighter = Metered::of(vec![4.0 * scene * first.stops.exp2(); 16]);
        let next = at(Some(&brighter)).stops;
        let wanted = curve - 2.0 * 4f32.ln();
        assert!(next <= first.stops && next > wanted, "{next} {wanted}");
        assert!(adapting(), "still on its way");
        // A cut of the camera starts again, and its first reading is taken whole.
        let cut = |metered: Option<&Metered>| {
            let moment = moment(&tables, 9.5, WeatherState::Clear, 0.0).expect("a moment");
            exposed(&tables, moment, (9.5, WeatherState::Clear), 1, metered)
        };
        assert_eq!(cut(Some(&brighter)).stops, unread.stops);
        // A scene of one candela a square metre, as the frame drawn at the cut's first stops holds it.
        let lit = Metered::of(vec![unread.stops.exp2(); 16]);
        let whole = cut(Some(&lit)).stops;
        let scene = 1.0;
        let settings = tables.exposure_settings();
        let curve = or_title::gta6_exposure::target_stops(&settings, &unread.own, scene);
        assert!(
            whole != unread.stops && (whole - curve).abs() < 1e-3,
            "{whole} {curve}"
        );
        assert!(!adapting(), "arrived");
    }

    #[test]
    fn an_even_sky_lights_its_clouds_with_its_own_radiance_and_a_clear_cycle_draws_none() {
        let even = SkyView::of(|_| [2.0, 3.0, 4.0], 1.0);
        let seen = hemisphere_radiance(&even.radiance);
        for (got, want) in seen.iter().zip([2.0, 3.0, 4.0]) {
            assert!((got - want).abs() < 0.02 * want, "{seen:?}");
        }
        let day = day_of(&tables(), 12.0, 0.0, WeatherState::Clear, 0.0).expect("a day");
        assert!(day.sky_view.expect("a table").clouds.is_none());
    }

    #[test]
    fn the_night_glows_by_the_moments_own_air_glow_in_thousandths() {
        let mut tables = tables();
        // ⚠ The last moment is held by the tables' address and the hour: each reading takes an hour of its own.
        let at = |tables: &DayTables, hour: f32| {
            let day = day_of(tables, hour, 0.0, WeatherState::Clear, 0.0).expect("a day");
            day.sky_view.expect("a table").night.expect("a night")
        };
        assert_eq!(
            at(&tables, 0.0).fill,
            [0.0; 3],
            "these tables state no glow"
        );
        // A cycle that states the air's glow: its base colour times its intensity, in thousandths.
        for (hour, intensity) in [(1.0, 1.0f32), (2.0, 0.1)] {
            let cycle = format!(
                r#"<timecycle_keyframe_data><cycle name="Sunny" regions="1"><region name="GLOBAL">
                <sky_air_glow_base_color_b>{}</sky_air_glow_base_color_b>
                <sky_air_glow_base_intensity>{}</sky_air_glow_base_intensity>
                </region></cycle></timecycle_keyframe_data>"#,
                " 0.5".repeat(13),
                format!(" {intensity}").repeat(13),
            );
            tables.timecycle = DayTables::of_texts([cycle.as_str()]).timecycle;
            let fill = at(&tables, hour).fill;
            let want = 0.5 * intensity * 0.001;
            assert!((fill[2] - want).abs() < 1e-8, "{fill:?} {want}");
        }
    }

    #[test]
    fn the_day_carries_the_installs_bloom_weights_temporal_times_and_sharpening() {
        let settings = |name: &str, default: f32| match name {
            "bloom.airy.weight2" => 0.712,
            "bloom.airy.weight256" => 0.317,
            "temporalaa.convergenceTimeFar" => 1.0,
            "sharpen.intensity" => 0.15,
            _ => default,
        };
        let mut own = or_io::gta5::data::timecycle::Keyframe::default();
        own.set("postfx_intensity_bloom", 0.1498);
        own.set("postfx_exposure_perpixel_amount", 0.2);
        let post = post_of(&settings, &own);
        assert_eq!(post.bloom_weights[0], 0.712);
        assert_eq!(post.bloom_weights[7], 0.317);
        // A size the settings leave out takes the executable's own weight
        assert_eq!(post.bloom_weights[3], 0.5);
        assert_eq!((post.bloom_mix, post.local.amount), (0.1498, 0.2));
        assert_eq!((post.temporal.far, post.sharpen), (1.0, 0.15));
        // A keyframe that states no compensation curve compensates nothing
        assert!(post.local.compensation.iter().all(|c| *c == 0.0));
    }

    #[test]
    fn every_weather_of_the_bar_names_a_cycle() {
        use WeatherState::*;
        let names = [Clear, Cloudy, Fog, Rain, Storm].map(weather_name);
        assert!(names.iter().all(|name| !name.is_empty()));
        assert_eq!(
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            5
        );
    }
}
