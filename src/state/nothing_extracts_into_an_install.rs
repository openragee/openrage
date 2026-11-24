//! The edit gate stands in front of an extraction, and one job model runs both.
//!
//! Extraction is the one write in the editor whose target a user aims
//! freely: every other one lands on a document's own path or inside an archive
//! already open. A folder picker will happily point at `<install>/x64/`, so the
//! gate has to be asked about the **destination**.
//!
//! ⚠ An extractor that writes nothing at all satisfies every assertion about a
//! refusal. So each case below runs the *same* call with the *same* fixture and
//! varies only where it points, and the permitted case asserts the bytes that
//! landed. A no-op implementation fails the first half; an ungated one fails
//! the second.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::container::rpf::BodyForm;

use super::document::{Document, DocumentId, RpfNodeId};
use super::extract_job::{ExtractStart, TypeFilter};
use super::prefs::EditMode;
use super::rpf_session::{RpfDocument, RpfEditSession};
use super::{AppState, DocPayload, PendingRetailEdit, RetailEditDecision};

/// An install as far as [`classify`](super::retail::classify) is concerned: a
/// directory with a game executable in it.
fn fake_install(root: &Path) -> PathBuf {
    let install = root.join("GTAV");
    std::fs::create_dir_all(install.join("x64")).expect("install dirs");
    std::fs::write(install.join("gta5.exe"), b"an install marker").expect("marker");
    install
}

fn state_holding_two_entries() -> (AppState, DocumentId, RpfNodeId) {
    let mut state = AppState::new();
    state.documents.clear();
    state.active_document = None;
    state.retail_edit_session_allow = false;

    let mut session = RpfEditSession::empty();
    let root = session.root_id();
    session
        .add_file_copy(root, "a.ytd", b"one".to_vec())
        .expect("a.ytd");
    let models = session.add_folder(root, "models").expect("models");
    session
        .add_file_copy(models, "b.ydr", b"two".to_vec())
        .expect("b.ydr");

    let doc_id = state.alloc_doc_id();
    let mut doc = Document::new(
        doc_id,
        Some(PathBuf::from("fixture.rpf")),
        DocPayload::Rpf(RpfDocument {
            session,
            expanded: Default::default(),
            focused_node: None,
            is_game_root: false,
            entry_documents: Default::default(),
        }),
    );
    doc.dirty = false;
    state.documents.push(doc);
    (state, doc_id, root)
}

/// Drive the per-frame tick until the worker's report has landed.
///
/// ⚠ Deliberately `poll_extract_job` rather than a test-only wait: the tick is
/// where a result is supposed to arrive, so a job that finishes and never
/// reaches it hangs here rather than passing.
pub(super) fn run_to_completion(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        if Instant::now() > deadline {
            panic!("the extraction never reported through the per-frame tick");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_destination_outside_an_install_extracts_and_one_inside_it_does_not() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = fake_install(tmp.path());
    let (mut state, doc_id, root) = state_holding_two_entries();
    state.prefs.edit_mode = EditMode::ReadOnly;

    // The control. Everything after this compares against a call that is
    // identical but for its destination.
    let outside = tmp.path().join("elsewhere");
    let started = state.extract_rpf_node(
        doc_id,
        root,
        outside.clone(),
        BodyForm::Decoded,
        TypeFilter::All,
    );
    assert_eq!(
        started,
        ExtractStart::Started { planned: 2 },
        "the fixture holds two entries; a run that planned none would make \
         every assertion below vacuous"
    );
    run_to_completion(&mut state);
    assert_eq!(
        files_under(&outside),
        vec!["a.ytd".to_string(), "models/b.ydr".to_string()],
        "the permitted extraction has to actually write, or a refusal proves \
         nothing"
    );
    assert_eq!(
        std::fs::read(outside.join("models/b.ydr")).expect("b.ydr"),
        b"two",
        "and write the entry's own bytes"
    );

    // ⚠ The same call, pointed inside the game as shipped.
    let refused = install.join("x64").join("extracted");
    let outcome = state.extract_rpf_node(
        doc_id,
        root,
        refused.clone(),
        BodyForm::Decoded,
        TypeFilter::All,
    );
    assert_eq!(
        outcome,
        ExtractStart::Refused,
        "`{}` is inside an install and the mode is read-only",
        refused.display()
    );
    assert!(
        !state.extract_jobs.is_running(),
        "a refused extraction must not have spawned a worker"
    );
    assert!(
        !refused.exists(),
        "the refusal still created the destination directory, so an extraction \
         refused after the `create_dir_all` would leave litter inside the install"
    );
    assert_eq!(
        files_under(&install),
        vec!["gta5.exe".to_string()],
        "the install gained a file from an extraction the gate refused"
    );
}

/// ⚠ The overlay is inside an install and writing there is the point.
///
/// Without this, "refuse everything under a game root" passes the test above
/// and breaks the one directory a modder is supposed to use.
#[test]
fn the_overlay_is_a_destination_the_gate_permits() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = fake_install(tmp.path());
    let (mut state, doc_id, root) = state_holding_two_entries();
    state.prefs.edit_mode = EditMode::Overlay;

    let dest = install.join("mods").join("extracted");
    assert_eq!(
        state.extract_rpf_node(
            doc_id,
            root,
            dest.clone(),
            BodyForm::Decoded,
            TypeFilter::All
        ),
        ExtractStart::Started { planned: 2 }
    );
    run_to_completion(&mut state);
    assert_eq!(
        files_under(&dest),
        vec!["a.ytd".to_string(), "models/b.ydr".to_string()]
    );
}

