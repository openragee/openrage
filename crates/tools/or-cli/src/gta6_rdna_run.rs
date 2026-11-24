//! How `rpf scan-rdna` runs: the programs of a kept corpus translated by child processes of this
//! tool, a range each, so that a program that overflows a stack, panics or never ends costs its
//! own outcome and a restart of its range, never the run.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use naga::valid::{Capabilities, ValidationFlags, Validator};
use or_shader_front::rdna::{self, Options, Program};
use or_shader_front::rdna_cache::{Cache, Cached, Key};
use or_shader_front::rdna_corpus::Corpus;
use serde::{Deserialize, Serialize};

use super::gta6_rdna_backends::{self as backends, fold, panic_text};
use super::gta6_rdna_limits as limits;

pub(crate) const TRANSLATED: &str = "translated";
/// Translated to a module naga's validator refuses: a fault of the lowering, never of the program.
pub(crate) const INVALID: &str = "invalid";
pub(crate) const REFUSED: &str = "refused";
pub(crate) const PANICKED: &str = "panicked";
pub(crate) const TIMED_OUT: &str = "timed out";
/// The worker died on it with no word, as a stack overflow kills a process.
pub(crate) const CRASHED: &str = "crashed";
/// The worker died on it asking for memory past its ceiling.
pub(crate) const OUT_OF_MEMORY: &str = "ran out of memory";
/// The program's stream did not inflate or its container did not unwrap.
pub(crate) const UNREAD: &str = "unread";

/// The stack a program is translated on: reserved, and committed only as far as it is used.
const STACK: usize = 512 << 20;

/// One block of one program and what the scan made of it: a line of `--json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    /// The program's place in the container, and the block's in the program.
    pub index: usize,
    pub block: usize,
    pub stage: String,
    /// What the translation is cached under: the block's content, the options and the translator.
    pub key: String,
    pub words: usize,
    pub outcome: String,
    /// Whether the words follow as control flow from the first to an end.
    pub graph: bool,
    /// The census word of a refusal, the innermost cause of an invalid module, a panic's message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The microseconds the translation took, or its load from the cache.
    pub micros: u64,
    /// Whether that was a load: from the translation cache, or of the outcome of the same block elsewhere in the run.
    pub cached: bool,
    /// The microseconds naga's validator took, and with `--backends` its writers.
    #[serde(default)]
    pub checked_micros: u64,
    /// The most memory its worker had committed when it was done, in megabytes, and how much of
    /// that the block added: a worker's peak only grows, so the growth is the block's own.
    #[serde(default)]
    pub peak_mb: u64,
    #[serde(default)]
    pub grew_mb: u64,
    pub resources: usize,
    /// What a device has to allow: subgroups, and with `--backends` the SPIR-V capabilities past the common ones.
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub approximations: BTreeMap<String, u32>,
    /// With `--backends`, each writer that refuses the module, panics on it or was not run, and why.
    #[serde(default)]
    pub backends: BTreeMap<String, String>,
    /// A digest of the module as it was lowered: two runs differ in it where the lowered code does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub module: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wgsl: Option<String>,
}

/// A line a worker prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Said {
    /// The block it starts on: what it died on, if it says nothing more.
    Begin(Record),
    Done(Record),
    /// The block so far and the writer it starts on: what it died writing, if it says nothing more.
    Writing(Record, String),
    /// Said for it once it stopped in a writer: the block as it stood, the stop held against that
    /// writer. The block is not run again.
    Lost(Record),
    /// Its range is finished.
    End,
    /// Said for it once it is dead: the last line of what it wrote as it died.
    Died(String),
}

/// What a worker is asked for past the outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Want {
    pub backends: bool,
    pub wgsl: bool,
}

/// A block's stage as the report names it.
pub(crate) fn stage_of(program: &Program) -> String {
    format!("{:?} (slot {})", program.kind, program.slot).to_lowercase()
}

/// An error and what caused it, outermost first.
fn chain(error: &dyn std::error::Error) -> Vec<String> {
    let mut out = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        out.push(cause.to_string());
        source = cause.source();
    }
    out
}

