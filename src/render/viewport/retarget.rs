//! Which mesh is paired with which skeleton, and moving the skinning across.

use crate::render::meshes::MeshUpload;
use crate::state::AppState;

/// A key over the mesh sources and the skeleton source, so the mesh cache
/// rebuilds when the pairing changes, a source's contents do, or the skeleton source's own model
/// is hidden or drawn.
///
/// ⚠ Never zero for a live animation scene — `| 1` — because zero is what a
/// non-animation workspace returns and the two must not collide.
///
/// ⚠ Each source's own count of changes: the active document, whose count the
/// mesh key holds already, is often not a source.
///
/// ⚠ The skeleton the session poses is in it as well, not only the one the
/// scene names. A frame ticks the animation first and draws last, so an undo
/// in the menu moves a source's count after the session was built, and the
/// meshes retargeted that frame are retargeted onto the old bones. The session
/// is built again the next tick, and only this part of the key moves then.
pub(super) fn anim_scene_signature(state: &AppState) -> u64 {
    use std::hash::{Hash, Hasher};

    if state.active_workspace != crate::ui::dock::Workspace::Animation {
        return 0;
    }
    let scene = &state.preview_scene;
    let contents = |doc| state.document(doc).map(|d| d.payload_epoch());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for source in &scene.mesh_sources {
        source.document.hash(&mut hasher);
        source.entry.hash(&mut hasher);
        contents(source.document).hash(&mut hasher);
    }
    match scene.skeleton_source {
        Some(source) => {
            source.document().hash(&mut hasher);
            contents(source.document()).hash(&mut hasher);
            if let crate::state::SkeletonSource::DictionaryEntry(_, index) = source {
                index.hash(&mut hasher);
            }
        }
        None => 0u8.hash(&mut hasher),
    }
    scene.draw_skeleton_model.hash(&mut hasher);
    state.anim.skeleton_built_from().hash(&mut hasher);
    hasher.finish() | 1
}

/// Move every skinned upload onto the skeleton the preview will pose against.
///
/// ⚠ **Only where the drawable carried the skeleton its palette indexes.** A
/// mesh with no palette tags is left alone and contributes nothing to the
/// report, because the file's own index is the only statement there is — and
/// most clothing components are in that case. Returns `None` where nothing
/// could be retargeted at all, which is what a panel should say rather than
/// "0 of 0 resolved".
pub(super) fn retarget_uploads(
    uploads: &mut [MeshUpload],
    state: &AppState,
) -> Option<or_anim::PaletteRetarget> {
    let target = &state.anim.binding.as_ref()?.skeleton;
    let mut total: Option<or_anim::PaletteRetarget> = None;
    for upload in uploads.iter_mut() {
        let Some(one) = upload.retarget_onto(target) else {
            continue;
        };
        let acc = total.get_or_insert_with(Default::default);
        acc.by_tag += one.by_tag;
        acc.slots.extend(one.slots);
        acc.unresolved.extend(one.unresolved);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton_of(tags: &[u16]) -> or_anim::Skeleton {
        use or_io::gta5::{CrBoneData, CrSkeletonData, Vec4V};

        let bones: Vec<CrBoneData> = tags
            .iter()
            .enumerate()
            .map(|(i, tag)| CrBoneData {
                rotation: Vec4V {
                    w: 1.0,
                    ..Vec4V::default()
                },
                scale: Vec4V {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                    ..Vec4V::default()
                },
                parent_index: if i == 0 { -1 } else { 0 },
                next_index: -1,
                index: i as u16,
                bone_id: *tag,
                ..CrBoneData::default()
            })
            .collect();
        or_anim::Skeleton::build(&CrSkeletonData {
            parent_indices: (0..tags.len())
                .map(|i| if i == 0 { -1 } else { 0 })
                .collect(),
            bones,
            ..CrSkeletonData::default()
        })
    }

    fn bound_to(tags: &[u16]) -> AppState {
        let mut state = AppState::new();
        state.anim.binding = Some(or_anim::BoundClip {
            skeleton: skeleton_of(tags),
            report: Default::default(),
            clip_name: "t".into(),
            duration: 1.0,
            frame_rate: None,
        });
        state
    }

    fn skinned_upload(palette: &[(u16, u16)]) -> MeshUpload {
        MeshUpload {
            bump: false,
            vertices: Vec::new(),
            indices: Vec::new(),
            texture_name: None,
            skinning: Some(or_anim::SkinnedMesh {
                influences: vec![or_anim::VertexInfluences::rigid(palette[0].0)],
                bones_used: vec![palette[0].0],
                ..Default::default()
            }),
            palette_bones: palette
                .iter()
                .map(|(index, tag)| or_anim::PaletteBone {
                    index: *index,
                    tag: *tag,
                })
                .collect(),
            channels: Vec::new(),
            source: None,
        }
    }

    fn first_bone(upload: &MeshUpload) -> u16 {
        upload.skinning.as_ref().unwrap().influences[0].bones[0]
    }

    #[test]
    fn the_palette_retarget_sums_across_every_upload() {
        let state = bound_to(&[300, 100, 200]);
        let mut uploads = vec![skinned_upload(&[(0, 100)]), skinned_upload(&[(1, 200)])];
        let out = retarget_uploads(&mut uploads, &state).expect("both carried tags");

        assert_eq!(out.by_tag, 2);
        assert_eq!(out.slots, vec![1, 2], "tag 100 is index 1, tag 200 is 2");
        assert!(out.unresolved.is_empty());
        assert_eq!(first_bone(&uploads[0]), 1);
        assert_eq!(first_bone(&uploads[1]), 2);
    }

    /// ⚠ **No tags is not "nothing resolved".** Most clothing components
    /// carry no skeleton, so the file's own index stands — a report of
    /// `0 of 0` would read as a failure.
    #[test]
    fn a_mesh_with_no_palette_tags_is_left_alone_and_reported_as_absent() {
        let state = bound_to(&[300, 100]);
        let mut uploads = vec![skinned_upload(&[(0, 100)])];
        uploads[0].palette_bones.clear();
        assert!(retarget_uploads(&mut uploads, &state).is_none());
        assert_eq!(first_bone(&uploads[0]), 0);
    }

    #[test]
    fn without_a_bound_skeleton_no_upload_is_retargeted() {
        let state = AppState::new();
        let mut uploads = vec![skinned_upload(&[(0, 100)])];
        assert!(retarget_uploads(&mut uploads, &state).is_none());
        assert_eq!(first_bone(&uploads[0]), 0);
    }
}
