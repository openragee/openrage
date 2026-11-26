use crate::state::AppState;
use crate::ui::commands;
use crate::ui::shortcuts::{self, presets};
use crate::ui::theme;

const CAPTURE_KEY: &str = "settings_shortcut_capture";
const CONFLICT_KEY: &str = "settings_shortcut_conflict";

pub fn reset_transient(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<Option<commands::Action>>(egui::Id::new(CAPTURE_KEY));
        d.remove::<PendingConflict>(egui::Id::new(CONFLICT_KEY));
    });
}

#[derive(Clone)]
struct PendingConflict {
    new_action: commands::Action,
    /// Every command Overwrite would leave without the key: two panels' commands can hold one.
    existing: Vec<commands::Action>,
    mods: shortcuts::ShortcutMods,
    key: egui::Key,
}

pub fn draw_shortcuts_page(ui: &mut egui::Ui, state: &mut AppState, query_lc: &str) {
    ui.heading(tr!("Keyboard shortcuts"));
    ui.add_space(theme::SPACING_HAIR);
    let primary = shortcuts::primary_modifier_name();
    ui.label(
        egui::RichText::new(tr!(
            "Click a row's Edit button, then press the keys. A shortcut follows the \
             character its key types on your layout. One without {primary} or Alt does \
             nothing while you type in a field, and Tab, Space, Enter and Backspace \
             need {primary}, Shift or Alt.",
            primary = primary
        ))
        .small()
        .weak(),
    );
    ui.add_space(theme::SPACING_DEFAULT);

    ui.horizontal(|ui| {
        preset_choice(ui, state);
        let has_overrides = shortcuts::has_overrides(&state.prefs);
        let restore_btn = egui::Button::new(tr!("Restore all defaults"));
        if ui
            .add_enabled(has_overrides, restore_btn)
            .on_hover_text(tr!(
                "Drop every user-customised binding and revert to the compile-time defaults."
            ))
            .clicked()
        {
            shortcuts::restore_defaults(&mut state.prefs);
            crate::state::save_prefs(&state.prefs);
            state.show_toast(tr!("All keyboard shortcuts restored to defaults."));
            reset_transient(ui.ctx());
        }
    });
    preset_note(ui, &state.prefs);
    ui.add_space(theme::SPACING_SNUG);
    ui.separator();
    ui.add_space(theme::SPACING_SNUG);

    let capture_id = egui::Id::new(CAPTURE_KEY);
    let conflict_id = egui::Id::new(CONFLICT_KEY);
    let capturing: Option<commands::Action> = ui
        .ctx()
        .data(|d| d.get_temp::<Option<commands::Action>>(capture_id))
        .unwrap_or(None);
    let pending_conflict: Option<PendingConflict> = ui
        .ctx()
        .data(|d| d.get_temp::<PendingConflict>(conflict_id));

    let rows: Vec<&commands::Spec> = commands::COMMANDS
        .iter()
        .filter(|spec| query_lc.is_empty() || matches_query(spec, &state.prefs, query_lc))
        .collect();

    if rows.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(tr!(
                    "No shortcuts match \"{query_lc}\"",
                    query_lc = query_lc
                ))
                .color(theme::TEXT_SECONDARY),
            );
        });
        return;
    }

    let mut to_capture: Option<Option<commands::Action>> = None;
    let mut to_reset: Option<commands::Action> = None;
    let mut conflict_to_set: Option<Option<PendingConflict>> = None;

    // ⚠ A long action name wraps: at its full width it pushed Edit and Reset out of the page.
    let name_width = (ui.available_width() - 260.0).max(160.0);
    egui::Grid::new("shortcuts_table")
        .num_columns(4)
        .max_col_width(name_width)
        .spacing([theme::SPACING_RELAXED, theme::SPACING_TIGHT])
        .striped(true)
        .show(ui, |ui| {
            ui.label(egui::RichText::new(tr!("Action")).strong());
            ui.label(egui::RichText::new(tr!("Binding")).strong());
            ui.label("");
            ui.label("");
            ui.end_row();

            for spec in &rows {
                let action = spec.action;
                let name = ui.label(name_of(spec));
                if !spec.hover.is_empty() {
                    name.on_hover_text(crate::i18n::tr(spec.hover));
                }

                if capturing == Some(action) {
                    ui.colored_label(
                        theme::INFO,
                        egui::RichText::new(tr!("Press a combo… (Esc to cancel)")).italics(),
                    );
                } else {
                    let labels = shortcuts::labels_for(action, &state.prefs);
                    let label_text = if labels.is_empty() {
                        tr!("(unbound)").to_string()
                    } else {
                        // One a line: side by side, a second binding pushes the
                        // Reset column out of the window.
                        labels.join("\n")
                    };
                    let mut text = egui::RichText::new(label_text).monospace();
                    let own = shortcuts::is_overridden(&state.prefs, action);
                    if own {
                        text = text.color(theme::ACCENT_TEXT);
                    } else if labels.is_empty() {
                        text = text.color(theme::TEXT_DISABLED);
                    }
                    let binding = ui.label(text);
                    if own {
                        binding.on_hover_text(tr!("A key you chose, which no preset changes"));
                    }
                }

                let edit_label = if capturing == Some(action) {
                    tr!("Cancel")
                } else {
                    tr!("Edit")
                };
                if ui.button(edit_label).clicked() {
                    if capturing == Some(action) {
                        to_capture = Some(None);
                    } else {
                        to_capture = Some(Some(action));
                        conflict_to_set = Some(None);
                    }
                }

                let can_reset = shortcuts::is_overridden(&state.prefs, action);
                if ui
                    .add_enabled(can_reset, egui::Button::new(tr!("Reset")))
                    .on_hover_text(tr!("Restore this shortcut to its default"))
                    .clicked()
                {
                    to_reset = Some(action);
                }
                ui.end_row();
            }
        });

    if let Some(action) = capturing {
        if let Some((mods, typed, position)) = next_combo(ui.ctx()) {
            let by_position = commands::spec(action).is_some_and(|spec| spec.by_position);
            let key = key_captured(by_position, typed, position);
            if key == egui::Key::Escape && !mods.ctrl && !mods.shift && !mods.alt {
                to_capture = Some(None);
            } else if !shortcuts::is_valid_combo(mods, key) {
                state.show_error_toast(tr!(
                    "Tab, Space, Enter and Backspace need Ctrl, Shift or Alt."
                ));
                to_capture = Some(None);
            } else {
                match shortcuts::set_binding(&mut state.prefs, action, mods, key) {
                    Ok(()) => {
                        crate::state::save_prefs(&state.prefs);
                        state.show_toast(tr!(
                            "Bound {action} to {shortcuts}.",
                            action = sc_description(action),
                            shortcuts = shortcuts::format_for(action, mods, key, &state.prefs)
                        ));
                        to_capture = Some(None);
                    }
                    Err(conflict) => {
                        // The command the refusal names, then every other that holds the key.
                        let mut existing = vec![conflict.existing];
                        let holders = shortcuts::holders(&state.prefs, action, mods, key);
                        existing.extend(holders.into_iter().filter(|&a| a != conflict.existing));
                        conflict_to_set = Some(Some(PendingConflict {
                            new_action: action,
                            existing,
                            mods,
                            key,
                        }));
                        to_capture = Some(None);
                    }
                }
            }
        }
    }

    if let Some(conflict) = pending_conflict.clone() {
        ui.add_space(theme::SPACING_DEFAULT);
        egui::Frame::group(ui.style())
            .fill(theme::BG_L2.get())
            .show(ui, |ui| {
                for &existing in &conflict.existing {
                    ui.colored_label(
                        theme::WARNING,
                        tr!(
                            "{shortcuts} is already bound to “{existing}”.",
                            shortcuts = shortcuts::format_for(
                                conflict.new_action,
                                conflict.mods,
                                conflict.key,
                                &state.prefs
                            ),
                            existing = sc_description(existing)
                        ),
                    );
                }
                ui.label(
                    egui::RichText::new(tr!(
                        "Overwriting will leave the previous action without a binding."
                    ))
                    .small()
                    .weak(),
                );
                ui.horizontal(|ui| {
                    if ui.button(tr!("Overwrite")).clicked() {
                        shortcuts::force_set_binding(
                            &mut state.prefs,
                            conflict.new_action,
                            conflict.mods,
                            conflict.key,
                        );
                        crate::state::save_prefs(&state.prefs);
                        for &existing in &conflict.existing {
                            state.show_toast(tr!(
                                "Bound {new_action} to {shortcuts}; {existing} is now unbound.",
                                new_action = sc_description(conflict.new_action),
                                shortcuts = shortcuts::format_for(
                                    conflict.new_action,
                                    conflict.mods,
                                    conflict.key,
                                    &state.prefs
                                ),
                                existing = sc_description(existing)
                            ));
                        }
                        conflict_to_set = Some(None);
                    }
                    if ui.button(tr!("Cancel")).clicked() {
                        conflict_to_set = Some(None);
                    }
                });
            });
    }

    if let Some(next) = to_capture {
        ui.ctx().data_mut(|d| d.insert_temp(capture_id, next));
    }
    if let Some(action) = to_reset {
        shortcuts::reset_binding(&mut state.prefs, action);
        crate::state::save_prefs(&state.prefs);
        state.show_toast(tr!(
            "Reset {action} to its default.",
            action = sc_description(action)
        ));
    }
    if let Some(next) = conflict_to_set {
        match next {
            Some(c) => ui.ctx().data_mut(|d| {
                d.insert_temp(conflict_id, c);
            }),
            None => ui.ctx().data_mut(|d| {
                d.remove::<PendingConflict>(conflict_id);
            }),
        }
    }
}

