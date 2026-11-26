//! `rpf scan-rdna-diff`: what changed between two `rpf scan-rdna --json` runs, block by block: the
//! outcomes that moved, the reasons that grew or shrank in each stage, and the translations that
//! got slower. A change to the translator is judged by this.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write as _};

use super::gta6_rdna_run::{Record, TRANSLATED};

/// The blocks of a class listed at most.
const LISTED: usize = 20;
/// The reason shifts printed at most, the largest first.
const SHIFTS: usize = 40;
/// A translation counts as slower at twice the time and this many microseconds more.
const SLOWER_BY: u64 = 2_000;

/// `records` to `path`, a JSON object a line.
pub(crate) fn write_records(path: &str, records: &[Record]) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("write({path}): {e}");
    let mut out = BufWriter::new(fs::File::create(path).map_err(failed)?);
    for record in records {
        let line = serde_json::to_string(record).map_err(|e| format!("write({path}): {e}"))?;
        writeln!(out, "{line}").map_err(failed)?;
    }
    out.flush().map_err(failed)
}

/// The records `--json` wrote to `path`.
pub(crate) fn read_records(path: &str) -> Result<Vec<Record>, String> {
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    (BufReader::new(file).lines().enumerate())
        .filter(|(_, line)| !matches!(line, Ok(line) if line.trim().is_empty()))
        .map(|(number, line)| {
            let line = line.map_err(|e| format!("read({path}): {e}"))?;
            serde_json::from_str(&line)
                .map_err(|e| format!("{path}, line {}: not a scan record: {e}", number + 1))
        })
        .collect()
}

/// A block as a word: its program's index, and its own place after a colon past the first.
pub(crate) fn place(record: &Record) -> String {
    match record.block {
        0 => record.index.to_string(),
        block => format!("{}:{block}", record.index),
    }
}

/// Counts by key, each as before and after.
type Shifts<K> = BTreeMap<K, [usize; 2]>;

fn signed(before: usize, after: usize) -> String {
    format!("{:+}", after as i64 - before as i64)
}

/// What changed from run `a` to run `b`, as the lines printed.
pub(crate) fn diff(a: &[Record], b: &[Record]) -> String {
    let before: BTreeMap<(usize, usize), &Record> =
        a.iter().map(|r| ((r.index, r.block), r)).collect();
    let mut out = String::new();
    let mut outcomes: Shifts<&str> = BTreeMap::new();
    let mut reasons: Shifts<(&str, &str, &str)> = BTreeMap::new();
    for (side, records) in [a, b].into_iter().enumerate() {
        for record in records {
            outcomes.entry(&record.outcome).or_default()[side] += 1;
            if record.outcome != TRANSLATED {
                let reason = record.reason.as_deref().unwrap_or("");
                reasons
                    .entry((&record.stage, &record.outcome, reason))
                    .or_default()[side] += 1;
            }
        }
    }
    let mut moved: BTreeMap<&str, Vec<&Record>> = BTreeMap::new();
    let mut slower: Vec<(&Record, &Record)> = Vec::new();
    let (mut both, mut rekeyed, mut time) = (0usize, 0usize, [0u64; 2]);
    for after in b {
        let Some(&earlier) = before.get(&(after.index, after.block)) else {
            continue;
        };
        both += 1;
        // The key holds the translator too, so only a block of another stage is another program.
        rekeyed += usize::from(earlier.stage != after.stage);
        if earlier.outcome != after.outcome {
            moved.entry(&after.outcome).or_default().push(after);
        }
        if !earlier.cached && !after.cached {
            time[0] += earlier.micros;
            time[1] += after.micros;
            if after.micros >= 2 * earlier.micros && after.micros >= earlier.micros + SLOWER_BY {
                slower.push((earlier, after));
            }
        }
    }
    let _ = writeln!(
        out,
        "  blocks        {} before, {} after, {both} in both",
        a.len(),
        b.len()
    );
    if rekeyed > 0 {
        let _ = writeln!(
            out,
            "  ⚠ {rekeyed} block(s) are of another stage in the two runs: these are not two runs of one container"
        );
    }
    for (outcome, [was, is]) in &outcomes {
        let _ = writeln!(out, "  {outcome:<13} {was} -> {is} ({})", signed(*was, *is));
    }
    for (outcome, blocks) in &moved {
        let listed: Vec<String> = blocks.iter().take(LISTED).map(|r| place(r)).collect();
        let more = match blocks.len().saturating_sub(LISTED) {
            0 => String::new(),
            more => format!(" and {more} more"),
        };
        let _ = writeln!(
            out,
            "  newly {outcome}: {}, at {}{more}",
            blocks.len(),
            listed.join(" ")
        );
    }
    if moved.is_empty() {
        let _ = writeln!(out, "  no block's outcome changed");
    }
    let mut shifts: Vec<_> = reasons.iter().filter(|(_, [was, is])| was != is).collect();
    shifts.sort_by_key(|(key, [was, is])| (std::cmp::Reverse(was.abs_diff(*is)), **key));
    for ((stage, outcome, reason), [was, is]) in shifts.iter().take(SHIFTS) {
        let _ = writeln!(
            out,
            "  {stage:<22} {outcome} {reason}: {was} -> {is} ({})",
            signed(*was, *is)
        );
    }
    if shifts.len() > SHIFTS {
        let _ = writeln!(
            out,
            "  and {} other reason(s) shifted",
            shifts.len() - SHIFTS
        );
    }
    let _ = writeln!(
        out,
        "  time          {} ms -> {} ms summed over the blocks neither run took from a cache; {} slower",
        time[0] / 1000,
        time[1] / 1000,
        slower.len()
    );
    slower.sort_by_key(|(was, is)| std::cmp::Reverse(is.micros - was.micros));
    for (was, is) in slower.iter().take(LISTED) {
        let _ = writeln!(
            out,
            "    slower {} ({}): {} us -> {} us",
            place(is),
            is.stage,
            was.micros,
            is.micros
        );
    }
    out
}

