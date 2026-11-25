//! The guides, in a window: the documents `docs/guides/Workbench_Guide.md` and `docs/guides/In_Game_Acceptance.md` themselves, which
//! the binary carries, opened at the heading a link names.

use egui::text::{LayoutJob, TextFormat};

use crate::state::front_door::GuideDocument;
use crate::state::AppState;
use crate::ui::theme;

/// The guide as written; a heading is a line that begins `## `, a bullet one that begins `- `, and a word between backticks is code.
const GUIDE: &str = include_str!("../../../docs/guides/Workbench_Guide.md");

/// The acceptance guide as written, which also holds a table, numbered steps, commands between fences and links.
const ACCEPTANCE: &str = include_str!("../../../docs/guides/In_Game_Acceptance.md");

const TITLE: &str = tr_key!("Guide");
const ACCEPTANCE_TITLE: &str = tr_key!("In-game acceptance guide");

/// The line that opens and closes a block of commands.
const FENCE: &str = "```";

/// The text of `document`, and the title of the window that shows it.
fn written(document: GuideDocument) -> (&'static str, &'static str) {
    match document {
        GuideDocument::Workbench => (GUIDE, TITLE),
        GuideDocument::Acceptance => (ACCEPTANCE, ACCEPTANCE_TITLE),
    }
}

/// The headings of the guide, which the links into it name.
#[cfg(test)]
pub fn headings() -> Vec<&'static str> {
    headings_of(GuideDocument::Workbench)
}

/// The headings of `document`, which the links into it name.
pub fn headings_of(document: GuideDocument) -> Vec<&'static str> {
    (written(document).0.lines())
        .filter_map(|line| line.strip_prefix("## "))
        .collect()
}

pub fn draw(ctx: &egui::Context, state: &mut AppState) {
    // A control's Shift+F1 over it is an ask to open the guide where it is explained.
    if let Some(section) = crate::ui::guide_links::take(ctx) {
        state.front_door.show_guide(&section);
    }
    let Some(guide) = state.front_door.guide.as_mut() else {
        return;
    };
    let (document, title) = written(guide.document);
    let mut open = true;
    egui::Window::new(crate::i18n::tr(title))
        .id(egui::Id::new("workbench_guide"))
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_size([640.0, 520.0])
        .show(ctx, |ui| {
            // ⚠ The guide is a long page of sections, and a row that names each of them is the only way to see what is in it.
            let mut jump: Option<&str> = None;
            ui.horizontal_wrapped(|ui| {
                for heading in headings_of(guide.document) {
                    let button = ui.small_button(heading);
                    button.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            tr!("Guide section: {heading}", heading = heading),
                        )
                    });
                    if button.clicked() {
                        jump = Some(heading);
                    }
                }
            });
            ui.separator();
            if let Some(heading) = jump {
                guide.section = heading.to_string();
                guide.scrolled = false;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                body(ui, document, &guide.section, &mut guide.scrolled);
            });
        });
    let escaped =
        ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input();
    if !open || escaped {
        state.front_door.guide = None;
    }
}

fn body(ui: &mut egui::Ui, document: &str, section: &str, scrolled: &mut bool) {
    // ⚠ The page keeps the place it was last read to, which a link into a section left: from the top is asked for like a section.
    if !*scrolled && section.is_empty() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
        *scrolled = true;
    }
    // The indent of the fence that opened the block of commands the line is in, which each of its lines sheds.
    let mut fenced: Option<usize> = None;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in document.lines() {
        let indent = line.len() - line.trim_start().len();
        if let Some(of_the_fence) = fenced {
            if line.trim_start().starts_with(FENCE) {
                fenced = None;
            } else {
                let command = line.get(of_the_fence.min(indent)..).unwrap_or(line);
                ui.add(egui::Label::new(egui::RichText::new(command).monospace()).wrap());
            }
            continue;
        }
        // A row of a table is kept until the line after the table, which draws them together.
        if let Some(row) = row_of(line) {
            rows.push(row);
            continue;
        }
        if !rows.is_empty() {
            table(ui, &std::mem::take(&mut rows));
        }
        if line.starts_with("# ") {
            continue;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            ui.add_space(theme::SPACING_RELAXED);
            let response = ui.label(egui::RichText::new(heading).strong().size(16.0));
            if !*scrolled && heading == section {
                response.scroll_to_me(Some(egui::Align::TOP));
                *scrolled = true;
            }
        } else if line.trim_start().starts_with(FENCE) {
            fenced = Some(indent);
        } else if let Some(item) = line.strip_prefix("- ") {
            ui.add(egui::Label::new(text(ui, &format!("• {item}"))).wrap());
        } else if line.is_empty() {
            ui.add_space(theme::SPACING_TIGHT);
        } else {
            // What continues a numbered step is indented under it, and is a paragraph all the same.
            ui.add(egui::Label::new(text(ui, line.trim_start())).wrap());
        }
    }
    if !rows.is_empty() {
        table(ui, &rows);
    }
}

