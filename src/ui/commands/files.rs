//! The commands a row of Files or of the Files view, a search's result and a
//! document's tab offer, each acting on what was clicked, else on what the
//! focused panel has selected.

use std::path::PathBuf;

use or_io::container::rpf::BodyForm;

use super::table::{not_busy, Action, On, Spec, BASE};
use super::target::{self, Hits, Target};
use crate::platform::file_dialogs;
use crate::state::explorer::Place;
use crate::state::extract_job::TypeFilter;
use crate::state::files_view::name_hash;
use crate::state::gltf::Model;
use crate::state::navigation::{Arrival, Location};
use crate::state::{
    AppState, DocPayload, DocumentId, DocumentKind, RpfNodeBacking, RpfNodeId, RpfSessionNode,
};
use crate::ui::panels::registry::ids;
use crate::ui::panels::{hierarchy, search};
use crate::ui::shortcuts::ShortcutMods;

const GONE: &str = tr_key!("That row is no longer there");
const ONE_ROW: &str = tr_key!("This acts on one row: click it on its own first");
const NO_ROW: &str = tr_key!("Select a file or folder in Files or in the centre first");
const NOTHING_TO_EXTRACT: &str =
    tr_key!("Select the files to extract in Files, in the centre or in a search's results first");

pub(super) const OPEN: Spec = Spec {
    action: Action::OpenSelection,
    id: "open_selection",
    label: tr_key!("Open"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Looks at the file where it is, or goes into the folder or archive. Changes nothing"
    ),
    on: Some(On {
        focused: target::selection,
        enabled: can_open,
        label: None,
        run: open,
    }),
    ..BASE
};

pub(super) const IMPORT_INTO_PROJECT: Spec = Spec {
    action: Action::ImportIntoProject,
    id: "import_into_project",
    label: tr_key!("Import into project"),
    category: tr_key!("Files"),
    hover: tr_key!("Turns the file into a source you can edit in the open project. The original, and the archive it is in, are not changed. A folder on disk is walked file by file, and an archive found in it is listed in the report and left out: import the files in it instead"),
    on: Some(On {
        focused: target::selection,
        enabled: |state, target| match target {
            Target::Rows { doc, rows, .. } => {
                state.rows_cannot_be_imported(*doc, rows).map_or(Ok(()), Err)
            }
            _ => Err(crate::i18n::tr(NO_ROW)),
        },
        label: None,
        run: |state, _, target| {
            if let Target::Rows { doc, rows, .. } = target {
                state.import_rows(*doc, rows);
            }
        },
    }),
    ..BASE
};

pub(super) const REVEAL_IN_TREE: Spec = Spec {
    action: Action::RevealInTree,
    id: "reveal_in_tree",
    label: tr_key!("Reveal in tree"),
    category: tr_key!("Files"),
    hover: tr_key!("Select the result's row in Files, opening every archive on its way"),
    on: Some(On {
        focused: target::selection,
        enabled: |_, target| match target {
            Target::Hits { .. } => Ok(()),
            _ => Err(tr!("Only a search's result is revealed in the tree")),
        },
        label: None,
        run: |state, _, target| {
            if let Target::Hits { list, rows } = target {
                walk(state, *list, first(rows), Arrival::Reveal);
                search::show_tree(state);
            }
        },
    }),
    ..BASE
};

pub(super) const REVEAL_ON_DISK: Spec = Spec {
    action: Action::RevealOnDisk,
    id: "reveal_on_disk",
    // ⚠ One name on every system, as the Project workspace's buttons say it: a person has one file manager and is not told which.
    label: tr_key!("Reveal in the file manager"),
    category: tr_key!("Files"),
    hover: tr_key!("Show the file in the file manager, selected"),
    on: Some(On {
        focused: target::selection_or_document,
        enabled: |state, target| on_disk(state, target).map(|_| ()),
        label: None,
        run: |state, _, target| {
            if let Ok(path) = on_disk(state, target) {
                crate::platform::shell::reveal(&path);
            }
        },
    }),
    ..BASE
};

