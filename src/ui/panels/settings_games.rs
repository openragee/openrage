//! The Games page of the settings window: the active game, each game's install folder, and the keys that open its encrypted archives.

use crate::platform::file_dialogs;
use crate::state::{AppState, Game};
use crate::ui::theme;
use egui_phosphor::regular as icon;

pub fn draw_games_page(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading(tr!("Games"));
    ui.add_space(theme::SPACING_HAIR);
    ui.label(
        egui::RichText::new(tr!(
            "OpenRage reads the active game's install for its files and its keys. Each \
             game's folder is remembered separately."
        ))
        .small()
        .weak(),
    );
    ui.add_space(theme::SPACING_SNUG);

    super::settings_launch::draw_launch_section(ui, state);
    ui.add_space(theme::SPACING_SNUG);

    ui.horizontal(|ui| {
        ui.label(tr!("Active game:"));
        let current = state.prefs.active_game;
        let selected_text = current.map(Game::label).unwrap_or(tr!("(none)"));
        let mut chosen: Option<Game> = None;
        let combo = egui::ComboBox::from_id_salt("active_game")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for game in Game::all() {
                    let supported =
                        game.profile().is_supported() || crate::state::gta_vi::read_by_scan(game);
                    let row = ui.add_enabled(
                        supported,
                        egui::Button::selectable(current == Some(game), game.label()),
                    );
                    if !supported {
                        row.on_hover_text(game.profile().description());
                    } else if row.clicked() {
                        chosen = Some(game);
                    }
                }
            });
        crate::ui::accessible_name::named(combo.response, tr!("Active game"));
        if let Some(game) = chosen {
            state.switch_active_game(game);
        }
    });

    ui.add_space(theme::SPACING_SNUG);
    for game in Game::all() {
        draw_game_row(ui, state, game);
    }
}

fn draw_game_row(ui: &mut egui::Ui, state: &mut AppState, game: Game) {
    let is_active = state.prefs.active_game == Some(game);
    let supported = game.profile().is_supported();
    let path = state.prefs.game_paths.get(game).cloned();
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            let label = if is_active {
                egui::RichText::new(game.label()).strong()
            } else {
                egui::RichText::new(game.label())
            };
            ui.label(label);
            if is_active {
                ui.colored_label(theme::SUCCESS, tr!("(active)"));
            }
            if !supported {
                let (colour, note, hover) = if crate::state::gta_vi::read_by_scan(game) {
                    let keys = crate::state::rpf9_scan::rpf9_table::gta6_keys_held::gta6_keys();
                    if keys == crate::state::rpf9_scan::rpf9_table::gta6_keys_held::Gta6Keys::Asking
                    {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(200));
                    }
                    gta6_note(&keys, game.profile().description())
                } else {
                    let note = tr!("(not readable yet)").to_string();
                    (
                        theme::WARNING,
                        note,
                        game.profile().description().to_string(),
                    )
                };
                ui.colored_label(colour, note).on_hover_text(hover);
            }
        });
        ui.add_space(theme::SPACING_HAIR);
        match &path {
            Some(p) => {
                ui.label(tr!("Folder: {p}", p = p.display()));
                if game.supports_key_extraction() && game.find_executable(p).is_none() {
                    let candidates = game.executable_candidates().join(" or ");
                    ui.colored_label(
                        theme::WARNING,
                        tr!(
                            "Warning: '{candidates}' not found in this folder",
                            candidates = candidates
                        ),
                    );
                }
            }
            None => {
                ui.label(
                    egui::RichText::new(tr!("Folder: (not configured)"))
                        .italics()
                        .weak(),
                );
            }
        }
        ui.horizontal(|ui| {
            let pick_label = if path.is_some() {
                tr!("Change folder…")
            } else {
                tr!("Select folder…")
            };
            if ui.button(pick_label).clicked() {
                let title = tr!("Select install folder for {game}", game = game.label());
                if let Some(dir) = file_dialogs::pick_game_install_folder(&title) {
                    if !supported {
                        state.open_title_folder(game, dir);
                    } else {
                        state.set_game_install_folder(game, Some(dir.clone()));
                        let kept = state.prefs.game_paths.get(game) == Some(&dir);
                        if kept && (is_active || state.prefs.active_game.is_none()) {
                            state.request_game_switch(game, dir);
                        }
                    }
                }
            }
            if let (Some(dir), false) = (&path, supported) {
                let label = if crate::state::gta_vi::read_by_scan(game) {
                    tr!("Open")
                } else {
                    tr!("Open as a folder")
                };
                let open = ui.button(label).on_hover_text(game.profile().description());
                if open.clicked() {
                    state.open_title_folder(game, dir.clone());
                }
            }
            if supported && !is_active && path.is_some() && ui.button(tr!("Set active")).clicked() {
                state.switch_active_game(game);
            }
            if path.is_some()
                && ui
                    .button(tr!("Forget folder"))
                    .on_hover_text(tr!(
                        "OpenRage stops using this folder. Nothing on disk is deleted."
                    ))
                    .clicked()
            {
                state.set_game_install_folder(game, None);
            }
        });
        if is_active {
            draw_game_keys(ui, state, game);
        }
    });
    ui.add_space(theme::SPACING_HAIR);
}

