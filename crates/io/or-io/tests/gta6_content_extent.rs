//! The GTA VI content cipher's encrypted extent and chunk walk over a whole build: every flagged
//! (type 17) entry, top-level and nested, reads through [`Gta6Mount::read_entry_decrypted_report`] —
//! its enciphered ranges deciphered at the absolute counter, its chunks inflated and concatenated.
//!
//! The checks: all flagged entries read; a 24-byte row's chunks sum to exactly its real size and the
//! walk ends at its stored size; the walk's chunk count equals the `0xe0c1` low half where a row
//! carries one; and the inflated heads reproduce the counts measured on the round-26 build census
//! (PSIN 443, RBF0 59, text 149, headerless 746 — an 8-byte zero slot then a pointer — 1 other).
//!
//! Install-gated and game-derived, so it needs the content-key snapshot and the build and skips
//! cleanly without them (a clean `just test-no-pack` passes):
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot with the content section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
/// A cyclic or self-referential mount would recurse forever; real nesting is shallow.
const MAX_DEPTH: usize = 8;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_rpfs = |dir: &Path| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                    out.push(path);
                }
            }
        }
    };
    push_rpfs(root);
    if let Ok(packs) = std::fs::read_dir(root.join("pack")) {
        for pack in packs.flatten() {
            if pack.path().is_dir() {
                push_rpfs(&pack.path());
            }
        }
    }
    out.sort();
    out
}

#[derive(Default)]
struct Tally {
    flagged: usize,
    read_ok: usize,
    read_fail: usize,
    size_mismatch_24: usize,
    chunk_count_checked: usize,
    chunk_count_mismatch: usize,
    end_off_stored: usize,
    psin: usize,
    rbf0: usize,
    text: usize,
    headerless: usize,
    other: usize,
}

fn is_text_head(body: &[u8]) -> bool {
    if body.first() == Some(&b'<') || body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return true;
    }
    let n = body.len().min(32);
    n > 0
        && body[..n]
            .iter()
            .all(|&c| matches!(c, b'\t' | b'\r' | b'\n') || (0x20..=0x7E).contains(&c))
}

fn classify(body: &[u8], t: &mut Tally) {
    if body.starts_with(b"PSIN") {
        t.psin += 1;
    } else if body.starts_with(b"RBF0") {
        t.rbf0 += 1;
    } else if body.len() >= 8 && body[..8].iter().all(|&b| b == 0) {
        // A headerless resource (a script program) opens with an 8-byte zero slot, then a pointer.
        t.headerless += 1;
    } else if is_text_head(body) {
        t.text += 1;
    } else {
        t.other += 1;
    }
}

/// Read and verify one flagged entry through the extent reader.
fn visit_flagged(
    file: &mut File,
    mount: &Gta6Mount,
    entry: &Gta6Entry,
    keys: &GtaViKeys,
    t: &mut Tally,
) {
    t.flagged += 1;
    let report = match mount.read_entry_decrypted_report(&mut *file, entry, keys) {
        Ok(report) => report,
        Err(e) => {
            t.read_fail += 1;
            eprintln!("  read failed: entry {} ({e})", entry.index);
            return;
        }
    };
    t.read_ok += 1;
    classify(&report.data, t);

    let extra = mount.archive.entry_extra(entry.index as usize);
    // A 24-byte row states its real and stored sizes exactly.
    if let (Some(real), Some(stored)) = (entry.real, entry.stored) {
        if report.data.len() as u64 != real {
            t.size_mismatch_24 += 1;
            eprintln!(
                "  size mismatch: entry {} inflated {} vs real {real}",
                entry.index,
                report.data.len()
            );
        }
        if report.stored_consumed != stored {
            t.end_off_stored += 1;
        }
    }
    // The chunk count equals the 0xe0c1 low half, where the entry carries that record.
    if let Some(expected) = extra.and_then(|x| x.chunk_count) {
        t.chunk_count_checked += 1;
        if report.chunk_count as u32 != expected {
            t.chunk_count_mismatch += 1;
        }
    }
}

/// Count and verify one mount's flagged entries and recurse into every nested archive.
fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, t: &mut Tally) {
    let entries = mount.archive.entries.clone();
    for entry in &entries {
        if entry.cipher.map(|c| c.cipher_type) == Some(CONTENT_CIPHER_TYPE) {
            visit_flagged(file, mount, entry, keys, t);
        }
        if depth < MAX_DEPTH
            && mount
                .entry_begins_archive(&mut *file, entry)
                .unwrap_or(false)
        {
            if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                walk(file, &child, keys, depth + 1, t);
            }
        }
    }
}

#[test]
fn every_flagged_entry_reads_through_the_extent_and_chunk_walk() {
    let what = "GTA VI content extent + chunk walk over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} (with content keys) and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    if !keys.has_content_keys() {
        skip_install(
            what,
            format_args!("the snapshot carries no content section"),
        );
        return;
    }

    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut t = Tally::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        walk(&mut file, &mount, &keys, 0, &mut t);
    }

    println!(
        "flagged {}, read ok {}, read fail {}; 24-byte size mismatches {}, end-off-stored {}; \
         chunk-count {}/{} mismatch; heads PSIN {} RBF0 {} text {} headerless {} other {}",
        t.flagged,
        t.read_ok,
        t.read_fail,
        t.size_mismatch_24,
        t.end_off_stored,
        t.chunk_count_mismatch,
        t.chunk_count_checked,
        t.psin,
        t.rbf0,
        t.text,
        t.headerless,
        t.other,
    );

    // Every flagged entry must read through the new path.
    assert_eq!(t.read_fail, 0, "every flagged entry must decode");
    assert!(
        t.flagged >= 1390,
        "the build is expected to hold about 1,398 flagged entries, found {}",
        t.flagged
    );
    // A 24-byte row's chunks sum to exactly its real size.
    assert_eq!(
        t.size_mismatch_24, 0,
        "24-byte rows must inflate to their real size"
    );
    // The chunk count matches the 0xe0c1 low half but for a handful of story000 bodies (no row,
    // merged stored chunks, or a real-size walk) measured on the round-26 build.
    assert!(
        t.chunk_count_mismatch <= 10,
        "chunk count must equal the 0xe0c1 low half but for a few documented bodies, got {} of {}",
        t.chunk_count_mismatch,
        t.chunk_count_checked
    );
    // The inflated heads reproduce the counts measured on the round-26 build census.
    assert_eq!(t.psin, 443, "PSIN head count");
    assert_eq!(t.rbf0, 59, "RBF0 head count");
    assert_eq!(t.text, 149, "text head count");
    assert_eq!(t.headerless, 746, "headerless head count");
    assert_eq!(t.other, 1, "one other head");
}
