//! The commands of a GTA VI script's source, which follow the name under the caret.

use egui::Key;

use super::table::{none_if, Action, Spec, BASE};
use crate::state::{AppState, DocPayload};
use crate::ui::panels::text_editor::script_view::ScriptAsk;
use crate::ui::shortcuts::ShortcutMods;

const NONE: ShortcutMods = ShortcutMods::none();
const SHIFT: ShortcutMods = ShortcutMods {
    ctrl: false,
    control: false,
    shift: true,
    alt: false,
};

fn a_script_in_front(state: &AppState) -> Result<(), &'static str> {
    let script =
        (state.active_document()).is_some_and(|d| matches!(d.payload, DocPayload::Gta6Script(_)));
    none_if(!script, tr!("No GTA VI script is in front"))
}

pub(super) const GO_TO_DEFINITION: Spec = Spec {
    action: Action::ScriptGoToDefinition,
    id: "script_go_to_definition",
    label: tr_key!("Go to definition"),
    category: tr_key!("Text"),
    defaults: &[(NONE, Key::F12)],
    hover: tr_key!("Move the caret to where this script defines the name under it"),
    enabled: a_script_in_front,
    panel_key: true,
    run: |state, _| state.panels.get::<ScriptAsk>().definition = true,
    ..BASE
};

pub(super) const FIND_REFERENCES: Spec = Spec {
    action: Action::ScriptFindReferences,
    id: "script_find_references",
    label: tr_key!("Find references"),
    category: tr_key!("Text"),
    defaults: &[(SHIFT, Key::F12)],
    hover: tr_key!("Find every place of this script that spells the name under the caret"),
    enabled: a_script_in_front,
    panel_key: true,
    run: |state, _| state.panels.get::<ScriptAsk>().references = true,
    ..BASE
};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use or_io::gta6::script::program::Program;
    use or_io::gta6::script::Level;

    use super::*;
    use crate::state::{DocumentKind, Gta6ScriptDocument};
    use crate::ui::shortcuts;

    #[test]
    fn the_two_commands_are_offered_over_a_script_alone_and_their_keys_ask_the_source() {
        let mut state = AppState::default();
        let id = state.new_empty_document(DocumentKind::Text);
        assert!(a_script_in_front(&state).is_err());
        let script =
            Gta6ScriptDocument::of(Program::default(), "void main()\n{\n}\n", Level::Source);
        let d = state.document_mut(id).expect("the blank document");
        d.payload = DocPayload::Gta6Script(Arc::new(script));
        d.kind = DocumentKind::Gta6Script;
        assert!(a_script_in_front(&state).is_ok());

        let pressed = |mods| shortcuts::action_for(mods, Key::F12, &state.prefs);
        assert_eq!(pressed(NONE), Some(Action::ScriptGoToDefinition));
        assert_eq!(pressed(SHIFT), Some(Action::ScriptFindReferences));
        let ctx = egui::Context::default();
        (GO_TO_DEFINITION.run)(&mut state, &ctx);
        let asked = state.panels.get::<ScriptAsk>();
        assert!(asked.definition && !asked.references);
        (FIND_REFERENCES.run)(&mut state, &ctx);
        assert!(state.panels.get::<ScriptAsk>().references);
    }
}
