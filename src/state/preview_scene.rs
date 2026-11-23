//! What the viewport is showing, in the application's own vocabulary.
//!
//! The half that answers *which skeleton does this clip want* has no
//! document in it and lives in [`or_anim::resolve`]; what stays here is the
//! part keyed by `DocumentId`.

use std::collections::BTreeSet;

use or_anim::{BoundClip, ClipPlayer, Pose, Skeleton};
use or_io::gta5::CrSkeletonData;

use super::document::{DocPayload, DocumentId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkeletonSource {
    Fragment(DocumentId),
    Drawable(DocumentId),
    DictionaryEntry(DocumentId, usize),
}

impl SkeletonSource {
    pub fn document(self) -> DocumentId {
        match self {
            Self::Fragment(id) | Self::Drawable(id) => id,
            Self::DictionaryEntry(id, _) => id,
        }
    }

    pub fn kind_label(self) -> &'static str {
        match self {
            Self::Fragment(_) => "fragment",
            Self::Drawable(_) => "drawable",
            Self::DictionaryEntry(_, _) => "dictionary entry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshSource {
    pub document: DocumentId,
    pub entry: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct MeshCandidate {
    pub source: MeshSource,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipSelection {
    pub document: DocumentId,
    pub clip_key: u32,
}

/// What a click in the preview landed on in a document other than the active one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub document: DocumentId,
    pub label: String,
    /// The Outline row of `document` that names it.
    pub row: u64,
}

#[derive(Debug, Clone)]
pub struct PreviewScene {
    pub clip: Option<ClipSelection>,
    pub skeleton_source: Option<SkeletonSource>,
    pub mesh_sources: Vec<MeshSource>,
    pub texture_sources: Vec<DocumentId>,
    pub allow_mismatched_skeleton: bool,
    /// The user has chosen a skeleton, so stop choosing one for them.
    ///
    /// ⚠ Not the same as `skeleton_source.is_some()`. Without this flag an
    /// automatic choice cannot tell "nothing picked yet" from "the user
    /// picked none", so picking none would be overwritten on the next frame
    /// and the option would be unselectable.
    pub skeleton_pinned: bool,
    /// Whether the Animation preview draws the skeleton source's bones, posed by the clip or at rest.
    pub show_bones: bool,
    /// Whether the viewport draws the bones of a drawable viewed outside the Animation workspace, at rest.
    pub show_rest_bones: bool,
    pub show_bind_pose_ghost: bool,
    pub apply_root_motion: bool,
    /// Which vertex channel the `Channel` shading mode shows.
    ///
    /// ⚠ Read at mesh-build time, so changing it has to invalidate the mesh
    /// cache: the channel's value is packed into the vertex rather than
    /// selected in the shader.
    pub debug_channel: Option<crate::render::debug_channel::DebugChannel>,
    /// The level of detail the viewport draws, of a drawable that has models at it.
    pub lod: usize,
    /// Whether the viewport draws the bound a drawable embeds.
    pub show_collision: bool,
    /// Whether a GTA VI weapon is drawn as its parts, each on its bone, in place of its own mesh.
    pub assemble_parts: bool,
    /// Whether the preview draws the skeleton source's own model while no mesh source is chosen, or the chosen sources draw nothing.
    ///
    /// ⚠ Ticked unless the user unticked it, and kept in the preferences: a ped's `.yft` holds a
    /// 25 metre plane and nothing else, which nothing tells from a model. Change it with
    /// `AppState::set_draw_skeleton_model`, which keeps the preference too.
    pub draw_skeleton_model: bool,
    /// ⚠ Held here and not in the selection: the Outline, the Inspector and the status bar follow
    /// the active document, a clip dictionary in the Animation workspace, and the selection holds
    /// one focus, so selecting in the document a click landed in would take the clip off them.
    pub picked: Option<Picked>,
}

impl Default for PreviewScene {
    fn default() -> Self {
        Self {
            clip: None,
            skeleton_source: None,
            mesh_sources: Vec::new(),
            texture_sources: Vec::new(),
            allow_mismatched_skeleton: false,
            skeleton_pinned: false,
            show_bones: true,
            show_rest_bones: false,
            show_bind_pose_ghost: false,
            apply_root_motion: false,
            debug_channel: None,
            lod: 0,
            show_collision: false,
            assemble_parts: true,
            draw_skeleton_model: true,
            picked: None,
        }
    }
}

impl PreviewScene {
    /// The scene a run opens on: what the preferences kept of it.
    pub fn restored(prefs: &super::prefs::Prefs) -> Self {
        Self {
            draw_skeleton_model: prefs.draw_skeleton_model,
            ..Self::default()
        }
    }

    /// Every document the scene names, which `forget_document` lets go of once it closes.
    pub fn documents(&self) -> impl Iterator<Item = DocumentId> + '_ {
        (self.clip.map(|clip| clip.document).into_iter())
            .chain(self.skeleton_source.map(SkeletonSource::document))
            .chain(self.mesh_sources.iter().map(|source| source.document))
            .chain(self.texture_sources.iter().copied())
            .chain(self.picked.as_ref().map(|picked| picked.document))
    }

    /// Let go of `document` wherever the scene names it, and say whether the clip, the skeleton source or the meshes changed by it; a texture source is no input of the clip session, so its going is not a change.
    pub fn forget_document(&mut self, document: DocumentId) -> bool {
        let before = (self.clip, self.skeleton_source, self.mesh_sources.len());
        if self.clip.is_some_and(|c| c.document == document) {
            self.clip = None;
        }
        if self
            .skeleton_source
            .is_some_and(|s| s.document() == document)
        {
            self.skeleton_source = None;
        }
        self.mesh_sources.retain(|m| m.document != document);
        self.texture_sources.retain(|t| *t != document);
        if (self.picked.as_ref()).is_some_and(|picked| picked.document == document) {
            self.picked = None;
        }
        before != (self.clip, self.skeleton_source, self.mesh_sources.len())
    }

    pub fn set_clip(&mut self, document: DocumentId, clip_key: u32) {
        self.clip = Some(ClipSelection { document, clip_key });
    }

    /// Choose a skeleton, on the user's behalf, from candidates ranked
    /// against the clip's own bone tags.
    ///
    /// Returns whether anything changed, so the caller can invalidate only
    /// when it must.
    pub fn adopt_suggested_skeleton(&mut self, suggestion: Option<SkeletonSuggestion>) -> bool {
        if self.skeleton_pinned || self.skeleton_source.is_some() {
            return false;
        }
        match suggestion {
            Some(s) => {
                self.skeleton_source = Some(s.source);
                true
            }
            None => false,
        }
    }

    /// Record that the choice is the user's now.
    pub fn pin_skeleton(&mut self, source: Option<SkeletonSource>) {
        self.skeleton_source = source;
        self.skeleton_pinned = true;
    }

    /// Hand the choice back to the automatic one.
    pub fn unpin_skeleton(&mut self) {
        self.skeleton_pinned = false;
        self.skeleton_source = None;
    }

    /// The documents, and the dictionary entry where one is chosen, whose drawables the preview draws.
    ///
    /// ⚠ The skeleton source's own drawable counts only while no mesh is chosen and
    /// `draw_skeleton_model` is ticked; the preview also draws it when the chosen meshes draw
    /// nothing, which a scene cannot tell.
    pub fn drawn_sources(&self) -> Vec<(DocumentId, Option<usize>)> {
        let skeleton = (self
            .skeleton_source
            .filter(|_| self.mesh_sources.is_empty() && self.draw_skeleton_model))
        .map(|source| match source {
            SkeletonSource::DictionaryEntry(doc, index) => (doc, Some(index)),
            other => (other.document(), None),
        });
        (self.mesh_sources.iter())
            .map(|source| (source.document, source.entry))
            .chain(skeleton)
            .collect()
    }

    /// The documents `drawn_sources` names, each once.
    pub fn drawn_documents(&self) -> Vec<DocumentId> {
        let mut documents = Vec::new();
        for (document, _) in self.drawn_sources() {
            if !documents.contains(&document) {
                documents.push(document);
            }
        }
        documents
    }

    pub fn add_mesh(&mut self, source: MeshSource) {
        if !self.mesh_sources.contains(&source) {
            self.mesh_sources.push(source);
        }
    }

    pub fn remove_mesh(&mut self, source: MeshSource) {
        self.mesh_sources.retain(|m| *m != source);
    }

    /// Forgets the channel once Channel mode is off: Solid and Material multiply by the colour it is packed into.
    pub fn follow_shading_mode(&mut self, mode: or_renderer::ShadingMode) {
        if mode != or_renderer::ShadingMode::Channel {
            self.debug_channel = None;
        }
    }
}

pub fn skeleton_data_from(payload: &DocPayload, source: SkeletonSource) -> Option<&CrSkeletonData> {
    let row = crate::state::capabilities::for_payload(payload)?;
    (row.skeleton_data?)(payload, source)
}

pub fn skeleton_sources_in(id: DocumentId, payload: &DocPayload) -> Vec<SkeletonSource> {
    crate::state::capabilities::for_payload(payload)
        .and_then(|row| row.skeleton_sources)
        .map(|sources| sources(id, payload))
        .unwrap_or_default()
}

pub fn mesh_sources_in(id: DocumentId, payload: &DocPayload, name: &str) -> Vec<MeshCandidate> {
    crate::state::capabilities::for_payload(payload)
        .and_then(|row| row.mesh_sources)
        .map(|sources| sources(id, payload, name))
        .unwrap_or_default()
}

/// How many entries a drawable dictionary of either target holds.
fn ydd_entries(payload: &DocPayload) -> Option<usize> {
    match payload {
        DocPayload::Resource(or_io::Asset::DrawableDict(dict)) => Some(dict.entries.len()),
        DocPayload::Resource(or_io::Asset::DrawableDictGen9(dict)) => Some(dict.entries.len()),
        _ => None,
    }
}

pub(crate) mod mesh_sources {
    use super::*;

    pub(crate) fn whole_document(
        id: DocumentId,
        _payload: &DocPayload,
        name: &str,
    ) -> Vec<MeshCandidate> {
        vec![MeshCandidate {
            source: MeshSource {
                document: id,
                entry: None,
            },
            label: name.to_string(),
        }]
    }

    pub(crate) fn ydd(id: DocumentId, payload: &DocPayload, name: &str) -> Vec<MeshCandidate> {
        let Some(entries) = ydd_entries(payload) else {
            return Vec::new();
        };
        let mut out = vec![MeshCandidate {
            source: MeshSource {
                document: id,
                entry: None,
            },
            label: format!("{name} (all {entries} entries)"),
        }];
        out.extend((0..entries).map(|index| MeshCandidate {
            source: MeshSource {
                document: id,
                entry: Some(index),
            },
            label: format!("{name} [{index}]"),
        }));
        out
    }
}

pub(crate) mod skeletons {
    use super::*;

    pub(crate) fn yft_sources(id: DocumentId, payload: &DocPayload) -> Vec<SkeletonSource> {
        match yft_data(payload, SkeletonSource::Fragment(id)).filter(|s| !s.bones.is_empty()) {
            Some(_) => vec![SkeletonSource::Fragment(id)],
            None => Vec::new(),
        }
    }

    pub(crate) fn yft_data(
        payload: &DocPayload,
        source: SkeletonSource,
    ) -> Option<&CrSkeletonData> {
        let SkeletonSource::Fragment(_) = source else {
            return None;
        };
        match payload {
            DocPayload::Resource(or_io::Asset::Fragment(frag)) => {
                frag.common_drawable.as_deref()?.skeleton.as_deref()
            }
            DocPayload::Resource(or_io::Asset::FragmentGen9(frag)) => {
                frag.common_drawable.as_deref()?.skeleton.as_deref()
            }
            _ => None,
        }
    }

    pub(crate) fn ydr_sources(id: DocumentId, payload: &DocPayload) -> Vec<SkeletonSource> {
        match ydr_data(payload, SkeletonSource::Drawable(id)).filter(|s| !s.bones.is_empty()) {
            Some(_) => vec![SkeletonSource::Drawable(id)],
            None => Vec::new(),
        }
    }

    pub(crate) fn ydr_data(
        payload: &DocPayload,
        source: SkeletonSource,
    ) -> Option<&CrSkeletonData> {
        let SkeletonSource::Drawable(_) = source else {
            return None;
        };
        match payload {
            DocPayload::Resource(or_io::Asset::Drawable(drawable)) => drawable.skeleton.as_deref(),
            DocPayload::Resource(or_io::Asset::DrawableGen9(drawable)) => {
                drawable.skeleton.as_deref()
            }
            _ => None,
        }
    }

    pub(crate) fn ydd_sources(id: DocumentId, payload: &DocPayload) -> Vec<SkeletonSource> {
        let Some(entries) = ydd_entries(payload) else {
            return Vec::new();
        };
        (0..entries)
            .map(|index| SkeletonSource::DictionaryEntry(id, index))
            .filter(|source| ydd_data(payload, *source).is_some_and(|s| !s.bones.is_empty()))
            .collect()
    }

    pub(crate) fn ydd_data(
        payload: &DocPayload,
        source: SkeletonSource,
    ) -> Option<&CrSkeletonData> {
        let SkeletonSource::DictionaryEntry(_, index) = source else {
            return None;
        };
        match payload {
            DocPayload::Resource(or_io::Asset::DrawableDict(dict)) => {
                dict.entries.get(index)?.as_deref()?.skeleton.as_deref()
            }
            DocPayload::Resource(or_io::Asset::DrawableDictGen9(dict)) => {
                dict.entries.get(index)?.as_deref()?.skeleton.as_deref()
            }
            _ => None,
        }
    }
}

/// One candidate skeleton's standing, with the document it came from put
/// back on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkeletonSuggestion {
    pub source: SkeletonSource,
    pub coverage: f32,
    pub bone_count: usize,
}

/// [`or_anim::rank_skeletons`] over candidates a document named, best first.
pub fn suggest_skeletons(
    clip_tags: &BTreeSet<u16>,
    candidates: &[(SkeletonSource, &CrSkeletonData)],
) -> Vec<SkeletonSuggestion> {
    let data: Vec<&CrSkeletonData> = candidates.iter().map(|(_, data)| *data).collect();
    or_anim::rank_skeletons(clip_tags, &data)
        .into_iter()
        .map(|ranked| SkeletonSuggestion {
            source: candidates[ranked.index].0,
            coverage: ranked.coverage,
            bone_count: ranked.bone_count,
        })
        .collect()
}

pub fn suggest_skeleton(
    clip_tags: &BTreeSet<u16>,
    candidates: &[(SkeletonSource, &CrSkeletonData)],
) -> Option<SkeletonSuggestion> {
    suggest_skeletons(clip_tags, candidates).into_iter().next()
}

#[derive(Debug, Default)]
pub struct AnimSession {
    pub player: Option<ClipPlayer>,
    pub binding: Option<BoundClip>,
    pub pose: Pose,
    pub world_matrices: Vec<glam::Mat4>,
    pub skin_matrices: Vec<glam::Mat4>,
    /// The clip, the skeleton source and the contents of the skeleton source's document (its `payload_epoch`) the session was built from.
    built_for: Option<(Option<ClipSelection>, Option<SkeletonSource>, Option<u64>)>,
    /// Set by `invalidate`, cleared by a build: the session is built again whatever it was built for.
    stale: bool,
    /// The clip and skeleton source whose own length, range and Loop the transport was last set to; a rebuild for them keeps what the user changed since.
    transport_for: Option<(Option<ClipSelection>, Option<SkeletonSource>)>,
    /// The clip's own Looped flag when the session was last installed, which the transport took then.
    ///
    /// ⚠ Kept by a session that is cleared, as `transport_for` is: a clip skipped as a mismatch has no
    /// player to compare, and its flag may be edited while it is.
    transport_looped: bool,
    evaluated_at: Option<f32>,
    edit_revision: usize,
    /// How often `skin_matrices` were computed again, which a view of them compares to know they changed.
    skin_revision: u64,
}

impl AnimSession {
    pub fn skin_revision(&self) -> u64 {
        self.skin_revision
    }

    /// Whether the clip, the skeleton source or what the skeleton source holds (`skeleton_epoch`) changed since the session was built, or it was invalidated.
    pub fn needs_rebuild(&self, scene: &PreviewScene, skeleton_epoch: Option<u64>) -> bool {
        self.stale || self.built_for != Some((scene.clip, scene.skeleton_source, skeleton_epoch))
    }

    /// The skeleton source and the contents of its document that the bones the session poses were read from.
    pub fn skeleton_built_from(&self) -> Option<(Option<SkeletonSource>, Option<u64>)> {
        self.built_for.map(|(_, source, epoch)| (source, epoch))
    }

    /// Whether the transport's working range and Loop are the user's for `scene`'s clip and skeleton source, whatever else was rebuilt since.
    pub fn holds_the_transport_for(&self, scene: &PreviewScene) -> bool {
        self.transport_for == Some((scene.clip, scene.skeleton_source))
    }

    /// Whether the clip's own Looped flag is not what the transport last took from it.
    pub fn looped_changed(&self, player: &ClipPlayer) -> bool {
        self.transport_looped != player.is_looped()
    }

    pub fn install(
        &mut self,
        scene: &PreviewScene,
        skeleton_epoch: Option<u64>,
        player: ClipPlayer,
        binding: BoundClip,
    ) {
        self.pose = Pose::bind(&binding.skeleton);
        self.transport_looped = player.is_looped();
        self.player = Some(player);
        self.binding = Some(binding);
        self.built_for = Some((scene.clip, scene.skeleton_source, skeleton_epoch));
        self.transport_for = Some((scene.clip, scene.skeleton_source));
        self.stale = false;
        self.evaluated_at = None;
        self.edit_revision = 0;
    }

    /// ⚠ Clearing for another choice lets go of the transport's: it still holds the old clip's range and Loop, which the new one must not inherit.
    pub fn clear(&mut self, scene: &PreviewScene, skeleton_epoch: Option<u64>) {
        self.player = None;
        self.binding = None;
        self.pose = Pose::default();
        self.world_matrices.clear();
        self.skin_matrices.clear();
        self.built_for = Some((scene.clip, scene.skeleton_source, skeleton_epoch));
        self.transport_for = self
            .transport_for
            .filter(|choice| *choice == (scene.clip, scene.skeleton_source));
        self.stale = false;
        self.evaluated_at = None;
        self.edit_revision = 0;
    }

    /// Builds the session again on the next tick, for the same clip and skeleton source unless the scene names others.
    pub fn invalidate(&mut self) {
        self.stale = true;
        self.evaluated_at = None;
        self.edit_revision = 0;
    }

    pub fn evaluate(
        &mut self,
        time: f32,
        apply_root_motion: bool,
        edits: &or_anim::PoseEdits,
    ) -> bool {
        let Some(player) = self.player.as_mut() else {
            return false;
        };
        let Some(binding) = self.binding.as_ref() else {
            return false;
        };
        let edit_revision = edits.len();
        if self.evaluated_at == Some(time) && self.edit_revision == edit_revision {
            return false;
        }
        player.evaluate_into(&binding.skeleton, time, &mut self.pose);

        if !edits.is_empty() {
            if let Some(frame_rate) = binding.frame_rate {
                let frame = (time * frame_rate).round().max(0.0);
                if frame <= f32::from(u16::MAX) {
                    edits.apply_to_pose(frame as u16, &mut self.pose);
                }
            }
        }

        if !apply_root_motion {
            self.pose.root_motion = or_anim::BoneTransform::IDENTITY;
        }
        self.pose.pose_into(
            &binding.skeleton,
            &mut self.world_matrices,
            &mut self.skin_matrices,
        );
        self.evaluated_at = Some(time);
        self.edit_revision = edit_revision;
        self.skin_revision += 1;
        true
    }

    pub fn frame_at(&self, time: f32) -> Option<u16> {
        let frame_rate = self.binding.as_ref()?.frame_rate?;
        let frame = (time * frame_rate).round();
        if frame < 0.0 || frame > f32::from(u16::MAX) {
            None
        } else {
            Some(frame as u16)
        }
    }

    pub fn skeleton(&self) -> Option<&Skeleton> {
        self.binding.as_ref().map(|b| &b.skeleton)
    }

    pub fn last_errors(&self) -> &[String] {
        self.player.as_ref().map(|p| p.last_errors()).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::codec::anim::{format_type, track};
    use or_io::decl::{Keyed, Open};
    use or_io::gta5::{
        AtMap, AtMapEntry, AtString, CrAnimDictionary, CrAnimation, CrClip, CrClipAnimation,
        CrClipAny, CrClipDictionary, Dof, EClipType,
    };
    use or_io::gta5::{CrBoneData, Vec4V};

    fn no_edits() -> or_anim::PoseEdits {
        or_anim::PoseEdits::new()
    }

    fn a_suggestion(source: SkeletonSource) -> SkeletonSuggestion {
        SkeletonSuggestion {
            source,
            coverage: 0.9,
            bone_count: 12,
        }
    }

    #[test]
    fn choosing_no_skeleton_is_a_choice_the_resolver_respects() {
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);

        assert!(scene.adopt_suggested_skeleton(Some(a_suggestion(SkeletonSource::Drawable(7)))));
        assert_eq!(scene.skeleton_source, Some(SkeletonSource::Drawable(7)));

        scene.pin_skeleton(None);
        assert!(scene.skeleton_pinned);
        for _ in 0..3 {
            assert!(
                !scene.adopt_suggested_skeleton(Some(a_suggestion(SkeletonSource::Drawable(7))))
            );
            assert_eq!(scene.skeleton_source, None);
        }

        scene.unpin_skeleton();
        assert!(scene.adopt_suggested_skeleton(Some(a_suggestion(SkeletonSource::Drawable(7)))));
        assert_eq!(scene.skeleton_source, Some(SkeletonSource::Drawable(7)));
    }

    /// Dropping an automatic choice when the clip changes was tried and
    /// reverted: a poor match is already reported by the binding report.
    #[test]
    fn a_skeleton_already_chosen_survives_a_clip_change() {
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1111);
        assert!(scene.adopt_suggested_skeleton(Some(a_suggestion(SkeletonSource::Fragment(3)))));

        scene.set_clip(1, 0x2222);
        assert_eq!(scene.skeleton_source, Some(SkeletonSource::Fragment(3)));

        // And it is not re-adopted over itself, so nothing invalidates.
        assert!(!scene.adopt_suggested_skeleton(Some(a_suggestion(SkeletonSource::Drawable(9)))));
        assert_eq!(scene.skeleton_source, Some(SkeletonSource::Fragment(3)));
    }

    #[test]
    fn no_candidate_means_no_choice_rather_than_a_bad_one() {
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);
        assert!(!scene.adopt_suggested_skeleton(None));
        assert_eq!(scene.skeleton_source, None);
        assert!(!scene.skeleton_pinned);
    }

    fn v4(x: f32, y: f32, z: f32, w: f32) -> Vec4V {
        Vec4V {
            x,
            y,
            z,
            w,
            ..Vec4V::default()
        }
    }

    fn bone(index: u16, tag: u16, parent: i16) -> CrBoneData {
        CrBoneData {
            rotation: v4(0.0, 0.0, 0.0, 1.0),
            translation: Vec4V::default(),
            scale: v4(1.0, 1.0, 1.0, 0.0),
            next_index: -1,
            parent_index: parent,
            name: format!("BONE_{index}"),
            dofs: 0,
            index,
            bone_id: tag,
            mirror_index: 0,
            ..CrBoneData::default()
        }
    }

    fn skeleton_with(tags: &[u16]) -> CrSkeletonData {
        let bones: Vec<CrBoneData> = tags
            .iter()
            .enumerate()
            .map(|(i, tag)| bone(i as u16, *tag, if i == 0 { -1 } else { i as i16 - 1 }))
            .collect();
        let parent_indices = (0..tags.len())
            .map(|i| if i == 0 { -1 } else { i as i16 - 1 })
            .collect();
        CrSkeletonData {
            parent_indices,
            bones,
            ..CrSkeletonData::default()
        }
    }

    fn one_entry<T: or_io::decl::Owned>(key: u32, data: T) -> AtMap<T>
    where
        or_io::decl::Raw<T>: or_io::decl::Record,
    {
        AtMap {
            hash: vec![Some(Box::new(AtMapEntry {
                key,
                data: Some(Box::new(data)),
                ..AtMapEntry::default()
            }))],
            ..AtMap::default()
        }
    }

    fn dictionary_driving(tags: &[u16]) -> CrClipDictionary {
        const IDLE: u32 = 0x1000;
        let dofs = tags
            .iter()
            .map(|tag| Dof {
                id: *tag,
                r#type: format_type::QUATERNION,
                track: track::BONE_ROTATION,
                ..Dof::default()
            })
            .collect();
        let animation = CrAnimation {
            num_frames: 2,
            frames_per_chunk: 127,
            duration: 1.0,
            dofs,
            ..CrAnimation::default()
        };
        let clip = CrClipAnimation {
            base: CrClip {
                r#type: Open::new(EClipType::Animation),
                name: AtString::new("idle"),
                ..CrClip::default()
            },
            animation: Keyed::to(IDLE),
            end_time: 1.0,
            rate: 1.0,
            ..CrClipAnimation::default()
        };
        CrClipDictionary {
            anim_dictionary: Some(Box::new(CrAnimDictionary {
                animations: one_entry(IDLE, animation),
                ..CrAnimDictionary::default()
            })),
            clips: one_entry(0x1234, CrClipAny::CrClipAnimation(clip)),
            ..CrClipDictionary::default()
        }
    }

    #[test]
    fn a_ydd_offers_the_whole_dictionary_and_each_entry() {
        let candidates = mesh_sources_in(
            1,
            &DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
            "clothes",
        );
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].source.entry, None);
        assert!(
            candidates[0].label.contains("all"),
            "a dictionary's whole-document candidate has to say it is all of them: {}",
            candidates[0].label
        );
    }

    #[test]
    fn a_ydr_offers_exactly_one_mesh_source() {
        let candidates = mesh_sources_in(
            2,
            &DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
            "prop",
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source.document, 2);
        assert_eq!(candidates[0].source.entry, None);
        assert_eq!(candidates[0].label, "prop");
    }

    #[test]
    fn formats_without_geometry_offer_nothing() {
        assert!(mesh_sources_in(
            1,
            &DocPayload::Resource(or_io::Asset::Clips(Default::default())),
            "a"
        )
        .is_empty());
        assert!(mesh_sources_in(
            1,
            &DocPayload::Resource(or_io::Asset::Bounds(
                or_io::gta5::PhBoundAny::PhBoundSphere(Default::default())
            )),
            "b"
        )
        .is_empty());
    }

    #[test]
    fn selecting_a_clip_keeps_the_skeleton_and_meshes() {
        let mut scene = PreviewScene {
            skeleton_source: Some(SkeletonSource::Fragment(2)),
            mesh_sources: vec![MeshSource {
                document: 3,
                entry: Some(0),
            }],
            ..PreviewScene::default()
        };
        scene.set_clip(1, 0xAAAA);
        scene.set_clip(1, 0xBBBB);
        assert_eq!(scene.clip.unwrap().clip_key, 0xBBBB);
        assert_eq!(scene.skeleton_source, Some(SkeletonSource::Fragment(2)));
        assert_eq!(scene.mesh_sources.len(), 1);
    }

    #[test]
    fn closing_a_document_clears_every_reference_to_it() {
        let mut scene = PreviewScene {
            clip: Some(ClipSelection {
                document: 1,
                clip_key: 0x1234,
            }),
            skeleton_source: Some(SkeletonSource::Fragment(2)),
            mesh_sources: vec![
                MeshSource {
                    document: 2,
                    entry: None,
                },
                MeshSource {
                    document: 3,
                    entry: Some(1),
                },
            ],
            texture_sources: vec![2, 4],
            ..PreviewScene::default()
        };

        assert!(scene.forget_document(2));
        assert!(scene.skeleton_source.is_none());
        assert_eq!(scene.mesh_sources.len(), 1);
        assert_eq!(scene.texture_sources, vec![4]);
        assert!(scene.clip.is_some());

        assert!(!scene.forget_document(99));
    }

    #[test]
    fn meshes_deduplicate_and_remove() {
        let mut scene = PreviewScene::default();
        let source = MeshSource {
            document: 1,
            entry: Some(0),
        };
        scene.add_mesh(source);
        scene.add_mesh(source);
        assert_eq!(scene.mesh_sources.len(), 1);
        scene.remove_mesh(source);
        assert!(scene.mesh_sources.is_empty());
    }

    /// The mapping this wrapper exists for: a ranking is an index into the
    /// slice that went in, and the source has to come back off the same row.
    #[test]
    fn suggestions_rank_by_coverage_and_skip_unrelated_skeletons() {
        let tags: BTreeSet<u16> = [100, 200, 300, 400].into_iter().collect();
        let full = skeleton_with(&[100, 200, 300, 400]);
        let half = skeleton_with(&[100, 200]);
        let none = skeleton_with(&[8888]);
        let candidates = vec![
            (SkeletonSource::Drawable(1), &half),
            (SkeletonSource::Fragment(2), &full),
            (SkeletonSource::Drawable(3), &none),
        ];

        let ranked = suggest_skeletons(&tags, &candidates);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].source, SkeletonSource::Fragment(2));
        assert!((ranked[0].coverage - 1.0).abs() < 1e-6);
        assert_eq!(
            suggest_skeleton(&tags, &candidates).map(|s| s.source),
            Some(SkeletonSource::Fragment(2))
        );
    }

    #[test]
    fn equal_coverage_prefers_the_smaller_skeleton() {
        let tags: BTreeSet<u16> = [100, 200].into_iter().collect();
        let exact = skeleton_with(&[100, 200]);
        let superset = skeleton_with(&[100, 200, 300, 400, 500]);
        let candidates = vec![
            (SkeletonSource::Fragment(1), &superset),
            (SkeletonSource::Fragment(2), &exact),
        ];
        let ranked = suggest_skeletons(&tags, &candidates);
        assert_eq!(ranked[0].source, SkeletonSource::Fragment(2));
        assert_eq!(ranked[0].bone_count, 2);
    }

    #[test]
    fn no_candidates_means_no_suggestion() {
        let tags: BTreeSet<u16> = [100].into_iter().collect();
        assert!(suggest_skeleton(&tags, &[]).is_none());
    }

    #[test]
    fn a_session_rebuilds_only_when_the_scene_or_the_skeleton_sources_contents_change() {
        let dict = dictionary_driving(&[100, 200]);
        let skeleton = skeleton_with(&[100, 200]);
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);
        scene.skeleton_source = Some(SkeletonSource::Fragment(2));

        let mut session = AnimSession::default();
        assert!(session.needs_rebuild(&scene, Some(7)));

        let (player, binding) = or_anim::bind(&dict, 0x1234, &skeleton).expect("bound");
        session.install(&scene, Some(7), player, binding);
        assert!(!session.needs_rebuild(&scene, Some(7)));
        assert!(session.needs_rebuild(&scene, Some(8)));

        scene.set_clip(1, 0x5678);
        assert!(session.needs_rebuild(&scene, Some(7)));
    }

    /// ⚠ A session cleared for a skeleton that did not fit never set the transport from the clip, so
    /// the bones it is edited into must; one cleared for a choice the transport was set from still holds it.
    #[test]
    fn the_transport_belongs_to_the_choice_it_was_last_set_from() {
        let dict = dictionary_driving(&[100, 200]);
        let skeleton = skeleton_with(&[100, 200]);
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);
        scene.skeleton_source = Some(SkeletonSource::Fragment(2));
        let bind = || or_anim::bind(&dict, 0x1234, &skeleton).expect("bound");

        let mut session = AnimSession::default();
        assert!(!session.holds_the_transport_for(&scene));
        session.clear(&scene, Some(7));
        assert!(!session.holds_the_transport_for(&scene));

        let (player, binding) = bind();
        session.install(&scene, Some(7), player, binding);
        assert!(session.holds_the_transport_for(&scene));
        assert_eq!(
            session.skeleton_built_from(),
            Some((scene.skeleton_source, Some(7)))
        );

        // Invalidating, and clearing for the same choice (the bones no longer fit), is no new choice.
        session.invalidate();
        assert!(session.holds_the_transport_for(&scene));
        session.clear(&scene, Some(8));
        assert!(session.holds_the_transport_for(&scene));

        scene.skeleton_source = Some(SkeletonSource::Fragment(3));
        assert!(!session.holds_the_transport_for(&scene));
        scene.skeleton_source = Some(SkeletonSource::Fragment(2));
        scene.set_clip(1, 0x5678);
        assert!(!session.holds_the_transport_for(&scene));

        // Clearing for another choice lets go of it for good: the first one is a new choice when it comes back.
        session.clear(&scene, Some(8));
        scene.set_clip(1, 0x1234);
        assert!(!session.holds_the_transport_for(&scene));
        let (player, binding) = bind();
        session.install(&scene, Some(8), player, binding);
        assert!(session.holds_the_transport_for(&scene));
    }

    #[test]
    fn invalidating_a_session_builds_it_again_and_moves_nothing_it_was_built_from() {
        let dict = dictionary_driving(&[100, 200]);
        let skeleton = skeleton_with(&[100, 200]);
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);
        scene.skeleton_source = Some(SkeletonSource::Fragment(2));
        let mut session = AnimSession::default();
        let (player, binding) = or_anim::bind(&dict, 0x1234, &skeleton).expect("bound");
        session.install(&scene, Some(7), player, binding);
        assert!(!session.needs_rebuild(&scene, Some(7)));

        let built_from = session.skeleton_built_from();
        session.invalidate();
        assert!(session.needs_rebuild(&scene, Some(7)));
        assert_eq!(session.skeleton_built_from(), built_from);
    }

    #[test]
    fn evaluating_twice_at_the_same_time_does_no_work() {
        let dict = dictionary_driving(&[100, 200]);
        let skeleton = skeleton_with(&[100, 200]);
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);
        scene.skeleton_source = Some(SkeletonSource::Fragment(2));
        let mut session = AnimSession::default();
        let (player, binding) = or_anim::bind(&dict, 0x1234, &skeleton).expect("bound");
        session.install(&scene, None, player, binding);

        assert!(session.evaluate(0.25, false, &no_edits()));
        assert!(!session.evaluate(0.25, false, &no_edits()));
        assert!(session.evaluate(0.5, false, &no_edits()));

        assert_eq!(session.world_matrices.len(), 2);
        assert_eq!(session.skin_matrices.len(), 2);
    }

    #[test]
    fn an_empty_session_evaluates_to_nothing() {
        let mut session = AnimSession::default();
        assert!(!session.evaluate(0.0, false, &no_edits()));
        assert!(session.skeleton().is_none());
        assert!(session.last_errors().is_empty());
    }

    #[test]
    fn clearing_a_session_keeps_the_scene_marker() {
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);
        let mut session = AnimSession::default();
        session.clear(&scene, None);
        assert!(!session.needs_rebuild(&scene, None));
        session.invalidate();
        assert!(session.needs_rebuild(&scene, None));
    }

    #[test]
    fn root_motion_is_suppressed_when_the_scene_asks() {
        let dict = dictionary_driving(&[100]);
        let skeleton = skeleton_with(&[100]);
        let mut scene = PreviewScene::default();
        scene.set_clip(1, 0x1234);
        let mut session = AnimSession::default();
        let (player, binding) = or_anim::bind(&dict, 0x1234, &skeleton).expect("bound");
        session.install(&scene, None, player, binding);

        session.evaluate(0.5, false, &no_edits());
        assert_eq!(session.pose.root_motion, or_anim::BoneTransform::IDENTITY);
    }

    #[test]
    fn skeleton_source_reports_its_document_and_kind() {
        assert_eq!(SkeletonSource::Fragment(7).document(), 7);
        assert_eq!(SkeletonSource::DictionaryEntry(9, 3).document(), 9);
        assert_eq!(SkeletonSource::Fragment(1).kind_label(), "fragment");
        assert_eq!(SkeletonSource::Drawable(1).kind_label(), "drawable");
        assert_eq!(
            SkeletonSource::DictionaryEntry(1, 0).kind_label(),
            "dictionary entry"
        );
    }
}
