//! A model as a title hands it, the game's own vertex bytes with their layout, made the vertices
//! the plain world pass draws: done on a reading thread, never on the frame's.

use glam::Vec3;
use or_io::half::half_to_f32;
use or_renderer::MeshVertex;
use or_world::model::{Element, ElementFormat, Mesh, Model, Semantic, Surface};

/// One mesh as the device's pages take it.
#[derive(Debug, Clone)]
pub struct ArrivedMesh {
    pub vertices: Vec<MeshVertex>,
    /// Three a triangle.
    pub indices: Vec<u32>,
    pub surface: Surface,
}

/// One level of detail, drawn while the camera is nearer than `until` metres.
#[derive(Debug, Clone)]
pub struct ArrivedLod {
    pub until: f32,
    pub meshes: Vec<ArrivedMesh>,
}

/// A model read and decoded, the most detailed level first.
#[derive(Debug, Clone, Default)]
pub struct ArrivedModel {
    pub lods: Vec<ArrivedLod>,
}

impl ArrivedModel {
    /// The bytes its vertices and indices take on the device.
    pub fn bytes(&self) -> u64 {
        let meshes = self.lods.iter().flat_map(|lod| &lod.meshes);
        meshes
            .map(|mesh| {
                (mesh.vertices.len() * std::mem::size_of::<MeshVertex>()
                    + mesh.indices.len() * std::mem::size_of::<u32>()) as u64
            })
            .sum()
    }
}

/// Up to four numbers of the element `element` of the vertex that starts at `vertex`; `None` where the bytes end before it does.
fn numbers(vertex: &[u8], element: &Element) -> Option<[f32; 4]> {
    let at = element.offset as usize;
    let bytes = vertex.get(at..at + element.format.size() as usize)?;
    let float = |n: usize| f32::from_le_bytes([bytes[n], bytes[n + 1], bytes[n + 2], bytes[n + 3]]);
    let half = |n: usize| half_to_f32(u16::from_le_bytes([bytes[n], bytes[n + 1]]));
    Some(match element.format {
        ElementFormat::F32x2 => [float(0), float(4), 0.0, 0.0],
        ElementFormat::F32x3 => [float(0), float(4), float(8), 0.0],
        ElementFormat::F32x4 => [float(0), float(4), float(8), float(12)],
        ElementFormat::U8x4Norm => [0, 1, 2, 3].map(|n| f32::from(bytes[n]) / 255.0),
        ElementFormat::U8x4 => [0, 1, 2, 3].map(|n| f32::from(bytes[n])),
        ElementFormat::F16x2 => [half(0), half(2), 0.0, 0.0],
        ElementFormat::F16x4 => [half(0), half(2), half(4), half(6)],
    })
}

/// A normal as the element stores it: a byte a component is a number from −1 to 1.
fn normal_of(stored: [f32; 4], format: ElementFormat) -> Vec3 {
    let n = Vec3::new(stored[0], stored[1], stored[2]);
    let n = match format {
        ElementFormat::U8x4Norm => n * 2.0 - Vec3::ONE,
        _ => n,
    };
    n.try_normalize().unwrap_or(Vec3::Z)
}

/// The normal of each vertex as the triangles about it face, for a mesh that states none.
fn faced(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|i| i as usize);
        let (Some(pa), Some(pb), Some(pc)) = (positions.get(a), positions.get(b), positions.get(c))
        else {
            continue;
        };
        // The cross product's length weighs a triangle by its area.
        let face = (*pb - *pa).cross(*pc - *pa);
        for at in [a, b, c] {
            normals[at] += face;
        }
    }
    normals
        .into_iter()
        .map(|n| n.try_normalize().unwrap_or(Vec3::Z))
        .collect()
}

