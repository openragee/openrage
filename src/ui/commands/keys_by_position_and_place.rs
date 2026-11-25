//! A command bound by where its key sits answers that key whatever it types, Shift is part of such a binding, and two commands
//! share a key only when the table reads them in two places.

use winit::keyboard::{Key as Typed, KeyCode, ModifiersState, PhysicalKey};

use super::table::{Action, Place, Spec, BASE, COMMANDS};
use crate::state::Prefs;
use crate::ui::shortcuts::{self, KeysSeen, ShortcutMods};

const NONE: ShortcutMods = ShortcutMods::none();
const SHIFT: ShortcutMods = ShortcutMods {
    ctrl: false,
    control: false,
    shift: true,
    alt: false,
};

/// A row that only its keys, its place and how its keys are named tell from another.
const fn row(
    action: Action,
    id: &'static str,
    defaults: &'static [(ShortcutMods, egui::Key)],
    place: Place,
    by_position: bool,
) -> Spec {
    Spec {
        action,
        id,
        defaults,
        place,
        by_position,
        ..BASE
    }
}

/// The command of `table` a press of the key at `code` runs, when the key types `text`.
fn pressed(table: &[Spec], text: &str, code: KeyCode, mods: ModifiersState) -> Option<Action> {
    let press = shortcuts::press_of(
        &Typed::Character(text.into()),
        PhysicalKey::Code(code),
        mods,
    )?;
    shortcuts::command_for(table, press, &Prefs::default(), None).map(|spec| spec.action)
}

/// The fourth key of the digit row, bound each way.
const BY_POSITION: [Spec; 1] = [row(
    Action::ShowGrid,
    "by_position",
    &[(NONE, egui::Key::Num4)],
    Place::App,
    true,
)];
const BY_CHARACTER: [Spec; 1] = [row(
    Action::ShowAxis,
    "by_character",
    &[(NONE, egui::Key::Num4)],
    Place::App,
    false,
)];

#[test]
fn the_fourth_key_of_the_digit_row_answers_by_its_place_on_a_french_keyboard() {
    // AZERTY types an apostrophe on the fourth key of the digit row, and 4 with Shift.
    let none = ModifiersState::empty();
    assert_eq!(
        pressed(&BY_POSITION, "'", KeyCode::Digit4, none),
        Some(Action::ShowGrid),
        "a command bound by where its key sits did not answer the key at that place"
    );
    assert_eq!(
        pressed(&BY_CHARACTER, "'", KeyCode::Digit4, none),
        None,
        "a command bound to the character 4 answered a key that typed an apostrophe"
    );
    // The same two on a US keyboard, where the key types 4: both answer.
    assert_eq!(
        pressed(&BY_POSITION, "4", KeyCode::Digit4, none),
        Some(Action::ShowGrid)
    );
    assert_eq!(
        pressed(&BY_CHARACTER, "4", KeyCode::Digit4, none),
        Some(Action::ShowAxis)
    );
    // The key a US keyboard types an apostrophe with sits elsewhere, and a binding by position does not follow the character there.
    assert_eq!(
        pressed(&BY_POSITION, "'", KeyCode::Quote, none),
        None,
        "a command bound by where its key sits followed the character"
    );
}

#[test]
fn a_digit_with_shift_and_the_digit_alone_are_two_commands_by_position() {
    let table = [
        row(
            Action::ShowGrid,
            "go",
            &[(NONE, egui::Key::Num4)],
            Place::App,
            true,
        ),
        row(
            Action::ShowAxis,
            "set",
            &[(SHIFT, egui::Key::Num4)],
            Place::App,
            true,
        ),
    ];
    let (none, shift) = (ModifiersState::empty(), ModifiersState::SHIFT);
    // A US keyboard types $ with Shift on the key, a French one 4.
    for (bare, shifted) in [("4", "$"), ("'", "4")] {
        assert_eq!(
            pressed(&table, bare, KeyCode::Digit4, none),
            Some(Action::ShowGrid),
            "the key that types {bare:?}"
        );
        assert_eq!(
            pressed(&table, shifted, KeyCode::Digit4, shift),
            Some(Action::ShowAxis),
            "Shift and the key that types {bare:?}"
        );
    }
    assert!(
        shortcuts::conflicts_in(&table, &Prefs::default()).is_empty(),
        "a digit and Shift with it, both by position, were held to be one key"
    );

    // By character Shift is not read for a digit, so the same two bindings are one key.
    let by_character = [
        row(
            Action::ShowGrid,
            "go",
            &[(NONE, egui::Key::Num4)],
            Place::App,
            false,
        ),
        row(
            Action::ShowAxis,
            "set",
            &[(SHIFT, egui::Key::Num4)],
            Place::App,
            false,
        ),
    ];
    assert_eq!(
        pressed(&by_character, "4", KeyCode::Digit4, shift),
        Some(Action::ShowGrid),
        "a binding by character began to read Shift for a digit"
    );
    assert_eq!(
        shortcuts::conflicts_in(&by_character, &Prefs::default()),
        [("go", "set")]
    );
}

