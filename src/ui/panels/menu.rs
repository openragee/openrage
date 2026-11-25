use crate::platform::file_dialogs;
use crate::state::{AppState, DocumentId};
use crate::ui::commands::{self, Action, Command};
use crate::ui::theme;

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::top("menu_bar").show_inside(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            // ⚠ A workspace button is taller than a menu title: without this the bar grows when the first one shows, and everything under it moves.
            ui.set_min_height(theme::TAB_STRIP_BUTTON_HEIGHT);
            file_menu(ui, state);
            edit_menu(ui, state);
            view_menu(ui, state);
            project_menu_of_the_window(ui, state);
            tools_menu(ui, state);
            help_menu(ui, state);

            ui.separator();
            workspace_buttons(ui, state);
            // From the right edge: Build, then what the edit mode says, since both are about where a write goes.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                build_control(ui, state);
                edit_mode_banner(ui, state);
            });
        });
    });
}

/// What the Build control takes at the right of the bar while a project is open, which the workspace buttons leave clear: a build's name and the longest word of its state.
const BUILD_ROOM: f32 = 260.0;

/// The window's one Build button, at the right of the bar while a project is open: the build it makes where the project has several, and the dot and the word that say how that build stands.
fn build_control(ui: &mut egui::Ui, state: &mut AppState) {
    use crate::state::workbench::{build_name, BuildDot, Scope};

    if state.project_path.is_none() {
        return;
    }
    let Some(build) = commands::of(state, Action::BuildProject) else {
        return;
    };
    let output = state.active_output_id();
    // ⚠ The name the person gave the build, as the Content panel and the Build tab say it: the id it is keyed by is no name of theirs.
    let label = match &output {
        Some(id) if state.project_carried.container.outputs.len() > 1 => {
            tr!(
                "Build {container}",
                container = build_name(&state.project_carried.container, id)
            )
        }
        _ => tr!("Build").to_string(),
    };
    let binding = build
        .binding
        .as_ref()
        .map(|b| format!(" ({b})"))
        .unwrap_or_default();
    let dot = output
        .as_deref()
        .map_or(BuildDot::Never, |id| state.workbench.dot(id));
    // ⚠ The dot is inside the button and a word goes with it: a state that rests on a colour is lost on some screens and to some eyes.
    let word = match output {
        Some(id) => dot.word(state.workbench.counts(Scope::Build(&id))),
        None => tr!("No build").to_string(),
    };
    let colour = match dot {
        BuildDot::Never => theme::TEXT_SECONDARY,
        BuildDot::Running => theme::INFO,
        BuildDot::Built => theme::SUCCESS,
        BuildDot::Failed => theme::ERROR,
        BuildDot::Blocked | BuildDot::Unchecked => theme::WARNING,
    };
    let mut text = egui::text::LayoutJob::default();
    for (part, colour) in [
        (
            format!("{} {label}  ", egui_phosphor::regular::HAMMER),
            None,
        ),
        ("●".to_string(), Some(colour)),
        (format!(" {word}"), None),
    ] {
        let part = egui::RichText::new(part).small();
        let part = match colour {
            Some(colour) => part.color(colour),
            None => part,
        };
        part.append_to(
            &mut text,
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Min,
        );
    }
    let response = ui.add_enabled(build.enabled.is_ok(), egui::Button::new(text));
    let response = match build.enabled {
        Ok(()) => response.on_hover_text(format!(
            "{}\n\n{}{binding}",
            dot.label(),
            build.hover.as_deref().unwrap_or_default()
        )),
        Err(why) => response.on_disabled_hover_text(why),
    };
    if response.clicked() {
        commands::run(state, ui.ctx(), &build);
    }
    if dot == BuildDot::Running {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// What the current edit mode permits, always visible, and the control that changes it.
///
/// ⚠ **Distinct per mode, and loudest for base-game editing**: a user who has
/// switched to it and forgotten is in the only state where a mistake is
/// unrecoverable. The read-only default is deliberately quiet, because a banner
/// that shouted in the safe state would be the one a user learns to ignore.
fn edit_mode_banner(ui: &mut egui::Ui, state: &mut AppState) {
    let Some((colour, text, hover)) = what_the_banner_says(state) else {
        return;
    };

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let caret = egui_phosphor::regular::CARET_DOWN;
        let title = egui::RichText::new(format!("{text} {caret}"))
            .color(colour)
            .small();
        ui.menu_button(title, |ui| {
            crate::ui::popup_of_the_shell(ui.ctx());
            ui.set_width(360.0);
            for command in commands::edit_modes(state) {
                let sentence = command.hover.as_deref().unwrap_or_default();
                let current = command.checked == Some(true);
                if super::edit_mode::choice(ui, &command.label, sentence, current).clicked() {
                    ui.close_kind(egui::UiKind::Menu);
                    commands::run(state, ui.ctx(), &command);
                }
            }
        })
        .response
        .on_hover_text(hover);
    });
}