/// The note beside GTA VI's name: whether its keys were found and how many names are held, its colour, and its hover text.
fn gta6_note(
    keys: &crate::state::rpf9_scan::rpf9_table::gta6_keys_held::Gta6Keys,
    description: &str,
) -> (theme::Themed, String, String) {
    use crate::state::rpf9_scan::rpf9_table::gta6_keys_held::Gta6Keys;
    let (colour, note) = match keys {
        Gta6Keys::Held { names: Some(held) } => (
            theme::SUCCESS,
            tr_n!(
                "(keys found, {count} name held)",
                "(keys found, {count} names held)",
                *held
            ),
        ),
        Gta6Keys::Held { names: None } => (
            theme::SUCCESS,
            tr!("(keys found, reading the names…)").to_string(),
        ),
        Gta6Keys::Asking => (
            theme::TEXT_SECONDARY,
            tr!("(looking for the keys…)").to_string(),
        ),
        Gta6Keys::Missing(why) => {
            let note = tr!("(no keys: its archives are read by a scan)").to_string();
            return (theme::WARNING, note, why.clone());
        }
    };
    (colour, note, description.to_string())
}

/// The open game's keys: one coloured line that says whether archives open, and the detail behind its arrow.
fn draw_game_keys(ui: &mut egui::Ui, state: &mut AppState, game: Game) {
    let gta_keys = game.supports_key_extraction();
    if !gta_keys && state.keys.title_status().is_none() {
        return;
    }
    if state.keys.extraction.is_running {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
        state.poll_key_extraction();
    }
    let (colour, summary) = match state.keys.title_status() {
        _ if gta_keys && state.keys.has_keys() => (theme::SUCCESS, "loaded"),
        _ if gta_keys && state.keys.extraction.is_running => (theme::INFO, tr!("extracting…")),
        _ if gta_keys => (theme::WARNING, tr!("not set up")),
        Some(Ok(_)) => (theme::SUCCESS, "ready"),
        Some(Err(_)) => (theme::ERROR, "unavailable"),
        None => return,
    };
    let header = egui::RichText::new(tr!(
        "{icon} Keys: {summary}",
        summary = summary,
        icon = icon::KEY
    ))
    .color(colour);
    egui::CollapsingHeader::new(header)
        .id_salt(("game_keys", game.profile().id()))
        .show(ui, |ui| {
            if gta_keys {
                draw_gta_keys_details(ui, state);
            } else {
                draw_title_keys_details(ui, state);
            }
        });
}

fn draw_gta_keys_details(ui: &mut egui::Ui, state: &mut AppState) {
    draw_keys_status(ui, state);
    ui.add_space(theme::SPACING_SNUG);
    if state.keys.extraction.is_running {
        draw_extraction_progress(ui, state);
    } else {
        draw_keys_actions(ui, state);
    }
    if let Some(err) = &state.keys.extraction.last_error {
        ui.add_space(theme::SPACING_TIGHT);
        ui.colored_label(theme::ERROR, tr!("Last error: {err}", err = err));
    }
}

