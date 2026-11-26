//! Every row of a build, top-level and nested, is named by its class byte or its type code but the
//! class 0x0E rows, whose two bodies' heads agree on nothing; class 0x60's row is named.
//!
//! Install-gated and slow (it mounts every nested table of a build), so it skips cleanly without:
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_kinds::entry_kinds;
use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const MAX_DEPTH: usize = 8;
const ARCHIVE_CLASS: u8 = 0x00;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    };
    rpfs_in(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            rpfs_in(&pack.path());
        }
    }
    out.sort();
    out
}

/// Counts every row's kind label under `mount`, nested archives included.
fn count(
    file: &mut File,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    depth: usize,
    labels: &mut BTreeMap<String, usize>,
) {
    assert!(
        depth <= MAX_DEPTH,
        "nesting past {MAX_DEPTH}: a cyclic mount"
    );
    for label in entry_kinds(&mount.archive) {
        *labels.entry(label.into_owned()).or_default() += 1;
    }
    for entry in &mount.archive.entries {
        if entry.cipher.is_some() || entry.class != Some(ARCHIVE_CLASS) {
            continue;
        }
        let begins = (mount.entry_begins_archive(file, entry)).unwrap_or_else(|e| {
            panic!("probe entry {} at base {:#X}: {e}", entry.index, mount.base)
        });
        if !begins {
            continue;
        }
        let child = (mount.open_nested(file, entry, keys))
            .unwrap_or_else(|e| panic!("entry {} at base {:#X}: {e}", entry.index, mount.base));
        count(file, &child, keys, depth + 1, labels);
    }
}

#[test]
fn every_row_of_the_build_has_a_kind_but_the_two_class_0x0e_rows() {
    let what = "GTA VI row kinds over a build";
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
    let mut labels = BTreeMap::new();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        count(&mut file, &mount, &keys, 0, &mut labels);
    }
    let unnamed: BTreeMap<&str, usize> = (labels.iter())
        .filter(|(l, _)| l.starts_with("class ") || l.starts_with("type ") || *l == "no row")
        .map(|(l, n)| (l.as_str(), *n))
        .collect();
    // Measured on the build: one top-level row and one nested, enciphered row.
    assert_eq!(unnamed, BTreeMap::from([("class 0x0e", 2)]), "{labels:?}");
    assert_eq!(labels.get("object trees"), Some(&1), "{labels:?}");
}
