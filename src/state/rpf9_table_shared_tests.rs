//! One mounted table supplies a tab, an expanded Files archive and the Archives panel.

use super::*;
use crate::state::{
    Document, DocumentId, DocumentKind, ExpansionRequest, LoadResult, Prefs, RpfDocument,
    RpfNodeBacking, RpfNodeId, WorkerArchiveSource,
};

fn reopen(table: &Rpf9TableDocument) -> Arc<Rpf9TableDocument> {
    Arc::new(
        Rpf9TableDocument::open_with_context(
            &table.path,
            || {
                Ok(TableContext {
                    keys: Arc::clone(&table.keys),
                    resolver: Arc::clone(&table.resolver),
                })
            },
            true,
        )
        .unwrap()
        .unwrap(),
    )
}

fn folder(state: &mut AppState, path: &Path) -> (DocumentId, RpfNodeId) {
    let id = state.alloc_doc_id();
    let dir = path.parent().unwrap().to_path_buf();
    let mut rpf = RpfDocument::new_folder(dir.clone(), state.keys.provider());
    let node = rpf
        .session
        .add_unopened_disk_archive(
            rpf.session.root_id(),
            path.file_name().unwrap().to_string_lossy().into_owned(),
            path.to_path_buf(),
        )
        .unwrap();
    state.documents.push(Document::new_folder(id, dir, rpf));
    (id, node)
}

fn mounted_in_folder(state: &AppState, doc: DocumentId, node: RpfNodeId) -> Arc<Rpf9TableDocument> {
    let DocPayload::Rpf(rpf) = &state.document(doc).unwrap().payload else {
        panic!("the folder is an archive tree")
    };
    let RpfNodeBacking::Rpf9Archive { table, .. } = &rpf.session.node(node).unwrap().backing else {
        panic!("the Files row mounted its table")
    };
    Arc::clone(table)
}

fn settle(state: &mut AppState) {
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        state.poll_expansions();
        state.poll_load_result();
        if state.load_receiver.is_none()
            && state
                .documents
                .iter()
                .all(|d| !state.expander.is_listing(d.id))
        {
            break;
        }
        assert!(std::time::Instant::now() < limit, "workers finish");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!state.show_error_dialog, "{}", state.error_dialog_message);
}

fn submit_table(
    state: &mut AppState,
    doc: DocumentId,
    node: RpfNodeId,
    table: Arc<Rpf9TableDocument>,
) {
    assert!(state.expander.submit(ExpansionRequest::Archive {
        doc_id: doc,
        node_id: node,
        source: WorkerArchiveSource::Rpf9 { table, owner: None },
        crypto: state.keys.provider(),
    }));
}

fn deliver_tab(state: &mut AppState, table: Arc<Rpf9TableDocument>) -> DocumentId {
    let path = table.path.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    state.load_receiver = Some((path.clone(), rx));
    tx.send(LoadResult::Success {
        path,
        payload: DocPayload::Rpf9Table(table),
        binding: None,
        resource: None,
    })
    .unwrap();
    state.poll_load_result();
    assert!(!state.show_error_dialog, "{}", state.error_dialog_message);
    state.active_document.unwrap()
}

#[test]
fn expanding_files_reuses_the_archive_tab_table_without_reloading_keys() {
    let (_dir, table) = ui_tests::sample_document();
    let expected = Arc::new(table);
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        expected.path.clone(),
        DocPayload::Rpf9Table(Arc::clone(&expected)),
        None,
        None,
    );
    let (doc, node) = folder(&mut state, &expected.path);
    state.request_expand(doc, node);
    settle(&mut state);
    assert!(Arc::ptr_eq(
        &mounted_in_folder(&state, doc, node),
        &expected
    ));
}

#[test]
fn opening_an_archive_tab_reuses_the_files_table_without_reloading_keys() {
    let (_dir, table) = ui_tests::sample_document();
    let expected = Arc::new(table);
    let mut state = AppState::from_prefs(Prefs::default());
    let (doc, node) = folder(&mut state, &expected.path);
    submit_table(&mut state, doc, node, Arc::clone(&expected));
    settle(&mut state);
    state.load_file(expected.path.clone());
    settle(&mut state);
    let DocPayload::Rpf9Table(table) = &state.active_document().unwrap().payload else {
        panic!("the archive opened in its own tab")
    };
    assert!(Arc::ptr_eq(table, &expected));
    state.poll_table_opens();
    let panel = state
        .panels
        .get::<crate::ui::panels::gta6_archive_tree::Gta6ArchiveExplorer>();
    assert!(Arc::ptr_eq(panel.table.as_ref().unwrap(), &expected));
}

#[test]
fn a_files_expansion_arriving_after_a_tab_keeps_the_same_table() {
    let (_dir, table) = ui_tests::sample_document();
    let expected = Arc::new(table);
    let incoming = reopen(&expected);
    assert!(!Arc::ptr_eq(&incoming, &expected));
    let mut state = AppState::from_prefs(Prefs::default());
    let (doc, node) = folder(&mut state, &expected.path);
    submit_table(&mut state, doc, node, incoming);
    state.insert_document(
        expected.path.clone(),
        DocPayload::Rpf9Table(Arc::clone(&expected)),
        None,
        None,
    );
    settle(&mut state);
    assert!(Arc::ptr_eq(
        &mounted_in_folder(&state, doc, node),
        &expected
    ));
}

