//! A GTA VI tree's save planned and then written as a job the list shows: the bytes planned or
//! written and the row or file in hand, and a stop that ends it soon after, leaving no file.

use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

use or_io::container::rpf::gta6_patch::plan_patch_with;
use or_io::container::rpf::{BodyPatch, GtaViKeys, PatchRefusal, Rpf9ArchiveWriter};
use or_job::{Cancel, Progress};

use super::{lock, stored_body, Planned, Replaced};
use crate::state::jobs::{file_name, JobKind, RunningJob};

/// What a plan tells its job: the row in hand, and the bytes planned of all; `false` stops it.
pub(crate) type Tick<'a> = &'a mut dyn FnMut(&str, u64, u64) -> bool;

/// The largest changed chunk the optimal parse is run over by a save with no progress and no
/// stop, a row saved on its own through [`plan_save`]; the tree's save plans at `OPTIMAL_CAP`.
const SAVE_OPTIMAL_CAP: usize = 256 << 10;

/// Why a tree's save gave no plan.
#[derive(Debug)]
pub(crate) enum Unplanned {
    /// Its job was stopped, and nothing was written.
    Stopped,
    /// The plan was refused, saying why.
    Refused(String),
}

impl From<String> for Unplanned {
    fn from(why: String) -> Self {
        Self::Refused(why)
    }
}

/// A save being planned, or written to the copy `writing` names: its archive's name, its stop, and
/// how far it has got.
struct Planning {
    file: String,
    writing: Option<String>,
    cancel: Cancel,
    progress: Progress,
}

// ponytail: a list for the process, as the save's own lists are; each plan leaves it as it ends.
static PLANNING: Mutex<Vec<Arc<Planning>>> = Mutex::new(Vec::new());

/// Takes its plan out of [`PLANNING`] however the plan ends, a panic included.
struct Listed(Arc<Planning>);

impl Drop for Listed {
    fn drop(&mut self) {
        lock(&PLANNING).retain(|job| !Arc::ptr_eq(job, &self.0));
    }
}

