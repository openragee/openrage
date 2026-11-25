use glam::Vec3;
use or_renderer::{OrbitCamera, ShadingMode};

use crate::render::viewport::pick::{pick_under, select_model_under, select_under, Ray};
use crate::render::viewport::{CameraInputs, GameSceneReport};
use crate::render::RenderBridge;
use crate::state::{AppState, Prefs};
use crate::ui::commands::{self, Action, Command, Target};
use crate::ui::shortcuts;
use crate::ui::theme;

use super::viewport_texture_badge::{apply_texture_badge_action, draw_texture_badge};
use super::{viewport_drawable_row, viewport_layout};

pub fn draw(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    let avail = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
    let response = crate::ui::accessible_name::named(response, "3D viewport");
    // ⚠ The right button moves no camera, so it is free for the menu; the command runs once the menu is drawn.
    let mut asked = None;
    response.context_menu(|ui| menu(ui, state, &mut asked));
    if let Some(command) = asked {
        commands::run(state, ui.ctx(), &command);
    }

    let Some(bridge) = bridge else {
        ui.painter().rect_filled(rect, 0.0, theme::BG_L2);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Viewport (renderer not initialized)",
            egui::FontId::default(),
            theme::TEXT_SECONDARY.get(),
        );
        return;
    };
    let restored = state.panels.get::<ModeRestored>();
    if !restored.0 {
        restored.0 = true;
        bridge.set_mode(restored_mode(
            state.prefs.viewport.shading,
            bridge.game_shaders_available(),
        ));
    }

    let ppp = ui.ctx().pixels_per_point();
    let px_w = (rect.width() * ppp).round() as u32;
    let px_h = (rect.height() * ppp).round() as u32;
    bridge.set_desired_size(px_w, px_h);
    bridge.refresh_lines(state);
    if !matches!(bridge.mode(), ShadingMode::Wireframe) {
        bridge.refresh_meshes(state);
        bridge.refresh_game_scene(state);
        bridge.refresh_skinning(state);
        if state.texture_report.epoch != bridge.texture_report().epoch {
            state.texture_report = bridge.texture_report().clone();
        }
        // The project's copy, then the archive sibling — a dictionary
        // travelling with the file — and a configured path last.
        if state.try_open_missing_textures() {
            bridge.invalidate_texture_resolution();
            ui.ctx().request_repaint();
        }
    }
    bridge.refresh_particles(state);

    let channels = bridge.debug_channels_available();
    let column = viewport_layout::column(
        rect,
        viewport_drawable_row::height(state),
        mode_bar_height(bridge.mode(), rect, channels),
    );
    let gizmo_rect = column.gizmo;
    let press_in_gizmo = response
        .interact_pointer_pos()
        .map(|p| gizmo_rect.contains(p))
        .unwrap_or(false);
    let pointer_in_gizmo = ui
        .input(|i| i.pointer.hover_pos())
        .map(|p| gizmo_rect.contains(p))
        .unwrap_or(false);

    let mut inputs = CameraInputs::default();
    if !press_in_gizmo {
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary)
                && ui.input(|i| i.modifiers.shift))
        {
            inputs.pan = response.drag_delta();
        } else if response.dragged_by(egui::PointerButton::Primary) {
            inputs.orbit = response.drag_delta();
        }
        select_on_click(&response, state, bridge.camera(), (px_w, px_h));
    }
    let mut framing = state.panels.get::<FramingRequest>().0.take();
    if response.hovered() && !pointer_in_gizmo {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        if scroll.y != 0.0 {
            inputs.zoom = (scroll.y / 80.0).clamp(-2.0, 2.0);
        }
        // ⚠ Not while a widget has keyboard focus: an F typed into a filter is text.
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing {
            framing = framing.or(ui.input(|i| framing_requested(i, &state.prefs)));
        }
    }
    match framing {
        Some(Action::FrameSelection) => inputs.frame_selection = true,
        Some(_) => inputs.frame_active = true,
        None => {}
    }
    bridge.push_inputs(inputs);

    if let Some(tex_id) = bridge.texture_id() {
        ui.painter().image(
            tex_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        ui.painter().rect_filled(rect, 0.0, theme::BG_L2);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Initializing viewport…",
            egui::FontId::default(),
            theme::TEXT_SECONDARY.get(),
        );
    }

    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let shaded = (bridge.game_scene_report())
        .filter(|_| bridge.mode() == ShadingMode::Shaded)
        .and_then(|report| Some(report.gta6.as_ref()?.1.as_str()));
    draw_viewport_overlay(
        ui,
        rect,
        state,
        bridge.palette_retarget(),
        shaded,
        gizmo_rect,
    );
    if !matches!(bridge.mode(), ShadingMode::Wireframe) {
        if let Some(action) = draw_texture_badge(ui, rect, state) {
            apply_texture_badge_action(state, bridge, action);
        }
    }
    draw_axis_gizmo(ui, gizmo_rect, bridge);
    if let Some(mode) = draw_shading_mode_pills(ui, rect, bridge) {
        state.remember_shading_mode(mode);
    }
    viewport_drawable_row::draw(ui, rect, column.row, state);
    draw_preview_options(ui, rect, state);
    state.preview_scene.follow_shading_mode(bridge.mode());
    if matches!(bridge.mode(), ShadingMode::Shaded) {
        draw_shaded_hud_bar(ui, rect, column.bar, state);
    }
    if matches!(bridge.mode(), ShadingMode::Channel) {
        draw_channel_selector(ui, rect, column.bar, bridge.debug_channels(), state);
    }
    // ⚠ Not while a colour is dragged, which changes it every frame.
    let mut kept = state.prefs.viewport.clone();
    kept.keep(&state.drivers);
    if kept != state.prefs.viewport && !ui.input(|i| i.pointer.any_down()) {
        state.persist_viewport_prefs();
    }
}

