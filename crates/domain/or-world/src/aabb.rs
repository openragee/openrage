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
