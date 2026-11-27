//! A GTA VI script in the outline and in the inspector: the classes and functions its source
//! lists, drawn a window at a time, and under the facts of its program the instructions of the
//! function picked.

use std::collections::BTreeSet;
use std::sync::Arc;

use or_io::gta6::script::program::FunctionRow;
use or_io::gta6::script::{decode, opcode, Level};

use super::*;
use crate::state::capabilities::FormatCapabilities;
use crate::state::gta6_script::{labels_on, names_dir, set_labels, Ask};
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

/// A filter over names, then a row per class and function; a click selects the row and puts the caret of the source on its head.
// Only the rows in view are drawn: a program lists tens of thousands.
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
            // A function the source of a large program leaves out is decompiled by the click that picks it.
            let left_out = |at: &u32| (doc.picked.as_ref()).is_some_and(|held| !held.contains(at));
            if let Some(at) = row.at.filter(left_out).filter(|_| !row.class) {
                state.pick_gta6_function(doc_id, at);
            }
        }
    }
}

/// The offset and the name of each instruction of `function`, a line each.
fn instructions(doc: &Gta6ScriptDocument, function: &FunctionRow) -> Vec<String> {
    let line = |insn: &decode::Insn| {
        let name = opcode::name(insn.opcode).unwrap_or("?");
        format!("{:06X}  {name}", insn.at)
    };
    match decode::function(&doc.program, function) {
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

/// The words a level is chosen by.
fn level_label(level: Level) -> &'static str {
    match level {
        Level::Lifted => tr!("Lifted listing"),
        Level::Structured => tr!("Structured statements"),
        Level::Source => tr!("Source with classes"),
    }
}

/// The decompile in flight with a way to stop waiting for it, else the level of the source, which a choice decompiles the script again at on a thread of its own.
fn draw_level(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    doc: &Gta6ScriptDocument,
) {
    if let Some(job) = state.gta6_script_job(doc_id) {
        let seconds = job.started.elapsed().as_secs();
        let done = job.progress.done();
        let functions = match &job.ask.picked {
            Some(picked) => picked.len(),
            None => doc.program.functions.len(),
        };
        let building = job.building.as_ref().filter(|building| !building.built());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
        let mut stop = false;
        ui.horizontal(|ui| {
            ui.spinner();
            let Some(building) = building else {
                ui.label(tr!(
                    "Decompiling, {seconds} s so far; functions written: {done} of {functions}",
                    seconds = seconds,
                    done = done,
                    functions = functions
                ));
                stop = (ui.button(tr!("Stop decompiling")))
                    .on_hover_text(tr!(
                        "The decompile stops after the function it is writing, and the script keeps the source it has"
                    ))
                    .clicked();
                return;
            };
            ui.label(tr!(
                "Building the project, {seconds} s so far: {found} programs found, {read} of {all} reads of them done",
                seconds = seconds,
                found = building.found(),
                read = building.read(),
                all = building.all()
            ));
            stop = (ui.button(tr!("Stop building the project")))
                .on_hover_text(tr!(
                    "The build stops after the program it is reading; no project is written, and the script keeps the source it has"
                ))
                .clicked();
        });
        if stop {
            state.drop_gta6_script_job(doc_id);
        }
        return;
    }
    let mut level = doc.level;
    ui.horizontal(|ui| {
        ui.label(tr!("Level"));
        egui::ComboBox::from_id_salt(("gta6_script_level_choice", doc_id))
            .selected_text(level_label(level))
            .show_ui(ui, |ui| {
                for choice in Level::ALL {
                    ui.selectable_value(&mut level, choice, level_label(choice));
                }
            });
    });
    if level != doc.level {
        let picked = doc.picked.clone();
        state.redo_gta6_script(doc_id, Ask { level, picked }, None);
    }
}

/// How much of a large program its source holds, and the way to decompile all of it.
fn draw_picked(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    doc: &Gta6ScriptDocument,
) {
    let Some(picked) = &doc.picked else {
        return;
    };
    ui.label(tr!(
        "{held} of {all} functions are decompiled. This program is large, so its source holds only the ones picked: click a function in the outline to add it",
        held = picked.len(),
        all = doc.program.functions.len()
    ));
    let idle = state.gta6_script_job(doc_id).is_none();
    let every = ui
        .add_enabled(idle, egui::Button::new(tr!("Decompile every function")))
        .on_hover_text(tr!(
            "Decompiles the whole program on a thread of its own. A program this large takes minutes"
        ));
    if every.clicked() {
        let level = doc.level;
        state.redo_gta6_script(
            doc_id,
            Ask {
                level,
                picked: None,
            },
            None,
        );
    }
}

/// Each names list read, in the order read, with what it named that no list before it did.
fn counted(read: &crate::state::gta6_script::Read) -> String {
    let each = (read.named.iter()).map(|(label, new)| format!("{label} ({new})"));
    each.collect::<Vec<String>>().join(", ")
}

/// The way to build a project of the archive a script decompiled alone was read from.
fn draw_build(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    doc: &Gta6ScriptDocument,
) {
    if doc.read.project.is_some() || doc.archive.is_none() {
        return;
    }
    let idle = state.gta6_script_job(doc_id).is_none();
    let build = ui
        .add_enabled(idle, egui::Button::new(tr!("Build project from this archive")))
        .on_hover_text(tr!(
            "Reads every program of this archive on a thread of its own, writes what they state of each other as project.json in the names folder, in place of the one there, and decompiles this script again with it. An archive of many programs takes minutes"
        ));
    if build.clicked() {
        state.build_gta6_project(doc_id);
    }
}

/// What named things in the source, what was refused, and whether a project stood beside the program.
fn draw_read(ui: &mut egui::Ui, doc: &Gta6ScriptDocument) {
    let dim = |text: String| egui::RichText::new(text).color(theme::TEXT_SECONDARY);
    let read = &doc.read;
    let natives = match (&read.natives, read.natives_lists.is_empty()) {
        (Some(list), _) => tr!("Natives are named from {list}", list = list),
        (None, true) => tr!(
            "No natives list was supplied, so a native is shown by the address of its handler, or by its hash where a names list gives the hash at that address"
        )
        .to_string(),
        (None, false) => tr!(
            "No native is named from {lists}: a compiled program calls a native by the address of its handler, and a natives list names one only where a names list gives the hash at that address (a table of hash and rva)",
            lists = read.natives_lists.join(", ")
        ),
    };
    let names = match read.lists.is_empty() {
        false => tr!(
            "Names are read from {lists}, each with how many hashes, cells, indices, functions or words it named that no list before it did",
            lists = counted(read)
        ),
        true => tr!(
            "No names list was supplied, so a type, a field or a method is shown by its hash or its offset"
        )
        .to_string(),
    };
    let project = match &read.project {
        Some(file) => tr!(
            "What other programs state of the types and functions this one uses is read from the project {file}",
            file = file.display()
        ),
        None => tr!(
            "Decompiled alone: a function that needs what another program states has no body. openrage-cli script export -o writes a project.json of every program of a build; put it in the names folder or beside this file, then open the script again"
        )
        .to_string(),
    };
    ui.label(dim(natives));
    ui.label(dim(names));
    if !read.refused.is_empty() {
        ui.label(tr!(
            "Not read from the names folder: {files}",
            files = read.refused.join("; ")
        ));
    }
    if !read.candidates.is_empty() {
        ui.label(dim(tr!(
            "Passed over in the names folder, as a file whose name holds candidate holds guesses and not names: {files}",
            files = read.candidates.join(", ")
        )));
    }
    ui.label(dim(project));
    if let Some(why) = &read.project_refused {
        ui.label(tr!(
            "A project file was found and not read: {why}",
            why = why
        ));
    }
}

/// The labels files of the names folder, which are guesses, and the choice that reads them; nothing where the folder holds none.
fn draw_labels(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    doc: &Gta6ScriptDocument,
) {
    let read = &doc.read;
    if read.labels.is_empty() {
        return;
    }
    let dim = |text: String| egui::RichText::new(text).color(theme::TEXT_SECONDARY);
    let idle = state.gta6_script_job(doc_id).is_none();
    let mut on = labels_on();
    let choice = egui::Checkbox::new(
        &mut on,
        tr!("Label natives from the labels files (guesses)"),
    );
    let toggled = ui
        .add_enabled(idle, choice)
        .on_hover_text(tr!(
            "A file of the names folder whose name holds labels gives guessed names of natives. It is read only while this is on; a native a natives list names keeps that name, and each label is written with a leading underscore"
        ))
        .changed();
    let line = match read.labelled.is_empty() {
        true => tr!(
            "Not read: {files}. A labels file holds guesses, read only while asked",
            files = read.labels.join(", ")
        ),
        false => {
            let each = (read.labelled.iter()).map(|(label, new)| format!("{label} ({new})"));
            tr!(
                "Natives no list names are labelled from {files}: these are guesses, not names",
                files = each.collect::<Vec<String>>().join(", ")
            )
        }
    };
    ui.label(dim(line));
    if toggled {
        set_labels(on);
        state.redo_gta6_script(doc_id, doc.ask(), None);
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
    draw_picked(ui, state, doc_id, &doc);
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
    draw_read(ui, &doc);
    draw_labels(ui, state, doc_id, &doc);
    draw_build(ui, state, doc_id, &doc);
    let open = ui.button(tr!("Open names folder")).on_hover_text(tr!(
        "Put names lists, natives lists and a project.json in this folder, then open the script again: a file here stands in place of a list OpenRage ships under the same name. Each file is read as a names list where it reads as one, else as a natives list; a file whose name starts with natives, or that holds JSON, is tried as a natives list first"
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
        // The level is chosen by words of its own, and a compiled program spells no native.
        assert!(centre.contains("Source with classes"), "{centre}");
        assert!(centre.contains("by the address of its handler"), "{centre}");
        assert!(!centre.contains("spells"), "{centre}");
        // No project stood beside this script: the facts say so, and how one is made.
        assert!(centre.contains("Decompiled alone"), "{centre}");
        assert!(centre.contains("script export"), "{centre}");
        assert!(!centre.contains("functions are decompiled"), "{centre}");

        // A large program's source holds the functions picked, and the facts say how many.
        let mut large = Gta6ScriptDocument::of(Program::default(), SOURCE, Level::Source);
        large.picked = Some(BTreeSet::new());
        large.read.natives_lists = vec!["demo_natives.json".to_owned()];
        large.read.refused = vec!["demo_broken.txt: not a row".to_owned()];
        let d = state.document_mut(id).expect("the document");
        d.payload = DocPayload::Gta6Script(Arc::new(large));
        let centre = painted(&mut state, false);
        assert!(
            centre.contains("0 of 0 functions are decompiled"),
            "{centre}"
        );
        assert!(centre.contains("Decompile every function"), "{centre}");
        assert!(
            centre.contains("No native is named from demo_natives.json"),
            "{centre}"
        );
        assert!(centre.contains("demo_broken.txt: not a row"), "{centre}");
    }

    #[test]
    fn a_script_of_an_archive_decompiled_alone_offers_a_project_built_of_the_archive() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::default();
        let id = state.new_empty_document(DocumentKind::Text);
        let painted = |state: &mut AppState, doc: Gta6ScriptDocument| {
            let d = state.document_mut(id).expect("the document");
            d.payload = DocPayload::Gta6Script(Arc::new(doc));
            d.kind = DocumentKind::Gta6Script;
            let output = egui::Context::default().run_ui(Default::default(), |ui| {
                draw_facts(ui, state, id, None);
            });
            format!("{:?}", output.shapes)
        };
        // Read from a file of its own: there is no archive to build from.
        let alone = Gta6ScriptDocument::of(Program::default(), SOURCE, Level::Source);
        let centre = painted(&mut state, alone.clone());
        assert!(!centre.contains("Build project"), "{centre}");

        let mut of_archive = alone;
        of_archive.archive = Some("demo_archive.rpf".into());
        of_archive.read.lists = vec!["demo_names.tsv".to_owned()];
        of_archive.read.named = vec![("demo_names.tsv".to_owned(), 7)];
        of_archive.read.candidates = vec!["demo-candidates.tsv".to_owned()];
        let centre = painted(&mut state, of_archive.clone());
        assert!(
            centre.contains("Build project from this archive"),
            "{centre}"
        );
        assert!(centre.contains("demo_names.tsv (7)"), "{centre}");
        assert!(centre.contains("demo-candidates.tsv"), "{centre}");

        // Decompiled with a project already: nothing to build.
        of_archive.read.project = Some("demo/project.json".into());
        let centre = painted(&mut state, of_archive);
        assert!(!centre.contains("Build project"), "{centre}");
    }

    #[test]
    fn labels_in_the_names_folder_are_offered_as_guesses_and_said_once_read() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::default();
        let id = state.new_empty_document(DocumentKind::Text);
        let painted = |state: &mut AppState, doc: Gta6ScriptDocument| {
            let d = state.document_mut(id).expect("the document");
            d.payload = DocPayload::Gta6Script(Arc::new(doc));
            d.kind = DocumentKind::Gta6Script;
            let output = egui::Context::default().run_ui(Default::default(), |ui| {
                draw_facts(ui, state, id, None);
            });
            format!("{:?}", output.shapes)
        };
        // No labels file: no choice is offered.
        let mut doc = Gta6ScriptDocument::of(Program::default(), SOURCE, Level::Source);
        let centre = painted(&mut state, doc.clone());
        assert!(!centre.contains("labels files"), "{centre}");

        doc.read.labels = vec!["demo-labels.txt".to_owned()];
        let centre = painted(&mut state, doc.clone());
        assert!(
            centre.contains("Label natives from the labels files"),
            "{centre}"
        );
        assert!(centre.contains("Not read: demo-labels.txt"), "{centre}");

        doc.read.labelled = vec![("demo-labels.txt".to_owned(), 2)];
        let centre = painted(&mut state, doc);
        assert!(centre.contains("demo-labels.txt (2)"), "{centre}");
        assert!(centre.contains("these are guesses"), "{centre}");
    }

    #[test]
    fn a_level_is_chosen_by_words_of_its_own_and_never_by_the_name_a_command_takes() {
        let labels: BTreeSet<&str> = Level::ALL.into_iter().map(level_label).collect();
        assert_eq!(labels.len(), Level::ALL.len());
        for level in Level::ALL {
            assert_ne!(level_label(level), level.name());
        }
    }
}
