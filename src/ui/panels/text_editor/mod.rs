pub mod editor_state;
pub mod find_bar;
pub mod find_panel;
pub mod fold_worker;
pub mod folds;
pub mod go_to_line;
pub mod highlighters;
pub mod replace_worker;
pub mod script_view;
pub mod search;
pub mod search_worker;

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use egui::{Align2, FontId, Galley, Rect, Sense, Stroke, Vec2};
use ropey::Rope;

use crate::state::{AppState, DocPayload, Document};
use crate::ui::commands::Action;
use crate::ui::{shortcuts, theme};

use editor_state::{
    load_state, CaretState, Edit, EditorState, FindKeys, FindMode, FoldComputeState, CARET_WIDTH,
    FIND_DEBOUNCE, GUTTER_PAD,
};
use find_bar::FindBarOutcome;
use fold_worker::{FoldJob, FoldWorker};
use folds::{FoldMap, FoldTree, FOLD_INLINE_BYTES};
use go_to_line::GoToLine;
use highlighters::{detect_log, Language};
use replace_worker::{ReplaceJob, ReplaceUpdate};
use search::{build_query, FindOptions, Query};
use search_worker::SearchJob;

/// What the text editor on screen can take back, and the step Edit ▸ Undo or Redo asks of it: its history is its own, not the document's.
#[derive(Default)]
pub struct TextHistory {
    doc: Option<crate::state::DocumentId>,
    can_undo: bool,
    can_redo: bool,
    asked: Option<bool>,
    /// Whether the editor was painted in this frame, and in the one before: a command reads the frame before, since the menus are drawn first.
    seen: bool,
    live: bool,
    /// The document in front, and the workspace it was in front in, when the tabs last were fitted to it.
    fitted: Option<(crate::state::DocumentId, crate::ui::dock::Workspace)>,
}

impl crate::ui::panel_state::PanelState for TextHistory {
    const KEY: &'static str = "text_history";
}

/// Before a frame is drawn: what the editor said in the last one is what the commands of this one read.
pub fn new_frame(state: &mut AppState) {
    let history = state.panels.get::<TextHistory>();
    history.live = std::mem::take(&mut history.seen);
    front_the_document_tab(state);
}

/// ⚠ A leaf keeps the tab last in front: a document the Text panel has no view of would open on its empty tab, so the Document tab beside it comes forward, once per document and workspace.
fn front_the_document_tab(state: &mut AppState) {
    use crate::ui::panels::registry::ids;
    let ws = state.active_workspace;
    let now = state.active_document.map(|doc| (doc, ws));
    let fitted = &mut state.panels.get::<TextHistory>().fitted;
    if now.is_none() || std::mem::replace(fitted, now) == now {
        return;
    }
    if (state.active_document()).is_none_or(|doc| has_text_view(&doc.payload)) {
        return;
    }
    let docks = &mut state.workspace_docks;
    let text_in_front_of_document = docks.maximized(ws).is_none()
        && (docks.peek(ws)).is_some_and(|dock| {
            dock.iter_leaves().any(|(_, leaf)| {
                let front = leaf.tabs.get(leaf.active.0).map(|tab| tab.id());
                front == Some(ids::TEXT_EDITOR) && leaf.tabs.iter().any(|t| t.id() == ids::DOCUMENT)
            })
        });
    if text_in_front_of_document {
        crate::ui::dock_commands::reveal(docks.get_mut(ws), ids::DOCUMENT);
    }
}

/// Whether the Text panel shows `payload`, rather than the line that asks for a text-form file.
fn has_text_view(payload: &DocPayload) -> bool {
    matches!(
        payload,
        DocPayload::Text(_)
            | DocPayload::NameTable(_)
            | DocPayload::Meta(_)
            | DocPayload::Gta6Script(_)
            | DocPayload::Gxt2(_)
    ) || crate::state::ScriptDocument::in_payload(payload).is_some()
}

/// Whether Edit ▸ Undo (or Redo, with `undo` false) is the text editor's: it shows the document in front and has a step to take.
pub fn offers(state: &AppState, undo: bool) -> bool {
    state.panels.peek::<TextHistory>().is_some_and(|h| {
        let step = if undo { h.can_undo } else { h.can_redo };
        h.live && step && h.doc.is_some() && h.doc == state.active_document
    })
}

/// Have the editor undo, or redo, a step at its next paint.
pub fn ask(state: &mut AppState, undo: bool) {
    state.panels.get::<TextHistory>().asked = Some(undo);
}

fn note_history(
    app_state: &mut AppState,
    doc: crate::state::DocumentId,
    editor: &editor_state::EditorStateHandle,
) {
    let editor = editor.lock().expect("editor state poisoned");
    let history = app_state.panels.get::<TextHistory>();
    history.doc = Some(doc);
    history.can_undo = !editor.history.undo.is_empty();
    history.can_redo = !editor.history.redo.is_empty();
    history.seen = true;
}

/// One step of the editor's history; whether there was one to take.
fn step_history(state: &mut EditorState, rope: &mut Rope, undo: bool) -> bool {
    let step = if undo {
        state.history.undo(rope)
    } else {
        state.history.redo(rope)
    };
    let Some((cursor, replaced)) = step else {
        return false;
    };
    if let Some(r) = replaced {
        *rope = r;
    }
    state.caret.cursor = cursor;
    state.caret.anchor = cursor;
    state.caret.desired_col = None;
    true
}

pub fn draw(ui: &mut egui::Ui, app_state: &mut AppState) {
    let Some(doc_id) = app_state.active_document else {
        empty_state(ui);
        return;
    };
    match app_state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Text(_) | DocPayload::NameTable(_)) => {
            draw_text_doc(ui, app_state, doc_id)
        }
        Some(DocPayload::Meta(_)) => draw_meta_doc(ui, app_state, doc_id),
        Some(DocPayload::Gta6Script(_)) => script_view::draw(ui, app_state, doc_id),
        Some(payload) if crate::state::ScriptDocument::in_payload(payload).is_some() => {
            draw_script_doc(ui, app_state, doc_id)
        }
        _ => empty_state(ui),
    }
}

/// A `.ysc`'s decompiled source, read-only: OpenRage cannot write bytecode back.
fn draw_script_doc(ui: &mut egui::Ui, app_state: &AppState, doc_id: crate::state::DocumentId) {
    let Some(doc) = app_state.document(doc_id) else {
        empty_state(ui);
        return;
    };
    let Some(script) = crate::state::ScriptDocument::in_payload(&doc.payload) else {
        empty_state(ui);
        return;
    };
    let editor_id = ui.id().with(("virtual_text_editor_v3", doc_id));
    let state_handle = load_state(ui.ctx(), doc_id);
    let mut state = state_handle.lock().expect("editor state poisoned");
    draw_footer(ui, doc, &script.source, "UTF-8", "LF", &state.caret, true);
    let body_rect = ui.available_rect_before_wrap();
    // A rope clone shares its nodes, and a read-only paint never edits it.
    let mut source = script.source.clone();
    state.find_keys = find_keys(ui, &app_state.prefs);
    let language = Language::Plain;
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
}

