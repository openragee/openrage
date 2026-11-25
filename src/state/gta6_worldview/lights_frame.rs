//! The lights the frame draws: each light's strength at the hour and at the camera's distance, in
//! the frame's units, and the brightest of them at the camera as the frame's point, spot and
//! capsule lights, each as the nested volumes that follow GTA VI's law (`lights_law`).

use glam::Vec3;
use or_io::gta5::data::visual_settings::VisualSettings;
use or_io::gta6::map::light_ext::{Light, LightKind};
use or_renderer::game_shaders::distant_lights::SpriteLook;
use or_renderer::world::light_math::{lamp_hour_factor, ATTR_DAY_BIT, ATTR_NIGHT_BIT};
use or_renderer::{CapsuleLight, DistantLight, PointLight, SpotLight};

use super::lights_law::{spot_cones, SHELLS, SHELL_EXPONENT};
use super::lights_lod::{Distant, MapLod};

/// The most lights the frame is handed: the placed lights a dense street holds within 150 m.
pub const MOST: usize = 2048;
/// Every hour of the day: a level-of-detail list states none.
const ALL_HOURS: u32 = 0x00FF_FFFF;
/// The falloff exponent every level-of-detail list states, which GTA VI's law does not read.
const LOD_FALLOFF: f32 = 1.0;

/// The volumes the frame draws the lights with, by the pass of each kind.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameLights {
    pub points: Vec<PointLight>,
    pub spots: Vec<SpotLight>,
    pub capsules: Vec<CapsuleLight>,
    /// How many lights they draw.
    pub lights: usize,
    /// Where each light drawn as volumes stands, and how much of it its reach leaves.
    pub kept: Vec<(Vec3, f32)>,
    /// The distant lights drawn as sprites, shared so that a frame that hands the same ones copies none, and how a sprite is drawn.
    pub sprites: std::sync::Arc<[DistantLight]>,
    pub sprite_look: SpriteLook,
    /// The coronas of the placed lights, and how the pass reads the depth about each.
    pub coronas: Option<(or_renderer::CoronaLook, Vec<or_renderer::Corona>)>,
    /// The lamps drawn, which light what blends, a lamp's own glass among it: the nearest of them count.
    pub lamps: Vec<or_renderer::WorldLamp>,
}
/// A level-of-detail light stands for a placed light this near it, in metres.
const SAME_PLACE: f32 = 0.02;

/// A light the frame may draw: a placed one, or one of a level-of-detail list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lamp {
    /// In the world.
    pub light: Light,
    /// For a level-of-detail light: how far from the camera it is drawn, and the last metres of that it fades out over.
    pub reach: Option<[f32; 2]>,
}

/// How far the level-of-detail lights of each size class (small, medium, large) are drawn, and the metres they fade out over, in metres.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LodReach {
    pub range: [f32; 3],
    pub fadeout: [f32; 3],
}

impl LodReach {
    /// As the install's visual settings state it; nought, which draws no such light, where they do not.
    pub fn of(settings: &VisualSettings) -> Self {
        let size = ["small", "medium", "large"];
        let get = |s: &str, what: &str| settings.get_f32(&format!("lodlight.{s}.{what}"), 0.0);
        Self {
            range: size.map(|s| get(s, "range")),
            fadeout: size.map(|s| get(s, "fadeout")),
        }
    }

    fn of_category(&self, category: u16) -> Option<[f32; 2]> {
        let c = usize::from(category);
        Some([*self.range.get(c)?, *self.fadeout.get(c)?])
    }
}

/// How far a light lit in the hours `hours` is on at `hour` (0 to 24), by GTA VI's night window: a lamp lit at midnight and not at noon is on from 18:00 to 06:00, one lit at noon and not at midnight the other way, one lit at both always, one lit at neither never.
// ⚠ The game picks the window by two bits and offsets it by two minute counts of its own light record, which no member of the file's light is known to carry: the hours the file states pick the bits here, and both counts are nought, so a lamp switches on the hour.
pub fn hour_strength(hours: u32, hour: f32) -> f32 {
    let lit = |h: u32| hours >> h & 1 == 1;
    let mode = match (lit(0), lit(12)) {
        (true, false) => ATTR_NIGHT_BIT,
        (false, true) => ATTR_DAY_BIT,
        (true, true) => 0,
        (false, false) => ATTR_NIGHT_BIT | ATTR_DAY_BIT,
    };
    lamp_hour_factor(mode, f64::from(hour.rem_euclid(24.0)), 0.0, 0.0)
}

