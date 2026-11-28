//! One bound as the shapes it is made of, under a tree.

use glam::Vec3;

use super::bvh::Bvh;
use super::shape::{Obb, Shape, Touch};
use super::{BoxOverlap, CollisionError, Hit, OrientedBox, Part};
use crate::aabb::Aabb;

#[derive(Debug, Clone, Copy)]
struct Source {
    part: Part,
    material: Option<u64>,
}

/// A bound's shapes in the bound's own space, with the tree its queries walk.
#[derive(Debug, Clone)]
pub struct BoundCollider {
    shapes: Vec<Shape>,
    sources: Vec<Source>,
    tree: Bvh,
    shapeless: usize,
}

impl BoundCollider {
    /// The collider of `pieces`, each a shape with the piece of its bound it is and that piece's material, refused where a shape is not sound; `shapeless` is how many pieces were given no shape.
    pub fn of_shapes(
        pieces: Vec<(Shape, Part, Option<u64>)>,
        shapeless: usize,
    ) -> Result<Self, CollisionError> {
        if u32::try_from(pieces.len()).is_err() {
            return Err(CollisionError::TooManyShapes {
                shapes: pieces.len(),
            });
        }
        let mut shapes = Vec::with_capacity(pieces.len());
        let mut sources = Vec::with_capacity(pieces.len());
        for (shape, part, material) in pieces {
            // ⚠ The tree is built of boxes, and the box of a shape with a number lost is the box of the rest of it.
            if !shape.sound() {
                return Err(CollisionError::Number {
                    leaf: part.leaf,
                    what: match part.polygon {
                        Some(polygon) => format!("polygon {polygon}"),
                        None => "the bound".into(),
                    },
                });
            }
            shapes.push(shape);
            sources.push(Source { part, material });
        }
        let boxes: Vec<Aabb> = shapes.iter().map(Shape::aabb).collect();
        Ok(Self {
            tree: Bvh::build(&boxes),
            shapes,
            sources,
            shapeless,
        })
    }

    /// How many shapes the bound came to: one a polygon, one a primitive bound.
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    /// The pieces that were given no shape: a plane, a bound of a type with no layout, a cylinder primitive of no length.
    pub fn shapeless(&self) -> usize {
        self.shapeless
    }

    /// The bytes the collider holds in memory: itself, its shapes and its tree.
    pub fn size_in_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(self.shapes.as_slice())
            + std::mem::size_of_val(self.sources.as_slice())
            + self.tree.size_in_bytes()
    }

    /// The box of every shape in the bound's own space; empty for a bound of none.
    pub fn local_box(&self) -> Aabb {
        self.tree.bounds()
    }

    /// The nearest thing a ray meets within `max_distance`; a ray of no direction meets nothing.
    pub fn ray_cast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<Hit> {
        let direction = direction.try_normalize()?;
        self.ray(origin, direction, max_distance, |_| true)
            .map(|(touch, item)| self.hit(&touch, item))
    }

    /// The first thing a sphere touches when moved `max_distance` along `direction`.
    pub fn sweep_sphere(
        &self,
        start: Vec3,
        direction: Vec3,
        max_distance: f32,
        radius: f32,
    ) -> Option<Hit> {
        let direction = direction.try_normalize()?;
        self.sweep(start, direction, max_distance, radius)
            .map(|(touch, item)| self.hit(&touch, item))
    }

    /// The pieces wholly in a box and the pieces partly in it.
    pub fn overlap_box(&self, region: &OrientedBox) -> BoxOverlap {
        self.overlap(&obb_of(region))
    }

    /// The shapes whose own box touches `reach`, each with the piece it is, in the order the bound lists them.
    pub(crate) fn shapes_reaching(&self, reach: &Aabb) -> Vec<(Part, Shape)> {
        let mut found = Vec::new();
        self.tree.overlap(reach, |item| found.push(item));
        found.sort_unstable();
        found
            .into_iter()
            .map(|item| (self.sources[item as usize].part, self.shapes[item as usize]))
            .collect()
    }

    pub(crate) fn hit(&self, touch: &Touch, item: usize) -> Hit {
        let source = &self.sources[item];
        Hit {
            distance: touch.t,
            position: touch.point,
            normal: touch.normal,
            material: source.material,
            part: source.part,
        }
    }

    /// The nearest touch of a unit-direction ray that `accept` takes, and the shape it is on.
    pub(crate) fn ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        accept: impl Fn(&Touch) -> bool,
    ) -> Option<(Touch, usize)> {
        // ⚠ An origin that is no number compares as neither side of anything, and would be let in.
        if !origin.is_finite() {
            return None;
        }
        let mut best: Option<(Touch, usize)> = None;
        self.tree.ray(origin, direction, max, 0.0, |item| {
            let limit = best.as_ref().map_or(max, |(touch, _)| touch.t);
            if let Some(touch) = self.shapes[item as usize].ray(origin, direction, limit) {
                let nearer = best.as_ref().is_none_or(|(b, _)| touch.t < b.t);
                if nearer && accept(&touch) {
                    best = Some((touch, item as usize));
                }
            }
            best.as_ref().map_or(max, |(touch, _)| touch.t)
        });
        best
    }

    pub(crate) fn sweep(
        &self,
        start: Vec3,
        direction: Vec3,
        max: f32,
        radius: f32,
    ) -> Option<(Touch, usize)> {
        // ⚠ A sphere that starts touching is met before any length is looked at, so a length
        // that is none is refused here.
        if max.is_nan() || max < 0.0 || !radius.is_finite() {
            return None;
        }
        // A sphere of no size is a ray, whose normal is the surface's own.
        if radius <= 0.0 {
            return self.ray(start, direction, max, |_| true);
        }
        let mut best: Option<(Touch, usize)> = None;
        self.tree.ray(start, direction, max, radius, |item| {
            let limit = best.as_ref().map_or(max, |(touch, _)| touch.t);
            if let Some(touch) = self.shapes[item as usize].sweep(start, direction, limit, radius) {
                if best.as_ref().is_none_or(|(b, _)| touch.t < b.t) {
                    best = Some((touch, item as usize));
                }
            }
            best.as_ref().map_or(max, |(touch, _)| touch.t)
        });
        best
    }

    pub(crate) fn overlap(&self, region: &Obb) -> BoxOverlap {
        let mut found = Vec::new();
        self.tree
            .overlap(&Shape::Box(*region).aabb(), |item| found.push(item));
        found.sort_unstable();
        let mut out = BoxOverlap::default();
        for item in found {
            let shape = &self.shapes[item as usize];
            let part = self.sources[item as usize].part;
            if shape.within(region) {
                out.inside.push(part);
            } else if shape.touches(region) {
                out.crossing.push(part);
            }
        }
        out
    }
}

