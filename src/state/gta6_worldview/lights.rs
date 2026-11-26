//! The lights of the maps around the camera, read on workers: the lights an entity owns, those its
//! archetype holds, and the level-of-detail lights of the maps that list them, as the lights the
//! frame draws.
//!
//! ⚠ A level-of-detail light is a copy of a placed light: where the placed one is read, it is drawn
//! and the copy is not.

use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;

use std::f32::consts::PI;

use glam::Vec3;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::map::light_ext;
use or_io::gta6::map::lights::{self as effects, LightOwner, Pose};
use or_io::gta6::map::{self, MapBody};
use or_io::meta::binary::rbm::schema::RbmModel;
use or_renderer::game_shaders::distant_lights::SpriteLook;
use or_renderer::world::light_math::{in_day_cap, light_in_day_reach, DAY_RANGE_CAP};

use super::index::{Enabled, WorldIndex};
use super::interior::{self, RoomsSeen};
use super::lights_corona::{self, CoronaReach, View};
use super::lights_distant::{self, DistantReach};
use super::lights_frame::{self, Chosen, FrameLights, Lamp, LodList, LodReach, Placed};
use super::lights_lod::{self, MapLod};
pub use super::lights_vars::LightVars;
use super::pool::Pool;

pub use super::lights_frame::MOST;

/// The bytes a map body is read to.
const BODY_LIMIT: u64 = 256 << 20;
/// How far the camera moves before the wanted maps are worked out again, in metres.
const RELOOK: f32 = 8.0;
/// How far from the camera a map's entities may lie for their own lights to be read, in metres.
const PLACED_REACH: f32 = 150.0;
/// The bits of a map's content flags that say it lists level-of-detail lights, and distant ones.
const LOD_LIGHTS: u32 = 1 << 7;
const DISTANT_LIGHTS: u32 = 1 << 8;
const HIGH_DETAIL: u32 = 1;
/// What [`super::index::MapRow::held`] says of a map that places nothing.
const HELD_EMPTY: u32 = 3;

/// What was read of the lights around the camera, for a panel to say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LightsReport {
    pub maps_read: usize,
    /// Lights an entity owns or its archetype holds, in the world.
    pub placed: usize,
    /// Lights of the maps' level-of-detail light lists.
    pub lod: usize,
    pub distant: usize,
    /// Reflection probes read, which nothing draws.
    pub probes: usize,
    /// Lights handed to the frame last.
    pub drawn: usize,
}

/// The lights of one map.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct MapLights {
    /// The map its level-of-detail lights take their places from.
    parent: u32,
    /// The lights its entities own and their archetypes hold, in the world.
    placed: Vec<Lamp>,
    /// The lights of the contents of the interiors it places, each with its interior's place among
    /// its entities and its room.
    inside: Vec<(Lamp, [u32; 2])>,
    lod: MapLod,
}

/// The lights of the map body `model` holds, its archetypes' lights found in `index`; `None` for any other body.
fn lights_of(model: &RbmModel, index: &WorldIndex) -> Option<MapLights> {
    let MapBody::Data(data) = map::read(model).ok()? else {
        return None;
    };
    let mut out = MapLights {
        parent: data.parent,
        ..MapLights::default()
    };
    let lamp = |light| Lamp { light, reach: None };
    for entity in &data.entities {
        let held = index.archetype_lights(entity.archetype_word);
        if held.is_empty() {
            continue;
        }
        let pose = Pose::of(entity);
        (out.placed).extend(
            held.iter()
                .filter_map(|row| Some(lamp(row.light()?.placed(&pose)))),
        );
    }
    match effects::read(model) {
        Ok(read) => {
            for (owner, effect) in &read.effects {
                let LightOwner::Entity(at) = owner else {
                    continue;
                };
                let Some(entity) = data.entities.get(*at) else {
                    continue;
                };
                let pose = Pose::of(entity);
                (out.placed).extend(light_ext::lights_of(effect).map(|l| lamp(l.placed(&pose))));
            }
        }
        Err(error) => {
            tracing::debug!(map = data.name, %error, "a map's own lights are not read")
        }
    }
    match lights_lod::read(model) {
        Ok(lod) => out.lod = lod,
        Err(error) => tracing::debug!(map = data.name, %error, "a map's light lists are not read"),
    }
    Some(out)
}

