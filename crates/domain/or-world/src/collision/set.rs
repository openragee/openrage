//! Bounds placed in the world, held by key and asked as one.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{Affine3A, Vec3};

use super::bound::{obb_of, BoundCollider};
use super::bvh::{boxes_touch, reaches};
use super::shape::{Obb, Shape, Similarity, Touch};
use super::{BoxOverlap, CollisionError, GroundProbe, Hit, OrientedBox, Part};
use crate::extents::Aabb;

/// How many steps of a float at the size of a point's coordinates the ground probe starts above its rise: what a point found on
/// a slope at map coordinates is off its plane by, measured over 4,000 asks at up to 7,000 m
/// (`the_ground_asked_at_the_height_it_answered_with_no_rise_is_the_same_ground`).
const ON_THE_GROUND: f32 = 16.0;

#[derive(Debug, Clone)]
struct Placed {
    collider: Arc<BoundCollider>,
    to_world: Similarity,
    to_local: Similarity,
    world_box: Aabb,
}

impl Placed {
    /// A local touch as a hit in the world.
    fn hit(&self, touch: &Touch, item: usize) -> Hit {
        let local = self.collider.hit(touch, item);
        Hit {
            distance: local.distance * self.to_world.scale(),
            position: self.to_world.point(local.position),
            normal: self.to_world.turn(local.normal),
            ..local
        }
    }
}

/// Placed bounds by key, answering in world space.
#[derive(Debug, Clone)]
pub struct CollisionSet<K> {
    // ⚠ Walked in key order, with the first of two equal answers kept, so what a query returns
    // does not depend on the order bounds were added in.
    placed: BTreeMap<K, Placed>,
}

impl<K> Default for CollisionSet<K> {
    fn default() -> Self {
        Self {
            placed: BTreeMap::new(),
        }
    }
}

