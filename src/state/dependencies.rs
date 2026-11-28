//! What a file uses and which maps place it, from the resolver behind
//! `openrage-cli depends`: the active install is read once, as a job, the
//! first time a row's Uses or Placed by asks, and each answer after it is a
//! lookup. A file replaced under the install drops what was read, and the next
//! ask reads it again: a save is never held back for the read.
//!
//! ⚠ Uses lists `or_cli::DependencyIndex::uses_checked`, which `uses`, the
//! call `openrage-cli depends of` makes, runs after reading a map's boxes
//! itself; a menu reads them on a worker instead. Placed by folds the maps
//! through `DependencyIndex::place`, the loop `depends placements` runs, so a
//! menu cannot answer otherwise than the command.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use or_cli::DependencyIndex;
use or_depends::Found;
use or_io::container::RpfCryptoProvider;
use or_job::{Cancel, Progress};
use or_title::vfs::{build_mounted_game_with, DlcCutoff};
use or_title::{GtaInstall, MountOptions};
use or_world_gta5::{Catalogue, Kind, Located, ModelContents, PlacementIndex};

use super::document::{DocumentId, RpfNodeId};
use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::memory::hash_table_bytes_for_len;
use super::{AppState, DocPayload};

const NO_INSTALL: &str = "Only a game install says what a file uses, through the archetypes its \
                          .ytyp files declare, and this file is in none: open it from the \
                          install chosen in Settings ▸ Games";

/// How many maps Placed by lists before it says how many more place the model.
///
/// ⚠ A menu taller than the window runs off it: 31 retail maps placing
/// `prop_tree_pine_01` already reach the bottom of a 900-point screen.
// ponytail: a cut, not a list to scroll; a panel of its own if the whole list is wanted.
const MAPS_LISTED: usize = 25;

/// One file a Uses or Placed by menu lists.
#[derive(Debug, Clone)]
pub struct Listed {
    /// Its name, or its hash and its kind's extension where the install holds none.
    pub name: String,
    /// What it is to the file, or how often the map places it, and why it cannot open where it cannot.
    pub note: String,
    /// Where it is in the install's tree; `None` where nothing there opens.
    pub at: Option<String>,
}

/// What a Uses or Placed by menu shows.
#[derive(Debug, Clone)]
pub enum Answer {
    /// Why it cannot answer.
    Cannot(String),
    /// The install is still being read.
    Looking,
    Listed {
        install: PathBuf,
        files: Arc<[Listed]>,
    },
}

/// A read on a worker.
struct Reading<T> {
    cancel: Cancel,
    progress: Progress,
    receiver: Receiver<Result<T, String>>,
}

impl<T: Send + 'static> Reading<T> {
    fn start(work: impl FnOnce(&Cancel, &Progress) -> Result<T, String> + Send + 'static) -> Self {
        let (cancel, progress) = (Cancel::new(), Progress::new());
        let (tx, receiver) = channel();
        let (worker_cancel, worker_progress) = (cancel.clone(), progress.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work(&worker_cancel, &worker_progress));
        });
        Self {
            cancel,
            progress,
            receiver,
        }
    }
}

/// ⚠ A read dropped, by a Cancel or by another install, tells its worker to stop.
impl<T> Drop for Reading<T> {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

enum Stage<T> {
    Reading(Reading<T>),
    /// `Err` says why there is no answer.
    Read(Result<Arc<T>, String>),
}

/// The install read and how: another install, mount or set of keys reads it again.
type Install = (PathBuf, MountOptions, bool);

/// The active install's dependency index and its placements, each read once.
// ponytail: read once per install, and all of it again after any file under it
// is replaced (75 to 90 s over retail); an index that knew its archives could
// read only the one.
#[derive(Default)]
pub struct Dependencies {
    install: Option<Install>,
    index: Option<Stage<DependencyIndex>>,
    placements: Option<Stage<PlacementIndex>>,
    /// Where the bounds of the map `boxes_of` sit against its streaming box, read apart from the index.
    boxes: Option<Stage<ModelContents>>,
    boxes_of: String,
    /// The last answer drawn, by whether it is Placed by and the file's path:
    /// a menu asks again each frame it is open.
    shown: Option<((bool, String), Answer)>,
    /// What the finished reads hold, counted when each lands.
    held: usize,
    /// Held while an index read mounts, kept across every install and every drop.
    turn: Arc<Mutex<()>>,
}

impl Dependencies {
    /// Read the install with `work`, as the one read.
    ///
    /// ⚠ A mount cannot stop, so a read a save has dropped goes on mounting
    /// unheard: this one waits for it, so two mounts never hold the install in
    /// memory together, and one dropped before its turn does not begin.
    pub(super) fn start(
        &mut self,
        work: impl FnOnce(&Cancel, &Progress) -> Result<DependencyIndex, String> + Send + 'static,
    ) {
        let turn = Arc::clone(&self.turn);
        self.index = Some(Stage::Reading(Reading::start(move |cancel, progress| {
            let _turn = turn.lock().unwrap_or_else(PoisonError::into_inner);
            if cancel.is_set() {
                return Err("Stopped".to_string());
            }
            work(cancel, progress)
        })));
    }