/// Make `lists` the level-of-detail lamps of each map of `loaded` whose parent is loaded too, but for those a placed light of `near` stands in for, making again only those of a map or parent that came or went.
fn relist(
    loaded: &HashMap<u32, MapLights>,
    reach: &LodReach,
    lists: &mut HashMap<u32, Option<LodList>>,
    near: &Placed,
) {
    lists.retain(|name, list| {
        (loaded.get(name)).is_some_and(|m| list.is_some() == loaded.contains_key(&m.parent))
    });
    for (name, m) in loaded {
        lists.entry(*name).or_insert_with(|| {
            let parent = &loaded.get(&m.parent)?.lod;
            let mut lamps = lights_frame::lod_lamps(&m.lod, parent, reach);
            lamps.retain(|lamp| !near.stands_at(lamp.light.position));
            Some(LodList::of(lamps))
        });
    }
}

/// The placed lights of `loaded`, a map after another.
fn placed_of(loaded: &HashMap<u32, MapLights>) -> Vec<Lamp> {
    (loaded.values())
        .flat_map(|m| m.placed.iter().copied())
        .collect()
}

/// The lights of the interiors' contents of `loaded`, each with its map, its interior's place and its room.
fn inside_of(loaded: &HashMap<u32, MapLights>) -> Vec<(Lamp, [u32; 3])> {
    (loaded.iter())
        .flat_map(|(name, m)| {
            (m.inside.iter()).map(|(lamp, [at, room])| (*lamp, [*name, *at, *room]))
        })
        .collect()
}

/// `lamps` cut to the [`or_renderer::world::WORLD_LAMPS`] nearest `eye`, nearest first: a lamp's glass glows by its own small light at any reach of it, so the nearest by place, not by the edge of their reach, light what blends.
fn nearest_lamps(lamps: &mut Vec<or_renderer::WorldLamp>, eye: Vec3) {
    let most = or_renderer::world::WORLD_LAMPS;
    let away = |l: &or_renderer::WorldLamp| l.position.distance_squared(eye);
    if lamps.len() > most {
        lamps.select_nth_unstable_by(most - 1, |a, b| away(a).total_cmp(&away(b)));
        lamps.truncate(most);
    }
    lamps.sort_unstable_by(|a, b| away(a).total_cmp(&away(b)));
}

/// A word that changes with what `rooms` says is seen, whatever order it holds it in.
fn digest(rooms: &RoomsSeen) -> u32 {
    use std::hash::{Hash, Hasher};
    (rooms.iter()).fold(0u64, |sum, entry| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        entry.hash(&mut hasher);
        sum.wrapping_add(hasher.finish())
    }) as u32
}

/// The lamps the frame may draw seen from `eye`: the level-of-detail lights of each map in `lists` within reach, which leave out those a placed light stands in for, then the placed lights.
fn lamps_seen<'a>(
    loaded: &'a HashMap<u32, MapLights>,
    lists: &'a HashMap<u32, Option<LodList>>,
    placed: &'a [Lamp],
    eye: Vec3,
) -> impl Iterator<Item = &'a Lamp> {
    let reached = (loaded.keys())
        .filter_map(|name| lists.get(name)?.as_ref())
        .filter(move |list| list.reaches(eye));
    reached.flat_map(|list| &list.lamps).chain(placed)
}

/// Every lamp of `loaded` from anywhere, as [`lamps_seen`] gives them.
#[cfg(test)]
fn lamps_of(
    loaded: &HashMap<u32, MapLights>,
    reach: &LodReach,
    lists: &mut HashMap<u32, Option<LodList>>,
) -> Vec<Lamp> {
    let placed = placed_of(loaded);
    relist(loaded, reach, lists, &Placed::of(&placed));
    (loaded.keys())
        .filter_map(|name| lists.get(name)?.as_ref())
        .flat_map(|list| list.lamps.iter().copied())
        .chain(placed.iter().copied())
        .collect()
}

/// The lights of the maps around the camera, read on workers.
pub struct Lights {
    index: Arc<WorldIndex>,
    readers: Pool<u32, Option<MapLights>>,
    loaded: HashMap<u32, MapLights>,
    /// The maps the camera wants the lights of, by name, with their distance.
    wanted: HashMap<u32, f32>,
    looked_at: Option<Vec3>,
    drawn: usize,
    /// What of the world is switched on, as the streamer is told it.
    enabled: Enabled,
    reach: LodReach,
    distant_reach: DistantReach,
    /// How the placed lights' coronas are sized and lit; none draws none.
    corona_reach: Option<CoronaReach>,
    /// Each loaded map's level-of-detail lamps, None while its parent is not loaded.
    lists: HashMap<u32, Option<LodList>>,
    /// The placed lights of the maps loaded, and where they stand, the interiors' among them.
    placed: Vec<Lamp>,
    near: Placed,
    /// The lights of the interiors' contents of the maps loaded, each with its map, interior and room.
    inside: Vec<(Lamp, [u32; 3])>,
    /// The lamps last chosen to be drawn as volumes, with what they were chosen for; none once the lamps change.
    chosen: Option<Choice>,
    /// The distant lights' sprites as last worked out, and the eye, the day's cap and the frame's variables they were for; none once the lamps change.
    sprites: Option<(Vec3, bool, u64, Arc<[or_renderer::DistantLight]>)>,
}

