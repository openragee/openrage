//! What a door's box says of the leaf: its width, the end it turns about, and how it opens.
//!
//! The rule, which reads the box and never the game's door tuning: the width runs along the longer
//! of the box's two level edges. A leaf turns about the upright line through its model's origin, so
//! where the origin stands within [`HINGE_EDGE_SHARE`] of the width from one end of the box, that
//! end is the hinge and the leaf swings. Where the origin stands further in, no end is a hinge: a
//! leaf at least [`LIFT_MIN_WIDTH`] wide is read as one that lifts, and a narrower one as one that slides.

use std::fmt;

use glam::{Quat, Vec3};

use super::find::PlacedEntity;
use crate::collision::OrientedBox;

/// How near an end of its box a model's origin stands for that end to be the hinge, as a share of the width.
pub const HINGE_EDGE_SHARE: f32 = 0.2;

/// The width from which a leaf with no hinge is read as one that lifts, in metres.
pub const LIFT_MIN_WIDTH: f32 = 2.5;

/// Which level axis of the model the width runs along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthAxis {
    X,
    Y,
}

impl WidthAxis {
    fn unit(self) -> Vec3 {
        match self {
            WidthAxis::X => Vec3::X,
            WidthAxis::Y => Vec3::Y,
        }
    }

    fn of(self, v: Vec3) -> f32 {
        match self {
            WidthAxis::X => v.x,
            WidthAxis::Y => v.y,
        }
    }
}

/// Which end of the width the hinge is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HingeSide {
    /// The end the width axis runs from, so the leaf reaches along the axis.
    Low,
    /// The end the width axis runs to, so the leaf reaches against the axis.
    High,
}

/// How a leaf opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Swing,
    Slide,
    Lift,
}

/// What in the box a proposed motion rests on, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reason {
    /// The origin stands this near one end of the width, so that end is a hinge.
    OriginAtAnEnd { from_end: f32, width: f32 },
    /// The origin stands away from both ends of a width under [`LIFT_MIN_WIDTH`].
    NoEndNarrow { from_nearer_end: f32, width: f32 },
    /// The origin stands away from both ends of a width of [`LIFT_MIN_WIDTH`] or more.
    NoEndWide { from_nearer_end: f32, width: f32 },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::OriginAtAnEnd { from_end, width } => write!(
                f,
                "its model's origin stands {from_end:.2} m from one end of a box {width:.2} m wide, so that end is read as its hinge"
            ),
            Reason::NoEndNarrow {
                from_nearer_end,
                width,
            } => write!(
                f,
                "its model's origin stands {from_nearer_end:.2} m from the nearer end of a box {width:.2} m wide, so no end is a hinge and it is read as sliding"
            ),
            Reason::NoEndWide {
                from_nearer_end,
                width,
            } => write!(
                f,
                "its model's origin stands {from_nearer_end:.2} m from the nearer end of a box {width:.2} m wide, so no end is a hinge, and a leaf that wide is read as lifting"
            ),
        }
    }
}

/// How a leaf is proposed to open, with what the proposal rests on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proposal {
    pub motion: Motion,
    pub reason: Reason,
}

/// The line a swinging leaf turns about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hinge {
    pub side: HingeSide,
    /// A point of the line: the entity's position.
    pub position: Vec3,
    /// The line's unit direction, which is the model's up as the entity is turned.
    pub axis: Vec3,
    /// The unit direction from the hinge to the leaf's free edge, with the leaf closed.
    pub toward_free_edge: Vec3,
    /// How far the free edge is from the hinge, in metres.
    pub reach: f32,
}

/// Why a door's leaf is not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unread {
    #[error("no type file handed in declares its archetype, so it has no box to read")]
    NoBox,
    #[error("its box, its scale or its placement holds a number that is not finite, or its box as scaled is of no finite size")]
    NotFinite,
    #[error("its box has a minimum above its maximum")]
    Inverted,
    #[error("its box has no width")]
    NoWidth,
}

