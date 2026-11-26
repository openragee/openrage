use winit::keyboard::{Key, ModifiersState, PhysicalKey};

use crate::state::{PersistedBinding, Prefs};
use crate::ui::commands::{self, Action, Place, Spec, COMMANDS};

mod position;
pub mod presets;

pub use position::{format_for, KeysSeen};

/// The modifiers a binding names.
///
/// ⚠ `ctrl` is the platform's primary modifier: Command on a Mac, Control elsewhere. The field keeps
/// its name because `prefs.json` holds it and a plugin writes `Ctrl+K`, so a saved or declared
/// binding means the same thing on every OS; [`primary_modifier_name`] is what a user reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShortcutMods {
    pub ctrl: bool,
    /// The Control key itself on a Mac, where it is not the primary modifier (Ctrl+Tab there is
    /// the native way to switch tabs, and Cmd+Tab is the system's); never set elsewhere.
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}

impl ShortcutMods {
    pub const fn ctrl() -> Self {
        Self {
            ctrl: true,
            control: false,
            shift: false,
            alt: false,
        }
    }
    pub const fn ctrl_shift() -> Self {
        Self {
            ctrl: true,
            control: false,
            shift: true,
            alt: false,
        }
    }
    pub const fn none() -> Self {
        Self {
            ctrl: false,
            control: false,
            shift: false,
            alt: false,
        }
    }
    /// The Control key itself: the primary modifier off a Mac, where it is not.
    pub const fn physical_control() -> Self {
        if cfg!(target_os = "macos") {
            Self {
                ctrl: false,
                control: true,
                shift: false,
                alt: false,
            }
        } else {
            Self::ctrl()
        }
    }

    // ⚠ Shift is ignored for a digit or a punctuation mark: the character
    // already says whether the layout needed it (`.` is Shift+`;` on AZERTY).
    fn accepts(self, held: ShortcutMods, key: egui::Key) -> bool {
        self.ctrl == held.ctrl
            && self.control == held.control
            && self.alt == held.alt
            && (self.shift == held.shift || shift_picks_the_character(key))
    }
}

/// Whether the platform's primary modifier is held: Command on a Mac, Control elsewhere.
fn primary_held(m: ModifiersState, mac: bool) -> bool {
    if mac {
        m.super_key()
    } else {
        m.control_key()
    }
}

impl From<ModifiersState> for ShortcutMods {
    fn from(m: ModifiersState) -> Self {
        Self {
            ctrl: primary_held(m, cfg!(target_os = "macos")),
            control: cfg!(target_os = "macos") && m.control_key(),
            shift: m.shift_key(),
            alt: m.alt_key(),
        }
    }
}

impl From<egui::Modifiers> for ShortcutMods {
    fn from(m: egui::Modifiers) -> Self {
        Self {
            // egui's `command` is Control off a Mac and Command on one.
            ctrl: m.command,
            control: cfg!(target_os = "macos") && m.ctrl,
            shift: m.shift,
            alt: m.alt,
        }
    }
}

/// The primary modifier as winit holds it for a press made outside the window: Command on a Mac.
pub(crate) const PRIMARY_STATE: ModifiersState = if cfg!(target_os = "macos") {
    ModifiersState::SUPER
} else {
    ModifiersState::CONTROL
};

/// The primary modifier as egui reports it for a key press on a real keyboard: `command`, and the
/// physical key beneath it, which `Modifiers::CTRL` alone is not on any platform.
#[cfg(test)]
pub(crate) const PRIMARY_EGUI: egui::Modifiers = egui::Modifiers {
    alt: false,
    ctrl: !cfg!(target_os = "macos"),
    shift: false,
    mac_cmd: cfg!(target_os = "macos"),
    command: true,
};

/// What the primary modifier is called on this platform.
pub fn primary_modifier_name() -> &'static str {
    if cfg!(target_os = "macos") {
        tr!("Cmd")
    } else {
        tr!("Ctrl")
    }
}

fn shift_picks_the_character(key: egui::Key) -> bool {
    use egui::Key as K;
    matches!(
        key,
        K::Num0
            | K::Num1
            | K::Num2
            | K::Num3
            | K::Num4
            | K::Num5
            | K::Num6
            | K::Num7
            | K::Num8
            | K::Num9
            | K::Colon
            | K::Comma
            | K::Backslash
            | K::Slash
            | K::Pipe
            | K::Questionmark
            | K::Exclamationmark
            | K::OpenBracket
            | K::CloseBracket
            | K::OpenCurlyBracket
            | K::CloseCurlyBracket
            | K::Backtick
            | K::Minus
            | K::Period
            | K::Plus
            | K::Equals
            | K::Semicolon
            | K::Quote
    )
}

