//! How GTA VI exposes its frame: which of the sun and the moon lights and how faded, the stops its
//! key value takes over a metered luminance with the keyframe's clamps and its adaptation, and the
//! light of a moment that comes of them.

use std::f32::consts::PI;

use glam::Vec3;

use or_io::gta5::data::timecycle::Keyframe;
use or_io::gta6::timecycle::{FrameLight, DOME_TRANSITION};

use crate::gta6_cloud::CloudLayer;
use crate::gta6_day_sun::{directional_light, Leader, LightTunables, LightVariables};
use crate::gta6_sky::Atmosphere;

/// The least luminance a key value is divided by.
const LEAST_LUMINANCE: f32 = 1e-6;
/// The grey the stand-in meter sees the world as, and the key value where a keyframe states none.
const GREY: f32 = 0.18;
/// The up part of a way along the horizon, where the dome and the fog take their colours.
const HORIZON: f32 = 0.02;
/// The crossfade a keyframe states for the fades worked out from the zenith angles.
const AUTOMATIC: f32 = -10.0;

/// The luminance of a colour.
fn luminance(c: [f32; 3]) -> f32 {
    0.2125 * c[0] + 0.7154 * c[1] + 0.0721 * c[2]
}

/// What the visual settings state of the exposure: the steps adaptation is held between and the bodies' fades.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ExposureSettings {
    /// The least and the most stops a second adaptation moves by.
    pub min_step: f32,
    pub max_step: f32,
    /// How fast adaptation closes on its target where a keyframe states no step multiplier of its own.
    pub step_mult: f32,
    /// The zenith angles the sun and the moon fade over, and how the moon's phase dims it.
    pub bodies: LightTunables,
}

impl ExposureSettings {
    /// The settings `get` reads from the visual settings by name, each with the executable's own value where there is none.
    pub fn of(get: &dyn Fn(&str, f32) -> f32) -> Self {
        let image = LightTunables::image_defaults();
        Self {
            // The executable looks these up by the hash of the settings' names.
            bodies: LightTunables {
                sun_zenith_start: get("misc.SunFade.Threshold", image.sun_zenith_start),
                sun_zenith_ramp: get("misc.SunFade.ThresholdLength", image.sun_zenith_ramp),
                moon_zenith_start: get("misc.MoonFade.Threshold", image.moon_zenith_start),
                moon_zenith_ramp: get("misc.MoonFade.ThresholdLength", image.moon_zenith_ramp),
                moon_phase: get("misc.Moon.Cycle", image.moon_phase),
                moon_floor: get("misc.MoonDimMult", image.moon_floor),
            },
            min_step: get("Adaptation.min.step.size", 0.15),
            max_step: get("Adaptation.max.step.size", 15.0),
            step_mult: get("Adaptation.step.size.mult", 9.0),
        }
    }
}

/// The compensation curve the per-pixel exposure reads at `ev`: two cubic Béziers over the keyframe's seven points, the input held to the first and last x; nought where the keyframe does not state all seven.
pub fn compensation(k: &Keyframe, ev: f32) -> f32 {
    let point = |i: usize| {
        let at = |axis: &str| k.get(&format!("postfx_exposure_compensation_ev_cp{i}_{axis}"));
        Some((at("x")?, at("y")?))
    };
    let Some(points) = (0..7).map(point).collect::<Option<Vec<_>>>() else {
        return 0.0;
    };
    let x = ev.clamp(points[0].0.min(points[6].0), points[6].0.max(points[0].0));
    let segment = match x < points[3].0 {
        true => &points[0..4],
        false => &points[3..7],
    };
    let along = |t: f32, pick: fn(&(f32, f32)) -> f32| {
        let u = 1.0 - t;
        let [p0, p1, p2, p3] = [0, 1, 2, 3].map(|i| pick(&segment[i]));
        u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
    };
    // ponytail: x(t) is solved by halving, which holds while the segment's x rises; which root the game keeps where it does not is not read.
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..32 {
        let middle = 0.5 * (low + high);
        match along(middle, |p| p.0) < x {
            true => low = middle,
            false => high = middle,
        }
    }
    along(0.5 * (low + high), |p| p.1)
}

/// The stops the frame wants at an average luminance of `luminance` cd/m²: the keyframe's key value over it, in stops, held to the keyframe's least and most with its exposure and tweak added.
// The build's exposure pass takes this form whatever the key-value mode states: the visual settings'
// curves and the compensation curve are read by the per-pixel exposure alone.
pub fn target_stops(k: &Keyframe, luminance: f32) -> f32 {
    let get = |name: &str, default: f32| k.get_or(name, default);
    let ev = (get("postfx_exposure_key_value", GREY) / luminance.max(LEAST_LUMINANCE)).log2();
    let least = get("postfx_exposure_min", f32::NEG_INFINITY);
    let most = get("postfx_exposure_max", f32::INFINITY);
    let hold = |stops: f32| stops.max(least.min(most)).min(most);
    hold(hold(ev) + get("postfx_exposure", 0.0) + get("postfx_exposure_tweak", 0.0))
}

