//! In the Animation workspace a click holds what it picked in the box at the viewport's bottom left
//! and leaves the clip list where it is, and a double click on the box's Picked line opens the
//! picked document at the picked row.

use glam::Vec3;
use or_io::decl::Gen8;

use super::the_animation_preview_draws_what_its_sources_hold::CLIP;
use super::the_viewport_picks_what_it_draws::{
    click_on_the_bone, previewing, previewing_a_triangle, ray, Holder,
};
use super::viewport_basics_for_inspecting_assets::the_outline_shows;
use super::{AppState, DocumentId};
use crate::render::viewport::pick::{select_model_under, select_under};
use crate::ui::dock::Workspace;
use crate::ui::panels::selection_ids::dict_entry_id;
use crate::ui::panels::viewport::draw_viewport_overlay;
use crate::ui::panels::viewport_layout::column;

/// The viewport's box, drawn as the editor draws it over a viewport of its own.
struct TheBox {
    ctx: egui::Context,
    rect: egui::Rect,
    time: f64,
}

impl TheBox {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Self {
            ctx,
            rect: egui::Rect::from_min_size(egui::pos2(300.0, 40.0), egui::vec2(800.0, 500.0)),
            time: 0.0,
        }
    }

    /// The names a screen reader is given for what a frame draws.
    fn names(&mut self, state: &mut AppState) -> Vec<String> {
        let output = self.frame(state, Vec::new());
        let nodes = output.platform_output.accesskit_update.map(|u| u.nodes);
        (nodes.into_iter().flatten())
            .filter_map(|(_, node)| node.label().map(str::to_owned))
            .collect()
    }

    fn frame(&mut self, state: &mut AppState, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 900.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let gizmo = column(self.rect, 0.0, 0.0).gizmo;
        let rect = self.rect;
        (self.ctx).run_ui(input, |ui| {
            draw_viewport_overlay(ui, rect, state, None, None, gizmo)
        })
    }

    /// Where the box paints its Picked line, if it draws one: a click there is the owner's gesture,
    /// whatever rect the line's widget was given.
    fn picked_line(&mut self, state: &mut AppState) -> Option<egui::Rect> {
        self.frame(state, Vec::new());
        let output = self.frame(state, Vec::new());
        (output.shapes.iter()).find_map(|clipped| painted_row(&clipped.shape, "Picked:"))
    }

    /// `key` pressed and let go, as a keyboard does.
    fn press(&mut self, state: &mut AppState, key: egui::Key) {
        self.time += 0.05;
        for pressed in [true, false] {
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(state, vec![event]);
        }
    }

    /// `times` presses and releases at `at`, close enough to make a double click of two.
    fn click(&mut self, state: &mut AppState, at: egui::Pos2, times: usize) {
        // ⚠ Far enough from the last click that egui does not count this one with it.
        self.time += 1.0;
        self.frame(state, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..times {
            for pressed in [true, false] {
                let event = egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                self.frame(state, vec![event]);
            }
        }
    }
}

/// Where the first text row that starts with `start` is painted, on the screen.
fn painted_row(shape: &egui::Shape, start: &str) -> Option<egui::Rect> {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| painted_row(shape, start)),
        egui::Shape::Text(text) => (text.galley.rows.iter())
            .find(|row| row.text().starts_with(start))
            .map(|row| row.rect().translate(text.pos.to_vec2())),
        _ => None,
    }
}

/// The picked document is open, at the picked row, with the Outline showing it.
fn assert_opened_at(state: &mut AppState, document: DocumentId, row: u64, label: &str, what: &str) {
    assert_eq!(
        state.active_document,
        Some(document),
        "{what}: the document"
    );
    assert_eq!(
        state.selection.selected(document),
        Some(row),
        "{what}: the row"
    );
    assert_eq!(
        crate::ui::panels::hierarchy::ui_state(state).scroll_to,
        Some((document, row)),
        "{what}: the Outline is not sent to the row"
    );
    assert_ne!(state.active_workspace, Workspace::Animation, "{what}");
    assert!(
        state.workspace_docks.is_revealing(
            state.active_workspace,
            crate::ui::panels::registry::ids::OUTLINE
        ),
        "{what}: the Outline is not brought forward"
    );
    assert!(
        the_outline_shows(state, label),
        "{what}: the Outline does not show `{label}`"
    );
}