/// One way to press a command: its modifiers, its key, and how the key is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub mods: ShortcutMods,
    /// The key that types the binding's character, or with `by_position` the key a US keyboard has at that place.
    pub key: egui::Key,
    pub by_position: bool,
}

/// A key press as a binding is held against it.
#[derive(Debug, Clone, Copy)]
pub struct Press {
    pub mods: ShortcutMods,
    /// The key the press types, else the key at its place where the character has no name.
    pub typed: egui::Key,
    /// Where the key sits, as a US keyboard names that place; `None` when the event does not say.
    // ⚠ egui names a digit of the numeric keypad as the digit row's, so the two are one place.
    pub position: Option<egui::Key>,
}

impl Binding {
    fn answers(self, press: Press) -> bool {
        if self.by_position {
            // ⚠ Every modifier counts: where a key sits does not move with Shift, so 4 and Shift+4 are two bindings.
            press.position == Some(self.key) && self.mods == press.mods
        } else {
            self.key == press.typed && self.mods.accepts(press.mods, press.typed)
        }
    }

    /// Whether one press of a US keyboard answers both.
    ///
    /// ⚠ A keyboard whose key at a bound place types another binding's character makes the two meet as well, which only a
    /// table of every layout would show; the matcher then runs the command the person bound, else the first of the table.
    fn collides(self, other: Binding) -> bool {
        let (a, b) = (self.mods, other.mods);
        let shift_is_read = self.by_position && other.by_position;
        let one_key = self.key == other.key
            && (a.shift == b.shift || (!shift_is_read && shift_picks_the_character(self.key)));
        (a.ctrl, a.control, a.alt) == (b.ctrl, b.control, b.alt)
            && (one_key || self.types_with_shift(other) || other.types_with_shift(self))
    }

    /// Whether this binding by position, with its Shift, types on a US keyboard the character `other` is bound to.
    fn types_with_shift(self, other: Binding) -> bool {
        self.by_position
            && !other.by_position
            && self.mods.shift
            && us_shifted(self.key) == Some(other.key)
    }
}

// The character a US keyboard types with Shift on the key at `position`, where egui has a key for that character:
// a binding by character on it answers the same press as a binding by position on Shift and that key.
fn us_shifted(position: egui::Key) -> Option<egui::Key> {
    use egui::Key as K;
    Some(match position {
        K::Num1 => K::Exclamationmark,
        K::Semicolon => K::Colon,
        K::Slash => K::Questionmark,
        K::Equals => K::Plus,
        K::Backslash => K::Pipe,
        K::OpenBracket => K::OpenCurlyBracket,
        K::CloseBracket => K::CloseCurlyBracket,
        _ => return None,
    })
}

fn id_of(action: Action) -> Option<&'static str> {
    commands::spec(action).map(|spec| spec.id)
}

const UNBOUND_SENTINEL: &str = "__none__";

/// The keys a command answers to: the person's own, else the chosen preset's, else the table's.
pub(crate) fn bindings_of(spec: &Spec, prefs: &Prefs) -> Vec<Binding> {
    let bound = |mods, key| Binding {
        mods,
        key,
        by_position: spec.by_position,
    };
    if let Some(persisted) = prefs.shortcut_bindings.get(spec.id) {
        if persisted.key == UNBOUND_SENTINEL {
            return Vec::new();
        }
        if let Some(key) = key_from_str(&persisted.key) {
            let mods = ShortcutMods {
                ctrl: persisted.ctrl,
                control: persisted.control,
                shift: persisted.shift,
                alt: persisted.alt,
            };
            return vec![bound(mods, key)];
        }
        tracing::warn!(
            action = ?spec.action,
            key = %persisted.key,
            "Persisted shortcut binding has unrecognised key; falling back to default"
        );
    }
    let keys = presets::keys_of(presets::chosen(prefs), spec);
    keys.iter().map(|&(mods, key)| bound(mods, key)).collect()
}

/// Where a key sits, as a US keyboard names that place.
fn position_of(physical: PhysicalKey) -> Option<egui::Key> {
    match physical {
        PhysicalKey::Code(code) => key_from_str(&format!("{code:?}")),
        PhysicalKey::Unidentified(_) => None,
    }
}

