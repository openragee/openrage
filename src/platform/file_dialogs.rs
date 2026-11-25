//! The platform's file dialogs, which a stand-in answers in a test build and a self-check.

use std::cell::RefCell;
use std::path::PathBuf;

use crate::state::{kinds, DocumentKind};

thread_local! {
    static NEXT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static LAST: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The title of the dialog a stand-in answered last on this thread, which a test reads to see which dialog a button asked for.
#[cfg(test)]
pub fn last_dialog() -> Option<String> {
    LAST.with(|last| last.borrow().clone())
}

/// Have the next dialog a stand-in answers pick `path`, as a script's `pick` step says; one asked with nothing named is cancelled.
pub fn answer_next(path: PathBuf) {
    NEXT.with(|next| *next.borrow_mut() = Some(path));
}

/// What the dialog titled `title`, which `show` finishes and shows, picks; a stand-in answers instead in a test build and a self-check.
pub fn ask(title: &str, show: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>) -> Option<PathBuf> {
    if super::standing_in() {
        return stand_in(title);
    }
    show(rfd::FileDialog::new().set_title(title))
}

fn stand_in(title: &str) -> Option<PathBuf> {
    #[cfg(test)]
    LAST.with(|last| *last.borrow_mut() = Some(title.to_string()));
    let answer = NEXT.with(|next| next.borrow_mut().take());
    match &answer {
        Some(path) => tracing::info!(
            "the stand-in for the dialog \"{title}\" picked {}",
            path.display()
        ),
        None => tracing::info!("the stand-in for the dialog \"{title}\" was cancelled"),
    }
    answer
}

pub fn open_any_file() -> Option<PathBuf> {
    ask("Open File", |dialog| any_file_filters(dialog).pick_file())
}

pub fn open_any_files() -> Option<Vec<PathBuf>> {
    if super::standing_in() {
        return stand_in("Open File").map(|path| vec![path]);
    }
    let picked = any_file_filters(rfd::FileDialog::new().set_title("Open File")).pick_files()?;
    if picked.is_empty() {
        None
    } else {
        Some(picked)
    }
}

/// The files `show` picks in a dialog titled `title`; a stand-in answers the one file a script named.
fn ask_many(
    title: &str,
    show: impl FnOnce(rfd::FileDialog) -> Option<Vec<PathBuf>>,
) -> Option<Vec<PathBuf>> {
    if super::standing_in() {
        return stand_in(title).map(|path| vec![path]);
    }
    show(rfd::FileDialog::new().set_title(title)).filter(|picked| !picked.is_empty())
}

/// Pick the game files to decompile into sources of the open project.
pub fn pick_game_files() -> Option<Vec<PathBuf>> {
    ask_many("Import Game Files", |dialog| {
        any_file_filters(dialog).pick_files()
    })
}

/// Pick the folder whose game files are decompiled into sources of the open project.
pub fn pick_import_folder() -> Option<PathBuf> {
    ask("Import Folder", rfd::FileDialog::pick_folder)
}

/// Pick the images a new textures source holds.
pub fn pick_images() -> Option<Vec<PathBuf>> {
    use or_io::authoring::texture_import::SourceFormat;

    ask_many("Add images as a textures source", |dialog| {
        dialog
            .add_filter("Images", &SourceFormat::EXTENSIONS)
            .add_filter("All Files", &["*"])
            .pick_files()
    })
}

fn any_file_filters(dialog: rfd::FileDialog) -> rfd::FileDialog {
    let everything = kinds::openable_extensions();
    let mut dialog = dialog.add_filter("Supported Files", &everything);

    for info in kinds::all() {
        let exts = info.extensions();
        if exts.is_empty() {
            continue;
        }
        dialog = dialog.add_filter(info.filter_label(), exts);
    }

    dialog.add_filter("All Files", &["*"])
}

pub fn save_any_file(kind: DocumentKind, default_name: Option<&str>) -> Option<PathBuf> {
    let info = kind.info();

    if info.keeps_source_extension {
        return save_keeping_extension(&info, default_name);
    }

    let ext = info.default_extension()?;

    ask(&info.save_title(), |dialog| {
        let mut dialog = dialog.add_filter(info.noun, &[ext]);
        if let Some(name) = default_name {
            dialog = dialog.set_file_name(name);
        }
        dialog.save_file()
    })
}

fn save_keeping_extension(info: &kinds::KindInfo, default_name: Option<&str>) -> Option<PathBuf> {
    let existing = default_name
        .and_then(|n| std::path::Path::new(n).extension())
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    let primary = match existing.as_deref() {
        Some(ext) => ext,
        None => info.default_extension()?,
    };

    ask(&info.save_title(), |dialog| {
        let mut dialog = dialog
            .add_filter(
                format!("{} File (.{primary})", primary.to_ascii_uppercase()),
                &[primary],
            )
            .add_filter(info.filter_label(), info.extensions())
            .add_filter("All Files", &["*"]);
        if let Some(name) = default_name {
            dialog = dialog.set_file_name(name);
        }
        dialog.save_file()
    })
}

/// Where a GTA VI archive's edits are written: always a new archive, offered as `default_name`.
pub fn save_gta6_archive_copy(default_name: &str) -> Option<PathBuf> {
    let title = tr!(
        "Save the edits to a new archive: a patched copy where every edit fits its entry's chunks, else a rebuilt archive, which the game refuses"
    );
    ask(title, |dialog| {
        dialog
            .add_filter("RPF archive", &["rpf"])
            .set_file_name(default_name)
            .save_file()
    })
}

/// Where a whole pack is written as one package: a file called for the pack, with the package's extension.
pub fn save_pack_package(pack: &str) -> Option<PathBuf> {
    let ext = or_wardrobe::pack_package::EXTENSION;
    ask("Export Pack As A Package", |dialog| {
        dialog
            .add_filter("OpenRage pack package", &[ext])
            .set_file_name(format!("{pack}.{ext}"))
            .save_file()
    })
}

/// A pack package to unpack into the project.
pub fn open_pack_package() -> Option<PathBuf> {
    let ext = or_wardrobe::pack_package::EXTENSION;
    ask("Unpack A Pack Package", |dialog| {
        dialog
            .add_filter("OpenRage pack package", &[ext])
            .pick_file()
    })
}

pub fn open_project_file() -> Option<PathBuf> {
    ask("Open Project", |dialog| {
        dialog
            .add_filter("OpenRage Project", &["openrage-proj"])
            .add_filter("All Files", &["*"])
            .pick_file()
    })
}

/// Pick a singleplayer DLC, an `.oiv` package or a grzyClothTool project to reopen as a project.
pub fn pick_built_pack() -> Option<PathBuf> {
    ask("Import Built Pack", |dialog| {
        dialog
            .add_filter("Built pack", &["rpf", "oiv", "gctproject"])
            .add_filter("All Files", &["*"])
            .pick_file()
    })
}

pub fn save_project_file(default_name: Option<&str>) -> Option<PathBuf> {
    ask("Save Project", |dialog| {
        let mut dialog = dialog.add_filter("OpenRage Project", &["openrage-proj"]);
        if let Some(name) = default_name {
            dialog = dialog.set_file_name(name);
        }
        dialog.save_file()
    })
}

pub fn pick_gta5_exe() -> Option<PathBuf> {
    ask("Select gta5.exe", |dialog| {
        dialog
            .add_filter("Executable", &["exe"])
            .add_filter("All Files", &["*"])
            .pick_file()
    })
}

pub fn pick_game_install_folder(title: &str) -> Option<PathBuf> {
    ask(title, rfd::FileDialog::pick_folder)
}

/// Pick an image to encode into a texture.
pub fn pick_source_image(title: &str) -> Option<PathBuf> {
    use or_io::authoring::texture_import::SourceFormat;

    ask(title, |dialog| {
        let mut dialog = dialog.add_filter("Images", &SourceFormat::EXTENSIONS);
        for format in SourceFormat::ALL {
            let extensions: Vec<&str> = SourceFormat::EXTENSIONS
                .iter()
                .copied()
                .filter(|e| SourceFormat::from_extension(e) == Some(format))
                .collect();
            dialog = dialog.add_filter(format.label(), &extensions);
        }
        dialog.add_filter("All Files", &["*"]).pick_file()
    })
}

/// Pick a `.glb` or `.gltf` to import over a model.
pub fn pick_mesh() -> Option<PathBuf> {
    ask("Replace from glTF", |dialog| {
        dialog.add_filter("glTF", &["glb", "gltf"]).pick_file()
    })
}

/// Pick a `.glb`, `.gltf` or `.fbx` to put into the geometry of a drawable source.
pub fn pick_model() -> Option<PathBuf> {
    ask("Replace geometry", |dialog| {
        dialog
            .add_filter("glTF, GLB or FBX", &["glb", "gltf", "fbx"])
            .pick_file()
    })
}

/// Pick a folder, under `title`.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    ask(title, rfd::FileDialog::pick_folder)
}

