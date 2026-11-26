//! What a key bound by where it sits is called: the character it types on the keyboard in use, once a press has shown it,
//! else its place on a US keyboard, said as such.

use std::collections::BTreeMap;

use winit::keyboard::{Key, PhysicalKey};

use super::{format_binding, format_keys, key_label, position_of, shift_picks_the_character};
use super::{Binding, ShortcutMods};
use crate::state::Prefs;
use crate::ui::commands::{self, Action};

/// What each key pressed since the app started types with no modifier held, by where the key sits.
// ⚠ Never kept in `prefs.json`: it is what the keyboard in use today types, which no file knows.
#[derive(Debug, Clone, Default)]
pub struct KeysSeen(BTreeMap<egui::Key, String>);

impl KeysSeen {
    /// Notes what the key of a press types; `unmodified` is the key as winit reads it with every modifier left out.
    pub fn see(&mut self, unmodified: &Key, physical: PhysicalKey) {
        // ⚠ The numeric keypad teaches nothing: its 4 has the digit row's name, and types 4 where the digit row's key does not.
        if matches!(physical, PhysicalKey::Code(code) if format!("{code:?}").starts_with("Numpad"))
        {
            return;
        }
        let (Key::Character(text), Some(position)) = (unmodified, position_of(physical)) else {
            return;
        };
        let typed = match egui::Key::from_name(text) {
            Some(key) => key_label(key).to_string(),
            None => text.to_string(),
        };
        self.0.insert(position, typed);
    }
}

// A key that types no character, as F2 or Home, is the same key on every keyboard.
fn types_a_character(key: egui::Key) -> bool {
    let name = key.name();
    shift_picks_the_character(key)
        || (name.len() == 1 && name.starts_with(|c: char| c.is_ascii_uppercase()))
}

fn position_label(key: egui::Key, seen: &KeysSeen) -> String {
    if !types_a_character(key) {
        return key_label(key).to_string();
    }
    match seen.0.get(&key) {
        Some(typed) => typed.clone(),
        None => tr!("{key} (US keyboard)", key = key_label(key)),
    }
}

impl Binding {
    /// How the binding reads in a menu, a hint and Settings.
    pub fn label(self, seen: &KeysSeen) -> String {
        match self.by_position {
            true => format_keys(self.mods, &position_label(self.key, seen)),
            false => format_binding(self.mods, self.key),
        }
    }
}

/// How `key` with `mods` reads as a binding of `action`, which says whether it names a character or a place.
pub fn format_for(action: Action, mods: ShortcutMods, key: egui::Key, prefs: &Prefs) -> String {
    let binding = Binding {
        mods,
        key,
        by_position: commands::spec(action).is_some_and(|spec| spec.by_position),
    };
    binding.label(&prefs.keys_seen)
}
