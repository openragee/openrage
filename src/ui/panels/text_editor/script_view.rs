//! A GTA VI script's decompiled source in the text editor, read-only, and the two commands that
//! follow the name under the caret: to its definition, and to every place that spells it.

use ropey::Rope;

use super::editor_state::{load_state, EditorState, FindMode};
use super::highlighters::Language;
use super::{draw_footer, empty_state, find_keys, paint_editor};
use crate::state::{AppState, DocPayload, DocumentId};
use crate::ui::commands::Action;
use crate::ui::panel_state::PanelState;
use crate::ui::shortcuts;

/// The characters of a name read each way from the caret: a longer run is no name.
const LONGEST_NAME: usize = 512;

/// What the script commands ask of the next paint of a script's source.
#[derive(Default)]
pub struct ScriptAsk {
    pub definition: bool,
    pub references: bool,
    /// The documents whose source was written anew, each with the line of it the caret goes to.
    pub landed: Vec<(DocumentId, u32)>,
    /// Whether this session asked to build a project, which it asks once.
    pub project_asked: bool,
    /// Whether the answer was to build: only then is the project of a script's archive built unasked.
    pub project_wanted: bool,
}

impl PanelState for ScriptAsk {
    const KEY: &'static str = "gta6_script_ask";
}

/// A name of the source, as the caret found it.
#[derive(Debug, PartialEq, Eq)]
struct Named {
    word: String,
    /// The name before the dot the word follows, empty where the dot follows no name; `None` for a word that stands alone.
    after_dot: Option<String>,
}

/// The name the character `caret` of `rope` is in or just after.
fn name_at(rope: &Rope, caret: usize) -> Option<Named> {
    let caret = caret.min(rope.len_chars());
    let line_index = rope.char_to_line(caret);
    let line = rope.line(line_index);
    let column = caret - rope.line_to_char(line_index);
    let in_name = |at: usize| (line.get_char(at)).is_some_and(|c| c.is_alphanumeric() || c == '_');
    // The start of the run of name characters that ends at `end`.
    let start_of = |end: usize| {
        let mut start = end;
        while start > 0 && end - start < LONGEST_NAME && in_name(start - 1) {
            start -= 1;
        }
        start
    };
    let start = start_of(column);
    let mut end = column;
    while end - start < 2 * LONGEST_NAME && in_name(end) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let dotted = start > 0 && line.get_char(start - 1) == Some('.');
    let after_dot = dotted.then(|| line.slice(start_of(start - 1)..start - 1).to_string());
    Some(Named {
        word: line.slice(start..end).to_string(),
        after_dot,
    })
}

/// Puts the caret on the first character of line `line`, which the next paint scrolls to the top of the view.
fn jump(state: &mut EditorState, rope: &Rope, line: u32) {
    let line = (line as usize).min(rope.len_lines().saturating_sub(1));
    state.caret.cursor = rope.line_to_char(line);
    state.caret.collapse();
    state.caret.desired_col = None;
    // ⚠ Past every line: the paint scrolls up to the caret's line, which leaves that line at the
    // top, where scrolling down to it would leave a function's head alone at the bottom.
    state.caret.scroll_y = f32::MAX;
    state.revision = state.revision.wrapping_add(1);
}

/// Puts the caret of the editor of `doc_id` on line `line` of `rope`.
pub(crate) fn go_to_line(ctx: &egui::Context, doc_id: DocumentId, rope: &Rope, line: u32) {
    let handle = load_state(ctx, doc_id);
    let Ok(mut state) = handle.lock() else {
        return;
    };
    jump(&mut state, rope, line);
}

/// Takes a source written anew: what was found is of the text before, and the caret goes to line `line`.
fn renewed(state: &mut EditorState, rope: &Rope, line: u32) {
    state.rope_revision = state.rope_revision.wrapping_add(1);
    state.find.invalidate_results();
    state.find.matched_at_revision = u64::MAX;
    jump(state, rope, line);
}

/// Opens the find bar on `pattern`, a regular expression read with its case.
fn find_every(state: &mut EditorState, pattern: String) {
    state.find.visible = true;
    state.find.mode = FindMode::Find;
    state.find.query = pattern;
    state.find.case_sensitive = true;
    state.find.whole_word = false;
    state.find.regex = true;
    state.find.in_selection = false;
    // Whatever was searched last, this is searched now.
    state.find.last_dispatched_query.clear();
    state.find.last_dispatched_flags = u32::MAX;
}