fn draw_text_doc(ui: &mut egui::Ui, app_state: &mut AppState, doc_id: crate::state::DocumentId) {
    let editor_id = ui.id().with(("virtual_text_editor_v3", doc_id));
    let state_handle = load_state(ui.ctx(), doc_id);
    let read_only =
        (app_state.document(doc_id)).is_some_and(crate::state::rpf9_scan::text_takes_no_edit);
    if let Some(doc) = app_state.document(doc_id) {
        if let DocPayload::Text(t) | DocPayload::NameTable(t) = &doc.payload {
            let state = state_handle.lock().expect("editor state poisoned");
            draw_footer(
                ui,
                doc,
                &t.text,
                t.encoding.label(),
                t.line_ending.label(),
                &state.caret,
                read_only,
            );
        }
    }
    let body_rect = ui.available_rect_before_wrap();

    let language_from_path =
        Language::from_path(app_state.document(doc_id).and_then(|d| d.path.as_deref()));
    let keys = find_keys(ui, &app_state.prefs);

    let asked = app_state.panels.get::<TextHistory>().asked.take();
    let edited = {
        let Some(doc) = app_state.document_mut(doc_id) else {
            empty_state(ui);
            return;
        };
        let (DocPayload::Text(text_doc) | DocPayload::NameTable(text_doc)) = &mut doc.payload
        else {
            empty_state(ui);
            return;
        };
        let mut state = state_handle.lock().expect("editor state poisoned");
        state.find_keys = keys;
        state.asked_history = asked;
        let language = resolve_language(language_from_path, &text_doc.text, &mut state);
        if state.last_language != language {
            state.last_language = language;
            state.cache.clear();
        }
        state.folds.language = language;
        paint_editor(
            ui,
            &mut state,
            &mut text_doc.text,
            body_rect,
            editor_id,
            language,
            read_only,
        )
    };
    note_history(app_state, doc_id, &state_handle);

    if edited {
        if let Some(doc) = app_state.document_mut(doc_id) {
            doc.mark_edited();
        }
    }
}

fn draw_meta_doc(ui: &mut egui::Ui, app_state: &mut AppState, doc_id: crate::state::DocumentId) {
    let meta_revision = match app_state.document_mut(doc_id) {
        Some(doc) => match &mut doc.payload {
            DocPayload::Meta(m) => {
                m.ensure_text_current();
                m.revision()
            }
            _ => {
                empty_state(ui);
                return;
            }
        },
        None => {
            empty_state(ui);
            return;
        }
    };

    let editor_id = ui.id().with(("virtual_text_editor_v3", doc_id));
    let state_handle = load_state(ui.ctx(), doc_id);
    // A scanned GTA VI metadata body saves its typed XML as a copy, as its form's edits.
    let read_only = (app_state.document(doc_id)).is_some_and(|d| {
        (d.read_only).is_some_and(|why| why != crate::state::rpf9_scan::READ_ONLY_BODY)
    });
    if let Some(doc) = app_state.document(doc_id) {
        if let DocPayload::Meta(meta) = &doc.payload {
            let state = state_handle.lock().expect("editor state poisoned");
            draw_footer(
                ui,
                doc,
                &meta.text.text,
                meta.text.encoding.label(),
                meta.text.line_ending.label(),
                &state.caret,
                read_only,
            );
        }
    }
    let body_rect = ui.available_rect_before_wrap();

    let rev_marker_id = ui.id().with(("meta_text_revision_marker_v1", doc_id));
    let prev_rev: Option<u64> = ui.data(|d| d.get_temp(rev_marker_id));
    let visual_edit_landed = prev_rev != Some(meta_revision);
    if visual_edit_landed {
        ui.data_mut(|d| d.insert_temp(rev_marker_id, meta_revision));
    }
    let keys = find_keys(ui, &app_state.prefs);

    let asked = app_state.panels.get::<TextHistory>().asked.take();
    let edited = {
        let Some(doc) = app_state.document_mut(doc_id) else {
            empty_state(ui);
            return;
        };
        let DocPayload::Meta(meta) = &mut doc.payload else {
            empty_state(ui);
            return;
        };
        let mut state = state_handle.lock().expect("editor state poisoned");
        state.find_keys = keys;
        state.asked_history = asked;
        if visual_edit_landed {
            state.cache.clear();
            state.rope_revision = state.rope_revision.wrapping_add(1);
            state.find.invalidate_results();
            state.find.matched_at_revision = u64::MAX;
        }
        let language = Language::Xml;
        if state.last_language != language {
            state.last_language = language;
            state.cache.clear();
        }
        state.folds.language = language;
        paint_editor(
            ui,
            &mut state,
            &mut meta.text.text,
            body_rect,
            editor_id,
            language,
            read_only,
        )
    };
    note_history(app_state, doc_id, &state_handle);

    if edited {
        if let Some(doc) = app_state.document_mut(doc_id) {
            if let DocPayload::Meta(meta) = &mut doc.payload {
                meta.mark_text_edited();
                ui.data_mut(|d| d.insert_temp(rev_marker_id, meta.revision()));
                doc.mark_edited();
            }
        }
    }
}

fn empty_state(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new(
                "Open a text-form file (.xml, .meta, .ini, .ide, .ipl, .json, .log, …)\n\
                 to edit here.",
            )
            .small()
            .color(theme::TEXT_SECONDARY),
        );
    });
}

fn resolve_language(from_path: Language, rope: &Rope, state: &mut EditorState) -> Language {
    if !matches!(from_path, Language::Plain) {
        return from_path;
    }
    if matches!(state.last_language, Language::Log) {
        return Language::Log;
    }
    if detect_log(rope) {
        return Language::Log;
    }
    Language::Plain
}