pub(super) const EXTRACT: Spec = Spec {
    action: Action::Extract,
    id: "extract",
    label: tr_key!("Extract…"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Write each file into a folder you pick as other tools open it: decompressed, \
            and a resource with its header put back"
    ),
    cli: Some(|state| {
        let game = state.prefs.active_game_root().map_or_else(
            || "<install>".to_string(),
            |root| format!("\"{}\"", root.display()),
        );
        format!("openrage-cli bulk --game {game} --glob <pattern> --out <dir>")
    }),
    on: Some(On {
        focused: target::selection,
        enabled: |_, target| match target {
            Target::Rows { rows, .. } if !rows.is_empty() => Ok(()),
            Target::Hits { .. } => Ok(()),
            _ => Err(crate::i18n::tr(NOTHING_TO_EXTRACT)),
        },
        label: None,
        run: extract,
    }),
    ..BASE
};

pub(super) const EXPORT_AS: Spec = Spec {
    action: Action::ExportAs,
    id: "export_as",
    label: tr_key!("Export as…"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Write the file where and as what you name it, as other tools open it: \
            decompressed, and a resource with its header put back"
    ),
    on: Some(On {
        focused: target::selection,
        enabled: |state, target| match row(state, target)? {
            (_, node) if node.is_dir() => Err(tr!("A folder is written out with Extract…")),
            _ => Ok(()),
        },
        label: None,
        run: |state, _, target| {
            let Ok((doc, node)) = row(state, target) else {
                return;
            };
            let id = node.id;
            if let Some(path) = file_dialogs::pick_export_path(&node.display_name()) {
                state.export_rpf_node_as(doc, id, path, None);
            }
        },
    }),
    ..BASE
};

pub(super) const CONVERT: Spec = Spec {
    action: Action::Convert,
    id: "convert",
    label: tr_key!("Convert edition…"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Show what the other edition has no field for, then write the file as it lays \
            it out; a whole folder converts with openrage-cli convert retarget"
    ),
    cli: Some(|_| "openrage-cli convert retarget <dir> --out <dir>".to_string()),
    on: Some(On {
        focused: selection_or_convertible_document,
        enabled: |state, target| converts_to(state, target).map(|_| ()),
        label: Some(|state, target| match converts_to(state, target) {
            Ok(Some(to)) => tr!(
                "Convert to {edition_label}…",
                edition_label = to.edition_label()
            ),
            _ => tr!("Convert edition…").to_string(),
        }),
        run: |state, _, target| match target {
            Target::Rows { doc, rows, .. } => state.convert_rows(*doc, rows.clone()),
            &Target::Document(doc) => state.convert_document(doc),
            _ => {}
        },
    }),
    ..BASE
};

pub(super) const EXPORT_GLTF: Spec = Spec {
    action: Action::ExportGltf,
    id: "export_gltf",
    label: tr_key!("Export to glTF…"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Show what glTF carries of the model and what it leaves out, then write a .glb, or \
            a .gltf beside a .bin and its textures, as openrage-cli mesh export does"
    ),
    cli: Some(|state| {
        format!(
            "openrage-cli mesh export {} <output.glb|.gltf>",
            path_in_front(state, "<input.ydr|.ydd|.yft>")
        )
    }),
    on: Some(On {
        focused: target::selection_or_document,
        enabled: |state, target| state.exports_gltf(model_of(state, target)?).map(|_| ()),
        label: None,
        run: |state, _, target| {
            if let Ok(model) = model_of(state, target) {
                state.export_gltf(model);
            }
        },
    }),
    ..BASE
};

pub(super) const IMPORT_GLTF: Spec = Spec {
    action: Action::ImportGltf,
    id: "import_gltf",
    label: tr_key!("Replace from glTF…"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Read a .glb or .gltf, show what it replaces or why it cannot, then change the open \
            model as one step to undo or save, as openrage-cli mesh import does"
    ),
    cli: Some(|state| {
        format!(
            "openrage-cli mesh import {} <mesh.glb|.gltf|.fbx> <output>",
            path_in_front(state, "<input.ydr|.ydd|.yft>")
        )
    }),
    on: Some(On {
        focused: target::document,
        enabled: |state, target| state.gltf_import_into(model_of(state, target)?).map(|_| ()),
        label: None,
        run: |state, _, target| {
            let model = model_of(state, target);
            let Ok(doc) = model.and_then(|model| state.gltf_import_into(model)) else {
                return;
            };
            if let Some(mesh) = file_dialogs::pick_mesh() {
                state.import_gltf(doc, mesh);
            }
        },
    }),
    ..BASE
};