/// The cells of `line` when it is a row of a table, with their links read as words.
fn row_of(line: &str) -> Option<Vec<String>> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|cell| unlinked(cell.trim())).collect())
}

/// Whether `row` is the rule under a table's head, which is drawn as no row.
fn is_rule(row: &[String]) -> bool {
    (row.iter()).all(|cell| !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':'))
}

/// The rows of a table, the first as its head, each column as wide as its longest text asks for within the window.
fn table(ui: &mut egui::Ui, rows: &[Vec<String>]) {
    let rows: Vec<&Vec<String>> = rows.iter().filter(|row| !is_rule(row)).collect();
    let columns = rows.iter().map(|row| row.len()).max().unwrap_or(0);
    // ⚠ A share of the width by the longest text of a column, held between a word and a sentence, so that an id is not wrapped and a sentence is.
    let share: Vec<f32> = (0..columns)
        .map(|column| {
            let longest = (rows.iter().filter_map(|row| row.get(column)))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0);
            longest.clamp(8, 40) as f32
        })
        .collect();
    let gap = ui.spacing().item_spacing.x;
    let room = (ui.available_width() - gap * columns.saturating_sub(1) as f32).max(0.0);
    let whole: f32 = share.iter().sum();
    for (place, row) in rows.iter().enumerate() {
        // ⚠ A row is as tall as its tallest wrapped cell, known once it is laid out, so its band is painted under it afterwards. An
        // `egui::Grid` sizes a row from the frame before and centres a cell in it, and a wrapped cell then runs under the next row's band.
        let band = ui.painter().add(egui::Shape::Noop);
        let drawn = ui.horizontal_top(|ui| {
            for (cell, share) in row.iter().zip(&share) {
                let width = room * share / whole;
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        let mut words = text(ui, cell);
                        if place == 0 {
                            let strong = ui.visuals().strong_text_color();
                            for part in &mut words.sections {
                                part.format.color = strong;
                            }
                        }
                        ui.add(egui::Label::new(words).wrap());
                    },
                );
            }
        });
        if place % 2 == 1 {
            let under = drawn.response.rect.expand2(egui::vec2(2.0, 1.0));
            let colour = ui.visuals().faint_bg_color;
            (ui.painter()).set(band, egui::Shape::rect_filled(under, 2.0, colour));
        }
    }
}

/// `text` with each Markdown link read as the words it shows, since the window follows no link.
fn unlinked(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|close| {
            let words = &after[..close];
            let target = after[close + 2..].find(')')?;
            (!words.contains('[')).then_some((words, close + 2 + target + 1))
        });
        match link {
            Some((words, end)) => {
                out.push_str(&rest[..open]);
                out.push_str(words);
                rest = &after[end..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `line` as a paragraph of text: the words between backticks in the monospace font, the words between `**` strong, and a link as its words.
fn text(ui: &egui::Ui, line: &str) -> LayoutJob {
    let (plain, strong) = (ui.visuals().text_color(), ui.visuals().strong_text_color());
    let mut job = LayoutJob::default();
    let mut bold = false;
    let line = unlinked(line);
    for (i, part) in line.split('`').enumerate() {
        if i % 2 == 1 {
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            job.append(part, 0.0, TextFormat::simple(font, plain));
            continue;
        }
        for (j, words) in part.split("**").enumerate() {
            bold ^= j > 0;
            let font = egui::TextStyle::Body.resolve(ui.style());
            let colour = if bold { strong } else { plain };
            job.append(words, 0.0, TextFormat::simple(font, colour));
        }
    }
    job
}
