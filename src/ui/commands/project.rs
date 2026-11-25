//! The commands of an open project: its Project menu, its keys for a build and for a workspace, and what the Assets table's rows offer.

use std::path::PathBuf;

use egui::Key;

use super::table::{a_project, none_if, Action, Place, Spec, BASE};
use crate::platform::file_dialogs;
use crate::state::workbench::Kept;
use crate::state::{AppState, PROJECT_EXTENSION};
use crate::ui::dock::Workspace;
use crate::ui::shortcuts::ShortcutMods;

const CTRL: ShortcutMods = ShortcutMods::ctrl();
const NONE: ShortcutMods = ShortcutMods::none();

const NO_BUILD: &str = tr_key!("The project declares no build: Project ▸ Add build… adds one");
const NO_ASSET: &str = tr_key!("Select an asset in the Assets table first");

fn has_a_build(state: &AppState) -> Result<(), &'static str> {
    a_project(state)?;
    none_if(
        state.project_carried.container.outputs.is_empty(),
        crate::i18n::tr(NO_BUILD),
    )
}

fn idle(state: &AppState) -> Result<(), &'static str> {
    none_if(
        state.workbench.builds().is_running(),
        tr!("A build is already running"),
    )
}

fn idle_with_a_build(state: &AppState) -> Result<(), &'static str> {
    has_a_build(state)?;
    idle(state)
}

/// ⚠ Not `idle_with_a_build`: a build removed from the project leaves what it wrote in `build/`, and Clean is how it goes.
fn idle_in_a_project(state: &AppState) -> Result<(), &'static str> {
    a_project(state)?;
    idle(state)
}

fn an_asset(state: &AppState) -> Result<(), &'static str> {
    a_project(state)?;
    none_if(state.selected_asset().is_none(), crate::i18n::tr(NO_ASSET))
}

fn a_source(state: &AppState) -> Result<(), &'static str> {
    an_asset(state)?;
    let source = state
        .selected_asset()
        .is_some_and(|row| row.kept == Kept::Source);
    none_if(!source, tr!("Only a source folder is cooked"))
}

/// The selected asset is a drawable or a drawables source that can take a model now.
fn a_model_source(state: &AppState) -> Result<(), &'static str> {
    an_asset(state)?;
    let refused = (state.selected_asset()).and_then(|row| state.replace_geometry_refused(row));
    refused.map_or(Ok(()), Err)
}

/// The selected asset is a source taken from the game that no check is running on.
fn a_source_from_the_game(state: &AppState) -> Result<(), &'static str> {
    an_asset(state)?;
    let refused = (state.selected_asset()).and_then(|row| state.check_refused(row));
    refused.map_or(Ok(()), Err)
}

fn a_file(state: &AppState) -> Result<(), &'static str> {
    an_asset(state)?;
    match state.selected_asset().map(|row| row.kept) {
        Some(Kept::File) => Ok(()),
        Some(Kept::Pack) => Err(tr!("A garment is removed from its pack in the Wardrobe")),
        _ => Err(tr!("A source folder is removed by deleting its folder")),
    }
}

pub(super) const ADD_CONTENT: Spec = Spec {
    action: Action::AddContent,
    id: "add_content",
    label: tr_key!("Add files…"),
    category: tr_key!("Project"),
    hover: tr_key!("Add your own files to the project: images, models, scripts, interface and configuration join it as they are, a source folder is taken as it is, and a folder is scanned. A game file you pick here is imported, as Import game files… does"),
    enabled: a_project,
    run: |state, _| {
        if let Some(files) = file_dialogs::open_any_files() {
            let done = state.add_to_project(files);
            if done.added > 0 {
                state.show_toast(tr!("{files} added to the project", files = crate::state::front_door::counted(done.added, "file")));
            }
            if let Some(first) = done.refused.first() {
                let more = done.refused.len() - 1;
                let tail = if more > 0 {
                    tr!(" (and {more} more)", more = more)
                } else {
                    String::new()
                };
                state.show_warning_toast(format!("{first}{tail}"));
            }
        }
    },
    ..BASE
};

/// Why the import commands wait: no project, or an import already running.
fn can_import(state: &AppState) -> Result<(), &'static str> {
    state.import_refused().map_or(Ok(()), Err)
}

