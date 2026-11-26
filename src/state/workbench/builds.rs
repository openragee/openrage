//! Building the project's outputs on a worker, one after the other, and what each build last said.
//!
//! ⚠ **A build is `or_project::build` and nothing else**: it plans, cooks the `.src` folders that
//! changed, writes to a fresh folder, reads every file back, swaps it in and keeps the last good build. This says what it did and never that the
//! build loads in game, which nobody here can run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::SystemTime;

use or_io::gta5::shaders::shader_defs::ParamRemap;
use or_job::Snapshot;
use or_project::acceptance::{kinds_in, Sheet};
use or_project::{
    build_all_watched, check_reproducible, BuildError, BuildStep, Container, Cooked, Diff, Loss,
    Memory, MetadataCount, ModuleKind, NameCheck, Plan, Reproducible, Target, Watch,
};
use or_source::cook::cook_source;
use or_wardrobe::sizes::Sizes;
use or_wardrobe::verify::Check;
use or_wardrobe::Depth;

use super::build_cooker::source_folders;
use super::scan::held;
use super::{build_name, content_name};
use crate::state::plugins::extensions::{extend, pack_document, Extension};

/// The key of a build's own entry that asks for the second, byte-identical build after it.
pub const VERIFY: &str = "verify";

/// Whether the build asks to be built a second time and compared, which cooks every source again.
pub fn verifies(output: &or_project::Output) -> bool {
    output
        .extra
        .get(VERIFY)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// What a build run is asked, all of it owned so a worker can read it.
#[derive(Debug, Clone)]
pub struct Job {
    pub container: Container,
    pub folder: PathBuf,
    /// Where resources land: the project's `build/` folder.
    pub root: PathBuf,
    pub members: Vec<PathBuf>,
    /// The ids of the outputs to build, in order.
    pub outputs: Vec<String>,
    /// The shader database a drawable that moves to another target is respelled through.
    pub remap: Arc<ParamRemap>,
    /// The plugins that read each plan, to validate it or to add files to it.
    pub extensions: Vec<Extension>,
    /// Whether to build each resource a second time into a scratch folder and compare, which cooks every source again.
    pub reproducible: bool,
}

/// What a build wrote and checked: one resource, or each of the resources an output split into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltSummary {
    pub at: SystemTime,
    pub parts: Vec<BuiltPart>,
    /// What freezing the drawables of the clothing it shipped did, one sentence for each content that has them frozen now or could not be.
    pub frozen: Vec<String>,
}

/// One resource a build wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltPart {
    pub resource: String,
    pub folder: PathBuf,
    pub files: u64,
    pub bytes: u64,
    /// Files read back and compared with the plan, byte for byte.
    pub checked: usize,
    /// Every file a reader of OpenRage opened, and the reader that did; never cut short.
    pub reloaded: Vec<(String, String)>,
    /// The metadata documents reloaded, by what they are.
    pub metadata: Vec<MetadataCount>,
    /// The names the game resolves once that the build checked, and the ones held twice.
    pub names: NameCheck,
    /// The streaming memory of the RSC7 files it wrote, against the build's ceiling.
    pub memory: Memory,
    /// Whether a second build wrote the same bytes; not run unless asked.
    pub reproducible: Reproducible,
    /// The build it replaced is kept beside it.
    pub kept_previous: bool,
    /// What differs from the build it replaced, by stable id; none when that build recorded no items.
    pub since: Option<Diff>,
    /// What a file dropped on its way to the output's target, because the target has no field for it.
    pub losses: Vec<Loss>,
    /// The sources the build cooked and shipped: the path of the file in the resource and the bytes it came out at.
    pub cooked: Vec<Cooked>,
    /// The sources of the game's own content that are still the game's file, which the build did not ship.
    pub unchanged: Vec<String>,
    /// The line `dlclist.xml` needs for a pack, as the path the game mounts it from.
    pub dlclist_entry: Option<String>,
    /// What the clothing engine read back of the resource against each pack it was made of, by content.
    pub pack_checks: Vec<(String, Vec<Check>)>,
    /// What the files of each pack weigh by garment and slot, as the build planned them and at the sizes the cooks came out at.
    pub sizes: Vec<(String, Sizes)>,
    /// The kinds of file it wrote, as the acceptance sheet names a kind: read once from the folder, by the worker.
    pub kinds: BTreeSet<String>,
    /// The edition it was written for, none when the build stated none and shipped each file as it was made.
    pub target: Option<Target>,
}

