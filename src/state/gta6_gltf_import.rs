//! A scanned GTA VI drawable's geometries replaced from a glTF in the app, as `rpf scan-import`
//! replaces them: worked out on a worker over the body read again, with the document's earlier
//! imports made first; its report waits on Replace, then it is one step of undo. A save makes each
//! import again over the body read again, after the document's other edits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_interchange::{ImportReport, Scene};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Role, Rpf9Sections};
use or_io::gta6::headerless::{
    read_dictionary, read_drawable, read_drawable_with_bound, Drawable, Entry, Model, Resource,
};

use super::document::Document;
use super::edit_history::{Change, Swap};
use super::gta6_edit::EditSections;
use super::gta6_resource_save::EditedSave;
use super::{AppState, DocPayload, DocumentId};

/// The most bytes an import reads of a body again.
const BODY_LIMIT: u64 = 1 << 30;

/// One import made to a document: the dictionary entry it replaces, `None` for the body's own
/// drawable or its fragment's, the mesh it was read from, and what it was asked.
type Held = (Option<u32>, Arc<Scene>, Choice);

/// The mesh a worker read and its choice, kept for the save.
type ReadScene = (Arc<Scene>, Choice);

/// What an import is asked, as `model replace` takes `--fill-missing` and `--lods replaced`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Choice {
    /// A channel the mesh omits at another vertex count takes its stated default.
    pub(super) fill: bool,
    /// A lower level the mesh names none of takes its level-0 mesh where the shader matches.
    pub(super) lower_lods: bool,
}

/// The imports made to each scanned body, by its document's path, oldest first.
// ponytail: one map for the whole app, as the scan's skeletons are; an undo swaps a path's list.
static IMPORTS: Mutex<BTreeMap<PathBuf, Vec<Held>>> = Mutex::new(BTreeMap::new());

fn imports() -> MutexGuard<'static, BTreeMap<PathBuf, Vec<Held>>> {
    IMPORTS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Forget the imports of the body at `path`, which opens anew with none.
pub(super) fn forget(path: &Path) {
    imports().remove(path);
}

/// What a worker needs to replace a scanned body's geometry from a glTF, and the mesh it read.
#[derive(Clone)]
pub(crate) struct Gta6Import {
    archive: PathBuf,
    offset: u64,
    code: Option<u32>,
    /// The imports the document already holds, made first.
    earlier: Vec<Held>,
    /// The mesh the worker read and its choice, which Replace keeps for the save.
    read: Arc<Mutex<Option<ReadScene>>>,
}

impl Gta6Import {
    /// The import into the drawable `doc` shows; `None` for any document but a scanned GTA VI body.
    pub(super) fn of(state: &AppState, doc: DocumentId) -> Option<Result<Self, String>> {
        let document = state.document(doc)?;
        let DocPayload::Gta6Drawable(resource) = &document.payload else {
            return None;
        };
        let selection = state.selection.selected(doc);
        let Some(code) = super::gta6_edit::drawable_code(resource, selection) else {
            return Some(Err(only_a_drawable()));
        };
        let path = document.path.as_deref();
        let Some((archive, offset)) = path.and_then(super::rpf9_scan::scanned_body) else {
            let why = tr!(
                "This body has no file to be read again from, so no glTF replaces its geometry."
            );
            return Some(Err(why.to_string()));
        };
        let earlier = path.and_then(|p| imports().get(p).cloned());
        let earlier = earlier.unwrap_or_default();
        Some(Ok(Self {
            archive: archive.to_path_buf(),
            offset,
            code,
            earlier,
            read: Arc::default(),
        }))
    }

    /// The body with the document's earlier imports and the glTF at `mesh` made to it, read back
    /// as the resource the document shows, and what each geometry took, as the report says it;
    /// `None` once `cancel` stops it between two replacements.
    pub(super) fn imported(
        &self,
        mesh: &Path,
        choice: Choice,
        cancel: &or_job::Cancel,
    ) -> Option<Result<(Box<DocPayload>, ImportReport), String>> {
        let made = self.made(mesh, choice, cancel);
        (!cancel.is_set()).then_some(made)
    }

