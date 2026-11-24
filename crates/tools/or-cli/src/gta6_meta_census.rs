//! `rpf scan-census`: the metadata rows of a GTA VI archive, or with `--kind` those of a kind, a
//! family or all, but those `--not` names, through `rpf scan-open`, each in a process of its own, tallied by what was read,
//! by refusal and by save line; nothing is written.

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use or_io::container::rpf::{Rpf9BodyKind, Rpf9Content, Rpf9Family, Rpf9Scan, Rpf9ScanFind};
use or_job::Step;

const USAGE: &str =
    "usage: rpf scan-census <archive.rpf> [--from <offset>] [--to <offset>] [--every <n>] [--kind <kind>|all] [--not <kind>[,<kind>...]] [--sample <n>] [--jobs <n>] [--limit <bytes>]";

/// What `scan-open` says when a reader of metadata, not the container, refuses a body.
const READER_REFUSALS: [&str; 4] = [
    "the PSO reader does not read this body",
    "the RBF reader does not read this body",
    "GTA VI's RBM reader does not read this body's two sections",
    "the metadata reader does not read this body",
];

/// The counts of one run, each message with its numbers as N.
#[derive(Default)]
struct Tally {
    /// Per row: read, refused.
    rows: BTreeMap<String, (u64, u64)>,
    /// Per refusal: how many, the first offset that gave it and that body's row.
    refusals: BTreeMap<String, (u64, u64, String)>,
    saves: BTreeMap<String, u64>,
    /// Per root type a metadata document states: how many, and the first offset that gave it.
    roots: BTreeMap<String, (u64, u64)>,
    by_readers: u64,
}

pub(crate) fn cmd_scan_census(args: &[String]) -> Result<(), String> {
    let (limit, args) = super::rpf::split_limit(args)?;
    let (from, args) = super::rpf::split_offset_flag(&args, "--from")?;
    let (to, args) = super::rpf::split_offset_flag(&args, "--to")?;
    let (every, args) = super::rpf::split_offset_flag(&args, "--every")?;
    let (sample, args) = super::rpf::split_offset_flag(&args, "--sample")?;
    let (jobs, args) = super::rpf::split_offset_flag(&args, "--jobs")?;
    let (kind, args) = super::rpf::split_value(&args, "--kind")?;
    let (not, args) = super::rpf::split_value(&args, "--not")?;
    let [path] = args.as_slice() else {
        return Err(USAGE.to_string());
    };
    let every = every.unwrap_or(1).max(1);
    let jobs = jobs.unwrap_or(1).clamp(1, 64) as usize;
    let exe = std::env::current_exe().map_err(|e| format!("where this program is: {e}"))?;
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let cancel = super::jobs::on_interrupt();
    let mut rows = Vec::new();
    let mut seen = 0u64;
    let range = from.unwrap_or(0)..to.unwrap_or(u64::MAX);
    for find in Rpf9Scan::new(&file, range).map_err(|e| e.to_string())? {
        if cancel.is_set() {
            return Err("cancelled".to_string());
        }
        let find = find.map_err(|e| e.to_string())?;
        let row = super::rpf::scan_row(&find);
        if !admits(kind.as_deref(), not.as_deref(), &find, &row) {
            continue;
        }
        seen += 1;
        if (seen - 1).is_multiple_of(every) {
            rows.push((find.offset, row));
        }
    }
    let mut found: BTreeMap<String, u64> = BTreeMap::new();
    for (_, row) in &rows {
        *found.entry(row.clone()).or_default() += 1;
    }
    if let Some(sample) = sample {
        rows = evenly_spaced(rows, &found, sample.max(1));
    }
    println!(
        "  census        {} of {seen} {} rows (every {every}{}), each through `rpf scan-open` in a process of its own, {jobs} at a time; nothing is written",
        rows.len(),
        kind.as_deref().unwrap_or("metadata"),
        sample.map_or(String::new(), |n| format!(
            ", at most {n} of a row, evenly spaced"
        )),
    );
    for (row, count) in &found {
        println!("  found         {row}: {count}");
    }
    let progress = or_job::Progress::new();
    let mut ticker = super::jobs::Ticker::new(&progress);
    let mut tally = Tally::default();
    let limit = limit.to_string();
    let open = |offset: u64| {
        Command::new(&exe)
            .args(["rpf", "scan-open", path.as_str(), &format!("{offset:#X}")])
            .args(["--limit", limit.as_str()])
            .output()
    };
    let batches: Vec<&[(u64, String)]> = rows.chunks(jobs).collect();
    let report = or_job::run(batches, &cancel, &progress, |batch| {
        ticker.tick();
        let outs: Vec<_> = std::thread::scope(|scope| {
            let running: Vec<_> = (batch.iter())
                .map(|(offset, _)| scope.spawn(|| open(*offset)))
                .collect();
            running.into_iter().map(|run| run.join()).collect()
        });
        let mut failed = None;
        for ((offset, row), out) in batch.iter().zip(outs) {
            match out {
                Ok(Ok(out)) => {
                    let read = out.status.success();
                    tally.add(*offset, row.clone(), read, &out.stdout, &out.stderr);
                }
                Ok(Err(e)) => failed = Some(format!("{offset:#X}: scan-open did not start: {e}")),
                Err(_) => failed = Some(format!("{offset:#X}: the thread that ran scan-open died")),
            }
        }
        failed.map_or(Step::Done(0), Step::Failed)
    });
    ticker.finish(&report);
    tally.print();
    match (report.cancelled, report.failures.first(), tally.by_readers) {
        (true, _, _) => Err("cancelled".to_string()),
        (false, Some(_), _) => Err(report.failures.join("; ")),
        (false, None, 0) => Ok(()),
        (false, None, n) => Err(format!("{n} bodies were refused by a reader of metadata")),
    }
}

