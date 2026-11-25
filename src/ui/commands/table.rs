//! The commands the app defines, one row each.

use egui::Key;

use super::predicates::{a_text_document, a_viewport, always, idle};
use super::target::Target;
use super::text_find::{find_in_text, text_editor_for};
use crate::platform::file_dialogs;
use crate::state::AppState;
use crate::ui::panels;
use crate::ui::panels::registry::ids;
use crate::ui::shortcuts::ShortcutMods;

// The checks live in `predicates`; the files of commands beside the table take them from here.
pub(super) use super::predicates::{a_document_that_is_written, a_project, none_if, not_busy};

/// A command of [`COMMANDS`], as code names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    NewProject,
    CreateProject,
    StartPage,
    OpenProjectInNewWindow,
    OpenBuiltPack,
    MakeFolderAProject,
    Open,
    ClearRecentFiles,
    OpenFolder,
    ClearRecentFolders,
    Save,
    SaveAs,
    SaveIntoArchiveCopy,
    CloseDocument,
    PreviousTab,
    ReopenClosedTab,
    OpenProject,
    SaveProject,
    SaveProjectAs,
    ClearRecentProjects,
    RemoveFromProject,
    CloseProject,
    OpenSettings,
    Exit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    CommandPalette,
    GoToFile,
    ResetLayout,
    ShowGrid,
    ShowAxis,
    ToggleMaximizePanel,
    FrameAll,
    FrameSelection,
    FocusHierarchyFilter,
    BackgroundJobs,
    BakeShaders,
    HarvestGta6Names,
    CommandLog,
    Batch,
    PluginSettings,
    InstallPluginFolder,
    InstallPluginFile,
    KeyboardShortcuts,
    OpenGuide,
    OpenSampleProject,
    TakeTour,
    Log,
    Statistics,
    CrashReports,
    About,
    ClearToasts,
    TextFind,
    TextReplace,
    TextGoToLine,
    TextFindInOpenDocs,
    OpenSelection,
    RevealInTree,
    RevealOnDisk,
    Extract,
    ExportAs,
    Convert,
    ExportGltf,
    ImportGltf,
    Rename,
    NewFolder,
    ImportFile,
    ImportIntoProject,
    CopyPath,
    CopyName,
    CopyHash,
    Properties,
    AddToFavourites,
    RemoveFromFavourites,
    CreateDlcPack,
    CloseOtherTabs,
    SelectForCompare,
    CompareWithSelected,
    CompareWithOriginal,
    AddContent,
    ImportGameFiles,
    ImportFolder,
    UndoImport,
    UndoProject,
    RedoProject,
    OpenProjectHere,
    AddImagesAsTexturesSource,
    NewTexturesSource,
    AddBuild,
    EditBuild,
    AddClothing,
    AddClothes,
    AddClothesFolder,
    FixAllSafe,
    OptimiseTextures,
    ReplaceTextureFromImage,
    ExportTexture,
    MakeMaps,
    BudgetTextures,
    CompleteMaterials,
    RepairModel,
    CheckMesh,
    AddLevels,
    RepairRig,
    CleanWeights,
    MirrorModel,
    CropUvs,
    UndoClothes,
    RedoClothes,
    AddWeaponPack,
    AddWeapons,
    NewWeapon,
    RemoveWeapon,
    UndoWeapons,
    RedoWeapons,
    AddMapPack,
    AddMaps,
    AddEntity,
    RemoveEntity,
    DropToGround,
    AddArchetype,
    RemoveArchetype,
    UndoMaps,
    RedoMaps,
    AddParticlePack,
    AddParticles,
    NewParticles,
    RestoreLastGood,
    RevealPack,
    LinkCloth,
    UnlinkCloth,
    ReplaceModel,
    UseModelSource,
    ReplaceTexture,
    AddTexture,
    DuplicateToOtherPed,
    MoveGarmentEarlier,
    MoveGarmentLater,
    ExportGarment,
    PackageGarment,
    AddPackage,
    ReviewVersions,
    CreditGarment,
    ExportPackPackage,
    ImportPackPackage,
    BuildProject,
    BuildAll,
    CleanProject,
    ProjectProperties,
    RevealProject,
    RevealBuild,
    OpenAsset,
    RevealAsset,
    RecookAsset,
    ReplaceGeometry,
    CheckAgainstGame,
    RemoveAsset,
    RemoveBuild,
    QuickLook,
    LinkContent,
    UnlinkContent,
    SaveAsTemplate,
    ApplyTemplate,
    CheckBeforeSharing,
    Workspace1,
    Workspace2,
    Workspace3,
    Workspace4,
    Workspace5,
    Workspace6,
    Workspace7,
    Workspace8,
    Workspace9,
}