fn reach_strength(reach: Option<[f32; 2]>, distance: f32) -> f32 {
    match reach {
        None => 1.0,
        Some([range, fade]) => ((range - distance) / fade.max(1e-3)).clamp(0.0, 1.0),
    }
}

/// The level-of-detail lights of `lod`, standing at the distant lights of `parent`, the map its `parent` names; none where the two lists are not as long.
pub fn lod_lamps(lod: &MapLod, parent: &MapLod, reach: &LodReach) -> Vec<Lamp> {
    let distant: &[Distant] = &parent.distant;
    let Some(reach) = reach.of_category(parent.category) else {
        return Vec::new();
    };
    if distant.len() != lod.falloff.len() || reach[0] <= 0.0 {
        return Vec::new();
    }
    let cone = |byte: u8| f32::from(byte) * 180.0 / 255.0;
    (distant.iter().enumerate())
        .filter_map(|(i, d)| {
            let kind = LightKind::of(d.light_type)?;
            let outer = lod.cone_outer[i];
            Some(Lamp {
                light: Light {
                    kind,
                    position: d.position,
                    direction: lod.direction[i],
                    colour: d.rgb.map(|c| f32::from(c) / 255.0),
                    intensity: d.intensity,
                    radius: lod.falloff[i],
                    falloff_exponent: LOD_FALLOFF,
                    hours: ALL_HOURS,
                    flags: 0,
                    cone_inner: cone(lod.cone_inner[i]),
                    cone_outer: cone(outer),
                    length: match kind {
                        LightKind::Capsule => f32::from(outer) * 140.0 / 255.0,
                        _ => 0.0,
                    },
                    corona: [0.0; 3],
                },
                reach: Some(reach),
            })
        })
        .collect()
}

/// Where the placed lights stand, to find the level-of-detail lights they stand in for.
#[derive(Debug, Default, PartialEq)]
pub struct Placed {
    at: std::collections::HashMap<[i64; 3], Vec<Vec3>>,
    /// The box of them, [`SAME_PLACE`] wider each way.
    low: Vec3,
    high: Vec3,
}

fn cell(p: Vec3) -> [i64; 3] {
    p.to_array().map(|x| (x / SAME_PLACE).floor() as i64)
}