    /// Every read dropped, and `install` the one the next is of.
    fn begin_again(&mut self, install: Option<Install>) {
        *self = Self {
            install,
            turn: Arc::clone(&self.turn),
            ..Self::default()
        };
    }

    /// The read running, as the job list draws it.
    pub fn job(&self) -> Option<RunningJob> {
        let install =
            (self.install.as_ref()).map_or_else(String::new, |(root, ..)| file_name(root));
        let (doing, progress) = match (&self.index, &self.placements, &self.boxes) {
            (Some(Stage::Reading(read)), ..) => (
                format!("Reading what {install} declares, for Uses"),
                &read.progress,
            ),
            (_, Some(Stage::Reading(read)), _) => (
                format!("Reading the maps of {install}, for Placed by"),
                &read.progress,
            ),
            (.., Some(Stage::Reading(read))) => (
                format!("Reading the bounds of {}, for Uses", leaf(&self.boxes_of)),
                &read.progress,
            ),
            _ => return None,
        };
        Some(RunningJob {
            kind: JobKind::Dependencies,
            doing,
            fraction: None,
            detail: progress.snapshot().item,
            stopping: false,
        })
    }

    /// A file under the install was replaced, so every read of it, done or running, is dropped and the next menu begins again; `true` when one was running.
    ///
    /// ⚠ The mount keeps each archive's entry offsets as the install had them,
    /// so a read after a save would take the replaced file's bytes at the old
    /// ones. A read dropped runs on unheard, as a cancelled one does, and
    /// nothing it reads is used.
    pub(super) fn replaced(&mut self, path: &Path) -> bool {
        let Some((root, ..)) = &self.install else {
            return false;
        };
        if !path.starts_with(root) {
            return false;
        }
        let running = self.job().is_some();
        self.begin_again(self.install.clone());
        running
    }

    /// What the reads hold: the mount and file catalogue of the index, and the placements.
    ///
    /// ⚠ The archetype, table and model indexes of the index are private to their crates and are
    /// not counted.
    pub fn held_bytes(&self) -> usize {
        self.held
    }

    /// What the finished index's mount holds, apart from its catalogue.
    #[cfg(test)]
    pub(super) fn mount_bytes(&self) -> usize {
        match &self.index {
            Some(Stage::Read(Ok(index))) => index.mount.held_bytes(),
            _ => 0,
        }
    }

    /// Drop the read running, which the next menu starts again; `true` when one ran.
    ///
    /// ⚠ A mount cannot stop halfway, so its worker finishes unheard; a read
    /// of the maps stops at the next map, and one of a map's bounds at the next bound.
    pub fn cancel(&mut self) -> bool {
        let index = self.index.take_if(reading).is_some();
        let maps = self.placements.take_if(reading).is_some();
        let bounds = self.boxes.take_if(reading).is_some();
        index || maps || bounds
    }