fn paint_editor(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    rope: &mut Rope,
    rect: Rect,
    id: egui::Id,
    language: Language,
    read_only: bool,
) -> bool {
    drain_search_updates(state);
    if !read_only {
        drain_replace_updates(state, rope);
    }
    drain_fold_updates(state, rope);

    schedule_fold_compute(state, rope, language);

    let fold_map = current_fold_map(state, rope);

    let painter = ui.painter_at(rect);
    let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
    let (row_height, space_advance) = ui
        .ctx()
        .fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, ' ')));
    let total_source_lines = rope.len_lines().max(1);

    let response = ui.interact(rect, id, Sense::click_and_drag());
    // ⚠ The editor paints its own text: without this a screen reader finds no text field here.
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, !read_only, tr!("Text"))
    });

    let chevron_w = if has_folds_to_show(state) {
        space_advance * 2.0
    } else {
        0.0
    };
    let line_no_w = (decimal_digits(total_source_lines) as f32) * space_advance + GUTTER_PAD * 2.0;
    let gutter_width = chevron_w + line_no_w;
    let body_rect = Rect::from_min_max(egui::pos2(rect.min.x + gutter_width, rect.min.y), rect.max);

    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(gutter_width, rect.height())),
        0.0,
        theme::BG_L0,
    );

    let modifiers = ui.input(|i| i.modifiers);
    let focus = response.has_focus() || response.gained_focus();
    if response.clicked() || response.gained_focus() {
        response.request_focus();
    }
    let mut dirty = false;
    let mut find_keys = FindKeyOutcome::default();
    let bar_focus = find_bar_has_focus(ui.ctx(), state);
    if focus {
        crate::ui::focus_ring::reads_typed_text(ui.ctx(), id);
        find_keys = consume_find_shortcuts(ui, state);
        if consume_text_input(ui, state, rope, modifiers, bar_focus, read_only) {
            dirty = true;
        }
        if !bar_focus {
            consume_navigation_keys(ui, state, rope, body_rect, row_height, &fold_map);
        }
        if !bar_focus && !read_only && consume_history_keys(ui, state, rope, modifiers) {
            dirty = true;
        }
        consume_fold_keys(ui, state, rope, &fold_map);
    }
    if let Some(undo) = state.asked_history.take() {
        dirty |= !read_only && step_history(state, rope, undo);
    }

    if dirty {
        state.revision = state.revision.wrapping_add(1);
        state.rope_revision = state.rope_revision.wrapping_add(1);
        state.cache.clear();
        state.find.invalidate_results();
        state.find.matched_at_revision = u64::MAX;
    }

    let fold_map = current_fold_map(state, rope);
    // ⚠ Counted after the edit: one that removed lines leaves the counts before it past the rope's end.
    let total_source_lines = rope.len_lines().max(1);
    let visual_lines = fold_map.visual_line_count() as usize;

    drive_find(ui.ctx(), state, rope, find_keys, read_only);

    if let Some(pointer) = response.interact_pointer_pos() {
        let local = pointer - body_rect.min + Vec2::new(state.caret.scroll_x, state.caret.scroll_y);
        let new_cursor = pos_to_char(
            ui.ctx(),
            &mut state.cache,
            rope,
            language,
            &font,
            local,
            row_height,
            space_advance,
            &fold_map,
        );
        if response.clicked() {
            state.caret.cursor = new_cursor;
            state.caret.anchor = new_cursor;
            state.caret.desired_col = None;
        } else if response.dragged() {
            state.caret.cursor = new_cursor;
            state.caret.desired_col = None;
        }
        if pointer.x < rect.min.x + chevron_w {
            handle_gutter_click(state, &fold_map, pointer, rect, row_height);
        }
    }

    if response.hovered() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y != 0.0 {
            state.caret.scroll_y = (state.caret.scroll_y - scroll_delta.y).max(0.0);
        }
        if scroll_delta.x != 0.0 {
            state.caret.scroll_x = (state.caret.scroll_x - scroll_delta.x).max(0.0);
        }
    }
    // Room kept beside the caret when the lines scroll sideways to it.
    let margin = space_advance * 4.0;

    if dirty || state.scrolled_at_revision != state.revision {
        let cursor_line = char_to_line(rope, state.caret.cursor) as u32;
        if fold_map.is_hidden(cursor_line) {
            if let Some(tree) = state.folds.tree_mut() {
                if let Some(fi) = tree.innermost_at(cursor_line) {
                    tree.set_collapsed(fi, false);
                }
            }
        }
        let fold_map = current_fold_map(state, rope);
        let visual_row = fold_map.source_to_visual(cursor_line).unwrap_or(0);
        let cursor_y = (visual_row as f32) * row_height;
        if cursor_y < state.caret.scroll_y {
            state.caret.scroll_y = cursor_y;
        } else if cursor_y + row_height > state.caret.scroll_y + body_rect.height() {
            state.caret.scroll_y = cursor_y + row_height - body_rect.height();
        }
        let line = cursor_line as usize;
        let galley =
            (state.cache).get_or_layout(ui.ctx(), &line_text_for(rope, line), &font, language);
        let x = column_to_x(&galley, state.caret.cursor - rope.line_to_char(line));
        if x < state.caret.scroll_x + margin {
            state.caret.scroll_x = (x - margin).max(0.0);
        } else if x + margin > state.caret.scroll_x + body_rect.width() {
            state.caret.scroll_x = x + margin - body_rect.width();
        }
        state.scrolled_at_revision = state.revision;
    }

    let total_height = (visual_lines as f32) * row_height;
    let max_scroll = (total_height - body_rect.height()).max(0.0);
    if state.caret.scroll_y > max_scroll {
        state.caret.scroll_y = max_scroll;
    }

    let first_visible_visual =
        ((state.caret.scroll_y / row_height).floor() as usize).saturating_sub(1);
    let visible_rows = (body_rect.height() / row_height).ceil() as usize + 2;
    let last_visible_visual = (first_visible_visual + visible_rows).min(visual_lines);

    let selection = state.caret.selection();
    let cursor_line = char_to_line(rope, state.caret.cursor);

    let first_src = fold_map.visual_to_source(first_visible_visual as u32) as usize;
    let last_src =
        fold_map.visual_to_source((last_visible_visual.saturating_sub(1)) as u32) as usize;
    let visible_first_char = rope.line_to_char(first_src.min(total_source_lines - 1));
    let visible_last_char = rope.line_to_char((last_src + 1).min(total_source_lines));

    let mut painted_collapsed_pill = false;
    // ⚠ The lines' own painter: one scrolled sideways must not draw over the gutter.
    let body_painter = painter.with_clip_rect(body_rect);
    let text_x = body_rect.min.x - state.caret.scroll_x;
    let mut widest: f32 = 0.0;

    for visual_row in first_visible_visual..last_visible_visual {
        let row_top = rect.min.y + (visual_row as f32 * row_height) - state.caret.scroll_y;
        let line_idx = fold_map.visual_to_source(visual_row as u32) as usize;

        if chevron_w > 0.0 {
            paint_chevron(
                ui,
                &painter,
                state,
                line_idx as u32,
                rect.min.x,
                row_top,
                chevron_w,
                row_height,
            );
        }
        painter.text(
            egui::pos2(rect.min.x + gutter_width - GUTTER_PAD, row_top),
            Align2::RIGHT_TOP,
            (line_idx + 1).to_string(),
            font.clone(),
            theme::TEXT_SECONDARY.get(),
        );

        let line_text = line_text_for(rope, line_idx);
        let galley = state
            .cache
            .get_or_layout(ui.ctx(), &line_text, &font, language);
        widest = widest.max(galley.size().x);

        if let Some(range) = &selection {
            if let Some((sx, ex)) = row_selection_range_galley(rope, line_idx, range, &galley) {
                let sel_rect = Rect::from_min_max(
                    egui::pos2(text_x + sx, row_top),
                    egui::pos2(text_x + ex, row_top + row_height),
                );
                body_painter.rect_filled(sel_rect, 0.0, theme::SELECTION_BG);
            }
        }

        paint_match_backgrounds(
            &body_painter,
            state,
            &fold_map,
            line_idx,
            row_top,
            text_x,
            row_height,
            rope,
            &galley,
            visible_first_char,
            visible_last_char,
        );

        body_painter.galley(
            egui::pos2(text_x, row_top),
            galley.clone(),
            theme::TEXT_PRIMARY.get(),
        );

        if let Some(tree) = state.folds.tree() {
            for (i, r) in tree.ranges.iter().enumerate() {
                if r.start_line as usize == line_idx && tree.collapsed[i] {
                    let line_w = galley.size().x;
                    let pill_origin = egui::pos2(text_x + line_w + 8.0, row_top + 2.0);
                    let pill_text = format!("⋯ {} lines", r.line_count());
                    let pill_galley = ui.ctx().fonts_mut(|f| {
                        f.layout(
                            pill_text,
                            font.clone(),
                            theme::TEXT_DIM.get(),
                            f32::INFINITY,
                        )
                    });
                    let pill_rect = Rect::from_min_size(
                        pill_origin,
                        Vec2::new(pill_galley.size().x + 12.0, row_height - 4.0),
                    );
                    body_painter.rect_filled(
                        pill_rect,
                        theme::RADIUS_SMALL as f32,
                        theme::FOLD_COLLAPSED_BG,
                    );
                    body_painter.galley(
                        pill_origin + Vec2::new(6.0, 0.0),
                        pill_galley,
                        theme::TEXT_DIM.get(),
                    );
                    painted_collapsed_pill = true;
                    break;
                }
            }
        }

        if focus && cursor_line == line_idx {
            let col = state.caret.cursor - rope.line_to_char(line_idx);
            let x = column_to_x(&galley, col);
            body_painter.line_segment(
                [
                    egui::pos2(text_x + x, row_top),
                    egui::pos2(text_x + x, row_top + row_height),
                ],
                Stroke::new(CARET_WIDTH, theme::ACCENT),
            );
        }
    }
    let scrollable = (widest + margin - body_rect.width()).max(0.0);
    state.caret.scroll_x = state.caret.scroll_x.min(scrollable);

    painter.line_segment(
        [
            egui::pos2(rect.min.x + gutter_width, rect.min.y),
            egui::pos2(rect.min.x + gutter_width, rect.max.y),
        ],
        Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );

    if state.find.visible {
        let replace_hint = state.find_keys.replace_hint.as_deref();
        let outcome = find_bar::draw(
            ui,
            body_rect,
            &mut state.find,
            state.rope_revision,
            replace_hint,
        );
        apply_find_bar_outcome(state, rope, outcome, read_only);
    }
    if let Some(bar) = state.go_to.as_mut() {
        let outcome = go_to_line::draw(ui, body_rect, bar, total_source_lines);
        if !matches!(outcome, go_to_line::Outcome::Wait) {
            state.go_to = None;
            ui.memory_mut(|m| m.request_focus(id));
        }
        if let go_to_line::Outcome::Go(line) = outcome {
            state.caret.cursor = rope.line_to_char((line - 1).min(rope.len_lines() - 1));
            state.caret.collapse();
            state.caret.desired_col = None;
            state.revision = state.revision.wrapping_add(1);
        }
    }

    let _ = painted_collapsed_pill;
    if focus {
        ui.ctx().request_repaint();
    }
    dirty
}

