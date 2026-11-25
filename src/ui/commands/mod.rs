//! Every command the app offers: [`COMMANDS`], the one table that the menus,
//! the context menus, the command palette, Settings ▸ Keyboard shortcuts and
//! the key dispatcher read, and the commands each recent file, panel, edit
//! mode, plugin and plugin command adds to it.

mod clipboard;
mod compare;
mod files;
mod front_door;
mod maps;
mod mesh;
mod particles;
mod predicates;
mod project;
mod table;
mod target;
mod text_find;
mod textures;
mod tools_and_help;
mod undo;
mod wardrobe;
mod weapons;

use std::path::{Path, PathBuf};

#[cfg(test)]
pub use clipboard::NOTHING_TO_PASTE;
pub use table::{spec, Action, Spec, COMMANDS};
pub use target::{focused_document, Hits, Target};

use crate::state::toasts::ToastAction;
use crate::state::workbench::NEEDS_A_PROJECT;
use crate::state::{AppState, EditMode, RecentFile};
use crate::ui::dock::Workspace;
use crate::ui::dock_commands::{self, Placement};
use crate::ui::panel_registry::PanelId;
use crate::ui::shortcuts;

/// A command as the menus and the palette offer it now.
#[derive(Debug, Clone)]
pub struct Command {
    /// Unique among the commands offered at once; a table command's is its [`Spec::id`].
    pub id: String,
    pub label: String,
    pub category: String,
    /// The user's current binding, as menus show it.
    pub binding: Option<String>,
    /// `Err` says why it cannot run now.
    pub enabled: Result<(), &'static str>,
    /// A toggle's or a choice's state.
    pub checked: Option<bool>,
    pub hover: Option<String>,
    /// The words it was found by before it was renamed, which the palette still answers to.
    pub was: &'static str,
    /// The `openrage-cli` line that does the same, with a `<name>` wherever the window does not know the argument.
    pub cli: Option<String>,
    run: Run,
}

#[derive(Debug, Clone)]
enum Run {
    Action(Action),
    On(Action, Target),
    OpenRecentFile(RecentFile),
    OpenRecentFolder(PathBuf),
    OpenRecentProject(PathBuf),
    RemoveFromProject(PathBuf),
    Workspace(Workspace),
    PinWorkspace(Workspace),
    EditMode(EditMode),
    TogglePanel(PanelId),
    FloatPanel(PanelId),
    /// Bring a panel forward in the side column of the screen in front.
    Side(PanelId),
    Plugin(String),
    PluginCommand {
        plugin: String,
        command: String,
    },
}

impl Command {
    fn new(id: String, label: String, category: &'static str, run: Run) -> Self {
        Self {
            id,
            label,
            category: crate::i18n::tr(category).to_string(),
            binding: None,
            enabled: Ok(()),
            checked: None,
            hover: None,
            was: "",
            cli: None,
            run,
        }
    }
}

/// `action`'s command as it reads now, acting on what the focused panel has selected where it acts on something.
pub fn of(state: &AppState, action: Action) -> Option<Command> {
    let spec = spec(action)?;
    if let Some(on) = &spec.on {
        return match (on.focused)(state) {
            Ok(target) => self::on(state, action, target),
            Err(why) => {
                let mut command = from(
                    state,
                    spec,
                    crate::i18n::tr(spec.label).to_string(),
                    Run::Action(action),
                );
                command.enabled = Err(why);
                Some(command)
            }
        };
    }
    let label = spec
        .label_now
        .map_or_else(|| crate::i18n::tr(spec.label).to_string(), |now| now(state));
    let mut command = from(state, spec, label, Run::Action(action));
    command.enabled = (spec.enabled)(state);
    Some(command)
}

/// `action`'s command acting on `target`, as a context menu offers it; `None` for a command that acts on nothing.
pub fn on(state: &AppState, action: Action, target: Target) -> Option<Command> {
    let spec = spec(action)?;
    let on = spec.on.as_ref()?;
    let label = on.label.map_or_else(
        || crate::i18n::tr(spec.label).to_string(),
        |label| label(state, &target),
    );
    let enabled = (on.enabled)(state, &target);
    let mut command = from(state, spec, label, Run::On(action, target));
    command.enabled = enabled;
    Some(command)
}

