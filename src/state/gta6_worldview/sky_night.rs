//! GTA VI's night sky as its sky program sums it: the glow of the air and the stars of its star map,
//! with the maps read once out of the install's texture archive on a worker, where its sky
//! dictionary holds them by name.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use glam::{Vec2, Vec3};
use or_io::codec::texture::decode_level;
use or_io::container::rpf::{read_rpf9_body, Gta6Mount, GtaViKeys, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::constellations;
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{read_dictionary, Entry};
use or_renderer::game_shaders::sky_stars::PointStars;
use or_renderer::game_shaders::sky_view::{Night, SkyMap, SkyMaps, AIR_SAMPLES};
use or_renderer::world::sky_dome::{
    self, DomeCall, PointStar, PointStarScales, SkyTunables, SkyVariables,
};
use or_title::gta6_day::{DayTables, Moment};
use or_title::gta6_mount::table::entry_offset;
use or_title::gta6_sky::Atmosphere;

use super::texture::Places;

/// The names of the star field, of the air's glow, of a point star's profile and of the moon's face in the sky's dictionary.
const STARS: &str = "stars";
const AIRGLOW: &str = "airglow";
const STAR: &str = "star";
const MOON: &str = "moon-new";
/// The archive and the document of the catalogue of point stars.
const CATALOGUE_ARCHIVE: &str = "data.rpf";
const CATALOGUE: &str = "data/effects/constellations";

/// The catalogue of each install asked for, read on a worker: `None` in the slot where it does not read.
type Catalogue = Arc<OnceLock<Option<Arc<PointStars>>>>;
static CATALOGUES: Mutex<Vec<(PathBuf, Catalogue)>> = Mutex::new(Vec::new());
/// The most a dictionary body read for its maps inflates to.
const BODY_LIMIT: u64 = 1 << 28;
/// The build states the air's glow in thousandths of the sky's unit.
const AIR_GLOW_UNIT: f32 = 0.001;

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
            star: map(&places, STAR),
            moon: map(&places, MOON),
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

/// The point stars of the install at `root`, read once on a worker and held: `None` until read, and where they do not read.
pub fn stars(root: &Path) -> Option<Arc<PointStars>> {
    let mut held = CATALOGUES.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, slot)) = held.iter().find(|(path, _)| path == root) {
        return slot.get()?.clone();
    }
    let slot = Arc::<OnceLock<Option<Arc<PointStars>>>>::default();
    held.push((root.to_path_buf(), Arc::clone(&slot)));
    let (root, keys) = (root.to_path_buf(), super::index::keys());
    std::thread::spawn(move || {
        let read = || match &keys {
            Some(keys) => read_stars(&root, keys),
            None => Err("no key snapshot deciphers the install's tables".to_string()),
        };
        let read = super::pool::or_on_panic("the star catalogue", read, Err("it stopped".into()));
        let stars = read
            .inspect_err(|why| tracing::warn!(%why, "the point stars are not read"))
            .ok();
        let _ = slot.set(stars.map(Arc::new));
    });
    None
}

// ⚠ The catalogue is ciphered: the table names it, and only the deciphering reader opens it.
fn read_stars(root: &Path, keys: &GtaViKeys) -> Result<PointStars, String> {
    let archive = root.join(CATALOGUE_ARCHIVE);
    let mut file = File::open(&archive).map_err(|e| e.to_string())?;
    let mount = Gta6Mount::open_root(&mut file, keys).map_err(|e| e.to_string())?;
    let at = entry_offset(&archive, CATALOGUE, keys).ok_or("the table names no star catalogue")?;
    let entry = (mount.archive.entries.iter())
        .find(|e| mount.absolute_start(e) == Some(at))
        .ok_or_else(|| format!("no entry at {at}"))?;
    let body = (mount.read_entry_decrypted(&mut file, entry, keys)).map_err(|e| e.to_string())?;
    // Each star at the frame's scales of one: the frame's own go to the pass.
    let unit = PointStarScales {
        size: 1.0,
        colour: 1.0,
    };
    let stars: Vec<[f32; 8]> = (constellations::read(&body)
        .map_err(|e| e.to_string())?
        .iter())
    .filter_map(|s| {
        let star = PointStar {
            // The colour stays in the bytes' steps it is stated in: the install's star intensity is scaled for them.
            colour: Vec3::from(s.colour),
            polar: s.position[0],
            azimuth: s.position[1],
            size: s.scale,
            intensity: s.intensity,
        };
        let (at, colour) = sky_dome::point_star_vertex(&star, &unit)?;
        Some([at.x, at.y, at.z, at.w, colour.x, colour.y, colour.z, 0.0])
    })
    .collect();
    tracing::info!(stars = stars.len(), "the point stars were read");
    Ok(PointStars { stars })
}