/// What the banner says, and its hover: the mode's words over the install, the project's over what the mode does not guard, and nothing where there is nothing to guard.
fn what_the_banner_says(state: &AppState) -> Option<(theme::Themed, String, String)> {
    use crate::state::EditMode;
    use crate::ui::dock::Workspace;

    let in_a_project = state.project_path.is_some();
    // ⚠ A second window on a project another window writes says so, since the mode's words are about the game and not about this.
    if let Some(why) = state.read_only_reason().filter(|_| {
        in_a_project
            && (state.active_workspace.needs_a_project() || over_a_project_file(state).is_some())
    }) {
        return Some((
            theme::TEXT_SECONDARY,
            tr!("Project: read only here").to_string(),
            why,
        ));
    }
    // ⚠ A task screen writes the project's own files and the mode never guards them: `Read only` over it would be false.
    if in_a_project && state.active_workspace.is_task_screen() {
        return Some(project_editable(
            state,
            &tr!(
                "The {active_workspace} edits the project's own files",
                active_workspace = state.active_workspace.label()
            ),
        ));
    }
    // ⚠ This workspace writes the project and handles no document, whatever tab is open behind it, so no word is there to be false or to be learned as noise.
    if in_a_project && state.active_workspace == Workspace::Project {
        return None;
    }
    if let Some(over_a_project_file) = over_a_project_file(state) {
        return Some(over_a_project_file);
    }
    let mode = state.prefs.edit_mode;
    // ⚠ Read only is a word about the game's files: with none on screen (the start page, or the Project workspace, which draws no tab of the install) there is nothing to guard, and a word with nothing to guard teaches people to ignore the corner. The loud modes still speak, since they are the ones that can hurt.
    if mode == EditMode::ReadOnly && document_on_show(state).is_none() {
        return None;
    }
    let (colour, text) = match mode {
        EditMode::ReadOnly => (theme::TEXT_DIM, tr!("Read only")),
        EditMode::Overlay => (theme::INFO, tr!("Editing: mods overlay")),
        EditMode::Everything if state.retail_edit_session_allow => {
            (theme::ERROR, tr!("Editing: BASE GAME (confirmed)"))
        }
        EditMode::Everything => (theme::WARNING, tr!("Editing: base game")),
    };
    Some((colour, text.to_string(), mode.semantics().to_string()))
}

/// The document the screen on show draws: none on the start page, and none on the Project workspace unless it is a file of the project.
fn document_on_show(state: &AppState) -> Option<&crate::state::Document> {
    use crate::ui::dock::Workspace;

    if state.front_door.raised(state.active_workspace) {
        return None;
    }
    let doc = state.active_document()?;
    let own_screen = state.project_path.is_some() && state.active_workspace == Workspace::Project;
    (!own_screen || state.is_project_file(doc)).then_some(doc)
}

/// What the banner says over a file of the project, which the edit mode does not guard; none over anything else.
///
/// ⚠ **Honest about the document, not about the mode alone.** The mode guards the game's files; a file
/// of the project is outside them, so it is written whatever the mode says, and `Read only` over it would be false.
fn over_a_project_file(state: &AppState) -> Option<(theme::Themed, String, String)> {
    state
        .active_document()
        .filter(|doc| state.is_project_file(doc))?;
    Some(project_editable(
        state,
        tr!("A file of the project is not part of the game"),
    ))
}

/// The banner over what the project owns: neutral, since the game's edit mode guards none of it.
fn project_editable(state: &AppState, what: &str) -> (theme::Themed, String, String) {
    (
        theme::TEXT_SECONDARY,
        tr!("Project: editable").to_string(),
        tr!(
            "{what}, so the edit mode does not guard it. {semantics}",
            what = what,
            semantics = state.prefs.edit_mode.semantics()
        ),
    )
}

/// The widest thing the banner says, which the strip leaves room for so that it never jumps when the mode changes.
const BANNER_WIDEST: &str = tr_key!("Editing: BASE GAME (confirmed)");

/// Draws `command` as a menu item and runs it when clicked; a toggle leaves the menu open to show its new state.
pub(super) fn draw_command(ui: &mut egui::Ui, state: &mut AppState, command: &Command) {
    let enabled = command.enabled.is_ok();
    let response = match command.checked {
        Some(mut on) => ui.add_enabled(
            enabled,
            egui::Checkbox::new(&mut on, commands::atoms(command)),
        ),
        None => commands::menu_button(ui, command),
    };
    if commands::hints(response, command).clicked() {
        if command.checked.is_none() {
            ui.close_kind(egui::UiKind::Menu);
        }
        commands::run(state, ui.ctx(), command);
    }
}

fn item(ui: &mut egui::Ui, state: &mut AppState, action: Action) {
    if let Some(command) = commands::of(state, action) {
        draw_command(ui, state, &command);
    }
}