fn from(state: &AppState, spec: &Spec, label: String, run: Run) -> Command {
    let mut command = Command::new(spec.id.to_string(), label, spec.category, run);
    if !spec.was.is_empty() {
        command.was = crate::i18n::tr(spec.was);
    }
    command.cli = spec.cli.map(|line| line(state));
    command.binding = shortcuts::label_for(spec.action, &state.prefs);
    command.checked = spec.checked.map(|checked| checked(state));
    command.hover = (!spec.hover.is_empty()).then(|| crate::i18n::tr(spec.hover).to_string());
    command
}

/// Offers `action` on `target` as a context menu's item, keeping the command in `asked` once clicked.
///
/// ⚠ The command runs where `asked` is read, once the frame's rows are drawn,
/// so it never changes the tree a menu is drawn from.
pub fn offer(
    ui: &mut egui::Ui,
    state: &AppState,
    action: Action,
    target: &Target,
    asked: &mut Option<Command>,
) {
    if let Some(command) = on(state, action, target.clone()) {
        item(ui, command, asked);
    }
}

/// Offers `action`, which acts on no row, as a context menu's item, keeping the command in `asked` once clicked.
pub fn offer_plain(
    ui: &mut egui::Ui,
    state: &AppState,
    action: Action,
    asked: &mut Option<Command>,
) {
    if let Some(command) = of(state, action) {
        item(ui, command, asked);
    }
}

fn item(ui: &mut egui::Ui, command: Command, asked: &mut Option<Command>) {
    let response = menu_button(ui, &command);
    if hints(response, &command).clicked() {
        ui.close_kind(egui::UiKind::Menu);
        *asked = Some(command);
    }
}

/// Every command offered now: the table's, then those of the recents, the project, workspaces, edit modes, panels, plugins and what plugins registered.
pub fn all(state: &AppState) -> Vec<Command> {
    let mut all: Vec<Command> = COMMANDS
        .iter()
        .filter_map(|spec| of(state, spec.action))
        .collect();
    let groups: [fn(&AppState) -> Vec<Command>; 10] = [
        recent_files,
        recent_folders,
        recent_projects,
        project_members,
        workspaces,
        workspace_pins,
        edit_modes,
        panels,
        plugins,
        plugin_commands,
    ];
    for group in groups {
        all.extend(group(state));
    }
    all
}

/// The files opened lately that are still on disk, each saying when it was opened.
pub fn recent_files(state: &AppState) -> Vec<Command> {
    let category = tr_key!("File ▸ Open recent");
    (state.prefs.recent_files.iter())
        .filter(|recent| recent.exists())
        .map(|recent| {
            let id = format!("{category}: {}", recent.location());
            let label = match recent.opened_ago() {
                Some(when) => format!("{}    {when}", recent.name()),
                None => recent.name().to_string(),
            };
            let run = Run::OpenRecentFile(recent.clone());
            let mut command = Command::new(id, label, category, run);
            command.hover = Some(recent.location());
            command
        })
        .collect()
}

pub fn recent_folders(state: &AppState) -> Vec<Command> {
    let paths = &state.prefs.recent_folders;
    recent(
        paths,
        "recent folder",
        display_recent_folder,
        Run::OpenRecentFolder,
    )
}

/// The recent projects but the one open, which the project's own menu in the status bar lists.
pub fn other_recent_projects(state: &AppState) -> Vec<Command> {
    let others: Vec<PathBuf> = (state.prefs.recent_projects.iter())
        .filter(|path| state.project_path.as_ref() != Some(*path))
        .cloned()
        .collect();
    recent(
        &others,
        "recent project",
        display_recent,
        Run::OpenRecentProject,
    )
}

pub fn recent_projects(state: &AppState) -> Vec<Command> {
    let paths = &state.prefs.recent_projects;
    recent(
        paths,
        "recent project",
        display_recent,
        Run::OpenRecentProject,
    )
}

/// The recent `paths` of one `kind`, which starts each id: a file, a folder and a project share one path of the File menu.
fn recent(
    paths: &[PathBuf],
    kind: &str,
    label: fn(&Path) -> String,
    run: fn(PathBuf) -> Run,
) -> Vec<Command> {
    let category = tr_key!("File ▸ Open recent");
    paths
        .iter()
        .map(|path| {
            let id = format!("{kind}: {}", path.display());
            let mut command = Command::new(id, label(path), category, run(path.clone()));
            command.hover = Some(path.display().to_string());
            command
        })
        .collect()
}