/// The key a press names: the character it types, else the key's name.
pub fn key_of(logical: &Key, physical: PhysicalKey) -> Option<egui::Key> {
    let by_meaning = match logical {
        Key::Character(text) => egui::Key::from_name(text),
        // winit names a named key as the UI Events spec does, and `from_name` reads those names.
        Key::Named(named) => egui::Key::from_name(&format!("{named:?}")),
        Key::Unidentified(_) | Key::Dead(_) => None,
    };
    // A character no binding can name (Cyrillic, Greek) falls back to where the
    // key sits on a US layout, as egui does, so `Ctrl+S` still saves there.
    by_meaning.or_else(|| position_of(physical))
}

/// The press a winit key event is, or `None` for a key nothing can name.
pub(crate) fn press_of(
    logical: &Key,
    physical: PhysicalKey,
    mods: ModifiersState,
) -> Option<Press> {
    Some(Press {
        mods: mods.into(),
        typed: key_of(logical, physical)?,
        position: position_of(physical),
    })
}

/// The command of `table` a press runs, among those read in `place` when one is given.
pub(crate) fn command_for<'a>(
    table: &'a [Spec],
    press: Press,
    prefs: &Prefs,
    place: Option<Place>,
) -> Option<&'a Spec> {
    let bound = |spec: &&Spec| {
        place.is_none_or(|place| spec.place == place)
            && bindings_of(spec, prefs)
                .iter()
                .any(|binding| binding.answers(press))
    };
    // ⚠ A key the user bound wins over another command's default: a default
    // added later may name a key someone already chose for something else.
    table
        .iter()
        .filter(|spec| prefs.shortcut_bindings.contains_key(spec.id))
        .find(bound)
        .or_else(|| table.iter().find(bound))
}

/// The action a key press runs, or `None` when it belongs to the focused widget or to nothing.
pub fn action_for_press(
    logical: &Key,
    physical: PhysicalKey,
    mods: ModifiersState,
    typing: bool,
    prefs: &Prefs,
) -> Option<Action> {
    let press = press_of(logical, physical, mods)?;
    if typing && the_focused_field_keeps(press.mods, press.typed) {
        return None;
    }
    command_for(COMMANDS, press, prefs, None).map(|spec| spec.action)
}

// A field keeps every key typed without Ctrl or Alt, and the editing keys
// typed with them; F and H are the text editor's find and replace, and
// Ctrl+Shift+F, finding in every open document, is the app's.
fn the_focused_field_keeps(mods: ShortcutMods, key: egui::Key) -> bool {
    use egui::Key as K;
    !(mods.ctrl || mods.control || mods.alt)
        || (key == K::F && !mods.shift)
        || matches!(
            key,
            K::A | K::C
                | K::H
                | K::V
                | K::X
                | K::Y
                | K::Z
                | K::Insert
                | K::Backspace
                | K::Delete
                | K::Home
                | K::End
                | K::PageUp
                | K::PageDown
                | K::ArrowLeft
                | K::ArrowRight
                | K::ArrowUp
                | K::ArrowDown
        )
}

/// The press an egui key event is, or `None` for a release and for any other event.
pub(crate) fn press_of_event(event: &egui::Event) -> Option<Press> {
    let egui::Event::Key {
        key,
        physical_key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    Some(Press {
        mods: (*modifiers).into(),
        typed: *key,
        position: *physical_key,
    })
}

/// The command read in `place` that an egui key press names, for the keys a panel reads itself.
pub fn action_for_event(place: Place, event: &egui::Event, prefs: &Prefs) -> Option<Action> {
    command_for(COMMANDS, press_of_event(event)?, prefs, Some(place)).map(|spec| spec.action)
}

/// The action a key names on a keyboard whose key at each place types what a US keyboard's does.
pub fn action_for(mods: ShortcutMods, key: egui::Key, prefs: &Prefs) -> Option<Action> {
    let press = Press {
        mods,
        typed: key,
        position: Some(key),
    };
    command_for(COMMANDS, press, prefs, None).map(|spec| spec.action)
}

/// The binding a name such as `Ctrl+Shift+K` gives, as a plugin declares one.
pub fn parse_binding(text: &str) -> Option<(ShortcutMods, egui::Key)> {
    let mut mods = ShortcutMods::none();
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" => mods.ctrl = true,
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            _ => {
                if key.replace(key_from_str(part)?).is_some() {
                    return None;
                }
            }
        }
    }
    let key = key?;
    is_valid_combo(mods, key).then_some((mods, key))
}