#[derive(Default, Clone, Copy)]
struct FindKeyOutcome {
    open_find: bool,
    open_replace: bool,
    next: bool,
    prev: bool,
}

/// This frame's presses of the editor's find and replace commands, as the user bound them.
fn find_keys(ui: &egui::Ui, prefs: &crate::state::Prefs) -> FindKeys {
    let pressed: Vec<Action> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|event| {
                shortcuts::action_for_event(crate::ui::commands::Place::TextEditor, event, prefs)
            })
            .collect()
    });
    FindKeys {
        find: pressed.contains(&Action::TextFind),
        replace: pressed.contains(&Action::TextReplace),
        go_to_line: pressed.contains(&Action::TextGoToLine),
        replace_hint: shortcuts::label_for(Action::TextReplace, prefs),
    }
}

fn find_bar_has_focus(ctx: &egui::Context, state: &EditorState) -> bool {
    if !state.find.visible {
        return false;
    }
    ctx.memory(|m| {
        m.focused().is_some_and(|fid| {
            let dbg = fid.short_debug_format();
            dbg.contains("text_editor_find")
        })
    })
}

fn consume_find_shortcuts(ui: &mut egui::Ui, state: &mut EditorState) -> FindKeyOutcome {
    let mut out = FindKeyOutcome::default();
    let events = ui.input(|i| i.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers: m,
            ..
        } = event
        else {
            continue;
        };
        if key == egui::Key::F3 {
            if m.shift {
                out.prev = true;
            } else {
                out.next = true;
            }
        }
    }
    out.open_find = state.find_keys.find;
    out.open_replace = state.find_keys.replace;
    if out.open_find {
        state.find.visible = true;
        state.find.mode = FindMode::Find;
        state.find.want_focus_query = true;
        if let Some(sel) = state.caret.selection() {
            let _ = sel;
        }
    }
    if out.open_replace {
        state.find.visible = true;
        state.find.mode = FindMode::Replace;
        state.find.want_focus_query = true;
    }
    if state.find_keys.go_to_line {
        state.go_to = Some(GoToLine {
            typed: String::new(),
            focus: true,
        });
    }
    out
}

fn drive_find(
    ctx: &egui::Context,
    state: &mut EditorState,
    rope: &Rope,
    _keys: FindKeyOutcome,
    _read_only: bool,
) {
    if !state.find.visible {
        return;
    }
    state.find.has_selection = state.caret.selection().is_some();
    let flags = state.find.flag_bits();
    let need_dispatch = state.find.query != state.find.last_dispatched_query
        || flags != state.find.last_dispatched_flags
        || state.find.matched_at_revision != state.rope_revision;
    if !need_dispatch {
        return;
    }
    if state.find.query.is_empty() {
        state.find.invalidate_results();
        state.find.last_dispatched_query.clear();
        state.find.last_dispatched_flags = flags;
        state.find.matched_at_revision = state.rope_revision;
        return;
    }
    // ⚠ `last_input` is the bar's, stamped at each edit: stamping it here too would restart the
    // wait every frame the query differs, and a new query would never be searched.
    let waited = state.find.last_input.elapsed();
    if waited < FIND_DEBOUNCE {
        ctx.request_repaint_after(FIND_DEBOUNCE - waited);
        return;
    }

    let opts = FindOptions {
        case_sensitive: state.find.case_sensitive,
        whole_word: state.find.whole_word,
        regex: state.find.regex,
    };
    let query = match build_query(&state.find.query, opts) {
        Ok(q) => q,
        Err(e) => {
            state.find.error = Some(e);
            state.find.invalidate_results();
            return;
        }
    };
    state.find.error = None;
    let scope = if state.find.in_selection {
        state.caret.selection().unwrap_or(0..rope.len_chars())
    } else {
        0..rope.len_chars()
    };

    state.find.cancel.store(true, Ordering::Relaxed);
    state.find.cancel = Arc::new(AtomicBool::new(false));
    state.find.gen = state.find.gen.wrapping_add(1);
    state.find.invalidate_results();
    state.find.matched_at_revision = state.rope_revision;
    state.find.last_dispatched_query = state.find.query.clone();
    state.find.last_dispatched_flags = flags;
    state.find.last_scope = scope.clone();
    state.find.searching = true;

    let job = SearchJob {
        gen: state.find.gen,
        rope: Arc::new(rope.clone()),
        scope,
        query,
        cancel: state.find.cancel.clone(),
    };
    let _ = state.search_worker().dispatch(job);
}

fn drain_search_updates(state: &mut EditorState) {
    let Some(worker) = state.search_worker.as_ref() else {
        return;
    };
    while let Some(upd) = worker.try_recv() {
        if upd.gen != state.find.gen {
            continue;
        }
        if !upd.batch.is_empty() {
            state.find.matches.extend(upd.batch);
            if state.find.active.is_none() {
                state.find.active = Some(0);
                if let Some(first) = state.find.matches.first() {
                    state.caret.cursor = first.start;
                    state.caret.anchor = first.start;
                    state.revision = state.revision.wrapping_add(1);
                }
            }
        }
        if upd.truncated {
            state.find.truncated = true;
        }
        if upd.done {
            state.find.searching = false;
        }
    }
}

fn apply_find_bar_outcome(
    state: &mut EditorState,
    rope: &mut Rope,
    outcome: FindBarOutcome,
    read_only: bool,
) {
    if outcome.close {
        state.find.visible = false;
        return;
    }
    if outcome.cancel {
        state.find.cancel.store(true, Ordering::Relaxed);
        state.find.searching = false;
    }
    if outcome.query_changed {
        state.find.last_input = Instant::now();
    }
    if outcome.go_next {
        navigate_match(state, rope, 1);
    }
    if outcome.go_prev {
        navigate_match(state, rope, -1);
    }
    if outcome.replace_one && !read_only {
        replace_active_match(state, rope);
    }
    if outcome.replace_all && !read_only {
        dispatch_replace_all(state, rope);
    }
}

fn navigate_match(state: &mut EditorState, _rope: &mut Rope, delta: i32) {
    if state.find.matches.is_empty() {
        return;
    }
    let n = state.find.matches.len() as i32;
    let cur = state.find.active.map(|i| i as i32).unwrap_or(-1);
    let new = (cur + delta).rem_euclid(n);
    state.find.active = Some(new as usize);
    let m = &state.find.matches[new as usize];
    state.caret.cursor = m.start;
    state.caret.anchor = m.end;
    state.caret.desired_col = None;
    state.revision = state.revision.wrapping_add(1);
}

fn replace_active_match(state: &mut EditorState, rope: &mut Rope) {
    let Some(active_idx) = state.find.active else {
        return;
    };
    let Some(m) = state.find.matches.get(active_idx).cloned() else {
        return;
    };
    let cursor_before = state.caret.cursor;
    let removed: String = rope.slice(m.start..m.end).to_string();
    let replacement = if state.find.regex {
        let opts = FindOptions {
            case_sensitive: state.find.case_sensitive,
            whole_word: state.find.whole_word,
            regex: true,
        };
        match build_query(&state.find.query, opts) {
            Ok(Query::Regex(re)) => match re.captures(&removed) {
                Some(caps) => {
                    let mut s = String::new();
                    caps.expand(&state.find.replacement, &mut s);
                    s
                }
                None => state.find.replacement.clone(),
            },
            _ => state.find.replacement.clone(),
        }
    } else {
        state.find.replacement.clone()
    };
    rope.remove(m.start..m.end);
    rope.insert(m.start, &replacement);

    let new_end = m.start + replacement.chars().count();
    let delta = new_end as i64 - m.end as i64;

    for m_later in state.find.matches.iter_mut().skip(active_idx + 1) {
        if delta >= 0 {
            m_later.start = m_later.start.saturating_add(delta as usize);
            m_later.end = m_later.end.saturating_add(delta as usize);
        } else {
            let d = (-delta) as usize;
            m_later.start = m_later.start.saturating_sub(d);
            m_later.end = m_later.end.saturating_sub(d);
        }
    }
    state.find.matches.remove(active_idx);
    if state.find.matches.is_empty() {
        state.find.active = None;
    } else {
        state.find.active = Some(active_idx.min(state.find.matches.len() - 1));
    }

    let cursor_after = new_end;
    state.caret.cursor = cursor_after;
    state.caret.anchor = cursor_after;
    state.caret.desired_col = None;
    state.history.push_edit(Edit {
        char_pos: m.start,
        removed,
        inserted: replacement,
        cursor_before,
        cursor_after,
        timestamp: Instant::now(),
        coalescable: false,
    });
    state.revision = state.revision.wrapping_add(1);
    state.rope_revision = state.rope_revision.wrapping_add(1);
    state.cache.clear();
    state.find.matched_at_revision = state.rope_revision;
}