    /// The boxes the map `file` needs, read on a worker the first time it is asked.
    fn boxes_for(
        &mut self,
        index: &Arc<DependencyIndex>,
        file: &str,
    ) -> Result<Arc<ModelContents>, Answer> {
        if self.boxes_of != file {
            self.boxes = None;
            file.clone_into(&mut self.boxes_of);
        }
        match &self.boxes {
            Some(Stage::Read(Ok(boxes))) => Ok(Arc::clone(boxes)),
            Some(Stage::Read(Err(why))) => Err(Answer::Cannot(why.clone())),
            Some(Stage::Reading(_)) => Err(Answer::Looking),
            None => {
                let (index, subject) = (Arc::clone(index), leaf(file).to_string());
                self.boxes = Some(Stage::Reading(Reading::start(move |cancel, progress| {
                    progress.now_on(format!("The bounds of {subject}"));
                    Ok(index.map_boxes_of(&subject, &|| cancel.is_set()))
                })));
                Err(Answer::Looking)
            }
        }
    }
}

impl AppState {
    /// What the file at `row` of `doc` uses; the install is read first, as a job, if no menu has asked yet.
    pub fn uses_of(&mut self, doc: DocumentId, row: RpfNodeId) -> Answer {
        self.dependency_answer(doc, row, false)
    }

    /// Which maps place the model at `row` of `doc`; they are read first, as a job, if no menu has asked yet.
    pub fn placed_by(&mut self, doc: DocumentId, row: RpfNodeId) -> Answer {
        self.dependency_answer(doc, row, true)
    }

    fn dependency_answer(&mut self, doc: DocumentId, row: RpfNodeId, placing: bool) -> Answer {
        let Some((root, file)) = self.in_the_install(doc, row) else {
            return Answer::Cannot(NO_INSTALL.to_string());
        };
        let install = (
            root.clone(),
            self.prefs.mount_options(),
            self.keys.has_keys(),
        );
        let deps = &mut self.dependencies;
        if deps.install.as_ref() != Some(&install) {
            deps.begin_again(Some(install.clone()));
        }
        let asked = (placing, file);
        if let Some((shown, answer)) = &deps.shown {
            if *shown == asked {
                return answer.clone();
            }
        }
        let index = match &deps.index {
            Some(Stage::Read(Ok(index))) => Arc::clone(index),
            Some(Stage::Read(Err(why))) => return Answer::Cannot(why.clone()),
            Some(Stage::Reading(_)) => return Answer::Looking,
            None => {
                let crypto = self.keys.provider();
                deps.start(move |cancel, progress| read_install(install, crypto, cancel, progress));
                return Answer::Looking;
            }
        };
        let answer = if placing {
            match &deps.placements {
                Some(Stage::Read(Ok(placements))) => placed_by(&index, placements, &root, &asked.1),
                Some(Stage::Read(Err(why))) => return Answer::Cannot(why.clone()),
                Some(Stage::Reading(_)) => return Answer::Looking,
                None => {
                    let work = move |cancel: &Cancel, progress: &Progress| {
                        Ok(read_placements(&index, cancel, progress))
                    };
                    deps.placements = Some(Stage::Reading(Reading::start(work)));
                    return Answer::Looking;
                }
            }
        } else if Kind::of_leaf(leaf(&asked.1)) == Kind::MapData {
            match deps.boxes_for(&index, &asked.1) {
                Ok(boxes) => uses(&index, &root, &asked.1, &boxes),
                Err(answer) => return answer,
            }
        } else {
            uses(&index, &root, &asked.1, &ModelContents::new())
        };
        deps.shown = Some((asked, answer.clone()));
        answer
    }

    /// The active install and `row`'s path in its tree, when `doc` is that install or inside it.
    fn in_the_install(&self, doc: DocumentId, row: RpfNodeId) -> Option<(PathBuf, String)> {
        let root = self.prefs.active_game_root()?;
        let document = self.document(doc)?;
        let below = document.path.as_deref()?.strip_prefix(root).ok()?;
        let DocPayload::Rpf(rpf) = &document.payload else {
            return None;
        };
        let mut path = below.to_string_lossy().replace('\\', "/");
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(&rpf.session.path_of(row)?);
        Some((root.clone(), path))
    }