/// Whether a key pressed with `held` is the binding `bound`.
pub fn binding_matches(
    bound: (ShortcutMods, egui::Key),
    held: ShortcutMods,
    key: egui::Key,
) -> bool {
    bound.1 == key && bound.0.accepts(held, key)
}

/// The plugin and command a key press runs, when no command of the app's own takes it.
pub fn plugin_command_for_press(
    logical: &Key,
    physical: PhysicalKey,
    mods: ModifiersState,
    typing: bool,
    state: &crate::state::AppState,
) -> Option<(String, String)> {
    let key = key_of(logical, physical)?;
    let mods = ShortcutMods::from(mods);
    if typing && the_focused_field_keeps(mods, key) {
        return None;
    }
    state.plugin_command_bound_to(mods, key)
}

/// How each binding of a command reads on the keyboard in use, the one menus show first.
pub(crate) fn labels_of(spec: &Spec, prefs: &Prefs) -> Vec<String> {
    bindings_of(spec, prefs)
        .into_iter()
        .map(|binding| binding.label(&prefs.keys_seen))
        .collect()
}

/// How each of an action's bindings reads, the one menus show first.
pub fn labels_for(action: Action, prefs: &Prefs) -> Vec<String> {
    commands::spec(action).map_or_else(Vec::new, |spec| labels_of(spec, prefs))
}

pub fn label_for(action: Action, prefs: &Prefs) -> Option<String> {
    let labels = labels_for(action, prefs);
    (!labels.is_empty()).then(|| labels.join(" or "))
}

pub fn format_binding(mods: ShortcutMods, key: egui::Key) -> String {
    format_keys(mods, key_label(key))
}

/// The modifiers of a binding, then what its key is called.
fn format_keys(mods: ShortcutMods, key: &str) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(4);
    if mods.control {
        parts.push(tr!("Ctrl"));
    }
    if mods.ctrl {
        parts.push(primary_modifier_name());
    }
    if mods.shift {
        parts.push(tr!("Shift"));
    }
    if mods.alt {
        parts.push(tr!("Alt"));
    }
    parts.push(key);
    parts.join("+")
}

// The arrows by name, and the minus and the quote as the key types them, where
// egui shows symbols or a name.
pub fn key_label(key: egui::Key) -> &'static str {
    use egui::Key as K;
    match key {
        K::ArrowDown | K::ArrowLeft | K::ArrowRight | K::ArrowUp => key.name(),
        K::Minus => "-",
        K::Quote => "'",
        _ => key.symbol_or_name(),
    }
}

// Bindings saved as winit key codes (`KeyZ`, `BracketLeft`) still read.
pub fn key_from_str(s: &str) -> Option<egui::Key> {
    match s {
        "BracketLeft" => Some(egui::Key::OpenBracket),
        "BracketRight" => Some(egui::Key::CloseBracket),
        _ => egui::Key::from_name(s.strip_prefix("Key").unwrap_or(s)),
    }
}

// Tab, Space, Enter and Backspace alone move focus, press buttons and edit text.
pub fn is_valid_combo(mods: ShortcutMods, key: egui::Key) -> bool {
    use egui::Key as K;
    mods.ctrl
        || mods.control
        || mods.shift
        || mods.alt
        || !matches!(key, K::Tab | K::Space | K::Enter | K::Backspace)
}

#[derive(Debug, Clone, Copy)]
pub struct Conflict {
    pub existing: Action,
}

/// Each pair of commands of `table` that one press reaches in one place, by their ids.
#[cfg(test)]
pub(crate) fn conflicts_in(table: &[Spec], prefs: &Prefs) -> Vec<(&'static str, &'static str)> {
    let bound: Vec<(&Spec, Vec<Binding>)> = table
        .iter()
        .map(|spec| (spec, bindings_of(spec, prefs)))
        .collect();
    let mut found = Vec::new();
    for (at, (spec, keys)) in bound.iter().enumerate() {
        for (other, other_keys) in &bound[at + 1..] {
            let meet = spec.place.overlaps(other.place)
                && keys
                    .iter()
                    .any(|a| other_keys.iter().any(|b| a.collides(*b)));
            if meet {
                found.push((spec.id, other.id));
            }
        }
    }
    found
}

/// Each command of `table` that `binding` would take a key from: one read where `spec` is, on a key that meets it.
fn all_taken_by<'a>(
    table: &'a [Spec],
    prefs: &'a Prefs,
    spec: &'a Spec,
    binding: Binding,
) -> impl Iterator<Item = &'a Spec> {
    table.iter().filter(move |other| {
        other.action != spec.action
            && other.place.overlaps(spec.place)
            && bindings_of(other, prefs)
                .iter()
                .any(|held| held.collides(binding))
    })
}