fn dispatch_replace_all(state: &mut EditorState, rope: &Rope) {
    let opts = FindOptions {
        case_sensitive: state.find.case_sensitive,
        whole_word: state.find.whole_word,
        regex: state.find.regex,
    };
    let query = match build_query(&state.find.query, opts) {
        Ok(q) => q,
        Err(e) => {
            state.find.error = Some(e);
            return;
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    state.find.cancel = cancel.clone();
    state.find.gen = state.find.gen.wrapping_add(1);
    state.find.searching = true;
    let job = ReplaceJob {
        gen: state.find.gen,
        rope: Arc::new(rope.clone()),
        query,
        replacement: state.find.replacement.clone(),
        regex_replacement: state.find.regex,
        cancel,
    };
    let _ = state.replace_worker().dispatch(job);
}

fn drain_replace_updates(state: &mut EditorState, rope: &mut Rope) {
    let Some(worker) = state.replace_worker.as_ref() else {
        return;
    };
    while let Some(upd) = worker.try_recv() {
        let ReplaceUpdate {
            gen,
            match_count,
            before,
            after,
        } = upd;
        if gen != state.find.gen {
            continue;
        }
        state.find.searching = false;
        let Some(after) = after else {
            continue;
        };
        let cursor_before = state.caret.cursor;
        let new_rope = (*after).clone();
        *rope = new_rope;
        let cursor_after = state.caret.cursor.min(rope.len_chars());
        state.caret.cursor = cursor_after;
        state.caret.anchor = cursor_after;
        state
            .history
            .push_snapshot(before, after, cursor_before, cursor_after);
        state.revision = state.revision.wrapping_add(1);
        state.rope_revision = state.rope_revision.wrapping_add(1);
        state.cache.clear();
        state.find.invalidate_results();
        state.find.last_dispatched_query.clear();
        state.find.matched_at_revision = u64::MAX;
        let _ = match_count;
    }
}

fn schedule_fold_compute(state: &mut EditorState, rope: &Rope, language: Language) {
    let foldable = matches!(language, Language::Xml);
    if !foldable {
        state.folds.compute = FoldComputeState::Idle;
        return;
    }
    let needs_recompute = match &state.folds.compute {
        FoldComputeState::Ready(t) => t.rope_revision != state.revision,
        FoldComputeState::Pending { .. } => false,
        FoldComputeState::Idle => true,
    };
    if !needs_recompute {
        return;
    }
    if rope.len_bytes() <= FOLD_INLINE_BYTES {
        let cancel = AtomicBool::new(false);
        let (ranges, tags, pool) = folds::compute_xml_folds(rope, &cancel, &mut |_| {});
        let prev = std::mem::replace(&mut state.folds.compute, FoldComputeState::Idle);
        let mut tree = FoldTree::new(state.revision, ranges, tags, pool);
        if let FoldComputeState::Ready(prev_tree) = prev {
            tree.carry_collapsed_state(&prev_tree);
        }
        state.folds.compute = FoldComputeState::Ready(tree);
        state.folds.computed_for_revision = state.revision;
    } else {
        if state.folds.worker.is_none() {
            state.folds.worker = Some(FoldWorker::spawn());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.folds.gen_counter = state.folds.gen_counter.wrapping_add(1);
        let job = FoldJob {
            gen: state.folds.gen_counter,
            rope: Arc::new(rope.clone()),
            cancel: cancel.clone(),
        };
        let gen = job.gen;
        if let Some(w) = &state.folds.worker {
            let _ = w.dispatch(job);
            state.folds.compute = FoldComputeState::Pending { gen };
        }
    }
}

fn drain_fold_updates(state: &mut EditorState, _rope: &mut Rope) {
    let Some(worker) = state.folds.worker.as_ref() else {
        return;
    };
    while let Some(upd) = worker.try_recv() {
        let pending_gen = match &state.folds.compute {
            FoldComputeState::Pending { gen, .. } => *gen,
            _ => continue,
        };
        if upd.gen != pending_gen {
            continue;
        }
        let prev = std::mem::replace(&mut state.folds.compute, FoldComputeState::Idle);
        let mut tree = FoldTree::new(state.revision, upd.ranges, upd.tag_indices, upd.tag_pool);
        if let FoldComputeState::Ready(prev_tree) = prev {
            tree.carry_collapsed_state(&prev_tree);
        }
        state.folds.compute = FoldComputeState::Ready(tree);
        state.folds.computed_for_revision = state.revision;
    }
}

fn current_fold_map(state: &mut EditorState, rope: &Rope) -> FoldMap {
    let total_lines = rope.len_lines() as u32;
    match state.folds.tree() {
        Some(t) => FoldMap::build(t, total_lines),
        None => FoldMap::empty(total_lines),
    }
}

fn has_folds_to_show(state: &EditorState) -> bool {
    matches!(state.folds.compute, FoldComputeState::Ready(_))
        && state.folds.tree().is_some_and(|t| !t.ranges.is_empty())
}

#[allow(clippy::too_many_arguments)]
fn paint_chevron(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    state: &EditorState,
    source_line: u32,
    gutter_x: f32,
    row_top: f32,
    chevron_w: f32,
    row_height: f32,
) {
    let Some(tree) = state.folds.tree() else {
        return;
    };
    let fold = tree
        .ranges
        .iter()
        .enumerate()
        .find(|(_, r)| r.start_line == source_line);
    if let Some((idx, _)) = fold {
        let collapsed = tree.collapsed[idx];
        let glyph = if collapsed { "▸" } else { "▾" };
        let font = FontId::monospace(ui.style().text_styles[&egui::TextStyle::Monospace].size);
        painter.text(
            egui::pos2(gutter_x + chevron_w * 0.5, row_top),
            Align2::CENTER_TOP,
            glyph,
            font,
            theme::TEXT_SECONDARY.get(),
        );
    } else {
        if tree
            .ranges
            .iter()
            .any(|r| source_line > r.start_line && source_line <= r.end_line)
        {
            painter.line_segment(
                [
                    egui::pos2(gutter_x + chevron_w * 0.5, row_top),
                    egui::pos2(gutter_x + chevron_w * 0.5, row_top + row_height),
                ],
                Stroke::new(1.0_f32, theme::FOLD_GUIDE),
            );
        }
    }
}

fn handle_gutter_click(
    state: &mut EditorState,
    fold_map: &FoldMap,
    pointer: egui::Pos2,
    rect: Rect,
    row_height: f32,
) {
    let visual_row = ((pointer.y - rect.min.y + state.caret.scroll_y) / row_height).floor() as u32;
    let source_line = fold_map.visual_to_source(visual_row);
    let Some(tree) = state.folds.tree_mut() else {
        return;
    };
    if let Some(idx) = tree.ranges.iter().position(|r| r.start_line == source_line) {
        tree.toggle(idx);
    }
}

fn consume_fold_keys(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    rope: &Rope,
    _fold_map: &FoldMap,
) -> bool {
    let mut toggled = false;
    let events = ui.input(|i| i.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers: m,
            ..
        } = event
        else {
            continue;
        };
        if !(m.command || m.ctrl) {
            continue;
        }
        let cur_line = char_to_line(rope, state.caret.cursor) as u32;
        let Some(tree) = state.folds.tree_mut() else {
            continue;
        };
        match key {
            egui::Key::OpenBracket if m.shift => {
                if let Some(i) = tree.innermost_at(cur_line) {
                    tree.set_collapsed(i, true);
                    toggled = true;
                }
            }
            egui::Key::CloseBracket if m.shift => {
                if let Some(i) = tree.innermost_at(cur_line) {
                    tree.set_collapsed(i, false);
                    toggled = true;
                }
            }
            _ => {}
        }
    }
    toggled
}

fn consume_text_input(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    rope: &mut Rope,
    modifiers: egui::Modifiers,
    suppress: bool,
    read_only: bool,
) -> bool {
    if suppress {
        return false;
    }
    let mut dirty = false;
    let events = ui.input(|i| i.events.clone());
    for event in events {
        match event {
            egui::Event::Copy => {
                if let Some(range) = state.caret.selection() {
                    let s: String = rope.slice(range).to_string();
                    ui.ctx().copy_text(s);
                }
            }
            _ if read_only => {}
            egui::Event::Text(s) if !modifiers.command && !modifiers.ctrl => {
                apply_replace(state, rope, &s, true);
                dirty = true;
            }
            egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                ..
            } => {
                apply_replace(state, rope, "\n", false);
                dirty = true;
            }
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed: true,
                modifiers: m,
                ..
            } if !m.shift => {
                apply_replace(state, rope, "    ", false);
                dirty = true;
            }
            egui::Event::Key {
                key: egui::Key::Backspace,
                pressed: true,
                ..
            } => {
                if state.caret.selection().is_some() {
                    apply_replace(state, rope, "", false);
                } else if state.caret.cursor > 0 {
                    let prev = state.caret.cursor - 1;
                    apply_remove(state, rope, prev..state.caret.cursor);
                }
                dirty = true;
            }
            egui::Event::Key {
                key: egui::Key::Delete,
                pressed: true,
                ..
            } => {
                if state.caret.selection().is_some() {
                    apply_replace(state, rope, "", false);
                    dirty = true;
                } else if state.caret.cursor < rope.len_chars() {
                    let next = state.caret.cursor + 1;
                    apply_remove(state, rope, state.caret.cursor..next);
                    dirty = true;
                }
            }
            egui::Event::Cut => {
                if let Some(range) = state.caret.selection() {
                    let s: String = rope.slice(range.clone()).to_string();
                    ui.ctx().copy_text(s);
                    apply_remove(state, rope, range);
                    dirty = true;
                }
            }
            egui::Event::Paste(s) => {
                apply_replace(state, rope, &s, false);
                dirty = true;
            }
            _ => {}
        }
    }
    dirty
}