/// What a primary click on the viewport `response` allocated does: it selects what the camera's ray
/// lands on, the model that holds it with Alt held, or goes to a plugin that asked for the clicks.
///
/// ⚠ `pixels` is the size the viewport draws at, which the ray's reach in pixels is measured in.
pub(crate) fn select_on_click(
    response: &egui::Response,
    state: &mut AppState,
    camera: &OrbitCamera,
    pixels: (u32, u32),
) {
    if !response.clicked_by(egui::PointerButton::Primary) {
        return;
    }
    let Some(at) = response.interact_pointer_pos() else {
        return;
    };
    let Some(ray) = Ray::at(camera, response.rect, pixels, at) else {
        return;
    };
    // A plugin that asked for the clicks takes them, and a click then selects nothing.
    if state.plugin_takes_clicks() {
        if let Some(pick) = pick_under(state, &ray) {
            state.plugin_pick(pick);
        }
    } else {
        match response.ctx.input(|i| i.modifiers.alt) {
            true => select_model_under(state, &ray),
            false => select_under(state, &ray),
        }
    }
}

/// Whether the shading mode the preferences keep has been handed to the bridge, which happens once.
#[derive(Default)]
struct ModeRestored(bool);

impl crate::ui::panel_state::PanelState for ModeRestored {
    const KEY: &'static str = "viewport-mode-restored";
}

/// The options of the clip preview behind a button at the viewport's top left, which opens them over the view: they say what the viewport draws, so they are a part of it and no panel beside it.
fn draw_preview_options(ui: &mut egui::Ui, rect: egui::Rect, state: &mut AppState) {
    let clips = matches!(
        state.active_document().map(|d| &d.payload),
        Some(crate::state::DocPayload::Resource(or_io::Asset::Clips(_)))
    );
    if state.active_workspace != crate::ui::dock::Workspace::Animation || !clips {
        return;
    }
    // ⚠ The words go when the pills at the right would meet them, and the glyph stays.
    let icon = egui_phosphor::regular::SLIDERS;
    let (text, width) = if rect.width() >= 340.0 {
        (tr!("{icon} Preview options", icon = icon), 150.0)
    } else {
        (icon.to_string(), 28.0)
    };
    let at = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(8.0, 8.0),
        egui::vec2(width, SHADING_PILL_H),
    );
    ui.painter()
        .rect_filled(at.expand(4.0), theme::RADIUS_MEDIUM, theme::overlay_bg());
    let button = ui.place(
        at,
        egui::Button::new(egui::RichText::new(text).small()).frame(false),
    );
    let button = crate::ui::accessible_name::named(button, tr!("Preview options")).on_hover_text(
        tr!("The clip played, the skeleton and the model it plays on, and what the preview draws"),
    );
    egui::Popup::menu(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::ui::popup_of_the_shell(ui.ctx());
            ui.set_width(340.0);
            ui.set_max_height(460.0);
            super::preview_settings::draw(ui, state);
        });
}

/// What a right click in the viewport offers: the framing of the camera, and what the model in front is exchanged as.
fn menu(ui: &mut egui::Ui, state: &AppState, asked: &mut Option<Command>) {
    match state.active_document {
        Some(doc) => crate::ui::popup_of_a_document(ui.ctx(), doc),
        None => crate::ui::popup_of_the_shell(ui.ctx()),
    }
    commands::offer_plain(ui, state, Action::FrameAll, asked);
    commands::offer_plain(ui, state, Action::FrameSelection, asked);
    let Some(doc) = state.active_document else {
        return;
    };
    if state.offers_gltf(crate::state::gltf::Model::Document(doc)) {
        let target = Target::Document(doc);
        ui.separator();
        commands::offer(ui, state, Action::ExportGltf, &target, asked);
        commands::offer(ui, state, Action::ImportGltf, &target, asked);
    }
}

/// Frame all or Frame selection, when a key pressed this frame is bound to one.
fn framing_requested(input: &egui::InputState, prefs: &Prefs) -> Option<Action> {
    input.events.iter().find_map(|event| {
        shortcuts::action_for_event(event, prefs)
            .filter(|action| matches!(action, Action::FrameAll | Action::FrameSelection))
    })
}

