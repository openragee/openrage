//! A GTA VI script in the outline and in the inspector: the classes and functions its source
//! lists, drawn a window at a time, and under the facts of its program the instructions of the
//! function picked.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use or_io::gta6::script::program::FunctionRow;
use or_io::gta6::script::{disasm, opcode, Level};

use super::*;
use crate::state::capabilities::FormatCapabilities;
use crate::state::gta6_script::{names_dir, set_level};
use crate::state::Gta6ScriptDocument;
use crate::ui::panels::text_editor::script_view;

/// The script's row of the capability table.
pub(crate) const CAPABILITIES: FormatCapabilities = FormatCapabilities {
    selection_summary: Some(summary),
    outline: Some(draw_rows),
    has_outline_children: Some(has_rows),
    inspector: Some(draw_facts),
    ..FormatCapabilities::none(DocumentKind::Gta6Script)
};

/// What the outline keeps between frames.
#[derive(Clone, Default)]
struct Held {
    /// The text a name must hold for its row to be drawn.
    filter: String,
    /// The classes whose members are folded away, by the line of their head.
    closed: BTreeSet<u32>,
}

/// What a decompile at another level comes to, once its thread is done.
type Releveled = Arc<Mutex<Option<DocPayload>>>;

/// How many functions the source lists.
fn whole(doc: &Gta6ScriptDocument) -> String {
    let functions = doc.outline.iter().filter(|row| !row.class).count();
    tr_n!("{count} function", "{count} functions", functions)
}

/// The status line of the row picked, else how many functions the source lists.
fn summary(payload: &DocPayload, sel: u64) -> String {
    let DocPayload::Gta6Script(doc) = payload else {
        return String::new();
    };
    let picked = usize::try_from(sel)
        .ok()
        .and_then(|row| doc.outline.get(row));
    match picked {
        Some(row) => tr_n!(
            "{name}: {count} line from line {line}",
            "{name}: {count} lines from line {line}",
            row.lines,
            name = row.name,
            line = row.line + 1
        ),
        None => whole(doc),
    }
}

/// Whether the source lists a class or a function.
fn has_rows(payload: &DocPayload) -> bool {
    matches!(payload, DocPayload::Gta6Script(doc) if !doc.outline.is_empty())
}

/// Whether `name` holds `filter`, which is in lower case, whatever the case of its ASCII letters.
fn holds(name: &str, filter: &str) -> bool {
    let (name, filter) = (name.as_bytes(), filter.as_bytes());
    filter.is_empty() || (name.windows(filter.len())).any(|at| at.eq_ignore_ascii_case(filter))
}

/// The rows drawn, by their place in the outline: those whose name holds the filter, else all but the members of a folded class; `None` for every row.
// ponytail: scans every row each frame while a filter is typed or a class is folded; keep the
// list between frames if a program of tens of thousands of functions is slow.
fn shown(doc: &Gta6ScriptDocument, held: &Held) -> Option<Vec<u32>> {
    let rows = || (0u32..).zip(&doc.outline);
    let filter = held.filter.trim().to_ascii_lowercase();
    if !filter.is_empty() {
        let kept = rows().filter(|(_, row)| holds(&row.name, &filter));
        return Some(kept.map(|(index, _)| index).collect());
    }
    if held.closed.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    // The depth of the folded class the rows read are under.
    let mut folded: Option<u8> = None;
    for (index, row) in rows() {
        if folded.is_some_and(|depth| row.depth > depth) {
            continue;
        }
        folded = (row.class && held.closed.contains(&row.line)).then_some(row.depth);
        out.push(index);
    }
    Some(out)
}

