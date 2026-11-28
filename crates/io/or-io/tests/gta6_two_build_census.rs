//! The reader census of GTA VI's builds: both builds open in one process, each with the keys of
//! its own executable, and every row of every archive at any depth is counted by its kind, with a
//! sample of each kind's bodies read through `RpfArchive`. Its table states, for each kind, what
//! each build holds and what share of the sample reads, and every difference is named.
//!
//! Install-gated: it skips cleanly without the builds.
//!   `OPENRAGE_GTA6_BUILDS`  the roots of an Xbox Series build and a PlayStation 5 build, `;` between

#![cfg(all(feature = "gta6", feature = "bundled-keys"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::gta6_extract::snapshot_for_exe;
use or_io::container::rpf::{Gta6Context, GtaViKeys, NameResolver, RpfArchive, RpfEntry};
use or_test_support::skip_install;

const BUILDS_ENV: &str = "OPENRAGE_GTA6_BUILDS";
/// The executables a build's keys are read with.
const EXECUTABLES: [&str; 2] = ["game_scarlett_master.exe", "eboot.bin"];
/// The most bodies of one kind read in one build, and the largest body read.
const SAMPLE: usize = 24;
const LARGEST: u64 = 32 << 20;

/// What one build holds of one kind.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Share {
    rows: usize,
    sampled: usize,
    read: usize,
}

type Census = BTreeMap<String, Share>;

fn archives(root: &Path) -> Vec<PathBuf> {
    let (mut found, mut folders) = (Vec::new(), vec![root.to_path_buf()]);
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

fn walk(archive: &Arc<RpfArchive>, census: &mut Census, failures: &mut Vec<String>, label: &str) {
    for index in 0..archive.entries.len() as u32 {
        let RpfEntry::Binary(row) = &archive.entries[index as usize] else {
            continue;
        };
        let kind = match archive.rpf9_kind(index) {
            Some(kind) => kind.label.to_string(),
            None => "no kind".to_string(),
        };
        let share = census.entry(kind.clone()).or_default();
        share.rows += 1;
        if kind == "archive" {
            share.sampled += 1;
            match archive.open_nested(index) {
                Ok(nested) => {
                    share.read += 1;
                    walk(&Arc::new(nested), census, failures, label);
                }
                Err(e) => failures.push(format!("{label}: nested archive {index}: {e}")),
            }
            continue;
        }
        // Spread over the build: one body in every few rows of the kind, up to the sample.
        let size = u64::from(row.uncompressed_size.max(row.consumed_size));
        if share.sampled >= SAMPLE || share.rows % 7 != 1 || size > LARGEST {
            continue;
        }
        share.sampled += 1;
        match archive.read_entry(index) {
            Ok(_) => share.read += 1,
            Err(e) => failures.push(format!("{label}: {kind} {index}: {e}")),
        }
    }
}

#[test]
fn each_reader_s_share_of_a_playstation_5_build_stands_beside_the_xbox_series_s() {
    let roots: Vec<PathBuf> = std::env::var_os(BUILDS_ENV)
        .map(|v| std::env::split_paths(&v).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    let exe_of = |root: &Path| {
        EXECUTABLES
            .iter()
            .map(|n| root.join(n))
            .find(|p| p.is_file())
    };
    let (Some(xbox), Some(ps5)) = (
        roots.iter().find(|r| r.join(EXECUTABLES[0]).is_file()),
        roots.iter().find(|r| r.join(EXECUTABLES[1]).is_file()),
    ) else {
        skip_install(
            "the reader census of GTA VI's builds",
            format_args!("set {BUILDS_ENV} to an Xbox Series build and a PlayStation 5 build"),
        );
        return;
    };

    // Both builds are open at once: each archive is read with the keys of its own build.
    let names = Arc::new(NameResolver::new());
    let opened: Vec<(Vec<Arc<RpfArchive>>, usize)> = [xbox, ps5]
        .map(|root| {
            let exe = exe_of(root).expect("the build's executable");
            let (plain, _) =
                snapshot_for_exe(None, &exe).expect("the executable keys its snapshot");
            let keys = Arc::new(GtaViKeys::parse(&plain).expect("the snapshot parses"));
            let context = Arc::new(Gta6Context {
                keys,
                names: Arc::clone(&names),
            });
            let every = archives(root);
            let open: Vec<_> = (every.iter())
                .filter_map(|path| RpfArchive::open_with(path, context.clone()).ok())
                .map(Arc::new)
                .collect();
            (open, every.len())
        })
        .into();
    let mut failures = Vec::new();
    let mut tables: Vec<Census> = Vec::new();
    for ((open, on_disk), label) in opened.iter().zip(["Xbox Series", "PlayStation 5"]) {
        let mut census = Census::new();
        census.insert(
            "table".into(),
            Share {
                rows: *on_disk,
                sampled: *on_disk,
                read: open.len(),
            },
        );
        for archive in open {
            walk(archive, &mut census, &mut failures, label);
        }
        tables.push(census);
    }

    let (xbox, ps5) = (&tables[0], &tables[1]);
    let kinds: Vec<&String> = (xbox.keys().chain(ps5.keys()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    println!(
        "{:<26} {:>9} {:>11}   {:>9} {:>11}",
        "kind", "Xbox rows", "read", "PS5 rows", "read"
    );
    let mut deltas = Vec::new();
    for kind in kinds {
        let (x, p) = (
            xbox.get(kind).copied().unwrap_or_default(),
            ps5.get(kind).copied().unwrap_or_default(),
        );
        println!(
            "{:<26} {:>9} {:>5}/{:<5}   {:>9} {:>5}/{:<5}",
            kind, x.rows, x.read, x.sampled, p.rows, p.read, p.sampled
        );
        // A difference is a kind only one build holds, or a sample one build reads less of.
        let whole = |s: Share| s.read == s.sampled;
        if (x.rows == 0) != (p.rows == 0) || whole(x) != whole(p) {
            deltas.push(kind.clone());
        }
    }
    for line in failures.iter().take(40) {
        eprintln!("  {line}");
    }
    println!("{} failures; deltas: {deltas:?}", failures.len());
    assert_eq!(deltas, DELTAS, "the named differences");
    assert!(
        failures.len() <= UNREAD,
        "{} bodies did not read",
        failures.len()
    );
}

/// The kinds whose share differs between the builds, measured.
const DELTAS: &[&str] = &[];
/// The sampled bodies that do not read, over both builds, measured.
const UNREAD: usize = 0;
