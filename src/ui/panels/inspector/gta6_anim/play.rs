//! Playing a clip or an animation of a scanned GTA VI body on the skeleton of a scanned drawable or
//! fragment the user picks, with the timeline GTA V's clips get, and holding the pose that drawable
//! is drawn in by the Animation workspace's viewport and the Model view.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Mat4;
use or_anim::gta6::{pose_layers, posed_matrices, Binding, Unjoined};
use or_io::gta6::anim::{AnimAt, AnimResource, ClipPlay, RootKind};
use or_io::gta6::headerless::Skeleton;
use or_transport::Transport;

use super::timeline::{Gta6Timeline, Playing};
use super::ROWS_IN_VIEW;
use crate::state::{AppState, DocPayload, DocumentId, DocumentKind};
use crate::ui::dock::Workspace;
use crate::ui::panel_state::PanelState;
use crate::ui::timeline::{self, TimelineSource, TimelineState};

#[path = "play_keys.rs"]
mod keys;
#[path = "play_pose.rs"]
mod posing;

/// The timeline's height in the inspector.
const TIMELINE_HEIGHT: f32 = 260.0;

/// The width of each of the bar's pickers, which a longer name is cut to.
const PICKER_WIDTH: f32 = 220.0;

struct Model {
    doc: DocumentId,
    name: String,
    read: Result<(Skeleton, or_anim::Skeleton), String>,
    /// Each animation played so far bound to the skeleton once, by its key; emptied when another
    /// clip is chosen.
    bound: Vec<(u32, Binding)>,
    /// The skin and world matrices of the pose last shown, empty while none is: what the drawable
    /// is skinned by and where its bones are drawn.
    skin: Vec<Mat4>,
    world: Vec<Mat4>,
    /// How often `skin` changed, which the viewport compares to know it did.
    revision: u64,
    /// What the geometries of the drawable drawn that do not join the skeleton say, and what it
    /// was found for.
    unjoined: Option<(DrawnAt, Vec<String>)>,
}

/// A drawable drawn: its document, its contents' epoch, its selection and level of detail.
type DrawnAt = (DocumentId, u64, Option<u64>, usize);

/// A GTA VI animation's pose on the model picked for it.
pub(crate) struct Posed<'a> {
    pub(crate) model: DocumentId,
    pub(crate) skeleton: &'a Skeleton,
    pub(crate) rig: &'a or_anim::Skeleton,
    pub(crate) skin: &'a [Mat4],
    pub(crate) world: &'a [Mat4],
    pub(crate) revision: u64,
}

/// The pose an animation of an open GTA VI body holds on an open model, if one does.
pub(crate) fn posed(state: &AppState) -> Option<Posed<'_>> {
    let play = state.panels.peek::<PlayState>()?;
    let model = play.model.as_ref()?;
    state.document(play.doc?)?;
    state.document(model.doc)?;
    let (skeleton, rig) = model.read.as_ref().ok()?;
    (!model.skin.is_empty()).then_some(Posed {
        model: model.doc,
        skeleton,
        rig,
        skin: &model.skin,
        world: &model.world,
        revision: model.revision,
    })
}

/// The pose the viewport draws: in the Animation workspace while the animation's tab is active,
/// elsewhere while a GTA VI drawable's is.
pub(crate) fn drawn(state: &AppState) -> Option<Posed<'_>> {
    let active = state.active_document()?;
    let shows = match state.active_workspace {
        Workspace::Animation => state
            .panels
            .peek::<PlayState>()
            .is_some_and(|p| p.doc == Some(active.id)),
        _ => active.kind == DocumentKind::Gta6Drawable,
    };
    shows.then(|| posed(state)).flatten()
}

/// The model the Animation workspace draws for the active GTA VI animation, and whether it is posed.
pub(crate) fn model_drawn(state: &AppState) -> Option<(DocumentId, bool)> {
    if state.active_workspace != Workspace::Animation {
        return None;
    }
    let active = state.active_document()?;
    let play = state.panels.peek::<PlayState>()?;
    let model = play
        .model
        .as_ref()
        .filter(|_| play.doc == Some(active.id))?;
    state.document(model.doc)?;
    Some((model.doc, !model.skin.is_empty()))
}