/// One submenu of everything opened lately, the projects, the files and the folders each under a heading, then the commands that clear each list; greyed with its reason while nothing has been opened.
fn recent_menu(ui: &mut egui::Ui, state: &mut AppState) {
    let lists = [
        (tr!("Projects"), commands::recent_projects(state)),
        (tr!("Files"), commands::recent_files(state)),
        (tr!("Folders"), commands::recent_folders(state)),
    ];
    let any = lists.iter().any(|(_, recent)| !recent.is_empty());
    ui.add_enabled_ui(any, |ui| {
        commands::submenu(ui, tr!("Open recent"), |ui| {
            for (heading, recent) in lists.iter().filter(|(_, recent)| !recent.is_empty()) {
                ui.label(
                    egui::RichText::new(*heading)
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
                for command in recent {
                    draw_command(ui, state, command);
                }
                ui.separator();
            }
            for clear in [
                Action::ClearRecentProjects,
                Action::ClearRecentFiles,
                Action::ClearRecentFolders,
            ] {
                item(ui, state, clear);
            }
        })
        .response
        .on_disabled_hover_text(tr!("Nothing has been opened lately"));
    });
}

/// ⚠ Opening a project is the most frequent act, so it sits beside New project…, ahead of the files.
fn file_menu(ui: &mut egui::Ui, state: &mut AppState) {
    ui.menu_button(tr!("File"), |ui| {
        crate::ui::popup_of_the_shell(ui.ctx());
        item(ui, state, Action::CreateProject);
        item(ui, state, Action::OpenProject);
        item(ui, state, Action::OpenProjectInNewWindow);
        item(ui, state, Action::OpenBuiltPack);
        ui.separator();
        item(ui, state, Action::Open);
        item(ui, state, Action::OpenFolder);
        recent_menu(ui, state);
        item(ui, state, Action::GoToFile);
        ui.separator();
        item(ui, state, Action::Save);
        item(ui, state, Action::SaveAs);
        item(ui, state, Action::SaveIntoArchiveCopy);
        ui.separator();
        item(ui, state, Action::CloseDocument);
        item(ui, state, Action::NewProject);
        ui.separator();
        item(ui, state, Action::OpenSettings);
        ui.separator();
        item(ui, state, Action::Exit);
    });
}

fn edit_menu(ui: &mut egui::Ui, state: &mut AppState) {
    ui.menu_button(tr!("Edit"), |ui| {
        crate::ui::popup_of_the_shell(ui.ctx());
        item(ui, state, Action::Undo);
        item(ui, state, Action::Redo);
        ui.separator();
        for action in [Action::Cut, Action::Copy, Action::Paste, Action::Delete] {
            item(ui, state, action);
        }
        ui.separator();
        item(ui, state, Action::CommandPalette);
    });
}

fn view_menu(ui: &mut egui::Ui, state: &mut AppState) {
    ui.menu_button(tr!("View"), |ui| {
        crate::ui::popup_of_the_shell(ui.ctx());
        item(ui, state, Action::StartPage);
        ui.separator();
        commands::submenu(ui, tr!("Workspace"), |ui| {
            for command in commands::workspaces(state) {
                let chosen = command.checked == Some(true);
                let response = ui.selectable_label(chosen, commands::atoms(&command));
                if commands::hints(response, &command).clicked() {
                    ui.close_kind(egui::UiKind::Menu);
                    commands::run(state, ui.ctx(), &command);
                }
            }
            let pins = commands::workspace_pins(state);
            if !pins.is_empty() {
                ui.separator();
                commands::submenu(ui, tr!("Buttons shown"), |ui| {
                    for command in &pins {
                        draw_command(ui, state, command);
                    }
                });
            }
        });
        commands::submenu(ui, tr!("Panels"), |ui| {
            // ⚠ A panel is listed once: floating one is the palette's command and its tab's own menu, and a row for each would double a list that cannot scroll.
            for command in commands::panels(state) {
                if !command.id.starts_with(commands::FLOAT_PANEL) {
                    draw_command(ui, state, &command);
                }
            }
        });
        item(ui, state, Action::ResetLayout);
        ui.separator();
        item(ui, state, Action::ShowGrid);
        item(ui, state, Action::ShowAxis);
    });
}

/// The Project menu, which the bar has with and without a project: with none its rows wait, each saying why, but the ones that make a project of what is open.
///
/// ⚠ A few rows and four submenus, so that every row is on screen in the smallest window: a popup does not scroll, and a
/// row below the screen is a command with no place. Each submenu's commands are in the palette under its path (`Project ▸ Fix`), and
/// the screens of a kind of content share one More.
fn project_menu_of_the_window(ui: &mut egui::Ui, state: &mut AppState) {
    ui.menu_button(tr!("Project"), |ui| {
        crate::ui::popup_of_the_shell(ui.ctx());
        item(ui, state, Action::UndoProject);
        item(ui, state, Action::RedoProject);
        ui.separator();
        // ⚠ By meaning: what goes into the project, the builds it declares, and the content it links to; Add files… and Import game files… are two rows a newcomer reads as one job unless something says they are not.
        item(ui, state, Action::AddContent);
        item(ui, state, Action::ImportGameFiles);
        item(ui, state, Action::ImportFolder);
        item(ui, state, Action::AddImagesAsTexturesSource);
        item(ui, state, Action::NewTexturesSource);
        ui.separator();
        item(ui, state, Action::AddBuild);
        item(ui, state, Action::EditBuild);
        ui.separator();
        item(ui, state, Action::LinkContent);
        // Greyed all the time where nothing is linked, with its reason only on hover, it is a row nobody can use.
        if state
            .project_carried
            .container
            .modules
            .iter()
            .any(|m| m.is_link())
        {
            item(ui, state, Action::UnlinkContent);
        }
        ui.separator();
        item(ui, state, Action::BuildProject);
        item(ui, state, Action::BuildAll);
        item(ui, state, Action::RevealBuild);
        item(ui, state, Action::CleanProject);
        ui.separator();
        // ⚠ The submenus of the screens of a kind of content are one More: there is one for each, and a popup does not scroll.
        let screens = crate::ui::task_screens::all();
        if !screens.is_empty() {
            commands::submenu(ui, crate::i18n::tr(MORE), |ui| {
                for screen in screens {
                    submenu(ui, state, crate::i18n::tr(screen.content), screen.menu);
                }
            });
        }
        submenu(
            ui,
            state,
            tr!("Fix"),
            &[
                Action::FixAllSafe,
                Action::OptimiseTextures,
                Action::MakeMaps,
                Action::BudgetTextures,
                Action::CompleteMaterials,
                Action::RepairModel,
                Action::CheckMesh,
                Action::AddLevels,
                Action::RepairRig,
                Action::CleanWeights,
                Action::MirrorModel,
                Action::CropUvs,
            ],
        );
        submenu(
            ui,
            state,
            tr!("History"),
            // ⚠ No undo of a kind of content: Edit ▸ Undo is that undo while its screen is on show, and the screen has its own buttons.
            &[
                Action::UndoImport,
                Action::RestoreLastGood,
                Action::ReviewVersions,
            ],
        );
        commands::submenu(ui, tr!("Properties and sharing"), |ui| {
            for action in [
                Action::SaveAsTemplate,
                Action::ApplyTemplate,
                Action::CheckBeforeSharing,
                Action::OpenProjectHere,
                Action::ProjectProperties,
                Action::RevealProject,
                Action::RemoveFromProject,
            ] {
                item(ui, state, action);
            }
            let members = commands::project_members(state);
            ui.add_enabled_ui(!members.is_empty(), |ui| {
                commands::submenu(ui, tr!("Remove from project"), |ui| {
                    for member in &members {
                        draw_command(ui, state, member);
                    }
                });
            });
        });
        ui.separator();
        item(ui, state, Action::MakeFolderAProject);
        item(ui, state, Action::SaveProject);
        item(ui, state, Action::SaveProjectAs);
        ui.separator();
        item(ui, state, Action::CloseProject);
    });
}

/// A submenu of the commands `actions`, in that order.
fn submenu(ui: &mut egui::Ui, state: &mut AppState, title: &str, actions: &[Action]) {
    commands::submenu(ui, title, |ui| {
        for &action in actions {
            item(ui, state, action);
        }
    });
}

/// The ways to see the open file, as buttons; the ones that do not fit go under More, since a strip never wraps.
fn workspace_buttons(ui: &mut egui::Ui, state: &mut AppState) {
    let prev_spacing = ui.spacing().item_spacing.x;
    let gap = 4.0;
    ui.spacing_mut().item_spacing.x = gap;
    let offered = commands::workspaces(state);
    let shown = state.workspaces_in_the_strip();
    let in_strip: Vec<&Command> = offered
        .iter()
        .filter(|command| shown.iter().any(|ws| command.label == ws.label()))
        .collect();

    // What the banner takes is kept clear of the buttons.
    let font = egui::TextStyle::Small.resolve(ui.style());
    let banner_width = ui
        .painter()
        .layout_no_wrap(
            format!("{BANNER_WIDEST} {}", egui_phosphor::regular::CARET_DOWN),
            font,
            egui::Color32::WHITE,
        )
        .size()
        .x
        + 2.0 * ui.spacing().button_padding.x
        + 16.0;
    let build_room = if state.project_path.is_some() {
        BUILD_ROOM
    } else {
        0.0
    };
    let room = (ui.available_width() - banner_width - build_room).max(0.0);
    let widths: Vec<f32> = in_strip
        .iter()
        .map(|command| strip_button_width(ui, &command.label))
        .collect();
    let more = strip_button_width(ui, crate::i18n::tr(MORE));
    let active = in_strip.iter().position(|c| c.checked == Some(true));
    let (visible, _) = crate::state::workbench::split_strip(&widths, gap, room, more, active);

    let mut after_a_file_kind = false;
    for &i in &visible {
        let task = crate::ui::dock::Workspace::ALL
            .iter()
            .any(|ws| ws.label() == in_strip[i].label && ws.is_task_screen());
        // The screens of a kind of content follow a separator from the screens of a kind of file.
        if task && after_a_file_kind {
            ui.separator();
        }
        after_a_file_kind |= !task;
        draw_workspace_button(ui, state, in_strip[i]);
    }
    // ⚠ Only what the strip had no room for: a workspace of a kind of file that cannot show the open file is in View ▸ Workspace and the palette, and not behind a button here.
    let under_more: Vec<&Command> = (in_strip.iter().enumerate())
        .filter(|(i, _)| !visible.contains(i))
        .map(|(_, command)| *command)
        .collect();
    if !under_more.is_empty() {
        commands::submenu(ui, crate::i18n::tr(MORE), |ui| {
            crate::ui::popup_of_the_shell(ui.ctx());
            for command in &under_more {
                let response =
                    ui.selectable_label(command.checked == Some(true), commands::atoms(command));
                if commands::hints(response, command).clicked() {
                    ui.close_kind(egui::UiKind::Menu);
                    commands::run(state, ui.ctx(), command);
                }
            }
            let pins = commands::workspace_pins(state);
            if !pins.is_empty() {
                ui.separator();
                commands::submenu(ui, tr!("Buttons shown"), |ui| {
                    for command in &pins {
                        draw_command(ui, state, command);
                    }
                });
            }
        });
    }
    ui.spacing_mut().item_spacing.x = prev_spacing;
}

const MORE: &str = tr_key!("More");

/// How wide the button for `label` draws: its text and padding, and never less than a workspace button's width.
fn strip_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, egui::Color32::WHITE)
        .size()
        .x;
    (text + 2.0 * ui.spacing().button_padding.x).max(theme::WORKSPACE_BUTTON_WIDTH)
}

