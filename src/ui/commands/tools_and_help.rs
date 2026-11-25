//! The commands of Automate, Help and Notifications: baking the game shaders, the plugin and shortcut settings, the log, the crash reports, About and the toasts.

use super::table::{none_if, Action, Spec, BASE};
use crate::ui::panels;

pub(super) const BAKE_SHADERS: Spec = Spec {
    action: Action::BakeShaders,
    id: "bake_shaders",
    label: tr_key!("Bake game shaders"),
    category: tr_key!("Automate"),
    was: tr_key!("Tools"),
    hover: tr_key!(
        "Translate your install's shaders so the viewport draws models as the \
            game does. Runs in the background, once; later launches reuse the result."
    ),
    enabled: |state| none_if(state.game_pack.is_baking(), tr!("A bake is under way")),
    label_now: Some(|state| {
        use crate::state::GamePackStatus;
        if state.game_pack.is_baking() {
            tr!(
                "{short_label} (View ▸ Background jobs cancels it)",
                short_label = state.game_pack.status().short_label()
            )
        } else if matches!(state.game_pack.status(), GamePackStatus::Ready { .. }) {
            tr!("Re-bake game shaders").to_string()
        } else {
            tr!("Bake game shaders").to_string()
        }
    }),
    run: |state, _| state.force_rebake_game_pack(),
    ..BASE
};

pub(super) const HARVEST_GTA6_NAMES: Spec = Spec {
    action: Action::HarvestGta6Names,
    id: "harvest_gta6_names",
    label: tr_key!("Harvest GTA VI names"),
    category: tr_key!("Automate"),
    hover: tr_key!(
        "Read every archive of the GTA VI folder open in a tab for the names its bodies spell, \
            in the background. They are kept, and hashes, texture keys and scanned drawables are \
            shown by name from then on."
    ),
    enabled: |_| {
        let harvesting = crate::state::gta6_names::harvesting();
        none_if(harvesting, tr!("A harvest is under way"))
    },
    run: |state, _| state.harvest_names_of_the_open_gta6_folder(),
    ..BASE
};

pub(super) const COMMAND_LOG: Spec = Spec {
    action: Action::CommandLog,
    id: "command_log",
    label: tr_key!("Command log"),
    category: tr_key!("Automate"),
    hover: tr_key!(
        "Every command run in this window, to run again or to copy as the openrage-cli line that does the same"
    ),
    run: |state, _| panels::command_log::reveal(state),
    ..BASE
};

pub(super) const BATCH: Spec = Spec {
    action: Action::Batch,
    id: "batch",
    label: tr_key!("Batch…"),
    category: tr_key!("Automate"),
    hover: tr_key!("Run one command over every file selected in Files, as one job"),
    run: |state, _| panels::batch::open(state),
    ..BASE
};

pub(super) const PLUGIN_SETTINGS: Spec = Spec {
    action: Action::PluginSettings,
    id: "plugin_settings",
    label: tr_key!("Plugin settings…"),
    hover: tr_key!("What each plugin asked for, what you answered, and why it did what it did"),
    category: tr_key!("Automate ▸ Run plugin"),
    was: tr_key!("Tools"),
    run: |state, _| {
        let dock = state.workspace_docks.get_mut(state.active_workspace);
        crate::ui::dock_commands::reveal_in_the_side(dock, panels::registry::ids::PLUGINS);
    },
    ..BASE
};

/// Copy the plugin at `source` into the plugins directory and say what came of it.
fn install_plugin(state: &mut crate::state::AppState, source: Option<std::path::PathBuf>) {
    let Some(source) = source else {
        return;
    };
    match state.install_plugin(&source) {
        Ok(name) => state.show_toast(tr!("Installed {name}", name = name)),
        Err(why) => state.show_error_toast(why),
    }
}

pub(super) const INSTALL_PLUGIN_FOLDER: Spec = Spec {
    action: Action::InstallPluginFolder,
    id: "install_plugin_folder",
    label: tr_key!("Install plugin from folder…"),
    category: tr_key!("Automate ▸ Run plugin"),
    hover: tr_key!("Copy a folder that holds plugin.json and plugin.wasm into the plugins directory. Nothing runs until you say yes to what it asks for"),
    run: |state, _| {
        let picked =
            crate::platform::file_dialogs::pick_folder(tr!("Install plugin from a folder"));
        install_plugin(state, picked);
    },
    ..BASE
};