/// Holds `pose` as the picked model's skin and world matrices, none for no pose.
fn publish(play: &mut PlayState, pose: Option<(Vec<Mat4>, Vec<Mat4>)>) {
    let (world, skin) = pose.unwrap_or_default();
    if let Some(model) = play.model.as_mut().filter(|m| m.skin != skin) {
        model.skin = skin;
        model.world = world;
        model.revision += 1;
    }
}

/// Why a geometry is drawn at bind, in words.
fn unjoined_text(why: &Unjoined, model: &str) -> String {
    match why {
        Unjoined::PaletteNotRead => {
            tr!("its model's palette is in neither measured form").to_string()
        }
        Unjoined::PastPalette { index, count } => tr!(
            "index {index} is past its palette of {count}",
            index = index,
            count = count
        ),
        Unjoined::NoSuchHash { hash } => tr!(
            "its palette names hash_{hash}, the name hash of no bone of {model}",
            hash = format!("{hash:08X}"),
            model = model
        ),
        Unjoined::NoOwnSkeleton => {
            tr!("its indices are bones of the drawable's own skeleton, which was not read")
                .to_string()
        }
        Unjoined::PastOwnSkeleton { index, count } => tr!(
            "index {index} is past the {count} bones of the drawable's own skeleton",
            index = index,
            count = count
        ),
        Unjoined::NoSuchOwnBone { name } => tr!(
            "its own skeleton's bone {bone} is no bone of {model}",
            bone = name,
            model = model
        ),
    }
}

/// What the geometries of the drawable drawn that do not join `skeleton` say: the active one in
/// the Model view, the picked one in the Animation workspace.
fn unjoined_lines(state: &AppState, model: &mut Model) -> Vec<String> {
    let doc = match state.active_document() {
        Some(d) if d.kind == DocumentKind::Gta6Drawable => d,
        _ if state.active_workspace == Workspace::Animation => {
            let Some(doc) = state.document(model.doc) else {
                return Vec::new();
            };
            doc
        }
        _ => return Vec::new(),
    };
    let Ok((skeleton, _)) = &model.read else {
        return Vec::new();
    };
    let selection = state.selection.selected(doc.id);
    let key = (
        doc.id,
        doc.payload_epoch(),
        selection,
        state.preview_scene.lod,
    );
    if model.unjoined.as_ref().is_none_or(|(k, _)| *k != key) {
        let found = crate::render::gta6_drawable::unjoined(doc, selection, key.3, skeleton);
        let lines = found
            .iter()
            .map(|(m, g, why)| {
                tr!(
                    "Model {model} geometry {geometry} is drawn at bind: {why}",
                    model = m,
                    geometry = g,
                    why = unjoined_text(why, &model.name)
                )
            })
            .collect();
        model.unjoined = Some((key, lines));
    }
    model
        .unjoined
        .as_ref()
        .map(|(_, l)| l.clone())
        .unwrap_or_default()
}

/// What the last frame posed, or why it posed nothing.
#[derive(Default)]
struct Shown {
    why_not_posed: Option<String>,
    error: Option<String>,
    skipped: usize,
    bones: Vec<u16>,
}

#[derive(Default)]
pub(super) struct PlayState {
    doc: Option<DocumentId>,
    choice: usize,
    model: Option<Model>,
    transport: Transport,
    timeline: TimelineState,
    shown: Shown,
    pose_layer: posing::PoseLayer,
}

impl PanelState for PlayState {
    const KEY: &'static str = "gta6-anim-play";
}

/// What a body offers to play: each clip, or each animation where it has no clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Clip(usize),
    Animation(AnimAt),
}

/// How many [`Choice`]s `resource` offers.
pub(crate) fn choice_count(resource: &AnimResource) -> usize {
    if resource.clips.is_empty() {
        resource.sets.iter().map(|s| s.animations.len()).sum()
    } else {
        resource.clips.len()
    }
}