impl BuiltSummary {
    pub fn files(&self) -> u64 {
        self.parts.iter().map(|p| p.files).sum()
    }

    pub fn checked(&self) -> usize {
        self.parts.iter().map(|p| p.checked).sum()
    }

    /// One sentence for each check of the clothing engine that failed.
    pub fn pack_problems(&self) -> Vec<String> {
        (self.parts.iter())
            .flat_map(|p| &p.pack_checks)
            .flat_map(|(module, checks)| {
                (checks.iter())
                    .filter(|c| !c.passed)
                    .map(move |c| format!("{module}: {}: {}", c.name, c.detail))
            })
            .collect()
    }
}

/// How a build last ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastBuild {
    Built(BuiltSummary),
    /// Why it was refused or stopped; the last good build is as it was.
    Failed(Vec<String>),
}

/// The state dot beside the Build button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildDot {
    /// Nothing has been built in this session.
    Never,
    Running,
    Built,
    Failed,
    /// The plan has a problem, so a build would be refused.
    Blocked,
    /// Written and read back, and the clothing engine's own check of what was written found a problem.
    Unchecked,
}

impl BuildDot {
    /// What the Build button says beside its dot, so the state does not rest on a colour: `counts` is what the build's plan holds, which a blocked one says by its errors.
    pub fn word(self, counts: super::Counts) -> String {
        match self {
            Self::Never => tr!("Not built").to_string(),
            Self::Running => tr!("Building…").to_string(),
            Self::Built => tr!("Built").to_string(),
            Self::Failed => tr!("Failed").to_string(),
            Self::Blocked if counts.errors == 1 => tr!("Blocked by 1 error").to_string(),
            Self::Blocked if counts.errors > 1 => {
                tr!("Blocked by {errors} errors", errors = counts.errors)
            }
            Self::Blocked => tr!("Blocked").to_string(),
            Self::Unchecked => tr!("Check failed").to_string(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Never => tr!("Not built yet"),
            Self::Running => tr!("Building"),
            Self::Built => tr!("Built and read back"),
            Self::Failed => tr!("The last build failed"),
            Self::Blocked => tr!("The plan has a problem a build would stop on"),
            Self::Unchecked => tr!("Built and read back, but the pack check found a problem"),
        }
    }
}

enum Message {
    Started(String),
    /// A build ended, and the step it was on: of [`STEPS`], the last one it entered.
    Finished(String, LastBuild, u8),
    Done,
}

/// What each step of a build is called where a screen lists them: the plan, then the steps `or_project` takes.
pub const STEPS: [&str; 5] = [
    tr_key!("Plan"),
    tr_key!("Cook what changed"),
    tr_key!("Write to a fresh folder"),
    tr_key!("Read every file back"),
    tr_key!("Swap it in"),
];

struct Running {
    /// The folder of the project that asked for the run, which may be closed or replaced by another before it ends.
    folder: PathBuf,
    messages: Receiver<Message>,
    outputs: Vec<String>,
    /// The name of each output of the run, as the project that asked for it called it.
    names: BTreeMap<String, String>,
    current: Option<String>,
    stop: Arc<AtomicBool>,
    /// The step the worker last entered, as an index of [`STEPS`].
    step: Arc<AtomicU8>,
    /// The file the build is on and how many bytes of its plan are done, and the flag that stops the one being written.
    watch: Watch,
}

/// The build running, if one is, and what each output last built.
pub struct Builds {
    running: Option<Running>,
    last: BTreeMap<String, LastBuild>,
    /// The step of [`STEPS`] each build that stopped was on, by id.
    stopped: BTreeMap<String, usize>,
    /// What people recorded of a running game, which a build's kinds of file are read against: the sheet the tree ships.
    sheet: Sheet,
}

impl Default for Builds {
    fn default() -> Self {
        // ⚠ A sheet that does not read says nothing of any build, and the tab's last line still says that no build is tested in game.
        let sheet = Sheet::shipped().unwrap_or_else(|why| {
            tracing::error!("{why}");
            Sheet::default()
        });
        Self {
            running: None,
            last: BTreeMap::new(),
            stopped: BTreeMap::new(),
            sheet,
        }
    }
}

impl Builds {
    /// The acceptance sheet the builds are read against.
    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    /// Read the builds against `sheet` in place of the one the tree ships.
    #[cfg(test)]
    pub fn read_against(&mut self, sheet: Sheet) {
        self.sheet = sheet;
    }

