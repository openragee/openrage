//! The Project workspace's panels: content and builds, the assets table, properties, problems and the build tab.
//!
//! Each panel reads what `state.workbench` holds and never touches the disk, so a project of thousands
//! of files draws as fast as a small one.

mod assets;
mod budget_window;
mod build_options;
mod build_part;
mod build_tab;
mod builds_editor;
mod builds_file_types;
mod bulk_window;
mod content;
mod create;
mod credit_window;
mod fixes;
pub(crate) mod in_game;
mod maps_window;
mod optimise_window;
mod picker;
mod problems;
mod properties;
mod questions;
mod readable;
mod reopen_question;
mod review_window;
mod rule_titles;
mod rules_window;
mod save_query_window;
mod size_map;

pub use assets::draw as draw_assets;
pub use build_tab::draw as draw_build;
pub use content::draw as draw_content;
pub use problems::draw as draw_problems;
pub use properties::draw as draw_properties;
pub use questions::draw as draw_questions;

use crate::state::workbench::{AssetRow, AssetState, Kept, Severity};
use crate::state::{AppState, DocumentKind};
use crate::ui::commands::{self, Action};
use crate::ui::dock::Workspace;
use crate::ui::theme::{self, Themed};

/// How often a panel that shows the disk asks to be drawn again while it is on screen.
pub(super) const REFRESH: std::time::Duration = crate::state::workbench::RESCAN_AFTER;

/// `action` as a menu or button row: its label and key, greyed with the reason when it cannot run; true once it ran.
pub(super) fn command_button(ui: &mut egui::Ui, state: &mut AppState, action: Action) -> bool {
    let Some(command) = commands::of(state, action) else {
        return false;
    };
    let response = ui.add_enabled(
        command.enabled.is_ok(),
        egui::Button::new(commands::atoms(&command)),
    );
    if commands::hints(response, &command).clicked() {
        ui.close_kind(egui::UiKind::Menu);
        commands::run(state, ui.ctx(), &command);
        return true;
    }
    false
}

pub(super) fn state_colour(state: AssetState) -> Themed {
    match state {
        AssetState::Ready | AssetState::Cooked => theme::SUCCESS,
        AssetState::Unsaved
        | AssetState::NotCooked
        | AssetState::OriginalChanged
        | AssetState::Warning => theme::WARNING,
        AssetState::Missing
        | AssetState::Folder
        | AssetState::Failed
        | AssetState::Unreadable
        | AssetState::Error => theme::ERROR,
    }
}

/// Each workspace the project offers, and whether it shows `row` or why not.
pub(super) fn where_it_opens(
    state: &AppState,
    row: &AssetRow,
) -> Vec<(Workspace, Result<(), &'static str>)> {
    let shown = (row.output.as_ref())
        .filter(|output| output.is_file())
        .unwrap_or(&row.path);
    let kind = DocumentKind::from_path(shown);
    // A row kept in a pack is shown on the screen of the content that holds it.
    let screen = (state.project_carried.container.module(&row.content))
        .and_then(|module| crate::ui::task_screens::of_kind(&module.kind));
    (state.workspaces_offered().into_iter())
        .map(|ws| {
            let answer = match (row.kept, kind) {
                (Kept::Pack, _) if screen.is_some_and(|s| s.workspace == ws) => Ok(()),
                (Kept::Pack, _) => Err(screen
                    .map_or(tr!("This is shown on the screen of its content"), |s| {
                        crate::i18n::tr(s.its_rows)
                    })),
                (_, None) => Err(tr!("OpenRage has no editor for this kind of file")),
                (_, Some(kind)) if ws.handles_kind(kind) => Ok(()),
                _ => Err(tr!("This workspace does not show this kind")),
            };
            (ws, answer)
        })
        .collect()
}

/// One workspace of an Open in menu, whether it can show what the menu is for or not: a workspace that cannot says why in a line under its name. Whether it was pressed.
pub(super) fn open_in_item(
    ui: &mut egui::Ui,
    ws: Workspace,
    answer: Result<(), &'static str>,
) -> bool {
    let item = ui.button(ws.label()).on_hover_text(ws.tooltip());
    if let Err(why) = answer {
        ui.label(
            egui::RichText::new(why)
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    }
    item.clicked()
}

/// The items of Open in for `row`: every workspace, since a workspace is a layout and not a kind of content; one that cannot show the asset opens and says why.
pub(super) fn open_in_items(ui: &mut egui::Ui, state: &mut AppState, row: &AssetRow) {
    for (ws, answer) in where_it_opens(state, row) {
        if open_in_item(ui, ws, answer) {
            ui.close_kind(egui::UiKind::Menu);
            match answer {
                Ok(()) => state.open_asset(&row.key, Some(ws)),
                Err(why) => state.open_workspace_that_shows_nothing_of(ws, &row.name, why),
            }
        }
    }
}

/// The headings of the Workbench guide the `?` of each project screen leads to.
pub(crate) const GUIDE_PROBLEMS: &str = "See what is wrong and fix it";
pub(crate) const GUIDE_BUILD: &str = "Build";

/// A small `?` that opens the Workbench guide at `section`, and says so when it is hovered.
pub(crate) fn help_link(ui: &mut egui::Ui, state: &mut AppState, section: &str) {
    if help_button(ui, section) {
        state.front_door.show_guide(section);
    }
}

/// The `?` itself, for a toolbar that cannot reach the state; whether it was clicked.
pub(crate) fn help_button(ui: &mut egui::Ui, section: &str) -> bool {
    let response = ui
        .small_button("?")
        .on_hover_text(tr!("Read \"{section}\" in the guide", section = section));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            tr!("Help: {section}", section = section),
        )
    });
    response.clicked()
}

