//! The scene: the shown set as boxes in the world, viewed from a camera that orbits, with the manipulator at the entity selected.
//!
//! A click picks the box under the pointer; a handle of the manipulator taken and dragged moves, turns or scales the entity, held to the
//! grid and the ground the person set, and let go writes the new placement to the map file as one step of Undo. The same placement is
//! reached from the keyboard: the arrows, Page Up and Page Down move the entity a step, the brackets turn it, and the Selection panel takes
//! the numbers typed. Nothing here needs the mouse alone.
//!
//! ⚠ The geometry is `or_maps::gizmo`'s and the lines are `crate::render::map_overlay`'s; this lays out the tools and turns the pointer into a
//! ray. The set is drawn as boxes: a model is drawn where its source is found in the pack, which this view does not yet do.

use glam::{Vec2, Vec3};
use or_maps::edit::Pose;
use or_maps::gizmo::{self, Drag, Gizmo, Handle, Mode, View};

use super::{action_button, MapsUi};
use crate::render::map_overlay;
use crate::render::viewport::CameraInputs;
use crate::render::RenderBridge;
use crate::state::workbench::{Pick, PickKind, Preview};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::commands::Action;
use crate::ui::theme;

/// The least the 3D view is drawn when the dock is far too short for everything.
const VIEW_MIN: f32 = 96.0;

/// A manipulator drag that is under way.
pub(super) struct Dragging {
    drag: Drag,
    /// The index of the entity among the scene's placed boxes.
    at: usize,
    start: Pose,
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    if !state.shows_maps() {
        crate::ui::panels::empty_state::draw(ui, state);
        return;
    }
    let mut shown = state.panels.take::<MapsUi>();
    shown.drop_scene = Some((ui.max_rect(), ui.ctx().cumulative_pass_nr()));
    tools(ui, state);
    view(ui, state, bridge, &mut shown);
    state.panels.put(shown);
}

/// The tools: what a drag changes, how it is held, and the commands that stand on the entity selected.
fn tools(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(tr!("Tool")).color(theme::TEXT_SECONDARY));
        for (mode, label, name) in [
            (
                Mode::Move,
                tr!("Move"),
                tr!("Move tool: drag an arrow or a square"),
            ),
            (Mode::Rotate, tr!("Turn"), tr!("Turn tool: drag a ring")),
            (
                Mode::Scale,
                tr!("Scale"),
                tr!("Scale tool: drag an arrow or the centre"),
            ),
        ] {
            let on = state.workbench.maps.tool == mode;
            let response = ui.selectable_label(on, label);
            if named(response, name).clicked() {
                state.workbench.maps.tool = mode;
            }
        }
        action_button(ui, state, Action::DropToGround, false);
    });
    ui.horizontal_wrapped(|ui| {
        let snap = &mut state.workbench.maps.snap;
        let grid = ui.checkbox(&mut snap.grid_on, tr!("Grid"));
        named(grid, tr!("Hold positions to the grid"));
        let step = ui.add(
            egui::DragValue::new(&mut snap.grid)
                .range(0.01..=100.0)
                .speed(0.05)
                .suffix(" m"),
        );
        named(step, tr!("Grid step in metres"));
        let turn = ui.checkbox(&mut snap.turn_on, tr!("Turns"));
        named(turn, tr!("Hold turns to a step"));
        let turn_step = ui.add(
            egui::DragValue::new(&mut snap.turn)
                .range(0.1..=180.0)
                .speed(0.5)
                .suffix("°"),
        );
        named(turn_step, tr!("Turn step in degrees"));
        let scale = ui.checkbox(&mut snap.scale_on, tr!("Scales"));
        named(scale, tr!("Hold scales to a step"));
        let scale_step = ui.add(
            egui::DragValue::new(&mut snap.scale)
                .range(0.01..=10.0)
                .speed(0.01),
        );
        named(scale_step, tr!("Scale step"));
        let ground = ui.checkbox(&mut snap.ground, tr!("On the ground"));
        named(ground, tr!("Keep the entity on the ground while it moves"));
        let floor = ui.add(
            egui::DragValue::new(&mut snap.floor)
                .speed(0.1)
                .suffix(" m"),
        );
        named(floor, tr!("Ground height in metres"));
    });
}