pub(super) const IMPORT_GAME_FILES: Spec = Spec {
    action: Action::ImportGameFiles,
    id: "import_game_files",
    label: tr_key!("Import game files…"),
    category: tr_key!("Project"),
    hover: tr_key!("Decompile the game files you pick into sources of the project: a .ytd becomes a textures source, a .ydr a drawable, a .ydd a drawables, a .yft a fragment and a .ymt, .ymap or .ytyp a metadata source. A compiled file with no source form yet is kept as it is in the Files content. The files you pick are not changed"),
    enabled: can_import,
    run: |state, _| {
        if let Some(files) = file_dialogs::pick_game_files() {
            state.import_game_files(files);
        }
    },
    ..BASE
};

pub(super) const IMPORT_FOLDER: Spec = Spec {
    action: Action::ImportFolder,
    id: "import_folder",
    label: tr_key!("Import folder…"),
    category: tr_key!("Project"),
    hover: tr_key!("Decompile every game file in a folder you pick, and in the folders under it, into sources of the project, as a drop of the folder on the Project workspace does. The walk goes a bounded number of levels and files down, which the report names, and an archive in it is listed and left out. The files in the folder are not changed"),
    enabled: can_import,
    run: |state, _| {
        if let Some(folder) = file_dialogs::pick_import_folder() {
            state.import_game_files(vec![folder]);
        }
    },
    ..BASE
};

pub(super) const UNDO_IMPORT: Spec = Spec {
    action: Action::UndoImport,
    id: "undo_import",
    label: tr_key!("Undo import"),
    category: tr_key!("Project ▸ History"),
    hover: tr_key!("Remove exactly the folders and files the last import created. Nothing is removed if any of them was changed since"),
    enabled: |state| {
        a_project(state)?;
        none_if(
            !state.import_can_be_undone(),
            tr!("No import has been made to take back"),
        )
    },
    run: |state, _| state.undo_last_import(),
    ..BASE
};

/// Whether Edit's Undo and Redo are the project's: the Project workspace is about the project, not about the document last clicked.
pub(super) fn undo_is_the_projects(state: &AppState) -> bool {
    state.project_path.is_some() && state.active_workspace == crate::ui::dock::Workspace::Project
}

pub(super) fn can_undo_project(state: &AppState) -> Result<(), &'static str> {
    a_project(state)?;
    none_if(
        !state.project_can_undo(),
        tr!("Nothing to undo in the project yet"),
    )
}

pub(super) fn can_redo_project(state: &AppState) -> Result<(), &'static str> {
    a_project(state)?;
    none_if(
        !state.project_can_redo(),
        tr!("Nothing to redo in the project"),
    )
}

pub(super) fn undo_the_project(state: &mut AppState) {
    match state.undo_project() {
        Ok(label) => state.show_toast(tr!("Undid '{label}'", label = label)),
        Err(why) => state.show_warning_toast(why),
    }
}

pub(super) fn redo_the_project(state: &mut AppState) {
    match state.redo_project() {
        Ok(label) => state.show_toast(tr!("Redid '{label}'", label = label)),
        Err(why) => state.show_warning_toast(why),
    }
}

pub(super) const UNDO_PROJECT: Spec = Spec {
    action: Action::UndoProject,
    id: "undo_project",
    label: tr_key!("Undo project change"),
    category: tr_key!("Project"),
    hover: tr_key!("Take back the last change to what the project holds: a file or content added or removed, a build, a link, a file located. History lists every step, and a click there goes back to any of them. Edit ▸ Undo does the same while the Project workspace is on screen"),
    enabled: can_undo_project,
    label_now: Some(|state| match state.project_undo_label() {
        Some(last) => tr!("Undo \"{last}\"", last = last),
        None => tr!("Undo project change").to_string(),
    }),
    run: |state, _| undo_the_project(state),
    ..BASE
};

pub(super) const REDO_PROJECT: Spec = Spec {
    action: Action::RedoProject,
    id: "redo_project",
    label: tr_key!("Redo project change"),
    category: tr_key!("Project"),
    hover: tr_key!("Make again the change of the project that was taken back last. A new change drops what was taken back. Edit ▸ Redo does the same while the Project workspace is on screen"),
    enabled: can_redo_project,
    label_now: Some(|state| match state.project_redo_label() {
        Some(next) => tr!("Redo \"{next}\"", next = next),
        None => tr!("Redo project change").to_string(),
    }),
    run: |state, _| redo_the_project(state),
    ..BASE
};

