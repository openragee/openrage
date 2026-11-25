//! The Wardrobe workspace's state: the packs the last scan read, the garment selected, and the edits that run on a worker.
//!
//! ⚠ **Nothing here is a rule or a write of its own.** Reading a pack, planning it, adding dropped
//! files, fixing a finding and recording a waiver are `or-wardrobe`'s; this selects, asks, runs
//! them off the UI thread one after the other and carries what they said back.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Instant, SystemTime};

use or_project::{Container, Contribution, Module, ModuleKind};
use or_wardrobe::edit::{self, Added, Linked, Removed, Revision, Watch};
use or_wardrobe::lock::PackLock;
use or_wardrobe::rules::name_of;
use or_wardrobe::{Allocation, Depth, Finding, Planned, Redo, Wardrobe};

use super::wardrobe_words::{describe, notes_of};
use crate::state::front_door::spoken;
use crate::state::toasts::ToastAction;
use crate::state::AppState;
use crate::ui::dock::Workspace;

/// One clothing content as the last scan read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub module: String,
    pub folder: PathBuf,
    /// The content is a link: another project writes it, so no fix or waiver is offered for it here.
    pub linked: bool,
    /// The pack and what the engine found in it, or why `wardrobe.json` could not be read.
    pub read: Result<PackRead, String>,
    /// For a pack that could not be read: the newest earlier version of it that reads, which can take its place.
    pub last_good: Option<LastGood>,
}

/// The newest version of a pack that reads.
#[derive(Debug, Clone, PartialEq)]
pub struct LastGood {
    pub label: String,
    pub at: SystemTime,
    pub garments: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackRead {
    pub wardrobe: Wardrobe,
    pub allocation: Allocation,
    pub findings: Vec<Finding>,
    /// What the plan counts: the drawables and the textures its metadata states.
    pub drawables: u64,
    pub textures: u64,
    /// What the files say of each garment that a query asks of, in the pack's order.
    pub counts: Vec<or_wardrobe::query::Counts>,
    /// What the files the plan writes for the pack come to, by garment and by slot.
    pub sizes: or_wardrobe::sizes::Sizes,
    /// The files and sources the garments name, once each, with where each leads.
    pub dependencies: Vec<or_wardrobe::sizes::Dependency>,
}

/// Read and plan every clothing content of `container`, as the engine does for a build.
///
/// ⚠ Runs on the scan's worker. The contributions are returned beside the packs so that the plan of
/// each build is made of the very reading the screen shows.
pub(super) fn read_packs(container: &Container, folder: &Path) -> (Vec<Pack>, Vec<Contribution>) {
    let (mut packs, mut handed) = (Vec::new(), Vec::new());
    for module in container
        .modules
        .iter()
        .filter(|m| m.kind == ModuleKind::Clothing)
    {
        let at =
            or_wardrobe::module_folder(folder, module).unwrap_or_else(|_| folder.to_path_buf());
        match or_wardrobe::planned_with_model(folder, module, Depth::Measure) {
            Ok((wardrobe, planned)) => {
                // What this scan read is kept for the next session, in the folder of a pack that this project writes and never in a linked one.
                if !module.is_link() {
                    or_wardrobe::cache::save(&at);
                }
                let sizes = or_wardrobe::sizes::sizes(&wardrobe, &planned, &[]);
                let Planned {
                    contribution,
                    allocation,
                    findings,
                    counts,
                    dependencies,
                    ..
                } = planned;
                let figure = |name: &str| {
                    (contribution.figures.iter())
                        .find(|f| f.name == name)
                        .map_or(0, |f| f.amount)
                };
                let read = PackRead {
                    drawables: figure("clothing_drawables"),
                    textures: figure("clothing_textures"),
                    wardrobe,
                    allocation,
                    findings,
                    counts,
                    sizes,
                    dependencies,
                };
                handed.push(contribution);
                packs.push(Pack {
                    module: module.id.clone(),
                    folder: at,
                    linked: module.is_link(),
                    read: Ok(read),
                    last_good: None,
                });
            }
            Err(e) => {
                let mut contribution = Contribution::of(&module.id);
                contribution.problems.push(e.to_string());
                handed.push(contribution);
                let last_good =
                    or_wardrobe::backup::last_good(&at).map(|(version, held)| LastGood {
                        label: version.label,
                        at: version.at,
                        garments: held.garments.len(),
                    });
                packs.push(Pack {
                    module: module.id.clone(),
                    folder: at,
                    linked: module.is_link(),
                    read: Err(e.to_string()),
                    last_good,
                });
            }
        }
    }
    (packs, handed)
}

/// What an edit that ran on the worker said, and what it did that Undo can take back.
pub(super) struct Done {
    said: Result<String, String>,
    change: Option<Change>,
    /// What the step of Undo this makes is called, where it is not the sentence: the word its button ends with, and what it takes back.
    step: Option<(&'static str, String)>,
    /// Lines for the Log that the sentence summarises: what a drop left out and what its shop metadata listed.
    notes: Vec<String>,
    /// The steps an Undo took off the stack and the worker refused to take back, which go back where their numbers put them.
    restore: Vec<Step>,
    /// What a worker that took a change back, made it again or was refused a Redo hands to the stacks.
    pub(super) moved: Option<Moved>,
}

/// What an Undo left for Redo: the steps it took back, and the Redo that makes them again.
#[derive(Debug, Clone, PartialEq)]
pub struct Undone {
    pub steps: Vec<Step>,
    pub redo: Redo,
    /// The content, and its folder, which the Redo is made in.
    pub module: String,
    pub folder: PathBuf,
}

/// Which way a change moved between the stack of Undo and the stack of Redo.
pub(super) enum Moved {
    /// Taken back: its steps wait for a Redo.
    Undone(Undone),
    /// Made again: its steps are steps of Undo again.
    Redone(Undone),
    /// A Redo that was refused: it waits as it did.
    RedoRefused(Undone),
}

impl Done {
    pub(super) fn said(said: Result<String, String>) -> Self {
        Self {
            said,
            change: None,
            step: None,
            notes: Vec::new(),
            restore: Vec::new(),
            moved: None,
        }
    }