/// The camera inputs a pointer over the view makes when it is not on a handle.
fn camera_inputs(ui: &egui::Ui, response: &egui::Response) -> CameraInputs {
    let mut inputs = CameraInputs::default();
    if response.dragged_by(egui::PointerButton::Middle)
        || (response.dragged_by(egui::PointerButton::Primary) && ui.input(|i| i.modifiers.shift))
    {
        inputs.pan = response.drag_delta();
    } else if response.dragged_by(egui::PointerButton::Primary) {
        inputs.orbit = response.drag_delta();
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        if scroll.y != 0.0 {
            inputs.zoom = (scroll.y / 80.0).clamp(-2.0, 2.0);
        }
    }
    inputs
}

/// The pose a key moves the entity to: the arrows and Page Up and Down move a step (ten with Shift), the brackets turn about the vertical.
fn nudge(ui: &egui::Ui, state: &AppState, pose: Pose) -> Option<(Pose, String)> {
    let snap = state.workbench.maps.snap;
    let step = if snap.grid_on { snap.grid } else { 0.1 };
    let (events, shift) = ui.input(|i| (i.events.clone(), i.modifiers.shift));
    let times = if shift { 10.0 } else { 1.0 };
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            ..
        } = event
        else {
            continue;
        };
        let by = |x: f32, y: f32, z: f32| Pose {
            position: pose.position + Vec3::new(x, y, z) * step * times,
            ..pose
        };
        let turn = |degrees: f32| Pose {
            rotation: (glam::Quat::from_rotation_z(degrees.to_radians()) * pose.rotation)
                .normalize(),
            ..pose
        };
        let turn_by = if snap.turn_on { snap.turn } else { 1.0 } * times;
        let moved = match key {
            egui::Key::ArrowLeft => (by(-1.0, 0.0, 0.0), tr!("Nudged west")),
            egui::Key::ArrowRight => (by(1.0, 0.0, 0.0), tr!("Nudged east")),
            egui::Key::ArrowUp => (by(0.0, 1.0, 0.0), tr!("Nudged north")),
            egui::Key::ArrowDown => (by(0.0, -1.0, 0.0), tr!("Nudged south")),
            egui::Key::PageUp => (by(0.0, 0.0, 1.0), tr!("Raised")),
            egui::Key::PageDown => (by(0.0, 0.0, -1.0), tr!("Lowered")),
            egui::Key::OpenBracket => (turn(turn_by), tr!("Turned left")),
            egui::Key::CloseBracket => (turn(-turn_by), tr!("Turned right")),
            _ => continue,
        };
        return Some((moved.0, moved.1.to_string()));
    }
    None
}