/// A filter over names, then a row per class and function, only those in view: a program lists
/// tens of thousands. A click selects the row and puts the caret of the source on its head.
fn draw_rows(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Gta6Script(doc)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let doc = Arc::clone(doc);
    let id = ui.make_persistent_id(("hierarchy_gta6_script", doc_id));
    let mut held: Held = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let classes = doc.outline.iter().filter(|row| row.class).count();
    let mut counted = whole(&doc);
    if classes > 0 {
        let classes = tr_n!("{count} class", "{count} classes", classes);
        counted = format!("{classes}, {counted}");
    }
    ui.label(egui::RichText::new(counted).color(theme::TEXT_SECONDARY));
    ui.add(egui::TextEdit::singleline(&mut held.filter).hint_text(tr!("Filter")));
    ui.add_space(theme::SPACING_HAIR);

    let shown = shown(&doc, &held);
    let count = shown.as_ref().map_or(doc.outline.len(), Vec::len);
    let row_height = ui.spacing().interact_size.y;
    let (mut clicked, mut folded) = (None, None);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        let clip = ui.clip_rect();
        let row_at = |y: f32| (((y - top) / row_height).max(0.0) as usize).min(count);
        let (first, end) = (row_at(clip.top()), (row_at(clip.bottom()) + 1).min(count));
        let first = first.min(end);
        ui.add_space(first as f32 * row_height);
        for place in first..end {
            let index = match &shown {
                Some(rows) => rows.get(place).map_or(usize::MAX, |index| *index as usize),
                None => place,
            };
            let Some(row) = doc.outline.get(index) else {
                continue;
            };
            let selected = state.selection.is_selected(doc_id, index as u64);
            ui.horizontal(|ui| {
                ui.set_height(row_height);
                ui.add_space(f32::from(row.depth) * ui.spacing().indent);
                if row.class {
                    let glyph = match held.closed.contains(&row.line) {
                        true => ph::CARET_RIGHT,
                        false => ph::CARET_DOWN,
                    };
                    if ui.small_button(glyph).clicked() {
                        folded = Some(row.line);
                    }
                }
                if ui.selectable_label(selected, row.name.as_str()).clicked() {
                    clicked = Some(index);
                }
                if let Some(offset) = row.at {
                    ui.add(detail_label(format!("0x{offset:06X}")));
                }
            });
        }
        ui.add_space((count - end) as f32 * row_height);
    });
    if let Some(line) = folded {
        if !held.closed.remove(&line) {
            held.closed.insert(line);
        }
    }
    ui.data_mut(|d| d.insert_temp(id, held));
    if let Some(index) = clicked {
        state.selection.select(doc_id, index as u64);
        if let Some(row) = doc.outline.get(index) {
            script_view::go_to_line(ui.ctx(), doc_id, &doc.source, row.line);
        }
    }
}

/// The offset and the name of each instruction of `function`, a line each.
fn instructions(doc: &Gta6ScriptDocument, function: &FunctionRow) -> Vec<String> {
    let line = |insn: &disasm::Insn| {
        let name = opcode::name(insn.opcode).unwrap_or("?");
        format!("{:06X}  {name}", insn.at)
    };
    match disasm::function(&doc.program, function) {
        Ok(insns) => insns.iter().map(line).collect(),
        Err(e) => vec![tr!(
            "The instructions of this function do not decode: {e}",
            e = e
        )],
    }
}

/// The instructions of the function `selection` picks, drawn a window at a time.
// ponytail: decoded on the frame that first shows them and kept; a worker if a function of tens
// of thousands of instructions stalls that frame.
fn draw_listing(
    ui: &mut egui::Ui,
    doc_id: DocumentId,
    doc: &Gta6ScriptDocument,
    selection: Option<u64>,
) {
    let picked = selection
        .and_then(|sel| usize::try_from(sel).ok())
        .and_then(|row| doc.outline.get(row));
    let function = picked
        .and_then(|row| row.at)
        .and_then(|at| doc.program.function(at).copied());
    let (Some(row), Some(function)) = (picked, function) else {
        let hint = tr!("Select a function in the outline to list its instructions");
        ui.label(egui::RichText::new(hint).color(theme::TEXT_SECONDARY));
        return;
    };
    let id = egui::Id::new(("gta6_script_listing", doc_id));
    let kept: Option<(u32, Arc<Vec<String>>)> = ui.data(|d| d.get_temp(id));
    let lines = match kept {
        Some((at, lines)) if at == function.at => lines,
        _ => {
            let lines = Arc::new(instructions(doc, &function));
            ui.data_mut(|d| d.insert_temp(id, (function.at, Arc::clone(&lines))));
            lines
        }
    };
    ui.strong(row.name.as_str());
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both()
        .id_salt(("gta6_script_listing", doc_id))
        .auto_shrink([false, false])
        .show_rows(ui, row_height, lines.len(), |ui, rows| {
            for line in &lines[rows] {
                ui.add(egui::Label::new(egui::RichText::new(line).monospace()).extend());
            }
        });
}

