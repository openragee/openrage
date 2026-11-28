//! Bounds built by hand for the collision suites, and the arithmetic their answers are held to.

#![allow(dead_code)]

use glam::{Affine3A, DVec3, Vec3};
use or_io::decl::{Open, Plain, Raw};
use or_io::gta5::bounds::build::{self, BoundMesh};
use or_io::gta5::shared::Vec4V;
use or_io::gta5::{
    BoundType, Mat34V, PhBound, PhBoundAny, PhBoundBox, PhBoundCapsule, PhBoundComposite,
    PhBoundCylinder, PhBoundDisc, PhBoundGeometry, PhBoundSphere, PhMaterialMgrId, PhPolygon,
    PhPrimBox, PhPrimCapsule, PhPrimCylinder, PhPrimSphere,
};

fn vec4(v: Vec3, w: f32) -> Vec4V {
    Vec4V {
        x: v.x,
        y: v.y,
        z: v.z,
        w,
        ..Default::default()
    }
}

/// A header with its box, its centroid at the box's middle, its radius around it, its margin and its material.
pub fn header(
    kind: BoundType,
    min: Vec3,
    max: Vec3,
    margin: f32,
    radius: f32,
    material: u64,
) -> PhBound {
    let centre = (min + max) * 0.5;
    let mut header = PhBound {
        r#type: Open::new(kind),
        radius_around_centroid: radius,
        bounding_box_max_xyz_margin_w: vec4(max, margin),
        centroid_offset_xyz_material_id0_w: vec4(centre, f32::from_bits(material as u32)),
        cg_offset_xyz_material_id1_w: vec4(centre, f32::from_bits((material >> 32) as u32)),
        ..Default::default()
    };
    header.set_bounding_box_min(min.to_array());
    header
}

pub fn sphere(centre: Vec3, radius: f32, material: u64) -> PhBoundAny {
    let reach = Vec3::splat(radius);
    PhBoundAny::PhBoundSphere(PhBoundSphere {
        base: header(
            BoundType::Sphere,
            centre - reach,
            centre + reach,
            radius,
            radius,
            material,
        ),
        ..Default::default()
    })
}

/// A capsule along Y: its margin is its radius, and its radius around the centroid that and half its length.
pub fn capsule(centre: Vec3, radius: f32, half_length: f32, material: u64) -> PhBoundAny {
    let reach = Vec3::new(radius, half_length + radius, radius);
    PhBoundAny::PhBoundCapsule(PhBoundCapsule {
        base: header(
            BoundType::Capsule,
            centre - reach,
            centre + reach,
            radius,
            half_length + radius,
            material,
        ),
        ..Default::default()
    })
}

pub fn boxed(min: Vec3, max: Vec3, material: u64) -> PhBoundAny {
    PhBoundAny::PhBoundBox(PhBoundBox {
        base: header(
            BoundType::Box,
            min,
            max,
            0.04,
            ((max - min) * 0.5).length(),
            material,
        ),
        ..Default::default()
    })
}

/// A cylinder along Y, which states its radius and half its length in its box alone.
pub fn cylinder(centre: Vec3, radius: f32, half_length: f32, material: u64) -> PhBoundAny {
    let reach = Vec3::new(radius, half_length, radius);
    PhBoundAny::PhBoundCylinder(PhBoundCylinder {
        base: header(
            BoundType::Cylinder,
            centre - reach,
            centre + reach,
            0.04,
            reach.truncate().length(),
            material,
        ),
        ..Default::default()
    })
}

/// A disc flat across X, half as thick as its margin, or as its box where the margin is 0.
pub fn disc(
    centre: Vec3,
    radius: f32,
    half_thickness: f32,
    margin: f32,
    material: u64,
) -> PhBoundAny {
    let reach = Vec3::new(half_thickness, radius, radius);
    PhBoundAny::PhBoundDisc(PhBoundDisc {
        base: header(
            BoundType::Disc,
            centre - reach,
            centre + reach,
            margin,
            radius,
            material,
        ),
        ..Default::default()
    })
}

pub fn matrix(place: Affine3A) -> Mat34V {
    let column = |v: glam::Vec3A| vec4(Vec3::from(v), 0.0);
    Mat34V {
        col0: column(place.matrix3.x_axis),
        col1: column(place.matrix3.y_axis),
        col2: column(place.matrix3.z_axis),
        col3: column(place.translation),
        ..Default::default()
    }
}

/// A composite of children, each with the matrix that places it; a `None` child keeps its slot.
pub fn composite(children: Vec<(Option<PhBoundAny>, Affine3A)>) -> PhBoundAny {
    let (bounds, matrices): (Vec<_>, Vec<_>) = children
        .into_iter()
        .map(|(child, place)| (child.map(Box::new), matrix(place)))
        .unzip();
    PhBoundAny::PhBoundComposite(PhBoundComposite {
        base: header(
            BoundType::Composite,
            Vec3::splat(-100.0),
            Vec3::splat(100.0),
            0.04,
            100.0,
            0,
        ),
        bounds,
        current_matrices: matrices,
        ..Default::default()
    })
}

/// A primitive's sixteen bytes as the polygon slot that holds them.
fn slot<R: Plain>(raw: &R) -> PhPolygon {
    let read = <Raw<PhPolygon> as Plain>::read_from(raw.as_bytes()).expect("sixteen bytes");
    PhPolygon {
        area: { read.area },
        vertex_indices: { read.vertex_indices },
        neighboring_polygons: { read.neighboring_polygons },
        ..Default::default()
    }
}

pub fn sphere_primitive(centre: u16, radius: f32) -> PhPolygon {
    let mut raw = <Raw<PhPrimSphere> as Plain>::zeroed();
    raw.prim_type = 1;
    raw.center_index = centre;
    raw.radius = radius;
    slot(&raw)
}

