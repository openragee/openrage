//! Accepting a decoded document, with its provenance set before opening side effects.

use super::*;

impl AppState {
    pub(super) fn insert_document(
        &mut self,
        path: PathBuf,
        payload: DocPayload,
        binding: Option<RpfEntryBinding>,
        resource: Option<Arc<document::ResourceImage>>,
    ) -> DocumentId {
        self.insert_document_with_source(path, payload, binding, resource, None)
    }

    pub(super) fn insert_document_with_source(
        &mut self,
        path: PathBuf,
        payload: DocPayload,
        binding: Option<RpfEntryBinding>,
        resource: Option<Arc<document::ResourceImage>>,
        source: Option<document::rpf9::entry::EntrySource>,
    ) -> DocumentId {
        let id = self.alloc_doc_id();
        let mut doc = Document::new(id, Some(path.clone()), payload).with_resource(resource);
        doc.rpf_entry_binding = binding;
        if let Some(source) = source {
            doc.rpf9_source = Some(source);
            doc.read_only = Some(document::rpf9::entry::READ_ONLY);
        }
        if rpf9_scan::scanned_body(&path).is_some() {
            doc.read_only = Some(rpf9_scan::READ_ONLY_BODY);
        }
        let kind = doc.kind;
        self.name_index.absorb(&doc.payload);
        if let Some((names, keys)) =
            ttpf_document::texture_naming_work(&self.documents, &doc.payload)
        {
            (self.texture_naming).push(gta6_texture_naming::TextureNaming::start(names, keys));
        }
        if matches!(kind, DocumentKind::Ypt) {
            self.file_watcher.watch(path.clone());
        }
        // A dictionary opens on its first entry: the viewport draws an entry only while it is selected.
        let first_entry = crate::ui::panels::selection_ids::ydd_first_entry_hash(&doc.payload);
        self.documents.push(doc);
        gta6_head::trim(&self.documents);
        self.remember_where_the_front_document_is_seen(id);
        self.active_document = Some(id);
        self.hide_start_page();
        if let Some(hash) = first_entry {
            let entry = crate::ui::panels::selection_ids::dict_entry_id(hash);
            self.selection.select(id, entry);
        }
        if !self.active_workspace.handles_kind(kind) {
            self.active_workspace = Workspace::for_kind(kind);
        }
        if let Some(binding) = binding {
            if let Some(parent) = self.document_mut(binding.parent_doc_id) {
                if let DocPayload::Rpf(rpf) = &mut parent.payload {
                    rpf.entry_documents.insert(binding.entry_id, id);
                }
            }
        }
        self.remember_recent(id);
        id
    }
}
