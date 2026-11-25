//! The arc a swinging leaf sweeps about its hinge, and the first thing in it that stops the leaf.
//!
//! ⚠ **What the closed leaf already touches never blocks it.** A leaf stands in a frame, on a
//! floor and inside the box of its building, so the pieces of a bound and the boxes of other
//! entities that the closed leaf touches are what it stands in, and only what it meets after it
//! has begun to turn is in its way. A thing that overlaps the closed leaf is therefore not
//! reported here.
//!
//! ⚠ The box that is swept is the leaf's box cut back by [`ARC_SKIN`], [`ARC_SILL`] and
//! [`ARC_HINGE_STRIP`]. The three are judgements, measured on no install.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Vec2, Vec3};

use super::find::EntityAt;
use super::leaf::{Hinge, HingeSide, Leaf, WidthAxis};
use crate::collision::{CollisionSet, OrientedBox, Part};

/// The widest turn between two tests of a sweep, in radians.
pub const ARC_STEP: f32 = std::f32::consts::PI / 180.0;

/// The farthest the leaf's free edge moves between two tests, in metres, so nothing thicker is passed over.
pub const ARC_TRAVEL: f32 = 0.02;

/// The most tests one sweep makes before it is found blocked; past them the free edge moves further than [`ARC_TRAVEL`] between two.
pub const ARC_MOST_STEPS: usize = 4096;

/// How far every face of the swept box is drawn in, in metres, so a leaf that rests against a thing is not held by it.
pub const ARC_SKIN: f32 = 0.01;

/// How far the bottom of the swept box is lifted, in metres, so the floor a leaf's box dips into does not hold it.
pub const ARC_SILL: f32 = 0.1;

/// How much of the leaf next to its hinge is left out of the sweep, in metres: that strip turns inside the wall the leaf hangs in.
pub const ARC_HINGE_STRIP: f32 = 0.2;

/// How many times the stretch between the last free test and the first blocked one is halved.
const HALVINGS: usize = 16;

/// Which way a leaf turns about its hinge, seen from above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    Anticlockwise,
    Clockwise,
}

impl Way {
    fn sign(self) -> f32 {
        match self {
            Way::Anticlockwise => 1.0,
            Way::Clockwise => -1.0,
        }
    }
}

/// Why no arc is swept.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum ArcError {
    #[error("the leaf does not swing: its box gives it no hinge, so it sweeps no arc")]
    NoHinge,
    #[error("an arc of {0} radians is not swept: the angle is above nought and no more than a half turn")]
    Angle(f32),
}

/// The volume a swinging leaf sweeps when it opens to an angle each way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorArc {
    /// A point of the hinge line.
    pub hinge: Vec3,
    /// The hinge line's unit direction.
    pub axis: Vec3,
    /// The unit direction from the hinge to the free edge of the closed leaf.
    pub closed: Vec3,
    /// How far the farthest corner of the leaf is from the hinge line, in metres.
    pub radius: f32,
    /// Where the volume begins and ends along the axis, from the hinge point, in metres.
    pub bottom: f32,
    pub top: f32,
    /// How far the leaf opens each way, in radians.
    pub angle: f32,
    leaf: Leaf,
}

impl DoorArc {
    /// The arc of a swinging leaf that opens `angle` radians each way.
    pub fn of(leaf: &Leaf, angle: f32) -> Result<Self, ArcError> {
        let hinge = leaf.hinge.ok_or(ArcError::NoHinge)?;
        if !(angle > 0.0 && angle <= std::f32::consts::PI) {
            return Err(ArcError::Angle(angle));
        }
        let radius = [leaf.min.x, leaf.max.x]
            .into_iter()
            .flat_map(|x| [leaf.min.y, leaf.max.y].map(|y| Vec2::new(x, y).length()))
            .fold(0.0, f32::max);
        Ok(Self {
            hinge: hinge.position,
            axis: hinge.axis,
            closed: hinge.toward_free_edge,
            radius,
            bottom: leaf.min.z,
            top: leaf.max.z,
            angle,
            leaf: *leaf,
        })
    }

    /// Whether the leaf passes through `point` on its way to the angle, either way.
    pub fn contains(&self, point: Vec3) -> bool {
        let local = self.leaf.rotation.inverse() * (point - self.leaf.position);
        if !local.is_finite() || local.z < self.bottom || local.z > self.top {
            return false;
        }
        let (min, max) = (self.leaf.min.truncate(), self.leaf.max.truncate());
        let inside = |p: Vec2| p.cmpge(min - EDGE).all() && p.cmple(max + EDGE).all();
        let flat = local.truncate();
        let from_axis = flat.length();
        // ⚠ Turning the leaf one way is turning the point the other, so the point is carried round its circle through the still box.
        if from_axis <= EDGE {
            return inside(flat);
        }
        let at = flat.y.atan2(flat.x);
        let within = |angle: f32| {
            let off = (angle - at + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            off.abs() <= self.angle
        };
        let on_circle = |angle: f32| Vec2::from_angle(angle) * from_axis;
        if [at - self.angle, at + self.angle]
            .into_iter()
            .any(|end| inside(on_circle(end)))
        {
            return true;
        }
        // Between its ends the circle enters the box only across one of the four lines of its edges.
        let crossings = [min.x, max.x]
            .into_iter()
            .filter(|x| x.abs() <= from_axis)
            .flat_map(|x| {
                let y = (from_axis * from_axis - x * x).sqrt();
                [Vec2::new(x, y), Vec2::new(x, -y)]
            })
            .chain(
                [min.y, max.y]
                    .into_iter()
                    .filter(|y| y.abs() <= from_axis)
                    .flat_map(|y| {
                        let x = (from_axis * from_axis - y * y).sqrt();
                        [Vec2::new(x, y), Vec2::new(-x, y)]
                    }),
            );
        crossings
            .into_iter()
            .any(|p| inside(p) && within(p.y.atan2(p.x)))
    }
}

/// What a point on an edge of the box is forgiven, in metres.
const EDGE: f32 = 1e-5;

/// Another entity's box, which a leaf may meet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obstacle {
    pub at: EntityAt,
    pub shape: OrientedBox,
}

