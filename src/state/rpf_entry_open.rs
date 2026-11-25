use super::*;

impl AppState {
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

        let source =
            self.document(parent_doc_id)
                .and_then(|doc| match &doc.payload {
                    DocPayload::Rpf(rpf) => rpf.session.node(entry_id).and_then(|node| match &node
                        .backing
                    {
                        RpfNodeBacking::Rpf9File { table, base, index } => {
                            Some((Arc::clone(table), *base, *index))
                        }
                        _ => None,
                    }),
                    _ => None,
                });
        if let Some((table, base, index)) = source {
            self.open_rpf9_row(&table, base, index, Some(row));
            return;
        }
        if self.open_gta6_archive_entry(parent_doc_id, entry_id) {
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
}