pub(super) const RENAME: Spec = Spec {
    action: Action::Rename,
    id: "rename",
    label: tr_key!("Rename"),
    category: tr_key!("Files"),
    defaults: &[(ShortcutMods::none(), egui::Key::F2)],
    hover: tr_key!(
        "Edit the name where the row is, or the name of the garment selected in the Wardrobe"
    ),
    on: Some(On {
        focused: target::selection,
        enabled: |state, target| match target {
            Target::Garments(garments) => super::wardrobe::can_rename_garment(state, garments),
            _ => row(state, target).map(|_| ()),
        },
        label: None,
        run: rename,
    }),
    ..BASE
};

pub(super) const NEW_FOLDER: Spec = Spec {
    action: Action::NewFolder,
    id: "new_folder",
    label: tr_key!("New folder"),
    category: tr_key!("Files"),
    hover: tr_key!("Make a folder here and edit its name in place"),
    on: Some(On {
        focused: target::selection,
        enabled: adds,
        label: None,
        run: |state, _, target| {
            if let &Target::Rows {
                doc, into, centre, ..
            } = target
            {
                hierarchy::new_folder(state, doc, into, centre);
            }
        },
    }),
    ..BASE
};

pub(super) const IMPORT_FILE: Spec = Spec {
    action: Action::ImportFile,
    id: "import_file",
    label: tr_key!("Copy into archive…"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Copy a file from disk into this folder or archive; an archive takes it when it is saved"
    ),
    on: Some(On {
        focused: target::selection,
        enabled: adds,
        label: None,
        run: |state, _, target| {
            let &Target::Rows { doc, into, .. } = target else {
                return;
            };
            let Some(file) = file_dialogs::open_any_file() else {
                return;
            };
            match state.rpf_import_file_to_folder(doc, into, file.clone()) {
                Ok(_) => state.show_toast(tr!("Copied {file} in", file = file.display())),
                Err(e) => state.show_error_toast(e),
            }
        },
    }),
    ..BASE
};

pub(super) const COPY_PATH: Spec = Spec {
    action: Action::CopyPath,
    id: "copy_path",
    label: tr_key!("Copy path"),
    category: tr_key!("Files"),
    hover: tr_key!("Put the path on the clipboard, one line for each"),
    on: Some(On {
        focused: target::selection_or_document,
        enabled: |state, target| paths(state, target).map(|_| ()),
        label: None,
        run: |state, ctx, target| {
            if let Ok(paths) = paths(state, target) {
                ctx.copy_text(paths.join("\n"));
            }
        },
    }),
    ..BASE
};

pub(super) const COPY_NAME: Spec = Spec {
    action: Action::CopyName,
    id: "copy_name",
    label: tr_key!("Copy name"),
    category: tr_key!("Files"),
    on: Some(On {
        focused: target::selection,
        enabled: |state, target| names(state, target).map(|_| ()),
        label: None,
        run: |state, ctx, target| {
            if let Ok(names) = names(state, target) {
                ctx.copy_text(names.join("\n"));
            }
        },
    }),
    ..BASE
};

pub(super) const COPY_HASH: Spec = Spec {
    action: Action::CopyHash,
    id: "copy_hash",
    label: tr_key!("Copy hash"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "The hash RAGE knows the name by, its extension left off, as a metadata file \
            or a script names it"
    ),
    on: Some(On {
        focused: target::selection,
        enabled: |state, target| row(state, target).map(|_| ()),
        label: Some(|state, target| match row(state, target) {
            Ok((_, node)) => tr!(
                "Copy hash  {hash}",
                hash = format!("0x{:08X}", name_hash(&node.name))
            ),
            Err(_) => tr!("Copy hash").to_string(),
        }),
        run: |state, ctx, target| {
            if let Ok((_, node)) = row(state, target) {
                ctx.copy_text(format!("0x{:08X}", name_hash(&node.name)));
            }
        },
    }),
    ..BASE
};

pub(super) const PROPERTIES: Spec = Spec {
    action: Action::Properties,
    id: "properties",
    label: tr_key!("Properties"),
    category: tr_key!("Files"),
    hover: tr_key!("Every column of the row, its hash and its path"),
    on: Some(On {
        focused: target::selection,
        enabled: |state, target| row_or_root(state, target).map(|_| ()),
        label: None,
        run: |state, _, target| {
            if let Ok((doc, node)) = row_or_root(state, target) {
                let shown = Some((doc, node.id));
                state.panels.get::<hierarchy::FilesViewUi>().properties = shown;
                let on_screen = state.active_workspace;
                (state.workspace_docks)
                    .reveal(on_screen, crate::ui::panels::registry::ids::DETAILS);
            }
        },
    }),
    ..BASE
};

