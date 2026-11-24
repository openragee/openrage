//! The `0xe0c1` chunk table over a whole build: every entry that carries a row, top-level and nested,
//! decodes, authors back byte-identical (the cell's `u1` and its descriptor slice in the heap), and
//! sums to the sizes its entry row states; every flagged (type 17) entry with a row reads through the
//! table walk to exactly its table's sizes.
//!
//! Install-gated and game-derived, so it needs the key snapshot and the build and skips cleanly
//! without them (a clean `just test-no-pack` passes):
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot with the content section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::path::{Path, PathBuf};

use byteorder::{ByteOrder, LE};
use or_io::container::rpf::gta6_chunk_table::{author, cell_offset, decode, Chunk};
use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
const MAX_DEPTH: usize = 8;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_rpfs = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    };
    push_rpfs(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            push_rpfs(&pack.path());
        }
    }
    out.sort();
    out
}

#[derive(Default)]
struct Tally {
    rows: usize,
    mode3: usize,
    mode2: usize,
    failures: Vec<String>,
    flagged_walked: usize,
}

fn ceil16(x: u32) -> u64 {
    (u64::from(x) + 15) & !15
}

/// Check one entry's row: decode, author back byte-identical, and sum to its entry row's sizes.
fn check_row(mount: &Gta6Mount, index: usize, at: usize, t: &mut Tally) -> Option<Vec<Chunk>> {
    let bytes = &mount.archive.toc.bytes;
    let cell: [u8; 8] = bytes[at..at + 8].try_into().ok()?;
    let fail = |t: &mut Tally, why: String| t.failures.push(format!("entry {index}: {why}"));
    let table = match decode(cell, &bytes[at..]) {
        Ok(table) => table,
        Err(e) => {
            fail(t, format!("decode: {e}"));
            return None;
        }
    };
    t.rows += 1;
    let mode = cell[7];
    if table.mode != mode {
        fail(
            t,
            format!("decoded mode {} from a mode-{mode} cell", table.mode),
        );
    }
    let (u1, words) = match author(&table) {
        Ok(authored) => authored,
        Err(e) => {
            fail(t, format!("author: {e}"));
            return None;
        }
    };
    let heap = at + LE::read_u32(&cell[0..]) as usize;
    let stored_words: Vec<u64> = bytes[heap..heap + 8 * words.len()]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|w| u64::from_le_bytes(*w))
        .collect();
    if u1 != LE::read_u32(&cell[4..]) || words != stored_words {
        fail(
            t,
            "authored cell or heap differs from the stored bytes".into(),
        );
    }
    if mode == 3 {
        t.mode3 += 1;
    } else {
        t.mode2 += 1;
    }
    let chunks = table.chunks;

    let inflated: u64 = chunks.iter().map(|c| u64::from(c.inflated)).sum();
    let stored: u64 = chunks.iter().map(|c| ceil16(c.stored)).sum();
    let entry = &mount.archive.entries[index];
    if let (Some(real), Some(row_stored)) = (entry.real, entry.stored) {
        if inflated != real || stored != row_stored {
            fail(
                t,
                format!("sums {inflated}/{stored}, row {real}/{row_stored}"),
            );
        }
    } else if let Some(nested) = mount.archive.entry_extra(index).and_then(|x| x.nested) {
        if stored != nested.stored_size().saturating_sub(16) {
            fail(
                t,
                format!("stored sum {stored}, 20-byte row {}", nested.stored_size()),
            );
        }
    }
    Some(chunks)
}

fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, t: &mut Tally) {
    for entry in &mount.archive.entries {
        let index = entry.index as usize;
        if let Some(at) = cell_offset(&mount.archive, index) {
            let chunks = check_row(mount, index, at, t);
            let flagged = entry.cipher.map(|c| c.cipher_type) == Some(CONTENT_CIPHER_TYPE);
            if let (Some(chunks), true) = (chunks, flagged) {
                t.flagged_walked += 1;
                match mount.read_entry_decrypted_report(&mut *file, entry, keys) {
                    Ok(body) => {
                        let inflated: u64 = chunks.iter().map(|c| u64::from(c.inflated)).sum();
                        let stored: u64 = chunks.iter().map(|c| ceil16(c.stored)).sum();
                        if body.chunk_count != chunks.len()
                            || body.data.len() as u64 != inflated
                            || body.stored_consumed != stored
                        {
                            t.failures
                                .push(format!("entry {index}: the walk left its table's sizes"));
                        }
                    }
                    Err(e) => t.failures.push(format!("entry {index}: read: {e}")),
                }
            }
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
fn every_chunk_row_authors_back_byte_identical_and_drives_the_walk() {
    let what = "GTA VI 0xe0c1 chunk tables over a build";
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
        "chunk rows {} (mode 3 {}, mode 2 {}), flagged walked {}, failures {}",
        t.rows,
        t.mode3,
        t.mode2,
        t.flagged_walked,
        t.failures.len()
    );
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        t.failures.is_empty(),
        "every chunk row must round-trip and read"
    );
    assert!(
        t.mode3 >= 928,
        "the top-level tables hold at least 928 mode-3 rows"
    );
    assert!(t.mode2 > 0, "the nested tables hold mode-2 rows");
}
