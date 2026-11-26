//! What an edit of the maps that no save has written changes beside the Map screen: Save with a tab in front writes the maps on
//! the Map screen and the tab on every other, and what reads the files as they were last saved, a build and Check before sharing,
//! says an edit is held.
//!
//! ⚠ Built as `the_map_screen_holds_a_session` builds its packs: no window, no install.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::the_map_screen_holds_a_session::{
    differing, folder, harbour, move_to, said, save, sources, HARBOUR,
};
use super::the_wardrobe_workspace::until;
use super::workbench::{ConcernKind, Place, Question, Severity};
use super::{AppState, ToastType};
use crate::ui::dock::Workspace;

/// A text file open in a tab, with an edit nothing saved.
fn an_edited_tab(state: &mut AppState, path: &Path) -> super::DocumentId {
    std::fs::write(path, "as it was saved").unwrap();
    let text = super::document::TextDocument::decode(b"as it was edited");
    let id = state.insert_document(
        path.to_path_buf(),
        super::DocPayload::Text(text),
        None,
        None,
    );
    state.document_mut(id).expect("just opened").mark_edited();
    state.active_document = Some(id);
    id
}

#[test]
fn save_with_a_tab_in_front_writes_the_maps_on_the_map_screen_and_the_tab_everywhere_else() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = harbour(dir.path());
    let pack = folder(&state);
    let before = sources(&pack);
    let notes = dir.path().join("notes.txt");
    let tab = an_edited_tab(&mut state, &notes);
    let unsaved = |state: &AppState| state.document(tab).expect("the tab").dirty;
    let save = |state: &mut AppState| {
        let ctx = egui::Context::default();
        crate::ui::commands::run_action(state, &ctx, crate::ui::commands::Action::Save);
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.save_receiver.is_some() {
            assert!(Instant::now() < deadline, "the save never ended");
            state.poll_save_result();
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    move_to(&mut state, HARBOUR, 0, 12.5);
    assert!(state.map_edits_unwritten() && unsaved(&state));

    // On another screen the tab in front is what Save writes, and the maps wait.
    state.active_workspace = Workspace::Data;
    save(&mut state);
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert!(!unsaved(&state), "the tab was saved");
    assert_eq!(std::fs::read(&notes).unwrap(), b"as it was edited");
    assert!(state.map_edits_unwritten());
    assert_eq!(differing(&before, &sources(&pack)), Vec::<PathBuf>::new());

    // On the Map screen the edits of the maps go first, and the tab keeps its own until the next Save.
    state.document_mut(tab).expect("the tab").mark_edited();
    state.active_workspace = Workspace::Map;
    save(&mut state);
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert!(!state.map_edits_unwritten(), "the maps were saved");
    assert_ne!(differing(&before, &sources(&pack)), Vec::<PathBuf>::new());
    assert!(unsaved(&state), "and the tab was not");
    save(&mut state);
    assert!(!unsaved(&state), "with no map edit left, Save is the tab's");
}

/// The warnings of Problems that say a build goes without something unsaved.
fn left_out_of_a_build(state: &AppState) -> Vec<(Place, String)> {
    (state.workbench.findings().iter())
        .filter(|finding| finding.text.contains("which a build does not include"))
        .map(|finding| {
            assert_eq!(finding.severity, Severity::Warning);
            (finding.place.clone(), finding.text.clone())
        })
        .collect()
}

/// A build and Check before sharing read the files, which hold what was last saved: each says so while an edit is held.
#[test]
fn an_edit_not_yet_saved_is_said_where_the_files_are_read_as_they_were_last_saved() {
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = harbour(dir.path());
    let module = state.maps_pack().expect("a pack").module.clone();
    let unsaved_concerns = |state: &mut AppState| {
        state.check_before_sharing();
        until(state, "the check reading the folders", |s| {
            matches!(s.workbench.asking, Some(Question::Sharing { .. }))
        });
        let Some(Question::Sharing { concerns }) = state.workbench.asking.take() else {
            unreachable!("the check was waited for");
        };
        (concerns.iter())
            .filter(|concern| concern.kind == ConcernKind::Unsaved)
            .count()
    };
    state.poll_workbench();
    assert_eq!(left_out_of_a_build(&state), []);
    assert_eq!(unsaved_concerns(&mut state), 0);

    move_to(&mut state, HARBOUR, 0, 12.5);
    state.poll_workbench();
    let warned = [(
        Place::Content(module),
        format!("{HARBOUR} has unsaved changes, which a build does not include"),
    )];
    assert_eq!(left_out_of_a_build(&state), warned);
    assert_eq!(unsaved_concerns(&mut state), 1);
    assert!(state.project_unsaved());

    // Taken back, nothing is left to write and nothing is said; made again, it is said again until the save.
    state.undo_maps_change();
    state.poll_workbench();
    assert_eq!(left_out_of_a_build(&state), []);
    state.redo_maps_change();
    state.poll_workbench();
    assert_eq!(left_out_of_a_build(&state), warned);
    save(&mut state);
    state.poll_workbench();
    assert_eq!(left_out_of_a_build(&state), []);
    assert_eq!(unsaved_concerns(&mut state), 0);
    assert!(!state.project_unsaved());
}