impl<K: Ord> CollisionSet<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Places a bound under `key`, in place of any bound already there; `to_world` is a rotation, one scale and a translation.
    pub fn add(
        &mut self,
        key: K,
        collider: Arc<BoundCollider>,
        to_world: Affine3A,
    ) -> Result<(), CollisionError> {
        let to_world = Similarity::new(to_world).ok_or(CollisionError::Placement)?;
        let local = collider.local_box();
        let mut world_box = Aabb::default();
        if !local.is_empty() {
            let whole = Obb {
                centre: (local.min + local.max) * 0.5,
                axes: Vec3::AXES,
                half: (local.max - local.min) * 0.5,
            };
            for corner in whole.corners() {
                world_box.add_point(to_world.point(corner));
            }
        }
        self.placed.insert(
            key,
            Placed {
                collider,
                to_local: to_world.inverse(),
                to_world,
                world_box,
            },
        );
        Ok(())
    }

    /// Takes the bound under `key` out; false where there was none.
    pub fn remove(&mut self, key: &K) -> bool {
        self.placed.remove(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.placed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// The world box of the bound under `key`, empty for a bound of no shape.
    pub fn world_box(&self, key: &K) -> Option<Aabb> {
        self.placed.get(key).map(|placed| placed.world_box)
    }

    /// Every key with its bound's world box, in key order.
    pub fn world_boxes(&self) -> impl Iterator<Item = (&K, Aabb)> {
        self.placed
            .iter()
            .map(|(key, placed)| (key, placed.world_box))
    }

    /// The nearest thing a ray meets within `max_distance`; a ray of no direction meets nothing.
    pub fn ray_cast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<(&K, Hit)> {
        self.nearest(origin, direction, max_distance, 0.0, |_, _| true)
    }

    /// The first thing a sphere touches when moved `max_distance` along `direction`.
    pub fn sweep_sphere(
        &self,
        start: Vec3,
        direction: Vec3,
        max_distance: f32,
        radius: f32,
    ) -> Option<(&K, Hit)> {
        if radius.is_nan() {
            return None;
        }
        self.nearest(start, direction, max_distance, radius.max(0.0), |_, _| true)
    }

    /// For each bound a box reaches, the pieces wholly in the box and the pieces partly in it.
    pub fn overlap_box(&self, region: &OrientedBox) -> Vec<(&K, BoxOverlap)> {
        let world = obb_of(region);
        let reach = Shape::Box(world).aabb();
        let mut out = Vec::new();
        for (key, placed) in &self.placed {
            if !boxes_touch(&placed.world_box, &reach) {
                continue;
            }
            let local = Obb {
                centre: placed.to_local.point(world.centre),
                axes: world.axes.map(|axis| placed.to_local.turn(axis)),
                half: world.half * placed.to_local.scale(),
            };
            let overlap = placed.collider.overlap(&local);
            if !(overlap.inside.is_empty() && overlap.crossing.is_empty()) {
                out.push((key, overlap));
            }
        }
        out
    }

    /// For each bound a box reaches, the shapes near the box as they stand in the world, each with the piece it is.
    pub(crate) fn shapes_near(&self, world: &Obb) -> Vec<(&K, Vec<(Part, Shape)>)> {
        let reach = Shape::Box(*world).aabb();
        let mut out = Vec::new();
        for (key, placed) in &self.placed {
            if !boxes_touch(&placed.world_box, &reach) {
                continue;
            }
            let local = Obb {
                centre: placed.to_local.point(world.centre),
                axes: world.axes.map(|axis| placed.to_local.turn(axis)),
                half: world.half * placed.to_local.scale(),
            };
            let near = placed.collider.shapes_reaching(&Shape::Box(local).aabb());
            out.push((
                key,
                near.into_iter()
                    .map(|(part, shape)| (part, shape.placed(&placed.to_world)))
                    .collect(),
            ));
        }
        out
    }

    /// The ground under a point: the highest surface no steeper than the probe allows, from `max_rise` above the point to `max_drop` below it.
    pub fn ground(&self, point: Vec3, probe: &GroundProbe) -> Option<(&K, Hit)> {
        let usable = |length: f32| length.is_finite() && length >= 0.0;
        if !usable(probe.max_rise) || !usable(probe.max_drop) {
            return None;
        }
        // ⚠ A few ulps under the cosine: the normal of a level triangle comes out of its cross product just short of one, and a
        // limit of nought would refuse the floor and answer with whatever lies under it.
        let flattest = probe.max_slope.cos() - 4.0 * f32::EPSILON;
        // A solid the top of the probe starts inside is no surface, and is passed through.
        let accept = |normal: Vec3, inside: bool| !inside && normal.z >= flattest;
        let top = point + Vec3::Z * probe.max_rise;
        // ⚠ A point that stands on a slope is a rounding either side of its plane, by the size of its coordinates: a probe that
        // starts at the point passed the surface the point stands on in two asks of five. What lies within that rounding of the
        // top of the probe, above it or below, is looked for first, from that much higher.
        let lift = ON_THE_GROUND * f32::EPSILON * point.abs().max_element().max(1.0);
        let at_the_top = self.nearest(top + Vec3::Z * lift, Vec3::NEG_Z, 2.0 * lift, 0.0, accept);
        if let Some((key, mut hit)) = at_the_top {
            hit.distance = (hit.distance - lift).max(0.0);
            return Some((key, hit));
        }
        self.nearest(
            top,
            Vec3::NEG_Z,
            probe.max_rise + probe.max_drop,
            0.0,
            accept,
        )
    }

    /// The nearest touch of a ray, or of a sphere moved along it, that `accept` takes by its world normal and whether the query began inside.
    fn nearest(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        radius: f32,
        accept: impl Fn(Vec3, bool) -> bool,
    ) -> Option<(&K, Hit)> {
        let direction = direction.try_normalize()?;
        let mut best: Option<(&K, Hit)> = None;
        for (key, placed) in &self.placed {
            let limit = best.as_ref().map_or(max_distance, |(_, hit)| hit.distance);
            if !reaches(&placed.world_box, origin, direction, limit, radius) {
                continue;
            }
            let shrink = placed.to_local.scale();
            let (from, along) = (
                placed.to_local.point(origin),
                placed.to_local.turn(direction),
            );
            let found = if radius > 0.0 {
                placed
                    .collider
                    .sweep(from, along, limit * shrink, radius * shrink)
            } else {
                placed.collider.ray(from, along, limit * shrink, |touch| {
                    accept(placed.to_world.turn(touch.normal), touch.inside)
                })
            };
            if let Some((touch, item)) = found {
                let hit = placed.hit(&touch, item);
                if best.as_ref().is_none_or(|(_, b)| hit.distance < b.distance) {
                    best = Some((key, hit));
                }
            }
        }
        best
    }
}
