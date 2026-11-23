//! A glTF primitive's joints and weights as a GTA VI geometry's blend indices and weights, each
//! joint mapped by its bone's name onto the model's palette, and its texcoords' tangents, for a
//! geometry whose vertex count the import changes.

use std::collections::BTreeSet;

use or_interchange::{Bone, Primitive};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::headerless::{Geometry, GeometryEdit, Model, Palette, Skeleton};

/// The bones a primitive's joints index and the drawable's own skeleton.
pub(super) struct Wearer<'a> {
    pub(super) bones: &'a [Bone],
    pub(super) own: Option<&'a Skeleton>,
}

/// `edit`'s weights, blend indices and texcoord tangents for geometry `g` of `model` from `p`,
/// where the vertex count changes; at the same count the geometry keeps them as it holds them.
pub(super) fn fill(
    edit: &mut GeometryEdit,
    g: &Geometry,
    model: &Model,
    p: &Primitive,
    wearer: &Wearer<'_>,
) -> Result<(), String> {
    let n = p.positions.len();
    if n == g.vertex_count as usize {
        return Ok(());
    }
    let declared = |k: u8| g.channels.iter().any(|c| c.index == k);
    // Channels 9 to 11 are the tangents of texcoords 1 to 3, channels 29 to 31.
    for k in (9..=11).filter(|k| declared(*k)) {
        if let Some((_, uvs)) = edit.uvs.iter().find(|(c, _)| *c == k + 20) {
            if let Some(t) = tangents(p, uvs) {
                edit.uv_tangents.push((k, t));
            }
        }
    }
    if !declared(20) {
        return Ok(());
    }
    if p.joints.len() != n || p.weights.len() != n {
        return Err(
            "the mesh gives no joint and weight for each vertex, and the geometry's blend \
                    indices are kept only at its vertex count"
                .into(),
        );
    }
    let used = used(model);
    let mut lanes = Vec::with_capacity(n);
    for (i, (joints, weights)) in p.joints.iter().zip(&p.weights).enumerate() {
        let mut out = [0u16; 4];
        let mut set = 0;
        for (joint, weight) in joints.iter().zip(weights) {
            if *weight > 0.0 {
                out[set] = index(*joint, model, wearer, &used)?;
                set += 1;
            }
        }
        // With no weight channel a vertex takes one index, which holds all of its weight.
        if set > 1 && !declared(16) {
            return Err(format!(
                "vertex {i} weights {set} bones, and the geometry holds one blend index a vertex"
            ));
        }
        lanes.push(out);
    }
    let weights: Vec<[f32; 4]> = (p.weights.iter())
        .map(|w| {
            let mut out = [0.0; 4];
            for (slot, x) in out.iter_mut().zip(w.iter().filter(|x| **x > 0.0)) {
                *slot = *x;
            }
            out
        })
        .collect();
    edit.joints.push((20, lanes));
    if declared(21) {
        edit.joints.push((21, vec![[0; 4]; n]));
    }
    if declared(16) {
        edit.weights.push((16, weights));
    }
    if declared(17) {
        edit.weights.push((17, vec![[0.0; 4]; n]));
    }
    Ok(())
}

/// The palette index of the bone `joint` names: a hash palette's entry of its name hash, or a
/// mask's bone of the own skeleton that a weighted lane of the model already uses.
fn index(
    joint: u16,
    model: &Model,
    wearer: &Wearer<'_>,
    used: &BTreeSet<u16>,
) -> Result<u16, String> {
    let name = (wearer.bones.get(usize::from(joint)))
        .map(|b| b.name.as_str())
        .ok_or_else(|| format!("joint {joint} names no bone of the mesh's skeleton"))?;
    let own = wearer
        .own
        .and_then(|s| s.bones.iter().position(|b| b.name == name));
    match &model.palette {
        Ok(Palette::Hashes(hashes)) => {
            let hash = match (wearer.own, own) {
                (Some(s), Some(at)) => s.bones[at].hash,
                _ => name_hash32(name.as_bytes()),
            };
            (hashes.iter().position(|h| *h == hash))
                .and_then(|at| u16::try_from(at).ok())
                .ok_or_else(|| {
                    format!(
                        "the joint {name} is a bone the model's palette lacks, and a new palette \
                         entry is not written"
                    )
                })
        }
        Ok(Palette::Own) => (own.and_then(|at| u16::try_from(at).ok()))
            .filter(|at| used.contains(at))
            .ok_or_else(|| {
                format!(
                    "the joint {name} is no bone of the drawable's own skeleton that the model \
                     already uses, and setting a bone's bit in its palette is not written"
                )
            }),
        Err(_) => {
            Err("the model's palette is in neither measured form, so no joint maps onto it".into())
        }
    }
}

/// The palette indices of the weighted lanes of the model's geometries; lane 0 where a geometry
/// has no weights.
fn used(model: &Model) -> BTreeSet<u16> {
    let mut out = BTreeSet::new();
    for g in model.geometries.iter().flatten() {
        let v = &g.vertices;
        for (c, joints) in &v.joints {
            let weights = (v.weights.iter())
                .find(|(w, _)| *w + 4 == *c)
                .map(|(_, w)| w);
            for (i, lanes) in joints.iter().enumerate() {
                for (k, index) in lanes.iter().enumerate() {
                    let weighted = weights.map_or(k == 0, |w| w.get(i).is_some_and(|w| w[k] > 0.0));
                    if weighted {
                        out.insert(*index);
                    }
                }
            }
        }
    }
    out
}

