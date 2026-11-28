//! `rpf scan-install`: every GTA VI archive under a folder scanned in turn as one cancellable job,
//! each tallied by the kinds `rpf scan` names, then the folder's totals; counts, and with
//! `--sample` a few offsets of each kind that `rpf scan-open` takes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Rpf9Family, Rpf9NamesByCount, Rpf9Scan, Rpf9ScanFind};
use or_job::{Cancel, Step};
use or_title::install::gta6_files::{MOST_ARCHIVES, MOST_FOLDERS};

use crate::project::Failure::{self, Run, Usage};
use crate::rpf::{is_rpf9, scan_row, split_value};

/// The most offsets `--sample` prints of a kind, and the most of a kind an archive keeps.
const MOST_SAMPLE: usize = 10_000;
const MOST_KEPT: usize = 1 << 22;

/// How many bodies of each kind, by the row `rpf scan` tallies them under.
type Tally = BTreeMap<String, u64>;
/// The offsets of the bodies of each kind, in scan order, at most [`MOST_KEPT`] a kind.
type Kept = BTreeMap<String, Vec<u64>>;

pub(crate) fn cmd_scan_install(args: &[String]) -> Result<(), Failure> {
    let (kind, args) = split_value(args, "--kind").map_err(Usage)?;
    let (sample, args) = split_value(&args, "--sample").map_err(Usage)?;
    let sample = match sample {
        None => None,
        Some(n) => Some(
            (n.parse::<usize>().ok())
                .filter(|n| (1..=MOST_SAMPLE).contains(n))
                .ok_or_else(|| Usage(format!("--sample takes 1 to {MOST_SAMPLE}, not {n}")))?,
        ),
    };
    let [folder] = args.as_slice() else {
        return Err(Usage(
            "usage: rpf scan-install <folder> [--kind <kind>] [--sample <n>]".to_string(),
        ));
    };
    let root = Path::new(folder);
    if !root.is_dir() {
        return Err(Usage(format!("{folder} is not a folder")));
    }
    let (archives, cut) = archives_under(root);
    if archives.is_empty() {
        return Err(Run(format!("no GTA VI archive (RPF9) under {folder}")));
    }
    println!("  scan          {}", Rpf9ScanFind::ORIGIN);
    if cut {
        println!(
            "  cut           the walk stops at {MOST_FOLDERS} folders and {MOST_ARCHIVES} archives: those past them are not scanned"
        );
    }
    if let Some(kind) = &kind {
        println!("  kind          {kind}: only the bodies of that kind or family");
    }
    if let Some(n) = sample {
        println!(
            "  sample        at most {n} offsets of each kind an archive holds, evenly spaced"
        );
    }
    let admits = |find: &Rpf9ScanFind| admitted(kind.as_deref(), find);
    let cancel = super::jobs::on_interrupt();
    let progress = or_job::Progress::new();
    let mut ticker = super::jobs::Ticker::new(&progress);
    let mut totals = Tally::new();
    let report = or_job::run(archives, &cancel, &progress, |path| {
        ticker.tick();
        let label = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        progress.now_on(label.clone());
        let (tally, kept, ended) = tally_archive(&path, &cancel, &admits, sample.is_some());
        println!("archive: {label}");
        print_tally(&tally);
        if let Some(n) = sample {
            print_sample(&tally, &kept, n);
        }
        for (row, count) in &tally {
            *totals.entry(row.clone()).or_default() += count;
        }
        match ended {
            Ok(()) => Step::Done(0),
            Err(why) => {
                println!("  stopped       {why}: the counts above are of the bodies before it");
                Step::Failed(format!("{label}: {why}"))
            }
        }
    });
    ticker.finish(&report);
    println!(
        "every archive ({} of {} scanned)",
        report.counts.attempted, report.counts.total
    );
    print_tally(&totals);
    match (report.cancelled, report.failures.first()) {
        (true, _) => Err(Run("cancelled".to_string())),
        (false, Some(_)) => Err(Run(report.failures.join("; "))),
        (false, None) => Ok(()),
    }
}

/// Whether `kind`, a row `rpf scan` tallies or a family `rpf list --kind` names, admits `find`.
fn admitted(kind: Option<&str>, find: &Rpf9ScanFind) -> bool {
    kind.is_none_or(|k| k == scan_row(find) || Rpf9Family::parse(k) == Some(Rpf9Family::of(find)))
}

