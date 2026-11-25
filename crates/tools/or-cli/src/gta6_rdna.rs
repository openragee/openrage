//! `rpf scan-rdna`: every program of a GTA VI effect container through the native-code front end,
//! as counts by stage and by refusal, a record a block for `--json`, the blocks of a refusal class,
//! and on request one program's disassembly and its lowered WGSL. The programs are kept inflated
//! under the configuration directory and translated by workers ([`super::gta6_rdna_run`]).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::panic::catch_unwind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind};
use or_io::gta5::shaders::sgd5::Sgd5File;
use or_shader_front::rdna;
use or_shader_front::rdna_corpus::{self, Corpus};
use sha1::{Digest, Sha1};

use super::gta6_rdna_backends::ORDER;
use super::gta6_rdna_diff::{place, write_records};
use super::gta6_rdna_limits as limits;
use super::gta6_rdna_run::{
    scan, Plan, Record, Want, CRASHED, INVALID, OUT_OF_MEMORY, PANICKED, REFUSED, TIMED_OUT,
    TRANSLATED, UNREAD,
};

/// The outcomes of a block that stopped a worker or was stopped by one.
const STOPPED: [&str; 4] = [PANICKED, TIMED_OUT, CRASHED, OUT_OF_MEMORY];

/// The reasons of a stage printed at most, the commonest first.
const TOP_REASONS: usize = 12;
/// The blocks listed at most where no `--sample` says how many.
const SAMPLE: usize = 20;
/// The blocks named for the memory they took.
const HUNGRY: usize = 5;
/// The programs a worker is handed at a time: few enough that a slow range is shared out.
const BATCH: usize = 48;
/// How long a block may take before it is given up, in milliseconds.
const BUDGET_MS: u64 = 10_000;
/// The bytes at the body's offset a kept corpus is named by, with the archive's size and date.
const FINGERPRINT_BYTES: usize = 64 << 10;

/// Takes `--flag <value>` out of `args`.
fn take_value(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let Some(at) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };
    if at + 1 >= args.len() {
        return Err(format!("{flag} needs a value"));
    }
    Ok(args.drain(at..at + 2).nth(1))
}

/// [`take_value`] as a number.
fn take_number(args: &mut Vec<String>, flag: &str) -> Result<Option<usize>, String> {
    take_value(args, flag)?
        .map(|word| (word.parse()).map_err(|e| format!("{flag} {word} is not a number: {e}")))
        .transpose()
}

/// Takes `flag` out of `args`; whether it was there.
fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != flag);
    args.len() != before
}

/// Where the scan keeps what it need not make twice.
fn cache_dir() -> Result<PathBuf, String> {
    or_title::config::config_dir()
        .map(|dir| dir.join("cache"))
        .ok_or_else(|| "this platform has no configuration directory to keep a cache in".to_owned())
}

/// A name for the body at `offset` of the archive at `path` that changes when the archive does:
/// its size, its date, the offset, the limit and the first bytes there.
fn fingerprint(path: &str, offset: u64, limit: u64) -> Result<String, String> {
    let failed = |e: std::io::Error| format!("open({path}): {e}");
    let mut file = fs::File::open(path).map_err(failed)?;
    let meta = file.metadata().map_err(failed)?;
    let mut head = Vec::new();
    file.seek(SeekFrom::Start(offset)).map_err(failed)?;
    (file.take(FINGERPRINT_BYTES as u64).read_to_end(&mut head)).map_err(failed)?;
    let mut hash = Sha1::new();
    hash.update(format!(
        "{} {:?} {offset} {limit} ",
        meta.len(),
        meta.modified().ok()
    ));
    hash.update(&head);
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The corpus file of the container at `offset`: the kept one, else read from the archive,
/// inflated and kept. `fresh` reads the archive whatever is kept.
fn corpus_of(path: &str, offset: u64, limit: u64, fresh: bool) -> Result<PathBuf, String> {
    let kept = cache_dir()?
        .join("rdna-corpus")
        .join(format!("{}.bin", fingerprint(path, offset, limit)?));
    let pairs = super::gta6_rdna_pairs::beside(&kept);
    if !fresh && Corpus::open(&kept).is_ok() && pairs.exists() {
        eprintln!("  corpus        kept at {}", kept.display());
        return Ok(kept);
    }
    let started = Instant::now();
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
        return Err(format!(
            "the body at {offset:#X} is not an SGD5 effect container"
        ));
    }
    let container = Sgd5File::open(&bytes)
        .map_err(|e| format!("the SGD5 container at {offset:#X} does not read: {e}"))?;
    let programs: Vec<Option<Vec<u8>>> = (container.programs.iter())
        .map(|program| {
            (bytes.get(program.stream.clone())).and_then(|stream| program.inflate(stream).ok())
        })
        .collect();
    rdna_corpus::write(&kept, &programs).map_err(|e| format!("write({}): {e}", kept.display()))?;
    fs::write(
        &pairs,
        super::gta6_rdna_pairs::text(&super::gta6_rdna_pairs::of(&container)),
    )
    .map_err(|e| format!("write({}): {e}", pairs.display()))?;
    eprintln!(
        "  corpus        {} program(s) read and inflated in {:.1} s, kept at {}",
        programs.len(),
        started.elapsed().as_secs_f64(),
        kept.display()
    );
    Ok(kept)
}

