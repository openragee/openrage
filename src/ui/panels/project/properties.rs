//! Properties: what the selected asset is, or the project itself when none is selected, and the ways to see it another way.

use super::{command_button, fitted, open_in_items, severity_colour, size, state_colour};
use crate::state::front_door::counted;
use crate::state::front_door::new_project::GAME_KEY;
use crate::state::front_door::GameChoice;
use crate::state::workbench::{content_name, AssetRow, Counts, Finding, Kept, Scope};
use crate::state::AppState;
use crate::ui::commands::Action;
use crate::ui::theme;

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    if state.project_path.is_none() {
        centred(ui, tr!("No project is open"));
        return;
    }
    let selected = state.selected_asset().cloned();
    // ⚠ The buttons come first and stay where they are, so none is under the facts or under a scroll, and a toast at the foot of the window never covers one.
    match &selected {
        Some(row) => asset_head(ui, state, row),
        None => project_head(ui, state),
    }
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match &selected {
            Some(row) => asset(ui, state, row),
            None => project(ui, state),
        });
}

fn centred(ui: &mut egui::Ui, text: &str) {
    theme::empty_message(ui, text);
}

/// The width under which each fact's value goes under its name: two columns would leave the value a few letters.
const STACK_BELOW: f32 = 230.0;

/// The facts `add` lists, in two columns, or each value under its name in a panel too narrow for them.
fn facts(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui, bool)) {
    if ui.available_width() < STACK_BELOW {
        ui.vertical(|ui| add(ui, true));
    } else {
        egui::Grid::new(id)
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| add(ui, false));
    }
}

/// A fact: a name and its value, in a row of two columns or, when `stacked`, one under the other.
fn fact(ui: &mut egui::Ui, stacked: bool, name: &str, value: &str) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    if stacked {
        ui.label(
            egui::RichText::new(name)
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    } else {
        ui.label(egui::RichText::new(name).color(theme::TEXT_SECONDARY));
    }
    // ⚠ A path breaks after a separator and never inside a name, whatever the width of the panel.
    let broken = fitted(ui, value, &font, ui.available_width());
    ui.add(egui::Label::new(broken).wrap());
    if stacked {
        ui.add_space(theme::SPACING_TIGHT);
    } else {
        ui.end_row();
    }
}

/// The name and state of the selected asset, and what can be done with it: only what can apply to it, so no button is there that never can.
fn asset_head(ui: &mut egui::Ui, state: &mut AppState, row: &AssetRow) {
    let current = state.workbench.state_of(row);
    ui.label(egui::RichText::new(&row.name).strong().size(16.0));
    ui.label(egui::RichText::new(format!("● {}", current.label())).color(state_colour(current)));
    ui.add_space(theme::SPACING_TIGHT);
    ui.horizontal_wrapped(|ui| {
        open_in(ui, state, row);
        // ⚠ The quick look has a button here, since the key that opens it is not on screen anywhere else: the hint of the button names it.
        command_button(ui, state, Action::QuickLook);
        command_button(ui, state, Action::RevealAsset);
        if row.kept == Kept::Source {
            command_button(ui, state, Action::RecookAsset);
            if matches!(
                row.source_kind.as_deref(),
                Some(or_source::models::DRAWABLE | or_source::models::DRAWABLES)
            ) {
                command_button(ui, state, Action::ReplaceGeometry);
            }
            if row.from_game {
                command_button(ui, state, Action::CheckAgainstGame);
            }
        }
    });
}

/// The facts a source states of itself that are bookkeeping and not what a person came for, which sit under More.
const BOOKKEEPING: [&str; 2] = ["Stable id", "Created with"];

/// What is wrong with the selected asset, a sentence for each with the buttons Problems has on its row, so a problem is mended where its asset is selected and not looked for in a list.
fn its_problems(ui: &mut egui::Ui, state: &mut AppState, row: &AssetRow) {
    let about: Vec<Finding> = state.workbench.findings_of(row).cloned().collect();
    if about.is_empty() {
        return;
    }
    let mut chosen = None;
    ui.label(
        egui::RichText::new(tr!("Problems: {said}", said = Counts::of(&about).said()))
            .color(theme::TEXT_SECONDARY),
    );
    for finding in &about {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("● {}", finding.severity.label()))
                    .color(severity_colour(finding.severity)),
            )
            .wrap(),
        );
        ui.add(egui::Label::new(&finding.text).wrap());
        ui.horizontal_wrapped(|ui| {
            if let Some(fix) = super::fixes::buttons(ui, finding, false) {
                chosen = Some(fix);
            }
        });
        ui.add_space(theme::SPACING_TIGHT);
    }
    ui.separator();
    if let Some(fix) = chosen {
        super::fixes::apply(state, fix);
    }
}