/// The open project's members, each a command that takes it out of the project.
pub fn project_members(state: &AppState) -> Vec<Command> {
    if state.project_path.is_none() {
        return Vec::new();
    }
    let category = tr_key!("Project ▸ Properties and sharing ▸ Remove from project");
    state
        .snapshot_project()
        .documents
        .into_iter()
        .map(|member| {
            let path = member.path;
            let label = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let id = format!("{category}: {}", path.display());
            let mut command =
                Command::new(id, label, category, Run::RemoveFromProject(path.clone()));
            command.hover = Some(path.display().to_string());
            command
        })
        .collect()
}

/// The workspaces on offer, each with the key that goes to its place in the strip.
pub fn workspaces(state: &AppState) -> Vec<Command> {
    // A screen of the project is a tab, and its key is the number of its place in the tab row.
    let order = state.screen_tabs();
    state
        .workspaces_offered()
        .into_iter()
        .map(|ws| {
            let id = format!("workspace: {}", ws.label());
            let mut command = Command::new(
                id,
                ws.label().to_string(),
                tr_key!("View ▸ Workspace"),
                Run::Workspace(ws),
            );
            command.checked = Some(state.active_workspace == ws);
            command.hover = Some(ws.tooltip().to_string());
            command.binding = order
                .iter()
                .position(|place| {
                    use crate::state::workbench::TabPlace;
                    matches!(place, TabPlace::Screen(of) | TabPlace::Content(of, _) if *of == ws)
                })
                .and_then(|place| project::WORKSPACE_ACTIONS.get(place))
                .and_then(|action| shortcuts::label_for(*action, &state.prefs));
            command
        })
        .collect()
}

/// A button of the strip each workspace may be pinned as, checked while it is, while a project is open to remember it.
pub fn workspace_pins(state: &AppState) -> Vec<Command> {
    if state.project_path.is_none() {
        return Vec::new();
    }
    let pinned = state.workspaces_pinned();
    state
        .workspaces_offered()
        .into_iter()
        .map(|ws| {
            let mut command = Command::new(
                format!("workspace button: {ws}", ws = ws.label()),
                ws.label().to_string(),
                tr_key!("View ▸ Workspace ▸ Buttons shown"),
                Run::PinWorkspace(ws),
            );
            command.checked = Some(pinned.contains(&ws));
            command.hover = Some(
                tr!("Show this workspace as a button in the strip; the project remembers it")
                    .to_string(),
            );
            if pinned == [ws] {
                command.enabled = Err(tr!("One button always stays"));
            }
            command
        })
        .collect()
}

/// The three edit modes, the current one checked, each with its sentence.
pub fn edit_modes(state: &AppState) -> Vec<Command> {
    EditMode::ALL
        .into_iter()
        .map(|mode| {
            let id = format!("edit mode: {mode}", mode = mode.label());
            let run = Run::EditMode(mode);
            let mut command = Command::new(id, mode.label().to_string(), tr_key!("Edit mode"), run);
            command.checked = Some(state.prefs.edit_mode == mode);
            command.hover = Some(mode.semantics().to_string());
            command
        })
        .collect()
}

/// What the focused panel has selected, for a window that runs over it.
pub fn selected(state: &AppState) -> Result<Target, &'static str> {
    target::selection(state)
}

/// The command that brings the panel `id` forward in the side column of the screen in front, named `label`: what a room of the rail runs, and nothing a menu lists.
/// The command that goes to the workspace `ws`, where the window offers it.
pub fn workspace(state: &AppState, ws: crate::ui::dock::Workspace) -> Option<Command> {
    let id = format!("workspace: {}", ws.label());
    workspaces(state)
        .into_iter()
        .find(|command| command.id == id)
}

pub fn side(id: PanelId, label: &str, hover: &str) -> Command {
    let mut command = Command::new(
        format!("side: {id}"),
        label.to_string(),
        tr_key!("View"),
        Run::Side(id),
    );
    command.hover = Some(hover.to_string());
    command
}

/// How the id of a command that floats a panel starts, which View ▸ Panels leaves to the palette.
pub const FLOAT_PANEL: &str = "float panel:";

