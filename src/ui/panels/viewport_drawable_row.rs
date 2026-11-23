//! The viewport's row for the drawable it draws: which level of detail, to what distance, whether
//! its collision shows and whether its bones do, and a GTA VI drawable's way out to glTF.

use crate::render::gta6_drawable;
use crate::render::lines::embedded_bound;
use crate::render::viewport::bones;
use crate::render::viewport::pick::levels;
use crate::state::gta6_resource::{dictionary_drawables, drawable_of};
use crate::state::{AppState, DocPayload, DocumentId};
use crate::ui::accessible_name::named;
use crate::ui::commands::{self, Action, Command, Target};
use crate::ui::dock::Workspace;
use crate::ui::panels::hierarchy::LOD_LABELS;
use egui_phosphor::regular as ph;

const ROW_H: f32 = 22.0;
const SWITCH_W: f32 = 170.0;
const TOGGLE_W: f32 = 84.0;
const BONES_W: f32 = 70.0;
const GAP: f32 = 6.0;
/// The least the switch is shrunk to, which shows the arrow and the start of a level's name.
const SWITCH_LEAST: f32 = 40.0;

/// How much of the viewport's top right the row takes under the shading pills.
pub(crate) fn height(state: &AppState) -> f32 {
    if levels(state).is_empty()
        && !has_collision(state)
        && !bones::can_draw(state)
        && gta6_model(state).is_none()
    {
        0.0
    } else {
        ROW_H + GAP
    }
}

/// The level-of-detail switch, the collision and bones toggles and a GTA VI drawable's export, right-aligned at `top` within `rect`.
pub(crate) fn draw(ui: &mut egui::Ui, rect: egui::Rect, top: f32, state: &mut AppState) {
    let parts = Parts {
        levels: levels(state),
        collision: has_collision(state),
        skeleton: bones::can_draw(state),
        weapon: (state.active_document())
            .and_then(|doc| doc.path.as_deref())
            .is_some_and(|path| crate::state::gta6_weapon_parts::of(path).is_some()),
        export: gta6_model(state)
            .and_then(|doc| commands::on(state, Action::ExportGltf, Target::Document(doc))),
    };
    draw_parts(ui, rect, top, state, parts);
}

/// The active document where it is a scanned GTA VI body that holds a drawable.
fn gta6_model(state: &AppState) -> Option<DocumentId> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(body) = &doc.payload else {
        return None;
    };
    let holds = drawable_of(body, None).is_some() || dictionary_drawables(body).next().is_some();
    holds.then_some(doc.id)
}

/// Whether the drawable on show holds a bound Collision draws.
fn has_collision(state: &AppState) -> bool {
    embedded_bound(state).is_some() || gta6_drawable::embedded_bound(state).is_some()
}

/// What the row offers for the drawable on show.
struct Parts {
    levels: Vec<(usize, f32)>,
    collision: bool,
    skeleton: bool,
    /// Whether the document is a GTA VI weapon whose parts are kept.
    weapon: bool,
    /// Export to glTF… on a GTA VI drawable, as the command registry offers it now.
    export: Option<Command>,
}