/// Whether a census of `kind`, but not of the kinds `not` lists with commas between them, opens
/// `find`, whose row is `row`: a kind is a row as `rpf scan` tallies it, a family as
/// `rpf list --kind` names it, or `all`.
fn admits(kind: Option<&str>, not: Option<&str>, find: &Rpf9ScanFind, row: &str) -> bool {
    let is = |k: &str| k == row || Rpf9Family::parse(k) == Some(Rpf9Family::of(find));
    let asked = match kind {
        None => is_metadata_row(&find.kind),
        Some("all") => true,
        Some(k) => is(k),
    };
    asked && !not.is_some_and(|kinds| kinds.split(',').any(is))
}

/// At most `sample` rows of each row label, evenly spaced among that label's, in the order found.
fn evenly_spaced(
    rows: Vec<(u64, String)>,
    found: &BTreeMap<String, u64>,
    sample: u64,
) -> Vec<(u64, String)> {
    let mut nth: BTreeMap<String, u64> = BTreeMap::new();
    let mut taken: BTreeMap<String, u64> = BTreeMap::new();
    rows.into_iter()
        .filter(|(_, row)| {
            let of = found[row];
            let i = nth.entry(row.clone()).or_default();
            let k = taken.entry(row.clone()).or_default();
            // The k-th of `sample` stands at index floor((k + 0.5) * of / sample).
            let keep = of <= sample || (*k < sample && *i == (2 * *k + 1) * of / (2 * sample));
            *i += 1;
            *k += u64::from(keep);
            keep
        })
        .collect()
}

/// Whether a find is a row a reader of metadata may be handed.
fn is_metadata_row(kind: &Rpf9BodyKind) -> bool {
    matches!(kind, Rpf9BodyKind::RawDeflate { .. })
        || matches!(
            kind,
            Rpf9BodyKind::Zlib(run) if matches!(
                run.content,
                Rpf9Content::Pso
                    | Rpf9Content::Rbf
                    | Rpf9Content::Xml
                    | Rpf9Content::ResourceShaped
                    | Rpf9Content::TextTable
                    | Rpf9Content::Other
            )
        )
}