    fn made(
        &self,
        mesh: &Path,
        choice: Choice,
        cancel: &or_job::Cancel,
    ) -> Result<(Box<DocPayload>, ImportReport), String> {
        let named = super::jobs::file_name(mesh);
        let source = std::fs::read(mesh)
            .map_err(|e| tr!("{mesh} did not read: {e}", e = e, mesh = named))?;
        let scene = or_interchange::gltf::read::load(mesh, &source).map_err(|e| {
            tr!(
                "{mesh} is not a mesh this can read: {e}",
                e = e,
                mesh = named
            )
        })?;
        let (mut sections, role) = body(&self.archive, self.offset)?;
        for (code, earlier, asked) in &self.earlier {
            if cancel.is_set() {
                return Err(String::new());
            }
            sections = or_cli::replace_gta6_model(&sections, role, earlier, &how(*code, *asked))?.0;
        }
        let (written, mut lines) =
            or_cli::replace_gta6_model(&sections, role, &scene, &how(self.code, choice))?;
        let dropped = scene.dropped.iter();
        lines.extend(dropped.map(|held| tr!("not carried: {held}", held = held)));
        let checks = tr!("This checks the body's bytes. It does not test that GTA VI loads them.");
        lines.push(checks.to_string());
        let resource = resource_of(&written, role)?;
        *self.read.lock().unwrap_or_else(|p| p.into_inner()) = Some((Arc::new(scene), choice));
        let report = ImportReport {
            splits: lines,
            ..ImportReport::default()
        };
        Ok((
            Box::new(DocPayload::Gta6Drawable(Arc::new(resource))),
            report,
        ))
    }
}

/// The body at `offset` of `archive` read again, as sections, and its role.
fn body(archive: &Path, offset: u64) -> Result<(Rpf9Sections, Rpf9Role), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let (find, bytes) = read_rpf9_body(&file, offset, BODY_LIMIT).map_err(|e| e.to_string())?;
    let sections = Rpf9Sections::of(&find, &bytes).ok_or_else(|| {
        tr!("The body is not a resource, so no edit of it is written.").to_string()
    })?;
    let role = match find.kind {
        Rpf9BodyKind::Zlib(run) => run.signature.and_then(|s| s.role()),
        _ => None,
    };
    let role = role.ok_or_else(|| tr!("The body's role is not known.").to_string())?;
    Ok((sections, role))
}

/// The resource the document of a body of `role` shows, read from `s`.
fn resource_of(s: &Rpf9Sections, role: Rpf9Role) -> Result<Resource, String> {
    let (v, p) = (&s.virtual_section, Some(s.physical.as_slice()));
    let read = match role {
        Rpf9Role::Drawable => read_drawable(v, p).map(Resource::Drawable),
        Rpf9Role::DrawableWithBound => {
            read_drawable_with_bound(v, p).map(Resource::DrawableWithBound)
        }
        Rpf9Role::Dictionary => read_dictionary(v, p).map(Resource::Dictionary),
        _ => return Err(only_a_drawable()),
    };
    read.map_err(|e| e.to_string())
}

/// How the app replaces drawable `code`'s meshes as `choice` asks.
fn how(code: Option<u32>, choice: Choice) -> or_cli::ModelReplace {
    or_cli::ModelReplace {
        entry: code,
        fill_missing: choice.fill,
        lods: match choice.lower_lods {
            true => or_cli::ModelLods::Replaced,
            false => or_cli::ModelLods::Left,
        },
    }
}

fn only_a_drawable() -> String {
    tr!("Only a GTA VI drawable, fragment or dictionary entry takes its geometry from a glTF.")
        .to_string()
}

/// The drawable of `resource` that `code` names: its own or its fragment's for `None`, a
/// dictionary's entry otherwise.
fn drawable_in(resource: &mut Resource, code: Option<u32>) -> Option<&mut Drawable> {
    match (resource, code) {
        (Resource::Drawable(d), None) => Some(d),
        (Resource::DrawableWithBound(r), None) => Some(&mut r.drawable),
        (Resource::Dictionary(dict), Some(code)) => {
            (dict.entries.iter_mut()).find_map(|(c, e)| match e {
                Ok(Entry::Drawable(d)) if *c == code => Some(&mut **d),
                _ => None,
            })
        }
        _ => None,
    }
}

/// That drawable of a document's payload, while no other holder of the body is reading it.
fn drawable_mut(payload: &mut DocPayload, code: Option<u32>) -> Option<&mut Drawable> {
    let DocPayload::Gta6Drawable(resource) = payload else {
        return None;
    };
    drawable_in(Arc::get_mut(resource)?, code)
}