pub(super) const OPEN_PROJECT_HERE: Spec = Spec {
    action: Action::OpenProjectHere,
    id: "open_project_here",
    label: tr_key!("Start this project on this workspace"),
    category: tr_key!("Project ▸ Properties and sharing"),
    hover: tr_key!("Write the workspace on screen into the project file as the one a machine that never opened it starts on. Switching workspace changes nothing in the file, so a project in Git does not change until someone says this"),
    enabled: |state| {
        a_project(state)?;
        none_if(
            state.project_carried.workspace == state.active_workspace,
            tr!("The project opens on this workspace already"),
        )
    },
    run: |state, _| state.open_project_here(),
    ..BASE
};

pub(super) const ADD_IMAGES: Spec = Spec {
    action: Action::AddImagesAsTexturesSource,
    id: "add_images_as_textures_source",
    label: tr_key!("Add images as a textures source…"),
    category: tr_key!("Project"),
    hover: tr_key!("Put the images you pick in one new textures source of the project, written as a person authored it; it compiles to a dictionary like any source. PNG, TGA, TIFF, BMP, JPG or DDS"),
    enabled: can_import,
    run: |state, _| {
        if let Some(images) = file_dialogs::pick_images() {
            state.add_images_as_textures_source(images);
        }
    },
    ..BASE
};

pub(super) const NEW_TEXTURES_SOURCE: Spec = Spec {
    action: Action::NewTexturesSource,
    id: "new_textures_source",
    label: tr_key!("New texture dictionary…"),
    category: tr_key!("Project"),
    hover: tr_key!("Start an empty texture dictionary in the project: a source with no texture, to be filled with images or DDS files, which compiles to a dictionary. Undo import takes it back"),
    enabled: |state| state.new_source_refused().map_or(Ok(()), Err),
    run: |state, _| state.ask_for_a_new_source(or_source::textures::KIND),
    ..BASE
};

pub(super) const ADD_BUILD: Spec = Spec {
    action: Action::AddBuild,
    id: "add_build",
    label: tr_key!("Add build…"),
    category: tr_key!("Project"),
    hover: tr_key!("Declare a build: a FiveM resource, a singleplayer DLC or an OIV package written from the content you choose"),
    enabled: a_project,
    run: |state, _| state.ask_about_builds(true),
    ..BASE
};

pub(super) const EDIT_BUILD: Spec = Spec {
    action: Action::EditBuild,
    id: "edit_build",
    label: tr_key!("Edit build…"),
    category: tr_key!("Project"),
    hover: tr_key!("Change the chosen build, or any other: its name, kind and edition, the content it takes, whether it divides when over a ceiling, and its ceilings"),
    enabled: |state| {
        has_a_build(state)?;
        none_if(state.workbench.builds().is_running(), tr!("A build is running"))
    },
    run: |state, _| state.ask_about_builds(false),
    ..BASE
};

fn a_link_is_chosen(state: &AppState) -> Result<(), &'static str> {
    a_project(state)?;
    none_if(
        state.selected_link().is_none(),
        tr!("Choose linked content in the Content panel first"),
    )
}

pub(super) const LINK_CONTENT: Spec = Spec {
    action: Action::LinkContent,
    id: "link_content",
    label: tr_key!("Link content…"),
    category: tr_key!("Project"),
    hover: tr_key!("Read the folder of a pack, or of a whole project, that another project holds into this project: its Files content and each pack it holds, and none of the links it holds. That project stays its only writer, this one reads it into its builds"),
    enabled: a_project,
    run: |state, _| {
        if let Some(folder) = file_dialogs::pick_open_folder() {
            match state.link_folder(&folder) {
                Ok(said) => state.show_toast(said),
                Err(why) => state.show_error_toast(why),
            }
        }
    },
    ..BASE
};

pub(super) const UNLINK_CONTENT: Spec = Spec {
    action: Action::UnlinkContent,
    id: "unlink_content",
    label: tr_key!("Unlink content"),
    category: tr_key!("Project"),
    hover: tr_key!("Stop reading the chosen linked content into this project; its folder and the project that holds it are not touched"),
    enabled: a_link_is_chosen,
    run: |state, _| {
        if let Some(id) = state.workbench.selected_content.clone() {
            match state.unlink_content(&id) {
                Ok(()) => state.show_toast(tr!("Unlinked {id}; Save project keeps it", id = id)),
                Err(why) => state.show_error_toast(why),
            }
        }
    },
    ..BASE
};