/// The tangent of texcoord set `uvs` at each vertex of `p`, `w` 1 where the cross product of
/// normal and tangent runs along v; `None` where the set or a triangle does not fit `p`'s vertices.
fn tangents(p: &Primitive, uvs: &[[f32; 2]]) -> Option<Vec<[f32; 4]>> {
    let n = p.positions.len();
    if uvs.len() != n || p.indices.iter().any(|i| *i as usize >= n) {
        return None;
    }
    let one = Primitive {
        positions: p.positions.clone(),
        normals: p.normals.clone(),
        uvs: vec![uvs.to_vec()],
        indices: p.indices.clone(),
        ..Primitive::default()
    };
    // The shared builder gives the sign the other way about.
    let built = or_interchange::doctor::prop::tangents_of(&one);
    Some(
        built
            .into_iter()
            .map(|[x, y, z, w]| [x, y, z, -w])
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::headerless::{Channel, Vertices};

    fn bone(name: &str) -> Bone {
        Bone {
            name: name.to_string(),
            tag: 0,
            parent: None,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }

    /// A geometry of three vertices declaring weights 16 and blend indices 20 in the byte form.
    fn skinned() -> Geometry {
        let channel = |index, code, size| Channel {
            index,
            offset: 0,
            size,
            code,
        };
        Geometry {
            vertex_data: Vec::new(),
            index_data: Vec::new(),
            shader: 0,
            vertex_count: 3,
            channels: vec![
                channel(0, 6, 12),
                channel(16, 0x1C, 4),
                channel(20, 0x1E, 4),
            ],
            triangles: vec![[0, 1, 2]],
            positions: Ok(vec![[0.0; 3]; 3]),
            vertices: Vertices {
                weights: vec![(16, vec![[1.0, 0.0, 0.0, 0.0]; 3])],
                joints: vec![(20, vec![[1, 0, 0, 0]; 3])],
                ..Vertices::default()
            },
        }
    }

    fn quad(joints: [u16; 4], weights: [f32; 4]) -> Primitive {
        Primitive {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            joints: vec![joints; 4],
            weights: vec![weights; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..Primitive::default()
        }
    }

    #[test]
    fn a_new_count_s_joints_map_by_bone_name_onto_a_hash_palette_and_a_missing_one_is_named() {
        let (spine, head) = (name_hash32(b"SKEL_Spine0"), name_hash32(b"SKEL_Head"));
        let model = Model {
            geometries: vec![Ok(skinned())],
            palette: Ok(Palette::Hashes(vec![spine.min(head), spine.max(head)])),
            matrix_index: 0,
        };
        let bones = [bone("SKEL_Head"), bone("SKEL_Spine0"), bone("SKEL_Toe")];
        let wearer = Wearer {
            bones: &bones,
            own: None,
        };
        let g = skinned();
        let mut edit = GeometryEdit::default();
        // Joint 1's lane comes first once the unweighted lane before it is left out.
        let p = quad([2, 1, 0, 0], [0.0, 0.75, 0.25, 0.0]);
        fill(&mut edit, &g, &model, &p, &wearer).expect("mapped");
        let at = |hash| u16::from(hash == spine.max(head));
        assert_eq!(edit.joints, [(20, vec![[at(spine), at(head), 0, 0]; 4])]);
        assert_eq!(edit.weights, [(16, vec![[0.75, 0.25, 0.0, 0.0]; 4])]);

        let mut edit = GeometryEdit::default();
        let toe = quad([2, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        let refused = fill(&mut edit, &g, &model, &toe, &wearer).expect_err("refused");
        assert!(refused.contains("SKEL_Toe"), "{refused}");

        // At the geometry's own count its blend indices stay as it holds them.
        let mut edit = GeometryEdit::default();
        let mut same = quad([1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        same.positions.truncate(3);
        fill(&mut edit, &g, &model, &same, &wearer).expect("kept");
        assert!(edit.joints.is_empty() && edit.weights.is_empty());
    }

    #[test]
    fn a_mask_palette_takes_an_own_bone_the_model_uses_and_names_one_it_does_not() {
        let own = Skeleton {
            bones: ["Root", "Arm", "Hand"]
                .iter()
                .map(|n| or_io::gta6::headerless::Bone {
                    name: n.to_string(),
                    hash: name_hash32(n.as_bytes()),
                    parent: None,
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    translation: [0.0; 3],
                    scale: [1.0; 3],
                    inverse_bind: [[0.0; 3]; 4],
                })
                .collect(),
        };
        let model = Model {
            geometries: vec![Ok(skinned())],
            palette: Ok(Palette::Own),
            matrix_index: 0,
        };
        let bones = [bone("Root"), bone("Arm"), bone("Hand")];
        let wearer = Wearer {
            bones: &bones,
            own: Some(&own),
        };
        let g = skinned();
        let mut edit = GeometryEdit::default();
        fill(
            &mut edit,
            &g,
            &model,
            &quad([1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
            &wearer,
        )
        .expect("bone 1 is used");
        assert_eq!(edit.joints, [(20, vec![[1, 0, 0, 0]; 4])]);
        let refused = fill(
            &mut GeometryEdit::default(),
            &g,
            &model,
            &quad([2, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
            &wearer,
        )
        .expect_err("refused");
        assert!(refused.contains("Hand"), "{refused}");
    }
}
