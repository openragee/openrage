//! A model out to glTF and a glTF back over it, from the app: each is worked
//! out on a worker, and its report waits on an answer before a file is
//! written or the model changes.
//!
//! ⚠ Each goes through the call `openrage-cli mesh export|import` makes,
//! `or_cli::export_scene` and `or_cli::replace_mesh`, so the app and the
//! command cannot report a file differently.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use or_interchange::gltf::{self, Container};
use or_interchange::{ExtractOptions, ImportOptions, ImportReport, LodSelection, Scene};
use or_job::{Cancel, Step};

use super::document::{Document, DocumentId, DocumentKind, RpfNodeId};
use super::extract_job::extension_of;
use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::payload_handlers::{handler_for_payload, PayloadHandler};
use super::{AppState, Change, DocPayload, GateOutcome, RpfEntryBinding};

const ONLY: &str =
    tr_key!("Only a drawable, a drawable dictionary or a fragment goes to and from glTF");
const BUSY: &str = tr_key!("A glTF export or a replacement from glTF is already under way");
const OPEN_IT: &str =
    tr_key!("Open it first: replacing from glTF changes the open model, which you then save");
const GTA6_NO_DRAWABLE: &str =
    tr_key!("Choose one of the dictionary's drawables first: its export is that drawable");

/// A model: a file's row in Files or the Files view, or an open document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Row(DocumentId, RpfNodeId),
    Document(DocumentId),
}

/// What an export is asked for: `--entry`, `--lods all`, `--no-textures`, and `.glb` or `.gltf`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportChoice {
    pub entry: Option<usize>,
    pub every_lod: bool,
    pub textures: bool,
    pub glb: bool,
}

/// What an import is asked for: `--entry`, `--fill-missing` and `--keep-skinning`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportChoice {
    pub entry: usize,
    pub fill_missing: bool,
    pub keep_skinning: bool,
    /// `--lods replaced`: a GTA VI drawable's lower levels take the level-0 mesh.
    pub lower_lods: bool,
}

pub struct Export {
    /// The model's file name, which says its format.
    pub name: String,
    pub(super) bytes: Arc<Vec<u8>>,
    pub choice: ExportChoice,
    /// Each drawable of a `.ydd` or `.yft`, by its `--entry` number, as the export of every one names it.
    pub drawables: Vec<(usize, String)>,
    /// `None` while it is worked out.
    pub made: Option<Result<Scene, String>>,
    /// A scanned GTA VI body's drawable, exported in place of `bytes`.
    pub(super) gta6: Option<super::gta6_export::Gta6Export>,
}

pub struct Import {
    pub doc: DocumentId,
    pub name: String,
    pub mesh: PathBuf,
    /// The model as a save writes it.
    pub(super) bytes: Arc<Vec<u8>>,
    /// `None` for a scanned GTA VI body, whose import is `gta6`.
    pub(super) handler: Option<PayloadHandler>,
    pub(super) gta6: Option<super::gta6_gltf_import::Gta6Import>,
    /// The document's edits when it was read, so an answer never lands over a later one.
    pub(super) edits: u64,
    pub choice: ImportChoice,
    /// Each drawable of a `.ydd` or `.yft`, by its `--entry` number.
    pub drawables: Vec<(usize, String)>,
    /// The model as the import leaves it and what changed, `None` while it is worked out.
    pub made: Option<Result<(Box<DocPayload>, ImportReport), String>>,
}

impl Import {
    /// Whether it replaces a scanned GTA VI body's geometry, whose report is its lines alone.
    pub fn is_gta6(&self) -> bool {
        self.gta6.is_some()
    }
}

/// The export or import whose report waits on an answer.
pub enum Asked {
    Export(Export),
    Import(Import),
}

pub(super) enum Worked {
    Export(Result<or_cli::Exported, String>),
    Import(Result<(Box<DocPayload>, ImportReport), String>),
}

struct Running {
    cancel: Cancel,
    doing: String,
    receiver: Receiver<Option<Worked>>,
}

/// The one glTF export or import being worked out, and the question it fills.
#[derive(Default)]
pub struct Interchange {
    running: Option<Running>,
    pub asked: Option<Asked>,
}

