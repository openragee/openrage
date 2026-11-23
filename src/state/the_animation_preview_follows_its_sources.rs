//! The Animation preview follows the sources it draws from and not the document in front of it: a
//! clip chosen while the preview is paused is posed, a mesh source's textures and level-of-detail
//! switch are its own, a source that closes leaves the list, and a Looped edit made while a clip is
//! skipped is not lost.

use std::path::PathBuf;

use or_io::codec::texture::GrcTextureFormat;
use or_io::decl::{Gen8, Gen9, Keyed};
use or_io::gta5::{
    AtMapEntry, CrClipAny, CrClipDictionary, DrawableTarget, GrmModel, GtaDrawable, PgDictionary,
    RmcLod,
};
use or_renderer::MeshVertex;
use or_transport::TimeRange;

use super::prefs::Prefs;
use super::texture_resolution::{probe, resolve, ResolutionRule};
use super::the_animation_preview_draws_what_its_sources_hold::{
    a_clip_over_the_rig, rigged, ANIMATION, CLIP, OTHER_CLIP,
};
use super::{
    AppState, Change, ClipFieldSnapshot, DocPayload, DocumentId, MeshSource, RpfEntryBinding,
    SkeletonSource,
};
use crate::render::meshes::MeshUpload;
use crate::render::viewport::pick::levels;
use crate::render::viewport::pose_uploads;
use crate::ui::dock::Workspace;
use crate::ui::panels::viewport_texture_badge::is_pinned_in_view;

const OTHER_ANIMATION: u32 = 0x1001;

fn open(state: &mut AppState, path: &str, payload: DocPayload) -> DocumentId {
    state.insert_document(PathBuf::from(path), payload, None, None)
}

fn clips(dictionary: CrClipDictionary) -> DocPayload {
    DocPayload::Resource(or_io::Asset::Clips(dictionary))
}

fn a_rig() -> DocPayload {
    DocPayload::Resource(or_io::Asset::Drawable(rigged::<Gen8>()))
}

/// The rig of each target as an asset, for a test whose subject does not depend on the target.
fn the_rigs() -> [or_io::Asset; 2] {
    [
        or_io::Asset::Drawable(rigged::<Gen8>()),
        or_io::Asset::DrawableGen9(rigged::<Gen9>()),
    ]
}

/// A dictionary of the one texture `name`.
fn textures(name: &str) -> DocPayload {
    let mut dictionary = PgDictionary::default();
    dictionary.insert(or_io::gta5::gen8::GrcTexture {
        name: name.to_string(),
        width: 4,
        height: 4,
        format: GrcTextureFormat::A8R8G8B8.to_raw_format(),
        ..Default::default()
    });
    DocPayload::Resource(or_io::Asset::TextureDict(dictionary))
}

/// `a_clip_over_the_rig` with the second clip on an animation of its own, so a key baked into it
/// leaves the first as it was.
fn the_clips_over_two_animations() -> CrClipDictionary {
    let mut dictionary = a_clip_over_the_rig();
    let animation = or_anim::clip::animation(&dictionary, ANIMATION)
        .expect("the animation")
        .clone();
    let animations = &mut (dictionary.anim_dictionary.as_deref_mut())
        .expect("the animations")
        .animations;
    animations.hash.push(Some(Box::new(AtMapEntry {
        key: OTHER_ANIMATION,
        data: Some(Box::new(animation)),
        ..AtMapEntry::default()
    })));
    let Some(CrClipAny::CrClipAnimation(clip)) =
        or_anim::clip::clip_mut(&mut dictionary, OTHER_CLIP)
    else {
        panic!("the second clip plays an animation");
    };
    clip.animation = Keyed::to(OTHER_ANIMATION);
    dictionary
}

/// What Insert key and then Bake do, on the clip chosen.
fn bake_a_key(state: &mut AppState) {
    let before = state.anim_edits.clone();
    let key = or_anim::EditKey {
        bone: 1,
        track: or_anim::EditTrack::Rotation,
        frame: 0,
    };
    let value = or_anim::EditValue::Rotation(glam::Quat::from_rotation_z(0.5));
    assert!(state.anim_edits.set(key, value));
    state.push_anim_edit("insert key", before);
    state.anim.invalidate();
    state.tick_animation();
    let baked = state.bake_anim_edits();
    assert!(matches!(baked, Ok(Some(_))), "{baked:?}");
}

