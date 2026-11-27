//! What a workspace shows while it has nothing of its own: what it lacks, and
//! the actions that give it something.

use std::path::{Path, PathBuf};

use crate::platform::file_dialogs;
use crate::state::{AppState, DocumentKind, Game, LoadingState, RecentFile};
use crate::ui::commands::{self, Action};
use crate::ui::dock::Workspace;
use crate::ui::panels::game_picker::{found_install, recent_files};
use crate::ui::panels::registry::ids;
use crate::ui::task_screens;
use crate::ui::theme;

/// Whether the workspace on screen has nothing of its own to show.
pub fn has_nothing_to_show(state: &AppState) -> bool {
    // ⚠ A document of a kind the workspace does not handle, such as an install
    // outside Browse or a model in Data, is another workspace's: a panel here
    // that drew it would be drawing another kind's view.
    state
        .active_document()
        .is_none_or(|doc| !state.active_workspace.handles_kind(doc.kind))
}

/// Whether the Document panel draws the empty state, which the inspector then leaves to it.
pub fn drawn_by_the_document_panel(state: &AppState) -> bool {
    state
        .workspace_docks
        .shows(state.active_workspace, ids::DOCUMENT)
}

/// What the workspace lacks, in a few words.
pub fn heading(ws: Workspace) -> &'static str {
    if let Some(screen) = task_screens::of_workspace_alone(ws) {
        return crate::i18n::tr(screen.empty);
    }
    match ws {
        Workspace::Project => tr!("No project open"),
        Workspace::Browse => tr!("Nothing selected"),
        Workspace::Model => tr!("No model open"),
        Workspace::Texture => tr!("No textures open"),
        Workspace::Animation => tr!("No animation open"),
        Workspace::Particle => tr!("No particle effects open"),
        Workspace::Data => tr!("No data file open"),
        // A task screen's heading is in its row, which returned above.
        _ => tr!("Nothing to show"),
    }
}

/// The empty state of the workspace on screen: what it lacks, then the actions that open something.
pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    let ws = state.active_workspace;
    ui.vertical_centered(|ui| {
        ui.add_space(theme::SPACING_SECTION);
        ui.label(
            egui::RichText::new(heading(ws))
                .size(16.0)
                .color(theme::TEXT_SECONDARY),
        );
        ui.add_space(theme::SPACING_DEFAULT);
        if ws == Workspace::Project {
            draw_project_actions(ui, state);
        }
        if let Some(screen) = task_screens::of_workspace_alone(ws) {
            if let Some(command) = commands::of(state, screen.add) {
                let response = ui.add_enabled(
                    command.enabled.is_ok(),
                    egui::Button::new(crate::i18n::tr(screen.add_button)),
                );
                if commands::hints(response, &command).clicked() {
                    commands::run(state, ui.ctx(), &command);
                }
            }
        }
        let idle = !matches!(state.loading, LoadingState::Loading);
        if ui
            .add_enabled(idle, egui::Button::new(tr!("Open file…")))
            .clicked()
        {
            if let Some(paths) = file_dialogs::open_any_files() {
                state.load_files(paths);
            }
        }
        let install = state
            .documents
            .iter()
            .find(|d| d.kind == DocumentKind::GameRoot)
            .map(|d| d.id);
        match install {
            Some(id) => {
                if ui.button(tr!("Browse the install")).clicked() {
                    state.set_active_document(id);
                }
            }
            None => {
                for (game, root) in installs_offered(state) {
                    if ui.button(tr!("Open {game}", game = game.label())).clicked() {
                        state.request_game_switch(game, root);
                    }
                }
            }
        }
        if ws == Workspace::Particle
            && can_create(DocumentKind::Ypt)
            && ui.button(tr!("New particle effects")).clicked()
        {
            let id = state.new_empty_document(DocumentKind::Ypt);
            state.set_active_document(id);
        }
        draw_recent(ui, state, ws);
    });
}

/// What gives the Project workspace something: a project to open, or the session saved as one, and the ones opened lately.
fn draw_project_actions(ui: &mut egui::Ui, state: &mut AppState) {
    for action in [Action::OpenProject, Action::SaveProject] {
        let Some(command) = commands::of(state, action) else {
            continue;
        };
        let label = match action {
            Action::SaveProject => tr!("Save this session as a project…"),
            _ => tr!("Open project…"),
        };
        let response = ui.add_enabled(command.enabled.is_ok(), egui::Button::new(label));
        if commands::hints(response, &command).clicked() {
            commands::run(state, ui.ctx(), &command);
        }
    }
    let recent: Vec<PathBuf> = state
        .prefs
        .recent_projects
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect();
    if !recent.is_empty() {
        ui.add_space(theme::SPACING_DEFAULT);
        ui.label(egui::RichText::new(tr!("Recent projects")).color(theme::TEXT_SECONDARY));
        for path in recent {
            let name = path.file_stem().map_or_else(
                || path.display().to_string(),
                |s| s.to_string_lossy().into_owned(),
            );
            if ui
                .link(name)
                .on_hover_text(path.display().to_string())
                .clicked()
            {
                state.open_project_asking(path);
            }
        }
    }
    ui.add_space(theme::SPACING_DEFAULT);
}