/// A step's list of imports of the body at `path`: the one the map does not hold now.
struct Imports {
    path: PathBuf,
    other: Vec<Held>,
}

impl Swap for Imports {
    fn swap(&mut self, _: &mut DocPayload) -> bool {
        let mut held = imports();
        let now = held.entry(self.path.clone()).or_default();
        std::mem::swap(now, &mut self.other);
        true
    }
}

impl AppState {
    /// The answered import's geometry into `doc`, from `payload`, the body as it read back: its
    /// levels and box in place of the drawable's, as one step of undo, the rest of it as edited.
    pub(super) fn apply_gta6_import(
        &mut self,
        doc: DocumentId,
        mesh: &Path,
        import: &Gta6Import,
        payload: DocPayload,
    ) {
        let scene = import
            .read
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let DocPayload::Gta6Drawable(read) = payload else {
            return;
        };
        let mut read = Arc::try_unwrap(read).ok();
        let fresh = read.as_mut().and_then(|r| drawable_in(r, import.code));
        let (Some((scene, choice)), Some(fresh)) = (scene, fresh) else {
            return self.show_error_toast(tr!(
                "The model changed while the glTF was read: replace it again"
            ));
        };
        let levels: [Option<Vec<Model>>; 4] = std::mem::take(&mut fresh.levels);
        let (box_min, box_max) = (fresh.box_min, fresh.box_max);
        let code = import.code;
        let Some(document) = self.document_mut(doc) else {
            return self.show_error_toast(tr!("That model is closed"));
        };
        let Some(path) = document.path.clone() else {
            return;
        };
        let Some(held) = drawable_mut(&mut document.payload, code) else {
            return self.show_error_toast(tr!(
                "The material is not edited while a task reads the body; try again once it ends."
            ));
        };
        let before = std::mem::replace(&mut held.levels, levels);
        let min = std::mem::replace(&mut held.box_min, box_min);
        let max = std::mem::replace(&mut held.box_max, box_max);
        let mut list = imports().get(&path).cloned().unwrap_or_default();
        list.push((code, scene, choice));
        let other = imports().insert(path.clone(), list).unwrap_or_default();
        let mut change = Change::part("gta6 gltf import", before, move |p| {
            drawable_mut(p, code).map(|d| &mut d.levels)
        })
        .and_part(min, move |p| drawable_mut(p, code).map(|d| &mut d.box_min))
        .and_part(max, move |p| drawable_mut(p, code).map(|d| &mut d.box_max));
        if let Change::Part { parts, .. } = &mut change {
            parts.push(Box::new(Imports { path, other }));
        }
        let name = super::jobs::file_name(mesh);
        self.record(doc, tr!("Replace from {mesh}", mesh = name), change);
        // An import is a step of its own, never folded into the next.
        if let Some(document) = self.document_mut(doc) {
            document.history.seal();
        }
        self.show_toast(tr!(
            "Replaced from {mesh}: save to keep it, or undo it",
            mesh = name
        ));
    }
}

/// The GTA VI drawable row's `edited_sections`: every other edit of the document, then each glTF
/// import it holds made again over the body, oldest first.
pub(super) fn edited_save(document: &Document) -> Option<Result<EditedSave, String>> {
    let save = super::gta6_embedded_edit::edited_save(document)?;
    let held = (document.path.as_deref()).and_then(|p| imports().get(p).cloned());
    let Some(held) = held.filter(|h| !h.is_empty()) else {
        return Some(save);
    };
    Some(save.map(|save| match save {
        EditedSave::Body {
            archive,
            offset,
            edit,
        } => {
            let edit: Arc<EditSections> = Arc::new(move |role, sections| {
                edit(role, sections)?;
                let role = role.ok_or_else(|| tr!("The body's role is not known.").to_string())?;
                for (code, scene, choice) in &held {
                    *sections =
                        or_cli::replace_gta6_model(sections, role, scene, &how(*code, *choice))?.0;
                }
                Ok(())
            });
            EditedSave::Body {
                archive,
                offset,
                edit,
            }
        }
        other => other,
    }))
}

#[cfg(test)]
#[path = "gta6_gltf_import_tests.rs"]
mod tests;