impl Interchange {
    /// Run `work`, which returns `None` once it was stopped, as the one export or import.
    pub(super) fn start(
        &mut self,
        doing: String,
        work: impl FnOnce(&Cancel) -> Option<Worked> + Send + 'static,
    ) {
        let cancel = Cancel::new();
        let (tx, receiver) = channel();
        let worker = cancel.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work(&worker));
        });
        self.running = Some(Running {
            cancel,
            doing,
            receiver,
        });
    }

    /// The export or import being worked out, as the job list draws it.
    pub fn job(&self) -> Option<RunningJob> {
        let running = self.running.as_ref()?;
        Some(RunningJob {
            kind: JobKind::Gltf,
            doing: running.doing.clone(),
            fraction: None,
            detail: String::new(),
            stopping: false,
        })
    }

    /// Drop what is being worked out and its question; `true` when something was.
    ///
    /// ⚠ The library cannot stop halfway, so its worker finishes unheard.
    pub fn cancel(&mut self) -> bool {
        self.asked = None;
        let running = self.running.take();
        if let Some(running) = &running {
            running.cancel.set();
        }
        running.is_some()
    }

    fn busy(&self) -> bool {
        self.running.is_some() || self.asked.is_some()
    }
}

/// A document's file name with the extension its kind has.
pub(super) fn model_name(document: &Document) -> String {
    let file = document.file_name();
    let stem = Path::new(&file)
        .file_stem()
        .map_or_else(|| file.clone(), |s| s.to_string_lossy().into_owned());
    format!("{stem}.{}", document.kind.extension())
}

/// Each drawable `payload` holds, by its `--entry` number; empty for a `.ydr`.
fn drawables(payload: &DocPayload) -> Vec<(usize, String)> {
    use or_io::gta5::{DrawableTarget, GtaFragType, RmcDrawable};
    use or_io::Asset;

    fn listed<'a, T: DrawableTarget + 'a>(
        drawables: impl Iterator<Item = Option<&'a RmcDrawable<T>>>,
    ) -> Vec<(usize, String)> {
        (drawables.enumerate())
            .filter_map(|(entry, drawable)| Some((entry, drawable?.name.clone())))
            .collect()
    }
    // 0 is the common drawable, as `--entry` numbers a fragment's.
    fn parts<T: DrawableTarget>(fragment: &GtaFragType<T>) -> Vec<(usize, String)> {
        let common = fragment.common_drawable.as_deref().map(|d| &d.base);
        let extra = fragment.extra_drawables.iter().map(|d| Some(&d.base));
        listed(std::iter::once(common).chain(extra))
    }
    let DocPayload::Resource(asset) = payload else {
        return Vec::new();
    };
    match asset {
        Asset::DrawableDict(d) => listed(d.entries.iter().map(|e| e.as_deref().map(|e| &e.base))),
        Asset::DrawableDictGen9(d) => {
            listed(d.entries.iter().map(|e| e.as_deref().map(|e| &e.base)))
        }
        Asset::Fragment(fragment) => parts(fragment),
        Asset::FragmentGen9(fragment) => parts(fragment),
        _ => Vec::new(),
    }
}

/// What `mesh import` makes of `bytes`, the model `name`, with the glTF at `mesh` over it, read back as the document it saves as.
fn imported(
    name: &str,
    bytes: &[u8],
    mesh: &Path,
    choice: &ImportChoice,
    handler: PayloadHandler,
) -> Result<(Box<DocPayload>, ImportReport), String> {
    let source = std::fs::read(mesh)
        .map_err(|e| tr!("{mesh} did not read: {e}", e = e, mesh = file_name(mesh)))?;
    let scene = gltf::read::load(mesh, &source).map_err(|e| {
        tr!(
            "{mesh} is not a mesh this can read: {e}",
            e = e,
            mesh = file_name(mesh)
        )
    })?;
    let options = ImportOptions {
        fill_missing: choice.fill_missing,
        keep_skinning: choice.keep_skinning,
    };
    let (saved, report) = or_cli::replace_mesh(name, bytes, choice.entry, &scene, &options)?;
    Ok((Box::new((handler.parse)(Path::new(name), &saved)?), report))
}