/// The active game's install on disk, else every game whose folder is set and there, a title read by scan included.
///
/// ⚠ Every one and not the first: no game being active, the prefs do not say which folder was set last.
fn installs_offered(state: &AppState) -> Vec<(Game, PathBuf)> {
    let found = |game: Game| {
        let readable = game.profile().is_supported() || crate::state::gta_vi::read_by_scan(game);
        Some((game, found_install(state, game).filter(|_| readable)?))
    };
    if let Some(active) = state.prefs.active_game.and_then(found) {
        return vec![active];
    }
    Game::all().filter_map(found).collect()
}

fn can_create(kind: DocumentKind) -> bool {
    crate::state::kinds::creatable()
        .iter()
        .any(|row| row.info.kind == kind && row.available())
}

/// The files opened lately that open into `ws`.
fn draw_recent(ui: &mut egui::Ui, state: &mut AppState, ws: Workspace) {
    let recent: Vec<RecentFile> = state
        .prefs
        .recent_files
        .iter()
        .filter(|r| {
            DocumentKind::from_path(Path::new(r.name()))
                .is_some_and(|k| Workspace::for_kind(k) == ws)
        })
        .cloned()
        .collect();
    if let Some(recent) = recent_files(ui, tr!("Recent"), &recent) {
        state.open_recent(&recent);
    }
}

#[cfg(test)]
mod tests {
    use crate::state::AppState;
    use crate::ui::dock::Workspace;

    /// Each labelled widget one frame of the workspace on screen draws, with its centre.
    fn drawn(
        ctx: &egui::Context,
        state: &mut AppState,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Pos2)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| crate::ui::dock::draw(ui, state, None));
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

    /// The labels on screen once the dock has laid itself out.
    fn settled(ctx: &egui::Context, state: &mut AppState) -> Vec<(String, egui::Pos2)> {
        drawn(ctx, state, Vec::new());
        drawn(ctx, state, Vec::new())
    }

    fn click(ctx: &egui::Context, state: &mut AppState, at: egui::Pos2) {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        drawn(ctx, state, vec![egui::Event::PointerMoved(at), press(true)]);
        drawn(ctx, state, vec![press(false)]);
    }

    fn nothing_open() -> AppState {
        let mut state = AppState::default();
        state.documents.clear();
        state.active_document = None;
        state
    }

    /// ⚠ Twice over: with nothing open, as a first run without an install
    /// sees it, and with only the install open, as every later start does.
    #[test]
    fn every_workspaces_empty_state_offers_an_action() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for ws in Workspace::ALL {
            let mut state = nothing_open();
            state.active_workspace = ws;
            let shown = settled(&ctx, &mut state);
            assert!(
                shown.iter().any(|(label, _)| label.ends_with("Open file…")),
                "{ws:?} with nothing open offers nothing to open: {shown:?}"
            );
        }

        let install = tempfile::tempdir().expect("an install folder");
        for ws in Workspace::ALL {
            if ws == Workspace::Browse {
                // Browse shows the install itself.
                continue;
            }
            let mut state = nothing_open();
            let id = state.open_game_root(install.path().to_path_buf());
            state.active_document = Some(id);
            state.active_workspace = ws;
            let shown = settled(&ctx, &mut state);
            assert!(
                shown.iter().any(|(label, _)| label.ends_with("Open file…")),
                "{ws:?} with only the install open offers nothing to open: {shown:?}"
            );
            let browse = shown
                .iter()
                .find(|(label, _)| label == "Browse the install")
                .unwrap_or_else(|| panic!("{ws:?} does not offer the install: {shown:?}"))
                .1;
            click(&ctx, &mut state, browse);
            assert_eq!(
                (state.active_workspace, state.active_document),
                (Workspace::Browse, Some(id)),
                "{ws:?}'s Browse the install did not show the install"
            );
        }
    }

    /// ⚠ With no game active, a GTA VI folder set in Settings was passed over for a GTA V install found earlier.
    #[test]
    fn with_no_game_active_the_empty_state_offers_each_folder_set_gta_vi_included() {
        use or_title::Game;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let (enhanced, gta6) = (
            tempfile::tempdir().expect("a"),
            tempfile::tempdir().expect("b"),
        );
        let mut state = nothing_open();
        state.active_workspace = Workspace::Model;
        state.prefs.active_game = None;
        state.prefs.game_paths = Default::default();
        let paths = &mut state.prefs.game_paths;
        paths.set(Game::GtaVEnhanced, Some(enhanced.path().to_path_buf()));
        paths.set(Game::GtaVI, Some(gta6.path().to_path_buf()));
        let shown = settled(&ctx, &mut state);
        for game in [Game::GtaVI, Game::GtaVEnhanced] {
            let offer = tr!("Open {game}", game = game.label()).to_string();
            assert!(
                shown.iter().any(|(label, _)| *label == offer),
                "{offer} is not offered: {shown:?}"
            );
        }
    }

    /// ⚠ Data once drew a model's grid in its Document panel.
    #[test]
    fn a_workspace_the_active_document_does_not_belong_to_shows_its_empty_state() {
        use crate::state::DocumentKind;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for ws in Workspace::ALL {
            let kind = [DocumentKind::Ydr, DocumentKind::Ytd, DocumentKind::Ycd]
                .into_iter()
                .find(|&kind| !ws.handles_kind(kind))
                .expect("no workspace shows all three");
            let mut state = nothing_open();
            state.new_empty_document(kind);
            state.active_workspace = ws;
            let shown = settled(&ctx, &mut state);
            assert!(
                shown.iter().any(|(label, _)| label.ends_with("Open file…")),
                "{ws:?} with a .{} active offers nothing to open: {shown:?}",
                kind.extension()
            );
        }
    }
}
