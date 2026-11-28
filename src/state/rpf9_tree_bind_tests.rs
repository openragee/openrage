//! A document opened from a GTA VI tree's row and saved lands in that row, and the row then plans
//! as a patched copy: hermetic checks on an archive built here under a synthetic white-box key, and
//! one install-gated check on a small row of the build.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use or_io::container::rpf::{Gta6Context, Gta6Mount, GtaViKeys, NameResolver, Rpf9ArchiveWriter};
use or_io::container::{NoOpCryptoProvider, RpfArchive};
use or_io::gta6::hash::name_hash64;
use or_io::meta::Value;

use super::super::{leave, name, row_of};
use crate::state::document::{DocumentId, RpfNodeId};
use crate::state::rpf9_scan::{body_path, View};
use crate::state::rpf_archive_open::tree_verbs::tests::{synthetic_keys, table_form_pso};
use crate::state::rpf_archive_open::tree_verbs::{
    chains, plan_save, write_planned, Planned, Replaced, Saved, Writes,
};
use crate::state::rpf_session::RpfDocument;
use crate::state::{AppState, DocPayload, Document, RpfNodeBacking, ToastType};

const KEY_ID: u32 = 0xDEAD_BEEF;
const DOC: DocumentId = 9_600;
const NOTES: &str = "data/notes.txt";
const PSO: &str = "data/table.pso";

/// A text body whose first letter an edit lowers to the case every other line has, so the edited
/// body deflates no longer than the stored one.
fn notes() -> Vec<u8> {
    let mut text = b"Line of the notes, kept as plain text\r\n".to_vec();
    text.extend(b"line of the notes, kept as plain text\r\n".repeat(30));
    text
}

/// Archives opened while this is held are read with the keys and names it was made with.
struct Reading(Option<Arc<Gta6Context>>);

impl Drop for Reading {
    fn drop(&mut self) {
        Gta6Context::swap_given(self.0.take());
    }
}

fn reading(keys: Arc<GtaViKeys>, named: &[&str]) -> Reading {
    let mut names = NameResolver::new();
    for name in named {
        names.insert(name_hash64(name.as_bytes()), *name);
    }
    let context = Gta6Context {
        keys,
        names: Arc::new(names),
    };
    Reading(Gta6Context::swap_given(Some(Arc::new(context))))
}

/// An archive of the text body and the PSO body, each under a 24-byte row, as `dir/source.rpf`.
fn source(dir: &Path) -> PathBuf {
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let entries = [
        (NOTES.to_owned(), Rpf9NewBody::Chunked(notes())),
        (PSO.to_owned(), Rpf9NewBody::Chunked(table_form_pso())),
    ];
    let plan = build(&entries, or_io::platform::Platform::XboxSeries, &key, &keys)
        .expect("the archive builds");
    let mut bytes = Vec::new();
    (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
        .expect("the archive is laid out");
    let path = dir.join("source.rpf");
    std::fs::write(&path, bytes).expect("the archive is written");
    path
}

/// An app showing the archive at `path` as the tree every archive opens as.
fn showing(path: &Path) -> AppState {
    let crypto = Arc::new(NoOpCryptoProvider::new());
    let archive = RpfArchive::open_with(path, crypto).expect("the archive opens as a tree");
    let tree = RpfDocument::new(archive).expect("the tree lists");
    let mut state = AppState::new();
    let doc = Document::new(DOC, Some(path.to_path_buf()), DocPayload::Rpf(tree));
    state.documents.push(doc);
    state.active_document = Some(DOC);
    state.toasts.clear();
    state
}

fn session(state: &AppState) -> &crate::state::rpf_session::RpfEditSession {
    match state.document(DOC).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => &rpf.session,
        _ => panic!("the tree is open"),
    }
}

/// The tree row at `inner`, its path in the archive.
fn row(state: &AppState, inner: &str) -> RpfNodeId {
    let session = session(state);
    (session.all_nodes().into_iter())
        .find(|&id| session.path_of(id).as_deref() == Some(inner))
        .unwrap_or_else(|| panic!("no row {inner}"))
}