/// A Frame all or Frame selection run from outside the viewport, for the next viewport frame to do.
#[derive(Default)]
pub struct FramingRequest(Option<Action>);

impl crate::ui::panel_state::PanelState for FramingRequest {
    const KEY: &'static str = "viewport-framing-request";
}

pub fn request_framing(state: &mut AppState, action: Action) {
    state.panels.get::<FramingRequest>().0 = Some(action);
}

/// The framing keys as the user bound them, for the overlay.
pub(super) fn framing_hint(prefs: &Prefs) -> String {
    [
        (Action::FrameSelection, "frames the selection"),
        (Action::FrameAll, "frames everything"),
    ]
    .into_iter()
    .filter_map(|(action, what)| {
        shortcuts::label_for(action, prefs).map(|keys| format!("{keys} {what}"))
    })
    .collect::<Vec<_>>()
    .join(" · ")
}

pub(crate) const SHADING_PILL_H: f32 = 22.0;
const HUD_ROW_H: f32 = 28.0;
const HUD_W: f32 = 380.0;
const CHANNEL_BAR_H: f32 = 26.0;

/// How tall the bar under the pills is, with its gap: Shaded's, or Channel's where it offers channels.
pub(crate) fn mode_bar_height(mode: ShadingMode, rect: egui::Rect, channels: bool) -> f32 {
    match mode {
        ShadingMode::Shaded => hud_rows(rect) * HUD_ROW_H + 6.0,
        ShadingMode::Channel if channels => CHANNEL_BAR_H + 6.0,
        _ => 0.0,
    }
}

/// The Shaded bar's rows: the background, the sky, the ground and the exposure, then the time and the weather, and a row more for what does not fit a viewport too narrow for them.
fn hud_rows(rect: egui::Rect) -> f32 {
    if rect.width() - 16.0 < HUD_W {
        3.0
    } else {
        2.0
    }
}

/// The mode a restart opens in: the one remembered, except that without a pack Shaded has nothing to draw with, so it opens in Material.
fn restored_mode(remembered: ShadingMode, game_pack: bool) -> ShadingMode {
    match remembered {
        ShadingMode::Shaded if !game_pack => ShadingMode::Material,
        other => other,
    }
}

/// What the Shaded pill says: what it draws with, and what came of the drawable on show.
fn shaded_tooltip(game_pack: bool, report: Option<&GameSceneReport>) -> String {
    if let Some((_, said)) = report.and_then(|report| report.gta6.as_ref()) {
        return said.clone();
    }
    if !game_pack {
        return "Shaded — the game's own shaders. Bake them first (Automate ▸ Bake game shaders); \
                until then it draws as Material"
            .to_string();
    }
    let mut text = "Shaded — the game's own passes, from the baked SPIR-V pack".to_string();
    if let Some(report) = report {
        text.push_str(&format!("\n{} surfaces drawn", report.drawn));
        if !report.left_out.is_empty() {
            text.push_str(&format!(", {} left out:", report.left_out.len()));
            for (effect, why) in report.left_out.iter().take(4) {
                text.push_str(&format!("\n  {effect}: {why}"));
            }
        }
    }
    text
}

/// The pills that pick the shading mode; the one clicked.
fn draw_shading_mode_pills(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    bridge: &mut RenderBridge,
) -> Option<ShadingMode> {
    const PILL_W: f32 = 28.0;
    const PILL_GAP: f32 = 2.0;
    const ROUNDING: f32 = 11.0;

    // ⚠ The channel pill is offered only when the asset has a channel to
    // show. A mode that renders a uniform mid-grey because nothing is there
    // looks like a broken viewport rather than an empty selection.
    let channel_pill = bridge.debug_channels_available();
    let mut pills: Vec<(ShadingMode, String)> = vec![
        (ShadingMode::Wireframe, "Wireframe".into()),
        (ShadingMode::Solid, "Solid".into()),
        (ShadingMode::Material, "Material".into()),
        (
            ShadingMode::Shaded,
            shaded_tooltip(bridge.game_shaders_available(), bridge.game_scene_report()),
        ),
    ];
    if channel_pill {
        pills.push((
            ShadingMode::Channel,
            "One vertex channel, unlit — pick which below".into(),
        ));
    }

    let total_w = PILL_W * pills.len() as f32 + PILL_GAP * (pills.len() as f32 - 1.0);
    let row_origin = egui::pos2(rect.right() - 8.0 - total_w, rect.top() + 8.0);
    let row_rect = egui::Rect::from_min_size(row_origin, egui::vec2(total_w, SHADING_PILL_H));

    ui.painter()
        .rect_filled(row_rect.expand(4.0), ROUNDING + 2.0, theme::overlay_bg());

    let active = bridge.mode();
    let mut chosen = None;
    for (i, (mode, tooltip)) in pills.iter().enumerate() {
        let x = row_origin.x + i as f32 * (PILL_W + PILL_GAP);
        let pill_rect = egui::Rect::from_min_size(
            egui::pos2(x, row_origin.y),
            egui::vec2(PILL_W, SHADING_PILL_H),
        );
        let id = ui.id().with(("shading_pill", *mode as u8));
        let resp = ui.interact(pill_rect, id, egui::Sense::click());

        let is_active = active == *mode;
        let bg = if is_active {
            theme::ACCENT.get()
        } else if resp.hovered() {
            theme::BG_L2.gamma_multiply(1.4)
        } else {
            theme::BG_L2.get()
        };
        let fg = if is_active {
            theme::ON_ACCENT.get()
        } else {
            theme::TEXT_PRIMARY.get()
        };

        ui.painter().rect_filled(pill_rect, ROUNDING, bg);
        if !is_active {
            ui.painter().rect_stroke(
                pill_rect,
                ROUNDING,
                egui::Stroke::new(1.0_f32, theme::BORDER),
                egui::StrokeKind::Inside,
            );
        }

        draw_shading_icon(ui.painter(), pill_rect.center(), *mode, fg);

        if resp.clicked() {
            bridge.set_mode(*mode);
            chosen = Some(*mode);
        }
        let _ = resp.on_hover_text(tooltip.as_str());
    }
    chosen
}

