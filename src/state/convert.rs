//! Converting files to the other edition from the app: the conversion runs on
//! a worker, and what it loses waits on Convert or Cancel before anything is
//! written.
//!
//! ⚠ Each file goes through `or_cli::retarget_file`, the call
//! `openrage-cli convert retarget` makes, so the app and the command cannot
//! report a file differently.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

use or_io::container::rpf::BodyForm;
use or_io::Generation;
use or_job::{Cancel, Progress, Step};

use super::convert_words;
use super::document::{DocumentId, DocumentKind, RpfNodeId};
use super::extract_job::{
    clash_question, extension_of, kept_copy, plan_named_rows, ExtractItem, OnClash, TypeFilter,
};
use super::gltf::model_name;
use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::rpf_session::RpfEditSession;
use super::{AppState, DocPayload, GateOutcome};

const ONLY: &str =
    "Only a drawable, a texture dictionary, a drawable dictionary or a fragment converts";

/// One file of a conversion: as the other edition lays it out, or why it was refused.
pub struct ConvertedFile {
    pub name: String,
    pub outcome: Result<or_cli::Retargeted, String>,
}

/// A finished conversion, waiting on Convert or Cancel.
pub struct Converted {
    pub files: Vec<ConvertedFile>,
    /// No shader database spelled the materials' parameters, so each was dropped.
    pub without_database: bool,
    /// An open document, converted as it stands with its unsaved edits, not a file as it lies in a folder or archive.
    pub of_document: bool,
}

/// What one file of a conversion is read from.
enum Source {
    /// A row's file, read out of its folder or archive.
    Row(ExtractItem),
    /// An open document as a save would write it.
    Open(Vec<u8>),
}

impl Source {
    fn read(self) -> Result<Vec<u8>, String> {
        match self {
            Self::Row(item) => item.read(BodyForm::Decoded),
            Self::Open(bytes) => Ok(bytes),
        }
    }
}

struct Running {
    cancel: Cancel,
    progress: Progress,
    receiver: Receiver<Option<Vec<ConvertedFile>>>,
    label: String,
    without_database: bool,
    of_document: bool,
    /// The archive files on disk its files are read out of, at the offsets they were planned with.
    reads: Vec<PathBuf>,
}

/// A folder a conversion is to be written into that already holds some of its files, waiting on Replace, Skip or Keep both.
#[derive(Debug, Clone)]
pub struct ConvertClash {
    pub dest: PathBuf,
    pub clashes: usize,
    pub planned: usize,
    /// The first file already there.
    pub first: String,
}

impl ConvertClash {
    pub fn question(&self) -> String {
        clash_question(self.clashes, self.planned, &self.first)
    }
}

/// The one conversion running, and the one waiting on its answer.
#[derive(Default)]
pub struct Conversions {
    running: Option<Running>,
    pub done: Option<Converted>,
    /// The answered conversion's write, held while the folder's clash is asked about.
    pub clash: Option<ConvertClash>,
}

impl Conversions {
    /// Run `work`, which returns `None` once it was stopped, as the one conversion, reading the archive files in `reads`.
    pub(super) fn start(
        &mut self,
        label: String,
        without_database: bool,
        of_document: bool,
        reads: Vec<PathBuf>,
        work: impl FnOnce(Cancel, Progress) -> Option<Vec<ConvertedFile>> + Send + 'static,
    ) {
        let (cancel, progress) = (Cancel::new(), Progress::new());
        let (tx, receiver) = channel();
        let (worker_cancel, worker_progress) = (cancel.clone(), progress.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work(worker_cancel, worker_progress));
        });
        self.running = Some(Running {
            cancel,
            progress,
            receiver,
            label,
            without_database,
            of_document,
            reads,
        });
    }

    /// The conversion running, named, and the archive files on disk it reads.
    pub fn reads(&self) -> Option<(String, &[PathBuf])> {
        let running = self.running.as_ref()?;
        let name = format!("the conversion of {}", running.label);
        Some((name, &running.reads))
    }

    /// The conversion running, as the job list draws it.
    pub fn job(&self) -> Option<RunningJob> {
        let running = self.running.as_ref()?;
        let snap = running.progress.snapshot();
        Some(RunningJob {
            kind: JobKind::Conversion,
            doing: format!("Converting {}", running.label),
            fraction: (snap.total > 0).then(|| snap.fraction()),
            detail: format!("{} of {} files  {}", snap.attempted, snap.total, snap.item),
            stopping: running.cancel.is_set(),
        })
    }

    /// ⚠ Stops between files, so the one in hand is finished first.
    pub fn cancel(&self) {
        if let Some(running) = &self.running {
            running.cancel.set();
        }
    }
}