fn settle(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.load_receiver.is_some() || !state.pending_loads.is_empty() {
        state.poll_load_result();
        assert!(Instant::now() < deadline, "the load never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Open `node` as the tree's door does, its table read with the synthetic keys, and the document.
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
        let start = place.base
            + archive
                .binary_entry(*entry_index)
                .expect("its row")
                .file_offset;
        (place, start)
    };
    let path = body_path(&place.file, place.offset, View::Bytes);
    leave(&path, &place, start, synthetic_keys, None);
    name(state, &path, DOC, node);
    state.load_file(path.clone());
    settle(state);
    (state.documents.iter())
        .find(|d| d.path.as_deref() == Some(path.as_path()))
        .map(|d| d.id)
        .unwrap_or_else(|| panic!("row {node} opened: {}", state.error_dialog_message))
}

/// Poll until a save says how it ended.
fn saved(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        state.poll_save_result();
        let ended = (state.toasts.iter())
            .any(|t| matches!(t.toast_type, ToastType::Success | ToastType::Error));
        if ended {
            return;
        }
        assert!(Instant::now() < deadline, "the save never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn toasts(state: &AppState) -> Vec<&str> {
    state.toasts.iter().map(|t| t.message.as_str()).collect()
}

/// Lower the first letter of the text document `doc` in place, keeping its length.
fn lower_first_letter(state: &mut AppState, doc: DocumentId) {
    let d = state.document_mut(doc).expect("open");
    let DocPayload::Text(text) = &mut d.payload else {
        panic!("a text document");
    };
    let at = (text.text.chars())
        .position(|c| c.is_ascii_uppercase())
        .expect("an upper-case letter");
    let lowered = text.text.char(at).to_ascii_lowercase();
    text.text.remove(at..at + 1);
    text.text.insert_char(at, lowered);
    d.mark_edited();
}

/// The body the tree's row `node` holds, and what a save of the tree plans for it with `keys`.
fn planned(state: &AppState, disk: &Path, node: RpfNodeId, keys: &GtaViKeys) -> (Vec<u8>, Planned) {
    let session = session(state);
    let Some(RpfNodeBacking::ArchiveFile {
        archive_id,
        entry_index,
        override_bytes: Some(body),
    }) = session.node(node).map(|n| &n.backing)
    else {
        panic!("row {node} holds the saved body");
    };
    let archive = &session.archive(*archive_id).expect("its archive").archive;
    let at = archive
        .rpf9_place(*entry_index)
        .expect("a GTA VI row")
        .offset;
    let mut file = File::open(disk).expect("the archive");
    let mut found = chains(&mut file, &BTreeSet::from([at]), keys).expect("the row is found");
    drop(file);
    let replaced = Replaced {
        name: session
            .node(node)
            .map(|n| n.name.clone())
            .unwrap_or_default(),
        at: found.remove(&at).expect("the row's place"),
        body: body.to_vec(),
    };
    let writer = Rpf9ArchiveWriter::new();
    let plan = plan_save(disk, keys, &[replaced], Vec::new(), writer).expect("the save plans");
    (body.to_vec(), plan)
}

/// The body of the top-level entry `name` of the archive at `path`.
fn body_of(path: &Path, name: &str, keys: &GtaViKeys) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("the archive");
    let mount = Gta6Mount::open_root(&mut Cursor::new(&bytes), keys).expect("it opens");
    let entry = (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(name.as_bytes()))
        .unwrap_or_else(|| panic!("no entry {name}"));
    (mount.read_entry(Cursor::new(&bytes), entry)).expect("the body reads")
}

/// Save the document `doc` and check it landed in the row `node` and nowhere on disk.
fn saved_into_its_row(state: &mut AppState, dir: &Path, doc: DocumentId, node: RpfNodeId) {
    state.active_document = Some(doc);
    state.toasts.clear();
    assert!(state.save_active_document(), "{:?}", toasts(state));
    saved(state);
    let into = [
        "saved into its row of source.rpf",
        "patched copy",
        "rebuilt archive",
    ];
    let said = toasts(state);
    assert!(
        said.iter().any(|t| into.iter().all(|w| t.contains(w))),
        "{said:?}"
    );
    let d = state.document(doc).expect("open");
    assert!(!d.dirty, "the document is saved");
    assert_eq!(
        d.rpf_entry_binding.map(|b| b.entry_id),
        Some(node),
        "bound to its row"
    );
    assert!(
        state.document(DOC).is_some_and(|t| t.dirty),
        "the tree holds an edit"
    );
    let backing = session(state).node(node).map(|n| &n.backing);
    assert!(
        matches!(
            backing,
            Some(RpfNodeBacking::ArchiveFile {
                override_bytes: Some(_),
                ..
            })
        ),
        "the row shows as edited"
    );
    let mut files: Vec<_> = (std::fs::read_dir(dir).expect("the folder lists"))
        .map(|e| e.expect("an entry").file_name())
        .collect();
    files.sort();
    assert_eq!(files, ["source.rpf"], "a save into a row writes no file");
}

#[test]
fn an_edited_text_body_opened_from_the_tree_saves_into_its_row_which_plans_a_patched_copy() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading(Arc::clone(&keys), &[NOTES, PSO]);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let before = std::fs::read(&disk).expect("the source");
    let mut state = showing(&disk);
    let node = row(&state, NOTES);
    let doc = opened(&mut state, node);
    let path = (state.document(doc))
        .and_then(|d| d.path.clone())
        .expect("a path");
    assert_eq!(row_of(&state, &path).map(|r| r.entry_id), Some(node));

    // An unedited document has nothing for its row, and says so.
    state.toasts.clear();
    assert!(!state.save_active_document());
    assert!(
        toasts(&state)
            .iter()
            .any(|t| t.contains("Not saved into its row")),
        "{:?}",
        toasts(&state)
    );

    lower_first_letter(&mut state, doc);
    saved_into_its_row(&mut state, dir.path(), doc, node);
    let mut edited = notes();
    edited[0] = b'l';
    let (body, plan) = planned(&state, &disk, node, &keys);
    assert_eq!(body, edited, "the row holds the edited text");
    assert_eq!(plan.writes(), Writes::Patched, "{:?}", plan.kept);

    let out = dir.path().join("copy.rpf");
    let written = write_planned(&disk, &out, &keys, &plan).expect("the copy is written");
    assert_eq!(written, Saved::Patched);
    assert_eq!(
        body_of(&out, NOTES, &keys),
        edited,
        "the copy reads the edit back"
    );
    assert_eq!(body_of(&out, PSO, &keys), table_form_pso());
    assert_eq!(std::fs::read(&disk).expect("the source"), before);

    // The row opens the document it holds again rather than a second one of its new bytes.
    let open = state.documents.len();
    state.open_rpf_entry(DOC, node);
    settle(&mut state);
    assert_eq!(state.documents.len(), open);
    assert_eq!(state.active_document, Some(doc));

    // Closed and opened again, the row opens the body it holds under the same kind, still bound to
    // the row, so a second save lands there too.
    state.close_document(doc);
    assert!(state.document(doc).is_none(), "the saved document closes");
    state.open_rpf_entry(DOC, node);
    settle(&mut state);
    let again = (state.documents.iter())
        .find(|d| d.path.as_deref() == Some(path.as_path()))
        .map(|d| d.id)
        .expect("the row opens again");
    match state.document(again).map(|d| &d.payload) {
        Some(DocPayload::Text(text)) => assert_eq!(text.encode(), edited),
        _ => panic!("the row opens as text again"),
    }
    assert_eq!(row_of(&state, &path).map(|r| r.entry_id), Some(node));
    let d = state.document_mut(again).expect("open");
    let DocPayload::Text(text) = &mut d.payload else {
        panic!("a text document");
    };
    text.text.remove(0..1);
    text.text.insert_char(0, 'L');
    d.mark_edited();
    std::fs::remove_file(&out).expect("the earlier copy goes");
    saved_into_its_row(&mut state, dir.path(), again, node);
    let (body, _) = planned(&state, &disk, node, &keys);
    assert_eq!(body, notes(), "the second save lands in the row");
}