impl Tally {
    fn add(&mut self, offset: u64, row: String, read: bool, stdout: &[u8], stderr: &[u8]) {
        let (stdout, stderr) = (
            String::from_utf8_lossy(stdout),
            String::from_utf8_lossy(stderr),
        );
        let counts = self.rows.entry(row.clone()).or_default();
        let mut refusal = None;
        match read {
            true => counts.0 += 1,
            false => {
                counts.1 += 1;
                refusal = (stderr.lines().rev())
                    .find_map(|l| l.strip_prefix("error: "))
                    .map(str::to_string);
            }
        }
        // A map body's typed reader can read a body the tree reader refused.
        let tree = (stdout.lines()).find_map(|l| {
            (l.trim_start().strip_prefix("tree "))
                .map(str::trim)
                .filter(|l| READER_REFUSALS.iter().any(|r| l.starts_with(r)))
        });
        if let Some(why) = refusal.as_deref().or(tree) {
            if READER_REFUSALS.iter().any(|r| why.starts_with(r)) {
                self.by_readers += 1;
            }
            let cause = self
                .refusals
                .entry(numbers_as_n(why))
                .or_insert((0, offset, row));
            cause.0 += 1;
        } else if !read {
            self.refusals
                .entry("refused, with no message".to_string())
                .or_insert((0, offset, row))
                .0 += 1;
        }
        if let Some(root) = (stdout.lines()).find_map(|l| l.trim_start().strip_prefix("root type "))
        {
            self.roots
                .entry(root.trim().to_string())
                .or_insert((0, offset))
                .0 += 1;
        }
        if let Some(save) = (stdout.lines()).find_map(|l| l.trim_start().strip_prefix("save ")) {
            *self.saves.entry(numbers_as_n(save.trim())).or_default() += 1;
        }
    }

    fn print(&self) {
        for (row, (read, refused)) in &self.rows {
            println!("  row           {row}: {read} read, {refused} refused");
        }
        let mut ranked: Vec<_> = self.refusals.iter().collect();
        ranked.sort_by_key(|(_, (count, ..))| std::cmp::Reverse(*count));
        for (cause, (count, first, row)) in ranked {
            println!("  refused       {count}  {cause}  (the first at {first:#X}, a {row})");
        }
        let mut roots: Vec<_> = self.roots.iter().collect();
        roots.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
        for (root, (count, first)) in roots {
            println!("  root          {count}  {root}  (the first at {first:#X})");
        }
        for (save, count) in &self.saves {
            println!("  save          {count}  {save}");
        }
        println!(
            "  readers       {} refusals by a reader of metadata",
            self.by_readers
        );
    }
}