/// The lamps chosen to be drawn as volumes, and the eye, the way it looked, the hour, the most, the rooms seen and the frame they were chosen for.
struct Choice {
    eye: Vec3,
    forward: Vec3,
    hour: u32,
    most: usize,
    rooms: u32,
    frame: u64,
    chosen: Vec<Chosen>,
}

/// What of the day's frame the lights take: its light variables, and its day phase (0.25 at sunrise, 0.5 at noon) where the day states one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LightFrame {
    pub vars: LightVars,
    pub day_phase: Option<f64>,
}

impl LightFrame {
    /// The day phase the lights' cap by day reads at `hour` (0 to 24): the frame's, else the clock's.
    // ponytail: the clock stands in where no day is read; it puts noon at 12:00 where the sun puts it later.
    fn phase(&self, hour: f32) -> f64 {
        (self.day_phase).unwrap_or_else(|| f64::from(hour.rem_euclid(24.0)) / 24.0)
    }

    fn bits(&self, hour: f32) -> u64 {
        self.vars.bits() ^ self.phase(hour).to_bits().rotate_left(17)
    }
}

/// How far the camera turns before the lamps are chosen again, in degrees: the cone they are chosen in is as much wider than the view.
const TURN: f32 = 5.0;
/// The widest screen the cone the lamps are chosen in allows for, width over height.
const WIDEST: f32 = 2.4;

