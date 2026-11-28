//! An opened GTA V world: the mount it reads through, its capabilities, its policy with GTA V's
//! own rules, its look, and the ground and the collision of the mount's static bounds.

use std::path::PathBuf;

use crate::world_forest::streams_in;
use crate::{WorldCollision, WorldSource};
use glam::Vec3;
use or_io::gta5::data::water::CWaterData;
use or_title::MountedGame;
use or_world::caps::{Capability, WorldCaps};
use or_world::collision::{BoxOverlap, GroundProbe, Hit, OrientedBox};
use or_world::contract::{
    MapReader, ModelReader, Worker, WorldError, WorldIndexer, WorldInstall, WorldLook,
};
use or_world::kept::{Input, Inputs, Stamp};
use or_world::model::{Corner, Feed, FrameFacts, Gate, Index, Moment, SwitchId, WaterQuad};
use or_world::policy::StreamPolicy;
use or_world::sentence;
use or_world::slots::{Extras, SwitchKind, WeatherKind};
use or_world::streaming::Children;

use crate::body::{world_of, WORDS};
use crate::source::Source;

/// The bytes of bounds an ask about the ground or the collision holds let go of: each ask stands alone, so none is.
const BOUNDS_BUDGET: u64 = 0;

/// The level's own water file, as the mount names it.
const LEVEL_WATER: &str = "common:/data/levels/gta5/water.xml";
/// The weather a world opens under: a clear day's maps are in.
const FIRST_WEATHER: &str = "CLEAR";
/// The exposure of the plain day: the game's colour maps are made for a sun several times a surface's colour, under an exposure of its own.
const DAY_EXPOSURE: f32 = 1.6;
/// How far south of overhead the sun stands at noon, in radians: a level sun at noon lights no wall.
const NOON_TILT: f32 = 0.6;

/// The way towards the sun at `hour`: it rises in the east (+X), stands in the south at noon and sets in the west.
pub(crate) fn sun_at(hour: f32) -> Vec3 {
    let turn = (hour.rem_euclid(24.0) - 12.0) / 24.0 * std::f32::consts::TAU;
    let (lean, up) = NOON_TILT.sin_cos();
    Vec3::new(-turn.sin(), -turn.cos() * lean, turn.cos() * up)
}

/// The sheets of water `file` states that the game draws: one it leaves out or hides is none.
pub(crate) fn water_of(file: &CWaterData) -> Vec<WaterQuad> {
    let drawn = file.water_quads.iter().filter(|quad| !quad.is_invisible);
    drawn
        .filter_map(|quad| {
            let half = match quad.r#type {
                0 => None,
                1 => Some(Corner::SouthWest),
                2 => Some(Corner::NorthWest),
                5 => return None,
                odd if odd % 2 == 1 => Some(Corner::NorthEast),
                _ => Some(Corner::SouthEast),
            };
            let rectangle = quad.rectangle();
            Some(WaterQuad {
                min: rectangle.min,
                max: rectangle.max,
                height: quad.z,
                half,
            })
        })
        .collect()
}

/// A GTA V install, mounted.
pub struct Gta5World {
    pub(crate) source: Source,
    pub(crate) policy: StreamPolicy,
}

impl Gta5World {
    /// The world of `game`, which was mounted from the install at `root`.
    pub fn over(game: MountedGame, root: PathBuf) -> Self {
        Self {
            source: Source::new(game, root),
            // A parent waits for exactly the children it states, and a map streams in by its
            // box grown by the distance it is asked for ahead.
            policy: StreamPolicy {
                children: Children::Stated,
                streams_in,
                ..StreamPolicy::default()
            },
        }
    }

    /// The mount the world reads through.
    pub fn game(&self) -> &MountedGame {
        &self.source.game
    }

    /// The mount as the world index and `WorldForest` read it.
    pub fn source(&self) -> &dyn WorldSource {
        &self.source
    }

    /// Whether the file on disk that holds root archive `archive` could be asked when it changed.
    pub(crate) fn stamped(&self, archive: usize) -> bool {
        let root = self.source.game.roots.get(archive);
        root.is_some_and(|root| root.stamp.is_some())
    }
}

impl WorldLook for Gta5World {
    fn switches(&self, index: &Index) -> Vec<SwitchKind> {
        // One for each map that only a request lets in, by the map's name.
        let held = index.maps().iter().filter_map(|row| match row.gate {
            Gate::Held { switch } => switch,
            Gate::Always | Gate::When { .. } => None,
        });
        let mut switches: Vec<SwitchId> = held.collect();
        switches.sort_unstable();
        switches.dedup();
        let kind = |id| SwitchKind {
            id,
            label: sentence!("Group"),
            on: false,
        };
        switches.into_iter().map(kind).collect()
    }

