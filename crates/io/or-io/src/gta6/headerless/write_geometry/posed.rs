//! A skinned or bone-attached drawable's box and sphere derived from its level-0 vertices posed at
//! bind: each vertex through its bones' bind world times inverse bind, weighted, as the skinning of
//! a posed drawable takes them.

use super::super::dictionary::entry_of;
use super::super::drawable::{drawable_at, Geometry, Model, Palette};
use super::super::fragment::{apply, bind_worlds, mul, M3};
use super::super::skeleton::Skeleton;
use super::{extremes, floats, put, put_box, sections, GeometryWriteError, Written};
use crate::container::rpf::{Rpf9Role, Rpf9Sections};
use crate::gta6::headerless::HeaderlessError;

use GeometryWriteError::Refused;

/// Each bone's bind world times its inverse bind, as a matrix and a translation.
fn skin_matrices(skeleton: &Skeleton) -> Vec<(M3, [f32; 3])> {
    let worlds = bind_worlds(&skeleton.bones);
    (worlds.iter().zip(&skeleton.bones))
        .map(|((r, t), bone)| {
            let c = bone.inverse_bind;
            let inverse: M3 = std::array::from_fn(|i| std::array::from_fn(|j| c[j][i]));
            let moved = apply(r, c[3]);
            (mul(r, &inverse), std::array::from_fn(|a| moved[a] + t[a]))
        })
        .collect()
}

/// The vertices of geometry `g` of `model` posed at bind on `skeleton`, the drawable's own.
// A geometry with blend indices follows the bones its palette names, its weights over the lanes
// that name one; a vertex none of whose weighted lanes names a bone follows its first lane's, and
// stays as stored where that names none. A geometry with none hangs off the model's matrix index.
pub fn bind_positions(g: &Geometry, model: &Model, skeleton: &Skeleton) -> Written<Vec<[f32; 3]>> {
    let positions = (g.positions.as_ref()).map_err(|_| {
        Refused("a level-0 geometry's positions do not read, so its bind pose is not known")
    })?;
    let skin = skin_matrices(skeleton);
    let own = |bone: usize| {
        (bone < skin.len()).then_some(bone).ok_or(Refused(
            "a bone index lies past the drawable's own skeleton, so its bind pose is not known",
        ))
    };
    let palette_bone = |index: u16| -> Written<Option<usize>> {
        match &model.palette {
            Ok(Palette::Own) => own(usize::from(index)).map(Some),
            Ok(Palette::Hashes(hashes)) => {
                let hash = *hashes
                    .get(usize::from(index))
                    .ok_or(Refused("a blend index lies past its model's palette"))?;
                // An entry of 0 names no bone.
                if hash == 0 {
                    return Ok(None);
                }
                (skeleton.bones.iter().position(|b| b.hash == hash))
                    .map(Some)
                    .ok_or(Refused(
                        "a palette entry names a bone the drawable's own skeleton does not hold, \
                         so its bind pose is not known",
                    ))
            }
            Err(_) => Err(Refused(
                "the model's palette does not read, so its bind pose is not known",
            )),
        }
    };
    let posed = |bone: usize, p: [f32; 3]| {
        let (m, t) = &skin[bone];
        let r = apply(m, p);
        std::array::from_fn::<f32, 3, _>(|a| r[a] + t[a])
    };
    let joints = |c: u8| {
        (g.vertices.joints.iter())
            .find(|(k, _)| *k == c)
            .map(|(_, j)| j)
    };
    let weights = |c: u8| {
        (g.vertices.weights.iter())
            .find(|(k, _)| *k == c)
            .map(|(_, w)| w)
    };
    let mut sets = Vec::new();
    if let Some(first) = joints(20) {
        sets.push((first, weights(16)));
        sets.extend(joints(21).map(|second| (second, weights(17))));
    }
    if sets.is_empty() {
        let bone = own(usize::from(model.matrix_index))?;
        return Ok(positions.iter().map(|p| posed(bone, *p)).collect());
    }
    let mut out = Vec::with_capacity(positions.len());
    for (i, p) in positions.iter().enumerate() {
        let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
        for (indices, set_weights) in &sets {
            let lanes = indices.get(i).copied().unwrap_or_default();
            let w = match set_weights {
                Some(w) => w.get(i).copied().unwrap_or_default(),
                None => [1.0, 0.0, 0.0, 0.0],
            };
            for (index, weight) in lanes.into_iter().zip(w) {
                if weight <= 0.0 {
                    continue;
                }
                let Some(bone) = palette_bone(index)? else {
                    continue;
                };
                let q = posed(bone, *p);
                sum = std::array::from_fn(|a| sum[a] + weight * q[a]);
                total += weight;
            }
        }
        out.push(match total > 0.0 {
            true => sum.map(|x| x / total),
            false => {
                let first = sets[0].0.get(i).map_or(0, |lanes| lanes[0]);
                palette_bone(first)?.map_or(*p, |bone| posed(bone, *p))
            }
        });
    }
    Ok(out)
}

/// The box and sphere of the drawable of `role` (or entry `code`) from its level 0 posed at bind.
// The drawable's own skeleton is `skeleton`; every other word is left as read.
pub fn rebox_bind_pose(
    s: &mut Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    skeleton: &Skeleton,
) -> Written<()> {
    let (root, drawable) = sections(&s.virtual_section, Some(&s.physical), |v, p| {
        let (root, bound_at_c8) = match (role, code) {
            (Rpf9Role::Drawable, None) => (0, true),
            (Rpf9Role::DrawableWithBound, None) => (v.to("drawable", 0x18)?, false),
            (Rpf9Role::Dictionary, Some(code)) => {
                let entry = entry_of(v, code)?.ok_or(HeaderlessError::NotInPlace(
                    "no entry of the dictionary has that code",
                ))?;
                (entry, true)
            }
            _ => {
                return Err(HeaderlessError::NotInPlace(
                    "the resource holds no drawable there",
                ))
            }
        };
        Ok((root, drawable_at(v, p, root, bound_at_c8)?))
    })?;
    let models = (drawable.levels[0].as_ref()).ok_or(Refused(
        "the drawable has no level 0 to derive its box from",
    ))?;
    let mut posed = Vec::new();
    for model in models {
        for g in &model.geometries {
            let g = g.as_ref().map_err(|_| {
                Refused(
                    "a level-0 geometry does not read, so the drawable's bind pose is not known",
                )
            })?;
            posed.extend(bind_positions(g, model, skeleton)?);
        }
    }
    if posed.is_empty() {
        return Err(Refused(
            "the drawable's level 0 holds no vertex to derive its box from",
        ));
    }
    let (min, max) = extremes(&posed);
    let v = &mut s.virtual_section;
    put_box(v, root + 0x30, (min, max), |_| 0.0)?;
    let centre: [f32; 3] =
        std::array::from_fn(|a| ((f64::from(min[a]) + f64::from(max[a])) / 2.0) as f32);
    let half: f64 = (0..3)
        .map(|a| (f64::from(max[a]) - f64::from(min[a])).powi(2))
        .sum::<f64>()
        .sqrt()
        / 2.0;
    put(
        v,
        root + 0x20,
        &floats(&[centre[0], centre[1], centre[2], half as f32]),
    )?;
    Ok(())
}
