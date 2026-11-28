//! Hermetic checks that a row of a GTA VI archive opened from the tree is named as a GTA V entry's
//! document is, on an archive built here under a synthetic white-box key.

use std::io::Cursor;
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use or_io::container::rpf::{
    write_rpf9_resource, Gta6Context, NameResolver, Rpf9Form, Rpf9Sections,
};
use or_io::container::RpfArchive;
use or_io::gta6::hash::name_hash64;

use super::*;
use crate::state::explorer::Place;
use crate::state::navigation::Location;
use crate::state::rpf_archive_open::tree_verbs::tests::{rbm_section, synthetic_keys};
use crate::state::rpf_session::RpfDocument;
use crate::state::Document;
use crate::ui::dock::Workspace;

const KEY_ID: u32 = 0xDEAD_BEEF;
const DOC: DocumentId = 9_400;
const FIRST: &str = "a/model.ydr";
const SECOND: &str = "b/model.ydr";

use crate::state::rpf_archive_open::gta6_session::{self, Reading};

/// Archives opened through [`showing`] while this is held are read with the synthetic keys and names.
fn reading() -> Reading {
    let mut names = NameResolver::new();
    for name in [FIRST, SECOND] {
        names.insert(name_hash64(name.as_bytes()), name);
    }
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    gta6_session::hold(Gta6Context {
        keys,
        names: Arc::new(names),
    })
}

/// A headerless resource's entry, which opens by a scan of its body and so needs no key to read.
fn resource(name: &str) -> (String, Rpf9NewBody) {
    let loose = write_rpf9_resource(&Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: rbm_section(),
        physical: Vec::new(),
    })
    .expect("a loose resource");
    let body = Rpf9NewBody::Resource {
        type_code: 0x02,
        loose,
    };
    (name.to_owned(), body)
}

