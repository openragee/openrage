//! A bound value's arrays edited where they change length: a geometry's primitives and vertices,
//! a composite's children, and a primitive child given another type.

use std::cmp::Ordering;

use super::write_bound_grow::{past, MAX_VERTICES, NO_BVH, ONLY_PRIMITIVE, OPEN, OPEN_EDGE};
use super::write_bound_runs::{geometry_value_copied, NOT_COPIED};
use super::write_bound_vertex::item_box;
use super::write_bvh::{corners, union};
use super::write_physics::{local_box, refuse};
use super::{rewire_triangle, Bound, BoundChild, BoundGeometry, ChildBound, Primitive, Result};

fn has_bvh(g: &BoundGeometry) -> Result<()> {
    match g.bvh {
        Some(_) => Ok(()),
        None => Err(refuse(NO_BVH)),
    }
}

/// The vertices `p` names.
pub(super) fn names(p: &Primitive) -> Vec<u16> {
    match *p {
        Primitive::Triangle { vertices, .. } => vertices.to_vec(),
        Primitive::Sphere { centre, .. } => vec![centre],
        Primitive::Capsule { ends, .. } | Primitive::Cylinder { ends, .. } => ends.to_vec(),
        Primitive::Box(corners) => corners.to_vec(),
    }
}

/// Primitive `primitive` of `g` deleted, its material with it.
// Each triangle across it is left open there and every neighbour past it is one less.
pub fn delete_bound_primitive(g: &mut BoundGeometry, primitive: usize) -> Result<()> {
    has_bvh(g)?;
    if primitive >= g.primitives.len() {
        return Err(past("primitive to delete", primitive, g.primitives.len()));
    }
    if g.primitives.len() == 1 {
        return Err(refuse("a geometry keeps one primitive at least"));
    }
    g.primitives.remove(primitive);
    if primitive < g.primitive_materials.len() {
        g.primitive_materials.remove(primitive);
    }
    for p in &mut g.primitives {
        if let Primitive::Triangle {
            neighbours, edges, ..
        } = p
        {
            for (n, e) in neighbours.iter_mut().zip(edges.iter_mut()) {
                if *n == OPEN {
                    continue;
                }
                match usize::from(*n).cmp(&primitive) {
                    Ordering::Equal => (*n, *e) = (OPEN, OPEN_EDGE),
                    Ordering::Greater => *n -= 1,
                    Ordering::Less => {}
                }
            }
        }
    }
    g.unknown_shapes.retain(|(i, _)| *i != primitive);
    for (i, _) in &mut g.unknown_shapes {
        if *i > primitive {
            *i -= 1;
        }
    }
    Ok(())
}

/// `primitive` added as `g`'s last with material index `material`; where it is.
// It lies inside its child's box; a triangle is linked across each edge it shares with a triangle
// open there, each such edge's byte given by their angle.
pub fn add_bound_primitive(
    g: &mut BoundGeometry,
    primitive: Primitive,
    material: u8,
) -> Result<usize> {
    has_bvh(g)?;
    let index = g.primitives.len();
    if index >= usize::from(OPEN) {
        return Err(refuse(
            "a primitive past the 65,535th is named by no neighbour",
        ));
    }
    let at = |i: u16| g.vertices.get(usize::from(i)).copied();
    let (lo, hi) = item_box(&primitive, &at, 0.0)
        .ok_or(refuse("a primitive names a vertex past the vertices"))?;
    if (0..3).any(|a| lo[a] < g.box_min[a] || hi[a] > g.box_max[a]) {
        return Err(refuse(
            "an added primitive lies past its child's box: move a vertex out first, which grows it",
        ));
    }
    let materials = g.material_ids.len().max(1);
    if !g.primitive_materials.is_empty() && usize::from(material) >= materials {
        return Err(past(
            "primitive's material index",
            material.into(),
            materials,
        ));
    }
    match primitive {
        Primitive::Triangle { vertices, .. } => {
            g.primitives.push(Primitive::Triangle {
                vertices,
                neighbours: [OPEN; 3],
                edges: [OPEN_EDGE; 3],
            });
            if let Err(e) = rewire_triangle(g, index, vertices) {
                g.primitives.pop();
                return Err(e);
            }
        }
        other => g.primitives.push(other),
    }
    if !g.primitive_materials.is_empty() {
        g.primitive_materials.push(material);
    }
    Ok(index)
}

