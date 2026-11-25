//! What GTA VI's day adds to its weather cycles, which GTA V's reader reads as they are: a time
//! table whose samples are places of the sun, the modifiers, and the
//! variables a frame built for GTA V's day takes, worked out from GTA VI's.

use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

use crate::gta5::data::timecycle::{attr, tokens, Keyframe, Token};

/// The latitude at the middle of the map, in radians, for an install whose settings state no range.
pub const LATITUDE: f32 = 25.8 * PI / 180.0;

/// Where a cycle's samples stand on the sun's round, east to west through the top: the angle of
/// the sun in the plane of east and up, nought where it rises, and the sample that holds there.
// The half turn is written short, and the round is closed by as short a turn: neither is pi.
#[allow(clippy::approx_constant)]
pub const SAMPLE_ANGLES: [(f32, usize); 13] = [
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

/// The hours of a frame's own day between which its sun rises and sets.
// ⚠ These are the frame's, not GTA VI's: `frame_hours` and `sun_lights` hold only while the
// frame's sun rises at 6, sets at 20 and hands over to the moon an hour and three quarters after.
const FRAME_RISE: f32 = 6.0;
const FRAME_SET: f32 = 20.0;
const FRAME_HANDOVER: f32 = 1.75;

/// What the frame's diffuse lacks beside a physical one, which every light it takes is multiplied by: it does not divide by pi.
// ⚠ GTA V's frame, assumed: a grey card under the noon sun should show about 1.4 before the tone
// curve. If it shows about 4.5, this is 1.
pub const FRAME_DIFFUSE: f32 = std::f32::consts::FRAC_1_PI;
/// The illuminance, in lux, of the frame's artificial ambient light, which a surface takes the share its G-buffer states of.
// ⚠ GTA VI states no exterior artificial ambient light: the frame's carries the light a room's
// lamps bounce, worked out per room, and a share of one is this much of it.
pub const ARTIFICIAL_AMBIENT_LUX: f32 = 1000.0;
/// The albedo of the ground the light bounces off, for the frame's light from below and the air's multiple scattering.
// ponytail: a grey ground in place of the game's traced bounce light, which the frame has no pass for.
pub const GROUND_ALBEDO: f32 = 0.2;
/// The up part of the way at which the frame's dome takes its transition colour whole: low, where the sky's radiance bends.
pub const DOME_TRANSITION: f32 = 0.25;
/// The frame's units of fog: its density is per ten thousand metres and its falloff per thousand.
const FRAME_FOG_DENSITY: f32 = 10_000.0;
const FRAME_FOG_FALLOFF: f32 = 1_000.0;

/// One sample of the day: a place of the sun, named by an hour, and the name of the curve the way from it to the next is bent by.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub name: String,
    pub hour: f32,
    pub interpolation_type: String,
}

/// GTA VI's time table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DayTable {
    pub samples: Vec<Sample>,
    pub regions: Vec<String>,
}

impl DayTable {
    pub fn parse(text: &str) -> Self {
        let mut table = Self::default();
        for token in tokens(text) {
            let Token::Open { name, attrs, .. } = token else {
                continue;
            };
            match name {
                "sample" => table.samples.push(Sample {
                    name: attr(attrs, "name").unwrap_or_default().to_string(),
                    hour: (attr(attrs, "hour").and_then(|v| v.trim().parse().ok())).unwrap_or(0.0),
                    interpolation_type: (attr(attrs, "interpolation_type"))
                        .unwrap_or_default()
                        .to_string(),
                }),
                "region" => (table.regions).extend(attr(attrs, "name").map(str::to_string)),
                _ => {}
            }
        }
        table
    }
}

/// The lean of the sun's path a frame's own time table takes for a sun at `latitude`, in radians.
pub fn sun_roll(latitude: f32) -> f32 {
    PI / 2.0 + latitude
}

