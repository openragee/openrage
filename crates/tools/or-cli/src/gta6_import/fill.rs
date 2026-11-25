//! A channel a GTA VI geometry declares and the mesh omits, at a changed vertex count, given the
//! stated default `mesh import --fill-missing` gives a GTA V geometry's: a normal +Z, a tangent +X,
//! a colour white, a texcoord 0, and a skin of one whole weight on the first palette slot.

use std::collections::BTreeSet;

use or_io::gta6::headerless::{Geometry, GeometryEdit};

const NORMAL: [f32; 3] = [0.0, 0.0, 1.0];
const TANGENT: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0; 4];
const WHOLE: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// `edit` with each channel `g` declares and `edit` gives no values for filled with its default;
/// the channels filled, ascending. At `g`'s own vertex count nothing is filled, since the write
/// keeps what the geometry holds.
pub(super) fn defaults(edit: &mut GeometryEdit, g: &Geometry) -> Vec<u8> {
    let n = edit.positions.len();
    if n == g.vertex_count as usize {
        return Vec::new();
    }
    let mut filled = BTreeSet::new();
    let declared = |k: u8| g.channels.iter().any(|c| c.index == k);
    if declared(4) && edit.normals.is_empty() {
        edit.normals = vec![NORMAL; n];
        filled.insert(4);
    }
    // Texcoord 0's tangent is channel 4's frame word, or channel 8 beside a 10:10:10 normal.
    if !g.vertices.tangents.is_empty() && edit.tangents.is_empty() {
        edit.tangents = vec![TANGENT; n];
        let framed = g.channels.iter().any(|c| c.index == 4 && c.code == 0x29);
        filled.insert(if framed { 4 } else { 8 });
    }
    for c in (24..=26).filter(|c| declared(*c)) {
        if fill(&mut edit.colours, c, WHITE, n) {
            filled.insert(c);
        }
    }
    // Channel 33 is texcoord 5 only in its 4-byte form; its 8-byte form derives from the positions.
    let texcoord = |c: u8| c < 33 || g.channels.iter().any(|k| k.index == 33 && k.code == 0x26);
    for c in (28..=33).filter(|c| declared(*c) && texcoord(*c)) {
        if fill(&mut edit.uvs, c, [0.0; 2], n) {
            filled.insert(c);
        }
    }
    for c in (9..=11).filter(|c| declared(*c)) {
        if fill(&mut edit.uv_tangents, c, TANGENT, n) {
            filled.insert(c);
        }
    }
    // The mesh states no skin: lane 0 names palette slot 0 and holds all of the weight.
    if declared(20) && edit.joints.is_empty() {
        for c in [20, 21].into_iter().filter(|c| declared(*c)) {
            edit.joints.push((c, vec![[0; 4]; n]));
            filled.insert(c);
        }
        for (c, value) in [(16, WHOLE), (17, [0.0; 4])] {
            if declared(c) && fill(&mut edit.weights, c, value, n) {
                filled.insert(c);
            }
        }
    }
    filled.into_iter().collect()
}

/// `value` for each of `n` vertices as channel `c` of `list`, where `list` holds no channel `c`;
/// whether it was added.
fn fill<T: Clone>(list: &mut Vec<(u8, Vec<T>)>, c: u8, value: T, n: usize) -> bool {
    let given = list.iter().any(|(k, _)| *k == c);
    if !given {
        list.push((c, vec![value; n]));
    }
    !given
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::headerless::{Channel, Vertices};

    fn channel(index: u8, code: u8, size: u8) -> Channel {
        Channel {
            index,
            offset: 0,
            size,
            code,
        }
    }

    /// Three vertices declaring a frame-word normal, colour 24, texcoords 28 and 29, texcoord 1's
    /// tangent, weights 16 and blend indices 20.
    fn declared() -> Geometry {
        Geometry {
            shader: 0,
            vertex_count: 3,
            channels: vec![
                channel(0, 6, 12),
                channel(4, 0x29, 4),
                channel(9, 0x18, 4),
                channel(16, 0x1C, 4),
                channel(20, 0x1E, 4),
                channel(24, 0x1C, 4),
                channel(28, 0x10, 8),
                channel(29, 0x10, 8),
            ],
            triangles: vec![[0, 1, 2]],
            positions: Ok(vec![[0.0; 3]; 3]),
            vertices: Vertices {
                normals: vec![NORMAL; 3],
                tangents: vec![TANGENT; 3],
                ..Vertices::default()
            },
            vertex_data: Vec::new(),
            index_data: Vec::new(),
        }
    }

    fn four() -> GeometryEdit {
        GeometryEdit {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            ..GeometryEdit::default()
        }
    }

    #[test]
    fn at_another_count_each_omitted_channel_takes_its_default_and_is_named() {
        let g = declared();
        let mut edit = four();
        let given = vec![[0.5, 0.5]; 4];
        edit.uvs.push((28, given.clone()));
        let filled = defaults(&mut edit, &g);
        assert_eq!(filled, [4, 9, 16, 20, 24, 29]);
        assert_eq!(edit.normals, vec![NORMAL; 4]);
        assert_eq!(edit.tangents, vec![TANGENT; 4]);
        assert_eq!(edit.uvs, [(28, given), (29, vec![[0.0; 2]; 4])]);
        assert_eq!(edit.colours, [(24, vec![WHITE; 4])]);
        assert_eq!(edit.uv_tangents, [(9, vec![TANGENT; 4])]);
        assert_eq!(edit.joints, [(20, vec![[0; 4]; 4])]);
        assert_eq!(edit.weights, [(16, vec![WHOLE; 4])]);
    }

    #[test]
    fn texcoord_5_s_four_byte_form_is_filled_and_channel_33_s_eight_byte_form_is_not() {
        let mut g = declared();
        g.channels.push(channel(33, 0x26, 4));
        let mut edit = four();
        assert!(defaults(&mut edit, &g).contains(&33));
        assert!(edit.uvs.contains(&(33, vec![[0.0; 2]; 4])));

        // The 8-byte form's lanes are derived from the positions, so no value stands in for them.
        g.channels.last_mut().expect("channel 33").code = 0x10;
        let mut edit = four();
        assert!(!defaults(&mut edit, &g).contains(&33));
        assert!(edit.uvs.iter().all(|(c, _)| *c != 33));
    }

    #[test]
    fn a_skin_or_normal_the_mesh_states_and_the_geometry_s_own_count_are_left_alone() {
        let g = declared();
        let mut edit = four();
        let normals = vec![[0.0, 1.0, 0.0]; 4];
        edit.normals = normals.clone();
        edit.joints.push((20, vec![[1, 0, 0, 0]; 4]));
        let filled = defaults(&mut edit, &g);
        // Channel 4 for its tangent alone, and no skin.
        assert_eq!(filled, [4, 9, 24, 28, 29]);
        assert_eq!(edit.normals, normals);
        assert_eq!(edit.joints, [(20, vec![[1, 0, 0, 0]; 4])]);
        assert!(edit.weights.is_empty());

        let mut same = GeometryEdit {
            positions: vec![[0.0; 3]; 3],
            triangles: vec![[0, 1, 2]],
            ..GeometryEdit::default()
        };
        let untouched = same.clone();
        assert!(defaults(&mut same, &g).is_empty());
        assert_eq!(same, untouched);
    }
}