/// Vertex `vertex` of `g`, which no primitive names, deleted, every index past it one less.
pub fn delete_bound_vertex(g: &mut BoundGeometry, vertex: usize) -> Result<()> {
    has_bvh(g)?;
    if vertex >= g.vertices.len() {
        return Err(past("vertex to delete", vertex, g.vertices.len()));
    }
    if (g.primitives.iter()).any(|p| names(p).iter().any(|v| usize::from(*v) == vertex)) {
        return Err(refuse(
            "a vertex a primitive names goes only after the primitive",
        ));
    }
    g.vertices.remove(vertex);
    for p in &mut g.primitives {
        let words: &mut [u16] = match p {
            Primitive::Triangle { vertices, .. } => vertices,
            Primitive::Sphere { centre, .. } => std::slice::from_mut(centre),
            Primitive::Capsule { ends, .. } | Primitive::Cylinder { ends, .. } => ends,
            Primitive::Box(corners) => corners,
        };
        for w in words {
            if usize::from(*w) > vertex {
                *w -= 1;
            }
        }
    }
    Ok(())
}

/// A vertex at `position` added as `g`'s last; where it is.
pub fn add_bound_vertex(g: &mut BoundGeometry, position: [f32; 3]) -> Result<usize> {
    has_bvh(g)?;
    if g.vertices.len() >= MAX_VERTICES {
        return Err(refuse(
            "a geometry holds at most 32,768 vertices: a box names its corners in fifteen bits",
        ));
    }
    if position.iter().any(|x| !x.is_finite()) {
        return Err(refuse("a position is three finite numbers"));
    }
    g.vertices.push(position);
    Ok(g.vertices.len() - 1)
}

/// `b`'s box set to the union of its children's boxes placed by their matrices.
// Its centroid is kept where it lies inside, else the box's centre, and its radius raised to reach
// each placed corner; refused, changing nothing, where a child's box is not read.
pub fn fit_composite_box(b: &mut Bound) -> Result<()> {
    let mut points = Vec::new();
    for c in &b.children {
        let span = (c.bound.as_ref())
            .and_then(|s| s.as_ref().ok())
            .and_then(local_box)
            .ok_or(refuse(
                "a child whose box is not read leaves the composite's box unknown",
            ))?;
        points.extend(corners(&c.matrix, span));
    }
    let Some(&first) = points.first() else {
        return Err(refuse("a composite keeps one child at least"));
    };
    let (lo, hi) = points
        .iter()
        .fold((first, first), |s, p| union(s, (*p, *p)));
    let centroid = match (0..3).all(|a| (lo[a]..=hi[a]).contains(&b.centroid[a])) {
        true => b.centroid,
        false => std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0),
    };
    let radius = (points.iter())
        .map(|p| {
            (0..3)
                .map(|a| (p[a] - centroid[a]).powi(2))
                .sum::<f32>()
                .sqrt()
        })
        .fold(b.radius, f32::max);
    (b.box_min, b.box_max, b.centroid, b.radius) = (lo, hi, centroid, radius);
    Ok(())
}

/// Child `child` of a lone bound's composite `b` taken out, the box fitted to the rest; the child.
pub fn delete_bound_child(b: &mut Bound, child: usize) -> Result<BoundChild> {
    if child >= b.children.len() {
        return Err(past("child to delete", child, b.children.len()));
    }
    if b.children.len() == 1 {
        return Err(refuse("a composite keeps one child at least"));
    }
    let taken = b.children.remove(child);
    if let Err(e) = fit_composite_box(b) {
        b.children.insert(child, taken);
        return Err(e);
    }
    Ok(taken)
}