pub(super) const CHECK_BEFORE_SHARING: Spec = Spec {
    action: Action::CheckBeforeSharing,
    id: "check_before_sharing",
    label: tr_key!("Check before sharing…"),
    category: tr_key!("Project ▸ Properties and sharing"),
    hover: tr_key!("List what the project's folder holds that is not yours to send: paths of this machine in the project file, sources taken from the game, and this machine's own folder. It changes nothing"),
    enabled: a_project,
    run: |state, _| state.check_before_sharing(),
    ..BASE
};

pub(super) const SAVE_AS_TEMPLATE: Spec = Spec {
    action: Action::SaveAsTemplate,
    id: "save_as_template",
    label: tr_key!("Save as template…"),
    category: tr_key!("Project ▸ Properties and sharing"),
    hover: tr_key!("Write the project's content, builds and layout as a template file another project can be made from. It holds no file, no link and no path of this machine"),
    enabled: a_project,
    run: |state, _| {
        let name = state.project_name().unwrap_or_default();
        let file = file_dialogs::save_template_file(&format!("{name}.{}", or_project::template::EXTENSION));
        if let Some(file) = file {
            match state.save_project_as_template(&file) {
                Ok(left_out) if left_out.is_empty() => {
                    state.show_toast(tr!("Saved the template {file}", file = file.display()));
                }
                Ok(left_out) => state.show_toast(tr!("Saved the template {file}; left out: {left_out}", file = file.display(), left_out = left_out.join("; "))),
                Err(why) => state.show_error_toast(why),
            }
        }
    },
    ..BASE
};

pub(super) const APPLY_TEMPLATE: Spec = Spec {
    action: Action::ApplyTemplate,
    id: "apply_template",
    label: tr_key!("Apply a template…"),
    category: tr_key!("Project ▸ Properties and sharing"),
    hover: tr_key!("Add the content and builds a template file states that this project does not hold yet. Nothing the project has is replaced, and Undo takes it back"),
    enabled: a_project,
    run: |state, _| {
        if let Some(file) = file_dialogs::open_template_file() {
            match state.apply_template(&file) {
                Ok(said) => state.show_toast(said),
                Err(why) => state.show_error_toast(why),
            }
        }
    },
    ..BASE
};

pub(super) const BUILD: Spec = Spec {
    action: Action::BuildProject,
    id: "build_project",
    label: tr_key!("Build"),
    category: tr_key!("Project"),
    defaults: &[(CTRL, Key::B)],
    hover: tr_key!(
        "Build the chosen build: plan it, write it to a fresh folder, read every file \
            back and swap it in, keeping the last good build"
    ),
    enabled: idle_with_a_build,
    // The project and the build are the ones on screen, so the line runs as it is copied.
    cli: Some(|state| {
        let project = state.project_path.as_ref().map_or_else(
            || "<project.openrage-proj>".to_string(),
            |path| format!("\"{}\"", path.display()),
        );
        match state.active_output_id() {
            Some(build) => format!("openrage-cli project build {project} --build {build}"),
            None => format!("openrage-cli project build {project}"),
        }
    }),
    run: |state, _| state.build_selected(),
    ..BASE
};

pub(super) const BUILD_ALL: Spec = Spec {
    action: Action::BuildAll,
    id: "build_all",
    label: tr_key!("Build all"),
    category: tr_key!("Project"),
    hover: tr_key!("Build every build the project declares, one after the other"),
    enabled: idle_with_a_build,
    run: |state, _| state.build_all(),
    ..BASE
};

pub(super) const CLEAN: Spec = Spec {
    action: Action::CleanProject,
    id: "clean_project",
    label: tr_key!("Clean…"),
    category: tr_key!("Project"),
    hover: tr_key!("Remove what the builds wrote under build/, after asking; no folder OpenRage did not build is touched"),
    enabled: idle_in_a_project,
    run: |state, _| state.ask_to_clean(),
    ..BASE
};

