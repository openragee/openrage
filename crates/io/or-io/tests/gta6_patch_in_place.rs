//! The in-place patch writer over a real GTA VI build: over a bounded sample of each row shape, a
//! row's own body plans no change and writes a byte-identical copy, and the body with one byte flipped
//! in its first stored chunk (else its first chunk) is written to a copy and read back, or refused by
//! name; the tally per shape is printed, with the fits only the optimal parse made, how far each
//! encoder's stream runs past a ceiling it misses, the edits of chunks past 256 KiB the optimal parse
//! was run over, the refusals after the harder tries, and the slowest plan; a chunk within
//! `OPTIMAL_CAP` is never refused untried. Each
//! archive is cut to the prefix its sampled rows need, so no archive is copied whole.
//!
//! Install-gated and game-derived, so it skips cleanly without its env vars:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_chunk_table::{cell_offset, decode};
use or_io::container::rpf::gta6_patch::OPTIMAL_CAP;
use or_io::container::rpf::{
    open_at, plan_patch, write_patched_copy, BodyPatch, Gta6Class, Gta6Entry, Gta6Mount, GtaViKeys,
    PatchRefusal, GTA6_SNAPSHOT_ENV,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A sampled row's stored bytes end within this many bytes of its file's start: the prefix copied.
const PREFIX_CAP: u64 = 16 << 20;
/// Rows sampled per shape across the build.
const PER_SHAPE: usize = 4;
/// Nested archives walked per archive, and how deep.
const NESTED_PER_ARCHIVE: usize = 4;
const MAX_DEPTH: usize = 3;
/// An edited chunk of more than this is tallied apart when the optimal parse is run over it.
const LARGE: usize = 256 << 10;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every `*.rpf` directly under `root`, then every `pack/*/*.rpf`, sorted.
fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                    out.push(path);
                }
            }
        }
    };
    rpfs_in(root);
    if let Ok(packs) = std::fs::read_dir(root.join("pack")) {
        for pack in packs.flatten() {
            if pack.path().is_dir() {
                rpfs_in(&pack.path());
            }
        }
    }
    out.sort();
    out
}

/// A row's shape: its row width, its chunk table's mode, and whether it is ciphered.
fn shape(mount: &Gta6Mount, e: &Gta6Entry) -> String {
    let row = if e.stored.is_some() { 24 } else { 20 };
    let table = match cell_offset(&mount.archive, e.index as usize) {
        Some(at) => {
            let heap = &mount.archive.toc.bytes[at..];
            let mut cell = [0u8; 8];
            cell.copy_from_slice(&heap[..8]);
            decode(cell, heap).map_or("a malformed table".to_owned(), |t| {
                format!("a mode {} table", t.mode)
            })
        }
        None => "no table".to_owned(),
    };
    let cipher = if e.cipher.is_some() {
        "ciphered"
    } else {
        "clear"
    };
    format!("{row}-byte row, {table}, {cipher}")
}

/// The end of `e`'s stored bytes, where the patch writer reads them.
fn stored_end(mount: &Gta6Mount, e: &Gta6Entry) -> Option<u64> {
    let start = mount.absolute_start(e)?;
    if let Some(stored) = e.stored {
        return Some(start + stored);
    }
    let nested = mount.archive.entry_extra(e.index as usize)?.nested?;
    let (at, len) = nested.ciphered_body_extent(start);
    Some(at + len)
}

struct Candidate {
    base: u64,
    entry: u32,
    shape: String,
    end: u64,
}

fn sample(
    file: &mut File,
    keys: &GtaViKeys,
    mount: &Gta6Mount,
    depth: usize,
    counts: &mut BTreeMap<String, usize>,
    out: &mut Vec<Candidate>,
) {
    let mut nested = 0;
    for e in &mount.archive.entries {
        let Some(end) = stored_end(mount, e).filter(|&end| end <= PREFIX_CAP) else {
            continue;
        };
        if e.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH && nested < NESTED_PER_ARCHIVE {
                nested += 1;
                if let Ok(inner) = mount.open_nested(&mut *file, e, keys) {
                    sample(file, keys, &inner, depth + 1, counts, out);
                }
            }
            continue;
        }
        let shape = shape(mount, e);
        let n = counts.entry(shape.clone()).or_default();
        if *n < PER_SHAPE {
            *n += 1;
            out.push(Candidate {
                base: mount.base,
                entry: e.index,
                shape,
                end,
            });
        }
    }
}

