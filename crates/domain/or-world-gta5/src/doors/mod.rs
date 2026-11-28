//! The doors of a set of maps: which entities are doors, what each leaf's box says of how it
//! opens, which two are one double door, and the arc a swinging leaf sweeps with what stops it.
//!
//! ⚠ **Everything here is a reading of an archetype's box and an entity's placement.** The game
//! keeps how a door opens in its door tuning, which nothing here reads, so a hinge, a motion and
//! an arc are proposals with their reasons, and an arc opens to the angle its caller asks for.

mod arc;
mod find;
mod leaf;
mod pair;

pub use arc::{
    clearance, ArcError, Blocked, Blocker, Clearance, DoorArc, Obstacle, Way, ARC_HINGE_STRIP,
    ARC_MOST_STEPS, ARC_SILL, ARC_SKIN, ARC_STEP, ARC_TRAVEL,
};
pub use find::{
    find, Door, EntityAt, Found, FoundBy, NamePatterns, Names, PatternError, PlacedEntity,
};
pub use leaf::{
    read_leaf, Hinge, HingeSide, Leaf, Motion, Proposal, Reason, Unread, WidthAxis,
    HINGE_EDGE_SHARE, LIFT_MIN_WIDTH,
};
pub use pair::{pair, Pair, PAIR_ANGLE_TOLERANCE, PAIR_GAP_SLACK, PAIR_PLANE_TOLERANCE};