/// Each panel the workspace on screen can show, and floating each docked one.
pub fn panels(state: &AppState) -> Vec<Command> {
    // ⚠ Showing a panel is what makes a floating window's close button safe:
    // closing one takes the panel out of the arrangement, and without somewhere
    // to tick it back on that is data loss a restart does not fix.
    let dock = state.workspace_docks.peek(state.active_workspace);
    let last = dock.is_some_and(|dock| dock.main_surface().num_tabs() <= 1);
    let mut out = Vec::new();
    for (id, title) in dock_commands::offerable(state) {
        let placement = dock.map_or(Placement::Absent, |dock| dock_commands::placement(dock, id));
        let mut toggle = Command::new(
            format!("panel: {id}"),
            title.clone(),
            tr_key!("View ▸ Panels"),
            Run::TogglePanel(id),
        );
        toggle.checked = Some(placement != Placement::Absent);
        out.push(toggle);
        if placement == Placement::Docked {
            let mut float = Command::new(
                format!("{FLOAT_PANEL} {id}"),
                tr!("↗ {title} in its own window", title = title),
                tr_key!("View ▸ Panels"),
                Run::FloatPanel(id),
            );
            if last {
                float.enabled = Err(tr!("The last panel in the window cannot leave it"));
            }
            out.push(float);
        }
    }
    out
}

/// Each installed plugin, as a command that runs it on the open document.
pub fn plugins(state: &AppState) -> Vec<Command> {
    // ⚠ Lists only what `AppState::installed_plugins` has read, so callers call it first.
    let open = state.active_document.is_some();
    state
        .plugins
        .scanned
        .iter()
        .flatten()
        .map(|plugin| {
            let id = &plugin.manifest.id;
            let mut command = Command::new(
                format!("plugin: {id}"),
                plugin.manifest.name.clone(),
                tr_key!("Automate ▸ Run plugin"),
                Run::Plugin(id.clone()),
            );
            if !open {
                command.enabled = Err(tr!("A plugin runs on the open document"));
            }
            command
        })
        .collect()
}

/// Each command a loaded plugin registered, with the binding it was given.
pub fn plugin_commands(state: &AppState) -> Vec<Command> {
    state
        .plugin_commands()
        .into_iter()
        .map(|registered| {
            let mut command = Command::new(
                format!(
                    "plugin command: {plugin}: {id}",
                    plugin = registered.plugin,
                    id = registered.id
                ),
                registered.label,
                tr_key!("Automate ▸ Plugin commands"),
                Run::PluginCommand {
                    plugin: registered.plugin,
                    command: registered.id,
                },
            );
            command.binding = registered.binding;
            if registered.stopped.is_some() {
                command.enabled = Err(tr!(
                    "The plugin stopped; Automate ▸ Run plugin loads it again"
                ));
            }
            command
        })
        .collect()
}

pub fn run(state: &mut AppState, ctx: &egui::Context, command: &Command) {
    tracing::info!(command = %command.id, "Running a command");
    crate::ui::panels::command_log::note(state, command);
    match &command.run {
        &Run::Action(action) => {
            if let Some(spec) = spec(action) {
                (spec.run)(state, ctx);
            }
        }
        Run::On(action, target) => {
            if let Some(on) = spec(*action).and_then(|spec| spec.on.as_ref()) {
                (on.run)(state, ctx, target);
            }
        }
        Run::OpenRecentFile(recent) if recent.exists() => state.open_recent(recent),
        Run::OpenRecentFile(recent) => gone(state, tr!("File"), Path::new(&recent.source)),
        Run::OpenRecentFolder(path) if path.is_dir() => state.open_folder(path.clone()),
        Run::OpenRecentFolder(path) => gone(state, tr!("Folder"), path),
        Run::OpenRecentProject(path) => state.open_project_asking(path.clone()),
        Run::RemoveFromProject(path) => {
            state.remove_from_project(path);
        }
        &Run::Workspace(ws) => state.active_workspace = ws,
        &Run::PinWorkspace(ws) => state.toggle_pinned(ws),
        &Run::EditMode(mode) => state.set_edit_mode(mode),
        &Run::TogglePanel(id) => {
            let dock = state.workspace_docks.get_mut(state.active_workspace);
            if dock_commands::placement(dock, id) == Placement::Absent {
                dock_commands::show(dock, id);
            } else {
                dock_commands::hide(dock, id);
            }
        }
        &Run::Side(id) => {
            state.hide_start_page();
            let dock = state.workspace_docks.get_mut(state.active_workspace);
            dock_commands::reveal_in_the_side(dock, id);
        }
        &Run::FloatPanel(id) => {
            let bounds = ctx.content_rect();
            let dock = state.workspace_docks.get_mut(state.active_workspace);
            dock_commands::detach(dock, id, bounds);
        }
        Run::Plugin(id) => state.request_plugin_run(id),
        Run::PluginCommand { plugin, command } => state.run_plugin_command(plugin, command),
    }
}