/// A shape's copy.
fn copied(shape: &ChildBound) -> ChildBound {
    match *shape {
        ChildBound::Geometry(ref g) => geometry_value_copied(g),
        ChildBound::Sphere { centre, radius } => ChildBound::Sphere { centre, radius },
        ChildBound::Capsule {
            box_min,
            box_max,
            radius,
        } => ChildBound::Capsule {
            box_min,
            box_max,
            radius,
        },
        ChildBound::Box { box_min, box_max } => ChildBound::Box { box_min, box_max },
        ChildBound::TaperedCapsule {
            centre,
            radii,
            length,
        } => ChildBound::TaperedCapsule {
            centre,
            radii,
            length,
        },
        ChildBound::Disc {
            centre,
            radius,
            margin,
        } => ChildBound::Disc {
            centre,
            radius,
            margin,
        },
        ChildBound::Cylinder {
            centre,
            radius,
            half_height,
        } => ChildBound::Cylinder {
            centre,
            radius,
            half_height,
        },
        ChildBound::Hull {
            centre,
            corners,
            margin,
        } => ChildBound::Hull {
            centre,
            corners,
            margin,
        },
        ChildBound::Frustum {
            centre,
            radii,
            factors,
            height,
            margin,
        } => ChildBound::Frustum {
            centre,
            radii,
            factors,
            height,
            margin,
        },
    }
}

/// Child `child` of `b` copied as `b`'s last child at its place; where the copy is.
pub fn copy_bound_child(b: &mut Bound, child: usize) -> Result<usize> {
    let c = (b.children.get(child)).ok_or(past("child to copy", child, b.children.len()))?;
    if b.children.len() >= usize::from(u16::MAX) {
        return Err(refuse("a composite counts its children in sixteen bits"));
    }
    let shape = (c.bound.as_ref())
        .and_then(|s| s.as_ref().ok())
        .map(copied)
        .ok_or(refuse(NOT_COPIED))?;
    let copy = BoundChild {
        matrix: c.matrix,
        flags: c.flags,
        bound: Some(Ok(shape)),
        material_id: c.material_id,
    };
    b.children.push(copy);
    Ok(b.children.len() - 1)
}

/// A primitive shape's type byte; `None` for a geometry.
pub(super) fn kind(shape: &ChildBound) -> Option<u8> {
    Some(match shape {
        ChildBound::Geometry(_) => return None,
        ChildBound::Sphere { .. } => 0,
        ChildBound::Capsule { .. } => 1,
        ChildBound::TaperedCapsule { .. } => 2,
        ChildBound::Box { .. } => 3,
        ChildBound::Disc { .. } => 8,
        ChildBound::Cylinder { .. } => 9,
        ChildBound::Hull { .. } => 13,
        ChildBound::Frustum { .. } => 14,
    })
}
/// The primitive of type `kind` the primitive child `shape` becomes, filling its box.
// About the same centre; refused for a hull and a frustum, whose own fields no box gives.
pub fn retyped_bound_child(shape: &ChildBound, kind: u8) -> Result<ChildBound> {
    let primitive = self::kind(shape).and_then(|_| local_box(shape));
    let (lo, hi) = primitive.ok_or(refuse(ONLY_PRIMITIVE))?;
    let centre: [f32; 3] = std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0);
    let h: [f32; 3] = std::array::from_fn(|a| (hi[a] - lo[a]) / 2.0);
    let round = h[0].min(h[2]);
    let about = |half: [f32; 3]| -> ([f32; 3], [f32; 3]) {
        (
            std::array::from_fn(|a| centre[a] - half[a]),
            std::array::from_fn(|a| centre[a] + half[a]),
        )
    };
    Ok(match kind {
        0 => ChildBound::Sphere {
            centre,
            radius: h[0].max(h[1]).max(h[2]),
        },
        1 => {
            let (box_min, box_max) = about([round, h[1], round]);
            ChildBound::Capsule {
                box_min,
                box_max,
                radius: round,
            }
        }
        2 => ChildBound::TaperedCapsule {
            centre,
            radii: [round; 2],
            length: (2.0 * (h[1] - round)).max(0.0),
        },
        3 => ChildBound::Box {
            box_min: lo,
            box_max: hi,
        },
        8 => ChildBound::Disc {
            centre,
            radius: h[1].max(h[2]),
            margin: h[0],
        },
        9 => ChildBound::Cylinder {
            centre,
            radius: h[0].max(h[2]),
            half_height: h[1],
        },
        _ => {
            return Err(refuse(
                "a child is given the type of a sphere, capsule, tapered capsule, box, disc or cylinder: a hull's corners and a frustum's factors no box gives",
            ))
        }
    })
}