/// A title whose cipher is built when its install opens (RDR2): where the tables came from, or why none.
fn draw_title_keys_details(ui: &mut egui::Ui, state: &AppState) {
    match state.keys.title_status() {
        Some(Ok(source)) => {
            ui.label(tr!("Encrypted archives open."));
            let missing = state.keys.provider().missing_keys();
            if !missing.is_empty() {
                let ids: Vec<String> = missing.iter().map(|id| format!("{id:#04X}")).collect();
                ui.colored_label(
                    theme::WARNING,
                    tr!(
                        "Some archives use keys this snapshot lacks ({ids}), so they stay closed. \
                         A snapshot made from a newer game build holds them.",
                        ids = ids.join(", ")
                    ),
                );
            }
            ui.label(
                egui::RichText::new(tr!("Cipher tables: {source}", source = source))
                    .small()
                    .weak(),
            );
            ui.label(
                egui::RichText::new(tr!(
                    "To use your own snapshot, set OPENRAGE_RDR2_SNAPSHOT or put rdr2_magic.dat \
                     in the configuration folder, then reopen the game."
                ))
                .small()
                .weak(),
            );
        }
        Some(Err(why)) => {
            ui.colored_label(theme::ERROR, why);
        }
        None => {}
    }
}

fn draw_keys_status(ui: &mut egui::Ui, state: &AppState) {
    let (color, text) = if state.keys.has_keys() {
        (
            theme::SUCCESS,
            tr!("Status: keys loaded — encrypted archives open"),
        )
    } else if state.keys.extraction.is_running {
        (theme::INFO, tr!("Status: extracting…"))
    } else {
        (
            theme::WARNING,
            tr!("Status: no keys — choose gta5.exe to open encrypted archives"),
        )
    };
    ui.colored_label(color, text);

    ui.add_space(theme::SPACING_HAIR);
    if let Some(path) = &state.prefs.gta_keys.gta5_path {
        ui.label(format!("gta5.exe: {}", path.display()));
    } else {
        ui.label(tr!("gta5.exe: (not selected)"));
    }
    if let Some(at) = state.prefs.gta_keys.extracted_at {
        ui.label(format!(
            "Last extracted: {}",
            or_io::container::KeysCacheMetadata {
                schema: or_io::container::KEYS_CACHE_SCHEMA,
                gta5_path: state.prefs.gta_keys.gta5_path.clone().unwrap_or_default(),
                gta5_sha1_hex: state
                    .prefs
                    .gta_keys
                    .gta5_sha1_hex
                    .clone()
                    .unwrap_or_default(),
                gta5_size: state.prefs.gta_keys.gta5_size.unwrap_or_default(),
                extracted_at: at,
            }
            .extracted_at_display()
        ));
    }

    let report = state.keys.provenance_report();
    if report.is_empty() {
        return;
    }
    ui.add_space(theme::SPACING_TIGHT);
    ui.label(egui::RichText::new(tr!("Where these keys came from")).strong());
    let colour = if state.keys.snapshot_disagrees() {
        theme::WARNING
    } else {
        theme::TEXT_DIM
    };
    for line in report {
        ui.colored_label(colour, egui::RichText::new(line).small());
    }
}

fn draw_extraction_progress(ui: &mut egui::Ui, state: &AppState) {
    let frac = state.keys.extraction.fraction();
    ui.label(format!(
        "{}: {} / {}",
        state.keys.extraction.stage, state.keys.extraction.found, state.keys.extraction.total
    ));
    ui.add(egui::ProgressBar::new(frac).show_percentage());
}