/// A digest of everything `module` holds.
fn digest(module: &naga::Module) -> String {
    use std::fmt::Write as _;
    use std::hash::Hasher as _;
    struct Hashed(std::collections::hash_map::DefaultHasher);
    impl std::fmt::Write for Hashed {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0.write(text.as_bytes());
            Ok(())
        }
    }
    let mut hashed = Hashed(std::collections::hash_map::DefaultHasher::new());
    let _ = write!(hashed, "{module:?}");
    format!("{:016x}", hashed.0.finish())
}

/// `program` translated, validated and, where wanted, written by every back end, into `record`.
fn judge(
    program: &Program,
    options: &Options,
    cache: &Cache,
    want: Want,
    mut record: Record,
    said: fn(&Said),
) -> Record {
    let started = Instant::now();
    backends::set_doing("translating");
    let (cached, kept) = cache.get_or_translate(program, options);
    record.micros = started.elapsed().as_micros() as u64;
    record.cached = kept;
    let lowered = match &*cached {
        Cached::Refused { name, message } => {
            record.outcome = REFUSED.to_owned();
            record.graph = name != "graph";
            record.reason = Some(name.clone());
            record.detail = Some(message.clone());
            return record;
        }
        Cached::Translated(lowered) => lowered,
    };
    record.graph = true;
    record.resources = lowered.resources.len();
    record.approximations = (lowered.approximations.iter())
        .map(|(name, count)| ((*name).to_owned(), *count))
        .collect();
    if lowered.subgroups {
        record.features.push("subgroups".to_owned());
    }
    let module = &lowered.module;
    record.module = digest(module);
    let started = Instant::now();
    backends::set_doing("validating");
    match Validator::new(ValidationFlags::all(), Capabilities::all()).validate(module) {
        Err(why) => {
            let causes = chain(&why.into_inner());
            record.outcome = INVALID.to_owned();
            record.reason = causes.last().map(|cause| fold(cause));
            record.detail = Some(causes.join(": "));
        }
        Ok(info) => {
            record.outcome = TRANSLATED.to_owned();
            record.features.extend(backends::requires(module));
            if want.backends {
                let so_far = record.clone();
                let before =
                    |writer: &str, failures: &BTreeMap<String, String>, needed: &[String]| {
                        let mut partial = so_far.clone();
                        partial.backends = failures.clone();
                        partial.features.extend(needed.iter().cloned());
                        said(&Said::Writing(partial, writer.to_owned()));
                    };
                let (failures, capabilities) = backends::check(module, &info, before);
                record.backends = failures;
                record.features.extend(capabilities);
            }
            if want.wgsl {
                record.wgsl = backends::wgsl(module, &info).ok();
            }
        }
    }
    record.checked_micros = started.elapsed().as_micros() as u64;
    record
}

/// The outcome a worker of this run left in `memo` for the block `stub` names by its key, as `stub`'s own.
fn remembered(memo: &Path, stub: &Record) -> Option<Record> {
    // A file another worker is still writing does not parse, and the block is judged here too.
    let kept: Record = serde_json::from_slice(&std::fs::read(memo.join(&stub.key)).ok()?).ok()?;
    Some(Record {
        index: stub.index,
        block: stub.block,
        cached: true,
        ..kept
    })
}

/// Leaves `record` in `memo` for the workers that meet the same block.
fn remember(memo: &Path, record: &Record) {
    if let Ok(line) = serde_json::to_vec(record) {
        let _ = std::fs::write(memo.join(&record.key), line);
    }
}

/// `record` as a worker left it when it stopped in `writer`: `why` held against that writer, and
/// the writers after it not run.
fn lost(mut record: Record, writer: &str, why: &str) -> Record {
    let after = backends::ORDER.iter().skip_while(|name| **name != writer);
    for later in after.skip(1) {
        let unrun = format!("not run: the worker stopped writing {writer}");
        record.backends.insert((*later).to_owned(), unrun);
    }
    record.backends.insert(writer.to_owned(), why.to_owned());
    record.detail = Some(why.to_owned());
    record
}

