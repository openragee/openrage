//! The static collision of a mounted GTA V install: the bounds files near a place, read from the
//! mount as a question asks for them, held as colliders under a budget of bytes and asked as one.
//!
//! ⚠ Only a bound a map group or a map's name carries is read, and it is placed as its file states
//! it, because such a file is in world space. The collision of an interior and of a placed prop is
//! in the space of the entity that places it, which nothing here reads: a caller that knows the
//! placement hands the collider in with [`WorldCollision::place`].
//!
//! ⚠ A file is found by the box its header states. One that states none is asked wherever a map
//! that carries it streams, so that a file that does not read is named and not passed over. An
//! interior's file states its box about the interior, so it is named by the maps that place the
//! interior and never by that box.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Affine3A, Vec3};
use or_io::decl::File as _;
use or_io::gta5::PhBoundAny;
use or_world::collision::{BoundCollider, CollisionError, CollisionSet, GroundProbe, Hit};

use crate::archetype_assets::Via;
use crate::collision::OfBound;
use crate::world_index::{Bound, BoundAt, Carrier, WorldIndex, WorldSource};

/// How far past the highest and the lowest collider of a column a probe is let reach, in metres,
/// so that it neither starts nor ends on a surface.
const MARGIN: f32 = 1.0;

/// What a collider of the set is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Collider {
    /// A bounds file of the mount, by its place in [`WorldIndex::bounds`].
    File(u32),
    /// A collider its caller placed, under the caller's own number.
    Placed(u64),
}

impl std::fmt::Display for Collider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(at) => write!(f, "bounds file {at} of the mount"),
            Self::Placed(key) => write!(f, "placed collider {key}"),
        }
    }
}

/// Why a bounds file gave no collider.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Unread {
    #[error("the archive that lists it gave no file")]
    Absent,
    #[error("it does not read as a bound: {0}")]
    NotABound(String),
    #[error("it is no collider: {0}")]
    NotACollider(#[from] CollisionError),
}

/// What the bounds files near a column came to, each list in the mount's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Streamed<'a> {
    /// The files held as colliders, which a question is asked of.
    pub held: Vec<&'a Bound>,
    /// The files that gave no collider, each with why.
    pub unread: Vec<(&'a Bound, Unread)>,
    /// The files of the interiors that the maps streaming there place: the interior's placement puts them, so they are not read.
    pub unplaced: Vec<&'a Bound>,
}

/// The ground a question met, and what it is the surface of.
#[derive(Debug, Clone, PartialEq)]
pub struct Ground<'a> {
    pub hit: Hit,
    pub collider: Collider,
    /// The bounds file, for a collider that is one.
    pub bound: Option<&'a Bound>,
}

/// The answer to a question of the ground: the ground where there is one, and the files of its column.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundAnswer<'a> {
    pub ground: Option<Ground<'a>>,
    pub streamed: Streamed<'a>,
}

/// The collider of a bounds file of the mount, read from the archive that lists it.
pub fn collider_of(source: &dyn WorldSource, bound: &Bound) -> Result<BoundCollider, Unread> {
    let bytes = source
        .open(bound.archive, &bound.logical)
        .ok_or(Unread::Absent)?;
    let file = PhBoundAny::from_bytes(&bytes).map_err(|e| Unread::NotABound(e.to_string()))?;
    Ok(BoundCollider::new(&file)?)
}

#[derive(Debug, Clone, Copy)]
struct Held {
    bytes: u64,
    /// The last question that asked for the file, by its number.
    used: u64,
}

/// The colliders of a mount's static bounds near the places asked about, and what a caller placed.
pub struct WorldCollision<'a> {
    index: &'a WorldIndex,
    source: &'a dyn WorldSource,
    /// Bytes of colliders read from the mount; what a caller placed is the caller's and is not counted.
    budget: u64,
    set: CollisionSet<Collider>,
    held: HashMap<u32, Held>,
    held_bytes: u64,
    /// ⚠ Kept for the mount's life, so a file that does not read is opened once and not at every question.
    refused: HashMap<u32, Unread>,
    asked: u64,
}