/// What a command that acts on something does: a context menu names what was
/// clicked, and the menu bar, the palette and its keys act on what `focused` finds.
pub struct On {
    pub focused: fn(&AppState) -> Result<Target, &'static str>,
    /// `Err` says why it cannot run on that.
    pub enabled: fn(&AppState, &Target) -> Result<(), &'static str>,
    /// What the label reads for that, where it says more than the label.
    pub label: Option<fn(&AppState, &Target) -> String>,
    pub run: fn(&mut AppState, &egui::Context, &Target),
}

/// One command: what it is called and where, its default keys, when it can run and what it does.
pub struct Spec {
    pub action: Action,
    /// ⚠ Never renamed: `prefs.json` keeps the user's binding under it.
    pub id: &'static str,
    pub label: &'static str,
    /// Its menu path, or its group where no menu offers it.
    pub category: &'static str,
    /// Menus show each one; a command with none is unbound until the user binds it.
    pub defaults: &'static [(ShortcutMods, Key)],
    pub hover: &'static str,
    /// `Err` says why it cannot run now.
    pub enabled: fn(&AppState) -> Result<(), &'static str>,
    /// The words it was found by before it was renamed, which the palette still answers to.
    pub was: &'static str,
    /// The `openrage-cli` line that does the same, which the command log offers to copy: the arguments the window knows, and a `<name>` for each it does not.
    pub cli: Option<fn(&AppState) -> String>,
    /// What the label reads now, where that says more than `label`.
    pub label_now: Option<fn(&AppState) -> String>,
    /// A toggle's state.
    pub checked: Option<fn(&AppState) -> bool>,
    /// ⚠ Its keys are read by their panel, under the pointer or with focus, never by the key dispatcher.
    pub panel_key: bool,
    pub run: fn(&mut AppState, &egui::Context),
    /// In place of `enabled` and `run`, for a command that acts on what was clicked.
    pub on: Option<On>,
}

// What a row leaves out; each row still names its own action, id, label, category and run.
pub(super) const BASE: Spec = Spec {
    action: Action::Exit,
    id: "",
    label: "",
    category: "",
    defaults: &[],
    hover: "",
    enabled: always,
    was: "",
    cli: None,
    label_now: None,
    checked: None,
    panel_key: false,
    run: |_, _| {},
    on: None,
};

const CTRL: ShortcutMods = ShortcutMods::ctrl();
const CTRL_SHIFT: ShortcutMods = ShortcutMods::ctrl_shift();
const NONE: ShortcutMods = ShortcutMods::none();