    /// Start `job` on a worker; `false` when a build is already running.
    pub fn start(&mut self, job: Job) -> bool {
        if self.running.is_some() {
            return false;
        }
        let (tx, messages) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let outputs = job.outputs.clone();
        let names = (outputs.iter())
            .map(|id| (id.clone(), build_name(&job.container, id)))
            .collect();
        let folder = job.folder.clone();
        let flag = stop.clone();
        let step = Arc::new(AtomicU8::new(0));
        let entered = step.clone();
        let watch = Watch::default();
        let watched = watch.clone();
        std::thread::spawn(move || run(&job, (&flag, &entered, &watched), &tx));
        self.running = Some(Running {
            folder,
            messages,
            outputs,
            names,
            current: None,
            stop,
            step,
            watch,
        });
        true
    }

    /// The output being built now and how many the run holds, while one runs.
    pub fn running(&self) -> Option<(&str, usize)> {
        let running = self.running.as_ref()?;
        let current = running
            .current
            .as_deref()
            .or(running.outputs.first().map(String::as_str))?;
        Some((current, running.outputs.len()))
    }

    /// The step the build running is on, as an index of [`STEPS`]; the last one it entered, since a build that has ended is not running.
    pub fn step(&self) -> Option<usize> {
        let running = self.running.as_ref()?;
        Some(usize::from(running.step.load(Ordering::Relaxed)))
    }

    /// Which of the outputs of the run is being built, from 0, and how many the run holds.
    pub fn place_in_the_run(&self) -> Option<(usize, usize)> {
        let running = self.running.as_ref()?;
        let current = running
            .current
            .as_deref()
            .or(running.outputs.first().map(String::as_str))?;
        let at = running.outputs.iter().position(|id| id == current)?;
        Some((at, running.outputs.len()))
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// The name the project that started the run in progress gave each of its builds, by id, which a project closed since no longer says.
    pub fn running_names(&self) -> BTreeMap<String, String> {
        self.running
            .as_ref()
            .map_or_else(BTreeMap::new, |running| running.names.clone())
    }

    /// The folder of the project the run in progress was started for.
    pub fn running_folder(&self) -> Option<&std::path::Path> {
        self.running
            .as_ref()
            .map(|running| running.folder.as_path())
    }

    /// Take `last` as how `output` last ended, as a worker's message does.
    #[cfg(test)]
    pub fn keep(&mut self, output: &str, last: LastBuild) {
        self.last.insert(output.to_string(), last);
    }

    /// Stop the build being written, which removes the folder it was writing and leaves the last good build as it was, and begin no other of the run.
    pub fn stop(&mut self) {
        if let Some(running) = &self.running {
            running.stop.store(true, Ordering::Relaxed);
            running.watch.cancel.set();
        }
    }

    /// The file the running build is on and how many bytes of its plan are done in the step it is on.
    pub fn progress(&self) -> Option<Snapshot> {
        Some(self.running.as_ref()?.watch.progress.snapshot())
    }

    pub fn is_stopping(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.stop.load(Ordering::Relaxed))
    }