/// A row opened from a folder's tree is left to the read-only refusal: no folder's tree writes a
/// GTA VI archive, so nothing may say the edit is saved into its row.
#[test]
fn a_row_whose_tree_is_a_folder_s_is_not_saved_into_its_row() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading(Arc::clone(&keys), &[NOTES, PSO]);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let mut state = showing(&disk);
    let node = row(&state, NOTES);
    let doc = opened(&mut state, node);
    lower_first_letter(&mut state, doc);
    // The same row, its tree now a folder's, as a GTA VI install opened as a folder shows it.
    let crypto = Arc::new(NoOpCryptoProvider::new());
    let folder = RpfDocument::new_folder(dir.path().to_path_buf(), crypto);
    state.document_mut(DOC).expect("the tree").payload = DocPayload::Rpf(folder);
    state.active_document = Some(doc);
    state.toasts.clear();
    assert_eq!(state.save_into_tree_row(doc), None);
    assert!(!state.save_active_document());
    state.poll_save_result();
    let d = state.document(doc).expect("open");
    assert!(d.dirty, "the edit is not saved");
    assert!(d.rpf_entry_binding.is_none(), "nor bound to a row");
    assert!(
        !toasts(&state)
            .iter()
            .any(|t| t.contains("saved into its row")),
        "{:?}",
        toasts(&state)
    );
}