/// Why a job gave no answer.
#[derive(Debug, PartialEq, Eq)]
enum Stopped {
    Panicked(String),
    TimedOut,
}

/// `job`'s answer from a thread of its own with a large stack, if it gives one within `budget`.
/// A job past its budget keeps its thread: the caller has to end the process to be rid of it.
fn with_budget<T: Send + 'static>(
    budget: Duration,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Stopped> {
    let (answer, heard) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let _ = answer.send(catch_unwind(AssertUnwindSafe(job)));
        });
    if let Err(why) = spawned {
        return Err(Stopped::Panicked(format!("no thread to run it on: {why}")));
    }
    match heard.recv_timeout(budget) {
        Ok(Ok(answer)) => Ok(answer),
        Ok(Err(panic)) => Err(Stopped::Panicked(panic_text(&*panic))),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Stopped::TimedOut),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Stopped::Panicked(
            "its thread ended with no answer".to_owned(),
        )),
    }
}

fn say(said: &Said) {
    // Standard output flushes at each line, so the parent has every line said before a crash.
    println!("{}", serde_json::to_string(said).unwrap_or_default());
}

/// The exit status of a worker that stopped on a program past its budget.
const EXIT_TIMED_OUT: i32 = 3;

/// `rpf scan-rdna-worker <corpus> <start> <end> <first block> <budget ms> <backends> <wgsl> <cache | -> <memo | -> <memory MB>`:
/// the programs `start..end` of a corpus, from `first block` of the first, a [`Said`] a line.
pub(crate) fn cmd_worker(args: &[String]) -> Result<(), String> {
    let [corpus, start, end, first, budget, with_backends, with_wgsl, cache, memo, memory] = args
    else {
        return Err("scan-rdna-worker is run by scan-rdna".to_owned());
    };
    let number = |word: &String| (word.parse::<usize>()).map_err(|e| format!("{word}: {e}"));
    let (start, end, first) = (number(start)?, number(end)?, number(first)?);
    let budget = Duration::from_millis(number(budget)? as u64);
    let want = Want {
        backends: with_backends == "1",
        wgsl: with_wgsl == "1",
    };
    // Before anything is read: an allocation past the ceiling ends this process, and the scan
    // counts the block it was on as out of memory.
    limits::limit_memory(number(memory)? as u64);
    let memo = (memo != "-").then(|| Path::new(memo));
    let pairs = super::gta6_rdna_pairs::read(&super::gta6_rdna_pairs::beside(Path::new(corpus)));
    let mut corpus = Corpus::open(Path::new(corpus)).map_err(|e| format!("{corpus}: {e}"))?;
    // A panic is an outcome here. Where the build cannot unwind it ends the worker, so its one
    // line is the worker's last word: what panicked, doing what, and where.
    std::panic::set_hook(Box::new(|info| {
        // The file by its name alone: its folders are the machine it was built on.
        let at = (info.location()).map_or(String::new(), |at| {
            let file = at
                .file()
                .rsplit(std::path::is_separator)
                .next()
                .unwrap_or("");
            format!(" at {file}:{}", at.line())
        });
        eprintln!(
            "panicked {}: {}{at}",
            backends::doing(),
            panic_text(info.payload())
        );
    }));
    for index in start..end.min(corpus.len()) {
        let first = if index == start { first } else { 0 };
        let blocks = (corpus.program(index).ok().flatten())
            .and_then(|dxbc| catch_unwind(|| rdna::unwrap(&dxbc).ok()).ok().flatten());
        let Some(blocks) = blocks else {
            if first == 0 {
                say(&Said::Done(Record {
                    index,
                    stage: "unread".to_owned(),
                    outcome: UNREAD.to_owned(),
                    ..Record::default()
                }));
            }
            continue;
        };
        for (block, program) in blocks.into_iter().enumerate().skip(first) {
            let options = super::gta6_rdna_pairs::options(&mut corpus, &pairs, index, &program);
            let stub = Record {
                index,
                block,
                stage: stage_of(&program),
                key: Key::of(&program, &options).hex(),
                words: program.words.len(),
                ..Record::default()
            };
            say(&Said::Begin(stub.clone()));
            let (started, held) = (Instant::now(), limits::peak_mb());
            let measured = |record: Record| {
                let peak_mb = limits::peak_mb();
                Record {
                    peak_mb,
                    grew_mb: peak_mb.saturating_sub(held),
                    ..record
                }
            };
            if let Some(mut record) = memo.and_then(|memo| remembered(memo, &stub)) {
                record.micros = started.elapsed().as_micros() as u64;
                say(&Said::Done(record));
                continue;
            }
            // A cache of this block alone: one that lived as long as the worker would hold every
            // module of its range in memory.
            let cache = match cache.as_str() {
                "-" => Cache::in_memory(),
                dir => Cache::on_disk(dir),
            };
            let record = stub.clone();
            let stopped = match with_budget(budget, move || {
                judge(&program, &options, &cache, want, record, say)
            }) {
                Ok(record) => {
                    let record = measured(record);
                    if let Some(memo) = memo {
                        remember(memo, &record);
                    }
                    say(&Said::Done(record));
                    continue;
                }
                Err(stopped) => stopped,
            };
            let record = measured(Record {
                micros: started.elapsed().as_micros() as u64,
                ..stub
            });
            match stopped {
                Stopped::Panicked(message) => say(&Said::Done(Record {
                    outcome: PANICKED.to_owned(),
                    reason: Some(fold(&message)),
                    detail: Some(message),
                    ..record
                })),
                Stopped::TimedOut => {
                    say(&Said::Done(Record {
                        outcome: TIMED_OUT.to_owned(),
                        reason: Some(format!("over {} ms", budget.as_millis())),
                        ..record
                    }));
                    // The thread still runs and cannot be stopped: the parent takes the range up after this block.
                    std::process::exit(EXIT_TIMED_OUT);
                }
            }
        }
    }
    say(&Said::End);
    Ok(())
}

