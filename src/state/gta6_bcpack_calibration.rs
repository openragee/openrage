//! The BCPack coder's tables fitted from the texture packs of a folder: a worker that is a row of
//! the job list, with its progress and Cancel, and that keeps what it fits for the packs it read.
//!
//! The tables are in no file and no source gives them, so a texture read through them is an
//! estimate, which the inspector, the gallery and an export say.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::gta6::bcpack::calibrate::save_cache;
use or_io::gta6::ttpf::calibrate_packs;
use or_io::gta6::TexturePack;
use or_job::{Cancel, Progress};

use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::AppState;

/// How a calibration ended.
#[derive(Debug, Clone, PartialEq)]
enum Ended {
    /// The packs read, their coded mips drawn through the fit, and the share of those mips'
    /// blocks drawn through a slot or symbol it did not solve, in percent.
    Fitted {
        packs: usize,
        bodies: u64,
        unknown: f64,
    },
    /// No texture pack opens under the folder.
    NoPack,
    /// The tables could not be kept: why.
    NotKept(String),
    Stopped,
}

struct Calibration {
    folder: PathBuf,
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<Ended>>>,
}

// ponytail: one calibration at a time for the whole app, as one game's packs are fitted together.
static RUN: Mutex<Option<Calibration>> = Mutex::new(None);

fn run() -> MutexGuard<'static, Option<Calibration>> {
    RUN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ended(calibration: &Calibration) -> Option<Ended> {
    let ended = calibration.ended.lock();
    ended
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Every `.ttpf` under `dir`, into `out`.
fn packs_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(listed) = std::fs::read_dir(dir) else {
        return;
    };
    for path in listed.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            packs_under(&path, out);
        } else if (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("ttpf")) {
            out.push(path);
        }
    }
}

/// The folder whose packs are calibrated with the pack at `path`: the game folder its
/// `data/ttdb` lies under, a `pack/<name>` folder counting as the game's, else the pack's own.
pub(crate) fn folder_of(path: &Path) -> PathBuf {
    let own = path.parent().unwrap_or(path);
    let root = own
        .ancestors()
        .find(|folder| folder.join("data").join("ttdb").is_dir());
    let Some(root) = root else {
        return own.to_path_buf();
    };
    let packs = (root.parent())
        .filter(|packs| (packs.file_name()).is_some_and(|name| name.eq_ignore_ascii_case("pack")));
    packs.and_then(Path::parent).unwrap_or(root).to_path_buf()
}

fn work(folder: &Path, cancel: &Cancel, progress: &Progress) -> Ended {
    let mut paths = Vec::new();
    packs_under(folder, &mut paths);
    paths.sort();
    let packs: Vec<TexturePack> = (paths.iter())
        .filter_map(|path| TexturePack::open(path).ok())
        .collect();
    if packs.is_empty() {
        return Ended::NoPack;
    }
    let found = calibrate_packs(
        &packs,
        &mut |done, total| {
            progress.reached(done, total);
        },
        &|| cancel.is_set(),
    );
    let Some(found) = found else {
        return Ended::Stopped;
    };
    let kept = super::ttpf_document::calibration_dir()
        .ok_or_else(|| "no configuration folder".to_string())
        .and_then(|dir| {
            save_cache(&dir, &found.tables, &found.members)
                .map_err(|e| format!("{}: {e}", dir.display()))
        });
    if let Err(why) = kept {
        return Ended::NotKept(why);
    }
    let (mut bodies, mut blocks, mut unknown) = (0u64, 0u64, 0u64);
    for summary in &found.summaries {
        bodies += summary.bodies;
        blocks += summary.blocks;
        unknown += summary.unknown;
    }
    Ended::Fitted {
        packs: packs.len(),
        bodies,
        unknown: 100.0 * unknown as f64 / blocks.max(1) as f64,
    }
}

/// The calibration running, as the job list draws it.
pub(super) fn job() -> Option<RunningJob> {
    let held = run();
    let calibration = held.as_ref().filter(|running| ended(running).is_none())?;
    Some(RunningJob {
        kind: JobKind::TextureCalibration,
        doing: tr!(
            "Calibrating GTA VI texture decoding from the packs of {folder}",
            folder = file_name(&calibration.folder)
        ),
        fraction: Some(calibration.progress.snapshot().fraction()),
        detail: String::new(),
        stopping: calibration.cancel.is_set(),
    })
}

