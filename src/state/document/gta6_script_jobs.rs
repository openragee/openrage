//! A script decompiled again on a thread of its own, at another level or with one more function
//! of a program too large to decompile whole, and the scripts that wait for the load worker.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use or_io::gta6::script::lift::Progress;

use super::{set_level, Ask, Gta6ScriptDocument};
use crate::state::{AppState, DocPayload, DocumentId, DocumentKind, RpfEntryBinding};
use crate::ui::panel_state::PanelState;
use crate::ui::panels::selection_ids::DOC_ROOT_SEL;
use crate::ui::panels::text_editor::script_view::ScriptAsk;

/// What a decompile comes to.
type Done = Result<Gta6ScriptDocument, String>;

/// One decompile in flight for a document.
pub struct Job {
    pub ask: Ask,
    /// The function whose head the caret goes to once the source lands.
    pub focus: Option<u32>,
    pub started: Instant,
    /// How many functions the thread has written, and the flag that stops it.
    pub progress: Arc<Progress>,
    /// Where the thread leaves what it made.
    // ⚠ The thread holds it weakly: a job dropped frees what the thread makes as soon as it is made.
    slot: Arc<Mutex<Option<Done>>>,
}

/// The decompiles in flight, by the document each is for.
#[derive(Default)]
pub struct Jobs(BTreeMap<DocumentId, Job>);

impl PanelState for Jobs {
    const KEY: &'static str = "gta6_script_jobs";
}

/// The bytes of a script that wait for the load worker, as the inline loader was handed them.
pub(crate) type WaitingLoad = (PathBuf, DocumentKind, Vec<u8>, Option<RpfEntryBinding>);

/// The scripts opened while another load was in flight, in the order asked.
#[derive(Default)]
pub struct Waiting(VecDeque<WaitingLoad>);

impl PanelState for Waiting {
    const KEY: &'static str = "gta6_script_waiting";
}

/// Whether `data`, opened as `kind`, goes to the script's reader: a decompile takes seconds, so such bytes are never read on the UI thread.
pub(crate) fn may_be_script(kind: DocumentKind, data: &[u8]) -> bool {
    let sniffed = matches!(kind, DocumentKind::Hex) && data.starts_with(&[0u8; 8]);
    sniffed || matches!(kind, DocumentKind::Gta6Script)
}

impl AppState {
    /// Keep a script's bytes for the load worker, which is busy; a path that waits already is kept once.
    pub(crate) fn gta6_script_waits(&mut self, load: WaitingLoad) {
        let waiting = &mut self.panels.get::<Waiting>().0;
        if !waiting.iter().any(|held| held.0 == load.0) {
            waiting.push_back(load);
        }
    }

    /// The script that has waited longest for the load worker.
    pub(crate) fn next_waiting_gta6_script(&mut self) -> Option<WaitingLoad> {
        // No store is made for a session that queued no script.
        self.panels.peek::<Waiting>()?;
        self.panels.get::<Waiting>().0.pop_front()
    }

    /// Drop every script that waits for the load worker.
    pub(crate) fn forget_waiting_gta6_scripts(&mut self) {
        if self.panels.peek::<Waiting>().is_some() {
            self.panels.get::<Waiting>().0.clear();
        }
    }

    /// The decompile in flight for `doc`.
    pub(crate) fn gta6_script_job(&self, doc: DocumentId) -> Option<&Job> {
        self.panels.peek::<Jobs>()?.0.get(&doc)
    }

    /// Stop the decompile in flight for `doc` once the function it is writing is written: what its thread makes is dropped.
    pub(crate) fn drop_gta6_script_job(&mut self, doc: DocumentId) {
        if let Some(job) = self.gta6_script_job(doc) {
            job.progress.stop();
            self.panels.get::<Jobs>().0.remove(&doc);
        }
    }

