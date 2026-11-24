use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::*;

#[path = "save_source.rs"]
mod save_source;
use save_source::write_the_file;
pub(super) use save_source::SaveSource;

/// A save on a worker: the file it writes, the flag that stops it before the write, and where its result arrives.
pub(super) struct SaveJob {
    pub(super) path: PathBuf,
    pub(super) cancel: or_job::Cancel,
    pub(super) receiver: Receiver<SaveResult>,
}

/// The files and folders named on the command line, which is how the shell
/// hands OpenRage something it has been asked to open.
pub fn command_line_paths(args: impl IntoIterator<Item = std::ffi::OsString>) -> Vec<PathBuf> {
    args.into_iter()
        .filter(|a| !a.to_string_lossy().starts_with("--"))
        .map(PathBuf::from)
        .collect()
}

impl AppState {
    /// Open what the command line named; with nothing named, the project that was open when OpenRage last ran, else the start page.
    ///
    /// ⚠ A path named means this window was asked for something: a second window is started on a
    /// project's path, and it must not also reopen the project the first window has.
    pub fn open_from_command_line(&mut self, paths: Vec<PathBuf>) {
        let (projects, paths): (Vec<PathBuf>, Vec<PathBuf>) = paths
            .into_iter()
            .partition(|path| super::front_door::is_project_file(path));
        let named = projects.into_iter().next();
        let reopening = named.is_none() && paths.is_empty();
        let project = named.or_else(|| reopening.then(|| self.last_project_to_reopen()).flatten());
        let opening = project.as_ref().is_some_and(|project| project.is_file());
        if paths.is_empty() && !opening {
            // ⚠ The start page is the Document panel's, which a workspace such
            // as Model does not show, so a launch left in one shows nothing to open.
            let shown = self.workspace_docks.shows(
                self.active_workspace,
                crate::ui::panels::registry::ids::DOCUMENT,
            );
            if self.documents.is_empty() && !shown {
                self.active_workspace = Workspace::Browse;
            }
        }
        match project {
            Some(project) if reopening => self.reopen_project(project),
            Some(project) => self.open_named_project(project),
            None => {}
        }
        if paths.is_empty() {
            return;
        }
        let (folders, files): (Vec<PathBuf>, Vec<PathBuf>) =
            paths.into_iter().partition(|p| p.is_dir());
        for folder in folders {
            self.open_folder(folder);
        }
        self.load_files(files);
    }

    pub fn load_file(&mut self, path: PathBuf) {
        self.pending_loads.push_back(path);
        self.try_start_next_load();
    }

    pub fn load_files(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if paths.len() > 1 || !self.pending_loads.is_empty() || self.load_receiver.is_some() {
            self.batch_loading = true;
        }
        for path in paths {
            self.pending_loads.push_back(path);
        }
        self.try_start_next_load();
    }

    fn needs_enhanced_switch_prompt(&mut self, path: &std::path::Path) -> bool {
        if self.pending_enhanced_open.is_some() {
            return false;
        }

        let Some((generation, version)) = read_file_generation(path) else {
            return false;
        };

        if self.prefs.active_game.map(|g| g.generation()) == Some(generation) {
            return false;
        }
        if !generation.is_gen9() {
            return false;
        }

        let Some(game) =
            Game::all().find(|g| g.generation() == generation && g.profile().is_supported())
        else {
            return false;
        };

        self.pending_enhanced_open = Some(PendingEnhancedOpen {
            path: path.to_path_buf(),
            game,
            version,
            enhanced_root_known: self.prefs.game_paths.get(game).is_some_and(|p| p.exists()),
        });
        true
    }

    pub fn apply_pending_enhanced_open(&mut self, decision: EnhancedOpenDecision) {
        let Some(prompt) = self.pending_enhanced_open.take() else {
            return;
        };

        match decision {
            EnhancedOpenDecision::Cancel => {
                if self.pending_loads.front() == Some(&prompt.path) {
                    self.pending_loads.pop_front();
                }
                self.show_info_toast(tr!("Cancelled — the file was not opened"));
            }
            EnhancedOpenDecision::Switch => {
                let game = prompt.game;
                let label = game.label();
                match self.prefs.game_paths.get(game).cloned() {
                    Some(root) if root.exists() => {
                        // ⚠ The file waits on the switch, which may wait on the user: it opens once the switch is made.
                        if self.pending_loads.front() == Some(&prompt.path) {
                            self.pending_loads.pop_front();
                        }
                        self.switch_game_then(game, root, Some(prompt.path));
                    }
                    _ => {
                        self.switch_active_game(game);
                        self.show_warning_toast(tr!(
                            "Switched to {label}. No {label} install is configured, \
                             so nothing was mounted — set the folder in Settings.",
                            label = label
                        ));
                    }
                }
            }
        }

        self.try_start_next_load();
    }