/// A vertex one metre beside the rig's second bone and skinned to it alone, as the viewport holds it.
fn a_skinned_vertex() -> MeshUpload {
    let beside_the_bone = [1.0, 2.0, 0.0];
    MeshUpload {
        bump: false,
        vertices: vec![MeshVertex::new(beside_the_bone, [0.0, 1.0, 0.0], [0.0; 2])],
        indices: vec![0, 0, 0],
        texture_name: None,
        skinning: Some(or_anim::SkinnedMesh {
            influences: vec![or_anim::VertexInfluences::rigid(1)],
            bind_positions: vec![beside_the_bone],
            bind_normals: vec![[0.0, 1.0, 0.0]],
            bones_used: vec![1],
            ..Default::default()
        }),
        palette_bones: Vec::new(),
        channels: Vec::new(),
        source: None,
    }
}

#[test]
fn choosing_another_clip_while_the_preview_is_paused_poses_it() {
    for rig in the_rigs() {
        let mut state = AppState::from_prefs(Prefs::default());
        let rig = open(&mut state, "mods/rig.ydr", DocPayload::Resource(rig));
        let dictionary = clips(the_clips_over_two_animations());
        let dictionary = open(&mut state, "mods/clips.ycd", dictionary);
        state
            .preview_scene
            .pin_skeleton(Some(SkeletonSource::Drawable(rig)));
        state.preview_scene.set_clip(dictionary, OTHER_CLIP);
        state.tick_animation();
        bake_a_key(&mut state);

        state.preview_scene.set_clip(dictionary, CLIP);
        state.tick_animation();
        assert!(!state.anim_transport.playing, "the preview is paused");
        let mut drawn = vec![a_skinned_vertex()];
        let (mut held, mut positions, mut normals) = (None, Vec::new(), Vec::new());
        let mut pose = |state: &AppState, drawn: &mut Vec<MeshUpload>| {
            let moved = pose_uploads(drawn, &mut held, state, &mut positions, &mut normals);
            (moved, drawn[0].vertices[0].position)
        };
        let (moved, first) = pose(&state, &mut drawn);
        assert_eq!(moved, [0], "the first clip is posed");

        state.preview_scene.set_clip(dictionary, OTHER_CLIP);
        state.tick_animation();
        assert_eq!(state.anim_transport.time, 0.0, "the time did not move");
        let (moved, second) = pose(&state, &mut drawn);
        assert_eq!(moved, [0], "the viewport keeps the first clip's pose");
        assert_ne!(
            second, first,
            "the other clip poses the vertex as the first did"
        );

        state.tick_animation();
        let (moved, third) = pose(&state, &mut drawn);
        assert!(
            moved.is_empty(),
            "a frame that poses nothing new skins again"
        );
        assert_eq!(third, second);
    }
}

/// A rig open as the preview's mesh source, a dictionary of "diffuse" open at each of
/// `dictionaries_at`, and a clip dictionary in front.
struct Among {
    state: AppState,
    source: MeshSource,
    clips: DocumentId,
    dictionaries: Vec<DocumentId>,
}

fn among(rig_at: &str, dictionaries_at: &[&str], clips_at: &str) -> Among {
    let mut state = AppState::from_prefs(Prefs::default());
    let rig = open(&mut state, rig_at, a_rig());
    let dictionaries = (dictionaries_at.iter())
        .map(|path| open(&mut state, path, textures("diffuse")))
        .collect();
    let dictionary = clips(a_clip_over_the_rig());
    let dictionary = open(&mut state, clips_at, dictionary);
    let source = MeshSource {
        document: rig,
        entry: None,
    };
    state.preview_scene.add_mesh(source);
    assert_eq!(state.active_workspace, Workspace::Animation);
    state.set_active_document(dictionary);
    Among {
        state,
        source,
        clips: dictionary,
        dictionaries,
    }
}

/// Which dictionary answers for the mesh source's "diffuse" and by which rule: as the viewport
/// binds it, and as the badge says.
fn answer(p: &Among) -> (DocumentId, ResolutionRule) {
    let bound = resolve(&p.state, Some(p.source), None, "diffuse").expect("a dictionary binds");
    let said = probe(&p.state, Some(p.source), None, "diffuse");
    let said = said.found.expect("the badge names a dictionary");
    assert_eq!(
        (bound.origin.document, bound.origin.rule),
        (said.document, said.rule),
        "the badge and the viewport disagree"
    );
    (said.document, said.rule)
}

