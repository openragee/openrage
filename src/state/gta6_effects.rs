//! An `SGD5` effect container a scan found, read on a worker as a row of the job list with its
//! progress and Cancel; its effects then name GTA VI drawables' shaders and their parameters.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

use or_io::container::rpf::{read_rpf9_body, Rpf9ScanFind};
use or_job::{Cancel, Progress};

use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::{AppState, DocPayload};

/// How an effects read ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectsEnded {
    /// How many effects the container holds.
    Read(usize),
    Stopped,
    Failed(String),
}

pub struct EffectsRead {
    doing: String,
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<EffectsEnded>>>,
}

impl EffectsRead {
    /// Read the container `find` of `archive`, at most `limit` bytes, on a worker.
    pub(super) fn start(archive: &Path, find: Rpf9ScanFind, limit: u64) -> Self {
        let doing = tr!(
            "Reading the effects at {offset} of {archive}",
            offset = format!("{:#X}", find.offset),
            archive = file_name(archive),
        );
        let archive = archive.to_path_buf();
        Self::run(doing, move |cancel, progress| {
            effects(&archive, find, limit, cancel, progress)
        })
    }

    /// Read every `SGD5` container a scan of `archive` finds, at most `limit` bytes each, on a worker.
    pub(super) fn every_container(archive: &Path, limit: u64) -> Self {
        let doing = tr!(
            "Reading the effects of {archive}",
            archive = file_name(archive)
        );
        let archive = archive.to_path_buf();
        Self::run(doing, move |cancel, progress| {
            every_container(&archive, limit, cancel, progress)
        })
    }

    fn run(
        doing: String,
        work: impl FnOnce(&Cancel, &Progress) -> Result<usize, String> + Send + 'static,
    ) -> Self {
        let read = Self {
            doing,
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            read.cancel.clone(),
            read.progress.clone(),
            Arc::clone(&read.ended),
        );
        std::thread::spawn(move || {
            let outcome = match work(&cancel, &progress) {
                Ok(count) => EffectsEnded::Read(count),
                Err(_) if cancel.is_set() => EffectsEnded::Stopped,
                Err(why) => EffectsEnded::Failed(why),
            };
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        read
    }

    /// The read while it runs, as the job list draws it.
    pub(super) fn job(&self) -> Option<RunningJob> {
        if self.ended().is_some() {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::EffectsRead,
            doing: self.doing.clone(),
            fraction: Some(self.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: self.cancel.is_set(),
        })
    }

    pub(super) fn stop(&self) {
        self.cancel.set();
    }

    pub(super) fn ended(&self) -> Option<EffectsEnded> {
        self.ended.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for EffectsRead {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// The container's bytes read through a file that reports how far it has got and stops when asked.
fn effects(
    archive: &Path,
    find: Rpf9ScanFind,
    limit: u64,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<usize, String> {
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    progress.reached(0, find.span);
    let watched = Watched {
        file,
        cancel,
        progress,
        from: find.offset,
        span: find.span,
    };
    let (_, bytes) = read_rpf9_body(watched, find.offset, limit).map_err(|e| e.to_string())?;
    if cancel.is_set() {
        return Err(tr!("Stopped before it finished").to_string());
    }
    let count = super::gta6_resource::learn_effects(&bytes)?;
    super::gta6_shaders::add_source(archive, find.offset, &bytes);
    Ok(count)
}

/// The effects of each `SGD5` container a scan of `archive` finds, the scan's place its progress.
fn every_container(
    archive: &Path,
    limit: u64,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<usize, String> {
    use or_io::container::rpf::{Rpf9BodyKind, Rpf9Scan};
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut scan = Rpf9Scan::new(&file, 0..u64::MAX).map_err(|e| e.to_string())?;
    let (len, mut learned) = (scan.file_len(), 0);
    while let Some(find) = scan.next() {
        if cancel.is_set() {
            return Err(tr!("Stopped before it finished").to_string());
        }
        progress.reached(scan.position().min(len), len);
        let find = find.map_err(|e| e.to_string())?;
        if matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
            let (_, bytes) =
                read_rpf9_body(&file, find.offset, limit).map_err(|e| e.to_string())?;
            learned += super::gta6_resource::learn_effects(&bytes)?;
            super::gta6_shaders::add_source(archive, find.offset, &bytes);
        }
    }
    Ok(learned)
}

/// The read of the effects of the GTA VI install folder's `shaders.rpf`, under the folder it was
/// started for; `None` beside a folder where none was started.
// ponytail: one read for the whole app, started once per install folder.
static INSTALL_READ: Mutex<Option<(std::path::PathBuf, Option<EffectsRead>)>> = Mutex::new(None);

fn install_read(
) -> std::sync::MutexGuard<'static, Option<(std::path::PathBuf, Option<EffectsRead>)>> {
    INSTALL_READ.lock().unwrap_or_else(|p| p.into_inner())
}

/// The install folder's effects read while it runs, as the job list draws it.
pub(super) fn install_job() -> Option<RunningJob> {
    install_read().as_ref()?.1.as_ref()?.job()
}

/// Ask the install folder's effects read to stop.
pub(super) fn stop_install_read() {
    if let Some((_, Some(read))) = install_read().as_ref() {
        read.stop();
    }
}

/// A file whose reads past `from` count toward `span` bytes, and fail once the read is stopped.
struct Watched<'a> {
    file: File,
    cancel: &'a Cancel,
    progress: &'a Progress,
    from: u64,
    span: u64,
}

impl Read for Watched<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // ⚠ Not `Interrupted`, which a reader retries forever.
        if self.cancel.is_set() {
            return Err(std::io::Error::other("stopped"));
        }
        let read = self.file.read(buf)?;
        let at = self.file.stream_position()?;
        let done = at.saturating_sub(self.from).min(self.span);
        self.progress.reached(done, self.span);
        Ok(read)
    }
}

impl Seek for Watched<'_> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(to)
    }
}