/// How one stage's blocks came out.
#[derive(Debug, Default, PartialEq, Eq)]
struct Stage {
    blocks: usize,
    /// Those whose control flow follows from the first word to an end, every word an instruction.
    graphs: usize,
    outcomes: BTreeMap<String, usize>,
    /// By outcome and reason, for the outcomes that have one.
    reasons: BTreeMap<(String, String), usize>,
    /// What the translated modules ask of a device, by how many ask it.
    needs: BTreeMap<String, usize>,
}

/// Counts as lines, the commonest first, `top` at most.
fn ranked(out: &mut String, label: &str, counts: &BTreeMap<String, usize>, top: usize) {
    let mut rows: Vec<_> = counts.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (name, count) in rows.iter().take(top) {
        let _ = writeln!(out, "    {label} {name}: {count}");
    }
    if rows.len() > top {
        let _ = writeln!(out, "    and {} other(s)", rows.len() - top);
    }
}

/// What a run is reported with past its records.
#[derive(Debug, Clone, Copy, Default)]
struct Run {
    /// The programs of the container, of which the run may have taken one.
    programs: usize,
    jobs: usize,
    restarts: usize,
    /// The memory each worker was held to, in megabytes.
    worker_mb: u64,
    wall: Duration,
    backends: bool,
}

