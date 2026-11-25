//! One bound as the shapes it is made of, under a tree.

use glam::{Affine3A, Vec3, Vec3A};
use or_io::gta5::bounds::VERTEX_INDEX_MASK;
use or_io::gta5::{Mat34V, PhBound, PhBoundAny, PhBoundComposite, PhBoundGeometry};

use super::bvh::Bvh;
use super::shape::{Obb, Shape, Similarity, Touch};
use super::{BoxOverlap, CollisionError, Hit, OrientedBox, Part};
use crate::extents::Aabb;

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
    /// The collider of a bound, through its composites; refused where the bound names what it does not hold.
    pub fn new(bound: &PhBoundAny) -> Result<Self, CollisionError> {
        let mut builder = Builder::default();
        builder.bound(bound, &Similarity::IDENTITY)?;
        let Builder {
            shapes,
            sources,
            shapeless,
            ..
        } = builder;
        if u32::try_from(shapes.len()).is_err() {
            return Err(CollisionError::TooManyShapes {
                shapes: shapes.len(),
            });
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

pub(crate) fn obb_of(region: &OrientedBox) -> Obb {
    let rotation = region.rotation.normalize();
    Obb {
        centre: region.centre,
        axes: Vec3::AXES.map(|axis| rotation * axis),
        half: region.half_extents.abs(),
    }
}

#[derive(Default)]
struct Builder {
    shapes: Vec<Shape>,
    sources: Vec<Source>,
    leaves: u32,
    shapeless: usize,
}

impl Builder {
    fn bound(&mut self, bound: &PhBoundAny, place: &Similarity) -> Result<(), CollisionError> {
        if let PhBoundAny::PhBoundComposite(composite) = bound {
            return self.composite(composite, place);
        }
        let leaf = self.leaves;
        self.leaves += 1;
        let (header, shape) = match bound {
            PhBoundAny::PhBoundGeometry(g) => return self.geometry(g, leaf, place),
            PhBoundAny::PhBoundBVH(b) => return self.geometry(&b.base, leaf, place),
            PhBoundAny::PhBoundSphere(b) => (
                &b.base,
                Shape::Sphere {
                    centre: centroid(&b.base),
                    radius: b.base.radius_around_centroid,
                },
            ),
            // ⚠ A capsule's radius is its margin, and half its length the rest of its radius around the centroid.
            PhBoundAny::PhBoundCapsule(b) => {
                let radius = margin(&b.base);
                let reach = Vec3::Y * (b.base.radius_around_centroid - radius);
                if reach.y < 0.0 {
                    return Err(CollisionError::Number {
                        leaf,
                        what: "a capsule shorter than its own radius".into(),
                    });
                }
                let centre = centroid(&b.base);
                (
                    &b.base,
                    Shape::Capsule {
                        a: centre - reach,
                        b: centre + reach,
                        radius,
                    },
                )
            }
            PhBoundAny::PhBoundBox(b) => {
                let (min, max) = corners(&b.base);
                (
                    &b.base,
                    Shape::Box(Obb {
                        centre: (min + max) * 0.5,
                        axes: Vec3::AXES,
                        half: (max - min) * 0.5,
                    }),
                )
            }
            // ⚠ A cylinder lies along Y and reads its radius and half its length off its box: the
            // words it declares for them are zero in every one shipped.
            PhBoundAny::PhBoundCylinder(b) => {
                let (min, max) = corners(&b.base);
                let size = max - min;
                (
                    &b.base,
                    Shape::Cylinder {
                        centre: centroid(&b.base),
                        axis: Vec3::Y,
                        half_length: size.y * 0.5,
                        // ⚠ `max` passes over a NaN: a width that is no number is asked for first.
                        radius: if size.is_finite() {
                            size.x.max(size.z) * 0.5
                        } else {
                            f32::NAN
                        },
                    },
                )
            }
            // ⚠ A disc lies flat across X: half its thickness is its margin, or half its box
            // along X where it states no margin. A margin that is no number is neither, and
            // is kept so that the shape is refused.
            PhBoundAny::PhBoundDisc(b) => {
                let (min, max) = corners(&b.base);
                let stated = margin(&b.base);
                let half_length = if stated <= 0.0 {
                    (max.x - min.x) * 0.5
                } else {
                    stated
                };
                (
                    &b.base,
                    Shape::Cylinder {
                        centre: centroid(&b.base),
                        axis: Vec3::X,
                        half_length,
                        radius: b.base.radius_around_centroid,
                    },
                )
            }
            // A plane's record states no plane, and a bare header is a type with no layout.
            PhBoundAny::PhBoundPlane(_) | PhBoundAny::Base(_) | PhBoundAny::PhBoundComposite(_) => {
                self.shapeless += 1;
                return Ok(());
            }
        };
        let source = Source {
            part: Part {
                leaf,
                polygon: None,
            },
            material: Some(header_material(header)),
        };
        self.push(shape, place, source)
    }

    fn composite(
        &mut self,
        composite: &PhBoundComposite,
        place: &Similarity,
    ) -> Result<(), CollisionError> {
        for (child, bound) in composite.bounds.iter().enumerate() {
            let Some(bound) = bound else { continue };
            let matrix =
                composite
                    .current_matrices
                    .get(child)
                    .ok_or(CollisionError::ChildMatrix {
                        child,
                        bounds: composite.bounds.len(),
                        matrices: composite.current_matrices.len(),
                    })?;
            let local =
                Similarity::new(affine(matrix)).ok_or(CollisionError::ChildTransform { child })?;
            self.bound(bound, &place.after(&local))?;
        }
        Ok(())
    }

    fn geometry(
        &mut self,
        g: &PhBoundGeometry,
        leaf: u32,
        place: &Similarity,
    ) -> Result<(), CollisionError> {
        let vertices: Vec<Vec3> = (0..g.compressed_vertices.len())
            .filter_map(|i| g.vertex_position(i))
            .map(Vec3::from)
            .collect();
        let indices = g.poly_mat_index_list.len();
        if indices != 0 && indices != g.polygons.len() {
            return Err(CollisionError::MaterialIndices {
                leaf,
                indices,
                polygons: g.polygons.len(),
            });
        }
        for (polygon, p) in g.polygons.iter().enumerate() {
            let vertex = |vertex: usize| {
                vertices.get(vertex).copied().ok_or(CollisionError::Vertex {
                    leaf,
                    polygon,
                    vertex,
                    vertices: vertices.len(),
                })
            };
            let unknown = CollisionError::UnknownPrimitive {
                leaf,
                polygon,
                bits: p.primitive(),
            };
            let shape = if p.primitive() == 0 {
                let [a, b, c] = p.vertex_indices.map(|v| usize::from(v & VERTEX_INDEX_MASK));
                Shape::Triangle {
                    a: vertex(a)?,
                    b: vertex(b)?,
                    c: vertex(c)?,
                }
            } else {
                let (named, radius) = g.polygon_vertices(p).ok_or(unknown.clone())?;
                match (p.primitive(), named.as_slice()) {
                    (1, &[centre]) => Shape::Sphere {
                        centre: vertex(centre)?,
                        radius,
                    },
                    (2, &[a, b]) => Shape::Capsule {
                        a: vertex(a)?,
                        b: vertex(b)?,
                        radius,
                    },
                    (3, &[a, b, c, d]) => box_of([vertex(a)?, vertex(b)?, vertex(c)?, vertex(d)?]),
                    (4, &[a, b]) => {
                        let (a, b) = (vertex(a)?, vertex(b)?);
                        let along = b - a;
                        // Two ends at one place state no axis, so no shape.
                        if along == Vec3::ZERO {
                            self.shapeless += 1;
                            continue;
                        }
                        Shape::Cylinder {
                            centre: (a + b) * 0.5,
                            axis: along.normalize(),
                            half_length: along.length() * 0.5,
                            radius,
                        }
                    }
                    _ => return Err(unknown),
                }
            };
            let material = match g.poly_mat_index_list.get(polygon) {
                Some(&index) => {
                    let material =
                        g.material_ids
                            .get(usize::from(index))
                            .ok_or(CollisionError::Material {
                                leaf,
                                polygon,
                                material: index.into(),
                                materials: g.material_ids.len(),
                            })?;
                    Some(material.id)
                }
                // With no index for it a polygon takes the first material, where there is one.
                None => g.material_ids.first().map(|material| material.id),
            };
            let source = Source {
                part: Part {
                    leaf,
                    polygon: Some(polygon as u32),
                },
                material,
            };
            self.push(shape, place, source)?;
        }
        Ok(())
    }

    fn push(
        &mut self,
        shape: Shape,
        place: &Similarity,
        source: Source,
    ) -> Result<(), CollisionError> {
        let shape = shape.placed(place);
        if !shape.sound() {
            return Err(CollisionError::Number {
                leaf: source.part.leaf,
                what: match source.part.polygon {
                    Some(polygon) => format!("polygon {polygon}"),
                    None => "the bound".into(),
                },
            });
        }
        self.shapes.push(shape);
        self.sources.push(source);
        Ok(())
    }
}

/// ⚠ A box primitive names four corners of its box that no edge joins
/// (`a_box_primitive_names_four_corners_no_edge_joins`), so half an edge is a
/// quarter of two of them less the other two.
fn box_of([a, b, c, d]: [Vec3; 4]) -> Shape {
    Shape::Box(Obb::from_half_edges(
        (a + b + c + d) * 0.25,
        [
            (a + b - c - d) * 0.25,
            (a + c - b - d) * 0.25,
            (a + d - b - c) * 0.25,
        ],
    ))
}

fn centroid(header: &PhBound) -> Vec3 {
    let c = &header.centroid_offset_xyz_material_id0_w;
    Vec3::new(c.x, c.y, c.z)
}

fn corners(header: &PhBound) -> (Vec3, Vec3) {
    let max = &header.bounding_box_max_xyz_margin_w;
    (
        Vec3::from(header.bounding_box_min()),
        Vec3::new(max.x, max.y, max.z),
    )
}

/// The margin, in the `w` of the bounding box's maximum.
fn margin(header: &PhBound) -> f32 {
    header.bounding_box_max_xyz_margin_w.w
}

/// The material of a bound that is one shape: the bits of two `w` words, the low half first.
fn header_material(header: &PhBound) -> u64 {
    let low = header.centroid_offset_xyz_material_id0_w.w.to_bits();
    let high = header.cg_offset_xyz_material_id1_w.w.to_bits();
    u64::from(low) | u64::from(high) << 32
}

fn affine(m: &Mat34V) -> Affine3A {
    let column = |c: &or_io::gta5::shared::Vec4V| Vec3A::new(c.x, c.y, c.z);
    Affine3A::from_cols(
        column(&m.col0),
        column(&m.col1),
        column(&m.col2),
        column(&m.col3),
    )
}