    /// Lines for the Log that the sentence summarises.
    pub(super) fn with_notes(self, notes: Vec<String>) -> Self {
        Self { notes, ..self }
    }

    pub(super) fn with(said: String, change: Change) -> Self {
        Self {
            change: Some(change),
            ..Self::said(Ok(said))
        }
    }

    /// An edit of the pack's own record: a step of Undo named `step`, and a sentence, which may be empty for an edit that is only a step.
    pub(super) fn revised(
        said: String,
        step: (&'static str, String),
        revision: Option<Revision>,
    ) -> Self {
        Self {
            change: revision.map(|r| Change::Revised(Box::new(r))),
            step: Some(step),
            ..Self::said(Ok(said))
        }
    }
}

pub(super) type Work = Box<dyn FnOnce() -> Done + Send>;

/// A change of the Wardrobe that Undo takes back: a drop, a garment taken out of the pack, or a cloth linked or unlinked.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Boxed: a drop holds the report of the shop metadata it read.
    Added(Box<Added>),
    /// Boxed: a whole garment is the larger by far.
    Removed(Box<Removed>),
    Cloth(Box<Linked>),
    /// Any other edit of the pack's record: the pack as it was and as the edit left it.
    Revised(Box<Revision>),
}

/// A change that can be taken back, under the number its toast's Undo names.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub number: u64,
    /// The content it was made in.
    pub module: String,
    /// The word the Undo button ends with.
    pub noun: &'static str,
    /// What taking it back undoes, as a sentence.
    pub label: String,
    pub change: Change,
    /// When it was made, which orders it among the project's steps for the one Undo of the workspace.
    pub at: SystemTime,
}

/// ⚠ One count for the whole process, so the Undo of a toast left over from another project never names a change of this one.
static STEPS: AtomicU64 = AtomicU64::new(0);

/// The Wardrobe workspace's own state, reset with the project.
#[derive(Default)]
pub struct WardrobeWork {
    /// The garment selected: its content and its id.
    pub selected: Option<(String, String)>,
    /// The garments picked in the list, the one selected among them; empty or stale while nothing is selected ([`AppState::picked_garments`]).
    pub(super) picked: Vec<String>,
    /// The garment a Shift click or Shift and an arrow spans from.
    pub(super) anchor: Option<String>,
    /// The quick look is open on the garment selected.
    pub quick_look: bool,
    /// The picture the quick look draws, read off the UI thread.
    pub glance: super::Glance,
    /// F2 asked for the garment's name to be typed, which the Garment panel answers with the keyboard.
    pub rename_asked: bool,
    /// The clothing content the workspace shows; the first when none is chosen.
    pub module: Option<String>,
    /// Bumped when what the scan read of the packs changes, so a list is laid out again; it never starts over with a project.
    pub generation: u64,
    /// The drops and removals Undo can take back, newest last.
    pub undo: Vec<Step>,
    /// What Undo took back and Redo can make again, newest last; a new change empties it.
    pub redo: Vec<Undone>,
    /// Which stack Undo took from, oldest first, so that Redo makes again in the order Undo took back.
    pub trail: Vec<super::Side>,
    /// The right to write each pack of this session, by its folder, held until the project is left.
    pub(super) locks: HashMap<PathBuf, PackLock>,
    /// The contents another window holds, which this one reads and does not write.
    pub locked_out: HashSet<String>,
    pub(super) last_retry: Option<Instant>,
    /// The versions of the pack shown, read when asked for: the content, the scan they were read at, and each version with its pack.
    pub versions: Option<super::Versions>,
    /// What the dressed ped shows and how it is set.
    pub dressed: super::Dressed,
    /// What the mesh doctor keeps between frames.
    pub mesh: super::MeshDesk,
    /// What the texture steps keep between frames: a budget plan, its pictures and the folders bound to slots.
    pub textures: super::TextureDesk,
    running: Option<Running>,
    queue: VecDeque<(String, String, Work, Option<Watch>)>,
}

struct Running {
    label: String,
    module: String,
    receiver: Receiver<Done>,
    /// How far the edit has got and the flag that stops it, for an edit that can be stopped.
    watch: Option<Watch>,
}

impl WardrobeWork {
    /// What the edit running has done so far, for the header: its label and, where it counts, how many of how many.
    pub fn progress_words(&self) -> Option<String> {
        let running = self.running.as_ref()?;
        let Some(watch) = &running.watch else {
            return Some(running.label.clone());
        };
        if watch.cancel.is_set() {
            return Some(tr!("{running}: stopping", running = running.label));
        }
        let at = watch.progress.snapshot();
        Some(match at.total {
            0 => format!("{}: {}", running.label, at.item),
            total => tr!(
                "{running}: {attempted} of {total}",
                running = running.label,
                attempted = at.attempted.min(total),
                total = total
            ),
        })
    }

