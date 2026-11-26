//! GTA VI's ray-tracing hierarchy on the CPU: the game's selection of a drawable's triangles, an
//! arena in the console's node layout, its builder, the intersection instruction as a function
//! and the walk the game's programs run around it.
//!
//! The reference every other body of the instruction and every lowered walk is checked against.

mod build;
mod intersect;
pub mod layout;
mod query;
#[cfg(test)]
mod reference;
mod refit;
#[cfg(test)]
mod scenes;
pub mod select;
mod walk;

pub use build::{AccelError, Arena, Instance, Split, Structure, Triangle};
pub use intersect::{intersect_ray, word, MISS};
pub use query::{
    agreed, device_triangles, handed_over, instance_nodes, shuffled, stated_boxes, Agreed, Query,
    Shown,
};
pub use walk::{
    candidates, candidates_as, record_of, walk_lane, walk_lane_among, walk_lane_with, walk_wave,
    Candidate, Code, Cull, Form, Hit, Near, Opacity, Outcome, Probe, Ray, Walk, Wave, WaveOutcome,
    RAY_CULL_BACK, RAY_CULL_FRONT, RAY_FORCE_NON_OPAQUE, RAY_FORCE_OPAQUE, RAY_SKIP_NON_OPAQUE,
    RAY_SKIP_OPAQUE,
};