/// The stops `seconds` after a frame at `previous`, moving towards `target` at a rate held between the settings' steps and never past it; a frame with no time behind it takes the target.
pub fn adapt(
    settings: &ExposureSettings,
    k: &Keyframe,
    previous: f32,
    target: f32,
    seconds: f32,
) -> f32 {
    let difference = target - previous;
    if difference == 0.0 || seconds <= 0.0 {
        return target;
    }
    let mult = k.get_or("postfx_exposure_adaption_step_mult", settings.step_mult);
    let rate = (difference.abs() * (seconds * mult).clamp(0.0, 1.0) / seconds)
        .max(settings.min_step.min(settings.max_step))
        .min(settings.max_step);
    let moved = previous + difference.signum() * rate * seconds;
    match difference > 0.0 {
        true => moved.min(target),
        false => moved.max(target),
    }
}

/// The light variables of the moment `k`, by their names, each at `default` where `k` does not state it.
pub fn light_variables(k: &Keyframe) -> LightVariables {
    let get = |name: &str, default: f32| k.get_or(name, default);
    let rgb = |stem: &str, default: f32| {
        Vec3::new(
            get(&format!("{stem}_r"), default),
            get(&format!("{stem}_g"), default),
            get(&format!("{stem}_b"), default),
        )
    };
    let xyz = |stem: &str| {
        Vec3::new(
            get(&format!("{stem}_x"), 0.0),
            get(&format!("{stem}_y"), -1.0),
            get(&format!("{stem}_z"), 0.0),
        )
    };
    LightVariables {
        day_colour: rgb("light_dir_col", 1.0),
        day_intensity: get("light_dir_mult", 1.0),
        sun_multiplier: get("light_dir_sun_mult", 1.0),
        moon_multiplier: get("light_dir_moon_mult", 1.0),
        third_colour: rgb("galaxy_light_dir_col", 1.0),
        third_intensity: get("galaxy_light_dir_mult", 1.0),
        pull_amount: get("light_direction_override", 0.0),
        disc_pull_scale: get("light_direction_override_overrides_sun", 1.0),
        sun_pull: get("sun_direction_override", 0.0),
        moon_pull: get("moon_direction_override", 0.0),
        sun_target: xyz("sun_direction"),
        moon_target: xyz("moon_direction"),
        sun_zenith_threshold: get("sun_fade_threshold", -1.0),
        moon_zenith_threshold: get("moon_fade_threshold", -1.0),
        leader_override: get("sun_moon_crossfade", AUTOMATIC),
    }
}

/// Which body lights a moment, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bodies {
    pub sun_fade: f32,
    pub moon_fade: f32,
    /// The weight of neither body being up, which the night sky's galaxy terms take.
    pub third_fade: f32,
    pub sun_leads: bool,
    /// The way towards the body that leads.
    pub body: [f32; 3],
    /// The way the light comes from: the bodies' blended, held to 0.15 up at least; straight down where neither lights.
    pub towards: [f32; 3],
    /// The light's weight times the leading body's own multiplier.
    pub fade: f32,
    /// The leading body's illuminance, and the sun's, in lux.
    pub illuminance: f32,
    pub sun_illuminance: f32,
}

/// Which of the sun towards `sun` and the moon towards `moon` lights the moment `k`, by the build's weights of their zenith angles over the ramps `tunables` state, or by its crossfade.
pub fn bodies(k: &Keyframe, sun: [f32; 3], moon: [f32; 3], tunables: &LightTunables) -> Bodies {
    let get = |name: &str, default: f32| k.get_or(name, default);
    // The moon's phase is the clock's part of the day, which dims nothing while the settings state a phase of one half.
    let state = directional_light(
        Vec3::from(sun),
        Vec3::from(moon),
        Vec3::Z,
        0.0,
        &light_variables(k),
        tunables,
    );
    let sun_leads = state.weights.leader() == Leader::Sun;
    let sun_illuminance = get("sky_scattering_sun_illuminance", 0.0)
        * get("sky_scattering_sun_intensity_multiplier", 1.0);
    let moon_illuminance = get("sky_scattering_moon_illuminance", 0.0)
        * get("sky_scattering_moon_intensity_multiplier", 1.0);
    Bodies {
        sun_fade: state.weights.sun,
        moon_fade: state.weights.moon,
        third_fade: state.weights.third,
        sun_leads,
        body: if sun_leads { sun } else { moon },
        towards: state.direction.into(),
        fade: state.intensity_weight,
        // ⚠ The CPU gives the light no unit: the illuminance of its body is the frame's stand-in for what the GPU multiplies it by.
        illuminance: if sun_leads {
            sun_illuminance
        } else {
            moon_illuminance
        },
        sun_illuminance,
    }
}