/// `text` with each number, decimal, `0x` hexadecimal or bare hexadecimal digits, written as N.
fn numbers_as_n(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if !c.is_ascii_digit() {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let len = rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(rest.len());
        let word = &rest[..len];
        let hex = word.strip_prefix("0x").unwrap_or(word);
        match !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            true => out.push('N'),
            false => out.push_str(word),
        }
        rest = &rest[len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_census_opens_the_kind_asked_for_but_not_the_kind_it_is_told_to_leave() {
        let find = |kind| Rpf9ScanFind {
            offset: 0x1000,
            kind,
            span: 16,
            stored_len: Some(16),
            inflated_len: Some(16),
            container: None,
        };
        let entry = find(Rpf9BodyKind::FirstEntry { head: *b"FORC" });
        let rbf = find(Rpf9BodyKind::RawDeflate {
            content: Rpf9Content::Rbf,
        });
        // With no kind a census is of metadata rows; `all` takes every row.
        assert!(admits(None, None, &rbf, "raw deflate rbf"));
        assert!(!admits(None, None, &entry, "first entry"));
        assert!(admits(Some("all"), None, &entry, "first entry"));
        // A row or a family is left out by name, and only it.
        assert!(!admits(
            Some("all"),
            Some("archives"),
            &entry,
            "first entry"
        ));
        assert!(!admits(
            Some("all"),
            Some("first entry"),
            &entry,
            "first entry"
        ));
        assert!(admits(
            Some("all"),
            Some("archives"),
            &rbf,
            "raw deflate rbf"
        ));
        // Several kinds are left out at once.
        assert!(!admits(
            Some("all"),
            Some("animations,archives"),
            &entry,
            "first entry"
        ));
        assert!(!admits(
            Some("all"),
            Some("archives,raw deflate rbf"),
            &rbf,
            "raw deflate rbf"
        ));
        assert!(admits(
            Some("all"),
            Some("animations,text"),
            &rbf,
            "raw deflate rbf"
        ));
        assert!(admits(
            Some("raw deflate rbf"),
            None,
            &rbf,
            "raw deflate rbf"
        ));
        assert!(!admits(
            Some("raw deflate rbf"),
            None,
            &entry,
            "first entry"
        ));
    }

    #[test]
    fn a_sample_takes_each_row_evenly_and_a_small_row_whole() {
        let mut rows: Vec<(u64, String)> = (0..10).map(|i| (i, "many".to_string())).collect();
        rows.insert(3, (100, "few".to_string()));
        let found = BTreeMap::from([("many".to_string(), 10), ("few".to_string(), 1)]);
        let kept: Vec<u64> = (evenly_spaced(rows, &found, 2).iter())
            .map(|(offset, _)| *offset)
            .collect();
        assert_eq!(kept, [2, 100, 7]);
    }

    #[test]
    fn numbers_become_n_and_words_that_start_with_a_digit_stay() {
        assert_eq!(
            numbers_as_n("block 12 (0x1AE00000) differs from byte 0x1F0"),
            "block N (N) differs from byte N"
        );
        assert_eq!(numbers_as_n("has no 0DRP at 0x10"), "has no 0DRP at N");
    }

    #[test]
    fn a_reader_refusal_is_told_from_the_containers_and_saves_are_counted() {
        let mut tally = Tally::default();
        let container = b"error: the zlib chunk at 0x40 fails partway through\n";
        tally.add(0x100, "zlib other".into(), false, b"", container);
        let reader = b"error: the PSO reader does not read this body: block 3 is short\n";
        tally.add(0x200, "zlib pso".into(), false, b"", reader);
        let read = b"  root type    MetaPed\n  save         writes back byte-identical; checksum 0x12345678 kept\n";
        tally.add(0x300, "zlib pso".into(), true, read, b"");
        let tree =
            b"  tree          GTA VI's RBM reader does not read this body's two sections: x\n";
        tally.add(0x400, "headerless map".into(), true, tree, b"");

        assert_eq!(tally.by_readers, 2);
        assert_eq!(tally.roots["MetaPed"], (1, 0x300));
        assert_eq!(tally.rows["zlib pso"], (1, 1));
        assert_eq!(
            tally.refusals["the zlib chunk at N fails partway through"],
            (1, 0x100, "zlib other".to_string())
        );
        assert_eq!(
            tally.refusals["the PSO reader does not read this body: block N is short"],
            (1, 0x200, "zlib pso".to_string())
        );
        assert_eq!(
            tally.saves["writes back byte-identical; checksum N kept"],
            1
        );

        // A raw DEFLATE stream is a metadata row, and its reader's refusal is a reader's.
        let raw = Rpf9BodyKind::RawDeflate {
            content: Rpf9Content::Rbf,
        };
        assert!(is_metadata_row(&raw));
        let refused = b"error: the metadata reader does not read this body: short\n";
        tally.add(0x500, "raw deflate rbf".into(), false, b"", refused);
        assert_eq!(tally.by_readers, 3);
    }
}