/// The run as the lines printed: the stages, what stopped a worker, the cache and the back ends.
fn report(records: &[Record], run: Run) -> String {
    let mut out = String::new();
    let count = |outcome: &str| records.iter().filter(|r| r.outcome == outcome).count();
    let _ = writeln!(
        out,
        "  programs      {} in the container, {} not read",
        run.programs,
        count(UNREAD)
    );
    let mut stages: BTreeMap<&str, Stage> = BTreeMap::new();
    for record in records.iter().filter(|r| r.outcome != UNREAD) {
        let stage = stages.entry(&record.stage).or_default();
        stage.blocks += 1;
        stage.graphs += usize::from(record.graph);
        *stage.outcomes.entry(record.outcome.clone()).or_default() += 1;
        for feature in &record.features {
            *stage.needs.entry(feature.clone()).or_default() += 1;
        }
        if let Some(reason) = &record.reason {
            *(stage.reasons)
                .entry((record.outcome.clone(), reason.clone()))
                .or_default() += 1;
        }
    }
    for (key, stage) in &stages {
        let of = |outcome: &str| stage.outcomes.get(outcome).copied().unwrap_or(0);
        let _ = write!(
            out,
            "  {key:<22} {}: {} with a graph, {} translated, {} translated but invalid, {} refused",
            stage.blocks,
            stage.graphs,
            of(TRANSLATED),
            of(INVALID),
            of(REFUSED)
        );
        for outcome in STOPPED {
            if of(outcome) > 0 {
                let _ = write!(out, ", {} {outcome}", of(outcome));
            }
        }
        out.push('\n');
        for outcome in [REFUSED, INVALID, PANICKED] {
            let reasons: BTreeMap<String, usize> = (stage.reasons.iter())
                .filter(|((of, _), _)| of == outcome)
                .map(|((_, reason), count)| (reason.clone(), *count))
                .collect();
            ranked(&mut out, outcome, &reasons, TOP_REASONS);
        }
        // Every one: what a device has to offer is read whole or not at all.
        ranked(&mut out, "needs", &stage.needs, usize::MAX);
    }
    let stopped: Vec<&Record> = (records.iter())
        .filter(|r| STOPPED.contains(&r.outcome.as_str()))
        .collect();
    let _ = writeln!(
        out,
        "  workers       {} at a time for {:.1} s, held to {} MB each; {} started again after a block that ran past its time or that a worker died on; {} panicked, {} timed out, {} crashed, {} ran out of memory",
        run.jobs,
        run.wall.as_secs_f64(),
        run.worker_mb,
        run.restarts,
        count(PANICKED),
        count(TIMED_OUT),
        count(CRASHED),
        count(OUT_OF_MEMORY)
    );
    let mut hungry: Vec<&Record> = records.iter().filter(|r| r.grew_mb > 0).collect();
    hungry.sort_by_key(|r| std::cmp::Reverse(r.grew_mb));
    let _ = writeln!(
        out,
        "  memory        the most a worker committed was {} MB; the blocks that added most to their worker's:",
        records.iter().map(|r| r.peak_mb).max().unwrap_or(0)
    );
    for record in hungry.iter().take(HUNGRY) {
        let _ = writeln!(
            out,
            "    +{} MB at {} ({}, {} words, {})",
            record.grew_mb,
            place(record),
            record.stage,
            record.words,
            record.outcome
        );
    }
    for record in stopped.iter().take(SAMPLE) {
        let _ = writeln!(
            out,
            "    {} {} ({}, {} words) after {} ms{}",
            record.outcome,
            place(record),
            record.stage,
            record.words,
            record.micros / 1000,
            (record.reason.as_ref()).map_or(String::new(), |why| format!(": {why}"))
        );
    }
    if stopped.len() > SAMPLE {
        let _ = writeln!(
            out,
            "    and {} more: --reason with the outcome's name lists them",
            stopped.len() - SAMPLE
        );
    }
    let (mut made, mut loaded, mut checked) = ([0u64; 2], [0u64; 2], 0u64);
    for record in records {
        let side = if record.cached {
            &mut loaded
        } else {
            &mut made
        };
        side[0] += 1;
        side[1] += record.micros;
        checked += record.checked_micros;
    }
    let _ = writeln!(
        out,
        "  translation   {} block(s) translated in {} ms summed, {} taken from the cache or from the same block elsewhere in {} ms summed; validated{} in {} ms summed",
        made[0],
        made[1] / 1000,
        loaded[0],
        loaded[1] / 1000,
        if run.backends { " and written" } else { "" },
        checked / 1000
    );
    let mut approximations: BTreeMap<String, usize> = BTreeMap::new();
    for record in records {
        for name in record.approximations.keys() {
            *approximations.entry(name.clone()).or_default() += 1;
        }
    }
    ranked(&mut out, "approximates", &approximations, TOP_REASONS);
    if run.backends {
        let _ = writeln!(
            out,
            "  back ends     of the {} block(s) translated to a valid module, those a writer refuses, panics on or was not run on:",
            count(TRANSLATED)
        );
        for backend in ORDER {
            let reasons = (records.iter())
                .filter_map(|r| r.backends.get(backend))
                .fold(BTreeMap::new(), |mut reasons, why| {
                    *reasons.entry(why.clone()).or_default() += 1;
                    reasons
                });
            let _ = writeln!(out, "  {backend:<22} {}", reasons.values().sum::<usize>());
            ranked(&mut out, "refuses", &reasons, TOP_REASONS);
        }
    }
    out
}

/// Whether `record` is of a stage that holds `stage` and came out with `reason` in its outcome,
/// its reason, its detail or a back end's refusal.
fn matches(record: &Record, stage: Option<&str>, reason: Option<&str>) -> bool {
    stage.is_none_or(|stage| record.stage.contains(stage))
        && reason.is_none_or(|reason| {
            record.outcome.contains(reason)
                || [&record.reason, &record.detail]
                    .into_iter()
                    .flatten()
                    .any(|text| text.contains(reason))
                || (record.backends.iter())
                    .any(|(backend, why)| backend.contains(reason) || why.contains(reason))
        })
}