    /// The builds that ended since the last poll.
    pub fn poll(&mut self) -> Vec<(String, LastBuild)> {
        let mut ended = Vec::new();
        let Some(running) = self.running.as_mut() else {
            return ended;
        };
        let mut done = false;
        for message in running.messages.try_iter() {
            match message {
                Message::Started(id) => running.current = Some(id),
                Message::Finished(id, last, at) => {
                    match &last {
                        LastBuild::Failed(_) => self.stopped.insert(id.clone(), usize::from(at)),
                        LastBuild::Built(_) => self.stopped.remove(&id),
                    };
                    self.last.insert(id.clone(), last.clone());
                    ended.push((id, last));
                }
                Message::Done => done = true,
            }
        }
        // ⚠ A worker that panicked drops its sender with no Done, and the run would stay forever.
        if done
            || matches!(
                running.messages.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Disconnected)
            )
        {
            self.running = None;
        }
        ended
    }

    /// What survives a change of project: a run still going, and none of the results, which belong to the project that made them.
    pub fn kept_while_running(mut self) -> Self {
        self.last.clear();
        self.stopped.clear();
        if self.running.is_some() {
            self
        } else {
            Self::default()
        }
    }

    /// Forget how `output` last ended, as a clean does.
    pub fn forget(&mut self, output: &str) {
        self.last.remove(output);
        self.stopped.remove(output);
    }

    /// How `output` last ended in this session.
    pub fn last(&self, output: &str) -> Option<&LastBuild> {
        self.last.get(output)
    }

    /// The step of [`STEPS`] the build of `output` that stopped was on, as an index; none while it is not known, as for a build that ended before this session read it.
    pub fn stopped_at(&self, output: &str) -> Option<usize> {
        self.stopped.get(output).copied()
    }

    /// The builds whose last run stopped in this session, and why, by id.
    pub fn failed(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.last.iter().filter_map(|(id, last)| match last {
            LastBuild::Failed(reasons) => Some((id.as_str(), reasons.as_slice())),
            LastBuild::Built(_) => None,
        })
    }

    /// The dot for `output`; `blocked` says its plan has a problem.
    pub fn dot(&self, output: &str, blocked: bool) -> BuildDot {
        if self.running().is_some_and(|(current, _)| current == output) {
            return BuildDot::Running;
        }
        match self.last(output) {
            Some(LastBuild::Failed(_)) => BuildDot::Failed,
            _ if blocked => BuildDot::Blocked,
            Some(LastBuild::Built(done)) if !done.pack_problems().is_empty() => BuildDot::Unchecked,
            Some(LastBuild::Built(_)) => BuildDot::Built,
            None => BuildDot::Never,
        }
    }
}

fn run(job: &Job, (stop, step, watch): (&AtomicBool, &AtomicU8, &Watch), tx: &Sender<Message>) {
    for id in &job.outputs {
        let _ = tx.send(Message::Started(id.clone()));
        let last = one(job, id, (step, watch));
        let _ = tx.send(Message::Finished(
            id.clone(),
            last,
            step.load(Ordering::Relaxed),
        ));
        if stop.load(Ordering::Relaxed) {
            break;
        }
    }
    let _ = tx.send(Message::Done);
}