pub(super) const PROPERTIES: Spec = Spec {
    action: Action::ProjectProperties,
    id: "project_properties",
    label: tr_key!("Properties…"),
    category: tr_key!("Project ▸ Properties and sharing"),
    hover: tr_key!("The project's name, place, content and builds, in the Project workspace"),
    enabled: a_project,
    run: |state, _| state.show_project_properties(),
    ..BASE
};

pub(super) const REVEAL_BUILD: Spec = Spec {
    action: Action::RevealBuild,
    id: "reveal_build",
    label: tr_key!("Show the build"),
    category: tr_key!("Project"),
    hover: tr_key!("Show the folder the chosen build was written to in the file manager"),
    enabled: |state| {
        has_a_build(state)?;
        let built = state.active_output_id().is_some_and(|id| {
            matches!(
                state.workbench.dot(&id),
                crate::state::workbench::BuildDot::Built
                    | crate::state::workbench::BuildDot::Unchecked
            )
        });
        none_if(!built, tr!("Build first: nothing has been written yet"))
    },
    run: |state, _| state.show_the_build(),
    ..BASE
};

pub(super) const REVEAL: Spec = Spec {
    action: Action::RevealProject,
    id: "reveal_project",
    label: tr_key!("Reveal in the file manager"),
    category: tr_key!("Project ▸ Properties and sharing"),
    hover: tr_key!("Show the project's file in the system's file manager"),
    enabled: a_project,
    run: |state, _| {
        if let Some(path) = &state.project_path {
            crate::platform::shell::reveal(path);
        }
    },
    ..BASE
};

pub(super) const OPEN_ASSET: Spec = Spec {
    action: Action::OpenAsset,
    id: "open_asset",
    label: tr_key!("Open asset"),
    category: tr_key!("Project ▸ Assets"),
    hover: tr_key!("Open the selected asset in the workspace that suits its kind"),
    enabled: an_asset,
    run: |state, _| {
        if let Some(key) = state.workbench.selected.clone() {
            state.open_asset(&key, None);
        }
    },
    ..BASE
};

pub(super) const REVEAL_ASSET: Spec = Spec {
    action: Action::RevealAsset,
    id: "reveal_asset",
    label: tr_key!("Reveal asset in the file manager"),
    category: tr_key!("Project ▸ Assets"),
    enabled: an_asset,
    run: |state, _| {
        if let Some(row) = state.selected_asset() {
            crate::platform::shell::reveal(&row.path);
        }
    },
    ..BASE
};

pub(super) const RECOOK_ASSET: Spec = Spec {
    action: Action::RecookAsset,
    id: "recook_asset",
    label: tr_key!("Compile again"),
    category: tr_key!("Project ▸ Assets"),
    hover: tr_key!("Compile the selected source folder into the project's shadow folder now, as saving a file in it does"),
    enabled: a_source,
    run: |state, _| {
        if let Some(key) = state.workbench.selected.clone() {
            state.recook_asset(&key);
        }
    },
    ..BASE
};

pub(super) const REPLACE_GEOMETRY: Spec = Spec {
    action: Action::ReplaceGeometry,
    id: "replace_geometry",
    label: tr_key!("Replace geometry…"),
    category: tr_key!("Project ▸ Assets"),
    hover: tr_key!("Put the model in a glTF, GLB or FBX into the selected drawable or drawables source, and list what the drawable has no place for. Nothing changes unless the source still compiles with it"),
    enabled: a_model_source,
    run: |state, _| {
        let Some(key) = state.workbench.selected.clone() else {
            return;
        };
        if let Some(file) = file_dialogs::pick_model() {
            state.start_replacing_geometry(&key, file);
        }
    },
    ..BASE
};

pub(super) const CHECK_AGAINST_GAME: Spec = Spec {
    action: Action::CheckAgainstGame,
    id: "check_against_game",
    label: tr_key!("Check against the game"),
    category: tr_key!("Project ▸ Assets"),
    hover: tr_key!("Compare the game's file this source was taken from with the install's now, once. The install is read only when you ask, never when the project is scanned"),
    enabled: a_source_from_the_game,
    run: |state, _| {
        if let Some(key) = state.workbench.selected.clone() {
            state.check_against_game(&key);
        }
    },
    ..BASE
};