/// Run `plan` of a save of the archive at `source` as a job the list shows, telling it the row in
/// hand and the bytes planned; a stop ends it as [`Unplanned::Stopped`], whatever it returned.
pub(super) fn tracked<T>(
    source: &Path,
    plan: impl FnOnce(Tick<'_>) -> Result<T, String>,
) -> Result<T, Unplanned> {
    run_listed(source, None, |job| {
        plan(&mut |row: &str, done: u64, total: u64| {
            job.progress.now_on(row);
            job.progress.reached(done, total);
            !job.cancel.is_set()
        })
    })
}

/// Run `write` of a planned save into the new file `out` as the same job, telling it the bytes
/// written of all; a stop ends it as [`Unplanned::Stopped`] and leaves no file at `out`.
pub(super) fn tracked_write<T>(
    source: &Path,
    out: &Path,
    write: impl FnOnce(&mut dyn FnMut(u64, u64) -> bool) -> Result<T, String>,
) -> Result<T, Unplanned> {
    run_listed(source, Some(out), |job| {
        let written = write(&mut |done: u64, total: u64| {
            job.progress.reached(done, total);
            !job.cancel.is_set()
        });
        // A stop the write saw too late to drop removes the file it finished; one it refused left none.
        if written.is_ok() && job.cancel.is_set() {
            let _ = std::fs::remove_file(out);
        }
        written
    })
}

/// Run `run` as a job the list shows while it runs; a stop ends it as [`Unplanned::Stopped`].
fn run_listed<T>(
    source: &Path,
    writing: Option<&Path>,
    run: impl FnOnce(&Planning) -> Result<T, String>,
) -> Result<T, Unplanned> {
    let job = Arc::new(Planning {
        file: file_name(source),
        writing: writing.map(file_name),
        cancel: Cancel::new(),
        progress: Progress::new(),
    });
    lock(&PLANNING).push(Arc::clone(&job));
    let listed = Listed(job);
    let ran = run(&listed.0);
    if listed.0.cancel.is_set() {
        return Err(Unplanned::Stopped);
    }
    ran.map_err(Unplanned::Refused)
}

/// Each save being planned or written, as the job list draws it.
pub(in crate::state) fn jobs() -> Vec<RunningJob> {
    (lock(&PLANNING).iter())
        .map(|job| {
            let at = job.progress.snapshot();
            let (done, total) = (
                or_job::human_bytes(at.attempted),
                or_job::human_bytes(at.total),
            );
            let (doing, detail) = match &job.writing {
                // A rebuilt archive's write tells no progress.
                Some(copy) => (
                    tr!("Writing the save of {file}", file = job.file),
                    match at.total {
                        0 => String::new(),
                        _ => tr!(
                            "writing {file}, {done} of {total}",
                            file = copy,
                            done = done,
                            total = total
                        ),
                    },
                ),
                None => (
                    tr!("Planning the save of {file}", file = job.file),
                    match at.item.is_empty() {
                        true => String::new(),
                        false => tr!(
                            "planning {row}, {done} of {total}",
                            row = at.item,
                            done = done,
                            total = total
                        ),
                    },
                ),
            };
            RunningJob {
                kind: JobKind::Save,
                doing,
                fraction: (at.total > 0).then(|| at.fraction()),
                detail,
                stopping: job.cancel.is_set(),
            }
        })
        .collect()
}

/// Stop every save being planned or written: each ends within a part of the optimal parse or of the copy, and leaves no file.
pub(in crate::state) fn stop() {
    lock(&PLANNING).iter().for_each(|job| job.cancel.set());
}

/// How far `got` bytes are from the `want` a row or a chunk states.
fn off_by(got: u64, want: u64) -> String {
    if got > want {
        tr_n!("{count} byte over", "{count} bytes over", got - want)
    } else {
        tr_n!("{count} byte short", "{count} bytes short", want - got)
    }
}

/// Plan `replaced` into the archive at `source`, reading only: a patched copy when each body fits
/// its entry's chunks and `kept` names no other edit, else a rebuild by `writer`, with each row that
/// keeps it from being patched named with the refusal and by how many bytes.
pub(crate) fn plan_save(
    source: &Path,
    keys: &GtaViKeys,
    replaced: &[Replaced],
    kept: Vec<String>,
    writer: Rpf9ArchiveWriter,
) -> Result<Planned, String> {
    let cap = SAVE_OPTIMAL_CAP;
    plan_save_with(source, keys, replaced, kept, writer, cap, &mut |_, _, _| {
        true
    })
}

/// [`plan_save`] with the optimal parse run over changed chunks of at most `cap` bytes, telling
/// `tick` the row in hand and the bytes planned of every changed body's; `false` stops the plan.
pub(super) fn plan_save_with(
    source: &Path,
    keys: &GtaViKeys,
    replaced: &[Replaced],
    mut kept: Vec<String>,
    writer: Rpf9ArchiveWriter,
    cap: usize,
    tick: Tick<'_>,
) -> Result<Planned, String> {
    let mut file = File::open(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut todo: Vec<(&str, BodyPatch)> = Vec::new();
    for row in replaced {
        let Some(&entry) = row.at.chain.last() else {
            kept.push(tr!("{name} has no body of its own", name = row.name));
            continue;
        };
        // A headerless row states no real size, so its length is left to the plan's chunks.
        let body = if row.at.headerless {
            let Some(body) = stored_body(&row.body) else {
                let why = tr!(
                    "{name} does not read as a resource from its start",
                    name = row.name
                );
                kept.push(why);
                continue;
            };
            body
        } else {
            row.body.clone()
        };
        let size = body.len() as u64;
        if !row.at.headerless && row.at.real != Some(size) {
            let real = (row.at.real).map_or_else(|| "?".to_owned(), |real| real.to_string());
            let why = tr!(
                "{name} is {size} bytes, not the {real} its row states",
                name = row.name,
                size = size,
                real = real
            );
            let by =
                (row.at.real).map_or_else(String::new, |real| format!(", {}", off_by(size, real)));
            kept.push(format!("{why}{by}"));
            continue;
        }
        let patch = BodyPatch {
            base: row.at.base,
            entry,
            body,
        };
        todo.push((row.name.as_str(), patch));
    }
    let total: u64 = (todo.iter())
        .map(|(_, patch)| patch.body.len() as u64)
        .sum();
    let (mut patches, mut before) = (Vec::new(), 0u64);
    for &(name, ref patch) in &todo {
        let told = &mut |done: u64, _: u64| tick(name, before + done, total);
        let planned = plan_patch_with(&mut file, keys, patch, cap, told);
        before += patch.body.len() as u64;
        match planned {
            Ok(plan) => patches.push(plan),
            Err(why @ PatchRefusal::Stopped { .. }) => return Err(why.to_string()),
            Err(why) => {
                let by = match &why {
                    PatchRefusal::ChunkDoesNotFit {
                        deflated,
                        optimal,
                        ceiling,
                        ..
                    } => {
                        let best = (*deflated).min(*optimal) as u64;
                        format!(", {}", off_by(best, u64::from(*ceiling)))
                    }
                    PatchRefusal::Length { got, want, .. } => format!(", {}", off_by(*got, *want)),
                    _ => String::new(),
                };
                kept.push(format!("{name}: {why}{by}"));
            }
        }
    }
    Ok(Planned {
        patches,
        kept,
        writer,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use or_io::container::rpf::gta6_container_write::Rpf9NewBody;

    use super::super::tests::{
        archive_of, listed, reading_with, row_holding, showing, synthetic_keys, toasts, tree, DOC,
    };
    use super::super::{
        plan_tree_with, tree_edits, write_planned_with, Edit, Saved, Writes, ENDED, PLANNED,
    };
    use super::*;
    use crate::state::jobs::{Outcome, Then};
    use crate::state::rpf_session::RpfDocument;
    use crate::state::{DocPayload, ToastType};

    const BIG: &str = "data/big.bin";

    /// A mebibyte of words, which the writer stores as sixteen 64 KiB chunks.
    fn big() -> Vec<u8> {
        let words = b"a mebibyte of words, chunked under its own row. ";
        let mut body = words.repeat((1 << 20) / words.len() + 1);
        body.truncate(1 << 20);
        body
    }

    /// An archive in `dir` of [`big`] alone, as the tree opens it with every byte changed at its
    /// size, the row's name, and the tree's edits.
    fn big_edited(dir: &Path) -> (PathBuf, RpfDocument, String, Vec<Edit>) {
        let path = archive_of(dir, vec![(BIG.to_owned(), Rpf9NewBody::Chunked(big()))]);
        let mut tree = tree(&path);
        let row = row_holding(&tree.session, &big());
        let name = tree.session.path_of(row).expect("a path");
        (tree.session.set_bytes_for(row, vec![b'b'; 1 << 20])).expect("the row takes the bytes");
        let edits = tree_edits(&tree.session).expect("the tree's edits");
        (path, tree, name, edits)
    }

    #[test]
    fn a_mebibyte_changed_at_its_size_plans_a_patch_as_a_job_the_list_shows_climbing() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(Arc::clone(&keys));
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (path, _, name, edits) = big_edited(dir.path());

        let mut seen = Vec::new();
        let planned = tracked(&path, |tick| {
            plan_tree_with(&path, &edits, &keys, &mut |row, done, total| {
                let go = tick(row, done, total);
                seen.push((done, total, jobs()));
                go
            })
        })
        .expect("the save plans");
        assert_eq!(planned.writes(), Writes::Patched, "{:?}", planned.kept);
        assert!(jobs().is_empty(), "a plan that ended leaves the job list");
        assert_eq!(
            listed(dir.path()),
            ["source.rpf"],
            "planning writes nothing"
        );

        // The bytes planned climb to the body's length, told at each of its sixteen chunks.
        let total = 1u64 << 20;
        assert!(seen.iter().all(|(_, t, _)| *t == total));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(seen.last().map(|s| s.0), Some(total));
        let steps: BTreeSet<u64> = seen.iter().map(|s| s.0).collect();
        assert!(steps.len() >= 16, "{steps:?}");
        let (_, _, shown) = &seen[seen.len() / 2];
        let [job] = shown.as_slice() else {
            panic!("one job is listed: {shown:?}");
        };
        assert_eq!(job.kind, JobKind::Save);
        assert!(job.doing.contains("source.rpf"), "{}", job.doing);
        assert!(job.detail.contains(&name), "{}", job.detail);
        let between = job.fraction.is_some_and(|f| f > 0.0 && f < 1.0);
        assert!(between, "{:?}", job.fraction);
        assert!(!job.stopping);
    }

    #[test]
    fn a_stopped_plan_ends_within_a_second_asks_nothing_and_writes_nothing() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(Arc::clone(&keys));
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (path, tree, _, edits) = big_edited(dir.path());

        let mut stopped_at = None;
        let planned = tracked(&path, |tick| {
            plan_tree_with(&path, &edits, &keys, &mut |row, done, total| {
                // Stopped as Cancel on the job's row stops it, once a chunk is behind it.
                if done > 0 && stopped_at.is_none() {
                    stop();
                    stopped_at = Some(Instant::now());
                }
                tick(row, done, total)
            })
        });
        let took = stopped_at.expect("the plan was stopped").elapsed();
        assert!(
            matches!(planned.as_ref().err(), Some(Unplanned::Stopped)),
            "{:?}",
            planned.err()
        );
        assert!(took < Duration::from_secs(1), "{took:?}");
        assert!(jobs().is_empty(), "a stopped plan leaves the job list");
        assert_eq!(listed(dir.path()), ["source.rpf"]);

        // The poll says the save wrote nothing, asks no question and shows no error.
        let mut state = showing(path.clone(), tree);
        lock(&PLANNED).push((DOC, Err(Unplanned::Stopped)));
        state.poll_archive_saves();
        assert_eq!(state.planned_save_question(), None);
        let ended = state
            .jobs
            .ended()
            .first()
            .expect("the save's end is in the list");
        assert_eq!(ended.kind, JobKind::Save);
        assert_eq!(ended.outcome, Outcome::Stopped(Then::Nothing));
        assert!(
            !(state.toasts.iter()).any(|t| matches!(t.toast_type, ToastType::Error)),
            "{:?}",
            toasts(&state)
        );
        assert_eq!(listed(dir.path()), ["source.rpf"]);

        // A save with no plan held, stopped while it planned, ends the same way.
        state.jobs.clear();
        let out = dir.path().join("copy.rpf");
        lock(&ENDED).push((DOC, 0, out, Err(Unplanned::Stopped)));
        state.poll_archive_saves();
        let ended = state
            .jobs
            .ended()
            .first()
            .map(|e| (e.kind, e.outcome.clone()));
        assert_eq!(
            ended,
            Some((JobKind::Save, Outcome::Stopped(Then::Nothing))),
            "{:?}",
            toasts(&state)
        );
        assert!(
            !(state.toasts.iter()).any(|t| matches!(t.toast_type, ToastType::Error)),
            "{:?}",
            toasts(&state)
        );

        // A plan stopped before its first chunk is stopped too.
        let stopped = tracked(&path, |tick| {
            stop();
            plan_tree_with(&path, &edits, &keys, tick)
        });
        assert!(
            matches!(stopped, Err(Unplanned::Stopped)),
            "{:?}",
            stopped.err()
        );
        assert_eq!(listed(dir.path()), ["source.rpf"]);
    }

    #[test]
    fn a_copy_is_written_as_a_listed_job_to_its_end_and_the_log_then_holds_its_path() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(Arc::clone(&keys));
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (path, tree, _, edits) = big_edited(dir.path());
        let planned =
            plan_tree_with(&path, &edits, &keys, &mut |_, _, _| true).expect("the save plans");
        assert_eq!(planned.writes(), Writes::Patched, "{:?}", planned.kept);
        let out = dir.path().join("copy.rpf");

        let mut seen = Vec::new();
        let saved = tracked_write(&path, &out, |tick| {
            write_planned_with(&path, &out, &keys, &planned, &mut |done, total| {
                let go = tick(done, total);
                seen.push((done, total, jobs()));
                go
            })
        })
        .expect("the copy is written");
        assert_eq!(saved, Saved::Patched);
        assert!(jobs().is_empty(), "a written copy leaves the job list");
        let len = std::fs::metadata(&path).expect("the source").len();
        assert_eq!(
            seen.last().map(|s| (s.0, s.1)),
            Some((len, len)),
            "the tick reaches the total"
        );
        let (_, _, shown) = &seen[0];
        let [job] = shown.as_slice() else {
            panic!("one job is listed: {shown:?}");
        };
        assert_eq!(job.kind, JobKind::Save);
        assert!(job.doing.contains("source.rpf"), "{}", job.doing);
        assert!(
            job.detail.starts_with("writing copy.rpf, "),
            "{}",
            job.detail
        );
        assert_eq!(listed(dir.path()), ["copy.rpf", "source.rpf"]);

        // A stopped plan, then a written copy: the log says the copy was saved, and where.
        let mut state = showing(path.clone(), tree);
        lock(&PLANNED).push((DOC, Err(Unplanned::Stopped)));
        state.poll_archive_saves();
        let first = state.jobs.ended().first().map(|e| e.outcome.clone());
        assert_eq!(first, Some(Outcome::Stopped(Then::Nothing)));
        lock(&ENDED).push((DOC, 0, out.clone(), Ok((Saved::Patched, None))));
        state.poll_archive_saves();
        let ended = state
            .jobs
            .ended()
            .first()
            .expect("the save's end is in the list");
        assert_eq!(ended.kind, JobKind::Save);
        let folder = Then::OpenFolder(dir.path().to_path_buf());
        assert_eq!(ended.outcome, Outcome::Done(folder));
        let copy = out.display().to_string();
        assert!(ended.said.contains(&copy), "{}", ended.said);
    }

    #[test]
    fn a_stopped_write_leaves_no_file_and_the_tree_as_it_was() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(Arc::clone(&keys));
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (path, tree, _, edits) = big_edited(dir.path());
        let planned =
            plan_tree_with(&path, &edits, &keys, &mut |_, _, _| true).expect("the save plans");
        let out = dir.path().join("copy.rpf");

        let stopped = tracked_write(&path, &out, |tick| {
            write_planned_with(&path, &out, &keys, &planned, &mut |done, total| {
                // Stopped as Cancel on the job's row stops it.
                stop();
                tick(done, total)
            })
        });
        assert!(
            matches!(stopped, Err(Unplanned::Stopped)),
            "{:?}",
            stopped.err()
        );
        assert!(jobs().is_empty(), "a stopped write leaves the job list");
        assert_eq!(listed(dir.path()), ["source.rpf"], "and no file");

        let mut state = showing(path.clone(), tree);
        lock(&ENDED).push((DOC, 0, out.clone(), Err(Unplanned::Stopped)));
        state.poll_archive_saves();
        let ended = state.jobs.ended().first().map(|e| e.outcome.clone());
        assert_eq!(ended, Some(Outcome::Stopped(Then::Nothing)));
        let Some(DocPayload::Rpf(rpf)) = state.document(DOC).map(|d| &d.payload) else {
            panic!("the tree is still shown");
        };
        let kept = tree_edits(&rpf.session).expect("the tree's edits");
        assert_eq!(kept.len(), 1, "the tree keeps its edit");
        assert_eq!(listed(dir.path()), ["source.rpf"]);
    }
}
