//! Overlay copies the title has outgrown: found when an install opens, weighed and refreshed on the user's word.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, TryRecvError};
use std::sync::Arc;

use or_io::container::RpfCryptoProvider;
use or_job::Cancel;
use or_title::{record_file, refresh_plan, stale_copies, BaseRecord};

use super::jobs::{file_name, JobKind, Outcome, Then};
use super::keys::gta_install_root_from_prefs;
use super::overlay_copy::{OverlayCopyOffer, Weighing};
use super::{AppState, DocPayload, GateOutcome};

/// Where what overlay copies were made from is kept; `None` where nothing is remembered.
pub(super) fn base_records_dir() -> Option<PathBuf> {
    super::prefs::config_root().map(|root| root.join("overlay_bases"))
}

/// Keep `record` of what a copy was made from at `file`; where there is none, no earlier one is left to call the copy stale.
pub(super) fn keep_record(record: Option<(Result<BaseRecord, String>, PathBuf)>) {
    let Some((record, file)) = record else {
        return;
    };
    if let Err(e) = record.and_then(|record| record.write(&file)) {
        tracing::info!("The title's updates are not tracked for this overlay copy: {e}");
        let _ = std::fs::remove_file(&file);
    }
}

impl AppState {
    /// The records' folder and the keys that read an archive's table, or `None` where nothing is remembered.
    pub(super) fn base_tracking(&self) -> Option<(PathBuf, Arc<dyn RpfCryptoProvider>)> {
        Some((base_records_dir()?, self.keys.provider()))
    }

    /// Record that the overlay copy at `copy` was made from the install's archive at `base`.
    pub(super) fn note_overlay_base(&self, base: &Path, copy: &Path) {
        keep_record(
            self.base_tracking().map(|(records, crypto)| {
                (BaseRecord::of(base, &crypto), record_file(&records, copy))
            }),
        );
    }

    /// The install the overlay belongs to: the one open, else the one the keys came from.
    fn overlay_install_root(&self) -> Option<PathBuf> {
        (self.prefs.active_game_root().cloned())
            .or_else(|| gta_install_root_from_prefs(&self.prefs))
    }

    /// `copy` as the overlay names it: its path below the overlay folder, else whole.
    pub fn overlay_name(&self, copy: &Path) -> String {
        let root = self.overlay_install_root();
        let inside = root.and_then(|root| copy.strip_prefix(root.join(or_title::OVERLAY_DIR)).ok());
        inside.unwrap_or(copy).display().to_string()
    }

