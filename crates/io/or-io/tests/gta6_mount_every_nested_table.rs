//! Every nested RPF9 archive in a build mounts from the table: an entry whose body begins with the
//! clear `9FPR` header is opened by decrypting its table at `parent_base + start`, recursively, and
//! must do so with no refusals. This is the table-driven replacement for the bounded header scan, and
//! the `parent_base + start` join is proven by the magic landing exactly where the row points.
//!
//! Install-gated and slow (it walks a whole build, tens of gigabytes of file positions), so it skips
//! cleanly without the env vars and belongs in the slow tier:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A self-referential or cyclic mount would recurse forever; real nesting is shallow, so a depth past
/// this is a bug, not a deep tree.
const MAX_DEPTH: usize = 8;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

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

/// What a walk of a build's archives counted.
#[derive(Default)]
struct Walk {
    /// Nested archives mounted at every depth.
    nested: usize,
    /// Nested archives directly under a top-level archive (depth 0).
    top_level: usize,
    /// Non-ciphered entries with no entry row: the silent gap an unknown column set would leave, since
    /// such an entry cannot be probed and would be listed as a leaf without a start.
    rowless: usize,
    /// The unknown columns of the first mount that held a rowless entry, for the failure message.
    rowless_columns: Option<Vec<(u64, u16)>>,
}

/// Mount every nested archive under `mount`, recursively, counting the headers decrypted. A header
/// that is present (its body begins with `9FPR`) but does not decrypt is a refusal and fails the test;
/// a non-ciphered entry with no entry row is a coverage gap and is counted.
fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, w: &mut Walk) {
    assert!(
        depth <= MAX_DEPTH,
        "nesting past {MAX_DEPTH}: a cyclic mount"
    );
    for entry in &mount.archive.entries {
        // An enciphered body is not a clear nested header; it is left to the content-cipher work.
        if entry.cipher.is_some() {
            continue;
        }
        if entry.start.is_none() {
            w.rowless += 1;
            w.rowless_columns
                .get_or_insert_with(|| mount.archive.unknown_columns());
            continue;
        }
        let begins = mount.entry_begins_archive(file, entry).unwrap_or_else(|e| {
            panic!("probe entry {} at base {:#X}: {e}", entry.index, mount.base)
        });
        if !begins {
            continue;
        }
        // The magic landed at `base + start`; its table must now decrypt — 0 refusals is the bar.
        let child = mount.open_nested(file, entry, keys).unwrap_or_else(|e| {
            panic!(
                "nested archive at {:#X} did not decrypt: {e}",
                mount.absolute_start(entry).unwrap_or_default()
            )
        });
        w.nested += 1;
        if depth == 0 {
            w.top_level += 1;
        }
        walk(file, &child, keys, depth + 1, w);
    }
}

#[test]
fn every_nested_table_in_the_build_mounts() {
    let what = "GTA VI RPF9 nested mount over a build";
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

    let mut w = Walk::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        walk(&mut file, &mount, &keys, 0, &mut w);
    }

    // Every non-ciphered entry resolves to a start, so none was silently dropped for want of a row
    // shape; the unknown columns of the first offender name what a future shape must read.
    assert_eq!(
        w.rowless, 0,
        "{} non-ciphered entries have no entry row (unknown columns {:?})",
        w.rowless, w.rowless_columns
    );
    // The build's 22 tables hold 4,351 nested archives, every one directly under the top level.
    assert_eq!(
        w.top_level, 4_351,
        "nested archives directly under the top level"
    );
    assert_eq!(w.nested, 4_351, "nested archives at every depth");
    println!(
        "mounted {} nested RPF9 archives ({} at the top level), 0 refusals",
        w.nested, w.top_level
    );
}