/// Where a range is taken up: a program, and a block of it.
pub(crate) type Place = (usize, usize);

/// Runs `worker` over `range` until every program of it has an outcome, handing each to `done`.
/// `worker` runs from a place to the range's end and says what a worker process prints. One that
/// stops short of [`Said::End`] without the outcome of the block it was on is run again from that
/// block, since a worker can die of the machine's troubles; a block a worker dies on twice is
/// counted crashed, or out of memory where its last word says so, and the range goes on after
/// it. A block a worker stopped writing is held against the writer and never run again. Returns how many times a worker was started again.
pub(crate) fn run_range(
    range: Range<usize>,
    mut worker: impl FnMut(Place, usize, &mut dyn FnMut(Said)),
    mut done: impl FnMut(Record),
) -> usize {
    let mut next: Place = (range.start, 0);
    let mut died_at: Option<Place> = None;
    let mut restarts = 0;
    while next.0 < range.end {
        let from = next;
        let mut begun: Option<Record> = None;
        let mut last_word: Option<String> = None;
        let (mut ended, mut gave_up) = (false, false);
        worker(next, range.end, &mut |said| match said {
            Said::Begin(stub) => begun = Some(stub),
            Said::Done(record) => {
                next = (record.index, record.block + 1);
                gave_up = record.outcome == TIMED_OUT;
                begun = None;
                done(record);
            }
            Said::Lost(record) => {
                next = (record.index, record.block + 1);
                gave_up = true;
                begun = None;
                done(record);
            }
            Said::Writing(..) => {}
            Said::End => ended = true,
            Said::Died(word) => last_word = Some(word),
        });
        if ended {
            break;
        }
        restarts += 1;
        // It stopped itself after a block past its budget and said so, or stopped in a writer.
        if begun.is_none() && gave_up {
            continue;
        }
        // A worker that began a block before the place it was started at is not one of this
        // tool's: its program is given up, so that the range ends whatever a worker says.
        if begun
            .as_ref()
            .is_some_and(|stub| (stub.index, stub.block) < from)
        {
            next = (from.0 + 1, 0);
            continue;
        }
        // With no block begun it died between two, or never started. A program some block of
        // which is done was read whole, so what it died reading is the one after.
        let at = match &begun {
            Some(stub) => (stub.index, stub.block),
            None => (next.0 + usize::from(next.1 > 0), 0),
        };
        next = at;
        if died_at != Some(at) {
            died_at = Some(at);
            continue;
        }
        let word = last_word.unwrap_or_else(|| "it left no word".to_owned());
        let outcome = match () {
            () if word.contains("memory allocation") => OUT_OF_MEMORY,
            () if word.starts_with("panicked ") => PANICKED,
            () => CRASHED,
        };
        next = match begun {
            Some(_) => (at.0, at.1 + 1),
            None => (at.0 + 1, 0),
        };
        let lost = begun.unwrap_or_else(|| Record {
            index: at.0,
            stage: "unknown".to_owned(),
            ..Record::default()
        });
        if at.0 < range.end {
            done(Record {
                outcome: outcome.to_owned(),
                reason: Some(format!("the worker died on it twice: {}", fold(&word))),
                detail: Some(word),
                ..lost
            });
        }
    }
    restarts
}