/// Runs `action` as a key press does: only while it can run, saying why when it cannot (bar the clipboard's), and never while a question waits.
pub fn run_action(state: &mut AppState, ctx: &egui::Context, action: Action) {
    let Some(command) = of(state, action) else {
        return;
    };
    if crate::ui::panels::question_waits(ctx) {
        tracing::debug!(command = %command.id, "Not run: a question waits on the user");
        return;
    }
    match command.enabled {
        Ok(()) => run(state, ctx, &command),
        Err(why) => {
            tracing::debug!(command = %command.id, why, "Not run");
            // ⚠ egui copies a selected label's text on Ctrl+C itself, so a refused Cut, Copy or
            // Paste may follow a copy that worked: it says nothing.
            let egui_takes_the_key = matches!(action, Action::Cut | Action::Copy | Action::Paste);
            // ⚠ Once: a held key repeats, and the same sentence would stack.
            if !egui_takes_the_key && !state.toasts.iter().any(|toast| toast.message == why) {
                if why == crate::i18n::tr(NEEDS_A_PROJECT) {
                    state.show_info_toast_with_action(why, ToastAction::NewProject);
                } else {
                    state.show_info_toast(why);
                }
            }
        }
    }
}

/// `command` as a menu's button, its icon in a gutter every item keeps so that the labels line up.
///
/// ⚠ The icon is painted, not text of the button: a screen reader and a test name the item by its label.
pub fn menu_button(ui: &mut egui::Ui, command: &Command) -> egui::Response {
    let enabled = command.enabled.is_ok();
    let mut atoms = atoms(command);
    atoms.push_left(gutter());
    let response = ui.add_enabled(enabled, egui::Button::new(atoms));
    if let Some(glyph) = glyph(&command.id) {
        let left = ui.spacing().button_padding.x + GUTTER / 2.0;
        let colour = ui.visuals().weak_text_color();
        ui.painter().text(
            response.rect.left_center() + egui::vec2(left, 0.0),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(13.0),
            if enabled {
                colour
            } else {
                colour.gamma_multiply(0.5)
            },
        );
    }
    response
}

/// The width of the icon gutter of a menu's items.
const GUTTER: f32 = 14.0;

fn gutter() -> egui::Atom<'static> {
    egui::Atom::custom(egui::Id::new("command_glyph"), egui::Vec2::splat(GUTTER))
}

/// A menu inside a menu, its label behind the gutter its neighbours keep their icons in.
pub fn submenu<R>(
    ui: &mut egui::Ui,
    label: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    ui.menu_button(behind_gutter(label), add)
}

/// `label` behind the gutter a menu's items keep their icons in, for an item that is no command.
pub fn behind_gutter(label: &str) -> egui::Atoms<'_> {
    let mut atoms = egui::Atoms::new(label);
    atoms.push_left(gutter());
    atoms
}