    /// `path` was written over, renamed, moved or deleted: what Uses and Placed by read of an install holding it, and the texture pack index of a folder holding it, is read again at the next ask.
    pub(super) fn file_replaced(&mut self, path: &Path) {
        self.texture_packs_changed(path);
        if self.dependencies.replaced(path) {
            let said = format!(
                "{} changed, so the install is read again when Uses or Placed by next asks",
                file_name(path)
            );
            self.jobs
                .record(JobKind::Dependencies, Outcome::Stopped(Then::Nothing), said);
        }
    }

    /// Where a finished read lands: the menus answer from it, or say why it failed.
    pub(super) fn poll_dependencies(&mut self) {
        let deps = &mut self.dependencies;
        let install =
            (deps.install.as_ref()).map_or_else(String::new, |(root, ..)| file_name(root));
        let ended = [
            ended(&mut deps.index).map(|read| (read, format!("Uses answers for {install}"))),
            ended(&mut deps.placements)
                .map(|read| (read, format!("Placed by answers for {install}"))),
            ended(&mut deps.boxes).map(|read| {
                let done = format!("Uses checked the bounds of {}", leaf(&deps.boxes_of));
                (read, done)
            }),
        ];
        if ended.iter().flatten().next().is_some() {
            deps.held = held_by(&deps.index, &deps.placements);
        }
        for (read, done) in ended.into_iter().flatten() {
            let (outcome, said) = match read {
                Ok(()) => (Outcome::Done(Then::Nothing), done),
                Err(why) => (Outcome::Failed, why),
            };
            self.jobs.record(JobKind::Dependencies, outcome, said);
        }
    }
}

/// The mount and file catalogue of a finished index, and the placements of a finished fold.
fn held_by(
    index: &Option<Stage<DependencyIndex>>,
    placements: &Option<Stage<PlacementIndex>>,
) -> usize {
    let indexed = match index {
        Some(Stage::Read(Ok(index))) => {
            index.mount.held_bytes() + catalogue_bytes(&index.catalogue)
        }
        _ => 0,
    };
    let placed = match placements {
        Some(Stage::Read(Ok(placements))) => placements.footprint(),
        _ => 0,
    };
    indexed + placed
}

/// A catalogue's table, one slot for each name, and the two strings each entry holds.
fn catalogue_bytes(catalogue: &Catalogue) -> usize {
    let entries = Kind::ALL.iter().flat_map(|kind| catalogue.of_kind(*kind));
    let text: usize = entries
        .map(|file| file.logical.capacity() + file.stem.capacity())
        .sum();
    hash_table_bytes_for_len::<((u32, Kind), Located)>(catalogue.len()) + text
}

fn reading<T>(stage: &mut Stage<T>) -> bool {
    matches!(stage, Stage::Reading(_))
}

/// Mount `install` and read what a resolver needs from it, as `openrage-cli depends` does.
fn read_install(
    install: Install,
    crypto: Arc<dyn RpfCryptoProvider>,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<DependencyIndex, String> {
    let (root, mount, _) = install;
    // Installs the app's snapshot, which the command's parse reads every table with.
    super::meta_schema();
    progress.now_on("Mounting the install");
    let game = build_mounted_game_with(&GtaInstall { root }, crypto, DlcCutoff::All, mount)
        .map_err(|e| format!("The install could not be mounted: {e}"))?;
    if cancel.is_set() {
        return Err("Stopped".to_string());
    }
    Ok(DependencyIndex::read(game, &mut |line| {
        tracing::info!("{line}");
        progress.now_on(line);
    }))
}

/// Every placement in every map the install loads, until `cancel` is set.
///
/// ponytail: every archetype's placements, tens of megabytes over a retail
/// install, so each later Placed by is a lookup; watch the archetypes asked
/// about if that memory matters.
fn read_placements(
    index: &DependencyIndex,
    cancel: &Cancel,
    progress: &Progress,
) -> PlacementIndex {
    let mut placements = PlacementIndex::new();
    index.place(&mut placements, |read| {
        progress.now_on(format!("{read} maps read"));
        cancel.is_set()
    });
    placements
}

/// Move a finished read's answer into its stage; whether it read, or why not.
fn ended<T>(stage: &mut Option<Stage<T>>) -> Option<Result<(), String>> {
    let Some(Stage::Reading(reading)) = stage else {
        return None;
    };
    let read = match reading.receiver.try_recv() {
        Err(TryRecvError::Empty) => return None,
        Ok(read) => read.map(Arc::new),
        Err(TryRecvError::Disconnected) => Err("The read stopped on an internal error".to_string()),
    };
    let said = read.as_ref().map(|_| ()).map_err(String::clone);
    *stage = Some(Stage::Read(read));
    Some(said)
}

/// What `file` uses as `openrage-cli depends` resolves it, the file itself aside.
///
/// `boxes` is what `DependencyIndex::map_boxes_of` read for a map, which the
/// command reads in the same call.
fn uses(index: &DependencyIndex, root: &Path, file: &str, boxes: &ModelContents) -> Answer {
    let name = leaf(file);
    let Some(set) = index.uses_checked(name, boxes) else {
        return Answer::Cannot(format!(
            "Nothing the install loads declares {name}, so nothing says what it uses"
        ));
    };
    let mut files: Vec<Listed> = Vec::new();
    for edge in &set.edges {
        let at = edge
            .logical()
            .and_then(|logical| index.mount.vfs.locate(logical));
        if at
            .as_deref()
            .is_some_and(|at| at.eq_ignore_ascii_case(file))
        {
            continue;
        }
        let mut why = or_depends::export::why(edge.via).to_string();
        if let Some(says) = or_depends::export::verdict(edge.via) {
            why = format!("{why}; {says}");
        }
        if matches!(edge.found, Found::Entry(_)) {
            why.push_str(", an entry inside it");
        }
        if let Some(listed) =
            (files.iter_mut()).find(|listed| listed.at.is_some() && listed.at == at)
        {
            listed.note = format!("{}; {why}", listed.note);
            continue;
        }
        let (name, note) = match (&at, edge.logical()) {
            (Some(at), _) => (leaf(at).to_string(), why),
            (None, Some(logical)) => (
                leaf(logical).to_string(),
                format!("{why}, which OpenRage cannot find in the install's folders"),
            ),
            (None, None) => (
                unheld(edge.name, edge.kind),
                format!("not in this install: {why}"),
            ),
        };
        files.push(Listed { name, note, at });
    }
    Answer::Listed {
        install: root.to_path_buf(),
        files: files.into(),
    }
}

/// The maps placing the model `file` is, as `openrage-cli depends placements` counts them.
fn placed_by(
    index: &DependencyIndex,
    placements: &PlacementIndex,
    root: &Path,
    file: &str,
) -> Answer {
    let name = leaf(file);
    let Some(archetype) = index.archetype_of(name) else {
        return Answer::Cannot(format!(
            "Nothing the install loads declares {name}, so no map places it"
        ));
    };
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for placement in placements.of(archetype) {
        *counts.entry(placement.map).or_default() += 1;
    }
    let mut maps: Vec<(String, Option<&str>, usize)> = (counts.iter())
        .map(|(&map, &count)| {
            let logical = (index.catalogue.find(map, Kind::MapData)).map(|l| l.logical.as_str());
            let name = logical.map_or_else(|| unheld(map, Kind::MapData), |l| leaf(l).to_string());
            (name, logical, count)
        })
        .collect();
    maps.sort();
    let more = maps.len().saturating_sub(MAPS_LISTED);
    // ⚠ Located after the cut, on the UI thread: each walks the mount's archives.
    maps.truncate(MAPS_LISTED);
    let mut files: Vec<Listed> = (maps.into_iter())
        .map(|(name, logical, count)| Listed {
            name,
            note: match count {
                1 => "1 placement".to_string(),
                n => format!("{n} placements"),
            },
            at: logical.and_then(|logical| index.mount.vfs.locate(logical)),
        })
        .collect();
    if more > 0 {
        files.push(Listed {
            name: format!("and {more} more maps"),
            note: format!("Placed by lists the first {MAPS_LISTED} by name"),
            at: None,
        });
    }
    Answer::Listed {
        install: root.to_path_buf(),
        files: files.into(),
    }
}

/// A name the install holds no file of: its hash, and its kind's extension.
fn unheld(name: u32, kind: Kind) -> String {
    match kind.extension() {
        "" => format!("{name:#010X}"),
        extension => format!("{name:#010X}.{extension}"),
    }
}

fn leaf(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
