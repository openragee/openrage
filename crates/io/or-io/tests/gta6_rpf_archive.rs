//! A GTA VI archive opens through `RpfArchive`, as every other generation does: every top-level
//! archive of a build lists as a tree holding each entry of its table once, a nested archive opens
//! as an archive of its own through `open_nested`, and a body read through `RpfArchive` is the body
//! the GTA VI reader gives for the same row.
//!
//! Install-gated: it skips cleanly without the keys and the build.
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot, or `OPENRAGE_GTA6_EXE` for the bundled one
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::{
    Gta6Entry, Gta6Mount, GtaViKeys, RpfArchive, RpfVersion, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body stored larger than this is not read back: the check is the read, not the volume.
const BODY_CAP: u64 = 1 << 20;
/// How many bodies of one archive, and how many of its nested archives, are compared.
const BODIES: usize = 24;
const NESTED: usize = 4;

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

#[derive(Default)]
struct Tally {
    archives: usize,
    entries: usize,
    nested_listed: usize,
    nested_opened: usize,
    bodies: usize,
    failures: Vec<String>,
}

/// Compare `archive` with the same table mounted by the GTA VI reader: one file per entry, the
/// nested archives named as archives, and a sample of bodies equal.
fn compare(
    file: &mut File,
    archive: &Arc<RpfArchive>,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    t: &mut Tally,
) -> Vec<u32> {
    let what = format!("the archive at {:#X}", mount.base);
    if archive.version != RpfVersion::Rpf9 {
        t.failures
            .push(format!("{what} opened as {}", archive.version));
    }
    if archive.file_count() != mount.archive.entries.len() {
        t.failures.push(format!(
            "{what} lists {} files for {} entries",
            archive.file_count(),
            mount.archive.entries.len()
        ));
    }
    t.entries += mount.archive.entries.len();

    let rows: HashMap<u64, &Gta6Entry> = (mount.archive.entries.iter())
        .filter_map(|e| e.start.map(|start| (start, e)))
        .collect();
    let mut nested = Vec::new();
    let mut compared = 0;
    for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
        let index = node.entry_index();
        let Some(binary) = archive.binary_entry(index) else {
            t.failures
                .push(format!("{what}: entry {index} is not a file"));
            continue;
        };
        let Some(row) = rows.get(&binary.file_offset) else {
            continue;
        };
        let begins = mount.entry_begins_archive(file, row).unwrap_or(false);
        if begins != node.name().ends_with(".rpf") {
            t.failures
                .push(format!("{what}: entry {index} is named against its body"));
        }
        if begins {
            nested.push(index);
            continue;
        }
        // The place a caller opens the body at is the row's own: its absolute offset and entry.
        let placed = (archive.rpf9_place(index)).map(|p| (p.base, p.offset, p.entry));
        let stated = (mount.absolute_start(row)).map(|at| (mount.base, at, row.index));
        if placed != stated {
            t.failures
                .push(format!("{what}: entry {index} is placed off its row"));
        }
        if compared == BODIES || u64::from(binary.read_size()) > BODY_CAP {
            continue;
        }
        compared += 1;
        let through_archive = archive.read_entry(index).map_err(|e| e.to_string());
        let through_reader =
            (mount.read_entry_decrypted(&mut *file, row, keys)).map_err(|e| e.to_string());
        // A body both refuse is refused alike; what is compared is that neither reads another body.
        if through_archive.as_ref().ok() != through_reader.as_ref().ok() {
            t.failures
                .push(format!("{what}: entry {index} reads differently"));
        }
        t.bodies += usize::from(through_archive.is_ok());
    }
    t.nested_listed += nested.len();
    nested
}

#[test]
fn every_archive_of_the_build_opens_as_a_tree_and_reads_as_the_gta_vi_reader_does() {
    let what = "a GTA VI build through RpfArchive";
    let build = std::env::var_os(BUILD_ENV)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let (Some(build), Ok(Some(keys))) = (build, GtaViKeys::for_install(None)) else {
        skip_install(
            what,
            format_args!("set {BUILD_ENV} and {GTA6_SNAPSHOT_ENV} (or {GTA6_EXE_ENV})"),
        );
        return;
    };
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut t = Tally::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the GTA VI reader mounts it");
        let archive = match RpfArchive::open(path) {
            Ok(archive) => Arc::new(archive),
            Err(e) => {
                t.failures.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        t.archives += 1;
        let nested = compare(&mut file, &archive, &mount, &keys, &mut t);

        let rows: HashMap<u64, &Gta6Entry> = (mount.archive.entries.iter())
            .filter_map(|e| e.start.map(|start| (start, e)))
            .collect();
        for index in nested.into_iter().take(NESTED) {
            let offset = archive.binary_entry(index).map(|b| b.file_offset);
            let Some(row) = offset.and_then(|at| rows.get(&at)) else {
                continue;
            };
            let child_mount = mount
                .open_nested(&mut file, row, &keys)
                .expect("the reader opens it");
            match archive.open_nested(index) {
                Ok(child) => {
                    t.nested_opened += 1;
                    compare(&mut file, &Arc::new(child), &child_mount, &keys, &mut t);
                }
                Err(e) => t
                    .failures
                    .push(format!("{}: nested {index}: {e}", path.display())),
            }
        }
    }

    println!(
        "RpfArchive over a GTA VI build: {} archives, {} entries listed, {} nested archives named, \
         {} opened, {} bodies equal, {} failures",
        t.archives,
        t.entries,
        t.nested_listed,
        t.nested_opened,
        t.bodies,
        t.failures.len()
    );
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        t.failures.is_empty(),
        "RpfArchive and the GTA VI reader must agree"
    );
    assert_eq!(
        t.archives,
        archives.len(),
        "every archive of the build opens"
    );
    assert!(
        t.nested_opened > 0,
        "nested archives open through open_nested"
    );
    assert!(t.bodies > 0, "bodies read through RpfArchive");
}
