//! Collision as a query over bounds placed in the world, Z up: a ray, a swept sphere, a box
//! overlap and the ground under a point. A [`BoundCollider`] is one bound's shapes in the bound's
//! own space under a tree built once, and a [`CollisionSet`] holds colliders by key, each with the
//! transform that places it.

use glam::{Quat, Vec3};

// ⚠ A shape is itself, never a mesh of it: a ray meets a sphere where the sphere is. The particle
// preview's collider (`or_simulation::collision`) makes triangles of the same bounds and leaves
// out the primitives of a geometry, so the two do not agree on a curved bound.
mod bound;
mod bvh;
mod set;
mod shape;

pub use bound::BoundCollider;
pub use set::CollisionSet;

/// Which piece of a bound a shape is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Part {
    /// The bound, counted depth first among those that are not composites.
    pub leaf: u32,
    /// The polygon of a geometry, by its place in `polygons`; `None` for a bound that is one shape.
    pub polygon: Option<u32>,
}

/// Where a query met a bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// How far along the query, in metres; 0 where it began touching.
    pub distance: f32,
    /// The point of the surface that was met.
    pub position: Vec3,
    /// The surface's unit normal there, facing the query.
    pub normal: Vec3,
    /// The `phMaterialMgr::Id` word of what was met; `None` for a polygon of a geometry that lists no material.
    pub material: Option<u64>,
    pub part: Part,
}

/// A box with axes of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrientedBox {
    pub centre: Vec3,
    pub rotation: Quat,
    pub half_extents: Vec3,
}

/// The pieces of one bound a box holds, in the order the bound lists them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxOverlap {
    /// Wholly in the box.
    pub inside: Vec<Part>,
    /// Partly in the box.
    pub crossing: Vec<Part>,
}

/// How the ground under a point is looked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundProbe {
    /// How far above the point a surface may be and still be the ground, in metres.
    pub max_rise: f32,
    /// How far below the point the ground is looked for, in metres.
    pub max_drop: f32,
    /// The steepest surface that is ground, as its normal's angle from up in radians.
    pub max_slope: f32,
}

/// Why a bound cannot be queried.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CollisionError {
    #[error("bound {leaf}: polygon {polygon} names vertex {vertex} of {vertices}")]
    Vertex {
        leaf: u32,
        polygon: usize,
        vertex: usize,
        vertices: usize,
    },
    #[error("bound {leaf}: polygon {polygon} names material {material} of {materials}")]
    Material {
        leaf: u32,
        polygon: usize,
        material: usize,
        materials: usize,
    },
    #[error("bound {leaf}: {indices} material indices for {polygons} polygons")]
    MaterialIndices {
        leaf: u32,
        indices: usize,
        polygons: usize,
    },
    #[error(
        "bound {leaf}: polygon {polygon}'s area bits end {bits:03b}, which names no primitive"
    )]
    UnknownPrimitive {
        leaf: u32,
        polygon: usize,
        bits: u32,
    },
    #[error("bound {leaf}: {what} is not a finite size or place")]
    Number { leaf: u32, what: String },
    #[error("a composite's child {child} has no matrix: {matrices} for {bounds} bounds")]
    ChildMatrix {
        child: usize,
        bounds: usize,
        matrices: usize,
    },
    #[error("a composite's child {child} is placed by a matrix that is not a rotation, one scale and a translation")]
    ChildTransform { child: usize },
    #[error(
        "a bound is placed by a transform that is not a rotation, one scale and a translation"
    )]
    Placement,
    #[error("a bound of {shapes} shapes is past the {} a tree counts", u32::MAX)]
    TooManyShapes { shapes: usize },
}