/// What stopped a leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker<K> {
    /// A piece of the bound a collision set holds under `key`.
    Collider { key: K, part: Part },
    /// The box of another entity.
    Entity(EntityAt),
}

/// Where a leaf stopped, and on what.
#[derive(Debug, Clone, PartialEq)]
pub struct Blocked<K> {
    /// The first turn at which the leaf is met, in radians from closed.
    pub angle: f32,
    pub by: Blocker<K>,
}

/// How far a leaf opens one way.
#[derive(Debug, Clone, PartialEq)]
pub struct Clearance<K> {
    pub way: Way,
    /// The angle it was asked to open to, in radians.
    pub asked: f32,
    /// None where it opens all the way.
    pub blocked: Option<Blocked<K>>,
}

impl<K> Clearance<K> {
    /// The angle the leaf reaches, in radians.
    pub fn reaches(&self) -> f32 {
        self.blocked.as_ref().map_or(self.asked, |b| b.angle)
    }
}

/// The box that is swept: the leaf's own, cut back from its faces, its bottom and its hinge.
fn swept(leaf: &Leaf, hinge: &Hinge, turn: f32) -> OrientedBox {
    let (mut min, mut max) = (leaf.min + ARC_SKIN, leaf.max - ARC_SKIN);
    min.z += ARC_SILL - ARC_SKIN;
    let strip = ARC_HINGE_STRIP.min(hinge.reach * 0.5);
    let along = match leaf.width_axis {
        WidthAxis::X => 0,
        WidthAxis::Y => 1,
    };
    match hinge.side {
        HingeSide::Low => min[along] = min[along].max(strip),
        HingeSide::High => max[along] = max[along].min(-strip),
    }
    // A leaf thinner or lower than what is cut from it is swept as the sheet through its middle.
    let middle = (leaf.min + leaf.max) * 0.5;
    let crossed = min.cmpgt(max);
    let (min, max) = (
        Vec3::select(crossed, middle, min),
        Vec3::select(crossed, middle, max),
    );
    let cut = Leaf { min, max, ..*leaf };
    cut.box_at(turn)
}

/// Opens a swinging leaf `angle` radians one way, in steps, and says the first turn at which a bound or another entity's box is in its way.
pub fn clearance<K: Ord + Clone>(
    leaf: &Leaf,
    way: Way,
    angle: f32,
    colliders: &CollisionSet<K>,
    obstacles: &[Obstacle],
) -> Result<Clearance<K>, ArcError> {
    let arc = DoorArc::of(leaf, angle)?;
    let hinge = leaf.hinge.ok_or(ArcError::NoHinge)?;
    let at = |turn: f32| swept(leaf, &hinge, way.sign() * turn);

    let closed = at(0.0);
    let mut stands_in: BTreeMap<&K, BTreeSet<Part>> = BTreeMap::new();
    for (key, overlap) in colliders.overlap_box(&closed) {
        stands_in
            .entry(key)
            .or_default()
            .extend(overlap.inside.into_iter().chain(overlap.crossing));
    }
    // Only a box that comes within the ball about the whole arc can be met.
    let centre = leaf.position + leaf.rotation * (Vec3::Z * (arc.bottom + arc.top) * 0.5);
    let reach = Vec2::new(arc.radius, (arc.top - arc.bottom) * 0.5).length();
    let in_the_way: Vec<&Obstacle> = obstacles
        .iter()
        .filter(|o| (o.shape.centre - centre).length() <= reach + o.shape.half_extents.length())
        .filter(|o| !closed.touches(&o.shape))
        .collect();

    let blocker = |turn: f32| -> Option<Blocker<K>> {
        let leaf_box = at(turn);
        for (key, overlap) in colliders.overlap_box(&leaf_box) {
            let stood_in = stands_in.get(key);
            let met = (overlap.inside.iter().chain(&overlap.crossing))
                .find(|part| !stood_in.is_some_and(|parts| parts.contains(part)));
            if let Some(part) = met {
                return Some(Blocker::Collider {
                    key: key.clone(),
                    part: *part,
                });
            }
        }
        in_the_way
            .iter()
            .find(|o| leaf_box.touches(&o.shape))
            .map(|o| Blocker::Entity(o.at))
    };

    let step = ARC_STEP.min(ARC_TRAVEL / arc.radius);
    let steps = ((angle / step).ceil() as usize).clamp(1, ARC_MOST_STEPS);
    let mut free = 0.0;
    for i in 1..=steps {
        let turn = angle * i as f32 / steps as f32;
        let Some(mut by) = blocker(turn) else {
            free = turn;
            continue;
        };
        let mut stopped = turn;
        for _ in 0..HALVINGS {
            let middle = (free + stopped) * 0.5;
            match blocker(middle) {
                Some(nearer) => {
                    stopped = middle;
                    by = nearer;
                }
                None => free = middle,
            }
        }
        return Ok(Clearance {
            way,
            asked: angle,
            blocked: Some(Blocked { angle: stopped, by }),
        });
    }
    Ok(Clearance {
        way,
        asked: angle,
        blocked: None,
    })
}