impl OrientedBox {
    /// Whether this box and another share a point.
    pub fn touches(&self, other: &OrientedBox) -> bool {
        Shape::Box(obb_of(self)).touches(&obb_of(other))
    }
}

pub(crate) fn obb_of(region: &OrientedBox) -> Obb {
    let rotation = region.rotation.normalize();
    Obb {
        centre: region.centre,
        axes: Vec3::AXES.map(|axis| rotation * axis),
        half: region.half_extents.abs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collider_counts_the_bytes_of_itself_its_shapes_their_sources_and_its_tree() {
        let corner = |v: u16| Vec3::new(f32::from(v / 2), f32::from(v % 2), 0.0);
        let pieces: Vec<(Shape, Part, Option<u64>)> = (0..60u16)
            .map(|t| {
                let part = Part {
                    leaf: 0,
                    polygon: Some(u32::from(t)),
                };
                let triangle = Shape::Triangle {
                    a: corner(t),
                    b: corner(t + 1),
                    c: corner(t + 2),
                };
                (triangle, part, None)
            })
            .collect();
        let collider = BoundCollider::of_shapes(pieces, 0).expect("whole");
        assert_eq!((collider.shapes.len(), collider.sources.len()), (60, 60));
        let tree = collider.tree.size_in_bytes();
        assert!(tree > 0);
        assert_eq!(
            collider.size_in_bytes(),
            std::mem::size_of::<BoundCollider>()
                + 60 * std::mem::size_of::<Shape>()
                + 60 * std::mem::size_of::<Source>()
                + tree
        );
    }

    #[test]
    fn a_collider_of_shapes_refuses_a_shape_that_is_no_number_and_names_its_piece() {
        let part = |polygon| Part { leaf: 3, polygon };
        let sphere = |radius| Shape::Sphere {
            centre: Vec3::ONE,
            radius,
        };
        let whole = vec![(sphere(1.0), part(None), None)];
        assert!(BoundCollider::of_shapes(whole, 0).is_ok());
        for (polygon, what) in [(None, "the bound"), (Some(7), "polygon 7")] {
            let pieces = vec![
                (sphere(1.0), part(None), None),
                (sphere(f32::NAN), part(polygon), None),
            ];
            assert_eq!(
                BoundCollider::of_shapes(pieces, 0).err(),
                Some(CollisionError::Number {
                    leaf: 3,
                    what: what.into()
                })
            );
        }
    }
}
