//! Presets of keys: a named set of bindings a person picks in Settings ▸ Keyboard shortcuts, under the keys they chose themselves.

use egui::Key;

use super::ShortcutMods;
use crate::state::Prefs;
use crate::ui::commands::{Spec, COMMANDS};

/// A named set of bindings: the commands it gives other keys than the table's, and no other.
pub struct Preset {
    /// ⚠ Never renamed: `prefs.json` keeps the choice under it.
    pub id: &'static str,
    pub name: &'static str,
    /// Each command it changes, by id, with the keys it gives it; a command given none is unbound.
    pub keys: &'static [(&'static str, &'static [(ShortcutMods, Key)])],
}

/// OpenRage's own keys: the table's, with no command changed.
pub static OPENRAGE: Preset = Preset {
    id: "openrage",
    name: tr_key!("OpenRage"),
    keys: &[],
};

/// The keys of CodeWalker's world view, for the commands the table has.
static CODEWALKER: Preset = Preset {
    id: "codewalker",
    name: tr_key!("CodeWalker"),
    keys: &[
        // ⚠ There the key makes a project only while none is open, and a map file in the open one otherwise.
        ("create_project", &[(ShortcutMods::ctrl(), Key::N)]),
    ],
};

/// Every preset Settings offers, OpenRage's own first.
pub static PRESETS: [&Preset; 2] = [&OPENRAGE, &CODEWALKER];

/// The preset the preferences name, or OpenRage's own when they name none there is.
pub fn chosen(prefs: &Prefs) -> &'static Preset {
    (PRESETS.iter().copied())
        .find(|preset| preset.id == prefs.shortcut_preset)
        .unwrap_or(&OPENRAGE)
}

/// Makes `preset` the one in use; the keys the person chose themselves stay over it.
pub fn choose(prefs: &mut Prefs, preset: &Preset) {
    prefs.shortcut_preset = preset.id.to_string();
}

/// The keys `preset` gives a command: its own for a command it names, else the table's.
pub fn keys_of(preset: &Preset, spec: &Spec) -> &'static [(ShortcutMods, Key)] {
    (preset.keys.iter())
        .find(|(id, _)| *id == spec.id)
        .map_or(spec.defaults, |(_, keys)| keys)
}

/// What choosing a preset does to the commands of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Changes {
    /// The commands it gives other keys than the table's.
    pub changed: usize,
    /// The commands it names that keep a key the person chose.
    pub kept: usize,
}