/// The first command of `table` that `binding` would take a key from, which is the one a refusal names.
pub(crate) fn taken_by<'a>(
    table: &'a [Spec],
    prefs: &'a Prefs,
    spec: &'a Spec,
    binding: Binding,
) -> Option<&'a Spec> {
    all_taken_by(table, prefs, spec, binding).next()
}

/// Every command that giving `key` to `action` would take it from: two panels' commands can hold one key.
pub fn holders(prefs: &Prefs, action: Action, mods: ShortcutMods, key: egui::Key) -> Vec<Action> {
    let Some(spec) = commands::spec(action) else {
        return Vec::new();
    };
    all_taken_by(COMMANDS, prefs, spec, binding_for(spec, mods, key))
        .map(|other| other.action)
        .collect()
}

fn binding_for(spec: &Spec, mods: ShortcutMods, key: egui::Key) -> Binding {
    Binding {
        mods,
        key,
        by_position: spec.by_position,
    }
}

/// Binds `key` to a command, refused while another command read in the same place has it.
pub fn set_binding(
    prefs: &mut Prefs,
    action: Action,
    mods: ShortcutMods,
    key: egui::Key,
) -> Result<(), Conflict> {
    let Some(spec) = commands::spec(action) else {
        return Ok(());
    };
    if let Some(existing) = taken_by(COMMANDS, prefs, spec, binding_for(spec, mods, key)) {
        return Err(Conflict {
            existing: existing.action,
        });
    }
    write_binding(prefs, action, mods, key);
    Ok(())
}

/// Binds `key` to a command and unbinds every command of [`holders`], which the caller has named to the person.
pub fn force_set_binding(prefs: &mut Prefs, action: Action, mods: ShortcutMods, key: egui::Key) {
    for existing in holders(prefs, action, mods, key) {
        clear_binding(prefs, existing);
    }
    write_binding(prefs, action, mods, key);
}

pub fn reset_binding(prefs: &mut Prefs, action: Action) {
    if let Some(id) = id_of(action) {
        prefs.shortcut_bindings.remove(id);
    }
}

/// Drops every key the person chose and returns to OpenRage's own preset.
pub fn restore_defaults(prefs: &mut Prefs) {
    prefs.shortcut_bindings.clear();
    prefs.shortcut_preset.clear();
}

/// Whether the keys differ from OpenRage's own: a key the person chose, or another preset.
pub fn has_overrides(prefs: &Prefs) -> bool {
    !prefs.shortcut_bindings.is_empty() || presets::chosen(prefs).id != presets::OPENRAGE.id
}

pub fn is_overridden(prefs: &Prefs, action: Action) -> bool {
    id_of(action).is_some_and(|id| prefs.shortcut_bindings.contains_key(id))
}

fn write_binding(prefs: &mut Prefs, action: Action, mods: ShortcutMods, key: egui::Key) {
    let Some(spec) = commands::spec(action) else {
        return;
    };
    let id = spec.id;
    // The key the command has with nothing of the person's own, which needs no entry.
    if presets::keys_of(presets::chosen(prefs), spec) == [(mods, key)] {
        prefs.shortcut_bindings.remove(id);
        return;
    }
    prefs.shortcut_bindings.insert(
        id.to_string(),
        PersistedBinding {
            ctrl: mods.ctrl,
            control: mods.control,
            shift: mods.shift,
            alt: mods.alt,
            key: key.name().to_string(),
        },
    );
}

pub fn unbind(prefs: &mut Prefs, action: Action) {
    let Some(id) = id_of(action) else {
        return;
    };
    prefs.shortcut_bindings.insert(
        id.to_string(),
        PersistedBinding {
            control: false,
            ctrl: false,
            shift: false,
            alt: false,
            key: UNBOUND_SENTINEL.to_string(),
        },
    );
}

fn clear_binding(prefs: &mut Prefs, action: Action) {
    unbind(prefs, action);
}

#[cfg(test)]
mod tests {
    use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

    use super::*;

    /// The primary modifier of the machine the tests run on.
    const CTRL: ModifiersState = PRIMARY_STATE;

    #[test]
    fn the_primary_modifier_is_command_on_a_mac_and_control_elsewhere() {
        for (held, mac, primary) in [
            (ModifiersState::SUPER, true, true),
            (ModifiersState::CONTROL, true, false),
            (ModifiersState::CONTROL, false, true),
            (ModifiersState::SUPER, false, false),
            (ModifiersState::empty(), true, false),
        ] {
            assert_eq!(primary_held(held, mac), primary, "{held:?} on mac={mac}");
        }
    }

