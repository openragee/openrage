//! The animation sets a GTA VI clip dictionary names and another body stores, looked for among the
//! animation bodies the scan of its archive found, on a worker as a row of the job list, a body at a
//! time with its progress and Cancel.

use crate::state::Rpf9Document;

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::anim::{AnimResource, RootKind};
use or_job::{Cancel, Progress};

use super::jobs::{JobKind, Outcome, RunningJob, Then};
use super::rpf9_scan::{scanned_body, view_of, Row, View};
use super::{AppState, DocPayload, DocumentId};

/// The bytes a body is read to: more than any lone set was measured to hold.
const BODY_LIMIT: u64 = 256 << 20;

/// The sets found so far, by the archive searched and their root key, for as long as the app runs.
type Found = BTreeMap<PathBuf, BTreeMap<u32, Arc<AnimResource>>>;

fn found() -> MutexGuard<'static, Found> {
    static FOUND: Mutex<Found> = Mutex::new(BTreeMap::new());
    FOUND.lock().unwrap_or_else(|p| p.into_inner())
}

/// The lone animation set of root key `key` a search of `archive` found, if one did.
pub(crate) fn found_set(archive: &Path, key: u32) -> Option<Arc<AnimResource>> {
    found().get(archive)?.get(&key).cloned()
}

/// How a search ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FindEnded {
    /// How many sets it found, and how many it looked for.
    Found(usize, usize),
    Stopped,
    Failed(String),
}

pub(super) struct SetFinding {
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<FindEnded>>>,
}

impl SetFinding {
    /// Read the bodies at `offsets` of `archive` until each set of `wanted` is found, on a worker.
    fn start(archive: PathBuf, offsets: Vec<u64>, mut wanted: HashSet<u32>) -> Self {
        let finding = Self {
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            finding.cancel.clone(),
            finding.progress.clone(),
            Arc::clone(&finding.ended),
        );
        std::thread::spawn(move || {
            let looked_for = wanted.len();
            let outcome = match File::open(&archive) {
                Err(e) => FindEnded::Failed(format!("{}: {e}", archive.display())),
                Ok(file) => {
                    let mut outcome = None;
                    for (done, offset) in offsets.iter().enumerate() {
                        if cancel.is_set() {
                            outcome = Some(FindEnded::Stopped);
                            break;
                        }
                        if wanted.is_empty() {
                            break;
                        }
                        progress.reached(done as u64, offsets.len() as u64);
                        if let Some(set) = set_at(&file, *offset, &wanted) {
                            wanted.remove(&set.key);
                            (found().entry(archive.clone()).or_default())
                                .insert(set.key, Arc::new(set));
                        }
                    }
                    outcome.unwrap_or(FindEnded::Found(looked_for - wanted.len(), looked_for))
                }
            };
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        finding
    }

    fn job(&self) -> Option<RunningJob> {
        if self.ended().is_some() {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::SetSearch,
            doing: tr!("Looking for the GTA VI animation sets other bodies store").to_string(),
            fraction: Some(self.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: self.cancel.is_set(),
        })
    }

    fn ended(&self) -> Option<FindEnded> {
        self.ended.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for SetFinding {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// The lone animation set of one of `wanted`'s keys the body at `offset` is, if it is one.
fn set_at(file: &File, offset: u64, wanted: &HashSet<u32>) -> Option<AnimResource> {
    let (find, bytes) = read_rpf9_body(file, offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let section = resource.sections(&bytes).0;
    // A set's root holds 0 at 0x6C and its key at 0x68, read before the whole body is.
    let key = u32::from_le_bytes(section.get(0x68..0x6C)?.try_into().ok()?);
    if section.get(0x6C..0x70)? != [0; 4] || !wanted.contains(&key) {
        return None;
    }
    let read = or_io::gta6::anim::read(section).ok()?;
    (read.root == RootKind::AnimationSet && read.key == key).then_some(read)
}

impl AppState {
    /// Look for the sets the GTA VI animation `doc` names and no open body or earlier search holds,
    /// among the animation bodies the open scan of its archive found.
    pub(crate) fn find_gta6_sets(&mut self, doc: DocumentId) -> Result<(), String> {
        if self
            .document(doc)
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return Ok(());
        }
        let Some(d) = self.document(doc) else {
            return Ok(());
        };
        let DocPayload::Gta6Anim(resource) = &d.payload else {
            return Ok(());
        };
        if resource.external_sets.is_empty() {
            return Ok(());
        }
        let archive = (d.path.as_deref())
            .and_then(scanned_body)
            .map(|(archive, _)| archive.to_path_buf())
            .ok_or_else(|| tr!("This animation is no body of a scanned archive, so no scan holds the sets it names.").to_string())?;
        let wanted: HashSet<u32> = (resource.external_sets.iter().copied())
            .filter(|key| found_set(&archive, *key).is_none())
            .collect();
        if wanted.is_empty() {
            return Ok(());
        }
        let offsets: Vec<u64> = (self.documents.iter())
            .filter_map(|d| match &d.payload {
                DocPayload::Rpf9(Rpf9Document::Scan(scan)) if scan.path == archive => Some(scan),
                _ => None,
            })
            .flat_map(|scan| {
                (scan.found().rows.iter())
                    .filter_map(|row| match row {
                        Row::Body(find) if matches!(view_of(find), Ok(View::Animation)) => {
                            Some(find.offset)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        if offsets.is_empty() {
            return Err(tr!(
                "Open the scan of {archive}: the sets are looked for among the animation bodies it finds.",
                archive = archive.display()
            ));
        }
        self.set_finding
            .push(SetFinding::start(archive, offsets, wanted));
        Ok(())
    }

    /// The searches still running, as the job list draws them.
    pub(super) fn set_finding_jobs(&self) -> impl Iterator<Item = RunningJob> + '_ {
        (self.set_finding.iter().filter_map(SetFinding::job))
            .chain(self.clip_finding_job())
            .chain(self.particle_model_finding_job())
    }

    /// Ask every search to stop; each says it stopped once its worker sees the stop.
    pub(super) fn stop_set_finding(&self) {
        self.set_finding.iter().for_each(|f| f.cancel.set());
        self.stop_clip_finding();
        self.stop_particle_model_finding();
    }

    /// Say in the job list how each search that ended went, and drop it.
    pub(super) fn poll_set_finding(&mut self) {
        self.poll_clip_finding();
        self.poll_particle_model_finding();
        let mut ended = Vec::new();
        self.set_finding.retain(|f| match f.ended() {
            Some(e) => {
                ended.push(e);
                false
            }
            None => true,
        });
        for e in ended {
            let (outcome, said) = match e {
                FindEnded::Found(found, of) => (
                    Outcome::Done(Then::Nothing),
                    tr_n!(
                        "{count} of {of} GTA VI animation set found in the scan",
                        "{count} of {of} GTA VI animation sets found in the scan",
                        found,
                        of = of
                    ),
                ),
                FindEnded::Stopped => (
                    Outcome::Stopped(Then::Nothing),
                    tr!("Stopped before it finished").to_string(),
                ),
                FindEnded::Failed(why) => (
                    Outcome::Failed,
                    tr!("The sets were not looked for: {why}", why = why),
                ),
            };
            self.jobs.record(JobKind::SetSearch, outcome, said);
        }
    }
}

#[cfg(test)]
#[path = "gta6_set_finding_tests.rs"]
mod tests;
