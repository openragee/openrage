//! An opened GTA V world: the mount it reads through, its capabilities, its policy with GTA V's
//! own rules, its look, and the ground and the collision of the mount's static bounds.

use std::path::PathBuf;

use glam::Vec3;
use or_depends::world_forest::streams_in;
use or_depends::{WorldCollision, WorldSource};
use or_title::MountedGame;
use or_world::caps::{Capability, WorldCaps};
use or_world::collision::{BoxOverlap, GroundProbe, Hit, OrientedBox};
use or_world::contract::{
    MapReader, ModelReader, Worker, WorldError, WorldIndexer, WorldInstall, WorldLook,
};
use or_world::kept::{Input, Inputs, Stamp};
use or_world::model::{Feed, FrameFacts, Gate, Index, Moment, SwitchId};
use or_world::policy::StreamPolicy;
use or_world::sentence;
use or_world::slots::{Extras, SwitchKind};
use or_world::streaming::Children;

use crate::body::{world_of, WORDS};
use crate::source::Source;

/// The bytes of bounds an ask about the ground or the collision holds let go of: each ask stands alone, so none is.
const BOUNDS_BUDGET: u64 = 0;

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

    /// The mount as the world index and the forest of `or_depends` read it.
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

    // ⚠ The plain day and no more: the sun overhead at noon and at the horizon at 6 h and 18 h.
    // What GTA V's own cycle states of a frame is its programs', which are fed by name.
    fn frame(&self, at: &Moment, _out: &mut dyn Feed) -> FrameFacts {
        let turn = (at.hour.rem_euclid(24.0) - 12.0) / 24.0 * std::f32::consts::TAU;
        let day = turn.cos().max(0.0);
        FrameFacts {
            sun: Vec3::new(turn.sin(), 0.0, turn.cos()),
            key: [day; 3],
            ambient: [0.05 + 0.25 * day; 3],
            exposure: 1.0,
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