/// Pick a `.ytd` to consult for a drawable's textures.
pub fn pick_texture_dictionary() -> Option<PathBuf> {
    ask("Attach Texture Dictionary", |dialog| {
        dialog
            .add_filter("Texture Dictionary", &["ytd"])
            .pick_file()
    })
}

pub fn pick_open_folder() -> Option<PathBuf> {
    ask("Open Folder", rfd::FileDialog::pick_folder)
}

/// The folder a `.src` member that moved is at now.
pub fn pick_source_folder() -> Option<PathBuf> {
    ask("Locate source folder", rfd::FileDialog::pick_folder)
}

/// The file an export of the archive entry `name` writes, offered under its own name.
pub fn pick_export_path(name: &str) -> Option<PathBuf> {
    pick_file_named(&format!("Export {name}"), name)
}

/// The file a conversion of `name` writes, offered under its own name.
pub fn pick_conversion_path(name: &str) -> Option<PathBuf> {
    pick_file_named(&format!("Convert {name}"), name)
}

fn pick_file_named(title: &str, name: &str) -> Option<PathBuf> {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    ask(title, |dialog| {
        let mut dialog = dialog.set_file_name(name);
        if !ext.is_empty() {
            dialog = dialog.add_filter(format!("{} file", ext.to_ascii_uppercase()), &[ext]);
        }
        dialog.add_filter("All Files", &["*"]).save_file()
    })
}