fn consume_history_keys(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    rope: &mut Rope,
    modifiers: egui::Modifiers,
) -> bool {
    if !(modifiers.command || modifiers.ctrl) {
        return false;
    }
    let events = ui.input(|i| i.events.clone());
    let mut dirty = false;
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers: m,
            ..
        } = event
        else {
            continue;
        };
        if !(m.command || m.ctrl) {
            continue;
        }
        match key {
            egui::Key::Z if !m.shift => dirty |= step_history(state, rope, true),
            egui::Key::Y => dirty |= step_history(state, rope, false),
            egui::Key::Z if m.shift => dirty |= step_history(state, rope, false),
            _ => {}
        }
    }
    dirty
}

fn apply_replace(state: &mut EditorState, rope: &mut Rope, s: &str, coalescable: bool) {
    let cursor_before = state.caret.cursor;
    let (range, removed) = if let Some(range) = state.caret.selection() {
        let removed: String = rope.slice(range.clone()).to_string();
        (range, removed)
    } else {
        (state.caret.cursor..state.caret.cursor, String::new())
    };
    if !removed.is_empty() {
        rope.remove(range.clone());
    }
    state.caret.cursor = range.start;
    if !s.is_empty() {
        rope.insert(state.caret.cursor, s);
        state.caret.cursor += s.chars().count();
    }
    state.caret.anchor = state.caret.cursor;
    state.caret.desired_col = None;
    state.history.push_edit(Edit {
        char_pos: range.start,
        removed,
        inserted: s.to_string(),
        cursor_before,
        cursor_after: state.caret.cursor,
        timestamp: Instant::now(),
        coalescable: coalescable && s.chars().count() == 1 && s != "\n",
    });
}

fn apply_remove(state: &mut EditorState, rope: &mut Rope, range: Range<usize>) {
    let cursor_before = state.caret.cursor;
    let removed: String = rope.slice(range.clone()).to_string();
    rope.remove(range.clone());
    state.caret.cursor = range.start;
    state.caret.anchor = range.start;
    state.caret.desired_col = None;
    state.history.push_edit(Edit {
        char_pos: range.start,
        removed,
        inserted: String::new(),
        cursor_before,
        cursor_after: state.caret.cursor,
        timestamp: Instant::now(),
        coalescable: false,
    });
}

fn consume_navigation_keys(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    rope: &Rope,
    body_rect: Rect,
    row_height: f32,
    _fold_map: &FoldMap,
) -> bool {
    let events = ui.input(|i| i.events.clone());
    let mut moved_any = false;
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers: m,
            ..
        } = event
        else {
            continue;
        };
        if (m.command || m.ctrl) && matches!(key, egui::Key::Z | egui::Key::Y) {
            continue;
        }
        if (m.command || m.ctrl) && matches!(key, egui::Key::F | egui::Key::H) {
            continue;
        }
        if matches!(key, egui::Key::F3) {
            continue;
        }

        let was_selecting = m.shift;
        let mut moved = false;

        match key {
            egui::Key::ArrowLeft => {
                if state.caret.cursor > 0 {
                    state.caret.cursor -= 1;
                    moved = true;
                }
                state.caret.desired_col = None;
            }
            egui::Key::ArrowRight => {
                if state.caret.cursor < rope.len_chars() {
                    state.caret.cursor += 1;
                    moved = true;
                }
                state.caret.desired_col = None;
            }
            egui::Key::ArrowUp => {
                vertical_move(&mut state.caret, rope, -1);
                moved = true;
            }
            egui::Key::ArrowDown => {
                vertical_move(&mut state.caret, rope, 1);
                moved = true;
            }
            egui::Key::Home => {
                if m.command || m.ctrl {
                    state.caret.cursor = 0;
                } else {
                    let line = char_to_line(rope, state.caret.cursor);
                    state.caret.cursor = rope.line_to_char(line);
                }
                state.caret.desired_col = None;
                moved = true;
            }
            egui::Key::End => {
                if m.command || m.ctrl {
                    state.caret.cursor = rope.len_chars();
                } else {
                    let line = char_to_line(rope, state.caret.cursor);
                    state.caret.cursor = line_end_char(rope, line);
                }
                state.caret.desired_col = None;
                moved = true;
            }
            egui::Key::PageUp => {
                let rows = (body_rect.height() / row_height).max(1.0) as i64;
                vertical_move(&mut state.caret, rope, -rows);
                moved = true;
            }
            egui::Key::PageDown => {
                let rows = (body_rect.height() / row_height).max(1.0) as i64;
                vertical_move(&mut state.caret, rope, rows);
                moved = true;
            }
            egui::Key::A if m.command || m.ctrl => {
                state.caret.anchor = 0;
                state.caret.cursor = rope.len_chars();
                state.caret.desired_col = None;
                moved_any = true;
                continue;
            }
            _ => continue,
        }

        if moved {
            if !was_selecting {
                state.caret.collapse();
            }
            moved_any = true;
        }
    }
    if moved_any {
        state.revision = state.revision.wrapping_add(1);
    }
    moved_any
}