/// The `index`-th [`Choice`], the clips' order or the sets' animations one after another.
pub(super) fn choice(resource: &AnimResource, index: usize) -> Option<Choice> {
    if !resource.clips.is_empty() {
        return (index < resource.clips.len()).then_some(Choice::Clip(index));
    }
    let mut left = index;
    for (set, s) in resource.sets.iter().enumerate() {
        if left < s.animations.len() {
            return Some(Choice::Animation(AnimAt {
                set,
                animation: left,
            }));
        }
        left -= s.animations.len();
    }
    None
}

fn animation_name(resource: &AnimResource, at: AnimAt) -> &str {
    (resource.sets.get(at.set))
        .and_then(|s| s.animations.get(at.animation))
        .map_or("", |a| a.name.as_str())
}

/// The `index`-th choice's name and its length in seconds.
pub(crate) fn choice_label(resource: &AnimResource, index: usize) -> Option<(String, f32)> {
    Some(match choice(resource, index)? {
        Choice::Animation(at) => {
            let animation = resource.sets.get(at.set)?.animations.get(at.animation)?;
            (animation.name.clone(), animation.duration)
        }
        Choice::Clip(c) => {
            let clip = &resource.clips[c];
            match &clip.play {
                ClipPlay::One {
                    animation,
                    start,
                    end,
                    rate,
                    ..
                } => (
                    format!("{} — {}", clip.name, animation_name(resource, *animation)),
                    super::timeline::window(*start, *end, *rate).unwrap_or(0.0),
                ),
                ClipPlay::Groups { total, .. } => (clip.name.clone(), *total),
                ClipPlay::Unresolved => (clip.name.clone(), 0.0),
            }
        }
    })
}

/// The animation sets other bodies store that `resource`, open as `doc`, names, from the open
/// documents and the searches of a scan of the same archive.
fn external_sets(
    state: &AppState,
    doc: Option<DocumentId>,
    resource: &AnimResource,
) -> Vec<Arc<AnimResource>> {
    if resource.external_sets.is_empty() {
        return Vec::new();
    }
    // A scanned body's path is its archive's joined with its offset: one parent, one archive.
    let archive = doc.and_then(|d| state.document(d)?.path.as_deref()?.parent());
    let mut sets: Vec<Arc<AnimResource>> = (state.documents.iter())
        .filter(|d| d.path.as_deref().and_then(Path::parent) == archive)
        .filter_map(|d| match &d.payload {
            DocPayload::Gta6Anim(r)
                if r.root == RootKind::AnimationSet && resource.external_sets.contains(&r.key) =>
            {
                Some(Arc::clone(r))
            }
            _ => None,
        })
        .collect();
    let searched = (resource.external_sets.iter())
        .filter(|key| !sets.iter().any(|s| s.key == **key))
        .filter_map(|key| crate::state::gta6_set_finding::found_set(archive?, *key))
        .collect::<Vec<_>>();
    sets.extend(searched);
    sets
}

/// Whether `doc` is a scanned body that may be a drawable or a fragment.
fn may_hold_a_skeleton(doc: &crate::state::Document) -> bool {
    let scanned = doc
        .path
        .as_deref()
        .and_then(|p| p.file_stem()?.to_str())
        .is_some_and(|stem| stem.starts_with("0x"));
    scanned && matches!(doc.kind, DocumentKind::Gta6Drawable | DocumentKind::Text)
}

/// The play state of `doc_id`, the choice following its Outline selection.
fn take(state: &mut AppState, doc_id: DocumentId, resource: &AnimResource) -> PlayState {
    let mut play = state.panels.take::<PlayState>();
    if play.doc != Some(doc_id) {
        play = PlayState {
            doc: Some(doc_id),
            ..PlayState::default()
        };
    }
    let selected = state.selection.selected(doc_id);
    if let Some(index) = selected.and_then(|s| usize::try_from(s).ok()) {
        if index < choice_count(resource) && index != play.choice {
            play.choice = index;
            play.transport.time = 0.0;
            if let Some(model) = play.model.as_mut() {
                model.bound.clear();
            }
        }
    }
    play
}

/// The player of the active GTA VI animation, as the Animation workspace's timeline panel draws it.
pub(crate) fn draw_timeline(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Gta6Anim(resource)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let resource = Arc::clone(resource);
    let mut play = take(state, doc_id, &resource);
    draw_player(ui, state, doc_id, &resource, &mut play, None);
    state.panels.put(play);
}