/// Where an extraction writes.
pub fn pick_extract_destination() -> Option<PathBuf> {
    ask("Extract Into Folder", rfd::FileDialog::pick_folder)
}

/// Where a conversion of several files writes.
pub fn pick_conversion_folder() -> Option<PathBuf> {
    ask("Convert Into Folder", rfd::FileDialog::pick_folder)
}

/// The folder a DLC pack's own folder is written into.
pub fn pick_dlc_pack_destination() -> Option<PathBuf> {
    ask("Create DLC Pack Into Folder", rfd::FileDialog::pick_folder)
}

pub fn pick_magic_dat() -> Option<PathBuf> {
    ask("Select your key snapshot (magic.dat)", |dialog| {
        dialog
            .add_filter("Key snapshot", &["dat"])
            .add_filter("All Files", &["*"])
            .pick_file()
    })
}

/// Where a template is written: the folder of saved templates first, which the New project dialog lists.
pub fn save_template_file(default_name: &str) -> Option<PathBuf> {
    ask("Save template", |dialog| {
        let mut dialog = dialog
            .add_filter("OpenRage template", &[or_project::template::EXTENSION])
            .set_file_name(default_name);
        if let Some(folder) = crate::state::front_door::new_project::templates_folder() {
            let _ = std::fs::create_dir_all(&folder);
            dialog = dialog.set_directory(folder);
        }
        dialog.save_file()
    })
}

/// The `plugin.json` or `plugin.wasm` of a plugin to install.
pub fn pick_plugin_file() -> Option<PathBuf> {
    ask("Install plugin from a file", |dialog| {
        dialog
            .add_filter("Plugin", &["json", "wasm"])
            .add_filter("All Files", &["*"])
            .pick_file()
    })
}

/// A template file somebody shared.
pub fn open_template_file() -> Option<PathBuf> {
    ask("Open template", |dialog| {
        dialog
            .add_filter("OpenRage template", &[or_project::template::EXTENSION])
            .add_filter("All Files", &["*"])
            .pick_file()
    })
}