    /// Whether the edit running can be stopped, and has not been asked to be.
    pub fn stoppable(&self) -> bool {
        (self.running.as_ref())
            .is_some_and(|r| r.watch.as_ref().is_some_and(|w| !w.cancel.is_set()))
    }

    /// Ask the edit running to stop; the pack stays as it was.
    pub fn stop(&self) {
        if let Some(watch) = self.running.as_ref().and_then(|r| r.watch.as_ref()) {
            watch.cancel.set();
        }
    }

    /// Whether no edit runs and none waits for the one that does.
    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.running.is_none() && self.queue.is_empty()
    }

    /// What the Undo button of the header says: the change it would take back.
    pub fn undo_words(&self) -> String {
        match self.undo.last() {
            Some(Step {
                change: Change::Added(added),
                ..
            }) if added.garments.is_empty() => tr!("Undo shop").to_string(),
            Some(Step {
                change: Change::Added(added),
                ..
            }) => tr!("Undo add ({count})", count = added.garments.len()),
            Some(Step {
                change: Change::Removed(_),
                ..
            }) => tr!("Undo remove").to_string(),
            Some(Step {
                change: Change::Cloth(_),
                ..
            }) => tr!("Undo cloth").to_string(),
            Some(step) => tr!("Undo {noun}", noun = step.noun),
            None => tr!("Undo").to_string(),
        }
    }