/// The blocks that match, `sample` of them a line each, as the lines printed.
fn listed(records: &[Record], stage: Option<&str>, reason: Option<&str>, sample: usize) -> String {
    let found: Vec<&Record> = (records.iter())
        .filter(|record| matches(record, stage, reason))
        .collect();
    let mut out = format!(
        "  matching      {} block(s), {} listed\n",
        found.len(),
        found.len().min(sample)
    );
    for record in found.iter().take(sample) {
        let _ = writeln!(
            out,
            "    {:<8} {:<20} {} words, {}{}",
            place(record),
            record.stage,
            record.words,
            record.outcome,
            (record.detail.as_ref().or(record.reason.as_ref()))
                .map_or(String::new(), |why| format!(": {why}"))
        );
        for (backend, why) in &record.backends {
            let _ = writeln!(out, "             {backend} refuses it: {why}");
        }
    }
    out
}

/// Program `index`: each block's outcome, its instructions, and its WGSL where it translated.
fn show(corpus: &Path, index: usize, records: &[Record]) -> Result<(), String> {
    let dxbc = Corpus::open(corpus)
        .and_then(|mut corpus| corpus.program(index))
        .map_err(|e| format!("{}: {e}", corpus.display()))?
        .ok_or_else(|| format!("program {index} did not read, or the container has fewer"))?;
    let blocks = catch_unwind(|| rdna::unwrap(&dxbc))
        .map_err(|_| format!("program {index}'s container panics the reader"))?
        .map_err(|e| format!("program {index}: {e}"))?;
    for (block, program) in blocks.iter().enumerate() {
        let record = (records.iter()).find(|r| (r.index, r.block) == (index, block));
        let line = match record {
            None => "no outcome".to_owned(),
            Some(r) if r.outcome == TRANSLATED => format!(
                "translated: {} resource(s), {} approximation(s)",
                r.resources,
                r.approximations.values().sum::<u32>()
            ),
            Some(r) => format!(
                "{}{}",
                r.outcome,
                (r.detail.as_ref().or(r.reason.as_ref()))
                    .map_or(String::new(), |why| format!(": {why}"))
            ),
        };
        println!(
            "program {index}, {}, {} words, {} lanes: {line}",
            super::gta6_rdna_run::stage_of(program),
            program.words.len(),
            if program.wave32 { 32 } else { 64 }
        );
        // Follows no control flow: data after the last instruction prints as what its words decode to.
        let decoded = catch_unwind(|| rdna::decode(&program.words))
            .map_err(|_| format!("program {index}'s words panic the decoder"))?;
        for (pc, inst) in decoded.map_err(|e| e.to_string())? {
            println!("  {pc:6}  {inst}");
        }
        if let Some(wgsl) = record.and_then(|r| r.wgsl.as_ref()) {
            println!("program {index}, block {block}, lowered:\n{wgsl}");
        }
    }
    Ok(())
}

const USAGE: &str = "usage: rpf scan-rdna <archive.rpf> <offset> [--program <n>] [--json <file>] \
    [--stage <text>] [--reason <text>] [--sample <n>] [--backends] [--cache] [--fresh] \
    [--jobs <n>] [--worker-memory <MB>] [--memory <MB>] [--batch <n>] [--budget <ms>] \
    [--limit <bytes>]";