/// The hour of a frame's own day at which its sun is `round` along its round.
pub fn frame_hours(round: f32) -> f32 {
    let round = round.rem_euclid(TAU);
    if round <= PI {
        return FRAME_RISE + (FRAME_SET - FRAME_RISE) * round / PI;
    }
    // The night is two quarter turns, each as long as the frame's clock gives it.
    let quarter = (round - PI) / (PI / 2.0);
    match quarter < 1.0 {
        true => FRAME_SET + (24.0 - FRAME_SET) * quarter,
        false => FRAME_RISE * (quarter - 1.0),
    }
}

/// Whether a frame at the hour `frame_hours` of its own day is lit by its sun, not its moon.
pub fn sun_lights(frame_hours: f32) -> bool {
    (FRAME_RISE - FRAME_HANDOVER + 0.5..FRAME_SET + FRAME_HANDOVER).contains(&frame_hours)
}

/// The single values a cycle's file states before its cycle (`High_Level_Vars`), by name.
pub fn high_level_vars(text: &str) -> BTreeMap<String, f32> {
    let mut vars = BTreeMap::new();
    let (mut inside, mut open) = (false, None);
    for token in tokens(text) {
        match token {
            Token::Open {
                name: "High_Level_Vars",
                ..
            } => inside = true,
            Token::Close("High_Level_Vars") => break,
            Token::Open { name, empty, .. } if inside && !empty => open = Some(name),
            Token::Text(text) => {
                if let (Some(name), Ok(value)) = (open, text.trim().parse()) {
                    vars.insert(name.to_string(), value);
                }
            }
            _ => open = None,
        }
    }
    vars
}

/// A timecycle modifier: each variable it states with its two values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Modifier {
    pub name: String,
    pub user_flags: u32,
    pub vars: BTreeMap<String, [f32; 2]>,
}

/// Every modifier a `timecycle_modifier_data` text holds; a variable that does not state two numbers is left out.
pub fn modifiers(text: &str) -> Vec<Modifier> {
    let mut out: Vec<Modifier> = Vec::new();
    let (mut inside, mut open) = (false, None);
    for token in tokens(text) {
        match token {
            Token::Open {
                name: "modifier",
                attrs,
                empty,
            } => {
                out.push(Modifier {
                    name: attr(attrs, "name").unwrap_or_default().to_string(),
                    user_flags: (attr(attrs, "userFlags").and_then(|v| v.trim().parse().ok()))
                        .unwrap_or(0),
                    vars: BTreeMap::new(),
                });
                inside = !empty;
            }
            Token::Close("modifier") => inside = false,
            Token::Open { name, empty, .. } if inside && !empty => open = Some(name),
            Token::Text(text) => {
                let values: Vec<f32> = (text.split_whitespace())
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if let (Some(name), Some(modifier), &[a, b]) = (open, out.last_mut(), &values[..]) {
                    modifier.vars.insert(name.to_string(), [a, b]);
                }
            }
            _ => open = None,
        }
    }
    out
}

/// How a modifier's two values meet a variable of the keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKind {
    /// The first value replaces it.
    Replace,
    /// It is taken to the first value, wholly.
    Interp,
    /// It is taken towards the first value by the second.
    InterpMult,
    /// It is held between the two values.
    Clamp,
    /// It is at least the first value.
    Max,
    /// The modifier leaves it: a level-of-detail scale.
    Leave,
}

/// How a modifier meets the variable `name`.
// ⚠ Measured on the names both titles' variable tables hold: the level-of-detail scales are left,
// the directional light's tint and strengths blend by the second value, and every other lighting,
// sky, exposure and fog variable is taken to the first. Which variables clamp or take a maximum is not read.
pub fn modifier_kind(name: &str) -> ModifierKind {
    match name {
        _ if name.starts_with("lod_mult") => ModifierKind::Leave,
        "light_dir_col_r" | "light_dir_col_g" | "light_dir_col_b" | "light_dir_mult"
        | "light_dir_sun_mult" => ModifierKind::InterpMult,
        _ => ModifierKind::Interp,
    }
}