    /// What the Undo button would take back, as a sentence, for its hover.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|step| step.label.as_str())
    }
}

impl AppState {
    /// The project's clothing content.
    pub fn clothing_modules(&self) -> Vec<&Module> {
        (self.project_carried.container.modules.iter())
            .filter(|m| m.kind == ModuleKind::Clothing)
            .collect()
    }

    pub fn has_clothing(&self) -> bool {
        self.project_path.is_some() && !self.clothing_modules().is_empty()
    }

    /// The id of the clothing content the workspace shows: the one chosen if it is still there, else the first.
    pub fn wardrobe_module_id(&self) -> Option<String> {
        let all = self.clothing_modules();
        (self.workbench.wardrobe.module.as_ref())
            .and_then(|id| all.iter().find(|m| &m.id == id))
            .or(all.first())
            .map(|m| m.id.clone())
    }

    /// What the last scan read of the content the workspace shows.
    pub fn wardrobe_pack(&self) -> Option<&Pack> {
        let id = self.wardrobe_module_id()?;
        self.workbench.packs().iter().find(|p| p.module == id)
    }

    /// Whether no change to the clothing runs or waits to.
    pub fn wardrobe_is_idle(&self) -> bool {
        self.workbench.wardrobe.running.is_none() && self.workbench.wardrobe.queue.is_empty()
    }

    /// The garment selected in the content shown, and its pack.
    pub fn selected_garment(&self) -> Option<(&Pack, usize)> {
        let (module, id) = self.workbench.wardrobe.selected.as_ref()?;
        let pack = self
            .workbench
            .packs()
            .iter()
            .find(|p| &p.module == module)?;
        let at = pack
            .read
            .as_ref()
            .ok()?
            .wardrobe
            .garments
            .iter()
            .position(|g| &g.id == id)?;
        Some((pack, at))
    }

    pub fn select_garment(&mut self, module: &str, id: Option<String>) {
        let work = &mut self.workbench.wardrobe;
        work.picked = id.iter().cloned().collect();
        work.anchor = id.clone();
        work.selected = id.map(|id| (module.to_string(), id));
        work.module = Some(module.to_string());
    }

    /// Show a garment: the Wardrobe workspace, with the garment selected.
    pub fn show_garment(&mut self, module: &str, id: String) {
        self.select_garment(module, Some(id));
        self.active_workspace = Workspace::Wardrobe;
    }

    /// Run `work` on a worker once the edits before it are done; `label` is what the header says meanwhile.
    pub(super) fn wardrobe_job(&mut self, label: impl Into<String>, module: &str, work: Work) {
        self.workbench
            .wardrobe
            .queue
            .push_back((label.into(), module.to_string(), work, None));
    }

    /// [`Self::wardrobe_job`] for an edit that tells how far it has got and stops when `watch` says so.
    pub(super) fn wardrobe_job_watched(
        &mut self,
        label: impl Into<String>,
        module: &str,
        watch: Watch,
        work: Work,
    ) {
        let queue = &mut self.workbench.wardrobe.queue;
        queue.push_back((label.into(), module.to_string(), work, Some(watch)));
    }