/// `rpf scan-rdna-diff <before.json> <after.json>`.
pub(crate) fn cmd_scan_rdna_diff(args: &[String]) -> Result<(), String> {
    let [before, after] = args else {
        return Err("usage: rpf scan-rdna-diff <before.json> <after.json>".to_owned());
    };
    print!("{}", diff(&read_records(before)?, &read_records(after)?));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gta6_rdna_run::{CRASHED, INVALID, REFUSED};

    fn record(index: usize, outcome: &str, reason: Option<&str>, micros: u64) -> Record {
        Record {
            index,
            stage: "pixel (slot 0)".to_owned(),
            outcome: outcome.to_owned(),
            reason: reason.map(str::to_owned),
            micros,
            ..Record::default()
        }
    }

    #[test]
    fn a_diff_names_what_moved_and_what_got_slower() {
        let before = [
            record(0, REFUSED, Some("made-up opcode"), 100),
            record(1, REFUSED, Some("made-up opcode"), 100),
            record(2, TRANSLATED, None, 500),
            record(3, TRANSLATED, None, 500),
            record(4, CRASHED, Some("the worker died on it"), 0),
            record(5, TRANSLATED, None, 400),
            record(6, REFUSED, Some("steady"), 10),
        ];
        let mut second_block = record(3, INVALID, Some("a made-up fault"), 500);
        second_block.block = 1;
        let after = [
            record(0, TRANSLATED, None, 100),
            record(1, REFUSED, Some("another form"), 100),
            record(2, INVALID, Some("a made-up fault"), 500),
            record(3, TRANSLATED, None, 9_000),
            second_block,
            record(4, REFUSED, Some("made-up opcode"), 50),
            record(5, TRANSLATED, None, 700),
            record(6, REFUSED, Some("steady"), 10),
        ];
        let text = diff(&before, &after);
        let has = |line: &str| assert!(text.contains(line), "no `{line}` in:\n{text}");
        has("  blocks        7 before, 8 after, 7 in both\n");
        has("  translated    3 -> 3 (+0)\n");
        has("  refused       3 -> 3 (+0)\n");
        has("pixel (slot 0)         refused made-up opcode: 2 -> 1 (-1)\n");
        has("  crashed       1 -> 0 (-1)\n");
        has("  newly translated: 1, at 0\n");
        has("  newly invalid: 1, at 2\n");
        has("  newly refused: 1, at 4\n");
        has("pixel (slot 0)         invalid a made-up fault: 0 -> 2 (+2)\n");
        has("pixel (slot 0)         refused another form: 0 -> 1 (+1)\n");
        has("pixel (slot 0)         crashed the worker died on it: 1 -> 0 (-1)\n");
        // Block 5 is slower by too little to name; block 3 is named.
        has("; 1 slower\n");
        has("    slower 3 (pixel (slot 0)): 500 us -> 9000 us\n");
        assert!(
            !text.contains("steady"),
            "an unchanged count is not a shift:\n{text}"
        );
        assert!(diff(&before, &before).contains("no block's outcome changed"));
    }

    #[test]
    fn records_come_back_from_the_file_they_were_written_to() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.json").to_string_lossy().into_owned();
        let mut records = vec![
            record(0, TRANSLATED, None, 12),
            record(7, REFUSED, Some("made up"), 3),
        ];
        records[0]
            .backends
            .insert("WGSL".to_owned(), "made up".to_owned());
        records[0].approximations.insert("made up".to_owned(), 2);
        records[0].wave = "made up".to_owned();
        records[0].intersections.push(crate::gta6_rdna_run::Site {
            word: 7,
            wide: true,
            function: "made up".to_owned(),
            depth: 3,
            loops: vec![2, 5],
        });
        write_records(&path, &records).unwrap();
        assert_eq!(read_records(&path).unwrap(), records);
        fs::write(&path, "{\"index\": 1}\nnot a record\n").unwrap();
        assert!(read_records(&path).unwrap_err().contains("line 1"));
        assert_eq!(
            (
                place(&records[1]),
                place(&Record {
                    block: 1,
                    ..records[1].clone()
                })
            ),
            ("7".to_owned(), "7:1".to_owned())
        );
    }
}