pub(super) fn draw_play(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    resource: &AnimResource,
) {
    let mut play = take(state, doc_id, resource);
    // The Animation workspace's timeline panel draws the player of the tab on screen; an animation
    // no timeline panel shows, as one this inspector is pinned to beside another tab, has it here.
    if state.active_workspace == Workspace::Animation && state.active_document == Some(doc_id) {
        draw_pose(ui, state, &mut play);
        posing::draw(ui, state, doc_id, resource, &mut play);
        let playing = choice(resource, play.choice);
        super::face::draw_expressions(ui, state, resource, playing, play.transport.time);
    } else {
        egui::CollapsingHeader::new(tr!("Play on a model"))
            .id_salt(("gta6_anim_play", doc_id))
            .default_open(true)
            .show(ui, |ui| {
                let height = Some(TIMELINE_HEIGHT);
                draw_player(ui, state, doc_id, resource, &mut play, height);
                draw_pose(ui, state, &mut play);
                posing::draw(ui, state, doc_id, resource, &mut play);
                let playing = choice(resource, play.choice);
                super::face::draw_expressions(ui, state, resource, playing, play.transport.time);
            });
    }
    state.panels.put(play);
}

/// The player: the bar that picks what plays and on which model, over the timeline, `height` tall
/// or as tall as the room left.
fn draw_player(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    resource: &AnimResource,
    play: &mut PlayState,
    height: Option<f32>,
) {
    if choice_count(resource) == 0 {
        ui.label(tr!("No clip or set of this body plays an animation the body stores, so nothing here plays."));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        if let Some(index) = draw_pickers(ui, state, resource, play) {
            state.selection.select(doc_id, index as u64);
        }
        ui.checkbox(
            &mut state.preview_scene.apply_root_motion,
            tr!("Apply root motion"),
        )
        .on_hover_text(tr!(
            "Let the mover tracks move the model's root; off, the animation plays in place."
        ));
        let waiting = super::timeline::waiting_sets(resource, &external_sets(state, Some(doc_id), resource));
        if !state.document(doc_id).is_some_and(crate::state::Document::is_from_rpf9_table)
            && !waiting.is_empty()
            && ui
                .button(tr!("Find the sets it names in the scan"))
                .on_hover_text(tr!(
                    "Reads the animation bodies the scan of this body's archive found, as a job, for the sets {sets}, which other bodies store; their layers then play.",
                    sets = super::timeline::keys(&waiting)
                ))
                .clicked()
        {
            if let Err(why) = state.find_gta6_sets(doc_id) {
                state.show_error_toast(why);
            }
        }
    });
    keys::draw(ui, state, doc_id, resource, play);
    let height = height.unwrap_or_else(|| ui.available_height());
    let size = egui::vec2(ui.available_width(), height);
    advance(ui, state, resource, play, size);
}

/// One picker of the bar: its label, then a combo box of [`PICKER_WIDTH`] showing `selected`.
fn picker(ui: &mut egui::Ui, label: &str, selected: String, rows: impl FnOnce(&mut egui::Ui)) {
    let named = ui.label(label);
    // ⚠ A combo box lays itself out in a row that takes what is left of this one, so it wraps
    // only as a space of the width it is held to.
    let room = egui::vec2(PICKER_WIDTH, ui.spacing().interact_size.y);
    ui.allocate_ui(room, |ui| {
        egui::ComboBox::from_id_salt(label)
            .width(PICKER_WIDTH)
            .truncate()
            .selected_text(selected)
            .show_ui(ui, rows)
            .response
            .labelled_by(named.id);
    });
}