pub(super) const REMOVE_ASSET: Spec = Spec {
    action: Action::RemoveAsset,
    id: "remove_asset",
    label: tr_key!("Remove asset from the project"),
    category: tr_key!("Project ▸ Assets"),
    hover: tr_key!("The file stays on disk"),
    enabled: a_file,
    run: |state, _| {
        if let Some(key) = state.workbench.selected.clone() {
            state.remove_asset(&key);
        }
    },
    ..BASE
};

pub(super) const REMOVE_BUILD: Spec = Spec {
    action: Action::RemoveBuild,
    id: "remove_build",
    label: tr_key!("Remove the chosen build"),
    category: tr_key!("Project"),
    hover: tr_key!("Stop declaring the build; what it already wrote stays until Clean"),
    enabled: has_a_build,
    run: |state, _| {
        if let Some(id) = state.active_output_id() {
            state.remove_build(&id);
        }
    },
    ..BASE
};

pub(super) const QUICK_LOOK: Spec = Spec {
    action: Action::QuickLook,
    id: "quick_look",
    label: tr_key!("Quick look"),
    category: tr_key!("Project ▸ Assets"),
    defaults: &[(NONE, Key::Space)],
    hover: tr_key!("Show the selected asset's, or in the Wardrobe the selected garment's, facts and picture without opening it"),
    enabled: |state| match looks_at_garments(state) {
        true => none_if(
            state.picked_garments().is_empty(),
            tr!("Select a garment in the Wardrobe first"),
        ),
        false => an_asset(state),
    },
    place: Place::Lists,
    run: |state, _| match looks_at_garments(state) {
        true => state.workbench.wardrobe.quick_look = !state.workbench.wardrobe.quick_look,
        false => state.workbench.quick_look = !state.workbench.quick_look,
    },
    ..BASE
};

/// Whether the quick look is the Wardrobe's: its workspace is on screen and the project has clothes in it.
fn looks_at_garments(state: &AppState) -> bool {
    state.active_workspace == Workspace::Wardrobe && state.has_clothing()
}

/// Ctrl+1 to Ctrl+9: the tab in that place of the tab row.
macro_rules! go_to_workspace {
    ($name:ident, $action:ident, $id:literal, $n:literal, $key:expr, $label:expr) => {
        pub(super) const $name: Spec = Spec {
            action: Action::$action,
            id: $id,
            label: $label,
            category: tr_key!("View ▸ Workspace"),
            defaults: &[(CTRL, $key)],
            hover: tr_key!("Bring the tab in this place of the tab row in front: the screens of the project first, then the files"),
            enabled: |state| {
                none_if(
                    state.tab_places().len() < $n,
                    tr!("There is no tab in this place"),
                )
            },
            label_now: Some(|state| match state.tab_places().get($n - 1) {
                Some(place) => tr!("Go to tab {n} ({tab})", n = $n, tab = state.tab_place_name(*place)),
                None => crate::i18n::tr($label).to_string(),
            }),
            run: |state, _| {
                state.go_to_tab_number($n);
            },
            ..BASE
        };
    };
}

go_to_workspace!(
    WORKSPACE_1,
    Workspace1,
    "workspace_1",
    1,
    Key::Num1,
    tr_key!("Go to tab 1")
);
go_to_workspace!(
    WORKSPACE_2,
    Workspace2,
    "workspace_2",
    2,
    Key::Num2,
    tr_key!("Go to tab 2")
);
go_to_workspace!(
    WORKSPACE_3,
    Workspace3,
    "workspace_3",
    3,
    Key::Num3,
    tr_key!("Go to tab 3")
);
go_to_workspace!(
    WORKSPACE_4,
    Workspace4,
    "workspace_4",
    4,
    Key::Num4,
    tr_key!("Go to tab 4")
);
go_to_workspace!(
    WORKSPACE_5,
    Workspace5,
    "workspace_5",
    5,
    Key::Num5,
    tr_key!("Go to tab 5")
);
go_to_workspace!(
    WORKSPACE_6,
    Workspace6,
    "workspace_6",
    6,
    Key::Num6,
    tr_key!("Go to tab 6")
);
go_to_workspace!(
    WORKSPACE_7,
    Workspace7,
    "workspace_7",
    7,
    Key::Num7,
    tr_key!("Go to tab 7")
);
go_to_workspace!(
    WORKSPACE_8,
    Workspace8,
    "workspace_8",
    8,
    Key::Num8,
    tr_key!("Go to tab 8")
);
go_to_workspace!(
    WORKSPACE_9,
    Workspace9,
    "workspace_9",
    9,
    Key::Num9,
    tr_key!("Go to tab 9")
);