#[derive(Default)]
struct Tally {
    sampled: usize,
    unread: usize,
    disagree: usize,
    fit: usize,
    /// Fits whose stream the optimal parse made, no level of the tree's deflater fitting.
    fit_optimal: usize,
    /// Own-body copies written, each byte-identical to the source.
    identical: usize,
    refused: BTreeMap<String, usize>,
    /// For a chunk that does not fit: how far its best stream runs past its ceiling (negative: short).
    over: Vec<i64>,
    /// The same for the optimal parse's stream.
    over_optimal: Vec<i64>,
    /// The longest one edited body took to plan.
    slowest: Duration,
    /// Edits of a chunk past [`LARGE`] that the optimal parse was run over.
    large_tried: usize,
    /// Refusals after the harder tries ran, the first optimal parse missing within the margin.
    harder_refused: usize,
}

fn refusal_name(r: &PatchRefusal) -> String {
    let debug = format!("{r:?}");
    debug
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn mount_at(file: &mut File, base: u64, keys: &GtaViKeys) -> Gta6Mount {
    Gta6Mount {
        base,
        archive: open_at(file, base, keys).expect("the table opens"),
    }
}

#[test]
fn a_one_byte_edit_patches_in_place_or_is_refused_by_name_per_row_shape() {
    let what = "GTA VI in-place patch over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (prefix, copy) = (
        scratch.path().join("prefix.rpf"),
        scratch.path().join("patched.rpf"),
    );

    let mut counts = BTreeMap::new();
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let Ok(root) = Gta6Mount::open_root(&mut file, &keys) else {
            continue;
        };
        let mut candidates = Vec::new();
        sample(&mut file, &keys, &root, 0, &mut counts, &mut candidates);
        let Some(len) = candidates.iter().map(|c| c.end).max() else {
            continue;
        };
        let mut held = vec![0u8; len as usize];
        file.seek(SeekFrom::Start(0)).unwrap();
        file.read_exact(&mut held).expect("read the prefix");
        std::fs::write(&prefix, &held).expect("write the prefix");

        for c in &candidates {
            let tally = tallies.entry(c.shape.clone()).or_default();
            tally.sampled += 1;
            let mut src = File::open(&prefix).unwrap();
            let mount = mount_at(&mut src, c.base, &keys);
            let entry = &mount.archive.entries[c.entry as usize];
            let Ok(original) = mount.read_entry_decrypted(&mut src, entry, &keys) else {
                tally.unread += 1;
                continue;
            };
            let same = BodyPatch {
                base: c.base,
                entry: c.entry,
                body: original.clone(),
            };
            let plan = match plan_patch(&mut src, &keys, &same) {
                Ok(plan) => plan,
                Err(r) => {
                    *tally.refused.entry(refusal_name(&r)).or_default() += 1;
                    continue;
                }
            };
            // The row's own body, as OpenRage reads it, must be what its chunks hold.
            if plan.chunks.iter().any(|k| k.changed) {
                tally.disagree += 1;
                continue;
            }
            // One own-body copy per shape: a patch that changes nothing writes the source again.
            if tally.identical == 0 {
                let report = write_patched_copy(&prefix, &copy, &keys, std::slice::from_ref(&same))
                    .unwrap_or_else(|e| panic!("{}: an own-body copy writes: {e}", c.shape));
                assert_eq!((report.chunks_changed, report.bytes_written), (0, 0));
                assert!(
                    std::fs::read(&copy).unwrap() == held,
                    "{}: identical",
                    c.shape
                );
                std::fs::remove_file(&copy).unwrap();
                tally.identical += 1;
            }
            // A stored chunk takes any content of its length; else the first chunk is edited.
            let chosen = plan.chunks.iter().position(|k| !k.zlib).unwrap_or(0);
            let end: u64 = plan.chunks[..=chosen]
                .iter()
                .map(|k| u64::from(k.inflated))
                .sum();
            let Some(last) = (end as usize).checked_sub(1) else {
                continue;
            };
            let mut altered = original;
            altered[last] ^= 1;
            let patch = BodyPatch {
                body: altered.clone(),
                ..same
            };
            let inflated = |k: usize| plan.chunks.get(k).map_or(0, |c| c.inflated as usize);
            let began = Instant::now();
            let planned = plan_patch(&mut src, &keys, &patch);
            tally.slowest = tally.slowest.max(began.elapsed());
            match planned {
                Err(r) => {
                    if let PatchRefusal::ChunkDoesNotFit {
                        chunk,
                        deflated,
                        optimal,
                        ceiling,
                        ref harder,
                        ..
                    } = r
                    {
                        tally.harder_refused += usize::from(harder.is_some());
                        let past =
                            |n: usize| i64::try_from(n).unwrap_or(i64::MAX) - i64::from(ceiling);
                        tally.over.push(past(deflated));
                        // `usize::MAX` is a chunk past the cap, on which the optimal parse is not tried.
                        if optimal != usize::MAX {
                            tally.over_optimal.push(past(optimal));
                            tally.large_tried += usize::from(inflated(chunk) > LARGE);
                        } else {
                            assert!(
                                inflated(chunk) > OPTIMAL_CAP,
                                "{}: chunk {chunk} of {} bytes is within the cap and was not given \
                                 the optimal parse",
                                c.shape,
                                inflated(chunk)
                            );
                        }
                    }
                    *tally.refused.entry(refusal_name(&r)).or_default() += 1;
                }
                Ok(plan) => {
                    let optimal = plan.chunks.iter().any(|k| k.optimal);
                    tally.fit_optimal += usize::from(optimal);
                    tally.large_tried += usize::from(optimal && inflated(chosen) > LARGE);
                    let report = write_patched_copy(&prefix, &copy, &keys, &[patch])
                        .unwrap_or_else(|e| panic!("{}: a planned patch writes: {e}", c.shape));
                    assert_eq!(report.chunks_changed, 1, "{}", c.shape);
                    assert!(!report.table_spans.is_empty());
                    let mut out = File::open(&copy).unwrap();
                    let patched = mount_at(&mut out, c.base, &keys);
                    let back = patched
                        .read_entry_decrypted(
                            &mut out,
                            &patched.archive.entries[c.entry as usize],
                            &keys,
                        )
                        .unwrap_or_else(|e| panic!("{}: the patched row reads: {e}", c.shape));
                    assert!(back == altered, "{}: the patched row reads back", c.shape);
                    tally.fit += 1;
                    drop(out);
                    std::fs::remove_file(&copy).unwrap();
                }
            }
        }
        assert!(
            std::fs::read(&prefix).unwrap() == held,
            "{}: the source prefix is untouched",
            path.display()
        );
    }

    let mut sampled = 0;
    for (shape, t) in &tallies {
        sampled += t.sampled;
        let sorted = |v: &[i64]| {
            let mut v = v.to_vec();
            v.sort_unstable();
            v
        };
        eprintln!(
            "{shape}: {} sampled, {} fit ({} only by the optimal parse), {} own-body copies \
             identical, {} unread, {} where reader and table disagree, refused {:?}, past the \
             ceiling by {:?} with the tree's deflater and {:?} with the optimal parse; {} edits of \
             a chunk past 256 KiB given the optimal parse; {} refused after the harder tries; \
             slowest plan {} ms",
            t.sampled,
            t.fit,
            t.fit_optimal,
            t.identical,
            t.unread,
            t.disagree,
            t.refused,
            sorted(&t.over),
            sorted(&t.over_optimal),
            t.large_tried,
            t.harder_refused,
            t.slowest.as_millis()
        );
    }
    for (shape, t) in &tallies {
        // A malformed refusal of a row the reader reads is the patcher misreading the build.
        assert_eq!(t.refused.get("Malformed"), None, "{shape}: {:?}", t.refused);
        // A row whose body the reader and the patch's chunk walk see apart would be patched wrong.
        assert_eq!(
            t.disagree, 0,
            "{shape}: the reader and the chunk table disagree"
        );
    }
    assert!(sampled > 0, "no row of the build was sampled");
}