/// The animation and model pickers; returns the choice picked, if one was.
fn draw_pickers(
    ui: &mut egui::Ui,
    state: &AppState,
    resource: &AnimResource,
    play: &mut PlayState,
) -> Option<usize> {
    let count = choice_count(resource);
    play.choice = play.choice.min(count - 1);
    let label = |i| choice_label(resource, i).map_or_else(String::new, |(l, _)| l);
    let mut picked = None;
    picker(ui, tr!("Animation"), label(play.choice), |ui| {
        // ⚠ Only the rows in view: a dictionary may hold hundreds of clips.
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical().show_rows(ui, row_height, count, |ui, range| {
            for i in range {
                if ui.selectable_label(i == play.choice, label(i)).clicked() {
                    picked = Some(i);
                }
            }
        });
    });
    if let Some(i) = picked {
        play.choice = i;
        play.transport.time = 0.0;
        if let Some(model) = play.model.as_mut() {
            model.bound.clear();
        }
    }
    let shown = play.model.as_ref().map(|m| m.name.clone());
    let shown = shown.unwrap_or_else(|| tr!("none").to_string());
    picker(ui, tr!("Model"), shown, |ui| {
        let models: Vec<(DocumentId, String, PathBuf)> = state
            .documents
            .iter()
            .filter(|d| may_hold_a_skeleton(d))
            .filter_map(|d| Some((d.id, d.file_name(), d.path.clone()?)))
            .collect();
        if models.is_empty() {
            ui.label(tr!(
                "Open a drawable or a fragment from a scan of a GTA VI archive to pick it here."
            ));
        }
        for (doc, name, path) in models {
            let now = play.model.as_ref().is_some_and(|m| m.doc == doc);
            if ui.selectable_label(now, &name).clicked() {
                // Read when the body opened, so picking reads nothing from disk.
                // A head is posed by the face skeleton its name names, where one is open.
                let read = crate::state::gta6_face::model_skeleton(state, doc, &path);
                let read = read.map(|source| {
                    let skeleton = or_anim::gta6::skeleton(&source);
                    (source, skeleton)
                });
                play.model = Some(Model {
                    doc,
                    name,
                    read,
                    bound: Vec::new(),
                    skin: Vec::new(),
                    world: Vec::new(),
                    revision: 0,
                    unjoined: None,
                });
            }
        }
    });
    picked
}

/// Ticks the transport, draws the timeline in `size` and poses the picked model at the playhead.
fn advance(
    ui: &mut egui::Ui,
    state: &AppState,
    resource: &AnimResource,
    play: &mut PlayState,
    size: egui::Vec2,
) {
    let Some(choice) = choice(resource, play.choice) else {
        return;
    };
    let external = external_sets(state, play.doc, resource);
    if play.transport.playing {
        play.transport.tick(ui.input(|i| i.stable_dt));
        ui.ctx().request_repaint();
    }
    let playing = Playing::new(resource, choice, &external, play.transport.time);
    if (play.transport.bounds.end - playing.duration).abs() > f32::EPSILON {
        play.transport.set_duration(playing.duration);
    }
    let waiting = super::timeline::waiting_sets(resource, &external);

    if let Some(Model {
        read: Ok((source, _)),
        bound,
        ..
    }) = &mut play.model
    {
        for layer in &playing.layers {
            if !bound.iter().any(|(key, _)| *key == layer.animation.key) {
                bound.push((layer.animation.key, Binding::build(source, layer.animation)));
            }
        }
    }
    let clip = match choice {
        Choice::Clip(c) => resource.clips.get(c),
        Choice::Animation(_) => None,
    };
    let model = match &play.model {
        None => Err(tr!("No model is picked, so no bone is posed: pick a scanned drawable or fragment whose skeleton names the bones this animation moves.").to_string()),
        Some(Model { read: Err(why), .. }) => Err(why.clone()),
        Some(Model { read: Ok((source, skeleton)), name, bound, .. }) => Ok((source, skeleton, name, bound.as_slice())),
    };
    let nothing = playing.layers.is_empty().then(|| match clip.map(|c| &c.play) {
        Some(ClipPlay::Unresolved) => tr!("An entry of this clip points at no animation of the sets, so what it plays is not known and nothing plays.").to_string(),
        _ => tr!(
            "Every animation this clip plays is in a set another body stores: {sets}. Open that body from the scan of its archive and its layers play.",
            sets = super::timeline::keys(&waiting)
        ),
    });
    let why_not_posed = nothing.or_else(|| match &model {
        Err(why) => Some(why.clone()),
        Ok((_, _, name, bound)) if bound.iter().all(|(_, b)| b.bones.is_empty()) => Some(tr!(
            "No DOF of this animation names a bone of {model}: their ids are the hashes of other bones' names, so the model is not posed.",
            model = name
        )),
        _ => None,
    });

    let expanded = play.timeline.rows.clone();
    let source = Gta6Timeline::new(
        clip,
        &playing,
        model.as_ref().ok().map(|(s, _, _, b)| (*s, *b)),
        why_not_posed.clone(),
        &waiting,
        &|id| expanded.is_expanded(id),
    );
    play.transport.frame_rate = source.frame_rate();
    play.timeline.ensure_subject(play.choice as u64);
    ui.allocate_ui(size, |ui| {
        let output = timeline::draw(ui, &source, &mut play.transport, &mut play.timeline);
        if let Some(time) = output.seek_to {
            play.transport.time = play.transport.range.clamp(time);
        }
    });

    let mut shown = Shown {
        why_not_posed,
        ..Shown::default()
    };
    let pose = match (&shown.why_not_posed, &model) {
        (None, Ok((_, skeleton, _, bound))) => {
            shown.skipped = bound.iter().map(|(_, b)| b.skipped.len()).sum();
            let layers: Vec<_> = (playing.layers.iter())
                .filter_map(|l| {
                    let (_, b) = bound.iter().find(|(key, _)| *key == l.animation.key)?;
                    Some((b, l.animation, l.start, l.time?))
                })
                .collect();
            match pose_layers(skeleton, &layers) {
                Ok(mut pose) => {
                    posing::apply(&mut play.pose_layer, &mut pose);
                    if !state.preview_scene.apply_root_motion {
                        pose.root_motion = or_anim::BoneTransform::IDENTITY;
                    }
                    let mut bones: Vec<u16> = (bound.iter())
                        .flat_map(|(_, b)| b.bones.iter().map(|d| d.bone))
                        .collect();
                    bones.sort_unstable();
                    bones.dedup();
                    shown.bones = bones;
                    Some(posed_matrices(&pose, skeleton))
                }
                Err(e) => {
                    shown.error = Some(tr!(
                        "The animation does not decode at this time: {e}",
                        e = e
                    ));
                    None
                }
            }
        }
        _ => None,
    };
    publish(play, pose);
    play.shown = shown;
}