    /// Start the next edit when none runs, and take what the one that ran said.
    pub(super) fn poll_wardrobe(&mut self) {
        let mut finished = None;
        if let Some(running) = &self.workbench.wardrobe.running {
            match running.receiver.try_recv() {
                Ok(done) => finished = Some(Some(done)),
                Err(TryRecvError::Empty) => {}
                // ⚠ A worker that panicked drops its sender; the edit is lost and the next one runs.
                Err(TryRecvError::Disconnected) => finished = Some(None),
            }
        }
        if let Some(done) = finished {
            let running = self.workbench.wardrobe.running.take();
            self.workbench.want_scan();
            match (done, running) {
                (Some(done), Some(running)) => {
                    self.wardrobe_done(done, running.label, running.module)
                }
                _ => self
                    .show_error_toast(tr!("A change to the clothing stopped before it was done")),
            }
        }
        self.watch_locks();
        if self.workbench.wardrobe.running.is_none() {
            if let Some((label, module, work, watch)) = self.workbench.wardrobe.queue.pop_front() {
                let (tx, receiver) = channel();
                std::thread::spawn(move || {
                    let _ = tx.send(work());
                });
                self.workbench.wardrobe.running = Some(Running {
                    label,
                    module,
                    receiver,
                    watch,
                });
            }
        }
    }

    fn wardrobe_done(&mut self, done: Done, what: String, module: String) {
        if let Some(moved) = done.moved {
            self.take_moved(moved);
        }
        match done.said {
            Ok(said) if said.is_empty() && done.change.is_none() => {}
            Ok(said) => {
                // ⚠ A sentence a worker wrote with `(s)` in it is read as a person says it, whoever wrote it.
                let said = spoken(&said);
                let (noun, label) = done.step.unwrap_or(("", said.clone()));
                self.workbench.note(if said.is_empty() {
                    label.clone()
                } else {
                    said.clone()
                });
                for line in done.notes {
                    self.workbench.note(line);
                }
                match done.change {
                    Some(change) => {
                        // ⚠ A new change ends the chance of a Redo: it would be made over a pack that is not the one Undo left.
                        self.end_redo();
                        let number = STEPS.fetch_add(1, Ordering::Relaxed) + 1;
                        self.workbench.wardrobe.undo.push(Step {
                            number,
                            module,
                            noun,
                            label,
                            change,
                            at: SystemTime::now(),
                        });
                        // An edit that says nothing is only a step: the button in the header takes it back.
                        if !said.is_empty() {
                            self.show_toast_with_action(said, ToastAction::Undo(number));
                        }
                    }
                    None => self.show_toast(said),
                }
            }
            Err(why) => {
                // ⚠ An Undo that is refused changed nothing, so its steps stay: the sentence says to take the later steps back first, which needs this one still there.
                // Steps are numbered in the order they were made, so the number says where each goes back.
                let undo = &mut self.workbench.wardrobe.undo;
                for step in done.restore {
                    let at = undo.partition_point(|s| s.number < step.number);
                    undo.insert(at, step);
                }
                self.workbench.note(format!("{what}: {why}"));
                self.show_error_toast(why);
            }
        }
    }

    /// Add the garments the dropped or picked `paths` state to the content shown, copying their files in.
    pub fn add_clothes(&mut self, paths: Vec<PathBuf>) {
        let Some(module) = self.wardrobe_module_id() else {
            self.show_info_toast(tr!(
                "This project has no clothing: Project ▸ Add a clothing pack… adds one"
            ));
            return;
        };
        let Some(folder) = self.folder_to_edit(&module) else {
            return;
        };
        let watch = Watch::default();
        let watched = watch.clone();
        self.wardrobe_job_watched(
            tr!("Adding clothes"),
            &module,
            watch,
            Box::new(
                move || match edit::add_clothes_watched(&folder, &paths, &watched) {
                    Ok(added) => {
                        let (said, notes) = (describe(&added), notes_of(&added));
                        // ⚠ A drop that added nothing and read no shop metadata has nothing to take back.
                        let mut done =
                            match added.garments.is_empty() && added.shop_before.is_none() {
                                true => Done::said(Ok(said)),
                                false => Done::with(said, Change::Added(Box::new(added))),
                            };
                        done.notes = notes;
                        done
                    }
                    // A stop that was asked for is not a failure: the pack is as it was.
                    Err(edit::Error::Stopped) => {
                        Done::said(Ok(tr!("Stopped: nothing was added").to_string()))
                    }
                    Err(why) => Done::said(Err(why.to_string())),
                },
            ),
        );
    }

