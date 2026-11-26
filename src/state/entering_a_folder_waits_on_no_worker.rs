//! Entering a folder of an open archive draws what its listing already holds: the thread that
//! draws opens and lists no archive, so it never reaches the GTA VI keys, the names or a table
//! read that a worker may be holding, and a nested archive is opened on a worker.

use std::time::{Duration, Instant};

use or_io::container::rpf::RpfBuilder;

use super::archive_work_on_this_thread;
use crate::state::navigation::Location;
use crate::state::rpf_session::RpfEditSession;
use crate::state::the_files_view_in_the_centre::{accessible, frame, listed, node, settled};
use crate::state::{AppState, DocPayload, DocumentId, DocumentKind, Prefs};

fn archive_of(entries: &[(&str, &[u8])]) -> RpfBuilder {
    let mut builder = RpfBuilder::default();
    for (path, bytes) in entries {
        builder.insert_with(path, *bytes, false).expect("add");
    }
    builder
}

/// An archive in Browse: `data` holding two files and two nested archives, and a file beside it.
fn browsing_nested() -> (AppState, DocumentId) {
    let inner = archive_of(&[("deep.txt", b"deep")])
        .to_bytes()
        .expect("inner");
    let other = archive_of(&[("far.txt", b"far")])
        .to_bytes()
        .expect("other");
    let archive = archive_of(&[
        ("data/a.txt", b"a"),
        ("data/b.txt", b"b"),
        ("data/inner.rpf", &inner),
        ("data/other.rpf", &other),
        ("top.txt", b"top"),
    ])
    .build()
    .expect("opens");
    let mut state = AppState::from_prefs(Prefs::default());
    let doc = state.new_empty_document(DocumentKind::Rpf);
    let Some(DocPayload::Rpf(rpf)) = state.document_mut(doc).map(|d| &mut d.payload) else {
        panic!("an empty archive document holds an archive");
    };
    rpf.session = RpfEditSession::from_archive(archive).expect("the session");
    state.set_active_document(doc);
    state.prefs.explorer = Default::default();
    (state, doc)
}

#[test]
fn entering_a_folder_and_a_nested_archive_opens_and_lists_nothing_on_the_thread_that_draws() {
    let (mut state, doc) = browsing_nested();
    let ctx = accessible();
    settled(&ctx, &mut state);
    let before = archive_work_on_this_thread();

    let data = node(&state, doc, "data");
    state.navigate_to(Location { doc, node: data });
    settled(&ctx, &mut state);
    let mut names = listed(&state, doc);
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt", "inner.rpf", "other.rpf"]);
    assert_eq!(
        archive_work_on_this_thread(),
        before,
        "entering a folder opened or listed an archive on the thread that draws"
    );

    let inner = node(&state, doc, "data/inner.rpf");
    state.navigate_to(Location { doc, node: inner });
    let until = Instant::now() + Duration::from_secs(10);
    while listed(&state, doc) != ["deep.txt"] {
        assert!(
            Instant::now() < until,
            "the nested archive was never listed"
        );
        state.poll_expansions();
        frame(&ctx, &mut state, Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        archive_work_on_this_thread(),
        before,
        "the nested archive was opened on the thread that draws, not on a worker"
    );

    // The count sees an open on this thread: an edit into a row no listing opened opens it here.
    let other = node(&state, doc, "data/other.rpf");
    state.open_archive_row(doc, other).expect("opens");
    assert!(
        archive_work_on_this_thread() > before,
        "the count saw no open"
    );
}