fn draw_parts(ui: &mut egui::Ui, rect: egui::Rect, top: f32, state: &mut AppState, parts: Parts) {
    let Parts {
        levels,
        collision,
        skeleton,
        weapon,
        export,
    } = parts;
    let toggle = if collision { TOGGLE_W } else { 0.0 };
    let bones_toggle = if skeleton { BONES_W } else { 0.0 };
    let parts_toggle = if weapon { BONES_W } else { 0.0 };
    // The button is as wide as its words, which a language changes.
    let button = |text: &str| {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let colour = egui::Color32::PLACEHOLDER;
        let words = ui.painter().layout_no_wrap(text.to_string(), font, colour);
        words.size().x + 2.0 * ui.spacing().button_padding.x
    };
    let counted = [
        !levels.is_empty(),
        collision,
        skeleton,
        weapon,
        export.is_some(),
    ];
    let gaps = counted.iter().filter(|c| **c).count().saturating_sub(1);
    let toggles = toggle + bones_toggle + parts_toggle + GAP * gaps as f32;
    let least = if levels.is_empty() { 0.0 } else { SWITCH_LEAST };
    let room = rect.width() - 16.0;
    // ⚠ Where the row is too wide with the switch at its least, the button is its icon alone, named by its words
    let worded = export
        .as_ref()
        .map_or(0.0, |command| button(&command.label));
    let icon_alone = export.is_some() && least + toggles + worded > room;
    let others = toggles
        + if icon_alone {
            button(ph::EXPORT)
        } else {
            worded
        };
    // ⚠ A viewport too narrow for the whole row takes the room from the switch, which shows less of its words, and not from the toggles, which would lie past the bar and the viewport's edge
    let switch = if levels.is_empty() {
        0.0
    } else {
        (room - others).clamp(SWITCH_LEAST, SWITCH_W)
    };
    let width = (switch + others).max(0.0);
    if width <= 0.0 {
        return;
    }
    let bar = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 8.0 - width, top),
        egui::vec2(width, ROW_H),
    );
    ui.painter().rect_filled(
        bar.expand(2.0),
        crate::ui::theme::RADIUS_MEDIUM,
        crate::ui::theme::overlay_bg(),
    );
    let chosen = state.preview_scene.lod;
    let drawn = (levels.iter().copied()).find(|(level, _)| *level == chosen);
    let mut picked = drawn.or(levels.first().copied()).map(|(level, _)| level);
    let label = |(level, distance): (usize, f32)| {
        let name = LOD_LABELS.get(level).copied().unwrap_or("?");
        format!("{name} LOD to {distance:.0} m")
    };
    let mut builder = egui::UiBuilder::new().max_rect(bar);
    builder = builder.layout(egui::Layout::left_to_right(egui::Align::Center));
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        if let Some(shown) = drawn.or(levels.first().copied()) {
            // ⚠ The width of a drop-down is its least, and it grows to its words: a box of the width left holds it to that
            ui.allocate_ui_with_layout(
                egui::vec2(switch, ROW_H),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    egui::ComboBox::from_id_salt("viewport_lod")
                        .truncate()
                        .width(switch)
                        .selected_text(label(shown))
                        .show_ui(ui, |ui| {
                            for level in &levels {
                                ui.selectable_value(&mut picked, Some(level.0), label(*level));
                            }
                        })
                        .response
                        .on_hover_text(
                            "The level of detail drawn, and the distance the game draws it to",
                        );
                },
            );
        }
        if collision {
            ui.checkbox(&mut state.preview_scene.show_collision, "Collision")
                .on_hover_text("Draw the bound the drawable embeds, as lines");
        }
        if skeleton {
            // The Animation preview's Show bones is this box, which draws the bones the clip poses.
            let ticked = match state.active_workspace {
                Workspace::Animation => &mut state.preview_scene.show_bones,
                _ => &mut state.preview_scene.show_rest_bones,
            };
            ui.checkbox(ticked, "Bones").on_hover_text(
                "Draw the skeleton as lines in front of the model; click a line to select its bone",
            );
        }
        if weapon {
            ui.checkbox(&mut state.preview_scene.assemble_parts, tr!("Parts"))
                .on_hover_text(tr!(
                    "Draw the weapon as the first of each kind of its parts, each on its bone, in place of its own mesh"
                ));
        }
        if let Some(command) = &export {
            let enabled = command.enabled.is_ok();
            let words = command.label.as_str();
            let text = if icon_alone { ph::EXPORT } else { words };
            let mut response = ui.add_enabled(enabled, egui::Button::new(text));
            if icon_alone {
                response = named(response, words)
                    .on_hover_text(words)
                    .on_disabled_hover_text(words);
            }
            if commands::hints(response, command).clicked() {
                commands::run(state, ui.ctx(), command);
            }
        }
    });
    if let Some(level) = picked.filter(|level| Some(*level) != drawn.map(|(l, _)| l)) {
        state.preview_scene.lod = level;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A viewport `width` points wide drawing the row of a drawable with three levels, collision and a skeleton in `workspace`, and the frames of it.
    struct Viewport {
        ctx: egui::Context,
        rect: egui::Rect,
        state: AppState,
        /// A GTA VI drawable's export, where the row has it.
        export: Option<Command>,
    }

    impl Viewport {
        fn new(width: f32, workspace: Workspace) -> Self {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut state = AppState::from_prefs(crate::state::Prefs::default());
            state.active_workspace = workspace;
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 300.0));
            Self {
                ctx,
                rect,
                state,
                export: None,
            }
        }

        /// One frame with `events`, and the bounds of each control named, the switch as `switch`.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(String, egui::accesskit::Rect)> {
            let rect = self.rect;
            let state = &mut self.state;
            let export = &self.export;
            let output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let parts = Parts {
                        levels: vec![(0, 15.0), (1, 40.0), (3, 300.0)],
                        collision: true,
                        skeleton: true,
                        weapon: false,
                        export: export.clone(),
                    };
                    draw_parts(ui, rect, 8.0, state, parts);
                },
            );
            (output.platform_output.accesskit_update.into_iter())
                .flat_map(|update| update.nodes)
                .filter_map(|(_, node)| {
                    let switch = node.role() == egui::accesskit::Role::ComboBox;
                    let name = switch.then(|| "switch".to_string());
                    let name = name.or_else(|| node.label().map(str::to_string))?;
                    Some((name, node.bounds()?))
                })
                .collect()
        }

        /// The bounds of the controls once laid out.
        fn laid_out(&mut self) -> Vec<(String, egui::accesskit::Rect)> {
            self.frame(Vec::new());
            self.frame(Vec::new())
        }

        /// A click on the control named `name`.
        fn click(&mut self, name: &str) {
            let found = self.laid_out();
            let (_, b) = found.iter().find(|(n, _)| n == name).expect("it is drawn");
            let at = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(at), button(true)]);
            self.frame(vec![button(false)]);
        }
    }

    #[test]
    fn every_control_of_the_row_lies_inside_a_viewport_as_narrow_as_220_points() {
        // ⚠ With a GTA VI drawable's export the row at its least is wider by the button's icon: 260 points hold it.
        for (export, least) in [(false, 220.0_f64), (true, 260.0)] {
            let mut button_widths = Vec::new();
            for width in [least, 300.0, 352.0, 700.0] {
                let mut viewport = Viewport::new(width as f32, Workspace::Model);
                if export {
                    viewport.export = commands::of(&viewport.state, Action::ExportGltf);
                    assert!(viewport.export.is_some(), "no command to offer");
                }
                let words = viewport.export.as_ref().map(|c| c.label.clone());
                let found = viewport.laid_out();
                for wanted in ["switch", "Collision", "Bones"]
                    .into_iter()
                    .chain(words.as_deref())
                {
                    let Some((_, at)) = found.iter().find(|(name, _)| name == wanted) else {
                        panic!("{width} points: no `{wanted}` in {found:?}");
                    };
                    assert!(
                        at.x0 >= 0.0 && at.x1 <= width,
                        "{width} points: `{wanted}` is at {at:?}, past the viewport"
                    );
                    if Some(wanted) == words.as_deref() {
                        button_widths.push(at.x1 - at.x0);
                    }
                }
            }
            if let [narrowest, .., widest] = button_widths[..] {
                assert!(
                    narrowest < widest,
                    "the narrowest row draws the button's words: {button_widths:?}"
                );
            }
        }
    }

    #[test]
    fn the_bones_box_is_show_bones_in_the_animation_workspace_and_the_rest_bones_box_elsewhere() {
        let defaults = crate::state::PreviewScene::default();
        let mut animation = Viewport::new(700.0, Workspace::Animation);
        animation.click("Bones");
        assert_eq!(
            animation.state.preview_scene.show_bones, !defaults.show_bones,
            "the click did not move Show bones"
        );
        assert_eq!(
            animation.state.preview_scene.show_rest_bones, defaults.show_rest_bones,
            "the click moved the other workspaces' flag"
        );
        let mut model = Viewport::new(700.0, Workspace::Model);
        model.click("Bones");
        assert_eq!(
            model.state.preview_scene.show_rest_bones, !defaults.show_rest_bones,
            "the click did not move the rest bones flag"
        );
        assert_eq!(
            model.state.preview_scene.show_bones, defaults.show_bones,
            "the click moved the Animation preview's flag"
        );
    }
}
