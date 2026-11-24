//! The `0x97c2` dependency lists over a whole build: in every table, top-level and nested, that has
//! the column, authoring the lists read back gives the stored type-4 table, every cell and the heap
//! byte for byte.
//!
//! Install-gated and game-derived, so it needs the key snapshot and the build and skips cleanly
//! without them (a clean `just test-no-pack` passes):
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::path::{Path, PathBuf};

use byteorder::{ByteOrder, LE};
use or_io::container::rpf::gta6_dependencies::{author, read, GROUP_DEPENDENCIES};
use or_io::container::rpf::{Gta6Mount, GtaViKeys, Rpf9Toc, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
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
    tables: usize,
    lists: usize,
    widths: [usize; 2],
    failures: Vec<String>,
}

/// The stored type-4 payload's start, found apart from the reader: the kind-4 descriptor's offset
/// from the metadata start.
fn stored_table_at(bytes: &[u8]) -> Option<usize> {
    let n = LE::read_u16(&bytes[0x1A..]) as usize;
    let metadata_start = 0x40 + ((8 * n + 15) & !15);
    (0..n)
        .find(|i| LE::read_u16(&bytes[0x40 + 8 * i..]) == 4)
        .map(|i| metadata_start + LE::read_u32(&bytes[0x44 + 8 * i..]) as usize)
}

/// Check one table: author what `read` gives and compare with the stored bytes.
fn check(toc: &Rpf9Toc, at: &str, t: &mut Tally) {
    let fail = |t: &mut Tally, why: String| t.failures.push(format!("{at}: {why}"));
    let deps = match read(toc) {
        Ok(Some(deps)) => deps,
        Ok(None) => return,
        Err(e) => return fail(t, format!("read: {e}")),
    };
    t.tables += 1;
    t.lists += deps.lists.len();
    t.widths[usize::from(deps.width == 2)] += 1;
    let authored = match author(&deps.lists) {
        Ok(a) => a,
        Err(e) => return fail(t, format!("author: {e}")),
    };
    let bytes = &toc.bytes;
    let Some(table_at) = stored_table_at(bytes) else {
        return fail(t, "no type-4 descriptor".into());
    };
    if bytes.get(table_at..table_at + authored.table.len()) != Some(&authored.table[..]) {
        fail(t, "the authored type-4 table differs".into());
    }
    let Some(group) = toc
        .groups
        .iter()
        .find(|g| g.name_hash == GROUP_DEPENDENCIES)
    else {
        return fail(t, "no 0x97c2 column".into());
    };
    // The heap starts at row 0's list; every other row follows packed.
    let first = group.data.start;
    let heap_at = first + LE::read_u32(&bytes[first..]) as usize;
    if bytes.get(heap_at..heap_at + authored.heap.len()) != Some(&authored.heap[..]) {
        fail(t, "the authored heap differs".into());
    }
    for (r, cell) in authored.cells.iter().enumerate() {
        let cell_at = first + 8 * r;
        match cell.bytes(cell_at, heap_at) {
            Ok(b) if b[..] == bytes[cell_at..cell_at + 8] => {}
            Ok(_) => fail(t, format!("the authored cell of row {r} differs")),
            Err(e) => fail(t, format!("row {r}: {e}")),
        }
    }
}

fn walk(
    file: &mut File,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    at: &str,
    depth: usize,
    t: &mut Tally,
) {
    check(&mount.archive.toc, at, t);
    if depth >= MAX_DEPTH {
        return;
    }
    for entry in &mount.archive.entries {
        if !mount
            .entry_begins_archive(&mut *file, entry)
            .unwrap_or(false)
        {
            continue;
        }
        if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
            let child_at = format!("{at}/{}", entry.index);
            walk(file, &child, keys, &child_at, depth + 1, t);
        }
    }
}

#[test]
fn every_dependency_table_authors_back_byte_identical() {
    let what = "GTA VI 0x97c2 dependency lists over a build";
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
    for (i, path) in archives.iter().enumerate() {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open archive {i}: {e}"));
        walk(&mut file, &mount, &keys, &format!("archive {i}"), 0, &mut t);
    }
    println!(
        "dependency tables {} (width 1 {}, width 2 {}), rows {}, failures {}",
        t.tables,
        t.widths[0],
        t.widths[1],
        t.lists,
        t.failures.len()
    );
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        t.failures.is_empty(),
        "every dependency table must author back byte-identical"
    );
    assert!(
        t.tables >= 76,
        "round 30's sample alone held 76 tables with the column"
    );
    assert!(
        t.widths[1] > 0,
        "the build holds tables of more than 256 keys"
    );
}