/// `rpf scan-rdna`: the programs of the `SGD5` container at an offset translated by workers, a
/// range each, and reported; `--program` takes one program and prints it.
pub(crate) fn cmd_scan_rdna(args: &[String]) -> Result<(), String> {
    let (limit, mut args) = super::rpf::split_limit(args)?;
    let program = take_number(&mut args, "--program")?;
    let json = take_value(&mut args, "--json")?;
    let stage = take_value(&mut args, "--stage")?;
    let reason = take_value(&mut args, "--reason")?;
    let sample = take_number(&mut args, "--sample")?;
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let worker_mb =
        take_number(&mut args, "--worker-memory")?.map_or(limits::WORKER_MB, |mb| mb as u64);
    let jobs = limits::workers(
        take_number(&mut args, "--jobs")?,
        threads,
        take_number(&mut args, "--memory")?.map(|mb| mb as u64),
        worker_mb,
        limits::physical_mb(),
    );
    let batch = take_number(&mut args, "--batch")?.unwrap_or(BATCH);
    let budget = take_number(&mut args, "--budget")?.map_or(BUDGET_MS, |ms| ms as u64);
    let want = Want {
        backends: take_flag(&mut args, "--backends"),
        wgsl: program.is_some(),
    };
    let (cached, fresh) = (
        take_flag(&mut args, "--cache"),
        take_flag(&mut args, "--fresh"),
    );
    let [path, offset] = args.as_slice() else {
        return Err(USAGE.to_owned());
    };
    let offset = super::rpf::parse_offset(offset)?;
    let corpus = corpus_of(path, offset, limit, fresh)?;
    let programs = Corpus::open(&corpus)
        .map_err(|e| format!("{}: {e}", corpus.display()))?
        .len();
    let translations = cached
        .then(|| cache_dir().map(|dir| dir.join("rdna-translations")))
        .transpose()?;
    // Scratch of this run: without it the scan still runs, and translates every copy of a block.
    let memo = std::env::temp_dir().join(format!("openrage-rdna-{}", std::process::id()));
    let plan = Plan {
        corpus: &corpus,
        budget: Duration::from_millis(budget),
        want,
        cache: translations.as_deref(),
        memo: fs::create_dir_all(&memo).is_ok().then_some(memo.as_path()),
        memory_mb: worker_mb,
    };
    let range = match program {
        Some(index) => index.min(programs)..(index + 1).min(programs),
        None => 0..programs,
    };
    let started = Instant::now();
    let (records, restarts) = scan(
        range,
        jobs,
        batch,
        // With the corpus gone every worker dies at once: none is started, and the run is refused below.
        |from, end, heard| {
            if corpus.exists() {
                plan.worker(from, end, heard);
            }
        },
        |finished| eprint!("\r  scanned       {finished} of {programs}"),
    );
    eprintln!();
    let _ = fs::remove_dir_all(&memo);
    if !corpus.exists() {
        return Err(format!(
            "{} was removed while the scan ran: nothing it counted stands",
            corpus.display()
        ));
    }
    let run = Run {
        programs,
        jobs,
        restarts,
        worker_mb,
        wall: started.elapsed(),
        backends: want.backends,
    };
    if let Some(json) = &json {
        write_records(json, &records)?;
    }
    if let Some(index) = program {
        show(&corpus, index, &records)?;
    }
    if stage.is_some() || reason.is_some() || sample.is_some() {
        print!(
            "{}",
            listed(
                &records,
                stage.as_deref(),
                reason.as_deref(),
                sample.unwrap_or(SAMPLE)
            )
        );
    }
    print!("{}", report(&records, run));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gta6_rdna_backends::{HLSL, MSL};

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    fn record(index: usize, stage: &str, outcome: &str, reason: Option<&str>) -> Record {
        Record {
            index,
            stage: stage.to_owned(),
            outcome: outcome.to_owned(),
            graph: reason != Some("graph"),
            reason: reason.map(str::to_owned),
            words: 10,
            micros: 2_000,
            ..Record::default()
        }
    }

    fn sample() -> Vec<Record> {
        let mut portable = record(0, "pixel (slot 0)", TRANSLATED, None);
        portable.features = vec!["subgroups".to_owned()];
        let mut unportable = record(1, "pixel (slot 0)", TRANSLATED, None);
        unportable.cached = true;
        (unportable.backends).insert(MSL.to_owned(), "a made-up refusal".to_owned());
        vec![
            portable,
            unportable,
            record(2, "pixel (slot 0)", REFUSED, Some("graph")),
            record(3, "pixel (slot 0)", REFUSED, Some("made-up opcode")),
            record(4, "vertex (slot 1)", INVALID, Some("a made-up fault")),
            record(5, "vertex (slot 1)", CRASHED, Some("the worker died on it")),
            record(6, "vertex (slot 1)", TIMED_OUT, Some("over 10000 ms")),
            record(7, "unread", UNREAD, None),
            Record {
                peak_mb: 300,
                grew_mb: 250,
                ..record(8, "vertex (slot 1)", OUT_OF_MEMORY, None)
            },
        ]
    }

    #[test]
    fn flags_are_taken_out_of_the_words() {
        let mut args = words(&["a.rpf", "--program", "7", "--backends", "0x10"]);
        assert_eq!(take_number(&mut args, "--program"), Ok(Some(7)));
        assert!(take_flag(&mut args, "--backends"));
        assert!(!take_flag(&mut args, "--cache"));
        assert_eq!(take_value(&mut args, "--json"), Ok(None));
        assert_eq!(args, words(&["a.rpf", "0x10"]));
        assert!(take_value(&mut words(&["--program"]), "--program").is_err());
        assert!(take_number(&mut words(&["--program", "x"]), "--program").is_err());
    }

    #[test]
    fn a_report_counts_every_outcome_by_stage_and_names_what_stopped_a_worker() {
        let run = Run {
            programs: 9,
            jobs: 4,
            restarts: 2,
            worker_mb: 512,
            wall: Duration::from_millis(1500),
            backends: true,
        };
        let text = report(&sample(), run);
        let has = |line: &str| assert!(text.contains(line), "no `{line}` in:\n{text}");
        has("  programs      9 in the container, 1 not read\n");
        has("  pixel (slot 0)         4: 3 with a graph, 2 translated, 0 translated but invalid, 2 refused\n");
        has("    refused graph: 1\n");
        has("  vertex (slot 1)        4: 4 with a graph, 0 translated, 1 translated but invalid, 0 refused, 1 timed out, 1 crashed, 1 ran out of memory\n");
        has("    invalid a made-up fault: 1\n");
        has("4 at a time for 1.5 s, held to 512 MB each; 2 started again");
        has("0 panicked, 1 timed out, 1 crashed, 1 ran out of memory\n");
        has("  memory        the most a worker committed was 300 MB;");
        has("    +250 MB at 8 (vertex (slot 1), 10 words, ran out of memory)\n");
        has("    crashed 5 (vertex (slot 1), 10 words) after 2 ms: the worker died on it\n");
        has("    timed out 6 (vertex (slot 1), 10 words) after 2 ms: over 10000 ms\n");
        has("8 block(s) translated in 16 ms summed, 1 taken from the cache or from the same block elsewhere in 2 ms summed; validated and written in 0 ms summed\n");
        has("    needs subgroups: 1\n");
        has(&format!(
            "  {MSL:<22} 1\n    refuses a made-up refusal: 1\n"
        ));
        has(&format!("  {HLSL:<22} 0\n"));
        assert!(!report(&sample(), Run::default()).contains("back ends"));
    }

    #[test]
    fn a_class_of_blocks_is_listed_by_stage_and_reason() {
        let records = sample();
        let places = |stage: Option<&str>, reason: Option<&str>| -> Vec<usize> {
            (records.iter())
                .filter(|r| matches(r, stage, reason))
                .map(|r| r.index)
                .collect()
        };
        assert_eq!(places(Some("vertex"), None), [4, 5, 6, 8]);
        assert_eq!(places(None, Some("made-up opcode")), [3]);
        assert_eq!(places(None, Some("crashed")), [5]);
        assert_eq!(places(Some("pixel"), Some("refused")), [2, 3]);
        // A back end's refusal is a reason too, by the back end's name or by what it said.
        assert_eq!(places(None, Some("MSL")), [1]);
        assert_eq!(places(None, Some("made-up refusal")), [1]);
        let text = listed(&records, Some("pixel"), Some("refused"), 1);
        assert!(text.starts_with("  matching      2 block(s), 1 listed\n"));
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("    2        pixel (slot 0)       10 words, refused: graph\n"));
    }

    #[test]
    fn a_corpus_is_named_by_the_archive_and_the_place_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("made-up.rpf");
        let name = |offset| fingerprint(&path.to_string_lossy(), offset, 100).unwrap();
        fs::write(&path, [1u8; 300]).unwrap();
        let first = name(16);
        assert_eq!(first, name(16));
        assert_ne!(first, name(32));
        fs::write(&path, [2u8; 300]).unwrap();
        assert_ne!(first, name(16));
        assert!(fingerprint(&dir.path().join("absent").to_string_lossy(), 0, 0).is_err());
    }
}