fn draw_workspace_button(ui: &mut egui::Ui, state: &mut AppState, command: &Command) {
    let active = command.checked == Some(true);

    let bg_fill = theme::workspace_button_fill(active);
    let fg_color = theme::workspace_button_fg(active);

    let label = egui::RichText::new(&command.label).color(fg_color);
    let button = egui::Button::new(label)
        .fill(bg_fill)
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_TINY))
        .min_size(egui::vec2(
            theme::WORKSPACE_BUTTON_WIDTH,
            theme::TAB_STRIP_BUTTON_HEIGHT,
        ));

    let response = commands::hints(ui.add(button), command);
    if response.clicked() && !active {
        commands::run(state, ui.ctx(), command);
    }
}

fn tools_menu(ui: &mut egui::Ui, state: &mut AppState) {
    ui.menu_button(tr!("Automate"), |ui| {
        crate::ui::popup_of_the_shell(ui.ctx());
        item(ui, state, Action::CommandLog);
        item(ui, state, Action::Batch);
        item(ui, state, Action::BakeShaders);
        item(ui, state, Action::HarvestGta6Names);
        ui.separator();
        ui.label(
            egui::RichText::new(state.game_pack.status().short_label())
                .color(theme::TEXT_SECONDARY),
        );

        ui.separator();
        commands::submenu(ui, tr!("Run plugin"), |ui| {
            // The first call reads the plugins directory.
            state.installed_plugins();
            let plugins = commands::plugins(state);
            if plugins.is_empty() {
                ui.label(egui::RichText::new(tr!("No plugins installed")).weak());
            }
            for plugin in &plugins {
                draw_command(ui, state, plugin);
            }
            ui.separator();
            item(ui, state, Action::PluginSettings);
            item(ui, state, Action::InstallPluginFolder);
            item(ui, state, Action::InstallPluginFile);
        });
        let added = commands::plugin_commands(state);
        if !added.is_empty() {
            commands::submenu(ui, tr!("Plugin commands"), |ui| {
                for command in &added {
                    draw_command(ui, state, command);
                }
            });
        }
    });
}

