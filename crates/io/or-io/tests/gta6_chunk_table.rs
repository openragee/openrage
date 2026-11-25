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
use std::io::{Read, Seek, SeekFrom};
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
    mode4: usize,
    failures: Vec<String>,
    flagged_walked: usize,
    clear_read: usize,
    deflated: usize,
    /// Deflated chunks whose stream ends before their stored size, and the most bytes left after one.
    slack_chunks: usize,
    most_slack: usize,
}

/// Where the zlib (or, failing that, raw DEFLATE) stream at the head of `bytes` ends.
fn stream_end(bytes: &[u8]) -> Option<usize> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut sink = vec![0u8; 1 << 16];
    for zlib in [true, false] {
        let mut d = Decompress::new(zlib);
        loop {
            let (at, out) = (d.total_in() as usize, d.total_out());
            match d.decompress(&bytes[at..], &mut sink, FlushDecompress::Finish) {
                Ok(Status::StreamEnd) => return Some(d.total_in() as usize),
                Ok(_) if d.total_in() as usize != at || d.total_out() != out => {}
                _ => break,
            }
        }
    }
    None
}

/// Count the deflated chunks of the deciphered `body` whose stream ends short of the stored size.
fn count_slack(body: &[u8], chunks: &[Chunk], t: &mut Tally) {
    let mut pos = 0usize;
    for c in chunks {
        let stored = c.stored as usize;
        if c.zlib {
            t.deflated += 1;
            let end = body.get(pos..pos + stored).and_then(stream_end);
            if let Some(end) = end.filter(|&end| end < stored) {
                t.slack_chunks += 1;
                t.most_slack = t.most_slack.max(stored - end);
            }
        }
        pos += ceil16(c.stored) as usize;
    }
}

/// A clear body larger than this is not read back: the check is the read, not the volume.
const CLEAR_READ_CAP: u64 = 4 << 20;

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
    match mode {
        3 => t.mode3 += 1,
        2 => t.mode2 += 1,
        _ => t.mode4 += 1,
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
            // A clear sized entry reads whole through its table, never cut off at a chunk boundary.
            let clear_sized = entry.cipher.is_none()
                && entry.stored.is_some()
                && entry.real.is_some_and(|real| real <= CLEAR_READ_CAP);
            if let (Some(chunks), true) = (&chunks, clear_sized) {
                let start = mount.absolute_start(entry).unwrap_or(0);
                let len = entry.stored.unwrap_or(0) as usize;
                let mut body = vec![0u8; len];
                if file.seek(SeekFrom::Start(start)).is_ok() && file.read_exact(&mut body).is_ok() {
                    count_slack(&body, chunks, t);
                }
            }
            if let (Some(chunks), true) = (&chunks, flagged) {
                if let Ok(body) = mount.read_entry_deciphered_stored(&mut *file, entry, keys) {
                    count_slack(&body, chunks, t);
                }
            }
            if chunks.is_some() && clear_sized {
                t.clear_read += 1;
                match mount.read_entry(&mut *file, entry) {
                    Ok(body) if Some(body.len() as u64) == entry.real => {}
                    Ok(body) => t.failures.push(format!(
                        "entry {index}: a clear chunked body read {} of {:?} bytes",
                        body.len(),
                        entry.real
                    )),
                    Err(e) => t.failures.push(format!("entry {index}: clear read: {e}")),
                }
            }
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
        "chunk rows {} (mode 3 {}, mode 2 {}, mode 4 {}), flagged walked {}, clear read {}, failures {}",
        t.rows,
        t.mode3,
        t.mode2,
        t.mode4,
        t.flagged_walked,
        t.clear_read,
        t.failures.len()
    );
    // The game's read stops at a chunk's stream end, so a stored size past it is not an error.
    println!(
        "deflated chunks {}, ending before their stored size {} (most bytes after the stream {})",
        t.deflated, t.slack_chunks, t.most_slack
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
    assert!(t.mode4 > 0, "the nested tables hold mode-4 rows");
    assert!(t.clear_read > 0, "clear chunked bodies were read back");
}