/// Asks once a session, over the first script of an archive decompiled with no project, whether to build one: OpenRage ships none, since one holds for one build of the game.
fn ask_project(
    ctx: &egui::Context,
    app_state: &mut AppState,
    doc_id: DocumentId,
    script: &crate::state::Gta6ScriptDocument,
) {
    let waits = script.read.project.is_none()
        && script.archive.is_some()
        && app_state.gta6_script_job(doc_id).is_none();
    if !waits || app_state.panels.get::<ScriptAsk>().project_asked {
        return;
    }
    let title = tr!("Build the project of this archive?");
    let answer = crate::ui::panels::question(ctx, title, |ui| {
        ui.set_max_width(460.0);
        ui.label(tr!(
            "This script was decompiled alone: a function that needs what another program states has no body. A project reads every program of this archive once, which takes minutes, and keeps what they state of each other in the names folder, where every script opened after it reads it. The inspector can build it later"
        ));
        ui.add_space(crate::ui::theme::SPACING_SNUG);
        ui.horizontal(|ui| {
            if ui.button(tr!("Build project from this archive")).clicked() {
                return Some(true);
            }
            ui.button(tr!("Not now")).clicked().then_some(false)
        })
        .inner
    });
    let answer = answer.or(crate::ui::panels::question_escaped(ctx, title).then_some(false));
    let Some(build) = answer else {
        return;
    };
    let asked = app_state.panels.get::<ScriptAsk>();
    (asked.project_asked, asked.project_wanted) = (true, build);
    if build {
        app_state.build_gta6_project(doc_id);
    }
}