fn help_menu(ui: &mut egui::Ui, state: &mut AppState) {
    ui.menu_button(tr!("Help"), |ui| {
        crate::ui::popup_of_the_shell(ui.ctx());
        item(ui, state, Action::OpenGuide);
        item(ui, state, Action::TakeTour);
        item(ui, state, Action::OpenSampleProject);
        item(ui, state, Action::KeyboardShortcuts);
        ui.separator();
        item(ui, state, Action::Log);
        item(ui, state, Action::Statistics);
        item(ui, state, Action::CrashReports);
        ui.separator();
        item(ui, state, Action::About);
    });
}

pub(crate) fn save_dialog_for_active(state: &AppState) -> Option<std::path::PathBuf> {
    save_dialog_for(state, state.active_document?)
}

/// The Save As dialog for a document, named after it.
pub(crate) fn save_dialog_for(state: &AppState, id: DocumentId) -> Option<std::path::PathBuf> {
    if let Some(name) = state.gta6_tree_copy_name(id) {
        return file_dialogs::save_gta6_archive_copy(&name);
    }
    let doc = state.document(id)?;
    file_dialogs::save_any_file(doc.kind, Some(&save_as_name(doc)))
}

/// The name Save As proposes for `doc`: its file's, with the extension it was opened with.
fn save_as_name(doc: &crate::state::Document) -> String {
    let name = doc.file_name();
    let extension = doc.kind.extension();
    if extension.is_empty() || std::path::Path::new(&name).extension().is_some() {
        name
    } else {
        format!("{name}.{extension}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the menu bar, returning where each labelled widget is.
    fn frame(
        ctx: &egui::Context,
        state: &mut AppState,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Pos2)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            events,
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| draw(ui, state));
        output
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let b = node.bounds()?;
                        let centre =
                            egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
                        Some((node.label()?.to_string(), centre))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn click(ctx: &egui::Context, state: &mut AppState, label: &str) {
        let at = frame(ctx, state, Vec::new())
            .into_iter()
            .find(|(l, _)| l.starts_with(label))
            .unwrap_or_else(|| panic!("no `{label}` in the menu bar"))
            .1;
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, state, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(ctx, state, vec![press(false)]);
    }

    #[test]
    fn save_as_proposes_the_files_name_with_its_extension_once() {
        use crate::state::{DocPayload, DocumentKind, TextDocument};

        let mut state = AppState::default();
        let particles = state.new_empty_document(DocumentKind::Ypt);
        let notes = state.new_empty_document(DocumentKind::Text);
        let proposed = |state: &AppState, id| save_as_name(state.document(id).expect("open"));
        assert_eq!(proposed(&state, particles), "Untitled.ypt");
        assert_eq!(proposed(&state, notes), "Untitled.txt");

        let xml = crate::state::Document::new(
            0,
            Some(std::path::PathBuf::from("C:/mods/handling.xml")),
            DocPayload::Text(TextDocument::empty()),
        );
        assert_eq!(
            save_as_name(&xml),
            "handling.xml",
            "a text file keeps the extension it was opened with"
        );
    }

    #[test]
    fn file_exit_asks_the_window_to_close() {
        let mut state = AppState::default();
        state.documents.clear();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        click(&ctx, &mut state, "File");
        click(&ctx, &mut state, "Exit");
        assert!(state.should_exit);
    }

    #[test]
    fn closing_everything_is_called_close_all_and_the_palette_still_finds_it_as_a_new_session() {
        let mut state = AppState::default();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        click(&ctx, &mut state, "File");
        let file: Vec<String> = frame(&ctx, &mut state, Vec::new())
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert!(
            file.iter().any(|label| label == "Close all"),
            "the File menu names no Close all: {file:?}"
        );
        for old in ["New Session", "New", "More Project Commands"] {
            assert!(!file.iter().any(|label| label == old), "{old}: {file:?}");
        }
        // ⚠ The id is the one a binding in `prefs.json` is kept under, and the words it had still find it.
        let command = commands::of(&state, Action::NewProject).expect("a command");
        assert_eq!(command.id, "new_project");
        assert_eq!(command.was, "New Session");
    }

    /// The edit mode's control and the workspace buttons draw and run the
    /// commands the palette offers: one label and one sentence each.
    #[test]
    fn the_edit_modes_and_the_workspaces_are_commands_the_menu_bar_and_the_palette_both_run() {
        use crate::state::{EditMode, Prefs};
        let mut state = AppState::from_prefs(Prefs::default());
        let document = state.new_empty_document(crate::state::DocumentKind::Text);
        state.set_active_document(document);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let modes: Vec<Command> = (commands::all(&state).into_iter())
            .filter(|command| command.category == "Edit mode")
            .collect();
        let labels: Vec<&str> = modes.iter().map(|c| c.label.as_str()).collect();
        let named: Vec<&str> = EditMode::ALL.iter().map(|m| m.label()).collect();
        assert_eq!(labels, named, "the palette offers each edit mode");

        let caret = egui_phosphor::regular::CARET_DOWN;
        let control = frame(&ctx, &mut state, Vec::new())
            .into_iter()
            .find(|(label, _)| label.ends_with(caret))
            .map(|(label, _)| label)
            .expect("the menu bar draws the edit mode");
        click(&ctx, &mut state, &control);
        let rows: Vec<String> = frame(&ctx, &mut state, Vec::new())
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        for mode in &modes {
            let sentence = mode.hover.clone().unwrap_or_default();
            let row = format!("{}\n{sentence}", mode.label);
            assert!(
                rows.contains(&row),
                "the control has no row `{row}`: {rows:?}"
            );
        }
        // ⚠ Read only again at once: the mode is saved where the next test's state reads it.
        click(
            &ctx,
            &mut state,
            &format!("{}\n", EditMode::Overlay.label()),
        );
        assert_eq!(state.prefs.edit_mode, EditMode::Overlay);
        commands::run(&mut state, &ctx, &modes[0]);
        assert_eq!(state.prefs.edit_mode, EditMode::ReadOnly);

        // A text is seen one way, so the strip has no button to choose by; the workspaces stay commands of the palette.
        let bar = frame(&ctx, &mut state, Vec::new());
        for workspace in commands::workspaces(&state) {
            assert!(
                !bar.iter().any(|(label, _)| *label == workspace.label),
                "a button for {} leads to an editor with nothing in it",
                workspace.id
            );
        }
        // Particle effects are seen as effects or as their textures, and each way is a button that runs its command.
        let effects = state.new_empty_document(crate::state::DocumentKind::Ypt);
        state.set_active_document(effects);
        let bar = frame(&ctx, &mut state, Vec::new());
        for way in ["Particle", "Texture"] {
            assert!(
                bar.iter().any(|(label, _)| label == way),
                "no button shows the effects as {way}: {bar:?}"
            );
        }
        click(&ctx, &mut state, "Texture");
        assert_eq!(state.active_workspace, crate::ui::dock::Workspace::Texture);
    }

    /// ⚠ Each is offered, and waits with its reason until the focused panel has a row to act on.
    #[test]
    fn the_edit_menu_offers_no_command_that_does_nothing() {
        use crate::state::{DocPayload, DocumentKind, Prefs};
        let mut state = AppState::from_prefs(Prefs::default());
        let doc = state.new_empty_document(DocumentKind::Rpf);
        let Some(DocPayload::Rpf(rpf)) = state.document_mut(doc).map(|d| &mut d.payload) else {
            panic!("an empty archive document holds an archive");
        };
        let root = rpf.session.root_id();
        let file = rpf
            .session
            .add_file_copy(root, "a.ytd", vec![1; 4])
            .expect("a file");
        let ws = state.active_workspace;
        let files = crate::ui::panels::registry::ids::FILES;
        crate::ui::dock_commands::reveal(state.workspace_docks.get_mut(ws), files);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        click(&ctx, &mut state, "Edit");
        let offered: Vec<String> = frame(&ctx, &mut state, Vec::new())
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert!(
            offered.iter().any(|label| label.starts_with("Undo")),
            "the Edit menu did not open: {offered:?}"
        );
        for verb in ["Cut", "Copy", "Paste", "Delete…"] {
            assert!(
                offered.iter().any(|label| label.starts_with(verb)),
                "Edit does not offer {verb}: {offered:?}"
            );
        }
        let edits = [Action::Cut, Action::Copy, Action::Paste, Action::Delete];
        let enabled = |state: &AppState, action| commands::of(state, action).map(|c| c.enabled);
        for action in edits {
            assert!(
                enabled(&state, action).is_some_and(|e| e.is_err()),
                "{action:?} is offered with nothing to act on"
            );
        }
        state.selection.select(doc, file);
        for action in [Action::Cut, Action::Copy, Action::Delete] {
            assert_eq!(enabled(&state, action), Some(Ok(())), "{action:?}");
        }
    }

    /// The labelled widgets of the bar drawn on a screen `width` wide.
    fn frame_at(
        ctx: &egui::Context,
        state: &mut AppState,
        width: f32,
    ) -> Vec<(String, egui::Pos2)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 720.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| draw(ui, state));
        output
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let b = node.bounds()?;
                        let centre =
                            egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
                        Some((node.label()?.to_string(), centre))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn a_project_is_open(state: &mut AppState) {
        state.project_path = Some(std::path::PathBuf::from("C:/mods/police.openrage-proj"));
        state
            .project_carried
            .container
            .outputs
            .push(or_project::Output::fivem_resource(
                "fivem",
                "police_pack",
                &["files"],
            ));
    }

    #[test]
    fn the_project_menu_is_there_with_no_project_and_offers_what_a_project_does_once_one_is_open() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        let bar = frame_at(&ctx, &mut state, 1280.0);
        // The menu and no workspace button: the Project workspace is offered only while a project is open.
        let named = bar.iter().filter(|(label, _)| label == "Project").count();
        assert_eq!(
            named, 1,
            "a window with no project has the Project menu and no Project button: {bar:?}"
        );

        a_project_is_open(&mut state);
        let bar = frame_at(&ctx, &mut state, 1280.0);
        let (_, at) = bar
            .iter()
            .filter(|(label, _)| label == "Project")
            .min_by(|a, b| a.1.x.total_cmp(&b.1.x))
            .expect("the bar has a Project menu");
        let press = |pressed| egui::Event::PointerButton {
            pos: *at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &ctx,
            &mut state,
            vec![egui::Event::PointerMoved(*at), press(true)],
        );
        frame(&ctx, &mut state, vec![press(false)]);
        let opened: Vec<String> = frame(&ctx, &mut state, Vec::new())
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        for item in [
            "Add files…",
            "Add build…",
            "Edit build…",
            "Link content…",
            "Build",
            "Build all",
            "Clean…",
            "Save project",
            "Save project as…",
            "Show the build",
            "Import folder…",
            "Close project",
        ] {
            assert!(
                opened.iter().any(|label| label.starts_with(item)),
                "the Project menu offers no {item}: {opened:?}"
            );
        }
        // One word for taking a change back, here as in Edit.
        let undoing: Vec<&String> = (opened.iter())
            .filter(|label| label.starts_with("Undo") || label.starts_with("Take back"))
            .collect();
        assert!(
            !undoing.is_empty() && undoing.iter().all(|label| label.starts_with("Undo")),
            "the Project menu's ways back do not all say Undo: {undoing:?}"
        );
        // The ways back that sit in the History submenu say Undo too.
        let mut closed = AppState::from_prefs(crate::state::Prefs::default());
        a_project_is_open(&mut closed);
        for action in [Action::UndoImport, Action::UndoClothes, Action::UndoProject] {
            let label = commands::of(&closed, action).expect("a command").label;
            assert!(label.starts_with("Undo"), "{action:?} is called `{label}`");
        }
    }

    /// ⚠ With no project the Project menu is still there: its rows wait, but the ones that make a project of what is open.
    #[test]
    fn a_window_with_no_project_has_the_project_menu_whose_rows_wait_but_the_ones_that_make_one() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        assert!(state.project_path.is_none());
        click(&ctx, &mut state, "Project");
        let opened: Vec<String> = frame(&ctx, &mut state, Vec::new())
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        for item in [
            "Make this folder a project",
            "Save project",
            "Save project as",
        ] {
            assert!(
                opened.iter().any(|label| label.starts_with(item)),
                "the Project menu of a window with no project offers no {item}: {opened:?}"
            );
        }
        for action in [Action::SaveProject, Action::SaveProjectAs] {
            let command = commands::of(&state, action).expect("a table command");
            assert_eq!(
                command.enabled,
                Ok(()),
                "{action:?} waits with no project, where it makes one"
            );
            assert_eq!(command.category, "Project", "{action:?}");
        }
        for action in [
            Action::BuildProject,
            Action::AddContent,
            Action::CloseProject,
        ] {
            let command = commands::of(&state, action).expect("a table command");
            assert!(
                command.enabled.is_err(),
                "{action:?} runs with no project open"
            );
        }
    }

    #[test]
    fn the_strip_draws_the_ways_to_see_the_open_file_and_no_screen_of_the_project() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        a_project_is_open(&mut state);
        let all = commands::workspaces(&state);
        assert_eq!(all.len(), 7);
        // ⚠ Project names the menu too, so its button is the second widget of that name in the bar.
        let buttons = |bar: &[(String, egui::Pos2)]| -> Vec<String> {
            (all.iter().map(|command| command.label.clone()))
                .filter(|label| {
                    let named = bar.iter().filter(|(l, _)| l == label).count();
                    named > usize::from(label == "Project")
                })
                .collect()
        };

        let bar = frame_at(&ctx, &mut state, 1600.0);
        assert_eq!(
            buttons(&bar),
            Vec::<String>::new(),
            "with no file in front the strip is empty, the project's screens being tabs: {bar:?}"
        );
        assert!(
            !bar.iter().any(|(label, _)| label == crate::i18n::tr(MORE)),
            "nothing is under More while every button fits: {bar:?}"
        );

        let effects = state.new_empty_document(crate::state::DocumentKind::Ypt);
        state.set_active_document(effects);
        let bar = frame_at(&ctx, &mut state, 1600.0);
        assert_eq!(
            buttons(&bar),
            ["Texture", "Particle"],
            "the strip is the ways to see particle effects: {bar:?}"
        );
    }

    /// What the edit mode's control in the bar reads now, none when the bar draws none.
    fn banner_if_any(ctx: &egui::Context, state: &mut AppState) -> Option<String> {
        let caret = egui_phosphor::regular::CARET_DOWN;
        frame_at(ctx, state, 1280.0)
            .into_iter()
            .find(|(label, _)| label.ends_with(caret))
            .map(|(label, _)| label.trim_end_matches(caret).trim_end().to_string())
    }

    fn banner_in_the_bar(ctx: &egui::Context, state: &mut AppState) -> String {
        banner_if_any(ctx, state).expect("the bar draws the edit mode")
    }

    #[test]
    fn the_banner_says_nothing_over_a_project_with_no_document_and_never_read_only_over_a_wardrobe()
    {
        use crate::ui::dock::Workspace;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        assert_eq!(
            banner_if_any(&ctx, &mut state),
            None,
            "with nothing open there is nothing for Read only to guard"
        );
        let document = state.new_empty_document(crate::state::DocumentKind::Text);
        state.set_active_document(document);
        assert_eq!(
            banner_if_any(&ctx, &mut state).as_deref(),
            Some("Read only"),
            "with no project the banner is today's over a file"
        );

        state.documents.clear();
        state.active_document = None;
        a_project_is_open(&mut state);
        state.active_workspace = Workspace::Project;
        assert_eq!(
            banner_if_any(&ctx, &mut state),
            None,
            "the Project workspace with no document guards nothing, so it says nothing"
        );

        state.active_workspace = Workspace::Wardrobe;
        assert_eq!(
            banner_in_the_bar(&ctx, &mut state),
            "Project: editable",
            "the Wardrobe writes the project's files, which the mode does not guard"
        );
        // The other screens keep the mode's words, over a project too, while the install is what they show.
        let dir = tempfile::tempdir().expect("a folder");
        let install = state.open_game_root(dir.path().to_path_buf());
        state.set_active_document(install);
        state.active_workspace = Workspace::Browse;
        assert_eq!(banner_in_the_bar(&ctx, &mut state), "Read only");
    }

    #[test]
    fn the_banner_follows_the_document_the_install_keeps_its_modes_and_a_project_file_is_editable()
    {
        use crate::state::{DocumentKind, EditMode};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let dir = tempfile::tempdir().expect("a folder");
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        let id = state.new_empty_document(DocumentKind::Text);
        let file = dir.path().join("client.lua");
        std::fs::write(&file, b"x").unwrap();
        state.document_mut(id).expect("open").path = Some(file);
        state.set_active_document(id);
        assert_eq!(
            banner_in_the_bar(&ctx, &mut state),
            "Read only",
            "with no project the banner is today's"
        );

        a_project_is_open(&mut state);
        assert_eq!(banner_in_the_bar(&ctx, &mut state), "Project: editable");

        // The edit mode still guards the install, and says so over it.
        state.prefs.edit_mode = EditMode::Overlay;
        let install = state.open_game_root(dir.path().to_path_buf());
        state.set_active_document(install);
        assert_eq!(banner_in_the_bar(&ctx, &mut state), "Editing: mods overlay");

        // And it is neutral over the project's file, however loud the mode is.
        state.prefs.edit_mode = EditMode::Everything;
        state.set_active_document(id);
        assert_eq!(banner_in_the_bar(&ctx, &mut state), "Project: editable");
        assert!(
            over_a_project_file(&state)
                .is_some_and(|(_, _, hover)| hover.contains("does not guard")),
            "the hover says why"
        );
    }
}
