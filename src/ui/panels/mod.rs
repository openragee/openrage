pub mod about;
pub mod adopt_banner;
pub mod batch;
pub mod command_log;
pub mod command_palette;
pub mod compare;
mod dlc_pack;
pub mod document_tabs;
pub mod document_view;
pub mod edit_mode;
pub mod empty_state;
pub mod file_preview;
pub mod file_prompts;
pub mod gallery;
pub mod game_picker;
pub mod gta6_anim_scene;
pub mod guide;
pub mod gxt2_table;
pub mod hierarchy;
pub mod history;
pub mod import_report;
pub mod inspector;
pub mod jobs;
pub mod log;
pub mod maps;
pub mod menu;
pub mod meta_editor;
pub mod mods;
pub mod new_project;
pub mod overlay_copy;
pub mod particles;
pub mod plugin_panel;
pub mod preview_settings;
pub mod project;
pub mod rail;
pub mod registry;
pub mod reopen_report;
pub mod save_destination;
pub mod search;
pub mod selection_ids;
pub mod settings;
pub mod settings_appearance;
pub mod settings_associations;
pub mod settings_editing;
pub mod settings_games;
pub mod settings_launch;
pub mod settings_materials;
pub mod settings_overlay;
pub mod settings_overlay_loader;
pub mod settings_plugins;
pub mod settings_shortcuts;
pub mod settings_window;
pub mod start_page;
pub mod statistics;
pub mod status_bar;
pub mod text_editor;
pub mod texture_preview;
pub mod timeline;
pub mod timeline_ycd;
pub mod tour;
pub mod viewport;
pub mod viewport_drawable_row;
pub mod viewport_layout;
pub mod viewport_texture_badge;
pub mod wardrobe;
pub mod weapons;
pub mod world_viewer;
mod world_viewer_map;
mod world_viewer_places;
pub mod world_viewer_prefs;
mod world_viewer_side;
pub mod ycd_clips;

/// A question titled `title` that the shell waits on: over a backdrop that takes every click until it is answered.
///
/// ⚠ egui leaves the layers behind a modal inert from the next frame, and
/// [`question_waits`] reads the same flag for the keys the app takes before
/// egui sees them.
pub fn question<R>(
    ctx: &egui::Context,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    asked(ctx, title, title, None, add_contents)
}

/// As [`question`], `width` wide, or as wide as the window leaves it when that is less.
///
/// ⚠ The width is set before the heading: a modal keeps the width of its last
/// frame, and the separator under the heading spans it, so a width set after
/// would not narrow the question when the window did.
pub fn question_within<R>(
    ctx: &egui::Context,
    title: &str,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    // The frame's margins and a little air.
    const WINDOW_EDGES: f32 = 32.0;
    asked(
        ctx,
        title,
        title,
        Some(width.min(ctx.content_rect().width() - WINDOW_EDGES)),
        add_contents,
    )
}

/// As [`question_within`] for a question that keeps its identity while its heading changes: `key` names it, here and for [`question_escaped`].
///
/// ⚠ A modal that changed its identity with its heading would be a new window of another size.
pub fn question_titled<R>(
    ctx: &egui::Context,
    key: &str,
    title: &str,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    // The frame's margins and a little air.
    const WINDOW_EDGES: f32 = 32.0;
    asked(
        ctx,
        key,
        title,
        Some(width.min(ctx.content_rect().width() - WINDOW_EDGES)),
        add_contents,
    )
}

fn asked<R>(
    ctx: &egui::Context,
    key: &str,
    title: &str,
    width: Option<f32>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let shown = egui::Modal::new(egui::Id::new(("question", key)))
        .frame(egui::Frame::window(&ctx.global_style()))
        .show(ctx, |ui| {
            if let Some(width) = width {
                ui.set_width(width);
            }
            ui.heading(title);
            ui.separator();
            add_contents(ui)
        });
    let answerable = shown.is_top_modal && !shown.any_popup_open;
    ctx.data_mut(|data| data.insert_temp(escape_id(key), answerable));
    shown.inner
}

fn escape_id(title: &str) -> egui::Id {
    egui::Id::new(("question_escape", title))
}

/// Whether Escape closes the question titled `title` that was just drawn: read it right after [`question`] and answer as Cancel does.
///
/// ⚠ Not while another question is over it or a popup of its own (a choice's list) is open, where Escape
/// belongs to that; and the key is taken only here, so a question that never asks keeps it.
pub fn question_escaped(ctx: &egui::Context, title: &str) -> bool {
    let answerable = ctx.data(|data| data.get_temp::<bool>(escape_id(title)));
    answerable == Some(true)
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

/// Whether a question waited on screen in the last frame drawn.
pub fn question_waits(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.top_modal_layer().is_some())
}

/// A shader's effect name from its hash, GTA V's or a GTA VI effect the app has read, or the hash
/// as `shader_XXXXXXXX` when no name is known.
pub fn effect_name(hash: u32) -> String {
    match or_io::gta5::shaders::shader_names::lookup(hash) {
        Some(name) => name.to_string(),
        None => crate::state::gta6_resource::known_effect(hash)
            .unwrap_or_else(|| format!("shader_{hash:08X}")),
    }
}

/// A `DragValue` over `value`, whose `inner` says whether the user changed it.
pub fn drag_value<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut N,
    style: impl FnOnce(egui::DragValue<'_>) -> egui::DragValue<'_>,
) -> egui::InnerResponse<bool> {
    // ⚠ The response's own `changed` is not that: with no input, egui turns a
    // NaN into an infinity and clamps a value past the range into it, and
    // reports either as a change. Files hold both, so trusting it edits a
    // document by drawing it.
    let mut shown = *value;
    let response = ui.add(style(egui::DragValue::new(&mut shown)).clamp_existing_to_range(false));
    let edited = shown != *value && !(shown.to_f64().is_nan() && value.to_f64().is_nan());
    if edited {
        *value = shown;
    }
    egui::InnerResponse::new(edited, response)
}
