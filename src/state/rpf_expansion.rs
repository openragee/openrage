use super::*;

impl AppState {
    pub fn request_expand(&mut self, doc_id: DocumentId, node_id: RpfNodeId) {
        if self.expander.is_in_flight(doc_id, node_id) {
            return;
        }
        let request = match self.build_expansion_request(doc_id, node_id) {
            Some(r) => r,
            None => return,
        };
        self.expander.submit(request);
    }

    /// Open an archive row the tree has not expanded, as expanding it does, so what is added to it reaches it.
    pub(super) fn open_archive_row(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
    ) -> Result<(), String> {
        self.require_writable_rpf_row(doc_id, node_id)?;
        let Some(ExpansionRequest::Archive { source, crypto, .. }) =
            self.build_expansion_request(doc_id, node_id)
        else {
            return Ok(());
        };
        match super::expander::open_archive(source, crypto) {
            Ok(resolved) => {
                self.apply_archive_tree(doc_id, node_id, resolved)?;
                self.require_writable_rpf_row(doc_id, node_id)
            }
            Err(error) => {
                self.mark_archive_open_failed(doc_id, node_id, error.clone());
                Err(error)
            }
        }
    }

    pub(super) fn build_expansion_request(
        &self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
    ) -> Option<ExpansionRequest> {
        let doc = self.document(doc_id)?;
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return None;
        };
        if !rpf.session.needs_expansion(node_id) {
            return None;
        }
        let node = rpf.session.node(node_id)?;
        match &node.backing {
            RpfNodeBacking::DiskFolder { fs_path, .. } => Some(ExpansionRequest::DiskFolder {
                doc_id,
                node_id,
                fs_path: fs_path.clone(),
            }),
            RpfNodeBacking::Archive { source, .. } => {
                let crypto = Arc::clone(&rpf.session.crypto);
                let source = match source {
                    ArchiveSource::Disk(Some(p)) => self
                        .mounted_rpf9_table(p)
                        .map(|table| WorkerArchiveSource::Rpf9 { table, owner: None })
                        .unwrap_or_else(|| WorkerArchiveSource::Disk(p.clone())),
                    ArchiveSource::Disk(None) => return None,
                    ArchiveSource::Nested {
                        opened: Some(archive),
                        ..
                    } => WorkerArchiveSource::Opened(Arc::clone(archive)),
                    ArchiveSource::Nested {
                        parent_archive_id,
                        parent_entry_index,
                        opened: None,
                    } => WorkerArchiveSource::Nested {
                        parent: rpf.session.device(*parent_archive_id)?,
                        parent_entry_index: *parent_entry_index,
                    },
                };
                Some(ExpansionRequest::Archive {
                    doc_id,
                    node_id,
                    source,
                    crypto,
                })
            }
            RpfNodeBacking::Rpf9Archive { table, owner, .. } => Some(ExpansionRequest::Archive {
                doc_id,
                node_id,
                source: WorkerArchiveSource::Rpf9 {
                    table: Arc::clone(table),
                    owner: *owner,
                },
                crypto: Arc::clone(&rpf.session.crypto),
            }),
            _ => None,
        }
    }

    pub fn poll_expansions(&mut self) {
        let (results, stale) = self.expander.drain();
        // ⚠ Listed from a file a save or a defragment has since replaced, a
        // row would read the new file at the old offsets.
        for (doc_id, node_id) in stale {
            self.expander.finish(doc_id, node_id);
            self.request_expand(doc_id, node_id);
        }
        for result in results {
            match result {
                ExpansionResult::DiskFolder {
                    doc_id,
                    node_id,
                    items,
                } => {
                    self.apply_disk_scan(doc_id, node_id, items);
                    self.expander.finish(doc_id, node_id);
                }
                ExpansionResult::Archive {
                    doc_id,
                    node_id,
                    result,
                } => {
                    match result {
                        Ok(resolved) => {
                            if let Err(error) = self.apply_archive_tree(doc_id, node_id, resolved) {
                                self.mark_archive_open_failed(doc_id, node_id, error);
                            }
                        }
                        Err(error) => {
                            tracing::warn!(doc_id, node_id, error = %error, "Archive open failed");
                            self.mark_archive_open_failed(doc_id, node_id, error);
                        }
                    }
                    self.expander.finish(doc_id, node_id);
                }
                ExpansionResult::Failure {
                    doc_id,
                    node_id,
                    error,
                } => {
                    tracing::warn!(doc_id, node_id, error = %error, "Expansion failed");
                    self.show_error_toast(tr!("Tree scan failed: {error}", error = error));
                    self.expander.finish(doc_id, node_id);
                }
            }
        }
    }

    pub(super) fn apply_disk_scan(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        items: Vec<DiskScanItem>,
    ) {
        let Some(doc) = self.document_mut(doc_id) else {
            return;
        };
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return;
        };
        for item in items {
            let res = match item {
                DiskScanItem::Folder { name, fs_path } => rpf.session.add_disk_child(
                    node_id,
                    name,
                    RpfNodeBacking::DiskFolder {
                        fs_path,
                        scanned: false,
                    },
                ),
                DiskScanItem::File {
                    name,
                    fs_path,
                    size,
                } => rpf.session.add_disk_child(
                    node_id,
                    name,
                    RpfNodeBacking::DiskFile {
                        fs_path,
                        size_hint: size,
                        override_bytes: None,
                    },
                ),
                DiskScanItem::Archive { name, fs_path } => rpf
                    .session
                    .add_unopened_disk_archive(node_id, name, fs_path),
            };
            if let Err(e) = res {
                tracing::trace!(error = %e, "Skipped duplicate / invalid scan child");
            }
        }
        rpf.session.mark_scanned(node_id);
    }

    fn apply_archive_tree(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        resolved: ResolvedArchiveTree,
    ) -> Result<(), String> {
        match resolved {
            ResolvedArchiveTree::Archive(resolved) => {
                self.apply_archive_open(doc_id, node_id, resolved)
            }
            ResolvedArchiveTree::Rpf9 { table, owner, base } => {
                let table = if owner.is_none() {
                    self.share_rpf9_table(table)
                } else {
                    table
                };
                let Some(doc) = self.document_mut(doc_id) else {
                    return Ok(());
                };
                let DocPayload::Rpf(rpf) = &mut doc.payload else {
                    return Ok(());
                };
                if !rpf.session.needs_expansion(node_id) {
                    return Ok(());
                }
                rpf.session
                    .populate_rpf9_children(node_id, Arc::clone(&table), base)?;
                if let Some(node) = rpf.session.node_mut(node_id) {
                    node.backing = RpfNodeBacking::Rpf9Archive {
                        table,
                        owner,
                        opened_base: Some(base),
                        open_error: None,
                    };
                }
            }
        }
        Ok(())
    }

    fn apply_archive_open(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        resolved: ResolvedArchive,
    ) {
        let archive_filename = resolved
            .fs_path
            .as_deref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
        let archive_size_for_hint = resolved.archive.size().min(u32::MAX as u64) as u32;
        let recovery_hint = match resolved.archive.toc_outcome {
            or_io::container::TocDecryptOutcome::Nominal => None,
            or_io::container::TocDecryptOutcome::RecoveredNg { ng_key_index } => {
                Some(format_recovery_hint(
                    &self.known_archive_names,
                    self.keys.keys().as_ref(),
                    ng_key_index,
                    archive_size_for_hint,
                ))
            }
        };
        if let Some(name) = &archive_filename {
            self.known_archive_names.insert(name.clone());
        }
        if let Some(hint) = &recovery_hint {
            tracing::info!(
                archive = ?archive_filename,
                hint = %hint,
                "Archive opened with a key found by trying every one"
            );
        }

        let Some(doc) = self.document_mut(doc_id) else {
            return;
        };
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return;
        };
        // ⚠ An edit opens a row at once; listing it again would list it twice.
        if !rpf.session.needs_expansion(node_id) {
            return;
        }
        let registered =
            (rpf.session).register_archive(resolved.archive.clone(), resolved.fs_path.clone());
        let archive_id = match registered {
            Ok(id) => id,
            Err(error) => {
                self.mark_archive_open_failed(doc_id, node_id, error);
                return;
            }
        };
        if let Some(node) = rpf.session.node_mut(node_id) {
            if let RpfNodeBacking::Archive {
                archive_id: aid,
                source,
                scanned,
                open_error,
                recovered_hint,
            } = &mut node.backing
            {
                *aid = Some(archive_id);
                if matches!(source, ArchiveSource::Disk(_)) {
                    *source = ArchiveSource::Disk(resolved.fs_path.clone());
                }
                *scanned = true;
                *open_error = None;
                *recovered_hint = recovery_hint;
            }
        }
        if let Err(e) =
            rpf.session
                .populate_archive_children(node_id, archive_id, &resolved.listing)
        {
            tracing::warn!(error = %e, "Failed to populate archive children");
        }
    }

    fn mark_archive_open_failed(&mut self, doc_id: DocumentId, node_id: RpfNodeId, error: String) {
        let Some(doc) = self.document_mut(doc_id) else {
            return;
        };
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return;
        };
        let Some(node) = rpf.session.node_mut(node_id) else {
            return;
        };
        match &mut node.backing {
            RpfNodeBacking::Archive {
                open_error,
                recovered_hint,
                ..
            } => {
                *open_error = Some(error);
                *recovered_hint = None;
            }
            RpfNodeBacking::Rpf9Archive { open_error, .. } => *open_error = Some(error),
            _ => {}
        }
    }

    pub fn update_rpf_entry_bytes(
        &mut self,
        parent_doc_id: DocumentId,
        entry_id: RpfNodeId,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        if let Some(path) = self
            .affected_disk_path(parent_doc_id, entry_id)
            .filter(|p| self.defrag.holds(p))
        {
            return Err(tr!(
                "`{path}` is being defragmented; save once that is done",
                path = path.display()
            ));
        }
        let doc = self.document_mut(parent_doc_id).ok_or_else(|| {
            tr!(
                "Parent RPF document {parent_doc_id} not found",
                parent_doc_id = parent_doc_id
            )
        })?;
        let DocPayload::Rpf(rpf) = &mut doc.payload else {
            return Err(tr!("Parent document is not an RPF").to_string());
        };
        rpf.session.set_bytes_for(entry_id, bytes)?;
        doc.mark_edited();
        Ok(())
    }
}
