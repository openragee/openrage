#[cfg(test)]
mod a_drop_means_what_the_screen_under_it_says;
#[cfg(test)]
mod a_second_task_screen_is_one_row;
#[cfg(test)]
mod a_weapon_of_the_install_is_drawn_on_screen;
pub mod accessible_name;
pub mod branding;
pub mod commands;
pub mod dock;
pub mod dock_commands;
pub mod dock_layout;
#[cfg(test)]
mod every_context_menu_draws_from_the_registry;
#[cfg(test)]
mod every_control_has_a_name_a_screen_reader_can_speak;
#[cfg(test)]
mod every_control_of_the_project_screens_has_a_reason;
#[cfg(test)]
mod every_hint_names_the_current_binding;
#[cfg(test)]
mod every_kind_of_icon_has_a_shape_of_its_own;
#[cfg(test)]
mod every_menu_item_is_in_the_palette;
#[cfg(test)]
mod every_tab_in_the_dock_is_a_button_named_after_its_panel;
#[cfg(test)]
mod every_tab_stop_in_the_dock_has_a_name;
#[cfg(test)]
mod every_text_colour_reads_at_4_5_to_1;
pub mod focus_ring;
#[cfg(test)]
mod frames_of_the_shell;
pub mod guide_links;
pub mod icons;
#[cfg(test)]
mod no_new_colour_is_written_outside_the_theme;
#[cfg(test)]
mod no_site_matches_on_the_kind_of_a_task_screen;
#[cfg(test)]
mod no_text_the_shell_paints_reads_under_4_5_to_1;
#[cfg(test)]
mod open_in_offers_every_workspace_and_says_why_one_shows_nothing;
pub mod os_drop;
pub mod panel_registry;
pub mod panel_state;
pub mod panels;
#[cfg(test)]
mod reopening_a_built_pack_on_screen;
pub mod root;
pub mod shortcuts;
pub mod task_screens;
#[cfg(test)]
mod the_addon_peds_on_screen;
#[cfg(test)]
mod the_build_tab_on_screen;
#[cfg(test)]
mod the_build_tab_says_what_was_seen_in_game_on_screen;
#[cfg(test)]
mod the_builds_read_plainly_on_screen;
#[cfg(test)]
mod the_builds_say_what_changed_and_what_they_checked_on_screen;
#[cfg(test)]
mod the_builds_window_as_a_person_meets_it;
#[cfg(test)]
mod the_counts_and_names_of_the_project_as_a_person_meets_them;
#[cfg(test)]
mod the_counts_of_only_one_type_read_nothing_until_clicked;
#[cfg(test)]
mod the_dressed_ped_layout_on_screen;
#[cfg(test)]
mod the_dressed_ped_tools_on_screen;
#[cfg(test)]
mod the_dressed_ped_views_and_texture_places_on_screen;
#[cfg(test)]
mod the_error_and_close_questions_say_what_they_act_on_on_screen;
#[cfg(test)]
mod the_files_tree_draws_only_the_rows_in_view;
#[cfg(test)]
mod the_front_door_dialogs_on_screen;
#[cfg(test)]
mod the_front_door_names_each_thing_once_on_screen;
#[cfg(test)]
mod the_front_door_on_screen;
#[cfg(test)]
mod the_front_door_reads_plainly_on_screen;
#[cfg(test)]
mod the_guide_on_screen;
#[cfg(test)]
mod the_history_on_screen;
#[cfg(test)]
mod the_import_screens_as_a_person_meets_them;
#[cfg(test)]
mod the_keyboard_is_always_seen;
#[cfg(test)]
mod the_languages_on_screen;
#[cfg(test)]
mod the_maps_screen_on_screen;
#[cfg(test)]
mod the_menus_and_onboarding_read_plainly_on_screen;
#[cfg(test)]
mod the_palette_ranks_fuzzy_and_takes_the_arrow_keys;
#[cfg(test)]
mod the_particle_screen_on_screen;
#[cfg(test)]
mod the_picker_on_screen;
#[cfg(test)]
mod the_power_features_on_screen;
#[cfg(test)]
mod the_power_windows_in_plain_words_on_screen;
#[cfg(test)]
mod the_project_panels_on_screen;
#[cfg(test)]
mod the_project_polish_on_screen;
#[cfg(test)]
mod the_project_screens_as_a_person_meets_them;
#[cfg(test)]
mod the_project_screens_on_screen;
#[cfg(test)]
mod the_task_screens_draw_as_they_did_before_the_table;
#[cfg(test)]
mod the_timeline_describes_what_it_paints_by_hand;
#[cfg(test)]
mod the_top_flows_end_to_end;
#[cfg(test)]
mod the_user_flows_end_to_end;
#[cfg(test)]
mod the_wardrobe_behaviour_on_screen;
#[cfg(test)]
mod the_wardrobe_edits_wind_and_shop_entries_on_screen;
#[cfg(test)]
mod the_wardrobe_import_breadth_on_screen;
#[cfg(test)]
mod the_wardrobe_list_and_its_header_as_a_person_meets_them;
#[cfg(test)]
mod the_wardrobe_list_ergonomics_on_screen;
#[cfg(test)]
mod the_wardrobe_list_keeps_its_room_on_screen;
#[cfg(test)]
mod the_wardrobe_on_screen;
#[cfg(test)]
mod the_wardrobe_operations_on_screen;
#[cfg(test)]
mod the_wardrobe_recovery_on_screen;
#[cfg(test)]
mod the_wardrobe_says_what_it_acts_on_on_screen;
#[cfg(test)]
mod the_weapons_screen_on_screen;
#[cfg(test)]
mod the_window_sends_the_tree_a_screen_reader_reads;
pub mod theme;
pub mod timeline;
#[cfg(test)]
mod tooltips_name_the_guide_section_that_explains_the_control;
pub mod virtual_rows;