/// The dome's variables from the moment's own keyframe by name, each with the variable table's default.
fn sky_variables(own: &dyn Fn(&str, f32) -> f32) -> SkyVariables {
    let rgb = |stem: &str, [r, g, b]: [f32; 3]| {
        Vec3::new(
            own(&format!("{stem}_r"), r),
            own(&format!("{stem}_g"), g),
            own(&format!("{stem}_b"), b),
        )
    };
    let kinds = |what: &str| {
        ["cloud", "dense", "sparse"].map(|kind| own(&format!("sky_stars_{kind}_{what}"), 1.0))
    };
    SkyVariables {
        sun_disc_gate: own("sky_scattering_sun_intensity_multiplier", 1.0),
        moon_disc_gate: own("sky_scattering_moon_intensity_multiplier", 1.0),
        row_fc: own("sky_scattering_fill_intensity_multiplier", 0.0),
        atmosphere_a_colour: rgb("sky_air_glow_color", [0.184, 1.0, 0.494]),
        atmosphere_a_scale: own("sky_air_glow_intensity", 0.2),
        atmosphere_b_colour: rgb("sky_air_glow_base_color", [0.072, 0.35, 1.0]),
        atmosphere_b_scale: own("sky_air_glow_base_intensity", 0.1),
        atmosphere_boost: own("sky_air_glow_galaxy_light_mult", 1.0),
        sun_size: own("sky_sun_disc_size", 2.5),
        row_194: own("sky_scattering_sun_disk_intensity_multiplier", 1.0),
        moon_colour: rgb("sky_moon_col", [1.0; 3]),
        moon_size: own("sky_moon_disc_size", 2.5),
        row_164: own("sky_moon_iten", 1.0),
        stars_on: own("sky_stars_iten", 1.0),
        point_star_strength: own("sky_stars_constellations_intensity_mult", 1.0),
        point_star_size: own("sky_stars_constellations_size_mult", 1.0),
        layer_strength: kinds("intensity_mult"),
        twinkle_rate: own("sky_stars_frequency_mult", 1.0),
        row_ec: own("sky_stars_height_blend_mult", 1.0),
        row_10c: own("sky_stars_blink_height_blend_mult", 1.0),
        point_star_strength_boost: own("sky_stars_constellations_galaxy_light_mult", 1.0),
        point_star_size_boost: own("sky_stars_constellations_size_galaxy_light_mult", 1.0),
        layer_boost: kinds("galaxy_light_mult"),
    }
}

/// What the night adds to the sky at `moment` for a camera at `latitude` (radians): the air's glow by the moment's own
/// variables and the stars by the install's star settings, each weighed as the build's sky program is given it.
pub fn night_of(tables: &DayTables, moment: &Moment, latitude: f32) -> Night {
    let own = |name: &str, default: f32| moment.own.get_or(name, default);
    let stated = |name: &str, default: f32| tables.visual_settings.get_f32(name, default);
    let galaxy = moment.bodies.third_fade;
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
    let variables = sky_variables(&own);
    let tunables = SkyTunables {
        twinkle_scale: stated("stars.frequency", 2.0),
        row_11c: stated("stars.constellationStarPower", 1.5),
        point_star_size: stated("stars.constellations.size", 1.0),
        point_star_night_strength: Vec2::new(
            stated("stars.constellations.intensity", 1.0),
            stated("stars.constellations.intensityGalaxy", 1.0),
        ),
        ..SkyTunables::image_defaults()
    };
    let call = DomeCall::from_words(0, sky_dome::MAIN_VIEW_FLAGS);
    let scales = sky_dome::point_star_scales(&variables, &tunables, galaxy);
    let drawn = sky_dome::point_stars_drawn(&variables, call, 1);
    // The stars blink by the game's clock in milliseconds, as the build's counter counts them.
    let ms = (or_transport::clock::game_seconds() * 1000.0) as u32;
    let (sun, moon) = (Vec3::from(moment.sun), Vec3::from(moment.moon));
    let block = sky_dome::fill_dome_block(
        &variables,
        &tunables,
        &sky_dome::Bodies {
            sun,
            moon,
            night_weight: galaxy,
        },
        &sky_dome::DomeView {
            camera: Vec3::ZERO,
            ground_height: 0.0,
            sky: sky_dome::SkyMatrix {
                rotation_rows: [glam::Vec4::X, glam::Vec4::Y, glam::Vec4::Z, glam::Vec4::W],
                fourth_row: glam::Vec4::W,
            },
            texture_width: None,
        },
        call,
        ms,
    );
    let atmosphere = Atmosphere::of(&moment.own);
    let through = std::array::from_fn(|i| {
        let up = (i as f32 / (AIR_SAMPLES - 1) as f32).powi(2);
        let [r, g, b] = atmosphere.transmittance(up);
        [r, g, b, 0.0]
    });
    let moon_disc = sky_dome::moon_disc(&variables, moon, call).map_or(0.0, |d| d.radius);
    let moon_colour = block.moon_colour * block.row_164;
    Night {
        air: through,
        // ⚠ The sun's disc is left to the older pass: its colour is a member of a block not read.
        sun_disc: sun.extend(0.0).to_array(),
        sun_colour: [block.row_194, block.row_194, block.row_194, block.row_190],
        moon_disc: block.moon_lit_direction.extend(moon_disc).to_array(),
        moon_colour: moon_colour.extend(0.0).to_array(),
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
        blink: [
            stated("stars.blinkGain.mult", 8.0),
            stated("stars.blinkGain.bias", 0.5),
            sky_dome::disc_angular_radius(variables.moon_size).cos(),
            sky_dome::twinkle_phase(variables.twinkle_rate, tunables.twinkle_scale, ms),
        ],
        turn: star_turn(latitude, moment.sun),
        moon: moment.moon,
        point: [
            scales.size,
            scales.colour,
            tunables.row_11c,
            f32::from(u8::from(drawn)),
        ],
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
