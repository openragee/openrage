//! A glTF primitive's joints and weights as a GTA VI geometry's blend indices and weights, each
//! joint mapped by its bone's name onto the model's palette, and its texcoords' tangents. At the
//! geometry's own vertex count a vertex whose bones and weights the mesh leaves as the geometry
//! holds them keeps its stored lanes.

use std::collections::BTreeSet;

use or_interchange::{Bone, Primitive};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::headerless::{
    Geometry, GeometryEdit, Model, Palette, PaletteAdd, Skeleton, Vertices,
};

/// How far apart a vertex's weight for a bone in the mesh and in the geometry may lie and be one.
const SAME_WEIGHT: f32 = 1e-3;
const UNMEASURED: &str =
    "the model's palette is in neither measured form, so no joint maps onto it";

/// The bones a primitive's joints index and the drawable's own skeleton.
pub(super) struct Wearer<'a> {
    pub(super) bones: &'a [Bone],
    pub(super) own: Option<&'a Skeleton>,
}

/// The values a list of channels holds for channel `c`.
fn of<T>(list: &[(u8, Vec<T>)], c: u8) -> Option<&Vec<T>> {
    list.iter().find(|(k, _)| *k == c).map(|(_, values)| values)
}

/// `edit`'s weights, blend indices and texcoord tangents for geometry `g` of `model` from `p`;
/// `added` are bones of the own skeleton the model's mask has just taken.
pub(super) fn fill(
    edit: &mut GeometryEdit,
    g: &Geometry,
    model: &Model,
    p: &Primitive,
    wearer: &Wearer<'_>,
    added: &[u16],
) -> Result<(), String> {
    let n = p.positions.len();
    let same = n == g.vertex_count as usize;
    let declared = |k: u8| g.channels.iter().any(|c| c.index == k);
    // A texcoord's tangents stand while neither it, a position nor a triangle changes.
    let shape_kept = same
        && g.positions.as_ref().is_ok_and(|held| *held == p.positions)
        && g.triangles == edit.triangles;
    // Channels 9 to 11 are the tangents of texcoords 1 to 3, channels 29 to 31.
    for k in (9..=11).filter(|k| declared(*k)) {
        if let Some((_, uvs)) = edit.uvs.iter().find(|(c, _)| *c == k + 20) {
            if shape_kept && of(&g.vertices.uvs, k + 20) == Some(uvs) {
                continue;
            }
            if let Some(t) = tangents(p, uvs) {
                edit.uv_tangents.push((k, t));
            }
        }
    }
    if !declared(20) {
        return Ok(());
    }
    if p.joints.len() != n || p.weights.len() != n {
        if same {
            return Ok(());
        }
        return Err(
            "the mesh gives no joint and weight for each vertex, and the geometry's blend \
                    indices are kept only at its vertex count"
                .into(),
        );
    }
    let mut used = used(model);
    used.extend(added);
    let names_bone = |k: u16| match &model.palette {
        Ok(Palette::Hashes(hashes)) => hashes.get(usize::from(k)).is_some_and(|h| *h != 0),
        _ => true,
    };
    let v = &g.vertices;
    let at = |c: u8, i: usize| of(&v.joints, c).and_then(|l| l.get(i));
    let weight_at = |c: u8, i: usize| of(&v.weights, c).and_then(|l| l.get(i));
    let (mut lanes, mut second) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let (mut weights, mut second_weights) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut changed = !same;
    for (i, (joints, given)) in p.joints.iter().zip(&p.weights).enumerate() {
        let (mut out, mut w) = ([0u16; 4], [0.0f32; 4]);
        let mut set = 0;
        for (joint, weight) in joints.iter().zip(given) {
            if *weight > 0.0 {
                out[set] = index(*joint, model, wearer, &used)?;
                w[set] = *weight;
                set += 1;
            }
        }
        // With no weight channel a vertex takes one index, which holds all of its weight.
        if set > 1 && !declared(16) {
            return Err(format!(
                "vertex {i} weights {set} bones, and the geometry holds one blend index a vertex"
            ));
        }
        let mesh: Vec<(u16, f32)> = out.into_iter().zip(w).take(set).collect();
        let kept = same
            && alike(
                &reduced(held_lanes(v, i), &names_bone),
                &reduced(mesh, &names_bone),
            );
        match kept {
            true => {
                lanes.push(at(20, i).copied().unwrap_or(out));
                second.push(at(21, i).copied().unwrap_or_default());
                weights.push(weight_at(16, i).copied().unwrap_or(w));
                second_weights.push(weight_at(17, i).copied().unwrap_or_default());
            }
            false => {
                changed = true;
                lanes.push(out);
                second.push([0; 4]);
                weights.push(w);
                second_weights.push([0.0; 4]);
            }
        }
    }
    if !changed {
        return Ok(());
    }
    edit.joints.push((20, lanes));
    if declared(21) && (!same || of(&v.joints, 21).is_some()) {
        edit.joints.push((21, second));
    }
    if declared(16) {
        edit.weights.push((16, weights));
    }
    if declared(17) && (!same || of(&v.weights, 17).is_some()) {
        edit.weights.push((17, second_weights));
    }
    Ok(())
}