/// Two commands on the key F, read where `first` and `second` say.
fn two_on_f(first: Place, second: Place) -> [Spec; 2] {
    [
        row(
            Action::ShowGrid,
            "first",
            &[(NONE, egui::Key::F)],
            first,
            false,
        ),
        row(
            Action::ShowAxis,
            "second",
            &[(NONE, egui::Key::F)],
            second,
            false,
        ),
    ]
}

#[test]
fn two_commands_share_a_key_only_when_the_table_reads_them_in_two_places() {
    let prefs = Prefs::default();
    for (first, second, shared) in [
        (Place::Viewport, Place::TextEditor, true),
        (Place::Lists, Place::Viewport, true),
        (Place::Viewport, Place::Viewport, false),
        (Place::App, Place::App, false),
        // The dispatcher reads every press, so its command meets a panel's.
        (Place::App, Place::Viewport, false),
        (Place::Lists, Place::App, false),
    ] {
        let table = two_on_f(first, second);
        let found = shortcuts::conflicts_in(&table, &prefs);
        assert_eq!(
            found.is_empty(),
            shared,
            "{first:?} and {second:?}: {found:?}"
        );
        // What Settings asks before it writes a key.
        let binding = shortcuts::bindings_of(&table[1], &prefs)[0];
        let taken = shortcuts::taken_by(&table, &prefs, &table[1], binding).map(|spec| spec.id);
        assert_eq!(
            taken,
            (!shared).then_some("first"),
            "{first:?} and {second:?}"
        );
    }
}

#[test]
fn a_press_read_in_a_place_runs_the_command_of_that_place() {
    let table = two_on_f(Place::Viewport, Place::Lists);
    let press = shortcuts::press_of(
        &Typed::Character("f".into()),
        PhysicalKey::Code(KeyCode::KeyF),
        ModifiersState::empty(),
    )
    .expect("F is a key");
    let prefs = Prefs::default();
    let read_in =
        |place| shortcuts::command_for(&table, press, &prefs, Some(place)).map(|spec| spec.id);
    assert_eq!(read_in(Place::Viewport), Some("first"));
    assert_eq!(read_in(Place::Lists), Some("second"));
    assert_eq!(read_in(Place::TextEditor), None);
}

#[test]
fn no_two_commands_of_the_table_read_in_one_place_share_a_key() {
    let found = shortcuts::conflicts_in(COMMANDS, &Prefs::default());
    assert!(
        found.is_empty(),
        "one key runs two commands of one place: {found:?}"
    );
    // The check reads the table: a key the table already gives a command of the dispatcher is found on a second one.
    let mut prefs = Prefs::default();
    shortcuts::force_set_binding(&mut prefs, Action::ShowGrid, NONE, egui::Key::F9);
    assert!(shortcuts::set_binding(&mut prefs, Action::ShowAxis, NONE, egui::Key::F9).is_err());
}