/// The confirm path reaches the extraction rather than only compiling.
///
/// ⚠ `every_confirmable_action_can_be_applied` checks that the arm exists in
/// the source. This checks that taking it writes the files — the difference
/// between a prompt a user answers and a prompt that silently does nothing,
/// which is the worse of the two failures because they believe it landed.
#[test]
fn confirming_a_shipped_destination_is_what_makes_it_write() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = fake_install(tmp.path());
    let (mut state, doc_id, root) = state_holding_two_entries();
    state.prefs.edit_mode = EditMode::Everything;

    let dest = install.join("x64").join("extracted");
    assert_eq!(
        state.extract_rpf_node(
            doc_id,
            root,
            dest.clone(),
            BodyForm::Decoded,
            TypeFilter::All
        ),
        ExtractStart::AwaitingConfirmation
    );
    assert!(
        !dest.exists(),
        "nothing may be written while the prompt is still up"
    );
    let prompt = state
        .pending_retail_edit
        .clone()
        .expect("the gate must have raised a prompt");
    assert!(
        matches!(prompt.action, PendingRetailEdit::ExtractInto { .. }),
        "the prompt has to carry the extraction, or confirming it applies \
         something else: {:?}",
        prompt.action
    );
    assert_eq!(
        prompt.target_path, dest,
        "and name what it is about to write"
    );

    state.apply_pending_retail_edit(RetailEditDecision::YesSession);
    run_to_completion(&mut state);
    assert_eq!(
        files_under(&dest),
        vec!["a.ytd".to_string(), "models/b.ydr".to_string()],
        "confirming the prompt did not reach the extraction"
    );
}

/// ⚠ Declining writes nothing, and leaves the gate where it was.
#[test]
fn declining_a_shipped_destination_writes_nothing_and_grants_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = fake_install(tmp.path());
    let (mut state, doc_id, root) = state_holding_two_entries();
    state.prefs.edit_mode = EditMode::Everything;

    let dest = install.join("x64").join("extracted");
    assert_eq!(
        state.extract_rpf_node(
            doc_id,
            root,
            dest.clone(),
            BodyForm::Decoded,
            TypeFilter::All
        ),
        ExtractStart::AwaitingConfirmation
    );
    state.apply_pending_retail_edit(RetailEditDecision::No);
    assert!(!state.extract_jobs.is_running());
    assert!(!dest.exists());
    assert!(
        !state.retail_edit_session_allow,
        "declining must not widen the gate for the rest of the session"
    );
}

/// A per-type extraction writes that type and leaves the rest.
#[test]
fn a_type_filter_reaches_the_bytes_on_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (mut state, doc_id, root) = state_holding_two_entries();
    state.prefs.edit_mode = EditMode::ReadOnly;

    let dest = tmp.path().join("only-ydr");
    assert_eq!(
        state.extract_rpf_node(
            doc_id,
            root,
            dest.clone(),
            BodyForm::Decoded,
            TypeFilter::Only(vec!["ydr".to_string()])
        ),
        ExtractStart::Started { planned: 1 },
        "one of the two entries is a `.ydr`"
    );
    run_to_completion(&mut state);
    assert_eq!(files_under(&dest), vec!["models/b.ydr".to_string()]);
}

/// ⚠ **One job model, not two that agree today.** Its properties — a per-item
/// failure counted rather than propagated, an interrupt that stops between
/// items — are tested once, in `or-job`; a consumer that grows its own loop
/// back gets them only by accident.
#[test]
fn the_cli_and_the_editor_run_the_same_job() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    const RUNS_THE_JOB: &[(&str, &str)] = &[
        (
            "src/state/extract_job/run.rs",
            "the editor's archive extraction",
        ),
        (
            "crates/tools/or-cli/src/rpf.rs",
            "`openrage-cli rpf extract`",
        ),
        ("crates/tools/or-cli/src/bulk.rs", "`openrage-cli bulk`"),
    ];
    for (rel, what) in RUNS_THE_JOB {
        let text = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("{rel} is named by this test: {e}"));
        assert!(
            text.contains("or_job::run("),
            "{what} ({rel}) does not go through `or_job::run`. Its own loop \
             gets the tolerance and the cancellation right only by accident, \
             and R21's Risk is that they are opposite requirements."
        );
    }

    let cli_jobs = std::fs::read_to_string(root.join("crates/tools/or-cli/src/jobs.rs"))
        .expect("or-cli/src/jobs.rs");
    for forked in ["struct Cancelled", "struct Progress"] {
        assert!(
            !cli_jobs.contains(forked),
            "`or-cli/src/jobs.rs` declares its own `{forked}` again. That type \
             is `or_job`'s, and a second one is how the editor came to be \
             unable to use the CLI's."
        );
    }
}