/// Plan `id` from the files as the disk shows them now, and build each resource it makes.
///
/// ⚠ **`or_project::build_all`, as `openrage-cli project build` runs it**: every resource of a split
/// output is checked before the first is written, and one that fails while it is written leaves the
/// resources before it built.
fn one(job: &Job, id: &str, (step, watch): (&AtomicU8, &Watch)) -> LastBuild {
    step.store(0, Ordering::Relaxed);
    let folders = match source_folders(&job.container, &job.folder, &job.members) {
        Ok(folders) => folders,
        Err(why) => return LastBuild::Failed(vec![why]),
    };
    let planned = match or_content::plans_from(
        &job.container,
        id,
        &job.folder,
        &held(&job.members),
        &folders,
    ) {
        Ok(planned) => planned,
        Err(e) => return LastBuild::Failed(vec![e.to_string()]),
    };
    // ⚠ The plan the table showed, with what the plugins added and the errors they found: a build writes exactly that.
    let pack = if job.extensions.iter().any(|e| e.reads_pack) {
        pack_document(&job.container, &job.folder, id)
    } else {
        Vec::new()
    };
    let planned: Vec<_> = (planned.into_iter())
        .map(|plan| extend(&job.extensions, plan, &pack).plan)
        .collect();
    // ⚠ The build's steps follow the plan, so each is one more than its place in `or_project`'s list.
    let entered = |entered: BuildStep| step.store(entered as u8 + 1, Ordering::Relaxed);
    let (mut built, stopped) = build_all_watched(
        &planned,
        &job.root,
        &job.remap,
        Some(&cook_source),
        &entered,
        watch,
    );
    if let Some(why) = stopped {
        let mut reasons = match why {
            BuildError::Refused(problems) => problems,
            BuildError::Unverified(found) => found.problems,
            BuildError::Stopped => vec![
                tr!("Stopped before it finished; the folder it was writing is removed and the last good build is as it was").to_string(),
            ],
            other => vec![other.to_string()],
        };
        if !built.is_empty() {
            reasons.push(tr!(
                "{count} resource(s) written before it stay built",
                count = built.len()
            ));
        }
        return LastBuild::Failed(reasons);
    }
    if job.reproducible || job.container.output(id).is_some_and(verifies) {
        for (plan, one) in planned.iter().zip(&mut built) {
            check_reproducible(plan, one, &job.root, &job.remap, Some(&cook_source));
        }
    }
    let split = planned.len() > 1;
    let parts = planned
        .iter()
        .zip(built)
        .map(|(plan, built)| {
            let (pack_checks, sizes) =
                pack_checks(job, id, plan, &built.folder, split, &built.cooked);
            let kinds = match kinds_in(&built.folder) {
                Ok(counted) => counted.into_keys().collect(),
                Err(why) => {
                    tracing::warn!("the kinds of file of a build were not read: {why}");
                    BTreeSet::new()
                }
            };
            BuiltPart {
                kinds,
                target: plan.target,
                resource: plan.resource.clone(),
                files: built.written.files,
                bytes: built.written.bytes,
                checked: built.verification.checked,
                pack_checks,
                sizes,
                folder: built.folder,
                metadata: built.verification.metadata(),
                names: built.verification.names,
                memory: built.verification.memory,
                reproducible: built.verification.reproducible,
                reloaded: built
                    .verification
                    .reloaded
                    .into_iter()
                    .map(|r| (r.path, r.reader))
                    .collect(),
                kept_previous: built.previous.is_some(),
                since: built.since,
                losses: built.losses,
                cooked: built.cooked,
                unchanged: built.unchanged.into_iter().map(|u| u.path).collect(),
                dlclist_entry: plan.dlclist_entry.clone(),
            }
        })
        .collect();
    LastBuild::Built(BuiltSummary {
        at: SystemTime::now(),
        parts,
        frozen: frozen(job, id),
    })
}