/// The modifier `a` blended with `b` by `along` (0 to 1), value by value; a variable one of them does not state is the other's.
pub fn blend_modifiers(
    a: &BTreeMap<String, [f32; 2]>,
    b: &BTreeMap<String, [f32; 2]>,
    along: f32,
) -> BTreeMap<String, [f32; 2]> {
    let mut out = a.clone();
    for (name, to) in b {
        let value = match a.get(name) {
            Some(from) => [0, 1].map(|i| from[i] + (to[i] - from[i]) * along),
            None => *to,
        };
        out.insert(name.clone(), value);
    }
    out
}

/// Lay the modifier `vars` over `keyframe` at full strength, each variable by its [`modifier_kind`].
pub fn apply_modifier(keyframe: &mut Keyframe, vars: &BTreeMap<String, [f32; 2]>) {
    // ponytail: no strength is stated for a weather's modifiers, so it is taken as one: a blend
    // towards the first value is that value, and the maximum weighs the cycle's own by nought.
    for (name, &[a, b]) in vars {
        let value = match (modifier_kind(name), keyframe.get(name)) {
            (ModifierKind::Leave, _) => continue,
            (ModifierKind::Replace | ModifierKind::Interp, _) => a,
            (_, None) => continue,
            (ModifierKind::InterpMult, Some(base)) => base + (a - base) * b,
            (ModifierKind::Clamp, Some(base)) => base.max(a.min(b)).min(a.max(b)),
            (ModifierKind::Max, Some(_)) => a.max(0.0),
        };
        keyframe.set(name, value);
    }
}

/// The variables GTA VI states under the name and in the unit a frame built for GTA V's day reads them by.
const SAME: &[&str] = &[
    "dir_shadow_distance_multiplier",
    "far_clip",
    "fog_start",
    "light_artificial_int_down_col_r",
    "light_artificial_int_down_col_g",
    "light_artificial_int_down_col_b",
    "light_artificial_int_down_intensity",
    "light_artificial_int_up_col_r",
    "light_artificial_int_up_col_g",
    "light_artificial_int_up_col_b",
    "light_artificial_int_up_intensity",
    "light_direction_override",
    "light_direction_override_overrides_sun",
    "sun_direction_x",
    "sun_direction_y",
    "sun_direction_z",
    "moon_direction_x",
    "moon_direction_y",
    "moon_direction_z",
    "postfx_correct_col_r",
    "postfx_correct_col_g",
    "postfx_correct_col_b",
    "postfx_correct_cutoff",
    "postfx_shift_col_r",
    "postfx_shift_col_g",
    "postfx_shift_col_b",
    "postfx_shift_cutoff",
    "postfx_desaturation",
    "postfx_intensity_bloom",
    "reflection_tweak_directional",
    "sky_moon_col_r",
    "sky_moon_col_g",
    "sky_moon_col_b",
    "sky_moon_disc_size",
    "sky_sun_disc_size",
];