pub(super) const ADD_FAVOURITE: Spec = Spec {
    action: Action::AddToFavourites,
    id: "add_to_favourites",
    label: tr_key!("Add to favourites"),
    category: tr_key!("Files"),
    hover: tr_key!("Keep it in Places, to come back to"),
    on: Some(On {
        focused: target::selection_or_document,
        enabled: |state, target| match place(state, target)? {
            place if state.prefs.explorer.is_favourite(&place) => Err(tr!("Already a favourite")),
            _ => Ok(()),
        },
        label: None,
        run: toggle_favourite,
    }),
    ..BASE
};

pub(super) const REMOVE_FAVOURITE: Spec = Spec {
    action: Action::RemoveFromFavourites,
    id: "remove_from_favourites",
    label: tr_key!("Remove from favourites"),
    category: tr_key!("Files"),
    on: Some(On {
        focused: target::selection_or_document,
        enabled: |state, target| match place(state, target)? {
            place if state.prefs.explorer.is_favourite(&place) => Ok(()),
            _ => Err(tr!("Not a favourite")),
        },
        label: None,
        run: toggle_favourite,
    }),
    ..BASE
};

pub(super) const CREATE_DLC_PACK: Spec = Spec {
    action: Action::CreateDlcPack,
    id: "create_dlc_pack",
    label: tr_key!("Create DLC pack…"),
    category: tr_key!("Files"),
    hover: tr_key!(
        "Pack the folder into a DLC pack as openrage-cli dlc create does, after showing \
            what goes in, what is refused and the line dlclist.xml needs"
    ),
    cli: Some(|state| {
        format!(
            "openrage-cli dlc create {} <out_dir> --name <name>",
            path_in_front(state, "<src_dir>")
        )
    }),
    on: Some(On {
        focused: target::selection_or_folder_opened,
        enabled: |state, target| pack_source(state, target).map(|_| ()),
        label: None,
        run: |state, _, target| {
            if let Ok(folder) = pack_source(state, target) {
                state.ask_dlc_pack(folder);
            }
        },
    }),
    ..BASE
};

/// The folder on disk the one row `target` names, which a DLC pack is built from; an Open Folder document's own row names the folder opened.
/// The path on disk of the document in front, quoted as one word of a command line; `placeholder` when it has none, as an entry of an archive has.
fn path_in_front(state: &AppState, placeholder: &str) -> String {
    let on_disk = (state.active_document())
        .filter(|doc| doc.rpf_entry_binding.is_none())
        .and_then(|doc| doc.path.as_ref());
    match on_disk {
        Some(path) => format!("\"{}\"", path.display()),
        None => placeholder.to_string(),
    }
}

fn pack_source(state: &AppState, target: &Target) -> Result<PathBuf, &'static str> {
    const NOT_A_FOLDER: &str = tr_key!("A DLC pack is built from a folder on disk");
    if let &Target::Document(doc) = target {
        return match state.document(doc) {
            Some(doc) if matches!(doc.kind, DocumentKind::Folder) => {
                doc.path.clone().ok_or(crate::i18n::tr(NOT_A_FOLDER))
            }
            Some(_) => Err(crate::i18n::tr(NOT_A_FOLDER)),
            None => Err(crate::i18n::tr(GONE)),
        };
    }
    match &row(state, target)?.1.backing {
        RpfNodeBacking::DiskFolder { fs_path, .. } => Ok(fs_path.clone()),
        _ => Err(crate::i18n::tr(NOT_A_FOLDER)),
    }
}

pub(super) const CLOSE_DOCUMENT: On = On {
    focused: target::document,
    enabled: |state, target| match target {
        &Target::Document(doc) if state.document(doc).is_some() => not_busy(state),
        _ => Err(tr!("No document is open")),
    },
    label: None,
    run: |state, _, target| {
        if let &Target::Document(doc) = target {
            state.close_document(doc);
        }
    },
};

pub(super) const CLOSE_OTHERS: Spec = Spec {
    action: Action::CloseOtherTabs,
    id: "close_other_tabs",
    label: tr_key!("Close others"),
    category: tr_key!("Tabs"),
    hover: tr_key!("Close every document but this one, the install and the archives it came from"),
    on: Some(On {
        focused: target::document,
        enabled: |state, target| match target {
            &Target::Document(doc) if state.other_tabs(doc).is_empty() => {
                Err(tr!("No other document can close"))
            }
            Target::Document(_) => not_busy(state),
            _ => Err(tr!("No document is open")),
        },
        label: None,
        run: |state, _, target| {
            if let &Target::Document(doc) = target {
                state.close_other_tabs(doc);
            }
        },
    }),
    ..BASE
};