/// The bones the weighted joints of `p` name that the model's palette lacks: hashes a hash
/// palette takes, or bones of the own skeleton no geometry of the model weights yet.
pub(super) fn additions(
    g: &Geometry,
    model: &Model,
    p: &Primitive,
    wearer: &Wearer<'_>,
) -> Result<PaletteAdd, String> {
    let n = p.positions.len();
    let mut add = PaletteAdd::default();
    let indexed = g.channels.iter().any(|c| c.index == 20);
    if !indexed || p.joints.len() != n || p.weights.len() != n {
        return Ok(add);
    }
    let weighted: BTreeSet<u16> = (p.joints.iter().zip(&p.weights))
        .flat_map(|(j, w)| j.iter().zip(w).filter(|(_, w)| **w > 0.0).map(|(j, _)| *j))
        .collect();
    let used = used(model);
    for joint in weighted {
        let (name, own) = bone(joint, wearer)?;
        match &model.palette {
            Ok(Palette::Hashes(hashes)) => {
                let hash = hash_of(name, own, wearer);
                if !hashes.contains(&hash) && !add.hashes.contains(&hash) {
                    add.hashes.push(hash);
                }
            }
            Ok(Palette::Own) => {
                let at = (own.and_then(|at| u16::try_from(at).ok())).ok_or_else(|| {
                    format!("the joint {name} is no bone of the drawable's own skeleton")
                })?;
                if !used.contains(&at) && !add.bones.contains(&at) {
                    add.bones.push(at);
                }
            }
            Err(_) => return Err(UNMEASURED.into()),
        }
    }
    Ok(add)
}

/// The name of the bone `joint` names, and its place in the drawable's own skeleton.
fn bone<'a>(joint: u16, wearer: &Wearer<'a>) -> Result<(&'a str, Option<usize>), String> {
    let bones: &'a [Bone] = wearer.bones;
    let name = (bones.get(usize::from(joint)))
        .map(|b| b.name.as_str())
        .ok_or_else(|| format!("joint {joint} names no bone of the mesh's skeleton"))?;
    let own = wearer
        .own
        .and_then(|s| s.bones.iter().position(|b| b.name == name));
    Ok((name, own))
}

/// The hash a hash palette names a bone by: the own skeleton's word for it, else its name's.
fn hash_of(name: &str, own: Option<usize>, wearer: &Wearer<'_>) -> u32 {
    match (wearer.own, own) {
        (Some(s), Some(at)) => s.bones[at].hash,
        _ => name_hash32(name.as_bytes()),
    }
}