/// The vertices the plain pass draws `mesh` with; `None` where it states no position, no vertex, or an index past its vertices.
// ⚠ A vertex colour is left white: the game's is light and wetness its own programs read, never a tint.
pub fn decoded(mesh: &Mesh) -> Option<ArrivedMesh> {
    let stride = mesh.layout.stride as usize;
    let find = |wanted: Semantic| mesh.layout.elements.iter().find(|e| e.semantic == wanted);
    let position = find(Semantic::Position)?;
    let (normal, uv) = (find(Semantic::Normal), find(Semantic::TexCoord(0)));
    let count = mesh.vertex_count();
    if count == 0 || mesh.indices.is_empty() || !mesh.indices.len().is_multiple_of(3) {
        return None;
    }
    if mesh.indices.iter().any(|index| *index as usize >= count) {
        return None;
    }
    let mut positions = Vec::with_capacity(count);
    let mut vertices = Vec::with_capacity(count);
    for vertex in mesh.vertices.chunks_exact(stride) {
        let p = numbers(vertex, position)?;
        let at = Vec3::new(p[0], p[1], p[2]);
        if !at.is_finite() {
            return None;
        }
        let n = match normal {
            Some(e) => Some(normal_of(numbers(vertex, e)?, e.format)),
            None => None,
        };
        let t = match uv {
            Some(e) => numbers(vertex, e)?,
            None => [0.0; 4],
        };
        positions.push(at);
        vertices.push(MeshVertex::new(
            at.to_array(),
            n.unwrap_or(Vec3::Z).to_array(),
            [t[0], t[1]],
        ));
    }
    if normal.is_none() {
        for (vertex, n) in vertices.iter_mut().zip(faced(&positions, &mesh.indices)) {
            vertex.normal = n.to_array();
        }
    }
    Some(ArrivedMesh {
        vertices,
        indices: mesh.indices.clone(),
        surface: mesh.surface,
    })
}

/// `model` as the pages take it, a level that holds no mesh that decodes left out; `None` where no level is left.
pub fn arrived(model: &Model) -> Option<ArrivedModel> {
    let lods: Vec<ArrivedLod> = (model.lods.iter())
        .filter_map(|lod| {
            let meshes: Vec<ArrivedMesh> = lod.meshes.iter().filter_map(decoded).collect();
            (!meshes.is_empty()).then_some(ArrivedLod {
                until: lod.distance,
                meshes,
            })
        })
        .collect();
    (!lods.is_empty()).then_some(ArrivedModel { lods })
}

#[cfg(test)]
mod tests {
    use or_world::model::VertexLayout;

    use super::*;

    fn element(semantic: Semantic, format: ElementFormat, offset: u32) -> Element {
        Element {
            semantic,
            format,
            offset,
        }
    }

    /// A triangle in the plane z = 2, its vertices a position, a normal as bytes and a half-float texture coordinate.
    fn triangle(with_normal: bool) -> Mesh {
        let mut elements = vec![
            element(Semantic::Position, ElementFormat::F32x3, 0),
            element(Semantic::TexCoord(0), ElementFormat::F16x2, 16),
        ];
        if with_normal {
            elements.push(element(Semantic::Normal, ElementFormat::U8x4Norm, 12));
        }
        let mut vertices = Vec::new();
        for (x, y) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0)] {
            for value in [x, y, 2.0] {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            // Towards −X as bytes: 0 is −1, 255 is 1.
            vertices.extend_from_slice(&[0, 128, 128, 0]);
            for value in [x * 0.5, y * 0.25] {
                vertices.extend_from_slice(&or_io::half::f32_to_half(value).to_le_bytes());
            }
        }
        Mesh {
            layout: VertexLayout {
                stride: 20,
                elements,
            },
            vertices,
            indices: vec![0, 1, 2],
            surface: Surface::default(),
        }
    }

    #[test]
    fn a_mesh_of_the_games_bytes_decodes_by_its_layout_and_one_with_no_normal_takes_its_faces() {
        let stated = decoded(&triangle(true)).expect("a whole triangle");
        assert_eq!(stated.vertices[1].position, [1.0, 0.0, 2.0]);
        assert_eq!(stated.vertices[2].uv, [0.0, 0.25]);
        let n = Vec3::from(stated.vertices[0].normal);
        assert!(n.distance(Vec3::NEG_X) < 0.01, "{n}");
        // Counter-clockwise seen from +Z, so its face looks up.
        let faced = decoded(&triangle(false)).expect("a whole triangle");
        assert_eq!(faced.vertices[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(faced.vertices[0].colour, [1.0; 4]);
    }

    #[test]
    fn a_mesh_that_is_not_whole_is_not_decoded() {
        let mut past = triangle(true);
        past.indices = vec![0, 1, 3];
        assert!(decoded(&past).is_none(), "an index past the vertices");
        let mut ragged = triangle(true);
        ragged.indices.push(0);
        assert!(decoded(&ragged).is_none(), "indices that are no triangles");
        let mut placeless = triangle(true);
        placeless.layout.elements.remove(0);
        assert!(decoded(&placeless).is_none(), "no position");
        let mut short = triangle(true);
        short.layout.elements[1].offset = 18;
        assert!(decoded(&short).is_none(), "an element past its vertex");
        let model = Model {
            lods: vec![or_world::model::Lod {
                distance: 50.0,
                meshes: vec![past],
            }],
            title: None,
        };
        assert!(arrived(&model).is_none(), "a model with no mesh left");
    }
}