/// The channel selector, shown only while the `Channel` mode is on.
pub(crate) fn draw_channel_selector(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    top: f32,
    offered: &[crate::render::debug_channel::DebugChannel],
    state: &mut AppState,
) {
    if offered.is_empty() {
        return;
    }
    // Default to the first channel the file has rather than to a fixed one,
    // so selecting the mode always shows something.
    if state
        .preview_scene
        .debug_channel
        .is_none_or(|c| !offered.contains(&c))
    {
        state.preview_scene.debug_channel = Some(offered[0]);
    }
    let current = state.preview_scene.debug_channel;

    let width = 200.0_f32.min(rect.width() - 16.0);
    let origin = egui::pos2(rect.right() - 8.0 - width, top);
    let bar = egui::Rect::from_min_size(origin, egui::vec2(width, CHANNEL_BAR_H));
    ui.painter()
        .rect_filled(bar.expand(4.0), theme::RADIUS_LARGE, theme::overlay_bg());

    let mut chosen = current;
    ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("viewport_channel")
                .width(width - 8.0)
                .selected_text(current.map(|c| c.label()).unwrap_or_default())
                .show_ui(ui, |ui| {
                    for channel in offered {
                        ui.selectable_value(&mut chosen, Some(*channel), channel.label())
                            .on_hover_text(channel.legend());
                    }
                });
        });
    });

    if chosen != current {
        state.preview_scene.debug_channel = chosen;
    }
}

