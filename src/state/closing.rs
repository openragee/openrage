//! Closing documents, a new session, opening a project and exiting: each asks
//! before it drops unsaved work, and waits for the answer.

use std::path::PathBuf;

use super::{AppState, DocPayload, Document, DocumentId, DocumentKind, Game};

/// What follows once a close has dropped its documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterClose {
    Nothing,
    NewSession,
    OpenProject(PathBuf),
    /// Leave the open project for the start page, with the install still mounted.
    CloseProject,
    /// Open this game's install at this folder in place of the one closed, then the file that asked for the switch.
    SwitchGame(Game, PathBuf, Option<PathBuf>),
    /// Forget this game's install folder, whose install the close dropped.
    ClearGameFolder(Game),
    Exit,
}

/// A close held until the user answers, because something it drops is unsaved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClose {
    pub documents: Vec<DocumentId>,
    pub then: AfterClose,
    /// Set once the user answers Save; the saves then run one at a time.
    pub saving: bool,
    /// The document whose save ran last.
    tried: Option<DocumentId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAnswer {
    Save,
    DontSave,
    Cancel,
}

impl AppState {
    /// Close a tab, asking first when it or a document opened from inside it is unsaved.
    pub fn close_document(&mut self, id: DocumentId) {
        self.request_close(vec![id], AfterClose::Nothing);
    }

    /// Exit once nothing unsaved is lost; `should_exit` says when.
    pub fn request_exit(&mut self) {
        self.request_close(self.every_document(), AfterClose::Exit);
    }

    pub(super) fn every_document(&self) -> Vec<DocumentId> {
        self.documents.iter().map(|d| d.id).collect()
    }

    /// Every document but the install, which a new session and an opened project keep mounted.
    pub(super) fn every_document_but_the_install(&self) -> Vec<DocumentId> {
        self.documents
            .iter()
            .filter(|d| d.kind != DocumentKind::GameRoot)
            .map(|d| d.id)
            .collect()
    }

    /// Drop `documents`, then do `then`: at once when nothing unsaved goes with them, else once the user answers.
    pub fn request_close(&mut self, documents: Vec<DocumentId>, then: AfterClose) {
        if self.pending_close.is_some() {
            self.show_warning_toast(tr!(
                "Answer the question about unsaved changes first, then try again"
            ));
            return;
        }
        let pending = PendingClose {
            documents,
            then,
            saving: false,
            tried: None,
        };
        if self.unsaved_documents(&pending.documents).is_empty()
            && !self.loses_project(&pending.then)
        {
            self.finish_close(pending);
        } else {
            self.pending_close = Some(pending);
        }
    }

    pub fn answer_close(&mut self, answer: CloseAnswer) {
        let Some(mut pending) = self.pending_close.take() else {
            return;
        };
        match answer {
            CloseAnswer::Cancel => {
                if let AfterClose::SwitchGame(_, _, Some(file)) = &pending.then {
                    let name = super::jobs::file_name(file);
                    self.show_info_toast(tr!(
                        "{name} was not opened: the game was not switched",
                        name = name
                    ));
                }
            }
            CloseAnswer::DontSave => self.finish_close(pending),
            CloseAnswer::Save => {
                pending.saving = true;
                self.pending_close = Some(pending);
            }
        }
    }