/// Every archive listed under `root` whose head is RPF9's, in the order listed, and whether the
/// walk stopped at a limit before it found them all.
fn archives_under(root: &Path) -> (Vec<PathBuf>, bool) {
    let listed = or_title::install::gta6_files::list(root);
    let named = listed.archives.into_iter().map(|archive| archive.path);
    let archives = named.filter(|path| is_rpf9(path)).collect();
    (archives, listed.truncated)
}

/// The bodies `admits` takes of the archive at `path`, by row, and their offsets where `keep`;
/// `Err` says why the scan stopped before the archive's end, the tally then being of what it
/// found before.
fn tally_archive(
    path: &Path,
    cancel: &Cancel,
    admits: &dyn Fn(&Rpf9ScanFind) -> bool,
    keep: bool,
) -> (Tally, Kept, Result<(), String>) {
    let (mut tally, mut kept) = (Tally::new(), Kept::new());
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) => return (tally, kept, Err(format!("open: {e}"))),
    };
    let scan = match Rpf9Scan::new(&file, 0..u64::MAX) {
        Ok(scan) => scan,
        Err(e) => return (tally, kept, Err(e.to_string())),
    };
    let mut by_count = Rpf9NamesByCount::default();
    for find in scan {
        if cancel.is_set() {
            return (tally, kept, Err("cancelled".to_string()));
        }
        match find {
            Ok(find) => {
                by_count.push(&find);
                if admits(&find) {
                    count(&mut tally, &mut kept, &find, keep);
                }
            }
            Err(e) => return (tally, kept, Err(e.to_string())),
        }
    }
    // The names placed by counting their header's entries, as `rpf scan` lists them.
    match by_count.place(&file) {
        Ok(placed) => (placed.iter())
            .filter(|names| admits(names))
            .for_each(|names| count(&mut tally, &mut kept, names, keep)),
        Err(e) => return (tally, kept, Err(e.to_string())),
    }
    (tally, kept, Ok(()))
}

/// Count `find` under its row, and keep its offset where `keep`.
fn count(tally: &mut Tally, kept: &mut Kept, find: &Rpf9ScanFind, keep: bool) {
    let row = scan_row(find);
    if keep {
        let offsets = kept.entry(row.clone()).or_default();
        if offsets.len() < MOST_KEPT {
            offsets.push(find.offset);
        }
    }
    *tally.entry(row).or_default() += 1;
}

fn print_tally(tally: &Tally) {
    println!("    {:<24}  {:>9}", "kind", "count");
    for (row, count) in tally {
        println!("    {row:<24}  {count:>9}");
    }
}

/// At most `n` offsets of each kind `kept` holds, evenly spaced, and where a kind held more
/// bodies than were kept, how many the offsets were drawn from.
fn print_sample(tally: &Tally, kept: &Kept, n: usize) {
    println!("    {:<24}  offsets", "sample");
    for (row, offsets) in kept {
        let picked: Vec<String> = (evenly(offsets, n).iter())
            .map(|at| format!("{at:#X}"))
            .collect();
        println!("    {row:<24}  {}", picked.join(" "));
        if tally
            .get(row)
            .is_some_and(|&count| count > offsets.len() as u64)
        {
            println!("    {:<24}  drawn from the first {}", "", offsets.len());
        }
    }
}