/// The palette index of the bone `joint` names: a hash palette's entry of its name hash, or a
/// mask's bone of the own skeleton that the model weights or `used` otherwise holds.
fn index(
    joint: u16,
    model: &Model,
    wearer: &Wearer<'_>,
    used: &BTreeSet<u16>,
) -> Result<u16, String> {
    let (name, own) = bone(joint, wearer)?;
    match &model.palette {
        Ok(Palette::Hashes(hashes)) => {
            let hash = hash_of(name, own, wearer);
            (hashes.iter().position(|h| *h == hash))
                .and_then(|at| u16::try_from(at).ok())
                .ok_or_else(|| format!("the joint {name} is a bone the model's palette lacks"))
        }
        Ok(Palette::Own) => (own.and_then(|at| u16::try_from(at).ok()))
            .filter(|at| used.contains(at))
            .ok_or_else(|| {
                format!(
                    "the joint {name} is no bone of the drawable's own skeleton that the model's \
                     mask holds"
                )
            }),
        Err(_) => Err(UNMEASURED.into()),
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

/// Vertex `i`'s lanes as the geometry holds them, each index with its weight; lane 0 alone, of
/// all the weight, where it has no weights.
fn held_lanes(v: &Vertices, i: usize) -> Vec<(u16, f32)> {
    let mut out = Vec::new();
    for (j, w) in [(20u8, 16u8), (21, 17)] {
        let Some(joints) = of(&v.joints, j).and_then(|l| l.get(i)) else {
            continue;
        };
        match of(&v.weights, w).and_then(|l| l.get(i)) {
            Some(weights) => out.extend(joints.iter().copied().zip(weights.iter().copied())),
            None if j == 20 => out.push((joints[0], 1.0)),
            None => {}
        }
    }
    out
}

/// `lanes` as the export gives them: the four heaviest that name a bone, their weights summing to
/// one, in index order.
fn reduced(mut lanes: Vec<(u16, f32)>, names_bone: &impl Fn(u16) -> bool) -> Vec<(u16, f32)> {
    lanes.retain(|(k, w)| *w > 0.0 && names_bone(*k));
    lanes.sort_by(|a, b| b.1.total_cmp(&a.1));
    lanes.truncate(4);
    let sum: f32 = lanes.iter().map(|(_, w)| w).sum();
    for (_, w) in &mut lanes {
        *w /= sum;
    }
    lanes.sort_by_key(|(k, _)| *k);
    lanes
}

fn alike(a: &[(u16, f32)], b: &[(u16, f32)]) -> bool {
    a.len() == b.len()
        && (a.iter().zip(b)).all(|((i, x), (j, y))| i == j && (x - y).abs() <= SAME_WEIGHT)
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
    use or_io::gta6::headerless::Channel;

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
        fill(&mut edit, &g, &model, &p, &wearer, &[]).expect("mapped");
        let at = |hash| u16::from(hash == spine.max(head));
        assert_eq!(edit.joints, [(20, vec![[at(spine), at(head), 0, 0]; 4])]);
        assert_eq!(edit.weights, [(16, vec![[0.75, 0.25, 0.0, 0.0]; 4])]);
        let none = additions(&g, &model, &p, &wearer).expect("read");
        assert!(none.is_empty(), "{none:?}");

        let mut edit = GeometryEdit::default();
        let toe = quad([2, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        let refused = fill(&mut edit, &g, &model, &toe, &wearer, &[]).expect_err("refused");
        assert!(refused.contains("SKEL_Toe"), "{refused}");
        let add = additions(&g, &model, &toe, &wearer).expect("read");
        assert_eq!(add.hashes, [name_hash32(b"SKEL_Toe")]);
        assert!(add.bones.is_empty());
    }

    #[test]
    fn at_its_own_count_a_vertex_the_mesh_leaves_keeps_its_lanes_and_another_is_written() {
        let (spine, head) = (name_hash32(b"SKEL_Spine0"), name_hash32(b"SKEL_Head"));
        let model = Model {
            geometries: vec![Ok(skinned())],
            palette: Ok(Palette::Hashes(vec![spine.min(head), spine.max(head)])),
            matrix_index: 0,
        };
        let bones = [bone("SKEL_Head"), bone("SKEL_Spine0")];
        let wearer = Wearer {
            bones: &bones,
            own: None,
        };
        let g = skinned();
        // Palette entry 1, which every vertex weights whole, is the greater hash.
        let (held, other) = match spine > head {
            true => (1, 0),
            false => (0, 1),
        };
        let mut same = quad([held, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        same.positions.truncate(3);
        same.joints.truncate(3);
        same.weights.truncate(3);
        let mut edit = GeometryEdit::default();
        fill(&mut edit, &g, &model, &same, &wearer, &[]).expect("kept");
        assert!(
            edit.joints.is_empty() && edit.weights.is_empty(),
            "{edit:?}"
        );

        // The glTF's own lanes are applied at the same count: vertex 2 moves to the other bone.
        same.joints[2] = [other, 0, 0, 0];
        let mut edit = GeometryEdit::default();
        fill(&mut edit, &g, &model, &same, &wearer, &[]).expect("re-skinned");
        let mut lanes = vec![[1, 0, 0, 0]; 3];
        lanes[2] = [0, 0, 0, 0];
        assert_eq!(edit.joints, [(20, lanes)]);
        assert_eq!(edit.weights, [(16, vec![[1.0, 0.0, 0.0, 0.0]; 3])]);

        // A mesh with no joints at the geometry's count keeps them as it holds them.
        same.joints.clear();
        same.weights.clear();
        let mut edit = GeometryEdit::default();
        fill(&mut edit, &g, &model, &same, &wearer, &[]).expect("kept");
        assert!(edit.joints.is_empty());
    }

    #[test]
    fn a_mask_palette_takes_an_own_bone_the_model_uses_or_has_just_set() {
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
        let arm = quad([1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        fill(&mut edit, &g, &model, &arm, &wearer, &[]).expect("bone 1 is used");
        assert_eq!(edit.joints, [(20, vec![[1, 0, 0, 0]; 4])]);
        let hand = quad([2, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        let refused = fill(
            &mut GeometryEdit::default(),
            &g,
            &model,
            &hand,
            &wearer,
            &[],
        )
        .expect_err("refused");
        assert!(refused.contains("Hand"), "{refused}");
        let add = additions(&g, &model, &hand, &wearer).expect("read");
        assert!(add.hashes.is_empty());
        assert_eq!(add.bones, [2]);
        let mut edit = GeometryEdit::default();
        fill(&mut edit, &g, &model, &hand, &wearer, &add.bones).expect("bone 2 is set");
        assert_eq!(edit.joints, [(20, vec![[2, 0, 0, 0]; 4])]);
    }
}
