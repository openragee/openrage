//! A scanned GTA VI cloth record in the model view: a type-1 record's element pairs, a type-2
//! record's edges, a type-3 record's strands and the triangles over their roots, drawn as a
//! surface and as lines, each root marked as the point its strand is held by, a type-4 record's
//! box turned by its rotation, and a type-5 record's shapes each as the box it states.

use std::sync::Arc;

use glam::{Quat, Vec3};
use or_io::container::rpf::{Rpf9Resource, Rpf9Role, Rpf9Signature};
use or_io::gta6::headerless::{read_cloth, Cloth, ClothKind, Resource};
use or_renderer::{LineVertex, MeshVertex};

use super::meshes::{face_normals, MeshUpload, CLOTH_SURFACE};
use crate::state::DocPayload;

const EDGE: [f32; 4] = [0.45, 0.85, 0.90, 0.90];
const ROOT_TRIANGLE: [f32; 4] = [0.95, 0.75, 0.35, 0.95];
const PIN: [f32; 4] = [0.95, 0.35, 0.40, 1.0];
const SHAPE: [f32; 4] = [0.70, 0.90, 0.45, 0.95];

/// A scanned cloth body as the model view's document where it reads and has vertices to draw.
pub(crate) fn opened(
    signature: Rpf9Signature,
    resource: &Rpf9Resource,
    bytes: &[u8],
) -> Option<DocPayload> {
    if signature.role() != Some(Rpf9Role::Cloth) {
        return None;
    }
    let (virtual_section, physical) = resource.sections(bytes);
    let cloth = read_cloth(virtual_section, Some(physical))
        .ok()
        .filter(drawn)?;
    Some(DocPayload::Gta6Drawable(Arc::new(Resource::Cloth(cloth))))
}

/// Whether `cloth` has positions that are laid out, and so opens in the model view.
fn drawn(cloth: &Cloth) -> bool {
    !points(cloth).is_empty()
}

/// Whether `payload` is a scanned cloth record, whose lines are drawn in every shading mode.
pub(crate) fn is_cloth(payload: &DocPayload) -> bool {
    matches!(payload, DocPayload::Gta6Drawable(r) if matches!(**r, Resource::Cloth(_)))
}

/// A type-1 record's elements, a type-2 record's vertices, a type-3 record's strand points, or the
/// corners of a type-4 or type-5 record's boxes.
fn points(cloth: &Cloth) -> Vec<Vec3> {
    let of = |points: &[[f32; 4]]| -> Vec<Vec3> { points.iter().map(at).collect() };
    match &cloth.kind {
        ClothKind::Elements { elements, .. } => of(elements.as_slice()),
        ClothKind::Edges { vertices, .. } => of(vertices.as_slice()),
        ClothKind::Mesh(m) => of(m.points.as_slice()),
        ClothKind::Collider { .. } | ClothKind::Shapes { .. } => {
            boxes(cloth).into_iter().flatten().collect()
        }
    }
}

/// The eight corners of each box a type-4 record's half extents turned by its rotation, or a
/// type-5 record's shapes' stated minimum and maximum, give; corner `i` takes the maximum on the
/// axes of its bits 1, 2 and 4.
fn boxes(cloth: &Cloth) -> Vec<[Vec3; 8]> {
    let corners = |lo: Vec3, hi: Vec3, turn: Quat| -> [Vec3; 8] {
        std::array::from_fn(|i| {
            let pick = |bit: usize, lo: f32, hi: f32| if i & bit == 0 { lo } else { hi };
            turn * Vec3::new(
                pick(1, lo.x, hi.x),
                pick(2, lo.y, hi.y),
                pick(4, lo.z, hi.z),
            )
        })
    };
    match &cloth.kind {
        ClothKind::Collider {
            rotation,
            half_extents,
            ..
        } => {
            let q = Quat::from_array(*rotation);
            let turn = match q.is_finite() && q.length_squared() > 1e-12 {
                true => q.normalize(),
                false => Quat::IDENTITY,
            };
            let half = Vec3::from(*half_extents);
            vec![corners(-half, half, turn)]
        }
        ClothKind::Shapes { shapes, .. } => (shapes.iter())
            .map(|s| corners(s.box_min.into(), s.box_max.into(), Quat::IDENTITY))
            .collect(),
        _ => Vec::new(),
    }
}