fn sc_description(action: commands::Action) -> &'static str {
    commands::spec(action).map_or(tr!("(unknown action)"), |spec| crate::i18n::tr(spec.label))
}

/// A command as the palette names it: where it sits, then its label.
fn name_of(spec: &commands::Spec) -> String {
    format!(
        "{}: {}",
        crate::i18n::tr(spec.category),
        crate::i18n::tr(spec.label)
    )
}

/// Whether the settings search names this command, its hover text or its binding.
pub fn matches_query(spec: &commands::Spec, prefs: &crate::state::Prefs, query_lc: &str) -> bool {
    [name_of(spec), crate::i18n::tr(spec.hover).to_string()]
        .into_iter()
        .chain(shortcuts::label_for(spec.action, prefs))
        .any(|text| text.to_lowercase().contains(query_lc))
}

/// The next key pressed: its modifiers, the key it types, and where it sits when the event says.
fn next_combo(
    ctx: &egui::Context,
) -> Option<(shortcuts::ShortcutMods, egui::Key, Option<egui::Key>)> {
    ctx.input(|i| {
        i.events.iter().find_map(|event| match event {
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } => Some(((*modifiers).into(), *key, *physical_key)),
            _ => None,
        })
    })
}

/// The key a press binds: for a command bound by where its key sits, the place pressed, whatever that key types.
fn key_captured(by_position: bool, typed: egui::Key, position: Option<egui::Key>) -> egui::Key {
    position.filter(|_| by_position).unwrap_or(typed)
}

