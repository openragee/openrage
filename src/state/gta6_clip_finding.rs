//! The clips a GTA VI motion dictionary's motions name by 64-bit clip key, looked for among the
//! clip dictionaries every open scan found, on a worker as a row of the job list, a body at a time
//! with its progress and Cancel.

use crate::state::Rpf9Document;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::anim::AnimResource;
use or_job::{Cancel, Progress};

use super::gta6_set_finding::FindEnded;
use super::jobs::{JobKind, Outcome, RunningJob, Then};
use super::rpf9_scan::{body_path, view_of, Row, View};
use super::{AppState, DocPayload, DocumentId};

/// The bytes a body is read to, as the set search reads them.
const BODY_LIMIT: u64 = 256 << 20;

/// A clip a scanned clip dictionary holds: the body it is in, its name and its dictionary's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipAt {
    pub archive: PathBuf,
    pub offset: u64,
    pub clip: String,
    pub dictionary: String,
}

/// The clips found so far, by clip key, for as long as the app runs.
// ponytail: one map for the whole app, holding only the keys a search looked for.
fn found() -> MutexGuard<'static, BTreeMap<u64, ClipAt>> {
    static FOUND: Mutex<BTreeMap<u64, ClipAt>> = Mutex::new(BTreeMap::new());
    FOUND.lock().unwrap_or_else(|p| p.into_inner())
}

/// The clip of key `key` a search found, if one did.
pub(crate) fn found_clip(key: u64) -> Option<ClipAt> {
    found().get(&key).cloned()
}

/// A search; a new one replaces the one running, and dropping it stops it.
pub(super) struct ClipFinding {
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<FindEnded>>>,
}

