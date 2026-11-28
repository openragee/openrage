//! The Map screen's panels: the map sets of a pack and what they place, the plan seen from above, the scene with its manipulator, and the
//! selection.
//!
//! ⚠ **Thin by design.** Every rule, every read and every write is `or-maps`'s and runs through `state.workbench.maps`; a panel reads what the
//! last scan left, draws it, and asks for a change. The manipulator's geometry is `or_world::gizmo`'s: a panel turns a pointer into a ray and
//! the ray into a pose.

mod entity;
mod list;
mod plan;
mod scanned;
mod scanned_author;
mod scanned_pose;
mod scene;
mod world;

pub use entity::draw as draw_selection;
pub use list::bar as draw_bar;
pub use list::draw as draw_list;
pub use plan::draw as draw_plan;
pub use scene::draw as draw_scene;

use crate::state::AppState;
use crate::ui::commands::{self, Action};
use crate::ui::panel_state::PanelState;

/// What the panels keep between frames.
#[derive(Default)]
pub struct MapsUi {
    /// The list's rectangle in the pass it was last drawn, which is where a drop is the Map screen's.
    drop: Option<(egui::Rect, u64)>,
    /// The same for the plan and the scene, which a drop of map files lands on as well.
    drop_plan: Option<(egui::Rect, u64)>,
    drop_scene: Option<(egui::Rect, u64)>,
    /// The manipulator's drag that is under way.
    drag: Option<scene::Dragging>,
    /// An entity dragged along the plan.
    plan_drag: Option<plan::PlanDrag>,
    /// Which GTA VI world data the plan draws over a scanned map.
    world: world::Shown,
}

impl PanelState for MapsUi {
    const KEY: &'static str = "maps";
}

/// The list's rectangle as drawn this frame, none when it was not drawn.
pub fn drop_rect(state: &AppState) -> Option<egui::Rect> {
    state.panels.peek::<MapsUi>()?.drop.map(|(rect, _)| rect)
}

/// Whether a drop at `at` lands on the Map screen's list, plan or scene, as drawn in this pass.
pub fn accepts_drop_at(ctx: &egui::Context, state: &AppState, at: egui::Pos2) -> bool {
    let pass = ctx.cumulative_pass_nr();
    state.panels.peek::<MapsUi>().is_some_and(|ui| {
        [ui.drop, ui.drop_plan, ui.drop_scene]
            .into_iter()
            .flatten()
            .any(|(rect, drawn)| drawn == pass && rect.contains(at))
    })
}

/// A name for people: its text where the files or the pack give one, else what the index of names holds, else the spelling a source's XML gives.
pub(super) fn shown(state: &AppState, name: &or_maps::Name) -> String {
    name.text
        .clone()
        .or_else(|| state.name_index.name_of(name.hash).map(str::to_string))
        .unwrap_or_else(|| name.shown())
}

/// The width of the icon buttons that take a change back and make it again.
pub(super) const STEP_BUTTON: f32 = 30.0;

/// The button that takes the last change to a map back or makes it again: an icon of one width, named for the change.
pub(super) fn step_button(ui: &mut egui::Ui, state: &mut AppState, action: Action, glyph: &str) {
    let Some(command) = commands::of(state, action) else {
        return;
    };
    let button = egui::Button::new(glyph).min_size(egui::vec2(STEP_BUTTON, 0.0));
    let response = ui.add_enabled(command.enabled.is_ok(), button);
    let response = crate::ui::accessible_name::named(response, &command.label);
    if commands::hints(response, &command).clicked() {
        commands::run(state, ui.ctx(), &command);
    }
}

/// `action` as a button: its label, greyed with the reason when it cannot run; true once it ran.
pub(super) fn action_button(
    ui: &mut egui::Ui,
    state: &mut AppState,
    action: Action,
    filled: bool,
) -> bool {
    let Some(command) = commands::of(state, action) else {
        return false;
    };
    let text = command.label.clone();
    let button = if filled {
        egui::Button::new(egui::RichText::new(text).color(crate::ui::theme::ON_ACCENT.get()))
            .fill(crate::ui::theme::ACCENT.get())
    } else {
        egui::Button::new(text)
    };
    let response = ui.add_enabled(command.enabled.is_ok(), button);
    if commands::hints(response, &command).clicked() {
        commands::run(state, ui.ctx(), &command);
        return true;
    }
    false
}