/// The one row `target` names, with its document.
pub(super) fn row<'a>(
    state: &'a AppState,
    target: &Target,
) -> Result<(DocumentId, &'a RpfSessionNode), &'static str> {
    let Target::Rows { doc, rows, .. } = target else {
        return Err(crate::i18n::tr(NO_ROW));
    };
    match rows[..] {
        [row] => node(state, *doc, row)
            .map(|node| (*doc, node))
            .ok_or(crate::i18n::tr(GONE)),
        [] => Err(crate::i18n::tr(NO_ROW)),
        _ => Err(crate::i18n::tr(ONE_ROW)),
    }
}

/// [`row`], or the root of the tree an Open Folder or archive document's own row names.
fn row_or_root<'a>(
    state: &'a AppState,
    target: &Target,
) -> Result<(DocumentId, &'a RpfSessionNode), &'static str> {
    let &Target::Document(doc) = target else {
        return row(state, target);
    };
    match state.document(doc).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => rpf
            .session
            .node(rpf.session.root_id())
            .map(|root| (doc, root))
            .ok_or(crate::i18n::tr(GONE)),
        Some(_) => Err(tr!("Only a file or folder in Files has properties")),
        None => Err(crate::i18n::tr(GONE)),
    }
}

fn node(state: &AppState, doc: DocumentId, row: RpfNodeId) -> Option<&RpfSessionNode> {
    match &state.document(doc)?.payload {
        DocPayload::Rpf(rpf) => rpf.session.node(row),
        _ => None,
    }
}

fn can_open(state: &AppState, target: &Target) -> Result<(), &'static str> {
    match target {
        Target::Hits { .. } => Ok(()),
        _ => row(state, target).map(|_| ()),
    }
}

fn open(state: &mut AppState, _: &egui::Context, target: &Target) {
    if let Target::Hits { list, rows } = target {
        walk(state, *list, first(rows), Arrival::Open);
        return;
    }
    let Ok((doc, node)) = row(state, target) else {
        return;
    };
    let (id, folder) = (node.id, node.is_dir().then_some(node.parent));
    match folder {
        Some(parent) => hierarchy::go_into(state, doc, parent.unwrap_or(id), id),
        None => state.open_rpf_entry(doc, id),
    }
}

/// The result clicked, which leads the rows.
fn first(rows: &[usize]) -> &[usize] {
    &rows[..rows.len().min(1)]
}

fn walk(state: &mut AppState, list: Hits, rows: &[usize], then: Arrival) {
    match list {
        Hits::Search => state.go_to_search_hits(rows, then),
        Hits::Inside(doc) => state.go_to_deep_find_matches(doc, rows, then),
    }
}

fn on_disk(state: &AppState, target: &Target) -> Result<PathBuf, &'static str> {
    if let &Target::Document(doc) = target {
        let path = (state.document(doc).and_then(|d| d.path.clone()))
            .ok_or(tr!("This document has no file yet"))?;
        if !path.exists() {
            return Err(tr!("This document's file is inside an archive"));
        }
        return Ok(path);
    }
    let (_, node) = row(state, target)?;
    (node.backing.fs_path())
        .map(|path| path.to_path_buf())
        .ok_or(tr!(
            "A file inside an archive has no file of its own to show"
        ))
}

fn extract(state: &mut AppState, _: &egui::Context, target: &Target) {
    let Some(dest) = file_dialogs::pick_extract_destination() else {
        return;
    };
    match target {
        Target::Rows { doc, rows, .. } => {
            let (form, filter) = (BodyForm::Decoded, TypeFilter::All);
            state.extract_rpf_nodes(*doc, rows.clone(), dest, form, filter);
        }
        Target::Hits { list, rows } => {
            let then = Arrival::Extract {
                dest,
                form: BodyForm::Decoded,
                filter: TypeFilter::All,
            };
            walk(state, *list, rows, then);
        }
        _ => {}
    }
}

/// What the focused panel has selected, else the open document: but not a folder or an archive Files lists, which is no document to convert.
fn selection_or_convertible_document(state: &AppState) -> Result<Target, &'static str> {
    let in_files = state.workspace_docks.focused(state.active_workspace) == Some(ids::FILES);
    match (target::selection(state), target::document(state)) {
        (Ok(rows), _) => Ok(rows),
        (Err(why), Ok(Target::Document(doc)))
            if in_files && state.document_conversion_to(doc).is_err() =>
        {
            Err(why)
        }
        (Err(_), document) => document,
    }
}

