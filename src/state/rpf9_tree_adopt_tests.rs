//! A GTA VI tree that became its copy shows each document opened from its rows under the copy, and
//! such a document's next save lands in the copy's tree: hermetic, on an archive built here under a
//! synthetic white-box key.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use or_io::container::rpf::{Gta6Context, NameResolver};
use or_io::container::RpfArchive;
use or_io::gta6::hash::name_hash64;

use super::{become_copy, listed, reopened};
use crate::state::document::{DocumentId, RpfNodeId};
use crate::state::rpf9_scan::{body_path, View};
use crate::state::rpf_archive_open::tree_open::{leave, name, row_of};
use crate::state::rpf_archive_open::tree_verbs::tests::synthetic_keys;
use crate::state::rpf_session::{RpfDocument, RpfEditSession};
use crate::state::{AppState, DocPayload, Document, RpfNodeBacking, ToastType};

const KEY_ID: u32 = 0xDEAD_BEEF;
const DOC: DocumentId = 9_700;
const NOTES: &str = "data/notes.txt";

/// A text body whose first letter an edit lowers, so the edited body deflates no longer.
fn notes() -> Vec<u8> {
    let mut text = b"Line of the notes, kept as plain text\r\n".to_vec();
    text.extend(b"line of the notes, kept as plain text\r\n".repeat(30));
    text
}

use crate::state::rpf_archive_open::gta6_session::{self, Reading};

/// Archives opened through [`showing`] while this is held are read with the synthetic keys and the row's name.
fn reading() -> Reading {
    let mut names = NameResolver::new();
    names.insert(name_hash64(NOTES.as_bytes()), NOTES);
    gta6_session::hold(Gta6Context {
        keys: synthetic_keys().expect("the synthetic snapshot parses"),
        names: Arc::new(names),
    })
}

/// An archive of the text body as `dir/source.rpf`.
fn source(dir: &Path) -> PathBuf {
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let entries = [(NOTES.to_owned(), Rpf9NewBody::Chunked(notes()))];
    let plan = build(&entries, &key, &keys).expect("the archive builds");
    let mut bytes = Vec::new();
    (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
        .expect("the archive is laid out");
    let path = dir.join("source.rpf");
    std::fs::write(&path, bytes).expect("the archive is written");
    path
}

fn showing(path: &Path) -> AppState {
    let crypto = gta6_session::crypto();
    let archive = RpfArchive::open_with(path, crypto).expect("the archive opens as a tree");
    let tree = RpfDocument::new(archive).expect("the tree lists");
    let mut state = AppState::new();
    let doc = Document::new(DOC, Some(path.to_path_buf()), DocPayload::Rpf(tree));
    state.documents.push(doc);
    state.active_document = Some(DOC);
    state.toasts.clear();
    state
}

fn session(state: &AppState) -> &RpfEditSession {
    match state.document(DOC).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => &rpf.session,
        _ => panic!("the tree is open"),
    }
}

fn row(state: &AppState, inner: &str) -> RpfNodeId {
    let session = session(state);
    (session.all_nodes().into_iter())
        .find(|&id| session.path_of(id).as_deref() == Some(inner))
        .unwrap_or_else(|| panic!("no row {inner}"))
}

/// Open `node` as the tree's door does, its table read with the synthetic keys.
fn opened(state: &mut AppState, node: RpfNodeId) -> DocumentId {
    let (place, start) = {
        let session = session(state);
        let Some(RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            ..
        }) = session.node(node).map(|n| &n.backing)
        else {
            panic!("row {node} is a file of the archive");
        };
        let archive = &session.archive(*archive_id).expect("its archive").archive;
        let place = archive.rpf9_place(*entry_index).expect("a GTA VI row");
        let entry = archive.binary_entry(*entry_index).expect("its row");
        let start = place.base + entry.file_offset;
        (place, start)
    };
    let path = body_path(&place.file, place.offset, View::Bytes);
    leave(&path, &place, start, synthetic_keys, None);
    name(state, &path, DOC, node);
    state.load_file(path.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.load_receiver.is_some() || !state.pending_loads.is_empty() {
        state.poll_load_result();
        assert!(Instant::now() < deadline, "the load never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    (state.documents.iter())
        .find(|d| d.path.as_deref() == Some(path.as_path()))
        .map(|d| d.id)
        .unwrap_or_else(|| panic!("row {node} opened: {}", state.error_dialog_message))
}

/// Whether a save has said how it ended.
fn ended(state: &AppState) -> bool {
    (state.toasts.iter()).any(|t| matches!(t.toast_type, ToastType::Success | ToastType::Error))
}

fn toasts(state: &AppState) -> Vec<&str> {
    state.toasts.iter().map(|t| t.message.as_str()).collect()
}

#[test]
fn a_row_s_document_follows_its_tree_into_the_copy_and_its_next_save_lands_in_the_copy_s_tree() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let mut state = showing(&disk);
    let node = row(&state, NOTES);
    let doc = opened(&mut state, node);
    let key = (state.document(doc))
        .and_then(|d| d.path.clone())
        .expect("a path");
    let before = PathBuf::from(state.document(doc).expect("open").location());
    assert!(before.starts_with(&disk), "{}", before.display());

    // A byte-identical copy keeps every table span, as a patched copy does.
    let out = dir.path().join("copy.rpf");
    std::fs::copy(&disk, &out).expect("the copy is written");
    let chains = listed(session(&state));
    let written = reopened(&out, gta6_session::crypto(), &chains).expect("the copy reopens");
    let tree = state.document_mut(DOC).expect("the tree");
    assert!(
        become_copy(tree, written, &out),
        "the tree becomes the copy"
    );

    let d = state.document(doc).expect("open");
    let shown = PathBuf::from(d.location());
    assert!(
        shown.starts_with(&out),
        "the path line names the copy: {}",
        shown.display()
    );
    assert!(
        shown.ends_with(NOTES),
        "and the row inside it: {}",
        shown.display()
    );
    assert_eq!(state.tab_title(d), "notes.txt — copy.rpf");
    let bound = row_of(&state, &key).expect("still bound to its row");
    assert_eq!((bound.parent_doc_id, bound.entry_id), (DOC, node));

    let d = state.document_mut(doc).expect("open");
    let DocPayload::Text(text) = &mut d.payload else {
        panic!("a text document");
    };
    text.text.remove(0..1);
    text.text.insert_char(0, 'l');
    d.mark_edited();
    state.active_document = Some(doc);
    state.toasts.clear();
    assert!(state.save_active_document(), "{:?}", toasts(&state));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ended(&state) {
        state.poll_save_result();
        assert!(Instant::now() < deadline, "the save never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let said = toasts(&state);
    assert!(
        said.iter()
            .any(|t| t.contains("saved into its row of copy.rpf")),
        "{said:?}"
    );
    let session = session(&state);
    let Some(RpfNodeBacking::ArchiveFile {
        archive_id,
        override_bytes: Some(body),
        ..
    }) = session.node(node).map(|n| &n.backing)
    else {
        panic!("the copy's row holds the save");
    };
    let mut edited = notes();
    edited[0] = b'l';
    assert_eq!(body.to_vec(), edited, "the row holds the edited text");
    let slot = session.archive(*archive_id).expect("its archive");
    assert_eq!(
        slot.fs_path.as_deref(),
        Some(out.as_path()),
        "the row is the copy's"
    );
}