    fn try_start_next_load(&mut self) {
        if self.load_receiver.is_some() {
            return;
        }
        let path = match self.pending_loads.pop_front() {
            Some(p) => p,
            None => {
                self.batch_loading = false;
                return;
            }
        };
        // ⚠ Every open from disk comes through here: a dialog, a recent file, a
        // drop, the command line, a project and a walk.
        if let Some(open) = self.document_holding(&path, None) {
            self.set_active_document(open);
            self.try_start_next_load();
            return;
        }

        let kind = DocumentKind::from_path(&path).unwrap_or(DocumentKind::Hex);

        if self.needs_enhanced_switch_prompt(&path) {
            self.pending_loads.push_front(path);
            return;
        }

        if matches!(kind, DocumentKind::Rpf) && self.maybe_defer_encrypted_rpf(&path) {
            self.show_info_toast(tr!(
                "Encrypted RPF requires GTA V keys; load deferred until keys are available"
            ));
            self.try_start_next_load();
            return;
        }

        tracing::info!(path = %path.display(), kind = ?kind, "Starting file load");
        self.loading = LoadingState::Loading;

        let (tx, rx): (Sender<LoadResult>, Receiver<LoadResult>) = channel();
        self.load_receiver = Some((path.clone(), rx));

        let provider = self.keys.provider();
        let path_clone = path.clone();
        thread::spawn(move || {
            let result =
                parse_payload_from_path(&path_clone, kind, provider).map(|(payload, resource)| {
                    LoadResult::Success {
                        path: path_clone.clone(),
                        payload,
                        binding: None,
                        resource,
                    }
                });

            let msg = match result {
                Ok(ok) => ok,
                Err(message) => LoadResult::Error {
                    path: path_clone,
                    message,
                },
            };
            let _ = tx.send(msg);
        });
    }

    pub fn open_rpf_entry(&mut self, parent_doc_id: DocumentId, entry_id: RpfNodeId) {
        let row = RpfEntryBinding {
            parent_doc_id,
            entry_id,
        };
        self.tabs.keep(row);
        if let Some(existing) = self.entry_document(row) {
            // A dictionary the app opened for itself is the user's once opened: it gets a tab and stays.
            self.auto_opened_texture_docs.remove(&existing);
            self.set_active_document(existing);
            self.remember_recent(existing);
            return;
        }
        // ⚠ Its load is on its way: a second one would parse on the UI thread while the first runs.
        if self.tabs.arriving == Some(row) {
            return;
        }

        let (parent_path, child_path) = {
            let Some(doc) = self.document(parent_doc_id) else {
                self.show_error_toast(tr!("RPF parent document is gone"));
                return;
            };
            let DocPayload::Rpf(rpf) = &doc.payload else {
                self.show_error_toast(tr!("Active document is not an RPF"));
                return;
            };
            let Some(node) = rpf.session.node(entry_id) else {
                self.show_error_toast(tr!("RPF node {entry_id} not found", entry_id = entry_id));
                return;
            };
            if node.is_dir() {
                return;
            }
            // ⚠ A GTA VI pack can be gigabytes and is read a texture at a time: it opens from
            // its file, never through its bytes.
            if let Some(pack) = node
                .backing
                .fs_path()
                .filter(|path| matches!(DocumentKind::from_path(path), Some(DocumentKind::Ttpf)))
            {
                let pack = pack.to_path_buf();
                self.load_file(pack);
                return;
            }
            let Some(child_path) = rpf.session.path_of(entry_id) else {
                self.show_error_toast(tr!("Failed to resolve RPF path"));
                return;
            };
            (doc.path.clone(), child_path)
        };

        let mut display_path = parent_path.unwrap_or_else(|| PathBuf::from(tr!("(unsaved RPF)")));
        for part in child_path.split('/') {
            display_path.push(part);
        }
        // A loose file in a folder's tree may already be open from disk.
        if let Some(open) = self.document_holding(&display_path, Some(row)) {
            self.set_active_document(open);
            return;
        }
        let bytes = match self.document(parent_doc_id).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => rpf.session.bytes_for(entry_id),
            _ => None,
        };
        let Some(bytes) = bytes else {
            self.show_error_toast(tr!(
                "Missing bytes for '{child_path}'",
                child_path = child_path
            ));
            return;
        };
        let bytes = (*bytes).clone();