/// The line of a dead worker's last words that says what it died of: the failed allocation, the
/// overflowed stack or the panic where one is named, since a note about backtraces can follow it, else the last.
fn last_word(words: &str) -> String {
    let lines = || words.lines().map(str::trim).filter(|line| !line.is_empty());
    let telling = ["memory allocation", "overflowed", "panicked "];
    (lines().find(|line| telling.iter().any(|word| line.contains(word))))
        .or_else(|| lines().next_back())
        .unwrap_or("it left no word")
        .to_owned()
}

/// What a scan is run with.
#[derive(Debug, Clone)]
pub(crate) struct Plan<'a> {
    pub corpus: &'a Path,
    pub budget: Duration,
    pub want: Want,
    /// The directory translations are kept in, if they are kept.
    pub cache: Option<&'a Path>,
    /// A directory of this run alone, where a worker leaves each block's outcome for the
    /// workers that meet the same block: the same code is in many programs.
    pub memo: Option<&'a Path>,
    /// The memory a worker may commit, in megabytes.
    pub memory_mb: u64,
}

impl Plan<'_> {
    /// A child of this tool over `from..end`, each line it prints handed to `heard`.
    pub(crate) fn worker(&self, from: Place, end: usize, heard: &mut dyn FnMut(Said)) {
        let flag = |on: bool| if on { "1" } else { "0" };
        let child = std::env::current_exe().and_then(|exe| {
            Command::new(exe)
                .args(["rpf", "scan-rdna-worker"])
                .arg(self.corpus)
                .args([from.0, end, from.1].map(|n| n.to_string()))
                .arg(self.budget.as_millis().to_string())
                .args([flag(self.want.backends), flag(self.want.wgsl)])
                .arg(self.cache.unwrap_or(Path::new("-")))
                .arg(self.memo.unwrap_or(Path::new("-")))
                .arg(self.memory_mb.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                // A worker is silent but for its last words, which are few: read once it is dead.
                .stderr(Stdio::piped())
                .spawn()
        });
        let Ok(mut child) = child else {
            return;
        };
        // The writer the worker is in, and the block as it stood when it started on it.
        let mut writing: Option<(Record, String)> = None;
        let lose = |(record, writer): (Record, String), why: &str| {
            let record = lost(record, &writer, why);
            if let Some(memo) = self.memo {
                remember(memo, &record);
            }
            Said::Lost(record)
        };
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                // A line cut short by the crash is no line.
                match serde_json::from_str(&line) {
                    Ok(Said::Writing(record, writer)) => writing = Some((record, writer)),
                    Ok(Said::Done(record)) => match writing.take() {
                        Some(partial) if record.outcome == TIMED_OUT => {
                            let over = record.reason.unwrap_or_default();
                            heard(lose(partial, &format!("timed out: {over}")));
                        }
                        _ => heard(Said::Done(record)),
                    },
                    Ok(said) => heard(said),
                    Err(_) => {}
                }
            }
        }
        let mut last_words = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut last_words);
        }
        if !child.wait().is_ok_and(|status| status.success()) {
            let word = last_word(&last_words);
            match writing {
                Some(partial) => heard(lose(partial, &word)),
                None => heard(Said::Died(word)),
            }
        }
    }
}

