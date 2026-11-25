//! A bound's collision reshaped where its arrays change length: a primitive or a vertex of a
//! geometry with a BVH added or deleted, a child of a lone bound's or a drawable's composite copied
//! or deleted, a primitive child given another type. What goes is taken out where it stands and the bytes after
//! the array's new end are zeroed; what comes is laid out past the virtual section's end with the
//! array it joins, so nothing already in the section moves. A geometry's +0x148 runs, which give
//! each primitive an entry of its +0x138 key table, follow a primitive added or deleted.

use std::cmp::Ordering;

use crate::container::rpf::{grow_section, Rpf9GrowError, Rpf9Section, Rpf9Sections};

use super::bound::{bound_at, child_bound, COMPOSITE, GEOMETRY};
use super::bytes::sections;
use super::write_bound_runs::{
    geometry_copied, geometry_value_copied, runs_after_add, runs_after_delete, NOT_COPIED,
};
use super::write_bound_vertex::item_box;
use super::write_bvh::{corners, refit_composite, tree, union, Span, NODE};
use super::write_physics::{local_box, put, put_shape, refuse};
use super::{
    read_bound, read_drawable, read_drawable_with_bound, rewire_triangle, Bound, BoundChild,
    BoundGeometry, ChildBound, HeaderlessError, Primitive, Result,
};

/// The word that addresses virtual offset 0.
const VIRTUAL: u64 = 0x5000_0000;
/// No neighbour across an edge.
const OPEN: u16 = 0xFFFF;
/// The byte of an edge with no neighbour.
const OPEN_EDGE: u8 = 0xFF;
/// The most primitives a geometry's BVH leaf holds.
const LEAF_ITEMS: usize = 4;
/// A box primitive names its corners in fifteen bits.
const MAX_VERTICES: usize = 0x8000;
/// A fragment child record's length.
const FRAGMENT_CHILD: usize = 0x50;
const NO_BVH: &str = "only a geometry with a BVH is reshaped: one with none carries a shrunk copy of each vertex, and how far a vertex is shrunk is not known";
const ONLY_PRIMITIVE: &str =
    "only a primitive child is copied or given another type: a geometry's arrays are its own";

/// Why a bound's collision is not reshaped.
#[derive(Debug, thiserror::Error)]
pub enum BoundGrowError {
    #[error(transparent)]
    Read(#[from] HeaderlessError),
    #[error(transparent)]
    Grow(#[from] Rpf9GrowError),
}

type Grown<T> = std::result::Result<T, BoundGrowError>;

/// Which bound of a body an edit names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundHolder {
    /// The body is the bound.
    Bound,
    /// The bound a drawable's root points at from 0xC8.
    Drawable,
    /// The composite a fragment's record at 0x10 holds.
    Fragment,
}

impl BoundHolder {
    fn at(self, s: &Rpf9Sections) -> Result<usize> {
        match self {
            Self::Bound => Ok(0),
            Self::Drawable => sections(&s.virtual_section, Some(&[]), |v, _| v.to("bound", 0xC8)),
            Self::Fragment => sections(&s.virtual_section, Some(&[]), |v, _| {
                v.to("bound", v.to("record that holds the bound", 0x10)? + 0x20)
            }),
        }
    }

    /// The bound this names in `s`.
    pub fn read(self, s: &Rpf9Sections) -> Result<Bound> {
        let (v, p) = (&s.virtual_section[..], &s.physical[..]);
        let none = || refuse("the body holds no bound");
        match self {
            Self::Bound => read_bound(v),
            Self::Drawable => read_drawable(v, Some(p))?.bound.ok_or_else(none),
            Self::Fragment => read_drawable_with_bound(v, Some(p))?.bound.ok_or_else(none),
        }
    }
}

/// A count change a save makes again on the body read again, each index as the body holds it then.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoundReshape {
    DeletePrimitive {
        child: usize,
        primitive: usize,
    },
    /// `record` added as the geometry's last primitive, its material index `material`.
    AddPrimitive {
        child: usize,
        record: Primitive,
        material: u8,
    },
    DeleteVertex {
        child: usize,
        vertex: usize,
    },
    /// A vertex added last at the quantisation centre, which the save's vertex move places.
    AddVertex {
        child: usize,
    },
    DeleteChild {
        child: usize,
    },
    /// A primitive child copied as the composite's last at its place.
    CopyChild {
        child: usize,
    },
}

fn past(what: &'static str, index: usize, count: usize) -> HeaderlessError {
    HeaderlessError::IndexPastCount {
        what,
        index: index as u64,
        count: count as u64,
    }
}

fn has_bvh(g: &BoundGeometry) -> Result<()> {
    match g.bvh {
        Some(_) => Ok(()),
        None => Err(refuse(NO_BVH)),
    }
}