impl ClipFinding {
    /// Read each archive's bodies of `bodies` until every clip of `wanted` is found, on a worker.
    fn start(bodies: Vec<(PathBuf, Vec<u64>)>, mut wanted: HashSet<u64>) -> Self {
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
            let total: usize = bodies.iter().map(|(_, offsets)| offsets.len()).sum();
            let mut done = 0usize;
            let mut outcome = None;
            'archives: for (archive, offsets) in &bodies {
                let file = match File::open(archive) {
                    Ok(file) => file,
                    Err(e) => {
                        outcome = Some(FindEnded::Failed(format!("{}: {e}", archive.display())));
                        break;
                    }
                };
                for offset in offsets {
                    if cancel.is_set() {
                        outcome = Some(FindEnded::Stopped);
                        break 'archives;
                    }
                    if wanted.is_empty() {
                        break 'archives;
                    }
                    progress.reached(done as u64, total as u64);
                    done += 1;
                    for (key, clip) in clips_at(&file, archive, *offset, &wanted) {
                        wanted.remove(&key);
                        found().insert(key, clip);
                    }
                }
            }
            let got = looked_for - wanted.len();
            let outcome = outcome.unwrap_or(FindEnded::Found(got, looked_for));
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        finding
    }

    fn ended(&self) -> Option<FindEnded> {
        self.ended.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for ClipFinding {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// The clips of `wanted`'s keys the clip dictionary at `offset` of `archive` holds, if it is one.
fn clips_at(file: &File, archive: &Path, offset: u64, wanted: &HashSet<u64>) -> Vec<(u64, ClipAt)> {
    let Some(dictionary) = dictionary_at(file, offset) else {
        return Vec::new();
    };
    (dictionary.clips.iter())
        .filter(|clip| wanted.contains(&clip.key))
        .map(|clip| {
            let at = ClipAt {
                archive: archive.to_path_buf(),
                offset,
                clip: clip.name.clone(),
                dictionary: dictionary.name.clone(),
            };
            (clip.key, at)
        })
        .collect()
}

/// The clip dictionary the body at `offset` is, if it is one.
fn dictionary_at(file: &File, offset: u64) -> Option<AnimResource> {
    let (find, bytes) = read_rpf9_body(file, offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let section = resource.sections(&bytes).0;
    // A clip dictionary's root holds 2 or 3 at 0x6C, read before the whole body is.
    let root_word = u32::from_le_bytes(section.get(0x6C..0x70)?.try_into().ok()?);
    if !matches!(root_word, 2 | 3) {
        return None;
    }
    or_io::gta6::anim::read(section).ok()
}

impl AppState {
    /// The job list's row for the search, while it runs.
    pub(super) fn clip_finding_job(&self) -> Option<RunningJob> {
        let finding = self.clip_finding.as_ref()?;
        if finding.ended().is_some() {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::SetSearch,
            doing: tr!("Looking for the GTA VI clips a motion dictionary names").to_string(),
            fraction: Some(finding.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: finding.cancel.is_set(),
        })
    }

    /// Ask the search to stop; it says it stopped once its worker sees the stop.
    pub(super) fn stop_clip_finding(&self) {
        if let Some(finding) = &self.clip_finding {
            finding.cancel.set();
        }
    }

    /// Look for the clips the motions of the GTA VI motion dictionary `doc` name that no open clip
    /// dictionary or earlier search holds, among the animation bodies every open scan found.
    pub(crate) fn find_gta6_clips(&mut self, doc: DocumentId) -> Result<(), String> {
        let Some(DocPayload::Gta6Anim(resource)) = self.document(doc).map(|d| &d.payload) else {
            return Ok(());
        };
        let open: HashSet<u64> = (self.documents.iter())
            .filter_map(|d| match &d.payload {
                DocPayload::Gta6Anim(r) => Some(r.clips.iter().map(|c| c.key)),
                _ => None,
            })
            .flatten()
            .collect();
        let wanted: HashSet<u64> = {
            let found = found();
            (resource.motions.iter())
                .flat_map(|m| m.clips.iter().copied())
                .filter(|key| !open.contains(key) && !found.contains_key(key))
                .collect()
        };
        if wanted.is_empty() {
            return Ok(());
        }
        let mut bodies: BTreeMap<PathBuf, BTreeSet<u64>> = BTreeMap::new();
        for d in &self.documents {
            let DocPayload::Rpf9(Rpf9Document::Scan(scan)) = &d.payload else {
                continue;
            };
            let offsets = (scan.found().rows.iter())
                .filter_map(|row| match row {
                    Row::Body(find)
                        if matches!(find.kind, Rpf9BodyKind::Zlib(_))
                            && matches!(view_of(find), Ok(View::Animation)) =>
                    {
                        Some(find.offset)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            bodies.entry(scan.path.clone()).or_default().extend(offsets);
        }
        let bodies: Vec<(PathBuf, Vec<u64>)> = (bodies.into_iter())
            .filter(|(_, offsets)| !offsets.is_empty())
            .map(|(archive, offsets)| (archive, offsets.into_iter().collect()))
            .collect();
        if bodies.is_empty() {
            return Err(tr!("Open the scan of an archive to look for clips in.").to_string());
        }
        self.clip_finding = Some(ClipFinding::start(bodies, wanted));
        Ok(())
    }

    /// Open the clip dictionary a search found `clip` in.
    pub(crate) fn open_found_clip(&mut self, clip: &ClipAt) {
        self.load_file(body_path(&clip.archive, clip.offset, View::Animation));
    }

    /// Say in the job list how the search ended, once it has, and drop it.
    pub(super) fn poll_clip_finding(&mut self) {
        let Some(ended) = self.clip_finding.as_ref().and_then(ClipFinding::ended) else {
            return;
        };
        self.clip_finding = None;
        let (outcome, said) = match ended {
            FindEnded::Found(found, of) => (
                Outcome::Done(Then::Nothing),
                tr_n!(
                    "{count} of {of} GTA VI clip found in the open scans",
                    "{count} of {of} GTA VI clips found in the open scans",
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
                tr!("The clips were not looked for: {why}", why = why),
            ),
        };
        self.jobs.record(JobKind::SetSearch, outcome, said);
    }
}

#[cfg(test)]
#[path = "gta6_clip_finding_tests.rs"]
mod tests;