fn vertical_move(caret: &mut CaretState, rope: &Rope, delta_rows: i64) {
    let total_lines = rope.len_lines();
    if total_lines == 0 {
        return;
    }
    let cur_line = char_to_line(rope, caret.cursor);
    let cur_col = caret.cursor - rope.line_to_char(cur_line);
    let target_col = caret.desired_col.unwrap_or(cur_col);
    caret.desired_col = Some(target_col);
    let new_line = ((cur_line as i64) + delta_rows).clamp(0, total_lines as i64 - 1) as usize;
    let line_start = rope.line_to_char(new_line);
    let line_len = line_char_len(rope, new_line);
    let col = target_col.min(line_len);
    caret.cursor = line_start + col;
}

#[allow(clippy::too_many_arguments)]
fn paint_match_backgrounds(
    painter: &egui::Painter,
    state: &EditorState,
    _fold_map: &FoldMap,
    line_idx: usize,
    row_top: f32,
    body_x: f32,
    row_height: f32,
    rope: &Rope,
    galley: &Galley,
    visible_first_char: usize,
    visible_last_char: usize,
) {
    if state.find.matches.is_empty() {
        return;
    }
    let line_start = rope.line_to_char(line_idx);
    let line_end = line_start + line_char_len(rope, line_idx);
    if line_end < visible_first_char || line_start > visible_last_char {
        return;
    }
    let first = state.find.matches.partition_point(|m| m.end <= line_start);
    for (i, m) in state.find.matches.iter().enumerate().skip(first) {
        if m.start > line_end {
            break;
        }
        let lo = m.start.max(line_start);
        let hi = m.end.min(line_end + 1);
        if hi <= lo {
            continue;
        }
        let sx = column_to_x(galley, lo - line_start);
        let ex = column_to_x(galley, (hi - line_start).min(line_end - line_start));
        let r = Rect::from_min_max(
            egui::pos2(body_x + sx, row_top),
            egui::pos2(body_x + ex.max(sx + 2.0), row_top + row_height),
        );
        let active = state.find.active == Some(i);
        let color = if active {
            theme::SEARCH_MATCH_ACTIVE
        } else {
            theme::SEARCH_MATCH_BG
        };
        painter.rect_filled(r, 0.0, color);
        painter.rect_stroke(
            r,
            0.0,
            Stroke::new(1.0_f32, theme::SEARCH_MATCH_EDGE),
            egui::epaint::StrokeKind::Inside,
        );
        if active {
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0_f32, theme::ACCENT),
                egui::epaint::StrokeKind::Outside,
            );
        }
    }
}

fn line_char_len(rope: &Rope, line_idx: usize) -> usize {
    let line = rope.line(line_idx);
    let raw_len = line.len_chars();
    if line.chars().last() == Some('\n') {
        raw_len.saturating_sub(1)
    } else {
        raw_len
    }
}

fn line_end_char(rope: &Rope, line_idx: usize) -> usize {
    rope.line_to_char(line_idx) + line_char_len(rope, line_idx)
}

fn char_to_line(rope: &Rope, char_idx: usize) -> usize {
    rope.char_to_line(char_idx.min(rope.len_chars()))
}

fn line_text_for(rope: &Rope, line_idx: usize) -> String {
    let line = rope.line(line_idx);
    let s: std::borrow::Cow<str> = line.into();
    s.strip_suffix('\n')
        .map(str::to_string)
        .unwrap_or_else(|| s.into_owned())
}

#[allow(clippy::too_many_arguments)]
fn pos_to_char(
    ctx: &egui::Context,
    cache: &mut editor_state::GalleyCache,
    rope: &Rope,
    language: Language,
    font: &FontId,
    local: Vec2,
    row_height: f32,
    _space_advance: f32,
    fold_map: &FoldMap,
) -> usize {
    let visual_row =
        ((local.y / row_height).floor() as u32).min(fold_map.visual_line_count().saturating_sub(1));
    let line = fold_map.visual_to_source(visual_row) as usize;
    let line_text = line_text_for(rope, line);
    let galley = cache.get_or_layout(ctx, &line_text, font, language);
    let ccursor = galley.cursor_from_pos(Vec2::new(local.x, row_height * 0.5));
    let col = ccursor.index.min(line_char_len(rope, line));
    rope.line_to_char(line) + col
}

fn column_to_x(galley: &Galley, col: usize) -> f32 {
    galley.pos_from_cursor(egui::text::CCursor::new(col)).min.x
}

fn row_selection_range_galley(
    rope: &Rope,
    line_idx: usize,
    range: &Range<usize>,
    galley: &Galley,
) -> Option<(f32, f32)> {
    let line_start = rope.line_to_char(line_idx);
    let line_end = line_start + line_char_len(rope, line_idx);
    let lo = range.start.max(line_start);
    let hi = range.end.min(line_end + 1);
    if hi <= lo {
        return None;
    }
    let sx = column_to_x(galley, lo - line_start);
    let ex = column_to_x(galley, (hi - line_start).min(line_end - line_start));
    Some((sx, ex.max(sx + 2.0)))
}

fn decimal_digits(n: usize) -> usize {
    if n == 0 {
        1
    } else {
        (n as f64).log10().floor() as usize + 1
    }
}