/// The variables [`frame_keyframe`] works out from GTA VI's under other names, whatever the keyframe states.
const WORKED_OUT: &[&str] = &[
    "postfx_exposure",
    "postfx_exposure_min",
    "postfx_exposure_max",
    "postfx_tonemap_filmic_override_dark",
    "postfx_tonemap_filmic_override_bright",
    "light_dir_col_r",
    "light_dir_col_g",
    "light_dir_col_b",
    "light_dir_mult",
    "light_natural_amb_down_col_r",
    "light_natural_amb_down_col_g",
    "light_natural_amb_down_col_b",
    "light_natural_amb_down_intensity",
    "light_natural_amb_up_col_r",
    "light_natural_amb_up_col_g",
    "light_natural_amb_up_col_b",
    "light_natural_amb_up_intensity",
    "light_natural_amb_up_intensity_mult",
    "light_artificial_ext_down_col_r",
    "light_artificial_ext_down_col_g",
    "light_artificial_ext_down_col_b",
    "light_artificial_ext_down_intensity",
    "light_artificial_ext_up_col_r",
    "light_artificial_ext_up_col_g",
    "light_artificial_ext_up_col_b",
    "light_artificial_ext_up_intensity",
    "sky_zenith_col_r",
    "sky_zenith_col_g",
    "sky_zenith_col_b",
    "sky_zenith_col_inten",
    "sky_zenith_transition_col_r",
    "sky_zenith_transition_col_g",
    "sky_zenith_transition_col_b",
    "sky_zenith_transition_col_inten",
    "sky_azimuth_east_col_r",
    "sky_azimuth_east_col_g",
    "sky_azimuth_east_col_b",
    "sky_azimuth_east_col_inten",
    "sky_azimuth_west_col_r",
    "sky_azimuth_west_col_g",
    "sky_azimuth_west_col_b",
    "sky_azimuth_west_col_inten",
    "sky_azimuth_transition_col_r",
    "sky_azimuth_transition_col_g",
    "sky_azimuth_transition_col_b",
    "sky_azimuth_transition_col_inten",
    "sky_hdr",
    "sky_zenith_transition_position",
    "sky_zenith_transition_east_blend",
    "sky_zenith_transition_west_blend",
    "sky_zenith_blend_start",
    "sky_sun_col_r",
    "sky_sun_col_g",
    "sky_sun_col_b",
    "sky_sun_disc_col_r",
    "sky_sun_disc_col_g",
    "sky_sun_disc_col_b",
    "sky_sun_hdr",
    "fog_col_r",
    "fog_col_g",
    "fog_col_b",
    "fog_east_col_r",
    "fog_east_col_g",
    "fog_east_col_b",
    "fog_near_col_r",
    "fog_near_col_g",
    "fog_near_col_b",
    "fog_haze_col_r",
    "fog_haze_col_g",
    "fog_haze_col_b",
    "fog_hdr",
    "fog_haze_hdr",
    "fog_haze_density",
    "sky_cloud_hdr",
];

/// The variables [`frame_keyframe`] works out from GTA VI's where the keyframe states what they come from.
const WHERE_STATED: &[&str] = &[
    "sky_moon_iten",
    "sky_stars_iten",
    "fog_density",
    "fog_base_height",
    "fog_falloff",
];

/// The names of a frame's variables [`frame_keyframe`] fills from a keyframe that states all it reads.
pub fn filled() -> impl Iterator<Item = &'static str> {
    SAME.iter().chain(WORKED_OUT).chain(WHERE_STATED).copied()
}

/// The sky colours of the frame's dome, in the order of [`FrameLight::dome`].
const DOME: [&str; 5] = [
    "sky_zenith_col",
    "sky_zenith_transition_col",
    "sky_azimuth_east_col",
    "sky_azimuth_west_col",
    "sky_azimuth_transition_col",
];
/// The frame's fog colours, in the order of [`FrameLight::fog`].
const FOG: [&str; 4] = ["fog_col", "fog_east_col", "fog_near_col", "fog_haze_col"];

/// What one moment's light is, worked out from GTA VI's keyframe, its atmosphere and its exposure: what [`frame_keyframe`] gives the frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameLight {
    /// What every light of the moment is multiplied by: two to the power of its stops.
    pub exposure: f32,
    /// The fade of the body that lights, times that body's own multiplier.
    pub fade: f32,
    /// The illuminance of the body that lights, in lux.
    pub illuminance: f32,
    /// The sun's illuminance, in lux, whichever body lights.
    pub sun_illuminance: f32,
    /// What the air lets through towards the body that lights.
    pub body_transmittance: [f32; 3],
    /// What the air lets through towards the sun.
    pub sun_transmittance: [f32; 3],
    /// The up part of the way towards the light, which the frame holds to at least 0.15.
    pub towards_up: f32,
    /// The sky's irradiance on the ground in lux, before the cycle's tints.
    pub sky: [f32; 3],
    /// The sky's radiance in cd/m² at the zenith, north and south at [`DOME_TRANSITION`] up, and on the horizon east, west and south.
    pub dome: [[f32; 3]; 5],
    /// The sky's radiance at [`DOME_TRANSITION`] up, east and west, which the dome reaches by blending its horizon towards its transition colour.
    pub dome_sides: [[f32; 3]; 2],
    /// The sky's radiance in cd/m² on the horizon under the light, east, opposite the light and south.
    pub fog: [[f32; 3]; 4],
    /// What the air takes out of light per metre at the ground, by luminance: the haze over distance.
    pub haze: f32,
}