/// Open in ▸ in the menu of the selected asset's row.
pub(super) fn open_in_submenu(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(row) = state.selected_asset().cloned() else {
        return;
    };
    ui.menu_button(tr!("Open in"), |ui| open_in_items(ui, state, &row));
}

pub(super) fn severity_colour(severity: Severity) -> Themed {
    match severity {
        Severity::Error => theme::ERROR,
        Severity::Warning => theme::WARNING,
    }
}

/// A size as a person reads it.
pub(super) fn size(bytes: u64) -> String {
    or_job::human_bytes(bytes)
}

/// How long ago `at` was, in a few words.
pub(super) fn ago(at: std::time::SystemTime) -> String {
    crate::state::prefs::ago(at)
}

/// `text` with its lines broken where `fits` says a line is full: at a space, or after a path separator, so a path is never cut inside a name.
///
/// ⚠ A name wider than a line stays on a line of its own and is left to the layout, which breaks it where it can.
pub(super) fn broken_at_separators(text: &str, fits: impl Fn(&str) -> bool) -> String {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for (at, word) in paragraph.split(' ').enumerate() {
            let pieces = word.split_inclusive(['\\', '/']);
            for (piece_at, piece) in pieces.enumerate() {
                let space = if at > 0 && piece_at == 0 { " " } else { "" };
                if !line.is_empty() && !fits(&format!("{line}{space}{piece}")) {
                    lines.push(std::mem::take(&mut line));
                    line.push_str(piece);
                } else {
                    line.push_str(space);
                    line.push_str(piece);
                }
            }
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// `text` cut in the middle until it fits `width` in `font`, keeping its start and more of its end.
///
/// ⚠ The end is kept longer because it is what tells the names of a family apart (`…^jbib_001_u` from `…^jbib_002_u`); the middle is the prefix they share.
pub(super) fn elided_in_the_middle(
    ui: &egui::Ui,
    text: &str,
    font: &egui::FontId,
    width: f32,
) -> String {
    let wide = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
    };
    let full = wide(text);
    let chars: Vec<char> = text.chars().collect();
    if full <= width || chars.len() <= 3 {
        return text.to_string();
    }
    let cut = |keep: usize| {
        let head = keep * 2 / 5;
        let tail = keep - head;
        let start: String = chars[..head].iter().collect();
        let end: String = chars[chars.len() - tail..].iter().collect();
        format!("{start}…{end}")
    };
    // Start from the share of the characters the width allows, and walk to the most that fit.
    let mut keep = ((chars.len() as f32 * width / full) as usize).clamp(1, chars.len() - 1);
    while keep > 1 && wide(&cut(keep)) > width {
        keep -= 1;
    }
    while keep + 1 < chars.len() && wide(&cut(keep + 1)) <= width {
        keep += 1;
    }
    cut(keep)
}

/// `text` as the lines it takes at `width` in `font`, broken by [`broken_at_separators`].
pub(super) fn fitted(ui: &egui::Ui, text: &str, font: &egui::FontId, width: f32) -> String {
    broken_at_separators(text, |line| {
        let drawn =
            ui.painter()
                .layout_no_wrap(line.to_string(), font.clone(), egui::Color32::WHITE);
        drawn.size().x <= width
    })
}

#[cfg(test)]
mod tests {
    use super::broken_at_separators;

    #[test]
    fn a_path_breaks_after_a_separator_and_a_sentence_at_a_space() {
        let fits = |line: &str| line.chars().count() <= 20;
        assert_eq!(
            broken_at_separators(r"C:\Users\user\AppData\Local\Temp\p13\gone.ydd", fits),
            [r"C:\Users\user\", r"AppData\Local\Temp\", r"p13\gone.ydd"].join("\n")
        );
        assert_eq!(
            broken_at_separators("gone.ydd is missing: a/b/c/d.ydd", fits),
            "gone.ydd is missing:\na/b/c/d.ydd"
        );
        assert_eq!(broken_at_separators("short", fits), "short");
        assert_eq!(broken_at_separators("", fits), "");
        // Lines the text has already are kept, and a name too wide for a line is left whole.
        assert_eq!(
            broken_at_separators("a\nreally_long_name_without_separators.ydd", fits),
            "a\nreally_long_name_without_separators.ydd"
        );
    }
}