pub(crate) fn draw_shaded_hud_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    top: f32,
    state: &mut AppState,
) {
    let narrow = rect.width() - 16.0 < HUD_W;
    let width = HUD_W.min(rect.width() - 16.0);
    let bar_origin = egui::pos2(rect.right() - 8.0 - width, top);
    let bar_rect =
        egui::Rect::from_min_size(bar_origin, egui::vec2(width, hud_rows(rect) * HUD_ROW_H));
    ui.painter().rect_filled(
        bar_rect.expand(4.0),
        theme::RADIUS_LARGE,
        theme::overlay_bg(),
    );
    let mut inner_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar_rect.shrink2(egui::vec2(8.0, 4.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    inner_ui.style_mut().spacing.item_spacing.x = 6.0;
    inner_ui.style_mut().spacing.item_spacing.y = 2.0;
    inner_ui.horizontal_wrapped(|ui| {
        ui.label("Background");
        edit_rgba(ui, &mut state.drivers.world_background.colour);
        crate::ui::accessible_name::named(
            ui.checkbox(&mut state.drivers.world_background.sky, "Sky")
                .on_hover_text("Draw the game's sky of the time of day where nothing else is, in place of the background colour"),
            "Sky background",
        );
        crate::ui::accessible_name::named(
            ui.checkbox(&mut state.drivers.world_background.ground, "Ground")
                .on_hover_text("Lay a ground under a model or a particle effect, for its shadow and its lights to land on"),
            "Ground under a model or an effect",
        );
        ui.add_space(theme::SPACING_DEFAULT);
        ui.label("Exp");
        let mut exp = state.drivers.exposure.value;
        let idle = super::timeline::holds_unsimulated_effects(state);
        let drag = egui::DragValue::new(&mut exp).speed(0.01).range(0.0..=1.0);
        let resp = ui.add_enabled(!idle, drag);
        let resp = crate::ui::accessible_name::named(resp, "Exposure")
            .on_disabled_hover_text(crate::i18n::tr(super::timeline::NOT_SIMULATED));
        if resp.changed() {
            state.drivers.exposure.set_value(exp);
        }
    });
    // The light of the install's timecycle at an hour and in a weather, which Shaded lights the drawable by
    inner_ui.horizontal_wrapped(|ui| {
        ui.label("Time of day");
        let mut hour = state.drivers.time_of_day.hour;
        let resp = ui.add(
            egui::DragValue::new(&mut hour)
                .speed(0.05)
                .range(0.0..=24.0)
                .fixed_decimals(2)
                .suffix("h"),
        );
        if crate::ui::accessible_name::named(resp, "Time of day").changed() {
            state.drivers.time_of_day.set_hour(hour);
        }
        let playing = state.drivers.time_of_day.playing;
        let play = ui
            .selectable_label(playing, if playing { "⏸" } else { "▶" })
            .on_hover_text(
                "Run the day, an hour a second: the sky, the moon and the clouds follow",
            );
        if crate::ui::accessible_name::named(play, "Run the day").clicked() {
            state.drivers.time_of_day.playing = !playing;
        }
        if playing {
            state.drivers.time_of_day.advance(ui.input(|i| i.stable_dt));
            ui.ctx().request_repaint();
        }
        // In a viewport too narrow for both, the weather has a row of its own
        if narrow {
            ui.end_row();
        } else {
            ui.add_space(theme::SPACING_DEFAULT);
        }
        ui.label("Weather");
        let current = state.drivers.weather.state;
        let chosen = egui::ComboBox::from_id_salt("shaded_weather")
            .selected_text(current.label())
            .show_ui(ui, |ui| {
                let mut chosen = current;
                for weather in crate::state::drivers::WeatherState::ALL {
                    if ui
                        .selectable_label(weather == current, weather.label())
                        .clicked()
                    {
                        chosen = weather;
                    }
                }
                chosen
            });
        crate::ui::accessible_name::named(chosen.response.clone(), "Weather");
        if let Some(weather) = chosen.inner.filter(|w| *w != current) {
            state.set_weather_state(weather);
        }
    });
}

fn edit_rgba(ui: &mut egui::Ui, colour: &mut [f32; 4]) {
    let [r, g, b, a] = *colour;
    let mut rgba = egui::Rgba::from_rgba_premultiplied(r, g, b, a);
    let picked = egui::color_picker::color_edit_button_rgba(
        ui,
        &mut rgba,
        egui::color_picker::Alpha::Opaque,
    );
    if picked.changed() {
        *colour = rgba.to_array();
    }
}

fn draw_shading_icon(
    painter: &egui::Painter,
    centre: egui::Pos2,
    mode: ShadingMode,
    fg: egui::Color32,
) {
    let r = 6.5;
    let stroke = egui::Stroke::new(1.4_f32, fg);
    let fill = egui::Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 200);
    let dim = egui::Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 90);

    match mode {
        ShadingMode::Wireframe => {
            painter.circle_stroke(centre, r, stroke);
            painter.line_segment(
                [
                    centre + egui::vec2(-r + 0.5, 0.0),
                    centre + egui::vec2(r - 0.5, 0.0),
                ],
                egui::Stroke::new(1.0_f32, fg),
            );
        }
        ShadingMode::Solid => {
            painter.circle_filled(centre, r, fill);
        }
        ShadingMode::Material => {
            painter.circle_filled(centre, r, dim);
            painter.circle_stroke(centre, r, egui::Stroke::new(1.0_f32, fg));
            let steps = 14;
            for i in 0..steps {
                let t = (i as f32 + 0.5) / steps as f32;
                let y = -r + t * 2.0 * r;
                let half_chord = (r * r - y * y).max(0.0).sqrt();
                painter.line_segment(
                    [
                        egui::pos2(centre.x, centre.y + y),
                        egui::pos2(centre.x + half_chord, centre.y + y),
                    ],
                    egui::Stroke::new(1.5_f32, fill),
                );
            }
        }
        ShadingMode::Shaded => {
            // A lit sphere with the shaders' own sheen: a ring round a disc, and a highlight
            painter.circle_stroke(centre, r, stroke);
            painter.circle_filled(centre, r * 0.6, fill);
            painter.circle_filled(
                centre + egui::vec2(-r * 0.25, -r * 0.25),
                r * 0.22,
                egui::Color32::WHITE,
            );
        }
        ShadingMode::Channel => {
            // Three quadrants in the channel legend's own colours, so the
            // pill says which mapping the mode uses.
            painter.circle_stroke(centre, r, egui::Stroke::new(1.0_f32, fg));
            for (offset, colour) in [
                (
                    egui::vec2(-r * 0.35, -r * 0.3),
                    egui::Color32::from_rgb(200, 60, 60),
                ),
                (
                    egui::vec2(r * 0.35, -r * 0.3),
                    egui::Color32::from_rgb(60, 190, 80),
                ),
                (
                    egui::vec2(0.0, r * 0.4),
                    egui::Color32::from_rgb(70, 110, 220),
                ),
            ] {
                painter.circle_filled(centre + offset, r * 0.33, colour);
            }
        }
    }
}