/// The 3D view: orbit by dragging, pan with the middle button or shift, zoom with the wheel; a handle of the manipulator taken moves the entity.
fn view(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bridge: Option<&mut RenderBridge>,
    shown: &mut MapsUi,
) {
    let size = egui::vec2(ui.available_width(), ui.available_height().max(VIEW_MIN));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let response = named(response, tr!("Scene of the map set"));
    let aspect = (rect.width() / rect.height().max(1.0)).max(0.0001);
    let screen = View {
        view_proj: state.workbench.maps.camera.view_proj(aspect),
        origin: Vec2::new(rect.min.x, rect.min.y),
        size: Vec2::new(rect.width().max(1.0), rect.height().max(1.0)),
    };

    // Where the manipulator stands, and which handle the pointer is over.
    let mode = state.workbench.maps.tool;
    let selected = state.selected_pose();
    let selected_at = state
        .selected_placed()
        .and_then(|p| state.shown_scene()?.find(&p.source, p.index));
    let pose_now = shown
        .drag
        .as_ref()
        .and_then(|_| state.workbench.maps.preview.map(|p| p.pose))
        .or(selected);
    let editable = state.places_selected();
    let gizmo = pose_now.filter(|_| editable).and_then(|pose| {
        Gizmo::at(
            mode,
            pose.position,
            map_overlay::frame_of(mode, pose.rotation),
            &screen,
        )
    });
    let pointer = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos())
        .map(|p| Vec2::new(p.x, p.y));
    let hovered = match (&gizmo, pointer, &shown.drag) {
        (Some(g), Some(p), None) => g.pick(&screen, p),
        (_, _, Some(d)) => Some(d.drag.handle),
        _ => None,
    };
    state.workbench.maps.hovered = hovered;

    // A handle taken, a drag under way, a drag let go.
    let mut inputs = CameraInputs::default();
    let mut commit: Option<(Pose, usize)> = None;
    if response.drag_started_by(egui::PointerButton::Primary) {
        let press = response.interact_pointer_pos().map(|p| Vec2::new(p.x, p.y));
        if let (Some(g), Some(p), Some(at), Some(start)) = (&gizmo, press, selected_at, selected) {
            if let (Some(handle), Some(ray)) = (g.pick(&screen, p), screen.ray(p)) {
                shown.drag =
                    Drag::begin(*g, handle, start, &ray).map(|drag| Dragging { drag, at, start });
            }
        }
    }
    if let Some(drag) = &shown.drag {
        if let Some(p) = response.interact_pointer_pos() {
            let p = Vec2::new(p.x, p.y);
            if let Some(ray) = screen.ray(p) {
                let invert = ui.input(|i| i.modifiers.command);
                let work = &state.workbench.maps;
                let mut pose = drag.drag.pose(&ray, work.snap.for_drag(invert));
                let on_the_ground = work.snap.ground
                    && !invert
                    && drag.drag.mode == Mode::Move
                    && !matches!(drag.drag.handle, Handle::Axis(gizmo::Axis::Z));
                if on_the_ground {
                    if let Some(scene) = state.shown_scene() {
                        pose = gizmo::drop_to_ground(&scene.obbs(), drag.at, pose, work.snap.floor);
                    }
                }
                let index = state
                    .shown_scene()
                    .and_then(|s| s.placed.get(drag.at))
                    .map_or(0, |p| p.index);
                state.workbench.maps.preview = Some(Preview { index, pose });
            }
        }
        if response.drag_stopped() {
            if let Some(Preview { pose, .. }) = state.workbench.maps.preview {
                if pose != drag.start {
                    commit = Some((pose, drag.at));
                }
            }
            shown.drag = None;
        }
    } else {
        inputs = camera_inputs(ui, &response);
    }
    if shown.drag.is_none() && commit.is_none() {
        // The preview of a drag that was let go is cleared once its placement is written.
        if state.workbench.maps.preview.is_some() && !response.dragged() {
            state.workbench.maps.preview = None;
        }
    }
    if let Some((pose, at)) = commit {
        let target = state
            .shown_scene()
            .and_then(|s| s.placed.get(at))
            .map(|p| (p.source.clone(), p.index));
        let label = match mode {
            Mode::Move => tr!("Moved entity"),
            Mode::Rotate => tr!("Turned entity"),
            Mode::Scale => tr!("Scaled entity"),
        };
        if let Some((source, index)) = target {
            state.place_entity_in(&source, index, pose, format!("{label} {index}"));
        }
    }

    // A click picks the box under the pointer.
    if response.clicked() && shown.drag.is_none() && hovered.is_none() {
        response.request_focus();
        if let Some(ray) = response
            .interact_pointer_pos()
            .and_then(|p| screen.ray(Vec2::new(p.x, p.y)))
        {
            pick_under(state, &ray);
        }
    }
    if response.double_clicked() && hovered.is_none() {
        state.open_selected_gta6_model();
    }

    // The keyboard does what the handles do.
    if response.has_focus() {
        // ⚠ The arrows are ours: without the lock egui moves the keyboard to the widget beside the scene at the first one.
        crate::ui::focus_ring::reads_typed_text(ui.ctx(), response.id);
        if let (Some(pose), Some(_), true) = (selected, selected_at, editable) {
            if let Some((moved, label)) = nudge(ui, state, pose) {
                state.place_selected_entity(moved, label);
            }
        }
        // ⚠ Through the command, as the model viewport reads it: a key the settings give Frame selection frames here too.
        let asked = ui.input(|i| crate::ui::panels::viewport::framing_requested(i, &state.prefs));
        if asked == Some(Action::FrameSelection) {
            inputs.frame_selection = true;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            shown.drag = None;
            state.workbench.maps.preview = None;
        }
    }

    // The lines of the frame, then the view.
    state.workbench.maps.view = Some([rect.min.x, rect.min.y, rect.width(), rect.height()]);
    let mut overlay = build_overlay(state, shown, &screen);
    overlay.extend_from_slice(crate::state::gta6_map::links::lod_lines(state));
    state.workbench.maps.overlay = overlay;
    let Some(bridge) = bridge else {
        ui.painter().rect_filled(rect, 0.0, theme::BG_L2);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr!("Viewport (renderer not initialized)"),
            egui::FontId::default(),
            theme::TEXT_SECONDARY.get(),
        );
        return;
    };
    let ppp = ui.ctx().pixels_per_point();
    bridge.set_desired_size(
        (rect.width() * ppp).round() as u32,
        (rect.height() * ppp).round() as u32,
    );
    bridge.refresh_dressed_as(state.models_in_place(), None);
    bridge.refresh_lines(state);
    bridge.push_inputs(inputs);

    match bridge.texture_id() {
        Some(texture) => {
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            ui.painter().rect_filled(rect, 0.0, theme::BG_L2);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                tr!("Initializing viewport…"),
                egui::FontId::default(),
                theme::TEXT_SECONDARY.get(),
            );
        }
    }
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let said = match state.shown_scene() {
        None => Some(tr!("Reading the maps…")),
        Some(scene) if scene.placed.is_empty() => Some(tr!("This set places nothing yet")),
        Some(_) => None,
    };
    if let Some(said) = said {
        ui.painter().text(
            rect.center_bottom() - egui::vec2(0.0, 12.0),
            egui::Align2::CENTER_BOTTOM,
            said,
            egui::FontId::default(),
            theme::TEXT_SECONDARY.get(),
        );
    }
}

