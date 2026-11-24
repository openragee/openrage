//! GTA VI animations posed on GTA VI skeletons: a DOF moves the bone whose name's hash is its id,
//! and a geometry's vertices follow the bones their blend indices name.
//!
//! A bone track's value replaces the bone's local bind transform, as GTA V's tracks of the same
//! names do, and the absolute mover tracks are the pose's root motion; every other track poses
//! nothing and is reported by name.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use or_io::gta5::{CrBoneData, CrSkeletonData, Vec4V};
use or_io::gta6::anim::{AnimError, Animation, ClipEntry, ClipGroup, Sampler};
use or_io::gta6::headerless;

use crate::pose::{BoneTransform, Pose};
use crate::skeleton::Skeleton;
use crate::skin::{SkinnedMesh, VertexInfluences, MAX_INFLUENCES};

/// The skeleton the pose code takes, its bones in the order the GTA VI skeleton stores them.
pub fn skeleton(source: &headerless::Skeleton) -> Skeleton {
    let v4 = |[x, y, z]: [f32; 3], w| Vec4V {
        x,
        y,
        z,
        w,
        ..Vec4V::default()
    };
    let parent = |b: &headerless::Bone| b.parent.map_or(-1, |p| p as i16);
    let bones = source
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let [x, y, z, w] = b.rotation;
            CrBoneData {
                rotation: v4([x, y, z], w),
                translation: v4(b.translation, 0.0),
                scale: v4(b.scale, 0.0),
                next_index: -1,
                parent_index: parent(b),
                name: b.name.clone(),
                index: i as u16,
                bone_id: i as u16,
                ..CrBoneData::default()
            }
        })
        .collect();
    let mut skeleton = Skeleton::build(&CrSkeletonData {
        parent_indices: source.bones.iter().map(parent).collect(),
        bones,
        ..CrSkeletonData::default()
    });
    // The skin matrices take the inverse binds the file states.
    skeleton.inverse_bind = source.bones.iter().map(inverse_bind).collect();
    skeleton
}

fn inverse_bind(bone: &headerless::Bone) -> Mat4 {
    let [x, y, z, w] = bone.inverse_bind.map(Vec3::from);
    Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), w.extend(1.0))
}

/// Why a geometry's vertices do not join the skeleton they are posed on, so it is drawn at bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unjoined {
    /// The model's palette list is in neither measured form.
    PaletteNotRead,
    /// A vertex's index is not below the palette's count.
    PastPalette { index: u16, count: usize },
    /// Every palette entry of a vertex's weighted lanes is the name hash of no bone of the skeleton,
    /// of the drawable's own skeleton or of its extra skeletons.
    NoSuchHash { hash: u32 },
    /// The index is a bone of the drawable's own skeleton, which was not read.
    NoOwnSkeleton,
    /// A vertex's index is not below the bone count of the drawable's own skeleton.
    PastOwnSkeleton { index: u16, count: usize },
    /// A bone of the drawable's own skeleton a vertex uses has a name no bone of the skeleton has.
    NoSuchOwnBone { name: String },
}

/// The root of a face skeleton, whose bones the body places by its head.
const FACIAL_ROOT: u32 = or_io::gta6::hash::name_hash32(b"FACIAL_C_FacialRoot");
const SKEL_HEAD: u32 = or_io::gta6::hash::name_hash32(b"SKEL_Head");

/// The influences of a geometry's vertices on the bones of `skeleton`, the body it is posed on, up
/// to eight a vertex.
pub fn skin(
    vertices: &headerless::Vertices,
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    model: &headerless::Model,
    own: Option<&headerless::Skeleton>,
    skeleton: &headerless::Skeleton,
) -> Result<SkinnedMesh, Unjoined> {
    skin_with_extra(vertices, positions, normals, model, own, &[], skeleton)
}