fn draw_keys_actions(ui: &mut egui::Ui, state: &mut AppState) {
    let has_path = state.prefs.gta_keys.gta5_path.is_some();
    let primary_label = if state.keys.has_keys() {
        tr!("Re-extract Keys")
    } else if has_path {
        tr!("Extract Keys")
    } else {
        tr!("Select gta5.exe…")
    };
    ui.horizontal(|ui| {
        if ui
            .button(primary_label)
            .on_hover_text(tr!(
                "Extract the AES key and hash table from your game executable. \
                 NG keys are in no shipped executable: they come from your own \
                 magic.dat if one sits beside the keys cache, else from the key \
                 snapshot bundled with OpenRage."
            ))
            .clicked()
        {
            let path = state
                .prefs
                .gta_keys
                .gta5_path
                .clone()
                .or_else(file_dialogs::pick_gta5_exe);
            if let Some(path) = path {
                state.start_key_extraction(path);
            }
        }
        if has_path
            && ui
                .button(tr!("Change gta5.exe…"))
                .on_hover_text(tr!("Pick a different gta5.exe"))
                .clicked()
        {
            if let Some(path) = file_dialogs::pick_gta5_exe() {
                state.start_key_extraction(path);
            }
        }
        if ui
            .add_enabled(state.keys.has_keys(), egui::Button::new(tr!("Forget")))
            .on_hover_text(tr!("Drop the cached keys + on-disk cache file"))
            .clicked()
        {
            state.forget_gta_keys();
        }
    });

    ui.add_space(theme::SPACING_TIGHT);
    ui.collapsing(tr!("Advanced"), |ui| {
        ui.label(
            egui::RichText::new(tr!(
                "Without NG keys, AES-encrypted archives and scrambled audio open \
                 and NG-encrypted archives do not. NG keys come from your own \
                 magic.dat beside the keys cache, else from the bundled snapshot."
            ))
            .small()
            .weak(),
        );
        ui.add_space(theme::SPACING_HAIR);
        let can_reextract = has_path;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_reextract, egui::Button::new(tr!("AES key only")))
                .on_hover_text(tr!("Skip both the NG sweep and any key snapshot. \
                     Yields a cache that reads AES-mode RPFs only; NG-mode \
                     RPFs return a structured error."))
                .clicked()
            {
                if let Some(path) = state.prefs.gta_keys.gta5_path.clone() {
                    state.keys.start_extraction_aes_only(path);
                    state.show_info_toast(tr!("Extracting AES key only…"));
                }
            }
            if ui
                .button(tr!("Your own magic.dat…"))
                .on_hover_text(tr!(
                    "Decode NG and AWC key material from a key snapshot you \
                     already have. Picks gta5.exe first if none is configured."
                ))
                .clicked()
            {
                let exe = state
                    .prefs
                    .gta_keys
                    .gta5_path
                    .clone()
                    .or_else(file_dialogs::pick_gta5_exe);
                if let Some(exe) = exe {
                    if let Some(magic) = file_dialogs::pick_magic_dat() {
                        state.start_key_extraction_from_magic(exe, magic);
                    }
                }
            }
        });

        ui.add_space(theme::SPACING_TIGHT);
        let can_warm_up = state.keys.has_keys() && !state.keys.has_encrypt_state();
        if ui
            .add_enabled(
                can_warm_up,
                egui::Button::new(tr!("Pre-derive NG encrypt state")),
            )
            .on_hover_text(tr!(
                "One-time ~10 s derivation that powers NG-mode RPF saves. \
                 Without it, the first NG save blocks for the same time. \
                 The result is persisted to disk (~25 MiB) so subsequent \
                 launches reuse it."
            ))
            .clicked()
        {
            state.keys.start_encrypt_state_derivation();
            state.show_info_toast(tr!("Deriving NG encrypt tables (one-time, ~10 s)…"));
        }
        if state.keys.has_encrypt_state() {
            ui.colored_label(
                theme::SUCCESS,
                tr!("NG encrypt state ready (cached on disk)."),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::rpf9_scan::rpf9_table::gta6_keys_held::Gta6Keys;

    #[test]
    fn gta_vi_s_note_says_the_keys_are_found_and_the_names_held_or_why_a_scan_reads_it() {
        let (_, note, _) = gta6_note(&Gta6Keys::Held { names: Some(3) }, "d");
        assert_eq!(note, "(keys found, 3 names held)");
        assert!(!note.contains("without names"));
        let (_, note, hover) = gta6_note(&Gta6Keys::Missing("zq why".to_string()), "d");
        assert!(note.contains("scan"), "{note}");
        assert_eq!(hover, "zq why");
    }
}