/// What the last frame posed: the bones and where each is, or why nothing is posed.
fn draw_pose(ui: &mut egui::Ui, state: &AppState, play: &mut PlayState) {
    // The timeline's header already says why no bone is posed.
    if play.shown.why_not_posed.is_some() {
        return;
    }
    if let Some(error) = &play.shown.error {
        ui.label(error.as_str());
        return;
    }
    let Some(Model {
        read: Ok((skeleton, _)),
        world,
        ..
    }) = &play.model
    else {
        return;
    };
    ui.label(tr_n!(
        "{count} DOF moves no bone: the timeline marks it, and says why.",
        "{count} DOFs move no bone: the timeline marks each, and says why.",
        play.shown.skipped,
    ));
    let bones = &play.shown.bones;
    ui.label(tr_n!(
        "{count} bone is posed. Where each posed bone is, in the model's space:",
        "{count} bones are posed. Where each posed bone is, in the model's space:",
        bones.len(),
    ));
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt("gta6_anim_play_bones")
        .max_height(row_height * ROWS_IN_VIEW)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, bones.len(), |ui, range| {
            for &bone in &bones[range] {
                let i = usize::from(bone);
                let (Some(bone), Some(m)) = (skeleton.bones.get(i), world.get(i)) else {
                    continue;
                };
                let p = m.w_axis;
                ui.monospace(format!(
                    "{} ({:.3}, {:.3}, {:.3})",
                    bone.name, p.x, p.y, p.z
                ));
            }
        });
    ui.label(tr!("The Animation workspace's viewport draws the picked drawable in this pose; the Model view draws a scanned GTA VI drawable in it while this inspector is pinned and the drawable's tab is active."));
    let lines = (play.model.as_mut())
        .map(|m| unjoined_lines(state, m))
        .unwrap_or_default();
    for line in lines {
        ui.colored_label(crate::ui::theme::WARNING, line);
    }
}