#[test]
fn a_mesh_sources_texture_is_ranked_by_its_own_folder_while_a_clip_dictionary_is_active() {
    let p = among(
        "rigs/rig.ydr",
        &["clips/tex.ytd", "rigs/tex.ytd"],
        "clips/clips.ycd",
    );
    let [beside_the_clips, beside_the_rig] = p.dictionaries[..] else {
        panic!("two dictionaries");
    };
    assert_eq!(p.state.active_document, Some(p.clips), "the focus");

    assert_eq!(answer(&p), (beside_the_rig, ResolutionRule::SameFolder));
    let others = probe(&p.state, Some(p.source), None, "diffuse").also_in;
    assert_eq!(
        others.iter().map(|o| o.document).collect::<Vec<_>>(),
        [beside_the_clips],
        "the dictionary beside the clips is the other one that holds it"
    );
}

#[test]
fn a_mesh_sources_texture_is_ranked_by_its_own_archive_while_a_clip_dictionary_is_active() {
    let mut p = among(
        "rigs/rig.ydr",
        &["packs/tex.ytd", "other/tex.ytd"],
        "clips/clips.ycd",
    );
    let [in_the_rigs_archive, in_the_clips_archive] = p.dictionaries[..] else {
        panic!("two dictionaries");
    };
    let rig = p.source.document;
    for (document, archive) in [
        (rig, 91),
        (in_the_rigs_archive, 91),
        (p.clips, 92),
        (in_the_clips_archive, 92),
    ] {
        p.state
            .document_mut(document)
            .expect("open")
            .rpf_entry_binding = Some(RpfEntryBinding {
            parent_doc_id: archive,
            entry_id: 1,
        });
    }

    assert_eq!(
        answer(&p),
        (in_the_rigs_archive, ResolutionRule::SameArchive)
    );
}

#[test]
fn the_badges_pins_are_the_mesh_sources_and_not_the_clip_dictionarys() {
    let mut p = among(
        "rigs/rig.ydr",
        &[
            "rigs/tex.ytd",
            "elsewhere/pinned.ytd",
            "elsewhere/attached.ytd",
        ],
        "clips/clips.ycd",
    );
    let [beside, pinned, attached] = p.dictionaries[..] else {
        panic!("three dictionaries");
    };
    let rig = p.source.document;
    assert_eq!(answer(&p), (beside, ResolutionRule::SameFolder));

    p.state.pin_texture_dictionary_in_view(pinned);
    assert_eq!(p.state.pinned_texture_dictionaries(rig), [pinned]);
    assert!(
        p.state.pinned_texture_dictionaries(p.clips).is_empty(),
        "the pin went to the clip dictionary"
    );
    assert_eq!(p.state.texture_pins_in_view(), [pinned]);
    assert_eq!(answer(&p), (pinned, ResolutionRule::Pinned));

    p.state.unpin_texture_dictionary_in_view(pinned);
    assert!(p.state.texture_pins_in_view().is_empty());
    assert_eq!(answer(&p), (beside, ResolutionRule::SameFolder));

    p.state.pin_texture_dictionary_in_view(pinned);
    p.state.clear_texture_pins_in_view();
    assert!(p.state.pinned_texture_dictionaries(rig).is_empty());
    assert_eq!(answer(&p), (beside, ResolutionRule::SameFolder));

    // An attachment is the session's, so it counts for the mesh source with no document of its own to name.
    p.state
        .attach_texture_dictionary(attached)
        .expect("a .ytd attaches");
    assert_eq!(answer(&p), (attached, ResolutionRule::Attached));
}

/// A rig of target `T` with models at each of `levels`, drawn to the distance beside it.
fn drawn_at<T: DrawableTarget>(levels: &[(usize, f32)]) -> GtaDrawable<T> {
    let mut drawable = rigged::<T>();
    for (level, distance) in levels {
        drawable.base.lod_group.lods[*level] = RmcLod {
            models: vec![Box::new(GrmModel::default())],
            ..Default::default()
        }
        .into();
        drawable.base.lod_group.lod_distances[*level] = *distance;
    }
    drawable
}