/// At most `n` of `all`, evenly spaced from the first.
fn evenly(all: &[u64], n: usize) -> Vec<u64> {
    match all.len() <= n {
        true => all.to_vec(),
        false => (0..n).map(|i| all[i * all.len() / n]).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use or_io::container::rpf::{read_rpf9_body, RPF9_HEADER_SIZE, RPF9_MAGIC};

    use super::*;

    /// Where the table of [`archive_with_a_body`]'s head ends, and so where its body starts.
    const BODY_AT: u32 = 0x1A0;

    /// An RPF9 head of layout 4 of `entries` entries and 0x40 bytes of names that holds every
    /// measured rule, and its table, `toc_end` bytes in all.
    fn head(entries: u32, toc_end: u32) -> Vec<u8> {
        let mut bytes = vec![0x5Au8; toc_end as usize];
        let mut head = vec![0u8; RPF9_HEADER_SIZE];
        let index = toc_end - (entries * 9).next_multiple_of(16);
        for (at, word) in [
            (0x00, RPF9_MAGIC),
            (0x04, toc_end),
            (0x08, index),
            (0x0C, 0x40),
            (0x1C, 0x150),
            (0x24, entries),
            (0x30, 0x10),
        ] {
            head[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        head[0x10..0x18].copy_from_slice(&[0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6, 0x07, 0x18]);
        head[0x19] = 0x7A;
        head[0x1A] = 4;
        head[0x20] = 5;
        head[0x22] = 1;
        head[0x2C] = 1;
        head[0x2D] = 0x0F;
        head[0x2E] = 0x09;
        bytes[..RPF9_HEADER_SIZE].copy_from_slice(&head);
        bytes
    }

    /// `text` as a zlib stream of one stored deflate block under the `78 DA` head a scan finds.
    fn zlib(text: &[u8]) -> Vec<u8> {
        let len = text.len() as u16;
        let mut bytes = vec![0x78, 0xDA, 0x01];
        bytes.extend(len.to_le_bytes());
        bytes.extend((!len).to_le_bytes());
        bytes.extend_from_slice(text);
        let (a, b) = text.iter().fold((1u32, 0u32), |(a, b), &c| {
            let a = (a + u32::from(c)) % 65_521;
            (a, (b + a) % 65_521)
        });
        bytes.extend(((b << 16) | a).to_be_bytes());
        bytes
    }

    /// An archive of one entry, its table to [`BODY_AT`], then `text` as a zlib stream.
    fn archive_with_a_body(text: &[u8]) -> Vec<u8> {
        let mut bytes = head(1, BODY_AT);
        bytes.extend(zlib(text));
        bytes
    }

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn the_archives_of_a_folder_are_found_by_their_head_and_tallied_apart_by_kind() {
        let dir = tempfile::tempdir().expect("a folder");
        let deep = dir.path().join("x64").join("levels");
        fs::create_dir_all(&deep).expect("folders");
        let text = b"<?xml version=\"1.0\"?>\n<CMapTypes>\n</CMapTypes>\n";
        let (first, second) = (deep.join("first.RPF"), dir.path().join("second.rpf"));
        fs::write(&first, archive_with_a_body(text)).expect("written");
        fs::write(
            &second,
            archive_with_a_body(b"plain words, no tag, more than one line\n"),
        )
        .expect("written");
        fs::write(dir.path().join("other.rpf"), b"RPF7 and the rest").expect("written");
        fs::write(dir.path().join("notes.txt"), archive_with_a_body(text)).expect("written");

        let mut wanted = vec![first.clone(), second.clone()];
        wanted.sort();
        let found = archives_under(dir.path());
        assert_eq!(found, (wanted, false), "only RPF9 heads named .rpf");

        let (find, _) = read_rpf9_body(
            fs::File::open(&first).expect("open"),
            u64::from(BODY_AT),
            1 << 20,
        )
        .expect("the body");
        let (row, family) = (scan_row(&find), Rpf9Family::of(&find).word());
        let never = Cancel::new();
        for kind in [row.as_str(), family] {
            let admits = |f: &Rpf9ScanFind| admitted(Some(kind), f);
            let (tally, kept, ended) = tally_archive(&first, &never, &admits, false);
            assert_eq!(ended, Ok(()));
            assert!(kept.is_empty(), "no offset is kept unasked");
            assert!(
                tally.get(&row).is_some_and(|n| *n >= 1),
                "{kind}: {tally:?}"
            );
            assert!(
                tally.keys().all(|r| kind == family || *r == row),
                "{kind} admitted another row: {tally:?}"
            );
        }
        let every = |_: &Rpf9ScanFind| true;
        let (all, kept, _) = tally_archive(&first, &never, &every, true);
        let mut scanned: Vec<u64> =
            Rpf9Scan::new(fs::File::open(&first).expect("open"), 0..u64::MAX)
                .expect("a scan")
                .map(|find| find.expect("a find").offset)
                .collect();
        scanned.sort_unstable();
        assert_eq!(
            all.values().sum::<u64>(),
            scanned.len() as u64,
            "every find is counted once"
        );
        for (row, count) in &all {
            assert_eq!(kept[row].len() as u64, *count, "{row}: every offset kept");
        }
        let mut offsets: Vec<u64> = kept.values().flatten().copied().collect();
        offsets.sort_unstable();
        assert_eq!(offsets, scanned, "each find's own offset");

        let stopped = Cancel::new();
        stopped.set();
        let (tally, _, ended) = tally_archive(&first, &stopped, &every, true);
        assert!(
            ended.is_err() && tally.is_empty(),
            "a stop is said and counts nothing"
        );

        let root = dir.path().to_string_lossy().into_owned();
        let root = root.as_str();
        assert_eq!(
            crate::rpf::run(&words(&["scan-install", root])),
            ExitCode::SUCCESS
        );
        assert_eq!(
            crate::rpf::run(&words(&["scan-install", root, "--kind", family])),
            ExitCode::SUCCESS
        );
        assert_eq!(
            crate::rpf::run(&words(&["scan-install", root, "--sample", "2"])),
            ExitCode::SUCCESS
        );
        for bad in ["0", "10001", "two"] {
            assert_eq!(
                crate::rpf::run(&words(&["scan-install", root, "--sample", bad])),
                ExitCode::from(2),
                "--sample {bad}"
            );
        }
        let empty = tempfile::tempdir().expect("a folder");
        let nothing = empty.path().to_string_lossy().into_owned();
        assert_eq!(
            crate::rpf::run(&words(&["scan-install", nothing.as_str()])),
            ExitCode::FAILURE
        );
        let file = format!("{root}/notes.txt");
        assert_eq!(
            crate::rpf::run(&words(&["scan-install", file.as_str()])),
            ExitCode::from(2)
        );
    }

    #[test]
    fn names_placed_by_count_are_tallied_and_listed_as_rpf_scan_lists_them() {
        // A nested archive of one entry whose names stand before a top-level body, no header.
        let nested = 0x1000u64;
        let mut bytes = head(2, 0x1B0);
        bytes.resize(nested as usize, 0);
        bytes.extend(head(1, 0x1B0));
        bytes.extend(zlib(b"<?xml version=\"1.0\"?>\n<CMapTypes/>\n"));
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        let names_at = bytes.len() as u64;
        bytes.extend([0x33; 0x40]);
        bytes.extend(zlib(b"<?xml version=\"1.0\"?>\n<CMapData/>\n"));
        let dir = tempfile::tempdir().expect("a folder");
        let path = dir.path().join("data.rpf");
        fs::write(&path, &bytes).expect("written");

        let file = fs::File::open(&path).expect("open");
        let names = or_io::container::rpf::rpf9_names_by_count_at(&file, nested)
            .expect("a count")
            .expect("names placed by count");
        assert_eq!(names.offset, names_at);
        let streamed = (Rpf9Scan::new(&file, 0..u64::MAX).expect("a scan"))
            .map(|find| find.expect("a find"))
            .any(|find| find.kind == names.kind);
        assert!(!streamed, "the stream scan places no names here");

        let every = |_: &Rpf9ScanFind| true;
        let (tally, kept, ended) = tally_archive(&path, &Cancel::new(), &every, true);
        assert_eq!(ended, Ok(()));
        let row = scan_row(&names);
        assert_eq!(tally.get(&row), Some(&1), "{tally:?}");
        assert_eq!(kept[&row], [names_at]);
        let only = |find: &Rpf9ScanFind| find.offset != names_at;
        let (without, _, _) = tally_archive(&path, &Cancel::new(), &only, false);
        assert_eq!(
            without.get(&row),
            None,
            "admits is asked of a placed find too"
        );

        let archive = path.to_string_lossy().into_owned();
        let archive = archive.as_str();
        for args in [
            vec!["list", archive, "--json"],
            vec!["list", archive, "--kind", "archives"],
        ] {
            assert_eq!(
                crate::rpf::run(&words(&args)),
                ExitCode::SUCCESS,
                "{args:?}"
            );
        }
        let json = crate::rpf::listed_rpf9(archive, None, true).unwrap_or_default();
        let document: serde_json::Value = serde_json::from_str(&json[0]).expect("one document");
        let bodies = document["bodies"].as_array().expect("bodies");
        let offsets: Vec<u64> = bodies.iter().filter_map(|b| b["offset"].as_u64()).collect();
        let placed = offsets
            .iter()
            .position(|o| *o == names_at)
            .expect("the placed names");
        assert!(offsets.windows(2).all(|w| w[0] <= w[1]), "{offsets:?}");
        assert_eq!(bodies[placed]["nested_in"], serde_json::json!(nested));
        let family = Some(Rpf9Family::of(&names));
        let lines = crate::rpf::listed_rpf9(archive, family, false).unwrap_or_default();
        let line = crate::rpf::scan_line(&names);
        let line = format!("{line}; placed by counting its header's entries");
        assert!(lines.contains(&line), "{lines:?}");
    }

    #[test]
    fn a_sample_takes_offsets_evenly_spaced_from_the_first() {
        let all: Vec<u64> = (0..10).map(|i| 0x100 * i).collect();
        let picked: Vec<u64> = (0..3).map(|i| all[i * all.len() / 3]).collect();
        assert_eq!(evenly(&all, 3), picked);
        assert_eq!(evenly(&all, 3), [0, 0x300, 0x600]);
        assert_eq!(evenly(&all, 10), all);
        assert_eq!(evenly(&all, 50), all, "fewer than asked are all taken");
        assert_eq!(evenly(&all, 1), [0]);
    }
}