fn asset(ui: &mut egui::Ui, state: &mut AppState, row: &AssetRow) {
    its_problems(ui, state, row);
    // ⚠ What the person came for first (where it came from, what it becomes, a garment's ped and slot); the ids and paths are under More.
    facts(ui, "asset_properties", |ui, stacked| {
        fact(ui, stacked, tr!("Kind"), &row.kind);
        for (name, value) in &row.facts {
            if !BOOKKEEPING.contains(name) {
                fact(ui, stacked, name, value);
            }
        }
        if row.kept == Kept::File {
            fact(
                ui,
                stacked,
                tr!("Origin"),
                tr!("A loose file of the project"),
            );
        }
        if let Some(verdict) = state.workbench.originals().verdict_of(&row.path) {
            fact(
                ui,
                stacked,
                tr!("Checked against the game"),
                &verdict.sentence(),
            );
        }
    });
    ui.add_space(theme::SPACING_TIGHT);
    // The place the project knows it by, with the whole path in Reveal's reach.
    let place = (state.project_path.as_deref())
        .map(or_project::project_folder)
        .and_then(|folder| or_project::relative_inside(&row.path, &folder))
        .unwrap_or_else(|| row.path.display().to_string());
    egui::CollapsingHeader::new(tr!("More"))
        .id_salt("asset_more")
        .show(ui, |ui| {
            facts(ui, "asset_more_facts", |ui, stacked| {
                fact(ui, stacked, tr!("Stored as"), row.kept.label());
                // ⚠ A file has no id of its own: it is its place, so a move is a new file.
                match &row.id {
                    Some(id) => fact(ui, stacked, tr!("Stable id"), id),
                    None => fact(
                        ui,
                        stacked,
                        tr!("Stable id"),
                        tr!("none: a file is known by its place"),
                    ),
                }
                for (name, value) in &row.facts {
                    if BOOKKEEPING.contains(name) && *name != "Stable id" {
                        fact(ui, stacked, name, value);
                    }
                }
                fact(ui, stacked, tr!("Place"), &place);
                if let Some(bytes) = row.bytes {
                    fact(ui, stacked, tr!("Size"), &size(bytes));
                }
            });
        });
}

/// Open in ▾: the one control for seeing this asset another way, which lists every workspace and says under the ones that do not show it why they do not.
fn open_in(ui: &mut egui::Ui, state: &mut AppState, row: &AssetRow) {
    let text = tr!("Open in {icon}", icon = egui_phosphor::regular::CARET_DOWN);
    ui.menu_button(text, |ui| {
        crate::ui::popup_of_the_shell(ui.ctx());
        open_in_items(ui, state, row);
    });
}