pub const COMMANDS: &[Spec] = &[
    Spec {
        action: Action::NewProject,
        id: "new_project",
        label: tr_key!("Close all"),
        category: tr_key!("File"),
        was: tr_key!("New Session"),
        hover: tr_key!("Close every open file but the install, and leave the project"),
        run: |state, _| state.new_project(),
        ..BASE
    },
    Spec {
        action: Action::Open,
        id: "open",
        label: tr_key!("Open file…"),
        category: tr_key!("File"),
        defaults: &[(CTRL, Key::O)],
        hover: tr_key!("Open one or more files: an archive, a model, a texture dictionary, \
                metadata or any other game file"),
        enabled: idle,
        run: open_files,
        ..BASE
    },
    Spec {
        action: Action::ClearRecentFiles,
        id: "clear_recent_files",
        label: tr_key!("Clear recent files"),
        category: tr_key!("File ▸ Open recent"),
        enabled: |state| none_if(state.prefs.recent_files.is_empty(), tr!("No recent files")),
        run: |state, _| {
            state.prefs.clear_recent_files();
            crate::state::save_prefs(&state.prefs);
        },
        ..BASE
    },
    Spec {
        action: Action::OpenFolder,
        id: "open_folder",
        label: tr_key!("Open folder…"),
        category: tr_key!("File"),
        hover: tr_key!("Its files and archives open as a tree, as an install's do"),
        enabled: idle,
        run: |state, _| {
            if let Some(folder) = file_dialogs::pick_open_folder() {
                state.open_folder(folder);
            }
        },
        ..BASE
    },
    Spec {
        action: Action::ClearRecentFolders,
        id: "clear_recent_folders",
        label: tr_key!("Clear recent folders"),
        category: tr_key!("File ▸ Open recent"),
        enabled: |state| none_if(state.prefs.recent_folders.is_empty(), tr!("No recent folders")),
        run: |state, _| {
            state.prefs.clear_recent_folders();
            crate::state::save_prefs(&state.prefs);
        },
        ..BASE
    },
    Spec {
        action: Action::Save,
        id: "save",
        label: tr_key!("Save"),
        category: tr_key!("File"),
        defaults: &[(CTRL, Key::S)],
        hover: tr_key!("Save the active document; one never saved asks where. With no document open, it saves the project"),
        enabled: super::project::something_to_save,
        run: super::project::save,
        ..BASE
    },
    Spec {
        action: Action::SaveAs,
        id: "save_as",
        label: tr_key!("Save as…"),
        category: tr_key!("File"),
        defaults: &[(CTRL_SHIFT, Key::S)],
        hover: tr_key!("Save the active document to a new location"),
        enabled: a_document_that_is_written,
        run: |state, _| {
            if let Some(path) = panels::menu::save_dialog_for_active(state) {
                state.save_active_document_as(path);
            }
        },
        ..BASE
    },
    Spec {
        action: Action::SaveIntoArchiveCopy,
        id: "save_into_archive_copy",
        label: tr_key!("Save into a copy of the archive…"),
        category: tr_key!("File"),
        hover: tr_key!("Write the GTA VI document's edits in place of its entry, in a new copy of the archive it was read from. The game refuses that copy, because its table's signature is stale"),
        enabled: |state| state.can_save_into_archive_copy(),
        run: |state, _| {
            let name = state.archive_copy_name();
            let kind = crate::state::DocumentKind::Rpf9;
            if let Some(path) = file_dialogs::save_any_file(kind, name.as_deref()) {
                state.save_into_archive_copy(path);
            }
        },
        ..BASE
    },
    Spec {
        action: Action::CloseDocument,
        id: "close_document",
        label: tr_key!("Close document"),
        category: tr_key!("File"),
        defaults: &[(CTRL, Key::W)],
        hover: tr_key!("Close the active document"),
        on: Some(super::files::CLOSE_DOCUMENT),
        ..BASE
    },
    Spec {
        action: Action::PreviousTab,
        id: "previous_tab",
        label: tr_key!("Switch to previous tab"),
        category: tr_key!("Tabs"),
        // ⚠ The Control key itself on a Mac: Ctrl+Tab is how tabs switch there, and Cmd+Tab is the system's.
        defaults: &[(ShortcutMods::physical_control(), Key::Tab)],
        hover: tr_key!("The tab used before this one; Tab again with Ctrl held goes further back"),
        enabled: |state| none_if(state.tab_order().len() < 2, tr!("Fewer than two tabs are open")),
        run: |state, _| state.cycle_tabs(),
        ..BASE
    },
    Spec {
        action: Action::ReopenClosedTab,
        id: "reopen_closed_tab",
        label: tr_key!("Reopen closed tab"),
        category: tr_key!("Tabs"),
        defaults: &[(CTRL_SHIFT, Key::T)],
        hover: tr_key!("Open the tab closed last again"),
        enabled: |state| none_if(!state.tabs.can_reopen(), tr!("No tab has been closed")),
        run: |state, _| state.reopen_closed_tab(),
        ..BASE
    },
    super::files::CLOSE_OTHERS,
    super::front_door::CREATE_PROJECT,
    super::front_door::START_PAGE,
    super::front_door::OPEN_PROJECT_IN_NEW_WINDOW,
    super::front_door::OPEN_BUILT_PACK,
    super::front_door::MAKE_FOLDER_A_PROJECT,
    super::project::ADD_CONTENT,
    super::project::IMPORT_GAME_FILES,
    super::project::IMPORT_FOLDER,
    super::project::ADD_IMAGES,
    super::project::NEW_TEXTURES_SOURCE,
    super::project::OPEN_PROJECT_HERE,
    super::project::ADD_BUILD,
    super::project::EDIT_BUILD,
    super::wardrobe::ADD_CLOTHING,
    super::wardrobe::ADD_CLOTHES,
    super::wardrobe::ADD_CLOTHES_FOLDER,
    super::weapons::ADD_WEAPON_PACK,
    super::weapons::ADD_WEAPONS,
    super::weapons::NEW_WEAPON,
    super::weapons::REMOVE_WEAPON,
    super::weapons::UNDO_WEAPONS,
    super::weapons::REDO_WEAPONS,
    super::maps::ADD_MAP_PACK,
    super::maps::ADD_MAPS,
    super::maps::ADD_ENTITY,
    super::maps::REMOVE_ENTITY,
    super::maps::DROP_TO_GROUND,
    super::maps::ADD_ARCHETYPE,
    super::maps::REMOVE_ARCHETYPE,
    super::maps::UNDO_MAPS,
    super::maps::REDO_MAPS,
    super::particles::ADD_PARTICLE_PACK,
    super::particles::ADD_PARTICLES,
    super::particles::NEW_PARTICLES,
    super::wardrobe::FIX_ALL_SAFE,
    super::wardrobe::OPTIMISE_TEXTURES,
    super::textures::REPLACE_TEXTURE_FROM_IMAGE,
    super::textures::EXPORT_TEXTURE,
    super::textures::MAKE_MAPS,
    super::textures::BUDGET_TEXTURES,
    super::textures::COMPLETE_MATERIALS,
    super::wardrobe::REPAIR_MODEL,
    super::mesh::CHECK_MESH,
    super::mesh::ADD_LEVELS,
    super::mesh::REPAIR_RIG,
    super::mesh::CLEAN_WEIGHTS,
    super::mesh::MIRROR_MODEL,
    super::mesh::CROP_UVS,
    super::wardrobe::LINK_CLOTH,
    super::wardrobe::UNLINK_CLOTH,
    super::wardrobe::REPLACE_MODEL,
    super::wardrobe::USE_MODEL_SOURCE,
    super::wardrobe::REPLACE_TEXTURE,
    super::wardrobe::ADD_TEXTURE,
    super::wardrobe::DUPLICATE_TO_OTHER_PED,
    super::wardrobe::MOVE_GARMENT_EARLIER,
    super::wardrobe::MOVE_GARMENT_LATER,
    super::wardrobe::EXPORT_GARMENT,
    super::wardrobe::PACKAGE_GARMENT,
    super::wardrobe::ADD_PACKAGE,
    super::wardrobe::REVIEW_VERSIONS,
    super::wardrobe::CREDIT_GARMENT,
    super::wardrobe::EXPORT_PACK_PACKAGE,
    super::wardrobe::IMPORT_PACK_PACKAGE,
    super::project::BUILD,
    super::project::BUILD_ALL,
    super::project::CLEAN,
    super::project::PROPERTIES,
    super::project::REVEAL,
    super::project::REVEAL_BUILD,
    super::project::OPEN_ASSET,
    super::project::REVEAL_ASSET,
    super::project::RECOOK_ASSET,
    super::project::REPLACE_GEOMETRY,
    super::project::CHECK_AGAINST_GAME,
    super::project::REMOVE_ASSET,
    super::project::REMOVE_BUILD,
    super::project::QUICK_LOOK,
    super::project::LINK_CONTENT,
    super::project::UNLINK_CONTENT,
    super::project::SAVE_AS_TEMPLATE,
    super::project::APPLY_TEMPLATE,
    super::project::CHECK_BEFORE_SHARING,
    super::project::WORKSPACE_1,
    super::project::WORKSPACE_2,
    super::project::WORKSPACE_3,
    super::project::WORKSPACE_4,
    super::project::WORKSPACE_5,
    super::project::WORKSPACE_6,
    super::project::WORKSPACE_7,
    super::project::WORKSPACE_8,
    super::project::WORKSPACE_9,
    Spec {
        action: Action::OpenProject,
        id: "open_project",
        label: tr_key!("Open project…"),
        category: tr_key!("File"),
        run: |state, _| {
            if let Some(path) = file_dialogs::open_project_file() {
                state.open_project_asking(path);
            }
        },
        ..BASE
    },
    Spec {
        action: Action::SaveProject,
        id: "save_project",
        label: tr_key!("Save project"),
        category: tr_key!("Project"),
        enabled: not_busy,
        run: |state, _| {
            state.save_active_project(super::project::save_project_dialog);
        },
        ..BASE
    },
    Spec {
        action: Action::SaveProjectAs,
        id: "save_project_as",
        label: tr_key!("Save project as…"),
        category: tr_key!("Project"),
        enabled: not_busy,
        run: super::project::save_project_as,
        ..BASE
    },
    Spec {
        action: Action::ClearRecentProjects,
        id: "clear_recent_projects",
        label: tr_key!("Clear recent projects"),
        category: tr_key!("File ▸ Open recent"),
        enabled: |state| none_if(state.prefs.recent_projects.is_empty(), tr!("No recent projects")),
        run: |state, _| {
            state.prefs.clear_recent_projects();
            crate::state::save_prefs(&state.prefs);
        },
        ..BASE
    },
    Spec {
        action: Action::RemoveFromProject,
        id: "remove_from_project",
        label: tr_key!("Remove active document from project"),
        category: tr_key!("Project ▸ Properties and sharing"),
        hover: tr_key!("The file stays on disk"),
        enabled: |state| {
            a_project(state)?;
            match state.active_document() {
                None => Err(tr!("No document is open")),
                Some(doc) => none_if(doc.path.is_none(), tr!("The active document has no file yet")),
            }
        },
        run: |state, _| {
            if let Some(path) = state.active_document().and_then(|d| d.path.clone()) {
                state.remove_from_project(&path);
            }
        },
        ..BASE
    },
    Spec {
        action: Action::CloseProject,
        id: "close_project",
        label: tr_key!("Close project"),
        category: tr_key!("Project"),
        enabled: a_project,
        run: |state, _| state.close_project(),
        ..BASE
    },
    Spec {
        action: Action::OpenSettings,
        id: "open_settings",
        label: tr_key!("Settings…"),
        category: tr_key!("File"),
        defaults: &[(CTRL, Key::Comma)],
        hover: tr_key!("Games, editing, appearance, keys, shortcuts and plugins"),
        run: |state, _| panels::settings::open_settings(state),
        ..BASE
    },
    Spec {
        action: Action::Exit,
        id: "exit",
        label: tr_key!("Exit"),
        category: tr_key!("File"),
        hover: tr_key!("Close OpenRage, asking first about unsaved work"),
        run: |state, _| state.request_exit(),
        ..BASE
    },
    super::undo::UNDO,
    super::undo::REDO,
    // ⚠ After Edit's Undo: the palette breaks a tie in the table's order, and `undo` must find Edit's first.
    super::project::UNDO_IMPORT,
    super::wardrobe::UNDO_CLOTHES,
    super::wardrobe::REDO_CLOTHES,
    super::wardrobe::RESTORE_LAST_GOOD,
    super::wardrobe::REVEAL_PACK,
    super::project::UNDO_PROJECT,
    super::project::REDO_PROJECT,
    Spec {
        action: Action::Cut,
        id: "cut",
        label: tr_key!("Cut"),
        category: tr_key!("Edit"),
        defaults: &[(CTRL, Key::X)],
        hover: tr_key!("Copy the selected files, which leave once they are pasted elsewhere"),
        on: Some(super::clipboard::CUT),
        ..BASE
    },
    Spec {
        action: Action::Copy,
        id: "copy",
        label: tr_key!("Copy"),
        category: tr_key!("Edit"),
        defaults: &[(CTRL, Key::C)],
        hover: tr_key!("Copy the selected files, to paste into an archive, a folder or Explorer, \
                or the records ticked in a metadata file's form"),
        on: Some(super::clipboard::COPY),
        ..BASE
    },
    Spec {
        action: Action::Paste,
        id: "paste",
        label: tr_key!("Paste"),
        category: tr_key!("Edit"),
        defaults: &[(CTRL, Key::V)],
        hover: tr_key!("Paste the files copied here or in Explorer into the folder selected, \
                or copied records after the last one ticked"),
        on: Some(super::clipboard::PASTE),
        ..BASE
    },
    Spec {
        action: Action::Delete,
        id: "delete",
        label: tr_key!("Delete…"),
        category: tr_key!("Edit"),
        defaults: &[(NONE, Key::Delete)],
        hover: tr_key!("Delete the selected file or folder once you say yes, the records ticked \
                in a metadata file's form, the effect or rule selected in a particle \
                file's outline, or the garments picked in the Wardrobe (their files stay)"),
        on: Some(super::clipboard::DELETE),
        ..BASE
    },
    super::files::OPEN,
    super::files::REVEAL_IN_TREE,
    super::files::REVEAL_ON_DISK,
    super::files::EXTRACT,
    super::files::EXPORT_AS,
    super::files::CONVERT,
    super::files::EXPORT_GLTF,
    super::files::IMPORT_GLTF,
    super::files::RENAME,
    super::files::NEW_FOLDER,
    super::files::IMPORT_FILE,
    super::files::IMPORT_INTO_PROJECT,
    super::files::COPY_PATH,
    super::files::COPY_NAME,
    super::files::COPY_HASH,
    super::files::PROPERTIES,
    super::files::ADD_FAVOURITE,
    super::files::REMOVE_FAVOURITE,
    super::files::CREATE_DLC_PACK,
    super::compare::SELECT_FOR_COMPARE,
    super::compare::COMPARE_WITH_SELECTED,
    super::compare::COMPARE_WITH_ORIGINAL,
    Spec {
        action: Action::CommandPalette,
        id: "command_palette",
        label: tr_key!("Command palette…"),
        category: tr_key!("Edit"),
        defaults: &[(CTRL_SHIFT, Key::P)],
        hover: tr_key!("Find any command by name and run it"),
        run: |state, _| panels::command_palette::open(state),
        ..BASE
    },
    Spec {
        action: Action::GoToFile,
        id: "go_to_file",
        label: tr_key!("Go to file…"),
        category: tr_key!("File"),
        defaults: &[(CTRL, Key::P)],
        hover: tr_key!("Find any file of the install or of an open folder by name or by hash, \
                from the field of the Files panel"),
        run: |state, _| panels::hierarchy::go_to_file(state),
        ..BASE
    },
    Spec {
        action: Action::ResetLayout,
        id: "reset_layout",
        label: tr_key!("Reset active workspace layout"),
        category: tr_key!("View"),
        hover: tr_key!("Put the panels of this workspace back where they start; no file or project is touched"),
        run: |state, _| {
            let ws = state.active_workspace;
            state.workspace_docks.reset(ws);
        },
        ..BASE
    },
    Spec {
        action: Action::ShowGrid,
        id: "show_grid",
        label: tr_key!("Show grid"),
        category: tr_key!("View"),
        checked: Some(|state| state.prefs.viewport.show_grid),
        run: |state, _| {
            state.prefs.viewport.show_grid = !state.prefs.viewport.show_grid;
            state.persist_viewport_prefs();
        },
        ..BASE
    },
    Spec {
        action: Action::ShowAxis,
        id: "show_axis",
        label: tr_key!("Show axis"),
        category: tr_key!("View"),
        checked: Some(|state| state.prefs.viewport.show_axis),
        run: |state, _| {
            state.prefs.viewport.show_axis = !state.prefs.viewport.show_axis;
            state.persist_viewport_prefs();
        },
        ..BASE
    },
    Spec {
        action: Action::ToggleMaximizePanel,
        id: "toggle_maximize_panel",
        label: tr_key!("Maximize panel"),
        category: tr_key!("View"),
        hover: tr_key!("Show the focused panel alone, or the arrangement again"),
        run: |state, _| {
            let ws = state.active_workspace;
            if !state.workspace_docks.toggle_maximized(ws) {
                state.show_toast(tr!("No panel to maximize: click one first"));
            }
        },
        ..BASE
    },
    Spec {
        action: Action::FrameAll,
        id: "frame_all",
        label: tr_key!("Frame all"),
        category: tr_key!("View"),
        defaults: &[(NONE, Key::Home)],
        hover: tr_key!("Frame everything in the viewport"),
        enabled: a_viewport,
        panel_key: true,
        run: |state, _| panels::viewport::request_framing(state, Action::FrameAll),
        ..BASE
    },
    Spec {
        action: Action::FrameSelection,
        id: "frame_selection",
        label: tr_key!("Frame selection"),
        category: tr_key!("View"),
        defaults: &[(NONE, Key::F), (NONE, Key::Period)],
        hover: tr_key!("Frame the selected model or geometry in the viewport"),
        enabled: a_viewport,
        panel_key: true,
        run: |state, _| panels::viewport::request_framing(state, Action::FrameSelection),
        ..BASE
    },
    Spec {
        action: Action::FocusHierarchyFilter,
        id: "focus_hierarchy_filter",
        label: tr_key!("Filter files"),
        category: tr_key!("View"),
        defaults: &[(NONE, Key::Slash)],
        hover: tr_key!("Type into the Files panel's filter"),
        enabled: |state| {
            let shown = state
                .workspace_docks
                .shows(state.active_workspace, ids::FILES);
            none_if(!shown, tr!("This workspace shows no Files panel"))
        },
        run: |state, _| panels::hierarchy::ui_state(state).focus_filter_next_frame = true,
        ..BASE
    },
    Spec {
        action: Action::BackgroundJobs,
        id: "background_jobs",
        label: tr_key!("Background jobs"),
        category: tr_key!("View"),
        hover: tr_key!("Every job running in the background and the ones just finished, with \
                their progress, a Cancel and what each left"),
        run: |state, _| panels::jobs::open(state),
        ..BASE
    },
    super::tools_and_help::BAKE_SHADERS,
    super::tools_and_help::HARVEST_GTA6_NAMES,
    super::tools_and_help::COMMAND_LOG,
    super::tools_and_help::BATCH,
    super::tools_and_help::PLUGIN_SETTINGS,
    super::tools_and_help::INSTALL_PLUGIN_FOLDER,
    super::tools_and_help::INSTALL_PLUGIN_FILE,
    super::tools_and_help::OPEN_GUIDE,
    super::tools_and_help::OPEN_SAMPLE_PROJECT,
    super::tools_and_help::TAKE_TOUR,
    super::tools_and_help::KEYBOARD_SHORTCUTS,
    super::tools_and_help::LOG,
    super::tools_and_help::STATISTICS,
    super::tools_and_help::CRASH_REPORTS,
    super::tools_and_help::ABOUT,
    super::tools_and_help::CLEAR_TOASTS,
    Spec {
        action: Action::TextFind,
        id: "find_in_document",
        label: tr_key!("Find in current document"),
        category: tr_key!("Text"),
        defaults: &[(CTRL, Key::F)],
        enabled: a_text_document,
        panel_key: true,
        run: |state, ctx| find_in_text(ctx, state, false),
        ..BASE
    },
    Spec {
        action: Action::TextReplace,
        id: "replace_in_document",
        label: tr_key!("Replace in current document"),
        category: tr_key!("Text"),
        defaults: &[(CTRL, Key::H)],
        enabled: a_text_document,
        panel_key: true,
        run: |state, ctx| find_in_text(ctx, state, true),
        ..BASE
    },
    Spec {
        action: Action::TextGoToLine,
        id: "go_to_line",
        label: tr_key!("Go to line…"),
        category: tr_key!("Text"),
        defaults: &[(CTRL, Key::G)],
        hover: tr_key!("Move the caret to a line of the text document, by its number"),
        enabled: a_text_document,
        panel_key: true,
        run: |state, ctx| {
            let Some(handle) = text_editor_for(ctx, state) else {
                return;
            };
            let Ok(mut editor) = handle.lock() else {
                return;
            };
            editor.go_to = Some(panels::text_editor::go_to_line::GoToLine {
                typed: String::new(),
                focus: true,
            });
        },
        ..BASE
    },
    Spec {
        action: Action::TextFindInOpenDocs,
        id: "find_in_open_documents",
        label: tr_key!("Find in open documents"),
        category: tr_key!("Text"),
        defaults: &[(CTRL_SHIFT, Key::F)],
        enabled: a_text_document,
        run: |state, _| {
            let find = state
                .panels
                .get::<panels::text_editor::find_panel::CrossDocFindState>();
            find.visible = true;
            find.want_focus = true;
        },
        ..BASE
    },
];

pub fn spec(action: Action) -> Option<&'static Spec> {
    COMMANDS.iter().find(|spec| spec.action == action)
}

fn open_files(state: &mut AppState, _: &egui::Context) {
    if let Some(paths) = file_dialogs::open_any_files() {
        state.load_files(paths);
    }
}