#[test]
fn the_level_of_detail_switch_lists_the_mesh_sources_levels_while_a_clip_dictionary_is_active() {
    type Make = fn(&[(usize, f32)]) -> or_io::Asset;
    let targets: [Make; 2] = [
        |levels| or_io::Asset::Drawable(drawn_at::<Gen8>(levels)),
        |levels| or_io::Asset::DrawableGen9(drawn_at::<Gen9>(levels)),
    ];
    for make in targets {
        let mut state = AppState::from_prefs(Prefs::default());
        // A garment with its one level, then a body with three of four.
        let garment = make(&[(0, 15.0)]);
        let garment = open(
            &mut state,
            "mods/garment.ydr",
            DocPayload::Resource(garment),
        );
        let body = make(&[(0, 15.0), (1, 40.0), (3, 300.0)]);
        let body = open(&mut state, "mods/body.ydr", DocPayload::Resource(body));
        let dictionary = clips(a_clip_over_the_rig());
        let dictionary = open(&mut state, "mods/clips.ycd", dictionary);
        assert_eq!(state.active_workspace, Workspace::Animation);
        state.set_active_document(dictionary);
        assert!(levels(&state).is_empty(), "nothing is previewed yet");

        for document in [garment, body] {
            state.preview_scene.add_mesh(MeshSource {
                document,
                entry: None,
            });
        }
        assert_eq!(
            levels(&state),
            [(0, 15.0), (1, 40.0), (3, 300.0)],
            "the levels of the drawables the preview draws"
        );
    }
}

#[test]
fn a_mesh_source_that_closes_leaves_the_previews_list() {
    let mut state = AppState::from_prefs(Prefs::default());
    let rig = open(&mut state, "mods/rig.ydr", a_rig());
    let body = open(&mut state, "mods/body.ydr", a_rig());
    let dictionary = clips(a_clip_over_the_rig());
    let dictionary = open(&mut state, "mods/clips.ycd", dictionary);
    state.preview_scene.set_clip(dictionary, CLIP);
    state
        .preview_scene
        .pin_skeleton(Some(SkeletonSource::Drawable(rig)));
    state.preview_scene.add_mesh(MeshSource {
        document: body,
        entry: None,
    });
    state.tick_animation();
    assert_eq!(state.preview_scene.mesh_sources.len(), 1);

    state.close_document(body);
    assert!(state.document(body).is_none(), "the source is closed");
    state.tick_animation();
    assert!(
        state.preview_scene.mesh_sources.is_empty(),
        "the preview lists {:?}, which is closed",
        state.preview_scene.mesh_sources
    );
    assert_eq!(
        state.preview_scene.skeleton_source,
        Some(SkeletonSource::Drawable(rig)),
        "the skeleton source is still there"
    );
}

/// The rig, and a clip over bones it has none of, open as a skipped clip with its mismatch allowed.
fn a_clip_over_other_bones(rig: or_io::Asset) -> (AppState, DocumentId) {
    let mut state = AppState::from_prefs(Prefs::default());
    let rig = open(&mut state, "mods/rig.ydr", DocPayload::Resource(rig));
    let mut dictionary = a_clip_over_the_rig();
    let animation = or_anim::clip::animation_mut(&mut dictionary, ANIMATION).expect("animation");
    for dof in &mut animation.dofs {
        dof.id += 1000;
    }
    let dictionary = open(&mut state, "mods/clips.ycd", clips(dictionary));
    state
        .preview_scene
        .pin_skeleton(Some(SkeletonSource::Drawable(rig)));
    state.preview_scene.set_clip(dictionary, CLIP);
    state.preview_scene.allow_mismatched_skeleton = true;
    state.tick_animation();
    assert!(state.anim.player.is_some(), "allowed, the clip plays");
    (state, dictionary)
}

/// What the Allow mismatched skeleton checkbox does.
fn allow_the_mismatch(state: &mut AppState, allow: bool) {
    state.preview_scene.allow_mismatched_skeleton = allow;
    state.anim.invalidate();
    state.tick_animation();
    assert_eq!(
        state.anim.player.is_some(),
        allow,
        "the clip plays only where the mismatch is allowed"
    );
}

/// What ticking the clip's Looped field does.
fn set_the_looped_field(state: &mut AppState, dictionary: DocumentId, looped: bool) {
    let before = state
        .clip_field_snapshot(dictionary, CLIP)
        .expect("the clip");
    let edited = ClipFieldSnapshot {
        looped,
        ..before.clone()
    };
    assert!(state.apply_clip_fields(dictionary, CLIP, &edited));
    let change = Change::ClipFields {
        clip_key: CLIP,
        other: before,
    };
    state.record(dictionary, "Edit clip fields", change);
    state.anim.invalidate();
    state.tick_animation();
}

#[test]
fn a_looped_edit_made_while_the_clip_is_skipped_wins_when_it_is_allowed_again() {
    for rig in the_rigs() {
        let (mut state, dictionary) = a_clip_over_other_bones(rig);
        let trimmed = TimeRange::new(0.25, 0.75);
        state.anim_transport.set_range(trimmed);
        assert!(!state.anim_transport.is_looping(), "the clip's own Loop");

        allow_the_mismatch(&mut state, false);
        set_the_looped_field(&mut state, dictionary, true);
        assert!(state.anim.player.is_none(), "still skipped");
        allow_the_mismatch(&mut state, true);

        assert!(
            state.anim_transport.is_looping(),
            "the clip's Looped field, ticked while it was skipped, is not Loop"
        );
        assert_eq!(state.anim_transport.range, trimmed, "the working range");
    }
}

