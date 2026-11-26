use crate::diagnostics::log_buffer::LogBuffer;
use crate::state::front_door::counted;
use crate::state::{capabilities, AppState, DocPayload, DocumentKind, Game, LoadingState};
use crate::ui::commands::{self, Action};
use crate::ui::panels::game_picker::found_install;
use crate::ui::panels::settings::{open_settings_at, SettingsCategory};
use crate::ui::theme;

/// Whether the status bar's game switcher is open.
#[derive(Default)]
pub struct GameSwitcher {
    pub open: bool,
}

impl crate::ui::panel_state::PanelState for GameSwitcher {
    const KEY: &'static str = "game-switcher";
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::bottom("status_bar")
        .min_size(22.0)
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                draw_left(ui, state);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    crate::ui::panels::jobs::draw_indicator(ui, state);
                    draw_right(ui, state)
                });
            });
        });
}

/// The title read by scan whose remembered folder holds the tab on screen: with no game active,
/// its files are still that game's.
fn scanned_title_of_the_tab(state: &AppState) -> Option<Game> {
    let path = state.active_document()?.path.as_deref()?;
    Game::all().find(|&game| {
        crate::state::gta_vi::read_by_scan(game)
            && (state.prefs.game_paths.get(game)).is_some_and(|root| path.starts_with(root))
    })
}

/// GTA VI while the World screen flies through the GTA VI folder Settings names, which is read by scan and never the open install; not once that folder was found to hold no world.
fn world_title(state: &AppState) -> Option<Game> {
    let on = state.active_workspace == crate::ui::dock::Workspace::World;
    let shown = on && crate::ui::panels::world_viewer::offered(state);
    let failed = || {
        crate::state::gta6_world::worldview::viewer()
            .failed
            .is_some()
    };
    (shown && !failed()).then_some(Game::GtaVI)
}

/// The game whose install a launch opens; a click lists the installs found to switch to.
fn draw_game_switcher(ui: &mut egui::Ui, state: &mut AppState) {
    let (label, hover) = match state.prefs.active_game {
        Some(game) => (
            game.label(),
            tr!(
                "OpenRage opens the {edition_label} install at launch. Click to switch to another \
                 install found on this machine.",
                edition_label = game.generation().edition_label()
            ),
        ),
        None => match scanned_title_of_the_tab(state).or_else(|| world_title(state)) {
            Some(title) => (title.label(), title.profile().description().to_string()),
            None => (
                tr!("No game"),
                tr!("No install is open. Click to open one found on this machine.").to_string(),
            ),
        },
    };
    let text = format!("{} {label}", egui_phosphor::regular::GAME_CONTROLLER);
    let response = ui
        .add(egui::Button::new(egui::RichText::new(text).small()).frame(false))
        .on_hover_text(hover);
    let mut open = state.panels.get::<GameSwitcher>().open;
    if response.clicked() {
        open = !open;
    }
    egui::Popup::menu(&response)
        .open_bool(&mut open)
        .show(|ui| draw_installs_found(ui, state));
    state.panels.get::<GameSwitcher>().open = open;
    ui.separator();
}

fn draw_installs_found(ui: &mut egui::Ui, state: &mut AppState) {
    crate::ui::popup_of_the_shell(ui.ctx());
    let open_root = state.prefs.active_game_root().cloned();
    let mut chosen = None;
    let mut found_any = false;
    for game in Game::all() {
        let Some(root) = found_install(state, game) else {
            continue;
        };
        found_any = true;
        let current = state.prefs.active_game == Some(game) && open_root.as_ref() == Some(&root);
        let row = ui
            .add_enabled(
                game.profile().is_supported() || crate::state::gta_vi::read_by_scan(game),
                egui::Button::selectable(current, format!("{}   {}", game.label(), root.display())),
            )
            .on_disabled_hover_text(game.profile().description());
        if row.clicked() {
            chosen = Some((game, root));
            ui.close_kind(egui::UiKind::Menu);
        }
    }
    if !found_any {
        ui.label(egui::RichText::new(tr!("No install found")).color(theme::TEXT_SECONDARY));
    }
    if state.discovery.is_running() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                egui::RichText::new(tr!("Looking for installs…")).color(theme::TEXT_SECONDARY),
            );
        });
    }
    ui.separator();
    if ui.button(tr!("Settings ▸ Games…")).clicked() {
        open_settings_at(state, SettingsCategory::Games);
        ui.close_kind(egui::UiKind::Menu);
    }
    if let Some((game, root)) = chosen {
        state.request_game_switch(game, root);
    }
}