/// How many commands `preset` changes under the keys the person chose, and how many it names that keep theirs.
pub fn changes(preset: &Preset, prefs: &Prefs) -> Changes {
    let mut changes = Changes {
        changed: 0,
        kept: 0,
    };
    for spec in COMMANDS {
        if keys_of(preset, spec) == spec.defaults {
            continue;
        }
        match prefs.shortcut_bindings.contains_key(spec.id) {
            true => changes.kept += 1,
            false => changes.changed += 1,
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::ui::commands::{self, Action};
    use crate::ui::shortcuts::{self, Binding};

    const ALT: ShortcutMods = ShortcutMods {
        ctrl: false,
        control: false,
        shift: false,
        alt: true,
    };

    /// The keys every command of the table answers to under `prefs`, by its id.
    fn keys(prefs: &Prefs) -> BTreeMap<&'static str, Vec<Binding>> {
        COMMANDS
            .iter()
            .map(|spec| (spec.id, shortcuts::bindings_of(spec, prefs)))
            .collect()
    }

    fn given(spec: &Spec, keys: &[(ShortcutMods, Key)]) -> Vec<Binding> {
        (keys.iter())
            .map(|&(mods, key)| Binding {
                mods,
                key,
                by_position: spec.by_position,
            })
            .collect()
    }

    fn named_by(preset: &Preset) -> BTreeSet<&'static str> {
        preset.keys.iter().map(|(id, _)| *id).collect()
    }

    #[test]
    fn every_preset_has_an_id_of_its_own_and_names_commands_of_the_table_once() {
        let ids: BTreeSet<&str> = PRESETS.iter().map(|preset| preset.id).collect();
        assert_eq!(ids.len(), PRESETS.len(), "two presets share an id");
        assert_eq!(
            PRESETS[0].id, OPENRAGE.id,
            "OpenRage's own is offered first"
        );
        assert!(OPENRAGE.keys.is_empty(), "OpenRage's own changes a command");
        for preset in PRESETS {
            assert_eq!(
                named_by(preset).len(),
                preset.keys.len(),
                "{} names a command twice",
                preset.id
            );
            for (id, keys) in preset.keys {
                let spec = COMMANDS
                    .iter()
                    .find(|spec| spec.id == *id)
                    .unwrap_or_else(|| panic!("{} names `{id}`, no command", preset.id));
                assert_ne!(
                    *keys, spec.defaults,
                    "{} names `{id}` and gives it the keys it has",
                    preset.id
                );
                for &(mods, key) in *keys {
                    assert!(
                        shortcuts::is_valid_combo(mods, key),
                        "{}: `{id}`",
                        preset.id
                    );
                }
            }
        }
    }

    #[test]
    fn a_preset_changes_the_commands_it_names_and_no_other_and_openrages_own_puts_them_back() {
        let before = keys(&Prefs::default());
        let mut named = 0;
        for preset in PRESETS {
            let mut prefs = Prefs::default();
            choose(&mut prefs, preset);
            assert_eq!(chosen(&prefs).id, preset.id);
            let after = keys(&prefs);
            let changed: BTreeSet<&str> = (before.iter())
                .filter(|(id, keys)| after[*id] != **keys)
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(changed, named_by(preset), "what {} changes", preset.id);
            for (id, keys) in preset.keys {
                let spec = COMMANDS.iter().find(|spec| spec.id == *id).expect("a row");
                assert_eq!(after[id], given(spec, keys), "{}: `{id}`", preset.id);
            }
            let said = changes(preset, &prefs);
            let all = Changes {
                changed: preset.keys.len(),
                kept: 0,
            };
            assert_eq!(said, all, "{}", preset.id);

            choose(&mut prefs, &OPENRAGE);
            assert_eq!(keys(&prefs), before, "leaving {}", preset.id);
            named += preset.keys.len();
        }
        assert!(named > 0, "no preset names a command, so nothing was held");
    }

    #[test]
    fn a_key_the_person_chose_stays_under_a_preset_and_after_it() {
        let grid = commands::spec(Action::ShowGrid).expect("a row");
        for preset in PRESETS.iter().filter(|preset| !preset.keys.is_empty()) {
            let (id, of_preset) = preset.keys[0];
            let named = COMMANDS.iter().find(|spec| spec.id == id).expect("a row");
            let mut prefs = Prefs::default();
            // One command the preset names and one it does not, each given a key of the person's own.
            for (spec, key) in [(named, Key::F7), (grid, Key::F8)] {
                let set = shortcuts::set_binding(&mut prefs, spec.action, ALT, key);
                assert!(set.is_ok(), "{set:?}");
            }
            let theirs = |prefs: &Prefs| {
                (
                    shortcuts::bindings_of(named, prefs),
                    shortcuts::bindings_of(grid, prefs),
                )
            };
            let chose = theirs(&prefs);
            assert_eq!(chose.0, given(named, &[(ALT, Key::F7)]));

            choose(&mut prefs, preset);
            assert_eq!(
                theirs(&prefs),
                chose,
                "{} took a key the person chose",
                preset.id
            );
            assert!(shortcuts::is_overridden(&prefs, named.action));
            let said = changes(preset, &prefs);
            let one_kept = Changes {
                changed: preset.keys.len() - 1,
                kept: 1,
            };
            assert_eq!(said, one_kept, "{}", preset.id);

            choose(&mut prefs, &OPENRAGE);
            assert_eq!(
                theirs(&prefs),
                chose,
                "leaving {} took a key the person chose",
                preset.id
            );

            // Reset under a preset gives the command the preset's key, which is the key it has with nothing chosen.
            choose(&mut prefs, preset);
            shortcuts::reset_binding(&mut prefs, named.action);
            assert_eq!(
                shortcuts::bindings_of(named, &prefs),
                given(named, of_preset)
            );
            assert_eq!(shortcuts::bindings_of(grid, &prefs), chose.1);

            // Restore all defaults leaves neither a key of the person's nor the preset.
            shortcuts::restore_defaults(&mut prefs);
            assert_eq!(keys(&prefs), keys(&Prefs::default()));
            assert!(!shortcuts::has_overrides(&prefs));
        }
    }

    #[test]
    fn a_key_the_person_gave_another_command_runs_that_command_under_a_preset_that_names_the_key() {
        // ⚠ The page does not say so: the preset's command still shows the key, as a table default a person's key covers does.
        let mut held = 0;
        for preset in PRESETS {
            let Some(&(id, &[(mods, key), ..])) = preset.keys.first() else {
                continue;
            };
            let mut prefs = Prefs::default();
            if shortcuts::set_binding(&mut prefs, Action::ShowGrid, mods, key).is_err() {
                continue;
            }
            choose(&mut prefs, preset);
            assert_eq!(
                shortcuts::action_for(mods, key, &prefs),
                Some(Action::ShowGrid),
                "{} took a key the person gave another command than `{id}`",
                preset.id
            );
            held += 1;
        }
        assert!(held > 0, "no preset names a key that is free in the table");
    }

    #[test]
    fn a_key_set_to_what_the_preset_gives_is_no_key_of_the_persons_own() {
        let mut held = 0;
        for preset in PRESETS {
            let Some(&(id, &[(mods, key), ..])) = preset.keys.first() else {
                continue;
            };
            let named = COMMANDS.iter().find(|spec| spec.id == id).expect("a row");
            let mut prefs = Prefs::default();
            choose(&mut prefs, preset);
            let own = shortcuts::set_binding(&mut prefs, named.action, ALT, Key::F7);
            assert!(own.is_ok(), "{own:?}");
            assert!(shortcuts::is_overridden(&prefs, named.action));

            // Set back by hand to the key the preset gives, the command is the preset's again and Reset has nothing to do.
            let back = shortcuts::set_binding(&mut prefs, named.action, mods, key);
            assert!(back.is_ok(), "{back:?}");
            assert!(
                !shortcuts::is_overridden(&prefs, named.action),
                "{}: `{id}` on the preset's key is shown as a key the person chose",
                preset.id
            );
            let all = Changes {
                changed: preset.keys.len(),
                kept: 0,
            };
            assert_eq!(changes(preset, &prefs), all, "{}", preset.id);
            held += 1;
        }
        assert!(held > 0, "no preset gives a command a key");
    }

    #[test]
    fn a_preset_other_than_openrages_own_is_a_change_restore_all_defaults_is_offered_for() {
        for preset in PRESETS {
            let mut prefs = Prefs::default();
            choose(&mut prefs, preset);
            assert_eq!(
                shortcuts::has_overrides(&prefs),
                preset.id != OPENRAGE.id,
                "{}",
                preset.id
            );
        }
    }

    #[test]
    fn no_preset_binds_one_key_to_two_commands_of_one_place() {
        for preset in PRESETS {
            let mut prefs = Prefs::default();
            choose(&mut prefs, preset);
            let found = shortcuts::conflicts_in(COMMANDS, &prefs);
            assert!(
                found.is_empty(),
                "{} gives one key to two commands read in one place: {found:?}",
                preset.id
            );
        }
    }

    #[test]
    fn the_preset_chosen_is_kept_with_the_preferences_and_what_the_keyboard_typed_is_not() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let path = dir.path().join("prefs.json");
        let preset = PRESETS[1];
        let mut prefs = Prefs::default();
        choose(&mut prefs, preset);
        prefs.keys_seen.see(
            &winit::keyboard::Key::Character("'".into()),
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Digit4),
        );
        let written = serde_json::to_string(&prefs).expect("preferences are JSON");
        assert!(!written.contains("keys_seen"), "{written}");
        std::fs::write(&path, written).expect("prefs.json written");
        let read = crate::state::prefs::load_prefs_from(&path);
        assert_eq!(chosen(&read).id, preset.id);

        // A file written before presets, and one that names a preset there no longer is, are OpenRage's own.
        for older in [
            r#"{ "schema": 1 }"#,
            r#"{ "schema": 1, "shortcut_preset": "gone" }"#,
        ] {
            std::fs::write(&path, older).expect("prefs.json written");
            let read = crate::state::prefs::load_prefs_from(&path);
            assert_eq!(chosen(&read).id, OPENRAGE.id, "{older}");
            assert!(!shortcuts::has_overrides(&read), "{older}");
        }
    }
}