use crate::render::RenderBridge;
use crate::state::AppState;

pub use dock::Workspace;

/// Whom the popup open now belongs to, as its contents say while they draw.
#[derive(Clone, Copy)]
enum Owner {
    /// A bar's menu, which no document owns.
    Shell,
    Document(crate::state::DocumentId),
}

const OWNER: &str = "the_owner_of_the_open_popup";

fn note_owner(ctx: &egui::Context, owner: Owner) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(OWNER), owner));
}

/// Called from the contents of a menu of the shell's own bars: a change of document leaves it open.
pub fn popup_of_the_shell(ctx: &egui::Context) {
    note_owner(ctx, Owner::Shell);
}

/// How wide a menu may be in English: egui's own.
const ENGLISH_MENU_WIDTH: f32 = 150.0;

/// A menu is as wide as its widest row, up to a limit: the rows of any other language than English are longer than the English ones.
fn room_for_the_words_of_the_language(ctx: &egui::Context) {
    let wanted = if crate::i18n::active().is_english() {
        ENGLISH_MENU_WIDTH
    } else {
        ENGLISH_MENU_WIDTH * 1.5
    };
    if ctx.global_style().spacing.menu_width != wanted {
        ctx.global_style_mut(|style| style.spacing.menu_width = wanted);
    }
}

/// Called from the contents of a menu acting on `doc`: a change of document closes it only when `doc` is the one left.
pub fn popup_of_a_document(ctx: &egui::Context, doc: crate::state::DocumentId) {
    note_owner(ctx, Owner::Document(doc));
}

/// Closes a list or a menu open over a document that is no longer the active one: it was opened for that document's controls.
///
/// ⚠ A popup whose contents named their owner stays open unless the owner is the document left; every
/// other one is a list in a document's panel, which acts on the active document it was drawn for.
fn close_lists_over_another_document(ctx: &egui::Context, state: &AppState) {
    let seen = egui::Id::new("the_document_under_the_open_lists");
    let before = ctx.data(|d| d.get_temp::<Option<crate::state::DocumentId>>(seen));
    // Noted again by the contents of the popup, if it is still open, as the frame draws.
    let owner = ctx.data_mut(|d| {
        let owner = d.get_temp::<Owner>(egui::Id::new(OWNER));
        d.remove::<Owner>(egui::Id::new(OWNER));
        owner
    });
    if let Some(left) = before.filter(|before| *before != state.active_document) {
        let kept = match owner {
            Some(Owner::Shell) => true,
            Some(Owner::Document(doc)) => Some(doc) != left,
            None => false,
        };
        if !kept {
            egui::Popup::close_all(ctx);
        }
    }
    ctx.data_mut(|d| d.insert_temp(seen, state.active_document));
}

pub fn draw(ctx: &egui::Context, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    room_for_the_words_of_the_language(ctx);
    close_lists_over_another_document(ctx, state);
    panels::text_editor::new_frame(state);
    // Order is layout: each panel takes its edge from what the ones above it
    // left, and the dock fills the rest.
    root::with_root_ui(ctx, |ui| {
        panels::menu::draw(ui, state);
        panels::status_bar::draw(ui, state);
        // ⚠ After the bars so the rail stands between them, and before the tabs so they start at its right.
        panels::rail::draw(ui, state);
        panels::document_tabs::draw(ui, state);
        dock::draw(ui, state, bridge);
    });
    os_drop::draw(ctx, state);
    if ctx.input(|i| !i.pointer.any_down()) && ctx.memory(|m| m.focused().is_none()) {
        state.seal_edits();
    }
    dock_layout::tick(state);
    panels::about::draw_about_modal(ctx, state);
    panels::command_palette::draw(ctx, state);
    panels::text_editor::find_panel::draw(ctx, state);
    panels::overlay_copy::draw(ctx, state);
    panels::edit_mode::draw_refused_write(ctx, state);
    panels::save_destination::draw(ctx, state);
    panels::file_prompts::draw(ctx, state);
    panels::project::draw_questions(ctx, state);
    panels::new_project::draw(ctx, state);
    panels::guide::draw(ctx, state);
    panels::tour::draw(ctx, state);
}