/// [`skin`], with `extra` the drawable's extra skeletons, whose bones its palettes may also name.
pub fn skin_with_extra(
    vertices: &headerless::Vertices,
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    model: &headerless::Model,
    own: Option<&headerless::Skeleton>,
    extra: &[headerless::Skeleton],
    skeleton: &headerless::Skeleton,
) -> Result<SkinnedMesh, Unjoined> {
    // An index goes through the model's palette: a hash palette names the bone by its name hash,
    // in `skeleton` or else in `own`, the drawable's own skeleton; any other names the bone of
    // `own` by position. A bone of `own` joins `skeleton` by its name hash, else moves with its
    // nearest ancestor that does, and a face skeleton's bone with none moves with `SKEL_Head`:
    // its bind is the head's times the face skeleton's own, which no clip of the body moves. A
    // geometry with no blend indices hangs off the model's matrix index in `own`.
    // A hash only `extra` holds names a bone no known clip or constraint drives, so it stays at
    // its bind: a vertex whose every weighted lane names one is drawn at bind.
    // ponytail: a vertex that mixes such a lane with body lanes follows the body lanes alone; an
    // identity slot in the skin palette would hold the extra lane's share at bind.
    let mut by_hash = HashMap::with_capacity(skeleton.bones.len());
    for (i, bone) in skeleton.bones.iter().enumerate() {
        by_hash.entry(bone.hash).or_insert(i as u16);
    }
    // A skeleton joined to itself keeps every index, though it holds a hash twice.
    let same = own.is_some_and(|own| own == skeleton);
    let own_joined = |own: &headerless::Skeleton, mut at: usize| -> Option<u16> {
        // Each step goes to a parent, so a skeleton's length bounds a walk that a cycle would not end.
        for _ in 0..own.bones.len() {
            let bone = own.bones.get(at)?;
            if let Some(&joined) = by_hash.get(&bone.hash) {
                return Some(joined);
            }
            match bone.parent {
                Some(parent) => at = usize::from(parent),
                None => break,
            }
        }
        let face = own.bones.first().is_some_and(|b| b.hash == FACIAL_ROOT);
        by_hash.get(&SKEL_HEAD).copied().filter(|_| face)
    };
    let own_bone = |index: u16| -> Result<u16, Unjoined> {
        let own = own.ok_or(Unjoined::NoOwnSkeleton)?;
        let bone = own
            .bones
            .get(usize::from(index))
            .ok_or(Unjoined::PastOwnSkeleton {
                index,
                count: own.bones.len(),
            })?;
        if same {
            return Ok(index);
        }
        own_joined(own, usize::from(index)).ok_or_else(|| Unjoined::NoSuchOwnBone {
            name: bone.name.clone(),
        })
    };
    let in_extra = |hash: u32| {
        let mut bones = extra.iter().flat_map(|s| &s.bones);
        bones.any(|b| b.hash == hash)
    };
    // `Ok(None)` where only `extra` holds the entry, and `Err` with the hash where no skeleton
    // does: either lane's weight is dropped.
    let palette_bone = |index: u16| -> Result<Result<Option<u16>, u32>, Unjoined> {
        match &model.palette {
            Ok(headerless::Palette::Hashes(hashes)) => {
                let hash = *hashes
                    .get(usize::from(index))
                    .ok_or(Unjoined::PastPalette {
                        index,
                        count: hashes.len(),
                    })?;
                // An entry of 0 names no bone, though a skeleton may hold a bone of hash 0.
                if hash == 0 {
                    return Ok(Err(0));
                }
                let in_own = || {
                    let own = own?;
                    own_joined(own, own.bones.iter().position(|b| b.hash == hash)?)
                };
                match by_hash.get(&hash).copied().or_else(in_own) {
                    Some(bone) => Ok(Ok(Some(bone))),
                    None if in_extra(hash) => Ok(Ok(None)),
                    None => Ok(Err(hash)),
                }
            }
            Ok(headerless::Palette::Own) => own_bone(index).map(|bone| Ok(Some(bone))),
            Err(_) => Err(Unjoined::PaletteNotRead),
        }
    };
    let channel = |c: u8| {
        vertices
            .joints
            .iter()
            .find(|(k, _)| *k == c)
            .map(|(_, j)| j)
    };
    let weights = |c: u8| {
        vertices
            .weights
            .iter()
            .find(|(k, _)| *k == c)
            .map(|(_, w)| w)
    };
    // Each set of four indices and the weights of their lanes; no weights is one index of all.
    let mut sets = Vec::new();
    if let Some(first) = channel(20) {
        sets.push((first, weights(16)));
        sets.extend(channel(21).map(|second| (second, weights(17))));
    }
    let rigid = if sets.is_empty() {
        Some(own_bone(u16::from(model.matrix_index))?)
    } else {
        None
    };

    let mut joined = HashMap::new();
    let mut mesh = SkinnedMesh {
        influences: Vec::with_capacity(positions.len()),
        bind_positions: positions.to_vec(),
        bind_normals: (0..positions.len())
            .map(|i| normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]))
            .collect(),
        ..SkinnedMesh::default()
    };
    let mut used = std::collections::BTreeSet::new();
    for i in 0..positions.len() {
        if let Some(bone) = rigid {
            mesh.influences.push(VertexInfluences::rigid(bone));
            used.insert(bone);
            continue;
        }
        let mut out = VertexInfluences::UNBOUND;
        let mut total = 0.0;
        let (mut first, mut missing, mut at_bind) = (None, None, false);
        for (indices, set_weights) in &sets {
            let lanes = indices.get(i).copied().unwrap_or_default();
            let w = match set_weights {
                Some(w) => w.get(i).copied().unwrap_or_default(),
                None => [1.0, 0.0, 0.0, 0.0],
            };
            first.get_or_insert(lanes[0]);
            for (index, weight) in lanes.into_iter().zip(w) {
                if weight <= 0.0 || usize::from(out.count) == MAX_INFLUENCES {
                    continue;
                }
                let bone = match joined.get(&index) {
                    Some(&bone) => bone,
                    None => {
                        let bone = palette_bone(index)?;
                        joined.insert(index, bone);
                        bone
                    }
                };
                let bone = match bone {
                    Ok(Some(bone)) => bone,
                    Ok(None) => {
                        at_bind = true;
                        continue;
                    }
                    Err(hash) => {
                        missing.get_or_insert(hash);
                        continue;
                    }
                };
                out.bones[usize::from(out.count)] = bone;
                out.weights[usize::from(out.count)] = weight;
                out.count += 1;
                total += weight;
            }
        }
        if out.count == 0 && at_bind {
            mesh.influences.push(VertexInfluences::UNBOUND);
            continue;
        }
        if let (0, Some(hash)) = (out.count, missing) {
            return Err(Unjoined::NoSuchHash { hash });
        }
        if out.count == 0 {
            mesh.degenerate_weight_vertices += 1;
            out = match palette_bone(first.unwrap_or(0))? {
                Ok(Some(bone)) => VertexInfluences::rigid(bone),
                Ok(None) => VertexInfluences::UNBOUND,
                Err(hash) => return Err(Unjoined::NoSuchHash { hash }),
            };
        } else {
            for w in &mut out.weights[..usize::from(out.count)] {
                *w /= total;
            }
        }
        used.extend(&out.bones[..usize::from(out.count)]);
        mesh.influences.push(out);
    }
    mesh.bones_used = used.into_iter().collect();
    Ok(mesh)
}