fn draw_footer(
    ui: &mut egui::Ui,
    doc: &Document,
    rope: &Rope,
    encoding_label: &str,
    line_ending_label: &str,
    caret: &CaretState,
    read_only: bool,
) {
    let total_chars = rope.len_chars();
    let cursor_line = rope.char_to_line(caret.cursor.min(total_chars));
    let line_start = rope.line_to_char(cursor_line);
    let cursor_col = caret.cursor.saturating_sub(line_start);

    // ⚠ Drawn before the lines, which take the room it leaves, so it shows
    // the caret and the unsaved mark as the last frame left them.
    egui::Panel::bottom(ui.make_persistent_id("text_editor_footer"))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
        .show_separator_line(false)
        .show_inside(ui, |ui| {
            let roomy = ui.available_width() >= 320.0;
            // The right side first: a narrow editor wraps it and cuts the name short.
            egui::Sides::new().shrink_left().truncate().show(
                ui,
                |ui| {
                    ui.label(
                        egui::RichText::new(&doc.name)
                            .small()
                            .color(theme::TEXT_SECONDARY),
                    );
                    if doc.dirty {
                        ui.label(
                            egui::RichText::new("•")
                                .small()
                                .color(theme::WARNING)
                                .strong(),
                        )
                        .on_hover_text("Unsaved changes");
                    }
                    // ⚠ The editor's own answer: a document read-only for its view can still take its text.
                    if let Some(why) = doc.read_only.filter(|_| read_only) {
                        let mark = egui::RichText::new(tr!("read-only")).small().strong();
                        ui.label(mark.color(theme::WARNING))
                            .on_hover_text(crate::i18n::tr(why));
                    }
                },
                |ui| {
                    let wrapping = egui::Layout::right_to_left(egui::Align::Center);
                    ui.with_layout(wrapping.with_main_wrap(true), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Ln {}, Col {}",
                                cursor_line + 1,
                                cursor_col + 1
                            ))
                            .small()
                            .color(theme::TEXT_SECONDARY),
                        );
                        // ⚠ The counts go first when the column is narrow: with them the right side wraps under the name it is beside.
                        if roomy {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} lines • {} chars • {} bytes",
                                    rope.len_lines(),
                                    total_chars,
                                    rope.len_bytes()
                                ))
                                .small()
                                .color(theme::TEXT_SECONDARY),
                            );
                        }
                        ui.separator();
                        ui.label(
                            egui::RichText::new(line_ending_label)
                                .small()
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.separator();
                        ui.label(
                            egui::RichText::new(encoding_label)
                                .small()
                                .color(theme::TEXT_SECONDARY),
                        );
                    });
                },
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DocumentId, PersistedBinding, Prefs, TextDocument};
    use crate::ui::dock::Workspace;

    /// A text document open in the editor, drawn as the Data workspace draws it.
    struct Editing {
        ctx: egui::Context,
        state: AppState,
        doc: DocumentId,
        time: f64,
    }

    impl Editing {
        fn new(text: &str) -> Self {
            Self::with(
                DocPayload::Text(TextDocument::decode(text.as_bytes())),
                None,
            )
        }

        fn with(payload: DocPayload, read_only: Option<&'static str>) -> Self {
            let mut state = AppState::from_prefs(Prefs::default());
            state.documents.clear();
            let doc = 4243;
            let mut document = Document::new(
                doc,
                Some(std::path::PathBuf::from("C:/mods/notes.txt")),
                payload,
            );
            document.read_only = read_only;
            state.documents.push(document);
            state.active_document = Some(doc);
            state.active_workspace = Workspace::Data;
            let mut editing = Self {
                ctx: egui::Context::default(),
                state,
                doc,
                time: 0.0,
            };
            editing.frame(Vec::new(), egui::Modifiers::NONE);
            let at = egui::pos2(300.0, 60.0);
            let press = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            editing.frame(
                vec![egui::Event::PointerMoved(at), press(true)],
                egui::Modifiers::NONE,
            );
            editing.frame(vec![press(false)], egui::Modifiers::NONE);
            editing.frame(Vec::new(), egui::Modifiers::NONE);
            editing
        }

        /// One frame drawn; the texts it painted.
        fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<String> {
            self.time += 1.0 / 30.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 300.0),
                )),
                time: Some(self.time),
                modifiers,
                events,
                ..Default::default()
            };
            let state = &mut self.state;
            let output = self.ctx.run_ui(input, |ui| draw(ui, state));
            (output.shapes.iter())
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                    _ => None,
                })
                .collect()
        }

        fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
            let event = egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            };
            self.frame(vec![event], modifiers);
            self.frame(Vec::new(), egui::Modifiers::NONE);
        }

        fn caret(&self) -> CaretState {
            load_state(&self.ctx, self.doc)
                .lock()
                .expect("editor state")
                .caret
        }

        fn text(&self) -> String {
            match &self.state.document(self.doc).expect("open").payload {
                DocPayload::Text(text) => text.text.to_string(),
                _ => unreachable!("a text document"),
            }
        }
    }

    #[test]
    fn a_long_line_scrolls_sideways_to_keep_the_caret_in_view() {
        let mut editing = Editing::new(&format!("short\n{}", "x".repeat(500)));
        editing.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        editing.key(egui::Key::End, egui::Modifiers::NONE);
        let caret = editing.caret();
        assert_eq!(
            caret.cursor, 506,
            "End did not reach the end of the long line"
        );
        assert!(
            caret.scroll_x > 0.0,
            "the caret is off the right edge and the lines did not scroll"
        );
        editing.key(egui::Key::Home, egui::Modifiers::NONE);
        assert_eq!(
            editing.caret().scroll_x,
            0.0,
            "Home leaves the lines scrolled"
        );
    }

    #[test]
    fn ctrl_g_goes_to_the_line_typed() {
        let text = "one\ntwo\nthree\nfour\nfive";
        let mut editing = Editing::new(text);
        editing.key(egui::Key::G, crate::ui::shortcuts::PRIMARY_EGUI);
        editing.frame(vec![egui::Event::Text("4".into())], egui::Modifiers::NONE);
        editing.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(
            editing.text(),
            text,
            "the line number was typed into the file"
        );
        assert_eq!(
            editing.caret().cursor,
            "one\ntwo\nthree\n".len(),
            "the caret is not at the start of line 4"
        );
    }

    /// What the editor reads of one frame in which `key` was pressed with Ctrl.
    fn with_ctrl(key: egui::Key, prefs: &Prefs) -> FindKeys {
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: crate::ui::shortcuts::PRIMARY_EGUI,
            }],
            ..Default::default()
        };
        let mut keys = FindKeys::default();
        let _ = egui::Context::default().run_ui(input, |ui| keys = find_keys(ui, prefs));
        keys
    }

    #[test]
    fn the_editor_finds_and_replaces_on_the_keys_they_are_bound_to() {
        let mut prefs = Prefs::default();
        assert!(with_ctrl(egui::Key::F, &prefs).find);
        assert!(with_ctrl(egui::Key::H, &prefs).replace);
        prefs.shortcut_bindings.insert(
            "replace_in_document".to_string(),
            PersistedBinding {
                control: false,
                ctrl: true,
                shift: false,
                alt: false,
                key: "G".to_string(),
            },
        );
        let rebound = with_ctrl(egui::Key::G, &prefs);
        assert!(rebound.replace, "Replace did not follow its binding");
        assert_eq!(
            rebound.replace_hint,
            Some(format!(
                "{}+G",
                crate::ui::shortcuts::primary_modifier_name()
            ))
        );
        assert!(!with_ctrl(egui::Key::H, &prefs).replace);
    }

    #[test]
    fn typing_over_every_line_selected_paints_the_one_line_left() {
        let lines: Vec<String> = (1..=47).map(|n| format!("line {n}")).collect();
        let mut editing = Editing::new(&lines.join("\n"));
        editing.key(egui::Key::A, crate::ui::shortcuts::PRIMARY_EGUI);
        assert_eq!(editing.caret().selection(), Some(0..editing.text().len()));
        editing.frame(vec![egui::Event::Text("x".into())], egui::Modifiers::NONE);
        editing.frame(Vec::new(), egui::Modifiers::NONE);
        assert_eq!(
            editing.text(),
            "x",
            "the typing did not replace the selection"
        );
    }

    #[test]
    fn a_scanned_metadata_body_s_text_says_read_only_only_when_it_takes_no_typing() {
        let painted = |why| {
            let meta = DocPayload::Meta(crate::state::MetaDocument::empty());
            let mut editing = Editing::with(meta, Some(why));
            editing.frame(Vec::new(), egui::Modifiers::NONE)
        };
        let mark = tr!("read-only").to_string();
        let body = painted(crate::state::rpf9_scan::READ_ONLY_BODY);
        assert!(
            !body.contains(&mark),
            "a body whose text takes typing is marked read-only"
        );
        let kept = painted("read-only for another reason");
        assert!(kept.contains(&mark), "a read-only document lost its mark");
    }

    #[test]
    fn a_document_with_no_text_view_opens_on_its_document_tab() {
        use crate::ui::panels::registry::ids;
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        state.active_workspace = Workspace::Data;
        let text_tab = |state: &mut AppState| {
            crate::ui::dock_commands::reveal(
                state.workspace_docks.get_mut(Workspace::Data),
                ids::TEXT_EDITOR,
            );
        };
        let in_front = |state: &AppState| {
            let dock = state
                .workspace_docks
                .peek(Workspace::Data)
                .expect("a layout");
            (dock.iter_leaves())
                .find(|(_, leaf)| leaf.tabs.iter().any(|t| t.id() == ids::DOCUMENT))
                .and_then(|(_, leaf)| leaf.tabs.get(leaf.active.0).map(|t| t.id()))
        };
        let open = |state: &mut AppState, doc: DocumentId, payload| {
            state.documents.push(Document::new(doc, None, payload));
            state.active_document = Some(doc);
            new_frame(state);
        };
        text_tab(&mut state);
        open(&mut state, 1, DocPayload::Text(TextDocument::decode(b"a")));
        assert_eq!(
            in_front(&state),
            Some(ids::TEXT_EDITOR),
            "a text document left Text"
        );
        let bytes = crate::state::HexDocument {
            bytes: vec![0; 4],
            banner: String::new(),
        };
        open(&mut state, 2, DocPayload::Hex(bytes));
        assert_eq!(
            in_front(&state),
            Some(ids::DOCUMENT),
            "a document with no text view opened on the empty Text tab"
        );
        text_tab(&mut state);
        new_frame(&mut state);
        assert_eq!(
            in_front(&state),
            Some(ids::TEXT_EDITOR),
            "the Text tab, clicked again, was taken away"
        );
    }
}