#[test]
fn a_double_click_on_the_picked_mesh_opens_its_document_at_the_geometry() {
    let clip_row = dict_entry_id(CLIP);
    for holder in Holder::ALL {
        let (mut state, mesh, clips) = previewing_a_triangle(holder, None);
        state.selection.select(clips, clip_row);
        let on_it = || ray(Vec3::new(0.25, 0.0, 0.25), -Vec3::Y, 3.0, egui::Vec2::ZERO);
        select_under(&mut state, &on_it());

        let mut overlay = TheBox::new();
        let line = overlay
            .picked_line(&mut state)
            .unwrap_or_else(|| panic!("{holder:?}: the box has no Picked line to open"));
        let names = overlay.names(&mut state);
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("Picked: Geometry 0 of model 0")
                    && name.contains("to open it in its own document")),
            "{holder:?}: no name for the Picked line in {names:?}"
        );
        overlay.click(&mut state, line.center(), 1);
        assert_eq!(
            (state.active_document, state.active_workspace),
            (Some(clips), Workspace::Animation),
            "{holder:?}: one click on the Picked line left the clip list"
        );

        overlay.click(&mut state, line.center(), 2);
        assert_opened_at(
            &mut state,
            mesh,
            holder.geometry(0),
            "Geometry 0",
            &format!("{holder:?} geometry"),
        );

        // With Alt the pick is the model that holds the geometry, and so is the row.
        let (mut state, mesh, _) = previewing_a_triangle(holder, None);
        select_model_under(&mut state, &on_it());
        let line = overlay.picked_line(&mut state).expect("a Picked line");
        overlay.click(&mut state, line.center(), 2);
        assert_opened_at(
            &mut state,
            mesh,
            holder.model(),
            "Model 0",
            &format!("{holder:?} model"),
        );
    }
}

#[test]
fn a_double_click_on_the_picked_bone_opens_its_rig_at_the_bone() {
    for holder in Holder::ALL {
        let mut p = previewing::<Gen8>(holder);
        select_under(&mut p.state, &click_on_the_bone());

        let mut overlay = TheBox::new();
        let line = overlay
            .picked_line(&mut p.state)
            .unwrap_or_else(|| panic!("{holder:?}: the box has no Picked line to open"));
        assert_eq!(
            p.state.active_document,
            Some(p.clips),
            "{holder:?}: the pick took the clip list away"
        );
        // Only the Picked line opens the pick: the line above it, which the box also paints, does not.
        let above = line.translate(egui::vec2(0.0, -line.height()));
        overlay.click(&mut p.state, above.center(), 2);
        assert_eq!(
            (p.state.active_document, p.state.active_workspace),
            (Some(p.clips), Workspace::Animation),
            "{holder:?}: a double click above the Picked line opened the pick"
        );
        overlay.click(&mut p.state, line.center(), 2);
        assert_opened_at(
            &mut p.state,
            p.rig,
            holder.bone(1),
            "BONE_1 (1)",
            &format!("{holder:?} bone"),
        );
    }
}

#[test]
fn enter_on_the_focused_picked_line_opens_the_pick_as_a_double_click_does() {
    for holder in Holder::ALL {
        let (mut state, mesh, clips) = previewing_a_triangle(holder, None);
        state.selection.select(clips, dict_entry_id(CLIP));
        select_under(
            &mut state,
            &ray(Vec3::new(0.25, 0.0, 0.25), -Vec3::Y, 3.0, egui::Vec2::ZERO),
        );
        let mut overlay = TheBox::new();
        overlay
            .picked_line(&mut state)
            .unwrap_or_else(|| panic!("{holder:?}: the box has no Picked line to open"));

        // Enter with nothing focused opens nothing
        overlay.press(&mut state, egui::Key::Enter);
        assert_eq!(
            (state.active_document, state.active_workspace),
            (Some(clips), Workspace::Animation),
            "{holder:?}: Enter with the line not focused opened the pick"
        );

        // The line is a Tab stop, and Enter on it is what a double click is
        overlay.press(&mut state, egui::Key::Tab);
        overlay.press(&mut state, egui::Key::Enter);
        assert_opened_at(
            &mut state,
            mesh,
            holder.geometry(0),
            "Geometry 0",
            &format!("{holder:?} keyboard"),
        );
    }
}