#[test]
fn a_tab_arriving_after_files_keeps_the_same_table() {
    let (_dir, table) = ui_tests::sample_document();
    let expected = Arc::new(table);
    let incoming = reopen(&expected);
    let mut state = AppState::from_prefs(Prefs::default());
    let (doc, node) = folder(&mut state, &expected.path);
    submit_table(&mut state, doc, node, Arc::clone(&expected));
    settle(&mut state);
    let id = deliver_tab(&mut state, incoming);
    let DocPayload::Rpf9Table(shown) = &state.document(id).unwrap().payload else {
        panic!("table tab")
    };
    assert!(Arc::ptr_eq(shown, &expected));
}

#[test]
fn an_archives_result_published_after_a_tab_reuses_the_mounted_table() {
    let (_dir, table) = ui_tests::sample_document();
    let expected = Arc::new(table);
    let incoming = reopen(&expected);
    assert!(incoming.opened_at > expected.opened_at);
    assert!(!Arc::ptr_eq(&incoming, &expected));
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        expected.path.clone(),
        DocPayload::Rpf9Table(Arc::clone(&expected)),
        None,
        None,
    );
    state
        .panels
        .get::<crate::ui::panels::gta6_archive_tree::Gta6ArchiveExplorer>()
        .show_table(Arc::clone(&incoming));
    let shared = state.share_rpf9_table(incoming);
    assert!(
        Arc::ptr_eq(&shared, &expected),
        "publishing an Archives worker result must still join the table already mounted"
    );
}

#[test]
fn a_new_file_snapshot_stays_selected_without_reviving_old_entry_sources() {
    use std::io::Write;

    let (_dir, table) = ui_tests::sample_document();
    let old = Arc::new(table);
    let source = old.entry(0, 0).unwrap();
    let mut state = AppState::from_prefs(Prefs::default());
    let (doc, node) = folder(&mut state, &old.path);
    submit_table(&mut state, doc, node, Arc::clone(&old));
    settle(&mut state);
    state.poll_table_opens();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&old.path)
        .unwrap()
        .write_all(&[0])
        .unwrap();
    assert!(old.check_current().is_err());
    let fresh = reopen(&old);
    assert!(fresh.check_current().is_ok());
    deliver_tab(&mut state, Arc::clone(&fresh));
    for _ in 0..3 {
        state.poll_table_opens();
        let selected = state.mounted_rpf9_table(&old.path).unwrap();
        assert!(Arc::ptr_eq(&selected, &fresh));
        let panel = state
            .panels
            .get::<crate::ui::panels::gta6_archive_tree::Gta6ArchiveExplorer>();
        assert!(Arc::ptr_eq(panel.table.as_ref().unwrap(), &fresh));
    }
    assert!(
        source.bytes().is_err(),
        "an old source cannot read the changed file"
    );
}

#[test]
fn serializing_a_table_archive_reports_its_read_only_reason() {
    let (_dir, table) = ui_tests::sample_document();
    let state = AppState::from_prefs(Prefs::default());
    let refused = crate::state::document_io::serialize_payload(
        &DocPayload::Rpf9Table(Arc::new(table)),
        crate::state::prefs::RpfOutputMode::Open,
        state.keys.provider(),
    )
    .unwrap_err();
    assert_eq!(
        refused,
        crate::state::document_io::never_written(DocumentKind::Rpf9).unwrap()
    );
}

fn change_file(table: &Rpf9TableDocument) {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&table.path)
        .unwrap()
        .write_all(&[0])
        .unwrap();
    assert!(table.check_current().is_err());
}

#[test]
fn expanding_files_reopens_a_changed_root_instead_of_reusing_the_old_tab() {
    let (_dir, table) = ui_tests::sample_document();
    let old = Arc::new(table);
    let source = old.entry(0, 0).unwrap();
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        old.path.clone(),
        DocPayload::Rpf9Table(Arc::clone(&old)),
        None,
        None,
    );
    let (doc, node) = folder(&mut state, &old.path);
    change_file(&old);
    state.request_expand(doc, node);
    settle(&mut state);
    let fresh = mounted_in_folder(&state, doc, node);
    assert!(!Arc::ptr_eq(&fresh, &old));
    assert!(fresh.check_current().is_ok());
    assert!(source.bytes().is_err());
    for _ in 0..3 {
        state.poll_table_opens();
        let selected = state.mounted_rpf9_table(&old.path).unwrap();
        assert!(Arc::ptr_eq(&selected, &fresh));
    }
}

#[test]
fn opening_a_tab_reopens_a_changed_root_instead_of_reusing_the_files_table() {
    let (_dir, table) = ui_tests::sample_document();
    let old = Arc::new(table);
    let source = old.entry(0, 0).unwrap();
    let mut state = AppState::from_prefs(Prefs::default());
    let (doc, node) = folder(&mut state, &old.path);
    submit_table(&mut state, doc, node, Arc::clone(&old));
    settle(&mut state);
    change_file(&old);
    state.load_file(old.path.clone());
    settle(&mut state);
    let DocPayload::Rpf9Table(fresh) = &state.active_document().unwrap().payload else {
        panic!("a fresh table tab")
    };
    assert!(!Arc::ptr_eq(fresh, &old));
    assert!(fresh.check_current().is_ok());
    assert!(source.bytes().is_err());
}