        let kind = DocumentKind::from_path(&PathBuf::from(&child_path))
            .map(|kind| kind.refine_with_bytes(&bytes))
            .unwrap_or(DocumentKind::Hex);

        self.spawn_inline_load(display_path, kind, bytes, Some(row));
    }

    /// Opens `bytes`, read on the loader, as a document of `kind` under `path`; one already open there is shown.
    pub fn open_bytes(&mut self, path: PathBuf, kind: DocumentKind, bytes: Vec<u8>) {
        if let Some(open) = self.document_holding(&path, None) {
            self.set_active_document(open);
            return;
        }
        self.spawn_inline_load(path, kind, bytes, None);
    }

    fn spawn_inline_load(
        &mut self,
        path: PathBuf,
        kind: DocumentKind,
        data: Vec<u8>,
        binding: Option<RpfEntryBinding>,
    ) {
        let provider = self.keys.provider();
        if self.load_receiver.is_some() {
            match parse_payload(&path, kind, &data, provider) {
                Ok(payload) => {
                    let resource = resource_image_for(kind, &data);
                    let _id = self.insert_document(path, payload, binding, resource);
                }
                Err(e) => self.report_load_error(&path, e),
            }
            return;
        }
        self.loading = LoadingState::Loading;
        let (tx, rx): (Sender<LoadResult>, Receiver<LoadResult>) = channel();
        self.load_receiver = Some((path.clone(), rx));
        self.tabs.arriving = binding;
        let path_clone = path.clone();
        thread::spawn(move || {
            let result = parse_payload(&path_clone, kind, &data, provider).map(|payload| {
                LoadResult::Success {
                    path: path_clone.clone(),
                    payload,
                    binding,
                    resource: resource_image_for(kind, &data),
                }
            });
            let msg = match result {
                Ok(ok) => ok,
                Err(message) => LoadResult::Error {
                    path: path_clone,
                    message,
                },
            };
            let _ = tx.send(msg);
        });
    }

    /// A file that would not open: its name and one plain sentence are said, and the reader's own words are the window's details and the Log's.
    fn report_load_error(&mut self, path: &Path, message: String) {
        let headline = super::load_error::headline(path);
        if self.batch_loading {
            self.show_error_toast(headline);
            self.loading = LoadingState::Idle;
        } else {
            self.set_error(tr!("Could not open the file"), message.clone());
            self.error_dialog_title = headline;
            // A body a scan found says what it is and why it does not open, which no plain sentence says.
            self.error_dialog_plain = (super::rpf9_scan::scanned_body(path).is_none()
                && !super::document::rpf9::entry::is_entry(path)
                && super::gta_vi::rpf9_refusal_at(path).is_none())
            .then(|| super::load_error::plain_reason(&message).to_string());
        }
    }

    /// Show the error window, titled with the operation that failed, as in `Could not save the file`.
    pub fn set_error(&mut self, title: &'static str, message: String) {
        self.loading = LoadingState::Error(message.clone());
        self.error_dialog_title = title.to_string();
        self.error_dialog_message = message;
        self.error_dialog_plain = None;
        self.show_error_dialog = true;
    }

    pub fn clear_error(&mut self) {
        if matches!(self.loading, LoadingState::Error(_)) {
            self.loading = LoadingState::Idle;
        }
        self.show_error_dialog = false;
        self.error_dialog_message.clear();
        self.error_dialog_plain = None;
    }

    pub fn poll_load_result(&mut self) {
        let result = match &self.load_receiver {
            Some((_, receiver)) => match receiver.try_recv() {
                Ok(result) => result,
                Err(_) => return,
            },
            None => return,
        };
        self.load_receiver = None;

        match result {
            LoadResult::Success {
                path,
                payload,
                binding,
                resource,
            } => {
                let said = tr!("Opened {path}", path = file_name(&path));
                let shown = if self.tabs.arrived(binding, &path) {
                    tracing::debug!(path = %path.display(), "a preview replaced before it loaded");
                    None
                } else if let Some(open) = self.document_holding(&path, binding) {
                    self.set_active_document(open);
                    Some(open)
                } else {
                    let id = self.insert_document(path, payload, binding, resource);
                    tracing::info!(id, "Document loaded");
                    Some(id)
                };
                self.loading = LoadingState::Idle;
                let then = shown.map_or(Then::Nothing, Then::Show);
                self.jobs.record(JobKind::Load, Outcome::Done(then), said);
            }
            LoadResult::Error { path, message } => {
                self.tabs.arrived(None, &path);
                tracing::error!(path = %path.display(), error = %message, "Load failed");
                let said = format!("{}: {message}", file_name(&path));
                self.report_load_error(&path, message);
                self.jobs.record(JobKind::Load, Outcome::Failed, said);
            }
        }

        self.try_start_next_load();
    }

    /// The open in flight, as the job list draws it.
    pub(super) fn load_job(&self) -> Option<RunningJob> {
        let (path, _) = self.load_receiver.as_ref()?;
        let waiting = self.pending_loads.len();
        Some(RunningJob {
            kind: JobKind::Load,
            doing: tr!("Opening {path}", path = file_name(path)),
            fraction: None,
            detail: match waiting {
                0 => String::new(),
                n => tr!("{n} more waiting", n = n),
            },
            stopping: false,
        })
    }

    /// Drop the open in flight and every one waiting; `false` when none runs.
    pub(super) fn cancel_loads(&mut self) -> bool {
        // ⚠ A parse takes no cancel, so its worker finishes unheard and opens nothing.
        if self.load_receiver.take().is_none() {
            return false;
        }
        self.pending_loads.clear();
        self.batch_loading = false;
        self.loading = LoadingState::Idle;
        true
    }

    /// The save in flight, as the job list draws it.
    pub(super) fn save_job(&self) -> Option<RunningJob> {
        let job = self.save_receiver.as_ref()?;
        Some(RunningJob {
            kind: JobKind::Save,
            doing: tr!("Saving {path}", path = file_name(&job.path)),
            fraction: None,
            detail: String::new(),
            stopping: job.cancel.is_set(),
        })
    }

    /// ⚠ A save stops only before its write begins: once writing, it finishes, or the file would be half of each.
    pub(super) fn cancel_save(&mut self) {
        if let Some(job) = &self.save_receiver {
            job.cancel.set();
        }
    }

    pub fn poll_save_result(&mut self) {
        let Some(job) = &self.save_receiver else {
            return;
        };
        let Ok(result) = job.receiver.try_recv() else {
            return;
        };
        let name = file_name(&job.path);
        self.save_receiver = None;
        match result {
            SaveResult::Success { doc_id, path, at } => {
                tracing::info!(path = %path.display(), "Save completed successfully");
                self.file_replaced(&path);
                if let Some(doc) = self.document_mut(doc_id) {
                    doc.saved_as_of(at);
                }
                self.reread_saved_pack(doc_id, &path);
                self.loading = LoadingState::Idle;
                let folder = path.parent().map(Path::to_path_buf);
                if let Some(saved) = super::explorer::Place::new(&path, "") {
                    self.prefs.touch_recent_file(saved);
                }
                save_prefs(&self.prefs);
                self.show_toast(tr!("File saved successfully"));
                let then = folder.map_or(Then::Nothing, Then::OpenFolder);
                let said = tr!("Saved {name}", name = name);
                self.jobs.record(JobKind::Save, Outcome::Done(then), said);
            }
            SaveResult::Error { message } => {
                self.set_error(tr!("Could not save the file"), message.clone());
                self.show_error_toast(tr!("Save failed: {message}", message = message));
                self.jobs.record(JobKind::Save, Outcome::Failed, message);
            }
            SaveResult::Stopped => {
                self.loading = LoadingState::Idle;
                let said = tr!("{name} was not written", name = name);
                self.jobs
                    .record(JobKind::Save, Outcome::Stopped(Then::Nothing), said);
            }
        }
    }

    pub fn save_active_document(&mut self) -> bool {
        let doc_id = match self.active_document {
            Some(id) => id,
            None => return false,
        };
        if let Some(saved) = self.save_animation_into_its_audio(doc_id) {
            return saved;
        }
        if let Some(started) = self.save_edited_resource(doc_id) {
            return started;
        }
        if self.refused_as_read_only_document(doc_id) {
            return false;
        }
        if let Some(doc) = self.document(doc_id) {
            if matches!(doc.kind, DocumentKind::GameRoot | DocumentKind::Folder) {
                self.save_dirty_archives(doc_id);
                return true;
            }
        }
        if !self.flush_anim_edits_for_save() {
            return false;
        }
        if self.offer_save_destination(doc_id) {
            return true;
        }
        let (path, kind) = {
            if let Some(doc) = self.document_mut(doc_id) {
                if let DocPayload::Meta(m) = &mut doc.payload {
                    m.ensure_text_current();
                }
            }
            let doc = match self.document(doc_id) {
                Some(d) => d,
                None => return false,
            };
            let path = match &doc.path {
                Some(p) => p.clone(),
                None => return false,
            };
            (path, doc.kind)
        };
        // ⚠ A loose file opened from inside an install — a `.ymt`, a `.ytd`, or
        // `commandline.txt` — is saved through here, so without the gate it is
        // overwritten with nothing asked.
        match self.gate_permits(Some(&path)) {
            GateOutcome::Refused => return false,
            GateOutcome::Confirm(target) => {
                self.pending_retail_edit = Some(PendingRetailEditPrompt {
                    action: PendingRetailEdit::SaveLooseFile { doc_id },
                    target_path: target,
                    verb: "save",
                });
                return true;
            }
            GateOutcome::Proceed => {}
        }
        let Some(source) = self.save_source_for(doc_id) else {
            return false;
        };
        self.spawn_save(doc_id, path, kind, source)
    }

    /// The tail of [`save_active_document`](Self::save_active_document), after
    /// the gate has been satisfied.
    pub(super) fn save_loose_file_inner(&mut self, doc_id: DocumentId) {
        if self.save_edited_resource_inner(doc_id).is_some() {
            return;
        }
        let (path, kind) = match self.document(doc_id) {
            Some(doc) => match &doc.path {
                Some(p) => (p.clone(), doc.kind),
                None => return,
            },
            None => return,
        };
        let Some(source) = self.save_source_for(doc_id) else {
            return;
        };
        self.spawn_save(doc_id, path, kind, source);
    }

    fn save_dirty_archives(&mut self, doc_id: DocumentId) {
        let target = self.gated_dirty_archive_path(doc_id);
        match self.gate_permits(target.as_deref()) {
            GateOutcome::Proceed => self.save_dirty_archives_inner(doc_id),
            GateOutcome::Refused => {}
            GateOutcome::Confirm(path) => {
                self.pending_retail_edit = Some(PendingRetailEditPrompt {
                    action: PendingRetailEdit::SaveDirtyArchives { doc_id },
                    target_path: path,
                    verb: "save",
                });
            }
        }
    }

    /// The dirty archive the gate is asked about: a shipped one, else one in the overlay.
    ///
    /// ⚠ Shipped first: `Overlay` permits the overlay, so asking about an
    /// overlay archive would let a shipped one save with it.
    fn gated_dirty_archive_path(&self, doc_id: DocumentId) -> Option<PathBuf> {
        let doc = self.document(doc_id)?;
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return None;
        };
        let mut overlay = None;
        for id in rpf.session.dirty_archive_ids() {
            let Some(path) = rpf.session.archive(id).and_then(|s| s.fs_path.as_ref()) else {
                continue;
            };
            match retail::classify(path, &self.prefs) {
                retail::PathClass::ShippedGame => return Some(path.clone()),
                retail::PathClass::Overlay => {
                    overlay.get_or_insert_with(|| path.clone());
                }
                retail::PathClass::Outside => {}
            }
        }
        overlay
    }

    pub(super) fn save_dirty_archives_inner(&mut self, doc_id: DocumentId) {
        // An archive nested in another has no path: saving the archive on disk holding it writes it.
        let dirty_ids: Vec<ArchiveId> = match self.document(doc_id) {
            Some(d) => match &d.payload {
                DocPayload::Rpf(rpf) => rpf
                    .session
                    .dirty_archive_ids()
                    .into_iter()
                    .filter(|id| {
                        rpf.session
                            .archive(*id)
                            .is_some_and(|slot| slot.fs_path.is_some())
                    })
                    .collect(),
                _ => return,
            },
            None => return,
        };
        if dirty_ids.is_empty() {
            self.show_info_toast(tr!("No archives need saving"));
            return;
        }
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut left = dirty_ids;
        while !left.is_empty() {
            // ⚠ A nested archive moved out of an archive holds that one's save
            // until the archive now holding it is saved, so that one goes first.
            let next = match self.document(doc_id).map(|d| &d.payload) {
                Some(DocPayload::Rpf(rpf)) => left
                    .iter()
                    .position(|&id| rpf.session.save_refused(id).is_none())
                    .unwrap_or(0),
                _ => 0,
            };
            let archive_id = left.remove(next);
            match self.save_one_archive(doc_id, archive_id) {
                Ok(path) => {
                    tracing::info!(?archive_id, path = %path.display(), "Archive saved");
                    succeeded += 1;
                }
                Err(e) => {
                    self.show_error_toast(tr!("Save archive failed: {e}", e = e));
                    failed += 1;
                }
            }
        }
        self.show_toast(if failed == 0 {
            tr!("Saved {saved} archive(s)", saved = succeeded)
        } else {
            tr!(
                "Saved {saved} archive(s), {failed} failed",
                saved = succeeded,
                failed = failed
            )
        });
    }

    pub(super) fn save_one_archive(
        &mut self,
        doc_id: DocumentId,
        archive_id: ArchiveId,
    ) -> Result<PathBuf, String> {
        let (bytes, fs_path) = self.archive_bytes(doc_id, archive_id, None)?;
        if let Some(DocPayload::Rpf(rpf)) = self.document_mut(doc_id).map(|d| &mut d.payload) {
            rpf.session.carry_readers_of(archive_id)?;
        }
        atomic_write_with_backup(&fs_path, &bytes)
            .map_err(|e| tr!("Write '{fs_path}': {e}", e = e, fs_path = fs_path.display()))?;
        self.expander.files_replaced();
        self.file_replaced(&fs_path);
        let Some(doc) = self.document_mut(doc_id) else {
            return Ok(fs_path);
        };
        doc.archive_written(archive_id);
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return Ok(fs_path);
        };
        let read_back = super::rpf_archive_open::open_archive_from_disk(
            &fs_path,
            Arc::clone(&rpf.session.crypto),
        )
        .and_then(|written| rpf.session.adopt_written_archive(archive_id, written));
        if let Err(e) = read_back {
            if let Some(root) = rpf.session.archive_root_node(archive_id) {
                rpf.session.unload_archive(root);
            }
            return Err(tr!("`{fs_path}` was written, but reading it back failed, so it is closed in the tree: {e}", e = e, fs_path = fs_path.display()));
        }
        Ok(fs_path)
    }

    /// An archive rebuilt from the session's records, and the path it was opened from.
    ///
    /// ⚠ The jobs are asked about the file the bytes go to, `dest` or else the
    /// path they came from: a copy written beside a shipped archive replaces
    /// nothing a job reads of it.
    pub(super) fn archive_bytes(
        &self,
        doc_id: DocumentId,
        archive_id: ArchiveId,
        dest: Option<&Path>,
    ) -> Result<(Vec<u8>, PathBuf), String> {
        type SaveInputs = (
            Vec<(String, Vec<u8>, bool)>,
            PathBuf,
            RpfOutputMode,
            Arc<dyn or_io::container::RpfCryptoProvider>,
        );
        let (records, fs_path, rpf_mode, crypto): SaveInputs = {
            let doc = self
                .document(doc_id)
                .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
            let DocPayload::Rpf(rpf) = &doc.payload else {
                return Err(tr!("Document is not an RPF / GameRoot").to_string());
            };
            let slot = rpf.session.archive(archive_id).ok_or_else(|| {
                tr!(
                    "Archive {archive_id} not registered",
                    archive_id = archive_id
                )
            })?;
            let fs_path = slot.fs_path.clone().ok_or_else(|| {
                tr!(
                    "Archive {archive_id} has no on-disk path",
                    archive_id = archive_id
                )
            })?;
            if self.defrag.holds(&fs_path) {
                return Err(tr!(
                    "`{fs_path}` is being defragmented; save once that is done",
                    fs_path = fs_path.display()
                ));
            }
            self.no_job_reads(dest.unwrap_or(&fs_path), "save")?;
            let records = rpf.session.file_records_for_archive(archive_id)?;
            (
                records,
                fs_path,
                self.prefs.rpf_output_mode,
                Arc::clone(&rpf.session.crypto),
            )
        };
        Ok((
            serialize_archive_records(records, rpf_mode, crypto)?,
            fs_path,
        ))
    }

    fn flush_anim_edits_for_save(&mut self) -> bool {
        match self.bake_anim_edits() {
            Ok(Some(summary)) => {
                self.show_toast(summary);
                true
            }
            Ok(None) => true,
            Err(reason) => {
                self.show_error_toast(tr!("Cannot save: {reason}", reason = reason));
                false
            }
        }
    }

    pub fn save_active_document_as(&mut self, path: PathBuf) -> bool {
        let doc_id = match self.active_document {
            Some(id) => id,
            None => return false,
        };
        // A body a scan found is saved as a copy, so only Save refuses it.
        let scanned = (self.document(doc_id))
            .is_some_and(|d| d.read_only == Some(super::rpf9_scan::READ_ONLY_BODY));
        if !scanned && self.refused_as_read_only_document(doc_id) {
            return false;
        }
        // ⚠ Save As is gated too: a file picker opened in a game directory
        // offers `update.rpf` as readily as anything else.
        //
        // The document's own path is *not* updated before the gate answers, so a
        // refusal leaves the document pointing where it did.
        match self.gate_permits(Some(&path)) {
            GateOutcome::Refused => return false,
            GateOutcome::Confirm(target) => {
                self.pending_retail_edit = Some(PendingRetailEditPrompt {
                    action: PendingRetailEdit::SaveAs {
                        doc_id,
                        path: path.clone(),
                    },
                    target_path: target,
                    verb: tr!("save to"),
                });
                return true;
            }
            GateOutcome::Proceed => {}
        }
        self.save_as_inner(doc_id, path)
    }

    pub(super) fn save_as_inner(&mut self, doc_id: DocumentId, path: PathBuf) -> bool {
        if let Some(started) = self.save_scanned_body_as(doc_id, &path) {
            return started;
        }
        // ⚠ Asked before the document takes the new path, so a refusal leaves it where it was.
        if self.refused_save(&path) {
            return false;
        }
        if !self.flush_anim_edits_for_save() {
            return false;
        }
        self.say_the_animation_hashes_are_kept(doc_id);
        let kind = {
            let doc = match self.document_mut(doc_id) {
                Some(d) => d,
                None => return false,
            };
            if let DocPayload::Meta(m) = &mut doc.payload {
                m.ensure_text_current();
            }
            let new_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(tr!("Untitled"))
                .to_string();
            doc.path = Some(path.clone());
            doc.name = new_name;
            doc.kind
        };
        let Some(source) = self.save_source_for(doc_id) else {
            return false;
        };
        self.spawn_save(doc_id, path, kind, source)
    }

    /// Refuse a write of a document no view may change, saying why; true when it was refused.
    pub(crate) fn refused_as_read_only_document(&mut self, doc_id: DocumentId) -> bool {
        match self.document(doc_id).and_then(|doc| doc.read_only) {
            Some(why) => {
                self.show_warning_toast(crate::i18n::tr(why));
                true
            }
            None => false,
        }
    }

    pub(super) fn save_source_for(&mut self, doc_id: DocumentId) -> Option<SaveSource> {
        // ⚠ This save clears the patched mark, so the next writes the payload, which must hold the image's edits.
        self.catch_up(doc_id);
        let doc = self.document(doc_id)?;

        if doc.patched() {
            if let Some(image) = doc.resource.clone() {
                return Some(SaveSource::PatchedImage(image));
            }
        }

        Some(SaveSource::Payload(Box::new(doc.payload.clone())))
    }

    /// Refuse a save of the document to `path` that a job reading it forbids, saying why; true when refused.
    fn refused_save(&mut self, path: &std::path::Path) -> bool {
        let why = self.no_job_reads(path, "save").err();
        let refused = why.is_some();
        if let Some(why) = why {
            self.show_error_toast(why);
        }
        refused
    }

    /// Write the document to `path` on a worker; `false` once a job reading `path` refused it.
    fn spawn_save(
        &mut self,
        doc_id: DocumentId,
        path: PathBuf,
        kind: DocumentKind,
        source: SaveSource,
    ) -> bool {
        if self.refused_save(&path) {
            return false;
        }
        tracing::info!(path = %path.display(), kind = ?kind, "Starting file save");
        let at = self.document(doc_id).map_or(0, |doc| doc.history.at());
        // A pack streams into a file beside `path`: it may be gigabytes, and its rows are read from the file it was opened from.
        if let SaveSource::Payload(payload) = &source {
            if let DocPayload::Ttpf(ttpf) = &**payload {
                if ttpf.pack.is_relaid() {
                    self.show_warning_toast(tr!(
                        "A texture of this pack changed its size, format or mips, so the textures after it moved: no game has been seen to load a pack written this way."
                    ));
                }
                let (ttpf, to) = (ttpf.clone(), path.clone());
                self.start_save(path, move |cancel| {
                    super::ttpf_document::saved(&ttpf, &to, cancel, doc_id, at)
                });
                return true;
            }
        }
        let path_for_thread = path.clone();
        let rpf_mode = self.prefs.rpf_output_mode;
        // ⚠ A list that is what a particles source compiles to is saved into the source: the file is the cook's own, written again by the next recook.
        let into_source = self.particles_source_of(&path);
        let crypto = self.keys.provider();
        // ⚠ A save in place asks nothing, so only the setting consents to a repair.
        let repair = match self.prefs.export_fixes {
            ExportFixes::Apply => or_io::decl::Repair::Lossless,
            ExportFixes::Ask | ExportFixes::Keep => or_io::decl::Repair::Refuse,
        };
        self.start_save(path, move |cancel| {
            let serialized = source.into_bytes(rpf_mode, crypto, repair);
            if cancel.is_set() {
                return SaveResult::Stopped;
            }
            match serialized {
                Ok((bytes, repairs)) => {
                    match write_the_file(&path_for_thread, &bytes, into_source.as_deref()) {
                        Ok(()) => {
                            for repair in repairs {
                                tracing::info!("Repaired {}: {repair}", path_for_thread.display());
                            }
                            SaveResult::Success {
                                doc_id,
                                path: path_for_thread,
                                at,
                            }
                        }
                        Err(e) => SaveResult::Error {
                            message: tr!("Write error: {e}", e = e),
                        },
                    }
                }
                Err(e) => SaveResult::Error {
                    message: tr!(
                        "Serialization error in {path_for_thread}: {e}",
                        path_for_thread = path_for_thread.display(),
                        e = e
                    ),
                },
            }
        });
        true
    }

    /// Run `work` as the one save, of the file at `path`.
    pub(super) fn start_save(
        &mut self,
        path: PathBuf,
        work: impl FnOnce(&or_job::Cancel) -> SaveResult + Send + 'static,
    ) {
        self.loading = LoadingState::Saving;
        let (tx, receiver) = channel();
        let cancel = or_job::Cancel::new();
        let worker_cancel = cancel.clone();
        thread::spawn(move || {
            let _ = tx.send(work(&worker_cancel));
        });
        self.save_receiver = Some(SaveJob {
            path,
            cancel,
            receiver,
        });
    }
}