    // The weathers a group of the game's maps may name, the clear day first.
    fn weathers(&self, _index: &Index) -> Vec<WeatherKind> {
        let kind = |name: &&str| WeatherKind {
            id: u64::from(or_io::string_hash(name.as_bytes())),
            name: name.to_string(),
        };
        let rest = (crate::state::WEATHER_NAMES.iter()).filter(|name| **name != FIRST_WEATHER);
        [FIRST_WEATHER].iter().chain(rest).map(kind).collect()
    }

    // ⚠ The plain day and no more: the sun in the south at noon and at the horizon at 6 h and 18 h.
    // What GTA V's own cycle states of a frame is its programs', which are fed by name.
    fn frame(&self, at: &Moment, _out: &mut dyn Feed) -> FrameFacts {
        let turn = (at.hour.rem_euclid(24.0) - 12.0) / 24.0 * std::f32::consts::TAU;
        let day = turn.cos().max(0.0);
        FrameFacts {
            sun: sun_at(at.hour),
            key: [day; 3],
            ambient: [0.05 + 0.25 * day; 3],
            exposure: DAY_EXPOSURE,
            clear: [0.1 * day, 0.2 * day, 0.4 * day],
        }
    }
}

impl WorldInstall for Gta5World {
    fn caps(&self) -> WorldCaps {
        WorldCaps::none()
            .with(Capability::Ground)
            .with(Capability::Collision)
            .with(Capability::Placements)
            .with(Capability::WeatherGate)
            .with(Capability::Water)
            .with_instance_words(WORDS)
    }

    fn policy(&self) -> StreamPolicy {
        self.policy
    }

    fn extras(&self) -> Extras {
        Extras::default()
    }

    fn inputs(&self) -> Inputs {
        let archives = (self.source.game.roots.iter()).map(|root| Input {
            name: format!("{}:{}", root.device, root.container),
            stamp: root.stamp.map(|stamp| Stamp {
                size: stamp.size,
                modified: stamp.modified,
            }),
        });
        Inputs {
            archives: archives.collect(),
            others: Vec::new(),
        }
    }

    fn indexer(&self) -> &dyn WorldIndexer {
        self
    }

    fn maps(&self) -> &dyn MapReader {
        self
    }

    fn models(&self) -> &dyn ModelReader {
        self
    }

    fn look(&self) -> &dyn WorldLook {
        self
    }

    // An install that holds no water file has no water, which is no fault of its world.
    fn water(&self, _index: &Index) -> Result<Vec<WaterQuad>, WorldError> {
        let Some(bytes) = self.source.game.vfs.open(LEVEL_WATER) else {
            return Ok(Vec::new());
        };
        let file = CWaterData::open(&bytes).map_err(|e| WorldError::Unread {
            what: LEVEL_WATER.to_string(),
            why: e.to_string(),
        })?;
        Ok(water_of(&file))
    }

    // ⚠ Each ask reads the bounds of its column afresh: nothing is held from one to the next.
    fn ground(
        &self,
        index: &Index,
        at: Vec3,
        probe: &GroundProbe,
        _worker: &mut Worker,
    ) -> Result<Option<Hit>, WorldError> {
        let mut bounds = WorldCollision::new(world_of(index)?, &self.source, BOUNDS_BUDGET);
        Ok(bounds.ground(at, probe).ground.map(|ground| ground.hit))
    }

    fn ray(
        &self,
        index: &Index,
        origin: Vec3,
        direction: Vec3,
        reach: f32,
        _worker: &mut Worker,
    ) -> Result<Option<Hit>, WorldError> {
        let mut bounds = WorldCollision::new(world_of(index)?, &self.source, BOUNDS_BUDGET);
        // As far from the upright line through the origin as the ray goes level.
        let level = direction.truncate().length() * reach.max(0.0);
        bounds.stream(origin.x, origin.y, level);
        let met = bounds.colliders().ray_cast(origin, direction, reach);
        Ok(met.map(|(_, hit)| hit))
    }

    fn overlap(
        &self,
        index: &Index,
        region: &OrientedBox,
        _worker: &mut Worker,
    ) -> Result<Vec<(u64, BoxOverlap)>, WorldError> {
        let mut bounds = WorldCollision::new(world_of(index)?, &self.source, BOUNDS_BUDGET);
        bounds.stream(
            region.centre.x,
            region.centre.y,
            region.half_extents.length(),
        );
        let held = bounds.colliders().overlap_box(region);
        let named = held.into_iter().filter_map(|(collider, overlap)| {
            Some((u64::from(bounds.bound_of(*collider)?.hash), overlap))
        });
        Ok(named.collect())
    }
}