/// The box at the viewport's bottom left, kept clear of `clear_of`.
pub(crate) fn draw_viewport_overlay(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    state: &mut AppState,
    retarget: Option<&or_anim::PaletteRetarget>,
    shaded: Option<&str>,
    clear_of: egui::Rect,
) {
    let viewport_layout::Overlay { text, picked } =
        viewport_layout::overlay(state, retarget, shaded);
    // The box is sized from its text, which wraps within 8 points of the
    // viewport's edges, or of what the box keeps clear of, and 8 of the box's.
    let padding = egui::vec2(8.0, 6.0);
    let left = rect.left() + 8.0;
    let lay_out = |right: f32| {
        let galley = ui.painter().layout(
            text.clone(),
            egui::FontId::monospace(11.0),
            theme::TEXT_SECONDARY.get(),
            (right - left - 2.0 * padding.x).max(0.0),
        );
        let size = galley.size() + 2.0 * padding;
        let frame = egui::Rect::from_min_size(egui::pos2(left, rect.bottom() - 8.0 - size.y), size);
        (frame, galley)
    };
    let (mut overlay_rect, mut galley) = lay_out(rect.right() - 8.0);
    if overlay_rect.intersects(clear_of) {
        (overlay_rect, galley) = lay_out(clear_of.left() - 8.0);
    }
    ui.painter()
        .rect_filled(overlay_rect, theme::RADIUS_SMALL, theme::overlay_bg());
    let origin = overlay_rect.min + padding;
    let line = picked.and_then(|line| line_rect(&galley, line));
    ui.painter()
        .galley(origin, galley, theme::TEXT_SECONDARY.get());
    // A double click on the line that names a pick, or Enter or Space on it once it has the focus, opens it where it is, since the Outline there
    // follows the clip dictionary.
    if let Some((line, said)) = line.zip(picked.and_then(|line| text.lines().nth(line))) {
        let response = ui.interact(
            line.translate(origin.to_vec2()),
            picked_line_id(),
            egui::Sense::click(),
        );
        // The box is painted text, so the line is named by what it says and what it does.
        const OPENS: &str =
            "Double-click, or press Enter, to open it in its own document, at its row";
        let response = crate::ui::accessible_name::named(response, &format!("{said}. {OPENS}"))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(OPENS);
        let by_key = response.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space));
        if response.double_clicked() || by_key {
            crate::render::viewport::pick::open_picked(state);
        }
    }
}

/// The widget of the box's Picked line, which a double click opens.
fn picked_line_id() -> egui::Id {
    egui::Id::new("viewport-picked-line")
}

/// Where line `line` of `galley` lies in it, counting lines by the text's own breaks.
fn line_rect(galley: &egui::Galley, line: usize) -> Option<egui::Rect> {
    let mut at = 0;
    let mut found: Option<egui::Rect> = None;
    for row in &galley.rows {
        if at == line {
            found = Some(found.map_or(row.rect(), |seen| seen.union(row.rect())));
        }
        if row.ends_with_newline {
            at += 1;
        }
    }
    found
}

/// The letters on a gizmo bubble: black reads on each of the axis colours at 4.5:1, and white does not on the green.
const GIZMO_LABEL: egui::Color32 = egui::Color32::BLACK;

/// What the gizmo's disc is filled with, over the viewport: black, which the axis colours read on.
const GIZMO_DISC: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 110);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum AxisView {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl AxisView {
    fn yaw_pitch(self) -> (f32, f32) {
        const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;
        const PI: f32 = std::f32::consts::PI;
        const POLE: f32 = HALF_PI - 0.01;
        match self {
            AxisView::PosX => (0.0, 0.0),
            AxisView::NegX => (PI, 0.0),
            AxisView::PosY => (HALF_PI, 0.0),
            AxisView::NegY => (-HALF_PI, 0.0),
            AxisView::PosZ => (0.0, POLE),
            AxisView::NegZ => (0.0, -POLE),
        }
    }

    fn label(self) -> &'static str {
        match self {
            AxisView::PosX => "X",
            AxisView::NegX => "-X",
            AxisView::PosY => "Y",
            AxisView::NegY => "-Y",
            AxisView::PosZ => "Z",
            AxisView::NegZ => "-Z",
        }
    }

    fn is_positive(self) -> bool {
        matches!(self, AxisView::PosX | AxisView::PosY | AxisView::PosZ)
    }

    fn world_dir(self) -> Vec3 {
        match self {
            AxisView::PosX => Vec3::X,
            AxisView::NegX => -Vec3::X,
            AxisView::PosY => Vec3::Y,
            AxisView::NegY => -Vec3::Y,
            AxisView::PosZ => Vec3::Z,
            AxisView::NegZ => -Vec3::Z,
        }
    }

    fn colour(self) -> egui::Color32 {
        match self {
            AxisView::PosX | AxisView::NegX => egui::Color32::from_rgb(0xf2, 0x4c, 0x4c),
            AxisView::PosY | AxisView::NegY => egui::Color32::from_rgb(0x4d, 0xd9, 0x6c),
            AxisView::PosZ | AxisView::NegZ => egui::Color32::from_rgb(0x55, 0x99, 0xff),
        }
    }
}

/// Whether a letter is drawn on `view`'s bubble: the positive axes always, a negative one under the pointer.
fn has_letter(view: AxisView, hovered: bool) -> bool {
    view.is_positive() || hovered
}

/// ⚠ Opaque wherever a letter is drawn on it, a bubble behind the others too: the letter reads on the axis colour, not on what a translucent fill lets through from the scene.
fn bubble_fill(view: AxisView, hovered: bool) -> egui::Color32 {
    if has_letter(view, hovered) {
        view.colour()
    } else {
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 140)
    }
}

