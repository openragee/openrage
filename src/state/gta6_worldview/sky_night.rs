//! GTA VI's night sky as its sky program sums it: the glow of the air and the stars of its star map,
//! with the maps read once out of the install's texture archive on a worker, where its sky
//! dictionary holds them by name.

use std::fs::File;
use std::sync::{Arc, Mutex};

use or_io::codec::texture::decode_level;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{read_dictionary, Entry};
use or_renderer::game_shaders::sky_view::{Night, SkyMap, SkyMaps};
use or_title::gta6_day::{DayTables, Moment};

use super::texture::Places;

/// The names of the star field and of the air's glow in the sky's dictionary.
const STARS: &str = "stars";
const AIRGLOW: &str = "airglow";
/// The most a dictionary body read for its maps inflates to.
const BODY_LIMIT: u64 = 1 << 28;
/// The build states the air's glow in thousandths of the sky's unit.
const AIR_GLOW_UNIT: f32 = 0.001;
/// How far away the game puts the moon, and how big one unit of its disc's size is there, in metres.
const SKY_DISTANCE: f32 = 20000.0;
const DISC_UNIT: f32 = 250.0;

static MAPS: Mutex<Option<Arc<SkyMaps>>> = Mutex::new(None);

/// The night sky's maps once read; `None` until then.
pub fn maps() -> Option<Arc<SkyMaps>> {
    MAPS.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Read the night sky's maps out of the texture archive `places` names, on a worker.
pub(super) fn read(places: Arc<Places>) {
    std::thread::spawn(move || {
        let read = || SkyMaps {
            stars: map(&places, STARS),
            airglow: map(&places, AIRGLOW),
        };
        let maps = super::pool::or_on_panic("the sky's maps", read, SkyMaps::default());
        tracing::info!(
            stars = ?maps.stars.as_ref().map(|m| (m.width, m.height, m.levels.len())),
            airglow = ?maps.airglow.as_ref().map(|m| (m.width, m.height, m.levels.len())),
            "the night sky's maps were read"
        );
        *MAPS.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(maps));
    });
}