#[test]
fn an_edited_metadata_body_opened_from_the_tree_saves_into_its_row_which_plans_a_patched_copy() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading(Arc::clone(&keys), &[NOTES, PSO]);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let mut state = showing(&disk);
    let node = row(&state, PSO);
    let doc = opened(&mut state, node);
    let Some(DocPayload::Meta(meta)) = state.document_mut(doc).map(|d| &mut d.payload) else {
        panic!("a metadata document");
    };
    let parsed = meta.parsed.as_mut().expect("it parsed");
    assert_eq!(parsed.instance.fields[0].1, Value::UInt(77));
    parsed.instance.fields[0].1 = Value::UInt(0);
    meta.after_visual_edit_in_place();

    saved_into_its_row(&mut state, dir.path(), doc, node);
    let (body, plan) = planned(&state, &disk, node, &keys);
    assert_eq!(
        body.len(),
        table_form_pso().len(),
        "a value edit keeps the length"
    );
    let reread = or_io::meta::MetaDocument::parse(&Default::default(), body.clone())
        .expect("the row's body reads");
    assert_eq!(reread.instance.fields[0].1, Value::UInt(0));
    assert_eq!(plan.writes(), Writes::Patched, "{:?}", plan.kept);

    let out = dir.path().join("copy.rpf");
    let written = write_planned(&disk, &out, &keys, &plan).expect("the copy is written");
    assert_eq!(written, Saved::Patched);
    assert_eq!(
        body_of(&out, PSO, &keys),
        body,
        "the copy reads the edit back"
    );
}