/// The light of the moment `k` with the sun towards `sun` and the moon towards `moon`, and the stops it is exposed at: the target over a grey world under that light, with no history.
// ponytail: a grey world stands in for the game's meter of its own frame (a trimmed mean of 256
// logarithmic bins); the frame has no meter yet, and with one the stops are `adapt`ed from frame to frame.
pub fn light(
    k: &Keyframe,
    sun: [f32; 3],
    moon: [f32; 3],
    settings: &ExposureSettings,
) -> (FrameLight, f32) {
    let get = |name: &str, default: f32| k.get_or(name, default);
    let rgb = |stem: &str| ["r", "g", "b"].map(|c| get(&format!("{stem}_{c}"), 1.0));
    let air = Atmosphere::of(k);
    let lit = bodies(k, sun, moon, &settings.bodies);
    let clear = air.transmittance(lit.body[2]);
    // ⚠ A stand-in shared with the dome: the ground takes the sky the dome draws, of the same air lit
    // by its body where it stands, under the horizon too, in place of the build's atmosphere tables.
    let sky_light = lit.body;
    let clear_sky = air.irradiance(sky_light, lit.illuminance);
    // The clouds shade the ground from the body by what they cover, and send on down as the sky's
    // light what they scatter of it: a storm's dark is its cloud, its sun as strong as a clear noon's.
    let clouds = CloudLayer::of(k);
    let up = lit.towards[2];
    let body_transmittance = clear.map(|t| t * clouds.direct_through(up));
    let scattered = clouds.diffuse_through(up) * lit.fade * lit.illuminance * up;
    // The clouds take the body's light in their own colour and strength, as the frame's block hands the local atmosphere it.
    let cloud_light = clouds.light;
    let sky = [0, 1, 2]
        .map(|c| clear_sky[c] * (1.0 - clouds.cover) + scattered * clear[c] * cloud_light[c]);

    let tint = rgb("light_dir_col");
    let direct = get("light_dir_mult", 1.0) * lit.fade * lit.illuminance;
    let direct = luminance([0, 1, 2].map(|c| tint[c] * body_transmittance[c])) * direct;
    let tints = [rgb("sky_scattering_sky_col"), rgb("ambient_sky_color_mult")];
    let ambient = luminance([0, 1, 2].map(|c| sky[c] * tints[0][c] * tints[1][c]))
        * get("ambient_sky_intensity", 1.0);
    let metered = GREY / PI * (direct * lit.towards[2] + ambient);
    let stops = target_stops(k, metered);

    let radiance = |view: [f32; 3]| air.radiance(view, sky_light, lit.illuminance);
    let level = |x: f32, y: f32| match x.hypot(y) > 1e-3 {
        true => [x, y, HORIZON],
        false => [1.0, 0.0, HORIZON],
    };
    let under = level(lit.body[0], lit.body[1]);
    let opposite = [-under[0], -under[1], HORIZON];
    let across = (1.0 - DOME_TRANSITION * DOME_TRANSITION).sqrt();
    let [east, west, north, south] = [[across, 0.0], [-across, 0.0], [0.0, across], [0.0, -across]]
        .map(|[x, y]| radiance([x, y, DOME_TRANSITION]));
    let transition = [0, 1, 2].map(|c| (north[c] + south[c]) / 2.0);
    let light = FrameLight {
        exposure: stops.exp2(),
        fade: lit.fade,
        illuminance: lit.illuminance,
        sun_illuminance: lit.sun_illuminance,
        body_transmittance,
        sun_transmittance: air.transmittance(sun[2]),
        towards_up: lit.towards[2],
        sky,
        dome: [
            radiance([0.0, 0.0, 1.0]),
            transition,
            radiance([1.0, 0.0, HORIZON]),
            radiance([-1.0, 0.0, HORIZON]),
            radiance([0.0, -1.0, HORIZON]),
        ],
        dome_sides: [east, west],
        fog: [under, [1.0, 0.0, HORIZON], opposite, [0.0, -1.0, HORIZON]].map(radiance),
        haze: luminance(air.ground_extinction()) / 1000.0,
    };
    (light, stops)
}

#[cfg(test)]
#[path = "gta6_exposure_tests.rs"]
mod tests;