/// The project's name, what it is, and the two things to do with it.
fn project_head(ui: &mut egui::Ui, state: &mut AppState) {
    let name = state.project_name().unwrap_or_default();
    ui.label(egui::RichText::new(&name).strong().size(16.0));
    ui.label(
        egui::RichText::new(tr!("The project itself; select an asset to see it"))
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    ui.add_space(theme::SPACING_TIGHT);
    ui.horizontal_wrapped(|ui| {
        command_button(ui, state, Action::RevealProject);
        command_button(ui, state, Action::OpenProjectHere);
    });
}

fn project(ui: &mut egui::Ui, state: &mut AppState) {
    let place = state
        .project_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let folder = state
        .project_path
        .as_deref()
        .map(or_project::project_folder)
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let container = &state.project_carried.container;
    // ⚠ Each content says what it holds as the Content panel and the start page's card do: the rows the table lists.
    let content = (container.modules.iter())
        .map(|m| {
            format!(
                "{} · {}",
                content_name(container, &m.id),
                state.workbench.held_by(m)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let builds = counted(container.outputs.len(), "build");
    let held = counted(state.workbench.rows().len(), "asset");
    let found = state.workbench.counts(Scope::Project).said();
    // ⚠ The game a project was made for is the edition a new build ships for, unless the build states its own.
    let game = (state.project_carried.top.get(GAME_KEY))
        .and_then(|game| game.as_str())
        .and_then(GameChoice::from_word)
        .map(|game| match game {
            GameChoice::Both => tr!("Legacy and Enhanced"),
            other => other.label(),
        });
    let made_from = (state
        .project_carried
        .top
        .get(or_project::template::TEMPLATE_KEY))
    .and_then(|name| name.as_str())
    .map(str::to_string);
    let opens_on = state.project_carried.workspace.label();
    let unsaved = if state.project_unsaved() {
        tr!("Yes: Save project writes them")
    } else {
        tr!("No")
    };
    facts(ui, "project_summary", |ui, stacked| {
        if !content.is_empty() {
            fact(ui, stacked, tr!("Content"), &content);
        }
        fact(ui, stacked, tr!("Builds"), &builds);
        fact(ui, stacked, tr!("Assets"), &held);
        fact(ui, stacked, tr!("Problems"), &found);
        fact(ui, stacked, tr!("Unsaved changes"), unsaved);
    });
    not_placed(ui, state);
    ui.add_space(theme::SPACING_TIGHT);
    // ⚠ Not named Details, which is the panel this fold is in.
    egui::CollapsingHeader::new(tr!("Project file"))
        .id_salt("project_details")
        .show(ui, |ui| {
            facts(ui, "project_properties", |ui, stacked| {
                fact(ui, stacked, tr!("File"), &place);
                fact(ui, stacked, tr!("Folder"), &folder);
                fact(ui, stacked, tr!("Opens on"), opens_on);
                if let Some(game) = game {
                    fact(ui, stacked, tr!("Game"), game);
                }
                if let Some(template) = made_from {
                    fact(ui, stacked, tr!("Made from"), &template);
                }
            });
        });
    egui::CollapsingHeader::new(tr!("If you keep this folder in Git"))
        .id_salt("project_git")
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(tr!("Choose what Git keeps."))
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
            repository(ui, state);
        });
    plugin_waivers(ui, state);
}

/// What a repository of the project holds, as the managed Git block states it: each box writes the block when it is touched.
fn repository(ui: &mut egui::Ui, state: &mut AppState) {
    let now = state.commit_choice();
    let mut next = now;
    let boxes = [
        (
            &mut next.sources,
            tr!("Commit the sources"),
            tr!("The source folders of the project, which a build cooks. Off, a repository ignores the sources folder"),
            true,
        ),
        (
            &mut next.build,
            tr!("Commit the build"),
            tr!("The finished resources in the build folder. On by choice: a repository does not hold the build by default"),
            true,
        ),
        (
            &mut next.game_sources,
            tr!("Commit sources taken from the game"),
            tr!("A source taken from the game's own files is the game's content, so a repository leaves it out unless this is on. Check before sharing still lists it"),
            now.sources,
        ),
    ];
    for (on, said, hover, enabled) in boxes {
        let response = ui
            .add_enabled(enabled, egui::Checkbox::new(on, said))
            .on_hover_text(hover);
        let value = *on;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, value, said)
        });
    }
    if next != now {
        if let Err(why) = state.choose_what_to_commit(next) {
            state.show_error_toast(why);
        }
    }
}

/// The findings of validator plugins the project lets stand, each with the reason, and the way to take the waiver away.
fn plugin_waivers(ui: &mut egui::Ui, state: &mut AppState) {
    let waivers = state.plugin_waivers();
    if waivers.is_empty() {
        return;
    }
    ui.add_space(theme::SPACING_DEFAULT);
    ui.label(egui::RichText::new(tr!("Findings of plugins that stand")).strong());
    for waiver in waivers {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&waiver.code).monospace().small());
            ui.label(waiver.reason.as_str());
            if ui
                .small_button(tr!("Remove waiver"))
                .on_hover_text(tr!("The finding stands again at the next scan"))
                .clicked()
            {
                state.remove_plugin_waiver(&waiver.code);
            }
        });
    }
}

/// What the adoption of the project's folder could not place, as the project file recorded it.
fn not_placed(ui: &mut egui::Ui, state: &AppState) {
    let Some(adopted) = state
        .project_carried
        .top
        .get(or_project::adopt::ADOPTED_KEY)
    else {
        return;
    };
    let unplaced = adopted
        .get("unplaced")
        .and_then(|list| list.as_array())
        .map_or(&[][..], Vec::as_slice);
    let notes = adopted
        .get("notes")
        .and_then(|list| list.as_array())
        .map_or(&[][..], Vec::as_slice);
    if unplaced.is_empty() && notes.is_empty() {
        return;
    }
    ui.add_space(theme::SPACING_DEFAULT);
    egui::CollapsingHeader::new(tr!(
        "Not placed when the folder was adopted ({count})",
        count = unplaced.len() + notes.len()
    ))
    .id_salt("adopted_unplaced")
    .show(ui, |ui| {
        for note in notes.iter().filter_map(|n| n.as_str()) {
            ui.add(
                egui::Label::new(egui::RichText::new(note).small().color(theme::WARNING)).wrap(),
            );
        }
        for left in unplaced {
            let path = left
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or_default();
            let why = left.get("why").and_then(|w| w.as_str()).unwrap_or_default();
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{path}: {why}"))
                        .small()
                        .color(theme::TEXT_SECONDARY),
                )
                .wrap(),
            );
        }
    });
}