/// The icon a command is drawn with, where it has one.
pub fn glyph(id: &str) -> Option<&'static str> {
    use egui_phosphor::regular as ph;
    Some(match id {
        "open" | "open_selection" | "open_asset" => ph::FILE,
        "open_folder" | "import_folder" => ph::FOLDER_OPEN,
        "open_project" | "open_project_here" | "open_project_in_new_window" => ph::FOLDER_SIMPLE,
        "create_project" | "new_folder" | "new_particles" | "new_weapon" => ph::PLUS,
        "new_project" => ph::X_CIRCLE,
        "open_built_pack" | "create_dlc_pack" => ph::PACKAGE,
        "open_sample_project" => ph::FLASK,
        "install_plugin_file" | "install_plugin_folder" => ph::PLUG,
        "check_against_game" | "check_before_sharing" | "check_mesh" => ph::SEAL_CHECK,
        "crash_reports" => ph::BUG,
        "batch" => ph::STACK,
        "show_grid" => ph::GRID_FOUR,
        "toggle_maximize_panel" => ph::ARROWS_OUT,
        "reopen_closed_tab" => ph::ARROW_U_UP_LEFT,
        "go_to_line" => ph::HASH,
        "save" | "save_project" => ph::FLOPPY_DISK,
        "save_as" | "save_project_as" | "save_as_template" => ph::FLOPPY_DISK_BACK,
        "close_document" | "close_other_tabs" | "close_project" => ph::X,
        "exit" => ph::SIGN_OUT,
        "undo" | "undo_clothes" | "undo_maps" | "undo_project" | "undo_weapons" | "undo_import" => {
            ph::ARROW_COUNTER_CLOCKWISE
        }
        "redo" | "redo_clothes" | "redo_maps" | "redo_project" | "redo_weapons" => {
            ph::ARROW_CLOCKWISE
        }
        "cut" => ph::SCISSORS,
        "copy" | "copy_name" | "copy_path" | "copy_hash" => ph::COPY,
        "paste" => ph::CLIPBOARD,
        "delete" | "remove_asset" | "remove_from_project" | "remove_build" => ph::TRASH,
        "rename" => ph::PENCIL_SIMPLE,
        "find_in_document" | "find_in_open_documents" | "go_to_file" => ph::MAGNIFYING_GLASS,
        "replace_in_document" => ph::SWAP,
        "extract" | "export_as" | "export_texture" | "export_garment" | "export_pack_package" => {
            ph::EXPORT
        }
        "export_gltf" => ph::CUBE,
        "import_gltf"
        | "import_file"
        | "import_game_files"
        | "import_into_project"
        | "import_pack_package" => ph::DOWNLOAD_SIMPLE,
        "convert" => ph::ARROWS_LEFT_RIGHT,
        "reveal_on_disk" | "reveal_asset" | "reveal_build" | "reveal_pack" | "reveal_project" => {
            ph::FOLDER_NOTCH_OPEN
        }
        "reveal_in_tree" => ph::TREE_VIEW,
        "replace_texture" | "replace_texture_from_image" | "add_texture" => ph::IMAGE,
        "replace_model" | "replace_geometry" => ph::CUBE_FOCUS,
        "build_all" | "build_project" => ph::HAMMER,
        "clean_project" => ph::BROOM,
        "open_settings" | "plugin_settings" | "project_properties" | "properties" => ph::GEAR,
        "keyboard_shortcuts" => ph::KEYBOARD,
        "command_palette" => ph::TERMINAL_WINDOW,
        "open_guide" | "open_acceptance_guide" => ph::BOOK_OPEN,
        "take_the_tour" => ph::ARROW_RIGHT,
        "about" => ph::INFO,
        "start_page" => ph::HOUSE,
        "log" | "command_log" => ph::LIST,
        "background_jobs" => ph::SPINNER,
        "statistics" => ph::CHART_BAR,
        "add_to_favourites" | "remove_from_favourites" => ph::STAR,
        "compare_with_original" | "compare_with_selected" | "select_for_compare" => ph::GIT_DIFF,
        "frame_all" | "frame_selection" => ph::CORNERS_OUT,
        "reset_layout" => ph::LAYOUT,
        "quick_look" => ph::EYE,
        _ => return None,
    })
}

/// The label and the binding, as a menu item and a palette row show them.
pub fn atoms(command: &Command) -> egui::Atoms<'_> {
    let mut atoms = egui::Atoms::new(command.label.as_str());
    if let Some(binding) = &command.binding {
        atoms.push_right(egui::Atom::grow());
        atoms.push_right(egui::RichText::new(binding).weak());
    }
    atoms
}

/// `response` with the command's hover text, and why it cannot run while it cannot.
pub fn hints(response: egui::Response, command: &Command) -> egui::Response {
    // ⚠ Not over a question: the tooltip of the button that opened it stays up above the dialog and reads as part of it.
    if crate::ui::panels::question_waits(&response.ctx) {
        return response;
    }
    let guide = crate::ui::guide_links::section_of(&command.id);
    let response = match (&command.hover, guide) {
        (Some(hover), Some(section)) => {
            response.on_hover_text(crate::ui::guide_links::hover(hover, section))
        }
        (Some(hover), None) => response.on_hover_text(hover),
        (None, _) => response,
    };
    if let Some(section) = guide {
        crate::ui::guide_links::ask_on_shift_f1(&response, section);
    }
    match command.enabled {
        Ok(()) => response,
        Err(why) => response.on_disabled_hover_text(why),
    }
}