impl Converted {
    /// Each file that converted, with what it became.
    pub fn made(&self) -> impl Iterator<Item = (&str, &or_cli::Retargeted)> {
        (self.files.iter())
            .filter_map(|file| Some((file.name.as_str(), file.outcome.as_ref().ok()?)))
    }

    /// The question's title: the edition, or editions, the files go to.
    pub fn title(&self) -> String {
        let mut to: Vec<&str> = self.made().map(|(_, out)| out.to.edition_label()).collect();
        to.sort_unstable();
        to.dedup();
        if to.is_empty() {
            "Convert".to_string()
        } else {
            format!("Convert to {}", to.join(" and "))
        }
    }

    /// What converts and what it loses, in a sentence.
    pub fn summary(&self) -> String {
        let made: Vec<_> = self.made().collect();
        let dropped: usize = made.iter().map(|(_, out)| out.report.0.len()).sum();
        match (&self.files[..], &made[..]) {
            ([_], [(name, out)]) => {
                let (from, to) = (out.from.edition_label(), out.to.edition_label());
                match dropped {
                    0 => format!("{name} converts from {from} to {to} with every value it holds."),
                    n => format!(
                        "{name} converts from {from} to {to}. {to} has no field for {n} of \
                         the values it holds, which are dropped:"
                    ),
                }
            }
            ([only], []) => format!("{} cannot be converted.", only.name),
            (files, made) => {
                let refused = files.len() - made.len();
                let mut said = format!(
                    "{} of {} files convert, dropping {dropped} values the other edition \
                     has no field for.",
                    made.len(),
                    files.len()
                );
                if refused > 0 {
                    said.push_str(&format!(" {refused} cannot be converted."));
                }
                said
            }
        }
    }

    /// Each file's line and, beneath it, each value it loses, in words.
    ///
    /// ⚠ The values are the ones `openrage-cli convert retarget` reports, which
    /// prints them as the declarations name them.
    pub fn report(&self) -> Vec<FileReport> {
        let one = |file: &ConvertedFile| match &file.outcome {
            Ok(out) => {
                let (from, to) = (out.from.edition_label(), out.to.edition_label());
                let dropped = match out.report.0.len() {
                    0 => "nothing dropped".to_string(),
                    n => format!("{n} dropped"),
                };
                FileReport {
                    line: format!("{}: {from} to {to}, {dropped}", file.name),
                    dropped: (out.report.0.iter()).map(convert_words::dropped).collect(),
                }
            }
            Err(why) => FileReport {
                line: format!("{}: cannot be converted: {why}", file.name),
                dropped: Vec::new(),
            },
        };
        self.files.iter().map(one).collect()
    }
}

/// One file of a conversion's report: its line, and a line for each value it drops.
pub struct FileReport {
    pub line: String,
    pub dropped: Vec<String>,
}

/// Whether a file called `name` is of a kind a conversion takes.
fn takes(name: &str) -> bool {
    or_cli::RETARGETED.contains(&extension_of(name).as_str())
}