#[test]
fn a_skipped_clip_allowed_again_keeps_the_users_loop_where_its_looped_field_did_not_change() {
    for rig in the_rigs() {
        let (mut state, dictionary) = a_clip_over_other_bones(rig);
        let trimmed = TimeRange::new(0.25, 0.75);
        state.anim_transport.set_range(trimmed);
        state.anim_transport.set_looping(true);

        allow_the_mismatch(&mut state, false);
        // Ticked and cleared again while skipped: the field is as it was.
        set_the_looped_field(&mut state, dictionary, true);
        set_the_looped_field(&mut state, dictionary, false);
        allow_the_mismatch(&mut state, true);

        assert!(state.anim_transport.is_looping(), "the user's Loop");
        assert_eq!(state.anim_transport.range, trimmed, "the working range");
    }
}

#[test]
fn the_level_of_detail_switch_lists_the_drawn_sources_levels_with_no_active_document() {
    let mut state = AppState::from_prefs(Prefs::default());
    let body = drawn_at::<Gen8>(&[(0, 15.0), (2, 90.0)]);
    let body = open(
        &mut state,
        "mods/body.ydr",
        DocPayload::Resource(or_io::Asset::Drawable(body)),
    );
    let dictionary = clips(a_clip_over_the_rig());
    open(&mut state, "mods/clips.ycd", dictionary);
    assert_eq!(state.active_workspace, Workspace::Animation);
    state.preview_scene.add_mesh(MeshSource {
        document: body,
        entry: None,
    });
    // Closing the last tab the person looked at leaves the workspace drawing its sources and none active
    state.active_document = None;
    assert_eq!(
        levels(&state),
        [(0, 15.0), (2, 90.0)],
        "the preview draws the source, whichever document is active"
    );
}

#[test]
fn a_texture_dictionary_that_closes_leaves_the_clip_session_as_it_was() {
    let mut state = AppState::from_prefs(Prefs::default());
    let rig = open(&mut state, "mods/rig.ydr", a_rig());
    let pictures = open(&mut state, "mods/pictures.ytd", textures("jacket"));
    let dictionary = clips(a_clip_over_the_rig());
    let dictionary = open(&mut state, "mods/clips.ycd", dictionary);
    state.preview_scene.set_clip(dictionary, CLIP);
    state
        .preview_scene
        .pin_skeleton(Some(SkeletonSource::Drawable(rig)));
    state.preview_scene.texture_sources.push(pictures);
    state.tick_animation();
    let built = state.anim.skin_revision();
    state.tick_animation();
    assert_eq!(
        state.anim.skin_revision(),
        built,
        "a second tick of a paused clip poses nothing again"
    );

    state.close_document(pictures);
    state.tick_animation();
    assert!(state.preview_scene.texture_sources.is_empty());
    assert_eq!(
        state.anim.skin_revision(),
        built,
        "a texture dictionary is no input of the clip session, so closing one builds it no more"
    );
}

#[test]
fn a_dictionary_is_pinned_in_the_view_only_when_every_drawn_source_pins_it() {
    let mut p = among("rigs/rig.ydr", &["elsewhere/pinned.ytd"], "clips/clips.ycd");
    let pinned = p.dictionaries[0];
    p.state.pin_texture_dictionary_in_view(pinned);
    assert!(
        is_pinned_in_view(&p.state, pinned),
        "the one source pins it"
    );

    // A second mesh source, which nothing pinned for
    let second = open(&mut p.state, "rigs/second.ydr", a_rig());
    // Opening a model leaves the Animation workspace, which the person returns to
    p.state.set_active_document(p.clips);
    assert_eq!(p.state.active_workspace, Workspace::Animation);
    p.state.preview_scene.add_mesh(MeshSource {
        document: second,
        entry: None,
    });
    assert_eq!(
        p.state.texture_pins_in_view(),
        [pinned],
        "it is still pinned for one of them"
    );
    assert!(
        !is_pinned_in_view(&p.state, pinned),
        "a box ticked for a source it does not pin for needs a second tick to pin that one"
    );

    p.state.pin_texture_dictionary_in_view(pinned);
    assert!(is_pinned_in_view(&p.state, pinned), "now it is every one's");
}