/// The DOFs that move one bone: indices into the animation's DOFs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoneDofs {
    pub bone: u16,
    pub translation: Option<usize>,
    pub rotation: Option<usize>,
    pub scale: Option<usize>,
}

/// Why a DOF of the animation moves no bone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// A bone track whose id is no bone's name hash in the skeleton.
    NoSuchBone { dof: usize, id: u32 },
    /// A track no bone's transform is: generic, camera, facial, a delta mover or unnamed.
    NotABoneTrack { dof: usize, track: u32 },
}

/// Which DOFs of an animation move which bones of a skeleton, and which move none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Binding {
    /// By bone index, rising.
    pub bones: Vec<BoneDofs>,
    /// `moverAbsoluteTranslation`: the root's position, absolute in the animation's time.
    pub mover_translation: Option<usize>,
    /// `moverAbsoluteRotation`: the root's orientation.
    pub mover_rotation: Option<usize>,
    pub skipped: Vec<Skipped>,
}

impl Binding {
    pub fn build(skeleton: &headerless::Skeleton, animation: &Animation) -> Self {
        let mut index = HashMap::with_capacity(skeleton.bones.len());
        for (i, bone) in skeleton.bones.iter().enumerate() {
            index.entry(bone.hash).or_insert(i as u16);
        }
        let mut by_bone: HashMap<u16, BoneDofs> = HashMap::new();
        let mut skipped = Vec::new();
        let (mut mover_translation, mut mover_rotation) = (None, None);
        for (d, dof) in animation.dofs.iter().enumerate() {
            let slot = match dof.track_name() {
                Some("boneTranslation") => 0,
                Some("boneRotation") => 1,
                Some("boneScale") => 2,
                // The delta movers' value convention is not known, so only the absolute ones move.
                Some("moverAbsoluteTranslation") => {
                    mover_translation.get_or_insert(d);
                    continue;
                }
                Some("moverAbsoluteRotation") => {
                    mover_rotation.get_or_insert(d);
                    continue;
                }
                _ => {
                    skipped.push(Skipped::NotABoneTrack {
                        dof: d,
                        track: dof.track,
                    });
                    continue;
                }
            };
            let Some(&bone) = index.get(&dof.id) else {
                skipped.push(Skipped::NoSuchBone { dof: d, id: dof.id });
                continue;
            };
            let entry = by_bone.entry(bone).or_insert(BoneDofs {
                bone,
                translation: None,
                rotation: None,
                scale: None,
            });
            let field = match slot {
                0 => &mut entry.translation,
                1 => &mut entry.rotation,
                _ => &mut entry.scale,
            };
            field.get_or_insert(d);
        }
        let mut bones: Vec<BoneDofs> = by_bone.into_values().collect();
        bones.sort_unstable_by_key(|b| b.bone);
        Self {
            bones,
            mover_translation,
            mover_rotation,
            skipped,
        }
    }