    /// Look for overlay copies the title has changed the original of, as the install opens.
    pub fn check_overlay_staleness(&mut self) {
        if !self.prefs.read_overlay_first {
            // Copies nothing reads hide nothing.
            self.overlay_copy.stale.clear();
            return;
        }
        let Some(root) = self.overlay_install_root() else {
            return;
        };
        let Some((records, crypto)) = self.base_tracking() else {
            return;
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let found = stale_copies(&root, &records, &crypto);
            let _ = tx.send((root, found));
        });
        // A check still running is dropped: it holds the keys from before this one.
        self.overlay_copy.checking = Some(rx);
    }

    /// Where a finished staleness check or weighing lands, each frame.
    pub fn poll_overlay_refresh(&mut self) {
        self.poll_overlay_staleness();
        self.poll_overlay_weighing();
    }

    fn poll_overlay_staleness(&mut self) {
        let Some(check) = &self.overlay_copy.checking else {
            return;
        };
        let (root, found) = match check.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.overlay_copy.checking = None;
                return;
            }
        };
        self.overlay_copy.checking = None;
        if self.overlay_install_root().as_ref() != Some(&root) {
            return;
        }
        let news: Vec<String> = (found.iter())
            .filter(|stale| self.overlay_copy.stale_copy(&stale.copy).is_none())
            .map(|stale| self.overlay_name(&stale.copy))
            .collect();
        self.overlay_copy.stale = found;
        if news.is_empty() {
            return;
        }
        let shown = news.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        let more = news.len().saturating_sub(3);
        let shown = if more > 0 {
            tr!("{shown} and {more} more", shown = shown, more = more)
        } else {
            shown
        };
        self.show_warning_toast(tr!(
            "The game was updated since these mods overlay copies were made, so they hide what the update changed: {shown}. Refresh them from the archive's menu or the Mods overlay settings.",
            shown = shown
        ));
    }

    /// Work out what refreshing the overlay copy at `copy` from the patched archive would discard, then offer it.
    pub fn refresh_overlay_copy(&mut self, copy: &Path) {
        let Some(stale) = self.overlay_copy.stale_copy(copy).cloned() else {
            return;
        };
        let busy = self.overlay_copy.running().is_some()
            || self.overlay_copy.weighing.is_some()
            || self.overlay_copy.offer.is_some();
        if busy {
            self.show_error_toast(tr!("An overlay copy is already under way"));
            return;
        }
        if !matches!(self.gate_permits(Some(copy)), GateOutcome::Proceed) {
            return;
        }
        if let Some(refusal) = self.overlay_refresh_blocker(copy) {
            self.show_error_toast(refusal);
            return;
        }
        let Some((records, crypto)) = self.base_tracking() else {
            return;
        };
        let Some(record) = BaseRecord::read(&record_file(&records, copy)) else {
            self.show_error_toast(tr!(
                "Nothing records what `{copy}` was made from",
                copy = copy.display()
            ));
            return;
        };
        let (cancel, (tx, rx)) = (Cancel::new(), channel());
        let (worker_cancel, worker_stale) = (cancel.clone(), stale.clone());
        std::thread::spawn(move || {
            let plan = refresh_plan(
                &worker_stale.copy,
                &worker_stale.base,
                &record,
                &crypto,
                &|| worker_cancel.is_set(),
            );
            let _ = tx.send(plan);
        });
        self.overlay_copy.weighing = Some(Weighing {
            stale,
            cancel,
            receiver: rx,
        });
    }

    fn poll_overlay_weighing(&mut self) {
        let Some(job) = &self.overlay_copy.weighing else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err(tr!("The weighing stopped without reporting").to_string())
            }
        };
        let (stale, cancelled) = (job.stale.clone(), job.cancel.is_set());
        self.overlay_copy.weighing = None;
        // A weighing asked to stop offers nothing, even where the worker finished first.
        match result {
            Ok(plan) if !cancelled => {
                let bytes = std::fs::metadata(&stale.base).map_or(0, |meta| meta.len());
                self.overlay_copy.offer = Some(OverlayCopyOffer {
                    source: stale.base,
                    dest: stale.copy,
                    bytes,
                    replaces: Some(plan),
                });
            }
            Err(e) if !cancelled => {
                self.show_error_toast(e.clone());
                self.jobs.record(JobKind::OverlayCopy, Outcome::Failed, e);
            }
            _ => {
                let said = tr!("Nothing was refreshed");
                self.jobs
                    .record(JobKind::OverlayCopy, Outcome::Stopped(Then::Nothing), said);
            }
        }
    }

    /// Why the overlay copy at `copy` cannot be replaced now, if it cannot.
    ///
    /// ⚠ The open session reads the file at the offsets it was planned with: an unsaved edit would be
    /// lost, and an open tab would point into a tree the replacement drops.
    pub(super) fn overlay_refresh_blocker(&self, copy: &Path) -> Option<String> {
        if let Err(reading) = self.no_job_holds(copy, tr!("refresh it")) {
            return Some(reading);
        }
        for doc in &self.documents {
            let DocPayload::Rpf(rpf) = &doc.payload else {
                continue;
            };
            let session = &rpf.session;
            let open = session
                .archives
                .iter()
                .filter(|(_, slot)| slot.fs_path.as_deref() == Some(copy));
            for (id, slot) in open {
                let Some(root) = session.archive_root_node(*id) else {
                    continue;
                };
                if slot.dirty {
                    return Some(
                        tr!("Save or discard the archive's edits before refreshing it").to_string(),
                    );
                }
                if (rpf.entry_documents.keys()).any(|&node| session.is_descendant(node, root)) {
                    return Some(
                        tr!("Close the tabs opened from inside the archive before refreshing it")
                            .to_string(),
                    );
                }
            }
        }
        None
    }

    /// The copy at `copy` was replaced: forget what the session read from it, and what was indexed of the old one.
    pub(super) fn refreshed_overlay_copy(&mut self, copy: &Path) {
        self.overlay_copy.stale.retain(|stale| stale.copy != copy);
        // ⚠ Every offset the session read from this file is stale now.
        self.expander.files_replaced();
        let mut reopen = Vec::new();
        for doc in self.documents.iter_mut() {
            let DocPayload::Rpf(rpf) = &mut doc.payload else {
                continue;
            };
            let opened: Vec<_> = (rpf.session.archives.iter())
                .filter(|(_, slot)| slot.fs_path.as_deref() == Some(copy))
                .filter_map(|(id, _)| rpf.session.archive_root_node(*id))
                .collect();
            for root in opened {
                rpf.session.unload_archive(root);
                reopen.push((doc.id, root));
            }
        }
        for (doc_id, root) in reopen {
            self.request_expand(doc_id, root);
        }
        self.name_index.stop_bg_scan();
        self.maybe_start_name_scan();
        let said = tr!(
            "Refreshed {copy} from the patched archive",
            copy = file_name(copy)
        );
        self.show_toast(said.clone());
        let then = self.reveal_in_install(copy);
        self.jobs
            .record(JobKind::OverlayCopy, Outcome::Done(then), said);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use or_io::container::rpf::RpfBuilder;
    use or_title::{Change, Discard, StaleCopy, Staleness};
    use tempfile::TempDir;

    use super::*;
    use crate::state::document::{DocumentId, RpfNodeId};
    use crate::state::prefs::EditMode;
    use crate::state::toasts::ToastType;
    use crate::state::RpfNodeBacking;

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = RpfBuilder::default();
        for (path, bytes) in entries {
            builder.insert_with(path, *bytes, false).expect("an entry");
        }
        builder.to_bytes().expect("save")
    }

    const SHIPPED: &[(&str, &[u8])] = &[
        ("data/version.txt", b"1.0.100"),
        ("data/handling.meta", b"handling v1"),
    ];

    /// The update: a longer version string, and a file the shipped archive lacked.
    const PATCHED: &[(&str, &[u8])] = &[
        ("data/version.txt", b"1.0.1000"),
        ("data/handling.meta", b"handling v1"),
        ("data/new.dat", b"new"),
    ];

    struct Scene {
        _tmp: TempDir,
        root: PathBuf,
        base: PathBuf,
        copy: PathBuf,
        state: AppState,
    }

    /// An install with `update/update.rpf` shipped, the app pointed at it in the overlay edit mode.
    fn scene() -> Scene {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("GTAV");
        std::fs::create_dir_all(root.join("update")).expect("install dirs");
        std::fs::write(root.join("gta5.exe"), b"an install marker").expect("marker");
        let base = root.join("update").join("update.rpf");
        std::fs::write(&base, archive(SHIPPED)).expect("the shipped archive");
        let mut state = AppState::new();
        state.retail_edit_session_allow = false;
        state.prefs.edit_mode = EditMode::Overlay;
        state
            .prefs
            .game_paths
            .set(or_title::Game::GtaVLegacy, Some(root.clone()));
        state.prefs.active_game = Some(or_title::Game::GtaVLegacy);
        let copy = root.join("mods").join("update").join("update.rpf");
        Scene {
            _tmp: tmp,
            root,
            base,
            copy,
            state,
        }
    }

    /// Poll until nothing the overlay is doing is still running.
    fn settle(state: &mut AppState) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while state.overlay_copy.running().is_some()
            || state.overlay_copy.weighing.is_some()
            || state.overlay_copy.checking.is_some()
        {
            state.poll_overlay_copy();
            state.poll_overlay_refresh();
            assert!(Instant::now() < deadline, "the overlay work never reported");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    impl Scene {
        /// The copy as the explorer's menu makes it, kept with a record of the original.
        fn copy_it(&mut self) {
            assert!(self.state.offer_overlay_copy(&self.base));
            self.state.answer_overlay_copy(true);
            settle(&mut self.state);
            assert!(self.copy.is_file(), "the copy was made");
        }

        fn patch(&self) {
            let modified = std::fs::metadata(&self.base)
                .expect("the original archive metadata")
                .modified()
                .expect("the original archive modification time");
            std::fs::write(&self.base, archive(PATCHED)).expect("the patched archive");
            // Both tiny archives occupy the same aligned length; a patch must advance its stat.
            std::fs::File::options()
                .write(true)
                .open(&self.base)
                .expect("the patched archive")
                .set_modified(modified + Duration::from_secs(2))
                .expect("the patch modification time");
        }

        fn check(&mut self) {
            self.state.check_overlay_staleness();
            settle(&mut self.state);
        }

        fn refresh(&mut self) {
            let copy = self.copy.clone();
            self.state.refresh_overlay_copy(&copy);
            settle(&mut self.state);
        }

        fn warned_about(&self, name: &Path) -> bool {
            let name = name.display().to_string();
            (self.state.toasts.iter())
                .any(|t| t.toast_type == ToastType::Warning && t.message.contains(&name))
        }

        /// The copy's archive open in the install's tree, as it is once expanded.
        fn open_in_tree(&mut self) -> (DocumentId, RpfNodeId) {
            self.state.open_game_root(self.root.clone());
            let opened = crate::state::rpf_archive_open::open_archive_from_disk(
                &self.copy,
                self.state.keys.provider(),
            )
            .expect("open the copy");
            let doc_id = self.state.documents[0].id;
            let Some(DocPayload::Rpf(rpf)) =
                self.state.document_mut(doc_id).map(|d| &mut d.payload)
            else {
                panic!("the install mounts as a GameRoot");
            };
            let root = rpf.session.root_id();
            let node = rpf
                .session
                .add_unopened_disk_archive(root, "update.rpf".to_string(), self.copy.clone())
                .expect("the copy's node");
            let id = rpf
                .session
                .register_archive(opened, Some(self.copy.clone()))
                .expect("an archive id");
            if let Some(RpfNodeBacking::Archive { archive_id, .. }) =
                rpf.session.node_mut(node).map(|n| &mut n.backing)
            {
                *archive_id = Some(id);
            }
            (doc_id, node)
        }
    }

    #[test]
    fn an_install_opened_after_a_patch_names_the_copies_it_outgrew() {
        let mut scene = scene();
        scene.copy_it();
        scene.check();
        assert!(
            scene.state.overlay_copy.stale.is_empty(),
            "a copy of an archive the title has not changed is not reported"
        );

        scene.patch();
        scene.check();
        assert_eq!(
            scene.state.overlay_copy.stale,
            vec![StaleCopy {
                copy: scene.copy.clone(),
                base: scene.base.clone(),
            }]
        );
        assert!(
            scene.warned_about(&Path::new("update").join("update.rpf")),
            "the warning names the archive, as the overlay holds it"
        );

        let toasts = scene.state.toasts.iter().count();
        scene.check();
        assert_eq!(
            scene.state.toasts.iter().count(),
            toasts,
            "a second look at the same copies says nothing again"
        );
    }

    #[test]
    fn a_refresh_lists_what_it_discards_and_writes_nothing_until_it_is_answered() {
        let mut scene = scene();
        scene.copy_it();
        // The user's save rebuilds the copy with their edit.
        let edited = archive(&[
            ("data/version.txt", b"1.0.100"),
            ("data/handling.meta", b"handling v2"),
        ]);
        std::fs::write(&scene.copy, &edited).expect("the edited copy");
        scene.patch();
        scene.check();

        scene.refresh();
        let offer = scene.state.overlay_copy.offer.clone().expect("an offer");
        let plan = offer.replaces.expect("a refresh says what it replaces");
        assert_eq!(
            plan.yours,
            vec![Discard::new("data/handling.meta", Change::Edited)]
        );
        assert_eq!(plan.by_update, vec!["data/version.txt".to_string()]);
        assert_eq!(
            std::fs::read(&scene.copy).expect("the copy"),
            edited,
            "nothing is written before the user answers"
        );

        scene.state.answer_overlay_copy(true);
        settle(&mut scene.state);
        assert_eq!(
            std::fs::read(&scene.copy).expect("the copy"),
            std::fs::read(&scene.base).expect("the patched archive"),
            "the refresh is the patched archive"
        );
        assert!(scene.state.overlay_copy.stale.is_empty());
        scene.check();
        assert!(
            scene.state.overlay_copy.stale.is_empty(),
            "the refreshed copy is of the patched archive, so it is fresh"
        );
    }

    #[test]
    fn a_refresh_the_user_declines_changes_nothing() {
        let mut scene = scene();
        scene.copy_it();
        let edited = archive(&[("data/version.txt", b"mine")]);
        std::fs::write(&scene.copy, &edited).expect("the edited copy");
        scene.patch();
        scene.check();

        scene.refresh();
        scene.state.answer_overlay_copy(false);
        settle(&mut scene.state);

        assert_eq!(std::fs::read(&scene.copy).expect("the copy"), edited);
        assert_eq!(scene.state.overlay_copy.stale.len(), 1, "still out of date");
    }

    #[test]
    fn a_cancel_during_the_weighing_leaves_no_offer_and_writes_nothing() {
        let mut scene = scene();
        scene.copy_it();
        let edited = archive(&[("data/version.txt", b"mine")]);
        std::fs::write(&scene.copy, &edited).expect("the edited copy");
        scene.patch();
        scene.check();

        let listed = |state: &AppState| {
            (state.running_jobs().into_iter()).find(|job| job.kind == JobKind::OverlayCopy)
        };
        let copy = scene.copy.clone();
        scene.state.refresh_overlay_copy(&copy);
        assert!(
            listed(&scene.state).is_some_and(|job| !job.stopping),
            "the weighing is a job in the list, not yet asked to stop"
        );

        scene.state.cancel_job(JobKind::OverlayCopy);
        assert!(
            listed(&scene.state).is_some_and(|job| job.stopping),
            "the list's Cancel reaches the weighing"
        );
        settle(&mut scene.state);

        assert!(scene.state.overlay_copy.offer.is_none());
        assert!(listed(&scene.state).is_none());
        assert_eq!(std::fs::read(&scene.copy).expect("the copy"), edited);
        let ended = (scene.state.jobs.ended().iter())
            .find(|job| job.kind == JobKind::OverlayCopy)
            .expect("the stop is recorded");
        assert_eq!(ended.outcome, Outcome::Stopped(Then::Nothing));
        assert_eq!(ended.said, "Nothing was refreshed");
        assert_eq!(scene.state.overlay_copy.stale.len(), 1, "still out of date");
    }

    #[test]
    fn a_refresh_is_not_offered_where_the_edit_mode_refuses_the_overlay() {
        let mut scene = scene();
        scene.copy_it();
        scene.patch();
        scene.check();

        scene.state.prefs.edit_mode = EditMode::ReadOnly;
        scene.refresh();
        assert!(scene.state.overlay_copy.offer.is_none());
    }

    #[test]
    fn a_copy_with_unsaved_edits_open_is_not_refreshed_under_them() {
        let mut scene = scene();
        scene.copy_it();
        scene.patch();
        scene.check();
        let (doc_id, node) = scene.open_in_tree();
        let Some(DocPayload::Rpf(rpf)) = scene.state.document_mut(doc_id).map(|d| &mut d.payload)
        else {
            panic!("an archive tree");
        };
        rpf.session.mark_archive_dirty(node);

        scene.refresh();
        assert!(scene.state.overlay_copy.offer.is_none());
        assert!(
            (scene.state.toasts.iter()).any(|t| t.message.contains("Save or discard")),
            "the refusal says what to do first"
        );
    }

    #[test]
    fn a_refreshed_copy_that_was_open_is_forgotten_by_the_tree() {
        let mut scene = scene();
        scene.copy_it();
        scene.patch();
        scene.check();
        let (doc_id, node) = scene.open_in_tree();

        scene.refresh();
        scene.state.answer_overlay_copy(true);
        settle(&mut scene.state);

        let Some(DocPayload::Rpf(rpf)) = scene.state.document_mut(doc_id).map(|d| &mut d.payload)
        else {
            panic!("an archive tree");
        };
        assert!(
            matches!(
                rpf.session.node(node).map(|n| &n.backing),
                Some(RpfNodeBacking::Archive {
                    archive_id: None,
                    ..
                })
            ),
            "what the session read from the replaced file is dropped, so the next expansion reads the new one"
        );
    }

    #[test]
    fn a_first_save_into_the_overlay_keeps_what_it_was_made_from() {
        let scene = scene();
        let (records, crypto) = scene.state.base_tracking().expect("a place to remember");
        scene.state.note_overlay_base(&scene.base, &scene.copy);
        let record = BaseRecord::read(&record_file(&records, &scene.copy)).expect("a record");
        assert_eq!(record.verdict(&scene.base, &crypto), Staleness::Fresh);
        scene.patch();
        assert_eq!(record.verdict(&scene.base, &crypto), Staleness::Stale);
    }
}