    /// Take back the newest change: a drop, with its garments and the files it copied, or a removal.
    pub fn undo_last_change(&mut self) {
        if let Some(last) = self.workbench.wardrobe.undo.last().map(|s| s.number) {
            self.undo_change(last);
        }
    }

    /// Take back the change that has this `number`, whatever came after it.
    pub fn undo_change(&mut self, number: u64) {
        let Some(at) = (self.workbench.wardrobe.undo.iter()).position(|s| s.number == number)
        else {
            self.show_info_toast(tr!("That change was taken back already"));
            return;
        };
        let module = self.workbench.wardrobe.undo[at].module.clone();
        let Some(folder) = self.folder_to_edit(&module) else {
            return;
        };
        let step = self.workbench.wardrobe.undo.remove(at);
        // What leaves the stack here goes back if the worker refuses, so a step is only lost by being taken back.
        let mut taken = vec![step.clone()];
        let Step { change, label, .. } = step;
        if let Change::Added(added) = &change {
            // A garment of this drop that was removed since cannot come back: its files go with the drop.
            let goes = |s: &mut Step| match &s.change {
                Change::Removed(r) => s.module == module && added.garments.contains(&r.garment.id),
                Change::Cloth(l) => s.module == module && added.garments.contains(&l.garment),
                Change::Added(_) | Change::Revised(_) => false,
            };
            taken.extend(self.workbench.wardrobe.undo.extract_if(.., goes));
        }
        let held_in = folder.clone();
        // The sentence, and the Redo that makes the change again.
        type Undoing = Box<dyn FnOnce() -> Result<(String, Redo), String> + Send>;
        let work: Undoing = match change {
            Change::Added(added) => Box::new(move || {
                let said = tr!(
                    "Took back the {count} garment(s) of a drop",
                    count = added.garments.len()
                );
                edit::undo(&folder, &added)
                    .map(|redo| (said, redo))
                    .map_err(|e| e.to_string())
            }),
            Change::Removed(removed) => Box::new(move || {
                let said = tr!(
                    "Put {garment} back in the pack",
                    garment = name_of(&removed.garment)
                );
                edit::undo_removal(&folder, &removed)
                    .map(|redo| (said, redo))
                    .map_err(|e| e.to_string())
            }),
            Change::Cloth(linked) => Box::new(move || {
                edit::undo_cloth(&folder, &linked)
                    .map(|redo| (tr!("Took back the change of a cloth").to_string(), redo))
                    .map_err(|e| e.to_string())
            }),
            Change::Revised(revision) => Box::new(move || {
                edit::undo_revision(&folder, &revision)
                    .map(|redo| (tr!("Took back: {label}", label = label), redo))
                    .map_err(|e| e.to_string())
            }),
        };
        let module_of = module.clone();
        self.wardrobe_job(
            tr!("Undoing a change"),
            &module,
            Box::new(move || match work() {
                Ok((said, redo)) => {
                    let mut done = Done::said(Ok(said));
                    done.moved = Some(Moved::Undone(Undone {
                        steps: taken,
                        redo,
                        module: module_of,
                        folder: held_in,
                    }));
                    done
                }
                Err(why) => {
                    let mut done = Done::said(Err(why));
                    done.restore = taken;
                    done
                }
            }),
        );
    }

    /// The name the pack's screens give garment `id` of the content `module`.
    pub(super) fn garment_name(&self, module: &str, id: &str) -> String {
        (self.workbench.packs().iter())
            .find(|p| p.module == module)
            .and_then(|p| p.read.as_ref().ok())
            .and_then(|read| read.wardrobe.garment(id))
            .map_or_else(|| tr!("the garment").to_string(), name_of)
    }