    #[test]
    fn the_key_that_saves_here_is_the_one_the_platform_calls_primary() {
        // ⚠ The real conversion, on whichever OS this runs: a test of `primary_held` alone
        // passes while `From` still reads the Control key.
        let prefs = Prefs::default();
        assert_eq!(
            press("s", KeyCode::KeyS, CTRL, false, &prefs),
            Some(Action::Save)
        );
        let other = if cfg!(target_os = "macos") {
            ModifiersState::CONTROL
        } else {
            ModifiersState::SUPER
        };
        assert_eq!(
            press("s", KeyCode::KeyS, other, false, &prefs),
            None,
            "the other key is not the primary one"
        );
    }

    #[test]
    fn egui_reports_the_primary_modifier_as_command_and_a_binding_follows_it() {
        let held: ShortcutMods = egui::Modifiers::COMMAND.into();
        assert!(
            held.ctrl,
            "egui's command is the primary modifier on every platform"
        );
        let physical_control_on_a_mac = egui::Modifiers {
            ctrl: true,
            ..egui::Modifiers::NONE
        };
        assert_eq!(
            ShortcutMods::from(physical_control_on_a_mac).ctrl,
            cfg!(not(target_os = "macos")) && physical_control_on_a_mac.command
        );
    }

    #[test]
    fn a_binding_is_named_for_the_platforms_primary_modifier() {
        let label = format_binding(ShortcutMods::ctrl(), egui::Key::S);
        assert_eq!(
            label,
            if cfg!(target_os = "macos") {
                "Cmd+S"
            } else {
                "Ctrl+S"
            }
        );
        // Written either way, a plugin's `Ctrl+K` and a user's `Cmd+K` are the same binding.
        assert_eq!(parse_binding("Ctrl+K"), parse_binding("Cmd+K"));
        assert!(parse_binding("Command+K").is_some());
    }

    /// A press of the key at `code` that types `text`.
    fn press(
        text: &str,
        code: KeyCode,
        mods: ModifiersState,
        typing: bool,
        prefs: &Prefs,
    ) -> Option<Action> {
        let logical = Key::Character(text.into());
        action_for_press(&logical, PhysicalKey::Code(code), mods, typing, prefs)
    }

    fn named(key: NamedKey, code: KeyCode, mods: ModifiersState, prefs: &Prefs) -> Option<Action> {
        action_for_press(
            &Key::Named(key),
            PhysicalKey::Code(code),
            mods,
            false,
            prefs,
        )
    }

    /// A binding as `prefs.json` holds it.
    fn saved(prefs: &mut Prefs, action: &str, ctrl: bool, key: &str) {
        prefs.shortcut_bindings.insert(
            action.to_string(),
            PersistedBinding {
                control: false,
                ctrl,
                shift: false,
                alt: false,
                key: key.to_string(),
            },
        );
    }

    #[test]
    fn ctrl_and_the_key_printed_z_undoes_on_a_french_layout() {
        // AZERTY prints Z on the key QWERTY calls W, and W on QWERTY's Z.
        let prefs = Prefs::default();
        assert_eq!(
            press("z", KeyCode::KeyW, CTRL, false, &prefs),
            Some(Action::Undo)
        );
    }

    #[test]
    fn ctrl_and_the_key_printed_w_closes_the_document_on_a_french_layout() {
        let prefs = Prefs::default();
        assert_eq!(
            press("w", KeyCode::KeyZ, CTRL, false, &prefs),
            Some(Action::CloseDocument)
        );
    }

    #[test]
    fn ctrl_s_saves_while_a_text_field_has_focus() {
        let prefs = Prefs::default();
        assert_eq!(
            press("s", KeyCode::KeyS, CTRL, true, &prefs),
            Some(Action::Save)
        );
    }

    #[test]
    fn the_other_ctrl_shortcuts_reach_the_app_while_a_text_field_has_focus() {
        let prefs = Prefs::default();
        let ctrl_shift = CTRL | ModifiersState::SHIFT;
        for (text, code, mods, action) in [
            ("w", KeyCode::KeyW, CTRL, Action::CloseDocument),
            ("o", KeyCode::KeyO, CTRL, Action::Open),
            ("p", KeyCode::KeyP, CTRL, Action::GoToFile),
            ("P", KeyCode::KeyP, ctrl_shift, Action::CommandPalette),
            (",", KeyCode::Comma, CTRL, Action::OpenSettings),
        ] {
            assert_eq!(
                press(text, code, mods, true, &prefs),
                Some(action),
                "{text}"
            );
        }
    }

