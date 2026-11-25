//! Every 20-byte nested row of a build is authored back from its own sections and chunk table, of
//! mode 2 or mode 4: `w0`, `w4`, `w8` and `w16` equal the stored words, per type code. A sample of
//! each code's clear bodies is the zero slot and 16-aligned chunks `stored_body` lays out, each chunk
//! inflating from exactly its stored bytes to its stated size, the sizes summing to the two sections.
//! No table's chunk tables mix mode 4 with another mode, the rule the container writer follows.
//!
//! Install-gated and slow (it mounts every nested table of a build), so it skips cleanly without:
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use byteorder::{ByteOrder, LE};
use or_io::container::rpf::gta6_chunk_table::{cell_offset, entry_chunks};
use or_io::container::rpf::gta6_form_h::{row, stored_body};
use or_io::container::rpf::{Gta6Entry, Gta6Mount, Gta6NestedRow, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
const MAX_DEPTH: usize = 8;
/// Clear bodies read per type code; the rows are checked without reading any body.
const BODIES_PER_CODE: usize = 16;
/// The chunk-table mode that states exact inflated sizes, found only inside nested archives.
const MODE4: u8 = 4;

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
struct Code {
    rows: usize,
    authored: usize,
    no_table: usize,
    authored_mode4: usize,
    bodies: usize,
}

#[derive(Default)]
struct Tally {
    codes: BTreeMap<u32, Code>,
    /// Tables holding a chunk table, and those whose chunk tables take mode 4.
    tables: usize,
    mode4_tables: usize,
    failures: Vec<String>,
}

/// The bytes a stored chunk inflates to, `None` unless zlib or raw DEFLATE ends exactly at its end.
fn inflated_len(stored: &[u8]) -> Option<u64> {
    let mut z = flate2::bufread::ZlibDecoder::new(stored);
    if let Ok(n) = std::io::copy(&mut z, &mut std::io::sink()) {
        if z.total_in() == stored.len() as u64 {
            return Some(n);
        }
    }
    let mut d = flate2::bufread::DeflateDecoder::new(stored);
    let n = std::io::copy(&mut d, &mut std::io::sink()).ok()?;
    (d.total_in() == stored.len() as u64).then_some(n)
}

fn check(
    file: &mut File,
    mount: &Gta6Mount,
    entry: &Gta6Entry,
    nested: Gta6NestedRow,
    t: &mut Tally,
) {
    let index = entry.index as usize;
    let type_code = nested.resource_type;
    let Tally {
        codes, failures, ..
    } = t;
    let code = codes.entry(type_code).or_default();
    code.rows += 1;
    let mut fail =
        |why: String| failures.push(format!("type {type_code:#x}, entry {index}: {why}"));
    let Some(at) = cell_offset(&mount.archive, index) else {
        code.no_table += 1;
        return;
    };
    let mode4 = mount.archive.toc.bytes[at + 7] == MODE4;
    let chunks = match entry_chunks(&mount.archive, index) {
        Ok(Some(chunks)) => chunks,
        Ok(None) => {
            code.no_table += 1;
            return;
        }
        Err(e) => return fail(format!("chunk table: {e}")),
    };
    let (virtual_len, physical_len) = (nested.virtual_size & !0xF, nested.flags & !0xF);
    let authored = match row(type_code, virtual_len, physical_len, &chunks) {
        Ok(authored) => authored,
        Err(e) => return fail(format!("author: {e}")),
    };
    let word = |i: usize| LE::read_u32(&authored[4 * i..]);
    let stored = [
        nested.virtual_size,
        nested.flags,
        nested.stored_pages,
        nested.resource_type,
    ];
    let authored_words = [word(0), word(1), word(2), word(4)];
    if authored_words != stored {
        return fail(format!(
            "authored {authored_words:08x?}, stored {stored:08x?}"
        ));
    }
    code.authored += 1;
    code.authored_mode4 += usize::from(mode4);

    let ciphered = entry.cipher.map(|c| c.cipher_type) == Some(CONTENT_CIPHER_TYPE);
    if ciphered || code.bodies >= BODIES_PER_CODE {
        return;
    }
    let Some(start) = mount.absolute_start(entry) else {
        return fail("no start".into());
    };
    let len = nested.stored_size();
    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if start.checked_add(len).is_none_or(|end| end > file_len) {
        return fail("the stored body runs past the file".into());
    }
    let mut body = vec![0u8; len as usize];
    if let Err(e) = file
        .seek(SeekFrom::Start(start))
        .and_then(|_| file.read_exact(&mut body))
    {
        return fail(format!("read: {e}"));
    }
    let mut slices = Vec::with_capacity(chunks.len());
    let mut pos = 16usize;
    let mut total = 0u64;
    for (i, chunk) in chunks.iter().enumerate() {
        let Some(bytes) = body.get(pos..pos + chunk.stored as usize) else {
            return fail(format!("chunk {i} runs past the stored body"));
        };
        let inflated = if chunk.zlib {
            inflated_len(bytes)
        } else {
            Some(bytes.len() as u64)
        };
        if inflated != Some(u64::from(chunk.inflated)) {
            return fail(format!("chunk {i} does not inflate to its stated size"));
        }
        total += u64::from(chunk.inflated);
        slices.push(bytes);
        pos = (pos + chunk.stored as usize).next_multiple_of(16);
    }
    if total != u64::from(virtual_len) + u64::from(physical_len) {
        return fail(format!(
            "the chunks inflate to {total} bytes, not the two sections"
        ));
    }
    if stored_body(&slices) != body {
        return fail("the stored body is not the slot and its chunks, 16-aligned".into());
    }
    code.bodies += 1;
}

fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, t: &mut Tally) {
    let modes: BTreeSet<u8> = (0..mount.archive.entries.len())
        .filter_map(|i| cell_offset(&mount.archive, i))
        .map(|at| mount.archive.toc.bytes[at + 7])
        .collect();
    if !modes.is_empty() {
        t.tables += 1;
    }
    if modes.contains(&MODE4) {
        t.mode4_tables += 1;
        if modes.len() > 1 {
            t.failures
                .push(format!("a table's chunk tables take modes {modes:?}"));
        }
    }
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(extras) {
        if let Some(nested) = extra.nested {
            check(file, mount, entry, nested, t);
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
fn every_headerless_row_authors_back_from_its_sections_and_chunks() {
    let what = "GTA VI 20-byte rows over a build";
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

    let mut t = Tally::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        walk(&mut file, &mount, &keys, 0, &mut t);
    }
    for (type_code, c) in &t.codes {
        println!(
            "type {type_code:#04x}: rows {}, authored {} (mode {MODE4} {}), no table {}, \
             bodies {}",
            c.rows, c.authored, c.authored_mode4, c.no_table, c.bodies
        );
    }
    println!(
        "tables with chunk tables {}, of them mode {MODE4} {}; failures {}",
        t.tables,
        t.mode4_tables,
        t.failures.len()
    );
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(t.failures.is_empty(), "every 20-byte row must author back");
    for (type_code, c) in &t.codes {
        assert_eq!(
            c.authored,
            c.rows - c.no_table,
            "type {type_code:#x}: every row with a chunk table authors back"
        );
    }
    let authored: usize = t.codes.values().map(|c| c.authored).sum();
    let mode4: usize = t.codes.values().map(|c| c.authored_mode4).sum();
    assert!(authored > 0, "the build's nested tables hold 20-byte rows");
    assert!(
        mode4 > 0,
        "the build's nested tables hold mode-{MODE4} rows"
    );
}