pub fn capsule_primitive(ends: [u16; 2], radius: f32) -> PhPolygon {
    let mut raw = <Raw<PhPrimCapsule> as Plain>::zeroed();
    raw.prim_type = 2;
    raw.end_index0 = ends[0];
    raw.end_index1 = ends[1];
    raw.radius = radius;
    slot(&raw)
}

pub fn box_primitive(corners: [u16; 4]) -> PhPolygon {
    let mut raw = <Raw<PhPrimBox> as Plain>::zeroed();
    raw.prim_type = 3;
    raw.vertex_indices = corners;
    slot(&raw)
}

pub fn cylinder_primitive(ends: [u16; 2], radius: f32) -> PhPolygon {
    let mut raw = <Raw<PhPrimCylinder> as Plain>::zeroed();
    raw.prim_type = 4;
    raw.end_index0 = ends[0];
    raw.end_index1 = ends[1];
    raw.radius = radius;
    slot(&raw)
}

/// The four corners of a box that no edge joins, from its centre and its three half edges.
pub fn far_corners(centre: Vec3, half: [Vec3; 3]) -> [Vec3; 4] {
    [
        centre + half[0] + half[1] + half[2],
        centre + half[0] - half[1] - half[2],
        centre - half[0] + half[1] - half[2],
        centre - half[0] - half[1] + half[2],
    ]
}

/// A geometry over `positions`: its triangles first, then `primitives`, each polygon with the
/// material `materials` lists for it by index.
pub fn geometry(
    positions: &[Vec3],
    triangles: &[[u16; 3]],
    primitives: Vec<PhPolygon>,
    materials: &[u64],
    polygon_materials: &[u8],
) -> PhBoundGeometry {
    let mesh = BoundMesh {
        positions: positions.iter().map(|p| p.to_array()).collect(),
        triangles: triangles.to_vec(),
        materials: materials
            .iter()
            .map(|id| PhMaterialMgrId {
                id: *id,
                ..Default::default()
            })
            .collect(),
        triangle_materials: polygon_materials[..triangles.len().min(polygon_materials.len())]
            .to_vec(),
    };
    let mut g = build::geometry(&mesh).expect("a mesh a bound can hold");
    g.poly_mat_index_list = polygon_materials.to_vec();
    g.polygons.extend(primitives);
    g
}

/// A geometry of triangles alone, with no material.
pub fn triangles(positions: &[Vec3], triangles: &[[u16; 3]]) -> PhBoundGeometry {
    geometry(positions, triangles, Vec::new(), &[], &[])
}

/// The place a geometry holds for a vertex, which sixteen bits an axis moved from where it was put.
pub fn stored(g: &PhBoundGeometry, vertex: usize) -> DVec3 {
    let p = g.vertex_position(vertex).expect("a vertex the bound holds");
    DVec3::new(p[0].into(), p[1].into(), p[2].into())
}

/// How far `p` is outside a solid, negative inside it, by formulas the code under test does not use.
#[derive(Debug, Clone, Copy)]
pub enum Solid {
    Sphere {
        centre: DVec3,
        radius: f64,
    },
    Capsule {
        a: DVec3,
        b: DVec3,
        radius: f64,
    },
    /// A box with its edges along the world's axes.
    Box {
        centre: DVec3,
        half: DVec3,
    },
    /// Flat ended, along a unit `axis`.
    Cylinder {
        centre: DVec3,
        axis: DVec3,
        half_length: f64,
        radius: f64,
    },
}

impl Solid {
    pub fn distance(&self, p: DVec3) -> f64 {
        match *self {
            Solid::Sphere { centre, radius } => (p - centre).length() - radius,
            Solid::Capsule { a, b, radius } => {
                let ab = b - a;
                let along = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                (p - a - ab * along).length() - radius
            }
            Solid::Box { centre, half } => {
                let q = (p - centre).abs() - half;
                q.max(DVec3::ZERO).length() + q.max_element().min(0.0)
            }
            Solid::Cylinder {
                centre,
                axis,
                half_length,
                radius,
            } => {
                let v = p - centre;
                let along = v.dot(axis);
                let out = (v - axis * along).length() - radius;
                let over = along.abs() - half_length;
                out.max(over).min(0.0) + out.max(0.0).hypot(over.max(0.0))
            }
        }
    }
}

/// The first distance along a ray at which `p` is within `grow` of a solid: stepped, then halved.
///
/// ⚠ A step is `max / 20_000`, so a ray must stay in the solid for longer than that to be seen.
pub fn first_within(
    solid: &Solid,
    grow: f64,
    origin: DVec3,
    direction: DVec3,
    max: f64,
) -> Option<f64> {
    const STEPS: usize = 20_000;
    let within = |t: f64| solid.distance(origin + direction * t) <= grow;
    if within(0.0) {
        return Some(0.0);
    }
    let mut before = 0.0;
    for step in 1..=STEPS {
        let t = max * step as f64 / STEPS as f64;
        if within(t) {
            let (mut out, mut inside) = (before, t);
            for _ in 0..60 {
                let middle = (out + inside) * 0.5;
                if within(middle) {
                    inside = middle;
                } else {
                    out = middle;
                }
            }
            return Some(inside);
        }
        before = t;
    }
    None
}

/// A repeatable stream of numbers in `0..1`.
pub struct Dice(pub u64);

impl Dice {
    pub fn roll(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }

    pub fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.roll()
    }

    pub fn point(&mut self, low: f32, high: f32) -> Vec3 {
        Vec3::new(
            self.between(low, high),
            self.between(low, high),
            self.between(low, high),
        )
    }
}