#[cfg(test)]
mod command_line_tests {
    use super::*;

    #[test]
    fn the_command_line_names_paths_and_flags_are_not_paths() {
        let args = ["--self-check", "C:/mods/car.ydr", "D:/game/x64a.rpf"].map(Into::into);
        assert_eq!(
            command_line_paths(args),
            vec![
                PathBuf::from("C:/mods/car.ydr"),
                PathBuf::from("D:/game/x64a.rpf")
            ]
        );
    }

    #[test]
    fn a_path_on_the_command_line_opens_and_with_none_the_start_page_is_on_screen() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut state = AppState::from_prefs(Prefs::default());
        state.open_from_command_line(vec![dir.path().to_path_buf()]);
        assert!(
            state
                .documents
                .iter()
                .any(|d| d.kind == DocumentKind::Folder && d.path.as_deref() == Some(dir.path())),
            "the folder named on the command line is open"
        );

        let mut nothing = AppState::from_prefs(Prefs::default());
        nothing.active_workspace = Workspace::Model;
        nothing.open_from_command_line(Vec::new());
        assert_eq!(
            nothing.active_workspace,
            Workspace::Browse,
            "the start page is the Document panel's, and Model shows none"
        );
        nothing.active_workspace = Workspace::Data;
        nothing.open_from_command_line(Vec::new());
        assert_eq!(nothing.active_workspace, Workspace::Data, "Data shows one");
    }
}