/// The vertices `p` names.
fn names(p: &Primitive) -> Vec<u16> {
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
fn kind(shape: &ChildBound) -> Option<u8> {
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

/// The bytes of a primitive child's record of type `kind`, as the reader lays each out.
fn record_size(kind: u8) -> Option<usize> {
    match kind {
        0 | 3 => Some(0x70),
        1 | 8 | 9 => Some(0x80),
        2 | 14 => Some(0x90),
        13 => Some(0x100),
        _ => None,
    }
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

/// Refuses a child edit unless `holder` names a composite laid out as a lone bound's.
// A fragment's composite children are also its child records. A drawable's composite passes where
// its counts at +0x98 and +0x9A agree and +0x80 names +0x78's matrices, as a lone bound's do.
fn child_holder(s: &Rpf9Sections, holder: BoundHolder) -> Result<()> {
    if holder == BoundHolder::Bound {
        return Ok(());
    }
    if holder == BoundHolder::Fragment {
        return Err(refuse(
            "a child of a fragment's composite is also a fragment child record, so it is not added or deleted",
        ));
    }
    let at = holder.at(s)?;
    let lone = sections(&s.virtual_section, Some(&[]), |b, _| {
        if b.u8("bound type", at + 0x10)? != COMPOSITE {
            return Ok(false);
        }
        let counts = b.u16("child count", at + 0x98)? == b.u16("child count", at + 0x9A)?;
        let matrices = b.u64("child matrices", at + 0x78)? == b.u64("child matrices", at + 0x80)?;
        Ok(counts && matrices)
    })?;
    match lone {
        true => Ok(()),
        false => Err(refuse(
            "only a composite whose arrays are laid out as a lone bound's has a child added or deleted",
        )),
    }
}

/// `op` made to the bound `holder` names in `s`, or refused with `s` unchanged.
pub fn reshape_bound_body(
    s: &mut Rpf9Sections,
    holder: BoundHolder,
    op: &BoundReshape,
) -> Grown<()> {
    let child_op = matches!(
        op,
        BoundReshape::DeleteChild { .. } | BoundReshape::CopyChild { .. }
    );
    if child_op {
        child_holder(s, holder)?;
    }
    let mut work = s.clone();
    let at = holder.at(&work)?;
    match *op {
        BoundReshape::DeletePrimitive { child, primitive } => {
            let c = child_record(&work.virtual_section, at, child)?;
            primitive_deleted(&mut work.virtual_section, c, primitive)?;
            runs_after_delete(&mut work, c, primitive)?;
        }
        BoundReshape::AddPrimitive {
            child,
            record,
            material,
        } => {
            let c = child_record(&work.virtual_section, at, child)?;
            primitive_added(&mut work, c, &record, material)?;
            runs_after_add(&mut work, c)?;
        }
        BoundReshape::DeleteVertex { child, vertex } => {
            let c = child_record(&work.virtual_section, at, child)?;
            vertex_deleted(&mut work.virtual_section, c, vertex)?;
        }
        BoundReshape::AddVertex { child } => {
            let c = child_record(&work.virtual_section, at, child)?;
            vertex_added(&mut work, c)?;
        }
        BoundReshape::DeleteChild { child } => {
            child_deleted(&mut work.virtual_section, at, child)?;
        }
        BoundReshape::CopyChild { child } => child_copied(&mut work, at, child)?,
    }
    holder.read(&work)?;
    *s = work;
    Ok(())
}

/// Each primitive child `edited` gives another type than the body's laid out anew past the end.
// At its type's size, its shape written and its pointer aimed there, and the composite's BVH
// fitted again over it; refused for a lone bound, whose record the body points at.
pub fn write_retyped(s: &mut Rpf9Sections, holder: BoundHolder, edited: &Bound) -> Grown<()> {
    let read = holder.read(s)?;
    if read.children.len() != edited.children.len() {
        return Err(refuse("the bound holds another number of children").into());
    }
    let shape = |c: &BoundChild| -> Option<u8> { kind(c.bound.as_ref()?.as_ref().ok()?) };
    let changed: Vec<usize> = (0..read.children.len())
        .filter(|&i| {
            let (r, e) = (shape(&read.children[i]), shape(&edited.children[i]));
            r.is_some() && e.is_some() && r != e
        })
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    let at = holder.at(s)?;
    let composite = sections(&s.virtual_section, Some(&[]), |v, _| {
        Ok(v.u8("bound type", at + 0x10)? == COMPOSITE)
    })?;
    if !composite {
        return Err(refuse("a lone bound keeps its type: the body points at its record").into());
    }
    let pointers = sections(&s.virtual_section, Some(&[]), |v, _| {
        v.to("child pointers", at + 0x70)
    })?;
    // A fragment's child records each name their bound again at +0x30, which must follow it.
    let records = match holder {
        BoundHolder::Fragment => sections(&s.virtual_section, Some(&[]), |v, _| {
            Ok(Some((
                v.to("children", 0x28)?,
                usize::from(v.u16("child count", 0x36)?),
            )))
        })?,
        _ => None,
    };
    let mut mixed = s.clone();
    for i in changed {
        let new = (edited.children[i].bound.as_ref())
            .and_then(|b| b.as_ref().ok())
            .ok_or(refuse(ONLY_PRIMITIVE))?;
        let to = kind(new).ok_or(refuse(ONLY_PRIMITIVE))?;
        let size = record_size(to).ok_or(refuse(ONLY_PRIMITIVE))?;
        let old = child_record(&mixed.virtual_section, at, i)?;
        let base = slice(&mixed.virtual_section, old, 0x70)?.to_vec();
        let word = sections(&mixed.virtual_section, Some(&[]), |v, _| {
            v.u64("composite's child", pointers + 8 * i)
        })?;
        let places = room(&mut mixed, &[size])?;
        let v = &mut mixed.virtual_section;
        put(v, places[0], &base)?;
        put(v, places[0] + 0x10, &[to])?;
        put_shape(v, places[0], new)?;
        put_ptr(v, pointers + 8 * i, places[0])?;
        if let Some((children, count)) = records {
            for r in 0..count {
                let bound = children + FRAGMENT_CHILD * r + 0x30;
                if slice(v, bound, 8)? == word.to_le_bytes() {
                    put_ptr(v, bound, places[0])?;
                }
            }
        }
    }
    refit_composite(&mut mixed.virtual_section, at, &read, edited, false)?;
    holder.read(&mixed)?;
    *s = mixed;
    Ok(())
}

/// The bytes `len` long at `at` of the virtual section `v`.
fn slice(v: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    (at.checked_add(len))
        .and_then(|end| v.get(at..end))
        .ok_or(HeaderlessError::PastEnd {
            what: "reshaped array",
            at: at as u64,
            section: "virtual",
            len: v.len() as u64,
        })
}

fn put_ptr(v: &mut [u8], at: usize, to: usize) -> Result<()> {
    put(v, at, &(VIRTUAL + to as u64).to_le_bytes())
}

/// Zeroed room for each of `lens` past the virtual section's end, each 16-aligned; where each
/// starts.
pub(super) fn room(s: &mut Rpf9Sections, lens: &[usize]) -> Grown<Vec<usize>> {
    let total: usize = lens.iter().map(|l| l.next_multiple_of(16)).sum();
    let mut grown = s.clone();
    let end = grown.virtual_section.len().next_multiple_of(16);
    grown.virtual_section.resize(end, 0);
    let mut at = grow_section(
        &mut grown,
        &[0x08],
        Rpf9Section::Virtual,
        end,
        total as isize,
    )?;
    *s = grown;
    Ok(lens
        .iter()
        .map(|l| {
            let here = at;
            at += l.next_multiple_of(16);
            here
        })
        .collect())
}

/// The `count` elements of `stride` bytes at `at` with element `gone` taken out, the freed last
/// one zeroed.
fn shift(v: &mut [u8], at: usize, stride: usize, count: usize, gone: usize) -> Result<()> {
    let mut bytes = slice(v, at, stride * count)?.to_vec();
    bytes.drain(stride * gone..stride * (gone + 1));
    bytes.resize(stride * count, 0);
    put(v, at, &bytes)
}

/// Where child `child` of the bound at `at` lies: the bound itself where it is no composite.
fn child_record(v: &[u8], at: usize, child: usize) -> Result<usize> {
    sections(v, Some(&[]), |b, _| {
        if b.u8("bound type", at + 0x10)? != COMPOSITE {
            return match child {
                0 => Ok(at),
                _ => Err(past("child", child, 1)),
            };
        }
        let count = usize::from(b.u16("child count", at + 0x98)?);
        if child >= count {
            return Err(past("child", child, count));
        }
        b.to("child", b.to("child pointers", at + 0x70)? + 8 * child)
    })
}

/// The geometry child at `at` as the reader reads it.
fn geometry(v: &[u8], at: usize) -> Result<BoundGeometry> {
    match sections(v, Some(&[]), |b, _| child_bound(b, at))?.0 {
        ChildBound::Geometry(g) => Ok(g),
        _ => Err(refuse(
            "only a geometry child's primitives and vertices are added or deleted",
        )),
    }
}

/// Where a geometry with a BVH holds its arrays, and their counts.
struct Layout {
    vertices: usize,
    vertex_count: usize,
    primitives: usize,
    primitive_count: usize,
    materials: Option<usize>,
    bvh: usize,
}

/// The arrays of the geometry child at `at`, once the reader reads it.
fn layout(v: &[u8], at: usize) -> Result<Layout> {
    geometry(v, at)?;
    sections(v, Some(&[]), |b, _| {
        if b.u8("bound type", at + 0x10)? != GEOMETRY {
            return Err(refuse(NO_BVH));
        }
        Ok(Layout {
            vertices: b.to("vertices", at + 0xB0)?,
            vertex_count: b.u32("vertex count", at + 0xD0)? as usize,
            primitives: b.to("primitives", at + 0x88)?,
            primitive_count: b.u32("primitive count", at + 0xD4)? as usize,
            materials: b.ptr("per-primitive material indices", at + 0x118)?,
            bvh: b.to("BVH", at + 0x130)?,
        })
    })
}

/// The 16 bytes primitive `p` is stored as.
fn record(p: &Primitive) -> [u8; 16] {
    let mut r = [0u8; 16];
    let (kind, words, radius): (u8, Vec<(usize, u16)>, Option<f32>) = match *p {
        Primitive::Triangle {
            vertices,
            neighbours,
            edges,
        } => {
            r[..3].copy_from_slice(&edges);
            let words: Vec<(usize, u16)> = (vertices.iter().chain(&neighbours).enumerate())
                .map(|(k, w)| (4 + 2 * k, *w))
                .collect();
            (0, words, None)
        }
        Primitive::Sphere { centre, radius } => (1, vec![(0, centre)], Some(radius)),
        Primitive::Capsule { ends, radius } => (2, vec![(0, ends[0]), (8, ends[1])], Some(radius)),
        Primitive::Box(c) => {
            let words: Vec<(usize, u16)> =
                c.iter().enumerate().map(|(k, w)| (4 + 2 * k, *w)).collect();
            (3, words, None)
        }
        Primitive::Cylinder { ends, radius } => (4, vec![(0, ends[0]), (8, ends[1])], Some(radius)),
    };
    r[3] = kind;
    for (o, w) in words {
        r[o..o + 2].copy_from_slice(&w.to_le_bytes());
    }
    if let Some(x) = radius {
        r[4..8].copy_from_slice(&x.to_le_bytes());
    }
    r
}

/// The vertex index words of a stored primitive record, by offset, and the bits an index holds.
fn vertex_words(record: &[u8; 16]) -> (&'static [usize], u16) {
    const TRIANGLE: &[usize] = &[4, 6, 8];
    const CENTRE: &[usize] = &[0];
    const ENDS: &[usize] = &[0, 8];
    const CORNERS: &[usize] = &[4, 6, 8, 0xA];
    const NONE: &[usize] = &[];
    match record[3] {
        0 => (TRIANGLE, 0xFFFF),
        1 => (CENTRE, 0xFFFF),
        2 | 4 => (ENDS, 0xFFFF),
        3 => (CORNERS, 0x7FFF),
        _ => (NONE, 0xFFFF),
    }
}

fn word(n: &[u8; NODE], o: usize) -> u16 {
    u16::from_le_bytes([n[o], n[o + 1]])
}

fn set_word(n: &mut [u8; NODE], o: usize, w: u16) {
    n[o..o + 2].copy_from_slice(&w.to_le_bytes());
}

fn nodes_of(v: &[u8], bvh: usize) -> Result<Vec<[u8; NODE]>> {
    sections(v, Some(&[]), |b, _| {
        let count = b.u32("BVH node count", bvh + 0x58)? as usize;
        let at = b.to("BVH nodes", bvh + 0x50)?;
        Ok(b.array("BVH nodes", at, count, NODE)?
            .as_chunks::<NODE>()
            .0
            .to_vec())
    })
}

/// Each node's parent, from the subtree counts the preorder states.
fn parents(nodes: &[[u8; NODE]]) -> Result<Vec<Option<usize>>> {
    let shape = || refuse("the BVH's nodes are not in the preorder their subtree counts state");
    let size = |i: usize| -> Result<usize> {
        let n = nodes.get(i).ok_or_else(shape)?;
        Ok(match word(n, 0xE) {
            0 => usize::from(word(n, 0xC)),
            _ => 1,
        })
    };
    if !nodes.is_empty() && size(0)? != nodes.len() {
        return Err(shape());
    }
    let mut parent = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        if word(n, 0xE) != 0 {
            continue;
        }
        let left = i + 1;
        let right = left + size(left)?;
        if 1 + size(left)? + size(right)? != size(i)? {
            return Err(shape());
        }
        parent[left] = Some(i);
        parent[right] = Some(i);
    }
    Ok(parent)
}

/// The subtree count of every node above `from` moved by `by`.
fn ancestors_by(
    nodes: &mut [[u8; NODE]],
    parent: &[Option<usize>],
    from: usize,
    by: isize,
) -> Result<()> {
    let mut at = parent.get(from).copied().flatten();
    while let Some(p) = at {
        let n = nodes
            .get_mut(p)
            .ok_or(refuse("a BVH node's parent is not one of its nodes"))?;
        let size = (usize::from(word(n, 0xC)).checked_add_signed(by))
            .and_then(|s| u16::try_from(s).ok())
            .ok_or(refuse("a BVH would hold more nodes than its words count"))?;
        set_word(n, 0xC, size);
        at = parent.get(p).copied().flatten();
    }
    Ok(())
}

/// `nodes` with item `item` taken out of the leaf that holds it and every later leaf's first item
/// one less; a leaf left empty goes, its parent replaced by its sibling. The first item of the leaf
/// that lost one, where it stays.
fn without_item(nodes: &mut Vec<[u8; NODE]>, item: usize) -> Result<Option<usize>> {
    let parent = parents(nodes)?;
    let (mut emptied, mut kept) = (None, None);
    for (i, n) in nodes.iter_mut().enumerate() {
        let (first, count) = (usize::from(word(n, 0xC)), usize::from(word(n, 0xE)));
        if count == 0 {
            continue;
        }
        if first > item {
            set_word(n, 0xC, (first - 1) as u16);
        } else if item < first + count {
            set_word(n, 0xE, (count - 1) as u16);
            match count {
                1 => emptied = Some(i),
                _ => kept = Some(first),
            }
        }
    }
    let Some(leaf) = emptied else {
        return match kept {
            Some(first) => Ok(Some(first)),
            None => Err(refuse("no BVH leaf holds the item")),
        };
    };
    let p = parent
        .get(leaf)
        .copied()
        .flatten()
        .ok_or(refuse("a BVH of one leaf keeps its last item"))?;
    ancestors_by(nodes, &parent, p, -2)?;
    // A parent comes before its children, so the sibling's subtree takes the parent's place.
    nodes.remove(leaf);
    nodes.remove(p);
    Ok(None)
}

/// `nodes` with item `item` added to the last leaf of the preorder where it holds fewer than `fill`
/// and ends at `item`, else in a leaf of its own beside that leaf under a new parent; the first
/// item of the leaf that holds it.
fn with_item(nodes: &mut Vec<[u8; NODE]>, item: usize, fill: usize) -> Result<usize> {
    let parent = parents(nodes)?;
    let last = nodes
        .len()
        .checked_sub(1)
        .ok_or(refuse("the BVH has no node"))?;
    let leaf = nodes[last];
    let (first, count) = (usize::from(word(&leaf, 0xC)), usize::from(word(&leaf, 0xE)));
    if count == 0 {
        return Err(refuse("the BVH's last node is no leaf"));
    }
    if fill > 1 && first + count != item {
        return Err(refuse(
            "the BVH's last leaf does not end at the last primitive, so the leaves are not in the primitives' order",
        ));
    }
    let named = u16::try_from(item)
        .ok()
        .filter(|w| *w != OPEN)
        .ok_or(refuse("an item past the 65,535th is named by no leaf"))?;
    if count < fill {
        set_word(&mut nodes[last], 0xE, (count + 1) as u16);
        return Ok(first);
    }
    if nodes.len() + 2 > usize::from(u16::MAX) {
        return Err(refuse("a BVH would hold more nodes than its words count"));
    }
    ancestors_by(nodes, &parent, last, 2)?;
    let mut inner = leaf;
    set_word(&mut inner, 0xC, 3);
    set_word(&mut inner, 0xE, 0);
    let mut own = leaf;
    set_word(&mut own, 0xC, named);
    set_word(&mut own, 0xE, 1);
    nodes[last] = inner;
    nodes.push(leaf);
    nodes.push(own);
    Ok(item)
}

/// The BVH record at `bvh`'s node count, and the count GTA V allocates over `items` primitives, or
/// for a composite's BVH at least the nodes.
fn put_counts(v: &mut [u8], bvh: usize, nodes: usize, items: Option<usize>) -> Result<()> {
    put(v, bvh + 0x58, &(nodes as u32).to_le_bytes())?;
    let allocated = match items {
        Some(items) => nodes.max(items / 2 + 1),
        None => nodes.max(sections(v, Some(&[]), |b, _| {
            b.u32("BVH allocated nodes", bvh + 0x5C)
        })? as usize),
    };
    put(v, bvh + 0x5C, &(allocated as u32).to_le_bytes())
}

/// `nodes` written where the BVH record at `bvh` holds `held` of them, the rest zeroed.
fn nodes_in_place(
    v: &mut [u8],
    bvh: usize,
    nodes: &[[u8; NODE]],
    held: usize,
    items: Option<usize>,
) -> Result<()> {
    let at = sections(v, Some(&[]), |b, _| b.to("BVH nodes", bvh + 0x50))?;
    let mut bytes = nodes.as_flattened().to_vec();
    bytes.resize(NODE * held.max(nodes.len()), 0);
    put(v, at, &bytes)?;
    put_counts(v, bvh, nodes.len(), items)
}

/// The vertices of the geometry child at `at` decoded as the engine's f32 multiply-add, and its
/// margin.
fn decoded(v: &[u8], at: usize) -> Result<(Vec<[f32; 3]>, f32)> {
    sections(v, Some(&[]), |b, _| {
        let count = b.u32("vertex count", at + 0xD0)? as usize;
        let scale = b.vec3("quantisation scale", at + 0x90)?;
        let centre = b.vec3("quantisation centre", at + 0xA0)?;
        let q = b.array("vertices", b.to("vertices", at + 0xB0)?, count, 6)?;
        let vertices = (q.as_chunks::<6>().0.iter())
            .map(|q| {
                std::array::from_fn(|a| {
                    let lane = i16::from_le_bytes([q[2 * a], q[2 * a + 1]]);
                    (f64::from(centre[a]) + f64::from(lane) * f64::from(scale[a])) as f32
                })
            })
            .collect();
        Ok((vertices, b.f32("margin", at + 0x2C)?))
    })
}

/// The BVH at `bvh` of the geometry child at `at` fitted again: the leaf whose first item is
/// `leaf` boxed over its primitives, every parent the union of its two.
fn refit_leaf(v: &mut [u8], at: usize, bvh: usize, leaf: Option<usize>) -> Result<()> {
    let g = geometry(v, at)?;
    let (vertices, margin) = decoded(v, at)?;
    let tree = tree(v, bvh)?;
    let at_vertex = |i: u16| vertices.get(usize::from(i)).copied();
    let boxes = tree.refit(|first, items| {
        if Some(first) != leaf {
            return Ok(None);
        }
        let held = (g.primitives.get(first..first + items))
            .ok_or(refuse("a BVH leaf names a primitive past the primitives"))?;
        let mut span: Option<Span> = None;
        for p in held {
            let b = item_box(p, &at_vertex, margin)
                .ok_or(refuse("a primitive names a vertex past the vertices"))?;
            span = Some(span.map_or(b, |s| union(s, b)));
        }
        Ok(span)
    })?;
    tree.write(v, &boxes)
}

/// The BVH at `bvh`'s parents fitted again as the union of their two children.
fn parents_fitted(v: &mut [u8], bvh: usize) -> Result<()> {
    let tree = tree(v, bvh)?;
    let boxes = tree.refit(|_, _| Ok(None))?;
    tree.write(v, &boxes)
}

fn primitive_deleted(v: &mut [u8], at: usize, p: usize) -> Result<()> {
    let l = layout(v, at)?;
    let n = l.primitive_count;
    if p >= n {
        return Err(past("primitive to delete", p, n));
    }
    if n == 1 {
        return Err(refuse("a geometry keeps one primitive at least"));
    }
    shift(v, l.primitives, 16, n, p)?;
    let mut records = slice(v, l.primitives, 16 * (n - 1))?.to_vec();
    for r in records.as_chunks_mut::<16>().0 {
        if r[3] != 0 {
            continue;
        }
        for k in 0..3 {
            let o = 0xA + 2 * k;
            let w = u16::from_le_bytes([r[o], r[o + 1]]);
            if w == OPEN {
                continue;
            }
            let w = match usize::from(w).cmp(&p) {
                Ordering::Equal => {
                    r[k] = OPEN_EDGE;
                    OPEN
                }
                Ordering::Greater => w - 1,
                Ordering::Less => w,
            };
            r[o..o + 2].copy_from_slice(&w.to_le_bytes());
        }
    }
    put(v, l.primitives, &records)?;
    if let Some(m) = l.materials {
        shift(v, m, 1, n, p)?;
    }
    put(v, at + 0xD4, &((n - 1) as u32).to_le_bytes())?;
    let mut nodes = nodes_of(v, l.bvh)?;
    let held = nodes.len();
    let leaf = without_item(&mut nodes, p)?;
    nodes_in_place(v, l.bvh, &nodes, held, Some(n - 1))?;
    refit_leaf(v, at, l.bvh, leaf)
}

fn primitive_added(s: &mut Rpf9Sections, at: usize, p: &Primitive, material: u8) -> Grown<()> {
    let l = layout(&s.virtual_section, at)?;
    let n = l.primitive_count;
    if n + 1 >= usize::from(OPEN) {
        return Err(refuse("a primitive past the 65,535th is named by no neighbour").into());
    }
    let mut records = slice(&s.virtual_section, l.primitives, 16 * n)?.to_vec();
    records.extend(record(p));
    let list = match l.materials {
        Some(m) => {
            let mut list = slice(&s.virtual_section, m, n)?.to_vec();
            list.push(material);
            Some(list)
        }
        None => None,
    };
    let mut nodes = nodes_of(&s.virtual_section, l.bvh)?;
    let held = nodes.len();
    let leaf = with_item(&mut nodes, n, LEAF_ITEMS)?;
    let grew = nodes.len() > held;
    let mut lens = vec![records.len()];
    lens.extend(list.as_ref().map(Vec::len));
    if grew {
        lens.push(NODE * nodes.len());
    }
    let places = room(s, &lens)?;
    let v = &mut s.virtual_section;
    put(v, places[0], &records)?;
    put_ptr(v, at + 0x88, places[0])?;
    let mut next = 1;
    if let Some(list) = &list {
        put(v, places[next], list)?;
        put_ptr(v, at + 0x118, places[next])?;
        next += 1;
    }
    put(v, at + 0xD4, &((n + 1) as u32).to_le_bytes())?;
    match grew {
        true => {
            put(v, places[next], nodes.as_flattened())?;
            put_ptr(v, l.bvh + 0x50, places[next])?;
            put_counts(v, l.bvh, nodes.len(), Some(n + 1))?;
        }
        false => nodes_in_place(v, l.bvh, &nodes, held, Some(n + 1))?,
    }
    refit_leaf(v, at, l.bvh, Some(leaf))?;
    Ok(())
}

/// The vertex count at +0xD0 written as `now`, and the hull's count at +0x84 where it was the
/// vertex count, as in every measured geometry with a BVH.
fn vertex_counts(v: &mut [u8], at: usize, was: usize, now: usize) -> Result<()> {
    put(v, at + 0xD0, &(now as u32).to_le_bytes())?;
    let hull = sections(v, Some(&[]), |b, _| {
        b.u16("convex hull vertex count", at + 0x84)
    })?;
    if usize::from(hull) == was {
        put(v, at + 0x84, &(now as u16).to_le_bytes())?;
    }
    Ok(())
}

fn vertex_deleted(v: &mut [u8], at: usize, vertex: usize) -> Result<()> {
    let l = layout(v, at)?;
    let n = l.vertex_count;
    if vertex >= n {
        return Err(past("vertex to delete", vertex, n));
    }
    let g = geometry(v, at)?;
    if (g.primitives.iter()).any(|p| names(p).iter().any(|w| usize::from(*w) == vertex)) {
        return Err(refuse(
            "a vertex a primitive names goes only after the primitive",
        ));
    }
    shift(v, l.vertices, 6, n, vertex)?;
    let mut records = slice(v, l.primitives, 16 * l.primitive_count)?.to_vec();
    for r in records.as_chunks_mut::<16>().0 {
        let (words, mask) = vertex_words(r);
        for &o in words {
            let w = u16::from_le_bytes([r[o], r[o + 1]]);
            if usize::from(w & mask) > vertex {
                r[o..o + 2].copy_from_slice(&(w - 1).to_le_bytes());
            }
        }
    }
    put(v, l.primitives, &records)?;
    vertex_counts(v, at, n, n - 1)
}

fn vertex_added(s: &mut Rpf9Sections, at: usize) -> Grown<()> {
    let l = layout(&s.virtual_section, at)?;
    let n = l.vertex_count;
    if n >= MAX_VERTICES {
        return Err(refuse(
            "a geometry holds at most 32,768 vertices: a box names its corners in fifteen bits",
        )
        .into());
    }
    let mut vertices = slice(&s.virtual_section, l.vertices, 6 * n)?.to_vec();
    vertices.extend([0u8; 6]);
    let places = room(s, &[vertices.len()])?;
    let v = &mut s.virtual_section;
    put(v, places[0], &vertices)?;
    put_ptr(v, at + 0xB0, places[0])?;
    vertex_counts(v, at, n, n + 1)?;
    Ok(())
}

/// What a child edit reads of a composite.
struct Composite {
    count: usize,
    pointers: usize,
    matrices: usize,
    /// +0x80, which every measured composite aims at the +0x78 array.
    second: Option<usize>,
    flags: Option<usize>,
    bvh: Option<usize>,
}

fn composite(v: &[u8], at: usize) -> Result<Composite> {
    sections(v, Some(&[]), |b, _| bound_at(b, at))?;
    sections(v, Some(&[]), |b, _| {
        Ok(Composite {
            count: usize::from(b.u16("child count", at + 0x98)?),
            pointers: b.to("child pointers", at + 0x70)?,
            matrices: b.to("child matrices", at + 0x78)?,
            second: b.ptr("child matrices", at + 0x80)?,
            flags: b.ptr("child type and include entries", at + 0x88)?,
            bvh: b.ptr("composite BVH", at + 0xA0)?,
        })
    })
}

fn child_deleted(v: &mut [u8], at: usize, child: usize) -> Result<()> {
    let c = composite(v, at)?;
    if child >= c.count {
        return Err(past("child to delete", child, c.count));
    }
    if c.count == 1 {
        return Err(refuse("a composite keeps one child at least"));
    }
    shift(v, c.pointers, 8, c.count, child)?;
    shift(v, c.matrices, 64, c.count, child)?;
    if let Some(second) = c.second.filter(|m| *m != c.matrices) {
        shift(v, second, 64, c.count, child)?;
    }
    if let Some(flags) = c.flags {
        shift(v, flags, 16, c.count, child)?;
    }
    let count = ((c.count - 1) as u16).to_le_bytes();
    put(v, at + 0x98, &count)?;
    put(v, at + 0x9A, &count)?;
    if let Some(bvh) = c.bvh {
        let mut nodes = nodes_of(v, bvh)?;
        let held = nodes.len();
        without_item(&mut nodes, child)?;
        nodes_in_place(v, bvh, &nodes, held, None)?;
        parents_fitted(v, bvh)?;
    }
    Ok(())
}

fn child_copied(s: &mut Rpf9Sections, at: usize, child: usize) -> Grown<()> {
    let v = &s.virtual_section;
    let c = composite(v, at)?;
    let n = c.count;
    if child >= n {
        return Err(past("child to copy", child, n).into());
    }
    if n + 1 >= usize::from(u16::MAX) {
        return Err(refuse("a composite counts its children in sixteen bits").into());
    }
    let source = child_record(v, at, child)?;
    let kind = sections(v, Some(&[]), |b, _| b.u8("bound type", source + 0x10))?;
    let geometry = geometry_copied(s, source)?;
    let v = &s.virtual_section;
    let copy = match geometry {
        Some(record) => record,
        None => slice(v, source, record_size(kind).ok_or(refuse(ONLY_PRIMITIVE))?)?.to_vec(),
    };
    let size = copy.len();
    let mut pointers = slice(v, c.pointers, 8 * n)?.to_vec();
    let mut matrices = slice(v, c.matrices, 64 * n)?.to_vec();
    matrices.extend_from_within(64 * child..64 * (child + 1));
    let flags = match c.flags {
        Some(f) => {
            let mut flags = slice(v, f, 16 * n)?.to_vec();
            flags.extend_from_within(16 * child..16 * (child + 1));
            Some(flags)
        }
        None => None,
    };
    let nodes = match c.bvh {
        Some(bvh) => {
            let mut nodes = nodes_of(v, bvh)?;
            let boxed = (nodes.iter())
                .find(|node| word(node, 0xE) == 1 && usize::from(word(node, 0xC)) == child)
                .map(|node| {
                    let mut b = [0u8; 12];
                    b.copy_from_slice(&node[..12]);
                    b
                })
                .ok_or(refuse(
                    "no leaf of the composite's BVH names the child alone",
                ))?;
            with_item(&mut nodes, n, 1)?;
            if let Some(last) = nodes.last_mut() {
                last[..12].copy_from_slice(&boxed);
            }
            Some((bvh, nodes))
        }
        None => None,
    };
    let mut lens = vec![size, 8 * (n + 1), matrices.len()];
    lens.extend(flags.as_ref().map(Vec::len));
    lens.extend(nodes.as_ref().map(|(_, nodes)| NODE * nodes.len()));
    let places = room(s, &lens)?;
    let v = &mut s.virtual_section;
    put(v, places[0], &copy)?;
    pointers.extend((VIRTUAL + places[0] as u64).to_le_bytes());
    put(v, places[1], &pointers)?;
    put_ptr(v, at + 0x70, places[1])?;
    // One array of matrices, which both words aim at.
    put(v, places[2], &matrices)?;
    put_ptr(v, at + 0x78, places[2])?;
    if c.second.is_some() {
        put_ptr(v, at + 0x80, places[2])?;
    }
    let mut next = 3;
    if let Some(flags) = &flags {
        put(v, places[next], flags)?;
        put_ptr(v, at + 0x88, places[next])?;
        next += 1;
    }
    let count = ((n + 1) as u16).to_le_bytes();
    put(v, at + 0x98, &count)?;
    put(v, at + 0x9A, &count)?;
    if let Some((bvh, nodes)) = &nodes {
        put(v, places[next], nodes.as_flattened())?;
        put_ptr(v, bvh + 0x50, places[next])?;
        put_counts(v, *bvh, nodes.len(), None)?;
        parents_fitted(v, *bvh)?;
    }
    Ok(())
}