impl Lights {
    pub fn new(index: Arc<WorldIndex>) -> Self {
        let shared = Arc::clone(&index);
        let readers = Pool::new(
            2,
            HashMap::<u32, File>::new,
            move |files, name: u32| -> Option<MapLights> {
                let row = shared.maps.get(shared.map_named(name)?)?;
                let file = match files.entry(row.archive) {
                    std::collections::hash_map::Entry::Occupied(open) => open.into_mut(),
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        let path = shared.archives.get(row.archive as usize)?;
                        slot.insert(File::open(path).ok()?)
                    }
                };
                let (find, bytes) = read_rpf9_body(&*file, row.offset, BODY_LIMIT).ok()?;
                let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                    resource: Some(resource),
                    ..
                }) = find.kind
                else {
                    return None;
                };
                let (virtual_section, physical) = resource.sections(&bytes);
                let model =
                    or_io::meta::binary::rbm::gta6::read_model(virtual_section, physical).ok()?;
                let mut read = lights_of(&model, &shared)?;
                // The lights of an interior's contents stand in the map that places it.
                let inside = super::interior::lights_placed(&shared, files, &model);
                read.inside = (inside.into_iter())
                    .map(|(light, at)| (Lamp { light, reach: None }, at))
                    .collect();
                Some(read)
            },
        );
        Self {
            index,
            readers,
            loaded: HashMap::new(),
            wanted: HashMap::new(),
            looked_at: None,
            drawn: 0,
            enabled: Enabled::default(),
            reach: LodReach::default(),
            distant_reach: DistantReach::default(),
            corona_reach: None,
            lists: HashMap::new(),
            placed: Vec::new(),
            near: Placed::default(),
            inside: Vec::new(),
            chosen: None,
            sprites: None,
        }
    }

    /// Say what of the world is switched on: the maps it holds back give no light.
    pub fn set_enabled(&mut self, enabled: &Enabled) {
        if self.enabled != *enabled {
            self.enabled = enabled.clone();
            // The wanted maps are worked out again at the next look.
            self.looked_at = None;
        }
    }

    /// Say how far the level-of-detail lights of each size class are drawn.
    pub fn set_reach(&mut self, reach: LodReach) {
        if self.reach != reach {
            self.reach = reach;
            self.lists.clear();
            relist(&self.loaded, &self.reach, &mut self.lists, &self.near);
            self.chosen = None;
        }
    }

    /// Say how far the distant lights of each size class are drawn as sprites, and how large.
    pub fn set_distant_reach(&mut self, reach: DistantReach) {
        if self.distant_reach != reach {
            self.distant_reach = reach;
            self.sprites = None;
        }
    }

    /// Say how the placed lights' coronas are sized and lit.
    pub fn set_corona_reach(&mut self, reach: CoronaReach) {
        self.corona_reach = Some(reach);
    }

    /// Ask for the lights of the maps within reach of `eye` and take in those read since; whether any came or went.
    pub fn look(&mut self, eye: Vec3) -> bool {
        let mut changed = false;
        for (name, read) in self.readers.take(|| true) {
            // A map that did not read holds no light, and is not asked for again while it is wanted.
            let read = read.unwrap_or_default();
            if self.wanted.contains_key(&name) {
                self.loaded.insert(name, read);
                changed = true;
            }
        }
        if changed || self.looked_at.is_none_or(|at| at.distance(eye) >= RELOOK) {
            self.looked_at = Some(eye);
            self.rewant(eye);
            let before = self.loaded.len();
            self.loaded.retain(|name, _| self.wanted.contains_key(name));
            changed |= self.loaded.len() != before;
            let asked = (self.wanted.iter())
                .filter(|(name, _)| !self.loaded.contains_key(name))
                .map(|(name, away)| (*away, *name))
                .collect();
            self.readers.want(asked);
        }
        if changed {
            self.relamp();
        }
        changed
    }

    /// Make the lamps of the maps loaded again.
    fn relamp(&mut self) {
        self.placed = placed_of(&self.loaded);
        self.inside = inside_of(&self.loaded);
        // A level-of-detail copy of an interior's lamp stays out whether its room is seen or not.
        let all: Vec<Lamp> = (self.placed.iter().copied())
            .chain(self.inside.iter().map(|(lamp, _)| *lamp))
            .collect();
        let near = Placed::of(&all);
        // ⚠ A list leaves out the lamps a placed light stands in for when it is made: where the placed lights stand changed, every list is made again.
        if near != self.near {
            self.lists.clear();
        }
        self.near = near;
        relist(&self.loaded, &self.reach, &mut self.lists, &self.near);
        self.chosen = None;
        self.sprites = None;
    }

    // ponytail: every map's box is tested each time the camera has moved, as the streamer does.
    fn rewant(&mut self, eye: Vec3) {
        self.wanted.clear();
        let modes = u32::from(self.enabled.modes);
        for row in self.index.maps.iter() {
            let lists = row.content_flags & (LOD_LIGHTS | DISTANT_LIGHTS) != 0;
            // A map of light lists places no entity, which the index counts as empty: its lights stream all the same.
            let empty_list = lists && row.held == HELD_EMPTY && row.modes & modes != 0;
            if !(self.enabled.allows(row) || empty_list) {
                continue;
            }
            let boxes = row.boxes();
            let (min, max) = boxes.entities;
            let away = eye.distance(eye.clamp(min, max));
            let lists = lists && boxes.wants(eye);
            let near = row.content_flags & HIGH_DETAIL != 0 && away <= PLACED_REACH;
            if lists || near {
                self.wanted.insert(row.name, away);
            }
        }
        // The map a loaded list takes its places from is wanted with it.
        let parents: Vec<(u32, f32)> = (self.loaded.iter())
            .filter(|(name, map)| map.parent != 0 && self.wanted.contains_key(name))
            .filter(|(_, map)| self.index.map_named(map.parent).is_some())
            .map(|(name, map)| (map.parent, self.wanted[name]))
            .collect();
        for (parent, away) in parents {
            self.wanted.entry(parent).or_insert(away);
        }
    }

    /// The lights lit at `hour` (0 to 24) the camera of `view` can see lit, nearest first and at most `most`, their stated intensity times `scale`, as the volumes the frame draws them with, the distant lights as sprites and the placed lights' coronas; an interior's lamps those of the rooms of `rooms` the camera sees.
    pub(super) fn lights(
        &mut self,
        view: &View,
        (hour, scale, most): (f32, f32, usize),
        rooms: &RoomsSeen,
        frame: &LightFrame,
    ) -> FrameLights {
        let turned = |was: Vec3| was.dot(view.forward) < TURN.to_radians().cos();
        let digest = digest(rooms);
        let bits = frame.bits(hour);
        let fresh = (self.chosen.as_ref()).is_some_and(|c| {
            c.eye == view.eye
                && c.hour == hour.to_bits()
                && c.most == most
                && c.rooms == digest
                && c.frame == bits
                && !turned(c.forward)
        });
        if !fresh {
            self.chosen = Some(self.choose(view, hour, most, rooms, frame));
        }
        let capped = in_day_cap(frame.phase(hour));
        let vars = frame.vars.bits();
        // ⚠ Worked out over every distant light of the maps loaded, which are many: only when the eye moves, the day's cap comes or goes, the frame's variables change or the lamps change.
        if (self.sprites.as_ref())
            .is_none_or(|(at, was, of, _)| *at != view.eye || *was != capped || *of != vars)
        {
            // A distant list's lights point the way the level-of-detail lights of the map that names it as parent do.
            let ways: HashMap<u32, &[[f32; 3]]> = (self.loaded.values())
                .filter(|m| m.parent != 0)
                .map(|m| (m.parent, m.lod.direction.as_slice()))
                .collect();
            let lists = (self.loaded.iter()).map(|(name, m)| {
                let way = ways.get(name).copied();
                (m.lod.distant.as_slice(), m.lod.category, way)
            });
            let mut sprites =
                lights_distant::sprites(lists, view.eye, &self.distant_reach, &frame.vars);
            if capped {
                sprites.retain(|s| s.position.distance(view.eye) <= DAY_RANGE_CAP);
            }
            self.sprites = Some((view.eye, capped, vars, sprites.into()));
        }
        let chosen = self
            .chosen
            .as_ref()
            .map_or(&[][..], |c| c.chosen.as_slice());
        let mut out = lights_frame::volumes_of(chosen, hour, scale);
        self.drawn = out.lights;
        nearest_lamps(&mut out.lamps, view.eye);
        // The frame's units for a radiance are the stated ones times the exposure, which `scale` holds over pi.
        let exposure = scale * PI;
        if let Some((_, _, _, sprites)) = &self.sprites {
            out.sprites = Arc::clone(sprites);
        }
        out.sprite_look = SpriteLook {
            exposure,
            ..self.distant_reach.look
        };
        out.coronas = (self.corona_reach.as_ref()).map(|reach| {
            let placed = self.placed.iter().chain(self.inside_lit(rooms));
            (
                reach.look,
                lights_corona::coronas(placed, view, hour, exposure, reach),
            )
        });
        out
    }

    /// The lamps the camera of `view` can see lit at `hour`, at most `most`: those whose reach lies within a cone about the view as wide as the widest screen and [`TURN`] more.
    // ⚠ Worked out over every lamp of the maps loaded, which are many: only when the eye moves, the hour changes or the camera turns past what the cone allows for.
    fn choose(
        &self,
        view: &View,
        hour: f32,
        most: usize,
        rooms: &RoomsSeen,
        frame: &LightFrame,
    ) -> Choice {
        let (eye, forward) = (view.eye, view.forward);
        let wide = (view.tan_half * WIDEST.hypot(1.0)).atan() + TURN.to_radians();
        let phase = frame.phase(hour);
        let seen = |p: Vec3, radius: f32| {
            if !light_in_day_reach(p, eye, phase) {
                return false;
            }
            let toward = p - eye;
            let d = toward.length();
            // Within a sphere about the axis of the cone that holds it, by the angle the sphere spans.
            d <= radius || toward.dot(forward) / d >= (wide + (radius / d).asin()).min(PI).cos()
        };
        let lamps =
            lamps_seen(&self.loaded, &self.lists, &self.placed, eye).chain(self.inside_lit(rooms));
        Choice {
            eye,
            forward,
            hour: hour.to_bits(),
            most,
            rooms: digest(rooms),
            frame: frame.bits(hour),
            chosen: lights_frame::choose(lamps, eye, hour, most, &frame.vars, &seen),
        }
    }

    /// The lamps of the interiors' contents whose room `rooms` says is seen.
    fn inside_lit<'a>(&'a self, rooms: &'a RoomsSeen) -> impl Iterator<Item = &'a Lamp> + 'a {
        (self.inside.iter())
            .filter(|(_, [map, at, room])| interior::lit(rooms, (*map, *at), *room))
            .map(|(lamp, _)| lamp)
    }

    /// The maps whose lights are wanted and not read yet.
    pub fn waiting(&self) -> usize {
        (self.wanted.keys())
            .filter(|name| !self.loaded.contains_key(name))
            .count()
    }

    pub fn report(&self) -> LightsReport {
        let sum = |of: fn(&MapLights) -> usize| -> usize { self.loaded.values().map(of).sum() };
        LightsReport {
            maps_read: self.loaded.len(),
            placed: sum(|m| m.placed.len() + m.inside.len()),
            lod: sum(|m| m.lod.falloff.len()),
            distant: sum(|m| m.lod.distant.len()),
            probes: sum(|m| m.lod.probes),
            drawn: self.drawn,
        }
    }
}

#[cfg(test)]
#[path = "lights_tests.rs"]
mod tests;