/// A door's leaf as its box and its placement give it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leaf {
    pub position: Vec3,
    pub rotation: Quat,
    /// The lowest corner of the box in the model's own space, scaled as the entity is.
    pub min: Vec3,
    /// The highest corner, likewise.
    pub max: Vec3,
    pub width_axis: WidthAxis,
    /// In metres, as the other two.
    pub width: f32,
    pub thickness: f32,
    pub height: f32,
    pub proposal: Proposal,
    /// None for a leaf that does not swing.
    pub hinge: Option<Hinge>,
}

impl Leaf {
    /// The leaf's box in the world, turned `turn` radians about the upright line through its origin, anticlockwise seen from above.
    pub fn box_at(&self, turn: f32) -> OrientedBox {
        let rotation = self.rotation * Quat::from_rotation_z(turn);
        OrientedBox {
            centre: self.position + rotation * ((self.min + self.max) * 0.5),
            rotation,
            half_extents: (self.max - self.min) * 0.5,
        }
    }
}

/// Reads the leaf of a door from its archetype's box and its placement.
pub fn read_leaf(entity: &PlacedEntity) -> Result<Leaf, Unread> {
    let bounds = entity.bounds.ok_or(Unread::NoBox)?;
    let finite = bounds.min.is_finite()
        && bounds.max.is_finite()
        && entity.scale.is_finite()
        && entity.position.is_finite()
        && entity.rotation.is_finite();
    if !finite {
        return Err(Unread::NotFinite);
    }
    if bounds.min.cmpgt(bounds.max).any() {
        return Err(Unread::Inverted);
    }
    // A scale below nought mirrors the box, so its corners are sorted again.
    let (a, b) = (bounds.min * entity.scale, bounds.max * entity.scale);
    let (min, max) = (a.min(b), a.max(b));
    let size = max - min;
    // ⚠ Finite corners at a finite scale can still be a box of no finite size.
    if !(min.is_finite() && max.is_finite() && size.is_finite()) {
        return Err(Unread::NotFinite);
    }
    let (width_axis, width, thickness) = if size.x >= size.y {
        (WidthAxis::X, size.x, size.y)
    } else {
        (WidthAxis::Y, size.y, size.x)
    };
    if width <= 0.0 {
        return Err(Unread::NoWidth);
    }
    // The origin is nought on the axis, so each end's distance from it is the end's own coordinate.
    let (from_low, from_high) = (-width_axis.of(min), width_axis.of(max));
    let edge = HINGE_EDGE_SHARE * width;
    let side = if from_low.abs() <= edge && from_low.abs() <= from_high.abs() {
        Some((HingeSide::Low, from_low, from_high))
    } else if from_high.abs() <= edge {
        Some((HingeSide::High, from_high, from_low))
    } else {
        None
    };
    let (proposal, hinge) = match side {
        Some((side, from_end, reach)) => {
            let along = match side {
                HingeSide::Low => width_axis.unit(),
                HingeSide::High => -width_axis.unit(),
            };
            (
                Proposal {
                    motion: Motion::Swing,
                    reason: Reason::OriginAtAnEnd {
                        from_end: from_end.abs(),
                        width,
                    },
                },
                Some(Hinge {
                    side,
                    position: entity.position,
                    axis: entity.rotation * Vec3::Z,
                    toward_free_edge: entity.rotation * along,
                    reach,
                }),
            )
        }
        None => {
            let from_nearer_end = from_low.abs().min(from_high.abs());
            let (motion, reason) = if width >= LIFT_MIN_WIDTH {
                (
                    Motion::Lift,
                    Reason::NoEndWide {
                        from_nearer_end,
                        width,
                    },
                )
            } else {
                (
                    Motion::Slide,
                    Reason::NoEndNarrow {
                        from_nearer_end,
                        width,
                    },
                )
            };
            (Proposal { motion, reason }, None)
        }
    };
    Ok(Leaf {
        position: entity.position,
        rotation: entity.rotation,
        min,
        max,
        width_axis,
        width,
        thickness,
        height: size.z,
        proposal,
        hinge,
    })
}
