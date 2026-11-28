//! The pairing rule of a GTA VI build's archives, measured: `X.rpf` and `X_concat.rpf` are two whole
//! archives. Every row's body lies inside the file that holds the row, so no row of `X.rpf` names a
//! body in `X_concat.rpf`, and a `_concat` holds nothing but nested archives, each of which mounts.
//!
//! Install-gated: it skips cleanly without the keys and the build.
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot of the build
//!   `OPENRAGE_GTA6_BUILD`  the build root

#![cfg(feature = "gta6")]

use std::fs::File;
use std::path::PathBuf;

use or_io::container::rpf::{Gta6Class, Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";

/// Every `.rpf` under `root`, at any depth.
fn archives(root: PathBuf) -> Vec<PathBuf> {
    let (mut found, mut folders) = (Vec::new(), vec![root]);
    while let Some(folder) = folders.pop() {
        for path in (std::fs::read_dir(&folder).into_iter().flatten().flatten()).map(|e| e.path()) {
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rpf") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_row_names_a_body_in_its_own_file_and_a_concat_holds_only_archives_that_mount() {
    let env = |var: &str| (std::env::var_os(var).map(PathBuf::from)).filter(|p| p.exists());
    let (Some(snapshot), Some(build)) = (env(GTA6_SNAPSHOT_ENV), env(BUILD_ENV)) else {
        skip_install(
            "the pairing rule of a GTA VI build's archives",
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let ps5 = build.join("eboot.bin").is_file();
    let every = archives(build.clone());
    let (mut rows, mut concats, mut pairs, mut nested) = (0usize, 0usize, 0usize, 0usize);
    let mut broken = Vec::new();
    for path in &every {
        let label = path
            .strip_prefix(&build)
            .unwrap_or(path)
            .display()
            .to_string();
        let len = std::fs::metadata(path).expect("the archive's length").len();
        let mut file = File::open(path).expect("open the archive");
        let mount =
            Gta6Mount::open_root(&mut file, &keys).unwrap_or_else(|e| panic!("open {label}: {e}"));
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let concat = stem.ends_with("_concat");
        concats += usize::from(concat);
        pairs +=
            usize::from(!concat && path.with_file_name(format!("{stem}_concat.rpf")).is_file());
        for entry in &mount.archive.entries {
            rows += 1;
            // The rule: the body is where the row says, in this file.
            // A 20-byte row states no stored size here, so its start alone is held to the file.
            let end = (entry.start).and_then(|start| start.checked_add(entry.stored.unwrap_or(0)));
            if end.is_none_or(|end| end > len) {
                broken.push(format!(
                    "{label}: entry {} ends outside the file",
                    entry.index
                ));
            }
            if !concat {
                continue;
            }
            if entry.kind() != Some(Gta6Class::NestedArchive) {
                broken.push(format!("{label}: entry {} is no archive", entry.index));
            } else if let Err(e) = mount.open_nested(&mut file, entry, &keys) {
                broken.push(format!(
                    "{label}: entry {} does not mount: {e}",
                    entry.index
                ));
            } else {
                nested += 1;
            }
        }
    }
    println!(
        "{} archives, {concats} of them a _concat in {pairs} pairs, {rows} rows, {nested} archives in a _concat, {} broken",
        every.len(),
        broken.len()
    );
    for line in broken.iter().take(20) {
        eprintln!("  {line}");
    }
    assert!(broken.is_empty(), "the pairing rule holds for every row");
    // Measured per build: the PlayStation 5 build ships 40 archives, 19 of them a `_concat`; the Xbox
    // Series build ships none.
    if ps5 {
        assert_eq!((every.len(), concats, pairs), (40, 19, 16));
    } else {
        assert_eq!(concats, 0, "an Xbox Series build ships no _concat");
    }
}