impl Placed {
    pub fn of(placed: &[Lamp]) -> Self {
        let mut out = Self {
            low: Vec3::INFINITY,
            high: Vec3::NEG_INFINITY,
            ..Self::default()
        };
        for lamp in placed {
            let p = Vec3::from(lamp.light.position);
            out.at.entry(cell(p)).or_default().push(p);
            (out.low, out.high) = (out.low.min(p), out.high.max(p));
        }
        (out.low, out.high) = (out.low - SAME_PLACE, out.high + SAME_PLACE);
        // The same places in whatever order the maps gave them make an equal value
        for cell in out.at.values_mut() {
            cell.sort_unstable_by(|a, b| {
                a.to_array()
                    .partial_cmp(&b.to_array())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        out
    }

    /// Whether a placed light stands within [`SAME_PLACE`] of `p`.
    pub fn stands_at(&self, p: [f32; 3]) -> bool {
        let p = Vec3::from(p);
        // ⚠ The placed lights stand near the camera and the lists' reach for kilometres: most of a list is no placed light's.
        if p.cmplt(self.low).any() || p.cmpgt(self.high).any() {
            return false;
        }
        let [x, y, z] = cell(p);
        (-1..=1).any(|dx| {
            (-1..=1).any(|dy| {
                (-1..=1).any(|dz| {
                    (self.at.get(&[x + dx, y + dy, z + dz]).into_iter().flatten())
                        .any(|q| q.distance(p) <= SAME_PLACE)
                })
            })
        })
    }
}

/// `lod` without the lights a lamp of `placed` stands within [`SAME_PLACE`] of: the placed one is drawn instead.
#[cfg(test)]
pub fn without_placed(lod: Vec<Lamp>, placed: &[Lamp]) -> Vec<Lamp> {
    let near = Placed::of(placed);
    (lod.into_iter())
        .filter(|l| !near.stands_at(l.light.position))
        .collect()
}

/// A map's level-of-detail lamps, with the box they stand in and how far they are drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct LodList {
    pub lamps: Vec<Lamp>,
    low: Vec3,
    high: Vec3,
    range: f32,
}

impl LodList {
    pub fn of(lamps: Vec<Lamp>) -> Self {
        let (mut low, mut high) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        let mut range = 0.0f32;
        for lamp in &lamps {
            let p = Vec3::from(lamp.light.position);
            (low, high) = (low.min(p), high.max(p));
            range = range.max(lamp.reach.map_or(f32::INFINITY, |[range, _]| range));
        }
        Self {
            lamps,
            low,
            high,
            range,
        }
    }

    /// Whether a lamp of it may be drawn from `eye`: none past the reach of every one of them is.
    pub fn reaches(&self, eye: Vec3) -> bool {
        if self.lamps.is_empty() {
            return false;
        }
        // A position that is not a number makes no box: such a list is never passed over
        if !self.low.cmple(self.high).all() {
            return true;
        }
        // A metre to spare, so that a lamp on the box's face is never judged by a distance rounded differently.
        eye.distance(eye.clamp(self.low, self.high)) < self.range + 1.0
    }
}

/// The lamps lit at `hour` and within reach of `eye`, their intensity times `scale` (the frame's units for a stated intensity), the `most` nearest the camera by the edge of their reach, nearest first, as the volumes of the frame's passes.
#[cfg(test)]
pub fn frame_lights<'a>(
    lamps: impl IntoIterator<Item = &'a Lamp>,
    eye: Vec3,
    hour: f32,
    scale: f32,
    most: usize,
) -> FrameLights {
    let chosen = choose(lamps, eye, hour, most, &|_, _| true);
    volumes_of(&chosen, hour, scale)
}

/// A lamp the frame draws as volumes: the light, and how much of it its reach leaves.
pub type Chosen = (Light, f32);

/// The lamps lit at `hour`, within reach of `eye` and of which `seen` holds a sphere at their place as large as their reach, the `most` nearest the camera by the edge of their reach, nearest first.
pub fn choose<'a>(
    lamps: impl IntoIterator<Item = &'a Lamp>,
    eye: Vec3,
    hour: f32,
    most: usize,
    seen: &dyn Fn(Vec3, f32) -> bool,
) -> Vec<Chosen> {
    let mut lit: Vec<(f32, Chosen)> = (lamps.into_iter())
        .filter_map(|lamp| {
            let l = lamp.light;
            if !(l.lights_around() && l.radius > 0.0 && l.colour.iter().any(|c| *c > 0.0)) {
                return None;
            }
            let position = Vec3::from(l.position);
            let distance = position.distance(eye);
            let reach = reach_strength(lamp.reach, distance);
            if !(l.intensity * hour_strength(l.hours, hour) * reach > 0.0
                && seen(position, l.radius))
            {
                return None;
            }
            // The nearest by the edge of their reach are kept: a light past them is a distant light's sprite.
            Some((-(distance - l.radius).max(0.0), (l, reach)))
        })
        .collect();
    if lit.len() > most && most > 0 {
        lit.select_nth_unstable_by(most - 1, |a, b| b.0.total_cmp(&a.0));
    }
    lit.truncate(most);
    lit.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    lit.into_iter().map(|(_, chosen)| chosen).collect()
}

/// The cones a spot of half angles `inner` and `outer` is drawn as by a pass that holds a cone within 89°, each as which way it points (1 along the light, −1 against it), its outer and inner half angles and its share; a share of a cone of 180° is a point. A ramp across a cone wider than a right angle is a point less the ramp of the cone behind it, split at the right angle where the inner cone is narrower than one.
pub fn cones_within(inner: f32, outer: f32) -> Vec<(f32, f32, f32, f32)> {
    const RIGHT: f32 = 90.0;
    if outer <= MOST_CONE {
        return vec![(1.0, outer, inner, 1.0)];
    }
    let (cos_in, cos_out) = (inner.to_radians().cos(), outer.to_radians().cos());
    if inner >= RIGHT {
        // Lit all round but within the cone behind it, which fades the other way.
        return vec![
            (1.0, 180.0, 0.0, 1.0),
            (-1.0, 180.0 - inner, 180.0 - outer, -1.0),
        ];
    }
    // The ramp from the outer cone to the right angle, and from the right angle to the inner cone.
    let span = (cos_in - cos_out).max(1e-6);
    let (behind, ahead) = (-cos_out / span, cos_in / span);
    let mut out = vec![
        (1.0, 180.0, 0.0, behind),
        (-1.0, RIGHT, 180.0 - outer, -behind),
        (1.0, RIGHT, inner, ahead),
    ];
    out.retain(|piece| piece.3.abs() > 1e-4);
    out
}