    #[test]
    fn ctrl_c_stays_with_a_focused_text_field() {
        // Bound here, so that only the field can keep them from the app.
        let mut prefs = Prefs::default();
        saved(&mut prefs, "open_settings", true, "KeyC");
        saved(&mut prefs, "go_to_file", true, "KeyF");
        assert_eq!(
            press("c", KeyCode::KeyC, CTRL, false, &prefs),
            Some(Action::OpenSettings)
        );
        for (text, code) in [
            ("c", KeyCode::KeyC),
            ("f", KeyCode::KeyF),
            ("z", KeyCode::KeyZ),
            ("y", KeyCode::KeyY),
        ] {
            assert_eq!(press(text, code, CTRL, true, &prefs), None, "Ctrl+{text}");
        }
        let redo = press(
            "Z",
            KeyCode::KeyZ,
            CTRL | ModifiersState::SHIFT,
            true,
            &prefs,
        );
        assert_eq!(redo, None);
    }

    #[test]
    fn ctrl_y_redoes_beside_ctrl_shift_z() {
        let prefs = Prefs::default();
        let ctrl_shift = CTRL | ModifiersState::SHIFT;
        assert_eq!(
            press("y", KeyCode::KeyY, CTRL, false, &prefs),
            Some(Action::Redo)
        );
        assert_eq!(
            press("Z", KeyCode::KeyZ, ctrl_shift, false, &prefs),
            Some(Action::Redo)
        );
        assert_eq!(
            press("z", KeyCode::KeyZ, CTRL, false, &prefs),
            Some(Action::Undo)
        );
    }

    #[test]
    fn a_binding_saved_as_a_winit_key_code_still_triggers_its_action() {
        let mut prefs = Prefs::default();
        saved(&mut prefs, "save", true, "KeyQ");
        saved(&mut prefs, "open", true, "Digit1");
        saved(&mut prefs, "open_settings", true, "BracketLeft");
        saved(&mut prefs, "command_palette", false, "F5");
        assert_eq!(
            press("q", KeyCode::KeyQ, CTRL, false, &prefs),
            Some(Action::Save)
        );
        assert_eq!(
            press("1", KeyCode::Digit1, CTRL, false, &prefs),
            Some(Action::Open)
        );
        assert_eq!(
            press("[", KeyCode::BracketLeft, CTRL, false, &prefs),
            Some(Action::OpenSettings)
        );
        let f5 = named(NamedKey::F5, KeyCode::F5, ModifiersState::empty(), &prefs);
        assert_eq!(f5, Some(Action::CommandPalette));
        assert_eq!(
            press("s", KeyCode::KeyS, CTRL, false, &prefs),
            None,
            "the saved binding replaces the default"
        );
    }

    #[test]
    fn a_binding_set_in_settings_reads_back_by_its_character() {
        let mut prefs = Prefs::default();
        let set = set_binding(
            &mut prefs,
            Action::Undo,
            ShortcutMods::ctrl(),
            egui::Key::Questionmark,
        );
        assert!(set.is_ok());
        assert_eq!(
            label_for(Action::Undo, &prefs),
            Some(format!("{}+?", primary_modifier_name()))
        );
        // A US layout types `?` with Shift.
        let shifted = CTRL | ModifiersState::SHIFT;
        assert_eq!(
            press("?", KeyCode::Slash, shifted, false, &prefs),
            Some(Action::Undo)
        );
        assert_eq!(press("z", KeyCode::KeyZ, CTRL, false, &prefs), None);
    }

    #[test]
    fn new_session_and_maximize_are_listed_and_unbound() {
        let prefs = Prefs::default();
        assert_eq!(press("n", KeyCode::KeyN, CTRL, false, &prefs), None);
        assert_eq!(named(NamedKey::Space, KeyCode::Space, CTRL, &prefs), None);
        for action in [Action::NewProject, Action::ToggleMaximizePanel] {
            assert!(
                COMMANDS.iter().any(|spec| spec.action == action),
                "{action:?} left the table"
            );
            assert_eq!(label_for(action, &prefs), None, "{action:?}");
        }
    }