#[test]
fn a_panels_key_is_given_to_a_command_of_another_panel_and_refused_to_one_of_the_dispatcher() {
    // F frames the selection in the viewport, by the table.
    let f = egui::Event::Key {
        key: egui::Key::F,
        physical_key: Some(egui::Key::F),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let mut prefs = Prefs::default();
    let lists = shortcuts::set_binding(&mut prefs, Action::QuickLook, NONE, egui::Key::F);
    assert!(
        lists.is_ok(),
        "the lists' Quick look was refused the viewport's key: {lists:?}"
    );
    assert_eq!(
        shortcuts::action_for_event(Place::Viewport, &f, &prefs),
        Some(Action::FrameSelection)
    );
    assert_eq!(
        shortcuts::action_for_event(Place::Lists, &f, &prefs),
        Some(Action::QuickLook)
    );
    assert_eq!(
        shortcuts::action_for_event(Place::TextEditor, &f, &prefs),
        None
    );

    let app = shortcuts::set_binding(&mut prefs, Action::ShowGrid, NONE, egui::Key::F);
    assert!(
        app.is_err(),
        "a command the dispatcher reads everywhere took a key two panels read"
    );
    let viewport = shortcuts::set_binding(&mut prefs, Action::FrameAll, NONE, egui::Key::F);
    assert_eq!(
        viewport.err().map(|conflict| conflict.existing),
        Some(Action::FrameSelection),
        "a second command of the viewport took a key the viewport already reads"
    );

    // Overwrite, the answer to that refusal, leaves the key to the one command: both panels' commands lose it.
    shortcuts::force_set_binding(&mut prefs, Action::ShowGrid, NONE, egui::Key::F);
    assert_eq!(
        shortcuts::action_for(NONE, egui::Key::F, &prefs),
        Some(Action::ShowGrid)
    );
    assert_eq!(shortcuts::conflicts_in(COMMANDS, &prefs), []);
}

#[test]
fn a_key_bound_by_position_is_named_by_what_it_types_once_a_press_has_shown_it() {
    let prefs = Prefs::default();
    let tool = shortcuts::bindings_of(&BY_POSITION[0], &prefs)[0];
    let set = shortcuts::Binding {
        mods: SHIFT,
        ..tool
    };
    let mut seen = KeysSeen::default();
    assert_eq!(tool.label(&seen), "4 (US keyboard)");
    assert_eq!(set.label(&seen), "Shift+4 (US keyboard)");

    // A French keyboard's key there, as winit names it with every modifier left out.
    seen.see(
        &Typed::Character("'".into()),
        PhysicalKey::Code(KeyCode::Digit4),
    );
    assert_eq!(tool.label(&seen), "'");
    assert_eq!(set.label(&seen), "Shift+'");
    // Its first key types a character egui has no key for, which is shown as typed.
    seen.see(
        &Typed::Character("&".into()),
        PhysicalKey::Code(KeyCode::Digit1),
    );
    let first = shortcuts::Binding {
        key: egui::Key::Num1,
        ..tool
    };
    assert_eq!(first.label(&seen), "&");

    // A binding by character is named as it always was, whatever was pressed.
    let by_character = shortcuts::bindings_of(&BY_CHARACTER[0], &prefs)[0];
    assert_eq!(by_character.label(&seen), "4");
    // A key that types nothing is the same key everywhere.
    let home = shortcuts::Binding {
        key: egui::Key::Home,
        ..tool
    };
    assert_eq!(home.label(&KeysSeen::default()), "Home");
}

/// An egui press of the key at `position` that types `typed`, as a panel reads one.
fn egui_press(typed: egui::Key, position: Option<egui::Key>, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key: typed,
        physical_key: position,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn a_panel_reads_a_key_bound_by_position_from_the_place_its_press_states() {
    let prefs = Prefs::default();
    let read = |table: &[Spec], event: &egui::Event| {
        let press = shortcuts::press_of_event(event)?;
        shortcuts::command_for(table, press, &prefs, Some(Place::Viewport)).map(|spec| spec.id)
    };
    let four = &[(NONE, egui::Key::Num4)];
    let by_position = [row(Action::ShowGrid, "tool", four, Place::Viewport, true)];
    let by_character = [row(Action::ShowAxis, "digit", four, Place::Viewport, false)];

    // A French keyboard's fourth key of the digit row, which types an apostrophe.
    let french = egui_press(egui::Key::Quote, Some(egui::Key::Num4), true);
    assert_eq!(
        read(&by_position, &french),
        Some("tool"),
        "a panel's command bound by where its key sits did not answer the key at that place"
    );
    assert_eq!(read(&by_character, &french), None);
    // The key a US keyboard types an apostrophe with sits elsewhere.
    let us_quote = egui_press(egui::Key::Quote, Some(egui::Key::Quote), true);
    assert_eq!(read(&by_position, &us_quote), None);

    // A press that states no place answers a binding by character alone.
    let unplaced = egui_press(egui::Key::Num4, None, true);
    assert_eq!(read(&by_position, &unplaced), None);
    assert_eq!(read(&by_character, &unplaced), Some("digit"));
    // A release is no press.
    let release = egui_press(egui::Key::Quote, Some(egui::Key::Num4), false);
    assert_eq!(read(&by_position, &release), None);
}

#[test]
fn a_hint_reads_a_key_bound_by_position_as_the_keyboard_in_use_types_it() {
    // What `label_for` hands a hint, a menu and the palette, for a row of the table.
    let mut prefs = Prefs::default();
    assert_eq!(
        shortcuts::labels_of(&BY_POSITION[0], &prefs),
        ["4 (US keyboard)"]
    );
    prefs.keys_seen.see(
        &Typed::Character("'".into()),
        PhysicalKey::Code(KeyCode::Digit4),
    );
    assert_eq!(shortcuts::labels_of(&BY_POSITION[0], &prefs), ["'"]);
    assert_eq!(shortcuts::labels_of(&BY_CHARACTER[0], &prefs), ["4"]);
}

#[test]
fn the_numeric_keypad_does_not_rename_a_key_of_the_digit_row() {
    // egui gives the keypad's 4 the name of the digit row's, and it types 4 on a French keyboard, whose digit row does not.
    let tool = shortcuts::bindings_of(&BY_POSITION[0], &Prefs::default())[0];
    let keypad = |seen: &mut KeysSeen| {
        seen.see(
            &Typed::Character("4".into()),
            PhysicalKey::Code(KeyCode::Numpad4),
        )
    };
    let mut seen = KeysSeen::default();
    keypad(&mut seen);
    assert_eq!(tool.label(&seen), "4 (US keyboard)");
    seen.see(
        &Typed::Character("'".into()),
        PhysicalKey::Code(KeyCode::Digit4),
    );
    keypad(&mut seen);
    assert_eq!(tool.label(&seen), "'");
}

#[test]
fn every_command_a_key_would_be_taken_from_is_named() {
    // F frames the selection in the viewport by the table; the lists' Quick look is given it too.
    let mut prefs = Prefs::default();
    let lists = shortcuts::set_binding(&mut prefs, Action::QuickLook, NONE, egui::Key::F);
    assert!(lists.is_ok(), "{lists:?}");
    let holders = |action| shortcuts::holders(&prefs, action, NONE, egui::Key::F);

    // The dispatcher reads every press, so its command would take the key from both panels.
    let both = holders(Action::ShowGrid);
    assert_eq!(both.len(), 2, "{both:?}");
    for held in [Action::FrameSelection, Action::QuickLook] {
        assert!(both.contains(&held), "{held:?} is not named: {both:?}");
    }
    assert_eq!(holders(Action::FrameAll), [Action::FrameSelection]);
    assert_eq!(holders(Action::TextFind), []);
    assert_eq!(holders(Action::QuickLook), []);
}

#[test]
fn a_shifted_key_by_position_meets_the_character_a_us_keyboard_types_there() {
    let prefs = Prefs::default();
    let shift = ModifiersState::SHIFT;
    let runs = |table: &[Spec], press| {
        shortcuts::command_for(table, press, &prefs, None).map(|spec| spec.id)
    };
    // Each character egui names that a US keyboard types with Shift, and the key it types it on.
    for (text, code) in [
        ("!", KeyCode::Digit1),
        (":", KeyCode::Semicolon),
        ("?", KeyCode::Slash),
        ("+", KeyCode::Equal),
        ("|", KeyCode::Backslash),
        ("{", KeyCode::BracketLeft),
        ("}", KeyCode::BracketRight),
    ] {
        let typed = Typed::Character(text.into());
        let press = shortcuts::press_of(&typed, PhysicalKey::Code(code), shift).expect("a key");
        let place = press.position.expect("a place");
        assert_ne!(press.typed, place, "{text}: egui has no key for it");

        let by_position = |mods| {
            let keys: &'static [_] = Box::leak(Box::new([(mods, place)]));
            row(Action::ShowGrid, "place", keys, Place::App, true)
        };
        let keys: &'static [_] = Box::leak(Box::new([(NONE, press.typed)]));
        let by_character = || row(Action::ShowAxis, "character", keys, Place::App, false);
        // The one press answers both, so the two are one key.
        assert_eq!(runs(&[by_position(SHIFT)], press), Some("place"), "{text}");
        assert_eq!(runs(&[by_character()], press), Some("character"), "{text}");
        assert_eq!(
            shortcuts::conflicts_in(&[by_position(SHIFT), by_character()], &prefs),
            [("place", "character")],
            "{text}: Shift and the key at its place were not held to be the key that types it"
        );
        assert_eq!(
            shortcuts::conflicts_in(&[by_character(), by_position(SHIFT)], &prefs),
            [("character", "place")],
            "{text}"
        );
        // Without Shift the key types another character, and the two are two keys.
        assert_eq!(
            shortcuts::conflicts_in(&[by_position(NONE), by_character()], &prefs),
            [],
            "{text}"
        );
    }
}
