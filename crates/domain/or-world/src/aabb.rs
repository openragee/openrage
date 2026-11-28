//! An axis-aligned box, which a collision tree, a placed bound and a title's extents are measured in.

use glam::Vec3;

/// An axis-aligned box that starts empty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self {
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
        }
    }
}

impl Aabb {
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    pub fn add_point(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn add_box(&mut self, min: Vec3, max: Vec3) {
        self.add_point(min);
        self.add_point(max);
    }

    pub fn expanded(&self, by: f32) -> Self {
        if self.is_empty() {
            return *self;
        }
        Self {
            min: self.min - by,
            max: self.max + by,
        }
    }

    pub fn union(&mut self, other: &Aabb) {
        if other.is_empty() {
            return;
        }
        self.add_box(other.min, other.max);
    }
}

/// How far a recomputed box is from a stored one, in world units: the largest disagreement of one axis over both corners, and infinite for an empty box.
pub fn agreement(computed: &Aabb, stored: (Vec3, Vec3)) -> f32 {
    if computed.is_empty() {
        return f32::INFINITY;
    }
    let (min, max) = stored;
    (computed.min - min)
        .abs()
        .max((computed.max - max).abs())
        .max_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_box_never_agrees_with_a_stored_one() {
        let empty = Aabb::default();
        assert!(agreement(&empty, (Vec3::ZERO, Vec3::ZERO)).is_infinite());
        assert!(empty.expanded(100.0).is_empty());
    }

    #[test]
    fn agreement_is_the_worst_single_axis() {
        let a = Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(1.0, 1.0, 1.0),
        };
        assert_eq!(agreement(&a, (Vec3::ZERO, Vec3::ONE)), 0.0);
        assert_eq!(agreement(&a, (Vec3::ZERO, Vec3::new(1.0, 1.0, 4.0))), 3.0);
    }
}