    /// Decompile the script `doc` holds again as `ask` asks, on a thread of its own; the caret goes to the function at `focus` once the source lands.
    // The decompile already in flight for `doc` is dropped with its job.
    pub(crate) fn redo_gta6_script(&mut self, doc: DocumentId, ask: Ask, focus: Option<u32>) {
        let Some(DocPayload::Gta6Script(script)) = self.document(doc).map(|d| &d.payload) else {
            return;
        };
        let script = Arc::clone(script);
        let slot: Arc<Mutex<Option<Done>>> = Arc::new(Mutex::new(None));
        let (weak, asked) = (Arc::downgrade(&slot), ask.clone());
        let progress = Arc::new(Progress::default());
        let counted = Arc::clone(&progress);
        let work = move || {
            let done = script.again(&asked, &counted);
            if let Some(slot) = weak.upgrade() {
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(done);
            }
        };
        let thread = std::thread::Builder::new().name("gta6-script".to_owned());
        if let Err(e) = thread.spawn(work) {
            return self.show_error_toast(tr!("The script was not decompiled: {e}", e = e));
        }
        let job = Job {
            ask,
            focus,
            started: Instant::now(),
            progress,
            slot,
        };
        self.panels.get::<Jobs>().0.insert(doc, job);
    }

    /// Add the function at `at` to those the source of the large script `doc` holds; nothing where the source holds every function, or this one.
    pub(crate) fn pick_gta6_function(&mut self, doc: DocumentId, at: u32) {
        let Some(DocPayload::Gta6Script(script)) = self.document(doc).map(|d| &d.payload) else {
            return;
        };
        let (held, known) = (script.ask(), script.program.function(at).is_some());
        // ⚠ A decompile in flight is asked again with its own picks: two clicks in a row keep both.
        let mut ask = self
            .gta6_script_job(doc)
            .map_or(held, |job| job.ask.clone());
        let fresh = known && (ask.picked.as_mut()).is_some_and(|picked| picked.insert(at));
        if fresh {
            self.redo_gta6_script(doc, ask, Some(at));
        }
    }

    /// Land each decompile that is done, and drop the job of a script that is closed.
    pub(crate) fn poll_gta6_scripts(&mut self) {
        let jobs = self.panels.peek::<Jobs>();
        if jobs.is_none_or(|jobs| jobs.0.is_empty()) {
            return;
        }
        let docs: Vec<DocumentId> = self.panels.get::<Jobs>().0.keys().copied().collect();
        for doc in docs {
            let open = matches!(
                self.document(doc).map(|d| &d.payload),
                Some(DocPayload::Gta6Script(_))
            );
            if !open {
                self.panels.get::<Jobs>().0.remove(&doc);
                continue;
            }
            let Some(job) = self.gta6_script_job(doc) else {
                continue;
            };
            // ⚠ In this order: a thread that ended has left what it made, or has left nothing for good.
            let ended = Arc::weak_count(&job.slot) == 0;
            let made = job
                .slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let done = match (made, ended) {
                (Some(done), _) => done,
                (None, true) => {
                    Err(tr!("the decompiler stopped on a fault in this program").to_string())
                }
                (None, false) => continue,
            };
            let focus = job.focus;
            self.panels.get::<Jobs>().0.remove(&doc);
            self.land_gta6_script(doc, focus, done);
        }
    }

    /// Put what a decompile made in place of the source `doc` holds, the caret on the function at `focus`.
    // ⚠ A decompile that failed leaves the document the source it had.
    fn land_gta6_script(&mut self, doc: DocumentId, focus: Option<u32>, done: Done) {
        let script = match done {
            Ok(script) => script,
            Err(e) => {
                return self.show_error_toast(tr!("The script was not decompiled: {e}", e = e));
            }
        };
        let was = match self.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Gta6Script(old)) => old.level,
            _ => return,
        };
        // The level chosen is the level the next script opens at, once a source at it exists.
        if script.level != was {
            set_level(script.level);
        }
        let is_focus = |row: &super::OutlineRow| !row.class && focus.is_some() && row.at == focus;
        let row = script.outline.iter().position(is_focus);
        let line = row
            .and_then(|row| script.outline.get(row))
            .map_or(0, |row| row.line);
        // The rows of the new source are other rows.
        if self.selection.selected(doc).is_some() {
            let selected = row.map_or(DOC_ROOT_SEL, |row| row as u64);
            self.selection.select(doc, selected);
        }
        self.replace_gta6_script(doc, DocPayload::Gta6Script(Arc::new(script)));
        let landed = &mut self.panels.get::<ScriptAsk>().landed;
        landed.retain(|(held, _)| *held != doc);
        landed.push((doc, line));
    }
}