/// The edition the rows or the document `target` names convert to, `None` when the rows go both ways.
fn converts_to(
    state: &AppState,
    target: &Target,
) -> Result<Option<or_io::Generation>, &'static str> {
    match target {
        Target::Rows { doc, rows, .. } => state.conversion_to(*doc, rows),
        &Target::Document(doc) => state.document_conversion_to(doc).map(Some),
        _ => Err(tr!(
            "Select the files to convert in Files or in the centre first"
        )),
    }
}

/// The model the row or the document `target` names.
fn model_of(state: &AppState, target: &Target) -> Result<Model, &'static str> {
    match target {
        &Target::Document(doc) => Ok(Model::Document(doc)),
        _ => row(state, target).map(|(doc, node)| Model::Row(doc, node.id)),
    }
}

fn rename(state: &mut AppState, _: &egui::Context, target: &Target) {
    if let Target::Garments(_) = target {
        return state.ask_to_rename_garment();
    }
    let (Ok((doc, node)), &Target::Rows { centre, .. }) = (row(state, target), target) else {
        return;
    };
    let (id, name) = (node.id, node.name.clone());
    if centre {
        hierarchy::ui_state(state).start_rename_in_centre(doc, id, &name);
        state.panels.get::<hierarchy::FilesViewUi>().scroll_to_focus = true;
    } else {
        hierarchy::ui_state(state).start_rename_rpf(doc, id, &name);
    }
}

fn adds(_: &AppState, target: &Target) -> Result<(), &'static str> {
    match target {
        Target::Rows { .. } => Ok(()),
        _ => Err(tr!(
            "Select the folder to add to in Files or in the centre first"
        )),
    }
}

fn paths(state: &AppState, target: &Target) -> Result<Vec<String>, &'static str> {
    let paths: Vec<String> = match target {
        Target::Rows { doc, rows, .. } => match &state.document(*doc).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => (rows.iter())
                .filter_map(|&row| rpf.session.path_of(row))
                .map(|path| format!("/{path}"))
                .collect(),
            _ => Vec::new(),
        },
        Target::Hits { list, rows } => hit_paths(state, *list, rows),
        &Target::Document(doc) => (state.document(doc).and_then(|d| d.path.as_ref()))
            .map(|path| vec![path.display().to_string()])
            .ok_or(tr!("This document has no file yet"))?,
        _ => return Err(crate::i18n::tr(NO_ROW)),
    };
    if paths.is_empty() {
        return Err(crate::i18n::tr(GONE));
    }
    Ok(paths)
}

fn hit_paths(state: &AppState, list: Hits, rows: &[usize]) -> Vec<String> {
    match list {
        Hits::Search => (rows.iter())
            .filter_map(|&row| state.search.hits.get(row))
            .map(crate::state::search_session::hit_path)
            .collect(),
        Hits::Inside(doc) => {
            let pattern = hierarchy::ui_state_ref(state).map_or("", |ui| ui.filter.trim());
            let Some(found) = state.deep_find.found_for(doc, pattern) else {
                return Vec::new();
            };
            (rows.iter())
                .filter_map(|&row| found.matches().get(row))
                .map(|path| crate::state::vfs_session::inner_path(path).to_string())
                .collect()
        }
    }
}

fn names(state: &AppState, target: &Target) -> Result<Vec<String>, &'static str> {
    let Target::Rows { doc, rows, .. } = target else {
        return Err(crate::i18n::tr(NO_ROW));
    };
    let names: Vec<String> = (rows.iter())
        .filter_map(|&row| node(state, *doc, row))
        .map(|node| node.name.clone())
        .collect();
    if names.is_empty() {
        return Err(crate::i18n::tr(GONE));
    }
    Ok(names)
}

/// Where a row or a document is, as Places keeps it.
fn place(state: &AppState, target: &Target) -> Result<Place, &'static str> {
    let at = match target {
        &Target::Document(doc) => Location::document(doc),
        _ => {
            let (doc, node) = row(state, target)?;
            Location { doc, node: node.id }
        }
    };
    state.place_of(at).ok_or(tr!(
        "Only something opened from disk can be a favourite: this has no path to come back to"
    ))
}

fn toggle_favourite(state: &mut AppState, _: &egui::Context, target: &Target) {
    if let Ok(place) = place(state, target) {
        state.toggle_favourite(place);
    }
}