/// The map named `name` in the texture archive's dictionaries, every level decoded; `None` where none holds it or it does not read.
fn map(places: &Places, name: &str) -> Option<SkyMap> {
    let key = name_hash64(name.as_bytes());
    let (archive, bodies) = &places.dictionaries;
    let offset = *bodies.get(&key)?;
    let file = File::open(archive).ok()?;
    let (find, bytes) = read_rpf9_body(&file, offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let dictionary = read_dictionary(virtual_section, Some(physical)).ok()?;
    let texture = dictionary
        .entries
        .iter()
        .find_map(|(_, entry)| match entry {
            Ok(Entry::Texture(t)) if name_hash64(t.name.as_bytes()) == key => {
                t.texture.as_ref().ok()
            }
            _ => None,
        })?;
    let texture = match texture.faces {
        1 => texture.clone(),
        _ => texture.face(0)?,
    };
    let top = decode_level(&texture, 0).ok()?;
    let (width, height, srgb) = (top.width, top.height, top.is_srgb);
    let mut levels = vec![top.data];
    for under in 1..texture.levels {
        let Ok(image) = decode_level(&texture, under) else {
            break;
        };
        let k = levels.len() as u32;
        if (image.width, image.height) != ((width >> k).max(1), (height >> k).max(1)) {
            break;
        }
        levels.push(image.data);
    }
    Some(SkyMap {
        width,
        height,
        srgb,
        levels,
    })
}

/// What the night adds to the sky at `moment` for a camera at `latitude` (radians): the air's glow by the moment's own
/// variables and the stars by the install's star settings, each weighed as the build's sky program is given it.
pub fn night_of(tables: &DayTables, moment: &Moment, latitude: f32) -> Night {
    let own = |name: &str, default: f32| moment.own.get_or(name, default);
    let stated = |name: &str, default: f32| tables.visual_settings.get_f32(name, default);
    let bodies = &moment.bodies;
    // ⚠ Inferred: the galaxy lights whole where neither body does; the build weighs it by how far each is past its threshold.
    let galaxy = f32::from(u8::from(bodies.sun_fade <= 0.0 && bodies.moon_fade <= 0.0));
    let boost = |name: &str| (galaxy * own(name, 1.0)).max(1.0);
    let air = |colour: &str, intensity: &str| {
        let scale = own(intensity, 0.0) * boost("sky_air_glow_galaxy_light_mult") * AIR_GLOW_UNIT;
        ["r", "g", "b"].map(|c| own(&format!("{colour}_{c}"), 0.0) * scale)
    };
    let mut fill = air("sky_air_glow_base_color", "sky_air_glow_base_intensity");
    if own("sky_stars_iten", 1.0) >= 1e-4 {
        let ambient =
            |name: &str| ["red", "green", "blue"].map(|c| stated(&format!("{name}.{c}"), 0.0));
        let (base, lit) = (
            ambient("stars.baseAmbient"),
            ambient("stars.baseAmbientGalaxy"),
        );
        fill = [0, 1, 2].map(|c| fill[c] + base[c] + (lit[c] - base[c]) * galaxy);
    }
    // Each kind of star: its colour times its strength, and how it blends towards the map's axis, with the executable's defaults.
    let kind = |name: &str, (dark, lit, axis): (f32, f32, f32)| {
        let stars = |what: &str| format!("sky_stars_{name}_{what}");
        let low = stated(&format!("stars.{name}.intensity"), dark);
        let high = stated(&format!("stars.{name}.intensityGalaxy"), lit);
        let strength = (low + (high - low) * galaxy)
            * boost(&stars("galaxy_light_mult"))
            * own(&stars("intensity_mult"), 1.0);
        let colour = ["red", "green", "blue"]
            .map(|c| stated(&format!("stars.{name}.color.{c}"), 1.0) * strength);
        [
            colour[0],
            colour[1],
            colour[2],
            stated(&format!("stars.{name}.axisBlend"), axis),
        ]
    };
    let disc = DISC_UNIT * own("sky_moon_disc_size", 2.5) / SKY_DISTANCE;
    Night {
        fill,
        glow: air("sky_air_glow_color", "sky_air_glow_intensity"),
        cloud: kind("cloud", (0.03, 0.03, 4.0)),
        dense: kind("dense", (7.5, 7.5, 4.0)),
        sparse: kind("sparse", (120.0, 120.0, 0.5)),
        scales: [
            stated("stars.dense.tiling", 4.0),
            stated("stars.sparse.tiling", 4.0),
            1.0 / (own("sky_stars_height_blend_mult", 1.0) * stated("stars.heightBlend", 4000.0)),
            1.0 / (own("sky_stars_blink_height_blend_mult", 1.0)
                * stated("stars.blinkHeightBlend", 24000.0)),
        ],
        // ⚠ Inferred: the blink's phase at nought; the build advances it with its clock.
        blink: [
            stated("stars.blinkGain.mult", 8.0),
            stated("stars.blinkGain.bias", 0.5),
            disc.atan().cos(),
            0.0,
        ],
        turn: star_turn(latitude, moment.sun),
        moon: moment.moon,
    }
}

/// The rows of the turn from the frame's axes (x east, y north, z up) at `latitude` to the star map's: its third axis
/// the celestial pole, turned about it with the sun's hour so that the map keeps its place against the sun.
// ⚠ Inferred: where the map's longitude starts; the build turns it from a start its own tables hold.
fn star_turn(latitude: f32, sun: [f32; 3]) -> [[f32; 3]; 3] {
    let (s, c) = latitude.sin_cos();
    // To the frame of the pole: x along the meridian's equator, y east, z the pole.
    let pole = [[0.0, -s, c], [1.0, 0.0, 0.0], [0.0, c, s]];
    let at = pole.map(|r| r[0] * sun[0] + r[1] * sun[1] + r[2] * sun[2]);
    let (sa, ca) = (-at[1].atan2(at[0])).sin_cos();
    let round = [[ca, -sa, 0.0], [sa, ca, 0.0], [0.0, 0.0, 1.0]];
    round.map(|r| [0, 1, 2].map(|j| r[0] * pole[0][j] + r[1] * pole[1][j] + r[2] * pole[2][j]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turned(turn: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
        turn.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
    }

    #[test]
    fn the_star_map_turns_about_the_pole_and_keeps_its_place_against_the_sun() {
        let latitude = 25.8f32.to_radians();
        let pole = [0.0, latitude.cos(), latitude.sin()];
        for hour in [0.0f32, 6.0, 13.0, 22.0] {
            let h = (hour / 24.0 - 0.5) * std::f32::consts::TAU;
            let sun = [
                -h.sin(),
                -latitude.sin() * h.cos(),
                latitude.cos() * h.cos(),
            ];
            let turn = star_turn(latitude, sun);
            let p = turned(&turn, pole);
            assert!((p[2] - 1.0).abs() < 1e-5, "{p:?}");
            let s = turned(&turn, sun);
            assert!(s[1].abs() < 1e-5 && s[0] > 0.9, "at {hour}: {s:?}");
            // A turn: each row of length one.
            for r in turn {
                assert!(((r[0] * r[0] + r[1] * r[1] + r[2] * r[2]) - 1.0).abs() < 1e-5);
            }
        }
    }
}