/// Freeze the drawables of each clothing content the build `id` shipped, now that it went through, and say what that did.
fn frozen(job: &Job, id: &str) -> Vec<String> {
    use or_wardrobe::freeze::{keep_for, Kept};
    keep_for(&job.container, &job.folder, id)
        .into_iter()
        .filter_map(|(module, kept)| {
            let module = content_name(&job.container, &module);
            match kept {
                Ok(Kept::Wrote { first: true, places }) => Some(tr!("The places of the {places} garments of {module} are locked, so a later build cannot move one unless a reason says why", places = places, module = module)),
                Ok(Kept::Wrote { places, .. }) => Some(tr!("The places of the {places} garments of {module} are locked again, as they are now", places = places, module = module)),
                Ok(Kept::Same | Kept::Off) => None,
                Err(why) => Some(tr!("The places of the garments of {module} could not be locked: {why}", module = module, why = why)),
            }
        })
        .collect()
}

/// What each content read back, by content.
type Checked = Vec<(String, Vec<Check>)>;

/// What the engines of the content read back of what the output `id` wrote in `resource`: each pack and each module of peds that `plan` holds,
/// and what the files of each pack weigh with the sources `cooked` at the sizes they came out at.
///
/// ⚠ A split divides between contents and never cuts one, so a pack is whole in one resource: the
/// one whose plan lists the files that pack contributes, and so is a module of peds. A resource is read against its own content only.
fn pack_checks(
    job: &Job,
    id: &str,
    plan: &Plan,
    resource: &std::path::Path,
    split: bool,
    cooked: &[Cooked],
) -> (Checked, Vec<(String, Sizes)>) {
    let Some(output) = job.container.output(id) else {
        return (Vec::new(), Vec::new());
    };
    let (mut found, mut weighed) = (Vec::new(), Vec::new());
    for module in (job.container.modules.iter())
        .filter(|m| m.kind == ModuleKind::Clothing && output.take.contains(&m.id))
    {
        // ⚠ Planned again from the files as they are now, which are the ones the build copied, for the build it was written by: its layout and its options.
        if let Ok((wardrobe, planned)) =
            or_wardrobe::planned_for(&job.folder, module, Depth::Digest, output)
        {
            // A package installs a pack below the folder it adds, so a path of the pack is the end of the plan's.
            let held = (planned.contribution.files.iter()).any(|f| {
                plan.files
                    .iter()
                    .any(|p| p.path == f.path || p.path.ends_with(&format!("/{}", f.path)))
            });
            if split && !held {
                continue;
            }
            let checks = match or_project::built_files(plan, resource) {
                Ok(files) => {
                    or_wardrobe::verify::verify_files(&wardrobe, &planned, files.as_ref()).checks
                }
                Err(why) => vec![Check {
                    name: "build".to_string(),
                    passed: false,
                    detail: why,
                }],
            };
            found.push((module.id.clone(), checks));
            let cooked = as_the_pack_names(cooked, &planned);
            weighed.push((
                module.id.clone(),
                or_wardrobe::sizes::sizes(&wardrobe, &planned, &cooked),
            ));
        }
    }
    for module in (job.container.modules.iter())
        .filter(|m| m.kind == ModuleKind::AddonPed && output.take.contains(&m.id))
    {
        if let Ok((model, planned)) = or_addonped::planned(&job.folder, module, Depth::Stat) {
            let held = (planned.contribution.files.iter())
                .any(|f| plan.files.iter().any(|p| p.path == f.path));
            if split && !held {
                continue;
            }
            let verification = or_addonped::verify::verify(&model, resource);
            found.push((module.id.clone(), verification.checks));
        }
    }
    (found, weighed)
}

/// The sources cooked in a resource, each by the path the pack states for its file: a package installs a pack below the folder it adds.
fn as_the_pack_names(cooked: &[Cooked], planned: &or_wardrobe::Planned) -> Vec<Cooked> {
    (cooked.iter())
        .filter_map(|c| {
            let file = (planned.contribution.files.iter())
                .find(|f| c.path == f.path || c.path.ends_with(&format!("/{}", f.path)))?;
            Some(Cooked {
                path: file.path.clone(),
                ..c.clone()
            })
        })
        .collect()
}