/// A small clear text row of one of the build's smallest archives, opened from the tree, edited at
/// its length and saved, lands in its row, and a patched copy of it under a temp dir reads the edit
/// back. Install-gated: it needs the key snapshot and the build, and skips without them.
#[test]
fn a_text_row_of_the_build_saved_from_its_document_reads_back_from_a_patched_copy() {
    const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
    const WHAT: &str = "gta6 document saved into its tree row";
    const ARCHIVES: usize = 8;
    const OPENS: usize = 48;
    let build = (std::env::var_os(BUILD_ENV).map(PathBuf::from)).filter(|p| p.is_dir());
    let keys = (build.as_ref()).and_then(|_| crate::state::rpf9_scan::rpf9_table::keys().ok());
    let (Some(build), Some(keys)) = (build, keys) else {
        or_test_support::skip_install(WHAT, format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}"));
        return;
    };
    let _serial = crate::ui::theme::serial();
    let _reading = reading(Arc::clone(&keys), &[]);
    let mut archives: Vec<(u64, PathBuf)> = (std::fs::read_dir(&build).into_iter().flatten())
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
        .collect();
    archives.sort();
    let mut opens = 0usize;
    for (_, disk) in archives.iter().take(ARCHIVES) {
        let crypto = Arc::new(NoOpCryptoProvider::new());
        let Ok(tree) = (RpfArchive::open_with(disk, crypto).map_err(|e| e.to_string()))
            .and_then(RpfDocument::new)
        else {
            continue;
        };
        let mut state = AppState::new();
        state.documents.push(Document::new(
            DOC,
            Some(disk.clone()),
            DocPayload::Rpf(tree),
        ));
        // Small clear rows of the top archive whose body starts at its row, as text is stored.
        let rows: Vec<(RpfNodeId, u32)> = {
            let session = session(&state);
            (session.all_nodes().into_iter())
                .filter_map(|id| {
                    let RpfNodeBacking::ArchiveFile {
                        archive_id,
                        entry_index,
                        ..
                    } = &session.node(id).filter(|n| !n.is_dir())?.backing
                    else {
                        return None;
                    };
                    let archive = &session.archive(*archive_id)?.archive;
                    let place = archive.rpf9_place(*entry_index)?;
                    let row = archive.binary_entry(*entry_index)?;
                    let small = (1..=0x10000).contains(&row.uncompressed_size);
                    let plain =
                        place.base == 0 && !place.ciphered && place.offset == row.file_offset;
                    (small && plain).then_some((id, place.entry))
                })
                .collect()
        };
        for (node, entry) in rows {
            if opens >= OPENS {
                break;
            }
            opens += 1;
            state.open_rpf_entry(DOC, node);
            settle(&mut state);
            let found = (state.documents.iter()).find(|d| {
                d.path
                    .as_deref()
                    .and_then(|p| row_of(&state, p))
                    .map(|r| r.entry_id)
                    == Some(node)
            });
            let Some(doc) = found.map(|d| d.id) else {
                continue;
            };
            let text = match state.document(doc).map(|d| (&d.payload, d.path.as_deref())) {
                Some((DocPayload::Text(text), Some(path)))
                    if crate::state::rpf9_scan::opened_as_text(path) =>
                {
                    text.text.to_string()
                }
                _ => continue,
            };
            // Swap the case of the first ASCII letter: an edit at the body's length.
            let Some(at) = text.chars().position(|c| c.is_ascii_alphabetic()) else {
                continue;
            };
            let d = state.document_mut(doc).expect("open");
            let DocPayload::Text(held) = &mut d.payload else {
                continue;
            };
            let c = held.text.char(at);
            let swapped = if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            };
            held.text.remove(at..at + 1);
            held.text.insert_char(at, swapped);
            d.mark_edited();
            state.active_document = Some(doc);
            state.toasts.clear();
            assert!(state.save_active_document(), "{:?}", toasts(&state));
            saved(&mut state);
            assert!(
                !(state.toasts.iter()).any(|t| matches!(t.toast_type, ToastType::Error)),
                "a row's text saves into its row: {:?}",
                toasts(&state)
            );
            assert_eq!(
                state
                    .document(doc)
                    .and_then(|d| d.rpf_entry_binding)
                    .map(|b| b.entry_id),
                Some(node)
            );
            let (body, plan) = planned(&state, disk, node, &keys);
            // A changed chunk that no deflater fits under its ceiling plans a rebuild; try another row.
            if plan.writes() != Writes::Patched {
                assert!(
                    plan.kept.iter().all(|why| why.contains("deflates at best")),
                    "rebuilt for another reason than a chunk that does not fit: {:?}",
                    plan.kept
                );
                continue;
            }
            let dir = tempfile::tempdir().expect("a scratch directory");
            let out = dir.path().join("copy.rpf");
            let written = write_planned(disk, &out, &keys, &plan).expect("the copy is written");
            assert_eq!(written, Saved::Patched);
            let copy = std::fs::read(&out).expect("the copy");
            let mount = Gta6Mount::open_root(&mut Cursor::new(&copy), &keys).expect("it opens");
            let held = (mount.archive.entries.iter())
                .find(|e| e.index == entry)
                .expect("the entry is kept");
            let read = (mount.read_entry(Cursor::new(&copy), held)).expect("the body reads");
            assert_eq!(read, body, "the copy reads the edit back");
            return;
        }
    }
    or_test_support::skip_install(
        WHAT,
        format_args!("no small text row whose edit fits in place within {OPENS} opens"),
    );
}