/// The level the source is written at, which a choice decompiles the script again at on a thread of its own.
fn draw_level(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    doc: &Arc<Gta6ScriptDocument>,
) {
    let id = egui::Id::new(("gta6_script_level", doc_id));
    let running: Option<Releveled> = ui.data(|d| d.get_temp(id));
    if let Some(slot) = running {
        let done = slot.lock().ok().and_then(|mut slot| slot.take());
        match done {
            Some(payload) => {
                ui.data_mut(|d| d.remove::<Releveled>(id));
                // The rows of the new source are other rows.
                state.selection.select(doc_id, DOC_ROOT_SEL);
                state.replace_gta6_script(doc_id, payload);
            }
            None => {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(150));
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr!("Decompiling again…"));
                });
            }
        }
        return;
    }
    let mut level = doc.level;
    ui.horizontal(|ui| {
        ui.label(tr!("Level"));
        egui::ComboBox::from_id_salt(("gta6_script_level_choice", doc_id))
            .selected_text(level.name())
            .show_ui(ui, |ui| {
                for choice in Level::ALL {
                    ui.selectable_value(&mut level, choice, choice.name());
                }
            });
    });
    if level != doc.level {
        set_level(level);
        let slot = Releveled::default();
        let (done, doc) = (Arc::clone(&slot), Arc::clone(doc));
        std::thread::spawn(move || {
            let payload = doc.at_level(level);
            if let Ok(mut done) = done.lock() {
                *done = Some(payload);
            }
        });
        ui.data_mut(|d| d.insert_temp(id, slot));
    }
}