fn gone(state: &mut AppState, what: &str, path: &Path) {
    let message = tr!(
        "{what} no longer exists: {path}",
        what = what,
        path = path.display()
    );
    tracing::warn!("{message}");
    state.show_error_toast(message);
}

fn display_recent(path: &Path) -> String {
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(tr!("(unknown)"));
    if !path.exists() {
        return format!("(missing) {file}");
    }
    let mtime = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(crate::state::prefs::ago)
        .unwrap_or_default();
    if mtime.is_empty() {
        file.to_string()
    } else {
        format!("{file}    {mtime}")
    }
}

fn display_recent_folder(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or(tr!("(unknown)")));
    if !path.is_dir() {
        return format!("(missing) {name}");
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use winit::keyboard::{Key, KeyCode, ModifiersState, PhysicalKey};

    use super::*;
    use crate::state::Prefs;
    use crate::ui::shortcuts::ShortcutMods;

    #[test]
    fn every_command_line_a_command_states_is_one_the_cli_takes() {
        let state = AppState::from_prefs(Prefs::default());
        let mut stated = 0;
        for spec in COMMANDS {
            let Some(line) = spec.cli else {
                continue;
            };
            let line = line(&state);
            let mut words = line.split_whitespace();
            assert_eq!(words.next(), Some("openrage-cli"), "{}: {line}", spec.id);
            // A placeholder stands for a value: one that names a model's exchange file ends as that file must.
            let args: Vec<String> = words
                .map(
                    |word| match word.strip_prefix('<').and_then(|w| w.strip_suffix('>')) {
                        Some(named) if named.contains(".glb") => "out.glb".to_string(),
                        Some(_) => "x".to_string(),
                        None => word.to_string(),
                    },
                )
                .collect();
            let command = args.first().map_or("", String::as_str);
            assert!(
                or_cli::COMMANDS.contains(&command),
                "{}: `{command}` is no command of openrage-cli",
                spec.id
            );
            // ⚠ The CLI's own reading of the line, not a list of flags kept here: a flag the CLI renames fails this.
            if let Err(why) = or_cli::accepts(&args) {
                panic!("{}: openrage-cli refuses `{line}`: {why}", spec.id);
            }
            stated += 1;
        }
        assert!(stated >= 6, "{stated} commands state a line");
    }

    #[test]
    fn every_command_has_one_row_one_id_and_a_name() {
        let mut actions = HashSet::new();
        let mut ids = HashSet::new();
        for spec in COMMANDS {
            assert!(
                actions.insert(spec.action),
                "{:?} has two rows",
                spec.action
            );
            assert!(
                ids.insert(spec.id),
                "two commands share the id `{}`",
                spec.id
            );
            assert!(
                !spec.label.is_empty() && !spec.category.is_empty(),
                "{:?} is offered with no name or no place",
                spec.action
            );
        }
    }

    #[test]
    fn a_key_bound_to_any_command_runs_it_through_the_registry() {
        let mut state = AppState::from_prefs(Prefs::default());
        let set = shortcuts::set_binding(
            &mut state.prefs,
            Action::ShowGrid,
            ShortcutMods::none(),
            egui::Key::G,
        );
        assert!(set.is_ok(), "{set:?}");
        let pressed = shortcuts::action_for_press(
            &Key::Character("g".into()),
            PhysicalKey::Code(KeyCode::KeyG),
            ModifiersState::empty(),
            false,
            &state.prefs,
        );
        assert_eq!(pressed, Some(Action::ShowGrid));

        let grid = state.prefs.viewport.show_grid;
        run_action(&mut state, &egui::Context::default(), Action::ShowGrid);
        assert_eq!(state.prefs.viewport.show_grid, !grid);
        assert_eq!(
            of(&state, Action::ShowGrid)
                .and_then(|c| c.binding)
                .as_deref(),
            Some("G"),
            "menus and the palette show another binding than the one that ran"
        );
    }
}