    /// Link the cloth file `source` to garment `id` of the content `module`, copying it into the pack, which a finding may name and the workspace not show. Undo takes it back.
    ///
    /// ⚠ A cloth belongs to one drawable, because the game finds it by the model's name, so it is the
    /// garment's child and `ownsCloth` is written from the link: nothing here states the flag. A cloth has
    /// no source form, so it is the one file kept as it is.
    pub(super) fn link_cloth_in(&mut self, module: String, id: String, source: PathBuf) {
        let Some(folder) = self.folder_to_edit(&module) else {
            return;
        };
        let name = self.garment_name(&module, &id);
        let file = source
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.wardrobe_job(
            tr!("Linking a cloth"),
            &module,
            Box::new(move || match edit::link_cloth(&folder, &id, &source) {
                Ok(linked) => Done::with(
                    tr!(
                        "Linked the cloth {file} to {name}",
                        file = file,
                        name = name
                    ),
                    Change::Cloth(Box::new(linked)),
                ),
                Err(why) => Done::said(Err(why.to_string())),
            }),
        );
    }

    /// Unlink the cloth of garment `id` of the content shown, or take back the claim of one; its file is not touched. Undo links it again.
    pub fn unlink_cloth(&mut self, id: String) {
        if let Some(module) = self.wardrobe_module_id() {
            self.unlink_cloth_in(module, id);
        }
    }

    /// Unlink the cloth of garment `id` of the content `module`.
    pub(super) fn unlink_cloth_in(&mut self, module: String, id: String) {
        let Some(folder) = self.folder_to_edit(&module) else {
            return;
        };
        let name = self.garment_name(&module, &id);
        self.wardrobe_job(
            tr!("Unlinking a cloth"),
            &module,
            Box::new(move || match edit::unlink_cloth(&folder, &id) {
                Ok(linked) => Done::with(
                    tr!("Unlinked the cloth of {name}; its file stays", name = name),
                    Change::Cloth(Box::new(linked)),
                ),
                Err(why) => Done::said(Err(why.to_string())),
            }),
        );
    }

    /// Put `image` into the texture of variation `variation` of garment `id` of the content shown: the texture is made from it, there is no convert button. Undo writes the old file back.
    pub fn replace_variation_image(&mut self, id: String, variation: usize, image: PathBuf) {
        self.replace_texture(id, variation, image);
    }

    /// Put the image dropped on a tile into the texture of that tile's variation.
    pub fn replace_dropped_image(&mut self, image: PathBuf) {
        if let Some((garment, variation)) = self.workbench.wardrobe.dressed.drop_tile.take() {
            self.replace_variation_image(garment, variation, image);
        }
    }

    /// The folder of the clothing content `module` to write in, or a toast that says why this project may not.
    ///
    /// ⚠ **Every write of the Wardrobe asks here**, and the engine answers: the project that holds a
    /// linked folder is its only writer, so a linked pack is refused with the project that edits it.
    pub(super) fn folder_to_edit(&mut self, module: &str) -> Option<PathBuf> {
        let asked = match (
            &self.project_path,
            self.project_carried.container.module(module),
        ) {
            (Some(project), Some(held)) => {
                or_wardrobe::module_folder_to_edit(project, &self.project_carried.container, held)
                    .map_err(|e| e.to_string())
            }
            _ => Err(tr!(
                "The project has no clothing content `{module}`",
                module = module
            )),
        };
        // ⚠ The right to write is asked for here too, so a second window reads a pack and cannot write it.
        match asked.and_then(|folder| self.hold_pack(module, &folder).map(|()| folder)) {
            Ok(folder) => Some(folder),
            Err(why) => {
                self.workbench.note(why.clone());
                self.show_error_toast(why);
                None
            }
        }
    }