/// A script's source, read-only: OpenRage writes no bytecode back.
pub(super) fn draw(ui: &mut egui::Ui, app_state: &mut AppState, doc_id: DocumentId) {
    let script = match app_state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Gta6Script(script)) => std::sync::Arc::clone(script),
        _ => return empty_state(ui),
    };
    let editor_id = ui.id().with(("virtual_text_editor_v3", doc_id));
    let state_handle = load_state(ui.ctx(), doc_id);
    let Ok(mut state) = state_handle.lock() else {
        return;
    };
    // ⚠ Before anything is drawn: the caret of the text before may lie past the end of this one.
    let landed = &mut app_state.panels.get::<ScriptAsk>().landed;
    if let Some(place) = landed.iter().position(|(doc, _)| *doc == doc_id) {
        let (_, line) = landed.remove(place);
        renewed(&mut state, &script.source, line);
    }
    // A decompile in flight lands on a frame, so frames keep coming while this source waits for one.
    if app_state.gta6_script_job(doc_id).is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    ask_project(ui.ctx(), app_state, doc_id, &script);
    if let Some(doc) = app_state.document(doc_id) {
        draw_footer(ui, doc, &script.source, "UTF-8", "LF", &state.caret, true);
    }
    let body_rect = ui.available_rect_before_wrap();
    // A rope clone shares its nodes, and a read-only paint never edits it.
    let mut source = script.source.clone();
    state.find_keys = find_keys(ui, &app_state.prefs);
    let language = Language::CSharp;
    if state.last_language != language {
        state.last_language = language;
        state.cache.clear();
    }
    state.folds.language = language;
    paint_editor(
        ui,
        &mut state,
        &mut source,
        body_rect,
        editor_id,
        language,
        true,
    );

    // The keys are read here, with the editor's focus, as its find keys are: the commands are the panel's.
    let focused = ui.memory(|m| m.has_focus(editor_id));
    let pressed: Vec<Action> = ui.input(|i| {
        (i.events.iter())
            .filter(|_| focused)
            .filter_map(|event| {
                shortcuts::action_for_event(
                    crate::ui::commands::Place::TextEditor,
                    event,
                    &app_state.prefs,
                )
            })
            .collect()
    });
    // A click with the primary modifier: the paint has already put the caret under the pointer.
    let clicked = focused
        && ui.input(|i| {
            let inside = i
                .pointer
                .interact_pos()
                .is_some_and(|at| body_rect.contains(at));
            i.pointer.primary_clicked() && i.modifiers.command && inside
        });
    let mut asked = std::mem::take(app_state.panels.get::<ScriptAsk>());
    // Another script's source written anew waits for the paint of that source.
    app_state.panels.get::<ScriptAsk>().landed = std::mem::take(&mut asked.landed);
    let definition = asked.definition || clicked || pressed.contains(&Action::ScriptGoToDefinition);
    let references = asked.references || pressed.contains(&Action::ScriptFindReferences);
    if !definition && !references {
        return;
    }
    ui.ctx().request_repaint();
    let Some(named) = name_at(&script.source, state.caret.cursor) else {
        app_state.show_info_toast(tr!("The caret is on no name"));
        return;
    };
    if references {
        let pattern = script.references(&named.word, named.after_dot.as_deref());
        return find_every(&mut state, pattern);
    }
    match (named.after_dot, script.definition(&named.word)) {
        (Some(_), _) => app_state.show_info_toast(tr!(
            "{name} follows a dot: it may be defined in another program, and only a name that stands alone is followed",
            name = named.word
        )),
        (None, Some(row)) => jump(&mut state, &script.source, row.line),
        (None, None) => app_state.show_info_toast(tr!(
            "This script defines no one thing called {name}",
            name = named.word
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(word: &str, after_dot: Option<&str>) -> Option<Named> {
        Some(Named {
            word: word.to_owned(),
            after_dot: after_dot.map(str::to_owned),
        })
    }

    #[test]
    fn the_caret_finds_the_name_it_is_in_or_after_and_what_its_dot_follows() {
        let rope = Rope::from_str("x = 1;\n    other.func_000040(p0).Run(é_1);\n");
        let at = |text: &str| {
            let whole = rope.to_string();
            let byte = whole.find(text).expect("the text is in the source");
            rope.byte_to_char(byte)
        };
        // In a name, at its start and just after its end.
        assert_eq!(
            name_at(&rope, at("0040")),
            named("func_000040", Some("other"))
        );
        assert_eq!(
            name_at(&rope, at("func_")),
            named("func_000040", Some("other"))
        );
        assert_eq!(
            name_at(&rope, at("(p0)")),
            named("func_000040", Some("other"))
        );
        assert_eq!(name_at(&rope, at("other")), named("other", None));
        assert_eq!(name_at(&rope, at("p0")), named("p0", None));
        // A dot after a bracket follows no name, and is still a dot.
        assert_eq!(name_at(&rope, at("Run")), named("Run", Some("")));
        assert_eq!(name_at(&rope, at("é_1")), named("é_1", None));
        assert_eq!(name_at(&rope, 0), named("x", None));
        // Between two things that are no name, at a line's end and past the text.
        assert_eq!(name_at(&rope, at("= 1")), None);
        assert_eq!(name_at(&rope, at(";\n") + 1), None);
        assert_eq!(name_at(&rope, usize::MAX), None);
        assert_eq!(name_at(&Rope::new(), 0), None);
    }

    #[test]
    fn a_jump_lands_on_a_line_of_the_text_and_a_find_is_asked_anew() {
        let rope = Rope::from_str("a\nbb\nccc");
        let mut state = EditorState::default();
        jump(&mut state, &rope, 2);
        assert_eq!((state.caret.cursor, state.caret.anchor), (5, 5));
        assert_eq!(state.revision, 1);
        // Below every line, so that the paint scrolls up to the line and leaves it at the top.
        assert!(state.caret.scroll_y >= f32::MAX);
        // A line past the end is the last line, never a fault.
        jump(&mut state, &rope, u32::MAX);
        assert_eq!(state.caret.cursor, 5);
        jump(&mut state, &Rope::new(), 7);
        assert_eq!(state.caret.cursor, 0);

        state.find.last_dispatched_query = "before".to_owned();
        find_every(&mut state, r"[^.\w](?:main)\b".to_owned());
        assert!(state.find.visible && state.find.regex && state.find.case_sensitive);
        assert!(!state.find.whole_word && !state.find.in_selection);
        assert_eq!(state.find.query, r"[^.\w](?:main)\b");
        assert!(!state.find.searched(state.rope_revision));
    }
}