/// The preset of keys in use, and the others to pick.
fn preset_choice(ui: &mut egui::Ui, state: &mut AppState) {
    let chosen = presets::chosen(&state.prefs);
    let mut picked = chosen.id;
    ui.label(tr!("Preset"));
    let combo = egui::ComboBox::from_id_salt("shortcut_preset")
        .selected_text(crate::i18n::tr(chosen.name))
        .show_ui(ui, |ui| {
            for preset in presets::PRESETS {
                ui.selectable_value(&mut picked, preset.id, crate::i18n::tr(preset.name));
            }
        });
    crate::ui::accessible_name::named(combo.response, tr!("Preset")).on_hover_text(tr!(
        "A set of keys for the commands it names: every other command keeps its key, and so does one whose key you chose. OpenRage is the set the app starts with."
    ));
    let Some(preset) = (presets::PRESETS.iter()).find(|preset| preset.id == picked) else {
        return;
    };
    if preset.id != chosen.id {
        presets::choose(&mut state.prefs, preset);
        crate::state::save_prefs(&state.prefs);
        reset_transient(ui.ctx());
    }
}

/// How many commands the preset in use changes, and how many keep a key the person chose; nothing for OpenRage's own.
fn preset_note(ui: &mut egui::Ui, prefs: &crate::state::Prefs) {
    let preset = presets::chosen(prefs);
    if preset.id == presets::OPENRAGE.id {
        return;
    }
    let changes = presets::changes(preset, prefs);
    let mut note = tr_n!(
        "{preset} changes the keys of {count} command.",
        "{preset} changes the keys of {count} commands.",
        changes.changed,
        preset = crate::i18n::tr(preset.name)
    );
    if changes.kept > 0 {
        note.push(' ');
        note.push_str(&tr_n!(
            "{count} command it names keeps the key you chose.",
            "{count} commands it names keep the keys you chose.",
            changes.kept
        ));
    }
    ui.label(egui::RichText::new(note).small().weak());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ThemeChoice;
    use crate::ui::commands::Action;
    use crate::ui::frames_of_the_shell::{key, Shell};
    use crate::ui::panels::settings::SettingsCategory;

    /// The page asked for a key for `action`, and `pressed` pressed; the frame after it is the one read.
    fn capture(shell: &mut Shell, action: Action, pressed: egui::Key) {
        // Opened afresh, as Edit clears a refusal still on the page.
        shell.settings(SettingsCategory::Shortcuts);
        (shell.ctx).data_mut(|d| d.insert_temp(egui::Id::new(CAPTURE_KEY), Some(action)));
        shell.draw_settings(vec![key(pressed)]);
        shell.draw_settings(Vec::new());
    }

    fn says(shell: &Shell, text: &str) -> bool {
        (shell.described.iter()).any(|widget| widget.name.as_deref() == Some(text))
    }

    #[test]
    fn a_key_captured_for_a_command_bound_by_position_is_the_place_pressed() {
        // A French keyboard's fourth key of the digit row types an apostrophe.
        let (typed, position) = (egui::Key::Quote, Some(egui::Key::Num4));
        assert_eq!(key_captured(true, typed, position), egui::Key::Num4);
        assert_eq!(key_captured(false, typed, position), egui::Key::Quote);
        // An event that says no place leaves the key it typed.
        assert_eq!(key_captured(true, typed, None), egui::Key::Quote);
    }

    #[test]
    fn the_page_gives_a_panels_key_to_another_panels_command_and_refuses_it_to_the_dispatchers() {
        let _serial = theme::serial();
        let mut shell = Shell::new(ThemeChoice::Dark);
        // F frames the selection in the viewport, by the table.
        let refusal = "F is already bound to “Frame selection”.";

        // Show grid is read by the dispatcher, wherever the pointer is.
        capture(&mut shell, Action::ShowGrid, egui::Key::F);
        assert!(
            says(&shell, refusal),
            "the dispatcher's command took a key a panel reads"
        );
        assert_eq!(
            shortcuts::label_for(Action::ShowGrid, &shell.state.prefs),
            None
        );

        // Frame all is the viewport's as well.
        capture(&mut shell, Action::FrameAll, egui::Key::F);
        assert!(
            says(&shell, refusal),
            "a second command of the viewport took its key"
        );
        assert_eq!(
            shortcuts::label_for(Action::FrameAll, &shell.state.prefs).as_deref(),
            Some("Home")
        );

        // The lists read Quick look themselves, and never where the viewport reads.
        capture(&mut shell, Action::QuickLook, egui::Key::F);
        assert!(
            !(shell.described.iter()).any(|widget| (widget.name.as_deref())
                .is_some_and(|name| name.contains("is already bound to"))),
            "the lists' command was refused the viewport's key"
        );
        assert_eq!(
            shortcuts::label_for(Action::QuickLook, &shell.state.prefs).as_deref(),
            Some("F")
        );
        assert_eq!(
            shortcuts::label_for(Action::FrameSelection, &shell.state.prefs).as_deref(),
            Some("F or .")
        );
    }

    #[test]
    fn the_page_names_every_command_a_key_is_taken_from_and_overwrite_unbinds_each() {
        let _serial = theme::serial();
        let mut shell = Shell::new(ThemeChoice::Dark);
        let none = shortcuts::ShortcutMods::none();
        // The lists' Quick look shares F with the viewport's Frame selection, as two panels may.
        let lists = shortcuts::set_binding(
            &mut shell.state.prefs,
            Action::QuickLook,
            none,
            egui::Key::F,
        );
        assert!(lists.is_ok(), "{lists:?}");

        // Show grid is read by the dispatcher, so it would take the key from both.
        // ⚠ The page is searched down to that row: under the whole table the refusal is drawn below the window, where no click lands.
        let page = shell
            .state
            .panels
            .get::<crate::ui::panels::settings::SettingsUiState>();
        page.query = "show grid".into();
        capture(&mut shell, Action::ShowGrid, egui::Key::F);
        let losing = ["Frame selection", "Quick look"];
        for existing in losing {
            assert!(
                says(&shell, &format!("F is already bound to “{existing}”.")),
                "the page does not say that {existing} would lose its key"
            );
        }

        let at = (shell.described.iter())
            .find(|widget| widget.name.as_deref() == Some("Overwrite"))
            .expect("a button to overwrite")
            .bounds
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        shell.draw_settings(vec![egui::Event::PointerMoved(at), button(true)]);
        shell.draw_settings(vec![button(false)]);
        shell.draw_settings(Vec::new());
        let label = |action| shortcuts::label_for(action, &shell.state.prefs);
        assert_eq!(label(Action::ShowGrid).as_deref(), Some("F"));
        assert_eq!(label(Action::FrameSelection), None);
        assert_eq!(label(Action::QuickLook), None);
        for existing in losing {
            let told = format!("Bound Show grid to F; {existing} is now unbound.");
            assert!(
                (shell.state.toasts.iter()).any(|toast| toast.message == told),
                "no toast says that {existing} lost its key"
            );
        }
    }
}