    /// Whether the clothing content shown is linked from another project, whose Wardrobe alone edits it.
    pub fn wardrobe_is_linked(&self) -> bool {
        self.wardrobe_module_id()
            .and_then(|id| self.project_carried.container.module(&id))
            .is_some_and(Module::is_link)
    }

    /// The names of the packs the project's clothing contents hold, read from their files.
    pub(super) fn pack_names(&self, project: &Path) -> Vec<String> {
        let folder = or_project::project_folder(project);
        (self.clothing_modules().iter())
            .filter_map(|m| or_wardrobe::module_folder(&folder, m).ok())
            .filter_map(|at| Wardrobe::read(&at).ok())
            .map(|w| w.pack)
            .collect()
    }

    /// Make the project hold clothing: a `clothing` folder with an empty pack, a content for it, and a place for it in every build.
    pub fn add_clothing_content(&mut self) {
        if self.refused_as_read_only() {
            return;
        }
        let Some(project) = self.project_path.clone() else {
            return;
        };
        let container = &self.project_carried.container;
        let taken: Vec<&str> = container.modules.iter().map(|m| m.id.as_str()).collect();
        let id = super::commands::unique("clothing", taken.into_iter());
        // ⚠ Two packs of one name claim the same collections, so a second pack is named for the project and a number.
        let wanted = or_wardrobe::names::identifier_for(&self.project_name().unwrap_or_default());
        let pack = free_pack_name(&wanted, &self.pack_names(&project));
        let folder = or_project::project_folder(&project).join(&id);
        // ⚠ A write at a linked folder is its owner's, also where a hand-edited project file made it the folder this content takes.
        let refused = or_project::links::refuse_write(
            &self.project_carried.container,
            &project,
            &folder.join(or_wardrobe::FILE),
        );
        if let Err(why) = refused {
            self.show_error_toast(why);
            return;
        }
        if let Err(e) = edit::create(&folder, &pack) {
            self.show_error_toast(e.to_string());
            return;
        }
        let container = &mut self.project_carried.container;
        container.modules.push(Module::clothing(&id, &id));
        for output in &mut container.outputs {
            output.take.push(id.clone());
        }
        self.workbench.want_scan();
        self.workbench.wardrobe.module = Some(id.clone());
        self.record_step(tr!("Added the clothing content {id}", id = id));
        self.active_workspace = Workspace::Wardrobe;
        self.show_toast(tr!(
            "Clothing added: drop models and textures on the Wardrobe to fill it"
        ));
    }

    /// Carry out the action a toast offered.
    pub fn run_toast_action(&mut self, action: ToastAction) {
        match action {
            ToastAction::Undo(number) => self.undo_change(number),
            ToastAction::NewProject => {
                self.start_new_project(crate::state::front_door::Template::Empty)
            }
            ToastAction::UndoImport(serial) => self.undo_import_of_toast(serial),
            ToastAction::ShowReport => {
                self.workbench.imports.shown = self.workbench.imports.report().is_some()
            }
            ToastAction::OpenProject => {
                if let Some(path) = crate::platform::file_dialogs::open_project_file() {
                    self.open_project_asking(path);
                }
            }
            ToastAction::ShowBuild => self.show_the_build(),
            ToastAction::RenamePack => {
                self.active_workspace = Workspace::Wardrobe;
                self.workbench.wardrobe.selected = None;
                self.workbench.wardrobe.rename_asked = true;
            }
            ToastAction::SavePack(pack) => self.save_pack(pack),
            ToastAction::ShowLog => {
                let panel = crate::ui::panels::registry::ids::LOG;
                self.workspace_docks.reveal(self.active_workspace, panel);
            }
        }
    }
}

/// The letter the game's files give variation `i` of a drawable.
pub(super) fn variation_letter(i: usize) -> char {
    char::from(b'a' + u8::try_from(i).unwrap_or(25).min(25))
}

pub(super) use or_content::template::free_pack_name;