/// The program's facts, the lists that named things, the level, then the instructions of the function picked.
fn draw_facts(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId, selection: Option<u64>) {
    let Some(DocPayload::Gta6Script(doc)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let doc = Arc::clone(doc);
    let dim = |text: String| egui::RichText::new(text).color(theme::TEXT_SECONDARY);
    ui.heading(format!("0x{:08X}", doc.program.program));
    ui.label(whole(&doc));
    if doc.stats.incomplete > 0 {
        ui.label(tr_n!(
            "{count} function has no body in the source",
            "{count} functions have no body in the source",
            doc.stats.incomplete
        ));
    }
    if !doc.program.imports.is_empty() {
        let imports: Vec<String> = (doc.program.imports.iter())
            .map(|import| format!("0x{:08X}", import.program))
            .collect();
        ui.label(tr_n!(
            "{count} imported program",
            "{count} imported programs",
            imports.len()
        ));
        ui.label(dim(imports.join(", ")));
    }
    ui.separator();
    let natives = match &doc.natives {
        Some(list) => tr!("Natives are named from {list}", list = list),
        None => tr!("No natives list was supplied, so a native is shown as the program spells it")
            .to_string(),
    };
    let names = match doc.lists.is_empty() {
        false => tr!("Names are read from {lists}", lists = doc.lists.join(", ")),
        true => tr!(
            "No names list was supplied, so a type, a field or a method is shown by its hash or its offset"
        )
        .to_string(),
    };
    ui.label(dim(natives));
    ui.label(dim(names));
    let open = ui.button(tr!("Open names folder")).on_hover_text(tr!(
        "Put natives lists and names lists in this folder, then open the script again: OpenRage ships none. A file whose name starts with natives is read as a natives list, any other as a names list"
    ));
    if open.clicked() {
        if let Some(dir) = names_dir() {
            crate::platform::shell::open_folder(&dir);
        }
    }
    draw_level(ui, state, doc_id, &doc);
    ui.separator();
    draw_listing(ui, doc_id, &doc, selection);
}

#[cfg(test)]
mod tests {
    use or_io::gta6::script::program::Program;

    use super::*;

    /// A source written by hand: a class of two methods, then a function; nothing of a game.
    const SOURCE: &str = "class Demo\n{\n    void Run()\n    {\n    }\n    void Stop()\n    {\n    }\n}\n\nvoid func_000010()\n{\n}\n";

    fn document() -> Arc<Gta6ScriptDocument> {
        Arc::new(Gta6ScriptDocument::of(
            Program::default(),
            SOURCE,
            Level::Source,
        ))
    }

    #[test]
    fn the_row_of_the_capability_table_lists_the_rows_of_a_source_and_says_what_is_picked() {
        let doc = document();
        let payload = DocPayload::Gta6Script(Arc::clone(&doc));
        assert_eq!(payload.kind(), DocumentKind::Gta6Script);
        assert!(has_rows(&payload));
        let empty = Gta6ScriptDocument::of(Program::default(), "", Level::Source);
        assert!(!has_rows(&DocPayload::Gta6Script(Arc::new(empty))));
        // Three functions, the class not among them; a row says its own lines.
        assert_eq!(summary(&payload, u64::MAX), whole(&doc));
        assert!(whole(&doc).contains('3'), "{}", whole(&doc));
        let picked = summary(&payload, 0);
        assert!(picked.contains("Demo") && picked.contains('9'), "{picked}");
        assert_ne!(summary(&payload, 1), picked);
        let row = crate::state::capabilities::for_kind(DocumentKind::Gta6Script);
        let row = row.expect("the script's row");
        assert!(row.outline.is_some() && row.inspector.is_some());
    }

    #[test]
    fn a_filter_keeps_the_names_that_hold_it_and_a_folded_class_hides_its_members_alone() {
        let doc = document();
        let mut held = Held::default();
        assert_eq!(shown(&doc, &held), None);
        held.filter = " sTOp ".to_owned();
        assert_eq!(shown(&doc, &held), Some(vec![2]));
        held.filter = "nothing".to_owned();
        assert_eq!(shown(&doc, &held), Some(Vec::new()));
        // The class at line 0 folded: its two methods go, and the function after it stays.
        held.filter.clear();
        held.closed.insert(0);
        assert_eq!(shown(&doc, &held), Some(vec![0, 3]));
        // A filter finds a member of a folded class.
        held.filter = "run".to_owned();
        assert_eq!(shown(&doc, &held), Some(vec![1]));
        assert!(holds("Func_00AB", "func_00ab") && holds("x", ""));
        assert!(!holds("ab", "abc"));
    }

    #[test]
    fn the_outline_draws_a_row_per_head_and_the_centre_the_facts_of_the_program() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::default();
        let id = state.new_empty_document(DocumentKind::Text);
        let d = state.document_mut(id).expect("the blank document");
        d.payload = DocPayload::Gta6Script(document());
        d.kind = DocumentKind::Gta6Script;
        let painted = |state: &mut AppState, outline: bool| {
            let output = egui::Context::default().run_ui(Default::default(), |ui| {
                if outline {
                    draw_rows(ui, state, id);
                } else {
                    draw_facts(ui, state, id, Some(3));
                }
            });
            format!("{:?}", output.shapes)
        };
        let outline = painted(&mut state, true);
        assert!(outline.contains("func_000010"), "{outline}");
        assert!(outline.contains("Stop"), "{outline}");
        let centre = painted(&mut state, false);
        assert!(centre.contains("3 functions"), "{centre}");
        // The program built here has no code, so the function picked has no instruction to list.
        assert!(centre.contains("Select a function"), "{centre}");
    }
}