    /// Start the next save a Save answer asked for, as `Ctrl+S` saves it, or finish the close once none is left; `choose` is Save As for a document never saved.
    pub fn drive_close(&mut self, choose: impl FnOnce(&Self, DocumentId) -> Option<PathBuf>) {
        let Some(PendingClose {
            documents,
            saving: true,
            tried,
            ..
        }) = &self.pending_close
        else {
            return;
        };
        if self.save_in_flight() {
            return;
        }
        let (documents, tried) = (documents.clone(), *tried);
        let unsaved = self.unsaved_documents(&documents);
        // ⚠ A save that failed, or was declined at one of its prompts, closes nothing.
        if let Some(failed) = tried.filter(|id| unsaved.contains(id)) {
            self.pending_close = None;
            let name = self.document(failed).map(Document::file_name);
            self.show_warning_toast(tr!(
                "{name} was not saved, so nothing was closed",
                name = name.unwrap_or_default()
            ));
            return;
        }
        let Some(&next) = unsaved.first() else {
            let Some(pending) = self.pending_close.take() else {
                return;
            };
            // ⚠ After the files: one saved for the first time has the path that makes it a member only now.
            if self.loses_project(&pending.then) && !self.save_active_project(|_| None) {
                self.show_warning_toast(tr!("The project was not saved, so nothing was closed"));
                return;
            }
            self.finish_close(pending);
            return;
        };
        if let Some(pending) = &mut self.pending_close {
            pending.tried = Some(next);
        }
        let previous = self.active_document.replace(next);
        if self.document(next).is_some_and(|d| d.path.is_some()) {
            self.save_active_document();
        } else if let Some(path) = choose(self, next) {
            self.save_active_document_as(path);
        }
        self.active_document = previous;
    }

    /// A save, or a prompt one raised, that has not finished.
    fn save_in_flight(&self) -> bool {
        self.is_busy()
            || self.pending_retail_edit.is_some()
            || self.save_destination.is_some()
            || self.pending_export_fixes.is_some()
    }

    /// ⚠ With nothing left the window is the start page's, which Browse draws: a workspace left on screen drew its empty panels instead.
    fn back_to_browse_with_nothing_left(&mut self) {
        if self.documents.is_empty() && self.project_path.is_none() {
            self.active_workspace = crate::ui::dock::Workspace::Browse;
        }
    }

    fn finish_close(&mut self, pending: PendingClose) {
        match pending.then {
            AfterClose::Nothing => {
                for id in pending.documents {
                    self.remember_closed(id);
                    self.discard_document(id);
                }
                self.back_to_browse_with_nothing_left();
            }
            AfterClose::NewSession => {
                self.start_new_session();
                self.back_to_browse_with_nothing_left();
            }
            AfterClose::CloseProject => self.leave_project_for_the_start_page(),
            AfterClose::OpenProject(path) => self.open_project_now(path),
            AfterClose::SwitchGame(game, install_root, then_open) => {
                // ⚠ Dropped, not discarded: an install joins no project.
                for id in pending.documents {
                    self.drop_document(id);
                }
                self.set_active_game(game, install_root);
                if let Some(file) = then_open {
                    self.load_file(file);
                }
            }
            AfterClose::ClearGameFolder(game) => {
                for id in pending.documents {
                    self.drop_document(id);
                }
                self.set_game_install_folder(game, None);
            }
            AfterClose::Exit => self.should_exit = true,
        }
    }

    /// The documents among `ids`, and those opened from inside them, whose close loses work, in tab order.
    pub fn unsaved_documents(&self, ids: &[DocumentId]) -> Vec<DocumentId> {
        let mut reach = ids.to_vec();
        let mut next = 0;
        while let Some(&id) = reach.get(next) {
            next += 1;
            if let Some(DocPayload::Rpf(rpf)) = self.document(id).map(|d| &d.payload) {
                for child in rpf.entry_documents.values() {
                    if !reach.contains(child) {
                        reach.push(*child);
                    }
                }
            }
        }
        self.documents
            .iter()
            .filter(|d| reach.contains(&d.id) && self.loses_work(d))
            .map(|d| d.id)
            .collect()
    }

    /// Whether closing `doc` loses work.
    pub fn loses_work(&self, doc: &Document) -> bool {
        // ⚠ Pose keys stay a layer over their clip until a save bakes them in.
        let unbaked = !self.anim_edits.is_empty()
            && self
                .preview_scene
                .clip
                .is_some_and(|clip| clip.document == doc.id);
        unbaked || doc.dirty
    }

    /// Whether `then` leaves the open project with a change to its members, or an edit of its maps, not yet written.
    pub fn loses_project(&self, then: &AfterClose) -> bool {
        self.project_unsaved()
            && matches!(
                then,
                AfterClose::NewSession
                    | AfterClose::OpenProject(_)
                    | AfterClose::CloseProject
                    | AfterClose::Exit
            )
    }