fn at(p: &[f32; 4]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

/// A type-3 record's triangles over its strands' roots, laid over each level of its strands as a
/// surface a level; none for the other types, which lay out no triangle.
pub(crate) fn meshes(cloth: &Cloth) -> Vec<MeshUpload> {
    let ClothKind::Mesh(m) = &cloth.kind else {
        return Vec::new();
    };
    let per = m.points_per_strand.max(1);
    (0..per)
        .filter_map(|k| sheet(&m.points, &m.triangles, k, per))
        .collect()
}

/// The triangles over point `k` of each strand, strand s's point k being point `per * s + k`.
fn sheet(points: &[[f32; 4]], triangles: &[[u16; 3]], k: usize, per: usize) -> Option<MeshUpload> {
    let level: Vec<[f32; 3]> = (points.iter().skip(k).step_by(per))
        .map(|p| at(p).to_array())
        .collect();
    let indices: Vec<u16> = (triangles.iter())
        .filter(|t| t.iter().all(|i| usize::from(*i) < level.len()))
        .flatten()
        .copied()
        .collect();
    if indices.is_empty() {
        return None;
    }
    let mut normals = Vec::new();
    face_normals(&level, &indices, &mut normals);
    Some(MeshUpload {
        bump: false,
        vertices: (level.iter().zip(&normals))
            .map(|(p, n)| MeshVertex::new_coloured(*p, *n, [0.0; 2], CLOTH_SURFACE))
            .collect(),
        indices: indices.into_iter().map(u32::from).collect(),
        texture_name: None,
        skinning: None,
        palette_bones: Vec::new(),
        channels: Vec::new(),
        source: None,
    })
}

/// The segment from `a` to `b`; none where either is not finite.
fn push(out: &mut Vec<LineVertex>, a: Vec3, b: Vec3, colour: [f32; 4]) {
    if !(a.is_finite() && b.is_finite()) {
        return;
    }
    out.push(LineVertex::new(a.to_array(), colour));
    out.push(LineVertex::new(b.to_array(), colour));
}

/// The sphere about the record's finite positions, which a camera frames.
pub(crate) fn bounds(cloth: &Cloth) -> Option<(Vec3, f32)> {
    let points: Vec<Vec3> = points(cloth)
        .into_iter()
        .filter(|p| p.is_finite())
        .collect();
    let first = *points.first()?;
    let (mut min, mut max) = (first, first);
    for &p in &points {
        (min, max) = (min.min(p), max.max(p));
    }
    let radius = (max - min).length() / 2.0;
    (radius.is_finite() && radius > 0.0).then(|| ((min + max) / 2.0, radius))
}

/// A type-1 record's pairs of elements or a type-2 record's edges, fillers skipped; a type-3
/// record's strands, root triangles and roots; the twelve edges of each box of a type 4 or 5.
pub(crate) fn lines(cloth: &Cloth) -> Vec<LineVertex> {
    let mut out = Vec::new();
    match &cloth.kind {
        ClothKind::Elements {
            elements: vertices,
            records: edges,
        }
        | ClothKind::Edges { vertices, edges } => {
            for e in edges.iter().filter(|e| e.indices[0] != e.indices[1]) {
                let [i, j] = e.indices.map(usize::from);
                if let (Some(a), Some(b)) = (vertices.get(i), vertices.get(j)) {
                    push(&mut out, at(a), at(b), EDGE);
                }
            }
        }
        ClothKind::Collider { .. } | ClothKind::Shapes { .. } => {
            for corners in boxes(cloth) {
                for i in 0..8 {
                    for bit in [1, 2, 4] {
                        if i & bit == 0 {
                            push(&mut out, corners[i], corners[i | bit], SHAPE);
                        }
                    }
                }
            }
        }
        ClothKind::Mesh(m) => {
            let per = m.points_per_strand.max(1);
            for strand in m.points.chunks(per) {
                for pair in strand.windows(2) {
                    push(&mut out, at(&pair[0]), at(&pair[1]), EDGE);
                }
            }
            let root = |i: u16| m.points.get(usize::from(i) * per).map(at);
            for &t in &m.triangles {
                if let [Some(a), Some(b), Some(c)] = t.map(root) {
                    for (from, to) in [(a, b), (b, c), (c, a)] {
                        push(&mut out, from, to, ROOT_TRIANGLE);
                    }
                }
            }
            let mark = bounds(cloth).map_or(0.01, |(_, r)| (r * 0.02).clamp(0.002, 0.05));
            for p in m.points.iter().step_by(per).map(at) {
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    push(&mut out, p - axis * mark, p + axis * mark, PIN);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::headerless::{ClothEdge, ClothMesh};

    fn cloth(kind: ClothKind) -> Cloth {
        Cloth {
            flag: 0,
            bytes: [0, 0],
            owner: 0,
            record: None,
            kind,
        }
    }

    /// Strand `s`'s point `k`, strands a metre apart along x and points 0.1 m apart along z.
    fn point(s: usize, k: usize) -> [f32; 4] {
        [s as f32, 0.5 * s as f32, 0.1 * k as f32, 1.0]
    }

    fn strands(count: usize, per: usize, triangles: Vec<[u16; 3]>) -> ClothKind {
        let points = (0..count)
            .flat_map(|s| (0..per).map(move |k| point(s, k)))
            .collect();
        ClothKind::Mesh(Box::new(ClothMesh {
            box_min: [0.0; 3],
            box_max: [2.0, 1.0, 0.0],
            bone_hashes: vec![1],
            records: 0,
            vertex_count: count as u16,
            vertex_arrays: [0; 11],
            index_arrays: [0; 8],
            triangles,
            points,
            points_per_strand: per,
            bones: vec![[0; 8]; count],
            weights: vec![[255, 0, 0, 0, 0, 0, 0, 0]; count],
            root_triangles: vec![0; count],
            rest_lengths: vec![0.1 * (per - 1) as f32; count],
            ..ClothMesh::default()
        }))
    }

    fn segments(lines: &[LineVertex], colour: [f32; 4]) -> Vec<([f32; 3], [f32; 3])> {
        (lines.as_chunks::<2>().0.iter())
            .filter(|pair| pair[0].colour == colour)
            .map(|pair| (pair[0].position, pair[1].position))
            .collect()
    }

    #[test]
    fn a_type_3_record_draws_each_strand_apart_its_root_triangles_and_a_cross_at_each_root() {
        let c = cloth(strands(3, 4, vec![[0, 1, 2]]));
        let lines = lines(&c);
        let edges = segments(&lines, EDGE);
        assert_eq!(edges.len(), 3 * 3, "three segments a strand");
        let point_at = |s, k| at(&point(s, k)).to_array();
        assert!(edges.contains(&(point_at(1, 2), point_at(1, 3))));
        // A strand's end is not joined to the next strand's root.
        assert!(!edges.contains(&(point_at(0, 3), point_at(1, 0))));
        let triangle = segments(&lines, ROOT_TRIANGLE);
        assert_eq!(triangle.len(), 3);
        assert!(triangle.contains(&(point_at(0, 0), point_at(1, 0))));
        let pins = segments(&lines, PIN);
        assert_eq!(pins.len(), 3 * 3, "three axes at each of three roots");
        let middle = |(a, b): &([f32; 3], [f32; 3])| (Vec3::from(*a) + Vec3::from(*b)) / 2.0;
        for s in 0..3 {
            let root = at(&point(s, 0));
            let marked = pins.iter().any(|p| middle(p).distance(root) < 1e-6);
            assert!(marked, "root {s}");
        }
    }

    #[test]
    fn a_type_3_record_draws_its_root_triangles_as_a_surface_over_each_level_of_its_strands() {
        let mut kind = strands(4, 3, vec![[0, 1, 2], [1, 3, 2], [0, 4, 1]]);
        let ClothKind::Mesh(m) = &mut kind else {
            unreachable!()
        };
        // Roots off one line, so that each triangle has a facing.
        for (s, root) in m.points.iter_mut().step_by(3).enumerate() {
            root[1] = (s * s) as f32;
        }
        let roots: Vec<[f32; 3]> = (m.points.iter().step_by(3))
            .map(|p| at(p).to_array())
            .collect();
        let surfaces = meshes(&cloth(kind));
        let [surface, second, last] = &surfaces[..] else {
            panic!("a surface for each of the three levels");
        };
        // Level k is point k of each strand, under the same triangles.
        for (k, level) in [(1, second), (2, last)] {
            let points: Vec<[f32; 3]> = level.vertices.iter().map(|v| v.position).collect();
            let expected: Vec<[f32; 3]> = (0..4).map(|s| at(&point(s, k)).to_array()).collect();
            assert_eq!(points, expected, "level {k}");
            assert_eq!(level.indices, [0, 1, 2, 1, 3, 2]);
        }
        let positions: Vec<[f32; 3]> = surface.vertices.iter().map(|v| v.position).collect();
        // A vertex at each strand's root, in strand order.
        assert_eq!(positions, roots);
        // The triangle naming a fifth root, which the record does not have, is left out.
        assert_eq!(surface.indices, [0, 1, 2, 1, 3, 2]);
        let [a, b, d] = [0, 1, 2].map(|k| Vec3::from(roots[k]));
        let facing = (b - a).cross(d - a).normalize();
        let n = Vec3::from(surface.vertices[0].normal);
        assert!((n.length() - 1.0).abs() < 1e-5);
        assert!(n.dot(facing) > 0.99, "{n:?} against {facing:?}");
        assert!(surface.vertices.iter().all(|v| v.colour == CLOTH_SURFACE));
        assert!(surface.skinning.is_none());
        assert!(surface.texture_name.is_none());
    }

    #[test]
    fn a_record_without_laid_out_triangles_draws_no_surface() {
        assert!(meshes(&cloth(strands(2, 2, Vec::new()))).is_empty());
        let edges = ClothKind::Edges {
            vertices: vec![[0.0; 4], [1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
            edges: Vec::new(),
        };
        assert!(meshes(&cloth(edges)).is_empty());
        let collider = ClothKind::Collider {
            rotation: [0.0, 0.0, 0.0, 1.0],
            half_extents: [0.1, 0.2, 0.3],
            radius: 0.5,
        };
        assert!(meshes(&cloth(collider)).is_empty());
    }

    #[test]
    fn a_type_2_record_draws_its_edges_and_skips_its_fillers() {
        let edge = |indices| ClothEdge {
            indices,
            length: 1.0,
            weight: 0.0,
            word: 0,
        };
        let vertices = vec![
            [0.0, 0.0, 0.0, 1.0],
            [2.0, 0.0, 0.0, 1.0],
            [0.0, 2.0, 0.0, 1.0],
        ];
        let c = cloth(ClothKind::Edges {
            vertices: vertices.clone(),
            edges: vec![edge([0, 1]), edge([1, 2]), edge([0, 0])],
        });
        let v = |i: usize| at(&vertices[i]).to_array();
        assert_eq!(segments(&lines(&c), EDGE), [(v(0), v(1)), (v(1), v(2))]);
        assert!(segments(&lines(&c), PIN).is_empty());
        let (min, max) = (Vec3::ZERO, Vec3::new(2.0, 2.0, 0.0));
        let framed = ((min + max) / 2.0, (max - min).length() / 2.0);
        assert_eq!(bounds(&c), Some(framed));
        assert!(drawn(&c));
    }

    #[test]
    fn a_record_with_nothing_laid_out_to_draw_is_not_drawn() {
        let c = cloth(ClothKind::Shapes {
            shapes: Vec::new(),
            keys: Vec::new(),
        });
        assert!(!drawn(&c));
        assert!(lines(&c).is_empty());
        assert_eq!(bounds(&c), None);
        assert!(drawn(&cloth(strands(1, 2, Vec::new()))));
    }

    #[test]
    fn a_type_1_record_draws_its_pairs_of_distinct_elements() {
        let pair = |indices| ClothEdge {
            indices,
            length: 1.0,
            weight: 0.25,
            word: 7,
        };
        let elements = vec![
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 3.0, 0.0, 0.5],
            [1.0, 1.0, 2.0, 10.0],
        ];
        let c = cloth(ClothKind::Elements {
            elements: elements.clone(),
            records: vec![pair([2, 0]), pair([1, 1]), pair([0, 1])],
        });
        let e = |i: usize| at(&elements[i]).to_array();
        assert_eq!(segments(&lines(&c), EDGE), [(e(2), e(0)), (e(0), e(1))]);
        assert!(drawn(&c));
        let (min, max) = (Vec3::ZERO, Vec3::new(1.0, 3.0, 2.0));
        assert_eq!(
            bounds(&c),
            Some(((min + max) / 2.0, (max - min).length() / 2.0))
        );
    }

    /// Whether `a` and `b` are the same point within float error.
    fn near(a: [f32; 3], b: Vec3) -> bool {
        Vec3::from(a).distance(b) < 1e-5
    }

    #[test]
    fn a_type_4_record_draws_its_half_extents_as_a_box_turned_by_its_rotation() {
        let half = std::f32::consts::FRAC_PI_4;
        let c = cloth(ClothKind::Collider {
            rotation: [0.0, 0.0, half.sin(), half.cos()],
            half_extents: [0.1, 0.2, 0.3],
            radius: 0.5,
        });
        let edges = segments(&lines(&c), SHAPE);
        assert_eq!(edges.len(), 12);
        // A quarter turn about z takes x to y and y to -x: the box spans 0.2 along x and 0.1 along y.
        let turned = |x: f32, y: f32, z: f32| Vec3::new(-0.2 * y, 0.1 * x, 0.3 * z);
        let mut corners = Vec::new();
        for x in [-1.0f32, 1.0] {
            for y in [-1.0f32, 1.0] {
                for z in [-1.0f32, 1.0] {
                    corners.push(turned(x, y, z));
                }
            }
        }
        for (a, b) in &edges {
            assert!(corners.iter().any(|c| near(*a, *c)), "{a:?}");
            assert!(corners.iter().any(|c| near(*b, *c)), "{b:?}");
            let length = Vec3::from(*a).distance(Vec3::from(*b));
            assert!(
                [0.2, 0.4, 0.6].iter().any(|l| (length - l).abs() < 1e-5),
                "an edge of the box, not a diagonal: {length}"
            );
        }
        let (centre, radius) = bounds(&c).expect("framed");
        assert!(centre.length() < 1e-5);
        assert!((radius - Vec3::new(0.2, 0.1, 0.3).length()).abs() < 1e-5);
    }

    #[test]
    fn a_type_5_record_draws_each_shape_as_the_box_it_states() {
        use or_io::gta6::headerless::ClothShape;
        let shape = |box_min, box_max| ClothShape {
            offset: [3.0, 2.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            bone_hash: 0x1234_5678,
            half_extents: [0.5, 0.5, 0.5],
            kind: 5,
            box_min,
            box_max,
        };
        let c = cloth(ClothKind::Shapes {
            shapes: vec![
                shape([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]),
                shape([4.0, 5.0, 6.0], [5.0, 7.0, 9.0]),
            ],
            keys: vec![(0xAB, vec![0, 1])],
        });
        let edges = segments(&lines(&c), SHAPE);
        assert_eq!(edges.len(), 24, "twelve edges a box");
        let along = |(a, b): &([f32; 3], [f32; 3])| {
            let d = Vec3::from(*b) - Vec3::from(*a);
            (d.x != 0.0) as u8 + (d.y != 0.0) as u8 + (d.z != 0.0) as u8
        };
        assert!(
            edges.iter().all(|e| along(e) == 1),
            "each edge runs along one axis"
        );
        assert!(edges.contains(&([4.0, 5.0, 6.0], [5.0, 5.0, 6.0])));
        assert!(edges.contains(&([-1.0, 2.0, 3.0], [1.0, 2.0, 3.0])));
        let (min, max) = (Vec3::new(-1.0, -2.0, -3.0), Vec3::new(5.0, 7.0, 9.0));
        assert_eq!(
            bounds(&c),
            Some(((min + max) / 2.0, (max - min).length() / 2.0))
        );
        assert!(drawn(&c));
    }
}