/// The widest cone the spot pass draws, in degrees.
const MOST_CONE: f32 = 89.0;

/// `l` drawn as `kind` as a lamp that lights what blends, of `strength` (its colour times its intensity in the frame's units): a spot by its cones, an area light as one from a right angle in, a capsule all round from the nearest point of its segment, anything else all round from its place.
fn lamp_of(l: &Light, kind: LightKind, strength: Vec3, direction: Vec3) -> or_renderer::WorldLamp {
    let (inner, outer) = match kind {
        LightKind::Spot => (l.cone_inner, l.cone_outer),
        LightKind::Area => (0.0, 90.0),
        _ => (0.0, 180.0),
    };
    let (cos_in, cos_out) = (inner.to_radians().cos(), outer.to_radians().cos());
    let mut lamp = or_renderer::WorldLamp {
        position: Vec3::from(l.position),
        radius: l.radius,
        colour: strength,
        direction: Vec3::ZERO,
        cone: [cos_out, 1.0 / (cos_in - cos_out).max(1e-4)],
    };
    match kind {
        LightKind::Spot | LightKind::Area => lamp.direction = direction,
        LightKind::Capsule => lamp = lamp.capsule(direction * (0.5 * l.length)),
        LightKind::Point => {}
    }
    lamp
}

/// The volumes of `chosen` lit at `hour`, their intensity times `scale`.
pub fn volumes_of(chosen: &[Chosen], hour: f32, scale: f32) -> FrameLights {
    let mut out = FrameLights {
        lights: chosen.len(),
        kept: (chosen.iter())
            .map(|(l, reach)| (Vec3::from(l.position), *reach))
            .collect(),
        ..FrameLights::default()
    };
    for &(l, reach) in chosen {
        let intensity = l.intensity * scale * hour_strength(l.hours, hour) * reach;
        let (position, colour) = (Vec3::from(l.position), Vec3::from(l.colour));
        let direction = Vec3::from(l.direction).normalize_or_zero();
        // A spot or a capsule with no axis shines all round.
        let kind = match direction == Vec3::ZERO {
            true => LightKind::Point,
            false => l.kind,
        };
        let inner = l.cone_inner;
        out.lamps
            .push(lamp_of(&l, kind, colour * intensity, direction));
        let pieces: Vec<(f32, f32, f32, f32)> = match kind {
            LightKind::Spot => (spot_cones(inner, l.cone_outer).iter())
                .filter(|(_, part)| *part > 0.0)
                .flat_map(|&(o, part)| {
                    cones_within(inner, o)
                        .into_iter()
                        .map(move |(way, o, i, share)| (way, o, i, share * part))
                })
                .collect(),
            // An area light shines from one side, as the cosine of the angle from its axis: one straight ramp across a right angle.
            LightKind::Area => cones_within(0.0, 90.0),
            _ => Vec::new(),
        };
        // The law's strength is the stated intensity over d², so each volume's is over R².
        let per_area = intensity / (l.radius * l.radius);
        for (share, strength) in SHELLS {
            let (radius, intensity) = (l.radius * share.sqrt(), per_area * strength);
            let point = |intensity| PointLight {
                position,
                colour,
                intensity,
                radius,
                falloff: SHELL_EXPONENT,
            };
            match kind {
                LightKind::Spot | LightKind::Area => {
                    for &(way, o, i, part) in &pieces {
                        if o >= 180.0 {
                            out.points.push(point(intensity * part));
                            continue;
                        }
                        out.spots.push(SpotLight {
                            position,
                            direction: direction * way,
                            colour,
                            intensity: intensity * part,
                            radius,
                            falloff: SHELL_EXPONENT,
                            outer_degrees: o.min(MOST_CONE),
                            inner_degrees: i,
                        });
                    }
                }
                LightKind::Capsule => out.capsules.push(CapsuleLight {
                    position,
                    direction,
                    length: l.length,
                    colour,
                    intensity,
                    radius,
                    falloff: SHELL_EXPONENT,
                }),
                LightKind::Point => out.points.push(point(intensity)),
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "lights_frame_tests.rs"]
mod tests;