/// Select the box a ray meets first among those the set draws, or nothing when it meets none.
fn pick_under(state: &mut AppState, ray: &gizmo::Ray) {
    let module = state.maps_pack().map(|p| p.module.clone());
    let set = state.shown_set().map(|(_, s)| s.id.clone());
    let show_lod = state.workbench.maps.show_lod;
    let Some(scene) = state.shown_scene() else {
        return;
    };
    let boxes: Vec<(usize, gizmo::Obb)> = scene
        .placed
        .iter()
        .enumerate()
        .filter(|(_, p)| show_lod || map_overlay::is_full_detail(p.lod_level.as_deref()))
        .map(|(at, p)| (at, p.obb))
        .collect();
    let obbs: Vec<gizmo::Obb> = boxes.iter().map(|(_, o)| *o).collect();
    let hit = gizmo::pick_entity(&obbs, ray).map(|(i, _)| boxes[i].0);
    let pick = hit.map(|at| Pick {
        source: scene.placed[at].source.clone(),
        kind: PickKind::Entity(scene.placed[at].index),
    });
    state.pick_shown(module, set, pick);
}

/// The lines for this frame.
fn build_overlay(state: &AppState, shown: &MapsUi, screen: &View) -> Vec<or_renderer::LineVertex> {
    let work = &state.workbench.maps;
    let Some(scene) = state.shown_scene() else {
        return Vec::new();
    };
    let selected = work.selected.as_ref().and_then(|p| match p.kind {
        PickKind::Entity(i) => scene.find(&p.source, i),
        PickKind::Archetype(_) => None,
    });
    let drawn = selected.and_then(|at| {
        let placed = scene.placed.get(at)?;
        let mut obb = placed.obb;
        if let Some(p) = work.preview.filter(|p| p.index == placed.index) {
            obb.position = p.pose.position;
            obb.rotation = p.pose.rotation;
            obb.scale = Vec3::new(p.pose.scale_xy, p.pose.scale_xy, p.pose.scale_z);
        }
        Some(obb)
    });
    let gizmo = drawn.filter(|_| state.places_selected()).and_then(|obb| {
        Gizmo::at(
            work.tool,
            obb.position,
            map_overlay::frame_of(work.tool, obb.rotation),
            screen,
        )
    });
    let _ = shown;
    let target = work.camera.target;
    let ground = Some((
        target,
        map_overlay::grid_step(work.camera.distance),
        work.snap.floor,
    ));
    map_overlay::lines_of(
        scene,
        selected,
        drawn,
        work.show_lod,
        gizmo.as_ref().map(|g| (g, work.hovered)),
        ground,
    )
}
