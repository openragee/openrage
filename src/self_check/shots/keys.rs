//! A script's key press taken as the window's key handler takes it: the command its winit key
//! names runs, else the key is egui's.

use winit::keyboard::{Key, KeyCode, ModifiersState, NativeKeyCode, PhysicalKey};

use crate::state::AppState;
use crate::ui::commands::{self, Action};
use crate::ui::shortcuts::{self, PRIMARY_STATE};

/// Runs the command the window's dispatcher picks for this press; whether one ran.
pub(super) fn press(
    state: &mut AppState,
    ctx: &egui::Context,
    key: egui::Key,
    held: egui::Modifiers,
) -> bool {
    let Some(action) = command_for(ctx, state, key, held) else {
        return false;
    };
    commands::run_action(state, ctx, action);
    true
}

/// The command the window's dispatcher runs for this press, or `None` when the key is egui's.
fn command_for(
    ctx: &egui::Context,
    state: &AppState,
    key: egui::Key,
    held: egui::Modifiers,
) -> Option<Action> {
    // The window reads whether a field holds the keyboard before egui sees the press.
    let typing = ctx.egui_wants_keyboard_input();
    // `key_of` reads a character back through egui's names, so the key's own name stands for what it types.
    let logical = Key::Character(key.name().into());
    let physical = code_of(key).map_or(
        PhysicalKey::Unidentified(NativeKeyCode::Unidentified),
        PhysicalKey::Code,
    );
    shortcuts::action_for_press(&logical, physical, mods_of(held), typing, &state.prefs)
        .filter(|action| !commands::spec(*action).is_some_and(|spec| spec.panel_key()))
}

/// What winit reports held for a script's modifiers; the script's `ctrl` is the platform's primary modifier.
fn mods_of(held: egui::Modifiers) -> ModifiersState {
    let mut mods = ModifiersState::empty();
    mods.set(PRIMARY_STATE, held.command);
    mods.set(ModifiersState::SHIFT, held.shift);
    mods.set(ModifiersState::ALT, held.alt);
    mods
}

/// Where a key sits on a US keyboard, which the bindings by position read.
fn code_of(key: egui::Key) -> Option<KeyCode> {
    use egui::Key as K;
    use KeyCode as C;
    Some(match key {
        K::A => C::KeyA,
        K::B => C::KeyB,
        K::C => C::KeyC,
        K::D => C::KeyD,
        K::E => C::KeyE,
        K::F => C::KeyF,
        K::G => C::KeyG,
        K::H => C::KeyH,
        K::I => C::KeyI,
        K::J => C::KeyJ,
        K::K => C::KeyK,
        K::L => C::KeyL,
        K::M => C::KeyM,
        K::N => C::KeyN,
        K::O => C::KeyO,
        K::P => C::KeyP,
        K::Q => C::KeyQ,
        K::R => C::KeyR,
        K::S => C::KeyS,
        K::T => C::KeyT,
        K::U => C::KeyU,
        K::V => C::KeyV,
        K::W => C::KeyW,
        K::X => C::KeyX,
        K::Y => C::KeyY,
        K::Z => C::KeyZ,
        K::Num0 => C::Digit0,
        K::Num1 => C::Digit1,
        K::Num2 => C::Digit2,
        K::Num3 => C::Digit3,
        K::Num4 => C::Digit4,
        K::Num5 => C::Digit5,
        K::Num6 => C::Digit6,
        K::Num7 => C::Digit7,
        K::Num8 => C::Digit8,
        K::Num9 => C::Digit9,
        K::F1 => C::F1,
        K::F2 => C::F2,
        K::F3 => C::F3,
        K::F4 => C::F4,
        K::F5 => C::F5,
        K::F6 => C::F6,
        K::F7 => C::F7,
        K::F8 => C::F8,
        K::F9 => C::F9,
        K::F10 => C::F10,
        K::F11 => C::F11,
        K::F12 => C::F12,
        K::ArrowDown => C::ArrowDown,
        K::ArrowLeft => C::ArrowLeft,
        K::ArrowRight => C::ArrowRight,
        K::ArrowUp => C::ArrowUp,
        K::Escape => C::Escape,
        K::Tab => C::Tab,
        K::Backspace => C::Backspace,
        K::Enter => C::Enter,
        K::Space => C::Space,
        K::Insert => C::Insert,
        K::Delete => C::Delete,
        K::Home => C::Home,
        K::End => C::End,
        K::PageUp => C::PageUp,
        K::PageDown => C::PageDown,
        K::Comma => C::Comma,
        K::Period => C::Period,
        K::Semicolon => C::Semicolon,
        K::Backslash => C::Backslash,
        K::Slash => C::Slash,
        K::OpenBracket => C::BracketLeft,
        K::CloseBracket => C::BracketRight,
        K::Backtick => C::Backquote,
        K::Quote => C::Quote,
        K::Minus => C::Minus,
        K::Equals => C::Equal,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::script::{parse, Step};
    use super::*;
    use crate::state::{DocPayload, Document, Prefs, TextDocument};

    fn key_step(line: &str) -> (egui::Key, egui::Modifiers) {
        let steps = parse(line).expect("a valid step");
        match steps[0].2 {
            Step::Key { key, modifiers } => (key, modifiers),
            ref other => panic!("not a key step: {other:?}"),
        }
    }

    #[test]
    fn key_ctrl_s_on_an_edited_document_runs_save() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"as read").expect("the file on disk");
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        let payload = DocPayload::Text(TextDocument::decode(b"as edited"));
        state
            .documents
            .push(Document::new(1, Some(path.clone()), payload));
        state.set_active_document(1);
        state.document_mut(1).expect("just pushed").mark_edited();

        let (key, held) = key_step("key ctrl+s");
        let ctx = egui::Context::default();
        assert_eq!(command_for(&ctx, &state, key, held), Some(Action::Save));
        assert!(press(&mut state, &ctx, key, held), "Ctrl+S ran nothing");
        let deadline = Instant::now() + Duration::from_secs(30);
        while std::fs::read(&path).ok().as_deref() != Some(b"as edited".as_slice()) {
            assert!(Instant::now() < deadline, "the save never reached the file");
            state.poll_save_result();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_plain_key_goes_to_a_focused_field_and_runs_nothing() {
        let mut state = AppState::from_prefs(Prefs::default());
        let ctx = egui::Context::default();
        let none = egui::Modifiers::NONE;
        // A plain key the app binds, which a field holding the keyboard keeps.
        assert_eq!(
            command_for(&ctx, &state, egui::Key::Slash, none),
            Some(Action::FocusHierarchyFilter)
        );
        let mut text = String::new();
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
            });
        }
        assert!(
            ctx.egui_wants_keyboard_input(),
            "the field holds the keyboard"
        );
        for key in [egui::Key::Slash, egui::Key::S] {
            assert!(!press(&mut state, &ctx, key, none), "{key:?} ran a command");
        }
        // Ctrl+S is still the app's while a field holds the keyboard, as in the window.
        let (key, held) = key_step("key S ctrl");
        assert_eq!(command_for(&ctx, &state, key, held), Some(Action::Save));
    }

    #[test]
    fn every_key_with_a_place_names_that_place_back() {
        for code in [
            KeyCode::KeyS,
            KeyCode::Digit4,
            KeyCode::BracketLeft,
            KeyCode::F2,
        ] {
            let key = shortcuts::key_from_str(&format!("{code:?}")).expect("an egui key");
            assert_eq!(code_of(key), Some(code));
        }
    }
}