/// An archive of two rows of one name in two folders, built from nothing as `dir/source.rpf`.
fn source(dir: &Path) -> PathBuf {
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let entries = [resource(FIRST), resource(SECOND)];
    let plan = build(&entries, or_io::platform::Platform::XboxSeries, &key, &keys)
        .expect("the archive builds");
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

/// The tree row at `inner`, its path in the archive.
fn row(state: &AppState, inner: &str) -> RpfNodeId {
    let Some(DocPayload::Rpf(rpf)) = state.document(DOC).map(|d| &d.payload) else {
        panic!("the tree is open");
    };
    let session = &rpf.session;
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

/// The document opened from the tree row `node`, by the path its body opens under.
fn opened(state: &AppState, node: RpfNodeId) -> (DocumentId, PathBuf) {
    let row = |d: &Document| d.path.as_deref().and_then(|path| row_of(state, path));
    let found = (state.documents.iter()).find(|d| row(d).map(|r| r.entry_id) == Some(node));
    let doc = found.unwrap_or_else(|| panic!("row {node} opened no document"));
    (doc.id, doc.path.clone().expect("a path"))
}

/// Two builds stand in for the Xbox and the PlayStation 5 build: two archives opened at once, each
/// with its own key-and-name session carried into the open, neither set in any global. The one entry
/// hash resolves to a different name in each session, which the tree shows, and a provider with no
/// session is refused — so each archive reads with its own keys, not a shared one. (R72f: the
/// process-wide key context is gone.)
#[test]
fn two_builds_open_at_once_each_reading_with_its_own_session() {
    use or_io::container::{NoOpCryptoProvider, RpfCryptoProvider};

    let dir = tempfile::tempdir().expect("a scratch directory");
    let bytes = {
        let key = Rpf9IndexKey {
            key_id: 0xDEAD_BEEF,
            iv: [0x33; 16],
        };
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let plan = build(
            &[resource(FIRST)],
            or_io::platform::Platform::XboxSeries,
            &key,
            &keys,
        )
        .expect("the archive builds");
        let mut bytes = Vec::new();
        (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
            .expect("the archive is laid out");
        bytes
    };
    let xbox = dir.path().join("xbox.rpf");
    let ps5 = dir.path().join("ps5.rpf");
    std::fs::write(&xbox, &bytes).expect("the xbox archive is written");
    std::fs::write(&ps5, &bytes).expect("the ps5 archive is written");

    // A session carrying the synthetic keys that resolves the one entry's hash to `shown`.
    let session = |shown: &str| -> Arc<dyn RpfCryptoProvider> {
        let mut names = NameResolver::new();
        names.insert(name_hash64(FIRST.as_bytes()), shown);
        Arc::new(Gta6Context {
            keys: synthetic_keys().expect("the synthetic snapshot parses"),
            names: Arc::new(names),
        })
    };

    // Opened back-to-back, each with its own session; no hold, no global.
    let a =
        RpfArchive::open_with(&xbox, session("xbox/model.ydr")).expect("the xbox archive opens");
    let b = RpfArchive::open_with(&ps5, session("ps5/model.ydr")).expect("the ps5 archive opens");

    let path_of = |archive: &RpfArchive| -> String {
        (archive.nodes.iter().find(|n| !n.is_dir()))
            .map(|n| n.path().to_owned())
            .expect("one file row")
    };
    // Each archive reads with the names of the session it was opened with, the other's never set.
    assert_eq!(path_of(&a), "xbox/model.ydr");
    assert_eq!(path_of(&b), "ps5/model.ydr");

    // The keys are the open's, not a global's: with no session the same archive is refused.
    let refused = RpfArchive::open_with(&xbox, Arc::new(NoOpCryptoProvider::new()));
    assert!(
        refused.is_err(),
        "an archive opened with no session is refused"
    );
}

#[test]
fn a_row_opened_from_the_tree_is_named_by_its_path_in_the_archive_and_its_save_finds_the_row() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let mut state = showing(&disk);
    let (first, second) = (row(&state, FIRST), row(&state, SECOND));
    state.open_rpf_entry(DOC, first);
    state.open_rpf_entry(DOC, second);
    settle(&mut state);

    let (one, one_path) = opened(&state, first);
    let (two, two_path) = opened(&state, second);
    assert_ne!(
        one, two,
        "two rows of one name in two folders have two tabs"
    );
    for (id, inner) in [(one, FIRST), (two, SECOND)] {
        let doc = state.document(id).expect("open");
        assert_eq!(
            doc.file_name(),
            "model.ydr",
            "Save as offers the row's name"
        );
        assert_eq!(doc.name, "model");
        assert_eq!(state.tab_title(doc), "model.ydr — source.rpf");
        assert!(
            Path::new(&doc.location()).ends_with(inner),
            "the tab's hover names the row's path inside the archive: {}",
            doc.location()
        );
        let place = state.place_of(Location::document(id));
        let expected = Place::new(&disk, inner);
        assert_eq!(
            place, expected,
            "the recents entry walks the tree to the row"
        );
    }
    state.active_document = Some(one);
    assert!(state.window_title().starts_with("model.ydr"));

    // The save reads the archive and the row's body from the path the body opened under.
    for (id, path) in [(one, &one_path), (two, &two_path)] {
        let (archive, offset) = crate::state::rpf9_scan::scanned_body(path).expect("its archive");
        assert_eq!(archive, disk.as_path());
        let (find, _) = crate::state::rpf9_scan::scan_origin(path).expect("its read is kept");
        assert_eq!(find.offset, offset, "the body read is the row's");
        state.active_document = Some(id);
        assert_eq!(state.can_save_into_archive_copy(), Ok(()));
        assert_eq!(
            state.archive_copy_name().as_deref(),
            Some("source_edited.rpf")
        );
    }
    assert_ne!(one_path, two_path, "each row's save finds its own body");
}

#[test]
fn a_rows_preview_tab_is_named_by_the_row() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let mut state = showing(&disk);
    state.active_workspace = Workspace::Model;
    let second = row(&state, SECOND);
    state.preview_row(DOC, second);
    settle(&mut state);
    let (id, _) = opened(&state, second);
    assert!(state.is_preview(id), "in the preview tab");
    let doc = state.document(id).expect("open");
    assert_eq!(state.tab_title(doc), "model.ydr — source.rpf");
}

/// A few small rows of the build's smallest archive, opened from the tree, are named by their row
/// and never by an offset. Install-gated: it needs the key snapshot and the build.
#[test]
fn rows_of_a_real_archive_opened_from_the_tree_are_named_by_their_rows() {
    const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
    const WHAT: &str = "gta6 tree row names";
    const ROWS: usize = 3;
    let build = (std::env::var_os(BUILD_ENV).map(PathBuf::from)).filter(|p| p.is_dir());
    let keys = build
        .as_ref()
        .and_then(|_| crate::state::rpf9_scan::rpf9_table::keys().ok());
    let (Some(build), Some(keys)) = (build, keys) else {
        let why = format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}");
        or_test_support::skip_install(WHAT, why);
        return;
    };
    let _serial = crate::ui::theme::serial();
    let names = crate::state::rpf9_scan::rpf9_table::resolver();
    let _reading = gta6_session::hold(Gta6Context { keys, names });
    let mut archives: Vec<(u64, PathBuf)> = (std::fs::read_dir(&build).into_iter().flatten())
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
        .collect();
    archives.sort();
    for (_, path) in &archives {
        let crypto = gta6_session::crypto();
        let Ok(tree) = (RpfArchive::open_with(path, crypto).map_err(|e| e.to_string()))
            .and_then(RpfDocument::new)
        else {
            continue;
        };
        let session = &tree.session;
        let small: Vec<RpfNodeId> = (session.all_nodes().into_iter())
            .filter(|&id| {
                let Some(node) = session.node(id).filter(|n| !n.is_dir()) else {
                    return false;
                };
                let crate::state::RpfNodeBacking::ArchiveFile {
                    archive_id,
                    entry_index,
                    ..
                } = &node.backing
                else {
                    return false;
                };
                (session.archive(*archive_id))
                    .and_then(|a| a.archive.binary_entry(*entry_index))
                    .is_some_and(|row| (1..=0x10000).contains(&row.uncompressed_size))
            })
            .take(ROWS)
            .collect();
        if small.is_empty() {
            continue;
        }
        let mut state = AppState::new();
        let doc = Document::new(DOC, Some(path.clone()), DocPayload::Rpf(tree));
        state.documents.push(doc);
        for &node in &small {
            state.open_rpf_entry(DOC, node);
        }
        settle(&mut state);
        // Names are game-derived, so a row is reported by its place in the list.
        let mut named = 0usize;
        for (at, &node) in small.iter().enumerate() {
            let Some(DocPayload::Rpf(rpf)) = state.document(DOC).map(|d| &d.payload) else {
                panic!("the tree is open");
            };
            let name = (rpf.session.node(node).map(|n| n.name.clone())).expect("the row");
            let row = |d: &Document| d.path.as_deref().and_then(|p| row_of(&state, p));
            let Some(doc) =
                (state.documents.iter()).find(|d| row(d).map(|r| r.entry_id) == Some(node))
            else {
                continue;
            };
            assert!(
                doc.file_name() == name,
                "row {at} of the smallest archive is named by its row"
            );
            assert!(state.tab_title(doc).starts_with(&name), "row {at}'s tab");
            named += 1;
        }
        assert!(
            named > 0,
            "no row of the smallest archive opened from its tree"
        );
        return;
    }
    or_test_support::skip_install(WHAT, format_args!("no small row in {BUILD_ENV}"));
}