impl AppState {
    /// Start reading the effects of the GTA VI install folder's `shaders.rpf` once for each folder
    /// set, unless the app already knows effects.
    fn start_install_effects(&self) {
        if self
            .active_document()
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return;
        }
        let Some(root) = self.prefs.game_paths.get(or_title::Game::GtaVI) else {
            return;
        };
        let mut read = install_read();
        if read.as_ref().is_some_and(|(tried, _)| tried == root) {
            return;
        }
        let archive = root.join("shaders.rpf");
        let start = !super::gta6_resource::effects_known() && archive.is_file();
        let started = start.then(|| EffectsRead::every_container(&archive, 1 << 30));
        *read = Some((root.clone(), started));
    }

    /// Say in the job list how each effects read that ended went.
    pub(super) fn poll_effects_reads(&mut self) {
        self.start_install_effects();
        let mut ended: Vec<EffectsEnded> = (self.documents.iter())
            .filter_map(|doc| match &doc.payload {
                DocPayload::Rpf9(scan) => scan.effects_ended(),
                _ => None,
            })
            .collect();
        if let Some((_, read)) = install_read().as_mut() {
            if let Some(end) = read.as_ref().and_then(EffectsRead::ended) {
                ended.push(end);
                *read = None;
            }
        }
        for ended in ended {
            let (outcome, said) = match ended {
                EffectsEnded::Read(count) => (
                    Outcome::Done(Then::Nothing),
                    tr_n!(
                        "{count} effect read: GTA VI drawables name their shaders by it",
                        "{count} effects read: GTA VI drawables name their shaders by them",
                        count
                    ),
                ),
                EffectsEnded::Stopped => (
                    Outcome::Stopped(Then::Nothing),
                    tr!("Stopped before it finished").to_string(),
                ),
                EffectsEnded::Failed(why) => (
                    Outcome::Failed,
                    tr!("The effects were not read: {why}", why = why),
                ),
            };
            self.jobs.record(JobKind::EffectsRead, outcome, said);
        }
        super::gta6_shaders::poll(&mut self.jobs);
    }
}

#[cfg(test)]
mod table_tests {
    use super::*;

    #[test]
    fn a_table_document_does_not_start_the_install_effects_read() {
        let dir = tempfile::tempdir().expect("scratch directory");
        crate::state::prefs::keep_prefs_in(dir.path());
        let mut prefs = crate::state::Prefs::default();
        prefs
            .game_paths
            .set(or_title::Game::GtaVI, Some(dir.path().to_path_buf()));
        let mut state = AppState::from_prefs(prefs);
        let path = dir.path().join("data.rpf/.rpf9-table/0/hash_model.ydr");
        let drawable = crate::state::opening_a_ttpf::drawable_keyed(1);
        let payload = DocPayload::Gta6Drawable(Arc::new(
            or_io::gta6::headerless::Resource::Drawable(drawable),
        ));
        state
            .documents
            .push(crate::state::Document::new(1, Some(path), payload));
        state.active_document = Some(1);
        state.start_install_effects();
        assert!(install_read()
            .as_ref()
            .is_none_or(|(root, _)| root != dir.path()));
    }
}
