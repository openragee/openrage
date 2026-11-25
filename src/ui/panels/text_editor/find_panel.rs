use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use egui::Align2;

use crate::state::{AppState, DocumentId};
use crate::ui::panel_state::PanelState;
use crate::ui::theme;

use super::editor_state::{load_state, FindMode};
use super::search::{build_query, find_iter, FindOptions, MATCH_CAP};

#[derive(Default)]
pub struct CrossDocFindState {
    pub visible: bool,
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub results: Vec<CrossDocResult>,
    pub error: Option<String>,
    pub want_focus: bool,
}

impl PanelState for CrossDocFindState {
    const KEY: &'static str = "cross_doc_find";
}

#[derive(Clone)]
pub struct CrossDocResult {
    pub doc_id: DocumentId,
    pub doc_name: String,
    pub line_no: usize,
    pub start_char: usize,
    pub preview: String,
}

pub fn draw(ctx: &egui::Context, app_state: &mut AppState) -> bool {
    if !app_state.panels.get::<CrossDocFindState>().visible {
        return false;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app_state.panels.get::<CrossDocFindState>().visible = false;
        return false;
    }

    let mut find = app_state.panels.take::<CrossDocFindState>();

    let mut keep = true;
    let mut chose: Option<CrossDocResult> = None;
    let mut search_now = false;

    egui::Window::new("Find in Open Text Documents")
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .collapsible(false)
        .resizable(true)
        .default_width(640.0)
        .open(&mut keep)
        .show(ctx, |ui| {
            ui.set_min_width(620.0);
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut find.query)
                        .id_salt("cross_doc_find_query")
                        .desired_width(380.0)
                        .hint_text("text or pattern"),
                );
                if find.want_focus {
                    resp.request_focus();
                    find.want_focus = false;
                }
                if resp.changed() || resp.lost_focus() {
                    search_now = true;
                }
                let mut cs = find.case_sensitive;
                if ui.toggle_value(&mut cs, "Aa").changed() {
                    find.case_sensitive = cs;
                    search_now = true;
                }
                let mut ww = find.whole_word;
                if ui.toggle_value(&mut ww, "\\b").changed() {
                    find.whole_word = ww;
                    search_now = true;
                }
                let mut re = find.regex;
                if ui.toggle_value(&mut re, ".*").changed() {
                    find.regex = re;
                    search_now = true;
                }
                if ui.button("Search").clicked() {
                    search_now = true;
                }
            });

            if let Some(err) = find.error.as_deref() {
                ui.label(egui::RichText::new(err).color(theme::ERROR).small());
            }

            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .show(ui, |ui| {
                    if find.results.is_empty() {
                        ui.label(
                            egui::RichText::new(if find.query.is_empty() {
                                "Type a query to search across every open .xml/.meta/… document."
                            } else {
                                "No matches."
                            })
                            .small()
                            .color(theme::TEXT_SECONDARY),
                        );
                    }
                    for r in &find.results {
                        let row = egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(6, 4))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&r.doc_name)
                                            .strong()
                                            .color(theme::TEXT_PRIMARY),
                                    );
                                    ui.label(
                                        egui::RichText::new(format!(":{}", r.line_no + 1))
                                            .color(theme::TEXT_SECONDARY)
                                            .small(),
                                    );
                                    ui.label(
                                        egui::RichText::new(&r.preview)
                                            .monospace()
                                            .color(theme::TEXT_PRIMARY)
                                            .small(),
                                    );
                                });
                            });
                        let resp = ui.interact(
                            row.response.rect,
                            ui.id()
                                .with(("cross_find_row", r.doc_id, r.start_char as u64)),
                            egui::Sense::click(),
                        );
                        if resp.clicked() {
                            chose = Some(r.clone());
                        }
                    }
                });
        });

    if !keep {
        find.visible = false;
    }

    if search_now {
        run_cross_doc_search(app_state, &mut find);
    }

    if let Some(c) = chose {
        jump_to_result(ctx, app_state, &find, &c);
        find.visible = false;
    }

    let visible = find.visible;
    app_state.panels.put(find);
    visible
}

fn run_cross_doc_search(app_state: &AppState, find: &mut CrossDocFindState) {
    find.error = None;
    find.results.clear();
    let q = find.query.trim().to_string();
    if q.is_empty() {
        return;
    }
    let opts = FindOptions {
        case_sensitive: find.case_sensitive,
        whole_word: find.whole_word,
        regex: find.regex,
    };
    let query = match build_query(&q, opts) {
        Ok(v) => v,
        Err(e) => {
            find.error = Some(e);
            return;
        }
    };
    let cancel = AtomicBool::new(false);
    let mut remaining_global = MATCH_CAP;
    for doc in app_state.documents.iter() {
        let Some(text) = doc.payload.found_in() else {
            continue;
        };
        let doc_id = doc.id;
        let doc_name = doc.name.clone();
        let mut local_emitted = 0usize;
        let _ = find_iter(text, 0..text.len_chars(), &query, &cancel, &mut |b| {
            for m in b {
                if remaining_global == 0 {
                    return ControlFlow::Break(());
                }
                let line_no = text.char_to_line(m.start);
                let line_start = text.line_to_char(line_no);
                let line_end = text.line_to_char((line_no + 1).min(text.len_lines()));
                let preview = preview_line(text, line_start, line_end, 160);
                find.results.push(CrossDocResult {
                    doc_id,
                    doc_name: doc_name.clone(),
                    line_no,
                    start_char: m.start,
                    preview,
                });
                remaining_global -= 1;
                local_emitted += 1;
                if local_emitted >= 200 {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        });
        if remaining_global == 0 {
            break;
        }
    }
}

fn preview_line(rope: &ropey::Rope, start: usize, end: usize, max_chars: usize) -> String {
    let take = end.saturating_sub(start).min(max_chars);
    let s: String = rope.slice(start..start + take).to_string();
    let s = s.replace(['\n', '\r'], " ");
    if take < end - start {
        format!("{s}…")
    } else {
        s
    }
}

fn jump_to_result(
    ctx: &egui::Context,
    app_state: &mut AppState,
    find: &CrossDocFindState,
    c: &CrossDocResult,
) {
    app_state.set_active_document(c.doc_id);
    let handle = load_state(ctx, c.doc_id);
    let mut guard = match handle.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    guard.find.visible = true;
    guard.find.mode = FindMode::Find;
    guard.find.query = find.query.clone();
    guard.find.case_sensitive = find.case_sensitive;
    guard.find.whole_word = find.whole_word;
    guard.find.regex = find.regex;
    guard.find.last_dispatched_query.clear();
    guard.find.last_dispatched_flags = u32::MAX;
    guard.caret.cursor = c.start_char;
    guard.caret.anchor = c.start_char;
    guard.revision = guard.revision.wrapping_add(1);
}