/// `range` cut into batches and run by `jobs` threads, a worker each at a time; the records in
/// order, and how many workers were restarted. `progress` hears how many programs are finished.
pub(crate) fn scan(
    range: Range<usize>,
    jobs: usize,
    batch: usize,
    worker: impl Fn(Place, usize, &mut dyn FnMut(Said)) + Sync,
    progress: impl Fn(usize) + Sync,
) -> (Vec<Record>, usize) {
    let batch = batch.max(1);
    let (next, finished, restarts) = (
        AtomicUsize::new(range.start),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    );
    let records = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| loop {
                let start = next.fetch_add(batch, Ordering::Relaxed);
                if start >= range.end {
                    break;
                }
                let end = (start + batch).min(range.end);
                let mut mine = Vec::new();
                let again = run_range(start..end, &worker, |record| mine.push(record));
                restarts.fetch_add(again, Ordering::Relaxed);
                (records.lock().unwrap_or_else(PoisonError::into_inner)).append(&mut mine);
                progress(finished.fetch_add(end - start, Ordering::Relaxed) + end - start);
            });
        }
    });
    let mut records = records.into_inner().unwrap_or_else(PoisonError::into_inner);
    records.sort_by_key(|record| (record.index, record.block));
    (records, restarts.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(index: usize, block: usize, outcome: &str) -> Record {
        Record {
            index,
            block,
            stage: "pixel (slot 0)".to_owned(),
            outcome: outcome.to_owned(),
            ..Record::default()
        }
    }

    /// A made-up worker over programs of two blocks each: it dies on the blocks of `crash` every
    /// time, stops itself after those of `slow`, and dies before it starts program `absent`.
    fn worker<'a>(
        crash: &'a [Place],
        slow: &'a [Place],
        absent: Option<usize>,
    ) -> impl Fn(Place, usize, &mut dyn FnMut(Said)) + Sync + 'a {
        move |from, end, heard| {
            for index in from.0..end {
                if absent == Some(index) {
                    return;
                }
                for block in (if index == from.0 { from.1 } else { 0 })..2 {
                    heard(Said::Begin(record(index, block, "")));
                    if crash.contains(&(index, block)) {
                        return;
                    }
                    if slow.contains(&(index, block)) {
                        heard(Said::Done(record(index, block, TIMED_OUT)));
                        return;
                    }
                    heard(Said::Done(record(index, block, TRANSLATED)));
                }
            }
            heard(Said::End);
        }
    }

    fn outcomes(records: &[Record]) -> Vec<(usize, usize, &str)> {
        (records.iter())
            .map(|r| (r.index, r.block, r.outcome.as_str()))
            .collect()
    }

    #[test]
    fn a_crash_costs_its_block_and_the_range_goes_on_after_it() {
        let mut records = Vec::new();
        let restarts = run_range(3..6, worker(&[(4, 0), (5, 1)], &[], None), |r| {
            records.push(r)
        });
        assert_eq!(restarts, 4);
        assert_eq!(
            outcomes(&records),
            [
                (3, 0, TRANSLATED),
                (3, 1, TRANSLATED),
                (4, 0, CRASHED),
                (4, 1, TRANSLATED),
                (5, 0, TRANSLATED),
                (5, 1, CRASHED),
            ]
        );
        assert_eq!(records[2].stage, "pixel (slot 0)");
    }

    #[test]
    fn a_worker_that_dies_once_loses_nothing_and_its_last_word_names_the_outcome() {
        // The first worker dies on block (1, 0); the one run again from there does not.
        let deaths = std::cell::Cell::new(0);
        let steady = worker(&[], &[], None);
        let mut records = Vec::new();
        let restarts = run_range(
            0..3,
            |from, end, heard| {
                if from == (0, 0) {
                    deaths.set(deaths.get() + 1);
                    heard(Said::Done(record(0, 0, TRANSLATED)));
                    heard(Said::Done(record(0, 1, TRANSLATED)));
                    heard(Said::Begin(record(1, 0, "")));
                    return;
                }
                steady(from, end, heard);
            },
            |r| records.push(r),
        );
        assert_eq!((restarts, deaths.get()), (1, 1));
        assert_eq!(records.len(), 6);
        assert!(records.iter().all(|r| r.outcome == TRANSLATED));
        // One that dies there every time, saying an allocation failed, is out of memory.
        let mut records = Vec::new();
        run_range(
            0..1,
            |from, _, heard| {
                if from != (0, 0) {
                    return heard(Said::End);
                }
                heard(Said::Begin(record(0, 0, "")));
                heard(Said::Died(
                    "memory allocation of 4096 bytes failed".to_owned(),
                ));
            },
            |r| records.push(r),
        );
        assert_eq!(outcomes(&records), [(0, 0, OUT_OF_MEMORY)]);
        assert_eq!(
            last_word("\nmemory allocation of 8 bytes failed\nnote: a made-up note\n"),
            "memory allocation of 8 bytes failed"
        );
        assert_eq!(last_word("first\nlast\n\n"), "last");
        assert_eq!(
            last_word("panicked writing made-up: why at a.rs:1:2\nnote: a made-up note"),
            "panicked writing made-up: why at a.rs:1:2"
        );
        assert_eq!(last_word(""), "it left no word");
        assert_eq!(
            records[0].reason.as_deref(),
            Some("the worker died on it twice: memory allocation of N bytes failed")
        );
    }

    #[test]
    fn a_block_a_worker_stopped_writing_keeps_its_translation_and_is_not_run_again() {
        let [first, second, third, last] = backends::ORDER;
        let mut partial = record(0, 1, TRANSLATED);
        (partial.backends).insert(first.to_owned(), "a made-up refusal".to_owned());
        let held = lost(partial, second, "panicked writing made-up at a.rs:7");
        assert_eq!(held.outcome, TRANSLATED);
        assert_eq!(held.backends[first], "a made-up refusal");
        assert_eq!(held.backends[second], "panicked writing made-up at a.rs:7");
        for unrun in [third, last] {
            assert!(held.backends[unrun].starts_with("not run: "));
        }
        let starts = std::cell::Cell::new(0);
        let steady = worker(&[], &[], None);
        let mut records = Vec::new();
        let restarts = run_range(
            0..2,
            |from, end, heard| {
                starts.set(starts.get() + 1);
                if from == (0, 0) {
                    heard(Said::Done(record(0, 0, TRANSLATED)));
                    heard(Said::Begin(record(0, 1, "")));
                    return heard(Said::Lost(held.clone()));
                }
                steady(from, end, heard);
            },
            |r| records.push(r),
        );
        assert_eq!((restarts, starts.get()), (1, 2));
        assert_eq!(records.len(), 4);
        assert!(records.iter().all(|r| r.outcome == TRANSLATED));
    }

    #[test]
    fn a_block_past_its_budget_is_not_counted_twice() {
        let mut records = Vec::new();
        let restarts = run_range(0..2, worker(&[], &[(0, 1)], None), |r| records.push(r));
        assert_eq!(restarts, 1);
        assert_eq!(
            outcomes(&records),
            [
                (0, 0, TRANSLATED),
                (0, 1, TIMED_OUT),
                (1, 0, TRANSLATED),
                (1, 1, TRANSLATED),
            ]
        );
    }

    #[test]
    fn a_worker_that_dies_before_a_program_gives_that_program_up() {
        let mut records = Vec::new();
        let restarts = run_range(0..3, worker(&[], &[], Some(1)), |r| records.push(r));
        assert_eq!(restarts, 2);
        assert_eq!(
            outcomes(&records),
            [
                (0, 0, TRANSLATED),
                (0, 1, TRANSLATED),
                (1, 0, CRASHED),
                (2, 0, TRANSLATED),
                (2, 1, TRANSLATED),
            ]
        );
        // A worker that never starts ends the range all the same.
        let mut given_up = 0;
        assert_eq!(run_range(0..4, |_, _, _| {}, |_| given_up += 1), 8);
        assert_eq!(given_up, 4);
    }

    #[test]
    fn batches_run_side_by_side_give_every_program_once_in_order() {
        let finished = AtomicUsize::new(0);
        let (records, restarts) = scan(
            0..50,
            4,
            7,
            worker(&[(13, 1), (14, 0), (49, 1)], &[(20, 0)], Some(30)),
            |n| {
                finished.fetch_max(n, Ordering::Relaxed);
            },
        );
        assert_eq!(restarts, 9);
        assert_eq!(finished.into_inner(), 50);
        assert_eq!(records.len(), 99);
        assert!(records.is_sorted_by_key(|r| (r.index, r.block)));
        let crashed: Vec<Place> = (records.iter())
            .filter(|r| r.outcome == CRASHED)
            .map(|r| (r.index, r.block))
            .collect();
        assert_eq!(crashed, [(13, 1), (14, 0), (30, 0), (49, 1)]);
        assert_eq!(records.iter().filter(|r| r.outcome == TIMED_OUT).count(), 1);
    }

    #[test]
    fn a_job_answers_on_a_deep_stack_or_runs_out_of_time() {
        let long = Duration::from_secs(60);
        assert_eq!(with_budget(long, || 7), Ok(7));
        // Deep enough to overflow a thread's default stack, and well inside the one a job gets.
        fn depth(n: u64) -> u64 {
            let pad = std::hint::black_box([n; 64]);
            if n == 0 {
                pad[0]
            } else {
                depth(n - 1) + 1
            }
        }
        assert_eq!(with_budget(long, || depth(20_000)), Ok(20_000));
        // A panic is not tried: the test profile cannot unwind, and there a panic ends the worker,
        // which the range's restart counts as a crash.
        let slow = with_budget(Duration::from_millis(20), || {
            std::thread::sleep(Duration::from_secs(2))
        });
        assert_eq!(slow, Err(Stopped::TimedOut));
    }

    #[test]
    fn a_block_met_again_takes_the_outcome_left_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = Record {
            key: "made-up-key".to_owned(),
            words: 4,
            micros: 900,
            ..record(3, 1, REFUSED)
        };
        let again = Record {
            key: first.key.clone(),
            ..record(8, 0, "")
        };
        assert_eq!(remembered(dir.path(), &again), None);
        remember(dir.path(), &first);
        let taken = remembered(dir.path(), &again).expect("the outcome was left");
        assert_eq!((taken.index, taken.block, taken.cached), (8, 0, true));
        assert_eq!((taken.outcome.as_str(), taken.words), (REFUSED, 4));
        // Half a file is no outcome.
        std::fs::write(dir.path().join("made-up-key"), b"{\"index\":").unwrap();
        assert_eq!(remembered(dir.path(), &again), None);
    }

    #[test]
    fn a_program_is_judged_by_what_stopped_it() {
        use or_shader_front::dxil::ShaderKind;
        let program = |kind, words| Program {
            kind,
            slot: 0,
            words,
            wave32: false,
            inputs: Vec::new(),
            vertex_inputs: Vec::new(),
            position_read: 0,
            front_face: false,
            launch: None,
        };
        const S_ENDPGM: u32 = 0xBF81_0000;
        let cache = Cache::in_memory();
        let want = Want {
            backends: true,
            wgsl: true,
        };
        let judged = |program: &Program| {
            judge(
                program,
                &Options::default(),
                &cache,
                want,
                Record::default(),
                |_| {},
            )
        };
        // A pixel program that only ends translates; a word of no family gives no graph.
        let ends = judged(&program(ShaderKind::Pixel, vec![S_ENDPGM]));
        assert_eq!((ends.outcome.as_str(), ends.graph), (TRANSLATED, true));
        assert_eq!(ends.backends, BTreeMap::new());
        assert!(ends.wgsl.is_some_and(|text| text.contains("fn ")));
        let stray = judged(&program(ShaderKind::Pixel, vec![0xC000_0000]));
        assert_eq!((stray.outcome.as_str(), stray.graph), (REFUSED, false));
        assert_eq!(stray.reason.as_deref(), Some("graph"));
        // The same program again is answered from the cache.
        assert!(judged(&program(ShaderKind::Pixel, vec![S_ENDPGM])).cached);
    }
}
