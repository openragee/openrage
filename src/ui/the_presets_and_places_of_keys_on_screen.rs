//! Settings ▸ Keyboard shortcuts as a person meets it: a preset is picked from a named control and says how many commands it
//! changes, and a key is refused to a second command only where both are read in one place.

use super::frames_of_the_shell::Shell;
use crate::state::ThemeChoice;
use crate::ui::commands::Action;
use crate::ui::panels::settings::SettingsCategory;
use crate::ui::shortcuts::{self, presets};
use crate::ui::theme;

/// A click in the Settings window, and the frames in which what it opened settles.
fn click(shell: &mut Shell, at: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    shell.draw_settings(vec![egui::Event::PointerMoved(at), button(true)]);
    shell.draw_settings(vec![button(false)]);
    shell.draw_settings(Vec::new());
    shell.draw_settings(Vec::new());
}

/// Where the widget a screen reader calls `name` is drawn.
fn named(shell: &Shell, name: &str) -> egui::Pos2 {
    let found: Vec<_> = (shell.described.iter())
        .filter(|widget| widget.name.as_deref() == Some(name))
        .filter(|widget| widget.role != egui::accesskit::Role::Label)
        .collect();
    match found.as_slice() {
        [only] => only.bounds.center(),
        _ => panic!(
            "{} controls are called {name:?}; the frame names {:?}",
            found.len(),
            (shell.described.iter())
                .filter_map(|widget| widget.name.clone())
                .collect::<Vec<_>>()
        ),
    }
}

/// Whether the last frame wrote `text`.
fn says(shell: &Shell, text: &str) -> bool {
    (shell.described.iter()).any(|widget| widget.name.as_deref() == Some(text))
}

fn pick(shell: &mut Shell, preset: &str) {
    let combo = named(shell, "Preset");
    click(shell, combo);
    let row = named(shell, preset);
    click(shell, row);
}

#[test]
fn a_preset_is_picked_from_a_named_control_and_says_how_many_commands_it_changes() {
    let _serial = theme::serial();
    let mut shell = Shell::new(ThemeChoice::Dark);
    shell.settings(SettingsCategory::Shortcuts);
    let before = shortcuts::label_for(Action::CreateProject, &shell.state.prefs);
    assert_eq!(presets::chosen(&shell.state.prefs).id, presets::OPENRAGE.id);
    assert!(
        !(shell.described.iter()).any(|w| w
            .name
            .as_deref()
            .is_some_and(|n| n.contains("changes the keys"))),
        "OpenRage's own keys are said to change a command"
    );

    pick(&mut shell, "CodeWalker");
    assert_eq!(presets::chosen(&shell.state.prefs).id, "codewalker");
    let primary = shortcuts::primary_modifier_name();
    assert_eq!(
        shortcuts::label_for(Action::CreateProject, &shell.state.prefs),
        Some(format!("{primary}+N"))
    );
    assert!(
        says(&shell, "CodeWalker changes the keys of 1 command."),
        "the page does not say how many commands the preset changes"
    );
    let kept = crate::state::prefs::load_prefs_from(
        &crate::state::prefs::prefs_path().expect("the test keeps its preferences in a folder"),
    );
    assert_eq!(
        presets::chosen(&kept).id,
        "codewalker",
        "the choice was not written"
    );

    // A key of the person's own on a command the preset names is kept, and said to be.
    let alt = shortcuts::ShortcutMods {
        ctrl: false,
        control: false,
        shift: false,
        alt: true,
    };
    let own = shortcuts::set_binding(
        &mut shell.state.prefs,
        Action::CreateProject,
        alt,
        egui::Key::F7,
    );
    assert!(own.is_ok(), "{own:?}");
    shell.draw_settings(Vec::new());
    assert!(
        says(
            &shell,
            "CodeWalker changes the keys of 0 commands. 1 command it names keeps the key you chose."
        ),
        "the page does not say that a key the person chose is kept"
    );

    pick(&mut shell, "OpenRage");
    assert_eq!(presets::chosen(&shell.state.prefs).id, presets::OPENRAGE.id);
    assert_eq!(
        shortcuts::label_for(Action::CreateProject, &shell.state.prefs).as_deref(),
        Some("Alt+F7"),
        "returning to OpenRage's own keys took the key the person chose"
    );
    shortcuts::reset_binding(&mut shell.state.prefs, Action::CreateProject);
    assert_eq!(
        shortcuts::label_for(Action::CreateProject, &shell.state.prefs),
        before
    );
}

/// The button that drops every key chosen and the preset, as the last frame drew it.
fn restore_all(shell: &Shell) -> &super::frames_of_the_shell::Described {
    (shell.described.iter())
        .find(|widget| {
            widget.name.as_deref() == Some("Restore all defaults")
                && widget.role == egui::accesskit::Role::Button
        })
        .expect("a button to restore every default")
}

#[test]
fn restore_all_defaults_is_offered_under_a_preset_and_returns_to_openrages_own() {
    let _serial = theme::serial();
    let mut shell = Shell::new(ThemeChoice::Dark);
    shell.settings(SettingsCategory::Shortcuts);
    assert!(
        restore_all(&shell).disabled,
        "there is nothing to restore under OpenRage's own keys"
    );

    pick(&mut shell, "CodeWalker");
    let button = restore_all(&shell);
    assert!(
        !button.disabled,
        "a preset is chosen and the page offers no way back but the list"
    );
    let at = button.bounds.center();
    click(&mut shell, at);
    assert_eq!(presets::chosen(&shell.state.prefs).id, presets::OPENRAGE.id);
    assert!(restore_all(&shell).disabled);
    assert!(
        !(shell.described.iter()).any(|w| w
            .name
            .as_deref()
            .is_some_and(|n| n.contains("changes the keys"))),
        "the page still counts what a preset changes"
    );
    let kept = crate::state::prefs::load_prefs_from(
        &crate::state::prefs::prefs_path().expect("the test keeps its preferences in a folder"),
    );
    assert_eq!(presets::chosen(&kept).id, presets::OPENRAGE.id);
}
