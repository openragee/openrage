use super::*;

impl AppState {
    pub(super) fn require_writable_rpf_row(
        &self,
        doc: DocumentId,
        node: RpfNodeId,
    ) -> Result<(), String> {
        if matches!(self.document(doc).map(|doc| &doc.payload),
            Some(DocPayload::Rpf(rpf)) if rpf.session.is_rpf9(node))
        {
            return Err(crate::i18n::tr(document::rpf9::entry::READ_ONLY).to_string());
        }
        Ok(())
    }
    pub fn rpf_add_folder(
        &mut self,
        doc_id: DocumentId,
        parent_id: RpfNodeId,
        name: &str,
    ) -> Result<RpfNodeId, String> {
        self.require_writable_rpf_row(doc_id, parent_id)?;
        let target = self.affected_disk_path(doc_id, parent_id);
        self.gate_tree_edit(
            target.as_deref(),
            PendingRetailEdit::AddFolder {
                doc_id,
                parent_id,
                name: name.to_string(),
            },
            tr!("create folder in"),
        )?;
        self.rpf_add_folder_inner(doc_id, parent_id, name)
    }

    pub(super) fn rpf_add_folder_inner(
        &mut self,
        doc_id: DocumentId,
        parent_id: RpfNodeId,
        name: &str,
    ) -> Result<RpfNodeId, String> {
        self.require_writable_rpf_row(doc_id, parent_id)?;
        if self.is_disk_folder(doc_id, parent_id) {
            let (name, new_path) = self.claim_name(doc_id, parent_id, name, None)?;
            std::fs::create_dir(&new_path).map_err(|e| {
                tr!(
                    "Create dir '{new_path}': {e}",
                    e = e,
                    new_path = new_path.display()
                )
            })?;
            let doc = self
                .document_mut(doc_id)
                .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
            let DocPayload::Rpf(rpf) = &mut doc.payload else {
                return Err(tr!("Document is not an RPF").to_string());
            };
            return rpf.session.add_disk_child(
                parent_id,
                name,
                RpfNodeBacking::DiskFolder {
                    fs_path: new_path,
                    scanned: true,
                },
            );
        }
        self.open_archive_row(doc_id, parent_id)?;
        let doc = self
            .document_mut(doc_id)
            .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return Err(tr!("Document is not an RPF").to_string());
        };
        let id = rpf.session.add_folder(parent_id, name)?;
        doc.record(
            tr!("Add folder {name}", name = name),
            super::rpf_undo::added(id),
        );
        Ok(id)
    }

    pub fn rpf_rename_node(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        name: &str,
    ) -> Result<(), String> {
        self.require_writable_rpf_row(doc_id, node_id)?;
        // ⚠ Before the gate: a name the row already has changes nothing, so it asks and writes nothing.
        let unchanged = matches!(
            self.document(doc_id).map(|d| &d.payload),
            Some(DocPayload::Rpf(rpf))
                if rpf.session.node(node_id).is_some_and(|n| n.name == name.trim())
        );
        if unchanged {
            return Ok(());
        }
        let target = self.affected_disk_path(doc_id, node_id);
        self.gate_tree_edit(
            target.as_deref(),
            PendingRetailEdit::RenameNode {
                doc_id,
                node_id,
                name: name.to_string(),
            },
            "rename",
        )?;
        self.rpf_rename_node_inner(doc_id, node_id, name)
    }

    fn rpf_rename_node_inner(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        name: &str,
    ) -> Result<(), String> {
        self.require_writable_rpf_row(doc_id, node_id)?;
        let disk_path = self.disk_path_for_node(doc_id, node_id);
        if let Some(old_path) = disk_path {
            self.no_job_holds(&old_path, tr!("rename it"))?;
            let parent = self
                .document(doc_id)
                .and_then(|d| match &d.payload {
                    DocPayload::Rpf(rpf) => rpf.session.node(node_id).and_then(|n| n.parent),
                    _ => None,
                })
                .ok_or_else(|| tr!("Cannot rename RPF root").to_string())?;
            let (name, new_path) = self.claim_name(doc_id, parent, name, Some(node_id))?;
            std::fs::rename(&old_path, &new_path).map_err(|e| {
                tr!(
                    "Rename '{old_path}': {e}",
                    e = e,
                    old_path = old_path.display()
                )
            })?;
            let doc = self
                .document_mut(doc_id)
                .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
            let DocPayload::Rpf(rpf) = &mut doc.payload else {
                return Err(tr!("Document is not an RPF").to_string());
            };
            rpf.session.rename_node(node_id, &name)?;
            rpf.session.moved_on_disk(node_id, &old_path, &new_path);
            let reached = !rpf.session.archives_opened_under(node_id).is_empty();
            doc.changed_on_disk(reached, false);
            self.file_replaced(&old_path);
            return Ok(());
        }
        let doc = self
            .document_mut(doc_id)
            .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return Err(tr!("Document is not an RPF").to_string());
        };
        let before = rpf.session.node(node_id).map(|n| n.name.clone());
        rpf.session.rename_node(node_id, name)?;
        let label = tr!(
            "Rename {before} to {name}",
            name = name,
            before = before.as_deref().unwrap_or_default()
        );
        doc.record(
            label,
            super::rpf_undo::renamed(node_id, before.unwrap_or_default()),
        );
        Ok(())
    }

    pub fn rpf_delete_node(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
    ) -> Result<(), String> {
        self.require_writable_rpf_row(doc_id, node_id)?;
        let target = self.affected_disk_path(doc_id, node_id);
        self.gate_tree_edit(
            target.as_deref(),
            PendingRetailEdit::DeleteNode { doc_id, node_id },
            "delete",
        )?;
        self.ask_delete(doc_id, node_id)
    }

    /// Delete the row `node_id`: inside an archive as a step, on disk into the Recycle Bin at once, handing back the rows it took.
    pub(super) fn rpf_delete_node_inner(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
    ) -> Result<Option<super::rpf_undo::Rows>, String> {
        self.require_writable_rpf_row(doc_id, node_id)?;
        let disk_path = self.disk_path_for_node(doc_id, node_id);
        if let Some(path) = &disk_path {
            self.no_job_holds(path, tr!("delete it"))?;
        }
        let doc = self
            .document_mut(doc_id)
            .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return Err(tr!("Document is not an RPF").to_string());
        };
        rpf.session.carry_readers_under(node_id)?;
        let Some(path) = disk_path else {
            let name = rpf.session.node(node_id).map(|n| n.name.clone());
            let rows = (rpf.session.take_rows(node_id))
                .ok_or_else(|| tr!("RPF node {node_id} cannot be deleted", node_id = node_id))?;
            let label = tr!("Delete {name}", name = name.unwrap_or_default());
            doc.record(label, super::rpf_undo::deleted(node_id, rows));
            return Ok(None);
        };
        let reached = !rpf.session.archives_opened_under(node_id).is_empty();
        crate::platform::recycle_bin::recycle(&path)
            .map_err(|e| tr!("Delete '{path}': {e}", e = e, path = path.display()))?;
        let rows = (rpf.session.take_rows(node_id))
            .ok_or_else(|| tr!("RPF node {node_id} cannot be deleted", node_id = node_id))?;
        doc.changed_on_disk(reached, true);
        self.file_replaced(&path);
        Ok(Some(rows))
    }

    pub fn rpf_move_node(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        new_parent: RpfNodeId,
    ) -> Result<(), String> {
        self.rpf_move_nodes(doc_id, &[node_id], new_parent)
    }

    pub(super) fn rpf_move_node_inner(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        new_parent: RpfNodeId,
    ) -> Result<(), String> {
        self.require_writable_rpf_row(doc_id, node_id)?;
        self.require_writable_rpf_row(doc_id, new_parent)?;
        let src_disk = self.disk_path_for_node(doc_id, node_id);
        match (src_disk.as_ref(), self.is_disk_folder(doc_id, new_parent)) {
            (Some(src), true) => {
                self.no_job_holds(src, tr!("move it"))?;
                let name = src
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| tr!("Source has no filename").to_string())?
                    .to_string();
                let (_, new_path) = self.claim_name(doc_id, new_parent, &name, Some(node_id))?;
                self.no_job_reads(&new_path, tr!("move it"))?;
                std::fs::rename(src, &new_path).map_err(|e| {
                    tr!(
                        "Move '{src}' -> '{new_path}': {e}",
                        e = e,
                        src = src.display(),
                        new_path = new_path.display()
                    )
                })?;
                let doc = self
                    .document_mut(doc_id)
                    .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
                let DocPayload::Rpf(rpf) = &mut doc.payload else {
                    return Err(tr!("Document is not an RPF").to_string());
                };
                rpf.session.move_node(node_id, new_parent)?;
                rpf.session.moved_on_disk(node_id, src, &new_path);
                let reached = !rpf.session.archives_opened_under(node_id).is_empty();
                doc.changed_on_disk(reached, false);
                self.file_replaced(src);
                self.file_replaced(&new_path);
                Ok(())
            }
            (None, false) => {
                self.open_archive_row(doc_id, new_parent)?;
                let doc = self
                    .document_mut(doc_id)
                    .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
                let DocPayload::Rpf(rpf) = &mut doc.payload else {
                    return Err(tr!("Document is not an RPF").to_string());
                };
                let was = (rpf.session.place_of(node_id))
                    .ok_or_else(|| tr!("RPF node {node_id} cannot be moved", node_id = node_id))?;
                let name = rpf.session.node(node_id).map(|n| n.name.clone());
                rpf.session.move_node(node_id, new_parent)?;
                let label = tr!("Move {name}", name = name.unwrap_or_default());
                doc.record(label, super::rpf_undo::moved(node_id, was));
                Ok(())
            }
            _ => {
                Err(tr!("Cannot move between disk-backed and archive-backed locations").to_string())
            }
        }
    }

    pub fn rpf_import_file_to_folder(
        &mut self,
        doc_id: DocumentId,
        folder_id: RpfNodeId,
        file_path: PathBuf,
    ) -> Result<RpfNodeId, String> {
        self.require_writable_rpf_row(doc_id, folder_id)?;
        let target = self.affected_disk_path(doc_id, folder_id);
        self.gate_tree_edit(
            target.as_deref(),
            PendingRetailEdit::ImportFileToFolder {
                doc_id,
                folder_id,
                file_path: file_path.clone(),
            },
            tr!("copy into"),
        )?;
        self.rpf_import_file_to_folder_inner(doc_id, folder_id, file_path)
    }

    pub(super) fn rpf_import_file_to_folder_inner(
        &mut self,
        doc_id: DocumentId,
        folder_id: RpfNodeId,
        file_path: PathBuf,
    ) -> Result<RpfNodeId, String> {
        self.require_writable_rpf_row(doc_id, folder_id)?;
        let bytes = std::fs::read(&file_path).map_err(|e| {
            tr!(
                "Read error ({file_path}): {e}",
                e = e,
                file_path = file_path.display()
            )
        })?;
        let name = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("imported.bin")
            .to_string();
        self.add_file_to_row(doc_id, folder_id, &name, bytes)
    }

    /// `bytes` added as `name` under the row `folder`: written into a folder on disk at once, or held in the archive the row shows, opened first, until a save writes it.
    pub(super) fn add_file_to_row(
        &mut self,
        doc_id: DocumentId,
        folder: RpfNodeId,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<RpfNodeId, String> {
        if self.is_disk_folder(doc_id, folder) {
            let (name, dst) = self.claim_name(doc_id, folder, name, None)?;
            self.no_job_reads(&dst, tr!("add it"))?;
            std::fs::write(&dst, &bytes)
                .map_err(|e| tr!("Write '{dst}': {e}", e = e, dst = dst.display()))?;
            self.file_replaced(&dst);
            let doc = self
                .document_mut(doc_id)
                .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
            let DocPayload::Rpf(rpf) = &mut doc.payload else {
                return Err(tr!("Document is not an RPF").to_string());
            };
            let (label, written) = (tr!("Add {name}", name = name), (dst.clone(), bytes.clone()));
            let id = rpf.session.add_disk_child(
                folder,
                name,
                RpfNodeBacking::DiskFile {
                    fs_path: dst,
                    size_hint: bytes.len() as u64,
                    override_bytes: None,
                },
            )?;
            // The file is on disk at once, so a step that takes it back is recorded as the file is written
            doc.record(label, super::rpf_undo::wrote(written.0, written.1, id));
            return Ok(id);
        }
        self.open_archive_row(doc_id, folder)?;
        let doc = self
            .document_mut(doc_id)
            .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return Err(tr!("Document is not an RPF").to_string());
        };
        let id = rpf.session.add_file_copy(folder, name, bytes)?;
        doc.record(tr!("Add {name}", name = name), super::rpf_undo::added(id));
        Ok(id)
    }

    pub(super) fn disk_path_for_node(
        &self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
    ) -> Option<PathBuf> {
        let doc = self.document(doc_id)?;
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return None;
        };
        let node = rpf.session.node(node_id)?;
        node.backing.fs_path().map(|p| p.to_path_buf())
    }

    /// Whether what is added under the row `node_id` goes into a folder on disk rather than into an archive.
    ///
    /// ⚠ Not `disk_path_for_node`: an archive on disk has a path too, its own
    /// file's, and nothing can be created under a file.
    pub(super) fn is_disk_folder(&self, doc_id: DocumentId, node_id: RpfNodeId) -> bool {
        let Some(DocPayload::Rpf(rpf)) = self.document(doc_id).map(|d| &d.payload) else {
            return false;
        };
        matches!(
            rpf.session.node(node_id).map(|n| &n.backing),
            Some(RpfNodeBacking::DiskFolder { .. })
        )
    }

    /// The name `name` is stored under in the disk folder `folder`, and its path, once neither the tree nor the disk holds it.
    ///
    /// ⚠ Asked before the disk is touched: on Windows `fs::rename` replaces an
    /// existing file and `fs::write` overwrites one, so a clash the tree refuses
    /// afterwards has already cost a file. `moving` is the row being renamed or
    /// moved, which does not clash with itself.
    fn claim_name(
        &self,
        doc_id: DocumentId,
        folder: RpfNodeId,
        name: &str,
        moving: Option<RpfNodeId>,
    ) -> Result<(String, PathBuf), String> {
        let doc = self
            .document(doc_id)
            .ok_or_else(|| tr!("Document {doc_id} not found", doc_id = doc_id))?;
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return Err(tr!("Document is not an RPF").to_string());
        };
        let dir = rpf
            .session
            .node(folder)
            .and_then(|n| n.backing.fs_path())
            .ok_or_else(|| tr!("The folder is not on disk").to_string())?;
        // ⚠ The tree stores a name trimmed, and a separator in one would put
        // the file somewhere other than the folder the tree shows it in.
        let name = name.trim();
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err(tr!("“{name}” cannot name a file", name = name));
        }
        let path = dir.join(name);
        let listed = rpf.session.children_of(folder).into_iter().any(|id| {
            Some(id) != moving
                && rpf
                    .session
                    .node(id)
                    .is_some_and(|n| n.name.eq_ignore_ascii_case(name))
        });
        let itself = moving
            .and_then(|id| rpf.session.node(id))
            .and_then(|n| n.backing.fs_path())
            .is_some_and(|own| own.as_os_str().eq_ignore_ascii_case(path.as_os_str()));
        if listed || (!itself && std::fs::symlink_metadata(&path).is_ok()) {
            return Err(tr!(
                "`{path}` already exists, so nothing was changed",
                path = path.display()
            ));
        }
        Ok((name.to_string(), path))
    }

    /// Change the edit mode, by any route.
    ///
    /// ⚠ A base-game Yes, and a base-game question still waiting, belong to the
    /// mode they were given under: a switch to Read only and back asks again.
    pub fn set_edit_mode(&mut self, mode: EditMode) {
        if self.prefs.edit_mode == mode {
            return;
        }
        self.prefs.edit_mode = mode;
        self.retail_edit_session_allow = false;
        self.pending_retail_edit = None;
        self.refused_write = None;
        save_prefs(&self.prefs);
    }

    /// What the current edit mode says about writing `target`.
    ///
    /// ⚠ Every write path must ask this; `the_gate_stands_in_front_of_every_write`
    /// holds that. A `None` target is allowed because no file is being written
    /// yet — a new document with no path — and the gate is asked again when it
    /// acquires one.
    ///
    /// ⚠ The session's base-game Yes answers `Everything`'s question and no
    /// other, so it is consulted only once the mode permits the write.
    pub(super) fn retail_guard_for(&self, target: Option<&Path>) -> RetailGuard {
        let path = match target {
            Some(p) => p,
            None => return RetailGuard::Allowed,
        };
        let mode = self.prefs.edit_mode;
        match retail::classify(path, &self.prefs) {
            retail::PathClass::Outside => RetailGuard::Allowed,
            retail::PathClass::Overlay => {
                if mode.permits_overlay() {
                    RetailGuard::Allowed
                } else {
                    RetailGuard::Refused(tr!(
                        "`{path}` is inside a game install. {semantics}",
                        path = path.display(),
                        semantics = mode.semantics()
                    ))
                }
            }
            retail::PathClass::ShippedGame => {
                if !mode.permits_shipped_game() {
                    RetailGuard::Refused(tr!(
                        "`{path}` is part of the game as shipped. {semantics}",
                        path = path.display(),
                        semantics = mode.semantics()
                    ))
                } else if self.retail_edit_session_allow {
                    RetailGuard::Allowed
                } else {
                    RetailGuard::NeedsConfirmation(path.to_path_buf())
                }
            }
        }
    }

    /// The edit mode a write of `target` needs where the current one refuses it, which a question offers as that change; none where the mode permits the write.
    pub fn mode_a_write_needs(&self, target: &Path) -> Option<EditMode> {
        let mode = self.prefs.edit_mode;
        match retail::classify(target, &self.prefs) {
            retail::PathClass::Outside => None,
            retail::PathClass::Overlay => (!mode.permits_overlay()).then_some(EditMode::Overlay),
            retail::PathClass::ShippedGame => {
                (!mode.permits_shipped_game()).then_some(EditMode::Everything)
            }
        }
    }

    /// Ask the gate, and report to the user if it refuses.
    ///
    /// Write paths call this rather than matching on [`RetailGuard`] themselves,
    /// so every refusal is toasted one way and offers to change the mode.
    pub(super) fn gate_permits(&mut self, target: Option<&Path>) -> GateOutcome {
        if let Some(path) = target.filter(|p| self.defrag.holds(p)) {
            self.show_error_toast(tr!(
                "`{path}` is being defragmented; try again once that is done",
                path = path.display()
            ));
            return GateOutcome::Refused;
        }
        match self.retail_guard_for(target) {
            RetailGuard::Allowed => {
                if let Some(path) = target {
                    self.warn_if_overlay_unread(path);
                }
                GateOutcome::Proceed
            }
            RetailGuard::Refused(because) => {
                self.show_error_toast(because);
                // ⚠ One question at a time: the offer of an overlay copy answers the refusal, so the modes are offered only where it is not raised.
                let offered = target
                    .filter(|_| self.prefs.edit_mode.permits_overlay())
                    .is_some_and(|path| self.offer_overlay_copy(path));
                if !offered {
                    self.refused_write = target.map(Path::to_path_buf);
                }
                GateOutcome::Refused
            }
            RetailGuard::NeedsConfirmation(path) => GateOutcome::Confirm(path),
        }
    }

    /// Ask the gate before a tree edit; `Err` once it refused or raised `action`'s prompt, both already shown.
    pub(super) fn gate_tree_edit(
        &mut self,
        target: Option<&Path>,
        action: PendingRetailEdit,
        verb: &'static str,
    ) -> Result<(), String> {
        match self.gate_permits(target) {
            GateOutcome::Proceed => Ok(()),
            GateOutcome::Refused => Err(retail_pending_message()),
            GateOutcome::Confirm(target_path) => {
                self.pending_retail_edit = Some(PendingRetailEditPrompt {
                    action,
                    target_path,
                    verb,
                });
                Err(retail_pending_message())
            }
        }
    }

    pub(super) fn affected_disk_path(
        &self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
    ) -> Option<PathBuf> {
        let doc = self.document(doc_id)?;
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return None;
        };
        let node = rpf.session.node(node_id)?;
        if let Some(p) = node.backing.fs_path() {
            return Some(p.to_path_buf());
        }
        match &node.backing {
            RpfNodeBacking::Rpf9Archive { table, .. }
            | RpfNodeBacking::Rpf9Dir { table, .. }
            | RpfNodeBacking::Rpf9File { table, .. } => return Some(table.path.clone()),
            _ => {}
        }
        match rpf.session.archive_on_disk_around(node_id) {
            Some((_, path)) => Some(path),
            None => doc.path.clone(),
        }
    }

    pub fn apply_pending_retail_edit(&mut self, decision: RetailEditDecision) {
        let prompt = match self.pending_retail_edit.take() {
            Some(p) => p,
            None => return,
        };
        match decision {
            RetailEditDecision::No => {
                self.show_info_toast(tr!("Cancelled"));
                return;
            }
            RetailEditDecision::YesSession => {
                self.retail_edit_session_allow = true;
            }
        }
        match prompt.action {
            PendingRetailEdit::AddFolder {
                doc_id,
                parent_id,
                name,
            } => {
                if let Err(e) = self.rpf_add_folder_inner(doc_id, parent_id, &name) {
                    self.show_error_toast(e);
                }
            }
            PendingRetailEdit::AddEmptyFile {
                doc_id,
                parent_folder,
                kind,
            } => {
                if let Err(e) = self.rpf_add_empty_file_inner(doc_id, parent_folder, kind) {
                    self.show_error_toast(e);
                }
            }
            PendingRetailEdit::RenameNode {
                doc_id,
                node_id,
                name,
            } => {
                if let Err(e) = self.rpf_rename_node_inner(doc_id, node_id, &name) {
                    self.show_error_toast(e);
                }
            }
            PendingRetailEdit::DeleteNode { doc_id, node_id } => {
                if let Err(e) = self.ask_delete(doc_id, node_id) {
                    self.show_error_toast(e);
                }
            }
            PendingRetailEdit::MoveNode {
                doc_id,
                node_ids,
                new_parent,
            } => {
                if let Err(e) = self.rpf_move_nodes_inner(doc_id, &node_ids, new_parent) {
                    self.show_error_toast(e);
                }
            }
            PendingRetailEdit::ImportFileToFolder {
                doc_id,
                folder_id,
                file_path,
            } => {
                if let Err(e) = self.rpf_import_file_to_folder_inner(doc_id, folder_id, file_path) {
                    self.show_error_toast(e);
                }
            }
            PendingRetailEdit::ImportPaths {
                doc_id,
                folder_id,
                paths,
                staged,
            } => {
                self.import_paths_into_folder_inner(doc_id, folder_id, paths, staged.is_some());
            }
            PendingRetailEdit::PasteRows { doc_id, folder_id } => {
                self.paste_rows_inner(doc_id, folder_id);
            }
            PendingRetailEdit::Step { doc_id, direction } => {
                self.step(doc_id, direction);
            }
            PendingRetailEdit::SaveLooseFile { doc_id } => {
                self.save_loose_file_inner(doc_id);
            }
            PendingRetailEdit::SaveAs { doc_id, path } => {
                self.save_as_inner(doc_id, path);
            }
            PendingRetailEdit::SaveDirtyArchives { doc_id } => {
                self.save_dirty_archives_inner(doc_id);
            }
            PendingRetailEdit::Defragment { doc_id, node_id } => {
                self.defragment_archive_inner(doc_id, node_id);
            }
            PendingRetailEdit::SaveEntry { doc_id, dest } => {
                self.save_entry_to_inner(doc_id, dest, None);
            }
            PendingRetailEdit::ExtractInto {
                doc_id,
                nodes,
                dest,
                form,
                filter,
            } => {
                self.begin_extraction(doc_id, &nodes, dest, form, filter);
            }
        }
    }
}