    /// The skeleton's bind pose with every bound bone's tracks at `time` seconds, the root moved from 0.
    pub fn pose(
        &self,
        skeleton: &Skeleton,
        animation: &Animation,
        time: f32,
    ) -> Result<Pose, AnimError> {
        pose_layers(skeleton, &[(self, animation, 0.0, time)])
    }

    fn mover_at(&self, sampler: &mut Sampler, time: f32) -> Result<(Vec3, Quat), AnimError> {
        let v = (self.mover_translation)
            .map(|d| vector(sampler, d, time))
            .transpose()?
            .flatten();
        let q = (self.mover_rotation)
            .map(|d| rotation(sampler, d, time))
            .transpose()?
            .flatten();
        Ok((v.unwrap_or(Vec3::ZERO), q.unwrap_or(Quat::IDENTITY)))
    }

    fn pose_onto(
        &self,
        pose: &mut Pose,
        animation: &Animation,
        start: f32,
        time: f32,
    ) -> Result<(), AnimError> {
        // One sampler for the pose: a sample alone walks its whole block.
        let sampler = &mut animation.sampler();
        // The mover is absolute and a window need not start at the origin: the root moves by
        // the mover's change since the window's start, in the start's frame.
        if self.mover_translation.is_some() || self.mover_rotation.is_some() {
            let from = self.mover_at(sampler, start)?;
            let to = self.mover_at(sampler, time)?;
            pose.root_motion = from_window_start(from, to);
        }
        for bone in &self.bones {
            let Some(local) = pose.local_mut(bone.bone) else {
                continue;
            };
            if let Some(dof) = bone.translation {
                if let Some(v) = vector(sampler, dof, time)? {
                    local.translation = v;
                }
            }
            if let Some(dof) = bone.rotation {
                if let Some(q) = rotation(sampler, dof, time)? {
                    local.rotation = q;
                }
            }
            if let Some(dof) = bone.scale {
                if let Some(v) = vector(sampler, dof, time)? {
                    local.scale = v;
                }
            }
        }
        Ok(())
    }
}