/// The edition a row's own resource version names, when the row is a file a conversion takes.
fn edition_of(session: &RpfEditSession, row: RpfNodeId) -> Result<Generation, &'static str> {
    let node = session.node(row).ok_or("That row is no longer there")?;
    if node.is_dir() || !takes(&node.name) {
        return Err(ONLY);
    }
    let ext = extension_of(&node.name);
    (super::files_view::row_facts(session, row).and_then(|facts| facts.version))
        .and_then(|version| {
            let format = or_io::gta5::formats::format(&ext)?;
            format.target_of_version(version).ok()
        })
        .ok_or("OpenRage cannot tell which edition it was made for, so it cannot convert it")
}

impl AppState {
    /// The edition `rows` convert to, `None` when they go both ways; `Err` says why they cannot.
    pub fn conversion_to(
        &self,
        doc: DocumentId,
        rows: &[RpfNodeId],
    ) -> Result<Option<Generation>, &'static str> {
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return Err("That document is closed");
        };
        let mut to = Vec::new();
        for &row in rows {
            let from = edition_of(&rpf.session, row)?;
            to.push(if from.is_gen9() {
                Generation::Gen8
            } else {
                Generation::Gen9
            });
        }
        to.dedup();
        match to[..] {
            [] => Err("Select the files to convert first"),
            [one] => Ok(Some(one)),
            _ => Ok(None),
        }
    }

    /// Whether a menu on `rows` offers Convert: each is a file of a kind a conversion takes.
    pub fn offers_conversion(&self, doc: DocumentId, rows: &[RpfNodeId]) -> bool {
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return false;
        };
        (rows.iter()).all(|&row| {
            (rpf.session.node(row)).is_some_and(|node| !node.is_dir() && takes(&node.name))
        })
    }

    /// Convert `rows` to the other edition on a worker; what they lose is asked about once it ends.
    ///
    /// ponytail: holds every converted file until the answer; a selection is
    /// not an install, whose batches stay in `openrage-cli convert retarget`.
    pub fn convert_rows(&mut self, doc: DocumentId, rows: Vec<RpfNodeId>) {
        let to = match self.conversion_to(doc, &rows) {
            Ok(to) => to,
            Err(why) => {
                self.show_error_toast(why);
                return;
            }
        };
        if self.convert.running.is_some() || self.convert.done.is_some() {
            self.show_error_toast("A conversion is already under way");
            return;
        }
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return;
        };
        // ⚠ Each lands under its own name in one folder, so two of one name keep the first.
        let (items, collided) = plan_named_rows(&rpf.session, &rows, &TypeFilter::All);
        let held = (items.iter())
            .find_map(|item| item.archive_read().filter(|path| self.defrag.holds(path)));
        if let Some(held) = held {
            self.show_error_toast(format!(
                "`{}` is being defragmented; convert once that is done",
                held.display()
            ));
            return;
        }
        if items.is_empty() {
            self.show_warning_toast("Nothing there can be read to convert");
            return;
        }
        if collided > 0 {
            self.show_warning_toast(format!(
                "{collided} selected file(s) share a name with another and were left out, \
                 since both would be written to one file"
            ));
        }
        let label = match (&items[..], to) {
            ([one], Some(to)) => format!("{} to {}", one.top_component(), to.edition_label()),
            (items, _) => format!("{} files", items.len()),
        };
        let reads = items.iter().filter_map(ExtractItem::archive_read).collect();
        let inputs = (items.into_iter())
            .map(|item| (item.top_component().to_string(), Source::Row(item)))
            .collect();
        self.start_conversion(label, inputs, reads, false);
    }

    /// The edition the open document `doc` converts to, or why it does not convert.
    pub fn document_conversion_to(&self, doc: DocumentId) -> Result<Generation, &'static str> {
        let document = self.document(doc).ok_or("That document is closed")?;
        if document.read_only == Some(super::document::rpf9::entry::READ_ONLY) {
            return Err(crate::i18n::tr(super::document::rpf9::entry::READ_ONLY));
        }
        match document.kind {
            DocumentKind::Ytd | DocumentKind::Ydr | DocumentKind::Ydd | DocumentKind::Yft => {}
            _ => return Err(ONLY),
        }
        Ok(if document.payload.generation().is_gen9() {
            Generation::Gen8
        } else {
            Generation::Gen9
        })
    }

    /// Convert the open document `doc` to the other edition as it stands, its unsaved edits included.
    pub fn convert_document(&mut self, doc: DocumentId) {
        let to = match self.document_conversion_to(doc) {
            Ok(to) => to,
            Err(why) => return self.show_error_toast(why),
        };
        if self.convert.running.is_some() || self.convert.done.is_some() {
            return self.show_error_toast("A conversion is already under way");
        }
        let bytes = match self.saved_bytes(doc) {
            Ok(bytes) => bytes,
            Err(why) => return self.show_error_toast(why),
        };
        let Some(name) = self.document(doc).map(model_name) else {
            return;
        };
        let label = format!("{name} to {}", to.edition_label());
        self.start_conversion(label, vec![(name, Source::Open(bytes))], Vec::new(), true);
    }

    /// Run `inputs`, each named, as the one conversion, reading the archive files in `reads`.
    fn start_conversion(
        &mut self,
        label: String,
        inputs: Vec<(String, Source)>,
        reads: Vec<PathBuf>,
        of_document: bool,
    ) {
        let remap = Arc::clone(&self.shader_defs.remap);
        let without_database = remap.shader_count() == 0;
        self.convert.start(
            label,
            without_database,
            of_document,
            reads,
            move |cancel, progress| {
                let mut files = Vec::new();
                let report = or_job::run(inputs, &cancel, &progress, |(name, source)| {
                    progress.now_on(name.clone());
                    let outcome = (source.read())
                        .and_then(|bytes| or_cli::retarget_file(&name, &bytes, &remap));
                    let step = match &outcome {
                        Ok(out) => Step::Done(out.bytes.len() as u64),
                        Err(why) => Step::Failed(why.clone()),
                    };
                    files.push(ConvertedFile { name, outcome });
                    step
                });
                (!report.cancelled).then_some(files)
            },
        );
    }

    /// Answer the question about files a conversion's folder already holds; `None` keeps the conversion's question, to choose another place.
    pub fn answer_conversion_clash(&mut self, answer: Option<OnClash>) {
        let Some(clash) = self.convert.clash.take() else {
            return;
        };
        if let Some(on_clash) = answer {
            self.write_conversion(clash.dest, Some(on_clash));
        }
    }

    /// Write what the answered conversion made: to `dest` for one file, else into the folder `dest`, each under its own name.
    ///
    /// ⚠ The gate is asked here, at the write, so a mode changed while the
    /// conversion ran still holds. The jobs are asked here too: one reading a
    /// file the write would create or replace refuses it, as it does a save.
    /// A refusal keeps the question, so another place can be chosen. Where
    /// the gate would ask about the game as shipped the write is refused
    /// instead, as Export as… is, since no prompt resumes it. A folder that holds some of the files asks Replace,
    /// Skip or Keep both once, as an extraction into it does, and is written
    /// again with the answer as `on_clash`.
    pub fn write_conversion(&mut self, dest: PathBuf, on_clash: Option<OnClash>) {
        let Some(done) = &self.convert.done else {
            return;
        };
        let names: Vec<String> = done.made().map(|(name, _)| name.to_string()).collect();
        if names.is_empty() {
            return;
        }
        let several = names.len() > 1;
        if let Some(what) = self.extract_jobs.running_noun() {
            self.show_error_toast(format!("{what} is running; write once it is done"));
            return;
        }
        match self.gate_permits(Some(&dest)) {
            GateOutcome::Proceed => {}
            GateOutcome::Refused => return,
            GateOutcome::Confirm(_) => {
                self.show_warning_toast(
                    "A conversion does not ask to write over the game as shipped: choose a \
                     place outside the install or in its mods overlay",
                );
                return;
            }
        }
        // ⚠ Each file by its own path, so a job reading one file of the folder does not hold back the rest.
        let targets: Vec<PathBuf> = if several {
            names.iter().map(|name| dest.join(name)).collect()
        } else {
            vec![dest.clone()]
        };
        if let Err(why) = self.no_job_reads_any(targets.iter().map(PathBuf::as_path), "write") {
            self.show_error_toast(why);
            return;
        }
        let taken: Vec<&String> = (names.iter())
            .filter(|name| several && std::fs::symlink_metadata(dest.join(name)).is_ok())
            .collect();
        let on_clash = match (on_clash, taken.first()) {
            (Some(answer), _) => answer,
            (None, Some(first)) => {
                self.convert.clash = Some(ConvertClash {
                    dest,
                    clashes: taken.len(),
                    planned: names.len(),
                    first: first.to_string(),
                });
                return;
            }
            // Nothing was there when asked, so a file that turns up before its
            // write is somebody else's. One file's own dialog asked about replacing it.
            (None, None) if several => OnClash::Skip,
            (None, None) => OnClash::Replace,
        };
        let Some(done) = self.convert.done.take() else {
            return;
        };
        let writes: Vec<(PathBuf, Vec<u8>)> = (done.files.into_iter())
            .filter_map(|file| {
                let path = if several {
                    dest.join(&file.name)
                } else {
                    dest.clone()
                };
                Some((path, file.outcome.ok()?.bytes))
            })
            .collect();
        let (label, folder) = match (several, dest.parent().map(Path::to_path_buf)) {
            (false, Some(parent)) => (file_name(&dest), parent),
            _ => (format!("{} files", writes.len()), dest),
        };
        // The write runs as the extraction's machinery, named as the conversion's job.
        self.extract_jobs.start(
            JobKind::Conversion,
            label,
            folder,
            Vec::new(),
            move |cancel, progress| {
                or_job::run(writes, &cancel, &progress, |(path, bytes)| {
                    progress.now_on(file_name(&path));
                    let mut path = path;
                    if std::fs::symlink_metadata(&path).is_ok() {
                        match on_clash {
                            OnClash::Replace | OnClash::Save => {}
                            OnClash::Skip => return Step::Skipped,
                            OnClash::KeepBoth => path = kept_copy(&path),
                        }
                    }
                    let written = (path.parent().map_or(Ok(()), std::fs::create_dir_all))
                        .and_then(|()| std::fs::write(&path, &bytes));
                    match written {
                        Ok(()) => Step::Done(bytes.len() as u64),
                        Err(e) => Step::Failed(format!("{}: {e}", path.display())),
                    }
                })
            },
        );
    }

    /// Where a finished conversion lands: its question waits on an answer, and a stopped one leaves nothing.
    pub(super) fn poll_conversion(&mut self) {
        let received = match &self.convert.running {
            Some(running) => running.receiver.try_recv(),
            None => return,
        };
        if matches!(received, Err(TryRecvError::Empty)) {
            return;
        }
        let Some(running) = self.convert.running.take() else {
            return;
        };
        let (outcome, said) = match received {
            Ok(Some(files)) => {
                self.convert.done = Some(Converted {
                    files,
                    without_database: running.without_database,
                    of_document: running.of_document,
                });
                let said = format!("{}: its report waits on Convert or Cancel", running.label);
                (Outcome::Done(Then::Nothing), said)
            }
            Ok(None) => (
                Outcome::Stopped(Then::Nothing),
                "Stopped before it finished; nothing was written".to_string(),
            ),
            Err(_) => (
                Outcome::Failed,
                "It stopped on an internal error; nothing was written".to_string(),
            ),
        };
        self.jobs.record(JobKind::Conversion, outcome, said);
    }
}