/// The variables of a frame built for GTA V's day, worked out from GTA VI's keyframe `own` and the moment's `light`; the frame's own exposure is held at nought, since every light it takes is exposed already.
pub fn frame_keyframe(own: &Keyframe, light: &FrameLight) -> Keyframe {
    let mut frame = Keyframe::default();
    for name in SAME {
        if let Some(value) = own.get(name) {
            frame.set(name, value);
        }
    }
    let get = |name: &str, default: f32| own.get_or(name, default);
    let rgb =
        |stem: &str, default: f32| ["r", "g", "b"].map(|c| get(&format!("{stem}_{c}"), default));
    let set_rgb = |frame: &mut Keyframe, stem: &str, colour: [f32; 3]| {
        for (c, value) in ["r", "g", "b"].iter().zip(colour) {
            frame.set(&format!("{stem}_{c}"), value);
        }
    };
    let times = |a: [f32; 3], b: [f32; 3]| [a[0] * b[0], a[1] * b[1], a[2] * b[2]];
    let scaled = |c: [f32; 3], by: f32| c.map(|c| c * by);
    // A colour and the intensity it is multiplied by in the frame: the colour at most one.
    let split = |c: [f32; 3]| {
        let most = c.iter().copied().fold(0.0, f32::max);
        match most > 0.0 {
            true => (c.map(|c| c / most), most),
            false => ([0.0; 3], 0.0),
        }
    };
    let lit = light.exposure * FRAME_DIFFUSE;

    // `stops` of the frame is `clamp(x, min(least, most), most)`: nought whatever its meter says.
    for name in [
        "postfx_exposure",
        "postfx_exposure_min",
        "postfx_exposure_max",
    ] {
        frame.set(name, 0.0);
    }
    // The tone curves are the visual settings', which the frame takes where no moment overrides them.
    frame.set("postfx_tonemap_filmic_override_dark", 0.0);
    frame.set("postfx_tonemap_filmic_override_bright", 0.0);

    let tint = times(rgb("light_dir_col", 1.0), light.body_transmittance);
    set_rgb(&mut frame, "light_dir_col", tint);
    let strength = get("light_dir_mult", 1.0) * light.fade * light.illuminance;
    frame.set("light_dir_mult", strength * lit);

    let sky_tint = times(
        rgb("sky_scattering_sky_col", 1.0),
        rgb("ambient_sky_color_mult", 1.0),
    );
    // ⚠ Inferred: the game's traced bounce light brings the sky's own irradiance and its sky ambient
    // adds `ambient_sky_intensity` of it again; the frame traces nothing, so it takes both.
    let sky = scaled(
        times(light.sky, sky_tint),
        1.0 + get("ambient_sky_intensity", 1.0),
    );
    let (colour, intensity) = split(scaled(sky, lit));
    set_rgb(&mut frame, "light_natural_amb_down_col", colour);
    frame.set("light_natural_amb_down_intensity", intensity);
    let direct = scaled(tint, strength);
    let bounce = GROUND_ALBEDO * get("ambient_bounce_intensity", 1.0) * lit;
    let ground = [0, 1, 2].map(|i| (direct[i] * light.towards_up + sky[i]) * bounce);
    let (colour, intensity) = split(ground);
    set_rgb(&mut frame, "light_natural_amb_up_col", colour);
    frame.set("light_natural_amb_up_intensity", intensity);
    frame.set("light_natural_amb_up_intensity_mult", 1.0);
    // ⚠ The frame marks no surface as an interior's, so the exterior's artificial light is the one every surface takes.
    for stem in ["ext_down", "ext_up"] {
        set_rgb(
            &mut frame,
            &format!("light_artificial_{stem}_col"),
            [1.0; 3],
        );
        let intensity = ARTIFICIAL_AMBIENT_LUX * lit;
        frame.set(&format!("light_artificial_{stem}_intensity"), intensity);
    }

    // The frame squares a sky colour, so the root of the colour is given.
    for (stem, radiance) in DOME.iter().zip(light.dome) {
        let (colour, intensity) = split(scaled(radiance, light.exposure));
        set_rgb(&mut frame, stem, colour.map(f32::sqrt));
        frame.set(&format!("{stem}_inten"), intensity);
    }
    frame.set("sky_hdr", 1.0);
    // The frame's dome runs from the horizon to the transition colour, whole, at its height, then to the zenith.
    // East and west, the dome's row at that height is its horizon blended towards the transition
    // colour: by how far the sky there lies along the way between them.
    frame.set("sky_zenith_transition_position", DOME_TRANSITION);
    for (side, horizon, name) in [
        (0, 2, "sky_zenith_transition_east_blend"),
        (1, 3, "sky_zenith_transition_west_blend"),
    ] {
        let (h, t, at) = (light.dome[horizon], light.dome[1], light.dome_sides[side]);
        let way: [f32; 3] = [0, 1, 2].map(|c| t[c] - h[c]);
        let length = way.iter().map(|w| w * w).sum::<f32>();
        let along = [0, 1, 2]
            .map(|c| (at[c] - h[c]) * way[c])
            .iter()
            .sum::<f32>();
        let blend = match length > 0.0 {
            true => (along / length).clamp(0.0, 1.0),
            false => 1.0,
        };
        frame.set(name, blend);
    }
    frame.set("sky_zenith_blend_start", 0.0);
    let sun = times(rgb("sky_scattering_sun_col", 1.0), light.sun_transmittance).map(f32::sqrt);
    set_rgb(&mut frame, "sky_sun_col", sun);
    // The disc is the sun's light times the disc's own multiplier, a radiance: it takes the exposure whole.
    let disc = get("sky_scattering_sun_disk_intensity_multiplier", 1.0) / FRAME_DIFFUSE;
    set_rgb(&mut frame, "sky_sun_disc_col", sun.map(|c| c * disc.sqrt()));
    frame.set("sky_sun_hdr", light.sun_illuminance * lit);
    for name in ["sky_moon_iten", "sky_stars_iten"] {
        if let Some(value) = own.get(name) {
            frame.set(name, value * lit);
        }
    }

    // ⚠ GTA V's exponential height fog is assumed to read GTA VI's local atmosphere's fog.
    if let Some(density) = own.get("local_atmosphere_fog_density") {
        frame.set("fog_density", density * FRAME_FOG_DENSITY);
    }
    if let Some(height) = own.get("local_atmosphere_fog_height") {
        frame.set("fog_base_height", height);
    }
    if let Some(falloff) = own.get("local_atmosphere_fog_falloff").filter(|f| *f > 0.0) {
        frame.set("fog_falloff", FRAME_FOG_FALLOFF / falloff);
    }
    // The haze is the air's own extinction, and what it fades to the sky's radiance at the horizon: a radiance, it takes the exposure whole.
    frame.set("fog_haze_density", light.haze * FRAME_FOG_DENSITY);
    for (stem, radiance) in FOG.iter().zip(light.fog) {
        set_rgb(&mut frame, stem, scaled(radiance, light.exposure));
    }
    for name in ["fog_hdr", "fog_haze_hdr", "sky_cloud_hdr"] {
        frame.set(name, 1.0);
    }
    frame
}

#[cfg(test)]
#[path = "timecycle_tests.rs"]
mod tests;