/// The command of the `n`th workspace, counting from 0.
pub const WORKSPACE_ACTIONS: [Action; 9] = [
    Action::Workspace1,
    Action::Workspace2,
    Action::Workspace3,
    Action::Workspace4,
    Action::Workspace5,
    Action::Workspace6,
    Action::Workspace7,
    Action::Workspace8,
    Action::Workspace9,
];

pub(super) fn save_project_as(state: &mut AppState, _: &egui::Context) {
    if let Some(path) = save_project_dialog(state) {
        match state.save_project_to(path) {
            Ok(()) => state.show_toast(tr!("Project saved")),
            Err(e) => state.show_error_toast(tr!("Failed to save project: {e}", e = e)),
        }
    }
}

/// Whether Save (Ctrl+S) has something to write: the document in front, or with none open the project's own changes.
pub(super) fn something_to_save(state: &AppState) -> Result<(), &'static str> {
    if state.active_document.is_none() && state.project_path.is_some() {
        super::table::not_busy(state)?;
        return none_if(
            !state.project_carried.changed,
            tr!("The project has no unsaved changes"),
        );
    }
    super::table::a_document_that_is_written(state)?;
    // One never saved has nowhere it stands as it is: it is saved to be given a place.
    let unchanged = (state.active_document()).is_some_and(|d| !d.dirty && d.path.is_some());
    none_if(unchanged, tr!("The document has no unsaved changes"))
}

/// Save (Ctrl+S): the document in front, one never saved asking where; with none open, the project.
pub(super) fn save(state: &mut AppState, _: &egui::Context) {
    if state.active_document.is_none() {
        state.save_active_project(save_project_dialog);
    } else if state.active_document().is_some_and(|d| d.path.is_some()) {
        state.save_active_document();
    } else if let Some(path) = crate::ui::panels::menu::save_dialog_for_active(state) {
        state.save_active_document_as(path);
    }
}

pub(super) fn save_project_dialog(state: &AppState) -> Option<PathBuf> {
    let default_name = state
        .project_path
        .as_ref()
        .and_then(|p| p.file_name().and_then(|s| s.to_str()).map(str::to_string))
        .unwrap_or_else(|| format!("session.{PROJECT_EXTENSION}"));
    file_dialogs::save_project_file(Some(&default_name))
}

#[cfg(test)]
mod tests {
    use winit::keyboard::{Key as LogicalKey, KeyCode, PhysicalKey};

    use super::*;
    use crate::state::Prefs;
    use crate::ui::commands::{self, COMMANDS};
    use crate::ui::dock::Workspace;
    use crate::ui::shortcuts::{self, PRIMARY_STATE};

    fn a_project(state: &mut AppState) {
        state.project_path = Some(std::path::PathBuf::from("p.openrage-proj"));
    }

    fn pressed(key: &str, code: KeyCode, state: &AppState) -> Option<Action> {
        shortcuts::action_for_press(
            &LogicalKey::Character(key.into()),
            PhysicalKey::Code(code),
            PRIMARY_STATE,
            false,
            &state.prefs,
        )
    }

    #[test]
    fn ctrl_b_builds_and_ctrl_1_to_ctrl_9_go_to_a_tab() {
        let mut state = AppState::from_prefs(Prefs::default());
        assert_eq!(
            pressed("b", KeyCode::KeyB, &state),
            Some(Action::BuildProject)
        );
        let digits = [
            ("1", KeyCode::Digit1),
            ("2", KeyCode::Digit2),
            ("3", KeyCode::Digit3),
            ("4", KeyCode::Digit4),
            ("5", KeyCode::Digit5),
            ("6", KeyCode::Digit6),
            ("7", KeyCode::Digit7),
            ("8", KeyCode::Digit8),
            ("9", KeyCode::Digit9),
        ];
        for (n, (text, code)) in digits.into_iter().enumerate() {
            assert_eq!(
                pressed(text, code, &state),
                Some(WORKSPACE_ACTIONS[n]),
                "Ctrl+{text} is not tab {}",
                n + 1
            );
        }

        // The number is the place of a tab: the files' with no project, and after the project's overview with one.
        let ctx = egui::Context::default();
        state.documents.clear();
        state.active_document = None;
        let text = state.new_empty_document(crate::state::DocumentKind::Text);
        let model = state.new_empty_document(crate::state::DocumentKind::Ydr);
        commands::run_action(&mut state, &ctx, Action::Workspace2);
        assert_eq!(state.active_document, Some(model));
        commands::run_action(&mut state, &ctx, Action::Workspace1);
        assert_eq!(state.active_document, Some(text));

        a_project(&mut state);
        commands::run_action(&mut state, &ctx, Action::Workspace1);
        assert_eq!(state.active_workspace, Workspace::Project);
        commands::run_action(&mut state, &ctx, Action::Workspace3);
        assert_eq!(state.active_document, Some(model));
    }