pub(super) const INSTALL_PLUGIN_FILE: Spec = Spec {
    action: Action::InstallPluginFile,
    id: "install_plugin_file",
    label: tr_key!("Install plugin from file…"),
    category: tr_key!("Automate ▸ Run plugin"),
    hover: tr_key!("Pick the plugin.json or the plugin.wasm of a plugin; its folder is copied into the plugins directory"),
    run: |state, _| {
        install_plugin(state, crate::platform::file_dialogs::pick_plugin_file());
    },
    ..BASE
};

pub(super) const OPEN_GUIDE: Spec = Spec {
    action: Action::OpenGuide,
    id: "open_guide",
    label: tr_key!("Guide"),
    category: tr_key!("Help"),
    hover: tr_key!("How to open a pack, fix it, build it, put it on a server and share a project, for someone who comes from Durty Cloth Tool"),
    run: |state, _| state.front_door.show_guide(""),
    ..BASE
};

pub(super) const OPEN_ACCEPTANCE_GUIDE: Spec = Spec {
    action: Action::OpenAcceptanceGuide,
    id: "open_acceptance_guide",
    label: tr_key!("In-game acceptance guide"),
    category: tr_key!("Help"),
    hover: tr_key!("What to look for in a running game to say that it loads what a build wrote, how to load a build on a FiveM server and in story mode, and how to write down what you saw"),
    run: |state, _| {
        let document = crate::state::front_door::GuideDocument::Acceptance;
        state.front_door.show_document(document, "");
    },
    ..BASE
};

pub(super) const OPEN_SAMPLE_PROJECT: Spec = Spec {
    action: Action::OpenSampleProject,
    id: "open_sample_project",
    label: tr_key!("Open the sample project"),
    category: tr_key!("Help"),
    hover: tr_key!("A small clothing project made of synthetic content, with one problem to find. It is kept beside the preferences and is yours to change"),
    run: |state, _| state.open_sample_project(),
    ..BASE
};

pub(super) const TAKE_TOUR: Spec = Spec {
    action: Action::TakeTour,
    id: "take_the_tour",
    label: tr_key!("Take the tour"),
    category: tr_key!("Help"),
    hover: tr_key!("Six steps over the screens of a project that end on the guide. With no project open they take the sample project"),
    run: |state, _| state.start_tour(),
    ..BASE
};

pub(super) const KEYBOARD_SHORTCUTS: Spec = Spec {
    action: Action::KeyboardShortcuts,
    id: "keyboard_shortcuts",
    label: tr_key!("Keyboard shortcuts…"),
    category: tr_key!("Help"),
    run: |state, _| {
        panels::settings::open_settings_at(state, panels::settings::SettingsCategory::Shortcuts);
    },
    ..BASE
};

pub(super) const LOG: Spec = Spec {
    action: Action::Log,
    id: "log",
    label: tr_key!("Log"),
    category: tr_key!("Help"),
    hover: tr_key!("Everything OpenRage has reported this session, to read and copy"),
    run: |state, _| panels::log::reveal(state),
    ..BASE
};

/// The door of a panel no layout docks.
pub(super) const STATISTICS: Spec = Spec {
    action: Action::Statistics,
    id: "statistics",
    label: tr_key!("Statistics"),
    category: tr_key!("Help"),
    hover: tr_key!("The frame time and the memory OpenRage holds, against their budgets"),
    run: |state, _| {
        let on_screen = state.active_workspace;
        (state.workspace_docks).reveal(on_screen, panels::registry::ids::STATISTICS);
    },
    ..BASE
};

pub(super) const CRASH_REPORTS: Spec = Spec {
    action: Action::CrashReports,
    id: "crash_reports",
    label: tr_key!("Open crash reports folder"),
    category: tr_key!("Help"),
    hover: tr_key!("Where OpenRage writes a report when it crashes"),
    enabled: |_| {
        let known = crate::diagnostics::crash::reports_dir().is_some();
        none_if(
            !known,
            tr!("This machine gives OpenRage no folder for them"),
        )
    },
    run: |_, _| {
        if let Some(dir) = crate::diagnostics::crash::reports_dir() {
            crate::platform::shell::open_folder(&dir);
        }
    },
    ..BASE
};

pub(super) const ABOUT: Spec = Spec {
    action: Action::About,
    id: "about",
    label: tr_key!("About"),
    category: tr_key!("Help"),
    run: |state, _| panels::about::open_about(state),
    ..BASE
};

pub(super) const CLEAR_TOASTS: Spec = Spec {
    action: Action::ClearToasts,
    id: "clear_toasts",
    label: tr_key!("Clear all toasts"),
    category: tr_key!("Notifications"),
    run: |state, _| state.clear_toasts(),
    ..BASE
};