    /// Answer the changed-on-disk question: reload over the unsaved edits, or keep them.
    pub fn answer_reload_conflicts(&mut self, reload: bool) {
        for path in std::mem::take(&mut self.reload_conflicts) {
            self.workbench.refreshed.remove(&path);
            if self.answer_for_a_map_source(&path, reload) {
                continue;
            }
            if reload {
                self.reload_ypt_from_disk(&path, true);
                if let Err(why) = self.reload_open_file(&path, true) {
                    self.show_error_toast(tr!(
                        "Could not read {path} again: {why}",
                        why = why,
                        path = path.display()
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::super::document::TextDocument;
    use super::super::document_io::{empty_file_bytes_for_kind, empty_payload_for_kind};
    use super::super::project::{save_project, ProjectFile};
    use super::super::RpfEntryBinding;
    use super::*;

    fn fresh() -> AppState {
        let mut state = AppState::new();
        state.documents.clear();
        state.active_document = None;
        state
    }

    /// A text file at `path` holding `text`, which is not saved yet.
    fn edited_text(state: &mut AppState, path: &Path, text: &str) -> DocumentId {
        let id = state.insert_document(
            path.to_path_buf(),
            DocPayload::Text(TextDocument::decode(text.as_bytes())),
            None,
            None,
        );
        state.document_mut(id).expect("just opened").mark_edited();
        id
    }

    /// Polls as the editor's frame does until the close is answered for good.
    fn settle(state: &mut AppState, choose: impl Fn(&AppState, DocumentId) -> Option<PathBuf>) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.pending_close.is_some() {
            assert!(Instant::now() < deadline, "the saves never finished");
            state.poll_save_result();
            state.drive_close(&choose);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Every path that drops a document, a reload from disk included, leaves
    /// a modified one open and modified until the user answers.
    #[test]
    fn a_modified_document_survives_every_path_until_the_user_answers() {
        type Run = Box<dyn Fn(&mut AppState, DocumentId)>;

        let dir = tempfile::tempdir().expect("a scratch directory");
        let project = dir.path().join("p.openrage-proj");
        save_project(&project, &ProjectFile::default()).expect("a project to open");
        let file = dir.path().join("smoke.ypt");
        let modified = |state: &mut AppState| {
            let blank = empty_file_bytes_for_kind(DocumentKind::Ypt).expect("a blank .ypt");
            std::fs::write(&file, blank).expect("the file on disk");
            let id = state.insert_document(
                file.clone(),
                DocPayload::Resource(or_io::Asset::Particles(Default::default())),
                None,
                None,
            );
            state.document_mut(id).expect("just opened").mark_edited();
            id
        };

        let paths: [(&str, Run, bool); 5] = [
            (
                "closing its tab",
                Box::new(|s: &mut AppState, id: DocumentId| s.close_document(id)),
                false,
            ),
            (
                "Close others",
                Box::new(|s: &mut AppState, _: DocumentId| {
                    let kept = s.insert_document(
                        PathBuf::from("kept.txt"),
                        DocPayload::Text(TextDocument::empty()),
                        None,
                        None,
                    );
                    let others = s.every_document().into_iter().filter(|d| *d != kept);
                    s.request_close(others.collect(), AfterClose::Nothing);
                }),
                false,
            ),
            (
                "New session",
                Box::new(|s: &mut AppState, _: DocumentId| s.new_project()),
                false,
            ),
            (
                "opening a project",
                Box::new(move |s: &mut AppState, _: DocumentId| {
                    s.load_project_from(project.clone())
                }),
                false,
            ),
            (
                "exiting",
                Box::new(|s: &mut AppState, _: DocumentId| s.request_exit()),
                true,
            ),
        ];
        for (path, run, exits) in &paths {
            let mut state = fresh();
            let id = modified(&mut state);
            run(&mut state, id);
            assert!(
                state.document(id).is_some_and(|d| d.dirty),
                "{path} dropped the document before the user answered"
            );
            assert!(state.pending_close.is_some(), "{path} asked nothing");
            state.answer_close(CloseAnswer::Cancel);
            assert!(
                state.document(id).is_some_and(|d| d.dirty) && !state.should_exit,
                "{path}: Cancel went ahead"
            );

            run(&mut state, id);
            state.answer_close(CloseAnswer::DontSave);
            if *exits {
                assert!(state.should_exit, "{path}: Don't save did not exit");
            } else {
                assert!(
                    state.document(id).is_none() && !state.should_exit,
                    "{path}: Don't save kept the document"
                );
            }
        }

        let mut state = fresh();
        let id = modified(&mut state);
        state.reload_ypt_from_disk(&file, false);
        assert!(
            state.document(id).is_some_and(|d| d.dirty),
            "a change on disk replaced the edits before the user answered"
        );
        assert_eq!(state.reload_conflicts, std::slice::from_ref(&file));
        state.answer_reload_conflicts(false);
        assert!(state.document(id).is_some_and(|d| d.dirty));
        state.reload_ypt_from_disk(&file, false);
        state.answer_reload_conflicts(true);
        assert!(
            state.document(id).is_some_and(|d| !d.dirty),
            "Reload kept the edits"
        );
        assert!(state.reload_conflicts.is_empty());
    }

    #[test]
    fn exiting_asks_once_for_every_unsaved_file_and_saves_each_before_it_exits() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (a, b, clean) = (
            dir.path().join("a.txt"),
            dir.path().join("b.txt"),
            dir.path().join("clean.txt"),
        );
        let mut state = fresh();
        let a_id = edited_text(&mut state, &a, "first");
        let b_id = edited_text(&mut state, &b, "second");
        state.insert_document(
            clean.clone(),
            DocPayload::Text(TextDocument::empty()),
            None,
            None,
        );

        state.request_exit();
        let pending = state.pending_close.clone().expect("exiting asks");
        assert_eq!(state.unsaved_documents(&pending.documents), [a_id, b_id]);
        state.answer_close(CloseAnswer::Save);
        settle(&mut state, |_, _| None);

        assert!(state.should_exit);
        assert_eq!(std::fs::read_to_string(&a).expect("a was written"), "first");
        assert_eq!(
            std::fs::read_to_string(&b).expect("b was written"),
            "second"
        );
        assert!(!clean.exists(), "an unmodified file was written");
    }

    #[test]
    fn a_save_that_fails_closes_nothing() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let not_a_folder = dir.path().join("file.txt");
        std::fs::write(&not_a_folder, b"in the way").expect("a file");
        let mut state = fresh();
        let id = edited_text(&mut state, &not_a_folder.join("a.txt"), "kept");

        state.close_document(id);
        state.answer_close(CloseAnswer::Save);
        settle(&mut state, |_, _| None);

        assert!(state.document(id).is_some_and(|d| d.dirty));
        assert!(
            state
                .toasts
                .iter()
                .any(|t| t.message.contains("nothing was closed")),
            "the user is told why the tab stayed"
        );
    }

    #[test]
    fn a_document_never_saved_is_saved_where_the_dialog_points() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let target = dir.path().join("named.txt");
        let mut state = fresh();
        let id = state.new_empty_document(DocumentKind::Text);

        state.close_document(id);
        state.answer_close(CloseAnswer::Save);
        settle(&mut state, |_, _| None);
        assert!(
            state.document(id).is_some(),
            "a dialog the user cancelled still closed the document"
        );

        state.close_document(id);
        state.answer_close(CloseAnswer::Save);
        settle(&mut state, |_, asked| {
            assert_eq!(asked, id);
            Some(target.clone())
        });
        assert!(state.document(id).is_none());
        assert!(target.is_file(), "the document was not written");
    }

    #[test]
    fn a_save_that_asks_where_it_goes_holds_the_close_until_it_is_answered() {
        let mut state = fresh();
        let id = state.alloc_doc_id();
        let mut entry = Document::new(
            id,
            Some(PathBuf::from("update.rpf").join("a.txt")),
            DocPayload::Text(TextDocument::decode(b"edited")),
        );
        entry.rpf_entry_binding = Some(RpfEntryBinding {
            parent_doc_id: 0,
            entry_id: 1,
        });
        entry.mark_edited();
        state.documents.push(entry);

        state.request_exit();
        state.answer_close(CloseAnswer::Save);
        state.drive_close(|_, _| None);
        assert!(
            state.save_destination.is_some(),
            "an archive entry saves as Ctrl+S saves it, asking where it goes"
        );
        state.drive_close(|_, _| None);
        assert!(state.pending_close.is_some() && !state.should_exit);

        state.cancel_save_destination();
        state.drive_close(|_, _| None);
        assert!(state.pending_close.is_none());
        assert!(!state.should_exit, "a save the user declined still exited");
        assert!(state.document(id).is_some_and(|d| d.dirty));
    }

    #[test]
    fn closing_an_archive_asks_about_what_was_opened_from_inside_it() {
        let mut state = fresh();
        let archive = state.insert_document(
            PathBuf::from("x.rpf"),
            empty_payload_for_kind(DocumentKind::Rpf).expect("a blank archive"),
            None,
            None,
        );
        let entry = state.insert_document(
            PathBuf::from("x.rpf").join("a.txt"),
            DocPayload::Text(TextDocument::empty()),
            Some(RpfEntryBinding {
                parent_doc_id: archive,
                entry_id: 1,
            }),
            None,
        );
        state.document_mut(entry).expect("open").mark_edited();

        state.close_document(archive);
        assert!(state.document(archive).is_some() && state.document(entry).is_some());
        let pending = state
            .pending_close
            .clone()
            .expect("closing the archive asks");
        assert_eq!(state.unsaved_documents(&pending.documents), [entry]);

        state.answer_close(CloseAnswer::DontSave);
        assert!(state.document(archive).is_none() && state.document(entry).is_none());
    }

    #[test]
    fn pose_keys_not_yet_baked_are_unsaved_work() {
        let mut state = fresh();
        let id = state.insert_document(
            PathBuf::from("anim.ycd"),
            DocPayload::Resource(or_io::Asset::Clips(Default::default())),
            None,
            None,
        );
        state.preview_scene.set_clip(id, 0x1234);
        state.anim_edits.set(
            or_anim::EditKey {
                bone: 0,
                track: or_anim::EditTrack::Translation,
                frame: 1,
            },
            or_anim::EditValue::Vector(glam::Vec3::X),
        );

        state.close_document(id);
        assert!(state.document(id).is_some() && state.pending_close.is_some());
    }

    /// ⚠ An install writes a disk edit as it happens, and asking about one
    /// whose archives are written would offer a Save that can never succeed.
    #[test]
    fn an_install_whose_archives_are_written_exits_without_asking() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut state = fresh();
        state.open_game_root(dir.path().to_path_buf());
        state.documents[0].mark_edited();

        state.request_exit();
        assert!(state.should_exit);
    }

    #[test]
    fn exit_asks_the_window_to_close() {
        let mut state = fresh();
        assert!(!state.should_exit);
        state.request_exit();
        assert!(state.should_exit);
    }

    #[test]
    fn the_window_title_names_the_document_and_the_project_and_marks_unsaved_work() {
        let mut state = fresh();
        assert_eq!(state.window_title(), "OpenRage");

        let id = state.insert_document(
            PathBuf::from("C:/mods/notes.txt"),
            DocPayload::Text(TextDocument::empty()),
            None,
            None,
        );
        assert_eq!(state.window_title(), "notes.txt — OpenRage");
        state.document_mut(id).expect("open").mark_edited();
        assert_eq!(state.window_title(), "notes.txt ● — OpenRage");

        state.project_path = Some(PathBuf::from("C:/mods/heist.openrage-proj"));
        assert_eq!(state.window_title(), "notes.txt ● — heist — OpenRage");
        state.project_carried.changed = true;
        assert_eq!(state.window_title(), "notes.txt ● — heist ● — OpenRage");
    }

    #[test]
    fn the_window_is_given_its_title_whenever_it_changes() {
        let mut state = fresh();
        assert_eq!(state.changed_window_title().as_deref(), Some("OpenRage"));
        assert_eq!(state.changed_window_title(), None, "nothing changed");

        state.new_empty_document(DocumentKind::Text);
        assert_eq!(
            state.changed_window_title().as_deref(),
            Some("Untitled.txt ● — OpenRage")
        );
    }
}