    #[test]
    fn a_number_with_no_tab_says_so_and_changes_nothing() {
        let mut state = AppState::from_prefs(Prefs::default());
        let ctx = egui::Context::default();
        let command = commands::of(&state, Action::Workspace9).expect("a row");
        assert_eq!(command.enabled, Err("There is no tab in this place"));
        commands::run_action(&mut state, &ctx, Action::Workspace9);
        assert_eq!(state.active_workspace, Workspace::default());
    }

    #[test]
    fn the_workspace_menu_names_the_key_of_a_screen_that_is_a_tab_and_of_no_other() {
        let mut state = AppState::from_prefs(Prefs::default());
        a_project(&mut state);
        let shown = commands::workspaces(&state);
        let keys = format!("{}+", shortcuts::primary_modifier_name());
        // The number keys go to the tabs, and of the workspaces only the project's screens are tabs.
        for command in &shown {
            let wanted = (command.label == "Project").then(|| format!("{keys}1"));
            assert_eq!(command.binding, wanted, "{}", command.label);
        }
        assert_eq!(shown[0].label, "Project");
    }

    #[test]
    fn a_build_waits_with_its_reason_until_a_project_with_a_build_is_open() {
        let mut state = AppState::from_prefs(Prefs::default());
        let reason = |state: &AppState, action| commands::of(state, action).map(|c| c.enabled);
        assert_eq!(
            reason(&state, Action::BuildProject),
            Some(Err("No project is open"))
        );
        a_project(&mut state);
        assert_eq!(
            reason(&state, Action::BuildProject),
            Some(Err(
                "The project declares no build: Project ▸ Add build… adds one"
            ))
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
        for action in [Action::BuildProject, Action::BuildAll, Action::CleanProject] {
            assert_eq!(reason(&state, action), Some(Ok(())), "{action:?}");
        }
        assert_eq!(
            reason(&state, Action::OpenAsset),
            Some(Err(crate::i18n::tr(NO_ASSET))),
            "an asset command waits for a selection"
        );
    }

    #[test]
    fn clean_is_offered_with_no_build_declared_because_a_removed_build_left_files() {
        let mut state = AppState::from_prefs(Prefs::default());
        a_project(&mut state);
        assert!(state.project_carried.container.outputs.is_empty());
        assert_eq!(
            commands::of(&state, Action::CleanProject).map(|c| c.enabled),
            Some(Ok(()))
        );
    }

    #[test]
    fn every_project_command_is_in_the_table_with_one_id_and_a_place() {
        let ours = [
            Action::AddContent,
            Action::AddBuild,
            Action::EditBuild,
            Action::BuildProject,
            Action::BuildAll,
            Action::CleanProject,
            Action::ProjectProperties,
            Action::RevealProject,
            Action::OpenAsset,
            Action::RevealAsset,
            Action::RecookAsset,
            Action::ReplaceGeometry,
            Action::CheckAgainstGame,
            Action::RemoveAsset,
            Action::RemoveBuild,
            Action::QuickLook,
            Action::LinkContent,
            Action::UnlinkContent,
            Action::SaveAsTemplate,
            Action::ApplyTemplate,
            Action::CheckBeforeSharing,
        ];
        for action in ours.into_iter().chain(WORKSPACE_ACTIONS) {
            let spec = COMMANDS
                .iter()
                .find(|spec| spec.action == action)
                .unwrap_or_else(|| panic!("{action:?} is not a command"));
            assert!(!spec.label.is_empty() && !spec.category.is_empty());
        }
    }
}