fn draw_left(ui: &mut egui::Ui, state: &mut AppState) {
    draw_game_switcher(ui, state);
    draw_project(ui, state);

    let active_snapshot = state.active_document().map(|doc| {
        let path = doc.location();
        // ⚠ A comparison's two paths would lose their middle together, and read as one path.
        let shown = match &doc.payload {
            DocPayload::Compare(compared) => {
                let [left, right] = compared.places.each_ref().map(|place| leaf(place));
                tr!("{left} compared with {right}", left = left, right = right)
            }
            // ⚠ The install is not the project's, and a bare path after the project's Build button reads as where the project is.
            _ if doc.kind == DocumentKind::GameRoot => {
                tr!("Game folder: {path}", path = elide_middle(&path, 80))
            }
            _ => elide_middle(&path, 80),
        };
        ActiveDocSnapshot {
            path,
            shown,
            dirty: doc.dirty,
        }
    });
    if let Some(snap) = active_snapshot {
        let label = egui::RichText::new(&snap.shown)
            .small()
            .color(theme::TEXT_PRIMARY);
        let resp = ui
            .add(egui::Label::new(label).sense(egui::Sense::click()))
            .on_hover_text(tr!("{path}\n\nClick to copy full path", path = snap.path));
        if resp.clicked() {
            ui.ctx().copy_text(snap.path.clone());
            state.show_toast(tr!("Path copied"));
        }
        if snap.dirty {
            ui.label(egui::RichText::new("●").small().color(theme::WARNING))
                .on_hover_text(tr!("Unsaved changes"));
        }
        ui.separator();
        let summary = selection_summary(state);
        if !summary.is_empty() {
            ui.label(
                egui::RichText::new(summary)
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
        }
    } else {
        ui.label(
            egui::RichText::new(nothing_open_hint(state))
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    }
}

/// What the project's name in the status bar lists: the Project workspace, the recent projects, the start page and Close project.
fn draw_project_menu(ui: &mut egui::Ui, state: &mut AppState) {
    crate::ui::popup_of_the_shell(ui.ctx());
    if let Some(open) = state.project_path.clone() {
        ui.label(
            egui::RichText::new(open.display().to_string())
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    }
    if ui.button(tr!("Project workspace")).clicked() {
        state.active_workspace = crate::ui::dock::Workspace::Project;
        ui.close_kind(egui::UiKind::Menu);
    }
    // ⚠ The commands the File menu and the palette hold, so a recent project is one action wherever it is offered.
    let recent = commands::other_recent_projects(state);
    if !recent.is_empty() {
        ui.separator();
        ui.label(
            egui::RichText::new(tr!("Recent projects"))
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    }
    for command in recent.iter().take(RECENT_PROJECTS_LISTED) {
        super::menu::draw_command(ui, state, command);
    }
    ui.separator();
    for action in [Action::StartPage, Action::CloseProject] {
        if super::project::command_button(ui, state, action) {
            ui.close_kind(egui::UiKind::Menu);
        }
    }
}

/// Whether the status bar's project menu is open.
#[derive(Default)]
pub struct ProjectMenu {
    pub open: bool,
}

impl crate::ui::panel_state::PanelState for ProjectMenu {
    const KEY: &'static str = "project-menu";
}

/// How many other recent projects the project menu lists.
const RECENT_PROJECTS_LISTED: usize = 5;

/// The open project's name, which opens its menu; nothing with no project. The Build button is the menu bar's.
fn draw_project(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(name) = state.project_name() else {
        return;
    };
    let unsaved = if state.project_unsaved() { " ●" } else { "" };
    let named = ui
        .add(
            egui::Button::new(
                egui::RichText::new(format!(
                    "{} {name}{unsaved}",
                    egui_phosphor::regular::FOLDER_OPEN
                ))
                .small(),
            )
            .frame(false),
        )
        .on_hover_text(tr!("{project_path}\n\nClick for the Project workspace, the recent projects and the start page.", project_path = state
                .project_path
                .as_ref()
                .map_or_else(String::new, |p| p.display().to_string())));
    let mut open = state.panels.get::<ProjectMenu>().open;
    if named.clicked() {
        open = !open;
    }
    egui::Popup::menu(&named)
        .open_bool(&mut open)
        .show(|ui| draw_project_menu(ui, state));
    state.panels.get::<ProjectMenu>().open = open;
    ui.separator();
}

/// Where to open a file from, and its key as the user bound it.
fn nothing_open_hint(state: &AppState) -> String {
    use crate::ui::dock::Workspace;

    // ⚠ On a screen of the project the hint is the project's: what a drop does there and the command that does it from a picker.
    let projects = match (state.project_path.is_some(), state.active_workspace) {
        (true, Workspace::Project) => Some((
            tr!("Drop game files here to import them"),
            Action::ImportGameFiles,
        )),
        (true, ws) => crate::ui::task_screens::of_workspace(ws)
            // A screen that is also the editor of a kind of file says it only while the project holds the content.
            .filter(|screen| {
                !screen.edits_files
                    || (state.project_carried.container.modules.iter())
                        .any(|m| m.kind == screen.kind)
            })
            .map(|screen| (crate::i18n::tr(screen.drop_hint), screen.drop)),
        _ => None,
    };
    if let Some((drop, action)) = projects {
        if let Some(command) = commands::of(state, action) {
            return tr!(
                "{drop}, or {command}",
                drop = drop,
                command = command.label.trim_end_matches('…')
            );
        }
    }
    let Some(open) = commands::of(state, Action::Open) else {
        return tr!("No document open").to_string();
    };
    let binding = open.binding.map(|b| format!(" ({b})")).unwrap_or_default();
    tr!(
        "No document open — {category} ▸ {open}{binding}",
        binding = binding,
        category = open.category,
        open = open.label
    )
}

fn draw_texture_resolution(ui: &mut egui::Ui, state: &AppState) {
    let report = &state.texture_report;
    if report.is_empty() || report.document != state.active_document {
        return;
    }
    let clean = report.is_clean();
    let colour = if clean {
        theme::TEXT_SECONDARY
    } else {
        theme::WARNING
    };
    let mut hover = tr!(
        "{resolved} of {requested} material textures resolved.",
        resolved = report.resolved(),
        requested = report.requested()
    );
    match report.primary_source() {
        Some(origin) => hover.push_str(&tr!(
            "\nMostly from {describe}.",
            describe = origin.describe()
        )),
        None => hover.push_str(tr!("\nNothing open supplies them.")),
    }
    if !clean {
        hover.push_str(tr!("\n\nOpen the Viewport panel's texture badge for the per-texture breakdown, and to pin a dictionary."));
    }
    ui.label(
        egui::RichText::new(format!("\u{25A4} {}", report.summary()))
            .small()
            .color(colour),
    )
    .on_hover_text(hover);
    ui.separator();
}

/// What the log entry says on hover: how many warnings and errors `buffer` counts, and the latest of them.
fn log_hover(buffer: &LogBuffer) -> String {
    let (warnings, errors) = buffer.counts();
    if (warnings, errors) == (0, 0) {
        return tr!("OpenRage has logged no warning or error this session. Click to open the Log.")
            .to_string();
    }
    let counted = tr!(
        "{errors} error(s) and {warnings} warning(s) from OpenRage this session. Click to \
         open the Log, where they can be read and copied.",
        errors = errors,
        warnings = warnings
    );
    match buffer.latest_own_serious() {
        Some(latest) => {
            let first = latest.message.lines().next().unwrap_or_default();
            tr!(
                "{counted}\n\nThe latest: {first}",
                counted = counted,
                first = elide_middle(first, 240)
            )
        }
        None => counted,
    }
}

/// How many warnings and errors OpenRage has logged this session; a click opens the Log.
fn draw_log_entry(ui: &mut egui::Ui, state: &mut AppState) {
    use egui_phosphor::regular as icon;

    let (warnings, errors) = crate::diagnostics::log_buffer::global().counts();
    let (text, colour) = match (warnings, errors) {
        (0, 0) => (
            tr!("{icon} Log", icon = icon::LIST_BULLETS),
            theme::TEXT_SECONDARY,
        ),
        (w, 0) => (
            tr!(
                "{icon} Log · {warnings}",
                icon = icon::WARNING,
                warnings = counted(w as usize, "warning")
            ),
            theme::WARNING,
        ),
        (0, e) => (
            tr!(
                "{icon} Log · {errors}",
                icon = icon::X_CIRCLE,
                errors = counted(e as usize, "error")
            ),
            theme::ERROR,
        ),
        (w, e) => (
            tr!(
                "{icon} Log · {errors}, {warnings}",
                icon = icon::X_CIRCLE,
                errors = counted(e as usize, "error"),
                warnings = counted(w as usize, "warning")
            ),
            theme::ERROR,
        ),
    };
    let hover = log_hover(crate::diagnostics::log_buffer::global());
    let response = ui
        .add(
            egui::Label::new(egui::RichText::new(text).small().color(colour))
                .sense(egui::Sense::click()),
        )
        .on_hover_text(hover);
    if response.clicked() {
        crate::ui::panels::log::reveal(state);
    }
    ui.separator();
}

fn draw_right(ui: &mut egui::Ui, state: &mut AppState) {
    draw_log_entry(ui, state);
    draw_texture_resolution(ui, state);

    if let LoadingState::Error(msg) = &state.loading {
        // ⚠ What failed, in the window's own words; what the reader said is the hover's and the Log's.
        ui.label(
            egui::RichText::new(tr!(
                "Error: {error_dialog_title}",
                error_dialog_title = state.error_dialog_title
            ))
            .small()
            .color(theme::ERROR),
        )
        .on_hover_text(msg);
        ui.separator();
    }

    let n = state.documents.len();
    if n > 0 {
        let any_dirty = state.any_dirty();
        let open = if n == 1 {
            tr!("1 tab open").to_string()
        } else {
            tr!("{n} tabs open", n = n)
        };
        let text = if any_dirty {
            format!("{open} ●")
        } else {
            open
        };
        let color = if any_dirty {
            theme::WARNING
        } else {
            theme::TEXT_SECONDARY
        };
        ui.label(egui::RichText::new(text).small().color(color));
    }
}

struct ActiveDocSnapshot {
    /// The document's location in full, which a hover shows and a click copies.
    path: String,
    /// What the status bar writes of it.
    shown: String,
    dirty: bool,
}

/// The last name in `place`, a path of either kind.
fn leaf(place: &str) -> &str {
    place.rsplit(['\\', '/']).next().unwrap_or(place)
}

fn selection_summary(state: &AppState) -> String {
    let Some(doc) = state.active_document() else {
        return String::new();
    };
    let Some(sel) = state.selection.selected(doc.id) else {
        return String::new();
    };
    capabilities::for_payload(&doc.payload)
        .and_then(|row| row.selection_summary)
        .map(|summary| summary(&doc.payload, sel))
        .unwrap_or_default()
}

pub(crate) fn elide_middle(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1) / 2;
    let prefix: String = s.chars().take(keep).collect();
    let suffix_skip = len.saturating_sub(keep);
    let suffix: String = s.chars().skip(suffix_skip).collect();
    format!("{prefix}…{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CloseAnswer, DocPayload, DocumentKind, Game, RpfEntryBinding};

    /// Draws the status bar, returning each labelled widget and its centre.
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
        // ⚠ Both bars: the window's one Build button is the menu bar's, and the project's name is the status bar's.
        let output = ctx.run_ui(input, |ui| {
            crate::ui::panels::menu::draw(ui, state);
            draw(ui, state);
        });
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

    fn click(ctx: &egui::Context, state: &mut AppState, naming: &str) {
        let at = frame(ctx, state, Vec::new())
            .into_iter()
            .find(|(label, _)| label.contains(naming))
            .unwrap_or_else(|| panic!("nothing in the status bar names `{naming}`"))
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

    /// What the log entry says on hover once `start` has run with the app's own subscriber as this thread's default.
    fn log_hover_after(start: impl FnOnce()) -> String {
        log_hover(&crate::diagnostics::captured(start))
    }

    /// Starts the app on the install `root` as a launch does, with no keys cached, and polls what that starts until the keys' job has ended.
    fn started_on(root: &std::path::Path, config: &std::path::Path) -> AppState {
        // ⚠ Every test shares one scratch configuration, where another may have cached keys.
        crate::state::prefs::keep_prefs_in(config);
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        assert!(
            !state.keys.has_keys(),
            "keys were loaded, so nothing is tested"
        );
        state.documents.clear();
        state.set_active_game(Game::GtaVLegacy, root.to_path_buf());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while state.keys.job().is_some() && std::time::Instant::now() < until {
            state.poll_key_extraction();
            state.poll_expansions();
            state.poll_jobs();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.keys.job().is_none(), "the keys' job never ended");
        state
    }

    #[test]
    fn a_clean_start_on_a_synthetic_install_with_no_keys_counts_no_warning() {
        let install = tempfile::tempdir().expect("an install folder");
        std::fs::write(install.path().join("x64a.rpf"), b"not an archive").expect("an archive");
        let config = tempfile::tempdir().expect("a configuration folder");
        let hover = log_hover_after(|| {
            started_on(install.path(), config.path());
        });
        assert!(
            hover.contains("logged no warning or error"),
            "a clean start counts something: {hover}"
        );
    }

    #[test]
    fn the_one_warning_a_start_counts_is_named_on_hover() {
        let install = tempfile::tempdir().expect("an install folder");
        std::fs::write(install.path().join("GTA5.exe"), b"not a program").expect("an executable");
        let config = tempfile::tempdir().expect("a configuration folder");
        let hover = log_hover_after(|| {
            started_on(install.path(), config.path());
        });
        assert!(
            hover.contains("1 error(s) and 0 warning(s)"),
            "the start counted other than one error: {hover}"
        );
        assert!(
            hover.contains("The latest: Key extraction failed"),
            "the count does not say what it counts: {hover}"
        );
    }

    #[test]
    fn a_comparison_says_what_it_compares_where_a_path_would_go() {
        use crate::state::compare::{Comparison, Found};
        let mut state = AppState::default();
        state.documents.clear();
        let comparison = Comparison {
            headings: ["a.ymt".into(), "b.ymt".into()],
            places: ["C:\\mods\\a.ymt".into(), "C:\\mods\\b.ymt".into()],
            found: Found::Lines(Vec::new()),
        };
        let mut doc = crate::state::Document::new(7, None, DocPayload::Compare(comparison));
        doc.name = "a.ymt ↔ b.ymt".into();
        state.documents.push(doc);
        state.set_active_document(7);

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui, &mut state));
        let shown: Vec<String> = (output.platform_output.accesskit_update.into_iter())
            .flat_map(|update| update.nodes)
            .filter_map(|(_, node)| Some(node.label().or_else(|| node.value())?.to_string()))
            .collect();
        assert!(
            !shown.iter().any(|label| label.contains("(unsaved)")),
            "a comparison is never saved, so it has no path to lack: {shown:?}"
        );
        assert!(
            shown
                .iter()
                .any(|label| label.contains("a.ymt") && label.contains("b.ymt")),
            "the status bar does not name both files: {shown:?}"
        );
    }

    #[test]
    fn the_frame_rate_is_left_to_the_statistics_panel() {
        let mut state = AppState::default();
        state.documents.clear();
        state.active_document = None;
        state.frame_stats.tick();
        std::thread::sleep(std::time::Duration::from_millis(2));
        state.frame_stats.tick();
        assert!(state.frame_stats.fps() > 0.0, "no frame was timed");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui, &mut state));
        let shown: Vec<String> = output
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| node.label().or_else(|| node.value()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            shown.iter().any(|text| text.contains("No document open")),
            "the status bar's own text was not read: {shown:?}"
        );
        assert!(
            !shown.iter().any(|text| text.ends_with(" fps")),
            "the status bar shows the frame rate: {shown:?}"
        );
    }

    #[test]
    fn the_status_bar_names_the_game_and_switches_to_an_install_found() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let (legacy, enhanced) = (dir.path().join("legacy"), dir.path().join("enhanced"));
        for root in [&legacy, &enhanced] {
            std::fs::create_dir(root).expect("an install folder");
        }
        let mut state = AppState::default();
        state.documents.clear();
        state.prefs.active_game = Some(Game::GtaVLegacy);
        state
            .prefs
            .game_paths
            .set(Game::GtaVLegacy, Some(legacy.clone()));
        state
            .prefs
            .game_paths
            .set(Game::GtaVEnhanced, Some(enhanced.clone()));
        let old = state.open_game_root(legacy.clone());
        // A blank is unsaved from the start; bound to the install, it is a file opened from it.
        let file = state.new_empty_document(DocumentKind::Text);
        state.document_mut(file).expect("open").rpf_entry_binding = Some(RpfEntryBinding {
            parent_doc_id: old,
            entry_id: 1,
        });
        if let Some(DocPayload::Rpf(rpf)) = state.document_mut(old).map(|d| &mut d.payload) {
            rpf.entry_documents.insert(1, file);
        }
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        click(&ctx, &mut state, Game::GtaVLegacy.label());
        click(&ctx, &mut state, Game::GtaVEnhanced.label());
        let pending = state
            .pending_close
            .clone()
            .expect("the switch did not go through the close request");
        assert_eq!(state.unsaved_documents(&pending.documents), [file]);
        assert!(
            state.document(old).is_some(),
            "the old install closed first"
        );
        state.answer_close(CloseAnswer::Cancel);
    }

    /// Every text of the status bar, from a frame of it: a button's label and a label's own words.
    fn texts(ctx: &egui::Context, state: &mut AppState) -> Vec<String> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        // ⚠ Both bars: the window's one Build button is the menu bar's, and the project's name is the status bar's.
        let output = ctx.run_ui(input, |ui| {
            crate::ui::panels::menu::draw(ui, state);
            draw(ui, state);
        });
        (output.platform_output.accesskit_update.into_iter())
            .flat_map(|update| update.nodes)
            .filter_map(|(_, node)| node.label().or_else(|| node.value()).map(str::to_string))
            .collect()
    }

    #[test]
    fn the_hint_over_a_project_workspace_is_the_projects_and_every_other_screen_keeps_its_own() {
        use crate::ui::dock::Workspace;
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let says = |state: &mut AppState, ws: Workspace, wanted: &str| {
            state.active_workspace = ws;
            let shown = texts(&ctx, state);
            assert!(
                shown.iter().any(|text| text.contains(wanted)),
                "{ws:?} says nothing like `{wanted}`: {shown:?}"
            );
            assert!(
                (ws == Workspace::Browse) == shown.iter().any(|t| t.contains("No document open")),
                "{ws:?} reads the install's hint over a project, or lacks it without one: {shown:?}"
            );
        };

        says(&mut state, Workspace::Browse, "No document open");
        state.project_path = Some(std::path::PathBuf::from("C:/mods/police.openrage-proj"));
        says(
            &mut state,
            Workspace::Project,
            "Drop game files here to import them",
        );
        says(
            &mut state,
            Workspace::Wardrobe,
            "Drop clothes here, or Add clothes",
        );
        says(&mut state, Workspace::Browse, "No document open");
    }

    #[test]
    fn the_build_button_says_how_the_build_stands_in_words_and_not_in_colour_alone() {
        use crate::state::workbench::{BuildDot, Counts};
        for (dot, errors, word) in [
            (BuildDot::Never, 0, "Not built"),
            (BuildDot::Running, 0, "Building…"),
            (BuildDot::Built, 0, "Built"),
            (BuildDot::Failed, 0, "Failed"),
            (BuildDot::Blocked, 19, "Blocked by 19 errors"),
            (BuildDot::Blocked, 1, "Blocked by 1 error"),
            (BuildDot::Unchecked, 0, "Check failed"),
        ] {
            let counts = Counts {
                errors,
                warnings: 2,
            };
            assert_eq!(dot.word(counts), word, "{dot:?}");
        }

        let dir = tempfile::tempdir().expect("a scratch folder");
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        state.project_path = Some(dir.path().join("police.openrage-proj"));
        state.poll_workbench();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let button = |state: &mut AppState| {
            texts(&ctx, state)
                .into_iter()
                .find(|text| text.contains("Build"))
                .expect("the bar has a Build button")
        };
        assert!(
            button(&mut state).ends_with("No build"),
            "no build is declared: {}",
            button(&mut state)
        );

        state
            .project_carried
            .container
            .outputs
            .push(or_project::Output::fivem_resource(
                "fivem",
                "pack",
                &["files"],
            ));
        assert!(
            button(&mut state).ends_with("Not built"),
            "{}",
            button(&mut state)
        );

        // Held: the build has started and nothing has taken what it said.
        state.build_selected();
        assert!(
            button(&mut state).ends_with("Building…"),
            "{}",
            button(&mut state)
        );
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while state.workbench.builds().last("fivem").is_none() {
            assert!(std::time::Instant::now() < until, "the build never ended");
            state.poll_workbench();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            button(&mut state).ends_with("Built"),
            "{}",
            button(&mut state)
        );
    }

    #[test]
    fn the_project_menu_lists_the_other_recent_projects_as_the_commands_the_table_has() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let [open, one, two] = ["alpha", "beta", "gamma"].map(|name| {
            let file = dir.path().join(format!("{name}.openrage-proj"));
            std::fs::write(&file, b"{}").expect("a project file");
            file
        });
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        state.prefs.recent_projects = vec![open.clone(), one, two];
        state.project_path = Some(open);
        state.poll_workbench();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        click(&ctx, &mut state, "alpha");
        let shown = texts(&ctx, &mut state);
        let others = commands::other_recent_projects(&state);
        assert_eq!(
            others.len(),
            2,
            "the open project is not its own recent one"
        );
        for command in &others {
            assert!(
                shown.contains(&command.label),
                "the menu has no row `{}`: {shown:?}",
                command.label
            );
        }
        assert!(
            !shown
                .iter()
                .any(|text| text.starts_with("alpha.openrage-proj")),
            "{shown:?}"
        );
    }

    #[test]
    fn the_status_bar_names_the_project_and_its_build_button_builds_it() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let labels = |ctx: &egui::Context, state: &mut AppState| -> Vec<String> {
            frame(ctx, state, Vec::new())
                .into_iter()
                .map(|(label, _)| label)
                .collect()
        };
        let shown = labels(&ctx, &mut state);
        assert!(
            !shown.iter().any(|label| label.contains("Build")),
            "a window with no project has no Build button: {shown:?}"
        );

        state.project_path = Some(dir.path().join("police.openrage-proj"));
        // The editor polls each frame, which is what starts a project's workbench.
        state.poll_workbench();
        let shown = labels(&ctx, &mut state);
        assert!(
            shown.iter().any(|label| label.ends_with("police")),
            "the project's name is on the bar: {shown:?}"
        );
        assert!(
            shown.iter().any(|label| label.contains("Build")),
            "{shown:?}"
        );

        // With no build declared the button waits, and with one it builds it.
        state
            .project_carried
            .container
            .outputs
            .push(or_project::Output::fivem_resource(
                "fivem",
                "pack",
                &["files"],
            ));
        click(&ctx, &mut state, "Build");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while state.workbench.builds().last("fivem").is_none() {
            assert!(std::time::Instant::now() < until, "the build never ended");
            state.poll_workbench();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            dir.path().join("build/pack/fxmanifest.lua").is_file(),
            "the button's build wrote the resource"
        );
    }
}