fn from_window_start(from: (Vec3, Quat), to: (Vec3, Quat)) -> BoneTransform {
    let back = from.1.inverse();
    BoneTransform {
        translation: back * (to.0 - from.0),
        rotation: (back * to.1).normalize(),
        scale: Vec3::ONE,
    }
}

/// The bind pose with each `(binding, animation, window start, time)` layer applied in order.
// A group clip's layers drive disjoint DOFs, so their order changes nothing.
pub fn pose_layers(
    skeleton: &Skeleton,
    layers: &[(&Binding, &Animation, f32, f32)],
) -> Result<Pose, AnimError> {
    let mut pose = Pose::bind(skeleton);
    for (binding, animation, start, time) in layers {
        binding.pose_onto(&mut pose, animation, *start, *time)?;
    }
    Ok(pose)
}

/// The world and skin matrices of `pose`, both moved by its root motion.
pub fn posed_matrices(pose: &Pose, skeleton: &Skeleton) -> (Vec<Mat4>, Vec<Mat4>) {
    let root = pose.root_motion.to_matrix();
    let (mut world, mut skin) = (Vec::new(), Vec::new());
    pose.pose_into(skeleton, &mut world, &mut skin);
    for m in world.iter_mut().chain(skin.iter_mut()) {
        *m = root * *m;
    }
    (world, skin)
}

/// One entry of a group clip as it plays at a time of the clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub group: usize,
    pub entry: usize,
    /// Seconds into the entry's animation: `start + rate × (clip time − group start)`.
    pub time: f32,
}

/// Where each group starts on the clip's timeline: groups play one after another.
pub fn group_starts(groups: &[ClipGroup]) -> Vec<f32> {
    groups
        .iter()
        .scan(0.0, |at, g| {
            let start = *at;
            *at += g.duration;
            Some(start)
        })
        .collect()
}

/// The layers a group clip plays at `time` seconds of the clip, each entry of the group holding it.
// The last group plays at or past the clip's end.
pub fn layers_at(groups: &[ClipGroup], time: f32) -> Vec<Layer> {
    if groups.is_empty() {
        return Vec::new();
    }
    let starts = group_starts(groups);
    let group = starts.iter().rposition(|s| time >= *s).unwrap_or(0);
    let local = (time - starts[group]).clamp(0.0, groups[group].duration.max(0.0));
    let entries = groups[group].entries.iter().enumerate();
    entries
        .map(|(entry, e)| Layer {
            group,
            entry,
            time: (e.start + e.rate * local).clamp(e.start.min(e.end), e.end.max(e.start)),
        })
        .collect()
}

/// The clip times at which the frames of `entry`'s window play, its group starting at `group_start`.
// None when the entry's rate does not advance.
pub fn entry_frame_times(group_start: f32, entry: &ClipEntry, animation: &Animation) -> Vec<f32> {
    let Some(rate) = animation.rate() else {
        return Vec::new();
    };
    if !(entry.rate > 0.0 && entry.rate.is_finite()) {
        return Vec::new();
    }
    (0..animation.frames)
        .map(|f| f32::from(f) / rate)
        .filter(|t| (entry.start..=entry.end).contains(t))
        .map(|t| group_start + (t - entry.start) / entry.rate)
        .collect()
}

fn vector(sampler: &mut Sampler, dof: usize, time: f32) -> Result<Option<Vec3>, AnimError> {
    let s = sampler.sample(dof, time)?;
    let [x, y, z, _] = s.components;
    let v = Vec3::new(x, y, z);
    Ok((s.count >= 3 && v.is_finite()).then_some(v))
}

fn rotation(sampler: &mut Sampler, dof: usize, time: f32) -> Result<Option<Quat>, AnimError> {
    let s = sampler.sample(dof, time)?;
    let [x, y, z, w] = s.components;
    let q = Quat::from_xyzw(x, y, z, w);
    Ok((s.count == 4 && q.is_finite() && q.length_squared() > 1e-12).then(|| q.normalize()))
}

#[cfg(test)]
#[path = "gta6_tests.rs"]
mod tests;