fn draw_axis_gizmo(ui: &mut egui::Ui, gizmo_rect: egui::Rect, bridge: &mut RenderBridge) {
    let centre = gizmo_rect.center();
    let size = gizmo_rect.width();

    let bg_resp = ui.interact(
        gizmo_rect,
        ui.id().with("viewport_gizmo_bg"),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter();
    painter.circle_filled(centre, size * 0.5, GIZMO_DISC);

    let camera = bridge.camera();
    let cp = camera.pitch.cos();
    let sp = camera.pitch.sin();
    let cy = camera.yaw.cos();
    let sy = camera.yaw.sin();
    let z_view = Vec3::new(cp * cy, cp * sy, sp);
    let x_view = OrbitCamera::WORLD_UP.cross(z_view).normalize_or_zero();
    let y_view = z_view.cross(x_view);

    let len = size * 0.36;
    let project = |w: Vec3| -> (egui::Vec2, f32) {
        let sx = w.dot(x_view);
        let sy = w.dot(y_view);
        let sz = w.dot(z_view);
        (egui::vec2(sx * len, -sy * len), sz)
    };

    let bubble_r = 9.0;
    let ring_r = 7.0;

    let views = [
        AxisView::PosX,
        AxisView::NegX,
        AxisView::PosY,
        AxisView::NegY,
        AxisView::PosZ,
        AxisView::NegZ,
    ];
    let mut endpoints: Vec<(AxisView, egui::Pos2, f32)> = views
        .iter()
        .map(|&v| {
            let (offset, depth) = project(v.world_dir());
            (v, centre + offset, depth)
        })
        .collect();
    endpoints.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    for (view, end, depth) in &endpoints {
        if !view.is_positive() {
            continue;
        }
        let colour = view.colour();
        let stroke = if *depth < 0.0 {
            egui::Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 110)
        } else {
            colour
        };
        ui.painter()
            .line_segment([centre, *end], egui::Stroke::new(2.0_f32, stroke));
    }

    let mut clicked: Option<AxisView> = None;
    let mut hovered: Option<AxisView> = None;
    let mut hover_tooltip: Option<(AxisView, egui::Response)> = None;
    for (view, end, depth) in &endpoints {
        let r = if view.is_positive() { bubble_r } else { ring_r };
        let bubble_rect = egui::Rect::from_center_size(*end, egui::vec2(r * 2.4, r * 2.4));
        let id = ui.id().with(("viewport_gizmo_bubble", *view as u8));
        let resp = ui.interact(bubble_rect, id, egui::Sense::click());
        if resp.clicked() {
            clicked = Some(*view);
        }
        if resp.hovered() {
            hovered = Some(*view);
            hover_tooltip = Some((*view, resp));
        }

        let is_hovered = hovered == Some(*view);
        let behind = *depth < -0.05;
        let alpha: u8 = if behind { 150 } else { 255 };
        let colour = view.colour();
        let painter = ui.painter();

        // A bubble behind the others is smaller, as it is farther.
        let radius = match (is_hovered, behind) {
            (true, _) => r + 1.5,
            (false, true) => r - 1.5,
            (false, false) => r,
        };
        painter.circle_filled(*end, radius, bubble_fill(*view, is_hovered));
        if !view.is_positive() {
            let edge =
                egui::Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), alpha);
            painter.circle_stroke(*end, radius, egui::Stroke::new(1.6_f32, edge));
        }
        if has_letter(*view, is_hovered) {
            let size = if view.is_positive() {
                11.0
            } else {
                theme::SMALL_TEXT
            };
            painter.text(
                *end,
                egui::Align2::CENTER_CENTER,
                view.label(),
                egui::FontId::proportional(size),
                GIZMO_LABEL,
            );
        }
    }

    if let Some((view, resp)) = hover_tooltip {
        let _ = resp.on_hover_text(format!(
            "Click: align view to {}\nDrag the gizmo center: rotate the view",
            view.label()
        ));
    }

    if let Some(view) = clicked {
        bridge.push_inputs(CameraInputs {
            view_angles: Some(view.yaw_pitch()),
            ..CameraInputs::default()
        });
    } else if bg_resp.dragged() {
        bridge.push_inputs(CameraInputs {
            orbit: bg_resp.drag_delta(),
            ..CameraInputs::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::GameSceneReport;
    use or_renderer::ShadingMode;

    #[test]
    fn a_restart_without_the_pack_does_not_open_in_the_mode_that_needs_it() {
        assert_eq!(
            super::restored_mode(ShadingMode::Shaded, false),
            ShadingMode::Material
        );
        assert_eq!(
            super::restored_mode(ShadingMode::Shaded, true),
            ShadingMode::Shaded
        );
        for mode in [
            ShadingMode::Wireframe,
            ShadingMode::Solid,
            ShadingMode::Material,
            ShadingMode::Channel,
        ] {
            assert_eq!(super::restored_mode(mode, false), mode);
        }
    }

    #[test]
    fn the_shaded_pill_says_what_the_drawable_on_show_came_to() {
        assert_eq!(
            super::shaded_tooltip(true, None),
            "Shaded — the game's own passes, from the baked SPIR-V pack"
        );
        let report = GameSceneReport {
            drawn: 3,
            left_out: (0..6)
                .map(|i| (format!("effect{i}"), "no pass".to_string()))
                .collect(),
            gta6: None,
        };
        let text = super::shaded_tooltip(true, Some(&report));
        assert!(text.contains("\n3 surfaces drawn, 6 left out:"), "{text}");
        assert!(text.contains("effect0: no pass") && text.contains("effect3: no pass"));
        assert!(
            !text.contains("effect4"),
            "the first four are listed: {text}"
        );
        let all_drawn = GameSceneReport {
            drawn: 2,
            left_out: Vec::new(),
            gta6: None,
        };
        assert!(super::shaded_tooltip(true, Some(&all_drawn)).ends_with("\n2 surfaces drawn"));
        // A GTA VI drawable's pill says what its own programs came to, whether or not a pack is baked.
        let gta6 = GameSceneReport {
            gta6: Some((false, "what the programs came to".to_string())),
            ..all_drawn
        };
        for baked in [false, true] {
            assert_eq!(
                super::shaded_tooltip(baked, Some(&gta6)),
                "what the programs came to"
            );
        }
    }

    use super::*;
    use crate::state::PersistedBinding;

    /// A letter is judged on the fill it is drawn on, under the pointer or not.
    #[test]
    fn the_gizmos_letters_read_on_every_axis_colour() {
        use AxisView::*;
        for view in [PosX, NegX, PosY, NegY, PosZ, NegZ] {
            for hovered in [false, true] {
                if !has_letter(view, hovered) {
                    continue;
                }
                let fill = bubble_fill(view, hovered);
                let state = format!("{} hovered {hovered}", view.label());
                assert_eq!(
                    fill.a(),
                    255,
                    "{state}: what is behind the bubble shows through"
                );
                let ratio = theme::contrast(GIZMO_LABEL, fill);
                assert!(ratio >= theme::AA, "{state}: reads at {ratio:.2}:1");
            }
        }
    }

    /// ⚠ Judged at full opacity on the viewport's default background, which the user can change; a bubble or an axis behind the others fades on purpose, and is smaller too.
    #[test]
    fn the_axis_colours_read_at_3_to_1_on_the_gizmos_disc() {
        use AxisView::*;
        let background = egui::Color32::from_rgb(0x2d, 0x2d, 0x30);
        let kept = 1.0 - f32::from(GIZMO_DISC.a()) / 255.0;
        let disc =
            egui::Color32::from(egui::Rgba::from(GIZMO_DISC) + egui::Rgba::from(background) * kept);
        for view in [PosX, NegX, PosY, NegY, PosZ, NegZ] {
            let ratio = theme::contrast(view.colour(), disc);
            assert!(
                ratio >= theme::GRAPHIC,
                "{}: {:?} reads at {ratio:.2}:1 on {disc:?}",
                view.label(),
                view.colour()
            );
        }
    }

    fn frames_on(key: egui::Key, prefs: &Prefs) -> Option<Action> {
        let ctx = egui::Context::default();
        let press = egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput {
            events: vec![press],
            ..Default::default()
        };
        let mut framed = None;
        let _ = ctx.run_ui(input, |ui| {
            framed = ui.input(|i| framing_requested(i, prefs))
        });
        framed
    }

    #[test]
    fn frame_all_frames_on_the_key_it_is_rebound_to() {
        let mut prefs = Prefs::default();
        assert_eq!(frames_on(egui::Key::Home, &prefs), Some(Action::FrameAll));
        prefs.shortcut_bindings.insert(
            "frame_all".to_string(),
            PersistedBinding {
                control: false,
                ctrl: false,
                shift: false,
                alt: false,
                key: "KeyG".to_string(),
            },
        );
        assert_eq!(frames_on(egui::Key::G, &prefs), Some(Action::FrameAll));
        assert_eq!(frames_on(egui::Key::Home, &prefs), None);
        for key in [egui::Key::F, egui::Key::Period] {
            assert_eq!(
                frames_on(key, &prefs),
                Some(Action::FrameSelection),
                "Frame selection keeps {key:?}"
            );
        }
    }

    #[test]
    fn the_overlay_names_the_framing_keys_as_bound() {
        let mut prefs = Prefs::default();
        assert_eq!(
            framing_hint(&prefs),
            "F or . frames the selection · Home frames everything"
        );
        let bound = |key: &str| PersistedBinding {
            control: false,
            ctrl: false,
            shift: false,
            alt: false,
            key: key.to_string(),
        };
        prefs
            .shortcut_bindings
            .insert("frame_all".to_string(), bound("KeyG"));
        prefs
            .shortcut_bindings
            .insert("frame_selection".to_string(), bound("__none__"));
        assert_eq!(framing_hint(&prefs), "G frames everything");
    }
}