/// Ask the calibration to stop; it says it stopped once its worker sees the stop.
pub(super) fn stop() {
    if let Some(calibration) = run().as_ref() {
        calibration.cancel.set();
    }
}

impl AppState {
    /// Fit the BCPack coder's tables from every texture pack under `folder`, on a worker.
    pub fn calibrate_texture_decoding(&mut self, folder: &Path) {
        let mut held = run();
        if (held.as_ref()).is_some_and(|running| ended(running).is_none()) {
            drop(held);
            self.show_info_toast(tr!(
                "A calibration of GTA VI texture decoding is already running"
            ));
            return;
        }
        let calibration = Calibration {
            folder: folder.to_path_buf(),
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (folder, cancel, progress, said) = (
            calibration.folder.clone(),
            calibration.cancel.clone(),
            calibration.progress.clone(),
            Arc::clone(&calibration.ended),
        );
        std::thread::spawn(move || {
            let outcome = work(&folder, &cancel, &progress);
            *said.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        *held = Some(calibration);
    }

    /// Say in the job list how a calibration that ended went, and read the open packs through
    /// what it kept.
    pub(super) fn poll_texture_calibration(&mut self) {
        let how = {
            let mut held = run();
            let Some(how) = held.as_ref().and_then(ended) else {
                return;
            };
            *held = None;
            how
        };
        let (outcome, said) = match how {
            Ended::Fitted {
                packs,
                bodies,
                unknown,
            } => {
                super::ttpf_document::forget_decodes();
                for doc in &mut self.documents {
                    super::ttpf_document::recalibrate(&mut doc.payload);
                }
                let said = tr_n!(
                    "{count} GTA VI texture pack calibrated: {bodies} coded mips now read as estimates, {unknown} % of their blocks through a slot not known",
                    "{count} GTA VI texture packs calibrated: {bodies} coded mips now read as estimates, {unknown} % of their blocks through a slot not known",
                    packs,
                    bodies = bodies,
                    unknown = format!("{unknown:.1}")
                );
                (Outcome::Done(Then::Nothing), said)
            }
            Ended::NoPack => (
                Outcome::Failed,
                tr!("No GTA VI texture pack opens under that folder").to_string(),
            ),
            Ended::NotKept(why) => (
                Outcome::Failed,
                tr!("The calibration could not be kept: {why}", why = why),
            ),
            Ended::Stopped => (
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished").to_string(),
            ),
        };
        self.jobs.record(JobKind::TextureCalibration, outcome, said);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// Waits out the calibration, then polls the jobs once, which says how it ended.
    fn worked(state: &mut AppState) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while job().is_some() {
            assert!(Instant::now() < deadline, "the calibration never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        state.poll_jobs();
    }

    #[test]
    fn a_calibration_of_texture_decoding_is_a_job_that_stops_when_asked() {
        // A folder with no pack keeps nothing, stopped or not.
        let dir = tempfile::tempdir().expect("a scratch folder");
        let (cancel, progress) = (Cancel::new(), Progress::new());
        assert_eq!(work(dir.path(), &cancel, &progress), Ended::NoPack);
        cancel.set();
        assert_eq!(work(dir.path(), &cancel, &progress), Ended::NoPack);

        // ⚠ The calibration is one for the whole app, so a test that polls another state's jobs
        // at the same time may be the one that logs its end.
        let mut state = AppState::new();
        state.calibrate_texture_decoding(dir.path());
        state.cancel_job(JobKind::TextureCalibration);
        worked(&mut state);
        let logged =
            (state.jobs.ended().iter()).find(|job| job.kind == JobKind::TextureCalibration);
        if let Some(logged) = logged {
            assert_eq!(logged.outcome, Outcome::Failed, "{}", logged.said);
        }
        assert!(job().is_none());
    }

    #[test]
    fn the_packs_of_a_game_are_calibrated_together_from_the_folder_above_data_ttdb() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let game = dir.path().join("game");
        let base = game.join("data").join("ttdb").join("maps");
        let added = game.join("pack").join("zq").join("data").join("ttdb");
        std::fs::create_dir_all(&base).expect("made");
        std::fs::create_dir_all(&added).expect("made");
        assert_eq!(folder_of(&base.join("a.ttpf")), game);
        assert_eq!(folder_of(&added.join("b.ttpf")), game);
        let loose = dir.path().join("loose");
        assert_eq!(folder_of(&loose.join("c.ttpf")), loose);
    }
}