impl<'a> WorldCollision<'a> {
    /// No collider yet, over `index` and the mount it was built from, holding at most `budget` bytes of what it reads.
    pub fn new(index: &'a WorldIndex, source: &'a dyn WorldSource, budget: u64) -> Self {
        Self {
            index,
            source,
            budget,
            set: CollisionSet::new(),
            held: HashMap::new(),
            held_bytes: 0,
            refused: HashMap::new(),
            asked: 0,
        }
    }

    /// Reads the static bounds whose box comes within `radius` metres of the upright line through `x` and `y`, and says what each came to.
    pub fn stream(&mut self, x: f32, y: f32, radius: f32) -> Streamed<'a> {
        self.asked += 1;
        let index = self.index;
        let mut out = Streamed::default();
        let mut wanted: Vec<(u32, &'a Bound)> = Vec::new();
        let by_box = index.bounds_near_column(x, y, radius);
        let boxless = index.boxless_bounds_near_column(x, y, radius);
        let of_an_interior = |found: &BoundAt<'_>| {
            let interior = |carrier: &Carrier| matches!(carrier.via, Via::ManifestInterior);
            found.carriers.iter().all(interior)
        };
        // ⚠ Neither list holds a file an interior alone carries: the box such a file states is
        // about the interior, and the index answers it at no position.
        for found in by_box.into_iter().chain(boxless) {
            let bound = found.bound;
            let Some(at) = index.place_of_bound(bound.hash) else {
                continue;
            };
            // ⚠ Before any file is read: a file of this question that is held must not be the
            // one dropped to make room for another of the same question.
            if let Some(held) = self.held.get_mut(&at) {
                held.used = self.asked;
            }
            wanted.push((at, bound));
        }
        wanted.sort_by_key(|(at, _)| *at);
        for (at, bound) in wanted {
            if self.held.contains_key(&at) {
                out.held.push(bound);
                continue;
            }
            if let Some(why) = self.refused.get(&at) {
                out.unread.push((bound, why.clone()));
                continue;
            }
            match self.read(at, bound) {
                Ok(()) => out.held.push(bound),
                Err(why) => {
                    tracing::warn!(file = %bound.logical, %why, "a bounds file gave no collider");
                    self.refused.insert(at, why.clone());
                    out.unread.push((bound, why));
                }
            }
        }
        let placed = index.interior_bounds_near_column(x, y, radius);
        let unplaced = placed.into_iter().filter(of_an_interior);
        out.unplaced = unplaced.map(|found| found.bound).collect();
        out
    }

    /// Reads one file into the set, then drops what the budget no longer holds.
    fn read(&mut self, at: u32, bound: &Bound) -> Result<(), Unread> {
        let collider = collider_of(self.source, bound)?;
        let bytes = collider.size_in_bytes() as u64;
        // A file of static map collision is in world space, so it is placed where it states.
        self.set
            .add(Collider::File(at), Arc::new(collider), Affine3A::IDENTITY)?;
        self.held.insert(
            at,
            Held {
                bytes,
                used: self.asked,
            },
        );
        self.held_bytes += bytes;
        self.shed();
        Ok(())
    }

    /// Drops the file asked for longest ago until the budget holds.
    ///
    /// ⚠ Never a file the question being answered asked for: one question whose files pass the
    /// budget keeps them all until the next.
    fn shed(&mut self) {
        while self.held_bytes > self.budget {
            let oldest = (self.held.iter())
                .filter(|(_, held)| held.used < self.asked)
                .min_by_key(|(at, held)| (held.used, **at))
                .map(|(at, _)| *at);
            let Some(at) = oldest else { break };
            if let Some(held) = self.held.remove(&at) {
                self.held_bytes -= held.bytes;
            }
            self.set.remove(&Collider::File(at));
        }
    }

    /// Every collider held, to ask a ray, a swept sphere or a box of.
    pub fn colliders(&self) -> &CollisionSet<Collider> {
        &self.set
    }

    /// The bounds file a collider is, for one that is a file.
    pub fn bound_of(&self, collider: Collider) -> Option<&'a Bound> {
        match collider {
            Collider::File(at) => self.index.bounds().get(at as usize),
            Collider::Placed(_) => None,
        }
    }

    /// How many bounds files are held as colliders.
    pub fn held_files(&self) -> usize {
        self.held.len()
    }

    /// The bytes of the colliders read from the mount.
    pub fn held_bytes(&self) -> u64 {
        self.held_bytes
    }

    /// Places a collider the caller holds under `key`, in place of any already there: an interior's or a prop's, by its entity's rotation, one scale and translation.
    // ⚠ `to_world` takes the collider's own space to the world. A map stores an entity's rotation
    // conjugated and an interior instance's as it is: `crate::Placement::rotation_of` reads either.
    pub fn place(
        &mut self,
        key: u64,
        collider: Arc<BoundCollider>,
        to_world: Affine3A,
    ) -> Result<(), CollisionError> {
        self.set.add(Collider::Placed(key), collider, to_world)
    }

    /// Takes out what was placed under `key`; false where there was nothing.
    pub fn displace(&mut self, key: u64) -> bool {
        self.set.remove(&Collider::Placed(key))
    }

    /// The ground under `point`, over the bounds files of its column and what was placed.
    pub fn ground(&mut self, point: Vec3, probe: &GroundProbe) -> GroundAnswer<'a> {
        let streamed = self.stream(point.x, point.y, 0.0);
        let ground = self.probed(point, probe);
        GroundAnswer { ground, streamed }
    }

    /// The highest ground of the column through `x` and `y`, no steeper than `max_slope` radians from level.
    pub fn highest_ground(&mut self, x: f32, y: f32, max_slope: f32) -> GroundAnswer<'a> {
        let streamed = self.stream(x, y, 0.0);
        let ground = self.column_span(x, y).and_then(|(_, top)| {
            let probe = GroundProbe {
                max_rise: 0.0,
                max_drop: f32::INFINITY,
                max_slope,
            };
            self.probed(Vec3::new(x, y, top + MARGIN), &probe)
        });
        GroundAnswer { ground, streamed }
    }

    /// The lowest and the highest that a held collider's box reaches in the column through `x` and `y`.
    fn column_span(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let mut span: Option<(f32, f32)> = None;
        for (_, held) in self.set.world_boxes() {
            let over = held.min.x <= x && x <= held.max.x && held.min.y <= y && y <= held.max.y;
            if held.is_empty() || !over {
                continue;
            }
            let (bottom, top) = span.unwrap_or((held.min.z, held.max.z));
            span = Some((bottom.min(held.min.z), top.max(held.max.z)));
        }
        span
    }

    fn probed(&self, point: Vec3, probe: &GroundProbe) -> Option<Ground<'a>> {
        let (bottom, top) = self.column_span(point.x, point.y)?;
        // ⚠ Not `min`, which passes over a NaN: a reach that is no length stays one, and is refused.
        let cut = |length: f32, to: f32| if length > to { to } else { length };
        // A reach past what the column holds is cut to it, so a probe may be given no end.
        let probe = GroundProbe {
            max_rise: cut(probe.max_rise, (top - point.z).max(0.0) + MARGIN),
            max_drop: cut(probe.max_drop, (point.z - bottom).max(0.0) + MARGIN),
            max_slope: probe.max_slope,
        };
        let (collider, hit) = self.set.ground(point, &probe)?;
        Some(Ground {
            hit,
            collider: *collider,
            bound: self.bound_of(*collider),
        })
    }
}
