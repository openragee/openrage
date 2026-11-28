//! The kind a GTA VI tree's row states, for the document opened from that row.

use crate::state::{AppState, DocPayload, DocumentId};

impl AppState {
    /// The kind in words of the tree row `doc_id` was opened from; `None` for a document no row opened.
    pub(crate) fn tree_row_kind(&self, doc_id: DocumentId) -> Option<String> {
        let path = self.document(doc_id)?.path.as_deref()?;
        let row = crate::state::rpf_archive_open::tree_open::row_of(self, path)?;
        let DocPayload::Rpf(rpf) = &self.document(row.parent_doc_id)?.payload else {
            return None;
        };
        let node = rpf.session.node(row.entry_id)?;
        Some(crate::state::files_view::kind_in_words(&rpf.session, node))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;

    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::{
        write_rpf9_resource, Gta6Context, NameResolver, Rpf9Form, Rpf9Sections,
    };
    use or_io::container::RpfArchive;
    use or_io::gta6::hash::name_hash64;

    use crate::state::rpf_archive_open::gta6_session;
    use crate::state::rpf_archive_open::tree_open::name;
    use crate::state::rpf_archive_open::tree_verbs::tests::{rbm_section, synthetic_keys};
    use crate::state::rpf_session::RpfDocument;
    use crate::state::{AppState, DocPayload, Document, DocumentId, TextDocument};

    const TREE: DocumentId = 9_700;
    const BODY: DocumentId = 9_701;
    const MODEL: &str = "data/model.ydr";

    #[test]
    fn a_body_opened_from_a_tree_row_is_named_by_the_kind_its_row_states() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let mut names = NameResolver::new();
        names.insert(name_hash64(MODEL.as_bytes()), MODEL);
        let _reading = gta6_session::hold(Gta6Context {
            keys: Arc::clone(&keys),
            names: Arc::new(names),
        });
        let loose = write_rpf9_resource(&Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: rbm_section(),
            physical: Vec::new(),
        })
        .expect("a loose resource");
        let entries = [(
            MODEL.to_owned(),
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose,
            },
        )];
        let key = Rpf9IndexKey {
            key_id: 0xDEAD_BEEF,
            iv: [0x33; 16],
        };
        let plan = build(&entries, or_io::platform::Platform::XboxSeries, &key, &keys)
            .expect("the archive builds");
        let mut bytes = Vec::new();
        (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
            .expect("the archive is laid out");
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("source.rpf");
        std::fs::write(&path, bytes).expect("the archive is written");
        let archive =
            RpfArchive::open_with(&path, gta6_session::crypto()).expect("the archive opens");
        let tree = RpfDocument::new(archive).expect("the tree lists");
        let mut state = AppState::new();
        (state.documents).push(Document::new(TREE, Some(path), DocPayload::Rpf(tree)));
        let (node, stated) = {
            let Some(DocPayload::Rpf(rpf)) = state.document(TREE).map(|d| &d.payload) else {
                panic!("the tree is open");
            };
            let session = &rpf.session;
            let node = (session.all_nodes().into_iter())
                .find(|&id| session.node(id).is_some_and(|n| n.name == "model.ydr"))
                .expect("the row is listed");
            let row = session.node(node).expect("a row");
            let kind = crate::state::explorer::gta6_kind(session, row).expect("a stated kind");
            (node, kind.label)
        };

        let body = dir.path().join("the row's body");
        let text = DocPayload::Text(TextDocument::empty());
        (state.documents).push(Document::new(BODY, Some(body.clone()), text));
        assert_eq!(state.tree_row_kind(BODY), None, "no row opened it");
        name(&state, &body, TREE, node);
        let kind = state.tree_row_kind(BODY).expect("the row's kind");
        assert_eq!(kind, stated);
        assert_ne!(kind, tr!("four zero bytes first"), "not a scan's guess");
    }
}