    #[test]
    fn a_punctuation_shortcut_follows_its_character_whichever_shift_it_takes() {
        // AZERTY types `.` with Shift on the key QWERTY calls Comma, and `/`
        // with Shift on QWERTY's Period.
        let prefs = Prefs::default();
        let shift = ModifiersState::SHIFT;
        assert_eq!(
            press(".", KeyCode::Comma, shift, false, &prefs),
            Some(Action::FrameSelection)
        );
        assert_eq!(
            press("/", KeyCode::Period, shift, false, &prefs),
            Some(Action::FocusHierarchyFilter)
        );
    }

    #[test]
    fn a_layout_without_latin_letters_keeps_the_shortcuts_where_a_us_layout_has_them() {
        // Russian ЙЦУКЕН types ы on the key QWERTY calls S.
        let prefs = Prefs::default();
        assert_eq!(
            press("ы", KeyCode::KeyS, CTRL, false, &prefs),
            Some(Action::Save)
        );
    }

    #[test]
    fn ctrl_shift_f_finds_in_every_open_document_while_a_text_field_has_focus() {
        let prefs = Prefs::default();
        let ctrl_shift = CTRL | ModifiersState::SHIFT;
        assert_eq!(
            press("F", KeyCode::KeyF, ctrl_shift, true, &prefs),
            Some(Action::TextFindInOpenDocs)
        );
        assert_eq!(
            press("f", KeyCode::KeyF, CTRL, true, &prefs),
            None,
            "Ctrl+F is the text editor's own find"
        );
    }

    /// Every binding a user could save before the command registry, each
    /// under its action's id, as `prefs.json` wrote it.
    const SAVED_BEFORE_THE_REGISTRY: &str = r#"{
        "schema": 1,
        "shortcut_bindings": {
            "new_project": { "ctrl": true, "shift": false, "alt": false, "key": "N" },
            "open": { "ctrl": true, "shift": true, "alt": false, "key": "O" },
            "save": { "ctrl": true, "shift": false, "alt": false, "key": "KeyQ" },
            "save_as": { "ctrl": false, "shift": false, "alt": true, "key": "S" },
            "close_document": { "ctrl": false, "shift": false, "alt": false, "key": "__none__" },
            "undo": { "ctrl": true, "shift": false, "alt": false, "key": "U" },
            "redo": { "ctrl": true, "shift": false, "alt": false, "key": "R" },
            "command_palette": { "ctrl": false, "shift": false, "alt": false, "key": "F1" },
            "go_to_file": { "ctrl": true, "shift": false, "alt": false, "key": "E" },
            "toggle_maximize_panel": { "ctrl": true, "shift": false, "alt": false, "key": "M" },
            "frame_all": { "ctrl": false, "shift": false, "alt": false, "key": "KeyG" },
            "frame_selection": { "ctrl": false, "shift": true, "alt": false, "key": "F" },
            "focus_hierarchy_filter": { "ctrl": true, "shift": false, "alt": false, "key": "K" },
            "open_settings": { "ctrl": true, "shift": false, "alt": false, "key": "BracketLeft" }
        }
    }"#;

    #[test]
    fn a_binding_saved_before_the_registry_still_binds() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let path = dir.path().join("prefs.json");
        std::fs::write(&path, SAVED_BEFORE_THE_REGISTRY).expect("prefs.json written");
        let prefs = crate::state::prefs::load_prefs_from(&path);
        assert_eq!(prefs.shortcut_bindings.len(), 14, "the file did not read");

        for (id, shown) in [
            ("new_project", "Ctrl+N"),
            ("open", "Ctrl+Shift+O"),
            ("save", "Ctrl+Q"),
            ("save_as", "Alt+S"),
            ("close_document", ""),
            ("undo", "Ctrl+U"),
            ("redo", "Ctrl+R"),
            ("command_palette", "F1"),
            ("go_to_file", "Ctrl+E"),
            ("toggle_maximize_panel", "Ctrl+M"),
            ("frame_all", "G"),
            ("frame_selection", "Shift+F"),
            ("focus_hierarchy_filter", "Ctrl+K"),
            ("open_settings", "Ctrl+["),
        ] {
            let command = COMMANDS
                .iter()
                .find(|spec| spec.id == id)
                .unwrap_or_else(|| panic!("no command keeps the id `{id}`"));
            assert_eq!(
                label_for(command.action, &prefs).unwrap_or_default(),
                shown.replace("Ctrl", primary_modifier_name()),
                "`{id}` lost the binding the user saved"
            );
        }
        assert_eq!(
            press("n", KeyCode::KeyN, CTRL, false, &prefs),
            Some(Action::NewProject)
        );
        assert_eq!(press("w", KeyCode::KeyW, CTRL, false, &prefs), None);
    }
}