impl AppState {
    /// The file name of the model `model` names, or why it names none.
    fn model_at(&self, model: Model) -> Result<String, &'static str> {
        let gone = tr!("That is no longer open");
        match model {
            Model::Row(doc, row) => {
                let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
                    return Err(gone);
                };
                let node = rpf.session.node(row).ok_or(gone)?;
                let ext = extension_of(&node.name);
                if node.is_dir() || !matches!(ext.as_str(), "ydr" | "ydd" | "yft") {
                    return Err(crate::i18n::tr(ONLY));
                }
                Ok(node.name.clone())
            }
            Model::Document(doc) => match self.document(doc).map(|d| (d, d.kind)) {
                Some((document, DocumentKind::Ydr | DocumentKind::Ydd | DocumentKind::Yft)) => {
                    Ok(model_name(document))
                }
                Some((document, DocumentKind::Gta6Drawable)) => {
                    let DocPayload::Gta6Drawable(body) = &document.payload else {
                        return Err(crate::i18n::tr(ONLY));
                    };
                    let chosen = self.selection.selected(doc);
                    match super::gta6_resource::drawable_of(body, chosen) {
                        Some(_) => Ok(document.file_name()),
                        None if super::gta6_resource::dictionary_drawables(body)
                            .next()
                            .is_none() =>
                        {
                            Err(crate::i18n::tr(ONLY))
                        }
                        None => Err(crate::i18n::tr(GTA6_NO_DRAWABLE)),
                    }
                }
                Some(_) => Err(crate::i18n::tr(ONLY)),
                None => Err(gone),
            },
        }
    }

    /// Whether a menu on `model` offers glTF: it is a drawable, a drawable dictionary or a fragment.
    pub fn offers_gltf(&self, model: Model) -> bool {
        self.model_at(model).is_ok()
    }

    /// Why `model` cannot go out to glTF now, or its file name.
    pub fn exports_gltf(&self, model: Model) -> Result<String, &'static str> {
        let name = self.model_at(model)?;
        if self.gltf.busy() {
            return Err(crate::i18n::tr(BUSY));
        }
        Ok(name)
    }

    /// The open document a glTF import into `model` changes, or why there is none.
    pub fn gltf_import_into(&self, model: Model) -> Result<DocumentId, &'static str> {
        self.model_at(model)?;
        let doc = match model {
            Model::Document(doc) => doc,
            Model::Row(doc, row) => self
                .open_for_row(doc, row)
                .ok_or(crate::i18n::tr(OPEN_IT))?,
        };
        self.model_at(Model::Document(doc))?;
        if self.gltf.busy() {
            return Err(crate::i18n::tr(BUSY));
        }
        Ok(doc)
    }

    /// The document open for `row` of `doc`, however it was opened.
    fn open_for_row(&self, doc: DocumentId, row: RpfNodeId) -> Option<DocumentId> {
        let binding = RpfEntryBinding {
            parent_doc_id: doc,
            entry_id: row,
        };
        let document = self.document(doc)?;
        let DocPayload::Rpf(rpf) = &document.payload else {
            return None;
        };
        let mut path = document.path.clone().unwrap_or_default();
        path.extend(rpf.session.path_of(row)?.split('/'));
        self.document_holding(&path, Some(binding))
    }

    /// `doc` as a save writes it.
    pub(super) fn saved_bytes(&mut self, doc: DocumentId) -> Result<Vec<u8>, String> {
        self.catch_up(doc);
        let document = self.document(doc).ok_or("That document is closed")?;
        let handler = handler_for_payload(&document.payload).ok_or(crate::i18n::tr(ONLY))?;
        (handler.save(&document.payload)).map_err(|why| {
            tr!(
                "{document} does not save as it stands: {why}",
                why = why,
                document = model_name(document)
            )
        })
    }

    /// Work out `model`'s glTF on a worker; its report is asked about once it ends.
    ///
    /// ⚠ A row is exported as its file holds it, an open document as a save would write it.
    pub fn export_gltf(&mut self, model: Model) {
        let name = match self.exports_gltf(model) {
            Ok(name) => name,
            Err(why) => return self.show_error_toast(why),
        };
        let gta6 = match model {
            Model::Document(doc) => super::gta6_export::Gta6Export::of(self, doc),
            Model::Row(..) => None,
        };
        let bytes = match model {
            _ if gta6.is_some() => Ok(Arc::default()),
            Model::Row(doc, row) => match self.document(doc).map(|d| &d.payload) {
                Some(DocPayload::Rpf(rpf)) => rpf
                    .session
                    .bytes_for(row)
                    .ok_or(tr!("That file did not read")),
                _ => Err(tr!("That is no longer open")),
            }
            .map_err(str::to_string),
            Model::Document(doc) => self.saved_bytes(doc).map(Arc::new),
        };
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(why) => return self.show_error_toast(why),
        };
        self.gltf.asked = Some(Asked::Export(Export {
            name,
            bytes,
            choice: ExportChoice {
                entry: None,
                every_lod: false,
                textures: true,
                glb: true,
            },
            drawables: Vec::new(),
            made: None,
            gta6,
        }));
        self.rerun_gltf();
    }

    /// Read the glTF at `mesh` over `doc` on a worker; what it replaces is asked about once it ends, and nothing changes before the answer.
    pub fn import_gltf(&mut self, doc: DocumentId, mesh: PathBuf) {
        if let Err(why) = self.gltf_import_into(Model::Document(doc)) {
            return self.show_error_toast(why);
        }
        if let Some(gta6) = super::gta6_gltf_import::Gta6Import::of(self, doc) {
            return self.import_gta6_gltf(doc, mesh, gta6);
        }
        let bytes = match self.saved_bytes(doc) {
            Ok(bytes) => bytes,
            Err(why) => return self.show_error_toast(why),
        };
        let Some(document) = self.document(doc) else {
            return;
        };
        let Some(handler) = handler_for_payload(&document.payload) else {
            return self.show_error_toast(crate::i18n::tr(ONLY));
        };
        let drawables = drawables(&document.payload);
        let entry = drawables.first().map_or(0, |(entry, _)| *entry);
        self.gltf.asked = Some(Asked::Import(Import {
            doc,
            name: model_name(document),
            mesh,
            bytes: Arc::new(bytes),
            handler: Some(handler),
            gta6: None,
            edits: document.edit_epoch(),
            choice: ImportChoice {
                entry,
                ..ImportChoice::default()
            },
            drawables,
            made: None,
        }));
        self.rerun_gltf();
    }

    /// [`Self::import_gltf`] of a scanned GTA VI body, whose geometry the worker replaces as
    /// `rpf scan-import` does.
    fn import_gta6_gltf(
        &mut self,
        doc: DocumentId,
        mesh: PathBuf,
        gta6: Result<super::gta6_gltf_import::Gta6Import, String>,
    ) {
        let gta6 = match gta6 {
            Ok(gta6) => gta6,
            Err(why) => return self.show_error_toast(why),
        };
        let Some(document) = self.document(doc) else {
            return;
        };
        self.gltf.asked = Some(Asked::Import(Import {
            doc,
            name: document.file_name(),
            mesh,
            bytes: Arc::default(),
            handler: None,
            gta6: Some(gta6),
            edits: document.edit_epoch(),
            choice: ImportChoice::default(),
            drawables: Vec::new(),
            made: None,
        }));
        self.rerun_gltf();
    }

    /// Work out the question's export or import again, for the choice it holds now.
    pub fn rerun_gltf(&mut self) {
        type Work = Box<dyn FnOnce(&Cancel) -> Option<Worked> + Send>;
        let (doing, work): (String, Work) = match &mut self.gltf.asked {
            Some(Asked::Export(asked)) => {
                asked.made = None;
                let (name, bytes) = (asked.name.clone(), Arc::clone(&asked.bytes));
                let choice = asked.choice.clone();
                let stem = Path::new(&name)
                    .file_stem()
                    .map_or_else(|| "mesh".to_string(), |s| s.to_string_lossy().into_owned());
                let doing = tr!("Exporting {name} to glTF", name = name);
                let gta6 = asked.gta6.clone();
                (
                    doing,
                    Box::new(move |_| {
                        if let Some(gta6) = gta6 {
                            let made = (gta6.scene(&stem, choice.every_lod))
                                .map(|scene| or_cli::Exported {
                                    scene,
                                    entries: Vec::new(),
                                })
                                .ok_or_else(|| crate::i18n::tr(GTA6_NO_DRAWABLE).to_string());
                            return Some(Worked::Export(made));
                        }
                        let options = ExtractOptions {
                            lods: if choice.every_lod {
                                LodSelection::All
                            } else {
                                LodSelection::Highest
                            },
                            textures: choice.textures,
                        };
                        let made =
                            or_cli::export_scene(&name, &bytes, &stem, choice.entry, &options);
                        Some(Worked::Export(made))
                    }),
                )
            }
            Some(Asked::Import(asked)) => {
                asked.made = None;
                let (name, bytes) = (asked.name.clone(), Arc::clone(&asked.bytes));
                let (mesh, choice, handler) =
                    (asked.mesh.clone(), asked.choice.clone(), asked.handler);
                let gta6 = asked.gta6.clone();
                let fill = choice.fill_missing;
                let fill = super::gta6_gltf_import::Choice {
                    fill,
                    lower_lods: choice.lower_lods,
                };
                let doing = tr!(
                    "Replacing {name} from {mesh}",
                    name = name,
                    mesh = file_name(&mesh)
                );
                (
                    doing,
                    Box::new(move |cancel| {
                        let made = match (&gta6, handler) {
                            (Some(gta6), _) => match gta6.imported(&mesh, fill, cancel) {
                                // Stopped between two of its replacements.
                                None => return None,
                                Some(made) => made,
                            },
                            (None, Some(handler)) => {
                                imported(&name, &bytes, &mesh, &choice, handler)
                            }
                            (None, None) => Err(crate::i18n::tr(ONLY).to_string()),
                        };
                        Some(Worked::Import(made))
                    }),
                )
            }
            None => return,
        };
        self.gltf.start(doing, work);
    }

    /// Where a finished export or import lands: in the question waiting on its answer.
    pub(super) fn poll_gltf(&mut self) {
        let received = match &self.gltf.running {
            Some(running) => running.receiver.try_recv(),
            None => return,
        };
        if matches!(received, Err(TryRecvError::Empty)) {
            return;
        }
        let Some(running) = self.gltf.running.take() else {
            return;
        };
        let broke = tr!("It stopped on an internal error; nothing was written").to_string();
        let (outcome, said) = match (received, &mut self.gltf.asked) {
            (Ok(None), _) => (
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished; nothing was written").to_string(),
            ),
            (Ok(Some(Worked::Export(made))), Some(Asked::Export(asked))) => {
                let made = made.map(|exported| {
                    // ⚠ Read off an export of every drawable, so choosing one leaves the others listed.
                    if asked.choice.entry.is_none() && extension_of(&asked.name) != "ydr" {
                        asked.drawables = (exported.entries.iter().copied())
                            .zip(exported.scene.objects.iter().map(|o| o.name.clone()))
                            .collect();
                    }
                    exported.scene
                });
                asked.made = Some(made);
                let said = tr!(
                    "{doing}: its report waits on Export… or Cancel",
                    doing = running.doing
                );
                (Outcome::Done(Then::Nothing), said)
            }
            (Ok(Some(Worked::Import(made))), Some(Asked::Import(asked))) => {
                asked.made = Some(made);
                let said = tr!(
                    "{doing}: its report waits on Replace or Cancel",
                    doing = running.doing
                );
                (Outcome::Done(Then::Nothing), said)
            }
            (Ok(Some(_)), _) => return,
            (Err(_), Some(Asked::Export(asked))) => {
                asked.made = Some(Err(broke.clone()));
                (Outcome::Failed, broke)
            }
            (Err(_), Some(Asked::Import(asked))) => {
                asked.made = Some(Err(broke.clone()));
                (Outcome::Failed, broke)
            }
            (Err(_), None) => (Outcome::Failed, broke),
        };
        self.jobs.record(JobKind::Gltf, outcome, said);
    }

    /// Write the answered export to `dest`: a `.glb`, or a `.gltf` with its `.bin` and textures beside it.
    ///
    /// ⚠ The gate is asked here, at the write, and so are the jobs reading a
    /// file it would create or replace, the export's or one beside it. A
    /// refusal keeps the question, so another place can be chosen; where the
    /// gate would ask about the game as shipped the write is refused instead,
    /// as Export as…'s is.
    pub fn write_gltf(&mut self, dest: PathBuf) {
        let Some(Asked::Export(Export {
            made: Some(Ok(_)), ..
        })) = &self.gltf.asked
        else {
            return;
        };
        let container = match extension_of(&dest.to_string_lossy()).as_str() {
            "glb" => Container::Glb,
            "gltf" => Container::Separate,
            _ => {
                return self.show_error_toast(tr!(
                    "A glTF export is named .glb, or .gltf for the JSON beside a .bin and its \
                     textures"
                ))
            }
        };
        if let Some(what) = self.extract_jobs.running_noun() {
            return self
                .show_error_toast(tr!("{what} is running; write once it is done", what = what));
        }
        match self.gate_permits(Some(&dest)) {
            GateOutcome::Proceed => {}
            GateOutcome::Refused => return,
            GateOutcome::Confirm(_) => {
                return self.show_warning_toast(tr!(
                    "A glTF export does not ask to write into the game as shipped: choose a \
                     place outside the install or in its mods overlay"
                ))
            }
        }
        let (Some(folder), Some(stem)) = (dest.parent(), dest.file_stem()) else {
            return;
        };
        let (folder, stem) = (folder.to_path_buf(), stem.to_string_lossy().into_owned());
        let Some(Asked::Export(Export {
            made: Some(Ok(scene)),
            ..
        })) = &self.gltf.asked
        else {
            return;
        };
        let files = gltf::write(scene, &stem, container);
        let sidecars = (files.sidecars.into_iter()).map(|(name, bytes)| (folder.join(name), bytes));
        let writes: Vec<(PathBuf, Vec<u8>)> = std::iter::once((dest.clone(), files.primary))
            .chain(sidecars)
            .collect();
        // ⚠ Asked before the question is taken, so a refusal keeps it; the sidecars' names are known only once the scene is serialized.
        if let Err(why) =
            self.no_job_reads_any(writes.iter().map(|(path, _)| path.as_path()), "write")
        {
            return self.show_error_toast(why);
        }
        self.gltf.asked = None;
        self.extract_jobs.start(
            JobKind::Export,
            file_name(&dest),
            folder,
            Vec::new(),
            move |cancel, progress| {
                or_job::run(writes, &cancel, &progress, |(path, bytes)| {
                    progress.now_on(file_name(&path));
                    match std::fs::write(&path, &bytes) {
                        Ok(()) => Step::Done(bytes.len() as u64),
                        Err(e) => Step::Failed(format!("{}: {e}", path.display())),
                    }
                })
            },
        );
    }

    /// Put the answered import into its model as one step of its history; nothing is written until it is saved.
    pub fn apply_gltf_import(&mut self) {
        let made = |asked: &Option<Asked>| {
            matches!(
                asked,
                Some(Asked::Import(Import {
                    made: Some(Ok(_)),
                    ..
                }))
            )
        };
        if !made(&self.gltf.asked) {
            return;
        }
        let Some(Asked::Import(Import {
            doc,
            mesh,
            edits,
            made: Some(Ok((payload, _))),
            gta6,
            ..
        })) = self.gltf.asked.take()
        else {
            return;
        };
        let Some(document) = self.document_mut(doc) else {
            return self.show_error_toast(tr!("That model is closed"));
        };
        if document.edit_epoch() != edits {
            return self.show_error_toast(tr!(
                "The model changed while the glTF was read: replace it again"
            ));
        }
        if let Some(gta6) = gta6 {
            return self.apply_gta6_import(doc, &mesh, &gta6, *payload);
        }
        let before = std::mem::replace(&mut document.payload, *payload);
        let change = Change::part(tr!("Replace from glTF"), before, |p: &mut DocPayload| {
            Some(p)
        });
        self.record(
            doc,
            tr!("Replace from {mesh}", mesh = file_name(&mesh)),
            change,
        );
        // ⚠ An import is a step of its own, never folded into the next.
        if let Some(document) = self.document_mut(doc) {
            document.history.seal();
        }
        self.show_toast(tr!(
            "Replaced from {mesh}: save to keep it, or undo it",
            mesh = file_name(&mesh)
        ));
    }
}
